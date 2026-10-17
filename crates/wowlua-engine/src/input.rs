//! The user's input (spec Tooling §16.1.4, the roadmap's GUI tests): the
//! mouse over a frame (`OnEnter`, `OnLeave`, `GetMouseFoci`, `IsMouseOver`),
//! clicks, an edit box's keyboard focus and typing (`OnChar`,
//! `OnTextChanged` with `userInput`), and keys a focused edit box takes.
//!
//! Verified against the client (`/stdtest engine input`, Forever 70425):
//! `OnEnter(true)` and `OnLeave(true)`; a click's scripts (`OnMouseDown`,
//! `OnMouseUp(button, true)`, then `PreClick`, `OnClick`, `PostClick` with
//! `(button, false)` for an enabled button); `OnChar(char)` then
//! `OnTextChanged(true)` per typed character; `MODIFIER_STATE_CHANGED`
//! with `LSHIFT` and 1 / 0; a click on an edit box focusing it; the focus
//! scripts' `userInput`, true while the user's action runs (also for a
//! `ClearFocus` in `OnEnterPressed`).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::widgets::{self, MethodFn, WidgetId};
use crate::{engine, engine_mut};

/// Where the mouse and the keyboard are.
#[derive(Default)]
pub struct Input {
    /// The frame under the mouse.
    pub mouse_focus: Option<WidgetId>,
    /// The edit box with the keyboard focus.
    pub keyboard_focus: Option<WidgetId>,
    /// Shift, Ctrl and Alt held (the left keys).
    pub modifiers: [bool; 3],
    /// The user's actions running (a click, typing, a key): scripts' and
    /// focus changes' `userInput`.
    pub user_actions: u32,
}

/// Runs `f` as the user's action.
pub fn as_user<T>(vm: &mut Vm, f: impl FnOnce(&mut Vm) -> T) -> T {
    engine_mut(vm).world.input.user_actions += 1;
    let out = f(vm);
    engine_mut(vm).world.input.user_actions -= 1;
    out
}

const MODIFIERS: [(&str, &str); 3] = [("shift", "LSHIFT"), ("ctrl", "LCTRL"), ("alt", "LALT")];

/// `setModifiers({ shift?, ctrl?, alt? })`: the keys held (the left ones);
/// `MODIFIER_STATE_CHANGED(key, 1 | 0)` for each that changes.
pub fn control_set_modifiers(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("mock.setModifiers: a table of the keys held expected"));
    };
    for (i, (field, key)) in MODIFIERS.iter().enumerate() {
        let k = vm.str(field);
        let held = vm.heap.table(t).get(k).truthy();
        if engine(vm).world.input.modifiers[i] != held {
            engine_mut(vm).world.input.modifiers[i] = held;
            let key = vm.str(key);
            crate::widgets::fire_event(
                vm,
                "MODIFIER_STATE_CHANGED",
                &[key, Value::Num(if held { 1.0 } else { 0.0 })],
            );
        }
    }
    Ok(vec![])
}

fn held(vm: &Vm, i: usize) -> Ret {
    Ok(vec![Value::Bool(engine(vm).world.input.modifiers[i])])
}

fn is_shift_key_down(vm: &mut Vm, _c: NativeCall) -> Ret {
    held(vm, 0)
}

fn is_control_key_down(vm: &mut Vm, _c: NativeCall) -> Ret {
    held(vm, 1)
}

fn is_alt_key_down(vm: &mut Vm, _c: NativeCall) -> Ret {
    held(vm, 2)
}

fn is_modifier_key_down(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        engine(vm).world.input.modifiers.iter().any(|m| *m),
    )])
}

