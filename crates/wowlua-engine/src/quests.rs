//! Quests, as the client recorded a quest giver's (`/stdtest engine
//! scenario quest`, a ping near 0.3 s): `defineQuest(id, data)` names a
//! quest, its giver and what it asks; `player:interact(npc)` talks to the
//! giver and a round trip later the dialog opens (`QUEST_DETAIL(0)` for a
//! quest offered, `QUEST_PROGRESS()` for one in the log); the dialog's
//! answers (`GetTitleText`, `GetQuestText`, `GetObjectiveText`,
//! `GetProgressText`, `GetRewardText`, `GetQuestItemInfo`, …).
//!
//! `AcceptQuest()` closes the dialog at once (`QUEST_FINISHED`,
//! `PLAYER_INTERACTION_MANAGER_FRAME_HIDE(4)` and `(0)`, `QUEST_FINISHED`);
//! the server's answer: `UNIT_QUEST_LOG_CHANGED`, "Quest accepted: …",
//! `QUEST_WATCH_LIST_CHANGED(id, true)`, `QUEST_ACCEPTED(id)`, a frame
//! later `QUEST_LOG_UPDATE()` and `SUPER_TRACKING_CHANGED()`, and later the
//! map's `QUEST_POI_UPDATE()`. The log is the character's recorded stats
//! (`C_QuestLog.GetInfo`, `GetQuestObjectives`, …), kept by the engine
//! from then on: a header, then the quest.
//!
//! An item an objective asks for, looted: `QUEST_WATCH_UPDATE(id)` before
//! the item's push, "Tough Wolf Meat: 1/8" (`UI_INFO_MESSAGE(317, …)`)
//! after, "Objective Complete." (314) when the quest is; the log tells the
//! new count with the server's next packet (`UNIT_QUEST_LOG_CHANGED`, a
//! frame later `QUEST_LOG_UPDATE`; till then the old count).
//!
//! `CompleteQuest()` (the progress panel's Continue): the server closes
//! the panel (`QUEST_FINISHED` and the interaction's hides) and opens the
//! rewards (`QUEST_COMPLETE()`). `GetQuestReward(choice)`: the server's
//! four packets, as recorded: the experience; the quest done ("… completed.",
//! "Experience gained: 170.", `QUEST_TURNED_IN(id, xp, money)`, the dialog
//! closed); the reward (`QUEST_LOOT_RECEIVED`, the reputation, the item's
//! push, the items asked for taken, `QUEST_REMOVED(id, false)`); the bags'
//! update with the reward in them.
//!
//! Unverified: more than one quest at a giver (a greeting), a quest ended
//! elsewhere, declining, objectives other than items, money rewards, the
//! packets' spacing beyond this recording's, a log the profile recorded
//! with quests in it (the engine's log takes its place).

use std::collections::BTreeSet;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::stats::{Answer, Entry};
use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// The server's turn-in packets after the first (the experience), apart
/// as recorded: the quest done, the reward, the bags.
const TURNED_IN_AFTER: f64 = 0.05;
const REWARDED_AFTER: f64 = 0.186;
const BAGS_AFTER: f64 = 0.355;
/// The log's packet after an item an objective asks for (recorded 0.06 to
/// 0.16 s; one value).
const LOG_AFTER: f64 = 0.08;
/// The map's points of interest after a quest is accepted (recorded).
const POI_AFTER: f64 = 0.477;
/// How near the player must be to talk to an NPC, in yards (unverified:
/// 9.9 was too far).
const INTERACT_RANGE: f64 = 5.0;
/// The player's interaction type of a quest giver
/// (`Enum.PlayerInteractionType.QuestGiver`).
const QUEST_GIVER: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Panel {
    Detail,
    Progress,
    Reward,
}

#[derive(Clone, Copy, Debug)]
pub struct Dialog {
    pub npc: UnitId,
    pub quest: i64,
    pub panel: Panel,
}

/// A quest in the log: the server's count for each objective; whether
/// the map shows it, and its point of interest near.
#[derive(Clone, Debug)]
pub struct Logged {
    pub id: i64,
    pub fulfilled: Vec<f64>,
    pub on_map: bool,
    pub local_poi: bool,
}

#[derive(Default)]
pub struct Quests {
    /// The defined quests: ID → the test's table.
    pub defined: std::collections::BTreeMap<i64, TableRef>,
    pub dialog: Option<Dialog>,
    pub log: Vec<Logged>,
    pub completed: BTreeSet<i64>,
    /// Whether the engine keeps the log (once a quest was accepted).
    pub kept: bool,
}

