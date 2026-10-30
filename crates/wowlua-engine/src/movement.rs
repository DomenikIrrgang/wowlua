//! The player moving (M13, decision T516): the movement functions the
//! key bindings call (`MoveForwardStart`, `TurnLeftStart`, `ToggleRun`,
//! …) hold their direction, and each frame moves and turns the player at
//! the game's speeds; the units' distances follow (T515). Starting and
//! stopping are told as `player:setMoving` tells them.
//!
//! Speeds: running 7 yards a second, walking 2.5, backwards 4.5, a turn
//! half a circle a second (the game's defaults; unverified on WoW
//! Forever).

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Vm};

use crate::{engine, engine_mut};

/// Running, in yards a second.
pub const RUN: f64 = 7.0;
/// Walking.
pub const WALK: f64 = 2.5;
/// Backwards (running).
pub const BACKWARD: f64 = 4.5;
/// Turning, in radians a second.
pub const TURN: f64 = PI;

/// The directions held.
#[derive(Clone, Debug, Default)]
pub struct Movement {
    pub forward: bool,
    pub backward: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub autorun: bool,
    /// `ToggleRun`: walking instead of running.
    pub walking: bool,
}

impl Movement {
    /// How it goes: forward (1), back (-1); left (1), right (-1).
    fn way(&self) -> (f64, f64) {
        let ahead =
            f64::from(u8::from(self.forward || self.autorun)) - f64::from(u8::from(self.backward));
        let side = f64::from(u8::from(self.strafe_left)) - f64::from(u8::from(self.strafe_right));
        (ahead, side)
    }

    /// Whether the player goes anywhere (turning in place is not moving).
    pub fn moving(&self) -> bool {
        self.way() != (0.0, 0.0)
    }

    fn turn(&self) -> f64 {
        f64::from(u8::from(self.turn_left)) - f64::from(u8::from(self.turn_right))
    }
}

/// Holds or lets go of a direction; starting or stopping told.
fn set(vm: &mut Vm, change: impl FnOnce(&mut Movement)) -> Ret {
    change(&mut engine_mut(vm).world.movement);
    let moving = engine(vm).world.movement.moving();
    if engine(vm).units.casting.moving != moving {
        engine_mut(vm).units.casting.moving = moving;
        let event = if moving {
            "PLAYER_STARTED_MOVING"
        } else {
            "PLAYER_STOPPED_MOVING"
        };
        crate::widgets::fire_event(vm, event, &[]);
    }
    Ok(vec![])
}

/// A frame of `seconds`: the player turns and moves as held.
pub fn tick(vm: &mut Vm, seconds: f64) {
    let m = engine(vm).world.movement.clone();
    let turn = m.turn();
    if !m.moving() && turn == 0.0 {
        return;
    }
    let Some((y, x, z, instance)) = crate::character::player_world(vm) else {
        return;
    };
    let facing =
        (engine(vm).world.character.position.facing + turn * TURN * seconds).rem_euclid(TAU);
    let (ahead, side) = m.way();
    // (north and west of the way: ahead along the facing, left a quarter
    // turn counter-clockwise from it)
    let north = ahead * facing.cos() + side * (facing + FRAC_PI_2).cos();
    let west = ahead * facing.sin() + side * (facing + FRAC_PI_2).sin();
    let length = north.hypot(west);
    let speed = if m.walking {
        WALK
    } else if ahead < 0.0 && side == 0.0 {
        BACKWARD
    } else {
        RUN
    };
    let (y, x) = if length > 0.0 {
        let step = speed * seconds / length;
        (y + north * step, x + west * step)
    } else {
        (y, x)
    };
    // (the map's position follows the world's)
    let map = engine(vm).world.character.position.map_id;
    let rect = map.and_then(|m| crate::maps::world_rect(vm, m as i64));
    let p = &mut engine_mut(vm).world.character.position;
    p.facing = facing;
    p.world = Some((y, x, z, instance));
    if let Some((_, left, top, width, height)) = rect
        && width != 0.0
        && height != 0.0
    {
        p.x = Some((left - x) / width);
        p.y = Some((top - y) / height);
    }
    crate::character::follow_places(vm);
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    fn f(f: fn(&mut Vm, NativeCall) -> Ret) -> NativeFn {
        f
    }
    vec![
        (
            "MoveForwardStart",
            f(|vm, _| {
                set(vm, |m| {
                    m.forward = true;
                    m.autorun = false;
                })
            }),
        ),
        ("MoveForwardStop", f(|vm, _| set(vm, |m| m.forward = false))),
        (
            "MoveBackwardStart",
            f(|vm, _| {
                set(vm, |m| {
                    m.backward = true;
                    m.autorun = false;
                })
            }),
        ),
        (
            "MoveBackwardStop",
            f(|vm, _| set(vm, |m| m.backward = false)),
        ),
        (
            "StrafeLeftStart",
            f(|vm, _| set(vm, |m| m.strafe_left = true)),
        ),
        (
            "StrafeLeftStop",
            f(|vm, _| set(vm, |m| m.strafe_left = false)),
        ),
        (
            "StrafeRightStart",
            f(|vm, _| set(vm, |m| m.strafe_right = true)),
        ),
        (
            "StrafeRightStop",
            f(|vm, _| set(vm, |m| m.strafe_right = false)),
        ),
        ("TurnLeftStart", f(|vm, _| set(vm, |m| m.turn_left = true))),
        ("TurnLeftStop", f(|vm, _| set(vm, |m| m.turn_left = false))),
        (
            "TurnRightStart",
            f(|vm, _| set(vm, |m| m.turn_right = true)),
        ),
        (
            "TurnRightStop",
            f(|vm, _| set(vm, |m| m.turn_right = false)),
        ),
        ("ToggleRun", f(|vm, _| set(vm, |m| m.walking = !m.walking))),
        (
            "ToggleAutoRun",
            f(|vm, _| set(vm, |m| m.autorun = !m.autorun)),
        ),
        ("StartAutoRun", f(|vm, _| set(vm, |m| m.autorun = true))),
        ("StopAutoRun", f(|vm, _| set(vm, |m| m.autorun = false))),
    ]
}
