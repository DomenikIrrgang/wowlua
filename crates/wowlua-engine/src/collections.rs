//! The account's collections and the systems around them: mounts, battle
//! pets and pet battles, toys, heirlooms, the wardrobe (appearances, sets,
//! custom sets, outfits), titles, statistics and the achievement gaps, the
//! barber shop, content tracking and player choices.
//!
//! What the client recorded is the source where it exists: the wardrobe's
//! appearances and sources (`/stdtest engine collections`:
//! `C_TransmogCollection.GetAppearanceSources`, `GetCategoryAppearances`,
//! `C_TransmogSets.GetAllSets`, `C_TransmogOutfitInfo.GetOutfitsInfo`), the
//! journal's mount list, the statistics' criteria (`/stdtest engine
//! achievements`), the title count. The account recorded has no battle
//! pets (`C_PetJournal.GetNumPets`: 0), no toys (`C_ToyBoxInfo.HasAnyToy`:
//! false), no heirlooms (`C_Heirloom.GetNumHeirlooms`: 0) and no mount the
//! journal displays (`GetNumDisplayedMounts`: 0), and is in no pet battle
//! (`C_PetBattles.IsInBattle`: false): those answer the "nothing collected"
//! state. The journals' filters and searches are kept per VM (in the
//! registry) so that a setter's value reads back; their defaults are
//! unverified unless said.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, units};

// ---------------------------------------------------------------------------
// Helpers.

fn num(vm: &Vm, c: &NativeCall, n: usize) -> Option<f64> {
    match vm.hidden(c.arg(n)) {
        Value::Num(x) => Some(x),
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).trim().parse().ok(),
        _ => None,
    }
}

fn text(vm: &Vm, c: &NativeCall, n: usize) -> String {
    match vm.hidden(c.arg(n)) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(x) if x.fract() == 0.0 => format!("{}", x as i64),
        Value::Num(x) => format!("{x}"),
        _ => String::new(),
    }
}

fn truthy(vm: &Vm, c: &NativeCall, n: usize) -> bool {
    vm.hidden(c.arg(n)).truthy()
}

fn first(values: Option<Vec<Value>>) -> Value {
    values
        .and_then(|v| v.first().copied())
        .unwrap_or(Value::Nil)
}

fn num_of(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn str_of(vm: &Vm, v: Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn list(vm: &mut Vm, values: impl IntoIterator<Item = Value>) -> Value {
    let t = vm.new_table();
    for (i, v) in values.into_iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num((i + 1) as f64), v);
    }
    Value::Table(t)
}

fn set_field(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

/// The recorded answer to this call; else noted as unanswered (what a
/// probe asks the client next).
fn recorded_or_noted(vm: &mut Vm, name: &str, c: &NativeCall) -> Option<Vec<Value>> {
    if let Some(v) = crate::recorded::lookup(vm, name, c) {
        return Some(v);
    }
    crate::note_default(vm, name, c, 1);
    None
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
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

fn zero_zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0)])
}

fn nil(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn empty_table(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn empty_tables(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (a, b) = (vm.new_table(), vm.new_table());
    Ok(vec![Value::Table(a), Value::Table(b)])
}

/// A name the client approves: any not blank (unverified: its profanity
/// and length checks).
fn valid_name(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(!text(vm, &c, 1).trim().is_empty())])
}

// ---------------------------------------------------------------------------
// The journals' settings, kept per VM in the registry: a value is stored
// only while it differs from its default, so a system uses its default
// filters while none of its keys is stored.

fn state(vm: &mut Vm) -> TableRef {
    let registry = vm.registry;
    let key = vm.heap.intern(b"wowlua.engine.collections");
    match vm.heap.table(registry).get_str(key) {
        Value::Table(t) => t,
        _ => {
            let t = vm.new_table();
            vm.heap.table_mut(registry).set_str(key, Value::Table(t));
            t
        }
    }
}

fn flag(vm: &mut Vm, key: &str, default: bool) -> bool {
    let t = state(vm);
    let k = vm.str(key);
    match vm.heap.table(t).get(k) {
        Value::Bool(b) => b,
        _ => default,
    }
}

fn set_flag(vm: &mut Vm, key: &str, on: bool, default: bool) {
    let t = state(vm);
    let v = if on == default {
        Value::Nil
    } else {
        Value::Bool(on)
    };
    set_field(vm, t, key, v);
}

fn number(vm: &mut Vm, key: &str, default: f64) -> f64 {
    let t = state(vm);
    let k = vm.str(key);
    num_of(vm.heap.table(t).get(k)).unwrap_or(default)
}

fn set_number(vm: &mut Vm, key: &str, n: f64, default: f64) {
    let t = state(vm);
    let v = if n == default {
        Value::Nil
    } else {
        Value::Num(n)
    };
    set_field(vm, t, key, v);
}

fn string(vm: &mut Vm, key: &str) -> String {
    let t = state(vm);
    let k = vm.str(key);
    let v = vm.heap.table(t).get(k);
    str_of(vm, v).unwrap_or_default()
}

fn set_string(vm: &mut Vm, key: &str, s: &str) {
    let t = state(vm);
    let v = if s.is_empty() { Value::Nil } else { vm.str(s) };
    set_field(vm, t, key, v);
}

/// The stored keys that start with `prefix`.
fn stored(vm: &mut Vm, prefix: &str) -> Vec<String> {
    let t = state(vm);
    let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
    entries
        .into_iter()
        .filter(|(_, v)| !v.is_nil())
        .filter_map(|(k, _)| str_of(vm, k))
        .filter(|k| k.starts_with(prefix))
        .collect()
}

fn reset(vm: &mut Vm, prefix: &str) {
    let t = state(vm);
    for key in stored(vm, prefix) {
        set_field(vm, t, &key, Value::Nil);
    }
}

/// Default-filter checks and resets: the system's filter keys (its search
/// and favorites aside) none stored.
fn defaults_in_use(vm: &mut Vm, prefix: &str) -> bool {
    stored(vm, prefix).is_empty()
}

macro_rules! filters {
    ($($is_default:ident, $set_default:ident: $prefix:literal;)*) => {
        $(
            fn $is_default(vm: &mut Vm, _c: NativeCall) -> Ret {
                Ok(vec![Value::Bool(defaults_in_use(vm, $prefix))])
            }
            fn $set_default(vm: &mut Vm, _c: NativeCall) -> Ret {
                reset(vm, $prefix);
                Ok(vec![])
            }
        )*
    };
}

filters! {
    mount_default_filters, set_mount_default_filters: "mount.filter.";
    pet_default_filters, set_pet_default_filters: "pet.filter.";
    toy_default_filters, set_toy_default_filters: "toy.filter.";
    heirloom_default_filters, set_heirloom_default_filters: "heirloom.filter.";
    transmog_default_filters, set_transmog_default_filters: "transmog.filter.";
    sets_default_filters, set_sets_default_filters: "sets.filter.";
    base_sets_default_filters, set_base_sets_default_filters: "basesets.filter.";
}

/// A boolean filter's getter and setter, by an index argument (`$indexed`)
/// or none, with its default.
macro_rules! flag_filters {
    ($($get:ident, $set:ident: $key:literal, $default:literal, $indexed:literal;)*) => {
        $(
            fn $get(vm: &mut Vm, c: NativeCall) -> Ret {
                let key = if $indexed {
                    format!("{}{}", $key, num(vm, &c, 1).unwrap_or(0.0) as i64)
                } else {
                    $key.to_string()
                };
                Ok(vec![Value::Bool(flag(vm, &key, $default))])
            }
            fn $set(vm: &mut Vm, c: NativeCall) -> Ret {
                let (key, on) = if $indexed {
                    (
                        format!("{}{}", $key, num(vm, &c, 1).unwrap_or(0.0) as i64),
                        truthy(vm, &c, 2),
                    )
                } else {
                    ($key.to_string(), truthy(vm, &c, 1))
                };
                set_flag(vm, &key, on, $default);
                Ok(vec![])
            }
        )*
    };
}

flag_filters! {
    // The mount journal's collected / not collected / unusable checks, its
    // sources and types: all checked.
    mount_collected_filter, set_mount_collected_filter: "mount.filter.collected.", true, true;
    mount_source_checked, set_mount_source_filter: "mount.filter.source.", true, true;
    mount_type_checked, set_mount_type_filter: "mount.filter.type.", true, true;
    // The pet journal's collected / not collected, sources and types.
    pet_filter_checked, set_pet_filter_checked: "pet.filter.flag.", true, true;
    pet_source_checked, set_pet_source_checked: "pet.filter.source.", true, true;
    pet_type_checked, set_pet_type_checked: "pet.filter.type.", true, true;
    // The toy box's: collected and not collected shown, unusable hidden.
    toy_collected_shown, set_toy_collected_shown: "toy.filter.collected", true, false;
    toy_uncollected_shown, set_toy_uncollected_shown: "toy.filter.uncollected", true, false;
    toy_unusable_shown, set_toy_unusable_shown: "toy.filter.unusable", false, false;
    toy_source_checked, set_toy_source_filter: "toy.filter.source.", true, true;
    toy_expansion_checked, set_toy_expansion_filter: "toy.filter.expansion.", true, true;
    // The heirlooms': collected and not collected shown (recorded true),
    // every source.
    heirloom_collected_shown, set_heirloom_collected_shown: "heirloom.filter.collected", true, false;
    heirloom_uncollected_shown, set_heirloom_uncollected_shown: "heirloom.filter.uncollected", true, false;
    heirloom_source_filter, set_heirloom_source_filter: "heirloom.filter.source.", true, true;
    // The wardrobe's: collected and not collected shown, every source type,
    // other factions' and races' appearances hidden.
    transmog_collected_shown, set_transmog_collected_shown: "transmog.filter.collected", true, false;
    transmog_uncollected_shown, set_transmog_uncollected_shown: "transmog.filter.uncollected", true, false;
    transmog_source_checked, set_transmog_source_filter: "transmog.filter.source.", true, true;
    transmog_factions_shown, set_transmog_factions_shown: "transmog.filter.factions", false, false;
    transmog_races_shown, set_transmog_races_shown: "transmog.filter.races", false, false;
    // The sets' filters (collected, not collected, PvE, PvP).
    sets_filter, set_sets_filter: "sets.filter.", true, true;
    base_sets_filter, set_base_sets_filter: "basesets.filter.", true, true;
}

/// The journal's `SetAll…(checked)`: each of `count` indexed filters.
fn set_all(vm: &mut Vm, c: &NativeCall, key: &str, count: i64) {
    let on = truthy(vm, c, 1);
    for i in 1..=count {
        set_flag(vm, &format!("{key}{i}"), on, true);
    }
}

fn all_checked(vm: &mut Vm, key: &str, count: i64) -> bool {
    (1..=count).all(|i| flag(vm, &format!("{key}{i}"), true))
}

/// Mount sources and types (`Enum.MountJournalSourceType`-sized, unverified
/// counts: 13 sources, 6 types).
const MOUNT_SOURCES: i64 = 13;
const MOUNT_TYPES: i64 = 6;
/// The pet journal's sources (recorded `C_PetJournal.GetNumPetSources`: 12)
/// and types (`C_PetJournal.GetNumPetTypes`).
const PET_SOURCES: i64 = 12;
const PET_TYPES: i64 = 10;
/// The toy box's sources and expansions (unverified counts).
const TOY_SOURCES: i64 = 12;
const TOY_EXPANSIONS: i64 = 12;
/// The wardrobe's source types (recorded
/// `C_TransmogCollection.GetNumTransmogSources`: 6).
const TRANSMOG_SOURCES: i64 = 6;

fn set_all_mount_sources(vm: &mut Vm, c: NativeCall) -> Ret {
    set_all(vm, &c, "mount.filter.source.", MOUNT_SOURCES);
    Ok(vec![])
}

fn set_all_mount_types(vm: &mut Vm, c: NativeCall) -> Ret {
    set_all(vm, &c, "mount.filter.type.", MOUNT_TYPES);
    Ok(vec![])
}

fn set_all_pet_sources(vm: &mut Vm, c: NativeCall) -> Ret {
    set_all(vm, &c, "pet.filter.source.", PET_SOURCES);
    Ok(vec![])
}

