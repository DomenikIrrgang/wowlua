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
    /// Pushed into a new slot, its data not arrived yet: the slot is
    /// taken but reads empty (`loot.rs`; recorded).
    pub arriving: bool,
    /// Worn gear's durability: current, maximum.
    pub durability: Option<(f64, f64)>,
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
    /// The continent or instance (`GetInstanceInfo`'s name and ID).
    pub instance: Option<(String, f64)>,
    /// `C_PvP.GetZonePVPInfo`: the zone's PvP type, whether free for all,
    /// its faction.
    pub pvp: Option<(String, bool, String)>,
    /// Under a roof (`IsIndoors`).
    pub indoors: bool,
}

pub struct Character {
    /// In copper.
    pub money: f64,
    /// By inventory slot ID (1 head … 19 tabard).
    pub equipment: BTreeMap<i64, Held>,
    /// Items' running cooldowns: start (the clock's), duration.
    pub item_cooldowns: BTreeMap<i64, (f64, f64)>,
    /// By bag ID (0 the backpack).
    pub bags: BTreeMap<i64, Bag>,
    pub position: Position,
    /// The player's stats (`stats.rs`).
    pub stats: crate::stats::Stats,
    /// The action bars (`actions.rs`).
    pub actions: crate::actions::Actions,
    /// The actions' usability last told (`ACTION_USABLE_CHANGED`).
    pub usable_seen: std::collections::BTreeMap<i64, (bool, bool)>,
    /// The chat channels (`channels.rs`).
    pub channels: crate::channels::Channels,
    /// The macros (`macros.rs`).
    pub macros: crate::macros::Macros,
    /// What the cursor holds (`cursor.rs`).
    pub cursor: crate::cursor::Cursor,
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
            item_cooldowns: BTreeMap::new(),
            bags,
            position: Position::default(),
            stats: crate::stats::defaults(),
            actions: Default::default(),
            usable_seen: Default::default(),
            channels: Default::default(),
            macros: Default::default(),
            cursor: Default::default(),
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
                arriving: false,
                durability: match (
                    num(field(vm, t, "durability")),
                    num(field(vm, t, "maxDurability")),
                ) {
                    (Some(d), Some(m)) => Some((d, m)),
                    _ => None,
                },
            })
        }
        _ => None,
    }
}

/// The item data a test defined for `id` (`defineItem`), else the
/// probes recorded.
fn item_data(vm: &mut Vm, id: i64) -> Option<TableRef> {
    crate::gamedata::item_data(vm, id)
}

/// The quality colors of the client's `ITEM_QUALITY_COLORS` (poor to
/// heirloom).
const QUALITY_COLORS: [&str; 8] = [
    "ff9d9d9d", "ffffffff", "ff1eff00", "ff0070dd", "ffa335ee", "ffff8000", "ffe6cc80", "ff00ccff",
];

/// An item's link: the test's, the item data's, or one made like the
/// client's (`|cff…|Hitem:ID::::::::LEVEL:::::|h[Name]|h|r`).
pub fn link_of(vm: &mut Vm, held: &Held) -> Option<Value> {
    let level = engine(vm).units.list[units::PLAYER.0].level;
    if let Some(l) = &held.link {
        let l = relevel(l, level);
        return Some(vm.str(&l));
    }
    let data = item_data(vm, held.id)?;
    let given = field(vm, data, "link");
    if let Some(l) = text(vm, given) {
        let l = relevel(&l, level);
        return Some(vm.str(&l));
    }
    if !given.is_nil() {
        return Some(given);
    }
    let link = made_link(vm, held.id, "", &[])?;
    Some(vm.str(&link))
}

