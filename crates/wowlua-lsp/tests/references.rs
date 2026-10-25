//! Find references and rename (decision T389): every name of the workspace
//! that refers to the same declaration, open in the editor or not, and a
//! rename of them all, refused where it can't be done whole; and the
//! message when a package can't be loaded (decision T390).

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod common;
use common::Client;

const UTIL: &str = "\
export function greet(name: string): string
\treturn `hi {name}`
end

export class Counter
\tcount: number = 0

\tfunction bump()
\t\tself.count += 1
\tend
end

export class Special extends Counter
end

print(greet(\"me\"), Counter.new())
";

const MAIN: &str = "\
import greet, Counter, Special from \"./util\"

local c: Counter = Counter.new()
c:bump()
local message = greet(\"you\")
print(message, c.count)
local s = Special.new()
s:bump()
C_Timer.After(1, function() end)
";

const OTHER: &str = "\
import greet as hi from \"./util\"

print(hi(\"there\"), hi(\"again\"))
";

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "lsp-refs", "version": "0.1.0" }, "flavors": { "camelot": {} } }"#,
            ),
            ("src/util.wowlua", UTIL),
            ("src/main.wowlua", MAIN),
            ("src/other.wowlua", OTHER),
        ],
    );
    dir
}

fn write(root: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn file(dir: &Path, rel: &str) -> PathBuf {
    wowlua_lsp::text::canonical(&dir.join(rel))
}

/// The protocol position of the `nth` (from 0) `needle` in `text`, one
/// character into it.
fn at(text: &str, needle: &str, nth: usize) -> Json {
    let offset = text.match_indices(needle).nth(nth).expect("found").0 + 1;
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let character = offset - before.rfind('\n').map_or(0, |i| i + 1);
    json!({ "line": line, "character": character })
}

/// A client with `path` open.
fn open(path: &Path) -> (Client, String) {
    let uri = wowlua_lsp::text::uri_of(path);
    let text = std::fs::read_to_string(path).unwrap();
    let c = Client::start();
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": text } }),
    );
    (c, uri)
}

/// Each location as (file name, line from 1, the text it covers).
fn places(locations: &Json) -> Vec<(String, usize, String)> {
    let mut out: Vec<(String, usize, String)> = locations
        .as_array()
        .unwrap_or_else(|| panic!("no locations: {locations}"))
        .iter()
        .map(|l| {
            let path = wowlua_lsp::text::path_of(l["uri"].as_str().unwrap()).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let start = wowlua_lsp::text::offset(&text, &l["range"]["start"]);
            let end = wowlua_lsp::text::offset(&text, &l["range"]["end"]);
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let line = l["range"]["start"]["line"].as_u64().unwrap() as usize + 1;
            (name, line, text[start..end].to_string())
        })
        .collect();
    out.sort();
    out
}

fn references(c: &mut Client, uri: &str, text: &str, needle: &str, nth: usize) -> Json {
    c.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": uri },
            "position": at(text, needle, nth),
            "context": { "includeDeclaration": true },
        }),
    )
}

fn rename(c: &mut Client, uri: &str, text: &str, needle: &str, nth: usize, new: &str) -> Json {
    c.request(
        "textDocument/rename",
        json!({ "textDocument": { "uri": uri }, "position": at(text, needle, nth), "newName": new }),
    )
}

