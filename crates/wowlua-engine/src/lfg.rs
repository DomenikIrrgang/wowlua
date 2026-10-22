//! The group finder (`C_LFGList`), as WoW Forever has it: the vanilla-style
//! premade group finder (`Blizzard_GroupFinder_VanillaStyle`; recorded
//! `C_LFGInfo.CanPlayerUsePremadeGroup()` true). Its game data is
//! `data/camelot-lfg.txt` (SuuUI's `/suu probe lfg` and `/suu probe
//! social`, 2026-10-14): the categories (Dungeons, Quests & Zones, Custom),
//! their activity groups and activities with their infos, the language
//! filters, and the probes' verdicts on who may search and list.
//!
//! Recorded: an addon's search works from a click, a key press (the Group
//! Finder opened with its key) or a slash command, and is blocked from a
//! timer (`ADDON_ACTION_BLOCKED(addon, "UNKNOWN()")` through `pcall`); its
//! answer, `LFG_LIST_SEARCH_RESULTS_RECEIVED()` without a payload, came a
//! second later (whole seconds recorded: the server's round trip here). An
//! addon's `UpdateListing` is blocked even from a click (the same event);
//! its `RemoveListing` from a click delists: `LFG_LIST_ACTIVE_ENTRY_UPDATE`
//! without a payload, `HasActiveEntryInfo()` false after. No listing at
//! first (`GetActiveEntryInfo()` answers nothing, `HasActiveEntryInfo()`
//! false, `GetFilteredSearchResults()` 0 and an empty list).
//!
//! The server's other listings are the engine's list (`Lfg::listings`),
//! empty: a search finds nothing until a test adds some ([`add_listing`];
//! the mock's `defineLFGListing`, `updateLFGListing`, `removeLFGListing`,
//! `lfgSearchResultID` and `failLFGSearch`).
//!
//! Recorded too (the lfg scenario: four listings posted from Blizzard's
//! listing panel, each delisted from its own entry among the browse tab's
//! results): a search finds the player's own listing (`hasSelf`, its
//! delist button shown); each search numbers its results anew, after the
//! last search's (the server's updates named results 2 to 34 after the
//! first search, 41 and 42 after the second, 69 and 70 after the fifth and
//! sixth, 82 to 107 after the seventh, 129 to 142 after the eighth, each
//! only of the latest search); a delisting tells `LFG_LIST_ACTIVE_ENTRY_
//! UPDATE(nil)`.
//!
//! Unverified: Blizzard's own (untainted) code searching and listing
//! without a hardware event (its UI searches from `LFG_LIST_ACTIVE_ENTRY_
//! UPDATE`, so the restriction is taken to be the addons'); `CreateListing`
//! blocked like `UpdateListing` (SuuUI's probe notes say both; only the
//! update was saved); a timer's `RemoveListing`; the events' timing beyond
//! "within a second"; a listing's own entry among the results (`hasSelf`);
//! the search's filters other than the category and the activities; a
//! listing expiring; where the own listing is among the results (first
//! here).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::PLAYER;
use crate::widgets::WidgetId;
use crate::{engine, engine_mut, protection, widgets};

/// A listing's time (`GetActiveEntryInfo().duration`, seconds left): the
/// recorded listing had 1794 left a few seconds after it was made, so
/// thirty minutes (unverified).
const LISTING_SECONDS: f64 = 1800.0;

/// The classes a search result's member counts name (recorded: the nine of
/// classic, each with 0 when none).
const CLASSES: [&str; 9] = [
    "DRUID", "HUNTER", "MAGE", "PALADIN", "PRIEST", "ROGUE", "SHAMAN", "WARLOCK", "WARRIOR",
];

/// `GroupFinderActivityInfo`'s fields, as `GetActivityInfoTable` has them.
const ACTIVITY_FIELDS: [&str; 26] = [
    "fullName",
    "shortName",
    "categoryID",
    "groupFinderActivityGroupID",
    "ilvlSuggestion",
    "filters",
    "minLevel",
    "minLevelSuggestion",
    "maxLevelSuggestion",
    "maxNumPlayers",
    "displayType",
    "orderIndex",
    "useHonorLevel",
    "showQuickJoinToast",
    "isMythicPlusActivity",
    "isRatedPvpActivity",
    "isCurrentRaidActivity",
    "isPvpActivity",
    "isMythicActivity",
    "allowCrossFaction",
    "isHeroicActivity",
    "isNormalActivity",
    "mapID",
    "difficultyID",
    "redirectedDifficultyID",
    "useDungeonRoleExpectations",
];

// ----------------------------------------------------------- the data

/// A category: its ID and `GetLfgCategoryInfo`'s fields.
struct Category {
    id: i64,
    info: Vec<(String, String)>,
}

/// An activity group: its category, ID and name.
struct Group {
    category: i64,
    id: i64,
    name: String,
}

/// An activity: its ID, its info's fields (recorded or estimated) and
/// which were estimated.
struct Activity {
    id: i64,
    fields: BTreeMap<&'static str, String>,
    estimated: Vec<&'static str>,
}

impl Activity {
    fn num(&self, field: &str) -> f64 {
        self.fields
            .get(field)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0)
    }
}

#[derive(Default)]
struct Catalogue {
    categories: Vec<Category>,
    groups: Vec<Group>,
    activities: Vec<Activity>,
    /// `GetAvailableActivities(category)`'s first activities, as recorded.
    ungrouped: BTreeMap<i64, Vec<i64>>,
    languages: Vec<String>,
    default_languages: Vec<String>,
}

impl Catalogue {
    fn activity(&self, id: f64) -> Option<&Activity> {
        self.activities.iter().find(|a| a.id as f64 == id)
    }
}

/// The classic instances' map IDs, for the dungeons whose full info was not
/// recorded: the six recorded (Wailing Caverns 43, Scholomance 289,
/// Ragefire Chasm 389, the Deadmines 36, Shadowfang Keep 33, Blackfathom
/// Deeps 48) are classic's, so these are taken to be too (unverified);
/// Forever's own dungeons' are unknown (0).
fn classic_map_id(activity: i64) -> Option<&'static str> {
    Some(match activity {
        803 => "90",
        804 => "47",
        805 | 827 | 828 | 829 => "189",
        806 => "129",
        807 => "70",
        808 => "209",
        809 => "349",
        810 => "109",
        811 => "230",
        812 | 837 => "229",
        813..=815 => "429",
        816 | 1603 => "329",
        _ => return None,
    })
}

