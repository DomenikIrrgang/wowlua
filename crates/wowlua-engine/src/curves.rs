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
    // The chosen color as bytes, read back as floats (the full probe,
    // 1.60.1: 0.5 came back 0.50196081399918, 128/255; unverified: the
    // color curves' colors, kept as computed).
    let rgba = rgba.map(|v| f64::from(((v.clamp(0.0, 1.0) * 255.0).round() / 255.0) as f32));
    Ok(vec![make_color(vm, rgba, secret || s)])
}

/// `C_CurveUtil.EvaluateColorValueFromBoolean(boolean, ifTrue, ifFalse)`:
/// the chosen value clamped to a color's 0..1 (the full probe, 1.60.1: -1
/// gave 0, 1.5 and 2147483647 gave 1), secret when the boolean is.
fn color_value_from_boolean(vm: &mut Vm, c: NativeCall) -> Ret {
    let b = c.arg(1);
    let secret = matches!(b, Value::Secret(_));
    let pick = if vm.hidden(b).truthy() {
        c.arg(2)
    } else {
        c.arg(3)
    };
    let pick = match vm.hidden(pick) {
        Value::Num(n) if !matches!(pick, Value::Secret(_)) => Value::Num(n.clamp(0.0, 1.0)),
        _ => pick,
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

pub(crate) fn reset(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
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

pub(crate) fn assign(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some(other) = widgets::widget_of(vm, c.arg(2)) else {
        return Err(vm.error("a duration object expected"));
    };
    let s = span(vm, other);
    set_span(vm, id, s);
    Ok(vec![])
}

pub(crate) fn copy(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
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

/// A duration's length and how much of it has passed now, 0 to 1 (0 for
/// an empty one), and whether it is secret.
pub(crate) fn elapsed_fraction(vm: &mut Vm, id: WidgetId) -> (f64, f64, bool) {
    let s = span(vm, id);
    let (total, elapsed, _) = measures(vm, s, Value::Nil);
    let f = if total > 0.0 { elapsed / total } else { 0.0 };
    (total, f, s.secret)
}

pub(crate) fn duration_get(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
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
pub(crate) fn duration_evaluate(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
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
pub(crate) fn duration_format(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
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

// ------------------------------------------------- duration text bindings

/// `C_DurationUtil.CreateDurationTextBinding()`: a binding that writes a
/// duration's text (and color) into a font string, as Blizzard's
/// documentation describes the client's `DurationTextBindingObject`
/// (Blizzard's custom aura buttons give each one, SuuUI configures them).
/// Unverified: a new binding starts disabled (warcraft.wiki.gg's
/// `ScriptObject_DurationTextBinding` example enables its own), updating
/// every frame (an interval of 0), on real time (`Enum.
/// DurationTimeModifier.RealTime`, the duration objects' default).
fn create_text_binding(vm: &mut Vm, _c: NativeCall) -> Ret {
    let b = widgets::create(vm, "DurationTextBindingObject", None, None);
    if let Some(id) = widgets::widget_of(vm, b) {
        binding_defaults(vm, id);
        engine_mut(vm).world.text_bindings.push(id);
    }
    Ok(vec![b])
}

/// What a binding holds, by its stored name.
const BINDING_KEYS: [&str; 12] = [
    "fontString",
    "duration",
    "format",
    "components",
    "colorCurve",
    "colorProperty",
    "enabled",
    "interval",
    "modifier",
    "expiredText",
    "zeroText",
    "secret",
];

/// `SetToDefaults()`: no font string, duration, format, formatter or
/// fallback text (the documentation); the rest as a new binding's
/// (unverified).
fn binding_defaults(vm: &mut Vm, id: WidgetId) {
    for k in BINDING_KEYS {
        set_prop(vm, id, k, Value::Nil);
    }
    set_prop(vm, id, "enabled", Value::Bool(false));
    set_prop(vm, id, "interval", Value::Num(0.0));
    set_prop(vm, id, "modifier", Value::Num(0.0));
    set_prop(vm, id, "secret", Value::Bool(false));
}

/// Marks the binding as holding a secret when `v` is one.
fn note_secret(vm: &mut Vm, id: WidgetId, v: Value) {
    if matches!(v, Value::Secret(_)) {
        set_prop(vm, id, "secret", Value::Bool(true));
    }
}

fn binding_to_defaults(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    binding_defaults(vm, id);
    Ok(vec![])
}

/// `Assign(other)`: the other binding's configuration, all of it
/// (unverified: its font string too).
fn binding_assign(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some(other) = widgets::widget_of(vm, c.arg(2)) else {
        return Err(vm.error("a duration text binding expected"));
    };
    copy_binding(vm, other, id);
    Ok(vec![])
}

fn copy_binding(vm: &mut Vm, from: WidgetId, to: WidgetId) {
    for k in BINDING_KEYS {
        let v = prop(vm, from, k);
        set_prop(vm, to, k, v);
    }
}

fn binding_copy(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let b = widgets::create(vm, "DurationTextBindingObject", None, None);
    if let Some(new) = widgets::widget_of(vm, b) {
        copy_binding(vm, id, new);
        engine_mut(vm).world.text_bindings.push(new);
    }
    Ok(vec![b])
}

/// The plain setters and getters: the value kept as given.
fn binding_set(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let key = match method_name(vm, &c).as_str() {
        "SetDuration" => "duration",
        "SetExpiredText" => "expiredText",
        "SetZeroDurationText" => "zeroText",
        "SetFontString" => "fontString",
        "SetTimeModifier" => "modifier",
        "SetUpdateInterval" => "interval",
        _ => return Ok(vec![]),
    };
    let v = c.arg(2);
    note_secret(vm, id, v);
    let v = vm.hidden(v);
    set_prop(vm, id, key, v);
    Ok(vec![])
}

fn binding_get(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let key = match method_name(vm, &c).as_str() {
        "GetDuration" => "duration",
        "GetExpiredText" => "expiredText",
        "GetZeroDurationText" => "zeroText",
        "GetFontString" => "fontString",
        "GetTimeModifier" => "modifier",
        "GetUpdateInterval" => "interval",
        "IsEnabled" => "enabled",
        _ => return Ok(vec![]),
    };
    Ok(vec![prop(vm, id, key)])
}

fn binding_enable(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let on = match method_name(vm, &c).as_str() {
        "Enable" => true,
        "Disable" => false,
        _ => {
            note_secret(vm, id, c.arg(2));
            vm.hidden(c.arg(2)).truthy()
        }
    };
    set_prop(vm, id, "enabled", Value::Bool(on));
    Ok(vec![])
}

/// The binding's components: (property, formatter) in order.
fn components(vm: &mut Vm, id: WidgetId) -> Vec<(f64, Value)> {
    let Value::Table(t) = prop(vm, id, "components") else {
        return Vec::new();
    };
    let n = vm.heap.table(t).border();
    (0..n / 2)
        .map(|i| {
            let p = match vm.heap.table(t).get(Value::Num((2 * i + 1) as f64)) {
                Value::Num(p) => p,
                _ => 0.0,
            };
            (p, vm.heap.table(t).get(Value::Num((2 * i + 2) as f64)))
        })
        .collect()
}

fn set_format(vm: &mut Vm, id: WidgetId, format: Value, parts: &[(f64, Value)]) {
    let t = vm.new_table();
    for (i, (p, f)) in parts.iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num((2 * i + 1) as f64), Value::Num(*p));
        let _ = vm.heap.table_mut(t).set(Value::Num((2 * i + 2) as f64), *f);
    }
    set_prop(vm, id, "format", format);
    set_prop(vm, id, "components", Value::Table(t));
}

/// `SetFormatter(formatter)`: the remaining duration through it (the
/// documentation), a format of that one component.
fn binding_set_formatter(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let f = vm.hidden(c.arg(2));
    if widgets::widget_of(vm, f).is_none() {
        return Err(vm.error("a numeric formatter expected"));
    }
    note_secret(vm, id, c.arg(2));
    let format = vm.str("{}");
    set_format(vm, id, format, &[(0.0, f)]);
    Ok(vec![])
}

/// `SetTextFormat(formatString, components)`: each `{}` of the string the
/// next component's property through its formatter (the documentation).
fn binding_set_text_format(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let format = vm.hidden(c.arg(2));
    let Value::Table(list) = vm.hidden(c.arg(3)) else {
        return Err(vm.error("a list of components expected"));
    };
    note_secret(vm, id, c.arg(2));
    let mut parts = Vec::new();
    let n = vm.heap.table(list).border();
    for i in 1..=n {
        let Value::Table(t) = vm.hidden(vm.heap.table(list).get(Value::Num(i as f64))) else {
            return Err(vm.error("a format component expected"));
        };
        let (p, ps) = field(vm, t, "property");
        let (f, fs) = field(vm, t, "formatter");
        if fs || ps {
            set_prop(vm, id, "secret", Value::Bool(true));
        }
        if widgets::widget_of(vm, f).is_none() {
            return Err(vm.error("a numeric formatter expected"));
        }
        let p = match p {
            Value::Num(p) => p,
            _ => 0.0,
        };
        parts.push((p, f));
    }
    set_format(vm, id, format, &parts);
    Ok(vec![])
}

fn binding_set_color_curve(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let curve = vm.hidden(c.arg(2));
    if widgets::widget_of(vm, curve).is_none() {
        return Err(vm.error("a color curve expected"));
    }
    note_secret(vm, id, c.arg(2));
    note_secret(vm, id, c.arg(3));
    let property = vm.hidden(c.arg(3));
    set_prop(vm, id, "colorCurve", curve);
    set_prop(vm, id, "colorProperty", property);
    Ok(vec![])
}

fn binding_clear_color_curve(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    set_prop(vm, id, "colorCurve", Value::Nil);
    set_prop(vm, id, "colorProperty", Value::Nil);
    Ok(vec![])
}

fn binding_get_color_curve(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let curve = prop(vm, id, "colorCurve");
    if curve.is_nil() {
        return Ok(vec![]);
    }
    Ok(vec![curve, prop(vm, id, "colorProperty")])
}

/// The widget a binding's stored value `key` is.
fn widget_prop(vm: &mut Vm, id: WidgetId, key: &str) -> Option<WidgetId> {
    let v = prop(vm, id, key);
    widgets::widget_of(vm, v)
}

/// Whether it has a format to write (its formatter or text format).
fn can_format(vm: &mut Vm, id: WidgetId) -> bool {
    !prop(vm, id, "format").is_nil()
}

fn can_update(vm: &mut Vm, id: WidgetId) -> bool {
    can_format(vm, id) && widget_prop(vm, id, "fontString").is_some()
}

fn binding_can(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let yes = if method_name(vm, &c) == "CanUpdateFontString" {
        can_update(vm, id)
    } else {
        can_format(vm, id)
    };
    Ok(vec![Value::Bool(yes)])
}

/// The binding's duration's span, if it has one.
fn binding_span(vm: &mut Vm, id: WidgetId) -> Option<Span> {
    let d = widget_prop(vm, id, "duration")?;
    Some(span(vm, d))
}

/// `Enum.DurationTextBindingProperty` `p` of the span now.
fn property_value(vm: &mut Vm, s: Span, modifier: Value, p: f64) -> f64 {
    let (total, elapsed, remaining) = measures(vm, s, modifier);
    let percent = |x: f64| if total > 0.0 { x / total } else { 0.0 };
    match p as i64 {
        1 => percent(remaining),
        2 => elapsed,
        3 => percent(elapsed),
        4 => total,
        5 => s.start,
        6 => s.end,
        _ => remaining,
    }
}

/// The text it would write, and whether it is secret: the zero-duration
/// text without a duration or for a span of none, the expired text (when
/// set) once it ended, else its format with each `{}` a component's
/// property formatted (the documentation's wording). Unverified: a
/// fallback text not set writes an empty string; a `{}` without a
/// component stays empty.
fn formatted_text(vm: &mut Vm, id: WidgetId) -> Result<(Vec<u8>, bool), LuaError> {
    let held = prop(vm, id, "secret").truthy();
    let text_of = |vm: &mut Vm, v: Value| match vm.hidden(v) {
        Value::Str(s) => vm.str_bytes(s).to_vec(),
        _ => Vec::new(),
    };
    let s = match binding_span(vm, id) {
        Some(s) if s.end - s.start > 0.0 => s,
        _ => {
            let t = prop(vm, id, "zeroText");
            return Ok((text_of(vm, t), held));
        }
    };
    let secret = held || s.secret;
    let expired = prop(vm, id, "expiredText");
    if !expired.is_nil() && crate::clock::now(vm) >= s.end {
        return Ok((text_of(vm, expired), secret));
    }
    let format = prop(vm, id, "format");
    let format = text_of(vm, format);
    let modifier = prop(vm, id, "modifier");
    let mut parts = components(vm, id).into_iter();
    let mut out = Vec::new();
    let mut secret = secret;
    let mut i = 0;
    while i < format.len() {
        if format[i..].starts_with(b"{}") {
            if let Some((p, f)) = parts.next() {
                let x = property_value(vm, s, modifier, p);
                let v = crate::formatters::format_with(vm, f, x, false)?;
                secret |= matches!(v, Value::Secret(_));
                out.extend(text_of(vm, v));
            }
            i += 2;
        } else {
            out.push(format[i]);
            i += 1;
        }
    }
    Ok((out, secret))
}

/// The color its curve gives (`None`: no curve).
fn formatted_color(vm: &mut Vm, id: WidgetId) -> Option<Value> {
    let curve = widget_prop(vm, id, "colorCurve")?;
    let s = binding_span(vm, id).unwrap_or_default();
    let p = match prop(vm, id, "colorProperty") {
        Value::Num(p) => p,
        _ => 0.0,
    };
    let modifier = prop(vm, id, "modifier");
    let x = property_value(vm, s, modifier, p);
    let secret = s.secret || prop(vm, id, "secret").truthy();
    Some(evaluate_value(vm, curve, x, secret))
}

fn binding_text(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let (text, secret) = formatted_text(vm, id)?;
    let v = vm.bytes(&text);
    Ok(vec![hide(vm, secret, v)])
}

fn binding_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(formatted_color(vm, id).into_iter().collect())
}

fn binding_secret(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let s = binding_span(vm, id).is_some_and(|s| s.secret);
    Ok(vec![Value::Bool(s || prop(vm, id, "secret").truthy())])
}

/// Writes its text (and color) into its font string, as the client's C
/// code does: no font string method runs.
fn write_font_string(vm: &mut Vm, id: WidgetId) -> Result<(), LuaError> {
    if !can_update(vm, id) {
        return Ok(());
    }
    let Some(fs) = widget_prop(vm, id, "fontString") else {
        return Ok(());
    };
    let (text, secret) = formatted_text(vm, id)?;
    let v = vm.bytes(&text);
    let v = hide(vm, secret, v);
    widgets::store_text(vm, fs, Some(v));
    if let Some(Value::Table(color)) = formatted_color(vm, id).map(|c| vm.hidden(c)) {
        let mut rgba = Vec::new();
        for k in ["r", "g", "b", "a"] {
            let key = vm.str(k);
            rgba.push(vm.heap.table(color).get(key));
        }
        widgets::store(vm, fs, "TextColor", &rgba);
    }
    set_prop(vm, id, "lastUpdate", Value::Num(crate::clock::now(vm)));
    Ok(())
}

fn binding_update(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    write_font_string(vm, id)?;
    Ok(vec![])
}

/// Each frame: the enabled bindings whose interval passed write their
/// font strings (the documentation: "A value of zero updates every game
/// tick").
pub fn update_text_bindings(vm: &mut Vm) {
    let list = engine(vm).world.text_bindings.clone();
    let now = crate::clock::now(vm);
    for id in list {
        if !prop(vm, id, "enabled").truthy() {
            continue;
        }
        let interval = match prop(vm, id, "interval") {
            Value::Num(n) => n,
            _ => 0.0,
        };
        if let Value::Num(last) = prop(vm, id, "lastUpdate")
            && now - last < interval
        {
            continue;
        }
        let _ = write_font_string(vm, id);
    }
}

/// `UpdateAddOnMemoryUsage()`: measures the memory now, for
/// `GetAddOnMemoryUsage`.
fn update_memory(vm: &mut Vm, _c: NativeCall) -> Ret {
    let kib = vm.heap.estimate_bytes() as f64 / 1024.0;
    crate::globalsinfo::put(vm, "addOnMemory", Value::Num(kib));
    Ok(vec![])
}

/// The memory in KiB as `UpdateAddOnMemoryUsage` last measured it, 0
/// before (the full probe: 0 for any name, its own addon's too); the VM
/// does not tell addons apart (unverified: the whole VM's).
fn addon_memory(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![crate::globalsinfo::get_or(
        vm,
        "addOnMemory",
        Value::Num(0.0),
    )])
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
        (
            "C_DurationUtil.CreateDurationTextBinding",
            create_text_binding,
        ),
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
    let bindings: [(&str, widgets::MethodFn); 30] = [
        ("Assign", binding_assign),
        ("CanFormatText", binding_can),
        ("CanUpdateFontString", binding_can),
        ("ClearTextColorCurve", binding_clear_color_curve),
        ("Copy", binding_copy),
        ("Disable", binding_enable),
        ("Enable", binding_enable),
        ("SetEnabled", binding_enable),
        ("IsEnabled", binding_get),
        ("GetDuration", binding_get),
        ("GetExpiredText", binding_get),
        ("GetFontString", binding_get),
        ("GetTimeModifier", binding_get),
        ("GetUpdateInterval", binding_get),
        ("GetZeroDurationText", binding_get),
        ("SetDuration", binding_set),
        ("SetExpiredText", binding_set),
        ("SetFontString", binding_set),
        ("SetTimeModifier", binding_set),
        ("SetUpdateInterval", binding_set),
        ("SetZeroDurationText", binding_set),
        ("GetFormattedText", binding_text),
        ("GetFormattedTextColor", binding_color),
        ("GetTextColorCurve", binding_get_color_curve),
        ("HasSecretValues", binding_secret),
        ("SetFormatter", binding_set_formatter),
        ("SetTextColorCurve", binding_set_color_curve),
        ("SetTextFormat", binding_set_text_format),
        ("SetToDefaults", binding_to_defaults),
        ("UpdateFontString", binding_update),
    ];
    for (name, f) in bindings {
        let full: &'static str =
            Box::leak(format!("DurationTextBindingObject:{name}").into_boxed_str());
        out.push((full, f));
    }
    out
}
