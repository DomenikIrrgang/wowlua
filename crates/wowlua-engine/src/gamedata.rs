//! Game data from the tests (decision T203): spells and items a test
//! defines (`defineSpell(id, data)`, `defineItem(id, data, opts)`), read
//! through `C_Spell`, `C_SpellBook` and `C_Item`. An ID nobody defined is
//! the client's own data as recorded (`/stdtest engine answers`; the spells
//! Blizzard's UI names), else data the client does not have. The data
//! stays the test's table, so any value may be a secret.
//!
//! As the M8 mock had them (unverified details): spell data is cached once
//! defined unless `cached = false`, and so is an item unless `cached =
//! false`, a `delay` or `missing` is given; such an item arrives `delay` seconds (0.2) after
//! `RequestLoadItemDataByID`, firing `ITEM_DATA_LOAD_RESULT`, and
//! `GetItemInfo` gives nothing until then and fetches it itself
//! (`GET_ITEM_INFO_RECEIVED`). An item's `itemLevel` is 1 unless given.

use std::collections::HashMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// The defined spells and items: ID → the test's table.
#[derive(Default)]
pub struct GameData {
    pub spells: HashMap<i64, TableRef>,
    pub items: HashMap<i64, TableRef>,
    /// The client's spell table's spells asked so far (`scanned_spell`).
    pub scanned: HashMap<i64, TableRef>,
    /// Items requested, in order.
    pub item_requests: Vec<i64>,
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn define(vm: &mut Vm, c: &NativeCall, items: bool) -> Ret {
    let id = vm.check_int(c, 1)?;
    let Value::Table(data) = c.arg(2) else {
        return Err(vm.error("a table of the data expected"));
    };
    widgets::keep(vm, Value::Table(data));
    if items {
        // `opts`: cached, delay, missing.
        if let Value::Table(opts) = c.arg(3) {
            for k in ["cached", "delay", "missing"] {
                let v = field(vm, opts, k);
                if !v.is_nil() {
                    set(vm, data, k, v);
                }
            }
        }
        // Every item has a level in the client (the full probe's crate and
        // Hearthstone: 1); Blizzard's sell frame compares it.
        if field(vm, data, "itemLevel").is_nil() {
            set(vm, data, "itemLevel", Value::Num(1.0));
        }
        // Cached unless the test says otherwise (`cached = false`, or a
        // `delay` or `missing`, which only an item still to load has), as
        // the mock's and the profiles' docs have it; an item neither cached
        // nor loadable would never arrive.
        if field(vm, data, "cached").is_nil()
            && field(vm, data, "delay").is_nil()
            && !field(vm, data, "missing").truthy()
        {
            set(vm, data, "cached", Value::Bool(true));
        }
        engine_mut(vm).world.data.items.insert(id, data);
    } else {
        engine_mut(vm).world.data.spells.insert(id, data);
    }
    Ok(vec![])
}

/// `defineSpell(id, { name, iconID, description, known, petKnown,
/// cooldown, charges, castTime, minRange, maxRange, cached })`.
pub fn define_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    define(vm, &c, false)
}

/// `defineItem(id, { name, link, quality, itemLevel, minLevel, type,
/// subType, stackCount, equipLoc, icon, sellPrice, classID, subclassID,
/// bindType, expansionID }, { cached, delay, missing })`.
pub fn define_item(vm: &mut Vm, c: NativeCall) -> Ret {
    define(vm, &c, true)
}

/// A spell the game data's probes recorded (a recipe's, a talent's, a
/// trainer's): defined from its recorded info and description (cached,
/// as the client had them), once.
fn probed_spell(vm: &mut Vm, id: i64) -> Option<TableRef> {
    let info = crate::recorded::probed(&format!("C_Spell.GetSpellInfo({id})"))?;
    let info = crate::traits::Rv::answer(info).into_iter().next()?;
    let t = vm.new_table();
    for key in ["name", "iconID", "castTime", "minRange", "maxRange"] {
        if let Some(v) = info.get(key) {
            let v = v.to_value(vm);
            set(vm, t, key, v);
        }
    }
    if let Some(text) = crate::recorded::probed(&format!("C_Spell.GetSpellDescription({id})"))
        && let Some(d) = crate::traits::Rv::answer(text).into_iter().next()
    {
        let d = d.to_value(vm);
        set(vm, t, "description", d);
    }
    // Its rank too (SuuUI probe spellbook: Holy Light's "Rank 3", which a
    // trainer's recording has; it was lost here).
    if let Some(text) = crate::recorded::probed(&format!("C_Spell.GetSpellSubtext({id})"))
        && let Some(s) = crate::traits::Rv::answer(text).into_iter().next()
    {
        let s = s.to_value(vm);
        set(vm, t, "subtext", s);
    }
    set(vm, t, "cached", Value::Bool(true));
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm).world.data.spells.insert(id, t);
    Some(t)
}

/// Items the full probe (1.60.1) asked about: its `C_Item.GetItemInfo`
/// answers (the crate its character carried, which no other recording
/// holds; the Hearthstone, whose bind type the snapshots lack).
const PROBED_ITEMS: &[(i64, &str)] = &[
    (
        6462,
        "18: string Secure Crate, string |cnIQ1:|Hitem:6462::::::::21:1486:::::::::|h[Secure Crate]|h|r, number 1, number 1, number 0, string Quest, string Quest, number 1, string INVTYPE_NON_EQUIP_IGNORE, number 132761, number 0, number 12, number 0, number 4, number 0, nil nil, boolean false, string ",
    ),
    (
        6948,
        "18: string Hearthstone, string |cnIQ1:|Hitem:6948::::::::21:1486:::::::::|h[Hearthstone]|h|r, number 1, number 1, number 0, string Miscellaneous, string Junk, number 1, string INVTYPE_NON_EQUIP_IGNORE, number 134414, number 0, number 15, number 0, number 1, number 0, nil nil, boolean false, string ",
    ),
];

