//! Targeting from the keyboard and the action bars (protected: Blizzard's
//! bindings and secure buttons call them; an addon's call is refused
//! first, `protection.rs`), and the player's own buffs cancelled
//! (decision T291).
//!
//! - `TargetNearestEnemy([reverse])` and its kin (`TargetNearest`,
//!   `TargetNearestEnemyPlayer`, `TargetNearestFriend`,
//!   `TargetNearestFriendPlayer`, `TargetNearestPartyMember`,
//!   `TargetNearestRaidMember`): the units in range of their kind, nearest
//!   first; with one targeted the next of them (the nearest again after
//!   the last), `reverse` the other way, as tab-targeting cycles.
//! - `TargetLastTarget()`: the target before the last change.
//! - `AssistUnit([unit])`: the unit's target (the target's by default).
//! - `FocusUnit([unit])`: the unit focused (the target by default),
//!   `PLAYER_FOCUS_CHANGED`.
//! - `CancelUnitBuff("player", index [, filter])` and
//!   `CancelSpellByName(name)`: the player's helpful aura removed
//!   (`UNIT_AURA`), as `C_UnitAuras.CancelAuraByInstanceID` does.
//!
//! Unverified, none recorded: the range (40 yards here), whether a
//! neutral creature counts as an enemy (it does here: it can be attacked),
//! the cycle's order among units at the same distance (the earlier made
//! first), a unit of unknown distance (left out), and an error message
//! when nothing is in range (none here).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, PLAYER, Reaction, UnitId};
use crate::{engine, engine_mut};

/// How far tab-targeting looks, in yards.
const RANGE: f64 = 40.0;

/// Which units a targeting function picks from.
#[derive(Clone, Copy)]
enum Kind {
    Enemy,
    EnemyPlayer,
    Friend,
    FriendPlayer,
    Party,
    Raid,
}

/// The units of `kind` in range, nearest first.
fn candidates(vm: &Vm, kind: Kind) -> Vec<UnitId> {
    let u = &engine(vm).units;
    let mut found: Vec<(f64, usize)> = u
        .list
        .iter()
        .enumerate()
        .filter(|&(i, unit)| {
            let id = UnitId(i);
            let fits = match kind {
                Kind::Enemy => unit.reaction != Reaction::Friendly,
                Kind::EnemyPlayer => unit.reaction != Reaction::Friendly && unit.player,
                Kind::Friend => unit.reaction == Reaction::Friendly,
                Kind::FriendPlayer => unit.reaction == Reaction::Friendly && unit.player,
                Kind::Party => !u.raid && u.group.contains(&id),
                Kind::Raid => u.raid && u.group.contains(&id),
            };
            id != PLAYER && unit.alive_in_world && !unit.dead && fits
        })
        .filter_map(|(i, unit)| match unit.distance {
            Some(d) if d <= RANGE => Some((d, i)),
            _ => None,
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, i)| UnitId(i)).collect()
}

/// The next unit of `kind` after the target (the first without one).
fn cycle(vm: &mut Vm, kind: Kind, reverse: bool) {
    let list = candidates(vm, kind);
    if list.is_empty() {
        return;
    }
    let n = list.len();
    let at = engine(vm)
        .units
        .target
        .and_then(|t| list.iter().position(|&u| u == t));
    let next = match (at, reverse) {
        (Some(i), false) => (i + 1) % n,
        (Some(i), true) => (i + n - 1) % n,
        (None, false) => 0,
        (None, true) => n - 1,
    };
    units::set_player_target(vm, Some(list[next]));
}

macro_rules! nearest {
    ($($name:ident => $kind:expr;)*) => {
        $(
            fn $name(vm: &mut Vm, c: NativeCall) -> Ret {
                cycle(vm, $kind, vm.hidden(c.arg(1)).truthy());
                Ok(vec![])
            }
        )*
    };
}

nearest! {
    target_nearest_enemy => Kind::Enemy;
    target_nearest_enemy_player => Kind::EnemyPlayer;
    target_nearest_friend => Kind::Friend;
    target_nearest_friend_player => Kind::FriendPlayer;
    target_nearest_party_member => Kind::Party;
    target_nearest_raid_member => Kind::Raid;
}

/// The unit an optional unit argument names, else the target.
fn unit_or_target(vm: &Vm, c: &NativeCall) -> Option<UnitId> {
    match vm.hidden(c.arg(1)) {
        Value::Nil => engine(vm).units.target,
        _ => units::unit_arg(vm, c, 1),
    }
}

fn target_last_target(vm: &mut Vm, _c: NativeCall) -> Ret {
    let last = engine(vm).units.last_target;
    if let Some(l) = last {
        let u = &engine(vm).units.list[l.0];
        if u.alive_in_world {
            units::set_player_target(vm, Some(l));
        }
    }
    Ok(vec![])
}

fn assist_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let theirs = unit_or_target(vm, &c).and_then(|u| engine(vm).units.list[u.0].target);
    if let Some(t) = theirs {
        units::set_player_target(vm, Some(t));
    }
    Ok(vec![])
}

fn focus_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let new = unit_or_target(vm, &c);
    if new.is_some() && engine(vm).units.focus != new {
        engine_mut(vm).units.focus = new;
        crate::widgets::fire_event(vm, "PLAYER_FOCUS_CHANGED", &[]);
    }
    Ok(vec![])
}

/// `CancelUnitBuff(unit, index [, filter])`: the player's `index`th
/// helpful aura passing the filter removed; another unit's stay.
fn cancel_unit_buff(vm: &mut Vm, c: NativeCall) -> Ret {
    if units::unit_arg(vm, &c, 1) != Some(PLAYER) {
        return Ok(vec![]);
    }
    let Value::Num(index) = vm.hidden(c.arg(2)) else {
        return Ok(vec![]);
    };
    let words = match vm.hidden(c.arg(3)) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    let rest: Vec<&str> = words
        .split(['|', ' '])
        .filter(|w| !w.is_empty() && !crate::auras::names_kind(w))
        .collect();
    let filter = format!("HELPFUL|{}", rest.join("|"));
    let list = crate::auras::listed(vm, PLAYER, &filter);
    let found = usize::try_from(index as i64 - 1)
        .ok()
        .and_then(|i| list.get(i))
        .map(|a| a.instance);
    if let Some(instance) = found {
        crate::auras::remove(vm, PLAYER, instance);
    }
    Ok(vec![])
}

/// `CancelSpellByName(name)`: the player's helpful aura of that name
/// removed.
fn cancel_spell_by_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(s) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let found = crate::auras::listed(vm, PLAYER, "HELPFUL")
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(&name))
        .map(|a| a.instance);
    if let Some(instance) = found {
        crate::auras::remove(vm, PLAYER, instance);
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("TargetNearest", target_nearest_enemy),
        ("TargetNearestEnemy", target_nearest_enemy),
        ("TargetNearestEnemyPlayer", target_nearest_enemy_player),
        ("TargetNearestFriend", target_nearest_friend),
        ("TargetNearestFriendPlayer", target_nearest_friend_player),
        ("TargetNearestPartyMember", target_nearest_party_member),
        ("TargetNearestRaidMember", target_nearest_raid_member),
        ("TargetLastTarget", target_last_target),
        ("AssistUnit", assist_unit),
        ("FocusUnit", focus_unit),
        ("CancelUnitBuff", cancel_unit_buff),
        ("CancelSpellByName", cancel_spell_by_name),
    ]
}
