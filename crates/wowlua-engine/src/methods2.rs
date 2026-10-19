//! Widget methods that had no behaviour and no stored setter to read
//! (their getters fell through to "not mocked" for addons until a setter
//! ran): read from the engine's models where it has one (layout, alpha,
//! strata, text, font objects, animations, tooltips' lines), else what
//! the matching setter stored, else the documented empty value.
//!
//! The defaults below are unverified unless a recording is named: they
//! come from Blizzard's API documentation (`Blizzard_APIDocumentation
//! Generated`), how Blizzard's UI code uses the method, and the fresh
//! widgets' recorded getters (`camelot-widget-defaults.txt`, read through
//! [`widgets::recorded_default`]).

use std::rc::Rc;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, TableRef, Value, Vm};

use crate::widgets::{self, MethodFn, WidgetId};
use crate::engine;

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

/// The first stored value that is a widget, else nil.
fn widget_or_nil(vm: &mut Vm, id: WidgetId, stem: &str) -> Ret {
    let v = first(vm, id, stem).filter(|v| widgets::widget_of(vm, *v).is_some());
    Ok(vec![v.unwrap_or(Value::Nil)])
}

/// A fresh widget's results for `getter`, as the client gave them for its
/// class (`/stdtest engine`).
fn fresh(vm: &mut Vm, id: WidgetId, getter: &str) -> Option<Vec<Value>> {
    let class = engine(vm).world.widgets[id.0].class.clone();
    let rec = widgets::recorded_default(engine(vm), &class, getter)?;
    Some(rec.iter().map(|r| r.value(vm)).collect())
}

/// The widget's text, empty without one.
fn text_of(vm: &Vm, id: WidgetId) -> String {
    match engine(vm).world.widgets[id.0].text {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    }
}

/// The same object: the same table or userdata.
fn same(a: Value, b: Value) -> bool {
    match (a, b) {
        (Value::Table(x), Value::Table(y)) => x == y,
        (Value::Userdata(x), Value::Userdata(y)) => x == y,
        _ => false,
    }
}

/// A global of the main environment.
fn global(vm: &mut Vm, name: &str) -> Value {
    let g = engine(vm).main_globals;
    let k = vm.str(name);
    vm.heap.table(g).get(k)
}

/// `obj:method(args...)` through Lua (a mixin's override runs too).
fn call_method(vm: &mut Vm, obj: Value, method: &str, args: &[Value]) -> Ret {
    if !matches!(obj, Value::Table(_)) {
        return Ok(vec![]);
    }
    let key = vm.str(method);
    let f = vm.index(obj, key, None)?;
    if f.is_nil() {
        return Ok(vec![]);
    }
    let mut all = vec![obj];
    all.extend_from_slice(args);
    vm.call(f, &all)
}

/// A behaviour of another module, run as if Lua called it on `id`.
fn run_behaviour(vm: &mut Vm, id: WidgetId, full: &str, short: &str) -> Option<Vec<Value>> {
    let f = engine(vm).method_behaviour(full, short)?;
    let me = widgets::value_of(engine(vm), id);
    let call = NativeCall {
        args: [me].into_iter().collect(),
        taints: [Default::default()].into_iter().collect(),
        origins: [Default::default()].into_iter().collect(),
        upvals: Rc::from([]),
        upval_taints: Rc::from([]),
    };
    f(vm, id, call).ok()
}

// ---------------------------------------------------------------- fonts

/// What setter `stem` stored on the font string or along its font objects
/// (`SetFontObject`, a template's `inherits`): a font string shows its
/// font object's setting until it sets its own (as `GetFont` reads).
fn font_chain(vm: &mut Vm, id: WidgetId, stem: &str) -> Option<Vec<Value>> {
    let mut cur = id;
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

/// `GetTextColor()`: the color set (its own or its font object's), alpha 1
/// when set without one; a fresh one's recorded white (`/stdtest engine`:
/// 1, 1, 1, 1 for font strings, edit boxes and message frames).
fn get_text_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(mut v) = font_chain(vm, id, "TextColor")
        && !v.is_empty()
    {
        v.truncate(4);
        v.resize(4, Value::Num(1.0));
        return Ok(v);
    }
    if let Some(v) = fresh(vm, id, "GetTextColor") {
        return Ok(v);
    }
    Ok(vec![Value::Num(1.0); 4])
}

/// `GetShadowColor()`: the shadow's color set (own or a font object's),
/// alpha 1 when set without one; none at first (0, 0, 0, 0; unverified).
fn get_shadow_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(mut v) = font_chain(vm, id, "ShadowColor")
        && !v.is_empty()
    {
        v.truncate(4);
        v.resize(4, Value::Num(1.0));
        return Ok(v);
    }
    Ok(vec![Value::Num(0.0); 4])
}

/// `GetShadowOffset()`: the offset set (own or a font object's), else 0, 0
/// (unverified).
fn get_shadow_offset(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let mut v = font_chain(vm, id, "ShadowOffset").unwrap_or_default();
    v.truncate(2);
    v.resize(2, Value::Num(0.0));
    Ok(v)
}

/// `GetSpacing()`: the line spacing set (own or a font object's), else 0.
fn get_spacing(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let v = font_chain(vm, id, "Spacing")
        .and_then(|v| v.first().copied())
        .unwrap_or(Value::Num(0.0));
    Ok(vec![v])
}

