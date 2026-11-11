//! Group loot rolls (decision T629): an item the group rolls for, as the
//! server hands it out. `mock.startLootRoll({ item, count?, rollTime?, bop?,
//! canNeed?, canGreed?, reasonNeed?, reasonGreed?, group?, rolls?, loot?,
//! encounterID?, encounterName? })` starts one: `START_LOOT_ROLL(rollID,
//! rollTime in ms, lootHandle)`; `GetLootRollItemInfo`, `GetLootRollItemLink`,
//! `GetLootRollTimeLeft` (the milliseconds left, on the clock),
//! `C_Loot.GetLootRollDuration` and `GetActiveLootRollIDs` (the rolls the
//! player has yet to choose on) answer for it till it ends.
//!
//! The player chooses with `RollOnLoot(rollID, rollType)` (0 Pass, 1 Need,
//! 2 Greed: WoW Forever has no Disenchant or Transmog roll). A Bind on
//! Pickup item's Need or Greed asks first, from inside the call
//! (`CONFIRM_LOOT_ROLL(rollID, rollType, confirmReason)`, Blizzard's popup
//! whose OK calls `ConfirmLootRoll(rollID, rollType)`); a choice made, the
//! client is done with the roll (`CANCEL_LOOT_ROLL(rollID)`, from inside
//! the call: SuuUI's tray, found in game) and the server tells it a round
//! trip later ("You have selected Need for: [item]"). The others' choices
//! are the test's (`mock.lootRollChoice(rollID, name, choice, roll?)`), told
//! at once ("Thrall has selected Greed for: [item]").
//!
//! Everyone chosen, or the time out (the player not chosen:
//! `CANCEL_LOOT_ROLL(rollID)`), the roll ends: Need beats Greed, each of the
//! best choice's rollers rolls 1 to 100 (the test's number, `rolls`, else
//! the engine's own generator) and the highest wins (the first to choose of
//! a tie): "Need Roll - 91 for [item] by Thrall", "Thrall won: [item]" (the
//! player's "You won: [item]", then the item into the bags with "You
//! receive loot: [item]"), or "Everyone passed on: [item]". Every roll of a
//! loot ended: `LOOT_ROLLS_COMPLETE(lootHandle)`. `mock.cancelLootRoll(id?)`
//! takes one roll back (`CANCEL_LOOT_ROLL`) or all (`CANCEL_ALL_LOOT_ROLLS`),
//! without a result.
//!
//! The loot history (`C_LootHistory`) keeps the drops by encounter (the
//! roll's `encounterID`, the encounter under way, else 0), each numbered
//! from 1 (`lootListKey`): who chose what, the numbers rolled, the winner
//! or everyone passed; `LOOT_HISTORY_UPDATE_ENCOUNTER(encounterID)` when a
//! drop comes, `LOOT_HISTORY_UPDATE_DROP(encounterID, lootListKey)` when it
//! changes. The chat lines are the client's (`LOOT_ROLL_*`, their loot
//! history link naming the encounter).
//!
//! Unverified (no recording of a roll yet: `tests/ingame/PROBES.md`, "Group
//! loot rolls"): roll IDs and loot handles (numbered from 1 here), the
//! confirmation's reason (empty here), the lines' sender (none), the order
//! of the result's lines and events, the player's own roll line (by the
//! player's name here), the lines for a time-out (none for who did not
//! choose), whether a choice the item does not allow does nothing (here),
//! `GetLootRollItemInfo`'s Disenchant values (false, 0, 0) and its answer
//! after the player chose (the item's still), the loot history's encounter
//! for a roll outside one, its times (seconds on `GetTime`'s clock) and
//! order, `LOOT_ROLL_STARTED`'s line and `MAIN_SPEC_NEED_ROLL` (neither
//! sent here).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, units, widgets};

/// A roll's time when the test gives none (seconds).
const ROLL_TIME: f64 = 60.0;
/// A group member's class when the test names them only.
const CLASS: &str = "WARRIOR";

/// A choice on a roll.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Need,
    Greed,
    Pass,
}

impl Choice {
    /// `RollOnLoot`'s roll type (0 Pass, 1 Need, 2 Greed).
    fn of_type(t: f64) -> Option<Choice> {
        [Choice::Pass, Choice::Need, Choice::Greed]
            .into_iter()
            .find(|ch| ch.roll_type() == t)
    }

    fn of_name(s: &str) -> Option<Choice> {
        match s {
            "need" => Some(Choice::Need),
            "greed" => Some(Choice::Greed),
            "pass" => Some(Choice::Pass),
            _ => None,
        }
    }

