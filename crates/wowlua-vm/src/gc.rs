//! The garbage collector (decision T191): stop-the-world mark and sweep over
//! the heap's arenas. It runs only at safe points of the interpreter
//! (allocating instructions), never inside a native function, so values a
//! native holds in Rust locals stay valid. Weak tables follow Lua 5.1
//! (`__mode`; strings are never cleared), and userdata with `__gc` are
//! finalized once, after being kept alive for the cycle that found them.

use std::collections::HashSet;
use std::rc::Rc;

use crate::error::LuaError;
use crate::function::{Function, Upval};
use crate::heap::UdRef;
use crate::proto::Proto;
use crate::thread::ThreadState;
use crate::value::Value;
use crate::vm::Vm;

/// Work for the mark phase.
enum Gray {
    Value(Value),
    Upval(crate::heap::UpvalRef),
    Proto(Rc<Proto>),
}

struct Marker {
    gray: Vec<Gray>,
    /// Weak tables found: (table, weak keys, weak values).
    weak: Vec<(crate::heap::TableRef, bool, bool)>,
    protos: HashSet<*const Proto>,
}

impl Vm {
    /// A safe point after an allocating instruction.
    pub(crate) fn gc_step(&mut self) -> Result<(), LuaError> {
        // No steps while a finalizer runs, as in Lua.
        if !self.gc_stopped && !self.in_finalizer && self.heap.debt > self.gc_threshold {
            self.collect()?;
        }
        Ok(())
    }

    /// `collectgarbage("step", size)`, emulating Lua 5.1's incremental
    /// collector: about one basic step per KiB of `size`, plus one, each
    /// worth `10 * stepmul` bytes of work. A cycle costs the heap's size; it
    /// collects, and the step returns true, once that work is done.
    pub fn step_collect(&mut self, size_kb: i64) -> Result<bool, LuaError> {
        let left = self
            .gc_cycle_work
            .unwrap_or_else(|| self.heap.estimate_bytes());
        let steps = size_kb.max(0) as usize + 1;
        let work = steps.saturating_mul(10 * self.gc_stepmul);
        // A step multiplier of 0 means no limit, as in Lua.
        if self.gc_stepmul == 0 || work >= left {
            self.collect()?;
            Ok(true)
        } else {
            self.gc_cycle_work = Some(left - work);
            Ok(false)
        }
    }

    /// A full collection (`collectgarbage("collect")`). Finalizers run at
    /// its end, unprotected as in Lua: an error in one propagates to the code
    /// that caused the collection, and the rest wait for the next one.
    pub fn collect(&mut self) -> Result<(), LuaError> {
        self.gc_cycles += 1;
        self.gc_cycle_work = None;
        let mut m = Marker {
            gray: Vec::new(),
            weak: Vec::new(),
            protos: HashSet::new(),
        };
        self.mark_roots(&mut m);
        self.propagate(&mut m);
        // Userdata with `__gc` that became unreachable, newest first as in
        // Lua's list: kept (with what they reach) for this cycle, marked
        // finalized now, finalized after the sweep.
        let mut found: Vec<(u64, UdRef)> = Vec::new();
        let gc_name = self.names.gc;
        for (idx, generation, u) in self.heap.userdata.iter() {
            if self.heap.userdata.is_marked(idx, generation) {
                continue;
            }
            let r = UdRef { idx, generation };
            if self.finalized.contains(&r) {
                continue;
            }
            let has_gc = u
                .meta
                .is_some_and(|mt| !self.heap.table(mt).get_str(gc_name).is_nil());
            if has_gc {
                found.push((u.serial, r));
            }
        }
        found.sort_by_key(|a| std::cmp::Reverse(a.0));
        for &(_, u) in &found {
            self.finalized.insert(u);
            m.gray.push(Gray::Value(Value::Userdata(u)));
        }
        self.to_finalize.extend(found.into_iter().map(|(_, u)| u));
        self.propagate(&mut m);
        self.clear_weak(&m);
        self.sweep();
        self.heap.debt = 0;
        let live = self.heap.estimate_bytes();
        self.gc_threshold = (live * self.gc_pause / 100).max(1 << 20);
        self.run_finalizers()
    }

