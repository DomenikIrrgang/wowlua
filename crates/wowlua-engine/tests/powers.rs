//! A unit has every power its class has (decision T512): `UnitPower` and
//! `UnitPowerMax` by type, the shown one without, the flavor table's
//! defaults, a profile's `powers`, and the events a change tells.

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

/// The results, a secret one as `secret:<what it hides>`.
fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm.load(src.as_bytes(), "@Test.lua").expect("compiles");
    let text = |vm: &Vm, v: Value| {
        let shown = match vm.hidden(v) {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            Value::Num(n) => format!("{n}"),
            Value::Bool(b) => format!("{b}"),
            Value::Nil => "nil".into(),
            other => format!("{other:?}"),
        };
        if !matches!(v, Value::Secret(_)) {
            shown
        } else {
            format!("secret:{shown}")
        }
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

fn run_err(vm: &mut Vm, src: &str) -> String {
    let f = vm.load(src.as_bytes(), "@Test.lua").expect("compiles");
    match vm.call(f, &[]) {
        Ok(_) => panic!("no error"),
        Err(e) => match e.value {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        },
    }
}

/// The events each change tells, with the unit and the token.
const LISTEN: &str = "local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'UNIT_POWER_UPDATE', 'UNIT_POWER_FREQUENT', 'UNIT_MAXPOWER', 'UNIT_DISPLAYPOWER' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, unit, token) seen[#seen + 1] = e .. ':' .. tostring(unit) .. (token and (':' .. token) or '') end)
";

#[test]
fn a_druid_has_mana_rage_energy_and_combo_points() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.setPlayer({ class = 'DRUID', race = 'Skyborne', level = 17, powerType = 0, maxPower = 692, power = 500 })
return UnitPowerType('player'), UnitPower('player'), UnitPowerMax('player'),
    UnitPower('player', 1), UnitPowerMax('player', 1), UnitPower('player', 3), UnitPowerMax('player', 3),
    UnitPower('player', 4), UnitPowerMax('player', 4), UnitPower('player', 9), UnitPowerMax('player', 9),
    UnitHasPowerType('player', 3), UnitHasPowerType('player', 9)",
    );
    assert_eq!(
        r,
        [
            "0",
            "secret:500",
            "692",
            // Another power it has: as secret as the shown one (unverified).
            "secret:0",
            "100",
            "secret:100",
            "100",
            // Combo points: never secret (`GetPowerTypeSecrecy(4)`).
            "0",
            "5",
            // A power it lacks: a plain 0 (recorded for a paladin).
            "0",
            "0",
            "true",
            "false"
        ]
    );
}

#[test]
fn a_class_s_defaults_and_its_shown_power() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local rogue = mock.spawnUnit({ name = 'Rogue', player = true, class = 'ROGUE', reaction = 'friendly' })
local warrior = mock.spawnUnit({ name = 'Warrior', player = true, class = 'WARRIOR', reaction = 'friendly' })
local wolf = mock.spawnUnit({ name = 'Wolf' })
mock.player:target(rogue)
mock.player:focus(warrior)
mock.player:mouseover(wolf)
local _, token = UnitPowerType('target')
local _, warriorToken = UnitPowerType('focus')
local _, wolfToken = UnitPowerType('mouseover')
return token, UnitPowerMax('target'), UnitPowerMax('target', 4), UnitPower('target', 0),
    warriorToken, UnitPowerMax('focus'), wolfToken, UnitPowerMax('mouseover'), UnitPower('mouseover', 1)",
    );
    // (another's maximum is secret on camelot; combo points never)
    assert_eq!(
        r,
        [
            "ENERGY",
            "secret:100",
            "5",
            "0",
            "RAGE",
            "secret:100",
            "MANA",
            "secret:100",
            "0"
        ]
    );
}

