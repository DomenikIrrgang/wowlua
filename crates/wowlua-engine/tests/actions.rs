//! The action bars: a slot's action, read as the client answers.

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
fn slots_answer_their_actions_as_recorded() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ACTIONBAR_SLOT_CHANGED')
f:SetScript('OnEvent', function(_, e, slot) seen[#seen + 1] = slot end)
mock.setAction(1, { type = 'spell', id = 20271, subType = 'spell', texture = 135959, count = '', usable = { false, false } })
mock.setAction(2, { type = 'spell', id = 1311656, subType = 'spell', texture = 135957 })
local cd1, cd5 = C_ActionBar.GetActionCooldown(1), C_ActionBar.GetActionCooldown(5)
return table.concat(seen, ','),
    C_ActionBar.HasAction(1), C_ActionBar.HasAction(5), select('#', C_ActionBar.HasAction(0)),
    table.concat({ GetActionInfo(2) }, ' '), select('#', GetActionInfo(5)),
    C_ActionBar.GetActionTexture(1), C_ActionBar.GetActionTexture(5),
    table.concat({ tostring(C_ActionBar.IsUsableAction(1)), tostring(select(2, C_ActionBar.IsUsableAction(1))) }, ' '),
    (C_ActionBar.IsUsableAction(2)), (C_ActionBar.IsUsableAction(5)),
    cd1.isEnabled, cd5.isEnabled, C_ActionBar.GetActionDisplayCount(5),
    select('#', C_ActionBar.GetActionCooldown(0)), C_ActionBar.IsAssistedCombatAction(0)",
    );
    assert_eq!(
        r,
        [
            "1,2",
            "true",
            "false",
            "0",
            "spell 1311656 spell",
            "0",
            "135959",
            "nil",
            "false false",
            "true",
            "false",
            "true",
            "false",
            "",
            "0",
            "false",
        ]
    );
}

#[test]
fn a_cleared_slot_has_no_action() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.setAction(3, { type = 'item', id = 6948 })
mock.setAction(3, nil)
return C_ActionBar.HasAction(3), select('#', GetActionInfo(3))",
    );
    assert_eq!(r, ["false", "0"]);
}
