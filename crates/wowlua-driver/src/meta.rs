//! The values of `wowlua:meta` for a build (spec chapter 6) and their
//! runtime tables in `wowlua/runtime.lua`.

use crate::project::Project;
use crate::vcs::VcsInfo;
use indexmap::IndexMap;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use wowlua_manifest::Profile;
use wowlua_manifest::model::MetaValue as ManifestMeta;
use wowlua_sema::meta::{Meta, MetaKind, MetaValue};

/// What [`build_meta`] needs besides the project.
pub struct MetaInput<'a> {
    pub profile: &'a Profile,
    /// The flavor of this compilation (`None` in shared flavor mode).
    pub flavor: Option<&'a str>,
    /// Every targeted flavor.
    pub flavors: &'a [String],
    pub version: &'a str,
    pub build_id: &'a str,
    pub vcs: Option<&'a VcsInfo>,
    /// Asset paths relative to the package root.
    pub assets: &'a [String],
    pub addon_name: &'a str,
}

fn json(value: &Value) -> MetaValue {
    match value {
        Value::Null => MetaValue::Nil,
        Value::Bool(b) => MetaValue::Bool(*b),
        Value::Number(n) => MetaValue::Number(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => MetaValue::String(s.clone()),
        Value::Array(items) => MetaValue::List(items.iter().map(json).collect()),
        Value::Object(map) => {
            MetaValue::Table(map.iter().map(|(k, v)| (k.clone(), json(v))).collect())
        }
    }
}

/// `experimental-ui` → `experimentalUi` (spec §9.6).
fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        if c == '-' || c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Days since 1970-01-01 → (year, month, day) (proleptic Gregorian).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn iso_time(secs: i64) -> String {
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The `wowlua:meta` values of one build configuration and flavor.
pub fn build_meta(project: &Project, input: &MetaInput) -> Meta {
    let manifest = &project.manifest;
    let package = manifest.package();
    let addon = manifest.manifest.addon.clone().unwrap_or_default();
    let deps = manifest
        .manifest
        .addon_dependencies
        .clone()
        .unwrap_or_default();
    let applies = |flavors: &[String]| {
        flavors.is_empty()
            || input.flavor.is_none_or(|f| {
                flavors.iter().any(|x| {
                    wowlua_flavors::table()
                        .resolve(x)
                        .is_some_and(|fl| fl.id == f)
                })
            })
    };
    let interface: Vec<MetaValue> = match input.flavor {
        Some(f) => project.interfaces.get(f).cloned().unwrap_or_default(),
        None => input
            .flavors
            .iter()
            .flat_map(|f| project.interfaces.get(f).cloned().unwrap_or_default())
            .collect(),
    }
    .into_iter()
    .map(|n| MetaValue::Number(f64::from(n)))
    .collect();
    let mut addon_table: IndexMap<String, MetaValue> = IndexMap::new();
    addon_table.insert("name".into(), MetaValue::str(input.addon_name));
    addon_table.insert(
        "package".into(),
        MetaValue::opt_str(package.map(|p| p.name.clone())),
    );
    addon_table.insert("title".into(), MetaValue::str(manifest.title()));
    addon_table.insert("notes".into(), MetaValue::opt_str(addon.notes.clone()));
    addon_table.insert(
        "version".into(),
        MetaValue::opt_str(package.map(|p| p.version.clone())),
    );
    addon_table.insert(
        "authors".into(),
        MetaValue::strings(package.map(|p| p.authors.clone()).unwrap_or_default()),
    );
    addon_table.insert(
        "license".into(),
        MetaValue::opt_str(package.and_then(|p| p.license.clone())),
    );
    addon_table.insert(
        "homepage".into(),
        MetaValue::opt_str(package.and_then(|p| p.homepage.clone())),
    );
    addon_table.insert("interface".into(), MetaValue::List(interface));
    addon_table.insert(
        "flavors".into(),
        MetaValue::strings(input.flavors.iter().cloned()),
    );
    // A flavor's own lists in split mode (decision T128).
    let own = input.flavor.and_then(|f| {
        addon.flavor_overrides.iter().find_map(|(name, o)| {
            wowlua_flavors::table()
                .resolve(name)
                .is_some_and(|fl| fl.id == f)
                .then_some(o)
        })
    });
    addon_table.insert(
        "savedVariables".into(),
        MetaValue::strings(
            own.and_then(|o| o.saved_variables.clone())
                .unwrap_or_else(|| addon.saved_variables.clone()),
        ),
    );
    addon_table.insert(
        "savedVariablesPerCharacter".into(),
        MetaValue::strings(
            own.and_then(|o| o.saved_variables_per_character.clone())
                .unwrap_or_else(|| addon.saved_variables_per_character.clone()),
        ),
    );
    let names = |list: &[wowlua_manifest::model::AddonDependency]| {
        MetaValue::strings(
            list.iter()
                .filter(|d| applies(d.flavors()))
                .map(|d| d.name().to_string()),
        )
    };
    addon_table.insert(
        "dependencies".into(),
        MetaValue::Table(IndexMap::from([
            ("required".to_string(), names(&deps.required)),
            ("optional".to_string(), names(&deps.optional)),
        ])),
    );
    addon_table.insert(
        "meta".into(),
        MetaValue::Table(
            addon
                .meta
                .iter()
                .map(|(k, v)| {
                    let value = match v {
                        ManifestMeta::Bool(b) => MetaValue::Bool(*b),
                        ManifestMeta::Number(n) => MetaValue::Number(n.as_f64().unwrap_or(0.0)),
                        ManifestMeta::String(s) => MetaValue::String(s.clone()),
                    };
                    (k.clone(), value)
                })
                .collect(),
        ),
    );

    let now = crate::host::unix_now();
    let mut build: IndexMap<String, MetaValue> = IndexMap::new();
    build.insert("profile".into(), MetaValue::str(&input.profile.name));
    build.insert("debug".into(), MetaValue::Bool(input.profile.debug));
    build.insert("version".into(), MetaValue::str(input.version));
    build.insert(
        "commit".into(),
        MetaValue::opt_str(input.vcs.map(|v| v.short_hash.clone())),
    );
    build.insert(
        "dirty".into(),
        MetaValue::Bool(input.vcs.is_some_and(|v| v.dirty)),
    );
    build.insert("time".into(), MetaValue::str(iso_time(now)));
    build.insert("timestamp".into(), MetaValue::Number(now as f64));
    build.insert("compiler".into(), MetaValue::str(crate::COMPILER_VERSION));
    build.insert("id".into(), MetaValue::str(input.build_id));
    build.insert("bundled".into(), MetaValue::Bool(input.profile.bundle));
    build.insert(
        "minify".into(),
        MetaValue::str(input.profile.minify.clone()),
    );
    build.insert("target".into(), MetaValue::Nil);

    let (enabled, _) = project.enabled_features();
    let features: IndexMap<String, MetaValue> = manifest
        .manifest
        .features
        .keys()
        .filter(|k| k.as_str() != "default")
        .map(|k| (camel(k), MetaValue::Bool(enabled.contains(k))))
        .collect();

    let prefix = format!("Interface\\AddOns\\{}\\", input.addon_name);
    let assets: HashMap<String, String> = input
        .assets
        .iter()
        .map(|a| (a.clone(), format!("{prefix}{}", a.replace('/', "\\"))))
        .collect();

    Meta {
        addon: MetaValue::Table(addon_table),
        build: MetaValue::Table(build),
        config: json(&Value::Object(input.profile.config.clone())),
        features: MetaValue::Table(features),
        flavor: input.flavor.map(str::to_string),
        assets,
    }
}

/// The `wowlua:meta` of a wowlua library (spec §10.5.1): `addon`, `build`
/// and `flavor` describe the consuming addon's build; `features` and
/// `config` are the library's own, enabled and overridden by the consumer's
/// dependency entry.
pub fn library_meta(
    base: &Meta,
    library: &wowlua_manifest::Manifest,
    entry: &wowlua_manifest::model::DependencyDetail,
) -> Meta {
    let mut enabled = BTreeSet::new();
    let mut stack: Vec<String> = entry.features.clone();
    if entry.default_features {
        stack.extend(library.features.get("default").cloned().unwrap_or_default());
    }
    while let Some(f) = stack.pop() {
        if f.starts_with("dep:") || !enabled.insert(f.clone()) {
            continue;
        }
        if let Some(more) = library.features.get(&f) {
            stack.extend(more.iter().cloned());
        }
    }
    let features: IndexMap<String, MetaValue> = library
        .features
        .keys()
        .filter(|k| k.as_str() != "default")
        .map(|k| (camel(k), MetaValue::Bool(enabled.contains(k))))
        .collect();
    fn merge(base: &mut Value, over: &Value) {
        match (base, over) {
            (Value::Object(b), Value::Object(o)) => {
                for (k, v) in o {
                    merge(b.entry(k.clone()).or_insert(Value::Null), v);
                }
            }
            (b, o) => *b = o.clone(),
        }
    }
    let mut config = Value::Object(library.config.clone());
    merge(&mut config, &Value::Object(entry.config.clone()));
    Meta {
        features: MetaValue::Table(features),
        config: json(&config),
        ..base.clone()
    }
}

/// Keeps the parts of `value` reached by `paths` (each a path below it; an
/// empty path keeps everything).
fn shake(value: &MetaValue, paths: &[&[String]]) -> MetaValue {
    if paths.iter().any(|p| p.is_empty()) {
        return value.clone();
    }
    match value {
        MetaValue::Table(fields) => {
            let mut out = IndexMap::new();
            for (k, v) in fields {
                let below: Vec<&[String]> = paths
                    .iter()
                    .filter(|p| p[0] == *k)
                    .map(|p| &p[1..])
                    .collect();
                if !below.is_empty() {
                    out.insert(k.clone(), shake(v, &below));
                }
            }
            MetaValue::Table(out)
        }
        other => other.clone(),
    }
}

/// Lua statements creating the runtime tables (`wl.meta`) for the used
/// paths (all of them with `"meta": { "runtime": "full" }`).
pub fn runtime_tables(
    meta: &Meta,
    used: &BTreeSet<(MetaKind, Vec<String>)>,
    full: bool,
    flavors: &[String],
    interfaces: &std::collections::BTreeMap<String, Vec<u32>>,
) -> Option<String> {
    if used.is_empty() && !full {
        return None;
    }
    let mut out = String::from("wl.meta = wl.meta or {}\n");
    let shared = meta.flavor.is_none();
    if shared && (full || used.iter().any(|(k, _)| *k == MetaKind::Flavor)) {
        // Shared mode: the flavor from WOW_PROJECT_ID (spec §6.3).
        out.push_str("wl.meta.flavor = ({");
        for (i, f) in flavors.iter().enumerate() {
            if let Some(flavor) = wowlua_flavors::table().flavor(f) {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("[{}] = \"{}\"", flavor.project_id, f));
            }
        }
        out.push_str("})[WOW_PROJECT_ID]\n");
    }
    for (kind, value) in [
        (MetaKind::Addon, &meta.addon),
        (MetaKind::Build, &meta.build),
        (MetaKind::Config, &meta.config),
        (MetaKind::Features, &meta.features),
    ] {
        let paths: Vec<&[String]> = used
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, p)| p.as_slice())
            .collect();
        if paths.is_empty() && !full {
            continue;
        }
        let kept = if full {
            value.clone()
        } else {
            shake(value, &paths)
        };
        let mut text = String::new();
        kept.to_lua(&mut text);
        out.push_str(&format!("wl.meta.{} = {text}\n", kind.name()));
        if shared
            && kind == MetaKind::Addon
            && (full || paths.iter().any(|p| p.is_empty() || p[0] == "interface"))
        {
            // The current flavor's interface versions, chosen at run time.
            let mut table = String::from("{");
            for f in flavors {
                let numbers: Vec<String> = interfaces
                    .get(f)
                    .map(|v| v.iter().map(u32::to_string).collect())
                    .unwrap_or_default();
                table.push_str(&format!("[\"{f}\"] = {{{}}}, ", numbers.join(", ")));
            }
            table.push('}');
            out.push_str(&format!(
                "wl.meta.addon.interface = ({table})[wl.meta.flavor]\n"
            ));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(camel("experimental-ui"), "experimentalUi");
        assert_eq!(iso_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_time(1_791_676_800), "2026-10-11T00:00:00Z");
    }
}