/// `GetJustifyH()` / `GetJustifyV()`: the justification set (own or a font
/// object's), else the fresh widget's recorded one (a font string's
/// `CENTER`, an edit box's `LEFT`, `MIDDLE` vertically), else `CENTER` /
/// `MIDDLE` (a font object's; unverified).
fn justify(vm: &mut Vm, id: WidgetId, stem: &str, getter: &str, default: &str) -> Ret {
    if let Some(Value::Str(s)) = font_chain(vm, id, stem).and_then(|v| v.first().copied()) {
        let up = String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase();
        return Ok(vec![vm.str(&up)]);
    }
    if let Some(v) = fresh(vm, id, getter) {
        return Ok(v);
    }
    Ok(vec![vm.str(default)])
}

fn get_justify_h(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    justify(vm, id, "JustifyH", "GetJustifyH", "CENTER")
}

fn get_justify_v(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    justify(vm, id, "JustifyV", "GetJustifyV", "MIDDLE")
}

/// `GetIndentedWordWrap()`: as set (own or a font object's), else false.
fn get_indented_word_wrap(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let on = font_chain(vm, id, "IndentedWordWrap")
        .and_then(|v| v.first().copied())
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(on)])
}

/// `GetFontObject()`: the font object set, else nil.
fn get_font_object(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widget_or_nil(vm, id, "FontObject")
}

/// `GetFontHeight()`: the font's height (own or a font object's), 0
/// without a font.
fn get_font_height(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let h = font_chain(vm, id, "Font")
        .and_then(|v| v.get(1).copied())
        .and_then(|v| number(vm, v))
        .unwrap_or(0.0);
    Ok(vec![Value::Num(h)])
}

/// `Font:GetFontObjectForAlphabet(alphabet)`: the object itself (the
/// engine keeps one font a family for every alphabet, as
/// `CreateFontFamily` makes it; unverified: a family's other members).
fn get_font_object_for_alphabet(_vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    Ok(vec![c.arg(1)])
}

/// `FontString:GetMaxLines()`: what `SetMaxLines` stored, 0 (no limit) at
/// first.
fn get_max_lines(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "MaxLines", 0.0)
}

/// `CanWordWrap()`: on until `SetWordWrap(false)` (as `measures.rs`
/// wraps).
fn can_word_wrap(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let on = font_chain(vm, id, "WordWrap")
        .and_then(|v| v.first().copied())
        .is_none_or(|v| v.truthy());
    Ok(vec![Value::Bool(on)])
}

/// `CanNonSpaceWrap()`: off until set.
fn can_non_space_wrap(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    flag_or(vm, id, "NonSpaceWrap", false)
}

/// The text's width in the engine's text model (as `GetStringWidth`: six
/// units a byte).
fn string_width(vm: &Vm, id: WidgetId) -> f64 {
    text_of(vm, id).len() as f64 * 6.0
}

/// `GetWrappedWidth()`: the text's width, at most the laid out width when
/// it wraps there.
fn get_wrapped_width(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let w = string_width(vm, id);
    let wraps = can_word_wrap(vm, id, c)?
        .first()
        .is_some_and(|v| v.truthy());
    let width = widgets::laid_out_size(vm, id).0;
    Ok(vec![Value::Num(if wraps && width > 0.0 {
        w.min(width)
    } else {
        w
    })])
}

/// `FontString:GetRotation()` / `Cooldown:GetRotation()`: the angle set,
/// 0 at first.
fn get_plain_rotation(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "Rotation", 0.0)
}

/// `Button:GetTextHeight()`: its font string's height (`GetStringHeight`
/// on the text model), 0 without one.
fn get_text_height(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let fs = first(vm, id, "FontString").and_then(|v| widgets::widget_of(vm, v));
    let h = fs
        .and_then(|fs| run_behaviour(vm, fs, "FontString:GetStringHeight", "GetStringHeight"))
        .and_then(|v| v.first().copied())
        .and_then(|v| number(vm, v))
        .unwrap_or(0.0);
    Ok(vec![Value::Num(h)])
}

/// `Button:GetNormalFontObject()` and the others: the font object set,
/// else nil.
fn get_normal_font_object(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widget_or_nil(vm, id, "NormalFontObject")
}

fn get_highlight_font_object(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widget_or_nil(vm, id, "HighlightFontObject")
}

fn get_disabled_font_object(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widget_or_nil(vm, id, "DisabledFontObject")
}

fn get_motion_scripts_while_disabled(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    flag_or(vm, id, "MotionScriptsWhileDisabled", false)
}

/// `CheckButton:GetChecked()`: whether checked (`SetChecked`, a click),
/// as a boolean; a fresh one's recorded false (`/stdtest engine`).
fn get_checked(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    flag_or(vm, id, "Checked", false)
}

// ------------------------------------------------------------ edit boxes

/// `EditBox:GetNumLetters()`: the text's characters.
fn get_num_letters(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(text_of(vm, id).chars().count() as f64)])
}

/// `EditBox:HasText()`.
fn has_text(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(!text_of(vm, id).is_empty())])
}

/// The cursor's byte offset: where `SetCursorPosition` put it (within the
/// text), else at the text's end (unverified: the client puts it there
/// as text is set or typed).
fn cursor(vm: &mut Vm, id: WidgetId) -> usize {
    let len = text_of(vm, id).len();
    match first_num(vm, id, "CursorPosition") {
        Some(n) if n >= 0.0 => (n as usize).min(len),
        Some(_) => 0,
        None => len,
    }
}

fn get_cursor_position(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(cursor(vm, id) as f64)])
}

/// `GetUTF8CursorPosition()`: the cursor in characters.
fn get_utf8_cursor_position(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let at = cursor(vm, id);
    let text = text_of(vm, id);
    let n = text
        .char_indices()
        .take_while(|(i, _)| *i < at)
        .count();
    Ok(vec![Value::Num(n as f64)])
}

