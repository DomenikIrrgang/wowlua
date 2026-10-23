//! The player's data changed by a test: told by events when asked, and
//! the answers that follow the level.

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
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

const LISTEN: &str = "Log = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'PLAYER_LEVEL_CHANGED', 'UNIT_LEVEL', 'UNIT_NAME_UPDATE', 'UNIT_MAXHEALTH',
    'UNIT_HEALTH', 'UNIT_MAXPOWER', 'UNIT_POWER_UPDATE', 'MAJOR_FACTION_RENOWN_LEVEL_CHANGED' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, ...)
    local parts = { e }
    for i = 1, select('#', ...) do parts[#parts + 1] = tostring((select(i, ...))) end
    Log[#Log + 1] = table.concat(parts, ' ')
end)";

/// `setPlayer` fires nothing by default; with `{ events = true }`, what
/// changed, as the client tells it.
#[test]
fn set_player_tells_its_changes_when_asked() {
    let mut vm = vm();
    run(&mut vm, LISTEN);
    let data = "{ name = 'Player', class = 'WARRIOR', race = 'Human', level = %L, health = 60, healthMax = 60, power = 0, powerMax = 100, powerType = 1 }";
    let quiet = format!(
        "mock.setPlayer({}) return #Log, UnitLevel('player')",
        data.replace("%L", "1")
    );
    assert_eq!(run(&mut vm, &quiet), ["0", "1"]);
    let told = format!(
        "mock.setPlayer({}, {{ events = true }}) return table.concat(Log, ';'), UnitLevel('player')",
        data.replace("%L", "12")
    );
    assert_eq!(
        run(&mut vm, &told),
        ["PLAYER_LEVEL_CHANGED 1 12 true;UNIT_LEVEL player", "12"]
    );
    let more = "Log = {} mock.setPlayer({ name = 'Other', class = 'WARRIOR', race = 'Human', level = 12, health = 50, healthMax = 80, power = 10, powerMax = 100, powerType = 1 }, { events = true })
return table.concat(Log, ';')";
    assert_eq!(
        run(&mut vm, more),
        [
            "UNIT_NAME_UPDATE player;UNIT_MAXHEALTH player;UNIT_HEALTH player;UNIT_POWER_UPDATE player RAGE"
        ]
    );
}

/// `setRenown` fires nothing by default; with `{ events = true }` a change
/// is `MAJOR_FACTION_RENOWN_LEVEL_CHANGED(faction, new, old)`.
#[test]
fn set_renown_tells_its_change_when_asked() {
    let mut vm = vm();
    run(&mut vm, LISTEN);
    let r = run(
        &mut vm,
        "mock.setRenown(2802, 1) local quiet = #Log
mock.setRenown(2802, 3, { events = true })
mock.setRenown(2802, 3, { events = true })
return quiet, table.concat(Log, ';'), C_MajorFactions.GetCurrentRenownLevel(2802)",
    );
    assert_eq!(r, ["0", "MAJOR_FACTION_RENOWN_LEVEL_CHANGED 2802 3 1", "3"]);
}

/// A talent point is earned from the client's first talent level (10),
/// lowered by the legacy adventure tree's "Talented" ranks: the default
/// character's 5 open it at level 5, whatever the recording's level was.
#[test]
fn talent_points_are_earned_from_the_talent_level() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local l = Constants.LegacyConsts
local config = C_Traits.GetConfigIDByTreeID(l.LEGACY_TREE_ADVENTURE_ID)
return Constants.LevelConstsExposed.MIN_TALENT_LEVEL,
    C_Traits.GetNodeInfo(config, l.LEGACY_TREE_ADVENTURE_TALENTED_NODE_ID).activeRank",
    );
    assert_eq!(r, ["10", "5"]);
    let at = |vm: &mut Vm, level: u32| {
        run(
            vm,
            &format!(
                "mock.setPlayer({{ name = 'Player', class = 'WARRIOR', race = 'Human', level = {level} }})
return tostring(C_SpecializationInfo.HasPlayerEarnedATalentPoint())"
            ),
        )
    };
    assert_eq!(at(&mut vm, 1), ["false"]);
    assert_eq!(at(&mut vm, 4), ["false"]);
    assert_eq!(at(&mut vm, 5), ["true"]);
    assert_eq!(at(&mut vm, 60), ["true"]);
}

/// The default character's talent configs name the tree the talent model
/// answers for: its active and combat configs (16273023, as the default
/// character's recording has them) are one, and the model has it (its
/// recorded warrior's tree under another number).
#[test]
fn the_players_configs_are_the_talent_models() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.player:setStats({ combatConfig = { [1] = 16273023 }, activeSpecGroup = 1 })
local active = C_ClassTalents.GetActiveConfigID()
local combat = C_SpecializationInfo.GetCombatConfigIDForSpecGroup(1)
local info = C_Traits.GetConfigInfo(combat)
return tostring(active == combat), tostring(info ~= nil), tostring(info and #info.treeIDs > 0),
    tostring(C_SpecializationInfo.GetCombatConfigIDForSpecGroup(2))",
    );
    assert_eq!(r, ["true", "true", "true", "nil"]);
}

/// The class talents' points follow the level: one a level from the unlock
/// level (5 with the default character's legacy ranks), at most 51; the
/// recorded ranks stay spent.
#[test]
fn talent_points_follow_the_level() {
    let mut vm = vm();
    let at = |vm: &mut Vm, level: u32| {
        run(
            vm,
            &format!(
                "mock.setPlayer({{ name = 'Player', class = 'WARRIOR', race = 'Human', level = {level} }})
mock.player:setStats({{ combatConfig = {{ [1] = 16273023 }}, activeSpecGroup = 1 }})
local config = C_ClassTalents.GetActiveConfigID()
local tree = C_Traits.GetConfigInfo(config).treeIDs[1]
local c = C_Traits.GetTreeCurrencyInfo(config, tree, false)[1]
return c.maxQuantity, c.quantity, c.spent"
            ),
        )
    };
    assert_eq!(at(&mut vm, 1), ["0", "0", "7"]);
    assert_eq!(at(&mut vm, 12), ["8", "1", "7"]);
    assert_eq!(at(&mut vm, 60), ["51", "44", "7"]);
}
