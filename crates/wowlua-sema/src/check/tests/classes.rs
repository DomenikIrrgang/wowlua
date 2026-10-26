//! Classes, interfaces, decorators and names (spec chapters 3 and 4).

use super::*;

/// The codes `src` reports in strict mode.
fn strict_codes(src: &str) -> Vec<String> {
    codes(&check_source(src, Mode::Strict))
}

#[track_caller]
fn has(found: &[String], code: &str) -> bool {
    found.iter().any(|c| c.starts_with(&format!("{code}:")))
}

/// Two interfaces with one name in one scope are E1031 (spec §3.3), at the
/// top level and in a block; only ambient `declare interface`s merge, and
/// `@if` alternatives the build does not both keep are fine.
#[test]
fn duplicate_interfaces_are_e1031() {
    let twice = "interface Named\n    name: string\nend\ninterface Named\n    id: number\nend\nlocal n: Named = { name = \"x\" }\nprint(n)\n";
    let found = strict_codes(twice);
    assert!(has(&found, "E1031"), "{found:#?}");
    let found = strict_codes(
        "export interface Named\n    name: string\nend\ninterface Named\n    id: number\nend\n",
    );
    assert!(has(&found, "E1031"), "{found:#?}");
    let found = strict_codes(
        "local function f()\n    interface Row\n        a: number\n    end\n    interface Row\n        b: number\n    end\nend\nf()\n",
    );
    assert!(has(&found, "E1031"), "{found:#?}");
    // Different scopes, and ambient declarations.
    strict_ok(
        "interface Row\n    a: number\nend\nlocal function f()\n    interface Row\n        b: number\n    end\nend\nf()\nlocal r: Row = { a = 1 }\nprint(r)\n",
    );
    strict_ok(
        "declare interface Shared\n    a: number\nend\ndeclare interface Shared\n    b: number\nend\nlocal s: Shared = { a = 1, b = 2 }\nprint(s)\n",
    );
    // One `@if` alternative or the other: the build keeps one.
    let found = strict_codes(
        "@if(false)\ninterface Named\n    name: string\nend\ninterface Named\n    id: number\nend\nlocal n: Named = { id = 1 }\nprint(n)\n",
    );
    assert!(!has(&found, "E1031"), "{found:#?}");
}

/// `implements I` is checked where the class is declared (E3006, spec
/// §3.1.8): a missing member, or one whose type doesn't fit.
#[test]
fn implements_checks_the_class() {
    let iface = "interface Named\n    name: string\n    nick?: string\n    function greet(other: string): string\nend\n";
    // Missing a method.
    let found = strict_codes(&format!(
        "{iface}class A implements Named\n    name: string = \"a\"\nend\nprint(A)\n"
    ));
    assert!(
        found
            .iter()
            .any(|c| c.starts_with("E3006:") && c.contains("`greet`")),
        "{found:#?}"
    );
    // A field of the wrong type.
    let found = strict_codes(&format!(
        "{iface}class A implements Named\n    name: number = 1\n    function greet(other: string): string return other end\nend\nprint(A)\n"
    ));
    assert!(
        found
            .iter()
            .any(|c| c.starts_with("E3006:") && c.contains("`name`")),
        "{found:#?}"
    );
    // A private member is not part of the shape.
    let found = strict_codes(&format!(
        "{iface}class A implements Named\n    private name: string = \"a\"\n    function greet(other: string): string return other end\nend\nprint(A)\n"
    ));
    assert!(has(&found, "E3006"), "{found:#?}");
    // Fits: optional members may be missing, inherited ones count, and
    // interfaces it extends are part of it.
    strict_ok(&format!(
        "{iface}class A implements Named\n    name: string = \"a\"\n    function greet(other: string): string return other end\nend\nprint(A)\n"
    ));
    strict_ok(&format!(
        "{iface}class Base\n    name: string = \"a\"\nend\nclass A extends Base implements Named\n    function greet(other: string): string return other end\nend\nprint(A)\n"
    ));
    let found = strict_codes(&format!(
        "{iface}interface Loud extends Named\n    function shout(): string\nend\nclass A implements Loud\n    name: string = \"a\"\n    function greet(other: string): string return other end\nend\nprint(A)\n"
    ));
    assert!(
        found
            .iter()
            .any(|c| c.starts_with("E3006:") && c.contains("`shout`")),
        "{found:#?}"
    );
    // A generic interface with its arguments.
    strict_ok(
        "interface Comparable<T>\n    function compare(other: T): number\nend\nclass V implements Comparable<V>\n    n: number = 0\n    function compare(other: V): number return self.n - other.n end\nend\nprint(V)\n",
    );
    let found = strict_codes(
        "interface Comparable<T>\n    function compare(other: T): number\nend\nclass V implements Comparable<V>\n    function compare(other: V): string return \"x\" end\nend\nprint(V)\n",
    );
    assert!(has(&found, "E3006"), "{found:#?}");
    // Basic mode checks it too.
    err(
        &format!("{iface}class A implements Named\n    name: string = \"a\"\nend\nprint(A)\n"),
        "E3006",
    );
}

