//! What the world map, the quest log's map and the battlefield minimap ask
//! of a map beyond its static data (that is recorded, `/stdtest engine
//! maps`, served from `data/camelot-answers.txt`): what goes on there.
//! Nothing does in the recorded world: every map's answer is the
//! recording's (`/stdtest engine answers`: the player's map 1412, the
//! world map 947, the city 1456, map 0; the fixture checks them).
//! Unverified: the maps not asked (no delve, event, invasion, bounty,
//! dig site, vignette or battleground anywhere in Forever's world).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

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
/// Forever lacks). A map never recorded counts as a map (unverified).
fn no_map(vm: &mut Vm, id: Value) -> bool {
    let Value::Num(n) = id else {
        return true;
    };
    if n <= 0.0 {
        return true;
    }
    let call = format!("C_Map.GetMapInfo({})", n as i64);
    crate::recorded::lookup_text(vm, &call).is_some_and(|v| v.is_empty())
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
        ("GetNumBattlefieldFlagPositions", zero),
    ]
}
