//! What a button tells of its states: `Get…Texture()` (normal, pushed,
//! disabled, highlight, a check button's checked and disabled checked) the
//! state's texture region as its setters left it (`widgets.rs`), nil
//! before any; `GetButtonState()` `"DISABLED"` while disabled, else what
//! `SetButtonState` set, `"NORMAL"` at first. The state textures show as
//! the state does ([`refresh_states`]).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::widgets::{self, MethodFn, WidgetId};
use crate::{engine, engine_mut};

/// The stored region of a state (`NormalTexture`), else nil.
fn get_texture(vm: &mut Vm, id: WidgetId, state: &str) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let region = widgets::stored_args(vm, props, &format!("{state}Texture"))
        .and_then(|v| v.first().copied())
        .filter(|v| widgets::widget_of(vm, *v).is_some());
    Ok(vec![region.unwrap_or(Value::Nil)])
}

/// `GetButtonState()`.
fn get_button_state(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    if !engine(vm).world.widgets[id.0].enabled {
        return Ok(vec![vm.str("DISABLED")]);
    }
    let props = engine(vm).world.widgets[id.0].props;
    let stored = widgets::stored_args(vm, props, "ButtonState")
        .and_then(|v| v.first().copied())
        .filter(|v| matches!(v, Value::Str(_)));
    Ok(vec![stored.unwrap_or_else(|| vm.str("NORMAL"))])
}

/// The state textures shown as the button's state has them (SuuUI probe
/// trees: of 380 state textures of enabled, unchecked buttons and check
/// buttons in the character window, every normal and highlight texture
/// shown, every pushed, disabled, checked and disabled checked one hidden;
/// the highlight is drawn on mouse-over by its layer, HIGHLIGHT, and stays
/// shown). Unverified, no disabled, pushed or checked button recorded: a
/// disabled button shows its disabled texture in place of its normal one,
/// a pushed one its pushed texture, a checked one its checked texture, or
/// its disabled checked one while disabled.
pub fn refresh_states(vm: &mut Vm, id: WidgetId) {
    let (enabled, props) = {
        let w = &engine(vm).world.widgets[id.0];
        (w.enabled, w.props)
    };
    let texture = |vm: &mut Vm, state: &str| {
        widgets::stored_args(vm, props, &format!("{state}Texture"))
            .and_then(|v| v.first().copied())
            .and_then(|v| widgets::widget_of(vm, v))
    };
    let [normal, pushed, disabled, checked, disabled_checked] =
        ["Normal", "Pushed", "Disabled", "Checked", "DisabledChecked"].map(|s| texture(vm, s));
    if [normal, pushed, disabled, checked, disabled_checked]
        .iter()
        .all(Option::is_none)
    {
        return;
    }
    let is_pushed = enabled
        && widgets::stored_args(vm, props, "ButtonState")
            .and_then(|v| v.first().copied())
            .is_some_and(|v| match v {
                Value::Str(s) => vm.str_bytes(s).eq_ignore_ascii_case(b"PUSHED"),
                _ => false,
            });
    let is_checked = widgets::stored_args(vm, props, "Checked")
        .and_then(|v| v.first().copied())
        .is_some_and(|v| v.truthy());
    let normal_shown = if enabled {
        !is_pushed || pushed.is_none()
    } else {
        disabled.is_none()
    };
    let shown = [
        (normal, normal_shown),
        (pushed, is_pushed),
        (disabled, !enabled),
        (
            checked,
            is_checked && (enabled || disabled_checked.is_none()),
        ),
        (disabled_checked, is_checked && !enabled),
    ];
    for (t, on) in shown {
        if let Some(t) = t {
            engine_mut(vm).world.widgets[t.0].shown = on;
        }
    }
}

/// The layer a state's texture is drawn in (SuuUI probe trees: XML's
/// `<HighlightTexture>` in HIGHLIGHT, `<CheckedTexture>` and
/// `<DisabledCheckedTexture>` in OVERLAY, the others in ARTWORK; a texture
/// a setter makes from a file taken to be alike, unverified).
pub fn state_layer(state: &str) -> &'static str {
    match state {
        "Highlight" => "HIGHLIGHT",
        "Checked" | "DisabledChecked" => "OVERLAY",
        _ => "ARTWORK",
    }
}

/// `SetChecked(checked)`: kept as a boolean; the checked textures follow.
fn set_checked(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    widgets::store(vm, id, "Checked", &[Value::Bool(c.arg(2).truthy())]);
    refresh_states(vm, id);
    Ok(vec![])
}

/// `SetButtonState(state [, lock])`: kept; the state textures follow.
fn set_button_state(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let args: Vec<Value> = c.args.iter().skip(1).copied().collect();
    widgets::store(vm, id, "ButtonState", &args);
    refresh_states(vm, id);
    Ok(vec![])
}

macro_rules! state_getters {
    ($($state:literal => $get:ident;)*) => {
        $(
            fn $get(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
                get_texture(vm, id, $state)
            }
        )*
        pub fn methods() -> Vec<(&'static str, MethodFn)> {
            let mut out: Vec<(&'static str, MethodFn)> = vec![
                ("GetButtonState", get_button_state),
                ("SetButtonState", set_button_state),
                ("SetChecked", set_checked),
            ];
            $(out.push((concat!("Get", $state, "Texture"), $get));)*
            out
        }
    };
}

state_getters! {
    "Normal" => get_normal;
    "Pushed" => get_pushed;
    "Disabled" => get_disabled;
    "Highlight" => get_highlight;
    "Checked" => get_checked;
    "DisabledChecked" => get_disabled_checked;
}
