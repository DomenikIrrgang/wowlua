//! Curve objects (`C_CurveUtil.CreateCurve`, `CreateColorCurve`): points a
//! value is mapped through, as Blizzard's documentation describes the
//! client's `LuaCurveObject` and `LuaColorCurveObject`; duration objects
//! (`C_DurationUtil.CreateDuration`, `UnitCastingDuration`). What the
//! documentation does not state is marked unverified where it is decided.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{LuaError, NativeCall, TableRef, Value, Vm};

use crate::widgets::{self, WidgetId};
use crate::{engine, engine_mut};

// ------------------------------------------------------------ curves

/// `Enum.LuaCurveType`.
const LINEAR: u8 = 0;
const STEP: u8 = 1;
const COSINE: u8 = 2;
const CUBIC: u8 = 3;

/// A curve's state: its type, its points (`y` a number in the first
/// channel, or a color's r, g, b, a), and whether any value it was given
/// was secret.
#[derive(Clone, Default)]
struct Curve {
    kind: u8,
    color: bool,
    secret: bool,
    points: Vec<(f64, [f64; 4])>,
}

fn create_curve(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![widgets::create(vm, "CurveObject", None, None)])
}

fn create_color_curve(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![widgets::create(vm, "ColorCurveObject", None, None)])
}

fn prop(vm: &mut Vm, id: WidgetId, key: &str) -> Value {
    let props = engine(vm).world.widgets[id.0].props;
    let k = vm.str(key);
    vm.heap.table(props).get(k)
}

fn set_prop(vm: &mut Vm, id: WidgetId, key: &str, v: Value) {
    let props = engine(vm).world.widgets[id.0].props;
    let k = vm.str(key);
    let _ = vm.heap.table_mut(props).set(k, v);
}

fn is_color_curve(vm: &Vm, id: WidgetId) -> bool {
    widgets::is_a(
        engine(vm),
        &engine(vm).world.widgets[id.0].class,
        "ColorCurveObject",
    )
}

/// The curve object `id` is (its points kept flat in its stored values:
/// x, then y or r, g, b, a).
fn curve(vm: &mut Vm, id: WidgetId) -> Curve {
    let color = is_color_curve(vm, id);
    let stride = if color { 5 } else { 2 };
    let mut out = Curve {
        kind: match prop(vm, id, "curveType") {
            Value::Num(n) => n as u8,
            _ => LINEAR,
        },
        color,
        secret: prop(vm, id, "secret").truthy(),
        points: Vec::new(),
    };
    if let Value::Table(t) = prop(vm, id, "points") {
        let n = vm.heap.table(t).border();
        let num = |vm: &Vm, i: usize| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Num(x) => x,
            _ => 0.0,
        };
        for p in 0..n / stride {
            let base = p * stride + 1;
            let mut y = [0.0, 0.0, 0.0, 1.0];
            for (k, slot) in y.iter_mut().enumerate().take(stride - 1) {
                *slot = num(vm, base + 1 + k);
            }
            out.points.push((num(vm, base), y));
        }
    }
    out
}

fn save(vm: &mut Vm, id: WidgetId, c: &Curve) {
    let t = vm.new_table();
    let stride = if c.color { 4 } else { 1 };
    let mut i = 1.0;
    for (x, y) in &c.points {
        let _ = vm.heap.table_mut(t).set(Value::Num(i), Value::Num(*x));
        i += 1.0;
        for v in &y[..stride] {
            let _ = vm.heap.table_mut(t).set(Value::Num(i), Value::Num(*v));
            i += 1.0;
        }
    }
    set_prop(vm, id, "points", Value::Table(t));
    set_prop(vm, id, "curveType", Value::Num(f64::from(c.kind)));
    set_prop(vm, id, "secret", Value::Bool(c.secret));
}

/// A number, secret or not: its value and whether it was secret (`None`:
/// not a number).
fn number(vm: &Vm, v: Value) -> Option<(f64, bool)> {
    match vm.hidden(v) {
        Value::Num(x) => Some((x, matches!(v, Value::Secret(_)))),
        _ => None,
    }
}

/// Field `key` of table `t`, its hidden value and whether it was secret.
fn field(vm: &mut Vm, t: TableRef, key: &str) -> (Value, bool) {
    let k = vm.str(key);
    let v = vm.heap.table(t).get(k);
    (vm.hidden(v), matches!(v, Value::Secret(_)))
}