/// An item's link made like the client's: camelot's name the quality's
/// color and carry the player's level and specialization
/// (`|cnIQ1:|Hitem:6529::::::::21:1486:::::::::|h…`, recorded), the item's
/// context and bonuses after them (a quest's: `::11:1:3524`).
fn made_link(vm: &mut Vm, id: i64, context: &str, bonuses: &[i64]) -> Option<String> {
    let data = item_data(vm, id)?;
    let name = field(vm, data, "name");
    let name = text(vm, name).unwrap_or_else(|| format!("item:{id}"));
    let quality = num(field(vm, data, "quality")).unwrap_or(1.0) as usize;
    let color = QUALITY_COLORS
        .get(quality)
        .copied()
        .unwrap_or(QUALITY_COLORS[1]);
    let level = engine(vm).units.list[units::PLAYER.0].level;
    if engine(vm).flavor == "camelot" {
        let spec = link_specialization(vm).map_or(String::new(), |s| format!("{s}"));
        // After the specialization: the upgrade, the context, the
        // bonuses (their count, each), then six more.
        let bonus = if bonuses.is_empty() {
            String::new()
        } else {
            let list: Vec<String> = bonuses.iter().map(|b| b.to_string()).collect();
            format!("{}:{}", bonuses.len(), list.join(":"))
        };
        return Some(format!(
            "|cnIQ{quality}:|Hitem:{id}::::::::{level}:{spec}::{context}:{bonus}::::::|h[{name}]|h|r"
        ));
    }
    Some(format!(
        "|c{color}|Hitem:{id}::::::::{level}:::::|h[{name}]|h|r"
    ))
}

/// An item's link in a context (a quest's dialog and reward: "11"), with
/// bonuses.
pub fn link_in_context(vm: &mut Vm, id: i64, context: &str, bonuses: &[i64]) -> Option<String> {
    made_link(vm, id, context, bonuses)
}

/// The specialization an item link carries: the player's current one
/// (`C_SpecializationInfo.GetSpecializationInfo(GetSpecialization())`'s
/// ID, as the character's stats recorded it).
fn link_specialization(vm: &Vm) -> Option<i64> {
    use crate::stats::{Answer, Entry};
    let stats = &engine(vm).world.character.stats;
    let Some(Entry::One(current)) = stats.get("specialization") else {
        return None;
    };
    let Some(Answer::Num(index)) = current.first() else {
        return None;
    };
    let info = match stats.get("specializationInfo")? {
        Entry::ByIndex(by) => by.get(&(*index as i64))?.first()?.clone(),
        Entry::One(list) => match list.get((*index as usize).checked_sub(1)?)? {
            Answer::Table(entries) => entries.first()?.1.clone(),
            a => a.clone(),
        },
    };
    match info {
        Answer::Num(id) if id > 0.0 => Some(id as i64),
        _ => None,
    }
}

