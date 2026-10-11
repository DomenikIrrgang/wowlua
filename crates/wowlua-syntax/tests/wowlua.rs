//! The wowlua grammar (spec appendix A) and its syntax errors (E01xx).

use wowlua_span::SourceDb;
use wowlua_syntax::ast::*;
use wowlua_syntax::parse;

fn parse_str(src: &str) -> wowlua_syntax::Parse {
    let mut db = SourceDb::new();
    let file = db.add("t.wowlua", src.to_string());
    parse(src.as_bytes(), file)
}

fn codes(src: &str) -> Vec<String> {
    parse_str(src)
        .diagnostics
        .iter()
        .filter_map(|d| d.code.map(str::to_string))
        .collect()
}

#[track_caller]
fn ok(src: &str) -> Chunk {
    let p = parse_str(src);
    assert!(
        !p.has_errors(),
        "{src:?}: {:?}",
        p.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    p.chunk
}

#[test]
fn statements_and_expressions() {
    for src in [
        "local count: number = 0",
        "local name: string?, realm: string? = UnitName('target')",
        "local frames: Frame[] = {}",
        "local m: Map<string, List<number>> = {}",
        "local t: List<number>= {}",
        "const MAX = 40",
        "const PREFIX: string = 'x'",
        "local { x, y } = point",
        "local { name, level = 1 }: Info = getInfo()",
        "total += amount label ..= 'x' flags |= 1 items[i()] -= 1 cache[k] ??= {} n //= 2 b <<= 1",
        "local function clamp(v: number, lo: number = 0, hi: number = 1): number return v end",
        "function map<T, U>(xs: T[], f: (T) -> U): U[] return {} end",
        "local id = function<T>(x: T): T return x end",
        "function f(...: number) end function g(...: T...) end",
        "function f(): (number, string) end function g(): () end function h(): T... end",
        "function isFrame(x: any): x is Frame return true end",
        "for i, item in ipairs(items) do if item.hidden then continue end show(item) end",
        "for i: number = 1, 10 do end for k: string, v: Info in pairs(t) do end",
        "local a = b?.c?.[d]?:e(f)?.(g)",
        "local v = a ?? b",
        "local s = `Hello {name}, {#items} item{#items == 1 and '' or 's'} \\{x\\} {`nested {x}`}`",
        "local v = raw :: string | number",
        "local m = (flags :: number) | MASK",
        "local x = a & b | c ~ d << 1 >> 2 // 3",
        "local y = ~x",
        "local n = 1_000_000 + 0xFF_FF + 0b1010_1010 + 3.141_592 + 1e1_0",
        "continue = 1",
        "continue.x = 1",
        "type = 5 local t = type(x) import('x') import 'x' export = {}",
    ] {
        ok(src);
    }
}

#[test]
fn declarations() {
    ok("@global('MyAddon')\nlocal function f() end");
    ok("@event('PLAYER_LOGIN') @profile\nfunction M.onLogin() end");
    ok("type Point = { x: number, y: number, [string]: any, ['end']: number }");
    ok("type F = <T>(x: T, ...string) -> (T, number)");
    ok("type U = | 'a' | 'b'");
    ok("type P<T...> = (T...) -> ()");
    ok("declare function pcall<A..., R...>(f: (A...) -> R..., ...: A...): (boolean, R...)");
    ok("type W = (A...) -> ...any");
    ok("type H = (EventArgs<E>...) -> ()");
    ok("type C = Thread<A..., (...any), R...>");
    ok("type O = typeof(defaults) & { extra: boolean? }");
    ok(
        "interface Shape extends A, B\n readonly area: number\n label?: string\n function draw(self, x: number): boolean\nend",
    );
    ok("enum Color { Red, Green = 2; Blue }");
    ok(
        "abstract class Base<T> extends Frame implements Shape, Other\n  private static count: number = 0\n  readonly name?: string\n  static: boolean\n  @trace function greet(self, who: string): string return who end\n  abstract function area(self): number\n  overload function f(self, x: number): number\n  get size(): number return 1 end\n  set size(v: number) end\n  constructor(name: string) self.name = name end\n  overload constructor(x: number)\nend",
    );
    ok("declare function GetTime(): number");
    ok("declare global MyAddonDB: { count: number }");
    ok(
        "declare namespace C_Timer\n function After(seconds: number, @deferred cb: () -> ()): ()\n const MAX: number\n namespace Inner\n  x: number\n end\nend",
    );
    ok(
        "declare class Frame extends Region\n static function new(): Frame\n readonly name: string\n function Show(self): ()\nend",
    );
    ok("declare interface Shape\n function x(self): number\nend");
    ok("declare event PLAYER_LOGIN(isInitial: boolean)");
    ok("declare message CacheReady(count: number)");
    ok("declare module 'lib'\n function f(): number\nend");
    ok("import * as mock from 'wowlua:mock'");
    ok("import _ from './side'");
    ok("import a, b as c from './x'");
    ok("import type Frame, Region as R from 'wow'");
    ok("import type from './x'");
    ok("export local a: number = 1");
    ok("export function f() end");
    ok("export const X = 1");
    ok("export abstract class A\nend");
    ok("export type T = number");
    ok("export interface I\nend");
    ok("export enum E { A }");
}

#[test]
fn syntax_errors_have_codes() {
    let cases: &[(&str, &str)] = &[
        ("local x = 1__0", "E0103"),
        ("local x = 0x_FF", "E0103"),
        ("local x = 1_", "E0103"),
        ("f\n(x)", "E0104"),
        ("local s = `\\q`", "E0105"),
        ("continue", "E0106"),
        ("break", "E0106"),
        ("repeat local done = f() continue until done", "E0107"),
        ("do import a from 'x' end", "E0108"),
        ("@dec\nx = 1", "E0109"),
        ("local v = a ?? b or c", "E0110"),
        ("local v = a and b ?? c", "E0110"),
        ("a, b += 1", "E0111"),
        ("const X", "E0112"),
        ("const X, Y = 1", "E0112"),
        ("a?.b = 1", "E0113"),
        ("a?.b.c += 1", "E0113"),
        ("while x do continue; f() end", "E0114"),
        ("do class A\nend end", "E0115"),
        ("do enum E { A } end", "E0115"),
        ("local s = `a } b`", "E0116"),
        ("import _, a from 'x'", "E0117"),
        ("export function a.b() end", "E0118"),
        ("export local x", "E0119"),
    ];
    let mut failures = Vec::new();
    for (src, code) in cases {
        let found = codes(src);
        if !found.iter().any(|c| c == code) {
            failures.push(format!("{src:?}: expected {code}, got {found:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn recovers_and_reports_several_errors() {
    let p = parse_str("local = 1\nprint('ok')\nlocal function (\nx = 2\nif then end\n");
    let errors = p.diagnostics.iter().filter(|d| d.is_error()).count();
    assert!(errors >= 3, "{:?}", p.diagnostics);
    // The valid statements in between are still in the tree.
    let calls = p
        .chunk
        .block
        .stmts
        .iter()
        .filter(|s| matches!(s.kind, StmtKind::Call(_)))
        .count();
    assert_eq!(calls, 1);
}

#[test]
fn precedence_follows_the_spec() {
    let chunk = ok("x = -a :: number ^ 2 .. b .. c or d");
    let StmtKind::Assign { values, .. } = &chunk.block.stmts[0].kind else {
        panic!()
    };
    // `or` is the root; `..` is right-associative; the cast binds to `a`.
    let ExprKind::Binary {
        op: BinOp::Or, lhs, ..
    } = &values[0].kind
    else {
        panic!("{:?}", values[0].kind)
    };
    let ExprKind::Binary {
        op: BinOp::Concat,
        lhs: first,
        rhs,
        ..
    } = &lhs.kind
    else {
        panic!()
    };
    assert!(matches!(
        rhs.kind,
        ExprKind::Binary {
            op: BinOp::Concat,
            ..
        }
    ));
    let ExprKind::Unary {
        op: UnOp::Neg,
        operand,
    } = &first.kind
    else {
        panic!("{:?}", first.kind)
    };
    assert!(matches!(
        operand.kind,
        ExprKind::Binary { op: BinOp::Pow, .. }
    ));
}

#[test]
fn directives_before_the_first_token() {
    let chunk = ok("-- License: MIT\n--!strict\n--!nolint\nlocal x = 1\n--!basic\n");
    let names: Vec<&str> = chunk.directives.iter().map(|d| d.name.as_ref()).collect();
    assert_eq!(names, ["strict", "nolint"]);
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let src = format!("x = {}1{}", "(".repeat(5000), ")".repeat(5000));
    let p = parse_str(&src);
    assert!(p.diagnostics.iter().any(|d| d.code == Some("E5001")));
    let src = "do ".repeat(3000) + &"end ".repeat(3000);
    assert!(parse_str(&src).has_errors());
}

#[test]
fn decorator_names_may_be_keywords() {
    let chunk = ok("@if(flavor == 'standard')\nlocal x = 1\n");
    assert_eq!(
        chunk.block.stmts[0].decorators[0].path[0].text.as_ref(),
        "if"
    );
}
