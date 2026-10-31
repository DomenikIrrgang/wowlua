//! The camera (M13, decision T520): it looks at the player from behind,
//! `distance` yards from their head (the zoom), `pitch` down and turned
//! `yaw` from their back, from the client's settings (`cameraSavedDistance`
//! 5.55, `cameraSavedPitch` 10°, `cameraFov` 90°, recorded). The camera
//! functions move it (`CameraZoomIn`, `MoveViewLeftStart`, `SetView`, …);
//! what it sees places the nameplates (decision T521).

use std::f64::consts::TAU;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, engine_mut};

/// The camera's pivot: this high above the player's feet (yards;
/// unverified).
pub const PIVOT: f64 = 2.0;
/// The furthest zoom before `cameraDistanceMaxZoomFactor` (yards;
/// unverified).
const MAX_DISTANCE: f64 = 15.0;
/// The steepest pitch either way (unverified).
const MAX_PITCH: f64 = 88.0 * std::f64::consts::PI / 180.0;
/// Nearer than this before the camera is not drawn (yards).
const NEAR: f64 = 0.2;

/// Where the camera is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Yards from the pivot.
    pub distance: f64,
    /// Radians down.
    pub pitch: f64,
    /// Radians counter-clockwise from the player's back.
    pub yaw: f64,
}

/// The camera, and the views saved (`SaveView`, 1 to 5).
#[derive(Clone, Debug, Default)]
pub struct Camera {
    pub view: Option<View>,
    /// The eye last found, and what it followed (the player's place and
    /// facing, the view, the field of view, the world on): found again
    /// only when one changed (decision T537).
    pub eye: Option<([u64; 10], Eye)>,
    pub views: [Option<View>; 5],
    /// The view `SetView`, `NextView` and `PrevView` went to last (from 0).
    pub current_view: usize,
    /// Held moves (`MoveViewLeftStart(speed)`, …): each one's speed.
    pub left: f64,
    pub right: f64,
    pub up: f64,
    pub down: f64,
    pub zoom_in: f64,
    pub zoom_out: f64,
}

