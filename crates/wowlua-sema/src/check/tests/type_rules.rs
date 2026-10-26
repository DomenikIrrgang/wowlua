//! The type rules of spec §2 (types) and §5.1.1 (function overloads), and
//! the diagnostics they report.

use super::*;

/// The errors of `src` checked in `mode`, as `code: message`.
fn errors(src: &str, mode: Mode) -> Vec<String> {
    check_source(src, mode)
        .iter()
        .filter(|x| x.is_error())
        .map(|x| format!("{}: {}", x.code.unwrap_or("-"), x.message))
        .collect()
}

/// Assigning to a `const`, an enum or a class reports one E2010 that
/// names what it is, and nothing about the value's type.
#[test]
fn assigning_a_constant_is_one_error() {
    assert_eq!(
        errors("const X = 40\nX = 41\nprint(X)\n", Mode::Basic),
        ["E2010: cannot assign to the constant `X`"]
    );
    assert_eq!(
        errors(
            "local function f()\n    const Y = 1\n    local function g() Y = 2 end\n    g()\nend\nf()\n",
            Mode::Strict
        ),
        ["E2010: cannot assign to the constant `Y`"]
    );
    assert_eq!(
        errors("enum E { A = 1 }\nE = 1\n", Mode::Basic),
        ["E2010: cannot assign to the enum `E`"]
    );
    assert_eq!(
        errors("class C\nend\nC = 2\n", Mode::Basic),
        ["E2010: cannot assign to the class `C`"]
    );
}

/// Assigning to an imported binding is E1010, and to an imported folded
/// constant E2010, as the lowering reports them.
#[test]
fn assigning_an_import_is_e1010() {
    let d = check_modules(
        &[
            ("a.wowlua", "export const A = 1\nexport function f() end\n"),
            (
                "main.wowlua",
                "import A, f from \"./a\"\nA = 2\nf = nil\nprint(A, f)\n",
            ),
        ],
        Mode::Basic,
    );
    assert_eq!(
        d,
        [
            "E2010: cannot assign to the constant `A`",
            "E1010: cannot assign to the imported binding `f`"
        ]
    );
}

/// The diagnostics of `src` checked in `mode` with the extra declarations
/// `extra`, as `code: message`.
fn with_decls(src: &str, mode: Mode, extra: &'static str) -> Vec<String> {
    check_with(src, mode, Some(extra), None)
        .iter()
        .map(|x| format!("{}: {}", x.code.unwrap_or("-"), x.message))
        .collect()
}

const SECRET_HEALTH: &str = "declare function UnitHealth(unit: string): number | Secret<number>\n";

/// A forbidden use of a possibly secret value is W2040 in basic mode
/// (E2040 in strict mode), and only that: comparing it is not also E2001
/// (spec §2.11).
#[test]
fn comparing_a_secret_is_only_the_secret_diagnostic() {
    let secret = "`number | Secret<number>` may be a secret value; narrow it with `canaccessvalue` or `issecretvalue` first";
    for (mode, code) in [(Mode::Basic, "W2040"), (Mode::Strict, "E2040")] {
        assert_eq!(
            with_decls(
                "local hp = UnitHealth(\"target\")\nif hp < 1000 then print(hp) end\n",
                mode,
                SECRET_HEALTH
            ),
            [format!("{code}: {secret}")]
        );
    }
    // the readable part is still compared as usual
    let d = with_decls(
        "local hp = UnitHealth(\"target\")\nif hp < \"x\" then print(hp) end\n",
        Mode::Basic,
        SECRET_HEALTH,
    );
    assert!(
        d.iter().any(|x| x.starts_with("E2001: cannot compare")),
        "{d:#?}"
    );
}

/// A secret used as a table key, read or written, is reported too.
#[test]
fn a_secret_table_key_is_reported() {
    for src in [
        "local hp = UnitHealth(\"target\")\nlocal t = {}\nt[hp] = 1\n",
        "local hp = UnitHealth(\"target\")\nlocal t: { [number]: string } = {}\nprint(t[hp])\n",
        "local hp = UnitHealth(\"target\")\nprint({ [hp] = true })\n",
    ] {
        let d = with_decls(src, Mode::Basic, SECRET_HEALTH);
        assert!(d.iter().any(|x| x.starts_with("W2040")), "{src}\n{d:#?}");
    }
}