    fn run_finalizers(&mut self) -> Result<(), LuaError> {
        let gc_name = self.names.gc;
        while let Some(u) = self.to_finalize.pop_front() {
            let tm = self.get_tm(Value::Userdata(u), gc_name);
            if tm.is_nil() {
                continue;
            }
            // Taint does not leave a `__gc` metamethod (patch 10.1).
            let taint = self.taint().saved();
            let outer = std::mem::replace(&mut self.in_finalizer, true);
            let r = self.call(tm, &[Value::Userdata(u)]);
            self.in_finalizer = outer;
            r?;
            self.taint_mut().restore(taint);
        }
        Ok(())
    }

    fn mark_roots(&mut self, m: &mut Marker) {
        let roots = [
            Value::Table(self.globals),
            Value::Table(self.registry),
            Value::Thread(self.main),
            Value::Thread(self.current),
        ];
        m.gray.extend(roots.iter().map(|v| Gray::Value(*v)));
        if let Some(t) = self.string_meta {
            m.gray.push(Gray::Value(Value::Table(t)));
        }
        for s in self.names.all() {
            m.gray.push(Gray::Value(Value::Str(s)));
        }
        m.gray.extend(self.pinned.iter().map(|v| Gray::Value(*v)));
        if let Some(e) = self.errfunc {
            m.gray.push(Gray::Value(e));
        }
        m.gray.push(Gray::Value(self.error_handler));
        m.gray.push(Gray::Value(self.print_handler));
        m.gray.extend(
            self.to_finalize
                .iter()
                .map(|&u| Gray::Value(Value::Userdata(u))),
        );
        for c in &self.cstack {
            m.gray.push(Gray::Value(Value::Func(c.func)));
        }
        for t in &self.resume_chain {
            m.gray.push(Gray::Value(Value::Thread(*t)));
        }
        m.gray
            .extend(self.yield_values.iter().map(|v| Gray::Value(v.v)));
        // The running thread's state lives in the VM, not in its object.
        let top = self.live_top();
        for slot in &mut self.cur.stack[top..] {
            *slot = crate::taint::Slot::NIL;
        }
        mark_state(&self.cur, top, m);
    }

    /// The highest stack slot in use by the running thread.
    pub(crate) fn live_top(&self) -> usize {
        // Inside a native called by the innermost Lua frame, Lua's `L->top`
        // is the native's: the caller's registers above the call are dead.
        if self
            .cstack
            .last()
            .is_some_and(|c| c.at_frames == self.cur.frames.len())
        {
            return self.cur.top.min(self.cur.stack.len());
        }
        let frames_top = self
            .cur
            .frames
            .iter()
            .map(|f| f.base + f.proto.max_stack as usize)
            .max()
            .unwrap_or(0);
        self.cur.top.max(frames_top).min(self.cur.stack.len())
    }

    fn propagate(&mut self, m: &mut Marker) {
        while let Some(g) = m.gray.pop() {
            match g {
                Gray::Value(v) => self.mark_value(v, m),
                Gray::Upval(u) => {
                    if !self.heap.upvals.mark(u.idx, u.generation) {
                        continue;
                    }
                    match *self.heap.upval(u) {
                        Upval::Closed(v) => m.gray.push(Gray::Value(v.v)),
                        // An open upvalue keeps its thread's stack alive.
                        Upval::Open { thread, .. } => {
                            m.gray.push(Gray::Value(Value::Thread(thread)))
                        }
                    }
                }
                Gray::Proto(p) => {
                    if !m.protos.insert(Rc::as_ptr(&p)) {
                        continue;
                    }
                    m.gray.extend(p.constants.iter().map(|v| Gray::Value(*v)));
                    m.gray.extend(p.protos.iter().cloned().map(Gray::Proto));
                }
            }
        }
    }

