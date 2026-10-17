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
//! Unverified: picking up while holding something (swaps), moving an item
//! to another slot, placing on a filled action slot, money.

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

/// Holds `new` (from empty): the grid shows, the cursor changes.
fn pick_up(vm: &mut Vm, new: Cursor) {
    let kind = new.kind();
    engine_mut(vm).world.character.cursor = new;
    fire(vm, "ACTIONBAR_SHOWGRID", &[]);
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
    fire(vm, "ACTIONBAR_HIDEGRID", &[]);
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
    let id = num(action.id).unwrap_or(0.0);
    let held = match action.kind.as_str() {
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
    };
    put_down(vm);
    pick_up(vm, held);
    fire(vm, "ACTIONBAR_SLOT_CHANGED", &[Value::Num(slot as f64)]);
    Ok(vec![])
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
    engine_mut(vm).world.character.actions.insert(slot, action);
    fire(vm, "ACTIONBAR_SLOT_CHANGED", &[Value::Num(slot as f64)]);
    Ok(vec![])
}

/// Takes the item at `source` onto the cursor, or puts the held item back
/// when it came from there; another slot: moves it (unverified).
fn pickup_item_at(vm: &mut Vm, source: Source) {
    match engine(vm).world.character.cursor.clone() {
        Cursor::Item(_, Some(from)) if from == source => {
            put_down(vm);
        }
        Cursor::Item(held, Some(from)) => {
            // A move to another (empty) slot.
            let ch = &engine(vm).world.character;
            let target_empty = match &source {
                Source::Bag(b, s) => ch.bags.get(b).is_some_and(|bag| !bag.items.contains_key(s)),
                Source::Equipped(s) => !ch.equipment.contains_key(s),
            };
            if target_empty {
                put_down(vm);
                let ch = &mut engine_mut(vm).world.character;
                match &from {
                    Source::Bag(b, s) => {
                        ch.bags.get_mut(b).map(|bag| bag.items.remove(s));
                    }
                    Source::Equipped(s) => {
                        ch.equipment.remove(s);
                    }
                }
                let mut held = held;
                held.locked = false;
                match &source {
                    Source::Bag(b, s) => {
                        ch.bags.get_mut(b).map(|bag| bag.items.insert(*s, held));
                    }
                    Source::Equipped(s) => {
                        ch.equipment.insert(*s, held);
                    }
                }
                for bag in [&from, &source] {
                    if let Source::Bag(b, _) = bag {
                        fire(vm, "BAG_UPDATE", &[Value::Num(*b as f64)]);
                    }
                }
                fire(vm, "BAG_UPDATE_DELAYED", &[]);
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