    fn roll_type(self) -> f64 {
        match self {
            Choice::Pass => 0.0,
            Choice::Need => 1.0,
            Choice::Greed => 2.0,
        }
    }

    /// `Enum.EncounterLootDropRollState`: NeedMainSpec, Greed, Pass.
    fn state(self) -> f64 {
        match self {
            Choice::Need => 0.0,
            Choice::Greed => 3.0,
            Choice::Pass => 5.0,
        }
    }
}

/// `Enum.EncounterLootDropRollState.NoRoll`: not chosen yet.
const NO_ROLL: f64 = 4.0;

/// A group member on a roll (the player first).
#[derive(Clone, Debug)]
pub struct Member {
    pub name: String,
    pub guid: String,
    pub class: String,
    pub is_self: bool,
    pub choice: Option<Choice>,
    /// When the choice was made, among the roll's choices.
    pub order: usize,
    /// The choice told by the server (the player's: a round trip later).
    pub told: bool,
    /// The number the test gives for the member's roll.
    pub given: Option<f64>,
    /// The number rolled at the end.
    pub roll: Option<f64>,
}

/// A roll.
#[derive(Clone, Debug)]
pub struct Roll {
    pub id: f64,
    pub item: i64,
    pub count: f64,
    /// Its time in seconds, and when it began (`GetTime`'s seconds).
    pub duration: f64,
    pub started: f64,
    pub bop: bool,
    pub can_need: bool,
    pub can_greed: bool,
    pub reason_need: f64,
    pub reason_greed: f64,
    /// The loot it is from (`lootHandle`).
    pub loot: f64,
    pub encounter: f64,
    pub key: f64,
    pub members: Vec<Member>,
    pub choices: usize,
    pub done: bool,
    pub winner: Option<usize>,
}

impl Roll {
    fn everyone_chosen(&self) -> bool {
        self.members.iter().all(|m| m.choice.is_some() && m.told)
    }
}

/// The rolls of the session and the loot history's encounters.
#[derive(Default)]
pub struct LootRolls {
    pub rolls: Vec<Roll>,
    pub next_id: f64,
    pub next_loot: f64,
    /// The encounters with drops: ID, name, the first drop's time.
    pub encounters: Vec<(f64, String, f64)>,
    seed: u32,
}

fn state(vm: &Vm) -> &LootRolls {
    &engine(vm).world.loot_rolls
}

fn state_mut(vm: &mut Vm) -> &mut LootRolls {
    &mut engine_mut(vm).world.loot_rolls
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn later(vm: &mut Vm, seconds: f64, name: &'static str, f: NativeFn, id: f64) {
    let native = vm.native_with(name, f, vec![Value::Num(id)]);
    crate::clock::after_engine(vm, seconds, native);
}

fn index_of(vm: &Vm, id: f64) -> Option<usize> {
    state(vm).rolls.iter().position(|r| r.id == id)
}

/// The roll an argument names, under way.
fn open_arg(vm: &Vm, c: &NativeCall, n: usize) -> Option<usize> {
    let Value::Num(id) = vm.hidden(c.arg(n)) else {
        return None;
    };
    index_of(vm, id).filter(|&i| !state(vm).rolls[i].done)
}

fn text_of(vm: &Vm, v: Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn opt(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn list(vm: &mut Vm, values: Vec<Value>) -> Value {
    let t = vm.new_table();
    for (i, v) in values.into_iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), v);
    }
    Value::Table(t)
}

fn link(vm: &mut Vm, item: i64) -> String {
    crate::character::link_of_id(vm, item)
        .and_then(|v| text_of(vm, v))
        .unwrap_or_default()
}

/// A part of a chat line's format: `%s` or `%d`, in order.
enum Part<'a> {
    S(&'a str),
    D(f64),
}

/// The client's global string `name` (else `default`) with its `%s` and
/// `%d` filled in order.
fn line(vm: &mut Vm, name: &str, default: &str, parts: &[Part]) -> String {
    let given = vm.get_global(name);
    let mut format = text_of(vm, given).unwrap_or_else(|| default.to_string());
    for p in parts {
        let (token, with) = match p {
            Part::S(s) => ("%s", (*s).to_string()),
            Part::D(n) => ("%d", format!("{}", n.trunc() as i64)),
        };
        format = format.replacen(token, &with, 1);
    }
    format
}

fn say(vm: &mut Vm, text: &str) {
    crate::chat::game_message(vm, "CHAT_MSG_LOOT", text, None);
}

