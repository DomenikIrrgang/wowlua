//! Embedding `lua-library` dependencies (spec §10.5.2): each is placed in
//! `<libs-dir>/<install-path>/` in its original layout. Only what the game
//! can load is copied: the entry files, everything reachable from them
//! through XML `<Script>` / `<Include>`, and the license files.

use crate::Session;
use crate::project::Project;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::model::{Dependency, DependencyDetail, Manifest, PackageKind};
use wowlua_pkg::ResolvedPackage;

/// One embedded `lua-library`.
#[derive(Clone, Debug)]
pub struct LuaLib {
    pub name: String,
    /// `<libs-dir>/<install-path>` (output folder).
    pub install: String,
    pub dir: PathBuf,
    /// Only these flavors (`None`: all).
    pub flavors: Option<Vec<String>>,
    /// Entry files in load order (paths relative to `dir`, on-disk case),
    /// each with its flavor restriction.
    pub entries: Vec<(String, Option<Vec<String>>)>,
    /// Every file of the package by lowercase relative path → on-disk path.
    pub index: BTreeMap<String, String>,
    /// License files (relative to `dir`).
    pub licenses: Vec<String>,
    /// The `license` of its manifest.
    pub license_id: Option<String>,
    /// Where it comes from, and the locked commit or revision (packager
    /// keywords in its files).
    pub source: wowlua_pkg::Source,
    pub pin: Option<String>,
    /// Its repository's packager project file and its path in there.
    pub project_file: Option<(PathBuf, String)>,
    /// Its standalone addon's name (`addon-name`), for nolib packages.
    pub addon_name: Option<String>,
    /// With `moved-up`: the folder it was moved out of, whose other files
    /// ship next to it, and the moved folders.
    pub moved_from: Option<(PathBuf, String)>,
}

impl LuaLib {
    pub fn output(&self, rel: &str) -> String {
        format!("{}/{rel}", self.install)
    }
}

/// `<libs-dir>/<install-path>`; a `libs-dir` of `.` puts libraries at the
/// top of the AddOn folder.
fn join_libs(libs_dir: &str, install: &str) -> String {
    match libs_dir {
        "" | "." => install.to_string(),
        d => format!("{d}/{install}"),
    }
}

/// Every file of a package directory (version-control folders excluded).
fn index(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            !(e.file_type().is_dir()
                && wowlua_pkg::checksum::VCS_DIRS
                    .contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(dir) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        out.insert(rel.to_ascii_lowercase(), rel);
    }
    out
}

/// `.` and `..` resolved in a `/`-separated path; `None` if it leaves.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The optional dependencies the enabled features turn on.
fn enabled_optional(project: &Project) -> BTreeSet<String> {
    let features = &project.manifest.manifest.features;
    let mut enabled = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = features.get("default").cloned().unwrap_or_default();
    while let Some(f) = stack.pop() {
        if let Some(dep) = f.strip_prefix("dep:") {
            enabled.insert(dep.to_string());
            continue;
        }
        if seen.insert(f.clone())
            && let Some(more) = features.get(&f)
        {
            stack.extend(more.iter().cloned());
        }
    }
    enabled
}

fn resolve_flavors(list: &[String]) -> Vec<String> {
    let table = wowlua_flavors::table();
    list.iter()
        .filter_map(|f| table.resolve(f).map(|f| f.id.clone()))
        .collect()
}

