//! What an editor draws from the server (spec §16.2.9): semantic tokens,
//! folding, inlay hints after a check, quick fixes from the diagnostics'
//! fixes, and the references lenses.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

mod common;

use common::Client;
use serde_json::json;

const TEXT: &str = "local function helper(count: number): number\n    return count * 2\nend\n\nlocal doubled = helper(2)\nlocal unused = doubled\nprint(helper(doubled))\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "lsp-editor", "version": "0.1.0", "wowlua": "^0.13" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.wowlua"), TEXT).unwrap();
    dir
}

#[test]
fn the_editor_gets_tokens_folds_hints_fixes_and_lenses() {
    let dir = project();
    let path = wowlua_lsp::text::canonical(&dir.path().join("src/main.wowlua"));
    let uri = wowlua_lsp::text::uri_of(&path);
    let mut c = Client::start();
    let init = c.request(
        "initialize",
        json!({ "rootUri": wowlua_lsp::text::uri_of(dir.path()) }),
    );
    let caps = &init["result"]["capabilities"];
    assert_eq!(caps["inlayHintProvider"], true, "{caps:#?}");
    assert_eq!(caps["semanticTokensProvider"]["full"], true, "{caps:#?}");
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": TEXT } }),
    );
    // The check: `unused` is never read (W1003, with a fix).
    let items = c.diagnostics(&uri, |d| d.iter().any(|d| d["code"] == "W1003"));
    let unused = items.iter().find(|d| d["code"] == "W1003").unwrap().clone();
    assert!(
        unused["data"]["fixes"][0]["edits"][0]["uri"].is_string(),
        "{unused:#?}"
    );

    let actions = c.request(
        "textDocument/codeAction",
        json!({ "textDocument": { "uri": uri }, "range": unused["range"], "context": { "diagnostics": [unused] } }),
    );
    assert_eq!(actions["result"][0]["kind"], "quickfix", "{actions:#?}");
    assert!(
        actions["result"][0]["edit"]["changes"][&uri].is_array(),
        "{actions:#?}"
    );

    let tokens = c.request(
        "textDocument/semanticTokens/full",
        json!({ "textDocument": { "uri": uri } }),
    );
    assert!(
        !tokens["result"]["data"].as_array().unwrap().is_empty(),
        "{tokens:#?}"
    );

    let folds = c.request(
        "textDocument/foldingRange",
        json!({ "textDocument": { "uri": uri } }),
    );
    assert!(
        folds["result"]
            .as_array()
            .unwrap()
            .contains(&json!({ "startLine": 0, "endLine": 1 })),
        "{folds:#?}"
    );

    // `doubled` is declared without a type: the check's is shown.
    let hints = c.request(
        "textDocument/inlayHint",
        json!({ "textDocument": { "uri": uri }, "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 9, "character": 0 } } }),
    );
    let hints = hints["result"].as_array().unwrap().clone();
    assert!(
        hints
            .iter()
            .any(|h| h["label"] == ": number" && h["position"]["line"] == 4),
        "{hints:#?}"
    );

    // `helper` is called twice.
    let lenses = c.request(
        "textDocument/codeLens",
        json!({ "textDocument": { "uri": uri } }),
    );
    assert_eq!(
        lenses["result"][0]["command"]["title"], "2 references",
        "{lenses:#?}"
    );
}
