//! A check that fails (here: a worker that ends without an answer) is
//! reported, and the server goes on answering (decision T370).

mod common;

use common::Client;
use serde_json::json;

#[cfg(unix)]
#[test]
fn a_failed_check_is_reported_and_the_server_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "lsp-fail", "version": "0.1.0", "wowlua": "^0.11" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let path = wowlua_lsp::text::canonical(&dir.path().join("src/main.wowlua"));
    std::fs::write(&path, "local function f() end\n").unwrap();
    let uri = wowlua_lsp::text::uri_of(&path);
    // `false` ends at once, writing nothing
    let mut c = Client::start_with(wowlua_lsp::Checker::Worker("false".into()));
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": "local function f() end\n" } }),
    );
    let logged = c.wait("the failure logged", |m| m["method"] == "window/logMessage");
    assert!(
        logged["params"]["message"]
            .as_str()
            .unwrap()
            .contains("failed"),
        "{logged}"
    );
    // still answering, also go to definition (which waited for the check)
    let syms = c.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
    );
    assert_eq!(syms["result"][0]["name"], "f", "{syms}");
    let def = c.request(
        "textDocument/definition",
        json!({ "textDocument": { "uri": uri }, "position": { "line": 0, "character": 16 } }),
    );
    assert!(def["result"].is_null(), "{def}");
    assert!(c.finish());
}
