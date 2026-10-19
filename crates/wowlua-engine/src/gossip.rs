//! An NPC's gossip, as the client recorded a class trainer's (`/stdtest
//! engine scenario trainer`, a ping near 0.3 s): `defineGossip(npc, {
//! text, options })`; `player:interact(npc)` opens it a round trip later
//! (`PLAYER_INTERACTION_MANAGER_FRAME_SHOW(3)`, the text's packet later
//! `GOSSIP_SHOW(nil)`), `C_GossipInfo`'s answers (its text, its options as
//! the client lists them, no quests). `C_GossipInfo.SelectOption(id)` (or
//! `SelectOptionByIndex(orderIndex)`, Blizzard's buttons):
//! `GOSSIP_CONFIRM_CANCEL()` at once, the server's `GOSSIP_CLOSED(true)` a
//! round trip later and what the option opens (`opens = "trainer"`: the
//! NPC's trainer, `trainer.rs`; `"taxi"`; `"merchant"` with the option's
//! `merchant` table, `openMerchant`'s).
//!
//! The text usually comes with the interaction, before its event
//! (recorded: two flight masters, an NPC talked to twice, the first time
//! too); a class trainer's 0.472 s later (`textAfter`).
//!
//! Unverified: an option that opens nothing, quests in a gossip, why the
//! trainer's text came later.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// The text's packet after the interaction's, by default: with it
/// (recorded at three NPCs; a class trainer's came 0.472 s later).
const TEXT_AFTER: f64 = 0.0;
/// `Enum.PlayerInteractionType.Gossip`.
const GOSSIP: f64 = 3.0;

#[derive(Default)]
pub struct Gossip {
    /// The NPCs' gossips: unit → the test's table.
    pub defined: std::collections::BTreeMap<usize, TableRef>,
    /// The NPC whose gossip is open.
    pub open: Option<UnitId>,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

/// `defineGossip(npc, { text, options = { { name, icon, gossipOptionID,
/// opens } } })`.
pub fn control_define(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(npc) = units::handle_of(vm, c.arg(1)) else {
        return Err(vm.error("defineGossip: an NPC expected"));
    };
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("defineGossip: a table of the gossip expected"));
    };
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm).world.gossip.defined.insert(npc.0, t);
    Ok(vec![])
}

/// Whether an NPC has gossip.
pub fn has(vm: &Vm, npc: UnitId) -> bool {
    engine(vm).world.gossip.defined.contains_key(&npc.0)
}

/// The player talks to an NPC with gossip: the interaction a round trip
/// later, the gossip with its text's packet.
pub fn interact(vm: &mut Vm, npc: UnitId) {
    let delay = crate::clock::server_delay(vm);
    let id = Value::Num(npc.0 as f64);
    // The text's packet's delay: the gossip's `textAfter`, else with the
    // interaction (0: before its event).
    let after = engine(vm)
        .world
        .gossip
        .defined
        .get(&npc.0)
        .copied()
        .map(|t| field(vm, t, "textAfter"));
    let after = match after {
        Some(Value::Num(n)) => n,
        _ => TEXT_AFTER,
    };
    if after <= 0.0 {
        let both = vm.native_with("gossipAtOnce", text_then_interaction as NativeFn, vec![id]);
        crate::clock::after_engine(vm, delay, both);
        return;
    }
    let shown = vm.native_with("gossipInteraction", interaction as NativeFn, vec![id]);
    crate::clock::after_engine(vm, delay, shown);
    let text = vm.native_with("gossipShown", shown_text as NativeFn, vec![id]);
    crate::clock::after_engine(vm, delay + after, text);
}

/// The text known: `GOSSIP_SHOW` then the interaction's frame event.
fn text_then_interaction(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(npc) = npc_of(&c) else {
        return Ok(vec![]);
    };
    engine_mut(vm).units.npc = Some(npc);
    engine_mut(vm).world.gossip.open = Some(npc);
    fire(vm, "GOSSIP_SHOW", &[Value::Nil]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(GOSSIP)],
    );
    Ok(vec![])
}

fn npc_of(c: &NativeCall) -> Option<UnitId> {
    match c.upvals.first() {
        Some(Value::Num(n)) => Some(UnitId(*n as usize)),
        _ => None,
    }
}

fn interaction(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(npc) = npc_of(&c) else {
        return Ok(vec![]);
    };
    engine_mut(vm).units.npc = Some(npc);
    engine_mut(vm).world.gossip.open = Some(npc);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(GOSSIP)],
    );
    Ok(vec![])
}

fn shown_text(vm: &mut Vm, c: NativeCall) -> Ret {
    if engine(vm).world.gossip.open != npc_of(&c) {
        return Ok(vec![]);
    }
    fire(vm, "GOSSIP_SHOW", &[Value::Nil]);
    Ok(vec![])
}

fn open_data(vm: &Vm) -> Option<TableRef> {
    let npc = engine(vm).world.gossip.open?;
    engine(vm).world.gossip.defined.get(&npc.0).copied()
}

