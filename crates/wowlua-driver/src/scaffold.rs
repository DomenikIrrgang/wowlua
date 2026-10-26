//! `wowlua new` (package templates, spec §12.9) and `wowlua init` (adopting
//! an existing addon, spec §12.6).

// Only a computer has this (spec M14, decision T394).
#![allow(clippy::disallowed_methods)]

use crate::fsutil;
use crate::vcs;
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::model::PackageKind;
use wowlua_manifest::{MANIFEST_FILE, pascal_case, schema_url};

/// `my addon`/`MyAddon` → `my-addon`.
pub fn kebab_case(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() && prev_lower {
            out.push('-');
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.len() < 2 {
        format!("{trimmed}-addon")
            .trim_start_matches('-')
            .to_string()
    } else {
        trimmed
    }
}

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("JSON serializes");
    text.push('\n');
    text
}

// ---------------------------------------------------------------------------
// wowlua new
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct NewRequest {
    pub path: PathBuf,
    pub kind: PackageKind,
    /// `--name`: the AddOn/library name (default: PascalCase of the folder).
    pub name: Option<String>,
    /// `--flavors` (IDs or aliases); empty means the default.
    pub flavors: Vec<String>,
    /// `--ci github`.
    pub ci_github: bool,
}

const GITIGNORE_ADDON: &str = "dist/\n";
const GITIGNORE_LIBRARY: &str = "dist/\nwowlua.lock\n";

const CI_WORKFLOW: &str = r#"name: ci
on:
  push:
    branches: [main]
    tags: ["v*"]
  pull_request:
jobs:
  build:
    runs-on: ubuntu-latest
    permissions:
      contents: write # GitHub releases
    steps:
      - uses: actions/checkout@v5
        with: { fetch-depth: 0 }
      - uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main
      - uses: actions/cache@v4
        with:
          path: ~/.wowlua/cache
          key: wowlua-${{ hashFiles('wowlua.lock') }}
      - run: wowlua build --release --locked --message-format github
      - if: startsWith(github.ref, 'refs/tags/')
        run: wowlua publish
        env:
          CF_API_TOKEN: ${{ secrets.CF_API_TOKEN }}
          WAGO_API_TOKEN: ${{ secrets.WAGO_API_TOKEN }}
          WOWI_API_TOKEN: ${{ secrets.WOWI_API_TOKEN }}
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
"#;

/// Creates a new package from a template. Returns the files written.
pub fn new_package(request: &NewRequest) -> Result<Vec<PathBuf>, Diagnostic> {
    let dir = &request.path;
    if dir.exists()
        && std::fs::read_dir(dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Err(Diagnostic::plain_error(format!(
            "`{}` already exists and is not empty",
            dir.display()
        )));
    }
    let folder = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "addon".into());
    let package_name = kebab_case(request.name.as_deref().unwrap_or(&folder));
    let display_name = request
        .name
        .clone()
        .unwrap_or_else(|| pascal_case(&package_name));

    let mut flavors = Map::new();
    for name in &request.flavors {
        let flavor = wowlua_flavors::table()
            .resolve(name)
            .ok_or_else(|| Diagnostic::error("E4004", format!("unknown flavor `{name}`")))?;
        flavors.insert(flavor.id.clone(), json!({}));
    }

    let mut package = Map::new();
    package.insert("name".into(), package_name.clone().into());
    package.insert("version".into(), "0.1.0".into());
    if request.kind != PackageKind::Addon {
        package.insert(
            "kind".into(),
            serde_json::to_value(request.kind).expect("kinds serialize"),
        );
    }
    if let Some(author) = vcs::user_name(dir.parent().unwrap_or(Path::new("."))) {
        package.insert("authors".into(), json!([author]));
    }
    package.insert("wowlua".into(), format!("^{}", minor_requirement()).into());

    let mut manifest = Map::new();
    manifest.insert("$schema".into(), schema_url().into());
    manifest.insert("package".into(), Value::Object(package));

    let mut files: Vec<(String, String)> = Vec::new();
    match request.kind {
        PackageKind::Addon => {
            let mut addon = Map::new();
            if display_name != pascal_case(&package_name) {
                addon.insert("name".into(), display_name.clone().into());
            }
            addon.insert("title".into(), display_name.clone().into());
            manifest.insert("addon".into(), Value::Object(addon));
            if !flavors.is_empty() {
                manifest.insert("flavors".into(), Value::Object(flavors.clone()));
            }
            files.push(("src/main.wowlua".into(), main_template()));
            files.push((
                "tests/main.test.wowlua".into(),
                addon_test_template(&display_name),
            ));
            files.push(("locales/enUS.json".into(), "{}\n".into()));
            files.push((
                "CHANGELOG.md".into(),
                format!("# Changelog\n\n## 0.1.0\n\n- Initial version of {display_name}.\n"),
            ));
            files.push((".gitignore".into(), GITIGNORE_ADDON.into()));
        }
        PackageKind::Library => {
            if !flavors.is_empty() {
                manifest.insert("flavors".into(), Value::Object(flavors.clone()));
            }
            files.push((
                "src/init.wowlua".into(),
                "-- Library entry module.\nlocal _, ns = ...\n".into(),
            ));
            files.push(("tests/init.test.wowlua".into(), test_template()));
            files.push((".gitignore".into(), GITIGNORE_LIBRARY.into()));
        }
        PackageKind::LuaLibrary => {
            manifest.insert(
                "lua-library".into(),
                json!({ "files": [format!("{display_name}.lua")] }),
            );
            files.push((
                format!("{display_name}.lua"),
                format!("-- {display_name}\n"),
            ));
            files.push((".gitignore".into(), GITIGNORE_LIBRARY.into()));
        }
        PackageKind::Types => {
            files.push((
                "types/index.d.wowlua".into(),
                "-- Type declarations.\n".into(),
            ));
            files.push((".gitignore".into(), GITIGNORE_LIBRARY.into()));
        }
    }
    files.push(("README.md".into(), format!("# {display_name}\n\nBuilt with [wowlua](https://github.com/DomenikIrrgang/wowlua).\n\n```sh\nwowlua build\n```\n")));
    if request.ci_github {
        files.push((".github/workflows/ci.yml".into(), CI_WORKFLOW.into()));
    }
    files.insert(0, (MANIFEST_FILE.into(), pretty(&Value::Object(manifest))));

    let mut written = Vec::new();
    for (rel, content) in files {
        let path = dir.join(&rel);
        fsutil::write_file(&path, content.as_bytes()).map_err(|e| {
            Diagnostic::plain_error(format!("cannot write `{}`: {e}", path.display()))
        })?;
        written.push(path);
    }
    if vcs::git_available() && !vcs::inside_work_tree(dir) {
        vcs::git_init(dir);
    }
    Ok(written)
}

fn minor_requirement() -> String {
    let v = crate::compiler_version();
    format!("{}.{}", v.major, v.minor)
}

fn main_template() -> String {
    "-- Entry point of the addon.\n\
     local addonName: string = ...\n\
     \n\
     local frame = CreateFrame(\"Frame\")\n\
     frame:RegisterEvent(\"PLAYER_LOGIN\")\n\
     frame:SetScript(\"OnEvent\", function()\n\
     \x20   print(addonName .. \" loaded.\")\n\
     end)\n"
        .to_string()
}

fn addon_test_template(addon: &str) -> String {
    format!(
        "-- Tests run in the emulated client with `wowlua test`. Test files are never\n\
         -- part of a build.\n\
         import test, expect from \"wowlua:test\"\n\
         import * as mock from \"wowlua:mock\"\n\
         \n\
         @test(\"greets at login\")\n\
         local function greets()\n\
         \x20   expect(mock.printed()).toContain(\"{addon} loaded.\")\n\
         end\n"
    )
}

fn test_template() -> String {
    "-- Tests run with `wowlua test` (milestone M9). Test files are never part of a build.\n"
        .to_string()
}

// ---------------------------------------------------------------------------
// wowlua init
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct InitRequest {
    pub dir: PathBuf,
    /// `--from-toc`: the TOC to adopt (default: all TOCs of the addon).
    pub from_toc: Option<PathBuf>,
    /// `--from-pkgmeta`: require a `.pkgmeta` (it is converted whenever it
    /// exists).
    pub from_pkgmeta: bool,
    /// `--from-pkgmeta <file>`: the packager project file to convert.
    pub pkgmeta_file: Option<PathBuf>,
    /// For a folder `move-folders` makes a workspace member: the parts of
    /// the parent's `.pkgmeta` inside it, relative to it.
    pub inherited: Option<crate::pkgmeta::PkgMeta>,
    /// The AddOn name when it is known (a `move-folders` target), so that
    /// `LittleWigs_Classic.toc` is not read as `LittleWigs` for Classic.
    pub addon_name: Option<String>,
    /// The AddOns next to this one once packaged, when they are not its
    /// folder's siblings: a `move-folders` member and the addon it is moved
    /// out of. `..\<name>\` TOC lines resolve to them.
    pub neighbors: Vec<Neighbor>,
}

/// An AddOn next to the one being adopted once packaged.
#[derive(Clone, Debug)]
pub struct Neighbor {
    pub name: String,
    pub dir: PathBuf,
    /// Its source directory when its files are already moved.
    pub src: Option<String>,
}

/// What `wowlua init` did.
#[derive(Clone, Debug, Default)]
pub struct InitReport {
    pub addon_name: String,
    pub flavors: Vec<String>,
    pub moved: usize,
    pub rewritten_paths: usize,
    pub warnings: Vec<Diagnostic>,
    pub old_tocs: Vec<String>,
    /// Dependencies converted from `.pkgmeta` externals.
    pub dependencies: usize,
    /// Workspace members created from `move-folders`.
    pub members: Vec<String>,
    /// Their interface pins (member folder, flavor → numbers), settled into
    /// the workspace lockfile by [`write_pins`].
    pub member_pins: Vec<(String, BTreeMap<String, Vec<u32>>)>,
    /// External addons from `move-folders` (AddOn name → source).
    pub external_addons: Vec<(String, Value)>,
}

const ASSET_EXTENSIONS: &[&str] = &[
    "tga", "blp", "png", "jpg", "jpeg", "ogg", "mp3", "wav", "ttf", "otf",
];

