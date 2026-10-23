//! More widget methods that had no behaviour and no stored setter to read
//! (their getters fell through to "not mocked" for addons until a setter
//! ran): model scenes and their actors, models, cooldowns, textures,
//! animations, frames' flags, simple HTML's text types, movie frames and a
//! long tail. Each getter reads back what its setter stored (as the stored
//! setter rule keeps it), else the documented or XML default.
//!
//! The defaults are unverified unless a recording is named: they come from
//! Blizzard's API documentation (`Blizzard_APIDocumentationGenerated`),
//! the defaults of Blizzard's XML schema (`Blizzard_SharedXML/UI.xsd`),
//! how Blizzard's UI code uses the method, and Ketho's annotations for
//! the methods the documentation lacks. The engine draws no models, plays
//! no movies and has no mouse over a map, so those answer the state with
//! nothing loaded, playing or hovered.

use std::rc::Rc;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, TableRef, Value, Vm};

use crate::engine;
use crate::widgets::{self, MethodFn, WidgetId};

// ------------------------------------------------------------- helpers

fn props(vm: &Vm, id: WidgetId) -> TableRef {
    engine(vm).world.widgets[id.0].props
}

/// What setter `stem` stored, if it ran.
fn stored(vm: &mut Vm, id: WidgetId, stem: &str) -> Option<Vec<Value>> {
    let p = props(vm, id);
    widgets::stored_args(vm, p, stem)
}

/// The first value setter `stem` stored.
fn first(vm: &mut Vm, id: WidgetId, stem: &str) -> Option<Value> {
    stored(vm, id, stem).and_then(|v| v.first().copied())
}

/// What setter `stem` stored, gone (a `Clear…` method).
fn unstore(vm: &mut Vm, id: WidgetId, stem: &str) {
    let p = props(vm, id);
    let k = vm.str(stem);
    let _ = vm.heap.table_mut(p).set(k, Value::Nil);
}

/// A value as a number (a numeric string as XML gives it; a secret's
/// hidden number).
fn number(vm: &Vm, v: Value) -> Option<f64> {
    let h = vm.hidden(v);
    vm.tonumber(h)
}

fn first_num(vm: &mut Vm, id: WidgetId, stem: &str) -> Option<f64> {
    let v = first(vm, id, stem)?;
    number(vm, v)
}

/// A stored number, else `default`.
fn num_or(vm: &mut Vm, id: WidgetId, stem: &str, default: f64) -> Ret {
    Ok(vec![Value::Num(first_num(vm, id, stem).unwrap_or(default))])
}

/// A stored flag (as a boolean), else `default`.
fn flag_or(vm: &mut Vm, id: WidgetId, stem: &str, default: bool) -> Ret {
    let on = first(vm, id, stem).map_or(default, |v| v.truthy());
    Ok(vec![Value::Bool(on)])
}

/// The stored numbers of `stem`, each missing one `default`, `n` of them.
fn numbers_or(vm: &mut Vm, id: WidgetId, stem: &str, n: usize, default: f64) -> Ret {
    let mut out = stored(vm, id, stem).unwrap_or_default();
    out.truncate(n);
    for v in out.iter_mut() {
        if !matches!(v, Value::Num(_) | Value::Secret(_)) {
            *v = Value::Num(number(vm, *v).unwrap_or(default));
        }
    }
    out.resize(n, Value::Num(default));
    Ok(out)
}

/// A color as `SetXColor(r, g, b[, a])` stored it: `n` values, alpha 1
/// when set without one; else `default`.
fn color_or(vm: &mut Vm, id: WidgetId, stem: &str, n: usize, default: [f64; 4]) -> Ret {
    match stored(vm, id, stem) {
        Some(set) if !set.is_empty() => {
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let d = if i == 3 { 1.0 } else { 0.0 };
                let v = set.get(i).copied().unwrap_or(Value::Nil);
                out.push(match v {
                    Value::Num(_) | Value::Secret(_) => v,
                    other => Value::Num(number(vm, other).unwrap_or(d)),
                });
            }
            Ok(out)
        }
        _ => Ok(default[..n].iter().map(|&x| Value::Num(x)).collect()),
    }
}

/// A stored string in upper case, else `default`.
fn upper_or(vm: &mut Vm, id: WidgetId, stem: &str, default: &str) -> Ret {
    match first(vm, id, stem) {
        Some(Value::Str(s)) => {
            let up = String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase();
            Ok(vec![vm.str(&up)])
        }
        _ => Ok(vec![vm.str(default)]),
    }
}

/// A stored value of any type, else `default`.
fn value_or(vm: &mut Vm, id: WidgetId, stem: &str, default: Value) -> Ret {
    Ok(vec![first(vm, id, stem).unwrap_or(default)])
}

/// The first stored value that is a widget, else nil.
fn widget_or_nil(vm: &mut Vm, id: WidgetId, stem: &str) -> Ret {
    let v = first(vm, id, stem).filter(|v| widgets::widget_of(vm, *v).is_some());
    Ok(vec![v.unwrap_or(Value::Nil)])
}

/// The widget's text, empty without one.
fn text_of(vm: &Vm, id: WidgetId) -> String {
    match engine(vm).world.widgets[id.0].text {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    }
}

/// A global of the main environment.
fn global(vm: &mut Vm, name: &str) -> Value {
    let g = engine(vm).main_globals;
    let k = vm.str(name);
    vm.heap.table(g).get(k)
}

/// The documented empty results of method `full` (`Class:Method`), as
/// Blizzard's code gets them for a method without behaviour.
fn documented_empty(vm: &mut Vm, full: &str) -> Vec<Value> {
    let api = engine(vm).api;
    match api.methods.iter().find(|f| f.name == full) {
        Some(f) => crate::defaults(vm, api, &f.doc),
        None => vec![],
    }
}

/// A call of `id` with `args` after it, as Lua would make it.
fn call_of(vm: &Vm, id: WidgetId, args: &[Value]) -> NativeCall {
    let me = widgets::value_of(engine(vm), id);
    let all: Vec<Value> = std::iter::once(me).chain(args.iter().copied()).collect();
    NativeCall {
        args: all.iter().copied().collect(),
        taints: all.iter().map(|_| Default::default()).collect(),
        origins: all.iter().map(|_| Default::default()).collect(),
        upvals: Rc::from([]),
        upval_taints: Rc::from([]),
    }
}

/// A vector as Blizzard's code makes it (`CreateVector3D` when its UI code
/// is loaded), else a table with `x`, `y` and `z`.
fn vector3(vm: &mut Vm, x: f64, y: f64, z: f64) -> Value {
    let create = global(vm, "CreateVector3D");
    if matches!(create, Value::Func(_))
        && let Ok(v) = vm.call(create, &[Value::Num(x), Value::Num(y), Value::Num(z)])
        && let Some(&v) = v.first()
    {
        return v;
    }
    let t = vm.new_table();
    for (k, v) in [("x", x), ("y", y), ("z", z)] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, Value::Num(v));
    }
    Value::Table(t)
}

/// A color as Blizzard's code makes it (`CreateColor` when its UI code is
/// loaded), else a table with `r`, `g`, `b` and `a`.
fn color_object(vm: &mut Vm, r: f64, g: f64, b: f64, a: f64) -> Value {
    let create = global(vm, "CreateColor");
    if matches!(create, Value::Func(_))
        && let Ok(v) = vm.call(
            create,
            &[Value::Num(r), Value::Num(g), Value::Num(b), Value::Num(a)],
        )
        && let Some(&v) = v.first()
    {
        return v;
    }
    let t = vm.new_table();
    for (k, v) in [("r", r), ("g", g), ("b", b), ("a", a)] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, Value::Num(v));
    }
    Value::Table(t)
}

