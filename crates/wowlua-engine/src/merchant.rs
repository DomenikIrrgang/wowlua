//! A vendor's window (spec Tooling §16.1.4): `mock.openMerchant(data)` and
//! `mock.closeMerchant()`, the merchant API, buying, selling (a bag item
//! used while the window is open, `mock.sell(bag, slot)` as the user's
//! right-click) and buying back.
//!
//! Verified against the client (`/stdtest engine record merchant` and
//! `/stdtest engine merchant`, Forever 70425): `MERCHANT_SHOW`,
//! `PLAYER_INTERACTION_MANAGER_FRAME_SHOW(5)`, then a frame later the items'
//! `MERCHANT_FILTER_ITEM_UPDATE(itemID)` and `MERCHANT_UPDATE`; closing:
//! `MERCHANT_CLOSED`, `PLAYER_INTERACTION_MANAGER_FRAME_HIDE(5)` and `(0)`;
//! the server's answers to a purchase, a sale and a buyback (their events
//! and order; a sale locks the item at once); `C_MerchantFrame.GetItemInfo`'s
//! fields, `numAvailable` -1 for unlimited, `GetMerchantItemCostInfo` 0,
//! `CanAffordMerchantItem` returning nothing, `GetBuybackItemInfo`'s seven
//! values, the closed window's answers.
//!
//! Unverified: the server's delay (0.4 s, as its other answers), a
//! purchase's price for stacks (the price per `stackCount`), which bag slot
//! a purchase fills (a stack of the item, else the first free slot), the
//! buyback list's order (the latest last) and size (12), repairs, and that
//! the filter update fires for every item.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::character::{self, Held};
use crate::{engine, engine_mut, units, widgets};

/// The server's answer to a purchase, sale or buyback.
const SERVER_DELAY: f64 = 0.4;

/// The buyback list's length.
const BUYBACK: usize = 12;

#[derive(Clone, Debug)]
pub struct Goods {
    pub id: i64,
    /// For `stack` items.
    pub price: f64,
    pub stack: f64,
    /// -1: unlimited.
    pub available: f64,
    pub max_stack: f64,
    pub usable: bool,
}

#[derive(Clone, Debug)]
pub struct Sold {
    pub held: Held,
    pub price: f64,
    /// Where it was.
    pub bag: i64,
    pub slot: usize,
}

#[derive(Default)]
pub struct Merchant {
    pub open: bool,
    pub items: Vec<Goods>,
    pub buyback: Vec<Sold>,
    pub can_repair: bool,
    pub repair_cost: f64,
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn fire(vm: &mut Vm, event: &str, args: &[Value]) {
    widgets::fire_event(vm, event, args);
}

fn merchant(vm: &Vm) -> &Merchant {
    &engine(vm).world.merchant
}

fn merchant_mut(vm: &mut Vm) -> &mut Merchant {
    &mut engine_mut(vm).world.merchant
}

/// `mock.openMerchant({ name, items = { { id, price, stack, available,
/// maxStack, usable } }, canRepair, repairCost })`.
pub fn control_open(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(data) = c.arg(1) else {
        return Err(vm.error("mock.openMerchant: a table of the vendor's data expected"));
    };
    let mut items = Vec::new();
    if let Value::Table(list) = field(vm, data, "items") {
        let n = vm.heap.table(list).border();
        for i in 1..=n {
            let Value::Table(t) = vm.heap.table(list).get(Value::Num(i as f64)) else {
                continue;
            };
            let Some(id) = num(field(vm, t, "id")) else {
                return Err(vm.error("mock.openMerchant: an item needs its id"));
            };
            let stack = num(field(vm, t, "stack")).unwrap_or(1.0);
            let max_stack = num(field(vm, t, "maxStack"))
                .or_else(|| num(character::item_field(vm, id as i64, "stackCount")))
                .unwrap_or(1.0);
            let usable = field(vm, t, "usable");
            items.push(Goods {
                id: id as i64,
                price: num(field(vm, t, "price")).unwrap_or(0.0),
                stack,
                available: num(field(vm, t, "available")).unwrap_or(-1.0),
                max_stack,
                usable: usable.is_nil() || usable.truthy(),
            });
        }
    }
    let name = match field(vm, data, "name") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "Merchant".into(),
    };
    let can_repair = field(vm, data, "canRepair").truthy();
    let repair_cost = num(field(vm, data, "repairCost")).unwrap_or(0.0);
    if merchant(vm).open {
        close(vm);
    }
    let npc = units::spawn_named(vm, &name);
    engine_mut(vm).units.npc = Some(npc);
    {
        let m = merchant_mut(vm);
        m.open = true;
        m.items = items;
        m.can_repair = can_repair;
        m.repair_cost = repair_cost;
    }
    fire(vm, "MERCHANT_SHOW", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(5.0)],
    );
    let later = vm.native("merchantFilled", filled);
    crate::clock::after_engine(vm, 1.0 / 60.0, later);
    Ok(vec![])
}

