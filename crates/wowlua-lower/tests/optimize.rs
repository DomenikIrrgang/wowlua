//! Profile-controlled optimizations (spec optimization §15.2):
//! `cache-globals` and the local half of `dead-code` keep behavior.

use wowlua_luair::{LineMode, Passes, emit};
use wowlua_span::SourceDb;

fn inlined(src: &str) -> (String, Vec<(u32, wowlua_luair::optimize::NotInlined)>) {
    let mut db = SourceDb::new();
    let file = db.add("t.lua", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    let mut chunk = wowlua_lower::lower(&parsed, src.as_bytes()).expect("lowers");
    let reasons = wowlua_luair::optimize::inline(&mut chunk);
    let text = emit(&chunk, &Passes::default().emit_options(LineMode::Preserve)).text;
    (
        String::from_utf8(text).unwrap(),
        reasons.into_iter().map(|(p, r)| (p.line, r)).collect(),
    )
}

fn optimize(src: &str, globals: bool, dead: bool) -> String {
    let mut db = SourceDb::new();
    let file = db.add("t.lua", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
    let mut chunk = wowlua_lower::lower(&parsed, src.as_bytes()).expect("lowers");
    if dead {
        wowlua_luair::optimize::remove_dead_locals(&mut chunk);
    }
    if globals {
        wowlua_luair::optimize::cache_globals(&mut chunk);
    }
    let passes = Passes::default();
    String::from_utf8(emit(&chunk, &passes.emit_options(LineMode::Statement)).text).unwrap()
}

fn run(code: &str) -> String {
    let lua = mlua::Lua::new();
    lua.load(
        r##"__out = {}
        print = function(...)
          local parts = {}
          for i = 1, select("#", ...) do parts[i] = tostring((select(i, ...))) end
          __out[#__out + 1] = table.concat(parts, " ")
        end
        C_Test = { value = function() return 7 end }"##,
    )
    .exec()
    .unwrap();
    lua.load(code)
        .exec()
        .unwrap_or_else(|e| panic!("{e}\n{code}"));
    lua.load("return table.concat(__out, '\\n')")
        .eval()
        .unwrap()
}

const PROGRAM: &str = r##"local function describe(t)
  local parts = {}
  for k, v in pairs(t) do parts[#parts + 1] = string.format("%s=%s", tostring(k), tostring(v)) end
  table.sort(parts)
  return table.concat(parts, ",")
end
local unusedHelper = function() return 1 end
local function alsoUnused() return unusedHelper() end
local sideEffect = print("kept")
print(describe({ a = 1, b = 2 }), type(describe), string.format("%d", 3), C_Test.value(), C_Test.value())
for _, v in ipairs({ 1, 2 }) do print(type(v), select("#", v)) end
"##;

#[test]
fn optimizations_keep_behavior() {
    let expected = run(PROGRAM);
    for (g, d) in [(true, false), (false, true), (true, true)] {
        let out = optimize(PROGRAM, g, d);
        assert_eq!(run(&out), expected, "{out}");
    }
}

#[test]
fn cache_globals_hoists_stable_names() {
    let out = optimize(PROGRAM, true, false);
    let first = out.lines().next().unwrap();
    for name in ["tostring", "type", "C_Test", "__wl_string_format"] {
        assert!(first.contains(name), "{first}");
    }
    // Names read once are not worth a local.
    assert!(
        !first.contains("pairs") && !first.contains("table"),
        "{first}"
    );
    assert!(
        !first.contains("print"),
        "print is hooked by addons: {first}"
    );
    assert!(!out.contains("string.format(\"%d\""), "{out}");
    // An environment change or an assignment keeps the names global.
    let env = optimize("setfenv(1, {})\nprint(type(1), type(2))\n", true, false);
    assert!(!env.starts_with("local"), "{env}");
    let assigned = optimize(
        "type = nil\nprint(type, type)\nstring.format = nil\nprint(string.format, string.format)\n",
        true,
        false,
    );
    assert!(!assigned.starts_with("local"), "{assigned}");
}

#[test]
fn dead_locals_go_until_none_is_left() {
    let out = optimize(PROGRAM, false, true);
    assert!(
        !out.contains("unusedHelper") && !out.contains("alsoUnused"),
        "{out}"
    );
    assert!(
        out.contains("local sideEffect = print(\"kept\")"),
        "calls stay: {out}"
    );
    assert!(out.contains("local function describe"), "{out}");
}

const HELPERS: &str = r##"factor = 10
local function square(x) return x * x end
local function add(a, b) return a + b end
local function scaled(v) return v * factor end
local factor = 3
local function fact(n) if n < 2 then return 1 end return n * fact(n - 1) end
local function rec(n) return n > 0 and rec(n - 1) or 0 end
local function escapes(x) return x end
local stored = escapes
local function twice(x) return square(x) + square(x) end
local function show(x) return tostring(x) end
local y = 4
local t = { add(1, y), square(y) }
print(square(y), add(y, 2) * 2, 10 - add(1, 2), twice(y), t[1], t[2], fact(4), rec(2), stored(5))
print(add(square(2), 1), show(y), scaled(2))
local function wrapper()
  local square = function(z) return -z end
  return square(1), add(y, y)
end
print(wrapper())
square(y)
"##;

#[test]
fn inlining_keeps_behavior() {
    let expected = run(HELPERS);
    let (out, _) = inlined(HELPERS);
    assert_eq!(run(&out), expected, "{out}");
    // Preserve mode keeps the lines.
    assert_eq!(out.lines().count(), HELPERS.lines().count(), "{out}");
}

#[test]
fn inlines_expression_helpers_only() {
    let (out, reasons) = inlined(HELPERS);
    // Called with locals and literals: inlined, and the helper goes.
    assert!(
        out.contains("print(y * y, (y + 2) * 2, 10 - (1 + 2)"),
        "{out}"
    );
    assert!(!out.contains("local function twice"), "{out}");
    // A call statement stays a call, so `square` stays.
    assert!(
        out.contains("local function square") && out.contains("\nsquare(y)"),
        "{out}"
    );
    // Not inlined: a call argument, a name that means something else at the
    // call (`factor` is declared after `scaled`), recursion, escapes.
    assert!(
        out.contains("add(square(2), 1)") || out.contains("add(2 * 2, 1)"),
        "{out}"
    );
    assert!(out.contains("scaled(2)"), "{out}");
    use wowlua_luair::optimize::NotInlined::*;
    assert!(reasons.contains(&(6, NotAnExpression)), "{reasons:?}");
    assert!(reasons.contains(&(7, Recursive)), "{reasons:?}");
    assert!(reasons.contains(&(8, Escapes)), "{reasons:?}");
}
