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
    call(vm, f)
}

/// Runs `src` as Blizzard's secure code runs.
fn run_secure(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "=secure").expect("compiles");
    call(vm, f)
}

/// Runs `f` as its own execution (from C: the taint it ends with is not
/// the next one's).
fn call(vm: &mut Vm, f: Value) -> Vec<String> {
    let text = |vm: &Vm, v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    };
    match vm.call_secure(f, &[]) {
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

/// The pet's spellbook (decision T599): `spells`, read by slot in the
/// pet's bank, its autocast shared with the bar, picked up as a pet action;
/// the book's autocast functions forbidden to the addon (decision T600).
#[test]
fn the_pets_spellbook() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'PET_BAR_UPDATE', 'SPELLS_CHANGED', 'ADDON_ACTION_FORBIDDEN' }) do f:RegisterEvent(e) end
f:SetScript('OnEvent', function(_, e, addon, action)
    seen[#seen + 1] = e == 'ADDON_ACTION_FORBIDDEN' and (addon .. ':' .. action) or e
end)
local Pet = Enum.SpellBookSpellBank.Pet
local without = select('#', C_SpellBook.HasPetSpells()) .. ' ' .. tostring(GetPetIcon())
local wolf = mock.spawnUnit({ name = 'Fang', level = 19, reaction = 'friendly' })
mock.setPet(wolf)
local default = select('#', C_SpellBook.HasPetSpells()) .. ' ' .. tostring(GetPetIcon())
    .. ' ' .. tostring(PetHasSpellbook())
local summoned = table.concat(seen, ' ')
seen = {}
mock.setPet(wolf, {
    actions = { [4] = { spell = 17253, name = 'Bite', icon = 132278, autocast = true } },
    spells = {
        { spell = 17253, name = 'Bite', subName = 'Rank 2', icon = 132278, autocast = true },
        { spell = 2649, name = 'Growl', subName = 'Rank 1', icon = 132270, autocast = false },
        { spell = 24604, name = 'Furious Howl', icon = 132203 },
        { spell = 4187, name = 'Great Stamina', icon = 136112, passive = true },
    },
})
local given = table.concat(seen, ' ')
local n, token = C_SpellBook.HasPetSpells()
local info = C_SpellBook.GetSpellBookItemInfo(2, Pet)
local fields = table.concat({ info.name, info.subName, info.spellID, info.actionID, info.itemType,
    info.iconID, tostring(info.isPassive), tostring(info.isOffSpec) }, ' ')
local name, sub = C_SpellBook.GetSpellBookItemName(1, Pet)
local kind, action, spell = C_SpellBook.GetSpellBookItemType(3, Pet)
local others = table.concat({ name, sub, C_SpellBook.GetSpellBookItemTexture(2, Pet),
    tostring(C_SpellBook.IsSpellBookItemPassive(4, Pet)), kind, action, spell,
    C_SpellBook.GetSpellBookItemLink(1, Pet), tostring((C_SpellBook.GetSpellBookItemInfo(5, Pet))),
    tostring(C_SpellBook.IsSpellInSpellBook(2649, Pet)), tostring(GetPetIcon()) }, ' ')
local function auto(i) local a, e = C_SpellBook.GetSpellBookItemAutoCast(i, Pet) return tostring(a) .. '/' .. tostring(e) end
local autos = auto(1) .. ' ' .. auto(2) .. ' ' .. auto(3)
seen = {}
C_SpellBook.ToggleSpellBookItemAutoCast(1, Pet)
C_SpellBook.SetSpellBookItemAutoCastEnabled(2, Pet, true)
local forbidden = table.concat(seen, ' ') .. ' ' .. auto(1) .. ' ' .. auto(2) .. ' ' .. #mock.refusals()
seen = {}
TogglePetAutocast(4)
local toggled = auto(1) .. ' ' .. tostring(select(6, GetPetActionInfo(4)))
TogglePetAutocast(4)
local back = auto(1)
local toggles = table.concat(seen, ' ')
C_SpellBook.PickupSpellBookItem(2, Pet)
local cursor = table.concat({ GetCursorInfo() }, ' ')
ClearCursor()
mock.setPet(wolf, { hunter = false, icon = 136218 })
local demon = select(2, C_SpellBook.HasPetSpells()) .. ' ' .. GetPetIcon()
seen = {}
mock.setPet(nil)
return without, default, summoned, given, n .. ' ' .. token, fields, others, autos, forbidden, toggled, back,
    toggles, cursor, demon, table.concat(seen, ' '), select('#', C_SpellBook.HasPetSpells())",
    );
    assert_eq!(
        r,
        [
            "0 nil",
            "0 nil true",
            "PET_BAR_UPDATE",
            "PET_BAR_UPDATE SPELLS_CHANGED",
            "4 PET",
            "Growl Rank 1 2649 2649 3 132270 false false",
            "Bite Rank 2 132270 true 3 24604 24604 \
             |cff71d5ff|Hspell:17253:0|h[Bite]|h|r nil true 132278",
            "true/true true/false false/false",
            "MyAddon:ToggleSpellBookItemAutoCast() \
             MyAddon:SetSpellBookItemAutoCastEnabled() true/true true/false 2",
            "true/false false",
            "true/true",
            "PET_BAR_UPDATE PET_BAR_UPDATE",
            "petaction 2649 2",
            "DEMON 136218",
            "PET_BAR_UPDATE SPELLS_CHANGED",
            "0",
        ]
    );
}

/// Blizzard's (secure) code switches the pet's spellbook autocast, in
/// combat too: where the spell may autocast, in the pet's bank only, on the
/// bar as well (decision T600).
#[test]
fn secure_code_switches_the_pet_books_autocast() {
    let mut vm = vm();
    run(
        &mut vm,
        "local wolf = mock.spawnUnit({ name = 'Fang', level = 19, reaction = 'friendly' })
mock.setPet(wolf, {
    actions = { [4] = { spell = 17253, name = 'Bite', icon = 132278, autocast = true } },
    spells = {
        { spell = 17253, name = 'Bite', icon = 132278, autocast = true },
        { spell = 2649, name = 'Growl', icon = 132270, autocast = false },
        { spell = 24604, name = 'Furious Howl', icon = 132203 },
    },
})
mock.player:enterCombat()",
    );
    let r = run_secure(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('PET_BAR_UPDATE')
f:RegisterEvent('ADDON_ACTION_FORBIDDEN')
f:SetScript('OnEvent', function(_, e) seen[#seen + 1] = e end)
local Pet = Enum.SpellBookSpellBank.Pet
local function auto(i) local a, e = C_SpellBook.GetSpellBookItemAutoCast(i, Pet) return tostring(a) .. '/' .. tostring(e) end
C_SpellBook.ToggleSpellBookItemAutoCast(1, Pet)
local toggled = auto(1) .. ' ' .. tostring(select(6, GetPetActionInfo(4)))
C_SpellBook.ToggleSpellBookItemAutoCast(3, Pet)
C_SpellBook.ToggleSpellBookItemAutoCast(2, Enum.SpellBookSpellBank.Player)
C_SpellBook.SetSpellBookItemAutoCastEnabled(2, Pet, true)
C_SpellBook.SetSpellBookItemAutoCastEnabled(1, Pet, true)
return toggled, auto(1) .. ' ' .. auto(2) .. ' ' .. auto(3), table.concat(seen, ' ')",
    );
    assert_eq!(
        r,
        [
            "true/false false",
            "true/true true/true false/false",
            "PET_BAR_UPDATE PET_BAR_UPDATE PET_BAR_UPDATE",
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
