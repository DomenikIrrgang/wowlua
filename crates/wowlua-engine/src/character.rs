//! The player's possessions and place (spec Tooling §16.1.4, decision
//! T213): money, equipped items, bags, the zone and map position, and how
//! far other units are. Tests and world profiles set them through the
//! controls; each change fires the events the client fires for it.
//!
//! Verified against the client (`/stdtest engine record`, Forever 70425):
//! `PLAYER_MONEY()`; `UNIT_INVENTORY_CHANGED("player")` before
//! `PLAYER_EQUIPMENT_CHANGED(slot, isEmpty)` (true for an emptied slot,
//! false for a filled one); `BAG_UPDATE` (the backpack's followed by
//! `BAG_UPDATE(-1)`), `BAG_CLOSED` for a bag taken away,
//! `BAG_CONTAINER_UPDATE()`, one `BAG_UPDATE_DELAYED()`, then
//! `ITEM_COUNT_CHANGED(itemID)`; a new zone's lone
//! `ZONE_CHANGED_NEW_AREA()`, a new subzone's lone `ZONE_CHANGED()`.
//!
//! Unverified (inferred from Blizzard's documentation and UI code):
//! `BAG_CONTAINER_UPDATE` for a bag put in, other flavors' item links, the
//! ranges of `CheckInteractDistance`'s indexes and `UnitInRange` (38
//! yards), and the empty backpack's 16 slots.

use std::collections::BTreeMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, units, widgets};

/// An item in an equipment slot or a bag slot.
#[derive(Clone, Debug, Default)]
pub struct Held {
    pub id: i64,
    pub count: f64,
    /// The link when the test gave one (else made from the item's data).
    pub link: Option<String>,
    pub bound: bool,
    pub locked: bool,
    /// A quest item (`C_Container.GetContainerItemQuestInfo`): one, the
    /// quest it starts, whether that quest is active.
    pub quest_item: bool,
    pub quest_id: Option<f64>,
    pub quest_active: bool,
}

#[derive(Clone, Debug)]
pub struct Bag {
    pub slots: usize,
    pub items: BTreeMap<usize, Held>,
}

/// Where the player is.
#[derive(Clone, Debug, Default)]
pub struct Position {
    pub map_id: Option<f64>,
    pub zone: String,
    pub sub_zone: String,
    /// The map position, 0 to 1.
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// `UnitPosition`'s world coordinates and instance ID, when given.
    pub world: Option<(f64, f64, f64, f64)>,
}

pub struct Character {
    /// In copper.
    pub money: f64,
    /// By inventory slot ID (1 head … 19 tabard).
    pub equipment: BTreeMap<i64, Held>,
    /// By bag ID (0 the backpack).
    pub bags: BTreeMap<i64, Bag>,
    pub position: Position,
    /// The player's stats (`stats.rs`).
    pub stats: crate::stats::Stats,
    /// The action bars (`actions.rs`).
    pub actions: crate::actions::Actions,
    /// The chat channels (`channels.rs`).
    pub channels: crate::channels::Channels,
}

