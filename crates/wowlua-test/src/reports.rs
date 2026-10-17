//! Reports of a test run (spec Tooling §16.1.5, §16.1.6): test results as
//! JUnit XML, TAP, JSON or GitHub annotations; coverage as lcov, Cobertura
//! XML, JSON or a self-contained HTML page in the wowlua design system.

use std::fmt::Write;
use std::path::Path;

use crate::coverage::{FileCoverage, Report, Totals, percent};
use crate::{FileResult, Outcome};

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn json_str(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

// ------------------------------------------------------------------ tests

/// JUnit XML: a suite per test file and flavor.
pub fn junit(results: &[FileResult]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n");
    for r in results {
        let failures = r
            .tests
            .iter()
            .filter(|t| matches!(t.outcome, Outcome::Fail { .. }))
            .count()
            + usize::from(r.error.is_some());
        let skipped = r
            .tests
            .iter()
            .filter(|t| matches!(t.outcome, Outcome::Skip(_)))
            .count();
        let _ = writeln!(
            out,
            "  <testsuite name=\"{} ({})\" tests=\"{}\" failures=\"{failures}\" skipped=\"{skipped}\" time=\"{:.3}\">",
            xml(&r.file),
            xml(&r.flavor),
            r.tests.len(),
            r.seconds
        );
        if let Some(e) = &r.error {
            let _ = writeln!(
                out,
                "    <testcase name=\"(loading)\" classname=\"{}\"><failure message=\"{}\">{}</failure></testcase>",
                xml(&r.file),
                xml(e.lines().next().unwrap_or("")),
                xml(e)
            );
        }
        for t in &r.tests {
            let _ = write!(
                out,
                "    <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\"",
                xml(&t.name),
                xml(&r.file),
                t.seconds
            );
            match &t.outcome {
                Outcome::Pass => out.push_str("/>\n"),
                Outcome::Skip(reason) => {
                    let _ = writeln!(out, "><skipped message=\"{}\"/></testcase>", xml(reason));
                }
                Outcome::Fail { message, stack } => {
                    let _ = writeln!(
                        out,
                        "><failure message=\"{}\">{}</failure></testcase>",
                        xml(message.lines().next().unwrap_or("")),
                        xml(&format!("{message}\n{stack}"))
                    );
                }
            }
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

/// TAP version 13.
pub fn tap(results: &[FileResult]) -> String {
    let tests: Vec<(String, &Outcome)> = results
        .iter()
        .flat_map(|r| {
            r.tests.iter().map(move |t| {
                (
                    format!("{} ({}) › {}", r.file, r.flavor, t.name),
                    &t.outcome,
                )
            })
        })
        .collect();
    let errors: Vec<&FileResult> = results.iter().filter(|r| r.error.is_some()).collect();
    let mut out = format!("TAP version 13\n1..{}\n", tests.len() + errors.len());
    let mut n = 0;
    for r in errors {
        n += 1;
        let _ = writeln!(out, "not ok {n} - {} ({}) loads", r.file, r.flavor);
        let _ = writeln!(
            out,
            "  ---\n  message: {}\n  ...",
            json_str(r.error.as_deref().unwrap_or(""))
        );
    }
    for (name, outcome) in tests {
        n += 1;
        match outcome {
            Outcome::Pass => {
                let _ = writeln!(out, "ok {n} - {name}");
            }
            Outcome::Skip(reason) => {
                let _ = writeln!(out, "ok {n} - {name} # SKIP {reason}");
            }
            Outcome::Fail { message, stack } => {
                let _ = writeln!(out, "not ok {n} - {name}");
                let at = crate::location(stack).unwrap_or_default();
                let _ = writeln!(
                    out,
                    "  ---\n  message: {}\n  at: {}\n  ...",
                    json_str(message),
                    json_str(&at)
                );
            }
        }
    }
    out
}

/// JSON: every file with its tests.
pub fn tests_json(results: &[FileResult]) -> String {
    let files: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            let tests: Vec<serde_json::Value> = r
                .tests
                .iter()
                .map(|t| {
                    let (status, message, location) = match &t.outcome {
                        Outcome::Pass => ("passed", None, None),
                        Outcome::Skip(reason) => ("skipped", Some(reason.clone()), None),
                        Outcome::Fail { message, stack } => {
                            let (m, at) = crate::split_failure(message, stack);
                            ("failed", Some(m), at)
                        }
                    };
                    serde_json::json!({
                        "name": t.name,
                        "status": status,
                        "message": message,
                        "location": location,
                        "seconds": t.seconds,
                    })
                })
                .collect();
            serde_json::json!({
                "file": r.file,
                "flavor": r.flavor,
                "error": r.error,
                "seconds": r.seconds,
                "tests": tests,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "files": files })).unwrap_or_default() + "\n"
}

/// GitHub workflow commands: an error annotation per failure.
pub fn github(results: &[FileResult]) -> String {
    let mut out = String::new();
    let esc = |s: &str| {
        s.replace('%', "%25")
            .replace('\r', "%0D")
            .replace('\n', "%0A")
    };
    for r in results {
        if let Some(e) = &r.error {
            let _ = writeln!(out, "::error file={},title=wowlua test::{}", r.file, esc(e));
        }
        for t in &r.tests {
            if let Outcome::Fail { message, stack } = &t.outcome {
                let (m, at) = crate::split_failure(message, stack);
                let (file, line) = at
                    .as_deref()
                    .and_then(|a| a.rsplit_once(':'))
                    .map_or((r.file.clone(), "1".to_string()), |(f, l)| {
                        (f.to_string(), l.to_string())
                    });
                let _ = writeln!(
                    out,
                    "::error file={file},line={line},title={}::{}",
                    esc(&format!("{} ({})", t.name, r.flavor)),
                    esc(&m)
                );
            }
        }
    }
    out
}

// --------------------------------------------------------------- coverage

/// lcov tracefile, with function and branch data.
pub fn lcov(report: &Report) -> String {
    let mut out = String::from("TN:wowlua\n");
    for (path, f) in &report.files {
        let _ = writeln!(out, "SF:{path}");
        for ((line, n), (name, _)) in &f.functions {
            let _ = writeln!(out, "FN:{line},{name}#{n}");
        }
        for ((_, n), (name, hits)) in &f.functions {
            let _ = writeln!(out, "FNDA:{hits},{name}#{n}");
        }
        let t = Report::file_totals(f);
        let _ = writeln!(out, "FNF:{}\nFNH:{}", t.functions.1, t.functions.0);
        for ((line, n), b) in &f.branches {
            for (i, taken) in b.iter().enumerate() {
                let _ = writeln!(out, "BRDA:{line},{n},{i},{taken}");
            }
        }
        let _ = writeln!(out, "BRF:{}\nBRH:{}", t.branches.1, t.branches.0);
        for (line, hits) in &f.lines {
            let _ = writeln!(out, "DA:{line},{hits}");
        }
        let _ = writeln!(out, "LF:{}\nLH:{}\nend_of_record", t.lines.1, t.lines.0);
    }
    out
}

fn rate((covered, total): (u64, u64)) -> String {
    if total == 0 {
        "1".into()
    } else {
        format!("{:.4}", covered as f64 / total as f64)
    }
}

/// Cobertura XML: a package per folder.
pub fn cobertura(report: &Report) -> String {
    let t = report.totals();
    let mut out = format!(
        "<?xml version=\"1.0\" ?>\n<!DOCTYPE coverage SYSTEM \"http://cobertura.sourceforge.net/xml/coverage-04.dtd\">\n<coverage line-rate=\"{}\" branch-rate=\"{}\" lines-covered=\"{}\" lines-valid=\"{}\" branches-covered=\"{}\" branches-valid=\"{}\" complexity=\"0\" version=\"wowlua\" timestamp=\"0\">\n  <sources><source>.</source></sources>\n  <packages>\n",
        rate(t.lines),
        rate(t.branches),
        t.lines.0,
        t.lines.1,
        t.branches.0,
        t.branches.1
    );
    let mut folders: std::collections::BTreeMap<String, Vec<(&String, &FileCoverage)>> =
        std::collections::BTreeMap::new();
    for (path, f) in &report.files {
        let folder = path.rsplit_once('/').map_or(".", |(d, _)| d).to_string();
        folders.entry(folder).or_default().push((path, f));
    }
    for (folder, files) in folders {
        let mut pt = Totals::default();
        for (_, f) in &files {
            let ft = Report::file_totals(f);
            pt.lines.0 += ft.lines.0;
            pt.lines.1 += ft.lines.1;
            pt.branches.0 += ft.branches.0;
            pt.branches.1 += ft.branches.1;
        }
        let _ = writeln!(
            out,
            "    <package name=\"{}\" line-rate=\"{}\" branch-rate=\"{}\" complexity=\"0\">\n      <classes>",
            xml(&folder),
            rate(pt.lines),
            rate(pt.branches)
        );
        for (path, f) in files {
            let ft = Report::file_totals(f);
            let _ = writeln!(
                out,
                "        <class name=\"{}\" filename=\"{}\" line-rate=\"{}\" branch-rate=\"{}\" complexity=\"0\">\n          <methods>",
                xml(path),
                xml(path),
                rate(ft.lines),
                rate(ft.branches)
            );
            for ((line, _), (name, hits)) in &f.functions {
                let _ = writeln!(
                    out,
                    "            <method name=\"{}\" signature=\"\" line-rate=\"{}\" branch-rate=\"1\"><lines><line number=\"{line}\" hits=\"{hits}\"/></lines></method>",
                    xml(name),
                    if *hits > 0 { "1" } else { "0" }
                );
            }
            out.push_str("          </methods>\n          <lines>\n");
            for (line, hits) in &f.lines {
                let branches: Vec<&[u64; 2]> = f
                    .branches
                    .iter()
                    .filter(|((l, _), _)| l == line)
                    .map(|(_, b)| b)
                    .collect();
                if branches.is_empty() {
                    let _ = writeln!(
                        out,
                        "            <line number=\"{line}\" hits=\"{hits}\" branch=\"false\"/>"
                    );
                } else {
                    let total = branches.len() * 2;
                    let covered: usize = branches
                        .iter()
                        .map(|b| b.iter().filter(|n| **n > 0).count())
                        .sum();
                    let _ = writeln!(
                        out,
                        "            <line number=\"{line}\" hits=\"{hits}\" branch=\"true\" condition-coverage=\"{}% ({covered}/{total})\"/>",
                        covered * 100 / total
                    );
                }
            }
            out.push_str("          </lines>\n        </class>\n");
        }
        out.push_str("      </classes>\n    </package>\n");
    }
    out.push_str("  </packages>\n</coverage>\n");
    out
}

fn totals_json(t: &Totals) -> serde_json::Value {
    let kind = |(c, n): (u64, u64)| serde_json::json!({ "covered": c, "total": n, "percent": percent((c, n)) });
    serde_json::json!({
        "lines": kind(t.lines),
        "statements": kind(t.lines),
        "branches": kind(t.branches),
        "functions": kind(t.functions),
        "conditions": kind(t.conditions),
    })
}

/// JSON: totals, and per file its totals and counts.
pub fn coverage_json(report: &Report) -> String {
    let files: serde_json::Map<String, serde_json::Value> = report
        .files
        .iter()
        .map(|(path, f)| {
            let lines: serde_json::Map<String, serde_json::Value> = f
                .lines
                .iter()
                .map(|(l, h)| (l.to_string(), serde_json::json!(h)))
                .collect();
            let branches: Vec<serde_json::Value> = f
                .branches
                .iter()
                .map(|((l, n), b)| serde_json::json!({ "line": l, "index": n, "taken": b[0], "notTaken": b[1] }))
                .collect();
            let functions: Vec<serde_json::Value> = f
                .functions
                .iter()
                .map(|((l, _), (name, h))| serde_json::json!({ "line": l, "name": name, "calls": h }))
                .collect();
            (
                path.clone(),
                serde_json::json!({
                    "totals": totals_json(&Report::file_totals(f)),
                    "lines": lines,
                    "branches": branches,
                    "functions": functions,
                }),
            )
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "totals": totals_json(&report.totals()),
        "files": files,
    }))
    .unwrap_or_default()
        + "\n"
}

const HTML_STYLE: &str = r#"
:root { --bg: #0d0f12; --panel: #15181d; --line: #262b33; --text: #d8dde4; --muted: #8a93a0;
  --accent: #e0b04a; --hit: rgba(70, 180, 110, 0.16); --miss: rgba(220, 80, 80, 0.20);
  --part: rgba(224, 176, 74, 0.18); --good: #5fc98a; --mid: #e0b04a; --bad: #e06464; }
@media (prefers-color-scheme: light) {
  :root { --bg: #f6f6f3; --panel: #ffffff; --line: #d9dbe0; --text: #1b1e23; --muted: #626b77;
    --accent: #a5761a; --hit: rgba(40, 150, 80, 0.13); --miss: rgba(200, 50, 50, 0.13);
    --part: rgba(190, 140, 30, 0.16); --good: #23864a; --mid: #a5761a; --bad: #b73a3a; }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--text);
  font: 13px/1.5 "IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
h1, h2 { font-family: "Martian Mono", "IBM Plex Mono", ui-monospace, monospace; font-weight: 600;
  letter-spacing: 0.02em; margin: 0; }
h1 { font-size: 18px; } h2 { font-size: 14px; }
header { padding: 20px 24px; border-bottom: 1px solid var(--line); background: var(--panel); }
header p { color: var(--muted); margin: 4px 0 0; }
main { padding: 16px 24px 48px; max-width: 1200px; }
table { border-collapse: collapse; width: 100%; background: var(--panel); border: 1px solid var(--line); }
th, td { padding: 6px 10px; border-bottom: 1px solid var(--line); text-align: right; white-space: nowrap; }
th:first-child, td:first-child { text-align: left; }
th { color: var(--muted); font-weight: 500; }
a { color: var(--accent); text-decoration: none; } a:hover { text-decoration: underline; }
.good { color: var(--good); } .mid { color: var(--mid); } .bad { color: var(--bad); }
section { margin-top: 28px; }
section h2 { margin-bottom: 8px; }
.src { border: 1px solid var(--line); background: var(--panel); overflow-x: auto; }
.src div { display: flex; white-space: pre; }
.src span { display: inline-block; }
.n { width: 56px; text-align: right; padding-right: 10px; color: var(--muted); border-right: 1px solid var(--line); user-select: none; }
.h { width: 64px; text-align: right; padding-right: 10px; color: var(--muted); border-right: 1px solid var(--line); }
.c { padding-left: 10px; flex: 1; }
.hit { background: var(--hit); } .miss { background: var(--miss); } .part { background: var(--part); }
.b { color: var(--mid); padding-left: 8px; }
"#;

fn class_of(p: f64) -> &'static str {
    if p >= 90.0 {
        "good"
    } else if p >= 60.0 {
        "mid"
    } else {
        "bad"
    }
}

fn pct_cell((c, n): (u64, u64)) -> String {
    let p = percent((c, n));
    format!(
        "<td class=\"{}\">{p:.1}% <span style=\"color:var(--muted)\">{c}/{n}</span></td>",
        class_of(p)
    )
}

/// A self-contained HTML page: totals per file and folder, then every
/// file's source with hit counts and branch markers.
pub fn html(report: &Report, root: &Path, title: &str) -> String {
    let t = report.totals();
    let mut out = format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{} coverage</title><style>{HTML_STYLE}</style></head><body>\n<header><h1>{} coverage</h1><p>lines {:.1}% · branches {:.1}% · functions {:.1}% · conditions {:.1}%</p></header>\n<main>\n<table><tr><th>File</th><th>Lines</th><th>Branches</th><th>Functions</th><th>Conditions</th></tr>\n",
        xml(title),
        xml(title),
        percent(t.lines),
        percent(t.branches),
        percent(t.functions),
        percent(t.conditions)
    );
    let mut folders: std::collections::BTreeMap<String, Totals> = std::collections::BTreeMap::new();
    for (path, f) in &report.files {
        let folder = path.rsplit_once('/').map_or(".", |(d, _)| d).to_string();
        let ft = Report::file_totals(f);
        let e = folders.entry(folder).or_default();
        for (a, b) in [
            (&mut e.lines, ft.lines),
            (&mut e.branches, ft.branches),
            (&mut e.functions, ft.functions),
            (&mut e.conditions, ft.conditions),
        ] {
            a.0 += b.0;
            a.1 += b.1;
        }
    }
    let mut current_folder = String::new();
    for (i, (path, f)) in report.files.iter().enumerate() {
        let folder = path.rsplit_once('/').map_or(".", |(d, _)| d).to_string();
        if folder != current_folder {
            let ft = folders[&folder];
            let _ = writeln!(
                out,
                "<tr><td><strong>{}/</strong></td>{}{}{}{}</tr>",
                xml(&folder),
                pct_cell(ft.lines),
                pct_cell(ft.branches),
                pct_cell(ft.functions),
                pct_cell(ft.conditions)
            );
            current_folder = folder;
        }
        let ft = Report::file_totals(f);
        let _ = writeln!(
            out,
            "<tr><td>&nbsp;&nbsp;<a href=\"#f{i}\">{}</a></td>{}{}{}{}</tr>",
            xml(path),
            pct_cell(ft.lines),
            pct_cell(ft.branches),
            pct_cell(ft.functions),
            pct_cell(ft.conditions)
        );
    }
    out.push_str("</table>\n");
    for (i, (path, f)) in report.files.iter().enumerate() {
        let text = std::fs::read_to_string(root.join(path)).unwrap_or_default();
        let _ = writeln!(
            out,
            "<section id=\"f{i}\"><h2>{}</h2><div class=\"src\">",
            xml(path)
        );
        for (n, line) in text.lines().enumerate() {
            let ln = n as u32 + 1;
            let branches: Vec<&[u64; 2]> = f
                .branches
                .iter()
                .filter(|((l, _), _)| *l == ln)
                .map(|(_, b)| b)
                .collect();
            let missed = branches
                .iter()
                .map(|b| b.iter().filter(|x| **x == 0).count())
                .sum::<usize>();
            let (cls, hits) = match f.lines.get(&ln) {
                Some(0) => ("miss", "0".to_string()),
                Some(h) if missed > 0 => ("part", h.to_string()),
                Some(h) => ("hit", h.to_string()),
                None => ("", String::new()),
            };
            let marker = if branches.is_empty() {
                String::new()
            } else {
                format!(
                    "<span class=\"b\" title=\"branches taken / not taken\">{}</span>",
                    branches
                        .iter()
                        .map(|b| format!("[{}|{}]", b[0], b[1]))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            };
            let _ = writeln!(
                out,
                "<div class=\"{cls}\"><span class=\"n\">{ln}</span><span class=\"h\">{hits}</span><span class=\"c\">{}{marker}</span></div>",
                xml(line)
            );
        }
        out.push_str("</div></section>\n");
    }
    out.push_str("</main></body></html>\n");
    out
}

/// The terminal summary: totals, and the files not fully covered with
/// their uncovered lines.
pub fn summary(report: &Report) -> String {
    let t = report.totals();
    let mut out = format!(
        "coverage: lines {:.1}% ({}/{}), branches {:.1}% ({}/{}), functions {:.1}% ({}/{}), conditions {:.1}% ({}/{})\n",
        percent(t.lines),
        t.lines.0,
        t.lines.1,
        percent(t.branches),
        t.branches.0,
        t.branches.1,
        percent(t.functions),
        t.functions.0,
        t.functions.1,
        percent(t.conditions),
        t.conditions.0,
        t.conditions.1
    );
    for (path, f) in &report.files {
        let missed: Vec<u32> = f
            .lines
            .iter()
            .filter(|(_, h)| **h == 0)
            .map(|(l, _)| *l)
            .collect();
        if missed.is_empty() {
            continue;
        }
        // Ranges: 3-5, 9.
        let mut ranges: Vec<String> = Vec::new();
        let mut start = missed[0];
        let mut prev = missed[0];
        for &l in &missed[1..] {
            if l != prev + 1 {
                ranges.push(if start == prev {
                    start.to_string()
                } else {
                    format!("{start}-{prev}")
                });
                start = l;
            }
            prev = l;
        }
        ranges.push(if start == prev {
            start.to_string()
        } else {
            format!("{start}-{prev}")
        });
        let ft = Report::file_totals(f);
        let _ = writeln!(
            out,
            "  {path}: lines {:.1}%, not run: {}",
            percent(ft.lines),
            ranges.join(", ")
        );
    }
    out
}
