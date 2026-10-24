//! Debug hooks (spec debugger §18.2, decisions T302 and T303): the VM tells
//! a [`Debugger`] the lines, calls, errors, loads and the engine's
//! dispatches it runs, and keeps the profiler's counters
//! ([`crate::profile`]).
//!
//! Cost: the interpreter tests one byte, [`Vm::hook_mask`], per
//! instruction; it is 0 unless something listens. Its bits are worked out
//! when a frame starts or resumes ([`Vm::frame_hooks`]): line events only
//! in functions with a breakpoint, or while a step runs.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustc_hash::FxHashSet;

use crate::error::LuaError;
use crate::function::Function;
use crate::heap::{FuncRef, ThreadRef};
use crate::proto::Proto;
use crate::value::Value;
use crate::vm::Vm;

/// Line events in the running frame.
pub(crate) const HOOK_LINE: u8 = 1;
/// Coverage counts the running frame.
pub(crate) const HOOK_COV: u8 = 1 << 1;
/// A debugger is attached: look at the interrupt flag now and then.
pub(crate) const HOOK_POLL: u8 = 1 << 2;
/// Work the bits out again before the next instruction.
pub(crate) const HOOK_RECALC: u8 = 1 << 3;
/// The session is being torn down: every instruction raises.
pub(crate) const HOOK_ABORT: u8 = 1 << 4;
/// The profiler counts instructions.
pub(crate) const HOOK_COUNT: u8 = 1 << 5;

/// Instructions between two looks at the interrupt flag.
const POLL_EVERY: u32 = 1024;

/// What the engine is running Lua for (a frame of its own in stacks and
/// profiles).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DispatchKind {
    /// An event's handlers (`PLAYER_LOGIN`).
    Event,
    /// A widget's script (`MyFrame:OnClick`).
    Script,
    /// A frame's `OnUpdate` (also a script, told apart for profiles).
    Update,
    /// A timer's callback.
    Timer,
    /// A game frame: every `OnUpdate`, timer and batched event of it.
    Frame,
    /// Anything else the host names (loading an addon, a test).
    Other,
}

impl DispatchKind {
    pub fn label(self) -> &'static str {
        match self {
            DispatchKind::Event => "event",
            DispatchKind::Script => "script",
            DispatchKind::Update => "update",
            DispatchKind::Timer => "timer",
            DispatchKind::Frame => "frame",
            DispatchKind::Other => "other",
        }
    }
}

/// A dispatch running: what and where it started.
#[derive(Clone, Debug)]
pub struct Dispatch {
    pub kind: DispatchKind,
    /// `PLAYER_LOGIN`, `MyFrame:OnEvent`, `frame 12`.
    pub name: Rc<str>,
    /// Its thread and the call depth there when it started: the Lua
    /// frames and native calls below it.
    pub thread: ThreadRef,
    pub frames: usize,
    pub natives: usize,
}

/// The token [`Vm::dispatch_enter`] gives, for [`Vm::dispatch_exit`].
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct DispatchToken(usize);

/// Why the VM tells the debugger about a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCause {
    /// The function has a breakpoint, or a step runs: the debugger decides.
    Line,
    /// A function breakpoint's function starts here.
    FunctionEntry,
}

/// The listener of the hooks. Each method may pause: it gets the VM to
/// inspect, evaluate in and change, and returns when execution goes on.
/// While one runs, no hook fires (an evaluation's code runs without them).
pub trait Debugger {
    /// Before the instruction at `pc` of the running frame, which starts
    /// line `line`.
    fn line(&mut self, vm: &mut Vm, line: u32, cause: LineCause);
    /// Lua calls a native function that has a breakpoint (an API call),
    /// before it runs.
    fn api_call(&mut self, _vm: &mut Vm, _f: FuncRef) {}
    /// An error was raised; the stack is still as it was. `caught`: a
    /// `pcall`, `xpcall` or `coroutine.resume` catches it.
    fn error(&mut self, _vm: &mut Vm, _error: &LuaError, _caught: bool) {}
    /// A chunk was compiled.
    fn load(&mut self, _vm: &mut Vm, _proto: &Rc<Proto>) {}
    /// The engine starts (`enter`) or ends a dispatch.
    fn dispatch(&mut self, _vm: &mut Vm, _dispatch: &Dispatch, _enter: bool) {}
    /// The interrupt flag was set ([`Vm::interrupt_handle`]).
    fn interrupted(&mut self, _vm: &mut Vm) {}
    /// Execution became tainted (`taint` breakpoints): `by` is the addon.
    fn tainted(&mut self, _vm: &mut Vm, _by: &str) {}
    /// The engine refused a protected action (`blocked` breakpoints).
    fn blocked(&mut self, _vm: &mut Vm, _action: &str, _addon: &str, _report: &str) {}
    /// The host is between game frames, no Lua running: requests can be
    /// served (and the session can pause here).
    fn idle(&mut self, _vm: &mut Vm) {}
}