fn right_key_down(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn widget_arg(
    vm: &mut Vm,
    c: &NativeCall,
    n: usize,
    what: &str,
) -> Result<WidgetId, wowlua_vm::LuaError> {
    widgets::widget_of(vm, c.arg(n)).ok_or_else(|| vm.error(&format!("{what}: a frame expected")))
}

fn label(vm: &Vm, id: WidgetId) -> String {
    let w = &engine(vm).world.widgets[id.0];
    match &w.name {
        Some(n) => n.clone(),
        None => format!("an unnamed {}", w.class),
    }
}

/// `hover(frame)` (`nil`: away): `OnLeave` for the frame the mouse left,
/// `OnEnter` for the new one (`motion` true).
pub fn control_hover(vm: &mut Vm, c: NativeCall) -> Ret {
    let new = widgets::widget_of(vm, c.arg(1));
    if !c.arg(1).is_nil() && new.is_none() {
        return Err(vm.error("mock.hover: a frame or nil expected"));
    }
    let old = engine(vm).world.input.mouse_focus;
    if old == new {
        return Ok(vec![]);
    }
    engine_mut(vm).world.input.mouse_focus = new;
    if let Some(o) = old {
        widgets::run_script(vm, o, "OnLeave", &[Value::Bool(true)]);
    }
    if let Some(n) = new {
        widgets::run_script(vm, n, "OnEnter", &[Value::Bool(true)]);
    }
    Ok(vec![])
}

/// `click(frame, button)`: the user's click on a visible frame: the mouse
/// over it (`OnEnter`), `GLOBAL_MOUSE_DOWN`, `OnMouseDown`,
/// `GLOBAL_MOUSE_UP`, `OnMouseUp`, then a button's click scripts unless it
/// is disabled.
pub fn control_click(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = widget_arg(vm, &c, 1, "mock.click")?;
    if !widgets::visible(engine(vm), id) {
        let name = label(vm, id);
        return Err(vm.error(&format!("mock.click: {name} is not visible")));
    }
    let button = match c.arg(2) {
        Value::Str(s) => Value::Str(s),
        _ => vm.str("LeftButton"),
    };
    // The mouse is over what it clicks.
    let old = engine(vm).world.input.mouse_focus;
    if old != Some(id) {
        engine_mut(vm).world.input.mouse_focus = Some(id);
        if let Some(o) = old {
            widgets::run_script(vm, o, "OnLeave", &[Value::Bool(true)]);
        }
        widgets::run_script(vm, id, "OnEnter", &[Value::Bool(true)]);
    }
    as_user(vm, |vm| {
        // Every click is also the global mouse events (Blizzard's menus close
        // on one outside them). Unverified: their order against the scripts.
        widgets::fire_event(vm, "GLOBAL_MOUSE_DOWN", &[button]);
        widgets::run_script(vm, id, "OnMouseDown", &[button]);
        widgets::fire_event(vm, "GLOBAL_MOUSE_UP", &[button]);
        widgets::run_script(vm, id, "OnMouseUp", &[button, Value::Bool(true)]);
        let class = engine(vm).world.widgets[id.0].class.clone();
        let is = |vm: &Vm, kind: &str| {
            widgets::lineage(engine(vm), &class)
                .iter()
                .any(|c| c.eq_ignore_ascii_case(kind))
        };
        let enabled = engine(vm).world.widgets[id.0].enabled;
        if is(vm, "Button") && enabled {
            for script in ["PreClick", "OnClick", "PostClick"] {
                widgets::run_script(vm, id, script, &[button, Value::Bool(false)]);
            }
        }
        if is(vm, "EditBox") && enabled {
            set_keyboard_focus(vm, Some(id));
        }
    });
    Ok(vec![])
}

fn set_keyboard_focus(vm: &mut Vm, new: Option<WidgetId>) {
    let old = engine(vm).world.input.keyboard_focus;
    if old == new {
        return;
    }
    engine_mut(vm).world.input.keyboard_focus = new;
    let user = Value::Bool(engine(vm).world.input.user_actions > 0);
    if let Some(o) = old {
        widgets::run_script(vm, o, "OnEditFocusLost", &[user]);
    }
    if let Some(n) = new {
        widgets::run_script(vm, n, "OnEditFocusGained", &[user]);
    }
}

fn text_of(vm: &mut Vm, id: WidgetId) -> String {
    match engine(vm).world.widgets[id.0].text {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    }
}

/// `type(editBox, text)`: focuses the edit box, then types each character:
/// the text grows, `OnChar(char)`, `OnTextChanged(true)`.
pub fn control_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = widget_arg(vm, &c, 1, "mock.type")?;
    let typed = vm.check_str(&c, 2)?;
    let typed = String::from_utf8_lossy(vm.str_bytes(typed)).into_owned();
    if !widgets::visible(engine(vm), id) {
        let name = label(vm, id);
        return Err(vm.error(&format!("mock.type: {name} is not visible")));
    }
    as_user(vm, |vm| {
        set_keyboard_focus(vm, Some(id));
        for ch in typed.chars() {
            let mut text = text_of(vm, id);
            text.push(ch);
            let v = vm.str(&text);
            engine_mut(vm).world.widgets[id.0].text = Some(v);
            let ch = vm.str(&ch.to_string());
            widgets::run_script(vm, id, "OnChar", &[ch]);
            widgets::run_script(vm, id, "OnTextChanged", &[Value::Bool(true)]);
        }
    });
    Ok(vec![])
}