/// `GetMaxLetters()`: the limit set (XML `letters`), 0 (none) at first.
fn get_max_letters(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "MaxLetters", 0.0)
}

fn get_alt_arrow_key_mode(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    flag_or(vm, id, "AltArrowKeyMode", false)
}

/// `GetInputLanguage()`: `ROMAN` (an English client's).
fn get_input_language(vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("ROMAN")])
}

// ---------------------------------------------------------------- frames

/// The stratas, lowest first (as `widgets::strata_rank` ranks them).
const STRATAS: [&str; 9] = [
    "BACKGROUND",
    "LOW",
    "MEDIUM",
    "HIGH",
    "DIALOG",
    "FULLSCREEN",
    "FULLSCREEN_DIALOG",
    "TOOLTIP",
    "BLIZZARD",
];

/// `GetFrameStrata()`: its own, else its parent's (the strata the engine
/// orders frames by), `MEDIUM` at the top (a fresh frame's, recorded).
fn get_frame_strata(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let rank = widgets::strata_rank(vm, id);
    Ok(vec![vm.str(STRATAS.get(rank).copied().unwrap_or("MEDIUM"))])
}

/// `GetEffectiveAlpha()`: its alpha times its parents', up to one that
/// ignores its parent's alpha (a fresh frame's 1, recorded).
fn get_effective_alpha(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let mut a = 1.0;
    let mut secret = false;
    let mut cur = Some(id);
    for _ in 0..256 {
        let Some(w) = cur else { break };
        let (alpha, s, parent) = {
            let w = &engine(vm).world.widgets[w.0];
            (w.alpha, w.secret_alpha, w.parent)
        };
        a *= alpha;
        secret |= s;
        if first(vm, w, "IgnoreParentAlpha").is_some_and(|v| v.truthy()) {
            break;
        }
        cur = parent;
    }
    let v = Value::Num(a);
    Ok(vec![if secret { vm.secret_wrap(v) } else { v }])
}

/// `GetWindow()`: the window set (`SetWindow`), else nil (the main
/// window; Blizzard's menus take nil for it).
fn get_window(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![first(vm, id, "Window").unwrap_or(Value::Nil)])
}

/// A rectangle as `widgets::bounds` gives it: left, bottom, width, height.
type Rect = (f64, f64, f64, f64);

fn union(a: Option<Rect>, b: Rect) -> Rect {
    match a {
        None => b,
        Some(a) => {
            let left = a.0.min(b.0);
            let bottom = a.1.min(b.1);
            let right = (a.0 + a.2).max(b.0 + b.2);
            let top = (a.1 + a.3).max(b.1 + b.3);
            (left, bottom, right - left, top - bottom)
        }
    }
}

/// A placed widget's rectangle on the screen.
fn screen_rect(vm: &Vm, id: WidgetId) -> Option<Rect> {
    if engine(vm).world.widgets[id.0].points.is_empty() {
        return None;
    }
    widgets::bounds(vm, id, 0)
}

/// `GetBoundsRect()`: the rectangle around it and its shown descendants,
/// in its own scale (unverified: which descendants count).
fn get_bounds_rect(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let mut rect = None;
    let mut todo = vec![id];
    let mut seen = 0;
    while let Some(w) = todo.pop() {
        seen += 1;
        if seen > 10_000 {
            break;
        }
        if let Some(r) = screen_rect(vm, w) {
            rect = Some(union(rect, r));
        }
        let e = engine(vm);
        todo.extend(
            e.world.widgets[w.0]
                .children
                .iter()
                .copied()
                .filter(|c| e.world.widgets[c.0].shown),
        );
    }
    let s = widgets::effective_scale(engine(vm), id);
    Ok(match rect {
        Some(r) => vec![
            Value::Num(r.0 / s),
            Value::Num(r.1 / s),
            Value::Num(r.2 / s),
            Value::Num(r.3 / s),
        ],
        None => vec![],
    })
}

/// `GetScaledRect()`: its rectangle in screen units (its scale applied);
/// nothing while it is not placed.
fn get_scaled_rect(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(match screen_rect(vm, id) {
        Some(r) => vec![
            Value::Num(r.0),
            Value::Num(r.1),
            Value::Num(r.2),
            Value::Num(r.3),
        ],
        None => vec![],
    })
}

/// `Intersects(region)`: whether the two placed rectangles overlap.
fn intersects(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let other = widgets::widget_of(vm, c.arg(2));
    let hit = match (screen_rect(vm, id), other.and_then(|o| screen_rect(vm, o))) {
        (Some(a), Some(b)) => {
            a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
        }
        _ => false,
    };
    Ok(vec![Value::Bool(hit)])
}

/// `ClearAttribute(name)`: the attribute gone; whether there was one.
/// Unverified: whether `OnAttributeChanged` runs (not run here).
fn clear_attribute(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Value::Str(s) = c.arg(2) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let lower = String::from_utf8_lossy(vm.str_bytes(s)).to_lowercase();
    let key = vm.str(&lower);
    let attributes = engine(vm).world.widgets[id.0].attributes;
    let had = !vm.heap.table(attributes).get(key).is_nil();
    if had {
        let _ = vm.heap.table_mut(attributes).set(key, Value::Nil);
    }
    Ok(vec![Value::Bool(had)])
}

