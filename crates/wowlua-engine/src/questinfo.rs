//! What addons and Blizzard's quest log, map and profession code ask about
//! quests, super-tracking, map pins and the systems around them, beyond
//! the quest dialog (`quests.rs`), the professions' windows
//! (`tradeskill.rs`) and the recorded maps (`maps.rs`).
//!
//! The quest log's answers come from the same place as `C_QuestLog.GetInfo`
//! (the character's stats: the recorded log, the engine's once it keeps
//! one) and from the quests the test defined (`defineQuest`'s rewards and
//! texts). Super-tracking, the user's map pin, the selected and abandoned
//! quest and the tracked recipes are kept here, per session (a table in the
//! registry). Systems Forever lacks (crafting orders, profession
//! specializations, world quests, bounties, quest sessions, vignettes,
//! quest lines) answer their documented empty values.
//!
//! Unverified unless said otherwise: none of these calls was recorded with
//! anything super-tracked, a pin set, a recipe tracked or a quest
//! abandoned; the events' order is the documentation's and Blizzard's
//! code's expectation.

use std::collections::BTreeSet;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::stats::{Answer, Entry};
use crate::units;
use crate::{engine, engine_mut, widgets};

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn num_of(vm: &Vm, v: Value) -> Option<f64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn text_of(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn list_of(vm: &mut Vm, values: &[Value]) -> Value {
    let t = vm.new_table();
    for (i, v) in values.iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num((i + 1) as f64), *v);
    }
    Value::Table(t)
}

// ------------------------------------------------------------- the state

/// This module's state: a table in the registry (super-tracking, the map
/// pin, the selected and abandoned quests, the tracked recipes).
fn state(vm: &mut Vm) -> TableRef {
    let registry = vm.registry;
    let key = vm.heap.intern(b"wowlua.engine.questinfo");
    match vm.heap.table(registry).get_str(key) {
        Value::Table(t) => t,
        _ => {
            let t = vm.new_table();
            vm.heap.table_mut(registry).set_str(key, Value::Table(t));
            t
        }
    }
}

fn get(vm: &mut Vm, key: &str) -> Value {
    let t = state(vm);
    field(vm, t, key)
}

fn put(vm: &mut Vm, key: &str, v: Value) {
    let t = state(vm);
    set(vm, t, key, v);
}

/// A table kept in the state (made empty the first time).
fn kept_table(vm: &mut Vm, key: &str) -> TableRef {
    match get(vm, key) {
        Value::Table(t) => t,
        _ => {
            let t = vm.new_table();
            put(vm, key, Value::Table(t));
            t
        }
    }
}

// --------------------------------------------------------- the quest log

/// A stat answered by index (`questLogIndex` by quest ID, …).
fn stat_at(vm: &Vm, key: &str, i: i64) -> Option<Vec<Answer>> {
    match engine(vm).world.character.stats.get(key) {
        Some(Entry::ByIndex(m)) => m.get(&i).cloned(),
        _ => None,
    }
}

/// A recorded structure's field.
fn answer_field<'a>(a: &'a Answer, key: &str) -> Option<&'a Answer> {
    match a {
        Answer::Table(fields) => fields
            .iter()
            .find(|(k, _)| matches!(k, Answer::Str(k) if k == key))
            .map(|(_, v)| v),
        _ => None,
    }
}

fn answer_num(a: Option<&Answer>) -> Option<f64> {
    match a {
        Some(Answer::Num(n)) => Some(*n),
        _ => None,
    }
}

/// An entry of the log as `C_QuestLog.GetInfo` tells it.
#[derive(Clone)]
struct LogEntry {
    index: i64,
    quest: i64,
    header: bool,
    title: String,
}

/// The log's entries in order (the character's stats: the recorded log,
/// the engine's once it keeps one).
fn log_entries(vm: &Vm) -> Vec<LogEntry> {
    let Some(Entry::ByIndex(log)) = engine(vm).world.character.stats.get("questLogInfo") else {
        return vec![];
    };
    log.iter()
        .filter_map(|(i, v)| {
            let info = v.first()?;
            Some(LogEntry {
                index: *i,
                quest: answer_num(answer_field(info, "questID")).unwrap_or(0.0) as i64,
                header: matches!(answer_field(info, "isHeader"), Some(Answer::Bool(true))),
                title: match answer_field(info, "title") {
                    Some(Answer::Str(s)) => s.clone(),
                    _ => String::new(),
                },
            })
        })
        .collect()
}

/// A quest's index in the log, if it is there.
fn log_index(vm: &Vm, quest: i64) -> Option<i64> {
    if quest <= 0 {
        return None;
    }
    if let Some(i) = stat_at(vm, "questLogIndex", quest).and_then(|a| answer_num(a.first())) {
        return Some(i as i64);
    }
    log_entries(vm)
        .into_iter()
        .find(|e| !e.header && e.quest == quest)
        .map(|e| e.index)
}

fn on_quest(vm: &Vm, quest: i64) -> bool {
    log_index(vm, quest).is_some()
}

/// The quest selected in the log (`C_QuestLog.GetSelectedQuest`'s).
fn selected(vm: &Vm) -> Option<i64> {
    match engine(vm).world.character.stats.get("selectedQuest") {
        Some(Entry::One(a)) => answer_num(a.first()).map(|n| n as i64).filter(|n| *n > 0),
        _ => None,
    }
}

/// The quest a call names at argument `n`, else the selected one (the
/// legacy quest log functions' optional `questID`).
fn quest_arg(vm: &Vm, c: &NativeCall, n: usize) -> Option<i64> {
    match num_of(vm, c.arg(n)) {
        Some(id) if id > 0.0 => Some(id as i64),
        _ => selected(vm),
    }
}

/// A defined quest's table (`defineQuest`).
fn defined(vm: &Vm, quest: i64) -> Option<TableRef> {
    engine(vm).world.quests.defined.get(&quest).copied()
}

fn defined_num(vm: &mut Vm, quest: Option<i64>, key: &str) -> f64 {
    match quest.and_then(|q| defined(vm, q)) {
        Some(t) => match field(vm, t, key) {
            Value::Num(n) => n,
            _ => 0.0,
        },
        None => 0.0,
    }
}

fn defined_text(vm: &mut Vm, quest: i64, key: &str) -> Option<String> {
    let t = defined(vm, quest)?;
    let v = field(vm, t, key);
    text_of(vm, v)
}

/// A defined quest's items (`rewards`, `choices`: `{ 80 }` or `{ { id =
/// 80, count = 1 } }`), as `quests.rs` reads them.
fn quest_items(vm: &mut Vm, quest: Option<i64>, key: &str) -> Vec<(i64, f64)> {
    let Some(t) = quest.and_then(|q| defined(vm, q)) else {
        return vec![];
    };
    let Value::Table(list) = field(vm, t, key) else {
        return vec![];
    };
    let n = vm.heap.table(list).border();
    let mut out = Vec::new();
    for i in 1..=n {
        match vm.heap.table(list).get(Value::Num(i as f64)) {
            Value::Num(id) => out.push((id as i64, 1.0)),
            Value::Table(e) => {
                let Value::Num(id) = field(vm, e, "id") else {
                    continue;
                };
                let count = match field(vm, e, "count") {
                    Value::Num(c) => c,
                    _ => 1.0,
                };
                out.push((id as i64, count));
            }
            _ => {}
        }
    }
    out
}

/// A quest's objectives as the log tells them (`C_QuestLog.GetQuestObjectives`).
fn objectives(vm: &Vm, quest: i64) -> Vec<Answer> {
    let Some(a) = stat_at(vm, "questObjectives", quest) else {
        return vec![];
    };
    match a.first() {
        Some(Answer::Table(entries)) => {
            let mut list: Vec<(f64, Answer)> = entries
                .iter()
                .filter_map(|(k, v)| match k {
                    Answer::Num(i) => Some((*i, v.clone())),
                    _ => None,
                })
                .collect();
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            list.into_iter().map(|(_, v)| v).collect()
        }
        _ => vec![],
    }
}

fn is_complete(vm: &Vm, quest: i64) -> bool {
    matches!(
        stat_at(vm, "questComplete", quest).as_deref(),
        Some([Answer::Bool(true), ..])
    )
}

// ------------------------------------------------- the quest log's rewards

/// `GetQuestLogRewardXP([questID])`: the defined quest's experience, total
/// and base (Blizzard's `QuestUtils` reads two; the selected quest when
/// none is named; 0 for a quest not defined).
fn get_quest_log_reward_xp(vm: &mut Vm, c: NativeCall) -> Ret {
    let quest = quest_arg(vm, &c, 1);
    let xp = defined_num(vm, quest, "xp");
    Ok(vec![Value::Num(xp), Value::Num(xp)])
}

/// `GetQuestLogRewardMoney([questID])`: the defined quest's money, else 0.
fn get_quest_log_reward_money(vm: &mut Vm, c: NativeCall) -> Ret {
    let quest = quest_arg(vm, &c, 1);
    Ok(vec![Value::Num(defined_num(vm, quest, "money"))])
}

/// `GetNumQuestLogRewards([questID])`: the defined quest's rewards.
fn get_num_quest_log_rewards(vm: &mut Vm, c: NativeCall) -> Ret {
    let quest = quest_arg(vm, &c, 1);
    let n = quest_items(vm, quest, "rewards").len();
    Ok(vec![Value::Num(n as f64)])
}

/// `GetNumQuestLogChoices([questID])`: the defined quest's choices.
fn get_num_quest_log_choices(vm: &mut Vm, c: NativeCall) -> Ret {
    let quest = quest_arg(vm, &c, 1);
    let n = quest_items(vm, quest, "choices").len();
    Ok(vec![Value::Num(n as f64)])
}

/// An item's reward info: name, texture, count, quality, usable, item ID,
/// item level (warcraft.wiki.gg's `GetQuestLogRewardInfo`; the dialog's
/// `GetQuestItemInfo` gives the first six alike).
fn reward_info(vm: &mut Vm, id: i64, count: f64) -> Vec<Value> {
    let name = crate::character::item_field(vm, id, "name");
    let icon = crate::character::item_field(vm, id, "icon");
    let quality = match crate::character::item_field(vm, id, "quality") {
        Value::Nil => Value::Num(1.0),
        q => q,
    };
    let level = match crate::character::item_field(vm, id, "itemLevel") {
        Value::Num(n) => Value::Num(n),
        _ => Value::Num(0.0),
    };
    vec![
        name,
        icon,
        Value::Num(count),
        quality,
        Value::Bool(true),
        Value::Num(id as f64),
        level,
    ]
}

fn listed_item(vm: &mut Vm, c: &NativeCall, key: &str) -> Option<(i64, f64)> {
    let i = num_of(vm, c.arg(1))?;
    let quest = quest_arg(vm, c, 2);
    let list = quest_items(vm, quest, key);
    list.get((i as usize).checked_sub(1)?).copied()
}

