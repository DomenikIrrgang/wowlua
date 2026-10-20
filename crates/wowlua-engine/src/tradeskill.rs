//! Professions' windows and crafting, as the client recorded the paladin's
//! (`/stdtest engine scenario profession`, a ping near 0.25 s): the windows'
//! answers are the client's for that profession (`/stdtest engine
//! tradeskill`, `data/camelot-tradeskill.txt` by profession).
//!
//! `player:openTradeSkill(spell, profession)` (the profession's spell
//! clicked) opens it at once: `TRADE_SKILL_SHOW()` (Blizzard's UI loads its
//! frame), the spell's instant success (a client cast GUID, no `SENT`), the
//! current cast and the bar's state, `TRADE_SKILL_DATA_SOURCE_CHANGING()`
//! and the list's update; a moment later `TRADE_SKILL_DATA_SOURCE_CHANGED()`
//! and the list's update, then the list's again. The window shown again
//! with the same profession: no list update until the data's, and one
//! only. Until the data comes the recipes' lists are the window's before
//! (none the first time). `C_TradeSkillUI.CloseTradeSkill()`:
//! `TRADE_SKILL_CLOSE()` twice.
//!
//! `C_TradeSkillUI.CraftRecipe(recipe, count)`: the click's
//! `CURRENT_SPELL_CAST_CHANGED`, `TRADE_SKILL_CRAFT_BEGIN(recipe)`, the
//! bar's state; a round trip later the recipe's cast (no target named),
//! the cooldowns' events. At its success the item made (`ITEM_PUSH`, "You
//! create: …", `BAG_NEW_ITEMS_UPDATED` (stacked or not),
//! `TRADE_SKILL_ITEM_CRAFTED_RESULT`; a new slot in the reagent bag told
//! at once, read empty until the bags' packet), the cooldowns again; the
//! reagents taken and the bags' packet a moment later (the equipped
//! spells' change before each bag's first update and each of the reagent
//! bag's, its inventory's after; a skill-up after the reagents; the counts
//! of the items held before and still); the window's list after each. A
//! count left: the next `TRADE_SKILL_CRAFT_BEGIN` at once, the item a
//! moment later, the next cast a round trip after; the last of several:
//! `UPDATE_TRADESKILL_CAST_STOPPED(false)`.
//!
//! A craft asked while casting (or with the next cast asked): the server
//! refuses it a round trip later (`SENT`, `FAILED_QUIET`, "Another action
//! is in progress", the window's cast stopped if it was the craft begun
//! last), or holds it if the cast ends within the queue window
//! (`SpellQueueWindow`, 400 ms) and starts it at the success, refusing the
//! craft's own next cast. Asked within the window with one already asked
//! or held, the client holds it (its own cast GUID) and lets it go when
//! the server holds the other (`FAILED_QUIET`, the current cast cancelled)
//! or the craft goes on ("Another action is in progress"). The player
//! moving while the next cast is asked (`player:interruptCast()`): the
//! client gives it up (`INTERRUPTED`, its own GUID); the server starts it
//! anyway (no `SENT`) and stops it a round trip after the move (the
//! interrupt's events, "Interrupted", the window's cast stopped).
//!
//! `player:craftSkillUps(recipe, { … })` says which crafts raise the
//! profession (the line and its parent a point up: "Your skill in …",
//! the stats', `CONSOLE_MESSAGE`s); the windows' `skillLevel` is the
//! character's. `player:setSpellFocus({ "Forge" })`: the spell focuses near
//! the player (the recorded windows were away from any).
//!
//! Unverified: the delays (one recording each), the clicks the client
//! ignored (a request unanswered: no `CRAFT_BEGIN`; not modelled), a held
//! craft after the last cast, a reagent's stack emptied (its packet
//! differs), the recipes' colours at another rank, the made items' GUIDs
//! (made up).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::OnceLock;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::traits::Rv;
use crate::units::{self, PLAYER};
use crate::{engine, engine_mut, widgets};

/// The window's data after it opened, and the list's update after that
/// (recorded: 0.029 and 0.012 s, 0.006 and 0.005 s; unverified).
const DATA_AFTER: f64 = 0.012;
const LIST_AGAIN: f64 = 0.012;
/// A craft's bags' packet after its item (recorded: the wheel's).
const BAGS_AFTER: f64 = 0.421;
/// After a success with its item later, the bags' packet after the
/// success (recorded: 0.18-0.43 s, mostly just after the next cast's
/// start).
const BAGS_AFTER_SUCCESS: f64 = 0.38;
/// A queued craft's item after its success (recorded).
const QUEUED_ITEM_AFTER: f64 = 0.068;
/// The window's list updated after a bag's change (recorded 0.005-0.014
/// s after the item's push, after the bags' packet).
const LIST_AFTER_BAGS: f64 = 0.008;

/// The recorded answers: (PROFESSION, call) → answer.
fn data() -> &'static HashMap<(&'static str, &'static str), &'static str> {
    static DATA: OnceLock<HashMap<(&'static str, &'static str), &'static str>> = OnceLock::new();
    DATA.get_or_init(|| {
        include_str!("../data/camelot-tradeskill.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (key, rest) = l.split_once('\t')?;
                let (call, answer) = rest.split_once('\t')?;
                let (call, answer) = (
                    crate::recorded::unescape(call),
                    crate::recorded::unescape(answer),
                );
                Some(((key, call), answer))
            })
            .collect()
    })
}

/// Any profession window's answer to a call.
pub fn any_answer(call: &str) -> Option<&'static str> {
    static BY_CALL: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    BY_CALL
        .get_or_init(|| data().iter().map(|((_, c), a)| (*c, *a)).collect())
        .get(call)
        .copied()
}

fn recorded(key: &str, call: &str) -> Option<Vec<Rv>> {
    data().get(&(key, call)).map(|a| Rv::answer(a))
}

fn first(key: &str, call: &str) -> Option<Rv> {
    recorded(key, call)?.into_iter().next()
}

/// A craft asked for: the recipe, the casts left, how many were asked.
#[derive(Clone)]
pub struct Craft {
    pub recipe: i64,
    pub left: i64,
    pub count: i64,
}

/// A recording's timing of a craft's cast (`craftTimings`), in seconds:
/// its start after its request or the last cast's success, its success
/// after its start, its item after its success, the bags' packet after its
/// item, a new slot's item data after its item (else with the bags). What
/// one leaves out is the engine's.
#[derive(Clone, Copy, Default)]
pub struct Timing {
    pub start: Option<f64>,
    pub success: Option<f64>,
    pub item: Option<f64>,
    pub bags: Option<f64>,
    pub data: Option<f64>,
    /// The bags' packet split: its first changes (`split`), the rest
    /// these seconds after (`rest`).
    pub split: Option<(usize, f64)>,
}

