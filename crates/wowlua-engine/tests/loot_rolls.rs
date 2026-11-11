//! Group loot rolls (`lootroll.rs`, decision T629): a roll the test starts
//! (`mock.startLootRoll`), the roll API answering for it, the player's
//! choice (Bind on Pickup asking first), the others' (`mock.lootRollChoice`),
//! the result in chat and the loot history, the time out, and Blizzard's
//! GroupLootFrame and its popup on it.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use std::path::{Path, PathBuf};

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

/// Runs an addon's code (tainted).
fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    match vm.call_secure(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// The roll events and loot lines an addon sees, kept in `Seen`; the items:
/// a Bind on Pickup sword and a Bind on Equip ring.
const SETUP: &str = "Seen = {}
mock.defineItem(19019, { name = 'Thunderfury', quality = 5, icon = 135349, bindType = 1 }, { cached = true })
mock.defineItem(2933, { name = 'Seal of Wrynn', quality = 3, icon = 133345, bindType = 2 }, { cached = true })
local f = CreateFrame('Frame')
for _, e in ipairs({ 'START_LOOT_ROLL', 'CONFIRM_LOOT_ROLL', 'CANCEL_LOOT_ROLL', 'CANCEL_ALL_LOOT_ROLLS',
    'LOOT_ROLLS_COMPLETE', 'CHAT_MSG_LOOT', 'LOOT_HISTORY_UPDATE_DROP', 'LOOT_HISTORY_UPDATE_ENCOUNTER' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, ...)
    local args = {}
    local n = e == 'CHAT_MSG_LOOT' and 1 or select('#', ...)
    for i = 1, n do args[i] = tostring((select(i, ...))) end
    Seen[#Seen + 1] = e .. '(' .. table.concat(args, ', ') .. ')'
end)
function SeenText() local s = table.concat(Seen, ' ') Seen = {} return s end
function Me() return (UnitName('player')) end";

fn setup() -> Vm {
    let mut vm = vm();
    run(&mut vm, SETUP);
    vm
}

const SWORD: &str = "|cnIQ5:|Hitem:19019::::::::60::::::::::|h[Thunderfury]|h|r";
const RING: &str = "|cnIQ3:|Hitem:2933::::::::60::::::::::|h[Seal of Wrynn]|h|r";

#[test]
fn no_roll_answers_nothing() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "return select('#', GetLootRollItemInfo(1)), select('#', GetLootRollItemLink(1)),
    GetLootRollTimeLeft(1), #GetActiveLootRollIDs(), tostring(C_Loot.GetLootRollDuration(1)),
    #C_LootHistory.GetAllEncounterInfos(), tostring(C_LootHistory.GetSortedDropsForEncounter(0))",
    );
    assert_eq!(r, ["0", "0", "0", "0", "nil", "0", "nil"]);
}

#[test]
fn a_roll_starts_and_the_api_answers_for_it() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local id = mock.startLootRoll({ item = 2933, rollTime = 30, group = { 'Thrall', 'Jaina' } })
local info = { GetLootRollItemInfo(id) }
for i = 1, 13 do info[i] = tostring(info[i]) end
local before = GetLootRollTimeLeft(id)
mock.advanceTime(10)
return id, SeenText(), table.concat(info, ' '), GetLootRollItemLink(id), before, GetLootRollTimeLeft(id),
    C_Loot.GetLootRollDuration(id), table.concat(GetActiveLootRollIDs(), ',')",
    );
    assert_eq!(
        r,
        [
            "1",
            "START_LOOT_ROLL(1, 30000, 1) LOOT_HISTORY_UPDATE_ENCOUNTER(0)",
            "133345 Seal of Wrynn 1 3 false true true false 0 0 0 0 false",
            RING,
            "30000",
            "20000",
            "30000",
            "1"
        ]
    );
}

#[test]
fn need_on_bind_on_pickup_asks_first() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local id = mock.startLootRoll({ item = 19019, group = { 'Thrall' } })
SeenText()
RollOnLoot(id, 1)
local asked = SeenText()
ConfirmLootRoll(id, 1)
local confirmed = SeenText()
local active = #GetActiveLootRollIDs()
mock.advanceTime(1)
return asked, confirmed, active, SeenText(), (select(2, GetLootRollItemInfo(id)))",
    );
    assert_eq!(
        r,
        [
            "CONFIRM_LOOT_ROLL(1, 1, )",
            "CANCEL_LOOT_ROLL(1)",
            "0",
            &format!(
                "CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: You have selected Need for: {SWORD}) LOOT_HISTORY_UPDATE_DROP(0, 1)"
            ),
            "Thunderfury"
        ]
    );
}

