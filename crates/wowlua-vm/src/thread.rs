//! Coroutines (decision T192): each has its own value stack and call
//! frames. The running thread's stack and frames live in the VM itself and
//! are swapped in and out on resume and yield.

use std::rc::Rc;

use crate::heap::{FuncRef, TableRef, UpvalRef};
use crate::proto::Proto;
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Created or yielded: can be resumed.
    Suspended,
    Running,
    /// Resumed another coroutine.
    Normal,
    Dead,
}

/// A Lua call frame.
#[derive(Clone)]
pub struct Frame {
    pub closure: FuncRef,
    pub proto: Rc<Proto>,
    /// The stack slot of the called function (results go here).
    pub func: usize,
    pub base: usize,
    pub pc: usize,
    /// Results the caller wants (-1: all).
    pub nresults: i32,
    pub nvarargs: usize,
    /// Reached through a tail call (shown in tracebacks).
    pub tail: bool,
    /// The first frame of a nested `execute` (a native function called Lua).
    pub entry: bool,
}

#[derive(Default)]
pub struct ThreadState {
    pub stack: Vec<Value>,
    pub top: usize,
    pub frames: Vec<Frame>,
    /// Open upvalues by stack slot, ascending.
    pub open_upvals: Vec<(usize, UpvalRef)>,
}

pub struct Thread {
    pub status: Status,
    /// The state while this thread is not running.
    pub state: ThreadState,
    /// The coroutine's function (before the first resume).
    pub start: Option<FuncRef>,
    /// Where the call to `coroutine.yield` was: its slot and the results
    /// it wants (the values of the next resume go there).
    pub yield_site: Option<(usize, i32)>,
    /// Its global table (`setfenv(0, t)`) while it is not running; the
    /// running thread's is the VM's.
    pub globals: TableRef,
}