/// Adopts the addon in `request.dir`: writes `wowlua.json`, moves its Lua
/// and XML files into `src/` and rewrites in-game paths to them.
pub fn init_package(request: &InitRequest) -> Result<InitReport, Diagnostic> {
    let dir = &request.dir;
    if dir.join(MANIFEST_FILE).exists() {
        return Err(Diagnostic::plain_error(format!(
            "`{MANIFEST_FILE}` already exists"
        )));
    }
    let tocs = match find_tocs(
        dir,
        request.from_toc.as_deref(),
        request.addon_name.as_deref(),
    ) {
        Ok(t) => t,
        Err(e) => {
            // A repository of several addons.
            if request.inherited.is_none()
                && request.from_toc.is_none()
                && let Some(meta) =
                    crate::pkgmeta::read_project(dir, request.pkgmeta_file.as_deref())?
            {
                return init_suite(dir, meta);
            }
            return Err(e);
        }
    };
    let mut pkgmeta = match &request.inherited {
        Some(m) => Some(m.clone()),
        None => crate::pkgmeta::read_project(dir, request.pkgmeta_file.as_deref())?,
    };
    let external_addons = match pkgmeta.as_mut() {
        Some(m) => {
            let package = m.package_as.clone().unwrap_or_else(|| tocs[0].1.clone());
            extract_external_addons(m, &package)
        }
        None => Vec::new(),
    };
    // Folders `move-folders` turns into addons of their own.
    let members: Vec<(String, String)> = pkgmeta
        .as_ref()
        .map(|m| member_folders(dir, m, &tocs[0].1))
        .unwrap_or_default();
    if let Some(m) = pkgmeta.as_mut() {
        flatten_externals(m, &members, true);
    }
    if request.from_pkgmeta && pkgmeta.is_none() {
        return Err(Diagnostic::plain_error(
            "`--from-pkgmeta` was given, but there is no `.pkgmeta` or `pkgmeta.yaml`",
        ));
    }
    let addon_name = tocs[0].1.clone();
    let src_dir = source_dir_for(dir);
    let mut report = InitReport {
        addon_name: addon_name.clone(),
        external_addons,
        old_tocs: tocs
            .iter()
            .map(|(p, _, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect(),
        ..Default::default()
    };

    // Parse every TOC. Each flavor's client reads exactly one of them: the
    // game-suffixed TOC, else the family-suffixed one, else the plain one.
    let table = wowlua_flavors::table();
    let mut infos: Vec<(wowlua_toc::ParsedToc, Option<String>, Vec<String>)> = Vec::new();
    let mut all_flavors: Vec<String> = Vec::new();
    // `## Interface-<Type>:` lines make TOCs per game only in the
    // packager's split and single-game builds.
    let per_game_tocs = match &pkgmeta {
        Some(m) if m.per_game => Some(true),
        Some(m) if m.split => Some(false),
        _ => None,
    };
    let (tocs, texts): (Vec<_>, Vec<_>) = toc_views(&tocs, pkgmeta.is_some(), per_game_tocs)?
        .into_iter()
        .unzip();
    for ((_, _, suffix), text) in tocs.iter().zip(&texts) {
        let toc = wowlua_toc::parse(text);
        let meaning = suffix.as_deref().and_then(|s| table.suffix_meaning(s));
        let mut candidates: Vec<String> = Vec::new();
        for interface in toc.interfaces() {
            if let Some(f) = table.by_interface(interface)
                && !candidates.contains(&f.id)
            {
                candidates.push(f.id.clone());
            }
        }
        if candidates.is_empty() {
            candidates = match meaning {
                Some(wowlua_flavors::SuffixMeaning::Flavor(f)) => vec![f.id.clone()],
                Some(wowlua_flavors::SuffixMeaning::Family(family)) => table
                    .flavors
                    .iter()
                    .filter(|f| f.family == family)
                    .map(|f| f.id.clone())
                    .collect(),
                None => vec!["standard".to_string()],
            };
        }
        // `## AllowLoadGameType:` / `## ExcludeLoadGameType:` restrict the
        // whole addon (repeated lines append).
        let addon_wide: Vec<wowlua_toc::InlineDirective> = toc
            .directives
            .iter()
            .filter(|d| {
                d.key.eq_ignore_ascii_case("AllowLoadGameType")
                    || d.key.eq_ignore_ascii_case("ExcludeLoadGameType")
            })
            .map(|d| wowlua_toc::InlineDirective {
                name: d.key.clone(),
                values: d
                    .value
                    .split(',')
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .collect(),
            })
            .collect();
        let allowed: Vec<wowlua_toc::InlineDirective> = addon_wide
            .iter()
            .filter(|d| d.name.eq_ignore_ascii_case("AllowLoadGameType"))
            .cloned()
            .reduce(|mut a, b| {
                a.values.extend(b.values);
                a
            })
            .into_iter()
            .chain(
                addon_wide
                    .iter()
                    .filter(|d| d.name.eq_ignore_ascii_case("ExcludeLoadGameType"))
                    .cloned(),
            )
            .collect();
        candidates = apply_conditions(&candidates, &allowed);
        // A suffixed TOC only serves the flavors its suffix names.
        candidates.retain(|id| match meaning {
            Some(wowlua_flavors::SuffixMeaning::Flavor(f)) => f.id == *id,
            Some(wowlua_flavors::SuffixMeaning::Family(family)) => {
                table.flavor(id).is_some_and(|f| f.family == family)
            }
            None => true,
        });
        for f in &candidates {
            if !all_flavors.contains(f) {
                all_flavors.push(f.clone());
            }
        }
        infos.push((toc, suffix.clone(), candidates));
    }
    if all_flavors.is_empty() {
        all_flavors.push("standard".into());
    }
    report.flavors = all_flavors.clone();

    // `views`: every TOC with the flavors whose client reads it.
    let rank =
        |suffix: &Option<String>| match suffix.as_deref().and_then(|s| table.suffix_meaning(s)) {
            Some(wowlua_flavors::SuffixMeaning::Flavor(_)) => 3,
            Some(wowlua_flavors::SuffixMeaning::Family(_)) => 2,
            None => 1,
        };
    let mut views: Vec<(wowlua_toc::ParsedToc, Vec<String>)> = infos
        .iter()
        .map(|(toc, _, _)| (toc.clone(), Vec::new()))
        .collect();
    for flavor in &all_flavors {
        let chosen = infos
            .iter()
            .enumerate()
            .filter(|(_, (_, _, candidates))| candidates.contains(flavor))
            .max_by_key(|(i, (_, suffix, _))| (rank(suffix), std::cmp::Reverse(*i)))
            .map(|(i, _)| i);
        if let Some(i) = chosen {
            views[i].1.push(flavor.clone());
        }
    }
    let unused: Vec<String> = tocs
        .iter()
        .zip(&views)
        .filter(|(_, (_, flavors))| flavors.is_empty())
        .filter_map(|((path, _, _), _)| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    if !unused.is_empty() {
        report.warnings.push(Diagnostic::plain_warning(format!(
            "no client reads {} (a more specific TOC wins for every flavor); ignored",
            unused.join(", ")
        )));
    }

    // Per-flavor file sequences (from the TOC each client reads), merged into
    // one order that respects every flavor's order. Variable lines
    // (`Locales\[TextLocale].lua`) take part like files.
    let mut sequences: Vec<Vec<String>> = Vec::new();
    let mut flavor_sequences: Vec<(String, Vec<String>)> = Vec::new();
    let mut canonical: HashMap<String, String> = HashMap::new();
    let mut file_flavors: IndexMap<String, BTreeSet<String>> = IndexMap::new();
    let mut variable_lines: IndexMap<String, (BTreeSet<String>, Option<Vec<String>>)> =
        IndexMap::new();
    // Lines loading files of another AddOn (`..\Other\file.lua`).
    let mut external_lines: IndexMap<String, BTreeSet<String>> = IndexMap::new();
    for flavor in &all_flavors {
        let Some((toc, _)) = views.iter().find(|(_, f)| f.contains(flavor)) else {
            continue;
        };
        let mut sequence = Vec::new();
        for file in &toc.files {
            if apply_conditions(std::slice::from_ref(flavor), &file.conditions).is_empty() {
                continue;
            }
            let key = canonical
                .entry(file.path.to_ascii_lowercase())
                .or_insert_with(|| file.path.clone())
                .clone();
            if key.starts_with("../") {
                external_lines
                    .entry(key.clone())
                    .or_default()
                    .insert(flavor.clone());
            } else if key.contains('[')
                || file
                    .conditions
                    .iter()
                    .any(|c| c.name.eq_ignore_ascii_case("AllowLoadTextLocale"))
            {
                let locales = file
                    .conditions
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case("AllowLoadTextLocale"))
                    .map(|c| c.values.clone());
                let entry = variable_lines
                    .entry(key.clone())
                    .or_insert_with(|| (BTreeSet::new(), locales));
                entry.0.insert(flavor.clone());
            } else {
                file_flavors
                    .entry(key.clone())
                    .or_default()
                    .insert(flavor.clone());
            }
            if !sequence.contains(&key) {
                sequence.push(key);
            }
        }
        flavor_sequences.push((flavor.clone(), sequence.clone()));
        sequences.push(sequence);
    }
    let (order, _) = merge_orders(&sequences);

    // `.pkgmeta` externals: their files are not the addon's own. TOC lines
    // into them become the dependencies' entry files.
    let externals: Vec<crate::pkgmeta::External> = pkgmeta
        .as_ref()
        .map(|m| m.externals.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|e| {
            !members.iter().any(|(rel, _)| {
                e.dir
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}/", rel.to_ascii_lowercase()))
            })
        })
        .collect();
    let external_of = |path: &str| -> Option<usize> {
        let lower = path.to_ascii_lowercase();
        externals
            .iter()
            .position(|e| lower.starts_with(&format!("{}/", e.dir.to_ascii_lowercase())))
    };
    let mut external_files: Vec<Vec<String>> = vec![Vec::new(); externals.len()];
    let mut external_flavors: Vec<BTreeSet<String>> = vec![BTreeSet::new(); externals.len()];
    for path in &order {
        if let Some(i) = external_of(path) {
            let rel = path[externals[i].dir.len() + 1..].to_string();
            if !external_files[i].contains(&rel) {
                external_files[i].push(rel);
            }
            if let Some(set) = file_flavors.get(path) {
                external_flavors[i].extend(set.iter().cloned());
            }
        }
    }

    // Files to move: TOC files, expansions of variable lines, and everything
    // they include through XML.
    let mut queue: Vec<String> = order
        .iter()
        .filter(|p| !p.contains('[') && !p.starts_with("../") && external_of(p).is_none())
        .cloned()
        .collect();
    for (path, (flavors, locales)) in &variable_lines {
        let candidates: Vec<String> = match locales {
            Some(list) => list.clone(),
            None => wowlua_manifest::model::LOCALES
                .iter()
                .map(|l| l.to_string())
                .collect(),
        };
        for flavor in flavors {
            for locale in &candidates {
                let concrete = crate::build::substitute(path, flavor, locale);
                if existing_case(dir, &concrete).is_some() {
                    queue.push(concrete);
                }
            }
        }
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut into_externals: Vec<String> = Vec::new();
    // Files of externals and of `move-folders` members are not this addon's.
    let skip = |p: &str| {
        let lower = p.to_ascii_lowercase();
        external_of(p).is_some()
            || members
                .iter()
                .any(|(rel, _)| lower.starts_with(&format!("{}/", rel.to_ascii_lowercase())))
    };
    let mut to_move = closure(
        dir,
        queue,
        &mut seen,
        &mut report.warnings,
        &skip,
        &mut into_externals,
        pkgmeta.is_some(),
    );
    // Files other AddOns next to this one load (`..\<this>\file.lua`) move
    // too and are shipped as `build.shared-files` (unless loaded here anyway).
    let neighbors = if request.neighbors.is_empty() {
        // The members `move-folders` places next to this addon.
        members
            .iter()
            .map(|(rel, name)| Neighbor {
                name: name.clone(),
                dir: dir.join(rel),
                src: None,
            })
            .collect()
    } else {
        request.neighbors.clone()
    };
    let referenced = sibling_references(dir, &addon_name, &neighbors);
    let ignored = |rel: &str| {
        pkgmeta
            .as_ref()
            .is_some_and(|m| crate::keywords::plain_copy(&m.ignore, &[rel]))
    };
    let shared: Vec<String> = referenced
        .iter()
        .filter_map(|rel| existing_case(dir, rel))
        .filter(|actual| !seen.contains(actual))
        // The packager does not ship files `.pkgmeta` ignores.
        .filter(|actual| !ignored(actual))
        .collect();
    to_move.extend(closure(
        dir,
        shared.clone(),
        &mut seen,
        &mut report.warnings,
        &skip,
        &mut into_externals,
        pkgmeta.is_some(),
    ));
    // Files the addon's own XML loads from externals are entry files too
    // (they are not listed in the TOC again, decision T107).
    for path in &into_externals {
        if let Some(i) = external_of(path) {
            let rel = path[externals[i].dir.len() + 1..].to_string();
            if !external_files[i]
                .iter()
                .any(|f| f.eq_ignore_ascii_case(&rel))
            {
                external_files[i].push(rel);
            }
        }
    }
    to_move.sort();

    // Move into src/ (git mv inside a repository).
    let in_git = vcs::inside_work_tree(dir);
    for rel in &to_move {
        let dest = format!("{src_dir}/{rel}");
        if let Some(parent) = dir.join(&dest).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Diagnostic::plain_error(format!("cannot create `{}`: {e}", parent.display()))
            })?;
        }
        let moved = in_git && vcs::git_mv(dir, rel, &dest);
        if !moved {
            std::fs::rename(dir.join(rel), dir.join(&dest))
                .map_err(|e| Diagnostic::plain_error(format!("cannot move `{rel}`: {e}")))?;
        }
        report.moved += 1;
    }
    remove_empty_dirs(dir, &to_move);

    // Rewrite absolute in-game paths to moved files.
    let moved_lower: BTreeSet<String> = to_move.iter().map(|p| p.to_ascii_lowercase()).collect();
    for rel in &to_move {
        let lower = rel.to_ascii_lowercase();
        if !(lower.ends_with(".lua") || lower.ends_with(".xml")) {
            continue;
        }
        let path = dir.join(src_dir).join(rel);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (rewritten, count) = rewrite_ingame_paths(&text, &addon_name, &moved_lower);
        if count > 0 {
            std::fs::write(&path, rewritten).map_err(|e| {
                Diagnostic::plain_error(format!("cannot write `{}`: {e}", path.display()))
            })?;
            report.rewritten_paths += count;
        }
    }

    // Assets stay in place.
    let assets = asset_globs(dir, src_dir);

    // Manifest. Structural fields are agreed across the TOCs clients read
    // (majority, saved variables united); bulk `X-` data goes to TOC data
    // files; metadata that differs per flavor becomes `flavor-overrides`.
    let structural = structural_toc(&views);
    let toc_data = extract_toc_data(dir, &views)?;
    let metadata = flavor_metadata(&views, &all_flavors, toc_data.is_some());
    let mut warnings = Vec::new();
    // Paths of external lines in the other AddOn's output.
    let mut external_paths: HashMap<String, String> = HashMap::new();
    for line in external_lines.keys() {
        let (path, warning) = resolve_external(dir, line, &neighbors);
        if let Some(warning) = warning {
            warnings.push(warning);
        }
        external_paths.insert(line.clone(), path);
    }
    // Entry files of embedded libraries keep their place in the load order
    // under their output path (`libs/LibStub/LibStub.lua`, decision T106).
    for path in &order {
        if external_of(path).is_some() {
            external_paths.insert(path.clone(), path.clone());
        }
    }
    let mut manifest = adopted_manifest(
        &structural,
        &views,
        &metadata,
        &addon_name,
        &all_flavors,
        &order,
        &to_move,
        &file_flavors,
        assets,
        &mut warnings,
        src_dir,
        &external_paths,
        crate::vcs::latest_tag(dir),
    );
    if let Some(addon) = manifest.get_mut("addon").and_then(Value::as_object_mut)
        && let Some(files) = &toc_data
    {
        addon.insert("toc-data".into(), json!(files));
    }
    // Addon-wide load conditions stay in the TOC, as the client reads them
    // there (the flavors already reflect them).
    if let Some(addon) = manifest.get_mut("addon").and_then(Value::as_object_mut) {
        for key in ["AllowLoadGameType", "ExcludeLoadGameType"] {
            let values: BTreeSet<String> = views
                .iter()
                .filter(|(_, f)| !f.is_empty())
                .map(|(t, _)| t.list(|k| k.eq_ignore_ascii_case(key)).join(", "))
                .collect();
            if values.len() == 1 {
                if let Some(v) = values.into_iter().next().filter(|v| !v.is_empty()) {
                    let extra = addon
                        .entry("toc-extra")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .expect("an object");
                    extra.insert(key.into(), json!(v));
                }
            } else {
                // Per flavor, as each flavor's TOC says.
                for (toc, flavors) in views.iter().filter(|(_, f)| !f.is_empty()) {
                    let v = toc.list(|k| k.eq_ignore_ascii_case(key)).join(", ");
                    if v.is_empty() {
                        continue;
                    }
                    for flavor in flavors {
                        let overrides = addon
                            .entry("flavor-overrides")
                            .or_insert_with(|| json!({}))
                            .as_object_mut()
                            .expect("an object");
                        let o = overrides
                            .entry(flavor.clone())
                            .or_insert_with(|| json!({}))
                            .as_object_mut()
                            .expect("an object");
                        let extra = o
                            .entry("toc-extra")
                            .or_insert_with(|| json!({}))
                            .as_object_mut()
                            .expect("an object");
                        extra.insert(key.into(), json!(v));
                    }
                }
            }
        }
    }
    per_flavor_toc_fields(&mut manifest, &views);
    // Flavors whose TOCs load files in another order than the merged one
    // keep their own (decision T126).
    let mut own_orders = Map::new();
    for (flavor, sequence) in &flavor_sequences {
        let merged: Vec<&String> = order.iter().filter(|p| sequence.contains(p)).collect();
        if merged.iter().copied().ne(sequence.iter()) {
            let entries: Vec<String> = sequence
                .iter()
                .filter_map(|p| order_entry(p, &external_paths, &to_move, src_dir))
                .collect();
            own_orders.insert(flavor.clone(), json!(entries));
        }
    }
    if !own_orders.is_empty()
        && let Some(build) = manifest
            .as_object_mut()
            .map(|m| m.entry("build").or_insert_with(|| json!({})))
            .and_then(Value::as_object_mut)
    {
        build.insert("flavor-load-order".into(), Value::Object(own_orders));
    }
    if let Some(meta) = &pkgmeta {
        convert_pkgmeta(
            dir,
            meta,
            &mut manifest,
            &externals,
            &external_files,
            &external_flavors,
            &all_flavors,
            &to_move,
            src_dir,
            &members,
            &mut report,
        );
        // The packager writes the TOC's own version line (decision T153).
        // Per flavor when the TOCs (or their flavor blocks) differ.
        let mut per_flavor = Map::new();
        for (toc, flavors) in &views {
            for f in flavors {
                per_flavor.insert(f.clone(), json!(toc.get("Version").unwrap_or_default()));
            }
        }
        let values: BTreeSet<String> = per_flavor.values().map(|v| v.to_string()).collect();
        let toc_version = match values.len() {
            0 => json!(""),
            1 => per_flavor.values().next().cloned().unwrap_or_default(),
            _ => Value::Object(per_flavor.clone()),
        };
        if let Some(compat) = manifest
            .pointer_mut("/compat")
            .and_then(Value::as_object_mut)
        {
            compat.insert("toc-version".into(), toc_version);
        }
    }
    if !variable_lines.is_empty()
        && let Some(build) = manifest
            .as_object_mut()
            .map(|m| m.entry("build").or_insert_with(|| json!({})))
            .and_then(Value::as_object_mut)
    {
        let entries: Vec<Value> = variable_lines
            .iter()
            .map(|(path, (flavors, locales))| {
                let mut entry = Map::new();
                entry.insert("path".into(), format!("{src_dir}/{path}").into());
                if let Some(locales) = locales {
                    entry.insert("locales".into(), json!(locales));
                }
                if flavors.len() < all_flavors.len() {
                    let ordered: Vec<&String> = all_flavors
                        .iter()
                        .filter(|f| flavors.contains(*f))
                        .collect();
                    entry.insert("flavors".into(), json!(ordered));
                }
                Value::Object(entry)
            })
            .collect();
        build.insert("variable-files".into(), Value::Array(entries));
    }
    if let Some(build) = manifest
        .as_object_mut()
        .map(|m| m.entry("build").or_insert_with(|| json!({})))
        .and_then(Value::as_object_mut)
    {
        if !external_lines.is_empty() {
            let entries: Vec<Value> = external_lines
                .iter()
                .map(|(line, flavors)| {
                    let mut entry = Map::new();
                    entry.insert("path".into(), external_paths[line].clone().into());
                    if flavors.len() < all_flavors.len() {
                        let ordered: Vec<&String> = all_flavors
                            .iter()
                            .filter(|f| flavors.contains(*f))
                            .collect();
                        entry.insert("flavors".into(), json!(ordered));
                    }
                    Value::Object(entry)
                })
                .collect();
            build.insert("external-files".into(), Value::Array(entries));
        }
        if !shared.is_empty() {
            let paths: Vec<String> = shared.iter().map(|p| format!("{src_dir}/{p}")).collect();
            build.insert("shared-files".into(), json!(paths));
        }
        if build.is_empty() {
            manifest.as_object_mut().map(|m| m.shift_remove("build"));
        }
    }
    report.warnings.extend(warnings);

    // Lua/XML files no TOC loads stay where they are; say so.
    let left: Vec<String> = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| fsutil::relative_slash(e.path(), dir))
        .filter(|rel| !rel.starts_with(&format!("{src_dir}/")) && !rel.starts_with("dist/"))
        .filter(|rel| !crate::build::is_bindings_file(rel))
        .filter(|rel| external_of(rel).is_none())
        .filter(|rel| {
            !report
                .members
                .iter()
                .any(|m| rel.starts_with(&format!("{m}/")))
        })
        .filter(|rel| {
            let lower = rel.to_ascii_lowercase();
            lower.ends_with(".lua") || lower.ends_with(".xml")
        })
        .collect();
    if !left.is_empty() {
        let shown: Vec<&str> = left.iter().take(5).map(String::as_str).collect();
        let more = if left.len() > 5 {
            format!(" and {} more", left.len() - 5)
        } else {
            String::new()
        };
        report.warnings.push(Diagnostic::plain_warning(format!(
            "{} Lua/XML file(s) are not loaded by any TOC and were left in place: {}{more}",
            left.len(),
            shown.join(", ")
        )));
    }

    std::fs::write(dir.join(MANIFEST_FILE), pretty(&manifest))
        .map_err(|e| Diagnostic::plain_error(format!("cannot write `{MANIFEST_FILE}`: {e}")))?;

    // The lockfile pins the interface numbers the TOCs list, so the
    // generated TOCs keep them (`wowlua types sync` moves them on).
    let mut pins: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for (toc, flavors) in &views {
        for interface in toc.interfaces() {
            if let Some(f) = table.by_interface(interface)
                && flavors.contains(&f.id)
            {
                let list = pins.entry(f.id.clone()).or_default();
                if !list.contains(&interface) {
                    list.push(interface);
                }
            }
        }
    }
    write_pins(dir, pins, &mut report)?;
    Ok(report)
}

