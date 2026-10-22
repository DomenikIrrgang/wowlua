//! The encounter timeline's script events (`C_EncounterTimeline`): the
//! timers addons put on it (decision T255). On WoW Forever the timeline
//! itself is off (`IsFeatureAvailable()` false, recorded), but an addon's
//! `AddScriptEvent` still puts a timer on it (found in game: SuuUI's pull
//! countdown shows on its timeline). The engine keeps them: added, listed,
//! read (`GetEventInfo`, `GetEventState`, `GetEventTimeRemaining`,
//! `GetEventTimeElapsed`), paused and resumed, finished, cancelled, and
//! finished by themselves when their time runs out.
//!
//! Unverified, from the API's documentation: the IDs (here counted from
//! after `ENCOUNTER_TIMELINE_RESERVED_EVENT_COUNT`, 40), the info's fields
//! for a script event (its name the `overrideName`, else the spell's), the
//! events (`ENCOUNTER_TIMELINE_EVENT_ADDED(eventInfo)`, `…_STATE_CHANGED(
//! eventID)` then `…_REMOVED(eventID)` as one ends), that an ended timer
//! leaves the list at once, the sorted list's order (by time left).

use std::collections::BTreeMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// `Enum.EncounterTimelineEventState`.
const ACTIVE: f64 = 0.0;
const PAUSED: f64 = 1.0;
const FINISHED: f64 = 2.0;
/// `Enum.EncounterTimelineEventSource.Script`.
const SCRIPT: f64 = 1.0;
/// `ENCOUNTER_TIMELINE_RESERVED_EVENT_COUNT`.
const RESERVED: u32 = 40;

/// A script event on the timeline.
#[derive(Clone, Debug)]
pub struct Event {
    spell_id: f64,
    icon: f64,
    duration: f64,
    max_queue: f64,
    name: Option<String>,
    icons: f64,
    severity: f64,
    /// When it began (the clock), moved on by a pause.
    start: f64,
    /// Time left while paused.
    paused: Option<f64>,
}

/// The timeline's script events, by ID.
#[derive(Default)]
pub struct Timeline {
    next: u32,
    events: BTreeMap<u32, Event>,
}

fn timeline(vm: &Vm) -> &Timeline {
    &engine(vm).world.timeline
}

fn timeline_mut(vm: &mut Vm) -> &mut Timeline {
    &mut engine_mut(vm).world.timeline
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// Whether a call's first argument is an event on the timeline (the
/// functions that require one answer nothing otherwise).
pub fn valid(vm: &Vm, c: &NativeCall) -> bool {
    id_arg(vm, c).is_some_and(|id| timeline(vm).events.contains_key(&id))
}

fn id_arg(vm: &Vm, c: &NativeCall) -> Option<u32> {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) if n >= 0.0 => Some(n as u32),
        _ => None,
    }
}

fn left(e: &Event, now: f64) -> f64 {
    match e.paused {
        Some(l) => l,
        None => (e.duration - (now - e.start)).max(0.0),
    }
}

fn state(e: &Event) -> f64 {
    if e.paused.is_some() { PAUSED } else { ACTIVE }
}

/// `C_EncounterTimeline.AddScriptEvent(eventInfo)`: a timer on the
/// timeline; its ID. `ENCOUNTER_TIMELINE_EVENT_ADDED(eventInfo)`.
fn add_script_event(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("AddScriptEvent: an event's info expected"));
    };
    let spell_id = num(field(vm, t, "spellID")).unwrap_or(0.0);
    let icon = num(field(vm, t, "iconFileID")).unwrap_or(0.0);
    let duration = num(field(vm, t, "duration")).unwrap_or(0.0).max(0.0);
    let max_queue = num(field(vm, t, "maxQueueDuration")).unwrap_or(0.0);
    let name = match field(vm, t, "overrideName") {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    };
    let icons = num(field(vm, t, "icons")).unwrap_or(0.0);
    let severity = num(field(vm, t, "severity")).unwrap_or(0.0);
    let paused = field(vm, t, "paused").truthy();
    let now = crate::clock::now(vm);
    let tl = timeline_mut(vm);
    tl.next = tl.next.max(RESERVED) + 1;
    let id = tl.next;
    tl.events.insert(
        id,
        Event {
            spell_id,
            icon,
            duration,
            max_queue,
            name,
            icons,
            severity,
            start: now,
            paused: paused.then_some(duration),
        },
    );
    if !paused {
        schedule_end(vm, id, duration);
    }
    let info = info_table(vm, id);
    widgets::fire_event(vm, "ENCOUNTER_TIMELINE_EVENT_ADDED", &[info]);
    Ok(vec![Value::Num(f64::from(id))])
}

