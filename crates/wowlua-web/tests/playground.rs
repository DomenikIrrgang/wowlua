//! The playground's API as a page uses it (spec M14 S2): files in memory,
//! check, build, format, share links and the language server. One test:
//! the files' host is the program's.

use serde_json::{Value as Json, json};
use wowlua_web::Playground;

const URI: &str = "file:///playground/src/main.wowlua";

/// The messages of one method among `messages`.
fn of<'a>(messages: &'a [Json], method: &str) -> Vec<&'a Json> {
    messages.iter().filter(|m| m["method"] == method).collect()
}

#[test]
fn a_page_edits_checks_builds_shares_and_asks_the_language_server() {
    let mut p = Playground::new();
    p.set_time(1_800_000_000_000.0, 0.0);

    // the template builds
    let built = p.build("dev");
    assert_eq!(built["ok"], true, "{built}");
    let files: Vec<&str> = built["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(files.contains(&"Playground.toc"), "{files:?}");
    let main = built["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "src/main.lua")
        .unwrap_or_else(|| panic!("{files:?}"));
    assert!(
        main["text"].as_str().unwrap().contains("PLAYER_LOGIN"),
        "{main}"
    );

    // a type error, where it is
    p.write_file("src/main.wowlua", "local n: number = \"one\"\nprint(n)\n");
    let found = p.check();
    let error = &found[0];
    assert_eq!(error["code"], "E2001", "{found}");
    assert_eq!(error["file"], "src/main.wowlua");
    assert_eq!(
        error["range"]["start"],
        json!({ "line": 0, "character": 6 })
    );

    // formatting
    p.write_file(
        "src/util.wowlua",
        "export function twice(x: number): number\n    return x  *  2\nend\n",
    );
    let formatted = p.format("src/util.wowlua");
    assert!(
        formatted["text"].as_str().unwrap().contains("return x * 2"),
        "{formatted}"
    );

    // a link gives the files back
    let link = p.share(None);
    p.reset([("wowlua.json".to_string(), "{}".to_string())]);
    assert!(
        p.open_shared(&format!("https://wowlua.dev/playground#{link}"))
            .is_some()
    );
    assert!(p.files().iter().any(|(path, _)| path == "src/util.wowlua"));

    // the language server: the open file's diagnostics, then its hover
    let mut out = p.lsp_message(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "rootUri": "file:///playground", "capabilities": {} },
    }));
    assert!(
        out[0]["result"]["capabilities"]["hoverProvider"]
            .as_bool()
            .unwrap_or(false)
            || out[0]["result"]["capabilities"]["hoverProvider"].is_object(),
        "{out:?}"
    );
    p.lsp_message(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
    let text = "local count = 1\nlocal n: string = count\nprint(n)\n";
    p.lsp_message(&json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": URI, "languageId": "wowlua", "version": 1, "text": text } },
    }));
    let wait = p.lsp_wait().expect("a check is due");
    p.set_time(1_800_000_001_000.0, wait.as_secs_f64() * 1000.0 + 1.0);
    out = p.lsp_run_due();
    let published = of(&out, "textDocument/publishDiagnostics");
    let mine = published
        .iter()
        .find(|m| m["params"]["uri"] == URI)
        .unwrap_or_else(|| panic!("{out:?}"));
    let codes: Vec<&Json> = mine["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| &d["code"])
        .collect();
    assert_eq!(codes, [&json!("E2001")], "{mine}");
    out = p.lsp_message(&json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover",
        "params": { "textDocument": { "uri": URI }, "position": { "line": 0, "character": 7 } },
    }));
    out.extend(p.lsp_run_due());
    let hover = out
        .iter()
        .find(|m| m["id"] == 2)
        .unwrap_or_else(|| panic!("{out:?}"));
    let shown = hover["result"]["contents"].to_string();
    assert!(shown.contains("number"), "{hover}");
}
