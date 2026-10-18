//! An auction house, as the client recorded a Horde paladin's in Orgrimmar
//! (`/stdtest engine scenario auction`, a ping near 0.33 s):
//! `defineAuctionHouse(unit, { browse, infoReceived, commodities, items,
//! deposit, nextAuctionID })` names what its searches find;
//! `player:interact(auctioneer)` opens it a round trip later.
//!
//! The server's messages are throttled: a query is sent at once
//! (`AUCTION_HOUSE_THROTTLED_MESSAGE_SENT`) when the system is ready, else
//! queued (`…_QUEUED`) and sent when the one before is answered; its answer
//! a round trip later (`…_RESPONSE_RECEIVED` among its events), the system
//! ready again a moment after (`…_SYSTEM_READY`) unless a queued one goes.
//!
//! Recorded: opening (the bids' query sent, `AUCTION_HOUSE_SHOW`, the
//! delivery delay, the interaction; its answer `BIDS_UPDATED`); a browse
//! search (`AUCTION_HOUSE_NEW_RESULTS_RECEIVED(nil)`, the browse results,
//! the items' infos received after); a commodity's search (its results,
//! `COMMODITY_SEARCH_RESULTS_UPDATED`, again once its owners' names are
//! known); buying (`StartCommoditiesPurchase`: the price; confirming: the
//! purchase completed, the won notification, the money, the results
//! refreshed); selling (an item put up: locked, its search; `PostItem`:
//! the auction created, the item gone, the deposit paid, the item's search
//! again); `CloseAuctionHouse`. The house's settings (filters, copper
//! values, auto-populated prices) are the client's recorded answers.
//!
//! Unverified: bids and buying items (not commodities), cancelling,
//! favorites, more than one page of results, the deposit's formula (the
//! data's `deposit`), item keys beyond their item ID.

use std::collections::{BTreeMap, VecDeque};

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::cursor::Source;
use crate::units::{self, UnitId};
use crate::{character, engine, engine_mut, widgets};

/// `Enum.PlayerInteractionType.Auctioneer`.
const AUCTIONEER: f64 = 21.0;
/// A commodity's price quote's length (unverified: not recorded).
const QUOTE_LENGTH: f64 = 30.0;
/// The system ready again after an answer (recorded: 0.11 to 0.135 s).
const READY_AFTER: f64 = 0.115;
/// The browse results' items' infos after the results (recorded).
const INFO_AFTER: f64 = 0.034;
/// A commodity's owners' names after its results (recorded).
const OWNERS_AFTER: f64 = 0.505;
/// A commodity purchase's completion, then its answer (recorded: 0.744
/// and 0.103 s after the confirmation).
const PURCHASED_AFTER: f64 = 0.744;
const PURCHASE_ANSWER_AFTER: f64 = 0.103;
/// The money after a purchase's answer (recorded).
const PAID_AFTER: f64 = 0.113;
/// A posted item: the inventory's update, the bag's and the deposit's
/// (recorded: 0.107 and 0.324 s after the answer).
const POSTED_INVENTORY_AFTER: f64 = 0.107;
const POSTED_BAG_AFTER: f64 = 0.324;

/// What a query's answer does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Query {
    Bids,
    Browse,
    Commodity(i64),
    Item(i64),
    Price(i64, f64),
    Purchase(i64, f64),
    Post(i64, usize, i64),
}

#[derive(Default)]
pub struct AuctionHouse {
    pub houses: BTreeMap<usize, TableRef>,
    pub open: Option<UnitId>,
    /// The throttled system ready for a message.
    ready: bool,
    queue: VecDeque<Query>,
    /// The browse results shown (after a browse search).
    browsed: bool,
    /// Commodities searched: the results' ends (`GetTime`) and owners
    /// known; how many were bought.
    searched: BTreeMap<i64, (f64, bool)>,
    bought: BTreeMap<i64, f64>,
    /// Items searched (their results told).
    item_searched: BTreeMap<i64, bool>,
    /// The player's auctions: (auction ID, item ID, buyout).
    owned: Vec<(f64, i64, f64)>,
    next_auction: f64,
    /// The buyout of the item being posted.
    posting: f64,
    /// A purchase's quote's end (`GetTime`).
    quote_until: f64,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn ah(vm: &Vm) -> &AuctionHouse {
    &engine(vm).world.auction
}

fn ah_mut(vm: &mut Vm) -> &mut AuctionHouse {
    &mut engine_mut(vm).world.auction
}

fn later(vm: &mut Vm, after: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, after, native);
}

fn data(vm: &Vm) -> Option<TableRef> {
    let a = ah(vm);
    a.open
        .and_then(|u| a.houses.get(&u.0).copied())
        .or_else(|| a.houses.values().next().copied())
}

fn list(vm: &mut Vm, t: TableRef) -> Vec<TableRef> {
    let n = vm.heap.table(t).border();
    (1..=n)
        .filter_map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Table(e) => Some(e),
            _ => None,
        })
        .collect()
}

