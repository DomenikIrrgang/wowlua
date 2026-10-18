//! The achievement search (the legacy system's challenges page): the
//! achievements are the client's recorded answers (`/stdtest engine
//! achievements`); `SetAchievementSearchString(text)` builds the results a
//! moment later (`ACHIEVEMENT_SEARCH_UPDATED`, recorded 0.081 s after the
//! page opened), none before.
//!
//! Unverified: which achievements a search finds (the categories' of
//! `GetCategoryList()`, by name, case aside; every one for ""), their
//! order, the delay's cause.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// The search's results after it is set (recorded).
const SEARCH_AFTER: f64 = 0.081;

#[derive(Default)]
pub struct Achievements {
    /// The search's results, once built.
    pub results: Vec<f64>,
}

fn recorded_first(vm: &mut Vm, call: &str) -> Value {
    crate::recorded::lookup_text(vm, call)
        .and_then(|v| v.first().copied())
        .unwrap_or(Value::Nil)
}

fn text(vm: &Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    }
}

/// The achievements a search finds.
fn search(vm: &mut Vm, wanted: &str) -> Vec<f64> {
    let wanted = wanted.to_lowercase();
    let Value::Table(list) = recorded_first(vm, "GetCategoryList()") else {
        return Vec::new();
    };
    let n = vm.heap.table(list).border();
    let categories: Vec<Value> = (1..=n)
        .map(|i| vm.heap.table(list).get(Value::Num(i as f64)))
        .collect();
    let mut out = Vec::new();
    for category in categories {
        let Value::Num(id) = category else {
            continue;
        };
        let id = id as i64;
        let count = match recorded_first(vm, &format!("GetCategoryNumAchievements({id}, true)")) {
            Value::Num(c) => c as i64,
            _ => 0,
        };
        for i in 1..=count {
            let Some(info) =
                crate::recorded::lookup_text(vm, &format!("GetAchievementInfo({id}, {i})"))
            else {
                continue;
            };
            let (Some(Value::Num(achievement)), Some(name)) = (info.first(), info.get(1)) else {
                continue;
            };
            let name = text(vm, *name).to_lowercase();
            if (wanted.is_empty() || name.contains(&wanted)) && !out.contains(achievement) {
                out.push(*achievement);
            }
        }
    }
    out
}

fn set_achievement_search_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let wanted = text(vm, vm.hidden(c.arg(1)));
    let s = vm.str(&wanted);
    let native = vm.native_with("achievementSearch", searched as NativeFn, vec![s]);
    crate::clock::after_engine(vm, SEARCH_AFTER, native);
    Ok(vec![])
}

fn searched(vm: &mut Vm, c: NativeCall) -> Ret {
    let wanted = text(vm, c.upvals.first().copied().unwrap_or(Value::Nil));
    let results = search(vm, &wanted);
    engine_mut(vm).world.achievements.results = results;
    widgets::fire_event(vm, "ACHIEVEMENT_SEARCH_UPDATED", &[]);
    Ok(vec![])
}

fn get_num_filtered_achievements(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = engine(vm).world.achievements.results.len();
    Ok(vec![Value::Num(n as f64)])
}

fn get_filtered_achievement_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(i) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    let results = &engine(vm).world.achievements.results;
    Ok(match results.get((i as usize).wrapping_sub(1)) {
        Some(id) => vec![Value::Num(*id)],
        None => vec![],
    })
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("SetAchievementSearchString", set_achievement_search_string),
        ("GetNumFilteredAchievements", get_num_filtered_achievements),
        ("GetFilteredAchievementID", get_filtered_achievement_id),
    ]
}