/// A number from 1 to 100 of the engine's own generator.
fn random_roll(vm: &mut Vm) -> f64 {
    let s = state_mut(vm);
    if s.seed == 0 {
        s.seed = 0x5bd1_e995;
    }
    s.seed = s.seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
    f64::from((s.seed >> 16) % 100) + 1.0
}

// ------------------------------------------------------------ the API

/// `GetLootRollItemInfo(rollID)`: texture, name, count, quality,
/// bindOnPickUp, canNeed, canGreed, canDisenchant, reasonNeed, reasonGreed,
/// reasonDisenchant, deSkillRequired, canTransmog (Blizzard's
/// GroupLootFrame's order); nothing for a roll not under way (recorded).
fn get_loot_roll_item_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = open_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let r = state(vm).rolls[i].clone();
    let icon = crate::character::item_field(vm, r.item, "icon");
    let name = crate::character::item_field(vm, r.item, "name");
    let quality = match crate::character::item_field(vm, r.item, "quality") {
        Value::Nil => Value::Num(1.0),
        q => q,
    };
    Ok(vec![
        icon,
        name,
        Value::Num(r.count),
        quality,
        Value::Bool(r.bop),
        Value::Bool(r.can_need),
        Value::Bool(r.can_greed),
        Value::Bool(false),
        Value::Num(r.reason_need),
        Value::Num(r.reason_greed),
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Bool(false),
    ])
}

/// `GetLootRollItemLink(rollID)`: the item's link; nothing for a roll not
/// under way (recorded).
fn get_loot_roll_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = open_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let item = state(vm).rolls[i].item;
    let l = link(vm, item);
    Ok(vec![vm.str(&l)])
}

/// The milliseconds a roll has left.
fn time_left(vm: &Vm, i: usize) -> f64 {
    let r = &state(vm).rolls[i];
    let left = r.duration - (crate::clock::now(vm) - r.started);
    (left * 1000.0).round().max(0.0)
}

/// `GetLootRollTimeLeft(rollID)`: the milliseconds left; 0 for a roll not
/// under way (recorded).
fn get_loot_roll_time_left(vm: &mut Vm, c: NativeCall) -> Ret {
    let left = open_arg(vm, &c, 1).map_or(0.0, |i| time_left(vm, i));
    Ok(vec![Value::Num(left)])
}

/// `C_Loot.GetLootRollDuration(rollID)`: the roll's time in milliseconds;
/// nil for a roll not under way (recorded).
fn get_loot_roll_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match open_arg(vm, &c, 1) {
        Some(i) => Value::Num((state(vm).rolls[i].duration * 1000.0).round()),
        None => Value::Nil,
    }])
}

/// `GetActiveLootRollIDs()`: the rolls under way the player has not chosen
/// on (Blizzard's roll frames come back for these after the gamepad UI).
fn get_active_loot_roll_ids(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ids: Vec<Value> = state(vm)
        .rolls
        .iter()
        .filter(|r| !r.done && r.members[0].choice.is_none())
        .map(|r| Value::Num(r.id))
        .collect();
    Ok(vec![list(vm, ids)])
}

/// The roll an API call chooses on: under way, the player not chosen yet,
/// a choice the item allows.
fn choosable(vm: &Vm, c: &NativeCall) -> Option<(usize, Choice)> {
    let i = open_arg(vm, c, 1)?;
    let Value::Num(t) = vm.hidden(c.arg(2)) else {
        return None;
    };
    let choice = Choice::of_type(t)?;
    let r = &state(vm).rolls[i];
    let allowed = match choice {
        Choice::Need => r.can_need,
        Choice::Greed => r.can_greed,
        Choice::Pass => true,
    };
    (allowed && r.members[0].choice.is_none()).then_some((i, choice))
}

/// `RollOnLoot(rollID, rollType)`: the player's choice; a Bind on Pickup
/// item's Need or Greed asks first (`CONFIRM_LOOT_ROLL`).
fn roll_on_loot(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((i, choice)) = choosable(vm, &c) else {
        return Ok(vec![]);
    };
    let r = &state(vm).rolls[i];
    if r.bop && choice != Choice::Pass {
        let id = r.id;
        let reason = vm.str("");
        fire(
            vm,
            "CONFIRM_LOOT_ROLL",
            &[Value::Num(id), Value::Num(choice.roll_type()), reason],
        );
        return Ok(vec![]);
    }
    choose(vm, i, choice);
    Ok(vec![])
}

/// `ConfirmLootRoll(rollID, rollType)`: the choice made (the Bind on
/// Pickup question answered).
fn confirm_loot_roll(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some((i, choice)) = choosable(vm, &c) {
        choose(vm, i, choice);
    }
    Ok(vec![])
}

