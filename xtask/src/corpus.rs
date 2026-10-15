//! The conformance corpus (spec compiler §13.5.3).
//!
//! - `cargo xtask corpus fetch`: downloads the pinned archives listed in
//!   `xtask/corpus.txt` into `target/corpus/` (checked against SHA-256).
//! - `cargo xtask corpus run [dir]`: for every `.lua` file, checks that
//!   wowlua accepts what Lua 5.1 accepts, and that `emit(lower(parse))`
//!   compiles to the same bytecode as the file in every line mode.
//! - `cargo xtask corpus addons [dir]`: adopts and builds every addon.
//! - `cargo xtask corpus vm [dir]`: wowlua's VM compiles every file to the
//!   same bytecode and line info as Lua 5.1, and runs its main chunk with
//!   the same output and error as PUC Lua.
//! - `cargo xtask corpus lua51 [dir]`: the official Lua 5.1 test suite on
//!   wowlua's VM (`lua51.rs`).
//!
//! Both run in parallel: `WOWLUA_CORPUS_JOBS` sets the threads (one per
//! core by default) and `WOWLUA_CORPUS_MEMORY` the GiB that addon builds in
//! flight may use (60% of the physical memory by default).

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use wowlua_luair::bytecode::{Strip, strip};
use wowlua_luair::{CommentMode, EmitOptions, LineMode, emit};
use wowlua_span::SourceDb;

pub fn fetch(root: &Path, dir: &Path) -> Result<()> {
    let list = std::fs::read_to_string(root.join("xtask/corpus.txt"))?;
    std::fs::create_dir_all(dir)?;
    for line in list
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [name, url, sha] = parts[..] else {
            bail!("malformed corpus line: {line}");
        };
        let target = dir.join(name);
        let stamp = target.join(".sha256");
        if std::fs::read_to_string(&stamp).ok().as_deref() == Some(sha) {
            continue;
        }
        eprintln!("fetching {name}");
        let archive = dir.join(format!("{name}.download"));
        let status = Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(&archive)
            .arg(url)
            .status()
            .context("running curl")?;
        if !status.success() {
            bail!("downloading {url} failed");
        }
        let bytes = std::fs::read(&archive)?;
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != sha {
            bail!("{name}: SHA-256 mismatch (expected {sha}, got {actual})");
        }
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir_all(&target)?;
        let status = if url.ends_with(".zip") {
            Command::new("unzip")
                .arg("-q")
                .arg("-o")
                .arg(&archive)
                .arg("-d")
                .arg(&target)
                .status()
        } else {
            Command::new("tar")
                .arg("xzf")
                .arg(&archive)
                .arg("-C")
                .arg(&target)
                .status()
        }
        .context("extracting")?;
        if !status.success() {
            bail!("extracting {name} failed");
        }
        std::fs::remove_file(&archive)?;
        std::fs::write(&stamp, sha)?;
    }
    Ok(())
}

fn lua_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lua"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn dump(lua: &mlua::Lua, src: &[u8]) -> Result<Vec<u8>, String> {
    lua.load(src)
        .set_name("=corpus")
        .into_function()
        .map(|f| f.dump(false))
        .map_err(|e| e.to_string())
}

/// Minification passes that leave the bytecode unchanged.
const BYTECODE_PASSES: &[&str] = &[
    "strip-comments",
    "strip-whitespace",
    "compact",
    "shorten-literals",
    "simplify",
    "rename-locals",
];

const ALL_PASSES: &[&str] = &[
    "strip-comments",
    "strip-whitespace",
    "compact",
    "shorten-literals",
    "simplify",
    "rename-locals",
    "dedupe-strings",
    "rename-private",
];

#[derive(Default)]
struct Stats {
    files: usize,
    lines: usize,
    both_reject: usize,
    full_equal: usize,
    formatted: usize,
    parse_time: std::time::Duration,
    failures: Vec<String>,
}

