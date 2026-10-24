//! Go to definition (decision T369): locals, imports, class members, types,
//! globals and the game's API, from what the check found.

use serde_json::{Value as Json, json};
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
";

const SCRIPT: &str = "\
function SayHi()
\tprint(\"hi\")
end
";

const MAIN: &str = "\
import greet, Counter, Special from \"./util\"

local c: Counter = Counter.new()
c:bump()
local message = greet(\"you\")
print(message, c.count)
C_Timer.After(1, function() end)
local s = Special.new()
s:bump()
SayHi()
";

fn project() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "lsp-defs", "version": "0.1.0", "wowlua": "^0.10" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/util.wowlua"), UTIL).unwrap();
    std::fs::write(dir.path().join("src/main.wowlua"), MAIN).unwrap();
    std::fs::write(dir.path().join("src/hello.wowlua"), SCRIPT).unwrap();
    let main = wowlua_lsp::text::canonical(&dir.path().join("src/main.wowlua"));
    let util = wowlua_lsp::text::canonical(&dir.path().join("src/util.wowlua"));
    (dir, main, util)
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

/// The text a location's range covers (a single line).
fn covered(location: &Json) -> (PathBuf, String) {
    let path = wowlua_lsp::text::path_of(location["uri"].as_str().unwrap()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let start = wowlua_lsp::text::offset(&text, &location["range"]["start"]);
    let end = wowlua_lsp::text::offset(&text, &location["range"]["end"]);
    (path, text[start..end].to_string())
}

#[test]
fn names_lead_to_their_definitions() {
    let (_dir, main, util) = project();
    let uri = wowlua_lsp::text::uri_of(&main);
    let asks: Vec<(&str, usize)> = vec![
        ("message", 1), // 2: the local
        ("greet", 1),   // 3: the import's function
        ("Counter", 2), // 4: the import's class (in `Counter.new`)
        ("bump", 0),    // 5: the method
        ("count", 0),   // 6: the field
        ("Counter", 1), // 7: the type in `c: Counter`
        ("After", 0),   // 8: the game's API
        ("greet", 0),   // 9: the name in the import
        ("bump", 1),    // 10: an inherited method
        ("SayHi", 0),   // 11: a global another script defines
    ];
    let mut c = Client::start();
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "wowlua", "version": 1, "text": MAIN } }),
    );
    // asked at once: each waits for the check
    let mut answers = Vec::new();
    for (needle, nth) in &asks {
        let r = c.request(
            "textDocument/definition",
            json!({ "textDocument": { "uri": uri }, "position": at(MAIN, needle, *nth) }),
        );
        answers.push(r);
    }
    assert!(c.finish());
    let answer = |id: usize| -> (PathBuf, String) {
        let r = &answers[id - 2];
        let first = &r["result"][0];
        assert!(first.is_object(), "no definition for request {id}: {r}");
        covered(first)
    };
    let same = |p: &Path, q: &Path| p == q;
    let (p, t) = answer(2);
    assert!(same(&p, &main) && t == "message", "{p:?} {t}");
    let (p, t) = answer(3);
    assert!(same(&p, &util) && t == "greet", "{p:?} {t}");
    let (p, t) = answer(4);
    assert!(same(&p, &util) && t == "Counter", "{p:?} {t}");
    let (p, t) = answer(5);
    assert!(same(&p, &util) && t == "bump", "{p:?} {t}");
    let (p, t) = answer(6);
    assert!(same(&p, &util) && t == "count", "{p:?} {t}");
    let (p, t) = answer(7);
    assert!(same(&p, &util) && t == "Counter", "{p:?} {t}");
    let (p, t) = answer(8);
    assert!(
        p.to_string_lossy().ends_with(".d.wowlua") && t == "After",
        "{p:?} {t}"
    );
    let (p, t) = answer(9);
    assert!(same(&p, &util) && t == "greet", "{p:?} {t}");
    let (p, t) = answer(10);
    assert!(same(&p, &util) && t == "bump", "{p:?} {t}");
    let (p, t) = answer(11);
    assert!(p.ends_with("src/hello.wowlua") && t == "SayHi", "{p:?} {t}");
}
