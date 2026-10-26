//! The package is the files the page wrote (spec M14 S2): `check`, `build`,
//! `run` and `format` read them as `wowlua check` reads the disk, even when
//! the editor has one open in the language server with another text; the
//! server reads an open document's text from the editor, the other files
//! as written, and checks again when the page writes, deletes or replaces
//! files, as after files changed on disk. One test: the files' host is the
//! program's.

use serde_json::{Value as Json, json};
use std::time::Duration;
use wowlua_web::Playground;

const MAIN: &str = "file:///playground/src/main.wowlua";
const UTIL: &str = "file:///playground/src/util.wowlua";

/// The codes the server last published for `uri` among `messages`.
fn published(messages: &[Json], uri: &str) -> Option<Vec<String>> {
    messages
        .iter()
        .rev()
        .find(|m| m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri)
        .map(|m| {
            m["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| d["code"].as_str().unwrap_or_default().to_string())
                .collect()
        })
}

/// The codes of a check's or build's diagnostics.
fn codes(diagnostics: &Json) -> Vec<String> {
    diagnostics
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Runs the server's checks until none waits, the clock moving as they
/// ask; what they published.
fn settle(p: &mut Playground, now: &mut f64) -> Vec<Json> {
    let mut out = Vec::new();
    for _ in 0..20 {
        let Some(wait) = p.lsp_wait() else { break };
        *now += wait.max(Duration::from_millis(1)).as_secs_f64() * 1000.0;
        p.set_time(1_800_000_000_000.0 + *now, *now);
        out.extend(p.lsp_run_due());
    }
    out
}

#[test]
fn what_the_page_wrote_is_the_package_and_the_server_sees_it() {
    let mut p = Playground::new();
    let mut now = 0.0;
    p.set_time(1_800_000_000_000.0, now);
    p.lsp_message(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "rootUri": "file:///playground", "capabilities": {} },
    }));
    p.lsp_message(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
    p.lsp_message(&json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": { "uri": MAIN, "languageId": "wowlua", "version": 1, "text": "local a = 1\n" } },
    }));
    let out = settle(&mut p, &mut now);
    assert_eq!(
        published(&out, MAIN),
        Some(vec!["W1003".to_string()]),
        "{out:?}"
    );

    // the page writes another text: the package is what it wrote
    p.write_file("src/main.wowlua", "local n: number = \"one\"\nprint(n)\n");
    assert_eq!(codes(&p.check()), ["E2001"]);
    let built = p.build("dev");
    assert_eq!(built["ok"], false, "{built}");
    assert_eq!(codes(&built["diagnostics"]), ["E2001"]);
    let ran = p.run(wowlua_web::run::Options {
        flavor: "camelot".into(),
        blizzard_ui: false,
        profile: None,
        debug: None,
        installation: None,
    });
    assert_eq!(ran["ok"], false, "{ran}");
    assert_eq!(codes(&ran["diagnostics"]), ["E2001"]);
    p.write_file("src/main.wowlua", "print(  1  )\n");
    assert_eq!(p.format("src/main.wowlua")["text"], "print(1)\n");

    // the server keeps the editor's text of the open file, and checks the
    // package again with a new file the page wrote
    p.write_file("src/util.wowlua", "local s: string = 2\nprint(s)\n");
    assert!(p.lsp_wait().is_some(), "a check is due after a write");
    assert_eq!(codes(&p.check()), ["E2001"]);
    let out = settle(&mut p, &mut now);
    assert_eq!(
        published(&out, UTIL),
        Some(vec!["E2001".to_string()]),
        "{out:?}"
    );
    assert_eq!(
        published(&out, MAIN).unwrap_or_else(|| vec!["W1003".into()]),
        ["W1003"],
        "{out:?}"
    );
    // and the file deleted: its diagnostics are gone
    p.delete_file("src/util.wowlua");
    let out = settle(&mut p, &mut now);
    assert_eq!(published(&out, UTIL), Some(vec![]), "{out:?}");
    assert_eq!(codes(&p.check()), Vec::<String>::new());
}