/// A class extends a class (E3008, spec §3.1.4): not an interface, a type
/// alias, an enum or a value that is no class.
#[test]
fn extends_a_non_class_is_e3008() {
    for (decl, base) in [
        ("interface I\n    x: number\nend\n", "I"),
        ("type T = { x: number }\n", "T"),
        ("enum E { A, B }\n", "E"),
        ("local tbl = {}\n", "tbl"),
    ] {
        let found = strict_codes(&format!("{decl}class A extends {base}\nend\nprint(A)\n"));
        assert!(
            found
                .iter()
                .any(|c| c.starts_with("E3008:") && c.contains(&format!("`{base}`"))),
            "{base}: {found:#?}"
        );
        assert!(!has(&found, "E2002"), "{base}: {found:#?}");
    }
    strict_ok("class B\nend\nclass A extends B\nend\nprint(A)\n");
}

/// Writing a `private` or `protected` member from outside is E2011, as
/// reading one is; static members too (spec §3.1.1).
#[test]
fn assigning_a_hidden_member_is_e2011() {
    let class = "class A\n    private x: number = 1\n    protected y: number = 2\n    private static n: number = 0\n    function set()\n        self.x = 3\n        self.y = 4\n        A.n = 5\n    end\nend\n";
    for (code, name) in [
        ("local a = A.new()\na.x = 2\n", "x"),
        ("local a = A.new()\na.y = 2\n", "y"),
        ("A.n = 2\n", "n"),
        ("print(A.n)\n", "n"),
    ] {
        let found = strict_codes(&format!("{class}{code}"));
        assert!(
            found
                .iter()
                .any(|c| c.starts_with("E2011:") && c.contains(&format!("`{name}`"))),
            "{code}: {found:#?}"
        );
    }
    strict_ok(&format!(
        "{class}class B extends A\n    function more() self.y = 1 end\nend\nlocal b = B.new()\nb:set()\nb:more()\n"
    ));
}

/// A subclass without a constructor takes its superclass's parameters
/// whatever the order the files (or the declarations) come in (spec
/// §3.1.2).
#[test]
fn inherited_constructors_do_not_depend_on_order() {
    let base = "export class Base\n    constructor(a: number, b: string) print(a, b) end\nend\n";
    let sub = "import Base from \"./z\"\nexport class Sub extends Base\nend\nlocal s = Sub.new(1, \"x\")\nprint(s)\n";
    for order in [
        [("b.wowlua", sub), ("z.wowlua", base)],
        [("z.wowlua", base), ("b.wowlua", sub)],
    ] {
        let errs = check_modules(&order, Mode::Strict);
        assert!(errs.is_empty(), "{errs:#?}");
    }
    let errs = check_modules(&[("b.wowlua", sub), ("z.wowlua", base)], Mode::Strict);
    assert!(errs.is_empty(), "{errs:#?}");
    let errs = check_modules(
        &[
            (
                "b.wowlua",
                "import Base from \"./z\"\nlocal s = Sub2.new()\nexport class Sub2 extends Base\nend\nprint(s)\n",
            ),
            ("z.wowlua", base),
        ],
        Mode::Strict,
    );
    assert!(
        errs.iter().any(|e| e.starts_with("E2007")),
        "missing arguments: {errs:#?}"
    );
    // One file, the subclass first.
    strict_ok(
        "class Sub extends Base\nend\nclass Base\n    constructor(a: number) print(a) end\nend\nprint(Sub.new(1))\n",
    );
}