/// The fields of `C_Item.GetItemInfo`'s answer, in order.
const ITEM_INFO_FIELDS: &[&str] = &[
    "name",
    "link",
    "quality",
    "itemLevel",
    "minLevel",
    "type",
    "subType",
    "stackCount",
    "equipLoc",
    "icon",
    "sellPrice",
    "classID",
    "subclassID",
    "bindType",
    "expansionID",
    "setID",
    "isCraftingReagent",
    "description",
];

/// An item the game data's probes recorded (a recipe's reagent or
/// product, the full probe's `PROBED_ITEMS`): defined from its recorded
/// `C_Item.GetItemInfo` (cached, as the client had it), once.
fn probed_item(vm: &mut Vm, id: i64) -> Option<TableRef> {
    let info = crate::recorded::probed(&format!("C_Item.GetItemInfo({id})"))
        .or_else(|| PROBED_ITEMS.iter().find(|(i, _)| *i == id).map(|(_, a)| *a))?;
    let values = crate::traits::Rv::answer(info);
    if values.is_empty() {
        return None;
    }
    let t = vm.new_table();
    for (key, v) in ITEM_INFO_FIELDS.iter().zip(values.iter()) {
        let v = v.to_value(vm);
        set(vm, t, key, v);
    }
    set(vm, t, "cached", Value::Bool(true));
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm).world.data.items.insert(id, t);
    Some(t)
}

/// An item's data: defined, else probed (`probed_item`). No ID below 1
/// names an item (a snapshot's empty ammo slot holds item 0; the full
/// probe: item 0 exists for no item function).
pub fn item_data(vm: &mut Vm, id: i64) -> Option<TableRef> {
    if id < 1 {
        return None;
    }
    match engine(vm).world.data.items.get(&id).copied() {
        Some(t) => {
            completed_item(vm, id, t);
            Some(t)
        }
        None => probed_item(vm, id),
    }
}

/// A defined item without its bind type (a snapshot's, which lacks it)
/// given the probes' recorded fields it lacks.
fn completed_item(vm: &mut Vm, id: i64, t: TableRef) {
    let key = vm.str("bindType");
    if !vm.heap.table(t).get(key).is_nil() {
        return;
    }
    let Some(info) = crate::recorded::probed(&format!("C_Item.GetItemInfo({id})"))
        .or_else(|| PROBED_ITEMS.iter().find(|(i, _)| *i == id).map(|(_, a)| *a))
    else {
        return;
    };
    for (k, v) in ITEM_INFO_FIELDS
        .iter()
        .zip(crate::traits::Rv::answer(info).iter())
    {
        let k = vm.str(k);
        if vm.heap.table(t).get(k).is_nil() {
            let v = v.to_value(vm);
            let _ = vm.heap.table_mut(t).set(k, v);
        }
    }
}

/// A spell's data: defined, else probed (`probed_spell`).
pub fn spell_data(vm: &mut Vm, id: i64) -> Option<TableRef> {
    match engine(vm).world.data.spells.get(&id).copied() {
        Some(t) => Some(t),
        None => probed_spell(vm, id),
    }
}

/// A spell of the client's spell table as SuuUI's spell probes read it
/// (`data/camelot-spells.txt`): its name, its subtext ("" where none was
/// read) and its icon file ID (none where none was read).
struct Scanned {
    name: &'static str,
    subtext: &'static str,
    icon: Option<f64>,
}

fn spell_table() -> &'static HashMap<i64, Scanned> {
    static TABLE: std::sync::OnceLock<HashMap<i64, Scanned>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("../data/camelot-spells.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut f = l.split('\t');
                let id = f.next()?.parse().ok()?;
                let name = crate::recorded::unescape(f.next()?);
                let subtext = crate::recorded::unescape(f.next().unwrap_or(""));
                let icon = f.next().and_then(|i| i.parse().ok());
                (!name.is_empty()).then_some((
                    id,
                    Scanned {
                        name,
                        subtext,
                        icon,
                    },
                ))
            })
            .collect()
    })
}

/// The art the client gives a spell without its own: `Interface\Icons\Temp`
/// (the recorded smelting spells' `iconID`, 136235). Unverified estimate
/// for a spell of the spell table whose icon the probes did not read (every
/// recorded spell info has an icon; the true one is unknown here).
const NO_ICON: f64 = 136235.0;

/// A spell no test defined and no probe recorded, from the client's spell
/// table (`spell_table`; camelot's): a table like a defined spell's (name,
/// iconID, subtext, cached), made once. The spells' data files and the
/// recorded answers win over it: callers ask it last. Unverified: its
/// cast time and range (none here, as `get_spell_info` defaults them), and
/// a spell's rank, which the scan read only where the client had it cached
/// (Holy Light's ranks are in the trainers' recordings, not in the scan).
pub fn scanned_spell(vm: &mut Vm, id: i64) -> Option<TableRef> {
    if let Some(t) = engine(vm).world.data.scanned.get(&id).copied() {
        return Some(t);
    }
    if engine(vm).flavor != "camelot" {
        return None;
    }
    let s = spell_table().get(&id)?;
    // The scan's icon, else the recorded spellbook item's (the full probe,
    // 1.60.1: Attack's 133728 and Cannibalize's 132278, which the scan
    // lacked, are their spellbook items' icons).
    let icon = s
        .icon
        .or_else(|| book_icon(vm, id as f64))
        .unwrap_or(NO_ICON);
    let t = vm.new_table();
    let name = vm.str(s.name);
    set(vm, t, "name", name);
    set(vm, t, "iconID", Value::Num(icon));
    let subtext = vm.str(s.subtext);
    set(vm, t, "subtext", subtext);
    set(vm, t, "cached", Value::Bool(true));
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm).world.data.scanned.insert(id, t);
    Some(t)
}