/// The player's choice made: the client is done with the roll
/// (`CANCEL_LOOT_ROLL`); the server tells the choice a round trip later.
fn choose(vm: &mut Vm, i: usize, choice: Choice) {
    let id = {
        let r = &mut state_mut(vm).rolls[i];
        r.choices += 1;
        r.members[0].choice = Some(choice);
        r.members[0].order = r.choices;
        r.id
    };
    fire(vm, "CANCEL_LOOT_ROLL", &[Value::Num(id)]);
    let delay = crate::clock::server_delay(vm);
    later(vm, delay, "lootRollChosen", chosen_told, id);
}

/// The server's word of the player's choice: its line, the loot history's
/// drop; everyone chosen, the roll ends.
fn chosen_told(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(id) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let Some(i) = index_of(vm, id) else {
        return Ok(vec![]);
    };
    if state(vm).rolls[i].done {
        return Ok(vec![]);
    }
    state_mut(vm).rolls[i].members[0].told = true;
    let r = state(vm).rolls[i].clone();
    let l = link(vm, r.item);
    let enc = Part::D(r.encounter);
    let text = match r.members[0].choice {
        Some(Choice::Need) => line(
            vm,
            "LOOT_ROLL_NEED_SELF",
            "|HlootHistory:%d|h[Loot]|h: You have selected Need for: %s",
            &[enc, Part::S(&l)],
        ),
        Some(Choice::Greed) => line(
            vm,
            "LOOT_ROLL_GREED_SELF",
            "|HlootHistory:%d|h[Loot]|h: You have selected Greed for: %s",
            &[enc, Part::S(&l)],
        ),
        _ => line(
            vm,
            "LOOT_ROLL_PASSED_SELF",
            "|HlootHistory:%d|h[Loot]|h: You passed on: %s",
            &[enc, Part::S(&l)],
        ),
    };
    say(vm, &text);
    drop_changed(vm, i);
    if state(vm).rolls[i].everyone_chosen() {
        resolve(vm, i);
    }
    Ok(vec![])
}

/// The roll's time is up: the player not chosen, the client is done with
/// it (`CANCEL_LOOT_ROLL`); the roll ends.
fn timed_out(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(id) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let Some(i) = index_of(vm, id) else {
        return Ok(vec![]);
    };
    if state(vm).rolls[i].done {
        return Ok(vec![]);
    }
    if state(vm).rolls[i].members[0].choice.is_none() {
        fire(vm, "CANCEL_LOOT_ROLL", &[Value::Num(id)]);
    }
    resolve(vm, i);
    Ok(vec![])
}

/// The roll ends: the best choice's rollers roll, the highest wins (the
/// player: the item to the bags); everyone passed, nobody; the loot's last
/// roll ended, `LOOT_ROLLS_COMPLETE`.
fn resolve(vm: &mut Vm, i: usize) {
    state_mut(vm).rolls[i].done = true;
    let r = state(vm).rolls[i].clone();
    let l = link(vm, r.item);
    let best = [Choice::Need, Choice::Greed]
        .into_iter()
        .find(|ch| r.members.iter().any(|m| m.choice == Some(*ch)));
    let enc = r.encounter;
    match best {
        None => {
            let text = line(
                vm,
                "LOOT_ROLL_ALL_PASSED",
                "|HlootHistory:%d|h[Loot]|h: Everyone passed on: %s",
                &[Part::D(enc), Part::S(&l)],
            );
            say(vm, &text);
        }
        Some(best) => {
            let mut rollers: Vec<usize> = (0..r.members.len())
                .filter(|&m| r.members[m].choice == Some(best))
                .collect();
            rollers.sort_by_key(|&m| r.members[m].order);
            let mut winner: Option<(usize, f64)> = None;
            for m in rollers {
                let n = match r.members[m].given {
                    Some(n) => n,
                    None => random_roll(vm),
                };
                state_mut(vm).rolls[i].members[m].roll = Some(n);
                let (name, default) = match best {
                    Choice::Need => (
                        "LOOT_ROLL_ROLLED_NEED",
                        "|HlootHistory:%d|h[Loot]|h: Need Roll - %d for %s by %s",
                    ),
                    _ => (
                        "LOOT_ROLL_ROLLED_GREED",
                        "|HlootHistory:%d|h[Loot]|h: Greed Roll - %d for %s by %s",
                    ),
                };
                let who = r.members[m].name.clone();
                let text = line(
                    vm,
                    name,
                    default,
                    &[Part::D(enc), Part::D(n), Part::S(&l), Part::S(&who)],
                );
                say(vm, &text);
                if winner.is_none_or(|(_, top)| n > top) {
                    winner = Some((m, n));
                }
            }
            if let Some((m, _)) = winner {
                state_mut(vm).rolls[i].winner = Some(m);
                let text = if r.members[m].is_self {
                    line(
                        vm,
                        "LOOT_ROLL_YOU_WON",
                        "|HlootHistory:%d|h[Loot]|h: You won: %s",
                        &[Part::D(enc), Part::S(&l)],
                    )
                } else {
                    let who = r.members[m].name.clone();
                    line(
                        vm,
                        "LOOT_ROLL_WON",
                        "|HlootHistory:%d|h[Loot]|h: %s won: %s",
                        &[Part::D(enc), Part::S(&who), Part::S(&l)],
                    )
                };
                say(vm, &text);
            }
        }
    }
    drop_changed(vm, i);
    if state(vm).rolls[i].winner == Some(0) {
        crate::loot::receive(vm, r.item, r.count);
    }
    complete_if_done(vm, r.loot);
}