/// An objective, as defined: `{ item = 750, required = 8, text = "Tough
/// Wolf Meat", type = "item", objectiveType = 1 }`.
#[derive(Clone, Debug)]
struct Objective {
    item: Option<i64>,
    required: f64,
    text: String,
    kind: String,
    objective_type: f64,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn text_of(vm: &Vm, v: Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn text(vm: &mut Vm, t: TableRef, key: &str) -> String {
    let v = field(vm, t, key);
    text_of(vm, v).unwrap_or_default()
}

fn num(vm: &mut Vm, t: TableRef, key: &str) -> Option<f64> {
    match field(vm, t, key) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn flag(vm: &mut Vm, t: TableRef, key: &str, default: bool) -> bool {
    match field(vm, t, key) {
        Value::Bool(b) => b,
        _ => default,
    }
}

fn data(vm: &Vm, id: i64) -> Option<TableRef> {
    engine(vm).world.quests.defined.get(&id).copied()
}

/// A list of items (`{ 80, 6070 }` or `{ { id = 80, count = 1 } }`).
fn items(vm: &mut Vm, t: TableRef, key: &str) -> Vec<(i64, f64)> {
    let Value::Table(list) = field(vm, t, key) else {
        return vec![];
    };
    let n = vm.heap.table(list).border();
    (1..=n)
        .filter_map(|i| match vm.heap.table(list).get(Value::Num(i as f64)) {
            Value::Num(id) => Some((id as i64, 1.0)),
            Value::Table(e) => {
                let id = num(vm, e, "id")? as i64;
                Some((id, num(vm, e, "count").unwrap_or(1.0)))
            }
            _ => None,
        })
        .collect()
}

fn objectives(vm: &mut Vm, t: TableRef) -> Vec<Objective> {
    let Value::Table(list) = field(vm, t, "objectives") else {
        return vec![];
    };
    let n = vm.heap.table(list).border();
    let mut out = Vec::new();
    for i in 1..=n {
        let Value::Table(o) = vm.heap.table(list).get(Value::Num(i as f64)) else {
            continue;
        };
        let item = num(vm, o, "item").map(|n| n as i64);
        let kind = text(vm, o, "type");
        out.push(Objective {
            item,
            required: num(vm, o, "required").unwrap_or(1.0),
            text: text(vm, o, "text"),
            kind: if kind.is_empty() { "item".into() } else { kind },
            objective_type: num(vm, o, "objectiveType").unwrap_or(1.0),
        });
    }
    out
}

// ------------------------------------------------------------ the controls

/// `defineQuest(id, { title, text, objective, progress, completion, xp,
/// money, choices, rewards, objectives, giver, header, headerSortKey,
/// level, difficultyLevel, questClassification, reputation })`.
pub fn control_define(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_int(&c, 1)?;
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("defineQuest: a table of the quest's data expected"));
    };
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm).world.quests.defined.insert(id, t);
    // Its classification (`C_QuestInfoSystem.GetQuestClassification`;
    // Normal, 7, unless given).
    let classification = num(vm, t, "questClassification").unwrap_or(7.0);
    let stats = &mut engine_mut(vm).world.character.stats;
    let mut by = match stats.get("questClassification") {
        Some(Entry::ByIndex(by)) => by.clone(),
        _ => Default::default(),
    };
    by.insert(id, vec![Answer::Num(classification)]);
    stats.insert("questClassification", Entry::ByIndex(by));
    Ok(vec![])
}

/// `player:interact(npc)`: the player talks to an NPC; a round trip later
/// its dialog opens.
pub fn control_interact(vm: &mut Vm, c: NativeCall) -> Ret {
    if units::this(vm, &c)? != units::PLAYER {
        return Err(vm.error("interact: the player's"));
    }
    let Some(npc) = units::handle_of(vm, c.arg(2)) else {
        return Err(vm.error("interact: an NPC to talk to expected"));
    };
    // Too far (recorded at 9.9 yards): the client's error at once.
    if engine(vm).units.list[npc.0]
        .distance
        .is_some_and(|d| d > INTERACT_RANGE)
    {
        let text = global(
            vm,
            "ERR_TOO_FAR_TO_INTERACT",
            "You need to be closer to interact with that target.",
        );
        let code = match vm.get_global("LE_GAME_ERR_TOO_FAR_TO_INTERACT") {
            Value::Num(n) => n,
            _ => 0.0,
        };
        let text = vm.str(&text);
        fire(vm, "UI_ERROR_MESSAGE", &[Value::Num(code), text]);
        return Ok(vec![]);
    }
    if crate::gossip::has(vm, npc) {
        crate::gossip::interact(vm, npc);
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    let later = vm.native_with(
        "questDialog",
        dialog_opened as NativeFn,
        vec![Value::Num(npc.0 as f64)],
    );
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn giver_of(vm: &mut Vm, t: TableRef) -> Option<UnitId> {
    let v = field(vm, t, "giver");
    units::handle_of(vm, v)
}

/// The server opens the NPC's dialog: the quest it ends, in the log;
/// else the quest it offers.
fn dialog_opened(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(n) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let npc = UnitId(n as usize);
    let quests: Vec<(i64, TableRef)> = engine(vm)
        .world
        .quests
        .defined
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    let mut chosen = None;
    for (id, t) in &quests {
        if giver_of(vm, *t) != Some(npc) {
            continue;
        }
        let q = &engine(vm).world.quests;
        if q.log.iter().any(|l| l.id == *id) {
            chosen = Some((*id, Panel::Progress));
            break;
        }
        if !q.completed.contains(id) && chosen.is_none() {
            chosen = Some((*id, Panel::Detail));
        }
    }
    let Some((quest, panel)) = chosen else {
        return Ok(vec![]);
    };
    open(vm, npc, quest, panel);
    Ok(vec![])
}

fn open(vm: &mut Vm, npc: UnitId, quest: i64, panel: Panel) {
    engine_mut(vm).units.npc = Some(npc);
    engine_mut(vm).world.quests.dialog = Some(Dialog { npc, quest, panel });
    match panel {
        Panel::Detail => fire(vm, "QUEST_DETAIL", &[Value::Num(0.0)]),
        Panel::Progress => fire(vm, "QUEST_PROGRESS", &[]),
        Panel::Reward => fire(vm, "QUEST_COMPLETE", &[]),
    }
}

/// The dialog closed: `QUEST_FINISHED()`, the interaction's hides.
fn close(vm: &mut Vm) -> bool {
    if engine_mut(vm).world.quests.dialog.take().is_none() {
        return false;
    }
    engine_mut(vm).units.npc = None;
    fire(vm, "QUEST_FINISHED", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(QUEST_GIVER)],
    );
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(0.0)],
    );
    true
}

