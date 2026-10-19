//! The group finder (`lfg.rs`): Forever's recorded categories and
//! activities, who may search and list (SuuUI's `/suu probe lfg`), the
//! searches' answers, and Blizzard's vanilla-style group finder on them.

use std::path::{Path, PathBuf};

use wowlua_engine::lfg::{Member, SearchResult};
use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn text(vm: &Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        Value::Table(_) => "table".into(),
        other => format!("{other:?}"),
    }
}

/// Runs `f` as its own execution (from C: the taint it ends with is not
/// the next one's).
fn call(vm: &mut Vm, f: Result<Value, String>) -> Vec<String> {
    let f = f.expect("compiles");
    match vm.call_secure(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// Runs an addon's code (tainted).
fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon");
    call(vm, f)
}

/// Runs Blizzard's (untainted) code.
fn secure(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "=blizzard");
    call(vm, f)
}

/// The events an addon sees, kept in `Seen` as `EVENT(args)`.
const WATCH: &str = "Seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'ADDON_ACTION_BLOCKED', 'ADDON_ACTION_FORBIDDEN', 'LFG_LIST_SEARCH_RESULTS_RECEIVED',
    'LFG_LIST_SEARCH_FAILED', 'LFG_LIST_ACTIVE_ENTRY_UPDATE' }) do f:RegisterEvent(e) end
f:SetScript('OnEvent', function(_, e, ...)
    local args = {}
    for i = 1, select('#', ...) do args[i] = tostring((select(i, ...))) end
    Seen[#Seen + 1] = e .. '(' .. table.concat(args, ', ') .. ')'
end)
function SeenText() local s = table.concat(Seen, ' ') Seen = {} return s end";

/// A button whose click runs `body`.
fn button(body: &str) -> String {
    format!(
        "local b = CreateFrame('Button', nil, UIParent)
b:SetSize(100, 20)
b:SetPoint('CENTER')
b:SetScript('OnClick', function() {body} end)
TheButton = b"
    )
}

#[test]
fn the_recorded_categories_and_activities() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local cats = C_LFGList.GetAvailableCategories()
local custom = C_LFGList.GetLfgCategoryInfo(120)
local groups = C_LFGList.GetAvailableActivityGroups(116)
local name, order = C_LFGList.GetActivityGroupInfo(296)
local dungeons = C_LFGList.GetAvailableActivities(2, 285)
local zones = C_LFGList.GetAvailableActivities(116)
local kalimdor = C_LFGList.GetAvailableActivities(116, 296)
local wc = C_LFGList.GetActivityInfoTable(796)
return table.concat(cats, ','), custom.name, tostring(custom.autoChooseActivity), tostring(custom.searchPromptOverride),
    table.concat(groups, ','), name, order, #C_LFGList.GetAvailableActivityGroups(120),
    #dungeons, dungeons[1], #zones, table.concat(zones, ',', 1, 7), #kalimdor, kalimdor[18],
    table.concat(C_LFGList.GetAvailableActivities(120), ','),
    wc.fullName, wc.minLevelSuggestion, wc.maxLevelSuggestion, wc.mapID, wc.filters, wc.displayType,
    tostring(wc.useDungeonRoleExpectations), C_LFGList.GetActivityFullName(1984),
    select('#', C_LFGList.GetActivityInfoTable(1)), select('#', C_LFGList.GetLfgCategoryInfo(3))",
    );
    assert_eq!(
        r,
        [
            "2,116,120",
            "Custom",
            "true",
            "nil",
            "296,295",
            "Kalimdor",
            "0",
            "0",
            "29",
            "796",
            "40",
            "1989,872,871,870,869,868,874",
            "18",
            "1991",
            "1064",
            "Wailing Caverns",
            "15",
            "24",
            "43",
            "166",
            "1",
            "true",
            "Hall of Thanes",
            "0",
            "0"
        ]
    );
}

