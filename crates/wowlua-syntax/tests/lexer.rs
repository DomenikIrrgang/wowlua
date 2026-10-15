//! Positions the lexer no longer stores: token and trivia lines come from
//! `Lexed::line_of`, a token's leading trivia from the previous token's end.

use wowlua_span::SourceDb;
use wowlua_syntax::Lexed;

/// The Lua line (1-based) of byte `offset`, counted the way Lua 5.1 does:
/// `\n`, `\r`, `\r\n` and `\n\r` are one line break each.
fn naive_line(src: &[u8], offset: u32) -> u32 {
    let mut line = 1;
    let mut i = 0;
    while i < offset as usize {
        match src[i] {
            c @ (b'\n' | b'\r') => {
                let next = src.get(i + 1).copied();
                i += if matches!(next, Some(b'\n' | b'\r')) && next != Some(c) {
                    2
                } else {
                    1
                };
                line += 1;
            }
            _ => i += 1,
        }
    }
    line
}

fn check(name: &str, src: &str, lexed: &Lexed) {
    let src = src.as_bytes();
    for (i, t) in lexed.tokens.iter().enumerate() {
        assert_eq!(
            lexed.line_of(t.start),
            naive_line(src, t.start),
            "{name}: line of token {i} {t:?}"
        );
        assert_eq!(
            lexed.line_of(t.end),
            naive_line(src, t.end),
            "{name}: end line of token {i} {t:?}"
        );
    }
    for (i, t) in lexed.trivia.iter().enumerate() {
        assert_eq!(
            lexed.line_of(t.start),
            naive_line(src, t.start),
            "{name}: line of trivia {i} {t:?}"
        );
    }
    // Every trivia record belongs to exactly one token, in order.
    let mut seen = 0;
    for i in 0..lexed.tokens.len() {
        for t in lexed.trivia_of(i) {
            assert_eq!(*t, lexed.trivia[seen], "{name}: trivia of token {i}");
            seen += 1;
        }
    }
    assert_eq!(
        seen,
        lexed.trivia.len(),
        "{name}: trivia after the last token"
    );
}

const CASES: &[(&str, &str)] = &[
    ("empty", ""),
    ("one line", "local a = 1"),
    ("final newline", "local a = 1\n"),
    (
        "line breaks",
        "a = 1\nb = 2\rc = 3\r\nd = 4\n\re = 5\n\n\r\r\n\n",
    ),
    ("long string", "x = [[a\nb\r\nc]] y = 1\nz = [==[\n]]\n]==]"),
    (
        "block comment",
        "--[[ a\nb ]] x = 1 --[==[\r\n\r\n]==]\ny = 2",
    ),
    ("line comment", "-- a\n-- b\r\nx = 1 -- c"),
    ("escaped newline", "x = 'a\\\nb' y = \"c\\\r\nd\"\nz = 1"),
    ("backtick", "x = `a\nb{1 +\n2}c\r\nd` y = `{`{\n}`}`\nz = 1"),
    ("shebang", "#!/usr/bin/lua\nprint(1)\n"),
    ("bom", "\u{feff}print(1)\r\nprint(2)"),
    ("bom and shebang", "\u{feff}#!lua\r\nprint(1)"),
    ("spaces", " \t\x0b\x0c x \t= \x0c1\n \t"),
    (
        "generic closers",
        "local a: Map<string, Map<string, number>> = {}\nlocal b: Array<Array<number>>= {}\nlocal function f(): A<B<C>>?\nend\nlocal c: X<Y<Z>>>= 1",
    ),
];

#[test]
fn token_and_trivia_lines_match_a_naive_count() {
    for (name, src) in CASES {
        let mut db = SourceDb::new();
        let file = db.add("t.wowlua", src.to_string());
        check(name, src, &wowlua_syntax::lexer::lex(src.as_bytes(), file));
        // The parser splits `>>`, `>=` and `>>=` in types, inserting tokens.
        check(name, src, &wowlua_syntax::parse(src.as_bytes(), file).lexed);
    }
}