/// `GetParentKey()`: the key `SetParentKey` gave it, else the key its
/// parent holds it under (XML `parentKey`), else nil.
fn get_parent_key(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(v) = first(vm, id, "ParentKey")
        && matches!(v, Value::Str(_))
    {
        return Ok(vec![v]);
    }
    let (parent, me) = {
        let e = engine(vm);
        (e.world.widgets[id.0].parent, widgets::value_of(e, id))
    };
    let Some(p) = parent else {
        return Ok(vec![Value::Nil]);
    };
    let pt = engine(vm).world.widgets[p.0].table;
    let mut keys: Vec<String> = vm
        .heap
        .table(pt)
        .entries()
        .filter(|(k, v)| matches!(k, Value::Str(_)) && same(*v, me))
        .filter_map(|(k, _)| match k {
            Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
            _ => None,
        })
        .collect();
    // One key, the same each time.
    keys.sort();
    Ok(vec![match keys.first() {
        Some(k) => vm.str(k),
        None => Value::Nil,
    }])
}

/// `CanBeAccessedInContext()`: addon code may use what it can see.
fn yes(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `HasAccessConstraints()`, `HasAnyForbiddenAspects()`: none on the
/// engine's widgets.
fn no(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `GetInheritableForbiddenAspects()`: none.
fn zero(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `ScrollFrame:GetScrollChild()`: the child set, else nil.
fn get_scroll_child(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widget_or_nil(vm, id, "ScrollChild")
}

/// `StatusBar:GetTimerDuration()`: the duration set, else nil.
fn get_timer_duration(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![first(vm, id, "TimerDuration").unwrap_or(Value::Nil)])
}

/// `Line:GetStartPoint()` / `GetEndPoint()`: as set, else the center of
/// nothing at no offset (unverified).
fn line_point(vm: &mut Vm, id: WidgetId, stem: &str) -> Ret {
    if let Some(mut v) = stored(vm, id, stem)
        && !v.is_empty()
    {
        v.truncate(4);
        while v.len() < 4 {
            let fill = if v.len() == 1 {
                Value::Nil
            } else {
                Value::Num(0.0)
            };
            v.push(fill);
        }
        return Ok(v);
    }
    Ok(vec![
        vm.str("CENTER"),
        Value::Nil,
        Value::Num(0.0),
        Value::Num(0.0),
    ])
}

fn get_start_point(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    line_point(vm, id, "StartPoint")
}

fn get_end_point(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    line_point(vm, id, "EndPoint")
}

/// `GetMapID()` / `GetUiMapID()`: the map set, 0 at first.
fn get_map_id(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "MapID", 0.0)
}

fn get_ui_map_id(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "UiMapID", 0.0)
}

// -------------------------------------------------------------- textures

/// `GetTexCoord()`: the eight corners (upper left, lower left, upper
/// right, lower right; x and y each) of what `SetTexCoord` set (its
/// four-value form is left, right, top, bottom), the whole texture at
/// first.
fn get_tex_coord(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let set = stored(vm, id, "TexCoord").unwrap_or_default();
    if set.len() >= 8 {
        return Ok(set[..8].to_vec());
    }
    if set.len() >= 4 {
        let (l, r, t, b) = (set[0], set[1], set[2], set[3]);
        return Ok(vec![l, t, l, b, r, t, r, b]);
    }
    Ok([0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0]
        .into_iter()
        .map(Value::Num)
        .collect())
}

/// A `vector2` as Blizzard's code makes it (`CreateVector2D` when its UI
/// code is loaded), else a table with `x` and `y`.
fn vector2(vm: &mut Vm, x: f64, y: f64) -> Value {
    let create = global(vm, "CreateVector2D");
    if matches!(create, Value::Func(_))
        && let Ok(v) = vm.call(create, &[Value::Num(x), Value::Num(y)])
        && let Some(&v) = v.first()
    {
        return v;
    }
    let t = vm.new_table();
    for (k, v) in [("x", x), ("y", y)] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, Value::Num(v));
    }
    Value::Table(t)
}

/// `TextureBase:GetRotation()`: the angle and the point it turns around
/// as set, 0 about the center at first.
fn get_texture_rotation(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let set = stored(vm, id, "Rotation").unwrap_or_default();
    let radians = set.first().copied().unwrap_or(Value::Num(0.0));
    let point = match set.get(1).copied() {
        Some(v @ Value::Table(_)) => v,
        _ => vector2(vm, 0.5, 0.5),
    };
    Ok(vec![radians, point])
}

/// `GetDesaturation()`: the strength set (`SetDesaturation`), else 1 when
/// `SetDesaturated(true)`, else 0.
fn get_desaturation(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(n) = first_num(vm, id, "Desaturation") {
        return Ok(vec![Value::Num(n)]);
    }
    let on = first(vm, id, "Desaturated").is_some_and(|v| v.truthy());
    Ok(vec![Value::Num(if on { 1.0 } else { 0.0 })])
}

/// `GetTextureFilePath()`: the file path set, else nil.
fn get_texture_file_path(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let v = first(vm, id, "Texture").filter(|v| matches!(v, Value::Str(_)));
    Ok(vec![v.unwrap_or(Value::Nil)])
}

// ------------------------------------------------------------- cooldowns

/// The cooldown set: start and duration in seconds, the rate; secret when
/// set from secrets. `Clear` leaves none.
fn cooldown(vm: &mut Vm, id: WidgetId) -> (f64, f64, f64, bool) {
    let set = stored(vm, id, "Cooldown").unwrap_or_default();
    let secret = set.iter().any(|v| matches!(v, Value::Secret(_)));
    let vm: &Vm = vm;
    let n = |i: usize, d: f64| set.get(i).and_then(|v| number(vm, *v)).unwrap_or(d);
    (n(0, 0.0), n(1, 0.0), n(2, 1.0), secret)
}