/// The icon of the recorded spellbook item holding the spell.
fn book_icon(vm: &mut Vm, id: f64) -> Option<f64> {
    let Some(Value::Table(item)) = crate::stats::spellbook_item(vm, id) else {
        return None;
    };
    match field(vm, item, "iconID") {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// The call with a spell's name in place of its ID: the spellbook's spell
/// of that name, else a defined spell's (the full probe, 1.60.1:
/// `C_Spell.GetSpellName(6603)`, "Attack", answered as 6603 by
/// `GetSpellInfo`, `GetSpellName`, `GetSpellTexture`, `GetSpellSubtext`,
/// `GetSpellLink`, `GetSpellCooldown`, `DoesSpellExist`,
/// `IsSpellDataCached`, `GetBaseSpell` and `GetOverrideSpell`).
fn by_id(vm: &mut Vm, mut c: NativeCall) -> NativeCall {
    if let Value::Str(_) = vm.hidden(c.arg(1))
        && let Some(id) = crate::spellinfo::spell_of(vm, c.arg(1))
        && !c.args.is_empty()
    {
        c.args[0] = Value::Num(id);
    }
    c
}

/// Whether the client has the spell: defined or probed, of its spell
/// table, or in the recorded spellbook.
fn spell_exists(vm: &mut Vm, id: f64) -> bool {
    spell_data(vm, id as i64).is_some()
        || scanned_spell(vm, id as i64).is_some()
        || crate::stats::spellbook_item(vm, id).is_some()
}

/// A spell's name: its data's (`spell_data`), else the client's spell
/// table's (`scanned_spell`); nil for one the engine does not know.
pub fn spell_name(vm: &mut Vm, id: i64) -> Value {
    match spell_data(vm, id).or_else(|| scanned_spell(vm, id)) {
        Some(t) => field(vm, t, "name"),
        None => Value::Nil,
    }
}

/// The spell table's spell an ID argument names (none for a name: the
/// client finds spells by name in the spellbook, not in its whole table).
fn scanned_arg(vm: &mut Vm, c: &NativeCall) -> Option<(i64, TableRef)> {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) => scanned_spell(vm, n as i64).map(|t| (n as i64, t)),
        _ => None,
    }
}

/// The spell a `spellIdentifier` names: an ID or a defined spell's name.
fn spell(vm: &mut Vm, c: &NativeCall) -> Option<(i64, TableRef)> {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) => {
            let id = n as i64;
            spell_data(vm, id).map(|t| (id, t))
        }
        Value::Str(s) => {
            let name = s;
            let spells: Vec<(i64, TableRef)> = engine(vm)
                .world
                .data
                .spells
                .iter()
                .map(|(k, v)| (*k, *v))
                .collect();
            spells.into_iter().find(|(_, t)| {
                let k = vm.str("name");
                matches!(vm.heap.table(*t).get(k), Value::Str(n) if n == name)
            })
        }
        _ => None,
    }
}

/// The spell a call names that no test defined and no probe recorded: the
/// client's recorded answer to the call wins (`Err`, its values), else
/// the spell table's spell (`Ok`), else what an unanswered call gets.
fn unrecorded(vm: &mut Vm, c: &NativeCall) -> Result<(i64, TableRef), Vec<Value>> {
    if let Some(name) = crate::recorded::called_name(vm, c)
        && let Some(values) = crate::recorded::lookup(vm, &name, c)
    {
        return Err(values);
    }
    match scanned_arg(vm, c) {
        Some(s) => Ok(s),
        None => Err(crate::recorded::client_answer(vm, c)),
    }
}

/// `C_Spell.GetSpellInfo(spell)`: a defined or probed spell's info, else
/// the client's recorded answer, else the spell table's spell's.
fn get_spell_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    let (id, s) = match spell(vm, &c) {
        Some(found) => found,
        None => match unrecorded(vm, &c) {
            Ok(found) => found,
            Err(values) => return Ok(values),
        },
    };
    let t = vm.new_table();
    let name = field(vm, s, "name");
    set(vm, t, "name", name);
    let icon = field(vm, s, "iconID");
    set(vm, t, "iconID", icon);
    set(vm, t, "originalIconID", icon);
    for (k, d) in [("castTime", 0.0), ("minRange", 0.0), ("maxRange", 0.0)] {
        let v = match field(vm, s, k) {
            Value::Nil => Value::Num(d),
            v => v,
        };
        set(vm, t, k, v);
    }
    set(vm, t, "spellID", Value::Num(id as f64));
    Ok(vec![Value::Table(t)])
}

fn spell_field(key: &'static str) -> impl Fn(&mut Vm, NativeCall) -> Ret {
    move |vm: &mut Vm, c: NativeCall| {
        let c = by_id(vm, c);
        spell_field_of(vm, &c, key)
    }
}

fn spell_field_of(vm: &mut Vm, c: &NativeCall, key: &str) -> Ret {
    match spell(vm, c) {
        Some((_, s)) => Ok(vec![field(vm, s, key)]),
        None => match unrecorded(vm, c) {
            Ok((_, s)) => Ok(match field(vm, s, key) {
                // A spell of the client's table without a description: an
                // empty one (the full probe, 1.60.1: Attack's ""; unverified:
                // a spell whose text the engine lacks).
                Value::Nil if key == "description" => vec![vm.str("")],
                Value::Nil => vec![],
                v => vec![v],
            }),
            Err(values) => Ok(values),
        },
    }
}

