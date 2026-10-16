//! Protected actions (spec emulator §17.4, decision T197): what tainted code
//! may not do. A protected function called from tainted execution does
//! nothing and fires `ADDON_ACTION_FORBIDDEN`; a protected method of a
//! protected frame does the same in combat lockdown with
//! `ADDON_ACTION_BLOCKED`. Both payloads are the tainting addon's name and
//! the action as Lua's debug information names the call (`UseAction()`
//! from `UseAction(…)`, `UNKNOWN()` through `pcall`; `/stdtest taint` and
//! `engine`); the call returns its documented results' empty values
//! (`false` from `ClearTarget`). Each refusal is kept with its taint
//! report, for the test it fails.
//!
//! Protected methods are the documentation's (`IsProtectedFunction`); the
//! documentation does not mark global functions, so [`PROTECTED`] lists the
//! ones known to be protected. `/stdtest taint` recorded a global's payload
//! (`"StdTest", "UseAction()"`); unverified until `/stdtest engine
//! protected` and `blocked` run: the list, a method's payload, that a
//! refusal returns nothing without an error and comes before the argument
//! checks, and that the event reaches handlers before the call returns.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::widgets::{self, WidgetId};
use crate::{engine, engine_mut};

/// Global functions only Blizzard's code may call (not in the
/// documentation's flags). `/stdtest engine protected` saw TargetUnit,
/// ClearTarget, FocusUnit, AssistUnit, SpellStopCasting,
/// SpellStopTargeting, MoveForwardStop, TurnLeftStop, StrafeLeftStop and
/// PetFollow forbidden, ClearFocus, TargetLastTarget and StopAttack not;
/// CastSpellByName with an unknown spell fired nothing (kept, unverified),
/// and the rest are unverified.
pub const PROTECTED: &[&str] = &[
    "AssistUnit",
    "AttackTarget",
    "CameraOrSelectOrMoveStart",
    "CameraOrSelectOrMoveStop",
    "CancelShapeshiftForm",
    "CastPetAction",
    "CastShapeshiftForm",
    "CastSpell",
    "CastSpellByID",
    "CastSpellByName",
    "ClearTarget",
    "C_Container.UseContainerItem",
    "FocusUnit",
    "JumpOrAscendStart",
    "MoveBackwardStart",
    "MoveBackwardStop",
    "MoveForwardStart",
    "MoveForwardStop",
    "PetAttack",
    "PetFollow",
    "RunMacro",
    "RunMacroText",
    "SpellStopCasting",
    "SpellStopTargeting",
    "SpellTargetUnit",
    "StartAttack",
    "StrafeLeftStart",
    "StrafeLeftStop",
    "StrafeRightStart",
    "StrafeRightStop",
    "TargetLastEnemy",
    "TargetNearestEnemy",
    "TargetNearestFriend",
    "TargetUnit",
    "ToggleAutoRun",
    "ToggleRun",
    "TurnLeftStart",
    "TurnLeftStop",
    "TurnRightStart",
    "TurnRightStop",
    "UseAction",
    "UseContainerItem",
    "UseInventoryItem",
    "UseItemByName",
];

/// A refused protected action.
pub struct Refusal {
    pub event: &'static str,
    pub addon: String,
    pub action: String,
    /// The taint chain from the refused call back to the addon's code.
    pub report: String,
}

/// Whether the running code is tainted.
pub fn tainted(vm: &Vm) -> bool {
    !vm.taint().stack.is_secure()
}

/// Whether the function calling the engine is Blizzard's (secure) code,
/// tainted execution or not: its gaps get empty values, not errors.
pub fn blizzard_caller(vm: &Vm) -> bool {
    // Called by a C function (`securecallfunction(DropCursorMoney)`): by
    // whoever runs, untainted execution being Blizzard's.
    vm.caller_is_secure(1) || (vm.caller_source(1).is_none() && !tainted(vm))
}

