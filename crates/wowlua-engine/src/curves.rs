//! Curve objects (`C_CurveUtil.CreateCurve`): points a value is mapped
//! through, as the client's `CurveObject` (linear between points, the end
//! points' values outside them; unverified for other curve types).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{LuaError, NativeCall, TableRef, Value, Vm};

use crate::engine;
use crate::widgets::{self, WidgetId};

fn create_curve(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![widgets::create(vm, "CurveObject", None, None)])
}

/// `C_StringUtil.CreateSecondsFormatter()`: a `SecondsFormatter`, its
/// setters storing and getters reading back (`Format` is not mocked).
fn create_seconds_formatter(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![widgets::create(vm, "SecondsFormatter", None, None)])
}

/// A curve's points, kept in its stored values (`points`: x1, y1, x2, …).
fn points_table(vm: &mut Vm, id: WidgetId) -> TableRef {
    let props = engine(vm).world.widgets[id.0].props;
    let key = vm.str("points");
    if let Value::Table(t) = vm.heap.table(props).get(key) {
        return t;
    }
    let t = vm.new_table();
    let _ = vm.heap.table_mut(props).set(key, Value::Table(t));
    t
}

fn points(vm: &mut Vm, id: WidgetId) -> Vec<(f64, f64)> {
    let t = points_table(vm, id);
    let n = vm.heap.table(t).border();
    let mut out: Vec<(f64, f64)> = (0..n / 2)
        .filter_map(|i| {
            let x = vm.heap.table(t).get(Value::Num((2 * i + 1) as f64));
            let y = vm.heap.table(t).get(Value::Num((2 * i + 2) as f64));
            match (x, y) {
                (Value::Num(x), Value::Num(y)) => Some((x, y)),
                _ => None,
            }
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

fn add_point(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let x = vm.check_number(&c, 2)?;
    let y = vm.check_number(&c, 3)?;
    let t = points_table(vm, id);
    let n = vm.heap.table(t).border() as f64;
    let _ = vm.heap.table_mut(t).set(Value::Num(n + 1.0), Value::Num(x));
    let _ = vm.heap.table_mut(t).set(Value::Num(n + 2.0), Value::Num(y));
    Ok(vec![])
}

fn clear_points(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let key = vm.str("points");
    let _ = vm.heap.table_mut(props).set(key, Value::Nil);
    Ok(vec![])
}

fn point_count(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(points(vm, id).len() as f64)])
}

/// The curve's value at `x`: linear between its points, its end points'
/// values outside them, `x` itself without points.
pub fn evaluate(vm: &mut Vm, id: WidgetId, x: f64) -> f64 {
    let p = points(vm, id);
    let (Some(first), Some(last)) = (p.first(), p.last()) else {
        return x;
    };
    if x <= first.0 {
        return first.1;
    }
    if x >= last.0 {
        return last.1;
    }
    for w in p.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x >= x0 && x <= x1 {
            return if x1 == x0 {
                y1
            } else {
                y0 + (y1 - y0) * (x - x0) / (x1 - x0)
            };
        }
    }
    last.1
}

fn evaluate_method(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let x = vm.check_number(&c, 2)?;
    Ok(vec![Value::Num(evaluate(vm, id, x))])
}

/// `value` through `curve` when one is given.
pub fn apply(vm: &mut Vm, curve: Value, value: f64) -> Result<Value, LuaError> {
    match curve {
        Value::Nil => Ok(Value::Num(value)),
        v => match widgets::widget_of(vm, v) {
            Some(id) => Ok(Value::Num(evaluate(vm, id, value))),
            None => Err(vm.error("a curve object expected")),
        },
    }
}

// -------------------------------------------------------------- durations

/// `C_DurationUtil.CreateDuration()`: a zero `DurationObject`, a time span
/// on the clock that keeps secret values secret (Blizzard's aura buttons
/// set theirs from `secretwrap`ped aura times). Unverified: the
/// `modifier`s (real time is used), the evaluated results' shape (the
/// curve's value).
fn create_duration(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![widgets::create(vm, "DurationObject", None, None)])
}

/// A duration's span: start, end (the clock's seconds), mod rate, secret.
#[derive(Clone, Copy, Default)]
struct Span {
    start: f64,
    end: f64,
    rate: f64,
    secret: bool,
}

