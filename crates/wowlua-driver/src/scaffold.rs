//! `wowlua new` (package templates, spec §12.9) and `wowlua init` (adopting
//! an existing addon, spec §12.6).

use crate::fsutil;
use crate::vcs;
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
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
     --\n\
     -- This compiler version (milestone M0) builds plain Lua: wowlua syntax such as\n\
     -- types, classes and imports becomes available with the wowlua compiler (M1+).\n\
     local addonName, ns = ...\n\
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
    let tocs = find_tocs(dir, request.from_toc.as_deref())?;
    let addon_name = tocs[0].1.clone();
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

    let mut order: Vec<String> = Vec::new();
    let mut file_flavors: IndexMap<String, BTreeSet<String>> = IndexMap::new();
    for (toc, flavors) in views.iter().filter(|(_, f)| !f.is_empty()) {
        for file in &toc.files {
            if file.path.contains('[') {
                report.warnings.push(Diagnostic::plain_warning(format!(
                    "TOC line `{}` uses a file variable, which `wowlua init` cannot convert; add the file to the manifest by hand",
                    file.path
                )));
                continue;
            }
            let applicable = apply_conditions(flavors, &file.conditions);
            if applicable.is_empty() {
                continue;
            }
            if !order.iter().any(|p| p.eq_ignore_ascii_case(&file.path)) {
                order.push(file.path.clone());
            }
            let key = order
                .iter()
                .find(|p| p.eq_ignore_ascii_case(&file.path))
                .cloned()
                .unwrap();
            file_flavors.entry(key).or_default().extend(applicable);
        }
    }

    // Files to move: TOC files and everything they include through XML.
    let mut to_move: Vec<String> = Vec::new();
    let mut queue: Vec<String> = order.clone();
    while let Some(rel) = queue.pop() {
        let Some(actual) = existing_case(dir, &rel) else {
            report.warnings.push(Diagnostic::plain_warning(format!(
                "the TOC lists `{rel}`, which does not exist"
            )));
            continue;
        };
        if to_move.contains(&actual) {
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
        to_move.push(actual);
    }
    to_move.sort();

    // Move into src/ (git mv inside a repository).
    let in_git = vcs::inside_work_tree(dir);
    for rel in &to_move {
        let dest = format!("src/{rel}");
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
        let path = dir.join("src").join(rel);
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
    let assets = asset_globs(dir);

    // Manifest. Flags and saved variables come from the most general TOC;
    // metadata that differs per flavor becomes `addon.flavor-overrides`.
    let primary_index = infos
        .iter()
        .enumerate()
        .min_by_key(|(i, (_, suffix, _))| (rank(suffix), *i))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let metadata = flavor_metadata(&views, &all_flavors);
    let mut warnings = Vec::new();
    let manifest = adopted_manifest(
        &infos[primary_index].0,
        &views,
        &metadata,
        &addon_name,
        &all_flavors,
        &order,
        &to_move,
        &file_flavors,
        assets,
        &mut warnings,
    );
    report.warnings.extend(warnings);

    // Lua/XML files no TOC loads stay where they are; say so.
    let left: Vec<String> = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| fsutil::relative_slash(e.path(), dir))
        .filter(|rel| !rel.starts_with("src/") && !rel.starts_with("dist/"))
        .filter(|rel| !crate::build::is_bindings_file(rel))
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
    Ok(report)
}

/// The TOC files of the addon in `dir`: `(path, addon name, suffix)`.
fn find_tocs(
    dir: &Path,
    explicit: Option<&Path>,
) -> Result<Vec<(PathBuf, String, Option<String>)>, Diagnostic> {
    let mut found: Vec<(PathBuf, String, Option<String>)> = std::fs::read_dir(dir)
        .map_err(|e| Diagnostic::plain_error(format!("cannot read `{}`: {e}", dir.display())))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let (base, suffix) = wowlua_toc::split_file_name(&file_name)?;
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
        None => {
            let folder = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            found
                .iter()
                .find(|(_, base, _)| *base == folder)
                .or_else(|| found.first())
                .map(|(_, base, _)| base.clone())
                .expect("found is not empty")
        }
    };
    let mut selected: Vec<_> = found
        .into_iter()
        .filter(|(_, base, _)| *base == wanted)
        .collect();
    // The plain TOC first, so it provides the metadata.
    selected.sort_by_key(|(_, _, suffix)| suffix.is_some());
    Ok(selected)
}

/// Flavors that load a file: the TOC's flavors filtered by the line's inline
/// `[AllowLoadGameType]` / `[ExcludeLoadGameType]` directives.
fn apply_conditions(
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
        let exact = current.join(part);
        let name = if exact.exists() {
            part.to_string()
        } else {
            std::fs::read_dir(&current)
                .ok()?
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.eq_ignore_ascii_case(part))?
        };
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
/// point at `…\<Name>\src\<path>`. Handles `\`, `\\` (escaped in Lua strings)
/// and `/` separators. Returns the new text and the number of rewrites.
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
fn asset_globs(dir: &Path) -> Vec<String> {
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
        if rel.starts_with("src/") || rel.starts_with("dist/") {
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
    ];
    !(structural.contains(&lower.as_str())
        || lower.starts_with("dep")
        || lower.starts_with("optionaldep"))
}

fn flavor_metadata(
    views: &[(wowlua_toc::ParsedToc, Vec<String>)],
    all_flavors: &[String],
) -> FlavorMetadata {
    // key -> value for each flavor, from the TOC its client reads.
    let mut per_flavor: IndexMap<String, IndexMap<String, String>> = IndexMap::new();
    let mut keys: Vec<String> = Vec::new();
    for (toc, flavors) in views {
        for flavor in flavors {
            let entry = per_flavor.entry(flavor.clone()).or_default();
            for d in toc.directives.iter().filter(|d| is_metadata(&d.key)) {
                let applies =
                    !apply_conditions(std::slice::from_ref(flavor), &d.conditions).is_empty();
                if applies && !entry.contains_key(&d.key) {
                    entry.insert(d.key.clone(), d.value.clone());
                    if !keys.contains(&d.key) {
                        keys.push(d.key.clone());
                    }
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
) -> Value {
    let package_name = kebab_case(addon_name);
    let version = toc
        .get("Version")
        .and_then(normalize_version)
        .unwrap_or_else(|| {
            warnings.push(Diagnostic::plain_warning(format!(
                "the TOC version `{}` is not a SemVer version; using 0.1.0",
                toc.get("Version").unwrap_or("")
            )));
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
    let flag = |name: &str| toc.get(name).is_some_and(|v| v.trim() == "1");
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
    let collect = |matches: &dyn Fn(&str) -> bool| -> IndexMap<String, BTreeSet<String>> {
        let mut entries: IndexMap<String, BTreeSet<String>> = IndexMap::new();
        for (parsed, toc_flavors) in all_tocs {
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
                }
            }
        }
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
    if !assets.is_empty() {
        build.insert("assets".into(), json!(assets));
    }
    let load_order: Vec<String> = order
        .iter()
        .filter_map(|p| moved.iter().find(|m| m.eq_ignore_ascii_case(p)))
        .map(|p| format!("src/{p}"))
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
            Some((format!("src/{actual}"), json!(ordered)))
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
