//! What the core needs of wowlua-engine's `questinfo`.

use wowlua_vm::{LuaError, Vm};

use crate::hooks;

pub fn navigation_frame(vm: &mut Vm) -> Result<(), LuaError> {
    (hooks::get().navigation_frame)(vm)
}

pub fn poi_map(vm: &mut Vm) -> f64 {
    (hooks::get().poi_map)(vm)
}

pub fn quest_poi_arrived(vm: &mut Vm, quest: i64) -> Result<(), LuaError> {
    (hooks::get().quest_poi_arrived)(vm, quest)
}
