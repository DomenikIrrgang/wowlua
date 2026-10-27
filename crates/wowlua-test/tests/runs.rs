//! A test run as a library takes it (`runs`, `run_with`): wowlua studio's
//! progress, its stop button, and the functions each test called.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use wowlua_driver::project::{LoadOptions, load_projects};
use wowlua_test::Outcome;
use wowlua_test::runs::{self, Switches};

const TRACKER: &str = r#"
export function double(x: number): number
    return x * 2
end
export function half(x: number): number
    return x / 2
end
export function unused(): number
    return 0
end
"#;

fn test_file(name: &str) -> String {
    format!(
        r#"
import test, expect from "wowlua:test"
import double, half from "/tracker"

@test("{name} doubles")
local function doubles()
    expect(double(2)).toBe(4)
end

@test("{name} halves")
local function halves()
    expect(half(4)).toBe(2)
end
"#
    )
}

/// A package `demo` for `standard`, with the tracker and three test files.
fn package() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        (
            "wowlua.json".to_string(),
            r#"{ "package": { "name": "demo", "version": "1.0.0" }, "flavors": { "standard": {} }, "addon": { "title": "Demo" } }"#.to_string(),
        ),
        ("src/tracker.wowlua".to_string(), TRACKER.to_string()),
        ("tests/a.test.wowlua".to_string(), test_file("a")),
        ("tests/b.test.wowlua".to_string(), test_file("b")),
        ("tests/c.test.wowlua".to_string(), test_file("c")),
    ];
    for (path, text) in files {
        let path = dir.path().join(path);
        wowlua_host::create_dir_all(path.parent().unwrap()).unwrap();
        wowlua_host::write(path, text).unwrap();
    }
    dir
}

/// Runs `f` with the package's test build and its suite, coverage on.
fn with_suite(root: &Path, f: impl FnOnce(&wowlua_test::Suite)) {
    let mut session = wowlua_driver::Session::new();
    let options = LoadOptions {
        cwd: root.to_path_buf(),
        lock_in_memory: true,
        offline: true,
        ..LoadOptions::default()
    };
    let projects = load_projects(&mut session, &options).expect("the package loads");
    let project = &projects[0];
    let (build, left_out) = runs::build(
        &mut session,
        project,
        vec!["standard".into()],
        "standard",
        "test",
        wowlua_driver::parallel::Jobs::default(),
        &mut |_| {},
    )
    .expect("the test build");
    assert!(left_out.is_empty());
    let switches = Switches {
        coverage: true,
        ..Switches::default()
    };
    let suite = runs::suite(project, &build, "standard", None, switches, None, None);
    f(&suite);
}

#[test]
fn a_run_tells_each_file_as_it_finishes_and_stops_when_asked() {
    let dir = package();
    with_suite(dir.path(), |suite| {
        let told = Mutex::new(Vec::new());
        let never = AtomicBool::new(false);
        let results = wowlua_test::run_with(suite, &[], 1, &never, &|r| {
            told.lock().unwrap().push(r.file.clone())
        });
        assert_eq!(results.len(), 3);
        assert_eq!(
            *told.lock().unwrap(),
            [
                "tests/a.test.wowlua",
                "tests/b.test.wowlua",
                "tests/c.test.wowlua"
            ]
        );

        // Stopped after the first file: no more run.
        let stop = AtomicBool::new(false);
        let results = wowlua_test::run_with(suite, &[], 1, &stop, &|_| {
            stop.store(true, Ordering::Relaxed)
        });
        assert_eq!(results.len(), 1);
    });
}

#[test]
fn with_coverage_each_test_names_the_functions_it_called() {
    let dir = package();
    with_suite(dir.path(), |suite| {
        let results = wowlua_test::run(suite, &["tests/a.test.wowlua".into()], 1);
        let tests = &results[0].tests;
        assert!(
            tests.iter().all(|t| t.outcome == Outcome::Pass),
            "{tests:?}"
        );
        let called = |name: &str| {
            tests
                .iter()
                .find(|t| t.name == name)
                .map(|t| t.functions.clone())
                .unwrap()
        };
        assert_eq!(called("a doubles"), [("src/tracker.wowlua".to_string(), 2)]);
        assert_eq!(called("a halves"), [("src/tracker.wowlua".to_string(), 5)]);
    });
}
