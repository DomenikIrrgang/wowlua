//! The language server's checker keeps the packages' modules on disk (spec
//! compiler §13.9 Phase 2): a new checker's first focused check takes the
//! unchanged imports an earlier checker's full check kept, with the same
//! errors.

use std::collections::HashSet;
use wowlua_lsp::check::{self, Store, Warm};

#[test]
fn a_new_checker_takes_what_an_earlier_one_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = wowlua_lsp::text::canonical(dir.path());
    std::fs::write(
        root.join("wowlua.json"),
        r#"{ "package": { "name": "lsp-persist", "version": "0.1.0", "wowlua": "^0.10" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/a.wowlua"),
        "export function value(): number\n    return 1\nend\n",
    )
    .unwrap();
    let main = root.join("src/main.wowlua");
    let text = "import value from \"./a\"\n\nlocal s: string = value()\nprint(s)\n";
    std::fs::write(&main, text).unwrap();
    let manifest = root.join("wowlua.json");
    let store = Store::Dir(root.join("store"));
    let docs = vec![(main.clone(), text.to_string())];
    let focus: HashSet<_> = [main.clone()].into_iter().collect();
    let errors = |answer: &serde_json::Value| {
        let checked = check::answer(answer).expect("an answer");
        let mut codes: Vec<String> = checked
            .iter()
            .flat_map(|c| c.diagnostics.get(&main).into_iter().flatten())
            .filter_map(|d| d["code"].as_str().map(str::to_string))
            .collect();
        codes.sort();
        codes
    };
    // the first server's full check keeps the modules
    let mut first = Warm::persistent(store.clone());
    let full = errors(&check::worker(
        &mut first,
        &check::request(&manifest, false, &docs, None, false, None),
    ));
    assert_eq!(full, vec!["E2001".to_string()]);
    assert!(std::fs::read_dir(root.join("store")).unwrap().count() == 1);
    // the next server's first pass takes `a`, with the same errors
    let mut second = Warm::persistent(store.clone());
    let focused = errors(&check::worker(
        &mut second,
        &check::request(&manifest, false, &docs, Some(&focus), false, None),
    ));
    assert_eq!(focused, full);
    assert_eq!(
        second.stats(&manifest).map(|s| s.modules),
        Some(1),
        "a taken"
    );
    // in memory only, nothing is taken
    let mut third = Warm::default();
    check::worker(
        &mut third,
        &check::request(&manifest, false, &docs, Some(&focus), false, None),
    );
    assert_eq!(third.stats(&manifest).map(|s| s.modules), Some(0));
}
