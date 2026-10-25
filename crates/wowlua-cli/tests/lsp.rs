//! `wowlua lsp` as an editor runs it (decisions T370, T373): checks in its
//! long-lived worker, the unsaved text's error reported, a local's
//! definition found, a file created on disk checked, and a clean exit.

use serde_json::{Value as Json, json};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn send(stdin: &mut impl Write, message: &Json) {
    wowlua_lsp::transport::write(stdin, message).expect("written");
}

#[test]
fn the_server_checks_in_a_child_and_answers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "lsp-child", "version": "0.1.0", "wowlua": "^0.10" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    // (as an editor spells it: a temporary folder may be reached through a
    // symbolic link, macOS's `/var`)
    let path = dir.path().join("src/main.wowlua");
    std::fs::write(&path, "print(1)\n").unwrap();
    let uri = wowlua_lsp::text::uri_of(&path);
    let mut server = Command::new(assert_cmd::cargo::cargo_bin("wowlua"))
        .arg("lsp")
        .current_dir(dir.path())
        .env("WOWLUA_HOME", dir.path().join(".wowlua-home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("wowlua lsp starts");
    let mut stdin = server.stdin.take().unwrap();
    let stdout = server.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = std::io::BufReader::new(stdout);
        while let Ok(Some(m)) = wowlua_lsp::transport::read(&mut input) {
            if tx.send(m).is_err() {
                return;
            }
        }
    });
    let wait = |what: &str, f: &dyn Fn(&Json) -> bool| -> Json {
        loop {
            match rx.recv_timeout(Duration::from_secs(120)) {
                Ok(m) if f(&m) => return m,
                Ok(_) => {}
                Err(_) => panic!("no {what}"),
            }
        }
    };
    send(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "capabilities": {}, "rootUri": wowlua_lsp::text::uri_of(dir.path()) } }),
    );
    wait("the initialize reply", &|m| m["id"] == 1);
    let text = "local count: number = \"none\"\nprint(count)\n";
    send(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": text } } }),
    );
    let published = wait("the diagnostics", &|m| {
        m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri.as_str()
    });
    let items = published["params"]["diagnostics"].as_array().unwrap();
    assert!(items.iter().any(|d| d["code"] == "E2001"), "{items:#?}");
    // `count` in `print(count)` leads to its declaration
    send(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition", "params": {
            "textDocument": { "uri": uri }, "position": { "line": 1, "character": 8 } } }),
    );
    let def = wait("the definition", &|m| m["id"] == 2);
    assert_eq!(def["result"][0]["range"]["start"]["line"], 0, "{def}");
    assert_eq!(def["result"][0]["range"]["start"]["character"], 6, "{def}");
    // a file created on disk: checked once the editor says so
    let other = path.with_file_name("other.wowlua");
    std::fs::write(&other, "local b: string = 2\nprint(b)\n").unwrap();
    let other_uri = wowlua_lsp::text::uri_of(&other);
    send(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": {
            "changes": [{ "uri": other_uri, "type": 1 }] } }),
    );
    let published = wait("the new file's diagnostics", &|m| {
        m["method"] == "textDocument/publishDiagnostics"
            && m["params"]["uri"] == other_uri.as_str()
            && !m["params"]["diagnostics"].as_array().unwrap().is_empty()
    });
    let items = published["params"]["diagnostics"].as_array().unwrap();
    assert!(items.iter().any(|d| d["code"] == "E2001"), "{items:#?}");
    send(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" }),
    );
    wait("the shutdown reply", &|m| m["id"] == 3);
    send(&mut stdin, &json!({ "jsonrpc": "2.0", "method": "exit" }));
    let status = server.wait().unwrap();
    assert!(status.success(), "{status}");
    // the disk was never written
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "print(1)\n");
}
