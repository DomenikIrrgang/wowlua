//! Looting a corpse, as the client recorded the player's with autoloot
//! (`/stdtest engine scenario combat` and `quest`, a ping near 0.3 s):
//! `player:loot(corpse, { auto? })` opens it a round trip later
//! (`UNIT_FLAGS`, `LOOT_READY(auto)`, `LOOT_OPENED(auto, false)`,
//! `LOOT_READY(auto)`), the slots answer (`GetNumLootItems`,
//! `GetLootSlotInfo`, `GetLootSlotLink`, `GetLootSlotType`,
//! `GetLootSourceInfo`). `LootSlot(i)` (autoloot asks every slot, one
//! after another) is answered by the server in three packets:
//!
//! - the item: `LOOT_SLOT_CLEARED(i)` twice, the item's `ITEM_PUSH(bag,
//!   icon)` and "You receive loot", `BAG_NEW_ITEMS_UPDATED()` for a new
//!   slot (a quest's item: `QUEST_WATCH_UPDATE` before, its count after);
//!   the last slot clears once, the corpse is untargeted and
//!   `LOOT_CLOSED()` fires twice, before the push;
//! - the bags, a moment later (shared by the items pushed meanwhile): for
//!   a stack grown the equipped spells' changes and its bag's update, for
//!   a new slot its bag's update; the corpse emptied: `UNIT_LOOT(guid,
//!   false)`; `BAG_UPDATE_DELAYED()`, `ITEM_COUNT_CHANGED` for each stack
//!   grown;
//! - a new slot's item data, later still (shared too): till then its slot
//!   is taken but reads empty; then the equipped spells', the bag's and
//!   the inventory's updates (`UNIT_INVENTORY_CHANGED`).
//!
//! `CloseLoot()` (the player walking off): `LOOT_CLOSED()` once; the slots
//! asked for still arrive (no `LOOT_SLOT_CLEARED`), the corpse stays
//! targeted.
//!
//! Unverified: the packets' spacing (recorded: the bags 0.06 to 0.25 s
//! after the item, the item data 0.17 to 1.18 s; one value each), money,
//! a slot left on a corpse closed early.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// The first slot's answer after the window opened, beyond a round trip,
/// and the next ones' spacing (autoloot, recorded).
const FIRST_SLOT_AFTER: f64 = 0.22;
const NEXT_SLOT_AFTER: f64 = 0.15;
/// The bags' packet after an item's, and the new items' data's.
const BAGS_AFTER: f64 = 0.08;
const ITEM_DATA_AFTER: f64 = 0.5;

/// An item a corpse holds.
#[derive(Clone, Debug)]
pub struct LootItem {
    pub id: i64,
    pub count: f64,
    pub taken: bool,
}

/// An item placed, its bags' packet waiting.
#[derive(Clone, Debug)]
pub struct Placed {
    pub id: i64,
    pub bag: i64,
    /// Onto a stack (else a new slot).
    pub stacked: bool,
}

/// The loot window.
#[derive(Default)]
pub struct Loot {
    /// The corpse looted, when the window is open.
    pub from: Option<UnitId>,
    /// The corpse the slots come from (also once the window closed).
    pub corpse: Option<UnitId>,
    pub auto: bool,
    pub items: Vec<LootItem>,
    /// The bags' packet's items, and whether it is on its way.
    pub bags: Vec<Placed>,
    pub bags_due: bool,
    /// The corpse emptied: its `UNIT_LOOT(guid, false)` with the bags.
    pub emptied: Option<UnitId>,
    /// New slots whose item data is on its way: bag, slot, item.
    pub arriving: Vec<(i64, usize, i64)>,
    pub arriving_due: bool,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn later(vm: &mut Vm, seconds: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, seconds, native);
}

