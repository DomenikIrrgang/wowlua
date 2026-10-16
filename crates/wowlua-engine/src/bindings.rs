//! Keybindings and the addons a test loads.
//!
//! Bindings: those an addon's `Bindings.xml` declares (`defineBinding`:
//! its name and the code to run) and the keys bound to them (`SetBinding`);
//! `pressKey(key)` runs the bound binding with "down", then "up"
//! (`runOnUp` ones only), `pressBinding(name)` runs it directly.
//!
//! Addons: `defineAddon(name, { metadata, loadOnDemand, load })` makes an
//! addon the client knows (its TOC metadata), loaded by `C_AddOns.LoadAddOn`
//! when on demand (`load` runs, then `ADDON_LOADED`).
//!
//! Unverified: `GetBinding`'s results beyond the command and keys, and the
//! addons' reasons and security strings.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// A binding from an addon's `Bindings.xml`.
pub struct Binding {
    pub name: String,
    pub header: Option<String>,
    pub action: Value,
    pub run_on_up: bool,
}

/// An addon a test defined.
pub struct Addon {
    pub name: String,
    pub metadata: Vec<(String, String)>,
    pub on_demand: bool,
    pub load: Value,
}

#[derive(Default)]
pub struct Bindings {
    pub bindings: Vec<Binding>,
    /// Key → binding name (`SetBinding`).
    pub keys: Vec<(String, String)>,
    pub addons: Vec<Addon>,
}

fn string(vm: &mut Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

/// `defineBinding(name, action [, { header, runOnUp }])`.
pub fn define_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let action = c.arg(2);
    widgets::keep(vm, action);
    let (header, run_on_up) = match c.arg(3) {
        Value::Table(opts) => {
            let h = field(vm, opts, "header");
            let up = field(vm, opts, "runOnUp").truthy();
            (string(vm, h), up)
        }
        _ => (None, false),
    };
    engine_mut(vm).world.bindings.bindings.push(Binding {
        name,
        header,
        action,
        run_on_up,
    });
    Ok(vec![])
}

fn run_binding(vm: &mut Vm, name: &str, keystate: &str) {
    let found = engine(vm)
        .world
        .bindings
        .bindings
        .iter()
        .find(|b| b.name == name)
        .map(|b| (b.action, b.run_on_up));
    if let Some((action, run_on_up)) = found
        && (keystate == "down" || run_on_up)
    {
        let k = vm.str(keystate);
        vm.call_from_engine(action, &[k]);
    }
}

/// `pressBinding(name [, keystate])`.
pub fn press_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let state = vm
        .opt_str(&c, 2)?
        .map(|s| String::from_utf8_lossy(vm.str_bytes(s)).into_owned())
        .unwrap_or_else(|| "down".into());
    run_binding(vm, &name, &state);
    Ok(vec![])
}

/// `pressKey(key)`: the key's binding, down then up.
pub fn press_key(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 1)?;
    let key = String::from_utf8_lossy(vm.str_bytes(key)).to_uppercase();
    // A focused edit box takes its keys first.
    if crate::input::edit_box_key(vm, &key) {
        return Ok(vec![]);
    }
    let bound = engine(vm)
        .world
        .bindings
        .keys
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, b)| b.clone());
    if let Some(b) = bound {
        run_binding(vm, &b, "down");
        run_binding(vm, &b, "up");
    }
    Ok(vec![])
}

/// `SetBinding(key [, command])`: true.
fn set_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 1)?;
    let key = String::from_utf8_lossy(vm.str_bytes(key)).to_uppercase();
    let command = string(vm, c.arg(2));
    let keys = &mut engine_mut(vm).world.bindings.keys;
    keys.retain(|(k, _)| *k != key);
    if let Some(command) = command {
        keys.push((key, command));
    }
    Ok(vec![Value::Bool(true)])
}

fn get_binding_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 1)?;
    let key = String::from_utf8_lossy(vm.str_bytes(key)).to_uppercase();
    let action = engine(vm)
        .world
        .bindings
        .keys
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, b)| b.clone())
        .unwrap_or_default();
    Ok(vec![vm.str(&action)])
}

fn keys_of(vm: &Vm, command: &str) -> Vec<String> {
    engine(vm)
        .world
        .bindings
        .keys
        .iter()
        .filter(|(_, b)| b == command)
        .map(|(k, _)| k.clone())
        .collect()
}

fn get_binding_key(vm: &mut Vm, c: NativeCall) -> Ret {
    let command = vm.check_str(&c, 1)?;
    let command = String::from_utf8_lossy(vm.str_bytes(command)).into_owned();
    let keys = keys_of(vm, &command);
    Ok(keys.iter().map(|k| vm.str(k)).collect())
}

fn get_num_bindings(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        engine(vm).world.bindings.bindings.len() as f64
    )])
}