impl Default for Character {
    fn default() -> Self {
        let mut bags = BTreeMap::new();
        bags.insert(
            0,
            Bag {
                slots: 16,
                items: BTreeMap::new(),
            },
        );
        Character {
            money: 0.0,
            equipment: BTreeMap::new(),
            bags,
            position: Position::default(),
            stats: crate::stats::defaults(),
            actions: Default::default(),
            channels: Default::default(),
        }
    }
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn text(vm: &mut Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// An item as a test gives it: its ID, or `{ id, count, link, bound,
/// locked, questItem, questID, questActive }`.
fn held_of(vm: &mut Vm, v: Value) -> Option<Held> {
    match v {
        Value::Num(n) => Some(Held {
            id: n as i64,
            count: 1.0,
            ..Held::default()
        }),
        Value::Table(t) => {
            let id = num(field(vm, t, "id"))? as i64;
            let link = field(vm, t, "link");
            Some(Held {
                id,
                count: num(field(vm, t, "count")).unwrap_or(1.0),
                link: text(vm, link),
                bound: field(vm, t, "bound").truthy(),
                locked: field(vm, t, "locked").truthy(),
                quest_item: field(vm, t, "questItem").truthy(),
                quest_id: num(field(vm, t, "questID")),
                quest_active: field(vm, t, "questActive").truthy(),
            })
        }
        _ => None,
    }
}

/// The item data a test defined for `id` (`defineItem`).
fn item_data(vm: &Vm, id: i64) -> Option<TableRef> {
    engine(vm).world.data.items.get(&id).copied()
}

/// The quality colors of the client's `ITEM_QUALITY_COLORS` (poor to
/// heirloom).
const QUALITY_COLORS: [&str; 8] = [
    "ff9d9d9d", "ffffffff", "ff1eff00", "ff0070dd", "ffa335ee", "ffff8000", "ffe6cc80", "ff00ccff",
];

/// An item's link: the test's, the item data's, or one made like the
/// client's (`|cff…|Hitem:ID::::::::LEVEL:::::|h[Name]|h|r`).
pub fn link_of(vm: &mut Vm, held: &Held) -> Option<Value> {
    if let Some(l) = &held.link {
        return Some(vm.str(l));
    }
    let data = item_data(vm, held.id)?;
    let given = field(vm, data, "link");
    if !given.is_nil() {
        return Some(given);
    }
    let name = field(vm, data, "name");
    let name = text(vm, name).unwrap_or_else(|| format!("item:{}", held.id));
    let quality = num(field(vm, data, "quality")).unwrap_or(1.0) as usize;
    let color = QUALITY_COLORS
        .get(quality)
        .copied()
        .unwrap_or(QUALITY_COLORS[1]);
    let level = engine(vm).units.list[units::PLAYER.0].level;
    // Camelot's links name the quality's color and carry the player's
    // specialization (`|cnIQ1:|Hitem:6529::::::::21:1486:::::::::|h…`,
    // recorded; the specialization unknown here: empty).
    if engine(vm).flavor == "camelot" {
        return Some(vm.str(&format!(
            "|cnIQ{quality}:|Hitem:{}::::::::{level}::::::::::|h[{name}]|h|r",
            held.id
        )));
    }
    Some(vm.str(&format!(
        "|c{color}|Hitem:{}::::::::{level}:::::|h[{name}]|h|r",
        held.id
    )))
}

/// An item's link by its ID (made like the client's).
pub fn link_of_id(vm: &mut Vm, id: i64) -> Option<Value> {
    let held = Held {
        id,
        count: 1.0,
        ..Held::default()
    };
    link_of(vm, &held)
}

pub fn item_field(vm: &mut Vm, id: i64, key: &str) -> Value {
    match item_data(vm, id) {
        Some(t) => field(vm, t, key),
        None => Value::Nil,
    }
}

// ------------------------------------------------------------------ money

fn get_money(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).world.character.money)])
}

/// `unit:setMoney(copper)` (the player): `PLAYER_MONEY`.
pub fn control_set_money(vm: &mut Vm, c: NativeCall) -> Ret {
    player_only(vm, &c, "setMoney")?;
    let n = vm.check_number(&c, 2)?;
    engine_mut(vm).world.character.money = n.max(0.0).floor();
    widgets::fire_event(vm, "PLAYER_MONEY", &[]);
    Ok(vec![])
}

fn player_only(vm: &mut Vm, c: &NativeCall, what: &str) -> Result<(), wowlua_vm::LuaError> {
    let me = units::this(vm, c)?;
    if me != units::PLAYER {
        return Err(vm.error(&format!("unit:{what}: only the player has that")));
    }
    Ok(())
}

// -------------------------------------------------------------- equipment

/// The equipped item of `unit` (the player only) in slot `n`.
fn equipped(vm: &mut Vm, c: &NativeCall) -> Option<Held> {
    if units::unit_arg(vm, c, 1) != Some(units::PLAYER) {
        return None;
    }
    let slot = num(c.arg(2))? as i64;
    engine(vm).world.character.equipment.get(&slot).cloned()
}

fn get_inventory_item_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(equipped(vm, &c)
        .map(|h| vec![Value::Num(h.id as f64)])
        .unwrap_or_default())
}

fn get_inventory_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(equipped(vm, &c)
        .and_then(|h| link_of(vm, &h))
        .map(|l| vec![l])
        .unwrap_or_default())
}

fn get_inventory_item_texture(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(equipped(vm, &c)
        .map(|h| vec![item_field(vm, h.id, "icon")])
        .unwrap_or_default())
}

