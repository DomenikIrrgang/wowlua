//! The client's C helpers for Lua code (the documentation's `FrameScript`
//! and neighbours): mixins, event callbacks, CVars, build information and
//! the login state, which Blizzard's UI code calls as it loads.
//!
//! Unverified (the documentation's signatures, behaviour inferred from
//! Blizzard's callers): event callbacks run after the frames' `OnEvent`,
//! global ones as `callback(nil, ...)` and a frame's as
//! `callback(frame, ...)`; `CreateSecureDelegate` and
//! `C_FunctionContainers.CreateCallback` return the function; an atlas's
//! information is a stand-in (atlases are game data); a CVar nobody
//! registered reads as nil.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::widgets::{self, WidgetId};
use crate::{engine, engine_mut};

/// An event callback: for an event (and unit), of a frame or global.
pub struct Callback {
    pub event: String,
    pub unit: Option<String>,
    pub owner: Option<WidgetId>,
    pub f: Value,
}

fn copy_fields(vm: &mut Vm, into: Value, from: Value) {
    let (Value::Table(dst), Value::Table(src)) = (into, from) else {
        return;
    };
    let mut k = Value::Nil;
    while let Some((key, v)) = vm.heap.table(src).next(k).ok().flatten() {
        let _ = vm.heap.table_mut(dst).set(key, v);
        k = key;
    }
}

/// `Mixin(object, ...)`: the mixins' fields copied into the object.
fn mixin(vm: &mut Vm, c: NativeCall) -> Ret {
    let object = c.arg(1);
    for m in c.args.iter().skip(1) {
        copy_fields(vm, object, *m);
    }
    Ok(vec![object])
}

/// `CreateFromMixins(...)`: a new table with the mixins' fields.
fn create_from_mixins(vm: &mut Vm, c: NativeCall) -> Ret {
    let object = Value::Table(vm.new_table());
    for m in c.args.iter() {
        copy_fields(vm, object, *m);
    }
    Ok(vec![object])
}

/// `GetCurrentEnvironment()`: the calling function's environment.
fn current_environment(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.caller_env(1))])
}

/// `GetGlobalEnvironment()`: the global table.
fn global_environment(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.globals)])
}

/// `Get<Class>Metatable()`: the metatable widgets of the class share.
fn class_metatable(class: &'static str) -> impl Fn(&mut Vm, NativeCall) -> Ret {
    move |vm: &mut Vm, _c: NativeCall| Ok(vec![Value::Table(widgets::class_meta(vm, class))])
}

macro_rules! metatable_fns {
    ($($name:ident => $class:literal),*) => {
        $(fn $name(vm: &mut Vm, c: NativeCall) -> Ret {
            class_metatable($class)(vm, c)
        })*
    };
}

metatable_fns!(
    frame_metatable => "Frame",
    button_metatable => "Button",
    texture_metatable => "Texture",
    font_string_metatable => "FontString",
    edit_box_metatable => "EditBox"
);