/// A link with the player's level in it: the client's links follow the
/// player's level (recorded: the bags' links after a level up).
fn relevel(link: &str, level: f64) -> String {
    let Some(start) = link.find("|Hitem:") else {
        return link.to_string();
    };
    let body = start + "|H".len();
    let Some(len) = link[body..].find("|h") else {
        return link.to_string();
    };
    let mut parts: Vec<String> = link[body..body + len]
        .split(':')
        .map(str::to_string)
        .collect();
    match parts.get_mut(9) {
        Some(p) if !p.is_empty() => *p = format!("{level}"),
        _ => return link.to_string(),
    }
    format!(
        "{}{}{}",
        &link[..body],
        parts.join(":"),
        &link[body + len..]
    )
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
    // Another player's, as inspected (`inspect.rs`).
    if let Some(r) = crate::inspect::item_link(vm, &c) {
        return Ok(r);
    }
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

/// `unit:setDurability(slot, current, max)` (the player): a worn item's
/// durability (no event: the snapshot's state).
pub fn control_set_durability(vm: &mut Vm, c: NativeCall) -> Ret {
    player_only(vm, &c, "setDurability")?;
    let slot = vm.check_int(&c, 2)?;
    let current = vm.check_number(&c, 3)?;
    let max = vm.check_number(&c, 4)?;
    match engine_mut(vm).world.character.equipment.get_mut(&slot) {
        Some(h) => h.durability = Some((current, max)),
        None => return Err(vm.error("setDurability: nothing worn there")),
    }
    Ok(vec![])
}

/// `unit:wear(slot?)` (the player): the worn item in `slot` loses a point
/// of durability (none: the server's update without a visible change);
/// `UPDATE_INVENTORY_DURABILITY()` (recorded in fights, now and then).
pub fn control_wear(vm: &mut Vm, c: NativeCall) -> Ret {
    player_only(vm, &c, "wear")?;
    if let Value::Num(slot) = c.arg(2)
        && let Some(h) = engine_mut(vm)
            .world
            .character
            .equipment
            .get_mut(&(slot as i64))
        && let Some((d, m)) = h.durability
    {
        h.durability = Some(((d - 1.0).max(0.0), m));
    }
    widgets::fire_event(vm, "UPDATE_INVENTORY_DURABILITY", &[]);
    Ok(vec![])
}

/// `GetInventoryItemDurability(slot)`: a worn item's durability; nothing
/// for an empty slot or an item without (recorded).
fn get_inventory_item_durability(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(slot) = num(c.arg(1)) else {
        return Ok(vec![]);
    };
    // Loading (a loading screen's, till the inventory's load): nothing
    // (recorded).
    if engine(vm).world.inventory_loading {
        return Ok(vec![]);
    }
    Ok(
        match engine(vm)
            .world
            .character
            .equipment
            .get(&(slot as i64))
            .and_then(|h| h.durability)
        {
            Some((d, m)) => vec![Value::Num(d), Value::Num(m)],
            None => vec![],
        },
    )
}

/// `IsIndoors()`: the place's (`moveTo`'s `indoors`; recorded false in
/// Orgrimmar's streets).
fn is_indoors(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        engine(vm).world.character.position.indoors,
    )])
}

/// `UnitOnTaxi(unit)`: the player on a flight (`taxi.rs`).
fn unit_on_taxi(vm: &mut Vm, c: NativeCall) -> Ret {
    let on = units::unit_arg(vm, &c, 1) == Some(units::PLAYER) && engine(vm).world.on_taxi;
    Ok(vec![Value::Bool(on)])
}

