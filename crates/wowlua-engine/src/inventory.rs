//! Equipping, unequipping, deleting and using items, as the client
//! recorded a shaman's (`/stdtest engine scenario items`, a ping near
//! 0.3 s). The server answers each two round trips later (0.57 to 0.84 s
//! recorded: it follows the ping).
//!
//! - Right-clicking a bag item that is worn (`UseContainerItem`): it is
//!   picked up and let go at once, staying locked; the server swaps it
//!   with what the slot held (a two-handed weapon with an off-hand worn
//!   and no free bag slot: "Inventory is full.", unlocked).
//! - Dragging to or from the gear (the cursor): let go at once, the
//!   server's answer the same.
//! - The swap's answer: the equipped spells' changes (once per item
//!   moved), the bag's update (and the target slot's unlock), the armor's
//!   `UNIT_RESISTANCES` (twice), the durability's, the equipped spells'
//!   again, `UNIT_INVENTORY_CHANGED`, the gear slot's unlock (filled), the
//!   group buffs' changes, `PLAYER_EQUIPMENT_CHANGED(slot, empty)`,
//!   `BAG_UPDATE_DELAYED`, the model and portrait; a moment later the
//!   portrait (and an armor's model) again, `PORTRAITS_UPDATED()`.
//! - Clicking the world with an item: `DELETE_ITEM_CONFIRM(name, quality,
//!   bonding, questWarn)` (Blizzard's popup); `DeleteCursorItem()`: let go,
//!   locked again, the server removes it.
//! - Using a consumable with a `spell` (food): the cast at once
//!   (`CURRENT_SPELL_CAST_CHANGED`), the server's casts (its `castSpells`
//!   first, triggered), the category's cooldowns, a moment later one fewer.
//!
//! Unverified: rings and trinkets (the first slot), a one-handed weapon
//! with a two-handed one worn, bags as gear, using an item with a target.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::character::{self, Held};
use crate::cursor::{self, Source};
use crate::units::{self, PLAYER};
use crate::{engine, engine_mut, widgets};

/// The portrait's second update and the portraits' after the swap
/// (recorded).
const PORTRAIT_AFTER: f64 = 0.042;
const PORTRAITS_AFTER: f64 = 0.05;
/// The item's count after its use (recorded).
const USED_AFTER: f64 = 0.455;
/// A cast item's effect after its success (recorded: the loading screen).
const EFFECT_AFTER: f64 = 0.145;

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

/// The server's answer: two round trips (recorded 0.57 to 0.84 s at a
/// 0.3 s ping).
fn server(vm: &Vm) -> f64 {
    2.0 * crate::clock::server_delay(vm)
}

fn later(vm: &mut Vm, after: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, after, native);
}

/// The gear slot an item goes to (`equipLoc`), none for one not worn.
pub fn slot_for(vm: &mut Vm, id: i64) -> Option<i64> {
    let loc = match character::item_field(vm, id, "equipLoc") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => return None,
    };
    Some(match loc.as_str() {
        "INVTYPE_HEAD" => 1,
        "INVTYPE_NECK" => 2,
        "INVTYPE_SHOULDER" => 3,
        "INVTYPE_BODY" => 4,
        "INVTYPE_CHEST" | "INVTYPE_ROBE" => 5,
        "INVTYPE_WAIST" => 6,
        "INVTYPE_LEGS" => 7,
        "INVTYPE_FEET" => 8,
        "INVTYPE_WRIST" => 9,
        "INVTYPE_HAND" => 10,
        "INVTYPE_FINGER" => 11,
        "INVTYPE_TRINKET" => 13,
        "INVTYPE_CLOAK" => 15,
        "INVTYPE_WEAPON" | "INVTYPE_2HWEAPON" | "INVTYPE_WEAPONMAINHAND" => 16,
        "INVTYPE_WEAPONOFFHAND" | "INVTYPE_SHIELD" | "INVTYPE_HOLDABLE" => 17,
        "INVTYPE_RANGED" | "INVTYPE_RANGEDRIGHT" | "INVTYPE_RELIC" => 18,
        "INVTYPE_TABARD" => 19,
        _ => return None,
    })
}

