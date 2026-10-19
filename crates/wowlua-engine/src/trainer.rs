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
//! `defineTrainer(npc, { class = "WARRIOR" })`: the class's services as
//! the client listed them (`/stdtest engine trainer`,
//! `data/camelot-trainers.txt`: the NPC's own list if recorded, a
//! starting zone's teaching fewer, else the class's), each one's state the
//! character's: "used" once known (a lower rank too, once a higher one
//! is), "available" from its level with the abilities it needs known,
//! else "unavailable". The list is the services the type filters let
//! through (available and unavailable on, used off: recorded) as the
//! window opened or a filter changed; a service learned meanwhile stays.
//!
//! Unverified: a profession trainer, the filters' list kept until the
//! window reopens, a spell bought without room on the bar (none pushed?),
//! the spellbook's new entry (not kept), the money packet's delay beyond
//! this recording, the class's spellbook tabs (Classic's), the recorded
//! services the probing character knew (their level 0: unknown).

use std::collections::HashMap;
use std::sync::OnceLock;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::traits::Rv;
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
    /// The type filters off ("available", "unavailable", "used"; recorded:
    /// only "used").
    pub filtered_out: Option<std::collections::BTreeSet<&'static str>>,
    /// The open trainer's recorded data (a class trainer's), if any.
    pub key: Option<&'static str>,
    /// Its services' ranks still loading this session: spell → the
    /// tooltip data each waits for (`loading`).
    pub loading: HashMap<i64, f64>,
    /// Its services shown (their indices in its data), as the window
    /// opened or a filter changed.
    pub shown: Vec<usize>,
}

/// The type filters off by default (recorded: every class trainer's).
const FILTERED_OUT: &[&str] = &["used"];
const FILTERS: [&str; 3] = ["available", "unavailable", "used"];

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
    /// A recorded trainer's: its key and index in the data.
    recorded: Option<(&'static str, usize)>,
    /// The abilities it needs (`"Thunder Clap (Rank 1)"`).
    abilities: Vec<String>,
}

/// The recorded trainers' answers, by key and call.
fn data() -> &'static HashMap<(&'static str, &'static str), &'static str> {
    static DATA: OnceLock<HashMap<(&'static str, &'static str), &'static str>> = OnceLock::new();
    DATA.get_or_init(|| {
        include_str!("../data/camelot-trainers.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (key, rest) = l.split_once('\t')?;
                let (call, answer) = rest.split_once('\t')?;
                Some((
                    (key, crate::recorded::unescape(call)),
                    crate::recorded::unescape(answer),
                ))
            })
            .collect()
    })
}

fn recorded(key: &str, call: &str) -> Vec<Rv> {
    data()
        .get(&(key, call))
        .map(|a| Rv::answer(a))
        .unwrap_or_default()
}

/// A recorded trainer's service, as the data has it.
#[derive(Clone, Debug)]
struct Listed {
    spell: i64,
    name: String,
    icon: f64,
    level: f64,
    rank: Option<String>,
    cost: f64,
    line: String,
    abilities: Vec<String>,
    /// Its rank still loading as recorded (nil): the tooltip data it
    /// waits for.
    pending: Option<f64>,
}

/// A class's spells' ranks, from all its trainers' recordings (one may
/// have a rank still loading).
fn class_ranks(class: &str) -> &'static HashMap<i64, String> {
    static RANKS: OnceLock<std::sync::Mutex<HashMap<String, &'static HashMap<i64, String>>>> =
        OnceLock::new();
    let all = RANKS.get_or_init(Default::default);
    let mut all = all.lock().unwrap_or_else(|e| e.into_inner());
    all.entry(class.to_string()).or_insert_with(|| {
        let mut ranks = HashMap::new();
        let mut keys: Vec<&'static str> = data()
            .keys()
            .map(|(k, _)| *k)
            .filter(|k| k.split('-').next() == Some(class))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            let n = match recorded(key, "GetNumTrainerServices()").first() {
                Some(Rv::Num(n)) => *n as usize,
                _ => 0,
            };
            for i in 1..=n {
                let tip = recorded(key, &format!("C_TooltipInfo.GetTrainerService({i})"));
                let info = recorded(key, &format!("GetTrainerServiceInfo({i})"));
                if let (Some(Rv::Num(id)), Some(Rv::Str(rank))) =
                    (tip.first().and_then(|t| t.get("id")), info.get(4))
                {
                    ranks.entry(*id as i64).or_insert_with(|| rank.clone());
                }
            }
        }
        Box::leak(Box::new(ranks))
    })
}