/// The packages embedded into the project's build, in load order
/// (dependencies of a package before it, otherwise in the manifest's
/// declaration order), each with its flavor restriction (`None`: all).
/// Walks from the manifest's `dependencies` (never `dev-dependencies`),
/// with optional ones only when a default feature enables them.
pub fn embedded(project: &Project) -> Vec<(&ResolvedPackage, Option<BTreeSet<String>>)> {
    let manifest = &project.manifest.manifest;
    let root_name = project
        .manifest
        .package()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let packages: HashMap<&str, &ResolvedPackage> = project
        .deps
        .packages
        .iter()
        .filter(|p| p.used_by.contains(&root_name))
        .map(|p| (p.name.as_str(), p))
        .collect();
    let optional_on = enabled_optional(project);
    let mut order: Vec<String> = Vec::new();
    let mut restriction: HashMap<String, Option<BTreeSet<String>>> = HashMap::new();
    fn visit(
        name: &str,
        parent: Option<BTreeSet<String>>,
        own: Option<BTreeSet<String>>,
        packages: &HashMap<&str, &ResolvedPackage>,
        order: &mut Vec<String>,
        restriction: &mut HashMap<String, Option<BTreeSet<String>>>,
        stack: &mut Vec<String>,
    ) {
        let Some(p) = packages.get(name) else {
            return;
        };
        let here = match (parent, own) {
            (Some(a), Some(b)) => Some(a.intersection(&b).cloned().collect()),
            (a, b) => a.or(b),
        };
        let merged = match restriction.get(name) {
            None => here.clone(),
            Some(None) => None,
            Some(Some(old)) => here.as_ref().map(|h| old.union(h).cloned().collect()),
        };
        let changed = restriction.get(name) != Some(&merged);
        restriction.insert(name.to_string(), merged.clone());
        if stack.iter().any(|s| s == name) || (!changed && order.iter().any(|o| o == name)) {
            return;
        }
        stack.push(name.to_string());
        for dep in &p.dependencies {
            let own = p
                .manifest
                .as_ref()
                .and_then(|m| m.dependencies.get(dep))
                .and_then(|d| match d {
                    Dependency::Detailed(d) if !d.flavors.is_empty() => {
                        Some(resolve_flavors(&d.flavors).into_iter().collect())
                    }
                    _ => None,
                });
            visit(
                dep,
                merged.clone(),
                own,
                packages,
                order,
                restriction,
                stack,
            );
        }
        stack.pop();
        if !order.iter().any(|o| o == name) {
            order.push(name.to_string());
        }
    }
    let mut stack = Vec::new();
    for (name, dep) in &manifest.dependencies {
        if dep.is_optional() && !optional_on.contains(name) {
            continue;
        }
        let own = match dep {
            Dependency::Detailed(d) if !d.flavors.is_empty() => {
                Some(resolve_flavors(&d.flavors).into_iter().collect())
            }
            _ => None,
        };
        visit(
            name,
            None,
            own,
            &packages,
            &mut order,
            &mut restriction,
            &mut stack,
        );
    }
    order
        .into_iter()
        .map(|n| {
            let r = restriction.get(&n).cloned().flatten();
            (packages[n.as_str()], r)
        })
        .collect()
}

/// The entry of a dependency as the root declares it, else as the package
/// that brought it in declares it.
fn entry_of(project: &Project, p: &ResolvedPackage) -> DependencyDetail {
    match project.manifest.manifest.dependencies.get(&p.name) {
        Some(Dependency::Detailed(d)) => (**d).clone(),
        _ => p.entry.clone(),
    }
}

/// The embedded `lua-library` dependencies of the project for `flavors`,
/// in load order.
pub fn lua_libraries(
    session: &mut Session,
    project: &Project,
    flavors: &[String],
) -> Option<Vec<LuaLib>> {
    let manifest = &project.manifest.manifest;
    let libs_dir = manifest.build.libs_dir.trim_matches('/').to_string();
    let mut libs = Vec::new();
    for (p, restriction) in embedded(project) {
        if p.kind != PackageKind::LuaLibrary {
            continue;
        }
        let name = p.name.clone();
        let flavors_of: Option<Vec<String>> = restriction.map(|set| {
            flavors
                .iter()
                .filter(|f| set.contains(*f))
                .cloned()
                .collect()
        });
        if flavors_of.as_ref().is_some_and(Vec::is_empty) {
            continue;
        }
        // The root's own entry wins for `files` and `install-path`.
        let entry = entry_of(project, p);
        let install_path = entry
            .install_path
            .clone()
            .unwrap_or_else(|| name.clone())
            .trim_matches('/')
            .to_string();
        let idx = index(&p.dir);
        let active: Vec<String> = flavors_of.clone().unwrap_or_else(|| flavors.to_vec());
        // `"load": false`: shipped, nothing loaded.
        let entries = if entry.load == Some(false) {
            Vec::new()
        } else {
            match entry_files(session, &name, p, &entry.files, &idx, &active) {
                Some(e) => e,
                None => continue,
            }
        };
        let lib_section = p.manifest.as_ref().and_then(|m| m.lua_library.as_ref());
        let licenses: Vec<String> = match lib_section.and_then(|l| l.license_files.as_ref()) {
            Some(list) => list
                .iter()
                .filter_map(|f| idx.get(&f.to_ascii_lowercase()).cloned())
                .collect(),
            None => idx
                .values()
                .filter(|rel| !rel.contains('/'))
                .filter(|rel| {
                    let l = rel.to_ascii_lowercase();
                    l.starts_with("license") || l.starts_with("licence") || l.starts_with("copying")
                })
                .cloned()
                .collect(),
        };
        libs.push(LuaLib {
            name: name.clone(),
            install: join_libs(&libs_dir, &install_path),
            dir: p.dir.clone(),
            flavors: flavors_of,
            entries,
            index: idx,
            project_file: p.project_file.clone(),
            addon_name: entry.addon_name.clone(),
            moved_from: entry.moved_up.as_ref().map(|up| {
                let up = up.trim_matches('/').to_string();
                let mut outer = p.dir.clone();
                for _ in up.split('/') {
                    outer.pop();
                }
                (outer, up)
            }),
            licenses,
            license_id: p
                .manifest
                .as_ref()
                .and_then(|m| m.package.as_ref())
                .and_then(|pk| pk.license.clone()),
            source: p.source.clone(),
            pin: p.pin.clone(),
        });
    }
    Some(libs)
}