/// `GetQuestLogRewardInfo(index [, questID])`.
fn get_quest_log_reward_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match listed_item(vm, &c, "rewards") {
        Some((id, count)) => reward_info(vm, id, count),
        // No such reward: its six values empty, 1 of quality 0 (the full
        // probe).
        None => {
            let mut out = vec![Value::Nil; 6];
            out[2] = Value::Num(1.0);
            out[3] = Value::Num(0.0);
            out
        }
    })
}

/// `GetQuestLogChoiceInfo(index [, questID])`.
fn get_quest_log_choice_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match listed_item(vm, &c, "choices") {
        Some((id, count)) => reward_info(vm, id, count),
        // No such choice: eight values, as a reward's (the full probe).
        None => {
            let mut out = vec![Value::Nil; 8];
            out[2] = Value::Num(1.0);
            out[3] = Value::Num(0.0);
            out
        }
    })
}

/// `GetQuestLogItemLink(type, index [, questID])`: the reward's link, in
/// the quest's context as the dialog's (`GetQuestItemLink`, recorded: 11
/// and bonus 3524; unverified for the log).
fn get_quest_log_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(kind) = text_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let key = match kind.as_str() {
        "choice" => "choices",
        "reward" => "rewards",
        _ => return Ok(vec![]),
    };
    let Some(i) = num_of(vm, c.arg(2)) else {
        return Ok(vec![]);
    };
    let quest = quest_arg(vm, &c, 3);
    let list = quest_items(vm, quest, key);
    let Some((id, _)) = (i as usize)
        .checked_sub(1)
        .and_then(|i| list.get(i).copied())
    else {
        return Ok(vec![]);
    };
    let link = crate::character::link_in_context(vm, id, "11", &[3524]);
    Ok(vec![link.map_or(Value::Nil, |l| vm.str(&l))])
}

/// `GetQuestLogQuestText([questLogIndex])`: the quest's description and
/// objectives as defined (the selected quest's when no index is given).
fn get_quest_log_quest_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let quest = match num_of(vm, c.arg(1)) {
        Some(i) => log_entries(vm)
            .into_iter()
            .find(|e| e.index == i as i64 && !e.header)
            .map(|e| e.quest),
        None => selected(vm),
    };
    let Some(quest) = quest else {
        // No such quest: two nils (the full probe).
        return Ok(vec![Value::Nil, Value::Nil]);
    };
    let text = defined_text(vm, quest, "text").unwrap_or_default();
    let objective = defined_text(vm, quest, "objective").unwrap_or_default();
    Ok(vec![vm.str(&text), vm.str(&objective)])
}

