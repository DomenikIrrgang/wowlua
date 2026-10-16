//! Widgets (spec emulator §17.6): frames, textures and font strings as the
//! client makes them. A widget is a Lua table holding its object (a
//! userdata) at `[0]`, whose metatable's `__index` is its class's method
//! table: the documented methods of the class and its bases, as natives.
//! A method runs its behaviour here, stores what a setter without one was
//! given (its getter returns it), does nothing if it returns nothing, or
//! fails as not mocked.
//!
//! As `/stdtest engine` recorded on WoW Forever 1.60.1 (fixture
//! `tests/client/forever-camelot-engine.lua`, checked by `wowlua-cli`'s
//! `tests/engine_probe.rs`): events reach frames in registration order;
//! `OnShow` / `OnHide` run children first; `SetScript` clears the hooks;
//! each class has its own method functions; methods check `self` and their
//! documented arguments first (counting `self` as #1, usage `self:Name(…)`,
//! `number` a C float); the client's error messages, with a "Lua Taint:"
//! line where it adds one; `Is…` getters are false until set (a button
//! takes the mouse); alpha keeps eight bits. A template only Blizzard's UI
//! code defines warns while that code is not loaded; any other unknown
//! name fails (decided 2026-10-16). Unverified: `OnSizeChanged` runs with
//! the next frame.

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
    /// When each event was registered (the dispatch order).
    pub event_order: HashMap<String, u64>,
    /// `IsProtected`'s second result: protected by a secure template.
    pub explicitly_protected: bool,
    pub points: Vec<Point>,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub alpha: f64,
    pub text: Option<Value>,
    pub attributes: TableRef,
    pub protected: bool,
    pub moving: bool,
    pub sizing: bool,
    pub enabled: bool,
    pub resize_bounds: Option<(f64, f64, f64, f64)>,
    /// Created by `CreateFrame` (not a region or another script object).
    pub frame: bool,
}

/// The widgets, and each class's method table and metatable.
#[derive(Default)]
pub struct World {
    pub widgets: Vec<Widget>,
    classes: HashMap<String, TableRef>,
    /// Templates and frame types tests (or stand-ins for Blizzard's UI
    /// code) define: name → (base class, function applied to the frame),
    /// the functions kept in the registry.
    pub templates: HashMap<String, Value>,
    pub frame_types: HashMap<String, (String, Value)>,
    /// The last event registration's number.
    pub next_registration: u64,
    /// Widgets whose size changed since the last frame (`OnSizeChanged`
    /// runs with the next frame, not at `SetSize`).
    pub resized: Vec<WidgetId>,
    /// Warnings for gaps that would make every test fail if they were errors
    /// (templates without Blizzard's UI code).
    pub warnings: Vec<String>,
    /// Blizzard's UI code, when loaded: its templates and frame types.
    pub ui: crate::ui::Registry,
    /// Event callbacks (`RegisterEventCallback`), in registration order.
    pub callbacks: Vec<crate::framescript::Callback>,
    /// CVars: name and value, as strings.
    pub cvars: Vec<(String, String)>,
    /// Whether `PLAYER_LOGIN` fired.
    pub logged_in: bool,
    /// Addons `ADDON_LOADED` fired for, lower-case.
    pub loaded_addons: std::collections::HashSet<String>,
    /// Spells and items the test defined.
    pub data: crate::gamedata::GameData,
    /// Addon messages and the guild.
    pub chat: crate::chat::Chat,
    /// Keybindings and the addons a test defined.
    pub bindings: crate::bindings::Bindings,
}

/// UIParent's size in screen units at scale 1: 1920 x 1080, as WoW Forever
/// had it (`/stdtest engine`).
pub const SCREEN: (f64, f64) = (1920.0, 1080.0);

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

/// An error the client's C code raises: no position, and with `taint_line`
/// a "Lua Taint: <addon>" line while execution is tainted (as recorded by
/// `/stdtest engine`).
pub fn c_error(vm: &mut Vm, msg: &str, taint_line: bool) -> LuaError {
    let mut text = msg.to_string();
    if taint_line && let Some(name) = vm.taint_name(vm.taint().stack) {
        text.push_str(&format!("\nLua Taint: {name}"));
    }
    let v = vm.str(&text);
    vm.raise(v)
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

/// The class a name means in any case: the annotations also declare
/// case variants (`alpha`, `ALPHA`) deriving from the real one (`Alpha`).
pub fn canonical_class(e: &Engine, name: &str) -> Option<String> {
    e.api
        .classes
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(name))
        .find(|(k, c)| !c.bases.iter().any(|b| b.eq_ignore_ascii_case(k) && b != *k))
        .map(|(k, _)| k.clone())
}

/// Whether class `name` is or derives from `base` (`IsObjectType`).
pub fn is_a(e: &Engine, name: &str, base: &str) -> bool {
    lineage(e, name)
        .iter()
        .any(|c| c.eq_ignore_ascii_case(base))
}