#[derive(Default)]
pub struct TradeSkill {
    /// The window open: its profession (upper case).
    pub open: Option<String>,
    /// Its data come (`TRADE_SKILL_DATA_SOURCE_CHANGED`).
    pub ready: bool,
    /// The profession of the window shown before.
    pub last: Option<String>,
    /// The craft running.
    pub craft: Option<Craft>,
    /// The recipe the client began last, or the server started (a refusal
    /// of it stops the window's cast).
    pub current: Option<i64>,
    /// The craft's next cast, asked of the server: its recipe.
    pub next: Option<i64>,
    /// Given up meanwhile (the player moved): when.
    pub given_up: Option<f64>,
    /// A craft asked while casting, unanswered: recipe, count.
    pub asked: Option<(i64, i64)>,
    /// One the server holds until the cast ends.
    pub held: Option<(i64, i64)>,
    /// One the client holds, with its own cast GUID.
    pub queued: Option<(i64, i64, String)>,
    /// The spell focuses near the player (a forge, an anvil).
    pub focus: Vec<String>,
    /// Whether each craft to come raises the skill, by recipe.
    pub skill_ups: HashMap<i64, VecDeque<bool>>,
    /// Items made so far (their GUIDs' counter).
    pub made: u64,
    /// The recorded timings of the casts to come, and the running one's.
    pub timings: VecDeque<Timing>,
    pub timing: Option<Timing>,
    /// The bags' packet's split of the item being made (`Timing::split`).
    pub split: Option<(usize, f64)>,
    /// The windows shown this session.
    pub shown: u32,
}

fn state(vm: &Vm) -> &TradeSkill {
    &engine(vm).world.tradeskill
}

fn state_mut(vm: &mut Vm) -> &mut TradeSkill {
    &mut engine_mut(vm).world.tradeskill
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn later(vm: &mut Vm, after: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, after, native);
}

fn int(vm: &mut Vm, v: Value) -> Option<i64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n as i64),
        _ => None,
    }
}

/// `player:openTradeSkill(spell, profession)`: the profession's spell
/// clicked, its window open (the module's doc).
pub fn control_open(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    if me != PLAYER {
        return Err(vm.error("openTradeSkill: the player's"));
    }
    let spell = vm.check_number(&c, 2)?;
    let Value::Str(p) = c.arg(3) else {
        return Err(vm.error("openTradeSkill: the profession's name expected"));
    };
    let key = String::from_utf8_lossy(vm.str_bytes(p)).to_uppercase();
    if !data().keys().any(|(k, _)| *k == key) {
        return Err(vm.error(&format!("openTradeSkill: no recorded window of {key}")));
    }
    let same = state(vm).last.as_deref() == Some(key.as_str());
    {
        let s = state_mut(vm);
        s.open = Some(key);
        s.ready = false;
        s.shown += 1;
    }
    fire(vm, "TRADE_SKILL_SHOW", &[]);
    // The spell's instant success, the client's own (no `SENT`).
    let n = {
        let c = &mut engine_mut(vm).units.casting;
        c.next += 1;
        c.next
    };
    let guid = vm.str(&format!("Cast-2-0-0-0-{}-{n:010X}", spell as i64));
    units::fire_unit(
        vm,
        "UNIT_SPELLCAST_SUCCEEDED",
        PLAYER,
        &[guid, Value::Num(spell), Value::Nil],
    );
    crate::casting::cast_changed(vm, PLAYER);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    fire(vm, "TRADE_SKILL_DATA_SOURCE_CHANGING", &[]);
    if !same {
        fire(vm, "TRADE_SKILL_LIST_UPDATE", &[]);
    }
    later(
        vm,
        DATA_AFTER,
        "tradeSkillData",
        data_came,
        vec![Value::Bool(same)],
    );
    Ok(vec![])
}

fn data_came(vm: &mut Vm, c: NativeCall) -> Ret {
    let same = c.upvals.first().is_some_and(|v| v.truthy());
    let Some(key) = state(vm).open.clone() else {
        return Ok(vec![]);
    };
    {
        let s = state_mut(vm);
        s.ready = true;
        s.last = Some(key);
    }
    fire(vm, "TRADE_SKILL_DATA_SOURCE_CHANGED", &[]);
    fire(vm, "TRADE_SKILL_LIST_UPDATE", &[]);
    if !same {
        later(vm, LIST_AGAIN, "tradeSkillList", list_again, vec![]);
    }
    Ok(vec![])
}

fn list_again(vm: &mut Vm, _c: NativeCall) -> Ret {
    if state(vm).open.is_some() {
        fire(vm, "TRADE_SKILL_LIST_UPDATE", &[]);
    }
    Ok(vec![])
}

/// The recipes' lists before the window's data: the window's before.
const LISTS: &[&str] = &[
    "C_TradeSkillUI.GetAllRecipeIDs",
    "C_TradeSkillUI.GetFilteredRecipeIDs",
    "C_TradeSkillUI.GetCategories",
];

/// A window's call: the open profession's recorded answer (a list before
/// the data: the window's before, else an empty one); no window open: the
/// general answers.
fn window(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = crate::recorded::called_name(vm, &c) else {
        return Ok(vec![]);
    };
    let Some(open) = state(vm).open.clone() else {
        return not_in_window(vm, c, &name);
    };
    let calls = crate::recorded::call_variants(vm, &name, &c);
    let key = if !state(vm).ready && LISTS.contains(&name.as_str()) {
        match state(vm).last.clone() {
            Some(last) => last,
            None => {
                let empty = vm.new_table();
                return Ok(vec![Value::Table(empty)]);
            }
        }
    } else {
        open
    };
    match calls.iter().find_map(|call| recorded(&key, call)) {
        Some(values) => Ok(values
            .into_iter()
            .map(|v| {
                let v = with_rank(vm, v);
                let v = match name.as_str() {
                    "C_TradeSkillUI.GetRecipeRequirements" => with_focus(vm, v),
                    "C_TradeSkillUI.GetBaseProfessionInfo" => with_counter(vm, v),
                    _ => v,
                };
                v.to_value(vm)
            })
            .collect()),
        None => not_in_window(vm, c, &name),
    }
}

/// Every recorded window's answer to a call, where they all agree (a
/// setting or game data the window does not change).
fn agreed(call: &str) -> Option<&'static str> {
    static BY_CALL: OnceLock<HashMap<&'static str, Option<&'static str>>> = OnceLock::new();
    BY_CALL
        .get_or_init(|| {
            let mut out: HashMap<&'static str, Option<&'static str>> = HashMap::new();
            for ((_, call), answer) in data() {
                out.entry(*call)
                    .and_modify(|a| {
                        if *a != Some(*answer) {
                            *a = None;
                        }
                    })
                    .or_insert(Some(*answer));
            }
            out
        })
        .get(call)
        .copied()
        .flatten()
}

/// A call the open window did not record, or any with none open: the
/// client's general answer, else what every recorded window answered
/// alike (the full probe, no window open: the filters, the professions'
/// slots and skill lines as the windows had them; the recipe lists, the
/// inventory slot filters and readiness are the window's own, not
/// these), else a recipe's answer from its spell (`unrecorded_recipe`).
fn not_in_window(vm: &mut Vm, c: NativeCall, name: &str) -> Ret {
    if let Some(values) = crate::recorded::lookup(vm, name, &c) {
        return Ok(values);
    }
    // The recipe lists without a window: none (an empty list, the
    // categories nothing), whatever the arguments.
    match name {
        "C_TradeSkillUI.GetAllRecipeIDs" | "C_TradeSkillUI.GetFilteredRecipeIDs" => {
            return Ok(vec![Value::Table(vm.new_table())]);
        }
        "C_TradeSkillUI.GetCategories" => return Ok(vec![]),
        _ => {}
    }
    const WINDOWS_OWN: &[&str] = &[
        "C_TradeSkillUI.IsInventorySlotFiltered",
        "C_TradeSkillUI.GetFilterableInventorySlotName",
        "C_TradeSkillUI.IsTradeSkillReady",
    ];
    if !WINDOWS_OWN.contains(&name) {
        let calls = crate::recorded::call_variants(vm, name, &c);
        if let Some(answer) = calls.iter().find_map(|call| agreed(call)) {
            return Ok(crate::recorded::parse_answer(vm, answer));
        }
    }
    if let Some(values) = unrecorded(vm, name, &c) {
        return Ok(values);
    }
    crate::recorded::recorded(vm, c)
}

