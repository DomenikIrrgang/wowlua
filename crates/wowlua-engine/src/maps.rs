//! What the world map, the quest log's map and the battlefield minimap ask
//! of a map beyond its static data (that is recorded, `/stdtest engine
//! maps`, served from `data/camelot-answers.txt`): what goes on there.
//! Nothing does in the recorded world: every map's answer is the
//! recording's (`/stdtest engine answers`: the player's map 1412, the
//! world map 947, the city 1456, map 0; the fixture checks them).
//! Unverified: the maps not asked (no delve, event, invasion, bounty,
//! dig site, vignette or battleground anywhere in Forever's world).
//! Where a map lies in the world (`C_Map.GetWorldPosFromMapPos`,
//! `UnitPosition`): its recorded rectangle on its continent and the
//! continent's corner (`CONTINENTS`, unverified).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units;

fn empty_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn none(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// Whether `id` names no map: 0 or below, or one the client has no info on
/// (`C_Map.GetMapInfo` recorded giving nothing: the cosmic map 946, which
/// Forever lacks). A map never recorded is none either: the maps probe
/// recorded every map, and the full probe's maps 1 and 2147483647 have no
/// quests on them as no map has.
fn no_map(vm: &mut Vm, id: Value) -> bool {
    let Value::Num(n) = id else {
        return true;
    };
    if n <= 0.0 {
        return true;
    }
    let call = format!("C_Map.GetMapInfo({})", n as i64);
    crate::recorded::lookup_text(vm, &call).is_none_or(|v| v.is_empty())
}

/// A `Vector2DMixin` (Blizzard's `CreateVector2D`, as the client's map
/// positions are), else its fields.
pub(crate) fn vector2d(vm: &mut Vm, x: f64, y: f64) -> Ret {
    let create = vm.get_global("CreateVector2D");
    if create.is_nil() {
        let t = vm.new_table();
        for (k, n) in [("x", x), ("y", y)] {
            let key = vm.str(k);
            let _ = vm.heap.table_mut(t).set(key, Value::Num(n));
        }
        return Ok(vec![Value::Table(t)]);
    }
    let r = vm.call(create, &[Value::Num(x), Value::Num(y)])?;
    Ok(vec![r.first().copied().unwrap_or(Value::Nil)])
}

/// `C_Map.GetMapPosFromWorldPos(instanceID, worldPosition [,
/// overrideUiMapID])`: the instance's continent and the point's position
/// on it (`world_rect`'s inverse); none for another instance or an
/// override other than 0 (the full probe: instance 1 on Kalimdor, 0 on
/// the Eastern Kingdoms; positions within 1e-7 of the client's, the
/// continents' corners unverified). Unverified: an override naming a map.
fn map_pos_from_world_pos(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(instance), Value::Table(pos)) = (c.arg(1), c.arg(2)) else {
        return Ok(vec![]);
    };
    if !matches!(c.arg(3), Value::Nil) && !matches!(c.arg(3), Value::Num(n) if n == 0.0) {
        return Ok(vec![]);
    }
    let (Some(x), Some(y)) = (raw_num(vm, pos, "x"), raw_num(vm, pos, "y")) else {
        return Ok(vec![]);
    };
    let instance = instance.trunc();
    let Some(&(map, ..)) = CONTINENTS.iter().find(|c| c.1 == instance) else {
        return Ok(vec![]);
    };
    let Some((_, left, top, width, height)) = world_rect(vm, map) else {
        return Ok(vec![]);
    };
    let (u, v) = ((left - y) / width, (top - x) / height);
    let mut out = vec![Value::Num(map as f64)];
    out.extend(vector2d(vm, u, v)?);
    Ok(out)
}

/// `C_ContentTracking.GetTrackablesOnMap(type, mapID)`: success (0) and
/// none; on no map failure (`Enum.ContentTrackingResult.Failure`, 2) and
/// none (recorded: types 0 and 3 on every map, 946 the only failure; map 0
/// unverified).
fn trackables_on_map(vm: &mut Vm, c: NativeCall) -> Ret {
    let result = if no_map(vm, c.arg(2)) { 2.0 } else { 0.0 };
    Ok(vec![Value::Num(result), Value::Table(vm.new_table())])
}

/// `C_TaskQuest.GetQuestsOnMap(mapID)`: none on a map; nothing at all on
/// no map (recorded: maps 0 and 946).
fn task_quests_on_map(vm: &mut Vm, c: NativeCall) -> Ret {
    if no_map(vm, c.arg(1)) {
        return Ok(vec![]);
    }
    Ok(vec![Value::Table(vm.new_table())])
}

/// `C_DeathInfo.GetCorpseMapPosition(mapID)`: none while alive (recorded).
/// A ghost's corpse on the map is not modelled: noted as unanswered.
fn corpse_map_position(vm: &mut Vm, c: NativeCall) -> Ret {
    if crate::engine(vm).units.list[units::PLAYER.0].ghost {
        crate::note_default(vm, "C_DeathInfo.GetCorpseMapPosition", &c, 1);
    }
    Ok(vec![Value::Nil])
}

