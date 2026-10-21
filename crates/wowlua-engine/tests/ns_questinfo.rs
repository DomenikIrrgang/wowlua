//! The quest log's rewards, super-tracking and the user's map pin
//! (`questinfo.rs`).

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

// `GetNumQuestLogChoices` with its quest: the client refuses a call without
// one ("Usage: GetNumQuestLogChoices(questID, [includeCurrencies])",
// `/stdtest engine fullprobe`).
#[test]
fn a_defined_quests_rewards_in_the_log_functions() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineItem(80, { name = 'Soft Fur-lined Shoes', quality = 1, icon = 132543 })
mock.defineQuest(33, { title = 'Wolves', xp = 170, money = 25, choices = { 80, 6070 },
    rewards = { { id = 80, count = 2 } } })
C_QuestLog.SetSelectedQuest(33)
local xp, base = GetQuestLogRewardXP()
local name, _, count, _, usable, id = GetQuestLogRewardInfo(1, 33)
return xp, base, GetQuestLogRewardMoney(33), GetNumQuestLogChoices(33), name, count, usable, id,
    C_QuestLog.GetSelectedQuest(), C_QuestLog.IsOnQuest(33), select('#', GetQuestLink(33))",
    );
    assert_eq!(
        r,
        [
            "170",
            "170",
            "25",
            "2",
            "Soft Fur-lined Shoes",
            "2",
            "true",
            "80",
            "33",
            "false",
            // Not in the log: no link, nothing (the full probe).
            "0"
        ]
    );
}

/// The pin cleared: its update, then the super-tracking's (recorded,
/// scenario mapnav).
#[test]
fn the_user_waypoint_set_tracked_linked_and_cleared() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('USER_WAYPOINT_UPDATED')
f:RegisterEvent('SUPER_TRACKING_CHANGED')
f:SetScript('OnEvent', function(_, e) seen[#seen + 1] = e end)
local set = C_Map.SetUserWaypoint({ uiMapID = 1412, position = { x = 0.5, y = 0.25 } })
C_SuperTrack.SetSuperTrackedUserWaypoint(true)
local tracking = C_SuperTrack.IsSuperTrackingUserWaypoint()
local p = C_Map.GetUserWaypoint()
local link = C_Map.GetUserWaypointHyperlink()
local back = C_Map.GetUserWaypointFromHyperlink(link)
C_Map.ClearUserWaypoint()
return table.concat(seen, ' '), set, tracking, p.uiMapID, p.position.x, link, back.position.y,
    C_Map.HasUserWaypoint(), C_SuperTrack.IsSuperTrackingUserWaypoint(),
    select('#', C_SuperTrack.GetSuperTrackedQuestID())",
    );
    assert_eq!(
        r,
        [
            "USER_WAYPOINT_UPDATED SUPER_TRACKING_CHANGED USER_WAYPOINT_UPDATED SUPER_TRACKING_CHANGED",
            "true",
            "true",
            "1412",
            "0.5",
            "|cffffff00|Hworldmap:1412:5000:2500|h[|A:Waypoint-MapPin-ChatIcon:13:13:0:0|a Map Pin Location]|h|r",
            "0.25",
            "false",
            "false",
            "0"
        ]
    );
}

#[test]
fn a_map_position_names_the_zone_under_it() {
    let mut vm = vm();
    // Mulgore's rectangle on Kalimdor (recorded): 0.396-0.564, 0.511-0.678.
    let r = run(
        &mut vm,
        "local info = C_Map.GetMapInfoAtPosition(1414, 0.48, 0.6)
return info and info.mapID, info and info.name",
    );
    assert_eq!(r, ["1412", "Mulgore"]);
}

/// `C_QuestLog.GetQuestsOnMap`: the log's quests at the places the client
/// recorded (Mulgore's map listed quest 959 for the recording character);
/// a recorded quest the log lacks is not listed; no map, nothing.
#[test]
fn quests_on_a_map_are_the_logs() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local before = #C_QuestLog.GetQuestsOnMap(1412)
local giver = mock.spawnUnit({ name = 'Giver', reaction = 'friendly' })
mock.defineQuest(959, { title = 'Trouble at the Docks', giver = giver })
giver:setDistance(3)
mock.player:interact(giver)
mock.advanceTime(1)
AcceptQuest()
mock.advanceTime(1)
local list = C_QuestLog.GetQuestsOnMap(1412)
return before, C_QuestLog.IsOnQuest(959), #list, list[1] and list[1].questID, select('#', C_QuestLog.GetQuestsOnMap(0))",
    );
    assert_eq!(r, ["0", "true", "1", "959", "0"]);
}