#[test]
fn unrecorded_fields_are_estimated_from_their_neighbours() {
    let mut vm = vm();
    // Every activity has every field.
    let r = run(
        &mut vm,
        "local incomplete = {}
for _, cat in ipairs(C_LFGList.GetAvailableCategories()) do
    for _, id in ipairs(C_LFGList.GetAvailableActivities(cat)) do
        local n = 0
        for _ in pairs(C_LFGList.GetActivityInfoTable(id)) do n = n + 1 end
        if n ~= 26 then incomplete[#incomplete + 1] = id end
    end
end
local brd, durotar, thanes = C_LFGList.GetActivityInfoTable(811), C_LFGList.GetActivityInfoTable(874),
    C_LFGList.GetActivityInfoTable(1984)
return #incomplete, brd.shortName, brd.mapID, brd.minLevelSuggestion, brd.filters, tostring(brd.isNormalActivity),
    durotar.minLevelSuggestion, durotar.displayType, durotar.mapID, thanes.mapID, thanes.minLevelSuggestion",
    );
    // Blackrock Depths (top 60) as Scholomance (58-60); Durotar (top 14) as
    // Tirisfal Glades (1-14); the Hall of Thanes (top 20) as Ragefire Chasm
    // (13-18).
    assert_eq!(
        r,
        [
            "0",
            "Blackrock Depths",
            "230",
            "58",
            "165",
            "true",
            "1",
            "4",
            "0",
            "0",
            "15"
        ]
    );
    assert!(wowlua_engine::lfg::estimated_fields(&vm, 796.0).is_empty());
    assert!(wowlua_engine::lfg::estimated_fields(&vm, 811.0).contains(&"mapID"));
}

#[test]
fn the_language_filters() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local avail = C_LFGList.GetAvailableLanguageSearchFilter()
local default = C_LFGList.GetDefaultLanguageSearchFilter()
local before = next(C_LFGList.GetLanguageSearchFilter())
C_LFGList.SaveLanguageSearchFilter({ deDE = true, enUS = true, frFR = false })
local after = C_LFGList.GetLanguageSearchFilter()
return #avail, avail[1], avail[11], tostring(default.enUS), tostring(next(default, 'enUS')), tostring(before),
    tostring(after.deDE), tostring(after.enUS), tostring(after.frFR)",
    );
    assert_eq!(
        r,
        [
            "11", "enUS", "itIT", "true", "nil", "nil", "true", "true", "nil"
        ]
    );
}

#[test]
fn nothing_listed_or_found_at_first() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local count, list = C_LFGList.GetFilteredSearchResults()
return select('#', C_LFGList.GetActiveEntryInfo()), tostring(C_LFGList.HasActiveEntryInfo()), count, #list,
    select('#', C_LFGList.GetSearchResultInfo(1)), tostring(C_LFGList.HasSearchResultInfo(1))",
    );
    assert_eq!(r, ["0", "false", "0", "0", "0", "false"]);
}

#[test]
fn an_addon_searches_from_a_click_not_from_a_timer() {
    let mut vm = vm();
    run(&mut vm, WATCH);
    // From a timer (recorded): blocked, nothing comes back.
    let r = run(
        &mut vm,
        "C_Timer.After(2, function() Called = pcall(C_LFGList.Search, 2) end)
mock.advanceTime(5)
return tostring(Called), SeenText()",
    );
    assert_eq!(r, ["true", "ADDON_ACTION_BLOCKED(MyAddon, UNKNOWN())"]);
    // Plain code (no click behind it) neither.
    let r = run(
        &mut vm,
        "C_LFGList.Search(2) mock.advanceTime(5) return SeenText()",
    );
    assert_eq!(r, ["ADDON_ACTION_BLOCKED(MyAddon, Search())"]);
    // From a click (recorded): the answer a round trip later.
    run(
        &mut vm,
        &button("pcall(C_LFGList.Search, 2, 0, 0, nil, false, nil, nil)"),
    );
    let r = run(
        &mut vm,
        "mock.click(TheButton)
local now = SeenText()
mock.advanceTime(1)
local count, list = C_LFGList.GetSearchResults()
return now, SeenText(), count, #list",
    );
    assert_eq!(r, ["", "LFG_LIST_SEARCH_RESULTS_RECEIVED()", "0", "0"]);
}