/// A color's r, g, b, a (alpha 1 when it has none, as `ColorMixin`
/// reads it), and whether any was secret.
fn color_of(vm: &mut Vm, v: Value) -> Option<([f64; 4], bool)> {
    let secret = matches!(v, Value::Secret(_));
    let Value::Table(t) = vm.hidden(v) else {
        return None;
    };
    let mut out = [0.0, 0.0, 0.0, 1.0];
    let mut any = secret;
    for (i, k) in ["r", "g", "b", "a"].iter().enumerate() {
        let (v, s) = field(vm, t, k);
        any |= s;
        if let Value::Num(n) = v {
            out[i] = n;
        }
    }
    Some((out, any))
}

/// A table with mixin `name`'s methods (when Blizzard's code defined it)
/// and these fields.
pub(crate) fn mixin_table(vm: &mut Vm, name: &str, fields: &[(&str, Value)]) -> Value {
    let t = vm.new_table();
    if let Value::Table(m) = vm.get_global(name) {
        let methods: Vec<(Value, Value)> = vm.heap.table(m).entries().collect();
        for (k, f) in methods {
            let _ = vm.heap.table_mut(t).set(k, f);
        }
    }
    for (k, v) in fields {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, *v);
    }
    Value::Table(t)
}

/// A `ColorMixin` color of these channels, each secret if `secret`.
fn make_color(vm: &mut Vm, rgba: [f64; 4], secret: bool) -> Value {
    let mut fields = Vec::new();
    for (k, v) in ["r", "g", "b", "a"].iter().zip(rgba) {
        let v = if secret {
            vm.secret_wrap(Value::Num(v))
        } else {
            Value::Num(v)
        };
        fields.push((*k, v));
    }
    mixin_table(vm, "ColorMixin", &fields)
}

/// Adds a point after any of the same `x` (unverified: the client keeps
/// its points ordered by `x`; Blizzard's curves all add them in order).
fn insert(c: &mut Curve, x: f64, y: [f64; 4]) {
    let at = c.points.partition_point(|p| p.0 <= x);
    c.points.insert(at, (x, y));
}

/// A point's `y` argument: a number, or a color for a color curve.
fn y_arg(vm: &mut Vm, color: bool, v: Value) -> Result<([f64; 4], bool), LuaError> {
    if color {
        return color_of(vm, v).ok_or_else(|| vm.error("a color expected"));
    }
    let (y, s) = number(vm, v).ok_or_else(|| vm.error("a number expected"))?;
    Ok(([y, 0.0, 0.0, 1.0], s))
}

fn add_point(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let mut cur = curve(vm, id);
    let (x, sx) = number(vm, c.arg(2)).ok_or_else(|| vm.error("a number expected"))?;
    let (y, sy) = y_arg(vm, cur.color, c.arg(3))?;
    insert(&mut cur, x, y);
    cur.secret |= sx || sy;
    save(vm, id, &cur);
    Ok(vec![])
}

/// `SetPoints(points)`: vectors (`x`, `y`), or color points (`x`, `y` a
/// color); a point as a list `{x, y}` is taken too (unverified).
fn set_points(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let mut cur = curve(vm, id);
    let Value::Table(list) = vm.hidden(c.arg(2)) else {
        return Err(vm.error("a table of points expected"));
    };
    cur.points.clear();
    let n = vm.heap.table(list).border();
    for i in 1..=n {
        let Value::Table(p) = vm.hidden(vm.heap.table(list).get(Value::Num(i as f64))) else {
            return Err(vm.error("a point expected"));
        };
        let (kx, ky) = (vm.str("x"), vm.str("y"));
        let (mut x, mut y) = (vm.heap.table(p).get(kx), vm.heap.table(p).get(ky));
        if x.is_nil() {
            x = vm.heap.table(p).get(Value::Num(1.0));
            y = vm.heap.table(p).get(Value::Num(2.0));
        }
        let Some((x, sx)) = number(vm, x) else {
            return Err(vm.error("a point's x expected"));
        };
        let (y, sy) = y_arg(vm, cur.color, y)?;
        insert(&mut cur, x, y);
        cur.secret |= sx || sy;
    }
    save(vm, id, &cur);
    Ok(vec![])
}