/// A step in progress: line events in frames up to this depth of this
/// thread (step into: any).
#[derive(Clone, Copy, Debug)]
pub struct Step {
    pub thread: Option<ThreadRef>,
    pub max_depth: usize,
}

/// The VM's side of the debugger: what makes it stop.
pub struct DebugState {
    /// Functions (by prototype) that have a line breakpoint.
    pub(crate) line_protos: FxHashSet<usize>,
    /// Functions whose calls stop (functions, API calls).
    pub(crate) func_breaks: FxHashSet<FuncRef>,
    pub(crate) step: Option<Step>,
    /// A function breakpoint was entered: its first line stops.
    pub(crate) entry_break: bool,
    pub(crate) interrupt: Arc<AtomicBool>,
    pub(crate) poll_left: u32,
    /// The dispatches running, outermost first.
    pub(crate) dispatches: Vec<Dispatch>,
    /// Tell the debugger when execution becomes tainted.
    pub(crate) watch_taint: bool,
}

impl Vm {
    /// Attaches a debugger: from now on its hooks run.
    pub fn attach_debugger(&mut self, d: Box<dyn Debugger>) {
        self.debugger = Some(d);
        if self.dbg.is_none() {
            self.dbg = Some(Box::new(DebugState {
                line_protos: FxHashSet::default(),
                func_breaks: FxHashSet::default(),
                step: None,
                entry_break: false,
                interrupt: Arc::new(AtomicBool::new(false)),
                poll_left: POLL_EVERY,
                dispatches: Vec::new(),
                watch_taint: false,
            }));
        }
        self.hook_base |= HOOK_POLL;
        self.hooks_changed();
    }

    /// Detaches the debugger (and drops what makes it stop).
    pub fn detach_debugger(&mut self) -> Option<Box<dyn Debugger>> {
        self.dbg = None;
        self.hook_base &= !HOOK_POLL;
        self.hooks_changed();
        self.debugger.take()
    }

    pub fn has_debugger(&self) -> bool {
        self.dbg.is_some()
    }

    /// Uses `flag` as the interrupt flag (one the host shares between the
    /// VMs of a session).
    pub fn set_interrupt_handle(&mut self, flag: Arc<AtomicBool>) {
        if let Some(d) = self.dbg.as_mut() {
            d.interrupt = flag;
        }
    }

    /// The engine refused a protected action: the debugger hears of it.
    pub fn notify_blocked(&mut self, action: &str, addon: &str, report: &str) {
        if self.debugger.is_some() {
            let (a, b, c) = (action.to_string(), addon.to_string(), report.to_string());
            self.with_debugger(|d, vm| d.blocked(vm, &a, &b, &c));
        }
    }

    /// How the caller named the function of Lua frame `index` (`global`,
    /// `local`, `method`, `field`, `upvalue`), when a Lua frame called it.
    pub fn frame_name(&self, index: usize) -> Option<(&'static str, String)> {
        let f = self.cur.frames.get(index)?;
        if f.tail || f.entry || index == 0 {
            return None;
        }
        let caller = &self.cur.frames[index - 1];
        let pc = caller.pc.saturating_sub(1);
        match caller.proto.code.get(pc) {
            Some(
                crate::proto::Op::Call { a, .. }
                | crate::proto::Op::TailCall { a, .. }
                | crate::proto::Op::TForLoop { a, .. },
            ) => crate::vm::obj_name(&self.heap, &caller.proto, pc, *a),
            _ => None,
        }
    }

