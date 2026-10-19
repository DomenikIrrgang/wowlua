//! A dungeon's or raid's instance and its boss encounters. No recording
//! has either yet: what this module does is modelled on the API
//! documentation (`ENCOUNTER_START`'s and `ENCOUNTER_END`'s payloads,
//! `GetInstanceInfo`'s results) and Blizzard's UI code (the boss frames
//! follow `INSTANCE_ENCOUNTER_ENGAGE_UNIT` and `UnitExists("bossN")`), all
//! of it unverified.
//!
//! `mock.enterInstance(kind, { name, instanceID, difficultyID,
//! difficultyName, maxPlayers })`: the map restricted (as before), and with
//! a `kind` the player in an instance (`IsInInstance`, `GetInstanceInfo`;
//! `UPDATE_INSTANCE_INFO`, `PLAYER_DIFFICULTY_CHANGED`).
//! `mock.startEncounter({ id, name, difficultyID, groupSize, bosses })`:
//! `ENCOUNTER_START`, the bosses on the `bossN` tokens,
//! `INSTANCE_ENCOUNTER_ENGAGE_UNIT`; `mock.endEncounter(success)`: a
//! success's `BOSS_KILL`, `ENCOUNTER_END` with each boss's status, the
//! tokens cleared and `INSTANCE_ENCOUNTER_ENGAGE_UNIT` again.
//!
//! Unverified: the events' order, entering an instance's loading screen
//! (`PLAYER_ENTERING_WORLD`, the zone's events: `moveTo` tells those), a
//! boss joining mid-fight, the encounter's own frames (timelines,
//! objectives).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// The instance the player is in.
#[derive(Clone)]
pub struct Instance {
    pub name: String,
    /// `"party"`, `"raid"`, `"pvp"`, `"arena"`, `"scenario"`.
    pub kind: String,
    pub instance_id: f64,
    pub difficulty_id: f64,
    pub difficulty_name: String,
    pub max_players: f64,
}

/// The encounter under way.
pub struct Active {
    pub id: f64,
    pub name: String,
    pub difficulty_id: f64,
    pub group_size: f64,
    /// The bosses engaged, on the `bossN` tokens in order.
    pub bosses: Vec<UnitId>,
}

#[derive(Default)]
pub struct Encounters {
    pub instance: Option<Instance>,
    pub active: Option<Active>,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn get(vm: &mut Vm, t: Option<TableRef>, key: &str) -> Value {
    match t {
        Some(t) => {
            let k = vm.str(key);
            vm.heap.table(t).get(k)
        }
        None => Value::Nil,
    }
}

fn text(vm: &mut Vm, t: Option<TableRef>, key: &str, default: &str) -> String {
    match get(vm, t, key) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => default.to_string(),
    }
}

fn number(vm: &mut Vm, t: Option<TableRef>, key: &str, default: f64) -> f64 {
    match get(vm, t, key) {
        Value::Num(n) => n,
        _ => default,
    }
}

/// An instance kind's usual difficulty, its name and size (the
/// `DifficultyID` enum's normal ones; unverified for camelot).
fn defaults(kind: &str) -> (f64, &'static str, f64) {
    match kind {
        "raid" => (14.0, "Normal", 40.0),
        "pvp" => (0.0, "", 40.0),
        "arena" => (0.0, "", 5.0),
        "scenario" => (12.0, "Normal", 3.0),
        _ => (1.0, "Normal", 5.0),
    }
}

/// `mock.enterInstance(kind?, opts?)`: the map restricted; with a kind the
/// player in an instance of it.
pub fn control_enter_instance(vm: &mut Vm, c: NativeCall) -> Ret {
    units::set_restriction(vm, units::MAP, true);
    let Value::Str(kind) = c.arg(1) else {
        return Ok(vec![]);
    };
    let kind = String::from_utf8_lossy(vm.str_bytes(kind)).into_owned();
    let opts = match c.arg(2) {
        Value::Table(t) => Some(t),
        _ => None,
    };
    let (difficulty, difficulty_name, size) = defaults(&kind);
    let instance = Instance {
        name: text(vm, opts, "name", "Instance"),
        instance_id: number(vm, opts, "instanceID", 0.0),
        difficulty_id: number(vm, opts, "difficultyID", difficulty),
        difficulty_name: text(vm, opts, "difficultyName", difficulty_name),
        max_players: number(vm, opts, "maxPlayers", size),
        kind,
    };
    engine_mut(vm).world.encounters.instance = Some(instance);
    fire(vm, "UPDATE_INSTANCE_INFO", &[]);
    fire(vm, "PLAYER_DIFFICULTY_CHANGED", &[]);
    Ok(vec![])
}

/// `mock.leaveInstance()`: out of the instance (an encounter under way
/// ends unwon), the map unrestricted.
pub fn control_leave_instance(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine(vm).world.encounters.active.is_some() {
        end(vm, false);
    }
    let was_in = engine_mut(vm).world.encounters.instance.take().is_some();
    units::set_restriction(vm, units::MAP, false);
    if was_in {
        fire(vm, "UPDATE_INSTANCE_INFO", &[]);
        fire(vm, "PLAYER_DIFFICULTY_CHANGED", &[]);
    }
    Ok(vec![])
}

