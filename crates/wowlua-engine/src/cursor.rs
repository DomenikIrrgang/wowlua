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
/// item but a trade good (recorded: Copper Ore without the grid, the
/// Hearthstone and a weapon with it; other classes unverified).
fn shows_grid(vm: &mut Vm, cursor: &Cursor) -> bool {
    match cursor {
        Cursor::Item(held, _) => {
            // `Enum.ItemClass.Tradegoods`.
            !matches!(
                crate::character::item_field(vm, held.id, "classID"),
                Value::Num(7.0)
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

/// `C_SpellBook.PickupSpellBookItem(slot, bank)`: the slot's spell.
fn pickup_spellbook_item(vm: &mut Vm, c: NativeCall) -> Ret {
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
fn pickup_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(slot) = num(c.arg(1)).map(|s| s as i64) else {
        return Ok(vec![]);
    };
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
        Cursor::Item(_, Some(Source::Bag(fb, fs))) => {
            // A move to another (empty) bag slot: the cursor let go, the
            // item locked till the server's answer a round trip later.
            let Source::Bag(tb, ts) = source else {
                return;
            };
            let ch = &engine(vm).world.character;
            if ch
                .bags
                .get(&tb)
                .is_some_and(|bag| !bag.items.contains_key(&ts))
            {
                release_keeping_lock(vm);
                move_later(vm, (fb, fs), (tb, ts));
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
    let before = crate::character::count_in_bags(vm, id);
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
    // Told the bank's bags first (recorded), then the backpack, the
    // reagent bag, the other bags (SuuUI probe bagsort: moves both ways
    // between the backpack and the reagent bag, a merge from the reagent
    // bag into bag 1, a burst of three; unverified among bags 1 to 4 and
    // for the keyring's `BAG_UPDATE(-1)` the client told between).
    let rank = |b: i64| match b {
        6.. => 0,
        0 => 1,
        5 => 2,
        1..=4 => 3,
        _ => 4,
    };
    let mut bags = vec![fb, tb];
    bags.sort_unstable_by(|a, b| rank(*a).cmp(&rank(*b)).then(b.cmp(a)));
    bags.dedup();
    for bag in bags {
        crate::character::equipped_spells_changed(vm);
        if bag == tb {
            set_locked(vm, &Source::Bag(tb, ts), false);
        }
        crate::character::bag_updated(vm, bag);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    if crate::character::count_in_bags(vm, id) != before {
        fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(id as f64)]);
    }
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
    ]
}
