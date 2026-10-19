//! A vendor's window (spec Tooling §16.1.4): `mock.openMerchant(data)` and
//! `mock.closeMerchant()`, the merchant API, buying, selling (a bag item
//! used while the window is open, `mock.sell(bag, slot)` as the user's
//! right-click) and buying back.
//!
//! Verified against the client (`/stdtest engine record merchant` and
//! `/stdtest engine merchant`, Forever 70425): `MERCHANT_SHOW`,
//! `PLAYER_INTERACTION_MANAGER_FRAME_SHOW(5)`, then a frame later the
//! unloaded items' `MERCHANT_FILTER_ITEM_UPDATE(itemID)` and
//! `MERCHANT_UPDATE` (none for loaded items: the vendor scenario); closing:
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
//! buyback list's order (the latest last) and size (12), and that the
//! filter update fires for every unloaded item. The client's answer to a purchase
//! orders `PLAYER_MONEY` and the bag's update differently from one time to
//! the next (the server's packets): the first recording's order when the
//! item's data come at once, `/stdtest engine scenario vendor`'s when they
//! come a packet later (the goods' `itemAfter`).
//! Recorded too (`/stdtest engine record vendor2`): `ITEM_PUSH` with the
//! bag's inventory slot, `CHAT_MSG_LOOT`, "Inventory is full." (3), a
//! repair's `UPDATE_INVENTORY_DURABILITY` per item and `PLAYER_MONEY`
//! (`repairMoneyAfter`).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{LuaError, NativeCall, NativeFn, TableRef, Value, Vm};

use crate::character::{self, Held};
use crate::{engine, engine_mut, units, widgets};

// The server answers a purchase, sale or buyback a round trip later
// (`clock::server_delay`).

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
    /// A bought one's data after the purchase's answer (seconds), the
    /// server's (`itemAfter`).
    pub item_after: Option<f64>,
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
    /// The damaged items a repair mends (one `UPDATE_INVENTORY_DURABILITY`
    /// each).
    pub damaged: f64,
    /// A repair's money told after this many items' durability.
    pub repair_money_after: Option<usize>,
    /// The chat's line IDs (`CHAT_MSG_LOOT`).
    pub line: f64,
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
    open(vm, data, None)?;
    Ok(vec![])
}

/// A vendor's window opened (`openMerchant`'s table): by `npc` (a gossip's
/// option), else by an NPC of its name.
pub fn open(vm: &mut Vm, data: TableRef, npc: Option<units::UnitId>) -> Result<(), LuaError> {
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
                item_after: num(field(vm, t, "itemAfter")),
            });
        }
    }
    let name = match field(vm, data, "name") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "Merchant".into(),
    };
    let can_repair = field(vm, data, "canRepair").truthy();
    let repair_cost = num(field(vm, data, "repairCost")).unwrap_or(0.0);
    let damaged =
        num(field(vm, data, "damaged")).unwrap_or(if repair_cost > 0.0 { 1.0 } else { 0.0 });
    let money_after = num(field(vm, data, "repairMoneyAfter")).map(|n| n.max(0.0) as usize);
    if merchant(vm).open {
        close(vm);
    }
    let npc = match npc {
        Some(n) => n,
        None => units::spawn_named(vm, &name),
    };
    engine_mut(vm).units.npc = Some(npc);
    {
        let m = merchant_mut(vm);
        m.open = true;
        m.items = items;
        m.can_repair = can_repair;
        m.repair_cost = repair_cost;
        m.damaged = damaged;
        m.repair_money_after = money_after;
    }
    fire(vm, "MERCHANT_SHOW", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(5.0)],
    );
    // The goods' data loading (a first visit, `gossip2`): their filter
    // updates a frame later; `cached = true`, the client had them (a visit
    // before, `vendor`: none, no item's data loaded).
    if !field(vm, data, "cached").truthy() {
        let later = vm.native("merchantFilled", filled);
        crate::clock::after_engine(vm, 1.0 / 60.0, later);
    }
    Ok(())
}