/// Step 6 of `wowlua init` (spec CLI §12.6): `.pkgmeta` externals become
/// dependencies; `ignore`, `plain-copy`, `package-as`, `manual-changelog`,
/// `enable-nolib-creation` and the relation lists become manifest settings;
/// `move-folders` that move whole addon folders become workspace members.
#[allow(clippy::too_many_arguments)]
fn convert_pkgmeta(
    dir: &Path,
    meta: &crate::pkgmeta::PkgMeta,
    manifest: &mut Value,
    externals: &[crate::pkgmeta::External],
    external_files: &[Vec<String>],
    external_flavors: &[BTreeSet<String>],
    all_flavors: &[String],
    moved: &[String],
    src_dir: &str,
    member_dirs: &[(String, String)],
    report: &mut InitReport,
) {
    let Some(object) = manifest.as_object_mut() else {
        return;
    };
    // Dependencies, in the order of the externals.
    // The folder of the externals that become dependencies (loaded ones,
    // not hg).
    let converted: Vec<crate::pkgmeta::External> = externals
        .iter()
        .zip(external_files)
        .filter(|(e, files)| {
            !files.is_empty() && !matches!(e.source, crate::pkgmeta::ExternalSource::Hg { .. })
        })
        .map(|(e, _)| e.clone())
        .collect();
    let libs_dir = crate::pkgmeta::libs_dir(&converted);
    let mut deps = Map::new();
    for (i, e) in externals.iter().enumerate() {
        if let crate::pkgmeta::ExternalSource::Hg { url } = &e.source {
            report.warnings.push(
                Diagnostic::warning(
                    "W4333",
                    format!(
                        "external `{}` is a Mercurial repository (`{url}`), which wowlua cannot fetch; not converted",
                        e.dir
                    ),
                )
                .with_help("replace it with a git mirror or a release archive"),
            );
            continue;
        }
        // The packager ships externals nothing loads too (decision T156).
        let unloaded = external_files[i].is_empty();
        let inside_libs = libs_dir
            .as_ref()
            .is_none_or(|d| e.dir.starts_with(&format!("{d}/")));
        if unloaded && !inside_libs {
            report.warnings.push(Diagnostic::plain_warning(format!(
                "external `{}` is not loaded by the TOC or the addon's XML and lies outside `{}`; not converted",
                e.dir,
                libs_dir.as_deref().unwrap_or_default()
            )));
            continue;
        }
        let mut entry = match crate::pkgmeta::dependency_entry(e, wowlua_pkg::fetch::sha256_of_url)
        {
            Ok(entry) => entry,
            Err(warning) => {
                report.warnings.push(warning);
                continue;
            }
        };
        if e.latest && matches!(e.source, crate::pkgmeta::ExternalSource::Svn { .. }) {
            report.warnings.push(Diagnostic::plain_warning(format!(
                "external `{}` uses `tag: latest` on SVN; the dependency follows trunk instead (pin a `tag`)",
                e.dir
            )));
        }
        let install = match &libs_dir {
            Some(d) => e.dir[d.len() + 1..].to_string(),
            None => e.dir.clone(),
        };
        entry.insert("install-path".into(), json!(install));
        if unloaded {
            entry.insert("load".into(), json!(false));
        } else {
            entry.insert("files".into(), json!(external_files[i]));
        }
        let flavors = &external_flavors[i];
        if !flavors.is_empty() && flavors.len() < all_flavors.len() {
            let ordered: Vec<&String> = all_flavors
                .iter()
                .filter(|f| flavors.contains(*f))
                .collect();
            entry.insert("flavors".into(), json!(ordered));
        }
        let mut name = crate::pkgmeta::dependency_name(&e.dir);
        let base = name.clone();
        let mut n = 2;
        while deps.contains_key(&name) {
            name = format!("{base}-{n}");
            n += 1;
        }
        deps.insert(name, Value::Object(entry));
        report.dependencies += 1;
    }
    if !deps.is_empty() {
        object.insert("dependencies".into(), Value::Object(deps));
        let build = object
            .entry("build")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("an object");
        match &libs_dir {
            Some(d) if d != "libs" => {
                build.insert("libs-dir".into(), json!(d));
            }
            None => {
                build.insert("libs-dir".into(), json!("."));
            }
            _ => {}
        }
    }
    // `package-as` names the AddOn folder.
    if let Some(name) = &meta.package_as {
        let addon = object
            .entry("addon")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("an object");
        if &report.addon_name != name {
            addon.insert("name".into(), json!(name));
            report.addon_name = name.clone();
        }
    }
    // `ignore` and `plain-copy`, relative to the package root; moved files
    // live under the source folder now.
    let moved_lower: BTreeSet<String> = moved.iter().map(|m| m.to_ascii_lowercase()).collect();
    let pattern_of = |p: &str| -> Vec<String> {
        let p = p
            .trim_end_matches("/*")
            .trim_end_matches('/')
            .replace('\\', "/");
        let lower = p.to_ascii_lowercase();
        let mut out = Vec::new();
        let moved_inside = moved_lower
            .iter()
            .any(|m| m.starts_with(&format!("{lower}/")));
        if moved_lower.contains(&lower) {
            out.push(format!("{src_dir}/{p}"));
        } else if dir.join(&p).is_dir() || moved_inside {
            if dir.join(&p).is_dir() {
                out.push(format!("{p}/**"));
            }
            if moved_inside {
                out.push(format!("{src_dir}/{p}/**"));
            }
        } else {
            // Nothing there yet (an external's folder): a file or a folder,
            // like the packager's pattern pair.
            out.push(format!("{p}/**"));
            out.push(p);
        }
        out
    };
    let build = object
        .entry("build")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("an object");
    // Each flavor's `ignore`: its own project file's (per-game packager
    // runs), else that of the other flavors of its family when they agree,
    // else the main file's. What every flavor ignores is excluded; the rest
    // restricts files to the flavors that keep them (`build.flavor-files`).
    let table = wowlua_flavors::table();
    let ignore_of = |flavor: &String| -> Vec<String> {
        if let Some(own) = meta.games.get(flavor) {
            return own.clone();
        }
        let family = table.flavor(flavor).map(|f| f.family);
        let kin: Vec<&Vec<String>> = meta
            .games
            .iter()
            .filter(|(f, _)| table.flavor(f).map(|f| f.family) == family)
            .map(|(_, ignore)| ignore)
            .collect();
        match kin.first() {
            Some(first) if kin.iter().all(|i| i == first) => (*first).clone(),
            _ => meta.ignore.clone(),
        }
    };
    let per_flavor: Vec<(String, Vec<String>)> = all_flavors
        .iter()
        .map(|f| (f.clone(), ignore_of(f)))
        .collect();
    if !meta.games.is_empty() {
        let main: Vec<&str> = per_flavor
            .iter()
            .filter(|(_, ignore)| *ignore == meta.ignore)
            .map(|(f, _)| f.as_str())
            .collect();
        report.warnings.push(Diagnostic::note(format!(
            "the CI workflows package games with different project files; `{}`'s `ignore` applies to {}, the others' to the rest (`build.flavor-files`)",
            meta.file,
            if main.is_empty() { "no flavor".to_string() } else { main.join(", ") }
        )));
    }
    let mut patterns: Vec<String> = Vec::new();
    for (_, ignore) in &per_flavor {
        for p in ignore {
            if !patterns.contains(p) {
                patterns.push(p.clone());
            }
        }
    }
    let mut exclude: Vec<String> = Vec::new();
    let mut flavor_files = Map::new();
    for p in &patterns {
        let keeping: Vec<&String> = per_flavor
            .iter()
            .filter(|(_, ignore)| !ignore.contains(p))
            .map(|(f, _)| f)
            .collect();
        if keeping.is_empty() {
            exclude.extend(pattern_of(p));
        } else {
            for glob in pattern_of(p) {
                flavor_files.insert(glob, json!(keeping));
            }
        }
    }
    if !exclude.is_empty() {
        build.insert("exclude".into(), json!(exclude));
    }
    if !flavor_files.is_empty() {
        let existing = build
            .entry("flavor-files")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("an object");
        for (glob, keeping) in flavor_files {
            existing.entry(glob).or_insert(keeping);
        }
    }
    // Packager flavor blocks (`@retail@`, `@non-retail@`, `@version-…@`)
    // need a compile per flavor.
    let flavor_blocks = meta.per_game
        && all_flavors.len() > 1
        && moved.iter().any(|rel| {
            let lower = rel.to_ascii_lowercase();
            (lower.ends_with(".lua") || lower.ends_with(".xml"))
                && std::fs::read_to_string(dir.join(src_dir).join(rel))
                    .is_ok_and(|text| crate::keywords::has_flavor_blocks(&text))
        });
    if flavor_blocks {
        build.insert("flavor-mode".into(), json!("split"));
    }
    // `plain-copy` patterns that match sources keep those files
    // unprocessed (`compat.plain-copy`); the rest become assets.
    let plain: Vec<String> = meta
        .plain_copy
        .iter()
        .filter(|p| {
            !moved.iter().any(|m| {
                globset::Glob::new(p.trim_end_matches('/'))
                    .map(|g| g.compile_matcher().is_match(m))
                    .unwrap_or(false)
            })
        })
        .flat_map(|p| pattern_of(p))
        .filter(|p| !p.starts_with(&format!("{src_dir}/")))
        // Keybinding files are copied anyway (processed unless plain-copy).
        .filter(|p| !crate::build::is_bindings_file(p))
        .collect();
    // The license `license-output` names ships with the addon too.
    let mut plain = plain;
    if let Some(license) = &meta.license_output
        && dir.join(license).is_file()
        && !plain.contains(license)
    {
        plain.push(license.clone());
    }
    if !plain.is_empty() {
        let assets = build
            .entry("assets")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("an array");
        for p in plain {
            if !assets.iter().any(|a| a.as_str() == Some(&p)) {
                assets.push(json!(p));
            }
        }
    }
    if build.is_empty() {
        object.shift_remove("build");
    }
    // The packager processes every converted project's files and its
    // externals' files (keywords, blocks, line endings, decision T120).
    let mut compat = Map::new();
    compat.insert("packager-keywords".into(), json!(true));
    compat.insert(
        "line-endings".into(),
        json!(if meta.unix { "lf" } else { "crlf" }),
    );
    if !meta.plain_copy.is_empty() {
        compat.insert("plain-copy".into(), json!(meta.plain_copy));
    }
    if let Some(license) = &meta.license_output {
        compat.insert("license-output".into(), json!(license));
    }
    if meta.per_game {
        compat.insert("per-game".into(), json!(meta.per_game_flavors));
    }
    if meta.enable_nolib {
        compat.insert("nolib-package".into(), json!(true));
    }
    if let Some(name) = &meta.run_name {
        compat.insert("archive-name".into(), json!(name));
    }
    if let Some(title) = &meta.changelog_title {
        compat.insert("changelog-title".into(), json!(title));
    }
    object.insert("compat".into(), Value::Object(compat));
    // Publishing.
    let publish = object
        .entry("publish")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("an object");
    if let Some((file, markup)) = &meta.manual_changelog {
        publish.insert("changelog".into(), json!(file));
        let format = match markup.as_deref() {
            Some("markdown") => Some("markdown"),
            Some("html") => Some("html"),
            Some("text" | "plain") => Some("text"),
            _ => None,
        };
        if let Some(f) = format {
            publish.insert("changelog-format".into(), json!(f));
        }
    }
    // CurseForge relations as the packager sends them: the `.pkgmeta`
    // lists, overridden by `@curseforge-project-slug: x@` markers in the
    // project's Lua files and by every external's slug (embedded).
    let mut relations: Vec<(String, String)> = meta.relations.clone();
    let mut embed = |slug: &str| {
        let slug = slug.to_ascii_lowercase();
        relations.retain(|(s, _)| *s != slug);
        relations.push((slug, "embeddedLibrary".into()));
    };
    let package = meta
        .package_as
        .clone()
        .unwrap_or_else(|| report.addon_name.clone());
    for slug in embedded_slug_markers(dir) {
        if !slug.eq_ignore_ascii_case(&package) {
            embed(&slug);
        }
    }
    for e in &meta.externals {
        if let Some(slug) = &e.slug {
            embed(slug);
        }
    }
    {
        let targets = publish
            .entry("targets")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("an object");
        // The packager uploads tagged builds to the GitHub repository of
        // `origin` (decision T157).
        if let Some(repository) = crate::vcs::github_repository(dir)
            && !targets.values().any(|t| t["type"] == "github")
        {
            targets.insert(
                "github".into(),
                json!({ "type": "github", "repository": repository }),
            );
        }
        // Project IDs every packager run passes (`-p` / `-w` / `-a`; 0 unsets).
        for (site, id) in [
            ("curseforge", &meta.run_curseforge),
            ("wowinterface", &meta.run_wowinterface),
            ("wago", &meta.run_wago),
        ] {
            let Some(id) = id else { continue };
            if id == "0" {
                targets.shift_remove(site);
                continue;
            }
            let value = match id.parse::<u64>() {
                Ok(n) if site != "wago" => json!(n),
                _ => json!(id),
            };
            targets
                .entry(site)
                .or_insert_with(|| json!({ "type": site }))
                .as_object_mut()
                .expect("an object")
                .insert("project-id".into(), value);
        }
        // The `.pkgmeta` WoWInterface settings.
        if let Some(wowi) = targets
            .get_mut("wowinterface")
            .and_then(Value::as_object_mut)
        {
            if meta.wowi_markdown {
                wowi.insert("changelog-output".into(), json!("markdown"));
            }
            // Without a manual changelog: no generated one.
            if meta.wowi_no_changelog && meta.manual_changelog.is_none() {
                wowi.insert("changelog".into(), Value::Null);
            }
            if meta.wowi_keep_previous {
                wowi.insert("archive-previous".into(), json!(false));
            }
        }
        if !relations.is_empty() {
            match targets.get_mut("curseforge").and_then(Value::as_object_mut) {
                Some(cf) => {
                    let list: Vec<Value> = relations
                        .iter()
                        .map(|(slug, kind)| json!({ "slug": slug, "type": kind }))
                        .collect();
                    cf.insert("relations".into(), Value::Array(list));
                }
                None => report.warnings.push(Diagnostic::plain_warning(
                    "the `.pkgmeta` relation lists need a CurseForge target (`## X-Curse-Project-ID`); not converted",
                )),
            }
        }
    }
    if publish
        .get("targets")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        publish.shift_remove("targets");
    }
    if publish.is_empty() {
        object.shift_remove("publish");
    }
    // Release builds stay what the packager ships: no bundling,
    // minification or optimization (decision T157).
    object
        .entry("profiles")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("an object")
        .insert(
            "release".into(),
            json!({
                "bundle": false,
                "minify": "none",
                "optimize": { "inline": false, "cache-globals": "off", "dead-code": false }
            }),
        );
    let root_addon = report.addon_name.clone();
    let members = init_members(dir, meta, member_dirs, Some((&root_addon, src_dir)), report);
    if !members.is_empty() || !report.external_addons.is_empty() {
        let mut ws = Map::new();
        ws.insert("members".into(), json!(members));
        if !report.external_addons.is_empty() {
            let ext: Map<String, Value> = report.external_addons.iter().cloned().collect();
            ws.insert("external-addons".into(), Value::Object(ext));
        }
        object.insert("workspace".into(), Value::Object(ws));
        report.members = members;
    }
}

