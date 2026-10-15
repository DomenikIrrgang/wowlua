//! Functions (Lua closures and native functions) and upvalue cells.

use std::rc::Rc;

use crate::error::LuaError;
use crate::heap::{TableRef, ThreadRef, UpvalRef};
use crate::proto::Proto;
use crate::value::Value;
use crate::vm::Vm;

/// A native function: receives its arguments, returns its results.
pub type NativeFn = fn(&mut Vm, NativeCall) -> Result<Vec<Value>, LuaError>;

/// One call of a native function.
pub struct NativeCall {
    pub args: Vec<Value>,
    /// The arguments' taints (their stack slots').
    pub taints: Vec<crate::taint::Taint>,
    /// The arguments' taints' origins.
    pub origins: Vec<crate::taint::Origin>,
    /// The native function's own upvalues (set when it was created).
    pub upvals: Rc<[Value]>,
    pub upval_taints: Rc<[crate::taint::Taint]>,
}

pub struct LuaFunction {
    pub proto: Rc<Proto>,
    pub upvals: Box<[UpvalRef]>,
    pub env: TableRef,
    /// The object's taint: calling a tainted closure taints the execution.
    pub taint: crate::taint::Taint,
}

pub struct NativeFunction {
    pub f: NativeFn,
    pub name: &'static str,
    pub upvals: Rc<[Value]>,
    pub upval_taints: Rc<[crate::taint::Taint]>,
    pub env: TableRef,
    pub taint: crate::taint::Taint,
}

pub enum Function {
    Lua(LuaFunction),
    Native(NativeFunction),
}

impl Function {
    pub fn set_taint(&mut self, t: crate::taint::Taint) {
        match self {
            Function::Lua(f) => f.taint = t,
            Function::Native(f) => f.taint = t,
        }
    }

    /// The function object's taint.
    pub fn taint(&self) -> crate::taint::Taint {
        match self {
            Function::Lua(f) => f.taint,
            Function::Native(f) => f.taint,
        }
    }

    pub fn env(&self) -> TableRef {
        match self {
            Function::Lua(f) => f.env,
            Function::Native(f) => f.env,
        }
    }

    pub fn set_env(&mut self, env: TableRef) {
        match self {
            Function::Lua(f) => f.env = env,
            Function::Native(f) => f.env = env,
        }
    }
}

/// An upvalue: open while it refers to a live stack slot of a thread,
/// closed once that slot's scope ends.
pub enum Upval {
    Open { thread: ThreadRef, idx: usize },
    Closed(crate::taint::Slot),
}