/// The two continents' world positions: their instance and the world
/// coordinates of their top left corner (`left` the world's Y, `top` its
/// X, both decreasing across the map). Unverified: not recorded. Kalimdor
/// is vanilla's `WorldMapArea` bounds; Eastern Kingdoms the corner that
/// Elwynn Forest's and Westfall's vanilla bounds (1535.42, -7939.58;
/// 3016.67, -9400) give with their recorded rectangles on it. Both agree
/// with HereBeDragons-2.0's Classic world-map table (instance 0: width
/// 44688.53, left 32601.04, top 9894.93; instance 1: 44878.66, 8723.96,
/// 14824.53) through the continents' recorded rectangles on Azeroth, and
/// the zones' recorded rectangles times the continents' sizes give the
/// zones' recorded sizes (`C_Map.GetMapWorldSize`).
const CONTINENTS: [(i64, f64, f64, f64); 2] = [
    (1414, 1.0, 17066.599609375, 12799.900390625),
    (1415, 0.0, 16000.0, 7466.6000976562),
];

fn recorded_numbers(vm: &mut Vm, call: &str) -> Option<Vec<f64>> {
    let v = crate::recorded::lookup_text(vm, call)?;
    v.iter()
        .map(|v| match v {
            Value::Num(n) => Some(*n),
            _ => None,
        })
        .collect()
}

/// A map's place in the world: its instance, the world coordinates of its
/// top left corner and its size (`C_Map.GetMapWorldSize`, recorded). A
/// zone's corner is its continent's moved by its recorded rectangle on it
/// (`C_Map.GetMapRectOnMap`); a map with no rectangle on a continent (a
/// battleground, an orphan, Azeroth) has none (unverified).
pub(crate) fn world_rect(vm: &mut Vm, map: i64) -> Option<(f64, f64, f64, f64, f64)> {
    let size = recorded_numbers(vm, &format!("C_Map.GetMapWorldSize({map})"))?;
    let &[width, height] = &size[..] else {
        return None;
    };
    for (id, instance, left, top) in CONTINENTS {
        if id == map {
            return Some((instance, left, top, width, height));
        }
        let call = format!("C_Map.GetMapRectOnMap({map}, {id})");
        let Some(&[x0, x1, y0, y1]) = recorded_numbers(vm, &call).as_deref() else {
            continue;
        };
        let continent = recorded_numbers(vm, &format!("C_Map.GetMapWorldSize({id})"));
        let (true, Some(&[cw, ch])) = (x1 > x0 && y1 > y0, continent.as_deref()) else {
            continue;
        };
        return Some((instance, left - x0 * cw, top - y0 * ch, width, height));
    }
    None
}

fn raw_num(vm: &mut Vm, t: TableRef, key: &str) -> Option<f64> {
    let k = vm.str(key);
    match vm.heap.table(t).get(k) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// `C_Map.GetWorldPosFromMapPos(uiMapID, mapPosition)`: the instance and
/// the world position (a vector, `x` the world's X, `y` its Y) of a point
/// on a map, linear over the map's world rectangle (`world_rect`). A map
/// the engine has no place for answers nothing (documented
/// `MayReturnNothing`), noted as unanswered. warcraft.wiki.gg's
/// API_C_Map.GetWorldPosFromMapPos gives Elwynn Forest's corners as these
/// give them (-7939.58, 1535.42; -10254.20, -1935.42); unverified here.
fn world_pos_from_map_pos(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(map), Value::Table(pos)) = (c.arg(1), c.arg(2)) else {
        return Ok(vec![]);
    };
    let (Some(u), Some(v)) = (raw_num(vm, pos, "x"), raw_num(vm, pos, "y")) else {
        return Ok(vec![]);
    };
    let Some((instance, left, top, width, height)) = world_rect(vm, map as i64) else {
        crate::note_default(vm, "C_Map.GetWorldPosFromMapPos", &c, 1);
        return Ok(vec![]);
    };
    let (x, y) = (top - v * height, left - u * width);
    // A `Vector2DMixin` (Blizzard's `CreateVector2D`), else its fields.
    let create = vm.get_global("CreateVector2D");
    let vector = if create.is_nil() {
        let t = vm.new_table();
        for (k, n) in [("x", x), ("y", y)] {
            let key = vm.str(k);
            let _ = vm.heap.table_mut(t).set(key, Value::Num(n));
        }
        Value::Table(t)
    } else {
        let point = [Value::Num(x), Value::Num(y)];
        vm.call(create, &point)?
            .first()
            .copied()
            .unwrap_or(Value::Nil)
    };
    Ok(vec![Value::Num(instance), vector])
}