/// Adopts the folders `move-folders` turns into addons as workspace
/// members, each with the externals and `ignore` entries inside it. Other
/// entries (except flattened library folders, see [`flatten_externals`])
/// are W4332.
fn init_members(
    dir: &Path,
    meta: &crate::pkgmeta::PkgMeta,
    member_dirs: &[(String, String)],
    root: Option<(&str, &str)>,
    report: &mut InitReport,
) -> Vec<String> {
    let mut members = Vec::new();
    // The AddOns of the package, side by side once packaged; the root
    // addon (name, source directory) is already adopted.
    let package: Vec<Neighbor> = root
        .map(|(name, src)| Neighbor {
            name: name.to_string(),
            dir: dir.to_path_buf(),
            src: Some(src.to_string()),
        })
        .into_iter()
        .chain(member_dirs.iter().map(|(rel, name)| Neighbor {
            name: name.clone(),
            dir: dir.join(rel),
            src: None,
        }))
        .collect();
    for (from, to) in &meta.move_folders {
        let Some((rel, _)) = member_dirs.iter().find(|(_, t)| t == to) else {
            // The packager skips entries whose folder does not exist.
            let package = meta
                .package_as
                .clone()
                .or_else(|| root.map(|(name, _)| name.to_string()))
                .unwrap_or_default();
            let exists = from
                .strip_prefix(&format!("{package}/"))
                .map(|s| s.trim_matches('/'))
                .is_some_and(|s| s.is_empty() || dir.join(s).exists());
            if exists && !is_flattening(from, to) {
                report.warnings.push(Diagnostic::warning(
                    "W4332",
                    format!("`move-folders` entry `{from}: {to}` does not move a whole addon folder; not converted"),
                ));
            }
            continue;
        };
        let prefix = format!("{}/", rel.to_ascii_lowercase());
        let inside = |p: &str| p.to_ascii_lowercase().starts_with(&prefix);
        let inherited = crate::pkgmeta::PkgMeta {
            file: meta.file.clone(),
            externals: meta
                .externals
                .iter()
                .filter(|e| inside(&e.dir))
                .map(|e| crate::pkgmeta::External {
                    dir: e.dir[prefix.len()..].to_string(),
                    ..e.clone()
                })
                .collect(),
            ignore: meta
                .ignore
                .iter()
                .filter(|p| inside(p))
                .map(|p| p[prefix.len()..].to_string())
                .collect(),
            per_game: meta.per_game,
            per_game_flavors: meta.per_game_flavors.clone(),
            split: meta.split,
            unix: meta.unix,
            games: meta
                .games
                .iter()
                .map(|(flavor, ignore)| {
                    let inner = ignore
                        .iter()
                        .filter(|p| inside(p))
                        .map(|p| p[prefix.len()..].to_string())
                        .collect();
                    (flavor.clone(), inner)
                })
                .collect(),
            // Patterns without a folder apply everywhere.
            plain_copy: meta
                .plain_copy
                .iter()
                .filter_map(|p| {
                    if inside(p) {
                        Some(p[prefix.len()..].to_string())
                    } else {
                        (!p.contains('/')).then(|| p.clone())
                    }
                })
                .collect(),
            ..Default::default()
        };
        // Per-game `ignore` lists matter only where they differ.
        let mut inherited = inherited;
        if inherited.games.values().all(|i| *i == inherited.ignore) {
            inherited.games.clear();
        }
        match init_package(&InitRequest {
            dir: dir.join(rel),
            from_toc: None,
            from_pkgmeta: false,
            pkgmeta_file: None,
            inherited: Some(inherited),
            addon_name: Some(to.clone()),
            neighbors: package.iter().filter(|n| n.name != *to).cloned().collect(),
        }) {
            Ok(member) => {
                report.warnings.extend(member.warnings);
                report.dependencies += member.dependencies;
                members.push(rel.clone());
            }
            Err(d) => report.warnings.push(Diagnostic::warning(
                "W4332",
                format!("`move-folders` entry `{from}`: {}", d.message),
            )),
        }
    }
    // One lockfile for the workspace: the members' pins are collected and
    // settled by `write_pins`.
    for rel in &members {
        let member = dir.join(rel);
        if let Ok(Some(lock)) = crate::lockfile::Lockfile::read(&member) {
            report.member_pins.push((
                rel.clone(),
                lock.flavor_table.interfaces.into_iter().collect(),
            ));
            let _ = std::fs::remove_file(member.join(crate::lockfile::LOCKFILE));
        }
    }
    members
}

/// The slugs of `@curseforge-project-slug: x@` markers in the Lua files of
/// a repository (libraries copied into it, which the packager reports as
/// embedded).
fn embedded_slug_markers(dir: &Path) -> Vec<String> {
    let mut slugs = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        .flatten()
    {
        if !entry.file_type().is_file() || !entry.file_name().to_string_lossy().ends_with(".lua") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let lower = text.to_ascii_lowercase();
        let Some(i) = lower.find("@curseforge-project-slug") else {
            continue;
        };
        let rest = &text[i + "@curseforge-project-slug".len()..];
        let rest = rest.trim_start_matches([' ', '\t']);
        let Some(rest) = rest.strip_prefix(':') else {
            continue;
        };
        let Some(end) = rest.find('@') else { continue };
        let slug: String = rest[..end].chars().filter(|c| !c.is_whitespace()).collect();
        if !slug.is_empty() && !slugs.contains(&slug) {
            slugs.push(slug);
        }
    }
    slugs
}