/// `defineAuctionHouse(unit, { browse = { { itemKey, minPrice,
/// totalQuantity, containsOwnerItem } }, infoReceived = { itemID, … },
/// commodities = { [itemID] = { { auctionID, quantity, unitPrice,
/// timeLeftSeconds, totalNumberOfOwners, owners, numOwnerItems,
/// containsOwnerItem, containsAccountItem } } }, items = { [itemID] = { …
/// } }, deposit, nextAuctionID })`.
pub fn control_define(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(unit) = units::handle_of(vm, c.arg(1)) else {
        return Err(vm.error("defineAuctionHouse: an auctioneer expected"));
    };
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("defineAuctionHouse: a table of the house expected"));
    };
    widgets::keep(vm, Value::Table(t));
    let next = num(field(vm, t, "nextAuctionID")).unwrap_or(1.0);
    let a = ah_mut(vm);
    a.houses.insert(unit.0, t);
    a.next_auction = next;
    a.ready = true;
    Ok(vec![])
}

pub fn has(vm: &Vm, unit: UnitId) -> bool {
    ah(vm).houses.contains_key(&unit.0)
}

/// The player at an auctioneer: the house opens a round trip later.
pub fn interact(vm: &mut Vm, unit: UnitId) {
    let delay = crate::clock::server_delay(vm);
    later(
        vm,
        delay,
        "auctionOpened",
        opened,
        vec![Value::Num(unit.0 as f64)],
    );
}

fn opened(vm: &mut Vm, c: NativeCall) -> Ret {
    let unit = UnitId(num(c.upvals[0]).unwrap_or(0.0) as usize);
    {
        let a = ah_mut(vm);
        a.open = Some(unit);
        a.ready = true;
        a.queue.clear();
    }
    engine_mut(vm).units.npc = Some(unit);
    // The bids asked for as it opens (recorded: the message before the
    // house shows).
    send(vm, Query::Bids);
    fire(vm, "AUCTION_HOUSE_SHOW", &[]);
    fire(
        vm,
        "AUCTION_HOUSE_ITEM_DELIVERY_DELAY_UPDATE",
        &[Value::Num(0.0), Value::Num(0.0)],
    );
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(AUCTIONEER)],
    );
    Ok(vec![])
}

fn encode(q: Query) -> Vec<Value> {
    let (kind, a, b, c) = match q {
        Query::Bids => (0.0, 0.0, 0.0, 0.0),
        Query::Browse => (1.0, 0.0, 0.0, 0.0),
        Query::Commodity(id) => (2.0, id as f64, 0.0, 0.0),
        Query::Item(id) => (3.0, id as f64, 0.0, 0.0),
        Query::Price(id, n) => (4.0, id as f64, n, 0.0),
        Query::Purchase(id, n) => (5.0, id as f64, n, 0.0),
        Query::Post(bag, slot, id) => (6.0, bag as f64, slot as f64, id as f64),
    };
    vec![
        Value::Num(kind),
        Value::Num(a),
        Value::Num(b),
        Value::Num(c),
    ]
}

fn decode(c: &NativeCall) -> Query {
    let n = |i: usize| num(c.upvals[i]).unwrap_or(0.0);
    match n(0) as i64 {
        0 => Query::Bids,
        1 => Query::Browse,
        2 => Query::Commodity(n(1) as i64),
        3 => Query::Item(n(1) as i64),
        4 => Query::Price(n(1) as i64, n(2)),
        5 => Query::Purchase(n(1) as i64, n(2)),
        _ => Query::Post(n(1) as i64, n(2) as usize, n(3) as i64),
    }
}

/// A query to the server: sent when the system is ready, else queued.
fn send(vm: &mut Vm, q: Query) {
    // The same query already waiting: nothing more (unverified: the
    // client's dedup; a refreshed search queued once, recorded).
    if ah(vm).queue.contains(&q) {
        return;
    }
    if !ah(vm).ready {
        ah_mut(vm).queue.push_back(q);
        fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_QUEUED", &[]);
        return;
    }
    go(vm, q);
}

fn go(vm: &mut Vm, q: Query) {
    ah_mut(vm).ready = false;
    fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_SENT", &[]);
    let delay = crate::clock::server_delay(vm);
    let after = match q {
        Query::Purchase(..) => PURCHASED_AFTER,
        _ => delay,
    };
    later(vm, after, "auctionAnswered", answered, encode(q));
}

/// The answer done: the next queued message sent, else the system ready
/// a moment later.
fn done(vm: &mut Vm) {
    if ah(vm).queue.is_empty() {
        later(vm, READY_AFTER, "auctionReady", ready, vec![]);
    }
}

/// The next queued message (with the frame's other news, recorded).
fn next_queued(vm: &mut Vm) {
    if let Some(q) = ah_mut(vm).queue.pop_front() {
        go(vm, q);
    }
}

fn ready(vm: &mut Vm, _c: NativeCall) -> Ret {
    if ah(vm).open.is_none() {
        return Ok(vec![]);
    }
    if let Some(q) = ah_mut(vm).queue.pop_front() {
        go(vm, q);
        return Ok(vec![]);
    }
    ah_mut(vm).ready = true;
    fire(vm, "AUCTION_HOUSE_THROTTLED_SYSTEM_READY", &[]);
    Ok(vec![])
}

