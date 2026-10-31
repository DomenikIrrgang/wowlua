//! Sliders and status bars: a value kept between a minimum and a maximum.
//!
//! Verified against the client (`/stdtest engine answers` CHECKS, Forever
//! 70425): values are kept as single floats; a slider's value snaps to its
//! step from the minimum (`min + round((v - min) / step) * step`, the step
//! 0.001 until set, so 7.25 reads back 7.2500004768372), a bar's does not;
//! both clamp to the range (0 to 0 until set). `OnValueChanged` runs at
//! once, hidden or disabled, only when the value changes: a slider's with
//! `(value, userInput)` (`SetValue`'s second argument), a bar's with the
//! value alone (`userInput` nil). `SetMinMaxValues` runs
//! `OnMinMaxChanged(min, max)` when the range changes, then clamps the
//! value; a range whose maximum is below its minimum is an error;
//! `SetValueStep` snaps the value again. Until a
//! slider's value is set, neither a range nor a step moves it (it stays 0);
//! a bar's first range clamps it, silently. A user's drag (`drag`) runs
//! `OnValueChanged(value, true)` at each move, unsnapped. A bar keeps
//! its color in eight bits; a slider's orientation is `VERTICAL` until set,
//! a bar's `HORIZONTAL`.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::widgets::{self, MethodFn, WidgetId};
use crate::{engine, engine_mut};

/// A slider's or a status bar's range and value.
#[derive(Clone, Copy)]
pub struct Bar {
    pub min: f64,
    pub max: f64,
    pub value: f64,
    /// A slider's step (none for a bar).
    pub step: Option<f64>,
    /// Whether a range was set: a bar's first clamps its value without
    /// `OnValueChanged` (a fresh bar given 3 to 10 reads 3).
    pub ranged: bool,
    /// Whether a slider's value was set: until then a range leaves it (a
    /// fresh slider given 1 to 10, then 2 to 10, reads 0, no
    /// `OnValueChanged`).
    pub valued: bool,
    /// Whether the range or the value was set from a secret: read back
    /// secret (unverified: the documented secret aspects; a player
    /// target's health bar).
    pub secret_range: bool,
    pub secret_value: bool,
}

/// A float as the client keeps it (a single float).
fn single(v: f64) -> f64 {
    f64::from(v as f32)
}

fn is_slider(vm: &Vm, id: WidgetId) -> bool {
    let class = engine(vm).world.widgets[id.0].class.clone();
    widgets::is_a(engine(vm), &class, "Slider")
}

fn bar(vm: &Vm, id: WidgetId) -> Bar {
    engine(vm).world.bars.get(&id.0).copied().unwrap_or(Bar {
        min: 0.0,
        max: 0.0,
        value: 0.0,
        step: is_slider(vm, id).then_some(single(0.001)),
        ranged: false,
        valued: false,
        secret_range: false,
        secret_value: false,
    })
}

/// `v` clamped to the range and snapped to the step.
fn kept_value(b: &Bar, v: f64) -> f64 {
    let mut x = v.clamp(b.min, b.max);
    if let Some(step) = b.step
        && step > 0.0
    {
        x = (b.min + ((x - b.min) / step).round() * step).clamp(b.min, b.max);
    }
    single(x)
}

/// Gives the bar value `v` as kept; `OnValueChanged` runs if it changed,
/// with the value as `GetValue` reads it (and `OnMinMaxChanged` with the
/// range as `GetMinMaxValues` does): secret after a secret was set, the
/// bar's value aspect (unverified for the scripts: not recorded).
fn change(vm: &mut Vm, id: WidgetId, v: f64, user: bool) {
    let mut b = bar(vm, id);
    let x = kept_value(&b, v);
    let changed = x != b.value;
    b.value = x;
    engine_mut(vm).world.bars.insert(id.0, b);
    if changed {
        let value = read(vm, x, b.secret_value);
        let args = if b.step.is_some() {
            vec![value, Value::Bool(user)]
        } else {
            vec![value]
        };
        widgets::run_script(vm, id, "OnValueChanged", &args);
    }
}

