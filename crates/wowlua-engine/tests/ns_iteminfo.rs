//! Items' answers from the bags, the gear and the item data (`iteminfo`).

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
fn quality_colors_and_armor() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local _, _, _, hex = C_Item.GetItemQualityColor(3)
local eff = C_PaperDollInfo.GetArmorEffectiveness(1149, 0)
return hex, math.floor(eff * 10000), C_PaperDollInfo.GetArmorEffectiveness(0, 0)",
    );
    assert_eq!(r, ["ff0070dd", "7031", "0"]);
}

#[test]
fn bag_items_by_location_and_guid() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineItem(90001, { name = 'Test Helm', quality = 2, itemLevel = 42, equipLoc = 'INVTYPE_HEAD',
    classID = 4, subclassID = 2 }, { cached = true })
mock.setBag(0, { slots = 4, items = { [2] = 90001 } })
local loc = { bagID = 0, slotIndex = 2 }
local guid = C_Item.GetItemGUID(loc)
local level = C_Item.GetDetailedItemLevelInfo(90001)
return C_Item.GetItemIDByGUID(guid), level, C_Item.GetCurrentItemLevel(loc),
    #C_Container.GetContainerFreeSlots(0), C_Item.IsEquippableItem(90001),
    C_Item.GetItemInventoryTypeByID(90001), C_Container.HasContainerItem(0, 1),
    C_PaperDollInfo.OffhandHasWeapon()",
    );
    assert_eq!(r, ["90001", "42", "42", "3", "true", "1", "false", "false"]);
}