/// Its end when its time runs out (checked then: a pause moves it).
fn schedule_end(vm: &mut Vm, id: u32, after: f64) {
    let native = vm.native_with("timelineEnd", ended, vec![Value::Num(f64::from(id))]);
    crate::clock::after_engine(vm, after, native);
}

fn ended(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(n)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let id = n as u32;
    let now = crate::clock::now(vm);
    let Some(e) = timeline(vm).events.get(&id).cloned() else {
        return Ok(vec![]);
    };
    if e.paused.is_some() {
        return Ok(vec![]);
    }
    let l = left(&e, now);
    if l > 0.0005 {
        schedule_end(vm, id, l);
        return Ok(vec![]);
    }
    end(vm, id);
    Ok(vec![])
}

/// An event leaving the timeline (finished or cancelled):
/// `ENCOUNTER_TIMELINE_EVENT_STATE_CHANGED(id)`, then `…_REMOVED(id)`.
fn end(vm: &mut Vm, id: u32) {
    if timeline_mut(vm).events.remove(&id).is_none() {
        return;
    }
    let v = Value::Num(f64::from(id));
    widgets::fire_event(vm, "ENCOUNTER_TIMELINE_EVENT_STATE_CHANGED", &[v]);
    widgets::fire_event(vm, "ENCOUNTER_TIMELINE_EVENT_REMOVED", &[v]);
}

fn cancel_script_event(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(id) = id_arg(vm, &c) {
        end(vm, id);
    }
    Ok(vec![])
}

fn finish_script_event(vm: &mut Vm, c: NativeCall) -> Ret {
    cancel_script_event(vm, c)
}

fn cancel_all_script_events(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ids: Vec<u32> = timeline(vm).events.keys().copied().collect();
    for id in ids {
        end(vm, id);
    }
    Ok(vec![])
}

fn pause_script_event(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = id_arg(vm, &c) else {
        return Ok(vec![]);
    };
    let now = crate::clock::now(vm);
    let changed = match timeline_mut(vm).events.get_mut(&id) {
        Some(e) if e.paused.is_none() => {
            e.paused = Some(left(e, now));
            true
        }
        _ => false,
    };
    if changed {
        let v = Value::Num(f64::from(id));
        widgets::fire_event(vm, "ENCOUNTER_TIMELINE_EVENT_STATE_CHANGED", &[v]);
    }
    Ok(vec![])
}

fn resume_script_event(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = id_arg(vm, &c) else {
        return Ok(vec![]);
    };
    let now = crate::clock::now(vm);
    let resumed = match timeline_mut(vm).events.get_mut(&id) {
        Some(e) => match e.paused.take() {
            Some(l) => {
                e.start = now - (e.duration - l);
                Some(l)
            }
            None => None,
        },
        None => None,
    };
    if let Some(l) = resumed {
        schedule_end(vm, id, l);
        let v = Value::Num(f64::from(id));
        widgets::fire_event(vm, "ENCOUNTER_TIMELINE_EVENT_STATE_CHANGED", &[v]);
    }
    Ok(vec![])
}

/// An event's info (`EncounterTimelineEventInfo`), or nil.
fn info_table(vm: &mut Vm, id: u32) -> Value {
    let Some(e) = timeline(vm).events.get(&id).cloned() else {
        return Value::Nil;
    };
    let name = match &e.name {
        Some(n) => vm.str(n),
        None => match crate::gamedata::spell_name(vm, e.spell_id as i64) {
            Value::Nil => vm.str(""),
            v => v,
        },
    };
    let t = vm.new_table();
    set(vm, t, "id", Value::Num(f64::from(id)));
    set(vm, t, "source", Value::Num(SCRIPT));
    set(vm, t, "spellName", name);
    set(vm, t, "spellID", Value::Num(e.spell_id));
    set(vm, t, "iconFileID", Value::Num(e.icon));
    set(vm, t, "duration", Value::Num(e.duration));
    set(vm, t, "maxQueueDuration", Value::Num(e.max_queue));
    set(vm, t, "icons", Value::Num(e.icons));
    set(vm, t, "severity", Value::Num(e.severity));
    set(vm, t, "isApproximate", Value::Bool(false));
    Value::Table(t)
}

fn get_event_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = id_arg(vm, &c) else {
        return Ok(vec![Value::Nil]);
    };
    Ok(vec![info_table(vm, id)])
}