const ACCOUNT: &str = "local Account = {}\nAccount.__index = Account\nfunction Account:deposit(amount: number)\n    self.balance += amount\nend\nfunction Account.new(balance: number)\n    return setmetatable({ balance = balance }, Account)\nend\nfunction Account:withdraw(amount: number)\n    self.balance -= amount\n    self.withdrawn = true\nend\nlocal acc = Account.new(100)\n";

/// `setmetatable(o, C)` with `C.__index = C` gives `o` the methods of `C`
/// (spec §2.4.6), and their calls are checked.
#[test]
fn setmetatable_with_a_self_index() {
    let d = check_source(&format!("{ACCOUNT}acc:deposit(\"5 gold\")\n"), Mode::Basic);
    assert!(
        d.iter().any(|x| x.code == Some("E2001")),
        "{:#?}",
        codes(&d)
    );
    for mode in [Mode::Basic, Mode::Strict] {
        let d = check_source(
            &format!("{ACCOUNT}acc:deposit(5)\nprint(acc.balance + 1)\n"),
            mode,
        );
        assert!(d.iter().all(|x| !x.is_error()), "{:#?}", codes(&d));
    }
}

/// The methods of the `__index` table are known by name, those defined
/// after the `setmetatable` call and the fields its methods set on `self`
/// too, so in strict mode a misspelled one is E2003 (spec §2.4.6).
#[test]
fn misspelled_setmetatable_methods() {
    strict_ok(&format!("{ACCOUNT}acc:withdraw(1)\nprint(acc.withdrawn)\n"));
    strict_err(&format!("{ACCOUNT}acc:depositt(5)\n"), "E2003");
    strict_err(&format!("{ACCOUNT}acc:withdrew(5)\n"), "E2003");
    strict_err(&format!("{ACCOUNT}print(acc.balanse)\n"), "E2003");
    // `{ __index = C }` too
    let inline = "local Account = {}\nfunction Account:deposit(amount: number) print(amount) end\nfunction Account.new()\n    return setmetatable({}, { __index = Account })\nend\nfunction Account:withdraw(amount: number) print(amount) end\nlocal acc = Account.new()\n";
    strict_ok(&format!("{inline}acc:deposit(1)\nacc:withdraw(2)\n"));
    strict_err(&format!("{inline}acc:depositt(1)\n"), "E2003");
    // a metatable whose `__index` is another table
    let other = "local Base = {}\nfunction Base:hello() print(\"hi\") end\nlocal mt = {}\nmt.__index = Base\nlocal o = setmetatable({}, mt)\n";
    strict_ok(&format!("{other}o:hello()\n"));
    strict_err(&format!("{other}o:helo()\n"), "E2003");
}

/// In strict mode reading a field an open table doesn't have, and that the
/// file never stores in it, is E2003 (decision T87 keeps it `any` in basic
/// mode). A field stored somewhere may be set by the time it is read.
#[test]
fn misspelled_fields_of_open_tables() {
    let stats =
        "local function f()\n    local stats = { deaths = 0, kills = 0 }\n    stats.total = 0\n";
    strict_err(
        &format!("{stats}    print(stats.deths)\nend\nf()\n"),
        "E2003",
    );
    strict_err(&format!("{stats}    stats:report()\nend\nf()\n"), "E2003");
    strict_ok(&format!(
        "{stats}    print(stats.deaths + stats.total)\nend\nf()\n"
    ));
    ok(&format!("{stats}    print(stats.deths)\nend\nf()\n"));
    // set on some path or later in a loop: not misspelled
    strict_ok(
        "local function f(c: boolean)\n    local t = {}\n    if c then t.x = 1 end\n    print(t.x)\n    for i = 1, 3 do\n        if t.seen then print(i) end\n        t.seen = true\n    end\nend\nf(true)\n",
    );
    // another function may run after the field is set
    strict_ok(
        "local M = {}\nfunction M.a(): number return M.b() end\nfunction M.b(): number return 1 end\nlocal t = {}\nlocal function g() print(t.later) end\nt.later = 1\ng()\nprint(M.a())\n",
    );
}

/// A script's global `function f` with overload signatures: the calls in
/// its file see the overloads (spec §5.1.1), before and after it.
#[test]
fn overloads_of_a_global_function() {
    let f = "overload function findItem(id: number): string?\noverload function findItem(name: string): string?\nfunction findItem(key: number | string): string?\n    return tostring(key)\nend\n";
    let d = check_source(&format!("{f}print(findItem(true))\n"), Mode::Basic);
    assert!(
        d.iter().any(|x| x.code == Some("E2012")),
        "{:#?}",
        codes(&d)
    );
    let d = check_source(
        &format!("local function early() return findItem(true) end\n{f}print(early())\n"),
        Mode::Basic,
    );
    assert!(
        d.iter().any(|x| x.code == Some("E2012")),
        "{:#?}",
        codes(&d)
    );
    ok(&format!("{f}print(findItem(1), findItem(\"x\"))\n"));
    // the results are the overload's
    err(&format!("{f}local n: number = findItem(1)\n"), "E2001");
}

