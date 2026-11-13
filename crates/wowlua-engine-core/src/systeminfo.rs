//! What wowlua-engine's `systeminfo` does that the core asks for.

use wowlua_vm::Vm;

use crate::hooks;

pub fn tracking_aura_changed(vm: &mut Vm, spell: f64) {
    (hooks::get().tracking_aura_changed)(vm, spell)
}
