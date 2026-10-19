//! Items' answers no other module gives: `C_Item`'s by item, by location
//! and by item GUID, `C_Container`'s, `C_PaperDollInfo`'s,
//! `C_EquipmentSet`'s, the rest of `C_Bank`'s, and the systems WoW Forever
//! lacks (item upgrades, sockets, Azerite, scrapping, item interactions),
//! from the same bags, gear, cursor, merchant and bank the other modules
//! keep (`character.rs`, `cursor.rs`, `merchant.rs`, `bank.rs`) and the
//! item data (`gamedata.rs`).
//!
//! An item GUID is the engine's own (`Item-<realm>-0-<16 hex>`): its place
//! (bag and slot, or gear slot) and its item ID, so a GUID names an item
//! where it was given out and nothing once it moved (unverified: the
//! client's item GUIDs are the server's and follow the item).
//!
//! Unverified unless noted: everything here but the armor effectiveness,
//! the off-hand's weapon and the quality colors. Equipment sets are not
//! modelled (none saved; the client's recorded character had two). Settings
//! of the bags (sorting direction, autosort, slot flags) are accepted and
//! not kept.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::character::{self, Held};
use crate::cursor::Source;
use crate::units::{self, PLAYER};
use crate::{engine, widgets};

// ---------------------------------------------------------------- helpers

fn num(vm: &Vm, v: Value) -> Option<f64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn list(vm: &mut Vm, values: &[Value]) -> TableRef {
    let t = vm.new_table();
    for (i, v) in values.iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num((i + 1) as f64), *v);
    }
    t
}

/// The item an `itemInfo` names (an ID, a link, a cached item's name).
fn item_of(vm: &mut Vm, v: Value) -> Option<i64> {
    crate::gamedata::item_id(vm, v)
}

fn data_num(vm: &mut Vm, id: i64, key: &str) -> Option<f64> {
    match character::item_field(vm, id, key) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// An item's data when the client has it loaded (`cached`).
fn cached(vm: &mut Vm, id: i64) -> Option<TableRef> {
    let t = crate::gamedata::item_data(vm, id)?;
    field(vm, t, "cached").truthy().then_some(t)
}

fn equip_loc(vm: &mut Vm, id: i64) -> Option<String> {
    let v = character::item_field(vm, id, "equipLoc");
    text(vm, v)
}

/// `Enum.InventoryType` by the item's `equipLoc` (the documentation's
/// enumeration; its `INVTYPE_` names as `GetItemInfo` gives them).
const INVENTORY_TYPES: &[(&str, f64)] = &[
    ("INVTYPE_NON_EQUIP_IGNORE", 0.0),
    ("INVTYPE_HEAD", 1.0),
    ("INVTYPE_NECK", 2.0),
    ("INVTYPE_SHOULDER", 3.0),
    ("INVTYPE_BODY", 4.0),
    ("INVTYPE_CHEST", 5.0),
    ("INVTYPE_WAIST", 6.0),
    ("INVTYPE_LEGS", 7.0),
    ("INVTYPE_FEET", 8.0),
    ("INVTYPE_WRIST", 9.0),
    ("INVTYPE_HAND", 10.0),
    ("INVTYPE_FINGER", 11.0),
    ("INVTYPE_TRINKET", 12.0),
    ("INVTYPE_WEAPON", 13.0),
    ("INVTYPE_SHIELD", 14.0),
    ("INVTYPE_RANGED", 15.0),
    ("INVTYPE_CLOAK", 16.0),
    ("INVTYPE_2HWEAPON", 17.0),
    ("INVTYPE_BAG", 18.0),
    ("INVTYPE_TABARD", 19.0),
    ("INVTYPE_ROBE", 20.0),
    ("INVTYPE_WEAPONMAINHAND", 21.0),
    ("INVTYPE_WEAPONOFFHAND", 22.0),
    ("INVTYPE_HOLDABLE", 23.0),
    ("INVTYPE_AMMO", 24.0),
    ("INVTYPE_THROWN", 25.0),
    ("INVTYPE_RANGEDRIGHT", 26.0),
    ("INVTYPE_QUIVER", 27.0),
    ("INVTYPE_RELIC", 28.0),
];

/// An item's `Enum.InventoryType`: 0 for one not worn, none for an item
/// without data.
fn inventory_type(vm: &mut Vm, id: i64) -> Option<f64> {
    crate::gamedata::item_data(vm, id)?;
    let loc = equip_loc(vm, id).unwrap_or_default();
    Some(
        INVENTORY_TYPES
            .iter()
            .find(|(n, _)| *n == loc)
            .map_or(0.0, |(_, t)| *t),
    )
}

/// Whether item `id` may be worn in gear slot `slot`: its own slot, the
/// second ring or trinket slot, a one-hander in the off hand (unverified:
/// dual wielding's skill not asked).
fn fits(vm: &mut Vm, id: i64, slot: i64) -> bool {
    let Some(home) = crate::inventory::slot_for(vm, id) else {
        return false;
    };
    home == slot
        || (slot == 12 && home == 11)
        || (slot == 14 && home == 13)
        || (slot == 17 && equip_loc(vm, id).as_deref() == Some("INVTYPE_WEAPON"))
}

/// A place an item is: a bag slot or a gear slot.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Bag(i64, usize),
    Worn(i64),
}

impl Place {
    fn source(self) -> Source {
        match self {
            Place::Bag(b, s) => Source::Bag(b, s),
            Place::Worn(s) => Source::Equipped(s),
        }
    }
}

/// The place an `ItemLocation` names.
fn place_of(vm: &mut Vm, location: Value) -> Option<Place> {
    let Value::Table(t) = vm.hidden(location) else {
        return None;
    };
    let worn = field(vm, t, "equipmentSlotIndex");
    if let Some(slot) = num(vm, worn) {
        return Some(Place::Worn(slot as i64));
    }
    let bag = field(vm, t, "bagID");
    let slot = field(vm, t, "slotIndex");
    Some(Place::Bag(num(vm, bag)? as i64, num(vm, slot)? as usize))
}

/// The item at a place (a bag slot's whose data has arrived).
fn held(vm: &Vm, place: Place) -> Option<Held> {
    let ch = &engine(vm).world.character;
    match place {
        Place::Bag(b, s) => ch
            .bags
            .get(&b)?
            .items
            .get(&s)
            .filter(|h| !h.arriving)
            .cloned(),
        Place::Worn(s) => ch.equipment.get(&s).cloned(),
    }
}

fn held_at(vm: &mut Vm, location: Value) -> Option<(Place, Held)> {
    let place = place_of(vm, location)?;
    held(vm, place).map(|h| (place, h))
}

/// The first place in the bags (the backpack first), else the gear, that
/// holds item `id`.
fn find(vm: &Vm, id: i64, gear: bool) -> Option<Place> {
    let ch = &engine(vm).world.character;
    let in_bags = ch
        .bags
        .iter()
        .filter(|(b, _)| character::carried(**b))
        .find_map(|(b, bag)| {
            bag.items
                .iter()
                .find(|(_, h)| h.id == id && !h.arriving)
                .map(|(s, _)| Place::Bag(*b, *s))
        });
    if in_bags.is_some() || !gear {
        return in_bags;
    }
    ch.equipment
        .iter()
        .find(|(_, h)| h.id == id)
        .map(|(s, _)| Place::Worn(*s))
}

/// The engine's GUID of the item at a place.
fn guid_of(vm: &Vm, place: Place, id: i64) -> String {
    let realm = engine(vm).units.list[PLAYER.0]
        .guid
        .split('-')
        .nth(1)
        .unwrap_or("0000")
        .to_string();
    let (code, slot) = match place {
        Place::Bag(b, s) => ((b + 0x100) as u64 & 0xFFFF, s as u64),
        Place::Worn(s) => (0xFFFF, s as u64),
    };
    format!(
        "Item-{realm}-0-{code:04X}{:04X}{:08X}",
        slot & 0xFFFF,
        id as u64 & 0xFFFF_FFFF
    )
}

