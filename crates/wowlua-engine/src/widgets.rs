//! Widgets (spec emulator §17.6): frames, textures and font strings as the
//! client makes them. A widget is a Lua table holding its object (a
//! userdata) at `[0]`, whose metatable's `__index` is its class's method
//! table: the documented methods of the class and its bases, as natives.
//! A method runs its behaviour here, stores what a setter without one was
//! given (its getter returns it), or fails as not mocked.
//!
//! Unverified until the probe battery runs (decided by the M8 mock and
//! plausibility): events dispatch in frame creation order; `SetScript`
//! clears the hooks; an unknown event, script or frame type fails with the
//! messages below; templates without Blizzard's UI code only warn.

use std::collections::HashMap;

use wowlua_vm::heap::Userdata;
use wowlua_vm::stdlib::Ret;
use wowlua_vm::{LuaError, NativeCall, TableRef, Value, Vm};

use crate::{Engine, engine, engine_mut};

/// A widget's object payload: its index in [`World::widgets`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidgetId(pub usize);

/// One anchor: `point`, the frame it is relative to, its point, offsets.
#[derive(Clone, Debug)]
pub struct Point {
    pub point: String,
    pub relative: Option<Value>,
    pub relative_point: String,
    pub x: f64,
    pub y: f64,
}

pub struct Widget {
    /// The class whose methods it has (`Frame`, `Button`, `FontString`).
    pub class: String,
    /// The Lua object.
    pub table: TableRef,
    /// Its scripts (`OnEvent` → function) and hooks (`OnEvent` → list), in
    /// Lua tables the registry keeps.
    pub scripts: TableRef,
    pub hooks: TableRef,
    /// Values setters without a behaviour stored, by method name's stem.
    pub props: TableRef,
    pub name: Option<String>,
    pub parent: Option<WidgetId>,
    pub children: Vec<WidgetId>,
    pub shown: bool,
    /// Registered events, with the units of unit events.
    pub events: HashMap<String, Option<Vec<String>>>,
    pub points: Vec<Point>,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub alpha: f64,
    pub text: Option<Value>,
    pub attributes: TableRef,
    pub protected: bool,
}

/// The widgets, and each class's method table and metatable.
#[derive(Default)]
pub struct World {
    pub widgets: Vec<Widget>,
    classes: HashMap<String, TableRef>,
    /// Each method's native, by method index.
    natives: HashMap<usize, Value>,
    /// Warnings for gaps that would make every test fail if they were errors
    /// (templates without Blizzard's UI code).
    pub warnings: Vec<String>,
}

/// UIParent's size in screen units at scale 1.
pub const SCREEN: (f64, f64) = (1024.0, 768.0);

/// Keeps one of the engine's Lua values alive (in a registry table).
pub fn keep(vm: &mut Vm, v: Value) {
    let registry = vm.registry;
    let key = vm.heap.intern(b"wowlua.engine");
    let list = match vm.heap.table(registry).get_str(key) {
        Value::Table(t) => t,
        _ => {
            let t = vm.new_table();
            vm.heap.table_mut(registry).set_str(key, Value::Table(t));
            t
        }
    };
    let n = vm.heap.table(list).border() as f64 + 1.0;
    let _ = vm.heap.table_mut(list).set(Value::Num(n), v);
}

/// The widget a value is, if it is one.
pub fn widget_of(vm: &Vm, v: Value) -> Option<WidgetId> {
    let Value::Table(t) = v else { return None };
    let Value::Userdata(u) = vm.heap.table(t).get(Value::Num(0.0)) else {
        return None;
    };
    vm.heap.userdata(u).data.downcast_ref::<WidgetId>().copied()
}

/// The class `name` and its bases, nearest first.
fn lineage(e: &Engine, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![name.to_string()];
    while let Some(c) = todo.pop() {
        if out.contains(&c) {
            continue;
        }
        if let Some(class) = e.api.classes.get(&c) {
            todo.extend(class.bases.iter().rev().cloned());
        }
        out.push(c);
    }
    out
}

/// Whether class `name` is or derives from `base` (`IsObjectType`).
pub fn is_a(e: &Engine, name: &str, base: &str) -> bool {
    lineage(e, name)
        .iter()
        .any(|c| c.eq_ignore_ascii_case(base))
}

