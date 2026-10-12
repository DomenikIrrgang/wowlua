//! `wowlua new` (package templates, spec §12.9) and `wowlua init` (adopting
//! an existing addon, spec §12.6).

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
    steps:
      - uses: actions/checkout@v5
        with: { fetch-depth: 0 }
      - uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main
      - uses: actions/cache@v4
        with:
          path: ~/.wowlua/cache
          key: wowlua-${{ hashFiles('wowlua.lock') }}
      - run: wowlua build --release --locked --message-format github
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
            files.push(("tests/main.test.wowlua".into(), test_template()));
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
    /// For a folder `move-folders` makes a workspace member: the parts of
    /// the parent's `.pkgmeta` inside it, relative to it.
    pub inherited: Option<crate::pkgmeta::PkgMeta>,
    /// The AddOn name when it is known (a `move-folders` target), so that
    /// `LittleWigs_Classic.toc` is not read as `LittleWigs` for Classic.
    pub addon_name: Option<String>,
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
                && let Some(meta) = crate::pkgmeta::read(dir)?
            {
                return init_suite(dir, meta);
            }
            return Err(e);
        }
    };
    let mut pkgmeta = match &request.inherited {
        Some(m) => Some(m.clone()),
        None => crate::pkgmeta::read(dir)?,
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
    for (path, _, suffix) in &tocs {
        let text = std::fs::read_to_string(path).map_err(|e| {
            Diagnostic::plain_error(format!("cannot read `{}`: {e}", path.display()))
        })?;
        // A packager project's TOC as the packager packages it.
        let text = if pkgmeta.is_some() {
            crate::keywords::packaged_toc(&text)
        } else {
            text
        };
        let toc = wowlua_toc::parse(&text);
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
            } else if key.contains('[') {
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
        sequences.push(sequence);
    }
    let (order, conflict) = merge_orders(&sequences);
    if conflict {
        report.warnings.push(Diagnostic::plain_warning(
            "the flavors' TOCs load some files in contradicting orders; `build.load-order` follows the first flavor",
        ));
    }

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
    let skip = |p: &str| external_of(p).is_some();
    let mut to_move = closure(
        dir,
        queue,
        &mut seen,
        &mut report.warnings,
        &skip,
        &mut into_externals,
    );
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
    // Files other AddOns next to this one load (`..\<this>\file.lua`) move
    // too and are shipped as `build.shared-files` (unless loaded here anyway).
    let referenced = sibling_references(dir);
    let shared: Vec<String> = referenced
        .iter()
        .filter_map(|rel| existing_case(dir, rel))
        .filter(|actual| !seen.contains(actual))
        .collect();
    to_move.extend(closure(
        dir,
        shared.clone(),
        &mut seen,
        &mut report.warnings,
        &skip,
        &mut into_externals,
    ));
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
        let (path, warning) = resolve_external(dir, line);
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
            if values.len() == 1
                && let Some(v) = values.into_iter().next().filter(|v| !v.is_empty())
            {
                let extra = addon
                    .entry("toc-extra")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .expect("an object");
                extra.insert(key.into(), json!(v));
            }
        }
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
    if !pins.is_empty() {
        // Members' pins may already be there (`move-folders`).
        let (mut lock, merge) = match crate::lockfile::Lockfile::read(dir) {
            Ok(Some(l)) => (l, true),
            _ => (crate::lockfile::required(None, &all_flavors), false),
        };
        for (flavor, list) in pins {
            if !merge {
                lock.flavor_table
                    .interfaces
                    .insert(flavor.clone(), Vec::new());
            }
            let entry = lock.flavor_table.interfaces.entry(flavor).or_default();
            for i in list {
                if !entry.contains(&i) {
                    entry.push(i);
                }
            }
        }
        lock.write(dir).map_err(|e| {
            Diagnostic::plain_error(format!("cannot write `{}`: {e}", crate::lockfile::LOCKFILE))
        })?;
    }
    Ok(report)
}

