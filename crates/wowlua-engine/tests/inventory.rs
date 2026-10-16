//! Every function of WoW Forever's in-game environment (`/stdtest
//! inventory`, fixture `wowlua-vm/tests/client/forever-camelot-functions.tsv`):
//! with camelot's engine and Blizzard UI code loaded, each exists, as a C
//! function or a Lua one as in the client.

use std::path::{Path, PathBuf};

use wowlua_vm::function::Function;
use wowlua_vm::{Value, Vm};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The client: camelot's engine with its UI code, or none when that is
/// not on disk (`cargo xtask types`).
fn client() -> Option<Vm> {
    let interface = repo().join("target/types-src/camelot/Interface");
    if !interface.exists() {
        return None;
    }
    let xtask = std::fs::read_to_string(repo().join("xtask/src/types.rs")).unwrap_or_default();
    let commit = xtask
        .split("\"camelot\",")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(3))
        .unwrap_or("");
    let strings = repo().join(format!("target/types-src/strings/{commit}.lua"));
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::ui::load(
        &mut vm,
        &interface,
        strings.exists().then_some(Path::new(&strings)),
    );
    // Errors after loading, for the report.
    let f = vm
        .load(
            b"InventoryErrors = {} seterrorhandler(function(e) InventoryErrors[#InventoryErrors + 1] = tostring(e) end)",
            "=errors",
        )
        .expect("compiles");
    vm.call(f, &[]).expect("runs");
    // Logged in and in the world, as when the inventory was taken
    // (Blizzard's code loads some addons then).
    wowlua_engine::widgets::fire_event(&mut vm, "PLAYER_LOGIN", &[]);
    wowlua_engine::widgets::fire_event(
        &mut vm,
        "PLAYER_ENTERING_WORLD",
        &[Value::Bool(true), Value::Bool(false)],
    );
    wowlua_engine::clock::advance_frames(&mut vm, 30, 1000.0 / 60.0);
    Some(vm)
}

/// What a name holds: "C", "Lua", another type's name, or "absent".
fn kind(vm: &mut Vm, name: &str) -> String {
    let mut v = Value::Table(vm.globals);
    for part in name.split('.') {
        let Value::Table(t) = v else {
            return "absent".into();
        };
        let k = vm.str(part);
        v = vm.heap.table(t).get(k);
    }
    match v {
        Value::Nil => "absent".into(),
        Value::Func(f) => match vm.heap.func(f) {
            Function::Native(_) => "C".into(),
            Function::Lua(_) => "Lua".into(),
        },
        other => other.type_name().into(),
    }
}

/// How many of the client's functions the engine still lacks: Blizzard Lua
/// from files that fail partway (missing enums, constants, game data,
/// animations). It may only go down.
const MAX_MISSING: usize = 95;

#[test]
fn every_function_of_the_client_exists_with_its_kind() {
    let Some(mut vm) = client() else {
        eprintln!("skipped: camelot's UI code is not on disk");
        return;
    };
    let fixture = std::fs::read_to_string(
        repo().join("crates/wowlua-vm/tests/client/forever-camelot-functions.tsv"),
    )
    .expect("the inventory");
    let mut missing = Vec::new();
    let mut wrong = Vec::new();
    let mut total = 0;
    for line in fixture.lines().filter(|l| !l.starts_with('#')) {
        let Some((name, expected)) = line.split_once('\t') else {
            continue;
        };
        total += 1;
        let ours = kind(&mut vm, name);
        if ours == "absent" {
            missing.push(format!("{name} ({expected})"));
        } else if ours != expected {
            wrong.push(format!("{name}: client {expected}, ours {ours}"));
        }
    }
    if let Ok(path) = std::env::var("WOWLUA_INVENTORY_REPORT") {
        let f = vm
            .load(b"return table.concat(InventoryErrors, '\\n')", "=report")
            .expect("compiles");
        let errors = match vm.call(f, &[]).ok().and_then(|r| r.first().copied()) {
            Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        };
        let defaulted: Vec<&str> = wowlua_engine::engine(&vm)
            .defaulted
            .iter()
            .copied()
            .collect();
        std::fs::write(
            path,
            format!(
                "missing:\n{}\n\nwrong kind:\n{}\n\nerrors after loading:\n{errors}\n\ndefaulted:\n{}\n",
                missing.join("\n"),
                wrong.join("\n"),
                defaulted.join("\n")
            ),
        )
        .expect("writes");
    }
    eprintln!("{} of {total} missing", missing.len());
    assert!(
        missing.len() <= MAX_MISSING && wrong.is_empty(),
        "of {total}: {} missing (at most {MAX_MISSING}), {} of another kind\nmissing: {}\nwrong kind: {}",
        missing.len(),
        wrong.len(),
        missing
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        wrong
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
}
