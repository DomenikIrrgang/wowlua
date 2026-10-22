//! The cursor: what the player picked up (a spell, a macro, an item, an
//! action) until it is put down or cleared, as `/stdtest engine cursor`
//! recorded on Forever (`tests/client/forever-camelot-cursor.lua`):
//!
//! - picking up: `ACTIONBAR_SHOWGRID`, `CURSOR_CHANGED(false, type, 0, 0)`,
//!   then the source's event (`ACTIONBAR_SLOT_CHANGED(slot)` for an action,
//!   `ITEM_LOCK_CHANGED(bag, slot)` or `(slot, nil)` for an item, which is
//!   locked while held);
//! - putting down (an item back where it was, an action placed) and
//!   clearing: the item's `ITEM_LOCK_CHANGED` first, then
//!   `ACTIONBAR_HIDEGRID`, `CURSOR_CHANGED(true, 0, old type, 0)`, then a
//!   placed action's `ACTIONBAR_SLOT_CHANGED`;
//! - `GetCursorInfo`: `spell, spellbook slot, "spell", spellID`; `macro,
//!   index`; `item, itemID, link`.
//!
//! An item put into another (empty) bag slot: the cursor let go at once,
//! the item still locked; the server's answer a round trip later moves it
//! (`/stdtest engine scenario bank`): for each bag, the bank's first, the
//! spells' update, the lock gone where it arrived, the bag's update; the
//! count's change when it crossed into or out of the bags. A bag item used
//! at the bank goes into the bank (or out of it) the same way, locked at
//! once. Trade goods do not show the action bars' grid.
//!
//! A spell or a macro picked up or put down also tells whether the click
//! bindings' highlights show (`CLICKBINDINGS_SET_HIGHLIGHTS_SHOWN`, true
//! after the grid shows, false before it hides); an item does not
//! (recorded: scenarios actionbar, six spells; macro, two; equip and items,
//! no item). An action placed on a filled slot swaps: the cursor let go,
//! the slot's old action picked up, then the slot's change; the action
//! bars' cooldowns updated after each slot's change (recorded: scenario
//! actionbar, twice, and every pickup and placement there and in macro).
//!
//! Unverified: picking up while holding something else, moving within one
//! bag, money.

use std::collections::BTreeMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::actions::Action;
use crate::character::Held;
use crate::{engine, engine_mut, widgets};

/// Where a held item came from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Bag(i64, usize),
    Equipped(i64),
}

#[derive(Clone, Debug, Default)]
pub enum Cursor {
    #[default]
    Empty,
    Spell(f64),
    Macro(f64),
    Item(Held, Option<Source>),
}

impl Cursor {
    /// `Enum.UICursorType`: none 0, item 1, spell 3, macro 7.
    fn kind(&self) -> f64 {
        match self {
            Cursor::Empty => 0.0,
            Cursor::Item(..) => 1.0,
            Cursor::Spell(_) => 3.0,
            Cursor::Macro(_) => 7.0,
        }
    }
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn fire(vm: &mut Vm, event: &str, args: &[Value]) {
    widgets::fire_event(vm, event, args);
}

/// Whether holding it shows the click bindings' highlights: a spell, a
/// macro (see the module).
fn shows_highlights(cursor: &Cursor) -> bool {
    matches!(cursor, Cursor::Spell(_) | Cursor::Macro(_))
}

/// Whether holding it shows the action bars' grid: a spell, a macro, an
/// item but a trade good or a quest item (recorded: Copper Ore and Rough
/// Stone without the grid, the Hearthstone, a weapon and a Mining Pick
/// with it; Blood Shards, a quest item, split off their stack without it,
/// `bags`: unverified whether for the class or the split; other classes
/// unverified).
fn shows_grid(vm: &mut Vm, cursor: &Cursor) -> bool {
    match cursor {
        Cursor::Item(held, _) => {
            // `Enum.ItemClass.Tradegoods`, `Enum.ItemClass.Questitem`.
            !matches!(
                crate::character::item_field(vm, held.id, "classID"),
                Value::Num(7.0 | 12.0)
            )
        }
        Cursor::Empty => false,
        _ => true,
    }
}

/// Holds `new` (from empty): the grid shows, the cursor changes.
fn pick_up(vm: &mut Vm, new: Cursor) {
    let kind = new.kind();
    let grid = shows_grid(vm, &new);
    engine_mut(vm).world.character.cursor = new;
    if grid {
        fire(vm, "ACTIONBAR_SHOWGRID", &[]);
    }
    if shows_highlights(&engine(vm).world.character.cursor) {
        fire(
            vm,
            "CLICKBINDINGS_SET_HIGHLIGHTS_SHOWN",
            &[Value::Bool(true)],
        );
    }
    fire(
        vm,
        "CURSOR_CHANGED",
        &[
            Value::Bool(false),
            Value::Num(kind),
            Value::Num(0.0),
            Value::Num(0.0),
        ],
    );
}

/// Holds nothing: a held item unlocked (`ITEM_LOCK_CHANGED`), the grid
/// hides, the cursor changes.
fn put_down(vm: &mut Vm) {
    let old = std::mem::take(&mut engine_mut(vm).world.character.cursor);
    if matches!(old, Cursor::Empty) {
        return;
    }
    if let Cursor::Item(_, Some(source)) = &old {
        set_locked(vm, source, false);
    }
    if shows_highlights(&old) {
        fire(
            vm,
            "CLICKBINDINGS_SET_HIGHLIGHTS_SHOWN",
            &[Value::Bool(false)],
        );
    }
    if shows_grid(vm, &old) {
        fire(vm, "ACTIONBAR_HIDEGRID", &[]);
    }
    fire(
        vm,
        "CURSOR_CHANGED",
        &[
            Value::Bool(true),
            Value::Num(0.0),
            Value::Num(old.kind()),
            Value::Num(0.0),
        ],
    );
}

/// Locks or unlocks the item at `source`, telling `ITEM_LOCK_CHANGED`.
fn set_locked(vm: &mut Vm, source: &Source, locked: bool) {
    let ch = &mut engine_mut(vm).world.character;
    let held = match source {
        Source::Bag(bag, slot) => ch.bags.get_mut(bag).and_then(|b| b.items.get_mut(slot)),
        Source::Equipped(slot) => ch.equipment.get_mut(slot),
    };
    if let Some(h) = held {
        h.locked = locked;
    }
    let args = match source {
        Source::Bag(bag, slot) => [Value::Num(*bag as f64), Value::Num(*slot as f64)],
        Source::Equipped(slot) => [Value::Num(*slot as f64), Value::Nil],
    };
    fire(vm, "ITEM_LOCK_CHANGED", &args);
    // Then which (recorded, scenario items).
    let which = if locked {
        "ITEM_LOCKED"
    } else {
        "ITEM_UNLOCKED"
    };
    fire(vm, which, &args);
}

fn clear_cursor(vm: &mut Vm, _c: NativeCall) -> Ret {
    put_down(vm);
    Ok(vec![])
}

/// `C_Spell.PickupSpell(spellID)`.
fn pickup_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = num(vm.hidden(c.arg(1))) else {
        return Ok(vec![]);
    };
    put_down(vm);
    pick_up(vm, Cursor::Spell(id));
    Ok(vec![])
}

