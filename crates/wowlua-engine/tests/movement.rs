//! The player moves (decision T516): the movement functions the bindings
//! call hold a direction, each frame moves and turns the player at the
//! game's speeds, starting and stopping are told, the units' distances
//! follow, and a key held down runs its binding's down, let go its up.

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

/// The player at (0, 0) facing north, the events kept in `Seen`.
const START: &str = "mock.moveTo({ world = { y = 0, x = 0, z = 0 }, facing = 0 })
Seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('PLAYER_STARTED_MOVING')
f:RegisterEvent('PLAYER_STOPPED_MOVING')
f:SetScript('OnEvent', function(_, e) Seen[#Seen + 1] = e end)
";

/// A number to the tenth.
const R: &str = "local function r(n) return string.format('%.1f', n) end\n";

#[test]
fn holding_forward_runs_seven_yards_a_second_and_tells_it() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}{R}local wolf = mock.spawnUnit({{ name = 'Wolf', reaction = 'hostile', world = {{ y = 20, x = 0, z = 0 }} }})
mock.player:target(wolf)
MoveForwardStart()
local moving = IsPlayerMoving()
mock.advanceFrames(60, 1000 / 60)
MoveForwardStop()
local y, x = UnitPosition('player')
return tostring(moving), r(y), r(x), tostring(IsPlayerMoving()), table.concat(Seen, ','), tostring(CheckInteractDistance('target', 4))"
        ),
    );
    // (7 yards north in a second; the wolf 13 yards away: within 28)
    assert_eq!(
        out,
        [
            "true",
            "7.0",
            "0.0",
            "false",
            "PLAYER_STARTED_MOVING,PLAYER_STOPPED_MOVING",
            "true"
        ]
    );
}

#[test]
fn turning_in_place_turns_without_moving() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}{R}TurnLeftStart()
mock.advanceFrames(30, 1000 / 60)
TurnLeftStop()
local y, x = UnitPosition('player')
return r(GetPlayerFacing()), r(y), r(x), #Seen"
        ),
    );
    // (half a second: a quarter turn, to the west)
    assert_eq!(out, ["1.6", "0.0", "0.0", "0"]);
}

#[test]
fn walking_and_strafing_go_their_ways() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}{R}ToggleRun()
MoveForwardStart()
mock.advanceFrames(60, 1000 / 60)
MoveForwardStop()
ToggleRun()
StrafeLeftStart()
mock.advanceFrames(60, 1000 / 60)
StrafeLeftStop()
local y, x = UnitPosition('player')
return r(y), r(x)"
        ),
    );
    // (2.5 yards north walking, then 7 yards west running)
    assert_eq!(out, ["2.5", "7.0"]);
}

#[test]
fn a_key_held_down_runs_its_binding_until_let_go() {
    let mut vm = vm();
    run(
        &mut vm,
        &format!(
            "{START}mock.defineBinding('TEST_FORWARD', function(state)
    if state == 'down' then MoveForwardStart() else MoveForwardStop() end
end, {{ runOnUp = true }})
SetBinding('W', 'TEST_FORWARD')"
        ),
    );
    wowlua_engine::input::key_down(&mut vm, "W");
    let out = run(
        &mut vm,
        "mock.advanceFrames(30, 1000 / 60)
return tostring(IsPlayerMoving())",
    );
    assert_eq!(out, ["true"]);
    wowlua_engine::input::key_up(&mut vm, "W");
    let out = run(
        &mut vm,
        "local y = UnitPosition('player')
return tostring(IsPlayerMoving()), string.format('%.1f', y)",
    );
    assert_eq!(out, ["false", "3.5"]);
}

/// A unit's place to the tenth, as `UnitPosition` answers it.
const WHERE: &str = "local function at(u) mock.player:target(u) local y, x = UnitPosition('target') return string.format('%.1f,%.1f', y, x) end\n";

#[test]
fn a_patrol_walks_its_path_and_stands_at_its_end() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}{WHERE}local wolf = mock.spawnUnit({{ name = 'Wolf', world = {{ y = 10, x = 0 }} }})
wolf:patrol({{ {{ y = 10, x = 0 }}, {{ y = 10, x = 5 }} }}, {{ speed = 5, loop = false }})
mock.advanceFrames(30, 1000 / 60)
local half = at(wolf)
mock.advanceFrames(90, 1000 / 60)
return half, at(wolf)"
        ),
    );
    // (5 yards a second west: half way after half a second, then standing)
    assert_eq!(out, ["10.0,2.5", "10.0,5.0"]);
}

#[test]
fn following_keeps_its_distance_and_approaching_comes_to_reach() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}local healer = mock.spawnUnit({{ name = 'Healer', reaction = 'friendly', world = {{ y = 30, x = 0 }} }})
healer:follow({{ keep = 18 }})
local ogre = mock.spawnUnit({{ name = 'Ogre', reaction = 'hostile', world = {{ y = -20, x = 0 }} }})
ogre:approach()
mock.advanceFrames(240, 1000 / 60)
mock.player:target(healer)
local d1 = select(1, UnitDistanceSquared('target'))
mock.player:target(ogre)
return string.format('%.1f', math.sqrt(d1)), tostring(CheckInteractDistance('target', 3))"
        ),
    );
    assert_eq!(out, ["18.0", "true"]);
}