fn nothing(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn nil(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn yes(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

fn no(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn zero(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn empty_table(vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

/// A model's or actor's `GetItemTransmogInfoList()`: an `ItemTransmogInfo` for
/// every equipment slot (1 to 19), Blizzard's own objects
/// (`ItemUtil.CreateItemTransmogInfo`) when its code is loaded. Blizzard's
/// dressing room reads the main hand's entry unchecked, so the client's list
/// has every slot (unverified: what an undressed model's slots hold, here
/// nothing, `appearanceID` 0; the engine dresses no model).
fn transmog_info_list(vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    let list = vm.new_table();
    let util = vm.get_global("ItemUtil");
    let create = match util {
        Value::Table(t) => {
            let k = vm.str("CreateItemTransmogInfo");
            vm.heap.table(t).get(k)
        }
        _ => Value::Nil,
    };
    for slot in 1..=19 {
        let info = match create {
            Value::Func(_) => vm
                .call_from_engine(create, &[Value::Num(0.0)])
                .and_then(|r| r.first().copied())
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        let info = match info {
            Value::Table(_) => info,
            _ => {
                let t = vm.new_table();
                for key in ["appearanceID", "secondaryAppearanceID", "illusionID"] {
                    let k = vm.str(key);
                    let _ = vm.heap.table_mut(t).set(k, Value::Num(0.0));
                }
                Value::Table(t)
            }
        };
        let _ = vm.heap.table_mut(list).set(Value::Num(slot as f64), info);
    }
    Ok(vec![Value::Table(list)])
}

/// Getters of one stored value each, by stem and default: a getter's name
/// is `Get` (or `Is`, `Does`, …) and the setter's stem.
macro_rules! getters {
    ($($name:ident: $kind:ident($stem:literal, $default:expr);)*) => {
        $(
            fn $name(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
                getters!(@ $kind vm, id, $stem, $default)
            }
        )*
    };
    (@ num $vm:ident, $id:ident, $stem:literal, $default:expr) => {
        num_or($vm, $id, $stem, $default)
    };
    (@ flag $vm:ident, $id:ident, $stem:literal, $default:expr) => {
        flag_or($vm, $id, $stem, $default)
    };
    (@ upper $vm:ident, $id:ident, $stem:literal, $default:expr) => {
        upper_or($vm, $id, $stem, $default)
    };
    (@ widget $vm:ident, $id:ident, $stem:literal, $default:expr) => {
        widget_or_nil($vm, $id, $stem)
    };
}

// ---------------------------------------------------------------- frames

/// Whether the widget is protected while the player is in combat (its
/// protected state can't change then, as the protection layer refuses).
fn locked(vm: &Vm, id: WidgetId) -> bool {
    let e = engine(vm);
    e.units.lockdown && crate::widgets::protected(e, id)
}

/// `Frame:CanChangeAttribute()`: not while a protected frame is locked
/// down in combat (unverified: the documentation's name).
fn can_change_attribute(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(!locked(vm, id))])
}

getters! {
    // `SetClipsChildren`; off at first (UI.xsd `clipChildren`).
    does_clip_children: flag("ClipsChildren", false);
    // `SetHyperlinkPropagateToParent`; off (UI.xsd
    // `propagateHyperlinksToParent`).
    does_hyperlink_propagate: flag("HyperlinkPropagateToParent", false);
    // `SetDontSavePosition`; off (UI.xsd `dontSavePosition`).
    get_dont_save_position: flag("DontSavePosition", false);
    // `SetHyperlinksEnabled`; off (UI.xsd `hyperlinksEnabled`).
    get_hyperlinks_enabled: flag("HyperlinksEnabled", false);
    // `SetOnUpdateMode`; `Enum.OnUpdateMode.RunWhenVisible` (UI.xsd
    // `onUpdateMode`: `visible`).
    get_on_update_mode: num("OnUpdateMode", 1.0);
    // `SetPropagateMouseClicks` / `SetPropagateMouseMotion`; off.
    can_propagate_mouse_clicks: flag("PropagateMouseClicks", false);
    can_propagate_mouse_motion: flag("PropagateMouseMotion", false);
    // `SetCollapsesLayout`; off (UI.xsd `collapsesLayout`).
    collapses_layout: flag("CollapsesLayout", false);
    // `SetRoundLayoutToNearestPixel`; off (UI.xsd
    // `roundLayoutToNearestPixel`).
    get_round_layout: flag("RoundLayoutToNearestPixel", false);
}

/// `Frame:GetEffectivelyFlattensRenderLayers()`: its own or an ancestor's
/// `SetFlattensRenderLayers(true)` (unverified: which ancestors count).
fn get_effectively_flattens(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let mut cur = Some(id);
    for _ in 0..256 {
        let Some(w) = cur else { break };
        if first(vm, w, "FlattensRenderLayers").is_some_and(|v| v.truthy()) {
            return Ok(vec![Value::Bool(true)]);
        }
        cur = engine(vm).world.widgets[w.0].parent;
    }
    Ok(vec![Value::Bool(false)])
}

/// `Frame:GetHighestFrameLevel([iterateAllChildren])`: the highest level
/// of it and its child frames (all descendants when asked).
fn get_highest_frame_level(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let deep = c.arg(2).truthy();
    let mut best = widgets::frame_level(vm, id);
    let mut todo: Vec<WidgetId> = engine(vm).world.widgets[id.0].children.clone();
    let mut seen = 0;
    while let Some(w) = todo.pop() {
        seen += 1;
        if seen > 10_000 {
            break;
        }
        let is_frame = {
            let e = engine(vm);
            widgets::is_a(e, &e.world.widgets[w.0].class, "Frame")
        };
        if !is_frame {
            continue;
        }
        best = best.max(widgets::frame_level(vm, w));
        if deep {
            todo.extend(engine(vm).world.widgets[w.0].children.iter().copied());
        }
    }
    Ok(vec![Value::Num(best)])
}

/// `Frame:GetRaisedFrameLevel()`: its level (unverified: the engine does
/// not raise frames above their level).
fn get_raised_frame_level(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(widgets::frame_level(vm, id))])
}

/// `ScriptRegion:ShouldButtonPassThrough(button)`: whether
/// `SetPassThroughButtons` named it.
fn should_button_pass_through(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Value::Str(want) = c.arg(2) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let want = String::from_utf8_lossy(vm.str_bytes(want)).to_lowercase();
    let set = stored(vm, id, "PassThroughButtons").unwrap_or_default();
    let hit = set.iter().any(|v| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(*s)).to_lowercase() == want,
        _ => false,
    });
    Ok(vec![Value::Bool(hit)])
}

/// `ScriptRegion:GetSourceLocation()`: where it was made; the engine keeps
/// none (the documented empty string).
fn get_source_location(vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

/// `FrameScriptObject:HasAnySecretAspect()`: the engine's secret aspect is
/// an alpha set from a secret.
fn has_any_secret_aspect(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        engine(vm).world.widgets[id.0].secret_alpha,
    )])
}

/// `FrameScriptObject:HasSecretAspect(aspect)`: the alpha aspect
/// (`Enum.SecretAspect.Alpha`, 128) when its alpha was set from a secret.
fn has_secret_aspect(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let aspect = number(vm, c.arg(2)).unwrap_or(0.0) as i64;
    let secret = engine(vm).world.widgets[id.0].secret_alpha;
    Ok(vec![Value::Bool(secret && (aspect & 128) != 0)])
}

// -------------------------------------------------------------- textures

getters! {
    // `SetBlendMode`; `BLEND` (UI.xsd `alphaMode`).
    get_blend_mode: upper("BlendMode", "BLEND");
    get_radial_end_offset: num("RadialProgressBarEndOffset", 0.0);
    get_radial_feather: num("RadialProgressBarFeather", 0.0);
    get_radial_percent: num("RadialProgressBarPercent", 0.0);
    get_radial_reverse: flag("RadialProgressBarReverse", false);
    get_radial_start_offset: num("RadialProgressBarStartOffset", 0.0);
    // 0.3 as the client's float (probed in WoW Forever, 2026-10-23).
    get_texel_snapping_bias: num("TexelSnappingBias", 0.3_f32 as f64);
    // `Enum.UITextureSliceMode.Stretched`.
    get_texture_slice_mode: num("TextureSliceMode", 0.0);
    // `Line:SetThickness`; 4 (UI.xsd `thickness`).
    get_thickness: num("Thickness", 4.0);
    get_hit_rect_thickness: num("HitRectThickness", 0.0);
}

const RADIAL: [&str; 5] = [
    "RadialProgressBarEndOffset",
    "RadialProgressBarFeather",
    "RadialProgressBarPercent",
    "RadialProgressBarReverse",
    "RadialProgressBarStartOffset",
];

/// `ClearRadialProgressBar()`: its settings gone.
fn clear_radial_progress_bar(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    for stem in RADIAL {
        unstore(vm, id, stem);
    }
    Ok(vec![])
}

/// `GetTextureSliceMargins()`: left, top, right, bottom as set, else 0.
fn get_texture_slice_margins(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "TextureSliceMargins", 4, 0.0)
}

/// `ClearTextureSlice()`: no slicing.
fn clear_texture_slice(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    unstore(vm, id, "TextureSliceMargins");
    unstore(vm, id, "TextureSliceMode");
    Ok(vec![])
}

/// A vertex's index (1 to 4), else none.
fn vertex_index(vm: &Vm, v: Value) -> Option<usize> {
    match number(vm, v) {
        Some(n) if (1.0..=4.0).contains(&n) => Some(n as usize),
        _ => None,
    }
}

/// `SetVertexOffset(vertexIndex, x, y)`: kept for each vertex (the stored
/// setter rule kept one for all).
fn set_vertex_offset(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    if let Some(i) = vertex_index(vm, c.arg(2)) {
        widgets::store(vm, id, &format!("VertexOffset{i}"), &[c.arg(3), c.arg(4)]);
    }
    Ok(vec![])
}

/// `GetVertexOffset(vertexIndex)`: the vertex's offset, 0, 0 at first.
fn get_vertex_offset(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    match vertex_index(vm, c.arg(2)) {
        Some(i) => numbers_or(vm, id, &format!("VertexOffset{i}"), 2, 0.0),
        None => Ok(vec![Value::Num(0.0), Value::Num(0.0)]),
    }
}

/// `ClearVertexOffsets()`: none.
fn clear_vertex_offsets(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    for i in 1..=4 {
        unstore(vm, id, &format!("VertexOffset{i}"));
    }
    Ok(vec![])
}

// ------------------------------------------------------------ status bars

getters! {
    // `Enum.StatusBarRenderMode.Linear` (UI.xsd `renderMode`).
    get_render_mode: num("RenderMode", 0.0);
}

