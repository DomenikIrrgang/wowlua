//! A button's state textures and state. `SetNormalTexture(asset)` (and
//! pushed, disabled, highlight, a check button's checked and disabled
//! checked) gives the button a texture region of that state: the one it
//! has, else a new one filling the button; a texture region given is
//! taken as it is (Blizzard's XML `<NormalTexture>` hands one over).
//! `Set…Atlas(atlas)` the same with an atlas. `Get…Texture()`: the region,
//! nil before any. `GetButtonState()`: `"DISABLED"` while disabled, else
//! what `SetButtonState` set, `"NORMAL"` at first. Unverified: the
//! regions' draw layers beyond the highlight's.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::engine;
use crate::widgets::{self, MethodFn, WidgetId};

/// The stored region of a state (`NormalTexture`), if any.
fn region_of(vm: &mut Vm, id: WidgetId, state: &str) -> Option<Value> {
    let props = engine(vm).world.widgets[id.0].props;
    let stored = widgets::stored_args(vm, props, &format!("{state}Texture"))?;
    stored
        .first()
        .copied()
        .filter(|v| widgets::widget_of(vm, *v).is_some())
}

/// Calls `method` on a widget's table.
fn call(vm: &mut Vm, obj: Value, method: &str, args: &[Value]) -> Result<(), wowlua_vm::LuaError> {
    let key = vm.str(method);
    let f = vm.index(obj, key, None)?;
    let mut all = vec![obj];
    all.extend_from_slice(args);
    vm.call(f, &all)?;
    Ok(())
}

/// The state's region, made when the button has none.
fn ensure_region(vm: &mut Vm, id: WidgetId, state: &str) -> Result<Value, wowlua_vm::LuaError> {
    if let Some(r) = region_of(vm, id, state) {
        return Ok(r);
    }
    let region = widgets::create(vm, "Texture", None, Some(id));
    let button = Value::Table(engine(vm).world.widgets[id.0].table);
    call(vm, region, "SetAllPoints", &[button])?;
    if state == "Highlight" {
        let layer = vm.str("HIGHLIGHT");
        call(vm, region, "SetDrawLayer", &[layer])?;
    }
    widgets::store(vm, id, &format!("{state}Texture"), &[region]);
    Ok(region)
}

/// `Set<State>Texture(asset | region | nil [, blendMode])`.
fn set_texture(vm: &mut Vm, id: WidgetId, c: &NativeCall, state: &str) -> Ret {
    let arg = c.arg(2);
    if arg.is_nil() {
        widgets::store(vm, id, &format!("{state}Texture"), &[]);
        return Ok(vec![]);
    }
    if widgets::widget_of(vm, arg).is_some() {
        widgets::store(vm, id, &format!("{state}Texture"), &[arg]);
        return Ok(vec![]);
    }
    let region = ensure_region(vm, id, state)?;
    call(vm, region, "SetTexture", &[arg])?;
    if !c.arg(3).is_nil() {
        call(vm, region, "SetBlendMode", &[c.arg(3)])?;
    }
    Ok(vec![])
}

/// `Set<State>Atlas(atlas)`.
fn set_atlas(vm: &mut Vm, id: WidgetId, c: &NativeCall, state: &str) -> Ret {
    let region = ensure_region(vm, id, state)?;
    call(vm, region, "SetAtlas", &[c.arg(2)])?;
    Ok(vec![])
}

fn get_texture(vm: &mut Vm, id: WidgetId, state: &str) -> Ret {
    Ok(vec![region_of(vm, id, state).unwrap_or(Value::Nil)])
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

macro_rules! state_methods {
    ($($state:literal => $set:ident, $atlas:ident, $get:ident;)*) => {
        $(
            fn $set(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
                set_texture(vm, id, &c, $state)
            }
            fn $atlas(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
                set_atlas(vm, id, &c, $state)
            }
            fn $get(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
                get_texture(vm, id, $state)
            }
        )*
        pub fn methods() -> Vec<(&'static str, MethodFn)> {
            let mut out: Vec<(&'static str, MethodFn)> = vec![("GetButtonState", get_button_state)];
            $(
                out.push((concat!("Set", $state, "Texture"), $set));
                out.push((concat!("Set", $state, "Atlas"), $atlas));
                out.push((concat!("Get", $state, "Texture"), $get));
            )*
            out
        }
    };
}

state_methods! {
    "Normal" => set_normal, atlas_normal, get_normal;
    "Pushed" => set_pushed, atlas_pushed, get_pushed;
    "Disabled" => set_disabled, atlas_disabled, get_disabled;
    "Highlight" => set_highlight, atlas_highlight, get_highlight;
    "Checked" => set_checked, atlas_checked, get_checked;
    "DisabledChecked" => set_disabled_checked, atlas_disabled_checked, get_disabled_checked;
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_state_has_its_three_methods() {
        // Normal, pushed, disabled, highlight, checked, disabled checked.
        assert_eq!(super::methods().len(), 1 + 3 * 6);
    }
}
