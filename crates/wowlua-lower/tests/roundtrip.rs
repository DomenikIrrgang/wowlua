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
        compact: false,
        mappings: true,
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

/// A build that makes no source maps emits without mappings; the text must
/// be the same.
#[test]
fn mappings_leave_the_text_unchanged() {
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", SAMPLE.to_string());
    let parsed = wowlua_syntax::parse(SAMPLE.as_bytes(), file);
    let chunk = wowlua_lower::lower(&parsed, SAMPLE.as_bytes()).expect("lowers");
    for line_mode in [LineMode::Preserve, LineMode::Statement, LineMode::Pretty] {
        for comments in [CommentMode::None, CommentMode::License, CommentMode::All] {
            for compact in [false, true] {
                let with = EmitOptions {
                    line_mode,
                    comments,
                    indent: "\t".into(),
                    compact,
                    mappings: true,
                };
                let without = EmitOptions {
                    mappings: false,
                    ..with.clone()
                };
                let (a, b) = (emit(&chunk, &with), emit(&chunk, &without));
                assert_eq!(a.text, b.text, "{line_mode:?} {comments:?} {compact}");
                assert_eq!(a.drift, b.drift, "{line_mode:?} {comments:?} {compact}");
                assert!(!a.mappings.is_empty());
                assert!(b.mappings.is_empty());
            }
        }
    }
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
fn debugging_decorators_check_their_targets() {
    let mut db = SourceDb::new();
    let src = "class A\nend\n@trace\nclass B\nend\n@reflect\nlocal x = 1\n";
    let file = db.add("t.wowlua", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    let errors = wowlua_lower::lower(&parsed, src.as_bytes()).unwrap_err();
    let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "`@trace` cannot be applied to classes",
            "`@reflect` cannot be applied to `local` and `const` declarations"
        ]
    );
}

/// Statement and pretty modes keep a trailing comment after everything its
/// source line lowers to, not on a generated line inside it (spec codegen
/// §14.5); a comment after `then` stays on the `if` line.
#[test]
fn trailing_comments_follow_their_whole_line() {
    let src = "local db = {}\nlocal cache = {}\nlocal show = db.show ?? true -- false stays false\nlocal function sample(unit)\n    cache[unit] ??= {} -- create once\n    print(cache[unit])\nend\nlocal x = 1 -- plain\nif x then -- after then\n    print(x) -- inside\nend -- after end\nif x then print(x) end -- one line\nprint(show) -- last\n";
    for mode in [LineMode::Statement, LineMode::Pretty] {
        let out = String::from_utf8(compile(src, mode, CommentMode::All)).unwrap();
        let line_of = |text: &str| -> &str {
            out.lines()
                .find(|l| l.contains(text))
                .unwrap_or_else(|| panic!("{mode:?}: no {text}\n{out}"))
                .trim()
        };
        assert_eq!(
            line_of("false stays false"),
            "local show = __wl_t1 -- false stays false",
            "{mode:?}\n{out}"
        );
        assert_eq!(
            line_of("create once"),
            "end -- create once",
            "{mode:?}\n{out}"
        );
        assert_eq!(line_of("plain"), "local x = 1 -- plain", "{mode:?}\n{out}");
        assert_eq!(
            line_of("after then"),
            "if x then -- after then",
            "{mode:?}\n{out}"
        );
        assert_eq!(line_of("inside"), "print(x) -- inside", "{mode:?}\n{out}");
        assert_eq!(line_of("after end"), "end -- after end", "{mode:?}\n{out}");
        assert_eq!(line_of("one line"), "end -- one line", "{mode:?}\n{out}");
        assert_eq!(line_of("last"), "print(show) -- last", "{mode:?}\n{out}");
    }
}