fn dialog(vm: &Vm) -> Option<Dialog> {
    engine(vm).world.quests.dialog
}

fn dialog_data(vm: &Vm) -> Option<(Dialog, TableRef)> {
    let d = dialog(vm)?;
    Some((d, data(vm, d.quest)?))
}

// ------------------------------------------------------------- the dialog

fn dialog_text(vm: &mut Vm, key: &str) -> Ret {
    let Some((_, t)) = dialog_data(vm) else {
        return Ok(vec![]);
    };
    let s = text(vm, t, key);
    Ok(vec![vm.str(&s)])
}

fn get_title_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    dialog_text(vm, "title")
}

fn get_quest_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    dialog_text(vm, "text")
}

fn get_objective_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    dialog_text(vm, "objective")
}

/// The progress text names the player as the client does ("Hey <name>.").
fn get_progress_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((_, t)) = dialog_data(vm) else {
        return Ok(vec![]);
    };
    let s = text(vm, t, "progress");
    let name = engine(vm).units.list[units::PLAYER.0].name.clone();
    let s = s.replace("$N", &name);
    Ok(vec![vm.str(&s)])
}

fn get_reward_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((_, t)) = dialog_data(vm) else {
        return Ok(vec![]);
    };
    let s = text(vm, t, "completion");
    let name = engine(vm).units.list[units::PLAYER.0].name.clone();
    let s = s.replace("$N", &name);
    Ok(vec![vm.str(&s)])
}

fn get_quest_id(vm: &mut Vm, _c: NativeCall) -> Ret {
    let id = dialog(vm).map_or(0.0, |d| d.quest as f64);
    Ok(vec![Value::Num(id)])
}

fn dialog_num(vm: &mut Vm, key: &str) -> Ret {
    let n = match dialog_data(vm) {
        Some((_, t)) => num(vm, t, key).unwrap_or(0.0),
        None => 0.0,
    };
    Ok(vec![Value::Num(n)])
}

fn get_reward_money(vm: &mut Vm, _c: NativeCall) -> Ret {
    dialog_num(vm, "money")
}

fn get_reward_xp(vm: &mut Vm, _c: NativeCall) -> Ret {
    dialog_num(vm, "xp")
}

/// No title rewarded: nothing (recorded).
fn get_reward_title(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn count_of(vm: &mut Vm, key: &str) -> Ret {
    let n = match dialog_data(vm) {
        Some((_, t)) => items(vm, t, key).len(),
        None => 0,
    };
    Ok(vec![Value::Num(n as f64)])
}

fn get_num_quest_rewards(vm: &mut Vm, _c: NativeCall) -> Ret {
    count_of(vm, "rewards")
}

fn get_num_quest_choices(vm: &mut Vm, _c: NativeCall) -> Ret {
    count_of(vm, "choices")
}

/// The items asked for: shown on the progress panel only (the detail
/// panel's answer 0, recorded).
fn required(vm: &mut Vm) -> Vec<(i64, f64)> {
    let Some((d, t)) = dialog_data(vm) else {
        return vec![];
    };
    if d.panel != Panel::Progress {
        return vec![];
    }
    objectives(vm, t)
        .into_iter()
        .filter_map(|o| Some((o.item?, o.required)))
        .collect()
}

fn get_num_quest_items(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = required(vm).len();
    Ok(vec![Value::Num(n as f64)])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// No portrait: `0, "", "", 0` (recorded).
fn get_quest_portrait(vm: &mut Vm, _c: NativeCall) -> Ret {
    let empty = vm.str("");
    Ok(vec![Value::Num(0.0), empty, empty, Value::Num(0.0)])
}

/// The item a dialog's list names: `choice`, `reward` or `required`.
fn listed(vm: &mut Vm, c: &NativeCall) -> Option<(i64, f64)> {
    let kind = text_of(vm, c.arg(1))?;
    let Value::Num(i) = c.arg(2) else { return None };
    let list = match kind.as_str() {
        "required" => required(vm),
        "choice" | "reward" => {
            let (_, t) = dialog_data(vm)?;
            items(
                vm,
                t,
                if kind == "choice" {
                    "choices"
                } else {
                    "rewards"
                },
            )
        }
        _ => return None,
    };
    list.get((i as usize).checked_sub(1)?).copied()
}

/// `GetQuestItemInfo(type, index)`: name, texture, count, quality,
/// usable, item ID.
fn get_quest_item_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((id, count)) = listed(vm, &c) else {
        return Ok(vec![]);
    };
    let name = crate::character::item_field(vm, id, "name");
    let icon = crate::character::item_field(vm, id, "icon");
    let quality = match crate::character::item_field(vm, id, "quality") {
        Value::Nil => Value::Num(1.0),
        q => q,
    };
    Ok(vec![
        name,
        icon,
        Value::Num(count),
        quality,
        Value::Bool(true),
        Value::Num(id as f64),
    ])
}

/// A quest dialog's item links carry the quest's context (11) and its
/// bonus (3524), as recorded for these items.
fn get_quest_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((id, _)) = listed(vm, &c) else {
        return Ok(vec![]);
    };
    let link = crate::character::link_in_context(vm, id, "11", &[3524]);
    Ok(vec![link.map_or(Value::Nil, |l| vm.str(&l))])
}