pub fn run(dir: &Path) -> Result<()> {
    let started = std::time::Instant::now();
    let files = lua_files(dir);
    let results = parallel(
        &files,
        |path| std::fs::metadata(path).map_or(0, |m| m.len()),
        u64::MAX,
        mlua::Lua::new,
        |lua, path| check_file(lua, dir, path),
    );
    let mut stats = Stats::default();
    for result in results {
        let file = result?;
        stats.files += file.files;
        stats.lines += file.lines;
        stats.both_reject += file.both_reject;
        stats.full_equal += file.full_equal;
        stats.formatted += file.formatted;
        stats.parse_time += file.parse_time;
        stats.failures.extend(file.failures);
    }
    let checked = stats.files - stats.both_reject;
    println!(
        "{} files ({} lines) in {:.1?} (parsing {:.1?}); {} rejected by both; line info identical for {}/{}; formatted {}; {} failures",
        stats.files,
        stats.lines,
        started.elapsed(),
        stats.parse_time,
        stats.both_reject,
        stats.full_equal,
        checked,
        stats.formatted,
        stats.failures.len()
    );
    for f in stats.failures.iter().take(50) {
        println!("  {f}");
    }
    if !stats.failures.is_empty() {
        bail!("{} corpus failures", stats.failures.len());
    }
    Ok(())
}

/// The corpus checks on one file.
fn check_file(lua: &mlua::Lua, dir: &Path, path: &Path) -> Result<Stats> {
    let mut stats = Stats::default();
    let stripped = Strip {
        line_defined: true,
        debug: true,
    };
    let bytes = std::fs::read(path)?;
    let src = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    if src.starts_with(b"#") {
        return Ok(stats);
    }
    stats.files += 1;
    stats.lines += src.iter().filter(|&&c| c == b'\n').count() + 1;
    let rel = path.strip_prefix(dir).unwrap_or(path).display().to_string();
    let mut db = SourceDb::new();
    let file = db.add(&rel, String::from_utf8_lossy(src).into_owned());
    let t = std::time::Instant::now();
    let parsed = wowlua_syntax::parse(src, file);
    stats.parse_time += t.elapsed();
    let reference = dump(lua, src);
    match (&reference, parsed.has_errors()) {
        (Err(_), true) => {
            stats.both_reject += 1;
            return Ok(stats);
        }
        (Err(e), false) => {
            stats
                .failures
                .push(format!("{rel}: accepted, but Lua 5.1 rejects it: {e}"));
            return Ok(stats);
        }
        (Ok(_), true) => {
            let first = parsed.diagnostics.iter().find(|d| d.is_error());
            let message = first.map(|d| d.message.clone()).unwrap_or_default();
            let line = first
                .and_then(|d| d.labels.first())
                .map(|l| parsed.lexed.line_of(l.span.start))
                .unwrap_or(0);
            stats.failures.push(format!(
                "{rel}:{line}: rejected, but Lua 5.1 accepts it: {message}"
            ));
            return Ok(stats);
        }
        (Ok(_), false) => {}
    }
    let original = reference.expect("checked");
    let chunk = match wowlua_lower::lower(&parsed, src) {
        Ok(chunk) => chunk,
        Err(errors) => {
            stats
                .failures
                .push(format!("{rel}: lowering failed: {}", errors[0].message));
            return Ok(stats);
        }
    };
    // The formatter: succeeds, keeps the meaning (checked inside
    // `format`), and is idempotent.
    let config = wowlua_fmt::Config::default();
    match wowlua_fmt::format(src, file, &config) {
        Ok(once) => match wowlua_fmt::format(&once, file, &config) {
            Ok(twice) if twice == once => stats.formatted += 1,
            Ok(twice) => {
                let a = String::from_utf8_lossy(&once).into_owned();
                let b = String::from_utf8_lossy(&twice).into_owned();
                let at = a
                    .lines()
                    .zip(b.lines())
                    .position(|(x, y)| x != y)
                    .unwrap_or(0);
                let la: Vec<&str> = a.lines().collect();
                let lb: Vec<&str> = b.lines().collect();
                let show = |l: &[&str]| {
                    l.get(at.saturating_sub(1)..(at + 2).min(l.len()))
                        .map(|s| s.join(" | "))
                        .unwrap_or_default()
                };
                stats.failures.push(format!("{rel}: formatting is not idempotent at line {}:\n      once:  {}\n      twice: {}", at + 1, show(&la), show(&lb)));
            }
            Err(e) => stats
                .failures
                .push(format!("{rel}: formatted output fails to format: {e:?}")),
        },
        Err(wowlua_fmt::Error::Unsupported(_)) => {}
        Err(e) => stats
            .failures
            .push(format!("{rel}: formatting failed: {e:?}")),
    }
    // Minification (spec optimization §15.4): every pass but
    // `dedupe-strings` keeps the bytecode, and preserve mode keeps the
    // lines of the plain output; with `dedupe-strings` it still compiles.
    for mode in [LineMode::Preserve, LineMode::Statement] {
        for names in [BYTECODE_PASSES, ALL_PASSES] {
            let passes = wowlua_luair::Passes::from_names(names.iter().copied());
            // The release optimizations (spec optimization §15.2) with
            // every pass, as the compiler runs them.
            let optimizations = if names == ALL_PASSES {
                wowlua_driver::Optimizations {
                    dead_code: true,
                    inline: true,
                    cache_globals: true,
                }
            } else {
                wowlua_driver::Optimizations::default()
            };
            let out = wowlua_driver::optimize_and_emit(
                chunk.clone(),
                &optimizations,
                &passes,
                &passes.emit_options(mode),
            )
            .emitted;
            let label = if names == BYTECODE_PASSES {
                "all passes but dedupe-strings"
            } else {
                "all passes"
            };
            let new = match dump(lua, &out.text) {
                Ok(new) => new,
                Err(e) => {
                    stats.failures.push(format!(
                        "{rel}: minified with {label} in {mode:?} mode, it does not compile: {e}"
                    ));
                    continue;
                }
            };
            if names != BYTECODE_PASSES {
                continue;
            }
            if strip(&original, stripped) != strip(&new, stripped) {
                stats.failures.push(format!(
                    "{rel}: minified with {label} in {mode:?} mode, it compiles to different bytecode"
                ));
            } else if mode == LineMode::Preserve {
                let plain = emit(&chunk, &EmitOptions::default());
                let lines = |code: &[u8]| {
                    dump(lua, code)
                        .ok()
                        .and_then(|d| wowlua_luair::bytecode::line_info(&d))
                };
                if lines(&plain.text) != lines(&out.text) {
                    stats.failures.push(format!(
                        "{rel}: minified in preserve mode, instructions move to other lines"
                    ));
                }
            }
        }
    }
    for mode in [LineMode::Preserve, LineMode::Statement, LineMode::Pretty] {
        let comments = if mode == LineMode::Preserve {
            CommentMode::All
        } else {
            CommentMode::None
        };
        let out = emit(
            &chunk,
            &EmitOptions {
                line_mode: mode,
                comments,
                indent: "\t".into(),
                compact: false,
                mappings: true,
            },
        );
        match dump(lua, &out.text) {
            Err(e) => {
                stats
                    .failures
                    .push(format!("{rel}: {mode:?} output does not compile: {e}"));
            }
            Ok(new) => {
                if strip(&original, stripped) != strip(&new, stripped) {
                    stats.failures.push(format!(
                        "{rel}: {mode:?} output compiles to different bytecode"
                    ));
                } else if mode == LineMode::Preserve && original == new {
                    stats.full_equal += 1;
                } else if mode == LineMode::Preserve
                    && std::env::var_os("WOWLUA_CORPUS_LINES").is_some()
                {
                    report_line_diff(&rel, src, &out.text, &original, &new);
                }
            }
        }
    }
    Ok(stats)
}

