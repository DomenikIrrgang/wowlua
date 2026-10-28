//! The language server's first pass (decisions T372, T375): a focused check
//! of a file takes the modules it imports from the cache while what they
//! read is unchanged, and checks them again when an import's interface
//! changed, so its errors are the ones a full check finds.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

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
        reproducible: false,
        progress: None,
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
        r#"{ "package": { "name": "focus-demo", "version": "0.1.0", "wowlua": "^0.13" }, "flavors": { "camelot": {} } }"#,
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

/// A taken module's classes keep the results their methods' bodies gave
/// (decision L56): its importers check as against the module checked.
#[test]
fn a_taken_modules_methods_keep_their_inferred_results() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("wowlua.json"),
        r#"{ "package": { "name": "focus-results", "version": "0.1.0", "wowlua": "^0.13" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let a = root.join("src/a.wowlua");
    let c = root.join("src/c.wowlua");
    std::fs::write(
        &a,
        "export class Counter\n    count: number = 0\n    function get()\n        return self.count\n    end\nend\n",
    )
    .unwrap();
    std::fs::write(
        &c,
        "import Counter from \"./a\"\n\nlocal s: string = Counter.new():get()\nprint(s)\n",
    )
    .unwrap();
    let mut cache = BuildCache::default();
    let (full, _) = check(root, &mut cache, None);
    assert!(full.contains(&"E2001".to_string()), "{full:?}");
    // `a` taken from the cache: `get` still gives a number
    let (codes, taken) = check(root, &mut cache, Some(&c));
    assert_eq!(taken, 1, "a taken");
    assert_eq!(codes, full);
    // `get`'s body changes its results: `a`'s interface changed, `c` sees it
    std::fs::write(
        &a,
        "export class Counter\n    count: number = 0\n    function get()\n        return tostring(self.count)\n    end\nend\n",
    )
    .unwrap();
    let (codes, taken) = check(root, &mut cache, Some(&c));
    assert_eq!(taken, 0, "a checked");
    assert!(!codes.contains(&"E2001".to_string()), "{codes:?}");
    let (codes, taken) = check(root, &mut cache, Some(&c));
    assert_eq!(taken, 1, "a taken again");
    assert!(!codes.contains(&"E2001".to_string()), "{codes:?}");
}

/// The modules kept on disk (spec compiler §13.9 Phase 2): a new checker
/// that reads them takes the unchanged ones as the one that wrote them did,
/// checks a changed one again, and reads nothing of a file that is damaged
/// or was written for other build-wide inputs.
#[test]
fn a_new_checker_takes_the_modules_kept_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let manifest = |strict: bool| {
        format!(
            r#"{{ "package": {{ "name": "focus-disk", "version": "0.1.0", "wowlua": "^0.13" }}, "flavors": {{ "camelot": {{}} }}{} }}"#,
            if strict {
                r#", "check": { "mode": "strict" }"#
            } else {
                ""
            }
        )
    };
    std::fs::write(root.join("wowlua.json"), manifest(false)).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let a = root.join("src/a.wowlua");
    let b = root.join("src/b.wowlua");
    let c = root.join("src/c.wowlua");
    std::fs::write(
        &a,
        "export class Counter\n    count: number = 0\n    function get()\n        return self.count\n    end\nend\n\nexport function make(): Counter\n    return Counter.new()\nend\n",
    )
    .unwrap();
    std::fs::write(
        &b,
        "import make from \"./a\"\n\nexport local counter = make()\nexport local names: { [string]: number } = { one = 1 }\n",
    )
    .unwrap();
    std::fs::write(
        &c,
        "import counter, names from \"./b\"\n\nlocal s: string = counter:get()\nlocal n: number = names.one ?? 0\nprint(s, n)\n",
    )
    .unwrap();
    let store = root.join("cache/modules");
    let mut first = BuildCache::default();
    let (full, _) = check(root, &mut first, None);
    assert_eq!(full, vec!["E2001".to_string()], "get gives a number");
    assert!(first.save_modules(&store).unwrap(), "written");
    assert!(
        !first.save_modules(&store).unwrap(),
        "the same: not written again"
    );
    // a new checker: `a` and `b` taken, the same errors
    let mut second = BuildCache::default();
    assert!(second.load_modules(&store));
    assert!(!second.save_modules(&store).unwrap(), "read back the same");
    let (codes, taken) = check(root, &mut second, Some(&c));
    assert_eq!(taken, 2, "a and b taken");
    assert_eq!(codes, full);
    // `a` changed while no checker ran: checked again, as is `b`
    std::fs::write(
        &a,
        "export class Counter\n    count: number = 0\n    function get()\n        return tostring(self.count)\n    end\nend\n\nexport function make(): Counter\n    return Counter.new()\nend\n",
    )
    .unwrap();
    let mut third = BuildCache::default();
    assert!(third.load_modules(&store));
    let (codes, taken) = check(root, &mut third, Some(&c));
    assert_eq!(taken, 0, "a and b checked");
    assert_eq!(codes, Vec::<String>::new());
    // other build-wide inputs (the manifest): nothing taken
    check(root, &mut third, None);
    third.save_modules(&store).unwrap();
    std::fs::write(root.join("wowlua.json"), manifest(true)).unwrap();
    let mut fourth = BuildCache::default();
    assert!(fourth.load_modules(&store));
    let (_, taken) = check(root, &mut fourth, Some(&c));
    assert_eq!(taken, 0, "another manifest");
    // a damaged file is not read
    std::fs::write(root.join("wowlua.json"), manifest(false)).unwrap();
    let bytes = std::fs::read(&store).unwrap();
    for cut in [0, 7, 40, 72, bytes.len() / 2, bytes.len() - 1] {
        std::fs::write(&store, &bytes[..cut]).unwrap();
        assert!(!BuildCache::default().load_modules(&store), "cut at {cut}");
    }
    let mut flipped = bytes.clone();
    flipped[9] ^= 1;
    std::fs::write(&store, &flipped).unwrap();
    assert!(
        !BuildCache::default().load_modules(&store),
        "another wowlua's"
    );
}
