//! Minification (spec optimization §15.4): every pass but `dedupe-strings`
//! leaves the bytecode unchanged once debug information is stripped, and
//! `preserve` mode keeps every instruction's line. `dedupe-strings` is
//! checked by running the code.

use wowlua_luair::bytecode::{Strip, line_info, strip};
use wowlua_luair::{LineMode, Passes, emit};
use wowlua_span::SourceDb;

const ALL_BUT_DEDUPE: &[&str] = &[
    "strip-comments",
    "strip-whitespace",
    "compact",
    "shorten-literals",
    "simplify",
    "rename-locals",
];

fn dump(src: &[u8]) -> Vec<u8> {
    let lua = mlua::Lua::new();
    lua.load(src)
        .set_name("=t")
        .into_function()
        .unwrap_or_else(|e| panic!("{e}\n{}", String::from_utf8_lossy(src)))
        .dump(false)
}

fn minify(src: &[u8], passes: &[&str], mode: LineMode) -> Vec<u8> {
    let mut db = SourceDb::new();
    let file = db.add("t.lua", String::from_utf8_lossy(src).into_owned());
    let parsed = wowlua_syntax::parse(src, file);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
    let mut chunk = wowlua_lower::lower(&parsed, src).expect("lowers");
    let passes = Passes::from_names(passes.iter().copied());
    wowlua_luair::minify::apply(&mut chunk, &passes, mode);
    emit(&chunk, &passes.emit_options(mode)).text
}

fn run(code: &[u8]) -> String {
    let lua = mlua::Lua::new();
    lua.load(
        r##"__out = {}
        print = function(...)
          local parts = {}
          for i = 1, select("#", ...) do parts[i] = tostring((select(i, ...))) end
          __out[#__out + 1] = table.concat(parts, " ")
        end"##,
    )
    .exec()
    .unwrap();
    lua.load(code)
        .set_name("=t")
        .exec()
        .unwrap_or_else(|e| panic!("{e}\n{}", String::from_utf8_lossy(code)));
    lua.load("return table.concat(__out, '\\n')")
        .eval()
        .unwrap()
}

fn corpus() -> Vec<(String, Vec<u8>)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus-sample");
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "lua"))
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    files.push(("program".into(), PROGRAM.as_bytes().to_vec()));
    files
}

const STRIPPED: Strip = Strip {
    line_defined: true,
    debug: true,
};

#[test]
fn passes_keep_the_bytecode() {
    for (name, src) in corpus() {
        let original = strip(&dump(&src), STRIPPED);
        for pass in ALL_BUT_DEDUPE {
            for mode in [LineMode::Preserve, LineMode::Statement] {
                let out = minify(&src, &[pass], mode);
                assert!(
                    strip(&dump(&out), STRIPPED) == original,
                    "{name}: `{pass}` in {mode:?} changes the bytecode:\n{}",
                    String::from_utf8_lossy(&out)
                );
            }
        }
        for mode in [LineMode::Preserve, LineMode::Statement] {
            let out = minify(&src, ALL_BUT_DEDUPE, mode);
            assert!(
                strip(&dump(&out), STRIPPED) == original,
                "{name}: all passes in {mode:?} change the bytecode:\n{}",
                String::from_utf8_lossy(&out)
            );
        }
    }
}

#[test]
fn preserve_mode_keeps_every_line() {
    for (name, src) in corpus() {
        let out = minify(&src, ALL_BUT_DEDUPE, LineMode::Preserve);
        assert_eq!(
            line_info(&dump(&out)),
            line_info(&dump(&src)),
            "{name}:\n{}",
            String::from_utf8_lossy(&out)
        );
    }
}

/// A test program that prints what it computes.
const PROGRAM: &str = r##"-- Copyright (c) 2026 Example: kept
local function pair() return 1, 2 end
local a, b = (pair())
print(a, b)
local x = 1 local y = x + 1
print(x, y)
local count = 0
local function bump(n) count = count + n return count end
local p = 1 local q = bump(p)
print(p, q, not (p == q), not (p ~= q), not (p < q))
local t = { "a string that repeats", "a string that repeats", key = 1000000, half = 0.5 }
print(t[1] == t[2], t.key, t.half, 1e300 * 10 > 1e308, 0x10, [[long
string]], '\65\066', "\"q\"", 'it\'s')
local function shadow(v)
  local v = v .. "!"
  do local v = 3 print(v) end
  return v
