//! The language server end to end: a package's file opened, edited without
//! saving, formatted and outlined; its diagnostics cleared once fixed.

mod common;

use common::Client;
use serde_json::json;

const ON_DISK: &str = "print(1)\n";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "lsp-demo", "version": "0.1.0", "wowlua": "^0.11" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.wowlua"), ON_DISK).unwrap();
    dir
}

#[test]
fn an_unsaved_edit_is_checked_formatted_and_outlined() {
    let dir = project();
    let path = wowlua_lsp::text::canonical(&dir.path().join("src/main.wowlua"));
    let uri = wowlua_lsp::text::uri_of(&path);
    let broken =
        "local x: number = \"one\"\nlocal function  helper() return x end\nprint(helper())\n";
    let fixed = "local x: number = 1\nprint(x)\n";
    let mut c = Client::start();
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": ON_DISK } }),
    );
    // the open file checked: nothing in it
    c.diagnostics(&uri, |d| d.is_empty());
    c.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": broken }] }),
    );
    // the unsaved text's error, on its line
    let items = c.diagnostics(&uri, |d| !d.is_empty());
    assert!(
        items
            .iter()
            .any(|d| d["code"] == "E2001" && d["range"]["start"]["line"] == 0),
        "{items:#?}"
    );
    // formatting: the double space gone
    let fmt = c.request(
        "textDocument/formatting",
        json!({ "textDocument": { "uri": uri }, "options": { "tabSize": 4, "insertSpaces": false } }),
    );
    assert!(
        fmt["result"][0]["newText"]
            .as_str()
            .unwrap()
            .contains("local function helper()"),
        "{fmt}"
    );
    // the outline: x and helper
    let syms = c.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
    );
    let names: Vec<&str> = syms["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["x", "helper"]);
    // fixed: the file's diagnostics cleared
    c.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 3 }, "contentChanges": [{ "text": fixed }] }),
    );
    c.diagnostics(&uri, |d| d.is_empty());
    c.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": uri } }),
    );
    // an unknown request is answered
    let unknown = c.request("textDocument/unknownThing", json!({}));
    assert_eq!(unknown["error"]["code"], -32601, "{unknown}");
    assert!(c.finish(), "exit came after shutdown");
    // the disk was never written, nor a lockfile
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ON_DISK);
    assert!(!dir.path().join("wowlua.lock").exists());
    // closed: the disk's text is read again
    assert_eq!(
        wowlua_driver::overlay::read(&path).unwrap(),
        ON_DISK.as_bytes()
    );
}

#[test]
fn ranged_edits_and_files_changed_on_disk_are_checked() {
    let dir = project();
    let path = wowlua_lsp::text::canonical(&dir.path().join("src/main.wowlua"));
    let uri = wowlua_lsp::text::uri_of(&path);
    let mut c = Client::start();
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": "local a: number = 1\nprint(a)\n" } }),
    );
    c.diagnostics(&uri, |d| d.is_empty());
    // `1` replaced by a string: only that range is sent
    c.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{
            "range": { "start": { "line": 0, "character": 18 }, "end": { "line": 0, "character": 19 } },
            "text": "\"one\"",
        }] }),
    );
    let items = c.diagnostics(&uri, |d| !d.is_empty());
    assert!(items.iter().any(|d| d["code"] == "E2001"), "{items:#?}");
    let syms = c.request(
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
    );
    assert_eq!(syms["result"][0]["name"], "a", "{syms}");
    // another file of the package, broken on disk: reported once the
    // editor says it changed
    let other = wowlua_lsp::text::canonical(&dir.path().join("src")).join("other.wowlua");
    std::fs::write(&other, "local b: string = 2\n").unwrap();
    let other_uri = wowlua_lsp::text::uri_of(&other);
    c.notify(
        "workspace/didChangeWatchedFiles",
        json!({ "changes": [{ "uri": other_uri, "type": 1 }] }),
    );
    let items = c.diagnostics(&other_uri, |d| !d.is_empty());
    assert!(items.iter().any(|d| d["code"] == "E2001"), "{items:#?}");
    assert!(c.finish());
}

#[test]
fn an_edit_checks_its_file_first_and_the_package_once_quiet() {
    let dir = project();
    let src = wowlua_lsp::text::canonical(&dir.path().join("src"));
    // another file of the package, with an error, never opened
    let other = src.join("other.wowlua");
    std::fs::write(&other, "local b: string = 2\nprint(b)\n").unwrap();
    let other_uri = wowlua_lsp::text::uri_of(&other);
    let path = src.join("main.wowlua");
    let uri = wowlua_lsp::text::uri_of(&path);
    let mut c = Client::start();
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": ON_DISK } }),
    );
    // first the open file alone, then (no edits for a while) the package
    c.diagnostics(&uri, |d| d.is_empty());
    let items = c.diagnostics(&other_uri, |d| !d.is_empty());
    assert!(items.iter().any(|d| d["code"] == "E2001"), "{items:#?}");
    // an edit's first pass speaks for the edited file only: the other
    // file's error stays
    c.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": "local a: number = \"x\"\nprint(a)\n" }] }),
    );
    c.diagnostics(&uri, |d| d.iter().any(|x| x["code"] == "E2001"));
    assert!(
        !c.seen
            .iter()
            .any(|m| m["method"] == "textDocument/publishDiagnostics"
                && m["params"]["uri"] == other_uri.as_str()
                && m["params"]["diagnostics"] == json!([])),
        "the other file's diagnostics were cleared: {:#?}",
        c.seen
    );
    assert!(c.finish());
}
