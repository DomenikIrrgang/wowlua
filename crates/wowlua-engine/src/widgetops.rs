//! Widget methods with no result whose effect the widget's getters show:
//! until these, the engine took them for setters without a getter and did
//! nothing (decision T289).
//!
//! - `Frame:Raise()` and `Frame:Lower()`: the frame's level above (below)
//!   every other shown frame's in its strata, as `GetFrameLevel` answers.
//! - An edit box's selection: `HighlightText([start [, end]])` selects
//!   (the whole text by default; `HighlightText(0, 0)` nothing),
//!   `ClearHighlightText()` drops it; `Insert(text)` puts the text in place
//!   of the selection, else at the cursor, the cursor after it. What the
//!   user types (`mock.type`) goes there too; `SetText` puts the cursor at
//!   the end and drops the selection, as the user's click or typing into a
//!   box without the focus does before its `OnEditFocusGained`.
//! - `Font:CopyFontObject(font)`: the other font's font, colors, shadow,
//!   spacing and justification copied, as its getters answer them.
//! - `AdjustPointsOffset(x, y)`: every anchor moved by (x, y);
//!   `ClearPointsOffset()`: every anchor's offset 0.
//! - The clears: `ClearModel()` (no model: `GetModelFileID` 0), a button's
//!   `ClearNormalTexture()` and the other states' (`GetNormalTexture` nil,
//!   the texture hidden), `FontString:ClearText()`, `ClearParentKey()`,
//!   `ResetTexCoord()`, `StopAnimating()` (every animation group of the
//!   region stopped).
//!
//! Unverified, none recorded: which frames `Raise` counts (the shown ones
//! of the strata here) and the level cap (10000), `Insert`'s
//! `OnTextChanged` (`false`, the frame's update after, as `SetText`'s),
//! a selection given backwards, `ClearPointsOffset` zeroing the offsets
//! `SetPoint` gave, and the stems `ClearModel` drops.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::widgets::{self, MethodFn, WidgetId};
use crate::{engine, engine_mut};

/// The level a frame can have at most.
const MAX_LEVEL: f64 = 10000.0;

/// The selection's stem (the engine's own: no setter has it).
const HIGHLIGHT: &str = "\u{0}Highlight";

/// Drops what setter `stem` stored.
fn unset(vm: &mut Vm, id: WidgetId, stem: &str) {
    let props = engine(vm).world.widgets[id.0].props;
    let key = vm.str(stem);
    let _ = vm.heap.table_mut(props).set(key, Value::Nil);
}

// --------------------------------------------------------------- levels

/// Whether `id` is `root` or one of its descendants.
fn under(vm: &Vm, mut id: WidgetId, root: WidgetId) -> bool {
    let e = engine(vm);
    loop {
        if id == root {
            return true;
        }
        match e.world.widgets[id.0].parent {
            Some(p) => id = p,
            None => return false,
        }
    }
}

/// The levels of the other shown frames in `id`'s strata (not its own
/// children's).
fn other_levels(vm: &mut Vm, id: WidgetId) -> Vec<f64> {
    let rank = widgets::strata_rank(vm, id);
    let candidates: Vec<WidgetId> = {
        let e = engine(vm);
        (0..e.world.widgets.len())
            .map(WidgetId)
            .filter(|&w| {
                let class = &e.world.widgets[w.0].class;
                widgets::is_a(e, class, "Frame") && widgets::visible(e, w)
            })
            .collect()
    };
    let mut out = Vec::new();
    for w in candidates {
        if under(vm, w, id) || widgets::strata_rank(vm, w) != rank {
            continue;
        }
        out.push(widgets::frame_level(vm, w));
    }
    out
}

/// `Frame:Raise()`: above every other shown frame of its strata.
fn raise(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let own = widgets::frame_level(vm, id);
    let top = other_levels(vm, id).into_iter().fold(f64::MIN, f64::max);
    if top >= own {
        let level = (top + 1.0).min(MAX_LEVEL);
        widgets::store(vm, id, "FrameLevel", &[Value::Num(level)]);
    }
    Ok(vec![])
}

/// `Frame:Lower()`: below every other shown frame of its strata (0 at the
/// lowest).
fn lower(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let own = widgets::frame_level(vm, id);
    let bottom = other_levels(vm, id).into_iter().fold(f64::MAX, f64::min);
    if bottom <= own {
        let level = (bottom - 1.0).max(0.0);
        widgets::store(vm, id, "FrameLevel", &[Value::Num(level)]);
    }
    Ok(vec![])
}

// ------------------------------------------------------------ edit boxes

fn text_of(vm: &Vm, id: WidgetId) -> String {
    match engine(vm).world.widgets[id.0].text {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.heap.str_bytes(s)).into_owned(),
        _ => String::new(),
    }
}