/// Packager keywords and blocks in a passthrough file (spec CLI §12.6).
fn has_packager_keywords(text: &str) -> bool {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    PATTERN
        .get_or_init(|| {
            regex::Regex::new(
                r"@(project|file|build)-[a-z-]+@|@(end-)?(non-)?(debug|alpha|do-not-package|retail|version-[a-z]+)@",
            )
            .expect("a valid pattern")
        })
        .is_match(text)
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
    let libs_dir = crate::pkgmeta::libs_dir(externals);
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
        if external_files[i].is_empty() {
            report.warnings.push(Diagnostic::plain_warning(format!(
                "external `{}` is not loaded by the TOC or the addon's XML; not converted",
                e.dir
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
        if e.latest {
            report.warnings.push(Diagnostic::plain_warning(format!(
                "external `{}` uses `tag: latest`; the dependency follows the default branch instead (pin a `tag`)",
                e.dir
            )));
        }
        let install = match &libs_dir {
            Some(d) => e.dir[d.len() + 1..].to_string(),
            None => e.dir.clone(),
        };
        entry.insert("install-path".into(), json!(install));
        entry.insert("files".into(), json!(external_files[i]));
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
        if moved_lower.contains(&lower) {
            out.push(format!("{src_dir}/{p}"));
        } else if dir.join(&p).is_dir() {
            out.push(format!("{p}/**"));
            if moved_lower
                .iter()
                .any(|m| m.starts_with(&format!("{lower}/")))
            {
                out.push(format!("{src_dir}/{p}/**"));
            }
        } else {
            out.push(p);
        }
        out
    };
    let build = object
        .entry("build")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("an object");
    let exclude: Vec<String> = meta.ignore.iter().flat_map(|p| pattern_of(p)).collect();
    if !exclude.is_empty() {
        build.insert("exclude".into(), json!(exclude));
    }
    let plain: Vec<String> = meta
        .plain_copy
        .iter()
        .flat_map(|p| pattern_of(p))
        .filter(|p| !p.starts_with(&format!("{src_dir}/")))
        .collect();
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
    // Keyword substitution, when the sources use packager keywords.
    let uses_keywords = moved.iter().any(|m| {
        let lower = m.to_ascii_lowercase();
        (lower.ends_with(".lua") || lower.ends_with(".xml"))
            && std::fs::read(dir.join(src_dir).join(m))
                .is_ok_and(|b| has_packager_keywords(&String::from_utf8_lossy(&b)))
    });
    if uses_keywords {
        object.insert("compat".into(), json!({ "packager-keywords": true }));
    }
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
    if meta.enable_nolib || !meta.relations.is_empty() {
        let targets = publish
            .entry("targets")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("an object");
        if meta.enable_nolib {
            match crate::vcs::github_repository(dir) {
                Some(repository) => {
                    let name = if targets.contains_key("github") {
                        "github-nolib"
                    } else {
                        "github"
                    };
                    targets.insert(
                        name.into(),
                        json!({ "type": "github", "repository": repository, "nolib": true }),
                    );
                }
                None => report.warnings.push(Diagnostic::plain_warning(
                    "`enable-nolib-creation` becomes a GitHub target with `nolib`, but the repository has no GitHub `origin`; not converted",
                )),
            }
        }
        if !meta.relations.is_empty() {
            match targets.get_mut("curseforge").and_then(Value::as_object_mut) {
                Some(cf) => {
                    let list: Vec<Value> = meta
                        .relations
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
    if publish.is_empty() {
        object.shift_remove("publish");
    }
    let members = init_members(dir, meta, member_dirs, report);
    if !members.is_empty() {
        object.insert("workspace".into(), json!({ "members": members }));
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
    report: &mut InitReport,
) -> Vec<String> {
    let mut members = Vec::new();
    for (from, to) in &meta.move_folders {
        let Some((rel, _)) = member_dirs.iter().find(|(_, t)| t == to) else {
            if !is_flattening(from, to) {
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
            ..Default::default()
        };
        match init_package(&InitRequest {
            dir: dir.join(rel),
            from_toc: None,
            from_pkgmeta: false,
            inherited: Some(inherited),
            addon_name: Some(to.clone()),
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
    // One lockfile for the workspace: the members' interface pins move to
    // the root's.
    let mut pins: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for rel in &members {
        let member = dir.join(rel);
        if let Ok(Some(lock)) = crate::lockfile::Lockfile::read(&member) {
            for (flavor, list) in lock.flavor_table.interfaces {
                let entry = pins.entry(flavor).or_default();
                for i in list {
                    if !entry.contains(&i) {
                        entry.push(i);
                    }
                }
            }
            let _ = std::fs::remove_file(member.join(crate::lockfile::LOCKFILE));
        }
    }
    if !pins.is_empty() {
        let flavors: Vec<String> = pins.keys().cloned().collect();
        let mut lock = match crate::lockfile::Lockfile::read(dir) {
            Ok(Some(l)) => l,
            _ => crate::lockfile::required(None, &flavors),
        };
        for (flavor, list) in pins {
            let entry = lock.flavor_table.interfaces.entry(flavor).or_default();
            for i in list {
                if !entry.contains(&i) {
                    entry.push(i);
                }
            }
        }
        if let Err(e) = lock.write(dir) {
            report.warnings.push(Diagnostic::plain_warning(format!(
                "cannot write `{}`: {e}",
                crate::lockfile::LOCKFILE
            )));
        }
    }
    members
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
    let members = member_folders(dir, &meta, &package);
    if members.is_empty() {
        return Err(Diagnostic::plain_error(format!(
            "no .toc file found in `{}`, and no `move-folders` entry of `{}` moves an addon folder",
            dir.display(),
            meta.file
        )));
    }
    flatten_externals(&mut meta, &members, false);
    let mut report = InitReport {
        addon_name: package,
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
    let converted = init_members(dir, &meta, &members, &mut report);
    let mut manifest = Map::new();
    manifest.insert("$schema".into(), schema_url().into());
    manifest.insert("workspace".into(), json!({ "members": converted }));
    if let Some((file, _)) = &meta.manual_changelog {
        manifest.insert("publish".into(), json!({ "changelog": file }));
    }
    report.members = converted;
    std::fs::write(dir.join(MANIFEST_FILE), pretty(&Value::Object(manifest)))
        .map_err(|e| Diagnostic::plain_error(format!("cannot write `{MANIFEST_FILE}`: {e}")))?;
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
            let rel = from
                .strip_prefix(&prefix)
                .unwrap_or(from)
                .trim_matches('/')
                .to_string();
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
            && let Ok(refs) = wowlua_xml::references(&text)
        {
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
fn sibling_references(dir: &Path) -> Vec<String> {
    let (Some(parent), Some(folder)) = (dir.parent(), dir.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("../{}/", folder.to_string_lossy().to_ascii_lowercase());
    let strip = |path: &str| -> Option<String> {
        let lower = path.to_ascii_lowercase();
        lower
            .starts_with(&prefix)
            .then(|| path[prefix.len()..].to_string())
    };
    let mut found: Vec<String> = Vec::new();
    let Ok(siblings) = std::fs::read_dir(parent) else {
        return found;
    };
    let mut siblings: Vec<PathBuf> = siblings
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.as_path() != dir)
        .collect();
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
fn resolve_external(dir: &Path, line: &str) -> (String, Option<Diagnostic>) {
    let Some((owner, file)) = line.strip_prefix("../").and_then(|r| r.split_once('/')) else {
        return (line.to_string(), None);
    };
    let owner_dir = dir.parent().and_then(|parent| {
        std::fs::read_dir(parent)
            .ok()?
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(owner))
            .map(|e| e.path())
    });
    let Some(owner_dir) = owner_dir else {
        return (
            line.to_string(),
            Some(Diagnostic::plain_warning(format!(
                "the TOC loads `{line}`, but there is no AddOn `{owner}` next to this one; kept as written"
            ))),
        );
    };
    let owner_name = owner_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let adopted = std::fs::read_to_string(owner_dir.join(MANIFEST_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let owner_src = match &adopted {
        Some(manifest) => manifest
            .pointer("/build/src")
            .and_then(Value::as_str)
            .unwrap_or("src")
            .to_string(),
        None => source_dir_for(&owner_dir).to_string(),
    };
    if let Some(actual) = existing_case(&owner_dir.join(&owner_src), file) {
        return (format!("../{owner_name}/src/{actual}"), None);
    }
    match existing_case(&owner_dir, file) {
        Some(actual) if adopted.is_none() => (format!("../{owner_name}/src/{actual}"), None),
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
    !(structural.contains(&lower.as_str())
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
        .filter_map(|p| {
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
        })
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
        assert!(path.join(".github/workflows/ci.yml").exists());
    }
}