/// Whether a bar's value or range was set from a secret (its bar value
/// aspect).
pub(crate) fn secret_value(vm: &Vm, id: WidgetId) -> bool {
    engine(vm)
        .world
        .bars
        .get(&id.0)
        .is_some_and(|b| b.secret_value || b.secret_range)
}

/// A number given, a secret's hidden one too.
fn number(vm: &Vm, v: Value) -> f64 {
    vm.tonumber(vm.hidden(v)).unwrap_or(0.0)
}

fn is_secret(v: Value) -> bool {
    matches!(v, Value::Secret(_))
}

/// A number read back, secret when it was set so.
fn read(vm: &mut Vm, n: f64, secret: bool) -> Value {
    if secret {
        vm.secret_wrap(Value::Num(n))
    } else {
        Value::Num(n)
    }
}

/// The user dragging a slider's thumb: to `to` (clamped, snapped only when
/// it obeys its step on drag), or where it is (`None`, the mouse down on
/// it). `OnValueChanged(value, true)` runs at each move, the value changed
/// or not (`/stdtest engine input`).
pub fn drag(vm: &mut Vm, id: WidgetId, to: Option<f64>) {
    engine_mut(vm).world.dragging_thumb = Some(id);
    let mut b = bar(vm, id);
    if let Some(v) = to {
        let obeys = stored_or(vm, id, "ObeyStepOnDrag", vec![])
            .first()
            .is_some_and(|v| v.truthy());
        let free = Bar {
            step: if obeys { b.step } else { None },
            ..b
        };
        b.value = kept_value(&free, v);
        engine_mut(vm).world.bars.insert(id.0, b);
    }
    widgets::run_script(
        vm,
        id,
        "OnValueChanged",
        &[Value::Num(b.value), Value::Bool(true)],
    );
}

/// The thumb let go.
pub fn end_drag(vm: &mut Vm, id: WidgetId) {
    if engine(vm).world.dragging_thumb == Some(id) {
        engine_mut(vm).world.dragging_thumb = None;
    }
}

fn is_dragging_thumb(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        engine(vm).world.dragging_thumb == Some(id),
    )])
}

/// `SetValue(value [, treatAsMouseEvent])` (a bar's second argument is
/// its interpolation).
fn set_value(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    stop_timer(vm, id);
    let v = number(vm, c.arg(2));
    let mut b = bar(vm, id);
    b.valued = true;
    b.secret_value = is_secret(c.arg(2));
    engine_mut(vm).world.bars.insert(id.0, b);
    let user = is_slider(vm, id) && c.arg(3).truthy();
    change(vm, id, v, user);
    Ok(vec![])
}

fn get_value(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if engine(vm).world.timer_bars.contains(&id) {
        follow_timer(vm, id);
    }
    let b = bar(vm, id);
    Ok(vec![read(vm, b.value, b.secret_value)])
}

/// `SetMinMaxValues(min, max)`.
fn set_min_max_values(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (min, max) = (single(number(vm, c.arg(2))), single(number(vm, c.arg(3))));
    if max < min {
        let class = engine(vm).world.widgets[id.0].class.clone();
        let mut msg = format!("{class}:SetMinMaxValues(): max must be >= min");
        // The client adds the taint of the addon code that called it.
        if !crate::protection::blizzard_caller(vm) {
            msg.push_str("\nLua Taint: *** ForceTaint_Strong ***");
        }
        let v = vm.str(&msg);
        return Err(vm.raise(v));
    }
    stop_timer(vm, id);
    let mut b = bar(vm, id);
    let secret = is_secret(c.arg(2)) || is_secret(c.arg(3));
    if b.secret_range != secret {
        b.secret_range = secret;
        engine_mut(vm).world.bars.insert(id.0, b);
    }
    if (b.min, b.max) == (min, max) {
        return Ok(vec![]);
    }
    (b.min, b.max) = (min, max);
    let first = !b.ranged;
    b.ranged = true;
    let slider = b.step.is_some();
    if first && !slider {
        b.value = kept_value(&b, b.value);
    }
    engine_mut(vm).world.bars.insert(id.0, b);
    let range = [read(vm, min, secret), read(vm, max, secret)];
    widgets::run_script(vm, id, "OnMinMaxChanged", &range);
    if !slider || b.valued {
        change(vm, id, b.value, false);
    }
    Ok(vec![])
}

