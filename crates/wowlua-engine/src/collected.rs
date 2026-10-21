//! The account's collections as a test or its world profile sets them
//! (decision T235): the mount journal's mounts (`mock.setMounts`), the toy
//! box's toys (`mock.setToys`) and the wardrobe's collected appearances
//! (`mock.setAppearances`), each recorded in the client by `/stdtest
//! snapshot`. Until a test sets one, it answers as the recorded account
//! (`collections.rs`: a journal of mounts without data, none collected; no
//! toy; the recorded wardrobe).
//!
//! The setters change state only, as no client event matches a whole
//! collection changing; a test fires the client's own events for what it
//! models (`NEW_MOUNT_ADDED`, `NEW_TOY_ADDED`, …).
//!
//! The journals' lists follow their filters and searches (`collections.rs`
//! keeps them): the mount journal's collected, not collected and unusable
//! checks (the unusable one for collected and uncollected mounts alike),
//! the toy box's collected and unusable ones (it lists collected toys
//! only), each one's search by name. Recorded (`/stdtest engine journals`
//! and `journals2`, `tests/client/forever-camelot-journals.lua`): the mount
//! list by name, an uncollected mount's favourite (offered, not taken), the
//! filters' events, an appearance's favourite's events. Unverified: where
//! favourites and collected mounts and toys go in the lists (first here),
//! the source and type filters (not applied), the events of a collected
//! mount's or toy's favourite (`MOUNT_JOURNAL_SEARCH_UPDATED`,
//! `TOYS_UPDATED(itemID)` here).

use std::collections::BTreeSet;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// A mount of the journal.
#[derive(Clone, Debug)]
pub struct Mount {
    pub id: i64,
    pub name: Value,
    pub spell: f64,
    pub icon: Value,
    pub source_type: f64,
    pub description: Value,
    pub source: Value,
    pub display: Value,
    pub mount_type: f64,
    pub model_scene: f64,
    pub anim: f64,
    pub visual_kit: f64,
    pub self_mount: bool,
    pub no_preview: bool,
    /// 0 the Horde's, 1 the Alliance's; none for both.
    pub faction: Option<f64>,
    /// Not shown for this character (`shouldHideOnChar`).
    pub hidden: bool,
    pub collected: bool,
    pub favorite: bool,
    pub usable: bool,
    pub steady_flight: bool,
    /// The item that teaches it (`GetMountFromItem`).
    pub item: Option<i64>,
}

/// A toy of the toy box, by its item.
#[derive(Clone, Debug)]
pub struct Toy {
    pub item: i64,
    pub collected: bool,
    pub favorite: bool,
    pub usable: bool,
    pub fanfare: bool,
}

/// The collections a test set; `None`: as recorded.
#[derive(Default)]
pub struct Collected {
    /// By mount ID.
    pub mounts: Option<Vec<Mount>>,
    /// By item ID.
    pub toys: Option<Vec<Toy>>,
    /// The wardrobe's collected sources.
    pub sources: Option<BTreeSet<i64>>,
    /// The wardrobe's favourite appearances (visual IDs).
    pub favorites: Option<BTreeSet<i64>>,
    /// A `TOYS_UPDATED` due on the next frame.
    pub toys_pending: bool,
}

// ---------------------------------------------------------------------------
// Helpers.

fn get(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn num_at(vm: &mut Vm, t: TableRef, key: &str) -> Option<f64> {
    match get(vm, t, key) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn bool_at(vm: &mut Vm, t: TableRef, key: &str, default: bool) -> bool {
    match get(vm, t, key) {
        Value::Bool(b) => b,
        Value::Nil => default,
        _ => true,
    }
}

fn arg_num(vm: &Vm, c: &NativeCall, n: usize) -> Option<f64> {
    match vm.hidden(c.arg(n)) {
        Value::Num(x) => Some(x),
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).trim().parse().ok(),
        _ => None,
    }
}

fn text_of(vm: &Vm, v: Value) -> String {
    match vm.hidden(v) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    }
}

/// A table's entries with whole-number keys, by key.
fn by_id(vm: &mut Vm, t: TableRef) -> Vec<(i64, Value)> {
    let mut out: Vec<(i64, Value)> = vm
        .heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match k {
            Value::Num(n) if n.fract() == 0.0 => Some((n as i64, v)),
            _ => None,
        })
        .collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

fn list(vm: &mut Vm, values: impl IntoIterator<Item = Value>) -> Value {
    let t = vm.new_table();
    for (i, v) in values.into_iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num((i + 1) as f64), v);
    }
    Value::Table(t)
}

/// The answer recorded for the call (this call's, else the load's).
fn recorded(vm: &mut Vm, name: &str, c: &NativeCall) -> Ret {
    Ok(crate::facts::recorded_answer(vm, name, c))
}

/// Whether `name` holds `search` (both lower-cased; an empty search holds
/// everything).
fn matches(name: &str, search: &str) -> bool {
    search.is_empty() || name.to_lowercase().contains(&search.to_lowercase())
}

// ---------------------------------------------------------------------------
// Mounts.

