//! Accept/reject parity with Lua 5.1 (spec compiler §13.5.2): every chunk
//! Lua 5.1 accepts must parse without errors, and every chunk it rejects
//! must be rejected, unless the chunk uses wowlua syntax.

use wowlua_span::SourceDb;
use wowlua_syntax::{features::features, parse, parses};

fn lua_accepts(src: &[u8]) -> Result<(), String> {
    let lua = mlua::Lua::new();
    lua.load(src)
        .into_function()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `Ok(())` if wowlua and Lua 5.1 agree on `src`.
fn agree(src: &[u8]) -> Result<(), String> {
    let mut db = SourceDb::new();
    let file = db.add("t.lua", String::from_utf8_lossy(src).into_owned());
    let parsed = parse(src, file);
    let ours = !parsed.has_errors();
    assert_eq!(
        parses(src, file),
        ours,
        "`parses` and `parse` disagree: {src:?}"
    );
    let lua = lua_accepts(src);
    match (ours, &lua) {
        (true, Ok(())) | (false, Err(_)) => Ok(()),
        (true, Err(e)) => {
            if features(&parsed.chunk).is_empty() {
                Err(format!("accepted, but Lua 5.1 rejects: {e}"))
            } else {
                Ok(()) // a wowlua extension
            }
        }
        (false, Ok(())) => Err(format!(
            "rejected, but Lua 5.1 accepts: {:?}",
            parsed
                .diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        )),
    }
}

const VALID: &[&str] = &[
    "local a = 1",
    "a = b\n;(f)()",
    "return",
    "return;",
    "return ...",
    "a=1;",
    "x = 1 y = 2",
    "x = .5 y = 5. z = 0xFF w = 0XAB v = 1E5 u = 1e+5 t = 1e-5 s = 08",
    "x = 0x1p4",
    "x = '\\q' y = '\\65\\066' z = '\\\n'",
    "x = [=[ ]] ]=] y = [==[\n]==]",
    "--[==[ c ]==] x = 1",
    "--[= x\nx = 1",
    "--[[ a ]] --[==[ [[ ]==]",
    "x = 1 -- c",
    "while true do break end",
    "repeat local x = 1 until x",
    "for i=1,2 do end for k,v in pairs(t) do end for i=1,2,3 do end",
    "if a then elseif b then else end",
    "function f(...) return ... end",
    "local function f(a, b, ...) end",
    "function a.b:c() end",
    "a.b:c'x' f{} f'' f[[x]] f\n{} f\n'x'",
    "x = a.b['c'].d(e):f(g)",
    "x = (f)() y = (f()) z = {f()}",
    "x = {[1]=2, a=3; 4,}",
    "x = - - 1 y = not not true z = 2^-3 w = #t",
    "x = a and b or c y = a == b == c z = 'a' .. 'b' .. 1 w = 1 .. 2",
    "x = a\n.b",
    "local x, y",
    "a.b.c = 1, 2 a, b = 1",
    "do end",
    "x = 1\r\ny = 2\n\rz = 3\r\rw = 4",
    "local t = { f = function() end, }",
    "x = nil == false",
    "local a = {} a[1] = 2",
    "x = 1 + 2 * 3 ^ 4 ^ 5 .. 6 .. 7 < 8 and 9 or 10",
];

const INVALID: &[&str] = &[
    "f\n()",
    "x = f(g)\n(h)()",
    "x = [[\n[[]]",
    "--[[ a [[ b ]]",
    "x = 0x",
    "x = 1e",
    "x = 3..2",
    "x = 1..2",
    "x = 'a\\300'",
    ";",
    "a=1;;",
    "return 1; x=1",
    "break",
    "while true do break; x=1 end",
    "function f() return ... end",
    "local a <const> = 1",
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
    "x = [= ]",
    "x = 'abc",
    "x = \"abc\nd\"",
    "x = function() end ()",
    "local function() end",
    "x = 1 = 2",
    "x = '\\",
    "x = [==[ ]=]",
    "--[[ unfinished",
    "if a then",
    "x = ",
    "f(",
    "x = !a",
    "x = a $ b",
];

#[test]
fn hand_picked_cases_agree() {
    let mut failures = Vec::new();
    for src in VALID {
        if let Err(e) = agree(src.as_bytes()) {
            failures.push(format!("valid {src:?}: {e}"));
        }
        assert!(
            lua_accepts(src.as_bytes()).is_ok(),
            "fixture should be valid Lua: {src:?}"
        );
    }
    for src in INVALID {
        if let Err(e) = agree(src.as_bytes()) {
            failures.push(format!("invalid {src:?}: {e}"));
        }
        assert!(
            lua_accepts(src.as_bytes()).is_err(),
            "fixture should be invalid Lua: {src:?}"
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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

/// Lua 5.1 tokens only: wowlua's extra tokens and contextual keywords are
/// left out, so a disagreement is a real parity bug.
const VOCAB: &[&str] = &[
    "local", "function", "end", "if", "then", "else", "elseif", "while", "do", "for", "in",
    "repeat", "until", "return", "break", "and", "or", "not", "nil", "true", "false", "a", "b",
    "f", "x", "1", "2.5", "'s'", "[[l]]", "...", "=", "==", "~=", "<", "<=", ">", ">=", "+", "-",
    "*", "/", "%", "^", "#", "..", "(", ")", "{", "}", "[", "]", ";", ",", ".", ":", "\n",
];

#[test]
fn random_token_sequences_agree() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut failures = Vec::new();
    for _ in 0..20_000 {
        let len = 1 + rng.next() % 12;
        let tokens: Vec<&str> = (0..len).map(|_| rng.pick(VOCAB)).collect();
        let src = tokens.join(" ");
        if let Err(e) = agree(src.as_bytes()) {
            failures.push(format!("{src:?}: {e}"));
        }
    }
    failures.truncate(20);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn mutated_valid_chunks_agree() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut failures = Vec::new();
    for _ in 0..20_000 {
        let base = VALID[(rng.next() % VALID.len() as u64) as usize];
        let mut tokens: Vec<String> = base.split(' ').map(str::to_string).collect();
        let at = (rng.next() % (tokens.len() as u64 + 1)) as usize;
        match rng.next() % 3 {
            0 if at < tokens.len() => {
                tokens.remove(at);
            }
            1 => tokens.insert(at.min(tokens.len()), rng.pick(VOCAB).to_string()),
            _ if at < tokens.len() => tokens[at] = rng.pick(VOCAB).to_string(),
            _ => {}
        }
        let src = tokens.join(" ");
        if let Err(e) = agree(src.as_bytes()) {
            failures.push(format!("{src:?}: {e}"));
        }
    }
    failures.truncate(20);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Parity on every `.lua` file below `$WOWLUA_CORPUS` (run with
/// `cargo test -p wowlua-syntax --test parity corpus -- --ignored`).
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
                .is_some_and(|e| e.eq_ignore_ascii_case("lua"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut failures = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap();
        // WoW skips a byte order mark; Lua's loadbuffer does not.
        let src = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        if src.starts_with(b"#") {
            continue; // `lua` skips a first `#` line; the game does not.
        }
        if let Err(e) = agree(src) {
            failures.push(format!("{}: {e}", path.display()));
        }
    }
    eprintln!("{} files, {} disagreements", files.len(), failures.len());
    failures.truncate(30);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
