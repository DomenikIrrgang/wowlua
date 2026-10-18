//! Game data from the tests (decision T203): spells and items a test
//! defines (`defineSpell(id, data)`, `defineItem(id, data, opts)`), read
//! through `C_Spell`, `C_SpellBook` and `C_Item`. An ID nobody defined is
//! the client's own data as recorded (`/stdtest engine answers`; the spells
//! Blizzard's UI names), else data the client does not have. The data
//! stays the test's table, so any value may be a secret.
//!
//! As the M8 mock had them (unverified details): spell data is cached once
//! defined unless `cached = false`; an item arrives `delay` seconds (0.2)
//! after `RequestLoadItemDataByID` unless `cached`, firing
//! `ITEM_DATA_LOAD_RESULT`; `GetItemInfo` gives nothing until then.

use std::collections::HashMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// The defined spells and items: ID → the test's table.
#[derive(Default)]
pub struct GameData {
    pub spells: HashMap<i64, TableRef>,
    pub items: HashMap<i64, TableRef>,
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

/// The spell a `spellIdentifier` names: an ID or a defined spell's name.
fn spell(vm: &mut Vm, c: &NativeCall) -> Option<(i64, TableRef)> {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) => {
            let id = n as i64;
            engine(vm).world.data.spells.get(&id).map(|t| (id, *t))
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

fn get_spell_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((id, s)) = spell(vm, &c) else {
        return Ok(crate::recorded::client_answer(vm, &c));
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
    move |vm: &mut Vm, c: NativeCall| match spell(vm, &c) {
        Some((_, s)) => Ok(vec![field(vm, s, key)]),
        None => Ok(crate::recorded::client_answer(vm, &c)),
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
    get_spell_texture => "iconID",
    get_spell_description => "description",
    get_spell_charges => "charges"
);

/// `GetSpellCooldown`: the spell's `cooldown`, or none running.
fn get_spell_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, s)) = spell(vm, &c) else {
        return Ok(vec![]);
    };
    if let Value::Table(cd) = field(vm, s, "cooldown") {
        return Ok(vec![Value::Table(cd)]);
    }
    let t = vm.new_table();
    for (k, v) in [
        ("startTime", Value::Num(0.0)),
        ("duration", Value::Num(0.0)),
        ("modRate", Value::Num(1.0)),
        ("isEnabled", Value::Bool(true)),
        ("isActive", Value::Bool(false)),
    ] {
        set(vm, t, k, v);
    }
    Ok(vec![Value::Table(t)])
}

fn get_spell_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((id, s)) = spell(vm, &c) else {
        return Ok(vec![]);
    };
    let name = match field(vm, s, "name") {
        Value::Str(n) => String::from_utf8_lossy(vm.str_bytes(n)).into_owned(),
        _ => return Ok(vec![]),
    };
    Ok(vec![
        vm.str(&format!("|cff71d5ff|Hspell:{id}|h[{name}]|h|r")),
    ])
}

fn is_spell_known(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, s)) = spell(vm, &c) else {
        return Ok(vec![Value::Bool(false)]);
    };
    // Spell bank 1 is the pet's.
    let key = if matches!(c.arg(2), Value::Num(n) if n == 1.0) {
        "petKnown"
    } else {
        "known"
    };
    Ok(vec![Value::Bool(field(vm, s, key).truthy())])
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

fn is_spell_data_cached(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(match spell(vm, &c) {
        Some((_, s)) => !matches!(field(vm, s, "cached"), Value::Bool(false)),
        None => client_spell(vm, &c),
    })])
}

/// `C_Spell.DoesSpellExist(spell)`: a defined spell or one the client has.
fn does_spell_exist(vm: &mut Vm, c: NativeCall) -> Ret {
    let exists = spell(vm, &c).is_some() || client_spell(vm, &c);
    Ok(vec![Value::Bool(exists)])
}

