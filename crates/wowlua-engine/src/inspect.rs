//! Inspecting another player, as the client recorded a Horde paladin
//! inspecting a hunter (`/stdtest engine scenario inspect`, a ping near
//! 0.5 s): `unit:setInspect({ spec, itemLevel, items = { [slot] = { link,
//! cached } }, received = { itemID, … }, config })` is what the server
//! knows of it; `NotifyInspect(unit)`: a round trip later
//! `INSPECT_HONOR_UPDATE()` and `INSPECT_READY(guid)` (the gear's links
//! the client has cached only), the first time then the other items' infos
//! one after another (`GET_ITEM_INFO_RECEIVED(id, true)`, each followed by
//! `INSPECT_READY(guid)`; every link known from the first).
//!
//! The answers: `GetInspectSpecialization`, `C_PaperDollInfo.
//! GetInspectItemLevel`, the unit's `GetInventoryItemLink`,
//! `C_Traits.GetConfigInfo(-1)` (the inspected's talent config).
//!
//! The inspected config's nodes (`-1`): the recorded character's the test
//! names (`traitCharacter`), else the tree's owner's with nothing bought
//! (`traits.rs`).
//!
//! Unverified: inspecting out of range, `ClearInspectPlayer`, the inspected
//! nodes' ranks (the recorded character's at its probe).

use std::collections::BTreeMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// The items' infos after the first `INSPECT_READY` (recorded).
const ITEMS_AFTER: f64 = 0.007;
/// The inspect talent config's ID (`C_Traits.GetConfigInfo(-1)`).
const INSPECT_CONFIG: f64 = -1.0;

#[derive(Default)]
pub struct Inspect {
    /// The units' data: unit → the test's table.
    pub data: BTreeMap<usize, TableRef>,
    /// Inspected: unit → every link known.
    pub done: BTreeMap<usize, bool>,
    pub last: Option<UnitId>,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn inspect(vm: &Vm) -> &Inspect {
    &engine(vm).world.inspect
}

fn inspect_mut(vm: &mut Vm) -> &mut Inspect {
    &mut engine_mut(vm).world.inspect
}

/// `unit:setInspect(data)`.
pub fn control_set_inspect(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("setInspect: a table of what the server knows expected"));
    };
    widgets::keep(vm, Value::Table(t));
    inspect_mut(vm).data.insert(me.0, t);
    Ok(vec![])
}

fn guid(vm: &mut Vm, id: UnitId) -> Value {
    let g = engine(vm).units.list[id.0].guid.clone();
    vm.str(&g)
}

/// `NotifyInspect(unit)`: the server's answer a round trip later.
fn notify_inspect(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = units::unit_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    if !inspect(vm).data.contains_key(&id.0) {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    let native = vm.native_with("inspectReady", ready, vec![Value::Num(id.0 as f64)]);
    crate::clock::after_engine(vm, delay, native);
    Ok(vec![])
}

fn unit_of(c: &NativeCall) -> UnitId {
    match c.upvals.first() {
        Some(Value::Num(n)) => UnitId(*n as usize),
        _ => UnitId(0),
    }
}

fn ready(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_of(&c);
    let first = !inspect(vm).done.contains_key(&id.0);
    inspect_mut(vm).last = Some(id);
    inspect_mut(vm).done.entry(id.0).or_insert(false);
    fire(vm, "INSPECT_HONOR_UPDATE", &[]);
    let g = guid(vm, id);
    fire(vm, "INSPECT_READY", &[g]);
    if first {
        let native = vm.native_with("inspectItems", items_received, c.upvals.to_vec());
        crate::clock::after_engine(vm, ITEMS_AFTER, native);
    }
    Ok(vec![])
}

/// The items' infos one after another, each with `INSPECT_READY`.
fn items_received(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_of(&c);
    inspect_mut(vm).done.insert(id.0, true);
    let Some(t) = inspect(vm).data.get(&id.0).copied() else {
        return Ok(vec![]);
    };
    let Value::Table(list) = field(vm, t, "received") else {
        return Ok(vec![]);
    };
    let n = vm.heap.table(list).border();
    for i in 1..=n {
        let item = vm.heap.table(list).get(Value::Num(i as f64));
        fire(vm, "GET_ITEM_INFO_RECEIVED", &[item, Value::Bool(true)]);
        let g = guid(vm, id);
        fire(vm, "INSPECT_READY", &[g]);
    }
    Ok(vec![])
}

/// The inspected data of a unit argument, once its answer came.
fn data_of(vm: &mut Vm, c: &NativeCall) -> Option<(UnitId, TableRef)> {
    let id = units::unit_arg(vm, c, 1)?;
    if !inspect(vm).done.contains_key(&id.0) {
        return None;
    }
    inspect(vm).data.get(&id.0).map(|t| (id, *t))
}

fn get_inspect_specialization(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match data_of(vm, &c) {
        Some((_, t)) => vec![field(vm, t, "spec")],
        None => vec![Value::Num(0.0)],
    })
}