/// `C_SpellBook.PickupSpellBookItem(slot, bank)`: the slot's spell; for
/// an addon in combat, blocked (decision T253; Blizzard's own code, the
/// secure drag handlers among it, may).
fn pickup_spellbook_item(vm: &mut Vm, c: NativeCall) -> Ret {
    if crate::protection::refuses_in_combat(vm) {
        return Ok(vec![]);
    }
    let Some(slot) = num(c.arg(1)) else {
        return Ok(vec![]);
    };
    let spell = spellbook_spell(vm, slot as i64);
    if let Some(id) = spell {
        put_down(vm);
        pick_up(vm, Cursor::Spell(id));
    }
    Ok(vec![])
}

/// The spell ID of the player's spellbook slot (its recorded item).
fn spellbook_spell(vm: &mut Vm, slot: i64) -> Option<f64> {
    let item = crate::stats::spellbook_item_at(vm, slot)?;
    let Value::Table(t) = item else {
        return None;
    };
    let k = vm.str("spellID");
    num(vm.heap.table(t).get(k))
}

fn pickup_macro(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(index) = num(c.arg(1)) else {
        return Ok(vec![]);
    };
    put_down(vm);
    pick_up(vm, Cursor::Macro(index));
    Ok(vec![])
}

/// `PickupAction(slot)`: the action onto the cursor, the slot emptied.
/// With something on the cursor, it goes into the slot and the slot's
/// action (if any) onto the cursor, as `PlaceAction` (unverified:
/// warcraft.wiki.gg's API_PickupAction, "the slot is filled with whatever
/// action was currently being drag-and-dropped"; LibActionButton's
/// buttons take a drop onto an empty slot so, through Blizzard's
/// `PickupAny`).
fn pickup_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(slot) = num(c.arg(1)).map(|s| s as i64) else {
        return Ok(vec![]);
    };
    if !matches!(engine(vm).world.character.cursor, Cursor::Empty) {
        return place_action(vm, c);
    }
    let Some(action) = engine_mut(vm).world.character.actions.remove(&slot) else {
        return Ok(vec![]);
    };
    let held = held_action(&action);
    put_down(vm);
    pick_up(vm, held);
    fire(vm, "ACTIONBAR_SLOT_CHANGED", &[Value::Num(slot as f64)]);
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    Ok(vec![])
}

/// What an action is on the cursor.
fn held_action(action: &Action) -> Cursor {
    let id = num(action.id).unwrap_or(0.0);
    match action.kind.as_str() {
        "macro" => Cursor::Macro(id),
        "item" => Cursor::Item(
            Held {
                id: id as i64,
                count: 1.0,
                ..Held::default()
            },
            None,
        ),
        _ => Cursor::Spell(id),
    }
}