/// The fields `data/camelot-lfg.txt` has only for the activities shown in
/// full, estimated for the others (`fill`).
fn fill(activities: &mut [Activity]) {
    let full: Vec<(i64, BTreeMap<&'static str, String>)> = activities
        .iter()
        .filter(|a| a.fields.contains_key("mapID"))
        .map(|a| (a.num("categoryID") as i64, a.fields.clone()))
        .collect();
    for a in activities.iter_mut() {
        if a.fields.contains_key("mapID") {
            continue;
        }
        let category = a.num("categoryID") as i64;
        let max = a.num("maxLevelSuggestion");
        let same: Vec<&BTreeMap<&'static str, String>> = full
            .iter()
            .filter(|(c, _)| *c == category)
            .map(|(_, f)| f)
            .collect();
        // The level band and the filters (their Recommended bit seems to
        // follow the player's level) of the recorded activity of the same
        // category whose top level is nearest, the lower on a tie.
        let level = |f: &BTreeMap<&'static str, String>, k: &str| -> f64 {
            f.get(k).and_then(|v| v.parse().ok()).unwrap_or(0.0)
        };
        let nearest = same.iter().copied().min_by(|x, y| {
            let dx = (level(x, "maxLevelSuggestion") - max).abs();
            let dy = (level(y, "maxLevelSuggestion") - max).abs();
            dx.total_cmp(&dy)
                .then(level(x, "maxLevelSuggestion").total_cmp(&level(y, "maxLevelSuggestion")))
        });
        let first = same.first().copied();
        for field in ACTIVITY_FIELDS {
            if a.fields.contains_key(field) {
                continue;
            }
            let value = match field {
                // Each recorded activity's short name is its full name.
                "shortName" => a.fields.get("fullName").cloned(),
                "minLevelSuggestion" => nearest.map(|n| {
                    let band = level(n, "maxLevelSuggestion") - level(n, "minLevelSuggestion");
                    format!("{}", (max - band).max(1.0).min(max))
                }),
                "filters" => nearest.and_then(|n| n.get("filters").cloned()),
                "mapID" if category == 2 => Some(classic_map_id(a.id).unwrap_or("0").to_string()),
                // The rest as the category's recorded activities have them
                // (each the same within its category).
                _ => first.and_then(|f| f.get(field).cloned()),
            };
            if let Some(v) = value {
                a.fields.insert(field, v);
                a.estimated.push(field);
            }
        }
    }
}

fn parse(text: &str) -> Catalogue {
    let mut cat = Catalogue::default();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        let pairs = |from: usize| -> Vec<(String, String)> {
            parts[from..]
                .iter()
                .filter_map(|p| p.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        match parts.first().copied() {
            Some("category") => cat.categories.push(Category {
                id: parts[1].parse().unwrap_or(0),
                info: pairs(2),
            }),
            Some("group") => cat.groups.push(Group {
                category: parts[1].parse().unwrap_or(0),
                id: parts[2].parse().unwrap_or(0),
                name: parts[3].to_string(),
            }),
            Some("activity") => {
                let mut fields = BTreeMap::new();
                for (k, v) in pairs(2) {
                    if let Some(f) = ACTIVITY_FIELDS.iter().find(|f| **f == k) {
                        fields.insert(*f, v);
                    }
                }
                cat.activities.push(Activity {
                    id: parts[1].parse().unwrap_or(0),
                    fields,
                    estimated: Vec::new(),
                });
            }
            Some("ungrouped") => {
                let ids = parts[2].split(',').filter_map(|s| s.parse().ok());
                cat.ungrouped
                    .insert(parts[1].parse().unwrap_or(0), ids.collect());
            }
            Some("languages") => {
                cat.languages = parts[1].split(',').map(String::from).collect();
            }
            Some("defaultLanguages") => {
                cat.default_languages = parts[1].split(',').map(String::from).collect();
            }
            _ => {}
        }
    }
    fill(&mut cat.activities);
    cat
}

/// The flavor's catalogue: camelot's recorded one; none for the others.
fn catalogue(vm: &Vm) -> &'static Catalogue {
    static CAMELOT: OnceLock<Catalogue> = OnceLock::new();
    static NONE: OnceLock<Catalogue> = OnceLock::new();
    if engine(vm).flavor == "camelot" {
        CAMELOT.get_or_init(|| parse(include_str!("../data/camelot-lfg.txt")))
    } else {
        NONE.get_or_init(Catalogue::default)
    }
}

/// The fields of an activity estimated, not recorded (for the tests).
pub fn estimated_fields(vm: &Vm, activity: f64) -> Vec<&'static str> {
    catalogue(vm)
        .activity(activity)
        .map(|a| a.estimated.clone())
        .unwrap_or_default()
}

// ----------------------------------------------------------- the state

/// The player's own listing.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub activity_ids: Vec<f64>,
    /// When it was made (`GetTime`).
    pub created: f64,
    /// Its description: the listing panel's comment box's text (the
    /// client's, `StoreSecureReference("LFGListingComment", editBox)`).
    pub comment: String,
    pub auto_accept: bool,
    pub private_group: bool,
    pub new_player_friendly: bool,
    pub cross_faction: bool,
    pub general_playstyle: f64,
    pub voice_mode: f64,
    pub required_item_level: f64,
}

/// A player of another group's or player's listing.
#[derive(Clone, Debug, Default)]
pub struct Member {
    pub name: String,
    pub class_name: String,
    pub class_filename: String,
    pub level: f64,
    pub area_name: String,
    /// `TANK`, `HEALER`, `DAMAGER` or `NONE`.
    pub assigned_role: String,
    pub tank: bool,
    pub healer: bool,
    pub dps: bool,
}

/// Another group's or player's listing, as the server has it; its
/// `searchResultID` is given by each search that finds it.
#[derive(Clone, Debug, Default)]
pub struct SearchResult {
    pub id: f64,
    /// The server's listing it is (`add_listing`'s handle; 0 the
    /// player's own).
    pub handle: u32,
    /// The player's own listing (`hasSelf`).
    pub has_self: bool,
    pub activity_ids: Vec<f64>,
    pub name: String,
    pub comment: String,
    pub voice_chat: String,
    pub voice_mode: f64,
    /// Seconds listed (unverified whether it grows between searches).
    pub age: f64,
    pub auto_accept: bool,
    pub delisted: bool,
    pub general_playstyle: f64,
    pub new_player_friendly: bool,
    /// The leader first.
    pub members: Vec<Member>,
}

/// What a request's answer, a round trip later, does.
#[derive(Clone, Debug)]
enum Answer {
    Search { category: f64, activities: Vec<f64> },
    Created(Listing),
    Updated(Listing),
    Removed,
}

#[derive(Default)]
pub struct Lfg {
    pub listing: Option<Listing>,
    /// The server's other listings (none unless a test adds them).
    pub listings: Vec<SearchResult>,
    /// The last search's answer: its results, numbered.
    results: Vec<SearchResult>,
    /// The last result's number (each search's after the last's).
    next_result: f64,
    /// The last listing's handle (`add_listing`'s).
    next_handle: u32,
    /// The next search fails (`fail_next_search`), with its reason.
    fail_search: Option<Option<String>>,
    /// Requests awaiting their answer, by number.
    pending: BTreeMap<u32, Answer>,
    next_request: u32,
    /// `SaveLanguageSearchFilter`'s locales (none saved: recorded `{}`).
    languages: Vec<String>,
    /// `StoreSecureReference`'s edit boxes, by name.
    secure_refs: BTreeMap<String, WidgetId>,
    /// `C_LFGListRoles.SetRoles`' tank, healer and damage.
    roles: [bool; 3],
}

fn lfg(vm: &Vm) -> &Lfg {
    &engine(vm).world.lfg
}

fn lfg_mut(vm: &mut Vm) -> &mut Lfg {
    &mut engine_mut(vm).world.lfg
}

/// Adds another group's or player's listing to the server's, for the
/// searches to find (each search numbers what it finds). Returns its
/// handle, for [`change_listing`], [`delist_listing`] and
/// [`search_result_id`].
pub fn add_listing(vm: &mut Vm, mut listing: SearchResult) -> u32 {
    let l = lfg_mut(vm);
    l.next_handle += 1;
    listing.handle = l.next_handle;
    l.listings.push(listing);
    l.next_handle
}

