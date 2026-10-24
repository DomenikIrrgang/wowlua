//! Completion (decision T377): the names in scope, a value's members after
//! `.` and `:`, type names in annotations, and doc comments on request.

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
        r#"{ "package": { "name": "lsp-complete", "version": "0.1.0", "wowlua": "^0.10" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/util.wowlua"), UTIL).unwrap();
    std::fs::write(dir.path().join("src/main.wowlua"), MAIN).unwrap();
    let main = wowlua_lsp::text::canonical(&dir.path().join("src/main.wowlua"));
    (dir, wowlua_lsp::text::uri_of(&main))
}

/// The completions with the document's text `text`, at its `|`.
fn complete(c: &mut Client, uri: &str, text: &str) -> Json {
    let offset = text.find('|').expect("a cursor");
    let text = text.replacen('|', "", 1);
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