fn get_inventory_item_quality(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(equipped(vm, &c)
        .map(|h| vec![item_field(vm, h.id, "quality")])
        .unwrap_or_default())
}

fn get_inventory_item_count(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Num(equipped(vm, &c).map_or(0.0, |h| h.count))])
}

/// `unit:equip(slot, item)` / `unit:equip(slot, nil)` (the player):
/// `UNIT_INVENTORY_CHANGED("player")`, then
/// `PLAYER_EQUIPMENT_CHANGED(slot, isEmpty)`.
pub fn control_equip(vm: &mut Vm, c: NativeCall) -> Ret {
    player_only(vm, &c, "equip")?;
    let slot = vm.check_int(&c, 2)?;
    let held = held_of(vm, c.arg(3));
    let has = held.is_some();
    let eq = &mut engine_mut(vm).world.character.equipment;
    match held {
        Some(h) => {
            eq.insert(slot, h);
        }
        None => {
            eq.remove(&slot);
        }
    }
    let player = vm.str("player");
    widgets::fire_event(vm, "UNIT_INVENTORY_CHANGED", &[player]);
    widgets::fire_event(
        vm,
        "PLAYER_EQUIPMENT_CHANGED",
        &[Value::Num(slot as f64), Value::Bool(!has)],
    );
    Ok(vec![])
}

// ------------------------------------------------------------------- bags

fn bag_slot(vm: &mut Vm, c: &NativeCall) -> Option<Held> {
    let bag = num(c.arg(1))? as i64;
    let slot = num(c.arg(2))? as usize;
    engine(vm)
        .world
        .character
        .bags
        .get(&bag)?
        .items
        .get(&slot)
        .cloned()
}

fn get_container_num_slots(vm: &mut Vm, c: NativeCall) -> Ret {
    let bag = num(c.arg(1)).unwrap_or(-99.0) as i64;
    let n = engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .map_or(0, |b| b.slots);
    Ok(vec![Value::Num(n as f64)])
}

fn get_container_num_free_slots(vm: &mut Vm, c: NativeCall) -> Ret {
    let bag = num(c.arg(1)).unwrap_or(-99.0) as i64;
    let free = engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .map_or(0, |b| b.slots.saturating_sub(b.items.len()));
    Ok(vec![Value::Num(free as f64), Value::Num(0.0)])
}

fn get_container_item_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(bag_slot(vm, &c)
        .map(|h| vec![Value::Num(h.id as f64)])
        .unwrap_or_default())
}

fn get_container_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(bag_slot(vm, &c)
        .and_then(|h| link_of(vm, &h))
        .map(|l| vec![l])
        .unwrap_or_default())
}

/// `C_Container.GetContainerItemInfo(bag, slot)`: a `ContainerItemInfo`.
fn get_container_item_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(h) = bag_slot(vm, &c) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    let link = link_of(vm, &h).unwrap_or(Value::Nil);
    let icon = item_field(vm, h.id, "icon");
    let quality = item_field(vm, h.id, "quality");
    for (k, v) in [
        ("iconFileID", icon),
        ("stackCount", Value::Num(h.count)),
        ("isLocked", Value::Bool(h.locked)),
        ("quality", quality),
        ("isReadable", Value::Bool(false)),
        ("hasLoot", Value::Bool(false)),
        ("hyperlink", link),
        ("isFiltered", Value::Bool(false)),
        ("hasNoValue", Value::Bool(false)),
        ("itemID", Value::Num(h.id as f64)),
        ("isBound", Value::Bool(h.bound)),
    ] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `C_Item.GetItemCount(item)`: in the bags (and equipped).
fn get_item_count(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = match vm.hidden(c.arg(1)) {
        Value::Num(n) => n as i64,
        _ => return Ok(vec![Value::Num(0.0)]),
    };
    let ch = &engine(vm).world.character;
    let in_bags: f64 = ch
        .bags
        .values()
        .flat_map(|b| b.items.values())
        .filter(|h| h.id == id)
        .map(|h| h.count)
        .sum();
    let worn: f64 = ch
        .equipment
        .values()
        .filter(|h| h.id == id)
        .map(|h| h.count)
        .sum();
    Ok(vec![Value::Num(in_bags + worn)])
}