/// The metatable of class `name`, made on first use: `__index` holds the
/// methods of the class and its bases (the nearest class's wins).
pub fn class_meta(vm: &mut Vm, name: &str) -> TableRef {
    if let Some(&t) = engine(vm).world.classes.get(name) {
        return t;
    }
    let methods = vm.new_table();
    let classes = lineage(engine(vm), name);
    // The client's own list where recorded (camelot): exactly those
    // methods, documented or not (`/stdtest engine`).
    let recorded = crate::widgetdata::data(&engine(vm).flavor)
        .and_then(|d| d.methods.get(name))
        .cloned();
    if let Some(names) = recorded {
        for short in names {
            // The nearest documented method of that name.
            let doc = classes.iter().find_map(|class| {
                let e = engine(vm);
                e.api.classes.get(class).and_then(|c| {
                    c.methods
                        .iter()
                        .copied()
                        .find(|&i| e.api.methods[i].short == short)
                })
            });
            let native = match doc {
                Some(i) => {
                    let short = engine(vm).api.methods[i].short;
                    vm.native_with(short, method, vec![Value::Num(i as f64)])
                }
                None => {
                    let full = vm.str(&format!("{name}:{short}"));
                    let leaked: &'static str = Box::leak(short.clone().into_boxed_str());
                    vm.native_with(leaked, undocumented_method, vec![full])
                }
            };
            keep(vm, native);
            let key = vm.heap.intern(short.as_bytes());
            vm.heap.table_mut(methods).set_str(key, native);
        }
        let meta = vm.new_table();
        let index = vm.heap.intern(b"__index");
        vm.heap
            .table_mut(meta)
            .set_str(index, Value::Table(methods));
        keep(vm, Value::Table(meta));
        engine_mut(vm).world.classes.insert(name.to_string(), meta);
        return meta;
    }
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
            // A function per class and method: `Frame.Show ~= Button.Show`
            // (`/stdtest engine`).
            let native = vm.native_with(short, method, vec![Value::Num(i as f64)]);
            keep(vm, native);
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
        event_order: HashMap::new(),
        explicitly_protected: false,
        moving: false,
        sizing: false,
        enabled: true,
        resize_bounds: None,
        frame: false,
    };
    let mut w = w;
    // A font string starts 1 x 1 (`/stdtest engine`).
    if class == "FontString" {
        w.width = 1.0;
        w.height = 1.0;
    }
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
    // The client's errors (`/stdtest engine`).
    let kind = match c.arg(1) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => {
            let v = vm.tostring(Value::Num(n))?;
            match v {
                Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                _ => String::new(),
            }
        }
        _ => {
            return Err(c_error(
                vm,
                "Usage: CreateFrame(\"frameType\" [, \"name\"] [, parent] [, \"template\"] [, id])",
                false,
            ));
        }
    };
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
    // A frame type defined for the run (`defineFrameType`): its base class
    // and the function that makes it what it is.
    let custom = engine(vm)
        .world
        .frame_types
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&kind))
        .map(|(_, v)| v.clone());
    let intrinsic = engine(vm)
        .world
        .ui
        .intrinsics
        .get(&kind.to_ascii_lowercase())
        .cloned();
    // An intrinsic type of Blizzard's UI code, not loaded: a frame
    // (unverified stand-in).
    let blizzard = engine(vm)
        .api
        .templates
        .contains(&kind.to_ascii_lowercase());
    if class.is_none() && custom.is_none() && intrinsic.is_none() && blizzard {
        engine_mut(vm).world.warnings.push(format!(
            "CreateFrame: frame type \"{kind}\" made a Frame (Blizzard's UI code is not loaded)"
        ));
    }
    let class = class
        .or_else(|| custom.as_ref().map(|(base, _)| base.clone()))
        .or_else(|| intrinsic.as_ref().map(|(base, _)| base.clone()))
        .or_else(|| blizzard.then(|| "Frame".to_string()));
    let Some(class) = class else {
        return Err(c_error(
            vm,
            &format!("CreateFrame(): Unknown frame type '{kind}'"),
            true,
        ));
    };
    let parent = match c.arg(3) {
        Value::Nil => None,
        v => match widget_of(vm, v) {
            Some(p) => Some(p),
            None => return Err(c_error(vm, "Wrong object type for function", false)),
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
    let templates: Vec<String> = match vm.opt_str(&c, 4)? {
        Some(t) => String::from_utf8_lossy(vm.str_bytes(t))
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => Vec::new(),
    };
    let frame = create(vm, &class, name, parent);
    if let Some(id) = widget_of(vm, frame) {
        let w = &mut engine_mut(vm).world.widgets[id.0];
        w.frame = true;
        // Blizzard's secure templates are protected (unverified without
        // their XML).
        w.protected = templates
            .iter()
            .any(|t| t.starts_with("Secure") && t.ends_with("Template"));
        w.explicitly_protected = w.protected;
    }
    if let Some((_, f)) = custom {
        vm.call(f, &[frame])?;
    }
    let from_xml = intrinsic.is_some()
        || templates.iter().any(|t| {
            engine(vm)
                .world
                .ui
                .templates
                .contains_key(&t.to_ascii_lowercase())
        });
    if let Some((_, el)) = intrinsic {
        crate::ui::build::apply_intrinsic(vm, frame, &el);
    }
    for t in templates {
        apply_template(vm, frame, &t)?;
    }
    // Templates of the UI code run their OnLoad once applied.
    if from_xml && let Some(id) = widget_of(vm, frame) {
        run_script(vm, id, "OnLoad", &[]);
    }
    if c.has(5) && !c.arg(5).is_nil() {
        let id = vm.check_number(&c, 5)?;
        let key = vm.str("ID");
        let w = widget_of(vm, frame).expect("a widget");
        let props = engine(vm).world.widgets[w.0].props;
        let _ = vm.heap.table_mut(props).set(key, Value::Num(id));
    }
    Ok(vec![frame])
}

/// Applies template `name` to a frame: the function a test defined for it
/// (`defineTemplate`), or a warning (Blizzard's UI code is not loaded).
pub fn apply_template(vm: &mut Vm, frame: Value, name: &str) -> Result<(), LuaError> {
    let defined = engine(vm)
        .world
        .templates
        .iter()
        .find(|(t, _)| t.eq_ignore_ascii_case(name))
        .map(|(_, f)| *f);
    match defined {
        Some(f) => {
            vm.call(f, &[frame])?;
        }
        None => {
            if !crate::ui::build::apply_template(vm, frame, name) {
                // A template of Blizzard's code only warns while it is not
                // loaded; any other name fails as in the client (decided
                // 2026-10-16, `/stdtest engine`).
                if !engine(vm)
                    .api
                    .templates
                    .contains(&name.to_ascii_lowercase())
                {
                    return Err(c_error(
                        vm,
                        &format!("CreateFrame(): Couldn't find inherited node '{name}'"),
                        true,
                    ));
                }
                engine_mut(vm).world.warnings.push(format!(
                    "CreateFrame: template \"{name}\" ignored (Blizzard's UI code is not loaded)"
                ));
            }
        }
    }
    Ok(())
}

/// Every widget method: checks `self` and the arguments, then runs its
/// behaviour, a stored setter or getter, or fails as not mocked.
fn method(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(i)) = c.upvals.first().copied() else {
        return Err(vm.error("a widget method without its index"));
    };
    let api = engine(vm).api;
    let f = &api.methods[i as usize];
    // `self`, as the client checks it (`/stdtest engine`).
    let id = match c.arg(1) {
        Value::Nil => return Err(vm.api_arg_error(1, &crate::args::usage(f))),
        Value::Table(_) => match widget_of(vm, c.arg(1)) {
            Some(id) => id,
            None => {
                let msg = format!("Wrong object type for function - {}", crate::args::usage(f));
                return Err(vm.api_arg_error(1, &msg));
            }
        },
        _ => {
            let msg = format!(
                "Attempt to find 'this' in non-table object (used '.' instead of ':' ?) - {}",
                crate::args::usage(f)
            );
            return Err(vm.api_arg_error(1, &msg));
        }
    };
    // The documented arguments first (`/stdtest engine blocked`: a bad
    // argument errors on a blocked frame), but where the client's method takes more
    // than the documentation lists (`SetPoint(point, x, y)`): its
    // behaviour checks those.
    const OWN_CHECKS: &[&str] = &[
        "SetPoint",
        // `SetAllPoints()` anchors to the parent; the documentation says
        // `relativeTo` is required.
        "SetAllPoints",
        "GetAttribute",
        "SetAttribute",
        "SetFontObject",
        "RegisterUnitEvent",
        "CreateFontString",
        "CreateTexture",
        "CreateMaskTexture",
        "CreateLine",
    ];
    if !OWN_CHECKS.contains(&f.short) {
        crate::args::check_from(vm, api, f, &c, 2)?;
    }
    if crate::protection::refuses_method(vm, id, f) {
        let api = engine(vm).api;
        return Ok(crate::defaults(vm, api, &f.doc));
    }
    let behaviour = engine(vm).method_behaviour(f.name, f.short);
    if let Some(b) = behaviour {
        return b(vm, id, c);
    }
    stored(vm, id, f, &c)
}

