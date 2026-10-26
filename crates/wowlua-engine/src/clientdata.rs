//! The client's own tables, where the host has the user's installation
//! (spec debugger §18.10, decision T321): model scenes, their cameras and
//! actors (`C_ModelInfo`), items, creatures, spells (decision T391), talent
//! trees (decision T435). Without one, the engine answers as before:
//! nothing.

use std::collections::HashMap;
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
    /// The UI camera the wardrobe shows an armor slot with
    /// (`UiCamFbackTransmogChrRace`): by race (`ChrRaces` ID), gender (0
    /// male, 1 female), inventory type and variation.
    fn transmog_camera(
        &self,
        _race: u32,
        _gender: u32,
        _slot: u32,
        _variation: u32,
    ) -> Option<u32> {
        None
    }
    /// A transmog source (an item modified appearance,
    /// `ItemModifiedAppearance`): its item and the inventory slot it goes
    /// in.
    fn appearance_source(&self, _source: u32) -> Option<(u32, u32)> {
        None
    }
    /// A creature's (NPC ID) first display ID.
    fn creature_display(&self, _creature: u32) -> Option<u32> {
        None
    }
    /// An area's name (`AreaTable`).
    fn area_name(&self, _area: u32) -> Option<String> {
        None
    }
    /// A game file's ID by its path (a texture's without its extension):
    /// `Some(None)` where the storage has no such file, `None` without
    /// one.
    fn game_file_id(&self, _path: &str) -> Option<Option<u32>> {
        None
    }
    /// An item's own transmog source (its first modifier's) and slot.
    fn item_source(&self, _item: u32) -> Option<(u32, u32)> {
        None
    }
    /// The UI camera the wardrobe shows a weapon with
    /// (`UiCamFbackTransmogWeapon`): by item class, subclass and inventory
    /// type.
    fn weapon_camera(&self, _class: u32, _subclass: u32, _slot: u32) -> Option<u32> {
        None
    }
    /// An item as the client's tables have it (`Item`, `ItemSearchName`).
    fn item(&self, _id: u32) -> Option<ClientItem> {
        None
    }
    /// The minimap texture (a file ID) of a map's terrain tile, column
    /// `x` and row `y` from the north-west (0 to 63).
    fn minimap_tile(&self, _map: u32, _x: u32, _y: u32) -> Option<u32> {
        None
    }
    /// A spell as the client's tables (and its hotfixes) have it
    /// (decision T391).
    fn spell(&self, _id: u32) -> Option<ClientSpell> {
        None
    }
    /// A spell's description for a caster, its numbers filled in; `None`
    /// without one or where a number cannot be given exactly.
    fn spell_description(&self, _id: u32, _caster: &SpellCaster) -> Option<String> {
        None
    }
    /// A glyph of one of the client's fonts (`Fonts\<file>`, the file's
    /// name in any case) at a pixel size: its advance and the right edge
    /// of its ink, in pixels (decision T392).
    fn glyph(&self, _font: &str, _px: u32, _c: char) -> Option<(f32, f32)> {
        None
    }
    /// The talent trees of the client's `Trait` tables (decision T435).
    fn traits(&self) -> Option<Arc<TraitTables>> {
        None
    }
}

/// The client's talent trees (its `Trait` tables), by ID: their nodes,
/// entries, definitions, edges, conditions and costs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraitTables {
    pub trees: HashMap<u32, TraitTree>,
    pub nodes: HashMap<u32, TraitNode>,
    pub entries: HashMap<u32, TraitEntry>,
    pub definitions: HashMap<u32, TraitDefinition>,
    pub conditions: HashMap<u32, TraitCondition>,
    /// The nodes of each node group (`TraitNodeGroupXTraitNode`).
    pub groups: HashMap<u32, Vec<u32>>,
}

/// A trait tree (`TraitTree`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraitTree {
    /// `Enum.TraitTreeFlag` bits (1: its ranks cannot be refunded).
    pub flags: u32,
    /// Its nodes, by ID.
    pub nodes: Vec<u32>,
    /// What a rank of its nodes costs: (trait currency, amount)
    /// (`TraitTreeXTraitCost`).
    pub costs: Vec<(u32, u32)>,
}

