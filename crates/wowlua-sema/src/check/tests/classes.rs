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
