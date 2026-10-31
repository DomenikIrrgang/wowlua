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

/// Playing (decision T528): a drag turns the camera (the right button
/// the player too), the wheel zooms, and a click on a unit picks it.
#[test]
fn the_mouse_turns_zooms_and_picks() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        "mock.moveTo({ world = { y = 0, x = 0, z = 0 }, facing = 0 })
Wolf = mock.spawnUnit({ name = 'Wolf', reaction = 'hostile', world = { y = 12, x = 0, z = 0 } })
-- (the mouse's speed as the character's settings have it: 1)
C_CVar.SetCVar('mouseSpeed', '1')
return GetCameraZoom()",
    );
    assert_eq!(out, ["5.55"]);
    // (a left drag: the camera only; a right drag: the player turns)
    wowlua_engine::camera::look(&mut vm, 100.0, 0.0, false);
    let facing = |vm: &mut Vm| run(vm, "return GetPlayerFacing()")[0].clone();
    assert_eq!(facing(&mut vm), "0");
    wowlua_engine::camera::look(&mut vm, -300.0, 0.0, true);
    let turned: f64 = facing(&mut vm).parse().unwrap();
    // (300 pixels at 0.3 degrees, less the camera's 100 already turned:
    // 60 degrees to the left, counter-clockwise)
    assert!((turned - 60f64.to_radians()).abs() < 0.01, "{turned}");
    wowlua_engine::camera::look(&mut vm, 0.0, 0.0, true);
    run(&mut vm, "mock.moveTo({ facing = 0 })");
    wowlua_engine::camera::wheel(&mut vm, 2.0);
    assert_eq!(run(&mut vm, "return GetCameraZoom()"), ["7.55"]);
    // (the wolf where the camera shows it: picked; the sky: nothing)
    let (w, h) = (1920.0, 1080.0);
    let (sx, sy) = wowlua_engine::camera::project(&mut vm, (12.0, 0.0, 1.0), w, h).expect("seen");
    let picked = wowlua_engine::camera::unit_at(&mut vm, w / 2.0 + sx, h / 2.0 - sy);
    assert!(picked.is_some());
    assert_eq!(wowlua_engine::camera::unit_at(&mut vm, w / 2.0, 5.0), None);
}

/// The place under a point of the screen (decision T529): without the
/// world, where the camera's line meets the level of the player's feet; a
/// place shown at a point comes back from it.
#[test]
fn a_point_of_the_screen_is_a_place_of_the_world() {
    let mut vm = vm();
    run(
        &mut vm,
        "mock.moveTo({ world = { y = 0, x = 0, z = 0 }, facing = 0 })",
    );
    let (w, h) = (1920.0, 1080.0);
    let (sx, sy) = wowlua_engine::camera::project(&mut vm, (20.0, 3.0, 0.0), w, h).expect("seen");
    let (y, x, z) =
        wowlua_engine::camera::ground_at(&mut vm, w / 2.0 + sx, h / 2.0 - sy).expect("a place");
    assert!(
        (y - 20.0).abs() < 1e-6 && (x - 3.0).abs() < 1e-6 && z == 0.0,
        "{y} {x} {z}"
    );
    // (the sky: no place)
    assert_eq!(
        wowlua_engine::camera::ground_at(&mut vm, w / 2.0, 1.0),
        None
    );
}

/// The player at (0, 0) facing north with the 3D world on; a vendor (an
/// interactive NPC) 10 yards north, a hostile wolf 12; every name and
/// plate setting off; the plate events in `Log`.
const PEOPLE: &str = "mock.moveTo({ world = { y = 0, x = 0, z = 0 }, facing = 0 })
for _, n in ipairs({ 'nameplateShowEnemies', 'nameplateShowFriendlyNpcs', 'nameplateShowFriendlyPlayers',
    'UnitNameNPC', 'UnitNameInteractiveNPC', 'UnitNameHostleNPC', 'UnitNameFriendlySpecialNPCName' }) do
    C_CVar.SetCVar(n, '0')
