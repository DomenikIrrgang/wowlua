//! The damage meter's sessions (`C_DamageMeter`) from the engine's fights,
//! and the client's rules Details! and WeakAuras ran into on WoW Forever.

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
fn a_fight_fills_the_sessions_and_tells_them() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('DAMAGE_METER_COMBAT_SESSION_UPDATED')
f:RegisterEvent('DAMAGE_METER_CURRENT_SESSION_UPDATED')
f:SetScript('OnEvent', function(_, e, ty, id)
    seen[#seen + 1] = e == 'DAMAGE_METER_CURRENT_SESSION_UPDATED' and 'current' or (ty .. ':' .. id)
end)
mock.setDamageMeterSession(3)
local before = #C_DamageMeter.GetCombatSessionFromType(1, 0).combatSources
local wolf = mock.spawnUnit({ name = 'Young Wolf', guid = 'Creature-0-1-0-1-299-0000000001', reaction = 'hostile', health = 57 })
mock.player:target(wolf)
mock.player:swing(wolf, 6)
wolf:swing(mock.player, 2)
mock.player:swing(wolf, 0, { kind = 'PARRY' })
local inCombat = C_DamageMeter.GetCombatSessionFromType(1, 0)
local secretName = issecretvalue(inCombat.combatSources[1].name)
local plainClass = inCombat.combatSources[1].classFilename
mock.advanceTime(2)
mock.player:swing(wolf, 51, { kill = { xp = 50 } })
mock.player:leaveCombat()
local s = C_DamageMeter.GetCombatSessionFromType(1, 0)
local src = s.combatSources[1]
local spells = C_DamageMeter.GetCombatSessionSourceFromType(1, 0, src.sourceGUID)
local enemy = C_DamageMeter.GetCombatSessionFromID(3, 10).combatSources[1]
local taken = C_DamageMeter.GetCombatSessionFromType(0, 7).combatSources[1]
local list = C_DamageMeter.GetAvailableCombatSessions()
return table.concat(seen, ' '), before, tostring(secretName), plainClass, src.name, src.totalAmount, s.totalAmount,
    tostring(src.isLocalPlayer), spells.combatSpells[1].spellID, spells.combatSpells[1].combatSpellDetails.unitName,
    enemy.name, enemy.sourceCreatureID, enemy.totalAmount, taken.name, taken.totalAmount, #list, list[1].sessionID,
    C_DamageMeter.GetSessionDurationSeconds(1)",
    );
    assert_eq!(
        r,
        [
            // The session begun with the fight; each blow the player deals
            // tells DamageDone and Dps, current then overall, then
            // EnemyDamageTaken; the blow taken DamageTaken; the parry
            // nothing.
            "current 0:3 1:3 0:0 1:0 10:3 10:0 7:3 7:0 0:3 1:3 0:0 1:0 10:3 10:0",
            "0",
            "true",
            "WARRIOR",
            "Player",
            "57",
            "57",
            "true",
            "6603",
            "Young Wolf",
            "Young Wolf",
            "299",
            "57",
            "Player",
            "2",
            "1",
            "3",
            "2"
        ]
    );
}

#[test]
fn a_fight_soon_after_goes_on_in_its_session() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local news = 0
local f = CreateFrame('Frame')
f:RegisterEvent('DAMAGE_METER_CURRENT_SESSION_UPDATED')
f:RegisterEvent('DAMAGE_METER_RESET')
local reset = false
f:SetScript('OnEvent', function(_, e) if e == 'DAMAGE_METER_RESET' then reset = true else news = news + 1 end end)
local a = mock.spawnUnit({ name = 'A', reaction = 'hostile', health = 50 })
mock.player:enterCombat()
mock.player:swing(a, 5)
mock.player:leaveCombat()
mock.advanceTime(2)
mock.player:enterCombat()
mock.player:swing(a, 5)
mock.player:leaveCombat()
local one = #C_DamageMeter.GetAvailableCombatSessions()
mock.advanceTime(5)
mock.player:enterCombat()
mock.player:leaveCombat()
local two = #C_DamageMeter.GetAvailableCombatSessions()
C_DamageMeter.ResetAllCombatSessions()
return news, one, two, tostring(reset), #C_DamageMeter.GetAvailableCombatSessions(),
    C_DamageMeter.GetCombatSessionFromType(0, 0).totalAmount",
    );
    assert_eq!(r, ["2", "1", "2", "true", "0", "0"]);
}

#[test]
fn heals_and_deaths_tell_their_types() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('DAMAGE_METER_COMBAT_SESSION_UPDATED')
f:SetScript('OnEvent', function(_, _, ty, id) seen[#seen + 1] = ty .. ':' .. id end)
local cat = mock.spawnUnit({ name = 'Cat', reaction = 'hostile', health = 50 })
mock.player:hit(cat, 10, { school = 2, spell = 20271 })
mock.player:hit(mock.player, 5, { kind = 'HEAL', spell = 635 })
mock.player:die()
return table.concat(seen, ' ')",
    );
    assert_eq!(
        r,
        ["0:1 1:1 0:0 1:0 10:1 10:0 2:1 3:1 2:0 3:0 7:1 7:0 9:1 9:0"]
    );
}

#[test]
fn timer_handles_keep_fields_written_to_them() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local got
local t = C_Timer.NewTicker(1, function(self) got = self.payload end, 1)
t.payload = 'data'
mock.advanceTime(1.5)
return type(t), got, t.payload, tostring(t.IsCancelled ~= nil), tostring(t.nothing)",
    );
    assert_eq!(r, ["userdata", "data", "data", "true", "nil"]);
}

#[test]
fn create_frame_ignores_an_id_that_is_no_number() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame', nil, UIParent, nil, 'BackdropTemplate')
local g = CreateFrame('Frame', nil, UIParent, nil, 7)
return f:GetID(), g:GetID()",
    );
    assert_eq!(r, ["0", "7"]);
}

#[test]
fn abbreviations_may_divide_by_one() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local ok = pcall(CreateAbbreviateConfig, {
    { breakpoint = 1000, abbreviation = 'K', significandDivisor = 100, fractionDivisor = 10, abbreviationIsGlobal = false },
    { breakpoint = 1, abbreviation = '', significandDivisor = 1, fractionDivisor = 1, abbreviationIsGlobal = false },
})
local bad = pcall(CreateAbbreviateConfig, {
    { breakpoint = 60, abbreviation = 'm', significandDivisor = 60, fractionDivisor = 1, abbreviationIsGlobal = false },
})
return tostring(ok), tostring(bad)",
    );
    assert_eq!(r, ["true", "false"]);
}

#[test]
fn a_used_spell_tells_its_cooldown() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineSpell(20600, { name = 'Perception', known = true, cooldown = 180, targeted = false })
local before = C_Spell.GetSpellCooldown(20600)
mock.player:useSpell(20600)
mock.advanceTime(1)
local cd = C_Spell.GetSpellCooldown(20600)
return before.duration, tostring(before.isActive), cd.duration, tostring(cd.isActive),
    math.floor((GetTime() - cd.startTime) * 10 + 0.5) / 10",
    );
    // Running since the server's answer, a ping (0.4 s) after the press.
    assert_eq!(r, ["0", "false", "180", "true", "0.6"]);
}