/// Every roll of the loot ended: `LOOT_ROLLS_COMPLETE(lootHandle)`.
fn complete_if_done(vm: &mut Vm, loot: f64) {
    let done = state(vm)
        .rolls
        .iter()
        .filter(|r| r.loot == loot)
        .all(|r| r.done);
    if done {
        fire(vm, "LOOT_ROLLS_COMPLETE", &[Value::Num(loot)]);
    }
}

fn drop_changed(vm: &mut Vm, i: usize) {
    let r = &state(vm).rolls[i];
    let (enc, key) = (r.encounter, r.key);
    fire(
        vm,
        "LOOT_HISTORY_UPDATE_DROP",
        &[Value::Num(enc), Value::Num(key)],
    );
}

// -------------------------------------------------------- the history

fn roll_info(vm: &mut Vm, r: &Roll, m: usize) -> Value {
    let member = &r.members[m];
    let t = vm.new_table();
    let name = vm.str(&member.name);
    set(vm, t, "playerName", name);
    let guid = vm.str(&member.guid);
    set(vm, t, "playerGUID", guid);
    let class = vm.str(&member.class);
    set(vm, t, "playerClass", class);
    set(vm, t, "isSelf", Value::Bool(member.is_self));
    let st = member.choice.map_or(NO_ROLL, Choice::state);
    set(vm, t, "state", Value::Num(st));
    set(vm, t, "isWinner", Value::Bool(r.winner == Some(m)));
    if let Some(n) = member.roll {
        set(vm, t, "roll", Value::Num(n));
    }
    Value::Table(t)
}

/// A drop as `C_LootHistory` tells it (`EncounterLootDropInfo`): the
/// rollers first, the highest number first, then by choice (Need, Greed,
/// Pass, none yet), then in the order chosen.
fn drop_info(vm: &mut Vm, r: &Roll) -> Value {
    let t = vm.new_table();
    set(vm, t, "lootListKey", Value::Num(r.key));
    let l = link(vm, r.item);
    let l = vm.str(&l);
    set(vm, t, "itemHyperlink", l);
    let mine = r.members[0].choice.map_or(NO_ROLL, Choice::state);
    set(vm, t, "playerRollState", Value::Num(mine));
    set(vm, t, "isTied", Value::Bool(false));
    if let Some(w) = r.winner {
        let leader = roll_info(vm, r, w);
        set(vm, t, "winner", leader);
        let leader = roll_info(vm, r, w);
        set(vm, t, "currentLeader", leader);
    }
    let passed = r.done && r.winner.is_none();
    set(vm, t, "allPassed", Value::Bool(passed));
    let mut order: Vec<usize> = (0..r.members.len()).collect();
    order.sort_by(|&a, &b| {
        let (ma, mb) = (&r.members[a], &r.members[b]);
        let rolled = mb.roll.unwrap_or(-1.0).total_cmp(&ma.roll.unwrap_or(-1.0));
        let st = |m: &Member| m.choice.map_or(NO_ROLL, Choice::state);
        let order = |m: &Member| if m.order == 0 { usize::MAX } else { m.order };
        rolled
            .then(st(ma).total_cmp(&st(mb)))
            .then(order(ma).cmp(&order(mb)))
    });
    let infos: Vec<Value> = order.into_iter().map(|m| roll_info(vm, r, m)).collect();
    let infos = list(vm, infos);
    set(vm, t, "rollInfos", infos);
    set(vm, t, "startTime", Value::Num(r.started));
    set(vm, t, "duration", Value::Num(r.duration));
    Value::Table(t)
}