/// The level a class's spell is learned at (`C_Spell.GetSpellLevelLearned`,
/// any of its trainers' recordings).
fn learned_level(class: &str, spell: i64) -> Option<f64> {
    let call = format!("C_Spell.GetSpellLevelLearned({spell})");
    data()
        .iter()
        .filter(|((k, c), _)| k.split('-').next() == Some(class) && *c == call)
        .find_map(|(_, a)| match Rv::answer(a).first() {
            Some(Rv::Num(n)) if *n > 0.0 => Some(*n),
            _ => None,
        })
}

/// A recorded trainer's services, in its order.
fn listed(key: &'static str) -> &'static [Listed] {
    static LISTS: OnceLock<std::sync::Mutex<HashMap<&'static str, &'static [Listed]>>> =
        OnceLock::new();
    let lists = LISTS.get_or_init(Default::default);
    let mut lists = lists.lock().unwrap_or_else(|e| e.into_inner());
    lists.entry(key).or_insert_with(|| {
        let n = match recorded(key, "GetNumTrainerServices()").first() {
            Some(Rv::Num(n)) => *n as usize,
            _ => 0,
        };
        let numf = |v: Option<&Rv>| match v {
            Some(Rv::Num(n)) => *n,
            _ => 0.0,
        };
        let text = |v: Option<&Rv>| match v {
            Some(Rv::Str(s)) => Some(s.clone()),
            _ => None,
        };
        let list: Vec<Listed> = (1..=n)
            .map(|i| {
                let info = recorded(key, &format!("GetTrainerServiceInfo({i})"));
                let tip = recorded(key, &format!("C_TooltipInfo.GetTrainerService({i})"));
                let abilities =
                    numf(recorded(key, &format!("GetTrainerServiceNumAbilityReq({i})")).first())
                        as usize;
                let spell = numf(tip.first().and_then(|t| t.get("id"))) as i64;
                let rank = text(info.get(4));
                let class = key.split('-').next().unwrap_or(key);
                let pending = match rank {
                    None => Some(numf(tip.first().and_then(|t| t.get("dataInstanceID")))),
                    Some(_) => None,
                };
                Listed {
                    spell,
                    name: text(info.first()).unwrap_or_default(),
                    icon: numf(info.get(2)),
                    level: match numf(
                        recorded(key, &format!("GetTrainerServiceLevelReq({i})")).first(),
                    ) {
                        // Known to the probing character (a known one's
                        // level reads 0): the level the spell is learned
                        // at, a passive's aside (the test checks the two
                        // agree where both are recorded; Parry is learned
                        // at 1, taught at 6).
                        0.0 if rank.as_deref() != Some("Passive") => {
                            learned_level(class, spell).unwrap_or(0.0)
                        }
                        0.0 => 0.0,
                        level => level,
                    },
                    rank: rank.or_else(|| class_ranks(class).get(&spell).cloned()),
                    pending,
                    cost: numf(recorded(key, &format!("GetTrainerServiceCost({i})")).first()),
                    line: text(recorded(key, &format!("GetTrainerServiceSkillLine({i})")).first())
                        .unwrap_or_default(),
                    abilities: (1..=abilities)
                        .filter_map(|j| {
                            text(
                                recorded(key, &format!("GetTrainerServiceAbilityReq({i}, {j})"))
                                    .first(),
                            )
                        })
                        .collect(),
                }
            })
            .collect();
        Box::leak(list.into_boxed_slice())
    })
}

/// The data's key for a class trainer: the NPC's own list, else the
/// class's.
fn key_for(vm: &Vm, npc: UnitId, class: &str) -> Option<&'static str> {
    let guid = &engine(vm).units.list[npc.0].guid;
    let own = guid.split('-').nth(5).map(|id| format!("{class}-{id}"));
    let keys = || data().keys().map(|(k, _)| *k);
    own.and_then(|own| keys().find(|k| *k == own))
        .or_else(|| keys().find(|k| *k == class))
}