/// Writes `wowlua.lock` with the root's interface pins and those of the
/// workspace members. A member whose numbers for a flavor differ from the
/// lockfile's gets them as explicit `flavors.<id>.interface` instead.
fn write_pins(
    dir: &Path,
    mut pins: BTreeMap<String, Vec<u32>>,
    report: &mut InitReport,
) -> Result<(), Diagnostic> {
    for (rel, member) in std::mem::take(&mut report.member_pins) {
        let mut explicit: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for (flavor, list) in member {
            match pins.get(&flavor) {
                None => {
                    pins.insert(flavor, list);
                }
                Some(l) if *l == list => {}
                Some(_) => {
                    explicit.insert(flavor, list);
                }
            }
        }
        if explicit.is_empty() {
            continue;
        }
        let path = dir.join(&rel).join(MANIFEST_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut m) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if let Some(flavors) = m.get_mut("flavors").and_then(Value::as_object_mut) {
            for (flavor, list) in explicit {
                if let Some(entry) = flavors.get_mut(&flavor).and_then(Value::as_object_mut) {
                    entry.insert("interface".into(), json!(list));
                }
            }
        }
        std::fs::write(&path, pretty(&m)).map_err(|e| {
            Diagnostic::plain_error(format!("cannot write `{}`: {e}", path.display()))
        })?;
    }
    if pins.is_empty() {
        return Ok(());
    }
    let flavors: Vec<String> = pins.keys().cloned().collect();
    let mut lock = crate::lockfile::required(None, &flavors, &[]);
    for (flavor, list) in pins {
        lock.flavor_table.interfaces.insert(flavor, list);
    }
    lock.write(dir).map_err(|e| {
        Diagnostic::plain_error(format!("cannot write `{}`: {e}", crate::lockfile::LOCKFILE))
    })
}

/// `move-folders` entries that make (part of) an external an AddOn of its
/// own (DBM's voice packs) become external addons (decision T127); the
/// external and the entry are consumed.
fn extract_external_addons(
    meta: &mut crate::pkgmeta::PkgMeta,
    package: &str,
) -> Vec<(String, Value)> {
    let prefix = format!("{package}/");
    let mut out = Vec::new();
    let mut consumed_moves = Vec::new();
    let mut consumed_externals = Vec::new();
    for (i, (from, to)) in meta.move_folders.iter().enumerate() {
        if to.contains('/') {
            continue;
        }
        let rel = from.strip_prefix(&prefix).unwrap_or(from).trim_matches('/');
        let Some((j, e)) = meta.externals.iter().enumerate().find(|(_, e)| {
            let d = e.dir.to_ascii_lowercase();
            let r = rel.to_ascii_lowercase();
            r == d || r.starts_with(&format!("{d}/"))
        }) else {
            continue;
        };
        let inner = rel[e.dir.len()..].trim_matches('/').to_string();
        let mut external = e.clone();
        if !inner.is_empty() {
            use crate::pkgmeta::ExternalSource;
            match &mut external.source {
                ExternalSource::Git { subdir, .. } | ExternalSource::Zip { subdir, .. } => {
                    *subdir = Some(match subdir.take() {
                        Some(s) => format!("{s}/{inner}"),
                        None => inner.clone(),
                    });
                }
                ExternalSource::Svn { url } => *url = format!("{url}/{inner}"),
                ExternalSource::Hg { .. } => {}
            }
        }
        let Ok(mut entry) =
            crate::pkgmeta::dependency_entry(&external, wowlua_pkg::fetch::sha256_of_url)
        else {
            continue;
        };
        entry.remove("kind");
        entry.remove("curseforge-slug");
        out.push((to.clone(), Value::Object(entry)));
        consumed_moves.push(i);
        consumed_externals.push(j);
    }
    let mut i = 0;
    meta.move_folders.retain(|_| {
        i += 1;
        !consumed_moves.contains(&(i - 1))
    });
    let mut j = 0;
    meta.externals.retain(|_| {
        j += 1;
        !consumed_externals.contains(&(j - 1))
    });
    out
}

/// `move-folders` entries that move a folder up into its parent
/// (`X/Libs/Lib/Lib: X/Libs/Lib`), which flattens a library checkout.
fn is_flattening(from: &str, to: &str) -> bool {
    to.contains('/') && from.starts_with(&format!("{to}/"))
}

/// Flattening `move-folders` entries become the `subdir` (`path`) of the
/// external at the destination. `members` maps addon names to their folders;
/// the root addon's files are at the repository root when `root_addon`.
fn flatten_externals(
    meta: &mut crate::pkgmeta::PkgMeta,
    members: &[(String, String)],
    root_addon: bool,
) {
    let package = meta.package_as.clone().unwrap_or_default();
    let entries: Vec<(String, String)> = meta
        .move_folders
        .iter()
        .filter(|(f, t)| is_flattening(f, t))
        .cloned()
        .collect();
    for (from, to) in entries {
        let inner = from[to.len() + 1..].to_string();
        let (addon, path) = to.split_once('/').unwrap_or((to.as_str(), ""));
        let base = match members.iter().find(|(_, t)| t == addon) {
            Some((rel, _)) if !(root_addon && addon == package) => format!("{rel}/"),
            _ if addon == package => String::new(),
            _ => continue,
        };
        let target = format!("{base}{path}").to_ascii_lowercase();
        if let Some(e) = meta
            .externals
            .iter_mut()
            .find(|e| e.dir.to_ascii_lowercase() == target)
        {
            use crate::pkgmeta::ExternalSource;
            match &mut e.source {
                ExternalSource::Git { subdir, .. } | ExternalSource::Zip { subdir, .. } => {
                    *subdir = Some(match subdir.take() {
                        Some(s) => format!("{s}/{inner}"),
                        None => inner.clone(),
                    });
                    e.moved_up = Some(inner.clone());
                }
                ExternalSource::Svn { url } => {
                    *url = format!("{url}/{inner}");
                }
                ExternalSource::Hg { .. } => {}
            }
        }
    }
}

/// A repository of several addons and no TOC of its own (WeakAuras, DBM,
/// ElvUI): `move-folders` makes every folder an addon, and the repository
/// root becomes a workspace root (spec CLI §12.6).
fn init_suite(dir: &Path, mut meta: crate::pkgmeta::PkgMeta) -> Result<InitReport, Diagnostic> {
    let package = meta.package_as.clone().unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let external_addons = extract_external_addons(&mut meta, &package);
    let members = member_folders(dir, &meta, &package);
    if members.is_empty() {
        return Err(Diagnostic::plain_error(format!(
            "no .toc file found in `{}`, and no `move-folders` entry of `{}` moves an addon folder",
            dir.display(),
            meta.file
        )));
    }
    flatten_externals(&mut meta, &members, false);
    // Externals at the top of the package land in the folder named like
    // the package, which `move-folders` merges the package's own addon into
    // (`DBM-Core/DBM-Core: DBM-Core`).
    if let Some((rel, _)) = members.iter().find(|(_, t)| *t == package) {
        let rel = rel.clone();
        for e in &mut meta.externals {
            let inside = members.iter().any(|(r, _)| {
                e.dir
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}/", r.to_ascii_lowercase()))
            });
            if !inside {
                e.dir = format!("{rel}/{}", e.dir);
            }
        }
    }
    let mut report = InitReport {
        addon_name: package,
        external_addons,
        ..Default::default()
    };
    for e in &meta.externals {
        let inside = members.iter().any(|(rel, _)| {
            e.dir
                .to_ascii_lowercase()
                .starts_with(&format!("{}/", rel.to_ascii_lowercase()))
        });
        if !inside {
            report.warnings.push(Diagnostic::plain_warning(format!(
                "external `{}` is outside every addon folder; not converted",
                e.dir
            )));
        }
    }
    let converted = init_members(dir, &meta, &members, None, &mut report);
    let mut manifest = Map::new();
    manifest.insert("$schema".into(), schema_url().into());
    let mut ws = Map::new();
    ws.insert("members".into(), json!(converted));
    if !report.external_addons.is_empty() {
        let ext: Map<String, Value> = report.external_addons.iter().cloned().collect();
        ws.insert("external-addons".into(), Value::Object(ext));
        // Shipped like the packager ships externals.
        manifest.insert("compat".into(), json!({ "packager-keywords": true }));
    }
    // The package folder becomes the member of that name; it gets the
    // packager's changelog and license files.
    let compat = manifest
        .entry("compat")
        .or_insert_with(|| json!({ "packager-keywords": true }))
        .as_object_mut()
        .expect("an object");
    compat.insert(
        "line-endings".into(),
        json!(if meta.unix { "lf" } else { "crlf" }),
    );
    if let Some(name) = &meta.package_as {
        compat.insert("package-as".into(), json!(name));
    }
    if let Some(license) = &meta.license_output {
        compat.insert("license-output".into(), json!(license));
    }
    if let Some(title) = &meta.changelog_title {
        compat.insert("changelog-title".into(), json!(title));
    }
    manifest.insert("workspace".into(), Value::Object(ws));
    // The root's own files ship with the package-as AddOn, minus `ignore`.
    let exclude: Vec<String> = meta
        .ignore
        .iter()
        .flat_map(|p| {
            let p = p
                .trim_end_matches("/*")
                .trim_end_matches('/')
                .replace('\\', "/");
            if dir.join(&p).is_dir() {
                vec![format!("{p}/**")]
            } else {
                vec![p.clone(), format!("{p}/**")]
            }
        })
        .collect();
    if !exclude.is_empty() {
        manifest.insert("build".into(), json!({ "exclude": exclude }));
    }
    if let Some((file, _)) = &meta.manual_changelog {
        manifest.insert("publish".into(), json!({ "changelog": file }));
    }
    report.members = converted;
    std::fs::write(dir.join(MANIFEST_FILE), pretty(&Value::Object(manifest)))
        .map_err(|e| Diagnostic::plain_error(format!("cannot write `{MANIFEST_FILE}`: {e}")))?;
    write_pins(dir, BTreeMap::new(), &mut report)?;
    Ok(report)
}

/// The `move-folders` entries that move a whole addon folder: (folder
/// relative to `dir`, addon name).
fn member_folders(
    dir: &Path,
    meta: &crate::pkgmeta::PkgMeta,
    addon: &str,
) -> Vec<(String, String)> {
    let prefix = format!(
        "{}/",
        meta.package_as.clone().unwrap_or_else(|| addon.to_string())
    );
    meta.move_folders
        .iter()
        .filter_map(|(from, to)| {
            // Paths outside the package folder move nothing (the packager
            // finds no such folder).
            let rel = from.strip_prefix(&prefix)?.trim_matches('/').to_string();
            let has_toc = std::fs::read_dir(dir.join(&rel))
                .ok()
                .is_some_and(|entries| {
                    entries.flatten().any(|e| {
                        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                        n.ends_with(".toc") && n.starts_with(&to.to_ascii_lowercase())
                    })
                });
            (!rel.is_empty() && has_toc).then(|| (rel, to.clone()))
        })
        .collect()
}

/// Fields the flavors' TOCs set differently stay per flavor: SavedVariables
/// lists as flavor overrides (decision T128), publish IDs as publish targets
/// restricted to their flavors (T129).
fn per_flavor_toc_fields(manifest: &mut Value, views: &[(wowlua_toc::ParsedToc, Vec<String>)]) {
    let used: Vec<&(wowlua_toc::ParsedToc, Vec<String>)> =
        views.iter().filter(|(_, f)| !f.is_empty()).collect();
    if used.len() < 2 {
        return;
    }
    let all: Vec<String> = used.iter().flat_map(|(_, f)| f.clone()).collect();
    let Some(object) = manifest.as_object_mut() else {
        return;
    };
    for (directive, key) in [
        ("SavedVariables", "saved-variables"),
        (
            "SavedVariablesPerCharacter",
            "saved-variables-per-character",
        ),
    ] {
        let lists: Vec<(Vec<String>, &Vec<String>)> = used
            .iter()
            .map(|(t, f)| (t.list(|k| k.eq_ignore_ascii_case(directive)), f))
            .collect();
        if lists.iter().all(|(l, _)| *l == lists[0].0) {
            continue;
        }
        // The list most flavors use is the base.
        let base = lists
            .iter()
            .max_by_key(|(l, _)| {
                lists
                    .iter()
                    .filter(|(o, _)| o == l)
                    .map(|(_, f)| f.len())
                    .sum::<usize>()
            })
            .map(|(l, _)| l.clone())
            .unwrap_or_default();
        let addon = object
            .entry("addon")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("an object");
        if base.is_empty() {
            addon.shift_remove(key);
        } else {
            addon.insert(key.into(), json!(base));
        }
        for (list, flavors) in &lists {
            if *list == base {
                continue;
            }
            let overrides = addon
                .entry("flavor-overrides")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("an object");
            for f in flavors.iter() {
                overrides
                    .entry(f.clone())
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .expect("an object")
                    .insert(key.into(), json!(list));
            }
        }
    }
    for (directive, name) in [
        ("X-Curse-Project-ID", "curseforge"),
        ("X-Wago-ID", "wago"),
        ("X-WoWI-ID", "wowinterface"),
    ] {
        let values: Vec<(Option<String>, &Vec<String>)> = used
            .iter()
            .map(|(t, f)| (t.get(directive).map(|v| v.trim().to_string()), f))
            .collect();
        if values.iter().all(|(v, _)| *v == values[0].0) {
            continue;
        }
        let mut groups: IndexMap<String, Vec<String>> = IndexMap::new();
        for (value, flavors) in &values {
            if let Some(v) = value {
                groups
                    .entry(v.clone())
                    .or_default()
                    .extend(flavors.iter().cloned());
            }
        }
        let publish = object
            .entry("publish")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("an object");
        let targets = publish
            .entry("targets")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("an object");
        targets.shift_remove(name);
        for (i, (value, flavors)) in groups.into_iter().enumerate() {
            let id = value
                .parse::<u64>()
                .map(Value::from)
                .unwrap_or_else(|_| Value::from(value.clone()));
            let mut target = Map::new();
            target.insert("type".into(), json!(name));
            target.insert("project-id".into(), id);
            if flavors.len() < all.len() {
                let ordered: Vec<&String> = all.iter().filter(|f| flavors.contains(*f)).collect();
                target.insert("flavors".into(), json!(ordered));
            }
            let key = if i == 0 {
                name.to_string()
            } else {
                format!("{name}-{}", flavors.join("-"))
            };
            targets.insert(key, Value::Object(target));
        }
        if targets.is_empty() {
            publish.shift_remove("targets");
        }
        if publish.is_empty() {
            object.shift_remove("publish");
        }
    }
}