fn is_quest_completable(vm: &mut Vm, _c: NativeCall) -> Ret {
    let done = dialog(vm).is_some_and(|d| complete(vm, d.quest));
    Ok(vec![Value::Bool(done)])
}

/// Whether every objective of a quest in the log is fulfilled.
fn complete(vm: &mut Vm, id: i64) -> bool {
    let Some(t) = data(vm, id) else { return false };
    let objectives = objectives(vm, t);
    let Some(l) = engine(vm).world.quests.log.iter().find(|l| l.id == id) else {
        return false;
    };
    objectives
        .iter()
        .zip(&l.fulfilled)
        .all(|(o, n)| *n >= o.required)
}

// ---------------------------------------------------------- the actions

/// `AcceptQuest()`: the dialog closes at once; the server's answer a
/// round trip later.
fn accept_quest(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some(d) = dialog(vm) else {
        return Ok(vec![]);
    };
    if d.panel != Panel::Detail {
        return Ok(vec![]);
    }
    close(vm);
    // A second, as recorded (the frame's hiding closes it again).
    fire(vm, "QUEST_FINISHED", &[]);
    let delay = crate::clock::server_delay(vm);
    let later = vm.native_with(
        "questAccepted",
        accepted as NativeFn,
        vec![Value::Num(d.quest as f64)],
    );
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn quest_of(c: &NativeCall) -> Option<i64> {
    match c.upvals[0] {
        Value::Num(n) => Some(n as i64),
        _ => None,
    }
}

/// The server took the quest: in the log, watched.
fn accepted(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = quest_of(&c) else {
        return Ok(vec![]);
    };
    let Some(t) = data(vm, id) else {
        return Ok(vec![]);
    };
    let n = objectives(vm, t).len();
    {
        let q = &mut engine_mut(vm).world.quests;
        q.kept = true;
        q.log.push(Logged {
            id,
            fulfilled: vec![0.0; n],
            on_map: false,
            local_poi: false,
        });
    }
    sync_log(vm);
    units::fire_unit(vm, "UNIT_QUEST_LOG_CHANGED", units::PLAYER, &[]);
    let title = text(vm, t, "title");
    let format = global(vm, "ERR_QUEST_ACCEPTED_S", "Quest accepted: %s");
    crate::chat::game_message(
        vm,
        "CHAT_MSG_SYSTEM",
        &format.replacen("%s", &title, 1),
        None,
    );
    fire(
        vm,
        "QUEST_WATCH_LIST_CHANGED",
        &[Value::Num(id as f64), Value::Bool(true)],
    );
    fire(vm, "QUEST_ACCEPTED", &[Value::Num(id as f64)]);
    let next = vm.native("questTracked", tracked as NativeFn);
    crate::clock::after_engine(vm, 1.0 / 60.0, next);
    let poi = vm.native_with(
        "questOnMap",
        on_map as NativeFn,
        vec![Value::Num(id as f64)],
    );
    crate::clock::after_engine(vm, POI_AFTER, poi);
    Ok(vec![])
}

fn tracked(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "QUEST_LOG_UPDATE", &[]);
    fire(vm, "SUPER_TRACKING_CHANGED", &[]);
    Ok(vec![])
}

fn on_map(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = quest_of(&c) else {
        return Ok(vec![]);
    };
    if let Some(l) = engine_mut(vm)
        .world
        .quests
        .log
        .iter_mut()
        .find(|l| l.id == id)
    {
        l.on_map = true;
        l.local_poi = true;
    }
    sync_log(vm);
    fire(vm, "QUEST_POI_UPDATE", &[]);
    Ok(vec![])
}

