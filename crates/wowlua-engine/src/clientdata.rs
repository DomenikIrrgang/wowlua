//! The client's own tables, where the host has the user's installation
//! (spec debugger §18.10, decision T321): model scenes, their cameras and
//! actors (`C_ModelInfo`). Without one, the engine answers as before:
//! nothing.

use std::sync::Arc;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::{engine, engine_mut};

/// A model scene (`UiModelScene`): its type, cameras, actors and flags.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelScene {
    pub kind: u32,
    pub cameras: Vec<u32>,
    pub actors: Vec<u32>,
    pub flags: u32,
}

/// A model scene's camera (`UiModelSceneCamera`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneCamera {
    pub id: u32,
    pub tag: String,
    pub kind: u32,
    pub target: [f32; 3],
    pub zoomed_target_offset: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub zoomed_yaw_offset: f32,
    pub zoomed_pitch_offset: f32,
    pub zoomed_roll_offset: f32,
    pub zoom_distance: f32,
    pub min_zoom_distance: f32,
    pub max_zoom_distance: f32,
    pub flags: u32,
}

/// A model scene's actor (`UiModelSceneActor`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneActor {
    pub id: u32,
    pub tag: String,
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub normalized_scale: f32,
    pub flags: u32,
    pub display: u32,
}

/// How an actor is shown (`UiModelSceneActorDisplay`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActorDisplay {
    pub animation: u32,
    pub variation: u32,
    pub anim_kit: u32,
    pub spell_visual_kit: u32,
    pub alpha: f32,
    pub scale: f32,
    pub anim_speed: f32,
}

/// A UI camera (`UiCamera`): where the model goes before the model
/// frame's camera, its yaw, pitch and roll (the table's `LookAt`); the
/// animation frame it shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiCamera {
    pub position: [f32; 3],
    pub angles: [f32; 3],
    pub animation: i32,
    pub frame: i32,
    pub variation: i32,
    pub flags: u32,
}

/// The client's tables, as the host reads them.
pub trait ClientData: Send + Sync {
    fn model_scene(&self, id: u32) -> Option<ModelScene>;
    fn scene_camera(&self, id: u32) -> Option<SceneCamera>;
    fn scene_actor(&self, id: u32) -> Option<SceneActor>;
    fn actor_display(&self, id: u32) -> Option<ActorDisplay>;
    /// An item's inventory type (`Enum.InventoryType`), from the client's
    /// `Item` table.
    fn item_inventory_type(&self, _id: u32) -> Option<u32> {
        None
    }
    /// A UI camera (`UiCamera`).
    fn ui_camera(&self, _id: u32) -> Option<UiCamera> {
        None
    }
    /// The minimap texture (a file ID) of a map's terrain tile, column
    /// `x` and row `y` from the north-west (0 to 63).
    fn minimap_tile(&self, _map: u32, _x: u32, _y: u32) -> Option<u32> {
        None
    }
}

/// A map's minimap texture at a terrain tile, as the client's files give
/// it.
pub(crate) fn minimap_tile(vm: &Vm, map: u32, x: u32, y: u32) -> Option<u32> {
    data(vm)?.minimap_tile(map, x, y)
}

/// An item's inventory type as the client's tables give it.
pub(crate) fn item_inventory_type(vm: &Vm, id: i64) -> Option<u32> {
    data(vm)?.item_inventory_type(u32::try_from(id).ok()?)
}

/// Gives the engine the client's tables.
pub fn set(vm: &mut Vm, data: Arc<dyn ClientData>) {
    engine_mut(vm).client_data = Some(data);
}

fn data(vm: &Vm) -> Option<Arc<dyn ClientData>> {
    engine(vm).client_data.clone()
}

fn id_arg(c: &NativeCall) -> Option<u32> {
    match c.arg(1) {
        Value::Num(n) if n >= 0.0 => Some(n as u32),
        _ => None,
    }
}

fn set_field(vm: &mut Vm, t: wowlua_vm::TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

/// A `Vector3DMixin` (Blizzard's `CreateVector3D`), else a plain table.
fn vector(vm: &mut Vm, v: [f32; 3]) -> Value {
    let args = [
        Value::Num(f64::from(v[0])),
        Value::Num(f64::from(v[1])),
        Value::Num(f64::from(v[2])),
    ];
    let f = vm.get_global("CreateVector3D");
    if matches!(f, Value::Func(_))
        && let Ok(r) = vm.call(f, &args)
        && let Some(t @ Value::Table(_)) = r.first().copied()
    {
        return t;
    }
    let t = vm.new_table();
    for (k, a) in ["x", "y", "z"].iter().zip(args) {
        set_field(vm, t, k, a);
    }
    Value::Table(t)
}

fn list(vm: &mut Vm, ids: &[u32]) -> Value {
    let t = vm.new_table();
    for (i, id) in ids.iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num((i + 1) as f64), Value::Num(f64::from(*id)));
    }
    Value::Table(t)
}

/// `C_ModelInfo.GetModelSceneInfoByID(id)`: type, camera IDs, actor IDs,
/// flags.
fn model_scene_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(s) = data(vm).and_then(|d| d.model_scene(id_arg(&c)?)) else {
        return Ok(vec![]);
    };
    let cameras = list(vm, &s.cameras);
    let actors = list(vm, &s.actors);
    Ok(vec![
        Value::Num(f64::from(s.kind)),
        cameras,
        actors,
        Value::Num(f64::from(s.flags)),
    ])
}