/// The files' texts with a rename's edits made, by file name.
fn renamed(reply: &Json) -> BTreeMap<String, String> {
    let changes = reply["result"]["changes"]
        .as_object()
        .unwrap_or_else(|| panic!("no edits: {reply}"));
    let mut out = BTreeMap::new();
    for (uri, edits) in changes {
        let path = wowlua_lsp::text::path_of(uri).unwrap();
        let mut text = std::fs::read_to_string(&path).unwrap();
        let mut edits: Vec<(usize, usize, String)> = edits
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    wowlua_lsp::text::offset(&text, &e["range"]["start"]),
                    wowlua_lsp::text::offset(&text, &e["range"]["end"]),
                    e["newText"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        for (s, e, new) in edits {
            text.replace_range(s..e, &new);
        }
        out.insert(
            path.file_name().unwrap().to_string_lossy().into_owned(),
            text,
        );
    }
    out
}

fn refused(reply: &Json) -> String {
    reply["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("not refused: {reply}"))
        .to_string()
}

fn p(file: &str, line: usize, text: &str) -> (String, usize, String) {
    (file.to_string(), line, text.to_string())
}

#[test]
fn references_are_found_in_every_file_of_the_package() {
    let dir = project();
    let main = file(dir.path(), "src/main.wowlua");
    let (mut c, uri) = open(&main);
    // an export, from a use: its declaration, the imports and their uses,
    // in files that are not open too (an alias's names in its file)
    let r = references(&mut c, &uri, MAIN, "greet(", 0);
    assert_eq!(
        places(&r["result"]),
        vec![
            p("main.wowlua", 1, "greet"),
            p("main.wowlua", 5, "greet"),
            p("other.wowlua", 1, "greet"),
            p("other.wowlua", 1, "hi"),
            p("other.wowlua", 3, "hi"),
            p("other.wowlua", 3, "hi"),
            p("util.wowlua", 1, "greet"),
            p("util.wowlua", 16, "greet"),
        ]
    );
    // a method: its calls, inherited ones too
    let r = references(&mut c, &uri, MAIN, "bump", 0);
    assert_eq!(
        places(&r["result"]),
        vec![
            p("main.wowlua", 4, "bump"),
            p("main.wowlua", 8, "bump"),
            p("util.wowlua", 8, "bump"),
        ]
    );
    // a field: `self.count` and `c.count`
    let r = references(&mut c, &uri, MAIN, "count", 0);
    assert_eq!(
        places(&r["result"]),
        vec![
            p("main.wowlua", 6, "count"),
            p("util.wowlua", 6, "count"),
            p("util.wowlua", 9, "count"),
        ]
    );
    // a local, without its declaration
    let r = c.request(
        "textDocument/references",
        json!({
            "textDocument": { "uri": uri },
            "position": at(MAIN, "message", 1),
            "context": { "includeDeclaration": false },
        }),
    );
    assert_eq!(places(&r["result"]), vec![p("main.wowlua", 6, "message")]);
    // a class: its import, the type and the value
    let r = references(&mut c, &uri, MAIN, "Counter", 1);
    let found = places(&r["result"]);
    assert!(found.contains(&p("main.wowlua", 3, "Counter")), "{found:?}");
    assert!(found.contains(&p("util.wowlua", 5, "Counter")), "{found:?}");
    assert!(found.iter().all(|(_, _, t)| t == "Counter"), "{found:?}");
    // from a declaration nothing refers to through the cursor's file: its
    // uses elsewhere
    let util = file(dir.path(), "src/util.wowlua");
    let util_uri = wowlua_lsp::text::uri_of(&util);
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": util_uri, "languageId": "wowlua", "version": 1, "text": UTIL } }),
    );
    let r = references(&mut c, &util_uri, UTIL, "greet", 0);
    assert_eq!(places(&r["result"]).len(), 8, "{r}");
    // a module's own (hoisted) function, used in it: go to definition too
    let r = c.request(
        "textDocument/definition",
        json!({ "textDocument": { "uri": util_uri }, "position": at(UTIL, "greet", 1) }),
    );
    assert_eq!(places(&r["result"]), vec![p("util.wowlua", 1, "greet")]);
    // nothing at a keyword
    let r = references(&mut c, &uri, MAIN, "local", 0);
    assert_eq!(r["result"], json!([]));
    assert!(c.finish());
}

#[test]
fn a_rename_changes_every_name_and_keeps_aliases() {
    let dir = project();
    let main = file(dir.path(), "src/main.wowlua");
    let (mut c, uri) = open(&main);
    let edited = renamed(&rename(&mut c, &uri, MAIN, "greet(", 0, "hello"));
    assert_eq!(
        edited["main.wowlua"],
        MAIN.replace("greet", "hello"),
        "{edited:?}"
    );
    assert_eq!(
        edited["other.wowlua"],
        OTHER.replace("greet", "hello"),
        "{edited:?}"
    );
    assert_eq!(edited["util.wowlua"], UTIL.replace("greet", "hello"));
    // an alias: its file's names only
    let other = file(dir.path(), "src/other.wowlua");
    let other_uri = wowlua_lsp::text::uri_of(&other);
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": other_uri, "languageId": "wowlua", "version": 1, "text": OTHER } }),
    );
    let edited = renamed(&rename(&mut c, &other_uri, OTHER, "hi(", 1, "hey"));
    assert_eq!(edited.len(), 1, "{edited:?}");
    assert_eq!(edited["other.wowlua"], OTHER.replace("hi", "hey"));
    // a method, called with `:`, inherited calls too
    let edited = renamed(&rename(&mut c, &uri, MAIN, "bump", 1, "step"));
    assert_eq!(edited["main.wowlua"], MAIN.replace("bump", "step"));
    assert_eq!(edited["util.wowlua"], UTIL.replace("bump", "step"));
    // a field, `self.count` too
    let edited = renamed(&rename(&mut c, &uri, MAIN, "count", 0, "total"));
    assert_eq!(edited["main.wowlua"], MAIN.replace("count", "total"));
    assert_eq!(edited["util.wowlua"], UTIL.replace("count", "total"));
    // a class
    let edited = renamed(&rename(&mut c, &uri, MAIN, "Counter", 1, "Tally"));
    assert_eq!(edited["main.wowlua"], MAIN.replace("Counter", "Tally"));
    assert_eq!(edited["util.wowlua"], UTIL.replace("Counter", "Tally"));
    // nothing was written: the files are as they were
    assert_eq!(std::fs::read_to_string(&main).unwrap(), MAIN);
    assert!(c.finish());
}