/// `StatusBar:GetStatusBarDesaturation()`: the strength set, else 1 when
/// `SetStatusBarDesaturated(true)`, else 0.
fn get_status_bar_desaturation(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(n) = first_num(vm, id, "StatusBarDesaturation") {
        return Ok(vec![Value::Num(n)]);
    }
    let on = first(vm, id, "StatusBarDesaturated").is_some_and(|v| v.truthy());
    Ok(vec![Value::Num(if on { 1.0 } else { 0.0 })])
}

// ----------------------------------------------------------------- buttons

/// `Button:GetPushedTextOffset()`: as set, else 0, 0.
fn get_pushed_text_offset(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "PushedTextOffset", 2, 0.0)
}

getters! {
    // `ColorSelect`'s textures: the texture set (`SetColorWheelTexture`, a
    // template's `<ColorWheelTexture>`), else nil.
    get_color_wheel_texture: widget("ColorWheelTexture", ());
    get_color_wheel_thumb_texture: widget("ColorWheelThumbTexture", ());
    get_color_value_texture: widget("ColorValueTexture", ());
    get_color_value_thumb_texture: widget("ColorValueThumbTexture", ());
    get_color_alpha_texture: widget("ColorAlphaTexture", ());
    get_color_alpha_thumb_texture: widget("ColorAlphaThumbTexture", ());
}

// ------------------------------------------------------------- cooldowns

/// The cooldown's duration set (seconds), secret when set from a secret.
fn cooldown_duration(vm: &mut Vm, id: WidgetId) -> (f64, bool) {
    let set = stored(vm, id, "Cooldown").unwrap_or_default();
    let secret = set.iter().any(|v| matches!(v, Value::Secret(_)));
    let d = set.get(1).and_then(|v| number(vm, *v)).unwrap_or(0.0);
    (d, secret)
}

/// `Cooldown:GetCooldownDisplayDuration()`: the duration shown, in
/// milliseconds as `GetCooldownTimes` gives it (unverified unit).
fn get_cooldown_display_duration(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let (d, secret) = cooldown_duration(vm, id);
    let v = Value::Num(d * 1000.0);
    Ok(vec![if secret { vm.secret_wrap(v) } else { v }])
}

getters! {
    get_countdown_abbrev_threshold: num("CountdownAbbrevThreshold", 0.0);
    get_countdown_ms_threshold: num("CountdownMillisecondsThreshold", 0.0);
    // A cooldown made in code draws its swipe and its bling (Blizzard's
    // code turns them off with `SetDrawSwipe(false)` / `SetDrawBling(
    // false)`; UI.xsd's `false` is the XML attribute's; unverified).
    get_draw_swipe: flag("DrawSwipe", true);
    get_draw_bling: flag("DrawBling", true);
    // UI.xsd `drawEdge`.
    get_draw_edge: flag("DrawEdge", false);
    get_edge_scale: num("EdgeScale", 1.0);
    // UI.xsd `minimumCountdownDuration`.
    get_minimum_countdown_duration: num("MinimumCountdownDuration", 2000.0);
    // UI.xsd `reverse`.
    get_reverse: flag("Reverse", false);
    get_use_aura_display_time: flag("UseAuraDisplayTime", false);
}

/// `Cooldown:GetCountdownFormatter()`: the formatter set, else nil.
fn get_countdown_formatter(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    value_or(vm, id, "CountdownFormatter", Value::Nil)
}

// ------------------------------------------------------------ edit boxes

getters! {
    // UI.xsd `blinkSpeed`.
    get_blink_speed: num("BlinkSpeed", 0.5);
    get_max_bytes: num("MaxBytes", 0.0);
    get_visible_text_byte_limit: num("VisibleTextByteLimit", 0.0);
}

/// `EditBox:GetDisplayText()`: the text as shown: a password's as stars
/// (unverified: the client's mask character).
fn get_display_text(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let text = text_of(vm, id);
    let password = first(vm, id, "Password").is_some_and(|v| v.truthy());
    let shown = if password {
        "*".repeat(text.chars().count())
    } else {
        text
    };
    Ok(vec![vm.str(&shown)])
}

/// `EditBox:GetHighlightColor()`: as set (alpha 1 without one), else the
/// documented empty 0, 0, 0, 0.
fn get_highlight_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    color_or(vm, id, "HighlightColor", 4, [0.0; 4])
}

// ---------------------------------------------------------- font strings

getters! {
    // `Enum.FontStringScaleAnimationMode.FontSize` (UI.xsd
    // `scaleAnimationMode`).
    get_scale_animation_mode: num("ScaleAnimationMode", 0.0);
    // UI.xsd `smoothScaling`.
    get_smooth_scaling: flag("SmoothScaling", false);
    get_field_size: num("FieldSize", 0.0);
}

/// `FontString:GetUnboundedStringWidthForText(text)`: the text's width in
/// the string's font (`text.rs`, decision T237).
fn get_unbounded_width_for_text(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let text = match c.arg(2) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => wowlua_vm::number::fmt_num(n),
        _ => String::new(),
    };
    Ok(vec![Value::Num(crate::text::width(vm, id, &text))])
}

/// `FontString:FindCharacterIndexAtCoordinate(x, y)`: the character under
/// `x` on the text's one line, and whether `x` is within the text
/// (`text.rs`; unverified against the client).
fn find_character_index(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let text = text_of(vm, id);
    let x = number(vm, c.arg(2)).unwrap_or(0.0);
    let (index, inside) = crate::text::index_at(vm, id, &text, x);
    Ok(vec![Value::Num(index as f64), Value::Bool(inside)])
}

/// `SetAlphaGradient(...)` (a font string's start and length, a frame's
/// index and gradient): kept for its getters, then the engine's own
/// behaviour (a font string's answer whether the text reaches it).
fn set_alpha_gradient(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    widgets::store(vm, id, "AlphaGradient", &[c.arg(2), c.arg(3)]);
    match engine(vm).method_behaviour("", "SetAlphaGradient") {
        Some(f) => f(vm, id, c),
        None => Ok(vec![]),
    }
}

/// `FontString:GetAlphaGradient()`: start and length as set, 0, 0 at
/// first.
fn get_alpha_gradient(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "AlphaGradient", 2, 0.0)
}

/// `ClearAlphaGradient()`: none.
fn clear_alpha_gradient(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    unstore(vm, id, "AlphaGradient");
    Ok(vec![])
}

/// `Frame:HasAlphaGradient()`: after `SetAlphaGradient`, until cleared.
fn has_alpha_gradient(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(stored(vm, id, "AlphaGradient").is_some())])
}

// --------------------------------------------------------- message frames

getters! {
    // UI.xsd `fadePower`.
    get_fade_power: num("FadePower", 1.0);
}