/// `player:loot(corpse, { auto? })`: the player loots a corpse.
pub fn control_loot(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    if me != units::PLAYER {
        return Err(vm.error("loot: the player's"));
    }
    let Some(corpse) = units::handle_of(vm, c.arg(2)) else {
        return Err(vm.error("loot: a corpse to loot expected"));
    };
    let auto = match c.arg(3) {
        Value::Table(t) => {
            let k = vm.str("auto");
            vm.heap.table(t).get(k).truthy()
        }
        _ => false,
    };
    let delay = crate::clock::server_delay(vm);
    later(
        vm,
        delay,
        "lootOpened",
        opened,
        vec![Value::Num(corpse.0 as f64), Value::Bool(auto)],
    );
    Ok(vec![])
}

/// The server opens the loot: the corpse's items in the window.
fn opened(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(id), Value::Bool(auto)) = (c.upvals[0], c.upvals[1]) else {
        return Ok(vec![]);
    };
    let corpse = UnitId(id as usize);
    let items: Vec<LootItem> = engine(vm).units.list[corpse.0]
        .loot
        .iter()
        .map(|(id, count)| LootItem {
            id: *id,
            count: *count,
            taken: false,
        })
        .collect();
    {
        let l = &mut engine_mut(vm).world.loot;
        l.from = Some(corpse);
        l.corpse = Some(corpse);
        l.auto = auto;
        l.items = items;
    }
    // Looting: the player's flags change (recorded just before).
    units::fire_unit(vm, "UNIT_FLAGS", units::PLAYER, &[]);
    fire(vm, "LOOT_READY", &[Value::Bool(auto)]);
    fire(vm, "LOOT_OPENED", &[Value::Bool(auto), Value::Bool(false)]);
    fire(vm, "LOOT_READY", &[Value::Bool(auto)]);
    if auto {
        let n = engine(vm).world.loot.items.len();
        let delay = crate::clock::server_delay(vm);
        for i in 1..=n {
            let after = delay + FIRST_SLOT_AFTER + NEXT_SLOT_AFTER * (i - 1) as f64;
            later(vm, after, "lootTaken", taken, vec![Value::Num(i as f64)]);
        }
    }
    Ok(vec![])
}

/// A slot asked for: the server answers a round trip later.
fn request(vm: &mut Vm, slot: usize) {
    let delay = crate::clock::server_delay(vm);
    later(vm, delay, "lootTaken", taken, vec![Value::Num(slot as f64)]);
}

/// The item's arrival told: `ITEM_PUSH(bag, icon)` and "You receive loot",
/// a new slot's `BAG_NEW_ITEMS_UPDATED()` (a quest's objective's:
/// `QUEST_WATCH_UPDATE` before, its count after).
fn push(vm: &mut Vm, id: i64, count: f64, bag: i64, new_slot: bool) {
    crate::quests::before_item(vm, id);
    let icon = crate::character::item_field(vm, id, "icon");
    fire(vm, "ITEM_PUSH", &[Value::Num(bag as f64), icon]);
    let link = crate::character::link_of_id(vm, id)
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        })
        .unwrap_or_default();
    // The client's own format (camelot's without a period).
    let format = |vm: &mut Vm, name: &str, default: &str| match vm.get_global(name) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => default.to_string(),
    };
    let text = if count > 1.0 {
        format(vm, "LOOT_ITEM_SELF_MULTIPLE", "You receive loot: %sx%d")
            .replacen("%s", &link, 1)
            .replacen("%d", &format!("{count}"), 1)
    } else {
        format(vm, "LOOT_ITEM_SELF", "You receive loot: %s").replacen("%s", &link, 1)
    };
    let sender = player_sender(vm);
    crate::chat::game_message(vm, "CHAT_MSG_LOOT", &text, Some(&sender));
    if new_slot {
        fire(vm, "BAG_NEW_ITEMS_UPDATED", &[]);
    }
    crate::quests::after_item(vm, id);
}

/// The player as a loot message's sender: the full name.
pub fn player_sender(vm: &Vm) -> String {
    let p = &engine(vm).units.list[units::PLAYER.0];
    match &p.last_name {
        Some(last) => format!("{} {last}", p.name),
        None => p.name.clone(),
    }
}