/// `type` and `interface` in a block are scoped to it (spec §1.3): used in
/// the block, unknown after it.
#[test]
fn types_declared_in_a_block() {
    let src = "do\n    interface P\n        x: number\n        q: Q\n    end\n    type Q = { y: number }\n    local p: P = { x = 1, q = { y = 2 } }\n    print(p.x, p.q.y)\nend\nlocal function f<T>(v: T): number\n    type Box = { value: T, n: number }\n    local b: Box = { value = v, n = 1 }\n    return b.n\nend\nprint(f(1))\n";
    strict_ok(src);
    err(
        "do\n    type Q = { y: number }\n    local q: Q = { y = \"no\" }\n    print(q)\nend\n",
        "E2001",
    );
    // a block's `typeof` alias reads the block's values
    err(
        "local function f()\n    local defaults = { scale = 1 }\n    type D = typeof(defaults)\n    local d: D = { scale = \"x\" }\n    print(d)\nend\nf()\n",
        "E2001",
    );
    // gone after the block; an outer type of the name again
    err(
        "do\n    type Q = number\nend\nlocal q: Q = 1\nprint(q)\n",
        "E2002",
    );
    ok(
        "type Q = string\ndo\n    type Q = number\n    local n: Q = 1\n    print(n)\nend\nlocal s: Q = \"x\"\nprint(s)\n",
    );
}

/// The codes of `src`'s diagnostics checked in basic mode.
fn codes_of(src: &str) -> Vec<&'static str> {
    check_source(src, Mode::Basic)
        .iter()
        .filter_map(|x| x.code)
        .collect()
}

/// E2053: an alias that refers to itself outside a table or function type
/// (spec §2.3).
#[test]
fn unguarded_recursive_aliases() {
    for src in [
        "type A = A | nil\n",
        "type B = C\ntype C = B\n",
        "type D = string | (D)?\n",
        "local function f()\n    type E = E | number\nend\nf()\n",
    ] {
        assert!(
            codes_of(src).contains(&"E2053"),
            "{src}: {:?}",
            codes_of(src)
        );
    }
    for src in [
        "type Tree = { value: number, children: Tree[] }\n",
        "type F = (f: F) -> F\n",
        "type L = L[]\n",
        "type N = { next: N? } | nil\n",
        "type Box<T> = { v: T }\ntype R = Box<R> | number\n",
    ] {
        assert!(
            !codes_of(src).contains(&"E2053"),
            "{src}: {:?}",
            codes_of(src)
        );
    }
}

/// E2054: merged `declare interface` declarations give a field two types
/// (spec §2.4.7).
#[test]
fn conflicting_merged_interface_fields() {
    let d = check_with(
        "print(1)\n",
        Mode::Basic,
        Some(
            "declare interface AddonNamespace\n    db: string\n    n: number\nend\ndeclare interface AddonNamespace\n    db: number\n    n: number\nend\n",
        ),
        None,
    );
    let e: Vec<_> = d.iter().filter(|x| x.code == Some("E2054")).collect();
    assert_eq!(e.len(), 1, "{:#?}", codes(&d));
    assert!(e[0].message.contains("`db`"), "{}", e[0].message);
}

/// E2056: a type pack where only one type can stand (spec §2.2.10).
#[test]
fn type_packs_in_invalid_positions() {
    for src in [
        "local function f<T...>(...: T...): T\n    return (...)\nend\nprint(f(1))\n",
        "local function g<T...>(x: T, ...: T...)\n    print(x, ...)\nend\ng(1)\n",
        "type P<T...> = { items: T[] }\nlocal p: P<number>? = nil\nprint(p)\n",
        "type Box<T> = { v: T }\nlocal function h<T...>(b: Box<T>, ...: T...)\n    print(b, ...)\nend\nh({ v = 1 })\n",
    ] {
        assert!(
            codes_of(src).contains(&"E2056"),
            "{src}: {:?}",
            codes_of(src)
        );
    }
    for src in [
        "local function retry<A..., R...>(f: (A...) -> R..., ...: A...): R...\n    return f(...)\nend\nprint(retry(function(x: number): number return x end, 1))\n",
        "local function first<T...>(...: T...): ...any\n    return ...\nend\nprint(first(1, 2))\n",
        "type Fn<A...> = (A...) -> ()\nlocal function wrap<A...>(f: Fn<A>): Fn<A>\n    return f\nend\nprint(wrap(function(x: number) end))\n",
    ] {
        assert!(
            !codes_of(src).contains(&"E2056"),
            "{src}: {:?}",
            codes_of(src)
        );
    }
}

