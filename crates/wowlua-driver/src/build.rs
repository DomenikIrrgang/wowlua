//! Building a project into a complete AddOn folder (spec chapter 14).
//!
//! Milestone M0 builds plain Lua projects: sources are syntax-checked and
//! passed through, XML references are rewritten, TOC files are generated, and
//! the assembled output is validated before it is written atomically.

use crate::project::Project;
use crate::sources::{self, Source, SourceKind};
use crate::{Session, fsutil, luacheck, vcs};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::BuiltinProfile;
use wowlua_manifest::model::TocLayout;
use wowlua_span::{FileId, Span};
use wowlua_toc::{Directive, FileEntry, FlavorTarget, Layout, TocError, TocFile, TocInput};

#[derive(Clone, Debug)]
pub struct BuildRequest {
    /// Profile name (`dev`, `release`, `test` or custom).
    pub profile: String,
    /// `--flavor`: restrict to these flavors (IDs or aliases).
    pub flavors: Option<Vec<String>>,
    /// `--out`: write the AddOn folder here instead of `<out>/<profile>/<Name>`.
    pub out_dir: Option<PathBuf>,
    /// `false` assembles and validates without writing (e.g. `wowlua toc`).
    pub write: bool,
}

impl BuildRequest {
    pub fn new(profile: impl Into<String>) -> Self {
        BuildRequest {
            profile: profile.into(),
            flavors: None,
            out_dir: None,
            write: true,
        }
    }
}

/// Content of one output file.
#[derive(Clone, Debug)]
pub enum Content {
    Bytes(Vec<u8>),
    Copy(PathBuf),
}

/// Result of a successful build.
#[derive(Debug)]
pub struct BuildOutput {
    pub addon_name: String,
    /// The AddOn folder (written when the request asked for it).
    pub dir: PathBuf,
    pub profile: String,
    pub flavors: Vec<String>,
    pub version: String,
    pub build_id: String,
    pub tocs: Vec<TocFile>,
    /// Output files by path relative to the AddOn folder.
    pub files: BTreeMap<String, Content>,
}

/// Output entries that are reserved at the top level of the AddOn (§14.8).
const RESERVED_DIRS: &[&str] = &["src", "wowlua", "libs"];
const RESERVED_FILES: &[&str] = &["Bindings.xml", "LICENSES.txt"];