/// `C_TradeSkillUI.GetAllFilterableInventorySlotsCount()`: the open
/// window's; none open, 0 (the full probe, whatever the arguments; the
/// answers battery's 12 older: the newest recording wins).
fn all_filterable_slots_count(vm: &mut Vm, c: NativeCall) -> Ret {
    if state(vm).open.is_some() {
        return window(vm, c);
    }
    Ok(vec![Value::Num(0.0)])
}

/// The icon the client gives a recipe's output without its spell
/// (the full probe: recipes 0, -1, 2147483647).
const NO_OUTPUT_ICON: f64 = 136243.0;

fn set_all(vm: &mut Vm, t: wowlua_vm::TableRef, fields: Vec<(&str, Value)>) {
    for (k, v) in fields {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, v);
    }
}

/// The calls of recipes (spells) no window recorded, as the client answers
/// them (the full probe: recipe 1, the spell "Word of Recall (OLD)", and
/// IDs of no spell): their info, output and schematic from the spell's
/// name and icon, or empty. The professions' display names "Unknown" for a
/// skill line not recorded; the secondary professions' slots (cooking 5,
/// fishing 10) fixed, the others' none. Unverified: a real recipe of a
/// profession no window recorded (its reagents, skill, category).
fn unrecorded(vm: &mut Vm, name: &str, c: &NativeCall) -> Option<Vec<Value>> {
    let id = match vm.hidden(c.arg(1)) {
        Value::Num(n) => Some(n.trunc() as i64),
        _ => None,
    };
    let spell = id.and_then(|id| {
        let t = crate::gamedata::spell_data(vm, id)
            .or_else(|| crate::gamedata::scanned_spell(vm, id))?;
        let k = vm.str("name");
        let n = vm.heap.table(t).get(k);
        let k = vm.str("iconID");
        let icon = match vm.heap.table(t).get(k) {
            Value::Num(i) => i,
            _ => 0.0,
        };
        Some((n, icon))
    });
    let t = vm.new_table();
    match name {
        "C_TradeSkillUI.GetRecipeInfo" => {
            let mut fields: Vec<(&str, Value)> = [
                "alwaysUsesLowestQuality",
                "canSkillUp",
                "disabled",
                "favorite",
                "firstCraft",
                "hasSingleItemOutput",
                "isDummyRecipe",
                "isEnchantingRecipe",
                "isGatheringRecipe",
                "isRecraft",
                "isSalvageRecipe",
                "learned",
                "supportsCraftingStats",
                "supportsQualities",
            ]
            .into_iter()
            .map(|k| (k, Value::Bool(false)))
            .collect();
            for k in [
                "categoryID",
                "itemLevel",
                "maxTrivialLevel",
                "numSkillUps",
                "skillLineAbilityID",
            ] {
                fields.push((k, Value::Num(0.0)));
            }
            fields.push(("craftable", Value::Bool(true)));
            match spell {
                Some((n, icon)) => fields.extend([
                    ("canCreateMultiple", Value::Bool(false)),
                    ("name", n),
                    ("icon", Value::Num(icon)),
                    ("recipeID", Value::Num(id? as f64)),
                    ("relativeDifficulty", Value::Num(0.0)),
                ]),
                None => {
                    let empty = vm.str("");
                    fields.extend([
                        ("canCreateMultiple", Value::Bool(true)),
                        ("name", empty),
                        ("recipeID", Value::Num(0.0)),
                    ])
                }
            }
            set_all(vm, t, fields);
        }
        "C_TradeSkillUI.GetRecipeOutputItemData" => {
            let icon = spell.map_or(NO_OUTPUT_ICON, |(_, i)| i);
            set_all(vm, t, vec![("icon", Value::Num(icon))]);
        }
        "C_TradeSkillUI.GetRecipeSchematic" => {
            let slots = vm.new_table();
            let mut fields = vec![
                ("hasCraftingOperationInfo", Value::Bool(false)),
                ("reagentSlotSchematics", Value::Table(slots)),
                ("recipeType", Value::Num(1.0)),
            ];
            match spell {
                Some((n, icon)) => fields.extend([
                    ("icon", Value::Num(icon)),
                    ("isRecraft", Value::Bool(c.arg(2).truthy())),
                    ("name", n),
                    ("quantityMax", Value::Num(6.0)),
                    ("quantityMin", Value::Num(1.0)),
                    ("recipeID", Value::Num(id? as f64)),
                ]),
                None => {
                    let empty = vm.str("");
                    fields.extend([
                        ("icon", Value::Num(0.0)),
                        ("isRecraft", Value::Bool(false)),
                        ("name", empty),
                        ("quantityMax", Value::Num(0.0)),
                        ("quantityMin", Value::Num(0.0)),
                        ("recipeID", Value::Num(0.0)),
                    ])
                }
            }
            set_all(vm, t, fields);
        }
        // No list to link (the windows differ; none open: no).
        "C_TradeSkillUI.CanTradeSkillListLink" => return Some(vec![Value::Bool(false)]),
        "C_TradeSkillUI.GetTradeSkillDisplayName" => {
            id?;
            return Some(vec![vm.str("Unknown")]);
        }
        "C_TradeSkillUI.GetProfessionSlots" => {
            let slots: &[f64] = match id {
                Some(5) => &[26.0, 27.0],
                Some(10) => &[28.0, 29.0, 30.0],
                _ => &[],
            };
            for (i, s) in slots.iter().enumerate() {
                let _ = vm
                    .heap
                    .table_mut(t)
                    .set(Value::Num(i as f64 + 1.0), Value::Num(*s));
            }
        }
        _ => return None,
    }
    Some(vec![Value::Table(t)])
}

/// A recipe's requirements where the player is: a spell focus's (type 0)
/// met near one (the windows were recorded away from any).
fn with_focus(vm: &Vm, v: Rv) -> Rv {
    let Rv::Table(list) = v else {
        return v;
    };
    let focus = &state(vm).focus;
    let list = list
        .into_iter()
        .map(|(i, r)| match r {
            Rv::Table(mut fields) => {
                let named = fields.iter().find_map(|(k, v)| match v {
                    Rv::Str(s) if k == "name" => Some(s.clone()),
                    _ => None,
                });
                let spell_focus = fields
                    .iter()
                    .any(|(k, v)| k == "type" && matches!(v, Rv::Num(t) if *t == 0.0));
                if spell_focus && let Some(named) = named {
                    for (k, v) in fields.iter_mut() {
                        if k == "met" {
                            *v = Rv::Bool(focus.contains(&named));
                        }
                    }
                }
                (i, Rv::Table(fields))
            }
            other => (i, other),
        })
        .collect();
    Rv::Table(list)
}

/// `player:setSpellFocus({ "Forge", … })`: the spell focuses near the
/// player (a recipe needing one can be made).
pub fn control_spell_focus(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    if me != PLAYER {
        return Err(vm.error("setSpellFocus: the player's"));
    }
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("setSpellFocus: a list of names expected"));
    };
    let n = vm.heap.table(t).border();
    let names: Vec<String> = (1..=n)
        .filter_map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
            _ => None,
        })
        .collect();
    state_mut(vm).focus = names;
    Ok(vec![])
}

