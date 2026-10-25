//! Spells and items the test defines, read through C_Spell and C_Item.

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

#[test]
fn spells_from_the_test() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineSpell(133, { name = 'Fireball', iconID = 135812, description = 'Hurls a fiery ball.', known = true, maxRange = 35 })
local info = C_Spell.GetSpellInfo(133)
local byName = C_Spell.GetSpellInfo('Fireball')
local cd = C_Spell.GetSpellCooldown(133)
return info.name, info.iconID, info.maxRange, info.spellID, byName.spellID, C_Spell.GetSpellName(133),
    C_Spell.GetSpellLink(133), tostring(C_SpellBook.IsSpellKnown(133)), tostring(C_SpellBook.IsSpellKnown(133, 1)),
    cd.duration, tostring(cd.isEnabled), tostring((C_Spell.GetSpellInfo(2))), tostring(C_Spell.IsSpellDataCached(133))",
    );
    assert_eq!(
        r,
        [
            "Fireball",
            "135812",
            "35",
            "133",
            "133",
            "Fireball",
            "|cff71d5ff|Hspell:133:0|h[Fireball]|h|r",
            "true",
            "false",
            "0",
            "true",
            "nil",
            "true"
        ]
    );
}

/// Spells nobody defined answer from the client's spell table
/// (`data/camelot-spells.txt`, SuuUI's scan): names, ranks where read,
/// icons where read (else `Interface\Icons\Temp`'s); probed spells win
/// (1038 is the trainers' Salvation, the scan's Blessing of Salvation).
#[test]
fn spells_from_the_client_spell_table() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local info = C_Spell.GetSpellInfo(355)
return info.name, info.iconID, info.castTime, C_Spell.GetSpellName(5), C_Spell.GetSpellTexture(5),
    C_Spell.GetSpellSubtext(768), C_Spell.GetSpellSubtext(355), C_Spell.GetSpellLink(355),
    tostring(C_Spell.DoesSpellExist(355)), tostring(C_Spell.DoesSpellExist(2)), tostring(C_Spell.IsSpellDataCached(355)),
    C_Spell.GetSpellName(1038), tostring((C_Spell.GetSpellName(2)))",
    );
    assert_eq!(
        r,
        [
            "Taunt",
            "136080",
            "0",
            "Death Touch",
            "136235",
            "Shapeshift",
            "",
            "|cff71d5ff|Hspell:355:0|h[Taunt]|h|r",
            "true",
            "false",
            "true",
            "Salvation",
            "nil"
        ]
    );
}

/// Item data the engine estimates where nothing was recorded (AtlasLoot's
/// login and price buttons ask them): the subclasses the answers probe
/// skipped, and an icon for any item (the question mark's for item 0 too:
/// the full probe, newer than the estimate of none).
#[test]
fn unrecorded_item_classes_and_icons() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local a, b = C_Item.GetItemSubClassInfo(3, 0)
return a, tostring(b), (C_Item.GetItemSubClassInfo(5, 0)), (C_Item.GetItemSubClassInfo(14, 0)),
    C_Item.GetItemIconByID(23247), tostring((C_Item.GetItemIconByID(0)))",
    );
    assert_eq!(
        r,
        [
            "Jewelry(OBSOLETE)",
            "false",
            "Reagent",
            "Permanent",
            "134400",
            "134400"
        ]
    );
}

#[test]
fn items_arrive_after_a_request() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ITEM_DATA_LOAD_RESULT')
f:SetScript('OnEvent', function(_, e, id, ok) seen[#seen + 1] = id .. ':' .. tostring(ok) end)
mock.defineItem(6948, { name = 'Hearthstone', quality = 1, icon = 134414 }, { cached = true })
mock.defineItem(19019, { name = 'Thunderfury', quality = 5, itemLevel = 80 }, { delay = 0.5 })
mock.defineItem(404, {}, { missing = true })
local before = select('#', C_Item.GetItemInfo(19019))
C_Item.RequestLoadItemDataByID(19019)
C_Item.RequestLoadItemDataByID(404)
mock.advanceTime(0.3)
local early = table.concat(seen, ',')
mock.advanceTime(0.3)
local name, _, quality, level = C_Item.GetItemInfo(19019)
return C_Item.GetItemInfo('Hearthstone'), before, early, table.concat(seen, ','), name, quality, level,
    C_Item.GetItemInfoInstant('|cffffffff|Hitem:6948::|h[Hearthstone]|h|r'), tostring(C_Item.IsItemDataCachedByID(404))",
    );
    assert_eq!(
        r,
        [
            "Hearthstone",
            "0",
            "404:false",
            "404:false,19019:true",
            "Thunderfury",
            "5",
            "80",
            "6948",
            "false"
        ]
    );
}
