//! A log of the events the client tells, kept while a host asks for it
//! (decision T483, Studio's game page): each event's name, its arguments
//! as text, the frames it reached and the time their handlers took.

use std::collections::VecDeque;
use std::rc::Rc;

use wowlua_vm::{Value, Vm};

use crate::widgets::{self, WidgetId};
use crate::{engine, engine_mut};

/// The most events kept between two reads: the oldest go first.
const KEPT: usize = 2000;

/// The log: what was told since the host last read it.
pub struct EventLog {
    entries: VecDeque<Logged>,
    /// The host's clock in nanoseconds, to time the handlers (none: not
    /// timed, as in the browser without one).
    clock: Option<Rc<dyn Fn() -> u64>>,
    next: u64,
}

/// One event told.
#[derive(Clone, Debug, PartialEq)]
pub struct Logged {
    /// Its place among the events told since the log began.
    pub seq: u64,
    pub name: String,
    /// The arguments as Lua shows them (`"player"`, `12`, `true`, `nil`).
    pub args: Vec<String>,
    /// The frames it reached, in the order their handlers ran.
    pub frames: Vec<String>,
    /// The time the handlers took, with a clock.
    pub nanos: Option<u64>,
}

/// Starts keeping the log (again from empty), timed by `clock`.
pub fn start(vm: &mut Vm, clock: Option<Rc<dyn Fn() -> u64>>) {
    engine_mut(vm).world.event_log = Some(EventLog {
        entries: VecDeque::new(),
        clock,
        next: 0,
    });
}

/// Stops keeping it.
pub fn stop(vm: &mut Vm) {
    engine_mut(vm).world.event_log = None;
}

/// The events told since the last read, oldest first; the log goes on.
pub fn take(vm: &mut Vm) -> Vec<Logged> {
    match &mut engine_mut(vm).world.event_log {
        Some(log) => log.entries.drain(..).collect(),
        None => Vec::new(),
    }
}

/// An event about to be told: its entry, and when it began (`fire_now`).
pub(crate) fn begin(
    vm: &mut Vm,
    event: &str,
    args: &[Value],
    targets: &[WidgetId],
) -> Option<(u64, Option<u64>)> {
    engine(vm).world.event_log.as_ref()?;
    let args: Vec<String> = args.iter().map(|&v| shown(vm, v)).collect();
    let frames: Vec<String> = targets
        .iter()
        .map(|&id| widgets::widget_label(engine(vm), id))
        .collect();
    let log = engine_mut(vm).world.event_log.as_mut()?;
    let seq = log.next;
    log.next += 1;
    if log.entries.len() == KEPT {
        log.entries.pop_front();
    }
    log.entries.push_back(Logged {
        seq,
        name: event.to_string(),
        args,
        frames,
        nanos: None,
    });
    let started = log.clock.as_ref().map(|c| c());
    Some((seq, started))
}

/// The event's handlers ran: their time.
pub(crate) fn end(vm: &mut Vm, begun: Option<(u64, Option<u64>)>) {
    let Some((seq, Some(started))) = begun else {
        return;
    };
    let Some(log) = engine_mut(vm).world.event_log.as_mut() else {
        return;
    };
    let now = log.clock.as_ref().map_or(started, |c| c());
    if let Some(e) = log.entries.iter_mut().rev().find(|e| e.seq == seq) {
        e.nanos = Some(now.saturating_sub(started));
    }
}

/// A value as Lua shows it in a list of arguments.
fn shown(vm: &Vm, v: Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", n as i64),
        Value::Num(n) => n.to_string(),
        Value::Str(s) => format!("{:?}", String::from_utf8_lossy(vm.str_bytes(s))),
        Value::Table(_) => "table".into(),
        Value::Func(_) => "function".into(),
        _ => "?".into(),
    }
}