/// The place and item ID an engine GUID names.
fn parse_guid(vm: &Vm, v: Value) -> Option<(Place, i64)> {
    let guid = text(vm, v)?;
    if !guid.starts_with("Item-") {
        return None;
    }
    let hex = guid.rsplit('-').next()?;
    if hex.len() != 16 {
        return None;
    }
    let code = u64::from_str_radix(&hex[0..4], 16).ok()?;
    let slot = u64::from_str_radix(&hex[4..8], 16).ok()?;
    let id = u64::from_str_radix(&hex[8..16], 16).ok()? as i64;
    let place = if code == 0xFFFF {
        Place::Worn(slot as i64)
    } else {
        Place::Bag(code as i64 - 0x100, slot as usize)
    };
    Some((place, id))
}

/// The item a GUID names, still where the GUID was given.
fn held_by_guid(vm: &Vm, v: Value) -> Option<(Place, Held)> {
    let (place, id) = parse_guid(vm, v)?;
    held(vm, place).filter(|h| h.id == id).map(|h| (place, h))
}

/// An `ItemLocation` object (Blizzard's constructors when loaded, else
/// the plain fields).
fn location_object(vm: &mut Vm, place: Place) -> Result<Value, wowlua_vm::LuaError> {
    let (ctor, args) = match place {
        Place::Bag(b, s) => (
            "CreateFromBagAndSlot",
            vec![Value::Num(b as f64), Value::Num(s as f64)],
        ),
        Place::Worn(s) => ("CreateFromEquipmentSlot", vec![Value::Num(s as f64)]),
    };
    if let Value::Table(class) = vm.get_global("ItemLocation") {
        let f = field(vm, class, ctor);
        if !f.is_nil() {
            let mut all = vec![Value::Table(class)];
            all.extend(args);
            if let Some(v @ Value::Table(_)) = vm.call(f, &all)?.first().copied() {
                return Ok(v);
            }
        }
    }
    let t = vm.new_table();
    match place {
        Place::Bag(b, s) => {
            set(vm, t, "bagID", Value::Num(b as f64));
            set(vm, t, "slotIndex", Value::Num(s as f64));
        }
        Place::Worn(s) => set(vm, t, "equipmentSlotIndex", Value::Num(s as f64)),
    }
    Ok(Value::Table(t))
}

fn player_level(vm: &Vm) -> f64 {
    engine(vm).units.list[PLAYER.0].level
}

fn rgb(c: [f64; 3]) -> Vec<Value> {
    c.iter().map(|v| Value::Num(*v)).collect()
}

// ----------------------------------------------------- generic answers

/// Accepted, nothing done: actions of systems WoW Forever lacks, and
/// settings the engine does not keep (unverified).
fn accept(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// False: what an `Is…`/`Can…` asks of a system WoW Forever lacks or the
/// engine does not model (unverified).
fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// 0: a count or level of a system WoW Forever lacks (unverified).
fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0)])
}

/// Nil: a documented nilable answer when there is nothing (unverified).
fn nil(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

/// No values: a function documented as possibly returning nothing, when
/// there is nothing (unverified).
fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn empty_table(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

/// The documentation's empty values (each structure's fields' zeros,
/// falses and empty strings): a structure of a system WoW Forever lacks
/// (unverified).
fn documented(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(Value::Num(i)) = c.upvals.first().copied() {
        let api = engine(vm).api;
        if let Some(f) = api.functions.get(i as usize) {
            return Ok(crate::defaults(vm, api, &f.doc));
        }
    }
    Ok(vec![])
}

// ------------------------------------------------------- C_Item by item

/// `C_Item.GetItemQualityColor(quality)`: the quality's color (the
/// client's recorded `qualityColors`, `facts.rs`) and its hex
/// (`ffRRGGBB`); white for an unknown quality (unverified).
fn get_item_quality_color(vm: &mut Vm, c: NativeCall) -> Ret {
    let color = num(vm, c.arg(1))
        .filter(|q| *q >= 0.0)
        .and_then(|q| crate::facts::QUALITY_COLORS.get(q as usize))
        .map_or([1.0, 1.0, 1.0], |c| [c[0], c[1], c[2]]);
    let byte = |v: f64| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let hex = format!(
        "ff{:02x}{:02x}{:02x}",
        byte(color[0]),
        byte(color[1]),
        byte(color[2])
    );
    let mut out = rgb(color);
    out.push(vm.str(&hex));
    Ok(out)
}

/// `GetItemLevelColor()`: the character sheet's item level color, the
/// epic quality's #a335ee as the client's floats, each channel times a
/// float 1/255 (recorded for a level 21 character in greens and blues,
/// 0.20784315466881 for 53; unverified: whether it ever changes).
fn get_item_level_color(_vm: &mut Vm, _c: NativeCall) -> Ret {
    let channel = |v: u8| f64::from(f32::from(v) * (1.0f32 / 255.0));
    Ok(rgb([channel(163), channel(53), channel(238)]))
}

/// `C_Item.GetItemSpell(item)`: the item's use spell (its data's `spell`,
/// as `inventory.rs` casts it): its name and ID; nothing without one.
fn get_item_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let Some(spell) = data_num(vm, id, "spell") else {
        return Ok(vec![]);
    };
    let name = match crate::gamedata::spell_data(vm, spell as i64) {
        Some(t) => field(vm, t, "name"),
        None => Value::Nil,
    };
    Ok(vec![name, Value::Num(spell)])
}

/// `C_Item.GetDetailedItemLevelInfo(item)`: a loaded item's level, not a
/// preview, its base level the same (no upgrades or scaling here);
/// nothing before its data loads.
fn get_detailed_item_level_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let Some(t) = cached(vm, id) else {
        return Ok(vec![]);
    };
    let level = field(vm, t, "itemLevel");
    if level.is_nil() {
        return Ok(vec![]);
    }
    Ok(vec![level, Value::Bool(false), level])
}

/// `C_Item.IsUsableItem(item)`: an item with a use spell or one worn, at
/// or above its required level; never short of mana (unverified: class
/// and skill requirements not asked).
fn is_usable_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let unusable = || Ok(vec![Value::Bool(false), Value::Bool(false)]);
    let Some(id) = item_of(vm, c.arg(1)) else {
        return unusable();
    };
    if crate::gamedata::item_data(vm, id).is_none() {
        return unusable();
    }
    let kind = data_num(vm, id, "spell").is_some() || crate::inventory::slot_for(vm, id).is_some();
    let min = data_num(vm, id, "minLevel").unwrap_or(0.0);
    let usable = kind && min <= player_level(vm);
    Ok(vec![Value::Bool(usable), Value::Bool(false)])
}

/// `C_Item.GetItemCooldown(item)`: its running cooldown's start and
/// duration (the cooldowns `character.rs` keeps), the timer enabled.
fn get_item_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    let cd = item_of(vm, c.arg(1)).and_then(|id| character::item_cooldown(vm, id));
    let (start, duration) = cd.unwrap_or((0.0, 0.0));
    Ok(vec![
        Value::Num(start),
        Value::Num(duration),
        Value::Bool(true),
    ])
}

/// The item's `stats` from its data (`{ ITEM_MOD_STAMINA_SHORT = 5, … }`),
/// as pairs.
fn stats_of(vm: &mut Vm, id: i64) -> Vec<(Value, f64)> {
    let Some(t) = cached(vm, id) else {
        return vec![];
    };
    let Value::Table(stats) = field(vm, t, "stats") else {
        return vec![];
    };
    vm.heap
        .table(stats)
        .entries()
        .filter_map(|(k, v)| match v {
            Value::Num(n) => Some((k, n)),
            _ => None,
        })
        .collect()
}

