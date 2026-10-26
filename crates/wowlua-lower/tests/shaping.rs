//! The optimizations that shape code for Lua 5.1 (spec optimization
//! §15.2.3): each applies where its preconditions hold and nowhere else,
//! and the optimized code prints what the code it came from prints.
//!
//! The facts the type checker proves in strict mode ([`Facts`]) are given
//! here by `assume`: every field and index is of a plain table, every
//! operator of operands that take it without a metamethod, every `ipairs`
//! and `table.insert` of a plain array. The programs are written so that
//! this is true; without the facts, the type-based passes change nothing.

use wowlua_luair::ir::*;
use wowlua_luair::optimize::{Shaping, shape};
use wowlua_luair::{LineMode, Passes, emit};
use wowlua_span::SourceDb;

fn lowered(src: &str) -> Chunk {
    let mut db = SourceDb::new();
    let file = db.add("t.lua", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
    wowlua_lower::lower(&parsed, src.as_bytes()).expect("lowers")
}

/// The facts of a program whose tables are plain and whose operators
/// take numbers and strings.
fn assume(chunk: &mut Chunk) {
    struct All;
    impl wowlua_luair::scope::Visitor for All {
        fn expr(&mut self, e: &mut Expr) {
            match &mut e.kind {
                ExprKind::Field { name, .. } => name.set_facts(name.facts().with(Facts::PLAIN)),
                ExprKind::Index { facts, .. } => *facts = Facts::PLAIN,
                ExprKind::Binary { facts, .. } | ExprKind::Unary { facts, .. } => {
                    *facts = Facts::NO_META
                }
                ExprKind::Call { callee, .. } => match &mut callee.kind {
                    ExprKind::Name(f) if f.name == "ipairs" => f.set_facts(Facts::ARG_ARRAY),
                    ExprKind::Name(f) if f.name == "tinsert" => f.set_facts(Facts::ARG_PLAIN),
                    ExprKind::Field { name, .. } if name.name == "insert" => {
                        name.set_facts(name.facts().with(Facts::ARG_PLAIN))
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    wowlua_luair::scope::walk(chunk, &mut All);
}

const ALL: Shaping = Shaping {
    constants: true,
    cache_fields: true,
    cache_upvalues: true,
    numeric_loops: true,
    table_insert: true,
    hoist_closures: true,
};

fn only(f: impl FnOnce(&mut Shaping)) -> Shaping {
    let mut s = Shaping::default();
    f(&mut s);
    s
}

fn text(chunk: &Chunk, mode: LineMode) -> String {
    String::from_utf8(emit(chunk, &Passes::default().emit_options(mode)).text).unwrap()
}

/// `src` shaped with `opts`, with the facts or without, in statement mode.
fn shaped(src: &str, opts: Shaping, facts: bool) -> String {
    shaped_in(src, opts, facts, LineMode::Statement)
}

fn shaped_in(src: &str, opts: Shaping, facts: bool, mode: LineMode) -> String {
    let mut chunk = lowered(src);
    if facts {
        assume(&mut chunk);
    }
    shape(&mut chunk, &opts, mode == LineMode::Preserve);
    text(&chunk, mode)
}

/// `src` as it is emitted without optimizations.
fn plain(src: &str) -> String {
    text(&lowered(src), LineMode::Statement)
}

fn run(code: &str) -> String {
    let lua = mlua::Lua::new();
    lua.load(
        r##"__out = {}
        tinsert = table.insert
        print = function(...)
          local parts = {}
          for i = 1, select("#", ...) do parts[i] = tostring((select(i, ...))) end
          __out[#__out + 1] = table.concat(parts, " ")
        end"##,
    )
    .exec()
    .unwrap();
    if let Err(e) = lua.load(code).exec() {
        return format!("error: {e}");
    }
    lua.load("return table.concat(__out, '\\n')")
        .eval()
        .unwrap()
}

/// Prints the same optimized and not, and returns the optimized code.
fn same_behavior(src: &str, opts: Shaping) -> String {
    let expected = run(&plain(src));
    assert!(!expected.starts_with("error"), "{expected}\n{src}");
    let out = shaped(src, opts, true);
    assert_eq!(run(&out), expected, "{out}");
    out
}

// ---------------------------------------------------------------- constants

#[test]
fn constants_replace_locals_assigned_once() {
    let src = "local LIMIT = 10\nlocal NAME = \"bar\"\nlocal OFF = false\nlocal function f(x) return x * LIMIT, NAME, OFF end\nprint(f(2))\n";
    let out = same_behavior(src, only(|s| s.constants = true));
    assert!(out.contains("x * 10, \"bar\", false"), "{out}");
    assert!(!out.contains("local LIMIT"), "{out}");
}

#[test]
fn constants_leave_other_locals() {
    // Assigned again, a long string, a call's value, an object.
    let src = "local n = 1\nn = n + 1\nlocal s = \"a string that is longer than forty bytes, quotes included\"\nlocal t = {}\nlocal c = math.floor(2.5)\nlocal u = \"x\"\nprint(n, s, t, c, u:upper())\n";
    let out = shaped(src, only(|s| s.constants = true), true);
    assert_eq!(out, plain(src));
}

// ------------------------------------------------------------ numeric loops

#[test]
fn ipairs_over_plain_arrays_becomes_a_numeric_loop() {
    let src = "local t = { 10, 20, 30 }\nfor i, v in ipairs(t) do print(i, v) end\nlocal holes = { 1, 2, nil, 4 }\nfor _, v in ipairs(holes) do print(v) end\nlocal grows = { 1 }\nfor i, v in ipairs(grows) do if i < 4 then grows[#grows + 1] = v * 2 end print(v) end\nlocal box = { items = { \"a\", \"b\" } }\nfor i in ipairs(box.items) do print(i) end\n";
    let out = same_behavior(src, only(|s| s.numeric_loops = true));
    assert!(out.contains("for i = 1, 2 ^ 53 do"), "{out}");
    assert!(out.contains("local v = t[i]"), "{out}");
    assert!(out.contains("if v == nil then"), "{out}");
    // The argument is evaluated once, into a local.
    assert!(out.contains("local __wl_a = box.items"), "{out}");
    assert!(!out.contains("ipairs("), "{out}");
}

#[test]
fn ipairs_stays_without_the_facts_or_when_it_is_assigned() {
    let src = "local t = { 1, 2 }\nfor i, v in ipairs(t) do print(i, v) end\n";
    assert_eq!(
        shaped(src, only(|s| s.numeric_loops = true), false),
        plain(src)
    );
    let assigned =
        "ipairs = ipairs\nlocal t = { 1, 2 }\nfor i, v in ipairs(t) do print(i, v) end\n";
    assert_eq!(
        shaped(assigned, only(|s| s.numeric_loops = true), true),
        plain(assigned)
    );
}

// ------------------------------------------------------------- table.insert

#[test]
fn table_insert_becomes_an_assignment() {
    let src = "local t = {}\nlocal box = { list = {} }\nfor i = 1, 3 do table.insert(t, i * 2) tinsert(box.list, \"x\" .. i) end\nprint(#t, t[3], #box.list, box.list[2])\n";
    let out = same_behavior(src, only(|s| s.table_insert = true));
    assert!(out.contains("t[#t + 1] = i * 2"), "{out}");
    assert!(
        out.contains("box.list[#box.list + 1] = \"x\" .. i"),
        "{out}"
    );
}

#[test]
fn table_insert_stays_where_it_may_differ() {
    // A position, a value that calls (and may change `#t`), a value used,
    // an `insert` the file replaces.
    let src = "local t = {}\nlocal function f() t[#t + 1] = 0 return 1 end\ntable.insert(t, 1, 5)\ntable.insert(t, f())\nprint(table.insert(t, 2))\n";
    assert_eq!(
        shaped(src, only(|s| s.table_insert = true), true),
        plain(src)
    );
    let replaced =
        "local t = {}\ntable.insert = function(a, b) rawset(a, 1, b) end\ntable.insert(t, 1)\n";
    assert_eq!(
        shaped(replaced, only(|s| s.table_insert = true), true),
        plain(replaced)
    );
    let unproven = "local t = {}\ntable.insert(t, 1)\n";
    assert_eq!(
        shaped(unproven, only(|s| s.table_insert = true), false),
        plain(unproven)
    );
}

// -------------------------------------------------------------- cache-fields

#[test]
fn loop_invariant_fields_are_read_before_the_loop() {
    let src = "local self = { frame = { texture = { alpha = 0.5 } }, scale = 2 }\nlocal sum = 0\nfor i = 1, 10 do\n    sum = sum + self.scale * i + self.frame.texture.alpha\nend\nprint(sum)\n";
    let out = same_behavior(src, only(|s| s.cache_fields = true));
    assert!(out.contains("local __wl_scale = self.scale"), "{out}");
    assert!(
        out.contains("local __wl_alpha = self.frame.texture.alpha"),
        "{out}"
    );
    assert!(out.contains("sum + __wl_scale * i + __wl_alpha"), "{out}");
}

#[test]
fn fields_a_loop_may_change_stay_in_it() {
    // A call, a write to the field (of any table), a write to the local.
    for body in [
        "sum = sum + self.scale print(i)",
        "sum = sum + self.scale other.scale = i",
        "sum = sum + self.scale self = other",
        "sum = sum + self.scale other[keys[i]] = 1",
    ] {
        let src = format!(
            "local self = {{ scale = 2 }}\nlocal other = {{ scale = 3 }}\nlocal keys = {{ \"scale\", \"x\", \"y\" }}\nlocal sum = 0\nfor i = 1, 3 do {body} end\nprint(sum)\n"
        );
        let out = shaped(&src, only(|s| s.cache_fields = true), true);
        assert!(!out.contains("__wl_scale"), "{body}\n{out}");
    }
    // A number key names no field: `self.scale` is read once.
    let src = "local self = { scale = 2 }\nlocal other = {}\nlocal sum = 0\nfor i = 1, 3 do sum = sum + self.scale other[i] = 1 other[#other + 1] = 2 end\nprint(sum, #other)\n";
    let out = same_behavior(src, only(|s| s.cache_fields = true));
    assert!(out.contains("local __wl_scale = self.scale"), "{out}");
    // Without the facts nothing is read early.
    let src = "local self = { scale = 2 }\nlocal sum = 0\nfor i = 1, 3 do sum = sum + self.scale end\nprint(sum)\n";
    assert_eq!(
        shaped(src, only(|s| s.cache_fields = true), false),
        plain(src)
    );
}

#[test]
fn a_loop_first_read_after_an_if_stays_in_the_loop() {
    // The checker may have proved `self.frame` only inside the `if`.
    let src = "local self = { frame = { a = 1 } }\nlocal n = 0\nfor i = 1, 3 do\n    if self.frame then\n        n = n + self.frame.a\n    end\nend\nprint(n)\n";
    let out = same_behavior(src, only(|s| s.cache_fields = true));
    assert!(!out.contains("local __wl_a = self.frame.a"), "{out}");
}

#[test]
fn repeated_reads_in_a_stretch_are_read_once() {
    let src = "local self = { pos = { x = 1, y = 2 } }\nlocal x = self.pos.x\nlocal y = self.pos.y\nprint(x + y)\nprint(self.pos.x)\n";
    let out = same_behavior(src, only(|s| s.cache_fields = true));
    assert!(out.contains("local __wl_pos = self.pos"), "{out}");
    assert!(out.contains("local x = __wl_pos.x"), "{out}");
    // A call ends the stretch: `print` may change `self.pos`.
    assert!(out.contains("print(self.pos.x)"), "{out}");
}

#[test]
fn a_stretch_ends_where_the_field_may_change() {
    for between in [
        "f()",
        "self.pos = { x = 3 }",
        "other.pos = 1",
        "self = other",
    ] {
        let src = format!(
            "local self = {{ pos = {{ x = 1 }} }}\nlocal other = {{}}\nlocal function f() end\nlocal a = self.pos.x\n{between}\nlocal b = self.pos.x\nprint(a, b)\n"
        );
        let out = shaped(&src, only(|s| s.cache_fields = true), true);
        assert!(!out.contains("__wl_"), "{between}\n{out}");
    }
}

// ------------------------------------------------------------ hoist-closures

#[test]
fn closures_that_capture_nothing_of_a_loop_are_made_before_it() {
    let src = "local total = 0\nlocal list = { 3, 1, 2 }\nfor i = 1, 3 do\n    local ok = pcall(function() total = total + 1 end)\n    table.sort(list, function(a, b) return a > b end)\n    local function twice(x) return x * 2 end\n    print(ok, twice(i), list[1])\nend\nprint(total)\n";
    let out = same_behavior(src, only(|s| s.hoist_closures = true));
    let loop_at = out.find("for i = 1, 3 do").expect("the loop");
    for made in [
        "local __wl_fn = function()",
        "local __wl_fn2 = function(a, b)",
        "local function __wl_twice(x)",
    ] {
        let at = out.find(made).unwrap_or_else(|| panic!("{made}\n{out}"));
        assert!(at < loop_at, "{made}\n{out}");
    }
}

#[test]
fn closures_that_capture_the_loop_or_may_be_kept_stay() {
    let src = "local kept = {}\nfor i = 1, 3 do\n    pcall(function() print(i) end)\n    kept[i] = function() return 1 end\n    C_Timer = C_Timer\n    local function f() return i end\n    print(f())\n    local g = function() return 2 end\n    kept[i + 3] = g\n    securecallfunction(function() return 3 end)\nend\n";
    let out = shaped(src, only(|s| s.hoist_closures = true), true);
    assert_eq!(out, plain(src));
    // Preserve mode keeps every instruction's line: closures stay.
    let lines = "for i = 1, 3 do\n    pcall(function()\n        print(1)\n    end)\nend\n";
    let out = shaped_in(
        lines,
        only(|s| s.hoist_closures = true),
        true,
        LineMode::Preserve,
    );
    assert!(!out.contains("__wl_fn"), "{out}");
}

// ------------------------------------------------------------ cache-upvalues

#[test]
fn upvalues_read_in_a_loop_become_locals() {
    let src = "local scale = 3\nlocal offsets = { 1, 2 }\nlocal function sum(t)\n    local s = 0\n    for i = 1, #t do s = s + t[i] * scale + offsets[1] end\n    return s\nend\nprint(sum({ 1, 2, 3 }))\n";
    let out = same_behavior(src, only(|s| s.cache_upvalues = true));
    assert!(
        out.contains("local offsets, scale = offsets, scale"),
        "{out}"
    );
}

#[test]
fn upvalues_assigned_or_read_little_stay() {
    // `count` is assigned; `once` read once outside a loop; `f` only called.
    let src = "local count = 0\nlocal once = 1\nlocal function f() end\nlocal function g()\n    for i = 1, 3 do count = count + i f() end\n    return once\nend\nprint(g(), count)\n";
    let out = shaped(src, only(|s| s.cache_upvalues = true), true);
    assert_eq!(out, plain(src));
}

// ------------------------------------------------------- statement inlining

fn inlined(src: &str, mode: LineMode) -> (String, Vec<wowlua_luair::optimize::NotInlined>) {
    let mut chunk = lowered(src);
    let reasons = wowlua_luair::optimize::inline_with(&mut chunk, mode != LineMode::Preserve);
    (
        text(&chunk, mode),
        reasons.into_iter().map(|(_, r)| r).collect(),
    )
}

#[test]
fn call_statements_of_statement_bodies_are_inlined() {
    let src = "local log = {}\nlocal function note(kind, msg)\n    log[#log + 1] = kind .. \": \" .. tostring(msg)\n    if kind == \"error\" then\n        log.errors = (log.errors or 0) + 1\n    end\nend\nnote(\"info\", \"started\")\nfor i = 1, 2 do note(\"error\", \"failed \" .. i) end\nnote(\"info\")\nprint(#log, log[1], log[3], log.errors)\n";
    let expected = run(&plain(src));
    let (out, reasons) = inlined(src, LineMode::Statement);
    assert_eq!(run(&out), expected, "{out}");
    assert!(reasons.is_empty(), "{reasons:?}");
    assert!(!out.contains("function note"), "{out}");
    assert!(
        out.contains("local kind, msg = \"info\", \"started\""),
        "{out}"
    );
    // Missing arguments are `nil`, as in the call.
    assert!(out.contains("local kind, msg = \"info\"\n"), "{out}");
}

#[test]
fn statement_bodies_that_return_or_whose_value_is_used_stay() {
    use wowlua_luair::optimize::NotInlined;
    let early = "local function f(x)\n    if x then return end\n    print(x)\nend\nf(1)\n";
    let (out, reasons) = inlined(early, LineMode::Statement);
    assert!(out.contains("local function f(x)"), "{out}");
    assert_eq!(reasons, [NotInlined::NotAnExpression]);
    let value = "local function g(x)\n    print(x)\nend\nlocal y = g(1)\nprint(y)\n";
    let (out, reasons) = inlined(value, LineMode::Statement);
    assert!(out.contains("local function g(x)"), "{out}");
    assert_eq!(reasons, [NotInlined::ValueUsed]);
    // Preserve mode keeps every instruction's line.
    let kept = "local function h(x)\n    print(x)\n    print(x)\nend\nh(1)\n";
    let (out, reasons) = inlined(kept, LineMode::Preserve);
    assert!(out.contains("local function h(x)"), "{out}");
    assert_eq!(reasons, [NotInlined::ValueUsed]);
    // A name the body uses means something else at the call.
    let shadowed =
        "local x = 1\nlocal function k() print(x) end\ndo\n    local x = 2\n    k()\nend\n";
    let expected = run(&plain(shadowed));
    let (out, _) = inlined(shadowed, LineMode::Statement);
    assert_eq!(run(&out), expected, "{out}");
    assert!(out.contains("local function k()"), "{out}");
}

#[test]
fn long_statement_bodies_are_inlined_only_where_called_once() {
    use wowlua_luair::optimize::NotInlined;
    let body: String = (0..14).map(|i| format!("    print(x + {i})\n")).collect();
    let src = format!("local function f(x)\n{body}end\nf(1)\nf(2)\n");
    let (out, reasons) = inlined(&src, LineMode::Statement);
    assert!(out.contains("local function f(x)"), "{out}");
    assert_eq!(reasons, [NotInlined::TooLarge]);
    let once = format!("local function f(x)\n{body}end\nf(1)\n");
    let (out, reasons) = inlined(&once, LineMode::Statement);
    assert!(!out.contains("local function f(x)"), "{out}");
    assert!(reasons.is_empty(), "{reasons:?}");
}

// ------------------------------------------------------------- differential

/// A program with every pattern the passes rewrite, and the ones near them
/// they must leave: optimized with all of them, it prints the same.
#[test]
fn every_optimization_keeps_behavior() {
    let src = r#"
local LIMIT = 4
local SEP = ", "
local cfg = { scale = 2, names = { "a", "b", "c" } }
local hits = 0
local log = {}
local function add(s) log[#log + 1] = s end
local Unit = { hp = 10, frame = { bar = { width = 100 } } }
local function render(self, values)
    local out = {}
    for i, v in ipairs(values) do
        local w = self.frame.bar.width * v / LIMIT
        table.insert(out, w * cfg.scale)
    end
    local n = 0
    for _, name in ipairs(cfg.names) do n = n + #name end
    for i = 1, LIMIT do
        hits = hits + 1
        pcall(function() add("tick") end)
    end
    local parts = {}
    for i = 1, #out do parts[#parts + 1] = tostring(out[i]) end
    return table.concat(parts, SEP), n
end
local function grow(t)
    for i, v in ipairs(t) do
        if v < 8 then t[#t + 1] = v * 2 end
    end
    return #t
end
print(render(Unit, { 1, 2, 3, nil, 5 }))
print(grow({ 1, 3 }), hits, #log)
local p = { pos = { x = 3, y = 4 } }
local d = p.pos.x * p.pos.x + p.pos.y * p.pos.y
p.pos = { x = 0, y = 0 }
print(d, p.pos.x)
local total = 0
local function each(list, f) for i = 1, #list do f(list[i]) end end
for round = 1, 3 do
    local function bump(v) total = total + v * round end
    each({ 1, 2 }, bump)
end
print(total)
"#;
    let out = same_behavior(src, ALL);
    let (inlined, _) = {
        let mut chunk = lowered(src);
        assume(&mut chunk);
        let r = wowlua_luair::optimize::inline_with(&mut chunk, true);
        shape(&mut chunk, &ALL, false);
        (text(&chunk, LineMode::Statement), r)
    };
    assert_eq!(run(&inlined), run(&plain(src)), "{inlined}");
    assert!(out.contains("2 ^ 53"), "{out}");
}

/// A generated local (a module's table, `__wl_m`) copied into a function
/// stays apart from the user's names: its copy is generated too.
#[test]
fn copies_of_generated_upvalues_are_generated() {
    let src = "local __wl_m = { x = 0 }\nlocal function f()\n    for i = 1, 3 do __wl_m.x = __wl_m.x + i end\n    return __wl_m.x\nend\nprint(f())\n";
    let mut chunk = lowered(src);
    struct Generated;
    impl wowlua_luair::scope::Visitor for Generated {
        fn ident(&mut self, id: &mut Ident, _use: wowlua_luair::scope::Use) {
            if id.name == "__wl_m" {
                id.set_generated(true);
            }
        }
    }
    wowlua_luair::scope::walk(&mut chunk, &mut Generated);
    shape(&mut chunk, &only(|s| s.cache_upvalues = true), false);
    let out = text(&chunk, LineMode::Statement);
    assert!(out.contains("local __wl_m2 = __wl_m"), "{out}");
    assert_eq!(run(&out), "6", "{out}");
}

/// Inlined statements that make functions, with more functions after them:
/// what the inliner counts stays the code it resolved.
#[test]
fn inlined_statements_with_functions_keep_the_rest_in_step() {
    let src = "local log = {}\nlocal function register(name)\n    log[#log + 1] = function() return name end\n    local function again() return name .. name end\n    log[#log + 1] = again\nend\nregister(\"a\")\nregister(\"b\")\nlocal function twice(x) return x * 2 end\nlocal function show(f) print(f()) end\nprint(twice(3), log[1](), log[2](), log[3](), log[4]())\nshow(function() return twice(5) end)\n";
    let expected = run(&plain(src));
    let (out, _) = inlined(src, LineMode::Statement);
    assert_eq!(run(&out), expected, "{out}");
    assert!(!out.contains("local function register"), "{out}");
    assert!(out.contains("print(3 * 2,"), "{out}");
}

/// A body that reads the call stack by level stays a function: inlined,
/// its levels would name other functions.
#[test]
fn bodies_that_use_stack_levels_are_not_inlined() {
    use wowlua_luair::optimize::NotInlined;
    for body in [
        "setfenv(2, {})",
        "error(\"bad\", 2)",
        "local e = getfenv(2)",
        "local s = debugstack(2)",
    ] {
        let src = format!("local function f()\n    {body}\nend\nf()\n");
        let (out, reasons) = inlined(&src, LineMode::Statement);
        assert!(out.contains("local function f()"), "{out}");
        assert_eq!(reasons, [NotInlined::Stack], "{body}");
    }
}
