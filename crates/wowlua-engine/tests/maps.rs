//! The maps' places in the world (`maps.rs`: `C_Map.GetWorldPosFromMapPos`,
//! `UnitPosition`) and the session's end at `ReloadUI` (`login.rs`,
//! `clock.rs`).

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    let text = |vm: &Vm, v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// A map's corners in the world, rounded to a tenth of a yard: instance,
/// top, left, bottom, right (nothing: `none`).
const CORNERS: &str = "local function corners(map)
    local i, a = C_Map.GetWorldPosFromMapPos(map, { x = 0, y = 0 })
    if not i then return 'none' end
    local _, b = C_Map.GetWorldPosFromMapPos(map, { x = 1, y = 1 })
    local r = function(n) return string.format('%.1f', n) end
    return table.concat({ i, r(a.x), r(a.y), r(b.x), r(b.y) }, ' ')
end
";

#[test]
fn zones_sit_where_vanillas_world_map_areas_put_them() {
    let mut vm = vm();
    let src = format!(
        "{CORNERS}return corners(1429), corners(1436), corners(1411), corners(1415), corners(1414)"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r,
        [
            // Elwynn Forest, Westfall, Durotar: vanilla's WorldMapArea
            // bounds (top, left, bottom, right).
            "0 -7939.6 1535.4 -10254.2 -1935.4",
            "0 -9400.0 3016.7 -11733.3 -483.3",
            "1 1808.3 -1962.5 -1716.7 -7250.0",
            // The continents.
            "0 7466.6 16000.0 -16000.0 -19199.9",
            "1 12799.9 17066.6 -11733.3 -19733.2",
        ]
    );
}

/// Maps off the two continents (recorded, the answers battery's "Map world
/// corners"): Azeroth's top left on Kalimdor and bottom right on the
/// Eastern Kingdoms, Warsong Gulch (instance 489) and map 2524 (instance
/// 2997) where they lie; the cosmic map and a map the client lacks none.
#[test]
fn maps_off_the_continents_lie_as_recorded() {
    let mut vm = vm();
    let src = format!(
        "{CORNERS}return corners(947), corners(1460), corners(2524), corners(946), corners(99999)"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r,
        [
            "1 14824.5 8724.0 -19900.2 -12087.5",
            "489 1627.1 2041.7 862.5 895.8",
            "2997 447.9 2918.8 -835.4 993.8",
            "none",
            "none"
        ]
    );
}

#[test]
fn the_player_stands_where_the_map_position_says() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.moveTo({ zone = 'Elwynn Forest', subZone = 'Northshire Valley', mapID = 1429,
    x = 0.480550229549408, y = 0.436522364616394 })
local y, x, z, instance = UnitPosition('player')
local m = C_Map.GetBestMapForUnit('player')
local p = C_Map.GetPlayerMapPosition(m, 'player')
local _, w = C_Map.GetWorldPosFromMapPos(m, p)
return string.format('%.2f %.2f', y, x), z, instance, string.format('%.2f %.2f', w.x, w.y)",
    );
    // The default character's place in Northshire.
    assert_eq!(r, ["-8949.95 -132.49", "0", "0", "-8949.95 -132.49"]);
}

#[test]
fn nothing_of_the_session_runs_after_a_reload() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local log = {}
local f = CreateFrame('Frame')
f:RegisterEvent('PLAYER_LOGOUT')
f:SetScript('OnEvent', function(_, e) log[#log + 1] = e end)
local updates = 0
f:SetScript('OnUpdate', function() updates = updates + 1 end)
C_Timer.After(1, function() log[#log + 1] = 'timer' end)
mock.advanceFrames(2)
local before = updates
C_UI.Reload()
mock.advanceFrames(120)
mock.advanceTime(5)
return table.concat(log, ' '), before, updates, mock.loggedOut(), GetTime() > 5",
    );
    assert_eq!(r, ["PLAYER_LOGOUT", "2", "2", "true", "true"]);
}
