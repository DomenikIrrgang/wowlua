//! What regions measure and hold beyond their setters: a font string's
//! lines and height (on the engine's text model: 6 units a character,
//! wrapped at the string's laid out width), a scroll frame's ranges, a
//! texture's atlas, the minimap's zoom. Unverified: the text model's
//! metrics against the client's fonts.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::engine;
use crate::widgets::{self, MethodFn, WidgetId};

/// A character's width in the engine's text model (as `GetStringWidth`).
const CHAR_WIDTH: f64 = 6.0;

/// The string's lines: its own, each wrapped at the laid out width when
/// it has one and wraps (`SetWordWrap(false)` keeps one each).
fn lines(vm: &mut Vm, id: WidgetId) -> usize {
    let text = match engine(vm).world.widgets[id.0].text {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => return 0,
    };
    if text.is_empty() {
        return 0;
    }
    let props = engine(vm).world.widgets[id.0].props;
    let wraps = widgets::stored_args(vm, props, "WordWrap")
        .and_then(|v| v.first().copied())
        .is_none_or(|v| v.truthy());
    let width = widgets::laid_out_size(vm, id).0;
    text.split('\n')
        .map(|line| {
            let w = line.chars().count() as f64 * CHAR_WIDTH;
            if wraps && width > 0.0 && w > width {
                (w / width).ceil() as usize
            } else {
                1
            }
        })
        .sum()
}

/// `GetNumLines()`.
fn get_num_lines(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(lines(vm, id) as f64)])
}

/// `GetStringHeight()`: its lines at the font's height, the spacing
/// between them.
fn get_string_height(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let n = lines(vm, id) as f64;
    let font = widgets::get_font(vm, id, c)?;
    let size = match font.get(1) {
        Some(Value::Num(h)) => *h,
        _ => 0.0,
    };
    let props = engine(vm).world.widgets[id.0].props;
    let spacing = match widgets::stored_args(vm, props, "Spacing").and_then(|v| v.first().copied())
    {
        Some(Value::Num(s)) => s,
        _ => 0.0,
    };
    let h = if n > 0.0 {
        n * size + (n - 1.0) * spacing
    } else {
        0.0
    };
    Ok(vec![Value::Num(h)])
}

/// A scroll frame's child's size beyond its own (0 without a child).
fn scroll_range(vm: &mut Vm, id: WidgetId, vertical: bool) -> f64 {
    let props = engine(vm).world.widgets[id.0].props;
    let child = widgets::stored_args(vm, props, "ScrollChild")
        .and_then(|v| v.first().copied())
        .and_then(|v| widgets::widget_of(vm, v));
    let Some(child) = child else {
        return 0.0;
    };
    let (cw, ch) = widgets::laid_out_size(vm, child);
    let (w, h) = widgets::laid_out_size(vm, id);
    if vertical {
        (ch - h).max(0.0)
    } else {
        (cw - w).max(0.0)
    }
}

fn get_vertical_scroll_range(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(scroll_range(vm, id, true))])
}

fn get_horizontal_scroll_range(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(scroll_range(vm, id, false))])
}

/// `GetAtlas()`: the atlas set, else nil.
fn get_atlas(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let atlas = widgets::stored_args(vm, props, "Atlas").and_then(|v| v.first().copied());
    Ok(vec![
        atlas
            .filter(|v| matches!(v, Value::Str(_)))
            .unwrap_or(Value::Nil),
    ])
}

/// `Minimap:GetZoom()`: the zoom set, 0 at first.
fn get_zoom(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let zoom = widgets::stored_args(vm, props, "Zoom").and_then(|v| v.first().copied());
    Ok(vec![
        zoom.filter(|v| matches!(v, Value::Num(_)))
            .unwrap_or(Value::Num(0.0)),
    ])
}

pub fn methods() -> Vec<(&'static str, MethodFn)> {
    vec![
        ("GetNumLines", get_num_lines),
        ("GetStringHeight", get_string_height),
        ("GetVerticalScrollRange", get_vertical_scroll_range),
        ("GetHorizontalScrollRange", get_horizontal_scroll_range),
        ("GetAtlas", get_atlas),
        ("GetZoom", get_zoom),
    ]
}
