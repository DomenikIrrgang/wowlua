//! The integration tests of `wowlua-cli`, one test binary: each
//! `tests/*.rs` is a module of it, so the crate and its dependencies are
//! linked once instead of once per file. A new file needs its `mod` line
//! here (`every_test_file_is_a_module` checks).

mod cli;
mod debugger;
mod devtools;
mod diagnostics;
mod engine_probe;
mod fullprobe;
mod lsp;
mod mcp;
mod packages;
mod perf_ui;
mod publish;
mod registry;
mod screenshots;
mod stdlib;
mod test_runner;

/// Each `tests/*.rs` file is a module here or a test target of its own:
/// `autotests = false` builds no other.
#[test]
fn every_test_file_is_a_module() {
    let main = include_str!("suite.rs");
    let own: &[&str] = &["suite"];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    #[allow(clippy::disallowed_methods)] // (the test reads its own directory)
    let entries = std::fs::read_dir(&dir).expect("tests/");
    for entry in entries {
        let path = entry.expect("an entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let name = path
            .file_stem()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        assert!(
            own.contains(&name.as_str()) || main.contains(&format!("\nmod {name};\n")),
            "tests/{name}.rs is not a module of tests/suite.rs: add `mod {name};`"
        );
    }
}
