//! The server's aura states (`AuraStateType`): flags on a unit that some
//! spells need (Execute a wounded target, Revenge after a dodge). The
//! client tells a change of the player's or its target's with
//! `PET_BAR_UPDATE_USABLE()`, then `SPELL_UPDATE_USABLE()`.
//!
//! Recorded (every `PET_BAR_UPDATE_USABLE` of the fight scenarios but the
//! death's own): the health states, each a threshold of the unit's health
//! (the modern enum's WOUNDED_20, _25, _35 and _50_PERCENT below,
//! HEALTHY_75_PERCENT above, WOUND_HEALTH_20_80 and _35_80 above 80).
//! `death`'s player, 79 health, hit from full: at 63 (below 80), 58 (75),
//! 39 (50), 26 (35), 18 (25) and 15 (20), each once, and not at the hits
//! between; `auradeath`'s cat (300 health) at 91 damage (80 and 75), 165
//! (50) and 267 (35, 25 and 20 at once: one); `combat`'s wolf the same
//! with its health 43 (the last blow's overkill unrecorded). And the
//! warrior's Defensive State (5302, `procOnAvoid`): on with a dodge, parry
//! or block, off about 5 s after the last one (`quest`, `death`: 4.8 to
//! 5.4 s after the proc; another proc while on tells nothing).
//!
//! When: with the blow's packet (after the attacker's power, before the
//! victim's: `combat` 6.283, `death` 12.969), or the server's next update
//! a moment later (0.03 to 0.43 s; per instance: `statesAfter`).
//! Unverified: other units' (a nameplate's, the focus'); healing's
//! besides regeneration; other classes' defensive states.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::units::{PLAYER, UnitId};
use crate::{engine, engine_mut, widgets};

/// The player's defensive state (on, and its procs counted: an expiry
/// owed for an older proc is no longer the state's).
#[derive(Default)]
pub struct AuraStates {
    defensive: bool,
    procs: u32,
    /// A change's telling owed (the server's next update): another change
    /// before it goes with it.
    owed: bool,
    /// The target's change owed to the frame's end (its update after the
    /// moment's other packets).
    target_owed: bool,
}

/// The defensive state's length when a test gives none (recorded 4.8 to
/// 5.4 s from the proc).
const DEFENSIVE_FOR: f64 = 5.0;

/// The unit's health states: one bit per threshold (none dead).
pub fn health_states(vm: &Vm, id: UnitId) -> u8 {
    let u = &engine(vm).units.list[id.0];
    if u.dead || u.health_max <= 0.0 {
        return 0;
    }
    let pct = u.health * 100.0 / u.health_max;
    let mut bits = 0;
    for (i, t) in [20.0, 25.0, 35.0, 50.0].into_iter().enumerate() {
        if pct < t {
            bits |= 1 << i;
        }
    }
    if pct > 75.0 {
        bits |= 1 << 4;
    }
    if pct > 80.0 {
        bits |= 1 << 5;
    }
    bits
}

/// The unit's health changed from states `before`: a change of the
/// player's or the target's told, at once or `after` seconds later.
pub fn health_changed(vm: &mut Vm, id: UnitId, before: u8, after: Option<f64>) {
    if engine(vm).units.list[id.0].dead || health_states(vm, id) == before {
        return;
    }
    if id != PLAYER && engine(vm).units.target != Some(id) {
        return;
    }
    // The target's with its update, after the moment's other packets (its
    // turn on the player too: `quest` 24.671 and 86.599, `combat` 6.283);
    // the player's at once, before its power (`death` 12.969, 20.037).
    if id != PLAYER && after.is_none() && engine(vm).batch.on {
        engine_mut(vm).units.aura_states.target_owed = true;
        return;
    }
    changed(vm, after);
}

/// The frame's end: the target's change owed told (after the creatures'
/// turns, before the actions' usability).
pub fn tell_owed(vm: &mut Vm) {
    if std::mem::take(&mut engine_mut(vm).units.aura_states.target_owed) {
        told(vm);
    }
}

/// The player dodged, parried or blocked with a defensive state's spell
/// known: the state on (told if it was off, at once or `after` seconds
/// later), off `length` seconds on unless another comes.
pub fn defended(vm: &mut Vm, after: Option<f64>, length: Option<f64>) {
    let (was, proc) = {
        let s = &mut engine_mut(vm).units.aura_states;
        let was = s.defensive;
        s.defensive = true;
        s.procs += 1;
        (was, s.procs)
    };
    if !was {
        changed(vm, after);
    }
    let off = vm.native_with("defensiveOff", defensive_off, vec![Value::Num(proc.into())]);
    crate::clock::after_engine(vm, length.unwrap_or(DEFENSIVE_FOR), off);
}

fn defensive_off(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(proc) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let s = &mut engine_mut(vm).units.aura_states;
    if !s.defensive || f64::from(s.procs) != proc {
        return Ok(vec![]);
    }
    s.defensive = false;
    told(vm);
    Ok(vec![])
}

/// A test's `statesAfter`: seconds, or `true` (owed until
/// `tellAuraStates`).
pub fn after_option(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        Value::Bool(true) => Some(f64::INFINITY),
        _ => None,
    }
}

/// An aura state changed: told now, or `after` seconds on (infinite: when
/// the test tells it).
fn changed(vm: &mut Vm, after: Option<f64>) {
    match after {
        Some(s) if s.is_infinite() => {
            engine_mut(vm).units.aura_states.owed = true;
        }
        Some(s) if s > 0.0 => {
            // Recorded: the Hammer of Justice's blows past three
            // thresholds of the cat's health, told once (auradeath
            // 15.722).
            if std::mem::replace(&mut engine_mut(vm).units.aura_states.owed, true) {
                return;
            }
            let later = vm.native("auraStatesTold", told_later);
            crate::clock::after_engine(vm, s, later);
        }
        _ => told(vm),
    }
}

fn told_later(vm: &mut Vm, _c: NativeCall) -> Ret {
    if std::mem::take(&mut engine_mut(vm).units.aura_states.owed) {
        told(vm);
    }
    Ok(vec![])
}

/// `tellAuraStates()`: the server's update owing an aura state change
/// (`statesAfter`) comes now, the moment's other packets before it
/// (recorded: `quest` 52.115, after a wolf's blow's power; `auradeath`
/// 15.722, after a heal's and a proc's).
pub fn control_tell(vm: &mut Vm, c: NativeCall) -> Ret {
    told_later(vm, c)
}

fn told(vm: &mut Vm) {
    widgets::fire_event(vm, "PET_BAR_UPDATE_USABLE", &[]);
    widgets::fire_event(vm, "SPELL_UPDATE_USABLE", &[]);
}
