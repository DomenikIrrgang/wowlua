//! The WoW client's engine for the wowlua VM (spec emulator §17.6): the API
//! a flavor's documentation describes, as natives of the VM (decision T198),
//! with behaviour where tests need it; a function without behaviour fails
//! with "`<name>` is not mocked", so gaps are visible.

pub mod animations;
pub mod api;
pub mod args;
pub mod auras;
pub mod bindings;
pub mod chat;
pub mod clock;
pub mod controls;
pub mod curves;
pub mod facts;
pub mod framescript;
pub mod gamedata;
pub mod protection;
pub mod ui;
pub mod units;
pub mod widgetdata;
pub mod widgets;

use std::collections::HashMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

/// The engine's state, kept in the VM's host slot.
pub struct Engine {
    pub flavor: String,
    pub api: &'static api::Api,
    /// Behaviour by function name (`C_Timer.After`).
    behaviours: HashMap<&'static str, NativeFn>,
    /// Widget methods' behaviour by name (`Show`), or by class and name
    /// (`Button:Click`) where a class differs.
    methods: HashMap<&'static str, widgets::MethodFn>,
    pub world: widgets::World,
    pub timers: clock::Timers,
    pub units: units::Units,
    /// Animation groups playing.
    pub animations: animations::Animations,
    /// Protected actions refused so far.
    pub refusals: Vec<protection::Refusal>,
    /// Functions without behaviour that secure code called (they returned
    /// their documented types' empty values), by name.
    pub defaulted: std::collections::BTreeSet<&'static str>,
    /// Tables Blizzard's code alone sees while it loads.
    pub secure_tables: Vec<SecureTable>,
}

impl Engine {
    /// The engine a VM runs, if it has one.
    pub fn of(vm: &Vm) -> Option<&Engine> {
        vm.host.as_ref()?.downcast_ref()
    }

    /// An API function's behaviour, by name (`C_Timer.After`).
    pub fn behaviour(&self, name: &str) -> Option<NativeFn> {
        self.behaviours.get(name).copied()
    }

    /// A widget method's behaviour, by its full name or its name.
    pub fn method_behaviour(&self, full: &str, short: &str) -> Option<widgets::MethodFn> {
        self.methods
            .get(full)
            .or_else(|| self.methods.get(short))
            .copied()
    }
}

/// The engine of a VM made with [`install`].
pub fn engine(vm: &Vm) -> &Engine {
    Engine::of(vm).expect("the engine is installed")
}

pub fn engine_mut(vm: &mut Vm) -> &mut Engine {
    vm.host
        .as_mut()
        .and_then(|h| h.downcast_mut())
        .expect("the engine is installed")
}