/// Builds the project. Returns `None` if any error was reported.
pub fn build(
    session: &mut Session,
    project: &Project,
    request: &BuildRequest,
) -> Option<BuildOutput> {
    let manifest = &project.manifest;
    let root = manifest.root.clone();
    let addon_name = project.addon_name();
    let kind = manifest.package().map(|p| p.kind).unwrap_or_default();
    if kind != wowlua_manifest::model::PackageKind::Addon {
        session.push(
            Diagnostic::plain_error("only packages of kind `addon` can be built")
                .with_path(&manifest.path)
                .with_note("building libraries arrives with the package manager (milestone M6)"),
        );
        return None;
    }

    let profile = match manifest.profile(&request.profile) {
        Ok(profile) => profile,
        Err(_) => {
            session.push(
                Diagnostic::error("E4006", format!("unknown profile `{}`", request.profile))
                    .with_path(&manifest.path),
            );
            return None;
        }
    };
    if profile.bundle {
        session.push(Diagnostic::note(
            "bundling is not available yet (milestone M7); building unbundled",
        ));
    }
    if profile.minify != "none" {
        session.push(Diagnostic::note(format!(
            "minification level `{}` is not available yet (milestone M7); output is not minified",
            profile.minify
        )));
    }

    let flavors = select_flavors(session, project, request.flavors.as_deref())?;
    for flavor in &flavors {
        if project.interfaces.get(flavor).is_none_or(Vec::is_empty) {
            session.push(
                Diagnostic::error(
                    "E4318",
                    format!("flavor `{flavor}` has no interface number"),
                )
                .with_path(&manifest.path),
            );
        }
    }

    // Sources and assets.
    let build_cfg = &manifest.manifest.build;
    let exclude = sources::glob_set(&build_cfg.exclude);
    let sources = sources::discover_sources(&root, &build_cfg.src, &build_cfg.out, &exclude);
    let assets = sources::discover_assets(
        &root,
        &build_cfg.out,
        &sources::glob_set(&build_cfg.assets),
        &exclude,
    );
    // An addon without code is valid (voice packs, saved-variable stores).

    // Load and check every source.
    // Raw bytes are kept: plain Lua passes through byte for byte (Lua strings
    // may contain any bytes). The source database holds a lossy text copy for
    // diagnostics.
    let mut files: Vec<(Source, FileId)> = Vec::new();
    let mut raw: HashMap<String, Vec<u8>> = HashMap::new();
    for source in sources {
        match std::fs::read(&source.abs) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let file = session.db.add(&source.rel, text);
                raw.insert(source.rel.clone(), bytes);
                files.push((source, file));
            }
            Err(error) => session.push(
                Diagnostic::plain_error(format!("cannot read source file: {error}"))
                    .with_path(&source.rel),
            ),
        }
    }
    let checks: Vec<Diagnostic> = files
        .iter()
        .filter(|(s, _)| s.kind != SourceKind::Xml)
        .filter_map(|(s, file)| {
            luacheck::check(
                &session.db,
                *file,
                &raw[&s.rel],
                s.kind == SourceKind::Wowlua,
            )
        })
        .collect();
    session.diagnostics.extend(checks);

    // XML references.
    let xml = resolve_xml(session, &files);

    // Flavor restrictions (`build.flavor-files`) and the TOC file list.
    let flavor_globs: Vec<(globset::GlobMatcher, Vec<String>)> = build_cfg
        .flavor_files
        .iter()
        .filter_map(|(pattern, list)| {
            let matcher = globset::Glob::new(pattern).ok()?.compile_matcher();
            let ids = list
                .iter()
                .filter_map(|f| wowlua_flavors::table().resolve(f).map(|f| f.id.clone()))
                .collect();
            Some((matcher, ids))
        })
        .collect();
    let restriction = |rel: &str| -> Option<Vec<String>> {
        let matching: BTreeSet<&String> = flavor_globs
            .iter()
            .filter(|(m, _)| m.is_match(rel))
            .flat_map(|(_, ids)| ids)
            .collect();
        (!matching.is_empty()).then(|| matching.into_iter().cloned().collect())
    };

    // TOC lines with variables, and the files they expand to.
    let (variable_items, loaded_by_variable) = variable_items(
        session,
        &build_cfg.src,
        &build_cfg.variable_files,
        &flavors,
        &files,
    );

    // Sources another AddOn's TOC loads (`build.shared-files`): shipped, but
    // not listed in this AddOn's TOC.
    let shared: BTreeSet<String> = build_cfg.shared_files.iter().cloned().collect();
    for path in &shared {
        if !files.iter().any(|(s, _)| s.rel == *path) {
            session.push(
                Diagnostic::error(
                    "E5002",
                    format!("shared file `{path}` is not a source file of this package"),
                )
                .with_path(&manifest.path),
            );
        }
    }

    let mut toc_items: Vec<TocItem> = Vec::new();
    let mut emitted: Vec<&Source> = Vec::new();
    for (source, _) in &files {
        let flavors_of = restriction(&source.rel);
        let included = flavors_of
            .as_ref()
            .is_none_or(|list| list.iter().any(|f| flavors.contains(f)));
        if !included && !loaded_by_variable.contains(&source.rel) {
            continue;
        }
        emitted.push(source);
        if !xml.loaded_by_xml.contains(&source.rel)
            && !loaded_by_variable.contains(&source.rel)
            && !shared.contains(&source.rel)
        {
            toc_items.push(TocItem {
                rel: source.rel.clone(),
                path: source.output_path(),
                flavors: flavors_of,
                locales: None,
            });
        }
    }
    toc_items.extend(variable_items);
    // Files of other AddOns (`../<AddOn>/<path>`), listed verbatim. Their
    // owner ships them (`build.shared-files`), so they are not checked here.
    for entry in &build_cfg.external_files {
        let flavors_of = entry.flavors.as_ref().map(|list| {
            list.iter()
                .filter_map(|f| wowlua_flavors::table().resolve(f).map(|f| f.id.clone()))
                .collect::<Vec<_>>()
        });
        if flavors_of
            .as_ref()
            .is_some_and(|list| !list.iter().any(|f| flavors.contains(f)))
        {
            continue;
        }
        toc_items.push(TocItem {
            rel: entry.path.clone(),
            path: entry.path.clone(),
            flavors: flavors_of,
            locales: None,
        });
    }
    order_by_load_order(&mut toc_items, &build_cfg.load_order);
    let toc_data = read_toc_data(session, project, &flavors);

    if session.has_errors() {
        return None;
    }

    // Version and build id.
    let vcs = vcs::info(&root);
    let version = build_version(session, project, &profile, vcs.as_ref())?;
    let build_id = build_id(session, project, &profile.name, &flavors, &files, &assets);

    // TOC files.
    let toc_input = TocInput {
        addon_name: addon_name.clone(),
        flavors: flavors
            .iter()
            .map(|id| FlavorTarget {
                id: id.clone(),
                interfaces: project.interfaces.get(id).cloned().unwrap_or_default(),
            })
            .collect(),
        directives: directives(
            project,
            &flavors,
            &profile.name,
            &version,
            &build_id,
            toc_data,
        ),
        files: toc_items
            .iter()
            .map(|item| FileEntry {
                path: item.path.clone(),
                flavors: item.flavors.clone(),
                locales: item.locales.clone(),
            })
            .collect(),
        layout: match build_cfg.toc_layout {
            TocLayout::Auto => Layout::Auto,
            TocLayout::Single => Layout::Single,
            TocLayout::PerFlavor => Layout::PerFlavor,
        },
    };
    let tocs = match wowlua_toc::generate(&toc_input) {
        Ok(generated) => generated.files,
        Err(errors) => {
            for error in errors {
                session.push(toc_error(error, &manifest.path));
            }
            return None;
        }
    };

    // Assemble the output tree.
    let mut out: BTreeMap<String, Content> = BTreeMap::new();
    for toc in &tocs {
        out.insert(
            toc.name.clone(),
            Content::Bytes(toc.content.clone().into_bytes()),
        );
    }
    for source in &emitted {
        let original = &raw[&source.rel];
        let content = match xml.rewrites.get(&source.rel) {
            Some(rewritten) if std::str::from_utf8(original).is_ok() => {
                rewritten.clone().into_bytes()
            }
            Some(_) => {
                session.push(
                    Diagnostic::error(
                        "E5002",
                        "cannot rewrite `.wowlua` references in an XML file that is not valid UTF-8",
                    )
                    .with_path(&source.rel),
                );
                continue;
            }
            None => original.clone(),
        };
        out.insert(source.output_path(), Content::Bytes(content));
    }
    out.insert(
        "LICENSES.txt".into(),
        Content::Bytes(licenses_text().into_bytes()),
    );
    for (name, bytes) in bindings_files(session, &root) {
        out.insert(name, Content::Bytes(bytes));
    }
    for asset in &assets {
        let first = asset.rel.split('/').next().unwrap_or_default();
        let reserved = RESERVED_DIRS.contains(&first) && asset.rel.contains('/')
            || RESERVED_FILES.contains(&asset.rel.as_str())
            || (!asset.rel.contains('/') && asset.rel.to_ascii_lowercase().ends_with(".toc"));
        if reserved || out.contains_key(&asset.rel) {
            session.push(
                Diagnostic::error(
                    "E4311",
                    format!(
                        "asset `{}` collides with a generated output path",
                        asset.rel
                    ),
                )
                .with_path(&asset.rel)
                .with_help("move the asset, or exclude it from `build.assets`"),
            );
            continue;
        }
        out.insert(asset.rel.clone(), Content::Copy(asset.abs.clone()));
    }

    let output_paths: HashMap<String, String> = emitted
        .iter()
        .map(|s| (s.rel.clone(), s.output_path()))
        .collect();
    let variable_outputs: BTreeSet<String> = emitted
        .iter()
        .filter(|s| loaded_by_variable.contains(&s.rel))
        .map(|s| s.output_path())
        .collect();
    let roots: BTreeSet<String> = emitted
        .iter()
        .filter(|s| shared.contains(&s.rel))
        .map(|s| s.output_path())
        .collect();
    validate_output(
        session,
        &out,
        &tocs,
        &xml,
        &output_paths,
        &variable_outputs,
        &roots,
    );
    if session.has_errors() {
        return None;
    }

    let dir = request
        .out_dir
        .clone()
        .unwrap_or_else(|| manifest.out_dir().join(&profile.name).join(&addon_name));
    if request.write
        && let Err(error) = write_tree(&dir, &out)
    {
        session.push(
            Diagnostic::plain_error(format!("cannot write the build output: {error}"))
                .with_path(&dir),
        );
        return None;
    }

    Some(BuildOutput {
        addon_name,
        dir,
        profile: profile.name,
        flavors,
        version,
        build_id,
        tocs,
        files: out,
    })
}

