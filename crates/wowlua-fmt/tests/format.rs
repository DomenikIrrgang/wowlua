//! Formatter behavior (spec tooling §16.3).

use wowlua_fmt::{Config, format};
use wowlua_span::SourceDb;

fn fmt(src: &str) -> String {
    let mut db = SourceDb::new();
    let file = db.add("t.lua", src.to_string());
    match format(src.as_bytes(), file, &Config::default()) {
        Ok(out) => String::from_utf8(out).unwrap(),
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn overload_signatures() {
    let src =
        "overload  function pick( n:number ):string\nexport overload function wrap<T>(v:T):{v:T}\n";
    assert_eq!(
        fmt(src),
        "overload function pick(n: number): string\nexport overload function wrap<T>(v: T): { v: T }\n"
    );
}

#[test]
fn show() {
    let src = r#"-- Header comment
local addonName,ns=...
local f=CreateFrame('Frame',nil,UIParent) -- the frame
f:RegisterEvent("PLAYER_LOGIN");f:SetScript('OnEvent',function(self,event,...)
  if event=="PLAYER_LOGIN" then print('hi',addonName)
  elseif event == 'X' then return end
end)

local t={a=1,b='two',[3]=4, nested={x=1}}
local long = {"aaaaaaaaaaaaaaa","bbbbbbbbbbbbbbbbb","cccccccccccccccccc","dddddddddddddddd","eeeeeeeeeeeee"}
for i=1,#t do local v=t[i] if not v then break end end
--[[ block
comment ]]
function ns.M:method(a,b) return a and b or-1 end
print(("x"):rep(3), - -1)
"#;
    println!("{}", fmt(src));
}

/// Shows where formatting changes the meaning of `$WOWLUA_FMT_FILE`.
#[test]
#[ignore]
fn diagnose_file() {
    let Ok(path) = std::env::var("WOWLUA_FMT_FILE") else {
        return;
    };
    let src = std::fs::read(&path).unwrap();
    let mut db = SourceDb::new();
    let file = db.add("t.lua", String::from_utf8_lossy(&src).into_owned());
    let config = Config {
        quote: wowlua_fmt::QuoteStyle::Preserve,
        ..Config::default()
    };
    match format(&src, file, &config) {
        Ok(_) => println!("preserve quotes: ok"),
        Err(e) => println!("preserve quotes: {e:?}"),
    }
    match format(&src, file, &Config::default()) {
        Ok(_) => println!("double quotes: ok"),
        Err(e) => println!("double quotes: {e:?}"),
    }
    let (a, b, printed) = wowlua_fmt::debug_fingerprints(&src, file, &Config::default());
    if let Some(at) = a.chars().zip(b.chars()).position(|(x, y)| x != y) {
        let from = at.saturating_sub(80);
        println!(
            "original:  {}",
            a.chars().skip(from).take(160).collect::<String>()
        );
        println!(
            "formatted: {}",
            b.chars().skip(from).take(160).collect::<String>()
        );
        let _ = printed;
    }
}

#[test]
#[ignore]
fn show_passes() {
    let Ok(path) = std::env::var("WOWLUA_FMT_FILE") else {
        return;
    };
    let src = std::fs::read(&path).unwrap();
    let mut db = SourceDb::new();
    let file = db.add("t.lua", String::from_utf8_lossy(&src).into_owned());
    let once = format(&src, file, &Config::default()).unwrap();
    let twice = format(&once, file, &Config::default()).unwrap();
    let pat = std::env::var("WOWLUA_FMT_GREP").unwrap_or_default();
    for (name, text) in [("source", &src), ("once", &once), ("twice", &twice)] {
        let text = String::from_utf8_lossy(text);
        let lines: Vec<&str> = text.lines().collect();
        if let Some(i) = lines.iter().position(|l| l.contains(&pat)) {
            println!("== {name}");
            for l in &lines[i.saturating_sub(4)..(i + 3).min(lines.len())] {
                println!("{}", l.chars().take(70).collect::<String>());
            }
        }
    }
}

/// Fields named by a string key keep their optional and read-only markers.
#[test]
fn string_keyed_type_fields() {
    let src = "type C = { kind: string, [\"not\"]?: boolean, [\"end\"]: number }\n";
    assert_eq!(fmt(src), src);
    let src = "type R = { readonly [\"end\"]: number, readonly [\"not\"]?: boolean }\n";
    assert_eq!(fmt(src), src);
}

/// A returned function's body is indented once.
#[test]
fn returned_function_body() {
    let src = "local function f()\n\treturn function()\n\t\tprint(1)\n\tend\nend\n";
    assert_eq!(fmt(src), src);
}

/// A signature too long for the line breaks between its parameters, one
/// per line, never inside a parameter's type.
#[test]
fn long_signatures_break_between_parameters() {
    let src = "export function onUnit<E extends EventName>(event: E, units: UnitToken | UnitToken[], fn: (name: E) -> (), owner: unknown): boolean\n    return true\nend\n";
    assert_eq!(
        fmt(src),
        "export function onUnit<E extends EventName>(\n\tevent: E,\n\tunits: UnitToken | UnitToken[],\n\tfn: (name: E) -> (),\n\towner: unknown\n): boolean\n\treturn true\nend\n"
    );
    // one that fits stays on its line
    assert_eq!(
        fmt("local function add(a: number, b: number): number\n    return a + b\nend\n"),
        "local function add(a: number, b: number): number\n\treturn a + b\nend\n"
    );
}

/// An empty body's `end` follows the signature when the line fits, and goes
/// below it otherwise (never past the line width).
#[test]
fn an_empty_body_ends_below_a_long_signature() {
    assert_eq!(fmt("function f(a)\nend\n"), "function f(a) end\n");
    let src = "class Bus\n    constructor(private dispatcher: Dispatcher = StdDispatcher.new(), private reporter: Reporter = log)\n    end\nend\n";
    let out = fmt(src);
    assert_eq!(
        out,
        "class Bus\n\tconstructor(\n\t\tprivate dispatcher: Dispatcher = StdDispatcher.new(),\n\t\tprivate reporter: Reporter = log\n\t)\n\tend\nend\n"
    );
    // fitting, but not with ` end` after it
    let src = "class Bus\n    constructor(private dispatcher: Dispatcher = StdDispatcher.new(), private reporter: Repo = log)\n    end\nend\n";
    assert_eq!(
        fmt(src),
        "class Bus\n\tconstructor(private dispatcher: Dispatcher = StdDispatcher.new(), private reporter: Repo = log)\n\tend\nend\n"
    );
    assert!(
        out.lines().all(|l| l.replace('\t', "    ").len() <= 100),
        "{out}"
    );
}

/// A comment after a table type's field stays on that field's line, pass
/// after pass.
#[test]
fn table_type_comments_stay_on_their_field() {
    let src = "export type Friend = {\n    name?: string,\n    color?: string,      -- a class colour token\n    online?: boolean,\n    -- the owner's own\n    sort?: string, -- last\n}\n";
    let once = fmt(src);
    assert_eq!(
        once,
        "export type Friend = {\n\tname?: string,\n\tcolor?: string, -- a class colour token\n\tonline?: boolean,\n\t-- the owner's own\n\tsort?: string, -- last\n}\n"
    );
    assert_eq!(fmt(&once), once);
}

/// A comment on its own line before a table stands before it and leaves the
/// table on one line.
#[test]
fn a_comment_before_a_table_does_not_break_it() {
    let src = "f({\n    -- the first\n    { \"0000\" },  -- four\n    { \"0001\" },  -- one\n})\n";
    let once = fmt(src);
    assert_eq!(
        once,
        "f({\n\t-- the first\n\t{ \"0000\" }, -- four\n\t{ \"0001\" }, -- one\n})\n"
    );
    assert_eq!(fmt(&once), once);
}