/// `ClearPoints()`: no points; the secret flag stays (unverified: the
/// documentation names only `SetToDefaults` as clearing it).
fn clear_points(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let mut cur = curve(vm, id);
    cur.points.clear();
    save(vm, id, &cur);
    Ok(vec![])
}

/// `SetToDefaults()`: no points, linear, the secret flag cleared.
fn curve_defaults(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let color = is_color_curve(vm, id);
    save(
        vm,
        id,
        &Curve {
            color,
            ..Curve::default()
        },
    );
    Ok(vec![])
}

fn remove_point(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let mut cur = curve(vm, id);
    let i = number(vm, c.arg(2)).map_or(0.0, |n| n.0);
    if i < 1.0 || i as usize > cur.points.len() {
        return Err(vm.error("point index out of range"));
    }
    cur.points.remove(i as usize - 1);
    save(vm, id, &cur);
    Ok(vec![])
}

fn point_count(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(curve(vm, id).points.len() as f64)])
}

/// A value the curve gives: secret when it holds secrets or `secret`.
fn hide(vm: &mut Vm, secret: bool, v: Value) -> Value {
    if secret { vm.secret_wrap(v) } else { v }
}

/// A point as the client returns it: a `Vector2DMixin` vector, or for a
/// color curve `{x = x, y = color}`.
fn point_value(vm: &mut Vm, cur: &Curve, i: usize) -> Value {
    let (x, y) = cur.points[i];
    let xv = hide(vm, cur.secret, Value::Num(x));
    if cur.color {
        let color = make_color(vm, y, cur.secret);
        let t = vm.new_table();
        for (k, v) in [("x", xv), ("y", color)] {
            let key = vm.str(k);
            let _ = vm.heap.table_mut(t).set(key, v);
        }
        return Value::Table(t);
    }
    let yv = hide(vm, cur.secret, Value::Num(y[0]));
    mixin_table(vm, "Vector2DMixin", &[("x", xv), ("y", yv)])
}

fn get_point(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let cur = curve(vm, id);
    let i = number(vm, c.arg(2)).map_or(0.0, |n| n.0);
    if i < 1.0 || i as usize > cur.points.len() {
        return Ok(vec![Value::Nil]);
    }
    Ok(vec![point_value(vm, &cur, i as usize - 1)])
}

fn get_points(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let cur = curve(vm, id);
    let t = vm.new_table();
    for i in 0..cur.points.len() {
        let p = point_value(vm, &cur, i);
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), p);
    }
    Ok(vec![Value::Table(t)])
}

fn get_type(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(f64::from(curve(vm, id).kind))])
}

/// `SetType(type)`: a secret type counts as a secret value (unverified).
fn set_type(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let mut cur = curve(vm, id);
    let (k, s) = number(vm, c.arg(2)).ok_or_else(|| vm.error("a curve type expected"))?;
    if !(0.0..=3.0).contains(&k) {
        return Err(vm.error("an invalid curve type"));
    }
    cur.kind = k as u8;
    cur.secret |= s;
    save(vm, id, &cur);
    Ok(vec![])
}

fn curve_secret(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(curve(vm, id).secret)])
}

fn curve_copy(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let cur = curve(vm, id);
    let class = engine(vm).world.widgets[id.0].class.clone();
    let new = widgets::create(vm, &class, None, None);
    if let Some(n) = widgets::widget_of(vm, new) {
        save(vm, n, &cur);
    }
    Ok(vec![new])
}

