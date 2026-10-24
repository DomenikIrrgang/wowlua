//! What model frames and model scene actors show (spec debugger §18.9.7,
//! decision T318): the last model set (`SetModel`, `SetDisplayInfo`,
//! `SetCreature`, `SetUnit`, `SetModelByCreatureDisplayID`, …), the items
//! tried on, as the scene reads them. The setters keep their arguments as
//! any setter does (`GetDisplayInfo` reads them back).

use wowlua_vm::{NativeCall, Value, Vm};

use crate::engine;
use wowlua_vm::stdlib::Ret;

use crate::widgets::{MethodFn, WidgetId, store};

/// The hidden properties: the source's kind and value, the items tried
/// on, and whether the model was undressed.
const KIND: &str = "\u{0}model";
const VALUE: &str = "\u{0}modelvalue";
const TRIED: &str = "\u{0}tryon";
const UNDRESSED: &str = "\u{0}undressed";
const UNDRESSED_SLOTS: &str = "\u{0}undressedslots";

fn set_prop(vm: &mut Vm, id: WidgetId, key: &str, v: Value) {
    let props = engine(vm).world.widgets[id.0].props;
    let k = vm.str(key);
    let _ = vm.heap.table_mut(props).set(k, v);
}

fn prop(vm: &Vm, id: WidgetId, key: &str) -> Value {
    let props = engine(vm).world.widgets[id.0].props;
    match vm.heap.interned(key.as_bytes()) {
        Some(k) => vm.heap.table(props).get(Value::Str(k)),
        None => Value::Nil,
    }
}

/// Sets the model's source, keeping the setter's arguments under `stem`.
fn set_source(vm: &mut Vm, id: WidgetId, c: &NativeCall, stem: &str, kind: &str, value: Value) {
    let args: Vec<Value> = c.args.iter().skip(1).copied().collect();
    store(vm, id, stem, &args);
    let k = vm.str(kind);
    set_prop(vm, id, KIND, k);
    set_prop(vm, id, VALUE, value);
    set_prop(vm, id, TRIED, Value::Nil);
    set_prop(vm, id, UNDRESSED, Value::Nil);
    set_prop(vm, id, UNDRESSED_SLOTS, Value::Nil);
}

/// A file asset: an ID, or a path (a number in a string is an ID).
fn asset_kind(vm: &Vm, v: Value) -> &'static str {
    match v {
        Value::Num(_) => "file",
        Value::Str(s) => {
            let text = String::from_utf8_lossy(vm.heap.str_bytes(s));
            if text.trim().parse::<u32>().is_ok() {
                "file"
            } else {
                "path"
            }
        }
        _ => "none",
    }
}

fn file_value(vm: &mut Vm, v: Value) -> Value {
    match v {
        Value::Str(s) => {
            let text = String::from_utf8_lossy(vm.heap.str_bytes(s)).into_owned();
            match text.trim().parse::<u32>() {
                Ok(n) => Value::Num(f64::from(n)),
                Err(_) => v,
            }
        }
        v => v,
    }
}

/// `Model:SetModel(asset)`.
fn set_model(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let a = c.arg(2);
    let kind = asset_kind(vm, a);
    let v = file_value(vm, a);
    set_source(vm, id, &c, "Model", kind, v);
    Ok(vec![])
}

/// `ClearModel()`: shows nothing.
fn clear_model(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    for key in [KIND, VALUE, TRIED, UNDRESSED] {
        set_prop(vm, id, key, Value::Nil);
    }
    // and what its setters kept for the getters (`GetModelFileID` 0)
    for stem in [
        "Model",
        "ModelByFileID",
        "ModelByPath",
        "ModelByCreatureDisplayID",
        "DisplayInfo",
        "Creature",
        "Unit",
        "Item",
        "ItemAppearance",
    ] {
        set_prop(vm, id, stem, Value::Nil);
    }
    Ok(vec![])
}