fn set_all_pet_types(vm: &mut Vm, c: NativeCall) -> Ret {
    set_all(vm, &c, "pet.filter.type.", PET_TYPES);
    Ok(vec![])
}

fn set_all_toy_sources(vm: &mut Vm, c: NativeCall) -> Ret {
    set_all(vm, &c, "toy.filter.source.", TOY_SOURCES);
    Ok(vec![])
}

fn set_all_toy_expansions(vm: &mut Vm, c: NativeCall) -> Ret {
    set_all(vm, &c, "toy.filter.expansion.", TOY_EXPANSIONS);
    Ok(vec![])
}

fn set_all_transmog_sources(vm: &mut Vm, c: NativeCall) -> Ret {
    set_all(vm, &c, "transmog.filter.source.", TRANSMOG_SOURCES);
    Ok(vec![])
}

fn all_transmog_sources_checked(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(all_checked(
        vm,
        "transmog.filter.source.",
        TRANSMOG_SOURCES,
    ))])
}

/// `C_TransmogCollection.SetAllCollectionTypeFilters(checked)` and
/// `AreAllCollectionTypeFiltersChecked()`: one switch for every category.
fn set_all_collection_types(vm: &mut Vm, c: NativeCall) -> Ret {
    let on = truthy(vm, &c, 1);
    set_flag(vm, "transmog.filter.categories", on, true);
    Ok(vec![])
}

fn all_collection_types_checked(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(flag(
        vm,
        "transmog.filter.categories",
        true,
    ))])
}

/// A search text kept for its getter (`C_PetJournal.SetSearchFilter`,
/// `GetSearchFilter`, `ClearSearchFilter`; the toy box's, the mount
/// journal's and the heirlooms' searches).
fn set_pet_search(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = text(vm, &c, 1);
    set_string(vm, "pet.search", &s);
    Ok(vec![])
}

fn clear_pet_search(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_string(vm, "pet.search", "");
    Ok(vec![])
}

fn get_pet_search(vm: &mut Vm, _c: NativeCall) -> Ret {
    let s = string(vm, "pet.search");
    Ok(vec![vm.str(&s)])
}

fn set_toy_search(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = text(vm, &c, 1);
    set_string(vm, "toy.search", &s);
    Ok(vec![])
}

fn set_mount_search(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = text(vm, &c, 1);
    set_string(vm, "mount.search", &s);
    Ok(vec![])
}

fn set_heirloom_search(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = text(vm, &c, 1);
    set_string(vm, "heirloom.search", &s);
    Ok(vec![])
}

/// `C_PetJournal.SetPetSortParameter(sort)`, `GetPetSortParameter()`: by
/// name (`LE_SORT_BY_NAME`, 1) until set.
fn set_pet_sort(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = num(vm, &c, 1).unwrap_or(1.0);
    set_number(vm, "pet.sort", n, 1.0);
    Ok(vec![])
}

fn get_pet_sort(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(number(vm, "pet.sort", 1.0))])
}

/// `C_Heirloom.SetClassAndSpecFilters(classID, specID)` and its getter
/// (recorded `GetClassAndSpecFilters()`: 0, 0).
fn set_heirloom_class_spec(vm: &mut Vm, c: NativeCall) -> Ret {
    let class = num(vm, &c, 1).unwrap_or(0.0);
    let spec = num(vm, &c, 2).unwrap_or(0.0);
    set_number(vm, "heirloom.filter.class", class, 0.0);
    set_number(vm, "heirloom.filter.spec", spec, 0.0);
    Ok(vec![])
}

fn get_heirloom_class_spec(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Num(number(vm, "heirloom.filter.class", 0.0)),
        Value::Num(number(vm, "heirloom.filter.spec", 0.0)),
    ])
}

// ---------------------------------------------------------------------------
// Mounts.

/// `C_MountJournal.GetMountIDs()` as recorded (`/stdtest engine
/// collections`: 139, `GetNumMounts` agrees).
const MOUNT_IDS: [u32; 139] = [
    3126, 83, 8, 62, 39, 2864, 106, 3169, 107, 41, 2923, 51, 119, 12, 21, 108, 2920, 3120, 70,
    2957, 71, 94, 2918, 3111, 20, 2952, 69, 79, 91, 17, 3067, 35, 66, 3085, 3128, 103, 311, 78,
    3168, 36, 22, 99, 2996, 72, 11, 31, 80, 6, 53, 105, 38, 3118, 120, 85, 40, 92, 3038, 7, 2998,
    109, 3037, 56, 19, 3115, 57, 96, 3121, 93, 76, 2955, 88, 74, 111, 3122, 117, 89, 14, 50, 63,
    95, 125, 18, 13, 54, 43, 24, 97, 84, 90, 3123, 27, 2919, 3124, 118, 102, 3116, 32, 68, 81, 98,
    2997, 3089, 73, 110, 104, 42, 3090, 2953, 52, 82, 26, 3001, 15, 46, 2958, 3117, 9, 34, 45, 101,
    28, 55, 2954, 3129, 87, 65, 64, 25, 100, 67, 116, 58, 3130, 75, 121, 3127, 3167, 2995, 77,
];

/// Only the mounts whose info was recorded: addons read each listed
/// mount's info and index by its spell (ElvUI's `E.MountIDs[spellID]`), so
/// a mount listed without info breaks them (none recorded yet: empty, as
/// Blizzard's code read before).
fn get_mount_ids(vm: &mut Vm, _c: NativeCall) -> Ret {
    let answers = crate::recorded::answers();
    let ids: Vec<Value> = MOUNT_IDS
        .iter()
        .filter(|id| {
            answers.contains_key(format!("C_MountJournal.GetMountInfoByID({id})").as_str())
        })
        .map(|id| Value::Num(f64::from(*id)))
        .collect();
    Ok(vec![list(vm, ids)])
}

/// `C_MountJournal.GetMountInfoByID(mountID)` and `GetMountInfoExtraByID`:
/// as the client answered, else nothing, noted (the mounts' data is not
/// recorded; none is displayed, so none is collected here).
fn get_mount_info_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(recorded_or_noted(vm, "C_MountJournal.GetMountInfoByID", &c).unwrap_or_default())
}

fn get_mount_info_extra_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(recorded_or_noted(vm, "C_MountJournal.GetMountInfoExtraByID", &c).unwrap_or_default())
}

/// `C_MountJournal.GetMountFromSpell(spellID)`, `GetMountFromItem(itemID)`:
/// as recorded, else none, noted (unverified: the mounts' spells and items
/// are not recorded).
fn get_mount_from_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(
        recorded_or_noted(vm, "C_MountJournal.GetMountFromSpell", &c)
            .unwrap_or_else(|| vec![Value::Nil]),
    )
}

fn get_mount_from_item(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(recorded_or_noted(vm, "C_MountJournal.GetMountFromItem", &c)
        .unwrap_or_else(|| vec![Value::Nil]))
}

/// `C_MountJournal.GetMountUsabilityByID(mountID, checkIndoors)`: not
/// usable, none being collected (unverified), nothing for an ID not in the
/// journal.
fn get_mount_usability(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(vm, &c, 1).unwrap_or(0.0);
    if MOUNT_IDS.iter().any(|m| f64::from(*m) == id) {
        return Ok(vec![Value::Bool(false), Value::Nil]);
    }
    Ok(vec![])
}

/// `C_MountJournal.IsValidSourceFilter(i)`, `IsValidTypeFilter(i)`: the
/// filters the dropdown lists (unverified counts).
fn is_valid_mount_source(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = num(vm, &c, 1).unwrap_or(0.0) as i64;
    Ok(vec![Value::Bool((1..=MOUNT_SOURCES).contains(&i))])
}

fn is_valid_mount_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = num(vm, &c, 1).unwrap_or(0.0) as i64;
    Ok(vec![Value::Bool((1..=MOUNT_TYPES).contains(&i))])
}

/// `C_MountJournal.GetIsFavorite(index)`: not a favorite, and none can be
/// set (no mount displayed, recorded `GetNumDisplayedMounts`: 0).
fn mount_is_favorite(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false), Value::Bool(false)])
}

// ---------------------------------------------------------------------------
// Titles.

/// The client's titles by ID (`data/camelot-titles.txt`, recorded: the
/// PvP ranks' and the others' names, a trailing space for a title before
/// the name; an empty name where the client has none).
fn title_names() -> &'static [&'static str] {
    static NAMES: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        include_str!("../data/camelot-titles.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .map(|(_, name)| name)
            .collect()
    })
}

/// `GetNumTitles()`: 111 (recorded), the titles listed.
fn get_num_titles(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(title_names().len() as f64)])
}

/// `GetTitleName(titleMaskID)`: as recorded, else the title's name and
/// true for a title (warcraft.wiki.gg's `playerTitle`; unverified), an
/// empty name and false for an ID without one (the name recorded empty;
/// unverified: the second value); past the list nothing, noted.
fn get_title_name(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(v) = crate::recorded::lookup(vm, "GetTitleName", &c) {
        return Ok(v);
    }
    let id = num(vm, &c, 1).unwrap_or(0.0) as i64;
    let names = title_names();
    if (1..=names.len() as i64).contains(&id) {
        let name = names[(id - 1) as usize];
        return Ok(vec![vm.str(name), Value::Bool(!name.is_empty())]);
    }
    crate::note_default(vm, "GetTitleName", &c, 1);
    Ok(vec![])
}

/// `IsTitleKnown(id)`: none known (unverified: the recorded character's
/// titles were not asked).
fn is_title_known(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `GetCurrentTitle()`: -1, no title (warcraft.wiki.gg; Blizzard's paper
/// doll checks `> 0`); `SetCurrentTitle(id)` changes nothing while no
/// title is known.
fn get_current_title(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(-1.0)])
}

// ---------------------------------------------------------------------------
// Toys.

/// `C_ToyBox.GetToyInfo(itemID)`: a missing item ID errors as recorded
/// ("Usage: GetToyInfo(itemID)"); else as recorded, or nothing, noted
/// (the toys' data is not recorded; the account has none).
fn get_toy_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if num(vm, &c, 1).is_none() {
        return Err(vm.error("Usage: GetToyInfo(itemID)"));
    }
    Ok(recorded_or_noted(vm, "C_ToyBox.GetToyInfo", &c).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Pets and pet battles.

/// `C_PetJournal.GetPetLoadOutInfo(slot)`: no pet, no abilities, locked
/// (unverified: no battle pets).
fn get_pet_loadout_info(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Nil,
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Bool(true),
    ])
}

/// `C_PetJournal.GetPetSummonInfo(guid)`: not summonable, no error
/// (`Enum.PetJournalError.None`), no text (unverified: no such pet).
fn get_pet_summon_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false), Value::Num(0.0), vm.str("")])
}

/// `C_PetJournal.GetPetCooldownByGUID(guid)`: none, enabled (as the
/// recorded `GetSummonBattlePetCooldown()`: 0, 0, 1).
fn get_pet_cooldown(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0), Value::Num(1.0)])
}

/// `C_PetJournal.GetNumPetTypes()`: the ten pet types (humanoid to
/// mechanical; unverified).
fn get_num_pet_types(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(PET_TYPES as f64)])
}

/// `C_PetBattles.GetAttackModifier(petType, enemyPetType)`: the pet types'
/// chart, 1.5 strong and 2/3 weak (warcraft.wiki.gg's chart; unverified on
/// Forever).
fn get_attack_modifier(vm: &mut Vm, c: NativeCall) -> Ret {
    // Strong against, weak against, by attacking type (1 humanoid, 2
    // dragonkin, 3 flying, 4 undead, 5 critter, 6 magic, 7 elemental, 8
    // beast, 9 aquatic, 10 mechanical).
    const CHART: [(i64, i64); 10] = [
        (2, 8),
        (6, 4),
        (9, 2),
        (1, 9),
        (4, 1),
        (3, 10),
        (10, 5),
        (5, 3),
        (7, 6),
        (8, 7),
    ];
    let (Some(own), Some(enemy)) = (num(vm, &c, 1), num(vm, &c, 2)) else {
        return Ok(vec![Value::Num(1.0)]);
    };
    let (own, enemy) = (own as i64, enemy as i64);
    let modifier = match CHART.get((own - 1).max(0) as usize) {
        Some((strong, _)) if (1..=10).contains(&own) && *strong == enemy => 1.5,
        Some((_, weak)) if (1..=10).contains(&own) && *weak == enemy => 2.0 / 3.0,
        _ => 1.0,
    };
    Ok(vec![Value::Num(modifier)])
}

