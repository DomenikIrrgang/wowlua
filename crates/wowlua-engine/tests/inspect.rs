//! Inspecting another player: once the inspect is in, the inspected unit's
//! gear answers `GetInventoryItemID`, `…Texture` and `…Quality` as its
//! link does (SuuUI's inspect window draws its slots from them).

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
fn an_inspected_players_gear_has_its_id_icon_and_quality() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineItem(19019, { name = 'Thunderfury', quality = 5, icon = 135349 })
mock.defineItem(5107, { name = 'Deckhand\\'s Shirt', quality = 1, icon = 135006 })
local other = mock.spawnUnit({ name = 'Otherplayer', player = true, level = 60, reaction = 'friendly' })
other:setDistance(3)
other:setInspect({ itemLevel = 70, items = {
    [16] = { link = '|cnIQ5:|Hitem:19019::::::::60:::::::::|h[Thunderfury]|h|r', cached = true },
    [4] = { link = '|cnIQ1:|Hitem:5107::::::::60:::::::::|h[Deckhand\\'s Shirt]|h|r' },
} })
mock.player:target(other)
NotifyInspect('target')
mock.advanceTime(2)
return GetInventoryItemID('target', 16), GetInventoryItemTexture('target', 16), GetInventoryItemQuality('target', 16),
    GetInventoryItemID('target', 4), GetInventoryItemQuality('target', 4),
    tostring(GetInventoryItemID('target', 1) == nil), tostring(GetInventoryItemTexture('target', 1) == nil)",
    );
    assert_eq!(r, ["19019", "135349", "5", "5107", "1", "true", "true"]);
}

/// `GetText(token, gender)`: the global string by its name, a woman's form
/// for gender 3 when there is one; nil for a name that is no string (the
/// inspect window's PvP rank).
#[test]
fn get_text_reads_the_global_strings() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "RANK_TEST, RANK_TEST_FEMALE, PLAIN_TEST = 'Sergeant', 'Sergeant (f)', 'Plain'
return GetText('RANK_TEST'), GetText('RANK_TEST', 2), GetText('RANK_TEST', 3), GetText('PLAIN_TEST', 3),
    tostring(GetText('NO_SUCH_TEST')), tostring(GetText('GetTime')), PVP_RANK_7_0 == GetText('PVP_RANK_7_0')",
    );
    assert_eq!(
        r,
        [
            "Sergeant",
            "Sergeant",
            "Sergeant (f)",
            "Plain",
            "nil",
            "nil",
            "true"
        ]
    );
}