fn maybe_secret(vm: &mut Vm, v: Value, secret: bool) -> Value {
    if secret { vm.secret_wrap(v) } else { v }
}

/// `Cooldown:GetCooldownTimes()`: start and duration as set, in
/// milliseconds (the client's unit), 0 and 0 without one. Unverified:
/// whether a finished cooldown still reports them (kept here).
fn get_cooldown_times(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let (start, duration, _, secret) = cooldown(vm, id);
    let a = maybe_secret(vm, Value::Num(start * 1000.0), secret);
    let b = maybe_secret(vm, Value::Num(duration * 1000.0), secret);
    Ok(vec![a, b])
}

/// `Cooldown:GetCooldownDuration()`: in milliseconds "multiplied by the
/// modRate" (the documentation).
fn get_cooldown_duration(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let (_, duration, rate, secret) = cooldown(vm, id);
    let v = Value::Num(duration * 1000.0 * rate);
    Ok(vec![maybe_secret(vm, v, secret)])
}

/// `Cooldown:SetCooldownDuration(duration [, modRate])`: a cooldown
/// starting now.
fn set_cooldown_duration(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let now = Value::Num(crate::clock::now(vm));
    let duration = c.arg(2);
    let rate = if c.arg(3).is_nil() {
        Value::Num(1.0)
    } else {
        c.arg(3)
    };
    widgets::store(vm, id, "Cooldown", &[now, duration, rate]);
    Ok(vec![])
}

/// `Cooldown:Clear()`: no cooldown.
fn clear_cooldown(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widgets::store(
        vm,
        id,
        "Cooldown",
        &[Value::Num(0.0), Value::Num(0.0), Value::Num(1.0)],
    );
    Ok(vec![])
}

/// `Cooldown:GetCountdownFontString()`: the cooldown's countdown text, a
/// font string of its own (made when first asked; then among its
/// regions, as the client's is).
fn get_countdown_font_string(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(v) = first(vm, id, "\u{0}CountdownFontString")
        && widgets::widget_of(vm, v).is_some()
    {
        return Ok(vec![v]);
    }
    let fs = widgets::create(vm, "FontString", None, Some(id));
    widgets::store(vm, id, "\u{0}CountdownFontString", &[fs]);
    Ok(vec![fs])
}

/// `GetHideCountdownNumbers()`: as set, shown (false) at first
/// (unverified: the client's default follows a CVar).
fn get_hide_countdown_numbers(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    flag_or(vm, id, "HideCountdownNumbers", false)
}

// ------------------------------------------------------------ animations

/// The animations of a group, in creation order.
fn animations_of(vm: &Vm, group: WidgetId) -> Vec<WidgetId> {
    let e = engine(vm);
    e.world.widgets[group.0]
        .children
        .iter()
        .copied()
        .filter(|c| widgets::is_a(e, &e.world.widgets[c.0].class, "Animation"))
        .collect()
}

fn stored_or_zero(vm: &mut Vm, id: WidgetId, stem: &str) -> f64 {
    first_num(vm, id, stem).unwrap_or(0.0)
}

/// An animation's order: as set (XML `order`), else 1 (unverified: the
/// client's default; `animations.rs` times unset orders together).
fn order_of(vm: &mut Vm, id: WidgetId) -> f64 {
    first_num(vm, id, "Order").unwrap_or(1.0)
}

/// Per order, the longest animation's time (start delay, duration, end
/// delay), lowest order first: as `animations.rs` plays a group.
fn order_lengths(vm: &mut Vm, group: WidgetId) -> std::collections::BTreeMap<i64, f64> {
    let mut by_order: std::collections::BTreeMap<i64, f64> = Default::default();
    for a in animations_of(vm, group) {
        let order = stored_or_zero(vm, a, "Order") as i64;
        let length = stored_or_zero(vm, a, "StartDelay")
            + stored_or_zero(vm, a, "Duration")
            + stored_or_zero(vm, a, "EndDelay");
        let slot = by_order.entry(order).or_insert(0.0);
        *slot = slot.max(length);
    }
    by_order
}

/// A group's time played: 0 unless it is playing (or paused).
fn group_elapsed(vm: &Vm, group: WidgetId) -> f64 {
    engine(vm)
        .animations
        .playing
        .get(&group.0)
        .map_or(0.0, |p| p.elapsed)
}

/// An animation's time played within its duration.
fn animation_elapsed(vm: &mut Vm, id: WidgetId) -> f64 {
    let Some(group) = engine(vm).world.widgets[id.0].parent else {
        return 0.0;
    };
    let order = stored_or_zero(vm, id, "Order") as i64;
    let offset: f64 = order_lengths(vm, group)
        .iter()
        .filter(|(o, _)| **o < order)
        .map(|(_, l)| *l)
        .sum();
    let t = group_elapsed(vm, group) - offset - stored_or_zero(vm, id, "StartDelay");
    let duration = stored_or_zero(vm, id, "Duration");
    t.clamp(0.0, duration.max(0.0))
}

fn animation_progress(vm: &mut Vm, id: WidgetId) -> f64 {
    let duration = stored_or_zero(vm, id, "Duration");
    if duration <= 0.0 {
        return 0.0;
    }
    (animation_elapsed(vm, id) / duration).clamp(0.0, 1.0)
}

/// `Animation:GetElapsed()`: its time played (0 unless its group plays).
fn get_animation_elapsed(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(animation_elapsed(vm, id))])
}

/// `Animation:GetProgress()`: its time played over its duration.
fn get_animation_progress(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(animation_progress(vm, id))])
}