#[test]
fn an_addon_searches_from_a_key_or_a_slash_command() {
    let mut vm = vm();
    run(&mut vm, WATCH);
    let r = run(
        &mut vm,
        "SLASH_FINDER1 = '/finder'
SlashCmdList.FINDER = function(msg) C_LFGList.Search(tonumber(msg)) end
mock.slash('/finder 116')
mock.advanceTime(1)
local slash = SeenText()
local k = CreateFrame('Frame', nil, UIParent)
k:SetAllPoints()
k:EnableKeyboard(true)
k:SetScript('OnKeyDown', function(self, key) if key == 'I' then C_LFGList.Search(2) end end)
mock.pressKey('I')
mock.advanceTime(1)
return slash, SeenText()",
    );
    assert_eq!(
        r,
        [
            "LFG_LIST_SEARCH_RESULTS_RECEIVED()",
            "LFG_LIST_SEARCH_RESULTS_RECEIVED()"
        ]
    );
}

#[test]
fn an_addon_may_not_create_or_update_a_listing() {
    let mut vm = vm();
    run(&mut vm, WATCH);
    run(
        &mut vm,
        &button(
            "Created = C_LFGList.CreateListing({ activityIDs = { 799 } })
local ok, updated = pcall(C_LFGList.UpdateListing, { activityIDs = { 799 } })
Updated = updated",
        ),
    );
    let r = run(
        &mut vm,
        "mock.click(TheButton)
mock.advanceTime(2)
return tostring(Created), tostring(Updated), SeenText(), tostring(C_LFGList.HasActiveEntryInfo())",
    );
    assert_eq!(
        r,
        [
            "false",
            "false",
            "ADDON_ACTION_BLOCKED(MyAddon, CreateListing()) ADDON_ACTION_BLOCKED(MyAddon, UNKNOWN())",
            "false"
        ]
    );
}

#[test]
fn blizzards_code_lists_and_an_addons_click_delists() {
    let mut vm = vm();
    run(&mut vm, WATCH);
    let r = secure(
        &mut vm,
        "return tostring(C_LFGList.CreateListing({ activityIDs = { 799 }, newPlayerFriendly = false }))",
    );
    assert_eq!(r, ["true"]);
    // Listed a round trip later, its time running down from thirty
    // minutes (recorded: 1794 left).
    let r = run(
        &mut vm,
        "local before = tostring(C_LFGList.HasActiveEntryInfo())
mock.advanceTime(6)
local e = C_LFGList.GetActiveEntryInfo()
return before, SeenText(), tostring(C_LFGList.HasActiveEntryInfo()), table.concat(e.activityIDs, ','), e.duration,
    e.comment, e.name, e.voiceChat, e.voiceMode, e.generalPlaystyle, tostring(e.autoAccept), tostring(e.privateGroup),
    tostring(e.censored), tostring(e.isCrossFactionListing), tostring(e.newPlayerFriendly), e.requiredItemLevel,
    e.requiredHonorLevel, tostring(e.questID)",
    );
    assert_eq!(
        r,
        [
            "false",
            "LFG_LIST_ACTIVE_ENTRY_UPDATE(true)",
            "true",
            "799",
            "1794",
            "",
            "",
            "",
            "0",
            "0",
            "false",
            "false",
            "false",
            "false",
            "false",
            "0",
            "0",
            "nil"
        ]
    );
    // Blizzard's update, then the addon's click delists (recorded).
    secure(
        &mut vm,
        "C_LFGList.UpdateListing({ activityIDs = { 796, 799 }, newPlayerFriendly = true })",
    );
    run(&mut vm, &button("C_LFGList.RemoveListing()"));
    let r = run(
        &mut vm,
        "mock.advanceTime(1)
local e = C_LFGList.GetActiveEntryInfo()
local updated = SeenText() .. ' ' .. table.concat(e.activityIDs, ',') .. ' ' .. tostring(e.newPlayerFriendly)
mock.click(TheButton)
mock.advanceTime(2)
return updated, SeenText(), tostring(C_LFGList.HasActiveEntryInfo()), select('#', C_LFGList.GetActiveEntryInfo())",
    );
    assert_eq!(
        r,
        [
            "LFG_LIST_ACTIVE_ENTRY_UPDATE() 796,799 true",
            "LFG_LIST_ACTIVE_ENTRY_UPDATE()",
            "false",
            "0"
        ]
    );
}

fn member(name: &str, class: &str, role: &str) -> Member {
    Member {
        name: name.into(),
        class_name: class.into(),
        class_filename: class.to_uppercase(),
        level: 30.0,
        area_name: "The Barrens".into(),
        assigned_role: role.into(),
        tank: role == "TANK",
        healer: role == "HEALER",
        dps: role == "DAMAGER",
    }
}