/// The metatable of class `name`, made on first use: `__index` holds the
/// methods of the class and its bases (the nearest class's wins).
fn class_meta(vm: &mut Vm, name: &str) -> TableRef {
    if let Some(&t) = engine(vm).world.classes.get(name) {
        return t;
    }
    let methods = vm.new_table();
    let classes = lineage(engine(vm), name);
    // Farthest base first, so nearer classes override.
    for class in classes.iter().rev() {
        let indexes = engine(vm)
            .api
            .classes
            .get(class)
            .map(|c| c.methods.clone())
            .unwrap_or_default();
        for i in indexes {
            let short = engine(vm).api.methods[i].short;
            // One function per method, whichever classes have it
            // (unverified).
            let native = match engine(vm).world.natives.get(&i).copied() {
                Some(n) => n,
                None => {
                    let n = vm.native_with(short, method, vec![Value::Num(i as f64)]);
                    keep(vm, n);
                    engine_mut(vm).world.natives.insert(i, n);
                    n
                }
            };
            let key = vm.heap.intern(short.as_bytes());
            vm.heap.table_mut(methods).set_str(key, native);
        }
    }
    let meta = vm.new_table();
    let index = vm.heap.intern(b"__index");
    vm.heap
        .table_mut(meta)
        .set_str(index, Value::Table(methods));
    keep(vm, Value::Table(meta));
    engine_mut(vm).world.classes.insert(name.to_string(), meta);
    meta
}

/// Makes a widget of `class` (already resolved), named and parented.
pub fn create(vm: &mut Vm, class: &str, name: Option<String>, parent: Option<WidgetId>) -> Value {
    let meta = class_meta(vm, class);
    let table = vm.new_table();
    let id = WidgetId(engine(vm).world.widgets.len());
    let ud = vm.heap.alloc_userdata(Userdata {
        serial: 0,
        meta: None,
        env: None,
        data: Box::new(id),
    });
    let _ = vm
        .heap
        .table_mut(table)
        .set(Value::Num(0.0), Value::Userdata(ud));
    vm.heap.table_mut(table).meta = Some(meta);
    let scripts = vm.new_table();
    let hooks = vm.new_table();
    let props = vm.new_table();
    let attributes = vm.new_table();
    for t in [table, scripts, hooks, props, attributes] {
        keep(vm, Value::Table(t));
    }
    let w = Widget {
        class: class.to_string(),
        table,
        scripts,
        hooks,
        props,
        name: name.clone(),
        parent,
        children: Vec::new(),
        shown: true,
        events: HashMap::new(),
        points: Vec::new(),
        width: 0.0,
        height: 0.0,
        scale: 1.0,
        alpha: 1.0,
        text: None,
        attributes,
        protected: false,
    };
    let world = &mut engine_mut(vm).world;
    world.widgets.push(w);
    if let Some(p) = parent {
        world.widgets[p.0].children.push(id);
    }
    if let Some(n) = name {
        vm.set_global(&n, Value::Table(table));
    }
    Value::Table(table)
}

/// `CreateFrame(frameType [, name, parent, template, id])`.
pub fn create_frame(vm: &mut Vm, c: NativeCall) -> Ret {
    let kind = vm.check_str(&c, 1)?;
    let kind = String::from_utf8_lossy(vm.str_bytes(kind)).into_owned();
    // The client takes the type in any case.
    let class = engine(vm)
        .api
        .classes
        .iter()
        .find(|(_, cl)| {
            cl.widget
                .as_deref()
                .is_some_and(|w| w.eq_ignore_ascii_case(&kind))
        })
        .map(|(name, _)| name.clone());
    let Some(class) = class else {
        return Err(vm.error(&format!("CreateFrame: Unknown frame type '{kind}'")));
    };
    let parent = match c.arg(3) {
        Value::Nil => None,
        v => match widget_of(vm, v) {
            Some(p) => Some(p),
            None => return Err(vm.error("CreateFrame: Couldn't find parent frame")),
        },
    };
    let name = match vm.opt_str(&c, 2)? {
        Some(s) => {
            let mut n = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            if n.contains("$parent") || n.contains("$Parent") {
                let pname = parent
                    .and_then(|p| engine(vm).world.widgets[p.0].name.clone())
                    .unwrap_or_default();
                n = n.replace("$parent", &pname).replace("$Parent", &pname);
            }
            Some(n)
        }
        None => None,
    };
    if let Some(t) = vm.opt_str(&c, 4)? {
        let t = String::from_utf8_lossy(vm.str_bytes(t)).into_owned();
        engine_mut(vm).world.warnings.push(format!(
            "CreateFrame: template \"{t}\" ignored (Blizzard's UI code is not loaded)"
        ));
    }
    let frame = create(vm, &class, name, parent);
    if c.has(5) && !c.arg(5).is_nil() {
        let id = vm.check_number(&c, 5)?;
        let key = vm.str("ID");
        let w = widget_of(vm, frame).expect("a widget");
        let props = engine(vm).world.widgets[w.0].props;
        let _ = vm.heap.table_mut(props).set(key, Value::Num(id));
    }
    Ok(vec![frame])
}