fn two_handed(vm: &mut Vm, id: i64) -> bool {
    matches!(character::item_field(vm, id, "equipLoc"), Value::Str(s)
        if vm.str_bytes(s) == b"INVTYPE_2HWEAPON")
}

/// Armor (its class 4, a subclass beyond miscellaneous): the resistances
/// change with it (recorded: boots yes, a shirt no).
fn armor(vm: &mut Vm, id: i64) -> bool {
    matches!(character::item_field(vm, id, "classID"), Value::Num(c) if c == 4.0)
        && matches!(character::item_field(vm, id, "subclassID"), Value::Num(s) if s > 0.0)
}

fn free_bag_slot(vm: &Vm) -> bool {
    engine(vm)
        .world
        .character
        .bags
        .iter()
        .filter(|(b, _)| character::carried(**b))
        .any(|(_, b)| (1..=b.slots).any(|s| !b.items.contains_key(&s)))
}

/// `UseContainerItem(bag, slot)` away from a vendor: equip a worn item,
/// use a consumable. Whether it did either.
pub fn use_container_item(vm: &mut Vm, bag: i64, slot: usize) -> bool {
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
        return true;
    }
    if let Some(target) = slot_for(vm, held.id) {
        cursor::hold_for_server(vm, Source::Bag(bag, slot));
        let after = server(vm);
        later(
            vm,
            after,
            "inventoryEquip",
            equipped,
            vec![
                Value::Num(bag as f64),
                Value::Num(slot as f64),
                Value::Num(target as f64),
                Value::Bool(false),
            ],
        );
        return true;
    }
    if matches!(character::item_field(vm, held.id, "spell"), Value::Num(_)) {
        use_item(vm, bag, slot, held.id);
        return true;
    }
    false
}

/// The cursor's item put to or from the gear: let go, the server's.
pub fn cursor_move(vm: &mut Vm, from: Source, to: Source) {
    cursor::release_keeping_lock(vm);
    let (bag, slot, gear, target_locked) = match (&from, &to) {
        (Source::Bag(b, s), Source::Equipped(g)) => (*b, *s, *g, false),
        (Source::Equipped(g), Source::Bag(b, s)) => {
            // The bag slot waits for the item (unlocked by the answer).
            if let Some(h) = engine_mut(vm)
                .world
                .character
                .bags
                .get_mut(b)
                .and_then(|bag| bag.items.get_mut(s))
            {
                h.locked = true;
            }
            (*b, *s, *g, true)
        }
        _ => return,
    };
    let after = server(vm);
    later(
        vm,
        after,
        "inventoryEquip",
        equipped,
        vec![
            Value::Num(bag as f64),
            Value::Num(slot as f64),
            Value::Num(gear as f64),
            Value::Bool(target_locked || matches!(from, Source::Equipped(_))),
        ],
    );
}

fn num(v: Value) -> f64 {
    match v {
        Value::Num(n) => n,
        _ => 0.0,
    }
}