/// A TOC line as a `build.load-order` entry: its path after `init` moved
/// it, or a library entry's output path.
fn order_entry(
    p: &str,
    external_paths: &HashMap<String, String>,
    moved: &[String],
    src_dir: &str,
) -> Option<String> {
    if let Some(external) = external_paths.get(p) {
        Some(external.clone())
    } else if p.contains('[') {
        Some(format!("{src_dir}/{p}"))
    } else {
        moved
            .iter()
            .find(|m| m.eq_ignore_ascii_case(p))
            .map(|m| format!("{src_dir}/{m}"))
    }
}

/// The TOC files of the addon in `dir`: `(path, addon name, suffix)`.
fn find_tocs(
    dir: &Path,
    explicit: Option<&Path>,
    name: Option<&str>,
) -> Result<Vec<(PathBuf, String, Option<String>)>, Diagnostic> {
    let folder = name.map(str::to_string).unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let mut found: Vec<(PathBuf, String, Option<String>)> = std::fs::read_dir(dir)
        .map_err(|e| Diagnostic::plain_error(format!("cannot read `{}`: {e}", dir.display())))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let (base, suffix) = split_toc_name(&file_name, &folder)?;
            Some((entry.path(), base, suffix))
        })
        .collect();
    found.sort();
    if found.is_empty() {
        return Err(
            Diagnostic::plain_error(format!("no .toc file found in `{}`", dir.display()))
                .with_help("run `wowlua init` inside the addon's folder, or use `wowlua new`"),
        );
    }
    let wanted = match explicit {
        Some(path) => {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            wowlua_toc::split_file_name(&name)
                .map(|(base, _)| base)
                .ok_or_else(|| {
                    Diagnostic::plain_error(format!("`{}` is not a TOC file", path.display()))
                })?
        }
        None => found
            .iter()
            .find(|(_, base, _)| *base == folder)
            .or_else(|| found.first())
            .map(|(_, base, _)| base.clone())
            .expect("found is not empty"),
    };
    let mut selected: Vec<_> = found
        .into_iter()
        .filter(|(_, base, _)| *base == wanted)
        .collect();
    // The plain TOC first, so it provides the metadata.
    selected.sort_by_key(|(_, _, suffix)| suffix.is_some());
    Ok(selected)
}

/// The text of each TOC as `init` reads it. A packager project's TOC is
/// read as the packager packages it; one with `## Interface-<Type>:` lines
/// stands for a TOC per game type, like the packager's split and
/// single-game builds make: each with that type's interface numbers and
/// flavor blocks, under the type's suffix, besides the plain (retail) TOC
/// when it has `## Interface:` numbers of its own.
#[allow(clippy::type_complexity)]
fn toc_views(
    tocs: &[(PathBuf, String, Option<String>)],
    packaged: bool,
    per_game: Option<bool>,
) -> Result<Vec<((PathBuf, String, Option<String>), String)>, Diagnostic> {
    let table = wowlua_flavors::table();
    let mut out = Vec::new();
    for entry in tocs {
        let (path, _, suffix) = entry;
        let text = std::fs::read_to_string(path).map_err(|e| {
            Diagnostic::plain_error(format!("cannot read `{}`: {e}", path.display()))
        })?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let text = crate::keywords::toc_version_placeholders(text);
        // The packager processes only the project's TOCs: the plain one and
        // those with its suffixes; others keep their blocks as written.
        let project_toc = suffix.as_deref().is_none_or(|s| {
            let name = s.trim_start_matches(['_', '-']).to_ascii_lowercase();
            (s.starts_with('_') || s.starts_with('-'))
                && [
                    "mainline", "classic", "vanilla", "bcc", "tbc", "wrath", "wotlkc", "cata",
                    "mists", "camelot",
                ]
                .contains(&name.as_str())
        });
        if !packaged || !project_toc {
            out.push((entry.clone(), text));
            continue;
        }
        // `Some(true)`: single-game builds; `Some(false)`: split builds,
        // which make no TOC for `Interface-Titan`.
        let typed: Vec<(&'static str, String)> = if suffix.is_none()
            && let Some(single) = per_game
        {
            wowlua_toc::parse(&text)
                .directives
                .iter()
                .filter_map(|d| {
                    let kind = d
                        .key
                        .get(..10)
                        .filter(|k| k.eq_ignore_ascii_case("Interface-"))
                        .map(|_| &d.key[10..])?;
                    if !single && kind.eq_ignore_ascii_case("titan") {
                        return None;
                    }
                    Some((
                        crate::keywords::interface_type_flavor(kind)?,
                        d.value.trim().to_string(),
                    ))
                })
                .collect()
        } else {
            Vec::new()
        };
        if typed.is_empty() {
            out.push((entry.clone(), crate::keywords::packaged_toc(&text)));
            continue;
        }
        let without_typed = |t: &str| -> String {
            t.split_inclusive('\n')
                .filter(|l| {
                    !l.trim_start()
                        .get(..13)
                        .is_some_and(|p| p.eq_ignore_ascii_case("## Interface-"))
                })
                .collect()
        };
        // The plain TOC, for retail, when it has interface numbers.
        if !wowlua_toc::parse(&text).interfaces().is_empty() {
            let plain = crate::keywords::packaged_toc_for(&text, Some("standard"));
            out.push((entry.clone(), without_typed(&plain)));
        }
        for (flavor, numbers) in typed {
            let Some(game_suffix) = table
                .flavor(flavor)
                .and_then(|f| f.game_suffixes.first().cloned())
            else {
                continue;
            };
            // A TOC of the game's own in the source tree wins.
            let own = tocs.iter().any(|(_, base, s)| {
                *base == entry.1
                    && s.as_deref()
                        .and_then(|s| table.suffix_meaning(s))
                        .is_some_and(|m| {
                            matches!(m, wowlua_flavors::SuffixMeaning::Flavor(f) if f.id == flavor)
                        })
            });
            if own {
                continue;
            }
            let view = without_typed(&crate::keywords::packaged_toc_for(&text, Some(flavor)));
            let mut replaced = false;
            let mut lines: String = view
                .split_inclusive('\n')
                .map(|l| {
                    let is_interface = l
                        .trim_start()
                        .get(..13)
                        .is_some_and(|p| p.eq_ignore_ascii_case("## Interface:"));
                    if is_interface && !replaced {
                        replaced = true;
                        format!("## Interface: {numbers}\n")
                    } else if is_interface {
                        String::new()
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            if !replaced {
                lines = format!("## Interface: {numbers}\n{lines}");
            }
            out.push(((path.clone(), entry.1.clone(), Some(game_suffix)), lines));
        }
    }
    Ok(out)
}

/// The folder `init` moves sources into: `src`, or `wowlua-src` when the
/// addon already has a `src` folder of its own (AllTheThings).
fn source_dir_for(dir: &Path) -> &'static str {
    let taken = std::fs::read_dir(dir).ok().is_some_and(|entries| {
        entries
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("src"))
    });
    if taken { "wowlua-src" } else { "src" }
}

/// Follows XML includes from `queue` and returns the files found (on-disk
/// spelling), skipping those already in `seen`.
fn closure(
    dir: &Path,
    mut queue: Vec<String>,
    seen: &mut BTreeSet<String>,
    warnings: &mut Vec<Diagnostic>,
    skip: &dyn Fn(&str) -> bool,
    skipped: &mut Vec<String>,
    packaged: bool,
) -> Vec<String> {
    let mut found = Vec::new();
    while let Some(rel) = queue.pop() {
        // Files of `.pkgmeta` externals are not part of the checkout.
        if skip(&rel) {
            if !skipped.contains(&rel) {
                skipped.push(rel);
            }
            continue;
        }
        let Some(actual) = existing_case(dir, &rel) else {
            warnings.push(Diagnostic::plain_warning(format!(
                "the TOC lists `{rel}`, which does not exist"
            )));
            continue;
        };
        if !seen.insert(actual.clone()) {
            continue;
        }
        if actual.to_ascii_lowercase().ends_with(".xml")
            && let Ok(text) = std::fs::read_to_string(dir.join(&actual))
            && let Ok(mut refs) = wowlua_xml::references(&text)
        {
            // A packager project's file belongs to the addon when any of
            // its builds loads it: the source tree's (`@debug@` blocks
            // active), a release's (`@non-debug@`) or a flavor's.
            if packaged {
                for view in [
                    crate::keywords::packaged_xml(&text),
                    crate::keywords::negated_xml(&text),
                ] {
                    for r in wowlua_xml::references(&view).unwrap_or_default() {
                        if !refs.iter().any(|x| x.file == r.file) {
                            refs.push(r);
                        }
                    }
                }
            }
            let base = actual
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_default();
            for r in refs {
                let joined = if base.is_empty() {
                    r.normalized()
                } else {
                    format!("{base}/{}", r.normalized())
                };
                queue.push(normalize_rel(&joined));
            }
        }
        found.push(actual);
    }
    found
}

/// Files of the addon in `dir` that other AddOns next to it load through
/// `..\<addon>\<path>` lines: from their TOCs, and from the
/// `build.external-files` of those already adopted. Paths are relative to
/// `dir`, before moving.
fn sibling_references(dir: &Path, name: &str, neighbors: &[Neighbor]) -> Vec<String> {
    let (Some(parent), Some(folder)) = (dir.parent(), dir.file_name()) else {
        return Vec::new();
    };
    let folder = if neighbors.is_empty() {
        folder.to_string_lossy().into_owned()
    } else {
        name.to_string()
    };
    let prefix = format!("../{}/", folder.to_ascii_lowercase());
    let strip = |path: &str| -> Option<String> {
        let lower = path.to_ascii_lowercase();
        lower
            .starts_with(&prefix)
            .then(|| path[prefix.len()..].to_string())
    };
    let mut found: Vec<String> = Vec::new();
    let mut siblings: Vec<PathBuf> = if neighbors.is_empty() {
        let Ok(siblings) = std::fs::read_dir(parent) else {
            return found;
        };
        siblings
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.as_path() != dir)
            .collect()
    } else {
        neighbors.iter().map(|n| n.dir.clone()).collect()
    };
    siblings.sort();
    for sibling in siblings {
        let Ok(entries) = std::fs::read_dir(&sibling) else {
            continue;
        };
        let mut tocs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("toc")))
            .collect();
        tocs.sort();
        for toc in tocs {
            let Ok(bytes) = std::fs::read(&toc) else {
                continue;
            };
            for file in wowlua_toc::parse(&String::from_utf8_lossy(&bytes)).files {
                if let Some(rel) = strip(&file.path) {
                    found.push(rel);
                }
            }
        }
        let manifest = std::fs::read_to_string(sibling.join(MANIFEST_FILE))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let externals = manifest
            .as_ref()
            .and_then(|m| m.pointer("/build/external-files"))
            .and_then(Value::as_array);
        for entry in externals.into_iter().flatten() {
            if let Some(rel) = entry.get("path").and_then(Value::as_str).and_then(strip) {
                // The sibling already points at the output location (`src/…`).
                let rel = rel.strip_prefix("src/").map(str::to_string).unwrap_or(rel);
                found.push(rel);
            }
        }
    }
    let mut unique = BTreeSet::new();
    found.retain(|rel| unique.insert(rel.to_ascii_lowercase()));
    found
}

