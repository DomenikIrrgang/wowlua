//! The client's behaviour as a fixture (spec emulator §17.8): StdTest's
//! `/stdtest probe` runs the conformance cases of
//! `tests/ingame/src/conformance.lua` in the game and saves what each
//! returned or raised; `tests/client/<client>.lua` holds those results, and
//! the VM, in that client's environment, must give the same.

use std::collections::BTreeMap;
use std::path::PathBuf;

use wowlua_vm::{Value, Vm};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Loads the cases file as the client does, with a stand-in for wowlua's
/// module runtime, and runs it: each case's result and each function's kind.
const DRIVER: &str = "local chunk = ...
local module
local ns = { __wowlua = { define = function(_, m) module = m end } }
chunk('StdTest', ns)
return module.run(), module.kinds()";

fn strings(vm: &Vm, t: Value) -> BTreeMap<String, String> {
    let Value::Table(t) = t else {
        panic!("a table expected")
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

/// Our results and kinds.
fn ours() -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let src = std::fs::read(repo().join("tests/ingame/src/conformance.lua")).expect("the cases");
    let mut vm = Vm::wow();
    let chunk = vm
        .load(&src, "@conformance.lua")
        .expect("the cases compile");
    let driver = vm.load(DRIVER.as_bytes(), "=driver").expect("compiles");
    let r = vm.call(driver, &[chunk]).unwrap_or_else(|e| {
        let msg = match e.value {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => other.type_name().to_string(),
        };
        panic!("the cases fail to run: {msg}")
    });
    (strings(&vm, r[0]), strings(&vm, r[1]))
}

/// A fixture: `return { client = …, interface = …, results = {…}, kinds = {…} }`.
fn fixture(name: &str) -> Option<(BTreeMap<String, String>, BTreeMap<String, String>)> {
    let path = repo().join(format!("crates/wowlua-vm/tests/client/{name}.lua"));
    let src = std::fs::read(path).ok()?;
    let mut vm = Vm::new();
    let f = vm.load(&src, "=fixture").expect("the fixture compiles");
    let r = vm.call(f, &[]).expect("the fixture runs");
    let Value::Table(t) = r[0] else {
        panic!("the fixture returns a table")
    };
    let get = |vm: &mut Vm, key: &str| {
        let k = vm.str(key);
        vm.heap.table(t).get(k)
    };
    let results = get(&mut vm, "results");
    let kinds = get(&mut vm, "kinds");
    Some((strings(&vm, results), strings(&vm, kinds)))
}

/// Cases whose behaviour the VM does not have yet, with why: they are
/// listed, not failed.
const PENDING: &[(&str, &str)] = &[
    (
        "local m = getmetatable('')",
        "string.rtgsub is Blizzard's Lua (S3)",
    ),
    (
        "strsplit(',', string.rep('a,', 79",
        "strsplit's limits: the crash-limit probes",
    ),
    ("secretwrap", "secret values (S2.3)"),
    ("canaccess", "secret values (S2.3)"),
    ("scrubsecretvalues", "secret values (S2.3)"),
    ("hasanysecretvalues", "secret values (S2.3)"),
    ("issecrettable", "secret values (S2.3)"),
    ("issecretvalue", "secret values (S2.3)"),
    (
        "assertf",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "ipairs_reverse",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "getglobal",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "setglobal",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "nop(",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "tostringall",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "getprinthandler",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "rtgsub",
        "Blizzard's Lua (SharedXML), loaded with the engine (S3)",
    ),
    (
        "coroutine.create(print)",
        "the client's print is Blizzard's Lua (S3)",
    ),
    (
        "includes, mapvalues",
        "Blizzard's Lua and engine functions (S3)",
    ),
    (
        "issecurevariable('StdTestConformanceGlobal')",
        "written by setglobal, Blizzard's Lua (S3)",
    ),
    (
        "issecurevariable('StdTestState')",
        "saved variables load as the addon's (S3)",
    ),
];

#[test]
fn forever_camelot() {
    let Some((results, _kinds)) = fixture("forever-camelot") else {
        return;
    };
    let (ours, _) = ours();
    let mut differences = Vec::new();
    let mut pending = 0;
    for (source, theirs) in &results {
        if PENDING.iter().any(|(p, _)| source.contains(p)) {
            pending += 1;
            continue;
        }
        match ours.get(source) {
            Some(mine) if mine == theirs => {}
            mine => differences.push(format!(
                "{source}\n    client: {theirs}\n    ours:   {}",
                mine.map_or("<not run>", String::as_str)
            )),
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} cases differ from the client ({pending} pending):\n{}",
        differences.len(),
        results.len(),
        differences.join("\n")
    );
    println!(
        "{} cases match the client; {pending} pending",
        results.len() - pending
    );
}

/// Prints our results (`cargo test -p wowlua-vm --test client -- --ignored
/// --nocapture`).
#[test]
#[ignore]
fn print_ours() {
    let (results, kinds) = ours();
    for (source, result) in &results {
        println!("{source}\n    {result}");
    }
    println!("{} cases, {} functions", results.len(), kinds.len());
}