fn item_key(vm: &mut Vm, id: i64) -> Value {
    let t = vm.new_table();
    set(vm, t, "itemID", Value::Num(id as f64));
    // Recorded 0 for a worn item put up (Battered Leather Gloves).
    set(vm, t, "itemLevel", Value::Num(0.0));
    set(vm, t, "itemSuffix", Value::Num(0.0));
    set(vm, t, "battlePetSpeciesID", Value::Num(0.0));
    Value::Table(t)
}

/// A search's key as the data has it (a browse row's), else made.
fn key_of(vm: &mut Vm, id: i64) -> Value {
    if let Some(t) = data(vm)
        && let Value::Table(b) = field(vm, t, "browse")
    {
        for row in list(vm, b) {
            if let Value::Table(k) = field(vm, row, "itemKey")
                && num(field(vm, k, "itemID")) == Some(id as f64)
            {
                return Value::Table(k);
            }
        }
    }
    item_key(vm, id)
}

fn answered(vm: &mut Vm, c: NativeCall) -> Ret {
    if ah(vm).open.is_none() {
        return Ok(vec![]);
    }
    match decode(&c) {
        Query::Bids => {
            fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_RESPONSE_RECEIVED", &[]);
            fire(vm, "BIDS_UPDATED", &[]);
            done(vm);
        }
        Query::Browse => {
            ah_mut(vm).browsed = true;
            fire(vm, "AUCTION_HOUSE_NEW_RESULTS_RECEIVED", &[Value::Nil]);
            fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_RESPONSE_RECEIVED", &[]);
            fire(vm, "AUCTION_HOUSE_BROWSE_RESULTS_UPDATED", &[]);
            later(vm, INFO_AFTER, "auctionInfos", infos, vec![]);
            done(vm);
        }
        Query::Commodity(id) => {
            commodity_results(vm, id, true);
            done(vm);
        }
        Query::Item(id) => {
            ah_mut(vm).item_searched.insert(id, true);
            let key = key_of(vm, id);
            fire(vm, "AUCTION_HOUSE_NEW_RESULTS_RECEIVED", &[key]);
            fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_RESPONSE_RECEIVED", &[]);
            fire(vm, "ITEM_SEARCH_RESULTS_UPDATED", &[key, Value::Nil]);
            done(vm);
        }
        Query::Price(id, n) => {
            let unit = unit_price(vm, id).unwrap_or(0.0);
            let now = crate::clock::now(vm);
            ah_mut(vm).quote_until = now + QUOTE_LENGTH;
            fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_RESPONSE_RECEIVED", &[]);
            fire(
                vm,
                "COMMODITY_PRICE_UPDATED",
                &[Value::Num(unit), Value::Num(unit * n)],
            );
            done(vm);
        }
        Query::Purchase(id, n) => {
            fire(vm, "AUCTION_HOUSE_PURCHASE_COMPLETED", &[Value::Num(0.0)]);
            fire(vm, "AUCTION_CANCELED", &[Value::Num(0.0)]);
            // The won item comes by mail (recorded: the house's sender).
            let sender = match data(vm) {
                Some(t) => match field(vm, t, "mailSender") {
                    Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                    _ => "Horde Auction House".to_string(),
                },
                None => "Horde Auction House".to_string(),
            };
            crate::mail::mail_coming(vm, &sender);
            fire(vm, "UPDATE_PENDING_MAIL", &[]);
            later(
                vm,
                PURCHASE_ANSWER_AFTER,
                "auctionBought",
                bought,
                vec![Value::Num(id as f64), Value::Num(n)],
            );
        }
        Query::Post(bag, slot, id) => {
            let auction = ah(vm).next_auction;
            ah_mut(vm).next_auction += 1.0;
            fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_RESPONSE_RECEIVED", &[]);
            fire(vm, "AUCTION_HOUSE_AUCTION_CREATED", &[Value::Num(auction)]);
            let buyout = ah(vm).posting;
            later(
                vm,
                POSTED_INVENTORY_AFTER,
                "auctionPosted",
                posted_inventory,
                vec![],
            );
            later(
                vm,
                POSTED_BAG_AFTER,
                "auctionPostedBag",
                posted_bag,
                vec![
                    Value::Num(bag as f64),
                    Value::Num(slot as f64),
                    Value::Num(id as f64),
                    Value::Num(auction),
                    Value::Num(buyout),
                ],
            );
            if let Some(q) = ah_mut(vm).queue.pop_front() {
                // The queued message goes with the inventory's news.
                later(
                    vm,
                    POSTED_INVENTORY_AFTER + 0.03,
                    "auctionQueued",
                    send_queued,
                    encode(q),
                );
            } else {
                done(vm);
            }
        }
    }
    Ok(vec![])
}

fn send_queued(vm: &mut Vm, c: NativeCall) -> Ret {
    go(vm, decode(&c));
    Ok(vec![])
}

fn infos(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some(t) = data(vm) else {
        return Ok(vec![]);
    };
    if let Value::Table(l) = field(vm, t, "infoReceived") {
        let n = vm.heap.table(l).border();
        for i in 1..=n {
            let id = vm.heap.table(l).get(Value::Num(i as f64));
            fire(vm, "ITEM_KEY_ITEM_INFO_RECEIVED", &[id]);
        }
    }
    Ok(vec![])
}