/// `PlaceAction(slot)`: what the cursor holds becomes the slot's action.
fn place_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(slot) = num(c.arg(1)).map(|s| s as i64) else {
        return Ok(vec![]);
    };
    let action = match engine(vm).world.character.cursor.clone() {
        Cursor::Empty => return Ok(vec![]),
        Cursor::Spell(id) => {
            let texture = crate::stats::spellbook_item(vm, id)
                .and_then(|item| match item {
                    Value::Table(t) => {
                        let k = vm.str("iconID");
                        Some(vm.heap.table(t).get(k))
                    }
                    _ => None,
                })
                .unwrap_or(Value::Nil);
            Action {
                kind: "spell".into(),
                id: Value::Num(id),
                sub_type: vm.str("spell"),
                texture,
                usable: (true, false),
                ..Action::default()
            }
        }
        Cursor::Macro(index) => Action {
            kind: "macro".into(),
            id: Value::Num(index),
            usable: (true, false),
            ..Action::default()
        },
        Cursor::Item(held, _) => Action {
            kind: "item".into(),
            id: Value::Num(held.id as f64),
            texture: crate::character::item_field(vm, held.id, "icon"),
            usable: (true, false),
            ..Action::default()
        },
    };
    for v in [action.id, action.sub_type, action.texture] {
        widgets::keep(vm, v);
    }
    put_down(vm);
    let old = engine_mut(vm).world.character.actions.insert(slot, action);
    // A filled slot's action onto the cursor (see the module).
    if let Some(old) = old {
        pick_up(vm, held_action(&old));
    }
    fire(vm, "ACTIONBAR_SLOT_CHANGED", &[Value::Num(slot as f64)]);
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    Ok(vec![])
}

/// Takes the item at `source` onto the cursor, or puts the held item back
/// when it came from there; another slot: moves it (unverified).
pub(crate) fn pickup_item_at(vm: &mut Vm, source: Source) {
    match engine(vm).world.character.cursor.clone() {
        Cursor::Item(_, Some(from)) if from == source => {
            put_down(vm);
        }
        Cursor::Item(_, Some(from))
            if matches!(from, Source::Equipped(_)) || matches!(source, Source::Equipped(_)) =>
        {
            // To or from the gear: the server's (`inventory.rs`).
            crate::inventory::cursor_move(vm, from, source);
        }
        Cursor::Item(held, Some(Source::Bag(fb, fs))) => {
            // A move to another (empty) bag slot: the cursor let go, the
            // item locked till the server's answer a round trip later.
            let Source::Bag(tb, ts) = source else {
                return;
            };
            let ch = &engine(vm).world.character;
            let Some(bag) = ch.bags.get(&tb) else {
                return;
            };
            let whole = ch
                .bags
                .get(&fb)
                .and_then(|b| b.items.get(&fs))
                .map_or(held.count, |h| h.count);
            match bag.items.get(&ts).cloned() {
                None if held.count < whole => {
                    // Part of a stack (`SplitContainerItem`) into an
                    // empty slot (recorded: `bags`).
                    release_keeping_lock(vm);
                    split_later(vm, (fb, fs), (tb, ts), held.count);
                }
                None => {
                    release_keeping_lock(vm);
                    move_later(vm, (fb, fs), (tb, ts));
                }
                // Onto another item: the two swap places, the target
                // locked at once, then the cursor let go (recorded:
                // `bags`, the Rough Stone onto a Sharpening Wheel; a
                // stack of the same item, a part of a stack: unverified,
                // nothing).
                Some(other) if other.id != held.id && held.count >= whole && !other.locked => {
                    set_locked(vm, &Source::Bag(tb, ts), true);
                    release_keeping_lock(vm);
                    swap_later(vm, (fb, fs), (tb, ts));
                }
                Some(_) => {}
            }
        }
        Cursor::Empty => {
            let ch = &engine(vm).world.character;
            let held = match &source {
                Source::Bag(b, s) => ch.bags.get(b).and_then(|bag| bag.items.get(s)).cloned(),
                Source::Equipped(s) => ch.equipment.get(s).cloned(),
            };
            if let Some(held) = held {
                pick_up(vm, Cursor::Item(held, Some(source.clone())));
                set_locked(vm, &source, true);
            }
        }
        _ => {}
    }
}

/// The item at `source` picked up and let go at once, staying locked for
/// the server's answer (a right-click equip: recorded `SHOWGRID`, the
/// lock, `HIDEGRID`).
pub fn hold_for_server(vm: &mut Vm, source: Source) {
    let ch = &engine(vm).world.character;
    let held = match &source {
        Source::Bag(b, s) => ch.bags.get(b).and_then(|bag| bag.items.get(s)).cloned(),
        Source::Equipped(s) => ch.equipment.get(s).cloned(),
    };
    if let Some(held) = held {
        pick_up(vm, Cursor::Item(held, Some(source.clone())));
        set_locked(vm, &source, true);
        release_keeping_lock(vm);
    }
}

/// The cursor cleared, its item staying locked (the server answers the
/// move): `ACTIONBAR_HIDEGRID`, `CURSOR_CHANGED`.
pub fn release_keeping_lock(vm: &mut Vm) {
    let old = std::mem::take(&mut engine_mut(vm).world.character.cursor);
    if shows_highlights(&old) {
        fire(
            vm,
            "CLICKBINDINGS_SET_HIGHLIGHTS_SHOWN",
            &[Value::Bool(false)],
        );
    }
    if shows_grid(vm, &old) {
        fire(vm, "ACTIONBAR_HIDEGRID", &[]);
    }
    fire(
        vm,
        "CURSOR_CHANGED",
        &[
            Value::Bool(true),
            Value::Num(0.0),
            Value::Num(old.kind()),
            Value::Num(0.0),
        ],
    );
}

