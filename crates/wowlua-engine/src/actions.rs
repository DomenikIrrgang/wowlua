//! The action bars: the action in each slot (a world profile's `actions`,
//! recorded by `/stdtest snapshot`; `mock.setAction`), read through
//! `C_ActionBar` and `GetActionInfo` as the client answers (recorded,
//! `/stdtest engine answers`): slot 0 and below answer nothing; an empty
//! slot no texture, an empty count, a cooldown not enabled, not usable; a
//! filled slot its action, its texture and count, a cooldown enabled and
//! idle, usable as recorded (else usable; unverified). No cooldowns,
//! charges or loss of control run yet.

use std::collections::BTreeMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// An action in a slot.
#[derive(Clone, Debug, Default)]
pub struct Action {
    /// `spell`, `item`, `macro`, `companion`, `equipmentset`, …
    pub kind: String,
    pub id: Value,
    pub sub_type: Value,
    pub texture: Value,
    /// The count shown (`GetActionDisplayCount`), "" for none.
    pub count: String,
    /// `GetActionText` (a macro's name).
    pub text: Value,
    /// `IsUsableAction`: usable, lacking power.
    pub usable: (bool, bool),
}

/// The actions, by slot.
pub type Actions = BTreeMap<i64, Action>;

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// The slot asked about: `None` for slot 0 and below (no answer).
fn slot(c: &NativeCall) -> Option<i64> {
    num(c.arg(1)).filter(|n| *n >= 1.0).map(|n| n as i64)
}

fn action(vm: &Vm, slot: i64) -> Option<Action> {
    engine(vm).world.character.actions.get(&slot).cloned()
}

fn table(vm: &mut Vm, fields: &[(&str, Value)]) -> Value {
    let t = vm.new_table();
    for (k, v) in fields {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, *v);
    }
    Value::Table(t)
}

fn has_action(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => vec![Value::Bool(action(vm, s).is_some())],
        None => vec![],
    })
}

fn get_action_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c).and_then(|s| action(vm, s)) {
        Some(a) => vec![vm.str(&a.kind), a.id, a.sub_type],
        None => vec![],
    })
}

fn get_action_texture(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => vec![action(vm, s).map_or(Value::Nil, |a| a.texture)],
        None => vec![],
    })
}

fn get_action_text(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => vec![action(vm, s).map_or(Value::Nil, |a| a.text)],
        None => vec![],
    })
}

fn get_action_display_count(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => {
            let count = action(vm, s).map(|a| a.count).unwrap_or_default();
            vec![vm.str(&count)]
        }
        None => vec![],
    })
}

fn is_usable_action(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => {
            let (usable, no_power) = action(vm, s).map_or((false, false), |a| a.usable);
            vec![Value::Bool(usable), Value::Bool(no_power)]
        }
        None => vec![],
    })
}

/// `IsCurrentAction`, `IsAutoRepeatAction`, `IsEquippedAction`,
/// `UsesActionText`: false for a slot (none running; unverified for
/// equipped items' actions).
fn not_for_slot(_vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(_) => vec![Value::Bool(false)],
        None => vec![],
    })
}

/// `IsAssistedCombatAction`: false, even for slot 0 (recorded).
fn not_assisted(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn get_action_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(s) = slot(&c) else {
        return Ok(vec![]);
    };
    let enabled = action(vm, s).is_some();
    let t = table(
        vm,
        &[
            ("startTime", Value::Num(0.0)),
            ("duration", Value::Num(0.0)),
            ("isEnabled", Value::Bool(enabled)),
            ("modRate", Value::Num(1.0)),
            ("isActive", Value::Bool(false)),
        ],
    );
    Ok(vec![t])
}

