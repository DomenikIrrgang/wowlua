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
