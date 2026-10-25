//! The guild's message of the day blocked for addons in combat (decision
//! T265).

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
fn the_guilds_message_is_refused_to_addons_in_combat() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ADDON_ACTION_BLOCKED')
f:SetScript('OnEvent', function(_, e, addon, action) seen[#seen + 1] = addon .. ' ' .. action end)
local before = type(GetGuildRosterMOTD())
mock.player:enterCombat()
local n = select('#', pcall(GetGuildRosterMOTD)) + select('#', pcall(C_GuildInfo.GetMOTD))
mock.player:leaveCombat()
local after = type(GetGuildRosterMOTD())
return before, n, after, table.concat(seen, ', ')",
    );
    assert_eq!(
        r,
        [
            "string",
            "2",
            "string",
            "MyAddon UNKNOWN(), MyAddon UNKNOWN()"
        ]
    );
}
