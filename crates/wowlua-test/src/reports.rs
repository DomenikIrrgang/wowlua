//! Reports of a test run (spec Tooling §16.1.5, §16.1.6): test results as
//! JUnit XML, TAP, JSON or GitHub annotations; coverage as lcov, Cobertura
//! XML, JSON or a self-contained HTML page in the wowlua design system.

use std::fmt::Write;
use std::path::PathBuf;

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

const HTML_STYLE: &str = include_str!("coverage_html/style.css");
const HTML_APP: &str = include_str!("coverage_html/app.js");
/// wowlua's mark as the tab's icon, in the dark or light colours.
const HTML_ICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 48 48'%3E%3Cstyle%3Epath%7Bstroke:%23ece6d6%7Dg%7Bfill:%23ffb547%7D@media (prefers-color-scheme:light)%7Bpath%7Bstroke:%231a1813%7Dg%7Bfill:%23975200%7D%7D%3C/style%3E%3Cpath d='M9.5 5H4V43H9.5M38.5 5H44V43H38.5' fill='none' stroke-width='3.5' stroke-linecap='square'/%3E%3Cg%3E%3Cpath style='stroke:none' transform='translate(12.14 31.63)' d='M0.36 -11.16H3.33L3.73 -6.48L3.87 -2.72H4.23L4.9 -11.16H8.08L8.69 -2.72H9.05L9.18 -6.48L9.56 -11.16H12.24L11 0.0H7.11L6.52 -8.41H6.16L5.51 0.0H1.6ZM20.12 -15.26V-2.84H23.36V0.0H13.64V-2.84H16.88V-12.42H13.64V-15.26Z'/%3E%3C/g%3E%3C/svg%3E";

/// A self-contained HTML page (spec Tooling §16.1.6): the index (totals,
/// where to test next, every file, sortable and filterable) and each file
/// (its totals, the lines that never ran, its functions, its source with
/// hits and branches), drawn by the page's script from the counts and
/// sources embedded as JSON. A file's source is read from the first of
/// `roots` (the workspace's addons) that has it.
pub fn html(report: &Report, roots: &[PathBuf], title: &str) -> String {
    let flat = |t: &Totals| {
        [t.lines, t.branches, t.functions, t.conditions]
            .iter()
            .flat_map(|(c, n)| [*c, *n])
            .collect::<Vec<u64>>()
    };
    let files: Vec<serde_json::Value> = report
        .files
        .iter()
        .map(|(path, f)| {
            let source = roots
                .iter()
                .find_map(|r| std::fs::read_to_string(r.join(path)).ok())
                .unwrap_or_default();
            let lines: Vec<u64> = f
                .lines
                .iter()
                .flat_map(|(l, h)| [u64::from(*l), *h])
                .collect();
            let branches: Vec<u64> = f
                .branches
                .iter()
                .flat_map(|((l, _), b)| [u64::from(*l), b[0], b[1]])
                .collect();
            let functions: Vec<serde_json::Value> = f
                .functions
                .iter()
                .flat_map(|((l, _), (name, h))| {
                    [
                        serde_json::json!(l),
                        serde_json::json!(name),
                        serde_json::json!(h),
                    ]
                })
                .collect();
            serde_json::json!({
                "p": path,
                "t": flat(&Report::file_totals(f)),
                "s": source,
                "l": lines,
                "b": branches,
                "f": functions,
            })
        })
        .collect();
    let data = serde_json::json!({
        "title": title,
        "totals": flat(&report.totals()),
        "files": files,
    });
    // inside <script>: no `</script>` (nor `<!--`) can end it early
    let data = serde_json::to_string(&data)
        .unwrap_or_default()
        .replace('<', "\\u003c");
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{t} coverage</title><link rel=\"icon\" href=\"{HTML_ICON}\"><style>{HTML_STYLE}</style></head><body>\n<div id=\"app\"><header><h1>{t} coverage</h1></header><main><p>The report needs JavaScript.</p></main></div>\n<script type=\"application/json\" id=\"data\">{data}</script>\n<script>{HTML_APP}</script>\n</body></html>\n",
        t = xml(title),
    )
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
