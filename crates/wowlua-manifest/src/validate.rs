//! Semantic validation of a manifest that already matches the schema.

use crate::load::{closest, span_of};
use crate::locate::JsonPath;
use crate::model::{Dependency, Manifest, PackageKind, TargetType};
use crate::profile::{self, BuiltinProfile, ProfileError};
use crate::{BUILTIN_MINIFY_LEVELS, MINIFY_PASSES};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::sync::LazyLock;
use wowlua_diagnostics::{Diagnostic, Diagnostics};
use wowlua_span::FileId;

static PACKAGE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9-]{2,64}$").unwrap());
/// Any valid folder name: printable, none of the characters Windows forbids
/// in file names, no leading or trailing space or dot (`!BugGrabber`).
static ADDON_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^[^\s.\\/:*?"<>|\p{Cc}]([^\\/:*?"<>|\p{Cc}]*[^\s.\\/:*?"<>|\p{Cc}])?$"#).unwrap()
});
static LUA_IDENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap());

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// TOC directives wowlua generates from other manifest keys. They must not be
/// set through `addon.toc-extra`.
const GENERATED_DIRECTIVES: &[&str] = &[
    "Interface",
    "Title",
    "Notes",
    "Author",
    "Version",
    "Category",
    "Group",
    "IconTexture",
    "IconAtlas",
    "LoadOnDemand",
    "DefaultState",
    "SavedVariables",
    "SavedVariablesPerCharacter",
    "LoadSavedVariablesFirst",
    "AllowAddOnTableAccess",
    "OnlyBetaAndPTR",
    "Dependencies",
    "RequiredDeps",
    "OptionalDeps",
    "LoadWith",
    "LoadManagers",
    "X-Wowlua-Build",
];

/// Other directives the client understands that are not generated.
const OTHER_KNOWN_DIRECTIVES: &[&str] = &[
    "AddonCompartmentFunc",
    "AddonCompartmentFuncOnEnter",
    "AddonCompartmentFuncOnLeave",
    "AllowLoadGameType",
    "ExcludeLoadGameType",
];

struct Ctx<'a> {
    text: &'a str,
    file: FileId,
    out: &'a mut Diagnostics,
}

impl Ctx<'_> {
    fn error(&mut self, code: &'static str, path: &JsonPath, message: String, label: &str) {
        let span = span_of(self.text, self.file, path, false);
        self.out
            .push(Diagnostic::error(code, message).with_label(span, label));
    }

    fn push(&mut self, diagnostic: Diagnostic) {
        self.out.push(diagnostic);
    }

    fn at(&self, path: &JsonPath) -> wowlua_span::Span {
        span_of(self.text, self.file, path, false)
    }
}

/// Runs all semantic checks and returns the normalized flavor IDs.
pub(crate) fn validate(
    manifest: &Manifest,
    raw: &Value,
    text: &str,
    file: FileId,
    out: &mut Diagnostics,
) -> Vec<String> {
    let mut ctx = Ctx { text, file, out };
    package(manifest, &mut ctx);
    addon(manifest, raw, &mut ctx);
    let flavors = flavors(manifest, &mut ctx);
    flavor_references(manifest, &mut ctx);
    dependencies(manifest, &mut ctx);
    external_addons(manifest, &mut ctx);
    features(manifest, &mut ctx);
    profiles(manifest, &mut ctx);
    minify_levels(manifest, &mut ctx);
    publish(manifest, &mut ctx);
    globs(manifest, &mut ctx);
    lints(manifest, &mut ctx);
    flavors
}

fn root() -> JsonPath {
    JsonPath::root()
}

