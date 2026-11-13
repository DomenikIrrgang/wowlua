//! What the core asks of the modules of `wowlua-engine` (which builds on
//! it): their behaviours, and the few answers of theirs the core's own
//! modules need. `wowlua_engine::install` sets them before it installs
//! the engine; each module of the core named after one of those modules
//! (`crate::trainer`) forwards to them, so the core's code calls them by
//! the same names as before the engine was split in two crates.

use std::sync::OnceLock;

use wowlua_vm::{LuaError, NativeFn, TableRef, Value, Vm};

use crate::Behaviours;
use crate::scene::Look;
use crate::units::UnitId;
use crate::widgets::MethodFn;

/// The functions of `wowlua-engine` the core calls.
pub struct Hooks {
    pub own_behaviours: fn(&str) -> Vec<Behaviours<NativeFn>>,
    pub fill_ins: fn() -> Vec<Behaviours<NativeFn>>,
    pub method_behaviours: fn() -> Vec<Behaviours<MethodFn>>,
    pub standins_install: fn(&mut Vm),
    pub silent_unchecked: fn(&serde_json::Value) -> bool,
    pub silent_checked: fn(&serde_json::Value) -> bool,
    pub macro_spell: fn(&mut Vm, Value) -> Option<f64>,
    pub spell_of: fn(&mut Vm, Value) -> Option<f64>,
    pub in_range: fn(&mut Vm, f64, Option<UnitId>) -> Value,
    pub range_checks_tick: fn(&mut Vm),
    pub item_data: fn(&mut Vm, i64) -> Option<TableRef>,
    pub spell_data: fn(&mut Vm, i64) -> Option<TableRef>,
    pub spell_name: fn(&mut Vm, i64) -> Value,
    pub item_id: fn(&mut Vm, Value) -> Option<i64>,
    pub scanned_spell: fn(&mut Vm, i64) -> Option<TableRef>,
    pub tracking_aura_changed: fn(&mut Vm, f64),
    pub give_up_asked: fn(&mut Vm),
    pub tradeskill_answer: fn(&str) -> Option<&'static str>,
    pub tradeskill_open: NativeFn,
    pub tradeskill_skill_ups: NativeFn,
    pub tradeskill_spell_focus: NativeFn,
    pub tradeskill_timings: NativeFn,
    pub beast_training: fn(&mut Vm),
    pub trainer_open: fn(&mut Vm, UnitId),
    pub pet_left: fn(&mut Vm),
    pub trainer_learn: NativeFn,
    pub trainer_unlearn: NativeFn,
    pub look_of: fn(&mut Vm, &str) -> Look,
    pub format_with: fn(&mut Vm, Value, f64, bool) -> Result<Value, LuaError>,
    pub navigation_frame: fn(&mut Vm) -> Result<(), LuaError>,
    pub poi_map: fn(&mut Vm) -> f64,
    pub quest_poi_arrived: fn(&mut Vm, i64) -> Result<(), LuaError>,
    pub get_num_languages: NativeFn,
    pub taxi_open: fn(&mut Vm, UnitId),
    pub file_id_of_path: fn(&mut Vm, &str) -> Option<f64>,
    pub auction_has: fn(&Vm, UnitId) -> bool,
    pub auction_interact: fn(&mut Vm, UnitId),
    pub traits_answer: fn(&str) -> Option<&'static &'static str>,
    pub player_config: fn(&Vm, f64) -> f64,
}

static HOOKS: OnceLock<&'static Hooks> = OnceLock::new();

/// Sets the hooks (`wowlua_engine::install`, each time: the first stays,
/// and they are the same each time).
pub fn set(hooks: &'static Hooks) {
    let _ = HOOKS.set(hooks);
}

/// The hooks `wowlua_engine::install` set.
pub fn get() -> &'static Hooks {
    HOOKS
        .get()
        .expect("the engine is installed with wowlua_engine::install")
}
