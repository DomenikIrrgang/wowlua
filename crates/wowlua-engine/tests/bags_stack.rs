//! An item put onto a part stack of its kind (`cursor.rs`, decision T256).

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

const SETUP: &str = "mock.defineItem(2770, { name = 'Copper Ore', stackCount = 20 }, { cached = true })
mock.defineItem(2589, { name = 'Linen Cloth', stackCount = 20 }, { cached = true })
mock.setBag(1, { slots = 6 })
local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'ITEM_LOCK_CHANGED', 'BAG_UPDATE', 'BAG_UPDATE_DELAYED' }) do f:RegisterEvent(e) end
f:SetScript('OnEvent', function(_, e, a, b) seen[#seen + 1] = e .. (a and (' ' .. a) or '') .. (b and (' ' .. b) or '') end)
local C = C_Container
local function count(bag, slot)
    local info = C.GetContainerItemInfo(bag, slot)
    return info and (info.itemID .. 'x' .. info.stackCount) or 'empty'
end
";

#[test]
fn a_stack_put_onto_a_part_stack_of_its_kind_stacks_onto_it() {
    let mut vm = vm();
    let src = format!(
        "{SETUP}
mock.setBag(5, {{ slots = 4, items = {{ [2] = {{ id = 2770, count = 10 }} }} }})
mock.setBag(1, {{ slots = 6, items = {{ [1] = {{ id = 2770, count = 4 }} }} }})
seen = {{}}
C.PickupContainerItem(5, 2)
C.PickupContainerItem(1, 1)
local cursor = tostring((GetCursorInfo()))
local locked = tostring(C.GetContainerItemInfo(1, 1).isLocked)
mock.advanceTime(1)
return cursor, locked, count(5, 2), count(1, 1), tostring(C.GetContainerItemInfo(1, 1).isLocked), table.concat(seen, ', ')"
    );
    assert_eq!(
        run(&mut vm, &src),
        [
            "nil",
            "true",
            "empty",
            "2770x14",
            "false",
            "ITEM_LOCK_CHANGED 5 2, ITEM_LOCK_CHANGED 1 1, BAG_UPDATE 5, ITEM_LOCK_CHANGED 1 1, BAG_UPDATE 1, BAG_UPDATE_DELAYED"
        ]
    );
}

#[test]
fn what_doesnt_fit_stays_behind_and_a_split_part_stacks_too() {
    let mut vm = vm();
    let src = format!(
        "{SETUP}
mock.setBag(1, {{ slots = 6, items = {{ [1] = {{ id = 2770, count = 15 }}, [2] = {{ id = 2770, count = 12 }}, [3] = {{ id = 2770, count = 20 }}, [4] = {{ id = 2589, count = 3 }} }} }})
C.PickupContainerItem(1, 2)
C.PickupContainerItem(1, 1)
mock.advanceTime(1)
local over = count(1, 1) .. ' ' .. count(1, 2)
C.SplitContainerItem(1, 2, 3)
C.PickupContainerItem(1, 4)
local other = tostring(GetCursorInfo())
ClearCursor()
C.SplitContainerItem(1, 2, 3)
C.PickupContainerItem(1, 3)
local full = tostring(GetCursorInfo())
ClearCursor()
mock.advanceTime(1)
return over, other, full, count(1, 2), count(1, 3), count(1, 4)"
    );
    assert_eq!(
        run(&mut vm, &src),
        [
            "2770x20 2770x7",
            "item",
            "item",
            "2770x7",
            "2770x20",
            "2589x3"
        ]
    );
    // A split part onto a part stack: stacked onto it.
    let r = run(
        &mut vm,
        "local C = C_Container
C.SplitContainerItem(1, 2, 3)
C.PickupContainerItem(1, 5)
mock.advanceTime(1)
C.SplitContainerItem(1, 2, 2)
C.PickupContainerItem(1, 5)
mock.advanceTime(1)
return C.GetContainerItemInfo(1, 2).stackCount, C.GetContainerItemInfo(1, 5).stackCount",
    );
    assert_eq!(r, ["2", "5"]);
    // A part stack onto a full one of its own: they swap places (seen in
    // game).
    let r = run(
        &mut vm,
        "local C = C_Container
C.PickupContainerItem(1, 2)
C.PickupContainerItem(1, 3)
local held = tostring((GetCursorInfo()))
mock.advanceTime(1)
return held, C.GetContainerItemInfo(1, 2).stackCount, C.GetContainerItemInfo(1, 3).stackCount",
    );
    assert_eq!(r, ["nil", "20", "2"]);
}