/// A key the focused edit box takes, whether it took it: every key while
/// it has the focus (unverified: typing into chat runs no binding),
/// `ENTER`, `ESCAPE`, `TAB` and `SPACE` running its scripts.
pub fn edit_box_key(vm: &mut Vm, key: &str) -> bool {
    let Some(id) = engine(vm).world.input.keyboard_focus else {
        return false;
    };
    let script = match key {
        "ENTER" => "OnEnterPressed",
        "ESCAPE" => "OnEscapePressed",
        "TAB" => "OnTabPressed",
        "SPACE" => "OnSpacePressed",
        _ => return true,
    };
    as_user(vm, |vm| widgets::run_script(vm, id, script, &[]));
    true
}

/// The shown frames taking keys (`EnableKeyboard`, or a key script),
/// the top first: by strata, then level, then the later made. Edit boxes
/// take keys by their focus alone (the chat's box, shown with a key
/// script, keeps no key from the bindings).
fn keyboard_frames(vm: &mut Vm) -> Vec<WidgetId> {
    let mut frames: Vec<(usize, i64, usize)> = Vec::new();
    for i in 0..engine(vm).world.widgets.len() {
        let id = WidgetId(i);
        let class = engine(vm).world.widgets[i].class.clone();
        if widgets::is_a(engine(vm), &class, "EditBox") {
            continue;
        }
        let props = engine(vm).world.widgets[i].props;
        let on = widgets::stored_args(vm, props, "KeyboardEnabled")
            .and_then(|v| v.first().copied())
            .is_some_and(|v| v.truthy());
        if on && widgets::visible(engine(vm), id) {
            let rank = widgets::strata_rank(vm, id);
            let level = widgets::frame_level(vm, id) as i64;
            frames.push((rank, level, i));
        }
    }
    frames.sort_by(|a, b| b.cmp(a));
    frames.into_iter().map(|(_, _, i)| WidgetId(i)).collect()
}

/// Whether a frame has an `OnKeyDown` script or hook.
fn has_key_handler(vm: &mut Vm, id: WidgetId) -> bool {
    let (scripts, hooks) = {
        let w = &engine(vm).world.widgets[id.0];
        (w.scripts, w.hooks)
    };
    let key = vm.heap.intern(b"OnKeyDown");
    !vm.heap.table(scripts).get_str(key).is_nil() || !vm.heap.table(hooks).get_str(key).is_nil()
}

fn propagates(vm: &mut Vm, id: WidgetId) -> bool {
    let props = engine(vm).world.widgets[id.0].props;
    widgets::stored_args(vm, props, "PropagateKeyboardInput")
        .and_then(|v| v.first().copied())
        .is_some_and(|v| v.truthy())
}

/// The modifier keys themselves (no chord of their own).
const MODIFIER_KEYS: &[&str] = &[
    "LSHIFT", "RSHIFT", "LCTRL", "RCTRL", "LALT", "RALT", "LMETA", "RMETA",
];

/// The key with the modifiers held, as bindings name it (`ALT-CTRL-SHIFT-`
/// order, Blizzard's `CreateKeyChordStringUsingMetaKeyState`).
fn chord(vm: &Vm, key: &str) -> String {
    if MODIFIER_KEYS.contains(&key) {
        return key.to_string();
    }
    let [shift, ctrl, alt] = engine(vm).world.input.modifiers;
    let mut out = String::new();
    for (held, name) in [(alt, "ALT-"), (ctrl, "CTRL-"), (shift, "SHIFT-")] {
        if held {
            out.push_str(name);
        }
    }
    out.push_str(key);
    out
}

