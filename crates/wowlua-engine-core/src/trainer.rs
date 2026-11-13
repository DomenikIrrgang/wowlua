//! The trainers' state (wowlua-engine's `trainer` answers from it).

use std::collections::HashMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, TableRef, Vm};

use crate::hooks;
use crate::units::UnitId;

pub fn beast_training(vm: &mut Vm) {
    (hooks::get().beast_training)(vm)
}

pub fn open(vm: &mut Vm, npc: UnitId) {
    (hooks::get().trainer_open)(vm, npc)
}

pub fn pet_left(vm: &mut Vm) {
    (hooks::get().pet_left)(vm)
}

pub fn control_learn(vm: &mut Vm, c: NativeCall) -> Ret {
    (hooks::get().trainer_learn)(vm, c)
}

pub fn control_unlearn(vm: &mut Vm, c: NativeCall) -> Ret {
    (hooks::get().trainer_unlearn)(vm, c)
}

/// The hunter's Beast Training (inferred: of the client's two spells of
/// that name, 5149 and 5300, the one classic's hunters cast).
pub const BEAST_TRAINING: f64 = 5149.0;

#[derive(Default)]
pub struct Trainers {
    /// The NPCs' trainers: unit → the test's table.
    pub defined: std::collections::BTreeMap<usize, TableRef>,
    /// The trainer open, and whether its list has come (`TRAINER_UPDATE`).
    pub open: Option<UnitId>,
    pub listed: bool,
    pub selected: Option<usize>,
    /// Whether the trainer open is Beast Training, the pet's (decision
    /// T618).
    pub pet: bool,
    /// Beast Training's list (`defineBeastTraining`).
    pub beast: Option<TableRef>,
    /// Whether the services' ranks waiting for their data have it.
    pub ranks_loaded: bool,
    /// The spells learned here.
    pub learned: std::collections::BTreeSet<i64>,
    /// The type filters off ("available", "unavailable", "used"; recorded:
    /// only "used").
    pub filtered_out: Option<std::collections::BTreeSet<&'static str>>,
    /// The open trainer's recorded data (a class trainer's), if any.
    pub key: Option<&'static str>,
    /// Its services' ranks still loading this session: spell → the
    /// tooltip data each waits for (`loading`).
    pub loading: HashMap<i64, f64>,
    /// Its services shown (their indices in its data), as the window
    /// opened or a filter changed.
    pub shown: Vec<usize>,
}