/// A profession's info at the character's rank (the windows were recorded
/// at another): `skillLevel` its line's.
fn with_rank(vm: &Vm, v: Rv) -> Rv {
    let Rv::Table(mut fields) = v else {
        return v;
    };
    let line = fields.iter().find_map(|(k, v)| match v {
        Rv::Num(n) if k == "professionID" => Some(*n as i64),
        _ => None,
    });
    if let Some(rank) = line.and_then(|l| crate::stats::skill_rank(vm, l)) {
        for (k, v) in fields.iter_mut() {
            if k == "skillLevel" {
                *v = Rv::Num(rank);
            }
        }
    }
    Rv::Table(fields)
}

/// A profession's info with the session's count of windows shown before
/// this one (`sourceCounter`; recorded: 0, 1, 2, 3 at the scenario's four
/// windows, the probes' 6 and 7 later in theirs).
fn with_counter(vm: &Vm, v: Rv) -> Rv {
    let Rv::Table(mut fields) = v else {
        return v;
    };
    let before = state(vm).shown.saturating_sub(1);
    for (k, v) in fields.iter_mut() {
        if k == "sourceCounter" {
            *v = Rv::Num(f64::from(before));
        }
    }
    Rv::Table(fields)
}

/// `C_TradeSkillUI.IsDataSourceChanging()`: from the window's
/// `TRADE_SKILL_DATA_SOURCE_CHANGING` until its data.
fn is_data_source_changing(vm: &mut Vm, _c: NativeCall) -> Ret {
    let s = state(vm);
    Ok(vec![Value::Bool(s.open.is_some() && !s.ready)])
}

/// `C_TradeSkillUI.IsRecipeInSkillLine(recipe, skillLine)`: the recipe's
/// profession is that line or its parent (its recorded
/// `GetProfessionInfoByRecipeID`: Blacksmithing 164's recipes in 2938).
fn is_recipe_in_skill_line(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(recipe), Some(line)) = (int(vm, c.arg(1)), int(vm, c.arg(2))) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let call = format!("C_TradeSkillUI.GetProfessionInfoByRecipeID({recipe})");
    let ours = data()
        .iter()
        .filter(|((_, k), _)| *k == call)
        .filter_map(|(_, a)| Rv::answer(a).into_iter().next())
        .any(|info| {
            ["professionID", "parentProfessionID"]
                .iter()
                .any(|k| matches!(info.get(k), Some(Rv::Num(n)) if *n as i64 == line))
        });
    Ok(vec![Value::Bool(ours)])
}

/// `C_TradeSkillUI.CanTradeSkillShowCraftingUI(spell)`: a profession's
/// spell of the character's (`setSpellTraits`' `profession`; unverified:
/// the client's rule); without those, the recorded character's answer.
fn can_show_crafting_ui(vm: &mut Vm, c: NativeCall) -> Ret {
    let spell = vm.hidden(c.arg(1));
    if !engine(vm).world.spell_traits.contains_key("profession")
        && let Some(values) =
            crate::recorded::lookup(vm, "C_TradeSkillUI.CanTradeSkillShowCraftingUI", &c)
    {
        return Ok(values);
    }
    Ok(vec![Value::Bool(crate::actions::has_trait(
        vm,
        spell,
        "profession",
    ))])
}

fn is_trade_skill_ready(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(state(vm).open.is_some())])
}

fn close_trade_skill(vm: &mut Vm, _c: NativeCall) -> Ret {
    if state_mut(vm).open.take().is_some() {
        fire(vm, "TRADE_SKILL_CLOSE", &[]);
        fire(vm, "TRADE_SKILL_CLOSE", &[]);
    }
    Ok(vec![])
}

/// Items by ID and count.
type Items = Vec<(i64, f64)>;

/// A recipe's schematic in the open window: its item made and how many
/// (`quantityMin`), each reagent's item and count.
fn schematic(vm: &Vm, recipe: i64) -> Option<(i64, f64, Items)> {
    let key = state(vm).open.clone().or_else(|| state(vm).last.clone())?;
    let s = first(
        &key,
        &format!("C_TradeSkillUI.GetRecipeSchematic({recipe}, false)"),
    )?;
    let output = match s.get("outputItemID") {
        Some(Rv::Num(n)) => *n as i64,
        _ => return None,
    };
    let quantity = match s.get("quantityMin") {
        Some(Rv::Num(n)) if *n >= 1.0 => *n,
        _ => 1.0,
    };
    let mut reagents = Vec::new();
    if let Some(Rv::Table(slots)) = s.get("reagentSlotSchematics") {
        for (_, slot) in slots {
            let count = match slot.get("quantityRequired") {
                Some(Rv::Num(n)) => *n,
                _ => 1.0,
            };
            if let Some(Rv::Table(list)) = slot.get("reagents")
                && let Some((_, r)) = list.first()
                && let Some(Rv::Num(id)) = r.get("itemID")
            {
                reagents.push((*id as i64, count));
            }
        }
    }
    Some((output, quantity, reagents))
}

/// `C_TradeSkillUI.GetCraftableCount(recipe)`: as many as the bags'
/// reagents make.
fn get_craftable_count(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(recipe) = int(vm, c.arg(1)) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    let Some((_, _, reagents)) = schematic(vm, recipe) else {
        return window(vm, c);
    };
    let n = reagents
        .iter()
        .map(|(id, need)| (crate::character::count_in_bags(vm, *id) / need).floor())
        .fold(f64::INFINITY, f64::min);
    Ok(vec![Value::Num(if n.is_finite() { n } else { 0.0 })])
}

/// How close to a cast's end a craft asked for is queued, not refused
/// (`SpellQueueWindow`, recorded 400 ms).
const QUEUE_WINDOW: f64 = 0.4;

fn begin(vm: &mut Vm, recipe: i64) {
    state_mut(vm).current = Some(recipe);
    fire(vm, "TRADE_SKILL_CRAFT_BEGIN", &[Value::Num(recipe as f64)]);
}

fn stopped(vm: &mut Vm) {
    fire(vm, "UPDATE_TRADESKILL_CAST_STOPPED", &[Value::Bool(false)]);
}

fn another_action(vm: &mut Vm) {
    let message = vm.str("Another action is in progress");
    fire(vm, "UI_ERROR_MESSAGE", &[Value::Num(280.0), message]);
}

/// A cast GUID of the client's own (no server's yet).
fn client_guid(vm: &mut Vm, spell: i64) -> String {
    let n = {
        let c = &mut engine_mut(vm).units.casting;
        c.next += 1;
        c.next
    };
    format!("Cast-2-0-0-0-{spell}-{n:010X}")
}

/// The end of the player's cast, if one runs.
fn cast_end(vm: &Vm) -> Option<f64> {
    engine(vm).units.casting.casts.get(&PLAYER).map(|c| c.end)
}

