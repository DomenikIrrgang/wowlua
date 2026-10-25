//! The client's global functions `globalsinfo.rs` answers: kept settings,
//! the table aliases, answers from the engine's world.

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
fn settings_are_kept() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local before = ShowingHelm()
ShowHelm(false)
local tab = GetCurrentGuildBankTab()
SetCurrentGuildBankTab(3)
EJ_SelectTier(2)
local logging = LoggingCombat(true)
return before, ShowingHelm(), tab, GetCurrentGuildBankTab(), EJ_GetCurrentTier(), logging, LoggingCombat()",
    );
    // The helmet not shown, and unchanged right after `ShowHelm` (the full
    // probe: the server answers later).
    assert_eq!(r, ["false", "false", "1", "3", "2", "true", "true"]);
}

#[test]
fn table_aliases_and_text() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "return TableIsEmpty({}), TableIsEmpty({ 1 }), CalculateStringEditDistance('kitten', 'sitting'),
    (GetGameMessageInfo(LE_GAME_ERR_ITEM_MAX_COUNT))",
    );
    assert_eq!(r, ["true", "false", "3", "ERR_ITEM_MAX_COUNT"]);
}

#[test]
fn answers_from_the_world() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local language, id = GetDefaultLanguage()
return GetNumClasses(), IsDualWielding(), language, id, GetRealmID()",
    );
    // The default player is a Human: the Alliance's Common.
    assert_eq!(r, ["9", "false", "Common", "7", "4618"]);
}
