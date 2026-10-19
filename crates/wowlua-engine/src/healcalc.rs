//! Heal prediction calculators (`CreateUnitHealPredictionCalculator`, the
//! documentation's `UnitHealPredictionCalculatorAPI`): objects
//! `UnitGetDetailedHealPrediction(unit, healer, calculator)` fills with a
//! unit's health, which addons' health bars read back.
//!
//! The engine's units have no absorbs and no incoming heals
//! (`UnitGetIncomingHeals`, `UnitGetTotalAbsorbs`: none), so those read 0,
//! never clamped. A unit's health is secret (`UnitHealth`: documented
//! `SecretReturns`; its absorbs recorded secret 0s), so a filled
//! calculator's amounts are secrets and `HasSecretValues` is true.
//! Unverified: the modes' defaults (the enumerations' first, 0), that a
//! calculator nothing filled reads plain 0s, which amounts stay plain.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::engine;
use crate::widgets::{self, MethodFn, WidgetId};

/// The class the calculators' methods are documented for.
const CLASS: &str = "UnitHealPredictionCalculator";

fn create(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![widgets::create(vm, CLASS, None, None)])
}

/// A number the calculator keeps (0 until filled or set).
fn kept(vm: &mut Vm, id: WidgetId, key: &str) -> f64 {
    let props = engine(vm).world.widgets[id.0].props;
    match widgets::stored_args(vm, props, key).and_then(|v| v.first().copied()) {
        Some(Value::Num(n)) => n,
        _ => 0.0,
    }
}

/// Whether a unit filled the calculator (its amounts are secrets then).
fn filled(vm: &mut Vm, id: WidgetId) -> bool {
    let props = engine(vm).world.widgets[id.0].props;
    widgets::stored_args(vm, props, "\u{0}filled").is_some()
}

/// An amount as the calculator gives it: a secret once filled.
fn amount(vm: &mut Vm, id: WidgetId, n: f64) -> Value {
    if filled(vm, id) {
        vm.secret_wrap(Value::Num(n))
    } else {
        Value::Num(n)
    }
}

/// `UnitGetDetailedHealPrediction(unit, healer, calculator)`: the unit's
/// health and maximum kept in the calculator (nothing for a unit that
/// does not exist: unverified).
fn detailed_heal_prediction(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(calc) = widgets::widget_of(vm, c.arg(3)) else {
        return Ok(vec![]);
    };
    let (health, max) = match crate::units::unit_arg(vm, &c, 1) {
        Some(u) => {
            let u = &engine(vm).units.list[u.0];
            (u.health, u.health_max)
        }
        None => (0.0, 0.0),
    };
    widgets::store(vm, calc, "\u{0}health", &[Value::Num(health)]);
    widgets::store(vm, calc, "\u{0}healthMax", &[Value::Num(max)]);
    widgets::store(vm, calc, "\u{0}filled", &[Value::Bool(true)]);
    Ok(vec![])
}

fn current_health(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let n = kept(vm, id, "\u{0}health");
    Ok(vec![amount(vm, id, n)])
}

fn maximum_health(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let n = kept(vm, id, "\u{0}healthMax");
    Ok(vec![amount(vm, id, n)])
}

fn missing_health(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let missing = kept(vm, id, "\u{0}healthMax") - kept(vm, id, "\u{0}health");
    Ok(vec![amount(vm, id, missing.max(0.0))])
}

/// The health as a fraction of the maximum, as `UnitHealthPercent`'s.
fn fraction(vm: &mut Vm, id: WidgetId) -> f64 {
    let max = kept(vm, id, "\u{0}healthMax");
    if max > 0.0 {
        kept(vm, id, "\u{0}health") / max
    } else {
        0.0
    }
}

fn current_percent(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let f = fraction(vm, id);
    Ok(vec![amount(vm, id, f)])
}

fn missing_percent(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let f = 1.0 - fraction(vm, id);
    Ok(vec![amount(vm, id, f)])
}

