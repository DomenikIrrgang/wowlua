//! The sources of the workspace addons an addon lists in
//! `addon-dependencies` (spec §10.8): read and type-checked with the
//! addon's own, so that their `@global` declarations reach it with their
//! types, and never compiled into it or reported.

use crate::compile::{CheckConfig, Declaration, LibraryUnit};
use crate::project::Project;
use crate::sources::{Source, SourceKind};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wowlua_manifest::model::{DependencyDetail, Manifest, PackageKind};
use wowlua_sema::meta::Meta;
use wowlua_span::{FileId, SourceDb};
use wowlua_syntax::Parse;

/// A package of the types-only sources: a dependency addon, or one of the
/// wowlua libraries it embeds.
pub struct ForeignPackage {
    pub name: String,
    pub manifest: Manifest,
    /// Its main module, relative to its source folder.
    main: Option<String>,
    /// Its `.wowlua` and `.d.wowlua` files by path relative to its source
    /// folder.
    files: BTreeMap<String, PathBuf>,
}

impl ForeignPackage {
    fn src_rel(&self, rel: &str) -> String {
        format!("{}{}/{rel}", wowlua_sema::PACKAGE_PREFIX, self.name)
    }
}

/// The types-only sources of an addon's build.
#[derive(Default)]
pub struct Foreign {
    pub files: Vec<(Source, FileId)>,
    pub parses: Vec<Parse>,
    pub declarations: Vec<(Declaration, Parse)>,
    pub packages: Vec<ForeignPackage>,
}

impl Foreign {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.declarations.is_empty()
    }

    /// Their files, whose diagnostics are not the addon's.
    pub fn file_ids(&self) -> HashSet<FileId> {
        self.files
            .iter()
            .map(|(_, f)| *f)
            .chain(self.declarations.iter().map(|(d, _)| d.file))
            .collect()
    }

    /// How each package is checked, and its `wowlua:meta`, for one
    /// flavor's `meta`.
    pub fn units(&self, meta: &Meta) -> Vec<(String, LibraryUnit)> {
        self.packages
            .iter()
            .map(|p| {
                let m = &p.manifest;
                let mut globals: HashSet<String> = m.check.globals.iter().cloned().collect();
                if let Some(addon) = &m.addon {
                    globals.extend(addon.saved_variables.iter().cloned());
                    globals.extend(addon.saved_variables_per_character.iter().cloned());
                }
                let unit = LibraryUnit {
                    check: CheckConfig {
                        mode: m.check.mode,
                        strict: crate::sources::glob_set(&m.check.strict),
                        lua: m.check.lua,
                        secret_widget_results: m.check.secret_widget_results,
                        globals,
                        stdlib: wowlua_sema::stdlib::StdlibConfig::default(),
                        index: false,
                        focus: None,
                        hover: None,
                        complete: None,
                    },
                    main: p.main.as_ref().map(|r| p.src_rel(r)),
                    meta: Arc::new(crate::meta::library_meta(
                        meta,
                        m,
                        &DependencyDetail::default(),
                    )),
                };
                (p.name.clone(), unit)
            })
            .collect()
    }
}

/// The `.wowlua` and `.d.wowlua` files below `src`, tests left out.
fn files_below(src: &Path) -> BTreeMap<String, PathBuf> {
    let mut files = BTreeMap::new();
    for path in crate::host::walk(src, true, &|_| true) {
        let Ok(rel) = path.strip_prefix(src) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let lower = rel.to_ascii_lowercase();
        if lower.ends_with(".wowlua") && !lower.ends_with(".test.wowlua") {
            files.insert(rel, path);
        }
    }
    files
}

/// What to read next: a file of a package, or a standard-library module.
enum Wanted {
    File(usize, String),
    Std(String),
}

