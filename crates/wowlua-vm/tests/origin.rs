//! Taint reports (spec emulator §17.4, decision T196): the chain from
//! secure code that became tainted back to the addon line behind it.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

/// The running execution's report, as a refused protected call gets it.
fn report(vm: &mut Vm, _c: NativeCall) -> Ret {
    let r = vm.execution_report();
    Ok(vec![vm.str(&r)])
}

#[test]
fn chain_through_secure_code() {
    let mut vm = Vm::wow();
    let f = vm.native("report", report);
    vm.set_global("report", f);
    // Blizzard's code, secure: one function stores what it computed from a
    // global, the other reads it back.
    let blizzard = "ActionButton1 = {}
function ActionBarButton_Update() ActionButton1.action = ACTIONBAR_PAGE * 12 end
function ActionButton_OnClick() local action = ActionButton1.action return report() end
function OnUpdate() ActionBarButton_Update() end
function OnClick() local report = ActionButton_OnClick() return report end";
    let chunk = vm
        .load(blizzard.as_bytes(), "=ActionButton.lua")
        .expect("compiles");
    vm.call(chunk, &[]).expect("runs");
    // The addon writes the global.
    let addon = vm
        .load_as(b"ACTIONBAR_PAGE = 2", "=MyAddon/bars.lua", "MyAddon")
        .expect("compiles");
    vm.call(addon, &[]).expect("runs");
    // The engine calls Blizzard's code secure, each event on its own.
    for event in ["OnUpdate", "OnClick"] {
        vm.taint_mut().stack = Default::default();
        let handler = vm.get_global(event);
        let r = vm.call(handler, &[]).expect("runs");
        if event == "OnUpdate" {
            continue;
        }
        let Value::Str(s) = r[0] else {
            panic!("a report expected")
        };
        assert_eq!(
            String::from_utf8_lossy(vm.str_bytes(s)),
            "  ActionButton_OnClick  ActionButton.lua:3
    read field 'action'
  written by ActionBarButton_Update
    ActionButton.lua:2
    while tainted by MyAddon, after reading
  global 'ACTIONBAR_PAGE'
    written by MyAddon  MyAddon/bars.lua:1
"
        );
    }
}

/// The report for a stored value (a test's security assertion).
#[test]
fn stored_value() {
    let mut vm = Vm::wow();
    let addon = vm
        .load_as(
            b"SomeFrame = {}\nSomeFrame.flag = true",
            "=MyAddon/core.lua",
            "MyAddon",
        )
        .expect("compiles");
    vm.call(addon, &[]).expect("runs");
    let Value::Table(t) = vm.get_global("SomeFrame") else {
        panic!("a table")
    };
    let key = vm.str("flag");
    assert_eq!(
        vm.value_report(t, key),
        "    written by MyAddon  MyAddon/core.lua:2\n"
    );
}

#[test]
fn slot_size() {
    // The origin sits in the slot's padding.
    assert_eq!(std::mem::size_of::<wowlua_vm::taint::Slot>(), 24);
}