/// The server took a slot: it clears (the window open); the item goes to
/// the bags.
fn taken(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(slot) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let slot = slot as usize;
    let Some(item) = engine(vm).world.loot.items.get(slot - 1).cloned() else {
        return Ok(vec![]);
    };
    if item.taken {
        return Ok(vec![]);
    }
    engine_mut(vm).world.loot.items[slot - 1].taken = true;
    let last = engine(vm).world.loot.items.iter().all(|i| i.taken);
    let open = engine(vm).world.loot.from.is_some();
    let corpse = engine(vm).world.loot.corpse;
    if open && last {
        // The last: the corpse emptied, the window closed.
        fire(vm, "LOOT_SLOT_CLEARED", &[Value::Num(slot as f64)]);
        engine_mut(vm).world.loot.from = None;
        if corpse.is_some() && engine(vm).units.target == corpse {
            units::clear_target(vm);
        }
        fire(vm, "LOOT_CLOSED", &[]);
        fire(vm, "LOOT_CLOSED", &[]);
    } else if open {
        fire(vm, "LOOT_SLOT_CLEARED", &[Value::Num(slot as f64)]);
        fire(vm, "LOOT_SLOT_CLEARED", &[Value::Num(slot as f64)]);
    }
    let placed = crate::character::push_to_bags(vm, item.id, item.count);
    let (bag, new_slot) = placed.unwrap_or((0, None));
    push(vm, item.id, item.count, bag, new_slot.is_some());
    if last && let Some(corpse) = corpse {
        engine_mut(vm).units.list[corpse.0].loot.clear();
        engine_mut(vm).world.loot.emptied = Some(corpse);
    }
    {
        let l = &mut engine_mut(vm).world.loot;
        l.bags.push(Placed {
            id: item.id,
            bag,
            stacked: new_slot.is_none(),
        });
    }
    if !engine(vm).world.loot.bags_due {
        engine_mut(vm).world.loot.bags_due = true;
        later(vm, BAGS_AFTER, "lootBags", bags_told, vec![]);
    }
    if let Some(slot) = new_slot {
        engine_mut(vm)
            .world
            .loot
            .arriving
            .push((bag, slot, item.id));
        if !engine(vm).world.loot.arriving_due {
            engine_mut(vm).world.loot.arriving_due = true;
            later(
                vm,
                ITEM_DATA_AFTER,
                "lootItemData",
                item_data_arrived,
                vec![],
            );
        }
    }
    Ok(vec![])
}

/// The bags' packet: each item's bag updated (a stack grown: the equipped
/// spells' changes first, its count after), the corpse emptied.
fn bags_told(vm: &mut Vm, _c: NativeCall) -> Ret {
    let placed = {
        let l = &mut engine_mut(vm).world.loot;
        l.bags_due = false;
        std::mem::take(&mut l.bags)
    };
    for p in placed.iter().filter(|p| p.stacked) {
        crate::character::equipped_spells_changed(vm);
        crate::character::bag_updated(vm, p.bag);
    }
    for p in placed.iter().filter(|p| !p.stacked) {
        crate::character::bag_updated(vm, p.bag);
    }
    if let Some(corpse) = engine_mut(vm).world.loot.emptied.take() {
        let g = engine(vm).units.list[corpse.0].guid.clone();
        let g = vm.str(&g);
        fire(vm, "UNIT_LOOT", &[g, Value::Bool(false)]);
        // No longer looting: the flags a round trip later (recorded).
        let delay = crate::clock::server_delay(vm);
        later(vm, delay, "lootFlags", loot_flags, vec![]);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    for p in placed.iter().filter(|p| p.stacked) {
        fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(p.id as f64)]);
    }
    Ok(vec![])
}