/// `SetDisplayInfo(displayID)`.
fn set_display_info(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_source(vm, id, &c, "DisplayInfo", "display", c.arg(2));
    Ok(vec![])
}

/// `SetCreature(creatureID [, displayID])`: the display given, else the
/// creature's first.
fn set_creature(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    if matches!(c.arg(3), Value::Num(n) if n > 0.0) {
        set_source(vm, id, &c, "Creature", "display", c.arg(3));
    } else {
        set_source(vm, id, &c, "Creature", "creature", c.arg(2));
    }
    Ok(vec![])
}

/// `SetUnit(unit)`: the unit as it looks now; true.
fn set_unit(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_source(vm, id, &c, "Unit", "unit", c.arg(2));
    // Not dressed when the model's auto dress is off.
    if auto_dress_off(vm, id) {
        set_prop(vm, id, UNDRESSED, Value::Bool(true));
    }
    Ok(vec![Value::Bool(true)])
}

/// `SetItem(itemID)`: the item's own model.
fn set_item(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_source(vm, id, &c, "Item", "item", c.arg(2));
    Ok(vec![])
}

/// `SetItemAppearance(itemAppearanceID)`.
fn set_item_appearance(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_source(vm, id, &c, "ItemAppearance", "appearance", c.arg(2));
    Ok(vec![])
}

/// An actor's `SetModelByFileID(asset)`: true.
fn by_file(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let a = c.arg(2);
    let kind = asset_kind(vm, a);
    let v = file_value(vm, a);
    set_source(vm, id, &c, "ModelByFileID", kind, v);
    Ok(vec![Value::Bool(true)])
}

/// An actor's `SetModelByPath(asset)`: true.
fn by_path(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let a = c.arg(2);
    let kind = asset_kind(vm, a);
    let v = file_value(vm, a);
    set_source(vm, id, &c, "ModelByPath", kind, v);
    Ok(vec![Value::Bool(true)])
}

/// An actor's `SetModelByCreatureDisplayID(displayID)`: true.
fn by_display(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_source(vm, id, &c, "ModelByCreatureDisplayID", "display", c.arg(2));
    Ok(vec![Value::Bool(true)])
}

/// An actor's `SetModelByUnit(unit, sheatheWeapons, autoDress, …)`: true.
fn by_unit(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let before = worn_sources(vm, id);
    by_unit_inner(vm, id, &c);
    tell_dressed(vm, id, &before);
    Ok(vec![Value::Bool(true)])
}

fn by_unit_inner(vm: &mut Vm, id: WidgetId, c: &NativeCall) {
    set_source(vm, id, c, "ModelByUnit", "unit", c.arg(2));
    // Not dressed when `autoDress` is false, or the actor's auto dress
    // is off.
    if matches!(c.arg(4), Value::Bool(false)) || auto_dress_off(vm, id) {
        set_prop(vm, id, UNDRESSED, Value::Bool(true));
    }
}

/// Whether `SetAutoDress(false)` was the last word.
fn auto_dress_off(vm: &mut Vm, id: WidgetId) -> bool {
    let props = engine(vm).world.widgets[id.0].props;
    crate::widgets::stored_args(vm, props, "AutoDress")
        .is_some_and(|a| a.first().is_some_and(|v| !v.truthy()))
}

/// `TryOn(itemLinkOrItemModifiedAppearanceID)`: worn over what the model
/// wears (`Enum.ItemTryOnReason.Success`).
fn try_on(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let item = c.arg(2);
    if !matches!(item, Value::Num(_) | Value::Str(_)) {
        return Ok(vec![Value::Num(0.0)]);
    }
    let list = match prop(vm, id, TRIED) {
        Value::Table(t) => t,
        _ => {
            let t = vm.new_table();
            set_prop(vm, id, TRIED, Value::Table(t));
            t
        }
    };
    let before = worn_sources(vm, id);
    let n = vm.heap.table(list).border();
    let _ = vm
        .heap
        .table_mut(list)
        .set(Value::Num((n + 1) as f64), item);
    tell_dressed(vm, id, &before);
    Ok(vec![Value::Num(0.0)])
}

