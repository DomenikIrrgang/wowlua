//! A trade skill window's state (wowlua-engine's `tradeskill` answers from it).

use std::collections::{HashMap, VecDeque};

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Vm};

use crate::hooks;

pub fn give_up_asked(vm: &mut Vm) {
    (hooks::get().give_up_asked)(vm)
}

pub fn any_answer(call: &str) -> Option<&'static str> {
    (hooks::get().tradeskill_answer)(call)
}

pub fn control_open(vm: &mut Vm, c: NativeCall) -> Ret {
    (hooks::get().tradeskill_open)(vm, c)
}

pub fn control_skill_ups(vm: &mut Vm, c: NativeCall) -> Ret {
    (hooks::get().tradeskill_skill_ups)(vm, c)
}

pub fn control_spell_focus(vm: &mut Vm, c: NativeCall) -> Ret {
    (hooks::get().tradeskill_spell_focus)(vm, c)
}

pub fn control_timings(vm: &mut Vm, c: NativeCall) -> Ret {
    (hooks::get().tradeskill_timings)(vm, c)
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
    /// The bags' packet split: its first changes (`split`, the item's
    /// first), the rest and the reagents' taking these seconds after
    /// (`rest`).
    pub split: Option<(usize, f64)>,
    /// A used-up reagent's removal after the item (`spent`; else with it).
    pub spent: Option<f64>,
    /// A new slot's bag told before the item's push (`slotFirst`).
    pub slot_first: bool,
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
    /// When the item being made's used-up reagents' removal is told after
    /// it (`Timing::spent`).
    pub spent: Option<f64>,
    /// The item being made's new slot told before its push
    /// (`Timing::slot_first`).
    pub slot_first: bool,
    /// The windows shown this session.
    pub shown: u32,
}
