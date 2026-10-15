//! Coroutines (decision T192): each has its own value stack and call
//! frames. The running thread's stack and frames live in the VM itself and
//! are swapped in and out on resume and yield.

use std::rc::Rc;

use crate::heap::{FuncRef, TableRef, UpvalRef};
use crate::proto::Proto;

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
    /// The function gave up its access to secrets (`dropsecretaccess`).
    pub secrets_dropped: bool,
}

#[derive(Default)]
pub struct ThreadState {
    pub stack: Vec<crate::taint::Slot>,
    pub taint: crate::taint::TaintState,
    pub top: usize,
    pub frames: Vec<Frame>,
    /// Open upvalues by stack slot, ascending.
    pub open_upvals: Vec<(usize, UpvalRef)>,
    /// Native calls running in this thread: each takes a call-info slot.
    pub natives: usize,
    /// The size of Lua's call-info array for this thread (`size_ci`; 0
    /// before the first call, meaning `BASIC_CI_SIZE`).
    pub ci_size: usize,
}

/// A new thread's call-info array (`BASIC_CI_SIZE`).
pub const BASIC_CI_SIZE: usize = 8;

impl ThreadState {
    pub fn ci_size(&self) -> usize {
        if self.ci_size == 0 {
            BASIC_CI_SIZE
        } else {
            self.ci_size
        }
    }

    /// Call-info slots in use above the base one (`L->ci - L->base_ci`).
    pub fn ci_used(&self) -> usize {
        self.frames.len() + self.natives
    }

    /// The collector shrinks an underused array (`checkstacksizes`, run as
    /// Lua 5.1 traverses a thread: twice a cycle), but not after an
    /// overflow.
    pub fn shrink_ci(&mut self) {
        for _ in 0..2 {
            let size = self.ci_size();
            if size > crate::vm::MAXCALLS {
                return;
            }
            if 4 * self.ci_used() < size && 2 * BASIC_CI_SIZE < size {
                self.ci_size = size / 2;
            }
        }
    }
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