fn mounts(vm: &Vm) -> Option<&Vec<Mount>> {
    engine(vm).world.collected.mounts.as_ref()
}

fn mount(vm: &Vm, id: i64) -> Option<Mount> {
    mounts(vm)?.iter().find(|m| m.id == id).cloned()
}

/// `mock.setMounts({ [mountID] = { name, spellID, icon, … } })`: the
/// journal's mounts, the whole journal (`MOUNT_JOURNAL_SEARCH_UPDATED`).
/// A mount's name and icon are its spell's unless given; it is collected
/// and usable unless it says not.
pub fn control_set_mounts(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setMounts: a table of mounts by mount ID expected"));
    };
    let mut out = Vec::new();
    for (id, data) in by_id(vm, t) {
        let Value::Table(d) = data else {
            return Err(vm.error(&format!("setMounts: mount {id}'s data is not a table")));
        };
        let Some(spell) = num_at(vm, d, "spellID") else {
            return Err(vm.error(&format!("setMounts: mount {id} has no spellID")));
        };
        let spell_data = crate::gamedata::spell_data(vm, spell as i64)
            .or_else(|| crate::gamedata::scanned_spell(vm, spell as i64));
        let mut name = get(vm, d, "name");
        if name.is_nil() {
            name = crate::gamedata::spell_name(vm, spell as i64);
        }
        let mut icon = get(vm, d, "icon");
        if icon.is_nil()
            && let Some(s) = spell_data
        {
            icon = get(vm, s, "iconID");
        }
        let empty = vm.str("");
        let description = match get(vm, d, "description") {
            Value::Nil => empty,
            v => v,
        };
        let source = match get(vm, d, "source") {
            Value::Nil => empty,
            v => v,
        };
        let display = get(vm, d, "display");
        for v in [name, icon, description, source] {
            widgets::keep(vm, v);
        }
        let m = Mount {
            id,
            name,
            spell,
            icon,
            source_type: num_at(vm, d, "sourceType").unwrap_or(0.0),
            description,
            source,
            display,
            mount_type: num_at(vm, d, "mountType").unwrap_or(230.0),
            model_scene: num_at(vm, d, "modelScene").unwrap_or(4.0),
            anim: num_at(vm, d, "animID").unwrap_or(91.0),
            visual_kit: num_at(vm, d, "visualKit").unwrap_or(0.0),
            self_mount: bool_at(vm, d, "selfMount", false),
            no_preview: bool_at(vm, d, "noPreview", false),
            faction: num_at(vm, d, "faction"),
            hidden: bool_at(vm, d, "hidden", false),
            collected: bool_at(vm, d, "collected", true),
            favorite: bool_at(vm, d, "favorite", false),
            usable: bool_at(vm, d, "usable", true),
            steady_flight: bool_at(vm, d, "steadyFlight", false),
            item: num_at(vm, d, "item").map(|i| i as i64),
        };
        out.push(m);
    }
    engine_mut(vm).world.collected.mounts = Some(out);
    Ok(vec![])
}

/// `GetMountInfoByID`'s values: name, spellID, icon, isActive, isUsable,
/// sourceType, isFavorite, isFactionSpecific, faction, shouldHideOnChar,
/// isCollected, mountID, isSteadyFlight (as recorded for an uncollected
/// mount: not usable).
fn info(m: &Mount) -> Vec<Value> {
    vec![
        m.name,
        Value::Num(m.spell),
        m.icon,
        Value::Bool(false),
        Value::Bool(m.collected && m.usable),
        Value::Num(m.source_type),
        Value::Bool(m.favorite),
        Value::Bool(m.faction.is_some()),
        m.faction.map_or(Value::Nil, Value::Num),
        Value::Bool(m.hidden),
        Value::Bool(m.collected),
        Value::Num(m.id as f64),
        Value::Bool(m.steady_flight),
    ]
}

/// `GetMountInfoExtraByID`'s: creatureDisplayInfoID, description, source,
/// isSelfMount, mountTypeID, uiModelSceneID, animID, spellVisualKitID,
/// disablePlayerMountPreview.
fn extra(m: &Mount) -> Vec<Value> {
    vec![
        m.display,
        m.description,
        m.source,
        Value::Bool(m.self_mount),
        Value::Num(m.mount_type),
        Value::Num(m.model_scene),
        Value::Num(m.anim),
        Value::Num(m.visual_kit),
        Value::Bool(m.no_preview),
    ]
}