/// A frame after opening: the filter updates of the goods whose data was
/// not loaded, with `MERCHANT_UPDATE` before the last of them. Recorded:
/// gossip2's twelve unloaded goods (their links empty) each fired one, the
/// update between the eleventh and the twelfth; vendor's eight loaded
/// goods fired none and no update. Unverified (one sample each): where the
/// update falls for other counts.
fn filled(vm: &mut Vm, _c: NativeCall) -> Ret {
    if !merchant(vm).open {
        return Ok(vec![]);
    }
    let ids: Vec<i64> = merchant(vm).items.iter().map(|g| g.id).collect();
    let loading: Vec<i64> = ids
        .into_iter()
        .filter(|&id| character::item_field(vm, id, "name").is_nil())
        .collect();
    let Some((&last, rest)) = loading.split_last() else {
        return Ok(vec![]);
    };
    for &id in rest {
        fire(vm, "MERCHANT_FILTER_ITEM_UPDATE", &[Value::Num(id as f64)]);
    }
    fire(vm, "MERCHANT_UPDATE", &[]);
    fire(
        vm,
        "MERCHANT_FILTER_ITEM_UPDATE",
        &[Value::Num(last as f64)],
    );
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

/// A merchant's goods' links: in the vendor's context (14, recorded:
/// `/stdtest engine scenario vendor`).
/// `GetMerchantItemLink(i)`: the goods' link in the vendor's context; none
/// for an item whose data is not loaded, no name (recorded: gossip2's
/// goods).
fn item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(g) = goods(vm, &c) else {
        return Ok(vec![]);
    };
    if character::item_field(vm, g.id, "name").is_nil() {
        return Ok(vec![]);
    }
    let link = character::link_in_context(vm, g.id, "14", &[]);
    Ok(link.map(|l| vec![vm.str(&l)]).unwrap_or_default())
}

/// `C_MerchantFrame.IsMerchantItemRefundable(i)`: a merchant's goods are
/// (inferred: the recorded purchase asked no confirmation, which Blizzard's
/// frame shows for a non-refundable item; unrecorded).
fn refundable(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(goods(vm, &c).is_some())])
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

/// `GetRepairAllCost()`: at a vendor that repairs, the cost and whether
/// anything needs it; elsewhere 0 alone (`/stdtest engine merchant`).
fn repair_all_cost(vm: &mut Vm, _c: NativeCall) -> Ret {
    let m = merchant(vm);
    if m.open && m.can_repair {
        Ok(vec![
            Value::Num(m.repair_cost),
            Value::Bool(m.repair_cost > 0.0),
        ])
    } else {
        Ok(vec![Value::Num(0.0)])
    }
}

/// `RepairAllItems()`: a round trip later (recorded: `vendor`), every item
/// mended, `UPDATE_INVENTORY_DURABILITY` for each damaged one and
/// `PLAYER_MONEY` among them after `repairMoneyAfter` of them (the
/// merchant's data, the server's packets: recorded after all four in
/// `/stdtest engine record vendor2`, after the first in `vendor`; by
/// default after all).
fn repair_all(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (open, can, cost) = {
        let m = merchant(vm);
        (m.open, m.can_repair, m.repair_cost)
    };
    if !open || !can || cost <= 0.0 || engine(vm).world.character.money < cost {
        return Ok(vec![]);
    }
    later(vm, "merchantRepaired", repaired, vec![]);
    Ok(vec![])
}

fn repaired(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (open, cost) = {
        let m = merchant(vm);
        (m.open, m.repair_cost)
    };
    if !open || cost <= 0.0 || engine(vm).world.character.money < cost {
        return Ok(vec![]);
    }
    engine_mut(vm).world.character.money -= cost;
    // The worn items mended (their durability, where known; else the
    // merchant's count).
    let worn = character::repair_worn(vm);
    let damaged = if worn > 0 {
        worn as f64
    } else {
        merchant(vm).damaged
    };
    merchant_mut(vm).repair_cost = 0.0;
    merchant_mut(vm).damaged = 0.0;
    let damaged = damaged as usize;
    let money_after = merchant(vm).repair_money_after.unwrap_or(damaged);
    for i in 0..damaged {
        if i == money_after {
            fire(vm, "PLAYER_MONEY", &[]);
        }
        fire(vm, "UPDATE_INVENTORY_DURABILITY", &[]);
    }
    if money_after >= damaged {
        fire(vm, "PLAYER_MONEY", &[]);
    }
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

/// Runs `answer` when the server answers (`clock::server_delay`).
fn later(vm: &mut Vm, name: &'static str, answer: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, answer, upvals);
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, native);
}

/// `BuyMerchantItem(i [, quantity])`.
fn buy(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(g) = goods(vm, &c) else {
        return Ok(vec![]);
    };
    let quantity = num(c.arg(2)).unwrap_or(g.stack).max(1.0);
    let cost = (g.price * quantity / g.stack.max(1.0)).round();
    if engine(vm).world.character.money < cost {
        // The server refuses (`/stdtest engine scenario vendor`).
        later(vm, "merchantTooDear", too_dear, vec![]);
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
            g.item_after.map_or(Value::Nil, Value::Num),
        ],
    );
    Ok(vec![])
}

