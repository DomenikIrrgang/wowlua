//! `cargo xtask api-reference`: the WoW API reference's data for the
//! website (`docs/reference/api/camelot.json.gz`): camelot's functions,
//! widget methods, events and tables as Blizzard's documentation describes
//! them, what each does (Blizzard's descriptions, warcraft.wiki.gg's and
//! wowlua's own, each marked with where it comes from), and how far the
//! emulator answers it (the coverage report's sources, `coverage.rs`). It
//! reads checked-in files only and gzips without a timestamp, so every run
//! writes the same bytes and `cargo xtask check` can tell a stale one.

use anyhow::{Context, Result};
use serde_json::{Map, Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use wowlua_engine::coverage::{Entry, Group, Source};
use wowlua_types_gen::docs::{self, Doc, Docs, Written};

use crate::coverage::{Counts, counts};

pub const PATH: &str = "docs/reference/api/camelot.json.gz";

/// The flavor the reference describes, and its client's name.
const FLAVOR: &str = "camelot";
const CLIENT: &str = "WoW Forever";

const WIKI: &str = "https://warcraft.wiki.gg/wiki/";

pub fn generate(root: &Path) -> Result<Vec<u8>> {
    let text = serde_json::to_vec(&reference(root)?)?;
    let mut gz = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut gz, &text)?;
    Ok(gz.finish()?)
}

/// The documentation's descriptions, merged as the declarations' doc
/// comments are (`wowlua-types-gen`), and each source's own.
struct Descriptions {
    merged: Docs,
    wiki: Docs,
    blizzard: Docs,
}

impl Descriptions {
    fn read(root: &Path, json: &Json) -> Result<Descriptions> {
        let read = |file: &str| -> Result<String> {
            fs::read_to_string(root.join(file)).with_context(|| file.to_string())
        };
        let wiki: Docs = serde_json::from_str(&read(crate::types::WIKI_DOCS)?)?;
        let written: BTreeMap<String, Written> =
            serde_json::from_str(&read(crate::types::WRITTEN_DOCS)?)?;
        let blizzard = docs::from_blizzard(json);
        let mut merged = docs::merge(&wiki, &blizzard);
        docs::add_written(&mut merged, &written);
        Ok(Descriptions {
            merged,
            wiki,
            blizzard,
        })
    }

    /// Where a description of `key` comes from: `blizzard`, `wiki` or
    /// `wowlua`, as `docs::merge` and `docs::add_written` take them: the
    /// wiki's over Blizzard's, wowlua's where neither says anything.
    /// `value` names an argument or result (`true`: a result).
    fn source(&self, key: &str, value: Option<(&str, bool)>, doc: &Doc) -> &'static str {
        let says = |d: Option<&Doc>| {
            d.is_some_and(|d| match value {
                None => !d.summary.is_empty(),
                Some((name, returns)) => {
                    let list = if returns { &d.returns } else { &d.params };
                    list.iter().any(|(n, _)| n == name)
                }
            })
        };
        if value.is_none() && doc.written {
            "wowlua"
        } else if says(self.wiki.get(key)) {
            "wiki"
        } else if says(self.blizzard.get(key)) {
            "blizzard"
        } else {
            "wowlua"
        }
    }
}

/// `IsProtectedFunction` as `isProtectedFunction`.
fn camel(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) => c.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// A list the documentation may write as `{}` when empty.
fn list<'a>(v: &'a Json, key: &str) -> &'a [Json] {
    v.get(key)
        .and_then(Json::as_array)
        .map_or(&[], Vec::as_slice)
}

fn text(v: &Json, key: &str) -> Option<String> {
    v.get(key).and_then(Json::as_str).map(str::to_string)
}

