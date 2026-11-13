//! The character's equipment sets (`C_EquipmentSet`), as a world sets them
//! (`mock.setEquipmentSets`), counted against the gear and bags the
//! character keeps (`character.rs`).
//!
//! Recorded (the full probe, 1.60.1: a character with two sets of all ten
//! items it wore, none ignored, no spec assigned): the IDs from 0 in the
//! order made; `GetEquipmentSetInfo` name, icon, ID, whether all are worn,
//! the items, those worn, in the bags, lost, ignored; `GetItemIDs` and
//! `GetItemLocations` by gear slot (a worn item's location the player's
//! slot, `ITEM_INVENTORY_LOCATION_PLAYER` plus the slot); every one of the
//! 19 slots in `GetIgnoredSlots`; nothing for an ID without a set;
//! `GetEquipmentSetForSpec(0)` the first set while none has a spec, and
//! `GetEquipmentSetAssignedSpec` nothing.
//!
//! Unverified: an item in the bags or lost (its location, the counts), a
//! set with a spec, making, changing and using sets (accepted, not kept).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut};

/// A saved set.
#[derive(Clone, Debug, Default)]
pub struct EquipmentSet {
    pub id: i64,
    pub name: String,
    pub icon: f64,
    /// By gear slot: the item ID.
    pub items: std::collections::BTreeMap<i64, i64>,
    /// The gear slots the set leaves alone.
    pub ignored: std::collections::BTreeSet<i64>,
    /// The spec it is assigned to (0: none).
    pub spec: i64,
}

/// The gear slots a set covers (`INVSLOT_FIRST_EQUIPPED` to the tabard).
const SLOTS: std::ops::RangeInclusive<i64> = 1..=19;
/// Blizzard's `ITEM_INVENTORY_LOCATION_PLAYER`, `_BAGS` and the bag's
/// shift (`ITEM_INVENTORY_BAG_BIT_OFFSET`).
const LOCATION_PLAYER: i64 = 0x0010_0000;
const LOCATION_BAGS: i64 = 0x0020_0000;
const BAG_BIT_OFFSET: i64 = 8;

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn num(vm: &Vm, v: Value) -> Option<f64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn sets(vm: &Vm) -> &[EquipmentSet] {
    &engine(vm).world.character.equipment_sets
}

/// The set an ID argument names.
fn set_of(vm: &Vm, c: &NativeCall) -> Option<EquipmentSet> {
    let id = num(vm, c.arg(1))?.trunc() as i64;
    sets(vm).iter().find(|s| s.id == id).cloned()
}

/// Where a set's item is: worn in its slot, else in the bags, else lost.
enum Whereabouts {
    Worn,
    Bag(i64, usize),
    Lost,
}

fn whereabouts(vm: &Vm, slot: i64, id: i64) -> Whereabouts {
    let ch = &engine(vm).world.character;
    if ch.equipment.get(&slot).is_some_and(|h| h.id == id) {
        return Whereabouts::Worn;
    }
    for (b, bag) in &ch.bags {
        if let Some((s, _)) = bag.items.iter().find(|(_, h)| h.id == id) {
            return Whereabouts::Bag(*b, *s);
        }
    }
    Whereabouts::Lost
}

/// `mock.setEquipmentSets({ { name, icon, items = { [slot] = itemID },
/// ignored = { slot, … }, spec, id }, … })`: the character's sets, their
/// IDs from 0 in order unless given. A world's state: no event.
pub fn control_set(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(list) = c.arg(1) else {
        return Err(vm.error("setEquipmentSets: a list of sets expected"));
    };
    let mut out = Vec::new();
    let n = vm.heap.table(list).border();
    for i in 1..=n {
        let Value::Table(t) = vm.heap.table(list).get(Value::Num(i as f64)) else {
            continue;
        };
        let name = match field(vm, t, "name") {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => return Err(vm.error("setEquipmentSets: each set's name expected")),
        };
        let mut number = |key: &str| {
            let v = field(vm, t, key);
            num(vm, v)
        };
        let id = number("id").map_or(out.len() as i64, |n| n as i64);
        let icon = number("icon").unwrap_or(134400.0);
        let spec = number("spec").unwrap_or(0.0) as i64;
        let mut set = EquipmentSet {
            id,
            name,
            icon,
            spec,
            ..Default::default()
        };
        if let Value::Table(items) = field(vm, t, "items") {
            for (k, v) in vm.heap.table(items).entries().collect::<Vec<_>>() {
                if let (Some(slot), Some(item)) = (num(vm, k), num(vm, v)) {
                    set.items.insert(slot as i64, item as i64);
                }
            }
        }
        if let Value::Table(ignored) = field(vm, t, "ignored") {
            for (_, v) in vm.heap.table(ignored).entries().collect::<Vec<_>>() {
                if let Some(slot) = num(vm, v) {
                    set.ignored.insert(slot as i64);
                }
            }
        }
        out.push(set);
    }
    engine_mut(vm).world.character.equipment_sets = out;
    Ok(vec![])
}