/// A bag item used at the bank: a carried one into the bank's first free
/// slot, a bank one into the bags; locked at once, moved a round trip
/// later (recorded).
pub fn use_at_bank(vm: &mut Vm, bag: i64, slot: usize) {
    let ch = &engine(vm).world.character;
    let Some(held) = ch.bags.get(&bag).and_then(|b| b.items.get(&slot)).cloned() else {
        return;
    };
    if held.locked {
        return;
    }
    let to = if crate::character::carried(bag) {
        crate::bank::free_slot(vm)
    } else {
        free_carried_slot(vm)
    };
    let Some(to) = to else {
        return;
    };
    set_locked(vm, &Source::Bag(bag, slot), true);
    move_later(vm, (bag, slot), to);
}

/// The bags' first free slot (the backpack first).
fn free_carried_slot(vm: &Vm) -> Option<(i64, usize)> {
    let ch = &engine(vm).world.character;
    ch.bags
        .iter()
        .filter(|(b, _)| crate::character::carried(**b))
        .find_map(|(b, bag)| {
            (1..=bag.slots)
                .find(|s| !bag.items.contains_key(s))
                .map(|s| (*b, s))
        })
}

fn move_later(vm: &mut Vm, from: (i64, usize), to: (i64, usize)) {
    let delay = crate::clock::server_delay(vm);
    let native = vm.native_with(
        "itemMoved",
        moved,
        vec![
            Value::Num(from.0 as f64),
            Value::Num(from.1 as f64),
            Value::Num(to.0 as f64),
            Value::Num(to.1 as f64),
        ],
    );
    crate::clock::after_engine(vm, delay, native);
}

/// The server moved the item: for each bag, the bank's first (recorded:
/// a bank tab before the backpack), the spells' update, the item's lock
/// gone where it arrived, the bag's update; the count's change when it
/// crossed into or out of the bags.
fn moved(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = |i: usize| num(c.upvals[i]).unwrap_or(0.0);
    let (fb, fs, tb, ts) = (n(0) as i64, n(1) as usize, n(2) as i64, n(3) as usize);
    let id = match engine(vm)
        .world
        .character
        .bags
        .get(&fb)
        .and_then(|b| b.items.get(&fs))
    {
        Some(h) => h.id,
        None => return Ok(vec![]),
    };
    let held = {
        let ch = &mut engine_mut(vm).world.character;
        ch.bags.get_mut(&fb).and_then(|b| b.items.remove(&fs))
    };
    let Some(held) = held else {
        return Ok(vec![]);
    };
    if let Some(b) = engine_mut(vm).world.character.bags.get_mut(&tb) {
        b.items.insert(ts, held);
    }
    for bag in update_order(vm, &[fb, tb]) {
        crate::character::equipped_spells_changed(vm);
        if bag == tb {
            set_locked(vm, &Source::Bag(tb, ts), false);
        }
        crate::character::bag_updated(vm, bag);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    // The count's change: crossing into or out of the bags (`bank`), and a
    // move among the bags too (`bags`: the Mining Pick from bag 1 into the
    // backpack, a swap between them).
    fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    Ok(vec![])
}

// ------------------------------------------------ the bags' server packets

/// The session's order of the bags' updates (`e.setBagOrder`).
const BAG_ORDER: &str = "cursor.bagOrder";
/// A new stack's data after the server's answer (`e.setItemDataDelay`).
const ITEM_DATA_DELAY: &str = "cursor.itemDataDelay";
/// The next sort's result (`e.setSortResult`).
const SORT_RESULT: &str = "cursor.sortResult";

/// `e.setBagOrder({ bag, … })`: the order the server's packets tell the
/// bags' updates in this session, before the default (`update_order`).
pub fn control_set_bag_order(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setBagOrder: a list of bags expected"));
    };
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm)
        .lua_helpers
        .insert(BAG_ORDER, Value::Table(t));
    Ok(vec![])
}

/// `e.setItemDataDelay(seconds)`: a new stack's data (a split's) this long
/// after the server's answer; nil: at once.
pub fn control_set_item_data_delay(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = match c.arg(1) {
        Value::Num(n) => Value::Num(n),
        _ => Value::Nil,
    };
    engine_mut(vm).lua_helpers.insert(ITEM_DATA_DELAY, v);
    Ok(vec![])
}

/// `e.setSortResult({ [bag] = { [slot] = { id, count, shownFirst, after } } })`:
/// where the next `C_Container.SortBags` puts the bags' items (the
/// server's sort; a merged stack shown first with `shownFirst`, its full
/// count `after` seconds later).
pub fn control_set_sort_result(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setSortResult: a table of the bags expected"));
    };
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm)
        .lua_helpers
        .insert(SORT_RESULT, Value::Table(t));
    Ok(vec![])
}