/// A `Documentation = {…}`'s lines as one text.
fn documentation(v: &Json) -> Option<String> {
    let lines: Vec<&str> = list(v, "Documentation")
        .iter()
        .filter_map(Json::as_str)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// The flags an entry sets: every key but those `known`, camelCased.
fn flags(v: &Json, known: &[&str]) -> Map<String, Json> {
    let mut out = Map::new();
    if let Some(o) = v.as_object() {
        for (k, value) in o {
            if !known.contains(&k.as_str()) {
                out.insert(camel(k), value.clone());
            }
        }
    }
    out
}

/// An entry's type as Blizzard names it: its mixin (`ColorMixin`) or its
/// type, `X[]` for a table of `X`, `?` when it may be nil.
fn type_of(v: &Json) -> String {
    let ty = text(v, "Type").unwrap_or_else(|| "any".into());
    let base = match (text(v, "Mixin"), text(v, "InnerType")) {
        (Some(m), _) => m,
        (None, Some(inner)) if ty == "table" => format!("{inner}[]"),
        _ => ty,
    };
    if v.get("Nilable").and_then(Json::as_bool) == Some(true) {
        format!("{base}?")
    } else {
        base
    }
}

const VALUE_KEYS: &[&str] = &[
    "Name",
    "Type",
    "InnerType",
    "Mixin",
    "Nilable",
    "Default",
    "Documentation",
];

/// An argument, result, payload value or field: its name, type, whether
/// it may be nil, its default, its description and flags.
fn value(v: &Json, described: Option<&str>) -> Json {
    let mut out = Map::new();
    out.insert("name".into(), json!(text(v, "Name").unwrap_or_default()));
    out.insert("type".into(), json!(type_of(v)));
    out.insert(
        "nilable".into(),
        json!(v.get("Nilable").and_then(Json::as_bool).unwrap_or(false)),
    );
    if let Some(d) = v.get("Default") {
        out.insert("default".into(), d.clone());
    }
    if let Some(d) = described.map(str::to_string).or_else(|| documentation(v)) {
        out.insert("doc".into(), json!(d));
    }
    let f = flags(v, VALUE_KEYS);
    if !f.is_empty() {
        out.insert("flags".into(), Json::Object(f));
    }
    Json::Object(out)
}

fn status(source: Source) -> &'static str {
    match source {
        Source::Model => "implemented",
        Source::Recorded => "recorded",
        Source::FillIn => "guessed",
        Source::Generated => "not implemented",
    }
}

fn counts_json(c: &Counts) -> Json {
    json!({
        "total": c.total(),
        "implemented": c.model,
        "recorded": c.recorded,
        "guessed": c.fill_in,
        "not implemented": c.generated,
    })
}

/// Arguments, results or a payload with their descriptions (of `key`'s
/// entry in the descriptions, `returns` for results), adding where those
/// come from to `sources`.
fn described_values(
    d: &Descriptions,
    key: &str,
    items: &[Json],
    returns: bool,
    sources: &mut BTreeSet<&'static str>,
) -> Json {
    let described = d.merged.get(key);
    let docs = described.map(|x| if returns { &x.returns } else { &x.params });
    Json::Array(
        items
            .iter()
            .map(|v| {
                let name = text(v, "Name").unwrap_or_default();
                let text = docs
                    .and_then(|l| l.iter().find(|(n, _)| *n == name))
                    .map(|(_, t)| t.as_str());
                if let (Some(_), Some(x)) = (text, described) {
                    sources.insert(d.source(key, Some((&name, returns)), x));
                }
                value(v, text)
            })
            .collect(),
    )
}

/// A function's or method's signature, flags, description and status.
/// `key` is its key in the descriptions (`C_Timer.After`, `Frame:Show`).
/// `from_wiki`: the signature is the wiki's (`wiki_signature`), a source.
fn callable(
    d: &Descriptions,
    key: &str,
    doc: Option<&Json>,
    from_wiki: bool,
    entry: &Entry,
) -> Map<String, Json> {
    let described = d.merged.get(key);
    let mut sources = BTreeSet::new();
    if from_wiki {
        sources.insert("wiki");
    }
    let mut values = |list_key: &str, returns: bool| -> Json {
        let items = doc.map(|f| list(f, list_key)).unwrap_or_default();
        described_values(d, key, items, returns, &mut sources)
    };
    let (args, returns) = (values("Arguments", false), values("Returns", true));
    let mut out = Map::new();
    out.insert("args".into(), args);
    out.insert("returns".into(), returns);
    out.insert(
        "flags".into(),
        Json::Object(doc.map_or_else(Map::new, |f| {
            flags(
                f,
                &[
                    "Name",
                    "Type",
                    "Arguments",
                    "Returns",
                    "Documentation",
                    "Namespace",
                ],
            )
        })),
    );
    out.insert("doc".into(), description(d, key, described, sources));
    out.insert("status".into(), json!(status(entry.source)));
    out.insert(
        "module".into(),
        if entry.module.is_empty() {
            Json::Null
        } else {
            json!(entry.module)
        },
    );
    out
}