/// A server's listing's `searchResultID` among the last search's results;
/// none when that search did not find it (each search numbers its results
/// anew: recorded, lfg).
pub fn search_result_id(vm: &Vm, handle: u32) -> Option<f64> {
    lfg(vm)
        .results
        .iter()
        .find(|r| r.handle == handle && !r.has_self)
        .map(|r| r.id)
}

/// A server's listing changed by `change`: later searches find it so, and
/// when the last search found it, its result changes too and the server
/// tells it (`LFG_LIST_SEARCH_RESULT_UPDATED(searchResultID)`). `None` for
/// an unknown handle, else whether it was told.
pub fn change_listing(
    vm: &mut Vm,
    handle: u32,
    change: impl FnOnce(&mut SearchResult),
) -> Option<bool> {
    let l = lfg_mut(vm);
    let listing = l.listings.iter_mut().find(|r| r.handle == handle)?;
    change(listing);
    listing.handle = handle;
    let changed = listing.clone();
    let Some(r) = l
        .results
        .iter_mut()
        .find(|r| r.handle == handle && !r.has_self)
    else {
        return Some(false);
    };
    let id = r.id;
    *r = SearchResult { id, ..changed };
    Some(result_updated(vm, id))
}

/// A server's listing delisted: later searches do not find it, and when
/// the last search found it, its result is told delisted (`isDelisted`,
/// `LFG_LIST_SEARCH_RESULT_UPDATED(searchResultID)`; unverified: the
/// recording's updates do not say which were delistings). `None` for an
/// unknown handle, else whether it was told.
pub fn delist_listing(vm: &mut Vm, handle: u32) -> Option<bool> {
    let l = lfg_mut(vm);
    let at = l.listings.iter().position(|r| r.handle == handle)?;
    l.listings.remove(at);
    let Some(r) = l
        .results
        .iter_mut()
        .find(|r| r.handle == handle && !r.has_self)
    else {
        return Some(false);
    };
    r.delisted = true;
    let id = r.id;
    Some(result_updated(vm, id))
}

/// The next search's answer a failure: `LFG_LIST_SEARCH_FAILED(reason)`,
/// the results none (Blizzard's browse tab shows `LFG_LIST_SEARCH_FAILED`
/// only when it has no results: `Blizzard_LFGVanilla_Browse.lua`;
/// unverified, never recorded).
pub fn fail_next_search(vm: &mut Vm, reason: Option<String>) {
    lfg_mut(vm).fail_search = Some(reason);
}

/// The server tells that a listing among the last search's results
/// changed: `LFG_LIST_SEARCH_RESULT_UPDATED(searchResultID)`, only for a
/// result of the last search (recorded, lfg: each of the 61 updates named a
/// result of the latest search). Returns whether it was one.
pub fn result_updated(vm: &mut Vm, id: f64) -> bool {
    if !lfg(vm).results.iter().any(|r| r.id == id) {
        return false;
    }
    widgets::fire_event(vm, "LFG_LIST_SEARCH_RESULT_UPDATED", &[Value::Num(id)]);
    true
}

// ----------------------------------------------------------- helpers

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn list(vm: &mut Vm, items: Vec<Value>) -> TableRef {
    let t = vm.new_table();
    for (i, v) in items.into_iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), v);
    }
    t
}

fn num_list(vm: &mut Vm, items: &[f64]) -> Value {
    let values = items.iter().map(|n| Value::Num(*n)).collect();
    Value::Table(list(vm, values))
}

fn num_arg(c: &NativeCall, i: usize) -> Option<f64> {
    match c.arg(i) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

/// A recorded value as Lua's: booleans, numbers, else a string.
fn value(vm: &mut Vm, field: &str, v: &str) -> Value {
    match v {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ if matches!(field, "fullName" | "shortName" | "name") => vm.str(v),
        _ => match v.parse::<f64>() {
            Ok(n) => Value::Num(n),
            Err(_) => vm.str(v),
        },
    }
}

/// The numbers of a list (`{796, 799}`).
fn numbers_of(vm: &mut Vm, v: Value) -> Vec<f64> {
    let Value::Table(t) = v else {
        return Vec::new();
    };
    (1..)
        .map_while(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Num(n) => Some(n),
            _ => None,
        })
        .collect()
}

/// Whether the call runs in a hardware event: the user's click, key press
/// or typed command (`input::as_user`).
fn hardware_event(vm: &Vm) -> bool {
    engine(vm).world.input.user_actions > 0
}

/// An addon's restricted call refused: `ADDON_ACTION_BLOCKED(addon,
/// action)` (recorded through `pcall`: `"UNKNOWN()"`). Addon code is
/// refused without a hardware event when `needs_hardware`, else always;
/// Blizzard's untainted code never (unverified).
fn blocked(vm: &mut Vm, needs_hardware: bool) -> bool {
    if !protection::tainted(vm) || (needs_hardware && hardware_event(vm)) {
        return false;
    }
    let action = protection::action_name(vm);
    protection::refuse(vm, "ADDON_ACTION_BLOCKED", action);
    true
}

/// `answer` a round trip later.
fn request(vm: &mut Vm, answer: Answer) {
    let l = lfg_mut(vm);
    l.next_request += 1;
    let n = l.next_request;
    l.pending.insert(n, answer);
    let upval = Value::Num(f64::from(n));
    let native = vm.native_with("lfgAnswer", answered, vec![upval]);
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, native);
}

fn answered(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(n)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let Some(answer) = lfg_mut(vm).pending.remove(&(n as u32)) else {
        return Ok(vec![]);
    };
    match answer {
        Answer::Search {
            category,
            activities,
        } => {
            if let Some(reason) = lfg_mut(vm).fail_search.take() {
                lfg_mut(vm).results.clear();
                let reason = reason.map_or(Value::Nil, |r| vm.str(&r));
                widgets::fire_event(vm, "LFG_LIST_SEARCH_FAILED", &[reason]);
                return Ok(vec![]);
            }
            let cat = catalogue(vm);
            let wanted = |r: &SearchResult| {
                let first = r.activity_ids.first().and_then(|a| cat.activity(*a));
                first.is_some_and(|a| a.num("categoryID") == category)
                    && (activities.is_empty()
                        || r.activity_ids.iter().any(|a| activities.contains(a)))
            };
            let own = own_result(vm).filter(|r| wanted(r));
            let mut found: Vec<SearchResult> = own
                .into_iter()
                .chain(lfg(vm).listings.iter().filter(|r| wanted(r)).cloned())
                .collect();
            let l = lfg_mut(vm);
            for r in &mut found {
                l.next_result += 1.0;
                r.id = l.next_result;
            }
            l.results = found;
            widgets::fire_event(vm, "LFG_LIST_SEARCH_RESULTS_RECEIVED", &[]);
        }
        Answer::Created(listing) => {
            let described = !listing.comment.is_empty();
            lfg_mut(vm).listing = Some(listing);
            widgets::fire_event(vm, "LFG_LIST_ACTIVE_ENTRY_UPDATE", &[Value::Bool(true)]);
            // A description is checked: in the same answer the listing's
            // update and the check's verdict, not censored (recorded once,
            // lfg: a Custom listing's; the three listings without one told
            // neither; unverified).
            if described {
                widgets::fire_event(vm, "LFG_LIST_ACTIVE_ENTRY_UPDATE", &[Value::Bool(false)]);
                widgets::fire_event(
                    vm,
                    "LFG_LIST_CENSORED_ACTIVE_ENTRY_UPDATE",
                    &[Value::Bool(false)],
                );
            }
        }
        Answer::Updated(listing) => {
            if lfg(vm).listing.is_some() {
                lfg_mut(vm).listing = Some(listing);
                widgets::fire_event(vm, "LFG_LIST_ACTIVE_ENTRY_UPDATE", &[]);
            }
        }
        Answer::Removed => {
            // Its payload nil (recorded, lfg: four times; SuuUI's probe).
            if lfg_mut(vm).listing.take().is_some() {
                widgets::fire_event(vm, "LFG_LIST_ACTIVE_ENTRY_UPDATE", &[Value::Nil]);
            }
        }
    }
    Ok(vec![])
}