/// "You don't have enough money." (`UI_ERROR_MESSAGE(48, …)`).
fn too_dear(vm: &mut Vm, _c: NativeCall) -> Ret {
    let code = match vm.get_global("LE_GAME_ERR_NOT_ENOUGH_MONEY") {
        Value::Num(n) => n,
        _ => 48.0,
    };
    let text = match vm.get_global("ERR_NOT_ENOUGH_MONEY") {
        Value::Str(s) => Value::Str(s),
        _ => vm.str("You don't have enough money."),
    };
    fire(vm, "UI_ERROR_MESSAGE", &[Value::Num(code), text]);
    Ok(vec![])
}

fn upval(c: &NativeCall, i: usize) -> f64 {
    match c.upvals.get(i) {
        Some(Value::Num(n)) => *n,
        _ => 0.0,
    }
}

/// Where `count` of item `id` go: a stack of it with room (the item's
/// `stackCount`, else 1), else the first free slot.
fn place(vm: &mut Vm, id: i64, count: f64) -> Option<(i64, usize)> {
    let max = num(character::item_field(vm, id, "stackCount")).unwrap_or(1.0);
    let bags = &engine(vm).world.character.bags;
    for (&bag, b) in bags.iter().filter(|(b, _)| character::carried(**b)) {
        if let Some((&slot, _)) = b
            .items
            .iter()
            .find(|(_, h)| h.id == id && h.count + count <= max)
        {
            return Some((bag, slot));
        }
    }
    for (&bag, b) in bags.iter().filter(|(b, _)| character::carried(**b)) {
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
    let Some((bag, slot)) = place(vm, id, quantity) else {
        let message = vm.str("Inventory is full.");
        fire(vm, "UI_ERROR_MESSAGE", &[Value::Num(3.0), message]);
        return Ok(vec![]);
    };
    // Bought: in the vendor's context (14, recorded).
    let link = character::link_in_context(vm, id, "14", &[]);
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
                        link,
                        ..Held::default()
                    },
                );
            }
        }
    }
    let icon = character::item_field(vm, id, "icon");
    let push = |vm: &mut Vm| {
        fire(
            vm,
            "ITEM_PUSH",
            &[Value::Num(character::bag_inventory_slot(bag)), icon],
        );
        loot_message(vm, id, quantity, "14");
        fire(vm, "BAG_NEW_ITEMS_UPDATED", &[]);
    };
    fire(vm, "MERCHANT_UPDATE", &[]);
    // The item's data a packet later (the goods' `itemAfter`), as the
    // client told it in `/stdtest engine scenario vendor` (a new slot): the
    // money, the bag, the item pushed; the slot reading empty till its
    // data come, the equipped spells', the bag's and the inventory's
    // updates then (as a looted item's), no count's change.
    let stacked = engine(vm).world.character.bags[&bag].items[&slot].count > quantity;
    if let Some(Value::Num(after)) = c.upvals.get(3).copied()
        && !stacked
    {
        if let Some(h) = engine_mut(vm)
            .world
            .character
            .bags
            .get_mut(&bag)
            .and_then(|b| b.items.get_mut(&slot))
        {
            h.arriving = true;
        }
        fire(vm, "PLAYER_MONEY", &[]);
        bag_update(vm, bag);
        push(vm);
        fire(vm, "BAG_UPDATE_DELAYED", &[]);
        let native = vm.native_with(
            "merchantItemData",
            item_data_arrived,
            vec![Value::Num(bag as f64), Value::Num(slot as f64)],
        );
        crate::clock::after_engine(vm, after, native);
        return Ok(vec![]);
    }
    // Its data at once (`/stdtest engine record merchant`): the item
    // pushed, the bag, the inventory, the money, the bag again, its count.
    push(vm);
    bag_update(vm, bag);
    let player = vm.str("player");
    fire(vm, "UNIT_INVENTORY_CHANGED", &[player]);
    fire(vm, "PLAYER_MONEY", &[]);
    bag_update(vm, bag);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    Ok(vec![])
}