/// Prints the first instruction whose line differs (with the source line).
fn report_line_diff(rel: &str, src: &[u8], out: &[u8], original: &[u8], new: &[u8]) {
    let (Some(a), Some(b)) = (
        wowlua_luair::bytecode::line_info(original),
        wowlua_luair::bytecode::line_info(new),
    ) else {
        return;
    };
    for (fa, fb) in a.iter().zip(&b) {
        if let Some(pc) = fa.iter().zip(fb).position(|(x, y)| x != y) {
            let line = |text: &[u8], n: u32| {
                String::from_utf8_lossy(text)
                    .lines()
                    .nth(n.saturating_sub(1) as usize)
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(90)
                    .collect::<String>()
            };
            println!(
                "LINES {rel}: pc {pc}: source line {} `{}` / output line {} `{}`",
                fa[pc],
                line(src, fa[pc]),
                fb[pc],
                line(out, fb[pc])
            );
            return;
        }
    }
}

/// `cargo xtask corpus addons [dir]`: adopts every addon of the corpus with
/// `init` and builds it (spec roadmap M2: a migrated plain-Lua addon keeps
/// its working load order). Checks that each generated TOC lists the
/// sources in the order `init` recorded from the original TOCs.
pub fn addons(dir: &Path) -> Result<()> {
    let mut addon_dirs = Vec::new();
    for pkg in std::fs::read_dir(dir)?.flatten() {
        for addon in std::fs::read_dir(pkg.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = addon.path();
            let name = addon.file_name().to_string_lossy().into_owned();
            let has_toc = std::fs::read_dir(&path)
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| {
                    let f = e.file_name().to_string_lossy().to_ascii_lowercase();
                    f.ends_with(".toc") && f.starts_with(&name.to_ascii_lowercase())
                });
            if path.is_dir() && has_toc {
                addon_dirs.push(path);
            }
        }
    }
    addon_dirs.sort();
    let started = std::time::Instant::now();
    // `WOWLUA_CORPUS_ONLY=<text>` checks only matching addons;
    // `WOWLUA_CORPUS_VERBOSE=1` prints the errors of failing builds, `=all`
    // every diagnostic.
    let only = std::env::var("WOWLUA_CORPUS_ONLY").ok();
    addon_dirs.retain(|d| {
        only.as_ref()
            .is_none_or(|o| d.display().to_string().contains(o.as_str()))
    });
    // Whole packages are copied and checked in turn: addons may load files
    // of their siblings. Packages run in parallel, within a memory budget.
    let mut packages: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for src in &addon_dirs {
        let pkg = src.parent().expect("a package folder").to_path_buf();
        packages.entry(pkg).or_default().push(src.clone());
    }
    let packages: Vec<(PathBuf, Vec<PathBuf>)> = packages.into_iter().collect();
    let results = parallel(
        &packages,
        |(pkg, _)| lua_bytes(pkg).saturating_mul(MEMORY_PER_SOURCE_BYTE),
        memory_budget(),
        || (),
        |(), (pkg, addons)| check_package(dir, pkg, addons),
    );
    let mut failures = Vec::new();
    let mut codes: BTreeMap<String, usize> = BTreeMap::new();
    let mut fixed = 0;
    for result in results {
        for addon in result? {
            print!("{}", addon.log);
            failures.extend(addon.failures);
            for (code, n) in addon.codes {
                *codes.entry(code).or_default() += n;
            }
            fixed += addon.fixed;
        }
    }
    if fixed > 0 {
        println!("applied {fixed} lint fixes");
    }
    println!(
        "{} addons in {:.1?}; diagnostics by code: {:?}; {} failures",
        addon_dirs.len(),
        started.elapsed(),
        codes,
        failures.len()
    );
    for f in failures.iter().take(40) {
        println!("  {f}");
    }
    if !failures.is_empty() {
        bail!(
            "{} addons changed their load order or failed",
            failures.len()
        );
    }
    Ok(())
}

