//! Completion (decision T377): the names in scope, a value's members after
//! `.` and `:`, type names in annotations, and doc comments on request.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use serde_json::{Value as Json, json};

mod common;
use common::Client;

const UTIL: &str = "\
--- Says hello to `name`.
export function greet(name: string): string
\treturn `hi {name}`
end

export class Counter
\tcount: number = 0
\tprivate secret: number = 1

\tfunction bump(by: number)
\t\tself.count += by
\tend

\tstatic function zero(): Counter
\t\treturn Counter.new()
\tend
end
";

const MAIN: &str = "\
import greet, Counter from \"./util\"
";

/// The project, and the URI of its `src/main.wowlua`.
fn project() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "lsp-complete", "version": "0.1.0", "wowlua": "^0.12" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/util.wowlua"), UTIL).unwrap();
    std::fs::write(dir.path().join("src/main.wowlua"), MAIN).unwrap();
    let main = wowlua_lsp::text::canonical(&dir.path().join("src/main.wowlua"));
    (dir, wowlua_lsp::text::uri_of(&main))
}

/// The completions with the document's text `text`, at its last `|` (one
/// before it is a type's).
fn complete(c: &mut Client, uri: &str, text: &str) -> Json {
    let offset = text.rfind('|').expect("a cursor");
    let mut text = text.to_string();
    text.remove(offset);
    let before = &text[..offset];
    let line = before.matches('\n').count();
    let character = offset - before.rfind('\n').map_or(0, |i| i + 1);
    c.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": text }] }),
    );
    c.request(
        "textDocument/completion",
        json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } }),
    )["result"]
        .clone()
}

fn labels(list: &Json) -> Vec<String> {
    list["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["label"].as_str().map(str::to_string))
        .collect()
}

fn item<'a>(list: &'a Json, label: &str) -> &'a Json {
    list["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i["label"] == label)
        .unwrap_or_else(|| panic!("no {label} in {:?}", labels(list)))
}

#[test]
fn completion_lists_what_can_stand_at_the_cursor() {
    let (_dir, uri) = project();
    let mut c = Client::start();
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": MAIN } }),
    );
    let head = "import greet, Counter from \"./util\"\nlocal counter: Counter = Counter.new()\nlocal message = greet(\"you\")\n";

    // an object's members: fields and methods, not its private ones
    let list = complete(&mut c, &uri, &format!("{head}counter.|\n"));
    let found = labels(&list);
    assert!(
        found.contains(&"count".into()) && found.contains(&"bump".into()),
        "{found:?}"
    );
    assert!(!found.contains(&"secret".into()), "{found:?}");
    assert_eq!(item(&list, "count")["detail"], "number");
    assert_eq!(item(&list, "count")["kind"], 5);
    assert_eq!(item(&list, "bump")["kind"], 2);

    // after `:`, the methods only, without `self`
    let list = complete(&mut c, &uri, &format!("{head}counter:b|\n"));
    let found = labels(&list);
    assert!(
        found.contains(&"bump".into()) && !found.contains(&"count".into()),
        "{found:?}"
    );
    let detail = item(&list, "bump")["detail"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(detail.starts_with("(by: number) ->"), "{detail}");

    // a class's statics
    let list = complete(&mut c, &uri, &format!("{head}local z = Counter.|\n"));
    assert!(
        labels(&list).contains(&"zero".into()),
        "{:?}",
        labels(&list)
    );

    // names in scope, the game's globals that match, keywords
    let list = complete(&mut c, &uri, &format!("{head}local n = me|\n"));
    let found = labels(&list);
    assert!(found.contains(&"message".into()), "{found:?}");
    assert_eq!(item(&list, "message")["detail"], "string");
    assert_eq!(item(&list, "greet")["kind"], 3);
    let list = complete(&mut c, &uri, &format!("{head}C_Tim|\n"));
    assert!(
        labels(&list).contains(&"C_Timer".into()),
        "{:?}",
        labels(&list)
    );
    let list = complete(&mut c, &uri, &format!("{head}C_Timer.|\n"));
    assert!(
        labels(&list).contains(&"After".into()),
        "{:?}",
        labels(&list)
    );
    let list = complete(&mut c, &uri, &format!("{head}loc|\n"));
    assert!(
        labels(&list).contains(&"local".into()),
        "{:?}",
        labels(&list)
    );

    // a string's methods
    let list = complete(&mut c, &uri, &format!("{head}local u = message:|\n"));
    assert!(
        labels(&list).contains(&"upper".into()),
        "{:?}",
        labels(&list)
    );

    // unfinished code around the cursor
    let list = complete(&mut c, &uri, &format!("{head}if counter.|\n"));
    assert!(
        labels(&list).contains(&"count".into()),
        "{:?}",
        labels(&list)
    );
    let list = complete(&mut c, &uri, &format!("{head}print(counter.|\n"));
    assert!(
        labels(&list).contains(&"count".into()),
        "{:?}",
        labels(&list)
    );

    // type names in an annotation
    let list = complete(&mut c, &uri, &format!("{head}local x: Cou|\n"));
    let found = labels(&list);
    assert!(
        found.contains(&"Counter".into()) && found.contains(&"number".into()),
        "{found:?}"
    );

    // none in a comment
    let list = complete(&mut c, &uri, &format!("{head}-- counter.|\n"));
    assert!(labels(&list).is_empty(), "{:?}", labels(&list));

    // a completion's doc comment, when the editor shows it
    let list = complete(&mut c, &uri, &format!("{head}gr|\n"));
    let greet = item(&list, "greet").clone();
    let resolved = c.request("completionItem/resolve", greet)["result"].clone();
    let doc = resolved["documentation"]["value"]
        .as_str()
        .unwrap_or_default();
    assert!(doc.contains("Says hello to `name`."), "{resolved}");
    assert!(c.finish());
}