// ---------------------------------------------------------------------------
// Heirlooms: none (recorded `C_HeirloomInfo.HeirloomsAvailable`: false).

// ---------------------------------------------------------------------------
// The wardrobe: the recorded sources and appearances.

/// A recorded table's entries (`{k=v,…}`): each key and its value's text.
fn fields(entry: &str) -> Vec<(&str, &str)> {
    let inner = entry
        .trim()
        .strip_prefix('{')
        .and_then(|e| e.strip_suffix('}'))
        .unwrap_or("");
    crate::recorded::split_top(inner, ',')
        .into_iter()
        .filter_map(|kv| kv.split_once('='))
        .collect()
}

fn field<'a>(entry: &'a str, name: &str) -> Option<&'a str> {
    fields(entry)
        .into_iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn field_num(entry: &str, name: &str) -> Option<f64> {
    field(entry, name)?.strip_prefix("number ")?.parse().ok()
}

fn field_bool(entry: &str, name: &str) -> bool {
    field(entry, name) == Some("boolean true")
}

fn field_str<'a>(entry: &'a str, name: &str) -> Option<&'a str> {
    field(entry, name)?.strip_prefix("string ")
}

/// A recorded one-value answer's list (`1: {1={…},2={…}}`): its entries in
/// order.
fn list_entries(answer: &'static str) -> Vec<&'static str> {
    let Some(rest) = answer.strip_prefix("1: ") else {
        return Vec::new();
    };
    let mut out: Vec<(f64, &'static str)> = fields(rest)
        .into_iter()
        .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, v)| v).collect()
}

/// A recorded entry as a table.
fn entry_value(vm: &mut Vm, entry: &str) -> Value {
    crate::recorded::parse_answer(vm, &format!("1: {entry}"))
        .into_iter()
        .next()
        .unwrap_or(Value::Nil)
}

/// The recorded answers to the calls starting with `prefix`, in the calls'
/// order.
fn recorded_calls(prefix: &str) -> Vec<&'static str> {
    let mut calls: Vec<(&'static str, &'static str)> = crate::recorded::answers()
        .iter()
        .filter(|(k, _)| k.starts_with(prefix))
        .map(|(k, v)| (*k, *v))
        .collect();
    calls.sort();
    calls.into_iter().map(|(_, v)| v).collect()
}

/// The wardrobe's sources by source ID (each recorded
/// `C_TransmogCollection.GetAppearanceSources`' `AppearanceSourceInfo`).
fn sources() -> &'static BTreeMap<i64, &'static str> {
    static SOURCES: OnceLock<BTreeMap<i64, &'static str>> = OnceLock::new();
    SOURCES.get_or_init(|| {
        let mut out = BTreeMap::new();
        for answer in recorded_calls("C_TransmogCollection.GetAppearanceSources(") {
            for entry in list_entries(answer) {
                if let Some(id) = field_num(entry, "sourceID") {
                    out.entry(id as i64).or_insert(entry);
                }
            }
        }
        out
    })
}

/// The appearances' sources, by appearance (visual) ID.
fn appearance_sources() -> &'static BTreeMap<i64, Vec<i64>> {
    static BY_VISUAL: OnceLock<BTreeMap<i64, Vec<i64>>> = OnceLock::new();
    BY_VISUAL.get_or_init(|| {
        let mut out: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        for (id, entry) in sources() {
            if let Some(visual) = field_num(entry, "visualID") {
                out.entry(visual as i64).or_default().push(*id);
            }
        }
        out
    })
}

/// The appearances (`TransmogCategoryAppearanceInfo`, recorded by
/// category), by visual ID.
fn appearances() -> &'static BTreeMap<i64, &'static str> {
    static APPEARANCES: OnceLock<BTreeMap<i64, &'static str>> = OnceLock::new();
    APPEARANCES.get_or_init(|| {
        let mut out = BTreeMap::new();
        for answer in recorded_calls("C_TransmogCollection.GetCategoryAppearances(") {
            for entry in list_entries(answer) {
                if let Some(id) = field_num(entry, "visualID") {
                    out.entry(id as i64).or_insert(entry);
                }
            }
        }
        out
    })
}

fn source(vm: &Vm, c: &NativeCall, n: usize) -> Option<&'static str> {
    let id = num(vm, c, n)? as i64;
    sources().get(&id).copied()
}

/// `C_TransmogCollection.GetSourceInfo(sourceID)`: the source as recorded
/// in its appearance's sources; nothing for one not recorded (unverified:
/// the client adds the item's name and quality).
fn get_source_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match source(vm, &c, 1) {
        Some(entry) => vec![entry_value(vm, entry)],
        None => vec![],
    })
}

/// `C_TransmogCollection.GetSourceItemID(source)`,
/// `C_Transmog.GetItemIDForSource(source)`: its item.
fn get_source_item_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(
        match source(vm, &c, 1).and_then(|e| field_num(e, "itemID")) {
            Some(item) => vec![Value::Num(item)],
            None => vec![],
        },
    )
}

fn get_item_id_for_source(vm: &mut Vm, c: NativeCall) -> Ret {
    let item = source(vm, &c, 1).and_then(|e| field_num(e, "itemID"));
    Ok(vec![item.map_or(Value::Nil, Value::Num)])
}

/// `C_TransmogCollection.GetCategoryForItem(source)`,
/// `C_TransmogOutfitInfo.GetItemModifiedAppearanceEffectiveCategory`: its
/// category (the latter 0 for one not recorded, unverified).
fn get_category_for_item(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(
        match source(vm, &c, 1).and_then(|e| field_num(e, "categoryID")) {
            Some(cat) => vec![Value::Num(cat)],
            None => vec![],
        },
    )
}

fn effective_category(vm: &mut Vm, c: NativeCall) -> Ret {
    let cat = source(vm, &c, 1).and_then(|e| field_num(e, "categoryID"));
    Ok(vec![Value::Num(cat.unwrap_or(0.0))])
}

/// `PlayerHasTransmogItemModifiedAppearance(source)`,
/// `PlayerKnowsSource(source)`: collected, as recorded.
fn source_collected(vm: &mut Vm, c: NativeCall) -> Ret {
    let collected = source(vm, &c, 1).is_some_and(|e| field_bool(e, "isCollected"));
    Ok(vec![Value::Bool(collected)])
}

/// `PlayerCanCollectSource(source)`, `AccountCanCollectSource(source)`: the
/// item's data known for a recorded source, and whether the player can
/// collect it (recorded `playerCanCollect`; unverified for the account).
fn can_collect_source(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match source(vm, &c, 1) {
        Some(e) => vec![
            Value::Bool(true),
            Value::Bool(field_bool(e, "playerCanCollect")),
        ],
        None => vec![Value::Bool(false), Value::Bool(false)],
    })
}

/// `C_TransmogCollection.GetAllAppearanceSources(appearanceID)`: its
/// recorded sources.
fn get_all_appearance_sources(vm: &mut Vm, c: NativeCall) -> Ret {
    let visual = num(vm, &c, 1).unwrap_or(0.0) as i64;
    let ids: Vec<Value> = appearance_sources()
        .get(&visual)
        .map(|s| s.iter().map(|id| Value::Num(*id as f64)).collect())
        .unwrap_or_default();
    Ok(vec![list(vm, ids)])
}