fn encounter_info(vm: &mut Vm, enc: &(f64, String, f64)) -> Value {
    let (id, name, start) = enc;
    let end = state(vm)
        .rolls
        .iter()
        .filter(|r| r.encounter == *id)
        .map(|r| r.started + r.duration)
        .fold(*start, f64::max);
    let t = vm.new_table();
    let n = vm.str(name);
    set(vm, t, "encounterName", n);
    set(vm, t, "encounterID", Value::Num(*id));
    set(vm, t, "startTime", Value::Num(*start));
    set(vm, t, "duration", Value::Num(end - start));
    Value::Table(t)
}

fn encounter_arg(vm: &Vm, c: &NativeCall) -> Option<(f64, String, f64)> {
    let Value::Num(id) = vm.hidden(c.arg(1)) else {
        return None;
    };
    state(vm).encounters.iter().find(|e| e.0 == id).cloned()
}

/// `C_LootHistory.GetAllEncounterInfos()`: the encounters with drops, the
/// newest first.
fn get_all_encounter_infos(vm: &mut Vm, _c: NativeCall) -> Ret {
    let encs: Vec<(f64, String, f64)> = state(vm).encounters.iter().rev().cloned().collect();
    let infos: Vec<Value> = encs.iter().map(|e| encounter_info(vm, e)).collect();
    Ok(vec![list(vm, infos)])
}

/// `C_LootHistory.GetInfoForEncounter(encounterID)`; nil for none
/// (recorded).
fn get_info_for_encounter(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match encounter_arg(vm, &c) {
        Some(e) => encounter_info(vm, &e),
        None => Value::Nil,
    }])
}

/// `C_LootHistory.GetSortedDropsForEncounter(encounterID)`: its drops in
/// order; nil for none (recorded).
fn get_sorted_drops_for_encounter(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((id, _, _)) = encounter_arg(vm, &c) else {
        return Ok(vec![Value::Nil]);
    };
    let rolls: Vec<Roll> = state(vm)
        .rolls
        .iter()
        .filter(|r| r.encounter == id)
        .cloned()
        .collect();
    let drops: Vec<Value> = rolls.iter().map(|r| drop_info(vm, r)).collect();
    Ok(vec![list(vm, drops)])
}

/// `C_LootHistory.GetSortedInfoForDrop(encounterID, lootListKey)`; nil
/// for none (recorded).
fn get_sorted_info_for_drop(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(enc), Value::Num(key)) = (vm.hidden(c.arg(1)), vm.hidden(c.arg(2))) else {
        return Ok(vec![Value::Nil]);
    };
    let found = state(vm)
        .rolls
        .iter()
        .find(|r| r.encounter == enc && r.key == key)
        .cloned();
    Ok(vec![match found {
        Some(r) => drop_info(vm, &r),
        None => Value::Nil,
    }])
}

/// `C_LootHistory.GetLootHistoryTime()`: `GetTime`'s seconds.
fn get_loot_history_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(crate::clock::now(vm))])
}

/// `C_TooltipInfo.GetLootRollItem(rollID)`: the item's tooltip; nothing for
/// a roll not under way (recorded).
fn get_loot_roll_item_tooltip(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = open_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let item = state(vm).rolls[i].item;
    crate::tooltip::item_by_id(vm, item)
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetLootRollItemInfo", get_loot_roll_item_info),
        ("GetLootRollItemLink", get_loot_roll_item_link),
        ("GetLootRollTimeLeft", get_loot_roll_time_left),
        ("GetActiveLootRollIDs", get_active_loot_roll_ids),
        ("C_Loot.GetLootRollDuration", get_loot_roll_duration),
        ("RollOnLoot", roll_on_loot),
        ("ConfirmLootRoll", confirm_loot_roll),
        (
            "C_LootHistory.GetAllEncounterInfos",
            get_all_encounter_infos,
        ),
        ("C_LootHistory.GetInfoForEncounter", get_info_for_encounter),
        (
            "C_LootHistory.GetSortedDropsForEncounter",
            get_sorted_drops_for_encounter,
        ),
        (
            "C_LootHistory.GetSortedInfoForDrop",
            get_sorted_info_for_drop,
        ),
        ("C_LootHistory.GetLootHistoryTime", get_loot_history_time),
        ("C_TooltipInfo.GetLootRollItem", get_loot_roll_item_tooltip),
    ]
}

// ----------------------------------------------------------- controls