fn cvar(vm: &mut Vm, name: &str, default: f64) -> f64 {
    crate::framescript::cvar_get(vm, name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The camera now (at first from the client's settings).
pub fn view(vm: &mut Vm) -> View {
    if let Some(v) = engine(vm).world.camera.view {
        return v;
    }
    let v = View {
        // (as the setting has it: the full probe's 30, "30.000000")
        distance: cvar(vm, "cameraSavedDistance", 5.55).max(0.0),
        pitch: cvar(vm, "cameraSavedPitch", 10.0).to_radians(),
        yaw: 0.0,
    };
    engine_mut(vm).world.camera.view = Some(v);
    v
}

/// The camera moved to `v`: zoomed no further than the furthest zoom (or
/// where it was, past it), pitched no steeper than 88° either way.
fn set_view(vm: &mut Vm, v: View) {
    let was = view(vm).distance;
    let max = max_distance(vm).max(was);
    engine_mut(vm).world.camera.view = Some(View {
        distance: v.distance.clamp(0.0, max),
        pitch: v.pitch.clamp(-MAX_PITCH, MAX_PITCH),
        yaw: v.yaw.rem_euclid(TAU),
    });
}

fn max_distance(vm: &mut Vm) -> f64 {
    MAX_DISTANCE * cvar(vm, "cameraDistanceMaxZoomFactor", 1.0)
}

/// The held moves take `dt` seconds: turning at `cameraYawMoveSpeed`
/// (180° a second), pitching at `cameraPitchMoveSpeed` (90°), zooming at
/// `cameraZoomSpeed` (20 yards), each times its speed.
pub fn tick(vm: &mut Vm, dt: f64) {
    let c = &engine(vm).world.camera;
    let (turn, pitch, zoom) = (c.left - c.right, c.down - c.up, c.zoom_out - c.zoom_in);
    if turn == 0.0 && pitch == 0.0 && zoom == 0.0 {
        return;
    }
    let mut v = view(vm);
    v.yaw += turn * cvar(vm, "cameraYawMoveSpeed", 180.0).to_radians() * dt;
    v.pitch += pitch * cvar(vm, "cameraPitchMoveSpeed", 90.0).to_radians() * dt;
    v.distance += zoom * cvar(vm, "cameraZoomSpeed", 20.0) * dt;
    set_view(vm, v);
}

/// Where the camera is and how it is turned: its eye and its axes (north,
/// west, up), and its horizontal field of view (radians).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eye {
    pub at: (f64, f64, f64),
    pub look: (f64, f64, f64),
    pub right: (f64, f64, f64),
    pub up: (f64, f64, f64),
    pub fov: f64,
}

/// The camera's eye now, none while the player is at no place: never
/// under the ground while the 3D world is on (decision T522).
pub fn eye(vm: &mut Vm) -> Option<Eye> {
    let (y, x, z, map) = crate::character::player_world(vm)?;
    let v = view(vm);
    let facing = crate::unitinfo::player_facing(vm);
    let fov_degrees = cvar(vm, "cameraFov", 90.0);
    let world = if engine(vm).world_on { 1.0 } else { 0.0 };
    let key = [
        y,
        x,
        z,
        map,
        facing,
        v.distance,
        v.pitch,
        v.yaw,
        fov_degrees,
        world,
    ]
    .map(f64::to_bits);
    if let Some((k, e)) = engine(vm).world.camera.eye
        && k == key
    {
        return Some(e);
    }
    let turn = facing + v.yaw;
    // (north, west, up): ahead along the facing; right a quarter turn
    // clockwise of it
    let (ahead_n, ahead_w) = (turn.cos(), turn.sin());
    let look = (
        ahead_n * v.pitch.cos(),
        ahead_w * v.pitch.cos(),
        -v.pitch.sin(),
    );
    let right = (ahead_w, -ahead_n, 0.0);
    let up = (
        ahead_n * v.pitch.sin(),
        ahead_w * v.pitch.sin(),
        v.pitch.cos(),
    );
    let mut at = (
        y - look.0 * v.distance,
        x - look.1 * v.distance,
        z + PIVOT - look.2 * v.distance,
    );
    // (in front of what stands between it and the player, T524: the
    // terrain and the buildings, not the props, T532)
    let pivot = [y, x, z + PIVOT];
    if let Some(f) = crate::clientdata::hit(vm, map, pivot, [at.0, at.1, at.2], false) {
        let keep = (f - NEAR / v.distance.max(NEAR)).max(0.0);
        at = (
            pivot[0] + (at.0 - pivot[0]) * keep,
            pivot[1] + (at.1 - pivot[1]) * keep,
            pivot[2] + (at.2 - pivot[2]) * keep,
        );
    }
    if let Some(ground) = crate::clientdata::ground(vm, map, at.0, at.1) {
        at.2 = at.2.max(ground + NEAR * 2.0);
    }
    let fov = fov_degrees.to_radians().clamp(0.1, 3.0);
    let found = Eye {
        at,
        look,
        right,
        up,
        fov,
    };
    engine_mut(vm).world.camera.eye = Some((key, found));
    Some(found)
}

/// Where a place shows on a screen `width` by `height` (its center's
/// offset, right and up, in the screen's units), or none off it: the
/// camera's, a horizontal field of view of `cameraFov` (90°; unverified:
/// horizontal).
pub fn project(vm: &mut Vm, at: (f64, f64, f64), width: f64, height: f64) -> Option<(f64, f64)> {
    let e = eye(vm)?;
    let rel = (at.0 - e.at.0, at.1 - e.at.1, at.2 - e.at.2);
    let dot = |a: (f64, f64, f64)| rel.0 * a.0 + rel.1 * a.1 + rel.2 * a.2;
    let depth = dot(e.look);
    if depth < NEAR {
        return None;
    }
    let focal = width / 2.0 / (e.fov / 2.0).tan();
    let (sx, sy) = (dot(e.right) * focal / depth, dot(e.up) * focal / depth);
    (sx.abs() <= width / 2.0 && sy.abs() <= height / 2.0).then_some((sx, sy))
}

/// How far the camera turns for a pixel the mouse moves (radians, times
/// `mouseSpeed`; unverified).
const LOOK: f64 = 0.3 * std::f64::consts::PI / 180.0;

/// The mouse dragged `dx`, `dy` pixels over the world (decision T528): the
/// camera turns around the player and pitches; with `turn` (the right
/// button, as in the game) the player turns with it, the camera behind.
pub fn look(vm: &mut Vm, dx: f64, dy: f64, turn: bool) {
    let k = LOOK * cvar(vm, "mouseSpeed", 1.0);
    let mut v = view(vm);
    v.pitch += dy * k;
    if turn {
        let p = &mut engine_mut(vm).world.character.position;
        p.facing = (p.facing - dx * k + v.yaw).rem_euclid(TAU);
        v.yaw = 0.0;
    } else {
        v.yaw -= dx * k;
    }
    set_view(vm, v);
    if turn {
        crate::character::follow_places(vm);
    }
}

/// The mouse wheel turned `steps` notches over the world: the camera
/// zooms out a yard a notch (in for negative ones; decision T528).
pub fn wheel(vm: &mut Vm, steps: f64) {
    let mut v = view(vm);
    v.distance += steps;
    set_view(vm, v);
}

/// The place of the world under a point of the screen (pixels, from the
/// top left; decision T529): where the camera's line through it meets the
/// terrain or a building while the 3D world is on, else the level of the
/// player's feet; none for a point above that level or the player at no
/// place.
pub fn ground_at(vm: &mut Vm, x: f64, y: f64) -> Option<(f64, f64, f64)> {
    let (w, h) = {
        let s = &engine(vm).screen;
        (s.width, s.height)
    };
    let e = eye(vm)?;
    let map = crate::character::player_world(vm)?.3;
    let focal = w / 2.0 / (e.fov / 2.0).tan();
    let (sx, sy) = ((x - w / 2.0) / focal, (h / 2.0 - y) / focal);
    let d = (
        e.look.0 + e.right.0 * sx + e.up.0 * sy,
        e.look.1 + e.right.1 * sx + e.up.1 * sy,
        e.look.2 + e.right.2 * sx + e.up.2 * sy,
    );
    let l = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt().max(1e-9);
    let d = (d.0 / l, d.1 / l, d.2 / l);
    let reach = cvar(vm, "farclip", 1000.0);
    let end = [
        e.at.0 + d.0 * reach,
        e.at.1 + d.1 * reach,
        e.at.2 + d.2 * reach,
    ];
    let from = [e.at.0, e.at.1, e.at.2];
    if let Some(f) = crate::clientdata::hit(vm, map, from, end, false) {
        return Some((
            from[0] + (end[0] - from[0]) * f,
            from[1] + (end[1] - from[1]) * f,
            from[2] + (end[2] - from[2]) * f,
        ));
    }
    // (no world: the level of the player's feet)
    let feet = crate::character::place_of(vm, crate::units::PLAYER)?.2;
    if d.2 >= -1e-9 {
        return None;
    }
    let t = (feet - e.at.2) / d.2;
    (t > 0.0).then_some((e.at.0 + d.0 * t, e.at.1 + d.1 * t, feet))
}

/// The unit a click at a point of the screen (pixels, from the top left)
/// is on (decision T528): of the units the camera sees, the nearest to
/// the point within a yard of its middle (a yard over its feet) as the
/// screen shows a yard there; none for none.
pub fn unit_at(vm: &mut Vm, x: f64, y: f64) -> Option<crate::units::UnitId> {
    let (w, h) = {
        let s = &engine(vm).screen;
        (s.width, s.height)
    };
    let (sx, sy) = (x - w / 2.0, h / 2.0 - y);
    let e = eye(vm)?;
    let count = engine(vm).units.list.len();
    let mut best: Option<(f64, crate::units::UnitId)> = None;
    for i in 0..count {
        let id = crate::units::UnitId(i);
        if id == crate::units::PLAYER || !engine(vm).units.list[i].alive_in_world {
            continue;
        }
        let Some(at) = crate::character::place_of(vm, id) else {
            continue;
        };
        let middle = (at.0, at.1, at.2 + 1.0);
        let Some((px, py)) = project(vm, middle, w, h) else {
            continue;
        };
        let depth = (middle.0 - e.at.0) * e.look.0
            + (middle.1 - e.at.1) * e.look.1
            + (middle.2 - e.at.2) * e.look.2;
        // (a yard at that depth, in pixels)
        let yard = w / 2.0 / (e.fov / 2.0).tan() / depth.max(0.1);
        let off = (px - sx).hypot(py - sy);
        if off <= yard * 1.2 && best.is_none_or(|(o, _)| off < o) {
            best = Some((off, id));
        }
    }
    best.map(|(_, id)| id)
}

fn speed(vm: &Vm, c: &NativeCall) -> f64 {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) => n,
        _ => 1.0,
    }
}