macro_rules! spell_fields {
    ($($name:ident => $key:literal),*) => {
        $(fn $name(vm: &mut Vm, c: NativeCall) -> Ret {
            spell_field($key)(vm, c)
        })*
    };
}

spell_fields!(
    get_spell_name => "name",
    get_spell_description => "description",
    get_spell_charges => "charges"
);

/// `C_Spell.GetSpellTexture(spell)`: its icon, its original icon (the
/// same) and no conditional icon (the full probe, 1.60.1: three values for
/// Attack and Cannibalize).
fn get_spell_texture(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    let r = spell_field_of(vm, &c, "iconID")?;
    Ok(match r.as_slice() {
        [icon @ Value::Num(_)] => vec![*icon, *icon, Value::Nil],
        _ => r,
    })
}

/// `GetSpellCooldown`: the spell's `cooldown`, or none running; any
/// spell of the client's (the full probe, 1.60.1: Attack's and
/// Cannibalize's, by ID or name), nothing for one it lacks.
fn get_spell_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    let found = match spell(vm, &c) {
        Some(found) => Some(found),
        None => match vm.hidden(c.arg(1)) {
            Value::Num(n) if spell_exists(vm, n) => Some((n as i64, vm.new_table())),
            _ => None,
        },
    };
    let Some((id, s)) = found else {
        return Ok(vec![]);
    };
    if let Value::Table(cd) = field(vm, s, "cooldown") {
        return Ok(vec![Value::Table(cd)]);
    }
    // One a use began (the data's `cooldown` seconds, as the spell's
    // `C_Spell.GetSpellCooldownDuration` has it): running since the use,
    // active (the documentation's fields; unverified: `isOnGCD` false).
    let now = crate::clock::now(vm);
    let running = match field(vm, s, "cooldown") {
        Value::Num(cd) => engine(vm)
            .units
            .casting
            .ready_at
            .get(&id)
            .copied()
            .filter(|at| *at > now)
            .map(|at| (at - cd, cd)),
        _ => None,
    };
    let (start, length) = running.unwrap_or((0.0, 0.0));
    let t = vm.new_table();
    if running.is_some() {
        set(vm, t, "isOnGCD", Value::Bool(false));
    }
    for (k, v) in [
        ("startTime", Value::Num(start)),
        ("duration", Value::Num(length)),
        ("modRate", Value::Num(1.0)),
        ("isEnabled", Value::Bool(true)),
        ("isActive", Value::Bool(running.is_some())),
    ] {
        set(vm, t, k, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `C_Spell.GetSpellLink(spell)`: a defined or probed spell's link, else
/// the client's recorded answer, else the spell table's spell's. The link
/// is `spell:<id>:0` (its glyph): the client's spellbook item links are
/// (`C_SpellBook.GetSpellBookItemLink`, the recorded character's Dodge
/// `|cff71d5ff|Hspell:81:0|h[Dodge]|h|r`), and so is warcraft.wiki.gg's
/// Hyperlinks page's (`spell:spellId:glyphId`). Unverified for this
/// function itself.
///
/// A glyph above 0 gives no link (the full probe, 1.60.1: glyph 1
/// nothing, 0 and none the link; the spellbook's -1 the link too;
/// unverified: a real glyph).
fn get_spell_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    if matches!(vm.hidden(c.arg(2)), Value::Num(g) if g > 0.0) {
        return Ok(vec![]);
    }
    let (id, s) = match spell(vm, &c) {
        Some(found) => found,
        None => match unrecorded(vm, &c) {
            Ok(found) => found,
            Err(values) => return Ok(values),
        },
    };
    let name = match field(vm, s, "name") {
        Value::Str(n) => String::from_utf8_lossy(vm.str_bytes(n)).into_owned(),
        _ => return Ok(vec![]),
    };
    Ok(vec![
        vm.str(&format!("|cff71d5ff|Hspell:{id}:0|h[{name}]|h|r")),
    ])
}

/// `C_SpellBook.IsSpellKnown(spellID [, bank])`: a defined spell's
/// `known` (`petKnown` for the pet's bank, 1), else whether the player's
/// recorded spellbook holds it (the full probe, 1.60.1: Attack and
/// Cannibalize known for bank 0, none and `"x"`, not for the pet's) or the
/// player learned it from a trainer in the test.
fn is_spell_known(vm: &mut Vm, c: NativeCall) -> Ret {
    let pet = matches!(c.arg(2), Value::Num(n) if n == 1.0);
    let Some((_, s)) = spell(vm, &c) else {
        let book = match vm.hidden(c.arg(1)) {
            Value::Num(n) if !pet => {
                crate::stats::spellbook_slot_of(vm, n).is_some() || crate::trainer::learned(vm, n)
            }
            _ => false,
        };
        return Ok(vec![Value::Bool(book)]);
    };
    // Spell bank 1 is the pet's.
    let key = if pet { "petKnown" } else { "known" };
    let taught =
        !pet && matches!(vm.hidden(c.arg(1)), Value::Num(n) if crate::trainer::learned(vm, n));
    Ok(vec![Value::Bool(field(vm, s, key).truthy() || taught)])
}

/// `C_Item.IsEquippableItem(item)`: an item with an equipment slot (its
/// `equipLoc`, `INVTYPE_NON_EQUIP_IGNORE` and none not); false for one not
/// known (unverified: an item whose data is not loaded).
fn is_equippable_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = item_id(vm, c.arg(1)).and_then(|id| item_data(vm, id)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let equippable = match field(vm, t, "equipLoc") {
        Value::Str(s) => {
            let loc = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            !loc.is_empty() && !loc.starts_with("INVTYPE_NON_EQUIP")
        }
        _ => false,
    };
    Ok(vec![Value::Bool(equippable)])
}

/// `C_Item.GetItemSpell(item)`: the spell its use casts (a defined
/// item's `spell`, else the one the client named for it,
/// `iteminfo::use_spell`: the full probe, 1.60.1, the Hearthstone's 8690
/// by its `item:` string): its name and ID; nothing for an item without
/// one.
fn get_item_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_id(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let Some(spell) = crate::iteminfo::use_spell(vm, id) else {
        return Ok(vec![]);
    };
    let name = spell_name(vm, spell as i64);
    Ok(vec![name, Value::Num(spell)])
}

/// `C_Spell.GetBaseSpell(spell [, spec])`: the spell's ID, as the
/// documentation says for a spell without an override (the engine models
/// none); a name the spell's ID, nothing for a spell not known by it.
///
/// The full probe (1.60.1): a name the spellbook's spell's ID, a string
/// naming no spell 0 (as `GetOverrideSpell`).
fn get_base_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    get_override_spell(vm, c)
}

