//! Every client case with the engine and the flavor's Blizzard UI code
//! loaded (when `cargo xtask types` fetched it), the addon's saved variables
//! loaded as the addon's: including those the VM alone cannot pass
//! (`wowlua-vm`'s `tests/client.rs` lists them as pending).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use wowlua_vm::{Value, Vm};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// As `wowlua-vm`'s driver: the cases inside two `xpcall`s.
const DRIVER: &str = "local chunk = ...
local module
local ns = { __wowlua = { define = function(_, m) module = m end } }
chunk('StdTest', ns)
local results, kinds
local function keep(e) return e end
xpcall(function()
    xpcall(function() results, kinds = module.run(), module.kinds() end, keep)
end, keep)
return results, kinds";

fn strings(vm: &Vm, t: Value) -> BTreeMap<String, String> {
    let Value::Table(t) = t else {
        return BTreeMap::new();
    };
    vm.heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(vm.str_bytes(v)).into_owned(),
            )),
            _ => None,
        })
        .collect()
}

fn fixture() -> BTreeMap<String, String> {
    let src = std::fs::read(repo().join("crates/wowlua-vm/tests/client/forever-camelot.lua"))
        .expect("the fixture");
    let mut vm = Vm::new();
    let f = vm.load(&src, "=fixture").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let k = vm.str("results");
    let results = vm.heap.table(t).get(k);
    strings(&vm, results)
}

#[test]
fn all_cases_match_the_client() {
    let interface = repo().join("target/types-src/camelot/Interface");
    if !interface.exists() {
        eprintln!("skipped: {} is missing", interface.display());
        return;
    }
    let xtask = std::fs::read_to_string(repo().join("xtask/src/types.rs")).unwrap_or_default();
    let commit = xtask
        .split("\"camelot\",")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(3))
        .unwrap_or("");
    let strings_path = repo().join(format!("target/types-src/strings/{commit}.lua"));
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::ui::load(
        &mut vm,
        &interface,
        strings_path.exists().then_some(Path::new(&strings_path)),
    );
    // StdTest's saved variables, loaded as the addon's.
    let saved = vm
        .load_as(
            b"StdTestState = {}",
            "@SavedVariables/StdTest.lua",
            "StdTest",
        )
        .expect("compiles");
    vm.call(saved, &[]).expect("runs");
    let src = std::fs::read(repo().join("tests/ingame/src/conformance.lua")).expect("the cases");
    let chunk = vm.load(&src, "@conformance.lua").expect("compiles");
    let driver = vm.load(DRIVER.as_bytes(), "=driver").expect("compiles");
    let r = vm.call(driver, &[chunk]).expect("the cases run");
    let ours = strings(&vm, r[0]);
    let theirs = fixture();
    let mut differences = Vec::new();
    let mut matched = 0;
    for (source, expected) in &theirs {
        match ours.get(source) {
            Some(mine) if mine == expected => matched += 1,
            mine => differences.push(format!(
                "{source}\n    client: {expected}\n    ours:   {}",
                mine.map_or("<not run>", String::as_str)
            )),
        }
    }
    eprintln!("{matched} cases match the client");
    assert!(
        differences.is_empty(),
        "{} cases differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}
