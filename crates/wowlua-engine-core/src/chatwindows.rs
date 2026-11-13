//! The chat windows' settings the client keeps (decision T263): what
//! Blizzard's chat code sets (`SetChatWindowName`, `…Size`, `…Color`,
//! `…Alpha`, `…Shown`, `…Locked`, `…Docked`, `…Uninteractable`,
//! `…SavedPosition`, `…SavedDimensions`) is what `GetChatWindowInfo`,
//! `GetChatWindowSavedPosition` and `GetChatWindowSavedDimensions` answer
//! after, over the recorded answers (the full probe: General docked first,
//! Log second, the others hidden). No events follow (unverified).

use std::collections::HashMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, engine_mut};

/// What was set for one window; `None`: as recorded.
#[derive(Clone, Debug, Default)]
pub struct Window {
    name: Option<String>,
    size: Option<f64>,
    color: Option<(f64, f64, f64)>,
    alpha: Option<f64>,
    shown: Option<bool>,
    locked: Option<bool>,
    /// Docked at this place, or undocked (`Some(None)`).
    docked: Option<Option<f64>>,
    uninteractable: Option<bool>,
    position: Option<(String, f64, f64)>,
    dimensions: Option<(f64, f64)>,
}

/// The settings by window ID.
pub type ChatWindows = HashMap<i64, Window>;

/// The client's chat windows (`NUM_CHAT_WINDOWS`): the others answer
/// nothing (recorded: `GetChatWindowInfo(0)` and `(-1)`).
const WINDOWS: std::ops::RangeInclusive<i64> = 1..=10;

/// The window ID argument: a number, else the client's usage error
/// (recorded: "Usage: GetChatWindowInfo(index)").
fn id_of(vm: &mut Vm, c: &NativeCall, name: &str) -> Result<i64, wowlua_vm::LuaError> {
    match c.arg(1) {
        Value::Num(n) => Ok(n as i64),
        _ => Err(vm.error(&format!("Usage: {name}(index)"))),
    }
}

/// The settings of the window a setter names; `None` for no window.
fn window<'a>(
    vm: &'a mut Vm,
    c: &NativeCall,
    name: &str,
) -> Result<Option<&'a mut Window>, wowlua_vm::LuaError> {
    let id = id_of(vm, c, name)?;
    if !WINDOWS.contains(&id) {
        return Ok(None);
    }
    Ok(Some(
        engine_mut(vm).world.chat_windows.entry(id).or_default(),
    ))
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn set_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text(vm, c.arg(2)).unwrap_or_default();
    if let Some(w) = window(vm, &c, "SetChatWindowName")? {
        w.name = Some(name);
    }
    Ok(vec![])
}

fn set_size(vm: &mut Vm, c: NativeCall) -> Ret {
    let size = num(c.arg(2));
    if let Some(s) = size
        && let Some(w) = window(vm, &c, "SetChatWindowSize")?
    {
        w.size = Some(s);
    }
    Ok(vec![])
}

fn set_color(vm: &mut Vm, c: NativeCall) -> Ret {
    let (r, g, b) = (num(c.arg(2)), num(c.arg(3)), num(c.arg(4)));
    if let (Some(r), Some(g), Some(b)) = (r, g, b)
        && let Some(w) = window(vm, &c, "SetChatWindowColor")?
    {
        w.color = Some((r, g, b));
    }
    Ok(vec![])
}

fn set_alpha(vm: &mut Vm, c: NativeCall) -> Ret {
    let alpha = num(c.arg(2));
    if let Some(a) = alpha
        && let Some(w) = window(vm, &c, "SetChatWindowAlpha")?
    {
        w.alpha = Some(a);
    }
    Ok(vec![])
}

fn set_shown(vm: &mut Vm, c: NativeCall) -> Ret {
    let on = c.arg(2).truthy();
    if let Some(w) = window(vm, &c, "SetChatWindowShown")? {
        w.shown = Some(on);
    }
    Ok(vec![])
}

fn set_locked(vm: &mut Vm, c: NativeCall) -> Ret {
    let on = c.arg(2).truthy();
    if let Some(w) = window(vm, &c, "SetChatWindowLocked")? {
        w.locked = Some(on);
    }
    Ok(vec![])
}

/// `SetChatWindowDocked(id, index)`: docked at that place; `nil` (or
/// `false`) undocks it (Blizzard's `FCF_UnDockFrame`).
fn set_docked(vm: &mut Vm, c: NativeCall) -> Ret {
    let at = num(c.arg(2));
    if let Some(w) = window(vm, &c, "SetChatWindowDocked")? {
        w.docked = Some(at);
    }
    Ok(vec![])
}