#[test]
fn pass_and_unbound_items_do_not_ask() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local sword = mock.startLootRoll({ item = 19019, group = { 'Thrall' } })
local ring = mock.startLootRoll({ item = 2933, group = { 'Thrall' } })
SeenText()
RollOnLoot(sword, 0)
RollOnLoot(ring, 2)
local now = SeenText()
-- chosen already: nothing
RollOnLoot(ring, 1)
return now, SeenText()",
    );
    assert_eq!(r, ["CANCEL_LOOT_ROLL(1) CANCEL_LOOT_ROLL(2)", ""]);
}

#[test]
fn a_choice_the_item_does_not_allow_does_nothing() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local id = mock.startLootRoll({ item = 2933, canNeed = false, reasonNeed = 1, group = { 'Thrall' } })
SeenText()
RollOnLoot(id, 1)
RollOnLoot(id, 3)
local need, _, reason = select(6, GetLootRollItemInfo(id)), nil, select(9, GetLootRollItemInfo(id))
return SeenText(), tostring(need), reason",
    );
    assert_eq!(r, ["", "false", "1"]);
}

#[test]
fn the_others_choose_and_the_highest_need_wins() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local id = mock.startLootRoll({ item = 2933, group = { 'Thrall', 'Jaina', 'Anduin' }, rolls = { [Me()] = 40 } })
SeenText()
mock.lootRollChoice(id, 'Jaina', 'greed', 99)
mock.lootRollChoice(id, 'Thrall', 'need', 91)
RollOnLoot(id, 1)
mock.advanceTime(1)
local before = SeenText()
mock.lootRollChoice(id, 'Anduin', 'pass')
return before, SeenText(), select('#', GetLootRollItemInfo(id)), C_Item.GetItemCount(2933)",
    );
    let me = run(&mut vm, "return Me()")[0].clone();
    assert_eq!(
        r[0],
        format!(
            "CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: Jaina has selected Greed for: {RING}) LOOT_HISTORY_UPDATE_DROP(0, 1) \
CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: Thrall has selected Need for: {RING}) LOOT_HISTORY_UPDATE_DROP(0, 1) \
CANCEL_LOOT_ROLL(1) \
CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: You have selected Need for: {RING}) LOOT_HISTORY_UPDATE_DROP(0, 1)"
        )
    );
    assert_eq!(
        r[1],
        format!(
            "CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: Anduin passed on: {RING}) LOOT_HISTORY_UPDATE_DROP(0, 1) \
CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: Need Roll - 91 for {RING} by Thrall) \
CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: Need Roll - 40 for {RING} by {me}) \
CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: Thrall won: {RING}) \
LOOT_HISTORY_UPDATE_DROP(0, 1) LOOT_ROLLS_COMPLETE(1)"
        )
    );
    assert_eq!(r[2..], ["0", "0"]);
}

#[test]
fn the_player_wins_and_gets_the_item() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local id = mock.startLootRoll({ item = 2933, group = { 'Thrall' } })
mock.lootRollChoice(id, 'Thrall', 'greed', 99)
RollOnLoot(id, 1)
mock.advanceTime(1)
local lines = {}
for _, s in ipairs(Seen) do if s:find('^CHAT_MSG_LOOT') then lines[#lines + 1] = s end end
Seen = {}
return #lines, lines[3], lines[4], lines[5], C_Item.GetItemCount(2933)",
    );
    let me = run(&mut vm, "return Me()")[0].clone();
    assert_eq!(r[0], "5", "{r:?}");
    assert!(
        r[1].contains("Need Roll - ") && r[1].ends_with(&format!("for {RING} by {me})")),
        "{r:?}"
    );
    assert_eq!(
        r[2..],
        [
            format!("CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: You won: {RING})"),
            format!("CHAT_MSG_LOOT(You receive loot: {RING})"),
            "1".to_string()
        ]
    );
}

