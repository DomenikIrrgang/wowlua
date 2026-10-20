//! The client's frame end: some events a frame's server updates and
//! actions owe are not fired as they come but at the frame's end. First
//! the actions' usability is checked (`ACTION_USABLE_CHANGED`, once),
//! then each unit's auras' update is told (`UNIT_AURA`, one per unit
//! token), then the batched events, each once (the same event with the
//! same values), in the order of their names (case aside; one name's in
//! the order they came).
//!
//! Recorded: in every scenario recording (20 of them, 369 frames with two
//! or more of these events) the frame's `ACTIONBAR_UPDATE_COOLDOWN`,
//! `ACTIONBAR_UPDATE_STATE`, `ACTIONBAR_UPDATE_USABLE`,
//! `BAG_UPDATE_COOLDOWN`, `DAMAGE_METER_…`, `SPELL_UPDATE_COOLDOWN`,
//! `SPELL_UPDATE_USABLE`, `UI_ERROR_MESSAGE`, `UNIT_HEALTH`,
//! `UNIT_THREAT_LIST_UPDATE`, `UNIT_THREAT_SITUATION_UPDATE` and
//! `UPDATE_SHAPESHIFT_COOLDOWN` come together after its other events, in
//! that order, none twice (one exception: a loading screen's frame). A
//! frame's `UNIT_AURA`s come right before them (59 of 60 frames), never
//! two for a unit token (198 frames); its `ACTION_USABLE_CHANGED` once
//! (61 frames), after the server's updates and before the auras'. Other
//! events seem batched too (`SPELL_UPDATE_CHARGES`, `TOYS_UPDATED`:
//! unverified, too few samples) and are fired at once. `UI_INFO_MESSAGE`
//! is batched: in all 11 recorded (quest's nine, mail's, group's) it is
//! its frame's last but for batched events after it in their names' order
//! (quest 46.868: `SPELL_UPDATE_USABLE` before it, `UNIT_HEALTH` after;
//! 92.298: `SPELL_UPDATE_USABLE` before), never before a server update of
//! its frame (quest 59.292: after the next item's push).
//!
//! Off unless a test turns it on (`setFrameBatching`): a test of an
//! addon sees each event as the control that causes it fires it.

