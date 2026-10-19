//! The second round's leftovers (`leftovers2.rs`): the action bars' pages,
//! channels joined by hand, the client's settings, combat sums and the
//! "nothing here" answers.

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    let text = |vm: &Vm, v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

#[test]
fn action_bar_pages() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "return C_ActionBar.GetVehicleBarIndex(), C_ActionBar.GetTempShapeshiftBarIndex(),
    C_ActionBar.GetOverrideBarIndex(), C_ActionBar.GetBonusBarIndexForSlot(73),
    C_ActionBar.GetBonusBarIndexForSlot(1)",
    );
    assert_eq!(r, ["31", "32", "33", "7", "nil"]);
}

#[test]
fn channels_joined_by_hand() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local zone, name = JoinPermanentChannel('MyChannel')
local again = select(2, JoinTemporaryChannel('mychannel'))
return zone, name, again",
    );
    assert_eq!(r, ["0", "MyChannel", "MyChannel"]);
}

#[test]
fn client_settings() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "ConsoleSetFontHeight(20)
local home, world = GetNetIpTypes()
return ConsoleGetFontHeight(), home, world, GetDownloadedPercentage(), C_Club.GetClubCapacity(),
    C_BattleNet.AreFriendTagsEnabled(), GetNumRaidProfiles()",
    );
    // Friend tags on (the full probe, 1.60.1).
    assert_eq!(r, ["20", "1", "1", "0", "1000", "true", "0"]);
}

#[test]
fn combat_sums() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "return GetGlancingBlowPenalty(300, 315), GetGlancingBlowPenalty(308, 315),
    ResistancePercent(150, 60)",
    );
    let n: Vec<f64> = r.iter().map(|s| s.parse().expect("a number")).collect();
    // Single-precision sums (the full probe: 0.049999952316284).
    assert!((n[0] - 0.35).abs() < 1e-6);
    assert!((n[1] - 0.05).abs() < 1e-6);
    assert!((n[2] - 50.0).abs() < 1e-9);
}