/// Reads the types-only sources of `project`'s build: the files of each
/// workspace addon it lists in `addon-dependencies` that make `@global`
/// declarations, and what they import, transitively (their libraries and
/// standard-library modules the build does not have yet). `present`: the
/// `src_rel` of every file the build has.
pub fn load(db: &mut SourceDb, project: &Project, present: &HashSet<String>) -> Foreign {
    let mut out = Foreign::default();
    let Some(deps) = &project.manifest.manifest.addon_dependencies else {
        return out;
    };
    // The packages: each dependency addon, then the libraries it embeds
    // that the build does not embed itself.
    let mut addons = Vec::new();
    for dep in deps.required.iter().chain(&deps.optional) {
        let Some(addon) = project
            .workspace_addons
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(dep.name()))
        else {
            continue;
        };
        if out.packages.iter().any(|p| p.name == addon.package) {
            continue;
        }
        addons.push(out.packages.len());
        out.packages.push(ForeignPackage {
            name: addon.package.clone(),
            manifest: addon.manifest.clone(),
            main: None,
            files: files_below(&addon.src),
        });
    }
    for &a in &addons.clone() {
        let root = out.packages[a].name.clone();
        for p in project.deps.of_root(&root) {
            let embedded = present
                .iter()
                .any(|s| s.starts_with(&format!("{}{}/", wowlua_sema::PACKAGE_PREFIX, p.name)));
            if p.kind != PackageKind::Library
                || embedded
                || out.packages.iter().any(|q| q.name == p.name)
            {
                continue;
            }
            let Some(manifest) = p.manifest.clone() else {
                continue;
            };
            let src = p.dir.join(&manifest.build.src);
            let main_path = manifest
                .package
                .as_ref()
                .and_then(|pk| pk.main.clone())
                .unwrap_or_else(|| format!("{}/init.wowlua", manifest.build.src));
            let files = files_below(&src);
            let main = p
                .dir
                .join(&main_path)
                .strip_prefix(&src)
                .ok()
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .filter(|r| files.contains_key(r));
            out.packages.push(ForeignPackage {
                name: p.name.clone(),
                manifest,
                main,
                files,
            });
        }
    }
    // The files that make `@global` declarations, then what they import.
    let mut wanted: Vec<Wanted> = Vec::new();
    for &a in &addons {
        for (rel, abs) in &out.packages[a].files {
            if rel.ends_with(".d.wowlua") {
                continue;
            }
            if crate::overlay::read(abs).is_ok_and(|b| b.windows(7).any(|w| w == b"@global")) {
                wanted.push(Wanted::File(a, rel.clone()));
            }
        }
    }
    let mut seen: HashSet<String> = present.clone();
    while let Some(next) = wanted.pop() {
        let (src_rel, rel, text, package) = match next {
            Wanted::File(p, rel) => {
                let src_rel = out.packages[p].src_rel(&rel);
                if !seen.insert(src_rel.clone()) {
                    continue;
                }
                let Ok(bytes) = crate::overlay::read(&out.packages[p].files[&rel]) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&bytes).into_owned();
                (src_rel.clone(), src_rel, text, Some(p))
            }
            Wanted::Std(name) => {
                let src_rel = wowlua_stdlib::src_rel(&name);
                if !seen.insert(src_rel.clone()) {
                    continue;
                }
                let Some(module) = wowlua_stdlib::module(&name) else {
                    continue;
                };
                if let Some(decl) = module.declaration {
                    let d_src_rel = format!("{}{name}.d.wowlua", wowlua_sema::STDLIB_PREFIX);
                    if seen.insert(d_src_rel.clone()) {
                        let rel = format!("<wowlua>/{name}.d.wowlua");
                        let file = db.add(&rel, decl.clone());
                        let parse = wowlua_syntax::parse(decl.as_bytes(), file);
                        out.declarations.push((
                            Declaration {
                                source: source(rel, d_src_rel, PathBuf::new()),
                                file,
                                bytes: decl.into_bytes().into(),
                            },
                            parse,
                        ));
                    }
                }
                (
                    src_rel,
                    format!("<wowlua>/{name}.wowlua"),
                    module.source,
                    None,
                )
            }
        };
        let abs = match package {
            Some(p) => {
                let pkg = &out.packages[p];
                let below = src_rel
                    .strip_prefix(&pkg.src_rel(""))
                    .unwrap_or(&src_rel)
                    .to_string();
                pkg.files.get(&below).cloned().unwrap_or_default()
            }
            None => PathBuf::new(),
        };
        let file = db.add(&rel, text.clone());
        let parse = wowlua_syntax::parse(text.as_bytes(), file);
        if src_rel.ends_with(".d.wowlua") {
            out.declarations.push((
                Declaration {
                    source: source(rel, src_rel, abs),
                    file,
                    bytes: text.into_bytes().into(),
                },
                parse,
            ));
            continue;
        }
        for s in parse.view().block().stmts() {
            let wowlua_syntax::view::StmtKind::Import(import) = s.kind() else {
                continue;
            };
            let spec = String::from_utf8_lossy(&import.from().value()).into_owned();
            if let Some(name) = spec.strip_prefix("wowlua:") {
                if name != "meta" {
                    wanted.push(Wanted::Std(name.to_string()));
                }
                continue;
            }
            let Some(p) = package else { continue };
            let (target, path) = if let Some(rest) = spec.strip_prefix('/') {
                (p, rest.to_string())
            } else if spec.starts_with("./") || spec.starts_with("../") {
                let dir = src_rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                let prefix = out.packages[p].src_rel("");
                let Some(rel) = crate::compile::normalize_rel(&format!("{dir}/{spec}"))
                    .and_then(|j| j.strip_prefix(&prefix).map(str::to_string))
                else {
                    continue;
                };
                (p, rel)
            } else {
                let (name, rest) = spec.split_once('/').unwrap_or((spec.as_str(), ""));
                let Some(q) = out.packages.iter().position(|q| q.name == name) else {
                    continue;
                };
                if rest.is_empty() {
                    if let Some(main) = &out.packages[q].main {
                        wanted.push(Wanted::File(q, main.clone()));
                    }
                    continue;
                }
                (q, rest.to_string())
            };
            let files = &out.packages[target].files;
            let candidates = if path.ends_with(".wowlua") {
                vec![path.clone()]
            } else {
                vec![
                    format!("{path}.wowlua"),
                    format!("{path}.d.wowlua"),
                    format!("{path}/init.wowlua"),
                ]
            };
            if let Some(found) = candidates.into_iter().find(|c| files.contains_key(c)) {
                wanted.push(Wanted::File(target, found));
            }
        }
        out.files.push((source(rel, src_rel, abs), file));
        out.parses.push(parse);
    }
    out
}

fn source(rel: String, src_rel: String, abs: PathBuf) -> Source {
    Source {
        rel,
        src_rel,
        abs,
        kind: SourceKind::Wowlua,
        output: None,
    }
}