/// The path of an external line (`../Other/file.lua`) in the other AddOn's
/// output: `../Other/src/file.lua` once that AddOn is (or will be) adopted,
/// since its `init` moves the files its siblings load into its sources.
fn resolve_external(
    dir: &Path,
    line: &str,
    neighbors: &[Neighbor],
) -> (String, Option<Diagnostic>) {
    let Some((owner, file)) = line.strip_prefix("../").and_then(|r| r.split_once('/')) else {
        return (line.to_string(), None);
    };
    let neighbor = neighbors
        .iter()
        .find(|n| n.name.eq_ignore_ascii_case(owner));
    let owner_dir = match neighbor {
        Some(n) => Some(n.dir.clone()),
        None => dir.parent().and_then(|parent| {
            std::fs::read_dir(parent)
                .ok()?
                .filter_map(Result::ok)
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(owner))
                .map(|e| e.path())
        }),
    };
    let Some(owner_dir) = owner_dir else {
        return (
            line.to_string(),
            Some(Diagnostic::plain_warning(format!(
                "the TOC loads `{line}`, but there is no AddOn `{owner}` next to this one; kept as written"
            ))),
        );
    };
    let owner_name = match neighbor {
        Some(n) => n.name.clone(),
        None => owner_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    };
    let adopted = std::fs::read_to_string(owner_dir.join(MANIFEST_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let owner_src = match (neighbor.and_then(|n| n.src.clone()), &adopted) {
        (Some(src), _) => src,
        (None, Some(manifest)) => manifest
            .pointer("/build/src")
            .and_then(Value::as_str)
            .unwrap_or("src")
            .to_string(),
        (None, None) => source_dir_for(&owner_dir).to_string(),
    };
    if let Some(actual) = existing_case(&owner_dir.join(&owner_src), file) {
        return (format!("../{owner_name}/src/{actual}"), None);
    }
    match existing_case(&owner_dir, file) {
        Some(actual) if adopted.is_none() && neighbor.is_none_or(|n| n.src.is_none()) => {
            (format!("../{owner_name}/src/{actual}"), None)
        }
        Some(actual) => (
            format!("../{owner_name}/{actual}"),
            Some(Diagnostic::plain_warning(format!(
                "`{line}` was not moved when `{owner_name}` was adopted, so its build does not ship it; add it to that package's `build.shared-files`"
            ))),
        ),
        None => (
            line.to_string(),
            Some(Diagnostic::plain_warning(format!(
                "the TOC loads `{line}`, which does not exist"
            ))),
        ),
    }
}

/// Splits a TOC file name into addon name and suffix, relative to the folder
/// the client finds it in: `LittleWigs_Classic.toc` in `LittleWigs_Classic/`
/// is the plain TOC, and `!RealUI_PreLoads.toc` in `!RealUI_Preloads/`
/// belongs to that folder (names compare case-insensitively).
fn split_toc_name(file_name: &str, folder: &str) -> Option<(String, Option<String>)> {
    let (generic_base, generic_suffix) = wowlua_toc::split_file_name(file_name)?;
    let stem = &file_name[..file_name.len() - ".toc".len()];
    if stem.eq_ignore_ascii_case(folder) {
        return Some((folder.to_string(), None));
    }
    if let Some(suffix) = stem.get(folder.len()..)
        && stem.is_char_boundary(folder.len())
        && stem[..folder.len()].eq_ignore_ascii_case(folder)
        && wowlua_flavors::table().suffix_meaning(suffix).is_some()
    {
        return Some((folder.to_string(), Some(suffix.to_string())));
    }
    Some((generic_base, generic_suffix))
}

/// Flavors that load a file: the TOC's flavors filtered by the line's inline
/// `[AllowLoadGameType]` / `[ExcludeLoadGameType]` directives.
pub fn apply_conditions(
    toc_flavors: &[String],
    conditions: &[wowlua_toc::InlineDirective],
) -> Vec<String> {
    let table = wowlua_flavors::table();
    // Only the flavors that read this TOC (an unread TOC loads nothing).
    let base: Vec<String> = toc_flavors.to_vec();
    let expand = |values: &[String]| -> Vec<String> {
        values
            .iter()
            .flat_map(|v| {
                let v = v.to_ascii_lowercase();
                table
                    .flavors
                    .iter()
                    .filter(move |f| f.id == v || f.family.name() == v)
                    .map(|f| f.id.clone())
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let mut result = base;
    for condition in conditions {
        if condition.name.eq_ignore_ascii_case("AllowLoadGameType") {
            let allowed = expand(&condition.values);
            result.retain(|f| allowed.contains(f));
        } else if condition.name.eq_ignore_ascii_case("ExcludeLoadGameType") {
            let excluded = expand(&condition.values);
            result.retain(|f| !excluded.contains(f));
        }
    }
    result
}

/// The on-disk spelling of a relative path (TOCs are case-insensitive on Windows).
fn existing_case(dir: &Path, rel: &str) -> Option<String> {
    let mut current = dir.to_path_buf();
    let mut parts = Vec::new();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        // Listing the directory gives the real spelling, also on
        // case-insensitive file systems where `exists()` accepts any case.
        let names: Vec<String> = std::fs::read_dir(&current)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let name = names
            .iter()
            .find(|n| *n == part)
            .or_else(|| names.iter().find(|n| n.eq_ignore_ascii_case(part)))?
            .clone();
        current = current.join(&name);
        parts.push(name);
    }
    current.is_file().then(|| parts.join("/"))
}

fn normalize_rel(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn remove_empty_dirs(dir: &Path, moved: &[String]) {
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for rel in moved {
        let mut path = dir.join(rel);
        while let Some(parent) = path.parent() {
            if parent == dir {
                break;
            }
            dirs.insert(parent.to_path_buf());
            path = parent.to_path_buf();
        }
    }
    // Deepest first; `remove_dir` only removes empty directories.
    for d in dirs.iter().rev() {
        let _ = std::fs::remove_dir(d);
    }
}

/// Rewrites `Interface\AddOns\<Name>\<path>` references to moved files so they
/// point at their output location `…\<Name>\src\<path>` (whatever the
/// package's source folder is called). Handles `\`, `\\` (escaped in Lua
/// strings) and `/` separators. Returns the new text and the number of rewrites.
pub fn rewrite_ingame_paths(
    text: &str,
    addon: &str,
    moved_lower: &BTreeSet<String>,
) -> (String, usize) {
    let pattern = format!(
        r#"(?i)(Interface)(\\\\|\\|/)(AddOns)(\\\\|\\|/)({})(\\\\|\\|/)([^"'\s<>\]\)]+)"#,
        regex::escape(addon)
    );
    let re = Regex::new(&pattern).expect("valid pattern");
    let mut count = 0;
    let out = re.replace_all(text, |caps: &regex::Captures| {
        let sep = &caps[6];
        let rest = &caps[7];
        let normalized = rest
            .replace("\\\\", "/")
            .replace('\\', "/")
            .to_ascii_lowercase();
        if moved_lower.contains(&normalized) {
            count += 1;
            format!(
                "{}{}{}{}{}{sep}src{sep}{rest}",
                &caps[1], &caps[2], &caps[3], &caps[4], &caps[5]
            )
        } else {
            caps[0].to_string()
        }
    });
    (out.into_owned(), count)
}

/// Globs for the asset files left in the addon folder after moving sources.
fn asset_globs(dir: &Path, src_dir: &str) -> Vec<String> {
    let mut globs: BTreeSet<String> = BTreeSet::new();
    let walker = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .require_git(false)
        .build();
    for entry in walker.filter_map(Result::ok) {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(rel) = fsutil::relative_slash(entry.path(), dir) else {
            continue;
        };
        if rel.starts_with(&format!("{src_dir}/")) || rel.starts_with("dist/") {
            continue;
        }
        let ext = rel
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !ASSET_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        match rel.split_once('/') {
            Some((first, _)) => {
                globs.insert(format!("{first}/**/*.{{{}}}", ASSET_EXTENSIONS.join(",")));
            }
            None => {
                globs.insert(rel);
            }
        }
    }
    globs.into_iter().collect()
}

/// Merges per-flavor file sequences into one order that keeps every
/// sequence's relative order (a topological merge; ties keep the order of
/// first appearance). Returns `true` as second value when the sequences
/// contradict each other; the remaining files then follow first appearance.
fn merge_orders(sequences: &[Vec<String>]) -> (Vec<String>, bool) {
    let mut first_seen: IndexMap<String, usize> = IndexMap::new();
    for sequence in sequences {
        for item in sequence {
            let next = first_seen.len();
            first_seen.entry(item.clone()).or_insert(next);
        }
    }
    let mut successors: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    let mut indegree: HashMap<&str, usize> = first_seen.keys().map(|k| (k.as_str(), 0)).collect();
    for sequence in sequences {
        for pair in sequence.windows(2) {
            if successors
                .entry(pair[0].as_str())
                .or_default()
                .insert(pair[1].as_str())
            {
                *indegree.get_mut(pair[1].as_str()).expect("known") += 1;
            }
        }
    }
    let rank = |item: &str| first_seen[item];
    let mut ready: BTreeSet<(usize, &str)> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(k, _)| (rank(k), *k))
        .collect();
    let mut order: Vec<String> = Vec::new();
    while let Some(&(r, item)) = ready.iter().next() {
        ready.remove(&(r, item));
        order.push(item.to_string());
        for next in successors.get(item).into_iter().flatten() {
            let d = indegree.get_mut(next).expect("known");
            *d -= 1;
            if *d == 0 {
                ready.insert((rank(next), next));
            }
        }
    }
    let conflict = order.len() < first_seen.len();
    if conflict {
        for item in first_seen.keys() {
            if !order.contains(item) {
                order.push(item.clone());
            }
        }
    }
    (order, conflict)
}

/// A synthetic TOC with the structural directives the flavors agree on:
/// version, flags and publish IDs by the most flavors among the TOCs that set
/// them, saved variables united.
fn structural_toc(views: &[(wowlua_toc::ParsedToc, Vec<String>)]) -> wowlua_toc::ParsedToc {
    let used: Vec<(&wowlua_toc::ParsedToc, usize)> = views
        .iter()
        .filter(|(_, f)| !f.is_empty())
        .map(|(t, f)| (t, f.len()))
        .collect();
    let mut directives = Vec::new();
    let majority = |key: &str| -> Option<String> {
        let mut counts: IndexMap<String, usize> = IndexMap::new();
        for (toc, weight) in &used {
            if let Some(v) = toc.get(key) {
                *counts.entry(v.to_string()).or_default() += weight;
            }
        }
        counts.into_iter().max_by_key(|(_, n)| *n).map(|(v, _)| v)
    };
    for key in [
        "Version",
        "LoadOnDemand",
        "DefaultState",
        "LoadSavedVariablesFirst",
        "AllowAddOnTableAccess",
        "OnlyBetaAndPTR",
        "X-Curse-Project-ID",
        "X-Wago-ID",
        "X-WoWI-ID",
    ] {
        if let Some(value) = majority(key) {
            directives.push(wowlua_toc::ParsedDirective {
                key: key.into(),
                value,
                conditions: Vec::new(),
            });
        }
    }
    for key in ["SavedVariables", "SavedVariablesPerCharacter"] {
        let mut names: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        for (toc, _) in &used {
            names.extend(toc.list(|k| k.eq_ignore_ascii_case(key)));
        }
        if !names.is_empty() {
            let value = names.into_iter().collect::<Vec<_>>().join(", ");
            directives.push(wowlua_toc::ParsedDirective {
                key: key.into(),
                value,
                conditions: Vec::new(),
            });
        }
    }
    wowlua_toc::ParsedToc {
        directives,
        files: Vec::new(),
    }
}

/// More `X-` directives than this in one TOC count as bulk data (§9.3.2).
const TOC_DATA_THRESHOLD: usize = 200;

/// When a client's TOC carries bulk `X-` data (QuestieDB stores its database
/// this way), writes it to `tocdata/<flavor>.tocdata` (flavors with identical
/// data share a file) and returns `addon.toc-data`.
fn extract_toc_data(
    dir: &Path,
    views: &[(wowlua_toc::ParsedToc, Vec<String>)],
) -> Result<Option<IndexMap<String, Vec<String>>>, Diagnostic> {
    let is_data = |key: &str| key.to_ascii_lowercase().starts_with("x-") && is_metadata(key);
    let bulk = views.iter().any(|(toc, flavors)| {
        !flavors.is_empty()
            && toc.directives.iter().filter(|d| is_data(&d.key)).count() > TOC_DATA_THRESHOLD
    });
    if !bulk {
        return Ok(None);
    }
    let mut files: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut written: Vec<(String, String)> = Vec::new(); // (content, file)
    for (toc, flavors) in views.iter().filter(|(_, f)| !f.is_empty()) {
        for flavor in flavors {
            let mut content = String::new();
            for d in toc.directives.iter().filter(|d| is_data(&d.key)) {
                if !apply_conditions(std::slice::from_ref(flavor), &d.conditions).is_empty() {
                    content.push_str(&format!("## {}: {}\n", d.key, d.value));
                }
            }
            if content.is_empty() {
                continue;
            }
            let file = match written.iter().find(|(c, _)| *c == content) {
                Some((_, file)) => file.clone(),
                None => {
                    let file = format!("tocdata/{flavor}.tocdata");
                    fsutil::write_file(&dir.join(&file), content.as_bytes()).map_err(|e| {
                        Diagnostic::plain_error(format!("cannot write `{file}`: {e}"))
                    })?;
                    written.push((content, file.clone()));
                    file
                }
            };
            files.insert(flavor.clone(), vec![file]);
        }
    }
    Ok(Some(files))
}

/// TOC metadata (title, notes, icon, `X-` fields, …) resolved per flavor:
/// `base` holds the value most flavors use; `overrides` the deviations
/// (`None` = the flavor has no such directive).
#[derive(Debug, Default)]
struct FlavorMetadata {
    base: IndexMap<String, String>,
    overrides: IndexMap<String, IndexMap<String, Option<String>>>,
}

/// Directives that are not per-flavor metadata (handled elsewhere).
fn is_metadata(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    let structural = [
        "interface",
        "version",
        "loadondemand",
        "defaultstate",
        "savedvariables",
        "savedvariablespercharacter",
        "loadsavedvariablesfirst",
        "allowaddontableaccess",
        "onlybetaandptr",
        "requireddeps",
        "loadwith",
        "loadmanagers",
        "x-curse-project-id",
        "x-wago-id",
        "x-wowi-id",
        "allowloadgametype",
        "excludeloadgametype",
    ];
    // `## Interface-<Type>:` lines are packager input it strips from the
    // packaged TOC.
    !(structural.contains(&lower.as_str())
        || lower.starts_with("interface-")
        || lower.starts_with("dep")
        || lower.starts_with("optionaldep"))
}

fn flavor_metadata(
    views: &[(wowlua_toc::ParsedToc, Vec<String>)],
    all_flavors: &[String],
    exclude_x: bool,
) -> FlavorMetadata {
    // key -> value for each flavor, from the TOC its client reads.
    let mut per_flavor: IndexMap<String, IndexMap<String, String>> = IndexMap::new();
    let mut keys: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    for (toc, flavors) in views {
        for flavor in flavors {
            let entry = per_flavor.entry(flavor.clone()).or_default();
            let wanted = |key: &str| {
                is_metadata(key) && !(exclude_x && key.to_ascii_lowercase().starts_with("x-"))
            };
            for d in toc.directives.iter().filter(|d| wanted(&d.key)) {
                let applies =
                    !apply_conditions(std::slice::from_ref(flavor), &d.conditions).is_empty();
                if applies && !entry.contains_key(&d.key) {
                    entry.insert(d.key.clone(), d.value.clone());
                    keys.insert(d.key.clone());
                }
            }
        }
    }
    let flavors: Vec<&String> = all_flavors
        .iter()
        .filter(|f| per_flavor.contains_key(*f))
        .collect();
    let mut metadata = FlavorMetadata::default();
    for key in keys {
        let values: Vec<Option<String>> = flavors
            .iter()
            .map(|f| per_flavor[*f].get(&key).cloned())
            .collect();
        // Most common value; ties go to the first flavor's value.
        let mut counts: IndexMap<Option<String>, usize> = IndexMap::new();
        for v in &values {
            *counts.entry(v.clone()).or_default() += 1;
        }
        let base = counts
            .iter()
            .max_by_key(|(v, n)| (**n, std::cmp::Reverse(values.iter().position(|x| x == *v))))
            .map(|(v, _)| v.clone())
            .unwrap_or(None);
        if let Some(value) = &base {
            metadata.base.insert(key.clone(), value.clone());
        }
        for (flavor, value) in flavors.iter().zip(values) {
            if value != base {
                metadata
                    .overrides
                    .entry((*flavor).clone())
                    .or_default()
                    .insert(key.clone(), value);
            }
        }
    }
    metadata
}

/// Writes one TOC metadata directive into an `addon` (or override) object.
fn insert_metadata(object: &mut Map<String, Value>, key: &str, value: Option<String>) {
    let value = value.map(Value::from).unwrap_or(Value::Null);
    let scalar = [
        ("title", "title"),
        ("notes", "notes"),
        ("author", "author"),
        ("icontexture", "icon-texture"),
        ("iconatlas", "icon-atlas"),
        ("category", "category"),
        ("group", "group"),
    ];
    let lower = key.to_ascii_lowercase();
    if let Some((_, field)) = scalar.iter().find(|(k, _)| *k == lower) {
        object.insert((*field).into(), value);
        return;
    }
    for (prefix, field) in [
        ("title-", "title-localized"),
        ("notes-", "notes-localized"),
        ("category-", "category-localized"),
    ] {
        if lower.starts_with(prefix) {
            let locale = &key[prefix.len()..];
            let map = object
                .entry(field)
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(map) = map {
                map.insert(locale.to_string(), value);
            }
            return;
        }
    }
    let map = object
        .entry("toc-extra")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(map) = map {
        map.insert(key.to_string(), value);
    }
}

/// Turns a TOC version into SemVer: `v15.26` → `15.26.0`, `2` → `2.0.0`.
/// Returns `None` for values such as `@project-version@`.
pub fn normalize_version(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    if semver::Version::parse(trimmed).is_ok() {
        return Some(trimmed.to_string());
    }
    let parts: Vec<&str> = trimmed.split('.').collect();
    let numeric = !parts.is_empty()
        && parts.len() <= 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !numeric {
        return None;
    }
    let mut padded: Vec<String> = parts
        .iter()
        .map(|p| p.trim_start_matches('0').to_string())
        .map(|p| if p.is_empty() { "0".into() } else { p })
        .collect();
    padded.resize(3, "0".into());
    Some(padded.join("."))
}

#[allow(clippy::too_many_arguments)]
fn adopted_manifest(
    toc: &wowlua_toc::ParsedToc,
    all_tocs: &[(wowlua_toc::ParsedToc, Vec<String>)],
    metadata: &FlavorMetadata,
    addon_name: &str,
    flavors: &[String],
    order: &[String],
    moved: &[String],
    file_flavors: &IndexMap<String, BTreeSet<String>>,
    assets: Vec<String>,
    warnings: &mut Vec<Diagnostic>,
    src_dir: &str,
    external_paths: &HashMap<String, String>,
    tag_version: Option<String>,
) -> Value {
    let package_name = kebab_case(addon_name);
    // `@project-version@` (packager keyword): the version of the latest tag.
    let keyword_version = toc
        .get("Version")
        .is_some_and(|v| v.contains("@project-version@"));
    let version = toc
        .get("Version")
        .filter(|_| !keyword_version)
        .and_then(normalize_version)
        .or_else(|| {
            keyword_version
                .then(|| tag_version.clone())
                .flatten()
                .and_then(|t| normalize_version(&t))
                .or_else(|| keyword_version.then(|| "0.1.0".to_string()))
        })
        .unwrap_or_else(|| {
            warnings.push(Diagnostic::plain_warning(match toc.get("Version") {
                Some(v) => format!("the TOC version `{v}` is not a SemVer version; using 0.1.0"),
                None => "the TOC has no `## Version`; using 0.1.0".to_string(),
            }));
            "0.1.0".into()
        });

    let mut package = Map::new();
    package.insert("name".into(), package_name.clone().into());
    package.insert("version".into(), version.into());
    if let Some(author) = metadata.base.get("Author") {
        package.insert("authors".into(), json!([author]));
    }
    package.insert("wowlua".into(), format!("^{}", minor_requirement()).into());

    let mut addon = Map::new();
    if pascal_case(&package_name) != addon_name {
        addon.insert("name".into(), addon_name.into());
    }
    // Base metadata (the value most flavors use).
    for (key, value) in &metadata.base {
        if key.eq_ignore_ascii_case("Author") {
            continue; // `package.authors`
        }
        insert_metadata(&mut addon, key, Some(value.clone()));
    }
    // The client accepts `1` and `true` (verified in game).
    let flag = |name: &str| {
        toc.get(name)
            .is_some_and(|v| v.trim() == "1" || v.trim().eq_ignore_ascii_case("true"))
    };
    if flag("LoadOnDemand") {
        addon.insert("load-on-demand".into(), true.into());
    }
    if toc
        .get("DefaultState")
        .is_some_and(|v| v.eq_ignore_ascii_case("disabled"))
    {
        addon.insert("default-state".into(), "disabled".into());
    }
    for (directive, key) in [
        ("SavedVariables", "saved-variables"),
        (
            "SavedVariablesPerCharacter",
            "saved-variables-per-character",
        ),
    ] {
        let list = toc.list(|k| k.eq_ignore_ascii_case(directive));
        if !list.is_empty() {
            addon.insert(key.into(), json!(list));
        }
    }
    for (directive, key) in [
        ("LoadSavedVariablesFirst", "load-saved-variables-first"),
        ("AllowAddOnTableAccess", "allow-table-access"),
        ("OnlyBetaAndPTR", "only-beta-and-ptr"),
    ] {
        if flag(directive) {
            addon.insert(key.into(), true.into());
        }
    }

    if !metadata.overrides.is_empty() {
        let mut overrides = Map::new();
        for (flavor, fields) in &metadata.overrides {
            let mut object = Map::new();
            for (key, value) in fields {
                insert_metadata(&mut object, key, value.clone());
            }
            overrides.insert(flavor.clone(), Value::Object(object));
        }
        addon.insert("flavor-overrides".into(), Value::Object(overrides));
    }

    let mut manifest = Map::new();
    manifest.insert("$schema".into(), schema_url().into());
    manifest.insert("package".into(), Value::Object(package));
    manifest.insert("addon".into(), Value::Object(addon));
    manifest.insert(
        "flavors".into(),
        Value::Object(flavors.iter().map(|f| (f.clone(), json!({}))).collect()),
    );

    // Dependencies from every TOC: an entry that only some flavors' TOCs (or
    // inline conditions) load gets a `flavors` restriction.
    let is_required = |k: &str| {
        let k = k.to_ascii_lowercase();
        k == "requireddeps" || (k.starts_with("dep") && !k.starts_with("dependson"))
    };
    let is_optional = |k: &str| k.to_ascii_lowercase().starts_with("optionaldep");
    // Entries keep every TOC's order (merged like file orders).
    let collect = |matches: &dyn Fn(&str) -> bool| -> IndexMap<String, BTreeSet<String>> {
        let mut entries: IndexMap<String, BTreeSet<String>> = IndexMap::new();
        let mut sequences: Vec<Vec<String>> = Vec::new();
        for (parsed, toc_flavors) in all_tocs.iter().filter(|(_, f)| !f.is_empty()) {
            let mut sequence = Vec::new();
            for directive in parsed.directives.iter().filter(|d| matches(&d.key)) {
                let applicable = apply_conditions(toc_flavors, &directive.conditions);
                for name in directive
                    .value
                    .split(',')
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                {
                    entries
                        .entry(name.to_string())
                        .or_default()
                        .extend(applicable.iter().cloned());
                    if !applicable.is_empty() && !sequence.iter().any(|n| n == name) {
                        sequence.push(name.to_string());
                    }
                }
            }
            sequences.push(sequence);
        }
        let (order, _) = merge_orders(&sequences);
        entries.sort_by_key(|name, _| order.iter().position(|n| n == name).unwrap_or(usize::MAX));
        entries
    };
    let mut deps = Map::new();
    for (key, entries, restrictable) in [
        ("required", collect(&is_required), true),
        ("optional", collect(&is_optional), true),
        (
            "load-with",
            collect(&|k: &str| k.eq_ignore_ascii_case("LoadWith")),
            false,
        ),
        (
            "load-managers",
            collect(&|k: &str| k.eq_ignore_ascii_case("LoadManagers")),
            false,
        ),
    ] {
        let list: Vec<Value> = entries
            .iter()
            .filter(|(_, set)| !set.is_empty())
            .map(|(name, set)| {
                if !restrictable || set.len() >= flavors.len() {
                    Value::from(name.clone())
                } else {
                    let ordered: Vec<&String> =
                        flavors.iter().filter(|f| set.contains(*f)).collect();
                    json!({ "name": name, "flavors": ordered })
                }
            })
            .collect();
        if !list.is_empty() {
            deps.insert(key.into(), Value::Array(list));
        }
    }
    if !deps.is_empty() {
        manifest.insert("addon-dependencies".into(), Value::Object(deps));
    }

    let mut build = Map::new();
    if src_dir != "src" {
        build.insert("src".into(), src_dir.into());
    }
    if !assets.is_empty() {
        build.insert("assets".into(), json!(assets));
    }
    let load_order: Vec<String> = order
        .iter()
        .filter_map(|p| order_entry(p, external_paths, moved, src_dir))
        .collect();
    if !load_order.is_empty() {
        build.insert("load-order".into(), json!(load_order));
    }
    let restricted: Map<String, Value> = file_flavors
        .iter()
        .filter(|(_, set)| set.len() < flavors.len())
        .filter_map(|(path, set)| {
            let actual = moved.iter().find(|m| m.eq_ignore_ascii_case(path))?;
            let ordered: Vec<&String> = flavors.iter().filter(|f| set.contains(*f)).collect();
            Some((format!("{src_dir}/{actual}"), json!(ordered)))
        })
        .collect();
    if !restricted.is_empty() {
        build.insert("flavor-files".into(), Value::Object(restricted));
    }
    if !build.is_empty() {
        manifest.insert("build".into(), Value::Object(build));
    }

    let mut targets = Map::new();
    for (directive, name, kind) in [
        ("X-Curse-Project-ID", "curseforge", "curseforge"),
        ("X-Wago-ID", "wago", "wago"),
        ("X-WoWI-ID", "wowinterface", "wowinterface"),
    ] {
        if let Some(id) = toc.get(directive) {
            let id_value = id
                .parse::<u64>()
                .map(Value::from)
                .unwrap_or_else(|_| Value::from(id));
            targets.insert(name.into(), json!({ "type": kind, "project-id": id_value }));
        }
    }
    if !targets.is_empty() {
        manifest.insert("publish".into(), json!({ "targets": targets }));
    }
    Value::Object(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_toc_versions() {
        assert_eq!(normalize_version("v15.26").as_deref(), Some("15.26.0"));
        assert_eq!(normalize_version("2").as_deref(), Some("2.0.0"));
        assert_eq!(
            normalize_version("1.2.3-beta.1").as_deref(),
            Some("1.2.3-beta.1")
        );
        assert_eq!(normalize_version("1.02.3").as_deref(), Some("1.2.3"));
        assert_eq!(normalize_version("@project-version@"), None);
        assert_eq!(normalize_version("1.2.3.4"), None);
    }

    #[test]
    fn kebab_case_names() {
        assert_eq!(kebab_case("MyAddon"), "my-addon");
        assert_eq!(kebab_case("DBM-Core"), "dbm-core");
        assert_eq!(kebab_case("my addon"), "my-addon");
    }

    #[test]
    fn rewrites_ingame_paths_to_moved_files_only() {
        let moved: BTreeSet<String> = ["core.lua".to_string(), "libs/x/x.xml".to_string()].into();
        let text = r#"local a = "Interface\\AddOns\\MyAddon\\core.lua"
<Script file="Interface\AddOns\MyAddon\Libs\X\X.xml"/>
local tex = "Interface\\AddOns\\MyAddon\\media\\icon.tga"
local other = "Interface/AddOns/OtherAddon/core.lua""#;
        let (out, count) = rewrite_ingame_paths(text, "MyAddon", &moved);
        assert_eq!(count, 2);
        assert!(out.contains(r#""Interface\\AddOns\\MyAddon\\src\\core.lua""#));
        assert!(out.contains(r#"file="Interface\AddOns\MyAddon\src\Libs\X\X.xml""#));
        assert!(out.contains(r#"MyAddon\\media\\icon.tga"#));
        assert!(out.contains("OtherAddon/core.lua"));
    }

    #[test]
    fn new_addon_template_is_a_valid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kill-tracker");
        new_package(&NewRequest {
            path: path.clone(),
            kind: PackageKind::Addon,
            name: None,
            flavors: vec!["retail".into(), "vanilla".into()],
            ci_github: true,
        })
        .unwrap();
        let mut db = wowlua_span::SourceDb::new();
        let text = std::fs::read_to_string(path.join(MANIFEST_FILE)).unwrap();
        let out = wowlua_manifest::parse(
            &mut db,
            Path::new("wowlua.json"),
            text,
            &crate::compiler_version(),
        );
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        let manifest = out.manifest.unwrap();
        assert_eq!(manifest.addon_name(), "KillTracker");
        assert_eq!(manifest.flavors, vec!["standard", "vanilla"]);
        assert!(path.join("src/main.wowlua").exists());
        let ci = std::fs::read_to_string(path.join(".github/workflows/ci.yml")).unwrap();
        assert!(ci.contains("run: wowlua publish"), "{ci}");
        assert!(ci.contains("contents: write"), "{ci}");
    }
}