/// `MessageFrame:HasMessageByID(messageID)`: whether a message added with
/// that ID is still there.
fn has_message_by_id(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some(wanted) = number(vm, c.arg(2)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let has = engine(vm)
        .world
        .messages
        .get(&id.0)
        .is_some_and(|list| list.iter().any(|(m, _)| *m == Some(wanted)));
    Ok(vec![Value::Bool(has)])
}

// ------------------------------------------------------------ simple HTML

/// A text type (`P`, `H1`, `H2`, `H3`) in upper case, if `v` is one.
fn text_type(vm: &Vm, v: Value) -> Option<String> {
    let Value::Str(s) = v else { return None };
    let t = String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase();
    matches!(t.as_str(), "P" | "H1" | "H2" | "H3").then_some(t)
}

/// The stem a simple HTML's setting of `text_type` is kept under (all
/// types' when set without one).
fn html_stem(stem: &str, text_type: Option<&str>) -> String {
    format!("\u{0}HTML{stem}:{}", text_type.unwrap_or(""))
}

/// A simple HTML's setter `SetX([textType,] values...)`: the values kept
/// for that text type, or for all when its first value is not one (as
/// Blizzard's XML loader calls them).
fn html_set(vm: &mut Vm, id: WidgetId, c: &NativeCall, stem: &str) -> Ret {
    let args: Vec<Value> = c.args.iter().skip(1).copied().collect();
    let tt = args.first().and_then(|v| text_type(vm, *v));
    // `SetJustifyH("LEFT")` names no text type; `SetJustifyH("P", "LEFT")`
    // does.
    let (tt, values) = match tt {
        Some(t) if args.len() > 1 => (Some(t), args[1..].to_vec()),
        _ => (None, args),
    };
    widgets::store(vm, id, &html_stem(stem, tt.as_deref()), &values);
    // The plain setting too, as the stored setter rule keeps it.
    widgets::store(vm, id, stem, &values);
    Ok(vec![])
}

/// What a simple HTML's setter `stem` kept for the text type asked
/// (`c`'s first argument), else for all types.
fn html_get(vm: &mut Vm, id: WidgetId, c: &NativeCall, stem: &str) -> Option<Vec<Value>> {
    let tt = text_type(vm, c.arg(2));
    if let Some(t) = tt.as_deref()
        && let Some(v) = stored(vm, id, &html_stem(stem, Some(t)))
    {
        return Some(v);
    }
    stored(vm, id, &html_stem(stem, None))
}

/// What the simple HTML's font object (`SetFontObject`) has for `stem`.
fn html_font_setting(vm: &mut Vm, id: WidgetId, stem: &str) -> Option<Vec<Value>> {
    let mut cur = first(vm, id, "FontObject").and_then(|f| widgets::widget_of(vm, f))?;
    for _ in 0..32 {
        let p = props(vm, cur);
        if let Some(v) = widgets::stored_args(vm, p, stem) {
            return Some(v);
        }
        let next = widgets::stored_args(vm, p, "FontObject")
            .and_then(|v| v.first().copied())
            .and_then(|f| widgets::widget_of(vm, f));
        match next {
            Some(n) if n != cur => cur = n,
            _ => break,
        }
    }
    None
}

macro_rules! html_setters {
    ($($name:ident: $stem:literal;)*) => {
        $(
            fn $name(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
                html_set(vm, id, &c, $stem)
            }
        )*
    };
}

html_setters! {
    html_set_text_color: "TextColor";
    html_set_shadow_color: "ShadowColor";
    html_set_shadow_offset: "ShadowOffset";
    html_set_spacing: "Spacing";
    html_set_justify_h: "JustifyH";
    html_set_justify_v: "JustifyV";
    html_set_indented_word_wrap: "IndentedWordWrap";
}

/// A color of a text type: set (alpha 1 without one), else its font
/// object's, else `default` for each of r, g, b and a.
fn html_color(vm: &mut Vm, id: WidgetId, c: &NativeCall, stem: &str, default: f64) -> Ret {
    let set = html_get(vm, id, c, stem).or_else(|| html_font_setting(vm, id, stem));
    match set {
        Some(mut out) if !out.is_empty() => {
            out.truncate(4);
            let len = out.len().max(3);
            out.resize(len, Value::Num(0.0));
            if out.len() < 4 {
                out.push(Value::Num(1.0));
            }
            Ok(out)
        }
        _ => Ok(vec![Value::Num(default); 4]),
    }
}

/// `SimpleHTML:GetTextColor(textType)`: white at first (a font string's
/// recorded fresh color).
fn html_get_text_color(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    html_color(vm, id, &c, "TextColor", 1.0)
}

/// `SimpleHTML:GetShadowColor(textType)`: none at first (0, 0, 0, 0).
fn html_get_shadow_color(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    html_color(vm, id, &c, "ShadowColor", 0.0)
}

/// `SimpleHTML:GetShadowOffset(textType)`: as set, else 0, 0.
fn html_get_shadow_offset(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let mut v = html_get(vm, id, &c, "ShadowOffset")
        .or_else(|| html_font_setting(vm, id, "ShadowOffset"))
        .unwrap_or_default();
    v.truncate(2);
    v.resize(2, Value::Num(0.0));
    Ok(v)
}

/// `SimpleHTML:GetSpacing(textType)`: as set, else 0.
fn html_get_spacing(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let v = html_get(vm, id, &c, "Spacing")
        .or_else(|| html_font_setting(vm, id, "Spacing"))
        .and_then(|v| v.first().copied())
        .unwrap_or(Value::Num(0.0));
    Ok(vec![v])
}

/// A justification of a text type: set, else its font object's, else
/// `default` (UI.xsd: a font's `justifyH` `CENTER`, `justifyV` `MIDDLE`).
fn html_justify(vm: &mut Vm, id: WidgetId, c: &NativeCall, stem: &str, default: &str) -> Ret {
    let v = html_get(vm, id, c, stem)
        .or_else(|| html_font_setting(vm, id, stem))
        .and_then(|v| v.first().copied());
    match v {
        Some(Value::Str(s)) => {
            let up = String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase();
            Ok(vec![vm.str(&up)])
        }
        _ => Ok(vec![vm.str(default)]),
    }
}

fn html_get_justify_h(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    html_justify(vm, id, &c, "JustifyH", "CENTER")
}

fn html_get_justify_v(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    html_justify(vm, id, &c, "JustifyV", "MIDDLE")
}

/// `SimpleHTML:GetIndentedWordWrap(textType)`: as set, else off (UI.xsd
/// `indented`).
fn html_get_indented_word_wrap(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let on = html_get(vm, id, &c, "IndentedWordWrap")
        .or_else(|| html_font_setting(vm, id, "IndentedWordWrap"))
        .and_then(|v| v.first().copied())
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(on)])
}

/// `SimpleHTML:GetFontObject(textType)`: the font object set (the engine
/// keeps one for all text types), else nil.
fn html_get_font_object(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widget_or_nil(vm, id, "FontObject")
}

/// `SimpleHTML:GetHyperlinkFormat()`: as set, else `|H%s|h%s|h` (UI.xsd
/// `hyperlinkFormat`).
fn html_get_hyperlink_format(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    match first(vm, id, "HyperlinkFormat") {
        Some(v @ Value::Str(_)) => Ok(vec![v]),
        _ => Ok(vec![vm.str("|H%s|h%s|h")]),
    }
}

/// The text of simple HTML markup as lines: tags dropped, a paragraph,
/// heading or `<br/>` ending a line.
fn html_lines(markup: &str) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut rest = markup;
    while let Some(start) = rest.find('<') {
        lines.last_mut().expect("a line").push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_lowercase();
        let name = tag.trim_start_matches('/').trim_end_matches('/').trim();
        let name = name.split_whitespace().next().unwrap_or("");
        let breaks =
            name == "br" || (tag.starts_with('/') && matches!(name, "p" | "h1" | "h2" | "h3"));
        if breaks {
            lines.push(String::new());
        }
        rest = &rest[start + end + 1..];
    }
    lines.last_mut().expect("a line").push_str(rest);
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        for part in l.split('\n') {
            out.push(part.to_string());
        }
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    while out.first().is_some_and(|l| l.trim().is_empty()) {
        out.remove(0);
    }
    out
}

/// `SimpleHTML:GetContentHeight()`: its text's lines (wrapped at its laid
/// out width as the client lays text out, `text.rs`) at its font's
/// height, as `GetStringHeight` measures a font string; 0 without text.
fn html_get_content_height(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let text = text_of(vm, id);
    let lines = html_lines(&text);
    if lines.is_empty() {
        return Ok(vec![Value::Num(0.0)]);
    }
    let width = widgets::laid_out_size(vm, id).0;
    let max = (width > 0.0).then_some(width);
    let n: usize = lines
        .iter()
        .map(|l| crate::text::wrapped_lines(vm, id, l, max).len())
        .sum();
    let call = call_of(vm, id, &[]);
    let size = widgets::get_font(vm, id, call)
        .ok()
        .and_then(|f| f.get(1).copied())
        .and_then(|v| number(vm, v))
        .filter(|h| *h > 0.0)
        // A font string's font without one set (unverified).
        .unwrap_or(12.0);
    let spacing = html_font_setting(vm, id, "Spacing")
        .or_else(|| stored(vm, id, "Spacing"))
        .and_then(|v| v.first().copied())
        .and_then(|v| number(vm, v))
        .unwrap_or(0.0);
    let n = n as f64;
    Ok(vec![Value::Num(n * size + (n - 1.0) * spacing)])
}

// ---------------------------------------------------------------- models

getters! {
    get_camera_distance: num("CameraDistance", 0.0);
    get_camera_facing: num("CameraFacing", 0.0);
    get_camera_roll: num("CameraRoll", 0.0);
    // UI.xsd `fogNear`, `fogFar` (a model's and a model scene's).
    get_fog_near: num("FogNear", 0.0);
    get_fog_far: num("FogFar", 1.0);
    get_model_alpha: num("ModelAlpha", 1.0);
    get_model_paused: flag("Paused", false);
    get_shadow_effect: num("ShadowEffect", 0.0);
    get_display_info: num("DisplayInfo", 0.0);
    get_do_blend: flag("DoBlend", false);
    get_keep_model_on_hide: flag("KeepModelOnHide", false);
    get_auto_dress: flag("AutoDress", false);
    get_obey_hide_in_transmog: flag("ObeyHideInTransmogFlag", false);
    get_use_transmog_choices: flag("UseTransmogChoices", false);
    get_use_transmog_skin: flag("UseTransmogSkin", false);
    // A model scene's.
    get_camera_far_clip: num("CameraFarClip", 0.0);
    get_camera_near_clip: num("CameraNearClip", 0.0);
    get_camera_field_of_view: num("CameraFieldOfView", 0.0);
    get_allow_overlapped_models: flag("AllowOverlappedModels", false);
    is_light_visible: flag("LightVisible", false);
    get_light_type: num("LightType", 0.0);
    // An actor's.
    get_animation_blend_operation: num("AnimationBlendOperation", 0.0);
    get_spell_visual_kit: num("SpellVisualKit", 0.0);
}

/// `Model:GetFogColor()`: r, g, b, a as set (alpha 1 without one), else
/// black.
fn get_model_fog_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    color_or(vm, id, "FogColor", 4, [0.0, 0.0, 0.0, 1.0])
}

/// `ModelScene:GetFogColor()`: r, g, b as set, else black.
fn get_scene_fog_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    color_or(vm, id, "FogColor", 3, [0.0; 4])
}

/// `ClearFog()`: no fog settings.
fn clear_fog(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    for stem in ["FogColor", "FogNear", "FogFar"] {
        unstore(vm, id, stem);
    }
    Ok(vec![])
}

/// `Model:GetModelDrawLayer()`: the layer set (sublevel 0), else `ARTWORK`
/// (UI.xsd `drawLayer`).
fn get_model_draw_layer(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let layer = match first(vm, id, "ModelDrawLayer") {
        Some(v @ Value::Str(_)) => v,
        _ => vm.str("ARTWORK"),
    };
    Ok(vec![layer, Value::Num(0.0)])
}