/// Every widget method: checks `self` and the arguments, then runs its
/// behaviour, a stored setter or getter, or fails as not mocked.
fn method(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(i)) = c.upvals.first().copied() else {
        return Err(vm.error("a widget method without its index"));
    };
    let api = engine(vm).api;
    let f = &api.methods[i as usize];
    let Some(id) = widget_of(vm, c.arg(1)) else {
        return Err(
            vm.error("Attempt to find 'this' in non-table object (used '.' instead of ':' ?)")
        );
    };
    // A behaviour checks its own arguments (the client's methods take
    // overloads the documentation does not list: `SetPoint(point, x, y)`).
    let behaviour = engine(vm).method_behaviour(f.name, f.short);
    if let Some(b) = behaviour {
        return b(vm, id, c);
    }
    crate::args::check_from(vm, api, f, &c, 2)?;
    stored(vm, id, f.short, f.name, &c)
}

/// A method without behaviour: `SetX(...)` stores its arguments and `GetX()`
/// returns them; anything else is not mocked.
fn stored(vm: &mut Vm, id: WidgetId, short: &str, name: &str, c: &NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    if let Some(stem) = short.strip_prefix("Set") {
        let list = vm.new_table();
        for (k, v) in c.args.iter().skip(1).enumerate() {
            let _ = vm.heap.table_mut(list).set(Value::Num(k as f64 + 1.0), *v);
        }
        let n = vm.str("n");
        let _ = vm
            .heap
            .table_mut(list)
            .set(n, Value::Num((c.len() - 1) as f64));
        let key = vm.str(stem);
        let _ = vm.heap.table_mut(props).set(key, Value::Table(list));
        return Ok(vec![]);
    }
    if let Some(stem) = short.strip_prefix("Get") {
        let key = vm.str(stem);
        if let Value::Table(list) = vm.heap.table(props).get(key) {
            let n = vm.str("n");
            let count = match vm.heap.table(list).get(n) {
                Value::Num(n) => n as usize,
                _ => 0,
            };
            return Ok((1..=count)
                .map(|k| vm.heap.table(list).get(Value::Num(k as f64)))
                .collect());
        }
    }
    Err(vm.error(&format!("{name} is not mocked")))
}

// ------------------------------------------------------------ scripts

/// The script types a class has (from the annotations' `Script<Class>`
/// unions; unverified), with `OnLoad` and `OnUpdate` for every widget.
pub fn has_script(e: &Engine, class: &str, script: &str) -> bool {
    if script == "OnLoad" || script == "OnUpdate" {
        return true;
    }
    // `ScriptButton = ScriptFrame | …`: the named bases are in the lineage
    // too.
    lineage(e, class).iter().any(|c| {
        e.api
            .aliases
            .get(&format!("Script{c}"))
            .is_some_and(|list| {
                list.split('|')
                    .any(|part| part.trim().trim_matches('"') == script)
            })
    })
}

/// Runs a widget's script and its hooks, as the client does: each from a
/// secure execution, an error going to the error handler.
pub fn run_script(vm: &mut Vm, id: WidgetId, script: &str, args: &[Value]) {
    let (table, scripts, hooks) = {
        let w = &engine(vm).world.widgets[id.0];
        (w.table, w.scripts, w.hooks)
    };
    let key = vm.heap.intern(script.as_bytes());
    let mut call = vec![Value::Table(table)];
    call.extend_from_slice(args);
    let f = vm.heap.table(scripts).get_str(key);
    if !f.is_nil() {
        vm.call_from_engine(f, &call);
    }
    if let Value::Table(list) = vm.heap.table(hooks).get_str(key) {
        let n = vm.heap.table(list).border();
        for k in 1..=n {
            let h = vm.heap.table(list).get(Value::Num(k as f64));
            vm.call_from_engine(h, &call);
        }
    }
}