/// A commodity's results told (their owners' names a moment later).
fn commodity_results(vm: &mut Vm, id: i64, answer: bool) {
    let now = crate::clock::now(vm);
    ah_mut(vm).searched.insert(id, (now, false));
    let key = key_of(vm, id);
    fire(vm, "AUCTION_HOUSE_NEW_RESULTS_RECEIVED", &[key]);
    if answer {
        fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_RESPONSE_RECEIVED", &[]);
    }
    fire(vm, "COMMODITY_SEARCH_RESULTS_RECEIVED", &[]);
    fire(
        vm,
        "COMMODITY_SEARCH_RESULTS_UPDATED",
        &[Value::Num(id as f64)],
    );
    later(
        vm,
        OWNERS_AFTER,
        "auctionOwners",
        owners_known,
        vec![Value::Num(id as f64)],
    );
}

fn owners_known(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(c.upvals[0]).unwrap_or(0.0) as i64;
    let Some(entry) = ah_mut(vm).searched.get_mut(&id) else {
        return Ok(vec![]);
    };
    if entry.1 {
        return Ok(vec![]);
    }
    entry.1 = true;
    fire(
        vm,
        "COMMODITY_SEARCH_RESULTS_UPDATED",
        &[Value::Num(id as f64)],
    );
    Ok(vec![])
}

/// The purchase's answer: the won notification, the results refreshed
/// (queued), the purchase succeeded, the results emptied; the money after.
fn bought(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(c.upvals[0]).unwrap_or(0.0) as i64;
    let n = num(c.upvals[1]).unwrap_or(1.0);
    let price = unit_price(vm, id).unwrap_or(0.0) * n;
    *ah_mut(vm).bought.entry(id).or_default() += n;
    ah_mut(vm).searched.remove(&id);
    fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_RESPONSE_RECEIVED", &[]);
    let name = character::item_field(vm, id, "name");
    fire(
        vm,
        "AUCTION_HOUSE_SHOW_COMMODITY_WON_NOTIFICATION",
        &[name, Value::Num(n)],
    );
    // The house refreshes the results (queued till the money's news).
    ah_mut(vm).queue.push_back(Query::Commodity(id));
    fire(vm, "AUCTION_HOUSE_THROTTLED_MESSAGE_QUEUED", &[]);
    fire(vm, "COMMODITY_PURCHASE_SUCCEEDED", &[]);
    fire(
        vm,
        "COMMODITY_SEARCH_RESULTS_UPDATED",
        &[Value::Num(id as f64)],
    );
    later(vm, PAID_AFTER, "auctionPaid", paid, vec![Value::Num(price)]);
    Ok(vec![])
}

fn paid(vm: &mut Vm, c: NativeCall) -> Ret {
    let price = num(c.upvals[0]).unwrap_or(0.0);
    {
        let money = &mut engine_mut(vm).world.character.money;
        *money = (*money - price).max(0.0);
    }
    fire(vm, "PLAYER_MONEY", &[]);
    next_queued(vm);
    Ok(vec![])
}

fn posted_inventory(vm: &mut Vm, _c: NativeCall) -> Ret {
    character::equipped_spells_changed(vm);
    units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", units::PLAYER, &[]);
    Ok(vec![])
}

/// The posted item gone from its bag, the deposit paid.
fn posted_bag(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = |i: usize| num(c.upvals[i]).unwrap_or(0.0);
    let (bag, slot, id) = (n(0) as i64, n(1) as usize, n(2) as i64);
    let deposit = match data(vm) {
        Some(t) => num(field(vm, t, "deposit")).unwrap_or(0.0),
        None => 0.0,
    };
    if let Some(b) = engine_mut(vm).world.character.bags.get_mut(&bag) {
        b.items.remove(&slot);
    }
    ah_mut(vm).owned.push((n(3), id, n(4)));
    {
        let money = &mut engine_mut(vm).world.character.money;
        *money = (*money - deposit).max(0.0);
    }
    fire(vm, "PLAYER_MONEY", &[]);
    character::bag_updated(vm, bag);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    Ok(vec![])
}

/// A commodity's cheapest price (its first result's).
fn unit_price(vm: &mut Vm, id: i64) -> Option<f64> {
    let results = commodity_data(vm, id);
    let first = results.first().copied()?;
    num(field(vm, first, "unitPrice"))
}

fn commodity_data(vm: &mut Vm, id: i64) -> Vec<TableRef> {
    let Some(t) = data(vm) else {
        return vec![];
    };
    let Value::Table(c) = field(vm, t, "commodities") else {
        return vec![];
    };
    match vm.heap.table(c).get(Value::Num(id as f64)) {
        Value::Table(l) => list(vm, l),
        _ => vec![],
    }
}

fn item_data(vm: &mut Vm, id: i64) -> Vec<TableRef> {
    let Some(t) = data(vm) else {
        return vec![];
    };
    let Value::Table(c) = field(vm, t, "items") else {
        return vec![];
    };
    match vm.heap.table(c).get(Value::Num(id as f64)) {
        Value::Table(l) => list(vm, l),
        _ => vec![],
    }
}