fn first_number(vm: &mut Vm, id: WidgetId, stem: &str) -> Option<f64> {
    let props = engine(vm).world.widgets[id.0].props;
    match widgets::stored_args(vm, props, stem).and_then(|v| v.first().copied()) {
        Some(Value::Num(n)) => Some(n),
        _ => None,
    }
}

/// A byte offset in `text`, moved back to a character's start.
fn at_char(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The cursor's byte offset (`SetCursorPosition`'s, else the text's end).
fn cursor(vm: &mut Vm, id: WidgetId, text: &str) -> usize {
    match first_number(vm, id, "CursorPosition") {
        Some(n) if n >= 0.0 => at_char(text, n as usize),
        Some(_) => 0,
        None => text.len(),
    }
}

/// The selection's byte range, if any text is selected.
fn selection(vm: &mut Vm, id: WidgetId, text: &str) -> Option<(usize, usize)> {
    let props = engine(vm).world.widgets[id.0].props;
    let v = widgets::stored_args(vm, props, HIGHLIGHT)?;
    let (Some(Value::Num(a)), Some(Value::Num(b))) = (v.first(), v.get(1)) else {
        return None;
    };
    let (a, b) = (at_char(text, *a as usize), at_char(text, *b as usize));
    (a < b).then_some((a, b))
}

/// `HighlightText([start [, end]])`: selects from `start` (0) to `end`
/// (the text's end; a negative one too), byte offsets as the cursor's.
fn highlight_text(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let len = text_of(vm, id).len() as f64;
    let start = match vm.hidden(c.arg(2)) {
        Value::Num(n) => n.max(0.0),
        _ => 0.0,
    };
    let end = match vm.hidden(c.arg(3)) {
        Value::Num(n) if n >= 0.0 => n.min(len),
        _ => len,
    };
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    widgets::store(vm, id, HIGHLIGHT, &[Value::Num(start), Value::Num(end)]);
    Ok(vec![])
}

fn clear_highlight_text(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    unset(vm, id, HIGHLIGHT);
    Ok(vec![])
}

/// Puts `text` into the edit box as the user's typing and `Insert` do: in
/// place of the selection, else at the cursor; the cursor after it.
pub fn insert(vm: &mut Vm, id: WidgetId, text: &str) {
    let mut now = text_of(vm, id);
    let (from, to) = match selection(vm, id, &now) {
        Some(range) => range,
        None => {
            let at = cursor(vm, id, &now);
            (at, at)
        }
    };
    now.replace_range(from..to, text);
    let v = vm.str(&now);
    widgets::store_text(vm, id, Some(v));
    unset(vm, id, HIGHLIGHT);
    let at = (from + text.len()) as f64;
    widgets::store(vm, id, "CursorPosition", &[Value::Num(at)]);
}

/// The text set: the cursor at its end, nothing selected.
pub fn text_set(vm: &mut Vm, id: WidgetId) {
    unset(vm, id, HIGHLIGHT);
    unset(vm, id, "CursorPosition");
}

/// `EditBox:Insert(text)`: the text in, its `OnTextChanged(false)` in the
/// frame's update, as `SetText`'s.
fn insert_text(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let text = match vm.hidden(c.arg(2)) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        v @ Value::Num(_) => match vm.tostring(v) {
            Ok(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    insert(vm, id, &text);
    if !engine(vm).world.text_set.contains(&id) {
        engine_mut(vm).world.text_set.push(id);
    }
    Ok(vec![])
}

// --------------------------------------------------------------- fonts

/// What a font's getters read, by setter stem.
const FONT_STEMS: &[&str] = &[
    "TextColor",
    "ShadowColor",
    "ShadowOffset",
    "Spacing",
    "JustifyH",
    "JustifyV",
    "IndentedWordWrap",
];

/// `Font:CopyFontObject(font)`: a font object or its name; its values
/// become this font's own.
fn copy_font_object(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let from = match c.arg(2) {
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            vm.get_global(&name)
        }
        v => v,
    };
    let Some(from) = widgets::widget_of(vm, from) else {
        return Err(vm.api_arg_error(2, "Usage: self:CopyFontObject(font)"));
    };
    let font = widgets::get_font(vm, from, c)?;
    if font.first().is_some_and(|v| !v.is_nil()) {
        widgets::store(vm, id, "Font", &font);
    }
    for stem in FONT_STEMS {
        if let Some(v) = crate::methods2::font_chain(vm, from, stem) {
            widgets::store(vm, id, stem, &v);
        }
    }
    Ok(vec![])
}

// --------------------------------------------------------------- points

/// `AdjustPointsOffset(x, y)`: every anchor's offset moved by (x, y).
fn adjust_points_offset(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let dx = vm.check_number(&c, 2)?;
    let dy = vm.check_number(&c, 3)?;
    for p in &mut engine_mut(vm).world.widgets[id.0].points {
        p.x += dx;
        p.y += dy;
    }
    widgets::layout_changed(vm);
    Ok(vec![])
}

/// `ClearPointsOffset()`: every anchor's offset 0.
fn clear_points_offset(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    for p in &mut engine_mut(vm).world.widgets[id.0].points {
        p.x = 0.0;
        p.y = 0.0;
    }
    widgets::layout_changed(vm);
    Ok(vec![])
}

// --------------------------------------------------------------- clears

fn same(a: Value, b: Value) -> bool {
    match (a, b) {
        (Value::Table(x), Value::Table(y)) => x == y,
        (Value::Userdata(x), Value::Userdata(y)) => x == y,
        _ => false,
    }
}

/// `ClearModel()`: no model shown.
fn clear_model(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    for stem in [
        "Model",
        "ModelByFileID",
        "ModelByCreatureDisplayID",
        "ModelByUnit",
        "DisplayInfo",
        "CreatureDisplayID",
        "Creature",
        "Unit",
    ] {
        unset(vm, id, stem);
    }
    Ok(vec![])
}

/// A button's `Clear…Texture()`: the state's texture gone (hidden, no
/// longer the button's).
fn clear_state_texture(vm: &mut Vm, id: WidgetId, state: &str) -> Ret {
    let stem = format!("{state}Texture");
    let props = engine(vm).world.widgets[id.0].props;
    let region = widgets::stored_args(vm, props, &stem)
        .and_then(|v| v.first().copied())
        .and_then(|v| widgets::widget_of(vm, v));
    if let Some(r) = region {
        engine_mut(vm).world.widgets[r.0].shown = false;
    }
    unset(vm, id, &stem);
    Ok(vec![])
}

fn clear_normal_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    clear_state_texture(vm, id, "Normal")
}

fn clear_pushed_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    clear_state_texture(vm, id, "Pushed")
}