/// What an entry does: its summary and where it comes from, its wiki page,
/// and every source its descriptions come from (`sources`, for the site's
/// line naming them); null when nothing describes it.
fn description(
    d: &Descriptions,
    key: &str,
    described: Option<&Doc>,
    mut sources: BTreeSet<&'static str>,
) -> Json {
    let summary = described.map(|x| x.summary.as_str()).unwrap_or("");
    let page = described.and_then(|x| x.page.as_deref());
    let summary_source = described
        .filter(|_| !summary.is_empty())
        .map(|x| d.source(key, None, x));
    if let Some(s) = summary_source {
        sources.insert(s);
    }
    if summary.is_empty() && page.is_none() && sources.is_empty() {
        return Json::Null;
    }
    json!({
        "summary": if summary.is_empty() { Json::Null } else { json!(summary) },
        "summarySource": summary_source,
        "wiki": page.map(|p| format!("{WIKI}{p}")),
        "sources": sources.into_iter().collect::<Vec<_>>(),
    })
}

/// A function's first form in the wiki's annotations, written as
/// Blizzard's documentation writes one (`Arguments`, `Returns`: `Name`,
/// `Type`, `Nilable`), its types in wowlua's syntax.
fn wiki_signature(decls: &wowlua_types_gen::model::Decls, name: &str) -> Option<Json> {
    let sigs = match name.split_once('.') {
        Some((ns, member)) => decls.namespaces.get(ns)?.functions.get(member)?,
        None => decls.functions.get(name)?,
    };
    let sig = sigs.first()?;
    let entry = |name: &str, ty: &str, optional: bool| {
        let nilable = optional || ty.ends_with('?');
        json!({"Name": name, "Type": ty.strip_suffix('?').unwrap_or(ty), "Nilable": nilable})
    };
    let mut args: Vec<Json> = sig
        .params
        .iter()
        .map(|p| entry(&p.name, &p.ty, p.optional))
        .collect();
    if let Some(ty) = &sig.vararg {
        args.push(entry("...", ty, false));
    }
    let names = decls.return_names.get(name);
    let mut returns: Vec<Json> = sig
        .returns
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let n = names.and_then(|n| n.get(i)).map_or("", String::as_str);
            entry(n, ty, false)
        })
        .collect();
    if let Some(ty) = &sig.ret_vararg {
        returns.push(entry("...", ty, false));
    }
    Some(json!({"Arguments": args, "Returns": returns}))
}

/// The client's build, as the declarations' header names the
/// documentation's (`wow-ui-source 1.60.1 (70450), commit …`).
fn client(root: &Path) -> Result<String> {
    let file = format!("crates/wowlua-api/data/{FLAVOR}.d.wowlua");
    let text = fs::read_to_string(root.join(&file)).with_context(|| file.clone())?;
    let header: String = text.lines().take(5).collect::<Vec<_>>().join("\n");
    let build = regex::Regex::new(r"wow-ui-source ([^,]+), commit")?
        .captures(&header)
        .with_context(|| format!("{file}: no wow-ui-source build in its header"))?[1]
        .to_string();
    Ok(format!("{CLIENT} {build}"))
}