/// `C_Map.GetMapChildrenInfo(uiMapID [, mapType [, allDescendants]])`: as
/// recorded, else the recorded children (all descendants' with the third
/// argument) of that type; nothing for a map not recorded (the full
/// probe: Warsong Gulch, type 6, under the Barrens for type 6, none for
/// the other types and for a text).
fn map_children_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(values) = crate::recorded::lookup(vm, "C_Map.GetMapChildrenInfo", &c) {
        return Ok(values);
    }
    let Value::Num(map) = c.arg(1) else {
        return Ok(vec![]);
    };
    let map = map.trunc() as i64;
    let call = if c.arg(3).truthy() {
        format!("C_Map.GetMapChildrenInfo({map}, nil, true)")
    } else {
        format!("C_Map.GetMapChildrenInfo({map})")
    };
    let Some(Value::Table(all)) =
        crate::recorded::lookup_text(vm, &call).and_then(|v| v.first().copied())
    else {
        return Ok(vec![]);
    };
    let kind = match c.arg(2) {
        Value::Nil => return Ok(vec![Value::Table(all)]),
        Value::Num(k) => Some(k),
        _ => None,
    };
    let entries: Vec<Value> = vm.heap.table(all).entries().map(|(_, v)| v).collect();
    let out = vm.new_table();
    let mut n = 0.0;
    for child in entries {
        let Value::Table(t) = child else { continue };
        if kind.is_some() && raw_num(vm, t, "mapType") == kind {
            n += 1.0;
            let _ = vm.heap.table_mut(out).set(Value::Num(n), child);
        }
    }
    Ok(vec![Value::Table(out)])
}

/// `C_Map.GetMapArtLayerTextures(uiMapID, layerIndex)`: as recorded (a
/// fraction as its whole number); another layer of a recorded map none,
/// no map nothing (the full probe: layers 0 and 2147483647 empty).
fn map_art_layer_textures(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(values) = crate::recorded::lookup(vm, "C_Map.GetMapArtLayerTextures", &c) {
        return Ok(values);
    }
    let Value::Num(map) = c.arg(1) else {
        return Ok(vec![]);
    };
    let call = format!("C_Map.GetMapArtLayerTextures({}, 1)", map.trunc() as i64);
    if crate::recorded::lookup_text(vm, &call).is_none() {
        return Ok(vec![]);
    }
    Ok(vec![Value::Table(vm.new_table())])
}

/// `C_Map.GetMapArtBackgroundAtlas(uiMapID)`: as recorded, else nil (the
/// full probe: no map).
fn map_art_background_atlas(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(
        crate::recorded::lookup(vm, "C_Map.GetMapArtBackgroundAtlas", &c)
            .unwrap_or_else(|| vec![Value::Nil]),
    )
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        // A list, empty (recorded `{}`).
        ("C_AreaPoiInfo.GetDelvesForMap", empty_list),
        ("C_AreaPoiInfo.GetEventsForMap", empty_list),
        (
            "C_ContributionCollector.GetContributionCollectorsForMap",
            empty_list,
        ),
        ("C_Garrison.GetGarrisonPlotsInstancesForMap", empty_list),
        ("C_ResearchInfo.GetDigSitesForMap", empty_list),
        ("C_QuestLine.GetAvailableQuestLines", empty_list),
        ("C_QuestLine.GetForceVisibleQuests", empty_list),
        ("C_VignetteInfo.GetVignettes", empty_list),
        // Out of a battleground (recorded on 1412; the documentation says
        // it may return nothing, the client gave a list).
        ("C_PvP.GetBattlefieldVehicles", empty_list),
        ("C_ContentTracking.GetTrackablesOnMap", trackables_on_map),
        ("C_TaskQuest.GetQuestsOnMap", task_quests_on_map),
        // nil (recorded; the wait time though documented a number).
        ("C_FogOfWar.GetFogOfWarForMap", none),
        ("C_GossipInfo.GetPoiForUiMapID", none),
        ("C_InvasionInfo.GetInvasionForUiMapID", none),
        ("C_PvP.GetOutdoorPvPWaitTime", none),
        ("C_QuestLog.GetBountiesForMapID", none),
        ("C_DeathInfo.GetCorpseMapPosition", corpse_map_position),
        // Nothing at all (recorded).
        ("C_Navigation.GetNextWaypointForMap", nothing),
        ("C_QuestLog.GetBountySetInfoForMapID", nothing),
        ("C_QuestLog.GetZoneStoryInfo", nothing),
        ("C_QuestLog.GetActiveThreatMaps", nothing),
        // No housing neighbourhood, no battleground flags (recorded).
        ("C_Housing.IsOnNeighborhoodMap", no),
        ("C_Map.GetWorldPosFromMapPos", world_pos_from_map_pos),
        ("C_Map.GetMapPosFromWorldPos", map_pos_from_world_pos),
        ("C_Map.GetMapChildrenInfo", map_children_info),
        ("C_Map.GetMapArtLayerTextures", map_art_layer_textures),
        ("C_Map.GetMapArtBackgroundAtlas", map_art_background_atlas),
        ("GetNumBattlefieldFlagPositions", zero),
    ]
}