/// `Animation:GetSmoothProgress()`: the progress through its smoothing
/// (`IN` squares it, `OUT` the reverse, `IN_OUT` both halves; unverified
/// curves).
fn get_smooth_progress(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let p = animation_progress(vm, id);
    let smoothing = match first(vm, id, "Smoothing") {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase(),
        _ => String::new(),
    };
    let s = match smoothing.as_str() {
        "IN" => p * p,
        "OUT" => 1.0 - (1.0 - p) * (1.0 - p),
        "IN_OUT" if p < 0.5 => 2.0 * p * p,
        "IN_OUT" => 1.0 - 2.0 * (1.0 - p) * (1.0 - p),
        _ => p,
    };
    Ok(vec![Value::Num(s)])
}

/// `Animation:GetTarget()`: the region it animates: the one set
/// (`SetTarget`, `SetTargetKey`/`SetChildKey` on its group's region,
/// `SetTargetName`), else its group's region.
fn get_target(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(v) = first(vm, id, "Target") {
        if widgets::widget_of(vm, v).is_some() {
            return Ok(vec![v]);
        }
        if let Value::Str(s) = v {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            return Ok(vec![global(vm, &name)]);
        }
    }
    if let Some(Value::Str(s)) = first(vm, id, "TargetName") {
        let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
        return Ok(vec![global(vm, &name)]);
    }
    let owner = engine(vm).world.widgets[id.0]
        .parent
        .and_then(|g| engine(vm).world.widgets[g.0].parent);
    let Some(owner) = owner else {
        return Ok(vec![Value::Nil]);
    };
    for stem in ["ChildKey", "TargetKey"] {
        if let Some(key @ Value::Str(_)) = first(vm, id, stem) {
            let t = engine(vm).world.widgets[owner.0].table;
            let v = vm.heap.table(t).get(key);
            if !v.is_nil() {
                return Ok(vec![v]);
            }
        }
    }
    Ok(vec![widgets::value_of(engine(vm), owner)])
}

/// `Animation:GetOrder()`.
fn get_animation_order(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(order_of(vm, id))])
}

/// `ControlPoint:GetOrder()`: as set (`CreateControlPoint`'s order), else
/// 0 (unverified).
fn get_control_point_order(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "Order", 0.0)
}

fn get_start_delay(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "StartDelay", 0.0)
}

fn get_end_delay(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "EndDelay", 0.0)
}

/// `AnimationGroup:GetElapsed()`: its time played this loop.
fn get_group_elapsed(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(group_elapsed(vm, id))])
}

/// `AnimationGroup:GetProgress()`: its time played over its length.
fn get_group_progress(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let length: f64 = order_lengths(vm, id).values().sum();
    let p = if length > 0.0 {
        (group_elapsed(vm, id) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Ok(vec![Value::Num(p)])
}

/// `AnimationGroup:GetLooping()`: as set, `NONE` at first.
fn get_looping(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    match first(vm, id, "Looping") {
        Some(Value::Str(s)) => {
            let up = String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase();
            Ok(vec![vm.str(&up)])
        }
        _ => Ok(vec![vm.str("NONE")]),
    }
}

fn get_animation_speed_multiplier(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "AnimationSpeedMultiplier", 1.0)
}

/// `Path:CreateControlPoint([name, template, order])`: a control point of
/// the path.
fn create_control_point(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let name = vm
        .opt_str(&c, 2)?
        .map(|s| String::from_utf8_lossy(vm.str_bytes(s)).into_owned());
    let point = widgets::create(vm, "ControlPoint", name, Some(id));
    if let Some(t) = vm.opt_str(&c, 3)? {
        let t = String::from_utf8_lossy(vm.str_bytes(t)).into_owned();
        widgets::apply_template(vm, point, &t)?;
    }
    if let Value::Num(n) = c.arg(4)
        && let Some(pid) = widgets::widget_of(vm, point)
    {
        widgets::store(vm, pid, "Order", &[Value::Num(n)]);
    }
    Ok(vec![point])
}

fn get_flip_book_columns(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "FlipBookColumns", 0.0)
}

fn get_flip_book_rows(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "FlipBookRows", 0.0)
}

fn get_flip_book_frames(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "FlipBookFrames", 0.0)
}

// -------------------------------------------------------------- tooltips

/// Line `n`'s font string on `side` of a named tooltip, if it has one.
fn tooltip_line(vm: &mut Vm, id: WidgetId, side: &str, n: usize) -> Value {
    let Some(name) = engine(vm).world.widgets[id.0].name.clone() else {
        return Value::Nil;
    };
    global(vm, &format!("{name}Text{side}{n}"))
}

fn line_index(vm: &Vm, c: &NativeCall) -> Option<usize> {
    match number(vm, c.arg(2)) {
        Some(n) if n >= 1.0 => Some(n as usize),
        _ => None,
    }
}

/// `GetLeftLine(n)` / `GetRightLine(n)`: line `n`'s font string (as
/// `tooltip.rs` fills them), nil past the tooltip's.
fn get_left_line(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    Ok(vec![match line_index(vm, &c) {
        Some(n) => tooltip_line(vm, id, "Left", n),
        None => Value::Nil,
    }])
}

fn get_right_line(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    Ok(vec![match line_index(vm, &c) {
        Some(n) => tooltip_line(vm, id, "Right", n),
        None => Value::Nil,
    }])
}

/// `GetPadding()`: right, bottom, left, top as set, 0 at first.
fn get_padding(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "Padding", 4, 0.0)
}

/// `ClearPadding()`: none.
fn clear_padding(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widgets::store(vm, id, "Padding", &[Value::Num(0.0); 4]);
    Ok(vec![])
}

