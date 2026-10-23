//! Errors raised by Lua code or the VM: any Lua value, plus the traceback
//! captured where it was raised.

use crate::value::Value;

#[derive(Clone, Debug)]
pub struct LuaError {
    pub value: Value,
    pub traceback: String,
    /// Not an error: `coroutine.yield` unwinding to its `resume` (the
    /// values wait in the VM).
    pub yielded: bool,
    /// The debugger saw it raised (`Vm::error_hook`).
    pub seen: bool,
}

impl LuaError {
    pub fn new(value: Value) -> Self {
        LuaError {
            value,
            traceback: String::new(),
            yielded: false,
            seen: false,
        }
    }

    pub(crate) fn yield_() -> Self {
        LuaError {
            value: Value::Nil,
            traceback: String::new(),
            yielded: true,
            seen: false,
        }
    }
}
