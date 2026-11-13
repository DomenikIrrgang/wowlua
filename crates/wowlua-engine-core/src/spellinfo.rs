//! The spells' answers of wowlua-engine's `spellinfo` the core asks for.

use wowlua_vm::{Value, Vm};

use crate::hooks;
use crate::units::UnitId;

pub fn macro_spell(vm: &mut Vm, id: Value) -> Option<f64> {
    (hooks::get().macro_spell)(vm, id)
}

pub fn spell_of(vm: &mut Vm, v: Value) -> Option<f64> {
    (hooks::get().spell_of)(vm, v)
}

pub fn in_range(vm: &mut Vm, id: f64, unit: Option<UnitId>) -> Value {
    (hooks::get().in_range)(vm, id, unit)
}

pub fn range_checks_tick(vm: &mut Vm) {
    (hooks::get().range_checks_tick)(vm)
}