/// A group member the test names: a name, a unit, or `{ name, class? }`.
fn member_of(vm: &mut Vm, v: Value, n: usize) -> Result<Member, wowlua_vm::LuaError> {
    let made = |name: String, class: String, n: usize| Member {
        guid: format!("Player-1-{:08X}", 0x0100_0000 + n),
        name,
        class,
        is_self: false,
        choice: None,
        order: 0,
        told: false,
        given: None,
        roll: None,
    };
    if let Some(name) = text_of(vm, v) {
        return Ok(made(name, CLASS.into(), n));
    }
    if let Some(u) = units::handle_of(vm, v) {
        let unit = &engine(vm).units.list[u.0];
        let mut m = made(unit.name.clone(), unit.class.clone(), n);
        m.guid = unit.guid.clone();
        if m.class.is_empty() {
            m.class = CLASS.into();
        }
        return Ok(m);
    }
    if let Value::Table(t) = v {
        let name = opt(vm, t, "name");
        let class = opt(vm, t, "class");
        if let Some(name) = text_of(vm, name) {
            let class = text_of(vm, class).unwrap_or_else(|| CLASS.into());
            return Ok(made(name, class, n));
        }
    }
    Err(vm.error("startLootRoll: a group member is a name, a unit or { name, class }"))
}

fn flag(vm: &mut Vm, t: TableRef, key: &str, default: bool) -> bool {
    match opt(vm, t, key) {
        Value::Nil => default,
        v => v.truthy(),
    }
}

fn number(vm: &mut Vm, t: TableRef, key: &str) -> Option<f64> {
    match opt(vm, t, key) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// `startLootRoll(roll)`: a roll begins (`START_LOOT_ROLL`); its ID.
pub fn control_start(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("startLootRoll: a table expected"));
    };
    let Some(item) = number(vm, t, "item") else {
        return Err(vm.error("startLootRoll: an item ID expected (item)"));
    };
    let item = item as i64;
    if crate::gamedata::item_data(vm, item).is_none() {
        return Err(vm.error(&format!(
            "startLootRoll: item {item} is unknown (mock.defineItem)"
        )));
    }
    let count = number(vm, t, "count").unwrap_or(1.0).max(1.0);
    let duration = number(vm, t, "rollTime").unwrap_or(ROLL_TIME).max(0.0);
    let bound = crate::character::item_field(vm, item, "bindType");
    let bop = flag(vm, t, "bop", matches!(bound, Value::Num(b) if b == 1.0));
    let can_need = flag(vm, t, "canNeed", true);
    let can_greed = flag(vm, t, "canGreed", true);
    let reason_need = number(vm, t, "reasonNeed").unwrap_or(0.0);
    let reason_greed = number(vm, t, "reasonGreed").unwrap_or(0.0);
    // The group: the player first, then the test's, else the player's group.
    let me = {
        let p = &engine(vm).units.list[units::PLAYER.0];
        Member {
            name: p.name.clone(),
            guid: p.guid.clone(),
            class: if p.class.is_empty() {
                CLASS.into()
            } else {
                p.class.clone()
            },
            is_self: true,
            choice: None,
            order: 0,
            told: false,
            given: None,
            roll: None,
        }
    };
    let mut members = vec![me];
    match opt(vm, t, "group") {
        Value::Table(g) => {
            let n = vm.heap.table(g).border();
            for k in 1..=n {
                let v = vm.heap.table(g).get(Value::Num(k as f64));
                let m = member_of(vm, v, k)?;
                members.push(m);
            }
        }
        Value::Nil => {
            let group = engine(vm).units.group.clone();
            for (k, u) in group.into_iter().enumerate() {
                let h = units::handle(vm, u);
                let m = member_of(vm, h, k + 1)?;
                members.push(m);
            }
        }
        _ => return Err(vm.error("startLootRoll: group is a list")),
    }
    if let Value::Table(given) = opt(vm, t, "rolls") {
        for m in &mut members {
            let k = vm.str(&m.name);
            if let Value::Num(n) = vm.heap.table(given).get(k) {
                m.given = Some(n);
            }
        }
    }
    // The encounter: the test's, the one under way, else none (0).
    let (encounter, enc_name) = match number(vm, t, "encounterID") {
        Some(id) => {
            let name = opt(vm, t, "encounterName");
            (id, text_of(vm, name).unwrap_or_default())
        }
        None => engine(vm)
            .world
            .encounters
            .active
            .as_ref()
            .map_or((0.0, String::new()), |a| (a.id, a.name.clone())),
    };
    let now = crate::clock::now(vm);
    let loot = match number(vm, t, "loot") {
        Some(l) => l,
        None => {
            let s = state_mut(vm);
            s.next_loot += 1.0;
            s.next_loot
        }
    };
    let id = {
        let s = state_mut(vm);
        if !s.encounters.iter().any(|e| e.0 == encounter) {
            s.encounters.push((encounter, enc_name, now));
        }
        let key = s.rolls.iter().filter(|r| r.encounter == encounter).count() as f64 + 1.0;
        s.next_id += 1.0;
        let id = s.next_id;
        s.rolls.push(Roll {
            id,
            item,
            count,
            duration,
            started: now,
            bop,
            can_need,
            can_greed,
            reason_need,
            reason_greed,
            loot,
            encounter,
            key,
            members,
            choices: 0,
            done: false,
            winner: None,
        });
        id
    };
    fire(
        vm,
        "START_LOOT_ROLL",
        &[
            Value::Num(id),
            Value::Num((duration * 1000.0).round()),
            Value::Num(loot),
        ],
    );
    fire(
        vm,
        "LOOT_HISTORY_UPDATE_ENCOUNTER",
        &[Value::Num(encounter)],
    );
    later(vm, duration, "lootRollTimeOut", timed_out, id);
    Ok(vec![Value::Num(id)])
}

