//! Instances and boss encounters, the player's pet, resurrection by
//! another: what the controls do (all unverified against the client: no
//! recording has them yet).

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
fn an_instance_and_its_encounter() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'ENCOUNTER_START', 'ENCOUNTER_END', 'BOSS_KILL', 'INSTANCE_ENCOUNTER_ENGAGE_UNIT' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, ...)
    local a = { ... }
    seen[#seen + 1] = e .. '(' .. table.concat({ tostring(a[1]), tostring(a[2]), tostring(a[3]), tostring(a[4]), tostring(a[5]) }, ',') .. ')'
        .. ':' .. tostring(UnitExists('boss1'))
end)
local before = table.concat({ tostring(IsInInstance()), select(2, IsInInstance()) }, ' ')
mock.enterInstance('party', { name = 'The Deadmines', instanceID = 36 })
local inside = table.concat({ tostring(IsInInstance()), select(2, IsInInstance()) }, ' ')
local name, kind, difficulty, difficultyName, size, _, _, id = GetInstanceInfo()
local boss = mock.spawnUnit({ name = 'Edwin VanCleef', level = 21, reaction = 'hostile', health = 100, maxHealth = 400 })
mock.startEncounter({ id = 2973, name = 'Edwin VanCleef', bosses = { boss } })
local going = C_InstanceEncounter.IsEncounterInProgress()
mock.endEncounter(true)
local status
f:SetScript('OnEvent', nil)
mock.startEncounter({ id = 2973, name = 'Edwin VanCleef', bosses = { boss } })
f:SetScript('OnEvent', function(_, e, _, _, _, _, success, units)
    if e == 'ENCOUNTER_END' then status = success .. ' ' .. units[1].creatureName .. ' ' .. units[1].remainingHealthPercent end
end)
mock.leaveInstance()
return before, inside, name, kind, difficulty, difficultyName, size, id, tostring(going),
    table.concat(seen, ' | '), status, tostring(IsInInstance()), tostring(C_InstanceEncounter.IsEncounterInProgress())",
    );
    assert_eq!(
        r,
        [
            "false none",
            "true party",
            "The Deadmines",
            "party",
            "1",
            "Normal",
            "5",
            "36",
            "true",
            "ENCOUNTER_START(2973,Edwin VanCleef,1,5,nil):false \
             | INSTANCE_ENCOUNTER_ENGAGE_UNIT(nil,nil,nil,nil,nil):true \
             | BOSS_KILL(2973,Edwin VanCleef,nil,nil,nil):true \
             | ENCOUNTER_END(2973,Edwin VanCleef,1,5,1):true \
             | INSTANCE_ENCOUNTER_ENGAGE_UNIT(nil,nil,nil,nil,nil):false",
            "0 Edwin VanCleef 25",
            "false",
            "false",
        ]
    );
}

#[test]
fn the_players_pet() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'UNIT_PET', 'PET_BAR_UPDATE', 'UNIT_HAPPINESS', 'UNIT_PET_EXPERIENCE' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, unit) seen[#seen + 1] = e .. (unit and (':' .. unit) or '') end)
local none = table.concat({ tostring(UnitExists('pet')), tostring(HasPetUI()), tostring(PetHasActionBar()) }, ' ')
local wolf = mock.spawnUnit({ name = 'Fang', level = 19, reaction = 'friendly' })
mock.setPet(wolf, { happiness = 2, xp = 10, xpMax = 100,
    actions = { [4] = { spell = 17253, name = 'Bite', icon = 132278, autocast = true } } })
local has, hunter = HasPetUI()
local attack = { GetPetActionInfo(1) }
local bite = { GetPetActionInfo(4) }
mock.setPet(wolf, { happiness = 3, xp = 20 })
local happiness, damage = C_PetInfo.GetPetHappiness()
local xp, max = GetPetExperience()
mock.setPet(nil)
return none, UnitName('pet') or 'none', tostring(has), tostring(hunter),
    table.concat({ attack[1], attack[2], tostring(attack[3]), tostring(attack[4]) }, ' '),
    table.concat({ bite[1], bite[2], tostring(bite[3]), tostring(bite[5]), tostring(bite[6]), bite[7] }, ' '),
    happiness .. ' ' .. damage, xp .. '/' .. max, table.concat(seen, ' '), tostring(UnitExists('pet'))",
    );
    assert_eq!(
        r,
        [
            "false false false",
            "none",
            "true",
            "true",
            "PET_ACTION_ATTACK PET_ATTACK_TEXTURE true false",
            "Bite 132278 false true true 17253",
            "3 125",
            "20/100",
            "UNIT_PET:player PET_BAR_UPDATE UNIT_HAPPINESS:pet UNIT_HAPPINESS:pet \
             UNIT_PET_EXPERIENCE:pet UNIT_PET:player PET_BAR_UPDATE",
            "false",
        ]
    );
}

#[test]
fn a_resurrection_offered_and_taken() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'RESURRECT_REQUEST', 'PLAYER_ALIVE', 'UNIT_HEALTH' }) do f:RegisterEvent(e) end
f:SetScript('OnEvent', function(_, e, arg) seen[#seen + 1] = e .. (arg and (':' .. arg) or '') end)
local priest = mock.spawnUnit({ name = 'Arnhild', level = 20, reaction = 'friendly', player = true })
local rogue = mock.spawnUnit({ name = 'Corvin', level = 20, reaction = 'friendly', player = true, maxHealth = 1000 })
mock.addToGroup(priest)
mock.addToGroup(rogue)
rogue:kill()
priest:resurrect(rogue, { health = 350 })
local member = tostring(UnitIsDead('party2'))
mock.player:die()
seen = {}
priest:resurrect(mock.player)
local offerer = ResurrectGetOfferer()
AcceptResurrect()
local waiting = tostring(UnitIsDead('player'))
mock.advanceTime(1)
return member, offerer, waiting, tostring(UnitIsDead('player')), tostring((ResurrectGetOfferer())), table.concat(seen, ' ')",
    );
    assert_eq!(
        r,
        [
            "false",
            "Arnhild",
            "true",
            "false",
            "nil",
            "RESURRECT_REQUEST:Arnhild PLAYER_ALIVE UNIT_HEALTH:player",
        ]
    );
}
