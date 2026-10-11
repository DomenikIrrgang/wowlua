//! Round trip (spec compiler §13.5.3): `emit(lower(parse(file)))` compiles
//! to the same Lua 5.1 bytecode as the file.

use wowlua_luair::bytecode::{Strip, strip};
use wowlua_luair::{CommentMode, EmitOptions, LineMode, emit};
use wowlua_span::SourceDb;

fn dump(src: &[u8]) -> Vec<u8> {
    let lua = mlua::Lua::new();
    lua.load(src)
        .set_name("=t")
        .into_function()
        .unwrap_or_else(|e| panic!("{e}\n{}", String::from_utf8_lossy(src)))
        .dump(false)
}

fn compile(src: &str, mode: LineMode, comments: CommentMode) -> Vec<u8> {
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    assert!(!parsed.has_errors(), "{src}: {:?}", parsed.diagnostics);
    let chunk = wowlua_lower::lower(&parsed, src.as_bytes()).expect("lowers");
    let options = EmitOptions {
        line_mode: mode,
        comments,
        indent: "\t".into(),
    };
    emit(&chunk, &options).text
}

/// Returns (stripped bytecode equal, full dump equal).
fn round_trip(src: &str, mode: LineMode, comments: CommentMode) -> (bool, bool, String) {
    let out = compile(src, mode, comments);
    let a = dump(src.as_bytes());
    let b = dump(&out);
    let s = Strip {
        line_defined: true,
        debug: true,
    };
    let stripped = strip(&a, s) == strip(&b, s);
    (stripped, a == b, String::from_utf8_lossy(&out).into_owned())
}

const SAMPLE: &str = r##"-- Copyright (c) 2026 Example
local addonName, ns = ...
local L = ns.L or {}

--[[ A block
comment ]]
local function clamp(v, lo, hi)
    if v < lo then return lo
    elseif v > hi then
        return hi
    else
        return v
    end
end

local t = {
    a = 1, -- trailing
    [2] = "two";
    'three',
    nested = { x = -1, y = - -2, z = not true },
}

function ns.Module:Method(a, b, ...)
    local n = select("#", ...)
    for i = 1, n do
        local v = select(i, ...)
        t[#t + 1] = v .. "x" .. (a or b) .. 1 .. 2
    end
    for k, v in pairs(t) do
        repeat
            local done = k == v
        until done or not v
    end
    while a and b or not (a == b) do
        a = (b)
        break
    end
    return (f(a)), f(b), ...
end

local s = [==[
long ]] string
]==]
local e = 2 ^ -3 ^ 2 + -x ^ 2 * #t % 3 / 4 - 5
local cmp = a < b == (c > d) ~= (e <= f) == (g >= h)
print(("%d"):format(1), #"abc", t.a.b["c"]:d "e" {f = 1})
f{1, 2}
g"str"
h[[long]]
do local _ = 0x1F + 1e3 + .5 + 5. + 0XAB end
x, y.z, w[1] = 1, 2
;(f or g)(1)
return clamp
"##;

#[test]
fn sample_round_trips_in_every_line_mode() {
    for mode in [LineMode::Preserve, LineMode::Statement, LineMode::Pretty] {
        for comments in [CommentMode::None, CommentMode::License, CommentMode::All] {
            let (stripped, _, out) = round_trip(SAMPLE, mode, comments);
            assert!(stripped, "{mode:?} {comments:?}:\n{out}");
        }
    }
    let (_, full, out) = round_trip(SAMPLE, LineMode::Preserve, CommentMode::All);
    assert!(full, "preserve mode should keep line info:\n{out}");
}

#[test]
fn preserve_mode_keeps_lines_and_comments() {
    let out = compile(SAMPLE, LineMode::Preserve, CommentMode::All);
    let out = String::from_utf8(out).unwrap();
    assert_eq!(out.lines().count(), SAMPLE.lines().count(), "{out}");
    assert!(out.contains("-- trailing"));
    assert!(out.starts_with("-- Copyright (c) 2026 Example\n"));
    let license_only =
        String::from_utf8(compile(SAMPLE, LineMode::Preserve, CommentMode::License)).unwrap();
    assert!(license_only.starts_with("-- Copyright"));
    assert!(!license_only.contains("trailing"));
}

#[test]
fn type_annotations_are_erased() {
    let src = "--!strict\ntype P = { x: number }\ninterface I\n x: number\nend\ndeclare function G(): number\nimport type F from 'x'\nexport type Q = P\nlocal x: number, y: string? = 1, nil\nlocal function f<T>(a: T, ...: number): (T, number)\n return (a :: any) :: T, 1\nend\nfor i: number = 1, 2 do end\n";
    let out = String::from_utf8(compile(src, LineMode::Preserve, CommentMode::None)).unwrap();
    assert_eq!(
        out,
        "\n\n\n\n\n\n\n\nlocal x, y = 1, nil\nlocal function f(a, ...)\n\treturn (a), 1\nend\nfor i = 1, 2 do end\n"
    );
}

#[test]
fn unsupported_syntax_names_the_milestone() {
    let mut db = SourceDb::new();
    let src = "local a = b?.c\nx += 1\nx += 2\n";
    let file = db.add("t.wowlua", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    let errors = wowlua_lower::lower(&parsed, src.as_bytes()).unwrap_err();
    assert_eq!(errors.len(), 2);
    assert_eq!(
        errors[0].message,
        "not available yet: optional chaining (arrives with milestone M2)"
    );
    assert!(
        errors[1].message.contains("used 2 times"),
        "{}",
        errors[1].message
    );
}