#[test]
fn a_rename_that_cannot_be_done_whole_is_refused() {
    let dir = project();
    let main = file(dir.path(), "src/main.wowlua");
    let (mut c, uri) = open(&main);
    // what wowlua declares
    let why = refused(&rename(&mut c, &uri, MAIN, "print", 0, "say"));
    assert!(why.contains("declared by wowlua"), "{why}");
    let why = refused(&rename(&mut c, &uri, MAIN, "After", 0, "Later"));
    assert!(why.contains("declared by wowlua"), "{why}");
    // a new name that is no name
    let why = refused(&rename(&mut c, &uri, MAIN, "message", 0, "end"));
    assert!(why.contains("keyword"), "{why}");
    let why = refused(&rename(&mut c, &uri, MAIN, "message", 0, "two words"));
    assert!(why.contains("not a name"), "{why}");
    // a name that another takes: `c` would be the string, not the counter
    let why = refused(&rename(&mut c, &uri, MAIN, "message", 0, "c"));
    assert!(why.contains("renaming to `c`"), "{why}");
    // nothing to rename at a keyword
    let r = rename(&mut c, &uri, MAIN, "local", 0, "x");
    assert!(r["error"].is_object(), "{r}");
    // prepareRename: the name's range, else why not
    let r = c.request(
        "textDocument/prepareRename",
        json!({ "textDocument": { "uri": uri }, "position": at(MAIN, "message", 0) }),
    );
    assert_eq!(r["result"]["placeholder"], "message", "{r}");
    let r = c.request(
        "textDocument/prepareRename",
        json!({ "textDocument": { "uri": uri }, "position": at(MAIN, "local", 0) }),
    );
    assert!(refused(&r).contains("keyword"));
    let r = c.request(
        "textDocument/prepareRename",
        json!({ "textDocument": { "uri": uri }, "position": at(MAIN, "After", 0) }),
    );
    assert!(refused(&r).contains("declared by wowlua"), "{r}");
    assert!(c.finish());
}

/// An override must keep its overridden method's name: the check of the
/// renamed text finds it (E3005), and the rename is refused.
#[test]
fn renaming_an_overridden_method_is_refused() {
    let dir = project();
    write(
        dir.path(),
        &[(
            "src/util.wowlua",
            &UTIL.replace(
                "export class Special extends Counter\nend",
                "export class Special extends Counter\n\toverride function bump()\n\tend\nend",
            ),
        )],
    );
    let main = file(dir.path(), "src/main.wowlua");
    let (mut c, uri) = open(&main);
    let why = refused(&rename(&mut c, &uri, MAIN, "bump", 0, "step"));
    assert!(why.contains("E3005") || why.contains("override"), "{why}");
    assert!(c.finish());
}

/// A file wowlua doesn't check that names the name: the rename is refused.
#[test]
fn a_name_in_an_unchecked_file_stops_a_rename() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "lsp-xml", "version": "0.1.0" }, "flavors": { "camelot": {} } }"#,
            ),
            (
                "src/main.wowlua",
                "function MyAddon_OnLoad()\nend\n\nMyAddon_OnLoad()\n",
            ),
            (
                "src/frames.xml",
                "<Ui><Frame name=\"MyAddonFrame\"><Scripts><OnLoad>MyAddon_OnLoad()</OnLoad></Scripts></Frame></Ui>\n",
            ),
        ],
    );
    let main = file(dir.path(), "src/main.wowlua");
    let text = std::fs::read_to_string(&main).unwrap();
    let (mut c, uri) = open(&main);
    let r = rename(&mut c, &uri, &text, "MyAddon_OnLoad", 1, "Renamed");
    let why = refused(&r);
    assert!(why.contains("frames.xml"), "{why}");
    assert!(c.finish());
}