fn get_action_charges(vm: &mut Vm, c: NativeCall) -> Ret {
    if slot(&c).is_none() {
        return Ok(vec![]);
    }
    let t = table(
        vm,
        &[
            ("currentCharges", Value::Num(0.0)),
            ("maxCharges", Value::Num(0.0)),
            ("cooldownStartTime", Value::Num(0.0)),
            ("cooldownDuration", Value::Num(0.0)),
            ("chargeModRate", Value::Num(1.0)),
            ("isActive", Value::Bool(false)),
        ],
    );
    Ok(vec![t])
}

fn get_loss_of_control_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    if slot(&c).is_none() {
        return Ok(vec![]);
    }
    let t = table(
        vm,
        &[
            ("startTime", Value::Num(0.0)),
            ("duration", Value::Num(0.0)),
            ("modRate", Value::Num(1.0)),
            ("isActive", Value::Bool(false)),
            ("shouldReplaceNormalCooldown", Value::Bool(false)),
        ],
    );
    Ok(vec![t])
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        Value::Num(n) => Some(format!("{n}")),
        _ => None,
    }
}

/// `mock.setAction(slot, action)`: the action in a slot (`nil`: none),
/// `{ type, id, subType, texture, count, text, usable }` (`usable`:
/// `{ usable, noPower }` or a boolean); `ACTIONBAR_SLOT_CHANGED`.
pub fn control_set_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = vm.check_int(&c, 1)?;
    let action = match c.arg(2) {
        Value::Nil => None,
        Value::Table(t) => {
            let kind = field(vm, t, "type");
            let Some(kind) = text(vm, kind) else {
                return Err(vm.error("setAction: the action's type expected"));
            };
            let usable = match field(vm, t, "usable") {
                Value::Bool(b) => (b, false),
                Value::Table(u) => (
                    matches!(vm.heap.table(u).get(Value::Num(1.0)), Value::Bool(true)),
                    matches!(vm.heap.table(u).get(Value::Num(2.0)), Value::Bool(true)),
                ),
                _ => (true, false),
            };
            let count = field(vm, t, "count");
            let id = field(vm, t, "id");
            let sub_type = field(vm, t, "subType");
            let texture = field(vm, t, "texture");
            let label = field(vm, t, "text");
            for v in [id, sub_type, texture, label] {
                widgets::keep(vm, v);
            }
            Some(Action {
                kind,
                id,
                sub_type,
                texture,
                count: text(vm, count).unwrap_or_default(),
                text: label,
                usable,
            })
        }
        _ => return Err(vm.error("setAction: a table of the action, or nil, expected")),
    };
    let actions = &mut engine_mut(vm).world.character.actions;
    match action {
        Some(a) => actions.insert(n, a),
        None => actions.remove(&n),
    };
    widgets::fire_event(vm, "ACTIONBAR_SLOT_CHANGED", &[Value::Num(n as f64)]);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_ActionBar.HasAction", has_action),
        ("HasAction", has_action),
        ("GetActionInfo", get_action_info),
        ("C_ActionBar.GetActionTexture", get_action_texture),
        ("GetActionTexture", get_action_texture),
        ("GetActionText", get_action_text),
        ("C_ActionBar.GetActionText", get_action_text),
        (
            "C_ActionBar.GetActionDisplayCount",
            get_action_display_count,
        ),
        ("C_ActionBar.IsUsableAction", is_usable_action),
        ("IsUsableAction", is_usable_action),
        ("C_ActionBar.IsCurrentAction", not_for_slot),
        ("C_ActionBar.IsAutoRepeatAction", not_for_slot),
        ("C_ActionBar.IsEquippedAction", not_for_slot),
        ("C_ActionBar.UsesActionText", not_for_slot),
        ("C_ActionBar.IsAssistedCombatAction", not_assisted),
        ("C_ActionBar.GetActionCooldown", get_action_cooldown),
        ("C_ActionBar.GetActionCharges", get_action_charges),
        (
            "C_ActionBar.GetActionLossOfControlCooldownInfo",
            get_loss_of_control_cooldown,
        ),
    ]
}