/// `Undress()`: nothing worn.
fn undress(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let before = worn_sources(vm, id);
    set_prop(vm, id, TRIED, Value::Nil);
    set_prop(vm, id, UNDRESSED, Value::Bool(true));
    set_prop(vm, id, UNDRESSED_SLOTS, Value::Nil);
    tell_dressed(vm, id, &before);
    Ok(vec![])
}

/// `Dress()`: what the unit wears.
fn dress(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let before = worn_sources(vm, id);
    set_prop(vm, id, TRIED, Value::Nil);
    set_prop(vm, id, UNDRESSED, Value::Nil);
    set_prop(vm, id, UNDRESSED_SLOTS, Value::Nil);
    tell_dressed(vm, id, &before);
    Ok(vec![])
}

/// `UndressSlot(slot)`: that slot shows nothing worn.
fn undress_slot(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Value::Num(slot) = c.arg(2) else {
        return Ok(vec![]);
    };
    let list = match prop(vm, id, UNDRESSED_SLOTS) {
        Value::Table(t) => t,
        _ => {
            let t = vm.new_table();
            set_prop(vm, id, UNDRESSED_SLOTS, Value::Table(t));
            t
        }
    };
    let _ = vm
        .heap
        .table_mut(list)
        .set(Value::Num(slot), Value::Bool(true));
    Ok(vec![])
}

/// The slots `UndressSlot` emptied.
pub fn undressed_slots(vm: &Vm, id: WidgetId) -> Vec<u32> {
    let Value::Table(t) = prop(vm, id, UNDRESSED_SLOTS) else {
        return Vec::new();
    };
    (1..=19u32)
        .filter(|s| vm.heap.table(t).get(Value::Num(f64::from(*s))).truthy())
        .collect()
}

/// The transmog sources a model wears, by inventory slot: its unit's
/// equipped items (none undressed, nor the slots `UndressSlot` emptied),
/// then what it tried on in its slot; each item as its source in the
/// client's tables (or the recordings').
pub fn worn_sources(vm: &mut Vm, id: WidgetId) -> std::collections::BTreeMap<u32, u32> {
    let mut out = std::collections::BTreeMap::new();
    let Some((source, tried, undressed)) = source_of(vm, id) else {
        return out;
    };
    if let Source::Unit(unit) = &source
        && !undressed
    {
        let emptied = undressed_slots(vm, id);
        for (slot, item) in crate::scene::look_of(vm, unit).items {
            if !emptied.contains(&slot)
                && let Some(s) = crate::collections::source_of_item(vm, i64::from(item))
            {
                out.insert(slot, s);
            }
        }
    }
    for t in tried {
        let entry = match t.trim().parse::<u32>() {
            // An item modified appearance's ID.
            Ok(source) => crate::clientdata::appearance_source(vm, i64::from(source))
                .map(|(_, slot)| (slot, source)),
            // An item link (`item:2575:…`) or its ID.
            Err(_) => item_of_link(&t).and_then(|item| {
                let source = crate::collections::source_of_item(vm, i64::from(item))?;
                let (_, slot) = crate::clientdata::item_source(vm, i64::from(item))?;
                Some((slot, source))
            }),
        };
        if let Some((slot, source)) = entry {
            out.insert(slot, source);
        }
    }
    out
}

/// The item ID of a link or an `item:` string.
fn item_of_link(link: &str) -> Option<u32> {
    let at = link.find("item:")? + 5;
    link[at..].split([':', '|']).next()?.parse().ok()
}