/// Each item's count in the bags.
fn bag_counts(vm: &Vm) -> BTreeMap<i64, f64> {
    let mut out = BTreeMap::new();
    for b in engine(vm).world.character.bags.values() {
        for h in b.items.values() {
            *out.entry(h.id).or_default() += h.count;
        }
    }
    out
}

/// `mock.setBag(bag, { slots = n, items = { [slot] = item, … } })`: the
/// bag's contents (`nil`: no bag there); `BAG_CLOSED(bag)` for a bag taken
/// away, `BAG_UPDATE(bag)` (the backpack's then `BAG_UPDATE(-1)`),
/// `BAG_CONTAINER_UPDATE()` for a bag put in or taken away,
/// `BAG_UPDATE_DELAYED()`, then `ITEM_COUNT_CHANGED(itemID)` per item whose
/// count changed.
pub fn control_set_bag(vm: &mut Vm, c: NativeCall) -> Ret {
    let bag = vm.check_int(&c, 1)?;
    let contents = match c.arg(2) {
        Value::Table(t) => {
            let slots = num(field(vm, t, "slots")).unwrap_or(16.0) as usize;
            let mut items = BTreeMap::new();
            if let Value::Table(list) = field(vm, t, "items") {
                for slot in 1..=slots {
                    let v = vm.heap.table(list).get(Value::Num(slot as f64));
                    if let Some(h) = held_of(vm, v) {
                        items.insert(slot, h);
                    }
                }
            }
            Some(Bag { slots, items })
        }
        _ => None,
    };
    let before = bag_counts(vm);
    let bags = &mut engine_mut(vm).world.character.bags;
    let had = bags.contains_key(&bag);
    let has = contents.is_some();
    match contents {
        Some(b) => {
            bags.insert(bag, b);
        }
        None => {
            bags.remove(&bag);
        }
    }
    if had && !has {
        widgets::fire_event(vm, "BAG_CLOSED", &[Value::Num(bag as f64)]);
    }
    widgets::fire_event(vm, "BAG_UPDATE", &[Value::Num(bag as f64)]);
    if bag == 0 {
        widgets::fire_event(vm, "BAG_UPDATE", &[Value::Num(-1.0)]);
    }
    if bag > 0 && had != has {
        widgets::fire_event(vm, "BAG_CONTAINER_UPDATE", &[]);
    }
    widgets::fire_event(vm, "BAG_UPDATE_DELAYED", &[]);
    let after = bag_counts(vm);
    let changed: Vec<i64> = before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|id| before.get(id) != after.get(id))
        .collect();
    for id in changed {
        widgets::fire_event(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    }
    Ok(vec![])
}

// ------------------------------------------------------------------ place

fn zone_text(get: fn(&Position) -> &str) -> impl Fn(&mut Vm, NativeCall) -> Ret {
    move |vm, _c| {
        let s = get(&engine(vm).world.character.position).to_string();
        Ok(vec![vm.str(&s)])
    }
}

fn get_zone_text(vm: &mut Vm, c: NativeCall) -> Ret {
    zone_text(|p| &p.zone)(vm, c)
}

fn get_sub_zone_text(vm: &mut Vm, c: NativeCall) -> Ret {
    zone_text(|p| &p.sub_zone)(vm, c)
}

/// The minimap's zone: the subzone, else the zone.
fn get_minimap_zone_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    let p = &engine(vm).world.character.position;
    let s = if p.sub_zone.is_empty() {
        p.zone.clone()
    } else {
        p.sub_zone.clone()
    };
    Ok(vec![vm.str(&s)])
}

fn get_best_map_for_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    if units::unit_arg(vm, &c, 1) != Some(units::PLAYER) {
        return Ok(vec![]);
    }
    Ok(engine(vm)
        .world
        .character
        .position
        .map_id
        .map(|m| vec![Value::Num(m)])
        .unwrap_or_default())
}

