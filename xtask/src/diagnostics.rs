//! `cargo xtask diagnostics`: the diagnostic explanations as JSON for the
//! website (`docs/reference/diagnostics.json`). The codes and summaries stay
//! in Appendix B, which the website reads itself; this adds each explained
//! code's text, examples and related codes, and the areas' short names.

use serde_json::{Map, Value as Json, json};
use std::path::Path;

pub const PATH: &str = "docs/reference/diagnostics.json";

pub fn generate(_root: &Path) -> String {
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
                json!({ "compiles": x.compiles, "files": files })
            })
            .collect();
        codes.insert(
            info.code.to_string(),
            json!({ "area": e.area, "text": e.text, "examples": examples, "related": e.related }),
        );
    }
    let mut out = serde_json::to_string_pretty(&json!({ "areas": areas, "codes": codes }))
        .expect("the explanations serialize");
    out.push('\n');
    out
}