/// `GetCameraZoom()`: the camera's distance.
fn get_camera_zoom(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(view(vm).distance)])
}

/// `CameraZoomIn(yards)`, `CameraZoomOut(yards)`: at once (unverified: the
/// client glides there).
fn zoom(vm: &mut Vm, c: &NativeCall, sign: f64) -> Ret {
    let mut v = view(vm);
    v.distance += sign * speed(vm, c);
    set_view(vm, v);
    Ok(vec![])
}

/// `FlipCameraYaw(degrees)`: the camera turned around the player.
fn flip_camera_yaw(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut v = view(vm);
    if let Value::Num(d) = vm.hidden(c.arg(1)) {
        v.yaw += d.to_radians();
    }
    set_view(vm, v);
    Ok(vec![])
}

/// The view `SetView(n)` and the others name (1 to 5).
fn view_index(vm: &Vm, c: &NativeCall) -> Option<usize> {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) if (1.0..=5.0).contains(&n) => Some(n as usize - 1),
        _ => None,
    }
}

/// `SetView(n)`: the camera where `SaveView(n)` left it (a view never
/// saved: as it is; unverified).
fn set_view_n(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(i) = view_index(vm, &c) {
        engine_mut(vm).world.camera.current_view = i;
        if let Some(v) = engine(vm).world.camera.views[i] {
            set_view(vm, v);
        }
    }
    Ok(vec![])
}