/// `GetQuestObjectiveInfo(questID, index, displayComplete)`: the
/// objective's text, type, whether finished, fulfilled and required (the
/// log's `C_QuestLog.GetQuestObjectives`; warcraft.wiki.gg's order);
/// `displayComplete` tells it as finished.
fn get_quest_objective_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(quest), Some(i)) = (num_of(vm, c.arg(1)), num_of(vm, c.arg(2))) else {
        return Ok(vec![]);
    };
    let list = objectives(vm, quest as i64);
    let Some(o) = (i as usize)
        .checked_sub(1)
        .and_then(|i| list.get(i).cloned())
    else {
        return Ok(vec![]);
    };
    let complete = c.arg(3).truthy();
    let text = match answer_field(&o, "text") {
        Some(Answer::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let kind = match answer_field(&o, "type") {
        Some(Answer::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let finished = complete || matches!(answer_field(&o, "finished"), Some(Answer::Bool(true)));
    let required = answer_num(answer_field(&o, "numRequired")).unwrap_or(0.0);
    let fulfilled = if complete {
        required
    } else {
        answer_num(answer_field(&o, "numFulfilled")).unwrap_or(0.0)
    };
    Ok(vec![
        vm.str(&text),
        vm.str(&kind),
        Value::Bool(finished),
        Value::Num(fulfilled),
        Value::Num(required),
    ])
}

/// `GetQuestLink(questID)`: a link to a quest in the log
/// (`|cffffff00|Hquest:ID:0|h[Title]|h|r`, coloured by its difficulty),
/// else nil (warcraft.wiki.gg: nil for a quest not in the log).
fn get_quest_link(vm: &mut Vm, c: NativeCall) -> Ret {
    // A quest not in the log: nothing (the full probe; the newest answer,
    // over nil).
    let Some(quest) = num_of(vm, c.arg(1)).map(|n| n as i64) else {
        return Ok(vec![]);
    };
    let Some(index) = log_index(vm, quest) else {
        return Ok(vec![]);
    };
    let info = stat_at(vm, "questLogInfo", index).and_then(|a| a.first().cloned());
    let level = info
        .as_ref()
        .and_then(|i| answer_num(answer_field(i, "level")))
        .unwrap_or(0.0);
    let title = match info.as_ref().and_then(|i| answer_field(i, "title")) {
        Some(Answer::Str(s)) => s.clone(),
        _ => String::new(),
    };
    // The quest's colour for the player's level, Blizzard's
    // `GetRelativeDifficultyColor` steps with `UnitQuestTrivialLevelRange`'s
    // 5; the link's level field 0 (the full probe: 1063, level 18 for a
    // 21, `|cff40c040|Hquest:1063:0|h`; the other colours' codes
    // unverified).
    let me = engine(vm).units.list[units::PLAYER.0].level;
    let diff = level - me;
    let color = if diff >= 5.0 {
        "ff1919"
    } else if diff >= 3.0 {
        "ff8040"
    } else if diff >= -2.0 {
        "ffff00"
    } else if -diff <= 5.0 {
        "40c040"
    } else {
        "808080"
    };
    let link = format!("|cff{color}|Hquest:{quest}:0|h[{title}]|h|r");
    Ok(vec![vm.str(&link)])
}

/// `GetBindLocation()`: where the Hearthstone takes the player (its
/// `teleport`'s subzone, else zone: the hearth recording's "Thunder
/// Bluff"); else empty, noted as unanswered.
fn get_bind_location(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Value::Table(t) = crate::character::item_field(vm, 6948, "teleport") {
        for key in ["subZone", "zone"] {
            let v = field(vm, t, key);
            if let Some(name) = text_of(vm, v).filter(|n| !n.is_empty()) {
                return Ok(vec![vm.str(&name)]);
            }
        }
    }
    crate::note_default(vm, "GetBindLocation", &c, 1);
    Ok(vec![vm.str("")])
}

/// `GetQuestLogPortraitTurnIn([index])`: no portrait for a quest of the log
/// (0, "", ""), nothing for an index naming none (the full probe).
fn log_portrait_turn_in(vm: &mut Vm, c: NativeCall) -> Ret {
    let quest = match num_of(vm, c.arg(1)) {
        Some(i) => log_entries(vm)
            .into_iter()
            .find(|e| e.index == i as i64 && !e.header)
            .map(|e| e.quest),
        None => selected(vm),
    };
    if quest.is_none() {
        return Ok(vec![]);
    }
    let empty = vm.str("");
    Ok(vec![Value::Num(0.0), empty, empty])
}

// ------------------------------------------------------------- C_QuestLog

/// `C_QuestLog.IsOnQuest(questID)`: in the log.
fn is_on_quest(vm: &mut Vm, c: NativeCall) -> Ret {
    let on = num_of(vm, c.arg(1)).is_some_and(|q| on_quest(vm, q as i64));
    Ok(vec![Value::Bool(on)])
}

/// `C_QuestLog.IsUnitOnQuest(unit, questID)`: false, the player too (the
/// full probe: "player" on two quests in its log false; it was the
/// player's log before); other units' logs are not known (false).
fn is_unit_on_quest(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `C_QuestLog.ReadyForTurnIn(questID)`: a quest in the log with every
/// objective done (its `C_QuestLog.IsComplete`); false otherwise.
fn ready_for_turn_in(vm: &mut Vm, c: NativeCall) -> Ret {
    let ready = num_of(vm, c.arg(1)).is_some_and(|q| {
        let q = q as i64;
        on_quest(vm, q) && is_complete(vm, q)
    });
    Ok(vec![Value::Bool(ready)])
}

/// `C_QuestLog.GetNumQuestObjectives(questID)`: the log's objectives.
fn get_num_quest_objectives(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = num_of(vm, c.arg(1)).map_or(0, |q| objectives(vm, q as i64).len());
    Ok(vec![Value::Num(n as f64)])
}

/// `C_QuestLog.GetTitleForLogIndex(questLogIndex)`: the entry's title (a
/// header's too); an index with no entry "" (the full probe: 0 and
/// 2147483647); nil without one.
fn get_title_for_log_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = num_of(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    match log_entries(vm).into_iter().find(|e| e.index == i as i64) {
        Some(e) => Ok(vec![vm.str(&e.title)]),
        None => Ok(vec![vm.str("")]),
    }
}

/// `C_QuestLog.GetHeaderIndexForQuest(questID)`: the index of the header
/// above the quest in the log, else nil.
fn get_header_index_for_quest(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(quest) = num_of(vm, c.arg(1)).map(|n| n as i64) else {
        return Ok(vec![Value::Nil]);
    };
    let mut header = None;
    for e in log_entries(vm) {
        if e.header {
            header = Some(e.index);
        } else if e.quest == quest {
            return Ok(vec![header.map_or(Value::Nil, |h| Value::Num(h as f64))]);
        }
    }
    Ok(vec![Value::Nil])
}

/// `C_QuestLog.IsOnMap(questID)`: the engine's log's map flags (shown, a
/// point of interest near), else false, false.
fn is_on_map(vm: &mut Vm, c: NativeCall) -> Ret {
    let quest = num_of(vm, c.arg(1)).map(|n| n as i64);
    let found = quest.and_then(|q| {
        engine(vm)
            .world
            .quests
            .log
            .iter()
            .find(|l| l.id == q)
            .map(|l| (l.on_map, l.local_poi))
    });
    let (on_map, local) = found.unwrap_or((false, false));
    Ok(vec![Value::Bool(on_map), Value::Bool(local)])
}

/// `C_QuestLog.DoesQuestAwardReputationWithFaction(questID, factionID)`:
/// the defined quest's reputation's faction.
fn does_quest_award_reputation(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(quest), Some(faction)) = (num_of(vm, c.arg(1)), num_of(vm, c.arg(2))) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let awards = match defined(vm, quest as i64) {
        Some(t) => match field(vm, t, "reputation") {
            Value::Table(r) => matches!(field(vm, r, "faction"), Value::Num(f) if f == faction),
            _ => false,
        },
        None => false,
    };
    Ok(vec![Value::Bool(awards)])
}

/// `C_QuestLog.IsQuestFlaggedCompletedOnAccount(questID)`: completed by
/// the character (the only one the engine knows).
fn is_completed_on_account(vm: &mut Vm, c: NativeCall) -> Ret {
    let done = num_of(vm, c.arg(1))
        .is_some_and(|q| engine(vm).world.quests.completed.contains(&(q as i64)));
    Ok(vec![Value::Bool(done)])
}

/// `C_QuestLog.GetAllCompletedQuestIDs()`: the quests completed, in order.
fn get_all_completed_quest_ids(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ids: Vec<Value> = engine(vm)
        .world
        .quests
        .completed
        .iter()
        .map(|id| Value::Num(*id as f64))
        .collect();
    Ok(vec![list_of(vm, &ids)])
}

/// `C_QuestLog.GetMaxNumQuestsCanAccept()`: 40 (recorded; its
/// `C_QuestLog.GetMaxNumQuests()` was recorded 175).
fn get_max_num_quests_can_accept(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(40.0)])
}

/// `C_QuestLog.ShouldShowQuestRewards(questID)`: a quest in the log (no
/// quest hides its rewards in the engine's world) or 0, yes; another no
/// (the full probe: 1, -1, 2147483647 no).
fn should_show_quest_rewards(vm: &mut Vm, c: NativeCall) -> Ret {
    let show = num_of(vm, c.arg(1)).is_some_and(|q| q == 0.0 || on_quest(vm, q as i64));
    Ok(vec![Value::Bool(show)])
}

/// `C_QuestLog.GetQuestLogPortraitGiver([logIndex])`: a quest's entry in
/// the log no portrait (`0, "", "", 0, nil`); a header, no entry or none
/// asked, nothing (the full probe: index 2 a quest, 1 a header).
/// Unverified: no index with a quest selected.
fn quest_log_portrait_giver(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = num_of(vm, c.arg(1)).filter(|i| i.fract() == 0.0) else {
        return Ok(vec![]);
    };
    let info = stat_at(vm, "questLogInfo", i as i64).and_then(|a| a.first().cloned());
    let quest = info
        .as_ref()
        .is_some_and(|i| matches!(answer_field(i, "isHeader"), Some(Answer::Bool(false))));
    if !quest {
        return Ok(vec![]);
    }
    let empty = vm.str("");
    Ok(vec![
        Value::Num(0.0),
        empty,
        empty,
        Value::Num(0.0),
        Value::Nil,
    ])
}

/// `C_QuestInfoSystem.GetQuestRewardCurrencies(questID)` and
/// `GetQuestRewardSpells`: none for a quest ID (from 0); nothing for a
/// fraction, a negative or none (the full probe: 0, 1, logged quests a
/// list; -1, 1.5, 2147483647, nil nothing). Unverified: where the quests'
/// IDs end (here below a million).
fn quest_reward_list(vm: &mut Vm, c: NativeCall) -> Ret {
    let quest =
        num_of(vm, c.arg(1)).is_some_and(|q| q.fract() == 0.0 && (0.0..1_000_000.0).contains(&q));
    if !quest {
        return Ok(vec![]);
    }
    Ok(vec![Value::Table(vm.new_table())])
}

/// `C_QuestLog.GetQuestType(questID)`: 0 for a quest of no type (the full
/// probe: every ID but a dungeon quest's, 6481 1; game data the engine
/// lacks).
fn get_quest_type(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `C_QuestLog.SetSelectedQuest(questID)`: the log's selection
/// (`C_QuestLog.GetSelectedQuest`'s answer from then on); a negative ID
/// selects none (the full probe, 1.60.1: `SetSelectedQuest(-1)` read 0).
fn set_selected_quest(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num_of(vm, c.arg(1)).unwrap_or(0.0).max(0.0);
    engine_mut(vm)
        .world
        .character
        .stats
        .insert("selectedQuest", Entry::One(vec![Answer::Num(id)]));
    Ok(vec![])
}

/// The quest watches as the stats tell them (`questWatchID` by watch
/// index).
fn watches(vm: &Vm) -> Vec<i64> {
    match engine(vm).world.character.stats.get("questWatchID") {
        Some(Entry::ByIndex(m)) => m
            .values()
            .filter_map(|a| answer_num(a.first()).map(|n| n as i64))
            .collect(),
        _ => vec![],
    }
}

fn set_watches(vm: &mut Vm, list: &[i64], added: Option<i64>, removed: Option<i64>) {
    let stats = &mut engine_mut(vm).world.character.stats;
    let ids = list
        .iter()
        .enumerate()
        .map(|(i, id)| ((i + 1) as i64, vec![Answer::Num(*id as f64)]))
        .collect();
    stats.insert("questWatchID", Entry::ByIndex(ids));
    stats.insert(
        "questWatches",
        Entry::One(vec![Answer::Num(list.len() as f64)]),
    );
    let mut types = match stats.get("questWatchType") {
        Some(Entry::ByIndex(m)) => m.clone(),
        _ => Default::default(),
    };
    if let Some(id) = added {
        // `Enum.QuestWatchType.Manual`.
        types.insert(id, vec![Answer::Num(1.0)]);
    }
    if let Some(id) = removed {
        types.remove(&id);
    }
    stats.insert("questWatchType", Entry::ByIndex(types));
}

/// `C_QuestLog.AddQuestWatch(questID)`: a quest in the log watched (last;
/// manually), `QUEST_WATCH_LIST_CHANGED(id, true)`; whether it was added.
fn add_quest_watch(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(quest) = num_of(vm, c.arg(1)).map(|n| n as i64) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let mut list = watches(vm);
    if !on_quest(vm, quest) || list.contains(&quest) {
        return Ok(vec![Value::Bool(false)]);
    }
    list.push(quest);
    set_watches(vm, &list, Some(quest), None);
    fire(
        vm,
        "QUEST_WATCH_LIST_CHANGED",
        &[Value::Num(quest as f64), Value::Bool(true)],
    );
    Ok(vec![Value::Bool(true)])
}

/// `C_QuestLog.RemoveQuestWatch(questID)`: the watch removed,
/// `QUEST_WATCH_LIST_CHANGED(id, false)`; whether it was watched.
fn remove_quest_watch(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(quest) = num_of(vm, c.arg(1)).map(|n| n as i64) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let mut list = watches(vm);
    if !list.contains(&quest) {
        return Ok(vec![Value::Bool(false)]);
    }
    list.retain(|q| *q != quest);
    set_watches(vm, &list, None, Some(quest));
    fire(
        vm,
        "QUEST_WATCH_LIST_CHANGED",
        &[Value::Num(quest as f64), Value::Bool(false)],
    );
    Ok(vec![Value::Bool(true)])
}

/// `C_QuestLog.SetAbandonQuest()`: the selected quest is the one to
/// abandon.
fn set_abandon_quest(vm: &mut Vm, _c: NativeCall) -> Ret {
    let id = selected(vm).unwrap_or(0);
    put(vm, "abandon", Value::Num(id as f64));
    Ok(vec![])
}

/// `C_QuestLog.GetAbandonQuest()`: the quest set to abandon, else 0.
fn get_abandon_quest(vm: &mut Vm, _c: NativeCall) -> Ret {
    let id = match get(vm, "abandon") {
        Value::Num(n) => n,
        _ => 0.0,
    };
    Ok(vec![Value::Num(id)])
}

/// `C_QuestLog.AbandonQuest()`: the quest set to abandon leaves the
/// engine's log a round trip later (`UNIT_QUEST_LOG_CHANGED`, its watch
/// and the quest removed, `QUEST_LOG_UPDATE`; as a turn-in's removal, not
/// recorded for an abandon). A recorded log is not changed.
fn abandon_quest(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Value::Num(id) = get(vm, "abandon") else {
        return Ok(vec![]);
    };
    put(vm, "abandon", Value::Num(0.0));
    let delay = crate::clock::server_delay(vm);
    let later = vm.native_with(
        "questAbandoned",
        abandoned as NativeFn,
        vec![Value::Num(id)],
    );
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn abandoned(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(id)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let id = id as i64;
    let q = &engine(vm).world.quests;
    if !q.kept || !q.log.iter().any(|l| l.id == id) {
        return Ok(vec![]);
    }
    engine_mut(vm).world.quests.log.retain(|l| l.id != id);
    crate::quests::sync_log(vm);
    if super_quest(vm) == Some(id) {
        put(vm, "quest", Value::Nil);
        fire(vm, "SUPER_TRACKING_CHANGED", &[]);
    }
    units::fire_unit(vm, "UNIT_QUEST_LOG_CHANGED", units::PLAYER, &[]);
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
    fire(vm, "QUEST_LOG_UPDATE", &[]);
    Ok(vec![])
}

/// `C_QuestLog.SetMapForQuestPOIs(uiMapID)`: kept for
/// `C_QuestLog.GetMapForQuestPOIs()` (0 before any); another map than the
/// one before whose view of the log differs (`quests::quest_map_changed`):
/// the log's `QUEST_LOG_UPDATE` in the same frame (recorded three times in
/// `questlog`: a quest's details showing its map, twice, the details' back
/// button; twice in `mapnav`: the world map zoomed out to Kalimdor and
/// back; none in `taxi` while the zones' maps changed under the flight,
/// the log's order the same). With the world map hidden, none: the log as
/// the new map shows it with the server's next update (unverified, once
/// in `hearth`: Thunder Bluff's map set as the player arrived, the next
/// update 0.07 s later).
fn set_map_for_quest_pois(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num_of(vm, c.arg(1)).unwrap_or(0.0);
    let before = match get(vm, "poiMap") {
        Value::Num(n) => n,
        _ => 0.0,
    };
    put(vm, "poiMap", Value::Num(id));
    if id != before && crate::quests::quest_map_changed(vm, id) && world_map_shown(vm) {
        let update = vm.native("questMapUpdate", quest_map_update as NativeFn);
        crate::clock::after_engine(vm, 0.0, update);
    }
    Ok(vec![])
}

/// Blizzard's world map shown.
fn world_map_shown(vm: &mut Vm) -> bool {
    let map = vm.get_global("WorldMapFrame");
    crate::widgets::widget_of(vm, map).is_some_and(|id| crate::widgets::visible(engine(vm), id))
}

fn quest_map_update(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "QUEST_LOG_UPDATE", &[]);
    Ok(vec![])
}

/// The map for quest POIs (0 before any).
pub fn poi_map(vm: &mut Vm) -> f64 {
    match get(vm, "poiMap") {
        Value::Num(n) => n,
        _ => 0.0,
    }
}

fn get_map_for_quest_pois(vm: &mut Vm, _c: NativeCall) -> Ret {
    let id = match get(vm, "poiMap") {
        Value::Num(n) => n,
        _ => 0.0,
    };
    Ok(vec![Value::Num(id)])
}

/// `C_QuestLog.RequestLoadQuestByID(questID)`: the quest's data a round
/// trip later, `QUEST_DATA_LOAD_RESULT(questID, success)`: success for a
/// quest the engine knows (defined, logged or completed).
fn request_load_quest(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = num_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let q = id as i64;
    let known = defined(vm, q).is_some()
        || on_quest(vm, q)
        || engine(vm).world.quests.completed.contains(&q);
    let delay = crate::clock::server_delay(vm);
    let later = vm.native_with(
        "questDataLoaded",
        quest_data_loaded as NativeFn,
        vec![Value::Num(id), Value::Bool(known)],
    );
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn quest_data_loaded(vm: &mut Vm, c: NativeCall) -> Ret {
    let args: Vec<Value> = c.upvals.to_vec();
    fire(vm, "QUEST_DATA_LOAD_RESULT", &args);
    Ok(vec![])
}

// --------------------------------------------------------- super-tracking

/// What is super-tracked: one thing at a time (setting one clears the
/// others; unverified).
const SUPER_KEYS: &[&str] = &[
    "quest",
    "waypoint",
    "pinType",
    "pinID",
    "vignette",
    "contentType",
    "contentID",
];

fn super_summary(vm: &mut Vm) -> String {
    let mut out = String::new();
    for k in SUPER_KEYS {
        let v = get(vm, k);
        let part = match v {
            Value::Num(n) => format!("{n}"),
            Value::Bool(b) => format!("{b}"),
            Value::Str(_) => text_of(vm, v).unwrap_or_default(),
            _ => String::new(),
        };
        out.push_str(&part);
        out.push('|');
    }
    out
}

/// Super-tracks what `sets` names (each key and its value), the rest
/// cleared; `SUPER_TRACKING_CHANGED()` when that changed anything.
fn super_track(vm: &mut Vm, sets: &[(&str, Value)]) {
    let before = super_summary(vm);
    for k in SUPER_KEYS {
        put(vm, k, Value::Nil);
    }
    for (k, v) in sets {
        put(vm, k, *v);
    }
    if super_summary(vm) != before {
        fire(vm, "SUPER_TRACKING_CHANGED", &[]);
    }
}

/// A quest's POI came: the navigation frame made if it is the quest
/// super-tracked (`navigation_frame`).
pub(crate) fn quest_poi_arrived(vm: &mut Vm, quest: i64) -> Result<(), wowlua_vm::LuaError> {
    if super_quest(vm) == Some(quest) {
        navigation_frame(vm)?;
    }
    Ok(())
}

fn super_quest(vm: &mut Vm) -> Option<i64> {
    match get(vm, "quest") {
        Value::Num(n) if n > 0.0 => Some(n as i64),
        _ => None,
    }
}

/// `C_SuperTrack.GetSuperTrackedQuestID()`: the quest super-tracked while
/// it is in the log; else nothing (recorded at load: nothing).
fn get_super_tracked_quest_id(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(match super_quest(vm).filter(|q| on_quest(vm, *q)) {
        Some(q) => vec![Value::Num(q as f64)],
        None => vec![],
    })
}

/// `C_SuperTrack.SetSuperTrackedQuestID(questID)`: that quest (0 clears).
fn set_super_tracked_quest_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num_of(vm, c.arg(1)).unwrap_or(0.0);
    if id > 0.0 {
        super_track(vm, &[("quest", Value::Num(id))]);
    } else if super_quest(vm).is_some() {
        super_track(vm, &[]);
    }
    Ok(vec![])
}

fn is_super_tracking_quest(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = super_quest(vm).is_some_and(|q| on_quest(vm, q));
    Ok(vec![Value::Bool(on)])
}

/// `C_SuperTrack.SetSuperTrackedUserWaypoint(superTracked)`: the user's
/// map pin, when there is one. The first place super-tracked creates the
/// client's navigation frame: `NAVIGATION_FRAME_CREATED(frame)` after
/// `SUPER_TRACKING_CHANGED`, the frame kept as the pin goes (recorded once,
/// scenario mapnav: no `NAVIGATION_FRAME_DESTROYED` at its removal).
fn set_super_tracked_user_waypoint(vm: &mut Vm, c: NativeCall) -> Ret {
    let on = c.arg(1).truthy();
    let tracked = get(vm, "waypoint").truthy();
    if on && waypoint(vm).is_some() {
        super_track(vm, &[("waypoint", Value::Bool(true))]);
        navigation_frame(vm)?;
    } else if !on && tracked {
        super_track(vm, &[]);
    }
    Ok(vec![])
}

/// The navigation frame made once (see `set_super_tracked_user_waypoint`):
/// when the place super-tracked first has a position. A user's pin has
/// one at once (`mapnav`); a quest once its POI comes (`quest` 5.690: with
/// `QUEST_POI_UPDATE`, 0.46 s after `SUPER_TRACKING_CHANGED`); the
/// corpse a moment after the release (`death` 29.621, `death::released`).
pub(crate) fn navigation_frame(vm: &mut Vm) -> Result<(), wowlua_vm::LuaError> {
    if !get(vm, "navFrame").is_nil() {
        return Ok(());
    }
    let create = vm.get_global("CreateFrame");
    if !matches!(create, Value::Func(_)) {
        return Ok(());
    }
    let kind = vm.str("Frame");
    let frame = vm
        .call(create, &[kind])?
        .first()
        .copied()
        .unwrap_or(Value::Nil);
    put(vm, "navFrame", frame);
    fire(vm, "NAVIGATION_FRAME_CREATED", &[frame]);
    Ok(())
}

/// `C_Navigation.GetFrame()`: the navigation frame once made, else nil.
fn get_navigation_frame(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![get(vm, "navFrame")])
}

fn is_super_tracking_user_waypoint(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = get(vm, "waypoint").truthy();
    Ok(vec![Value::Bool(on)])
}

/// `C_SuperTrack.SetSuperTrackedMapPin(type, typeID)`.
fn set_super_tracked_map_pin(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(kind), Some(id)) = (num_of(vm, c.arg(1)), num_of(vm, c.arg(2))) else {
        return Ok(vec![]);
    };
    super_track(
        vm,
        &[("pinType", Value::Num(kind)), ("pinID", Value::Num(id))],
    );
    Ok(vec![])
}

