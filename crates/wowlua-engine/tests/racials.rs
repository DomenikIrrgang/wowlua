//! A character's faction follows its race, and it knows its race's racial
//! spells (decision T513).

use wowlua_vm::{Value, Vm};

fn vm(flavor: &str) -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, flavor).expect("installs");
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
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// A race of one faction has its own, whatever the data says; a race of
/// either the one given (none: none, its race the one recorded, the
/// Windshaper's); a player of no race a Human, an Orc of the Horde.
#[test]
fn the_faction_follows_the_race() {
    let mut vm = vm("camelot");
    let ask = "return tostring((UnitFactionGroup('player'))), UnitRace('player')";
    let player = |vm: &mut Vm, data: &str| run(vm, &format!("mock.setPlayer({data}) {ask}"));
    assert_eq!(
        player(&mut vm, "{ race = 'Orc', faction = 'Alliance' }"),
        ["Horde", "Orc", "Orc", "2"]
    );
    assert_eq!(
        player(&mut vm, "{ race = 'Human' }"),
        ["Alliance", "Human", "Human", "1"]
    );
    assert_eq!(
        player(&mut vm, "{ race = 'Skyborne', faction = 'Alliance' }"),
        ["Alliance", "High Order Skyborne", "Skyborne", "95"]
    );
    assert_eq!(
        player(&mut vm, "{ race = 'Skyborne', faction = 'Horde' }"),
        ["Horde", "Windshaper Skyborne", "Skyborne", "96"]
    );
    assert_eq!(
        player(&mut vm, "{ race = 'Skyborne' }"),
        ["nil", "Windshaper Skyborne", "Skyborne", "96"]
    );
    assert_eq!(
        player(&mut vm, "{ faction = 'Horde' }"),
        ["Horde", "Orc", "Orc", "2"]
    );
    // Another player's too.
    assert_eq!(
        run(
            &mut vm,
            "local u = mock.spawnUnit({ name = 'Other', player = true, race = 'Tauren', factionGroup = 'Alliance' })
             mock.player:target(u) return UnitFactionGroup('target')"
        ),
        ["Horde", "Horde"]
    );
}

/// The player knows its race's racial spells for its faction and class
/// without learning them (`IsPlayerSpell`, `C_SpellBook.IsSpellKnown`).
#[test]
fn the_player_knows_its_racials() {
    let mut vm = vm("camelot");
    let knows = |vm: &mut Vm, data: &str, spells: &[u32]| -> Vec<String> {
        let checks: Vec<String> = spells
            .iter()
            .map(|s| {
                format!(
                    "tostring(IsPlayerSpell({s}) == true and C_SpellBook.IsSpellKnown({s}) == true)"
                )
            })
            .collect();
        run(
            vm,
            &format!("mock.setPlayer({data}) return {}", checks.join(", ")),
        )
    };
    // The Skyborne: Walk on Air either way; Read Leyline the Alliance's
    // (the High Order), Skysight the Horde's (the Windshapers).
    let skyborne = [1259416, 1259705, 1259686];
    assert_eq!(
        knows(
            &mut vm,
            "{ race = 'Skyborne', faction = 'Alliance', class = 'MAGE' }",
            &skyborne
        ),
        ["true", "true", "false"]
    );
    assert_eq!(
        knows(
            &mut vm,
            "{ race = 'Skyborne', faction = 'Horde', class = 'SHAMAN' }",
            &skyborne
        ),
        ["true", "false", "true"]
    );
    assert_eq!(
        knows(&mut vm, "{ race = 'Skyborne', class = 'DRUID' }", &skyborne),
        ["true", "false", "false"]
    );
    // An Undead's Touch of the Grave by class; a Human's Perception, not
    // an Orc's.
    let grave = [1260189, 1260201, 20600];
    assert_eq!(
        knows(&mut vm, "{ race = 'Scourge', class = 'WARRIOR' }", &grave),
        ["true", "false", "false"]
    );
    assert_eq!(
        knows(&mut vm, "{ race = 'Human', class = 'PALADIN' }", &grave),
        ["false", "false", "true"]
    );
    assert_eq!(
        knows(
            &mut vm,
            "{ race = 'Orc', class = 'HUNTER' }",
            &[20600, 20572]
        ),
        ["false", "true"]
    );
}

/// Another flavor's racials, from its own client: a TBC Blood Elf mage's
/// Arcane Torrent, not the rogue's.
#[test]
fn other_flavors_have_theirs() {
    let mut vm = vm("tbc");
    assert_eq!(
        run(
            &mut vm,
            "mock.setPlayer({ race = 'BloodElf', class = 'MAGE' })
             return tostring(IsPlayerSpell(28730) == true), tostring(IsPlayerSpell(25046) == true), UnitFactionGroup('player')"
        ),
        ["true", "false", "Horde", "Horde"]
    );
}