fn span(vm: &mut Vm, id: WidgetId) -> Span {
    let props = engine(vm).world.widgets[id.0].props;
    let get = |vm: &mut Vm, k: &str| {
        let key = vm.str(k);
        vm.heap.table(props).get(key)
    };
    let num = |v: Value| match v {
        Value::Num(n) => n,
        _ => 0.0,
    };
    Span {
        start: num(get(vm, "start")),
        end: num(get(vm, "end")),
        rate: match get(vm, "modRate") {
            Value::Num(n) => n,
            _ => 1.0,
        },
        secret: get(vm, "secret").truthy(),
    }
}

fn set_span(vm: &mut Vm, id: WidgetId, s: Span) {
    let props = engine(vm).world.widgets[id.0].props;
    for (k, v) in [
        ("start", Value::Num(s.start)),
        ("end", Value::Num(s.end)),
        ("modRate", Value::Num(s.rate)),
        ("secret", Value::Bool(s.secret)),
    ] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(props).set(key, v);
    }
}

/// Number argument `n`, secret or not (`None`: absent): its value and
/// whether it was secret.
fn time_arg(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<Option<(f64, bool)>, LuaError> {
    let v = c.arg(n);
    let secret = matches!(v, Value::Secret(_));
    match vm.hidden(v) {
        Value::Num(x) => Ok(Some((x, secret))),
        Value::Nil => Ok(None),
        _ => Err(vm.error("a number expected")),
    }
}

/// `SetTimeFromEnd(end, duration, rate?)`, `SetTimeFromStart(start,
/// duration, rate?)`, `SetTimeSpan(start, end)`.
fn set_time(vm: &mut Vm, id: WidgetId, c: NativeCall, kind: u8) -> Ret {
    let (Some((a, sa)), Some((b, sb))) = (time_arg(vm, &c, 2)?, time_arg(vm, &c, 3)?) else {
        return Err(vm.error("two times expected"));
    };
    let rate = time_arg(vm, &c, 4)?;
    let secret = sa || sb || rate.is_some_and(|r| r.1);
    let (start, end) = match kind {
        0 => (a - b, a),
        1 => (a, a + b),
        _ => (a, b),
    };
    let rate = rate.map_or(1.0, |r| r.0);
    set_span(
        vm,
        id,
        Span {
            start,
            end,
            rate,
            secret,
        },
    );
    Ok(vec![])
}

fn set_time_from_end(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_time(vm, id, c, 0)
}

fn set_time_from_start(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_time(vm, id, c, 1)
}

fn set_time_span(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_time(vm, id, c, 2)
}

fn reset(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    set_span(
        vm,
        id,
        Span {
            rate: 1.0,
            ..Span::default()
        },
    );
    Ok(vec![])
}

fn assign(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some(other) = widgets::widget_of(vm, c.arg(2)) else {
        return Err(vm.error("a duration object expected"));
    };
    let s = span(vm, other);
    set_span(vm, id, s);
    Ok(vec![])
}

fn copy(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let s = span(vm, id);
    let new = widgets::create(vm, "DurationObject", None, None);
    if let Some(n) = widgets::widget_of(vm, new) {
        set_span(vm, n, s);
    }
    Ok(vec![new])
}

/// The method called: a documented one's index, else its full name, is
/// the native's upvalue.
fn method_name(vm: &Vm, c: &NativeCall) -> String {
    match c.upvals.first().copied() {
        Some(Value::Num(i)) => engine(vm)
            .api
            .methods
            .get(i as usize)
            .map_or_else(String::new, |f| f.short.to_string()),
        Some(Value::Str(s)) => {
            let full = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            full.rsplit(':').next().unwrap_or_default().to_string()
        }
        _ => String::new(),
    }
}

/// A value the duration gives: secret when it holds secrets.
fn out(vm: &mut Vm, s: Span, v: Value) -> Value {
    if s.secret { vm.secret_wrap(v) } else { v }
}

/// The span's measures now: (total, elapsed, remaining).
fn measures(vm: &mut Vm, s: Span) -> (f64, f64, f64) {
    let now = crate::clock::now(vm);
    let total = (s.end - s.start).max(0.0);
    let elapsed = (now - s.start).clamp(0.0, total);
    (total, elapsed, total - elapsed)
}

fn duration_get(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let which = method_name(vm, &c);
    let s = span(vm, id);
    let (total, elapsed, remaining) = measures(vm, s);
    let percent = |x: f64| if total > 0.0 { x / total } else { 0.0 };
    let now = crate::clock::now(vm);
    let v = match which.as_str() {
        "GetStartTime" => Value::Num(s.start),
        "GetEndTime" => Value::Num(s.end),
        "GetTotalDuration" => Value::Num(total),
        "GetElapsedDuration" => Value::Num(elapsed),
        "GetRemainingDuration" => Value::Num(remaining),
        "GetElapsedPercent" => Value::Num(percent(elapsed)),
        "GetRemainingPercent" => Value::Num(percent(remaining)),
        "GetModRate" => Value::Num(s.rate),
        "IsZero" => Value::Bool(s.start == 0.0 && s.end == 0.0),
        "HasExpired" => Value::Bool(total > 0.0 && now >= s.end),
        "HasStarted" => Value::Bool(now >= s.start),
        "IsActive" => Value::Bool(total > 0.0 && now >= s.start && now < s.end),
        // Not about the span: never secret.
        "HasSecretValues" => return Ok(vec![Value::Bool(s.secret)]),
        "GetClockTime" => return Ok(vec![Value::Num(now)]),
        _ => return Ok(vec![]),
    };
    Ok(vec![out(vm, s, v)])
}

/// `Evaluate…(curve)`: the measure through the curve.
fn duration_evaluate(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let which = method_name(vm, &c);
    let s = span(vm, id);
    let (total, elapsed, remaining) = measures(vm, s);
    let percent = |x: f64| if total > 0.0 { x / total } else { 0.0 };
    let x = match which.as_str() {
        "EvaluateElapsedDuration" => elapsed,
        "EvaluateElapsedPercent" => percent(elapsed),
        "EvaluateRemainingDuration" => remaining,
        "EvaluateRemainingPercent" => percent(remaining),
        _ => total,
    };
    let v = apply(vm, c.arg(2), x)?;
    Ok(vec![out(vm, s, v)])
}

fn update_memory(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// The memory in KiB; the VM does not tell addons apart (unverified: the
/// whole VM's).
fn addon_memory(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(vm.heap.estimate_bytes() as f64 / 1024.0)])
}

