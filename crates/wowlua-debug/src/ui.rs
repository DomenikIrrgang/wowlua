//! The UI requests (spec debugger §18.9, §18.12): the frame tree, finding
//! frames, input as the user gives it, firing events.

use serde_json::{Value as Json, json};
use wowlua_engine::widgets::{self, WidgetId};
use wowlua_vm::{Value, Vm};

use crate::protocol::Input;

/// The widget a name gives: a global's (`MyFrame`), or a path from one
/// (`MyFrame.title`).
pub fn widget_named(vm: &mut Vm, name: &str) -> Option<WidgetId> {
    let path: Vec<String> = name.split('.').map(str::to_string).collect();
    let v = crate::values::lookup_path(vm, &path)?;
    widgets::widget_of(vm, v)
}

fn node(vm: &Vm, id: WidgetId, hidden: bool, depth: usize) -> Option<Json> {
    let e = wowlua_engine::engine(vm);
    let w = &e.world.widgets[id.0];
    let visible = widgets::visible(e, id);
    if !hidden && !w.shown {
        return None;
    }
    let mut n = json!({
        "id": id.0,
        "type": w.class,
        "shown": w.shown,
        "visible": visible,
    });
    if let Some(name) = &w.name {
        n["name"] = json!(name);
    }
    // Screen pixels from the top left, as screenshots are.
    if let Some(r) = wowlua_engine::scene::rect_of(vm, id) {
        n["rect"] = json!({
            "x": r.x.round(),
            "y": r.y.round(),
            "width": r.w.round(),
            "height": r.h.round(),
        });
    }
    if let Some(Value::Str(s)) = w.text {
        n["text"] = json!(String::from_utf8_lossy(vm.str_bytes(s)));
    }
    if depth < 64 {
        let children: Vec<Json> = w
            .children
            .iter()
            .filter_map(|&c| node(vm, c, hidden, depth + 1))
            .collect();
        if !children.is_empty() {
            n["children"] = json!(children);
        }
    }
    Some(n)
}

/// The frame tree under `root` (UIParent's, and the frames without a
/// parent, by default).
pub fn frame_tree(vm: &mut Vm, root: Option<&str>, hidden: bool) -> Result<Json, String> {
    if let Some(name) = root {
        let id = widget_named(vm, name).ok_or_else(|| format!("no frame {name}"))?;
        return Ok(node(vm, id, true, 0).unwrap_or(Json::Null));
    }
    let e = wowlua_engine::engine(vm);
    let roots: Vec<WidgetId> = (0..e.world.widgets.len())
        .map(WidgetId)
        .filter(|&id| {
            // Frames without a parent: UIParent and WorldFrame (the engine
            // makes them, not `CreateFrame`) and those made without one.
            let w = &e.world.widgets[id.0];
            w.parent.is_none() && (w.frame || widgets::is_a(e, &w.class, "Frame"))
        })
        .collect();
    let list: Vec<Json> = roots
        .into_iter()
        .filter_map(|id| node(vm, id, hidden, 0))
        .collect();
    Ok(json!(list))
}

/// Frames whose name, type or text contains `query` (any case).
pub fn find_frames(vm: &mut Vm, query: &str) -> Json {
    let q = query.to_lowercase();
    let e = wowlua_engine::engine(vm);
    let mut out = Vec::new();
    for (i, w) in e.world.widgets.iter().enumerate() {
        let text = match w.text {
            Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        };
        let name = w.name.clone().unwrap_or_default();
        if name.to_lowercase().contains(&q)
            || w.class.to_lowercase() == q
            || text.to_lowercase().contains(&q)
        {
            if let Some(mut n) = node(vm, WidgetId(i), true, 64) {
                n.as_object_mut().map(|o| o.remove("children"));
                if let Some(p) = w.parent {
                    n["parent"] = json!(widgets::widget_label(e, p));
                }
                out.push(n);
            }
            if out.len() >= 200 {
                break;
            }
        }
    }
    json!(out)
}