    /// A function object's taint.
    pub fn func_taint(&self, f: FuncRef) -> crate::taint::Taint {
        self.heap.func(f).taint()
    }

    /// A flag another thread (or the browser's event loop) sets to pause
    /// the running code: the debugger's `interrupted` hook runs soon after.
    pub fn interrupt_handle(&self) -> Option<Arc<AtomicBool>> {
        self.dbg.as_ref().map(|d| d.interrupt.clone())
    }

    /// Marks the functions of `proto` (and nested ones) that `has_line`
    /// says have a line breakpoint; the others lose their mark.
    pub fn mark_breakpoints(
        &mut self,
        proto: &Rc<Proto>,
        has_line: &mut dyn FnMut(&Proto) -> bool,
    ) {
        fn walk(set: &mut FxHashSet<usize>, p: &Rc<Proto>, f: &mut dyn FnMut(&Proto) -> bool) {
            let key = Rc::as_ptr(p) as usize;
            if f(p) {
                set.insert(key);
            } else {
                set.remove(&key);
            }
            for c in &p.protos {
                walk(set, c, f);
            }
        }
        if let Some(d) = self.dbg.as_mut() {
            walk(&mut d.line_protos, proto, has_line);
        }
        self.hooks_changed();
    }

    /// The functions whose calls stop.
    pub fn set_function_breakpoints(&mut self, funcs: impl IntoIterator<Item = FuncRef>) {
        if let Some(d) = self.dbg.as_mut() {
            d.func_breaks = funcs.into_iter().collect();
        }
        self.update_call_hooks();
    }

    /// Starts a step (`None` ends it).
    pub fn set_step(&mut self, step: Option<Step>) {
        if let Some(d) = self.dbg.as_mut() {
            d.step = step;
        }
        self.hooks_changed();
    }

    /// Tell the debugger when execution becomes tainted.
    pub fn watch_taint(&mut self, on: bool) {
        if let Some(d) = self.dbg.as_mut() {
            d.watch_taint = on;
        }
    }

    pub(crate) fn taint_watched(&self) -> bool {
        self.dbg.as_ref().is_some_and(|d| d.watch_taint)
    }

    /// The running thread and its call depth (Lua frames and native calls):
    /// what a step compares.
    pub fn depth(&self) -> (ThreadRef, usize) {
        (self.current, self.cur.frames.len())
    }

    pub fn current_thread(&self) -> ThreadRef {
        self.current
    }

    pub fn main_thread(&self) -> ThreadRef {
        self.main
    }

    /// The dispatches running, outermost first.
    pub fn dispatches(&self) -> &[Dispatch] {
        match &self.dbg {
            Some(d) => &d.dispatches,
            None => &[],
        }
    }

    /// Tears the session down: from now on every Lua instruction raises
    /// "session ended", so whatever runs unwinds quickly.
    pub fn abort(&mut self) {
        self.aborted = true;
        self.hook_base |= HOOK_ABORT;
        self.hook_mask |= HOOK_ABORT;
    }

    pub fn aborted(&self) -> bool {
        self.aborted
    }

    /// Something that decides the hook bits changed: they are worked out
    /// again before the next instruction.
    pub(crate) fn hooks_changed(&mut self) {
        self.hooks_on = self.hook_base != 0 || self.coverage.is_some() || self.dbg.is_some();
        if self.hooks_on {
            self.hook_mask |= HOOK_RECALC;
        } else {
            self.hook_mask = 0;
        }
        self.update_call_hooks();
    }

    pub(crate) fn update_call_hooks(&mut self) {
        self.call_hooks =
            self.profiler.is_some() || self.dbg.as_ref().is_some_and(|d| !d.func_breaks.is_empty());
    }

    /// The hook bits for a frame of `proto` starting or resuming (and the
    /// coverage index of it).
    #[inline]
    pub(crate) fn frame_hooks(&mut self, proto: &Rc<Proto>) {
        if !self.hooks_on {
            return;
        }
        self.frame_hooks_slow(proto);
    }

