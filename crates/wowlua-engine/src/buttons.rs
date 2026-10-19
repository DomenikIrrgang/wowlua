//! What a button tells of its states: `Get…Texture()` (normal, pushed,
//! disabled, highlight, a check button's checked and disabled checked) the
//! state's texture region as its setters left it (`widgets.rs`), nil
//! before any; `GetButtonState()` `"DISABLED"` while disabled, else what
//! `SetButtonState` set, `"NORMAL"` at first.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::engine;
use crate::widgets::{self, MethodFn, WidgetId};

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

macro_rules! state_getters {
    ($($state:literal => $get:ident;)*) => {
        $(
            fn $get(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
                get_texture(vm, id, $state)
            }
        )*
        pub fn methods() -> Vec<(&'static str, MethodFn)> {
            let mut out: Vec<(&'static str, MethodFn)> = vec![("GetButtonState", get_button_state)];
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