/// `GetEventState(id)`: active or paused; an event no longer there,
/// finished (unverified: what the client answers for an unknown ID).
fn get_event_state(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = id_arg(vm, &c)
        .and_then(|id| timeline(vm).events.get(&id).map(state))
        .unwrap_or(FINISHED);
    Ok(vec![Value::Num(s)])
}

fn get_event_time_remaining(vm: &mut Vm, c: NativeCall) -> Ret {
    let now = crate::clock::now(vm);
    let l = id_arg(vm, &c)
        .and_then(|id| timeline(vm).events.get(&id).map(|e| left(e, now)))
        .unwrap_or(0.0);
    Ok(vec![Value::Num(l)])
}

fn get_event_time_elapsed(vm: &mut Vm, c: NativeCall) -> Ret {
    let now = crate::clock::now(vm);
    let el = id_arg(vm, &c)
        .and_then(|id| {
            timeline(vm)
                .events
                .get(&id)
                .map(|e| e.duration - left(e, now))
        })
        .unwrap_or(0.0);
    Ok(vec![Value::Num(el)])
}

fn id_list(vm: &mut Vm, ids: Vec<u32>) -> Ret {
    let t = vm.new_table();
    for (i, id) in ids.into_iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(i as f64 + 1.0), Value::Num(f64::from(id)));
    }
    Ok(vec![Value::Table(t)])
}

fn get_event_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ids: Vec<u32> = timeline(vm).events.keys().copied().collect();
    id_list(vm, ids)
}

/// `GetSortedEventList(maxCount, maxDuration, …)`: by time left, the
/// first `maxCount` of those within `maxDuration`.
fn get_sorted_event_list(vm: &mut Vm, c: NativeCall) -> Ret {
    let now = crate::clock::now(vm);
    let max_count = num(vm.hidden(c.arg(1)));
    let max_duration = num(vm.hidden(c.arg(2)));
    let mut list: Vec<(f64, u32)> = timeline(vm)
        .events
        .iter()
        .map(|(id, e)| (left(e, now), *id))
        .filter(|(l, _)| max_duration.is_none_or(|m| *l <= m))
        .collect();
    list.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut ids: Vec<u32> = list.into_iter().map(|(_, id)| id).collect();
    if let Some(n) = max_count {
        ids.truncate(n.max(0.0) as usize);
    }
    id_list(vm, ids)
}

fn has(vm: &Vm, paused: Option<bool>) -> bool {
    timeline(vm)
        .events
        .values()
        .any(|e| paused.is_none_or(|p| e.paused.is_some() == p))
}

fn has_any_events(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(has(vm, None))])
}

fn has_active_events(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(has(vm, Some(false)))])
}

fn has_paused_events(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(has(vm, Some(true)))])
}

/// `GetEventCountBySource(source)`: the script events for `Script`; none
/// for the encounter's and edit mode's (the recorded 0s).
fn get_event_count_by_source(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = match num(vm.hidden(c.arg(1))) {
        Some(s) if s == SCRIPT => timeline(vm).events.len() as f64,
        _ => 0.0,
    };
    Ok(vec![Value::Num(n)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_EncounterTimeline.AddScriptEvent", add_script_event),
        ("C_EncounterTimeline.CancelScriptEvent", cancel_script_event),
        ("C_EncounterTimeline.FinishScriptEvent", finish_script_event),
        (
            "C_EncounterTimeline.CancelAllScriptEvents",
            cancel_all_script_events,
        ),
        ("C_EncounterTimeline.PauseScriptEvent", pause_script_event),
        ("C_EncounterTimeline.ResumeScriptEvent", resume_script_event),
        ("C_EncounterTimeline.GetEventInfo", get_event_info),
        ("C_EncounterTimeline.GetEventState", get_event_state),
        (
            "C_EncounterTimeline.GetEventTimeRemaining",
            get_event_time_remaining,
        ),
        (
            "C_EncounterTimeline.GetEventTimeElapsed",
            get_event_time_elapsed,
        ),
        ("C_EncounterTimeline.GetEventList", get_event_list),
        (
            "C_EncounterTimeline.GetSortedEventList",
            get_sorted_event_list,
        ),
        ("C_EncounterTimeline.HasAnyEvents", has_any_events),
        ("C_EncounterTimeline.HasVisibleEvents", has_any_events),
        ("C_EncounterTimeline.HasActiveEvents", has_active_events),
        ("C_EncounterTimeline.HasPausedEvents", has_paused_events),
        (
            "C_EncounterTimeline.GetEventCountBySource",
            get_event_count_by_source,
        ),
    ]
}