    #[inline(never)]
    pub(crate) fn frame_hooks_slow(&mut self, proto: &Rc<Proto>) {
        let mut m = self.hook_base;
        self.cov_frame = match &self.coverage {
            Some(c) => c.index_of(proto),
            None => None,
        };
        if self.cov_frame.is_some() {
            m |= HOOK_COV;
        }
        if let Some(d) = &self.dbg {
            let stepping = d.step.is_some_and(|s| {
                s.thread.is_none_or(|t| t == self.current) && self.cur.frames.len() <= s.max_depth
            });
            if stepping || d.entry_break || d.line_protos.contains(&(Rc::as_ptr(proto) as usize)) {
                m |= HOOK_LINE;
            }
        }
        self.hook_mask = m;
    }

    /// The slow part of an instruction with hooks on.
    #[inline(never)]
    pub(crate) fn instr_hook(
        &mut self,
        fi: usize,
        pc: usize,
        proto: &Rc<Proto>,
    ) -> Result<(), LuaError> {
        if self.hook_mask & HOOK_RECALC != 0 {
            self.hook_mask &= !HOOK_RECALC;
            self.frame_hooks_slow(proto);
        }
        let m = self.hook_mask;
        if m & HOOK_ABORT != 0 {
            return Err(self.abort_error());
        }
        if m & HOOK_COV != 0
            && let Some(ci) = self.cov_frame
            && let Some(c) = self.coverage.as_mut()
        {
            c.hit(ci, pc);
        }
        if m & HOOK_POLL != 0
            && let Some(d) = self.dbg.as_mut()
        {
            d.poll_left -= 1;
            if d.poll_left == 0 {
                d.poll_left = POLL_EVERY;
                if d.interrupt.swap(false, Ordering::Relaxed) {
                    self.with_debugger(|d, vm| d.interrupted(vm));
                    if self.aborted {
                        return Err(self.abort_error());
                    }
                }
            }
        }
        if m & HOOK_LINE != 0 {
            let f = &mut self.cur.frames[fi];
            let old = if f.hook_pc == usize::MAX {
                pc.wrapping_sub(1)
            } else {
                f.hook_pc
            };
            f.hook_pc = pc;
            let line = proto.lines.get(pc).copied().unwrap_or(0);
            let new_line = pc == 0
                || old == usize::MAX
                || pc <= old
                || proto.lines.get(old).copied() != Some(line);
            if new_line {
                let cause = match self.dbg.as_mut() {
                    Some(d) if d.entry_break => {
                        d.entry_break = false;
                        LineCause::FunctionEntry
                    }
                    _ => LineCause::Line,
                };
                self.with_debugger(|d, vm| d.line(vm, line, cause));
                if self.aborted {
                    return Err(self.abort_error());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn abort_error(&mut self) -> LuaError {
        let v = self.str("session ended");
        LuaError::new(v)
    }

    /// Runs `f` with the debugger taken out of the VM (no hooks fire
    /// meanwhile); the hook bits are worked out again afterwards.
    pub fn with_debugger(&mut self, f: impl FnOnce(&mut dyn Debugger, &mut Vm)) {
        if let Some(mut d) = self.debugger.take() {
            let mask = self.hook_mask;
            f(&mut *d, self);
            if self.debugger.is_none() && self.dbg.is_some() {
                self.debugger = Some(d);
            }
            self.hook_mask = mask | HOOK_RECALC;
            self.hooks_changed();
        }
    }

    /// A call entered (the callee's frame or native entry is pushed).
    #[inline(never)]
    pub(crate) fn call_hook(&mut self, f: FuncRef, native: bool) {
        let depth = self.cur.frames.len() + self.cstack.len();
        if let Some(p) = self.profiler.as_mut() {
            let key = match self.heap.func(f) {
                Function::Lua(l) => crate::profile::RawKey::Lua(l.proto.clone()),
                Function::Native(n) => crate::profile::RawKey::Native(n.name),
            };
            p.enter(
                key,
                depth,
                self.current,
                &self.resume_chain,
                self.instructions,
                self.heap.allocated(),
            );
        }
        if let Some(d) = self.dbg.as_mut()
            && d.func_breaks.contains(&f)
        {
            if native {
                self.with_debugger(|d, vm| d.api_call(vm, f));
            } else {
                d.entry_break = true;
                self.hooks_changed();
            }
        }
    }

    /// Calls at depth `depth` and above returned (or unwound).
    #[inline(never)]
    pub(crate) fn return_hook(&mut self, depth: usize) {
        if let Some(p) = self.profiler.as_mut() {
            p.exit_to(
                depth,
                self.current,
                self.instructions,
                self.heap.allocated(),
            );
        }
    }

    /// Before an error unwinds: the debugger sees the stack where it was
    /// raised (once per error).
    pub(crate) fn error_hook(&mut self, e: &mut LuaError) {
        if e.seen || e.yielded || self.aborted || self.debugger.is_none() {
            return;
        }
        e.seen = true;
        // Where the error goes: the nearest handler. The client's error
        // handler (a handler or script the engine ran: errors reach it) is
        // not catching; an `xpcall`'s handler is; without a handler, a
        // `pcall` running is (decision T358).
        let caught = match (self.errfunc, self.errfunc_proxy) {
            (Some(Value::Func(f)), Some(Value::Func(p))) if f == p => false,
            (Some(_), _) => true,
            (None, _) => self.protected > 0,
        };
        let err = e.clone();
        self.with_debugger(|d, vm| d.error(vm, &err, caught));
    }

    pub(crate) fn load_hook(&mut self, proto: &Rc<Proto>) {
        if self.debugger.is_some() {
            let p = proto.clone();
            self.with_debugger(|d, vm| d.load(vm, &p));
        }
    }

    pub(crate) fn taint_hook(&mut self, by: &str) {
        let by = by.to_string();
        self.with_debugger(|d, vm| d.tainted(vm, &by));
    }

    /// Whether anything listens to dispatches (a debugger, the profiler):
    /// work out their names only then.
    #[inline]
    pub fn dispatch_wanted(&self) -> bool {
        self.dbg.is_some() || self.profiler.is_some()
    }

    /// The engine starts running Lua for `kind` (`name` made only when
    /// something listens). `None` when nothing does.
    #[inline]
    pub fn dispatch_enter(
        &mut self,
        kind: DispatchKind,
        name: impl FnOnce() -> String,
    ) -> Option<DispatchToken> {
        if self.dbg.is_none() && self.profiler.is_none() {
            return None;
        }
        Some(self.dispatch_enter_slow(kind, name().into()))
    }

    #[inline(never)]
    fn dispatch_enter_slow(&mut self, kind: DispatchKind, name: Rc<str>) -> DispatchToken {
        let d = Dispatch {
            kind,
            name,
            thread: self.current,
            frames: self.cur.frames.len(),
            natives: self.cstack.len(),
        };
        let depth = d.frames + d.natives;
        if let Some(p) = self.profiler.as_mut() {
            p.enter_dispatch(
                kind,
                d.name.clone(),
                depth,
                self.current,
                &self.resume_chain,
                self.instructions,
                self.heap.allocated(),
            );
        }
        let token = match self.dbg.as_mut() {
            Some(s) => {
                s.dispatches.push(d.clone());
                s.dispatches.len() - 1
            }
            None => 0,
        };
        self.update_call_hooks();
        if self.debugger.is_some() {
            self.with_debugger(|dbg, vm| dbg.dispatch(vm, &d, true));
        }
        DispatchToken(token)
    }

    /// The dispatch [`dispatch_enter`](Self::dispatch_enter) started ended.
    #[inline]
    pub fn dispatch_exit(&mut self, token: Option<DispatchToken>) {
        if let Some(t) = token {
            self.dispatch_exit_slow(t);
        }
    }

    #[inline(never)]
    fn dispatch_exit_slow(&mut self, token: DispatchToken) {
        let depth = self.cur.frames.len() + self.cstack.len();
        if let Some(p) = self.profiler.as_mut() {
            p.exit_dispatch(
                depth,
                self.current,
                self.instructions,
                self.heap.allocated(),
            );
        }
        let ended = match self.dbg.as_mut() {
            Some(s) if token.0 < s.dispatches.len() => {
                let d = s.dispatches[token.0].clone();
                s.dispatches.truncate(token.0);
                Some(d)
            }
            _ => None,
        };
        self.update_call_hooks();
        if let Some(d) = ended
            && self.debugger.is_some()
        {
            self.with_debugger(|dbg, vm| dbg.dispatch(vm, &d, false));
        }
    }

    // ------------------------------------------------- inspecting frames

    /// The Lua frames of the running thread, innermost first: their
    /// function, prototype, the pc of the instruction running (or about
    /// to), and the frame index for [`frame_slots`](Self::frame_slots).
    pub fn lua_frames(&self) -> Vec<FrameInfo> {
        self.cur
            .frames
            .iter()
            .enumerate()
            .rev()
            .map(|(i, f)| FrameInfo {
                index: i,
                func: f.closure,
                proto: f.proto.clone(),
                pc: f.pc.saturating_sub(1),
                tail: f.tail,
                natives_above: self
                    .cstack
                    .iter()
                    .filter(|c| c.at_frames == i + 1)
                    .map(|c| self.native_name(c.func))
                    .collect(),
            })
            .collect()
    }

    /// The name a native function was made with.
    pub fn native_name(&self, f: FuncRef) -> &'static str {
        match self.heap.func(f) {
            Function::Native(n) => n.name,
            Function::Lua(_) => "?",
        }
    }

    /// A frame's active locals (name, register) at its pc, parameters
    /// first, and its varargs' count.
    pub fn frame_locals(&self, index: usize) -> Vec<(Rc<str>, usize)> {
        let f = &self.cur.frames[index];
        let pc = f.pc.saturating_sub(1);
        let mut out = Vec::new();
        let mut reg = 0;
        for v in &f.proto.locvars {
            if v.start_pc > pc {
                break;
            }
            if pc < v.end_pc {
                out.push((v.name.clone(), f.base + reg));
                reg += 1;
            }
        }
        out
    }

    /// A frame's varargs, as slots.
    pub fn frame_varargs(&self, index: usize) -> Vec<crate::taint::Slot> {
        let f = &self.cur.frames[index];
        let n = f.nvarargs;
        self.cur.stack[f.base - n..f.base].to_vec()
    }

    /// The stack slot at `i` of the running thread.
    pub fn stack_slot(&self, i: usize) -> crate::taint::Slot {
        self.cur.stack[i]
    }

    pub fn set_stack_slot(&mut self, i: usize, s: crate::taint::Slot) {
        self.cur.stack[i] = s;
    }

    /// A Lua function's upvalues: name and slot.
    pub fn upvalues(&self, f: FuncRef) -> Vec<(Rc<str>, crate::taint::Slot)> {
        match self.heap.func(f) {
            Function::Lua(l) => l
                .upvals
                .iter()
                .enumerate()
                .map(|(i, &u)| {
                    let name = l
                        .proto
                        .upval_names
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| "?".into());
                    (name, self.upval_slot(u))
                })
                .collect(),
            Function::Native(_) => Vec::new(),
        }
    }

    /// Sets a Lua function's `i`-th upvalue.
    pub fn set_upvalue(&mut self, f: FuncRef, i: usize, s: crate::taint::Slot) {
        let u = match self.heap.func(f) {
            Function::Lua(l) => l.upvals.get(i).copied(),
            Function::Native(_) => None,
        };
        if let Some(u) = u {
            self.upval_set(u, s);
        }
    }

    /// A function's prototype, if it is a Lua function.
    pub fn func_proto(&self, f: FuncRef) -> Option<Rc<Proto>> {
        match self.heap.func(f) {
            Function::Lua(l) => Some(l.proto.clone()),
            Function::Native(_) => None,
        }
    }

    /// The running execution's taint.
    pub fn execution_taint(&self) -> crate::taint::Taint {
        self.cur.taint.write()
    }

    /// Runs `f` with the execution taint `t` (an evaluation as the
    /// stopped frame's code), restoring the state after.
    pub fn with_execution_taint<R>(
        &mut self,
        t: crate::taint::Taint,
        f: impl FnOnce(&mut Vm) -> R,
    ) -> R {
        let saved = self.cur.taint.saved();
        self.cur.taint.stack = t;
        let r = f(self);
        self.cur.taint.restore(saved);
        r
    }
}

/// One Lua frame, for stacks.
#[derive(Clone, Debug)]
pub struct FrameInfo {
    pub index: usize,
    pub func: FuncRef,
    pub proto: Rc<Proto>,
    pub pc: usize,
    pub tail: bool,
    /// The native calls this frame made that are still running (their
    /// names), innermost last.
    pub natives_above: Vec<&'static str>,
}