/// A trait node (`TraitNode` and its joins).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraitNode {
    pub tree: u32,
    pub x: i32,
    pub y: i32,
    /// `Enum.TraitNodeType`.
    pub kind: u32,
    /// `Enum.TraitNodeFlag` bits.
    pub flags: u32,
    pub sub_tree: u32,
    /// Its entries, in their index's order.
    pub entries: Vec<u32>,
    /// Its groups, in their index's order.
    pub groups: Vec<u32>,
    /// Its own conditions, then its groups' (by ID).
    pub conditions: Vec<u32>,
    /// What a rank costs beyond its tree's: its own costs, then its
    /// groups'.
    pub costs: Vec<(u32, u32)>,
    /// The edges from it to the nodes it unlocks, by ID.
    pub edges: Vec<TraitEdge>,
}

/// An edge from a node to one it unlocks (`TraitEdge`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TraitEdge {
    pub target: u32,
    /// `Enum.TraitEdgeType`.
    pub kind: u32,
    /// `Enum.TraitEdgeVisualStyle`.
    pub style: u32,
}

/// A node's entry (`TraitNodeEntry`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraitEntry {
    pub definition: u32,
    pub max_ranks: u32,
    /// `Enum.TraitNodeEntryType`.
    pub kind: u32,
    pub sub_tree: u32,
    pub conditions: Vec<u32>,
    pub costs: Vec<(u32, u32)>,
}

/// What an entry gives (`TraitDefinition`); 0 and empty for none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraitDefinition {
    pub spell: u32,
    pub override_name: String,
    pub override_subtext: String,
    pub override_description: String,
    pub override_icon: u32,
    pub overridden_spell: u32,
    pub visible_spell: u32,
}

/// A condition of nodes (`TraitCond`): the points of a currency to spend
/// in its node group (a tab of a talent tree), else its tree, first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraitCondition {
    /// `Enum.TraitConditionType`.
    pub kind: u32,
    /// `Enum.TraitCondFlag` bits (1 a gate, 2 always met, 4 sufficient).
    pub flags: u32,
    pub tree: u32,
    /// The node group the points are spent in, 0 for the tree.
    pub group: u32,
    pub currency: u32,
    /// The points to spend, 0 for none.
    pub spent: u32,
    /// "Spend %d more |4point:points; in %s …" (its `SharedString`).
    pub tooltip: Option<String>,
}

/// The client's talent trees, where its tables are at hand.
pub(crate) fn traits(vm: &Vm) -> Option<Arc<TraitTables>> {
    data(vm)?.traits()
}

/// A spell of the client's tables: its name, rank, icon, cast time,
/// range, school, cooldowns and costs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientSpell {
    pub name: String,
    /// "Rank 2", empty for none.
    pub subtext: String,
    pub icon: Option<u32>,
    pub cast_ms: u32,
    pub min_range: f32,
    pub max_range: f32,
    /// The schools (`Enum.Damageclass` bits: 1 physical, 2 holy, …).
    pub school_mask: u32,
    pub cooldown_ms: u32,
    pub gcd_ms: u32,
    pub costs: Vec<SpellCost>,
}

/// A spell's power cost as `GetSpellPowerCost` gives it (rage and runic
/// power in whole points).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpellCost {
    /// `Enum.PowerType`.
    pub power_type: i32,
    /// The most it takes (a finisher's combo points).
    pub cost: u32,
    pub min_cost: u32,
    /// In percent of the base power (0 for a fixed cost).
    pub percent: f32,
}

/// What a spell description's numbers depend on in the caster.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpellCaster {
    pub level: u32,
    /// `GetSpellBonusDamage` of the spell's school.
    pub spell_power: f32,
    /// `GetSpellBonusHealing`.
    pub healing_power: f32,
    pub attack_power: f32,
}

/// A spell as the client's tables have it.
pub(crate) fn spell(vm: &Vm, id: i64) -> Option<ClientSpell> {
    data(vm)?.spell(u32::try_from(id).ok()?)
}