/// A method the client has but the documentation lacks (`CheckButton:
/// GetChecked`, `GameTooltip:AddLine`): its behaviour, the stored setter
/// and getter rules, nothing for Blizzard's code, else not mocked.
fn undocumented_method(vm: &mut Vm, c: NativeCall) -> Ret {
    let full = match c.upvals.first().copied() {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "?".into(),
    };
    let short = full
        .split_once(':')
        .map_or(full.as_str(), |(_, s)| s)
        .to_string();
    let Some(id) = widget_of(vm, c.arg(1)) else {
        let msg = format!("Wrong object type for function - Usage: self:{short}()");
        return Err(vm.api_arg_error(1, &msg));
    };
    let behaviour = engine(vm).method_behaviour(&full, &short);
    if let Some(b) = behaviour {
        return b(vm, id, c);
    }
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
    for prefix in ["Get", "Is"] {
        if let Some(stem) = short.strip_prefix(prefix)
            && let Some(values) = stored_args(vm, props, stem)
        {
            return Ok(if prefix == "Is" {
                vec![Value::Bool(values.first().is_some_and(|v| v.truthy()))]
            } else {
                values
            });
        }
    }
    let class = engine(vm).world.widgets[id.0].class.clone();
    if let Some(values) = recorded_default(engine(vm), &class, &short) {
        return Ok(values.iter().map(|r| r.value(vm)).collect());
    }
    if crate::protection::blizzard_caller(vm) {
        return Ok(vec![]);
    }
    Err(vm.error(&format!("{full} is not mocked")))
}

/// A fresh widget's results for a getter, as the client gave them for the
/// class or its nearest recorded base (`/stdtest engine`).
pub fn recorded_default(
    e: &Engine,
    class: &str,
    getter: &str,
) -> Option<&'static Vec<crate::widgetdata::Recorded>> {
    let data = crate::widgetdata::data(&e.flavor)?;
    lineage(e, class)
        .iter()
        .find_map(|c| data.defaults.get(&(c.clone(), getter.to_string())))
}

/// The stored arguments of setter `stem`, if it ran.
pub fn stored_args(vm: &mut Vm, props: TableRef, stem: &str) -> Option<Vec<Value>> {
    let key = vm.str(stem);
    let Value::Table(list) = vm.heap.table(props).get(key) else {
        return None;
    };
    let n = vm.str("n");
    let count = match vm.heap.table(list).get(n) {
        Value::Num(n) => n as usize,
        _ => 0,
    };
    Some(
        (1..=count)
            .map(|k| vm.heap.table(list).get(Value::Num(k as f64)))
            .collect(),
    )
}

/// A method without behaviour (decided for S3, unverified): `SetX(...)` and
/// `EnableX(...)` store their arguments; `GetX()` returns what `SetX` stored,
/// `IsX()` whether its first value is true (`IsXEnabled` reads `EnableX`);
/// another method that returns nothing does nothing; anything else is not
/// mocked.
fn stored(vm: &mut Vm, id: WidgetId, f: &crate::api::Function, c: &NativeCall) -> Ret {
    let (short, name) = (f.short, f.name);
    let props = engine(vm).world.widgets[id.0].props;
    let returns = f
        .doc
        .get("Returns")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let setter = short
        .strip_prefix("Set")
        .map(str::to_string)
        .or_else(|| short.strip_prefix("Enable").map(|s| format!("{s}Enabled")));
    if let Some(stem) = setter {
        let list = vm.new_table();
        for (k, v) in c.args.iter().skip(1).enumerate() {
            let _ = vm.heap.table_mut(list).set(Value::Num(k as f64 + 1.0), *v);
        }
        let n = vm.str("n");
        let _ = vm
            .heap
            .table_mut(list)
            .set(n, Value::Num((c.len() - 1) as f64));
        let key = vm.str(&stem);
        let _ = vm.heap.table_mut(props).set(key, Value::Table(list));
        // A setter's documented results (`success`): it worked.
        return Ok(returns
            .iter()
            .map(
                |r| match r.get("Type").and_then(serde_json::Value::as_str) {
                    Some("bool") => Value::Bool(true),
                    _ => Value::Nil,
                },
            )
            .collect());
    }
    if let Some(stem) = short.strip_prefix("Get")
        && let Some(values) = stored_args(vm, props, stem)
    {
        return Ok(values);
    }
    // Nothing stored: a fresh widget's results, where recorded.
    let has_stored = short
        .strip_prefix("Is")
        .is_some_and(|stem| stored_args(vm, props, stem).is_some());
    if !has_stored {
        let class = engine(vm).world.widgets[id.0].class.clone();
        if let Some(values) = recorded_default(engine(vm), &class, short) {
            return Ok(values.iter().map(|r| r.value(vm)).collect());
        }
    }
    if let Some(stem) = short.strip_prefix("Is")
        && returns.len() == 1
    {
        let on = match stored_args(vm, props, stem) {
            Some(values) => values.first().is_some_and(|v| v.truthy()),
            // Off until set, but a button takes the mouse
            // (`/stdtest engine`).
            None => {
                let class = engine(vm).world.widgets[id.0].class.clone();
                short == "IsMouseEnabled" && is_a(engine(vm), &class, "Button")
            }
        };
        return Ok(vec![Value::Bool(on)]);
    }
    if returns.is_empty() {
        return Ok(vec![]);
    }
    // Blizzard's code gets empty values; the addon's code an error.
    if crate::protection::blizzard_caller(vm) {
        engine_mut(vm).defaulted.insert(name);
        let api = engine(vm).api;
        return Ok(crate::defaults(vm, api, &f.doc));
    }
    Err(vm.error(&format!("{name} is not mocked")))
}