fn clear_super_tracked_map_pin(vm: &mut Vm, _c: NativeCall) -> Ret {
    if !get(vm, "pinType").is_nil() {
        super_track(vm, &[]);
    }
    Ok(vec![])
}

/// `C_SuperTrack.GetSuperTrackedMapPin()`: its type and ID, else nothing
/// (recorded at load: nothing).
fn get_super_tracked_map_pin(vm: &mut Vm, _c: NativeCall) -> Ret {
    match (get(vm, "pinType"), get(vm, "pinID")) {
        (Value::Num(t), Value::Num(id)) => Ok(vec![Value::Num(t), Value::Num(id)]),
        _ => Ok(vec![]),
    }
}

fn is_super_tracking_map_pin(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = !get(vm, "pinType").is_nil();
    Ok(vec![Value::Bool(on)])
}

/// `C_SuperTrack.SetSuperTrackedContent(trackableType, trackableID)`.
fn set_super_tracked_content(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(kind), Some(id)) = (num_of(vm, c.arg(1)), num_of(vm, c.arg(2))) else {
        return Ok(vec![]);
    };
    super_track(
        vm,
        &[
            ("contentType", Value::Num(kind)),
            ("contentID", Value::Num(id)),
        ],
    );
    Ok(vec![])
}

fn clear_super_tracked_content(vm: &mut Vm, _c: NativeCall) -> Ret {
    if !get(vm, "contentType").is_nil() {
        super_track(vm, &[]);
    }
    Ok(vec![])
}

/// `C_SuperTrack.GetSuperTrackedContent()`: its type and ID, else nothing
/// (recorded at load: nothing).
fn get_super_tracked_content(vm: &mut Vm, _c: NativeCall) -> Ret {
    match (get(vm, "contentType"), get(vm, "contentID")) {
        (Value::Num(t), Value::Num(id)) => Ok(vec![Value::Num(t), Value::Num(id)]),
        _ => Ok(vec![]),
    }
}

fn is_super_tracking_content(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = !get(vm, "contentType").is_nil();
    Ok(vec![Value::Bool(on)])
}

/// `C_SuperTrack.SetSuperTrackedVignette(vignetteGUID)`.
fn set_super_tracked_vignette(vm: &mut Vm, c: NativeCall) -> Ret {
    match vm.hidden(c.arg(1)) {
        v @ Value::Str(_) => super_track(vm, &[("vignette", v)]),
        _ => {
            if !get(vm, "vignette").is_nil() {
                super_track(vm, &[]);
            }
        }
    }
    Ok(vec![])
}

/// `C_SuperTrack.GetSuperTrackedVignette()`: its GUID, else nothing
/// (recorded at load: nothing).
fn get_super_tracked_vignette(vm: &mut Vm, _c: NativeCall) -> Ret {
    // None tracked: nil (the full probe).
    Ok(match get(vm, "vignette") {
        v @ Value::Str(_) => vec![v],
        _ => vec![Value::Nil],
    })
}

/// `C_SuperTrack.ClearAllSuperTracked()`.
fn clear_all_super_tracked(vm: &mut Vm, _c: NativeCall) -> Ret {
    super_track(vm, &[]);
    Ok(vec![])
}

/// What is super-tracked, as `Enum.SuperTrackingType` (Quest 0,
/// UserWaypoint 1, Content 4, MapPin 6, Vignette 7), else none.
fn tracking_type(vm: &mut Vm) -> Option<f64> {
    if super_quest(vm).is_some_and(|q| on_quest(vm, q)) {
        Some(0.0)
    } else if get(vm, "waypoint").truthy() {
        Some(1.0)
    } else if !get(vm, "contentType").is_nil() {
        Some(4.0)
    } else if !get(vm, "pinType").is_nil() {
        Some(6.0)
    } else if !get(vm, "vignette").is_nil() {
        Some(7.0)
    } else {
        None
    }
}

/// `C_SuperTrack.GetHighestPrioritySuperTrackingType()`: what is
/// super-tracked (one thing at a time here), else nil.
fn get_highest_priority_type(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![tracking_type(vm).map_or(Value::Nil, Value::Num)])
}

fn is_super_tracking_anything(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = tracking_type(vm).is_some();
    Ok(vec![Value::Bool(on)])
}

// ------------------------------------------------------ the user's map pin

/// The user's map pin: its map, x, y and z (nil when not given).
fn waypoint(vm: &mut Vm) -> Option<(f64, f64, f64, Value)> {
    let Value::Table(t) = get(vm, "point") else {
        return None;
    };
    let (Value::Num(map), Value::Num(x), Value::Num(y)) = (
        field(vm, t, "uiMapID"),
        field(vm, t, "x"),
        field(vm, t, "y"),
    ) else {
        return None;
    };
    let z = field(vm, t, "z");
    Some((map, x, y, z))
}

fn set_waypoint(vm: &mut Vm, map: f64, x: f64, y: f64, z: Value) {
    let t = vm.new_table();
    set(vm, t, "uiMapID", Value::Num(map));
    set(vm, t, "x", Value::Num(x));
    set(vm, t, "y", Value::Num(y));
    set(vm, t, "z", z);
    put(vm, "point", Value::Table(t));
}

/// A `UiMapPoint`: `{ uiMapID, position = CreateVector2D(x, y), z }`
/// (Blizzard's `UiMapPoint.CreateFromCoordinates`).
fn map_point(
    vm: &mut Vm,
    map: f64,
    x: f64,
    y: f64,
    z: Value,
) -> Result<Value, wowlua_vm::LuaError> {
    let make = vm.get_global("CreateVector2D");
    let position = if make.is_nil() {
        let p = vm.new_table();
        set(vm, p, "x", Value::Num(x));
        set(vm, p, "y", Value::Num(y));
        Value::Table(p)
    } else {
        vm.call(make, &[Value::Num(x), Value::Num(y)])?
            .first()
            .copied()
            .unwrap_or(Value::Nil)
    };
    let t = vm.new_table();
    set(vm, t, "uiMapID", Value::Num(map));
    set(vm, t, "position", position);
    set(vm, t, "z", z);
    Ok(Value::Table(t))
}

