//! The client's actions on modelled state (`actions2.rs`): the cursor's
//! item into a bag, a letter's money taken and the letter deleted, raid
//! members moved between subgroups.

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
fn the_cursors_item_goes_into_a_bag() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.setBag(0, { slots = 2, items = { [1] = 6948 } })
mock.setBag(1, { slots = 2, items = {} })
local empty = PutItemInBag(31)
C_Container.PickupContainerItem(0, 1)
return empty, PutItemInBag(31), CursorHasItem()",
    );
    assert_eq!(r, ["false", "true", "false"]);
    wowlua_engine::clock::advance(&mut vm, 1.0);
    let r = run(
        &mut vm,
        "return (C_Container.GetContainerItemID(1, 1)), (C_Container.GetContainerItemID(0, 1))",
    );
    assert_eq!(r, ["6948", "nil"]);
}

#[test]
fn a_letters_money_is_taken_and_a_letter_deleted() {
    let mut vm = vm();
    run(
        &mut vm,
        "local box = mock.spawnUnit({ name = 'Mailbox' })
mock.defineMailbox(box, { letters = {
    { sender = 'Auction House', subject = 'Sold', money = 500, auction = true },
    { sender = 'Someone', subject = 'Hi', textCreated = true },
} })
mock.player:setMoney(100)
TakeInboxMoney(1)",
    );
    // The money the player's as the emptied letter goes (recorded:
    // `mail2`), not with the take's answer.
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(&mut vm, "return GetMoney(), (GetInboxNumItems())");
    assert_eq!(r, ["100", "2"]);
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(
        &mut vm,
        "local n = GetInboxNumItems() DeleteInboxItem(1) return GetMoney(), n",
    );
    assert_eq!(r, ["600", "1"]);
    wowlua_engine::clock::advance(&mut vm, 0.5);
    let r = run(&mut vm, "return (GetInboxNumItems())");
    assert_eq!(r, ["0"]);
}

#[test]
fn raid_members_move_between_subgroups() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local a = mock.spawnUnit({ name = 'Aa', player = true })
local b = mock.spawnUnit({ name = 'Bb', player = true })
mock.addToGroup(a, true)
mock.addToGroup(b, true)
SetRaidSubgroup(2, 3)
local moved = select(3, GetRaidRosterInfo(2))
SwapRaidSubgroup(2, 3)
return moved, select(3, GetRaidRosterInfo(2)), (select(3, GetRaidRosterInfo(3)))",
    );
    assert_eq!(r, ["3", "1", "3"]);
}

/// A take with full bags is refused: "Inventory is full." at once,
/// `MAIL_FAILED` with the item a round trip later, the item still in the
/// letter; a COD letter's item is paid for as it is taken (issue #265,
/// decision T661).
#[test]
fn mail_takes_need_room_and_pay_cod() {
    let mut vm = vm();
    run(
        &mut vm,
        "mock.defineItem(2589, { name = 'Linen Cloth' })
mock.defineItem(159, { name = 'Refreshing Spring Water' })
mock.setBag(0, { slots = 1, items = { [1] = 159 } })
for bag = 1, 4 do mock.setBag(bag, { slots = 0 }) end
mock.player:setMoney(10000)
local box = mock.spawnUnit({ name = 'Mailbox' })
mock.defineMailbox(box, { letters = {
    { sender = 'Someone', subject = 'full bags', items = { { id = 2589, count = 1 } } },
    { sender = 'Someone', subject = 'cod', cod = 500, items = { { id = 2589, count = 1 } } },
} })
SEEN = {}
local f = CreateFrame('Frame')
f:RegisterEvent('MAIL_FAILED')
f:RegisterEvent('UI_ERROR_MESSAGE')
f:SetScript('OnEvent', function(_, e, a, b) SEEN[#SEEN + 1] = e .. ' ' .. tostring(e == 'UI_ERROR_MESSAGE' and b or a) end)
TakeInboxItem(1, 1)
NOW = #SEEN",
    );
    wowlua_engine::clock::advance(&mut vm, 1.0);
    let r = run(
        &mut vm,
        "local seen = table.concat(SEEN, '; ')
local still = GetInboxItem(1, 1)
mock.setBag(1, { slots = 4 })
TakeInboxItem(2, 1)
return NOW, seen, still, GetMoney()",
    );
    assert_eq!(
        r,
        [
            "1",
            "UI_ERROR_MESSAGE Inventory is full.; MAIL_FAILED 2589",
            "Linen Cloth",
            "10000"
        ]
    );
    wowlua_engine::clock::advance(&mut vm, 1.0);
    let r = run(
        &mut vm,
        "local _, _, _, _, _, cod = GetInboxHeaderInfo(2) return GetMoney(), cod, C_Container.GetContainerItemID(1, 1)",
    );
    assert_eq!(r, ["9500", "0", "2589"]);
}