#[test]
fn fleeing_runs_away_and_move_to_goes_there() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}{WHERE}local rabbit = mock.spawnUnit({{ name = 'Rabbit', world = {{ y = 5, x = 0 }} }})
rabbit:flee()
local guard = mock.spawnUnit({{ name = 'Guard', world = {{ y = 0, x = 10 }} }})
guard:moveTo({{ y = 0, x = 20 }}, {{ speed = 5 }})
mock.advanceFrames(60, 1000 / 60)
local fled = at(rabbit)
mock.advanceFrames(120, 1000 / 60)
return fled, at(guard)"
        ),
    );
    // (7 yards a second away, north; the guard 10 yards at 5 a second)
    assert_eq!(out, ["12.0,0.0", "0.0,20.0"]);
}

#[test]
fn a_profile_s_movement_is_spawned_with_the_unit() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}{WHERE}local wolf = mock.spawnUnit({{ name = 'Wolf', world = {{ y = 0, x = 4 }},
    movement = {{ kind = 'patrol', path = {{ {{ y = 0, x = 4 }}, {{ y = 0, x = 8 }} }}, loop = false }} }})
mock.advanceFrames(120, 1000 / 60)
local ok, err = pcall(mock.spawnUnit, {{ name = 'Lost', movement = {{ kind = 'wander' }} }})
return at(wolf), tostring(ok), err"
        ),
    );
    // (walking 2.5 yards a second: there in 1.6 seconds)
    assert_eq!(out[0], "0.0,8.0");
    assert_eq!(out[1], "false");
    assert!(out[2].contains("wander"), "{out:?}");
}

/// A wall across the way 10 yards north of (0, 0), and every model's body
/// 0.5 yards wide.
struct Wall;

impl wowlua_engine::clientdata::ClientData for Wall {
    fn model_scene(&self, _: u32) -> Option<wowlua_engine::clientdata::ModelScene> {
        None
    }
    fn scene_camera(&self, _: u32) -> Option<wowlua_engine::clientdata::SceneCamera> {
        None
    }
    fn scene_actor(&self, _: u32) -> Option<wowlua_engine::clientdata::SceneActor> {
        None
    }
    fn actor_display(&self, _: u32) -> Option<wowlua_engine::clientdata::ActorDisplay> {
        None
    }
    fn walk(&self, _: u32, a: [f64; 3], b: [f64; 3], radius: f64) -> Option<f64> {
        // (one stops one's body short of it)
        let stop = 10.0 - radius;
        Some(if b[0] <= stop {
            1.0
        } else if a[0] >= stop {
            0.0
        } else {
            (stop - a[0]) / (b[0] - a[0])
        })
    }
    fn body(&self, _: &wowlua_engine::scene::ModelSource) -> Option<f64> {
        Some(0.5)
    }
}

/// One's body has its model's width (decision T532): running at a wall
/// with the 3D world on stops the player that far from it.
#[test]
fn a_wall_stops_one_a_body_short_of_it() {
    let mut vm = vm();
    wowlua_engine::clientdata::set(&mut vm, std::sync::Arc::new(Wall));
    wowlua_engine::set_world(&mut vm, true);
    let out = run(
        &mut vm,
        &format!(
            "{START}{R}MoveForwardStart()
mock.advanceFrames(120, 1000 / 60)
MoveForwardStop()
local y, x = UnitPosition('player')
return r(y), r(x)"
        ),
    );
    assert_eq!(out, ["9.5", "0.0"]);
}

/// The player's model in the 3D world: its animation and facing.
fn pose(vm: &mut Vm) -> (u32, f64) {
    let world = wowlua_engine::scene::build(vm).world.expect("the world on");
    let me = world.units.first().expect("the player");
    (me.animation, (me.facing * 100.0).round() / 100.0)
}

/// The player's model moves as it goes (decision T536): running ahead,
/// turned toward a strafe, walking backwards facing ahead, shuffling as
/// it turns in place; the right button held makes the turn keys strafe.
#[test]
fn the_model_moves_as_the_player_goes() {
    let mut vm = vm();
    wowlua_engine::set_world(&mut vm, true);
    run(&mut vm, START);
    let mut seen = Vec::new();
    for (start, stop) in [
        ("MoveForwardStart()", "MoveForwardStop()"),
        ("StrafeLeftStart()", "StrafeLeftStop()"),
        ("MoveBackwardStart()", "MoveBackwardStop()"),
        ("TurnRightStart()", "TurnRightStop()"),
    ] {
        run(&mut vm, &format!("mock.moveTo({{ facing = 0 }}) {start}"));
        seen.push(pose(&mut vm));
        run(&mut vm, stop);
    }
    assert_eq!(
        seen,
        [(5, 0.0), (5, 1.57), (13, 0.0), (12, 0.0)],
        "run, strafe left, back, turn right"
    );
    // (the right button held over the world: the turn key strafes)
    run(
        &mut vm,
        "mock.moveTo({ world = { y = 0, x = 0, z = 0 }, facing = 0 })",
    );
    let (w, h) = {
        let s = &wowlua_engine::engine(&vm).screen;
        (s.width, s.height)
    };
    wowlua_engine::play::mouse_down(&mut vm, w / 2.0, h / 4.0, true);
    let out = run(
        &mut vm,
        "TurnLeftStart()
mock.advanceFrames(60, 1000 / 60)
TurnLeftStop()
local y, x = UnitPosition('player')
return string.format('%.1f', y), string.format('%.1f', x), string.format('%.1f', GetPlayerFacing())",
    );
    wowlua_engine::play::mouse_up(&mut vm, w / 2.0, h / 4.0, true);
    // (7 yards west in a second, still facing north)
    assert_eq!(out, ["0.0", "7.0", "0.0"]);
}