/// What checking one addon found.
#[derive(Default)]
struct AddonReport {
    failures: Vec<String>,
    codes: BTreeMap<String, usize>,
    fixed: usize,
    /// The verbose output.
    log: String,
}

/// Checks the addons of one package, in a temporary copy of it.
fn check_package(dir: &Path, pkg: &Path, addons: &[PathBuf]) -> Result<Vec<AddonReport>> {
    let temp = tempfile::tempdir()?;
    copy_dir(pkg, temp.path())?;
    addons
        .iter()
        .map(|src| {
            let rel = src.strip_prefix(dir).unwrap_or(src).display().to_string();
            let work = temp.path().join(src.file_name().expect("a folder name"));
            check_addon(&rel, &work)
        })
        .collect()
}

/// Adopts one addon with `init`, builds it and checks the result.
fn check_addon(rel: &str, work: &Path) -> Result<AddonReport> {
    use std::fmt::Write as _;
    let mut out = AddonReport::default();
    let verbose = std::env::var_os("WOWLUA_CORPUS_VERBOSE").is_some();
    let work = work.to_path_buf();
    let report = wowlua_driver::scaffold::init_package(&wowlua_driver::scaffold::InitRequest {
        dir: work.clone(),
        from_toc: None,
        from_pkgmeta: false,
        pkgmeta_file: None,
        inherited: None,
        addon_name: None,
        neighbors: Vec::new(),
    });
    if let Err(e) = report {
        out.failures
            .push(format!("{rel}: init failed: {}", e.message));
        return Ok(out);
    }
    // Type-check the plain Lua files too (basic mode, warnings only):
    // the corpus must check with zero errors (spec compiler §13.5.3).
    let manifest_path = work.join("wowlua.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;
    manifest["check"] = serde_json::json!({ "lua": true });
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
    let mut session = wowlua_driver::Session::new();
    let options = wowlua_driver::LoadOptions {
        manifest: None,
        cwd: work.clone(),
        ..Default::default()
    };
    // A converted `move-folders` makes a workspace: check its root.
    let Some(project) = wowlua_driver::load_projects(&mut session, &options)
        .and_then(|ps| ps.into_iter().find(|p| p.manifest.root == work))
    else {
        out.failures
            .push(format!("{rel}: the adopted manifest does not load"));
        return Ok(out);
    };
    let mut request = wowlua_driver::BuildRequest::new("dev");
    request.write = false;
    let build = wowlua_driver::build(&mut session, &project, &request);
    for d in session.diagnostics.iter() {
        *out.codes
            .entry(d.code.unwrap_or("-").to_string())
            .or_default() += 1;
    }
    if std::env::var("WOWLUA_CORPUS_VERBOSE").as_deref() == Ok("all") {
        let renderer = wowlua_diagnostics::Renderer::new(
            wowlua_diagnostics::MessageFormat::Human,
            wowlua_diagnostics::ColorChoice::Never,
        );
        writeln!(out.log, "== {rel}")?;
        for d in session.diagnostics.iter() {
            writeln!(out.log, "{}", renderer.render_to_string(&session.db, d))?;
        }
    }
    let Some(build) = build else {
        if verbose {
            let renderer = wowlua_diagnostics::Renderer::new(
                wowlua_diagnostics::MessageFormat::Human,
                wowlua_diagnostics::ColorChoice::Never,
            );
            writeln!(out.log, "== {rel}")?;
            for d in session.diagnostics.iter().filter(|d| d.is_error()) {
                writeln!(out.log, "{}", renderer.render_to_string(&session.db, d))?;
            }
        }
        let first = session.diagnostics.iter().find(|d| d.is_error());
        out.failures.push(format!(
            "{rel}: build failed: {} {}",
            first.and_then(|d| d.code).unwrap_or(""),
            first.map(|d| d.message.as_str()).unwrap_or("")
        ));
        return Ok(out);
    };
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(work.join("wowlua.json"))?)?;
    let order: Vec<String> = manifest
        .pointer("/build/load-order")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    for toc in &build.tocs {
        let listed: Vec<String> = toc
            .content
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .map(|l| l.split(" [").next().unwrap_or(l).replace('\\', "/"))
            .collect();
        let positions: Vec<(usize, &String)> = listed
            .iter()
            .filter_map(|f| order.iter().position(|o| o == f).map(|p| (p, f)))
            .collect();
        if let Some(w) = positions.windows(2).find(|w| w[1].0 < w[0].0) {
            out.failures.push(format!(
                "{rel}: {} loads {} before {}",
                toc.name, w[1].1, w[0].1
            ));
            break;
        }
    }
    // Optimized builds (spec optimization §15.4.5): bundled, with the
    // release optimizations, at every minification level; every Lua
    // file of the output must compile.
    let lua = mlua::Lua::new();
    for level in ["none", "strip", "compact", "rename", "max"] {
        let mut session = wowlua_driver::Session::new();
        let mut release = wowlua_driver::BuildRequest::new("release");
        release.write = false;
        release.minify = Some(level.into());
        release.bundle = Some(true);
        let Some(built) = wowlua_driver::build(&mut session, &project, &release) else {
            let first = session.diagnostics.iter().find(|d| d.is_error());
            out.failures.push(format!(
                "{rel}: the bundled `{level}` build fails: {} {}",
                first.and_then(|d| d.code).unwrap_or(""),
                first.map(|d| d.message.as_str()).unwrap_or("")
            ));
            continue;
        };
        for (path, content) in &built.files {
            if !path.to_ascii_lowercase().ends_with(".lua") {
                continue;
            }
            let bytes = match content {
                wowlua_driver::build::Content::Bytes(b) => b.clone(),
                wowlua_driver::build::Content::Copy(p) => std::fs::read(p)?,
            };
            let code = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            if let Err(e) = lua.load(code).into_function() {
                out.failures.push(format!(
                    "{rel}: `{level}` output {path} does not compile: {e}"
                ));
            }
        }
    }
    // `WOWLUA_CORPUS_FIX=1`: apply the lint fixes, then the addon must
    // still build without errors.
    if std::env::var_os("WOWLUA_CORPUS_FIX").is_some() {
        for _ in 0..5 {
            let mut session = wowlua_driver::Session::new();
            let Some(project) = wowlua_driver::load_project(&mut session, &options) else {
                break;
            };
            let _ = wowlua_driver::build(&mut session, &project, &request);
            let diagnostics: Vec<wowlua_diagnostics::Diagnostic> =
                session.take_diagnostics().into_vec();
            let lints: Vec<&wowlua_diagnostics::Diagnostic> =
                diagnostics.iter().filter(|d| d.is_lint()).collect();
            let applied = wowlua_driver::fix::apply(&session.db, &lints, project.root())?;
            if applied.fixes == 0 {
                break;
            }
            out.fixed += applied.fixes;
        }
        let mut session = wowlua_driver::Session::new();
        let rebuilt = wowlua_driver::load_project(&mut session, &options)
            .and_then(|project| wowlua_driver::build(&mut session, &project, &request));
        if rebuilt.is_none() {
            let first = session.diagnostics.iter().find(|d| d.is_error());
            let renderer = wowlua_diagnostics::Renderer::new(
                wowlua_diagnostics::MessageFormat::Short,
                wowlua_diagnostics::ColorChoice::Never,
            );
            out.failures.push(format!(
                "{rel}: the fixed sources fail: {}",
                first
                    .map(|d| renderer.render_to_string(&session.db, d))
                    .unwrap_or_default()
            ));
        }
    }
    Ok(out)
}

