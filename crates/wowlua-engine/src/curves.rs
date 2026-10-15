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
    ]
}
