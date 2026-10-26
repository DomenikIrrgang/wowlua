//! The diagnostic explanations' examples (`crates/wowlua-diagnostics/
//! explanations/`), checked as `wowlua check` checks a package: each `fails`
//! example reports its code, each `compiles` example no error and not its
//! code. One test: the files' host is the program's.

use serde_json::Value as Json;
use wowlua_diagnostics::{all_codes, explanation};
use wowlua_web::{Playground, TEMPLATE};

#[test]
fn every_example_reports_its_code_or_compiles() {
    let mut p = Playground::new();
    let mut failures = Vec::new();
    let mut checked = 0;
    for info in all_codes() {
        let Some(e) = explanation(info.code) else {
            continue;
        };
        for (i, example) in e.examples.iter().enumerate() {
            // The template's manifest, unless the example brings its own.
            let mut files: Vec<(String, String)> = TEMPLATE
                .iter()
                .filter(|(path, _)| *path == "wowlua.json")
                .map(|(path, text)| (path.to_string(), text.to_string()))
                .collect();
            for f in &example.files {
                files.retain(|(path, _)| *path != f.path);
                files.push((f.path.clone(), f.text.clone()));
            }
            p.reset(files);
            let found = p.check();
            let found = found.as_array().cloned().unwrap_or_default();
            let codes: Vec<&str> = found.iter().filter_map(|d| d["code"].as_str()).collect();
            let errors = found.iter().filter(|d| d["severity"] == "error").count();
            let reported = codes.contains(&info.code);
            let ok = if example.compiles {
                !reported && errors == 0
            } else {
                reported
            };
            checked += 1;
            if !ok {
                let verdict = if example.compiles {
                    "compiles"
                } else {
                    "fails"
                };
                failures.push(format!(
                    "{} example {} ({verdict}): got {}",
                    info.code,
                    i + 1,
                    messages(&found)
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} examples are wrong:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn messages(found: &[Json]) -> String {
    if found.is_empty() {
        return "no diagnostics".into();
    }
    found
        .iter()
        .map(|d| {
            format!(
                "{} {}",
                d["code"].as_str().unwrap_or("-"),
                d["message"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}