/// A `UiMapPoint`'s map, x, y, z.
fn read_point(vm: &mut Vm, v: Value) -> Option<(f64, f64, f64, Value)> {
    let Value::Table(t) = vm.hidden(v) else {
        return None;
    };
    let map = field(vm, t, "uiMapID");
    let map = num_of(vm, map)?;
    let Value::Table(p) = field(vm, t, "position") else {
        return None;
    };
    let (x, y) = (field(vm, p, "x"), field(vm, p, "y"));
    let (x, y) = (num_of(vm, x)?, num_of(vm, y)?);
    let z = match field(vm, t, "z") {
        Value::Num(z) => Value::Num(z),
        _ => Value::Nil,
    };
    Some((map, x, y, z))
}

/// Whether the map takes a pin (the recorded
/// `C_Map.CanSetUserWaypointOnMap`; a map not recorded: yes).
fn can_set_waypoint(vm: &mut Vm, map: f64) -> bool {
    let call = format!("C_Map.CanSetUserWaypointOnMap({})", map as i64);
    match crate::recorded::lookup_text(vm, &call) {
        Some(v) => v.first().is_some_and(|v| v.truthy()),
        None => map > 0.0,
    }
}

/// `C_Map.SetUserWaypoint(point)`: the pin set where the map takes one,
/// `USER_WAYPOINT_UPDATED()`; whether it was set.
/// A point that is not one, on a map that takes no pin, or off its map
/// (outside 0 to 1) is refused with the usage (the full probe, 1.60.1:
/// `"x"`, map 0 and (2, 2) on 1413; other maps without pins unverified).
fn set_user_waypoint(vm: &mut Vm, c: NativeCall) -> Ret {
    let refuse = |vm: &mut Vm| {
        Err(vm.api_arg_error(1, "Usage: local wasSet = C_Map.SetUserWaypoint(point)"))
    };
    let Some((map, x, y, z)) = read_point(vm, c.arg(1)) else {
        return refuse(vm);
    };
    if !can_set_waypoint(vm, map) || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return refuse(vm);
    }
    set_waypoint(vm, map, x, y, z);
    fire(vm, "USER_WAYPOINT_UPDATED", &[]);
    Ok(vec![Value::Bool(true)])
}

/// `C_Map.ClearUserWaypoint()`: the pin gone, `USER_WAYPOINT_UPDATED()`,
/// then its super-tracking (recorded once, scenario mapnav: the pin
/// ctrl-clicked away, the update before `SUPER_TRACKING_CHANGED`).
fn clear_user_waypoint(vm: &mut Vm, _c: NativeCall) -> Ret {
    if waypoint(vm).is_none() {
        return Ok(vec![]);
    }
    put(vm, "point", Value::Nil);
    fire(vm, "USER_WAYPOINT_UPDATED", &[]);
    if get(vm, "waypoint").truthy() {
        super_track(vm, &[]);
    }
    Ok(vec![])
}

/// `C_Map.GetUserWaypointPositionForMap(uiMapID)`: the pin's place on that
/// map (a `vector2`): its own map's point, else through its map's
/// rectangle on that one (`C_Map.GetMapRectOnMap`, recorded; unverified),
/// else nothing.
fn get_user_waypoint_position_for_map(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some((map, x, y, _)), Some(on)) = (waypoint(vm), num_of(vm, c.arg(1))) else {
        return Ok(vec![]);
    };
    if map == on {
        return Ok(vec![crate::methods2::vector2(vm, x, y)]);
    }
    let rect = crate::recorded::lookup_text(
        vm,
        &format!("C_Map.GetMapRectOnMap({}, {})", map as i64, on as i64),
    );
    let Some(
        [
            Value::Num(x0),
            Value::Num(x1),
            Value::Num(y0),
            Value::Num(y1),
        ],
    ) = rect.as_deref()
    else {
        return Ok(vec![]);
    };
    if x1 <= x0 || y1 <= y0 {
        return Ok(vec![]);
    }
    let (px, py) = (x0 + x * (x1 - x0), y0 + y * (y1 - y0));
    Ok(vec![crate::methods2::vector2(vm, px, py)])
}

/// `C_Map.GetUserWaypoint()`: the pin, else nothing; its position a plain
/// `{x, y}` (the full probe, 1.60.1: no vector methods; Blizzard's
/// waypoint pin reads `position.x`).
fn get_user_waypoint(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((map, x, y, z)) = waypoint(vm) else {
        return Ok(vec![]);
    };
    let p = vm.new_table();
    set(vm, p, "x", Value::Num(x));
    set(vm, p, "y", Value::Num(y));
    let t = vm.new_table();
    set(vm, t, "uiMapID", Value::Num(map));
    set(vm, t, "position", Value::Table(p));
    set(vm, t, "z", z);
    Ok(vec![Value::Table(t)])
}

fn has_user_waypoint(vm: &mut Vm, _c: NativeCall) -> Ret {
    let has = waypoint(vm).is_some();
    Ok(vec![Value::Bool(has)])
}

/// `C_Map.GetUserWaypointHyperlink()`: the pin as a chat link,
/// `|cffffff00|Hworldmap:MAP:X:Y|h[MAP_PIN_HYPERLINK]|h|r` with x and y in
/// ten-thousandths (the retail client's form; unverified on Forever).
fn get_user_waypoint_hyperlink(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((map, x, y, _)) = waypoint(vm) else {
        return Ok(vec![]);
    };
    let label = vm.get_global("MAP_PIN_HYPERLINK");
    let label = text_of(vm, label)
        .unwrap_or_else(|| "|A:Waypoint-MapPin-ChatIcon:13:13:0:0|a Map Pin Location".into());
    let link = format!(
        "|cffffff00|Hworldmap:{}:{}:{}|h[{label}]|h|r",
        map as i64,
        (x * 10000.0).floor() as i64,
        (y * 10000.0).floor() as i64
    );
    Ok(vec![vm.str(&link)])
}

/// `C_Map.GetUserWaypointFromHyperlink(hyperlink)`: the point a
/// `worldmap:MAP:X:Y` link names, else nothing; its coordinates C floats
/// (SuuUI probe marks: `5657:3315` gave x 0.56569999456406, y
/// 0.33149999380112).
fn get_user_waypoint_from_hyperlink(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(link) = text_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let Some(rest) = link.split("worldmap:").nth(1) else {
        return Ok(vec![]);
    };
    let parts: Vec<f64> = rest
        .split(['|', ':'])
        .take(3)
        .map_while(|p| p.parse::<f64>().ok())
        .collect();
    let &[map, x, y] = &parts[..] else {
        return Ok(vec![]);
    };
    Ok(vec![map_point(
        vm,
        map,
        f64::from((x / 10000.0) as f32),
        f64::from((y / 10000.0) as f32),
        Value::Nil,
    )?])
}

/// A recorded map's `C_Map.GetMapInfo` answer.
fn recorded_map_info(vm: &mut Vm, map: i64) -> Option<Value> {
    let v = crate::recorded::lookup_text(vm, &format!("C_Map.GetMapInfo({map})"))?;
    match v.first().copied() {
        Some(t @ Value::Table(_)) => Some(t),
        _ => None,
    }
}

/// `C_Map.GetMapInfoAtPosition(uiMapID, x, y)`: the smallest of the map's
/// recorded children whose rectangle on it (`C_Map.GetMapRectOnMap`) holds
/// the point, else the map itself (unverified: the client's zone shapes are
/// not rectangles; rectangles of neighbours overlap).
fn get_map_info_at_position(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(map), Some(x), Some(y)) = (
        num_of(vm, c.arg(1)),
        num_of(vm, c.arg(2)),
        num_of(vm, c.arg(3)),
    ) else {
        return Ok(vec![]);
    };
    // A point on the map's edge or off it: nothing (the full probe: 0, 1,
    // -1, 1.5 and 2147483647 on either axis).
    if !(x > 0.0 && x < 1.0 && y > 0.0 && y < 1.0) {
        return Ok(vec![]);
    }
    let map = map as i64;
    let mut best: Option<(f64, Value)> = None;
    let children = crate::recorded::lookup_text(vm, &format!("C_Map.GetMapChildrenInfo({map})"));
    if let Some(Value::Table(list)) = children.and_then(|v| v.first().copied()) {
        let entries: Vec<(Value, Value)> = vm.heap.table(list).entries().collect();
        for (_, child) in entries {
            let Value::Table(child) = child else { continue };
            let Value::Num(id) = field(vm, child, "mapID") else {
                continue;
            };
            let rect = crate::recorded::lookup_text(
                vm,
                &format!("C_Map.GetMapRectOnMap({}, {map})", id as i64),
            );
            let Some(
                [
                    Value::Num(x0),
                    Value::Num(x1),
                    Value::Num(y0),
                    Value::Num(y1),
                ],
            ) = rect.as_deref()
            else {
                continue;
            };
            let (x0, x1, y0, y1) = (*x0, *x1, *y0, *y1);
            if x < x0 || x > x1 || y < y0 || y > y1 {
                continue;
            }
            let area = (x1 - x0) * (y1 - y0);
            if best.as_ref().is_none_or(|(a, _)| area < *a) {
                best = Some((area, Value::Table(child)));
            }
        }
    }
    if let Some((_, child)) = best {
        let Value::Table(child) = child else {
            return Ok(vec![]);
        };
        let id = match field(vm, child, "mapID") {
            Value::Num(n) => n as i64,
            _ => 0,
        };
        return Ok(vec![
            recorded_map_info(vm, id).unwrap_or(Value::Table(child)),
        ]);
    }
    Ok(match recorded_map_info(vm, map) {
        Some(info) => vec![info],
        None => vec![],
    })
}

// --------------------------------------------------------------- gossip

/// The open gossip's NPC and the quests it lists under `key`
/// (`availableQuests`), their IDs.
fn gossip_quests(vm: &mut Vm, key: &str) -> Option<(units::UnitId, Vec<i64>)> {
    let npc = engine(vm).world.gossip.open?;
    let t = engine(vm).world.gossip.defined.get(&npc.0).copied()?;
    let Value::Table(list) = field(vm, t, key) else {
        return Some((npc, vec![]));
    };
    let n = vm.heap.table(list).border();
    let mut ids = Vec::new();
    for i in 1..=n {
        if let Value::Table(q) = vm.heap.table(list).get(Value::Num(i as f64))
            && let Value::Num(id) = field(vm, q, "questID")
        {
            ids.push(id as i64);
        }
    }
    Some((npc, ids))
}