/// The curve's value at `x` in each channel: an empty curve's is zero (the
/// documentation); outside its points, its end points' values; between
/// them, by its type. A step takes the last point at or before `x` (the
/// comment of Blizzard's aura duration curve: "Curve points promote on
/// exact matches"). Unverified: the cosine's and the cubic's exact forms
/// (a half cosine; Catmull-Rom through four points).
fn evaluate_at(c: &Curve, x: f64) -> [f64; 4] {
    let p = &c.points;
    let (Some(first), Some(last)) = (p.first(), p.last()) else {
        return [0.0; 4];
    };
    if x <= first.0 {
        // A step at its first point's `x` is that point's (the last of
        // that `x`).
        if c.kind == STEP && x == first.0 {
            let at = p.partition_point(|q| q.0 <= x);
            return p[at - 1].1;
        }
        return first.1;
    }
    if x >= last.0 {
        return last.1;
    }
    // The segment: p[i] <= x < p[i + 1].
    let i = p.partition_point(|q| q.0 <= x) - 1;
    let ((x0, y0), (x1, y1)) = (p[i], p[i + 1]);
    if c.kind == STEP {
        return y0;
    }
    let t = if x1 == x0 { 1.0 } else { (x - x0) / (x1 - x0) };
    let lerp = |t: f64| std::array::from_fn(|k| y0[k] + (y1[k] - y0[k]) * t);
    match c.kind {
        COSINE => lerp((1.0 - (std::f64::consts::PI * t).cos()) / 2.0),
        CUBIC if p.len() >= 4 => {
            let before = if i == 0 { y0 } else { p[i - 1].1 };
            let after = p.get(i + 2).map_or(y1, |q| q.1);
            std::array::from_fn(|k| {
                let (a, b, c, d) = (before[k], y0[k], y1[k], after[k]);
                let t2 = t * t;
                let t3 = t2 * t;
                0.5 * (2.0 * b
                    + (-a + c) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (-a + 3.0 * b - 3.0 * c + d) * t3)
            })
        }
        // Fewer than four points: cosine (the documentation).
        CUBIC => lerp((1.0 - (std::f64::consts::PI * t).cos()) / 2.0),
        _ => lerp(t),
    }
}

/// The curve `id`'s value at `x` as Lua sees it: a number, or a color for
/// a color curve; secret when the curve holds secrets or `x` is secret
/// (the documentation: "Curves with secret values always produce secret
/// results").
pub fn evaluate_value(vm: &mut Vm, id: WidgetId, x: f64, secret: bool) -> Value {
    let cur = curve(vm, id);
    let y = evaluate_at(&cur, x);
    let secret = secret || cur.secret;
    if cur.color {
        return make_color(vm, y, secret);
    }
    hide(vm, secret, Value::Num(y[0]))
}

/// The curve's numeric value at `x` (a color curve's red).
pub fn evaluate(vm: &mut Vm, id: WidgetId, x: f64) -> f64 {
    let cur = curve(vm, id);
    evaluate_at(&cur, x)[0]
}

fn evaluate_method(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (x, s) = number(vm, c.arg(2)).ok_or_else(|| vm.error("a number expected"))?;
    Ok(vec![evaluate_value(vm, id, x, s)])
}

/// `EvaluateUnpacked(x)`: a color curve's r, g, b, a.
fn evaluate_unpacked(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (x, s) = number(vm, c.arg(2)).ok_or_else(|| vm.error("a number expected"))?;
    let cur = curve(vm, id);
    let y = evaluate_at(&cur, x);
    let secret = s || cur.secret;
    Ok(y.iter().map(|v| hide(vm, secret, Value::Num(*v))).collect())
}

/// `value` through `curve` when one is given (a color curve gives a
/// color).
pub fn apply(vm: &mut Vm, curve: Value, value: f64) -> Result<Value, LuaError> {
    match curve {
        Value::Nil => Ok(Value::Num(value)),
        v => match widgets::widget_of(vm, v) {
            Some(id) => Ok(evaluate_value(vm, id, value, false)),
            None => Err(vm.error("a curve object expected")),
        },
    }
}

/// `C_CurveUtil.EvaluateColorFromBoolean(boolean, ifTrue, ifFalse)`: a new
/// color of the chosen one's channels, secret when the boolean is
/// (unverified: a new color rather than the one passed).
fn color_from_boolean(vm: &mut Vm, c: NativeCall) -> Ret {
    let b = c.arg(1);
    let secret = matches!(b, Value::Secret(_));
    let pick = if vm.hidden(b).truthy() {
        c.arg(2)
    } else {
        c.arg(3)
    };
    let (rgba, s) = color_of(vm, pick).ok_or_else(|| vm.error("a color expected"))?;
    Ok(vec![make_color(vm, rgba, secret || s)])
}

/// `C_CurveUtil.EvaluateColorValueFromBoolean(boolean, ifTrue, ifFalse)`:
/// the chosen value, secret when the boolean is.
fn color_value_from_boolean(vm: &mut Vm, c: NativeCall) -> Ret {
    let b = c.arg(1);
    let secret = matches!(b, Value::Secret(_));
    let pick = if vm.hidden(b).truthy() {
        c.arg(2)
    } else {
        c.arg(3)
    };
    Ok(vec![hide(vm, secret, pick)])
}

// -------------------------------------------------------------- durations

