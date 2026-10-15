//! Errors raised by Lua code or the VM: any Lua value, plus the traceback
//! captured where it was raised.

use crate::value::Value;

#[derive(Clone, Debug)]
pub struct LuaError {
    pub value: Value,
    pub traceback: String,
}

impl LuaError {
    pub fn new(value: Value) -> Self {
        LuaError {
            value,
            traceback: String::new(),
        }
    }
}