/// `lootRollChoice(rollID, name, choice, roll?)`: a group member's choice
/// (`"need"`, `"greed"`, `"pass"`), told at once; `roll` the number they
/// roll if it comes to that.
pub fn control_choice(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_number(&c, 1)?;
    let Some(i) = index_of(vm, id).filter(|&i| !state(vm).rolls[i].done) else {
        return Err(vm.error(&format!("lootRollChoice: no roll {id} under way")));
    };
    let name = text_of(vm, c.arg(2)).unwrap_or_default();
    let choice = text_of(vm, c.arg(3)).and_then(|s| Choice::of_name(&s));
    let Some(choice) = choice else {
        return Err(vm.error("lootRollChoice: \"need\", \"greed\" or \"pass\" expected"));
    };
    let m = state(vm).rolls[i]
        .members
        .iter()
        .position(|m| !m.is_self && m.name == name);
    let Some(m) = m else {
        return Err(vm.error(&format!(
            "lootRollChoice: {name} is not in the roll's group (the player chooses with RollOnLoot)"
        )));
    };
    if state(vm).rolls[i].members[m].choice.is_some() {
        return Err(vm.error(&format!("lootRollChoice: {name} has chosen already")));
    }
    let given = match c.arg(4) {
        Value::Num(n) => Some(n),
        _ => None,
    };
    let r = {
        let r = &mut state_mut(vm).rolls[i];
        r.choices += 1;
        let member = &mut r.members[m];
        member.choice = Some(choice);
        member.order = r.choices;
        member.told = true;
        if given.is_some() {
            member.given = given;
        }
        r.clone()
    };
    let l = link(vm, r.item);
    let enc = Part::D(r.encounter);
    let text = match choice {
        Choice::Need => line(
            vm,
            "LOOT_ROLL_NEED",
            "|HlootHistory:%d|h[Loot]|h: %s has selected Need for: %s",
            &[enc, Part::S(&name), Part::S(&l)],
        ),
        Choice::Greed => line(
            vm,
            "LOOT_ROLL_GREED",
            "|HlootHistory:%d|h[Loot]|h: %s has selected Greed for: %s",
            &[enc, Part::S(&name), Part::S(&l)],
        ),
        Choice::Pass => line(
            vm,
            "LOOT_ROLL_PASSED",
            "|HlootHistory:%d|h[Loot]|h: %s passed on: %s",
            &[enc, Part::S(&name), Part::S(&l)],
        ),
    };
    say(vm, &text);
    drop_changed(vm, i);
    if r.everyone_chosen() {
        resolve(vm, i);
    }
    Ok(vec![])
}

/// `cancelLootRoll(rollID?)`: the roll taken back without a result
/// (`CANCEL_LOOT_ROLL(rollID)`); without an ID every roll under way
/// (`CANCEL_ALL_LOOT_ROLLS()`).
pub fn control_cancel(vm: &mut Vm, c: NativeCall) -> Ret {
    match c.arg(1) {
        Value::Num(id) => {
            let Some(i) = index_of(vm, id).filter(|&i| !state(vm).rolls[i].done) else {
                return Err(vm.error(&format!("cancelLootRoll: no roll {id} under way")));
            };
            state_mut(vm).rolls[i].done = true;
            fire(vm, "CANCEL_LOOT_ROLL", &[Value::Num(id)]);
        }
        _ => {
            for r in &mut state_mut(vm).rolls {
                r.done = true;
            }
            fire(vm, "CANCEL_ALL_LOOT_ROLLS", &[]);
        }
    }
    Ok(vec![])
}