/// `C_TradeSkillUI.CraftRecipe(recipe, count)`: the craft asked for (the
/// module's doc). The player casting, or its next cast asked: the server
/// answers a round trip later (`craft_answer`), or near the cast's end,
/// with one already asked or held, the client holds it.
fn craft_recipe(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(recipe) = int(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let count = int(vm, c.arg(2)).unwrap_or(1).max(1);
    let end = cast_end(vm);
    let busy = end.is_some() || state(vm).next.is_some();
    fire(vm, "CURRENT_SPELL_CAST_CHANGED", &[Value::Bool(false)]);
    begin(vm, recipe);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    let delay = crate::clock::server_delay(vm);
    let (r, n) = (Value::Num(recipe as f64), Value::Num(count as f64));
    if !busy {
        let s = state_mut(vm);
        s.craft = Some(Craft {
            recipe,
            left: count,
            count,
        });
        s.next = Some(recipe);
        let start = start_after(vm);
        later(vm, start, "craftCast", craft_cast, vec![r]);
        return Ok(vec![]);
    }
    let now = crate::clock::now(vm);
    let near_end = end.is_some_and(|end| end - now <= QUEUE_WINDOW);
    let waiting = state(vm).asked.is_some() || state(vm).held.is_some();
    if near_end && waiting {
        let guid = client_guid(vm, recipe);
        state_mut(vm).queued = Some((recipe, count, guid));
    } else {
        state_mut(vm).asked = Some((recipe, count));
        later(vm, delay, "craftAnswer", craft_answer, vec![r, n]);
    }
    Ok(vec![])
}

/// The server's answer to a craft asked while casting: near the cast's
/// end it holds it (the client lets its own held one go), else refuses it
/// ("Another action is in progress"; the window's cast stopped if it was
/// the craft begun last); no cast: it starts.
fn craft_answer(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(Value::Num(recipe)), Some(Value::Num(count))) =
        (c.upvals.first().copied(), c.upvals.get(1).copied())
    else {
        return Ok(vec![]);
    };
    let (recipe, count) = (recipe as i64, count as i64);
    state_mut(vm).asked = None;
    let now = crate::clock::now(vm);
    match cast_end(vm) {
        Some(end) if end - now <= QUEUE_WINDOW => {
            state_mut(vm).held = Some((recipe, count));
            if let Some((r, _, g)) = state_mut(vm).queued.take() {
                let_go(vm, r, &g, true);
                fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
                stopped(vm);
            }
        }
        Some(_) => {
            let g = crate::casting::cast_guid(vm, 3, recipe as f64);
            let g = vm.str(&g);
            let id = Value::Num(recipe as f64);
            units::fire_unit(vm, "UNIT_SPELLCAST_SENT", PLAYER, &[Value::Nil, g, id]);
            units::fire_unit(
                vm,
                "UNIT_SPELLCAST_FAILED_QUIET",
                PLAYER,
                &[g, id, Value::Nil],
            );
            crate::casting::cast_changed(vm, PLAYER);
            fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
            another_action(vm);
            if state(vm).current == Some(recipe) {
                stopped(vm);
            }
        }
        None => {
            let s = state_mut(vm);
            s.craft = Some(Craft {
                recipe,
                left: count,
                count,
            });
            s.next = None;
            start_craft(vm, recipe, true)?;
        }
    }
    Ok(vec![])
}

/// The client lets a craft it held go: `FAILED_QUIET` with its own GUID
/// (`cancelled`: the current cast's change says so).
fn let_go(vm: &mut Vm, recipe: i64, guid: &str, cancelled: bool) {
    let g = vm.str(guid);
    units::fire_unit(
        vm,
        "UNIT_SPELLCAST_FAILED_QUIET",
        PLAYER,
        &[g, Value::Num(recipe as f64), Value::Nil],
    );
    fire(vm, "CURRENT_SPELL_CAST_CHANGED", &[Value::Bool(cancelled)]);
}

/// The recipe's spell as the window knows it: name, icon, seconds.
fn recipe_spell(vm: &mut Vm, recipe: i64) -> (Value, Value, f64) {
    let key = state(vm).open.clone().or_else(|| state(vm).last.clone());
    let info = key.and_then(|k| first(&k, &format!("C_Spell.GetSpellInfo({recipe})")));
    let name = match info.as_ref().and_then(|i| i.get("name")) {
        Some(Rv::Str(s)) => vm.str(s),
        _ => vm.str(&format!("Spell {recipe}")),
    };
    let icon = match info.as_ref().and_then(|i| i.get("iconID")) {
        Some(Rv::Num(n)) => Value::Num(*n),
        _ => Value::Num(136243.0),
    };
    let seconds = match info.as_ref().and_then(|i| i.get("castTime")) {
        Some(Rv::Num(ms)) => ms / 1000.0,
        _ => 1.5,
    };
    (name, icon, seconds)
}

/// The server casts the craft's next (no target named: its data says so);
/// given up meanwhile (`give_up_asked`), without `SENT`, and stopped a
/// round trip after the giving up.
fn craft_cast(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(recipe)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    if state(vm).next != Some(recipe as i64) {
        return Ok(vec![]);
    }
    state_mut(vm).next = None;
    let given_up = state_mut(vm).given_up.take();
    start_craft(vm, recipe as i64, given_up.is_none())?;
    if let Some(at) = given_up
        && let Some(cast) = engine(vm).units.casting.casts.get(&PLAYER)
    {
        let g = cast.guid.clone();
        let g = vm.str(&g);
        let after = (at + crate::clock::server_delay(vm) - crate::clock::now(vm)).max(0.0);
        later(vm, after, "craftInterrupted", craft_interrupted, vec![g]);
    }
    Ok(vec![])
}

/// The recipe's cast started: its `SENT` (`announce`), `START`, the
/// cooldowns' events.
fn start_craft(vm: &mut Vm, recipe: i64, announce: bool) -> Ret {
    state_mut(vm).current = Some(recipe);
    let look = recipe_spell(vm, recipe);
    let t = match crate::gamedata::spell_data(vm, recipe) {
        Some(t) => t,
        None => {
            let t = vm.new_table();
            widgets::keep(vm, Value::Table(t));
            engine_mut(vm).world.data.spells.insert(recipe, t);
            t
        }
    };
    let k = vm.str("targeted");
    let _ = vm.heap.table_mut(t).set(k, Value::Bool(false));
    let id = recipe as f64;
    let done = vm.native_with("crafted", crafted as NativeFn, vec![Value::Num(id)]);
    crate::casting::mark_craft(vm);
    take_timing(vm);
    if announce {
        crate::casting::cast_at(vm, PLAYER, id, look, false, false, None, Some(done))?;
    } else {
        crate::casting::cast_unsent(vm, id, look, Some(done))?;
    }
    crate::loot::gather_cooldowns(vm, id, false);
    Ok(vec![])
}

/// The server starts the craft it held; the craft's next cast, asked
/// meanwhile, refused.
fn held_start(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(Value::Num(recipe)), Some(Value::Num(count))) =
        (c.upvals.first().copied(), c.upvals.get(1).copied())
    else {
        return Ok(vec![]);
    };
    let (recipe, count) = (recipe as i64, count as i64);
    let refused = {
        let s = state_mut(vm);
        s.craft = Some(Craft {
            recipe,
            left: count,
            count,
        });
        s.next.take()
    };
    state_mut(vm).current = Some(recipe);
    let look = recipe_spell(vm, recipe);
    let id = recipe as f64;
    let done = vm.native_with("crafted", crafted as NativeFn, vec![Value::Num(id)]);
    crate::casting::mark_craft(vm);
    take_timing(vm);
    crate::casting::cast_at(vm, PLAYER, id, look, false, false, None, Some(done))?;
    if let Some(old) = refused {
        let g = crate::casting::cast_guid(vm, 3, old as f64);
        let g = vm.str(&g);
        let old = Value::Num(old as f64);
        units::fire_unit(vm, "UNIT_SPELLCAST_SENT", PLAYER, &[Value::Nil, g, old]);
        units::fire_unit(
            vm,
            "UNIT_SPELLCAST_FAILED_QUIET",
            PLAYER,
            &[g, old, Value::Nil],
        );
        crate::casting::cast_changed(vm, PLAYER);
    }
    cooldowns(vm, id, false, refused.is_some());
    Ok(vec![])
}

