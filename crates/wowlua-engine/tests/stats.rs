//! The player's stats: the recorded answers of the client functions that
//! report them, set by a profile or `mock.player:setStats`.

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

#[test]
fn set_stats_answer_as_recorded_and_tell_their_events() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('UNIT_STATS')
f:RegisterEvent('PLAYER_XP_UPDATE')
f:RegisterEvent('UNIT_ATTACK_SPEED')
f:SetScript('OnEvent', function(_, e, unit) seen[#seen + 1] = e .. ':' .. tostring(unit) end)
mock.player:setStats({
    stat = { [1] = { 67, 67, 22, 0 } },
    attackSpeed = { n = 3, 2.6 },
    xp = 6093,
    attackPowerForStat = { [1] = 134 },
})
table.sort(seen)
return table.concat(seen, ','), UnitStat('player', 1), select('#', UnitAttackSpeed('player')),
    UnitAttackSpeed('player'), UnitXP('player'), GetAttackPowerForStat(1, 67), GetAttackPowerForStat(1, 80),
    UnitStat('player', 2)",
    );
    assert_eq!(
        r,
        [
            "PLAYER_XP_UPDATE:player,UNIT_ATTACK_SPEED:player,UNIT_STATS:player",
            "67",
            "3",
            "2.6",
            "6093",
            "134",
            // Another attribute value: the recorded rate per point (the full
            // probe: 2 a point of strength).
            "160",
            // Not set: the documented empty values (all four).
            "0",
            "0",
            "0",
            "0",
        ]
    );
}

#[test]
fn the_default_character_divides_by_its_damage_and_swing() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local percent = select(7, UnitDamage('player'))
return percent, (UnitAttackSpeed('player')), select('#', UnitDamage('target'))",
    );
    assert_eq!(r, ["1", "2", "7"]);
}

#[test]
fn unknown_stats_and_other_units_are_refused() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local a = select(2, pcall(mock.player.setStats, mock.player, { strength = 1 }))
local b = select(2, pcall(mock.player.setStats, mock.player, { stat = 1 }))
local mob = mock.spawnUnit({ name = 'Mob' })
local c = select(2, pcall(mob.setStats, mob, { xp = 1 }))
return a, b, c",
    );
    assert!(r[0].contains("no stat \"strength\""), "{r:?}");
    assert!(r[1].contains("stat is by index"), "{r:?}");
    assert!(r[2].contains("only the player"), "{r:?}");
}

/// The snapshot records the stats the engine answers (one list in each).
#[test]
fn the_snapshot_records_every_stat() {
    let snapshot = include_str!("../../../tests/ingame/src/snapshot.wowlua");
    let ask = |a: wowlua_engine::stats::Ask| match a {
        wowlua_engine::stats::Ask::Unit => "unit",
        wowlua_engine::stats::Ask::Plain => "plain",
        wowlua_engine::stats::Ask::UnitIndex => "unitIndex",
        wowlua_engine::stats::Ask::Index => "index",
        wowlua_engine::stats::Ask::ForStat => "forStat",
        wowlua_engine::stats::Ask::Location => "location",
        wowlua_engine::stats::Ask::Slot => "slot",
        wowlua_engine::stats::Ask::FlyoutSlot => "flyoutSlot",
    };
    let rows: Vec<&str> = snapshot
        .lines()
        .filter(|l| l.trim_start().starts_with("{ key = "))
        .map(str::trim)
        .collect();
    let ours: Vec<String> = wowlua_engine::stats::STATS
        .iter()
        .map(|s| {
            format!(
                "{{ key = \"{}\", fn = \"{}\", ask = \"{}\" }},",
                s.key,
                s.function,
                ask(s.ask)
            )
        })
        .collect();
    assert_eq!(rows, ours);
}