/// `C_SpellBook.FindBaseSpellByID(spellID)`,
/// `C_SpellBook.FindSpellOverrideByID(spellID)`: a spell the client has
/// itself (no overrides modelled), nil for one it lacks (the full probe,
/// 1.60.1: 0 nil, 1, Attack and Cannibalize themselves).
fn find_spell_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match vm.hidden(c.arg(1)) {
        Value::Num(n) if spell_exists(vm, n) => Value::Num(n),
        _ => Value::Nil,
    }])
}

/// `C_Spell.GetSpellIDForSpellIdentifier(spell)`: the ID of a spell the
/// client has, a name's too; nothing for none (the full probe, 1.60.1: 0
/// and `"x"` nothing).
fn get_spell_id_for_identifier(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    Ok(match vm.hidden(c.arg(1)) {
        Value::Num(n) if spell_exists(vm, n) => vec![Value::Num(n)],
        _ => vec![],
    })
}

/// `C_Spell.IsAutoAttackSpell(spell)`: Attack (6603), by ID or name (the
/// full probe, 1.60.1).
fn is_auto_attack_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    Ok(vec![Value::Bool(
        matches!(vm.hidden(c.arg(1)), Value::Num(n) if n == 6603.0),
    )])
}

/// `C_Spell.IsRangedAutoAttackSpell(spell)`: Auto Shot (75) and Shoot
/// (5019), and a spell the client lacks (the full probe, 1.60.1: 0 and
/// `"x"` true, Attack and Cannibalize false; unverified: the ranged ones).
fn is_ranged_auto_attack_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    let ranged = match vm.hidden(c.arg(1)) {
        Value::Num(n) => n == 75.0 || n == 5019.0 || !spell_exists(vm, n),
        _ => true,
    };
    Ok(vec![Value::Bool(ranged)])
}

/// `C_SpellBook.IsSpellKnownOrInSpellBook(spellID [, bank,
/// includeOverrides])`: known (`C_SpellBook.IsSpellKnown`) or in the book
/// (`C_SpellBook.IsSpellInSpellBook`), as its documentation says.
/// Unverified: overrides.
fn is_spell_known_or_in_spellbook(vm: &mut Vm, c: NativeCall) -> Ret {
    let copy = |c: &NativeCall| NativeCall {
        args: c.args.clone(),
        taints: c.taints.clone(),
        origins: c.origins.clone(),
        upvals: c.upvals.clone(),
        upval_taints: c.upval_taints.clone(),
    };
    let known = is_spell_known(vm, copy(&c))?;
    if known.first().is_some_and(|v| v.truthy()) {
        return Ok(known);
    }
    crate::stats::is_spell_in_spellbook(vm, c)
}

/// A spell the client has though no test defined it: in the recorded
/// spellbook, or with recorded info (`C_Spell.GetSpellInfo`).
fn client_spell(vm: &mut Vm, c: &NativeCall) -> bool {
    let Value::Num(id) = vm.hidden(c.arg(1)) else {
        return false;
    };
    crate::stats::spellbook_item(vm, id).is_some()
        || crate::recorded::lookup_text(vm, &format!("C_Spell.GetSpellInfo({})", id as i64))
            .is_some_and(|v| !v.is_empty() && !v[0].is_nil())
}

/// `C_Spell.IsSpellDataCached(spell)`: a defined spell's `cached`, else
/// whether the client has it (`client_spell`), or the spell table has it
/// (unverified: the scan read every name without loading any, so the
/// client keeps them at hand).
fn is_spell_data_cached(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    Ok(vec![Value::Bool(match spell(vm, &c) {
        Some((_, s)) => !matches!(field(vm, s, "cached"), Value::Bool(false)),
        None => client_spell(vm, &c) || scanned_arg(vm, &c).is_some(),
    })])
}

/// `C_Spell.DoesSpellExist(spell)`: a defined spell, else as the client
/// answered the call, else one the client has (`client_spell`) or one of
/// its spell table (every spell the scan named exists, as
/// `C_Spell.GetSpellName` answered it).
fn does_spell_exist(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    if spell(vm, &c).is_some() {
        return Ok(vec![Value::Bool(true)]);
    }
    if let Some(values) = crate::recorded::lookup(vm, "C_Spell.DoesSpellExist", &c) {
        return Ok(values);
    }
    let exists = client_spell(vm, &c) || scanned_arg(vm, &c).is_some();
    Ok(vec![Value::Bool(exists)])
}

