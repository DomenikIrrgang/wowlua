//! The conformance corpus (spec compiler §13.5.3).
//!
//! - `cargo xtask corpus fetch`: downloads the pinned archives listed in
//!   `xtask/corpus.txt` into `target/corpus/` (checked against SHA-256).
//! - `cargo xtask corpus run [dir]`: for every `.lua` file, checks that
//!   wowlua accepts what Lua 5.1 accepts, and that `emit(lower(parse))`
//!   compiles to the same bytecode as the file in every line mode.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
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