fn select_flavors(
    session: &mut Session,
    project: &Project,
    requested: Option<&[String]>,
) -> Option<Vec<String>> {
    let Some(requested) = requested else {
        return Some(project.manifest.flavors.clone());
    };
    let mut selected = Vec::new();
    for name in requested {
        match wowlua_flavors::table().resolve(name) {
            None => session.push(Diagnostic::error(
                "E4004",
                format!("unknown flavor `{name}`"),
            )),
            Some(f) if !project.manifest.flavors.contains(&f.id) => session.push(
                Diagnostic::plain_error(format!(
                    "flavor `{}` is not listed in `flavors` of the manifest",
                    f.id
                ))
                .with_path(&project.manifest.path),
            ),
            Some(f) => {
                if !selected.contains(&f.id) {
                    selected.push(f.id.clone());
                }
            }
        }
    }
    (!session.has_errors()).then_some(selected)
}

/// XML reference analysis: which files are loaded by XML, and rewritten XML.
#[derive(Debug, Default)]
pub(crate) struct XmlInfo {
    /// Sources (root-relative) loaded through `<Script>`/`<Include>`.
    loaded_by_xml: BTreeSet<String>,
    /// XML sources whose references were rewritten.
    rewrites: HashMap<String, String>,
    /// Reference graph: XML source → referenced sources.
    edges: BTreeMap<String, Vec<String>>,
}

fn resolve_xml(session: &mut Session, files: &[(Source, FileId)]) -> XmlInfo {
    let by_src_rel: HashMap<String, &Source> = files
        .iter()
        .map(|(s, _)| (s.src_rel.to_ascii_lowercase(), s))
        .collect();
    let mut info = XmlInfo::default();
    let mut diagnostics = Vec::new();
    for (source, file) in files.iter().filter(|(s, _)| s.kind == SourceKind::Xml) {
        let text = session.db.get(*file).text();
        let refs = match wowlua_xml::references(text) {
            Ok(refs) => refs,
            Err(error) => {
                let end = (error.offset + 1).min(text.len());
                diagnostics.push(
                    Diagnostic::error("E5002", format!("invalid XML: {}", error.message))
                        .with_label(Span::new(*file, error.offset.min(end)..end), "here"),
                );
                continue;
            }
        };
        let dir = source
            .src_rel
            .rsplit_once('/')
            .map(|(d, _)| d)
            .unwrap_or("");
        let mut replacements = Vec::new();
        for reference in &refs {
            let joined = normalize(&if dir.is_empty() {
                reference.normalized()
            } else {
                format!("{dir}/{}", reference.normalized())
            });
            // A reference may also name the compiled `.lua` of a `.wowlua` source.
            let direct = by_src_rel.get(&joined.to_ascii_lowercase()).copied();
            let via_lua = direct.is_none();
            let target = direct.or_else(|| {
                joined
                    .strip_suffix(".lua")
                    .and_then(|stem| by_src_rel.get(&format!("{stem}.wowlua").to_ascii_lowercase()))
                    .copied()
            });
            match target {
                Some(target) => {
                    let expected = if via_lua {
                        format!(
                            "{}.lua",
                            target
                                .src_rel
                                .strip_suffix(".wowlua")
                                .unwrap_or(&target.src_rel)
                        )
                    } else {
                        target.src_rel.clone()
                    };
                    // The client resolves references case-insensitively (W4310).
                    if expected != joined {
                        diagnostics.push(
                            Diagnostic::warning(
                                "W4310",
                                format!(
                                    "XML reference `{}` differs in case from the file `{}`",
                                    reference.file, target.src_rel
                                ),
                            )
                            .with_label(Span::new(*file, reference.span.clone()), "case mismatch"),
                        );
                    }
                    if target.kind == SourceKind::Wowlua && !via_lua {
                        // Keep inline directives (`x.wowlua [AllowLoadGameType tbc]`).
                        let path = reference.path();
                        let lua = format!("{}.lua", &path[..path.len() - ".wowlua".len()]);
                        replacements.push((reference, reference.file.replacen(&path, &lua, 1)));
                    }
                    info.loaded_by_xml.insert(target.rel.clone());
                    info.edges
                        .entry(source.rel.clone())
                        .or_default()
                        .push(target.rel.clone());
                }
                None => diagnostics.push(
                    Diagnostic::error(
                        "E5002",
                        format!(
                            "XML reference `{}` does not resolve to a source file",
                            reference.path()
                        ),
                    )
                    .with_label(Span::new(*file, reference.span.clone()), "not found"),
                ),
            }
        }
        if !replacements.is_empty() {
            info.rewrites
                .insert(source.rel.clone(), wowlua_xml::rewrite(text, &replacements));
        }
    }
    session.diagnostics.extend(diagnostics);
    info
}