/// `C_DurationUtil.CreateDuration()`: a zero `DurationObject`, a time span
/// on the clock that keeps secret values secret (Blizzard's aura buttons
/// set theirs from `secretwrap`ped aura times). Unverified: the evaluated
/// results' shape (the curve's value).
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
    let num = |v: Value| match v {
        Value::Num(n) => n,
        _ => 0.0,
    };
    Span {
        start: num(prop(vm, id, "start")),
        end: num(prop(vm, id, "end")),
        rate: match prop(vm, id, "modRate") {
            Value::Num(n) => n,
            _ => 1.0,
        },
        secret: prop(vm, id, "secret").truthy(),
    }
}

fn set_span(vm: &mut Vm, id: WidgetId, s: Span) {
    for (k, v) in [
        ("start", Value::Num(s.start)),
        ("end", Value::Num(s.end)),
        ("modRate", Value::Num(s.rate)),
        ("secret", Value::Bool(s.secret)),
    ] {
        set_prop(vm, id, k, v);
    }
}

/// A new duration object of this span.
fn new_duration(vm: &mut Vm, s: Span) -> Value {
    let new = widgets::create(vm, "DurationObject", None, None);
    if let Some(n) = widgets::widget_of(vm, new) {
        set_span(vm, n, s);
    }
    new
}

/// Number argument `n`, secret or not (`None`: absent): its value and
/// whether it was secret.
fn time_arg(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<Option<(f64, bool)>, LuaError> {
    let v = c.arg(n);
    match vm.hidden(v) {
        Value::Nil => Ok(None),
        _ => number(vm, v)
            .map(Some)
            .ok_or_else(|| vm.error("a number expected")),
    }
}

/// `SetTimeFromEnd(end, duration, rate?)`, `SetTimeFromStart(start,
/// duration, rate?)`, `SetTimeSpan(start, end)` (an end before the start
/// clamps to zero: the documentation).
fn set_time(vm: &mut Vm, id: WidgetId, c: NativeCall, kind: u8) -> Ret {
    let (Some((a, sa)), Some((b, sb))) = (time_arg(vm, &c, 2)?, time_arg(vm, &c, 3)?) else {
        return Err(vm.error("two times expected"));
    };
    let rate = if kind == 2 {
        None
    } else {
        time_arg(vm, &c, 4)?
    };
    let secret = sa || sb || rate.is_some_and(|r| r.1);
    let (start, end) = match kind {
        0 => (a - b, a),
        1 => (a, a + b),
        _ => (a, b.max(a)),
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
    Ok(vec![new_duration(vm, s)])
}

/// The method called: a documented one's index, else its full name, is
/// the native's upvalue.
pub(crate) fn method_name(vm: &Vm, c: &NativeCall) -> String {
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

/// The span's measures now: (total, elapsed, remaining), in base time
/// with `Enum.DurationTimeModifier.BaseTime` (1): real time divided by
/// the mod rate, as the documentation calls it "the divisor used to
/// convert a duration from real time to base time" (unverified).
fn measures(vm: &mut Vm, s: Span, modifier: Value) -> (f64, f64, f64) {
    let now = crate::clock::now(vm);
    let total = (s.end - s.start).max(0.0);
    let elapsed = (now - s.start).clamp(0.0, total);
    let base = matches!(vm.hidden(modifier), Value::Num(m) if m == 1.0);
    let k = if base && s.rate > 0.0 {
        1.0 / s.rate
    } else {
        1.0
    };
    (total * k, elapsed * k, (total - elapsed) * k)
}

fn duration_get(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let which = method_name(vm, &c);
    let s = span(vm, id);
    let (total, elapsed, remaining) = measures(vm, s, c.arg(2));
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
    Ok(vec![hide(vm, s.secret, v)])
}

/// The measure a `Evaluate…` or `Format…` method names.
fn measure(vm: &mut Vm, which: &str, s: Span, modifier: Value) -> f64 {
    let (total, elapsed, remaining) = measures(vm, s, modifier);
    let percent = |x: f64| if total > 0.0 { x / total } else { 0.0 };
    match which {
        "EvaluateElapsedDuration" | "FormatElapsedDuration" => elapsed,
        "EvaluateElapsedPercent" => percent(elapsed),
        "EvaluateRemainingDuration" | "FormatRemainingDuration" => remaining,
        "EvaluateRemainingPercent" => percent(remaining),
        _ => total,
    }
}

/// `Evaluate…(curve, modifier)`: the measure through the curve, secret when
/// the curve or the span is.
fn duration_evaluate(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let which = method_name(vm, &c);
    let s = span(vm, id);
    let x = measure(vm, &which, s, c.arg(3));
    let Some(cid) = widgets::widget_of(vm, c.arg(2)) else {
        return Err(vm.error("a curve object expected"));
    };
    Ok(vec![evaluate_value(vm, cid, x, s.secret)])
}

/// `Format…Duration(formatter, modifier)`: the measure through a numeric
/// formatter, secret when the formatter or the span is.
fn duration_format(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let which = method_name(vm, &c);
    let s = span(vm, id);
    let x = measure(vm, &which, s, c.arg(3));
    let v = crate::formatters::format_with(vm, c.arg(2), x, s.secret)?;
    Ok(vec![v])
}

/// `UnitCastingDuration(unit)`, `UnitChannelDuration(unit)`: a new duration
/// object of the unit's cast (or channel) from its start to its end, as
/// `UnitCastingInfo` gives them; nothing without one. Its values are
/// secret, the object not (the documentation's `SecretReturns`; an
/// addon calls its methods on any unit's, SuuUI's cast bars in the
/// client). Unverified: the player's own cast's values secret too.
fn cast_duration(vm: &mut Vm, c: &NativeCall, channel: bool) -> Ret {
    let Some(unit) = crate::units::unit_arg(vm, c, 1) else {
        return Ok(vec![]);
    };
    let Some(cast) = engine(vm).units.casting.casts.get(&unit).cloned() else {
        return Ok(vec![]);
    };
    if cast.channel != channel {
        return Ok(vec![]);
    }
    let secret = !channel;
    let d = new_duration(
        vm,
        Span {
            start: cast.start,
            end: cast.end,
            rate: 1.0,
            secret,
        },
    );
    engine_mut(vm).plain_results = true;
    Ok(vec![d])
}

fn unit_casting_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    cast_duration(vm, &c, false)
}

/// `UnitChannelDuration(unit)`: the documentation marks no secrecy.
fn unit_channel_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    cast_duration(vm, &c, true)
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
        ("C_CurveUtil.CreateColorCurve", create_color_curve),
        ("C_CurveUtil.EvaluateColorFromBoolean", color_from_boolean),
        (
            "C_CurveUtil.EvaluateColorValueFromBoolean",
            color_value_from_boolean,
        ),
        ("C_DurationUtil.CreateDuration", create_duration),
        ("UnitCastingDuration", unit_casting_duration),
        ("UnitChannelDuration", unit_channel_duration),
        ("UpdateAddOnMemoryUsage", update_memory),
        ("GetAddOnMemoryUsage", addon_memory),
    ]
}