/// `C_TransmogCollection.GetAppearanceInfoBySource(source)`: its
/// appearance's collection state from the recorded source and appearance
/// (unverified: the conditional and non-level requirement fields).
fn get_appearance_info_by_source(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(entry) = source(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let visual = field_num(entry, "visualID").unwrap_or(0.0);
    let siblings: Vec<&'static str> = appearance_sources()
        .get(&(visual as i64))
        .map(|s| {
            s.iter()
                .filter_map(|id| sources().get(id).copied())
                .collect()
        })
        .unwrap_or_default();
    let appearance = appearances().get(&(visual as i64)).copied();
    let source_collected = field_bool(entry, "isCollected");
    let appearance_collected = appearance
        .map_or(siblings.iter().any(|e| field_bool(e, "isCollected")), |a| {
            field_bool(a, "isCollected")
        });
    let t = vm.new_table();
    for (k, v) in [
        ("appearanceID", Value::Num(visual)),
        ("appearanceIsCollected", Value::Bool(appearance_collected)),
        ("sourceIsCollected", Value::Bool(source_collected)),
        ("sourceIsCollectedPermanent", Value::Bool(source_collected)),
        ("sourceIsCollectedConditional", Value::Bool(false)),
        ("meetsTransmogPlayerCondition", Value::Bool(true)),
        ("appearanceHasAnyNonLevelRequirements", Value::Bool(false)),
        ("appearanceMeetsNonLevelRequirements", Value::Bool(true)),
        (
            "appearanceIsUsable",
            Value::Bool(appearance.is_some_and(|a| field_bool(a, "isUsable"))),
        ),
        ("appearanceNumSources", Value::Num(siblings.len() as f64)),
        ("sourceIsKnown", Value::Bool(source_collected)),
        (
            "canDisplayOnPlayer",
            Value::Bool(field_bool(entry, "canDisplayOnPlayer")),
        ),
        (
            "isAnySourceValidForPlayer",
            Value::Bool(
                siblings
                    .iter()
                    .any(|e| field_bool(e, "isValidSourceForPlayer")),
            ),
        ),
    ] {
        set_field(vm, t, k, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `C_TransmogCollection.GetAppearanceCameraIDBySource(source)`: its
/// appearance's recorded camera, else 0 (unverified).
fn get_camera_by_source(vm: &mut Vm, c: NativeCall) -> Ret {
    let visual = source(vm, &c, 1).and_then(|e| field_num(e, "visualID"));
    let camera = visual.and_then(|v| {
        num_of(first(crate::recorded::lookup_text(
            vm,
            &format!("C_TransmogCollection.GetAppearanceCameraID({})", v as i64),
        )))
    });
    Ok(vec![Value::Num(camera.unwrap_or(0.0))])
}

/// `C_TransmogCollection.IsAppearanceHiddenVisual(appearanceID)`: as its
/// recorded appearance says.
fn is_hidden_visual(vm: &mut Vm, c: NativeCall) -> Ret {
    let visual = num(vm, &c, 1).unwrap_or(0.0) as i64;
    let hidden = appearances()
        .get(&visual)
        .is_some_and(|a| field_bool(a, "isHideVisual"));
    Ok(vec![Value::Bool(hidden)])
}

/// `C_TransmogCollection.GetIsAppearanceFavorite(appearanceID)`: as set,
/// else as recorded; `SetIsAppearanceFavorite` sets it; `HasFavorites` any
/// (unverified: the client's events for it).
fn appearance_favorite(vm: &mut Vm, visual: i64) -> bool {
    let recorded = appearances()
        .get(&visual)
        .is_some_and(|a| field_bool(a, "isFavorite"));
    flag(vm, &format!("transmog.favorite.{visual}"), recorded)
}

fn get_appearance_favorite(vm: &mut Vm, c: NativeCall) -> Ret {
    let visual = num(vm, &c, 1).unwrap_or(0.0) as i64;
    Ok(vec![Value::Bool(appearance_favorite(vm, visual))])
}

fn set_appearance_favorite(vm: &mut Vm, c: NativeCall) -> Ret {
    let visual = num(vm, &c, 1).unwrap_or(0.0) as i64;
    let recorded = appearances()
        .get(&visual)
        .is_some_and(|a| field_bool(a, "isFavorite"));
    let on = truthy(vm, &c, 2);
    set_flag(vm, &format!("transmog.favorite.{visual}"), on, recorded);
    Ok(vec![])
}

fn has_transmog_favorites(vm: &mut Vm, _c: NativeCall) -> Ret {
    let recorded: Vec<i64> = appearances()
        .iter()
        .filter(|(_, a)| field_bool(a, "isFavorite"))
        .map(|(v, _)| *v)
        .collect();
    let mut any = false;
    for v in recorded {
        any |= appearance_favorite(vm, v);
    }
    for key in stored(vm, "transmog.favorite.") {
        let visual: i64 = key["transmog.favorite.".len()..].parse().unwrap_or(0);
        any |= appearance_favorite(vm, visual);
    }
    Ok(vec![Value::Bool(any)])
}

/// The item an `ItemInfo` argument names: an item ID, or the ID in an item
/// link or `item:` string.
fn item_id(vm: &Vm, c: &NativeCall, n: usize) -> Option<i64> {
    match vm.hidden(c.arg(n)) {
        Value::Num(x) => Some(x as i64),
        Value::Str(s) => {
            let s = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            if let Ok(id) = s.trim().parse::<f64>() {
                return Some(id as i64);
            }
            let rest = &s[s.find("item:")? + 5..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        }
        _ => None,
    }
}

/// The recorded sources by item.
fn item_index() -> &'static BTreeMap<i64, Vec<&'static str>> {
    static BY_ITEM: OnceLock<BTreeMap<i64, Vec<&'static str>>> = OnceLock::new();
    BY_ITEM.get_or_init(|| {
        let mut out: BTreeMap<i64, Vec<&'static str>> = BTreeMap::new();
        for entry in sources().values().copied() {
            if let Some(item) = field_num(entry, "itemID") {
                out.entry(item as i64).or_default().push(entry);
            }
        }
        out
    })
}

/// The recorded sources of an item (by modifier, when one is given).
fn item_sources(item: i64, modifier: Option<i64>) -> Vec<&'static str> {
    item_index()
        .get(&item)
        .map(|list| {
            list.iter()
                .copied()
                .filter(|e| {
                    modifier.is_none_or(|m| field_num(e, "itemModID").unwrap_or(0.0) as i64 == m)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `C_TransmogCollection.GetItemInfo(itemInfo)`: the item's appearance and
/// source, as recorded (its first source; unverified for an item with
/// several).
fn transmog_item_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(item) = item_id(vm, &c, 1) else {
        return Ok(vec![]);
    };
    Ok(match item_sources(item, None).first() {
        Some(e) => vec![
            Value::Num(field_num(e, "visualID").unwrap_or(0.0)),
            Value::Num(field_num(e, "sourceID").unwrap_or(0.0)),
        ],
        None => vec![],
    })
}

/// `C_TransmogCollection.IsCategoryValidForItem(category, itemInfo)`: the
/// item's recorded source is in it.
fn category_valid_for_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let category = num(vm, &c, 1);
    let valid = item_id(vm, &c, 2).is_some_and(|item| {
        item_sources(item, None)
            .iter()
            .any(|e| field_num(e, "categoryID") == category)
    });
    Ok(vec![Value::Bool(valid)])
}

/// `C_TransmogCollection.PlayerHasTransmog(itemID [, modID])`,
/// `PlayerHasTransmogByItemInfo(itemInfo)`: a recorded source of it
/// collected.
fn player_has_transmog(vm: &mut Vm, c: NativeCall) -> Ret {
    let modifier = num(vm, &c, 2).unwrap_or(0.0) as i64;
    let has = item_id(vm, &c, 1).is_some_and(|item| {
        item_sources(item, Some(modifier))
            .iter()
            .any(|e| field_bool(e, "isCollected"))
    });
    Ok(vec![Value::Bool(has)])
}

fn player_has_transmog_by_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let has = item_id(vm, &c, 1).is_some_and(|item| {
        item_sources(item, None)
            .iter()
            .any(|e| field_bool(e, "isCollected"))
    });
    Ok(vec![Value::Bool(has)])
}

/// `C_Transmog.GetSlotForInventoryType(inventoryType)`: the inventory slot
/// an inventory type (counted from 1, as `AppearanceSourceInfo.invType`
/// is) goes in (unverified).
fn slot_for_inventory_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let inv = num(vm, &c, 1).unwrap_or(0.0) as i64 - 1;
    let slot = match inv {
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 | 20 => 5,
        6 => 6,
        7 => 7,
        8 => 8,
        9 => 9,
        10 => 10,
        11 => 11,
        12 => 13,
        13 | 17 | 21 => 16,
        14 | 22 | 23 => 17,
        15 | 25 | 26 => 18,
        16 => 15,
        19 => 19,
        _ => return Ok(vec![]),
    };
    Ok(vec![Value::Num(f64::from(slot))])
}

/// `C_Transmog.CanHaveSecondaryAppearanceForSlotID(slotID)`: the shoulders
/// (split shoulders; unverified on Forever).
fn can_have_secondary(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(num(vm, &c, 1) == Some(3.0))])
}

/// `C_Transmog.ExtractTransmogIDList(input)`: the numbers in the text
/// (unverified).
fn extract_transmog_ids(vm: &mut Vm, c: NativeCall) -> Ret {
    let input = text(vm, &c, 1);
    let ids: Vec<Value> = input
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f64>().ok())
        .map(Value::Num)
        .collect();
    Ok(vec![list(vm, ids)])
}

/// `C_TransmogCollection.SetSearch(type, text)`, `ClearSearch(type)`:
/// complete at once (unverified: no search database to load).
fn search_complete(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

// ---------------------------------------------------------------------------
// Transmog sets (recorded `C_TransmogSets.GetAllSets()`).

fn all_sets() -> &'static Vec<&'static str> {
    static SETS: OnceLock<Vec<&'static str>> = OnceLock::new();
    SETS.get_or_init(|| {
        crate::recorded::answers()
            .get("C_TransmogSets.GetAllSets()")
            .copied()
            .map(list_entries)
            .unwrap_or_default()
    })
}

fn set_entry(vm: &Vm, c: &NativeCall) -> Option<&'static str> {
    let id = num(vm, c, 1)?;
    all_sets()
        .iter()
        .copied()
        .find(|e| field_num(e, "setID") == Some(id))
}

fn set_favorite(vm: &mut Vm, entry: &'static str) -> bool {
    let id = field_num(entry, "setID").unwrap_or(0.0) as i64;
    flag(
        vm,
        &format!("sets.favorite.{id}"),
        field_bool(entry, "favorite"),
    )
}

/// A set as a table, its favorite as set.
fn set_value(vm: &mut Vm, entry: &'static str) -> Value {
    let v = entry_value(vm, entry);
    if let Value::Table(t) = v {
        let fav = set_favorite(vm, entry);
        set_field(vm, t, "favorite", Value::Bool(fav));
    }
    v
}

fn sets_where(vm: &mut Vm, keep: impl Fn(&str) -> bool) -> Value {
    let entries: Vec<&'static str> = all_sets().iter().copied().filter(|e| keep(e)).collect();
    let values: Vec<Value> = entries.into_iter().map(|e| set_value(vm, e)).collect();
    list(vm, values)
}

fn is_base(e: &str) -> bool {
    field_num(e, "baseSetID").is_none()
}

fn is_available(e: &str) -> bool {
    field_bool(e, "validForCharacter")
        && (!field_bool(e, "hiddenUntilCollected") || field_bool(e, "collected"))
}

fn is_usable(e: &str) -> bool {
    field_bool(e, "validForCharacter") && field_bool(e, "collected")
}

/// `C_TransmogSets.GetSetInfo(setID)`: as recorded.
fn get_set_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match set_entry(vm, &c) {
        Some(e) => vec![set_value(vm, e)],
        None => vec![],
    })
}

/// `C_TransmogSets.GetBaseSetID(setID)`: its base, itself for a base set.
fn get_base_set_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match set_entry(vm, &c) {
        Some(e) => vec![Value::Num(
            field_num(e, "baseSetID")
                .or_else(|| field_num(e, "setID"))
                .unwrap_or(0.0),
        )],
        None => vec![],
    })
}

/// `C_TransmogSets.GetVariantSets(setID)`: the sets based on it.
fn get_variant_sets(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = num(vm, &c, 1) else {
        return Ok(vec![]);
    };
    Ok(vec![sets_where(vm, |e| {
        field_num(e, "baseSetID") == Some(id)
    })])
}

/// `C_TransmogSets.GetBaseSets()`, `GetAvailableSets()`, `GetUsableSets()`:
/// the base sets not hidden until collected; those valid for the
/// character and not hidden; those valid and collected (unverified: which
/// the client lists; the recorded set is hidden until collected, and its
/// camera IDs are not recorded, so the wardrobe's sets tab shows none).
fn get_base_sets(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![sets_where(vm, |e| {
        is_base(e) && (!field_bool(e, "hiddenUntilCollected") || field_bool(e, "collected"))
    })])
}

fn get_available_sets(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![sets_where(vm, is_available)])
}

fn get_usable_sets(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![sets_where(vm, is_usable)])
}

fn has_available_sets(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        all_sets().iter().any(|e| is_available(e)),
    )])
}

fn has_usable_sets(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(all_sets().iter().any(|e| is_usable(e)))])
}

fn has_any_valid_sets(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        all_sets()
            .iter()
            .any(|e| field_bool(e, "validForCharacter")),
    )])
}

/// The base sets' collected and total counts, of those `keep` keeps.
fn base_counts(keep: impl Fn(&str) -> bool) -> Ret {
    let base: Vec<&'static str> = all_sets()
        .iter()
        .copied()
        .filter(|e| is_base(e) && keep(e))
        .collect();
    let collected = base.iter().filter(|e| field_bool(e, "collected")).count();
    Ok(vec![
        Value::Num(collected as f64),
        Value::Num(base.len() as f64),
    ])
}

fn full_base_counts(_vm: &mut Vm, _c: NativeCall) -> Ret {
    base_counts(|_| true)
}

fn valid_base_counts(_vm: &mut Vm, _c: NativeCall) -> Ret {
    base_counts(|e| field_bool(e, "validForCharacter"))
}

/// `GetFilteredBaseSetsCounts()`: the sets' collected / not collected
/// filters applied (unverified: the PvE/PvP filters).
fn filtered_base_counts(vm: &mut Vm, _c: NativeCall) -> Ret {
    let collected = flag(vm, "basesets.filter.1", true);
    let uncollected = flag(vm, "basesets.filter.2", true);
    base_counts(|e| {
        if field_bool(e, "collected") {
            collected
        } else {
            uncollected
        }
    })
}

fn is_base_set_collected(vm: &mut Vm, c: NativeCall) -> Ret {
    let collected = set_entry(vm, &c).is_some_and(|e| field_bool(e, "collected"));
    Ok(vec![Value::Bool(collected)])
}

fn is_set_visible(vm: &mut Vm, c: NativeCall) -> Ret {
    let visible = set_entry(vm, &c)
        .is_some_and(|e| !field_bool(e, "hiddenUntilCollected") || field_bool(e, "collected"));
    Ok(vec![Value::Bool(visible)])
}

/// `C_TransmogSets.GetIsFavorite(setID)`: as set, else as recorded; a
/// variant's group favorite is its base's (unverified).
fn get_set_is_favorite(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(e) = set_entry(vm, &c) else {
        return Ok(vec![]);
    };
    let own = set_favorite(vm, e);
    let base = field_num(e, "baseSetID").and_then(|b| {
        all_sets()
            .iter()
            .copied()
            .find(|s| field_num(s, "setID") == Some(b))
    });
    let group = match base {
        Some(b) => set_favorite(vm, b),
        None => own,
    };
    Ok(vec![Value::Bool(own), Value::Bool(group)])
}

fn set_set_is_favorite(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(e) = set_entry(vm, &c) else {
        return Ok(vec![]);
    };
    let id = field_num(e, "setID").unwrap_or(0.0) as i64;
    let on = truthy(vm, &c, 2);
    set_flag(
        vm,
        &format!("sets.favorite.{id}"),
        on,
        field_bool(e, "favorite"),
    );
    Ok(vec![])
}

