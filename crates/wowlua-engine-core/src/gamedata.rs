//! The defined spells and items (wowlua-engine's `gamedata` answers from them).

use std::collections::HashMap;

use wowlua_vm::{TableRef, Value, Vm};

use crate::hooks;

pub fn item_data(vm: &mut Vm, id: i64) -> Option<TableRef> {
    (hooks::get().item_data)(vm, id)
}

pub fn spell_data(vm: &mut Vm, id: i64) -> Option<TableRef> {
    (hooks::get().spell_data)(vm, id)
}

pub fn spell_name(vm: &mut Vm, id: i64) -> Value {
    (hooks::get().spell_name)(vm, id)
}

pub fn item_id(vm: &mut Vm, v: Value) -> Option<i64> {
    (hooks::get().item_id)(vm, v)
}

pub fn scanned_spell(vm: &mut Vm, id: i64) -> Option<TableRef> {
    (hooks::get().scanned_spell)(vm, id)
}

/// The defined spells and items: ID → the test's table.
#[derive(Default)]
pub struct GameData {
    pub spells: HashMap<i64, TableRef>,
    pub items: HashMap<i64, TableRef>,
    /// The client's spell table's spells asked so far (`scanned_spell`).
    pub scanned: HashMap<i64, TableRef>,
    /// Items requested, in order.
    pub item_requests: Vec<i64>,
}
