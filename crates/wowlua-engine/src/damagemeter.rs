//! The damage meter's sessions (`C_DamageMeter`): the engine keeps no
//! combat's numbers, so a session asked for is one with no sources
//! (`DamageMeterCombatSession`'s documented defaults: no `combatSources`,
//! `maxAmount` and `totalAmount` 0, no duration), and there are no
//! sessions to list. Unverified: the client's session before any fight,
//! and after one (where the player would be a source).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

/// A session without sources.
fn empty_session(vm: &mut Vm) -> Value {
    let t = vm.new_table();
    let sources = vm.new_table();
    for (k, v) in [
        ("combatSources", Value::Table(sources)),
        ("maxAmount", Value::Num(0.0)),
        ("totalAmount", Value::Num(0.0)),
    ] {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, v);
    }
    Value::Table(t)
}

/// `GetCombatSessionFromType(sessionType, type)`,
/// `GetCombatSessionFromID(sessionID, type)`. Secret in combat as
/// documented, but a session without sources has nothing to hide: told
/// plain, so its empty `combatSources` reads as empty (unverified; SuuUI,
/// error-free in the client, takes the length of a session's sources).
fn combat_session(vm: &mut Vm, _c: NativeCall) -> Ret {
    crate::engine_mut(vm).plain_results = true;
    Ok(vec![empty_session(vm)])
}

/// A source's spells (`GetCombatSessionSourceFromType`, `…FromID`): none.
fn combat_session_source(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = vm.new_table();
    let spells = vm.new_table();
    for (k, v) in [
        ("combatSpells", Value::Table(spells)),
        ("maxAmount", Value::Num(0.0)),
        ("totalAmount", Value::Num(0.0)),
    ] {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `GetAvailableCombatSessions()`: none kept.
fn available_sessions(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_DamageMeter.GetCombatSessionFromType", combat_session),
        ("C_DamageMeter.GetCombatSessionFromID", combat_session),
        (
            "C_DamageMeter.GetCombatSessionSourceFromType",
            combat_session_source,
        ),
        (
            "C_DamageMeter.GetCombatSessionSourceFromID",
            combat_session_source,
        ),
        (
            "C_DamageMeter.GetAvailableCombatSessions",
            available_sessions,
        ),
    ]
}