/// The journal's list as shown: not hidden for the character, through the
/// collected / not collected / unusable checks and the search; favourites
/// first, then collected ones, then by name (unverified order).
fn displayed(vm: &mut Vm) -> Vec<Mount> {
    let Some(all) = mounts(vm).cloned() else {
        return Vec::new();
    };
    let collected = crate::collections::mount_filter(vm, 1);
    let not_collected = crate::collections::mount_filter(vm, 2);
    let unusable = crate::collections::mount_filter(vm, 3);
    let search = crate::collections::search(vm, "mount.search");
    let mut shown: Vec<(String, Mount)> = all
        .into_iter()
        .filter(|m| !m.hidden)
        .filter(|m| {
            if m.collected {
                collected
            } else {
                not_collected
            }
        })
        // Usable for the character, collected or not (recorded: the
        // unusable check hid 37 of 90 uncollected mounts).
        .filter(|m| unusable || m.usable)
        .map(|m| (text_of(vm, m.name), m))
        .filter(|(name, _)| matches(name, &search))
        .collect();
    shown.sort_by(|(an, a), (bn, b)| {
        (b.favorite && b.collected)
            .cmp(&(a.favorite && a.collected))
            .then(b.collected.cmp(&a.collected))
            .then(an.to_lowercase().cmp(&bn.to_lowercase()))
    });
    shown.into_iter().map(|(_, m)| m).collect()
}

fn displayed_at(vm: &mut Vm, c: &NativeCall) -> Option<Mount> {
    let i = arg_num(vm, c, 1)? as i64;
    if i < 1 {
        return None;
    }
    displayed(vm).into_iter().nth(i as usize - 1)
}

fn get_mount_ids(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(all) = mounts(vm) else {
        return crate::collections::get_mount_ids(vm, c);
    };
    let ids: Vec<Value> = all.iter().map(|m| Value::Num(m.id as f64)).collect();
    Ok(vec![list(vm, ids)])
}

fn get_num_mounts(vm: &mut Vm, c: NativeCall) -> Ret {
    match mounts(vm) {
        Some(all) => Ok(vec![Value::Num(all.len() as f64)]),
        None => recorded(vm, "C_MountJournal.GetNumMounts", &c),
    }
}

fn get_mount_info_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::get_mount_info_by_id(vm, c);
    }
    let id = arg_num(vm, &c, 1).unwrap_or(0.0) as i64;
    Ok(mount(vm, id).map(|m| info(&m)).unwrap_or_default())
}

fn get_mount_info_extra_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::get_mount_info_extra_by_id(vm, c);
    }
    let id = arg_num(vm, &c, 1).unwrap_or(0.0) as i64;
    Ok(mount(vm, id).map(|m| extra(&m)).unwrap_or_default())
}

fn get_num_displayed_mounts(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return recorded(vm, "C_MountJournal.GetNumDisplayedMounts", &c);
    }
    Ok(vec![Value::Num(displayed(vm).len() as f64)])
}

fn get_displayed_mount_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return recorded(vm, "C_MountJournal.GetDisplayedMountInfo", &c);
    }
    Ok(displayed_at(vm, &c).map(|m| info(&m)).unwrap_or_default())
}

/// `GetDisplayedMountID(i)`: 0 past the list (the full probe).
fn get_displayed_mount_id(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return Ok(vec![Value::Num(0.0)]);
    }
    let id = displayed_at(vm, &c).map_or(0.0, |m| m.id as f64);
    Ok(vec![Value::Num(id)])
}

fn get_displayed_mount_info_extra(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::get_displayed_mount_info_extra(vm, c);
    }
    Ok(displayed_at(vm, &c).map(|m| extra(&m)).unwrap_or_default())
}

/// A mount's one display, visible (as `collections.rs` answers a recorded
/// mount's).
fn all_display_info(vm: &mut Vm, m: Option<Mount>) -> Ret {
    let Some(display) = m.map(|m| m.display).filter(|d| !d.is_nil()) else {
        return Ok(vec![]);
    };
    let entry = vm.new_table();
    let k = vm.str("creatureDisplayID");
    let _ = vm.heap.table_mut(entry).set(k, display);
    let k = vm.str("isVisible");
    let _ = vm.heap.table_mut(entry).set(k, Value::Bool(true));
    Ok(vec![list(vm, [Value::Table(entry)])])
}

fn get_displayed_mount_all_display_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::get_displayed_mount_all_display_info(vm, c);
    }
    let m = displayed_at(vm, &c);
    all_display_info(vm, m)
}

fn get_mount_all_display_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::get_mount_all_display_info(vm, c);
    }
    let id = arg_num(vm, &c, 1).unwrap_or(0.0) as i64;
    let m = mount(vm, id);
    all_display_info(vm, m)
}

fn get_all_display_ids_for_mount(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::get_all_display_ids_for_mount(vm, c);
    }
    let id = arg_num(vm, &c, 1).unwrap_or(0.0) as i64;
    let ids: Vec<Value> = mount(vm, id)
        .map(|m| m.display)
        .filter(|d| !d.is_nil())
        .into_iter()
        .collect();
    Ok(vec![list(vm, ids)])
}

/// `GetIsFavorite(i)`: the displayed mount's favourite, and whether it can
/// be one: any displayed mount (recorded: an uncollected one's true, though
/// `SetIsFavorite` leaves it as it is).
fn get_is_favorite(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::mount_is_favorite(vm, c);
    }
    let m = displayed_at(vm, &c);
    Ok(vec![
        Value::Bool(m.as_ref().is_some_and(|m| m.favorite)),
        Value::Bool(m.is_some()),
    ])
}