/// `C_ModelInfo.GetModelSceneCameraInfoByID(id)`.
fn camera_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(k) = data(vm).and_then(|d| d.scene_camera(id_arg(&c)?)) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    let num = |v: f32| Value::Num(f64::from(v));
    set_field(vm, t, "modelSceneCameraID", Value::Num(f64::from(k.id)));
    let tag = vm.str(&k.tag);
    set_field(vm, t, "scriptTag", tag);
    // The client's one camera type.
    let kind = vm.str("OrbitCamera");
    set_field(vm, t, "cameraType", kind);
    let target = vector(vm, k.target);
    set_field(vm, t, "target", target);
    set_field(vm, t, "yaw", num(k.yaw));
    set_field(vm, t, "pitch", num(k.pitch));
    set_field(vm, t, "roll", num(k.roll));
    set_field(vm, t, "zoomDistance", num(k.zoom_distance));
    set_field(vm, t, "minZoomDistance", num(k.min_zoom_distance));
    set_field(vm, t, "maxZoomDistance", num(k.max_zoom_distance));
    let offset = vector(vm, k.zoomed_target_offset);
    set_field(vm, t, "zoomedTargetOffset", offset);
    set_field(vm, t, "zoomedYawOffset", num(k.zoomed_yaw_offset));
    set_field(vm, t, "zoomedPitchOffset", num(k.zoomed_pitch_offset));
    set_field(vm, t, "zoomedRollOffset", num(k.zoomed_roll_offset));
    set_field(vm, t, "flags", Value::Num(f64::from(k.flags)));
    Ok(vec![Value::Table(t)])
}

/// `C_ModelInfo.GetModelSceneActorInfoByID(id)`.
fn actor_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(a) = data(vm).and_then(|d| d.scene_actor(id_arg(&c)?)) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    let num = |v: f32| Value::Num(f64::from(v));
    set_field(vm, t, "modelActorID", Value::Num(f64::from(a.id)));
    let tag = vm.str(&a.tag);
    set_field(vm, t, "scriptTag", tag);
    let position = vector(vm, a.position);
    set_field(vm, t, "position", position);
    set_field(vm, t, "yaw", num(a.yaw));
    set_field(vm, t, "pitch", num(a.pitch));
    set_field(vm, t, "roll", num(a.roll));
    if a.normalized_scale != 0.0 {
        set_field(
            vm,
            t,
            "normalizeScaleAggressiveness",
            num(a.normalized_scale),
        );
    }
    // Flags 1, 2, 4: the center for the origin on x, y, z (unverified).
    set_field(vm, t, "useCenterForOriginX", Value::Bool(a.flags & 1 != 0));
    set_field(vm, t, "useCenterForOriginY", Value::Bool(a.flags & 2 != 0));
    set_field(vm, t, "useCenterForOriginZ", Value::Bool(a.flags & 4 != 0));
    if a.display != 0 {
        set_field(
            vm,
            t,
            "modelActorDisplayID",
            Value::Num(f64::from(a.display)),
        );
    }
    Ok(vec![Value::Table(t)])
}

/// `C_ModelInfo.GetModelSceneActorDisplayInfoByID(id)`.
fn actor_display_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(d) = data(vm).and_then(|d| d.actor_display(id_arg(&c)?)) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    let num = |v: f32| Value::Num(f64::from(v));
    set_field(vm, t, "animation", Value::Num(f64::from(d.animation)));
    set_field(
        vm,
        t,
        "animationVariation",
        Value::Num(f64::from(d.variation)),
    );
    set_field(vm, t, "animSpeed", num(d.anim_speed));
    if d.anim_kit != 0 {
        set_field(vm, t, "animationKitID", Value::Num(f64::from(d.anim_kit)));
    }
    if d.spell_visual_kit != 0 {
        set_field(
            vm,
            t,
            "spellVisualKitID",
            Value::Num(f64::from(d.spell_visual_kit)),
        );
    }
    set_field(vm, t, "alpha", num(d.alpha));
    set_field(vm, t, "scale", num(d.scale));
    Ok(vec![Value::Table(t)])
}

/// `GetUICameraInfo(id)`: the model's position, yaw, pitch and roll
/// before the model frame's camera (the row's `Pos` and its angles), its
/// animation, variation and frame, and whether its center is the origin;
/// as the client answers (its recordings, rows 238 and 249). Without the
/// tables, the recorded answers.
fn ui_camera_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(cam) = data(vm).and_then(|d| d.ui_camera(id_arg(&c)?)) else {
        // The client's answers as recorded.
        return crate::facts::recorded_call(vm, c);
    };
    let n = |v: f32| Value::Num(f64::from(v));
    Ok(vec![
        n(cam.position[0]),
        n(cam.position[1]),
        n(cam.position[2]),
        n(cam.angles[0]),
        n(cam.angles[1]),
        n(cam.angles[2]),
        Value::Num(f64::from(cam.animation)),
        Value::Num(f64::from(cam.variation)),
        Value::Num(f64::from(cam.frame)),
        // Flag 1: the model's center is its origin (the recordings: flag 1
        // answers true).
        Value::Bool(cam.flags & 1 != 0),
    ])
}

pub(crate) fn behaviours() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        ("GetUICameraInfo", ui_camera_info as wowlua_vm::NativeFn),
        (
            "C_ModelInfo.GetModelSceneInfoByID",
            model_scene_info as wowlua_vm::NativeFn,
        ),
        ("C_ModelInfo.GetModelSceneCameraInfoByID", camera_info),
        ("C_ModelInfo.GetModelSceneActorInfoByID", actor_info),
        (
            "C_ModelInfo.GetModelSceneActorDisplayInfoByID",
            actor_display_info,
        ),
    ]
}
