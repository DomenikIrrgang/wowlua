//! The player moving (M13, decision T516): the movement functions the
//! key bindings call (`MoveForwardStart`, `TurnLeftStart`, `ToggleRun`,
//! …) hold their direction, and each frame moves and turns the player at
//! the game's speeds; the units' distances follow (T515). Starting and
//! stopping are told as `player:setMoving` tells them.
//!
//! Speeds: running 7 yards a second, walking 2.5, backwards 4.5, a turn
//! half a circle a second (the game's defaults; unverified on WoW
//! Forever).
//!
//! Units move on their own too (decision T517): a unit's walk patrols a
//! path, follows the player, comes to them, runs from them or goes to a
//! place, each frame, facing its way; its distance follows.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut};

/// How near a unit coming to the player stops (melee reach).
pub const REACH: f64 = 2.0;

/// A place: north, west, up.
type At = (f64, f64, f64);

/// How a unit moves on its own (decision T517).
#[derive(Clone, Debug, PartialEq)]
pub enum Walk {
    /// Along its waypoints, from `next`; back to the first after the last
    /// when looping, else standing at the last.
    Patrol {
        points: Vec<At>,
        next: usize,
        speed: f64,
        looping: bool,
    },
    /// After the player, keeping `keep` yards.
    Follow { keep: f64, speed: f64 },
    /// To the player, stopping at `reach`.
    Approach { reach: f64, speed: f64 },
    /// Away from the player.
    Flee { speed: f64 },
    /// To a place, then standing.
    To { at: At, speed: f64 },
}

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
    /// The right mouse button held over the world (decision T536): the
    /// turn keys strafe, as in the game.
    pub mouselook: bool,
}

impl Movement {
    /// How it goes: forward (1), back (-1); left (1), right (-1).
    pub fn way(&self) -> (f64, f64) {
        let ahead =
            f64::from(u8::from(self.forward || self.autorun)) - f64::from(u8::from(self.backward));
        let mut side =
            f64::from(u8::from(self.strafe_left)) - f64::from(u8::from(self.strafe_right));
        if self.mouselook {
            side = (side + self.turn_keys()).clamp(-1.0, 1.0);
        }
        (ahead, side)
    }

    /// The turn keys held: left (1), right (-1).
    fn turn_keys(&self) -> f64 {
        f64::from(u8::from(self.turn_left)) - f64::from(u8::from(self.turn_right))
    }

    /// Whether the player goes anywhere (turning in place is not moving).
    pub fn moving(&self) -> bool {
        self.way() != (0.0, 0.0)
    }

    /// How it turns: left (1), right (-1); not while the right button
    /// makes the turn keys strafe.
    pub fn turn(&self) -> f64 {
        if self.mouselook {
            return 0.0;
        }
        self.turn_keys()
    }