fn session_order(vm: &Vm) -> Vec<i64> {
    let Some(Value::Table(t)) = engine(vm).lua_helpers.get(BAG_ORDER).copied() else {
        return vec![];
    };
    let n = vm.heap.table(t).border();
    (1..=n)
        .filter_map(|i| num(vm.heap.table(t).get(Value::Num(i as f64))))
        .map(|b| b as i64)
        .collect()
}

/// The bags as a packet tells their updates: the session's order first
/// (`e.setBagOrder`; `bags` told bag 1, the reagent bag, then the backpack,
/// in two moves and a sort), else the bank's bags, the backpack, the
/// reagent bag, the other bags (`bank`; SuuUI probe bagsort: moves both ways
/// between the backpack and the reagent bag, a merge from the reagent bag
/// into bag 1, a burst of three; unverified among bags 1 to 4 and for the
/// keyring's `BAG_UPDATE(-1)` the client told between). The server's order
/// differs between sessions.
fn update_order(vm: &Vm, bags: &[i64]) -> Vec<i64> {
    let session = session_order(vm);
    let rank = |b: i64| match session.iter().position(|s| *s == b) {
        Some(p) => p as i64,
        None => {
            100 + match b {
                6.. => 0,
                0 => 1,
                5 => 2,
                1..=4 => 3,
                _ => 4,
            }
        }
    };
    let mut out = bags.to_vec();
    out.sort_unstable_by(|a, b| rank(*a).cmp(&rank(*b)).then(b.cmp(a)));
    out.dedup();
    out
}

/// The items whose counts a packet's moves changed, as the client told
/// them (`bags`' sort and swap): the backpack's slots first, then the
/// reagent bag's and the other bags' (unverified: nothing new came after
/// the backpack's), each changed slot's old item, then its new one, each
/// item once.
fn count_changes(changes: &[(i64, usize, Option<i64>, Option<i64>)]) -> Vec<i64> {
    let rank = |b: i64| match b {
        0 => 0,
        5 => 1,
        1..=4 => 2,
        _ => 3,
    };
    let mut sorted = changes.to_vec();
    sorted.sort_by_key(|(b, s, _, _)| (rank(*b), *b, *s));
    let mut out = Vec::new();
    for (_, _, old, new) in sorted {
        for id in [old, new].into_iter().flatten() {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

fn item_at(vm: &Vm, bag: i64, slot: usize) -> Option<Held> {
    engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .and_then(|b| b.items.get(&slot))
        .cloned()
}

fn later_native(vm: &mut Vm, after: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, after, native);
}

/// `C_Container.SplitContainerItem(bag, slot, count)`: `count` of the
/// stack on the cursor (`CURSOR_CHANGED`, the stack locked: recorded,
/// `bags`), its slot keeping the rest till the part is put down.
fn split_container_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(bag), Some(slot), Some(count)) = (
        num(vm.hidden(c.arg(1))),
        num(vm.hidden(c.arg(2))),
        num(vm.hidden(c.arg(3))),
    ) else {
        return Ok(vec![]);
    };
    let (bag, slot) = (bag as i64, slot as usize);
    if !matches!(engine(vm).world.character.cursor, Cursor::Empty) {
        return Ok(vec![]);
    }
    let Some(held) = item_at(vm, bag, slot) else {
        return Ok(vec![]);
    };
    if held.locked || held.arriving || count < 1.0 || count >= held.count {
        return Ok(vec![]);
    }
    let mut part = held;
    part.count = count;
    let source = Source::Bag(bag, slot);
    pick_up(vm, Cursor::Item(part, Some(source.clone())));
    set_locked(vm, &source, true);
    Ok(vec![])
}

fn split_later(vm: &mut Vm, from: (i64, usize), to: (i64, usize), count: f64) {
    let delay = crate::clock::server_delay(vm);
    later_native(
        vm,
        delay,
        "itemSplit",
        split_done,
        vec![
            Value::Num(from.0 as f64),
            Value::Num(from.1 as f64),
            Value::Num(to.0 as f64),
            Value::Num(to.1 as f64),
            Value::Num(count),
        ],
    );
}

/// The server split the stack (recorded once, `bags`: six Blood Shards,
/// three into an empty backpack slot): the new stack's slot taken, its
/// data a packet later (`e.setItemDataDelay`: the slot reads empty till
/// then), its bag's update; the old stack's lock gone, the spells' update,
/// its bag's update with the count less; one `BAG_UPDATE_DELAYED`, the
/// count's change; the new stack's data then as a bought item's
/// (`merchant::item_data_arrived`: the spells', the bag's, the
/// inventory's updates, no count's).
fn split_done(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = |i: usize| num(c.upvals[i]).unwrap_or(0.0);
    let (fb, fs, tb, ts, count) = (n(0) as i64, n(1) as usize, n(2) as i64, n(3) as usize, n(4));
    let Some(held) = item_at(vm, fb, fs) else {
        return Ok(vec![]);
    };
    if count >= held.count || item_at(vm, tb, ts).is_some() {
        return Ok(vec![]);
    }
    let delay = match engine(vm).lua_helpers.get(ITEM_DATA_DELAY) {
        Some(Value::Num(d)) => Some(*d),
        _ => None,
    };
    let part = Held {
        count,
        locked: false,
        arriving: delay.is_some(),
        ..held.clone()
    };
    if let Some(b) = engine_mut(vm).world.character.bags.get_mut(&tb) {
        b.items.insert(ts, part);
    }
    crate::character::bag_updated(vm, tb);
    if let Some(h) = engine_mut(vm)
        .world
        .character
        .bags
        .get_mut(&fb)
        .and_then(|b| b.items.get_mut(&fs))
    {
        h.count -= count;
    }
    set_locked(vm, &Source::Bag(fb, fs), false);
    crate::character::equipped_spells_changed(vm);
    crate::character::bag_updated(vm, fb);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(held.id as f64)]);
    if let Some(d) = delay {
        later_native(
            vm,
            d,
            "itemSplitData",
            crate::merchant::item_data_arrived,
            vec![Value::Num(tb as f64), Value::Num(ts as f64)],
        );
    }
    Ok(vec![])
}