/// The class's spellbook tabs after General, by skill line (Classic's;
/// unverified: Forever's beyond the warrior's Fury 3).
const TABS: &[(&str, [&str; 3])] = &[
    ("WARRIOR", ["Arms", "Fury", "Protection"]),
    ("DRUID", ["Balance", "Feral Combat", "Restoration"]),
    ("HUNTER", ["Beast Mastery", "Marksmanship", "Survival"]),
    ("MAGE", ["Arcane", "Fire", "Frost"]),
    ("PALADIN", ["Holy", "Protection", "Retribution"]),
    ("PRIEST", ["Discipline", "Holy", "Shadow Magic"]),
    ("ROGUE", ["Assassination", "Combat", "Subtlety"]),
    ("SHAMAN", ["Elemental Combat", "Enhancement", "Restoration"]),
    ("WARLOCK", ["Affliction", "Demonology", "Destruction"]),
];

fn tab_of(key: &str, line: &str) -> f64 {
    let class = key.split('-').next().unwrap_or(key);
    TABS.iter()
        .find(|(c, _)| *c == class)
        .and_then(|(_, tabs)| tabs.iter().position(|t| *t == line))
        .map_or(1.0, |i| i as f64 + 2.0)
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
/// level, rank, cost, skillLine, skillLineIndex } } })`, or `{ class,
/// loading = { [spell] = tooltip data } }`: the class's recorded services
/// (the ranks still loading this session: nil till their data comes).
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
    let data = engine(vm).world.trainers.defined[&npc.0];
    let key = match field(vm, data, "class") {
        Value::Str(c) => {
            let class = String::from_utf8_lossy(vm.str_bytes(c)).to_uppercase();
            key_for(vm, npc, &class)
        }
        _ => None,
    };
    let mut loading = HashMap::new();
    if let Value::Table(l) = field(vm, data, "loading") {
        let entries: Vec<(Value, Value)> = vm.heap.table(l).entries().collect();
        for (spell, id) in entries {
            if let (Value::Num(spell), Value::Num(id)) = (spell, id) {
                loading.insert(spell as i64, id);
            }
        }
    }
    {
        let t = &mut engine_mut(vm).world.trainers;
        t.open = Some(npc);
        t.listed = false;
        t.selected = None;
        t.ranks_loaded = false;
        t.key = key;
        t.loading = loading;
    }
    refilter(vm);
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

/// A recorded trainer's list: the services the filters let through.
fn refilter(vm: &mut Vm) {
    let Some(key) = engine(vm).world.trainers.key else {
        return;
    };
    let shown: Vec<usize> = listed(key)
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            let state = state_of(vm, key, l);
            !filtered_out(vm).contains(&state)
        })
        .map(|(i, _)| i)
        .collect();
    engine_mut(vm).world.trainers.shown = shown;
}

fn filtered_out(vm: &Vm) -> std::collections::BTreeSet<&'static str> {
    engine(vm)
        .world
        .trainers
        .filtered_out
        .clone()
        .unwrap_or_else(|| FILTERED_OUT.iter().copied().collect())
}

/// Whether the character knows a recorded service's spell (`has`: the
/// spells it has): it, or a higher rank of it (unverified: the spellbook
/// keeping lower ranks).
fn knows(key: &'static str, l: &Listed, has: &dyn Fn(i64) -> bool) -> bool {
    let rank = |r: &Option<String>| {
        r.as_deref()
            .and_then(|r| r.strip_prefix("Rank "))
            .and_then(|n| n.parse::<u32>().ok())
    };
    has(l.spell)
        || rank(&l.rank).is_some_and(|mine| {
            listed(key).iter().any(|o| {
                o.name == l.name && rank(&o.rank).is_some_and(|r| r > mine) && has(o.spell)
            })
        })
}

/// Abilities by name: what the character knows outside the trainer's
/// list (a starting spell, a quest's).
type Named<'a> = &'a dyn Fn(&str) -> bool;