fn set_uninteractable(vm: &mut Vm, c: NativeCall) -> Ret {
    let on = c.arg(2).truthy();
    if let Some(w) = window(vm, &c, "SetChatWindowUninteractable")? {
        w.uninteractable = Some(on);
    }
    Ok(vec![])
}

fn set_saved_position(vm: &mut Vm, c: NativeCall) -> Ret {
    let point = text(vm, c.arg(2));
    let (x, y) = (num(c.arg(3)), num(c.arg(4)));
    if let (Some(p), Some(x), Some(y)) = (point, x, y)
        && let Some(w) = window(vm, &c, "SetChatWindowSavedPosition")?
    {
        w.position = Some((p, x, y));
    }
    Ok(vec![])
}

fn set_saved_dimensions(vm: &mut Vm, c: NativeCall) -> Ret {
    let (width, height) = (num(c.arg(2)), num(c.arg(3)));
    if let (Some(width), Some(height)) = (width, height)
        && let Some(w) = window(vm, &c, "SetChatWindowSavedDimensions")?
    {
        w.dimensions = Some((width, height));
    }
    Ok(vec![])
}

/// The recorded answer for window `id` (by its ID alone: Blizzard's code
/// passes more, recorded as `(id)`), padded with nils to `n` values; none
/// for no window.
fn recorded(vm: &mut Vm, name: &str, id: i64, n: usize) -> Vec<Value> {
    if !WINDOWS.contains(&id) {
        return vec![];
    }
    let call = format!("{name}({id})");
    let mut out = crate::recorded::lookup_text(vm, &call).unwrap_or_default();
    out.resize(n.max(out.len()), Value::Nil);
    out
}

/// `GetChatWindowInfo(id)`: name, font size, r, g, b, alpha, shown,
/// locked, docked, uninteractable; as recorded, with what was set since.
fn get_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = id_of(vm, &c, "GetChatWindowInfo")?;
    let mut out = recorded(vm, "GetChatWindowInfo", id, 10);
    let Some(w) = engine(vm).world.chat_windows.get(&id).cloned() else {
        return Ok(out);
    };
    if let Some(name) = &w.name {
        out[0] = vm.str(name);
    }
    if let Some(size) = w.size {
        out[1] = Value::Num(size);
    }
    if let Some((r, g, b)) = w.color {
        out[2] = Value::Num(r);
        out[3] = Value::Num(g);
        out[4] = Value::Num(b);
    }
    if let Some(a) = w.alpha {
        out[5] = Value::Num(a);
    }
    if let Some(on) = w.shown {
        out[6] = Value::Bool(on);
    }
    if let Some(on) = w.locked {
        out[7] = Value::Bool(on);
    }
    if let Some(at) = w.docked {
        out[8] = at.map_or(Value::Nil, Value::Num);
    }
    if let Some(on) = w.uninteractable {
        out[9] = Value::Bool(on);
    }
    Ok(out)
}

/// `GetChatWindowSavedPosition(id)`: point, x, y.
fn get_saved_position(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = "GetChatWindowSavedPosition";
    let id = id_of(vm, &c, name)?;
    let kept = engine(vm)
        .world
        .chat_windows
        .get(&id)
        .and_then(|w| w.position.clone());
    match kept {
        Some((p, x, y)) => Ok(vec![vm.str(&p), Value::Num(x), Value::Num(y)]),
        None => Ok(recorded(vm, name, id, 0)),
    }
}

/// `GetChatWindowSavedDimensions(id)`: width, height.
fn get_saved_dimensions(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = "GetChatWindowSavedDimensions";
    let id = id_of(vm, &c, name)?;
    let kept = engine(vm)
        .world
        .chat_windows
        .get(&id)
        .and_then(|w| w.dimensions);
    match kept {
        Some((w, h)) => Ok(vec![Value::Num(w), Value::Num(h)]),
        None => Ok(recorded(vm, name, id, 0)),
    }
}

/// The functions with behaviour here.
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("SetChatWindowName", set_name),
        ("SetChatWindowSize", set_size),
        ("SetChatWindowColor", set_color),
        ("SetChatWindowAlpha", set_alpha),
        ("SetChatWindowShown", set_shown),
        ("SetChatWindowLocked", set_locked),
        ("SetChatWindowDocked", set_docked),
        ("SetChatWindowUninteractable", set_uninteractable),
        ("SetChatWindowSavedPosition", set_saved_position),
        ("SetChatWindowSavedDimensions", set_saved_dimensions),
        ("GetChatWindowInfo", get_info),
        ("GetChatWindowSavedPosition", get_saved_position),
        ("GetChatWindowSavedDimensions", get_saved_dimensions),
    ]
}