/// The server's swap of a bag slot and a gear slot (either may be empty).
fn equipped(vm: &mut Vm, c: NativeCall) -> Ret {
    let bag = num(c.upvals[0]) as i64;
    let slot = num(c.upvals[1]) as usize;
    let gear = num(c.upvals[2]) as i64;
    let unequipping = c.upvals[3].truthy();
    let (in_bag, worn) = {
        let ch = &engine(vm).world.character;
        (
            ch.bags.get(&bag).and_then(|b| b.items.get(&slot)).cloned(),
            ch.equipment.get(&gear).cloned(),
        )
    };
    // A two-handed weapon with an off-hand worn needs a free slot for it.
    if let Some(h) = &in_bag
        && !unequipping
        && two_handed(vm, h.id)
        && engine(vm).world.character.equipment.contains_key(&17)
        && !free_bag_slot(vm)
    {
        cursor::lock(vm, &Source::Bag(bag, slot), false);
        let code = vm.get_global("LE_GAME_ERR_INV_FULL");
        let text = vm.get_global("ERR_INV_FULL");
        fire(vm, "UI_ERROR_MESSAGE", &[code, text]);
        return Ok(vec![]);
    }
    let moved = usize::from(in_bag.is_some() && !unequipping) + usize::from(worn.is_some());
    let armor_changed = in_bag.as_ref().is_some_and(|h| armor(vm, h.id))
        || worn.as_ref().is_some_and(|h| armor(vm, h.id));
    // The swap.
    {
        let ch = &mut engine_mut(vm).world.character;
        let into_bag: Option<Held> = worn.clone().map(|mut h| {
            h.locked = false;
            h
        });
        let into_gear: Option<Held> = if unequipping {
            None
        } else {
            in_bag.clone().map(|mut h| {
                h.locked = false;
                h
            })
        };
        if let Some(b) = ch.bags.get_mut(&bag) {
            match into_bag {
                Some(h) => {
                    b.items.insert(slot, h);
                }
                None if !unequipping => {
                    b.items.remove(&slot);
                }
                None => {}
            }
        }
        match into_gear {
            Some(h) => {
                ch.equipment.insert(gear, h);
            }
            None => {
                ch.equipment.remove(&gear);
            }
        }
    }
    let empty = !engine(vm).world.character.equipment.contains_key(&gear);
    for _ in 0..moved {
        character::equipped_spells_changed(vm);
    }
    if unequipping {
        let args = [Value::Num(bag as f64), Value::Num(slot as f64)];
        fire(vm, "ITEM_LOCK_CHANGED", &args);
        fire(vm, "ITEM_UNLOCKED", &args);
    }
    character::bag_updated(vm, bag);
    if armor_changed {
        units::fire_unit(vm, "UNIT_RESISTANCES", PLAYER, &[]);
        units::fire_unit(vm, "UNIT_RESISTANCES", PLAYER, &[]);
    }
    fire(vm, "UPDATE_INVENTORY_DURABILITY", &[]);
    for _ in 0..moved {
        character::equipped_spells_changed(vm);
    }
    units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", PLAYER, &[]);
    if !empty {
        let args = [Value::Num(gear as f64), Value::Nil];
        fire(vm, "ITEM_LOCK_CHANGED", &args);
        fire(vm, "ITEM_UNLOCKED", &args);
    }
    character::group_buffs_changed(vm);
    fire(
        vm,
        "PLAYER_EQUIPMENT_CHANGED",
        &[Value::Num(gear as f64), Value::Bool(empty)],
    );
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    units::fire_unit(vm, "UNIT_MODEL_CHANGED", PLAYER, &[]);
    units::fire_unit(vm, "UNIT_PORTRAIT_UPDATE", PLAYER, &[]);
    later(
        vm,
        PORTRAIT_AFTER,
        "inventoryPortrait",
        portrait,
        vec![Value::Bool(armor_changed)],
    );
    Ok(vec![])
}

fn portrait(vm: &mut Vm, c: NativeCall) -> Ret {
    units::fire_unit(vm, "UNIT_PORTRAIT_UPDATE", PLAYER, &[]);
    if c.upvals[0].truthy() {
        units::fire_unit(vm, "UNIT_MODEL_CHANGED", PLAYER, &[]);
    }
    later(
        vm,
        PORTRAITS_AFTER - PORTRAIT_AFTER,
        "inventoryPortraits",
        portraits,
        vec![],
    );
    Ok(vec![])
}

fn portraits(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "PORTRAITS_UPDATED", &[]);
    Ok(vec![])
}