/// `C_Map.GetPlayerMapPosition(mapID, unit)`: a vector (`x`, `y`,
/// `GetXY`) on the player's map.
fn get_player_map_position(vm: &mut Vm, c: NativeCall) -> Ret {
    let p = engine(vm).world.character.position.clone();
    let on_map = num(c.arg(1)).is_some_and(|m| Some(m) == p.map_id);
    let (Some(x), Some(y)) = (p.x, p.y) else {
        return Ok(vec![]);
    };
    if !on_map || units::unit_arg(vm, &c, 2) != Some(units::PLAYER) {
        return Ok(vec![]);
    }
    let v = vm.new_table();
    for (k, n) in [("x", x), ("y", y)] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(v).set(key, Value::Num(n));
    }
    let get_xy = vm.native("GetXY", vector_xy);
    let key = vm.str("GetXY");
    let _ = vm.heap.table_mut(v).set(key, get_xy);
    Ok(vec![Value::Table(v)])
}

fn vector_xy(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Ok(vec![]);
    };
    let x = field(vm, t, "x");
    let y = field(vm, t, "y");
    Ok(vec![x, y])
}

fn unit_position(vm: &mut Vm, c: NativeCall) -> Ret {
    if units::unit_arg(vm, &c, 1) != Some(units::PLAYER) {
        return Ok(vec![]);
    }
    Ok(match engine(vm).world.character.position.world {
        Some((y, x, z, instance)) => vec![
            Value::Num(y),
            Value::Num(x),
            Value::Num(z),
            Value::Num(instance),
        ],
        None => vec![],
    })
}

/// `mock.moveTo({ mapID, zone, subZone, x, y, world = { y, x, z,
/// instanceID } })`: `ZONE_CHANGED_NEW_AREA` when the zone changes,
/// `ZONE_CHANGED` when only the subzone does.
pub fn control_move_to(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("mock.moveTo: a table of the place expected"));
    };
    let old = engine(vm).world.character.position.clone();
    let mut p = old.clone();
    if let Some(m) = num(field(vm, t, "mapID")) {
        p.map_id = Some(m);
    }
    let zone = field(vm, t, "zone");
    if let Some(z) = text(vm, zone) {
        p.zone = z;
        p.sub_zone = String::new();
    }
    let sub = field(vm, t, "subZone");
    if let Some(s) = text(vm, sub) {
        p.sub_zone = s;
    }
    if let Some(x) = num(field(vm, t, "x")) {
        p.x = Some(x);
    }
    if let Some(y) = num(field(vm, t, "y")) {
        p.y = Some(y);
    }
    if let Value::Table(w) = field(vm, t, "world") {
        let get = |vm: &mut Vm, k: &str| num(field(vm, w, k)).unwrap_or(0.0);
        p.world = Some((
            get(vm, "y"),
            get(vm, "x"),
            get(vm, "z"),
            get(vm, "instanceID"),
        ));
    }
    let new_area = p.zone != old.zone || p.map_id != old.map_id;
    let new_sub = p.sub_zone != old.sub_zone;
    engine_mut(vm).world.character.position = p;
    if new_area {
        widgets::fire_event(vm, "ZONE_CHANGED_NEW_AREA", &[]);
    } else if new_sub {
        widgets::fire_event(vm, "ZONE_CHANGED", &[]);
    }
    Ok(vec![])
}

// --------------------------------------------------------------- distance

fn distance_of(vm: &Vm, c: &NativeCall) -> Option<f64> {
    let id = units::unit_arg(vm, c, 1)?;
    if id == units::PLAYER {
        return Some(0.0);
    }
    engine(vm).units.list[id.0].distance
}

/// `CheckInteractDistance(unit, index)`: 1 inspect (28 yd), 2 trade
/// (11.11), 3 duel (9.9), 4 follow (28).
fn check_interact_distance(vm: &mut Vm, c: NativeCall) -> Ret {
    let range = match num(c.arg(2)).unwrap_or(0.0) as i64 {
        1 | 4 => 28.0,
        2 => 11.11,
        3 => 9.9,
        _ => return Ok(vec![Value::Bool(false)]),
    };
    Ok(vec![Value::Bool(
        distance_of(vm, &c).is_some_and(|d| d <= range),
    )])
}

/// `UnitInRange(unit)`: a group member within 38 yards; `checkedRange`.
fn unit_in_range(vm: &mut Vm, c: NativeCall) -> Ret {
    match distance_of(vm, &c) {
        Some(d) => Ok(vec![Value::Bool(d <= 38.0), Value::Bool(true)]),
        None => Ok(vec![Value::Bool(false), Value::Bool(false)]),
    }
}