/// The player's own listing as a search finds it: the player its only
/// member (alone in the recording; a group's members unverified), listed
/// for as long as it has been.
fn own_result(vm: &Vm) -> Option<SearchResult> {
    let l = lfg(vm).listing.clone()?;
    let me = &engine(vm).units.list[PLAYER.0];
    let member = Member {
        name: me.name.clone(),
        class_name: me.class_name.clone(),
        class_filename: me.class.clone(),
        level: me.level,
        area_name: engine(vm).world.character.position.zone.clone(),
        assigned_role: "NONE".into(),
        ..Member::default()
    };
    Some(SearchResult {
        has_self: true,
        activity_ids: l.activity_ids.clone(),
        comment: l.comment.clone(),
        voice_mode: l.voice_mode,
        age: (crate::clock::now(vm) - l.created).floor().max(0.0),
        auto_accept: l.auto_accept,
        general_playstyle: l.general_playstyle,
        new_player_friendly: l.new_player_friendly,
        members: vec![member],
        ..SearchResult::default()
    })
}

// ----------------------------------------------------------- the catalogue

/// `C_LFGList.GetAvailableCategories([filter])`: recorded 2, 116, 120.
fn get_available_categories(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ids: Vec<f64> = catalogue(vm)
        .categories
        .iter()
        .map(|c| c.id as f64)
        .collect();
    Ok(vec![num_list(vm, &ids)])
}

