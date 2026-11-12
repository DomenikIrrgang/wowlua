//! Tapping (decision T635): who tagged an NPC first, and whether its tap
//! is denied to the player (`UnitIsTapDenied`).
//!
//! - The first player-controlled unit to hit an NPC no one has tagged
//!   (`unit:hit`, `unit:swing`, `unit:kill({ by })`) tags it: a player
//!   unit for itself, the player's pet for the player. An NPC's own blows
//!   tag nothing.
//! - `UnitIsTapDenied(unit)`: true for an NPC tagged by someone outside
//!   the player's group (another player unit, or `tappedByOther`, a player
//!   the world has no unit for); false for one the player or a group
//!   member tagged, one no one tagged, a player-controlled unit (a player,
//!   the player's pet) and a unit that does not exist (recorded false for
//!   every token on WoW Forever 1.60.1, nothing tagged).
//! - Its tap denied, or no longer, fires `UNIT_FACTION` for each of its
//!   tokens (Blizzard's target frame greys a tap-denied unit on it,
//!   `CheckFaction`). The player's own tag fires none (recorded: the
//!   `combat` scenario, a mob the player pulled and killed, no
//!   `UNIT_FACTION`).
//! - The tests' controls: `tappedByOther = true` in a unit's data (and
//!   `unit:update`), `unit:tap(by?)` (`by` a player unit or the player's
//!   pet; nil: a player outside the group) and `unit:untap()`.
//!
//! Unverified (probe "UnitIsTapDenied of a mob tagged by another"): the
//! event (`UNIT_FACTION`) for another's tag and its order among the hit's, a missed blow
//! tagging, the tag kept on the corpse, a tagger joining or leaving the
//! group (the answer follows the group, untold), the tag lost when the mob
//! leaves combat (not modelled: `unit:untap()`).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, PLAYER, UnitId, Units};
use crate::{engine, engine_mut};

/// Who tagged an NPC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tapper {
    /// A unit of the world: the player, a group member, another player.
    Unit(UnitId),
    /// A player outside the group the world has no unit for.
    Other,
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![("UnitIsTapDenied", unit_is_tap_denied)]
}

/// Whether a unit is player-controlled: a player or the player's pet.
fn controlled(u: &Units, id: UnitId) -> bool {
    u.list[id.0].player || u.pet == Some(id)
}

/// Whether the player is denied the unit's tap: an NPC tagged by someone
/// outside the player's group.
pub fn denied(u: &Units, id: UnitId) -> bool {
    if controlled(u, id) {
        return false;
    }
    match u.list[id.0].tapped {
        None => false,
        Some(Tapper::Other) => true,
        Some(Tapper::Unit(t)) => !units::ours(u, t),
    }
}

/// `UnitIsTapDenied(unit)`.
fn unit_is_tap_denied(vm: &mut Vm, c: NativeCall) -> Ret {
    let denied = units::unit_arg(vm, &c, 1).is_some_and(|id| denied(&engine(vm).units, id));
    Ok(vec![Value::Bool(denied)])
}

/// `by` hit `victim`: an NPC no one tagged is tagged by a player-controlled
/// attacker (the player's pet's blow: the player's).
pub fn hit(vm: &mut Vm, by: UnitId, victim: UnitId) {
    let u = &engine(vm).units;
    if by == victim
        || controlled(u, victim)
        || !controlled(u, by)
        || u.list[victim.0].tapped.is_some()
    {
        return;
    }
    let tapper = if u.pet == Some(by) { PLAYER } else { by };
    set(vm, victim, Some(Tapper::Unit(tapper)));
}

/// Who tagged the unit now; its tap denied or no longer: `UNIT_FACTION`.
pub fn set(vm: &mut Vm, id: UnitId, tap: Option<Tapper>) {
    let before = denied(&engine(vm).units, id);
    engine_mut(vm).units.list[id.0].tapped = tap;
    if denied(&engine(vm).units, id) != before {
        units::fire_unit(vm, "UNIT_FACTION", id, &[]);
    }
}

/// `unit:tap(by?)`: an NPC tagged by `by` (a player unit, or the player's
/// pet for the player), else by a player outside the group.
pub fn control_tap(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    if controlled(&engine(vm).units, me) {
        return Err(vm.error("tap: a player or the player's pet is never tagged"));
    }
    let tapper = match c.arg(2) {
        Value::Nil => Tapper::Other,
        v => {
            let by = units::handle_of(vm, v)
                .filter(|&by| controlled(&engine(vm).units, by))
                .ok_or_else(|| vm.error("tap(by): by is a player unit or the player's pet"))?;
            Tapper::Unit(if engine(vm).units.pet == Some(by) {
                PLAYER
            } else {
                by
            })
        }
    };
    set(vm, me, Some(tapper));
    Ok(vec![])
}

/// `unit:untap()`: no one's tag on it.
pub fn control_untap(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    set(vm, me, None);
    Ok(vec![])
}
