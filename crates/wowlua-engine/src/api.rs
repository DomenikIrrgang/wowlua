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
}

pub struct Api {
    /// Global and namespaced functions; widget methods are not here.
    pub functions: Vec<Function>,
    /// `Enum.X` tables: name → (field, value).
    pub enums: Vec<(&'static str, Vec<(String, f64)>)>,
    /// `Constants.X` tables: name → (field, value).
    pub constants: Vec<(&'static str, Vec<(String, Json)>)>,
    /// The documentation's enumerations and structures, by name.
    pub enum_names: HashSet<String>,
    pub structures: HashSet<String>,
    /// Type aliases from the declarations (`type ClubId = string`).
    pub aliases: HashMap<String, String>,
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
    let api: &'static Api = Box::leak(Box::new(parsed));
    cache.insert(flavor.to_string(), api);
    Some(api)
}

fn parse(gz: &[u8]) -> Api {
    let mut text = Vec::new();
    flate2::read::GzDecoder::new(gz)
        .read_to_end(&mut text)
        .expect("the embedded documentation decompresses");
    let systems: Vec<Json> =
        serde_json::from_slice(&text).expect("the embedded documentation parses");
    let mut api = Api {
        functions: Vec::new(),
        enums: Vec::new(),
        constants: Vec::new(),
        enum_names: HashSet::new(),
        structures: HashSet::new(),
        aliases: HashMap::new(),
    };
    let str_of = |v: &Json, k: &str| v.get(k).and_then(Json::as_str).map(str::to_string);
    let list = |v: &Json, k: &str| {
        v.get(k)
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for system in &systems {
        let kind = str_of(system, "Type").unwrap_or_default();
        let sname = leak(&str_of(system, "Name").unwrap_or_default());
        let system_namespace = str_of(system, "Namespace").filter(|n| !n.is_empty());
        let system_env = str_of(system, "Environment");
        if kind != "ScriptObject" {
            for f in list(system, "Functions") {
                let Some(short) = str_of(&f, "Name") else {
                    continue;
                };
                // Blizzard's code only: addons do not have it.
                let env = str_of(&f, "Environment").or_else(|| system_env.clone());
                if env.as_deref() == Some("SecureOnly") {
                    continue;
                }
                // A function may name its own namespace.
                let namespace = str_of(&f, "Namespace")
                    .filter(|n| !n.is_empty())
                    .or_else(|| system_namespace.clone());
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
                });
            }
        }
        for t in list(system, "Tables") {
            let Some(tname) = str_of(&t, "Name") else {
                continue;
            };
            match str_of(&t, "Type").as_deref() {
                Some("Structure") => {
                    api.structures.insert(tname);
                }
                Some("Enumeration") => {
                    api.enum_names.insert(tname.clone());
                    let fields = list(&t, "Fields")
                        .iter()
                        .filter_map(|f| Some((str_of(f, "Name")?, f.get("EnumValue")?.as_f64()?)))
                        .collect();
                    api.enums.push((leak(&tname), fields));
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