#[test]
fn powers_by_token_and_the_shown_one_s_old_names() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local druid = mock.spawnUnit({ name = 'Druid', player = true, class = 'DRUID', reaction = 'friendly',
    powerType = 3, maxPower = 120, powers = { MANA = { current = 300, max = 900 }, COMBO_POINTS = { current = 2 } } })
mock.player:target(druid)
local _, token = UnitPowerType('target')
return token, UnitPower('target'), UnitPowerMax('target'), UnitPower('target', 0), UnitPowerMax('target', 0),
    UnitPower('target', 4), UnitPowerPercent('target', 0)",
    );
    assert_eq!(
        r,
        [
            "ENERGY",
            "secret:100",
            "secret:120",
            "secret:300",
            "secret:900",
            "2",
            "secret:0.3333333333333333"
        ]
    );
    let err = run_err(
        &mut vm,
        "mock.spawnUnit({ name = 'X', powers = { BOGUS = { current = 1 } } })",
    );
    assert!(err.contains("not `BOGUS`"), "{err}");
}

#[test]
fn changing_a_power_tells_its_token() {
    let mut vm = vm();
    let src = format!(
        "local druid = mock.spawnUnit({{ name = 'Druid', player = true, class = 'DRUID', reaction = 'friendly' }})
mock.player:target(druid)
{LISTEN}
druid:update({{ powers = {{ RAGE = {{ current = 30 }}, ENERGY = {{ max = 110 }} }} }})
local a = table.concat(seen, ' ')
seen = {{}}
druid:update({{ powerType = 1 }})
local b = table.concat(seen, ' ')
local shown = UnitPower('target')
seen = {{}}
druid:setPower(3, {{ powerType = 'COMBO_POINTS' }})
druid:setPower(40, {{ powerType = 3, frequent = true }})
druid:setPower(12)
local c = table.concat(seen, ' ')
local ok, err = pcall(function() druid:setPower(1, {{ powerType = 'HOLY_POWER' }}) end)
return a, b, shown, c, UnitPower('target', 4), UnitPower('target', 3), UnitPower('target', 1), tostring(err)"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r[..7],
        [
            "UNIT_POWER_UPDATE:target:RAGE UNIT_MAXPOWER:target:ENERGY",
            "UNIT_DISPLAYPOWER:target",
            "secret:30",
            "UNIT_POWER_UPDATE:target:COMBO_POINTS UNIT_POWER_FREQUENT:target:ENERGY \
             UNIT_POWER_UPDATE:target:RAGE",
            "3",
            "secret:40",
            "secret:12"
        ]
    );
    assert!(r[7].contains("no power HOLY_POWER"), "{}", r[7]);
}

#[test]
fn a_new_class_brings_its_powers_and_shows_its_own() {
    let mut vm = vm();
    let src = format!(
        "local u = mock.spawnUnit({{ name = 'U', player = true, class = 'DRUID', reaction = 'friendly',
    powers = {{ RAGE = {{ current = 25 }} }} }})
mock.player:target(u)
{LISTEN}
u:update({{ class = 'WARRIOR' }})
local _, token = UnitPowerType('target')
return table.concat(seen, ' '), token, UnitPower('target'), UnitPowerMax('target', 0), UnitHasPowerType('target', 3)"
    );
    let r = run(&mut vm, &src);
    assert_eq!(
        r,
        [
            "UNIT_DISPLAYPOWER:target",
            "RAGE",
            "secret:25",
            "0",
            "false"
        ]
    );
}

#[test]
fn the_player_s_powers_are_told_as_set_player_tells_them() {
    let mut vm = vm();
    let src = format!(
        "{LISTEN}
mock.setPlayer({{ class = 'ROGUE', powers = {{ COMBO_POINTS = {{ current = 1 }} }} }}, {{ events = true }})
return table.concat(seen, ' ')"
    );
    let r = run(&mut vm, &src);
    // The warrior's rage gone; energy full as its default; a combo point.
    assert_eq!(
        r,
        ["UNIT_POWER_UPDATE:player:COMBO_POINTS UNIT_DISPLAYPOWER:player"]
    );
}