/// Peak memory of an addon build per byte of its Lua source, measured on
/// the corpus.
const MEMORY_PER_SOURCE_BYTE: u64 = 180;

/// The size of the Lua files under `dir`.
fn lua_bytes(dir: &Path) -> u64 {
    lua_files(dir)
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| m.len())
        .sum()
}

/// Worker threads: `WOWLUA_CORPUS_JOBS`, else one per core.
fn jobs() -> usize {
    std::env::var("WOWLUA_CORPUS_JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
        .max(1)
}

/// Memory for builds in flight: `WOWLUA_CORPUS_MEMORY` (GiB), else 60% of
/// the physical memory.
fn memory_budget() -> u64 {
    const GIB: u64 = 1 << 30;
    if let Some(gib) = std::env::var("WOWLUA_CORPUS_MEMORY")
        .ok()
        .and_then(|m| m.parse::<f64>().ok())
    {
        return (gib * GIB as f64) as u64;
    }
    physical_memory().map_or(8 * GIB, |m| m / 10 * 6)
}

fn physical_memory() -> Option<u64> {
    if let Ok(info) = std::fs::read_to_string("/proc/meminfo") {
        let kib = info
            .lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))?
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse::<u64>()
            .ok()?;
        return Some(kib * 1024);
    }
    let out = Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