#[test]
fn a_search_finds_the_servers_listings() {
    let mut vm = vm();
    let solo = wowlua_engine::lfg::add_listing(
        &mut vm,
        SearchResult {
            activity_ids: vec![1983.0],
            age: 73.0,
            general_playstyle: 2.0,
            members: vec![member("Player One", "Hunter", "DAMAGER")],
            ..SearchResult::default()
        },
    );
    let group = wowlua_engine::lfg::add_listing(
        &mut vm,
        SearchResult {
            activity_ids: vec![800.0, 799.0],
            comment: "LF healer".into(),
            members: vec![
                member("Otherplayer Lastname", "Warrior", "TANK"),
                member("Player Two", "Mage", "DAMAGER"),
                member("Player Three", "Mage", "NONE"),
            ],
            ..SearchResult::default()
        },
    );
    let _zone = wowlua_engine::lfg::add_listing(
        &mut vm,
        SearchResult {
            activity_ids: vec![876.0],
            members: vec![member("Player Four", "Druid", "HEALER")],
            ..SearchResult::default()
        },
    );
    assert_eq!((solo, group), (1.0, 2.0));
    run(
        &mut vm,
        &button("C_LFGList.Search(Category, 0, 0, nil, false, nil, Activities)"),
    );
    let r = run(
        &mut vm,
        "local before = select('#', C_LFGList.GetSearchResultInfo(1))
Category = 2
mock.click(TheButton)
mock.advanceTime(1)
local count, list = C_LFGList.GetSearchResults()
local info = C_LFGList.GetSearchResultInfo(1)
local counts = C_LFGList.GetSearchResultMemberCounts(1)
local leader = C_LFGList.GetSearchResultLeaderInfo(2)
local third = C_LFGList.GetSearchResultPlayerInfo(2, 3)
local groupCounts = C_LFGList.GetSearchResultMemberCounts(2)
return before, count, table.concat(list, ','), info.searchResultID, table.concat(info.activityIDs, ','), info.age,
    info.leaderName, info.numMembers, info.generalPlaystyle, tostring(info.isDelisted), tostring(info.hasSelf),
    info.partyGUID, #info.leaderDungeonScoreInfo, info.leaderDungeonScoreInfo[1].mapName, info.comment,
    counts.DAMAGER, counts.DAMAGER_REMAINING, counts.TANK_REMAINING, counts.HEALER_REMAINING, counts.HUNTER,
    counts.classesByRole.DAMAGER.HUNTER, counts.NOROLE,
    leader.name, leader.classFilename, tostring(leader.isLeader), tostring(leader.lfgRoles.tank),
    third.assignedRole, tostring(third.isLeader),
    groupCounts.TANK, groupCounts.NOROLE, groupCounts.MAGE, groupCounts.TANK_REMAINING, groupCounts.DAMAGER_REMAINING,
    tostring(C_LFGList.HasSearchResultInfo(3)), select('#', C_LFGList.GetSearchResultPlayerInfo(2, 4))",
    );
    assert_eq!(
        r,
        [
            "0",
            "2",
            "1,2",
            "1",
            "1983",
            "73",
            "Player One",
            "1",
            "2",
            "false",
            "false",
            "Party-0000-1-000000000001",
            "1",
            "",
            "",
            "1",
            "2",
            "1",
            "1",
            "1",
            "1",
            "0",
            "Otherplayer Lastname",
            "WARRIOR",
            "true",
            "true",
            "NONE",
            "false",
            "1",
            "1",
            "2",
            "0",
            "2",
            "false",
            "0"
        ]
    );
    // The activities asked for; another category; the results cleared.
    let r = run(
        &mut vm,
        "Activities = { 799 }
mock.click(TheButton)
mock.advanceTime(1)
local _, list = C_LFGList.GetSearchResults()
local first = table.concat(list, ',')
Category, Activities = 116, nil
mock.click(TheButton)
mock.advanceTime(1)
local _, zones = C_LFGList.GetFilteredSearchResults()
C_LFGList.ClearSearchResults()
return first, table.concat(zones, ','), (C_LFGList.GetSearchResults())",
    );
    assert_eq!(r, ["2", "3", "0"]);
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Blizzard's group finder on the engine's: opened with its key, the
/// browse tab's search (nothing found, then a listing found and shown),
/// a listing posted from the listing tab and delisted, without an error.
#[test]
fn blizzards_group_finder_searches_and_lists() {
    let interface = repo().join("target/types-src/camelot/Interface");
    if !interface.exists() {
        eprintln!("skipped: camelot's UI code is not on disk");
        return;
    }
    let xtask = std::fs::read_to_string(repo().join("xtask/src/types.rs")).unwrap_or_default();
    let commit = xtask
        .split("\"camelot\",")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(3))
        .unwrap_or("");
    let strings = repo().join(format!("target/types-src/strings/{commit}.lua"));
    let mut vm = vm();
    let report = wowlua_engine::ui::load(
        &mut vm,
        &interface,
        strings.exists().then_some(Path::new(&strings)),
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    wowlua_engine::login::login(&mut vm, "MyAddon", false, false);
    wowlua_engine::clock::advance_frames(&mut vm, 30, 1000.0 / 60.0);
    let errors_before = wowlua_engine::engine(&vm).world.ui.errors.len();
    wowlua_engine::lfg::add_listing(
        &mut vm,
        SearchResult {
            activity_ids: vec![797.0],
            age: 30.0,
            members: vec![member("Player One", "Hunter", "DAMAGER")],
            ..SearchResult::default()
        },
    );
    run(&mut vm, WATCH);
    let r = run(
        &mut vm,
        "mock.pressBinding('TOGGLEGROUPFINDER')
mock.advanceFrames(2)
local shown = tostring(LFGParentFrame:IsShown())
mock.click(LFGParentFrame.BrowsingTab)
mock.advanceFrames(2)
local browsing = tostring(LFGBrowseFrame:IsShown())
-- Dungeons chosen (the menu's choice searches too): the level 60
-- character's ten.
LFGBrowseFrame.CategoryDropdown:SetValue(2)
mock.click(LFGBrowseFrameRefreshButton)
local spinning = tostring(LFGBrowseFrame.searching)
mock.advanceTime(1)
mock.advanceFrames(2)
return shown, browsing, spinning, tostring(LFGBrowseFrame.searching), LFGBrowseFrame.totalResults, SeenText(),
    #LFGUtil_GetFilteredActivities(2)",
    );
    let errors = wowlua_engine::engine(&vm).world.ui.errors[errors_before..].to_vec();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(
        r,
        [
            "true",
            "true",
            "true",
            "false",
            "1",
            "LFG_LIST_SEARCH_RESULTS_RECEIVED()",
            "10"
        ]
    );
    // The listing tab: Dungeons, Scholomance, posted (Blizzard's code
    // creates it; its answer searches the listing's activities).
    let r = secure(
        &mut vm,
        "LFGParentFrameTab1_OnClick()
LFGListingFrame:SetCategorySelection(2)
LFGListingFrame:SetActivity(797, true, true, true)
LFGListingFrame:SetDirty(true)
local selected = LFGListingFrame.selectedActivities
LFGListingFrame:CreateOrUpdateListing()
return selected",
    );
    assert_eq!(r, ["1"]);
    let r = run(
        &mut vm,
        "mock.advanceTime(1)
mock.advanceFrames(2)
mock.advanceTime(1)
mock.advanceFrames(2)
local e = C_LFGList.GetActiveEntryInfo()
return table.concat(e.activityIDs, ','), SeenText(), tostring(LFGBrowseFrame:IsShown())",
    );
    let errors = wowlua_engine::engine(&vm).world.ui.errors[errors_before..].to_vec();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(
        r,
        [
            "797",
            "LFG_LIST_ACTIVE_ENTRY_UPDATE(true) LFG_LIST_SEARCH_RESULTS_RECEIVED()",
            "true"
        ]
    );
    // Delisted by the listing panel (its own entry's delist button is a
    // search result's: the player's listing is not among them here).
    secure(&mut vm, "LFGListingFrame:RemoveListing()");
    let r = run(
        &mut vm,
        "mock.advanceTime(1)
mock.advanceFrames(2)
return tostring(C_LFGList.HasActiveEntryInfo()), SeenText()",
    );
    let errors = wowlua_engine::engine(&vm).world.ui.errors[errors_before..].to_vec();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(r, ["false", "LFG_LIST_ACTIVE_ENTRY_UPDATE()"]);
}
