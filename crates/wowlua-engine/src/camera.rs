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

/// Where a place shows on a screen `width` by `height` (its center's
/// offset, right and up, in the screen's units), or none off it: the
/// camera's, a horizontal field of view of `cameraFov` (90°; unverified:
/// horizontal).
pub fn project(vm: &mut Vm, at: (f64, f64, f64), width: f64, height: f64) -> Option<(f64, f64)> {
    let (y, x, z, _) = crate::character::player_world(vm)?;
    let v = view(vm);
    let turn = crate::unitinfo::player_facing(vm) + v.yaw;
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
    let eye = (
        y - look.0 * v.distance,
        x - look.1 * v.distance,
        z + PIVOT - look.2 * v.distance,
    );
    let rel = (at.0 - eye.0, at.1 - eye.1, at.2 - eye.2);
    let dot = |a: (f64, f64, f64)| rel.0 * a.0 + rel.1 * a.1 + rel.2 * a.2;
    let depth = dot(look);
    if depth < NEAR {
        return None;
    }
    let fov = cvar(vm, "cameraFov", 90.0).to_radians().clamp(0.1, 3.0);
    let focal = width / 2.0 / (fov / 2.0).tan();
    let (sx, sy) = (dot(right) * focal / depth, dot(up) * focal / depth);
    (sx.abs() <= width / 2.0 && sy.abs() <= height / 2.0).then_some((sx, sy))
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
