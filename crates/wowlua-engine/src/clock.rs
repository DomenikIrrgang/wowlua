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
    /// The timers still to fire, by index (finished ones leave it: the
    /// list keeps every timer for its handle).
    pub active: Vec<usize>,
    handle_meta: Option<TableRef>,
    /// The last frame's seconds (`GetTickTime`), none before the first.
    pub tick_time: f64,
}

/// The server's round trip: a request's answer comes this much later (the
/// player's ping; 0.4 s unless a test sets it, near the recordings' ~0.3
/// s on WoW Forever's US servers).
pub fn server_delay(vm: &Vm) -> f64 {
    engine(vm).world.latency.unwrap_or(0.4)
}

/// Seconds since the session began (`GetTime`), in whole milliseconds as
/// the client's (recorded: `GetTime()`'s three decimals).
pub fn now(vm: &Vm) -> f64 {
    vm.clock.elapsed_ms.round() / 1000.0
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
    // Handles are userdata whose metatable is hidden (`/stdtest engine`:
    // no methods table to read; that `__metatable` is false is unverified).
    let meta = vm.new_table();
    let index = vm.heap.intern(b"__index");
    vm.heap
        .table_mut(meta)
        .set_str(index, Value::Table(methods));
    let hidden = vm.heap.intern(b"__metatable");
    vm.heap.table_mut(meta).set_str(hidden, Value::Bool(false));
    crate::widgets::keep(vm, Value::Table(meta));
    engine_mut(vm).timers.handle_meta = Some(meta);
    meta
}

fn timer_of(vm: &Vm, v: Value) -> Option<TimerId> {
    let Value::Userdata(u) = v else { return None };
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
        let ud = vm.heap.alloc_userdata(Userdata {
            serial: 0,
            meta: Some(meta),
            env: None,
            data: Box::new(id),
        });
        let _ = vm
            .heap
            .table_mut(slot)
            .set(Value::Num(2.0), Value::Userdata(ud));
        out = Value::Userdata(ud);
    }
    let due = now(vm) + seconds.max(0.0);
    let index = engine(vm).timers.list.len();
    engine_mut(vm).timers.active.push(index);
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

/// Moves the clock by `seconds` without running anything (the client while
/// it loads: `/stdtest engine startup`, no timer or OnUpdate before the
/// loading screen goes).
pub fn jump(vm: &mut Vm, seconds: f64) {
    tick(vm, seconds.max(0.0) * 1000.0);
}

/// Timers overdue after loading come due now, once (a ticker does not
/// catch up: recorded one tick of a 0.5 s ticker after a 7 s load).
pub fn catch_up(vm: &mut Vm) {
    let now = now(vm);
    for t in engine_mut(vm).timers.list.iter_mut() {
        if t.due < now {
            t.due = now;
        }
    }
}

/// Advances the clock by `seconds`, running due timers in the order they
/// come due (each at its time).
pub fn advance(vm: &mut Vm, seconds: f64) {
    // The clock's own milliseconds (`now` rounds them): timers due by
    // the time read at the end.
    let end_ms = vm.clock.elapsed_ms + seconds.max(0.0) * 1000.0;
    let target = end_ms.round() / 1000.0;
    loop {
        let timers = &mut engine_mut(vm).timers;
        let list = &timers.list;
        timers
            .active
            .retain(|&i| !list[i].cancelled && list[i].left > 0.0);
        let next = timers
            .active
            .iter()
            .map(|&i| (i, &timers.list[i]))
            .filter(|(_, t)| t.due <= target)
            .min_by(|a, b| a.1.due.total_cmp(&b.1.due).then(a.0.cmp(&b.0)))
            .map(|(i, t)| (TimerId(i), t.due));
        let Some((id, due)) = next else { break };
        if due * 1000.0 > vm.clock.elapsed_ms {
            tick(vm, due * 1000.0 - vm.clock.elapsed_ms);
        }
        {
            // A ticker without an interval fires once a frame (once this
            // advance), not again and again at the same moment.
            let t = &mut engine_mut(vm).timers.list[id.0];
            t.left -= 1.0;
            t.due = if t.every > 0.0 {
                t.due + t.every
            } else {
                target + 1e-9
            };
        }
        fire(vm, id);
    }
    if end_ms > vm.clock.elapsed_ms {
        tick(vm, end_ms - vm.clock.elapsed_ms);
    }
}

/// `n` frames of `ms` milliseconds: each moves the clock, runs the
/// `OnSizeChanged` the last frame's resizing owes (before `OnUpdate`,
/// `/stdtest engine`), every visible widget's
/// `OnUpdate` (with the frame's seconds), the `OnTextChanged` the frame's
/// `SetText` owes (those before it and in `OnUpdate`), then the due
/// timers, those an `OnUpdate` just set included (`/stdtest engine`:
/// "update1 after0 after0 (set in update1) update2"; "update1 … changed
/// after0").
pub fn advance_frames(vm: &mut Vm, n: usize, ms: f64) {
    for _ in 0..n {
        tick(vm, ms);
        engine_mut(vm).timers.tick_time = ms / 1000.0;
        widgets::run_size_changes(vm);
        let count = engine(vm).world.widgets.len();
        let elapsed = Value::Num(ms / 1000.0);
        // The widgets given an `OnUpdate`, in creation order (one given it
        // during the frame too), not those made in it. The name interned as
        // when each widget was looked at: before the first, and after each
        // `OnUpdate` (a collection in it may have freed the string, and
        // interning it again counts towards the next).
        if count > 0 {
            vm.heap.intern(b"OnUpdate");
        }
        let mut next = 0;
        while let Some(&i) = engine(vm).world.updating.range(next..count).next() {
            next = i + 1;
            let id = WidgetId(i);
            if widgets::has_handler(vm, id, "OnUpdate") && widgets::visible(engine(vm), id) {
                widgets::run_script(vm, id, "OnUpdate", &[elapsed]);
                if next < count {
                    vm.heap.intern(b"OnUpdate");
                }
            }
        }
        widgets::run_text_changes(vm);
        crate::animations::tick(vm, ms / 1000.0);
        advance(vm, 0.0);
    }
}

/// `GetTickTime()`: the frame's seconds, as its `OnUpdate` got them
/// (unverified: before the first frame, 0).
fn get_tick_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).timers.tick_time)])
}

/// The documented functions this module gives behaviour.
pub fn behaviours() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        ("GetTime", get_time),
        ("GetTickTime", get_tick_time),
        ("GetTimePreciseSec", get_time),
        ("C_Timer.After", after),
        ("C_Timer.NewTimer", new_timer),
        ("C_Timer.NewTicker", new_ticker),
    ]
}
