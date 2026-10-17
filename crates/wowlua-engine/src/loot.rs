//! Looting a corpse, as the client recorded the player's with autoloot
//! (`/stdtest engine scenario combat`, a ping near 0.3 s): `player:loot(
//! corpse, { auto? })` opens it a round trip later (`LOOT_READY(auto)`,
//! `LOOT_OPENED(auto, false)`, `LOOT_READY(auto)`), the slots answer
//! (`GetNumLootItems`, `GetLootSlotInfo`, `GetLootSlotLink`,
//! `GetLootSlotType`, `GetLootSourceInfo`); `LootSlot(i)` (autoloot asks
//! every slot) is answered a round trip later: `LOOT_SLOT_CLEARED(i)`, the
//! item's `ITEM_PUSH(bag, icon)` and "You receive loot". The last slot
//! taken: the corpse untargeted, `LOOT_CLOSED()` twice, the bags' updates
//! (`ITEM_COUNT_CHANGED` for a stack grown), the last item's push,
//! `BAG_NEW_ITEMS_UPDATED()` for a new slot, `UNIT_LOOT(guid, false)`.
//! Unverified: that interleaving beyond this recording's two items, money,
//! a loot window closed before it is empty.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// An item a corpse holds.
#[derive(Clone, Debug)]
pub struct LootItem {
    pub id: i64,
    pub count: f64,
    pub taken: bool,
}

/// The loot window.
#[derive(Default)]
pub struct Loot {
    /// The corpse looted, when the window is open.
    pub from: Option<UnitId>,
    pub auto: bool,
    pub items: Vec<LootItem>,
    /// Items taken whose bag update waits for the window to close.
    pub pushed: Vec<(i64, f64, i64, bool)>,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
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
    let later = vm.native_with(
        "lootOpened",
        opened as NativeFn,
        vec![Value::Num(corpse.0 as f64), Value::Bool(auto)],
    );
    crate::clock::after_engine(vm, delay, later);
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
        l.auto = auto;
        l.items = items;
        l.pushed.clear();
    }
    fire(vm, "LOOT_READY", &[Value::Bool(auto)]);
    fire(vm, "LOOT_OPENED", &[Value::Bool(auto), Value::Bool(false)]);
    fire(vm, "LOOT_READY", &[Value::Bool(auto)]);
    if auto {
        let n = engine(vm).world.loot.items.len();
        for i in 1..=n {
            request(vm, i);
        }
    }
    Ok(vec![])
}

/// A slot asked for: the server answers a round trip later.
fn request(vm: &mut Vm, slot: usize) {
    let delay = crate::clock::server_delay(vm);
    let later = vm.native_with(
        "lootTaken",
        taken as NativeFn,
        vec![Value::Num(slot as f64)],
    );
    crate::clock::after_engine(vm, delay, later);
}

/// The item's arrival told: `ITEM_PUSH(bag, icon)` and "You receive loot".
fn push(vm: &mut Vm, id: i64, count: f64, bag: i64) {
    let icon = crate::character::item_field(vm, id, "icon");
    fire(vm, "ITEM_PUSH", &[Value::Num(bag as f64), icon]);
    let link = crate::character::link_of_id(vm, id)
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        })
        .unwrap_or_default();
    let text = if count > 1.0 {
        format!("You receive loot: {link}x{count}.")
    } else {
        format!("You receive loot: {link}.")
    };
    let sender = {
        let p = &engine(vm).units.list[units::PLAYER.0];
        match &p.last_name {
            Some(last) => format!("{} {last}", p.name),
            None => p.name.clone(),
        }
    };
    crate::chat::game_message(vm, "CHAT_MSG_LOOT", &text, Some(&sender));
}

/// The server took a slot: it clears; the item goes to the bags.
fn taken(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(slot) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let slot = slot as usize;
    let Some(item) = engine(vm).world.loot.items.get(slot - 1).cloned() else {
        return Ok(vec![]);
    };
    if item.taken || engine(vm).world.loot.from.is_none() {
        return Ok(vec![]);
    }
    engine_mut(vm).world.loot.items[slot - 1].taken = true;
    fire(vm, "LOOT_SLOT_CLEARED", &[Value::Num(slot as f64)]);
    let placed = crate::character::add_to_bags(vm, item.id, item.count);
    let (bag, new_slot) = placed.unwrap_or((0, false));
    let last = engine(vm).world.loot.items.iter().all(|i| i.taken);
    if !last {
        push(vm, item.id, item.count, bag);
        engine_mut(vm)
            .world
            .loot
            .pushed
            .push((item.id, item.count, bag, new_slot));
        return Ok(vec![]);
    }
    // The last: the corpse emptied, the window closed, the bags told.
    let corpse = engine_mut(vm).world.loot.from.take();
    if let Some(corpse) = corpse {
        engine_mut(vm).units.list[corpse.0].loot.clear();
        if engine(vm).units.target == Some(corpse) {
            units::clear_target(vm);
        }
    }
    fire(vm, "LOOT_CLOSED", &[]);
    fire(vm, "LOOT_CLOSED", &[]);
    let pushed = std::mem::take(&mut engine_mut(vm).world.loot.pushed);
    for (id, _, bag, _) in &pushed {
        crate::character::bag_updated(vm, *bag);
        fire(vm, "BAG_UPDATE_DELAYED", &[]);
        fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(*id as f64)]);
    }
    crate::character::bag_updated(vm, bag);
    push(vm, item.id, item.count, bag);
    if new_slot {
        fire(vm, "BAG_NEW_ITEMS_UPDATED", &[]);
    }
    if let Some(corpse) = corpse {
        let g = engine(vm).units.list[corpse.0].guid.clone();
        let g = vm.str(&g);
        fire(vm, "UNIT_LOOT", &[g, Value::Bool(false)]);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
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
/// locked, quest item, quest, active quest, coin (recorded for items).
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
        Value::Bool(false),
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

/// `CloseLoot()`: the window closes (`LOOT_CLOSED`; unverified once).
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
