//! The clock and timers (spec emulator §17.6): `GetTime` and `C_Timer` on
//! the VM's virtual clock, which tests advance (`mock.advanceTime`), and
//! `OnUpdate` per simulated frame (`mock.nextFrame`).
//!
//! Unverified until probed: `GetTime()` starts at 0; a timer handle is a
//! table with `Cancel`, `IsCancelled` and `Invoke`; within a frame, due
//! timers run before the `OnUpdate` scripts.

use wowlua_vm::heap::Userdata;
use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, TableRef, Value, Vm};

use crate::widgets::{self, WidgetId};
use crate::{engine, engine_mut};

/// A timer's payload in its handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerId(pub usize);

pub struct Timer {
    pub due: f64,
    pub every: f64,
    /// Runs left (`f64::INFINITY` for a ticker without a count).
    pub left: f64,
    pub cancelled: bool,
    /// The callback and the handle it gets (none for `C_Timer.After`), kept
    /// in a registry table.
    pub slot: TableRef,
}

#[derive(Default)]
pub struct Timers {
    pub list: Vec<Timer>,
    handle_meta: Option<TableRef>,
}

/// Seconds since the session began (`GetTime`).
pub fn now(vm: &Vm) -> f64 {
    vm.clock.elapsed_ms / 1000.0
}

fn get_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(now(vm))])
}

/// The handles' metatable: `Cancel`, `IsCancelled`, `Invoke`.
fn handle_meta(vm: &mut Vm) -> TableRef {
    if let Some(t) = engine(vm).timers.handle_meta {
        return t;
    }
    let methods = vm.new_table();
    for (name, f) in [
        ("Cancel", cancel as wowlua_vm::NativeFn),
        ("IsCancelled", is_cancelled),
        ("Invoke", invoke),
    ] {
        let native = vm.native(name, f);
        let key = vm.heap.intern(name.as_bytes());
        vm.heap.table_mut(methods).set_str(key, native);
    }
    let meta = vm.new_table();
    let index = vm.heap.intern(b"__index");
    vm.heap
        .table_mut(meta)
        .set_str(index, Value::Table(methods));
    crate::widgets::keep(vm, Value::Table(meta));
    engine_mut(vm).timers.handle_meta = Some(meta);
    meta
}

fn timer_of(vm: &Vm, v: Value) -> Option<TimerId> {
    let Value::Table(t) = v else { return None };
    let Value::Userdata(u) = vm.heap.table(t).get(Value::Num(0.0)) else {
        return None;
    };
    vm.heap.userdata(u).data.downcast_ref::<TimerId>().copied()
}

fn cancel(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(id) = timer_of(vm, c.arg(1)) {
        engine_mut(vm).timers.list[id.0].cancelled = true;
    }
    Ok(vec![])
}

fn is_cancelled(vm: &mut Vm, c: NativeCall) -> Ret {
    let cancelled = timer_of(vm, c.arg(1)).is_some_and(|id| engine(vm).timers.list[id.0].cancelled);
    Ok(vec![Value::Bool(cancelled)])
}

fn invoke(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(id) = timer_of(vm, c.arg(1)) {
        fire(vm, id);
    }
    Ok(vec![])
}

/// Runs `callback` once after `seconds` (the engine's own timers: data
/// arriving from the server).
pub fn after_engine(vm: &mut Vm, seconds: f64, callback: Value) {
    schedule(vm, seconds, callback, 1.0, false);
}

/// Schedules `callback` after `seconds`, `runs` times; returns its handle
/// when `handle`.
fn schedule(vm: &mut Vm, seconds: f64, callback: Value, runs: f64, handle: bool) -> Value {
    let id = TimerId(engine(vm).timers.list.len());
    let slot = vm.new_table();
    crate::widgets::keep(vm, Value::Table(slot));
    let _ = vm.heap.table_mut(slot).set(Value::Num(1.0), callback);
    let mut out = Value::Nil;
    if handle {
        let meta = handle_meta(vm);
        let t = vm.new_table();
        let ud = vm.heap.alloc_userdata(Userdata {
            serial: 0,
            meta: None,
            env: None,
            data: Box::new(id),
        });
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(0.0), Value::Userdata(ud));
        vm.heap.table_mut(t).meta = Some(meta);
        let _ = vm
            .heap
            .table_mut(slot)
            .set(Value::Num(2.0), Value::Table(t));
        out = Value::Table(t);
    }
    let due = now(vm) + seconds.max(0.0);
    engine_mut(vm).timers.list.push(Timer {
        due,
        every: seconds.max(0.0),
        left: runs,
        cancelled: false,
        slot,
    });
    out
}

fn after(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_number(&c, 1)?;
    schedule(vm, s, c.arg(2), 1.0, false);
    Ok(vec![])
}

fn new_timer(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_number(&c, 1)?;
    Ok(vec![schedule(vm, s, c.arg(2), 1.0, true)])
}

fn new_ticker(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_number(&c, 1)?;
    let runs = match c.arg(3) {
        Value::Nil => f64::INFINITY,
        _ => vm.check_number(&c, 3)?,
    };
    Ok(vec![schedule(vm, s, c.arg(2), runs, true)])
}

/// Runs a timer's callback once (as the client's C code calls it).
fn fire(vm: &mut Vm, id: TimerId) {
    let slot = engine(vm).timers.list[id.0].slot;
    let callback = vm.heap.table(slot).get(Value::Num(1.0));
    let handle = vm.heap.table(slot).get(Value::Num(2.0));
    let args: Vec<Value> = if handle.is_nil() {
        vec![]
    } else {
        vec![handle]
    };
    vm.call_from_engine(callback, &args);
}

/// Moves the clock by `ms` milliseconds (and the wall clock with it).
fn tick(vm: &mut Vm, ms: f64) {
    vm.clock.elapsed_ms += ms;
    vm.clock.now += ms / 1000.0;
}

/// Advances the clock by `seconds`, running due timers in the order they
/// come due (each at its time).
pub fn advance(vm: &mut Vm, seconds: f64) {
    let target = now(vm) + seconds.max(0.0);
    loop {
        let next = engine(vm)
            .timers
            .list
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.cancelled && t.left > 0.0 && t.due <= target)
            .min_by(|a, b| a.1.due.total_cmp(&b.1.due))
            .map(|(i, t)| (TimerId(i), t.due));
        let Some((id, due)) = next else { break };
        if due > now(vm) {
            tick(vm, (due - now(vm)) * 1000.0);
        }
        {
            let t = &mut engine_mut(vm).timers.list[id.0];
            t.left -= 1.0;
            t.due += t.every.max(1e-9);
        }
        fire(vm, id);
    }
    if target > now(vm) {
        tick(vm, (target - now(vm)) * 1000.0);
    }
}

/// `n` frames of `ms` milliseconds: each runs the due timers, then every
/// visible widget's `OnUpdate` (with the frame's seconds).
pub fn advance_frames(vm: &mut Vm, n: usize, ms: f64) {
    for _ in 0..n {
        advance(vm, ms / 1000.0);
        let count = engine(vm).world.widgets.len();
        let elapsed = Value::Num(ms / 1000.0);
        for i in 0..count {
            let id = WidgetId(i);
            if widgets::visible(engine(vm), id) {
                widgets::run_script(vm, id, "OnUpdate", &[elapsed]);
            }
        }
    }
}

/// The documented functions this module gives behaviour.
pub fn behaviours() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        ("GetTime", get_time),
        ("GetTimePreciseSec", get_time),
        ("C_Timer.After", after),
        ("C_Timer.NewTimer", new_timer),
        ("C_Timer.NewTicker", new_ticker),
    ]
}