/// E2058: one name declared as an event and as a message; E2059: a
/// default value in their parameters (spec §2.9).
#[test]
fn event_and_message_declarations() {
    assert!(
        codes_of("declare event MY_THING(a: string)\ndeclare message MY_THING(a: string)\n")
            .contains(&"E2058")
    );
    assert!(!codes_of("declare message MY_OTHER(a: string)\n").contains(&"E2058"));
    for src in [
        "declare message MY_CHANGED(key: string = \"a\")\n",
        "declare event MY_EVENT(unit: string, n: number = 1)\n",
    ] {
        assert_eq!(codes_of(src), ["E2059"], "{src}");
    }
}

/// A table literal returned from a function has escaped: it is sealed
/// (spec §2.4.2), so in strict mode a new field is E2051 and an unknown one
/// E2003, while its own fields read and write as usual.
#[test]
fn returned_tables_are_sealed() {
    let make = "local function make()\n    local t = { count = 0 }\n    t.name = \"x\"\n    return t\nend\nlocal o = make()\n";
    strict_ok(&format!("{make}o.count = o.count + 1\nprint(o.name)\n"));
    strict_err(&format!("{make}o.extra = 1\n"), "E2051");
    strict_err(&format!("{make}print(o.missing)\n"), "E2003");
    // basic mode allows new fields on sealed tables
    ok(&format!("{make}o.extra = 1\nprint(o.extra)\n"));
    // a table made and returned directly, too
    strict_err(
        "local function mk()\n    return { a = 1 }\nend\nlocal m = mk()\nm.b = 2\n",
        "E2051",
    );
}

/// An existence test of a field the type lacks is never reported, and the
/// code it guards reads the field as `any` (spec §6.5), for a local's
/// record too.
#[test]
fn existence_tests_of_record_fields() {
    let r = "local r: { name: string } = { name = \"a\" }\n";
    strict_ok(&format!("{r}if r.maybe then print(r.maybe) end\n"));
    strict_ok(&format!("{r}if not r.maybe then print(r.name) end\n"));
    strict_ok(&format!(
        "{r}if r.maybe ~= nil and r.maybe.deep then print(1) end\n"
    ));
    // outside a test it is still E2003
    strict_err(&format!("{r}print(r.maybe)\n"), "E2003");
    strict_err(
        &format!("{r}if r.maybe then print(1) end\nprint(r.maybe)\n"),
        "E2003",
    );
    // an open table's field set later in a loop is tested as before
    strict_ok(
        "local t = {}\nfor i = 1, 2 do\n    if t.seen then print(t.seen, i) end\n    t.seen = true\nend\n",
    );
}

/// A misspelled method is "has no method", a misspelled field "has no
/// field".
#[test]
fn missing_methods_are_named_methods() {
    let r = "local r: { name: string } = { name = \"a\" }\n";
    let d = errors(&format!("{r}r:greet()\n"), Mode::Strict);
    assert_eq!(d, ["E2003: `{ name: string }` has no method `greet`"]);
    let d = errors(&format!("{r}print(r.greet)\n"), Mode::Strict);
    assert_eq!(d, ["E2003: `{ name: string }` has no field `greet`"]);
}

/// `self` in `function T:m()` is `T` with every field the file stores in
/// it, and messages name it `T` (not the table as it was at that line).
#[test]
fn self_of_a_table_method_is_named() {
    let src = "local Account = {}\nAccount.__index = Account\nfunction Account:deposit(amount: number)\n    self.balance += amount\n    local s: string = self\n    self:withdraw(1)\nend\nfunction Account:withdraw(amount: number)\n    self.balance -= amount\nend\nprint(Account)\n";
    let d = errors(src, Mode::Basic);
    assert_eq!(
        d,
        ["E2001: `Account` is not assignable to `string`"],
        "{d:#?}"
    );
    // the method as the table holds it
    let d = errors(
        &format!("{src}local n: number = Account.deposit\n"),
        Mode::Basic,
    );
    assert!(
        d.iter()
            .any(|x| x.contains("(self: Account, amount: number) -> ()")),
        "{d:#?}"
    );
}
