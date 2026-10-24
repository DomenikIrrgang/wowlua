//! The language server end to end: a package's file opened, edited without
//! saving, formatted and outlined.

use serde_json::{Value as Json, json};
use std::sync::mpsc;

const ON_DISK: &str = "print(1)\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "lsp-demo", "version": "0.1.0", "wowlua": "^0.10" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.wowlua"), ON_DISK).unwrap();
    dir
}

/// The messages the server wrote, read back.
fn written(out: &[u8]) -> Vec<Json> {
    let mut input = std::io::Cursor::new(out.to_vec());
    let mut all = Vec::new();
    while let Ok(Some(m)) = wowlua_lsp::transport::read(&mut input) {
        all.push(m);
    }
    all
}

/// The client's turns: each sends its messages, then pauses long enough
/// for the edits to settle and be checked; then `shutdown` and `exit`.
fn run(turns: Vec<Vec<Json>>) -> Vec<Json> {
    let (tx, rx) = mpsc::channel();
    let sender = std::thread::spawn(move || {
        for turn in turns {
            for m in turn {
                tx.send(m).unwrap();
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        tx.send(json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }))
            .unwrap();
        tx.send(json!({ "jsonrpc": "2.0", "method": "exit" }))
            .unwrap();
    });
    let mut out = Vec::new();
    let clean = wowlua_lsp::serve(&rx, &mut out).unwrap();
    assert!(clean, "exit came after shutdown");
    sender.join().unwrap();
    written(&out)
}

#[test]
fn an_unsaved_edit_is_checked_formatted_and_outlined() {
    let dir = project();
    let path = std::fs::canonicalize(dir.path().join("src/main.wowlua")).unwrap();
    let uri = wowlua_lsp::text::uri_of(&path);
    let broken =
        "local x: number = \"one\"\nlocal function  helper() return x end\nprint(helper())\n";
    let fixed = "local x: number = 1\nprint(x)\n";
    let out = run(vec![
        vec![
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
            json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
            json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": ON_DISK } } }),
            json!({ "jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
            "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": broken }] } }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/formatting", "params": {
            "textDocument": { "uri": uri }, "options": { "tabSize": 4, "insertSpaces": false } } }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "textDocument/documentSymbol", "params": { "textDocument": { "uri": uri } } }),
        ],
        vec![
            json!({ "jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
            "textDocument": { "uri": uri, "version": 3 }, "contentChanges": [{ "text": fixed }] } }),
        ],
        vec![
            json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
        ],
    ]);
    let init = out
        .iter()
        .find(|m| m["id"] == 1)
        .expect("initialize answered");
    assert_eq!(
        init["result"]["capabilities"]["documentFormattingProvider"],
        true
    );
    // the unsaved text's error, on its line
    let diags: Vec<&Json> = out
        .iter()
        .filter(|m| {
            m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri.as_str()
        })
        .collect();
    let first = diags
        .first()
        .unwrap_or_else(|| panic!("no diagnostics for {uri}: {out:#?}"));
    let items = first["params"]["diagnostics"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|d| d["code"] == "E2001" && d["range"]["start"]["line"] == 0),
        "{items:#?}"
    );
    // formatting: the double space gone
    let fmt = out
        .iter()
        .find(|m| m["id"] == 2)
        .expect("formatting answered");
    let edit = &fmt["result"][0];
    assert!(
        edit["newText"]
            .as_str()
            .unwrap()
            .contains("local function helper()"),
        "{fmt}"
    );
    // the outline: x and helper
    let syms = out.iter().find(|m| m["id"] == 3).expect("symbols answered");
    let names: Vec<&str> = syms["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["x", "helper"]);
    // fixed: the file's diagnostics cleared
    let last = diags.last().unwrap();
    assert_eq!(last["params"]["diagnostics"], json!([]), "{diags:#?}");
    // the disk was never written, nor a lockfile
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ON_DISK);
    assert!(!dir.path().join("wowlua.lock").exists());
    // closed: the disk's text is read again
    assert_eq!(
        wowlua_driver::overlay::read(&path).unwrap(),
        ON_DISK.as_bytes()
    );
}