/// A frame after opening: the items' filter updates, `MERCHANT_UPDATE`.
fn filled(vm: &mut Vm, _c: NativeCall) -> Ret {
    if !merchant(vm).open {
        return Ok(vec![]);
    }
    let ids: Vec<i64> = merchant(vm).items.iter().map(|g| g.id).collect();
    for id in ids {
        fire(vm, "MERCHANT_FILTER_ITEM_UPDATE", &[Value::Num(id as f64)]);
    }
    fire(vm, "MERCHANT_UPDATE", &[]);
    Ok(vec![])
}

fn close(vm: &mut Vm) {
    if !merchant(vm).open {
        return;
    }
    merchant_mut(vm).open = false;
    engine_mut(vm).units.npc = None;
    fire(vm, "MERCHANT_CLOSED", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(5.0)],
    );
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(0.0)],
    );
}

/// `mock.closeMerchant()`, `CloseMerchant()`.
pub fn control_close(vm: &mut Vm, _c: NativeCall) -> Ret {
    close(vm);
    Ok(vec![])
}

/// The merchant's item `i` (1-based) while open.
fn goods(vm: &Vm, c: &NativeCall) -> Option<Goods> {
    let m = merchant(vm);
    if !m.open {
        return None;
    }
    let i = num(c.arg(1))? as usize;
    m.items.get(i.checked_sub(1)?).cloned()
}

fn num_items(vm: &mut Vm, _c: NativeCall) -> Ret {
    let m = merchant(vm);
    let n = if m.open { m.items.len() } else { 0 };
    Ok(vec![Value::Num(n as f64)])
}

/// `C_MerchantFrame.GetItemInfo(i)`.
fn item_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(g) = goods(vm, &c) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    let name = character::item_field(vm, g.id, "name");
    let icon = character::item_field(vm, g.id, "icon");
    let fields = [
        ("name", name),
        ("texture", icon),
        ("price", Value::Num(g.price)),
        ("stackCount", Value::Num(g.stack)),
        ("numAvailable", Value::Num(g.available)),
        ("isPurchasable", Value::Bool(true)),
        ("isUsable", Value::Bool(g.usable)),
        ("hasExtendedCost", Value::Bool(false)),
        ("isQuestStartItem", Value::Bool(false)),
    ];
    for (k, v) in fields {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, v);
    }
    Ok(vec![Value::Table(t)])
}

fn item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(g) = goods(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(character::link_of_id(vm, g.id).into_iter().collect())
}

fn item_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(goods(vm, &c)
        .map(|g| vec![Value::Num(g.id as f64)])
        .unwrap_or_default())
}

fn item_max_stack(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(goods(vm, &c)
        .map(|g| vec![Value::Num(g.max_stack)])
        .unwrap_or_default())
}

/// `GetMerchantItemCostInfo(i)`: no extended costs.
fn cost_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(goods(vm, &c)
        .map(|_| vec![Value::Num(0.0)])
        .unwrap_or_default())
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn yes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `GetMerchantFilter()`: 2 (recorded open and closed).
fn filter(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(2.0)])
}

fn can_repair(vm: &mut Vm, _c: NativeCall) -> Ret {
    let m = merchant(vm);
    Ok(vec![Value::Bool(m.open && m.can_repair)])
}