fn get_inspect_item_level(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match data_of(vm, &c) {
        Some((_, t)) => vec![field(vm, t, "itemLevel")],
        None => vec![Value::Num(0.0)],
    })
}

/// An inspected unit's gear link: the cached ones at first, every one once
/// the items' infos came.
pub fn item_link(vm: &mut Vm, c: &NativeCall) -> Option<Vec<Value>> {
    let id = units::unit_arg(vm, c, 1)?;
    if id == units::PLAYER {
        return None;
    }
    let (_, t) = data_of(vm, c)?;
    let all = inspect(vm).done.get(&id.0).copied().unwrap_or(false);
    let Value::Num(slot) = vm.hidden(c.arg(2)) else {
        return Some(vec![]);
    };
    let Value::Table(items) = field(vm, t, "items") else {
        return Some(vec![]);
    };
    let Value::Table(item) = vm.heap.table(items).get(Value::Num(slot)) else {
        return Some(vec![]);
    };
    if !all && !field(vm, item, "cached").truthy() {
        return Some(vec![]);
    }
    Some(vec![field(vm, item, "link")])
}

/// `C_Traits.GetConfigInfo(-1)`: the inspected's config (the traits'
/// behaviour asks).
pub fn config_info(vm: &mut Vm, c: &NativeCall) -> Option<Vec<Value>> {
    if let Value::Num(n) = vm.hidden(c.arg(1))
        && n == INSPECT_CONFIG
        && let Some(id) = inspect(vm).last
        && let Some(t) = inspect(vm).data.get(&id.0).copied()
        && let Value::Table(config) = field(vm, t, "config")
    {
        return Some(vec![widgets::answer_copy(vm, Value::Table(config))]);
    }
    None
}

/// The recorded character the inspected unit is (`setInspect`'s
/// `traitCharacter`: its class config's ID), if the test said.
pub fn trait_character(vm: &mut Vm) -> Option<i64> {
    let id = inspect(vm).last?;
    let t = inspect(vm).data.get(&id.0).copied()?;
    match field(vm, t, "traitCharacter") {
        Value::Num(n) => Some(n as i64),
        _ => None,
    }
}

fn can_inspect(vm: &mut Vm, c: NativeCall) -> Ret {
    let ok = units::unit_arg(vm, &c, 1).is_some_and(|id| inspect(vm).data.contains_key(&id.0));
    Ok(vec![Value::Bool(ok)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("NotifyInspect", notify_inspect),
        ("GetInspectSpecialization", get_inspect_specialization),
        // What the global becomes once Blizzard_PlayerSpells loads.
        (
            "C_SpecializationInfo.GetInspectSpecialization",
            get_inspect_specialization,
        ),
        (
            "C_PaperDollInfo.GetInspectItemLevel",
            get_inspect_item_level,
        ),
        ("CanInspect", can_inspect),
    ]
}