/// `SetIsFavorite(i, isFavorite)`: the displayed mount's, a collected one's
/// (`MOUNT_JOURNAL_SEARCH_UPDATED`, the list sorted again).
fn set_is_favorite(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return Ok(vec![]);
    }
    let Some(m) = displayed_at(vm, &c).filter(|m| m.collected) else {
        return Ok(vec![]);
    };
    let on = vm.hidden(c.arg(2)).truthy();
    if let Some(all) = engine_mut(vm).world.collected.mounts.as_mut()
        && let Some(entry) = all.iter_mut().find(|e| e.id == m.id)
    {
        entry.favorite = on;
    }
    widgets::fire_event(vm, "MOUNT_JOURNAL_SEARCH_UPDATED", &[]);
    Ok(vec![])
}

fn get_mount_from_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::get_mount_from_spell(vm, c);
    }
    let spell = arg_num(vm, &c, 1).unwrap_or(0.0);
    let id = mounts(vm)
        .and_then(|all| all.iter().find(|m| m.spell == spell))
        .map_or(Value::Nil, |m| Value::Num(m.id as f64));
    Ok(vec![id])
}

fn get_mount_from_item(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::get_mount_from_item(vm, c);
    }
    let item = arg_num(vm, &c, 1).unwrap_or(0.0) as i64;
    let id = mounts(vm)
        .and_then(|all| all.iter().find(|m| m.item == Some(item)))
        .map_or(Value::Nil, |m| Value::Num(m.id as f64));
    Ok(vec![id])
}

/// `GetMountUsabilityByID(mountID, checkIndoors)`: a collected usable
/// mount's true; else false with the client's reason for one not collected
/// (recorded), none for one not usable here (unverified); nothing for an ID
/// not in the journal.
fn get_mount_usability(vm: &mut Vm, c: NativeCall) -> Ret {
    if mounts(vm).is_none() {
        return crate::collections::get_mount_usability(vm, c);
    }
    let id = arg_num(vm, &c, 1).unwrap_or(0.0) as i64;
    Ok(match mount(vm, id) {
        Some(m) if m.collected && m.usable => vec![Value::Bool(true), Value::Nil],
        Some(m) if !m.collected => {
            let why = vm.str("You have not collected this mount.");
            vec![Value::Bool(false), why]
        }
        Some(_) => vec![Value::Bool(false), Value::Nil],
        None => vec![],
    })
}

fn has_displayable_mount(vm: &mut Vm, c: NativeCall) -> Ret {
    match mounts(vm) {
        Some(all) => Ok(vec![Value::Bool(
            all.iter().any(|m| m.collected && !m.hidden),
        )]),
        None => recorded(vm, "C_MountJournal.HasDisplayableMount", &c),
    }
}

/// Whether a set mount casts the spell (`None`: no mounts set).
pub(crate) fn is_set_mount_spell(vm: &Vm, spell: f64) -> Option<bool> {
    Some(mounts(vm)?.iter().any(|m| m.spell == spell))
}

// ---------------------------------------------------------------------------
// Toys.

fn toys(vm: &Vm) -> Option<&Vec<Toy>> {
    engine(vm).world.collected.toys.as_ref()
}

fn toy(vm: &Vm, item: i64) -> Option<Toy> {
    toys(vm)?.iter().find(|t| t.item == item).cloned()
}

/// `mock.setToys({ [itemID] = { collected, favorite, usable, fanfare } })`:
/// the toy box's toys, the whole box (`TOYS_UPDATED`); a toy is collected
/// and usable unless it says not, its name and icon its item's.
pub fn control_set_toys(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setToys: a table of toys by item ID expected"));
    };
    let mut out = Vec::new();
    for (item, data) in by_id(vm, t) {
        let toy = match data {
            Value::Table(d) => Toy {
                item,
                collected: bool_at(vm, d, "collected", true),
                favorite: bool_at(vm, d, "favorite", false),
                usable: bool_at(vm, d, "usable", true),
                fanfare: bool_at(vm, d, "fanfare", false),
            },
            v if v.truthy() => Toy {
                item,
                collected: true,
                favorite: false,
                usable: true,
                fanfare: false,
            },
            _ => continue,
        };
        out.push(toy);
    }
    engine_mut(vm).world.collected.toys = Some(out);
    Ok(vec![])
}

/// A toy's item's name, when the client has the item loaded.
fn toy_name(vm: &mut Vm, item: i64) -> Value {
    match crate::gamedata::item_data(vm, item) {
        Some(t) if get(vm, t, "cached").truthy() => get(vm, t, "name"),
        _ => Value::Nil,
    }
}

