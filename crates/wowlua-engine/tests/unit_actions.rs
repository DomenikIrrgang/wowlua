//! Units act on their own (decision T518): an attack swings at the player
//! every `speed` seconds while within reach, a cast comes every so many
//! seconds, a unit turns to the player when they come within range.

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

/// The player at (0, 0) facing north, 1000 health.
const START: &str = "mock.moveTo({ world = { y = 0, x = 0, z = 0 }, facing = 0 })
mock.setPlayer({ health = 1000, maxHealth = 1000 })
";

#[test]
fn an_attack_swings_every_few_seconds_while_in_reach() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}mock.spawnUnit({{ name = 'Ogre', reaction = 'hostile', distance = 3,
    actions = {{ {{ at = 0, ['do'] = 'attack', damage = 50, speed = 2 }} }} }})
mock.spawnUnit({{ name = 'Far', reaction = 'hostile', distance = 20,
    actions = {{ {{ at = 0, ['do'] = 'attack', damage = 999 }} }} }})
mock.advanceFrames(180, 1000 / 60)
return UnitHealth('player')"
        ),
    );
    // (two swings in three seconds: at once and two seconds later; the far
    // one out of reach)
    assert_eq!(out, ["900"]);
}

#[test]
fn a_cast_comes_every_few_seconds() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}Starts = 0
local f = CreateFrame('Frame')
f:RegisterEvent('UNIT_SPELLCAST_START')
f:SetScript('OnEvent', function() Starts = Starts + 1 end)
local ogre = mock.spawnUnit({{ name = 'Ogre', reaction = 'hostile', distance = 8,
    actions = {{ {{ every = 8, ['do'] = 'cast', spell = 133, castTime = 2 }} }} }})
mock.player:target(ogre)
mock.advanceFrames(9 * 60, 1000 / 60)
local once = Starts
mock.advanceFrames(8 * 60, 1000 / 60)
return once, Starts"
        ),
    );
    assert_eq!(out, ["1", "2"]);
}

#[test]
fn a_unit_turns_to_the_player_coming_within_range_and_bad_actions_say_so() {
    let mut vm = vm();
    let out = run(
        &mut vm,
        &format!(
            "{START}Wolf = mock.spawnUnit({{ name = 'Wolf', reaction = 'hostile', world = {{ y = 20, x = 0, z = 0 }}, facing = 0,
    actions = {{ {{ within = 10, ['do'] = 'face' }} }} }})
mock.advanceFrames(10, 1000 / 60)
mock.moveTo({{ world = {{ y = 12, x = 0, z = 0 }} }})
mock.advanceFrames(2, 1000 / 60)
local ok, err = pcall(mock.spawnUnit, {{ name = 'Odd', actions = {{ {{ at = 1, ['do'] = 'dance' }} }} }})
return tostring(ok), err"
        ),
    );
    assert_eq!(out[0], "false");
    assert!(out[1].contains("dance"), "{out:?}");
    // (the wolf faces south now, toward the player: half a turn)
    let wolf = wowlua_engine::engine(&vm)
        .units
        .list
        .iter()
        .find(|u| u.name == "Wolf")
        .expect("the wolf");
    let facing = wolf.facing.expect("a facing");
    assert!((facing - std::f64::consts::PI).abs() < 1e-6, "{facing}");
}