/// `C_Spell.GetSpellSubtext(spell)`: a defined spell's `subtext`, a
/// spellbook spell's (its item's `subName`), else as the client answered,
/// else the spell table's (its rank where the scan read one, else ""
/// as the client answers an unranked spell: the 133 recorded "").
fn get_spell_subtext(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    if let Some((_, s)) = spell(vm, &c) {
        return Ok(vec![field(vm, s, "subtext")]);
    }
    if let Value::Num(id) = vm.hidden(c.arg(1))
        && let Some(Value::Table(item)) = crate::stats::spellbook_item(vm, id)
    {
        return Ok(vec![field(vm, item, "subName")]);
    }
    match unrecorded(vm, &c) {
        Ok((_, s)) => Ok(vec![field(vm, s, "subtext")]),
        Err(values) => Ok(values),
    }
}

/// `C_Spell.GetSpellPowerCost(spell)`: a spell the game data's probes
/// recorded answers as recorded (a trainer's: Holy Light's 110 mana,
/// which SuuUI probe spellbook's tooltip shows), else the client's
/// recorded answer, else nothing.
fn get_spell_power_cost(vm: &mut Vm, c: NativeCall) -> Ret {
    let call = crate::call_text(vm, "C_Spell.GetSpellPowerCost", &c, 1);
    if let Some(answer) = crate::recorded::probed(&call) {
        return Ok(crate::recorded::parse_answer(vm, answer));
    }
    Ok(crate::recorded::client_answer(vm, &c))
}

/// The engine's timers run a native after a delay: the load results.
fn after(vm: &mut Vm, delay: f64, f: NativeFn, args: Vec<Value>) {
    let native = vm.native_with("load", f, args);
    crate::clock::after_engine(vm, delay, native);
}

fn spell_loaded(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = c.upvals.first().copied().unwrap_or(Value::Nil);
    // A spell of the client's spell table loads too (unverified: as a
    // defined one; the scan read every name at once).
    let known = match id {
        Value::Num(n) => {
            engine(vm).world.data.spells.contains_key(&(n as i64))
                || scanned_spell(vm, n as i64).is_some()
        }
        _ => false,
    };
    if let (true, Value::Num(n)) = (known, id)
        && let Some(s) = engine(vm).world.data.spells.get(&(n as i64)).copied()
    {
        set(vm, s, "cached", Value::Bool(true));
    }
    widgets::fire_event(vm, "SPELL_DATA_LOAD_RESULT", &[id, Value::Bool(known)]);
    Ok(vec![])
}

fn request_load_spell_data(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_number(&c, 1)?;
    after(vm, 0.2, spell_loaded, vec![Value::Num(id)]);
    Ok(vec![])
}

// ----------------------------------------------------------------- items

/// The item an `itemInfo` names: an ID, a link, or a cached item's name.
pub(crate) fn item_id(vm: &mut Vm, v: Value) -> Option<i64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n as i64),
        Value::Str(s) => {
            let text = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            if let Some(rest) = text.split("item:").nth(1) {
                return rest
                    .split(|ch: char| !ch.is_ascii_digit())
                    .next()
                    .and_then(|d| d.parse().ok());
            }
            if let Ok(n) = text.parse() {
                return Some(n);
            }
            let items: Vec<(i64, TableRef)> = engine(vm)
                .world
                .data
                .items
                .iter()
                .map(|(k, v)| (*k, *v))
                .collect();
            items.into_iter().find_map(|(id, t)| {
                let cached = field(vm, t, "cached").truthy();
                let name = field(vm, t, "name");
                (cached && matches!(name, Value::Str(n) if n == s)).then_some(id)
            })
        }
        _ => None,
    }
}

fn cached_item(vm: &mut Vm, c: &NativeCall) -> Option<TableRef> {
    let id = item_id(vm, c.arg(1))?;
    let t = item_data(vm, id)?;
    field(vm, t, "cached").truthy().then_some(t)
}

/// `C_Item.GetItemInfo(item)`: a cached item's info; one not cached yet,
/// nothing, and one defined `cached = false` (its data's load not asked)
/// is fetched (`GET_ITEM_INFO_RECEIVED(id, true)` its `delay` later,
/// cached from then; recorded: the WoW Token's as the auction house
/// opened, 0.034 s; unverified: an item whose load is asked too).
fn get_item_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = cached_item(vm, &c) else {
        if let Some(id) = item_id(vm, c.arg(1))
            && !engine(vm).world.data.item_requests.contains(&id)
            && let Some(t) = item_data(vm, id)
            && matches!(field(vm, t, "cached"), Value::Bool(false))
            && !field(vm, t, "fetching").truthy()
            && !field(vm, t, "missing").truthy()
        {
            set(vm, t, "fetching", Value::Bool(true));
            let delay = match field(vm, t, "delay") {
                Value::Num(d) => d,
                _ => 0.2,
            };
            after(vm, delay, item_info_received, vec![Value::Num(id as f64)]);
        }
        return Ok(vec![]);
    };
    let mut out: Vec<Value> = ITEM_INFO_FIELDS.iter().map(|k| field(vm, t, k)).collect();
    // The link, made as the client makes one, where the data has none (a
    // test's `mock.defineItem`): the client always gives one.
    if out[1].is_nil()
        && let Some(id) = item_id(vm, c.arg(1))
    {
        out[1] = crate::character::link_of_id(vm, id).unwrap_or(Value::Nil);
    }
    // The expansion 0 where the data has none (recorded: all 609 of the
    // client's GetItemInfo answers in the probes, Forever's items), and
    // eighteen values: no set, not a crafting reagent, no description where
    // the data has none (recorded: the full probe's and the profession
    // probes' answers, 1.60.1, all eighteen).
    if out[14].is_nil() {
        out[14] = Value::Num(0.0);
    }
    if out[16].is_nil() {
        out[16] = Value::Bool(false);
    }
    if out[17].is_nil() {
        out[17] = vm.str("");
    }
    Ok(out)
}

