//! The `wow-api` declaration generator (spec package manager §10.9):
//! Blizzard's API documentation per flavor, enriched with LuaLS
//! annotations (Ketho's vscode-wow-api), written as `.d.wowlua`.

pub mod blizzard;
pub mod framexml;
pub mod luals;
pub mod model;

use model::Decls;
use std::collections::{BTreeSet, HashSet};

/// The generated declarations: one file shared by every flavor (from the
/// annotations) and one per flavor (from Blizzard's documentation).
pub struct Output {
    pub shared: String,
    pub flavors: Vec<(String, String)>,
    /// Each flavor's documentation as JSON, every key kept (the engine's
    /// data, spec emulator §17.6).
    pub api: Vec<(String, serde_json::Value)>,
}

/// One flavor's input: its ID, a description of the sources (for the
/// header), the documentation files, Blizzard's UI code (Lua and XML) and
/// the names of its global strings.
pub struct FlavorDocs {
    pub id: String,
    pub source: String,
    pub files: Vec<(String, String)>,
    pub ui_lua: Vec<(String, String)>,
    pub ui_xml: Vec<(String, String)>,
    pub strings: Vec<String>,
    /// The global API functions the flavor's client has (`Name`,
    /// `Namespace.Member`), when known.
    pub api_names: Option<HashSet<String>>,
    /// The events the flavor's client has, when known.
    pub event_names: Option<HashSet<String>>,
    /// The UI files only load-on-demand addons load (their globals exist
    /// only once the addon is loaded): path (lowercase, `/`) → addon.
    pub lod_files: std::collections::HashMap<String, String>,
}

/// Where the annotations are short: `Frame:GetAttribute` also takes
/// `(prefix, name, suffix)`, the modified lookup Blizzard's
/// `SecureTemplates.lua` uses.
fn widget_corrections(classes: &mut indexmap::IndexMap<String, model::Class>) {
    if let Some(frame) = classes.get_mut("Frame")
        && let Some(sigs) = frame.methods.get_mut("GetAttribute")
        && sigs.iter().all(|s| s.params.len() == 1)
    {
        let param = |name: &str, optional| model::ParamDecl {
            name: name.into(),
            ty: "string".into(),
            optional,
        };
        sigs.push(model::Sig {
            params: vec![
                param("prefix", false),
                param("name", false),
                param("suffix", true),
            ],
            returns: vec!["any".into()],
            ..model::Sig::default()
        });
    }
}

/// Where Blizzard's documentation is wrong: `C_Timer.NewTimer` and
/// `NewTicker` return a handle with `Cancel()` and `IsCancelled()` (the
/// documentation says the callback), and their callbacks receive it.
/// `TimerHandle` and `TickerHandle` are declared by the compiler.
fn corrections(d: &mut Decls) {
    let Some(timer) = d.namespaces.get_mut("C_Timer") else {
        return;
    };
    for (name, handle) in [("NewTimer", "TimerHandle"), ("NewTicker", "TickerHandle")] {
        for sig in timer.functions.get_mut(name).into_iter().flatten() {
            sig.returns = vec![handle.to_string()];
            if let Some(callback) = sig.params.iter_mut().find(|p| p.name == "callback") {
                callback.ty = format!("(handle: {handle}) -> ()");
            }
        }
    }
}