#[test]
fn everyone_passes() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local id = mock.startLootRoll({ item = 2933, group = { 'Thrall' } })
mock.lootRollChoice(id, 'Thrall', 'pass')
RollOnLoot(id, 0)
SeenText()
mock.advanceTime(1)
return SeenText()",
    );
    assert_eq!(
        r,
        [format!(
            "CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: You passed on: {RING}) LOOT_HISTORY_UPDATE_DROP(0, 1) \
CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: Everyone passed on: {RING}) LOOT_HISTORY_UPDATE_DROP(0, 1) LOOT_ROLLS_COMPLETE(1)"
        )]
    );
}

#[test]
fn the_time_out_ends_the_roll() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local id = mock.startLootRoll({ item = 2933, rollTime = 20, group = { 'Thrall', 'Jaina' } })
mock.lootRollChoice(id, 'Thrall', 'greed', 12)
SeenText()
mock.advanceTime(19.9)
local before = SeenText()
mock.advanceTime(0.2)
return before, SeenText(), GetLootRollTimeLeft(id), #GetActiveLootRollIDs()",
    );
    assert_eq!(
        r,
        [
            String::new(),
            format!(
                "CANCEL_LOOT_ROLL(1) CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: Greed Roll - 12 for {RING} by Thrall) \
CHAT_MSG_LOOT(|HlootHistory:0|h[Loot]|h: Thrall won: {RING}) LOOT_HISTORY_UPDATE_DROP(0, 1) LOOT_ROLLS_COMPLETE(1)"
            ),
            "0".to_string(),
            "0".to_string()
        ]
    );
}

#[test]
fn rolls_of_one_loot_complete_together() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local a = mock.startLootRoll({ item = 2933, loot = 7, group = { 'Thrall' } })
local b = mock.startLootRoll({ item = 19019, loot = 7, group = { 'Thrall' } })
mock.lootRollChoice(a, 'Thrall', 'pass')
mock.lootRollChoice(b, 'Thrall', 'pass')
RollOnLoot(a, 0)
RollOnLoot(b, 0)
Seen = {}
mock.advanceTime(1)
local complete = {}
for _, s in ipairs(Seen) do if s:find('^LOOT_ROLLS_COMPLETE') then complete[#complete + 1] = s end end
return table.concat(complete, ' ')",
    );
    assert_eq!(r, ["LOOT_ROLLS_COMPLETE(7)"]);
}

#[test]
fn the_loot_history_keeps_the_drops() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local id = mock.startLootRoll({ item = 2933, encounterID = 1084, encounterName = 'Onyxia',
    group = { 'Thrall', { name = 'Jaina', class = 'MAGE' } }, rolls = { Jaina = 77 } })
mock.lootRollChoice(id, 'Thrall', 'greed')
local pending = C_LootHistory.GetSortedInfoForDrop(1084, 1)
mock.lootRollChoice(id, 'Jaina', 'need')
RollOnLoot(id, 2)
mock.advanceTime(1)
local drop = C_LootHistory.GetSortedInfoForDrop(1084, 1)
local enc = C_LootHistory.GetAllEncounterInfos()[1]
local states = {}
for i, roll in ipairs(drop.rollInfos) do
    states[i] = roll.playerName .. ':' .. roll.state .. ':' .. tostring(roll.roll) .. ':' .. tostring(roll.isWinner)
end
local self = drop.rollInfos[3]
return pending.playerRollState, tostring(pending.winner), drop.playerRollState, drop.winner.playerName,
    drop.winner.playerClass, drop.winner.roll, tostring(drop.allPassed), drop.itemHyperlink,
    table.concat(states, ' '), tostring(self.isSelf), self.playerName == Me() and 'me' or self.playerName,
    enc.encounterID, enc.encounterName, #C_LootHistory.GetSortedDropsForEncounter(1084),
    C_LootHistory.GetInfoForEncounter(1084).encounterName",
    );
    let me = run(&mut vm, "return Me()")[0].clone();
    assert_eq!(
        r,
        [
            "4",
            "nil",
            "3",
            "Jaina",
            "MAGE",
            "77",
            "false",
            RING,
            &format!("Jaina:0:77:true Thrall:3:nil:false {me}:3:nil:false"),
            "true",
            "me",
            "1084",
            "Onyxia",
            "1",
            "Onyxia"
        ]
    );
}

