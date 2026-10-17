//! The parser that builds the flat tree against the legacy pipeline (the
//! boxed tree, flattened): the same tree as the views show it, the same
//! tokens, trivia and diagnostics.

use super::parse as legacy_parse;
use crate::parse;
use wowlua_span::SourceDb;

/// The parse as text: everything the views show, then the tokens, trivia
/// and diagnostics. `pretty` puts it on lines, for reading a difference;
/// it costs a pass per level of nesting, too slow to compare deep trees.
fn dump(p: &crate::Parse, pretty: bool) -> String {
    let tokens: Vec<_> = p.lexed.tokens.iter().collect();
    if pretty {
        format!(
            "{:#?}\n{tokens:?}\n{:?}\n{:#?}",
            p.view(),
            p.lexed.trivia,
            p.diagnostics
        )
    } else {
        format!(
            "{:?}\n{tokens:?}\n{:?}\n{:?}",
            p.view(),
            p.lexed.trivia,
            p.diagnostics
        )
    }
}

/// `Ok(())` when both pipelines agree on `src`.
fn agree(src: &[u8]) -> Result<(), String> {
    let src = src.to_vec();
    // A large stack: the dump recurses over the whole tree.
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let mut db = SourceDb::new();
            let file = db.add("t.wowlua", String::from_utf8_lossy(&src).into_owned());
            let (new, old) = (parse(&src, file), legacy_parse(&src, file));
            if dump(&new, false) == dump(&old, false) {
                return Ok(());
            }
            let (new, old) = (dump(&new, true), dump(&old, true));
            let at = new
                .lines()
                .zip(old.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            let around = |s: &str| {
                s.lines()
                    .skip(at.saturating_sub(3))
                    .take(8)
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Err(format!(
                "differ at line {at}:\n--- new\n{}\n--- legacy\n{}",
                around(&new),
                around(&old)
            ))
        })
        .expect("thread")
        .join()
        .expect("no panic")
}