fn repair_all_cost(vm: &mut Vm, _c: NativeCall) -> Ret {
    let m = merchant(vm);
    let cost = if m.open && m.can_repair {
        m.repair_cost
    } else {
        0.0
    };
    Ok(vec![Value::Num(cost)])
}

/// `RepairAllItems()`: the cost paid (`PLAYER_MONEY`; unverified).
fn repair_all(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (open, can, cost) = {
        let m = merchant(vm);
        (m.open, m.can_repair, m.repair_cost)
    };
    if !open || !can || cost <= 0.0 || engine(vm).world.character.money < cost {
        return Ok(vec![]);
    }
    engine_mut(vm).world.character.money -= cost;
    merchant_mut(vm).repair_cost = 0.0;
    fire(vm, "PLAYER_MONEY", &[]);
    Ok(vec![])
}

fn sell_all_junk_enabled(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(merchant(vm).open)])
}

/// `C_MerchantFrame.GetNumJunkItems()`: poor items in the bags.
fn num_junk(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ids: Vec<i64> = engine(vm)
        .world
        .character
        .bags
        .values()
        .flat_map(|b| b.items.values().map(|h| h.id))
        .collect();
    let n = ids
        .into_iter()
        .filter(|&id| num(character::item_field(vm, id, "quality")) == Some(0.0))
        .count();
    Ok(vec![Value::Num(n as f64)])
}

fn empty_table(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

// --------------------------------------------------------------- buyback

fn sold(vm: &Vm, c: &NativeCall) -> Option<Sold> {
    let i = num(c.arg(1))? as usize;
    merchant(vm).buyback.get(i.checked_sub(1)?).cloned()
}

fn num_buyback(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(merchant(vm).buyback.len() as f64)])
}

/// `GetBuybackItemInfo(i)`: name, texture, price, quantity, numAvailable,
/// isUsable, isBound.
fn buyback_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(s) = sold(vm, &c) else {
        return Ok(vec![]);
    };
    let name = character::item_field(vm, s.held.id, "name");
    let icon = character::item_field(vm, s.held.id, "icon");
    Ok(vec![
        name,
        icon,
        Value::Num(s.price),
        Value::Num(s.held.count),
        Value::Num(0.0),
        Value::Bool(true),
        Value::Bool(s.held.bound),
    ])
}

fn buyback_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(s) = sold(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(character::link_of(vm, &s.held).into_iter().collect())
}

fn buyback_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(sold(vm, &c)
        .map(|s| vec![Value::Num(s.held.id as f64)])
        .unwrap_or_default())
}

// ---------------------------------------------------------- the server

/// Runs `answer` when the server answers (`SERVER_DELAY`).
fn later(vm: &mut Vm, name: &'static str, answer: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, answer, upvals);
    crate::clock::after_engine(vm, SERVER_DELAY, native);
}

/// `BuyMerchantItem(i [, quantity])`.
fn buy(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(g) = goods(vm, &c) else {
        return Ok(vec![]);
    };
    let quantity = num(c.arg(2)).unwrap_or(g.stack).max(1.0);
    let cost = (g.price * quantity / g.stack.max(1.0)).round();
    if engine(vm).world.character.money < cost {
        return Ok(vec![]);
    }
    later(
        vm,
        "merchantBought",
        bought,
        vec![
            Value::Num(g.id as f64),
            Value::Num(quantity),
            Value::Num(cost),
        ],
    );
    Ok(vec![])
}

fn upval(c: &NativeCall, i: usize) -> f64 {
    match c.upvals.get(i) {
        Some(Value::Num(n)) => *n,
        _ => 0.0,
    }
}

/// Where `count` of item `id` go: a stack of it, else the first free slot.
fn place(vm: &Vm, id: i64) -> Option<(i64, usize)> {
    let bags = &engine(vm).world.character.bags;
    for (&bag, b) in bags {
        if let Some((&slot, _)) = b.items.iter().find(|(_, h)| h.id == id) {
            return Some((bag, slot));
        }
    }
    for (&bag, b) in bags {
        if let Some(slot) = (1..=b.slots).find(|s| !b.items.contains_key(s)) {
            return Some((bag, slot));
        }
    }
    None
}