    fn mark_value(&mut self, v: Value, m: &mut Marker) {
        match v {
            Value::Str(s) => {
                self.heap.strings.mark(s.idx, s.generation);
            }
            Value::Table(t) => {
                if !self.heap.tables.mark(t.idx, t.generation) {
                    return;
                }
                let table = self.heap.table(t);
                let (mut weak_k, mut weak_v) = (false, false);
                if let Some(mt) = table.meta {
                    m.gray.push(Gray::Value(Value::Table(mt)));
                    if let Value::Str(mode) = self.heap.table(mt).get_str(self.names.mode) {
                        let mode = self.heap.str_bytes(mode);
                        weak_k = mode.contains(&b'k');
                        weak_v = mode.contains(&b'v');
                    }
                }
                if weak_k || weak_v {
                    m.weak.push((t, weak_k, weak_v));
                }
                for (k, val) in self.heap.table(t).entries() {
                    // Strings are never weak (Lua 5.1).
                    if !weak_k || matches!(k, Value::Str(_)) {
                        m.gray.push(Gray::Value(k));
                    }
                    if !weak_v || matches!(val, Value::Str(_)) {
                        m.gray.push(Gray::Value(val));
                    }
                }
            }
            Value::Func(f) => {
                if !self.heap.funcs.mark(f.idx, f.generation) {
                    return;
                }
                match self.heap.func(f) {
                    Function::Lua(lf) => {
                        m.gray.push(Gray::Value(Value::Table(lf.env)));
                        m.gray.push(Gray::Proto(lf.proto.clone()));
                        m.gray.extend(lf.upvals.iter().map(|u| Gray::Upval(*u)));
                    }
                    Function::Native(nf) => {
                        m.gray.push(Gray::Value(Value::Table(nf.env)));
                        m.gray.extend(nf.upvals.iter().map(|v| Gray::Value(*v)));
                    }
                }
            }
            Value::Thread(t) => {
                if !self.heap.threads.mark(t.idx, t.generation) {
                    return;
                }
                if t != self.current {
                    let th = self.heap.thread(t);
                    m.gray.push(Gray::Value(Value::Table(th.globals)));
                    if let Some(f) = th.start {
                        m.gray.push(Gray::Value(Value::Func(f)));
                    }
                    let top = th.state.stack.len();
                    mark_state(&th.state, top, m);
                }
            }
            Value::Userdata(u) => {
                if !self.heap.userdata.mark(u.idx, u.generation) {
                    return;
                }
                let ud = self.heap.userdata(u);
                if let Some(mt) = ud.meta {
                    m.gray.push(Gray::Value(Value::Table(mt)));
                }
                if let Some(env) = ud.env {
                    m.gray.push(Gray::Value(Value::Table(env)));
                }
            }
            Value::Nil | Value::Bool(_) | Value::Num(_) => {}
        }
    }

    /// A userdata marked for finalization: Lua clears it from weak
    /// values, though not from weak keys.
    fn is_finalized(&self, v: Value) -> bool {
        matches!(v, Value::Userdata(u) if self.finalized.contains(&u))
    }

    fn is_dead(&self, v: Value) -> bool {
        match v {
            Value::Table(t) => !self.heap.tables.is_marked(t.idx, t.generation),
            Value::Func(f) => !self.heap.funcs.is_marked(f.idx, f.generation),
            Value::Thread(t) => !self.heap.threads.is_marked(t.idx, t.generation),
            Value::Userdata(u) => !self.heap.userdata.is_marked(u.idx, u.generation),
            _ => false,
        }
    }

    fn clear_weak(&mut self, m: &Marker) {
        for &(t, weak_k, weak_v) in &m.weak {
            let dead: Vec<(Value, Value)> = self
                .heap
                .table(t)
                .entries()
                .filter(|(k, v)| {
                    (weak_k && self.is_dead(*k))
                        || (weak_v && (self.is_dead(*v) || self.is_finalized(*v)))
                })
                .collect();
            if dead.is_empty() {
                continue;
            }
            let dead_keys: Vec<Value> = dead.into_iter().map(|(k, _)| k).collect();
            self.heap
                .table_mut(t)
                .retain(|k, _| !dead_keys.iter().any(|d| d.raw_eq(k)));
        }
    }

    fn sweep(&mut self) {
        self.heap.tables.sweep(|_| {});
        self.heap.funcs.sweep(|_| {});
        self.heap.upvals.sweep(|_| {});
        self.heap.threads.sweep(|_| {});
        let finalized = &mut self.finalized;
        let mut freed = Vec::new();
        self.heap
            .userdata
            .sweep_with(|idx, generation| freed.push(UdRef { idx, generation }));
        for u in freed {
            finalized.remove(&u);
        }
        self.heap.sweep_strings();
    }
}

fn mark_state(state: &ThreadState, top: usize, m: &mut Marker) {
    for v in &state.stack[..top.min(state.stack.len())] {
        m.gray.push(Gray::Value(v.v));
    }
    for f in &state.frames {
        m.gray.push(Gray::Value(Value::Func(f.closure)));
    }
    for &(_, u) in &state.open_upvals {
        m.gray.push(Gray::Upval(u));
    }
}
