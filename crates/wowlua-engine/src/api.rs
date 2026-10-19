//! A flavor's API documentation (decision T199), read once per process: its
//! functions, enums and constants, with each function's documentation entry
//! as it is (every flag Blizzard's tables carry).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::{Mutex, OnceLock};

use serde_json::Value as Json;

/// A documented function: global (`GetTime`), in a namespace
/// (`C_Timer.After`), or a widget method (`Frame:Show`).
pub struct Function {
    /// The name code calls it by: `GetTime`, `C_Timer.After`.
    pub name: &'static str,
    /// The name alone (`After`), as stack traces show a native.
    pub short: &'static str,
    pub namespace: Option<&'static str>,
    /// The documentation system it belongs to (`FrameScript`, `Timer`).
    pub system: &'static str,
    /// Its documentation entry: `Arguments`, `Returns` and the flags.
    pub doc: Json,
    /// Its arguments as the client checks them (`crate::args`), worked out
    /// on the first call: from the first argument on, and from the second
    /// (a method's, after `self`).
    pub checked_arguments: [OnceLock<Vec<crate::args::Checked>>; 2],
    /// The secrecy flags its documentation sets (`units::secret_flags`),
    /// read on the first call.
    pub secret_flags: OnceLock<u32>,
}

/// A widget class: its bases (from the annotations), its `CreateFrame`
/// type, and its own methods (indexes into [`Api::methods`]).
#[derive(Default)]
pub struct Class {
    pub bases: Vec<String>,
    pub widget: Option<String>,
    pub methods: Vec<usize>,
}