pub fn methods() -> Vec<(&'static str, widgets::MethodFn)> {
    let mut out: Vec<(&'static str, widgets::MethodFn)> = Vec::new();
    for class in ["CurveObject", "ColorCurveObject"] {
        let methods: [(&str, widgets::MethodFn); 10] = [
            ("AddPoint", add_point),
            ("ClearPoints", clear_points),
            ("Copy", curve_copy),
            ("Evaluate", evaluate_method),
            ("GetPoint", get_point),
            ("GetPointCount", point_count),
            ("GetPoints", get_points),
            ("RemovePoint", remove_point),
            ("SetPoints", set_points),
            ("SetToDefaults", curve_defaults),
        ];
        for (name, f) in methods {
            let full: &'static str = Box::leak(format!("{class}:{name}").into_boxed_str());
            out.push((full, f));
        }
    }
    out.extend([
        ("CurveObjectBase:GetType", get_type as widgets::MethodFn),
        ("CurveObjectBase:SetType", set_type),
        ("CurveObjectBase:HasSecretValues", curve_secret),
        ("ColorCurveObject:EvaluateUnpacked", evaluate_unpacked),
    ]);
    out.extend([
        (
            "DurationObject:SetTimeFromEnd",
            set_time_from_end as widgets::MethodFn,
        ),
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
        ("DurationObject:FormatElapsedDuration", duration_format),
        ("DurationObject:FormatRemainingDuration", duration_format),
        ("DurationObject:FormatTotalDuration", duration_format),
    ]);
    out
}