/// `CompleteQuest()` (the progress panel's Continue): the server shows
/// the rewards a round trip later.
fn complete_quest(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some(d) = dialog(vm) else {
        return Ok(vec![]);
    };
    if d.panel != Panel::Progress || !complete(vm, d.quest) {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    let later = vm.native("questRewards", rewards_shown as NativeFn);
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn rewards_shown(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some(d) = dialog(vm) else {
        return Ok(vec![]);
    };
    close(vm);
    open(vm, d.npc, d.quest, Panel::Reward);
    Ok(vec![])
}

/// `GetQuestReward(choice)`: the quest turned in; the server's packets a
/// round trip later.
fn get_quest_reward(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(d) = dialog(vm) else {
        return Ok(vec![]);
    };
    if d.panel != Panel::Reward {
        return Ok(vec![]);
    }
    let choice = match c.arg(1) {
        Value::Num(n) => n,
        _ => 0.0,
    };
    let delay = crate::clock::server_delay(vm);
    for (after, f, name) in [
        (0.0, experienced as NativeFn, "questExperience"),
        (TURNED_IN_AFTER, turned_in, "questTurnedIn"),
        (REWARDED_AFTER, rewarded, "questRewarded"),
        (BAGS_AFTER, reward_in_bags, "questRewardInBags"),
    ] {
        let later = vm.native_with(
            name,
            f,
            vec![Value::Num(d.quest as f64), Value::Num(choice)],
        );
        crate::clock::after_engine(vm, delay + after, later);
    }
    Ok(vec![])
}

/// The experience: "You gain 170 experience." (the bar a packet later).
fn experienced(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = quest_of(&c).and_then(|id| data(vm, id)) else {
        return Ok(vec![]);
    };
    let xp = num(vm, t, "xp").unwrap_or(0.0);
    if xp > 0.0 {
        let format = global(
            vm,
            "COMBATLOG_XPGAIN_FIRSTPERSON_UNNAMED",
            "You gain %d experience.",
        );
        let text = format.replacen("%d", &format!("{xp}"), 1);
        crate::chat::game_message(vm, "CHAT_MSG_COMBAT_XP_GAIN", &text, None);
    }
    crate::character::equipped_spells_changed(vm);
    fire(vm, "QUEST_LOG_UPDATE", &[]);
    Ok(vec![])
}

/// The quest done: its messages, `QUEST_TURNED_IN`, the dialog closed.
fn turned_in(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = quest_of(&c) else {
        return Ok(vec![]);
    };
    let Some(t) = data(vm, id) else {
        return Ok(vec![]);
    };
    let title = text(vm, t, "title");
    let xp = num(vm, t, "xp").unwrap_or(0.0);
    let money = num(vm, t, "money").unwrap_or(0.0);
    let format = global(vm, "ERR_QUEST_COMPLETE_S", "%s completed.");
    crate::chat::game_message(
        vm,
        "CHAT_MSG_SYSTEM",
        &format.replacen("%s", &title, 1),
        None,
    );
    if xp > 0.0 {
        let format = global(vm, "ERR_QUEST_REWARD_EXP_I", "Experience gained: %d.");
        crate::chat::game_message(
            vm,
            "CHAT_MSG_SYSTEM",
            &format.replacen("%d", &format!("{xp}"), 1),
            None,
        );
    }
    fire(vm, "QUICK_TICKET_THROTTLE_CHANGED", &[]);
    fire(
        vm,
        "QUEST_TURNED_IN",
        &[Value::Num(id as f64), Value::Num(xp), Value::Num(money)],
    );
    if close(vm) {
        fire(vm, "QUEST_FINISHED", &[]);
    }
    fire(vm, "SUPER_TRACKING_CHANGED", &[]);
    Ok(vec![])
}

/// The reward: the item chosen, the reputation, the items asked for
/// taken, the quest out of the log.
fn rewarded(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = quest_of(&c) else {
        return Ok(vec![]);
    };
    let Some(t) = data(vm, id) else {
        return Ok(vec![]);
    };
    let choice = match c.upvals[1] {
        Value::Num(n) => n as usize,
        _ => 0,
    };
    let mut given = items(vm, t, "rewards");
    if let Some(item) = choice
        .checked_sub(1)
        .and_then(|i| items(vm, t, "choices").get(i).copied())
    {
        given.push(item);
    }
    for (item, count) in &given {
        let link = crate::character::link_in_context(vm, *item, "11", &[]).unwrap_or_default();
        let link = vm.str(&link);
        fire(
            vm,
            "QUEST_LOOT_RECEIVED",
            &[Value::Num(id as f64), link, Value::Num(*count)],
        );
    }
    reputation(vm, t);
    for (item, count) in &given {
        let bag = crate::character::bag_for(vm, *item, *count).unwrap_or(0);
        let icon = crate::character::item_field(vm, *item, "icon");
        fire(vm, "ITEM_PUSH", &[Value::Num(bag as f64), icon]);
        let link = crate::character::link_in_context(vm, *item, "11", &[]).unwrap_or_default();
        let format = global(vm, "LOOT_ITEM_PUSHED_SELF", "You receive item: %s");
        let sender = crate::loot::player_sender(vm);
        crate::chat::game_message(
            vm,
            "CHAT_MSG_LOOT",
            &format.replacen("%s", &link, 1),
            Some(&sender),
        );
        fire(vm, "BAG_NEW_ITEMS_UPDATED", &[]);
    }
    // The items asked for go to the giver.
    let objectives = objectives(vm, t);
    let mut bags = BTreeSet::new();
    for o in &objectives {
        if let Some(item) = o.item {
            bags.extend(crate::character::take_from_bags(vm, item, o.required));
        }
    }
    if let Some(l) = engine_mut(vm)
        .world
        .quests
        .log
        .iter_mut()
        .find(|l| l.id == id)
    {
        // Its point near gone, the map still showing it (recorded).
        l.local_poi = false;
    }
    units::fire_unit(vm, "UNIT_QUEST_LOG_CHANGED", units::PLAYER, &[]);
    let xp = num(vm, t, "xp").unwrap_or(0.0);
    if xp > 0.0 {
        crate::stats::gain_xp(vm, xp);
    }
    if bags.is_empty() {
        bags.insert(0);
    }
    for bag in bags {
        crate::character::bag_updated(vm, bag);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    sync_log(vm);
    fire(vm, "QUEST_LOG_UPDATE", &[]);
    {
        let q = &mut engine_mut(vm).world.quests;
        q.log.retain(|l| l.id != id);
        q.completed.insert(id);
    }
    sync_log(vm);
    fire(
        vm,
        "QUEST_WATCH_LIST_CHANGED",
        &[Value::Num(id as f64), Value::Bool(false)],
    );
    fire(
        vm,
        "QUEST_REMOVED",
        &[Value::Num(id as f64), Value::Bool(false)],
    );
    let next = vm.native("questLogEmptied", log_updated as NativeFn);
    crate::clock::after_engine(vm, 1.0 / 60.0, next);
    Ok(vec![])
}

fn log_updated(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "QUEST_LOG_UPDATE", &[]);
    Ok(vec![])
}

/// The reputation a quest gives: "Reputation with Stormwind increased by
/// 100.", `FACTION_STANDING_CHANGED(faction, standing)`.
fn reputation(vm: &mut Vm, t: TableRef) {
    let Value::Table(r) = field(vm, t, "reputation") else {
        return;
    };
    let name = text(vm, r, "name");
    let amount = num(vm, r, "amount").unwrap_or(0.0);
    let faction = num(vm, r, "faction").unwrap_or(0.0);
    let standing = num(vm, r, "standing").unwrap_or(0.0);
    let format = global(
        vm,
        "FACTION_STANDING_INCREASED",
        "Reputation with %s increased by %d.",
    );
    let text = format
        .replacen("%s", &name, 1)
        .replacen("%d", &format!("{amount}"), 1);
    crate::chat::game_message(vm, "CHAT_MSG_COMBAT_FACTION_CHANGE", &text, None);
    let faction_kind = vm.str("FACTION");
    fire(vm, "COMBAT_TEXT_UPDATE", &[faction_kind]);
    fire(
        vm,
        "FACTION_STANDING_CHANGED",
        &[Value::Num(faction), Value::Num(standing)],
    );
    fire(vm, "UPDATE_FACTION", &[]);
}

/// The bags with the reward in them: their update, the inventory's.
fn reward_in_bags(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = quest_of(&c).and_then(|id| data(vm, id)) else {
        return Ok(vec![]);
    };
    let choice = match c.upvals[1] {
        Value::Num(n) => n as usize,
        _ => 0,
    };
    let mut given = items(vm, t, "rewards");
    if let Some(item) = choice
        .checked_sub(1)
        .and_then(|i| items(vm, t, "choices").get(i).copied())
    {
        given.push(item);
    }
    crate::character::equipped_spells_changed(vm);
    let mut bags = BTreeSet::new();
    for (item, count) in given {
        if let Some((bag, _)) = crate::character::add_to_bags(vm, item, count) {
            crate::character::set_context(vm, bag, item, "11");
            bags.insert(bag);
        }
    }
    for bag in bags {
        crate::character::bag_updated(vm, bag);
    }
    units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", units::PLAYER, &[]);
    fire(vm, "PLAYER_AVG_ITEM_LEVEL_UPDATE", &[]);
    fire(vm, "PLAYER_AVG_ITEM_LEVEL_UPDATE", &[]);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    units::fire_unit(vm, "UNIT_MAXHEALTH", units::PLAYER, &[]);
    if let Some(target) = engine(vm).units.target {
        units::fire_unit(vm, "UNIT_MAXHEALTH", target, &[]);
    }
    Ok(vec![])
}

/// `CloseQuest()`: the dialog closes (nothing when it is not open).
fn close_quest(vm: &mut Vm, _c: NativeCall) -> Ret {
    close(vm);
    Ok(vec![])
}

/// `DeclineQuest()`: as closing (unverified).
fn decline_quest(vm: &mut Vm, _c: NativeCall) -> Ret {
    close(vm);
    Ok(vec![])
}

fn is_quest_flagged_completed(vm: &mut Vm, c: NativeCall) -> Ret {
    let done = match c.arg(1) {
        Value::Num(n) => engine(vm).world.quests.completed.contains(&(n as i64)),
        _ => false,
    };
    Ok(vec![Value::Bool(done)])
}

// ------------------------------------------------------ items and the log

/// Before an item's push: `QUEST_WATCH_UPDATE(id)` for each quest whose
/// unfinished objective asks for it.
pub fn before_item(vm: &mut Vm, item: i64) {
    for id in asking_for(vm, item) {
        fire(vm, "QUEST_WATCH_UPDATE", &[Value::Num(id as f64)]);
    }
}

/// The quests in the log with an unfinished objective for an item.
fn asking_for(vm: &mut Vm, item: i64) -> Vec<i64> {
    let log = engine(vm).world.quests.log.clone();
    let mut out = Vec::new();
    for l in log {
        let Some(t) = data(vm, l.id) else { continue };
        let objectives = objectives(vm, t);
        if objectives
            .iter()
            .zip(&l.fulfilled)
            .any(|(o, n)| o.item == Some(item) && *n < o.required)
        {
            out.push(l.id);
        }
    }
    out
}

/// Whether a quest in the log asks for an item (a loot slot's quest item).
pub fn is_quest_item(vm: &mut Vm, item: i64) -> bool {
    let log = engine(vm).world.quests.log.clone();
    log.iter().any(|l| {
        data(vm, l.id).is_some_and(|t| objectives(vm, t).iter().any(|o| o.item == Some(item)))
    })
}

/// After an item's push: each objective's new count told
/// (`UI_INFO_MESSAGE`), the log's packet later.
pub fn after_item(vm: &mut Vm, item: i64) {
    let have = crate::character::count_in_bags(vm, item);
    for id in asking_for(vm, item) {
        let Some(t) = data(vm, id) else { continue };
        let objectives = objectives(vm, t);
        for (i, o) in objectives.iter().enumerate() {
            if o.item != Some(item) {
                continue;
            }
            let n = have.min(o.required);
            if let Some(l) = engine_mut(vm)
                .world
                .quests
                .log
                .iter_mut()
                .find(|l| l.id == id)
            {
                l.fulfilled[i] = n;
            }
            let format = global(vm, "ERR_QUEST_ADD_ITEM_SII", "%s: %d/%d");
            let text = format
                .replacen("%s", &o.text, 1)
                .replacen("%d", &format!("{n}"), 1)
                .replacen("%d", &format!("{}", o.required), 1);
            game_error(vm, "ERR_QUEST_ADD_ITEM_SII", &text);
        }
        if complete(vm, id) {
            let text = global(vm, "ERR_QUEST_UNKNOWN_COMPLETE", "Objective Complete.");
            game_error(vm, "ERR_QUEST_UNKNOWN_COMPLETE", &text);
        }
        let later = vm.native("questLogChanged", log_changed as NativeFn);
        crate::clock::after_engine(vm, LOG_AFTER, later);
    }
}

fn log_changed(vm: &mut Vm, _c: NativeCall) -> Ret {
    sync_log(vm);
    units::fire_unit(vm, "UNIT_QUEST_LOG_CHANGED", units::PLAYER, &[]);
    let next = vm.native("questLogUpdated", log_updated as NativeFn);
    crate::clock::after_engine(vm, 1.0 / 60.0, next);
    Ok(vec![])
}

/// `UI_INFO_MESSAGE(LE_GAME_ERR_<name>, text)`.
fn game_error(vm: &mut Vm, name: &str, text: &str) {
    let code = match vm.get_global(&format!("LE_GAME_ERR_{}", &name[4..])) {
        Value::Num(n) => n,
        _ => 0.0,
    };
    let text = vm.str(text);
    fire(vm, "UI_INFO_MESSAGE", &[Value::Num(code), text]);
}

fn global(vm: &mut Vm, name: &str, default: &str) -> String {
    let v = vm.get_global(name);
    text_of(vm, v).unwrap_or_else(|| default.to_string())
}

fn s(v: &str) -> Answer {
    Answer::Str(v.to_string())
}

fn b(v: bool) -> Answer {
    Answer::Bool(v)
}

fn n(v: f64) -> Answer {
    Answer::Num(v)
}

fn structure(fields: Vec<(&str, Answer)>) -> Answer {
    Answer::Table(fields.into_iter().map(|(k, v)| (s(k), v)).collect())
}

/// The log as the character's stats tell it: a header per zone, then its
/// quests (`C_QuestLog.GetInfo`'s fields as recorded).
fn sync_log(vm: &mut Vm) {
    if !engine(vm).world.quests.kept {
        return;
    }
    let log = engine(vm).world.quests.log.clone();
    let mut info = std::collections::BTreeMap::new();
    let mut objectives_by = std::collections::BTreeMap::new();
    let mut complete_by = std::collections::BTreeMap::new();
    let mut titles = std::collections::BTreeMap::new();
    let mut indices = std::collections::BTreeMap::new();
    let mut watch_ids = std::collections::BTreeMap::new();
    let mut watch_types = std::collections::BTreeMap::new();
    let mut headers: Vec<String> = Vec::new();
    let mut index = 0i64;
    for (w, l) in log.iter().enumerate() {
        let Some(t) = data(vm, l.id) else { continue };
        let header = text(vm, t, "header");
        let classification = num(vm, t, "questClassification").unwrap_or(7.0);
        if !header.is_empty() && !headers.contains(&header) {
            headers.push(header.clone());
            index += 1;
            let sort_key = num(vm, t, "headerSortKey").unwrap_or(0.0);
            info.insert(
                index,
                vec![structure(vec![
                    ("difficultyLevel", n(0.0)),
                    ("hasLocalPOI", b(false)),
                    ("headerSortKey", n(sort_key)),
                    ("isAbandonOnDisable", b(false)),
                    ("isAutoComplete", b(false)),
                    ("isBounty", b(false)),
                    ("isCollapsed", b(true)),
                    ("isHeader", b(true)),
                    ("isHidden", b(false)),
                    ("isInternalOnly", b(false)),
                    ("isOnMap", b(false)),
                    ("isScaling", b(false)),
                    ("isStory", b(false)),
                    ("isTask", b(false)),
                    ("level", n(0.0)),
                    ("overridesSortOrder", b(false)),
                    ("questClassification", n(classification)),
                    ("questID", n(0.0)),
                    ("questLogIndex", n(index as f64)),
                    ("readyForTranslation", b(false)),
                    ("sortAsNormalQuest", b(true)),
                    ("startEvent", b(false)),
                    ("suggestedGroup", n(0.0)),
                    ("title", s(&header)),
                    ("useMinimalHeader", b(false)),
                ])],
            );
        }
        index += 1;
        let title = text(vm, t, "title");
        let level = num(vm, t, "level").unwrap_or(1.0);
        let difficulty = num(vm, t, "difficultyLevel").unwrap_or(level);
        let scaling = flag(vm, t, "isScaling", true);
        info.insert(
            index,
            vec![structure(vec![
                ("difficultyLevel", n(difficulty)),
                ("frequency", n(0.0)),
                ("hasLocalPOI", b(l.local_poi)),
                ("isAbandonOnDisable", b(false)),
                ("isAutoComplete", b(false)),
                ("isBounty", b(false)),
                ("isCollapsed", b(true)),
                ("isHeader", b(false)),
                ("isHidden", b(false)),
                ("isInternalOnly", b(false)),
                ("isOnMap", b(l.on_map)),
                ("isScaling", b(scaling)),
                ("isStory", b(false)),
                ("isTask", b(false)),
                ("level", n(level)),
                ("overridesSortOrder", b(false)),
                ("questClassification", n(classification)),
                ("questID", n(l.id as f64)),
                ("questLogIndex", n(index as f64)),
                ("readyForTranslation", b(true)),
                ("sortAsNormalQuest", b(true)),
                ("startEvent", b(false)),
                ("suggestedGroup", n(0.0)),
                ("title", s(&title)),
                ("useMinimalHeader", b(false)),
            ])],
        );
        let objectives = objectives(vm, t);
        let list: Vec<(Answer, Answer)> = objectives
            .iter()
            .zip(&l.fulfilled)
            .enumerate()
            .map(|(i, (o, have))| {
                (
                    n((i + 1) as f64),
                    structure(vec![
                        ("finished", b(*have >= o.required)),
                        ("numFulfilled", n(*have)),
                        ("numRequired", n(o.required)),
                        ("objectiveType", n(o.objective_type)),
                        ("text", s(&format!("{have}/{} {}", o.required, o.text))),
                        ("type", s(&o.kind)),
                    ]),
                )
            })
            .collect();
        objectives_by.insert(l.id, vec![Answer::Table(list)]);
        let done = objectives
            .iter()
            .zip(&l.fulfilled)
            .all(|(o, have)| *have >= o.required);
        complete_by.insert(l.id, vec![b(done)]);
        titles.insert(l.id, vec![s(&title)]);
        indices.insert(l.id, vec![n(index as f64)]);
        watch_ids.insert((w + 1) as i64, vec![n(l.id as f64)]);
        watch_types.insert(l.id, vec![n(1.0)]);
    }
    let shown = index as f64;
    let quests = log.len() as f64;
    let stats = &mut engine_mut(vm).world.character.stats;
    stats.insert("questLogEntries", Entry::One(vec![n(shown), n(quests)]));
    stats.insert("questLogInfo", Entry::ByIndex(info));
    stats.insert("questObjectives", Entry::ByIndex(objectives_by));
    stats.insert("questComplete", Entry::ByIndex(complete_by));
    stats.insert("questTitle", Entry::ByIndex(titles));
    stats.insert("questLogIndex", Entry::ByIndex(indices));
    stats.insert("questWatches", Entry::One(vec![n(quests)]));
    stats.insert("questWatchID", Entry::ByIndex(watch_ids));
    stats.insert("questWatchType", Entry::ByIndex(watch_types));
}

/// `C_QuestLog.GetNumQuestLogEntries()`: the log the character's stats
/// tell (the recorded one, the engine's once it keeps one), else empty.
fn num_quest_log_entries(vm: &mut Vm, _c: NativeCall) -> Ret {
    if let Some(Entry::One(a)) = engine(vm).world.character.stats.get("questLogEntries") {
        let (shown, quests) = match (a.first(), a.get(1)) {
            (Some(Answer::Num(s)), Some(Answer::Num(q))) => (*s, *q),
            _ => (0.0, 0.0),
        };
        return Ok(vec![Value::Num(shown), Value::Num(quests)]);
    }
    Ok(vec![Value::Num(0.0), Value::Num(0.0)])
}

/// `GetQuestUiMapID(questID)`: the map a quest's place is on (`defineQuest`'s
/// `uiMapID`), else 0, none (unverified: not recorded; a number, as
/// Blizzard's quest log compares it with 0).
fn get_quest_ui_map_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let map = match vm.hidden(c.arg(1)) {
        Value::Num(id) => data(vm, id as i64).and_then(|t| num(vm, t, "uiMapID")),
        _ => None,
    };
    Ok(vec![Value::Num(map.unwrap_or(0.0))])
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        // What a quest without them shows (unverified: not recorded):
        // no currencies, no hidden item (`0`), the parchment, not failed,
        // no criteria spell, no group, no skill points, artifact power or
        // honor, an item's loot type (0).
        ("GetNumQuestCurrencies", zero),
        ("IsQuestItemHidden", zero),
        ("GetQuestBackgroundMaterial", nothing),
        ("IsCurrentQuestFailed", no),
        ("GetCriteriaSpell", nothing),
        ("GetSuggestedGroupSize", zero),
        ("GetRewardSkillPoints", nothing),
        ("GetRewardArtifactXP", zeros),
        ("GetRewardHonor", zero),
        ("GetQuestItemInfoLootType", zero),
        ("C_QuestLog.GetNumQuestLogEntries", num_quest_log_entries),
        ("GetQuestUiMapID", get_quest_ui_map_id),
        ("GetTitleText", get_title_text),
        ("GetQuestText", get_quest_text),
        ("GetObjectiveText", get_objective_text),
        ("GetProgressText", get_progress_text),
        ("GetRewardText", get_reward_text),
        ("GetQuestID", get_quest_id),
        ("GetRewardMoney", get_reward_money),
        ("GetRewardXP", get_reward_xp),
        ("GetRewardTitle", get_reward_title),
        ("GetNumQuestRewards", get_num_quest_rewards),
        ("GetNumQuestChoices", get_num_quest_choices),
        ("GetNumQuestItems", get_num_quest_items),
        ("QuestGetAutoAccept", no),
        ("QuestIsFromAreaTrigger", no),
        ("QuestIsFromAdventureMap", no),
        ("QuestFlagsPVP", no),
        ("GetQuestMoneyToGet", zero),
        ("GetQuestPortraitGiver", get_quest_portrait),
        ("GetQuestPortraitTurnIn", get_quest_portrait),
        ("GetQuestItemInfo", get_quest_item_info),
        ("GetQuestItemLink", get_quest_item_link),
        ("IsQuestCompletable", is_quest_completable),
        ("AcceptQuest", accept_quest),
        ("DeclineQuest", decline_quest),
        ("CloseQuest", close_quest),
        ("CompleteQuest", complete_quest),
        ("GetQuestReward", get_quest_reward),
        (
            "C_QuestLog.IsQuestFlaggedCompleted",
            is_quest_flagged_completed,
        ),
    ]
}
