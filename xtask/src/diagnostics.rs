//! `cargo xtask diagnostics`: the diagnostic explanations as JSON for the
//! website (`docs/reference/diagnostics.json`). The codes and summaries stay
//! in Appendix B, which the website reads itself; this adds each explained
//! code's text, examples (a failing one with what `wowlua check` prints for
//! it, from the playground's in-memory package) and related codes, and the
//! areas' short names.

use serde_json::{Map, Value as Json, json};
use std::path::Path;

pub const PATH: &str = "docs/reference/diagnostics.json";

pub fn generate(_root: &Path) -> String {
    let playground = wowlua_web::Playground::new();
    let areas: Vec<Json> = wowlua_diagnostics::areas()
        .into_iter()
        .map(|(range, short)| json!({ "range": range, "short": short }))
        .collect();
    let mut codes = Map::new();
    for info in wowlua_diagnostics::all_codes() {
        let Some(e) = wowlua_diagnostics::explanation(info.code) else {
            continue;
        };
        let examples: Vec<Json> = e
            .examples
            .iter()
            .map(|x| {
                let files: Vec<Json> = x
                    .files
                    .iter()
                    .map(|f| json!({ "path": f.path, "lang": f.lang, "text": f.text }))
                    .collect();
                let output = if x.compiles {
                    None
                } else {
                    output(&playground, info.code, &x.files)
                };
                json!({ "compiles": x.compiles, "files": files, "output": output })
            })
            .collect();
        codes.insert(
            info.code.to_string(),
            json!({ "area": e.area, "text": e.text, "examples": examples, "related": e.related }),
        );
    }
    // The playground put its in-memory files in place of the disk; the other
    // tasks read the repository again.
    wowlua_driver::host::install(None);
    let mut out = serde_json::to_string_pretty(&json!({ "areas": areas, "codes": codes }))
        .expect("the explanations serialize");
    out.push('\n');
    out
}

/// What `wowlua check` prints for an example's diagnostic with `code`.
fn output(
    playground: &wowlua_web::Playground,
    code: &str,
    files: &[wowlua_diagnostics::ExampleFile],
) -> Option<String> {
    let mut package: Vec<(String, String)> = wowlua_web::TEMPLATE
        .iter()
        .filter(|(path, _)| *path == "wowlua.json")
        .map(|(path, text)| (path.to_string(), text.to_string()))
        .collect();
    for f in files {
        package.retain(|(path, _)| *path != f.path);
        package.push((f.path.clone(), f.text.clone()));
    }
    playground.reset(package);
    let found = playground.check();
    found
        .as_array()?
        .iter()
        .find(|d| d["code"] == code)
        .and_then(|d| d["text"].as_str())
        .map(|t| t.trim_end().to_string())
}