fn bag_update(vm: &mut Vm, bag: i64) {
    fire(vm, "BAG_UPDATE", &[Value::Num(bag as f64)]);
    if bag == 0 {
        fire(vm, "BAG_UPDATE", &[Value::Num(-1.0)]);
    }
}

/// The purchase, as the server answers it (`/stdtest engine record
/// merchant`).
fn bought(vm: &mut Vm, c: NativeCall) -> Ret {
    let (id, quantity, cost) = (upval(&c, 0) as i64, upval(&c, 1), upval(&c, 2));
    let Some((bag, slot)) = place(vm, id) else {
        let message = vm.str("Inventory is full.");
        fire(vm, "UI_ERROR_MESSAGE", &[Value::Num(3.0), message]);
        return Ok(vec![]);
    };
    {
        let w = &mut engine_mut(vm).world;
        w.character.money -= cost;
        let items = &mut w.character.bags.get_mut(&bag).expect("a bag").items;
        match items.get_mut(&slot) {
            Some(h) => h.count += quantity,
            None => {
                items.insert(
                    slot,
                    Held {
                        id,
                        count: quantity,
                        link: None,
                        bound: false,
                        locked: false,
                    },
                );
            }
        }
    }
    let icon = character::item_field(vm, id, "icon");
    fire(vm, "MERCHANT_UPDATE", &[]);
    fire(vm, "ITEM_PUSH", &[Value::Num(bag as f64), icon]);
    fire(vm, "BAG_NEW_ITEMS_UPDATED", &[]);
    bag_update(vm, bag);
    let player = vm.str("player");
    fire(vm, "UNIT_INVENTORY_CHANGED", &[player]);
    fire(vm, "PLAYER_MONEY", &[]);
    bag_update(vm, bag);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    Ok(vec![])
}

/// Sells the item in `bag`, `slot` to the open merchant: locked at once,
/// sold when the server answers. False when nothing is sold.
pub fn sell(vm: &mut Vm, bag: i64, slot: usize) -> bool {
    if !merchant(vm).open {
        return false;
    }
    let Some(held) = engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .and_then(|b| b.items.get(&slot))
        .cloned()
    else {
        return false;
    };
    if held.locked {
        return false;
    }
    set_locked(vm, bag, slot, true);
    let at = [Value::Num(bag as f64), Value::Num(slot as f64)];
    fire(vm, "ITEM_LOCK_CHANGED", &at);
    fire(vm, "ITEM_LOCKED", &at);
    later(
        vm,
        "merchantSold",
        sold_answer,
        vec![Value::Num(bag as f64), Value::Num(slot as f64)],
    );
    true
}

fn set_locked(vm: &mut Vm, bag: i64, slot: usize, locked: bool) {
    if let Some(h) = engine_mut(vm)
        .world
        .character
        .bags
        .get_mut(&bag)
        .and_then(|b| b.items.get_mut(&slot))
    {
        h.locked = locked;
    }
}

fn sold_answer(vm: &mut Vm, c: NativeCall) -> Ret {
    let (bag, slot) = (upval(&c, 0) as i64, upval(&c, 1) as usize);
    let Some(mut held) = engine_mut(vm)
        .world
        .character
        .bags
        .get_mut(&bag)
        .and_then(|b| b.items.remove(&slot))
    else {
        return Ok(vec![]);
    };
    let each = num(character::item_field(vm, held.id, "sellPrice")).unwrap_or(0.0);
    let price = each * held.count;
    held.locked = false;
    let id = held.id;
    {
        let w = &mut engine_mut(vm).world;
        w.character.money += price;
        let list = &mut w.merchant.buyback;
        list.push(Sold {
            held,
            price,
            bag,
            slot,
        });
        if list.len() > BUYBACK {
            list.remove(0);
        }
    }
    fire(vm, "PLAYER_MONEY", &[]);
    bag_update(vm, bag);
    fire(vm, "MERCHANT_UPDATE", &[]);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    Ok(vec![])
}