/// One embedded wowlua `library` (spec §10.5.1): its modules are compiled
/// into the addon as `@pkg/<name>/<path>`, emitted under `out`.
#[derive(Clone, Debug)]
pub struct WowluaLib {
    pub name: String,
    /// The package root.
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// `<libs-dir>/<install-path>` (output folder).
    pub out: String,
    /// The main module, relative to `src` (`init.wowlua`).
    pub main: Option<String>,
    /// Its source files by path relative to `src`.
    pub files: BTreeMap<String, PathBuf>,
    /// The declaration that embeds it (features, config).
    pub entry: DependencyDetail,
}

impl WowluaLib {
    /// `@pkg/<name>/<rel>`.
    pub fn src_rel(&self, rel: &str) -> String {
        format!("{}{}/{rel}", wowlua_sema::PACKAGE_PREFIX, self.name)
    }

    /// The output path of a source (`.wowlua` compiles to `.lua`).
    pub fn output(&self, rel: &str) -> String {
        let rel = match rel.strip_suffix(".wowlua") {
            Some(stem) => format!("{stem}.lua"),
            None => rel.to_string(),
        };
        format!("{}/{rel}", self.out)
    }
}

/// The embedded wowlua libraries, and types packages for checking.
/// Libraries that do not support a targeted flavor are E4327.
pub fn wowlua_libraries(
    session: &mut Session,
    project: &Project,
    flavors: &[String],
) -> Vec<WowluaLib> {
    let libs_dir = project
        .manifest
        .manifest
        .build
        .libs_dir
        .trim_matches('/')
        .to_string();
    let mut out = Vec::new();
    for (p, restriction) in embedded(project) {
        if p.kind != PackageKind::Library {
            continue;
        }
        let Some(manifest) = p.manifest.clone() else {
            continue;
        };
        let targeted: Vec<&String> = flavors
            .iter()
            .filter(|f| restriction.as_ref().is_none_or(|r| r.contains(*f)))
            .collect();
        if let Some(listed) = manifest.flavors.as_ref().filter(|f| !f.is_empty()) {
            let supported = resolve_flavors(&listed.keys().cloned().collect::<Vec<_>>());
            let missing: Vec<&str> = targeted
                .iter()
                .filter(|f| !supported.contains(f))
                .map(|f| f.as_str())
                .collect();
            if !missing.is_empty() {
                session.push(
                    Diagnostic::error(
                        "E4327",
                        format!(
                            "library `{}` does not support the flavor{} {}",
                            p.name,
                            if missing.len() == 1 { "" } else { "s" },
                            missing.join(", ")
                        ),
                    )
                    .with_path(&project.manifest.path)
                    .with_help(format!(
                        "restrict the dependency with `\"flavors\": [{}]`",
                        supported
                            .iter()
                            .map(|f| format!("\"{f}\""))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                );
                continue;
            }
        }
        let entry = entry_of(project, p);
        let src = p.dir.join(&manifest.build.src);
        let mut files = BTreeMap::new();
        for entry in walkdir::WalkDir::new(&src).into_iter().flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&src) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let lower = rel.to_ascii_lowercase();
            if lower.ends_with(".test.wowlua") {
                continue;
            }
            if [".wowlua", ".lua", ".xml"]
                .iter()
                .any(|e| lower.ends_with(e))
            {
                files.insert(rel, entry.path().to_path_buf());
            }
        }
        let main_path = manifest
            .package
            .as_ref()
            .and_then(|pk| pk.main.clone())
            .unwrap_or_else(|| format!("{}/init.wowlua", manifest.build.src));
        let main = p
            .dir
            .join(&main_path)
            .strip_prefix(&src)
            .ok()
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .filter(|r| files.contains_key(r));
        let install = entry
            .install_path
            .clone()
            .unwrap_or_else(|| p.name.clone())
            .trim_matches('/')
            .to_string();
        out.push(WowluaLib {
            name: p.name.clone(),
            dir: p.dir.clone(),
            manifest,
            out: join_libs(&libs_dir, &install),
            main,
            files,
            entry,
        });
    }
    out
}