/// An item key's item ID (a table's `itemID`, or an ID).
fn key_id(vm: &mut Vm, v: Value) -> Option<i64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n as i64),
        Value::Table(t) => num(field(vm, t, "itemID")).map(|n| n as i64),
        _ => None,
    }
}

fn copy(vm: &mut Vm, from: TableRef) -> TableRef {
    let to = vm.new_table();
    let entries: Vec<(Value, Value)> = vm.heap.table(from).entries().collect();
    for (k, v) in entries {
        let _ = vm.heap.table_mut(to).set(k, v);
    }
    to
}

// The answers.

fn is_ready(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(ah(vm).open.is_some() && ah(vm).ready)])
}

fn send_browse_query(vm: &mut Vm, _c: NativeCall) -> Ret {
    if ah(vm).open.is_some() {
        send(vm, Query::Browse);
    }
    Ok(vec![])
}

/// `C_AuctionHouse.SendSearchQuery(itemKey, sorts, separateOwnerItems)`:
/// a commodity's results, else an item's.
fn send_search_query(vm: &mut Vm, c: NativeCall) -> Ret {
    if ah(vm).open.is_none() {
        return Ok(vec![]);
    }
    let Some(id) = key_id(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    if commodity_data(vm, id).is_empty() {
        send(vm, Query::Item(id));
    } else {
        send(vm, Query::Commodity(id));
    }
    Ok(vec![])
}

fn send_sell_search_query(vm: &mut Vm, c: NativeCall) -> Ret {
    send_search_query(vm, c)
}

fn refresh_commodity_search_results(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(id) = key_id(vm, c.arg(1))
        && ah(vm).open.is_some()
    {
        send(vm, Query::Commodity(id));
    }
    Ok(vec![])
}

fn refresh_item_search_results(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(id) = key_id(vm, c.arg(1))
        && ah(vm).open.is_some()
    {
        send(vm, Query::Item(id));
    }
    Ok(vec![])
}

fn browse_rows(vm: &mut Vm) -> Vec<TableRef> {
    if !ah(vm).browsed {
        return vec![];
    }
    let Some(t) = data(vm) else {
        return vec![];
    };
    match field(vm, t, "browse") {
        Value::Table(b) => list(vm, b),
        _ => vec![],
    }
}

/// `C_AuctionHouse.GetBrowseResults()`: the rows (a bought commodity's
/// quantity less).
fn get_browse_results(vm: &mut Vm, _c: NativeCall) -> Ret {
    let out = vm.new_table();
    for (i, row) in browse_rows(vm).into_iter().enumerate() {
        let r = copy(vm, row);
        if let Value::Table(k) = field(vm, row, "itemKey")
            && let Some(id) = num(field(vm, k, "itemID"))
            && let Some(n) = ah(vm).bought.get(&(id as i64)).copied()
            && let Some(q) = num(field(vm, row, "totalQuantity"))
        {
            set(vm, r, "totalQuantity", Value::Num(q - n));
        }
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num((i + 1) as f64), Value::Table(r));
    }
    Ok(vec![Value::Table(out)])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn yes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// The commodity's results while searched (none once bought, till the
/// refresh).
fn commodity_results_of(vm: &mut Vm, c: &NativeCall) -> (i64, Vec<TableRef>) {
    let Some(id) = key_id(vm, c.arg(1)) else {
        return (0, vec![]);
    };
    if !ah(vm).searched.contains_key(&id) {
        return (id, vec![]);
    }
    (id, commodity_data(vm, id))
}

fn get_num_commodity_search_results(vm: &mut Vm, c: NativeCall) -> Ret {
    let (_, r) = commodity_results_of(vm, &c);
    Ok(vec![Value::Num(r.len() as f64)])
}

fn has_full_commodity_search_results(vm: &mut Vm, c: NativeCall) -> Ret {
    let (_, r) = commodity_results_of(vm, &c);
    Ok(vec![Value::Bool(!r.is_empty())])
}

/// `C_AuctionHouse.GetCommoditySearchResultInfo(itemID, i)`: the data's
/// row, its time left from the search, its owners once known, the first
/// row less what was bought.
fn get_commodity_search_result_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let (id, rows) = commodity_results_of(vm, &c);
    let Some(i) = num(vm.hidden(c.arg(2))) else {
        return Ok(vec![]);
    };
    let Some(row) = rows.get((i as usize).wrapping_sub(1)).copied() else {
        return Ok(vec![]);
    };
    let r = copy(vm, row);
    let (since, owners) = ah(vm).searched.get(&id).copied().unwrap_or((0.0, false));
    let now = crate::clock::now(vm);
    if let Some(left) = num(field(vm, row, "timeLeftSeconds")) {
        set(
            vm,
            r,
            "timeLeftSeconds",
            Value::Num((left - (now - since)).floor().max(0.0)),
        );
    }
    if !owners {
        let empty = vm.new_table();
        set(vm, r, "owners", Value::Table(empty));
    }
    if i as usize == 1
        && let Some(n) = ah(vm).bought.get(&id).copied()
        && let Some(q) = num(field(vm, row, "quantity"))
    {
        set(vm, r, "quantity", Value::Num(q - n));
    }
    Ok(vec![Value::Table(r)])
}

