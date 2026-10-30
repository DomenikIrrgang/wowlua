//! The 3D world from the WoW installation of this machine (decision T522);
//! skipped where there is none.

use wowlua_vm::{Value, Vm};

/// A camelot session with the client's data, at Northshire Abbey where a
/// new human starts.
fn session() -> Option<Vm> {
    let root = std::env::var_os("WOWLUA_WOW_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/World of Warcraft".into());
    let storage = wowlua_casc::FsStorage { root };
    let product = wowlua_casc::Casc::product_of_folder(&storage, "_forever_")?;
    let casc = std::sync::Arc::new(wowlua_casc::Casc::open(Box::new(storage), &product).ok()?);
    let data = std::sync::Arc::new(wowlua_render::client::CascClientData::new(Box::new(
        move || Some(casc.clone()),
    )));
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").ok()?;
    wowlua_engine::controls::install(&mut vm, "mock");
    wowlua_engine::clientdata::set(&mut vm, data);
    Some(vm)
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "@Test.lua").expect("compiles");
    let text = |vm: &Vm, v: Value| match vm.hidden(v) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{}", (n * 100.0).round() / 100.0),
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// With the world on, the player and the units stand on the ground; off,
/// where the test put them.
#[test]
fn the_player_and_units_stand_on_the_ground() {
    let Some(mut vm) = session() else {
        eprintln!("no camelot client: skipped");
        return;
    };
    let src = "mock.moveTo({ world = { y = -8949.95, x = -132.493, z = 0, instanceID = 0 } })
local wolf = mock.spawnUnit({ name = 'Wolf', reaction = 'hostile',
    world = { y = -8930, x = -132.493, z = 0 } })
mock.player:target(wolf)
local _, _, z = UnitPosition('player')
local _, _, wz = UnitPosition('target')
return z, wz";
    let off = run(&mut vm, src);
    assert_eq!(off, ["0", "0"]);
    wowlua_engine::set_world(&mut vm, true);
    let on = run(&mut vm, src);
    assert_eq!(on[0], "83.53");
    // (20 yards north, up the abbey's slope)
    let wolf: f64 = on[1].parse().expect("a height");
    assert!(wolf > 75.0 && wolf < 95.0 && wolf != 83.53, "{wolf}");
}