fn script_name(vm: &mut Vm, id: WidgetId, c: &NativeCall, op: &str) -> Result<String, LuaError> {
    let s = vm.check_str(c, 2)?;
    let s = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let class = engine(vm).world.widgets[id.0].class.clone();
    if !has_script(engine(vm), &class, &s) {
        return Err(vm.error(&format!(
            "{class}:{op}(): {class} doesn't have a \"{s}\" script"
        )));
    }
    Ok(s)
}

fn set_script(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let s = script_name(vm, id, &c, "SetScript")?;
    let (scripts, hooks) = {
        let w = &engine(vm).world.widgets[id.0];
        (w.scripts, w.hooks)
    };
    let key = vm.heap.intern(s.as_bytes());
    let f = c.arg(3);
    vm.heap.table_mut(scripts).set_str(key, f);
    vm.heap.table_mut(hooks).set_str(key, Value::Nil);
    Ok(vec![])
}

fn get_script(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let s = script_name(vm, id, &c, "GetScript")?;
    let scripts = engine(vm).world.widgets[id.0].scripts;
    let key = vm.heap.intern(s.as_bytes());
    Ok(vec![vm.heap.table(scripts).get_str(key)])
}

fn hook_script(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let s = script_name(vm, id, &c, "HookScript")?;
    let hooks = engine(vm).world.widgets[id.0].hooks;
    let key = vm.heap.intern(s.as_bytes());
    let list = match vm.heap.table(hooks).get_str(key) {
        Value::Table(t) => t,
        _ => {
            let t = vm.new_table();
            vm.heap.table_mut(hooks).set_str(key, Value::Table(t));
            t
        }
    };
    let n = vm.heap.table(list).border() as f64 + 1.0;
    let _ = vm.heap.table_mut(list).set(Value::Num(n), c.arg(3));
    Ok(vec![])
}

fn has_script_method(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let s = vm.check_str(&c, 2)?;
    let s = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let class = engine(vm).world.widgets[id.0].class.clone();
    Ok(vec![Value::Bool(has_script(engine(vm), &class, &s))])
}

// ------------------------------------------------------------- events

fn event_name(vm: &mut Vm, c: &NativeCall, op: &str) -> Result<String, LuaError> {
    let e = vm.check_str(c, 2)?;
    let e = String::from_utf8_lossy(vm.str_bytes(e)).into_owned();
    if !engine(vm).api.events.contains(&e) {
        let class = widget_of(vm, c.arg(1))
            .map(|id| engine(vm).world.widgets[id.0].class.clone())
            .unwrap_or_default();
        return Err(vm.error(&format!(
            "{class}:{op}(): Attempt to register unknown event \"{e}\""
        )));
    }
    Ok(e)
}

fn register_event(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let e = event_name(vm, &c, "RegisterEvent")?;
    engine_mut(vm).world.widgets[id.0].events.insert(e, None);
    Ok(vec![Value::Bool(true)])
}

fn register_unit_event(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let e = event_name(vm, &c, "RegisterUnitEvent")?;
    let mut units = Vec::new();
    for n in 3..=c.len() {
        if let Value::Str(s) = c.arg(n) {
            units.push(String::from_utf8_lossy(vm.str_bytes(s)).into_owned());
        }
    }
    engine_mut(vm).world.widgets[id.0]
        .events
        .insert(e, Some(units));
    Ok(vec![Value::Bool(true)])
}

fn unregister_event(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let e = vm.check_str(&c, 2)?;
    let e = String::from_utf8_lossy(vm.str_bytes(e)).into_owned();
    engine_mut(vm).world.widgets[id.0].events.remove(&e);
    Ok(vec![])
}

fn unregister_all_events(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    engine_mut(vm).world.widgets[id.0].events.clear();
    Ok(vec![])
}

fn is_event_registered(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let e = vm.check_str(&c, 2)?;
    let e = String::from_utf8_lossy(vm.str_bytes(e)).into_owned();
    let registered = engine(vm).world.widgets[id.0].events.contains_key(&e);
    Ok(vec![Value::Bool(registered)])
}