/// Generates the declarations. `skip`: global names the compiler declares
/// itself (Lua 5.1 and WoW's Lua environment). `annotation_source` goes
/// into the shared file's header.
pub fn generate(
    flavors: &[FlavorDocs],
    annotations: &[(String, String)],
    annotation_source: &str,
    skip: &HashSet<String>,
    compiler_types: &HashSet<String>,
) -> Result<Output, String> {
    let mut per_flavor: Vec<(String, String, Decls)> = Vec::new();
    let mut api = Vec::new();
    for f in flavors {
        api.push((f.id.clone(), blizzard::read_json(&f.files)?));
        let mut d = blizzard::read(&f.files)?;
        corrections(&mut d);
        d.functions.retain(|n, _| !skip.contains(n));
        d.namespaces.retain(|n, _| !skip.contains(n));
        per_flavor.push((f.id.clone(), f.source.clone(), d));
    }
    // Older flavors' documentation is incomplete (vanilla's lacks
    // `GetTime`): a flavor gets the globals, namespaces and types its own
    // documentation lacks from the other flavors', in the given order.
    // Functions come only where the flavor's client has them (when known).
    let fallback: Vec<Decls> = per_flavor.iter().map(|(_, _, d)| d.clone()).collect();
    for ((_, _, d), f) in per_flavor.iter_mut().zip(flavors) {
        let has = |name: &str| f.api_names.as_ref().is_none_or(|a| a.contains(name));
        for other in &fallback {
            for (name, sigs) in &other.functions {
                if has(name) {
                    d.functions
                        .entry(name.clone())
                        .or_insert_with(|| sigs.clone());
                }
            }
            for (name, ns) in &other.namespaces {
                for (fname, sigs) in &ns.functions {
                    if has(&format!("{name}.{fname}")) {
                        d.namespaces
                            .entry(name.clone())
                            .or_default()
                            .functions
                            .entry(fname.clone())
                            .or_insert_with(|| sigs.clone());
                    }
                }
            }
            for (name, fields) in &other.interfaces {
                d.interfaces
                    .entry(name.clone())
                    .or_insert_with(|| fields.clone());
            }
            for (name, ty) in &other.aliases {
                d.aliases.entry(name.clone()).or_insert_with(|| ty.clone());
            }
            for (name, members) in &other.enums {
                d.enums
                    .entry(name.clone())
                    .or_insert_with(|| members.clone());
            }
            // Events only where the flavor's client has them (when known).
            for (name, params) in &other.events {
                if f.event_names.as_ref().is_none_or(|e| e.contains(name)) {
                    d.events
                        .entry(name.clone())
                        .or_insert_with(|| params.clone());
                }
            }
        }
    }
    let mut shared = luals::convert(annotations);
    // Blizzard's data wins: drop what any flavor's documentation defines,
    // and what the compiler declares itself.
    let mut bliz_functions: HashSet<String> = HashSet::new();
    let mut bliz_namespaces: HashSet<String> = HashSet::new();
    let mut bliz_types: HashSet<String> = HashSet::new();
    for (_, _, d) in &per_flavor {
        bliz_functions.extend(d.functions.keys().cloned());
        bliz_namespaces.extend(d.namespaces.keys().cloned());
        bliz_types.extend(d.interfaces.keys().cloned());
        bliz_types.extend(d.aliases.keys().cloned());
    }
    shared
        .functions
        .retain(|n, _| !bliz_functions.contains(n) && !skip.contains(n));
    shared.namespaces.retain(|n, _| {
        !bliz_namespaces.contains(n) && !skip.contains(n) && n != "Enum" && n != "Constants"
    });
    shared.aliases.retain(|n, _| !bliz_types.contains(n));
    shared.interfaces.retain(|n, _| !bliz_types.contains(n));
    shared.classes.retain(|n, _| !bliz_types.contains(n));
    shared.globals.retain(|n, _| {
        !skip.contains(n) && !bliz_functions.contains(n) && !bliz_namespaces.contains(n)
    });
    shared.enums.clear();
    shared.constants.clear();
    blizzard::mark_widgets(&mut shared.classes);
    widget_corrections(&mut shared.classes);
    // `CreateFrame("Button", …)` returns a `Button`.
    let mut create = Vec::new();
    for t in blizzard::FRAME_TYPES {
        if shared.classes.contains_key(*t) {
            create.push(create_frame_sig(&format!("{t:?}"), t));
        }
    }
    create.push(create_frame_sig("string", "Frame"));
    shared.functions.insert("CreateFrame".into(), create);
    // Arguments Blizzard documents as accepting secrets
    // (`SecretArguments`) accept them in the annotated methods too.
    for (_, _, d) in &per_flavor {
        for (class_name, class) in &d.classes {
            let Some(target) = shared.classes.get_mut(class_name) else {
                continue;
            };
            for (method, sigs) in &class.methods {
                let Some(targets) = target.methods.get_mut(method) else {
                    continue;
                };
                for sig in sigs {
                    for (i, p) in sig.params.iter().enumerate() {
                        if !p.ty.contains("Secret<") {
                            continue;
                        }
                        for t in targets.iter_mut() {
                            if let Some(tp) = t.params.get_mut(i)
                                && tp.name == p.name
                                && !tp.ty.contains("Secret<")
                            {
                                tp.ty = format!("{} | Secret<{}>", tp.ty, tp.ty);
                            }
                        }
                    }
                }
            }
        }
    }
    // Classes only Blizzard knows stay in the flavor files; methods of
    // classes the annotations declare come from the annotations.
    for (_, _, d) in per_flavor.iter_mut() {
        d.classes.retain(|n, _| !shared.classes.contains_key(n));
        blizzard::mark_widgets(&mut d.classes);
    }
    // Annotated functions some flavor's client lacks move to the flavors
    // that have them.
    let all_have = |name: &str| {
        flavors
            .iter()
            .all(|f| f.api_names.as_ref().is_none_or(|a| a.contains(name)))
    };
    let moved: Vec<String> = shared
        .functions
        .keys()
        .filter(|n| {
            !all_have(n)
                && flavors
                    .iter()
                    .any(|f| f.api_names.as_ref().is_some_and(|a| a.contains(*n)))
        })
        .cloned()
        .collect();
    for name in moved {
        let sigs = shared.functions.shift_remove(&name).expect("present");
        for ((_, _, d), f) in per_flavor.iter_mut().zip(flavors) {
            if f.api_names.as_ref().is_none_or(|a| a.contains(&name)) {
                d.functions
                    .entry(name.clone())
                    .or_insert_with(|| sigs.clone());
            }
        }
    }
    // API functions no source documents: untyped.
    for ((_, _, d), f) in per_flavor.iter_mut().zip(flavors) {
        let Some(api) = &f.api_names else { continue };
        let mut names: Vec<&String> = api.iter().collect();
        names.sort();
        for name in names {
            match name.split_once('.') {
                None => {
                    if model::is_ident(name)
                        && !skip.contains(name)
                        && !d.functions.contains_key(name)
                        && !shared.functions.contains_key(name)
                        && !d.namespaces.contains_key(name)
                        && !shared.namespaces.contains_key(name)
                        && !shared.classes.contains_key(name)
                    {
                        d.globals
                            .entry(name.clone())
                            .or_insert_with(|| "(...any) -> ...any".into());
                    }
                }
                Some((ns, member))
                    if model::is_ident(ns) && model::is_ident(member) && !skip.contains(ns) =>
                {
                    let in_shared = shared
                        .namespaces
                        .get(ns)
                        .is_some_and(|n| n.functions.contains_key(member));
                    if !in_shared && !shared.classes.contains_key(ns) {
                        let target = d.namespaces.entry(ns.to_string()).or_default();
                        target
                            .functions
                            .entry(member.to_string())
                            .or_insert_with(|| {
                                vec![model::Sig {
                                    vararg: Some("any".into()),
                                    ret_vararg: Some("any".into()),
                                    ..model::Sig::default()
                                }]
                            });
                    }
                }
                _ => {}
            }
        }
    }
    // Globals of Blizzard's UI code and global strings, where nothing else
    // declares them; those of every flavor go into the shared file.
    let widget_classes: HashSet<String> = shared.classes.keys().cloned().collect();
    // Each with the load-on-demand addon that alone defines it, if any.
    let mut ui_globals: Vec<indexmap::IndexMap<String, (String, Option<String>)>> = Vec::new();
    for (f, (_, _, d)) in flavors.iter().zip(&per_flavor) {
        let lod = framexml::lod_origins(&f.ui_lua, &f.ui_xml, &f.lod_files, &widget_classes);
        let mut g: indexmap::IndexMap<String, (String, Option<String>)> =
            framexml::globals(&f.ui_lua, &f.ui_xml, &widget_classes)
                .into_iter()
                .map(|(n, ty)| {
                    let addon = lod.get(&n).cloned();
                    (n, (ty, addon))
                })
                .collect();
        for s in &f.strings {
            g.entry(s.clone())
                .or_insert_with(|| ("string".into(), None));
        }
        g.retain(|n, _| {
            !skip.contains(n)
                && !d.functions.contains_key(n)
                && !d.namespaces.contains_key(n)
                && !d.globals.contains_key(n)
                && !shared.functions.contains_key(n)
                && !shared.namespaces.contains_key(n)
                && !shared.globals.contains_key(n)
                && !shared.classes.contains_key(n)
        });
        ui_globals.push(g);
    }
    if let Some(first) = ui_globals.first().cloned() {
        for (name, (ty, addon)) in first {
            if ui_globals
                .iter()
                .all(|g| g.get(&name) == Some(&(ty.clone(), addon.clone())))
            {
                shared.globals.insert(name.clone(), ty);
                if let Some(a) = addon {
                    shared.global_addons.insert(name.clone(), a);
                }
                for g in ui_globals.iter_mut() {
                    g.shift_remove(&name);
                }
            }
        }
    }
    for ((_, _, d), g) in per_flavor.iter_mut().zip(ui_globals) {
        for (name, (ty, addon)) in g {
            if !d.globals.contains_key(&name)
                && let Some(a) = addon
            {
                d.global_addons.insert(name.clone(), a);
            }
            d.globals.entry(name).or_insert(ty);
        }
    }
    // The aura filter words each flavor's UI code accepts (wowlua:aura
    // checks literal filters against them); a flavor without the table
    // takes the words every other flavor has.
    let words: Vec<Vec<String>> = flavors
        .iter()
        .map(|f| framexml::aura_filter_words(&f.ui_lua))
        .collect();
    let known: Vec<&Vec<String>> = words.iter().filter(|w| !w.is_empty()).collect();
    let fallback: Vec<String> = known
        .first()
        .map(|first| {
            first
                .iter()
                .filter(|w| known.iter().all(|k| k.contains(w)))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for ((_, _, d), w) in per_flavor.iter_mut().zip(&words) {
        let w = if w.is_empty() { &fallback } else { w };
        if !w.is_empty() {
            let union: Vec<String> = w.iter().map(|x| format!("{x:?}")).collect();
            d.aliases.insert("AuraFilterWord".into(), union.join(" | "));
        }
    }
    let shared_defs = shared.defined_types();
    let shared_refs = shared.referenced_types();
    let mut out_flavors = Vec::new();
    for (id, source, mut d) in per_flavor {
        // Names referred to but declared nowhere become `any` (the
        // compiler's own declarations define some).
        let mut defined: BTreeSet<String> = shared_defs.clone();
        defined.extend(d.defined_types());
        defined.extend(compiler_types.iter().cloned());
        let mut refs = shared_refs.clone();
        refs.extend(d.referenced_types());
        for name in refs.difference(&defined) {
            if model::is_ident(name) {
                d.aliases
                    .entry(name.clone())
                    .or_insert_with(|| "any".into());
            }
        }
        let header = format!(
            "wow-api declarations for the `{id}` flavor, generated by wowlua-types-gen from\nBlizzard's API documentation ({source}). Do not edit."
        );
        // The engine's data: the documentation, and each class's bases and
        // `CreateFrame` type (from the annotations; the documentation has
        // no inheritance).
        if let Some((_, json)) = api.iter_mut().find(|(f, _)| *f == id) {
            let systems = std::mem::take(json);
            let classes: serde_json::Map<String, serde_json::Value> = shared
                .classes
                .iter()
                .chain(d.classes.iter())
                .map(|(name, c)| {
                    (
                        name.clone(),
                        serde_json::json!({
                            "bases": c.bases.iter().chain(c.also.iter()).collect::<Vec<_>>(),
                            "widget": c.widget,
                        }),
                    )
                })
                .collect();
            // Every event the client knows, documented or not.
            let mut events: Vec<&String> = d.events.keys().collect();
            if let Some(names) = flavors
                .iter()
                .find(|f| f.id == id)
                .and_then(|f| f.event_names.as_ref())
            {
                events.extend(names.iter());
            }
            events.sort();
            events.dedup();
            *json = serde_json::json!({ "systems": systems, "classes": classes, "events": events });
        }
        out_flavors.push((id, d.emit(&header)));
    }
    let header = format!(
        "wow-api declarations shared by every flavor, generated by wowlua-types-gen from\n{annotation_source}. Do not edit."
    );
    Ok(Output {
        shared: shared.emit(&header),
        flavors: out_flavors,
        api,
    })
}

fn create_frame_sig(frame_type: &str, result: &str) -> model::Sig {
    let p = |name: &str, ty: &str, optional: bool| model::ParamDecl {
        name: name.into(),
        ty: ty.into(),
        optional,
    };
    model::Sig {
        params: vec![
            p("frameType", frame_type, false),
            p("name", "string", true),
            p("parent", "any", true),
            p("template", "string", true),
            p("id", "number", true),
        ],
        returns: vec![result.into()],
        ..model::Sig::default()
    }
}