fn get_min_max_values(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if engine(vm).world.timer_bars.contains(&id) {
        follow_timer(vm, id);
    }
    let b = bar(vm, id);
    let (min, max) = (b.min, b.max);
    Ok(vec![
        read(vm, min, b.secret_range),
        read(vm, max, b.secret_range),
    ])
}

/// `SetValueStep(step)`: a value set snaps to it.
fn set_value_step(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let mut b = bar(vm, id);
    b.step = Some(single(number(vm, c.arg(2))));
    engine_mut(vm).world.bars.insert(id.0, b);
    if b.valued {
        change(vm, id, b.value, false);
    }
    Ok(vec![])
}

fn get_value_step(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(bar(vm, id).step.unwrap_or(0.0))])
}

/// `StatusBar:SetTimerDuration(duration [, interpolation [, direction]])`:
/// the bar's value follows the duration from now on, each frame, across
/// its range: `Enum.StatusBarTimerDirection.ElapsedTime` (0, the default)
/// fills it, `RemainingTime` (1) drains it, secret while the duration is
/// (unverified: what the client's `GetValue` reads, whether
/// `OnValueChanged` runs; it doesn't here). A value set stops it
/// (unverified). The interpolation is how the client eases a change, which
/// the engine doesn't draw.
fn set_timer_duration(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    widgets::store(vm, id, "TimerDuration", &[c.arg(2), c.arg(3), c.arg(4)]);
    let timers = &mut engine_mut(vm).world.timer_bars;
    if !timers.contains(&id) {
        timers.push(id);
    }
    follow_timer(vm, id);
    Ok(vec![])
}

fn stop_timer(vm: &mut Vm, id: WidgetId) {
    engine_mut(vm).world.timer_bars.retain(|&t| t != id);
}

/// A timer bar's value now, from its duration and direction.
fn follow_timer(vm: &mut Vm, id: WidgetId) {
    let props = engine(vm).world.widgets[id.0].props;
    let args = widgets::stored_args(vm, props, "TimerDuration").unwrap_or_default();
    let Some(duration) = args.first().and_then(|&d| widgets::widget_of(vm, d)) else {
        return;
    };
    let draining = args
        .get(2)
        .is_some_and(|&d| vm.tonumber(vm.hidden(d)) == Some(1.0));
    // The engine's clock: a duration on a manual clock (`SetClock`) reads
    // it at the engine's time here.
    let (total, elapsed, secret) = crate::curves::elapsed_fraction(vm, duration);
    let f = if draining { 1.0 - elapsed } else { elapsed };
    // The duration is the bar's range (Blizzard's code sets none for a
    // timer, and clears one with `SetMinMaxValues(0, 0)`).
    let mut b = bar(vm, id);
    (b.min, b.max, b.ranged, b.secret_range) = (0.0, total, true, secret);
    b.value = total * f;
    b.secret_value = secret;
    engine_mut(vm).world.bars.insert(id.0, b);
}

/// Each frame: the shown timer bars' values follow their durations (a
/// hidden one's when it is read: `GetValue`, `GetMinMaxValues`; T541).
pub fn update_timers(vm: &mut Vm) {
    for id in engine(vm).world.timer_bars.clone() {
        if widgets::visible(engine(vm), id) {
            follow_timer(vm, id);
        }
    }
}

/// What setter `stem` stored, else `default`.
fn stored_or(vm: &mut Vm, id: WidgetId, stem: &str, default: Vec<Value>) -> Vec<Value> {
    let props = engine(vm).world.widgets[id.0].props;
    widgets::stored_args(vm, props, stem).unwrap_or(default)
}

fn get_steps_per_page(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(stored_or(vm, id, "StepsPerPage", vec![Value::Num(0.0)]))
}

fn get_obey_step_on_drag(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let on = stored_or(vm, id, "ObeyStepOnDrag", vec![])
        .first()
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(on)])
}

