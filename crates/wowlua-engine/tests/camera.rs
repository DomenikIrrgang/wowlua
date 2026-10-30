//! The camera (decision T520) and the nameplates where it sees their
//! units (T521).

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "@Test.lua").expect("compiles");
    let text = |vm: &Vm, v: Value| match vm.hidden(v) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{}", (n * 100.0).round() / 100.0),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

#[test]
fn the_camera_zooms_turns_and_keeps_views() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        "local at = { GetCameraZoom() }
CameraZoomOut(3)
at[#at + 1] = GetCameraZoom()
CameraZoomIn(100)
at[#at + 1] = GetCameraZoom()
MoveViewOutStart()
mock.advanceFrames(30, 1000 / 60)
MoveViewOutStop()
at[#at + 1] = GetCameraZoom()
SaveView(2)
CameraZoomOut(50)
at[#at + 1] = GetCameraZoom()
SetView(2)
at[#at + 1] = GetCameraZoom()
return table.concat(at, ' ')",
    );
    // (the client's setting, 3 out, all the way in, half a second out at
    // 20 yards a second, out to the furthest zoom, the view saved)
    assert_eq!(out, ["5.55 8.55 0 10 15 10"]);
}

/// The player at (0, 0) facing north, an ogre 20 yards north with its
/// nameplate shown; the plate events in `Log`.
const START: &str = "mock.moveTo({ world = { y = 0, x = 0, z = 0 }, facing = 0 })
Log = {}
local f = CreateFrame('Frame')
f:RegisterEvent('NAME_PLATE_UNIT_ADDED')
f:RegisterEvent('NAME_PLATE_UNIT_REMOVED')
f:SetScript('OnEvent', function(_, e, token)
    Log[#Log + 1] = (e == 'NAME_PLATE_UNIT_ADDED' and '+' or '-') .. token
end)
Ogre = mock.spawnUnit({ name = 'Ogre', reaction = 'hostile', world = { y = 20, x = 0, z = 0 } })
Ogre:showNameplate()
";

#[test]
fn a_nameplate_shows_while_the_camera_sees_its_unit() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}mock.moveTo({{ facing = math.pi }})
mock.advanceFrames(1, 1000 / 60)
mock.moveTo({{ facing = 0 }})
mock.advanceFrames(1, 1000 / 60)
Ogre:update({{ world = {{ y = 70, x = 0, z = 0 }} }})
mock.advanceFrames(1, 1000 / 60)
return table.concat(Log, ' ')"
        ),
    );
    // (in sight; turned away; back; beyond 45 yards)
    assert_eq!(out, ["+nameplate1 -nameplate1 +nameplate1 -nameplate1"]);
}

#[test]
fn a_nameplate_sits_where_the_camera_sees_its_unit() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}local plate = C_NamePlate.GetNamePlateForUnit('nameplate1')
local _, _, _, ahead = plate:GetPoint()
Ogre:update({{ world = {{ y = 20, x = -5, z = 0 }} }})
mock.advanceFrames(1, 1000 / 60)
local _, _, _, right = plate:GetPoint()
return ahead, right > 50"
        ),
    );
    // (straight ahead: the middle; 5 yards east: right of it)
    assert_eq!(out, ["0", "true"]);
}