pub fn behaviours() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        ("C_CurveUtil.CreateCurve", create_curve),
        ("C_DurationUtil.CreateDuration", create_duration),
        (
            "C_StringUtil.CreateSecondsFormatter",
            create_seconds_formatter,
        ),
        ("UpdateAddOnMemoryUsage", update_memory),
        ("GetAddOnMemoryUsage", addon_memory),
    ]
}

pub fn methods() -> Vec<(&'static str, widgets::MethodFn)> {
    vec![
        ("CurveObject:AddPoint", add_point),
        ("CurveObject:ClearPoints", clear_points),
        ("CurveObject:GetPointCount", point_count),
        ("CurveObject:Evaluate", evaluate_method),
        ("DurationObject:SetTimeFromEnd", set_time_from_end),
        ("DurationObject:SetTimeFromStart", set_time_from_start),
        ("DurationObject:SetTimeSpan", set_time_span),
        ("DurationObject:Reset", reset),
        ("DurationObject:SetToDefaults", reset),
        ("DurationObject:Assign", assign),
        ("DurationObject:Copy", copy),
        ("DurationObject:GetStartTime", duration_get),
        ("DurationObject:GetEndTime", duration_get),
        ("DurationObject:GetTotalDuration", duration_get),
        ("DurationObject:GetElapsedDuration", duration_get),
        ("DurationObject:GetRemainingDuration", duration_get),
        ("DurationObject:GetElapsedPercent", duration_get),
        ("DurationObject:GetRemainingPercent", duration_get),
        ("DurationObject:GetModRate", duration_get),
        ("DurationObject:IsZero", duration_get),
        ("DurationObject:HasExpired", duration_get),
        ("DurationObject:HasStarted", duration_get),
        ("DurationObject:IsActive", duration_get),
        ("DurationObject:HasSecretValues", duration_get),
        ("DurationObject:GetClockTime", duration_get),
        ("DurationObject:EvaluateElapsedDuration", duration_evaluate),
        ("DurationObject:EvaluateElapsedPercent", duration_evaluate),
        (
            "DurationObject:EvaluateRemainingDuration",
            duration_evaluate,
        ),
        ("DurationObject:EvaluateRemainingPercent", duration_evaluate),
        ("DurationObject:EvaluateTotalDuration", duration_evaluate),
    ]
}
