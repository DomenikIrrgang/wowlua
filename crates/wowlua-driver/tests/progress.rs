//! A build tells its steps (decision T503): the sources read, each file
//! parsed and linted as it finishes, the program checked, bundled, written.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use std::sync::{Arc, Mutex};
use wowlua_driver::build::{Progress, Step};
use wowlua_driver::{BuildRequest, Session, build};

fn steps(root: &std::path::Path, bundle: bool) -> Vec<Step> {
    let told: Arc<Mutex<Vec<Step>>> = Arc::default();
    let sink = told.clone();
    let mut request = BuildRequest::new("dev");
    request.bundle = Some(bundle);
    request.progress = Some(Progress(Arc::new(move |step| {
        sink.lock().unwrap().push(step)
    })));
    let mut session = Session::new();
    let options = wowlua_driver::LoadOptions {
        cwd: root.to_path_buf(),
        offline: true,
        ..Default::default()
    };
    let project = wowlua_driver::load_project(&mut session, &options).expect("loads");
    assert!(build(&mut session, &project, &request).is_some(), "builds");
    told.lock().unwrap().clone()
}

/// Where the first step `matches` is.
fn first(steps: &[Step], matches: impl Fn(&Step) -> bool) -> usize {
    steps
        .iter()
        .position(matches)
        .unwrap_or_else(|| panic!("missing in {steps:?}"))
}

#[test]
fn a_build_tells_its_steps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("wowlua.json"),
        r#"{ "package": { "name": "progress", "version": "1.0.0" } }"#,
    )
    .unwrap();
    std::fs::write(root.join("src/a.wowlua"), "local a = 1\nprint(a)\n").unwrap();
    std::fs::write(root.join("src/b.wowlua"), "local b = 2\nprint(b)\n").unwrap();

    let told = steps(root, false);
    assert_eq!(told[0], Step::Read { files: 2 }, "{told:?}");
    // Each file parsed and linted once, in any order (worker threads).
    let parsed: Vec<(usize, usize)> = told
        .iter()
        .filter_map(|s| match s {
            Step::Parsed { done, of } => Some((*done, *of)),
            _ => None,
        })
        .collect();
    let of = parsed[0].1;
    assert!(of >= 2, "the sources and the declarations: {parsed:?}");
    let mut done: Vec<usize> = parsed.iter().map(|(d, _)| *d).collect();
    done.sort();
    assert_eq!(done, (1..=of).collect::<Vec<_>>());
    let mut linted: Vec<(usize, usize)> = told
        .iter()
        .filter_map(|s| match s {
            Step::Linted { done, of } => Some((*done, *of)),
            _ => None,
        })
        .collect();
    linted.sort();
    assert_eq!(linted, vec![(1, 2), (2, 2)]);
    let read = first(&told, |s| matches!(s, Step::Read { .. }));
    let parse = first(&told, |s| matches!(s, Step::Parsed { .. }));
    let lint = first(&told, |s| matches!(s, Step::Linted { .. }));
    let check = first(&told, |s| matches!(s, Step::Checking { index: 0, .. }));
    let write = first(&told, |s| *s == Step::Writing);
    assert!(
        read < parse && parse < lint && lint < check && check < write,
        "{told:?}"
    );
    assert_eq!(write, told.len() - 1, "written last");
    assert!(!told.contains(&Step::Bundling));

    let bundled = steps(root, true);
    let bundle = first(&bundled, |s| *s == Step::Bundling);
    assert_eq!(bundled[bundle + 1..], [Step::Writing], "{bundled:?}");
}
