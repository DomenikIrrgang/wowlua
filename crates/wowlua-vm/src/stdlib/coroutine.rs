//! Lua 5.1's coroutine library, on stackless threads (decision T192): a
//! yield unwinds the Rust stack back to `resume` and leaves the coroutine's
//! Lua frames in its thread; the next resume continues the interpreter on
//! them. As in Lua 5.1, a yield cannot cross a native call or metamethod.

use crate::error::LuaError;
use crate::function::{Function, NativeCall};
use crate::heap::ThreadRef;
use crate::stdlib::{Ret, register};
use crate::taint::Slot;
use crate::thread::{Status, Thread, ThreadState};
use crate::value::Value;
use crate::vm::Vm;

pub fn open(vm: &mut Vm) {
    register(
        vm,
        "coroutine",
        &[
            ("create", create),
            ("resume", resume),
            ("yield", yield_),
            ("status", status),
            ("wrap", wrap),
            ("running", running),
        ],
    );
}

fn create(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = c.arg(1);
    let ok = match f {
        Value::Func(fr) => {
            vm.rules.native_coroutines || matches!(vm.heap.func(fr), Function::Lua(_))
        }
        _ => false,
    };
    if !ok {
        return Err(vm.arg_error(1, "Lua function expected"));
    }
    let Value::Func(fr) = f else { unreachable!() };
    let t = vm.heap.alloc_thread(Thread {
        status: Status::Suspended,
        state: ThreadState::default(),
        start: Some(fr),
        yield_site: None,
        // A new thread shares its creator's global table, as in Lua.
        globals: vm.globals,
    });
    Ok(vec![Value::Thread(t)])
}

fn check_thread(vm: &mut Vm, c: &NativeCall) -> Result<ThreadRef, LuaError> {
    match c.arg(1) {
        Value::Thread(t) => Ok(t),
        _ => Err(vm.arg_error(1, "coroutine expected")),
    }
}

impl Vm {
    /// Resumes `co` with `args`: its yielded or returned values, or its
    /// error value.
    pub fn resume(&mut self, co: ThreadRef, args: Vec<Value>) -> Result<Vec<Value>, Value> {
        let args = args.into_iter().map(|v| self.cur.taint.fresh(v)).collect();
        self.resume_slots(co, args)
            .map(|r| r.into_iter().map(|s| s.v).collect())
            .map_err(|e| e.v)
    }

    /// [`resume`](Self::resume) with taints: the values cross as they are,
    /// and the taint state passes to the coroutine and back
    /// (`luaR_taintthread`).
    pub fn resume_slots(&mut self, co: ThreadRef, args: Vec<Slot>) -> Result<Vec<Slot>, Slot> {
        let st = self.heap.thread(co).status;
        if co == self.current || st != Status::Suspended {
            let msg = if st == Status::Dead {
                "cannot resume dead coroutine"
            } else {
                "cannot resume non-suspended coroutine"
            };
            let v = self.str(msg);
            return Err(self.cur.taint.fresh(v));
        }
        // Switch to the coroutine, with the caller's taint.
        let taint = self.cur.taint.saved();
        let caller = self.current;
        let caller_state = std::mem::take(&mut self.cur);
        self.heap.thread_mut(caller).state = caller_state;
        self.heap.thread_mut(caller).status = Status::Normal;
        self.heap.thread_mut(caller).globals = self.globals;
        self.globals = self.heap.thread(co).globals;
        self.cur = std::mem::take(&mut self.heap.thread_mut(co).state);
        self.cur.taint = taint;
        self.heap.thread_mut(co).status = Status::Running;
        self.current = co;
        self.resume_chain.push(caller);
        let saved_cstack = std::mem::take(&mut self.cstack);
        let saved_errfunc = self.errfunc.take();
        let saved_base = std::mem::replace(&mut self.co_base, self.run_depth + 1);

        let start = self.heap.thread_mut(co).start.take();
        let r = match start {
            Some(f) => {
                // The first resume calls the body with the arguments.
                let n = args.len();
                if self.cur.stack.len() < n + 64 {
                    self.cur.stack.resize(n + 64, Slot::NIL);
                }
                self.cur.stack[0] = self.cur.taint.fresh(Value::Func(f));
                for (i, &a) in args.iter().enumerate() {
                    self.cur.stack[1 + i] = self.cur.taint.copy(a);
                }
                self.cur.top = n + 1;
                self.call_at(0, n, -1, false)
            }
            None => {
                // The values become the results of the pending yield call.
                let (func, nresults) = self
                    .heap
                    .thread(co)
                    .yield_site
                    .expect("suspended at a yield");
                let n = if nresults < 0 {
                    args.len()
                } else {
                    nresults as usize
                };
                if self.cur.stack.len() < func + n + 1 {
                    self.cur.stack.resize(func + n + 64, Slot::NIL);
                }
                for i in 0..n {
                    self.cur.stack[func + i] = match args.get(i) {
                        Some(&a) => self.cur.taint.copy(a),
                        None => self.cur.taint.fresh(Value::Nil),
                    };
                }
                if nresults < 0 {
                    self.cur.top = func + n;
                }
                self.c_depth += 1;
                let r = self.execute();
                self.c_depth -= 1;
                r
            }
        };
        let outcome = match r {
            Ok(()) => {
                // The body returned: its results are at the bottom.
                let results = self.cur.stack[..self.cur.top].to_vec();
                self.heap.thread_mut(co).status = Status::Dead;
                Ok(results)
            }
            Err(e) if e.yielded => {
                let site = self.yield_site.take();
                self.heap.thread_mut(co).yield_site = site;
                self.heap.thread_mut(co).status = Status::Suspended;
                Ok(std::mem::take(&mut self.yield_values))
            }
            Err(e) => {
                self.heap.thread_mut(co).status = Status::Dead;
                Err(self.cur.taint.fresh(e.value))
            }
        };
        // Back to the caller, with the coroutine's taint.
        let taint = self.cur.taint.saved();
        self.cstack = saved_cstack;
        self.errfunc = saved_errfunc;
        self.co_base = saved_base;
        self.resume_chain.pop();
        let co_state = std::mem::take(&mut self.cur);
        self.heap.thread_mut(co).state = co_state;
        self.heap.thread_mut(co).globals = self.globals;
        self.globals = self.heap.thread(caller).globals;
        self.cur = std::mem::take(&mut self.heap.thread_mut(caller).state);
        self.cur.taint.restore(taint);
        self.heap.thread_mut(caller).status = Status::Running;
        self.current = caller;
        outcome
    }
}