pub struct Api {
    /// Global and namespaced functions; widget methods are not here.
    pub functions: Vec<Function>,
    /// Widget methods (`name`: `Frame:Show`, `namespace`: the class).
    pub methods: Vec<Function>,
    pub classes: HashMap<String, Class>,
    /// Every event the client knows, and the documented ones' entries
    /// (`Payload`).
    pub events: HashSet<String>,
    pub event_docs: HashMap<String, Json>,
    /// Callback types (`TimerCallback`): functions.
    pub callbacks: HashSet<String>,
    /// `Enum.X` tables: name → (field, value).
    pub enums: Vec<(&'static str, Vec<(String, f64)>)>,
    /// `Enum.XMeta` tables: name → (NumValues, MinValue, MaxValue).
    pub enum_meta: Vec<(&'static str, [f64; 3])>,
    /// `Constants.X` tables: name → (field, value).
    pub constants: Vec<(&'static str, Vec<(String, Json)>)>,
    /// The documentation's enumerations and structures, by name.
    pub enum_names: HashSet<String>,
    pub structures: HashSet<String>,
    /// A structure's fields (`Name`, `Type`, `Nilable`, …).
    pub structure_fields: HashMap<String, Vec<Json>>,
    /// Type aliases from the declarations (`type ClubId = string`).
    pub aliases: HashMap<String, String>,
    /// The templates and intrinsic frame types Blizzard's UI code defines,
    /// lower-case.
    pub templates: HashSet<String>,
    /// Classes the declarations describe: their base and method names
    /// (for widget types the client's own list was not recorded for).
    pub declared: HashMap<String, (Option<String>, Vec<String>)>,
    /// Functions the documentation gives Blizzard's secure code alone
    /// (`Environment: SecureOnly`): namespace and name.
    pub secure_only: Vec<(&'static str, &'static str)>,
}

/// Names live as long as the process: natives' names are `&'static str`.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// The flavor's API, parsed on first use.
pub fn api(flavor: &str) -> Option<&'static Api> {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static Api>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().expect("the API cache");
    if let Some(&api) = cache.get(flavor) {
        return Some(api);
    }
    let bytes = wowlua_api::documentation(flavor)?;
    let mut parsed = parse(bytes);
    if let Some(files) = wowlua_api::declarations(flavor) {
        for file in files {
            // Widget classes' declared methods (Ketho's annotations):
            // `declare class X extends Y` … `function Name(…)` … `end`.
            let mut class: Option<String> = None;
            for line in file.text.lines() {
                if let Some(rest) = line.strip_prefix("declare class ") {
                    let mut words = rest.split_whitespace();
                    let name = words.next().unwrap_or("").to_string();
                    let base = match (words.next(), words.next()) {
                        (Some("extends"), Some(b)) => Some(b.to_string()),
                        _ => None,
                    };
                    let entry = parsed.declared.entry(name.clone()).or_default();
                    if entry.0.is_none() {
                        entry.0 = base;
                    }
                    class = Some(name);
                    continue;
                }
                if line == "end" {
                    class = None;
                    continue;
                }
                if let Some(c) = &class
                    && let Some(f) = line.trim().strip_prefix("function ")
                    && let Some(name) = f.split('(').next()
                {
                    let entry = parsed.declared.entry(c.clone()).or_default();
                    if !entry.1.iter().any(|m| m == name) {
                        entry.1.push(name.to_string());
                    }
                }
            }
            for line in file.text.lines() {
                if let Some((name, ty)) = line
                    .strip_prefix("type ")
                    .and_then(|rest| rest.split_once(" = "))
                {
                    parsed.aliases.insert(name.to_string(), ty.to_string());
                }
            }
        }
    }
    // Frame types the client has that the documentation lacks or gives no
    // base (`/stdtest engine`: each a Frame by IsObjectType).
    for name in ["Browser", "OffScreenFrame", "AuraContainer", "Checkout"] {
        let class = parsed.classes.entry(name.to_string()).or_default();
        if class.bases.is_empty() {
            class.bases.push("Frame".to_string());
        }
        if class.widget.is_none() {
            class.widget = Some(name.to_string());
        }
    }
    // A nameplate (the client makes them in code, `NAME_PLATE_CREATED`):
    // a frame with the hit-test methods (unverified: no probe of its type).
    let class = parsed.classes.entry("NamePlate".to_string()).or_default();
    if class.bases.is_empty() {
        class.bases.push("Frame".to_string());
    }
    // A model scene's actor has its base's methods (`ClearModel`, `Show`)
    // and an object's (`GetParent`: its scene); the documentation gives
    // neither base.
    for (class, base) in [
        ("ModelSceneFrameActor", "ModelSceneFrameActorBase"),
        ("ModelSceneFrameActorBase", "Object"),
    ] {
        let c = parsed.classes.entry(class.to_string()).or_default();
        if c.bases.is_empty() {
            c.bases.push(base.to_string());
        }
    }
    let api: &'static Api = Box::leak(Box::new(parsed));
    cache.insert(flavor.to_string(), api);
    Some(api)
}

fn parse(gz: &[u8]) -> Api {
    let mut text = Vec::new();
    flate2::read::GzDecoder::new(gz)
        .read_to_end(&mut text)
        .expect("the embedded documentation decompresses");
    let data: Json = serde_json::from_slice(&text).expect("the embedded documentation parses");
    let systems: Vec<Json> = data
        .get("systems")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    let mut api = Api {
        functions: Vec::new(),
        methods: Vec::new(),
        classes: HashMap::new(),
        events: data
            .get("events")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Json::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        event_docs: HashMap::new(),
        callbacks: HashSet::new(),
        enums: Vec::new(),
        enum_meta: Vec::new(),
        constants: Vec::new(),
        enum_names: HashSet::new(),
        structures: HashSet::new(),
        structure_fields: HashMap::new(),
        aliases: HashMap::new(),
        templates: data
            .get("templates")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Json::as_str)
                    .map(str::to_ascii_lowercase)
                    .collect()
            })
            .unwrap_or_default(),
        declared: HashMap::new(),
        secure_only: Vec::new(),
    };
    let str_of = |v: &Json, k: &str| v.get(k).and_then(Json::as_str).map(str::to_string);
    let list = |v: &Json, k: &str| {
        v.get(k)
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default()
    };
    if let Some(classes) = data.get("classes").and_then(Json::as_object) {
        for (name, c) in classes {
            let class = api.classes.entry(name.clone()).or_default();
            class.bases = c
                .get("bases")
                .and_then(Json::as_array)
                .map(|b| {
                    b.iter()
                        .filter_map(Json::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            class.widget = c.get("widget").and_then(Json::as_str).map(str::to_string);
        }
    }
    for system in &systems {
        let kind = str_of(system, "Type").unwrap_or_default();
        let sname = leak(&str_of(system, "Name").unwrap_or_default());
        let system_namespace = str_of(system, "Namespace").filter(|n| !n.is_empty());
        let system_env = str_of(system, "Environment");
        if kind == "ScriptObject" {
            let class = str_of(system, "Class").unwrap_or_else(|| sname.to_string());
            for f in list(system, "Functions") {
                let Some(short) = str_of(&f, "Name") else {
                    continue;
                };
                api.methods.push(Function {
                    name: leak(&format!("{class}:{short}")),
                    short: leak(&short),
                    namespace: Some(leak(&class)),
                    system: sname,
                    doc: f,
                    checked_arguments: Default::default(),
                    secret_flags: OnceLock::new(),
                });
                let index = api.methods.len() - 1;
                api.classes
                    .entry(class.clone())
                    .or_default()
                    .methods
                    .push(index);
            }
        }
        if kind != "ScriptObject" {
            for f in list(system, "Functions") {
                let Some(short) = str_of(&f, "Name") else {
                    continue;
                };
                // A function may name its own namespace.
                let namespace = str_of(&f, "Namespace")
                    .filter(|n| !n.is_empty())
                    .or_else(|| system_namespace.clone());
                // Blizzard's secure code only: addons do not have it.
                let env = str_of(&f, "Environment").or_else(|| system_env.clone());
                if env.as_deref() == Some("SecureOnly") {
                    if let Some(ns) = &namespace {
                        api.secure_only.push((leak(ns), leak(&short)));
                    }
                    continue;
                }
                let name = match &namespace {
                    Some(ns) => format!("{ns}.{short}"),
                    None => short.clone(),
                };
                api.functions.push(Function {
                    name: leak(&name),
                    short: leak(&short),
                    namespace: namespace.as_deref().map(leak),
                    system: sname,
                    doc: f,
                    checked_arguments: Default::default(),
                    secret_flags: OnceLock::new(),
                });
            }
        }
        for e in list(system, "Events") {
            if let Some(literal) = str_of(&e, "LiteralName") {
                api.event_docs.insert(literal, e);
            }
        }
        for t in list(system, "Tables") {
            let Some(tname) = str_of(&t, "Name") else {
                continue;
            };
            match str_of(&t, "Type").as_deref() {
                Some("CallbackType") => {
                    api.callbacks.insert(tname);
                }
                Some("Structure") => {
                    api.structure_fields
                        .insert(tname.clone(), list(&t, "Fields"));
                    api.structures.insert(tname);
                }
                Some("Enumeration") => {
                    api.enum_names.insert(tname.clone());
                    let fields = list(&t, "Fields")
                        .iter()
                        .filter_map(|f| Some((str_of(f, "Name")?, f.get("EnumValue")?.as_f64()?)))
                        .collect();
                    api.enums.push((leak(&tname), fields));
                    let num = |k: &str| t.get(k).and_then(Json::as_f64);
                    if let (Some(n), Some(min), Some(max)) =
                        (num("NumValues"), num("MinValue"), num("MaxValue"))
                    {
                        api.enum_meta
                            .push((leak(&format!("{tname}Meta")), [n, min, max]));
                    }
                }
                Some("Constants") => {
                    let values = list(&t, "Values")
                        .iter()
                        .filter_map(|v| Some((str_of(v, "Name")?, v.get("Value")?.clone())))
                        .collect();
                    api.constants.push((leak(&tname), values));
                }
                _ => {}
            }
        }
    }
    api
}