const CASES: &[&str] = &[
    // Lua 5.1
    "local a = 1",
    "a = b\n;(f)()",
    "return",
    "return ...",
    "x = .5 y = 5. z = 0xFF w = 0XAB v = 1E5 u = 1e+5 t = 1e-5 s = 08",
    "x = '\\q' y = '\\65\\066' z = '\\\n' w = \"a\\tb\" v = 'plain'",
    "x = [=[ ]] ]=] y = [==[\n]==] z = [[\nfirst newline]] w = [[\r\nx]]",
    "--[==[ c ]==] x = 1 -- c\n--[[ a ]]",
    "while true do break end repeat local x = 1 until x",
    "for i=1,2 do end for k,v in pairs(t) do end for i=1,2,3 do end",
    "if a then elseif b then elseif c then else end",
    "function f(...) return ... end local function f(a, b, ...) end function a.b:c() end",
    "a.b:c'x' f{} f'' f[[x]] f\n{} f\n'x'",
    "x = a.b['c'].d(e):f(g) y = (f)() z = (f()) w = {f()}",
    "x = {[1]=2, a=3; 4,} y = {{1, 2, {3}}, k = {v = {}}}",
    "x = - - 1 y = not not true z = 2^-3 w = #t",
    "x = a and b or c y = a == b == c z = 'a' .. 'b' .. 1 w = 1 .. 2",
    "local x, y a.b.c = 1, 2 a, b = 1 do end",
    // wowlua
    "local count: number = 0",
    "local name: string?, realm: string? = UnitName('target')",
    "local m: Map<string, List<number>> = {} local t: List<number>= {}",
    "local a: Map<string, Map<string, number>>= {} local b: X<Y<Z>>",
    "const MAX = 40 const PREFIX: string = 'x'",
    "local { x, y } = point local { name, level = 1 }: Info = getInfo()",
    "total += amount label ..= 'x' flags |= 1 items[i()] -= 1 cache[k] ??= {} n //= 2 b <<= 1",
    "local function clamp(v: number, lo: number = 0, hi: number = 1): number return v end",
    "function map<T, U>(xs: T[], f: (T) -> U): U[] return {} end",
    "local id = function<T>(x: T): T return x end",
    "function f(...: number) end function g(...: T...) end",
    "function f(): (number, string) end function g(): () end function h(): T... end",
    "function isFrame(x: any): x is Frame return true end",
    "for i, item in ipairs(items) do if item.hidden then continue end show(item) end",
    "for i: number = 1, 10 do end for k: string, v: Info in pairs(t) do end",
    "local a = b?.c?.[d]?:e(f)?.(g) local v = a ?? b",
    "local s = `Hello {name}, {#items} item{#items == 1 and '' or 's'} \\{x\\} {`nested {x}`}`",
    "local v = raw :: string | number local m = (flags :: number) | MASK",
    "local x = a & b | c ~ d << 1 >> 2 // 3 local y = ~x",
    "local n = 1_000_000 + 0xFF_FF + 0b1010_1010 + 3.141_592 + 1e1_0",
    "continue = 1 continue.x = 1 type = 5 local t = type(x) import('x') import 'x' export = {}",
    "@global('MyAddon')\nlocal function f() end",
    "@event('PLAYER_LOGIN') @profile\nfunction M.onLogin() end",
    "@a.b.c(1, 'x') @d{1} @e'x' @f\nfunction g() end",
    "type Point = { x: number, y: number, [string]: any, ['end']: number, readonly z?: number }",
    "type F = <T>(x: T, ...string) -> (T, number)",
    "type U = | 'a' | 'b' type N = 1 | 2 | nil | true | false | function",
    "type P<T...> = (T...) -> () type W = (A...) -> ...any type H = (EventArgs<E>...) -> ()",
    "declare function pcall<A..., R...>(f: (A...) -> R..., ...: A...): (boolean, R...)",
    "type C = Thread<A..., (...any), R...> type D = Q<(A, B, ...C)>",
    "type O = typeof(defaults) & { extra: boolean? } type A = string[] type M = (number)?",
    "interface Shape extends A, B\n readonly area: number\n label?: string\n function draw(self, x: number): boolean\nend",
    "enum Color { Red, Green = 2; Blue }",
    "abstract class Base<T> extends Frame implements Shape, Other\n  private static count: number = 0\n  readonly name?: string\n  static: boolean\n  @trace function greet(self, who: string): string return who end\n  abstract function area(self): number\n  overload function f(self, x: number): number\n  get size(): number return 1 end\n  set size(v: number) end\n  constructor(name: string) self.name = name end\n  overload constructor(x: number)\nend",
    "declare function GetTime(): number",
    "declare global MyAddonDB: { count: number }",
    "declare namespace C_Timer\n function After(seconds: number, @deferred cb: () -> ()): ()\n const MAX: number\n namespace Inner\n  x: number\n end\n class K extends J\n  static function new(): K\n  readonly v: number\n end\n interface I\n  y: string\n end\n type T = number\n enum E { A }\nend",
    "declare class Frame extends Region implements X\n static function new(): Frame\n readonly name: string\n optional?: number\n function Show(self): ()\nend",
    "declare interface Shape\n function x(self): number\nend",
    "declare event PLAYER_LOGIN(isInitial: boolean) declare message CacheReady(count: number)",
    "declare module 'lib'\n function f(): number\nend",
    "import * as mock from 'wowlua:mock' import _ from './side' import a, b as c from './x'",
    "import type Frame, Region as R from 'wow' import type from './x'",
    "export local a: number = 1 export function f() end export const X = 1",
    "export abstract class A\nend export type T = number export interface I\nend export enum E { A }",
    "local function f() repeat local x = 1 if x then continue end until x end end",
    // errors and recovery
    "local = 1\nprint('ok')\nlocal function (\nx = 2\nif then end\n",
    "local t = {\n  a = 1,\n  b = function()\n    local x = { c = 2, d = = 3 }\n    print(4, 5 +)\n    return 6\n  end,\n  e = 7,\n}\nf(8, function()\n  g(9, { 10, 11 + })\nend, 12)\n",
    "f\n() x = f(g)\n(h)() x = 0x y = 1e z = 3..2",
    ";",
    "a=1;;",
    "return 1; x=1",
    "break",
    "while true do break; x=1 end",
    "function f() return ... end",
    "(a) = 1",
    "a() = 1",
    "a",
    "x = {,}",
    "goto x",
    "for i=1 do end",
    "function f(..., a) end",
    "function a:b.c() end",
    "return return",
    "end",
    "x = true.y",
    "while x end",
    "x = 'abc",
    "x = \"abc\nd\"",
    "x = function() end ()",
    "local function() end",
    "x = 1 = 2",
    "if a then",
    "x = ",
    "f(",
    "x = !a",
    "x = a $ b",
    "a?.b = 1",
    "a, b += 1",
    "local x <const> = 1",
    "class A\n  x: = 1\n  function f(self) end\n  ???\nend",
    "@x local y = 1 @z print(1)",
    "do continue end",
    "while x do continue; y() end",
    "x = a ?? b or c",
    "x = (a ?? b) or c",
    "type X = (A, B)",
    "local x: Map<string",
    "export function a.b() end",
    "const a, b = 1",
    "const c",
    "export local q",
    "import _, a from 'x'",
    "import * from 'x'",
    "import a 'x'",
    "local s = `a {b`",
    "local s = `{}`",
    "declare module 5",
    "declare class X x end",
    "repeat local a = 1 continue until a",
    "x = 1 ++ 2",
];