/// Resolves `.` and `..` in a `/`-separated relative path.
fn normalize(path: &str) -> String {
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

/// One TOC file line: a source file or a variable line.
#[derive(Debug, Clone)]
struct TocItem {
    /// Path relative to the package root (may contain TOC variables).
    rel: String,
    /// Path inside the AddOn folder.
    path: String,
    flavors: Option<Vec<String>>,
    locales: Option<Vec<String>>,
}

/// Orders TOC entries: `build.load-order` entries first, in their order (an
/// entry naming an item exactly matches it, even when it contains `[…]`;
/// other entries are globs expanding in path order), then everything else by
/// path (spec §5.4; the load-order analysis of milestone M2 refines this).
fn order_by_load_order(entries: &mut Vec<TocItem>, load_order: &[String]) {
    let mut ordered = Vec::with_capacity(entries.len());
    let mut remaining: Vec<TocItem> = std::mem::take(entries);
    for pattern in load_order {
        let (matched, rest): (Vec<_>, Vec<_>) = if remaining.iter().any(|i| i.rel == *pattern) {
            remaining.into_iter().partition(|i| i.rel == *pattern)
        } else {
            let Ok(glob) = globset::Glob::new(pattern) else {
                continue;
            };
            let matcher = glob.compile_matcher();
            remaining
                .into_iter()
                .partition(|i| matcher.is_match(&i.rel))
        };
        ordered.extend(matched);
        remaining = rest;
    }
    remaining.sort_by(|a, b| a.rel.cmp(&b.rel));
    ordered.extend(remaining);
    *entries = ordered;
}

/// `[Game]` values per flavor.
fn game_name(flavor: &str) -> &'static str {
    match flavor {
        "standard" => "Standard",
        "camelot" => "Camelot",
        "mists" => "Mists",
        "cata" => "Cata",
        "wrath" => "Wrath",
        "tbc" => "TBC",
        _ => "Vanilla",
    }
}

fn family_name(flavor: &str) -> &'static str {
    match wowlua_flavors::table().flavor(flavor).map(|f| f.family) {
        Some(wowlua_flavors::Family::Mainline) => "Mainline",
        _ => "Classic",
    }
}

/// Replaces TOC variables in `path` (case-insensitive).
pub(crate) fn substitute(path: &str, flavor: &str, locale: &str) -> String {
    let mut out = path.to_string();
    for (var, value) in [
        ("[game]", game_name(flavor)),
        ("[family]", family_name(flavor)),
        ("[textlocale]", locale),
    ] {
        while let Some(i) = out.to_ascii_lowercase().find(var) {
            out.replace_range(i..i + var.len(), value);
        }
    }
    out
}