/// `Model:GetViewTranslation()` / a scene's: as set, else 0, 0.
fn get_view_translation(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "ViewTranslation", 2, 0.0)
}

/// `Model:GetWorldScale()`: 1 (the engine scales no models).
fn get_world_scale(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1.0)])
}

/// `Model:GetLight()`: what `SetLight` set, else off with the documented
/// empty light.
fn get_light(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(set) = stored(vm, id, "Light")
        && !set.is_empty()
    {
        let on = set.first().is_some_and(|v| v.truthy());
        let light = match set.get(1).copied() {
            Some(v @ Value::Table(_)) => v,
            _ => Value::Table(vm.new_table()),
        };
        return Ok(vec![Value::Bool(on), light]);
    }
    let mut out = documented_empty(vm, "Model:GetLight");
    out.resize(2, Value::Nil);
    out[0] = Value::Bool(false);
    if out[1].is_nil() {
        out[1] = Value::Table(vm.new_table());
    }
    Ok(out)
}

/// `Model:MakeCurrentCameraCustom()`: its camera custom from now on.
fn make_current_camera_custom(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widgets::store(vm, id, "\u{0}CustomCamera", &[Value::Bool(true)]);
    Ok(vec![])
}

/// `Model:HasCustomCamera()`: after `MakeCurrentCameraCustom` or
/// `SetCustomCamera`.
fn has_custom_camera(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let custom = first(vm, id, "\u{0}CustomCamera").is_some_and(|v| v.truthy())
        || stored(vm, id, "CustomCamera").is_some();
    Ok(vec![Value::Bool(custom)])
}

/// `Model:TransformCameraSpaceToModelSpace(position)`: the position as a
/// vector (unverified: the engine's model space is its camera space, no
/// model being loaded).
fn transform_camera_to_model(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    let v = c.arg(2);
    let mut xyz = [0.0; 3];
    if let Value::Table(t) = v {
        for (i, k) in ["x", "y", "z"].iter().enumerate() {
            let key = vm.str(k);
            let n = vm.heap.table(t).get(key);
            xyz[i] = number(vm, n).unwrap_or(0.0);
        }
    }
    Ok(vec![vector3(vm, xyz[0], xyz[1], xyz[2])])
}

/// A model scene's camera axes: forward, right, up.
pub(crate) fn scene_axes(vm: &mut Vm, id: WidgetId) -> [[f64; 3]; 3] {
    let axes = stored(vm, id, "CameraOrientationByAxisVectors").unwrap_or_default();
    let ypr = stored(vm, id, "CameraOrientationByYawPitchRoll").unwrap_or_default();
    let vm: &Vm = vm;
    if axes.len() >= 9 {
        let n: Vec<f64> = axes.iter().map(|x| number(vm, *x).unwrap_or(0.0)).collect();
        return [[n[0], n[1], n[2]], [n[3], n[4], n[5]], [n[6], n[7], n[8]]];
    }
    let a = |i: usize| ypr.get(i).and_then(|x| number(vm, *x)).unwrap_or(0.0);
    let (yaw, pitch, roll) = (a(0), a(1), a(2));
    let (cy, sy, cp, sp, cr, sr) = (
        yaw.cos(),
        yaw.sin(),
        pitch.cos(),
        pitch.sin(),
        roll.cos(),
        roll.sin(),
    );
    // x forward, y left, z up: yaw about z, pitch about y, roll about x
    // (unverified signs).
    let forward = [cy * cp, sy * cp, -sp];
    let left = [cy * sp * sr - sy * cr, sy * sp * sr + cy * cr, cp * sr];
    let up = [cy * sp * cr + sy * sr, sy * sp * cr - cy * sr, cp * cr];
    let right = [-left[0], -left[1], -left[2]];
    [forward, right, up]
}

fn axis(vm: &mut Vm, id: WidgetId, i: usize) -> Ret {
    let a = scene_axes(vm, id)[i];
    Ok(a.iter().map(|&x| Value::Num(x)).collect())
}

/// `ModelScene:GetCameraForward()` / `GetCameraRight()` / `GetCameraUp()`:
/// the axes `SetCameraOrientationByAxisVectors` set, else those of the
/// yaw, pitch and roll set (forward along x at first).
fn get_camera_forward(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    axis(vm, id, 0)
}

fn get_camera_right(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    axis(vm, id, 1)
}

fn get_camera_up(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    axis(vm, id, 2)
}

/// `ModelScene:GetLightAmbientColor()` and the light's other values: as
/// set, else 0s.
fn get_light_ambient_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "LightAmbientColor", 3, 0.0)
}

fn get_light_diffuse_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "LightDiffuseColor", 3, 0.0)
}

fn get_light_direction(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "LightDirection", 3, 0.0)
}

fn get_light_position(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "LightPosition", 3, 0.0)
}

/// `ModelScene:Project3DPointTo2D(x, y, z)`: the scene's center at no
/// depth (unverified: the engine draws no 3D scene).
fn project_3d_point(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let (x, y) = match widgets::bounds(vm, id, 0) {
        Some((l, b, w, h)) if !engine(vm).world.widgets[id.0].points.is_empty() => {
            (l + w / 2.0, b + h / 2.0)
        }
        _ => (0.0, 0.0),
    };
    Ok(vec![Value::Num(x), Value::Num(y), Value::Num(0.0)])
}

/// An actor's bounding boxes: none while no model is loaded (Blizzard's
/// `ModelSceneActorMixin`: "Could be nil for invisible models").
fn bounding_box(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `GetAnimationVariation()`: `SetAnimation`'s variation, else 0.
fn get_animation_variation(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let v = stored(vm, id, "Animation")
        .and_then(|v| v.get(1).copied())
        .and_then(|v| number(vm, v))
        .unwrap_or(0.0);
    Ok(vec![Value::Num(v)])
}

/// `GetModelPath()`: the path `SetModelByPath` set, else "".
fn get_model_path(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    match first(vm, id, "ModelByPath") {
        Some(v @ Value::Str(_)) => Ok(vec![v]),
        _ => Ok(vec![vm.str("")]),
    }
}

/// `GetModelUnitGUID()`: the GUID of the unit `SetModelByUnit` showed (as
/// `UnitGUID` answers), else "".
fn get_model_unit_guid(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(unit @ Value::Str(_)) = first(vm, id, "ModelByUnit") {
        let f = global(vm, "UnitGUID");
        if matches!(f, Value::Func(_))
            && let Ok(r) = vm.call(f, &[unit])
            && let Some(v @ Value::Str(_)) = r.first().copied()
        {
            return Ok(vec![v]);
        }
    }
    Ok(vec![vm.str("")])
}

/// `GetParticleOverrideScale()`: as set, else nil.
fn get_particle_override_scale(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    value_or(vm, id, "ParticleOverrideScale", Value::Nil)
}

/// `IsUsingCenterForOrigin()`: x, y, z as `SetUseCenterForOrigin` set.
fn is_using_center_for_origin(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let set = stored(vm, id, "UseCenterForOrigin").unwrap_or_default();
    Ok((0..3)
        .map(|i| Value::Bool(set.get(i).is_some_and(|v| v.truthy())))
        .collect())
}

/// An actor's `GetPaused()`: paused and whether that pauses globally, as
/// `SetPaused(paused, affectsGlobalPause = true)` set.
fn get_actor_paused(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let set = stored(vm, id, "Paused").unwrap_or_default();
    let paused = set.first().is_some_and(|v| v.truthy());
    let global = paused && set.get(1).is_none_or(|v| v.is_nil() || v.truthy());
    Ok(vec![Value::Bool(paused), Value::Bool(global)])
}

/// `CalculateMountScale(rider)`: 1 (no models loaded to fit; unverified).
fn calculate_mount_scale(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1.0)])
}

/// `DressUpModel:GetItemTransmogInfo(slot)`: the documented empty info (no
/// items tried on in the engine).
fn dress_up_item_transmog_info(vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(documented_empty(vm, "DressUpModel:GetItemTransmogInfo"))
}

/// `MapScene:GetModelDrawLayer()`: as set, else `ARTWORK`.
fn get_map_scene_draw_layer(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    get_model_draw_layer(vm, id, c)
}

// ------------------------------------------------------------ animations

getters! {
    // UI.xsd `fromAlpha`, `toAlpha`.
    get_from_alpha: num("FromAlpha", 0.0);
    get_to_alpha: num("ToAlpha", 1.0);
    // UI.xsd `fromPercent`, `toPercent`.
    get_from_percent: num("FromPercent", 0.0);
    get_to_percent: num("ToPercent", 0.0);
    // UI.xsd `smoothing`.
    get_smoothing: upper("Smoothing", "NONE");
    // UI.xsd `flipBookFrameWidth`, `flipBookFrameHeight`.
    get_flip_book_frame_width: num("FlipBookFrameWidth", 0.0);
    get_flip_book_frame_height: num("FlipBookFrameHeight", 0.0);
    // UI.xsd `curve`.
    get_curve_type: upper("CurveType", "NONE");
}

/// `Animation:GetRegionParent()`: the region its group belongs to.
fn get_region_parent(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let e = engine(vm);
    let owner = e.world.widgets[id.0]
        .parent
        .and_then(|g| e.world.widgets[g.0].parent);
    Ok(vec![match owner {
        Some(o) => widgets::value_of(e, o),
        None => Value::Nil,
    }])
}