use wowlua_vm::{TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// The events batched to a frame's end.
const BATCHED: &[&str] = &[
    "ACTIONBAR_UPDATE_COOLDOWN",
    "ACTIONBAR_UPDATE_STATE",
    "ACTIONBAR_UPDATE_USABLE",
    "BAG_UPDATE_COOLDOWN",
    "DAMAGE_METER_COMBAT_SESSION_UPDATED",
    "DAMAGE_METER_CURRENT_SESSION_UPDATED",
    // Recorded once: a tracking spell's aura's, between the bags' and the
    // spells' cooldowns (scenario tracking).
    "MINIMAP_UPDATE_TRACKING",
    // Recorded once, `quest`'s level up (69.677): the player's
    // specialization's update between the bags' and the spells' cooldowns
    // (unverified).
    "PLAYER_SPECIALIZATION_CHANGED",
    "SPELL_UPDATE_COOLDOWN",
    "SPELL_UPDATE_USABLE",
    // Recorded twice, `quest`'s level up (69.677, 70.006): after the
    // spells' usability, before the shapeshift bar's cooldown and the
    // health, the group buffs' changes right before it (`spells_changed`).
    "SPELLS_CHANGED",
    // Recorded twice, `death`: the corpse's tracking at the spirit's
    // release and at the resurrection, between the spells' usability and
    // the health; the shapeshift bar's usability after the health (the
    // resurrection's).
    "SUPER_TRACKING_CHANGED",
    "UI_ERROR_MESSAGE",
    "UI_INFO_MESSAGE",
    "UNIT_HEALTH",
    "UNIT_THREAT_LIST_UPDATE",
    "UNIT_THREAT_SITUATION_UPDATE",
    "UPDATE_SHAPESHIFT_COOLDOWN",
    "UPDATE_SHAPESHIFT_USABLE",
];

#[derive(Default)]
pub struct Batch {
    /// Batching (`setFrameBatching`).
    pub on: bool,
    /// The events owed, each `{ name, count, values… }` (in a table the
    /// collector sees).
    pending: Option<TableRef>,
    count: usize,
    flushing: bool,
    /// The actions' usability to check at the frame's end: all of them
    /// (2), those bound to the target alone (1), none (0).
    usability: u8,
    checking: bool,
    /// Creatures that turned on the player in a fight, their update owed.
    turns: Vec<usize>,
    /// A test's functions to run before an event's next firing
    /// (`beforeEvent`), by the event's name, and how many.
    before: Option<TableRef>,
    befores: usize,
}

/// Whether a creature's turning on the player waits for the frame's end
/// (then owed).
///
/// Recorded: in a fight, the player's power in the packet before the
/// creature's target and flags (`combat`'s wolf, `death`'s kobolds,
/// `quest`'s wolves); before a fight the creature's first (`death`'s and
/// `auradeath`'s first blows: the caller's).
pub fn owe_turn(vm: &mut Vm, unit: usize) -> bool {
    let b = &mut engine_mut(vm).batch;
    if !b.on {
        return false;
    }
    if !b.turns.contains(&unit) {
        b.turns.push(unit);
    }
    true
}

/// Whether the actions' usability waits for the frame's end (`all`, or
/// only those bound to the target).
///
/// Recorded: a frame's `ACTION_USABLE_CHANGED` comes once, after its
/// server updates (the power, the flags, a creature's target), before its
/// auras' updates and the batched events (`quest`'s fights, `auradeath`'s
/// Consecration).
pub fn owe_usability(vm: &mut Vm, all: bool) -> bool {
    let b = &mut engine_mut(vm).batch;
    if !b.on || b.checking {
        return false;
    }
    b.usability = b.usability.max(if all { 2 } else { 1 });
    true
}

const PENDING: &str = "frameBatch";
const FIRING: &str = "frameBatchFiring";

fn pending(vm: &mut Vm) -> TableRef {
    if let Some(t) = engine(vm).batch.pending {
        return t;
    }
    let t = vm.new_table();
    widgets::keep_as(vm, PENDING, Value::Table(t));
    engine_mut(vm).batch.pending = Some(t);
    t
}

/// Whether `event` waits for the frame's end (then it is owed, once).
pub fn defer(vm: &mut Vm, event: &str, args: &[Value]) -> bool {
    let aura = event == "UNIT_AURA";
    if !engine(vm).batch.on || !(aura || BATCHED.contains(&event)) {
        return false;
    }
    let list = pending(vm);
    let name = vm.str(event);
    let n = engine(vm).batch.count;
    for i in 1..=n {
        let Value::Table(e) = vm.heap.table(list).get(Value::Num(i as f64)) else {
            continue;
        };
        let t = vm.heap.table(e);
        if !t.get(Value::Num(1.0)).raw_eq(name)
            || !t.get(Value::Num(2.0)).raw_eq(Value::Num(args.len() as f64))
        {
            continue;
        }
        // A unit's auras changed again: one update telling both.
        if aura && args.len() == 2 && t.get(Value::Num(3.0)).raw_eq(args[0]) {
            if let (Value::Table(old), Value::Table(new)) = (t.get(Value::Num(4.0)), args[1]) {
                merge_aura_update(vm, old, new);
            }
            return true;
        }
        if args
            .iter()
            .enumerate()
            .all(|(k, a)| t.get(Value::Num(k as f64 + 3.0)).raw_eq(*a))
        {
            return true;
        }
    }
    let e = vm.new_table();
    let _ = vm.heap.table_mut(e).set(Value::Num(1.0), name);
    let _ = vm
        .heap
        .table_mut(e)
        .set(Value::Num(2.0), Value::Num(args.len() as f64));
    for (k, a) in args.iter().enumerate() {
        let _ = vm.heap.table_mut(e).set(Value::Num(k as f64 + 3.0), *a);
    }
    let _ = vm
        .heap
        .table_mut(list)
        .set(Value::Num(n as f64 + 1.0), Value::Table(e));
    engine_mut(vm).batch.count = n + 1;
    true
}

/// One `UNIT_AURA` update telling another's changes too: its lists
/// lengthened, its other fields kept (a secret list as it is: unverified).
fn merge_aura_update(vm: &mut Vm, old: TableRef, new: TableRef) {
    let fields: Vec<(Value, Value)> = vm.heap.table(new).entries().collect();
    for (k, v) in fields {
        match (vm.heap.table(old).get(k), v) {
            (Value::Nil, _) => {
                let _ = vm.heap.table_mut(old).set(k, v);
            }
            (Value::Table(a), Value::Table(b)) => {
                let more: Vec<Value> = {
                    let b = vm.heap.table(b);
                    (1..=b.border())
                        .map(|i| b.get(Value::Num(i as f64)))
                        .collect()
                };
                for x in more {
                    let n = vm.heap.table(a).border();
                    let _ = vm.heap.table_mut(a).set(Value::Num(n as f64 + 1.0), x);
                }
            }
            _ => {}
        }
    }
}

/// An aura update's fields secret if auras are restricted at the frame's
/// end (recorded: Consecration's on the cat in the frame the fight began,
/// auradeath).
fn secret_if_restricted(vm: &mut Vm, info: TableRef) {
    use crate::units;
    let restricted = units::active(
        &engine(vm).units,
        &[
            units::COMBAT,
            units::ENCOUNTER,
            units::CHALLENGE_MODE,
            units::PVP_MATCH,
        ],
    );
    if !restricted {
        return;
    }
    let fields: Vec<(Value, Value)> = vm.heap.table(info).entries().collect();
    for (k, v) in fields {
        if !matches!(v, Value::Secret(_)) {
            let w = vm.secret_wrap(v);
            let _ = vm.heap.table_mut(info).set(k, w);
        }
    }
}

/// The frame's end: the events owed, in their names' order (those its
/// handlers cause after them).
pub fn flush(vm: &mut Vm) {
    if engine(vm).batch.flushing {
        return;
    }
    engine_mut(vm).batch.flushing = true;
    loop {
        // The creatures' turns first (their update after the player's).
        for unit in std::mem::take(&mut engine_mut(vm).batch.turns) {
            crate::units::told_turn(vm, crate::units::UnitId(unit));
        }
        // The target's aura states with its update (aurastates.rs).
        crate::aurastates::tell_owed(vm);
        // The actions' usability first (its `ACTIONBAR_UPDATE_USABLE`
        // batched with the rest).
        match std::mem::take(&mut engine_mut(vm).batch.usability) {
            2 => {
                engine_mut(vm).batch.checking = true;
                let changed = crate::actions::usability_changed(vm);
                engine_mut(vm).batch.checking = false;
                if changed {
                    defer(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
                }
            }
            1 => {
                engine_mut(vm).batch.checking = true;
                crate::actions::target_usability_changed(vm);
                engine_mut(vm).batch.checking = false;
            }
            _ => {}
        }
        if engine(vm).batch.count == 0 {
            break;
        }
        // The owed events move to a table of their own, kept while they
        // fire; new ones are owed anew.
        let firing = pending(vm);
        let n = engine(vm).batch.count;
        widgets::keep_as(vm, FIRING, Value::Table(firing));
        let fresh = vm.new_table();
        widgets::keep_as(vm, PENDING, Value::Table(fresh));
        engine_mut(vm).batch.pending = Some(fresh);
        engine_mut(vm).batch.count = 0;
        let mut owed: Vec<(String, Vec<Value>)> = Vec::with_capacity(n);
        for i in 1..=n {
            let Value::Table(e) = vm.heap.table(firing).get(Value::Num(i as f64)) else {
                continue;
            };
            let t = vm.heap.table(e);
            let Value::Str(s) = t.get(Value::Num(1.0)) else {
                continue;
            };
            let count = match t.get(Value::Num(2.0)) {
                Value::Num(c) => c as usize,
                _ => 0,
            };
            let args = (0..count)
                .map(|k| t.get(Value::Num(k as f64 + 3.0)))
                .collect();
            owed.push((String::from_utf8_lossy(vm.str_bytes(s)).into_owned(), args));
        }
        // The auras' updates first, in the order they came (recorded:
        // right before the batched events in 59 of 60 frames).
        owed.sort_by_key(|(name, _)| (name != "UNIT_AURA", name.to_ascii_lowercase()));
        for (name, args) in owed {
            if name == "SPELLS_CHANGED" {
                crate::character::group_buffs_changed(vm);
            }
            if name == "UNIT_AURA"
                && let Some(Value::Table(info)) = args.get(1)
            {
                secret_if_restricted(vm, *info);
            }
            widgets::fire_now(vm, &name, &args);
        }
        widgets::keep_as(vm, FIRING, Value::Nil);
    }
    engine_mut(vm).batch.flushing = false;
}

/// The player's spells changed: the group buffs' changes, then
/// `SPELLS_CHANGED`; batched, both at the frame's end (see `BATCHED`).
pub fn spells_changed(vm: &mut Vm) {
    if !engine(vm).batch.on {
        crate::character::group_buffs_changed(vm);
    }
    widgets::fire_event(vm, "SPELLS_CHANGED", &[]);
}

/// `setFrameBatching(on)`: the client's batching of a frame's events on
/// or off (off: those owed fired now).
pub fn control_set(vm: &mut Vm, c: wowlua_vm::NativeCall) -> wowlua_vm::stdlib::Ret {
    let on = c.arg(1).truthy();
    if !on {
        flush(vm);
    }
    engine_mut(vm).batch.on = on;
    Ok(vec![])
}

/// `tellTurns()`: the creatures' turns owed told now, their packet before
/// the moment's later ones (per instance, recorded: `quest` 48.121 and
/// 86.599, a wolf's turn between the swing's power and its own blow's;
/// `death` 12.318, after the player's rage told twice).
pub fn control_tell_turns(vm: &mut Vm, _c: wowlua_vm::NativeCall) -> wowlua_vm::stdlib::Ret {
    for unit in std::mem::take(&mut engine_mut(vm).batch.turns) {
        crate::units::told_turn(vm, crate::units::UnitId(unit));
    }
    Ok(vec![])
}

/// `beforeEvent(name, fn)`: `fn` runs once just before the engine next
/// fires event `name`: a world event inside a server packet's (recorded:
/// `mail2` 16.286, another player's death between a taken item's bag
/// updates and their `BAG_UPDATE_DELAYED`).
pub fn control_before_event(vm: &mut Vm, c: wowlua_vm::NativeCall) -> wowlua_vm::stdlib::Ret {
    let name = vm.check_str(&c, 1)?;
    let f = c.arg(2);
    if !matches!(f, Value::Func(_)) {
        return Err(vm.error("beforeEvent: a function expected"));
    }
    let t = match engine(vm).batch.before {
        Some(t) => t,
        None => {
            let t = vm.new_table();
            widgets::keep_as(vm, "beforeEvent", Value::Table(t));
            engine_mut(vm).batch.before = Some(t);
            t
        }
    };
    if vm.heap.table(t).get(Value::Str(name)).is_nil() {
        engine_mut(vm).batch.befores += 1;
    }
    let _ = vm.heap.table_mut(t).set(Value::Str(name), f);
    Ok(vec![])
}

/// Before `event` fires: a test's function waiting for it, run once.
pub fn run_before(vm: &mut Vm, event: &str) {
    if engine(vm).batch.befores == 0 {
        return;
    }
    let Some(t) = engine(vm).batch.before else {
        return;
    };
    let k = vm.str(event);
    let f = vm.heap.table(t).get(k);
    if f.is_nil() {
        return;
    }
    let _ = vm.heap.table_mut(t).set(k, Value::Nil);
    engine_mut(vm).batch.befores -= 1;
    vm.call_from_engine(f, &[]);
}
