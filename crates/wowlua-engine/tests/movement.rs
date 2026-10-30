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
        ["true", "7.0", "0.0", "false", "PLAYER_STARTED_MOVING,PLAYER_STOPPED_MOVING", "true"]
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