/// `C_TransmogSets.GetValidClassForSet(setID)`: none for a set any class
/// may wear (class mask 0), else the mask's lowest class (unverified).
fn get_valid_class_for_set(vm: &mut Vm, c: NativeCall) -> Ret {
    let mask = set_entry(vm, &c)
        .and_then(|e| field_num(e, "classMask"))
        .unwrap_or(0.0) as u64;
    if mask == 0 {
        return Ok(vec![Value::Nil]);
    }
    Ok(vec![Value::Num(f64::from(mask.trailing_zeros() + 1))])
}

/// A known set's items are not recorded: none (unverified).
fn set_items(vm: &mut Vm, c: NativeCall) -> Ret {
    if set_entry(vm, &c).is_none() {
        return Ok(vec![]);
    }
    Ok(vec![Value::Table(vm.new_table())])
}

/// `C_TransmogSets.GetTransmogSetsClassFilter()`: the player's class until
/// set.
fn get_sets_class_filter(vm: &mut Vm, _c: NativeCall) -> Ret {
    let class = engine(vm).units.list[units::PLAYER.0].class_id;
    Ok(vec![Value::Num(number(vm, "sets.class", class))])
}

fn set_sets_class_filter(vm: &mut Vm, c: NativeCall) -> Ret {
    let class = engine(vm).units.list[units::PLAYER.0].class_id;
    let n = num(vm, &c, 1).unwrap_or(class);
    set_number(vm, "sets.class", n, class);
    Ok(vec![])
}

// ---------------------------------------------------------------------------
// Outfits (recorded `C_TransmogOutfitInfo.GetOutfitsInfo()`).

fn outfits() -> &'static Vec<&'static str> {
    static OUTFITS: OnceLock<Vec<&'static str>> = OnceLock::new();
    OUTFITS.get_or_init(|| {
        crate::recorded::answers()
            .get("C_TransmogOutfitInfo.GetOutfitsInfo()")
            .copied()
            .map(list_entries)
            .unwrap_or_default()
    })
}

fn outfit_where(vm: &mut Vm, keep: impl Fn(&str) -> bool) -> Ret {
    Ok(match outfits().iter().copied().find(|e| keep(e)) {
        Some(e) => vec![entry_value(vm, e)],
        None => vec![],
    })
}

/// `C_TransmogOutfitInfo.GetOutfitInfo(outfitID)`, `GetOutfitInfoByName`,
/// `GetOutfitInfoByPlayerFacingIndex`: the recorded outfit.
fn get_outfit_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(vm, &c, 1);
    outfit_where(vm, |e| id.is_some() && field_num(e, "outfitID") == id)
}

fn get_outfit_by_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text(vm, &c, 1);
    outfit_where(vm, |e| field_str(e, "name") == Some(name.as_str()))
}

fn get_outfit_by_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = num(vm, &c, 1);
    outfit_where(vm, |e| {
        i.is_some() && field_num(e, "playerFacingOutfitIndex") == i
    })
}

/// `GetMaxNumberOfUsableOutfits()`: the outfits recorded (unverified: the
/// client's limit not asked).
fn max_usable_outfits(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(outfits().len() as f64)])
}

/// A number the client was not asked (the outfit shown and viewed, the
/// outfits' costs and limits, the custom sets' limit): as recorded, else 0,
/// noted.
fn noted_zero(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(name) = crate::recorded::called_name(vm, &c)
        && let Some(v) = recorded_or_noted(vm, &name, &c)
    {
        return Ok(v);
    }
    Ok(vec![Value::Num(0.0)])
}

/// `C_TransmogOutfitInfo.IsSlotWeaponSlot(slot)`: the weapons'
/// (`Enum.TransmogOutfitSlot` 12 to 14).
fn is_weapon_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    let slot = num(vm, &c, 1).unwrap_or(-1.0);
    Ok(vec![Value::Bool((12.0..=14.0).contains(&slot))])
}

// ---------------------------------------------------------------------------
// Achievements and statistics (recorded by `/stdtest engine achievements`).

fn lookup(vm: &mut Vm, call: &str) -> Option<Vec<Value>> {
    crate::recorded::lookup_text(vm, call)
}

/// A statistic's value as the client shows it: a statistic of one
/// criterion its quantity's text; of several, their quantities' sum; "--"
/// for none (the criteria recorded; unverified: the sum); empty for an ID
/// without criteria, no statistic (recorded: `GetStatistic(122)`, a
/// category's ID, is "").
fn statistic_value(vm: &mut Vm, id: i64) -> String {
    let n = num_of(first(lookup(
        vm,
        &format!("GetAchievementNumCriteria({id})"),
    )))
    .unwrap_or(0.0) as i64;
    if n == 0 {
        return String::new();
    }
    let mut sum = 0.0;
    let mut single = None;
    for i in 1..=n {
        let Some(v) = lookup(vm, &format!("GetAchievementCriteriaInfo({id}, {i})")) else {
            continue;
        };
        sum += v.get(3).copied().and_then(num_of).unwrap_or(0.0);
        if n == 1 {
            single = v.get(8).and_then(|s| str_of(vm, *s));
        }
    }
    match single {
        Some(s) => s,
        None if sum > 0.0 && sum.fract() == 0.0 => format!("{}", sum as i64),
        None if sum > 0.0 => format!("{sum}"),
        None => "--".to_string(),
    }
}

/// `GetStatistic(id)` or `GetStatistic(categoryID, index)`: the value's
/// text, not skipped, and the statistic's ID (a number here; the wiki
/// calls it a string, unverified).
fn get_statistic(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(first_arg) = num(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let id = match num(vm, &c, 2) {
        Some(i) => {
            let call = format!("GetAchievementInfo({}, {})", first_arg as i64, i as i64);
            match num_of(first(lookup(vm, &call))) {
                Some(id) => id as i64,
                None => return Ok(vec![]),
            }
        }
        None => first_arg as i64,
    };
    let value = statistic_value(vm, id);
    Ok(vec![
        vm.str(&value),
        Value::Bool(false),
        Value::Num(id as f64),
    ])
}

/// `HasCompletedAnyAchievement()`: any completed (recorded true;
/// `GetNumCompletedAchievements()` recorded 2 completed).
fn has_completed_any_achievement(vm: &mut Vm, _c: NativeCall) -> Ret {
    let completed = lookup(vm, "GetNumCompletedAchievements()")
        .and_then(|v| v.get(1).copied().and_then(num_of))
        .unwrap_or(1.0);
    Ok(vec![Value::Bool(completed > 0.0)])
}

/// `GetCategoryAchievementPoints(categoryID, includeSubCategories)`: the
/// points of its completed achievements (and its subcategories'), as
/// recorded (unverified; the legacy achievements recorded are worth 0).
fn get_category_points(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(category) = num(vm, &c, 1) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    let mut categories = vec![category as i64];
    if truthy(vm, &c, 2)
        && let Value::Table(t) = first(lookup(vm, "GetCategoryList()"))
    {
        let n = vm.heap.table(t).border();
        let ids: Vec<i64> = (1..=n)
            .filter_map(|i| num_of(vm.heap.table(t).get(Value::Num(i as f64))))
            .map(|id| id as i64)
            .collect();
        for id in ids {
            let parent = lookup(vm, &format!("GetCategoryInfo({id})"))
                .and_then(|v| v.get(1).copied().and_then(num_of));
            if parent == Some(category) {
                categories.push(id);
            }
        }
    }
    let mut points = 0.0;
    for cat in categories {
        let n = num_of(first(lookup(
            vm,
            &format!("GetCategoryNumAchievements({cat})"),
        )))
        .unwrap_or(0.0) as i64;
        for i in 1..=n {
            if let Some(v) = lookup(vm, &format!("GetAchievementInfo({cat}, {i})"))
                && matches!(v.get(3), Some(Value::Bool(true)))
            {
                points += v.get(2).copied().and_then(num_of).unwrap_or(0.0);
            }
        }
    }
    Ok(vec![Value::Num(points)])
}

/// `GetAchievementCriteriaInfoByID(achievementID, criteriaID)`: the
/// recorded criterion with that ID (`GetAchievementCriteriaInfo`'s values).
fn criteria_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(id), Some(criteria)) = (num(vm, &c, 1), num(vm, &c, 2)) else {
        return Ok(vec![]);
    };
    let id = id as i64;
    let n = num_of(first(lookup(
        vm,
        &format!("GetAchievementNumCriteria({id})"),
    )))
    .unwrap_or(0.0) as i64;
    for i in 1..=n {
        if let Some(v) = lookup(vm, &format!("GetAchievementCriteriaInfo({id}, {i})"))
            && v.get(9).copied().and_then(num_of) == Some(criteria)
        {
            return Ok(v);
        }
    }
    Ok(vec![])
}

/// `GetAchievementSearchProgress()`, `GetAchievementSearchSize()`: the
/// search done, its results' count both (unverified: the client's counts
/// what it searched; never 0, Blizzard's bar divides by the size).
fn achievement_search_count(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = engine(vm).world.achievements.results.len().max(1);
    Ok(vec![Value::Num(n as f64)])
}

/// `GetAchievementGuildRep(id)`: no guild reputation required.
fn achievement_guild_rep(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Bool(false),
        Value::Bool(false),
        Value::Num(0.0),
    ])
}

// ---------------------------------------------------------------------------
// Content tracking: nothing trackable (recorded `GetTrackedIDs`: none).

/// `C_ContentTracking.GetBestMapForTrackable`, `GetNextWaypointForTrackable`:
/// `Enum.ContentTrackingResult.Failure` (2) and no map.
fn tracking_failure(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(2.0), Value::Nil])
}

/// `C_ContentTracking.IsNavigable`: failure, not navigable.
fn tracking_not_navigable(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(2.0), Value::Bool(false)])
}

