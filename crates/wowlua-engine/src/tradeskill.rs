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
//! `TRADE_SKILL_ITEM_CRAFTED_RESULT`), the cooldowns again; the reagents
//! taken and the bags' packet a moment later (each bag's equipped spells
//! and update, the reagent bag's inventory update; the counts of the items
//! still held). A count left: the next `TRADE_SKILL_CRAFT_BEGIN` at once,
//! the item a moment later, the next cast a round trip after.
//!
//! Unverified: the delays (one recording each), the queue beyond a count,
//! a craft asked while casting, skill-ups and the recipes' colours, the
//! made items' GUIDs (made up).

use std::collections::{BTreeMap, HashMap};
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
/// A craft's bags' packet after its success (recorded).
const BAGS_AFTER: f64 = 0.421;
/// A queued craft's item after its success (recorded).
const QUEUED_ITEM_AFTER: f64 = 0.068;

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

/// A craft asked for: the recipe, the casts left.
#[derive(Clone)]
pub struct Craft {
    pub recipe: i64,
    pub left: i64,
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
    /// Items made so far (their GUIDs' counter).
    pub made: u64,
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
        return crate::recorded::recorded(vm, c);
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
        Some(values) => Ok(values.iter().map(|v| v.to_value(vm)).collect()),
        None => crate::recorded::recorded(vm, c),
    }
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
/// the client's rule).
fn can_show_crafting_ui(vm: &mut Vm, c: NativeCall) -> Ret {
    let spell = vm.hidden(c.arg(1));
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

/// A recipe's schematic in the open window: its item made, and each
/// reagent's item and count.
fn schematic(vm: &Vm, recipe: i64) -> Option<(i64, Vec<(i64, f64)>)> {
    let key = state(vm).open.clone().or_else(|| state(vm).last.clone())?;
    let s = first(
        &key,
        &format!("C_TradeSkillUI.GetRecipeSchematic({recipe}, false)"),
    )?;
    let output = match s.get("outputItemID") {
        Some(Rv::Num(n)) => *n as i64,
        _ => return None,
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
    Some((output, reagents))
}

/// `C_TradeSkillUI.GetCraftableCount(recipe)`: as many as the bags'
/// reagents make.
fn get_craftable_count(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(recipe) = int(vm, c.arg(1)) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    let Some((_, reagents)) = schematic(vm, recipe) else {
        return window(vm, c);
    };
    let n = reagents
        .iter()
        .map(|(id, need)| (crate::character::count_in_bags(vm, *id) / need).floor())
        .fold(f64::INFINITY, f64::min);
    Ok(vec![Value::Num(if n.is_finite() { n } else { 0.0 })])
}

/// `C_TradeSkillUI.CraftRecipe(recipe, count)`: the craft asked for.
fn craft_recipe(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(recipe) = int(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let count = int(vm, c.arg(2)).unwrap_or(1).max(1);
    state_mut(vm).craft = Some(Craft {
        recipe,
        left: count,
    });
    fire(vm, "CURRENT_SPELL_CAST_CHANGED", &[Value::Bool(false)]);
    fire(vm, "TRADE_SKILL_CRAFT_BEGIN", &[Value::Num(recipe as f64)]);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    let delay = crate::clock::server_delay(vm);
    later(
        vm,
        delay,
        "craftCast",
        craft_cast,
        vec![Value::Num(recipe as f64)],
    );
    Ok(vec![])
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

/// The server casts the recipe (no target named: its data says so).
fn craft_cast(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(recipe)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let look = recipe_spell(vm, recipe as i64);
    let t = match crate::gamedata::spell_data(vm, recipe as i64) {
        Some(t) => t,
        None => {
            let t = vm.new_table();
            widgets::keep(vm, Value::Table(t));
            engine_mut(vm).world.data.spells.insert(recipe as i64, t);
            t
        }
    };
    let k = vm.str("targeted");
    let _ = vm.heap.table_mut(t).set(k, Value::Bool(false));
    let done = vm.native_with("crafted", crafted as NativeFn, vec![Value::Num(recipe)]);
    crate::casting::cast_at(vm, PLAYER, recipe, look, false, false, None, Some(done))?;
    crate::loot::gather_cooldowns(vm, recipe, false);
    Ok(vec![])
}

/// The cast succeeded: the item made (at once, or a moment later when
/// another cast follows), the next one asked.
fn crafted(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(recipe)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    crate::casting::cast_changed(vm, PLAYER);
    let left = {
        let s = state_mut(vm);
        match s.craft.as_mut() {
            Some(cr) if cr.recipe == recipe as i64 => {
                cr.left -= 1;
                cr.left
            }
            _ => 0,
        }
    };
    if left > 0 {
        fire(vm, "TRADE_SKILL_CRAFT_BEGIN", &[Value::Num(recipe)]);
        crate::casting::cast_changed(vm, PLAYER);
        let bag_cd = |vm: &mut Vm| {
            fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
            let nils = [
                Value::Num(recipe),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ];
            fire(vm, "SPELL_UPDATE_COOLDOWN", &nils);
        };
        bag_cd(vm);
        later(
            vm,
            QUEUED_ITEM_AFTER,
            "craftedItem",
            made,
            vec![Value::Num(recipe)],
        );
        let delay = crate::clock::server_delay(vm);
        later(vm, delay, "craftCast", craft_cast, vec![Value::Num(recipe)]);
    } else {
        state_mut(vm).craft = None;
        make(vm, recipe as i64);
        crate::loot::gather_cooldowns(vm, recipe, true);
    }
    Ok(vec![])
}

fn made(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(Value::Num(recipe)) = c.upvals.first().copied() {
        make(vm, recipe as i64);
    }
    Ok(vec![])
}

/// The item made: pushed and told; the reagents taken; the bags' packet
/// later.
fn make(vm: &mut Vm, recipe: i64) {
    let Some((output, reagents)) = schematic(vm, recipe) else {
        return;
    };
    let before: BTreeMap<i64, f64> = reagents
        .iter()
        .map(|(id, _)| *id)
        .chain([output])
        .map(|id| (id, crate::character::count_in_bags(vm, id)))
        .collect();
    let placed = crate::character::push_to_bags(vm, output, 1.0);
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
    let format = match vm.get_global("LOOT_ITEM_CREATED_SELF") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "You create: %s.".to_string(),
    };
    let text = format.replacen("%s", &link, 1);
    let sender = crate::loot::player_sender(vm);
    crate::chat::game_message(vm, "CHAT_MSG_LOOT", &text, Some(&sender));
    // Made, it is new whether a slot or a stack took it (recorded: every
    // wheel and bar made).
    let _ = new_slot;
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
        ("quantity".into(), Rv::Num(1.0)),
        ("recraftable".into(), Rv::Bool(false)),
    ]);
    let v = result.to_value(vm);
    fire(vm, "TRADE_SKILL_ITEM_CRAFTED_RESULT", &[v]);
    // The reagents taken; the bags changed, in order (a bag once per slot
    // changed).
    let mut changes: Vec<i64> = Vec::new();
    for (id, count) in &reagents {
        changes.extend(crate::character::take_from_bags(vm, *id, *count));
    }
    changes.push(bag);
    let mut upvals: Vec<Value> = changes.iter().map(|b| Value::Num(*b as f64)).collect();
    upvals.insert(0, Value::Num(changes.len() as f64));
    for (id, n) in before {
        upvals.push(Value::Num(id as f64));
        upvals.push(Value::Num(n));
    }
    later(vm, BAGS_AFTER, "craftBags", craft_bags, upvals);
}

/// The craft's bags' packet: each bag's equipped spells and its updates
/// (the reagent bag's inventory update), the counts of the items changed
/// and still held.
fn craft_bags(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(n)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let n = n as usize;
    let changes: Vec<i64> = c.upvals[1..=n]
        .iter()
        .filter_map(|v| match v {
            Value::Num(b) => Some(*b as i64),
            _ => None,
        })
        .collect();
    // Each bag in its first change's order, as often as it changed.
    let mut bags: Vec<(i64, usize)> = Vec::new();
    for b in changes {
        match bags.iter_mut().find(|(x, _)| *x == b) {
            Some((_, k)) => *k += 1,
            None => bags.push((b, 1)),
        }
    }
    // The reagent bag first (recorded).
    bags.sort_by_key(|(b, _)| *b != crate::character::REAGENT_BAG);
    for (bag, times) in bags {
        crate::character::equipped_spells_changed(vm);
        for _ in 0..times {
            crate::character::bag_updated(vm, bag);
            if bag == crate::character::REAGENT_BAG {
                units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", PLAYER, &[]);
            }
        }
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    let counts = &c.upvals[1 + n..];
    for pair in counts.chunks(2) {
        if let [Value::Num(id), Value::Num(before)] = pair {
            let now = crate::character::count_in_bags(vm, *id as i64);
            if now != *before && now > 0.0 {
                fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(*id)]);
            }
        }
    }
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
            ("C_TradeSkillUI.GetCraftableCount", get_craftable_count),
            ("C_TradeSkillUI.CraftRecipe", craft_recipe),
        ])
        .collect()
}
