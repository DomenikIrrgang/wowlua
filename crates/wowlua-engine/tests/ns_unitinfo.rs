//! The unit information `unitinfo.rs` answers from the world's units, the
//! group and the auras.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

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
fn a_hostile_creature_and_the_player_by_guid() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local none = UnitSelectionType('target')
local hogger = mock.spawnUnit({ name = 'Hogger', level = 11, reaction = 'hostile' })
mock.player:target(hogger)
local _, class = UnitClass('player')
local _, infoClass, _, _, _, name = GetPlayerInfoByGUID(UnitGUID('player'))
return none, UnitSelectionType('target'), C_PlayerInfo.GetContentDifficultyCreatureForPlayer('target'),
    infoClass == class, name == UnitName('player'), GetPlayerInfoByGUID(UnitGUID('target')) == nil,
    UnitNameFromGUID(UnitGUID('target')), UnitInAnyGroup('player'), UnitIsAFK('player')",
    );
    assert_eq!(
        r,
        [
            "4", "0", "0", "true", "true", "true", "Hogger", "false", "false"
        ]
    );
}

#[test]
fn the_players_auras_listed() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.player:applyAura({ spellId = 1459, name = 'Arcane Intellect', duration = 3600, source = mock.player })
local list = C_UnitAuras.GetUnitAuras('player', 'HELPFUL')
local id = list[1].auraInstanceID
return #list, list[1].name, #C_UnitAuras.GetUnitAuras('player', 'HARMFUL'),
    C_UnitAuras.GetUnitAuraBySpellID('player', 1459).name,
    C_UnitAuras.IsAuraFilteredOutByInstanceID('player', id, 'HARMFUL'),
    C_UnitAuras.GetAuraApplicationDisplayCount('player', id)",
    );
    assert_eq!(
        r,
        ["1", "Arcane Intellect", "0", "Arcane Intellect", "true", ""]
    );
}