/// The worn items' durability restored (a repair): how many were damaged.
pub fn repair_worn(vm: &mut Vm) -> usize {
    let mut n = 0;
    for h in engine_mut(vm).world.character.equipment.values_mut() {
        if let Some((d, m)) = h.durability
            && d < m
        {
            h.durability = Some((m, m));
            n += 1;
        }
    }
    n
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
        .filter(|h| !h.arriving)
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
    // No vendor price: no value (a Hearthstone, recorded).
    let no_value = matches!(item_field(vm, h.id, "sellPrice"), Value::Num(p) if p == 0.0);
    let name = item_field(vm, h.id, "name");
    // A book's pages (its data's `readable`, recorded).
    let readable = item_field(vm, h.id, "readable").truthy();
    for (k, v) in [
        ("iconFileID", icon),
        ("stackCount", Value::Num(h.count)),
        ("isLocked", Value::Bool(h.locked)),
        ("quality", quality),
        ("isReadable", Value::Bool(readable)),
        ("hasLoot", Value::Bool(false)),
        ("hyperlink", link),
        ("isFiltered", Value::Bool(false)),
        ("hasNoValue", Value::Bool(no_value)),
        ("itemName", name),
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
        .fold(0.0, |a, b| a + b);
    let worn: f64 = ch
        .equipment
        .values()
        .filter(|h| h.id == id)
        .map(|h| h.count)
        .fold(0.0, |a, b| a + b);
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
/// A bag of `slots` from a test's table of items (`{ [slot] = item }`),
/// put in place without telling (a bank tab's, there before the bank
/// opens).
pub fn place_bag(vm: &mut Vm, bag: i64, slots: usize, items: Option<TableRef>) {
    let mut held = BTreeMap::new();
    if let Some(list) = items {
        for slot in 1..=slots {
            let v = vm.heap.table(list).get(Value::Num(slot as f64));
            if let Some(h) = held_of(vm, v) {
                held.insert(slot, h);
            }
        }
    }
    engine_mut(vm)
        .world
        .character
        .bags
        .insert(bag, Bag { slots, items: held });
}

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
                        // A snapshot's running cooldown: `{ remaining,
                        // duration }`.
                        if let Value::Table(item) = v
                            && let Value::Table(cd) = field(vm, item, "cooldown")
                            && let (Some(left), Some(duration)) = (
                                num(field(vm, cd, "remaining")),
                                num(field(vm, cd, "duration")),
                            )
                        {
                            set_item_cooldown(vm, h.id, duration, left);
                        }
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

/// An item's cooldown: `duration` seconds, `left` of them to run.
pub fn set_item_cooldown(vm: &mut Vm, id: i64, duration: f64, left: f64) {
    let now = crate::clock::now(vm);
    let cds = &mut engine_mut(vm).world.character.item_cooldowns;
    if duration > 0.0 && left > 0.0 {
        cds.insert(id, (now - (duration - left), duration));
    } else {
        cds.remove(&id);
    }
}

/// An item's running cooldown: its start and duration.
pub fn item_cooldown(vm: &Vm, id: i64) -> Option<(f64, f64)> {
    let now = crate::clock::now(vm);
    engine(vm)
        .world
        .character
        .item_cooldowns
        .get(&id)
        .copied()
        .filter(|(start, duration)| start + duration > now)
}

/// `mock.setItemCooldown(itemID, duration, remaining?)`: the item's
/// cooldown (all of it to run by default; 0: none).
pub fn control_set_item_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_int(&c, 1)?;
    let duration = vm.check_number(&c, 2)?;
    let left = num(c.arg(3)).unwrap_or(duration);
    set_item_cooldown(vm, id, duration, left);
    widgets::fire_event(vm, "BAG_UPDATE_COOLDOWN", &[]);
    Ok(vec![])
}

/// `C_Container.GetItemCooldown(itemID)`: its running cooldown's start and
/// duration, enabled (unverified: none recorded).
fn get_item_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    let cd = num(c.arg(1)).and_then(|id| item_cooldown(vm, id as i64));
    let (start, duration) = cd.unwrap_or((0.0, 0.0));
    Ok(vec![
        Value::Num(start),
        Value::Num(duration),
        Value::Num(1.0),
    ])
}

/// A bag the player carries (the backpack, the bags, the reagent bag; not
/// the bank's tabs, 6 and up).
pub fn carried(bag: i64) -> bool {
    (0..=5).contains(&bag)
}

/// The reagent bag (`Enum.BagIndex.ReagentBag`).
pub const REAGENT_BAG: i64 = 5;

/// The bags a new slot of item `id` may take, in order: a trade good's
/// (`classID` 7) the reagent bag first (recorded: rough stone mined and
/// copper bars made with a slot free in the backpack), any other item's
/// never the reagent bag (unverified: the client's flag for reagents).
fn new_slot_bags(vm: &mut Vm, id: i64) -> Vec<i64> {
    let reagent = matches!(item_field(vm, id, "classID"), Value::Num(c) if c == 7.0);
    let bags: Vec<i64> = engine(vm)
        .world
        .character
        .bags
        .keys()
        .copied()
        .filter(|b| carried(*b))
        .collect();
    let mut out: Vec<i64> = Vec::new();
    if reagent && bags.contains(&REAGENT_BAG) {
        out.push(REAGENT_BAG);
    }
    out.extend(bags.into_iter().filter(|b| *b != REAGENT_BAG));
    out
}

