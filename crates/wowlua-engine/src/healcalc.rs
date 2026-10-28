//! Heal prediction calculators (`CreateUnitHealPredictionCalculator`, the
//! documentation's `UnitHealPredictionCalculatorAPI`): objects
//! `UnitGetDetailedHealPrediction(unit, healer, calculator)` fills with a
//! unit's health, which addons' health bars read back.
//!
//! A unit fills it with its health, its damage and heal absorbs and the
//! heals being cast on it (`setAbsorb`, `setHealAbsorb`, `setIncomingHeal`;
//! `UnitGetTotalAbsorbs` and the others answer the same), and the getters
//! work out the amounts by the documented modes: heal absorbs and
//! incoming heals reduce each other (`ReducedByIncomingHeals`) or not,
//! then each is clamped to its boundary, `clamped` telling an amount past
//! it. A unit's health is secret (`UnitHealth`: documented
//! `SecretReturns`; its absorbs recorded secret 0s), so a filled
//! calculator's amounts and `clamped` are secrets and `HasSecretValues` is
//! true. Unverified: the modes' defaults (the enumerations' first, 0; the
//! overflow 1), the order of the reductions
//! and clamps, that a calculator nothing filled reads plain 0s.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::engine;
use crate::units::UnitId;
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

/// The heals being cast on a unit: all of them, or the healer's
/// (`all` false and no healer: none).
pub(crate) fn incoming(vm: &Vm, id: UnitId, healer: Option<UnitId>, all: bool) -> f64 {
    engine(vm).units.list[id.0]
        .incoming_heals
        .iter()
        .filter(|(h, _)| all || Some(*h) == healer)
        .map(|(_, n)| n)
        .sum()
}

/// `UnitGetDetailedHealPrediction(unit, healer, calculator)`: the unit's
/// health, maximum, absorbs and incoming heals kept in the calculator,
/// the healer's heals 0 without a healer (documented; nothing for a unit
/// that does not exist: unverified).
fn detailed_heal_prediction(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(calc) = widgets::widget_of(vm, c.arg(3)) else {
        return Ok(vec![]);
    };
    let values = match crate::units::unit_arg(vm, &c, 1) {
        Some(id) => {
            let healer = crate::units::unit_arg(vm, &c, 2);
            let u = &engine(vm).units.list[id.0];
            let (health, max, absorbs, heal_absorbs) =
                (u.health, u.health_max, u.absorbs, u.heal_absorbs);
            [
                health,
                max,
                incoming(vm, id, None, true),
                incoming(vm, id, healer, false),
                absorbs,
                heal_absorbs,
            ]
        }
        None => [0.0; 6],
    };
    for (key, n) in KEPT.iter().zip(values) {
        widgets::store(vm, calc, key, &[Value::Num(n)]);
    }
    widgets::store(vm, calc, "\u{0}filled", &[Value::Bool(true)]);
    Ok(vec![])
}

/// What a unit fills in: health, maximum, incoming heals (all, the
/// healer's), damage absorbs, heal absorbs (where `SetPredictedValues`
/// keeps them too: objects2.rs).
const KEPT: [&str; 6] = [
    "\u{0}health",
    "\u{0}healthMax",
    "\u{0}totalIncomingHeals",
    "\u{0}totalIncomingHealsFromHealer",
    "\u{0}totalDamageAbsorbs",
    "\u{0}totalHealAbsorbs",
];

/// The amounts the getters give, by the calculator's modes, before their
/// clamps: each with its boundary.
struct Worked {
    damage_absorbs: f64,
    damage_limit: f64,
    heal_absorbs: f64,
    heal_limit: f64,
    incoming: f64,
    incoming_healer: f64,
    incoming_limit: f64,
}