#[test]
fn completion_in_a_string_lists_the_strings_its_type_names() {
    let (_dir, uri) = project();
    let mut c = Client::start();
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": MAIN } }),
    );
    let head = "local frame = CreateFrame(\"Frame\")\n";
    // an event name
    let list = complete(
        &mut c,
        &uri,
        &format!("{head}frame:RegisterEvent(\"UNIT_|\")\n"),
    );
    let found = labels(&list);
    assert!(
        found.contains(&"UNIT_HEALTH".into()),
        "{} items",
        found.len()
    );
    assert_eq!(item(&list, "UNIT_HEALTH")["kind"], 12);
    assert!(!found.contains(&"local".into()), "no keywords in a string");
    assert!(
        item(&list, "UNIT_HEALTH")["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("string"),
        "the event's payload"
    );
    // an anchor point (an overloaded method)
    let list = complete(&mut c, &uri, &format!("{head}frame:SetPoint(\"TOP|\")\n"));
    let found = labels(&list);
    assert!(
        found.contains(&"TOPLEFT".into()) && found.contains(&"CENTER".into()),
        "{found:?}"
    );
    // still being typed, no closing quote yet
    let list = complete(
        &mut c,
        &uri,
        &format!("{head}frame:RegisterEvent(\"PLAYER_|\n"),
    );
    assert!(labels(&list).contains(&"PLAYER_LOGIN".into()));
    // an annotated local's strings
    let list = complete(
        &mut c,
        &uri,
        &format!("{head}local mode: \"on\" | \"off\" = \"|\"\n"),
    );
    assert_eq!(labels(&list), vec!["on".to_string(), "off".to_string()]);
    // compared with a value of such a type
    let list = complete(
        &mut c,
        &uri,
        &format!("{head}local function f(e: \"a\" | \"b\")\n\tif e == \"|\" then end\nend\n"),
    );
    assert_eq!(labels(&list), vec!["a".to_string(), "b".to_string()]);
    // a string no type names: nothing
    let list = complete(&mut c, &uri, &format!("{head}print(\"hel|\")\n"));
    assert!(labels(&list).is_empty(), "{:?}", labels(&list));
    assert!(c.finish());
}

