//! Protected actions: refused for tainted code, with the client's events
//! and a taint report.

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
fn protected_functions_are_forbidden_to_addons() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ADDON_ACTION_FORBIDDEN')
f:SetScript('OnEvent', function(_, e, addon, action) seen[#seen + 1] = e .. ' ' .. addon .. ' ' .. action end)
local mob = mock.spawnUnit({ name = 'Mob' })
local n = select('#', TargetUnit('Mob'))
local refused = mock.refusals()
return table.concat(seen, ','), n, tostring(UnitExists('target')), #refused, refused[1].action,
    refused[1].report:find('main.lua', 1, true) ~= nil, type(CastSpellByName), type(C_Container.UseContainerItem)",
    );
    assert_eq!(
        r,
        [
            "ADDON_ACTION_FORBIDDEN MyAddon TargetUnit()",
            "0",
            "false",
            "1",
            "TargetUnit()",
            "true",
            "function",
            "function"
        ]
    );
}

#[test]
fn protected_frames_are_blocked_in_combat() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ADDON_ACTION_BLOCKED')
f:SetScript('OnEvent', function(_, e, addon, action) seen[#seen + 1] = action end)
local named = CreateFrame('Button', 'MySecureButton', UIParent, 'SecureActionButtonTemplate')
local anon = CreateFrame('Button', nil, UIParent, 'SecureUnitButtonTemplate')
local plain = CreateFrame('Button', nil, UIParent)
named:Hide()
local outOfCombat = named:IsShown()
named:Show()
mock.player:enterCombat()
named:Hide()
anon:SetPoint('CENTER')
plain:Hide()
named:SetAlpha(0.5)
return tostring(named:IsProtected()), tostring(outOfCombat), table.concat(seen, ','), tostring(named:IsShown()),
    anon:GetNumPoints(), tostring(plain:IsShown()), named:GetAlpha(), #mock.refusals()",
    );
    assert_eq!(
        r,
        [
            "true",
            "false",
            "Hide(),SetPoint()",
            "true",
            "0",
            "false",
            "0.501960813999176",
            "2"
        ]
    );
}

/// An addon's spellbook pickup is blocked in combat (decision T253), not
/// out of it.
#[test]
fn a_spellbook_pickup_is_blocked_in_combat() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ADDON_ACTION_BLOCKED')
f:SetScript('OnEvent', function(_, e, addon, action) seen[#seen + 1] = addon .. ' ' .. action end)
local bank = Enum.SpellBookSpellBank.Player
C_SpellBook.PickupSpellBookItem(1, bank)
local outside = #seen
mock.player:enterCombat()
C_SpellBook.PickupSpellBookItem(1, bank)
return outside, table.concat(seen, ','), #mock.refusals()",
    );
    assert_eq!(r, ["0", "MyAddon PickupSpellBookItem()", "1"]);
}
