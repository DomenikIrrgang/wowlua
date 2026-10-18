//! A class trainer, as the client recorded the warrior's (`/stdtest engine
//! scenario trainer`, a ping near 0.3 s): `defineTrainer(npc, { greeting,
//! services })`, opened through its gossip (`gossip.rs`): `TRAINER_SHOW()`
//! (Blizzard's UI loads its trainer frame), the interaction's
//! `PLAYER_INTERACTION_MANAGER_FRAME_SHOW(7)`, `TRAINER_UPDATE()` with the
//! services listed (none before it); a rank waiting for its data
//! (`rankData`: Rend's) is nil till `TOOLTIP_DATA_UPDATE(id)` and
//! `TRAINER_SERVICE_INFO_NAME_UPDATE()` a moment later. A service is "available" from its
//! level, "unavailable" below it, "used" once known.
//!
//! `BuyTrainerService(i)`: the server's answer a round trip later: "You have
//! learned a new ability: …", `LEARNED_SPELL_IN_SKILL_LINE(spell, line,
//! false)`, the spell pushed to the action bar's first empty slot (the grid
//! shown and hidden, `ACTIONBAR_SLOT_CHANGED`, `SPELL_PUSHED_TO_ACTIONBAR`),
//! the list updated, the spells changed; the money's packet after it.
//! `CloseTrainer()`: `TRAINER_CLOSED()` and the interaction's hides.
//!
//! Unverified: a profession trainer, service filters, a spell bought
//! without room on the bar (none pushed?), the spellbook's new entry (not
//! kept), the money packet's delay beyond this recording.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// The money's packet after the spell's (recorded).
const MONEY_AFTER: f64 = 0.093;
/// A rank's data after the list (recorded).
const RANK_DATA_AFTER: f64 = 0.013;
/// `Enum.PlayerInteractionType.Trainer`.
const TRAINER: f64 = 7.0;

#[derive(Default)]
pub struct Trainers {
    /// The NPCs' trainers: unit → the test's table.
    pub defined: std::collections::BTreeMap<usize, TableRef>,
    /// The trainer open, and whether its list has come (`TRAINER_UPDATE`).
    pub open: Option<UnitId>,
    pub listed: bool,
    pub selected: Option<usize>,
    /// Whether the services' ranks waiting for their data have it.
    pub ranks_loaded: bool,
    /// The spells learned here.
    pub learned: std::collections::BTreeSet<i64>,
}

#[derive(Clone, Debug)]
struct Service {
    spell: i64,
    name: Value,
    icon: Value,
    level: f64,
    rank: Value,
    cost: f64,
    skill_line: Value,
    line_index: f64,
    /// The tooltip data its rank waits for (`rankData`), if any.
    rank_data: Option<f64>,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// `defineTrainer(npc, { greeting, services = { { spell, name, icon,
/// level, rank, cost, skillLine, skillLineIndex } } })`.
pub fn control_define(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(npc) = units::handle_of(vm, c.arg(1)) else {
        return Err(vm.error("defineTrainer: an NPC expected"));
    };
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("defineTrainer: a table of the trainer expected"));
    };
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm).world.trainers.defined.insert(npc.0, t);
    Ok(vec![])
}

/// The trainer opens (its gossip's option chosen).
pub fn open(vm: &mut Vm, npc: UnitId) {
    if !engine(vm).world.trainers.defined.contains_key(&npc.0) {
        return;
    }
    {
        let t = &mut engine_mut(vm).world.trainers;
        t.open = Some(npc);
        t.listed = false;
        t.selected = None;
        t.ranks_loaded = false;
    }
    engine_mut(vm).units.npc = Some(npc);
    fire(vm, "TRAINER_SHOW", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(TRAINER)],
    );
    engine_mut(vm).world.trainers.listed = true;
    fire(vm, "TRAINER_UPDATE", &[]);
    // A rank waiting for its data: it comes a moment later (recorded).
    let waiting: Vec<f64> = services(vm).iter().filter_map(|s| s.rank_data).collect();
    if !waiting.is_empty() {
        let ids = waiting.into_iter().map(Value::Num).collect();
        let later = vm.native_with("trainerRanks", ranks_loaded as NativeFn, ids);
        crate::clock::after_engine(vm, RANK_DATA_AFTER, later);
    }
}

/// The ranks' data: `TOOLTIP_DATA_UPDATE(id)` each, the names' update.
fn ranks_loaded(vm: &mut Vm, c: NativeCall) -> Ret {
    if engine(vm).world.trainers.open.is_none() {
        return Ok(vec![]);
    }
    engine_mut(vm).world.trainers.ranks_loaded = true;
    for id in c.upvals.iter().copied().collect::<Vec<_>>() {
        fire(vm, "TOOLTIP_DATA_UPDATE", &[id]);
    }
    fire(vm, "TRAINER_SERVICE_INFO_NAME_UPDATE", &[]);
    Ok(vec![])
}

