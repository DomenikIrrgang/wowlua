//! What `wowlua check` reports, end to end: each problem once, worded as a
//! sentence.

use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn wowlua(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", dir.join(".wowlua-home"))
        .env_remove("WOWLUA_WOW_PATH");
    cmd
}

/// The diagnostic lines `wowlua check` prints for a package of `files`.
fn check(files: &[(&str, &str)]) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let manifest = (
        "wowlua.json",
        "{ \"package\": { \"name\": \"x-y\", \"version\": \"1.0.0\" } }\n",
    );
    for (path, content) in std::iter::once(&manifest).chain(files) {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("error[") || l.contains("warning["))
        .map(str::to_string)
        .collect()
}

/// Assigning to a `const` is one error, though both the checker and the
/// lowering find it.
#[test]
fn assigning_a_constant_is_reported_once() {
    assert_eq!(
        check(&[("src/main.wowlua", "const X = 40\nX = 41\nprint(X)\n")]),
        ["src/main.wowlua:2:1: error[E2010]: cannot assign to the constant `X`"]
    );
    assert_eq!(
        check(&[
            (
                "src/a.wowlua",
                "export const A = 1\nexport function f() end\n"
            ),
            (
                "src/main.wowlua",
                "import A, f from \"./a\"\nA = 2\nf = nil\nprint(A, f)\n"
            ),
        ]),
        [
            "src/main.wowlua:2:1: error[E2010]: cannot assign to the constant `A`",
            "src/main.wowlua:3:1: error[E1010]: cannot assign to the imported binding `f`"
        ]
    );
}
