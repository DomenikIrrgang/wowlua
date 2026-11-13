//! `C_Item`'s answers by item location (Blizzard's `ItemLocation`: a bag
//! and slot, or a gear slot): whether an item is there, its ID, link,
//! name, icon, quality, count, bound, locked; from the bags and gear.
//!
//! Unverified: the answers for an empty location (none, as documented
//! nilable; `DoesItemExist` false).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::character::{self, Held};
use crate::engine;

fn num(vm: &mut Vm, v: Value) -> Option<f64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn field(vm: &mut Vm, v: Value, key: &str) -> Option<f64> {
    let Value::Table(t) = v else {
        return None;
    };
    let k = vm.str(key);
    let f = vm.heap.table(t).get(k);
    num(vm, f)
}

/// The item at a location, if any.
pub fn held_at(vm: &mut Vm, location: Value) -> Option<Held> {
    if let Some(slot) = field(vm, location, "equipmentSlotIndex") {
        return engine(vm)
            .world
            .character
            .equipment
            .get(&(slot as i64))
            .cloned();
    }
    let bag = field(vm, location, "bagID")?;
    let slot = field(vm, location, "slotIndex")?;
    engine(vm)
        .world
        .character
        .bags
        .get(&(bag as i64))
        .and_then(|b| b.items.get(&(slot as usize)))
        .cloned()
}

fn does_item_exist(vm: &mut Vm, c: NativeCall) -> Ret {
    let exists = held_at(vm, c.arg(1)).is_some();
    Ok(vec![Value::Bool(exists)])
}

fn get_item_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match held_at(vm, c.arg(1)) {
        Some(h) => vec![Value::Num(h.id as f64)],
        None => vec![],
    })
}

fn get_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(h) = held_at(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    Ok(character::link_of(vm, &h)
        .map(|l| vec![l])
        .unwrap_or_default())
}

fn data_of(vm: &mut Vm, c: &NativeCall, key: &str) -> Ret {
    let Some(h) = held_at(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    Ok(vec![character::item_field(vm, h.id, key)])
}

fn get_item_name(vm: &mut Vm, c: NativeCall) -> Ret {
    data_of(vm, &c, "name")
}

fn get_item_icon(vm: &mut Vm, c: NativeCall) -> Ret {
    data_of(vm, &c, "icon")
}

fn get_item_quality(vm: &mut Vm, c: NativeCall) -> Ret {
    data_of(vm, &c, "quality")
}

fn get_stack_count(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match held_at(vm, c.arg(1)) {
        Some(h) => vec![Value::Num(h.count)],
        None => vec![],
    })
}

/// `C_Item.IsBound(location)`: a bag item as it was given; a worn item
/// bound, as wearing binds it (the full probe, 1.60.1: the recorded
/// character's shoulders; unverified: an item that never binds, its data's
/// `bindType` 0, left unbound).
fn is_bound(vm: &mut Vm, c: NativeCall) -> Ret {
    let worn = field(vm, c.arg(1), "equipmentSlotIndex").is_some();
    let bound = held_at(vm, c.arg(1)).is_some_and(|h| {
        h.bound || (worn && num_of(character::item_field(vm, h.id, "bindType")) != Some(0.0))
    });
    Ok(vec![Value::Bool(bound)])
}

fn num_of(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn is_locked(vm: &mut Vm, c: NativeCall) -> Ret {
    let locked = held_at(vm, c.arg(1)).is_some_and(|h| h.locked);
    Ok(vec![Value::Bool(locked)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_Item.DoesItemExist", does_item_exist),
        ("C_Item.GetItemID", get_item_id),
        ("C_Item.GetItemLink", get_item_link),
        ("C_Item.GetItemName", get_item_name),
        ("C_Item.GetItemIcon", get_item_icon),
        ("C_Item.GetItemQuality", get_item_quality),
        ("C_Item.GetStackCount", get_stack_count),
        ("C_Item.IsBound", is_bound),
        ("C_Item.IsLocked", is_locked),
    ]
}
