//! Spells and items the test defines, read through C_Spell and C_Item.

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
    cd.duration, tostring(cd.isEnabled), tostring((C_Spell.GetSpellInfo(999))), tostring(C_Spell.IsSpellDataCached(133))",
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
            "|cff71d5ff|Hspell:133|h[Fireball]|h|r",
            "true",
            "false",
            "0",
            "true",
            "nil",
            "true"
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
