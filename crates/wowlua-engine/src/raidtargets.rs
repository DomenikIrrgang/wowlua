//! Raid target markers (decision T631): the eight icons the player puts
//! on units (`SetRaidTarget`), read back with `GetRaidTargetIndex`.
//!
//! - `SetRaidTarget(unit, index)`: index 1 to 8 puts that marker on the
//!   unit, taking it off the unit that had it and the unit's old marker
//!   off; 0 takes the unit's marker off. The server answers a round trip
//!   later: the marker changes then and `RAID_TARGET_UPDATE` fires (no
//!   payload). A unit that does not exist, another index or no change:
//!   nothing.
//! - `GetRaidTargetIndex(unit)`: the unit's marker, else nil (recorded nil
//!   on every flavor with none placed; secret on camelot by the
//!   documentation's `SecretReturns`).
//!
//! On camelot an addon's `SetRaidTarget` in combat lockdown is forbidden
//! (`ADDON_ACTION_FORBIDDEN(addon, "SetRaidTarget()")`, nothing set):
//! found in game (WoW Forever 1.60.1.70888, 2026-11-11), the unit menu's
//! raid target icons tainted by an addon's restyle were refused in combat,
//! untainted they set the marker in combat. Out of combat, and Blizzard's
//! own untainted call in combat, set it.
//!
//! Unverified: the round trip (the client may set a solo player's marker
//! at once), the other flavors' combat rule (every flavor's documentation
//! marks it `HasRestrictions`; kept unrestricted there, probe
//! `SetRaidTarget in combat`), a group's rules (who may mark), a dead or
//! vanished unit keeping its marker, and an index out of range.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut};

/// The number of markers.
pub const MARKERS: usize = 8;

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("SetRaidTarget", set_raid_target),
        ("GetRaidTargetIndex", get_raid_target_index),
    ]
}

/// A unit's marker (1 to 8), if it has one.
pub fn marker_of(vm: &Vm, unit: UnitId) -> Option<usize> {
    engine(vm)
        .units
        .raid_targets
        .iter()
        .position(|u| *u == Some(unit))
        .map(|i| i + 1)
}

/// `SetRaidTarget(unit, index)`: asks the server for the marker.
fn set_raid_target(vm: &mut Vm, c: NativeCall) -> Ret {
    // Camelot's: forbidden to an addon in combat (found in game).
    if engine(vm).flavor == "camelot" && crate::protection::forbids_in_combat(vm) {
        return Ok(vec![]);
    }
    let Some(unit) = units::unit_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let index = match vm.hidden(c.arg(2)) {
        Value::Num(n) => n.trunc(),
        _ => return Ok(vec![]),
    };
    if !(0.0..=MARKERS as f64).contains(&index) {
        return Ok(vec![]);
    }
    let answer = vm.native_with(
        "raidTargetSet",
        raid_target_set as NativeFn,
        vec![Value::Num(unit.0 as f64), Value::Num(index)],
    );
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, answer);
    Ok(vec![])
}

/// The server's answer: the marker set (or taken off), told when it
/// changed.
fn raid_target_set(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(Value::Num(unit)), Some(Value::Num(index))) =
        (c.upvals.first().copied(), c.upvals.get(1).copied())
    else {
        return Ok(vec![]);
    };
    let unit = UnitId(unit as usize);
    let index = index as usize;
    let markers = &mut engine_mut(vm).units.raid_targets;
    let before = *markers;
    for m in markers.iter_mut() {
        if *m == Some(unit) {
            *m = None;
        }
    }
    if index > 0 {
        markers[index - 1] = Some(unit);
    }
    if *markers != before {
        crate::widgets::fire_event(vm, "RAID_TARGET_UPDATE", &[]);
    }
    Ok(vec![])
}

/// `GetRaidTargetIndex(unit)`: the unit's marker, else nil.
fn get_raid_target_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let marker = units::unit_arg(vm, &c, 1).and_then(|u| marker_of(vm, u));
    Ok(vec![marker.map_or(Value::Nil, |m| Value::Num(m as f64))])
}