/// `AnimationGroup:GetLoopState()`: `FORWARD` while a looping group plays,
/// else `NONE` (unverified: a bouncing group's way back, `REVERSE`).
fn get_loop_state(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let playing = engine(vm).animations.playing.contains_key(&id.0);
    let looping = match first(vm, id, "Looping") {
        Some(Value::Str(s)) => {
            !String::from_utf8_lossy(vm.str_bytes(s)).eq_ignore_ascii_case("NONE")
        }
        _ => false,
    };
    Ok(vec![vm.str(if playing && looping {
        "FORWARD"
    } else {
        "NONE"
    })])
}

/// `Rotation:SetDegrees(angle)` / `SetRadians(angle)`: the angle, read in
/// either unit.
fn set_degrees(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let d = number(vm, c.arg(2)).unwrap_or(0.0);
    widgets::store(vm, id, "Degrees", &[Value::Num(d)]);
    widgets::store(vm, id, "Radians", &[Value::Num(d.to_radians())]);
    Ok(vec![])
}

fn set_radians(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let r = number(vm, c.arg(2)).unwrap_or(0.0);
    widgets::store(vm, id, "Radians", &[Value::Num(r)]);
    widgets::store(vm, id, "Degrees", &[Value::Num(r.to_degrees())]);
    Ok(vec![])
}

getters! {
    // UI.xsd `degrees`, `radians`.
    get_degrees: num("Degrees", 0.0);
    get_radians: num("Radians", 0.0);
}

/// `Rotation:GetOrigin()` / `Scale:GetOrigin()`: as set, else the center
/// (UI.xsd `Origin`'s `point`).
fn get_origin(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let set = stored(vm, id, "Origin").unwrap_or_default();
    let point = match set.first().copied() {
        Some(Value::Str(s)) => {
            let up = String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase();
            vm.str(&up)
        }
        _ => vm.str("CENTER"),
    };
    let x = set.get(1).and_then(|v| number(vm, *v)).unwrap_or(0.0);
    let y = set.get(2).and_then(|v| number(vm, *v)).unwrap_or(0.0);
    Ok(vec![point, Value::Num(x), Value::Num(y)])
}

/// `Scale:GetScaleFrom()` / `GetScaleTo()`: as set, else 1, 1 (UI.xsd
/// `fromScaleX` and the rest).
fn get_scale_from(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "ScaleFrom", 2, 1.0)
}

fn get_scale_to(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "ScaleTo", 2, 1.0)
}

/// `Translation:GetOffset()`, `TextureCoordTranslation:GetOffset()`,
/// `ControlPoint:GetOffset()`: as set, else 0, 0 (UI.xsd `offsetX`).
fn get_offset(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "Offset", 2, 0.0)
}

/// `VertexColor:GetStartColor()` / `GetEndColor()`: the color set, else
/// white (unverified).
fn vertex_color(vm: &mut Vm, id: WidgetId, stem: &str) -> Ret {
    match first(vm, id, stem) {
        Some(v @ Value::Table(_)) => Ok(vec![v]),
        _ => Ok(vec![color_object(vm, 1.0, 1.0, 1.0, 1.0)]),
    }
}

fn get_start_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    vertex_color(vm, id, "StartColor")
}

fn get_end_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    vertex_color(vm, id, "EndColor")
}

/// A path's control points, in creation order.
fn control_points(vm: &Vm, path: WidgetId) -> Vec<WidgetId> {
    let e = engine(vm);
    e.world.widgets[path.0]
        .children
        .iter()
        .copied()
        .filter(|c| widgets::is_a(e, &e.world.widgets[c.0].class, "ControlPoint"))
        .collect()
}

/// `Path:GetControlPoints()`: each of its control points.
fn get_control_points(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let points = control_points(vm, id);
    let e = engine(vm);
    Ok(points
        .into_iter()
        .map(|p| widgets::value_of(e, p))
        .collect())
}

/// `Path:GetMaxControlPointOrder()`: the highest order of its points, 0
/// without one.
fn get_max_control_point_order(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let mut best: f64 = 0.0;
    for p in control_points(vm, id) {
        best = best.max(first_num(vm, p, "Order").unwrap_or(0.0));
    }
    Ok(vec![Value::Num(best)])
}

// ------------------------------------------------------- the tail: frames

/// `Minimap:GetPingPosition()`: the last ping's place from the player now,
/// in widths of the minimap's view (east, north): the client keeps a ping
/// where it was in the world (probed in WoW Forever, 2026-10-23: a click
/// up and right of the center gave 0.174, 0.260 at zoom 0, its offset over
/// the minimap's width; one from before a flight, -5.76); none, 0, 0.
fn get_ping_position(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let ping = first(vm, id, "MinimapPing");
    let (Some(Value::Table(t)), Some((pn, pw, _, _))) = (ping, crate::character::player_world(vm))
    else {
        return Ok(vec![Value::Num(0.0), Value::Num(0.0)]);
    };
    let get = |i: f64| match vm.heap.table(t).get(Value::Num(i)) {
        Value::Num(n) => n,
        _ => 0.0,
    };
    let (north, west) = (get(1.0), get(2.0));
    let span = crate::scene::MINIMAP_YARDS[crate::scene::minimap_zoom(vm)];
    Ok(vec![
        Value::Num((pw - west) / span),
        Value::Num((north - pn) / span),
    ])
}

/// `Minimap:PingLocation(x, y)`: the player pings that place (UI units
/// from the minimap's center, as Blizzard's minimap click gives them): the
/// place in the world (at the minimap's zoom, turned back with
/// `rotateMinimap`) kept with the time, for `GetPingPosition` and the
/// minimap to draw (`scene::minimap_ping`), and `MINIMAP_PING("player",
/// x, y)` (the documentation names the payload y, x; the order
/// unverified), a callback event on camelot: `RegisterEvent` refuses it
/// (probed 2026-10-23), `RegisterEventCallback` gets it.
fn ping_location(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (Value::Num(x), Value::Num(y)) = (vm.hidden(c.arg(2)), vm.hidden(c.arg(3))) else {
        return Ok(vec![]);
    };
    let Some((pn, pw, _, _)) = crate::character::player_world(vm) else {
        return Ok(vec![]);
    };
    let width = crate::widgets::bounds(vm, id, 0)
        .map_or(140.0, |b| b.2)
        .max(1.0);
    let ypu = crate::scene::MINIMAP_YARDS[crate::scene::minimap_zoom(vm)] / width;
    // East and north of the player, the minimap's turn undone.
    let (mut east, mut up) = (x * ypu, y * ypu);
    if crate::scene::minimap_rotates(vm) {
        let (sin, cos) = crate::unitinfo::player_facing(vm).sin_cos();
        // The minimap turned clockwise by the facing: turned back.
        (east, up) = (east * cos - up * sin, east * sin + up * cos);
    }
    let t = vm.new_table();
    let now = crate::clock::now(vm);
    for (i, v) in [pn + up, pw - east, now].into_iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(i as f64 + 1.0), Value::Num(v));
    }
    crate::widgets::store(vm, id, "MinimapPing", &[Value::Table(t)]);
    let player = vm.str("player");
    crate::widgets::fire_event(vm, "MINIMAP_PING", &[player, Value::Num(x), Value::Num(y)]);
    Ok(vec![])
}

/// `NamePlate:GetHitTestPoints()`: what `SetHitTestPoints` set, else none.
fn get_hit_test_points(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    match first(vm, id, "HitTestPoints") {
        Some(v @ Value::Table(_)) => Ok(vec![v]),
        _ => Ok(vec![Value::Table(vm.new_table())]),
    }
}

getters! {
    get_player_ping_scale: num("PlayerPingScale", 1.0);
    get_mask_scalar: num("MaskScalar", 1.0);
    get_max_snapshots: num("MaxSnapshots", 0.0);
    get_custom_line_spacing: num("CustomLineSpacing", 0.0);
}

/// `FogOfWarFrame`'s atlases: as set, else "".
fn get_fog_background_atlas(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let d = vm.str("");
    value_or(vm, id, "FogOfWarBackgroundAtlas", d)
}

fn get_fog_mask_atlas(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let d = vm.str("");
    value_or(vm, id, "FogOfWarMaskAtlas", d)
}

/// `FogOfWarFrame`'s textures: as set, else nil.
fn get_fog_background_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    value_or(vm, id, "FogOfWarBackgroundTexture", Value::Nil)
}

fn get_fog_mask_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    value_or(vm, id, "FogOfWarMaskTexture", Value::Nil)
}

/// `MovieFrame:StartMovie(...)`: no movie plays in the engine (false and
/// no error code; Blizzard's `MovieFrameMixin:PlayMovie` then ends it as
/// failed, as it did with no answer).
fn start_movie(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false), Value::Num(0.0)])
}

/// `QuestPOIFrame:UpdateMouseOverTooltip(x, y)`: no quest under the mouse.
fn quest_poi_update_tooltip(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil, Value::Nil])
}

