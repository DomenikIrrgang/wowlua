//! What the core needs of wowlua-engine's `standins`.

use wowlua_vm::Vm;

use crate::hooks;

pub fn install(vm: &mut Vm) {
    (hooks::get().standins_install)(vm)
}