/// Installs a flavor's engine into a VM made with `Vm::wow`: every
/// documented global and namespaced function (those the VM's runtime does
/// not already define), and the `Enum` and `Constants` tables.
pub fn install(vm: &mut Vm, flavor: &str) -> Result<(), String> {
    let api = api::api(flavor).ok_or_else(|| format!("no API documentation for `{flavor}`"))?;
    for (i, f) in api.functions.iter().enumerate() {
        let index = Value::Num(i as f64);
        match f.namespace {
            None => {
                if vm.get_global(f.name).is_nil() {
                    let native = vm.native_with(f.short, api_function, vec![index]);
                    vm.set_global(f.name, native);
                }
            }
            Some(ns) => {
                let table = namespace(vm, ns);
                let key = vm.heap.intern(f.short.as_bytes());
                if vm.heap.table(table).get_str(key).is_nil() {
                    let native = vm.native_with(f.short, api_function, vec![index]);
                    vm.heap.table_mut(table).set_str(key, native);
                }
            }
        }
    }
    let enums = namespace(vm, "Enum");
    for (name, fields) in &api.enums {
        let t = vm.new_table();
        for (field, value) in fields {
            let k = vm.heap.intern(field.as_bytes());
            vm.heap.table_mut(t).set_str(k, Value::Num(*value));
        }
        let k = vm.heap.intern(name.as_bytes());
        vm.heap.table_mut(enums).set_str(k, Value::Table(t));
    }
    for (name, [n, min, max]) in &api.enum_meta {
        let t = vm.new_table();
        for (field, value) in [("NumValues", n), ("MinValue", min), ("MaxValue", max)] {
            let k = vm.heap.intern(field.as_bytes());
            vm.heap.table_mut(t).set_str(k, Value::Num(*value));
        }
        let k = vm.heap.intern(name.as_bytes());
        vm.heap.table_mut(enums).set_str(k, Value::Table(t));
    }
    let constants = namespace(vm, "Constants");
    for (name, values) in &api.constants {
        let t = vm.new_table();
        for (field, value) in values {
            let v = match value {
                serde_json::Value::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
                serde_json::Value::String(s) => vm.str(s),
                serde_json::Value::Bool(b) => Value::Bool(*b),
                _ => continue,
            };
            let k = vm.heap.intern(field.as_bytes());
            vm.heap.table_mut(t).set_str(k, v);
        }
        let k = vm.heap.intern(name.as_bytes());
        vm.heap.table_mut(constants).set_str(k, Value::Table(t));
    }
    // The number globals the client defines (`LE_*`).
    if flavor == "camelot" {
        for line in include_str!("../data/camelot-globals.txt").lines() {
            if let Some((name, n)) = line.split_once('\t')
                && let Ok(n) = n.parse::<f64>()
                && vm.get_global(name).is_nil()
            {
                vm.set_global(name, Value::Num(n));
            }
        }
    }
    let secure_tables = if flavor == "camelot" {
        recorded_tables(vm, include_str!("../data/camelot-enums.txt"))
    } else {
        Vec::new()
    };
    vm.host = Some(Box::new(Engine {
        flavor: flavor.to_string(),
        api,
        behaviours: clock::behaviours()
            .into_iter()
            .chain(units::behaviours())
            .chain(units::more_behaviours())
            .chain(curves::behaviours())
            .chain(widgets::functions())
            .chain(auras::behaviours())
            .chain(gamedata::behaviours())
            .chain(chat::behaviours())
            .chain(bindings::behaviours())
            .chain(facts::behaviours())
            .chain(framescript::functions())
            .collect(),
        methods: widgets::behaviours()
            .into_iter()
            .chain(curves::methods())
            .chain(framescript::methods())
            .chain(animations::methods())
            .collect(),
        world: widgets::World::default(),
        timers: clock::Timers::default(),
        units: units::Units::default(),
        refusals: Vec::new(),
        animations: Default::default(),
        defaulted: Default::default(),
        secure_tables,
    }));
    // Protected functions the documentation lacks.
    for &name in protection::PROTECTED {
        let (ns, short) = match name.split_once('.') {
            Some((ns, short)) => (Some(ns), short),
            None => (None, name),
        };
        let upval = vm.str(name);
        match ns {
            None if vm.get_global(short).is_nil() => {
                let native = vm.native_with(short, protection::protected_global, vec![upval]);
                vm.set_global(short, native);
            }
            Some(ns) => {
                let t = namespace(vm, ns);
                let key = vm.heap.intern(short.as_bytes());
                if vm.heap.table(t).get_str(key).is_nil() {
                    let native = vm.native_with(short, protection::protected_global, vec![upval]);
                    vm.heap.table_mut(t).set_str(key, native);
                }
            }
            None => {}
        }
    }
    // Functions with behaviour the documentation lacks (`IsInGroup`): the
    // client has them (camelot's inventory; other flavors unverified).
    let names: Vec<(&'static str, NativeFn)> = engine(vm)
        .behaviours
        .iter()
        .map(|(&n, &f)| (n, f))
        .collect();
    for (name, f) in names {
        let (ns, short) = match name.split_once('.') {
            Some((ns, short)) => (Some(ns), short),
            None => (None, name),
        };
        let exists = match ns {
            None => !vm.get_global(short).is_nil(),
            Some(ns) => {
                let t = namespace(vm, ns);
                let key = vm.heap.intern(short.as_bytes());
                !vm.heap.table(t).get_str(key).is_nil()
            }
        };
        if !exists {
            let native = vm.native(short, f);
            match ns {
                None => vm.set_global(short, native),
                Some(ns) => {
                    let t = namespace(vm, ns);
                    let key = vm.heap.intern(short.as_bytes());
                    vm.heap.table_mut(t).set_str(key, native);
                }
            }
        }
    }
    // The client's recorded answers about itself: documented functions get
    // them as behaviour, the others as natives of their own.
    for name in facts::recorded_names(flavor) {
        let (ns, short) = match name.split_once('.') {
            Some((ns, short)) => (Some(ns), short),
            None => (None, name),
        };
        let table = match ns {
            None => vm.globals,
            Some(ns) => namespace(vm, ns),
        };
        let key = vm.heap.intern(short.as_bytes());
        if !vm.heap.table(table).get_str(key).is_nil() {
            engine_mut(vm)
                .behaviours
                .entry(name)
                .or_insert(facts::recorded_call);
            continue;
        }
        let upval = vm.str(name);
        let native = vm.native_with(short, facts::recorded_call, vec![upval]);
        vm.heap.table_mut(table).set_str(key, native);
    }
    // The client's undocumented C functions (camelot's inventory): they
    // exist, without behaviour.
    if flavor == "camelot" {
        for line in include_str!("../data/camelot-functions.txt").lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let (ns, short) = match line.split_once('.') {
                Some((ns, short)) => (Some(ns), short),
                None => (None, line),
            };
            let name = vm.str(line);
            match ns {
                None => {
                    if vm.get_global(short).is_nil() {
                        let native = vm.native_with(short, undocumented, vec![name]);
                        vm.set_global(short, native);
                    }
                }
                Some(ns) => {
                    let t = namespace(vm, ns);
                    let key = vm.heap.intern(short.as_bytes());
                    if vm.heap.table(t).get_str(key).is_nil() {
                        let native = vm.native_with(short, undocumented, vec![name]);
                        vm.heap.table_mut(t).set_str(key, native);
                    }
                }
            }
        }
    }
    units::install(vm);
    // The CVars' defaults (camelot's, recorded by `/stdtest engine`; other
    // flavors: the deprecation fallbacks, which the client has).
    let cvars: Vec<(String, String)> = if flavor == "camelot" {
        include_str!("../data/camelot-cvars.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    } else {
        vec![("loadDeprecationFallbacks".into(), "1".into())]
    };
    engine_mut(vm).world.cvar_defaults = cvars.clone();
    engine_mut(vm).world.cvars.extend(cvars);
    // Undocumented C functions with behaviour.
    let create_frame = vm.native("CreateFrame", widgets::create_frame);
    vm.set_global("CreateFrame", create_frame);
    // The frames the client starts with.
    let ui = widgets::create(vm, "Frame", Some("UIParent".into()), None);
    if let Some(id) = widgets::widget_of(vm, ui) {
        let w = &mut engine_mut(vm).world.widgets[id.0];
        w.width = widgets::SCREEN.0;
        w.height = widgets::SCREEN.1;
        // Anchored to the screen at both corners, and protected
        // (`/stdtest engine`).
        w.protected = true;
        w.explicitly_protected = true;
        for p in ["TOPLEFT", "BOTTOMRIGHT"] {
            w.points.push(widgets::Point {
                point: p.into(),
                relative: None,
                relative_point: p.into(),
                x: 0.0,
                y: 0.0,
            });
        }
    }
    widgets::create(vm, "Frame", Some("WorldFrame".into()), None);
    Ok(())
}

/// `Enum` and `Constants` as the client has them where its documentation
/// differs (`data/<flavor>-enums.txt`, from `/stdtest engine`): a line
/// `Enum.X.Field<TAB>number 5` sets a field, `…<TAB>-` removes a field or a
/// whole table, `…<TAB>secure` takes a table out for Blizzard's code
/// alone (returned: put back while it loads, `ui::load`).
fn recorded_tables(vm: &mut Vm, data: &str) -> Vec<SecureTable> {
    let mut secure = Vec::new();
    for line in data.lines().filter(|l| !l.starts_with('#')) {
        let Some((name, value)) = line.split_once('\t') else {
            continue;
        };
        let mut parts = name.splitn(3, '.');
        let (Some(root), Some(table)) = (parts.next(), parts.next()) else {
            continue;
        };
        let root = namespace(vm, root);
        let tkey = vm.heap.intern(table.as_bytes());
        let Some(field) = parts.next() else {
            if value == "secure" {
                let table = match vm.heap.table(root).get_str(tkey) {
                    t @ Value::Table(_) => t,
                    _ => stand_in_enum(vm),
                };
                widgets::keep(vm, table);
                widgets::keep(vm, Value::Str(tkey));
                secure.push(SecureTable {
                    root,
                    name: tkey,
                    table,
                });
            }
            if value == "-" || value == "secure" {
                vm.heap.table_mut(root).set_str(tkey, Value::Nil);
            }
            continue;
        };
        let t = match vm.heap.table(root).get_str(tkey) {
            Value::Table(t) => t,
            _ if value == "-" => continue,
            _ => {
                let t = vm.new_table();
                vm.heap.table_mut(root).set_str(tkey, Value::Table(t));
                t
            }
        };
        let v = match value.split_once(' ') {
            Some(("number", n)) => n.parse().map_or(Value::Nil, Value::Num),
            Some(("string", s)) => vm.str(s),
            Some(("boolean", b)) => Value::Bool(b == "true"),
            _ => Value::Nil,
        };
        let fkey = vm.heap.intern(field.as_bytes());
        vm.heap.table_mut(t).set_str(fkey, v);
    }
    secure
}

/// A table only Blizzard's code sees while it loads (`Enum.StoreError`).
#[derive(Clone, Copy)]
pub struct SecureTable {
    pub root: wowlua_vm::TableRef,
    pub name: wowlua_vm::StrRef,
    pub table: Value,
}

/// An enumeration whose values are unknown: each field read gets the next
/// number (0, 1, …) and keeps it.
fn stand_in_enum(vm: &mut Vm) -> Value {
    fn next_value(vm: &mut Vm, c: NativeCall) -> Ret {
        let Value::Table(t) = c.arg(1) else {
            return Ok(vec![Value::Nil]);
        };
        let n = vm.heap.table(t).entries().count() as f64;
        let _ = vm.heap.table_mut(t).set(c.arg(2), Value::Num(n));
        Ok(vec![Value::Num(n)])
    }
    let (t, meta) = (vm.new_table(), vm.new_table());
    let index = vm.native("__index", next_value);
    let k = vm.names.index;
    vm.heap.table_mut(meta).set_str(k, index);
    vm.heap.table_mut(t).meta = Some(meta);
    Value::Table(t)
}

/// The global table `name`, made if missing.
fn namespace(vm: &mut Vm, name: &str) -> wowlua_vm::TableRef {
    if let Value::Table(t) = vm.get_global(name) {
        return t;
    }
    let t = vm.new_table();
    vm.set_global(name, Value::Table(t));
    t
}

/// A function's documented results as empty values: nil where nilable,
/// false, 0, "", an empty table (unverified stand-ins).
pub fn defaults(vm: &mut Vm, api: &api::Api, doc: &serde_json::Value) -> Vec<Value> {
    let returns = doc
        .get("Returns")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if doc
        .get("MayReturnNothing")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return vec![];
    }
    returns
        .iter()
        .map(|r| {
            let nilable = r.get("Nilable").and_then(serde_json::Value::as_bool) == Some(true);
            let ty = r
                .get("Type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if nilable {
                return Value::Nil;
            }
            let ty = api.aliases.get(ty).map(String::as_str).unwrap_or(ty);
            match ty {
                "bool" | "boolean" => Value::Bool(false),
                "string" | "cstring" => vm.str(""),
                "table" => Value::Table(vm.new_table()),
                _ if api.structures.contains(ty) => Value::Table(vm.new_table()),
                _ if api.enum_names.contains(ty) => api
                    .enums
                    .iter()
                    .find(|(n, _)| *n == ty)
                    .and_then(|(_, f)| f.first().map(|(_, v)| Value::Num(*v)))
                    .unwrap_or(Value::Num(0.0)),
                _ if ty.contains("string") => vm.str(""),
                // Numbers by their alias; other types are objects.
                "number" | "luaIndex" | "uiUnit" | "fileID" | "time_t" | "BigInteger"
                | "BigUInteger" | "WOWMONEY" | "SingleColorValue" | "uiFontHeight" => {
                    Value::Num(0.0)
                }
                // An object of its mixin (`ColorMixin`), when Blizzard's code
                // defined it.
                _ if r.get("Mixin").is_some() => {
                    let t = vm.new_table();
                    let mixin = r.get("Mixin").and_then(serde_json::Value::as_str);
                    if let Some(Value::Table(m)) = mixin.map(|m| vm.get_global(m)) {
                        let methods: Vec<(Value, Value)> = vm.heap.table(m).entries().collect();
                        for (k, f) in methods {
                            let _ = vm.heap.table_mut(t).set(k, f);
                        }
                    }
                    Value::Table(t)
                }
                _ if ty.ends_with("ID") || ty.ends_with("Id") || ty.ends_with("Index") => {
                    Value::Num(0.0)
                }
                _ => Value::Table(vm.new_table()),
            }
        })
        .collect()
}