/// A craft's cooldown events (a gathering's), a refusal's message told
/// before the stance bar's (recorded twice: the tin's held craft started,
/// the copper's last success).
fn cooldowns(vm: &mut Vm, spell: f64, bags: bool, refused: bool) {
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    if bags {
        fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
    }
    let nils = [
        Value::Num(spell),
        Value::Nil,
        Value::Nil,
        Value::Nil,
        Value::Nil,
    ];
    fire(vm, "SPELL_UPDATE_COOLDOWN", &nils);
    if refused {
        another_action(vm);
    }
    fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
}

/// The player moved while the craft's next cast was asked: the client
/// gives it up (`INTERRUPTED` with its own GUID); the server's cast, if it
/// starts, is stopped a round trip later.
pub fn give_up_asked(vm: &mut Vm) {
    let Some(recipe) = state(vm).next else {
        return;
    };
    let g = client_guid(vm, recipe);
    let g = vm.str(&g);
    units::fire_unit(
        vm,
        "UNIT_SPELLCAST_INTERRUPTED",
        PLAYER,
        &[g, Value::Num(recipe as f64), Value::Nil, Value::Nil],
    );
    crate::casting::cast_changed(vm, PLAYER);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    let now = crate::clock::now(vm);
    state_mut(vm).given_up = Some(now);
}

/// The server stops the cast the client gave up: `INTERRUPTED` and `STOP`,
/// `INTERRUPTED` twice more, "Interrupted", the window's cast stopped; the
/// craft over.
fn craft_interrupted(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Str(g)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let guid = String::from_utf8_lossy(vm.str_bytes(g)).into_owned();
    let Some(cast) = engine(vm).units.casting.casts.get(&PLAYER).cloned() else {
        return Ok(vec![]);
    };
    if cast.guid != guid {
        return Ok(vec![]);
    }
    crate::casting::stop(vm, PLAYER, true);
    let id = Value::Num(cast.spell_id);
    for _ in 0..2 {
        units::fire_unit(
            vm,
            "UNIT_SPELLCAST_INTERRUPTED",
            PLAYER,
            &[Value::Str(g), id, Value::Nil, Value::Nil],
        );
        crate::casting::cast_changed(vm, PLAYER);
    }
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    let message = vm.str("Interrupted");
    fire(vm, "UI_ERROR_MESSAGE", &[Value::Num(57.0), message]);
    stopped(vm);
    state_mut(vm).craft = None;
    Ok(vec![])
}

/// The cast succeeded. Casts left: the next begun (a craft the client held
/// let go), its item a moment later, the next cast asked (after the one
/// the server held, if any). The last: the item (a moment later after
/// several, the window's cast stopped); one held starts.
fn crafted(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(recipe)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    crate::casting::cast_changed(vm, PLAYER);
    let (left, count) = {
        let s = state_mut(vm);
        match s.craft.as_mut() {
            Some(cr) if cr.recipe == recipe as i64 => {
                cr.left -= 1;
                (cr.left, cr.count)
            }
            _ => (0, 1),
        }
    };
    let held = state_mut(vm).held.take();
    let delay = crate::clock::server_delay(vm);
    let timing = state_mut(vm).timing.take().unwrap_or_default();
    state_mut(vm).split = timing.split;
    let r = Value::Num(recipe);
    let queued_bags = BAGS_AFTER_SUCCESS - QUEUED_ITEM_AFTER;
    if left > 0 {
        begin(vm, recipe as i64);
        crate::casting::cast_changed(vm, PLAYER);
        let dropped = state_mut(vm).queued.take();
        if let Some((q, _, g)) = &dropped {
            let_go(vm, *q, g, false);
        }
        // The item with the success: before the cooldowns (recorded).
        let after = item_made(vm, recipe, timing, QUEUED_ITEM_AFTER, queued_bags);
        cooldowns(vm, recipe, true, dropped.is_some());
        if dropped.is_some() {
            stopped(vm);
        }
        if let Some((after, bags)) = after {
            let data = Value::Num(timing.data.unwrap_or(-1.0));
            later(
                vm,
                after,
                "craftedItem",
                made,
                vec![r, Value::Num(bags), data],
            );
        }
        let start = start_after(vm);
        if let Some((h, n)) = held {
            let upvals = vec![Value::Num(h as f64), Value::Num(n as f64)];
            later(vm, start, "craftHeld", held_start, upvals);
        }
        state_mut(vm).next = Some(recipe as i64);
        later(vm, start, "craftCast", craft_cast, vec![r]);
        return Ok(vec![]);
    }
    state_mut(vm).craft = None;
    let later_item = if count > 1 {
        let after = item_made(vm, recipe, timing, QUEUED_ITEM_AFTER, queued_bags);
        crate::loot::gather_cooldowns(vm, recipe, true);
        stopped(vm);
        after
    } else {
        let after = item_made(vm, recipe, timing, 0.0, BAGS_AFTER);
        crate::loot::gather_cooldowns(vm, recipe, true);
        after
    };
    if let Some((after, bags)) = later_item {
        let data = Value::Num(timing.data.unwrap_or(-1.0));
        later(
            vm,
            after,
            "craftedItem",
            made,
            vec![r, Value::Num(bags), data],
        );
    }
    // Unverified: what follows the last cast, one held.
    if let Some((h, n)) = held {
        let upvals = vec![Value::Num(h as f64), Value::Num(n as f64)];
        let start = start_after(vm);
        later(vm, start, "craftHeld", held_start, upvals);
    } else if let Some((q, n, _)) = state_mut(vm).queued.take() {
        state_mut(vm).asked = Some((q, n));
        let upvals = vec![Value::Num(q as f64), Value::Num(n as f64)];
        later(vm, delay, "craftAnswer", craft_answer, upvals);
    }
    Ok(vec![])
}

fn made(vm: &mut Vm, c: NativeCall) -> Ret {
    if let (Some(Value::Num(recipe)), Some(Value::Num(bags))) =
        (c.upvals.first().copied(), c.upvals.get(1).copied())
    {
        let data = match c.upvals.get(2) {
            Some(Value::Num(d)) if *d >= 0.0 => Some(*d),
            _ => None,
        };
        make(vm, recipe as i64, bags, data);
    }
    Ok(vec![])
}

/// The cast's item: made now if it comes with the success, else when
/// (its delay and the bags' after it returned); the recording's timing,
/// else `item` and `bags`.
fn item_made(vm: &mut Vm, recipe: f64, timing: Timing, item: f64, bags: f64) -> Option<(f64, f64)> {
    let item = timing.item.unwrap_or(item);
    let bags = timing.bags.unwrap_or(bags);
    if item <= 0.0 {
        make(vm, recipe as i64, bags, timing.data);
        return None;
    }
    Some((item, bags))
}

/// The next cast's start after its request or the last success: the
/// recording's, else a round trip.
fn start_after(vm: &Vm) -> f64 {
    state(vm)
        .timings
        .front()
        .and_then(|t| t.start)
        .unwrap_or_else(|| crate::clock::server_delay(vm))
}

/// A cast starting: its recorded timing taken (its success's time).
fn take_timing(vm: &mut Vm) {
    let timing = state_mut(vm).timings.pop_front();
    if let Some(s) = timing.and_then(|t| t.success) {
        engine_mut(vm).units.casting.succeeds_after = Some(s);
    }
    state_mut(vm).timing = timing;
}