end
Log = {}
local f = CreateFrame('Frame')
f:RegisterEvent('NAME_PLATE_UNIT_ADDED')
f:RegisterEvent('NAME_PLATE_UNIT_REMOVED')
f:SetScript('OnEvent', function(_, e, token)
    Log[#Log + 1] = (e == 'NAME_PLATE_UNIT_ADDED' and '+' or '-') .. token
end)
Clara = mock.spawnUnit({ name = 'Clara', title = 'Cloth Armor Merchant', reaction = 'friendly', world = { y = 10, x = 0, z = 0 } })
mock.defineVendor(Clara, { items = { { id = 159 } } })
Wolf = mock.spawnUnit({ name = 'Wolf', reaction = 'hostile', world = { y = 12, x = 3, z = 0 } })
";

/// The names the 3D world draws over the units (decision T534).
fn names(vm: &mut Vm) -> Vec<String> {
    let scene = wowlua_engine::scene::build(vm);
    scene
        .items
        .iter()
        .filter_map(|i| match i {
            wowlua_engine::scene::Item::Text(t) => Some(t),
            _ => None,
        })
        .flat_map(|t| {
            t.lines
                .iter()
                .flat_map(|l| l.runs.iter().map(|r| r.text.clone()))
        })
        .filter(|t| ["Clara", "<Cloth Armor Merchant>", "Wolf"].contains(&t.as_str()))
        .collect()
}

/// With the 3D world on, the game's settings give units their nameplates
/// (decision T534): an enemy's by `nameplateShowEnemies`, a friendly
/// NPC's by `nameplateShowFriendlyNpcs`, coming and going as they change.
#[test]
fn the_settings_give_units_their_nameplates() {
    let mut vm = vm();
    wowlua_engine::set_world(&mut vm, true);
    let out = run(
        &mut vm,
        &format!(
            "{PEOPLE}mock.advanceFrames(1, 1000 / 60)
local none = #Log
C_CVar.SetCVar('nameplateShowEnemies', '1')
mock.advanceFrames(1, 1000 / 60)
C_CVar.SetCVar('nameplateShowFriendlyNpcs', '1')
mock.advanceFrames(1, 1000 / 60)
C_CVar.SetCVar('nameplateShowEnemies', '0')
mock.advanceFrames(1, 1000 / 60)
return none, table.concat(Log, ' ')"
        ),
    );
    assert_eq!(out, ["0", "+nameplate1 +nameplate2 -nameplate1"]);
}

/// With the 3D world on, the names over the units follow the game's name
/// settings (decision T534): an interactive NPC's with its title by
/// `UnitNameInteractiveNPC`, a hostile one's by `UnitNameHostleNPC`; a
/// unit whose nameplate shows has none.
#[test]
fn names_show_over_the_units_the_settings_name() {
    let mut vm = vm();
    wowlua_engine::set_world(&mut vm, true);
    run(
        &mut vm,
        &format!("{PEOPLE}mock.advanceFrames(1, 1000 / 60)"),
    );
    assert!(names(&mut vm).is_empty());
    run(
        &mut vm,
        "C_CVar.SetCVar('UnitNameInteractiveNPC', '1') mock.advanceFrames(1, 1000 / 60)",
    );
    assert_eq!(names(&mut vm), ["Clara", "<Cloth Armor Merchant>"]);
    run(
        &mut vm,
        "C_CVar.SetCVar('UnitNameHostleNPC', '1') mock.advanceFrames(1, 1000 / 60)",
    );
    // (the farther first)
    assert_eq!(names(&mut vm), ["Wolf", "Clara", "<Cloth Armor Merchant>"]);
    run(
        &mut vm,
        "C_CVar.SetCVar('nameplateShowFriendlyNpcs', '1') mock.advanceFrames(1, 1000 / 60)",
    );
    assert_eq!(names(&mut vm), ["Wolf"]);
    // (the world off: no names)
    wowlua_engine::set_world(&mut vm, false);
    assert!(names(&mut vm).is_empty());
}

/// The scene tells which items stand over units (decision T546): the
/// names over them and what their nameplates draw, so a view from another
/// camera can leave them out; the rest of the UI is not among them.
#[test]
fn the_items_over_units_are_told_apart() {
    use wowlua_engine::scene::Item;
    let mut vm = vm();
    wowlua_engine::set_world(&mut vm, true);
    run(
        &mut vm,
        &format!(
            "{PEOPLE}local f = CreateFrame('Frame')
f:RegisterEvent('NAME_PLATE_UNIT_ADDED')
f:SetScript('OnEvent', function(_, _, unit)
    local t = C_NamePlate.GetNamePlateForUnit(unit):CreateTexture()
    t:SetSize(40, 8)
    t:SetPoint('CENTER')
    t:SetColorTexture(1, 0, 0)
end)
local ui = UIParent:CreateTexture()
ui:SetSize(20, 20)
ui:SetPoint('CENTER')
ui:SetColorTexture(0, 0, 1)
C_CVar.SetCVar('UnitNameInteractiveNPC', '1')
C_CVar.SetCVar('nameplateShowEnemies', '1')
mock.advanceFrames(1, 1000 / 60)"
        ),
    );
    let scene = wowlua_engine::scene::build(&mut vm);
    let over = |i: usize| scene.over_units.iter().any(|r| r.contains(&i));
    let (mut names, mut plates, mut ui) = (Vec::new(), 0, 0);
    for (i, item) in scene.items.iter().enumerate() {
        match item {
            Item::Text(t) => {
                let text: Vec<String> = t
                    .lines
                    .iter()
                    .map(|l| l.runs.iter().map(|r| r.text.as_str()).collect())
                    .collect();
                let text = text.join("\n");
                if over(i) {
                    names.push(text);
                }
            }
            Item::Quad(q) if q.color[..3] == [1.0, 0.0, 0.0] => {
                assert!(over(i), "the nameplate's texture is over its unit");
                plates += 1;
            }
            Item::Quad(q) if q.color[..3] == [0.0, 0.0, 1.0] => {
                assert!(!over(i), "the UI's texture is not");
                ui += 1;
            }
            _ => {}
        }
    }
    assert_eq!(names, ["Clara\n<Cloth Armor Merchant>"]);
    assert_eq!((plates, ui), (1, 1));
}