/// `import(path)`: Blizzard's module loader; a stand-in giving an empty
/// module (its semantics are not known).
fn import(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

/// Returns its first argument (a stand-in that keeps the function).
fn same(_vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![c.arg(1)])
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn falsy(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn string_arg(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<String, wowlua_vm::LuaError> {
    let s = vm.check_str(c, n)?;
    Ok(String::from_utf8_lossy(vm.str_bytes(s)).into_owned())
}

fn register(vm: &mut Vm, event: String, unit: Option<String>, owner: Option<WidgetId>, f: Value) {
    widgets::keep(vm, f);
    engine_mut(vm).world.callbacks.push(Callback {
        event,
        unit,
        owner,
        f,
    });
}

fn unregister(vm: &mut Vm, event: &str, owner: Option<WidgetId>, f: Option<Value>) {
    engine_mut(vm).world.callbacks.retain(|cb| {
        let same_f = match (f, cb.f) {
            (None, _) => true,
            (Some(Value::Func(a)), Value::Func(b)) => a == b,
            (Some(Value::Table(a)), Value::Table(b)) => a == b,
            _ => false,
        };
        !(cb.event == event && cb.owner == owner && same_f)
    });
}

fn register_event_callback(vm: &mut Vm, c: NativeCall) -> Ret {
    let event = string_arg(vm, &c, 1)?;
    register(vm, event, None, None, c.arg(2));
    Ok(vec![])
}

fn register_unit_event_callback(vm: &mut Vm, c: NativeCall) -> Ret {
    let event = string_arg(vm, &c, 1)?;
    let unit = string_arg(vm, &c, 3)?;
    register(vm, event, Some(unit), None, c.arg(2));
    Ok(vec![])
}

fn unregister_event_callback(vm: &mut Vm, c: NativeCall) -> Ret {
    let event = string_arg(vm, &c, 1)?;
    unregister(vm, &event, None, Some(c.arg(2)));
    Ok(vec![])
}

fn frame_register_event_callback(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let event = string_arg(vm, &c, 2)?;
    register(vm, event, None, Some(id), c.arg(3));
    Ok(vec![Value::Bool(true)])
}

fn frame_unregister_event_callback(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let event = string_arg(vm, &c, 2)?;
    unregister(vm, &event, Some(id), None);
    Ok(vec![])
}

/// Runs the callbacks registered for `event` (after the frames' handlers).
pub fn run_callbacks(vm: &mut Vm, event: &str, args: &[Value]) {
    let unit = match args.first() {
        Some(Value::Str(s)) => Some(String::from_utf8_lossy(vm.str_bytes(*s)).into_owned()),
        _ => None,
    };
    let due: Vec<(Option<WidgetId>, Value)> = engine(vm)
        .world
        .callbacks
        .iter()
        .filter(|cb| cb.event == event && (cb.unit.is_none() || cb.unit == unit))
        .map(|cb| (cb.owner, cb.f))
        .collect();
    for (owner, f) in due {
        let mut call = vec![owner.map_or(Value::Nil, |o| {
            Value::Table(engine(vm).world.widgets[o.0].table)
        })];
        call.extend_from_slice(args);
        vm.call_from_engine(f, &call);
    }
}

// ----------------------------------------------------------------- CVars

fn cvar_get(vm: &mut Vm, name: &str) -> Option<String> {
    engine(vm)
        .world
        .cvars
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn cvar_set(vm: &mut Vm, name: &str, value: String) {
    let cvars = &mut engine_mut(vm).world.cvars;
    match cvars.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
        Some(slot) => slot.1 = value,
        None => cvars.push((name.to_string(), value)),
    }
}

fn value_text(vm: &mut Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) if n.fract() == 0.0 => format!("{}", n as i64),
        Value::Num(n) => n.to_string(),
        Value::Bool(b) => if b { "1" } else { "0" }.into(),
        _ => String::new(),
    }
}

fn get_cvar(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = string_arg(vm, &c, 1)?;
    Ok(vec![cvar_get(vm, &name).map_or(Value::Nil, |v| vm.str(&v))])
}

fn get_cvar_bool(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = string_arg(vm, &c, 1)?;
    Ok(vec![cvar_get(vm, &name).map_or(Value::Nil, |v| {
        Value::Bool(v != "0" && !v.is_empty())
    })])
}

fn register_cvar(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = string_arg(vm, &c, 1)?;
    if cvar_get(vm, &name).is_none() {
        let v = value_text(vm, c.arg(2));
        cvar_set(vm, &name, v);
    }
    Ok(vec![])
}

fn set_cvar(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = string_arg(vm, &c, 1)?;
    let text = value_text(vm, c.arg(2));
    cvar_set(vm, &name, text.clone());
    let n = vm.str(&name);
    let v = vm.str(&text);
    widgets::fire_event(vm, "CVAR_UPDATE", &[n, v]);
    Ok(vec![Value::Bool(true)])
}

// ---------------------------------------------------------------- others

/// `C_Texture.GetAtlasInfo(atlas)`: a stand-in (64 x 64, whole texture).
fn atlas_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let t = vm.new_table();
    let set = |vm: &mut Vm, k: &str, v: Value| {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, v);
    };
    set(vm, "elementName", Value::Str(name));
    for (k, v) in [
        ("width", 64.0),
        ("height", 64.0),
        ("leftTexCoord", 0.0),
        ("rightTexCoord", 1.0),
        ("topTexCoord", 0.0),
        ("bottomTexCoord", 1.0),
        ("file", 0.0),
    ] {
        set(vm, k, Value::Num(v));
    }
    set(vm, "tilesHorizontally", Value::Bool(false));
    set(vm, "tilesVertically", Value::Bool(false));
    let file = vm.str("Interface\\Buttons\\WHITE8X8");
    set(vm, "filename", file);
    Ok(vec![Value::Table(t)])
}

fn truthy(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `GetBuildInfo()`: the flavor's client version and interface (the build
/// number and date are not known: "0", "").
fn build_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    let flavor = engine(vm).flavor.clone();
    let table = wowlua_flavors::table();
    let client = table.clients.iter().find(|c| c.flavor == flavor);
    let (version, interface) = client.map_or(("0.0.0".to_string(), 0), |c| {
        (c.version.clone(), c.interface)
    });
    let v = vm.str(&version);
    let build = vm.str("0");
    let date = vm.str("");
    let info = vm.str(&format!("{version}.0"));
    Ok(vec![
        v,
        build,
        date,
        Value::Num(f64::from(interface)),
        v,
        info,
    ])
}

