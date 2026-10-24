//! Differential execution of the syntax lowerings (spec compiler §13.11):
//! each wowlua program must print exactly what its hand-written Lua 5.1
//! reference prints, and line-preserving output keeps every line.

use wowlua_luair::{EmitOptions, LineMode, emit};
use wowlua_span::SourceDb;

/// A Lua 5.1 stand-in for WoW's `bit` library (LuaBitOp semantics).
const BIT: &str = r#"
local function tobit(x)
  x = x % 4294967296
  if x >= 2147483648 then x = x - 4294967296 end
  return x
end
local function u(x) return x % 4294967296 end
local function op(a, b, f)
  local r, p = 0, 1
  a, b = u(a), u(b)
  for _ = 1, 32 do
    local x, y = a % 2, b % 2
    if f(x, y) then r = r + p end
    a, b, p = (a - x) / 2, (b - y) / 2, p * 2
  end
  return tobit(r)
end
local function fold(f, ...)
  local args = {...}
  local r = args[1]
  for i = 2, #args do r = op(r, args[i], f) end
  return tobit(r)
end
bit = {
  band = function(...) return fold(function(x, y) return x == 1 and y == 1 end, ...) end,
  bor = function(...) return fold(function(x, y) return x == 1 or y == 1 end, ...) end,
  bxor = function(...) return fold(function(x, y) return x ~= y end, ...) end,
  bnot = function(a) return tobit(-1 - u(a)) end,
  lshift = function(a, n) return tobit(u(a) * 2 ^ (n % 32)) end,
  rshift = function(a, n) return tobit(math.floor(u(a) / 2 ^ (n % 32))) end,
}
"#;

