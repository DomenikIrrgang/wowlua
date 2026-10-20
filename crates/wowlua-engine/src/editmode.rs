//! Edit Mode's layouts (`C_EditMode`): the account's saved layouts and the
//! active one. Blizzard's code adds its preset layouts itself
//! (`EditModePresetLayoutManager`); the client gives the saved ones.
//!
//! The login sends them (`EDIT_MODE_LAYOUTS_UPDATED(layoutInfo, false)`,
//! recorded with Blizzard's addons loading). The test's world is a fresh
//! account's: no saved layouts, the default preset active
//! (`GetEditModeDefaultLayout`; unverified: that a fresh account has it).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// The account's layouts: the active one's index, the saved ones (a Lua
/// list of layout infos, as `SaveLayouts` gave them).
#[derive(Default)]
pub struct EditMode {
    pub active: Option<f64>,
    pub saved: Option<Value>,
}

/// A deep copy of a Lua value (the client gives new tables each call).
const COPY: &str = r#"
local function copy(v)
    if type(v) ~= "table" then return v end
    local t = {}
    for k, x in pairs(v) do t[k] = copy(x) end
    return t
end
return copy
"#;

fn copy(vm: &mut Vm, v: Value) -> Value {
    match crate::lua_helper(vm, "EditModeCopy", COPY) {
        Ok(f) => vm
            .call(f, &[v])
            .ok()
            .and_then(|r| r.first().copied())
            .unwrap_or(Value::Nil),
        Err(_) => Value::Nil,
    }
}

/// The default preset's place among the layouts: Blizzard lists its
/// presets first, in `Enum.EditModePresetLayouts` order, so the recorded
/// default (0, Modern) is the first (unverified: a fresh account's).
fn default_layout(vm: &mut Vm) -> f64 {
    crate::recorded::lookup_text(vm, "C_EditMode.GetEditModeDefaultLayout()")
        .and_then(|v| match v.first() {
            Some(Value::Num(n)) => Some(*n),
            _ => None,
        })
        .unwrap_or(0.0)
        + 1.0
}

/// The layout info the client gives: `{ activeLayout, layouts }`.
pub fn layout_info(vm: &mut Vm) -> Value {
    let active = match engine(vm).world.edit_mode.active {
        Some(a) => a,
        None => default_layout(vm),
    };
    let layouts = match engine(vm).world.edit_mode.saved {
        Some(saved) => copy(vm, saved),
        None => Value::Table(vm.new_table()),
    };
    let t = vm.new_table();
    let (k_active, k_layouts) = (vm.str("activeLayout"), vm.str("layouts"));
    let _ = vm.heap.table_mut(t).set(k_active, Value::Num(active));
    let _ = vm.heap.table_mut(t).set(k_layouts, layouts);
    Value::Table(t)
}

fn get_layouts(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![layout_info(vm)])
}

/// `SaveLayouts(layoutInfo)`: its saved layouts and active one kept.
fn save_layouts(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(info) = c.arg(1) else {
        return Ok(vec![]);
    };
    let (k_active, k_layouts) = (vm.str("activeLayout"), vm.str("layouts"));
    let active = vm.heap.table(info).get(k_active);
    let layouts = vm.heap.table(info).get(k_layouts);
    let saved = copy(vm, layouts);
    widgets::keep_as(vm, "editModeLayouts", saved);
    let e = &mut engine_mut(vm).world.edit_mode;
    if let Value::Num(a) = active {
        e.active = Some(a);
    }
    e.saved = Some(saved);
    Ok(vec![])
}

/// `SetActiveLayout(index)`.
fn set_active_layout(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Value::Num(a) = c.arg(1) {
        engine_mut(vm).world.edit_mode.active = Some(a);
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_EditMode.GetLayouts", get_layouts),
        ("C_EditMode.SaveLayouts", save_layouts),
        ("C_EditMode.SetActiveLayout", set_active_layout),
    ]
}