fn swap_later(vm: &mut Vm, from: (i64, usize), to: (i64, usize)) {
    let delay = crate::clock::server_delay(vm);
    later_native(
        vm,
        delay,
        "itemSwapped",
        swap_done,
        vec![
            Value::Num(from.0 as f64),
            Value::Num(from.1 as f64),
            Value::Num(to.0 as f64),
            Value::Num(to.1 as f64),
        ],
    );
}

/// The server swapped the two items (recorded once, `bags`: the Rough
/// Stone from the backpack onto bag 1's Sharpening Wheel): for each bag
/// in the packet's order (`update_order`), the spells' update for each of
/// the two items moved, the lock gone in the bag's slot, the bag's update;
/// one `BAG_UPDATE_DELAYED`; both counts' changes (`count_changes`).
fn swap_done(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = |i: usize| num(c.upvals[i]).unwrap_or(0.0);
    let (fb, fs, tb, ts) = (n(0) as i64, n(1) as usize, n(2) as i64, n(3) as usize);
    let (Some(a), Some(b)) = (item_at(vm, fb, fs), item_at(vm, tb, ts)) else {
        return Ok(vec![]);
    };
    {
        let bags = &mut engine_mut(vm).world.character.bags;
        if let Some(bag) = bags.get_mut(&fb) {
            bag.items.insert(fs, b.clone());
        }
        if let Some(bag) = bags.get_mut(&tb) {
            bag.items.insert(ts, a.clone());
        }
    }
    for bag in update_order(vm, &[fb, tb]) {
        crate::character::equipped_spells_changed(vm);
        crate::character::equipped_spells_changed(vm);
        for (sb, ss) in [(tb, ts), (fb, fs)] {
            if sb == bag {
                set_locked(vm, &Source::Bag(sb, ss), false);
            }
        }
        crate::character::bag_updated(vm, bag);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    let changes = [
        (fb, fs, Some(a.id), Some(b.id)),
        (tb, ts, Some(b.id), Some(a.id)),
    ];
    for id in count_changes(&changes) {
        fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    }
    Ok(vec![])
}

/// What the sort locks, in the client's order (recorded once, `bags`): the
/// worn gear by slot, then each bag in the packets' order (the session's,
/// then the bank's bags): the bag itself where it is worn, then its items
/// by slot.
fn sort_locks(vm: &Vm) -> Vec<Source> {
    let ch = &engine(vm).world.character;
    let bag_slots: Vec<i64> = (1..=5)
        .map(|b| crate::character::bag_inventory_slot(b) as i64)
        .collect();
    let mut out: Vec<Source> = ch
        .equipment
        .iter()
        .filter(|(s, h)| h.id != 0 && !bag_slots.contains(s))
        .map(|(s, _)| Source::Equipped(*s))
        .collect();
    let mut bags: Vec<i64> = ch.bags.keys().copied().collect();
    let session = session_order(vm);
    let rank = |b: i64| match session.iter().position(|s| *s == b) {
        Some(p) => (0, p as i64),
        None if crate::character::carried(b) => (1, b),
        None => (2, b),
    };
    bags.sort_by_key(|b| rank(*b));
    for bag in bags {
        if (1..=5).contains(&bag) {
            let inv = crate::character::bag_inventory_slot(bag) as i64;
            if ch.equipment.get(&inv).is_some_and(|h| h.id != 0) {
                out.push(Source::Equipped(inv));
            }
        }
        if let Some(b) = ch.bags.get(&bag) {
            out.extend(b.items.keys().map(|s| Source::Bag(bag, *s)));
        }
    }
    out
}

/// `C_Container.SortBags()`: everything the player has locked at once (the
/// click's frame), the server's sort a round trip later (`sorted`).
fn sort_bags(vm: &mut Vm, _c: NativeCall) -> Ret {
    for source in sort_locks(vm) {
        set_locked(vm, &source, true);
    }
    let delay = crate::clock::server_delay(vm);
    later_native(vm, delay, "bagsSorted", sorted, vec![]);
    Ok(vec![])
}

/// A sorted slot's item: its ID, count, and a merged stack's count shown
/// first and when its full count comes.
type Placed = (i64, f64, Option<(f64, f64)>);

/// The server's sort (recorded once, `bags`): everything unlocked in the
/// locks' order; the items where `e.setSortResult` puts them (else where
/// they are); for each bag in the packets' order, each changed slot by
/// slot: the spells' update for the item gone and for the item come
/// (once for a slot emptied or filled), the bag's update (the bags read
/// their sorted state throughout); one `BAG_UPDATE_DELAYED`; the counts'
/// changes (`count_changes`); a merged stack's full count after its
/// `after` (the spells', the bag's, the inventory's updates, the count's).
fn sorted(vm: &mut Vm, _c: NativeCall) -> Ret {
    for source in sort_locks(vm) {
        set_locked(vm, &source, false);
    }
    let Some(Value::Table(result)) = engine_mut(vm).lua_helpers.remove(SORT_RESULT) else {
        return Ok(vec![]);
    };
    // The new layout, from the result's table.
    let mut layout: Vec<(i64, BTreeMap<usize, Placed>)> = Vec::new();
    let entries: Vec<(Value, Value)> = vm.heap.table(result).entries().collect();
    for (k, v) in entries {
        let (Value::Num(bag), Value::Table(slots)) = (k, v) else {
            continue;
        };
        let mut items = BTreeMap::new();
        let list: Vec<(Value, Value)> = vm.heap.table(slots).entries().collect();
        for (sk, sv) in list {
            let (Value::Num(slot), Value::Table(item)) = (sk, sv) else {
                continue;
            };
            let get = |vm: &mut Vm, key: &str| {
                let k = vm.str(key);
                num(vm.heap.table(item).get(k))
            };
            let Some(id) = get(vm, "id") else { continue };
            let count = get(vm, "count").unwrap_or(1.0);
            let merge = get(vm, "shownFirst").map(|first| (first, get(vm, "after").unwrap_or(0.0)));
            items.insert(slot as usize, (id as i64, count, merge));
        }
        layout.push((bag as i64, items));
    }
    // The old items, to carry their data along (links, binding), with
    // where they were.
    let mut pool: Vec<(i64, usize, Held)> = Vec::new();
    for (bag, _) in &layout {
        if let Some(b) = engine(vm).world.character.bags.get(bag) {
            pool.extend(b.items.iter().map(|(s, h)| (*bag, *s, h.clone())));
        }
    }
    let mut changes: Vec<(i64, usize, Option<i64>, Option<i64>)> = Vec::new();
    let mut merges: Vec<(i64, usize, f64)> = Vec::new();
    let mut new_bags: Vec<(i64, BTreeMap<usize, Held>)> = Vec::new();
    for (bag, items) in &layout {
        let mut held = BTreeMap::new();
        for (slot, (id, count, merge)) in items {
            let base = match pool.iter().position(|(_, _, h)| h.id == *id) {
                Some(p) => pool.remove(p).2,
                None => Held {
                    id: *id,
                    ..Held::default()
                },
            };
            let shown = match merge {
                Some((first, _)) => *first,
                None => *count,
            };
            if let Some((_, after)) = merge {
                merges.push((*bag, *slot, *after));
            }
            held.insert(
                *slot,
                Held {
                    count: shown,
                    locked: false,
                    arriving: false,
                    ..base
                },
            );
            let _ = count;
        }
        new_bags.push((*bag, held));
    }
    for (bag, held) in new_bags {
        let Some(slots) = engine(vm).world.character.bags.get(&bag).map(|b| b.slots) else {
            continue;
        };
        for slot in 1..=slots {
            let old = item_at(vm, bag, slot).map(|h| (h.id, h.count));
            let new = held.get(&slot).map(|h| (h.id, h.count));
            if old != new {
                changes.push((bag, slot, old.map(|o| o.0), new.map(|n| n.0)));
            }
        }
        if let Some(b) = engine_mut(vm).world.character.bags.get_mut(&bag) {
            b.items = held;
        }
    }
    // A stack merged into another (an old item left over): its spells'
    // update first in the packet, not in its old slot's (recorded: the
    // second Blood Shard stack's, three in bag 1's first slot's update, one
    // in the backpack's 19th's).
    let gone: Vec<(i64, usize)> = pool.iter().map(|(b, s, _)| (*b, *s)).collect();
    let bags: Vec<i64> = changes.iter().map(|c| c.0).collect();
    let mut first = true;
    for bag in update_order(vm, &bags) {
        for (_, slot, old, new) in changes
            .iter()
            .filter(|c| c.0 == bag)
            .cloned()
            .collect::<Vec<_>>()
        {
            if first {
                for _ in &gone {
                    crate::character::equipped_spells_changed(vm);
                }
                first = false;
            }
            let old = old.is_some() && !gone.contains(&(bag, slot));
            crate::character::equipped_spells_changed(vm);
            if old && new.is_some() {
                crate::character::equipped_spells_changed(vm);
            }
            crate::character::bag_updated(vm, bag);
        }
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    for id in count_changes(&changes) {
        fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    }
    for (bag, slot, after) in merges {
        let count = layout
            .iter()
            .find(|(b, _)| *b == bag)
            .and_then(|(_, items)| items.get(&slot))
            .map_or(0.0, |(_, c, _)| *c);
        later_native(
            vm,
            after,
            "bagsMerged",
            merged,
            vec![
                Value::Num(bag as f64),
                Value::Num(slot as f64),
                Value::Num(count),
            ],
        );
    }
    Ok(vec![])
}

/// A sort's merged stack at its full count (recorded: 0.181 s after the
/// sort's packet): the spells', the bag's, the inventory's updates, the
/// count's change.
fn merged(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = |i: usize| num(c.upvals[i]).unwrap_or(0.0);
    let (bag, slot, count) = (n(0) as i64, n(1) as usize, n(2));
    let Some(id) = engine_mut(vm)
        .world
        .character
        .bags
        .get_mut(&bag)
        .and_then(|b| b.items.get_mut(&slot))
        .map(|h| {
            h.count = count;
            h.id
        })
    else {
        return Ok(vec![]);
    };
    crate::character::equipped_spells_changed(vm);
    crate::character::bag_updated(vm, bag);
    let player = vm.str("player");
    fire(vm, "UNIT_INVENTORY_CHANGED", &[player]);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    Ok(vec![])
}

/// What the cursor holds: an item and where it came from.
pub fn held_item(vm: &Vm) -> Option<(Held, Source)> {
    match &engine(vm).world.character.cursor {
        Cursor::Item(h, Some(s)) => Some((h.clone(), s.clone())),
        _ => None,
    }
}

/// An item lock told: `ITEM_LOCK_CHANGED` and `ITEM_LOCKED`/`ITEM_UNLOCKED`;
/// nothing if it is already so (recorded: a posted auction's item unlocked
/// once, though Blizzard's sell frame unlocks it again).
pub fn lock(vm: &mut Vm, source: &Source, locked: bool) {
    let ch = &engine(vm).world.character;
    let held = match source {
        Source::Bag(bag, slot) => ch.bags.get(bag).and_then(|b| b.items.get(slot)),
        Source::Equipped(slot) => ch.equipment.get(slot),
    };
    if held.is_some_and(|h| h.locked == locked) {
        return;
    }
    set_locked(vm, source, locked);
}

fn pickup_container_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(bag), Some(slot)) = (num(c.arg(1)), num(c.arg(2))) else {
        return Ok(vec![]);
    };
    pickup_item_at(vm, Source::Bag(bag as i64, slot as usize));
    Ok(vec![])
}

