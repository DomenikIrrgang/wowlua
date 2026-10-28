//! `unit:update(data)`: a unit's data changed in place, each change told as
//! the client tells it (decision T497).

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "@Test.lua").expect("compiles");
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

/// The events each change tells, for each of the unit's tokens.
const LISTEN: &str = "local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'UNIT_LEVEL', 'UNIT_NAME_UPDATE', 'UNIT_HEALTH', 'UNIT_MAXHEALTH',
    'UNIT_POWER_UPDATE', 'UNIT_MAXPOWER', 'UNIT_DISPLAYPOWER', 'UNIT_FACTION',
    'UNIT_CLASSIFICATION_CHANGED', 'UNIT_PORTRAIT_UPDATE', 'UNIT_MODEL_CHANGED',
    'UNIT_FLAGS', 'UNIT_DIED', 'PLAYER_LEVEL_CHANGED' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, unit) seen[#seen + 1] = e .. ':' .. tostring(unit) end)
";

#[test]
fn a_unit_s_fields_change_in_place_and_are_told() {
    let mut vm = vm();
    let src = format!(
        "local ogre = mock.spawnUnit({{ name = 'Ogre', level = 60, reaction = 'hostile', health = 100 }})
mock.player:target(ogre)
local guid = UnitGUID('target')
{LISTEN}
ogre:update({{ level = 62, name = 'Training Ogre', classification = 'elite', reaction = 'neutral',
    healthMax = 200, health = 150, powerType = 1, distance = 6 }})
local _, _, classID = UnitClass('target')
return table.concat(seen, ' '), UnitLevel('target'), UnitName('target'), UnitClassification('target'),
    UnitReaction('player', 'target'), UnitPowerType('target'), UnitGUID('target') == guid"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r[0],
        "UNIT_LEVEL:target UNIT_NAME_UPDATE:target UNIT_MAXHEALTH:target UNIT_HEALTH:target \
         UNIT_DISPLAYPOWER:target UNIT_FACTION:target UNIT_CLASSIFICATION_CHANGED:target"
    );
    // (a hostile unit's health is secret on camelot: the player's below)
    assert_eq!(r[1..], ["62", "Training Ogre", "elite", "4", "1", "true"]);
}

#[test]
fn fields_it_does_not_name_stay_and_nothing_is_told_for_no_change() {
    let mut vm = vm();
    let src = format!(
        "local mage = mock.spawnUnit({{ name = 'Mage', player = true, class = 'MAGE', race = 'Gnome', level = 30 }})
mock.player:target(mage)
{LISTEN}
mage:update({{ level = 30 }})
local quiet = #seen
mage:update({{ class = 'WARLOCK' }})
local _, class = UnitClass('target')
return quiet, table.concat(seen, ' '), class, UnitLevel('target'), (UnitRace('target'))"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r,
        [
            "0",
            "UNIT_PORTRAIT_UPDATE:target UNIT_MODEL_CHANGED:target",
            "WARLOCK",
            "30",
            "Gnome"
        ]
    );
}

#[test]
fn dead_and_alive_again() {
    let mut vm = vm();
    let src = format!(
        "local wolf = mock.spawnUnit({{ name = 'Wolf', health = 80 }})
wolf:showNameplate()
{LISTEN}
wolf:update({{ dead = true }})
local dead = UnitIsDead('nameplate1')
local died = table.concat(seen, ' ')
seen = {{}}
wolf:update({{ dead = false }})
return died, tostring(dead), table.concat(seen, ' '), UnitIsDead('nameplate1')"
    );
    let r = run(&mut vm, &src);
    assert_eq!(r[0].split(' ').count(), 2, "{}", r[0]);
    assert!(r[0].starts_with("UNIT_DIED:Creature-"), "{}", r[0]);
    assert!(r[0].ends_with("UNIT_HEALTH:nameplate1"), "{}", r[0]);
    assert_eq!(r[1], "true");
    assert_eq!(r[2], "UNIT_HEALTH:nameplate1 UNIT_FLAGS:nameplate1");
    assert_eq!(r[3], "false");
}

#[test]
fn the_player_s_level_is_told_as_set_player_tells_it_and_its_guid_stays() {
    let mut vm = vm();
    let src = format!(
        "local guid = UnitGUID('player')
{LISTEN}
mock.player:update({{ level = 21, healthMax = 500, health = 320 }})
local ok, err = pcall(function() mock.player:update({{ guid = 'Player-1-2' }}) end)
local ok2, err2 = pcall(function() mock.player:update({{ dead = true }}) end)
return table.concat(seen, ' '), UnitLevel('player'), UnitGUID('player') == guid, err, err2,
    UnitHealth('player'), UnitHealthMax('player')"
    );
    let r = run(&mut vm, &src);
    assert!(r[0].starts_with("PLAYER_LEVEL_CHANGED:"), "{}", r[0]);
    assert!(
        r[0].contains("UNIT_LEVEL:player UNIT_MAXHEALTH:player UNIT_HEALTH:player"),
        "{}",
        r[0]
    );
    // (the player's health is secret on camelot: its maximum is not)
    assert_eq!(r[6], "500");
    assert_eq!(r[1..3], ["21", "true"]);
    assert!(r[3].contains("keeps its GUID"), "{}", r[3]);
    assert!(r[4].contains("player:die()"), "{}", r[4]);
}