/// `count` of item `id` into the bags: onto a stack of it with room (its
/// data's `stackCount`), else into the first free slot (`new_slot_bags`);
/// the bag, and whether the item took a new slot. None when the bags are
/// full.
pub fn add_to_bags(vm: &mut Vm, id: i64, count: f64) -> Option<(i64, bool)> {
    let order = new_slot_bags(vm, id);
    let stack = match item_field(vm, id, "stackCount") {
        Value::Num(n) if n >= 1.0 => n,
        _ => 1.0,
    };
    // Bound when picked up: its `bindType` 1 (on pickup) or 4 (a quest's;
    // recorded bound: a quest's item, a quest's reward).
    let binds = matches!(item_field(vm, id, "bindType"), Value::Num(b) if b == 1.0 || b == 4.0);
    let bags = &mut engine_mut(vm).world.character.bags;
    for (bag, b) in bags.iter_mut().filter(|(b, _)| carried(**b)) {
        for held in b.items.values_mut() {
            if held.id == id && held.count + count <= stack {
                held.count += count;
                return Some((*bag, false));
            }
        }
    }
    for bag in order {
        let Some(b) = bags.get_mut(&bag) else {
            continue;
        };
        if let Some(slot) = (1..=b.slots).find(|s| !b.items.contains_key(s)) {
            b.items.insert(
                slot,
                Held {
                    id,
                    count,
                    bound: binds,
                    ..Held::default()
                },
            );
            return Some((bag, true));
        }
    }
    None
}

/// `count` of item `id` into the bags as `add_to_bags`; a new slot's
/// item arriving (its data a packet later: `arrived`). The bag and the
/// new slot, if one.
pub fn push_to_bags(vm: &mut Vm, id: i64, count: f64) -> Option<(i64, Option<usize>)> {
    let before: Vec<(i64, usize)> = engine(vm)
        .world
        .character
        .bags
        .iter()
        .flat_map(|(b, bag)| bag.items.keys().map(move |s| (*b, *s)))
        .collect();
    let (bag, new_slot) = add_to_bags(vm, id, count)?;
    if !new_slot {
        return Some((bag, None));
    }
    let b = engine_mut(vm).world.character.bags.get_mut(&bag)?;
    let slot = b
        .items
        .keys()
        .copied()
        .find(|s| !before.contains(&(bag, *s)))?;
    if let Some(h) = b.items.get_mut(&slot) {
        h.arriving = true;
    }
    Some((bag, Some(slot)))
}

/// A new slot's item data arrived: the slot reads it.
pub fn arrived(vm: &mut Vm, bag: i64, slot: usize) {
    if let Some(h) = engine_mut(vm)
        .world
        .character
        .bags
        .get_mut(&bag)
        .and_then(|b| b.items.get_mut(&slot))
    {
        h.arriving = false;
    }
}

/// The bag `count` of item `id` would go to (as `add_to_bags`).
pub fn bag_for(vm: &mut Vm, id: i64, count: f64) -> Option<i64> {
    let stack = match item_field(vm, id, "stackCount") {
        Value::Num(n) if n >= 1.0 => n,
        _ => 1.0,
    };
    let bags = &engine(vm).world.character.bags;
    for (bag, b) in bags.iter().filter(|(b, _)| carried(**b)) {
        if b.items
            .values()
            .any(|h| h.id == id && h.count + count <= stack)
        {
            return Some(*bag);
        }
    }
    let free = |b: &i64| {
        bags.get(b)
            .is_some_and(|b| (1..=b.slots).any(|s| !b.items.contains_key(&s)))
    };
    let free: Vec<i64> = engine(vm)
        .world
        .character
        .bags
        .keys()
        .copied()
        .filter(free)
        .collect();
    new_slot_bags(vm, id).into_iter().find(|b| free.contains(b))
}

/// `count` of item `id` taken from the bags (the last stacks first,
/// unverified); the bags changed.
pub fn take_from_bags(vm: &mut Vm, id: i64, count: f64) -> Vec<i64> {
    let mut left = count;
    let mut changed = Vec::new();
    let bags = &mut engine_mut(vm).world.character.bags;
    for (bag, b) in bags.iter_mut().rev().filter(|(b, _)| carried(**b)) {
        let slots: Vec<usize> = b
            .items
            .iter()
            .rev()
            .filter(|(_, h)| h.id == id)
            .map(|(s, _)| *s)
            .collect();
        for slot in slots {
            if left <= 0.0 {
                break;
            }
            let held = b.items.get_mut(&slot).expect("a slot just found");
            let taken = held.count.min(left);
            held.count -= taken;
            left -= taken;
            if held.count <= 0.0 {
                b.items.remove(&slot);
            }
            if !changed.contains(bag) {
                changed.push(*bag);
            }
        }
    }
    changed
}