/// `UnitPositionFrame:GetMouseOverUnits()`: no unit under the mouse.
fn get_mouse_over_units(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `SimpleHTML:GetTextData()`: the documented empty list (unverified: the
/// engine does not parse its markup into nodes).
fn html_get_text_data(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    empty_table(vm, id, c)
}

// ------------------------------------------------------------------ list

pub fn methods() -> Vec<(&'static str, MethodFn)> {
    let mut out: Vec<(&'static str, MethodFn)> = vec![
        // Frames and regions.
        ("Frame:CanChangeAttribute", can_change_attribute as MethodFn),
        ("Frame:DoesClipChildren", does_clip_children),
        (
            "Frame:DoesHyperlinkPropagateToParent",
            does_hyperlink_propagate,
        ),
        ("Frame:GetDontSavePosition", get_dont_save_position),
        (
            "Frame:GetEffectivelyFlattensRenderLayers",
            get_effectively_flattens,
        ),
        ("Frame:GetHighestFrameLevel", get_highest_frame_level),
        ("Frame:GetHyperlinksEnabled", get_hyperlinks_enabled),
        ("Frame:GetOnUpdateMode", get_on_update_mode),
        ("Frame:GetRaisedFrameLevel", get_raised_frame_level),
        ("Frame:GetRolesetNames", empty_table),
        ("Frame:SetAlphaGradient", set_alpha_gradient),
        ("Frame:HasAlphaGradient", has_alpha_gradient),
        ("Frame:ClearAlphaGradient", clear_alpha_gradient),
        ("Frame:InterceptStartDrag", no),
        (
            "ScriptRegion:CanPropagateMouseClicks",
            can_propagate_mouse_clicks,
        ),
        (
            "ScriptRegion:CanPropagateMouseMotion",
            can_propagate_mouse_motion,
        ),
        ("ScriptRegion:CollapsesLayout", collapses_layout),
        (
            "ScriptRegion:GetRoundLayoutToNearestPixel",
            get_round_layout,
        ),
        ("ScriptRegion:GetSourceLocation", get_source_location),
        (
            "ScriptRegion:ShouldButtonPassThrough",
            should_button_pass_through,
        ),
        ("FrameScriptObject:GetAccessRestrictions", zero),
        ("FrameScriptObject:GetForbiddenAspects", zero),
        (
            "FrameScriptObject:AddForbiddenAspects",
            crate::protection::add_forbidden_aspects,
        ),
        ("FrameScriptObject:HasAnyAccessRestrictions", no),
        (
            "FrameScriptObject:HasAnySecretAspect",
            has_any_secret_aspect,
        ),
        ("FrameScriptObject:HasSecretAspect", has_secret_aspect),
        // Textures and lines.
        ("TextureBase:GetBlendMode", get_blend_mode),
        (
            "TextureBase:GetRadialProgressBarEndOffset",
            get_radial_end_offset,
        ),
        (
            "TextureBase:GetRadialProgressBarFeather",
            get_radial_feather,
        ),
        (
            "TextureBase:GetRadialProgressBarPercent",
            get_radial_percent,
        ),
        (
            "TextureBase:GetRadialProgressBarReverse",
            get_radial_reverse,
        ),
        (
            "TextureBase:GetRadialProgressBarStartOffset",
            get_radial_start_offset,
        ),
        (
            "TextureBase:ClearRadialProgressBar",
            clear_radial_progress_bar,
        ),
        ("TextureBase:GetTexelSnappingBias", get_texel_snapping_bias),
        (
            "TextureBase:GetTextureSliceMargins",
            get_texture_slice_margins,
        ),
        ("TextureBase:GetTextureSliceMode", get_texture_slice_mode),
        ("TextureBase:ClearTextureSlice", clear_texture_slice),
        ("TextureBase:SetVertexOffset", set_vertex_offset),
        ("TextureBase:GetVertexOffset", get_vertex_offset),
        ("TextureBase:ClearVertexOffsets", clear_vertex_offsets),
        ("Line:GetThickness", get_thickness),
        ("Line:GetHitRectThickness", get_hit_rect_thickness),
        // Status bars, buttons, color selects.
        ("StatusBar:GetRenderMode", get_render_mode),
        (
            "StatusBar:GetStatusBarDesaturation",
            get_status_bar_desaturation,
        ),
        ("Button:GetPushedTextOffset", get_pushed_text_offset),
        ("ColorSelect:GetColorWheelTexture", get_color_wheel_texture),
        (
            "ColorSelect:GetColorWheelThumbTexture",
            get_color_wheel_thumb_texture,
        ),
        ("ColorSelect:GetColorValueTexture", get_color_value_texture),
        (
            "ColorSelect:GetColorValueThumbTexture",
            get_color_value_thumb_texture,
        ),
        ("ColorSelect:GetColorAlphaTexture", get_color_alpha_texture),
        (
            "ColorSelect:GetColorAlphaThumbTexture",
            get_color_alpha_thumb_texture,
        ),
        // Cooldowns.
        (
            "Cooldown:GetCooldownDisplayDuration",
            get_cooldown_display_duration,
        ),
        (
            "Cooldown:GetCountdownAbbrevThreshold",
            get_countdown_abbrev_threshold,
        ),
        (
            "Cooldown:GetCountdownMillisecondsThreshold",
            get_countdown_ms_threshold,
        ),
        ("Cooldown:GetCountdownFormatter", get_countdown_formatter),
        ("Cooldown:GetDrawSwipe", get_draw_swipe),
        ("Cooldown:GetDrawBling", get_draw_bling),
        ("Cooldown:GetDrawEdge", get_draw_edge),
        ("Cooldown:GetEdgeScale", get_edge_scale),
        (
            "Cooldown:GetMinimumCountdownDuration",
            get_minimum_countdown_duration,
        ),
        ("Cooldown:GetReverse", get_reverse),
        ("Cooldown:GetUseAuraDisplayTime", get_use_aura_display_time),
        // Edit boxes.
        ("EditBox:GetBlinkSpeed", get_blink_speed),
        ("EditBox:GetDisplayText", get_display_text),
        ("EditBox:GetHighlightColor", get_highlight_color),
        ("EditBox:GetMaxBytes", get_max_bytes),
        (
            "EditBox:GetVisibleTextByteLimit",
            get_visible_text_byte_limit,
        ),
        // Font strings.
        ("FontString:GetScaleAnimationMode", get_scale_animation_mode),
        ("FontString:GetSmoothScaling", get_smooth_scaling),
        ("FontString:GetFieldSize", get_field_size),
        (
            "FontString:GetUnboundedStringWidthForText",
            get_unbounded_width_for_text,
        ),
        (
            "FontString:FindCharacterIndexAtCoordinate",
            find_character_index,
        ),
        ("FontString:CalculateScreenAreaFromCharacterSpan", nil),
        ("FontString:SetAlphaGradient", set_alpha_gradient),
        ("FontString:GetAlphaGradient", get_alpha_gradient),
        ("FontString:ClearAlphaGradient", clear_alpha_gradient),
        // Message frames.
        ("MessageFrame:GetFadePower", get_fade_power),
        ("MessageFrame:HasMessageByID", has_message_by_id),
        // Simple HTML.
        ("SimpleHTML:SetTextColor", html_set_text_color),
        ("SimpleHTML:SetShadowColor", html_set_shadow_color),
        ("SimpleHTML:SetShadowOffset", html_set_shadow_offset),
        ("SimpleHTML:SetSpacing", html_set_spacing),
        ("SimpleHTML:SetJustifyH", html_set_justify_h),
        ("SimpleHTML:SetJustifyV", html_set_justify_v),
        (
            "SimpleHTML:SetIndentedWordWrap",
            html_set_indented_word_wrap,
        ),
        ("SimpleHTML:GetTextColor", html_get_text_color),
        ("SimpleHTML:GetShadowColor", html_get_shadow_color),
        ("SimpleHTML:GetShadowOffset", html_get_shadow_offset),
        ("SimpleHTML:GetSpacing", html_get_spacing),
        ("SimpleHTML:GetJustifyH", html_get_justify_h),
        ("SimpleHTML:GetJustifyV", html_get_justify_v),
        (
            "SimpleHTML:GetIndentedWordWrap",
            html_get_indented_word_wrap,
        ),
        ("SimpleHTML:GetFontObject", html_get_font_object),
        ("SimpleHTML:GetHyperlinkFormat", html_get_hyperlink_format),
        ("SimpleHTML:GetContentHeight", html_get_content_height),
        ("SimpleHTML:GetTextData", html_get_text_data),
        // Models.
        ("Model:GetCameraDistance", get_camera_distance),
        ("Model:GetCameraFacing", get_camera_facing),
        ("Model:GetCameraRoll", get_camera_roll),
        ("Model:GetFogColor", get_model_fog_color),
        ("Model:GetFogNear", get_fog_near),
        ("Model:GetFogFar", get_fog_far),
        ("Model:ClearFog", clear_fog),
        ("Model:GetLight", get_light),
        ("Model:GetModelAlpha", get_model_alpha),
        ("Model:GetModelDrawLayer", get_model_draw_layer),
        ("Model:GetPaused", get_model_paused),
        ("Model:GetShadowEffect", get_shadow_effect),
        ("Model:GetViewTranslation", get_view_translation),
        ("Model:GetWorldScale", get_world_scale),
        ("Model:HasAttachmentPoints", no),
        ("Model:HasCustomCamera", has_custom_camera),
        ("Model:MakeCurrentCameraCustom", make_current_camera_custom),
        (
            "Model:TransformCameraSpaceToModelSpace",
            transform_camera_to_model,
        ),
        ("CharacterModelBase:GetDisplayInfo", get_display_info),
        ("CharacterModelBase:GetDoBlend", get_do_blend),
        (
            "CharacterModelBase:GetKeepModelOnHide",
            get_keep_model_on_hide,
        ),
        // No model is loaded to have an animation (unverified).
        ("CharacterModelBase:HasAnimation", no),
        ("DressUpModel:GetAutoDress", get_auto_dress),
        (
            "DressUpModel:GetItemTransmogInfo",
            dress_up_item_transmog_info,
        ),
        ("DressUpModel:GetItemTransmogInfoList", transmog_info_list),
        (
            "DressUpModel:GetObeyHideInTransmogFlag",
            get_obey_hide_in_transmog,
        ),
        (
            "DressUpModel:GetUseTransmogChoices",
            get_use_transmog_choices,
        ),
        ("DressUpModel:GetUseTransmogSkin", get_use_transmog_skin),
        // A guild's tabard is not shown in the engine: no files, nothing
        // to save.
        ("TabardModel:GetLowerBackgroundFileName", zero),
        ("TabardModel:GetLowerBorderFile", zero),
        ("TabardModel:GetLowerEmblemFile", zero),
        ("TabardModel:GetUpperBackgroundFileName", zero),
        ("TabardModel:GetUpperBorderFile", zero),
        ("TabardModel:GetUpperEmblemFile", zero),
        ("TabardModelBase:CanSaveTabardNow", no),
        // Model scenes' actors.
        (
            "ModelSceneFrameActorBase:GetActiveBoundingBox",
            bounding_box,
        ),
        ("ModelSceneFrameActorBase:GetMaxBoundingBox", bounding_box),
        (
            "ModelSceneFrameActorBase:GetAnimationBlendOperation",
            get_animation_blend_operation,
        ),
        (
            "ModelSceneFrameActorBase:GetAnimationVariation",
            get_animation_variation,
        ),
        ("ModelSceneFrameActorBase:GetModelPath", get_model_path),
        (
            "ModelSceneFrameActorBase:GetModelUnitGUID",
            get_model_unit_guid,
        ),
        (
            "ModelSceneFrameActorBase:GetParticleOverrideScale",
            get_particle_override_scale,
        ),
        (
            "ModelSceneFrameActorBase:GetSpellVisualKit",
            get_spell_visual_kit,
        ),
        (
            "ModelSceneFrameActorBase:IsUsingCenterForOrigin",
            is_using_center_for_origin,
        ),
        ("ModelSceneFrameActor:AttachToMount", no),
        ("ModelSceneFrameActor:DetachFromMount", no),
        (
            "ModelSceneFrameActor:CalculateMountScale",
            calculate_mount_scale,
        ),
        ("ModelSceneFrameActor:GetAutoDress", get_auto_dress),
        ("ModelSceneFrameActor:GetItemTransmogInfo", nil),
        (
            "ModelSceneFrameActor:GetItemTransmogInfoList",
            transmog_info_list,
        ),
        (
            "ModelSceneFrameActor:GetObeyHideInTransmogFlag",
            get_obey_hide_in_transmog,
        ),
        ("ModelSceneFrameActor:GetPaused", get_actor_paused),
        (
            "ModelSceneFrameActor:GetUseTransmogChoices",
            get_use_transmog_choices,
        ),
        (
            "ModelSceneFrameActor:GetUseTransmogSkin",
            get_use_transmog_skin,
        ),
        // Nothing to release (unverified).
        ("ModelSceneFrameActor:ReleaseFrontEndCharacterDisplays", yes),
        // Map scenes.
        ("MapScene:GetMaxCharacterSlotCount", zero),
        ("MapScene:GetModelDrawLayer", get_map_scene_draw_layer),
        // Animations.
        ("Alpha:GetFromAlpha", get_from_alpha),
        ("Alpha:GetToAlpha", get_to_alpha),
        ("AnimRadialProgress:GetFromPercent", get_from_percent),
        ("AnimRadialProgress:GetToPercent", get_to_percent),
        ("Animation:GetRegionParent", get_region_parent),
        ("Animation:GetSmoothing", get_smoothing),
        ("AnimationGroup:GetLoopState", get_loop_state),
        ("FlipBook:GetFlipBookFrameWidth", get_flip_book_frame_width),
        (
            "FlipBook:GetFlipBookFrameHeight",
            get_flip_book_frame_height,
        ),
        ("Path:GetControlPoints", get_control_points),
        ("Path:GetCurveType", get_curve_type),
        ("Path:GetMaxControlPointOrder", get_max_control_point_order),
        ("Rotation:SetDegrees", set_degrees),
        ("Rotation:SetRadians", set_radians),
        ("Rotation:GetDegrees", get_degrees),
        ("Rotation:GetRadians", get_radians),
        ("Rotation:GetOrigin", get_origin),
        ("Scale:GetOrigin", get_origin),
        ("Scale:GetScaleFrom", get_scale_from),
        ("Scale:GetScaleTo", get_scale_to),
        ("Translation:GetOffset", get_offset),
        ("TextureCoordTranslation:GetOffset", get_offset),
        ("ControlPoint:GetOffset", get_offset),
        ("VertexColor:GetStartColor", get_start_color),
        ("VertexColor:GetEndColor", get_end_color),
        // The tail.
        ("Minimap:GetPingPosition", get_ping_position),
        ("Minimap:PingLocation", ping_location),
        // Unverified: whether the client lets addons change them (the
        // documented empty false).
        ("NamePlate:CanChangeHitTestPoints", no),
        ("NamePlate:GetHitTestPoints", get_hit_test_points),
        ("UnitPositionFrame:GetMouseOverUnits", get_mouse_over_units),
        (
            "UnitPositionFrame:GetPlayerPingScale",
            get_player_ping_scale,
        ),
        (
            "FogOfWarFrame:GetFogOfWarBackgroundAtlas",
            get_fog_background_atlas,
        ),
        (
            "FogOfWarFrame:GetFogOfWarBackgroundTexture",
            get_fog_background_texture,
        ),
        ("FogOfWarFrame:GetFogOfWarMaskAtlas", get_fog_mask_atlas),
        ("FogOfWarFrame:GetFogOfWarMaskTexture", get_fog_mask_texture),
        ("FogOfWarFrame:GetMaskScalar", get_mask_scalar),
        ("OffScreenFrame:GetMaxSnapshots", get_max_snapshots),
        ("OffScreenFrame:TakeSnapshot", nil),
        ("OffScreenFrame:ApplySnapshot", no),
        ("OffScreenFrame:TestPrintToFile", no),
        ("OffScreenFrame:UsesNPOT", nil),
        // Undocumented on these types (Ketho's annotations; `/stdtest
        // engine` lists them): a game tooltip's, a movie frame's, the
        // shop's checkout frame's, a map's quest and scenario blobs'.
        ("GameTooltip:GetCustomLineSpacing", get_custom_line_spacing),
        ("MovieFrame:StartMovie", start_movie),
        ("MovieFrame:StartMovieByName", start_movie),
        ("MovieFrame:StopMovie", nothing),
        ("MovieFrame:EnableSubtitles", nothing),
        ("Checkout:OpenCheckout", no),
        ("Checkout:CancelOpenCheckout", nothing),
        ("Checkout:CloseCheckout", nothing),
        ("Checkout:ClearFocus", nothing),
        ("Checkout:CopyExternalLink", nothing),
        ("Checkout:OpenExternalLink", nothing),
        ("QuestPOIFrame:GetNumTooltips", zero),
        ("QuestPOIFrame:GetTooltipIndex", zero),
        (
            "QuestPOIFrame:UpdateMouseOverTooltip",
            quest_poi_update_tooltip,
        ),
        ("ScenarioPOIFrame:GetScenarioTooltipText", nil),
        ("ScenarioPOIFrame:UpdateMouseOverTooltip", no),
    ];
    // A model scene's camera, fog and light: documented on
    // `ModelSceneFrame`, undocumented on the `ModelScene` type the client
    // makes (its own name then).
    for class in ["ModelScene", "ModelSceneFrame"] {
        let list: [(&str, MethodFn); 19] = [
            ("GetAllowOverlappedModels", get_allow_overlapped_models),
            ("GetCameraFarClip", get_camera_far_clip),
            ("GetCameraNearClip", get_camera_near_clip),
            ("GetCameraFieldOfView", get_camera_field_of_view),
            ("GetCameraForward", get_camera_forward),
            ("GetCameraRight", get_camera_right),
            ("GetCameraUp", get_camera_up),
            ("GetFogColor", get_scene_fog_color),
            ("GetFogNear", get_fog_near),
            ("GetFogFar", get_fog_far),
            ("ClearFog", clear_fog),
            ("GetLightAmbientColor", get_light_ambient_color),
            ("GetLightDiffuseColor", get_light_diffuse_color),
            ("GetLightDirection", get_light_direction),
            ("GetLightPosition", get_light_position),
            ("GetLightType", get_light_type),
            ("IsLightVisible", is_light_visible),
            ("GetViewTranslation", get_view_translation),
            ("Project3DPointTo2D", project_3d_point),
        ];
        for (name, f) in list {
            let full: &'static str = Box::leak(format!("{class}:{name}").into_boxed_str());
            out.push((full, f));
        }
    }
    out
}