/// The toy box's list: collected toys only (recorded: none of 21 listed,
/// the uncollected check on, for a character with none), through the
/// collected check (`by_collected`), the unusable check and the search.
fn filtered(vm: &mut Vm, by_collected: bool) -> Vec<Toy> {
    let Some(all) = toys(vm).cloned() else {
        return Vec::new();
    };
    let collected = crate::collections::toy_filter(vm, "toy.filter.collected", true);
    let unusable = crate::collections::toy_filter(vm, "toy.filter.unusable", false);
    let search = crate::collections::search(vm, "toy.search");
    let mut shown: Vec<(String, Toy)> = Vec::new();
    for t in all {
        if !t.collected || (by_collected && !collected) {
            continue;
        }
        if !unusable && !t.usable {
            continue;
        }
        let name = crate::character::item_field(vm, t.item, "name");
        let name = text_of(vm, name);
        if matches(&name, &search) {
            shown.push((name, t));
        }
    }
    shown.sort_by(|(an, a), (bn, b)| {
        (b.favorite && b.collected)
            .cmp(&(a.favorite && a.collected))
            .then(b.collected.cmp(&a.collected))
            .then(an.to_lowercase().cmp(&bn.to_lowercase()))
            .then(a.item.cmp(&b.item))
    });
    shown.into_iter().map(|(_, t)| t).collect()
}

fn get_num_toys(vm: &mut Vm, c: NativeCall) -> Ret {
    match toys(vm) {
        Some(all) => Ok(vec![Value::Num(all.len() as f64)]),
        None => recorded(vm, "C_ToyBox.GetNumToys", &c),
    }
}

fn get_num_filtered_toys(vm: &mut Vm, c: NativeCall) -> Ret {
    if toys(vm).is_none() {
        return recorded(vm, "C_ToyBox.GetNumFilteredToys", &c);
    }
    Ok(vec![Value::Num(filtered(vm, true).len() as f64)])
}

/// `GetToyFromIndex(i)`: the shown toy's item, -1 past them (camelot).
fn get_toy_from_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = arg_num(vm, &c, 1).unwrap_or(0.0) as i64;
    let item = if toys(vm).is_some() && i >= 1 {
        filtered(vm, true)
            .into_iter()
            .nth(i as usize - 1)
            .map(|t| t.item)
    } else {
        None
    };
    Ok(vec![Value::Num(item.map_or(-1.0, |i| i as f64))])
}

/// `GetNumLearnedDisplayedToys()`, `GetNumTotalDisplayedToys()`: the toys
/// the search and the other filters show, collected and all (unverified:
/// as the toy box's progress bar counts them).
fn get_num_learned_displayed(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = filtered(vm, false).iter().filter(|t| t.collected).count();
    Ok(vec![Value::Num(n as f64)])
}

fn get_num_total_displayed(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(filtered(vm, false).len() as f64)])
}

/// `GetToyInfo(itemID)`: itemID, toyName, icon, isFavorite, hasFanfare,
/// itemQuality; the name and quality while the item's data is loaded.
fn get_toy_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let item = arg_num(vm, &c, 1).map(|i| i as i64);
    let Some(t) = item.and_then(|i| toy(vm, i)) else {
        return crate::collections::get_toy_info(vm, c);
    };
    let name = toy_name(vm, t.item);
    let quality = if name.is_nil() {
        Value::Nil
    } else {
        crate::character::item_field(vm, t.item, "quality")
    };
    let icon = crate::character::item_field(vm, t.item, "icon");
    Ok(vec![
        Value::Num(t.item as f64),
        name,
        icon,
        Value::Bool(t.favorite),
        Value::Bool(t.fanfare),
        quality,
    ])
}

fn toy_arg(vm: &mut Vm, c: &NativeCall) -> Option<Toy> {
    let item = arg_num(vm, c, 1)? as i64;
    toy(vm, item)
}

fn player_has_toy(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        toy_arg(vm, &c).is_some_and(|t| t.collected),
    )])
}

fn is_toy_usable(vm: &mut Vm, c: NativeCall) -> Ret {
    // Not a toy of the account: nothing (the full probe: -1, 0, 1, 2).
    Ok(toy_arg(vm, &c)
        .map(|t| vec![Value::Bool(t.usable)])
        .unwrap_or_default())
}

fn get_toy_favorite(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        toy_arg(vm, &c).is_some_and(|t| t.favorite),
    )])
}

/// `SetIsFavorite(itemID, value)`: a collected toy's (`TOYS_UPDATED`).
fn set_toy_favorite(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = toy_arg(vm, &c).filter(|t| t.collected) else {
        return Ok(vec![]);
    };
    let on = vm.hidden(c.arg(2)).truthy();
    if let Some(all) = engine_mut(vm).world.collected.toys.as_mut()
        && let Some(entry) = all.iter_mut().find(|e| e.item == t.item)
    {
        entry.favorite = on;
    }
    widgets::fire_event(vm, "TOYS_UPDATED", &[Value::Num(t.item as f64)]);
    Ok(vec![])
}

fn toys_have_favorites(vm: &mut Vm, c: NativeCall) -> Ret {
    match toys(vm) {
        Some(all) => Ok(vec![Value::Bool(all.iter().any(|t| t.favorite))]),
        None => recorded(vm, "C_ToyBox.HasFavorites", &c),
    }
}

fn has_any_toy(vm: &mut Vm, c: NativeCall) -> Ret {
    match toys(vm) {
        Some(all) => Ok(vec![Value::Bool(all.iter().any(|t| t.collected))]),
        None => recorded(vm, "C_ToyBoxInfo.HasAnyToy", &c),
    }
}