/// How many of item `id` the bags hold (not the bank's).
pub fn count_in_bags(vm: &Vm, id: i64) -> f64 {
    engine(vm)
        .world
        .character
        .bags
        .iter()
        .filter(|(b, _)| carried(**b))
        .flat_map(|(_, b)| b.items.values())
        .filter(|h| h.id == id)
        .map(|h| h.count)
        // Not `sum`: nothing held is 0, not its -0.
        .fold(0.0, |a, b| a + b)
}

/// The link of item `id` in `bag` made in a context (a quest reward's).
pub fn set_context(vm: &mut Vm, bag: i64, id: i64, context: &str) {
    let Some(link) = made_link(vm, id, context, &[]) else {
        return;
    };
    if let Some(b) = engine_mut(vm).world.character.bags.get_mut(&bag) {
        for held in b.items.values_mut() {
            if held.id == id && held.link.is_none() {
                held.link = Some(link.clone());
            }
        }
    }
}

/// The class's hidden group buffs: the test's (`e.setHiddenGroupBuffs`),
/// else the player's class's as recorded (`data/camelot-hidden-group-buffs.txt`:
/// a paladin's, a druid's, a shaman's), else none (a level 1 warrior's,
/// recorded; other classes unverified).
fn hidden_group_buffs(vm: &Vm) -> Vec<f64> {
    if let Some(ids) = &engine(vm).world.hidden_group_buffs {
        return ids.clone();
    }
    let class = &engine(vm).units.list[units::PLAYER.0].class;
    include_str!("../data/camelot-hidden-group-buffs.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .find(|(c, _)| c == class)
        .map(|(_, ids)| ids.split(',').filter_map(|i| i.parse().ok()).collect())
        .unwrap_or_default()
}

/// `C_UnitAuras.GetHiddenGroupBuffs()`: the class's (`hidden_group_buffs`).
fn get_hidden_group_buffs(vm: &mut Vm, _c: NativeCall) -> Ret {
    let list = vm.new_table();
    for (i, id) in hidden_group_buffs(vm).into_iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(list)
            .set(Value::Num((i + 1) as f64), Value::Num(id));
    }
    Ok(vec![Value::Table(list)])
}

/// `HIDDEN_GROUP_BUFFS_CHANGED(list)` (the class's hidden group buffs) and
/// `GROUP_BUFF_VISUAL_ALERTS_CHANGED({})`.
pub fn group_buffs_changed(vm: &mut Vm) {
    let list = vm.new_table();
    let ids = hidden_group_buffs(vm);
    for (i, id) in ids.into_iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(list)
            .set(Value::Num((i + 1) as f64), Value::Num(id));
    }
    widgets::fire_event(vm, "HIDDEN_GROUP_BUFFS_CHANGED", &[Value::Table(list)]);
    let none = vm.new_table();
    widgets::fire_event(
        vm,
        "GROUP_BUFF_VISUAL_ALERTS_CHANGED",
        &[Value::Table(none)],
    );
}

/// `setHiddenGroupBuffs(list)`: the class's hidden group buffs' spells.
pub fn control_set_hidden_group_buffs(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setHiddenGroupBuffs: a list of spell IDs expected"));
    };
    let n = vm.heap.table(t).border();
    let ids: Vec<f64> = (1..=n)
        .filter_map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Num(x) => Some(x),
            _ => None,
        })
        .collect();
    engine_mut(vm).world.hidden_group_buffs = Some(ids);
    Ok(vec![])
}

