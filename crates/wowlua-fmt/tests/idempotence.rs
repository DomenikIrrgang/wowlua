//! The formatter over every wowlua file of the repository (spec tooling
//! §16.3, milestone M9 S6): each formats without changing its meaning, and
//! formatting the result again changes nothing.

use std::path::{Path, PathBuf};
use wowlua_fmt::{Config, Error, format};
use wowlua_span::SourceDb;

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !p.ends_with("target") && !p.ends_with("dist") && !p.ends_with(".git") {
                files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "wowlua") {
            out.push(p);
        }
    }
}

#[test]
fn every_wowlua_file_formats_stably() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut all = Vec::new();
    for dir in ["crates", "tests", "examples"] {
        files(&root.join(dir), &mut all);
    }
    assert!(all.len() > 100, "{} files", all.len());
    let config = Config::default();
    let mut failures = Vec::new();
    for path in &all {
        let src = std::fs::read(path).unwrap();
        let mut db = SourceDb::new();
        let file = db.add(
            path.to_string_lossy().as_ref(),
            String::from_utf8_lossy(&src).into_owned(),
        );
        let once = match format(&src, file, &config) {
            Ok(out) => out,
            // Files that do not parse (fixtures of errors) are left alone.
            Err(Error::Syntax(_)) => continue,
            Err(e) => {
                failures.push(format!("{}: {e:?}", path.display()));
                continue;
            }
        };
        match format(&once, file, &config) {
            Ok(twice) if twice == once => {}
            Ok(_) => failures.push(format!("{}: not idempotent", path.display())),
            Err(e) => failures.push(format!("{}: second pass: {e:?}", path.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} files:\n{}",
        failures.len(),
        all.len(),
        failures.join("\n")
    );
}

/// How much of the hand-formatted standard library the formatter keeps as
/// it is (4 spaces): shows its differences (`--ignored --nocapture`).
#[test]
#[ignore]
fn standard_library_as_written() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/wowlua-stdlib/std");
    let mut all = Vec::new();
    files(&root, &mut all);
    let config = Config {
        indent: "    ".into(),
        ..Config::default()
    };
    let mut same = 0;
    for path in &all {
        let src = std::fs::read(path).unwrap();
        let mut db = SourceDb::new();
        let file = db.add("x", String::from_utf8_lossy(&src).into_owned());
        let Ok(out) = format(&src, file, &config) else {
            continue;
        };
        if out == src {
            same += 1;
            continue;
        }
        let a = String::from_utf8_lossy(&src);
        let b = String::from_utf8_lossy(&out);
        let changed: Vec<(usize, &str, &str)> = a
            .lines()
            .zip(b.lines())
            .enumerate()
            .filter(|(_, (x, y))| x != y)
            .map(|(i, (x, y))| (i + 1, x, y))
            .take(3)
            .collect();
        println!(
            "{}: {:?}",
            path.file_name().unwrap().to_string_lossy(),
            changed
        );
    }
    println!("{same} of {} unchanged", all.len());
}

/// The generated API declarations as the formatter prints them
/// (`--ignored --nocapture`): the first differences.
#[test]
#[ignore]
fn api_declarations_as_written() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/wowlua-api/data/camelot.d.wowlua");
    let src = std::fs::read(&path).unwrap();
    let mut db = SourceDb::new();
    let file = db.add("x", String::from_utf8_lossy(&src).into_owned());
    let config = Config {
        indent: "    ".into(),
        ..Config::default()
    };
    let out = format(&src, file, &config).unwrap();
    let a = String::from_utf8_lossy(&src);
    let b = String::from_utf8_lossy(&out);
    let diffs: Vec<String> = a
        .lines()
        .zip(b.lines())
        .enumerate()
        .filter(|(_, (x, y))| x != y)
        .take(8)
        .map(|(i, (x, y))| format!("{}:\n- {x}\n+ {y}", i + 1))
        .collect();
    println!(
        "{} lines, {} differ\n{}",
        a.lines().count(),
        a.lines().zip(b.lines()).filter(|(x, y)| x != y).count(),
        diffs.join("\n")
    );
}