/// A spell's description for a caster, as the client's tables give it.
pub(crate) fn spell_description(vm: &Vm, id: i64, caster: &SpellCaster) -> Option<String> {
    data(vm)?.spell_description(u32::try_from(id).ok()?, caster)
}

/// An item of the client's tables: its name, quality, levels, expansion,
/// stack size, sell price, bind type and description (`ItemSparse`, else the name,
/// quality and levels from `ItemSearchName`), icon, class, subclass and
/// inventory type (`Item`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientItem {
    pub name: String,
    pub quality: u32,
    pub icon: u32,
    pub class: u32,
    pub subclass: u32,
    pub inventory_type: u32,
    pub item_level: u32,
    pub required_level: u32,
    pub expansion: u32,
    /// How many to a stack, 0 when the client's table does not say.
    pub stack: u32,
    pub sell_price: u32,
    /// `Enum.ItemBind`.
    pub bind: u32,
    /// The text under its name ("Teaches you how to summon this
    /// companion."), empty for most items.
    pub description: String,
}

/// An item as the client's tables have it.
pub(crate) fn item(vm: &Vm, id: i64) -> Option<ClientItem> {
    data(vm)?.item(u32::try_from(id).ok()?)
}

/// A map's minimap texture at a terrain tile, as the client's files give
/// it.
pub(crate) fn minimap_tile(vm: &Vm, map: u32, x: u32, y: u32) -> Option<u32> {
    data(vm)?.minimap_tile(map, x, y)
}

/// The UI camera the client's tables give an item on a character of a
/// race (`ChrRaces` ID) and gender (0 male, 1 female): its slot's for the
/// race (armor), else its kind's (weapons, shields).
pub(crate) fn item_camera(
    vm: &Vm,
    item: i64,
    race: u32,
    gender: u32,
    variation: u32,
) -> Option<u32> {
    let d = data(vm)?;
    let it = d.item(u32::try_from(item).ok()?)?;
    if it.class != 2
        && let Some(cam) = d.transmog_camera(race, gender, it.inventory_type, variation)
    {
        return Some(cam);
    }
    d.weapon_camera(it.class, it.subclass, it.inventory_type)
}

/// A creature's display ID, as the client's tables (or its creature cache)
/// give it.
pub(crate) fn creature_display(vm: &Vm, creature: i64) -> Option<u32> {
    data(vm)?.creature_display(u32::try_from(creature).ok()?)
}

/// An area's name, as the client's tables give it.
pub(crate) fn area_name(vm: &Vm, area: f64) -> Option<String> {
    if area.fract() != 0.0 || !(1.0..=f64::from(u32::MAX)).contains(&area) {
        return None;
    }
    data(vm)?.area_name(area as u32)
}

/// A game file's ID by its path: `Some(None)` where the client's storage
/// lacks it, `None` without the storage.
pub(crate) fn game_file_id(vm: &Vm, path: &str) -> Option<Option<u32>> {
    data(vm)?.game_file_id(path)
}

/// A transmog source's item and slot, as the client's tables give them.
pub(crate) fn appearance_source(vm: &Vm, source: i64) -> Option<(u32, u32)> {
    data(vm)?.appearance_source(u32::try_from(source).ok()?)
}

/// An item's transmog source and slot, as the client's tables give them.
pub(crate) fn item_source(vm: &Vm, item: i64) -> Option<(u32, u32)> {
    data(vm)?.item_source(u32::try_from(item).ok()?)
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
        // The client's answers as recorded, else none: nine zeros and
        // false (the full probe: `GetUICameraInfo(0)`, `(1)`).
        let recorded = crate::facts::recorded_call(vm, c)?;
        if !recorded.is_empty() {
            return Ok(recorded);
        }
        let mut none = vec![Value::Num(0.0); 9];
        none.push(Value::Bool(false));
        return Ok(none);
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
            "C_TransmogCollection.GetAppearanceCameraID",
            crate::collections::get_appearance_camera,
        ),
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
