//! A project held in memory (spec M14, decision T394): with a
//! `MemoryHost` installed, loading, checking, building and formatting read
//! and write only its files, as the browser's build does. One test: the
//! host is the process's.

use std::path::Path;
use std::sync::Arc;
use wowlua_driver::host::{self, MemoryHost};
use wowlua_driver::{BuildRequest, Session};

fn request(write: bool) -> BuildRequest {
    BuildRequest {
        profile: "dev".into(),
        flavors: None,
        out_dir: None,
        write,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: wowlua_driver::parallel::Jobs {
            threads: std::num::NonZeroUsize::MIN,
            memory: 1 << 30,
            explicit: true,
        },
        keep_code: false,
        tests: false,
        left_out: Vec::new(),
    }
}

fn load(session: &mut Session) -> wowlua_driver::Project {
    let options = wowlua_driver::LoadOptions {
        cwd: "/play".into(),
        offline: true,
        ..Default::default()
    };
    match wowlua_driver::load_project(session, &options) {
        Some(p) => p,
        None => panic!("{:?}", session.take_diagnostics().into_vec()),
    }
}

#[test]
fn a_project_in_memory_checks_builds_and_formats() {
    // (a path no disk has: a read that misses the host fails)
    let memory = Arc::new(MemoryHost::with_clock(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
    ));
    memory.insert(
        "/play/wowlua.json",
        &br#"{ "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.11" }, "flavors": { "camelot": {} } }"#[..],
    );
    memory.insert(
        "/play/src/main.wowlua",
        &b"import double from \"./math\"\n\nlocal n: string = double(2)\nprint(n)\n"[..],
    );
    memory.insert(
        "/play/src/math.wowlua",
        &b"export function double(x: number): number\n    return x  *  2\nend\n"[..],
    );
    host::install(Some(memory.clone()));

    // a check finds the type error
    let mut session = Session::new();
    let project = load(&mut session);
    let _ = wowlua_driver::build(&mut session, &project, &request(false));
    let codes: Vec<String> = session
        .take_diagnostics()
        .into_vec()
        .iter()
        .filter_map(|d| d.code.map(str::to_string))
        .collect();
    assert_eq!(codes, ["E2001"], "{codes:?}");

    // fixed, a build writes the AddOn into memory
    memory.insert(
        "/play/src/main.wowlua",
        &b"import double from \"./math\"\n\nlocal n: number = double(2)\nprint(n)\n"[..],
    );
    let mut session = Session::new();
    let project = load(&mut session);
    let Some(output) = wowlua_driver::build(&mut session, &project, &request(true)) else {
        panic!("{:?}", session.take_diagnostics().into_vec());
    };
    assert!(
        !session.has_errors(),
        "{:?}",
        session.take_diagnostics().into_vec()
    );
    let toc = output.dir.join("Playground.toc");
    assert!(host::is_file(&toc), "{:?}", memory.files());
    #[allow(clippy::disallowed_methods)] // (the disk's, on purpose)
    let on_disk = toc.exists();
    assert!(!on_disk, "nothing on the disk");
    let toc = host::read_to_string(&toc).unwrap();
    assert!(toc.contains("## Title"), "{toc}");

    // formatting rewrites the file in memory
    let mut session = Session::new();
    let project = load(&mut session);
    let report = wowlua_driver::format::format(
        &mut session,
        &project,
        &wowlua_driver::format::FormatRequest {
            paths: Vec::new(),
            check: false,
        },
    );
    assert_eq!(report.changed, ["src/math.wowlua"]);
    let math = host::read_to_string(Path::new("/play/src/math.wowlua")).unwrap();
    assert!(math.contains("return x * 2"), "{math}");
    host::install(None);
}