/// A quest picked from a gossip: a round trip later the gossip closes
/// (`GOSSIP_CLOSED(true)`, as an option's) and the quest's dialog opens.
fn gossip_quest_picked(vm: &mut Vm, npc: units::UnitId, quest: i64, progress: bool) {
    let delay = crate::clock::server_delay(vm);
    let later = vm.native_with(
        "gossipQuest",
        gossip_quest_opened as NativeFn,
        vec![
            Value::Num(npc.0 as f64),
            Value::Num(quest as f64),
            Value::Bool(progress),
        ],
    );
    crate::clock::after_engine(vm, delay, later);
}

fn gossip_quest_opened(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(Value::Num(npc)), Some(Value::Num(quest))) =
        (c.upvals.first().copied(), c.upvals.get(1).copied())
    else {
        return Ok(vec![]);
    };
    let progress = c.upvals.get(2).is_some_and(|v| v.truthy());
    let npc = units::UnitId(npc as usize);
    if engine(vm).world.gossip.open != Some(npc) {
        return Ok(vec![]);
    }
    engine_mut(vm).world.gossip.open = None;
    fire(vm, "GOSSIP_CLOSED", &[Value::Bool(true)]);
    let panel = if progress {
        crate::quests::Panel::Progress
    } else {
        crate::quests::Panel::Detail
    };
    crate::quests::open(vm, npc, quest as i64, panel);
    Ok(vec![])
}

/// `C_GossipInfo.SelectAvailableQuest(questID)`: a quest the gossip offers
/// (and the test defined) opens its details.
fn select_available_quest(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(quest) = num_of(vm, c.arg(1)).map(|n| n as i64) else {
        return Ok(vec![]);
    };
    let Some((npc, offered)) = gossip_quests(vm, "availableQuests") else {
        return Ok(vec![]);
    };
    if offered.contains(&quest) && defined(vm, quest).is_some() {
        gossip_quest_picked(vm, npc, quest, false);
    }
    Ok(vec![])
}

/// `C_GossipInfo.SelectActiveQuest(questID)`: a quest in the log the
/// gossip's NPC takes back (its `activeQuests`, or a defined quest it
/// gave) opens its progress.
fn select_active_quest(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(quest) = num_of(vm, c.arg(1)).map(|n| n as i64) else {
        return Ok(vec![]);
    };
    let Some((npc, active)) = gossip_quests(vm, "activeQuests") else {
        return Ok(vec![]);
    };
    let giver = match defined(vm, quest) {
        Some(t) => {
            let v = field(vm, t, "giver");
            units::handle_of(vm, v) == Some(npc)
        }
        None => false,
    };
    if (active.contains(&quest) || giver) && defined(vm, quest).is_some() && on_quest(vm, quest) {
        gossip_quest_picked(vm, npc, quest, true);
    }
    Ok(vec![])
}

// --------------------------------------------------------- professions

/// The recipes tracked (`isRecraft` apart), in order.
fn tracked(vm: &mut Vm, recraft: bool) -> BTreeSet<i64> {
    let t = kept_table(vm, if recraft { "trackedRecraft" } else { "tracked" });
    vm.heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match k {
            Value::Num(n) if v.truthy() => Some(n as i64),
            _ => None,
        })
        .collect()
}

/// `C_TradeSkillUI.GetRecipesTracked(isRecraft)`.
fn get_recipes_tracked(vm: &mut Vm, c: NativeCall) -> Ret {
    let recraft = c.arg(1).truthy();
    let ids: Vec<Value> = tracked(vm, recraft)
        .into_iter()
        .map(|id| Value::Num(id as f64))
        .collect();
    Ok(vec![list_of(vm, &ids)])
}

/// `C_TradeSkillUI.SetRecipeTracked(recipeID, tracked, isRecraft)`:
/// `TRACKED_RECIPE_UPDATE(recipeID, tracked)` when it changed.
fn set_recipe_tracked(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = num_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let on = c.arg(2).truthy();
    let recraft = c.arg(3).truthy();
    let was = tracked(vm, recraft).contains(&(id as i64));
    if was == on {
        return Ok(vec![]);
    }
    let t = kept_table(vm, if recraft { "trackedRecraft" } else { "tracked" });
    let v = if on { Value::Bool(true) } else { Value::Nil };
    let _ = vm.heap.table_mut(t).set(Value::Num(id), v);
    fire(
        vm,
        "TRACKED_RECIPE_UPDATE",
        &[Value::Num(id), Value::Bool(on)],
    );
    Ok(vec![])
}

/// `C_TradeSkillUI.GetRecipeItemLink(recipeID)`: the link of what the
/// recipe makes (its `GetRecipeOutputItemData`'s `hyperlink`, the
/// professions' recorded answer); else the recipe's spell as an enchant
/// link and its ID (`|cffffd000|Henchant:1|h[Word of Recall (OLD)]|h|r`,
/// 1), nothing for no spell (the full probe: spells 1 and 6462, IDs -1, 0
/// and 2 nothing).
fn get_recipe_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = num_of(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let ns = vm.get_global("C_TradeSkillUI");
    let Value::Table(ns) = ns else {
        return Ok(vec![Value::Nil]);
    };
    let f = field(vm, ns, "GetRecipeOutputItemData");
    if !f.is_nil()
        && let Some(Value::Table(t)) = vm.call(f, &[Value::Num(id)])?.first().copied()
    {
        let link = field(vm, t, "hyperlink");
        if !link.is_nil() {
            return Ok(vec![link]);
        }
    }
    let id = id.trunc() as i64;
    let spell = crate::gamedata::spell_name(vm, id);
    let Some(name) = text_of(vm, spell) else {
        return Ok(vec![]);
    };
    let link = format!("|cffffd000|Henchant:{id}|h[{name}]|h|r");
    Ok(vec![vm.str(&link), Value::Num(id as f64)])
}

/// `C_TradeSkillUI.IsRecipeRepeating()`: a craft of several running
/// (unverified).
fn is_recipe_repeating(vm: &mut Vm, _c: NativeCall) -> Ret {
    let repeating = engine(vm)
        .world
        .tradeskill
        .craft
        .as_ref()
        .is_some_and(|c| c.count > 1 && c.left > 0);
    Ok(vec![Value::Bool(repeating)])
}

/// `C_TradeSkillUI.GetRecipeItemLevelFilter()`: no filter, 0 and 0.
fn zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0)])
}

// ------------------------------------------------------------- generic