/// An ability a service needs (`"Thunder Clap (Rank 1)"`, `"Parry"`),
/// known: the trainer's service of it, else by name (`named`).
fn ability_known(
    key: &'static str,
    ability: &str,
    has: &dyn Fn(i64) -> bool,
    named: Named,
) -> bool {
    let mut taught = false;
    let known = listed(key).iter().any(|o| {
        let full = match &o.rank {
            Some(r) if !r.is_empty() => format!("{} ({r})", o.name),
            _ => o.name.clone(),
        };
        let this = full == ability || o.name == ability;
        taught |= this;
        this && knows(key, o, has)
    });
    known || (!taught && named(ability))
}

/// A service's state for a character of `level` having the spells `has`.
fn state_for(
    key: &'static str,
    l: &Listed,
    level: f64,
    has: &dyn Fn(i64) -> bool,
    named: Named,
) -> &'static str {
    if knows(key, l, has) {
        "used"
    } else if level >= l.level
        && l.abilities
            .iter()
            .all(|a| ability_known(key, a, has, named))
    {
        "available"
    } else {
        "unavailable"
    }
}

/// A service's state for the player: its level, its spells (learned, in
/// the spellbook).
fn state_of(vm: &Vm, key: &'static str, l: &Listed) -> &'static str {
    let has = |spell: i64| {
        engine(vm).world.trainers.learned.contains(&spell)
            || crate::stats::spellbook_slot_of(vm, spell as f64).is_some()
    };
    let named = |ability: &str| crate::stats::spellbook_has_named(vm, ability);
    let level = engine(vm).units.list[units::PLAYER.0].level;
    state_for(key, l, level, &has, &named)
}

/// An ability a service needs, known to the player.
fn player_knows_ability(vm: &Vm, key: &'static str, ability: &str) -> bool {
    let has = |spell: i64| {
        engine(vm).world.trainers.learned.contains(&spell)
            || crate::stats::spellbook_slot_of(vm, spell as f64).is_some()
    };
    let named = |ability: &str| crate::stats::spellbook_has_named(vm, ability);
    ability_known(key, ability, &has, &named)
}

fn services(vm: &mut Vm) -> Vec<Service> {
    let t = &engine(vm).world.trainers;
    if !t.listed {
        return vec![];
    }
    if let Some(key) = t.key {
        let shown = t.shown.clone();
        let loading = t.loading.clone();
        let list = listed(key);
        return shown
            .into_iter()
            .map(|i| {
                let l = &list[i];
                Service {
                    spell: l.spell,
                    name: vm.str(&l.name),
                    icon: Value::Num(l.icon),
                    level: l.level,
                    rank: match &l.rank {
                        Some(r) => vm.str(r),
                        None => Value::Nil,
                    },
                    cost: l.cost,
                    skill_line: vm.str(&l.line),
                    line_index: tab_of(key, &l.line),
                    rank_data: loading.get(&l.spell).copied().or(l.pending),
                    recorded: Some((key, i + 1)),
                    abilities: l.abilities.clone(),
                }
            })
            .collect();
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
            recorded: None,
            abilities: Vec::new(),
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
    if let Some((key, i)) = s.recorded {
        return state_of(vm, key, &listed(key)[i - 1]);
    }
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
    let key = t.key;
    let greeting = t
        .open
        .and_then(|n| t.defined.get(&n.0).copied())
        .map(|d| field(vm, d, "greeting"));
    Ok(match (greeting, key) {
        (Some(Value::Str(s)), _) => vec![Value::Str(s)],
        (_, Some(key)) => answer_values(vm, key, "GetTrainerGreetingText()"),
        _ => vec![vm.str("")],
    })
}

/// A recorded answer as values (its colors Blizzard's, `CreateColor`'s).
fn answer_values(vm: &mut Vm, key: &str, call: &str) -> Vec<Value> {
    match data().get(&(key, call)) {
        Some(answer) => crate::recorded::parse_answer(vm, answer),
        None => Vec::new(),
    }
}

/// A recorded service's answer to `name(i, …)` as its data has it; an
/// explicit service's `otherwise`.
fn per_service(
    vm: &mut Vm,
    c: &NativeCall,
    name: &str,
    otherwise: fn(&mut Vm, &Service) -> Vec<Value>,
) -> Ret {
    let Some((_, s)) = service(vm, c) else {
        return Ok(vec![]);
    };
    match s.recorded {
        Some((key, i)) => Ok(answer_values(vm, key, &format!("{name}({i})"))),
        None => Ok(otherwise(vm, &s)),
    }
}

fn get_trainer_service_description(vm: &mut Vm, c: NativeCall) -> Ret {
    per_service(vm, &c, "GetTrainerServiceDescription", |_, _| vec![])
}

fn get_trainer_service_icon(vm: &mut Vm, c: NativeCall) -> Ret {
    per_service(vm, &c, "GetTrainerServiceIcon", |_, s| vec![s.icon])
}

fn get_trainer_service_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    per_service(vm, &c, "GetTrainerServiceItemLink", |_, _| vec![])
}

fn get_trainer_service_skill_req(vm: &mut Vm, c: NativeCall) -> Ret {
    per_service(vm, &c, "GetTrainerServiceSkillReq", |_, _| vec![])
}

fn get_trainer_service_tooltip(vm: &mut Vm, c: NativeCall) -> Ret {
    per_service(vm, &c, "C_TooltipInfo.GetTrainerService", |_, _| vec![])
}

/// `GetTrainerServiceLevelReq(i)`: the level it needs.
fn get_trainer_service_level_req(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, s)) = service(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(vec![Value::Num(s.level)])
}

