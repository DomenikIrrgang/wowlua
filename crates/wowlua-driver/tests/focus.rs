//! The language server's first pass (decisions T372, T375): a focused check
//! of a file takes the modules it imports from the cache while what they
//! read is unchanged, and checks them again when an import's interface
//! changed, so its errors are the ones a full check finds.

use std::path::Path;
use wowlua_driver::{BuildCache, BuildRequest, Session, build_with_cache};

fn request() -> BuildRequest {
    BuildRequest {
        profile: "dev".into(),
        flavors: None,
        out_dir: None,
        write: false,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: wowlua_driver::parallel::Jobs::default(),
        keep_code: false,
        tests: false,
        left_out: Vec::new(),
    }
}

/// A check of the package (focused on `focus`, or whole): the codes of the
/// errors and warnings found in `focus`, and the modules taken.
fn check(root: &Path, cache: &mut BuildCache, focus: Option<&Path>) -> (Vec<String>, usize) {
    let mut session = Session::new();
    session.focus = focus.map(|f| [std::fs::canonicalize(f).unwrap()].into_iter().collect());
    let options = wowlua_driver::LoadOptions {
        cwd: root.to_path_buf(),
        offline: true,
        ..Default::default()
    };
    let project = wowlua_driver::load_project(&mut session, &options).expect("loads");
    let _ = build_with_cache(&mut session, &project, &request(), cache);
    let mut codes: Vec<String> = session
        .take_diagnostics()
        .into_vec()
        .iter()
        .filter(|d| {
            d.labels
                .first()
                .is_some_and(|l| session.db.get(l.span.file).path().ends_with("c.wowlua"))
        })
        .filter_map(|d| d.code.map(str::to_string))
        .collect();
    codes.sort();
    (codes, cache.stats.modules)
}

#[test]
fn an_import_is_taken_until_its_interface_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("wowlua.json"),
        r#"{ "package": { "name": "focus-demo", "version": "0.1.0", "wowlua": "^0.10" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let a = root.join("src/a.wowlua");
    let b = root.join("src/b.wowlua");
    let c = root.join("src/c.wowlua");
    std::fs::write(&a, "export function value(): number\n    return 1\nend\n").unwrap();
    // `b`'s export has the type `a`'s gives
    std::fs::write(
        &b,
        "import value from \"./a\"\n\nexport local current = value()\n",
    )
    .unwrap();
    std::fs::write(
        &c,
        "import current from \"./b\"\n\nlocal n: number = current\nprint(n)\n",
    )
    .unwrap();
    let mut cache = BuildCache::default();
    // a full check keeps every module; a focused one takes `a` and `b`
    assert_eq!(check(root, &mut cache, None).0, Vec::<String>::new());
    let (codes, taken) = check(root, &mut cache, Some(&c));
    assert_eq!(codes, Vec::<String>::new());
    assert_eq!(taken, 2, "a and b taken");
    // `a`'s body changes, not its interface: `b` is still taken
    std::fs::write(&a, "export function value(): number\n    return 2\nend\n").unwrap();
    let (codes, taken) = check(root, &mut cache, Some(&c));
    assert_eq!(codes, Vec::<String>::new());
    assert_eq!(taken, 1, "b taken, a checked");
    // `a`'s interface changes: `b` is checked again, and `c` sees it
    std::fs::write(
        &a,
        "export function value(): string\n    return \"x\"\nend\n",
    )
    .unwrap();
    let (codes, taken) = check(root, &mut cache, Some(&c));
    assert_eq!(taken, 0, "a and b checked");
    assert!(codes.contains(&"E2001".to_string()), "{codes:?}");
    // as a full check finds
    let mut fresh = BuildCache::default();
    let (full, _) = check(root, &mut fresh, None);
    assert!(full.contains(&"E2001".to_string()), "{full:?}");
}