end
print(shadow("x"), select("#", (pair())))
local obj = { name = "obj" }
function obj:get(suffix) return self.name .. suffix end
print(obj:get "a string that repeats", ("%d-%s"):format(7, "a string that repeats"))
local list = {}
for i = 1, 3 do list[#list + 1] = function() return i * count end end
for _, f in ipairs(list) do print(f()) end
local s = 0
repeat local step = 2 s = s + step until s > 5 or step == 0
print(s, - -2, 2 ^ -1, (2 ^ 3) ^ 2, 2 ^ 3 ^ 2, "a" .. "b" .. ("c" .. "d"), 10 - (4 - 3))
local state = { counter = 0, label = "state", 10 }
function state.bump(by) state.counter = state.counter + by return state.counter end
state.extra = state.bump(2) + state[1]
print(state.label, state.counter, state.extra, state.missing)
local escaping = { visible = 1 }
print(rawget(escaping, "visible"), escaping.visible)
"##;

#[test]
fn every_level_behaves_the_same() {
    let expected = run(PROGRAM.as_bytes());
    let all: Vec<&str> = ALL_BUT_DEDUPE
        .iter()
        .copied()
        .chain(["dedupe-strings", "rename-private"])
        .collect();
    for mode in [LineMode::Preserve, LineMode::Statement] {
        for passes in [&["dedupe-strings"][..], &["rename-private"], &all] {
            let out = minify(PROGRAM.as_bytes(), passes, mode);
            assert_eq!(
                run(&out),
                expected,
                "{passes:?} {mode:?}:\n{}",
                String::from_utf8_lossy(&out)
            );
        }
    }
}

#[test]
fn levels_shrink_the_code() {
    let src = PROGRAM.as_bytes();
    let strip = minify(
        src,
        &["strip-comments", "strip-whitespace", "simplify"],
        LineMode::Preserve,
    );
    let text = String::from_utf8(strip.clone()).unwrap();
    // License headers stay; lines stay.
    assert!(text.starts_with("-- Copyright"));
    assert_eq!(text.lines().count(), PROGRAM.lines().count());
    assert!(!text.contains("\t"));
    let all: Vec<&str> = ALL_BUT_DEDUPE
        .iter()
        .copied()
        .chain(["dedupe-strings"])
        .collect();
    let max = String::from_utf8(minify(src, &all, LineMode::Statement)).unwrap();
    assert!(max.len() < strip.len() * 3 / 4, "{max}");
    // The repeated string is written once.
    assert_eq!(max.matches("a string that repeats").count(), 1, "{max}");
    assert!(max.contains("1e6") && max.contains(".5"), "{max}");
    // Every statement keeps a line of its own.
    assert!(max.lines().count() > 20, "{max}");
}

#[test]
fn simplify_rewrites_safely() {
    let out = |src: &str| {
        String::from_utf8(minify(src.as_bytes(), &["simplify"], LineMode::Statement)).unwrap()
    };
    assert_eq!(out("local a = (f())\n"), "local a = (f())\n");
    assert_eq!(out("x = (a + b) * c\n"), "x = (a + b) * c\n");
    assert_eq!(out("x = (a * b) + (c)\n"), "x = a * b + c\n");
    assert_eq!(out("x = not (a == b)\n"), "x = a ~= b\n");
    assert_eq!(out("x = not (a < b)\n"), "x = not (a < b)\n");
    assert_eq!(out("local a = 1\nlocal b = 2\n"), "local a, b = 1, 2\n");
    assert_eq!(
        out("local a = 1\nlocal b = a\n"),
        "local a = 1\nlocal b = a\n"
    );
    assert_eq!(
        out("local a = f()\nlocal b = g()\n"),
        "local a = f()\nlocal b = g()\n"
    );
    assert_eq!(
        out("local a, b = f()\nlocal c = 1\n"),
        "local a, b = f()\nlocal c = 1\n"
    );
    // In preserve mode only statements on one line merge.
    let preserve = |src: &str| {
        String::from_utf8(minify(src.as_bytes(), &["simplify"], LineMode::Preserve)).unwrap()
    };
    assert_eq!(
        preserve("local a = 1\nlocal b = 2\n"),
        "local a = 1\nlocal b = 2\n"
    );
    assert_eq!(preserve("local a = 1 local b = 2\n"), "local a, b = 1, 2\n");
}

#[test]
fn rename_private_renames_fields_of_local_tables() {
    let out = |src: &str| {
        String::from_utf8(minify(
            src.as_bytes(),
            &["rename-private"],
            LineMode::Statement,
        ))
        .unwrap()
    };
    assert_eq!(
        out("local t = { alpha = 1 }\nfunction t.beta() return t.alpha end\nt.alpha = t.beta()\n"),
        "local t = {a = 1}\nfunction t.b()\nreturn t.a\nend\nt.a = t.b()\n"
    );
    // Passed, called with `:`, indexed or assigned: the names stay.
    for src in [
        "local t = { alpha = 1 }\nprint(t)\n",
        "local t = { alpha = 1 }\nt:alpha()\n",
        "local t = { alpha = 1 }\nprint(t[\"alpha\"])\n",
        "local t = { alpha = 1 }\nt = {}\n",
        "local t = { [\"alpha\"] = 1 }\nprint(t.alpha)\n",
        "local t = { alpha = 1 }\nfunction t:m() end\n",
    ] {
        assert!(out(src).contains("alpha"), "{src}");
    }
}

#[test]
fn rename_keeps_globals_and_self() {
    let src =
        "local value = 1\nfunction obj:m(param, arg) return self, param, arg, value, print end\n";
    let out = String::from_utf8(minify(
        src.as_bytes(),
        &["rename-locals", "compact"],
        LineMode::Statement,
    ))
    .unwrap();
    // `arg` keeps its name (see `rename.rs`).
    assert_eq!(
        out,
        "local a=1\nfunction obj:m(b,arg)\nreturn self,b,arg,a,print\nend\n"
    );
}