/// `GetToyLink(itemID)`: a toy's item's link.
fn get_toy_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = toy_arg(vm, &c) else {
        // Nothing for a toy the account lacks (the full probe: -1).
        return Ok(vec![]);
    };
    Ok(vec![
        crate::character::link_of_id(vm, t.item).unwrap_or(Value::Nil),
    ])
}

fn toy_needs_fanfare(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        toy_arg(vm, &c).is_some_and(|t| t.fanfare),
    )])
}

fn toy_clear_fanfare(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(item) = arg_num(vm, &c, 1).map(|i| i as i64)
        && let Some(all) = engine_mut(vm).world.collected.toys.as_mut()
        && let Some(entry) = all.iter_mut().find(|e| e.item == item)
    {
        entry.fanfare = false;
    }
    Ok(vec![])
}

// ---------------------------------------------------------------------------
// The wardrobe.

/// `mock.setAppearances({ collected = { sourceID, … }, favorites = {
/// appearanceID, … } })`: the wardrobe's collected sources and favourite
/// appearances, over the recorded catalogue (`TRANSMOG_COLLECTION_UPDATED`);
/// a part left out stays as it was.
pub fn control_set_appearances(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setAppearances: a table expected ({ collected, favorites })"));
    };
    for (key, favorites) in [("collected", false), ("favorites", true)] {
        let Value::Table(ids) = get(vm, t, key) else {
            continue;
        };
        let set: BTreeSet<i64> = by_id(vm, ids)
            .into_iter()
            .filter_map(|(_, v)| match v {
                Value::Num(n) => Some(n as i64),
                _ => None,
            })
            .collect();
        if favorites {
            crate::collections::reset_appearance_favorites(vm);
        }
        let collected = &mut engine_mut(vm).world.collected;
        if favorites {
            collected.favorites = Some(set);
        } else {
            collected.sources = Some(set);
        }
    }
    Ok(vec![])
}

/// Whether the source is collected: as set, else `recorded`.
pub(crate) fn source_collected(vm: &Vm, source: Option<f64>, recorded: bool) -> bool {
    match &engine(vm).world.collected.sources {
        Some(set) => source.is_some_and(|s| set.contains(&(s as i64))),
        None => recorded,
    }
}

/// Whether the appearance is a favourite by default (before the game's
/// `SetIsAppearanceFavorite`): as set, else `recorded`.
pub(crate) fn favorite_default(vm: &Vm, visual: i64, recorded: bool) -> bool {
    match &engine(vm).world.collected.favorites {
        Some(set) => set.contains(&visual),
        None => recorded,
    }
}

/// The favourites a test set (`None`: as recorded).
pub(crate) fn set_favorites(vm: &Vm) -> Option<Vec<i64>> {
    Some(
        engine(vm)
            .world
            .collected
            .favorites
            .as_ref()?
            .iter()
            .copied()
            .collect(),
    )
}

/// Whether a test set the collected sources.
pub(crate) fn sources_set(vm: &Vm) -> bool {
    engine(vm).world.collected.sources.is_some()
}

/// Sets each listed source's `isCollected` as set, and its item's name and
/// quality (a list of `AppearanceSourceInfo`).
pub(crate) fn patch_sources(vm: &mut Vm, v: Value) {
    let Value::Table(t) = v else {
        return;
    };
    let set = sources_set(vm);
    for (_, entry) in by_id(vm, t) {
        let Value::Table(e) = entry else {
            continue;
        };
        if set {
            let id = num_at(vm, e, "sourceID");
            let collected = source_collected(vm, id, false);
            let k = vm.str("isCollected");
            let _ = vm.heap.table_mut(e).set(k, Value::Bool(collected));
        }
        fill_item(vm, e);
    }
}

/// A source's `name` and `quality`: its item's, while the client has the
/// item loaded (the documentation's optional fields; the recordings, made
/// before the items loaded, lack them).
pub(crate) fn fill_item(vm: &mut Vm, e: TableRef) {
    let Some(item) = num_at(vm, e, "itemID") else {
        return;
    };
    let Some(data) = crate::gamedata::item_data(vm, item as i64) else {
        return;
    };
    if !get(vm, data, "cached").truthy() {
        return;
    }
    for key in ["name", "quality"] {
        let v = get(vm, data, key);
        if !v.is_nil() && get(vm, e, key).is_nil() {
            let k = vm.str(key);
            let _ = vm.heap.table_mut(e).set(k, v);
        }
    }
}

/// Sets each listed appearance's `isCollected` (any of its sources
/// collected) and `isFavorite` as set (a list of
/// `TransmogCategoryAppearanceInfo`).
pub(crate) fn patch_appearances(vm: &mut Vm, v: Value) {
    let Value::Table(t) = v else {
        return;
    };
    let sources = sources_set(vm);
    let favorites = set_favorites(vm);
    if !sources && favorites.is_none() {
        return;
    }
    for (_, entry) in by_id(vm, t) {
        let Value::Table(e) = entry else {
            continue;
        };
        let Some(visual) = num_at(vm, e, "visualID") else {
            continue;
        };
        if sources {
            let collected = crate::collections::appearance_collected(vm, visual as i64);
            let k = vm.str("isCollected");
            let _ = vm.heap.table_mut(e).set(k, Value::Bool(collected));
        }
        if let Some(f) = &favorites {
            let on = f.contains(&(visual as i64));
            let k = vm.str("isFavorite");
            let _ = vm.heap.table_mut(e).set(k, Value::Bool(on));
        }
    }
}