/// `e.clickWorld()`: the user clicks the world: an item on the cursor is
/// to be deleted (`DELETE_ITEM_CONFIRM(name, quality, bonding,
/// questWarn)`, recorded; Blizzard shows its popup).
pub fn control_click_world(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((held, _)) = cursor::held_item(vm) else {
        return Ok(vec![]);
    };
    let name = character::item_field(vm, held.id, "name");
    let quality = match character::item_field(vm, held.id, "quality") {
        Value::Nil => Value::Num(1.0),
        q => q,
    };
    fire(
        vm,
        "DELETE_ITEM_CONFIRM",
        &[name, quality, Value::Num(0.0), Value::Num(0.0), Value::Nil],
    );
    Ok(vec![])
}

/// `DeleteCursorItem()`: let go (unlocked, the grid hidden), locked again
/// while the server removes it.
fn delete_cursor_item(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((_, source)) = cursor::held_item(vm) else {
        return Ok(vec![]);
    };
    cursor::lock(vm, &source, false);
    cursor::release_keeping_lock(vm);
    cursor::lock(vm, &source, true);
    let Source::Bag(bag, slot) = source else {
        return Ok(vec![]);
    };
    let after = server(vm);
    later(
        vm,
        after,
        "inventoryDeleted",
        deleted,
        vec![Value::Num(bag as f64), Value::Num(slot as f64)],
    );
    Ok(vec![])
}

fn deleted(vm: &mut Vm, c: NativeCall) -> Ret {
    let bag = num(c.upvals[0]) as i64;
    let slot = num(c.upvals[1]) as usize;
    if let Some(b) = engine_mut(vm).world.character.bags.get_mut(&bag) {
        b.items.remove(&slot);
    }
    character::equipped_spells_changed(vm);
    units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", PLAYER, &[]);
    character::bag_updated(vm, bag);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    Ok(vec![])
}

/// An item used: the cast at once (one with a cast time: its cooldown
/// foretold); the server's casts and cooldowns; one fewer a moment later.
fn use_item(vm: &mut Vm, bag: i64, slot: usize, id: i64) {
    fire(vm, "CURRENT_SPELL_CAST_CHANGED", &[Value::Bool(false)]);
    if matches!(character::item_field(vm, id, "castTime"), Value::Num(_)) {
        crate::actions::start_global_cooldown(vm);
        cast_cooldown(vm, id, false);
        let after = crate::clock::server_delay(vm);
        later(
            vm,
            after,
            "inventoryCastStart",
            cast_started,
            vec![Value::Num(id as f64)],
        );
        return;
    }
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    let after = server(vm);
    later(
        vm,
        after,
        "inventoryUsed",
        used,
        vec![
            Value::Num(bag as f64),
            Value::Num(slot as f64),
            Value::Num(id as f64),
        ],
    );
}

/// A cast's cooldown told: the bars', the spell's (its category, the
/// global cooldown's 133; the item's at the end), the forms'.
fn cast_cooldown(vm: &mut Vm, id: i64, done: bool) {
    let spell = num(character::item_field(vm, id, "spell"));
    let category = num(character::item_field(vm, id, "cooldownCategory"));
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    let cd = |item: Value| {
        [
            Value::Num(spell),
            Value::Nil,
            Value::Num(category),
            Value::Num(crate::actions::GLOBAL_COOLDOWN),
            item,
        ]
    };
    if done {
        fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
        for shared in spell_list(vm, id, "sharedCooldowns") {
            fire(
                vm,
                "SPELL_UPDATE_COOLDOWN",
                &[
                    Value::Num(shared),
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                ],
            );
        }
        fire(vm, "SPELL_UPDATE_COOLDOWN", &cd(Value::Num(id as f64)));
    }
    fire(vm, "SPELL_UPDATE_COOLDOWN", &cd(Value::Nil));
    fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
}