fn get_item_info_instant(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_id(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    // No item below 1: nothing (the full probe: -1, 0).
    let Some(t) = item_data(vm, id) else {
        return Ok(if id < 1 {
            vec![]
        } else {
            vec![Value::Num(id as f64)]
        });
    };
    let mut out = vec![Value::Num(id as f64)];
    for k in [
        "type",
        "subType",
        "equipLoc",
        "icon",
        "classID",
        "subclassID",
    ] {
        out.push(field(vm, t, k));
    }
    Ok(out)
}

/// A cached item's field; nil for no item (the full probe: "x", -1),
/// nothing for one not loaded yet.
fn item_field(vm: &mut Vm, c: &NativeCall, key: &str) -> Ret {
    match cached_item(vm, c) {
        Some(t) => Ok(vec![field(vm, t, key)]),
        None => {
            let known = item_id(vm, c.arg(1)).is_some_and(|id| item_data(vm, id).is_some());
            Ok(if known { vec![] } else { vec![Value::Nil] })
        }
    }
}

fn get_item_name_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    item_field(vm, &c, "name")
}

/// `C_Item.GetItemQualityByID(itemInfo)`: its data's quality (the
/// client's needs its data loaded: a cached item's).
fn get_item_quality_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    item_field(vm, &c, "quality")
}

/// `C_Item.GetItemMaxStackSizeByID(itemInfo)`: its data's stack size.
fn get_item_max_stack_size_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    item_field(vm, &c, "stackCount")
}

/// The question mark icon (`QUESTION_MARK_ICON`, 134400), which Blizzard's
/// PvP UI shows for a reward item `C_Item.GetItemIconByID` has no icon for.
const QUESTION_MARK_ICON: f64 = 134400.0;

/// `C_Item.GetItemIconByID(item)`: a defined or probed item's icon, else
/// as the client answered, else the question mark's for an item the engine
/// has no data for. Unverified estimate: the client reads icons from its
/// item table without loading the item (as `GetItemInfoInstant` does), so
/// an item it has always answers one, and the engine cannot know which;
/// AtlasLoot's price buttons ask again until they get an icon (a stack
/// overflow on nothing). The full probe: the question mark for -1, 0 and
/// a text naming no item too.
fn get_item_icon_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = item_id(vm, c.arg(1));
    if let Some(t) = id.and_then(|id| item_data(vm, id)) {
        return Ok(vec![field(vm, t, "icon")]);
    }
    if let Some(values) = crate::recorded::lookup(vm, "C_Item.GetItemIconByID", &c) {
        return Ok(values);
    }
    Ok(vec![Value::Num(QUESTION_MARK_ICON)])
}

/// `C_Item.DoesItemExistByID(item)`: a defined item (not `missing`),
/// else as the client answered. The item may be a link or a name too
/// (the API documentation's `ItemInfo`; Blizzard's `ItemMixin:IsItemEmpty`
/// asks it with an item's link, as the profession window's reagent
/// containers do: the engine answered nothing for links before, and the
/// mixin's `ContinueWithCancelOnItemLoad` failed inside
/// `secureexecuterange`). Unverified for names (as `item_id` reads one: a
/// cached item's).
fn does_item_exist_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let defined = match item_id(vm, c.arg(1)) {
        Some(id) => item_data(vm, id),
        None => None,
    };
    match defined {
        Some(t) => Ok(vec![Value::Bool(!field(vm, t, "missing").truthy())]),
        // Else as the client answered, else no (the full probe: "x", -1,
        // 0; unverified: an item that exists unknown to the engine).
        None => {
            let answer = crate::recorded::client_answer(vm, &c);
            Ok(if answer.is_empty() {
                vec![Value::Bool(false)]
            } else {
                answer
            })
        }
    }
}

fn is_item_data_cached(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(cached_item(vm, &c).is_some())])
}

fn item_info_received(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = c.upvals.first().copied().unwrap_or(Value::Nil);
    if let Value::Num(n) = id
        && let Some(t) = item_data(vm, n as i64)
    {
        set(vm, t, "cached", Value::Bool(true));
        widgets::fire_event(vm, "GET_ITEM_INFO_RECEIVED", &[id, Value::Bool(true)]);
    }
    Ok(vec![])
}

fn item_loaded(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = c.upvals.first().copied().unwrap_or(Value::Nil);
    let item = match id {
        Value::Num(n) => item_data(vm, n as i64),
        _ => None,
    };
    let ok = match item {
        Some(t) if !field(vm, t, "missing").truthy() => {
            set(vm, t, "cached", Value::Bool(true));
            true
        }
        _ => false,
    };
    widgets::fire_event(vm, "ITEM_DATA_LOAD_RESULT", &[id, Value::Bool(ok)]);
    Ok(vec![])
}

fn request_load_item_data(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_id(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    engine_mut(vm).world.data.item_requests.push(id);
    let delay = match item_data(vm, id) {
        Some(t) => match field(vm, t, "delay") {
            Value::Num(d) => d,
            _ => 0.2,
        },
        None => 0.2,
    };
    after(vm, delay, item_loaded, vec![Value::Num(id as f64)]);
    Ok(vec![])
}

/// The item subclasses the answers probe did not ask (it asked classes
/// 0, 1, 2, 4, 6, 7, 9, 11, 12, 13 and 15), as AtlasLoot's English table of
/// the client's names has them (`Core/ItemInfo.lua`'s `LOC_DATA`, which
/// asks each at login and fails on a missing one). Unverified estimates:
/// the recorded single-subclass classes name their subclass after
/// themselves (Reagent, Quest), as these do but Permanent.
const UNPROBED_SUBCLASSES: &[(i64, i64, &str)] = &[
    (3, 0, "Jewelry(OBSOLETE)"),
    (8, 0, "Generic(OBSOLETE)"),
    (10, 0, "Money(OBSOLETE)"),
    (14, 0, "Permanent"),
];

/// `C_Item.GetItemSubClassInfo(class, subclass)`: as the client answered
/// (`/stdtest engine answers`), else an unprobed subclass's estimate
/// (`UNPROBED_SUBCLASSES`: its name, not a weapon-type subclass as the
/// recorded ones answer false), else as an unrecorded call answers.
fn get_item_subclass_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(values) = crate::recorded::lookup(vm, "C_Item.GetItemSubClassInfo", &c) {
        return Ok(values);
    }
    let (class, sub) = (vm.hidden(c.arg(1)), vm.hidden(c.arg(2)));
    if let (Value::Num(class), Value::Num(sub)) = (class, sub)
        && let Some((_, _, name)) = UNPROBED_SUBCLASSES
            .iter()
            .find(|(k, s, _)| *k as f64 == class && *s as f64 == sub)
    {
        return Ok(vec![vm.str(name), Value::Bool(false)]);
    }
    crate::recorded::recorded(vm, c)
}