#[test]
fn completion_after_at_lists_the_decorators_that_apply() {
    let (_dir, uri) = project();
    let mut c = Client::start();
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": MAIN } }),
    );
    let head = "import * as events from \"wowlua:events\"\nlocal function memo(f: () -> ()): () -> () return f end\n";
    // on a function: the core decorators that apply, user decorators
    let list = complete(&mut c, &uri, &format!("{head}@|\nlocal function f() end\n"));
    let found = labels(&list);
    for d in ["inline", "global", "deprecated", "memo", "events"] {
        assert!(found.contains(&d.to_string()), "no {d} in {found:?}");
    }
    assert!(!found.contains(&"script".into()), "{found:?}");
    let doc = item(&list, "global")["documentation"]["value"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(doc.contains("global"), "{doc}");
    assert_eq!(item(&list, "global")["detail"], "@global(name?)");
    // nothing after it yet
    let list = complete(&mut c, &uri, &format!("{head}@deb|"));
    assert!(
        labels(&list).contains(&"debugOnly".into()),
        "{:?}",
        labels(&list)
    );
    // on a method: `@script`
    let list = complete(
        &mut c,
        &uri,
        &format!("{head}class B extends Button\n\t@sc|\n\tfunction click() end\nend\n"),
    );
    assert!(
        labels(&list).contains(&"script".into()),
        "{:?}",
        labels(&list)
    );
    // a module's decorators, and an event decorator's event names
    let list = complete(
        &mut c,
        &uri,
        &format!("{head}@events.|\nlocal function g() end\n"),
    );
    assert!(
        labels(&list).contains(&"event".into()),
        "{:?}",
        labels(&list)
    );
    let list = complete(
        &mut c,
        &uri,
        &format!("{head}@events.event(\"PLAYER_|\")\nlocal function h() end\n"),
    );
    assert!(
        labels(&list).contains(&"PLAYER_LOGIN".into()),
        "{} items",
        labels(&list).len()
    );
    assert!(c.finish());
}

#[test]
fn signature_help_shows_the_parameters_and_the_one_at_the_cursor() {
    let (_dir, uri) = project();
    let mut c = Client::start();
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": MAIN } }),
    );
    let help = |c: &mut Client, text: &str| -> Json {
        let offset = text.rfind('|').expect("a cursor");
        let mut t = text.to_string();
        t.remove(offset);
        let before = &t[..offset];
        let line = before.matches('\n').count();
        let character = offset - before.rfind('\n').map_or(0, |i| i + 1);
        c.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [{ "text": t }] }),
        );
        c.request(
            "textDocument/signatureHelp",
            json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } }),
        )["result"]
            .clone()
    };
    let head = "import greet, Counter from \"./util\"\nlocal counter: Counter = Counter.new()\n";
    // an imported function: its parameter, its doc comment
    let h = help(&mut c, &format!("{head}local s = greet(|)\n"));
    let sig = &h["signatures"][0];
    assert_eq!(sig["label"], "greet(name: string) -> string", "{h}");
    assert_eq!(sig["parameters"][0]["label"], json!([6, 18]), "{h}");
    assert!(
        sig["documentation"]["value"]
            .as_str()
            .unwrap_or("")
            .contains("Says hello"),
        "{h}"
    );
    assert_eq!(h["activeParameter"], 0);
    // a method, the second argument; unfinished
    let h = help(&mut c, &format!("{head}counter:bump(1, |\n"));
    assert!(
        h["signatures"][0]["label"]
            .as_str()
            .unwrap_or("")
            .starts_with("bump(by: number)"),
        "{h}"
    );
    assert_eq!(h["activeParameter"], 1);
    // the game's API, with its parameters' descriptions
    let h = help(
        &mut c,
        &format!("{head}local hp = UnitHealth(\"player\", |)\n"),
    );
    let sig = &h["signatures"][0];
    assert!(
        sig["label"]
            .as_str()
            .unwrap_or("")
            .starts_with("UnitHealth(unit: string, usePredicted: boolean?)"),
        "{h}"
    );
    assert_eq!(h["activeParameter"], 1);
    assert!(
        sig["parameters"][1]["documentation"]["value"]
            .as_str()
            .unwrap_or("")
            .contains("Predicts"),
        "{h}"
    );
    // no call around the cursor
    let h = help(&mut c, &format!("{head}local z = |\n"));
    assert!(h.is_null(), "{h}");
    assert!(c.finish());
}