fn external_addons(manifest: &Manifest, ctx: &mut Ctx) {
    let Some(ws) = &manifest.workspace else {
        return;
    };
    for (name, d) in &ws.external_addons {
        let path = root().key("workspace").key("external-addons").key(name);
        let sources = [
            d.git.is_some(),
            d.svn.is_some(),
            d.path.is_some(),
            d.archive.is_some(),
        ]
        .iter()
        .filter(|s| **s)
        .count();
        if sources != 1 {
            ctx.error(
                "E4002",
                &path,
                format!("external addon `{name}` must name exactly one source: `git`, `svn`, `path` or `archive`"),
                "one source",
            );
        }
        if d.archive.is_some() && d.sha256.is_none() {
            ctx.error(
                "E4002",
                &path,
                format!("archive external addon `{name}` needs a `sha256` checksum"),
                "missing checksum",
            );
        }
        if name.is_empty() || name.contains(['/', '\\']) {
            ctx.error(
                "E4002",
                &path,
                format!("`{name}` is not an AddOn folder name"),
                "invalid name",
            );
        }
    }
}

fn package(manifest: &Manifest, ctx: &mut Ctx) {
    let Some(package) = &manifest.package else {
        if manifest.workspace.is_none() {
            ctx.error(
                "E4002",
                &root(),
                "missing `package` section".into(),
                "required unless this is a workspace root",
            );
        }
        return;
    };
    let path = root().key("package");
    if !PACKAGE_NAME.is_match(&package.name) {
        ctx.push(
            Diagnostic::error("E4003", format!("invalid package name `{}`", package.name))
                .with_label(ctx.at(&path.clone().key("name")), "invalid name")
                .with_help("use 2–64 lowercase letters, digits and `-`"),
        );
    }
    if let Err(error) = semver::Version::parse(&package.version) {
        ctx.error(
            "E4003",
            &path.key("version"),
            format!("invalid package version `{}`: {error}", package.version),
            "not a SemVer 2.0 version",
        );
    }
}

fn addon(manifest: &Manifest, raw: &Value, ctx: &mut Ctx) {
    let kind = manifest
        .package
        .as_ref()
        .map(|p| p.kind)
        .unwrap_or_default();
    let Some(addon) = &manifest.addon else { return };
    let path = root().key("addon");
    if kind != PackageKind::Addon {
        ctx.error(
            "E4317",
            &path,
            "the `addon` section is only valid for packages of kind `addon`".into(),
            "not allowed here",
        );
    }
    if let Some(name) = &addon.name
        && !ADDON_NAME.is_match(name)
    {
        ctx.error(
            "E4002",
            &path.clone().key("name"),
            format!(
                "invalid AddOn name `{name}`: it must be a valid folder name (no \\ / : * ? \" < > |, no leading or trailing space or dot)"
            ),
            "invalid name",
        );
    }

    let mut seen = HashSet::new();
    for (key, list) in [
        ("saved-variables", &addon.saved_variables),
        (
            "saved-variables-per-character",
            &addon.saved_variables_per_character,
        ),
    ] {
        for (i, name) in list.iter().enumerate() {
            let item = path.clone().key(key).index(i);
            if !LUA_IDENT.is_match(name) || LUA_KEYWORDS.contains(&name.as_str()) {
                ctx.error(
                    "E4002",
                    &item,
                    format!("SavedVariable `{name}` is not a valid Lua identifier"),
                    "invalid name",
                );
            } else if !seen.insert(name.as_str()) {
                ctx.error(
                    "E4002",
                    &item,
                    format!("SavedVariable `{name}` is declared twice"),
                    "duplicate",
                );
            }
        }
    }

    if let Some(expose) = &addon.expose_meta
        && !LUA_IDENT.is_match(expose)
    {
        ctx.error(
            "E4002",
            &path.clone().key("expose-meta"),
            format!("`{expose}` is not a valid Lua global name"),
            "invalid name",
        );
    }

    let explicit_not_lod = raw.pointer("/addon/load-on-demand") == Some(&Value::Bool(false));
    let has_load_with = manifest
        .addon_dependencies
        .as_ref()
        .is_some_and(|d| !d.load_with.is_empty());
    if explicit_not_lod && has_load_with {
        ctx.push(
            Diagnostic::error(
                "E4312",
                "`addon.load-on-demand` is false, but `addon-dependencies.load-with` makes the addon load on demand",
            )
            .with_label(ctx.at(&path.clone().key("load-on-demand")), "set to false here")
            .with_label(ctx.at(&root().key("addon-dependencies").key("load-with")), "implies load on demand"),
        );
    }

    let targeted: Vec<String> = manifest
        .flavors
        .iter()
        .flatten()
        .filter_map(|(name, _)| wowlua_flavors::table().resolve(name).map(|f| f.id.clone()))
        .collect();
    for (flavor, overrides) in &addon.flavor_overrides {
        let fpath = path.clone().key("flavor-overrides").key(flavor);
        if let Some(id) = check_flavor(flavor, &fpath, ctx) {
            let listed = if manifest.flavors.is_some() {
                targeted.contains(&id)
            } else {
                id == "standard"
            };
            if !listed {
                ctx.error(
                    "E4002",
                    &fpath,
                    format!("`flavor-overrides` names `{id}`, which is not listed in `flavors`"),
                    "not a targeted flavor",
                );
            }
        }
        for key in overrides.toc_extra.keys() {
            check_toc_extra_key(key, &fpath.clone().key("toc-extra").key(key), ctx);
        }
    }

    for key in addon.toc_extra.keys() {
        check_toc_extra_key(key, &path.clone().key("toc-extra").key(key), ctx);
    }

    for flavor in addon.toc_data.keys() {
        let fpath = path.clone().key("toc-data").key(flavor);
        if let Some(id) = check_flavor(flavor, &fpath, ctx) {
            let listed = if manifest.flavors.is_some() {
                targeted.contains(&id)
            } else {
                id == "standard"
            };
            if !listed {
                ctx.error(
                    "E4002",
                    &fpath,
                    format!("`toc-data` names `{id}`, which is not listed in `flavors`"),
                    "not a targeted flavor",
                );
            }
        }
    }
}