/// `player:craftTimings({ { start?, success?, item?, bags?, data? }, … })`: a
/// recording's timings of the crafts' casts to come, in order (the
/// server's answers): each cast's start after its request or the last
/// success, its success after its start, its item after its success (0:
/// with it), the bags' packet after its item, a new slot's item data after
/// its item (its own packet; else with the bags').
pub fn control_timings(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    if me != PLAYER {
        return Err(vm.error("craftTimings: the player's"));
    }
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("craftTimings: a list of timings expected"));
    };
    let n = vm.heap.table(t).border();
    for i in 1..=n {
        let Value::Table(e) = vm.heap.table(t).get(Value::Num(i as f64)) else {
            continue;
        };
        let mut get = |k: &str| {
            let key = vm.str(k);
            match vm.heap.table(e).get(key) {
                Value::Num(x) => Some(x),
                _ => None,
            }
        };
        let timing = Timing {
            start: get("start"),
            success: get("success"),
            item: get("item"),
            bags: get("bags"),
            data: get("data"),
            split: get("split").zip(get("rest")).map(|(k, r)| (k as usize, r)),
        };
        state_mut(vm).timings.push_back(timing);
    }
    Ok(vec![])
}

/// The item made: pushed and told (its new slot's bag updated); the
/// reagents taken; the bags' packet `bags_after` later (a skill-up in
/// it); the window's list after each.
fn make(vm: &mut Vm, recipe: i64, bags_after: f64, data_after: Option<f64>) {
    let Some((output, quantity, reagents)) = schematic(vm, recipe) else {
        return;
    };
    let before: BTreeMap<i64, f64> = reagents
        .iter()
        .map(|(id, _)| *id)
        .chain([output])
        .map(|id| (id, crate::character::count_in_bags(vm, id)))
        .collect();
    let placed = crate::character::push_to_bags(vm, output, quantity);
    let (bag, new_slot) = placed.unwrap_or((0, None));
    let icon = crate::character::item_field(vm, output, "icon");
    fire(
        vm,
        "ITEM_PUSH",
        &[Value::Num(crate::character::bag_inventory_slot(bag)), icon],
    );
    let link = crate::character::link_of_id(vm, output)
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        })
        .unwrap_or_default();
    let global = |vm: &mut Vm, name: &str, default: &str| match vm.get_global(name) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => default.to_string(),
    };
    let text = if quantity > 1.0 {
        global(vm, "LOOT_ITEM_CREATED_SELF_MULTIPLE", "You create: %sx%d.")
            .replacen("%s", &link, 1)
            .replacen("%d", &format!("{quantity}"), 1)
    } else {
        global(vm, "LOOT_ITEM_CREATED_SELF", "You create: %s.").replacen("%s", &link, 1)
    };
    let sender = crate::loot::player_sender(vm);
    crate::chat::game_message(vm, "CHAT_MSG_LOOT", &text, Some(&sender));
    // Made, it is new whether a slot or a stack took it (recorded: every
    // wheel and bar made).
    fire(vm, "BAG_NEW_ITEMS_UPDATED", &[]);
    let n = {
        let s = state_mut(vm);
        s.made += 1;
        s.made
    };
    let result = Rv::Table(vec![
        ("bonusCraft".into(), Rv::Bool(false)),
        ("concentrationCurrencyID".into(), Rv::Num(0.0)),
        ("concentrationSpent".into(), Rv::Num(0.0)),
        ("critBonusSkill".into(), Rv::Num(0.0)),
        ("firstCraftReward".into(), Rv::Bool(false)),
        ("hasIngenuityProc".into(), Rv::Bool(false)),
        ("hyperlink".into(), Rv::Str(link)),
        ("ingenuityRefund".into(), Rv::Num(0.0)),
        ("isCrit".into(), Rv::Bool(false)),
        ("isEnchant".into(), Rv::Bool(false)),
        (
            "itemGUID".into(),
            Rv::Str(format!("Item-4618-0-{:016X}", 0x4000_0000_8000_0000u64 + n)),
        ),
        ("itemID".into(), Rv::Num(output as f64)),
        ("multicraft".into(), Rv::Num(0.0)),
        ("operationID".into(), Rv::Num(0.0)),
        ("qualityProgress".into(), Rv::Num(0.0)),
        ("quantity".into(), Rv::Num(quantity)),
        ("recraftable".into(), Rv::Bool(false)),
    ]);
    let v = result.to_value(vm);
    fire(vm, "TRADE_SKILL_ITEM_CRAFTED_RESULT", &[v]);
    later(vm, LIST_AFTER_BAGS, "tradeSkillList", list_again, vec![]);
    // A new slot's bag told at once (recorded: the first copper and bronze
    // bars; unverified: the wheel's in the backpack, told later only).
    if new_slot.is_some() && bag == crate::character::REAGENT_BAG {
        crate::character::bag_updated(vm, bag);
        fire(vm, "BAG_UPDATE_DELAYED", &[]);
    }
    // The reagents taken; the bags changed, in order (a bag once per slot
    // changed), the item's last.
    let mut changes: Vec<i64> = Vec::new();
    let mut emptied: Vec<i64> = Vec::new();
    for (id, count) in &reagents {
        let bags = crate::character::take_from_bags(vm, *id, *count);
        // A reagent used up: its slots' removal told with the item (the
        // equipped spells', the inventory's), their bags' updates bare at
        // the bags' packet's end, after the item's (recorded once:
        // `profession` 232.767 and 233.007, the last bronze bar taking the
        // last copper and tin bars; unverified).
        if crate::character::count_in_bags(vm, *id) == 0.0 {
            for b in &bags {
                crate::character::equipped_spells_changed(vm);
                if *b == crate::character::REAGENT_BAG {
                    units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", PLAYER, &[]);
                }
            }
            emptied.extend(bags);
        } else {
            changes.extend(bags);
        }
    }
    let taken = changes.len();
    // A new slot's item data in its own packet (recorded: the first tin
    // and bronze bars'), else with the bags'.
    let data_after = data_after.filter(|_| new_slot.is_some());
    if let (Some(after), Some(slot)) = (data_after, new_slot) {
        let upvals = vec![Value::Num(bag as f64), Value::Num(slot as f64)];
        later(vm, after, "craftItemData", item_data, upvals);
    } else {
        changes.push(bag);
    }
    let new_slot = if data_after.is_some() { None } else { new_slot };
    // The used-up reagents' updates last, the skill-up after them.
    let bare = emptied.len();
    let taken = if bare > 0 {
        changes.len() + bare
    } else {
        taken
    };
    changes.extend(emptied);
    let skill_up = state_mut(vm)
        .skill_ups
        .get_mut(&recipe)
        .and_then(|q| q.pop_front())
        .unwrap_or(false);
    let mut upvals = vec![
        Value::Num(changes.len() as f64),
        Value::Num(taken as f64),
        Value::Num(if skill_up { recipe as f64 } else { 0.0 }),
        Value::Num(bag as f64),
        Value::Num(new_slot.map_or(-1.0, |s| s as f64)),
        Value::Num(bare as f64),
    ];
    let n = changes.len();
    let split = state_mut(vm).split.take();
    let first = split.map_or(n, |(k, _)| k.min(n));
    upvals.extend([
        Value::Num(0.0),
        Value::Num(first as f64),
        Value::Num(split.map_or(-1.0, |(_, r)| r)),
    ]);
    upvals.extend(changes.iter().map(|b| Value::Num(*b as f64)));
    for (id, n) in before {
        upvals.push(Value::Num(id as f64));
        upvals.push(Value::Num(n));
    }
    later(vm, bags_after, "craftBags", craft_bags, upvals);
}