/// Tells a model scene its actor dressed (`OnDressModel(itemModified
/// AppearanceID, invSlot, removed)`), as the client does when an actor
/// puts on or takes off an item: Blizzard's dressing room lists the outfit
/// from it.
fn tell_dressed(vm: &mut Vm, id: WidgetId, before: &std::collections::BTreeMap<u32, u32>) {
    let Some(scene) = engine(vm).world.widgets[id.0].parent else {
        return;
    };
    let class = engine(vm).world.widgets[scene.0].class.clone();
    if !crate::widgets::is_a(engine(vm), &class, "ModelScene") {
        return;
    }
    let after = worn_sources(vm, id);
    let mut changes: Vec<(u32, u32, bool)> = Vec::new();
    for (slot, source) in &after {
        if before.get(slot) != Some(source) {
            changes.push((*source, *slot, false));
        }
    }
    for (slot, source) in before {
        if !after.contains_key(slot) {
            changes.push((*source, *slot, true));
        }
    }
    for (source, slot, removed) in changes {
        crate::widgets::run_script(
            vm,
            scene,
            "OnDressModel",
            &[
                Value::Num(f64::from(source)),
                Value::Num(f64::from(slot)),
                Value::Bool(removed),
            ],
        );
    }
}

/// What a model shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    File(u32),
    Path(String),
    Display(u32),
    Creature(u32),
    Unit(String),
    Item(u32),
    Appearance(u32),
}

/// A model's source, items tried on, and whether it is undressed.
pub fn source_of(vm: &Vm, id: WidgetId) -> Option<(Source, Vec<String>, bool)> {
    let Value::Str(kind) = prop(vm, id, KIND) else {
        return None;
    };
    let kind = String::from_utf8_lossy(vm.heap.str_bytes(kind)).into_owned();
    let value = vm.hidden(prop(vm, id, VALUE));
    let num = match value {
        Value::Num(n) if n >= 0.0 => Some(n as u32),
        _ => None,
    };
    let text = match value {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.heap.str_bytes(s)).into_owned()),
        _ => None,
    };
    let source = match kind.as_str() {
        "file" => Source::File(num?),
        "path" => Source::Path(text?),
        "display" => Source::Display(num?),
        "creature" => Source::Creature(num?),
        "unit" => Source::Unit(text?),
        "item" => Source::Item(num?),
        "appearance" => Source::Appearance(num?),
        _ => return None,
    };
    let mut tried = Vec::new();
    if let Value::Table(t) = prop(vm, id, TRIED) {
        let n = vm.heap.table(t).border();
        for k in 1..=n {
            match vm.heap.table(t).get(Value::Num(k as f64)) {
                Value::Str(s) => {
                    tried.push(String::from_utf8_lossy(vm.heap.str_bytes(s)).into_owned())
                }
                Value::Num(n) => tried.push(format!("{n}")),
                _ => {}
            }
        }
    }
    let undressed = prop(vm, id, UNDRESSED).truthy();
    Some((source, tried, undressed))
}

pub(crate) fn methods() -> Vec<(&'static str, MethodFn)> {
    vec![
        ("Model:SetModel", set_model as MethodFn),
        ("Model:ClearModel", clear_model),
        ("CharacterModelBase:SetDisplayInfo", set_display_info),
        ("CharacterModelBase:SetCreature", set_creature),
        ("CharacterModelBase:SetUnit", set_unit),
        ("CharacterModelBase:SetItem", set_item),
        ("CharacterModelBase:SetItemAppearance", set_item_appearance),
        ("ModelSceneFrameActorBase:ClearModel", clear_model),
        ("ModelSceneFrameActorBase:SetModelByFileID", by_file),
        ("ModelSceneFrameActorBase:SetModelByPath", by_path),
        (
            "ModelSceneFrameActorBase:SetModelByCreatureDisplayID",
            by_display,
        ),
        ("ModelSceneFrameActorBase:SetModelByUnit", by_unit),
        ("ModelSceneFrameActorBase:TryOn", try_on),
        ("ModelSceneFrameActor:Dress", dress),
        ("ModelSceneFrameActor:Undress", undress),
        ("DressUpModel:TryOn", try_on),
        ("DressUpModel:Dress", dress),
        ("DressUpModel:Undress", undress),
        ("DressUpModel:UndressSlot", undress_slot),
        ("ModelSceneFrameActor:UndressSlot", undress_slot),
    ]
}