fn get_trainer_service_num_ability_req(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, s)) = service(vm, &c) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    Ok(vec![Value::Num(s.abilities.len() as f64)])
}

/// `GetTrainerServiceAbilityReq(i, j)`: the ability's name, whether it is
/// known.
fn get_trainer_service_ability_req(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, s)) = service(vm, &c) else {
        return Ok(vec![]);
    };
    let (Some((key, _)), Value::Num(j)) = (s.recorded, vm.hidden(c.arg(2))) else {
        return Ok(vec![]);
    };
    let Some(ability) = s.abilities.get((j as usize).wrapping_sub(1)) else {
        return Ok(vec![]);
    };
    let known = player_knows_ability(vm, key, ability);
    Ok(vec![vm.str(ability), Value::Bool(known)])
}

/// `GetTrainerServiceTypeFilter(type)`: whether it is shown.
fn get_trainer_service_type_filter(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(t) = vm.hidden(c.arg(1)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let t = String::from_utf8_lossy(vm.str_bytes(t)).into_owned();
    Ok(vec![Value::Bool(!filtered_out(vm).contains(t.as_str()))])
}

/// `SetTrainerServiceTypeFilter(type, on)`: the list again, its update
/// (unverified).
fn set_trainer_service_type_filter(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(t) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    let t = String::from_utf8_lossy(vm.str_bytes(t)).into_owned();
    let Some(t) = FILTERS.iter().copied().find(|f| *f == t) else {
        return Ok(vec![]);
    };
    let on = vm.hidden(c.arg(2)).truthy();
    let mut off = filtered_out(vm);
    if on {
        off.remove(t);
    } else {
        off.insert(t);
    }
    engine_mut(vm).world.trainers.filtered_out = Some(off);
    if engine(vm).world.trainers.open.is_some() {
        refilter(vm);
        fire(vm, "TRAINER_UPDATE", &[]);
    }
    Ok(vec![])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
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
        widgets::keep(vm, sub_type);
        widgets::keep(vm, s.icon);
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
        // A class trainer's: a recorded one's as its data has them; an
        // explicit one's (unverified) no skill or ability required, no
        // step.
        ("GetTrainerServiceSkillReq", get_trainer_service_skill_req),
        (
            "GetTrainerServiceNumAbilityReq",
            get_trainer_service_num_ability_req,
        ),
        (
            "GetTrainerServiceAbilityReq",
            get_trainer_service_ability_req,
        ),
        ("GetTrainerServiceStepIndex", nothing),
        (
            "GetTrainerServiceTypeFilter",
            get_trainer_service_type_filter,
        ),
        (
            "SetTrainerServiceTypeFilter",
            set_trainer_service_type_filter,
        ),
        (
            "GetTrainerServiceDescription",
            get_trainer_service_description,
        ),
        ("GetTrainerServiceIcon", get_trainer_service_icon),
        ("GetTrainerServiceItemLink", get_trainer_service_item_link),
        ("GetTrainerServiceLevelReq", get_trainer_service_level_req),
        (
            "C_TooltipInfo.GetTrainerService",
            get_trainer_service_tooltip,
        ),
        ("GetTrainerTradeskillRankValues", nothing),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every recorded class trainer's services again from the character:
    /// each one's state, level and needed abilities as listed, the
    /// character knowing the services listed used, at its level (recorded,
    /// else the one that fits).
    #[test]
    fn the_recorded_services_follow_from_the_character() {
        let mut keys: Vec<&'static str> = data().keys().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        keys.dedup();
        assert!(!keys.is_empty());
        let mut wrong = Vec::new();
        for key in keys {
            let list = listed(key);
            let theirs: Vec<(String, Vec<(String, bool)>)> = (1..=list.len())
                .map(|i| {
                    let info = recorded(key, &format!("GetTrainerServiceInfo({i})"));
                    let state = match info.get(1) {
                        Some(Rv::Str(s)) => s.clone(),
                        _ => String::new(),
                    };
                    let abilities = (1..=list[i - 1].abilities.len())
                        .map(|j| {
                            match recorded(key, &format!("GetTrainerServiceAbilityReq({i}, {j})"))
                                .as_slice()
                            {
                                [Rv::Str(n), Rv::Bool(met)] => (n.clone(), *met),
                                _ => (String::new(), false),
                            }
                        })
                        .collect();
                    (state, abilities)
                })
                .collect();
            let used: Vec<i64> = list
                .iter()
                .zip(&theirs)
                .filter(|(_, (s, _))| s == "used")
                .map(|(l, _)| l.spell)
                .collect();
            let has = |spell: i64| used.contains(&spell);
            // Abilities outside the list (the spellbook's): as recorded.
            let named = |ability: &str| {
                theirs
                    .iter()
                    .flat_map(|(_, a)| a)
                    .any(|(n, met)| n == ability && *met)
            };
            let ours = |level: f64| -> Vec<(String, Vec<(String, bool)>)> {
                list.iter()
                    .map(|l| {
                        (
                            state_for(key, l, level, &has, &named).to_string(),
                            l.abilities
                                .iter()
                                .map(|a| (a.clone(), ability_known(key, a, &has, &named)))
                                .collect(),
                        )
                    })
                    .collect()
            };
            let misses = |l: f64| ours(l).iter().zip(&theirs).filter(|(o, t)| o != t).count();
            let level = match recorded(key, r#"UnitLevel("player")"#).first() {
                Some(Rv::Num(n)) => *n,
                _ => (1..=60)
                    .map(f64::from)
                    .min_by_key(|l| misses(*l))
                    .unwrap_or(1.0),
            };
            for (i, (o, t)) in ours(level).iter().zip(&theirs).enumerate() {
                if o != t {
                    wrong.push(format!("{key} {} at {level}: {o:?}, recorded {t:?}", i + 1));
                }
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// A service's level is the level its spell is learned at, wherever
    /// both are recorded, a passive's aside (what fills the levels the
    /// probing character's known services lack).
    #[test]
    fn a_service_s_level_is_its_spell_s() {
        let mut checked = 0;
        let mut wrong = Vec::new();
        for ((key, call), answer) in data() {
            let Some(i) = call
                .strip_prefix("GetTrainerServiceLevelReq(")
                .and_then(|r| r.strip_suffix(')'))
            else {
                continue;
            };
            let level = match Rv::answer(answer).first() {
                Some(Rv::Num(n)) if *n > 0.0 => *n,
                _ => continue,
            };
            let tip = recorded(key, &format!("C_TooltipInfo.GetTrainerService({i})"));
            let Some(Rv::Num(spell)) = tip.first().and_then(|t| t.get("id")) else {
                continue;
            };
            let info = recorded(key, &format!("GetTrainerServiceInfo({i})"));
            if matches!(info.get(4), Some(Rv::Str(r)) if r == "Passive") {
                continue;
            }
            let class = key.split('-').next().unwrap_or(key);
            if let Some(learned) = learned_level(class, *spell as i64) {
                checked += 1;
                if learned != level {
                    wrong.push(format!(
                        "{key} {i} (spell {spell}): {level}, learned {learned}"
                    ));
                }
            }
        }
        assert!(checked > 0);
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }
}