/// `C_LFGList.GetLfgCategoryInfo(categoryID)`: as recorded (no
/// `searchPromptOverride`); an unknown category nothing (unverified).
fn get_lfg_category_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num_arg(&c, 1).unwrap_or(-1.0);
    let Some(cat) = catalogue(vm).categories.iter().find(|x| x.id as f64 == id) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    for (k, v) in &cat.info {
        let v = value(vm, k, v);
        set(vm, t, k, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `C_LFGList.GetAvailableActivityGroups(categoryID [, filter])`: recorded
/// (Quests & Zones: Kalimdor, then the Eastern Kingdoms; Custom none).
fn get_available_activity_groups(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num_arg(&c, 1).unwrap_or(-1.0);
    let ids: Vec<f64> = catalogue(vm)
        .groups
        .iter()
        .filter(|g| g.category as f64 == id)
        .map(|g| g.id as f64)
        .collect();
    Ok(vec![num_list(vm, &ids)])
}

/// `C_LFGList.GetActivityGroupInfo(groupID)`: its name and order (0, as
/// every recorded activity's `orderIndex`; unverified); an unknown group
/// nothing.
fn get_activity_group_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num_arg(&c, 1).unwrap_or(-1.0);
    let Some(g) = catalogue(vm).groups.iter().find(|g| g.id as f64 == id) else {
        return Ok(vec![]);
    };
    Ok(vec![vm.str(&g.name), Value::Num(0.0)])
}

/// `C_LFGList.GetAvailableActivities([categoryID [, groupID [, filter]]])`:
/// a group's activities as recorded; a category's without a group its
/// recorded first ones, then the rest in its groups' order (unverified);
/// no category, all (unverified). The filter is not applied (unverified).
fn get_available_activities(vm: &mut Vm, c: NativeCall) -> Ret {
    let cat = catalogue(vm);
    let category = num_arg(&c, 1);
    let group = num_arg(&c, 2);
    let mut ids: Vec<f64> = Vec::new();
    if let (Some(category), None) = (category, group)
        && let Some(head) = cat.ungrouped.get(&(category as i64))
    {
        ids.extend(head.iter().map(|a| *a as f64));
    }
    for a in &cat.activities {
        let id = a.id as f64;
        if category.is_some_and(|c| a.num("categoryID") != c)
            || group.is_some_and(|g| a.num("groupFinderActivityGroupID") != g)
            || ids.contains(&id)
        {
            continue;
        }
        ids.push(id);
    }
    Ok(vec![num_list(vm, &ids)])
}

/// `C_LFGList.GetActivityInfoTable(activityID [, questID [, showWarmode]])`:
/// recorded for 13 activities, the rest of the fields estimated
/// (`fill`); an unknown activity nothing.
fn get_activity_info_table(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num_arg(&c, 1).unwrap_or(-1.0);
    let Some(a) = catalogue(vm).activity(id) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    for (k, v) in &a.fields {
        let v = value(vm, k, v);
        set(vm, t, k, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `C_LFGList.GetActivityFullName(activityID)`: its info's `fullName`, the
/// ID taken whole; an activity not known still a string (the full probe,
/// 1.60.1: -1, 1.5 and 2147483647 answered a string, blanked there: here
/// empty, unverified).
fn get_activity_full_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num_arg(&c, 1).map_or(-1.0, f64::trunc);
    let name = catalogue(vm)
        .activity(id)
        .and_then(|a| a.fields.get("fullName"))
        .cloned()
        .unwrap_or_default();
    Ok(vec![vm.str(&name)])
}

/// `C_LFGList.GetAvailableLanguageSearchFilter()`: recorded, the eleven
/// locales.
fn get_available_language_search_filter(vm: &mut Vm, _c: NativeCall) -> Ret {
    let names: Vec<String> = catalogue(vm).languages.clone();
    let values = names.iter().map(|n| vm.str(n)).collect();
    Ok(vec![Value::Table(list(vm, values))])
}

fn locale_set(vm: &mut Vm, locales: &[String]) -> Ret {
    let t = vm.new_table();
    for l in locales {
        set(vm, t, l, Value::Bool(true));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_LFGList.GetDefaultLanguageSearchFilter()`: recorded `{enUS = true}`.
fn get_default_language_search_filter(vm: &mut Vm, _c: NativeCall) -> Ret {
    let locales = catalogue(vm).default_languages.clone();
    locale_set(vm, &locales)
}

/// `C_LFGList.GetLanguageSearchFilter()`: the saved locales, none at first
/// (recorded `{}`).
fn get_language_search_filter(vm: &mut Vm, _c: NativeCall) -> Ret {
    let locales = lfg(vm).languages.clone();
    locale_set(vm, &locales)
}

/// `C_LFGList.SaveLanguageSearchFilter(locales)`: kept, its true keys
/// (unverified).
fn save_language_search_filter(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut kept = Vec::new();
    if let Value::Table(t) = c.arg(1) {
        let available = catalogue(vm).languages.clone();
        for l in available {
            if !matches!(field(vm, t, &l), Value::Nil | Value::Bool(false)) {
                kept.push(l);
            }
        }
    }
    lfg_mut(vm).languages = kept;
    Ok(vec![])
}

// ----------------------------------------------------------- the listing

/// `C_LFGList.HasActiveEntryInfo()`: whether the player is listed (recorded
/// false at first, true while listed).
fn has_active_entry_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(lfg(vm).listing.is_some())])
}

/// `C_LFGList.GetActiveEntryInfo()`: nothing unlisted (recorded); listed,
/// the recorded entry's fields (no `questID`, `playstyle` or required
/// scores), its time left counting down from thirty minutes.
fn get_active_entry_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some(l) = lfg(vm).listing.clone() else {
        return Ok(vec![]);
    };
    let left = (LISTING_SECONDS - (crate::clock::now(vm) - l.created))
        .floor()
        .max(0.0);
    let t = vm.new_table();
    let ids = num_list(vm, &l.activity_ids);
    set(vm, t, "activityIDs", ids);
    set(vm, t, "autoAccept", Value::Bool(l.auto_accept));
    set(vm, t, "censored", Value::Bool(false));
    let comment = vm.str(&l.comment);
    set(vm, t, "comment", comment);
    for k in ["name", "voiceChat"] {
        let empty = vm.str("");
        set(vm, t, k, empty);
    }
    set(vm, t, "duration", Value::Num(left));
    set(vm, t, "generalPlaystyle", Value::Num(l.general_playstyle));
    set(vm, t, "isCrossFactionListing", Value::Bool(l.cross_faction));
    set(
        vm,
        t,
        "newPlayerFriendly",
        Value::Bool(l.new_player_friendly),
    );
    set(vm, t, "privateGroup", Value::Bool(l.private_group));
    set(vm, t, "requiredHonorLevel", Value::Num(0.0));
    set(
        vm,
        t,
        "requiredItemLevel",
        Value::Num(l.required_item_level),
    );
    set(vm, t, "voiceMode", Value::Num(l.voice_mode));
    Ok(vec![Value::Table(t)])
}

/// A listing from `LfgListingCreateData` (its documented defaults for the
/// fields it lacks), over `base`.
fn listing_from(vm: &mut Vm, data: Value, base: Listing) -> Listing {
    let Value::Table(t) = data else {
        return base;
    };
    let mut l = base;
    let ids = field(vm, t, "activityIDs");
    if matches!(ids, Value::Table(_)) {
        l.activity_ids = numbers_of(vm, ids);
    }
    let flag = |vm: &mut Vm, k: &str, old: bool| match field(vm, t, k) {
        Value::Nil => old,
        v => !matches!(v, Value::Bool(false)),
    };
    let number = |vm: &mut Vm, k: &str, old: f64| match field(vm, t, k) {
        Value::Num(n) => n,
        _ => old,
    };
    l.auto_accept = flag(vm, "isAutoAccept", l.auto_accept);
    l.private_group = flag(vm, "isPrivateGroup", l.private_group);
    l.new_player_friendly = flag(vm, "newPlayerFriendly", l.new_player_friendly);
    l.cross_faction = flag(vm, "isCrossFactionListing", l.cross_faction);
    l.general_playstyle = number(vm, "generalPlaystyle", l.general_playstyle);
    l.voice_mode = number(vm, "voiceMode", l.voice_mode);
    l.required_item_level = number(vm, "requiredItemLevel", l.required_item_level);
    if let Some(comment) = comment_box_text(vm) {
        l.comment = comment;
    }
    l
}

/// The listing panel's comment box's text, which the client takes as the
/// listing's description (its secure reference; Blizzard's code passes no
/// comment to `CreateListing`).
fn comment_box_text(vm: &mut Vm) -> Option<String> {
    let id = *lfg(vm).secure_refs.get("LFGListingComment")?;
    match engine(vm).world.widgets[id.0].text {
        Some(Value::Str(s)) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => Some(String::new()),
    }
}

/// `StoreSecureReference(name, object)`: the client keeps the object for
/// its own use (the listing panel's comment box, whose text is the
/// listing's description).
fn store_secure_reference(vm: &mut Vm, c: NativeCall) -> Ret {
    if let (Value::Str(name), Some(id)) = (c.arg(1), widgets::widget_of(vm, c.arg(2))) {
        let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
        lfg_mut(vm).secure_refs.insert(name, id);
    }
    Ok(vec![])
}

/// Sets the comment box's text as the client does, its `OnTextChanged`
/// (not the user's) to follow as a `SetText`'s.
fn set_comment_box_text(vm: &mut Vm, text: &str) {
    let Some(id) = lfg(vm).secure_refs.get("LFGListingComment").copied() else {
        return;
    };
    let v = vm.str(text);
    widgets::store_text(vm, id, Some(v));
    if !engine(vm).world.text_set.contains(&id) {
        engine_mut(vm).world.text_set.push(id);
    }
}

/// `C_LFGList.ClearCreationTextFields()`: the comment box emptied
/// (recorded, lfg: a dungeon listed after a described Custom listing, the
/// panel cleared between, told no description's check; unverified
/// otherwise).
fn clear_creation_text_fields(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_comment_box_text(vm, "");
    Ok(vec![])
}

/// `C_LFGList.CopyActiveEntryInfoToCreationFields()`: the listing's
/// description into the comment box (unverified).
fn copy_active_entry_info_to_creation_fields(vm: &mut Vm, _c: NativeCall) -> Ret {
    if let Some(l) = lfg(vm).listing.clone() {
        set_comment_box_text(vm, &l.comment);
    }
    Ok(vec![])
}

const ROLES: [&str; 3] = ["tank", "healer", "dps"];

/// `C_LFGListRoles.GetRoles()`: the roles `SetRoles` chose, none at first
/// (as `GetSavedRoles`; unverified).
fn get_roles(vm: &mut Vm, _c: NativeCall) -> Ret {
    let roles = lfg(vm).roles;
    let t = vm.new_table();
    for (k, on) in ROLES.iter().zip(roles) {
        set(vm, t, k, Value::Bool(on));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_LFGListRoles.SetRoles(roles [, limitToClassRoles])`: kept, true; no
/// event (recorded, lfg: Blizzard's listing panel set them at each of four
/// listings, unchanged, and no `LFG_LIST_ROLE_UPDATE` came; a change's
/// unverified).
fn set_roles(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Value::Table(t) = c.arg(1) {
        let mut roles = [false; 3];
        for (i, k) in ROLES.iter().enumerate() {
            roles[i] = field(vm, t, k).truthy();
        }
        lfg_mut(vm).roles = roles;
    }
    Ok(vec![Value::Bool(true)])
}

/// `C_LFGList.IsPlayerAuthenticatedForLFG(categoryID)`: true (recorded,
/// lfg: the Custom category's listing posted, which needs a description
/// typed into the comment box, enabled only for an authenticated
/// account; the recorded account's, unverified for others).
fn is_player_authenticated_for_lfg(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `C_LFGList.CreateListing(createData)`: blocked for addons (SuuUI's
/// probe notes; unverified, see the module's notes), false; Blizzard's
/// code lists the player a round trip later, `LFG_LIST_ACTIVE_ENTRY_
/// UPDATE(true)` (the documented `created`, which Blizzard's listing panel
/// reads; unverified), and gets true.
fn create_listing(vm: &mut Vm, c: NativeCall) -> Ret {
    if blocked(vm, false) {
        return Ok(vec![Value::Bool(false)]);
    }
    let created = crate::clock::now(vm);
    let listing = listing_from(
        vm,
        c.arg(1),
        Listing {
            created,
            ..Listing::default()
        },
    );
    request(vm, Answer::Created(listing));
    Ok(vec![Value::Bool(true)])
}

/// `C_LFGList.UpdateListing(createData)`: blocked for addons, even from a
/// click (recorded), false; Blizzard's code changes the listing a round
/// trip later, `LFG_LIST_ACTIVE_ENTRY_UPDATE()` (unverified), and gets
/// true; false unlisted (unverified).
fn update_listing(vm: &mut Vm, c: NativeCall) -> Ret {
    if blocked(vm, false) {
        return Ok(vec![Value::Bool(false)]);
    }
    let Some(base) = lfg(vm).listing.clone() else {
        return Ok(vec![Value::Bool(false)]);
    };
    let listing = listing_from(vm, c.arg(1), base);
    request(vm, Answer::Updated(listing));
    Ok(vec![Value::Bool(true)])
}

/// `C_LFGList.RemoveListing()`: delisted a round trip later,
/// `LFG_LIST_ACTIVE_ENTRY_UPDATE()` (recorded from an addon's click; from
/// a timer unverified); unlisted, nothing (unverified).
fn remove_listing(vm: &mut Vm, _c: NativeCall) -> Ret {
    if lfg(vm).listing.is_some() {
        request(vm, Answer::Removed);
    }
    Ok(vec![])
}

// ----------------------------------------------------------- searching

/// `C_LFGList.Search(categoryID, filter, preferredFilters, languageFilter,
/// searchCrossFactionListings, advancedFilter, activityIDsFilter)`: an
/// addon's needs a hardware event, else it is blocked (recorded); the
/// answer, a round trip later: the server's listings in the category (of
/// the activities asked for, if any), `LFG_LIST_SEARCH_RESULTS_RECEIVED()`.
/// Returns nothing (recorded).
fn search(vm: &mut Vm, c: NativeCall) -> Ret {
    if blocked(vm, true) {
        return Ok(vec![]);
    }
    let category = num_arg(&c, 1).unwrap_or(0.0);
    let activities = numbers_of(vm, c.arg(7));
    request(
        vm,
        Answer::Search {
            category,
            activities,
        },
    );
    Ok(vec![])
}

/// `C_LFGList.GetSearchResults()` and `GetFilteredSearchResults()`: the
/// last search's results, their count (recorded 0 and `{}` before one;
/// the filtered ones the same, unverified).
fn get_search_results(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ids: Vec<f64> = lfg(vm).results.iter().map(|r| r.id).collect();
    let t = num_list(vm, &ids);
    Ok(vec![Value::Num(ids.len() as f64), t])
}

/// `C_LFGList.ClearSearchResults()`: none (unverified: no event).
fn clear_search_results(vm: &mut Vm, _c: NativeCall) -> Ret {
    lfg_mut(vm).results.clear();
    Ok(vec![])
}

/// A result of the last search.
fn result(vm: &Vm, id: Option<f64>) -> Option<SearchResult> {
    let id = id?;
    lfg(vm).results.iter().find(|r| r.id == id).cloned()
}

/// `C_LFGList.HasSearchResultInfo(searchResultID)`.
fn has_search_result_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(result(vm, num_arg(&c, 1)).is_some())])
}

/// `C_LFGList.GetSearchResultInfo(searchResultID)`: the recorded result's
/// fields (no score, rating, faction, friend or war mode of Forever's: all
/// 0, false or empty as recorded); an unknown ID nothing.
fn get_search_result_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(r) = result(vm, num_arg(&c, 1)) else {
        return Ok(vec![]);
    };
    let t = vm.new_table();
    let ids = num_list(vm, &r.activity_ids);
    set(vm, t, "activityIDs", ids);
    set(vm, t, "age", Value::Num(r.age));
    set(vm, t, "autoAccept", Value::Bool(r.auto_accept));
    for (k, v) in [
        ("comment", &r.comment),
        ("name", &r.name),
        ("voiceChat", &r.voice_chat),
    ] {
        let s = vm.str(v);
        set(vm, t, k, s);
    }
    for k in ["censored", "crossFactionListing", "isWarMode"] {
        set(vm, t, k, Value::Bool(false));
    }
    set(vm, t, "hasSelf", Value::Bool(r.has_self));
    set(vm, t, "isDelisted", Value::Bool(r.delisted));
    set(
        vm,
        t,
        "newPlayerFriendly",
        Value::Bool(r.new_player_friendly),
    );
    set(vm, t, "generalPlaystyle", Value::Num(r.general_playstyle));
    for k in [
        "leaderFactionGroup",
        "leaderOverallDungeonScore",
        "numBNetFriends",
        "numCharFriends",
        "numGuildMates",
        "playstyle",
        "requiredDungeonScore",
        "requiredHonorLevel",
        "requiredItemLevel",
        "requiredPvpRating",
    ] {
        set(vm, t, k, Value::Num(0.0));
    }
    let score = vm.new_table();
    for k in [
        "bestLevelIncrement",
        "bestRunDurationMs",
        "bestRunLevel",
        "mapScore",
    ] {
        set(vm, score, k, Value::Num(0.0));
    }
    set(vm, score, "finishedSuccess", Value::Bool(false));
    let empty = vm.str("");
    set(vm, score, "mapName", empty);
    let scores = list(vm, vec![Value::Table(score)]);
    set(vm, t, "leaderDungeonScoreInfo", Value::Table(scores));
    let ratings = vm.new_table();
    set(vm, t, "leaderPvpRatingInfo", Value::Table(ratings));
    if let Some(leader) = r.members.first() {
        let name = vm.str(&leader.name);
        set(vm, t, "leaderName", name);
    }
    set(vm, t, "numMembers", Value::Num(r.members.len() as f64));
    // Recorded `Party-<server>-1-<hex>`; the server's number written 0000.
    let guid = vm.str(&format!("Party-0000-1-{:012X}", r.id as u64));
    set(vm, t, "partyGUID", guid);
    set(vm, t, "searchResultID", Value::Num(r.id));
    set(vm, t, "voiceMode", Value::Num(r.voice_mode));
    Ok(vec![Value::Table(t)])
}

/// `C_LFGList.GetSearchResultMemberCounts(searchResultID)`: the members by
/// role (`NOROLE` without one) and by class, each class by role, and the
/// roles still open: a dungeon's (its first activity's `useDungeonRole
/// Expectations`) a tank, a healer and the rest damage (recorded: a lone
/// damage dealer's group of five needs 1, 1 and 2), none for other
/// activities (unverified); an unknown ID nothing.
fn get_search_result_member_counts(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(r) = result(vm, num_arg(&c, 1)) else {
        return Ok(vec![]);
    };
    let role_of = |m: &Member| match m.assigned_role.as_str() {
        "TANK" | "HEALER" | "DAMAGER" => m.assigned_role.clone(),
        _ => "NOROLE".to_string(),
    };
    let count = |role: &str, class: Option<&str>| -> f64 {
        r.members
            .iter()
            .filter(|m| role.is_empty() || role_of(m) == role)
            .filter(|m| class.is_none_or(|c| m.class_filename == c))
            .count() as f64
    };
    let t = vm.new_table();
    for role in ["TANK", "HEALER", "DAMAGER", "NOROLE"] {
        set(vm, t, role, Value::Num(count(role, None)));
    }
    let mut classes: Vec<String> = CLASSES.iter().map(|c| c.to_string()).collect();
    for m in &r.members {
        if !m.class_filename.is_empty() && !classes.contains(&m.class_filename) {
            classes.push(m.class_filename.clone());
        }
    }
    for class in &classes {
        set(vm, t, class, Value::Num(count("", Some(class))));
    }
    let by_role = vm.new_table();
    for role in ["TANK", "HEALER", "DAMAGER"] {
        let inner = vm.new_table();
        for class in &classes {
            set(vm, inner, class, Value::Num(count(role, Some(class))));
        }
        set(vm, by_role, role, Value::Table(inner));
    }
    set(vm, t, "classesByRole", Value::Table(by_role));
    let first = r
        .activity_ids
        .first()
        .and_then(|a| catalogue(vm).activity(*a));
    let dungeon = first.is_some_and(|a| {
        a.fields
            .get("useDungeonRoleExpectations")
            .map(String::as_str)
            == Some("true")
    });
    let size = first.map_or(0.0, |a| a.num("maxNumPlayers"));
    for (role, wanted) in [
        ("TANK", 1.0),
        ("HEALER", 1.0),
        ("DAMAGER", (size - 2.0).max(0.0)),
    ] {
        let open = if dungeon {
            (wanted - count(role, None)).max(0.0)
        } else {
            0.0
        };
        set(vm, t, &format!("{role}_REMAINING"), Value::Num(open));
    }
    Ok(vec![Value::Table(t)])
}

/// A member as `LfgSearchResultPlayerInfo` (documented; no spec on
/// Forever, unverified).
fn player_info(vm: &mut Vm, m: &Member, leader: bool) -> Value {
    let t = vm.new_table();
    for (k, v) in [
        ("name", &m.name),
        ("className", &m.class_name),
        ("classFilename", &m.class_filename),
        ("areaName", &m.area_name),
        ("assignedRole", &m.assigned_role),
    ] {
        let s = vm.str(v);
        set(vm, t, k, s);
    }
    set(vm, t, "level", Value::Num(m.level));
    let roles = vm.new_table();
    set(vm, roles, "tank", Value::Bool(m.tank));
    set(vm, roles, "healer", Value::Bool(m.healer));
    set(vm, roles, "dps", Value::Bool(m.dps));
    set(vm, t, "lfgRoles", Value::Table(roles));
    set(vm, t, "isLeader", Value::Bool(leader));
    set(vm, t, "isLeaver", Value::Bool(false));
    Value::Table(t)
}

/// `C_LFGList.GetSearchResultLeaderInfo(searchResultID)`: its leader (the
/// first member); nothing for an unknown ID.
fn get_search_result_leader_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(m) = result(vm, num_arg(&c, 1)).and_then(|r| r.members.first().cloned()) else {
        return Ok(vec![]);
    };
    Ok(vec![player_info(vm, &m, true)])
}

/// `C_LFGList.GetSearchResultPlayerInfo(searchResultID, memberIndex)`.
fn get_search_result_player_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = num_arg(&c, 2).unwrap_or(0.0);
    let Some(r) = result(vm, num_arg(&c, 1)) else {
        return Ok(vec![]);
    };
    if index < 1.0 || index as usize > r.members.len() {
        return Ok(vec![]);
    }
    let m = r.members[index as usize - 1].clone();
    Ok(vec![player_info(vm, &m, index == 1.0)])
}

// ----------------------------------------------------------- controls

/// A string of the control's table.
fn str_field(vm: &mut Vm, t: TableRef, key: &str) -> Option<String> {
    match field(vm, t, key) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn num_field(vm: &mut Vm, t: TableRef, key: &str) -> Option<f64> {
    match field(vm, t, key) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn bool_field(vm: &mut Vm, t: TableRef, key: &str) -> Option<bool> {
    match field(vm, t, key) {
        Value::Bool(b) => Some(b),
        _ => None,
    }
}

/// A member as the controls give it: `{ name, class, className?, level?,
/// area?, role?, roles? }` (the class its file, `WARRIOR`; its name the
/// class's by default; the roles it would take its assigned one's by
/// default).
fn member_of(vm: &mut Vm, t: TableRef) -> Result<Member, String> {
    let name = str_field(vm, t, "name").ok_or("a member's name expected")?;
    let class = str_field(vm, t, "class").ok_or("a member's class expected")?;
    let class_name = str_field(vm, t, "className")
        .or_else(|| crate::units::class_name_of(&class).map(str::to_string))
        .ok_or_else(|| format!("no class {class}: its className expected"))?;
    let role = str_field(vm, t, "role").unwrap_or_else(|| "NONE".into());
    if !matches!(role.as_str(), "TANK" | "HEALER" | "DAMAGER" | "NONE") {
        return Err(format!(
            "role {role}: TANK, HEALER, DAMAGER or NONE expected"
        ));
    }
    let (mut tank, mut healer, mut dps) = (role == "TANK", role == "HEALER", role == "DAMAGER");
    if let Value::Table(r) = field(vm, t, "roles") {
        tank = bool_field(vm, r, "tank").unwrap_or(false);
        healer = bool_field(vm, r, "healer").unwrap_or(false);
        dps = bool_field(vm, r, "dps").unwrap_or(false);
    }
    Ok(Member {
        name,
        class_name,
        class_filename: class,
        level: num_field(vm, t, "level").unwrap_or(1.0),
        area_name: str_field(vm, t, "area").unwrap_or_default(),
        assigned_role: role,
        tank,
        healer,
        dps,
    })
}

/// A listing's fields the control's table gives, onto `r` (the others
/// kept).
fn apply_fields(vm: &mut Vm, t: TableRef, r: &mut SearchResult) -> Result<(), String> {
    let activities = field(vm, t, "activities");
    if !activities.is_nil() {
        let ids = numbers_of(vm, activities);
        if ids.is_empty() {
            return Err("activities: a list of activity IDs expected".into());
        }
        r.activity_ids = ids;
    }
    if let Some(v) = str_field(vm, t, "comment") {
        r.comment = v;
    }
    if let Some(v) = str_field(vm, t, "name") {
        r.name = v;
    }
    if let Some(v) = num_field(vm, t, "age") {
        r.age = v;
    }
    if let Some(v) = num_field(vm, t, "playstyle") {
        r.general_playstyle = v;
    }
    if let Some(v) = bool_field(vm, t, "newPlayerFriendly") {
        r.new_player_friendly = v;
    }
    if let Some(v) = num_field(vm, t, "voiceMode") {
        r.voice_mode = v;
    }
    if let Some(v) = bool_field(vm, t, "autoAccept") {
        r.auto_accept = v;
    }
    if let Value::Table(m) = field(vm, t, "members") {
        let mut members = Vec::new();
        for i in 1.. {
            let Value::Table(one) = vm.heap.table(m).get(Value::Num(f64::from(i))) else {
                break;
            };
            members.push(member_of(vm, one)?);
        }
        if members.is_empty() {
            return Err("members: at least the leader expected".into());
        }
        r.members = members;
    }
    Ok(())
}

fn handle_arg(vm: &mut Vm, c: &NativeCall, control: &str) -> Result<u32, wowlua_vm::LuaError> {
    match c.arg(1) {
        Value::Num(n) if n >= 1.0 && n.fract() == 0.0 => Ok(n as u32),
        _ => Err(vm.error(&format!("{control}: a listing's handle expected"))),
    }
}

/// `defineLFGListing({ activities, comment?, name?, age?, playstyle?,
/// newPlayerFriendly?, voiceMode?, autoAccept?, members })`: another
/// group's or player's listing on the server, for the searches to find;
/// its handle.
pub fn control_define(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("defineLFGListing: a table expected"));
    };
    let mut r = SearchResult::default();
    let given = apply_fields(vm, t, &mut r).and_then(|()| {
        if r.activity_ids.is_empty() {
            Err("activities expected".to_string())
        } else if r.members.is_empty() {
            Err("members expected".to_string())
        } else {
            Ok(())
        }
    });
    if let Err(e) = given {
        return Err(vm.error(&format!("defineLFGListing: {e}")));
    }
    let handle = add_listing(vm, r);
    Ok(vec![Value::Num(f64::from(handle))])
}

/// `updateLFGListing(handle, fields)`: the listing changed
/// (`change_listing`); whether the client was told.
pub fn control_update(vm: &mut Vm, c: NativeCall) -> Ret {
    let handle = handle_arg(vm, &c, "updateLFGListing")?;
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("updateLFGListing: a table of fields expected"));
    };
    let Some(mut r) = lfg(vm)
        .listings
        .iter()
        .find(|r| r.handle == handle)
        .cloned()
    else {
        return Err(vm.error(&format!("updateLFGListing: no listing {handle}")));
    };
    if let Err(e) = apply_fields(vm, t, &mut r) {
        return Err(vm.error(&format!("updateLFGListing: {e}")));
    }
    let told = change_listing(vm, handle, |l| *l = r).unwrap_or(false);
    Ok(vec![Value::Bool(told)])
}