/// The inventory changed: `PLAYER_EQUIPED_SPELLS_CHANGED(slot)` for each
/// main action bar slot holding a spell (recorded: the warrior's 1, 2, 10
/// and 11; the paladin's 1 to 4 and 12).
pub fn equipped_spells_changed(vm: &mut Vm) {
    let slots: Vec<i64> = engine(vm)
        .world
        .character
        .actions
        .iter()
        .filter(|(slot, a)| (1..=12).contains(*slot) && a.kind == "spell")
        .map(|(slot, _)| *slot)
        .collect();
    for slot in slots {
        widgets::fire_event(
            vm,
            "PLAYER_EQUIPED_SPELLS_CHANGED",
            &[Value::Num(slot as f64)],
        );
    }
}

/// A bag as its inventory slot (`ITEM_PUSH`'s): the backpack 0, bag 1 31,
/// bag 5 35 (recorded).
pub fn bag_inventory_slot(bag: i64) -> f64 {
    if bag == 0 { 0.0 } else { (bag + 30) as f64 }
}

/// A bag's contents changed: `BAG_UPDATE(bag)` (the backpack's then
/// `BAG_UPDATE(-1)`).
pub fn bag_updated(vm: &mut Vm, bag: i64) {
    widgets::fire_event(vm, "BAG_UPDATE", &[Value::Num(bag as f64)]);
    if bag == 0 {
        widgets::fire_event(vm, "BAG_UPDATE", &[Value::Num(-1.0)]);
    }
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
    // Behind a loading screen: none (recorded).
    if engine(vm).world.loading_screen {
        return Ok(vec![Value::Nil]);
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
    move_to(vm, t, true);
    Ok(vec![])
}

fn instance_of(vm: &mut Vm, t: TableRef) -> Option<(String, f64)> {
    let instance = field(vm, t, "instance");
    let name = text(vm, instance)?;
    Some((name, num(field(vm, t, "instanceID")).unwrap_or(0.0)))
}

/// The continent or instance of a place (`moveTo`'s table) entered, its
/// zone not yet.
pub fn enter_instance(vm: &mut Vm, t: TableRef) {
    if let Some(i) = instance_of(vm, t) {
        engine_mut(vm).world.character.position.instance = Some(i);
    }
}

/// The player at a place (`moveTo`'s table: mapID, zone, subZone, x, y,
/// world, instance, instanceID, pvp); its zone's events when `told`.
pub fn move_to(vm: &mut Vm, t: TableRef, told: bool) {
    let old = engine(vm).world.character.position.clone();
    let mut p = old.clone();
    if let Some(m) = num(field(vm, t, "mapID")) {
        p.map_id = Some(m);
    }
    let zone = field(vm, t, "zone");
    if let Some(z) = text(vm, zone) {
        // Another zone: its own subzone and PvP status (none unless given).
        if z != p.zone {
            p.pvp = None;
        }
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
    if let Some(i) = instance_of(vm, t) {
        p.instance = Some(i);
    }
    if let Value::Table(pvp) = field(vm, t, "pvp") {
        let kind = vm.heap.table(pvp).get(Value::Num(1.0));
        let faction = vm.heap.table(pvp).get(Value::Num(3.0));
        p.pvp = Some((
            text(vm, kind).unwrap_or_default(),
            vm.heap.table(pvp).get(Value::Num(2.0)).truthy(),
            text(vm, faction).unwrap_or_default(),
        ));
    }
    // Another zone: a new area; another subzone or map: the zone changed
    // (inside a world map object, a city's: indoors), recorded.
    if let Value::Bool(b) = field(vm, t, "indoors") {
        p.indoors = b;
    }
    let new_area = p.zone != old.zone;
    let new_map = p.map_id != old.map_id;
    let changed = new_area || new_map || p.sub_zone != old.sub_zone;
    engine_mut(vm).world.character.position = p;
    if !told || !changed {
        return;
    }
    let event = if new_area {
        "ZONE_CHANGED_NEW_AREA"
    } else if field(vm, t, "wmo").truthy() && !new_map {
        "ZONE_CHANGED_INDOORS"
    } else {
        "ZONE_CHANGED"
    };
    area_changed(vm, event, new_map);
}

/// The world around changed (`event`): the mount journal's usability, a
/// moment later the map's points of interest and fog, the quest watch with
/// another map (recorded on a flight).
fn area_changed(vm: &mut Vm, event: &str, new_map: bool) {
    widgets::fire_event(vm, event, &[]);
    widgets::fire_event(vm, "MOUNT_JOURNAL_USABILITY_CHANGED", &[]);
    let native = vm.native_with("areaUpdated", area_updated, vec![Value::Bool(new_map)]);
    crate::clock::after_engine(vm, AREA_AFTER, native);
}

/// The map's updates after the area's change (recorded: two frames).
const AREA_AFTER: f64 = 0.033;

fn area_updated(vm: &mut Vm, c: NativeCall) -> Ret {
    widgets::fire_event(vm, "AREA_POIS_UPDATED", &[]);
    widgets::fire_event(vm, "FOG_OF_WAR_UPDATED", &[]);
    if c.upvals[0].truthy() {
        widgets::fire_event(vm, "QUEST_WATCH_LIST_CHANGED", &[Value::Nil, Value::Nil]);
    }
    Ok(vec![])
}

/// `setResting(resting)`: the player enters or leaves a resting place:
/// `PLAYER_FLAGS_CHANGED("player")`, `PLAYER_UPDATE_RESTING()` (recorded:
/// a shaman walking into a town); `IsResting()` says so.
pub fn control_set_resting(vm: &mut Vm, c: NativeCall) -> Ret {
    let resting = c.arg(1).truthy();
    if engine(vm).world.resting == resting {
        return Ok(vec![]);
    }
    engine_mut(vm).world.resting = resting;
    let player = vm.str("player");
    widgets::fire_event(vm, "PLAYER_FLAGS_CHANGED", &[player]);
    widgets::fire_event(vm, "PLAYER_UPDATE_RESTING", &[]);
    Ok(vec![])
}

fn is_resting(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).world.resting)])
}

