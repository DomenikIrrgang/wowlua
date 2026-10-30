//! Units have places in the world (decision T515): a unit given a
//! distance stands that far in front of the player, its distance follows
//! the two places as either moves, `setDistance` moves it along its
//! bearing, and `UnitPosition` answers its place.

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

/// The player at (100, 200), facing north.
const HERE: &str = "mock.moveTo({ world = { y = 100, x = 200, z = 5 }, facing = 0 })\n";

#[test]
fn a_unit_given_a_distance_stands_that_far_in_front_of_the_player() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{HERE}local ogre = mock.spawnUnit({{ name = 'Ogre', reaction = 'hostile', distance = 10 }})
mock.player:target(ogre)
local y, x, z = UnitPosition('target')
return y, x, z, CheckInteractDistance('target', 3)"
        ),
    );
    assert_eq!(out, ["110", "200", "5", "false"]);
}

#[test]
fn distances_follow_the_places_as_either_moves() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{HERE}local wolf = mock.spawnUnit({{ name = 'Wolf', reaction = 'hostile', world = {{ y = 130, x = 240, z = 5 }} }})
mock.player:target(wolf)
local far = CheckInteractDistance('target', 4)
-- the player walks to it
mock.moveTo({{ world = {{ y = 127, x = 236, z = 5 }} }})
local near = CheckInteractDistance('target', 3)
-- it walks away
wolf:update({{ world = {{ y = 160, x = 236, z = 5 }} }})
return tostring(far), tostring(near), CheckInteractDistance('target', 4)"
        ),
    );
    // (50 yards: beyond 28; 5 yards: within 9.9; 33 yards: beyond 28)
    assert_eq!(out, ["false", "true", "false"]);
}

#[test]
fn set_distance_moves_a_placed_unit_along_its_bearing() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{HERE}local wolf = mock.spawnUnit({{ name = 'Wolf', reaction = 'hostile', world = {{ y = 100, x = 240, z = 5 }} }})
mock.player:target(wolf)
wolf:setDistance(4)
local y, x = UnitPosition('target')
return y, x"
        ),
    );
    // (straight west of the player, now 4 yards away)
    assert_eq!(out, ["100", "204"]);
}

#[test]
fn without_a_place_for_the_player_a_distance_is_only_a_distance() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        "local ogre = mock.spawnUnit({ name = 'Ogre', reaction = 'hostile', distance = 10 })
mock.player:target(ogre)
return UnitPosition('target'), CheckInteractDistance('target', 4)",
    );
    assert_eq!(out, ["nil", "true"]);
}