fn services(vm: &mut Vm) -> Vec<Service> {
    let t = &engine(vm).world.trainers;
    if !t.listed {
        return vec![];
    }
    let Some(data) = t.open.and_then(|n| t.defined.get(&n.0).copied()) else {
        return vec![];
    };
    let Value::Table(list) = field(vm, data, "services") else {
        return vec![];
    };
    let n = vm.heap.table(list).border();
    let mut out = Vec::new();
    for i in 1..=n {
        let Value::Table(s) = vm.heap.table(list).get(Value::Num(i as f64)) else {
            continue;
        };
        out.push(Service {
            spell: num(field(vm, s, "spell")).unwrap_or(0.0) as i64,
            name: field(vm, s, "name"),
            icon: field(vm, s, "icon"),
            level: num(field(vm, s, "level")).unwrap_or(1.0),
            rank: field(vm, s, "rank"),
            cost: num(field(vm, s, "cost")).unwrap_or(0.0),
            skill_line: field(vm, s, "skillLine"),
            line_index: num(field(vm, s, "skillLineIndex")).unwrap_or(1.0),
            rank_data: num(field(vm, s, "rankData")),
        });
    }
    out
}

fn service(vm: &mut Vm, c: &NativeCall) -> Option<(usize, Service)> {
    let i = num(vm.hidden(c.arg(1)))? as usize;
    let s = services(vm).get(i.checked_sub(1)?)?.clone();
    Some((i, s))
}

fn kind(vm: &Vm, s: &Service) -> &'static str {
    if engine(vm).world.trainers.learned.contains(&s.spell) {
        "used"
    } else if engine(vm).units.list[units::PLAYER.0].level >= s.level {
        "available"
    } else {
        "unavailable"
    }
}

fn get_num_trainer_services(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = services(vm).len();
    Ok(vec![Value::Num(n as f64)])
}

/// `GetTrainerServiceInfo(i)`: name, type, texture, required level, rank,
/// category ("", recorded).
fn get_trainer_service_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, s)) = service(vm, &c) else {
        return Ok(vec![]);
    };
    let kind = vm.str(kind(vm, &s));
    let category = vm.str("");
    let rank = if s.rank_data.is_some() && !engine(vm).world.trainers.ranks_loaded {
        Value::Nil
    } else {
        s.rank
    };
    Ok(vec![
        s.name,
        kind,
        s.icon,
        Value::Num(s.level),
        rank,
        category,
    ])
}

/// `GetTrainerServiceCost(i)`: the money, not a profession.
fn get_trainer_service_cost(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, s)) = service(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(vec![Value::Num(s.cost), Value::Bool(false)])
}

fn get_trainer_service_skill_line(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, s)) = service(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(vec![s.skill_line])
}

fn get_trainer_greeting_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = &engine(vm).world.trainers;
    let greeting = t
        .open
        .and_then(|n| t.defined.get(&n.0).copied())
        .map(|d| field(vm, d, "greeting"));
    Ok(vec![match greeting {
        Some(Value::Str(s)) => Value::Str(s),
        _ => vm.str(""),
    }])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn yes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `GetTrainerSelectionIndex()`: nothing until one is selected (recorded).
fn get_trainer_selection_index(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(engine(vm)
        .world
        .trainers
        .selected
        .map(|i| vec![Value::Num(i as f64)])
        .unwrap_or_default())
}

fn select_trainer_service(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some((i, _)) = service(vm, &c) {
        engine_mut(vm).world.trainers.selected = Some(i);
    }
    Ok(vec![])
}