/// `GetBinding(index)`: the command, its category (header), its keys.
fn get_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = vm.check_int(&c, 1)?;
    let found = usize::try_from(i - 1)
        .ok()
        .and_then(|i| engine(vm).world.bindings.bindings.get(i))
        .map(|b| (b.name.clone(), b.header.clone()));
    let Some((name, header)) = found else {
        return Ok(vec![]);
    };
    let mut out = vec![vm.str(&name), header.map_or(Value::Nil, |h| vm.str(&h))];
    for k in keys_of(vm, &name) {
        out.push(vm.str(&k));
    }
    Ok(out)
}

// ----------------------------------------------------------------- addons

/// `defineAddon(name, { metadata = { Title = …, Version = … }, loadOnDemand,
/// load })`.
pub fn define_addon(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let mut metadata = Vec::new();
    let (mut on_demand, mut load) = (false, Value::Nil);
    if let Value::Table(opts) = c.arg(2) {
        if let Value::Table(m) = field(vm, opts, "metadata") {
            let mut k = Value::Nil;
            while let Some((key, v)) = vm.heap.table(m).next(k).ok().flatten() {
                k = key;
                if let (Some(key), Some(v)) = (string(vm, key), string(vm, v)) {
                    metadata.push((key, v));
                }
            }
        }
        on_demand = field(vm, opts, "loadOnDemand").truthy();
        load = field(vm, opts, "load");
        widgets::keep(vm, load);
    }
    engine_mut(vm).world.bindings.addons.push(Addon {
        name,
        metadata,
        on_demand,
        load,
    });
    Ok(vec![])
}

/// The addon an `uiAddon` argument names: its name or index.
fn addon_index(vm: &mut Vm, v: Value) -> Option<usize> {
    let addons = &engine(vm).world.bindings.addons;
    match vm.hidden(v) {
        Value::Num(n) => {
            let i = n as usize;
            (i >= 1 && i <= addons.len()).then(|| i - 1)
        }
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            engine(vm)
                .world
                .bindings
                .addons
                .iter()
                .position(|a| a.name.eq_ignore_ascii_case(&name))
        }
        _ => None,
    }
}

/// `GetAddOnMetadata(name, field)`: the TOC's field, any case.
fn get_addon_metadata(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = addon_index(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let field = string(vm, c.arg(2)).unwrap_or_default();
    let value = engine(vm).world.bindings.addons[i]
        .metadata
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&field))
        .map(|(_, v)| v.clone());
    Ok(vec![value.map_or(Value::Nil, |v| vm.str(&v))])
}

fn get_num_addons(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        engine(vm).world.bindings.addons.len() as f64
    )])
}

/// `GetAddOnInfo(name)`: name, title, notes, loadable, reason, security.
fn get_addon_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = addon_index(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let a = &engine(vm).world.bindings.addons[i];
    let meta = |k: &str| {
        a.metadata
            .iter()
            .find(|(m, _)| m.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.clone())
    };
    let (name, title, notes) = (a.name.clone(), meta("Title"), meta("Notes"));
    let title = vm.str(&title.unwrap_or_else(|| name.clone()));
    Ok(vec![
        vm.str(&name),
        title,
        notes.map_or(Value::Nil, |n| vm.str(&n)),
        Value::Bool(true),
        Value::Nil,
        vm.str("INSECURE"),
    ])
}

fn is_addon_load_on_demand(vm: &mut Vm, c: NativeCall) -> Ret {
    let lod =
        addon_index(vm, c.arg(1)).is_some_and(|i| engine(vm).world.bindings.addons[i].on_demand);
    Ok(vec![Value::Bool(lod)])
}

/// Loads a test-defined addon on demand: its `load`, then `ADDON_LOADED`.
/// `None` for a name the test did not define.
pub fn load_defined(vm: &mut Vm, name: &str) -> Option<bool> {
    let i = engine(vm)
        .world
        .bindings
        .addons
        .iter()
        .position(|a| a.name.eq_ignore_ascii_case(name))?;
    let lower = name.to_lowercase();
    if engine(vm).world.loaded_addons.contains(&lower) {
        return Some(true);
    }
    let (load, real) = {
        let a = &engine(vm).world.bindings.addons[i];
        (a.load, a.name.clone())
    };
    if !load.is_nil() {
        vm.call_from_engine(load, &[]);
    }
    let n = vm.str(&real);
    widgets::fire_event(vm, "ADDON_LOADED", &[n, Value::Bool(false)]);
    Some(true)
}

/// The functions with behaviour here.
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("SetBinding", set_binding),
        ("GetBindingAction", get_binding_action),
        ("GetBindingKey", get_binding_key),
        ("GetNumBindings", get_num_bindings),
        ("GetBinding", get_binding),
        ("C_AddOns.GetAddOnMetadata", get_addon_metadata),
        ("C_AddOns.GetNumAddOns", get_num_addons),
        ("C_AddOns.GetAddOnInfo", get_addon_info),
        ("C_AddOns.IsAddOnLoadOnDemand", is_addon_load_on_demand),
    ]
}
