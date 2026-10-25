//! The chat windows' settings the client keeps (`chatwindows.rs`, decision
//! T263).

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
fn a_chat_windows_settings_are_kept() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local function info(id)
    local name, size, r, g, b, a, shown, locked, docked, unint = GetChatWindowInfo(id)
    return table.concat({ name, size, r, g, b, string.format('%.2f', a), tostring(shown), tostring(locked), tostring(docked), tostring(unint) }, ' ')
end
local before = info(1)
SetChatWindowName(1, 'Main')
SetChatWindowSize(1, 14)
SetChatWindowColor(1, 0.1, 0.2, 0.3)
SetChatWindowAlpha(1, 0.5)
SetChatWindowLocked(1, false)
SetChatWindowDocked(1, nil)
SetChatWindowUninteractable(1, true)
SetChatWindowShown(3, true)
SetChatWindowSavedPosition(3, 'BOTTOMLEFT', 0.1, 0.2)
SetChatWindowSavedDimensions(3, 400, 200)
local point, x, y = GetChatWindowSavedPosition(3)
local w, h = GetChatWindowSavedDimensions(3)
return before, info(1), select(7, GetChatWindowInfo(3)), point .. ' ' .. x .. ' ' .. y, w .. ' ' .. h, info(2)",
    );
    assert_eq!(
        r,
        [
            "General 18 0 0 0 0.16 true true 1 false",
            "Main 14 0.1 0.2 0.3 0.50 true false nil true",
            "true",
            "BOTTOMLEFT 0.1 0.2",
            "400 200",
            "Log 18 0 0 0 0.16 false true 2 false",
        ]
    );
}