/// `removeLFGListing(handle)`: the listing delisted (`delist_listing`);
/// whether the client was told.
pub fn control_remove(vm: &mut Vm, c: NativeCall) -> Ret {
    let handle = handle_arg(vm, &c, "removeLFGListing")?;
    match delist_listing(vm, handle) {
        Some(told) => Ok(vec![Value::Bool(told)]),
        None => Err(vm.error(&format!("removeLFGListing: no listing {handle}"))),
    }
}

/// `postLFGListing(createData)`: the player lists themselves, as from the
/// game's own listing panel (`C_LFGList.CreateListing`'s table): the
/// listing a round trip later, `LFG_LIST_ACTIVE_ENTRY_UPDATE` told by the
/// server, as Blizzard's call gets it.
pub fn control_post(vm: &mut Vm, c: NativeCall) -> Ret {
    if !matches!(c.arg(1), Value::Table(_)) {
        return Err(vm.error("postLFGListing: a table expected"));
    }
    let created = crate::clock::now(vm);
    let listing = listing_from(
        vm,
        c.arg(1),
        Listing {
            created,
            ..Listing::default()
        },
    );
    if listing.activity_ids.is_empty() {
        return Err(vm.error("postLFGListing: activityIDs expected"));
    }
    request(vm, Answer::Created(listing));
    Ok(vec![])
}

