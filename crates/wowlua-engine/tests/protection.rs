//! Protected actions: refused for tainted code, with the client's events
//! and a taint report.

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

/// A frame that holds a protected frame is protected too, though not
/// explicitly: blocked in combat, and secure code may use its handle
/// (decision T254).
#[test]
fn a_frame_holding_a_protected_frame_is_protected() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ADDON_ACTION_BLOCKED')
f:SetScript('OnEvent', function(_, e, addon, action) seen[#seen + 1] = action end)
local holder = CreateFrame('Frame', nil, UIParent)
local inner = CreateFrame('Frame', nil, holder)
local button = CreateFrame('Button', nil, inner, 'SecureActionButtonTemplate')
local plain = CreateFrame('Frame', nil, UIParent)
local p1, e1 = holder:IsProtected()
local p2, e2 = plain:IsProtected()
mock.player:enterCombat()
holder:Hide()
plain:Hide()
return tostring(p1), tostring(e1), tostring(p2), tostring(e2), table.concat(seen, ','),
    tostring(holder:IsShown()), tostring(plain:IsShown()), tostring(holder:CanChangeAttribute())",
    );
    assert_eq!(
        r,
        [
            "true", "false", "false", "false", "Hide()", "true", "false", "false"
        ]
    );
}

/// Runs `src` as Blizzard's code (loaded without an addon), starting
/// untainted as the client runs each event.
fn secure(vm: &mut Vm, src: &str) -> Vec<String> {
    let r = vm.with_execution_taint(wowlua_vm::Taint::SECURE, |vm| {
        let f = vm
            .load(src.as_bytes(), "@Interface/AddOns/Blizzard_Test/test.lua")
            .expect("compiles");
        vm.call(f, &[])
    });
    match r {
        Ok(r) => r
            .into_iter()
            .map(|v| match v {
                Value::Num(n) => format!("{n}"),
                Value::Bool(b) => format!("{b}"),
                Value::Nil => "nil".into(),
                other => format!("{other:?}"),
            })
            .collect(),
        Err(e) => panic!("{:?}", e.value),
    }
}

/// A frame Blizzard's code puts under access restrictions (as its aura
/// containers do with each button, decisions T632, T636) is the addon's
/// until then: its build works. After, the restriction
/// (`DenyTaintedAccessWhenAurasAreSecret`) applies while auras are secret:
/// out of combat the addon uses it, `EnumerateFrames` lists it and the
/// addon's frame on it hands it out; in combat its methods raise Lua's
/// bad-`self` error ("bad argument #1 to '?'" through `pcall`, "calling
/// 'X' on bad self" as a method call), `EnumerateFrames` leaves it out and
/// `GetParent` of the addon's frame on it raises the same error. Its
/// `IsForbidden` answers false throughout. Blizzard's code still uses
/// both. An addon's own `AddAccessRestrictions` does nothing.
#[test]
fn restricted_frames_refuse_addon_code_while_auras_are_secret() {
    let mut vm = vm();
    secure(
        &mut vm,
        "RestrictedButton = CreateFrame('Button', nil, UIParent)",
    );
    let built = run(
        &mut vm,
        "local b = RestrictedButton
Kid = CreateFrame('Frame', nil, b)
b:SetSize(20, 20)
b:CreateTexture()
local own = CreateFrame('Frame')
own:AddAccessRestrictions(1)
return tostring(b:IsForbidden()), tostring(Kid:GetParent() == b), own:GetAccessRestrictions(), tostring(own:IsForbidden())",
    );
    assert_eq!(built, ["false", "true", "0", "false"]);
    let inside = secure(
        &mut vm,
        "local b = RestrictedButton
b:AddAccessRestrictions(1) -- DenyTaintedAccessWhenAurasAreSecret
local kid = b:GetChildren()
return b:IsForbidden(), b:GetWidth(), kid:GetParent() == b, b:GetAccessRestrictions(),
    b:HasAnyAccessRestrictions(), b:CanBeAccessedInContext()",
    );
    assert_eq!(inside, ["false", "20", "true", "1", "true", "true"]);
    let ask = "local b = RestrictedButton
local ok, err = pcall(b.GetWidth, b)
local ok2, err2 = pcall(Kid.GetParent, Kid)
local ok3, err3 = pcall(function() return b:GetWidth() end)
local listed = false
local f = EnumerateFrames()
while f do
    if f == b then listed = true end
    f = EnumerateFrames(f)
end
return tostring(C_Secrets.ShouldAurasBeSecret()), tostring(b:IsForbidden()), tostring(b:CanBeAccessedInContext()),
    tostring(b:HasAccessConstraints()), tostring(ok), tostring(err), tostring(Kid:IsForbidden()), tostring(ok2),
    tostring(err2 == b or err2), tostring(ok3), tostring(err3), tostring(listed)";
    let out_of_combat = run(&mut vm, ask);
    assert_eq!(
        out_of_combat,
        [
            "false", "false", "true", "true", "true", "20", "false", "true", "true", "true", "20",
            "true",
        ]
    );
    run(&mut vm, "mock.player:enterCombat()");
    let in_combat = run(&mut vm, ask);
    assert_eq!(
        in_combat,
        [
            "true",
            "false",
            "false",
            "true",
            "false",
            "bad argument #1 to '?' (Attempt to access forbidden object from code tainted by an AddOn - Usage: local width = self:GetWidth([ignoreRect]))",
            "false",
            "false",
            "bad argument #1 to '?' (Attempt to access forbidden object from code tainted by an AddOn - Usage: local parent = self:GetParent())",
            "false",
            "MyAddon/main.lua:4: calling 'GetWidth' on bad self (Attempt to access forbidden object from code tainted by an AddOn - Usage: local width = self:GetWidth([ignoreRect]))",
            "false",
        ]
    );
    let secure_in_combat = secure(
        &mut vm,
        "local b = RestrictedButton
local kid = b:GetChildren()
return b:GetWidth(), kid:GetParent() == b",
    );
    assert_eq!(secure_in_combat, ["20", "true"]);
    run(&mut vm, "mock.player:leaveCombat()");
    assert_eq!(run(&mut vm, ask), out_of_combat);
}