// ------------------------------------------------------------ scripts

/// The script types a class has (from the annotations' `Script<Class>`
/// unions; unverified), with `OnLoad` and `OnUpdate` for every widget.
pub fn has_script(e: &Engine, class: &str, script: &str) -> bool {
    // The client's answers where recorded (camelot: `/stdtest engine`).
    if let Some(data) = crate::widgetdata::data(&e.flavor)
        && let Some(&yes) = lineage(e, class)
            .iter()
            .find_map(|c| data.scripts.get(&(c.clone(), script.to_string())))
    {
        return yes;
    }
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

/// Whether any widget class has script type `script`.
fn known_script(e: &Engine, script: &str) -> bool {
    script == "OnLoad"
        || script == "OnUpdate"
        || e.api.aliases.iter().any(|(name, list)| {
            name.starts_with("Script")
                && list
                    .split('|')
                    .any(|part| part.trim().trim_matches('"') == script)
        })
}

/// The script type argument: a type no widget has is an argument error; one
/// this widget lacks, "Cannot assign script handler" (`/stdtest engine`).
fn script_name(vm: &mut Vm, id: WidgetId, c: &NativeCall, op: &str) -> Result<String, LuaError> {
    let s = vm.check_str(c, 2)?;
    let s = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let class = engine(vm).world.widgets[id.0].class.clone();
    // Blizzard's code knows the client's scripts better than the lists.
    if has_script(engine(vm), &class, &s) || crate::protection::blizzard_caller(vm) {
        return Ok(s);
    }
    if !known_script(engine(vm), &s) {
        let api = engine(vm).api;
        let usage = api
            .methods
            .iter()
            .find(|m| m.short == op)
            .map(crate::args::usage)
            .unwrap_or_default();
        return Err(vm.api_arg_error(2, &usage));
    }
    let lower = s.to_lowercase();
    Err(c_error(
        vm,
        &format!(
            "{class}:{op}(): Cannot assign script handler for '{lower}' (script type not supported by this object)"
        ),
        true,
    ))
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

/// Whether an event is a unit event: its payload starts with a unit token
/// (undocumented ones: named `UNIT_…`).
fn unit_event(e: &Engine, event: &str) -> bool {
    match e.api.event_docs.get(event) {
        Some(doc) => doc
            .get("Payload")
            .and_then(serde_json::Value::as_array)
            .and_then(|p| p.first())
            .and_then(|p| p.get("Type"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|t| t.starts_with("UnitToken")),
        None => event.starts_with("UNIT_"),
    }
}

/// The event argument, refused as the client refuses it: `RegisterEvent`'s
/// message names the method twice, `RegisterUnitEvent` refuses events that
/// are not unit events (`/stdtest engine`).
fn event_name(vm: &mut Vm, c: &NativeCall, op: &str) -> Result<String, LuaError> {
    let e = vm.check_str(c, 2)?;
    let e = String::from_utf8_lossy(vm.str_bytes(e)).into_owned();
    // Blizzard's code knows the client's events better than the lists.
    let known = engine(vm).api.events.contains(&e) || crate::protection::blizzard_caller(vm);
    let unit_ok = op != "RegisterUnitEvent" || unit_event(engine(vm), &e);
    if !known || !unit_ok {
        let class = widget_of(vm, c.arg(1))
            .map(|id| engine(vm).world.widgets[id.0].class.clone())
            .unwrap_or_default();
        let prefix = if op == "RegisterEvent" {
            format!("{class}:{op}(): {class}:{op}(): ")
        } else {
            format!("{class}:{op}(): ")
        };
        return Err(c_error(
            vm,
            &format!("{prefix}Attempt to register unknown event \"{e}\""),
            false,
        ));
    }
    Ok(e)
}

/// Registers an event, keeping when it was first registered (frames get
/// events in registration order); false if it already was.
fn register(vm: &mut Vm, id: WidgetId, event: String, units: Option<Vec<String>>) -> bool {
    let world = &mut engine_mut(vm).world;
    world.next_registration += 1;
    let seq = world.next_registration;
    let w = &mut world.widgets[id.0];
    let fresh = !w.events.contains_key(&event);
    if fresh {
        w.event_order.insert(event.clone(), seq);
    }
    w.events.insert(event, units);
    fresh
}

fn register_event(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let e = event_name(vm, &c, "RegisterEvent")?;
    let fresh = register(vm, id, e, None);
    Ok(vec![Value::Bool(fresh)])
}

fn register_unit_event(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let e = event_name(vm, &c, "RegisterUnitEvent")?;
    let mut units = Vec::new();
    for n in 3..=c.len() {
        if let Value::Str(s) = c.arg(n) {
            units.push(String::from_utf8_lossy(vm.str_bytes(s)).into_owned());
        }
    }
    register(vm, id, e, Some(units));
    Ok(vec![Value::Bool(true)])
}

fn unregister_event(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let e = vm.check_str(&c, 2)?;
    let e = String::from_utf8_lossy(vm.str_bytes(e)).into_owned();
    let w = &mut engine_mut(vm).world.widgets[id.0];
    w.events.remove(&e);
    w.event_order.remove(&e);
    Ok(vec![])
}

fn unregister_all_events(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let w = &mut engine_mut(vm).world.widgets[id.0];
    w.events.clear();
    w.event_order.clear();
    Ok(vec![])
}

fn is_event_registered(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let e = vm.check_str(&c, 2)?;
    let e = String::from_utf8_lossy(vm.str_bytes(e)).into_owned();
    // (isRegistered, units): the units a unit event was registered for
    // (unverified).
    let units = match engine(vm).world.widgets[id.0].events.get(&e) {
        None => return Ok(vec![Value::Bool(false)]),
        Some(units) => units.clone().unwrap_or_default(),
    };
    let mut out = vec![Value::Bool(true)];
    for u in units {
        out.push(vm.str(&u));
    }
    Ok(out)
}

/// Fires `event` at every widget registered for it, in creation order
/// (unverified); a unit event only where the unit (the first argument)
/// is one the widget registered.
pub fn fire_event(vm: &mut Vm, event: &str, args: &[Value]) {
    let unit = match args.first() {
        Some(Value::Str(s)) => Some(String::from_utf8_lossy(vm.str_bytes(*s)).into_owned()),
        _ => None,
    };
    // In registration order (`/stdtest engine order`).
    let mut targets: Vec<(u64, WidgetId)> = engine(vm)
        .world
        .widgets
        .iter()
        .enumerate()
        .filter(|(_, w)| match w.events.get(event) {
            Some(None) => true,
            Some(Some(units)) => unit.as_ref().is_some_and(|u| units.contains(u)),
            None => false,
        })
        .map(|(i, w)| (w.event_order.get(event).copied().unwrap_or(0), WidgetId(i)))
        .collect();
    targets.sort_by_key(|(seq, _)| *seq);
    let targets: Vec<WidgetId> = targets.into_iter().map(|(_, id)| id).collect();
    if event == "PLAYER_LOGIN" {
        engine_mut(vm).world.logged_in = true;
    }
    if event == "ADDON_LOADED"
        && let Some(Value::Str(s)) = args.first()
    {
        let name = String::from_utf8_lossy(vm.str_bytes(*s)).to_lowercase();
        engine_mut(vm).world.loaded_addons.insert(name);
    }
    let name = vm.str(event);
    let mut call = vec![name];
    call.extend_from_slice(args);
    for id in targets {
        // A handler may unregister others meanwhile.
        if engine(vm).world.widgets[id.0].events.contains_key(event) {
            run_script(vm, id, "OnEvent", &call);
        }
    }
    crate::framescript::run_callbacks(vm, event, args);
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

/// The widget and its descendants, children before their parent (deepest
/// first, siblings in creation order).
fn subtree(e: &Engine, id: WidgetId) -> Vec<WidgetId> {
    fn visit(e: &Engine, id: WidgetId, out: &mut Vec<WidgetId>) {
        for &c in &e.world.widgets[id.0].children {
            visit(e, c, out);
        }
        out.push(id);
    }
    let mut out = Vec::new();
    visit(e, id, &mut out);
    out
}

/// Shows or hides a widget; `OnShow` / `OnHide` run for each widget whose
/// visibility changed, children before their parent (`/stdtest engine`:
/// "g- c- p-", "g+ c+").
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

/// The documentation's mixin classes: not object types
/// (`IsObjectType("ScriptRegion")` is false, `/stdtest engine`).
const MIXIN_CLASSES: &[&str] = &[
    "ScriptObject",
    "FrameScriptObject",
    "ScriptRegion",
    "ScriptRegionResizing",
    "TextureBase",
    "AnimatableObject",
];

/// `IsObjectType(type)`: the widget's object types (Object, Region, Frame,
/// Button, …; lines and mask textures are textures too).
fn is_object_type(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let t = vm.check_str(&c, 2)?;
    let t = String::from_utf8_lossy(vm.str_bytes(t)).into_owned();
    let class = engine(vm).world.widgets[id.0].class.clone();
    let e = engine(vm);
    let texture =
        matches!(class.as_str(), "Line" | "MaskTexture") && t.eq_ignore_ascii_case("Texture");
    let yes = texture
        || lineage(e, &class).iter().any(|c| {
            c.eq_ignore_ascii_case(&t) && !MIXIN_CLASSES.iter().any(|m| m.eq_ignore_ascii_case(c))
        });
    Ok(vec![Value::Bool(yes)])
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
        .filter(|&&k| {
            let class = &e.world.widgets[k.0].class;
            !is_a(e, class, "Frame") && is_a(e, class, "Region")
        })
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
    let region = create(vm, class, name, Some(id));
    // `CreateFontString(name, layer, inherits)`: a font object's font, or
    // a template.
    if let Some(t) = vm.opt_str(&c, 4)? {
        let t = String::from_utf8_lossy(vm.str_bytes(t)).into_owned();
        let g = vm.get_global(&t);
        let is_font =
            widget_of(vm, g).is_some_and(|w| engine(vm).world.widgets[w.0].class == "Font");
        let rid = widget_of(vm, region).expect("a widget");
        if is_font {
            let props = engine(vm).world.widgets[rid.0].props;
            let list = vm.new_table();
            let _ = vm.heap.table_mut(list).set(Value::Num(1.0), g);
            let n = vm.str("n");
            let _ = vm.heap.table_mut(list).set(n, Value::Num(1.0));
            let key = vm.str("FontObject");
            let _ = vm.heap.table_mut(props).set(key, Value::Table(list));
        } else {
            apply_template(vm, region, &t)?;
        }
    }
    Ok(vec![region])
}

// -------------------------------------------------------------- layout

fn number_arg(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<f64, LuaError> {
    vm.check_number(c, n)
}

/// The anchor points a region has.
const POINTS: &[&str] = &[
    "TOPLEFT",
    "TOP",
    "TOPRIGHT",
    "LEFT",
    "CENTER",
    "RIGHT",
    "BOTTOMLEFT",
    "BOTTOM",
    "BOTTOMRIGHT",
];

fn set_point(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let class = engine(vm).world.widgets[id.0].class.clone();
    let Value::Str(point) = c.arg(2) else {
        return Err(c_error(
            vm,
            &format!(
                "{class}:SetPoint(): Usage: (\"point\" [, region or nil] [, \"relativePoint\"] [, offsetX, offsetY]"
            ),
            false,
        ));
    };
    let raw = String::from_utf8_lossy(vm.str_bytes(point)).into_owned();
    let point = raw.to_uppercase();
    if !POINTS.contains(&point.as_str()) {
        return Err(c_error(
            vm,
            &format!("{class}:SetPoint(): Invalid region point {raw}"),
            false,
        ));
    }
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

/// A line has start and end points, not anchors (`/stdtest engine`).
fn not_for_lines(vm: &mut Vm, id: WidgetId, method: &str) -> Result<(), LuaError> {
    let class = engine(vm).world.widgets[id.0].class.clone();
    if class == "Line" {
        return Err(c_error(
            vm,
            &format!("{class}:{method}(): Wrong object type for function"),
            true,
        ));
    }
    Ok(())
}

/// `GetRect()`: left, bottom, width and height, nothing while unanchored.
fn get_rect(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let s = effective_scale(engine(vm), id);
    Ok(match bounds(vm, id, 0) {
        Some(b) if !engine(vm).world.widgets[id.0].points.is_empty() => vec![
            Value::Num(b.0 / s),
            Value::Num(b.1 / s),
            Value::Num(b.2 / s),
            Value::Num(b.3 / s),
        ],
        _ => vec![],
    })
}

fn get_num_points(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    not_for_lines(vm, id, "GetNumPoints")?;
    Ok(vec![Value::Num(
        engine(vm).world.widgets[id.0].points.len() as f64,
    )])
}

fn get_point(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    not_for_lines(vm, id, "GetPoint")?;
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
    // `OnSizeChanged` waits for the next frame (`/stdtest engine`: none at
    // `SetSize`; that it runs then is unverified).
    let world = &mut engine_mut(vm).world;
    if new != old && !world.resized.contains(&id) {
        world.resized.push(id);
    }
}

/// `OnSizeChanged` for the widgets resized since the last frame.
pub fn run_size_changes(vm: &mut Vm) {
    let resized = std::mem::take(&mut engine_mut(vm).world.resized);
    for id in resized {
        let w = &engine(vm).world.widgets[id.0];
        let size = [Value::Num(w.width), Value::Num(w.height)];
        run_script(vm, id, "OnSizeChanged", &size);
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
    if s <= 0.0 {
        let class = engine(vm).world.widgets[id.0].class.clone();
        return Err(c_error(
            vm,
            &format!("{class}:SetScale(): Scale must be > 0"),
            true,
        ));
    }
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
    // Kept in eight bits, read back as a float: 0.5 is 128/255
    // (`/stdtest engine`).
    let a = f64::from((a.clamp(0.0, 1.0) * 255.0).round() as f32 / 255.0);
    engine_mut(vm).world.widgets[id.0].alpha = a;
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

/// Whether a widget has a font: its own or a font object's.
fn has_font(vm: &mut Vm, id: WidgetId) -> bool {
    let props = engine(vm).world.widgets[id.0].props;
    stored_args(vm, props, "Font").is_some() || stored_args(vm, props, "FontObject").is_some()
}

fn set_text(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    // A font string needs a font first (`/stdtest engine`).
    if engine(vm).world.widgets[id.0].class == "FontString" && !has_font(vm, id) {
        return Err(c_error(vm, "FontString:SetText(): Font not set", true));
    }
    let t = match c.arg(2) {
        Value::Nil => None,
        v => Some(v),
    };
    engine_mut(vm).world.widgets[id.0].text = t;
    Ok(vec![])
}

fn get_text(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let w = &engine(vm).world.widgets[id.0];
    // An edit box's text starts empty, not nil (`/stdtest engine`).
    let edit_box = w.class == "EditBox";
    Ok(vec![match w.text {
        Some(t) => t,
        None if edit_box => vm.str(""),
        None => Value::Nil,
    }])
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

/// Attribute names are case-insensitive (unverified): kept lower-case.
fn set_attribute(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Value::Str(key) = c.arg(2) else {
        return Err(c_error(vm, "Arguments: (\"name\", value)", true));
    };
    let lower = String::from_utf8_lossy(vm.str_bytes(key)).to_lowercase();
    let name = vm.str(&lower);
    let attributes = engine(vm).world.widgets[id.0].attributes;
    let v = c.arg(3);
    let _ = vm.heap.table_mut(attributes).set(name, v);
    run_script(vm, id, "OnAttributeChanged", &[name, v]);
    Ok(vec![])
}

fn attribute(vm: &mut Vm, id: WidgetId, name: &str) -> Value {
    let attributes = engine(vm).world.widgets[id.0].attributes;
    let key = vm.str(&name.to_lowercase());
    vm.heap.table(attributes).get(key)
}

/// `GetAttribute(name)`, or `GetAttribute(prefix, name, suffix)`: the first
/// set of `prefix..name..suffix`, `*..name..suffix`, `prefix..name..*`,
/// `*..name..*` and `name` (unverified order).
fn get_attribute(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let first = vm.check_str(&c, 2)?;
    let first = String::from_utf8_lossy(vm.str_bytes(first)).into_owned();
    if c.len() < 4 {
        return Ok(vec![attribute(vm, id, &first)]);
    }
    let name = vm.check_str(&c, 3)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let suffix = vm.opt_str(&c, 4)?;
    let suffix = suffix
        .map(|s| String::from_utf8_lossy(vm.str_bytes(s)).into_owned())
        .unwrap_or_default();
    let prefix = first;
    for key in [
        format!("{prefix}{name}{suffix}"),
        format!("*{name}{suffix}"),
        format!("{prefix}{name}*"),
        format!("*{name}*"),
        name.clone(),
    ] {
        let v = attribute(vm, id, &key);
        if !v.is_nil() {
            return Ok(vec![v]);
        }
    }
    Ok(vec![Value::Nil])
}

fn is_protected(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let w = &engine(vm).world.widgets[id.0];
    Ok(vec![
        Value::Bool(w.protected),
        Value::Bool(w.explicitly_protected),
    ])
}

// --------------------------------------------------------------- levels

/// `GetFrameLevel()`: what `SetFrameLevel` set, else one above the parent
/// (unverified).
fn get_frame_level(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let mut level = 0.0;
    let mut cur = Some(id);
    let mut depth = 0.0;
    while let Some(w) = cur {
        let props = engine(vm).world.widgets[w.0].props;
        if let Some(v) = stored_args(vm, props, "FrameLevel")
            && let Some(Value::Num(n)) = v.first()
        {
            level = *n;
            break;
        }
        cur = engine(vm).world.widgets[w.0].parent;
        if cur.is_some() {
            depth += 1.0;
        }
    }
    Ok(vec![Value::Num(level + depth)])
}

fn get_id(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let v = stored_args(vm, props, "ID")
        .and_then(|v| v.first().copied())
        .unwrap_or(Value::Num(0.0));
    Ok(vec![v])
}

/// `GetStringWidth()`: six units a character at the font's height 12
/// (a stand-in: the client measures glyphs).
fn get_string_width(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let len = match engine(vm).world.widgets[id.0].text {
        Some(Value::Str(s)) => vm.str_bytes(s).len(),
        _ => 0,
    };
    Ok(vec![Value::Num(len as f64 * 6.0)])
}

// --------------------------------------------------------------- fonts

/// `SetFontObject(font)`: a font object or its name; the widget's font is
/// then the object's until `SetFont` sets its own.
fn set_font_object(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let class = engine(vm).world.widgets[id.0].class.clone();
    let props = engine(vm).world.widgets[id.0].props;
    let font = match c.arg(2) {
        // Nil clears it (`/stdtest engine`).
        Value::Nil => {
            let key = vm.str("FontObject");
            let _ = vm.heap.table_mut(props).set(key, Value::Nil);
            return Ok(vec![]);
        }
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            let g = vm.get_global(&name);
            if widget_of(vm, g).is_none() {
                return Err(c_error(
                    vm,
                    &format!("{class}:SetFontObject(): Couldn't find font named {name}"),
                    false,
                ));
            }
            g
        }
        v => v,
    };
    if widget_of(vm, font).is_none() {
        return Err(vm.api_arg_error(2, "Usage: self:SetFontObject(font)"));
    }
    let list = vm.new_table();
    let _ = vm.heap.table_mut(list).set(Value::Num(1.0), font);
    let n = vm.str("n");
    let _ = vm.heap.table_mut(list).set(n, Value::Num(1.0));
    let key = vm.str("FontObject");
    let _ = vm.heap.table_mut(props).set(key, Value::Table(list));
    // Its own font gives way to the object's.
    let own = vm.str("Font");
    let _ = vm.heap.table_mut(props).set(own, Value::Nil);
    Ok(vec![])
}

/// `GetFont()`: the widget's own font, else its font object's.
fn get_font(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let mut cur = id;
    for _ in 0..32 {
        let props = engine(vm).world.widgets[cur.0].props;
        if let Some(v) = stored_args(vm, props, "Font") {
            return Ok(v);
        }
        let next = stored_args(vm, props, "FontObject")
            .and_then(|v| v.first().copied())
            .and_then(|f| widget_of(vm, f));
        match next {
            Some(n) => cur = n,
            None => break,
        }
    }
    // No font: no file, a height, no flags (`/stdtest engine`; a font
    // string's height is garbage there).
    let flags = vm.str("");
    Ok(vec![Value::Nil, Value::Num(0.0), flags])
}

/// Addon code's widgets are never forbidden (Blizzard's protected UI is
/// not loaded).
fn is_forbidden(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
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

// ---------------------------------------------------- moving and state

fn start_moving(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let movable =
        stored_args(vm, props, "Movable").is_some_and(|v| v.first().is_some_and(|m| m.truthy()));
    if !movable {
        let class = engine(vm).world.widgets[id.0].class.clone();
        return Err(c_error(
            vm,
            &format!("{class}:StartMoving(): Frame is not movable"),
            true,
        ));
    }
    engine_mut(vm).world.widgets[id.0].moving = true;
    Ok(vec![])
}

fn start_sizing(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    engine_mut(vm).world.widgets[id.0].sizing = true;
    Ok(vec![])
}

fn stop_moving_or_sizing(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let w = &mut engine_mut(vm).world.widgets[id.0];
    w.moving = false;
    w.sizing = false;
    Ok(vec![])
}

fn set_resize_bounds(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let min_w = number_arg(vm, &c, 2)?;
    let min_h = number_arg(vm, &c, 3)?;
    // Without a maximum, no limit (unverified).
    let max_w = vm.opt_number(&c, 4, 0.0)?;
    let max_h = vm.opt_number(&c, 5, 0.0)?;
    engine_mut(vm).world.widgets[id.0].resize_bounds = Some((min_w, min_h, max_w, max_h));
    Ok(vec![])
}

fn get_resize_bounds(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let (a, b, c, d) = engine(vm).world.widgets[id.0]
        .resize_bounds
        .unwrap_or_default();
    Ok(vec![
        Value::Num(a),
        Value::Num(b),
        Value::Num(c),
        Value::Num(d),
    ])
}

/// Enables or disables a button, edit box or slider; `OnEnable` /
/// `OnDisable` run when it changes (unverified).
fn set_enabled_to(vm: &mut Vm, id: WidgetId, on: bool) {
    let w = &mut engine_mut(vm).world.widgets[id.0];
    let was = std::mem::replace(&mut w.enabled, on);
    let class = w.class.clone();
    let script = if on { "OnEnable" } else { "OnDisable" };
    if was != on && has_script(engine(vm), &class, script) {
        run_script(vm, id, script, &[]);
    }
}

fn set_enabled(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    set_enabled_to(vm, id, c.arg(2).truthy());
    Ok(vec![])
}

fn enable(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    set_enabled_to(vm, id, true);
    Ok(vec![])
}

fn disable(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    set_enabled_to(vm, id, false);
    Ok(vec![])
}

fn is_enabled(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).world.widgets[id.0].enabled)])
}

/// `SetVertexColor(r, g, b [, a])`: alpha 1 when left out; white until set
/// (unverified).
fn set_vertex_color(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let mut rgba = [1.0; 4];
    for (i, v) in rgba.iter_mut().enumerate().take(3) {
        *v = number_arg(vm, &c, i + 2)?;
    }
    rgba[3] = vm.opt_number(&c, 5, 1.0)?;
    let props = engine(vm).world.widgets[id.0].props;
    let list = vm.new_table();
    for (i, v) in rgba.iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(list)
            .set(Value::Num(i as f64 + 1.0), Value::Num(*v));
    }
    let n = vm.str("n");
    let _ = vm.heap.table_mut(list).set(n, Value::Num(4.0));
    let key = vm.str("VertexColor");
    let _ = vm.heap.table_mut(props).set(key, Value::Table(list));
    Ok(vec![])
}

fn get_vertex_color(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    Ok(stored_args(vm, props, "VertexColor").unwrap_or_else(|| vec![Value::Num(1.0); 4]))
}

/// The engine's controls for the mouse: a drag of a frame by (dx, dy)
/// screen units, if it (or, for an overlay, its parent) starts moving.
pub fn drag(vm: &mut Vm, id: WidgetId, dx: f64, dy: f64) {
    let left = vm.str("LeftButton");
    run_script(vm, id, "OnDragStart", &[left]);
    let e = engine(vm);
    let moved = if e.world.widgets[id.0].moving {
        Some(id)
    } else {
        e.world.widgets[id.0]
            .parent
            .filter(|p| e.world.widgets[p.0].moving)
    };
    if let Some(m) = moved
        && let Some(b) = bounds(vm, m, 0)
    {
        let s = effective_scale(engine(vm), m);
        let screen = vm.get_global("UIParent");
        let w = &mut engine_mut(vm).world.widgets[m.0];
        w.points = vec![Point {
            point: "BOTTOMLEFT".into(),
            relative: Some(screen),
            relative_point: "BOTTOMLEFT".into(),
            x: (b.0 + dx) / s,
            y: (b.1 + dy) / s,
        }];
    }
    run_script(vm, id, "OnDragStop", &[]);
}

/// A drag of a resize grip by (dw, dh) screen units: its parent, if it
/// starts sizing, keeps its top left.
pub fn size_by(vm: &mut Vm, grip: WidgetId, dw: f64, dh: f64) {
    let left = vm.str("LeftButton");
    run_script(vm, grip, "OnMouseDown", &[left]);
    if let Some(f) = engine(vm).world.widgets[grip.0].parent
        && engine(vm).world.widgets[f.0].sizing
        && let Some(b) = bounds(vm, f, 0)
    {
        let s = effective_scale(engine(vm), f);
        let screen = vm.get_global("UIParent");
        let w = &mut engine_mut(vm).world.widgets[f.0];
        let (min_w, min_h, max_w, max_h) = w.resize_bounds.unwrap_or_default();
        let limit = |v: f64, min: f64, max: f64| {
            let v = v.max(min);
            if max > 0.0 { v.min(max) } else { v }
        };
        w.width = limit(w.width + dw / s, min_w, max_w);
        w.height = limit(w.height + dh / s, min_h, max_h);
        w.points = vec![Point {
            point: "TOPLEFT".into(),
            relative: Some(screen),
            relative_point: "BOTTOMLEFT".into(),
            x: b.0 / s,
            y: (b.1 + b.3) / s,
        }];
    }
    run_script(vm, grip, "OnMouseUp", &[left]);
}

// ----------------------------------------------------------- functions

/// `GetFramesRegisteredForEvent(event)`: the frames registered for it, in
/// dispatch order.
fn frames_registered_for_event(vm: &mut Vm, c: NativeCall) -> Ret {
    let e = vm.check_str(&c, 1)?;
    let e = String::from_utf8_lossy(vm.str_bytes(e)).into_owned();
    Ok(engine(vm)
        .world
        .widgets
        .iter()
        .filter(|w| w.events.contains_key(&e))
        .map(|w| Value::Table(w.table))
        .collect())
}

/// `C_EventUtils.IsEventValid(event)`: whether the client knows the event.
fn is_event_valid(vm: &mut Vm, c: NativeCall) -> Ret {
    let e = vm.check_str(&c, 1)?;
    let e = String::from_utf8_lossy(vm.str_bytes(e)).into_owned();
    Ok(vec![Value::Bool(engine(vm).api.events.contains(&e))])
}

/// `CreateFont(name)`: a font object, global under its name.
fn create_font(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    Ok(vec![create(vm, "Font", Some(name), None)])
}

/// The frame system's API functions with behaviour.
/// `EnumerateFrames([frame])`: the first frame, or the one after `frame`
/// (regions, fonts and animations are no frames). Creation order here; the
/// client's is another (`/stdtest engine`: UIParent 212th of 3,331 named
/// frames, newer addons' frames first), its rule unknown.
fn enumerate_frames(vm: &mut Vm, c: NativeCall) -> Ret {
    let start = match c.arg(1) {
        Value::Nil => 0,
        v => match widget_of(vm, v) {
            Some(id) => id.0 + 1,
            None => return Ok(vec![Value::Nil]),
        },
    };
    let e = engine(vm);
    let next = e.world.widgets[start.min(e.world.widgets.len())..]
        .iter()
        .find(|w| is_a(e, &w.class, "Frame"))
        .map(|w| Value::Table(w.table));
    Ok(vec![next.unwrap_or(Value::Nil)])
}

pub fn functions() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        ("CreateFont", create_font),
        ("EnumerateFrames", enumerate_frames),
        ("GetFramesRegisteredForEvent", frames_registered_for_event),
        ("C_EventUtils.IsEventValid", is_event_valid),
    ]
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
        ("GetRect", get_rect),
        ("SetText", set_text),
        ("GetText", get_text),
        ("SetFormattedText", set_formatted_text),
        ("SetAttribute", set_attribute),
        ("GetAttribute", get_attribute),
        ("IsProtected", is_protected),
        ("Click", click),
        ("IsForbidden", is_forbidden),
        ("SetFontObject", set_font_object),
        ("GetFrameLevel", get_frame_level),
        ("GetID", get_id),
        ("GetStringWidth", get_string_width),
        ("GetTextWidth", get_string_width),
        ("GetUnboundedStringWidth", get_string_width),
        ("GetFont", get_font),
        ("StartMoving", start_moving),
        ("StartSizing", start_sizing),
        ("StopMovingOrSizing", stop_moving_or_sizing),
        ("SetResizeBounds", set_resize_bounds),
        ("GetResizeBounds", get_resize_bounds),
        ("SetEnabled", set_enabled),
        ("Enable", enable),
        ("Disable", disable),
        ("IsEnabled", is_enabled),
        ("SetVertexColor", set_vertex_color),
        ("GetVertexColor", get_vertex_color),
    ]
}