/// Whether a global function is protected.
pub fn is_protected(name: &str, doc: Option<&serde_json::Value>) -> bool {
    PROTECTED.contains(&name)
        || doc
            .and_then(|d| d.get("IsProtectedFunction"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

/// The refused action's name: how the call site names the function, else
/// `UNKNOWN`, with `()`.
pub fn action_name(vm: &Vm) -> String {
    format!("{}()", vm.called_as().unwrap_or_else(|| "UNKNOWN".into()))
}

/// Refuses `action`: keeps the refusal and fires `event` with the
/// tainting addon and the action.
pub fn refuse(vm: &mut Vm, event: &'static str, action: String) {
    let addon = vm.taint_name(vm.taint().stack).unwrap_or_default();
    let report = vm.execution_report();
    engine_mut(vm).refusals.push(Refusal {
        event,
        addon: addon.clone(),
        action: action.clone(),
        report,
    });
    let a = vm.str(&addon);
    let f = vm.str(&action);
    widgets::fire_event(vm, event, &[a, f]);
}

/// A protected method of `id` refused: in combat lockdown, on a protected
/// widget, for tainted code. The action names the widget by its name, or
/// its type when it has none (unverified).
pub fn refuses_method(vm: &mut Vm, id: WidgetId, f: &crate::api::Function) -> bool {
    // `SetAttribute` too (`/stdtest engine blocked`).
    let protected_method = f
        .doc
        .get("IsProtectedFunction")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        || f.short == "SetAttribute";
    let e = engine(vm);
    let w = &e.world.widgets[id.0];
    if !(protected_method && w.protected && e.units.lockdown && tainted(vm)) {
        return false;
    }
    let action = action_name(vm);
    refuse(vm, "ADDON_ACTION_BLOCKED", action);
    true
}

/// A protected global function the documentation lacks: refused for
/// tainted code; otherwise its behaviour, or not mocked.
pub fn protected_global(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Str(name)) = c.upvals.first().copied() else {
        return Err(vm.error("a protected function without its name"));
    };
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    // A spell the player does not have fails before the protection
    // (`/stdtest engine protected`: no event for an unknown spell).
    if name == "CastSpellByName" {
        let known = match c.arg(1) {
            Value::Str(s) => {
                let spell = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
                let spells: Vec<wowlua_vm::TableRef> =
                    engine(vm).world.data.spells.values().copied().collect();
                spells.into_iter().any(|t| {
                    let k = vm.str("name");
                    matches!(vm.heap.table(t).get(k), Value::Str(n) if vm.str_bytes(n) == spell.as_bytes())
                })
            }
            _ => false,
        };
        if !known {
            return Ok(vec![]);
        }
    }
    if tainted(vm) {
        let action = action_name(vm);
        refuse(vm, "ADDON_ACTION_FORBIDDEN", action);
        // What the recording saw these return when refused.
        if matches!(name.as_str(), "SpellStopCasting" | "SpellStopTargeting") {
            return Ok(vec![Value::Bool(false)]);
        }
        return Ok(vec![]);
    }
    match engine(vm).behaviour(&name) {
        Some(b) => b(vm, c),
        // Secure code's protected call without behaviour does nothing.
        None => Ok(vec![]),
    }
}

/// `refusals()`: the refused actions so far, as tables (`event`, `addon`,
/// `action`, `report`).
pub fn refusals_control(vm: &mut Vm, _c: NativeCall) -> Ret {
    let list: Vec<[String; 4]> = engine(vm)
        .refusals
        .iter()
        .map(|r| {
            [
                r.event.to_string(),
                r.addon.clone(),
                r.action.clone(),
                r.report.clone(),
            ]
        })
        .collect();
    let out = vm.new_table();
    for (i, fields) in list.into_iter().enumerate() {
        let t = vm.new_table();
        for (key, value) in ["event", "addon", "action", "report"].iter().zip(fields) {
            let k = vm.str(key);
            let v = vm.str(&value);
            let _ = vm.heap.table_mut(t).set(k, v);
        }
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num(i as f64 + 1.0), Value::Table(t));
    }
    Ok(vec![Value::Table(out)])
}