/// A documented function's empty answer (its documentation's empty values:
/// nil, false, 0, "", an empty table or structure; nothing where it may
/// return nothing): the state of a system the engine's world does not
/// have.
fn documented_empty(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(i)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let api = engine(vm).api;
    let Some(f) = api.functions.get(i as usize) else {
        return Ok(vec![]);
    };
    Ok(crate::defaults(vm, api, &f.doc))
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn none(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn yes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// The obliterum forge's spell (Obliterate, 193024: the full probe).
fn obliterate_spell_id(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(193024.0)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `GetQuestSortIndex(questIndex)`: the log entry's header's place among
/// the log's headers, counting the entry itself if a header; 0 past the
/// log or below its start (the full probe: the header at 1 and its quest
/// at 2 answered 1; -1, 0 and 1063 0).
fn get_quest_sort_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = vm.tonumber(vm.hidden(c.arg(1))).unwrap_or(0.0).trunc() as i64;
    let mut headers = 0.0;
    for i in 1..=index.max(0) {
        let Some(info) = stat_at(vm, "questLogInfo", i).and_then(|a| a.first().cloned()) else {
            return Ok(vec![Value::Num(0.0)]);
        };
        if matches!(answer_field(&info, "isHeader"), Some(Answer::Bool(true))) {
            headers += 1.0;
        }
    }
    Ok(vec![Value::Num(headers)])
}

/// `GetQuestExpansion(questID)`: -2 for any quest (Forever's carry no
/// expansion), 0 for an ID below 1 (the full probe: 1, 2 and the log's
/// 1063 -2; 0 and -1 0; a quest of an expansion unverified).
fn get_quest_expansion(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.tonumber(vm.hidden(c.arg(1))).unwrap_or(0.0);
    Ok(vec![Value::Num(if id >= 1.0 { -2.0 } else { 0.0 })])
}

fn empty_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

/// `C_QuestLog.GetRequiredMoney(questID)`: -1, no money asked (SuuUI
/// probe quest: all 23 quests of a log; a quest asking money unverified).
fn get_required_money(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(-1.0)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    let mut out: Vec<(&'static str, NativeFn)> = vec![
        ("C_QuestLog.GetRequiredMoney", get_required_money),
        // The quest log's rewards and texts (the defined quests').
        ("GetQuestLogRewardXP", get_quest_log_reward_xp),
        ("GetQuestLogRewardMoney", get_quest_log_reward_money),
        ("GetNumQuestLogRewards", get_num_quest_log_rewards),
        ("GetNumQuestLogChoices", get_num_quest_log_choices),
        ("GetQuestLogRewardInfo", get_quest_log_reward_info),
        ("GetQuestLogChoiceInfo", get_quest_log_choice_info),
        ("GetQuestLogItemLink", get_quest_log_item_link),
        ("GetQuestLogQuestText", get_quest_log_quest_text),
        ("GetQuestObjectiveInfo", get_quest_objective_info),
        ("GetQuestLink", get_quest_link),
        ("GetBindLocation", get_bind_location),
        (
            "C_QuestLog.GetQuestLogPortraitGiver",
            quest_log_portrait_giver,
        ),
        ("C_QuestLog.GetQuestType", get_quest_type),
        ("GetQuestLogPortraitTurnIn", log_portrait_turn_in),
        // What no quest in the engine's world rewards or has (as the
        // dialog's own answers, `quests.rs`; unverified): no artifact
        // power, honor, skill points, title, criteria spell, timer,
        // factions, currencies, item drops, progress bar, treasure picker,
        // reward packs; classic quests (expansion 0).
        ("GetQuestLogRewardArtifactXP", zeros),
        ("GetQuestLogRewardHonor", zero),
        ("GetQuestLogRewardSkillPoints", nothing),
        ("GetQuestLogRewardTitle", nothing),
        ("GetQuestLogCriteriaSpell", nothing),
        ("GetQuestLogTimeLeft", none),
        ("GetNumQuestLogRewardFactions", zero),
        ("GetQuestLogRewardFactionInfo", nothing),
        ("GetNumQuestLogTasks", zero),
        ("GetNumQuestItemDrops", zero),
        ("GetQuestLogItemDrop", nothing),
        // No currency for a reward asked properly: 0 (the full probe:
        // `(1, 1)`).
        ("GetQuestCurrencyID", zero),
        ("GetQuestLogChoiceInfoLootType", zero),
        ("GetQuestProgressBarPercent", zero),
        ("GetQuestPOIBlobCount", zero),
        // A quest not in the log: nothing (the full probe; a quest's own
        // faction unverified, the probe's second quest answered 2).
        ("GetQuestFactionGroup", nothing),
        ("GetQuestExpansion", get_quest_expansion),
        ("GetQuestSortIndex", get_quest_sort_index),
        ("GetQuestLogQuestType", none),
        ("QuestLogShouldShowPortrait", no),
        ("QuestLogRewardHasTreasurePicker", no),
        ("IsQuestIDValidSpellTarget", no),
        ("IsQuestLogSpecialItemInRange", none),
        ("GetRewardNumSkillUps", zero),
        ("GetRewardSkillLineID", zero),
        ("GetRewardPackArtifactPower", zero),
        ("GetRewardPackCurrencies", empty_list),
        ("GetRewardPackItems", empty_list),
        ("GetRewardPackMoney", zero),
        ("GetRewardPackTitle", none),
        ("GetRewardPackTitleName", none),
        // Sharing a quest: no group member takes it (unverified).
        ("QuestLogPushQuest", nothing),
        // C_QuestLog from the log.
        ("C_QuestLog.IsOnQuest", is_on_quest),
        ("C_QuestLog.IsUnitOnQuest", is_unit_on_quest),
        ("C_QuestLog.ReadyForTurnIn", ready_for_turn_in),
        ("C_QuestLog.GetNumQuestObjectives", get_num_quest_objectives),
        ("C_QuestLog.GetTitleForLogIndex", get_title_for_log_index),
        (
            "C_QuestLog.GetHeaderIndexForQuest",
            get_header_index_for_quest,
        ),
        ("C_QuestLog.IsOnMap", is_on_map),
        (
            "C_QuestLog.DoesQuestAwardReputationWithFaction",
            does_quest_award_reputation,
        ),
        (
            "C_QuestLog.IsQuestFlaggedCompletedOnAccount",
            is_completed_on_account,
        ),
        (
            "C_QuestLog.GetAllCompletedQuestIDs",
            get_all_completed_quest_ids,
        ),
        (
            "C_QuestLog.GetMaxNumQuestsCanAccept",
            get_max_num_quests_can_accept,
        ),
        (
            "C_QuestLog.ShouldShowQuestRewards",
            should_show_quest_rewards,
        ),
        ("C_QuestLog.SetSelectedQuest", set_selected_quest),
        ("C_QuestLog.AddQuestWatch", add_quest_watch),
        ("C_QuestLog.RemoveQuestWatch", remove_quest_watch),
        ("C_QuestLog.SetAbandonQuest", set_abandon_quest),
        ("C_QuestLog.GetAbandonQuest", get_abandon_quest),
        ("C_QuestLog.AbandonQuest", abandon_quest),
        ("C_QuestLog.SetMapForQuestPOIs", set_map_for_quest_pois),
        ("C_QuestLog.GetMapForQuestPOIs", get_map_for_quest_pois),
        ("C_QuestLog.RequestLoadQuestByID", request_load_quest),
        // Lists that may be documented as returning nothing: empty
        // (Blizzard's code iterates them).
        ("C_QuestLog.GetAbandonQuestItems", empty_list),
        // None: nothing, whatever the quest (the full probe; Blizzard's
        // quest info tests it for nil).
        (
            "C_QuestLog.GetQuestLogMajorFactionReputationRewards",
            nothing,
        ),
        (
            "C_QuestInfoSystem.GetQuestRewardCurrencies",
            quest_reward_list,
        ),
        ("C_QuestInfoSystem.GetQuestRewardSpells", quest_reward_list),
        // An option's widgets: none (the full probe, any option).
        (
            "C_GossipInfo.GetOptionUIWidgetSetsAndTypesByOptionID",
            empty_list,
        ),
        (
            "C_QuestOffer.GetQuestOfferMajorFactionReputationRewards",
            empty_list,
        ),
        // Super-tracking.
        (
            "C_SuperTrack.GetSuperTrackedQuestID",
            get_super_tracked_quest_id,
        ),
        (
            "C_SuperTrack.SetSuperTrackedQuestID",
            set_super_tracked_quest_id,
        ),
        ("C_SuperTrack.IsSuperTrackingQuest", is_super_tracking_quest),
        (
            "C_SuperTrack.SetSuperTrackedUserWaypoint",
            set_super_tracked_user_waypoint,
        ),
        (
            "C_SuperTrack.IsSuperTrackingUserWaypoint",
            is_super_tracking_user_waypoint,
        ),
        (
            "C_SuperTrack.SetSuperTrackedMapPin",
            set_super_tracked_map_pin,
        ),
        (
            "C_SuperTrack.ClearSuperTrackedMapPin",
            clear_super_tracked_map_pin,
        ),
        (
            "C_SuperTrack.GetSuperTrackedMapPin",
            get_super_tracked_map_pin,
        ),
        (
            "C_SuperTrack.IsSuperTrackingMapPin",
            is_super_tracking_map_pin,
        ),
        (
            "C_SuperTrack.SetSuperTrackedContent",
            set_super_tracked_content,
        ),
        (
            "C_SuperTrack.ClearSuperTrackedContent",
            clear_super_tracked_content,
        ),
        (
            "C_SuperTrack.GetSuperTrackedContent",
            get_super_tracked_content,
        ),
        (
            "C_SuperTrack.IsSuperTrackingContent",
            is_super_tracking_content,
        ),
        (
            "C_SuperTrack.SetSuperTrackedVignette",
            set_super_tracked_vignette,
        ),
        (
            "C_SuperTrack.GetSuperTrackedVignette",
            get_super_tracked_vignette,
        ),
        ("C_SuperTrack.ClearAllSuperTracked", clear_all_super_tracked),
        (
            "C_SuperTrack.GetHighestPrioritySuperTrackingType",
            get_highest_priority_type,
        ),
        (
            "C_SuperTrack.IsSuperTrackingAnything",
            is_super_tracking_anything,
        ),
        // No corpse arrow, no item, no path (unverified).
        ("C_SuperTrack.GetNextWaypointForMap", nothing),
        // The user's map pin.
        ("C_Map.SetUserWaypoint", set_user_waypoint),
        ("C_Map.ClearUserWaypoint", clear_user_waypoint),
        ("C_Map.GetUserWaypoint", get_user_waypoint),
        ("C_Map.HasUserWaypoint", has_user_waypoint),
        (
            "C_Map.GetUserWaypointHyperlink",
            get_user_waypoint_hyperlink,
        ),
        (
            "C_Map.GetUserWaypointFromHyperlink",
            get_user_waypoint_from_hyperlink,
        ),
        ("C_Map.GetMapInfoAtPosition", get_map_info_at_position),
        ("C_Navigation.GetFrame", get_navigation_frame),
        (
            "C_Map.GetUserWaypointPositionForMap",
            get_user_waypoint_position_for_map,
        ),
        // Gossip quests.
        ("C_GossipInfo.SelectAvailableQuest", select_available_quest),
        ("C_GossipInfo.SelectActiveQuest", select_active_quest),
        // Professions: tracked recipes, links, filters the windows'
        // recorded answers keep (setting them changes nothing there).
        ("C_TradeSkillUI.GetRecipesTracked", get_recipes_tracked),
        ("C_TradeSkillUI.SetRecipeTracked", set_recipe_tracked),
        ("C_TradeSkillUI.GetRecipeItemLink", get_recipe_item_link),
        ("C_TradeSkillUI.IsRecipeRepeating", is_recipe_repeating),
        ("C_TradeSkillUI.GetRecipeItemLevelFilter", zeros),
        ("C_TradeSkillUI.IsRecipeSearchInProgress", no),
        ("C_TradeSkillUI.CanObliterateCursorItem", no),
        // No window open: nothing (the full probe); the obliterum forge's
        // spell (193024) and no item pending; a recraft's reagents valid,
        // no quality reagent link, no profession for an ability (the full
        // probe, whatever the arguments).
        ("C_TradeSkillUI.IsRecipeCategoryFiltered", nothing),
        ("C_TradeSkillUI.GetAllFilterableInventorySlots", nothing),
        ("C_TradeSkillUI.GetFilterableInventorySlots", nothing),
        ("C_TradeSkillUI.GetObliterateSpellID", obliterate_spell_id),
        ("C_TradeSkillUI.GetPendingObliterateItemID", nothing),
        ("C_TradeSkillUI.GetPendingObliterateItemLink", nothing),
        ("C_TradeSkillUI.IsRecraftReagentValid", yes),
        ("C_TradeSkillUI.GetRecipeQualityReagentLink", none),
        ("C_TradeSkillUI.GetProfessionNameForSkillLineAbility", none),
        ("C_TradeSkillUI.GetSubCategories", nothing),
    ];
    // Accepted, without an effect the engine shows (filters the recorded
    // windows answer, actions of systems Forever lacks).
    for name in [
        "C_TradeSkillUI.SetShowLearned",
        "C_TradeSkillUI.SetShowUnlearned",
        "C_TradeSkillUI.SetRecipeFavorite",
        "C_TradeSkillUI.SetInventorySlotFilter",
        "C_TradeSkillUI.ClearInventorySlotFilter",
        "C_TradeSkillUI.SetOnlyShowFirstCraftRecipes",
        "C_TradeSkillUI.SetOnlyShowMakeableRecipes",
        "C_TradeSkillUI.SetOnlyShowSkillUpRecipes",
        "C_TradeSkillUI.SetOnlyShowAvailableForOrders",
        "C_TradeSkillUI.ClearRecipeCategoryFilter",
        "C_TradeSkillUI.ClearRecipeSourceTypeFilter",
        "C_TradeSkillUI.SetRecipeCategoryFilter",
        "C_TradeSkillUI.SetRecipeSourceTypeFilter",
        "C_TradeSkillUI.SetSourceTypeFilter",
        "C_TradeSkillUI.SetRecipeItemLevelFilter",
        "C_TradeSkillUI.SetRecipeItemNameFilter",
        "C_TradeSkillUI.SetProfessionChildSkillLineID",
        "C_TradeSkillUI.ClearPendingObliterateItem",
        "C_TradeSkillUI.CloseObliterumForge",
        "C_TradeSkillUI.DropPendingObliterateItemFromCursor",
        "C_TradeSkillUI.ObliterateItem",
        "C_TradeSkillUI.StopRecipeRepeat",
        "C_TradeSkillUI.OpenRecipe",
        "C_TradeSkillUI.CancelProfessionRespec",
        "C_TradeSkillUI.ConfirmProfessionRespec",
        "C_TradeSkillUI.CraftEnchant",
        "C_TradeSkillUI.CraftSalvage",
        "C_QuestLog.SortQuestWatches",
        "C_QuestLog.UpdateCampaignHeaders",
        "C_QuestLine.RequestQuestLinesForMap",
        "C_QuestSession.RequestSessionStart",
        "C_QuestSession.RequestSessionStop",
        "C_QuestSession.SendSessionBeginResponse",
        "C_QuestSession.SetQuestIsSuperTracked",
        "C_TaskQuest.RequestPreloadRewardData",
        "C_GossipInfo.RefreshOptions",
        "C_Map.CloseWorldMapInteraction",
        "C_Map.RequestPreloadMap",
        "C_CraftingOrders.CancelOrder",
        "C_CraftingOrders.RejectOrder",
        "C_CraftingOrders.SetCustomerOptionFavorited",
        "C_CraftingOrders.ClaimOrder",
        "C_CraftingOrders.CloseCrafterCraftingOrders",
        "C_CraftingOrders.CloseCustomerCraftingOrders",
        "C_CraftingOrders.FulfillOrder",
        "C_CraftingOrders.ListMyOrders",
        "C_CraftingOrders.OpenCrafterCraftingOrders",
        "C_CraftingOrders.OpenCustomerCraftingOrders",
        "C_CraftingOrders.ParseCustomerOptions",
        "C_CraftingOrders.PlaceNewOrder",
        "C_CraftingOrders.ReleaseOrder",
        "C_CraftingOrders.RequestCrafterOrders",
        "C_CraftingOrders.RequestCustomerOrders",
        "C_CraftingOrders.UpdateIgnoreList",
    ] {
        out.push((name, nothing));
    }
    // Systems Forever lacks or the engine's world has nothing of: their
    // documented empty answers (crafting orders, profession
    // specializations, world and task quests, bounties, threats, callings,
    // quest lines, sessions, hubs, offers' currencies, vignettes, area POI
    // timers, recrafting, obliterating, item qualities; recorded:
    // `C_CraftingOrders.SkillLineHasOrders` and `ShouldShowCraftingOrderTab`
    // false in every profession window).
    for name in [
        "C_TradeSkillUI.GetRecipeItemQualityInfo",
        "C_TradeSkillUI.OpenTradeSkill",
        "C_TradeSkillUI.IsOriginalCraftRecipeLearned",
        "C_TradeSkillUI.GetDependentReagents",
        "C_TradeSkillUI.GetOriginalCraftRecipeID",
        "C_TradeSkillUI.GetRecraftItems",
        "C_TradeSkillUI.GetSkillLineForGear",
        "C_TradeSkillUI.GetItemCraftedQualityByItemInfo",
        "C_TradeSkillUI.GetProfessionByInventorySlot",
        "C_TradeSkillUI.GetRecraftRemovalWarnings",
        "C_TradeSkillUI.CanStoreEnchantInItem",
        "C_TradeSkillUI.CheckRespecNPC",
        "C_TradeSkillUI.DoesRecraftingRecipeAcceptItem",
        "C_TradeSkillUI.GetCraftingOperationInfoForOrder",
        "C_TradeSkillUI.GetCraftingReagentBonusText",
        "C_TradeSkillUI.GetCraftingTargetItems",
        "C_TradeSkillUI.GetEnchantItems",
        "C_TradeSkillUI.GetItemReagentQualityByItemInfo",
        "C_TradeSkillUI.GetItemSlotModifications",
        "C_TradeSkillUI.GetItemSlotModificationsForOrder",
        "C_TradeSkillUI.GetProfessionForCursorItem",
        "C_TradeSkillUI.GetReagentDifficultyText",
        "C_TradeSkillUI.GetReagentSlotStatus",
        "C_TradeSkillUI.GetRecipeInfoForSkillLineAbility",
        "C_TradeSkillUI.GetSalvagableItemIDs",
        "C_TradeSkillUI.IsRecraftItemEquipped",
        "C_TradeSkillUI.RecraftLimitCategoryValid",
        "C_TradeSkillUI.RecraftRecipe",
        "C_TradeSkillUI.RecraftRecipeForOrder",
        "C_TradeSkillUI.IsEnchantTargetValid",
        "C_CraftingOrders.GetNumFavoriteCustomerOptions",
        "C_CraftingOrders.SkillLineHasOrders",
        "C_CraftingOrders.ShouldShowCraftingOrderTab",
        "C_CraftingOrders.AreOrderNotesDisabled",
        "C_CraftingOrders.GetClaimedOrder",
        "C_CraftingOrders.GetOrderClaimInfo",
        "C_CraftingOrders.HasFavoriteCustomerOptions",
        "C_CraftingOrders.IsCustomerOptionFavorited",
        "C_CraftingOrders.GetCustomerOrders",
        "C_CraftingOrders.OrderCanBeRecrafted",
        "C_CraftingOrders.CalculateCraftingOrderPostingFee",
        "C_CraftingOrders.CanOrderSkillAbility",
        "C_CraftingOrders.GetCrafterBuckets",
        "C_CraftingOrders.GetCrafterOrders",
        "C_CraftingOrders.GetCraftingOrderTime",
        "C_CraftingOrders.GetCustomerCategories",
        "C_CraftingOrders.GetCustomerOptions",
        "C_CraftingOrders.GetMyOrders",
        "C_CraftingOrders.GetDefaultOrdersSkillLine",
        "C_ProfSpecs.GetStateForTab",
        "C_ProfSpecs.GetSpendCurrencyForPath",
        "C_ProfSpecs.GetDefaultSpecSkillLine",
        "C_ProfSpecs.GetTabInfo",
        "C_ProfSpecs.GetUnlockEntryForPath",
        "C_ProfSpecs.GetDescriptionForPerk",
        "C_ProfSpecs.GetPerksForPath",
        "C_ProfSpecs.GetSourceTextForPath",
        "C_ProfSpecs.GetSpecTabInfo",
        "C_ProfSpecs.GetStateForPath",
        "C_ProfSpecs.GetStateForPerk",
        "C_ProfSpecs.GetUnlockRankForPerk",
        "C_ProfSpecs.GetChildrenForPath",
        "C_ProfSpecs.GetCurrencyInfoForSkillLine",
        "C_ProfSpecs.GetDescriptionForPath",
        "C_ProfSpecs.GetEntryIDForPerk",
        "C_ProfSpecs.GetSpecTabIDsForSkillLine",
        "C_ProfSpecs.GetSpendEntryForPath",
        "C_ProfSpecs.ShouldShowSpecTab",
        "C_ProfSpecs.CanRefundPath",
        "C_ProfSpecs.CanUnlockTab",
        "C_ProfSpecs.GetRootPathForTab",
        "C_QuestLog.GetQuestRewardCurrencies",
        "C_QuestLog.IsQuestCalling",
        "C_QuestLog.IsThreatQuest",
        "C_QuestLog.IsQuestCriteriaForBounty",
        "C_QuestLog.IsQuestTask",
        "C_QuestLog.IsQuestTrivial",
        "C_QuestLog.IsAccountQuest",
        "C_QuestLog.IsQuestBounty",
        "C_QuestLog.QuestCanHaveWarModeBonus",
        "C_QuestLog.GetQuestRewardCurrencyInfo",
        "C_QuestLog.IsQuestReplayable",
        "C_QuestLog.IsWorldQuest",
        "C_QuestLog.AddWorldQuestWatch",
        "C_QuestLog.RemoveWorldQuestWatch",
        "C_QuestLog.GetQuestIDForWorldQuestWatchIndex",
        "C_QuestLog.GetActivePreyQuest",
        "C_QuestLog.GetDistanceSqToQuest",
        "C_QuestLog.GetNextWaypoint",
        "C_QuestLog.GetNextWaypointForMap",
        "C_QuestLog.GetQuestAdditionalHighlights",
        "C_QuestLog.GetSuggestedGroupSize",
        "C_QuestLog.IsImportantQuest",
        "C_QuestLog.IsMetaQuest",
        "C_QuestLog.IsRepeatableQuest",
        "C_QuestLog.QuestContainsFirstTimeRepBonusForPlayer",
        "C_QuestLog.QuestHasQuestSessionBonus",
        "C_QuestLog.QuestHasWarModeBonus",
        "C_QuestLog.QuestIgnoresAccountCompletedFiltering",
        "C_QuestLog.ShouldDisplayTimeRemaining",
        "C_QuestLog.IsQuestFromContentPush",
        "C_QuestLog.IsQuestInvasion",
        "C_QuestLog.IsQuestReplayedRecently",
        "C_QuestLog.UnitIsRelatedToActiveQuest",
        "C_QuestInfoSystem.GetQuestLogRewardFavor",
        "C_QuestInfoSystem.GetQuestRewardSpellInfo",
        "C_QuestInfoSystem.HasQuestRewardCurrencies",
        "C_QuestInfoSystem.GetQuestShouldToastCompletion",
        "C_QuestInfoSystem.HasQuestRewardSpells",
        "C_QuestInfoSystem.GetQuestHasShortExpirationWarning",
        "C_QuestLine.GetQuestLineInfo",
        "C_QuestLine.GetQuestLineQuests",
        "C_QuestLine.IsComplete",
        "C_QuestLine.QuestLineIgnoresAccountCompletedFiltering",
        "C_QuestSession.CanStart",
        "C_QuestSession.GetProposedMaxLevelForSession",
        "C_QuestSession.HasJoined",
        "C_QuestSession.CanStop",
        "C_QuestSession.GetSuperTrackedQuest",
        "C_QuestSession.HasPendingCommand",
        "C_QuestHub.IsAreaPOICurrentlyRelatedToHub",
        "C_QuestHub.IsQuestCurrentlyRelatedToHub",
        "C_QuestOffer.GetHideRequiredItems",
        "C_QuestOffer.GetQuestRequiredCurrencyInfo",
        "C_QuestOffer.GetQuestRewardCurrencyInfo",
        "C_QuestItemUse.CanUseQuestItemOnObject",
        "C_SuperTrack.GetSuperTrackedItemName",
        "C_SuperTrack.IsSuperTrackingCorpse",
        "C_TaskQuest.GetQuestInfoByQuestID",
        "C_TaskQuest.GetQuestTimeLeftSeconds",
        "C_TaskQuest.GetQuestTimeLeftMinutes",
        "C_TaskQuest.GetQuestUIWidgetSetByType",
        "C_TaskQuest.GetQuestProgressBarInfo",
        "C_TaskQuest.GetQuestZoneID",
        "C_TaskQuest.GetQuestLocation",
        "C_TaskQuest.GetThreatQuests",
        "C_TaskQuest.IsActive",
        "C_AreaPoiInfo.GetAreaPOISecondsLeft",
        "C_AreaPoiInfo.IsAreaPOITimed",
        "C_AreaPoiInfo.GetDragonridingRacesForMap",
        "C_VignetteInfo.GetVignettePosition",
        "C_VignetteInfo.FindBestUniqueVignette",
        "C_VignetteInfo.GetHealthPercent",
        "C_VignetteInfo.GetRecommendedGroupSize",
        "C_VignetteInfo.GetVignetteInfo",
        "C_GossipInfo.GetFriendshipReputationRanks",
        "C_GossipInfo.ForceGossip",
        "C_GossipInfo.GetCompletedOptionDescriptionString",
        "C_GossipInfo.GetCustomGossipDescriptionString",
        "C_GossipInfo.GetPoiInfo",
        "C_Map.GetBountySetMaps",
        "C_Map.GetMapGroupMembersInfo",
        "C_Map.GetMapHighlightInfoAtPosition",
        "C_MapExplorationInfo.GetExploredAreaIDsAtPosition",
        "C_FogOfWar.GetFogOfWarInfo",
        "GetTaxiBenchmarkMode",
    ] {
        out.push((name, documented_empty));
    }
    out
}