/// `GetMinimumWidth()`: the width set and whether forced, 0 and false at
/// first.
fn get_minimum_width(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let set = stored(vm, id, "MinimumWidth").unwrap_or_default();
    let w = set.first().and_then(|v| number(vm, *v)).unwrap_or(0.0);
    let forced = set.get(1).is_some_and(|v| v.truthy());
    Ok(vec![Value::Num(w), Value::Bool(forced)])
}

/// `GameTooltip:GetAnchorType()`: the anchor set (`SetAnchorType`, or
/// `SetOwner`'s, `ANCHOR_LEFT` when it gave none), `ANCHOR_NONE` without
/// an owner (unverified).
fn get_anchor_type(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if let Some(v @ Value::Str(_)) = first(vm, id, "AnchorType") {
        return Ok(vec![v]);
    }
    match stored(vm, id, "Owner") {
        Some(owner) if owner.first().is_some_and(|v| !v.is_nil()) => {
            match owner.get(1).copied() {
                Some(v @ Value::Str(_)) => Ok(vec![v]),
                _ => Ok(vec![vm.str("ANCHOR_LEFT")]),
            }
        }
        _ => Ok(vec![vm.str("ANCHOR_NONE")]),
    }
}

/// `GameTooltip:AppendText(text)`: the text added to the end of the first
/// line.
fn append_text(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let fs = tooltip_line(vm, id, "Left", 1);
    let Some(fid) = widgets::widget_of(vm, fs) else {
        return Ok(vec![]);
    };
    let mut text = text_of(vm, fid);
    match c.arg(2) {
        Value::Str(s) => text.push_str(&String::from_utf8_lossy(vm.str_bytes(s))),
        Value::Num(n) => {
            let v = vm.tostring(Value::Num(n))?;
            if let Value::Str(s) = v {
                text.push_str(&String::from_utf8_lossy(vm.str_bytes(s)));
            }
        }
        _ => {}
    }
    let v = vm.str(&text);
    call_method(vm, fs, "SetText", &[v])?;
    Ok(vec![])
}

/// `GameTooltip:FadeOut()`: hidden (unverified: the client fades it out
/// over a moment first).
fn fade_out(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widgets::set_shown(vm, id, false);
    Ok(vec![])
}

/// `AddTexture`, `AddAtlas`: a picture on the last line (nothing drawn).
fn accept(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![])
}

// ---------------------------------------------------------------- models

fn get_position(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "Position", 3, 0.0)
}

fn get_camera_position(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "CameraPosition", 3, 0.0)
}

fn get_camera_target(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "CameraTarget", 3, 0.0)
}

fn get_view_insets(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    numbers_or(vm, id, "ViewInsets", 4, 0.0)
}

fn get_facing(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "Facing", 0.0)
}

fn get_yaw(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "Yaw", 0.0)
}

fn get_pitch(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "Pitch", 0.0)
}

fn get_roll(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "Roll", 0.0)
}

fn get_model_scale(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "ModelScale", 1.0)
}

/// `GetModelFileID()`: the file ID set (`SetModelByFileID`, or `SetModel`
/// given one), else 0.
fn get_model_file_id(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    for stem in ["ModelByFileID", "Model"] {
        if let Some(Value::Num(n)) = first(vm, id, stem) {
            return Ok(vec![Value::Num(n)]);
        }
    }
    Ok(vec![Value::Num(0.0)])
}

fn get_sheathed(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    flag_or(vm, id, "Sheathed", false)
}

/// An actor's `GetAnimation()`: the animation set, 0 (standing) at first.
fn get_actor_animation(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "Animation", 0.0)
}

/// `TryOn(item)`: tried on (`Enum.ItemTryOnReason.Success`; the engine
/// draws no model).
fn try_on(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

// --------------------------------------------------------- message frames

/// `MessageFrame:GetTimeVisible()`: as set, else 10 seconds (unverified:
/// the long-known default).
fn get_time_visible(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "TimeVisible", 10.0)
}

/// `GetFadeDuration()`: as set, else 3 seconds (unverified).
fn get_fade_duration(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    num_or(vm, id, "FadeDuration", 3.0)
}

/// `GetFading()`: as set, else on (unverified).
fn get_fading(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    flag_or(vm, id, "Fading", true)
}

/// `GetInsertMode()`: as set, else `BOTTOM` (unverified).
fn get_insert_mode(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    match first(vm, id, "InsertMode") {
        Some(Value::Str(s)) => {
            let up = String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase();
            Ok(vec![vm.str(&up)])
        }
        _ => Ok(vec![vm.str("BOTTOM")]),
    }
}

// ------------------------------------------------------------------ list

