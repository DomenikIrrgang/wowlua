//! `parses` answers exactly `!parse(..).has_errors()` without keeping a
//! syntax tree; `lex_with` without trivia scans and reports the same.

use wowlua_span::SourceDb;
use wowlua_syntax::lexer::{LexOptions, lex, lex_with};
use wowlua_syntax::{parse, parses};

#[track_caller]
fn agree(src: &[u8]) {
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", String::from_utf8_lossy(src).into_owned());
    let full = parse(src, file);
    assert_eq!(
        parses(src, file),
        !full.has_errors(),
        "{:?}: {:?}",
        String::from_utf8_lossy(src),
        full.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let with = lex(src, file);
    let without = lex_with(src, file, LexOptions { trivia: false });
    assert!(without.trivia.is_empty());
    let zeroed: Vec<_> = with
        .tokens
        .iter()
        .map(|t| wowlua_syntax::token::Token {
            trivia_end: 0,
            ..*t
        })
        .collect();
    assert_eq!(without.tokens, zeroed);
    assert_eq!(without.line_starts, with.line_starts);
    assert_eq!(
        format!("{:?}", without.diagnostics),
        format!("{:?}", with.diagnostics)
    );
    assert_eq!((without.bom, without.shebang), (with.bom, with.shebang));
}

#[test]
fn targeted_cases_agree() {
    let nested =
        |open: &str, close: &str, n: usize| format!("x = {}1{}", open.repeat(n), close.repeat(n));
    let mut cases: Vec<Vec<u8>> = [
        // `repeat` with `continue`: E0107 reads the body, so `parses` falls back.
        "repeat local a = 1 continue until a",
        "repeat local a = 1 if a then continue end until b",
        "repeat local a = 1 if a then continue end until f({a})",
        "while x do repeat local a = 1 if a then continue end until a end",
        // `??` mixed with `and`/`or` (E0110), optional chains as targets (E0113).
        "x = a ?? b or c",
        "x = (a ?? b) or c",
        "a?.b = 1",
        "a?.b.c = 1",
        // Ambiguous call syntax (E0104), `...` outside a vararg function.
        "local f = g\n(h)()",
        "function f() return ... end",
        "function f(...) return ... end",
        // A byte that starts no token.
        "x = 1 $ 2",
        "x = \u{1}",
        // Statements after `return`/`break`, `break` outside a loop.
        "return 1 x = 2",
        "do break end",
        "while true do break x = 1 end",
        // Unfinished long strings and comments, `[[` inside a level-0 one.
        "x = [[abc",
        "--[[ abc",
        "--[[ a [[ b ]]",
        "x = [[ a [[ b ]]",
        "x = [==[ a ]] ]==]",
        // Malformed numerals and bad escapes.
        "x = 1e",
        "x = 0x",
        "x = 3..2",
        "x = 1.2.3",
        "x = '\\999'",
        "x = '\\q'",
        "x = 'abc",
        // Tables of every field form, nested, with trailing separators.
        "t = { 1, 'a', [k] = v, n = 1, { m = { 2; 3; }, }, }",
        "t = { n = }",
        "t = { [k] }",
        "t = { 1 2 }",
        "t = { a = 1, b = 2, a.b = 3 }",
        // Decorators, exports, imports, classes: kept nodes read back.
        "@inline\nlocal function f() return 1 end",
        "@inline\nx = 1",
        "export function a.b() end",
        "export function a() end",
        "import _ from 'x'",
        "import * as _ from 'x'",
        "class A\n  x: number\nend",
        "local t: (number) = 1",
        "local f: (a: number) -> string = g",
        "local f: ((number)) -> () = g",
        // Directives, a BOM, a `#!` first line.
        "--!strict\nlocal x = 1",
        "\u{feff}local x = 1",
        "\u{feff}local x = ",
        "#!/usr/bin/lua\nlocal x = 1",
        "#!/usr/bin/lua\nlocal x =",
        // Every line break form, inside and around strings and comments.
        "local x = 1\r\nlocal y = 2\n\rlocal z = 3\rreturn x",
        "x = 'a\\\r\nb'",
        "x = [[a\r\n\n\rb]] -- c\r\n",
        "x = 'a\nb'",
        // Backtick strings.
        "x = `a {b} c`",
        "x = `a {b c`",
        "x = `a {{1, 2}} c`",
        "",
        "  \n -- only a comment",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    // NUL bytes inside and outside strings.
    cases.push(b"x = 'a\0b'".to_vec());
    cases.push(b"x = 1\0".to_vec());
    cases.push(b"x = [[a\0]]".to_vec());
    cases.push(b"-- \0\nx = 1".to_vec());
    // Nesting near the limit (wowlua counts 200 levels).
    for n in [197, 198, 199, 200, 201] {
        cases.push(nested("{", "}", n).into_bytes());
        cases.push(nested("(", ")", n).into_bytes());
        cases.push(nested("{ a = ", "}", n).into_bytes());
        cases.push(("do ".repeat(n) + &"end ".repeat(n)).into_bytes());
        cases.push(("f(".repeat(n) + &")".repeat(n)).into_bytes());
    }
    for src in &cases {
        agree(src);
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

/// Lua 5.1 and wowlua tokens, weighted towards what the validating mode
/// leaves out (tables, lists, blocks, literals) and the checks that read
/// back (`repeat`/`continue`, `??`, `?.`, decorators, `export`).
const VOCAB: &[&str] = &[
    "local", "function", "end", "if", "then", "else", "while", "do", "for", "in", "repeat",
    "until", "return", "break", "continue", "and", "or", "not", "nil", "a", "b", "f", "1", "0x1F",
    "1e", "'s'", "[[l]]", "\"\\n\"", "...", "=", "==", "+", "-", "..", "(", ")", "{", "}", "{",
    "}", "[", "]", ";", ",", ",", ".", ":", "??", "?.", "+=", "@inline", "export", "const", "a = ",
    "{ n = 1,", "`x{a}`", "\n", "\n", "--c\n", "--[[c]]",
];

#[test]
fn random_token_sequences_agree() {
    let mut rng = Rng(0x0DDB_1A5E_5BAD_5EED);
    for _ in 0..30_000 {
        let len = 1 + rng.next() % 16;
        let tokens: Vec<&str> = (0..len).map(|_| rng.pick(VOCAB)).collect();
        agree(tokens.join(" ").as_bytes());
    }
}

/// Every `.lua` file below `$WOWLUA_CORPUS` (run with
/// `cargo test -p wowlua-syntax --test validate corpus -- --ignored`); also
/// worth running on the `dist/` of built projects, which is emitted code.
#[test]
#[ignore]
fn corpus() {
    let Ok(root) = std::env::var("WOWLUA_CORPUS") else {
        return;
    };
    let mut files = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
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
    for path in &files {
        agree(&std::fs::read(path).unwrap());
    }
    eprintln!("{} files agree", files.len());
}