#[test]
fn the_flat_parser_builds_the_legacy_tree() {
    let mut failures = Vec::new();
    for src in CASES {
        if let Err(e) = agree(src.as_bytes()) {
            failures.push(format!("{src:?}: {e}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn deep_chains_and_nesting_agree() {
    for src in [
        format!("local b = {}\n", vec!["true"; 3000].join(" and ")),
        format!("x = {}1{}", "(".repeat(300), ")".repeat(300)),
        "do ".repeat(300) + &"end ".repeat(300),
        format!("x = a{}", ".b".repeat(3000)),
    ] {
        if let Err(e) = agree(src.as_bytes()) {
            panic!("{}: {e}", &src[..40.min(src.len())]);
        }
    }
}

/// A small deterministic generator (xorshift), so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[(self.next() % items.len() as u64) as usize]
    }
}

const VOCAB: &[&str] = &[
    "local",
    "function",
    "end",
    "if",
    "then",
    "else",
    "elseif",
    "while",
    "do",
    "for",
    "in",
    "repeat",
    "until",
    "return",
    "break",
    "and",
    "or",
    "not",
    "nil",
    "true",
    "false",
    "a",
    "b",
    "f",
    "x",
    "1",
    "2.5",
    "'s'",
    "[[l]]",
    "...",
    "=",
    "==",
    "~=",
    "<",
    "<=",
    ">",
    ">=",
    "+",
    "-",
    "*",
    "/",
    "%",
    "^",
    "#",
    "..",
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ";",
    ",",
    ".",
    ":",
    "\n",
    "::",
    "?.",
    "?:",
    "??",
    "+=",
    "..=",
    "??=",
    ">>",
    ">>=",
    "&",
    "|",
    "~",
    "//",
    "->",
    "@g",
    "@a.b",
    "`t{x}u`",
    "continue",
    "const",
    "class",
    "type",
    "interface",
    "enum",
    "declare",
    "import",
    "export",
    "from",
    "as",
    "extends",
    "implements",
    "get",
    "set",
    "constructor",
    "static",
    "readonly",
    "abstract",
    "overload",
    "namespace",
    "global",
    "event",
    "module",
    "typeof",
    "'p'",
    "0b101",
    "1_000",
    "is",
    "?",
    "_",
    "*",
];

#[test]
fn random_token_sequences_agree() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut failures = Vec::new();
    for _ in 0..20_000 {
        let len = 1 + rng.next() % 14;
        let tokens: Vec<&str> = (0..len).map(|_| rng.pick(VOCAB)).collect();
        let src = tokens.join(" ");
        if let Err(e) = agree(src.as_bytes()) {
            failures.push(format!("{src:?}: {e}"));
        }
    }
    failures.truncate(5);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn mutated_cases_agree() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut failures = Vec::new();
    for _ in 0..20_000 {
        let base = CASES[(rng.next() % CASES.len() as u64) as usize];
        let mut tokens: Vec<String> = base.split(' ').map(str::to_string).collect();
        for _ in 0..1 + rng.next() % 3 {
            let at = (rng.next() % (tokens.len() as u64 + 1)) as usize;
            match rng.next() % 3 {
                0 if at < tokens.len() => {
                    tokens.remove(at);
                }
                1 => tokens.insert(at.min(tokens.len()), rng.pick(VOCAB).to_string()),
                _ if at < tokens.len() => tokens[at] = rng.pick(VOCAB).to_string(),
                _ => {}
            }
        }
        let src = tokens.join(" ");
        if let Err(e) = agree(src.as_bytes()) {
            failures.push(format!("{src:?}: {e}"));
        }
    }
    failures.truncate(5);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Every `.lua` and `.wowlua` file below the directories in
/// `$WOWLUA_TREE_DIRS` (`:`-separated; run with `cargo test -p
/// wowlua-syntax --lib legacy -- --ignored`).
#[test]
#[ignore]
fn corpus() {
    let Ok(roots) = std::env::var("WOWLUA_TREE_DIRS") else {
        return;
    };
    let mut files = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = roots.split(':').map(Into::into).collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lua") || e.eq_ignore_ascii_case("wowlua"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut failures = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        if let Err(e) = agree(&bytes) {
            failures.push(format!("{}: {e}", path.display()));
        }
    }
    eprintln!("{} files, {} differences", files.len(), failures.len());
    failures.truncate(5);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