/// Runs `f` on every item on [`jobs`] threads, each with its own state from
/// `init`, and returns the results in item order. Heavier items start
/// first. An item starts only while the weights in flight stay within
/// `budget` (one over it runs alone); meanwhile workers take lighter items.
fn parallel<T: Sync, S, R: Send>(
    items: &[T],
    weight: impl Fn(&T) -> u64 + Sync,
    budget: u64,
    init: impl Fn() -> S + Sync,
    f: impl Fn(&mut S, &T) -> R + Sync,
) -> Vec<R> {
    use std::sync::{Condvar, Mutex};
    struct Queue {
        /// Item indices and weights, the heaviest first.
        pending: Vec<(usize, u64)>,
        in_flight: u64,
        running: usize,
    }
    let mut pending: Vec<(usize, u64)> = items
        .iter()
        .enumerate()
        .map(|(i, t)| (i, weight(t)))
        .collect();
    pending.sort_by_key(|&(i, w)| (std::cmp::Reverse(w), i));
    let queue = Mutex::new(Queue {
        pending,
        in_flight: 0,
        running: 0,
    });
    let ready = Condvar::new();
    let results: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..jobs().min(items.len()) {
            scope.spawn(|| {
                let mut state = init();
                loop {
                    let (i, w) = {
                        let mut q = queue.lock().unwrap();
                        loop {
                            if q.pending.is_empty() {
                                return;
                            }
                            let fits = q.pending.iter().position(|&(_, w)| {
                                q.running == 0 || q.in_flight.saturating_add(w) <= budget
                            });
                            if let Some(at) = fits {
                                let next = q.pending.remove(at);
                                q.in_flight += next.1;
                                q.running += 1;
                                break next;
                            }
                            q = ready.wait(q).unwrap();
                        }
                    };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        f(&mut state, &items[i])
                    }));
                    {
                        let mut q = queue.lock().unwrap();
                        q.in_flight -= w;
                        q.running -= 1;
                    }
                    ready.notify_all();
                    match result {
                        Ok(r) => results.lock().unwrap()[i] = Some(r),
                        // The others go on; the scope then re-raises it.
                        Err(panic) => std::panic::resume_unwind(panic),
                    }
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// `cargo xtask corpus vm`: compiles every corpus file with wowlua's VM
/// compiler and compares the bytecode with Lua 5.1's, instruction for
/// instruction (spec chapter 17, §17.8).
pub fn vm(dir: &Path) -> Result<()> {
    let started = std::time::Instant::now();
    let files = lua_files(dir);
    let results = parallel(
        &files,
        |path| std::fs::metadata(path).map_or(0, |m| m.len()),
        u64::MAX,
        mlua::Lua::new,
        |lua, path| vm_file(lua, dir, path),
    );
    let (mut checked, mut identical, mut both_reject, mut line_diffs, mut ran_same) =
        (0, 0, 0, 0usize, 0);
    let mut failures = Vec::new();
    let mut by_file: Vec<(usize, String)> = Vec::new();
    for (r, path) in results.into_iter().zip(&files) {
        let r = r?;
        checked += r.0;
        identical += r.1;
        both_reject += r.2;
        line_diffs += r.3;
        ran_same += r.4;
        if r.3 > 0 {
            by_file.push((
                r.3,
                path.strip_prefix(dir).unwrap_or(path).display().to_string(),
            ));
        }
        failures.extend(r.5);
    }
    by_file.sort_by(|a, b| b.cmp(a));
    for (n, f) in by_file.iter().take(10) {
        println!("{n:6} instructions with other lines: {f}");
    }
    for f in failures.iter().take(40) {
        println!("{f}");
    }
    println!(
        "{checked} files in {:.1?}: bytecode identical to Lua 5.1 for {identical}; {both_reject} rejected by both; {} instructions with other lines; {ran_same} run the same; {} failures",
        started.elapsed(),
        line_diffs,
        failures.len()
    );
    if !failures.is_empty() {
        bail!("{} files differ", failures.len());
    }
    Ok(())
}

/// Checked, bytecode identical, rejected by both, instructions with other
/// lines, ran the same, failures.
type VmFile = (usize, usize, usize, usize, usize, Vec<String>);

fn vm_file(lua: &mut mlua::Lua, dir: &Path, path: &Path) -> Result<VmFile> {
    let bytes = std::fs::read(path)?;
    let src = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    if src.starts_with(b"#") {
        return Ok((0, 0, 0, 0, 0, Vec::new()));
    }
    let rel = path.strip_prefix(dir).unwrap_or(path).display().to_string();
    let reference = dump(lua, src);
    let mut db = SourceDb::new();
    let file = db.add(&rel, String::from_utf8_lossy(src).into_owned());
    let parsed = wowlua_syntax::parse(src, file);
    let mut heap = wowlua_vm::heap::Heap::default();
    let ours = if parsed.has_errors() {
        Err("syntax error".to_string())
    } else {
        // The reference is a stock Lua 5.1 build, with LUA_COMPAT_VARARG.
        let options = wowlua_vm::compiler::Options {
            compat_vararg: true,
        };
        wowlua_vm::compiler::compile_with(
            &mut heap,
            &parsed.chunk,
            src,
            std::rc::Rc::from(&b"=corpus"[..]),
            options,
        )
        .map_err(|e| format!("line {}: {}", e.line, e.message))
    };
    match (reference, ours) {
        (Err(_), Err(_)) => Ok((1, 0, 1, 0, 0, Vec::new())),
        (Err(e), Ok(_)) => Ok((
            1,
            0,
            0,
            0,
            0,
            vec![format!("{rel}: compiles, but Lua 5.1 rejects it: {e}")],
        )),
        (Ok(_), Err(e)) => Ok((
            1,
            0,
            0,
            0,
            0,
            vec![format!("{rel}: Lua 5.1 compiles it, ours fails: {e}")],
        )),
        (Ok(dumped), Ok(proto)) => {
            let Some(theirs) = wowlua_vm::luac::decode(&dumped) else {
                bail!("{rel}: cannot decode Lua's bytecode");
            };
            let lines = wowlua_vm::luac::line_differences(&proto, &theirs);
            let mut failures = Vec::new();
            let difference = wowlua_vm::luac::compare(&heap, &proto, &theirs, "main");
            let identical = usize::from(difference.is_none());
            failures.extend(difference.map(|d| format!("{rel}: {d}")));
            // The Lua test suite needs stock 5.1's compatibility features
            // the client lacks; `corpus lua51` runs it with its exclusions.
            if rel.starts_with("lua51-tests") {
                return Ok((1, identical, 0, lines, 0, failures));
            }
            let run = run_both(src, &rel)?;
            let ran_same = usize::from(run.is_none());
            failures.extend(run.map(|d| format!("{rel}: runs differently: {d}")));
            Ok((1, identical, 0, lines, ran_same, failures))
        }
    }
}

/// Captures `print` the same way in both VMs.
const RUN_PRELUDE: &str = "local out, tostring, select, concat = {}, tostring, select, table.concat \
    print = function (...) local t = {} for i = 1, select('#', ...) do \
    t[i] = tostring((select(i, ...))) end out[#out + 1] = concat(t, '\\t') end \
    __corpus_out = out";
const RUN_OUTPUT: &str = "return table.concat(__corpus_out, '\\n')";

/// Runs the file's main chunk in wowlua's VM and in PUC Lua 5.1 with the
/// libraries the client has, until it ends or fails (usually at the first
/// WoW API call): the same output and the same error, or the difference.
fn run_both(src: &[u8], rel: &str) -> Result<Option<String>> {
    let name = format!("@{rel}");
    // Ours.
    let mut vm = wowlua_vm::Vm::new();
    let prelude = vm
        .load(RUN_PRELUDE.as_bytes(), "=prelude")
        .map_err(anyhow::Error::msg)?;
    vm.call(prelude, &[])
        .map_err(|_| anyhow::anyhow!("prelude failed"))?;
    let ours_err = match vm.load_raw(src, name.as_bytes()) {
        Err(e) => Some(String::from_utf8_lossy(&e).into_owned()),
        Ok(f) => vm.call(f, &[]).err().map(|e| vm_message(&vm, e.value)),
    };
    let output = vm
        .load(RUN_OUTPUT.as_bytes(), "=output")
        .map_err(anyhow::Error::msg)?;
    let ours_out = match vm.call(output, &[]) {
        Ok(r) => r.first().map(|v| vm_message(&vm, *v)).unwrap_or_default(),
        Err(_) => "<output lost>".to_string(),
    };
    // PUC Lua (its base library includes `coroutine`).
    use mlua::StdLib;
    let lua_err = |e: mlua::Error| anyhow::anyhow!("{e}");
    let lua = mlua::Lua::new_with(
        StdLib::STRING | StdLib::TABLE | StdLib::MATH,
        mlua::LuaOptions::default(),
    )
    .map_err(lua_err)?;
    lua.load("dofile, loadfile, string.dump = nil")
        .exec()
        .map_err(lua_err)?;
    lua.load(RUN_PRELUDE).exec().map_err(lua_err)?;
    let theirs_err = lua
        .load(src)
        .set_name(&name)
        .exec()
        .err()
        .map(|e| mlua_message(&e));
    let theirs_out: String = lua
        .load(RUN_OUTPUT)
        .eval()
        .unwrap_or_else(|_| "<output lost>".into());
    let (ours_out, theirs_out) = (without_addresses(&ours_out), without_addresses(&theirs_out));
    let ours_err = ours_err.map(|e| without_addresses(&e));
    let theirs_err = theirs_err.map(|e| without_addresses(&e));
    if ours_err != theirs_err {
        return Ok(Some(format!("error {ours_err:?}, Lua's {theirs_err:?}")));
    }
    if ours_out != theirs_out {
        return Ok(Some(format!("output {ours_out:?}, Lua's {theirs_out:?}")));
    }
    Ok(None)
}

fn vm_message(vm: &wowlua_vm::Vm, v: wowlua_vm::Value) -> String {
    match v {
        wowlua_vm::Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        other => format!("({})", other.type_name()),
    }
}

fn mlua_message(e: &mlua::Error) -> String {
    let msg = match e {
        mlua::Error::RuntimeError(m) | mlua::Error::SyntaxError { message: m, .. } => m.clone(),
        mlua::Error::CallbackError { cause, .. } => cause.to_string(),
        other => other.to_string(),
    };
    // mlua appends a traceback to runtime errors.
    msg.split("\nstack traceback:")
        .next()
        .unwrap_or("")
        .to_string()
}

/// `0x…` addresses replaced, so object identities compare equal.
fn without_addresses(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("0x") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 2..];
        let n = tail.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        if n >= 6 {
            out.push_str("ADDR");
        } else {
            out.push_str(&rest[i..i + 2 + n]);
        }
        rest = &tail[n..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regular CI: the full corpus checks on the files in
    /// `tests/corpus-sample/`.
    #[test]
    fn sample_corpus() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        run(&root.join("tests/corpus-sample")).unwrap();
    }
}
