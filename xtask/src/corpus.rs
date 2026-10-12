//! The conformance corpus (spec compiler §13.5.3).
//!
//! - `cargo xtask corpus fetch`: downloads the pinned archives listed in
//!   `xtask/corpus.txt` into `target/corpus/` (checked against SHA-256).
//! - `cargo xtask corpus run [dir]`: for every `.lua` file, checks that
//!   wowlua accepts what Lua 5.1 accepts, and that `emit(lower(parse))`
//!   compiles to the same bytecode as the file in every line mode.

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

#[derive(Default)]
struct Stats {
    files: usize,
    lines: usize,
    both_reject: usize,
    full_equal: usize,
    formatted: usize,
    failures: Vec<String>,
}

pub fn run(dir: &Path) -> Result<()> {
    let lua = mlua::Lua::new();
    let mut stats = Stats::default();
    let started = std::time::Instant::now();
    let mut parse_time = std::time::Duration::ZERO;
    let stripped = Strip {
        line_defined: true,
        debug: true,
    };
    for path in lua_files(dir) {
        let bytes = std::fs::read(&path)?;
        let src = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        if src.starts_with(b"#") {
            continue;
        }
        stats.files += 1;
        stats.lines += src.iter().filter(|&&c| c == b'\n').count() + 1;
        let rel = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .display()
            .to_string();
        let mut db = SourceDb::new();
        let file = db.add(&rel, String::from_utf8_lossy(src).into_owned());
        let t = std::time::Instant::now();
        let parsed = wowlua_syntax::parse(src, file);
        parse_time += t.elapsed();
        let reference = dump(&lua, src);
        match (&reference, parsed.has_errors()) {
            (Err(_), true) => {
                stats.both_reject += 1;
                continue;
            }
            (Err(e), false) => {
                stats
                    .failures
                    .push(format!("{rel}: accepted, but Lua 5.1 rejects it: {e}"));
                continue;
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
                continue;
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
                continue;
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
                },
            );
            match dump(&lua, &out.text) {
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
    }
    let checked = stats.files - stats.both_reject;
    println!(
        "{} files ({} lines) in {:.1?} (parsing {:.1?}); {} rejected by both; line info identical for {}/{}; formatted {}; {} failures",
        stats.files,
        stats.lines,
        started.elapsed(),
        parse_time,
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
    let mut failures = Vec::new();
    let mut codes: BTreeMap<String, usize> = BTreeMap::new();
    let started = std::time::Instant::now();
    let mut fixed = 0;
    // Whole packages are copied: addons may load files of their siblings.
    let mut copies: BTreeMap<PathBuf, tempfile::TempDir> = BTreeMap::new();
    // `WOWLUA_CORPUS_ONLY=<text>` checks only matching addons;
    // `WOWLUA_CORPUS_VERBOSE=1` prints the errors of failing builds, `=all`
    // every diagnostic.
    let only = std::env::var("WOWLUA_CORPUS_ONLY").ok();
    let verbose = std::env::var_os("WOWLUA_CORPUS_VERBOSE").is_some();
    addon_dirs.retain(|d| {
        only.as_ref()
            .is_none_or(|o| d.display().to_string().contains(o.as_str()))
    });
    for src in &addon_dirs {
        let rel = src.strip_prefix(dir).unwrap_or(src).display().to_string();
        let pkg = src.parent().expect("a package folder").to_path_buf();
        if !copies.contains_key(&pkg) {
            let temp = tempfile::tempdir()?;
            copy_dir(&pkg, temp.path())?;
            copies.insert(pkg.clone(), temp);
        }
        let work = copies[&pkg]
            .path()
            .join(src.file_name().expect("a folder name"));
        let report = wowlua_driver::scaffold::init_package(&wowlua_driver::scaffold::InitRequest {
            dir: work.clone(),
            from_toc: None,
            from_pkgmeta: false,
            inherited: None,
            addon_name: None,
        });
        if let Err(e) = report {
            failures.push(format!("{rel}: init failed: {}", e.message));
            continue;
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
            failures.push(format!("{rel}: the adopted manifest does not load"));
            continue;
        };
        let mut request = wowlua_driver::BuildRequest::new("dev");
        request.write = false;
        let build = wowlua_driver::build(&mut session, &project, &request);
        for d in session.diagnostics.iter() {
            *codes.entry(d.code.unwrap_or("-").to_string()).or_default() += 1;
        }
        if std::env::var("WOWLUA_CORPUS_VERBOSE").as_deref() == Ok("all") {
            let renderer = wowlua_diagnostics::Renderer::new(
                wowlua_diagnostics::MessageFormat::Human,
                wowlua_diagnostics::ColorChoice::Never,
            );
            println!("== {rel}");
            for d in session.diagnostics.iter() {
                println!("{}", renderer.render_to_string(&session.db, d));
            }
        }
        let Some(build) = build else {
            if verbose {
                let renderer = wowlua_diagnostics::Renderer::new(
                    wowlua_diagnostics::MessageFormat::Human,
                    wowlua_diagnostics::ColorChoice::Never,
                );
                println!("== {rel}");
                for d in session.diagnostics.iter().filter(|d| d.is_error()) {
                    println!("{}", renderer.render_to_string(&session.db, d));
                }
            }
            let first = session.diagnostics.iter().find(|d| d.is_error());
            failures.push(format!(
                "{rel}: build failed: {} {}",
                first.and_then(|d| d.code).unwrap_or(""),
                first.map(|d| d.message.as_str()).unwrap_or("")
            ));
            continue;
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
                failures.push(format!(
                    "{rel}: {} loads {} before {}",
                    toc.name, w[1].1, w[0].1
                ));
                break;
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
                fixed += applied.fixes;
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
                failures.push(format!(
                    "{rel}: the fixed sources fail: {}",
                    first
                        .map(|d| renderer.render_to_string(&session.db, d))
                        .unwrap_or_default()
                ));
            }
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