fn reference(root: &Path) -> Result<Json> {
    let file = format!("crates/wowlua-api/data/{FLAVOR}.api.json.gz");
    let gz = fs::read(root.join(&file)).with_context(|| file.clone())?;
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(gz.as_slice()), &mut raw)?;
    let json: Json = serde_json::from_slice(&raw)?;
    let d = Descriptions::read(root, &json)?;
    let api = wowlua_engine::api::api(FLAVOR).context("camelot's API")?;
    let functions = wowlua_engine::coverage::functions(FLAVOR).map_err(anyhow::Error::msg)?;
    let methods = wowlua_engine::coverage::methods(FLAVOR).map_err(anyhow::Error::msg)?;
    let events = wowlua_engine::coverage::events(FLAVOR).map_err(anyhow::Error::msg)?;
    let (exact, probed) = crate::coverage::full_probe(root)?;
    let systems = list(&json, "systems");

    // Functions: the documented ones and the client's undocumented ones
    // (the engine's own, which the client lacks, are left out).
    // The undocumented functions' signatures, as warcraft.wiki.gg's pages
    // give them (`cargo xtask types wiki`), read as the declarations read
    // them.
    let wiki_text = fs::read_to_string(root.join(crate::types::WIKI_DATA))
        .with_context(|| crate::types::WIKI_DATA.to_string())?;
    let wiki_decls =
        wowlua_types_gen::luals::convert(&[(crate::types::WIKI_DATA.to_string(), wiki_text)]);
    let documented: BTreeMap<&str, &wowlua_engine::api::Function> =
        api.functions.iter().rev().map(|f| (f.name, f)).collect();
    let mut by_namespace: BTreeMap<String, (Counts, BTreeSet<&str>)> = BTreeMap::new();
    let mut function_list = Vec::new();
    for entry in functions.iter().filter(|e| e.group != Group::EngineOnly) {
        let f = documented.get(entry.name.as_str()).copied();
        let namespace = match f {
            Some(f) => f.namespace.unwrap_or("").to_string(),
            None => entry
                .name
                .split_once('.')
                .map_or(String::new(), |(ns, _)| ns.to_string()),
        };
        let ns = by_namespace.entry(namespace.clone()).or_default();
        ns.0.add(entry.source);
        if let Some(f) = f {
            ns.1.insert(f.system);
        }
        // Blizzard's descriptions name a function by its system's
        // namespace (`C_PvP.GetArenaOpponentSpec` of a global).
        let key = match f {
            Some(f) if !d.merged.contains_key(f.name) => match systems
                .iter()
                .find(|s| text(s, "Name").as_deref() == Some(f.system))
                .and_then(|s| text(s, "Namespace"))
            {
                Some(ns) => format!("{ns}.{}", f.short),
                None => f.name.to_string(),
            },
            _ => entry.name.clone(),
        };
        let mut out = Map::new();
        out.insert("name".into(), json!(entry.name));
        out.insert("namespace".into(), json!(namespace));
        out.insert("system".into(), json!(f.map(|f| f.system)));
        out.insert("documented".into(), json!(f.is_some()));
        let wiki = match f {
            Some(_) => None,
            None => wiki_signature(&wiki_decls, &entry.name),
        };
        let doc = f.map(|f| &f.doc).or(wiki.as_ref());
        out.extend(callable(&d, &key, doc, wiki.is_some(), entry));
        function_list.push(Json::Object(out));
    }

    // Namespaces: the functions', and the documentation's systems with
    // one.
    let namespace_docs: BTreeMap<String, Vec<String>> = {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for s in systems {
            if let (Some(ns), Some(doc)) = (text(s, "Namespace"), documentation(s)) {
                m.entry(ns).or_default().push(doc);
            }
        }
        m
    };
    let mut system_names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in systems {
        if text(s, "Type").as_deref() == Some("System")
            && let (Some(ns), Some(name)) = (text(s, "Namespace"), text(s, "Name"))
            && !ns.is_empty()
        {
            system_names.entry(ns.clone()).or_default().insert(name);
            by_namespace.entry(ns).or_default();
        }
    }
    let namespaces: Vec<Json> = by_namespace
        .iter()
        .map(|(name, (c, systems))| {
            let mut all: BTreeSet<String> = systems.iter().map(|s| s.to_string()).collect();
            if let Some(more) = system_names.get(name) {
                all.extend(more.iter().cloned());
            }
            json!({
                "name": name,
                "systems": all.into_iter().collect::<Vec<_>>(),
                "doc": namespace_docs.get(name).map(|d| d.join(" ")),
                "counts": counts_json(c),
            })
        })
        .collect();

    // Events, with their systems.
    let mut event_systems: BTreeMap<String, (String, Option<String>, &Json)> = BTreeMap::new();
    for s in systems {
        let name = text(s, "Name").unwrap_or_default();
        let ns = text(s, "Namespace").filter(|n| !n.is_empty());
        for e in list(s, "Events") {
            if let Some(literal) = text(e, "LiteralName") {
                event_systems
                    .entry(literal)
                    .or_insert((name.clone(), ns.clone(), e));
            }
        }
    }
    let mut event_counts = (0, 0);
    let event_list: Vec<Json> = events
        .iter()
        .map(|e| {
            if e.fired {
                event_counts.0 += 1;
            } else {
                event_counts.1 += 1;
            }
            let documented = event_systems.get(&e.name);
            let key = format!("event:{}", e.name);
            let mut sources = BTreeSet::new();
            let items = documented.map_or(&[][..], |(_, _, doc)| list(doc, "Payload"));
            let payload = described_values(&d, &key, items, false, &mut sources);
            let doc = description(&d, &key, d.merged.get(&key), sources);
            json!({
                "name": e.name,
                "namespace": documented.and_then(|(_, ns, _)| ns.clone()),
                "system": documented.map(|(s, _, _)| s),
                "documented": e.group == Group::Documented,
                "payload": payload,
                "flags": documented.map_or_else(Map::new, |(_, _, doc)| {
                    flags(doc, &["Name", "LiteralName", "Type", "Payload", "Documentation"])
                }),
                "doc": doc,
                "status": if e.fired { "implemented" } else { "not implemented" },
                "modules": e.modules,
            })
        })
        .collect();

    // Widgets: the classes with methods or a frame type, and their bases;
    // not the lower- and upper-case names `CreateFrame` takes.
    let method_entries: BTreeMap<&str, &Entry> =
        methods.iter().map(|m| (m.name.as_str(), m)).collect();
    let alias = |name: &str, c: &wowlua_engine::api::Class| matches!(c.bases.as_slice(), [b] if b != name && b.eq_ignore_ascii_case(name));
    let mut widget_names: BTreeSet<&str> = api
        .classes
        .iter()
        .filter(|(n, c)| !alias(n, c) && (!c.methods.is_empty() || c.widget.is_some()))
        .map(|(n, _)| n.as_str())
        .collect();
    let mut queue: Vec<&str> = widget_names.iter().copied().collect();
    while let Some(name) = queue.pop() {
        for base in &api.classes[name].bases {
            if let Some((k, _)) = api.classes.get_key_value(base)
                && widget_names.insert(k.as_str())
            {
                queue.push(k.as_str());
            }
        }
    }
    let mut method_counts = Counts::default();
    let widgets: Vec<Json> = widget_names
        .iter()
        .map(|&name| {
            let class = &api.classes[name];
            let mut seen = BTreeSet::new();
            let methods: Vec<Json> = class
                .methods
                .iter()
                .map(|&i| &api.methods[i])
                .filter(|f| seen.insert(f.name))
                .filter_map(|f| {
                    let entry = method_entries.get(f.name)?;
                    method_counts.add(entry.source);
                    let mut out = Map::new();
                    out.insert("name".into(), json!(f.short));
                    out.extend(callable(&d, f.name, Some(&f.doc), false, entry));
                    Some(Json::Object(out))
                })
                .collect();
            json!({
                "name": name,
                "bases": class.bases,
                "widget": class.widget,
                "methods": methods,
            })
        })
        .collect();

    // Tables: structures, enumerations, constants and callback types, the
    // first system's of a name.
    let mut tables = Map::new();
    for s in systems {
        let system = text(s, "Name");
        for t in list(s, "Tables") {
            let Some(name) = text(t, "Name") else {
                continue;
            };
            if tables.contains_key(&name) {
                continue;
            }
            let kind = text(t, "Type").unwrap_or_default();
            let mut out = Map::new();
            out.insert("kind".into(), json!(kind));
            out.insert("system".into(), json!(system));
            match kind.as_str() {
                "Structure" => {
                    out.insert(
                        "fields".into(),
                        list(t, "Fields").iter().map(|v| value(v, None)).collect(),
                    );
                }
                "Enumeration" => {
                    out.insert(
                        "values".into(),
                        list(t, "Fields")
                            .iter()
                            .map(|f| json!({"name": text(f, "Name"), "value": f.get("EnumValue")}))
                            .collect(),
                    );
                }
                "Constants" => {
                    out.insert(
                        "values".into(),
                        list(t, "Values")
                            .iter()
                            .map(|v| {
                                json!({
                                    "name": text(v, "Name"),
                                    "type": text(v, "Type"),
                                    "value": v.get("Value"),
                                })
                            })
                            .collect(),
                    );
                }
                _ => {
                    out.insert(
                        "args".into(),
                        list(t, "Arguments")
                            .iter()
                            .map(|v| value(v, None))
                            .collect(),
                    );
                }
            }
            if let Some(doc) = documentation(t) {
                out.insert("doc".into(), json!(doc));
            }
            tables.insert(name, Json::Object(out));
        }
    }

    let documented_counts = counts(functions.iter().filter(|e| e.group == Group::Documented));
    let undocumented_counts = counts(functions.iter().filter(|e| e.group == Group::Undocumented));
    assert_eq!(
        method_counts.total(),
        methods.len(),
        "every method on a widget"
    );
    Ok(json!({
        "flavor": FLAVOR,
        "client": client(root)?,
        "sources": {
            "blizzard": "Blizzard's API documentation (Blizzard_APIDocumentationGenerated)",
            "wiki": "warcraft.wiki.gg's API pages, by its contributors, CC BY-SA 4.0 (https://creativecommons.org/licenses/by-sa/4.0/)",
            "wowlua": "written by wowlua where no documentation describes the entry",
        },
        "summary": {
            "functions": counts_json(&documented_counts),
            "undocumented": counts_json(&undocumented_counts),
            "methods": counts_json(&counts(methods.iter())),
            "events": {
                "total": events.len(),
                "implemented": event_counts.0,
                "not implemented": event_counts.1,
            },
            "namespaces": by_namespace.keys().filter(|n| !n.is_empty()).count(),
            "probes": { "exact": exact, "total": probed },
        },
        "namespaces": namespaces,
        "functions": function_list,
        "events": event_list,
        "widgets": widgets,
        "tables": tables,
    }))
}