/// `C_Item.GetItemStats(link)`: a loaded item's stats (its data's
/// `stats`, else none); nothing before its data loads.
fn get_item_stats(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    if cached(vm, id).is_none() {
        return Ok(vec![]);
    }
    let stats = stats_of(vm, id);
    let t = vm.new_table();
    for (k, v) in stats {
        let _ = vm.heap.table_mut(t).set(k, Value::Num(v));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_Item.GetItemStatDelta(link1, link2)`: the first item's stats less
/// the second's, the unchanged left out (unverified: the direction).
fn get_item_stat_delta(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(a), Some(b)) = (item_of(vm, c.arg(1)), item_of(vm, c.arg(2))) else {
        return Ok(vec![]);
    };
    let first = stats_of(vm, a);
    let second = stats_of(vm, b);
    let t = vm.new_table();
    for (k, v) in &first {
        let other = second
            .iter()
            .find(|(k2, _)| k2.raw_eq(*k))
            .map_or(0.0, |(_, n)| *n);
        if v - other != 0.0 {
            let _ = vm.heap.table_mut(t).set(*k, Value::Num(v - other));
        }
    }
    for (k, v) in &second {
        if !first.iter().any(|(k1, _)| k1.raw_eq(*k)) && *v != 0.0 {
            let _ = vm.heap.table_mut(t).set(*k, Value::Num(-v));
        }
    }
    Ok(vec![Value::Table(t)])
}

/// `C_Item.IsEquippedItem(item)`: the player wears it.
fn is_equipped_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let worn = item_of(vm, c.arg(1)).is_some_and(|id| {
        engine(vm)
            .world
            .character
            .equipment
            .values()
            .any(|h| h.id == id)
    });
    Ok(vec![Value::Bool(worn)])
}

/// `C_Item.PickupItem(item)`: the first of it in the bags (else worn) onto
/// the cursor, as `PickupContainerItem` takes it.
fn pickup_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    if let Some(place) = find(vm, id, true) {
        crate::cursor::pickup_item_at(vm, place.source());
    }
    Ok(vec![])
}

/// `C_Item.EquipItemByName(item [, slot])`: the first of it in the bags
/// worn, as a right-click wears it (`inventory.rs`; unverified: the slot
/// asked for is not followed).
fn equip_item_by_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    if crate::inventory::slot_for(vm, id).is_none() {
        return Ok(vec![]);
    }
    if let Some(Place::Bag(b, s)) = find(vm, id, false) {
        crate::inventory::use_container_item(vm, b, s);
    }
    Ok(vec![])
}

/// `C_Item.UseItemByName(item [, target])`: the first of it in the bags
/// used, as a right-click uses it (unverified: the target).
fn use_item_by_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    if let Some(Place::Bag(b, s)) = find(vm, id, false) {
        crate::inventory::use_container_item(vm, b, s);
    }
    Ok(vec![])
}

/// `C_Item.IsConsumableItem(item)`: a consumable (`Enum.ItemClass`
/// `Consumable`, 0).
fn is_consumable_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let yes = item_of(vm, c.arg(1)).is_some_and(|id| data_num(vm, id, "classID") == Some(0.0));
    Ok(vec![Value::Bool(yes)])
}

/// `C_Item.IsCurrentItem(item)`: the player casting its use spell.
fn is_current_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let spell = item_of(vm, c.arg(1)).and_then(|id| data_num(vm, id, "spell"));
    let current = spell.is_some_and(|s| {
        engine(vm)
            .units
            .casting
            .casts
            .get(&PLAYER)
            .is_some_and(|cast| cast.spell_id == s)
    });
    Ok(vec![Value::Bool(current)])
}

/// `C_Item.GetItemInventoryTypeByID(item)`: its `Enum.InventoryType` (0
/// for one not worn); nil for an item without data.
fn get_item_inventory_type_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = item_of(vm, c.arg(1)).and_then(|id| inventory_type(vm, id));
    Ok(vec![t.map_or(Value::Nil, Value::Num)])
}

/// `C_Item.GetItemInventorySlotKey(inventoryType)`: its `INVTYPE_` name;
/// empty for one unknown.
fn get_item_inventory_slot_key(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = num(vm, c.arg(1)).unwrap_or(-1.0);
    let name = INVENTORY_TYPES
        .iter()
        .find(|(_, n)| *n == t)
        .map_or("", |(name, _)| *name);
    Ok(vec![vm.str(name)])
}

/// `C_Item.IsEquippableItem(item)`: it has a gear slot (bags too).
fn is_equippable_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let yes = item_of(vm, c.arg(1))
        .and_then(|id| inventory_type(vm, id))
        .unwrap_or(0.0)
        > 0.0;
    Ok(vec![Value::Bool(yes)])
}

/// `C_Item.IsDressableItemByID(item)`: worn where it shows on the model
/// (not a neck, ring, trinket, bag, ammo, quiver or relic).
fn is_dressable_item_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = item_of(vm, c.arg(1))
        .and_then(|id| inventory_type(vm, id))
        .unwrap_or(0.0) as i64;
    let shows = matches!(
        t,
        1 | 3
            | 4
            | 5
            | 6
            | 7
            | 8
            | 9
            | 10
            | 13
            | 14
            | 15
            | 16
            | 17
            | 19
            | 20
            | 21
            | 22
            | 23
            | 25
            | 26
    );
    Ok(vec![Value::Bool(shows)])
}

/// `bindType`, by item.
fn bind_type(vm: &mut Vm, v: Value) -> Option<f64> {
    let id = item_of(vm, v)?;
    data_num(vm, id, "bindType")
}

/// `C_Item.IsItemBindToAccount(item)`: bound to the account
/// (`Enum.ItemBind` `ToWoWAccount` 7, `ToBnetAccount` 8).
fn is_item_bind_to_account(vm: &mut Vm, c: NativeCall) -> Ret {
    let b = bind_type(vm, c.arg(1));
    Ok(vec![Value::Bool(matches!(b, Some(7.0 | 8.0)))])
}

/// `C_Item.IsItemBindToAccountUntilEquip(item)`: `Enum.ItemBind`
/// `ToBnetAccountUntilEquipped` (9).
fn is_item_bind_to_account_until_equip(vm: &mut Vm, c: NativeCall) -> Ret {
    let b = bind_type(vm, c.arg(1));
    Ok(vec![Value::Bool(b == Some(9.0))])
}

/// `C_Item.GetItemIDForItemInfo(item)`: its ID.
fn get_item_id_for_item_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match item_of(vm, c.arg(1)) {
        Some(id) => vec![Value::Num(id as f64)],
        None => vec![],
    })
}

/// `C_Item.GetItemCreationContext(item)`: its ID and no creation context.
fn get_item_creation_context(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match item_of(vm, c.arg(1)) {
        Some(id) => vec![Value::Num(id as f64), vm.str("")],
        None => vec![],
    })
}

/// `C_Item.GetItemFamily(item)`: no bag family (0) for an item with data,
/// nil otherwise (unverified: special bags' and their goods' families).
fn get_item_family(vm: &mut Vm, c: NativeCall) -> Ret {
    let known = match item_of(vm, c.arg(1)) {
        Some(id) => crate::gamedata::item_data(vm, id).is_some(),
        None => false,
    };
    Ok(vec![if known { Value::Num(0.0) } else { Value::Nil }])
}

/// `C_Item.GetItemUniquenessByID(item)`: not unique, no limit category,
/// for an item with data; nothing otherwise.
fn get_item_uniqueness_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let known = match item_of(vm, c.arg(1)) {
        Some(id) => crate::gamedata::item_data(vm, id).is_some(),
        None => false,
    };
    Ok(if known {
        vec![Value::Bool(false), Value::Nil, Value::Nil, Value::Nil]
    } else {
        vec![]
    })
}

/// `C_Item.GetDelvePreviewItemQuality(itemID, context)`: the item's own
/// quality (no delves here), poor without data.
fn get_delve_preview_item_quality(vm: &mut Vm, c: NativeCall) -> Ret {
    let q = num(vm, c.arg(1)).and_then(|id| data_num(vm, id as i64, "quality"));
    Ok(vec![Value::Num(q.unwrap_or(0.0))])
}

/// `C_Item.IsHelpfulItem(item)`: one with a use spell (unverified: the
/// spell's target not asked); `IsHarmfulItem`: none here.
fn is_helpful_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let yes = item_of(vm, c.arg(1)).is_some_and(|id| data_num(vm, id, "spell").is_some());
    Ok(vec![Value::Bool(yes)])
}

// --------------------------------------------------- C_Item by location

/// `C_Item.GetCurrentItemLevel(location)`: the item's level there.
fn get_current_item_level(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match held_at(vm, c.arg(1)) {
        Some((_, h)) => character::item_field(vm, h.id, "itemLevel"),
        None => Value::Nil,
    }])
}

/// `C_Item.IsBoundToAccountUntilEquip(location)`: an unbound item bound to
/// the account once worn (`bindType` 9).
fn is_bound_to_account_until_equip(vm: &mut Vm, c: NativeCall) -> Ret {
    let yes = match held_at(vm, c.arg(1)) {
        Some((_, h)) => !h.bound && data_num(vm, h.id, "bindType") == Some(9.0),
        None => false,
    };
    Ok(vec![Value::Bool(yes)])
}

/// `C_Item.GetItemGUID(location)`: the engine's GUID of the item there
/// (see the module); nothing for an empty place.
fn get_item_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((place, h)) = held_at(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let guid = guid_of(vm, place, h.id);
    Ok(vec![vm.str(&guid)])
}

/// `C_Item.GetItemInventoryType(location)`: the item's
/// `Enum.InventoryType`.
fn get_item_inventory_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = match held_at(vm, c.arg(1)) {
        Some((_, h)) => inventory_type(vm, h.id),
        None => None,
    };
    Ok(vec![t.map_or(Value::Nil, Value::Num)])
}

/// `C_Item.GetItemMaxStackSize(location)`: the item's stack size.
fn get_item_max_stack_size(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match held_at(vm, c.arg(1)) {
        Some((_, h)) => character::item_field(vm, h.id, "stackCount"),
        None => Value::Nil,
    }])
}

/// `C_Item.IsItemDataCached(location)`: the item there has its data.
fn is_item_data_cached(vm: &mut Vm, c: NativeCall) -> Ret {
    let yes = match held_at(vm, c.arg(1)) {
        Some((_, h)) => cached(vm, h.id).is_some(),
        None => false,
    };
    Ok(vec![Value::Bool(yes)])
}

/// `C_Item.RequestLoadItemData(location)`: the item there asked for, as
/// `C_Item.RequestLoadItemDataByID` asks (`ITEM_DATA_LOAD_RESULT` later).
fn request_load_item_data(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, h)) = held_at(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    if let Value::Table(ns) = vm.get_global("C_Item") {
        let f = field(vm, ns, "RequestLoadItemDataByID");
        if !f.is_nil() {
            vm.call(f, &[Value::Num(h.id as f64)])?;
        }
    }
    Ok(vec![])
}

// ------------------------------------------------------ C_Item by GUID

/// `C_Item.GetItemLinkByGUID(guid)`: the item's link, nil once it moved.
fn get_item_link_by_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let link = match held_by_guid(vm, c.arg(1)) {
        Some((_, h)) => character::link_of(vm, &h),
        None => None,
    };
    Ok(vec![link.unwrap_or(Value::Nil)])
}

/// `C_Item.GetItemLocation(guid)`: where the item is, as an
/// `ItemLocation`.
fn get_item_location(vm: &mut Vm, c: NativeCall) -> Ret {
    match held_by_guid(vm, c.arg(1)) {
        Some((place, _)) => Ok(vec![location_object(vm, place)?]),
        None => Ok(vec![Value::Nil]),
    }
}

/// `C_Item.GetItemIDByGUID(guid)`: the item's ID.
fn get_item_id_by_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match held_by_guid(vm, c.arg(1)) {
        Some((_, h)) => Value::Num(h.id as f64),
        None => Value::Nil,
    }])
}

/// `C_Item.IsItemGUIDInInventory(guid)`: the item carried or worn.
fn is_item_guid_in_inventory(vm: &mut Vm, c: NativeCall) -> Ret {
    let yes = match held_by_guid(vm, c.arg(1)) {
        Some((Place::Bag(b, _), _)) => character::carried(b),
        Some((Place::Worn(_), _)) => true,
        None => false,
    };
    Ok(vec![Value::Bool(yes)])
}

// ------------------------------------------------------------ containers

fn bag_place(vm: &Vm, c: &NativeCall) -> Option<Place> {
    let bag = num(vm, c.arg(1))? as i64;
    let slot = num(vm, c.arg(2))? as usize;
    Some(Place::Bag(bag, slot))
}

/// `C_Container.GetContainerItemDurability(bag, slot)`: the item's
/// durability, nothing for one without.
fn get_container_item_durability(vm: &mut Vm, c: NativeCall) -> Ret {
    let d = bag_place(vm, &c)
        .and_then(|p| held(vm, p))
        .and_then(|h| h.durability);
    Ok(match d {
        Some((d, m)) => vec![Value::Num(d), Value::Num(m)],
        None => vec![],
    })
}

/// `C_Container.HasContainerItem(bag, slot)`.
fn has_container_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let has = bag_place(vm, &c).and_then(|p| held(vm, p)).is_some();
    Ok(vec![Value::Bool(has)])
}

/// `C_Container.GetContainerFreeSlots(bag)`: the empty slots' indexes;
/// nothing for no bag there.
fn get_container_free_slots(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(bag) = num(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let free: Vec<Value> = match engine(vm).world.character.bags.get(&(bag as i64)) {
        Some(b) => (1..=b.slots)
            .filter(|s| !b.items.contains_key(s))
            .map(|s| Value::Num(s as f64))
            .collect(),
        None => return Ok(vec![]),
    };
    Ok(vec![Value::Table(list(vm, &free))])
}

/// The Hearthstone.
const HEARTHSTONE: i64 = 6948;

/// `C_Container.PlayerHasHearthstone()`: the Hearthstone's ID when it is
/// in the bags (unverified: toys that hearth).
fn player_has_hearthstone(vm: &mut Vm, _c: NativeCall) -> Ret {
    let has = find(vm, HEARTHSTONE, false).is_some();
    Ok(vec![if has {
        Value::Num(HEARTHSTONE as f64)
    } else {
        Value::Nil
    }])
}

/// `C_Container.UseHearthstone()`: the Hearthstone in the bags used, as a
/// right-click uses it (`inventory.rs`); whether there was one.
fn use_hearthstone(vm: &mut Vm, _c: NativeCall) -> Ret {
    let used = match find(vm, HEARTHSTONE, false) {
        Some(Place::Bag(b, s)) => crate::inventory::use_container_item(vm, b, s),
        _ => false,
    };
    Ok(vec![Value::Bool(used)])
}

/// `C_Container.GetContainerItemEquipmentSetInfo(bag, slot)`: in no set
/// (none saved here).
fn get_container_item_equipment_set_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false), vm.str("")])
}

// ------------------------------------------------------------- the gear

/// The armor's damage reduction against an attacker's level: classic's
/// `armor / (armor + 400 + 85 * level)`, beyond level 59 `85 * 4.5` more
/// per level, at most 75 % (recorded: 1149 armor against level 0 is
/// 0.70318…, as against level 1; 0 armor 0).
fn armor_effectiveness(armor: f64, level: f64) -> f64 {
    if armor <= 0.0 {
        return 0.0;
    }
    let level = level.max(1.0);
    let mut k = 400.0 + 85.0 * level;
    if level > 59.0 {
        k += 4.5 * 85.0 * (level - 59.0);
    }
    (armor / (armor + k)).min(0.75)
}

/// `C_PaperDollInfo.GetArmorEffectiveness(armor, attackerLevel)`.
fn get_armor_effectiveness(vm: &mut Vm, c: NativeCall) -> Ret {
    let armor = num(vm, c.arg(1)).unwrap_or(0.0);
    let level = num(vm, c.arg(2)).unwrap_or(0.0);
    Ok(vec![Value::Num(armor_effectiveness(armor, level))])
}

/// `C_PaperDollInfo.GetArmorEffectivenessAgainstTarget(armor)`: against
/// the target's level (a hidden level's as ten above the player's); nil
/// without a target (recorded; unverified with one).
fn get_armor_effectiveness_against_target(vm: &mut Vm, c: NativeCall) -> Ret {
    let armor = num(vm, c.arg(1)).unwrap_or(0.0);
    let u = &engine(vm).units;
    let Some(target) = units::resolve(u, "target").filter(|t| *t != PLAYER) else {
        return Ok(vec![Value::Nil]);
    };
    let level = match u.list[target.0].level {
        l if l > 0.0 => l,
        _ => player_level(vm) + 10.0,
    };
    Ok(vec![Value::Num(armor_effectiveness(armor, level))])
}

fn worn(vm: &Vm, slot: i64) -> Option<i64> {
    engine(vm)
        .world
        .character
        .equipment
        .get(&slot)
        .map(|h| h.id)
}

/// `C_PaperDollInfo.OffhandHasWeapon()`: a weapon (`Enum.ItemClass`
/// `Weapon`, 2) in the off hand (recorded false without one).
fn offhand_has_weapon(vm: &mut Vm, _c: NativeCall) -> Ret {
    let yes = worn(vm, 17).is_some_and(|id| data_num(vm, id, "classID") == Some(2.0));
    Ok(vec![Value::Bool(yes)])
}

/// `C_PaperDollInfo.OffhandHasShield()`: a shield in the off hand.
fn offhand_has_shield(vm: &mut Vm, _c: NativeCall) -> Ret {
    let yes = worn(vm, 17).is_some_and(|id| equip_loc(vm, id).as_deref() == Some("INVTYPE_SHIELD"));
    Ok(vec![Value::Bool(yes)])
}

/// `C_PaperDollInfo.AmmoNeeded()`: a bow, gun or crossbow in the ranged
/// slot (weapon subclasses 2, 3, 18).
fn ammo_needed(vm: &mut Vm, _c: NativeCall) -> Ret {
    let yes = worn(vm, 18).is_some_and(|id| {
        data_num(vm, id, "classID") == Some(2.0)
            && matches!(data_num(vm, id, "subclassID"), Some(2.0 | 3.0 | 18.0))
    });
    Ok(vec![Value::Bool(yes)])
}

/// `C_PaperDollInfo.IsRangedSlotShown()`: WoW Forever's gear has the
/// ranged slot (recorded: `GetInventorySlotInfo("RANGEDSLOT")` is 18;
/// unverified: this answer).
fn is_ranged_slot_shown(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `C_PaperDollInfo.IsInventorySlotEnabled(slotName)`: the gear's slots
/// yes, the professions' tool and gear slots no (unverified: the client
/// lists them, `GetInventorySlotInfo("Prof0ToolSlot")` 20, recorded, but
/// whether WoW Forever uses them is not).
fn is_inventory_slot_enabled(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text(vm, c.arg(1)).unwrap_or_default().to_ascii_lowercase();
    let profession = ["prof", "cooking", "fishing"]
        .iter()
        .any(|p| name.starts_with(p));
    Ok(vec![Value::Bool(!name.is_empty() && !profession)])
}

/// `C_PaperDollInfo.CursorCanGoInSlot(slot)`: the cursor's item may be
/// worn there.
fn cursor_can_go_in_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    let slot = num(vm, c.arg(1)).unwrap_or(-1.0) as i64;
    let yes = crate::cursor::held_item(vm).is_some_and(|(h, _)| fits(vm, h.id, slot));
    Ok(vec![Value::Bool(yes)])
}

/// `C_PaperDollInfo.CanAutoEquipCursorItem()`: the cursor's item has a
/// gear slot.
fn can_auto_equip_cursor_item(vm: &mut Vm, _c: NativeCall) -> Ret {
    let yes = crate::cursor::held_item(vm)
        .is_some_and(|(h, _)| crate::inventory::slot_for(vm, h.id).is_some());
    Ok(vec![Value::Bool(yes)])
}

/// `EquipCursorItem(slot)`: the cursor's item put on that gear slot, as
/// `PickupInventoryItem` puts it (`cursor.rs`).
fn equip_cursor_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(slot) = num(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    if crate::cursor::held_item(vm).is_some() {
        crate::cursor::pickup_item_at(vm, Source::Equipped(slot as i64));
    }
    Ok(vec![])
}

/// `C_PaperDollInfo.GetStaggerPercentage(unit)`: none (monks' only).
fn get_stagger_percentage(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Nil])
}

/// The client's bit flags of `GetInventoryItemsForSlot`'s keys
/// (Blizzard's `ITEM_INVENTORY_LOCATION_PLAYER`, `_BAGS`, the bag shifted
/// by `ITEM_INVENTORY_BAG_BIT_OFFSET`).
const LOCATION_PLAYER: i64 = 0x0010_0000;
const LOCATION_BAGS: i64 = 0x0020_0000;
const BAG_BIT_OFFSET: i64 = 8;

/// `GetInventoryItemsForSlot(slot, table [, transmog])`: the items that
/// may be worn in the slot, worn and carried, by their location's bit
/// flags, as links (warcraft.wiki.gg; unverified: the bank's).
fn get_inventory_items_for_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    let out = match c.arg(2) {
        Value::Table(t) => t,
        _ => vm.new_table(),
    };
    let Some(slot) = num(vm, c.arg(1)).map(|s| s as i64) else {
        return Ok(vec![Value::Table(out)]);
    };
    let mut found: Vec<(i64, Held)> = Vec::new();
    if let Some(h) = engine(vm).world.character.equipment.get(&slot).cloned() {
        found.push((LOCATION_PLAYER | slot, h));
    }
    let carried: Vec<(i64, usize, Held)> = engine(vm)
        .world
        .character
        .bags
        .iter()
        .filter(|(b, _)| character::carried(**b))
        .flat_map(|(b, bag)| {
            bag.items
                .iter()
                .filter(|(_, h)| !h.arriving)
                .map(move |(s, h)| (*b, *s, h.clone()))
        })
        .collect();
    for (b, s, h) in carried {
        if fits(vm, h.id, slot) {
            found.push((
                LOCATION_PLAYER | LOCATION_BAGS | (b << BAG_BIT_OFFSET) | s as i64,
                h,
            ));
        }
    }
    for (key, h) in found {
        let link = character::link_of(vm, &h).unwrap_or(Value::Nil);
        let _ = vm.heap.table_mut(out).set(Value::Num(key as f64), link);
    }
    Ok(vec![Value::Table(out)])
}

// ------------------------------------------------------- equipment sets

/// `C_EquipmentSet.CanUseEquipmentSets()`: WoW Forever has the equipment
/// manager (recorded: a character with two sets).
fn can_use_equipment_sets(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

// ----------------------------------------------------------------- bank

/// The character's bank (`Enum.BankType.Character`).
const CHARACTER_BANK: f64 = 0.0;
/// The character's first bank tab's bag (`Enum.BagIndex.CharacterBankTab_1`).
const FIRST_TAB: i64 = 6;

/// `C_Bank.FetchPurchasedBankTabIDs(bankType)`: the character's tabs'
/// bags (6 on), as many as `FetchNumPurchasedBankTabs` counts.
fn fetch_purchased_bank_tab_ids(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = if num(vm, c.arg(1)) == Some(CHARACTER_BANK) {
        engine(vm).world.bank.tabs.len()
    } else {
        0
    };
    let ids: Vec<Value> = (0..n)
        .map(|i| Value::Num((FIRST_TAB + i as i64) as f64))
        .collect();
    Ok(vec![Value::Table(list(vm, &ids))])
}

/// `C_Bank.FetchViewableBankTypes()`: the character's at the bank, none
/// away from it (as `C_Bank.CanViewBank`).
fn fetch_viewable_bank_types(vm: &mut Vm, _c: NativeCall) -> Ret {
    let types = if crate::bank::is_open(vm) {
        vec![Value::Num(CHARACTER_BANK)]
    } else {
        vec![]
    };
    Ok(vec![Value::Table(list(vm, &types))])
}

/// `C_Bank.IsItemAllowedInBankType(bankType, location)`: any item in the
/// character's bank, none in the account's (WoW Forever has none).
fn is_item_allowed_in_bank_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let yes = num(vm, c.arg(1)) == Some(CHARACTER_BANK) && held_at(vm, c.arg(2)).is_some();
    Ok(vec![Value::Bool(yes)])
}

/// `C_Bank.UpdateBankTabSettings(bankType, tabID, name, icon, flags)`: the
/// character's tab renamed, its icon and deposit flags set; the server's
/// `BANK_TAB_SETTINGS_UPDATED(0)` a round trip later (as after a purchase,
/// recorded; unverified here).
fn update_bank_tab_settings(vm: &mut Vm, c: NativeCall) -> Ret {
    if num(vm, c.arg(1)) != Some(CHARACTER_BANK) {
        return Ok(vec![]);
    }
    let Some(tab_id) = num(vm, c.arg(2)) else {
        return Ok(vec![]);
    };
    let index = tab_id as i64 - FIRST_TAB;
    let Some(tab) = usize::try_from(index)
        .ok()
        .and_then(|i| engine(vm).world.bank.tabs.get(i).copied())
    else {
        return Ok(vec![]);
    };
    if let Some(name) = text(vm, c.arg(3)) {
        let v = vm.str(&name);
        set(vm, tab, "name", v);
    }
    let icon = match vm.hidden(c.arg(4)) {
        Value::Num(n) => Some(n),
        Value::Str(_) => text(vm, c.arg(4)).and_then(|s| s.parse::<f64>().ok()),
        _ => None,
    };
    if let Some(icon) = icon {
        set(vm, tab, "icon", Value::Num(icon));
    }
    if let Some(flags) = num(vm, c.arg(5)) {
        set(vm, tab, "depositFlags", Value::Num(flags));
    }
    let delay = crate::clock::server_delay(vm);
    let native = vm.native("bankTabSettingsUpdated", bank_tab_settings_updated);
    crate::clock::after_engine(vm, delay, native);
    Ok(vec![])
}

fn bank_tab_settings_updated(vm: &mut Vm, _c: NativeCall) -> Ret {
    widgets::fire_event(
        vm,
        "BANK_TAB_SETTINGS_UPDATED",
        &[Value::Num(CHARACTER_BANK)],
    );
    Ok(vec![])
}

// ------------------------------------------------------------- merchant

/// `C_MerchantFrame.SellAllJunkItems()`: each poor item in the bags sold
/// to the open vendor, as a right-click sells it (`merchant.rs`).
fn sell_all_junk_items(vm: &mut Vm, _c: NativeCall) -> Ret {
    if !engine(vm).world.merchant.open {
        return Ok(vec![]);
    }
    let carried: Vec<(i64, usize, i64)> = engine(vm)
        .world
        .character
        .bags
        .iter()
        .filter(|(b, _)| character::carried(**b))
        .flat_map(|(b, bag)| {
            bag.items
                .iter()
                .filter(|(_, h)| !h.locked && !h.arriving)
                .map(move |(s, h)| (*b, *s, h.id))
        })
        .collect();
    for (b, s, id) in carried {
        if data_num(vm, id, "quality") == Some(0.0) {
            crate::merchant::sell(vm, b, s);
        }
    }
    Ok(vec![])
}

/// The functions with behaviour here.
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        // C_Item by item.
        ("C_Item.GetItemQualityColor", get_item_quality_color),
        ("GetItemLevelColor", get_item_level_color),
        ("C_Item.GetItemSpell", get_item_spell),
        (
            "C_Item.GetDetailedItemLevelInfo",
            get_detailed_item_level_info,
        ),
        ("C_Item.IsUsableItem", is_usable_item),
        ("C_Item.GetItemCooldown", get_item_cooldown),
        ("C_Item.GetItemStats", get_item_stats),
        ("C_Item.GetItemStatDelta", get_item_stat_delta),
        ("C_Item.IsItemInRange", nil),
        ("C_Item.ItemHasRange", no),
        ("C_Item.IsEquippedItem", is_equipped_item),
        ("C_Item.IsItemKeystoneByID", no),
        ("C_Item.PickupItem", pickup_item),
        ("C_Item.EquipItemByName", equip_item_by_name),
        ("C_Item.UseItemByName", use_item_by_name),
        ("C_Item.GetItemGem", nothing),
        ("C_Item.GetItemGemID", nothing),
        ("C_Item.GetItemNumSockets", zero),
        ("C_Item.GetItemNumAddedSockets", zero),
        ("C_Item.GetItemUpgradeInfo", nil),
        ("C_Item.IsConsumableItem", is_consumable_item),
        ("C_Item.IsCurrentItem", is_current_item),
        (
            "C_Item.GetItemInventoryTypeByID",
            get_item_inventory_type_by_id,
        ),
        (
            "C_Item.GetItemInventorySlotKey",
            get_item_inventory_slot_key,
        ),
        ("C_Item.IsEquippableItem", is_equippable_item),
        ("C_Item.IsDressableItemByID", is_dressable_item_by_id),
        ("C_Item.IsItemBindToAccount", is_item_bind_to_account),
        (
            "C_Item.IsItemBindToAccountUntilEquip",
            is_item_bind_to_account_until_equip,
        ),
        ("C_Item.GetItemIDForItemInfo", get_item_id_for_item_info),
        ("C_Item.GetItemCreationContext", get_item_creation_context),
        ("C_Item.GetItemFamily", get_item_family),
        ("C_Item.GetItemUniquenessByID", get_item_uniqueness_by_id),
        ("C_Item.GetItemUniqueness", nothing),
        ("C_Item.GetItemChildInfo", empty_table),
        ("C_Item.GetItemSpecInfo", nothing),
        ("C_Item.GetItemSetInfo", get_item_set_info),
        ("C_Item.GetItemLearnTransmogSet", nil),
        ("C_Item.GetLimitedCurrencyItemInfo", nothing),
        ("C_Item.GetFirstTriggeredSpellForItem", nil),
        ("C_Item.GetSetBonusesForSpecializationByItemID", nothing),
        ("C_Item.GetDelvePreviewItemLink", nil),
        (
            "C_Item.GetDelvePreviewItemQuality",
            get_delve_preview_item_quality,
        ),
        ("C_Item.IsHelpfulItem", is_helpful_item),
        ("C_Item.IsHarmfulItem", no),
        ("C_Item.IsAnimaItemByID", no),
        ("C_Item.IsArtifactPowerItem", no),
        ("C_Item.IsItemSpecificToPlayerClass", no),
        ("C_Item.DoesItemContainSpec", no),
        // C_Item by location.
        ("C_Item.GetCurrentItemLevel", get_current_item_level),
        (
            "C_Item.IsBoundToAccountUntilEquip",
            is_bound_to_account_until_equip,
        ),
        ("C_Item.GetItemGUID", get_item_guid),
        ("C_Item.GetItemInventoryType", get_item_inventory_type),
        ("C_Item.GetItemMaxStackSize", get_item_max_stack_size),
        ("C_Item.IsItemDataCached", is_item_data_cached),
        ("C_Item.RequestLoadItemData", request_load_item_data),
        ("C_Item.CanBeRefunded", no),
        ("C_Item.CanViewItemPowers", no),
        ("C_Item.CanScrapItem", no),
        ("C_Item.IsItemConduit", no),
        ("C_Item.IsItemCorrupted", no),
        ("C_Item.IsItemCorruptionRelated", no),
        ("C_Item.IsItemCorruptionResistant", no),
        ("C_Item.IsItemConvertibleAndValidForPlayer", no),
        ("C_Item.DoesItemMatchTargetEnchantingSpell", no),
        ("C_Item.DoesItemMatchBonusTreeReplacement", no),
        ("C_Item.DoesItemMatchSpellItemCondition", no),
        ("C_Item.DoesItemMatchTrackJump", no),
        ("C_Item.GetItemConversionOutputIcon", nil),
        ("C_Item.GetAppliedItemTransmogInfo", nil),
        ("C_Item.GetCurrentItemTransmogInfo", nil),
        ("C_Item.GetBaseItemTransmogInfo", nil),
        ("C_Item.CanItemTransmogAppearance", documented),
        // C_Item by GUID.
        ("C_Item.GetItemLinkByGUID", get_item_link_by_guid),
        ("C_Item.GetItemLocation", get_item_location),
        ("C_Item.GetItemIDByGUID", get_item_id_by_guid),
        ("C_Item.IsItemGUIDInInventory", is_item_guid_in_inventory),
        ("C_Item.LockItemByGUID", accept),
        ("C_Item.UnlockItemByGUID", accept),
        // C_Item's confirmations and actions (accepted, no effect here).
        ("C_Item.DeleteItem", accept),
        ("C_Item.ConfirmDeleteItem", accept),
        ("C_Item.DropItemOnUnit", accept),
        ("C_Item.ActionBindsItem", accept),
        ("C_Item.BindEnchant", accept),
        ("C_Item.ConfirmBindOnUse", accept),
        ("C_Item.ConfirmNoRefundOnUse", accept),
        ("C_Item.ConfirmOnUse", accept),
        ("C_Item.EndBoundTradeable", accept),
        ("C_Item.EndRefund", accept),
        ("C_Item.ReplaceEnchant", accept),
        ("C_Item.ReplaceTradeEnchant", accept),
        ("C_Item.ReplaceTradeskillEnchant", accept),
        // C_Container.
        ("C_Container.SetItemSearch", accept),
        ("C_Container.SetSortBagsRightToLeft", accept),
        ("C_Container.GetSortBagsRightToLeft", no),
        ("C_Container.SetInsertItemsLeftToRight", accept),
        ("C_Container.GetInsertItemsLeftToRight", no),
        ("C_Container.SetBackpackAutosortDisabled", accept),
        ("C_Container.SetBackpackSellJunkDisabled", accept),
        ("C_Container.SetBankAutosortDisabled", accept),
        ("C_Container.GetBankAutosortDisabled", no),
        ("C_Container.SetBagSlotFlag", accept),
        ("C_Container.SetBagPortraitTexture", accept),
        ("C_Container.ShowContainerSellCursor", accept),
        ("C_Container.SortBags", accept),
        ("C_Container.SortBank", accept),
        ("C_Container.SortBankBags", accept),
        ("C_Container.SortAccountBankBags", accept),
        ("C_Container.SplitContainerItem", accept),
        ("C_Container.ContainerRefundItemPurchase", accept),
        ("C_Container.GetContainerItemPurchaseInfo", nothing),
        ("C_Container.GetContainerItemPurchaseCurrency", nothing),
        ("C_Container.GetContainerItemPurchaseItem", nothing),
        (
            "C_Container.GetContainerItemDurability",
            get_container_item_durability,
        ),
        (
            "C_Container.GetContainerFreeSlots",
            get_container_free_slots,
        ),
        ("C_Container.HasContainerItem", has_container_item),
        ("C_Container.IsBattlePayItem", no),
        ("C_Container.SocketContainerItem", no),
        ("C_Container.GetMaxArenaCurrency", zero),
        ("C_Container.PlayerHasHearthstone", player_has_hearthstone),
        ("C_Container.UseHearthstone", use_hearthstone),
        (
            "C_Container.GetContainerItemEquipmentSetInfo",
            get_container_item_equipment_set_info,
        ),
        // C_EquipmentSet (no sets saved).
        ("C_EquipmentSet.CanUseEquipmentSets", can_use_equipment_sets),
        ("C_EquipmentSet.GetEquipmentSetIDs", empty_table),
        ("C_EquipmentSet.GetNumEquipmentSets", zero),
        ("C_EquipmentSet.GetEquipmentSetInfo", nothing),
        ("C_EquipmentSet.GetEquipmentSetID", nothing),
        ("C_EquipmentSet.GetEquipmentSetAssignedSpec", nothing),
        ("C_EquipmentSet.GetEquipmentSetForSpec", nothing),
        ("C_EquipmentSet.GetItemIDs", nothing),
        ("C_EquipmentSet.GetItemLocations", nothing),
        ("C_EquipmentSet.GetIgnoredSlots", nothing),
        ("C_EquipmentSet.IsSlotIgnoredForSave", no),
        ("C_EquipmentSet.EquipmentSetContainsLockedItems", no),
        ("C_EquipmentSet.UseEquipmentSet", no),
        ("C_EquipmentSet.IgnoreSlotForSave", accept),
        ("C_EquipmentSet.UnignoreSlotForSave", accept),
        ("C_EquipmentSet.ClearIgnoredSlotsForSave", accept),
        ("C_EquipmentSet.CreateEquipmentSet", accept),
        ("C_EquipmentSet.ModifyEquipmentSet", accept),
        ("C_EquipmentSet.SaveEquipmentSet", accept),
        ("C_EquipmentSet.DeleteEquipmentSet", accept),
        ("C_EquipmentSet.PickupEquipmentSet", accept),
        ("C_EquipmentSet.AssignSpecToEquipmentSet", accept),
        ("C_EquipmentSet.UnassignEquipmentSetSpec", accept),
        // C_PaperDollInfo.
        ("C_PaperDollInfo.GetTemporaryEnchantmentInfo", nothing),
        ("C_PaperDollInfo.CancelTemporaryEnchantment", accept),
        (
            "C_PaperDollInfo.GetArmorEffectiveness",
            get_armor_effectiveness,
        ),
        (
            "C_PaperDollInfo.GetArmorEffectivenessAgainstTarget",
            get_armor_effectiveness_against_target,
        ),
        ("C_PaperDollInfo.CursorCanGoInSlot", cursor_can_go_in_slot),
        (
            "C_PaperDollInfo.CanAutoEquipCursorItem",
            can_auto_equip_cursor_item,
        ),
        (
            "C_PaperDollInfo.GetStaggerPercentage",
            get_stagger_percentage,
        ),
        ("C_PaperDollInfo.IsRangedSlotShown", is_ranged_slot_shown),
        ("C_PaperDollInfo.OffhandHasWeapon", offhand_has_weapon),
        ("C_PaperDollInfo.OffhandHasShield", offhand_has_shield),
        ("C_PaperDollInfo.AmmoNeeded", ammo_needed),
        ("C_PaperDollInfo.GetMinItemLevel", nil),
        (
            "C_PaperDollInfo.IsInventorySlotEnabled",
            is_inventory_slot_enabled,
        ),
        ("C_PaperDollInfo.GetInspectGuildInfo", documented),
        (
            "C_PaperDollInfo.GetInspectAzeriteItemEmpoweredChoices",
            nothing,
        ),
        ("C_PaperDollInfo.GetInspectRatedBGBlitzData", documented),
        ("C_PaperDollInfo.GetInspectRatedBGData", documented),
        ("C_PaperDollInfo.GetInspectRatedSoloShuffleData", documented),
        ("EquipCursorItem", equip_cursor_item),
        ("GetInventoryItemsForSlot", get_inventory_items_for_slot),
        // C_Bank.
        ("C_Bank.FetchDepositedMoney", zero),
        ("C_Bank.DepositMoney", accept),
        ("C_Bank.WithdrawMoney", accept),
        ("C_Bank.DoesBankTypeSupportMoneyTransfer", no),
        ("C_Bank.DoesBankTypeSupportAutoDeposit", no),
        ("C_Bank.AutoDepositItemsIntoBank", accept),
        ("C_Bank.ShouldUsePlayerBagsInBank", no),
        ("C_Bank.BankBagTypeAndIDToInvSlot", nil),
        (
            "C_Bank.IsItemAllowedInBankType",
            is_item_allowed_in_bank_type,
        ),
        ("C_Bank.UpdateBankTabSettings", update_bank_tab_settings),
        (
            "C_Bank.FetchPurchasedBankTabIDs",
            fetch_purchased_bank_tab_ids,
        ),
        ("C_Bank.FetchViewableBankTypes", fetch_viewable_bank_types),
        // The merchant and the cursor.
        ("C_MerchantFrame.SellAllJunkItems", sell_all_junk_items),
        ("GetMerchantItemCostItem", nothing),
        ("GetCursorDelta", zeros),
        ("GetCursorCenterPosition", zeros),
        ("ClearCursorHoveredItem", accept),
        ("GetEquipmentNameFromSpell", nil),
        // Item upgrades (none in WoW Forever).
        ("C_ItemUpgrade.GetItemUpgradeItemInfo", nothing),
        ("C_ItemUpgrade.CanUpgradeItem", no),
        ("C_ItemUpgrade.CloseItemUpgrade", accept),
        ("C_ItemUpgrade.ClearItemUpgrade", accept),
        ("C_ItemUpgrade.GetItemUpgradeCurrentLevel", nothing),
        ("C_ItemUpgrade.SetItemUpgradeFromCursorItem", accept),
        ("C_ItemUpgrade.SetItemUpgradeFromLocation", accept),
        ("C_ItemUpgrade.UpgradeItem", accept),
        ("C_ItemUpgrade.GetItemHyperlink", nothing),
        ("C_ItemUpgrade.GetItemUpgradeEffect", nothing),
        (
            "C_ItemUpgrade.GetItemUpgradePvpItemLevelDeltaValues",
            nothing,
        ),
        ("C_ItemUpgrade.GetNumItemUpgradeEffects", zero),
        ("C_ItemUpgrade.IsItemBound", no),
        ("C_ItemUpgrade.GetHighWatermarkForItem", nothing),
        ("C_ItemUpgrade.GetHighWatermarkForSlot", nothing),
        ("C_ItemUpgrade.GetHighWatermarkSlotForItem", nothing),
        // Item interactions (none).
        ("C_ItemInteraction.ClearPendingItem", accept),
        ("C_ItemInteraction.SetPendingItem", no),
        ("C_ItemInteraction.GetChargeInfo", documented),
        (
            "C_ItemInteraction.GetItemConversionCurrencyCost",
            documented,
        ),
        ("C_ItemInteraction.CloseUI", accept),
        ("C_ItemInteraction.GetItemInteractionInfo", nil),
        ("C_ItemInteraction.GetItemInteractionSpellId", zero),
        ("C_ItemInteraction.InitializeFrame", accept),
        ("C_ItemInteraction.PerformItemInteraction", accept),
        ("C_ItemInteraction.Reset", accept),
        // Sockets (no socketing open).
        ("C_ItemSocketInfo.GetExistingSocketInfo", documented),
        ("C_ItemSocketInfo.GetNewSocketInfo", documented),
        ("C_ItemSocketInfo.AcceptSockets", accept),
        ("C_ItemSocketInfo.CompleteSocketing", accept),
        ("C_ItemSocketInfo.ClickSocketButton", accept),
        ("C_ItemSocketInfo.CloseSocketInfo", accept),
        ("C_ItemSocketInfo.GetExistingSocketLink", nil),
        ("C_ItemSocketInfo.GetNewSocketLink", nil),
        ("C_ItemSocketInfo.GetNumSockets", zero),
        ("C_ItemSocketInfo.GetSocketItemBoundTradeable", no),
        ("C_ItemSocketInfo.GetSocketItemInfo", documented),
        ("C_ItemSocketInfo.GetSocketItemRefundable", no),
        ("C_ItemSocketInfo.GetSocketTypes", nil),
        ("C_ItemSocketInfo.HasBoundGemProposed", no),
        ("C_ItemSocketInfo.GetCurrUIType", zero),
        // Azerite and artifacts (none).
        ("C_AzeriteItem.GetPowerLevel", zero),
        ("C_AzeriteItem.GetUnlimitedPowerLevel", zero),
        ("C_AzeriteItem.IsAzeriteItemEnabled", no),
        ("C_AzeriteItem.GetAzeriteItemXPInfo", nothing),
        ("C_AzeriteItem.HasActiveAzeriteItem", no),
        ("C_AzeriteItem.IsAzeriteItem", no),
        ("C_AzeriteItem.IsAzeriteItemByID", no),
        ("C_AzeriteItem.IsUnlimitedLevelingUnlocked", no),
        ("IsArtifactCompletionHistoryAvailable", no),
        ("SocketItemToArtifact", accept),
        // New items, scrapping, trade money.
        ("C_NewItems.ClearAll", accept),
        ("C_NewItems.RemoveNewItem", accept),
        ("C_ScrappingMachineUI.ValidateScrappingList", accept),
        ("C_ScrappingMachineUI.CloseScrappingMachine", accept),
        (
            "C_ScrappingMachineUI.DropPendingScrapItemFromCursor",
            accept,
        ),
        (
            "C_ScrappingMachineUI.GetCurrentPendingScrapItemLocationByIndex",
            nothing,
        ),
        ("C_ScrappingMachineUI.GetScrapSpellID", zero),
        ("C_ScrappingMachineUI.GetScrappingMachineName", nothing),
        ("C_ScrappingMachineUI.HasScrappableItems", no),
        ("C_ScrappingMachineUI.RemoveAllScrapItems", accept),
        ("C_ScrappingMachineUI.RemoveCurrentScrappingItem", accept),
        ("C_ScrappingMachineUI.RemoveItemToScrap", accept),
        ("C_ScrappingMachineUI.ScrapItems", accept),
        ("C_TradeInfo.AddTradeMoney", accept),
        ("C_TradeInfo.PickupTradeMoney", accept),
        ("C_TradeInfo.SetTradeMoney", accept),
        ("C_TradeInfo.ShouldShowTradeOfferWarning", no),
    ]
}

/// `C_Item.GetItemSetInfo(setID)`: no item sets known here, an empty name
/// (unverified).
fn get_item_set_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

#[cfg(test)]
mod tests {
    #[test]
    fn armor_effectiveness_matches_the_recording() {
        // Recorded: GetArmorEffectiveness(1149, 0) = 0.70318239927292.
        let e = super::armor_effectiveness(1149.0, 0.0);
        assert!((e - 0.70318239927292).abs() < 1e-6, "{e}");
        assert_eq!(super::armor_effectiveness(0.0, 0.0), 0.0);
        assert!(super::armor_effectiveness(1e9, 60.0) <= 0.75);
    }
}