fn get_commodity_search_results_quantity(vm: &mut Vm, c: NativeCall) -> Ret {
    let (id, rows) = commodity_results_of(vm, &c);
    let mut total = 0.0;
    for row in rows {
        total += num(field(vm, row, "quantity")).unwrap_or(0.0);
    }
    if total > 0.0 {
        total -= ah(vm).bought.get(&id).copied().unwrap_or(0.0);
    }
    Ok(vec![Value::Num(total)])
}

/// An item's results once searched: the data's, and the player's own
/// auctions of it.
fn item_results(vm: &mut Vm, c: &NativeCall) -> Vec<TableRef> {
    let Some(id) = key_id(vm, c.arg(1)) else {
        return vec![];
    };
    if !ah(vm).item_searched.contains_key(&id) {
        return vec![];
    }
    let mut rows = item_data(vm, id);
    let owned: Vec<(f64, i64, f64)> = ah(vm).owned.iter().filter(|o| o.1 == id).copied().collect();
    for (auction, item, buyout) in owned {
        let r = vm.new_table();
        set(vm, r, "auctionID", Value::Num(auction));
        let key = key_of(vm, item);
        set(vm, r, "itemKey", key);
        set(vm, r, "buyoutAmount", Value::Num(buyout));
        set(vm, r, "quantity", Value::Num(1.0));
        set(vm, r, "containsOwnerItem", Value::Bool(true));
        set(vm, r, "containsAccountItem", Value::Bool(true));
        set(vm, r, "containsSocketedItem", Value::Bool(false));
        set(vm, r, "timeLeft", Value::Num(1.0));
        let owners = vm.new_table();
        let me = vm.str("player");
        let _ = vm.heap.table_mut(owners).set(Value::Num(1.0), me);
        set(vm, r, "owners", Value::Table(owners));
        set(vm, r, "totalNumberOfOwners", Value::Num(1.0));
        if let Some(l) = character::link_of_id(vm, item) {
            set(vm, r, "itemLink", l);
        }
        rows.push(r);
    }
    rows
}

fn get_num_item_search_results(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = item_results(vm, &c).len();
    Ok(vec![Value::Num(n as f64)])
}

fn has_full_item_search_results(vm: &mut Vm, c: NativeCall) -> Ret {
    let searched = key_id(vm, c.arg(1)).is_some_and(|id| ah(vm).item_searched.contains_key(&id));
    Ok(vec![Value::Bool(searched)])
}

fn get_item_search_result_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let rows = item_results(vm, &c);
    let Some(i) = num(vm.hidden(c.arg(2))) else {
        return Ok(vec![]);
    };
    Ok(match rows.get((i as usize).wrapping_sub(1)) {
        Some(r) => vec![Value::Table(*r)],
        None => vec![],
    })
}

/// `C_AuctionHouse.StartCommoditiesPurchase(itemID, quantity)`: its price.
fn start_commodities_purchase(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(id), Some(n)) = (key_id(vm, c.arg(1)), num(vm.hidden(c.arg(2)))) else {
        return Ok(vec![]);
    };
    if ah(vm).open.is_some() {
        send(vm, Query::Price(id, n));
    }
    Ok(vec![])
}

fn confirm_commodities_purchase(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(id), Some(n)) = (key_id(vm, c.arg(1)), num(vm.hidden(c.arg(2)))) else {
        return Ok(vec![]);
    };
    if ah(vm).open.is_some() {
        send(vm, Query::Purchase(id, n));
    }
    Ok(vec![])
}

/// An item location's bag and slot (`ItemLocation`'s `bagID`,
/// `slotIndex`).
fn location(vm: &mut Vm, v: Value) -> Option<(i64, usize)> {
    let Value::Table(t) = v else {
        return None;
    };
    let bag = num(field(vm, t, "bagID"))?;
    let slot = num(field(vm, t, "slotIndex"))?;
    Some((bag as i64, slot as usize))
}

fn held_at(vm: &Vm, bag: i64, slot: usize) -> Option<i64> {
    engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .and_then(|b| b.items.get(&slot))
        .map(|h| h.id)
}

/// `C_Item.LockItem(location)`, `C_Item.UnlockItem`: the bag slot's lock
/// told.
fn lock_item(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some((bag, slot)) = location(vm, c.arg(1)) {
        crate::cursor::lock(vm, &Source::Bag(bag, slot), true);
    }
    Ok(vec![])
}

fn unlock_item(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some((bag, slot)) = location(vm, c.arg(1)) {
        crate::cursor::lock(vm, &Source::Bag(bag, slot), false);
    }
    Ok(vec![])
}