/// The widget an input acts on: by name, or the topmost mouse-enabled
/// visible frame at a point.
fn target(
    vm: &mut Vm,
    frame: &Option<String>,
    x: Option<f64>,
    y: Option<f64>,
) -> Result<Value, String> {
    let id = match (frame, x, y) {
        (Some(name), _, _) => widget_named(vm, name).ok_or_else(|| format!("no frame {name}"))?,
        (None, Some(x), Some(y)) => wowlua_engine::scene::hit_test(vm, x, y)
            .ok_or_else(|| format!("no mouse-enabled frame at {x}, {y}"))?,
        _ => return Err("a frame name or a point".into()),
    };
    Ok(Value::Table(
        wowlua_engine::engine(vm).world.widgets[id.0].table,
    ))
}

fn call_control(
    vm: &mut Vm,
    name: &'static str,
    f: wowlua_vm::NativeFn,
    args: &[Value],
) -> Result<(), String> {
    let native = vm.native(name, f);
    vm.pcall(native, args)
        .map(|_| ())
        .map_err(|e| crate::values::error_text(vm, e.value))
}

/// Input as the user gives it.
pub fn input(vm: &mut Vm, input: &Input) -> Result<(), String> {
    use wowlua_engine::input as inp;
    match input {
        Input::Click {
            frame,
            x,
            y,
            button,
        } => {
            let t = target(vm, frame, *x, *y)?;
            let b = vm.str(button);
            call_control(vm, "click", inp::control_click, &[t, b])
        }
        Input::Hover { frame, x, y } => {
            let t = if frame.is_none() && x.is_none() {
                Value::Nil
            } else {
                target(vm, frame, *x, *y)?
            };
            call_control(vm, "hover", inp::control_hover, &[t])
        }
        Input::Drag { from, to } => {
            let a = target(vm, &Some(from.clone()), None, None)?;
            let b = target(vm, &Some(to.clone()), None, None)?;
            call_control(vm, "dragTo", wowlua_engine::controls::drag_to, &[a, b])
        }
        Input::Type { text } => {
            let focus = wowlua_engine::engine(vm).world.input.keyboard_focus;
            let Some(id) = focus else {
                return Err("no edit box has the keyboard focus (click one first)".into());
            };
            let t = Value::Table(wowlua_engine::engine(vm).world.widgets[id.0].table);
            let s = vm.str(text);
            call_control(vm, "type", inp::control_type, &[t, s])
        }
        Input::Key { key } => {
            inp::as_user(vm, |vm| inp::press_key(vm, key));
            Ok(())
        }
        Input::Slash { text } => {
            let t = vm.str(text);
            call_control(vm, "slash", wowlua_engine::chat::slash_control, &[t])
        }
    }
}

/// Fires a documented event with JSON arguments.
pub fn fire_event(vm: &mut Vm, event: &str, args: &[Json]) -> Result<(), String> {
    let mut values = Vec::new();
    for a in args {
        values.push(match a {
            Json::Null => Value::Nil,
            Json::Bool(b) => Value::Bool(*b),
            Json::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
            Json::String(s) => vm.str(s),
            other => {
                return Err(format!(
                    "event arguments are nil, booleans, numbers or strings, not {other}"
                ));
            }
        });
    }
    widgets::fire_event(vm, event, &values);
    Ok(())
}

/// Standard base64 (for PNGs in JSON).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Changes the world: runs `script` (Lua) with `mock` the engine's
/// controls, untainted as the world's own state, as a world profile is
/// applied (decision T401). Other names are the globals.
pub fn world(vm: &mut Vm, script: &str) -> Result<(), String> {
    let controls = wowlua_engine::controls::table(vm);
    let env = vm.new_table();
    let meta = vm.new_table();
    let index = vm.str("__index");
    let globals = Value::Table(vm.globals);
    let _ = vm.heap.table_mut(meta).set(index, globals);
    vm.heap.table_mut(env).meta = Some(meta);
    let key = vm.str("mock");
    let _ = vm.heap.table_mut(env).set(key, Value::Table(controls));
    let saved = *vm.taint();
    {
        let t = vm.taint_mut();
        t.stack = wowlua_vm::Taint::SECURE;
        t.fixed = wowlua_vm::Taint::SECURE;
        t.new_object = wowlua_vm::Taint::SECURE;
        t.new_closure = wowlua_vm::Taint::SECURE;
    }
    let result = vm.load(script.as_bytes(), "=[world]").and_then(|f| {
        if let Value::Func(func) = f {
            vm.set_func_env(func, env);
        }
        vm.call(f, &[]).map(drop).map_err(|e| match e.value {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        })
    });
    *vm.taint_mut() = saved;
    result
}