/// Fires `event` at every widget registered for it, in creation order
/// (unverified); a unit event only where the unit (the first argument)
/// is one the widget registered.
pub fn fire_event(vm: &mut Vm, event: &str, args: &[Value]) {
    let unit = match args.first() {
        Some(Value::Str(s)) => Some(String::from_utf8_lossy(vm.str_bytes(*s)).into_owned()),
        _ => None,
    };
    let targets: Vec<WidgetId> = engine(vm)
        .world
        .widgets
        .iter()
        .enumerate()
        .filter(|(_, w)| match w.events.get(event) {
            Some(None) => true,
            Some(Some(units)) => unit.as_ref().is_some_and(|u| units.contains(u)),
            None => false,
        })
        .map(|(i, _)| WidgetId(i))
        .collect();
    let name = vm.str(event);
    let mut call = vec![name];
    call.extend_from_slice(args);
    for id in targets {
        // A handler may unregister others meanwhile.
        if engine(vm).world.widgets[id.0].events.contains_key(event) {
            run_script(vm, id, "OnEvent", &call);
        }
    }
}

// --------------------------------------------------------- visibility

/// Whether a widget is visible: shown, and its parents too.
pub fn visible(e: &Engine, id: WidgetId) -> bool {
    let mut cur = Some(id);
    while let Some(c) = cur {
        let w = &e.world.widgets[c.0];
        if !w.shown {
            return false;
        }
        cur = w.parent;
    }
    true
}

/// The widget and its descendants, depth first.
fn subtree(e: &Engine, id: WidgetId) -> Vec<WidgetId> {
    let mut out = vec![id];
    let mut i = 0;
    while i < out.len() {
        out.extend(e.world.widgets[out[i].0].children.iter().copied());
        i += 1;
    }
    out
}

/// Shows or hides a widget; `OnShow` / `OnHide` run for each widget whose
/// visibility changed, the widget itself first (unverified order).
pub fn set_shown(vm: &mut Vm, id: WidgetId, shown: bool) {
    let before: Vec<(WidgetId, bool)> = subtree(engine(vm), id)
        .into_iter()
        .map(|w| (w, visible(engine(vm), w)))
        .collect();
    engine_mut(vm).world.widgets[id.0].shown = shown;
    for (w, was) in before {
        let now = visible(engine(vm), w);
        if now != was {
            run_script(vm, w, if now { "OnShow" } else { "OnHide" }, &[]);
        }
    }
}

fn show(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    set_shown(vm, id, true);
    Ok(vec![])
}

fn hide(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    set_shown(vm, id, false);
    Ok(vec![])
}

fn set_shown_method(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_shown(vm, id, c.arg(2).truthy());
    Ok(vec![])
}

fn is_shown(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).world.widgets[id.0].shown)])
}

fn is_visible(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(visible(engine(vm), id))])
}

// ------------------------------------------------------------ identity

fn get_name(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let name = engine(vm).world.widgets[id.0].name.clone();
    Ok(vec![name.map_or(Value::Nil, |n| vm.str(&n))])
}

fn get_object_type(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let class = engine(vm).world.widgets[id.0].class.clone();
    Ok(vec![vm.str(&class)])
}

fn is_object_type(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let t = vm.check_str(&c, 2)?;
    let t = String::from_utf8_lossy(vm.str_bytes(t)).into_owned();
    let class = engine(vm).world.widgets[id.0].class.clone();
    Ok(vec![Value::Bool(is_a(engine(vm), &class, &t))])
}

fn get_parent(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let e = engine(vm);
    Ok(vec![e.world.widgets[id.0].parent.map_or(Value::Nil, |p| {
        Value::Table(e.world.widgets[p.0].table)
    })])
}

fn set_parent(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let parent = match c.arg(2) {
        Value::Nil => None,
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            let g = vm.get_global(&name);
            widget_of(vm, g)
        }
        v => widget_of(vm, v),
    };
    let before = visible(engine(vm), id);
    let world = &mut engine_mut(vm).world;
    if let Some(old) = world.widgets[id.0].parent {
        world.widgets[old.0].children.retain(|&w| w != id);
    }
    world.widgets[id.0].parent = parent;
    if let Some(p) = parent {
        world.widgets[p.0].children.push(id);
    }
    let after = visible(engine(vm), id);
    if before != after {
        run_script(vm, id, if after { "OnShow" } else { "OnHide" }, &[]);
    }
    Ok(vec![])
}

fn get_children(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let e = engine(vm);
    Ok(e.world.widgets[id.0]
        .children
        .iter()
        .filter(|&&k| is_a(e, &e.world.widgets[k.0].class, "Frame"))
        .map(|&k| Value::Table(e.world.widgets[k.0].table))
        .collect())
}