/// `__wowlua` on the addon namespace is the runtime's: assigning it is
/// E2003 (spec §2.4.7), not a type mismatch.
#[test]
fn assigning_wowlua_on_the_namespace_is_e2003() {
    // (the runtime's declarations give `ns.__wowlua` its type)
    let runtime = "declare interface WowluaRuntime\n    version: number\nend\n";
    let with_runtime = |src: &str, mode: Mode| codes(&check_with(src, mode, Some(runtime), None));
    let found = with_runtime("local _, ns = ...\nns.__wowlua = 1\n", Mode::Strict);
    assert!(has(&found, "E2003"), "{found:#?}");
    assert!(!has(&found, "E2001"), "{found:#?}");
    let found = with_runtime("local _, ns = ...\nns.__wowlua = {}\n", Mode::Basic);
    assert!(has(&found, "E2003"), "{found:#?}");
    // Reading it is fine.
    let found = with_runtime(
        "local _, ns = ...\nlocal rt = ns.__wowlua\nprint(rt)\n",
        Mode::Strict,
    );
    assert!(!found.iter().any(|c| c.starts_with('E')), "{found:#?}");
}

/// The language's own types (`DecoratorContext`, spec §4.5).
const LANGUAGE: &str = include_str!("../../../../wowlua-stdlib/decl/wowlua.d.wowlua");

/// The codes `src` reports with the language's types.
fn decorated(src: &str, mode: Mode) -> Vec<String> {
    codes(&check_with(src, mode, Some(LANGUAGE), None))
}

/// A decorator's arguments are evaluated where the declaration is: a name
/// that is neither in scope nor a declared global is E1003, in either mode
/// (spec §4.5.2), and arguments are checked against the factory's
/// parameters.
#[test]
fn decorator_arguments_are_checked() {
    let factory = "local function throttle(sec: number): (fn: (...any) -> (), ctx: DecoratorContext) -> (...any) -> ()\n    return function(fn: (...any) -> (), _: DecoratorContext): (...any) -> ()\n        return function(...: any) if sec > 0 then fn(...) end end\n    end\nend\n";
    for mode in [Mode::Basic, Mode::Strict] {
        let found = decorated(
            &format!("{factory}@throttle(delay)\nlocal function f() end\nf()\n"),
            mode,
        );
        assert!(
            found
                .iter()
                .any(|c| c.starts_with("E1003:") && c.contains("`delay`")),
            "{found:#?}"
        );
        assert!(!has(&found, "W1001") && !has(&found, "E1001"), "{found:#?}");
    }
    let found = decorated(
        &format!("{factory}@throttle(\"x\")\nlocal function f() end\nf()\n"),
        Mode::Basic,
    );
    assert!(has(&found, "E2001"), "{found:#?}");
    // A user decorator's argument too, and the decorator's own path.
    let found = decorated(
        "local function deco(_: any): FunctionDecorator<() -> ()>\n    return function(fn: () -> (), _: DecoratorContext) return fn end\nend\n@deco(missing)\nlocal function f() end\nf()\n",
        Mode::Basic,
    );
    assert!(has(&found, "E1003"), "{found:#?}");
    // In scope: locals, upvalues and declared globals; a function in an
    // argument runs later, and is checked as such.
    let found = decorated(
        &format!(
            "local delay = 0.5\n{factory}@throttle(delay)\nlocal function f() end\n@throttle(math.max(delay, 1))\nlocal function g() end\nf()\ng()\n"
        ),
        Mode::Strict,
    );
    assert!(!found.iter().any(|c| c.starts_with('E')), "{found:#?}");
}