pub fn methods() -> Vec<(&'static str, MethodFn)> {
    let mut out: Vec<(&'static str, MethodFn)> = Vec::new();
    // Fonts: a font string's, an edit box's, a message frame's and a font
    // object's (not a SimpleHTML's, which takes a text type first).
    for class in ["FontString", "Font", "EditBox", "MessageFrame"] {
        let list: [(&str, MethodFn); 8] = [
            ("GetTextColor", get_text_color),
            ("GetShadowColor", get_shadow_color),
            ("GetShadowOffset", get_shadow_offset),
            ("GetSpacing", get_spacing),
            ("GetJustifyH", get_justify_h),
            ("GetJustifyV", get_justify_v),
            ("GetIndentedWordWrap", get_indented_word_wrap),
            ("GetFontObject", get_font_object),
        ];
        for (name, f) in list {
            let full: &'static str = Box::leak(format!("{class}:{name}").into_boxed_str());
            out.push((full, f));
        }
    }
    out.extend([
        ("Font:GetFontHeight", get_font_height as MethodFn),
        ("FontString:GetFontHeight", get_font_height),
        ("Font:GetFontObjectForAlphabet", get_font_object_for_alphabet),
        ("FontString:GetMaxLines", get_max_lines),
        ("FontString:CanWordWrap", can_word_wrap),
        ("FontString:CanNonSpaceWrap", can_non_space_wrap),
        ("FontString:GetWrappedWidth", get_wrapped_width),
        ("FontString:GetRotation", get_plain_rotation),
        ("Button:GetTextHeight", get_text_height),
        ("Button:GetNormalFontObject", get_normal_font_object),
        ("Button:GetHighlightFontObject", get_highlight_font_object),
        ("Button:GetDisabledFontObject", get_disabled_font_object),
        (
            "Button:GetMotionScriptsWhileDisabled",
            get_motion_scripts_while_disabled,
        ),
        ("GetChecked", get_checked),
        // Edit boxes.
        ("EditBox:GetNumLetters", get_num_letters),
        ("EditBox:HasText", has_text),
        ("EditBox:GetCursorPosition", get_cursor_position),
        ("EditBox:GetUTF8CursorPosition", get_utf8_cursor_position),
        ("EditBox:GetMaxLetters", get_max_letters),
        ("EditBox:GetAltArrowKeyMode", get_alt_arrow_key_mode),
        ("EditBox:GetInputLanguage", get_input_language),
        // Frames and regions.
        ("GetFrameStrata", get_frame_strata),
        ("GetEffectiveAlpha", get_effective_alpha),
        ("GetWindow", get_window),
        ("GetBoundsRect", get_bounds_rect),
        ("GetScaledRect", get_scaled_rect),
        ("Intersects", intersects),
        ("ClearAttribute", clear_attribute),
        ("GetParentKey", get_parent_key),
        ("CanBeAccessedInContext", yes),
        ("HasAccessConstraints", no),
        ("HasAnyForbiddenAspects", no),
        ("GetInheritableForbiddenAspects", zero),
        ("ScrollFrame:GetScrollChild", get_scroll_child),
        ("StatusBar:GetTimerDuration", get_timer_duration),
        ("Line:GetStartPoint", get_start_point),
        ("Line:GetEndPoint", get_end_point),
        ("GetMapID", get_map_id),
        ("GetUiMapID", get_ui_map_id),
        // Textures.
        ("TextureBase:GetTexCoord", get_tex_coord),
        ("TextureBase:GetRotation", get_texture_rotation),
        ("TextureBase:GetTextureFilePath", get_texture_file_path),
        ("GetDesaturation", get_desaturation),
        // Cooldowns.
        ("Cooldown:GetCooldownTimes", get_cooldown_times),
        ("Cooldown:GetCooldownDuration", get_cooldown_duration),
        ("Cooldown:SetCooldownDuration", set_cooldown_duration),
        ("Cooldown:Clear", clear_cooldown),
        ("Cooldown:GetCountdownFontString", get_countdown_font_string),
        ("Cooldown:GetHideCountdownNumbers", get_hide_countdown_numbers),
        ("Cooldown:GetRotation", get_plain_rotation),
        // Animations.
        ("Animation:GetElapsed", get_animation_elapsed),
        ("Animation:GetProgress", get_animation_progress),
        ("Animation:GetSmoothProgress", get_smooth_progress),
        ("Animation:GetTarget", get_target),
        ("Animation:GetOrder", get_animation_order),
        ("Animation:GetStartDelay", get_start_delay),
        ("Animation:GetEndDelay", get_end_delay),
        ("ControlPoint:GetOrder", get_control_point_order),
        ("AnimationGroup:GetElapsed", get_group_elapsed),
        ("AnimationGroup:GetProgress", get_group_progress),
        ("AnimationGroup:GetLooping", get_looping),
        (
            "AnimationGroup:GetAnimationSpeedMultiplier",
            get_animation_speed_multiplier,
        ),
        ("Path:CreateControlPoint", create_control_point),
        ("FlipBook:GetFlipBookColumns", get_flip_book_columns),
        ("FlipBook:GetFlipBookRows", get_flip_book_rows),
        ("FlipBook:GetFlipBookFrames", get_flip_book_frames),
        // Tooltips (`GameTooltip`'s lack the documentation's `Tooltip`).
        ("GetLeftLine", get_left_line),
        ("GetRightLine", get_right_line),
        ("GetPadding", get_padding),
        ("ClearPadding", clear_padding),
        ("GetMinimumWidth", get_minimum_width),
        ("GetAnchorType", get_anchor_type),
        ("AppendText", append_text),
        ("FadeOut", fade_out),
        ("AddTexture", accept),
        ("AddAtlas", accept),
        // Models.
        ("GetPosition", get_position),
        ("GetCameraPosition", get_camera_position),
        ("GetCameraTarget", get_camera_target),
        ("GetViewInsets", get_view_insets),
        ("GetFacing", get_facing),
        ("GetYaw", get_yaw),
        ("GetPitch", get_pitch),
        ("GetRoll", get_roll),
        ("GetModelScale", get_model_scale),
        ("GetModelFileID", get_model_file_id),
        ("GetSheathed", get_sheathed),
        ("ModelSceneFrameActorBase:GetAnimation", get_actor_animation),
        ("TryOn", try_on),
        // Message frames.
        ("GetTimeVisible", get_time_visible),
        ("GetFadeDuration", get_fade_duration),
        ("GetFading", get_fading),
        ("GetInsertMode", get_insert_mode),
    ]);
    out
}