/// `UnitDistanceSquared(unit)`: the distance squared, `checked`.
fn unit_distance_squared(vm: &mut Vm, c: NativeCall) -> Ret {
    match distance_of(vm, &c) {
        Some(d) => Ok(vec![Value::Num(d * d), Value::Bool(true)]),
        None => Ok(vec![Value::Num(0.0), Value::Bool(false)]),
    }
}

/// `unit:setDistance(yards)` (`nil`: unknown).
pub fn control_set_distance(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let d = num(c.arg(2));
    engine_mut(vm).units.list[me.0].distance = d;
    Ok(vec![])
}

// ------------------------------------------------------- bags and slots

/// `C_Container.GetContainerItemQuestInfo(bag, slot)`: the item's quest
/// (none for an empty slot; recorded).
fn container_item_quest_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let h = bag_slot(vm, &c).unwrap_or_default();
    let t = vm.new_table();
    let set = |vm: &mut Vm, k: &str, v: Value| {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, v);
    };
    set(vm, "isQuestItem", Value::Bool(h.quest_item));
    set(vm, "isActive", Value::Bool(h.quest_active));
    if let Some(id) = h.quest_id {
        set(vm, "questID", Value::Num(id));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_Container.GetBagName(bag)`: the backpack's name, else the name of
/// the bag in the bag's inventory slot (bag + 30); nil without one
/// (recorded).
fn bag_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(bag) = num(c.arg(1)).map(|b| b as i64) else {
        return Ok(vec![Value::Nil]);
    };
    if bag == 0 {
        let name = match vm.get_global("BACKPACK_TOOLTIP") {
            Value::Str(s) => Value::Str(s),
            _ => vm.str("Backpack"),
        };
        return Ok(vec![name]);
    }
    let held = engine(vm)
        .world
        .character
        .equipment
        .get(&(bag + 30))
        .map(|h| h.id);
    Ok(vec![match held {
        Some(id) => item_field(vm, id, "name"),
        None => Value::Nil,
    }])
}

/// `GetInventoryAlertStatus(slot)`: no durability warning (recorded for
/// whole gear; unverified for damaged).
fn inventory_alert_status(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `GetInventoryItemCooldown(unit, slot)`: none running (recorded).
fn inventory_item_cooldown(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Bool(false),
    ])
}

/// `GetInventoryItemBroken(unit, slot)`, `GetInventoryItemEquippedUnusable(
/// unit, slot)`: whole and usable (recorded; unverified otherwise).
fn inventory_item_no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// The functions with behaviour here.
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetMoney", get_money),
        (
            "C_Container.GetContainerItemQuestInfo",
            container_item_quest_info,
        ),
        ("C_Container.GetBagName", bag_name),
        ("GetInventoryAlertStatus", inventory_alert_status),
        ("GetInventoryItemCooldown", inventory_item_cooldown),
        ("GetInventoryItemBroken", inventory_item_no),
        ("GetInventoryItemEquippedUnusable", inventory_item_no),
        ("GetInventoryItemID", get_inventory_item_id),
        ("GetInventoryItemLink", get_inventory_item_link),
        ("GetInventoryItemTexture", get_inventory_item_texture),
        ("GetInventoryItemQuality", get_inventory_item_quality),
        ("GetInventoryItemCount", get_inventory_item_count),
        ("C_Container.GetContainerNumSlots", get_container_num_slots),
        (
            "C_Container.GetContainerNumFreeSlots",
            get_container_num_free_slots,
        ),
        ("C_Container.GetContainerItemID", get_container_item_id),
        ("C_Container.GetContainerItemLink", get_container_item_link),
        ("C_Container.GetContainerItemInfo", get_container_item_info),
        ("C_Item.GetItemCount", get_item_count),
        ("GetItemCount", get_item_count),
        ("GetZoneText", get_zone_text),
        ("GetRealZoneText", get_zone_text),
        ("GetSubZoneText", get_sub_zone_text),
        ("GetMinimapZoneText", get_minimap_zone_text),
        ("C_Map.GetBestMapForUnit", get_best_map_for_unit),
        ("C_Map.GetPlayerMapPosition", get_player_map_position),
        ("UnitPosition", unit_position),
        ("CheckInteractDistance", check_interact_distance),
        ("UnitInRange", unit_in_range),
        ("UnitDistanceSquared", unit_distance_squared),
    ]
}