/// `mock.sell(bag, slot)`: the user's right-click on a bag item at a
/// vendor.
pub fn control_sell(vm: &mut Vm, c: NativeCall) -> Ret {
    let bag = vm.check_int(&c, 1)?;
    let slot = vm.check_int(&c, 2)?;
    if !merchant(vm).open {
        return Err(vm.error("mock.sell: no merchant is open (mock.openMerchant)"));
    }
    crate::input::as_user(vm, |vm| sell(vm, bag, slot as usize));
    Ok(vec![])
}

/// `C_Container.UseContainerItem(bag, slot)` at a vendor (Blizzard's bag
/// buttons): the sale; elsewhere nothing (unverified).
fn use_container_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(bag), Some(slot)) = (num(c.arg(1)), num(c.arg(2))) else {
        return Ok(vec![]);
    };
    sell(vm, bag as i64, slot as usize);
    Ok(vec![])
}

/// `BuybackItem(i)`.
fn buyback(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(s) = sold(vm, &c) else {
        return Ok(vec![]);
    };
    if !merchant(vm).open || engine(vm).world.character.money < s.price {
        return Ok(vec![]);
    }
    let i = num(c.arg(1)).unwrap_or(1.0);
    later(vm, "merchantBoughtBack", bought_back, vec![Value::Num(i)]);
    Ok(vec![])
}

/// The buyback, as the server answers it: back where it was when free.
fn bought_back(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = upval(&c, 0) as usize;
    let list = &mut merchant_mut(vm).buyback;
    if i == 0 || i > list.len() {
        return Ok(vec![]);
    }
    let s = list.remove(i - 1);
    let free = engine(vm)
        .world
        .character
        .bags
        .get(&s.bag)
        .is_some_and(|b| !b.items.contains_key(&s.slot));
    let Some((bag, slot)) = (if free {
        Some((s.bag, s.slot))
    } else {
        place(vm, s.held.id)
    }) else {
        merchant_mut(vm).buyback.insert(i - 1, s);
        return Ok(vec![]);
    };
    let id = s.held.id;
    {
        let w = &mut engine_mut(vm).world;
        w.character.money -= s.price;
        if let Some(b) = w.character.bags.get_mut(&bag) {
            b.items.insert(slot, s.held);
        }
    }
    fire(vm, "PLAYER_MONEY", &[]);
    let at = [Value::Num(bag as f64), Value::Num(slot as f64)];
    fire(vm, "ITEM_LOCK_CHANGED", &at);
    fire(vm, "ITEM_UNLOCKED", &at);
    bag_update(vm, bag);
    fire(vm, "MERCHANT_UPDATE", &[]);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetMerchantNumItems", num_items),
        ("C_MerchantFrame.GetItemInfo", item_info),
        ("GetMerchantItemLink", item_link),
        ("GetMerchantItemID", item_id),
        ("GetMerchantItemMaxStack", item_max_stack),
        ("GetMerchantItemCostInfo", cost_info),
        ("CanAffordMerchantItem", nothing),
        ("C_MerchantFrame.IsMerchantItemRefundable", no),
        ("GetMerchantFilter", filter),
        ("CanMerchantRepair", can_repair),
        ("GetRepairAllCost", repair_all_cost),
        ("RepairAllItems", repair_all),
        ("CanGuildBankRepair", yes),
        ("InRepairMode", no),
        (
            "C_MerchantFrame.IsSellAllJunkEnabled",
            sell_all_junk_enabled,
        ),
        ("C_MerchantFrame.GetNumJunkItems", num_junk),
        ("C_MerchantFrame.GetMerchantCurrencies", empty_table),
        ("GetNumBuybackItems", num_buyback),
        ("GetBuybackItemInfo", buyback_info),
        ("GetBuybackItemLink", buyback_link),
        ("C_MerchantFrame.GetBuybackItemID", buyback_id),
        ("BuyMerchantItem", buy),
        ("BuybackItem", buyback),
        ("CloseMerchant", control_close),
        ("C_Container.UseContainerItem", use_container_item),
        ("UseContainerItem", use_container_item),
    ]
}