fn pickup_inventory_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(slot) = num(c.arg(1)) else {
        return Ok(vec![]);
    };
    // The repair cursor mends the item instead (`repair`, recorded).
    if crate::actions2::repairing(vm) {
        crate::merchant::repair_slot(vm, slot as i64);
        return Ok(vec![]);
    }
    pickup_item_at(vm, Source::Equipped(slot as i64));
    Ok(vec![])
}

/// `GetCursorInfo()`: what the cursor holds (see the module).
fn get_cursor_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(match engine(vm).world.character.cursor.clone() {
        Cursor::Empty => vec![],
        Cursor::Spell(id) => {
            let slot = crate::stats::spellbook_slot_of(vm, id)
                .map_or(Value::Nil, |s| Value::Num(s as f64));
            vec![vm.str("spell"), slot, vm.str("spell"), Value::Num(id)]
        }
        Cursor::Macro(index) => vec![vm.str("macro"), Value::Num(index)],
        Cursor::Item(held, _) => {
            let link = crate::character::link_of(vm, &held).unwrap_or(Value::Nil);
            vec![vm.str("item"), Value::Num(held.id as f64), link]
        }
    })
}

fn has(vm: &Vm, kind: f64) -> Ret {
    Ok(vec![Value::Bool(
        engine(vm).world.character.cursor.kind() == kind,
    )])
}