/// `C_TransmogCollection.GetAppearanceSources(appearanceID, category,
/// location)`: as recorded for the call; asked without the category the
/// recordings hold, every recorded source of the appearance (the probes
/// asked with one; unverified: the client's order); each source collected
/// as set.
fn get_appearance_sources(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = "C_TransmogCollection.GetAppearanceSources";
    let values = match crate::recorded::lookup(vm, name, &c) {
        Some(v) => v,
        None => {
            let visual = arg_num(vm, &c, 1).unwrap_or(0.0) as i64;
            match crate::collections::appearance_source_list(vm, visual) {
                Some(list) => vec![list],
                None => crate::facts::recorded_answer(vm, name, &c),
            }
        }
    };
    if let Some(v) = values.first().copied() {
        patch_sources(vm, v);
    }
    Ok(values)
}

/// `GetCategoryCollectedCount(category)`: as recorded, else the category's
/// appearances with a collected source.
fn get_category_collected_count(vm: &mut Vm, c: NativeCall) -> Ret {
    if !sources_set(vm) {
        return recorded(vm, "C_TransmogCollection.GetCategoryCollectedCount", &c);
    }
    let category = arg_num(vm, &c, 1).unwrap_or(-1.0) as i64;
    let n = crate::collections::category_visuals(category)
        .into_iter()
        .filter(|v| crate::collections::appearance_collected(vm, *v))
        .count();
    Ok(vec![Value::Num(n as f64)])
}

// ---------------------------------------------------------------------------
// The journals' events (recorded: `/stdtest engine journals` and `journals2`,
// `tests/client/forever-camelot-journals.lua`).

/// The mount journal's checks as `mountJournalGeneralFilters` keeps them: 1
/// collected, 2 not collected, 4 unusable (recorded: 0, 1, 3, 5 and 7).
fn mount_checks(vm: &mut Vm) -> u32 {
    (1..=3)
        .filter(|i| crate::collections::mount_filter(vm, *i))
        .map(|i| 1 << (i - 1))
        .sum()
}

/// `C_MountJournal.SetCollectedFilterSetting(i, checked)`: a change tells
/// the list's (`MOUNT_JOURNAL_SEARCH_UPDATED`), then the checks' CVar
/// (`CVAR_UPDATE`), at once; set as it was, nothing (recorded).
fn set_mount_collected_filter(vm: &mut Vm, c: NativeCall) -> Ret {
    let before = mount_checks(vm);
    let out = crate::collections::set_mount_collected_filter(vm, c)?;
    let after = mount_checks(vm);
    if before != after {
        widgets::fire_event(vm, "MOUNT_JOURNAL_SEARCH_UPDATED", &[]);
        crate::framescript::set_cvar_text(
            vm,
            "mountJournalGeneralFilters",
            after.to_string(),
            true,
        )?;
    }
    Ok(out)
}

fn toy_checks(vm: &mut Vm) -> [bool; 3] {
    [
        crate::collections::toy_filter(vm, "toy.filter.collected", true),
        crate::collections::toy_filter(vm, "toy.filter.uncollected", true),
        crate::collections::toy_filter(vm, "toy.filter.unusable", false),
    ]
}

/// The toy box's list refiltered: `TOYS_UPDATED` with its three values nil
/// on the next frame, once for the changes of a frame (recorded: three
/// checks changed, one event). Its `toyBoxCollectedFilters` CVar (letters)
/// is not modelled: the recordings do not tell its encoding.
fn toy_filter_changed(vm: &mut Vm, before: [bool; 3]) {
    if toy_checks(vm) == before || engine(vm).world.collected.toys_pending {
        return;
    }
    engine_mut(vm).world.collected.toys_pending = true;
    let native = vm.native_with("toys", toys_refiltered, vec![]);
    crate::clock::after_engine(vm, 0.0, native);
}

fn toys_refiltered(vm: &mut Vm, _c: NativeCall) -> Ret {
    engine_mut(vm).world.collected.toys_pending = false;
    widgets::fire_event(vm, "TOYS_UPDATED", &[Value::Nil, Value::Nil, Value::Nil]);
    Ok(vec![])
}

macro_rules! toy_setters {
    ($($name:ident: $inner:ident;)*) => {
        $(
            fn $name(vm: &mut Vm, c: NativeCall) -> Ret {
                let before = toy_checks(vm);
                let out = crate::collections::$inner(vm, c)?;
                toy_filter_changed(vm, before);
                Ok(out)
            }
        )*
    };
}

toy_setters! {
    set_toy_collected_shown: set_toy_collected_shown;
    set_toy_uncollected_shown: set_toy_uncollected_shown;
    set_toy_unusable_shown: set_toy_unusable_shown;
}