fn save_view(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(i) = view_index(vm, &c) {
        let v = view(vm);
        engine_mut(vm).world.camera.views[i] = Some(v);
    }
    Ok(vec![])
}

fn reset_view(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(i) = view_index(vm, &c) {
        engine_mut(vm).world.camera.views[i] = None;
    }
    Ok(vec![])
}

/// `NextView()`, `PrevView()`: the next or last view, around.
fn step_view(vm: &mut Vm, by: usize) -> Ret {
    let i = (engine(vm).world.camera.current_view + by) % 5;
    engine_mut(vm).world.camera.current_view = i;
    if let Some(v) = engine(vm).world.camera.views[i] {
        set_view(vm, v);
    }
    Ok(vec![])
}

/// `CenterCamera()`: the camera behind the player again (unverified).
fn center_camera(vm: &mut Vm, _c: NativeCall) -> Ret {
    let mut v = view(vm);
    v.yaw = 0.0;
    set_view(vm, v);
    Ok(vec![])
}

/// A held move (`MoveViewLeftStart(speed)`) started at its speed (1 by
/// default), or stopped.
fn hold(vm: &mut Vm, c: &NativeCall, f: fn(&mut Camera) -> &mut f64, on: bool) -> Ret {
    let s = if on { speed(vm, c) } else { 0.0 };
    *f(&mut engine_mut(vm).world.camera) = s;
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetCameraZoom", get_camera_zoom),
        ("CameraZoomIn", |vm, c| zoom(vm, &c, -1.0)),
        ("CameraZoomOut", |vm, c| zoom(vm, &c, 1.0)),
        ("FlipCameraYaw", flip_camera_yaw),
        ("SetView", set_view_n),
        ("SaveView", save_view),
        ("ResetView", reset_view),
        ("NextView", |vm, _| step_view(vm, 1)),
        ("PrevView", |vm, _| step_view(vm, 4)),
        ("CenterCamera", center_camera),
        ("MoveViewLeftStart", |vm, c| {
            hold(vm, &c, |m| &mut m.left, true)
        }),
        ("MoveViewLeftStop", |vm, c| {
            hold(vm, &c, |m| &mut m.left, false)
        }),
        ("MoveViewRightStart", |vm, c| {
            hold(vm, &c, |m| &mut m.right, true)
        }),
        ("MoveViewRightStop", |vm, c| {
            hold(vm, &c, |m| &mut m.right, false)
        }),
        ("MoveViewUpStart", |vm, c| hold(vm, &c, |m| &mut m.up, true)),
        ("MoveViewUpStop", |vm, c| hold(vm, &c, |m| &mut m.up, false)),
        ("MoveViewDownStart", |vm, c| {
            hold(vm, &c, |m| &mut m.down, true)
        }),
        ("MoveViewDownStop", |vm, c| {
            hold(vm, &c, |m| &mut m.down, false)
        }),
        ("MoveViewInStart", |vm, c| {
            hold(vm, &c, |m| &mut m.zoom_in, true)
        }),
        ("MoveViewInStop", |vm, c| {
            hold(vm, &c, |m| &mut m.zoom_in, false)
        }),
        ("MoveViewOutStart", |vm, c| {
            hold(vm, &c, |m| &mut m.zoom_out, true)
        }),
        ("MoveViewOutStop", |vm, c| {
            hold(vm, &c, |m| &mut m.zoom_out, false)
        }),
    ]
}