fn get_orientation(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let default = if is_slider(vm, id) {
        "VERTICAL"
    } else {
        "HORIZONTAL"
    };
    let default = vec![vm.str(default)];
    Ok(stored_or(vm, id, "Orientation", default))
}

fn get_fill_style(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(stored_or(vm, id, "FillStyle", vec![Value::Num(0.0)]))
}

fn flag(vm: &mut Vm, id: WidgetId, stem: &str) -> Ret {
    let on = stored_or(vm, id, stem, vec![])
        .first()
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(on)])
}

fn get_reverse_fill(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    flag(vm, id, "ReverseFill")
}

fn get_rotates_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    flag(vm, id, "RotatesTexture")
}

/// A texture given by a file or atlas becomes a texture of the frame
/// (`SetStatusBarTexture("Interface\\...")`, `SetThumbTexture`); a
/// texture object is used as it is.
fn texture_slot(vm: &mut Vm, id: WidgetId, stem: &str, asset: Value) -> Value {
    if widgets::widget_of(vm, asset).is_some() {
        widgets::store(vm, id, stem, &[asset]);
        return asset;
    }
    let tex = match stored_or(vm, id, stem, vec![]).first().copied() {
        Some(t) if widgets::widget_of(vm, t).is_some() => t,
        _ => {
            let t = widgets::create(vm, "Texture", None, Some(id));
            widgets::store(vm, id, stem, &[t]);
            t
        }
    };
    if let Some(t) = widgets::widget_of(vm, tex) {
        widgets::store(vm, t, "Texture", &[asset]);
    }
    tex
}

/// `SetStatusBarTexture(asset)`: success.
fn set_status_bar_texture(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let tex = texture_slot(vm, id, "StatusBarTexture", c.arg(2));
    // The bar's color is the texture's.
    let [r, g, b, a] = status_bar_color(vm, id);
    if let Some(t) = widgets::widget_of(vm, tex) {
        widgets::store(vm, t, "VertexColor", &[r, g, b, a].map(Value::Num));
    }
    Ok(vec![Value::Bool(true)])
}

fn get_status_bar_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![
        stored_or(vm, id, "StatusBarTexture", vec![Value::Nil])[0],
    ])
}

