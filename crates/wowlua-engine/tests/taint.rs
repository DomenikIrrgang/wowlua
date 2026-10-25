//! StdTest's taint experiments (`/stdtest taint`, fixture
//! `wowlua-vm/tests/client/forever-camelot-taint.lua`) with camelot's UI
//! code loaded: an addon taints one value on the path a click on action
//! button 1 takes to the protected `UseAction`; the click is then forbidden
//! exactly when the client forbade it.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use std::path::{Path, PathBuf};

use wowlua_vm::{Value, Vm};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A VM with camelot's UI code, or nothing when it is not on disk.
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
    wowlua_engine::controls::install(&mut vm, "__engine");
    wowlua_engine::ui::load(
        &mut vm,
        &interface,
        strings.exists().then_some(Path::new(&strings)),
    );
    Some(vm)
}

fn text(vm: &Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    }
}

/// Runs addon code as StdTest's.
fn addon(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(
            src.as_bytes(),
            "@Interface/AddOns/StdTest/taintlab.lua",
            "StdTest",
        )
        .expect("compiles");
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// The experiments: what each taints, as taintlab.wowlua applies it.
const EXPERIMENTS: &[(&str, &str)] = &[
    ("baseline: nothing tainted (the click must work)", ""),
    (
        "a Lua global on the path, reassigned to itself (reading a tainted global)",
        "KeybindFrames_InQuickKeybindMode = KeybindFrames_InQuickKeybindMode",
    ),
    (
        "a C function's global on the path, reassigned to itself",
        "GetCursorInfo = GetCursorInfo",
    ),
    (
        "the button's own field TriggerSecureClick, reassigned (a tainted table field)",
        "ActionButton1.TriggerSecureClick = ActionButton1.TriggerSecureClick",
    ),
    (
        "Settings.GetValue, reassigned (a field of a Blizzard table)",
        "Settings.GetValue = Settings.GetValue",
    ),
    (
        "hooksecurefunc on the Lua global (hooks must not taint)",
        "hooksecurefunc('KeybindFrames_InQuickKeybindMode', function() end)",
    ),
    (
        "hooksecurefunc on the button's field",
        "hooksecurefunc(ActionButton1, 'TriggerSecureClick', function() end)",
    ),
    (
        "setfenv on the Lua global's function, same environment (does it taint the function?)",
        "local f = KeybindFrames_InQuickKeybindMode setfenv(f, getfenv(f))",
    ),
];

/// The fixture: each experiment's name and what was blocked.
fn fixture() -> Vec<(String, Vec<String>)> {
    let src = std::fs::read(repo().join("crates/wowlua-vm/tests/client/forever-camelot-taint.lua"))
        .expect("the fixture");
    let mut vm = Vm::new();
    let f = vm.load(&src, "=fixture").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(list) = r[0] else {
        panic!("a list")
    };
    let mut out = Vec::new();
    for i in 1.. {
        let Value::Table(e) = vm.heap.table(list).get(Value::Num(f64::from(i))) else {
            break;
        };
        let k = vm.str("name");
        let name = text(&vm, vm.heap.table(e).get(k));
        let k = vm.str("blocked");
        let mut blocked = Vec::new();
        if let Value::Table(b) = vm.heap.table(e).get(k) {
            for j in 1.. {
                match vm.heap.table(b).get(Value::Num(f64::from(j))) {
                    Value::Nil => break,
                    v => blocked.push(text(&vm, v)),
                }
            }
        }
        out.push((name, blocked));
    }
    out
}

#[test]
fn a_click_is_forbidden_when_the_client_forbade_it() {
    let expected = fixture();
    let mut differences = Vec::new();
    for (name, taint) in EXPERIMENTS {
        let Some(mut vm) = client() else {
            eprintln!("skipped: camelot's UI code is not on disk");
            return;
        };
        addon(
            &mut vm,
            "StdTestBlocked = {}
StdTestErrors = {}
seterrorhandler(function(e) StdTestErrors[#StdTestErrors + 1] = tostring(e) end)
local f = CreateFrame('Frame')
f:RegisterEvent('ADDON_ACTION_BLOCKED')
f:RegisterEvent('ADDON_ACTION_FORBIDDEN')
f:SetScript('OnEvent', function(_, event, ...)
    StdTestBlocked[#StdTestBlocked + 1] = event .. ': ' .. strjoin(', ', tostringall(...))
end)",
        );
        if !taint.is_empty() {
            addon(&mut vm, taint);
        }
        // A mouse click: a fresh, secure execution.
        let button = vm.get_global("ActionButton1");
        assert!(
            wowlua_engine::widgets::widget_of(&vm, button).is_some(),
            "ActionButton1 exists"
        );
        let id = wowlua_engine::widgets::widget_of(&vm, button).unwrap();
        let left = vm.str("LeftButton");
        wowlua_engine::widgets::run_script(&mut vm, id, "OnClick", &[left, Value::Bool(false)]);
        let blocked = addon(&mut vm, "return table.concat(StdTestBlocked, '; ')");
        let ours: Vec<String> = blocked[0]
            .split("; ")
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let theirs = expected
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        if ours != theirs {
            let errors = addon(&mut vm, "return table.concat(StdTestErrors, ' | ')");
            differences.push(format!(
                "{name}\n    client: {theirs:?}\n    ours:   {ours:?}\n    errors: {}",
                errors[0]
            ));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