/// The new slots' item data: each slot reads its item; the equipped
/// spells', the bag's and the inventory's updates (a quest's item: the
/// log's update after).
fn item_data_arrived(vm: &mut Vm, _c: NativeCall) -> Ret {
    let arriving = {
        let l = &mut engine_mut(vm).world.loot;
        l.arriving_due = false;
        std::mem::take(&mut l.arriving)
    };
    let mut quest = false;
    for (bag, slot, id) in arriving {
        crate::character::arrived(vm, bag, slot);
        crate::character::equipped_spells_changed(vm);
        crate::character::bag_updated(vm, bag);
        units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", units::PLAYER, &[]);
        quest |= crate::quests::is_quest_item(vm, id);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    if quest {
        fire(vm, "QUEST_LOG_UPDATE", &[]);
    }
    Ok(vec![])
}

fn loot_flags(vm: &mut Vm, _c: NativeCall) -> Ret {
    units::fire_unit(vm, "UNIT_FLAGS", units::PLAYER, &[]);
    Ok(vec![])
}

fn open_item(vm: &Vm, c: &NativeCall) -> Option<LootItem> {
    let l = &engine(vm).world.loot;
    l.from?;
    let Value::Num(i) = c.arg(1) else { return None };
    l.items
        .get((i as usize).checked_sub(1)?)
        .filter(|it| !it.taken)
        .cloned()
}

fn get_num_loot_items(vm: &mut Vm, _c: NativeCall) -> Ret {
    let l = &engine(vm).world.loot;
    let n = if l.from.is_some() { l.items.len() } else { 0 };
    Ok(vec![Value::Num(n as f64)])
}

/// `GetLootSlotInfo(slot)`: icon, name, quantity, currency, quality,
/// locked, quest item (an item a quest in the log asks for: recorded),
/// quest, active quest, coin (recorded for items).
fn get_loot_slot_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(item) = open_item(vm, &c) else {
        return Ok(vec![]);
    };
    let icon = crate::character::item_field(vm, item.id, "icon");
    let name = crate::character::item_field(vm, item.id, "name");
    let quality = crate::character::item_field(vm, item.id, "quality");
    Ok(vec![
        icon,
        name,
        Value::Num(item.count),
        Value::Nil,
        if quality.is_nil() {
            Value::Num(1.0)
        } else {
            quality
        },
        Value::Bool(false),
        Value::Bool(crate::quests::is_quest_item(vm, item.id)),
        Value::Nil,
        Value::Nil,
        Value::Bool(false),
    ])
}

fn get_loot_slot_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(item) = open_item(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(vec![
        crate::character::link_of_id(vm, item.id).unwrap_or(Value::Nil),
    ])
}

/// `GetLootSlotType(slot)`: 1 for an item (0 none).
fn get_loot_slot_type(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Num(if open_item(vm, &c).is_some() {
        1.0
    } else {
        0.0
    })])
}

/// `GetLootSourceInfo(slot)`: the corpse's GUID and the count.
fn get_loot_source_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(item) = open_item(vm, &c) else {
        return Ok(vec![]);
    };
    let Some(corpse) = engine(vm).world.loot.from else {
        return Ok(vec![]);
    };
    let g = engine(vm).units.list[corpse.0].guid.clone();
    Ok(vec![vm.str(&g), Value::Num(item.count)])
}

fn is_fishing_loot(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `LootSlot(slot)`: the slot asked for.
fn loot_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    if let (Some(_), Value::Num(i)) = (open_item(vm, &c), c.arg(1)) {
        request(vm, i as usize);
    }
    Ok(vec![])
}

fn loot_slot_has_item(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(open_item(vm, &c).is_some())])
}

/// `CloseLoot()`: the window closes (`LOOT_CLOSED` once, recorded: the
/// player walking off); the slots asked for still arrive.
fn close_loot(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine_mut(vm).world.loot.from.take().is_some() {
        fire(vm, "LOOT_CLOSED", &[]);
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetNumLootItems", get_num_loot_items),
        ("GetLootSlotInfo", get_loot_slot_info),
        ("GetLootSlotLink", get_loot_slot_link),
        ("GetLootSlotType", get_loot_slot_type),
        ("GetLootSourceInfo", get_loot_source_info),
        ("IsFishingLoot", is_fishing_loot),
        ("LootSlot", loot_slot),
        ("LootSlotHasItem", loot_slot_has_item),
        ("CloseLoot", close_loot),
    ]
}