fn get_regions(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let e = engine(vm);
    Ok(e.world.widgets[id.0]
        .children
        .iter()
        .filter(|&&k| !is_a(e, &e.world.widgets[k.0].class, "Frame"))
        .map(|&k| Value::Table(e.world.widgets[k.0].table))
        .collect())
}

fn get_num_children(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let n = get_children(vm, id, c)?.len();
    Ok(vec![Value::Num(n as f64)])
}

/// `CreateFontString`, `CreateTexture`, `CreateMaskTexture`, `CreateLine`:
/// a region of that class, named and parented.
fn create_region(vm: &mut Vm, id: WidgetId, c: NativeCall, class: &str) -> Ret {
    let name = match vm.opt_str(&c, 2)? {
        Some(s) => {
            let mut n = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            let pname = engine(vm).world.widgets[id.0]
                .name
                .clone()
                .unwrap_or_default();
            n = n.replace("$parent", &pname).replace("$Parent", &pname);
            Some(n)
        }
        None => None,
    };
    Ok(vec![create(vm, class, name, Some(id))])
}

// -------------------------------------------------------------- layout

fn number_arg(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<f64, LuaError> {
    vm.check_number(c, n)
}

fn set_point(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let point = vm.check_str(&c, 2)?;
    let point = String::from_utf8_lossy(vm.str_bytes(point)).to_uppercase();
    // SetPoint(point [, relativeTo [, relativePoint]] [, x, y])
    let (relative, relative_point, x, y) = match c.arg(3) {
        Value::Num(_) => (
            None,
            point.clone(),
            number_arg(vm, &c, 3)?,
            vm.opt_number(&c, 4, 0.0)?,
        ),
        rel => {
            let relative = match rel {
                Value::Nil => None,
                Value::Str(s) => {
                    let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
                    Some(vm.get_global(&name))
                }
                v => Some(v),
            };
            let rp = match c.arg(4) {
                Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase(),
                _ => point.clone(),
            };
            let (xi, yi) = if matches!(c.arg(4), Value::Num(_)) {
                (4, 5)
            } else {
                (5, 6)
            };
            (
                relative,
                rp,
                vm.opt_number(&c, xi, 0.0)?,
                vm.opt_number(&c, yi, 0.0)?,
            )
        }
    };
    let w = &mut engine_mut(vm).world.widgets[id.0];
    w.points.retain(|p| p.point != point);
    w.points.push(Point {
        point,
        relative,
        relative_point,
        x,
        y,
    });
    Ok(vec![])
}

fn clear_all_points(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    engine_mut(vm).world.widgets[id.0].points.clear();
    Ok(vec![])
}

fn set_all_points(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let rel = match c.arg(2) {
        Value::Nil => None,
        v => Some(v),
    };
    let w = &mut engine_mut(vm).world.widgets[id.0];
    w.points.clear();
    for p in ["TOPLEFT", "BOTTOMRIGHT"] {
        w.points.push(Point {
            point: p.into(),
            relative: rel,
            relative_point: p.into(),
            x: 0.0,
            y: 0.0,
        });
    }
    Ok(vec![])
}

fn get_num_points(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        engine(vm).world.widgets[id.0].points.len() as f64,
    )])
}

fn get_point(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let i = vm.opt_int(&c, 2, 1)?;
    let Some(p) = engine(vm).world.widgets[id.0]
        .points
        .get((i - 1).max(0) as usize)
        .cloned()
    else {
        return Ok(vec![]);
    };
    let parent = engine(vm).world.widgets[id.0]
        .parent
        .map(|p| Value::Table(engine(vm).world.widgets[p.0].table));
    let point = vm.str(&p.point);
    let rp = vm.str(&p.relative_point);
    Ok(vec![
        point,
        p.relative.or(parent).unwrap_or(Value::Nil),
        rp,
        Value::Num(p.x),
        Value::Num(p.y),
    ])
}

fn set_size(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let w = number_arg(vm, &c, 2)?;
    let h = number_arg(vm, &c, 3)?;
    resize(vm, id, Some(w), Some(h));
    Ok(vec![])
}

fn set_width(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let w = number_arg(vm, &c, 2)?;
    resize(vm, id, Some(w), None);
    Ok(vec![])
}

fn set_height(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let h = number_arg(vm, &c, 2)?;
    resize(vm, id, None, Some(h));
    Ok(vec![])
}