/// `C_Item.GetItemClassInfo(classID)`: as the client answered (a fraction
/// as its whole number), else nil (the full probe: -1, 2147483647).
fn get_item_class_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(crate::recorded::lookup(vm, "C_Item.GetItemClassInfo", &c)
        .unwrap_or_else(|| vec![Value::Nil]))
}

/// `C_Item.GetItemInventorySlotInfo(inventoryType)`: as the client
/// answered; the profession tool's and gear's (29, 30) their
/// `INVTYPE_PROFESSION_*` strings (the full probe), else as an unrecorded
/// call answers.
fn get_item_inventory_slot_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(values) = crate::recorded::lookup(vm, "C_Item.GetItemInventorySlotInfo", &c) {
        return Ok(values);
    }
    let global = match vm.hidden(c.arg(1)) {
        Value::Num(n) if n.trunc() == 29.0 => Some(("INVTYPE_PROFESSION_TOOL", "Profession Tool")),
        Value::Num(n) if n.trunc() == 30.0 => {
            Some(("INVTYPE_PROFESSION_GEAR", "Profession Equipment"))
        }
        _ => None,
    };
    if let Some((name, text)) = global {
        return Ok(vec![match vm.get_global(name) {
            Value::Str(s) => Value::Str(s),
            _ => vm.str(text),
        }]);
    }
    crate::recorded::recorded(vm, c)
}

/// The functions with behaviour here.
/// `C_Spell.GetOverrideSpell(spell)`: the spell itself (unverified: no
/// override modelled); a name its spell's ID, a string naming no spell 0
/// (the full probe, 1.60.1: "Attack" 6603, `"x"` 0).
fn get_override_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let c = by_id(vm, c);
    Ok(match vm.hidden(c.arg(1)) {
        Value::Num(n) => vec![Value::Num(n)],
        Value::Str(_) => vec![Value::Num(0.0)],
        _ => vec![],
    })
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_Spell.GetSpellInfo", get_spell_info),
        ("C_Spell.GetSpellName", get_spell_name),
        ("C_Spell.GetSpellTexture", get_spell_texture),
        ("C_Spell.GetSpellDescription", get_spell_description),
        ("C_Spell.GetSpellCharges", get_spell_charges),
        ("C_Spell.GetSpellCooldown", get_spell_cooldown),
        ("C_Spell.GetSpellLink", get_spell_link),
        ("C_Spell.GetOverrideSpell", get_override_spell),
        ("C_SpellBook.FindSpellOverrideByID", find_spell_by_id),
        ("C_SpellBook.FindBaseSpellByID", find_spell_by_id),
        (
            "C_Spell.GetSpellIDForSpellIdentifier",
            get_spell_id_for_identifier,
        ),
        ("C_Spell.IsAutoAttackSpell", is_auto_attack_spell),
        (
            "C_Spell.GetSchoolString",
            crate::spellinfo::get_school_string,
        ),
        (
            "C_Spell.IsRangedAutoAttackSpell",
            is_ranged_auto_attack_spell,
        ),
        ("C_Spell.IsSpellDataCached", is_spell_data_cached),
        ("C_Spell.DoesSpellExist", does_spell_exist),
        ("C_Spell.GetSpellSubtext", get_spell_subtext),
        ("C_Spell.GetSpellPowerCost", get_spell_power_cost),
        ("C_Spell.RequestLoadSpellData", request_load_spell_data),
        ("C_SpellBook.IsSpellKnown", is_spell_known),
        ("C_Spell.GetBaseSpell", get_base_spell),
        ("C_Item.GetItemSpell", get_item_spell),
        ("C_Item.IsEquippableItem", is_equippable_item),
        (
            "C_SpellBook.IsSpellKnownOrInSpellBook",
            is_spell_known_or_in_spellbook,
        ),
        ("C_Item.GetItemSubClassInfo", get_item_subclass_info),
        ("C_Item.GetItemClassInfo", get_item_class_info),
        (
            "C_Item.GetItemInventorySlotInfo",
            get_item_inventory_slot_info,
        ),
        ("C_Item.GetItemInfo", get_item_info),
        ("C_Item.GetItemInfoInstant", get_item_info_instant),
        ("C_Item.GetItemNameByID", get_item_name_by_id),
        ("C_Item.GetItemIconByID", get_item_icon_by_id),
        ("C_Item.GetItemQualityByID", get_item_quality_by_id),
        (
            "C_Item.GetItemMaxStackSizeByID",
            get_item_max_stack_size_by_id,
        ),
        ("C_Item.IsItemDataCachedByID", is_item_data_cached),
        ("C_Item.DoesItemExistByID", does_item_exist_by_id),
        ("C_Item.RequestLoadItemDataByID", request_load_item_data),
    ]
}
