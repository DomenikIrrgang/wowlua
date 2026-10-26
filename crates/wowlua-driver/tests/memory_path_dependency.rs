//! A path dependency of a project held in memory (spec M14, decision T394):
//! with a `MemoryHost` installed, the package manager finds the dependency
//! and reads its manifest through the host, as the browser's build does.
//! One test: the host is the process's.

use std::sync::Arc;
use wowlua_driver::host::{self, MemoryHost};
use wowlua_driver::{BuildRequest, Session};

#[test]
fn a_path_dependency_in_memory_builds_into_the_addon() {
    // (paths no disk has: a read that misses the host fails)
    let memory = Arc::new(MemoryHost::with_clock(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
    ));
    memory.insert(
        "/play/wowlua.json",
        &br#"{ "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" }, "flavors": { "camelot": {} },
  "dependencies": { "greeter": { "path": "../greeter" } } }"#[..],
    );
    memory.insert(
        "/play/src/main.wowlua",
        &b"import greet from \"greeter\"\nprint(greet(\"you\"))\n"[..],
    );
    memory.insert(
        "/greeter/wowlua.json",
        &br#"{ "package": { "name": "greeter", "version": "1.2.0", "kind": "library", "license": "MIT" } }"#[..],
    );
    memory.insert(
        "/greeter/src/init.wowlua",
        &b"export function greet(name: string): string\n    return \"hello \" .. name\nend\n"[..],
    );
    host::install(Some(memory.clone()));

    let mut session = Session::new();
    let options = wowlua_driver::LoadOptions {
        cwd: "/play".into(),
        offline: true,
        ..Default::default()
    };
    let Some(project) = wowlua_driver::load_project(&mut session, &options) else {
        panic!("{:?}", session.take_diagnostics().into_vec());
    };
    let greeter = project
        .deps
        .packages
        .iter()
        .find(|p| p.name == "greeter")
        .expect("the path dependency resolves");
    assert_eq!(greeter.version.to_string(), "1.2.0");
    let request = BuildRequest {
        profile: "dev".into(),
        flavors: None,
        out_dir: None,
        write: true,
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
        reproducible: false,
    };
    let output = wowlua_driver::build(&mut session, &project, &request);
    assert!(
        !session.has_errors(),
        "{:?}",
        session.take_diagnostics().into_vec()
    );
    let output = output.expect("built");
    let toc = host::read_to_string(output.dir.join("Playground.toc")).unwrap();
    assert!(toc.contains("libs\\greeter\\init.lua"), "{toc}");
    host::install(None);
}