/// `BuyTrainerService(i)`: the server teaches it a round trip later.
fn buy_trainer_service(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((i, s)) = service(vm, &c) else {
        return Ok(vec![]);
    };
    if kind(vm, &s) != "available" || engine(vm).world.character.money < s.cost {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    let later = vm.native_with(
        "trainerTaught",
        taught as NativeFn,
        vec![Value::Num(i as f64)],
    );
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn taught(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(i)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let Some(s) = services(vm).get(i as usize - 1).cloned() else {
        return Ok(vec![]);
    };
    engine_mut(vm).world.trainers.learned.insert(s.spell);
    let name = match s.name {
        Value::Str(n) => String::from_utf8_lossy(vm.str_bytes(n)).into_owned(),
        _ => String::new(),
    };
    let link = format!("|cff71d5ff|Hspell:{}:0|h[{name}]|h|r", s.spell);
    let format = match vm.get_global("ERR_LEARN_ABILITY_S") {
        Value::Str(f) => String::from_utf8_lossy(vm.str_bytes(f)).into_owned(),
        _ => "You have learned a new ability: %s.".into(),
    };
    crate::chat::game_message(
        vm,
        "CHAT_MSG_SYSTEM",
        &format.replacen("%s", &link, 1),
        None,
    );
    let spell = Value::Num(s.spell as f64);
    fire(
        vm,
        "LEARNED_SPELL_IN_SKILL_LINE",
        &[spell, Value::Num(s.line_index), Value::Bool(false)],
    );
    // Onto the main bar's first empty slot (recorded: 3, after 1 and 2).
    let slot = (1..=12).find(|n| !engine(vm).world.character.actions.contains_key(n));
    if let Some(slot) = slot {
        let sub_type = vm.str("spell");
        let action = crate::actions::Action {
            kind: "spell".into(),
            id: spell,
            sub_type,
            texture: s.icon,
            count: String::new(),
            text: Value::Nil,
            usable: (true, false),
        };
        engine_mut(vm).world.character.actions.insert(slot, action);
        fire(vm, "ACTIONBAR_SHOWGRID", &[]);
        fire(
            vm,
            "CLICKBINDINGS_SET_HIGHLIGHTS_SHOWN",
            &[Value::Bool(true)],
        );
        fire(
            vm,
            "CLICKBINDINGS_SET_HIGHLIGHTS_SHOWN",
            &[Value::Bool(false)],
        );
        fire(vm, "ACTIONBAR_HIDEGRID", &[]);
        let n = Value::Num(slot as f64);
        fire(vm, "ACTIONBAR_SLOT_CHANGED", &[n]);
        fire(vm, "SPELL_PUSHED_TO_ACTIONBAR", &[spell, n, n]);
        fire(vm, "TRAINER_UPDATE", &[]);
        fire(vm, "ACTIONBAR_SLOT_CHANGED", &[n]);
    } else {
        fire(vm, "TRAINER_UPDATE", &[]);
    }
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    crate::character::group_buffs_changed(vm);
    fire(vm, "SPELLS_CHANGED", &[]);
    let later = vm.native_with("trainerPaid", paid as NativeFn, vec![Value::Num(s.cost)]);
    crate::clock::after_engine(vm, MONEY_AFTER, later);
    Ok(vec![])
}

/// The money's packet: the cost paid, the skill's line learned.
fn paid(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(cost)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    fire(vm, "TRAINER_UPDATE", &[]);
    engine_mut(vm).world.character.money -= cost;
    fire(vm, "PLAYER_MONEY", &[]);
    for e in ["UNIT_ATTACK", "UNIT_DEFENSE", "UNIT_RANGED_ATTACK_POWER"] {
        units::fire_unit(vm, e, units::PLAYER, &[]);
    }
    fire(vm, "SKILL_LINES_CHANGED", &[]);
    fire(vm, "TRAINER_UPDATE", &[]);
    fire(vm, "MOUNT_JOURNAL_USABILITY_CHANGED", &[]);
    Ok(vec![])
}

/// `CloseTrainer()`: `TRAINER_CLOSED()`, the interaction's hides.
fn close_trainer(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine_mut(vm).world.trainers.open.take().is_none() {
        return Ok(vec![]);
    }
    engine_mut(vm).units.npc = None;
    fire(vm, "TRAINER_CLOSED", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(TRAINER)],
    );
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(0.0)],
    );
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetNumTrainerServices", get_num_trainer_services),
        ("GetTrainerServiceInfo", get_trainer_service_info),
        ("GetTrainerServiceCost", get_trainer_service_cost),
        ("GetTrainerServiceSkillLine", get_trainer_service_skill_line),
        ("GetTrainerGreetingText", get_trainer_greeting_text),
        ("IsTradeskillTrainer", no),
        ("GetTrainerSelectionIndex", get_trainer_selection_index),
        ("SelectTrainerService", select_trainer_service),
        ("BuyTrainerService", buy_trainer_service),
        ("CloseTrainer", close_trainer),
        // What a class trainer's services have (unverified: not
        // recorded): no skill or ability required, no step, every filter
        // on.
        ("GetTrainerServiceSkillReq", nothing),
        ("GetTrainerServiceNumAbilityReq", zero),
        ("GetTrainerServiceAbilityReq", nothing),
        ("GetTrainerServiceStepIndex", nothing),
        ("GetTrainerServiceTypeFilter", yes),
        ("GetTrainerTradeskillRankValues", nothing),
    ]
}