/// TOC items for `build.variable-files`, and the source files they load.
/// Every expansion for the targeted flavors (and listed locales) must exist.
fn variable_items(
    session: &mut Session,
    src: &str,
    entries: &[wowlua_manifest::model::VariableFile],
    flavors: &[String],
    files: &[(Source, FileId)],
) -> (Vec<TocItem>, BTreeSet<String>) {
    let by_lower: HashMap<String, &Source> = files
        .iter()
        .map(|(s, _)| (s.rel.to_ascii_lowercase(), s))
        .collect();
    let table = wowlua_flavors::table();
    let mut items = Vec::new();
    let mut loaded = BTreeSet::new();
    for entry in entries {
        let entry_flavors: Vec<String> = match &entry.flavors {
            Some(list) => list
                .iter()
                .filter_map(|f| table.resolve(f).map(|f| f.id.clone()))
                .filter(|f| flavors.contains(f))
                .collect(),
            None => flavors.to_vec(),
        };
        if entry_flavors.is_empty() {
            continue;
        }
        let has_locale = entry.path.to_ascii_lowercase().contains("[textlocale]");
        let locales: Vec<String> = match (&entry.locales, has_locale) {
            (Some(list), _) => list.clone(),
            (None, true) => wowlua_manifest::model::LOCALES
                .iter()
                .map(|l| l.to_string())
                .collect(),
            (None, false) => vec![String::new()],
        };
        let explicit_locales = entry.locales.is_some() || !has_locale;
        for flavor in &entry_flavors {
            for locale in &locales {
                let concrete = substitute(&entry.path, flavor, locale);
                match by_lower.get(&concrete.to_ascii_lowercase()) {
                    Some(source) => {
                        loaded.insert(source.rel.clone());
                    }
                    None if explicit_locales => session.push(
                        Diagnostic::error(
                            "E5002",
                            format!(
                                "`{}` expands to `{concrete}`, which does not exist",
                                entry.path
                            ),
                        )
                        .with_help(
                            "add the file, or restrict `flavors` / `locales` of this variable file",
                        ),
                    ),
                    None => {}
                }
            }
        }
        let restricted = (entry_flavors.len() < flavors.len()).then(|| entry_flavors.clone());
        let line_locales = entry.locales.clone();
        let supported = entry_flavors
            .iter()
            .flat_map(|f| table.clients_of(f))
            .all(|c| c.features.file_variables);
        let output = |path: &str| {
            let relative = path.strip_prefix(&format!("{src}/")).unwrap_or(path);
            format!("src/{relative}")
        };
        if supported {
            items.push(TocItem {
                rel: entry.path.clone(),
                path: output(&entry.path),
                flavors: restricted,
                locales: line_locales,
            });
        } else {
            // Expand per flavor (and per locale through [AllowLoadTextLocale]).
            for flavor in &entry_flavors {
                for locale in &locales {
                    let concrete = substitute(&entry.path, flavor, locale);
                    if let Some(source) = by_lower.get(&concrete.to_ascii_lowercase()) {
                        items.push(TocItem {
                            rel: entry.path.clone(),
                            path: source.output_path(),
                            flavors: Some(vec![flavor.clone()]),
                            locales: has_locale.then(|| vec![locale.clone()]),
                        });
                    }
                }
            }
        }
    }
    (items, loaded)
}

/// Per-flavor `## Key: Value` lines from `addon.toc-data` files.
type TocData = Vec<(String, Vec<(String, String)>)>;

fn read_toc_data(session: &mut Session, project: &Project, flavors: &[String]) -> TocData {
    let Some(addon) = &project.manifest.manifest.addon else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, paths) in &addon.toc_data {
        let Some(flavor) = wowlua_flavors::table().resolve(name).map(|f| f.id.clone()) else {
            continue;
        };
        if !flavors.contains(&flavor) {
            continue;
        }
        let mut lines = Vec::new();
        for rel in paths {
            let path = project.root().join(rel);
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => {
                    session.push(
                        Diagnostic::plain_error(format!("cannot read TOC data file: {error}"))
                            .with_path(rel),
                    );
                    continue;
                }
            };
            for (number, line) in text.lines().enumerate() {
                let line = line.trim_end();
                if line.trim().is_empty() || (line.starts_with('#') && !line.starts_with("##")) {
                    continue;
                }
                let parsed = line
                    .strip_prefix("##")
                    .and_then(|rest| rest.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()));
                match parsed {
                    Some((key, value)) if !wowlua_manifest::is_generated_directive(&key) => lines.push((key, value)),
                    Some((key, _)) => session.push(
                        Diagnostic::error("E4002", format!("TOC directive `{key}` is generated by wowlua and cannot come from TOC data"))
                            .with_path(format!("{rel}:{}", number + 1)),
                    ),
                    None => session.push(
                        Diagnostic::error("E4002", "TOC data files may only contain `## Key: Value` lines")
                            .with_path(format!("{rel}:{}", number + 1)),
                    ),
                }
            }
        }
        out.push((flavor, lines));
    }
    out
}

/// `build.version` (spec §9.9.2).
fn build_version(
    session: &mut Session,
    project: &Project,
    profile: &wowlua_manifest::Profile,
    vcs: Option<&vcs::VcsInfo>,
) -> Option<String> {
    let version = project
        .manifest
        .package()
        .map(|p| p.version.clone())
        .unwrap_or_else(|| "0.0.0".into());
    let hash = vcs
        .map(|v| format!("+{}", v.short_hash))
        .unwrap_or_default();
    if profile.base != BuiltinProfile::Release {
        return Some(format!("{version}-dev{hash}"));
    }
    let Some(vcs) = vcs else {
        return Some(format!("{version}-alpha"));
    };
    if vcs.tags.is_empty() || vcs.dirty {
        return Some(format!("{version}-alpha{hash}"));
    }
    let matching = vcs
        .tags
        .iter()
        .any(|tag| tag.strip_prefix('v').unwrap_or(tag) == version);
    if !matching {
        session.push(
            Diagnostic::error(
                "E4319",
                format!(
                    "the commit is tagged `{}`, which does not match the package version `{version}`",
                    vcs.tags.join("`, `")
                ),
            )
            .with_path(&project.manifest.path)
            .with_help("update `package.version` or the tag"),
        );
        return None;
    }
    Some(version)
}