/// An appearance's favourite changed (`SetIsAppearanceFavorite`):
/// `TRANSMOG_COLLECTION_UPDATED(category + 1, source, appearance,
/// "favorite")` at once, and when it is no longer one
/// `TRANSMOG_COLLECTION_ITEM_FAVORITE_UPDATE(appearance, false)`; then
/// `TRANSMOG_COLLECTION_UPDATED` with its four values nil 0.6 s later
/// (recorded in categories 2, 4, 11 and 14, the later one between 0.5 and
/// 0.8 s; unverified: an appearance not collected).
pub(crate) fn appearance_favorite_changed(vm: &mut Vm, visual: i64, on: bool) {
    let (source, category) = crate::collections::favorite_source(vm, visual);
    let kind = vm.str("favorite");
    let args = [
        category.map_or(Value::Nil, |c| Value::Num(c + 1.0)),
        source.map_or(Value::Nil, Value::Num),
        Value::Num(visual as f64),
        kind,
    ];
    widgets::fire_event(vm, "TRANSMOG_COLLECTION_UPDATED", &args);
    if !on {
        widgets::fire_event(
            vm,
            "TRANSMOG_COLLECTION_ITEM_FAVORITE_UPDATE",
            &[Value::Num(visual as f64), Value::Bool(false)],
        );
    }
    let native = vm.native_with("transmog", transmog_updated, vec![]);
    crate::clock::after_engine(vm, 0.6, native);
}

fn transmog_updated(vm: &mut Vm, _c: NativeCall) -> Ret {
    widgets::fire_event(vm, "TRANSMOG_COLLECTION_UPDATED", &[Value::Nil; 4]);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        (
            "C_MountJournal.SetCollectedFilterSetting",
            set_mount_collected_filter as NativeFn,
        ),
        ("C_ToyBox.SetCollectedShown", set_toy_collected_shown),
        ("C_ToyBox.SetUncollectedShown", set_toy_uncollected_shown),
        ("C_ToyBox.SetUnusableShown", set_toy_unusable_shown),
        ("C_MountJournal.GetMountIDs", get_mount_ids as NativeFn),
        ("C_MountJournal.GetNumMounts", get_num_mounts),
        ("C_MountJournal.GetMountInfoByID", get_mount_info_by_id),
        (
            "C_MountJournal.GetMountInfoExtraByID",
            get_mount_info_extra_by_id,
        ),
        (
            "C_MountJournal.GetNumDisplayedMounts",
            get_num_displayed_mounts,
        ),
        (
            "C_MountJournal.GetDisplayedMountInfo",
            get_displayed_mount_info,
        ),
        ("C_MountJournal.GetDisplayedMountID", get_displayed_mount_id),
        (
            "C_MountJournal.GetDisplayedMountInfoExtra",
            get_displayed_mount_info_extra,
        ),
        (
            "C_MountJournal.GetDisplayedMountAllCreatureDisplayInfo",
            get_displayed_mount_all_display_info,
        ),
        (
            "C_MountJournal.GetMountAllCreatureDisplayInfoByID",
            get_mount_all_display_info,
        ),
        (
            "C_MountJournal.GetAllCreatureDisplayIDsForMountID",
            get_all_display_ids_for_mount,
        ),
        ("C_MountJournal.GetIsFavorite", get_is_favorite),
        ("C_MountJournal.SetIsFavorite", set_is_favorite),
        ("C_MountJournal.GetMountFromSpell", get_mount_from_spell),
        ("C_MountJournal.GetMountFromItem", get_mount_from_item),
        ("C_MountJournal.GetMountUsabilityByID", get_mount_usability),
        ("C_MountJournal.HasDisplayableMount", has_displayable_mount),
        ("C_ToyBox.GetNumToys", get_num_toys),
        ("C_ToyBox.GetNumFilteredToys", get_num_filtered_toys),
        ("C_ToyBox.GetToyFromIndex", get_toy_from_index),
        (
            "C_ToyBox.GetNumLearnedDisplayedToys",
            get_num_learned_displayed,
        ),
        ("C_ToyBox.GetNumTotalDisplayedToys", get_num_total_displayed),
        ("C_ToyBox.GetToyInfo", get_toy_info),
        ("PlayerHasToy", player_has_toy),
        ("C_ToyBox.IsToyUsable", is_toy_usable),
        ("C_ToyBox.GetIsFavorite", get_toy_favorite),
        ("C_ToyBox.SetIsFavorite", set_toy_favorite),
        ("C_ToyBox.HasFavorites", toys_have_favorites),
        ("C_ToyBoxInfo.HasAnyToy", has_any_toy),
        ("C_ToyBox.GetToyLink", get_toy_link),
        ("C_ToyBoxInfo.NeedsFanfare", toy_needs_fanfare),
        ("C_ToyBoxInfo.ClearFanfare", toy_clear_fanfare),
        (
            "C_TransmogCollection.GetAppearanceSources",
            get_appearance_sources,
        ),
        (
            "C_TransmogCollection.GetCategoryCollectedCount",
            get_category_collected_count,
        ),
    ]
}
