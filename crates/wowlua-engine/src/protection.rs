//! Protected actions (spec emulator §17.4, decision T197): what tainted code
//! may not do. A protected function called from tainted execution does
//! nothing and fires `ADDON_ACTION_FORBIDDEN`; a protected method of a
//! protected frame does the same in combat lockdown with
//! `ADDON_ACTION_BLOCKED`. Both payloads are the tainting addon's name and
//! the action (`TargetUnit()`, `MyButton:Show()`); each refusal is kept
//! with its taint report, for the test it fails.
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
/// documentation's flags; unverified, see the module).
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
    "ClearFocus",
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
    "StopAttack",
    "StrafeLeftStart",
    "StrafeLeftStop",
    "StrafeRightStart",
    "StrafeRightStop",
    "TargetLastEnemy",
    "TargetLastTarget",
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

/// Whether a global function is protected.
pub fn is_protected(name: &str, doc: Option<&serde_json::Value>) -> bool {
    PROTECTED.contains(&name)
        || doc
            .and_then(|d| d.get("IsProtectedFunction"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
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
    let protected_method = f
        .doc
        .get("IsProtectedFunction")
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    let e = engine(vm);
    let w = &e.world.widgets[id.0];
    if !(protected_method && w.protected && e.units.lockdown && tainted(vm)) {
        return false;
    }
    let who = w.name.clone().unwrap_or_else(|| w.class.clone());
    refuse(vm, "ADDON_ACTION_BLOCKED", format!("{who}:{}()", f.short));
    true
}

/// A protected global function the documentation lacks: refused for
/// tainted code; otherwise its behaviour, or not mocked.
pub fn protected_global(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Str(name)) = c.upvals.first().copied() else {
        return Err(vm.error("a protected function without its name"));
    };
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    if tainted(vm) {
        refuse(vm, "ADDON_ACTION_FORBIDDEN", format!("{name}()"));
        return Ok(vec![]);
    }
    match engine(vm).behaviour(&name) {
        Some(b) => b(vm, c),
        None => Err(vm.error(&format!("{name} is not mocked"))),
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