/// A bought item's data come: its slot reads it; the equipped spells',
/// the bag's and the inventory's updates (recorded, as a looted item's).
fn item_data_arrived(vm: &mut Vm, c: NativeCall) -> Ret {
    let (bag, slot) = (upval(&c, 0) as i64, upval(&c, 1) as usize);
    character::arrived(vm, bag, slot);
    character::equipped_spells_changed(vm);
    bag_update(vm, bag);
    let player = vm.str("player");
    fire(vm, "UNIT_INVENTORY_CHANGED", &[player]);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    Ok(vec![])
}

/// `CHAT_MSG_LOOT("You receive item: <link>", …)` for the player, with the
/// payload the client gave it (`/stdtest engine record vendor2`; a count
/// above one unverified: "<link>x<n>"); the link in its `context` (a
/// vendor's "14", mail's none).
pub fn loot_message(vm: &mut Vm, id: i64, quantity: f64, context: &str) {
    let link = character::link_in_context(vm, id, context, &[]).unwrap_or_default();
    let count = if quantity > 1.0 {
        format!("x{}", quantity as i64)
    } else {
        String::new()
    };
    let text = vm.str(&format!("You receive item: {link}{count}"));
    let (name, guid) = {
        let p = &engine(vm).units.list[units::PLAYER.0];
        let name = match &p.last_name {
            Some(last) => format!("{} {last}", p.name),
            None => p.name.clone(),
        };
        (name, p.guid.clone())
    };
    let line = {
        let m = merchant_mut(vm);
        m.line += 1.0;
        m.line
    };
    let name = vm.str(&name);
    let empty = vm.str("");
    let guid = vm.str(&guid);
    let extra = vm.new_table();
    for (k, v) in [
        ("forwardedMessage", empty),
        ("fromDiscord", Value::Bool(false)),
        ("globalName", empty),
        ("hasAttachment", Value::Bool(false)),
        ("hasEmbed", Value::Bool(false)),
        ("hasEmoji", Value::Bool(false)),
        ("hasError", Value::Bool(false)),
        ("hasForwardedMessage", Value::Bool(false)),
        ("hasPoll", Value::Bool(false)),
        ("hasSticker", Value::Bool(false)),
        ("lastOnlineName", empty),
        ("type", Value::Num(0.0)),
        ("userID", Value::Num(0.0)),
    ] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(extra).set(key, v);
    }
    let no = Value::Bool(false);
    fire(
        vm,
        "CHAT_MSG_LOOT",
        &[
            text,
            name,
            empty,
            empty,
            name,
            empty,
            Value::Num(0.0),
            Value::Num(0.0),
            empty,
            Value::Num(0.0),
            Value::Num(line),
            guid,
            Value::Num(0.0),
            no,
            no,
            no,
            no,
            Value::Table(extra),
        ],
    );
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
    // The inventory changed: the equipped spells' changes before the bag's
    // (recorded: each of `vendor`'s four sales).
    character::equipped_spells_changed(vm);
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
    // Writing a letter: attached (`mail.rs`).
    if crate::mail::writing(vm) {
        crate::mail::attach(vm, bag as i64, slot as usize);
        return Ok(vec![]);
    }
    // At the bank: a carried item into its first free slot, a bank item
    // into the bags (`cursor.rs`).
    if crate::bank::is_open(vm) {
        crate::cursor::use_at_bank(vm, bag as i64, slot as usize);
        return Ok(vec![]);
    }
    // Away from a vendor: worn, or used (`inventory.rs`).
    if !merchant(vm).open {
        crate::inventory::use_container_item(vm, bag as i64, slot as usize);
        return Ok(vec![]);
    }
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

/// The buyback, as the server answers it: placed as a purchase is, not
/// back where it was (recorded: a claw sold from slot 7 bought back into
/// the first free slot, 6).
fn bought_back(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = upval(&c, 0) as usize;
    let list = &mut merchant_mut(vm).buyback;
    if i == 0 || i > list.len() {
        return Ok(vec![]);
    }
    let s = list.remove(i - 1);
    let Some((bag, slot)) = place(vm, s.held.id, s.held.count) else {
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
    // The equipped spells' changes, as a sale's (recorded).
    character::equipped_spells_changed(vm);
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
        ("C_MerchantFrame.IsMerchantItemRefundable", refundable),
        ("GetMerchantFilter", filter),
        ("CanMerchantRepair", can_repair),
        ("GetRepairAllCost", repair_all_cost),
        ("RepairAllItems", repair_all),
        ("CanGuildBankRepair", yes),
        ("InRepairMode", crate::actions2::in_repair_mode),
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