/// Declaration files the checker reads from `types` packages (dependencies
/// and dev-dependencies) and from `types` paths of dependency entries.
pub fn type_declarations(project: &Project) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let root = project
        .manifest
        .package()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    for p in project.deps.of_root(&root) {
        if p.kind != PackageKind::Types {
            continue;
        }
        let globs = p
            .manifest
            .as_ref()
            .and_then(|m| m.types.as_ref())
            .map(|t| t.declarations.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| vec!["**/*.d.wowlua".into()]);
        let set = crate::sources::glob_set(&globs);
        for entry in walkdir::WalkDir::new(&p.dir).into_iter().flatten() {
            let Ok(rel) = entry.path().strip_prefix(&p.dir) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if entry.file_type().is_file() && rel.ends_with(".d.wowlua") && set.is_match(&rel) {
                out.push((
                    format!("{}{}/{rel}", wowlua_sema::PACKAGE_PREFIX, p.name),
                    entry.path().to_path_buf(),
                ));
            }
        }
    }
    let manifest = &project.manifest.manifest;
    for (name, dep) in manifest
        .dependencies
        .iter()
        .chain(&manifest.dev_dependencies)
    {
        let Dependency::Detailed(d) = dep else {
            continue;
        };
        let Some(types) = &d.types else { continue };
        if project.deps.packages.iter().any(|p| &p.name == types) {
            continue;
        }
        let path = project.root().join(types);
        let files: Vec<PathBuf> = if path.is_dir() {
            walkdir::WalkDir::new(&path)
                .into_iter()
                .flatten()
                .filter(|e| {
                    e.file_type().is_file() && e.path().to_string_lossy().ends_with(".d.wowlua")
                })
                .map(|e| e.path().to_path_buf())
                .collect()
        } else {
            vec![path.clone()]
        };
        for f in files {
            let rel = f
                .strip_prefix(project.root())
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| f.display().to_string());
            out.push((
                format!("{}{name}.types/{rel}", wowlua_sema::PACKAGE_PREFIX),
                f,
            ));
        }
    }
    out
}