/// `C_Spell.GetSpellSubtext(spell)`: a defined spell's `subtext`, a
/// spellbook spell's (its item's `subName`), else as the client answered.
fn get_spell_subtext(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some((_, s)) = spell(vm, &c) {
        return Ok(vec![field(vm, s, "subtext")]);
    }
    if let Value::Num(id) = vm.hidden(c.arg(1))
        && let Some(Value::Table(item)) = crate::stats::spellbook_item(vm, id)
    {
        return Ok(vec![field(vm, item, "subName")]);
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
    let known =
        matches!(id, Value::Num(n) if engine(vm).world.data.spells.contains_key(&(n as i64)));
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
fn item_id(vm: &mut Vm, v: Value) -> Option<i64> {
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
    let t = engine(vm).world.data.items.get(&id).copied()?;
    field(vm, t, "cached").truthy().then_some(t)
}

fn get_item_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = cached_item(vm, &c) else {
        return Ok(vec![]);
    };
    Ok([
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
    ]
    .iter()
    .map(|k| field(vm, t, k))
    .collect())
}

fn get_item_info_instant(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_id(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let Some(t) = engine(vm).world.data.items.get(&id).copied() else {
        return Ok(vec![Value::Num(id as f64)]);
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

fn item_field(vm: &mut Vm, c: &NativeCall, key: &str) -> Ret {
    match cached_item(vm, c) {
        Some(t) => Ok(vec![field(vm, t, key)]),
        None => Ok(vec![]),
    }
}

fn get_item_name_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    item_field(vm, &c, "name")
}

fn get_item_icon_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = item_id(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    match engine(vm).world.data.items.get(&id).copied() {
        Some(t) => Ok(vec![field(vm, t, "icon")]),
        None => Ok(vec![]),
    }
}

/// `C_Item.DoesItemExistByID(item)`: a defined item (not `missing`),
/// else as the client answered.
fn does_item_exist_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let defined = match vm.hidden(c.arg(1)) {
        Value::Num(n) => engine(vm).world.data.items.get(&(n as i64)).copied(),
        _ => None,
    };
    match defined {
        Some(t) => Ok(vec![Value::Bool(!field(vm, t, "missing").truthy())]),
        None => Ok(crate::recorded::client_answer(vm, &c)),
    }
}

fn is_item_data_cached(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(cached_item(vm, &c).is_some())])
}

fn item_loaded(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = c.upvals.first().copied().unwrap_or(Value::Nil);
    let item = match id {
        Value::Num(n) => engine(vm).world.data.items.get(&(n as i64)).copied(),
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
    let delay = match engine(vm).world.data.items.get(&id).copied() {
        Some(t) => match field(vm, t, "delay") {
            Value::Num(d) => d,
            _ => 0.2,
        },
        None => 0.2,
    };
    after(vm, delay, item_loaded, vec![Value::Num(id as f64)]);
    Ok(vec![])
}

/// The functions with behaviour here.
/// `C_Spell.GetOverrideSpell(spell)`: the spell itself (unverified: no
/// override modelled).
fn get_override_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match vm.hidden(c.arg(1)) {
        Value::Num(n) => vec![Value::Num(n)],
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
        ("C_Spell.IsSpellDataCached", is_spell_data_cached),
        ("C_Spell.DoesSpellExist", does_spell_exist),
        ("C_Spell.GetSpellSubtext", get_spell_subtext),
        ("C_Spell.RequestLoadSpellData", request_load_spell_data),
        ("C_SpellBook.IsSpellKnown", is_spell_known),
        ("C_Item.GetItemInfo", get_item_info),
        ("C_Item.GetItemInfoInstant", get_item_info_instant),
        ("C_Item.GetItemNameByID", get_item_name_by_id),
        ("C_Item.GetItemIconByID", get_item_icon_by_id),
        ("C_Item.IsItemDataCachedByID", is_item_data_cached),
        ("C_Item.DoesItemExistByID", does_item_exist_by_id),
        ("C_Item.RequestLoadItemDataByID", request_load_item_data),
    ]
}