/// `mock.startEncounter({ id, name, difficultyID, groupSize, bosses })`.
pub fn control_start(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("startEncounter: a table of the encounter expected"));
    };
    if engine(vm).world.encounters.active.is_some() {
        return Err(vm.error("startEncounter: an encounter is under way"));
    }
    let t = Some(t);
    let instance = engine(vm).world.encounters.instance.clone();
    let mut bosses = Vec::new();
    if let Value::Table(list) = get(vm, t, "bosses") {
        let n = vm.heap.table(list).border();
        for i in 1..=n {
            let v = vm.heap.table(list).get(Value::Num(i as f64));
            match units::handle_of(vm, v) {
                Some(id) => bosses.push(id),
                None => return Err(vm.error("startEncounter: `bosses` are units")),
            }
        }
    }
    let active = Active {
        id: number(vm, t, "id", 0.0),
        name: text(vm, t, "name", "Encounter"),
        difficulty_id: number(
            vm,
            t,
            "difficultyID",
            instance.as_ref().map_or(1.0, |i| i.difficulty_id),
        ),
        group_size: number(
            vm,
            t,
            "groupSize",
            instance.as_ref().map_or(5.0, |i| i.max_players),
        ),
        bosses: bosses.clone(),
    };
    let args = [
        Value::Num(active.id),
        vm.str(&active.name),
        Value::Num(active.difficulty_id),
        Value::Num(active.group_size),
    ];
    engine_mut(vm).world.encounters.active = Some(active);
    fire(vm, "ENCOUNTER_START", &args);
    engine_mut(vm).units.bosses = bosses.into_iter().map(Some).collect();
    fire(vm, "INSTANCE_ENCOUNTER_ENGAGE_UNIT", &[]);
    Ok(vec![])
}

/// `mock.endEncounter(success)`.
pub fn control_end(vm: &mut Vm, c: NativeCall) -> Ret {
    if engine(vm).world.encounters.active.is_none() {
        return Err(vm.error("endEncounter: no encounter under way"));
    }
    end(vm, c.arg(1).truthy());
    Ok(vec![])
}

/// A creature's ID, from its GUID (`Creature-0-…-npcID-spawn`).
fn creature_id(guid: &str) -> f64 {
    guid.split('-')
        .nth(5)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}

fn end(vm: &mut Vm, success: bool) {
    let Some(active) = engine_mut(vm).world.encounters.active.take() else {
        return;
    };
    let name = vm.str(&active.name);
    if success {
        fire(vm, "BOSS_KILL", &[Value::Num(active.id), name]);
    }
    let status = vm.new_table();
    for (i, id) in active.bosses.iter().enumerate() {
        let (guid, boss_name, pct) = {
            let u = &engine(vm).units.list[id.0];
            let pct = if u.health_max > 0.0 {
                (u.health / u.health_max * 100.0).round()
            } else {
                0.0
            };
            (u.guid.clone(), u.name.clone(), pct)
        };
        let entry = vm.new_table();
        for (k, v) in [
            ("creatureID", Value::Num(creature_id(&guid))),
            ("creatureName", vm.str(&boss_name)),
            ("remainingHealthPercent", Value::Num(pct)),
        ] {
            let k = vm.str(k);
            let _ = vm.heap.table_mut(entry).set(k, v);
        }
        let _ = vm
            .heap
            .table_mut(status)
            .set(Value::Num(i as f64 + 1.0), Value::Table(entry));
    }
    fire(
        vm,
        "ENCOUNTER_END",
        &[
            Value::Num(active.id),
            name,
            Value::Num(active.difficulty_id),
            Value::Num(active.group_size),
            Value::Num(if success { 1.0 } else { 0.0 }),
            Value::Table(status),
        ],
    );
    engine_mut(vm).units.bosses.clear();
    fire(vm, "INSTANCE_ENCOUNTER_ENGAGE_UNIT", &[]);
}

/// `IsInInstance()`: in one and its kind; else `false, "none"` (recorded
/// outside any).
fn is_in_instance(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(match engine(vm).world.encounters.instance.clone() {
        Some(i) => vec![Value::Bool(true), vm.str(&i.kind)],
        None => vec![Value::Bool(false), vm.str("none")],
    })
}

/// `GetInstanceInfo()` in an instance: its name, kind, difficulty and its
/// name, size, no dynamic difficulty, its ID, the group's size, no LFG
/// dungeon (the documentation's order; unverified values).
pub fn instance_info(vm: &mut Vm) -> Option<Vec<Value>> {
    let i = engine(vm).world.encounters.instance.clone()?;
    Some(vec![
        vm.str(&i.name),
        vm.str(&i.kind),
        Value::Num(i.difficulty_id),
        vm.str(&i.difficulty_name),
        Value::Num(i.max_players),
        Value::Num(0.0),
        Value::Bool(false),
        Value::Num(i.instance_id),
        Value::Num(group_size(vm)),
        Value::Nil,
        Value::Bool(false),
    ])
}

/// The player's group's size (the player alone: 1).
fn group_size(vm: &Vm) -> f64 {
    if crate::party::grouped(vm) {
        engine(vm).units.group.len() as f64 + 1.0
    } else {
        1.0
    }
}

/// `C_InstanceEncounter.IsEncounterInProgress()`.
fn is_encounter_in_progress(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        engine(vm).world.encounters.active.is_some(),
    )])
}

/// `RequestRaidInfo()`: the server's answer a round trip later,
/// `UPDATE_INSTANCE_INFO()` (recorded, lfg: each time the group finder's
/// listing panel showed and asked, 0.35 to 0.6 s later, five times). One
/// answer per request (unverified).
fn request_raid_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    let native = vm.native("raidInfoAnswer", raid_info_answered);
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, native);
    Ok(vec![])
}

fn raid_info_answered(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "UPDATE_INSTANCE_INFO", &[]);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("RequestRaidInfo", request_raid_info),
        ("IsInInstance", is_in_instance),
        (
            "C_InstanceEncounter.IsEncounterInProgress",
            is_encounter_in_progress,
        ),
    ]
}