    /// The player's model as it goes (decision T536): its animation
    /// (`AnimationData`: 0 standing, 4 walking, 5 running, 11 and 12
    /// shuffling left and right as it turns in place, 13 walking
    /// backwards) and how far it turns from the facing (radians, left
    /// positive): toward where it goes, or walking backwards the way it
    /// faces away from.
    pub fn pose(&self) -> (u32, f64) {
        let (ahead, side) = self.way();
        if ahead == 0.0 && side == 0.0 {
            let shuffle = match self.turn() {
                t if t > 0.0 => 11,
                t if t < 0.0 => 12,
                _ => 0,
            };
            return (shuffle, 0.0);
        }
        if ahead < 0.0 {
            return (13, side.atan2(ahead) - std::f64::consts::PI);
        }
        (if self.walking { 4 } else { 5 }, side.atan2(ahead))
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

/// A frame of `seconds`: the player turns and moves as held, the units
/// walk.
pub fn tick(vm: &mut Vm, seconds: f64) {
    player_tick(vm, seconds);
    walks_tick(vm, seconds);
}

/// The units that walk take a frame's step (decision T517).
fn walks_tick(vm: &mut Vm, seconds: f64) {
    let walking: Vec<usize> = engine(vm)
        .units
        .list
        .iter()
        .enumerate()
        .filter(|(_, u)| u.walk.is_some() && u.world.is_some())
        .map(|(i, _)| i)
        .collect();
    if walking.is_empty() {
        return;
    }
    let player = crate::character::place_of(vm, units::PLAYER);
    let mut moved = false;
    for i in walking {
        let id = UnitId(i);
        // (the frame's time: a patrol at a waypoint goes on to the next)
        let mut left = seconds;
        for _ in 0..8 {
            let Some(at) = crate::character::place_of(vm, id) else {
                break;
            };
            let Some(walk) = engine(vm).units.list[i].walk.clone() else {
                break;
            };
            // (where to, how fast, how near it stops)
            let (goal, speed, stop_at) = match &walk {
                Walk::Patrol {
                    points,
                    next,
                    speed,
                    ..
                } => (points.get(*next).copied(), *speed, 0.0),
                Walk::Follow { keep, speed } => (player, *speed, *keep),
                Walk::Approach { reach, speed } => (player, *speed, *reach),
                Walk::Flee { speed } => (
                    player.map(|p| (2.0 * at.0 - p.0, 2.0 * at.1 - p.1, at.2)),
                    *speed,
                    0.0,
                ),
                Walk::To { at, speed } => (Some(*at), *speed, 0.0),
            };
            let Some(goal) = goal else {
                break;
            };
            let (dn, dw) = (goal.0 - at.0, goal.1 - at.1);
            let far = dn.hypot(dw);
            let room = (far - stop_at).max(0.0);
            if room <= 1e-9 {
                arrived(vm, id, &walk);
                if matches!(walk, Walk::Patrol { .. }) {
                    continue;
                }
                break;
            }
            if speed <= 0.0 || left <= 0.0 {
                break;
            }
            let mut step = (speed * left).min(room);
            left -= step / speed;
            let (n, w) = (dn / far, dw / far);
            // (a wall stops it there, T524)
            let map = engine(vm)
                .world
                .character
                .position
                .world
                .map_or(0.0, |w| w.3);
            let to = [at.0 + n * step, at.1 + w * step, at.2];
            let body = crate::character::body_of(vm, id);
            let free = crate::clientdata::walk(vm, map, [at.0, at.1, at.2], to, body);
            let blocked = free < 1.0;
            step *= free;
            let u = &mut engine_mut(vm).units.list[i];
            u.world = Some((at.0 + n * step, at.1 + w * step));
            u.z = at.2 + (goal.2 - at.2) * (step / far);
            u.facing = Some(w.atan2(n).rem_euclid(TAU));
            moved = true;
            if blocked || step < room {
                break;
            }
            arrived(vm, id, &walk);
        }
    }
    if moved {
        crate::character::follow_places(vm);
    }
}

/// A unit at its walk's goal: a patrol's next waypoint, a place's end.
fn arrived(vm: &mut Vm, id: UnitId, walk: &Walk) {
    let next = match walk {
        Walk::Patrol {
            points,
            next,
            speed,
            looping,
        } => {
            let n = next + 1;
            if n < points.len() {
                Some(Walk::Patrol {
                    points: points.clone(),
                    next: n,
                    speed: *speed,
                    looping: *looping,
                })
            } else if *looping {
                Some(Walk::Patrol {
                    points: points.clone(),
                    next: 0,
                    speed: *speed,
                    looping: true,
                })
            } else {
                None
            }
        }
        Walk::To { .. } => None,
        // (keeping on: the player may move)
        other => Some(other.clone()),
    };
    engine_mut(vm).units.list[id.0].walk = next;
}

/// A place from a table `{ y, x, z? }`.
fn place_in(vm: &mut Vm, t: TableRef) -> Option<At> {
    let get = |vm: &mut Vm, k: &str| {
        let key = vm.str(k);
        match vm.heap.table(t).get(key) {
            Value::Num(n) => Some(n),
            _ => None,
        }
    };
    Some((get(vm, "y")?, get(vm, "x")?, get(vm, "z").unwrap_or(0.0)))
}

fn number(vm: &mut Vm, t: TableRef, k: &str) -> Option<f64> {
    let key = vm.str(k);
    match vm.heap.table(t).get(key) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// A unit's `movement` (a world profile's, `spawnUnit`'s, `update`'s):
/// `{ kind = "stand" | "patrol" | "follow" | "approach" | "flee", path,
/// loop, speed, keep }`; none: it stands.
pub fn walk_of(vm: &mut Vm, v: Value) -> Result<Option<Walk>, String> {
    let Value::Table(t) = v else {
        return Ok(None);
    };
    let key = vm.str("kind");
    let kind = match vm.heap.table(t).get(key) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "stand".into(),
    };
    let speed = number(vm, t, "speed");
    Ok(match kind.as_str() {
        "stand" => None,
        "patrol" => {
            let key = vm.str("path");
            let Value::Table(path) = vm.heap.table(t).get(key) else {
                return Err("movement: a patrol needs its path".into());
            };
            let mut points = Vec::new();
            for i in 1.. {
                match vm.heap.table(path).get(Value::Num(i as f64)) {
                    Value::Table(p) => {
                        points.push(place_in(vm, p).ok_or("movement: a waypoint is { y, x, z }")?)
                    }
                    _ => break,
                }
            }
            if points.is_empty() {
                return Err("movement: a patrol needs its path".into());
            }
            let key = vm.str("loop");
            Some(Walk::Patrol {
                points,
                next: 0,
                speed: speed.unwrap_or(WALK),
                looping: !matches!(vm.heap.table(t).get(key), Value::Bool(false)),
            })
        }
        "follow" => Some(Walk::Follow {
            keep: number(vm, t, "keep").unwrap_or(REACH),
            speed: speed.unwrap_or(RUN),
        }),
        "approach" => Some(Walk::Approach {
            reach: number(vm, t, "keep").unwrap_or(REACH),
            speed: speed.unwrap_or(RUN),
        }),
        "flee" => Some(Walk::Flee {
            speed: speed.unwrap_or(RUN),
        }),
        other => {
            return Err(format!(
                "movement: `{other}` is no kind (stand, patrol, follow, approach, flee)"
            ));
        }
    })
}

fn options(c: &NativeCall, i: usize) -> Option<TableRef> {
    match c.arg(i) {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

/// `unit:patrol(path, { speed?, loop? })`.
pub fn control_patrol(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let t = vm.new_table();
    let k = vm.str("kind");
    let v = vm.str("patrol");
    let _ = vm.heap.table_mut(t).set(k, v);
    let k = vm.str("path");
    let _ = vm.heap.table_mut(t).set(k, c.arg(2));
    if let Some(o) = options(&c, 3) {
        for name in ["speed", "loop"] {
            let k = vm.str(name);
            let v = vm.heap.table(o).get(k);
            let _ = vm.heap.table_mut(t).set(k, v);
        }
    }
    let walk = walk_of(vm, Value::Table(t)).map_err(|e| vm.error(&format!("patrol: {e}")))?;
    engine_mut(vm).units.list[me.0].walk = walk;
    Ok(vec![])
}

/// `unit:follow({ keep?, speed? })`, `unit:approach({ keep?, speed? })`,
/// `unit:flee({ speed? })`: the kind its name says.
fn control_kind(vm: &mut Vm, c: NativeCall, kind: &str) -> Ret {
    let me = units::this(vm, &c)?;
    let t = match options(&c, 2) {
        Some(o) => o,
        None => vm.new_table(),
    };
    let k = vm.str("kind");
    let v = vm.str(kind);
    let _ = vm.heap.table_mut(t).set(k, v);
    let walk = walk_of(vm, Value::Table(t)).map_err(|e| vm.error(&format!("{kind}: {e}")))?;
    engine_mut(vm).units.list[me.0].walk = walk;
    Ok(vec![])
}

pub fn control_follow(vm: &mut Vm, c: NativeCall) -> Ret {
    control_kind(vm, c, "follow")
}

pub fn control_approach(vm: &mut Vm, c: NativeCall) -> Ret {
    control_kind(vm, c, "approach")
}

pub fn control_flee(vm: &mut Vm, c: NativeCall) -> Ret {
    control_kind(vm, c, "flee")
}

/// `unit:moveTo({ y, x, z? }, { speed? })`: it goes there, then stands.
pub fn control_move_to(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let Value::Table(p) = c.arg(2) else {
        return Err(vm.error("moveTo: a place { y, x, z } expected"));
    };
    let at = place_in(vm, p).ok_or_else(|| vm.error("moveTo: a place { y, x, z } expected"))?;
    let speed = options(&c, 3)
        .and_then(|o| number(vm, o, "speed"))
        .unwrap_or(RUN);
    engine_mut(vm).units.list[me.0].walk = Some(Walk::To { at, speed });
    Ok(vec![])
}

/// `unit:stop()`: it stands where it is.
pub fn control_stop(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    engine_mut(vm).units.list[me.0].walk = None;
    Ok(vec![])
}

/// A frame of `seconds`: the player turns and moves as held.
fn player_tick(vm: &mut Vm, seconds: f64) {
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
        let (ty, tx) = (y + north * step, x + west * step);
        // (a wall stops the player; they slide along it: north, then
        // west, each as far as it lets them; T524)
        let body = crate::character::body_of(vm, crate::units::PLAYER);
        if crate::clientdata::walk(vm, instance, [y, x, z], [ty, tx, z], body) >= 1.0 {
            (ty, tx)
        } else {
            let fy = crate::clientdata::walk(vm, instance, [y, x, z], [ty, x, z], body);
            let ny = y + (ty - y) * fy;
            let fx = crate::clientdata::walk(vm, instance, [ny, x, z], [ny, tx, z], body);
            (ny, x + (tx - x) * fx)
        }
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