/// An undocumented C function without behaviour: nothing for Blizzard's
/// code, "not mocked" for the addon's.
fn undocumented(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = match c.upvals.first().copied() {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "?".into(),
    };
    if protection::blizzard_caller(vm) {
        return Ok(vec![]);
    }
    Err(vm.error(&format!("{name} is not mocked")))
}

/// Every API function: its behaviour, or the "not mocked" error.
fn api_function(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(i)) = c.upvals.first().copied() else {
        return Err(vm.error("an API function without its index"));
    };
    let Some(engine) = Engine::of(vm) else {
        return Err(vm.error("the engine is not installed"));
    };
    let api = engine.api;
    let f = &api.functions[i as usize];
    let behaviour = engine.behaviours.get(f.name).copied();
    // Refused before the arguments are checked (unverified).
    if protection::is_protected(f.name, Some(&f.doc)) && protection::tainted(vm) {
        let action = protection::action_name(vm);
        protection::refuse(vm, "ADDON_ACTION_FORBIDDEN", action);
        return Ok(defaults(vm, api, &f.doc));
    }
    args::check(vm, api, f, &c)?;
    let Some(behaviour) = behaviour else {
        // Blizzard's code gets empty values; the addon's code an error.
        if protection::blizzard_caller(vm) {
            engine_mut(vm).defaulted.insert(f.name);
            return Ok(defaults(vm, api, &f.doc));
        }
        return Err(vm.error(&format!("{} is not mocked", f.name)));
    };
    // The client's secrecy: results secret as the function's documentation
    // and the restrictions active say; a return marked never secret stays.
    let secret = units::secret_results(vm, &f.doc, &c);
    let results = behaviour(vm, c)?;
    if !secret {
        return Ok(results);
    }
    let returns = f
        .doc
        .get("Returns")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(results
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let never = returns
                .get(i)
                .and_then(|r| r.get("NeverSecret"))
                .and_then(serde_json::Value::as_bool)
                == Some(true);
            if never || v.is_nil() {
                v
            } else {
                vm.secret_wrap(v)
            }
        })
        .collect())
}