/// `delistLFG()`: the player's own listing withdrawn, as from the game's
/// Delist: a round trip later, `LFG_LIST_ACTIVE_ENTRY_UPDATE(nil)`.
/// Whether there was one.
pub fn control_delist(vm: &mut Vm, _c: NativeCall) -> Ret {
    let listed = lfg(vm).listing.is_some();
    if listed {
        request(vm, Answer::Removed);
    }
    Ok(vec![Value::Bool(listed)])
}

/// `lfgSearchResultID(handle)`: its `searchResultID` among the last
/// search's results, or nil.
pub fn control_search_result_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let handle = handle_arg(vm, &c, "lfgSearchResultID")?;
    Ok(vec![
        search_result_id(vm, handle).map_or(Value::Nil, Value::Num),
    ])
}

/// `failLFGSearch(reason?)`: the next search fails (`fail_next_search`).
pub fn control_fail_search(vm: &mut Vm, c: NativeCall) -> Ret {
    let reason = match c.arg(1) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    };
    fail_next_search(vm, reason);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_LFGList.GetAvailableCategories", get_available_categories),
        ("C_LFGList.GetLfgCategoryInfo", get_lfg_category_info),
        (
            "C_LFGList.GetAvailableActivityGroups",
            get_available_activity_groups,
        ),
        ("C_LFGList.GetActivityGroupInfo", get_activity_group_info),
        ("C_LFGList.GetAvailableActivities", get_available_activities),
        ("C_LFGList.GetActivityInfoTable", get_activity_info_table),
        ("C_LFGList.GetActivityFullName", get_activity_full_name),
        (
            "C_LFGList.GetAvailableLanguageSearchFilter",
            get_available_language_search_filter,
        ),
        (
            "C_LFGList.GetDefaultLanguageSearchFilter",
            get_default_language_search_filter,
        ),
        (
            "C_LFGList.GetLanguageSearchFilter",
            get_language_search_filter,
        ),
        (
            "C_LFGList.SaveLanguageSearchFilter",
            save_language_search_filter,
        ),
        ("StoreSecureReference", store_secure_reference),
        ("C_LFGListRoles.GetRoles", get_roles),
        ("C_LFGListRoles.SetRoles", set_roles),
        (
            "C_LFGList.ClearCreationTextFields",
            clear_creation_text_fields,
        ),
        (
            "C_LFGList.CopyActiveEntryInfoToCreationFields",
            copy_active_entry_info_to_creation_fields,
        ),
        (
            "C_LFGList.IsPlayerAuthenticatedForLFG",
            is_player_authenticated_for_lfg,
        ),
        ("C_LFGList.HasActiveEntryInfo", has_active_entry_info),
        ("C_LFGList.GetActiveEntryInfo", get_active_entry_info),
        ("C_LFGList.CreateListing", create_listing),
        ("C_LFGList.UpdateListing", update_listing),
        ("C_LFGList.RemoveListing", remove_listing),
        ("C_LFGList.Search", search),
        ("C_LFGList.GetSearchResults", get_search_results),
        ("C_LFGList.GetFilteredSearchResults", get_search_results),
        ("C_LFGList.ClearSearchResults", clear_search_results),
        ("C_LFGList.HasSearchResultInfo", has_search_result_info),
        ("C_LFGList.GetSearchResultInfo", get_search_result_info),
        (
            "C_LFGList.GetSearchResultMemberCounts",
            get_search_result_member_counts,
        ),
        (
            "C_LFGList.GetSearchResultLeaderInfo",
            get_search_result_leader_info,
        ),
        (
            "C_LFGList.GetSearchResultPlayerInfo",
            get_search_result_player_info,
        ),
    ]
}