/// Stable build id: a hash of the compiler version, profile, flavors,
/// manifest and every input file.
fn build_id(
    session: &Session,
    project: &Project,
    profile: &str,
    flavors: &[String],
    files: &[(Source, FileId)],
    assets: &[sources::Asset],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(crate::COMPILER_VERSION.as_bytes());
    hasher.update(profile.as_bytes());
    for flavor in flavors {
        hasher.update(flavor.as_bytes());
    }
    hasher.update(session.db.get(project.manifest.file).text().as_bytes());
    hasher.update(project.lock.to_json().as_bytes());
    for (source, file) in files {
        hasher.update(source.rel.as_bytes());
        hasher.update(session.db.get(*file).text().as_bytes());
    }
    for asset in assets {
        hasher.update(asset.rel.as_bytes());
        if let Ok(bytes) = std::fs::read(&asset.abs) {
            hasher.update(&bytes);
        }
    }
    hasher.finalize().to_hex()[..8].to_string()
}

/// TOC metadata directives in the order of spec §14.9.4.
/// Emits one metadata field, honoring `addon.flavor-overrides`: flavors that
/// share a value share one directive, restricted to those flavors when not all
/// targeted flavors use it. A flavor whose override is `null` gets none.
fn push_field(
    out: &mut Vec<Directive>,
    key: &str,
    base: Option<String>,
    overrides: &[(String, Option<Option<String>>)],
    flavors: &[String],
) {
    let mut groups: indexmap::IndexMap<Option<String>, Vec<String>> = indexmap::IndexMap::new();
    for flavor in flavors {
        let value = overrides
            .iter()
            .find(|(f, _)| f == flavor)
            .map(|(_, v)| v.clone().flatten())
            .unwrap_or_else(|| base.clone());
        groups.entry(value).or_default().push(flavor.clone());
    }
    for (value, group) in groups {
        let Some(value) = value else { continue };
        out.push(Directive {
            key: key.to_string(),
            value,
            flavors: (group.len() < flavors.len()).then_some(group),
        });
    }
}

