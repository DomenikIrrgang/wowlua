//! The long tail of namespaces (`extras.rs`): the systems Forever lacks
//! answer "nothing here", the small helpers answer from the world.

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
fn systems_forever_lacks_answer_nothing() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "return C_Garrison.IsOnGarrisonMap(), C_Garrison.GetNumFollowers(), type(C_Garrison.GetFollowerAbilities(1)),
    C_Garrison.GetFollowerInfo(1), C_Covenants.GetActiveCovenantID(), C_Commentator.IsSpectating()",
    );
    assert_eq!(r, ["false", "0", "table", "nil", "0", "false"]);
}

#[test]
fn creatures_and_races() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local race = C_CreatureInfo.GetRaceInfo(5)
local faction = C_CreatureInfo.GetFactionInfo(5)
return C_CreatureInfo.GetCreatureID('Creature-0-1-2-3-299-0000ABCD'), C_CreatureInfo.GetCreatureID('Player-0000-00000000'),
    race.raceName, race.clientFileString, faction.groupTag",
    );
    assert_eq!(r, ["299", "nil", "Undead", "Scourge", "Horde"]);
}

#[test]
fn text_helpers() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "return C_Intl.ToUpper('abc'), C_Intl.ToTitle('hello world'), C_Intl.FormatNumber(1234.5, 0),
    C_Intl.FormatNumber(1234567, 1), C_Intl.Length('abc'), C_Intl.CompareStrings('a', 'B', 0)",
    );
    assert_eq!(r, ["ABC", "Hello World", "1,234.5", "1,234,567", "3", "-1"]);
}

#[test]
fn settings_are_kept() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local before = C_Trainer.GetCategorizeTrainerUI()
C_Trainer.SetCategorizeTrainerUI(false)
C_NamePlateManager.SetNamePlateHitTestInsets(1, 2, 3, 4, 5)
local l, r, t, b = C_NamePlateManager.GetNamePlateHitTestInsets(1)
return before, C_Trainer.GetCategorizeTrainerUI(), l, r, t, b,
    C_PlayerInteractionManager.IsInteractingWithNpcOfType(Enum.PlayerInteractionType.Merchant)",
    );
    assert_eq!(r, ["true", "false", "2", "3", "4", "5", "false"]);
}