/// `C_EquipmentSet.GetEquipmentSetIDs()`: the sets' IDs.
fn get_equipment_set_ids(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ids: Vec<i64> = sets(vm).iter().map(|s| s.id).collect();
    let t = vm.new_table();
    for (i, id) in ids.into_iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(i as f64 + 1.0), Value::Num(id as f64));
    }
    Ok(vec![Value::Table(t)])
}

fn get_num_equipment_sets(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(sets(vm).len() as f64)])
}

/// `C_EquipmentSet.GetEquipmentSetInfo(id)`: name, icon, ID, all worn,
/// the items, those worn, in the bags, lost, the slots ignored.
fn get_equipment_set_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(set) = set_of(vm, &c) else {
        return Ok(vec![]);
    };
    let (mut worn, mut bags, mut lost) = (0usize, 0usize, 0usize);
    for (slot, id) in &set.items {
        match whereabouts(vm, *slot, *id) {
            Whereabouts::Worn => worn += 1,
            Whereabouts::Bag(..) => bags += 1,
            Whereabouts::Lost => lost += 1,
        }
    }
    let name = vm.str(&set.name);
    Ok(vec![
        name,
        Value::Num(set.icon),
        Value::Num(set.id as f64),
        Value::Bool(worn == set.items.len()),
        Value::Num(set.items.len() as f64),
        Value::Num(worn as f64),
        Value::Num(bags as f64),
        Value::Num(lost as f64),
        Value::Num(set.ignored.len() as f64),
    ])
}

/// `C_EquipmentSet.GetEquipmentSetID(name)`: the set of that name's ID.
fn get_equipment_set_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(s) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    Ok(sets(vm)
        .iter()
        .find(|s| s.name == name)
        .map(|s| vec![Value::Num(s.id as f64)])
        .unwrap_or_default())
}

/// `C_EquipmentSet.GetItemIDs(id)`: the set's items by gear slot.
fn get_item_ids(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(set) = set_of(vm, &c) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    for (slot, id) in &set.items {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(*slot as f64), Value::Num(*id as f64));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_EquipmentSet.GetItemLocations(id)`: where each of the set's items
/// is, by gear slot (unverified for the bags'; a lost item's -1, as
/// Blizzard's `EQUIPMENT_SET_ITEM_MISSING`).
fn get_item_locations(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(set) = set_of(vm, &c) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    for (slot, id) in &set.items {
        let location = match whereabouts(vm, *slot, *id) {
            Whereabouts::Worn => LOCATION_PLAYER | slot,
            Whereabouts::Bag(b, s) => {
                LOCATION_PLAYER | LOCATION_BAGS | (b << BAG_BIT_OFFSET) | s as i64
            }
            Whereabouts::Lost => -1,
        };
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(*slot as f64), Value::Num(location as f64));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_EquipmentSet.GetIgnoredSlots(id)`: every gear slot, ignored or not.
fn get_ignored_slots(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(set) = set_of(vm, &c) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    for slot in SLOTS {
        let _ = vm.heap.table_mut(t).set(
            Value::Num(slot as f64),
            Value::Bool(set.ignored.contains(&slot)),
        );
    }
    Ok(vec![Value::Table(t)])
}

/// `C_EquipmentSet.GetEquipmentSetAssignedSpec(id)`: the set's spec,
/// nothing without one.
fn get_equipment_set_assigned_spec(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match set_of(vm, &c) {
        Some(set) if set.spec > 0 => vec![Value::Num(set.spec as f64)],
        _ => vec![],
    })
}

/// `C_EquipmentSet.GetEquipmentSetForSpec(specIndex)`: the first set
/// assigned to that spec, a set without one counting as spec 0 (recorded:
/// 0 answered the first set, 1 nothing; one sample).
fn get_equipment_set_for_spec(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(spec) = num(vm, c.arg(1)).map(|n| n.trunc() as i64) else {
        return Ok(vec![]);
    };
    Ok(sets(vm)
        .iter()
        .find(|s| s.spec == spec)
        .map(|s| vec![Value::Num(s.id as f64)])
        .unwrap_or_default())
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_EquipmentSet.GetEquipmentSetIDs", get_equipment_set_ids),
        ("C_EquipmentSet.GetNumEquipmentSets", get_num_equipment_sets),
        ("C_EquipmentSet.GetEquipmentSetInfo", get_equipment_set_info),
        ("C_EquipmentSet.GetEquipmentSetID", get_equipment_set_id),
        ("C_EquipmentSet.GetItemIDs", get_item_ids),
        ("C_EquipmentSet.GetItemLocations", get_item_locations),
        ("C_EquipmentSet.GetIgnoredSlots", get_ignored_slots),
        (
            "C_EquipmentSet.GetEquipmentSetAssignedSpec",
            get_equipment_set_assigned_spec,
        ),
        (
            "C_EquipmentSet.GetEquipmentSetForSpec",
            get_equipment_set_for_spec,
        ),
    ]
}