fn directives(
    project: &Project,
    flavors: &[String],
    profile: &str,
    version: &str,
    build_id: &str,
    toc_data: TocData,
) -> Vec<Directive> {
    let manifest = &project.manifest;
    let addon = manifest.manifest.addon.clone().unwrap_or_default();
    // Overrides keyed by normalized flavor ID.
    let overrides: Vec<(String, wowlua_manifest::model::AddonOverride)> = addon
        .flavor_overrides
        .iter()
        .filter_map(|(name, o)| {
            wowlua_flavors::table()
                .resolve(name)
                .map(|f| (f.id.clone(), o.clone()))
        })
        .collect();
    let scalar = |get: fn(&wowlua_manifest::model::AddonOverride) -> &Option<Option<String>>| {
        overrides
            .iter()
            .filter_map(|(f, o)| get(o).clone().map(|v| (f.clone(), Some(v))))
            .collect::<Vec<_>>()
    };
    let localized = |out: &mut Vec<Directive>,
                     prefix: &str,
                     base: &indexmap::IndexMap<String, String>,
                     get: fn(
        &wowlua_manifest::model::AddonOverride,
    ) -> &indexmap::IndexMap<String, Option<String>>| {
        let mut locales: Vec<String> = base.keys().cloned().collect();
        for (_, o) in &overrides {
            for locale in get(o).keys() {
                if !locales.contains(locale) {
                    locales.push(locale.clone());
                }
            }
        }
        for locale in locales {
            let per_flavor: Vec<(String, Option<Option<String>>)> = overrides
                .iter()
                .filter_map(|(f, o)| get(o).get(&locale).map(|v| (f.clone(), Some(v.clone()))))
                .collect();
            push_field(
                out,
                &format!("{prefix}-{locale}"),
                base.get(&locale).cloned(),
                &per_flavor,
                flavors,
            );
        }
    };

    let mut out = Vec::new();
    push_field(
        &mut out,
        "Title",
        Some(manifest.title()),
        &scalar(|o| &o.title),
        flavors,
    );
    localized(&mut out, "Title", &addon.title_localized, |o| {
        &o.title_localized
    });
    push_field(
        &mut out,
        "Notes",
        addon.notes.clone(),
        &scalar(|o| &o.notes),
        flavors,
    );
    localized(&mut out, "Notes", &addon.notes_localized, |o| {
        &o.notes_localized
    });
    push_field(
        &mut out,
        "Author",
        manifest.author(),
        &scalar(|o| &o.author),
        flavors,
    );
    out.push(Directive::new("Version", version));
    push_field(
        &mut out,
        "IconTexture",
        addon.icon_texture.clone(),
        &scalar(|o| &o.icon_texture),
        flavors,
    );
    push_field(
        &mut out,
        "IconAtlas",
        addon.icon_atlas.clone(),
        &scalar(|o| &o.icon_atlas),
        flavors,
    );
    push_field(
        &mut out,
        "Category",
        addon.category.clone(),
        &scalar(|o| &o.category),
        flavors,
    );
    localized(&mut out, "Category", &addon.category_localized, |o| {
        &o.category_localized
    });
    push_field(
        &mut out,
        "Group",
        addon.group.clone(),
        &scalar(|o| &o.group),
        flavors,
    );
    if addon.load_on_demand {
        out.push(Directive::new("LoadOnDemand", "1"));
    }
    if addon.default_state == wowlua_manifest::model::DefaultState::Disabled {
        out.push(Directive::new("DefaultState", "disabled"));
    }
    if addon.only_beta_and_ptr {
        out.push(Directive::new("OnlyBetaAndPTR", "1"));
    }

    if let Some(deps) = &manifest.manifest.addon_dependencies {
        for (key, list) in [
            ("Dependencies", &deps.required),
            ("OptionalDeps", &deps.optional),
        ] {
            // Consecutive entries for the same flavors share one directive.
            // The lines keep the manifest order, which may influence which
            // dependency the client loads first.
            let mut runs: Vec<(Option<Vec<String>>, Vec<&str>)> = Vec::new();
            for dep in list {
                let restriction = if dep.flavors().is_empty() {
                    None
                } else {
                    let ids: Vec<String> = flavors
                        .iter()
                        .filter(|id| {
                            dep.flavors().iter().any(|f| {
                                wowlua_flavors::table()
                                    .resolve(f)
                                    .is_some_and(|f| f.id == **id)
                            })
                        })
                        .cloned()
                        .collect();
                    if ids.is_empty() {
                        continue; // only for untargeted flavors
                    }
                    (ids.len() < flavors.len()).then_some(ids)
                };
                match runs.last_mut() {
                    Some((last, names)) if *last == restriction => names.push(dep.name()),
                    _ => runs.push((restriction, vec![dep.name()])),
                }
            }
            for (restriction, names) in runs {
                out.push(Directive {
                    key: key.to_string(),
                    value: names.join(", "),
                    flavors: restriction,
                });
            }
        }
        if !deps.load_with.is_empty() {
            out.push(Directive::new("LoadWith", deps.load_with.join(", ")));
        }
        if !deps.load_managers.is_empty() {
            out.push(Directive::new(
                "LoadManagers",
                deps.load_managers.join(", "),
            ));
        }
    }

    if !addon.saved_variables.is_empty() {
        out.push(Directive::new(
            "SavedVariables",
            addon.saved_variables.join(", "),
        ));
    }
    if !addon.saved_variables_per_character.is_empty() {
        out.push(Directive::new(
            "SavedVariablesPerCharacter",
            addon.saved_variables_per_character.join(", "),
        ));
    }
    if addon.load_saved_variables_first {
        out.push(Directive::new("LoadSavedVariablesFirst", "1"));
    }
    if addon.allow_table_access {
        out.push(Directive::new("AllowAddOnTableAccess", "1"));
    }
    // Ordered set: addons may carry very many `X-` directives (QuestieDB
    // stores its database in TOC metadata).
    let mut extra_keys: indexmap::IndexSet<String> = addon.toc_extra.keys().cloned().collect();
    for (_, o) in &overrides {
        extra_keys.extend(o.toc_extra.keys().cloned());
    }
    for key in extra_keys {
        let per_flavor: Vec<(String, Option<Option<String>>)> = overrides
            .iter()
            .filter_map(|(f, o)| o.toc_extra.get(&key).map(|v| (f.clone(), Some(v.clone()))))
            .collect();
        push_field(
            &mut out,
            &key,
            addon.toc_extra.get(&key).cloned(),
            &per_flavor,
            flavors,
        );
    }
    // TOC data: lines shared by several flavors become one directive.
    let mut data: indexmap::IndexMap<String, Vec<(String, String)>> = indexmap::IndexMap::new();
    for (flavor, lines) in toc_data {
        for (key, value) in lines {
            data.entry(key).or_default().push((flavor.clone(), value));
        }
    }
    for (key, per_flavor) in data {
        let mut groups: indexmap::IndexMap<String, Vec<String>> = indexmap::IndexMap::new();
        for (flavor, value) in per_flavor {
            groups.entry(value).or_default().push(flavor);
        }
        for (value, group) in groups {
            out.push(Directive {
                key: key.clone(),
                value,
                flavors: (group.len() < flavors.len()).then_some(group),
            });
        }
    }
    out.push(Directive::new(
        "X-Wowlua-Build",
        format!("{profile} {build_id}"),
    ));
    if let Some(publish) = &manifest.manifest.publish {
        use wowlua_manifest::model::{ProjectId, TargetType};
        let mut seen = BTreeSet::new();
        for target in publish.targets.values() {
            let key = match target.target_type {
                TargetType::Curseforge => "X-Curse-Project-ID",
                TargetType::Wago => "X-Wago-ID",
                TargetType::Wowinterface => "X-WoWI-ID",
                _ => continue,
            };
            let Some(id) = &target.project_id else {
                continue;
            };
            if seen.insert(key) {
                let value = match id {
                    ProjectId::Number(n) => n.to_string(),
                    ProjectId::String(s) => s.clone(),
                };
                out.push(Directive::new(key, value));
            }
        }
    }
    out
}

fn toc_error(error: TocError, manifest: &Path) -> Diagnostic {
    match error {
        TocError::SingleUnsupported { client, feature } => Diagnostic::error(
            "E4301",
            format!(
                "`build.toc-layout` is `single`, but the client {client} does not support {feature}"
            ),
        )
        .with_path(manifest)
        .with_help("use `\"toc-layout\": \"auto\"` or drop the flavor"),
        TocError::Indistinguishable { flavors } => Diagnostic::error(
            "E4302",
            format!(
                "per-flavor TOC files cannot tell the flavors {} apart",
                flavors.join(", ")
            ),
        )
        .with_path(manifest)
        .with_help("use the single TOC layout or drop a flavor"),
        TocError::LineTooLong { toc, line } => Diagnostic::error(
            "E4303",
            format!("a line in {toc} is longer than 1024 characters: `{line}…`"),
        )
        .with_path(manifest),
    }
}

