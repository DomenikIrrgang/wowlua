//! The WoW client's engine for the wowlua VM (spec emulator §17.6): the API
//! a flavor's documentation describes, as natives of the VM (decision T198),
//! with behaviour where tests need it; a function without behaviour fails
//! with "`<name>` is not mocked", so gaps are visible.

pub mod api;
pub mod args;
pub mod clock;
pub mod controls;
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
}

impl Engine {
    /// The engine a VM runs, if it has one.
    pub fn of(vm: &Vm) -> Option<&Engine> {
        vm.host.as_ref()?.downcast_ref()
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
    vm.host = Some(Box::new(Engine {
        flavor: flavor.to_string(),
        api,
        behaviours: clock::behaviours().into_iter().collect(),
        methods: widgets::behaviours().into_iter().collect(),
        world: widgets::World::default(),
        timers: clock::Timers::default(),
    }));
    // Undocumented C functions with behaviour.
    let create_frame = vm.native("CreateFrame", widgets::create_frame);
    vm.set_global("CreateFrame", create_frame);
    // The frames the client starts with.
    let ui = widgets::create(vm, "Frame", Some("UIParent".into()), None);
    if let Some(id) = widgets::widget_of(vm, ui) {
        let w = &mut engine_mut(vm).world.widgets[id.0];
        w.width = widgets::SCREEN.0;
        w.height = widgets::SCREEN.1;
    }
    widgets::create(vm, "Frame", Some("WorldFrame".into()), None);
    Ok(())
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
    args::check(vm, api, f, &c)?;
    match behaviour {
        Some(behaviour) => behaviour(vm, c),
        None => Err(vm.error(&format!("{} is not mocked", f.name))),
    }
}