fn run(code: &[u8]) -> String {
    let lua = mlua::Lua::new();
    lua.load(BIT).exec().unwrap();
    lua.load(
        r##"
        __out = {}
        print = function(...)
          local parts = {}
          for i = 1, select("#", ...) do parts[i] = tostring((select(i, ...))) end
          __out[#__out + 1] = table.concat(parts, " ")
        end
        "##,
    )
    .exec()
    .unwrap();
    // The addon table with the runtime, passed like WoW passes `...`.
    let ns = lua.create_table().unwrap();
    let runtime = wowlua_runtime::runtime(&wowlua_runtime::RuntimeOptions {
        slices: [
            wowlua_runtime::Slice::Widget,
            wowlua_runtime::Slice::Enum,
            wowlua_runtime::Slice::Decorate,
        ]
        .into_iter()
        .collect(),
        ..wowlua_runtime::RuntimeOptions::default()
    });
    lua.load(&runtime).call::<()>(("Test", ns.clone())).unwrap();
    if let Err(e) = lua
        .load(code)
        .into_function()
        .and_then(|f| f.call::<()>(("Test", ns)))
    {
        panic!(
            "runtime error: {e}\n--- code ---\n{}",
            String::from_utf8_lossy(code)
        );
    }
    let out: mlua::Table = lua.globals().get("__out").unwrap();
    let lines: Vec<String> = out
        .sequence_values::<String>()
        .map(Result::unwrap)
        .collect();
    lines.join("\n")
}

fn compile(src: &str) -> String {
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    assert!(!parsed.has_errors(), "{src}\n{:?}", parsed.diagnostics);
    let chunk = match wowlua_lower::lower(&parsed, src.as_bytes()) {
        Ok(c) => c,
        Err(e) => panic!(
            "{src}\n{:?}",
            e.iter().map(|d| &d.message).collect::<Vec<_>>()
        ),
    };
    let out = emit(&chunk, &EmitOptions::default());
    let text = String::from_utf8(out.text).unwrap();
    assert_eq!(
        text.lines().count(),
        src.lines().count(),
        "preserve mode must keep the line count\n--- source ---\n{src}\n--- output ---\n{text}"
    );
    for mode in [LineMode::Statement, LineMode::Pretty] {
        let other = emit(
            &chunk,
            &EmitOptions {
                line_mode: mode,
                ..EmitOptions::default()
            },
        );
        assert_eq!(
            run(&other.text),
            run(text.as_bytes()),
            "{mode:?} output behaves differently"
        );
    }
    text
}

#[track_caller]
fn same(wowlua: &str, reference: &str) {
    let compiled = compile(wowlua);
    let expected = run(reference.as_bytes());
    let actual = run(compiled.as_bytes());
    assert_eq!(
        actual, expected,
        "\n--- wowlua ---\n{wowlua}\n--- compiled ---\n{compiled}"
    );
}

#[test]
fn overload_signatures_emit_nothing() {
    let src = "overload function pick(n: number): string\noverload function pick(s: string): number\nlocal function pick(v)\n  return v\nend\nprint(pick(1))";
    let compiled = compile(src);
    assert!(!compiled.contains("overload"), "{compiled}");
    same(src, "local function pick(v) return v end\nprint(pick(1))");
}

#[test]
fn compound_assignment() {
    same(
        "local x = 1\nx += 2\nx *= 3 + 1\nx -= 1\nx /= 2\nx %= 4\nx ^= 2\nprint(x)\nlocal s = 'a'\ns ..= 1 .. 2\nprint(s)",
        "local x = 1 x = x + 2 x = x * (3 + 1) x = x - 1 x = x / 2 x = x % 4 x = x ^ 2 print(x) local s = 'a' s = s .. (1 .. 2) print(s)",
    );
    // Subexpressions with calls run once; `??=` keeps `false`.
    same(
        "local n = 0\nlocal function k() n = n + 1 return 1 end\nlocal t = {5, {v = 1}}\nt[k()] += 10\nt[2].v += 1\nprint(t[1], t[2].v, n)\nlocal u = {k = false}\nu.k ??= 1\nu.m ??= 2\nprint(u.k, u.m)\ncfg = {a = 'x'}\ncfg.a ..= 'y'\nprint(cfg.a)\nlocal i = 7\ni //= 2\nprint(i)\nlocal b = 6\nb &= 3\nb |= 8\nb <<= 1\nb >>= 2\nprint(b)",
        "local n = 0 local function k() n = n + 1 return 1 end local t = {5, {v = 1}} local key = k() t[key] = t[key] + 10 t[2].v = t[2].v + 1 print(t[1], t[2].v, n) print(false, 2) print('xy') print(3) print(5)",
    );
}

#[test]
fn continue_in_every_loop() {
    same(
        "for i = 1, 6 do\n  if i % 2 == 0 then continue end\n  if i == 5 then break end\n  print(i)\nend\nlocal j = 0\nwhile j < 5 do\n  j += 1\n  if j == 2 then continue end\n  print('w', j)\nend\nlocal k = 0\nrepeat\n  k += 1\n  if k == 1 then continue end\n  print('r', k)\nuntil k >= 3\nfor _, v in ipairs({1, 2, 3}) do\n  for x = 1, 2 do\n    if x == 1 then continue end\n    print(v, x)\n  end\n  if v == 2 then continue end\n  print('v', v)\nend",
        "print(1) print(3) print('w', 1) print('w', 3) print('w', 4) print('w', 5) print('r', 2) print('r', 3) print(1, 2) print('v', 1) print(2, 2) print(3, 2) print('v', 3)",
    );
}

#[test]
fn optional_chaining() {
    same(
        "local calls = 0\nlocal function arg() calls = calls + 1 return 1 end\nlocal a = nil\nprint(a?.b, a?.b.c(arg()), a?:m(arg()), a?.[arg()])\nlocal t = {b = {c = function(x) return x + 1, 'extra' end}, f = false}\nprint(t?.b.c(arg()))\nprint(t?.missing?.x, t.f, calls)\nlocal obj = {n = 2, get = function(self, d) return self.n + d end}\nprint(obj?:get(1))\nlocal fn = nil\nfn?.(arg())\nlocal g = function() print('called') end\ng?.()\nprint(calls)",
        "local calls = 0 local function arg() calls = calls + 1 return 1 end print(nil, nil, nil, nil) calls = calls + 1 print(2) print(nil, false, 1) print(3) print('called') print(1)",
    );
}

#[test]
fn nil_coalescing() {
    same(
        "local log = {}\nlocal function f(x) log[#log + 1] = x return x end\nlocal a = false\nprint(a ?? 'default', nil ?? 'd2', f('l') ?? f('r'))\nprint(f(1), nil ?? f(2), f(3))\nprint(table.concat(log, ','))\nlocal v = (nil ?? nil) ?? 'last'\nprint(v)",
        "print(false, 'd2', 'l') print(1, 2, 3) print('l,1,2,3') print('last')",
    );
}

#[test]
fn interpolated_strings() {
    let mut many = String::from("local parts = `");
    for i in 0..60 {
        many.push_str(&format!("{{{i}}}-"));
    }
    many.push_str("`\nprint(#parts)");
    same(
        "local name, n = 'you', 3\nprint(`Hello {name}, {n} item{n == 1 and '' or 's'} {nil} {true}`)\nprint(`\\{literal\\} \\u{48}\\x49 {`nested {n * 2}`}`)\nprint(`plain`, ``)\nprint(`multi\nline {n}`)",
        "print('Hello you, 3 items nil true') print('{literal} HI nested 6') print('plain', '') print('multi\\nline 3')",
    );
    same(
        &many,
        &format!(
            "print({})",
            (0..60).map(|i| i.to_string().len() + 1).sum::<usize>()
        ),
    );
}

#[test]
fn bitwise_and_integer_division() {
    same(
        "local a, b = 12, 10\nprint(a & b, a | b, a ~ b, ~a, a << 2, a >> 1, a | b | 1)\nprint(0xFF & 0x0F, 1 << 33, -1 >> 28, 7 // 2, -7 // 2, a // 5)\nlocal bit = 'shadowed'\nprint(a & 6, bit)",
        "print(8, 14, 6, -13, 48, 6, 15) print(15, 2, 15, 3, -4, 2) print(4, 'shadowed')",
    );
}

#[test]
fn numeric_literals_and_constants() {
    same(
        "print(1_000_000, 0xFF_FF, 0b1010_1010, 3.141_5, 1e1_0)\nconst MAX = 40\nconst HALF = MAX / 2\nconst NAME: string = 'w' .. 'l' .. HALF\nconst T = {1, 2}\nprint(MAX, HALF, NAME, #T)\nfor i = 1, 2 do\n  const now = i * 10\n  print(now)\nend\nconst NAN = 0 / 0\nconst INF = -1 / 0\nprint(NAN ~= NAN, INF)",
        "print(1000000, 65535, 170, 3.1415, 1e10) print(40, 20, 'wl20', 2) print(10) print(20) print(true, -1/0)",
    );
}

#[test]
fn default_parameters_and_destructuring() {
    same(
        "local function clamp(v, lo = 0, hi = lo + 1)\n  return math.max(lo, math.min(hi, v))\nend\nprint(clamp(5), clamp(-1, nil, 3), clamp(0.5, false and 1 or 0.6))\nlocal obj = {}\nfunction obj:greet(who = 'world') return 'hi ' .. who end\nprint(obj:greet(), obj:greet('you'))\nlocal reads = 0\nlocal function info() reads = reads + 1 return {name = 'n', level = nil} end\nlocal { name, level = 1, extra = name .. '!' } = info()\nprint(name, level, extra, reads)\nlocal point = {x = 1, y = 2}\nlocal { x, y } = point\nprint(x, y)",
        "print(1, 0, 0.6) print('hi world', 'hi you') print('n', 1, 'n!', 1) print(1, 2)",
    );
}

#[test]
fn preceding_statements_keep_evaluation_order() {
    same(
        "local log = {}\nlocal function f(x) log[#log + 1] = tostring(x) return x end\nlocal a, b, c = f(1), f(2) ?? f(9), f(3)\nprint(a, b, c)\nif f(nil) ?? f(false) then print('no') elseif f(4) ?? f(9) then print('yes') end\nlocal i = 0\nwhile (i < 2 and i) ?? f('never') do i += 1 end\nrepeat local z = 1 until f(nil) ?? f(true)\nfor k = f(nil) ?? 1, 2 do f('k' .. k) end\nlocal function r() return f(nil) ?? f('ret') end\nprint(r())\nprint(table.concat(log, ','))",
        "print(1, 2, 3) print('yes') print('ret') print('1,2,3,nil,false,4,nil,true,nil,k1,k2,nil,ret')",
    );
}

#[test]
fn shadowed_globals_use_aliases() {
    same(
        "local tostring, math = nil, nil\nprint(`{1}{2}`, 7 // 2)",
        "print('12', 3)",
    );
}

#[test]
fn constants_cannot_be_reassigned() {
    let src = "const X = 1\nX = 2\n";
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    let errors = wowlua_lower::lower(&parsed, src.as_bytes()).unwrap_err();
    assert_eq!(errors[0].code, Some("E2010"));
}

/// Only conditions that read a `const` (or `wowlua:meta`) are static; plain
/// Lua conditions are emitted as written (decision T71).
#[test]
fn static_conditions_need_a_constant() {
    let compiled = compile(
        "const DEBUG = false\nif DEBUG then print('debug') else print('release') end\nprint(DEBUG and f() or 'x')\nif true then print(1) end\nprint(1 == 1 and 2)\n",
    );
    assert!(!compiled.contains("debug"), "{compiled}");
    assert!(!compiled.contains("f()"), "{compiled}");
    assert!(compiled.contains("if true then"), "{compiled}");
    assert!(compiled.contains("1 == 1 and 2"), "{compiled}");
    assert_eq!(run(compiled.as_bytes()), "release\nx\n1\n2");
}

#[test]
fn classes_inheritance_super_and_statics() {
    same(
        r#"class Animal
    name: string
    protected sound: string = "..."
    static count: number = 0
    constructor(name: string)
        self.name = name
        Animal.count += 1
    end
    function speak(): string
        return `{self.name} says {self.sound}`
    end
    static function create(name: string): Animal
        return Animal.new(name)
    end
    get label(): string
        return `{self.name}#{Animal.count}`
    end
    function __tostring(): string
        return `Animal({self.name})`
    end
end
class Dog extends Animal
    tricks: string[] = {}
    constructor(name: string)
        super(name)
        self.sound = "woof"
    end
    override function speak(): string
        return super:speak() .. "!"
    end
end
local d = Dog.new("rex")
local a = Animal.create("cat")
print(d:speak(), a:speak(), Animal.count, Dog.count, d.label, tostring(d), #d.tricks)
local d2 = Dog("max")
print(d2:speak(), Animal.count)"#,
        "print('rex says woof!', 'cat says ...', 2, 2, 'rex#2', 'Animal(rex)', 0) print('max says woof!', 3)",
    );
}

#[test]
fn abstract_classes_accessors_and_implicit_constructors() {
    same(
        r#"abstract class Shape
    abstract function area(): number
    function describe(): string return `area {self:area()}` end
end
class Square extends Shape
    side: number = 2
    override function area(): number return self.side * self.side end
end
class Temperature
    private celsius: number = 0
    get fahrenheit(): number return self.celsius * 9 / 5 + 32 end
    set fahrenheit(value: number) self.celsius = (value - 32) * 5 / 9 end
    static get unit(): string return "F" end
end
local t = Temperature.new()
t.fahrenheit = 212
print(Square.new():describe(), t.fahrenheit, t.celsius, Temperature.unit)
print((pcall(function() Temperature.unit = 1 end)))"#,
        "print('area 4', 212, 100, 'F') print(false)",
    );
}

#[test]
fn enums_are_inlined_unless_used_as_values() {
    let src = "enum Direction { Up, Down, Left = 10, Right }\nenum Anchor { TopLeft = \"TOPLEFT\", Center = \"CENTER\" }\nprint(Direction.Up, Direction.Right, Anchor.Center, Anchor[\"TopLeft\"])\n";
    let compiled = compile(src);
    assert!(!compiled.contains("Direction ="), "{compiled}");
    assert_eq!(run(compiled.as_bytes()), "1 11 CENTER TOPLEFT");
    same(
        "enum Quality { Poor = 0, Common, Rare }\nlocal n = 0\nfor _, v in pairs(Quality) do n += v end\nprint(n, Quality.Rare)",
        "print(3, 2)",
    );
}

#[test]
fn user_decorators_apply_bottom_to_top() {
    same(
        r#"local log = {}
local function tag(name: string)
    return function(fn, ctx)
        log[#log + 1] = name .. ":" .. ctx.kind .. ":" .. ctx.name
        return function(...) return name .. "(" .. fn(...) .. ")" end
    end
end
local function plain(fn, ctx) log[#log + 1] = "plain:" .. ctx.name end
@tag("a")
@tag("b")
local function f(x: number): string
    return tostring(x)
end
@plain
function g() return "g" end
print(f(1), g(), table.concat(log, ","))
local seen = {}
local function register(cls, ctx) seen[#seen + 1] = ctx.kind .. ":" .. ctx.name end
local function field(_, ctx) seen[#seen + 1] = ctx.kind .. ":" .. ctx.name .. ":" .. tostring(ctx.static) end
local function twice(fn, ctx) seen[#seen + 1] = ctx.kind .. ":" .. ctx.name; return function(self, ...) return fn(self, ...) * 2 end end
@register
class Counter
    @field
    static total: number = 1
    @twice
    function value(): number return 21 end
end
print(Counter.new():value(), table.concat(seen, ","))"#,
        "print('a(b(1))', 'g', 'b:function:f,a:function:f,plain:g') print(42, 'field:total:true,method:value,class:Counter')",
    );
}

#[test]
fn core_decorators() {
    same(
        "@global(\"MyAddonApi\")\nlocal api = { v = 1 }\n@debugOnly\nlocal function dbg(m: string) print(m) end\ndbg(\"x\")\nprint(MyAddonApi.v, \"after\")",
        "print(1, 'after')",
    );
}