fn cursor_has_item(vm: &mut Vm, _c: NativeCall) -> Ret {
    has(vm, 1.0)
}

fn cursor_has_spell(vm: &mut Vm, _c: NativeCall) -> Ret {
    has(vm, 3.0)
}

fn cursor_has_macro(vm: &mut Vm, _c: NativeCall) -> Ret {
    has(vm, 7.0)
}

fn cursor_has_money(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("ClearCursor", clear_cursor),
        ("GetCursorInfo", get_cursor_info),
        ("CursorHasItem", cursor_has_item),
        ("CursorHasSpell", cursor_has_spell),
        ("CursorHasMacro", cursor_has_macro),
        ("CursorHasMoney", cursor_has_money),
        ("C_Spell.PickupSpell", pickup_spell),
        ("PickupSpell", pickup_spell),
        ("C_SpellBook.PickupSpellBookItem", pickup_spellbook_item),
        ("PickupSpellBookItem", pickup_spellbook_item),
        ("PickupMacro", pickup_macro),
        ("PickupAction", pickup_action),
        ("PlaceAction", place_action),
        ("C_Container.PickupContainerItem", pickup_container_item),
        ("PickupContainerItem", pickup_container_item),
        ("PickupInventoryItem", pickup_inventory_item),
        ("C_Container.SplitContainerItem", split_container_item),
        ("SplitContainerItem", split_container_item),
        ("C_Container.SortBags", sort_bags),
    ]
}