/// Whether `name` is a keybinding file the client loads automatically from
/// the AddOn root: `Bindings.xml` or `Bindings_<Suffix>.xml`.
pub fn is_bindings_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "bindings.xml"
        || lower
            .strip_prefix("bindings_")
            .and_then(|rest| rest.strip_suffix(".xml"))
            .is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_alphanumeric())
            })
}

/// Keybinding files in the package root, passed through unchanged (they are
/// loaded by the client, not by the TOC). Each must be well-formed XML.
fn bindings_files(session: &mut Session, root: &Path) -> Vec<(String, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut files: Vec<(String, Vec<u8>)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            is_bindings_file(&name).then(|| std::fs::read(e.path()).ok().map(|b| (name, b)))?
        })
        .collect();
    files.sort();
    for (name, bytes) in &files {
        let text = String::from_utf8_lossy(bytes);
        if let Err(error) = wowlua_xml::references(&text) {
            session.push(
                Diagnostic::error(
                    "E5002",
                    format!("invalid XML in `{name}`: {}", error.message),
                )
                .with_path(name),
            );
        }
    }
    files
}

fn licenses_text() -> String {
    format!(
        "Licenses of components embedded in this AddOn.\n\
         Generated by wowlua {}.\n\n\
         This build embeds no third-party components.\n",
        crate::COMPILER_VERSION
    )
}

/// Output validation (spec §13.8).
fn validate_output(
    session: &mut Session,
    out: &BTreeMap<String, Content>,
    tocs: &[TocFile],
    xml: &XmlInfo,
    output_paths: &HashMap<String, String>,
    variable_outputs: &BTreeSet<String>,
    roots: &BTreeSet<String>,
) {
    // Paths must not collide case-insensitively (E4308).
    let mut lower: HashMap<String, &str> = HashMap::new();
    for path in out.keys() {
        if let Some(other) = lower.insert(path.to_ascii_lowercase(), path) {
            session.push(Diagnostic::error(
                "E4308",
                format!("output paths `{other}` and `{path}` differ only in case"),
            ));
        }
    }

    // Every TOC entry exists, and every Lua/XML file is reachable (E5002).
    // Shared files are loaded by another AddOn's TOC.
    let mut reachable: BTreeSet<String> = roots.clone();
    for toc in tocs {
        for line in toc
            .content
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let path = line.split(" [").next().unwrap_or(line).replace('\\', "/");
            if path.starts_with("../") {
                continue; // a file of another AddOn
            }
            if path.contains('[') {
                // A variable line: its expansions were checked when it was built.
                reachable.extend(variable_outputs.iter().cloned());
                continue;
            }
            if !out.contains_key(&path) {
                session.push(Diagnostic::error(
                    "E5002",
                    format!(
                        "{} lists `{path}`, which is not part of the output",
                        toc.name
                    ),
                ));
            }
            reachable.insert(path);
        }
    }
    // Follow XML references from reachable XML files.
    let mut changed = true;
    while changed {
        changed = false;
        for (xml_source, targets) in &xml.edges {
            let xml_reachable = output_paths
                .get(xml_source)
                .is_some_and(|p| reachable.contains(p));
            if xml_reachable {
                for target in targets.iter().filter_map(|t| output_paths.get(t)) {
                    changed |= reachable.insert(target.clone());
                }
            }
        }
    }
    for path in out.keys() {
        let lower = path.to_ascii_lowercase();
        let is_code = lower.ends_with(".lua") || lower.ends_with(".xml");
        let exempt = path == "Bindings.xml" || !path.starts_with("src/");
        if is_code && !exempt && !reachable.contains(path) {
            session.push(Diagnostic::error(
                "E5002",
                format!("`{path}` is part of the output but is not loaded by any TOC or XML file"),
            ));
        }
    }
}

/// Writes the output tree to a temporary directory and swaps it into place.
fn write_tree(dir: &Path, out: &BTreeMap<String, Content>) -> std::io::Result<()> {
    let staged = fsutil::temp_sibling(dir)?;
    for (path, content) in out {
        let dest = staged.join(path);
        match content {
            Content::Bytes(bytes) => fsutil::write_file(&dest, bytes)?,
            Content::Copy(from) => {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(from, &dest)?;
            }
        }
    }
    fsutil::replace_dir(&staged, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flavors(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fields_group_flavors_by_value() {
        let all = flavors(&["standard", "mists", "wrath"]);
        let mut out = Vec::new();
        let overrides = vec![
            ("mists".to_string(), Some(Some("Mists".to_string()))),
            ("wrath".to_string(), Some(None)),
        ];
        push_field(&mut out, "Title", Some("Base".into()), &overrides, &all);
        assert_eq!(out.len(), 2);
        assert_eq!(
            (out[0].value.as_str(), out[0].flavors.clone()),
            ("Base", Some(flavors(&["standard"])))
        );
        assert_eq!(
            (out[1].value.as_str(), out[1].flavors.clone()),
            ("Mists", Some(flavors(&["mists"])))
        );

        let mut out = Vec::new();
        push_field(&mut out, "Group", Some("G".into()), &[], &all);
        assert_eq!(
            out[0].flavors, None,
            "a value every flavor shares needs no condition"
        );
    }

    #[test]
    fn recognizes_bindings_files() {
        assert!(is_bindings_file("Bindings.xml"));
        assert!(is_bindings_file("bindings_Mainline.xml"));
        assert!(!is_bindings_file("Bindings_.xml"));
        assert!(!is_bindings_file("MyBindings.xml"));
    }
}