/// The server started the cast: `SENT`, `START` (its cast bar), the
/// cooldown; done after its cast time.
fn cast_started(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(c.upvals[0]) as i64;
    let spell = num(character::item_field(vm, id, "spell"));
    let cast_time = num(character::item_field(vm, id, "castTime"));
    let guid = crate::casting::cast_guid(vm, 3, spell);
    let n = engine(vm).units.casting.next;
    let bar = format!("CastBar-{n:016X}");
    let player = vm.str("player");
    let g = vm.str(&guid);
    fire(
        vm,
        "UNIT_SPELLCAST_SENT",
        &[player, Value::Nil, g, Value::Num(spell)],
    );
    let name = character::item_field(vm, id, "name");
    let icon = character::item_field(vm, id, "icon");
    crate::casting::begin_cast(vm, spell, name, icon, cast_time, &guid, &bar);
    let b = vm.str(&bar);
    units::fire_unit(
        vm,
        "UNIT_SPELLCAST_START",
        PLAYER,
        &[g, Value::Num(spell), b],
    );
    fire(vm, "CURRENT_SPELL_CAST_CHANGED", &[Value::Bool(false)]);
    cast_cooldown(vm, id, false);
    later(
        vm,
        cast_time,
        "inventoryCastDone",
        cast_done,
        vec![Value::Num(id as f64), g, b],
    );
    Ok(vec![])
}

/// The cast done: `SUCCEEDED`, `STOP`, the cooldowns (the shared ones',
/// the item's); its effect a moment later (a Hearthstone's: the loading
/// screen to `teleport`'s map).
fn cast_done(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(c.upvals[0]) as i64;
    let (g, b) = (c.upvals[1], c.upvals[2]);
    let spell = num(character::item_field(vm, id, "spell"));
    crate::casting::end_cast(vm);
    units::fire_unit(
        vm,
        "UNIT_SPELLCAST_SUCCEEDED",
        PLAYER,
        &[g, Value::Num(spell), b],
    );
    units::fire_unit(
        vm,
        "UNIT_SPELLCAST_STOP",
        PLAYER,
        &[g, Value::Num(spell), b],
    );
    fire(vm, "CURRENT_SPELL_CAST_CHANGED", &[Value::Bool(false)]);
    // The spell's aura on the player (the Hearthstone's Honorless Target,
    // recorded).
    if let Value::Table(aura) = character::item_field(vm, id, "aura") {
        let source = vm.str("source");
        let player = vm.str("player");
        let _ = vm.heap.table_mut(aura).set(source, player);
        crate::auras::apply(vm, PLAYER, aura);
    }
    cast_cooldown(vm, id, true);
    let place = character::item_field(vm, id, "teleport");
    if let Value::Table(_) = place {
        later(
            vm,
            EFFECT_AFTER,
            "inventoryTeleport",
            teleported,
            vec![place],
        );
    }
    Ok(vec![])
}

/// The Hearthstone's place (`teleport`: moveTo's table and the continent's
/// `map`): the loading screen.
fn teleported(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let k = vm.str("map");
    let map = num(vm.heap.table(t).get(k));
    crate::login::teleport(vm, map, Some(t));
    Ok(vec![])
}

fn spell_list(vm: &mut Vm, id: i64, key: &str) -> Vec<f64> {
    let Value::Table(t) = character::item_field(vm, id, key) else {
        return vec![];
    };
    let n = vm.heap.table(t).border();
    (1..=n)
        .filter_map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Num(s) => Some(s),
            _ => None,
        })
        .collect()
}

fn spells_of(vm: &mut Vm, id: i64) -> Vec<f64> {
    let Value::Table(t) = character::item_field(vm, id, "castSpells") else {
        return vec![];
    };
    let n = vm.heap.table(t).border();
    (1..=n)
        .filter_map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Num(s) => Some(s),
            _ => None,
        })
        .collect()
}