/// `C_AddOns.IsAddOnLoaded(name)`: (loaded or loading, loaded), once
/// `ADDON_LOADED` fired for it.
fn is_addon_loaded(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = match vm.hidden(c.arg(1)) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).to_lowercase(),
        _ => return Ok(vec![Value::Bool(false), Value::Bool(false)]),
    };
    let loaded = engine(vm).world.loaded_addons.contains(&name);
    Ok(vec![Value::Bool(loaded), Value::Bool(loaded)])
}

/// `C_AddOns.LoadAddOn(name)`: a Blizzard addon of the loaded UI code
/// loads (true); without it, or for another addon, (false, "MISSING")
/// (unverified reason).
fn load_addon(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = match vm.hidden(c.arg(1)) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    match crate::ui::load_addon(vm, &name) {
        Some(true) => Ok(vec![Value::Bool(true), Value::Nil]),
        _ => Ok(vec![Value::Bool(false), vm.str("MISSING")]),
    }
}

/// `C_LFGList.GetPremadeGroupFinderStyle()`: camelot's group finder is the
/// vanilla style (the client loads Blizzard_GroupFinder_VanillaStyle,
/// `/stdtest engine`); other flavors' is unverified (the enum's first).
fn premade_group_finder_style(vm: &mut Vm, _c: NativeCall) -> Ret {
    let api = engine(vm).api;
    let wanted = if engine(vm).flavor == "camelot" {
        "Vanilla"
    } else {
        ""
    };
    let style = api
        .enums
        .iter()
        .find(|(n, _)| *n == "PremadeGroupFinderStyle")
        .and_then(|(_, fields)| {
            fields
                .iter()
                .find(|(f, _)| f == wanted)
                .or_else(|| fields.first())
                .map(|(_, v)| *v)
        })
        .unwrap_or(0.0);
    Ok(vec![Value::Num(style)])
}

/// `C_LFGInfo.IsGroupFinderEnabled()`: on camelot, yes (the client loads
/// its group finder at login, `/stdtest engine`); elsewhere unverified.
fn group_finder_enabled(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).flavor == "camelot")])
}

fn is_logged_in(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).world.logged_in)])
}

/// `GetCallstackHeight()`: how deep the Lua call stack is.
fn callstack_height(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(vm.call_depth() as f64)])
}

/// The functions with behaviour here.
pub fn functions() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetCurrentEnvironment", current_environment),
        ("GetGlobalEnvironment", global_environment),
        ("SwapToGlobalEnvironment", nothing),
        ("GetFrameMetatable", frame_metatable),
        ("GetButtonMetatable", button_metatable),
        ("GetTextureMetatable", texture_metatable),
        ("GetFontStringMetatable", font_string_metatable),
        ("GetEditBoxMetatable", edit_box_metatable),
        ("IsGMClient", falsy),
        // No modifier is held (the test controls none yet).
        ("IsModifiedClick", falsy),
        ("import", import),
        ("addframetext", nothing),
        ("Mixin", mixin),
        ("CreateFromMixins", create_from_mixins),
        ("CreateSecureDelegate", same),
        ("C_FunctionContainers.CreateCallback", same),
        ("AddSourceLocationExclude", nothing),
        ("GetCallstackHeight", callstack_height),
        ("RegisterEventCallback", register_event_callback),
        ("RegisterUnitEventCallback", register_unit_event_callback),
        ("UnregisterEventCallback", unregister_event_callback),
        ("UnregisterUnitEventCallback", unregister_event_callback),
        ("C_CVar.GetCVar", get_cvar),
        ("C_CVar.GetCVarBool", get_cvar_bool),
        ("C_CVar.GetCVarDefault", get_cvar),
        ("C_CVar.RegisterCVar", register_cvar),
        ("C_CVar.SetCVar", set_cvar),
        ("C_Texture.GetAtlasInfo", atlas_info),
        ("C_Texture.GetAtlasExists", truthy),
        ("C_InputInterfaceStyle.GetCurrentStyle", zero),
        ("C_Glue.IsOnGlueScreen", falsy),
        ("IsMacClient", falsy),
        ("GetBuildInfo", build_info),
        ("IsLoggedIn", is_logged_in),
        ("C_AddOns.IsAddOnLoaded", is_addon_loaded),
        ("C_AddOns.LoadAddOn", load_addon),
        (
            "C_LFGList.GetPremadeGroupFinderStyle",
            premade_group_finder_style,
        ),
        ("C_LFGInfo.IsGroupFinderEnabled", group_finder_enabled),
    ]
}

/// Widget methods with behaviour here.
pub fn methods() -> Vec<(&'static str, widgets::MethodFn)> {
    vec![
        ("RegisterEventCallback", frame_register_event_callback),
        ("UnregisterEventCallback", frame_unregister_event_callback),
    ]
}