/// The entry files (spec §10.5.2): the declaration's `files`, the library
/// manifest's `lua-library.files`, a single `.toc` (per flavor), or a single
/// `lib.xml` / `embeds.xml` / `<name>.xml`.
fn entry_files(
    session: &mut Session,
    name: &str,
    p: &ResolvedPackage,
    declared: &[String],
    idx: &BTreeMap<String, String>,
    flavors: &[String],
) -> Option<Vec<(String, Option<Vec<String>>)>> {
    let listed: Vec<String> = if !declared.is_empty() {
        declared.to_vec()
    } else {
        p.manifest
            .as_ref()
            .and_then(|m| m.lua_library.as_ref())
            .map(|l| l.files.clone())
            .unwrap_or_default()
    };
    if !listed.is_empty() {
        let mut out = Vec::new();
        for f in listed {
            let Some(rel) = normalize(&f.replace('\\', "/"))
                .and_then(|n| idx.get(&n.to_ascii_lowercase()).cloned())
            else {
                session.push(
                    Diagnostic::error("E4110", format!("lua-library `{name}` has no file `{f}`"))
                        .with_note(format!("looked in {}", p.dir.display())),
                );
                return None;
            };
            if rel.to_ascii_lowercase().ends_with(".toc") {
                out.extend(
                    toc_files(session, name, p, &rel, idx)?
                        .into_iter()
                        .map(|f| (f, None)),
                );
            } else {
                out.push((rel, None));
            }
        }
        return Some(out);
    }
    // A single `.toc` in the root, flavor-suffixed variants per flavor.
    let tocs: Vec<&String> = idx
        .values()
        .filter(|r| !r.contains('/') && r.to_ascii_lowercase().ends_with(".toc"))
        .collect();
    let bases: BTreeSet<String> = tocs
        .iter()
        .filter_map(|t| wowlua_toc::split_file_name(t).map(|(b, _)| b.to_ascii_lowercase()))
        .collect();
    if bases.len() == 1 {
        let table = wowlua_flavors::table();
        // Which TOC each flavor's client reads, and its files.
        let mut per_toc: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for flavor in flavors {
            let suffixes: Vec<String> = table
                .clients_of(flavor)
                .next()
                .map(|c| c.suffixes.clone())
                .unwrap_or_default();
            let chosen = suffixes
                .iter()
                .find_map(|s| {
                    tocs.iter().find(|t| {
                        wowlua_toc::split_file_name(t)
                            .and_then(|(_, suffix)| suffix)
                            .is_some_and(|x| x.eq_ignore_ascii_case(s))
                    })
                })
                .or_else(|| {
                    tocs.iter()
                        .find(|t| wowlua_toc::split_file_name(t).is_some_and(|(_, s)| s.is_none()))
                });
            if let Some(toc) = chosen {
                per_toc
                    .entry((*toc).clone())
                    .or_default()
                    .push(flavor.clone());
            }
        }
        if !per_toc.is_empty() {
            let mut lists: Vec<(Vec<String>, Vec<String>)> = Vec::new();
            for (toc, fl) in per_toc {
                lists.push((toc_files(session, name, p, &toc, idx)?, fl));
            }
            if lists.len() == 1 {
                return Some(lists.remove(0).0.into_iter().map(|f| (f, None)).collect());
            }
            // Files every TOC lists load for all; the rest per flavor, in
            // the first TOC's order with the others' additions after.
            let mut out: Vec<(String, Option<Vec<String>>)> = Vec::new();
            for (files, _) in &lists {
                for f in files {
                    let users: Vec<String> = lists
                        .iter()
                        .filter(|(fs, _)| fs.contains(f))
                        .flat_map(|(_, fl)| fl.clone())
                        .collect();
                    if out.iter().any(|(o, _)| o == f) {
                        continue;
                    }
                    let all = users.len() == flavors.len();
                    out.push((f.clone(), (!all).then_some(users)));
                }
            }
            return Some(out);
        }
    }
    for candidate in ["lib.xml", "embeds.xml", &format!("{name}.xml")] {
        if let Some(rel) = idx.get(&candidate.to_ascii_lowercase())
            && !rel.contains('/')
        {
            return Some(vec![(rel.clone(), None)]);
        }
    }
    session.push(
        Diagnostic::error(
            "E4110",
            format!("cannot determine the files to load for lua-library `{name}`"),
        )
        .with_note("it has no `files`, no single .toc and no lib.xml / embeds.xml")
        .with_help(format!(
            "list the entry files in the dependency: `\"files\": [\"{name}.lua\"]`"
        )),
    );
    None
}

/// The file lines of a library's `.toc`, resolved in the package.
fn toc_files(
    session: &mut Session,
    name: &str,
    p: &ResolvedPackage,
    toc: &str,
    idx: &BTreeMap<String, String>,
) -> Option<Vec<String>> {
    let text = std::fs::read(p.dir.join(toc))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let dir = toc.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut out = Vec::new();
    for file in wowlua_toc::parse(&text).files {
        let joined = if dir.is_empty() {
            file.path.replace('\\', "/")
        } else {
            format!("{dir}/{}", file.path.replace('\\', "/"))
        };
        match normalize(&joined).and_then(|n| idx.get(&n.to_ascii_lowercase()).cloned()) {
            Some(rel) => out.push(rel),
            None => {
                session.push(Diagnostic::error(
                    "E4110",
                    format!(
                        "`{toc}` of lua-library `{name}` lists `{}`, which does not exist",
                        file.path
                    ),
                ));
                return None;
            }
        }
    }
    Some(out)
}