/// A slider's thumb, placed by the slider as the client does when it has no
/// points of its own: (the slider, how far along its value is from the
/// minimum, 0 to 1, whether the slider is vertical). The thumb travels the
/// slider's length less its own, centered across it (unverified: the
/// client's insets and the thumb's exact span).
pub(crate) fn thumb_place(vm: &Vm, thumb: WidgetId) -> Option<(WidgetId, f64, bool)> {
    let slider = engine(vm).world.widgets[thumb.0].parent?;
    if !is_slider(vm, slider) {
        return None;
    }
    let props = engine(vm).world.widgets[slider.0].props;
    let set = widgets::stored_ref(vm, props, "ThumbTexture")?;
    if widgets::widget_of(vm, *set.first()?) != Some(thumb) {
        return None;
    }
    let b = bar(vm, slider);
    let at = if b.max > b.min {
        ((b.value - b.min) / (b.max - b.min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let vertical = widgets::stored_ref(vm, props, "Orientation")
        .and_then(|v| v.first().copied())
        .and_then(|v| match v {
            Value::Str(s) => Some(vm.str_bytes(s).eq_ignore_ascii_case(b"VERTICAL")),
            _ => None,
        })
        // A slider is vertical until told otherwise (`GetOrientation`).
        .unwrap_or(true);
    Some((slider, at, vertical))
}

/// A status bar's texture, the part its value fills: (the bar, how much of
/// it, 0 to 1, whether it fills upwards, whether from the right or top).
/// A bar without a value yet shows it whole.
pub(crate) fn fill_place(vm: &Vm, tex: WidgetId) -> Option<(WidgetId, f64, bool, bool)> {
    let bar_id = engine(vm).world.widgets[tex.0].parent?;
    let props = engine(vm).world.widgets[bar_id.0].props;
    let first = |stem| widgets::stored_ref(vm, props, stem).and_then(|v| v.first().copied());
    let is_fill = ["StatusBarTexture", "StatusBarTextureObject"]
        .into_iter()
        .any(|k| first(k).and_then(|v| widgets::widget_of(vm, v)) == Some(tex));
    if !is_fill {
        return None;
    }
    let f = engine(vm).world.bars.get(&bar_id.0).map_or(1.0, |b| {
        let range = b.max - b.min;
        if range > 0.0 {
            ((b.value - b.min) / range).clamp(0.0, 1.0)
        } else {
            0.0
        }
    });
    let vertical = matches!(first("Orientation").map(|v| vm.hidden(v)), Some(Value::Str(s)) if vm.str_bytes(s).eq_ignore_ascii_case(b"VERTICAL"));
    let reverse = first("ReverseFill").is_some_and(|v| v.truthy());
    Some((bar_id, f, vertical, reverse))
}

fn set_thumb_texture(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    texture_slot(vm, id, "ThumbTexture", c.arg(2));
    Ok(vec![])
}

fn get_thumb_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![stored_or(vm, id, "ThumbTexture", vec![Value::Nil])[0]])
}

fn status_bar_color(vm: &mut Vm, id: WidgetId) -> [f64; 4] {
    let stored = stored_or(vm, id, "StatusBarColor", vec![]);
    let at = |i: usize, d: f64| match stored.get(i) {
        Some(Value::Num(n)) => *n,
        _ => d,
    };
    [at(0, 1.0), at(1, 1.0), at(2, 1.0), at(3, 1.0)]
}

/// `SetStatusBarColor(r, g, b [, a])`: kept in eight bits, the texture's
/// color too.
fn set_status_bar_color(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let rgba = [2, 3, 4, 5].map(|i| match c.arg(i) {
        Value::Nil if i == 5 => 1.0,
        v => {
            let n = number(vm, v);
            widgets::byte_float(vm, n)
        }
    });
    widgets::store(vm, id, "StatusBarColor", &rgba.map(Value::Num));
    let tex = stored_or(vm, id, "StatusBarTexture", vec![])
        .first()
        .copied();
    if let Some(t) = tex.and_then(|t| widgets::widget_of(vm, t)) {
        widgets::store(vm, t, "VertexColor", &rgba.map(Value::Num));
    }
    Ok(vec![])
}

fn get_status_bar_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(status_bar_color(vm, id).map(Value::Num).to_vec())
}

pub fn methods() -> Vec<(&'static str, MethodFn)> {
    vec![
        ("Slider:SetValue", set_value),
        ("Slider:GetValue", get_value),
        ("Slider:SetMinMaxValues", set_min_max_values),
        ("Slider:GetMinMaxValues", get_min_max_values),
        ("Slider:SetValueStep", set_value_step),
        ("Slider:GetValueStep", get_value_step),
        ("Slider:GetStepsPerPage", get_steps_per_page),
        ("Slider:GetObeyStepOnDrag", get_obey_step_on_drag),
        ("Slider:GetOrientation", get_orientation),
        ("Slider:SetThumbTexture", set_thumb_texture),
        ("Slider:GetThumbTexture", get_thumb_texture),
        ("Slider:IsDraggingThumb", is_dragging_thumb),
        ("StatusBar:SetValue", set_value),
        ("StatusBar:GetValue", get_value),
        ("StatusBar:GetInterpolatedValue", get_value),
        ("StatusBar:SetMinMaxValues", set_min_max_values),
        ("StatusBar:GetMinMaxValues", get_min_max_values),
        ("StatusBar:GetOrientation", get_orientation),
        ("StatusBar:GetFillStyle", get_fill_style),
        ("StatusBar:GetReverseFill", get_reverse_fill),
        ("StatusBar:GetRotatesTexture", get_rotates_texture),
        ("StatusBar:SetTimerDuration", set_timer_duration),
        ("StatusBar:SetStatusBarTexture", set_status_bar_texture),
        ("StatusBar:GetStatusBarTexture", get_status_bar_texture),
        ("StatusBar:SetStatusBarColor", set_status_bar_color),
        ("StatusBar:GetStatusBarColor", get_status_bar_color),
    ]
}