fn clear_disabled_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    clear_state_texture(vm, id, "Disabled")
}

fn clear_highlight_texture(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    clear_state_texture(vm, id, "Highlight")
}

/// `FontString:ClearText()`: no text (`GetText` nil).
fn clear_text(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    widgets::store_text(vm, id, None);
    Ok(vec![])
}

/// `ClearParentKey()`: the parent no longer holds it under its key.
fn clear_parent_key(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let (parent, me) = {
        let e = engine(vm);
        (e.world.widgets[id.0].parent, widgets::value_of(e, id))
    };
    if let Some(p) = parent {
        let pt = engine(vm).world.widgets[p.0].table;
        let keys: Vec<Value> = vm
            .heap
            .table(pt)
            .entries()
            .filter(|(k, v)| matches!(k, Value::Str(_)) && same(*v, me))
            .map(|(k, _)| k)
            .collect();
        for k in keys {
            let _ = vm.heap.table_mut(pt).set(k, Value::Nil);
        }
    }
    unset(vm, id, "ParentKey");
    Ok(vec![])
}

/// `ResetTexCoord()`: the whole texture (`GetTexCoord`'s 0, 0, 0, 1, 1, 0,
/// 1, 1).
fn reset_tex_coord(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    unset(vm, id, "TexCoord");
    Ok(vec![])
}

/// `StopAnimating()`: every animation group of the region stopped.
fn stop_animating(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let groups: Vec<WidgetId> = {
        let e = engine(vm);
        e.world.widgets[id.0]
            .children
            .iter()
            .copied()
            .filter(|g| e.world.widgets[g.0].class == "AnimationGroup")
            .collect()
    };
    for g in groups {
        crate::animations::stop_group(vm, g);
    }
    Ok(vec![])
}

pub fn methods() -> Vec<(&'static str, MethodFn)> {
    vec![
        ("Frame:Raise", raise),
        ("Frame:Lower", lower),
        ("EditBox:HighlightText", highlight_text),
        ("EditBox:ClearHighlightText", clear_highlight_text),
        ("EditBox:Insert", insert_text),
        ("Font:CopyFontObject", copy_font_object),
        ("AdjustPointsOffset", adjust_points_offset),
        ("ClearPointsOffset", clear_points_offset),
        ("ClearModel", clear_model),
        ("Button:ClearNormalTexture", clear_normal_texture),
        ("Button:ClearPushedTexture", clear_pushed_texture),
        ("Button:ClearDisabledTexture", clear_disabled_texture),
        ("Button:ClearHighlightTexture", clear_highlight_texture),
        ("FontString:ClearText", clear_text),
        ("ClearParentKey", clear_parent_key),
        ("ResetTexCoord", reset_tex_coord),
        ("StopAnimating", stop_animating),
    ]
}