/// The options as defined, each a table.
fn options(vm: &mut Vm, t: TableRef) -> Vec<TableRef> {
    let Value::Table(list) = field(vm, t, "options") else {
        return vec![];
    };
    let n = vm.heap.table(list).border();
    (1..=n)
        .filter_map(|i| match vm.heap.table(list).get(Value::Num(i as f64)) {
            Value::Table(o) => Some(o),
            _ => None,
        })
        .collect()
}

/// `C_GossipInfo.GetText()`.
fn get_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some(t) = open_data(vm) else {
        return Ok(vec![vm.str("")]);
    };
    Ok(vec![field(vm, t, "text")])
}

/// `C_GossipInfo.GetOptions()`: each option's fields as recorded (flags 0,
/// no rewards, status 0, its order from 0).
fn get_options(vm: &mut Vm, _c: NativeCall) -> Ret {
    let out = vm.new_table();
    let Some(t) = open_data(vm) else {
        return Ok(vec![Value::Table(out)]);
    };
    for (i, o) in options(vm, t).into_iter().enumerate() {
        let e = vm.new_table();
        for key in ["name", "icon", "gossipOptionID"] {
            let v = field(vm, o, key);
            set(vm, e, key, v);
        }
        set(vm, e, "flags", Value::Num(0.0));
        set(vm, e, "orderIndex", Value::Num(i as f64));
        let rewards = vm.new_table();
        set(vm, e, "rewards", Value::Table(rewards));
        set(vm, e, "selectOptionWhenOnlyOption", Value::Bool(false));
        set(vm, e, "status", Value::Num(0.0));
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num((i + 1) as f64), Value::Table(e));
    }
    Ok(vec![Value::Table(out)])
}

fn empty_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `C_GossipInfo.SelectOption(id)`: `GOSSIP_CONFIRM_CANCEL()` at once; the
/// server closes the gossip and opens what the option does.
fn select_option(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = open_data(vm) else {
        return Ok(vec![]);
    };
    let Value::Num(wanted) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    let chosen = options(vm, t)
        .into_iter()
        .find(|o| matches!(field(vm, *o, "gossipOptionID"), Value::Num(id) if id == wanted));
    match chosen {
        Some(option) => choose(vm, option),
        None => Ok(vec![]),
    }
}

/// `C_GossipInfo.SelectOptionByIndex(orderIndex)` (Blizzard's buttons):
/// as `SelectOption`, by its order from 0.
fn select_option_by_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = open_data(vm) else {
        return Ok(vec![]);
    };
    let Value::Num(i) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    match options(vm, t).get(i as usize).copied() {
        Some(option) if i >= 0.0 => choose(vm, option),
        _ => Ok(vec![]),
    }
}

fn choose(vm: &mut Vm, option: TableRef) -> Ret {
    fire(vm, "GOSSIP_CONFIRM_CANCEL", &[]);
    let opens = field(vm, option, "opens");
    let merchant = field(vm, option, "merchant");
    let delay = crate::clock::server_delay(vm);
    let later = vm.native_with(
        "gossipChosen",
        chosen_option as NativeFn,
        vec![opens, merchant],
    );
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn chosen_option(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(npc) = engine_mut(vm).world.gossip.open.take() else {
        return Ok(vec![]);
    };
    fire(vm, "GOSSIP_CLOSED", &[Value::Bool(true)]);
    let opens = match c.upvals.first() {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(*s)).into_owned(),
        _ => String::new(),
    };
    match opens.as_str() {
        "trainer" => crate::trainer::open(vm, npc),
        "taxi" => crate::taxi::open(vm, npc),
        // The vendor's window, its goods the option's `merchant` table
        // (recorded: gossip2).
        "merchant" => {
            if let Some(Value::Table(data)) = c.upvals.get(1).copied() {
                crate::merchant::open(vm, data, Some(npc))?;
            }
        }
        _ => {}
    }
    Ok(vec![])
}

/// `C_GossipInfo.CloseGossip()`: the gossip closed, at once (recorded:
/// gossip2, closed with Escape).
fn close_gossip(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine_mut(vm).world.gossip.open.take().is_some() {
        fire(vm, "GOSSIP_CLOSED", &[Value::Bool(false)]);
        fire(
            vm,
            "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
            &[Value::Num(GOSSIP)],
        );
        fire(
            vm,
            "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
            &[Value::Num(0.0)],
        );
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_GossipInfo.GetText", get_text),
        ("C_GossipInfo.GetOptions", get_options),
        ("C_GossipInfo.GetAvailableQuests", empty_list),
        ("C_GossipInfo.GetActiveQuests", empty_list),
        ("C_GossipInfo.GetNumAvailableQuests", zero),
        ("C_GossipInfo.GetNumActiveQuests", zero),
        ("C_GossipInfo.SelectOption", select_option),
        ("C_GossipInfo.SelectOptionByIndex", select_option_by_index),
        ("C_GossipInfo.CloseGossip", close_gossip),
    ]
}
