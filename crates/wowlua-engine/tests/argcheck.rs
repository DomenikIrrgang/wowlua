//! The client's argument checks (`/stdtest argcheck`, WoW Forever 1.60.1):
//! the engine runs the same probe and must give the same results.

use std::collections::BTreeMap;
use std::path::PathBuf;

use wowlua_vm::{Value, Vm};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `{ [name] = { [variant] = result } }` as nested maps.
fn table(vm: &Vm, v: Value) -> BTreeMap<String, BTreeMap<String, String>> {
    let text = |v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        other => format!("{other:?}"),
    };
    let Value::Table(t) = v else {
        panic!("a table")
    };
    vm.heap
        .table(t)
        .entries()
        .map(|(k, row)| {
            let Value::Table(row) = row else {
                panic!("a row")
            };
            let row = vm
                .heap
                .table(row)
                .entries()
                .map(|(k, v)| (text(k), text(v)))
                .collect();
            (text(k), row)
        })
        .collect()
}

/// Cases that wait for the engine's game state: functions a predicate
/// guards return nothing when it fails, before checking.
const PENDING: &[&str] = &[
    "C_Club.GetLastTicketResponse",
    "C_Commentator.FindSpectatedUnit",
    "C_SpellDiminish.GetAllSpellDiminishCategories",
];

/// Secret-argument messages the client cuts at 199 bytes, where others as
/// long are whole: which ones it cuts is not known.
const CUT: &[&str] = &[
    "C_AchievementInfo.GetSupercedingAchievements",
    "C_Club.GetInvitationCandidates",
    "C_LegendaryCrafting.GetRuneforgePowers",
    "C_TooltipComparison.GetItemComparisonDelta",
];

#[test]
fn forever_camelot() {
    let fixture = std::fs::read(
        repo().join("crates/wowlua-engine/tests/client/forever-camelot-argcheck.lua"),
    )
    .expect("the fixture");
    let probe = std::fs::read(repo().join("tests/ingame/src/argcheck.lua")).expect("the probe");
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let f = vm.load(&fixture, "=fixture").expect("compiles");
    let theirs = vm.call(f, &[]).expect("runs")[0];
    let theirs = table(&vm, theirs);
    // As the client runs it: addon code, its execution tainted.
    let chunk = vm
        .load_as(
            &probe,
            "@Interface/AddOns/StdTest/src/argcheck.lua",
            "StdTest",
        )
        .expect("compiles");
    let driver = vm
        .load(b"local chunk = ... local m local ns = { __wowlua = { define = function(_, x) m = x end } } chunk('StdTest', ns) return m.run()", "=driver")
        .expect("compiles");
    let ours = match vm.call(driver, &[chunk]) {
        Ok(r) => r[0],
        Err(e) => panic!(
            "the probe fails: {:?}",
            match e.value {
                Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                _ => String::new(),
            }
        ),
    };
    let ours = table(&vm, ours);
    let mut differences = Vec::new();
    let mut total = 0;
    for (name, row) in &theirs {
        if PENDING.contains(&name.as_str()) {
            continue;
        }
        for (variant, result) in row {
            total += 1;
            let mine = ours.get(name).and_then(|r| r.get(variant));
            // Where the client returned results, the check passed: ours
            // must too (its behaviour may not be there yet).
            let passed =
                !result.starts_with("error: ") && mine.is_some_and(|m| !m.contains("bad argument"));
            let cut = CUT.contains(&name.as_str()) && variant.starts_with("secret");
            if mine != Some(result) && !passed && !cut {
                differences.push(format!(
                    "{name}({variant})\n    client: {result}\n    ours:   {}",
                    mine.map_or("<none>", String::as_str)
                ));
            }
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {total} calls differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}