/// A key pressed and released (as the user): the focused edit box takes
/// it; else each keyboard frame, the top first, runs `OnKeyDown(key)`
/// until one does not pass it on (it then has the key: its `OnKeyUp` alone
/// runs, and no binding). A key no frame kept runs its chord's command
/// (`chord`), down then up.
///
/// Verified (`/stdtest engine keys`, Forever 70425): the order, the frame
/// that does not propagate keeping the key, its binding not run, the key's
/// release to that frame alone. Unverified: the release when no frame kept
/// the key (here: to every keyboard frame), a propagation turned off in the
/// handler (here: it counts for this key), an unbound chord falling back
/// to the bare key's command, a frame without a key handler passing keys.
pub fn press_key(vm: &mut Vm, key: &str) {
    if edit_box_key(vm, key) {
        return;
    }
    let k = vm.str(key);
    let frames = keyboard_frames(vm);
    let mut kept = None;
    let mut reached = Vec::new();
    for id in frames {
        // A frame without a key handler lets keys by (unverified:
        // `KeybindListener` stays keyboard-enabled after it listened, and
        // the client's keys keep working).
        if !has_key_handler(vm, id) {
            continue;
        }
        widgets::run_script(vm, id, "OnKeyDown", &[k]);
        reached.push(id);
        if !propagates(vm, id) {
            kept = Some(id);
            break;
        }
    }
    if let Some(id) = kept {
        widgets::run_script(vm, id, "OnKeyUp", &[k]);
        return;
    }
    let chord = chord(vm, key);
    let command = crate::bindings::action_for(vm, &chord).or_else(|| {
        (chord != key)
            .then(|| crate::bindings::action_for(vm, key))
            .flatten()
    });
    if let Some(command) = &command {
        crate::bindings::run_command(vm, command, "down");
    }
    for id in reached {
        widgets::run_script(vm, id, "OnKeyUp", &[k]);
    }
    if let Some(command) = &command {
        crate::bindings::run_command(vm, command, "up");
    }
}

fn set_focus(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    set_keyboard_focus(vm, Some(id));
    Ok(vec![])
}

fn clear_focus(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if engine(vm).world.input.keyboard_focus == Some(id) {
        set_keyboard_focus(vm, None);
    }
    Ok(vec![])
}

fn has_focus(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        engine(vm).world.input.keyboard_focus == Some(id),
    )])
}

/// `IsMouseOver()`: the frame under the mouse is this one or inside it.
fn is_mouse_over(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let mut at = engine(vm).world.input.mouse_focus;
    while let Some(w) = at {
        if w == id {
            return Ok(vec![Value::Bool(true)]);
        }
        at = engine(vm).world.widgets[w.0].parent;
    }
    Ok(vec![Value::Bool(false)])
}

fn focus_value(vm: &mut Vm, id: Option<WidgetId>) -> Value {
    match id {
        Some(w) => Value::Table(engine(vm).world.widgets[w.0].table),
        None => Value::Nil,
    }
}

/// `GetMouseFoci()`: the frames under the mouse (here, one or none).
fn get_mouse_foci(vm: &mut Vm, _c: NativeCall) -> Ret {
    let focus = engine(vm).world.input.mouse_focus;
    let t = vm.new_table();
    if focus.is_some() {
        let v = focus_value(vm, focus);
        let _ = vm.heap.table_mut(t).set(Value::Num(1.0), v);
    }
    Ok(vec![Value::Table(t)])
}

fn get_mouse_focus(vm: &mut Vm, _c: NativeCall) -> Ret {
    let focus = engine(vm).world.input.mouse_focus;
    Ok(vec![focus_value(vm, focus)])
}

fn get_current_keyboard_focus(vm: &mut Vm, _c: NativeCall) -> Ret {
    let focus = engine(vm).world.input.keyboard_focus;
    Ok(vec![focus_value(vm, focus)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetMouseFoci", get_mouse_foci),
        ("GetMouseFocus", get_mouse_focus),
        ("GetCurrentKeyBoardFocus", get_current_keyboard_focus),
        ("IsShiftKeyDown", is_shift_key_down),
        ("IsLeftShiftKeyDown", is_shift_key_down),
        ("IsRightShiftKeyDown", right_key_down),
        ("IsControlKeyDown", is_control_key_down),
        ("IsLeftControlKeyDown", is_control_key_down),
        ("IsRightControlKeyDown", right_key_down),
        ("IsAltKeyDown", is_alt_key_down),
        ("IsLeftAltKeyDown", is_alt_key_down),
        ("IsRightAltKeyDown", right_key_down),
        ("IsModifierKeyDown", is_modifier_key_down),
        ("IsMetaKeyDown", right_key_down),
    ]
}

pub fn methods() -> Vec<(&'static str, MethodFn)> {
    vec![
        ("EditBox:SetFocus", set_focus),
        ("EditBox:ClearFocus", clear_focus),
        ("EditBox:HasFocus", has_focus),
        ("IsMouseOver", is_mouse_over),
    ]
}