/// A user decorator's parameters take the value and the context (E3022),
/// and the declared name gets the decorator's result (spec §4.5.1).
#[test]
fn decorators_are_typed() {
    let memoize = "local function memoize<A, R>(fn: (A) -> R, _: DecoratorContext): (A) -> R\n    return fn\nend\n";
    // Accepts: generic, its type parameters from the function.
    let found = decorated(
        &format!(
            "{memoize}@memoize\nlocal function lookup(id: number): string return tostring(id) end\nlocal s: string = lookup(1)\nprint(s)\n"
        ),
        Mode::Strict,
    );
    assert!(!found.iter().any(|c| c.starts_with('E')), "{found:#?}");
    // A function the decorator does not take.
    let found = decorated(
        "local function onNumbers(fn: (n: number) -> (), _: DecoratorContext) return fn end\n@onNumbers\nlocal function f(s: string) print(s) end\nf(\"x\")\n",
        Mode::Basic,
    );
    assert!(
        found
            .iter()
            .any(|c| c.starts_with("E3022:") && c.contains("@onNumbers")),
        "{found:#?}"
    );
    // A context parameter that is no context, a third parameter, a value
    // that is no function.
    for deco in [
        "local function d(fn: () -> (), ctx: number) return fn end\n",
        "local function d(fn: () -> (), ctx: DecoratorContext, extra: string) return fn end\n",
        "local d = 5\n",
    ] {
        let found = decorated(
            &format!("{deco}@d\nlocal function f() end\nf()\n"),
            Mode::Basic,
        );
        assert!(has(&found, "E3022"), "{deco}{found:#?}");
    }
    // The result changes the signature: callers see it.
    let wrap = "local function stringly(fn: (n: number) -> (), _: DecoratorContext): (s: string) -> ()\n    return function(s: string) fn(tonumber(s) or 0) end\nend\n";
    let found = decorated(
        &format!("{wrap}@stringly\nlocal function f(n: number) print(n) end\nf(\"12\")\n"),
        Mode::Strict,
    );
    assert!(!found.iter().any(|c| c.starts_with('E')), "{found:#?}");
    let found = decorated(
        &format!("{wrap}@stringly\nlocal function f(n: number) print(n) end\nf(12)\n"),
        Mode::Strict,
    );
    assert!(has(&found, "E2001"), "{found:#?}");
    // Also a module's hoisted function, and an exported one.
    let found = decorated(
        &format!("{wrap}@stringly\nfunction F(n: number) print(n) end\nF(\"1\")\n"),
        Mode::Basic,
    );
    assert!(!found.iter().any(|c| c.starts_with('E')), "{found:#?}");
    let wrap_any = wrap.replace("DecoratorContext", "any");
    let errs = check_modules(
        &[
            (
                "m.wowlua",
                &format!(
                    "{wrap_any}@stringly\nexport function f(n: number) print(n) end\nf(\"1\")\n"
                ),
            ),
            ("main.wowlua", "import f from \"./m\"\nf(\"2\")\nf(3)\n"),
        ],
        Mode::Strict,
    );
    assert!(errs.len() == 1 && errs[0].starts_with("E2001"), "{errs:#?}");
    // `nil` (or `F?`) keeps the function; a decorator typed `any` too.
    let found = decorated(
        "local function keep(fn: (...any) -> (), _: DecoratorContext): nil return nil end\nlocal function opt<F>(fn: F, _: DecoratorContext): F? return fn end\nlocal untyped: any = nil\n@keep\n@opt\n@untyped\nlocal function f(n: number) print(n) end\nf(1)\n",
        Mode::Strict,
    );
    assert!(!found.iter().any(|c| c.starts_with('E')), "{found:#?}");
}

/// Methods, constructors, accessors, fields and classes: their decorators
/// take the method's function (`self` first), the class table, or `nil`.
#[test]
fn member_decorators_are_typed() {
    let method_deco = "local function logged<F>(fn: F, _: DecoratorContext): F return fn end\nlocal function onlyStrings(fn: (s: string) -> (), _: DecoratorContext) return fn end\nlocal function register(_: nil, _: DecoratorContext) end\nlocal function classDeco<C>(c: C, _: DecoratorContext): C return c end\n";
    let found = decorated(
        &format!(
            "{method_deco}@classDeco\nclass A\n    @register\n    x: number = 1\n    @logged\n    constructor() end\n    @logged\n    function f(n: number): number return n end\n    @logged\n    get g(): number return 1 end\nend\nlocal n: number = A.new():f(1)\nprint(n)\n"
        ),
        Mode::Strict,
    );
    assert!(!found.iter().any(|c| c.starts_with('E')), "{found:#?}");
    // A method's function takes `self` first: a decorator for plain
    // `(string) -> ()` functions does not take it.
    let found = decorated(
        &format!(
            "{method_deco}class A\n    @onlyStrings\n    function f(s: string) print(s) end\nend\nprint(A)\n"
        ),
        Mode::Basic,
    );
    assert!(has(&found, "E3022"), "{found:#?}");
    // A field decorator receives `nil`.
    let found = decorated(
        &format!("{method_deco}class A\n    @onlyStrings\n    x: number = 1\nend\nprint(A)\n"),
        Mode::Strict,
    );
    assert!(has(&found, "E3022"), "{found:#?}");
    // A decorated method's callers see the decorator's result.
    let found = decorated(
        "local function toString(fn: (self: any, n: number) -> (), _: DecoratorContext): (self: any, s: string) -> ()\n    return function(self: any, s: string) fn(self, tonumber(s) or 0) end\nend\nclass A\n    @toString\n    function f(n: number) print(n) end\nend\nA.new():f(\"1\")\n",
        Mode::Strict,
    );
    assert!(!found.iter().any(|c| c.starts_with('E')), "{found:#?}");
}

