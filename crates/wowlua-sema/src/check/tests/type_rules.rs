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