#[test]
fn a_roll_cancelled() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local a = mock.startLootRoll({ item = 2933, group = { 'Thrall' } })
local b = mock.startLootRoll({ item = 19019, group = { 'Thrall' } })
SeenText()
mock.cancelLootRoll(a)
local one = SeenText() .. ' ' .. select('#', GetLootRollItemInfo(a)) .. ' ' .. select('#', GetLootRollItemInfo(b))
mock.cancelLootRoll()
mock.advanceTime(120)
return one, SeenText(), #GetActiveLootRollIDs()",
    );
    assert_eq!(
        r,
        ["CANCEL_LOOT_ROLL(1) 0 13", "CANCEL_ALL_LOOT_ROLLS()", "0"]
    );
}

#[test]
fn the_controls_check_their_arguments() {
    let mut vm = setup();
    let errors = [
        (
            "mock.startLootRoll({})",
            "startLootRoll: an item ID expected",
        ),
        (
            "mock.startLootRoll({ item = 99999 })",
            "item 99999 is unknown",
        ),
        (
            "mock.lootRollChoice(mock.startLootRoll({ item = 2933, group = { 'Thrall' } }), 'Jaina', 'need')",
            "Jaina is not in the roll's group",
        ),
        (
            "mock.lootRollChoice(mock.startLootRoll({ item = 2933, group = { 'Thrall' } }), 'Thrall', 'disenchant')",
            "\"need\", \"greed\" or \"pass\" expected",
        ),
        (
            "mock.lootRollChoice(42, 'Thrall', 'need')",
            "no roll 42 under way",
        ),
    ];
    for (src, want) in errors {
        let f = vm
            .load_as(src.as_bytes(), "@MyAddon/x.lua", "MyAddon")
            .expect("compiles");
        let e = vm.call_secure(f, &[]).expect_err(src);
        let got = text(&vm, e.value);
        assert!(got.contains(want), "{src}: {got}");
    }
}

/// The group, by default the player's.
#[test]
fn the_players_group_rolls_by_default() {
    let mut vm = setup();
    let r = run(
        &mut vm,
        "local thrall = mock.spawnUnit({ name = 'Thrall', class = 'SHAMAN', player = true, reaction = 'friendly' })
mock.addToGroup(thrall)
local id = mock.startLootRoll({ item = 2933 })
mock.lootRollChoice(id, 'Thrall', 'need', 50)
RollOnLoot(id, 0)
mock.advanceTime(1)
local drop = C_LootHistory.GetSortedInfoForDrop(0, 1)
return drop.winner.playerName, drop.winner.playerClass, drop.winner.playerGUID == UnitGUID('party1')",
    );
    assert_eq!(r, ["Thrall", "SHAMAN", "true"]);
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Blizzard's GroupLootFrame on a roll: shown with the item, Need on a
/// Bind on Pickup item asks through its popup, whose OK rolls; the frame
/// goes; the result in chat; no error.
#[test]
fn blizzards_roll_frame_and_its_popup() {
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
    run(&mut vm, SETUP);
    let r = run(
        &mut vm,
        "local id = mock.startLootRoll({ item = 19019, rollTime = 60, group = { 'Thrall' } })
mock.advanceFrames(2)
local frame = GroupLootFrame1
local shown, name, rollID = tostring(frame:IsShown()), frame.Name:GetText(), frame.rollID
mock.click(frame.LootButtonContainer.NeedButton)
mock.advanceFrames(2)
local popup = StaticPopup_FindVisible('CONFIRM_LOOT_ROLL')
local asked = popup and popup:GetText() or 'none'
local still = tostring(frame:IsShown())
mock.click(popup:GetButton1())
mock.advanceFrames(2)
local gone = tostring(frame:IsShown())
local popupGone = tostring(StaticPopup_FindVisible('CONFIRM_LOOT_ROLL') == nil)
mock.lootRollChoice(id, 'Thrall', 'greed')
mock.advanceTime(1)
mock.advanceFrames(2)
local won = 'no'
for _, s in ipairs(Seen) do if s:find('You won') then won = 'yes' end end
return shown, name, rollID, asked, still, gone, popupGone, won, C_Item.GetItemCount(19019)",
    );
    let errors = wowlua_engine::engine(&vm).world.ui.errors[errors_before..].to_vec();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(
        r,
        [
            "true",
            "Thunderfury",
            "1",
            "Looting |cffff8000Thunderfury|r will bind it to you.",
            "true",
            "false",
            "true",
            "yes",
            "1"
        ]
    );
}