/// W3021: a local function named like a core decorator is ignored in
/// decorator position (spec §4.2); re-exports of the core decorators are
/// not.
#[test]
fn core_decorator_names_hide_bindings() {
    let found = decorated(
        "local function profile(fn: () -> (), _: DecoratorContext) return fn end\n@profile\nlocal function f() end\nf()\nprint(profile)\n",
        Mode::Basic,
    );
    assert!(
        found
            .iter()
            .any(|c| c.starts_with("W3021:") && c.contains("`profile`")),
        "{found:#?}"
    );
    // A value that is no function, or no binding at all: nothing.
    let found = decorated(
        "local trace = 1\n@trace\nlocal function f() end\nf()\nprint(trace)\n@profile\nlocal function g() end\ng()\n",
        Mode::Basic,
    );
    assert!(!has(&found, "W3021"), "{found:#?}");
}

/// Frame types as the API's declarations give them: scripts with typed
/// handlers, and others only the script type alias names.
const WIDGETS: &str = "type ScriptType = string
type ScriptFrame = ScriptType | \"OnShow\" | \"OnGamePadStick\"
type ScriptButton = ScriptFrame | \"OnClick\"
@widget(\"Frame\")
declare class Frame
    function SetScript(scriptType: \"OnShow\", handler: ((self: Self) -> ())?)
    function SetScript(scriptType: ScriptFrame, handler: ((...any) -> ())?)
end
@widget(\"Button\")
declare class Button extends Frame
    function SetScript(scriptType: \"OnShow\", handler: ((self: Self) -> ())?)
    function SetScript(scriptType: \"OnClick\", handler: ((self: Self, button: string, down: boolean) -> ())?)
    function SetScript(scriptType: ScriptButton, handler: ((...any) -> ())?)
    function HookScript(scriptType: \"OnClick\", handler: (self: Self, button: string, down: boolean) -> ())
    function HookScript(scriptType: ScriptButton, handler: (...any) -> ())
end
";

/// `@script(name)` names a script of the widget type, and the method takes
/// that script's arguments (E2001, spec §3.2.1).
#[test]
fn script_handlers_are_checked() {
    let check = |members: &str| {
        let src = format!("class B extends Button\n{members}end\nprint(B)\n");
        codes(&check_with(&src, Mode::Strict, Some(WIDGETS), None))
    };
    let fine = check(
        "    @script(\"OnClick\")\n    function clicked(button: string, down: boolean) print(button, down) end\n    @script(\"OnClick\", { hook = true })\n    function also(button: string) print(button) end\n    @script(\"OnShow\")\n    function shown() end\n    @script(\"OnGamePadStick\")\n    function stick(...: any) print(...) end\n",
    );
    assert!(!fine.iter().any(|c| c.starts_with('E')), "{fine:#?}");
    let found = check("    @script(\"OnClik\")\n    function clicked() end\n");
    assert!(
        found
            .iter()
            .any(|c| c.starts_with("E2001:") && c.contains("`OnClik` is not a script of `Button`")),
        "{found:#?}"
    );
    let found =
        check("    @script(\"OnClick\")\n    function clicked(button: number) print(button) end\n");
    assert!(
        found
            .iter()
            .any(|c| c.starts_with("E2001:") && c.contains("cannot handle `OnClick`")),
        "{found:#?}"
    );
    // A subclass of a widget class: the frame type its chain ends in.
    let src = "class B extends Button\nend\nclass C extends B\n    @script(\"OnClik\")\n    function clicked() end\nend\nprint(C)\n";
    let found = codes(&check_with(src, Mode::Strict, Some(WIDGETS), None));
    assert!(has(&found, "E2001"), "{found:#?}");
}