/// `C_AuctionHouse.PostItem(location, duration, quantity, bid, buyout)`:
/// sent; the browse results' update; its answer the auction created.
fn post_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((bag, slot)) = location(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let Some(id) = held_at(vm, bag, slot) else {
        return Ok(vec![]);
    };
    if ah(vm).open.is_none() {
        return Ok(vec![]);
    }
    ah_mut(vm).posting = num(vm.hidden(c.arg(5))).unwrap_or(0.0);
    send(vm, Query::Post(bag, slot, id));
    // Recorded right after the message: the browse results' update.
    fire(vm, "AUCTION_HOUSE_BROWSE_RESULTS_UPDATED", &[]);
    Ok(vec![Value::Bool(true)])
}

/// `C_AuctionHouse.CalculateItemDeposit(location, duration, quantity)`:
/// the data's deposit.
fn calculate_item_deposit(vm: &mut Vm, _c: NativeCall) -> Ret {
    let deposit = match data(vm) {
        Some(t) => num(field(vm, t, "deposit")).unwrap_or(0.0),
        None => 0.0,
    };
    Ok(vec![Value::Num(deposit)])
}

/// `C_AuctionHouse.CloseAuctionHouse()`: closed, the interaction's hides.
fn close_auction_house(vm: &mut Vm, _c: NativeCall) -> Ret {
    if ah_mut(vm).open.take().is_none() {
        return Ok(vec![]);
    }
    engine_mut(vm).units.npc = None;
    fire(vm, "AUCTION_HOUSE_CLOSED", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(AUCTIONEER)],
    );
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(0.0)],
    );
    Ok(vec![])
}

fn enum_value(vm: &mut Vm, table: &str, key: &str) -> Value {
    let Value::Table(e) = vm.get_global("Enum") else {
        return Value::Nil;
    };
    let t = field(vm, e, table);
    let Value::Table(t) = t else {
        return Value::Nil;
    };
    field(vm, t, key)
}

/// `C_AuctionHouse.GetItemKeyInfo(itemKey)`: the item's name, quality,
/// icon from its data; none for an item the engine has no data of (the
/// client then asks the server, `ITEM_KEY_ITEM_INFO_RECEIVED`).
fn get_item_key_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = key_id(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let name = character::item_field(vm, id, "name");
    if name.is_nil() {
        return Ok(vec![]);
    }
    let t = vm.new_table();
    set(vm, t, "itemID", Value::Num(id as f64));
    set(vm, t, "battlePetSpeciesID", Value::Num(0.0));
    set(vm, t, "itemName", name);
    let quality = character::item_field(vm, id, "quality");
    set(vm, t, "quality", quality);
    let icon = character::item_field(vm, id, "icon");
    set(vm, t, "iconFileID", icon);
    set(vm, t, "isPet", Value::Bool(false));
    let stack = num(character::item_field(vm, id, "stackCount")).unwrap_or(1.0);
    set(vm, t, "isCommodity", Value::Bool(stack > 1.0));
    let equipment = crate::inventory::slot_for(vm, id).is_some();
    set(vm, t, "isEquipment", Value::Bool(equipment));
    Ok(vec![Value::Table(t)])
}