fn used(vm: &mut Vm, c: NativeCall) -> Ret {
    let (bag, slot, id) = (
        num(c.upvals[0]) as i64,
        num(c.upvals[1]) as usize,
        num(c.upvals[2]) as i64,
    );
    let spell = num(character::item_field(vm, id, "spell"));
    let category = num(character::item_field(vm, id, "cooldownCategory"));
    let triggered = spells_of(vm, id);
    let guid = crate::casting::cast_guid(vm, 4, spell);
    let player = vm.str("player");
    let g = vm.str(&guid);
    fire(
        vm,
        "UNIT_SPELLCAST_SENT",
        &[player, Value::Nil, g, Value::Num(spell)],
    );
    // The action slots holding the item, and for food any food (recorded:
    // eating a souffle, the jerky's slots changed).
    let food = |vm: &mut Vm, item: i64| {
        matches!(character::item_field(vm, item, "classID"), Value::Num(c) if c == 0.0)
            && matches!(character::item_field(vm, item, "subclassID"), Value::Num(s) if s == 5.0)
    };
    let eating = food(vm, id);
    let held: Vec<(i64, i64)> = engine(vm)
        .world
        .character
        .actions
        .iter()
        .filter_map(|(s, a)| match (a.kind.as_str(), a.id) {
            ("item", Value::Num(n)) => Some((*s, n as i64)),
            _ => None,
        })
        .collect();
    let slots: Vec<i64> = held
        .into_iter()
        .filter(|(_, item)| *item == id || (eating && food(vm, *item)))
        .map(|(s, _)| s)
        .collect();
    for s in &slots {
        fire(vm, "ACTIONBAR_SLOT_CHANGED", &[Value::Num(*s as f64)]);
    }
    for t in &triggered {
        let tg = crate::casting::cast_guid(vm, 15, *t);
        let tg = vm.str(&tg);
        units::fire_unit(
            vm,
            "UNIT_SPELLCAST_SUCCEEDED",
            PLAYER,
            &[tg, Value::Num(*t), Value::Nil],
        );
    }
    fire(vm, "CURRENT_SPELL_CAST_CHANGED", &[Value::Bool(false)]);
    let g = vm.str(&guid);
    units::fire_unit(
        vm,
        "UNIT_SPELLCAST_SUCCEEDED",
        PLAYER,
        &[g, Value::Num(spell), Value::Nil],
    );
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
    let cd = |s: f64, item: Value| {
        [
            Value::Num(s),
            Value::Nil,
            Value::Num(category),
            Value::Num(0.0),
            item,
        ]
    };
    for t in &triggered {
        fire(vm, "SPELL_UPDATE_COOLDOWN", &cd(*t, Value::Nil));
    }
    fire(
        vm,
        "SPELL_UPDATE_COOLDOWN",
        &cd(spell, Value::Num(id as f64)),
    );
    fire(vm, "SPELL_UPDATE_COOLDOWN", &cd(spell, Value::Nil));
    later(
        vm,
        USED_AFTER,
        "inventoryUsedUp",
        used_up,
        vec![
            Value::Num(bag as f64),
            Value::Num(slot as f64),
            Value::Num(id as f64),
        ],
    );
    Ok(vec![])
}

fn used_up(vm: &mut Vm, c: NativeCall) -> Ret {
    let (bag, slot, id) = (
        num(c.upvals[0]) as i64,
        num(c.upvals[1]) as usize,
        num(c.upvals[2]) as i64,
    );
    if let Some(b) = engine_mut(vm).world.character.bags.get_mut(&bag) {
        let gone = match b.items.get_mut(&slot) {
            Some(h) => {
                h.count -= 1.0;
                h.count <= 0.0
            }
            None => false,
        };
        if gone {
            b.items.remove(&slot);
        }
    }
    character::equipped_spells_changed(vm);
    character::bag_updated(vm, bag);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![("DeleteCursorItem", delete_cursor_item)]
}
