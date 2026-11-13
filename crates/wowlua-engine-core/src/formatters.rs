//! What the core needs of wowlua-engine's `formatters`.

use wowlua_vm::{LuaError, Value, Vm};

use crate::hooks;

pub fn format_with(vm: &mut Vm, formatter: Value, x: f64, secret: bool) -> Result<Value, LuaError> {
    (hooks::get().format_with)(vm, formatter, x, secret)
}