/// Sets the size; `OnSizeChanged` runs when it changes.
fn resize(vm: &mut Vm, id: WidgetId, w: Option<f64>, h: Option<f64>) {
    let widget = &mut engine_mut(vm).world.widgets[id.0];
    let old = (widget.width, widget.height);
    widget.width = w.unwrap_or(old.0);
    widget.height = h.unwrap_or(old.1);
    let new = (widget.width, widget.height);
    if new != old {
        run_script(
            vm,
            id,
            "OnSizeChanged",
            &[Value::Num(new.0), Value::Num(new.1)],
        );
    }
}

fn get_width(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).world.widgets[id.0].width)])
}

fn get_height(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).world.widgets[id.0].height)])
}

fn get_size(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let w = &engine(vm).world.widgets[id.0];
    Ok(vec![Value::Num(w.width), Value::Num(w.height)])
}

fn set_scale(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let s = number_arg(vm, &c, 2)?;
    engine_mut(vm).world.widgets[id.0].scale = s;
    Ok(vec![])
}

fn get_scale(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).world.widgets[id.0].scale)])
}

/// The scale times its parents' scales.
pub fn effective_scale(e: &Engine, id: WidgetId) -> f64 {
    let mut s = 1.0;
    let mut cur = Some(id);
    while let Some(c) = cur {
        let w = &e.world.widgets[c.0];
        s *= w.scale;
        cur = w.parent;
    }
    s
}

fn get_effective_scale(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(effective_scale(engine(vm), id))])
}

fn set_alpha(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let a = number_arg(vm, &c, 2)?;
    engine_mut(vm).world.widgets[id.0].alpha = a.clamp(0.0, 1.0);
    Ok(vec![])
}

fn get_alpha(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).world.widgets[id.0].alpha)])
}

/// Where an anchor point sits along a width and height (0 to 1).
fn anchor(point: &str) -> (f64, f64) {
    let x = if point.contains("LEFT") {
        0.0
    } else if point.contains("RIGHT") {
        1.0
    } else {
        0.5
    };
    let y = if point.contains("BOTTOM") {
        0.0
    } else if point.contains("TOP") {
        1.0
    } else {
        0.5
    };
    (x, y)
}

/// The widget's rectangle in screen units (left, bottom, width, height),
/// from its first anchor; UIParent is the screen.
pub fn bounds(vm: &Vm, id: WidgetId, depth: usize) -> Option<(f64, f64, f64, f64)> {
    let e = engine(vm);
    let w = &e.world.widgets[id.0];
    if w.name.as_deref() == Some("UIParent") || depth > 64 {
        return Some((0.0, 0.0, SCREEN.0, SCREEN.1));
    }
    let p = w.points.first()?;
    let rel = match p.relative {
        Some(v) => widget_of(vm, v),
        None => w.parent,
    };
    let r = rel
        .and_then(|r| bounds(vm, r, depth + 1))
        .unwrap_or((0.0, 0.0, SCREEN.0, SCREEN.1));
    let s = effective_scale(e, id);
    let (rx, ry) = anchor(&p.relative_point);
    let (px, py) = anchor(&p.point);
    let ax = r.0 + rx * r.2 + p.x * s;
    let ay = r.1 + ry * r.3 + p.y * s;
    let (width, height) = (w.width * s, w.height * s);
    Some((ax - px * width, ay - py * height, width, height))
}

fn edge(vm: &mut Vm, id: WidgetId, f: fn((f64, f64, f64, f64)) -> f64) -> Ret {
    let s = effective_scale(engine(vm), id);
    Ok(vec![
        bounds(vm, id, 0).map_or(Value::Nil, |b| Value::Num(f(b) / s)),
    ])
}

fn get_left(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    edge(vm, id, |b| b.0)
}

fn get_right(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    edge(vm, id, |b| b.0 + b.2)
}

fn get_bottom(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    edge(vm, id, |b| b.1)
}

fn get_top(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    edge(vm, id, |b| b.1 + b.3)
}

fn get_center(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let s = effective_scale(engine(vm), id);
    Ok(match bounds(vm, id, 0) {
        Some(b) => vec![
            Value::Num((b.0 + b.2 / 2.0) / s),
            Value::Num((b.1 + b.3 / 2.0) / s),
        ],
        None => vec![],
    })
}

// --------------------------------------------------------------- text

fn set_text(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let t = match c.arg(2) {
        Value::Nil => None,
        v => Some(v),
    };
    engine_mut(vm).world.widgets[id.0].text = t;
    Ok(vec![])
}

fn get_text(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![
        engine(vm).world.widgets[id.0].text.unwrap_or(Value::Nil),
    ])
}