/// Whether wowlua generates this TOC directive itself (so it must not come
/// from `toc-extra` or `toc-data`). Localized variants count for the
/// localizable directives only (`Title-deDE`, not `IconTexture-blank`).
pub fn is_generated_directive(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    GENERATED_DIRECTIVES
        .iter()
        .any(|d| lower == d.to_ascii_lowercase())
        || ["title-", "notes-", "category-"]
            .iter()
            .any(|p| lower.starts_with(p))
}

fn check_toc_extra_key(key: &str, item: &JsonPath, ctx: &mut Ctx) {
    {
        let generated = is_generated_directive(key);
        if generated {
            ctx.error(
                "E4002",
                item,
                format!("TOC directive `{key}` is generated by wowlua; set it through the manifest instead"),
                "generated directive",
            );
        } else if !key.starts_with("X-")
            && !OTHER_KNOWN_DIRECTIVES
                .iter()
                .any(|d| d.eq_ignore_ascii_case(key))
        {
            ctx.push(
                Diagnostic::warning(
                    "W4001",
                    format!("unknown TOC directive `{key}` in `addon.toc-extra`"),
                )
                .with_label(ctx.at(item), "unknown directive")
                .with_help("custom metadata fields should start with `X-`"),
            );
        }
    }
}

fn known_flavor_ids() -> String {
    wowlua_flavors::table()
        .flavors
        .iter()
        .map(|f| f.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn check_flavor(name: &str, path: &JsonPath, ctx: &mut Ctx) -> Option<String> {
    match wowlua_flavors::table().resolve(name) {
        Some(flavor) => Some(flavor.id.clone()),
        None => {
            ctx.push(
                Diagnostic::error("E4004", format!("unknown flavor `{name}`"))
                    .with_label(ctx.at(path), "unknown flavor")
                    .with_note(format!("known flavors: {}", known_flavor_ids())),
            );
            None
        }
    }
}

fn flavors(manifest: &Manifest, ctx: &mut Ctx) -> Vec<String> {
    let kind = manifest
        .package
        .as_ref()
        .map(|p| p.kind)
        .unwrap_or_default();
    let Some(entries) = &manifest.flavors else {
        return if kind == PackageKind::Addon {
            vec!["standard".to_string()]
        } else {
            wowlua_flavors::table()
                .flavors
                .iter()
                .map(|f| f.id.clone())
                .collect()
        };
    };
    let mut ids: Vec<String> = Vec::new();
    for name in entries.keys() {
        let path = root().key("flavors").key(name);
        if let Some(id) = check_flavor(name, &path, ctx) {
            if ids.contains(&id) {
                ctx.error(
                    "E4002",
                    &path,
                    format!("flavor `{id}` is listed twice (`{name}` is an alias of `{id}`)"),
                    "duplicate flavor",
                );
            } else {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() && !entries.is_empty() {
        return ids;
    }
    if entries.is_empty() {
        ctx.error(
            "E4002",
            &root().key("flavors"),
            "`flavors` must list at least one flavor".into(),
            "empty",
        );
    }
    ids
}

fn flavor_references(manifest: &Manifest, ctx: &mut Ctx) {
    if let Some(deps) = &manifest.addon_dependencies {
        for (key, list) in [("required", &deps.required), ("optional", &deps.optional)] {
            for (i, dep) in list.iter().enumerate() {
                for (j, flavor) in dep.flavors().iter().enumerate() {
                    let path = root()
                        .key("addon-dependencies")
                        .key(key)
                        .index(i)
                        .key("flavors")
                        .index(j);
                    check_flavor(flavor, &path, ctx);
                }
            }
        }
    }
    for (i, entry) in manifest.build.variable_files.iter().enumerate() {
        let epath = root().key("build").key("variable-files").index(i);
        let variables: Vec<&&str> = crate::model::TOC_VARIABLES
            .iter()
            .filter(|v| {
                entry
                    .path
                    .to_ascii_lowercase()
                    .contains(&v.to_ascii_lowercase())
            })
            .collect();
        // A line without variables but with `locales` loads one file for
        // some client locales (`deDE.lua [AllowLoadTextLocale deDE]`).
        if variables.is_empty() && entry.locales.is_none() {
            ctx.error(
                "E4002",
                &epath.clone().key("path"),
                format!(
                    "`{}` contains no TOC variable ([Game], [Family] or [TextLocale]) and no `locales`",
                    entry.path
                ),
                "no variable",
            );
        }
        if let Some(locales) = &entry.locales {
            for (j, locale) in locales.iter().enumerate() {
                if !crate::model::LOCALES.contains(&locale.as_str()) {
                    ctx.error(
                        "E4002",
                        &epath.clone().key("locales").index(j),
                        format!("unknown locale `{locale}`"),
                        "unknown locale",
                    );
                }
            }
        }
        for (j, flavor) in entry.flavors.iter().flatten().enumerate() {
            check_flavor(flavor, &epath.clone().key("flavors").index(j), ctx);
        }
    }
    for (i, entry) in manifest.build.external_files.iter().enumerate() {
        let epath = root().key("build").key("external-files").index(i);
        let valid = entry
            .path
            .strip_prefix("../")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(addon, file)| {
                !addon.is_empty()
                    && ADDON_NAME.is_match(addon)
                    && !file.is_empty()
                    && !file.split('/').any(|p| p == "..")
            });
        if !valid {
            ctx.error(
                "E4002",
                &epath.clone().key("path"),
                format!("`{}` must have the form `../<AddOn>/<path>`", entry.path),
                "not a file of another AddOn",
            );
        }
        for (j, flavor) in entry.flavors.iter().flatten().enumerate() {
            check_flavor(flavor, &epath.clone().key("flavors").index(j), ctx);
        }
    }
    for (i, path) in manifest.build.shared_files.iter().enumerate() {
        if path.starts_with('/') || path.split('/').any(|p| p == "..") {
            ctx.error(
                "E4002",
                &root().key("build").key("shared-files").index(i),
                format!("`{path}` must be a path inside the package"),
                "outside the package",
            );
        }
    }
    for (glob, flavors) in &manifest.build.flavor_files {
        for (j, flavor) in flavors.iter().enumerate() {
            let path = root().key("build").key("flavor-files").key(glob).index(j);
            check_flavor(flavor, &path, ctx);
        }
    }
}

fn dependencies(manifest: &Manifest, ctx: &mut Ctx) {
    for (section, deps) in [
        ("dependencies", &manifest.dependencies),
        ("dev-dependencies", &manifest.dev_dependencies),
    ] {
        for (name, dep) in deps {
            let path = root().key(section).key(name);
            if !PACKAGE_NAME.is_match(name) {
                ctx.error(
                    "E4003",
                    &path,
                    format!("invalid dependency name `{name}`"),
                    "invalid name",
                );
            }
            match dep {
                Dependency::Version(req) => {
                    if let Err(error) = semver::VersionReq::parse(req) {
                        ctx.error(
                            "E4002",
                            &path,
                            format!("invalid version requirement `{req}`: {error}"),
                            "invalid requirement",
                        );
                    }
                }
                Dependency::Detailed(detail) => {
                    let sources = [
                        detail.version.is_some() || detail.registry.is_some(),
                        detail.git.is_some(),
                        detail.svn.is_some(),
                        detail.path.is_some(),
                        detail.archive.is_some(),
                    ];
                    let count = sources.iter().filter(|s| **s).count();
                    // With `compat.pkgmeta`, `.pkgmeta` supplies the source
                    // of its externals.
                    if count != 1
                        && !(count == 0 && manifest.compat.pkgmeta && section == "dependencies")
                    {
                        ctx.error(
                            "E4002",
                            &path,
                            format!("dependency `{name}` must name exactly one source: `version`, `git`, `svn`, `path` or `archive`"),
                            if count == 0 { "no source" } else { "several sources" },
                        );
                    }
                    let refs = [&detail.tag, &detail.branch, &detail.rev]
                        .iter()
                        .filter(|r| r.is_some())
                        .count();
                    if detail.git.is_some() && refs > 1 {
                        ctx.error(
                            "E4002",
                            &path,
                            format!(
                                "dependency `{name}`: use only one of `tag`, `branch` and `rev`"
                            ),
                            "conflicting git refs",
                        );
                    }
                    if detail.git.is_none()
                        && (detail.tag.is_some()
                            || detail.branch.is_some()
                            || (detail.rev.is_some() && detail.svn.is_none()))
                    {
                        ctx.error(
                            "E4002",
                            &path,
                            format!("dependency `{name}`: `tag` and `branch` require `git`, `rev` requires `git` or `svn`"),
                            "ref without git",
                        );
                    }
                    if let (Some(_), Some(rev)) = (&detail.svn, &detail.rev)
                        && rev.parse::<u64>().is_err()
                    {
                        ctx.error(
                            "E4002",
                            &path.clone().key("rev"),
                            format!("dependency `{name}`: an SVN `rev` is a revision number"),
                            "not a revision number",
                        );
                    }
                    if detail.archive.is_some() && detail.sha256.is_none() {
                        ctx.error(
                            "E4002",
                            &path,
                            format!("archive dependency `{name}` needs a `sha256` checksum"),
                            "missing checksum",
                        );
                    }
                    if let Some(req) = &detail.version
                        && let Err(error) = semver::VersionReq::parse(req)
                    {
                        ctx.error(
                            "E4002",
                            &path.clone().key("version"),
                            format!("invalid version requirement `{req}`: {error}"),
                            "invalid requirement",
                        );
                    }
                    for (j, flavor) in detail.flavors.iter().enumerate() {
                        check_flavor(flavor, &path.clone().key("flavors").index(j), ctx);
                    }
                }
            }
        }
    }
}

fn features(manifest: &Manifest, ctx: &mut Ctx) {
    for (feature, entries) in &manifest.features {
        for (i, entry) in entries.iter().enumerate() {
            let path = root().key("features").key(feature).index(i);
            if let Some(dep) = entry.strip_prefix("dep:") {
                match manifest.dependencies.get(dep) {
                    None => ctx.error("E4314", &path, format!("feature `{feature}` enables unknown dependency `{dep}`"), "unknown dependency"),
                    Some(d) if !d.is_optional() => ctx.push(
                        Diagnostic::error("E4314", format!("feature `{feature}` enables `{dep}`, which is not an optional dependency"))
                            .with_label(ctx.at(&path), "not optional")
                            .with_help(format!("mark `{dep}` with `\"optional\": true`")),
                    ),
                    Some(_) => {}
                }
            } else if !manifest.features.contains_key(entry) || entry == "default" {
                ctx.error(
                    "E4314",
                    &path,
                    format!("feature `{feature}` refers to unknown feature `{entry}`"),
                    "unknown feature",
                );
            }
        }
    }
}

fn level_exists(manifest: &Manifest, level: &str) -> bool {
    BUILTIN_MINIFY_LEVELS.contains(&level) || manifest.minify_levels.contains_key(level)
}

fn profiles(manifest: &Manifest, ctx: &mut Ctx) {
    let names: BTreeSet<&str> = BuiltinProfile::ALL
        .iter()
        .map(|p| p.name())
        .chain(manifest.profiles.keys().map(String::as_str))
        .collect();
    for name in names {
        let path = root().key("profiles").key(name);
        match profile::chain(manifest, name) {
            Err(ProfileError::MissingInherits(p)) => ctx.push(
                Diagnostic::error(
                    "E4315",
                    format!("custom profile `{p}` must name the profile it builds on"),
                )
                .with_label(
                    ctx.at(&root().key("profiles").key(&p)),
                    "missing `inherits`",
                )
                .with_help("add `\"inherits\": \"dev\"` or `\"inherits\": \"release\"`"),
            ),
            Err(ProfileError::Unknown(p)) => ctx.error(
                "E4006",
                &path.clone().key("inherits"),
                format!("profile `{name}` inherits from unknown profile `{p}`"),
                "unknown profile",
            ),
            Err(ProfileError::Cycle(chain)) => ctx.error(
                "E4007",
                &path.clone().key("inherits"),
                format!("profile inheritance cycle: {}", chain.join(" → ")),
                "cycle",
            ),
            Ok(_) => {}
        }
        let Some(settings) = manifest.profiles.get(name) else {
            continue;
        };
        if let Some(level) = &settings.minify
            && !level_exists(manifest, level)
        {
            ctx.error(
                "E4006",
                &path.clone().key("minify"),
                format!("unknown minification level `{level}`"),
                "unknown level",
            );
        }
        if let Some(config) = &settings.config {
            for (key_path, reason) in profile::config_mismatches(&manifest.config, config) {
                let mut p = path.clone().key("config");
                for key in &key_path {
                    p = p.key(key);
                }
                ctx.error(
                    "E4012",
                    &p,
                    format!(
                        "profile `{name}`: config key `{}` {reason}",
                        key_path.join(".")
                    ),
                    "invalid config key",
                );
            }
        }
    }
}

fn minify_levels(manifest: &Manifest, ctx: &mut Ctx) {
    for (name, level) in &manifest.minify_levels {
        let path = root().key("minify-levels").key(name);
        for (i, pass) in level.passes.iter().enumerate() {
            if !MINIFY_PASSES.contains(&pass.as_str()) {
                let mut diagnostic =
                    Diagnostic::error("E6003", format!("unknown minification pass `{pass}`"))
                        .with_label(ctx.at(&path.clone().key("passes").index(i)), "unknown pass");
                if let Some(best) = closest(pass, MINIFY_PASSES) {
                    diagnostic = diagnostic.with_help(format!("did you mean `{best}`?"));
                }
                ctx.push(diagnostic);
            }
        }
        // `extends` must exist and must not form a cycle.
        let mut seen = vec![name.as_str()];
        let mut current = level.extends.as_deref();
        while let Some(parent) = current {
            if seen.contains(&parent) {
                seen.push(parent);
                ctx.error(
                    "E4007",
                    &path.clone().key("extends"),
                    format!("minification level cycle: {}", seen.join(" → ")),
                    "cycle",
                );
                break;
            }
            if !level_exists(manifest, parent) {
                ctx.error(
                    "E4006",
                    &path.clone().key("extends"),
                    format!("unknown minification level `{parent}`"),
                    "unknown level",
                );
                break;
            }
            seen.push(parent);
            current = manifest
                .minify_levels
                .get(parent)
                .and_then(|l| l.extends.as_deref());
        }
    }
}

fn publish(manifest: &Manifest, ctx: &mut Ctx) {
    let Some(publish) = &manifest.publish else {
        return;
    };
    for (name, target) in &publish.targets {
        let path = root().key("publish").key("targets").key(name);
        if let Some(profile) = &target.profile
            && profile::chain(manifest, profile).is_err()
        {
            ctx.error(
                "E4006",
                &path.clone().key("profile"),
                format!("unknown profile `{profile}`"),
                "unknown profile",
            );
        }
        if let Some(level) = &target.minify
            && !level_exists(manifest, level)
        {
            ctx.error(
                "E4006",
                &path.clone().key("minify"),
                format!("unknown minification level `{level}`"),
                "unknown level",
            );
        }
        for (i, flavor) in target.flavors.iter().flatten().enumerate() {
            check_flavor(flavor, &path.clone().key("flavors").index(i), ctx);
        }
        let required: &[(&str, bool)] = match target.target_type {
            TargetType::Curseforge | TargetType::Wowinterface | TargetType::Wago => {
                &[("project-id", target.project_id.is_some())]
            }
            TargetType::Github => &[("repository", target.repository.is_some())],
            TargetType::Registry => &[],
        };
        for (field, present) in required {
            if !present {
                ctx.error(
                    "E4002",
                    &path,
                    format!("publish target `{name}` is missing `{field}`"),
                    "incomplete target",
                );
            }
        }
    }
}

fn globs(manifest: &Manifest, ctx: &mut Ctx) {
    let build = root().key("build");
    for (key, list) in [
        ("assets", &manifest.build.assets),
        ("exclude", &manifest.build.exclude),
        ("load-order", &manifest.build.load_order),
    ] {
        for (i, pattern) in list.iter().enumerate() {
            if let Err(error) = globset::Glob::new(pattern) {
                ctx.error(
                    "E4002",
                    &build.clone().key(key).index(i),
                    format!("invalid glob `{pattern}`: {error}"),
                    "invalid glob",
                );
            }
        }
    }
    for (flavor, list) in &manifest.build.flavor_load_order {
        let path = build.clone().key("flavor-load-order").key(flavor);
        check_flavor(flavor, &path, ctx);
        for (i, pattern) in list.iter().enumerate() {
            if let Err(error) = globset::Glob::new(pattern) {
                ctx.error(
                    "E4002",
                    &path.clone().index(i),
                    format!("invalid glob `{pattern}`: {error}"),
                    "invalid glob",
                );
            }
        }
    }
    for pattern in manifest.build.flavor_files.keys() {
        if let Err(error) = globset::Glob::new(pattern) {
            ctx.error(
                "E4002",
                &build.clone().key("flavor-files").key(pattern),
                format!("invalid glob `{pattern}`: {error}"),
                "invalid glob",
            );
        }
    }
}

fn lints(manifest: &Manifest, ctx: &mut Ctx) {
    for code in manifest.check.lints.keys() {
        let known = code.starts_with('W') && wowlua_diagnostics::lookup(code).is_some();
        if !known {
            ctx.error(
                "E4002",
                &root().key("check").key("lints").key(code),
                format!("`{code}` is not a warning code"),
                "only warnings (W codes) can be configured",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{LoadOutput, parse};
    use std::path::Path;
    use wowlua_span::SourceDb;

    fn run(text: &str) -> LoadOutput {
        let mut db = SourceDb::new();
        parse(
            &mut db,
            Path::new("wowlua.json"),
            text.to_string(),
            &semver::Version::new(0, 1, 0),
        )
    }

    fn codes(text: &str) -> Vec<&'static str> {
        run(text)
            .diagnostics
            .iter()
            .filter_map(|d| d.code)
            .collect()
    }

    const PKG: &str = r#""package": { "name": "my-addon", "version": "1.0.0" }"#;

    #[test]
    fn full_spec_example_is_valid() {
        let text = r#"{
          "package": { "name": "my-addon", "version": "1.4.2", "kind": "addon",
                       "authors": ["Jane Doe <jane@example.com>"], "wowlua": "^0.1" },
          "addon": { "name": "MyAddon", "title": "|cff33ff99My|r Addon",
                     "title-localized": { "deDE": "Mein Addon" },
                     "saved-variables": ["MyAddonDB"], "saved-variables-per-character": ["MyAddonCharDB"],
                     "toc-extra": { "X-Discord": "https://discord.gg/example" },
                     "meta": { "discord": "https://discord.gg/example" } },
          "flavors": { "standard": {}, "mists": {}, "vanilla": { "interface": [11509] } },
          "dependencies": {
            "libstub": { "git": "https://github.com/example/LibStub", "tag": "1.0.3", "kind": "lua-library" },
            "wowlua-ui": { "version": "^0.4", "optional": true } },
          "addon-dependencies": { "optional": [{ "name": "Details", "curseforge-slug": "details" }] },
          "features": { "default": ["minimap"], "minimap": [], "experimental-ui": ["dep:wowlua-ui"] },
          "build": { "src": "src", "assets": ["media/**"] },
          "config": { "logChannel": "SYSTEM", "maxHistory": 50 },
          "profiles": { "dev": { "config": { "maxHistory": 500 } } },
          "publish": { "changelog": "CHANGELOG.md", "targets": {
            "curseforge": { "type": "curseforge", "project-id": 123456 },
            "wago": { "type": "wago", "project-id": "a1b2c3d4" },
            "github": { "type": "github", "repository": "jane/my-addon" } } }
        }"#;
        let out = run(text);
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        let m = out.manifest.unwrap();
        assert_eq!(m.flavors, vec!["standard", "mists", "vanilla"]);
        assert_eq!(m.author().as_deref(), Some("Jane Doe"));
    }

    #[test]
    fn flavor_aliases_and_unknown_flavors() {
        let out = run(&format!(
            r#"{{ {PKG}, "flavors": {{ "retail": {{}}, "classic-era": {{}} }} }}"#
        ));
        assert_eq!(out.manifest.unwrap().flavors, vec!["standard", "vanilla"]);
        assert_eq!(
            codes(&format!(r#"{{ {PKG}, "flavors": {{ "wotlk": {{}} }} }}"#)),
            vec!["E4004"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "flavors": {{ "retail": {{}}, "standard": {{}} }} }}"#
            )),
            vec!["E4002"]
        );
    }

    #[test]
    fn semantic_errors_have_codes() {
        assert_eq!(
            codes(r#"{ "package": { "name": "Bad Name", "version": "1" } }"#),
            vec!["E4003", "E4003"]
        );
        assert_eq!(
            codes(
                r#"{ "package": { "name": "lib", "version": "1.0.0", "kind": "library" }, "addon": {} }"#
            ),
            vec!["E4317"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "addon": {{ "saved-variables": ["1bad", "end"] }} }}"#
            )),
            vec!["E4002", "E4002"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "features": {{ "a": ["nope"], "b": ["dep:x"] }} }}"#
            )),
            vec!["E4314", "E4314"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "profiles": {{ "beta": {{ "debug": true }} }} }}"#
            )),
            vec!["E4315"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "config": {{ "a": 1 }}, "profiles": {{ "dev": {{ "config": {{ "a": "x", "b": 1 }} }} }} }}"#
            )),
            vec!["E4012", "E4012"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "minify-levels": {{ "tiny": {{ "passes": ["strip-coments"] }} }} }}"#
            )),
            vec!["E6003"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "addon": {{ "load-on-demand": false }}, "addon-dependencies": {{ "load-with": ["X"] }} }}"#
            )),
            vec!["E4312"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "dependencies": {{ "a-b": {{ "git": "x", "path": "y" }} }} }}"#
            )),
            vec!["E4002"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "publish": {{ "targets": {{ "cf": {{ "type": "curseforge" }} }} }} }}"#
            )),
            vec!["E4002"]
        );
    }

    #[test]
    fn toc_extra_rules() {
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "addon": {{ "toc-extra": {{ "Foo": "1" }} }} }}"#
            )),
            vec!["W4001"]
        );
        assert_eq!(
            codes(&format!(
                r#"{{ {PKG}, "addon": {{ "toc-extra": {{ "Title-deDE": "x" }} }} }}"#
            )),
            vec!["E4002"]
        );
        assert!(
            codes(&format!(
                r#"{{ {PKG}, "addon": {{ "toc-extra": {{ "X-Website": "x" }} }} }}"#
            ))
            .is_empty()
        );
    }
}
