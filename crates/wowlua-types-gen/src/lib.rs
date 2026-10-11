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
}

/// Generates the declarations. `skip`: global names the compiler declares
/// itself (Lua 5.1 and WoW's Lua environment). `annotation_source` goes
/// into the shared file's header.
pub fn generate(
    flavors: &[FlavorDocs],
    annotations: &[(String, String)],
    annotation_source: &str,
    skip: &HashSet<String>,
) -> Result<Output, String> {
    let mut per_flavor: Vec<(String, String, Decls)> = Vec::new();
    for f in flavors {
        let mut d = blizzard::read(&f.files)?;
        d.functions.retain(|n, _| !skip.contains(n));
        d.namespaces.retain(|n, _| !skip.contains(n));
        per_flavor.push((f.id.clone(), f.source.clone(), d));
    }
    // Older flavors' documentation is incomplete (vanilla's lacks
    // `GetTime`): a flavor gets the globals, namespaces and types its own
    // documentation lacks from the other flavors', in the given order.
    let fallback: Vec<Decls> = per_flavor.iter().map(|(_, _, d)| d.clone()).collect();
    for (_, _, d) in per_flavor.iter_mut() {
        for other in &fallback {
            for (name, sigs) in &other.functions {
                d.functions
                    .entry(name.clone())
                    .or_insert_with(|| sigs.clone());
            }
            for (name, ns) in &other.namespaces {
                let target = d.namespaces.entry(name.clone()).or_default();
                for (f, sigs) in &ns.functions {
                    target
                        .functions
                        .entry(f.clone())
                        .or_insert_with(|| sigs.clone());
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
            for (name, params) in &other.events {
                d.events
                    .entry(name.clone())
                    .or_insert_with(|| params.clone());
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
    // `CreateFrame("Button", …)` returns a `Button`.
    let mut create = Vec::new();
    for t in blizzard::FRAME_TYPES {
        if shared.classes.contains_key(*t) {
            create.push(create_frame_sig(&format!("{t:?}"), t));
        }
    }
    create.push(create_frame_sig("string", "Frame"));
    shared.functions.insert("CreateFrame".into(), create);
    // Classes only Blizzard knows stay in the flavor files; methods of
    // classes the annotations declare come from the annotations.
    for (_, _, d) in per_flavor.iter_mut() {
        d.classes.retain(|n, _| !shared.classes.contains_key(n));
        blizzard::mark_widgets(&mut d.classes);
    }
    // Globals of Blizzard's UI code and global strings, where nothing else
    // declares them; those of every flavor go into the shared file.
    let widget_classes: HashSet<String> = shared.classes.keys().cloned().collect();
    let mut ui_globals: Vec<indexmap::IndexMap<String, String>> = Vec::new();
    for (f, (_, _, d)) in flavors.iter().zip(&per_flavor) {
        let mut g = framexml::globals(&f.ui_lua, &f.ui_xml, &widget_classes);
        for s in &f.strings {
            g.entry(s.clone()).or_insert_with(|| "string".into());
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
        for (name, ty) in first {
            if ui_globals.iter().all(|g| g.get(&name) == Some(&ty)) {
                shared.globals.insert(name.clone(), ty);
                for g in ui_globals.iter_mut() {
                    g.shift_remove(&name);
                }
            }
        }
    }
    for ((_, _, d), g) in per_flavor.iter_mut().zip(ui_globals) {
        for (name, ty) in g {
            d.globals.entry(name).or_insert(ty);
        }
    }
    let shared_defs = shared.defined_types();
    let shared_refs = shared.referenced_types();
    let mut out_flavors = Vec::new();
    for (id, source, mut d) in per_flavor {
        // Names referred to but declared nowhere become `any`.
        let mut defined: BTreeSet<String> = shared_defs.clone();
        defined.extend(d.defined_types());
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
        out_flavors.push((id, d.emit(&header)));
    }
    let header = format!(
        "wow-api declarations shared by every flavor, generated by wowlua-types-gen from\n{annotation_source}. Do not edit."
    );
    Ok(Output {
        shared: shared.emit(&header),
        flavors: out_flavors,
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