/// A workspace: the packages that depend on one find its names too.
#[test]
fn references_reach_the_packages_that_depend_on_it() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "workspace": { "members": ["addons/*", "libs/*"] } }"#,
            ),
            (
                "addons/core/wowlua.json",
                r#"{ "package": { "name": "suite-core", "version": "1.0.0" }, "addon": { "name": "SuiteCore" }, "dependencies": { "shared": "*" } }"#,
            ),
            (
                "addons/core/src/main.wowlua",
                "import greet from \"shared/greet\"\nprint(greet(\"you\"))\n",
            ),
            (
                "libs/shared/wowlua.json",
                r#"{ "package": { "name": "shared", "version": "0.1.0", "kind": "library", "license": "MIT" } }"#,
            ),
            (
                "libs/shared/src/greet.wowlua",
                "export function greet(name: string): string\n    return \"hi \" .. name\nend\n",
            ),
        ],
    );
    let greet = file(dir.path(), "libs/shared/src/greet.wowlua");
    let text = std::fs::read_to_string(&greet).unwrap();
    let (mut c, uri) = open(&greet);
    let r = references(&mut c, &uri, &text, "greet", 0);
    assert_eq!(
        places(&r["result"]),
        vec![
            p("greet.wowlua", 1, "greet"),
            p("main.wowlua", 1, "greet"),
            p("main.wowlua", 2, "greet"),
        ]
    );
    let edited = renamed(&rename(&mut c, &uri, &text, "greet", 0, "hello"));
    assert_eq!(edited.len(), 2, "{edited:?}");
    assert!(edited["main.wowlua"].contains("print(hello(\"you\"))"));
    // go to definition from the addon: the library's file itself
    let main = file(dir.path(), "addons/core/src/main.wowlua");
    let main_text = std::fs::read_to_string(&main).unwrap();
    let main_uri = wowlua_lsp::text::uri_of(&main);
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": main_uri, "languageId": "wowlua", "version": 1, "text": main_text } }),
    );
    let r = c.request(
        "textDocument/definition",
        json!({ "textDocument": { "uri": main_uri }, "position": at(&main_text, "greet(", 0) }),
    );
    let target = wowlua_lsp::text::path_of(r["result"][0]["uri"].as_str().unwrap_or_default());
    assert_eq!(
        target.map(|t| wowlua_lsp::text::canonical(&t)),
        Some(greet),
        "{r}"
    );
    assert!(c.finish());
}

/// A package whose manifest can't be loaded: the editor is told once,
/// and the manifest's diagnostic is published as before.
#[test]
fn a_package_that_cannot_be_loaded_is_told_once() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "too-old", "version": "0.1.0", "wowlua": "^0.9" } }"#,
            ),
            ("src/main.wowlua", "print(1)\n"),
        ],
    );
    let main = file(dir.path(), "src/main.wowlua");
    let (mut c, uri) = open(&main);
    let shown = c.wait("a message", |m| m["method"] == "window/showMessage");
    let message = shown["params"]["message"].as_str().unwrap();
    assert_eq!(shown["params"]["type"], 1);
    assert!(
        message.starts_with("too-old can't be checked: this package requires wowlua ^0.9"),
        "{message}"
    );
    assert!(message.ends_with("(wowlua.json)"), "{message}");
    let manifest = wowlua_lsp::text::uri_of(&file(dir.path(), "wowlua.json"));
    c.diagnostics(&manifest, |d| d.iter().any(|d| d["code"] == "E4300"));
    // an edit and a save, more checks: no second message
    c.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": "print(2)\n" }] }),
    );
    c.notify(
        "textDocument/didSave",
        json!({ "textDocument": { "uri": uri } }),
    );
    // (the save's full check)
    c.diagnostics(&manifest, |d| d.iter().any(|d| d["code"] == "E4300"));
    assert!(
        !c.seen.iter().any(|m| m["method"] == "window/showMessage"),
        "{:#?}",
        c.seen
    );
    assert!(c.finish());
}