fn worked(vm: &mut Vm, id: WidgetId) -> Worked {
    let health = kept(vm, id, "\u{0}health");
    let mut max = kept(vm, id, "\u{0}healthMax");
    let absorbs = kept(vm, id, "\u{0}totalDamageAbsorbs");
    let (mut heal_absorbs, mut incoming) = (
        kept(vm, id, "\u{0}totalHealAbsorbs"),
        kept(vm, id, "\u{0}totalIncomingHeals"),
    );
    // UnitMaximumHealthMode.WithAbsorbs: the boundaries count the shields.
    if kept(vm, id, "MaximumHealthMode") == 1.0 {
        max += absorbs;
    }
    // UnitHealAbsorbMode.ReducedByIncomingHeals.
    if kept(vm, id, "HealAbsorbMode") == 0.0 {
        (heal_absorbs, incoming) = (
            (heal_absorbs - incoming).max(0.0),
            (incoming - heal_absorbs).max(0.0),
        );
    }
    let overflow = overflow(vm, id);
    let incoming_limit = match kept(vm, id, "IncomingHealClampMode") {
        1.0 => max,
        _ => (max * overflow - health).max(0.0),
    };
    let incoming_healer = kept(vm, id, "\u{0}totalIncomingHealsFromHealer");
    let damage_limit = match kept(vm, id, "DamageAbsorbClampMode") {
        2.0 => max,
        1.0 => (max - health).max(0.0),
        _ => (max - health - incoming.min(incoming_limit)).max(0.0),
    };
    let heal_limit = match kept(vm, id, "HealAbsorbClampMode") {
        1.0 => max,
        _ => health,
    };
    Worked {
        damage_absorbs: absorbs,
        damage_limit,
        heal_absorbs,
        heal_limit,
        incoming,
        incoming_healer,
        incoming_limit,
    }
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

/// An amount clamped to its boundary, and whether it went past it.
fn clamp(vm: &mut Vm, id: WidgetId, n: f64, limit: f64) -> [Value; 2] {
    let clamped = Value::Bool(n > limit);
    [
        amount(vm, id, n.min(limit)),
        if filled(vm, id) {
            vm.secret_wrap(clamped)
        } else {
            clamped
        },
    ]
}

fn damage_absorbs(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let w = worked(vm, id);
    Ok(clamp(vm, id, w.damage_absorbs, w.damage_limit).to_vec())
}

fn heal_absorbs(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let w = worked(vm, id);
    Ok(clamp(vm, id, w.heal_absorbs, w.heal_limit).to_vec())
}

/// All incoming heals, the healer's (never more than all) and the
/// others', clamped.
fn incoming_heals(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let w = worked(vm, id);
    let all = w.incoming.min(w.incoming_limit);
    let healer = w.incoming_healer.min(all);
    let [amount_all, clamped] = clamp(vm, id, w.incoming, w.incoming_limit);
    Ok(vec![
        amount_all,
        amount(vm, id, healer),
        amount(vm, id, all - healer),
        clamped,
    ])
}

/// A getter of one number.
fn number(vm: &mut Vm, id: WidgetId, n: impl FnOnce(&mut Vm, WidgetId) -> f64) -> Ret {
    let n = n(vm, id);
    Ok(vec![amount(vm, id, n)])
}

fn maximum_damage_absorbs(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    number(vm, id, |vm, id| worked(vm, id).damage_limit)
}

fn maximum_heal_absorbs(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    number(vm, id, |vm, id| worked(vm, id).heal_limit)
}

fn maximum_incoming_heals(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    number(vm, id, |vm, id| worked(vm, id).incoming_limit)
}

fn total_damage_absorbs(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    number(vm, id, |vm, id| kept(vm, id, "\u{0}totalDamageAbsorbs"))
}

fn total_heal_absorbs(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    number(vm, id, |vm, id| kept(vm, id, "\u{0}totalHealAbsorbs"))
}

fn total_incoming_heals(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    number(vm, id, |vm, id| kept(vm, id, "\u{0}totalIncomingHeals"))
}

fn total_incoming_heals_from_healer(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    number(vm, id, |vm, id| {
        kept(vm, id, "\u{0}totalIncomingHealsFromHealer")
    })
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

/// The incoming heals' overflow: what was set, else 1 (unverified).
fn incoming_heal_overflow(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(overflow(vm, id))])
}

fn overflow(vm: &mut Vm, id: WidgetId) -> f64 {
    let props = engine(vm).world.widgets[id.0].props;
    match widgets::stored_args(vm, props, "IncomingHealOverflowPercent")
        .and_then(|v| v.first().copied())
    {
        Some(Value::Num(n)) => n,
        _ => 1.0,
    }
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
        ("GetDamageAbsorbs", damage_absorbs),
        ("GetHealAbsorbs", heal_absorbs),
        ("GetIncomingHeals", incoming_heals),
        ("GetMaximumDamageAbsorbs", maximum_damage_absorbs),
        ("GetMaximumHealAbsorbs", maximum_heal_absorbs),
        ("GetMaximumIncomingHeals", maximum_incoming_heals),
        ("GetTotalDamageAbsorbs", total_damage_absorbs),
        ("GetTotalHealAbsorbs", total_heal_absorbs),
        ("GetTotalIncomingHeals", total_incoming_heals),
        (
            "GetTotalIncomingHealsFromHealer",
            total_incoming_heals_from_healer,
        ),
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
