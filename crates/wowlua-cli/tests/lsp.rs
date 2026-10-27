//! `wowlua lsp` as an editor runs it (decisions T370, T373): checks in its
//! long-lived worker, the unsaved text's error reported, a local's
//! definition, references and rename found (T389), a file created on disk
//! checked, and a clean exit; a package that can't be loaded told (T390).

use serde_json::{Value as Json, json};
use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// `wowlua lsp` started in `dir`, initialized.
struct Server {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Json>,
}

impl Server {
    fn start(dir: &Path) -> Server {
        let mut child = Command::new(assert_cmd::cargo::cargo_bin("wowlua"))
            .arg("lsp")
            .current_dir(dir)
            .env("WOWLUA_HOME", dir.join(".wowlua-home"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("wowlua lsp starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut input = std::io::BufReader::new(stdout);
            while let Ok(Some(m)) = wowlua_lsp::transport::read(&mut input) {
                if tx.send(m).is_err() {
                    return;
                }
            }
        });
        let mut s = Server { child, stdin, rx };
        s.send(
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "capabilities": {}, "rootUri": wowlua_lsp::text::uri_of(dir) } }),
        );
        s.wait("the initialize reply", &|m| m["id"] == 1);
        s
    }

    fn send(&mut self, message: &Json) {
        wowlua_lsp::transport::write(&mut self.stdin, message).expect("written");
        self.stdin.flush().unwrap();
    }

    fn wait(&self, what: &str, f: &dyn Fn(&Json) -> bool) -> Json {
        loop {
            match self.rx.recv_timeout(Duration::from_secs(120)) {
                Ok(m) if f(&m) => return m,
                Ok(_) => {}
                Err(_) => panic!("no {what}"),
            }
        }
    }

    /// `shutdown`, `exit`: whether it stopped cleanly.
    fn finish(mut self) -> bool {
        self.send(&json!({ "jsonrpc": "2.0", "id": 999, "method": "shutdown" }));
        self.wait("the shutdown reply", &|m| m["id"] == 999);
        self.send(&json!({ "jsonrpc": "2.0", "method": "exit" }));
        self.child.wait().unwrap().success()
    }
}

#[test]
fn the_server_checks_in_a_child_and_answers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "lsp-child", "version": "0.1.0", "wowlua": "^0.13" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    // (as an editor spells it: a temporary folder may be reached through a
    // symbolic link, macOS's `/var`)
    let path = dir.path().join("src/main.wowlua");
    std::fs::write(&path, "print(1)\n").unwrap();
    let uri = wowlua_lsp::text::uri_of(&path);
    let mut s = Server::start(dir.path());
    let text = "local count: number = \"none\"\nprint(count)\n";
    s.send(
        &json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": text } } }),
    );
    let published = s.wait("the diagnostics", &|m| {
        m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri.as_str()
    });
    let items = published["params"]["diagnostics"].as_array().unwrap();
    assert!(items.iter().any(|d| d["code"] == "E2001"), "{items:#?}");
    // `count` in `print(count)` leads to its declaration
    let at = json!({ "line": 1, "character": 8 });
    s.send(
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition", "params": {
            "textDocument": { "uri": uri }, "position": at } }),
    );
    let def = s.wait("the definition", &|m| m["id"] == 2);
    assert_eq!(def["result"][0]["range"]["start"]["line"], 0, "{def}");
    assert_eq!(def["result"][0]["range"]["start"]["character"], 6, "{def}");
    // its references, and a rename of both names
    s.send(&json!({ "jsonrpc": "2.0", "id": 3, "method": "textDocument/references", "params": {
            "textDocument": { "uri": uri }, "position": at, "context": { "includeDeclaration": true } } }));
    let refs = s.wait("the references", &|m| m["id"] == 3);
    let lines: Vec<&Json> = refs["result"]
        .as_array()
        .unwrap_or_else(|| panic!("{refs}"))
        .iter()
        .map(|l| &l["range"]["start"]["line"])
        .collect();
    assert_eq!(lines, [0, 1], "{refs}");
    s.send(
        &json!({ "jsonrpc": "2.0", "id": 4, "method": "textDocument/rename", "params": {
            "textDocument": { "uri": uri }, "position": at, "newName": "total" } }),
    );
    let renamed = s.wait("the rename", &|m| m["id"] == 4);
    let edits = renamed["result"]["changes"][uri.as_str()]
        .as_array()
        .unwrap_or_else(|| panic!("{renamed}"));
    assert_eq!(edits.len(), 2, "{renamed}");
    assert!(edits.iter().all(|e| e["newText"] == "total"), "{renamed}");
    // a file created on disk: checked once the editor says so
    let other = path.with_file_name("other.wowlua");
    std::fs::write(&other, "local b: string = 2\nprint(b)\n").unwrap();
    let other_uri = wowlua_lsp::text::uri_of(&other);
    s.send(
        &json!({ "jsonrpc": "2.0", "method": "workspace/didChangeWatchedFiles", "params": {
            "changes": [{ "uri": other_uri, "type": 1 }] } }),
    );
    let published = s.wait("the new file's diagnostics", &|m| {
        m["method"] == "textDocument/publishDiagnostics"
            && m["params"]["uri"] == other_uri.as_str()
            && !m["params"]["diagnostics"].as_array().unwrap().is_empty()
    });
    let items = published["params"]["diagnostics"].as_array().unwrap();
    assert!(items.iter().any(|d| d["code"] == "E2001"), "{items:#?}");
    assert!(s.finish());
    // the disk was never written
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "print(1)\n");
}

#[test]
fn a_package_that_cannot_be_loaded_is_an_error_message() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "too-old", "version": "0.1.0", "wowlua": "^0.9" } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let path = dir.path().join("src/main.wowlua");
    std::fs::write(&path, "print(1)\n").unwrap();
    let mut s = Server::start(dir.path());
    s.send(&json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": { "uri": wowlua_lsp::text::uri_of(&path), "languageId": "wowlua", "version": 1, "text": "print(1)\n" } } }));
    let shown = s.wait("the message", &|m| m["method"] == "window/showMessage");
    let message = shown["params"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("too-old can't be checked: this package requires wowlua ^0.9"),
        "{message}"
    );
    assert!(s.finish());
}