/// `newWmoChunk()`: the player in another part of a world map object (a
/// city's walls): `NEW_WMO_CHUNK()` and the area's updates (recorded).
pub fn control_new_wmo_chunk(vm: &mut Vm, _c: NativeCall) -> Ret {
    area_changed(vm, "NEW_WMO_CHUNK", false);
    Ok(vec![])
}

/// `areaChanged()`: the area's updates without a zone's event (recorded
/// on a flight over the Barrens, its cause unknown).
pub fn control_area_changed(vm: &mut Vm, _c: NativeCall) -> Ret {
    widgets::fire_event(vm, "MOUNT_JOURNAL_USABILITY_CHANGED", &[]);
    let native = vm.native_with("areaUpdated", area_updated, vec![Value::Bool(false)]);
    crate::clock::after_engine(vm, AREA_AFTER, native);
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

/// `unit:setDistance(yards)` (`nil`: unknown); an NPC walked away from
/// closes its gossip (`gossip::moved_away`).
pub fn control_set_distance(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let d = num(c.arg(2));
    engine_mut(vm).units.list[me.0].distance = d;
    crate::gossip::moved_away(vm, me, d);
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
        ("IsResting", is_resting),
        ("C_UnitAuras.GetHiddenGroupBuffs", get_hidden_group_buffs),
        (
            "C_Container.GetContainerItemQuestInfo",
            container_item_quest_info,
        ),
        ("C_Container.GetBagName", bag_name),
        ("GetInventoryAlertStatus", inventory_alert_status),
        ("GetInventoryItemCooldown", inventory_item_cooldown),
        ("C_Container.GetItemCooldown", get_item_cooldown),
        ("GetInventoryItemBroken", inventory_item_no),
        ("GetInventoryItemEquippedUnusable", inventory_item_no),
        ("GetInventoryItemID", get_inventory_item_id),
        ("GetInventoryItemDurability", get_inventory_item_durability),
        ("UnitOnTaxi", unit_on_taxi),
        ("IsIndoors", is_indoors),
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