/// A new slot's item data: the slot read, the equipped spells', its bag's
/// and the inventory's updates (as a loot's).
fn item_data(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(Value::Num(bag)), Some(Value::Num(slot))) = (c.upvals.first(), c.upvals.get(1))
    else {
        return Ok(vec![]);
    };
    let (bag, slot) = (*bag as i64, *slot as usize);
    crate::character::arrived(vm, bag, slot);
    crate::character::equipped_spells_changed(vm);
    crate::character::bag_updated(vm, bag);
    if bag == crate::character::REAGENT_BAG {
        units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", PLAYER, &[]);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    later(vm, LIST_AFTER_BAGS, "tradeSkillList", list_again, vec![]);
    Ok(vec![])
}

/// The craft's bags' packet: the item's new slot read, each change's bag
/// updated (the equipped
/// spells' change before each of the reagent bag's, its inventory's after,
/// before another bag's first), a skill-up after the reagents taken; the
/// counts of the items held before and changed.
fn craft_bags(vm: &mut Vm, c: NativeCall) -> Ret {
    let num = |i: usize| match c.upvals.get(i) {
        Some(Value::Num(n)) => *n,
        _ => 0.0,
    };
    let (n, taken, skill_recipe) = (num(0) as usize, num(1) as usize, num(2) as i64);
    // The changes told now (`from..to`), the rest `rest` seconds later.
    let (from, to, rest) = (num(6) as usize, num(7) as usize, num(8));
    // The item's new slot read with it.
    if from == 0 && num(4) >= 0.0 {
        crate::character::arrived(vm, num(3) as i64, num(4) as usize);
    }
    const HEAD: usize = 9;
    let bare = num(5) as usize;
    let changes: Vec<i64> = (0..n).map(|i| num(HEAD + i) as i64).collect();
    let mut skill = None;
    let mut previous = from.checked_sub(1).map(|i| changes[i]);
    for (i, bag) in changes.into_iter().enumerate().take(to).skip(from) {
        if i == taken && skill_recipe != 0 {
            skill = skill_up(vm, skill_recipe);
        }
        // A used-up reagent's: its bag's update alone (`make`).
        if i + bare >= n {
            crate::character::bag_updated(vm, bag);
            continue;
        }
        let reagent_bag = bag == crate::character::REAGENT_BAG;
        if reagent_bag || previous != Some(bag) {
            crate::character::equipped_spells_changed(vm);
        }
        previous = Some(bag);
        crate::character::bag_updated(vm, bag);
        if reagent_bag {
            units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", PLAYER, &[]);
        }
    }
    // The packet split (a recording's, `craftTimings`' `split`): the rest
    // later, the packet's end with it.
    if to < n {
        let mut upvals = c.upvals.to_vec();
        upvals[6] = Value::Num(to as f64);
        upvals[7] = Value::Num(n as f64);
        upvals[8] = Value::Num(-1.0);
        later(vm, rest, "craftBags", craft_bags, upvals);
        return Ok(vec![]);
    }
    // The item's bag told apart (its data's own packet): the skill-up
    // after the reagents still.
    if taken >= n && skill_recipe != 0 {
        skill = skill_up(vm, skill_recipe);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    for pair in c.upvals[HEAD + n..].chunks(2) {
        if let [Value::Num(id), Value::Num(before)] = pair {
            let now = crate::character::count_in_bags(vm, *id as i64);
            if now != *before && now > 0.0 && *before > 0.0 {
                fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(*id)]);
            }
        }
    }
    if let Some((lines, from, to)) = skill {
        fire(vm, "SPELL_UPDATE_USABLE", &[]);
        for line in lines {
            let text = vm.str(&format!("Skill {line} increased from {from} to {to}"));
            fire(vm, "CONSOLE_MESSAGE", &[text, Value::Num(0.0)]);
        }
    }
    later(vm, LIST_AFTER_BAGS, "tradeSkillList", list_again, vec![]);
    Ok(vec![])
}

/// The recipe's profession a point up (its recorded
/// `GetProfessionInfoByRecipeID`: the line and its parent's, the
/// character's rank): told; its lines and the ranks.
fn skill_up(vm: &mut Vm, recipe: i64) -> Option<(Vec<i64>, f64, f64)> {
    let key = state(vm).open.clone().or_else(|| state(vm).last.clone())?;
    let info = first(
        &key,
        &format!("C_TradeSkillUI.GetProfessionInfoByRecipeID({recipe})"),
    )?;
    let line = |k: &str| match info.get(k) {
        Some(Rv::Num(n)) if *n > 0.0 => Some(*n as i64),
        _ => None,
    };
    let lines: Vec<i64> = [line("parentProfessionID"), line("professionID")]
        .into_iter()
        .flatten()
        .collect();
    let from = crate::stats::skill_rank(vm, *lines.first()?)?;
    let name = match info
        .get("parentProfessionName")
        .or(info.get("professionName"))
    {
        Some(Rv::Str(s)) => s.clone(),
        _ => key.clone(),
    };
    crate::stats::raise_skill_lines(vm, &lines, from + 1.0);
    crate::units::skill_raised(vm, &name, from + 1.0, 0);
    Some((lines, from, from + 1.0))
}

/// `player:craftSkillUps(recipe, { true, false, … })`: whether each craft
/// of the recipe to come raises the profession's skill (the server's
/// chance; the script says).
pub fn control_skill_ups(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    if me != PLAYER {
        return Err(vm.error("craftSkillUps: the player's"));
    }
    let recipe = vm.check_number(&c, 2)? as i64;
    let Value::Table(t) = c.arg(3) else {
        return Err(vm.error("craftSkillUps: a list of booleans expected"));
    };
    let n = vm.heap.table(t).border();
    let ups: VecDeque<bool> = (1..=n)
        .map(|i| vm.heap.table(t).get(Value::Num(i as f64)).truthy())
        .collect();
    state_mut(vm).skill_ups.insert(recipe, ups);
    Ok(vec![])
}

/// The functions the windows were asked, and the window's own.
pub fn behaviours(flavor: &str) -> Vec<(&'static str, NativeFn)> {
    if flavor != "camelot" {
        return Vec::new();
    }
    let mut names: Vec<&'static str> = data()
        .keys()
        .filter_map(|(_, call)| call.split('(').next())
        .filter(|n| n.starts_with("C_TradeSkillUI.") || n.starts_with("C_ProfSpecs."))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|n| (n, window as NativeFn))
        .chain([
            (
                "C_TradeSkillUI.IsTradeSkillReady",
                is_trade_skill_ready as NativeFn,
            ),
            (
                "C_TradeSkillUI.IsDataSourceChanging",
                is_data_source_changing,
            ),
            (
                "C_TradeSkillUI.IsRecipeInSkillLine",
                is_recipe_in_skill_line,
            ),
            (
                "C_TradeSkillUI.CanTradeSkillShowCraftingUI",
                can_show_crafting_ui,
            ),
            ("C_TradeSkillUI.CloseTradeSkill", close_trade_skill),
            (
                "C_TradeSkillUI.GetAllFilterableInventorySlotsCount",
                all_filterable_slots_count,
            ),
            ("C_TradeSkillUI.GetCraftableCount", get_craftable_count),
            ("C_TradeSkillUI.CraftRecipe", craft_recipe),
        ])
        .collect()
}
