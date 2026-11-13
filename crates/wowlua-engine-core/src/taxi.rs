//! The flight masters' state (wowlua-engine's `taxi` answers from it).

use wowlua_vm::{TableRef, Vm};

use crate::hooks;
use crate::units::UnitId;

pub fn open(vm: &mut Vm, npc: UnitId) {
    (hooks::get().taxi_open)(vm, npc)
}

#[derive(Default)]
pub struct Taxis {
    /// The flight masters' maps: unit → the test's table.
    pub defined: std::collections::BTreeMap<usize, TableRef>,
    pub open: Option<UnitId>,
    /// The flights taken: a landing of an earlier one is stale.
    pub flights: u32,
}