fn resume(vm: &mut Vm, c: NativeCall) -> Ret {
    let co = check_thread(vm, &c)?;
    let r = match vm.resume_slots(co, c.slots(2)) {
        Ok(mut values) => {
            values.insert(0, vm.cur.taint.fresh(Value::Bool(true)));
            values
        }
        Err(e) => vec![vm.cur.taint.fresh(Value::Bool(false)), e],
    };
    Ok(vm.return_slots(r))
}

fn yield_(vm: &mut Vm, c: NativeCall) -> Ret {
    // Only the yield itself may be native, called straight from the
    // coroutine's own run (the main thread is never one). Lua 5.1 reports
    // all of it as crossing the boundary, without a position.
    let direct = vm.current != vm.main
        && vm.cstack.len() == 1
        && vm.cstack[0].from_lua
        && vm.run_depth == vm.co_base;
    if !direct {
        return Err(vm.rt_error("attempt to yield across metamethod/C-call boundary"));
    }
    vm.yield_values = c.slots(1);
    Err(LuaError::yield_())
}

fn status(vm: &mut Vm, c: NativeCall) -> Ret {
    let co = check_thread(vm, &c)?;
    let s = if co == vm.current {
        "running"
    } else {
        match vm.heap.thread(co).status {
            Status::Suspended => "suspended",
            Status::Normal => "normal",
            Status::Running => "running",
            Status::Dead => "dead",
        }
    };
    Ok(vec![vm.str(s)])
}

fn wrap(vm: &mut Vm, c: NativeCall) -> Ret {
    let r = create(vm, c)?;
    Ok(vec![vm.native_with("wrap", wrapped, r)])
}

fn wrapped(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Thread(co) = c.upvals[0] else {
        unreachable!()
    };
    match vm.resume_slots(co, c.slots(1)) {
        Ok(values) => Ok(vm.return_slots(values)),
        Err(e) => {
            let e = e.v;
            // String errors get the caller's position.
            if matches!(e, Value::Str(_) | Value::Num(_)) {
                let mut text = vm.where_(1);
                vm.append_strlike(&mut text, e);
                let msg = vm.bytes(&text);
                return Err(vm.raise(msg));
            }
            Err(vm.raise(e))
        }
    }
}

fn running(vm: &mut Vm, _c: NativeCall) -> Ret {
    if vm.current == vm.main {
        Ok(vec![Value::Nil])
    } else {
        Ok(vec![Value::Thread(vm.current)])
    }
}