/// The files to copy: `roots` and everything they reach through XML, plus
/// the license files. Unresolvable references are E5002.
pub fn reachable(session: &mut Session, lib: &LuaLib, roots: &[String]) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = roots.to_vec();
    while let Some(rel) = queue.pop() {
        if !seen.insert(rel.clone()) {
            continue;
        }
        if !rel.to_ascii_lowercase().ends_with(".xml") {
            continue;
        }
        let Ok(bytes) = std::fs::read(lib.dir.join(&rel)) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let refs = match wowlua_xml::references(&text) {
            Ok(r) => r,
            Err(e) => {
                session.push(Diagnostic::error(
                    "E5002",
                    format!(
                        "invalid XML in `{}` of lua-library `{}`: {}",
                        rel, lib.name, e.message
                    ),
                ));
                continue;
            }
        };
        let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for r in refs {
            let joined = if dir.is_empty() {
                r.normalized()
            } else {
                format!("{dir}/{}", r.normalized())
            };
            match normalize(&joined).and_then(|n| lib.index.get(&n.to_ascii_lowercase()).cloned()) {
                Some(target) => queue.push(target),
                None => session.push(Diagnostic::error(
                    "E5002",
                    format!(
                        "`{}` of lua-library `{}` references `{}`, which does not exist",
                        rel,
                        lib.name,
                        r.path()
                    ),
                )),
            }
        }
    }
    seen.extend(lib.licenses.iter().cloned());
    seen
}

/// The license text of a wowlua library: its license files, else its
/// manifest's `license`.
pub fn wowlua_license_text(lib: &WowluaLib) -> Option<String> {
    let mut parts = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&lib.dir) {
        let mut names: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                p.is_file()
                    && (n.starts_with("license")
                        || n.starts_with("licence")
                        || n.starts_with("copying"))
            })
            .collect();
        names.sort();
        for p in names {
            if let Ok(bytes) = std::fs::read(&p) {
                parts.push(String::from_utf8_lossy(&bytes).trim_end().to_string());
            }
        }
    }
    if parts.is_empty() {
        return lib
            .manifest
            .package
            .as_ref()
            .and_then(|p| p.license.clone())
            .map(|id| format!("License: {id}"));
    }
    Some(parts.join("\n\n"))
}

/// The license text of a library for `LICENSES.txt`: its license files, or
/// the license header of its first entry file. `None` (W4306) if neither.
pub fn license_text(lib: &LuaLib) -> Option<String> {
    let mut parts = Vec::new();
    for file in &lib.licenses {
        if let Ok(bytes) = std::fs::read(lib.dir.join(file)) {
            parts.push(String::from_utf8_lossy(&bytes).trim_end().to_string());
        }
    }
    if parts.is_empty() {
        for (entry, _) in &lib.entries {
            let lua = if entry.to_ascii_lowercase().ends_with(".lua") {
                Some(entry.clone())
            } else {
                None
            };
            let Some(lua) = lua else { continue };
            if let Ok(bytes) = std::fs::read(lib.dir.join(&lua))
                && let Some(header) = license_header(&String::from_utf8_lossy(&bytes))
            {
                parts.push(header);
                break;
            }
        }
    }
    if parts.is_empty() {
        return lib.license_id.as_ref().map(|id| format!("License: {id}"));
    }
    Some(parts.join("\n\n"))
}

/// The leading comment block of a Lua file, when it mentions a license,
/// copyright or SPDX identifier.
pub fn license_header(text: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        let trimmed = rest.trim_start();
        if let Some(body) = trimmed.strip_prefix("--[[") {
            let end = body.find("]]")?;
            lines.push(body[..end].trim().to_string());
            rest = &body[end + 2..];
        } else if let Some(body) = trimmed.strip_prefix("--") {
            let (line, after) = body.split_once('\n').unwrap_or((body, ""));
            lines.push(line.trim().to_string());
            rest = after;
        } else {
            break;
        }
    }
    let block = lines.join("\n").trim().to_string();
    let lower = block.to_ascii_lowercase();
    (lower.contains("license")
        || lower.contains("licence")
        || lower.contains("copyright")
        || lower.contains("public domain")
        || lower.contains("spdx-license-identifier"))
    .then_some(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_headers() {
        let text = "-- an addon\nlocal x";
        assert!(license_header(text).is_none());
        let text = "-- LibStub is hereby placed in the Public Domain\n-- Credits: Kaelten\nlocal x";
        assert!(license_header(text).is_some());
        let text = "--[[ Copyright (c) 2020 X\nMIT License ]]\nlocal x";
        assert_eq!(
            license_header(text).unwrap(),
            "Copyright (c) 2020 X\nMIT License"
        );
        let text = "-- SPDX-License-Identifier: MIT\nlocal x";
        assert!(license_header(text).is_some());
    }
}