/// `C_TransmogSets.GetCameraIDs()`: 1019 and 1044 (recorded; Blizzard's
/// wardrobe asks the camera of what it gets).
fn get_camera_ids(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1019.0), Value::Num(1044.0)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        // Mounts: the recorded list; none displayed or collected.
        ("C_MountJournal.GetMountIDs", get_mount_ids as NativeFn),
        ("C_MountJournal.GetMountInfoByID", get_mount_info_by_id),
        (
            "C_MountJournal.GetMountInfoExtraByID",
            get_mount_info_extra_by_id,
        ),
        ("C_MountJournal.GetMountFromSpell", get_mount_from_spell),
        ("C_MountJournal.GetMountFromItem", get_mount_from_item),
        ("C_MountJournal.GetMountUsabilityByID", get_mount_usability),
        ("C_MountJournal.SummonByID", nothing),
        ("C_MountJournal.Dismiss", nothing),
        ("C_MountJournal.Pickup", nothing),
        ("C_MountJournal.SetIsFavorite", nothing),
        ("C_MountJournal.GetIsFavorite", mount_is_favorite),
        ("C_MountJournal.NeedsFanfare", no),
        ("C_MountJournal.ClearFanfare", nothing),
        ("C_MountJournal.ClearRecentFanfares", nothing),
        // No mount displayed (recorded `GetNumDisplayedMounts`: 0).
        ("C_MountJournal.GetDisplayedMountID", nothing),
        ("C_MountJournal.GetDisplayedMountInfoExtra", nothing),
        (
            "C_MountJournal.GetDisplayedMountAllCreatureDisplayInfo",
            nothing,
        ),
        ("C_MountJournal.GetMountAllCreatureDisplayInfoByID", nothing),
        (
            "C_MountJournal.GetAllCreatureDisplayIDsForMountID",
            empty_table,
        ),
        ("C_MountJournal.GetMountLink", nil),
        ("C_MountJournal.GetCollectedDragonridingMounts", empty_table),
        // Mount equipment: not available (recorded
        // `MountEquipmentAvailable`: false).
        ("C_MountJournal.GetAppliedMountEquipmentID", nil),
        ("C_MountJournal.ApplyMountEquipment", no),
        ("C_MountJournal.IsItemMountEquipment", no),
        ("C_MountJournal.IsMountEquipmentApplied", no),
        ("C_MountJournal.AreMountEquipmentEffectsSuppressed", no),
        ("C_MountJournal.GetMountEquipmentUnlockLevel", zero),
        // Dynamic flight: not unlocked (recorded `IsDragonridingUnlocked`:
        // false).
        ("C_MountJournal.PickupDynamicFlightMode", nothing),
        ("C_MountJournal.SwapDynamicFlightMode", nothing),
        // The mount journal's filters.
        (
            "C_MountJournal.GetCollectedFilterSetting",
            mount_collected_filter,
        ),
        (
            "C_MountJournal.SetCollectedFilterSetting",
            set_mount_collected_filter,
        ),
        ("C_MountJournal.IsSourceChecked", mount_source_checked),
        ("C_MountJournal.SetSourceFilter", set_mount_source_filter),
        ("C_MountJournal.SetAllSourceFilters", set_all_mount_sources),
        ("C_MountJournal.IsTypeChecked", mount_type_checked),
        ("C_MountJournal.SetTypeFilter", set_mount_type_filter),
        ("C_MountJournal.SetAllTypeFilters", set_all_mount_types),
        ("C_MountJournal.IsValidSourceFilter", is_valid_mount_source),
        ("C_MountJournal.IsValidTypeFilter", is_valid_mount_type),
        ("C_MountJournal.SetSearch", set_mount_search),
        (
            "C_MountJournal.IsUsingDefaultFilters",
            mount_default_filters,
        ),
        (
            "C_MountJournal.SetDefaultFilters",
            set_mount_default_filters,
        ),
        // Titles.
        ("GetNumTitles", get_num_titles),
        ("GetTitleName", get_title_name),
        ("IsTitleKnown", is_title_known),
        ("GetCurrentTitle", get_current_title),
        ("SetCurrentTitle", nothing),
        // Toys: none (recorded `C_ToyBoxInfo.HasAnyToy`: false; none
        // displayed, `GetNumLearnedDisplayedToys`, `GetNumTotalDisplayedToys`
        // recorded 0).
        ("PlayerHasToy", no),
        ("C_ToyBox.GetToyInfo", get_toy_info),
        ("C_ToyBox.GetToyLink", nil),
        ("C_ToyBox.IsToyUsable", no),
        ("C_ToyBox.GetIsFavorite", no),
        ("C_ToyBox.SetIsFavorite", nothing),
        ("C_ToyBox.PickupToyBoxItem", nothing),
        ("C_ToyBox.ForceToyRefilter", nothing),
        ("C_ToyBox.GetNumLearnedDisplayedToys", zero),
        ("C_ToyBox.GetNumTotalDisplayedToys", zero),
        ("C_ToyBoxInfo.NeedsFanfare", no),
        ("C_ToyBoxInfo.ClearFanfare", nothing),
        // No toy source offered (unverified: the heirlooms' recorded
        // `IsHeirloomSourceValid` are all false; the toys' not asked).
        ("C_ToyBoxInfo.IsToySourceValid", no),
        ("C_ToyBox.GetCollectedShown", toy_collected_shown),
        ("C_ToyBox.SetCollectedShown", set_toy_collected_shown),
        ("C_ToyBox.GetUncollectedShown", toy_uncollected_shown),
        ("C_ToyBox.SetUncollectedShown", set_toy_uncollected_shown),
        ("C_ToyBox.GetUnusableShown", toy_unusable_shown),
        ("C_ToyBox.SetUnusableShown", set_toy_unusable_shown),
        ("C_ToyBox.IsSourceTypeFilterChecked", toy_source_checked),
        ("C_ToyBox.SetSourceTypeFilter", set_toy_source_filter),
        ("C_ToyBox.SetAllSourceTypeFilters", set_all_toy_sources),
        (
            "C_ToyBox.IsExpansionTypeFilterChecked",
            toy_expansion_checked,
        ),
        ("C_ToyBox.SetExpansionTypeFilter", set_toy_expansion_filter),
        (
            "C_ToyBox.SetAllExpansionTypeFilters",
            set_all_toy_expansions,
        ),
        ("C_ToyBox.SetFilterString", set_toy_search),
        ("C_ToyBoxInfo.IsUsingDefaultFilters", toy_default_filters),
        ("C_ToyBoxInfo.SetDefaultFilters", set_toy_default_filters),
        // Battle pets: none owned (recorded `C_PetJournal.GetNumPets`: 0, 0);
        // the species' data is not recorded.
        ("C_PetJournal.GetOwnedPetIDs", empty_table),
        ("C_PetJournal.GetPetInfoByPetID", nothing),
        ("C_PetJournal.GetPetInfoTableByPetID", nothing),
        ("C_PetJournal.GetPetInfoBySpeciesID", nothing),
        ("C_PetJournal.GetPetInfoTableBySpeciesID", nothing),
        ("C_PetJournal.GetPetInfoByItemID", nothing),
        ("C_PetJournal.GetPetStats", nothing),
        ("C_PetJournal.GetPetAbilityInfo", nothing),
        ("C_PetJournal.GetPetAbilityList", empty_tables),
        ("C_PetJournal.GetPetAbilityListTable", empty_table),
        ("C_PetJournal.GetPetModelSceneInfoBySpeciesID", nothing),
        ("C_PetJournal.GetBattlePetLink", nil),
        ("C_PetJournal.GetOwnedBattlePetString", nil),
        ("C_PetJournal.GetNonBattlePetLinkByIndex", nothing),
        ("C_PetJournal.FindPetIDByName", nothing),
        ("C_PetJournal.GetNumCollectedInfo", zero_zero),
        ("C_PetJournal.GetNumPetsInJournal", zero_zero),
        ("C_PetJournal.GetNumDisplays", nil),
        ("C_PetJournal.GetDisplayIDByIndex", nil),
        ("C_PetJournal.GetDisplayProbabilityByIndex", nil),
        ("C_PetJournal.PetUsesRandomDisplay", nil),
        ("C_PetJournal.GetPetLoadOutInfo", get_pet_loadout_info),
        ("C_PetJournal.SetPetLoadOutInfo", nothing),
        ("C_PetJournal.SetAbility", nothing),
        ("C_PetJournal.GetPetTeamAverageLevel", zero),
        ("C_PetJournal.GetSummonedPetGUID", nil),
        ("C_PetJournal.GetSummonRandomFavoritePetGUID", nil),
        ("C_PetJournal.GetPetSummonInfo", get_pet_summon_info),
        ("C_PetJournal.GetPetCooldownByGUID", get_pet_cooldown),
        ("C_PetJournal.GetNumPetTypes", get_num_pet_types),
        ("C_PetJournal.HasFavoritePets", no),
        ("C_PetJournal.PetIsFavorite", no),
        ("C_PetJournal.PetIsSummonable", no),
        ("C_PetJournal.IsCurrentlySummoned", no),
        ("C_PetJournal.PetIsRevoked", no),
        ("C_PetJournal.PetIsSlotted", no),
        ("C_PetJournal.PetIsLockedForConvert", no),
        ("C_PetJournal.PetCanBeReleased", no),
        ("C_PetJournal.PetIsHurt", no),
        ("C_PetJournal.PetIsTradable", no),
        ("C_PetJournal.PetIsCapturable", no),
        ("C_PetJournal.PetIsUsable", no),
        ("C_PetJournal.PetNeedsFanfare", no),
        ("C_PetJournal.ClearFanfare", nothing),
        ("C_PetJournal.ClearRecentFanfares", nothing),
        // The journal unlocked and writable, no battles to find
        // (unverified).
        ("C_PetJournal.IsJournalUnlocked", yes),
        ("C_PetJournal.IsJournalReadOnly", no),
        ("C_PetJournal.IsFindBattleEnabled", no),
        // A pet's actions: none to act on.
        ("C_PetJournal.SummonPetByGUID", nothing),
        ("C_PetJournal.SummonRandomPet", nothing),
        ("C_PetJournal.DismissSummonedPet", nothing),
        ("C_PetJournal.PickupPet", nothing),
        ("C_PetJournal.PickupSummonRandomPet", nothing),
        ("C_PetJournal.SetCustomName", nothing),
        ("C_PetJournal.SetFavorite", nothing),
        ("C_PetJournal.CagePetByID", nothing),
        ("C_PetJournal.ReleasePetByID", nothing),
        ("C_PetJournal.SpellTargetBattlePet", nothing),
        ("C_PetJournal.SetHoveredBattlePet", nothing),
        ("C_PetJournal.ClearHoveredBattlePet", nothing),
        // The pet journal's filters, search and sort.
        ("C_PetJournal.IsFilterChecked", pet_filter_checked),
        ("C_PetJournal.SetFilterChecked", set_pet_filter_checked),
        ("C_PetJournal.IsPetSourceChecked", pet_source_checked),
        ("C_PetJournal.SetPetSourceChecked", set_pet_source_checked),
        ("C_PetJournal.SetAllPetSourcesChecked", set_all_pet_sources),
        ("C_PetJournal.IsPetTypeChecked", pet_type_checked),
        ("C_PetJournal.SetPetTypeFilter", set_pet_type_checked),
        ("C_PetJournal.SetAllPetTypesChecked", set_all_pet_types),
        ("C_PetJournal.SetSearchFilter", set_pet_search),
        ("C_PetJournal.ClearSearchFilter", clear_pet_search),
        ("C_PetJournal.GetSearchFilter", get_pet_search),
        ("C_PetJournal.SetPetSortParameter", set_pet_sort),
        ("C_PetJournal.GetPetSortParameter", get_pet_sort),
        ("C_PetJournal.IsUsingDefaultFilters", pet_default_filters),
        ("C_PetJournal.SetDefaultFilters", set_pet_default_filters),
        // Pet battles: not in one (recorded `C_PetBattles.IsInBattle`:
        // false): a battle's pets, abilities, auras and states nothing.
        ("C_PetBattles.GetActivePet", nothing),
        ("C_PetBattles.GetAbilityInfo", nothing),
        ("C_PetBattles.GetAbilityInfoByID", nothing),
        ("C_PetBattles.GetAbilityState", nothing),
        ("C_PetBattles.GetAbilityEffectInfo", nothing),
        ("C_PetBattles.GetAbilityProcTurnIndex", nothing),
        ("C_PetBattles.GetAbilityStateModification", nothing),
        ("C_PetBattles.GetAuraInfo", nothing),
        ("C_PetBattles.GetNumAuras", zero),
        ("C_PetBattles.GetPetType", nothing),
        ("C_PetBattles.GetPetSpeciesID", nothing),
        ("C_PetBattles.GetDisplayID", nothing),
        ("C_PetBattles.GetBreedQuality", nothing),
        ("C_PetBattles.GetName", nothing),
        ("C_PetBattles.GetIcon", nothing),
        ("C_PetBattles.GetStateValue", nothing),
        ("C_PetBattles.GetAllStates", empty_table),
        ("C_PetBattles.GetTurnTimeInfo", zero_zero),
        ("C_PetBattles.GetForfeitPenalty", zero),
        ("C_PetBattles.GetPlayerTrapAbility", zero),
        ("C_PetBattles.GetAttackModifier", get_attack_modifier),
        ("C_PetBattles.IsWildBattle", no),
        ("C_PetBattles.IsPlayerNPC", no),
        ("C_PetBattles.IsWaitingOnOpponent", no),
        ("C_PetBattles.CanActivePetSwapOut", no),
        ("C_PetBattles.CanAcceptQueuedPVPMatch", no),
        ("C_PetBattles.ForfeitGame", nothing),
        ("C_PetBattles.UseAbility", nothing),
        ("C_PetBattles.UseTrap", nothing),
        ("C_PetBattles.SkipTurn", nothing),
        ("C_PetBattles.ChangePet", nothing),
        ("C_PetBattles.StartPVPDuel", nothing),
        ("C_PetBattles.AcceptPVPDuel", nothing),
        ("C_PetBattles.CancelPVPDuel", nothing),
        ("C_PetBattles.StartPVPMatchmaking", nothing),
        ("C_PetBattles.StopPVPMatchmaking", nothing),
        ("C_PetBattles.AcceptQueuedPVPMatch", nothing),
        ("C_PetBattles.DeclineQueuedPVPMatch", nothing),
        ("C_PetBattles.SetPendingReportBattlePetTarget", nothing),
        ("C_PetBattles.SetPendingReportTargetFromUnit", nothing),
        // Companions (the old pet and mount spells): nothing to call.
        ("CallCompanion", nothing),
        ("DismissCompanion", nothing),
        // Heirlooms: none (recorded `GetNumHeirlooms`: 0,
        // `HeirloomsAvailable`: false).
        ("C_Heirloom.PlayerHasHeirloom", no),
        ("C_Heirloom.IsItemHeirloom", no),
        ("C_Heirloom.GetHeirloomInfo", nothing),
        ("C_Heirloom.GetHeirloomLink", nil),
        ("C_Heirloom.GetHeirloomMaxUpgradeLevel", nil),
        ("C_Heirloom.GetHeirloomItemIDs", empty_table),
        ("C_Heirloom.GetNumDisplayedHeirlooms", zero),
        ("C_Heirloom.GetHeirloomItemIDFromDisplayedIndex", nil),
        ("C_Heirloom.CanHeirloomUpgradeFromPending", no),
        ("C_Heirloom.IsPendingHeirloomUpgrade", no),
        ("C_Heirloom.ShouldShowHeirloomHelp", no),
        ("C_Heirloom.CreateHeirloom", nothing),
        ("C_Heirloom.UpgradeHeirloom", nothing),
        ("C_Heirloom.SetSearch", set_heirloom_search),
        (
            "C_Heirloom.GetCollectedHeirloomFilter",
            heirloom_collected_shown,
        ),
        (
            "C_Heirloom.SetCollectedHeirloomFilter",
            set_heirloom_collected_shown,
        ),
        (
            "C_Heirloom.GetUncollectedHeirloomFilter",
            heirloom_uncollected_shown,
        ),
        (
            "C_Heirloom.SetUncollectedHeirloomFilter",
            set_heirloom_uncollected_shown,
        ),
        ("C_Heirloom.GetHeirloomSourceFilter", heirloom_source_filter),
        (
            "C_Heirloom.SetHeirloomSourceFilter",
            set_heirloom_source_filter,
        ),
        ("C_Heirloom.GetClassAndSpecFilters", get_heirloom_class_spec),
        ("C_Heirloom.SetClassAndSpecFilters", set_heirloom_class_spec),
        (
            "C_HeirloomInfo.IsUsingDefaultFilters",
            heirloom_default_filters,
        ),
        (
            "C_HeirloomInfo.SetDefaultFilters",
            set_heirloom_default_filters,
        ),
        // The wardrobe's sources and appearances, as recorded.
        ("C_TransmogCollection.GetSourceInfo", get_source_info),
        ("C_TransmogCollection.GetSourceItemID", get_source_item_id),
        ("C_Transmog.GetItemIDForSource", get_item_id_for_source),
        (
            "C_TransmogCollection.GetCategoryForItem",
            get_category_for_item,
        ),
        (
            "C_TransmogOutfitInfo.GetItemModifiedAppearanceEffectiveCategory",
            effective_category,
        ),
        (
            "C_TransmogCollection.PlayerHasTransmogItemModifiedAppearance",
            source_collected,
        ),
        ("C_TransmogCollection.PlayerKnowsSource", source_collected),
        (
            "C_TransmogCollection.PlayerCanCollectSource",
            can_collect_source,
        ),
        (
            "C_TransmogCollection.AccountCanCollectSource",
            can_collect_source,
        ),
        (
            "C_TransmogCollection.GetAllAppearanceSources",
            get_all_appearance_sources,
        ),
        (
            "C_TransmogCollection.GetAppearanceInfoBySource",
            get_appearance_info_by_source,
        ),
        (
            "C_TransmogCollection.GetAppearanceCameraIDBySource",
            get_camera_by_source,
        ),
        (
            "C_TransmogCollection.IsAppearanceHiddenVisual",
            is_hidden_visual,
        ),
        (
            "C_TransmogCollection.GetIsAppearanceFavorite",
            get_appearance_favorite,
        ),
        (
            "C_TransmogCollection.SetIsAppearanceFavorite",
            set_appearance_favorite,
        ),
        ("C_TransmogCollection.HasFavorites", has_transmog_favorites),
        ("C_TransmogCollection.GetItemInfo", transmog_item_info),
        (
            "C_TransmogCollection.IsCategoryValidForItem",
            category_valid_for_item,
        ),
        (
            "C_TransmogCollection.PlayerHasTransmog",
            player_has_transmog,
        ),
        (
            "C_TransmogCollection.PlayerHasTransmogByItemInfo",
            player_has_transmog_by_item,
        ),
        // Not recorded: a source's icon, holiday, drops; artifacts'
        // appearances (Forever has none); illusions (recorded
        // `GetIllusions`: none).
        ("C_TransmogCollection.GetSourceIcon", nothing),
        ("C_TransmogCollection.GetSourceRequiredHoliday", nothing),
        ("C_TransmogCollection.GetAppearanceSourceDrops", empty_table),
        ("C_TransmogCollection.GetPairedArtifactAppearance", nothing),
        ("C_TransmogCollection.GetArtifactAppearanceStrings", nothing),
        ("C_TransmogCollection.GetIllusionInfo", nothing),
        ("C_TransmogCollection.GetIllusionStrings", nothing),
        ("C_TransmogCollection.CanAppearanceHaveIllusion", no),
        (
            "C_TransmogCollection.IsSpellItemEnchantmentHiddenVisual",
            no,
        ),
        ("C_TransmogCollection.GetFallbackWeaponAppearance", nil),
        ("C_TransmogCollection.IsValidTransmogSource", yes),
        ("C_TransmogCollection.ClearNewAppearance", nothing),
        ("C_TransmogCollection.UpdateUsableAppearances", nothing),
        // The wardrobe's search: complete at once.
        ("C_TransmogCollection.SetSearch", search_complete),
        ("C_TransmogCollection.ClearSearch", search_complete),
        ("C_TransmogCollection.EndSearch", nothing),
        ("C_TransmogCollection.IsSearchDBLoading", no),
        ("C_TransmogCollection.SearchProgress", zero),
        ("C_TransmogCollection.SearchSize", zero),
        ("C_TransmogCollection.SetSearchAndFilterCategory", nothing),
        // The wardrobe's filters.
        (
            "C_TransmogCollection.GetCollectedShown",
            transmog_collected_shown,
        ),
        (
            "C_TransmogCollection.SetCollectedShown",
            set_transmog_collected_shown,
        ),
        (
            "C_TransmogCollection.GetUncollectedShown",
            transmog_uncollected_shown,
        ),
        (
            "C_TransmogCollection.SetUncollectedShown",
            set_transmog_uncollected_shown,
        ),
        (
            "C_TransmogCollection.IsSourceTypeFilterChecked",
            transmog_source_checked,
        ),
        (
            "C_TransmogCollection.SetSourceTypeFilter",
            set_transmog_source_filter,
        ),
        (
            "C_TransmogCollection.SetAllSourceTypeFilters",
            set_all_transmog_sources,
        ),
        (
            "C_TransmogCollection.AreAllSourceTypeFiltersChecked",
            all_transmog_sources_checked,
        ),
        (
            "C_TransmogCollection.SetAllCollectionTypeFilters",
            set_all_collection_types,
        ),
        (
            "C_TransmogCollection.AreAllCollectionTypeFiltersChecked",
            all_collection_types_checked,
        ),
        (
            "C_TransmogCollection.GetAllFactionsShown",
            transmog_factions_shown,
        ),
        (
            "C_TransmogCollection.SetAllFactionsShown",
            set_transmog_factions_shown,
        ),
        (
            "C_TransmogCollection.GetAllRacesShown",
            transmog_races_shown,
        ),
        (
            "C_TransmogCollection.SetAllRacesShown",
            set_transmog_races_shown,
        ),
        // The class filter's getter is the player's class (queries.rs).
        ("C_TransmogCollection.SetClassFilter", nothing),
        (
            "C_TransmogCollection.IsUsingDefaultFilters",
            transmog_default_filters,
        ),
        (
            "C_TransmogCollection.SetDefaultFilters",
            set_transmog_default_filters,
        ),
        // Custom sets: none saved (unverified: the limit not recorded,
        // noted).
        ("C_TransmogCollection.GetCustomSets", empty_table),
        ("C_TransmogCollection.GetCustomSetInfo", nothing),
        (
            "C_TransmogCollection.GetCustomSetItemTransmogInfoList",
            nothing,
        ),
        ("C_TransmogCollection.GetNumMaxCustomSets", noted_zero),
        ("C_TransmogCollection.NewCustomSet", nil),
        ("C_TransmogCollection.ModifyCustomSet", nothing),
        ("C_TransmogCollection.RenameCustomSet", nothing),
        ("C_TransmogCollection.DeleteCustomSet", nothing),
        ("C_TransmogCollection.IsValidCustomSetName", valid_name),
        (
            "C_TransmogCollection.GetCustomSetHyperlinkFromItemTransmogInfoList",
            nothing,
        ),
        (
            "C_TransmogCollection.GetItemTransmogInfoListFromCustomSetHyperlink",
            nothing,
        ),
        // No unit inspected.
        (
            "C_TransmogCollection.GetInspectItemTransmogInfoList",
            nothing,
        ),
        // C_Transmog.
        (
            "C_Transmog.GetSlotForInventoryType",
            slot_for_inventory_type,
        ),
        (
            "C_Transmog.CanHaveSecondaryAppearanceForSlotID",
            can_have_secondary,
        ),
        ("C_Transmog.ExtractTransmogIDList", extract_transmog_ids),
        ("C_Transmog.GetAllSetAppearancesByID", nil),
        // Not at a transmogrifier (the engine has none).
        ("C_Transmog.IsAtTransmogNPC", no),
        // Transmog sets, as recorded.
        ("C_TransmogSets.GetSetInfo", get_set_info),
        ("C_TransmogSets.GetCameraIDs", get_camera_ids),
        ("C_TransmogSets.GetBaseSetID", get_base_set_id),
        ("C_TransmogSets.GetVariantSets", get_variant_sets),
        ("C_TransmogSets.GetBaseSets", get_base_sets),
        ("C_TransmogSets.GetAvailableSets", get_available_sets),
        ("C_TransmogSets.GetUsableSets", get_usable_sets),
        ("C_TransmogSets.HasAvailableSets", has_available_sets),
        ("C_TransmogSets.HasUsableSets", has_usable_sets),
        ("C_TransmogSets.HasAnyValidSets", has_any_valid_sets),
        ("C_TransmogSets.GetFullBaseSetsCounts", full_base_counts),
        (
            "C_TransmogSets.GetValidBaseSetsCountsForCharacter",
            valid_base_counts,
        ),
        (
            "C_TransmogSets.GetFilteredBaseSetsCounts",
            filtered_base_counts,
        ),
        ("C_TransmogSets.IsBaseSetCollected", is_base_set_collected),
        ("C_TransmogSets.IsSetVisible", is_set_visible),
        ("C_TransmogSets.GetIsFavorite", get_set_is_favorite),
        ("C_TransmogSets.SetIsFavorite", set_set_is_favorite),
        (
            "C_TransmogSets.GetValidClassForSet",
            get_valid_class_for_set,
        ),
        ("C_TransmogSets.GetSetPrimaryAppearances", set_items),
        ("C_TransmogSets.GetAllSourceIDs", set_items),
        ("C_TransmogSets.GetSourcesForSlot", set_items),
        ("C_TransmogSets.GetSourceIDsForSlot", set_items),
        ("C_TransmogSets.GetSetNewSources", set_items),
        ("C_TransmogSets.GetSetsContainingSourceID", empty_table),
        // No new sources (`Constants.Transmog.NoTransmogID`-like 0 for the
        // latest; unverified).
        ("C_TransmogSets.GetLatestSource", zero),
        ("C_TransmogSets.ClearLatestSource", nothing),
        ("C_TransmogSets.IsNewSource", no),
        ("C_TransmogSets.ClearNewSource", nothing),
        ("C_TransmogSets.SetHasNewSources", no),
        ("C_TransmogSets.SetHasNewSourcesForSlot", no),
        ("C_TransmogSets.ClearSetNewSourcesForSlot", nothing),
        ("C_TransmogSets.GetSetsFilter", sets_filter),
        ("C_TransmogSets.SetSetsFilter", set_sets_filter),
        (
            "C_TransmogSets.IsUsingDefaultSetsFilters",
            sets_default_filters,
        ),
        (
            "C_TransmogSets.SetDefaultSetsFilters",
            set_sets_default_filters,
        ),
        ("C_TransmogSets.GetBaseSetsFilter", base_sets_filter),
        ("C_TransmogSets.SetBaseSetsFilter", set_base_sets_filter),
        (
            "C_TransmogSets.IsUsingDefaultBaseSetsFilters",
            base_sets_default_filters,
        ),
        (
            "C_TransmogSets.SetDefaultBaseSetsFilters",
            set_base_sets_default_filters,
        ),
        (
            "C_TransmogSets.GetTransmogSetsClassFilter",
            get_sets_class_filter,
        ),
        (
            "C_TransmogSets.SetTransmogSetsClassFilter",
            set_sets_class_filter,
        ),
        // Outfits, as recorded; the transmogrifier's pending changes and
        // views: none outside its frame.
        ("C_TransmogOutfitInfo.GetOutfitInfo", get_outfit_info),
        (
            "C_TransmogOutfitInfo.GetOutfitInfoByName",
            get_outfit_by_name,
        ),
        (
            "C_TransmogOutfitInfo.GetOutfitInfoByPlayerFacingIndex",
            get_outfit_by_index,
        ),
        (
            "C_TransmogOutfitInfo.GetMaxNumberOfUsableOutfits",
            max_usable_outfits,
        ),
        ("C_TransmogOutfitInfo.GetActiveOutfitID", noted_zero),
        (
            "C_TransmogOutfitInfo.GetCurrentlyViewedOutfitID",
            noted_zero,
        ),
        ("C_TransmogOutfitInfo.GetNextOutfitCost", noted_zero),
        (
            "C_TransmogOutfitInfo.GetMaxNumberOfTotalOutfitsForSource",
            noted_zero,
        ),
        (
            "C_TransmogOutfitInfo.GetNumberOfOutfitsUnlockedForSource",
            noted_zero,
        ),
        ("C_TransmogOutfitInfo.IsSlotWeaponSlot", is_weapon_slot),
        ("C_TransmogOutfitInfo.IsValidTransmogOutfitName", valid_name),
        ("C_TransmogOutfitInfo.GetViewedOutfitSlotInfo", nothing),
        (
            "C_TransmogOutfitInfo.GetCollectionInfoForSlotAndOption",
            nothing,
        ),
        ("C_TransmogOutfitInfo.GetPendingTransmogCost", nothing),
        ("C_TransmogOutfitInfo.GetOptionsForSlot", nothing),
        ("C_TransmogOutfitInfo.GetWeaponOptionsForSlot", nothing),
        (
            "C_TransmogOutfitInfo.GetIllusionDefaultIMAIDForCollectionType",
            nothing,
        ),
        (
            "C_TransmogOutfitInfo.GetAllTransmogOutfitOptionSheatheCategoryInfo",
            nothing,
        ),
        (
            "C_TransmogOutfitInfo.GetEquippedSlotOptionFromTransmogSlot",
            nothing,
        ),
        ("C_TransmogOutfitInfo.GetSlotGroupInfo", nothing),
        ("C_TransmogOutfitInfo.GetSourceIDsForSlot", nothing),
        ("C_TransmogOutfitInfo.GetSetSourcesForSlot", nothing),
        (
            "C_TransmogOutfitInfo.GetUISituationCategoriesAndOptions",
            nothing,
        ),
        ("C_TransmogOutfitInfo.GetUnassignedAtlasForSlot", nothing),
        (
            "C_TransmogOutfitInfo.GetUnassignedDisplayAtlasForSlot",
            nothing,
        ),
        (
            "C_TransmogOutfitInfo.GetTransmogOutfitSlotForInventoryType",
            nothing,
        ),
        ("C_TransmogOutfitInfo.InTransmogEvent", no),
        ("C_TransmogOutfitInfo.TransmogEventActive", no),
        ("C_TransmogOutfitInfo.IsUsableDiscountAvailable", no),
        ("C_TransmogOutfitInfo.GetSecondarySlotState", no),
        ("C_TransmogOutfitInfo.SlotHasSecondary", no),
        ("C_TransmogOutfitInfo.CanPlayerTransmogSlot", no),
        ("C_TransmogOutfitInfo.GetOutfitSituation", no),
        ("C_TransmogOutfitInfo.GetOutfitSituationsEnabled", no),
        ("C_TransmogOutfitInfo.HasPendingOutfitSituations", no),
        ("C_TransmogOutfitInfo.HasPendingOutfitTransmogs", no),
        ("C_TransmogOutfitInfo.IsEquippedGearOutfitDisplayed", no),
        ("C_TransmogOutfitInfo.IsEquippedGearOutfitLocked", no),
        ("C_TransmogOutfitInfo.IsLockedOutfit", no),
        ("C_TransmogOutfitInfo.CommitAndApplyAllPending", nothing),
        ("C_TransmogOutfitInfo.PickupOutfit", nothing),
        ("C_TransmogOutfitInfo.UpdatePendingSituation", nothing),
        ("C_TransmogOutfitInfo.ChangeDisplayedOutfit", nothing),
        ("C_TransmogOutfitInfo.ChangeToOutfit", nothing),
        ("C_TransmogOutfitInfo.ChangeViewedOutfit", nothing),
        ("C_TransmogOutfitInfo.ClearAllPendingSituations", nothing),
        ("C_TransmogOutfitInfo.ClearAllPendingTransmogs", nothing),
        ("C_TransmogOutfitInfo.ClearOutfit", nothing),
        ("C_TransmogOutfitInfo.ClearDisplayedOutfit", nothing),
        ("C_TransmogOutfitInfo.SetPendingTransmog", nothing),
        ("C_TransmogOutfitInfo.AddNewOutfit", nothing),
        ("C_TransmogOutfitInfo.CommitOutfitInfo", nothing),
        ("C_TransmogOutfitInfo.CommitPendingSituations", nothing),
        ("C_TransmogOutfitInfo.ResetOutfitSituations", nothing),
        ("C_TransmogOutfitInfo.RevertPendingTransmog", nothing),
        ("C_TransmogOutfitInfo.SetOutfitSituationsEnabled", nothing),
        ("C_TransmogOutfitInfo.SetOutfitToCustomSet", nothing),
        ("C_TransmogOutfitInfo.SetOutfitToOutfit", nothing),
        ("C_TransmogOutfitInfo.SetOutfitToSet", nothing),
        (
            "C_TransmogOutfitInfo.SetPendingTransmogSheatheCategory",
            nothing,
        ),
        ("C_TransmogOutfitInfo.SetSecondarySlotState", nothing),
        ("C_TransmogOutfitInfo.SetViewedOptionForSlot", nothing),
        ("C_TransmogOutfitInfo.SetViewedWeaponOptionForSlot", nothing),
        // Achievements and statistics.
        ("GetStatistic", get_statistic),
        ("HasCompletedAnyAchievement", has_completed_any_achievement),
        ("GetCategoryAchievementPoints", get_category_points),
        ("GetAchievementCriteriaInfoByID", criteria_by_id),
        ("GetAchievementSearchProgress", achievement_search_count),
        ("GetAchievementSearchSize", achievement_search_count),
        ("GetAchievementGuildRep", achievement_guild_rep),
        // No rewards (the recorded achievements' reward texts are empty).
        ("GetAchievementNumRewards", zero),
        ("GetAchievementReward", nothing),
        ("C_AchievementInfo.GetRewardItemID", nil),
        ("C_AchievementInfo.GetSupercedingAchievements", empty_table),
        // No guild achievements (recorded `GetGuildCategoryList`: none).
        ("C_AchievementInfo.AreGuildAchievementsEnabled", no),
        // No comparison unit set: nothing to compare.
        ("GetComparisonAchievementPoints", zero),
        ("GetAchievementComparisonInfo", no),
        ("GetLatestCompletedComparisonAchievements", nothing),
        ("C_AchievementInfo.SetPortraitTexture", nothing),
        // Player choices: none offered (recorded
        // `GetCurrentPlayerChoiceInfo`: nothing).
        ("C_PlayerChoice.GetNumRerolls", zero),
        ("C_PlayerChoice.OnUIClosed", nothing),
        ("C_PlayerChoice.RequestRerollPlayerChoice", nothing),
        ("C_PlayerChoice.SendPlayerChoiceResponse", nothing),
        // The barber shop: not at one.
        ("C_BarberShop.GetCurrentCharacterData", nothing),
        ("C_BarberShop.GetAvailableCustomizations", nothing),
        ("C_BarberShop.HasCustomizationFeature", no),
        ("C_BarberShop.GetCurrentCost", zero),
        ("C_BarberShop.HasAlteredForm", no),
        ("C_BarberShop.GetCurrentCameraZoom", zero),
        ("C_BarberShop.GetViewingChrModel", nil),
        ("C_BarberShop.HasAnyChanges", no),
        ("C_BarberShop.IsViewingAlteredForm", no),
        ("C_BarberShop.ApplyCustomizationChoices", no),
        ("C_BarberShop.Cancel", nothing),
        ("C_BarberShop.ClearPreviewChoices", nothing),
        ("C_BarberShop.MarkCustomizationChoiceAsSeen", nothing),
        ("C_BarberShop.MarkCustomizationOptionAsSeen", nothing),
        ("C_BarberShop.PreviewCustomizationChoice", nothing),
        ("C_BarberShop.RandomizeCustomizationChoices", nothing),
        ("C_BarberShop.ResetCameraRotation", nothing),
        ("C_BarberShop.ResetCustomizationChoices", nothing),
        ("C_BarberShop.RotateCamera", nothing),
        ("C_BarberShop.SaveSeenChoices", nothing),
        ("C_BarberShop.SetCameraDistanceOffset", nothing),
        ("C_BarberShop.SetCameraZoomLevel", nothing),
        ("C_BarberShop.SetCustomizationChoice", nothing),
        ("C_BarberShop.SetModelDressState", nothing),
        ("C_BarberShop.SetSelectedSex", nothing),
        ("C_BarberShop.SetViewingAlteredForm", nothing),
        ("C_BarberShop.SetViewingChrModel", nothing),
        ("C_BarberShop.SetViewingShapeshiftForm", nothing),
        ("C_BarberShop.ZoomCamera", nothing),
        // Content tracking: nothing trackable (recorded `GetTrackedIDs`:
        // none).
        ("C_ContentTracking.IsTrackable", no),
        ("C_ContentTracking.StartTracking", nil),
        ("C_ContentTracking.StopTracking", nothing),
        ("C_ContentTracking.ToggleTracking", nil),
        ("C_ContentTracking.GetCurrentTrackingTarget", nothing),
        ("C_ContentTracking.GetEncounterTrackingInfo", nothing),
        ("C_ContentTracking.GetVendorTrackingInfo", nothing),
        ("C_ContentTracking.GetTitle", nothing),
        ("C_ContentTracking.GetObjectiveText", nothing),
        ("C_ContentTracking.GetWaypointText", nothing),
        ("C_ContentTracking.GetCollectableSourceTrackingEnabled", no),
        ("C_ContentTracking.GetBestMapForTrackable", tracking_failure),
        (
            "C_ContentTracking.GetNextWaypointForTrackable",
            tracking_failure,
        ),
        ("C_ContentTracking.IsNavigable", tracking_not_navigable),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn recorded_entries_parse() {
        let entry: &'static str = "{canDisplayOnPlayer=boolean false,categoryID=number 1,isCollected=boolean true,sourceID=number 21598,visualID=number 11257}";
        assert_eq!(super::field_num(entry, "sourceID"), Some(21598.0));
        assert!(super::field_bool(entry, "isCollected"));
        assert!(!super::field_bool(entry, "canDisplayOnPlayer"));
        let list: &'static str = "1: {1={a=number 1},10={a=number 10},2={a=number 2}}";
        let entries = super::list_entries(list);
        assert_eq!(entries.len(), 3);
        assert_eq!(super::field_num(entries[2], "a"), Some(10.0));
    }
}