fn set_formatted_text(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let format = vm.get_global("string");
    let Value::Table(string) = format else {
        return Err(vm.error("no string library"));
    };
    let key = vm.str("format");
    let f = vm.heap.table(string).get(key);
    let args: Vec<Value> = c.args.iter().skip(1).copied().collect();
    let r = vm.call(f, &args)?;
    engine_mut(vm).world.widgets[id.0].text = r.first().copied();
    Ok(vec![])
}

// --------------------------------------------------------- attributes

fn set_attribute(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 2)?;
    let attributes = engine(vm).world.widgets[id.0].attributes;
    let v = c.arg(3);
    vm.heap.table_mut(attributes).set_str(key, v);
    let lower = String::from_utf8_lossy(vm.str_bytes(key)).to_lowercase();
    let name = vm.str(&lower);
    run_script(vm, id, "OnAttributeChanged", &[name, v]);
    Ok(vec![])
}

fn get_attribute(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let key = vm.check_str(&c, 2)?;
    let attributes = engine(vm).world.widgets[id.0].attributes;
    Ok(vec![vm.heap.table(attributes).get_str(key)])
}

fn is_protected(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let p = engine(vm).world.widgets[id.0].protected;
    Ok(vec![Value::Bool(p), Value::Bool(false)])
}

fn click(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let button = match c.arg(2) {
        Value::Str(s) => Value::Str(s),
        _ => vm.str("LeftButton"),
    };
    run_script(vm, id, "PreClick", &[button, Value::Bool(false)]);
    run_script(vm, id, "OnClick", &[button, Value::Bool(false)]);
    run_script(vm, id, "PostClick", &[button, Value::Bool(false)]);
    Ok(vec![])
}

/// A method's behaviour: the widget, the call (self first).
pub type MethodFn = fn(&mut Vm, WidgetId, NativeCall) -> Ret;

/// The methods with behaviour, by name (any class that has the method).
pub fn behaviours() -> Vec<(&'static str, MethodFn)> {
    fn font_string(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
        create_region(vm, id, c, "FontString")
    }
    fn texture(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
        create_region(vm, id, c, "Texture")
    }
    fn mask(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
        create_region(vm, id, c, "MaskTexture")
    }
    fn line(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
        create_region(vm, id, c, "Line")
    }
    vec![
        ("SetScript", set_script),
        ("GetScript", get_script),
        ("HookScript", hook_script),
        ("HasScript", has_script_method),
        ("RegisterEvent", register_event),
        ("RegisterUnitEvent", register_unit_event),
        ("UnregisterEvent", unregister_event),
        ("UnregisterAllEvents", unregister_all_events),
        ("IsEventRegistered", is_event_registered),
        ("Show", show),
        ("Hide", hide),
        ("SetShown", set_shown_method),
        ("IsShown", is_shown),
        ("IsVisible", is_visible),
        ("GetName", get_name),
        ("GetObjectType", get_object_type),
        ("IsObjectType", is_object_type),
        ("GetParent", get_parent),
        ("SetParent", set_parent),
        ("GetChildren", get_children),
        ("GetRegions", get_regions),
        ("GetNumChildren", get_num_children),
        ("CreateFontString", font_string),
        ("CreateTexture", texture),
        ("CreateMaskTexture", mask),
        ("CreateLine", line),
        ("SetPoint", set_point),
        ("ClearAllPoints", clear_all_points),
        ("SetAllPoints", set_all_points),
        ("GetNumPoints", get_num_points),
        ("GetPoint", get_point),
        ("SetSize", set_size),
        ("SetWidth", set_width),
        ("SetHeight", set_height),
        ("GetWidth", get_width),
        ("GetHeight", get_height),
        ("GetSize", get_size),
        ("SetScale", set_scale),
        ("GetScale", get_scale),
        ("GetEffectiveScale", get_effective_scale),
        ("SetAlpha", set_alpha),
        ("GetAlpha", get_alpha),
        ("GetLeft", get_left),
        ("GetRight", get_right),
        ("GetTop", get_top),
        ("GetBottom", get_bottom),
        ("GetCenter", get_center),
        ("SetText", set_text),
        ("GetText", get_text),
        ("SetFormattedText", set_formatted_text),
        ("SetAttribute", set_attribute),
        ("GetAttribute", get_attribute),
        ("IsProtected", is_protected),
        ("Click", click),
    ]
}