/// A curve's value for the fraction, secret as the fraction is.
fn evaluate(vm: &mut Vm, id: WidgetId, curve: Value, f: f64) -> Ret {
    let v = crate::curves::apply(vm, curve, f)?;
    Ok(vec![if filled(vm, id) { vm.secret_wrap(v) } else { v }])
}

fn evaluate_current(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let f = fraction(vm, id);
    evaluate(vm, id, c.arg(2), f)
}

fn evaluate_missing(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let f = 1.0 - fraction(vm, id);
    evaluate(vm, id, c.arg(2), f)
}

/// No absorbs: the amount 0, not clamped.
fn none_clamped(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![amount(vm, id, 0.0), Value::Bool(false)])
}

/// No incoming heals: all, the healer's and the others' 0, not clamped.
fn no_incoming(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![
        amount(vm, id, 0.0),
        amount(vm, id, 0.0),
        amount(vm, id, 0.0),
        Value::Bool(false),
    ])
}

fn zero(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![amount(vm, id, 0.0)])
}

fn has_secret_values(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(filled(vm, id))])
}

/// A mode's getter: what its setter set, else 0.
fn mode(vm: &mut Vm, id: WidgetId, stem: &str) -> Ret {
    Ok(vec![Value::Num(kept(vm, id, stem))])
}

fn max_health_mode(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    mode(vm, id, "MaximumHealthMode")
}

fn damage_absorb_clamp_mode(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    mode(vm, id, "DamageAbsorbClampMode")
}

fn heal_absorb_clamp_mode(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    mode(vm, id, "HealAbsorbClampMode")
}

fn heal_absorb_mode(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    mode(vm, id, "HealAbsorbMode")
}

fn incoming_heal_clamp_mode(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    mode(vm, id, "IncomingHealClampMode")
}

fn incoming_heal_overflow(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    mode(vm, id, "IncomingHealOverflowPercent")
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("CreateUnitHealPredictionCalculator", create),
        ("UnitGetDetailedHealPrediction", detailed_heal_prediction),
    ]
}

pub fn methods() -> Vec<(&'static str, MethodFn)> {
    let list: [(&str, MethodFn); 21] = [
        ("GetCurrentHealth", current_health),
        ("GetMaximumHealth", maximum_health),
        ("GetMissingHealth", missing_health),
        ("GetCurrentHealthPercent", current_percent),
        ("GetMissingHealthPercent", missing_percent),
        ("EvaluateCurrentHealthPercent", evaluate_current),
        ("EvaluateMissingHealthPercent", evaluate_missing),
        ("GetDamageAbsorbs", none_clamped),
        ("GetHealAbsorbs", none_clamped),
        ("GetIncomingHeals", no_incoming),
        ("GetMaximumDamageAbsorbs", zero),
        ("GetMaximumHealAbsorbs", zero),
        ("GetMaximumIncomingHeals", zero),
        ("GetTotalDamageAbsorbs", zero),
        ("GetTotalHealAbsorbs", zero),
        ("GetTotalIncomingHeals", zero),
        ("GetTotalIncomingHealsFromHealer", zero),
        ("HasSecretValues", has_secret_values),
        ("GetMaximumHealthMode", max_health_mode),
        ("GetDamageAbsorbClampMode", damage_absorb_clamp_mode),
        ("GetHealAbsorbClampMode", heal_absorb_clamp_mode),
    ];
    let more: [(&str, MethodFn); 3] = [
        ("GetHealAbsorbMode", heal_absorb_mode),
        ("GetIncomingHealClampMode", incoming_heal_clamp_mode),
        ("GetIncomingHealOverflowPercent", incoming_heal_overflow),
    ];
    list.into_iter()
        .chain(more)
        .map(|(short, f)| {
            let full: &'static str = Box::leak(format!("{CLASS}:{short}").into_boxed_str());
            (full, f)
        })
        .collect()
}