/// `C_AuctionHouse.MakeItemKey(itemID, itemLevel, itemSuffix,
/// battlePetSpeciesID)`: the key, 0 for what is not given.
fn make_item_key(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = num(vm.hidden(c.arg(1))) else {
        return Err(vm.error(
            "bad argument #1 to '?' (Usage: local itemKey = C_AuctionHouse.MakeItemKey(itemID [, itemLevel, itemSuffix, battlePetSpeciesID]))",
        ));
    };
    let t = vm.new_table();
    set(vm, t, "itemID", Value::Num(id));
    for (i, k) in ["itemLevel", "itemSuffix", "battlePetSpeciesID"]
        .iter()
        .enumerate()
    {
        let v = num(vm.hidden(c.arg(i + 2))).unwrap_or(0.0);
        set(vm, t, k, Value::Num(v));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_AuctionHouse.HasSearchResults(itemKey)`: the key searched (a
/// commodity's or an item's).
fn has_search_results(vm: &mut Vm, c: NativeCall) -> Ret {
    let has = key_id(vm, c.arg(1)).is_some_and(|id| {
        let a = ah(vm);
        a.searched.contains_key(&id) || a.item_searched.contains_key(&id)
    });
    Ok(vec![Value::Bool(has)])
}

/// The bag item at an item location, if any: its ID, bound.
fn located_item(vm: &mut Vm, v: Value) -> Option<(i64, bool)> {
    let (bag, slot) = location(vm, v)?;
    engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .and_then(|b| b.items.get(&slot))
        .map(|h| (h.id, h.bound))
}

/// `C_AuctionHouse.IsSellItemValid(location, displayError)`: an unbound
/// bag item (unverified beyond the recorded gloves').
fn is_sell_item_valid(vm: &mut Vm, c: NativeCall) -> Ret {
    let ok = located_item(vm, c.arg(1)).is_some_and(|(_, bound)| !bound);
    Ok(vec![Value::Bool(ok)])
}

/// `C_AuctionHouse.GetItemCommodityStatus(location)`: a commodity when it
/// stacks, else an item (unverified: the client's rule).
fn get_item_commodity_status(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((id, _)) = located_item(vm, c.arg(1)) else {
        return Ok(vec![enum_value(vm, "ItemCommodityStatus", "Unknown")]);
    };
    let stack = num(character::item_field(vm, id, "stackCount")).unwrap_or(1.0);
    let status = if stack > 1.0 { "Commodity" } else { "Item" };
    Ok(vec![enum_value(vm, "ItemCommodityStatus", status)])
}

/// `C_AuctionHouse.GetItemKeyFromItem(location)`: its key (recorded for
/// the gloves: level 0).
fn get_item_key_from_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((id, _)) = located_item(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    Ok(vec![key_of(vm, id)])
}

/// `C_AuctionHouse.GetAvailablePostCount(location)`: the stack's count.
fn get_available_post_count(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((bag, slot)) = location(vm, c.arg(1)) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    let count = engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .and_then(|b| b.items.get(&slot))
        .map_or(0.0, |h| h.count);
    Ok(vec![Value::Num(count)])
}

/// `C_AuctionHouse.GetQuoteDurationRemaining()`: the quote's seconds
/// left.
fn get_quote_duration_remaining(vm: &mut Vm, _c: NativeCall) -> Ret {
    let now = crate::clock::now(vm);
    let left = (ah(vm).quote_until - now).max(0.0).ceil();
    Ok(vec![Value::Num(left)])
}

/// `C_AuctionHouse.QueryBids()`, `QueryOwnedAuctions()`: asked as the house
/// opens, the client's own query already gone (no message recorded).
fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_AuctionHouse.GetItemKeyInfo", get_item_key_info),
        ("C_AuctionHouse.MakeItemKey", make_item_key),
        (
            "C_AuctionHouse.GetQuoteDurationRemaining",
            get_quote_duration_remaining,
        ),
        ("C_AuctionHouse.CancelCommoditiesPurchase", nothing),
        ("C_AuctionHouse.HasSearchResults", has_search_results),
        ("C_AuctionHouse.IsSellItemValid", is_sell_item_valid),
        (
            "C_AuctionHouse.GetItemCommodityStatus",
            get_item_commodity_status,
        ),
        ("C_AuctionHouse.GetItemKeyFromItem", get_item_key_from_item),
        (
            "C_AuctionHouse.GetAvailablePostCount",
            get_available_post_count,
        ),
        ("C_WowTokenPublic.IsAuctionableWowToken", no),
        ("C_AuctionHouse.QueryBids", nothing),
        ("C_AuctionHouse.IsFavoriteItem", no),
        ("C_AuctionHouse.QueryOwnedAuctions", nothing),
        ("C_AuctionHouse.GetNumOwnedAuctionTypes", zero),
        ("C_AuctionHouse.GetNumBidTypes", zero),
        ("C_AuctionHouse.IsThrottledMessageSystemReady", is_ready),
        ("C_AuctionHouse.HasFavorites", no),
        // The player's auctions not asked for (`QueryOwnedAuctions`): none
        // told, even after a post (recorded).
        ("C_AuctionHouse.GetNumOwnedAuctions", zero),
        ("C_AuctionHouse.GetNumBids", zero),
        ("C_AuctionHouse.SendBrowseQuery", send_browse_query),
        ("C_AuctionHouse.SendSearchQuery", send_search_query),
        ("C_AuctionHouse.SendSellSearchQuery", send_sell_search_query),
        (
            "C_AuctionHouse.RefreshCommoditySearchResults",
            refresh_commodity_search_results,
        ),
        (
            "C_AuctionHouse.RefreshItemSearchResults",
            refresh_item_search_results,
        ),
        ("C_AuctionHouse.GetBrowseResults", get_browse_results),
        ("C_AuctionHouse.HasFullBrowseResults", no),
        (
            "C_AuctionHouse.GetNumCommoditySearchResults",
            get_num_commodity_search_results,
        ),
        (
            "C_AuctionHouse.HasFullCommoditySearchResults",
            has_full_commodity_search_results,
        ),
        (
            "C_AuctionHouse.GetCommoditySearchResultInfo",
            get_commodity_search_result_info,
        ),
        (
            "C_AuctionHouse.GetCommoditySearchResultsQuantity",
            get_commodity_search_results_quantity,
        ),
        (
            "C_AuctionHouse.GetNumItemSearchResults",
            get_num_item_search_results,
        ),
        (
            "C_AuctionHouse.HasFullItemSearchResults",
            has_full_item_search_results,
        ),
        (
            "C_AuctionHouse.GetItemSearchResultInfo",
            get_item_search_result_info,
        ),
        (
            "C_AuctionHouse.StartCommoditiesPurchase",
            start_commodities_purchase,
        ),
        (
            "C_AuctionHouse.ConfirmCommoditiesPurchase",
            confirm_commodities_purchase,
        ),
        ("C_AuctionHouse.PostItem", post_item),
        (
            "C_AuctionHouse.CalculateItemDeposit",
            calculate_item_deposit,
        ),
        ("C_AuctionHouse.CloseAuctionHouse", close_auction_house),
        ("C_AuctionHouse.FavoritesAreAvailable", yes),
        ("C_Item.LockItem", lock_item),
        ("C_Item.UnlockItem", unlock_item),
    ]
}
