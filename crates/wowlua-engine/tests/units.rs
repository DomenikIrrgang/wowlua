//! Units, tokens, combat and restrictions, driven by the controls.

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
fn targeting_an_enemy_and_reading_it() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('PLAYER_TARGET_CHANGED')
f:RegisterUnitEvent('UNIT_HEALTH', 'target')
f:SetScript('OnEvent', function(_, e, unit) seen[#seen + 1] = e .. (unit and (':' .. unit) or '') end)
local hogger = mock.spawnUnit({ name = 'Hogger', level = 11, reaction = 'hostile', health = 666 })
mock.player:target(hogger)
local name = UnitName('target')
hogger:damage(100)
return table.concat(seen, ' '), name, UnitLevel('target'), UnitExists('target'), UnitCanAttack('player', 'target'),
    UnitIsEnemy('player', 'target'), issecretvalue(UnitHealth('target')), UnitIsPlayer('target')",
    );
    assert_eq!(
        r,
        [
            "PLAYER_TARGET_CHANGED UNIT_HEALTH:target",
            "Hogger",
            "11",
            "true",
            "true",
            "true",
            "true",
            "false"
        ]
    );
}

#[test]
fn combat_restricts_and_locks_down() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('PLAYER_REGEN_DISABLED')
f:RegisterEvent('ADDON_RESTRICTION_STATE_CHANGED')
f:SetScript('OnEvent', function(_, e, t, s) seen[#seen + 1] = e .. (t and ('(' .. t .. ',' .. s .. ')') or '') .. ':' .. tostring(InCombatLockdown()) end)
local mob = mock.spawnUnit({ name = 'Mob', power = 50 })
mock.player:target(mob)
local before = issecretvalue(UnitPower('target'))
mock.player:enterCombat()
return table.concat(seen, ' '), before, issecretvalue(UnitPower('target')), InCombatLockdown(),
    C_RestrictedActions.IsAddOnRestrictionActive(Enum.AddOnRestrictionType.Combat), C_Secrets.ShouldAurasBeSecret(),
    issecretvalue(UnitName('target')), C_RestrictedActions.CheckAllowProtectedFunctions(UIParent, true)",
    );
    assert_eq!(
        r,
        [
            "PLAYER_REGEN_DISABLED:false ADDON_RESTRICTION_STATE_CHANGED(0,1):false",
            "false",
            "true",
            "true",
            "true",
            "true",
            "false",
            "false"
        ]
    );
}

#[test]
fn identity_is_secret_on_restricted_maps() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local mob = mock.spawnUnit({ name = 'Mob' })
local friend = mock.spawnUnit({ name = 'Friend', player = true, reaction = 'friendly' })
mock.addToGroup(friend)
mock.player:target(mob)
mock.enterInstance()
return issecretvalue(UnitName('target')), issecretvalue(UnitName('party1')), UnitName('party1'), issecretvalue(UnitGUID('player')),
    issecretvalue(UnitHealthMax('party1')), issecretvalue(UnitHealthMax('target')), GetNumGroupMembers(), IsInGroup()",
    );
    assert_eq!(
        r,
        [
            "true", "false", "Friend", "false", "false", "true", "2", "true"
        ]
    );
}

#[test]
fn nameplates_and_compound_tokens() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local added = {}
local f = CreateFrame('Frame')
f:RegisterEvent('NAME_PLATE_UNIT_ADDED')
f:SetScript('OnEvent', function(_, _, unit) added[#added + 1] = unit end)
local a = mock.spawnUnit({ name = 'A' })
local b = mock.spawnUnit({ name = 'B' })
a:showNameplate()
b:showNameplate()
a:target(mock.player)
mock.player:target(a)
return table.concat(added, ','), UnitName('nameplate2'), UnitName('targettarget'), UnitIsUnit('target', 'nameplate1'),
    UnitExists('nameplate3'), (UnitName('targettargettarget'))",
    );
    assert_eq!(
        r,
        ["nameplate1,nameplate2", "B", "Player", "true", "false", "A"]
    );
}
