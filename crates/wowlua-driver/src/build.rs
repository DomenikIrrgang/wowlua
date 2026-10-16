//! Building a project into a complete AddOn folder (spec chapter 14).
//!
//! Lua sources are syntax-checked and passed through byte for byte;
//! `.wowlua` sources and the standard-library modules they import are
//! compiled. XML references are rewritten, TOC files are generated, and the
//! assembled output is validated before it is written atomically.

use crate::project::Project;
use crate::sources::{self, Source, SourceKind};
use crate::{Session, fsutil, vcs};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wowlua_diagnostics::Diagnostic;
use wowlua_luair::sourcemap::EncodedMap;
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
    /// A nolib package: without embedded `lua-library` dependencies
    /// (Publishing §11.2.2).
    pub nolib: bool,
    /// A publish target's `minify` / `bundle` over the profile's.
    pub minify: Option<String>,
    pub bundle: Option<bool>,
    /// `wowlua compile`: only these files, without assets and without the
    /// in-game dev tooling (there is no AddOn folder to load it from).
    pub only: Option<Only>,
    /// Worker threads and memory for per-file work (`--jobs`, #8).
    pub jobs: crate::parallel::Jobs,
    /// `wowlua test`: the test side (test files, mocks, helpers) is compiled
    /// with the addon, outside its TOC ([`BuildOutput::tests`]).
    pub tests: bool,
}

/// The files `wowlua compile` compiles.
#[derive(Clone, Debug, Default)]
pub struct Only {
    /// Absolute paths of `.wowlua` and `.lua` files, and `.d.wowlua`
    /// declaration files, all under the package root.
    pub files: Vec<PathBuf>,
    /// The project's own declaration files apply too (a real project; a
    /// standalone compile has only the given ones).
    pub project_declarations: bool,
}

impl BuildRequest {
    pub fn new(profile: impl Into<String>) -> Self {
        BuildRequest {
            profile: profile.into(),
            flavors: None,
            out_dir: None,
            write: true,
            nolib: false,
            minify: None,
            bundle: None,
            only: None,
            jobs: crate::parallel::Jobs::default(),
            tests: false,
        }
    }
}

/// A compiled file of the test side (spec Tooling §16.1.2).
#[derive(Clone, Debug)]
pub struct TestFile {
    /// Path relative to the AddOn folder.
    pub path: String,
    /// Path relative to the package root (`<wowlua>/test.wowlua` for
    /// standard-library modules).
    pub rel: String,
    pub role: crate::sources::TestRole,
    /// Output paths of the files it imports (addon files too), which must
    /// load first.
    pub deps: Vec<String>,
}

/// Content of one output file.
#[derive(Clone, Debug)]
pub enum Content {
    Bytes(Vec<u8>),
    Copy(PathBuf),
}

/// Result of a successful build.
#[derive(Clone, Debug)]
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
    /// With [`BuildRequest::tests`]: the test side, in an order its imports
    /// allow; none of it is in the TOCs.
    pub tests: Vec<TestFile>,
}

/// The Lua files of a build in load order (the TOCs' order, then files
/// loaded otherwise), with their content (`wowlua compile`).
pub fn lua_outputs(build: &BuildOutput) -> Vec<(String, Vec<u8>)> {
    let mut order: Vec<String> = Vec::new();
    for toc in &build.tocs {
        for line in toc.content.lines() {
            let path = line.trim().replace('\\', "/");
            if path.ends_with(".lua") && build.files.contains_key(&path) && !order.contains(&path) {
                order.push(path);
            }
        }
    }
    for path in build.files.keys() {
        if path.ends_with(".lua") && !order.contains(path) {
            order.push(path.clone());
        }
    }
    order
        .into_iter()
        .map(|path| {
            let bytes = match &build.files[&path] {
                Content::Bytes(b) => b.clone(),
                Content::Copy(from) => std::fs::read(from).unwrap_or_default(),
            };
            (path, bytes)
        })
        .collect()
}

/// The dependency edges of one compilation's flavor (`None` in shared mode).
type FlavorEdges = (Option<String>, Vec<(String, String)>);

/// One compilation (per flavor in split mode).
struct FlavorBuild {
    flavor: Option<String>,
    meta: wowlua_sema::meta::Meta,
    output: crate::compile::CompileOutput,
}

/// A per-flavor copy of an XML file (spec codegen §14.8.4): references to
/// per-flavor files stay relative (they sit in the same flavor tree);
/// references to shared files become absolute in-game paths. Returns the
/// content and the shared files it loads.
fn flavor_xml(
    text: &str,
    rel: &str,
    files: &[(Source, FileId)],
    per_flavor: &BTreeSet<String>,
    xml_per_flavor: &BTreeSet<String>,
    addon: &str,
) -> (String, Vec<String>) {
    let Ok(refs) = wowlua_xml::references(text) else {
        return (text.to_string(), Vec::new());
    };
    let Some((me, _)) = files.iter().find(|(s, _)| s.rel == rel) else {
        return (text.to_string(), Vec::new());
    };
    let dir = me.src_rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut replacements = Vec::new();
    let mut shared = Vec::new();
    for reference in &refs {
        // Resolved by path: a flavor's variant may load other files than
        // the shared text (packager flavor blocks).
        let joined = normalize(&if dir.is_empty() {
            reference.normalized()
        } else {
            format!("{dir}/{}", reference.normalized())
        })
        .to_ascii_lowercase();
        let Some((source, _)) = files.iter().find(|(s, _)| {
            let src = s.src_rel.to_ascii_lowercase();
            src == joined
                || src
                    .strip_suffix(".wowlua")
                    .is_some_and(|stem| format!("{stem}.lua") == joined)
        }) else {
            continue;
        };
        if per_flavor.contains(&source.rel) || xml_per_flavor.contains(&source.rel) {
            continue;
        }
        let output = source.output_path();
        shared.push(output.clone());
        let conditions = reference.file[reference.path().len()..].to_string();
        replacements.push((
            reference,
            format!(
                "Interface\\AddOns\\{addon}\\{}{conditions}",
                output.replace('/', "\\")
            ),
        ));
    }
    (wowlua_xml::rewrite(text, &replacements), shared)
}

/// The runtime's output path (spec codegen §14.8).
const RUNTIME_PATH: &str = "wowlua/runtime.lua";
/// Where the compiled locale files go (spec stdlib "locale").
const LOCALES_PATH: &str = "wowlua/locales/";

/// Output entries that are reserved at the top level of the AddOn (§14.8).
const RESERVED_DIRS: &[&str] = &["src", "wowlua", "libs"];
const RESERVED_FILES: &[&str] = &["Bindings.xml", "LICENSES.txt"];

/// Builds the project. Returns `None` if any error was reported.
pub fn build(
    session: &mut Session,
    project: &Project,
    request: &BuildRequest,
) -> Option<BuildOutput> {
    build_inner(session, project, request, None)
}

/// [`build`] for `wowlua dev`: files compiled by an earlier build with
/// `cache` are taken from it where compiling them again would give the
/// same, and this build's are kept for the next.
pub fn build_with_cache(
    session: &mut Session,
    project: &Project,
    request: &BuildRequest,
    cache: &mut crate::BuildCache,
) -> Option<BuildOutput> {
    build_inner(session, project, request, Some(cache))
}

fn build_inner(
    session: &mut Session,
    project: &Project,
    request: &BuildRequest,
    mut cache: Option<&mut crate::BuildCache>,
) -> Option<BuildOutput> {
    if let Some(cache) = cache.as_deref_mut() {
        cache.begin(crate::incremental::config_hash(
            &session.db,
            project,
            request,
        ));
    }
    let manifest = &project.manifest;
    let root = manifest.root.clone();
    let addon_name = project.addon_name();
    let kind = manifest.package().map(|p| p.kind).unwrap_or_default();
    // A wowlua library is checked on its own and built into its addons.
    let checkable = kind == wowlua_manifest::model::PackageKind::Library && !request.write;
    if kind != wowlua_manifest::model::PackageKind::Addon && !checkable {
        let mut d = Diagnostic::plain_error(format!(
            "a package of kind `{}` is not built on its own",
            serde_json::to_value(kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default()
        ))
        .with_path(&manifest.path)
        .with_note("libraries are embedded into the addons that depend on them");
        if kind == wowlua_manifest::model::PackageKind::Library {
            d = d.with_help("run `wowlua check` to check the library");
        }
        session.push(d);
        return None;
    }

    let profile = match manifest.profile(&request.profile) {
        Ok(mut profile) => {
            if let Some(m) = &request.minify {
                profile.minify = m.clone();
            }
            if let Some(b) = request.bundle {
                profile.bundle = b;
            }
            if request.only.is_some() {
                profile.dev_console = false;
                profile.monitor = false;
                profile.capture_errors = false;
            }
            profile
        }
        Err(_) => {
            session.push(
                Diagnostic::error("E4006", format!("unknown profile `{}`", request.profile))
                    .with_path(&manifest.path),
            );
            return None;
        }
    };
    // Source maps are only ever written next to the AddOn folder, so a
    // build that writes nothing (check, toc, compile) does not make them.
    // The profile keeps its value: `meta` and the runtime read the profile.
    let source_maps = profile.source_maps && request.write;
    let Some(level) = wowlua_manifest::minify_passes(&manifest.manifest, &profile.minify) else {
        session.push(
            Diagnostic::error(
                "E4006",
                format!("unknown minification level `{}`", profile.minify),
            )
            .with_path(&manifest.path),
        );
        return None;
    };
    let passes = wowlua_luair::Passes::from_names(level.iter().map(String::as_str));

    let flavors = select_flavors(session, project, request.flavors.as_deref())?;
    // Embedded `lua-library` dependencies (spec §10.5.2).
    let libs = crate::embed::lua_libraries(session, project, &flavors)?;
    // wowlua `library` dependencies, compiled with the package (§10.5.1).
    let wlibs = crate::embed::wowlua_libraries(session, project, &flavors);
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
    let (sources, given_declarations) = match &request.only {
        Some(only) => {
            let (sources, declarations) =
                sources::given(session, &only.files, &root, &build_cfg.src);
            (sources, Some(declarations))
        }
        None => {
            let mut found =
                sources::discover_sources(&root, &build_cfg.src, &build_cfg.out, &exclude);
            if request.tests {
                found.extend(sources::discover_tests(
                    &root,
                    &build_cfg.src,
                    &build_cfg.out,
                    &exclude,
                ));
            }
            (found, None)
        }
    };
    let mut assets = sources::discover_assets(
        &root,
        &build_cfg.out,
        &sources::glob_set_ci(&build_cfg.assets),
        &exclude,
    );
    let mut sources = sources;
    if request.only.is_some() {
        assets.clear();
    }
    // Release builds of a packager project ship only what git tracks, like
    // the packager, which leaves out every untracked file (decision T147).
    if manifest.manifest.compat.packager_keywords && !profile.debug {
        let tracked = sources::tracked_files(&root);
        if !tracked.is_empty() {
            // Both sides are paths under `root`.
            let is_tracked = |abs: &Path| tracked.contains(abs);
            let mut left_out: Vec<String> = Vec::new();
            sources.retain(|s| {
                let keep = is_tracked(&s.abs);
                if !keep {
                    left_out.push(s.rel.clone());
                }
                keep
            });
            assets.retain(|a| {
                let keep = is_tracked(&a.abs);
                if !keep {
                    left_out.push(a.rel.clone());
                }
                keep
            });
            if !left_out.is_empty() {
                session.push(Diagnostic::note(format!(
                    "left out of the release build, as git does not track them: {}",
                    left_out.join(", ")
                )));
            }
        }
    }
    // An addon without code is valid (voice packs, saved-variable stores).

    // Load and check every source.
    // Raw bytes are kept: plain Lua passes through byte for byte (Lua strings
    // may contain any bytes). The source database holds a lossy text copy for
    // diagnostics.
    let mut files: Vec<(Source, FileId)> = Vec::new();
    let mut raw: HashMap<String, Arc<[u8]>> = HashMap::new();
    for source in sources {
        match std::fs::read(&source.abs) {
            Ok(bytes) => {
                // Valid UTF-8 (nearly every file) is kept once, for the
                // source database and as the bytes.
                let (file, bytes) = match String::from_utf8(bytes) {
                    Ok(text) => {
                        let text: Arc<str> = text.into();
                        (
                            session.db.add_shared(&source.rel, text.clone()),
                            text.into(),
                        )
                    }
                    Err(error) => {
                        let bytes = error.into_bytes();
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        (session.db.add(&source.rel, text), bytes.into())
                    }
                };
                raw.insert(source.rel.clone(), bytes);
                files.push((source, file));
            }
            Err(error) => session.push(
                Diagnostic::plain_error(format!("cannot read source file: {error}"))
                    .with_path(&source.rel),
            ),
        }
    }
    let mut declarations = Vec::new();
    let declaration_sources = match (&request.only, given_declarations) {
        (Some(only), Some(given)) if !only.project_declarations => given,
        (_, given) => {
            let mut all =
                sources::discover_declarations(&root, &build_cfg.src, &build_cfg.out, &exclude);
            for source in given.unwrap_or_default() {
                if !all.iter().any(|d| d.abs == source.abs) {
                    all.push(source);
                }
            }
            all
        }
    };
    for source in declaration_sources {
        match std::fs::read(&source.abs) {
            Ok(bytes) => {
                let file = session
                    .db
                    .add(&source.rel, String::from_utf8_lossy(&bytes).into_owned());
                declarations.push(crate::compile::Declaration {
                    source,
                    file,
                    bytes: bytes.into(),
                });
            }
            Err(error) => session.push(
                Diagnostic::plain_error(format!("cannot read declaration file: {error}"))
                    .with_path(&source.rel),
            ),
        }
    }
    // Declarations of `types` packages and `types` paths.
    for (src_rel, abs) in crate::embed::type_declarations(project) {
        match std::fs::read(&abs) {
            Ok(bytes) => {
                let file = session
                    .db
                    .add(&src_rel, String::from_utf8_lossy(&bytes).into_owned());
                declarations.push(crate::compile::Declaration {
                    source: sources::Source {
                        rel: src_rel.clone(),
                        src_rel,
                        abs,
                        kind: SourceKind::Wowlua,
                        output: None,
                    },
                    file,
                    bytes: bytes.into(),
                });
            }
            Err(error) => session.push(Diagnostic::plain_error(format!(
                "cannot read declaration file `{}`: {error}",
                abs.display()
            ))),
        }
    }
    // Generated declaration files of the workspace addons this addon lists
    // in `addon-dependencies` (spec §10.8).
    if let Some(deps) = &manifest.manifest.addon_dependencies {
        for dep in deps.required.iter().chain(&deps.optional) {
            let Some(other) = project
                .workspace_addons
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(dep.name()))
            else {
                continue;
            };
            let text = crate::workspace::global_declarations(&other.src);
            let rel = format!("@workspace/{}.d.wowlua", other.name);
            let file = session.db.add(&rel, text.clone());
            declarations.push(crate::compile::Declaration {
                source: sources::Source {
                    rel: rel.clone(),
                    src_rel: rel,
                    abs: PathBuf::new(),
                    kind: SourceKind::Wowlua,
                    output: None,
                },
                file,
                bytes: text.into_bytes().into(),
            });
        }
    }
    // The compiler's implicit declarations (Lua 5.1 and WoW's Lua).
    for builtin in wowlua_stdlib::declarations() {
        let file = session.db.add_static(builtin.path, builtin.text);
        declarations.push(crate::compile::Declaration {
            source: sources::Source {
                rel: builtin.path.to_string(),
                src_rel: builtin.path.to_string(),
                abs: PathBuf::new(),
                kind: SourceKind::Wowlua,
                output: None,
            },
            file,
            bytes: builtin.text.as_bytes().into(),
        });
    }
    // Version and build id (`wowlua:meta` needs them while compiling).
    let vcs = vcs::info(&root);
    let version = build_version(session, project, &profile, vcs.as_ref())?;
    let build_id = build_id(session, project, &profile.name, &flavors, &files, &assets);
    let asset_rels: Vec<String> = assets.iter().map(|a| a.rel.clone()).collect();

    // Packager keywords in passthrough files (spec CLI §12.6). Flavor blocks
    // are applied per flavor after compiling.
    let keywords = manifest.manifest.compat.packager_keywords;
    let mut flavor_blocks: BTreeSet<String> = BTreeSet::new();
    let keyword_info = keywords.then(|| {
        let alpha = semver::Version::parse(&version).is_ok_and(|v| !v.pre.is_empty());
        let now = crate::keywords::build_time();
        crate::keywords::KeywordInfo::from_git(&root, &version, profile.debug, alpha, now)
    });
    if let Some(info) = &keyword_info {
        let plain = &manifest.manifest.compat.plain_copy;
        for (source, _) in &files {
            if !matches!(source.kind, SourceKind::Lua | SourceKind::Xml)
                || crate::keywords::plain_copy(plain, &[&source.src_rel, &source.rel])
            {
                continue;
            }
            // Line endings as `packager -u` writes them (decision T124).
            let normalized = crate::keywords::lf(&raw[&source.rel]);
            if normalized[..] != raw[&source.rel][..] {
                raw.insert(source.rel.clone(), normalized.into());
            }
            let Ok(text) = std::str::from_utf8(&raw[&source.rel]) else {
                continue;
            };
            if crate::keywords::uses_localization(text) {
                session.push(
                    Diagnostic::warning(
                        "W4030",
                        "`@localization(…)@` is not supported; the line is left unchanged",
                    )
                    .with_path(&source.rel)
                    .with_help("use `wowlua:locale` instead"),
                );
            }
            let mut processed = crate::keywords::apply(
                text,
                info,
                Path::new(&source.rel),
                Path::new(&source.src_rel),
                source.kind == SourceKind::Xml,
            );
            if request.nolib && source.kind == SourceKind::Xml {
                processed = crate::keywords::nolib_xml(&processed);
            }
            // Like the packager: a project released game by game, or one
            // with a single flavor, gets each game's blocks; a multi-version
            // build leaves `@retail@` blocks as written and rejects
            // `@version-…@` ones (decision T145).
            let multi_version =
                !manifest.manifest.compat.per_game() && project.manifest.flavors.len() > 1;
            if multi_version && crate::keywords::has_flavor_blocks(&processed) {
                if crate::keywords::has_version_blocks(&processed) {
                    session.push(
                        Diagnostic::error(
                            "E4335",
                            "build type version keywords (`@version-…@`) are not allowed in a multi-version build",
                        )
                        .with_path(&source.rel)
                        .with_help("use Lua conditions on `WOW_PROJECT_ID` or TOC load conditions, like the packager asks"),
                    );
                } else {
                    session.push(
                        Diagnostic::warning(
                            "W4031",
                            "`@retail@` blocks are left as written in a multi-version build, like the packager does",
                        )
                        .with_path(&source.rel)
                        .with_help("the code runs on every client; use Lua conditions on `WOW_PROJECT_ID` to limit it"),
                    );
                }
            } else if crate::keywords::has_flavor_blocks(&processed) {
                if flavors.len() == 1 {
                    processed = crate::keywords::apply_flavor(
                        &processed,
                        &flavors[0],
                        source.kind == SourceKind::Xml,
                    );
                } else if build_cfg.flavor_mode == wowlua_manifest::model::FlavorMode::Split {
                    flavor_blocks.insert(source.rel.clone());
                } else {
                    session.push(
                        Diagnostic::plain_error(
                            "packager flavor blocks (`@version-…@`, `@retail@`) need `build.flavor-mode: \"split\"` when several flavors are built",
                        )
                        .with_path(&source.rel)
                        .with_help("use `wowlua:meta`'s `flavor` or TOC load conditions instead"),
                    );
                }
            }
            if processed != text {
                raw.insert(source.rel.clone(), processed.into_bytes().into());
            }
        }
        // Later steps (XML references, diagnostics) read the processed text.
        for (source, file) in &mut files {
            let current = session.db.get(*file).text().as_bytes();
            if let Some(bytes) = raw.get(&source.rel)
                && bytes[..] != *current
            {
                *file = session
                    .db
                    .add(&source.rel, String::from_utf8_lossy(bytes).into_owned());
            }
        }
    }

    // XML references.
    let xml = resolve_xml(
        session,
        &files,
        &libs,
        &addon_name,
        keywords && profile.debug,
        (&flavor_blocks, &flavors),
    );

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

    // Flavors with an order of their own (`build.flavor-load-order`,
    // decision T126).
    let own_orders: BTreeMap<String, Vec<String>> = build_cfg
        .flavor_load_order
        .iter()
        .filter_map(|(f, list)| {
            let id = wowlua_flavors::table().resolve(f)?.id.clone();
            flavors.contains(&id).then(|| (id, list.clone()))
        })
        .collect();
    let order_of = |flavor: Option<&str>| -> &[String] {
        flavor
            .and_then(|f| own_orders.get(f))
            .unwrap_or(&build_cfg.load_order)
    };
    let loaded_by: Vec<Option<usize>> = {
        let index: HashMap<&str, usize> = files
            .iter()
            .enumerate()
            .map(|(i, (s, _))| (s.rel.as_str(), i))
            .collect();
        let mut parent = vec![None; files.len()];
        for (xml_file, targets) in &xml.edges {
            for t in targets {
                if let (Some(&p), Some(&c)) = (index.get(xml_file.as_str()), index.get(t.as_str()))
                {
                    parent[c].get_or_insert(p);
                }
            }
        }
        parent
    };
    // A load-on-demand addon loads after its required addons have run
    // (spec §5.4.3): the load order sees which tables are theirs.
    let addon = manifest.manifest.addon.as_ref();
    let after_dependencies = addon.is_some_and(|a| a.load_on_demand)
        && manifest
            .manifest
            .addon_dependencies
            .as_ref()
            .is_some_and(|d| !d.required.is_empty());
    let saved_variables = addon.map(|a| a.all_saved_variables()).unwrap_or_default();

    // Compile: once per flavor in split mode, once in shared mode (spec
    // §6.3). Sources are parsed once.
    let mut diagnostics = Vec::new();
    // The checker's settings and the wow-api declarations of each flavor
    // (embedded until the package manager provides them, decision T83).
    let check_cfg = crate::compile::CheckConfig {
        mode: manifest.manifest.check.mode,
        strict: sources::glob_set(&manifest.manifest.check.strict),
        lua: manifest.manifest.check.lua,
        globals: manifest
            .manifest
            .check
            .globals
            .iter()
            .cloned()
            .chain(
                manifest
                    .manifest
                    .addon
                    .iter()
                    .flat_map(|a| a.all_saved_variables()),
            )
            .collect(),
        stdlib: wowlua_sema::stdlib::StdlibConfig {
            saved: manifest
                .manifest
                .addon
                .iter()
                .flat_map(|a| {
                    a.saved_variables.iter().cloned().chain(
                        a.flavor_overrides
                            .values()
                            .flat_map(|o| o.saved_variables.iter().flatten().cloned()),
                    )
                })
                .collect(),
            saved_per_char: manifest
                .manifest
                .addon
                .iter()
                .flat_map(|a| {
                    a.saved_variables_per_character.iter().cloned().chain(
                        a.flavor_overrides.values().flat_map(|o| {
                            o.saved_variables_per_character.iter().flatten().cloned()
                        }),
                    )
                })
                .collect(),
            xml_bindings: xml_binding_names(&root),
            auras: flavors
                .iter()
                .filter_map(|f| {
                    let words = wowlua_api::aura_filter_words(f)?;
                    Some((
                        f.clone(),
                        wowlua_sema::stdlib::FlavorAuras {
                            words: words.into_iter().collect(),
                            containers: wowlua_api::has_aura_containers(f),
                        },
                    ))
                })
                .collect(),
            tooltips: flavors
                .iter()
                .filter_map(|f| {
                    let info = wowlua_api::tooltip_info(f)?;
                    Some((
                        f.clone(),
                        wowlua_sema::stdlib::FlavorTooltips {
                            kinds: info.kinds.into_iter().collect(),
                            data_system: info.data_system,
                        },
                    ))
                })
                .collect(),
        },
    };
    // Shared flavor mode: where each API exists, for flavor guards (E2030).
    let shared_flavors = (build_cfg.flavor_mode == wowlua_manifest::model::FlavorMode::Shared
        && flavors.len() > 1)
        .then(|| {
            let table = wowlua_flavors::table();
            let mut per_flavor: Vec<(String, std::collections::HashSet<String>)> = Vec::new();
            for f in &flavors {
                if let Some([_, own]) = wowlua_api::declarations(f) {
                    // The text leaves out its globals of a primitive type.
                    let mut names = wowlua_api::declared_names(own.text);
                    names.extend(own.globals.iter().map(|g| g.name.to_string()));
                    per_flavor.push((f.clone(), names));
                }
            }
            let mut availability: HashMap<String, BTreeSet<String>> = HashMap::new();
            for (_, names) in &per_flavor {
                for n in names {
                    if availability.contains_key(n) {
                        continue;
                    }
                    let have: BTreeSet<String> = per_flavor
                        .iter()
                        .filter(|(_, ns)| ns.contains(n))
                        .map(|(f, _)| f.clone())
                        .collect();
                    if have.len() < per_flavor.len() {
                        availability.insert(n.clone(), have);
                    }
                }
            }
            let project_ids = flavors
                .iter()
                .filter_map(|f| table.flavor(f).map(|fl| (fl.project_id, f.clone())))
                .collect();
            let project_constants = [
                ("WOW_PROJECT_MAINLINE", 1),
                ("WOW_PROJECT_CLASSIC", 2),
                ("WOW_PROJECT_BURNING_CRUSADE_CLASSIC", 5),
                ("WOW_PROJECT_WRATH_CLASSIC", 11),
                ("WOW_PROJECT_CATACLYSM_CLASSIC", 14),
                ("WOW_PROJECT_MISTS_CLASSIC", 19),
            ]
            .into_iter()
            .map(|(n, v)| (n.to_string(), v))
            .collect();
            wowlua_sema::check::flavors::SharedFlavors {
                flavors: flavors.iter().cloned().collect(),
                project_ids,
                project_constants,
                availability,
            }
        });
    // `wowlua dev` does not parse unchanged plain Lua files again (#7
    // stage 3), unless a pass reads every parse (`rename-private`, dead
    // modules) or `check.lua` type-checks and lints them.
    let summarize =
        cache.is_some() && !passes.rename_private && !profile.optimize.dead_code && !check_cfg.lua;
    let mut parsed = crate::compile::parse_sources(
        &session.db,
        &mut diagnostics,
        &files,
        &declarations,
        &raw,
        cache.as_deref_mut(),
        summarize,
        &request.jobs,
    );
    crate::compile::add_libraries(
        &mut session.db,
        &mut diagnostics,
        &mut files,
        &mut declarations,
        &mut raw,
        &mut parsed,
        &wlibs,
    );
    // Locales (spec stdlib "locale"): read first, so `L` gets their type;
    // reported and compiled only when the package uses `wowlua:locale`.
    let mut locale_diagnostics = Vec::new();
    let locales = crate::locale::load(
        &root.join(&build_cfg.locales_dir),
        &build_cfg.locales_dir,
        &build_cfg.locale_base,
        &mut locale_diagnostics,
    );
    let overrides: HashMap<&str, String> =
        [("locale", crate::locale::declaration(locales.as_ref()))].into();
    crate::compile::add_stdlib(
        &mut session.db,
        &mut diagnostics,
        &mut files,
        &mut declarations,
        &mut raw,
        &mut parsed,
        &overrides,
    );
    let uses_locale = files
        .iter()
        .any(|(s, _)| s.src_rel == format!("{}locale.wowlua", wowlua_sema::STDLIB_PREFIX));
    let locale_files = if uses_locale {
        diagnostics.extend(locale_diagnostics);
        locales
            .as_ref()
            .map(|l| crate::locale::emit(l, build_cfg.locales))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // Files of dependencies: their warnings and lints are not the package's.
    let dependency_files: HashSet<wowlua_span::FileId> = files
        .iter()
        .filter(|(s, _)| wowlua_sema::package_of(&s.src_rel).is_some())
        .map(|(_, f)| *f)
        .chain(
            declarations
                .iter()
                .filter(|d| wowlua_sema::package_of(&d.source.src_rel).is_some())
                .map(|d| d.file),
        )
        .collect();
    // Lints (spec appendix B), once per file: `.wowlua` files, and `.lua`
    // files with `check.lua`. `--!nolint` turns them off for a file.
    let mut nolint: HashSet<wowlua_span::FileId> = HashSet::new();
    let mut lint_files: Vec<usize> = Vec::new();
    for (i, (source, file)) in files.iter().enumerate() {
        // A summarized file is not linted (it would not be summarized).
        if let Some(crate::compile::Front::Summarized { nolint: off, .. }) = parsed.fronts.get(i) {
            if *off {
                nolint.insert(*file);
            }
            continue;
        }
        let Some(parse) = parsed.parses.get(i).and_then(Option::as_ref) else {
            continue;
        };
        if !parsed.ok[i]
            || wowlua_sema::is_compiler_known(&source.src_rel)
            || dependency_files.contains(file)
        {
            continue;
        }
        if parse
            .chunk
            .directives
            .iter()
            .any(|d| d.name.as_ref() == "nolint")
        {
            nolint.insert(*file);
            continue;
        }
        let lintable = match source.kind {
            SourceKind::Wowlua => true,
            SourceKind::Lua => check_cfg.lua,
            _ => false,
        };
        if lintable {
            lint_files.push(i);
        }
    }
    // Linting is pure per file: on worker threads (#8), in file order.
    request.jobs.region(
        lint_files,
        |&i| raw[&files[i].0.rel].len() as u64,
        20,
        |i| {
            let parse = parsed.parses[i].as_ref().expect("linted files are parsed");
            wowlua_sema::lint::lint(parse, &raw[&files[i].0.rel])
        },
        |lints| diagnostics.extend(lints),
    );
    // Plain Lua files compiled for instrumentation (decision T97): not
    // embedded libraries, nor files another AddOn loads.
    let lower_lua: HashSet<String> =
        if profile.instrument == wowlua_manifest::model::Instrument::All || profile.monitor {
            files
                .iter()
                .map(|(s, _)| s)
                .filter(|s| {
                    s.kind == SourceKind::Lua
                        && !wowlua_sema::is_compiler_known(&s.src_rel)
                        && wowlua_sema::package_of(&s.src_rel).is_none()
                })
                .filter(|s| {
                    !s.src_rel
                        .split('/')
                        .next()
                        .is_some_and(|first| first.eq_ignore_ascii_case(&build_cfg.libs_dir))
                })
                .filter(|s| !build_cfg.shared_files.contains(&s.rel))
                .map(|s| s.rel.clone())
                .collect()
        } else {
            HashSet::new()
        };
    // Plain Lua that is minified and optimized with the compiled code: not
    // libraries (unless `minify-libraries`), packager flavor blocks or
    // `plain-copy`.
    // `rename-private` keeps the names the XML and libraries use.
    let foreign: Vec<Vec<u8>> = if passes.rename_private {
        files
            .iter()
            .filter(|(s, _)| s.kind == SourceKind::Xml)
            .filter_map(|(s, _)| raw.get(&s.rel).map(|b| b.to_vec()))
            .chain(libs.iter().flat_map(|lib| {
                lib.index
                    .values()
                    .filter(|rel| {
                        let lower = rel.to_ascii_lowercase();
                        lower.ends_with(".lua") || lower.ends_with(".xml")
                    })
                    .filter_map(|rel| std::fs::read(lib.dir.join(rel)).ok())
                    .collect::<Vec<_>>()
            }))
            .collect()
    } else {
        Vec::new()
    };
    let minify =
        crate::compile::Minify {
            passes,
            foreign,
            keep: build_cfg.keep.clone(),
            lua: if passes.is_empty()
                && !profile.optimize.dead_code
                && profile.optimize.cache_globals == wowlua_manifest::model::CacheGlobals::Off
            {
                HashSet::new()
            } else {
                files
                    .iter()
                    .map(|(s, _)| s)
                    .filter(|s| s.kind == SourceKind::Lua && !flavor_blocks.contains(&s.rel))
                    .filter(|s| {
                        profile.minify_libraries
                            || !s.src_rel.split('/').next().is_some_and(|first| {
                                first.eq_ignore_ascii_case(&build_cfg.libs_dir)
                            })
                    })
                    .filter(|s| {
                        !crate::keywords::plain_copy(
                            &manifest.manifest.compat.plain_copy,
                            &[&s.src_rel, &s.rel],
                        )
                    })
                    .map(|s| s.rel.clone())
                    .collect()
            },
        };
    let split = build_cfg.flavor_mode == wowlua_manifest::model::FlavorMode::Split;
    let compile_flavors: Vec<Option<String>> = if split {
        flavors.iter().cloned().map(Some).collect()
    } else {
        vec![None]
    };
    let mut builds: Vec<FlavorBuild> = Vec::new();
    // Files a flavor compiles the same as an earlier one (or, in `wowlua
    // dev`, an earlier build) are taken from it.
    let mut flavor_cache = cache
        .as_deref_mut()
        .map(crate::BuildCache::lend)
        .unwrap_or_default();
    let mut api_shared: Option<crate::compile::ParsedDeclaration> = None;
    for (idx, flavor) in compile_flavors.iter().enumerate() {
        let meta = crate::meta::build_meta(
            project,
            &crate::meta::MetaInput {
                profile: &profile,
                flavor: flavor.as_deref(),
                flavors: &flavors,
                version: &version,
                build_id: &build_id,
                vcs: vcs.as_ref(),
                assets: &asset_rels,
                addon_name: &addon_name,
            },
        );
        let active: Vec<bool> = files
            .iter()
            .map(|(source, _)| {
                // Files another AddOn's TOC loads (`build.shared-files`) are
                // shipped, not loaded by this one.
                !build_cfg.shared_files.contains(&source.rel)
                    && flavor.as_ref().is_none_or(|f| {
                        restriction(&source.rel).is_none_or(|list| list.contains(f))
                            && (!in_other_variant(&source.rel, f, &build_cfg.variable_files)
                                || build_cfg.load_order.contains(&source.rel))
                    })
            })
            .collect();
        // Shared mode types against the first flavor's API; flavor guards
        // (E2030) cover what other flavors lack.
        let api_flavor = flavor
            .clone()
            .or_else(|| flavors.first().cloned())
            .unwrap_or_default();
        // The declarations every flavor shares are parsed once, the first
        // time a flavor needs them, right before its own: a one-flavor build
        // adds the files in the same order as always. No other flavor of the
        // build types against a flavor's own API (split flavors are
        // distinct, and shared mode compiles once), so that parse is freed
        // with the flavor.
        let api_own;
        let api: Vec<&crate::compile::ParsedDeclaration> = match wowlua_api::own(&api_flavor) {
            Some(own) => {
                let keep_tokens = minify.passes.rename_private;
                if api_shared.is_none() {
                    api_shared = Some(parse_api(
                        &mut session.db,
                        wowlua_api::shared(),
                        keep_tokens,
                    ));
                }
                api_own = parse_api(&mut session.db, own, keep_tokens);
                vec![api_shared.as_ref().expect("parsed above"), &api_own]
            }
            None => Vec::new(),
        };
        let units: HashMap<String, crate::compile::LibraryUnit> = wlibs
            .iter()
            .map(|l| {
                let m = &l.manifest;
                let unit = crate::compile::LibraryUnit {
                    check: crate::compile::CheckConfig {
                        mode: m.check.mode,
                        strict: sources::glob_set(&m.check.strict),
                        lua: m.check.lua,
                        globals: m.check.globals.iter().cloned().collect(),
                        stdlib: wowlua_sema::stdlib::StdlibConfig::default(),
                    },
                    main: l.main.as_ref().map(|r| l.src_rel(r)),
                    meta: std::sync::Arc::new(crate::meta::library_meta(&meta, m, &l.entry)),
                };
                (l.name.clone(), unit)
            })
            .collect();
        let output = crate::compile::compile_flavor(
            &session.db,
            &mut diagnostics,
            &files,
            &declarations,
            &raw,
            &parsed,
            &profile,
            source_maps,
            std::sync::Arc::new(meta.clone()),
            wowlua_sema::loadorder::Layout {
                listed: &listed_positions(
                    &files,
                    order_of(flavor.as_deref()),
                    &xml,
                    flavor.as_ref().map_or(&flavors[..], std::slice::from_ref),
                ),
                active: &active,
                loaded_by: &loaded_by,
                after_dependencies,
                saved_variables: &saved_variables,
            },
            &api,
            &check_cfg,
            if flavor.is_none() {
                shared_flavors.as_ref()
            } else {
                None
            },
            &lower_lua,
            &units,
            &minify,
            &mut flavor_cache,
            idx + 1 < compile_flavors.len(),
            &request.jobs,
        );
        builds.push(FlavorBuild {
            flavor: flavor.clone(),
            meta,
            output,
        });
    }
    // The parses and the flavor cache are not read after compiling; freeing
    // them here keeps them out of the memory that assembly and bundling add
    // on top. `wowlua dev` keeps the cache for the next build.
    // Plain Lua files no flavor type-checked and the build did not lint
    // need no parse in a later build.
    if let Some(cache) = cache.as_deref_mut() {
        let checked: HashSet<usize> = builds
            .iter()
            .flat_map(|b| b.output.checked.iter().copied())
            .collect();
        for (i, front) in parsed.fronts.iter().enumerate() {
            if let crate::compile::Front::Fresh {
                hash,
                usage,
                nolint,
            } = front
                && !checked.contains(&i)
            {
                let (source, file) = &files[i];
                cache.keep_front(&source.rel, *file, *hash, usage.clone(), *nolint);
            }
        }
    }
    // The package's syntax trees, freed on the workers (#49).
    let weights: Vec<u64> = files
        .iter()
        .map(|(s, _)| raw.get(&s.rel).map_or(0, |b| b.len() as u64))
        .collect();
    request.jobs.drop_all(
        parsed.into_parses().into_iter().enumerate().collect(),
        |(i, _)| weights.get(*i).copied().unwrap_or(0),
    );
    drop(declarations);
    match cache.as_deref_mut() {
        Some(cache) => cache.keep(flavor_cache),
        None => drop(flavor_cache),
    }
    drop(api_shared);
    // Packager flavor blocks: each flavor's variant of the file.
    for rel in &flavor_blocks {
        let is_xml = rel.to_ascii_lowercase().ends_with(".xml");
        for b in &mut builds {
            let Some(f) = b.flavor.clone() else { continue };
            let base = b
                .output
                .compiled
                .get(rel)
                .map(|c| c.code.clone())
                .or_else(|| xml.rewrites.get(rel).map(|t| t.clone().into_bytes()))
                .unwrap_or_else(|| raw[rel].to_vec());
            let code = crate::keywords::apply_flavor(&String::from_utf8_lossy(&base), &f, is_xml);
            b.output.compiled.insert(
                rel.clone(),
                std::sync::Arc::new(crate::compile::Compiled {
                    code: code.into_bytes(),
                    map: None,
                    compiles: false,
                }),
            );
        }
    }
    // One message per problem, not one per flavor.
    let mut seen = BTreeSet::new();
    for d in diagnostics {
        let file = d.labels.first().map(|l| l.span.file);
        if d.is_lint() && file.is_some_and(|f| nolint.contains(&f)) {
            continue;
        }
        if !d.is_error() && file.is_some_and(|f| dependency_files.contains(&f)) {
            continue;
        }
        let key = format!(
            "{:?}|{}|{:?}",
            d.code,
            d.message,
            d.labels.first().map(|l| (l.span.start, l.span.end))
        );
        if seen.insert(key) {
            session.push(d);
        }
    }
    // Compiled code that differs between flavors is stored per flavor
    // (spec codegen §14.8.3).
    let mut per_flavor: BTreeSet<String> = BTreeSet::new();
    for (source, _) in &files {
        let codes: Vec<Option<&Vec<u8>>> = builds
            .iter()
            .map(|b| b.output.compiled.get(&source.rel).map(|c| &c.code))
            .collect();
        if codes.windows(2).any(|w| w[0] != w[1]) {
            per_flavor.insert(source.rel.clone());
        }
    }
    let mut edges: Vec<(String, String)> = builds
        .iter()
        .flat_map(|b| b.output.edges.iter().cloned())
        .collect();
    edges.sort();
    edges.dedup();
    let mut edge_reasons = HashMap::new();
    for b in &builds {
        for (pair, reason) in &b.output.edge_reasons {
            edge_reasons
                .entry(pair.clone())
                .or_insert_with(|| reason.clone());
        }
    }
    let needs_runtime = builds.iter().any(|b| {
        b.output.needs_runtime
            || !b.output.meta_used.is_empty()
            || !tool_slices(&profile, &b.output.slices).is_empty()
    });
    let runtimes: Vec<String> = builds
        .iter()
        .map(|b| {
            wowlua_runtime::runtime(&wowlua_runtime::RuntimeOptions {
                modules: b.output.modules.clone(),
                debug: profile.debug,
                log_level: serde_json::to_value(profile.log_level)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default(),
                log_history: manifest.manifest.stdlib.log.history_size,
                profile_errors: profile.profile_errors,
                saved_first: manifest
                    .manifest
                    .addon
                    .as_ref()
                    .is_some_and(|a| a.load_saved_variables_first),
                dev_info: if profile.dev_console {
                    vec![
                        ("version".into(), version.to_string()),
                        ("profile".into(), profile.name.clone()),
                        ("build".into(), build_id.clone()),
                        // The same in every flavor's runtime, so one TOC
                        // can load it.
                        ("flavors".into(), flavors.join(", ")),
                        ("compiler".into(), crate::COMPILER_VERSION.to_string()),
                    ]
                } else {
                    Vec::new()
                },
                runtime_version: runtime_version(),
                meta: crate::meta::runtime_tables(
                    &b.meta,
                    &b.output.meta_used,
                    manifest.manifest.meta.runtime == wowlua_manifest::model::MetaRuntime::Full,
                    &flavors,
                    &project.interfaces,
                ),
                slices: tool_slices(&profile, &b.output.slices),
            })
        })
        .collect();
    // The runtime is minified like the code that uses it.
    let runtime_mode = crate::compile::line_mode(&profile);
    let runtimes: Vec<String> = runtimes
        .into_iter()
        .map(|text| {
            crate::compile::minify_lua(
                &mut session.db,
                RUNTIME_PATH,
                text.as_bytes(),
                &minify.passes,
                runtime_mode,
            )
            .and_then(|code| String::from_utf8(code).ok())
            .unwrap_or(text)
        })
        .collect();
    let runtime_per_flavor = runtimes.windows(2).any(|w| w[0] != w[1]);

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

    // Modules `dead-code` removed in every flavor (W6002).
    let removed: BTreeSet<String> = builds
        .iter()
        .map(|b| b.output.removed.clone())
        .reduce(|a, b| a.intersection(&b).cloned().collect())
        .unwrap_or_default();
    for rel in &removed {
        session.push(
            Diagnostic::warning(
                "W6002",
                "this module is removed: it only declares things and nothing uses it",
            )
            .with_path(rel)
            .with_help("import it where it is needed, or list it in `build.keep`"),
        );
    }
    // Expansions for flavors this build does not target (`--flavor`): not
    // this build's files.
    let variants = variant_files(&build_cfg.variable_files, &files);
    let test_side = test_side(&files, &edges);
    let mut toc_items: Vec<TocItem> = Vec::new();
    let mut emitted: Vec<&Source> = Vec::new();
    for (source, _) in &files {
        if removed.contains(&source.rel) {
            continue;
        }
        if variants.contains(&source.rel) && !loaded_by_variable.contains(&source.rel) {
            continue;
        }
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
            && !test_side.contains_key(&source.rel)
        {
            toc_items.push(TocItem {
                rel: source.rel.clone(),
                path: source.output_path(),
                flavors: flavors_of,
                locales: None,
                dependency: false,
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
            dependency: false,
        });
    }
    // Library entry files, unless the package's own XML loads them.
    for (i, lib) in libs.iter().enumerate() {
        if request.nolib {
            break;
        }
        for (entry, entry_flavors) in &lib.entries {
            if xml.lib_refs.get(&i).is_some_and(|r| r.contains(entry)) {
                continue;
            }
            let flavors_of = match (&lib.flavors, entry_flavors) {
                (Some(a), Some(b)) => Some(a.iter().filter(|f| b.contains(f)).cloned().collect()),
                (a, b) => a.clone().or_else(|| b.clone()),
            };
            let path = lib.output(entry);
            toc_items.push(TocItem {
                rel: path.clone(),
                path,
                flavors: flavors_of,
                locales: None,
                dependency: true,
            });
        }
    }
    let unordered = (!own_orders.is_empty()).then(|| toc_items.clone());
    order_by_load_order(&mut toc_items, &build_cfg.load_order);
    let flavor_edges: Vec<FlavorEdges> = builds
        .iter()
        .map(|b| (b.flavor.clone(), b.output.edges.clone()))
        .collect();
    let mut rel_orders = order_by_dependencies(
        session,
        &mut toc_items,
        &edges,
        &edge_reasons,
        &flavor_edges,
        &xml,
        &build_cfg.load_order,
    );
    // Conflicting edges force per-flavor orders (W4304); explicit ones are
    // wanted.
    let conflicting = rel_orders.is_some();
    if let Some(base) = unordered {
        let mut orders = rel_orders.take().unwrap_or_default();
        let default: Vec<String> = toc_items.iter().map(|i| i.rel.clone()).collect();
        for f in &flavors {
            let rels = match own_orders.get(f) {
                Some(order) => {
                    let mut items = base.clone();
                    order_by_load_order(&mut items, order);
                    let own_edges = flavor_edges
                        .iter()
                        .find(|(fl, _)| fl.as_deref() == Some(f.as_str()))
                        .map(|(_, e)| e.clone())
                        .unwrap_or_else(|| edges.clone());
                    let _ = order_by_dependencies(
                        session,
                        &mut items,
                        &own_edges,
                        &edge_reasons,
                        &[(Some(f.clone()), own_edges.clone())],
                        &xml,
                        order,
                    );
                    items.iter().map(|i| i.rel.clone()).collect()
                }
                None => orders.get(f).cloned().unwrap_or_else(|| default.clone()),
            };
            orders.insert(f.clone(), rels);
        }
        // Identical orders need no per-flavor TOCs.
        let first = orders.values().next().cloned();
        rel_orders = (!orders.values().all(|o| Some(o) == first.as_ref())).then_some(orders);
    }
    // XML files that load per-flavor code are emitted per flavor too.
    let mut xml_per_flavor: BTreeSet<String> = BTreeSet::new();
    loop {
        let before = xml_per_flavor.len();
        for (x, targets) in &xml.edges {
            if targets
                .iter()
                .any(|t| per_flavor.contains(t) || xml_per_flavor.contains(t))
            {
                xml_per_flavor.insert(x.clone());
            }
        }
        if xml_per_flavor.len() == before {
            break;
        }
    }
    // The runtime loads before every module (spec §5.4.1).
    if needs_runtime {
        toc_items.insert(
            0,
            TocItem {
                rel: RUNTIME_PATH.into(),
                path: RUNTIME_PATH.into(),
                flavors: None,
                locales: None,
                dependency: false,
            },
        );
    }
    // Locale files before everything, the base first; `[AllowLoadTextLocale]`
    // where every targeted client knows it (else their guards skip them).
    let text_locale = flavors.iter().all(|f| {
        wowlua_flavors::table()
            .clients_of(f)
            .all(|c| c.features.allow_load_text_locale)
    });
    for (i, file) in locale_files.iter().enumerate() {
        toc_items.insert(
            i,
            TocItem {
                rel: file.path.clone(),
                path: file.path.clone(),
                flavors: None,
                locales: if text_locale {
                    file.locales.clone()
                } else {
                    None
                },
                dependency: false,
            },
        );
    }
    // Per-flavor variants take the place of the shared file in the order.
    let flavor_paths = |rel: &str| -> bool {
        per_flavor.contains(rel)
            || xml_per_flavor.contains(rel)
            || (rel == RUNTIME_PATH && runtime_per_flavor)
    };
    let toc_items: Vec<TocItem> = toc_items
        .into_iter()
        .flat_map(|item| {
            if !flavor_paths(&item.rel) {
                return vec![item];
            }
            builds
                .iter()
                .filter_map(|b| b.flavor.clone())
                .filter(|f| item.flavors.as_ref().is_none_or(|list| list.contains(f)))
                .map(|f| TocItem {
                    rel: item.rel.clone(),
                    path: format!("wowlua/flavors/{f}/{}", item.path),
                    flavors: Some(vec![f]),
                    locales: item.locales.clone(),
                    dependency: item.dependency,
                })
                .collect()
        })
        .collect();
    // Flavors that need incompatible orders get per-flavor TOC files, each
    // in its own order (spec §5.4.6).
    if conflicting && build_cfg.toc_layout == TocLayout::Auto {
        session.push(
            Diagnostic::warning(
                "W4304",
                "the flavors need different load orders; using per-flavor TOC files",
            )
            .with_path(&manifest.path),
        );
    }
    let flavor_orders = rel_orders.map(|orders| {
        orders
            .into_iter()
            .map(|(flavor, rels)| {
                let applies =
                    |item: &TocItem| item.flavors.as_ref().is_none_or(|l| l.contains(&flavor));
                let mut indices: Vec<usize> = toc_items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| {
                        (item.rel == RUNTIME_PATH || item.rel.starts_with(LOCALES_PATH))
                            && applies(item)
                    })
                    .map(|(i, _)| i)
                    .collect();
                for rel in &rels {
                    indices.extend(
                        toc_items
                            .iter()
                            .enumerate()
                            .filter(|(_, item)| &item.rel == rel && applies(item))
                            .map(|(i, _)| i),
                    );
                }
                (flavor, indices)
            })
            .collect()
    });
    let toc_data = read_toc_data(session, project, &flavors);

    if session.has_errors() {
        return None;
    }

    // Assemble the output tree.
    let mut out: BTreeMap<String, Content> = BTreeMap::new();
    let mut flavor_roots: BTreeSet<String> = BTreeSet::new();
    let mut maps: BTreeMap<String, Arc<EncodedMap>> = BTreeMap::new();
    // Output paths of compiled code that Lua 5.1 compiled as it is: bundling
    // need not compile it again while the output keeps exactly those bytes.
    let mut known: HashMap<String, std::sync::Arc<crate::compile::Compiled>> = HashMap::new();
    for source in &emitted {
        if per_flavor.contains(&source.rel) {
            for b in &builds {
                let (Some(f), Some(c)) = (&b.flavor, b.output.compiled.get(&source.rel)) else {
                    continue;
                };
                let path = format!("wowlua/flavors/{f}/{}", source.output_path());
                if xml.loaded_by_xml.contains(&source.rel) {
                    flavor_roots.insert(path.clone());
                }
                if let Some(map) = &c.map {
                    maps.insert(format!("{f}/{}.map", source.output_path()), map.clone());
                }
                // A flavor's XML loads shared files by their in-game path.
                if source.kind == SourceKind::Xml {
                    let (content, shared_targets) = flavor_xml(
                        &String::from_utf8_lossy(&c.code),
                        &source.rel,
                        &files,
                        &per_flavor,
                        &xml_per_flavor,
                        &addon_name,
                    );
                    flavor_roots.extend(shared_targets);
                    out.insert(path, Content::Bytes(content.into_bytes()));
                    continue;
                }
                if c.compiles {
                    known.insert(path.clone(), std::sync::Arc::clone(c));
                }
                out.insert(path, Content::Bytes(c.code.clone()));
            }
            continue;
        }
        let compiled = builds
            .iter()
            .find_map(|b| b.output.compiled.get(&source.rel));
        if let Some(map) = compiled.and_then(|c| c.map.clone()) {
            maps.insert(format!("{}.map", source.output_path()), map);
        }
        let original: &[u8] = compiled.map_or(&raw[&source.rel], |c| &c.code);
        if xml_per_flavor.contains(&source.rel) {
            let text = match xml.rewrites.get(&source.rel) {
                Some(rewritten) => rewritten.clone(),
                None => String::from_utf8_lossy(original).into_owned(),
            };
            for b in &builds {
                let Some(f) = &b.flavor else { continue };
                let (content, shared_targets) = flavor_xml(
                    &text,
                    &source.rel,
                    &files,
                    &per_flavor,
                    &xml_per_flavor,
                    &addon_name,
                );
                for t in shared_targets {
                    flavor_roots.insert(t);
                }
                let path = format!("wowlua/flavors/{f}/{}", source.output_path());
                if xml.loaded_by_xml.contains(&source.rel) {
                    flavor_roots.insert(path.clone());
                }
                out.insert(path, Content::Bytes(content.into_bytes()));
            }
            continue;
        }
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
            None => original.to_vec(),
        };
        if let Some(c) = compiled.filter(|c| c.compiles) {
            known.insert(source.output_path(), std::sync::Arc::clone(c));
        }
        out.insert(source.output_path(), Content::Bytes(content));
    }
    let mut lib_licenses: Vec<(String, String)> = Vec::new();
    let mut unlicensed: Vec<String> = Vec::new();
    // Output paths of embedded libraries' files: they win over the
    // project's own files at the same path, like the packager's externals.
    let mut lib_outputs: BTreeSet<String> = BTreeSet::new();
    // Output files the packager copies as they are (archives, a library's
    // `plain-copy`, a downloaded license): no line-ending conversion.
    let mut raw_paths: BTreeSet<String> = BTreeSet::new();
    for (i, lib) in libs.iter().enumerate() {
        // A nolib package leaves the libraries out (their licenses too).
        if request.nolib {
            break;
        }
        let mut roots: Vec<String> = lib.entries.iter().map(|(e, _)| e.clone()).collect();
        roots.extend(xml.lib_refs.get(&i).into_iter().flatten().cloned());
        let reachable = crate::embed::reachable(session, lib, &roots);
        // A converted packager project ships every file of an external but
        // dotfiles and what its `.pkgmeta` or the project ignores, like the
        // packager (decision T144); otherwise what the addon can load.
        let own = lib.project_file.as_ref().and_then(|(file, sub)| {
            let text = std::fs::read_to_string(file).ok()?;
            Some((crate::pkgmeta::parse(&text).ok()?, sub.clone()))
        });
        let in_repo = |rel: &str| match &own {
            Some((_, sub)) if !sub.is_empty() => format!("{sub}/{rel}"),
            _ => rel.to_string(),
        };
        let ignored = |repo_rel: &str| {
            own.as_ref()
                .is_some_and(|(m, _)| crate::keywords::plain_copy(&m.ignore, &[repo_rel]))
        };
        let visible = |rel: &str| !rel.split('/').any(|part| part.starts_with('.'));
        // (file, path in the library's output folder, path in its repository)
        let mut shipped: Vec<(PathBuf, String, String)> = if keywords {
            lib.index
                .values()
                .filter(|rel| visible(rel) && !ignored(&in_repo(rel)))
                .map(|rel| (lib.dir.join(rel), rel.clone(), in_repo(rel)))
                .collect()
        } else {
            reachable
                .into_iter()
                .map(|rel| (lib.dir.join(&rel), rel.clone(), in_repo(&rel)))
                .collect()
        };
        // A folder `move-folders` moved up leaves its neighbours in place.
        if keywords && let Some((outer, up)) = &lib.moved_from {
            let outer_sub = own
                .as_ref()
                .map(|(_, sub)| {
                    sub.trim_end_matches(up.as_str())
                        .trim_end_matches('/')
                        .to_string()
                })
                .unwrap_or_default();
            let inner: BTreeSet<String> = shipped
                .iter()
                .map(|(_, r, _)| r.to_ascii_lowercase())
                .collect();
            for entry in walkdir::WalkDir::new(outer).into_iter().flatten() {
                if !entry.file_type().is_file() {
                    continue;
                }
                let Ok(rel) = entry.path().strip_prefix(outer) else {
                    continue;
                };
                let rel = rel.to_string_lossy().replace('\\', "/");
                let repo_rel = if outer_sub.is_empty() {
                    rel.clone()
                } else {
                    format!("{outer_sub}/{rel}")
                };
                if rel.starts_with(&format!("{up}/"))
                    || !visible(&rel)
                    || ignored(&repo_rel)
                    || inner.contains(&rel.to_ascii_lowercase())
                {
                    continue;
                }
                shipped.push((entry.path().to_path_buf(), rel, repo_rel));
            }
        }
        shipped.retain(|(_, rel, _)| !exclude.is_match(lib.output(rel)));
        // Archives and `plain-copy` files are copied as they are.
        let archive = matches!(lib.source, wowlua_pkg::Source::Archive { .. });
        // SVN author and tag need the network; only asked for when used.
        let svn_detail = keywords
            && matches!(lib.source, wowlua_pkg::Source::Svn { .. })
            && shipped.iter().any(|(abs, rel, _)| {
                crate::keywords::processed_extension(rel)
                    && std::fs::read_to_string(abs).is_ok_and(|t| {
                        crate::keywords::SVN_DETAIL_KEYWORDS
                            .iter()
                            .any(|k| t.contains(k))
                    })
            });
        let mut lib_info: Option<crate::keywords::KeywordInfo> = None;
        for (abs, rel, repo_rel) in shipped {
            let lower = rel.to_ascii_lowercase();
            let xml = lower.ends_with(".xml");
            let unchanged = archive
                || own
                    .as_ref()
                    .is_some_and(|(m, _)| crate::keywords::plain_copy(&m.plain_copy, &[&repo_rel]));
            lib_outputs.insert(lib.output(&rel));
            if unchanged {
                raw_paths.insert(lib.output(&rel));
            }
            // Library files get the packager's processing (decisions T120,
            // T124): keywords and blocks, LF line endings.
            if keywords && !unchanged && crate::keywords::processed_extension(&rel) {
                let bytes = std::fs::read(&abs).unwrap_or_default();
                let normalized = crate::keywords::lf(&bytes);
                let processed = match std::str::from_utf8(&normalized) {
                    Ok(text) if text.contains('@') => {
                        let info = lib_info.get_or_insert_with(|| {
                            crate::keywords::package_info(
                                &lib.source,
                                lib.pin.as_deref(),
                                profile.debug,
                                svn_detail,
                            )
                        });
                        let mut out = if xml || lower.ends_with(".lua") {
                            crate::keywords::apply_library(text, info, xml)
                        } else {
                            crate::keywords::substitute(text, info)
                        };
                        if out.contains("@file-") {
                            out = crate::keywords::library_file_keywords(
                                &out, lib, info, &rel, &bytes,
                            );
                        }
                        out.into_bytes()
                    }
                    _ => normalized,
                };
                if processed != bytes {
                    out.insert(lib.output(&rel), Content::Bytes(processed));
                    continue;
                }
            }
            out.insert(lib.output(&rel), Content::Copy(abs));
        }
        match crate::embed::license_text(lib) {
            Some(text) => lib_licenses.push((format!("{} ({})", lib.name, lib.install), text)),
            None => unlicensed.push(lib.name.clone()),
        }
    }
    // `minify-libraries`: the libraries' Lua files are minified too.
    if profile.minify_libraries && !minify.passes.is_empty() {
        let mode = crate::compile::line_mode(&profile);
        for path in &lib_outputs {
            if raw_paths.contains(path) || !path.to_ascii_lowercase().ends_with(".lua") {
                continue;
            }
            let Some(content) = out.get(path) else {
                continue;
            };
            let bytes = match content {
                Content::Bytes(b) => b.clone(),
                Content::Copy(p) => std::fs::read(p).unwrap_or_default(),
            };
            if let Some(code) =
                crate::compile::minify_lua(&mut session.db, path, &bytes, &minify.passes, mode)
            {
                out.insert(path.clone(), Content::Bytes(code));
            }
        }
    }
    // Files the project keeps in a library's folder ship too where the
    // library has none, as the packager copies the project first and the
    // externals over it (decision T144).
    if let Some(info) = &keyword_info
        && !request.nolib
    {
        let tracked = (!profile.debug).then(|| sources::tracked_files(&root));
        for lib in &libs {
            let folder = root.join(&lib.install);
            for entry in walkdir::WalkDir::new(&folder)
                .into_iter()
                .filter_entry(|e| {
                    e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.')
                })
                .flatten()
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let Ok(rel) = entry.path().strip_prefix(&root) else {
                    continue;
                };
                let rel = rel.to_string_lossy().replace('\\', "/");
                let untracked = tracked
                    .as_ref()
                    .is_some_and(|t| !t.is_empty() && !t.contains(entry.path()));
                if out.contains_key(&rel) || exclude.is_match(&rel) || untracked {
                    continue;
                }
                let plain =
                    crate::keywords::plain_copy(&manifest.manifest.compat.plain_copy, &[&rel]);
                let lower = rel.to_ascii_lowercase();
                let content = match std::fs::read(entry.path()) {
                    Ok(bytes) if !plain && crate::keywords::processed_extension(&rel) => {
                        let normalized = crate::keywords::lf(&bytes);
                        match std::str::from_utf8(&normalized) {
                            Ok(text) if lower.ends_with(".lua") || lower.ends_with(".xml") => {
                                let path = Path::new(&rel);
                                Content::Bytes(
                                    crate::keywords::apply(
                                        text,
                                        info,
                                        path,
                                        path,
                                        lower.ends_with(".xml"),
                                    )
                                    .into_bytes(),
                                )
                            }
                            _ => Content::Bytes(packaged_text(&bytes, info, &rel)),
                        }
                    }
                    _ => Content::Copy(entry.path().to_path_buf()),
                };
                lib_outputs.insert(rel.clone());
                if plain {
                    raw_paths.insert(rel.clone());
                }
                out.insert(rel, content);
            }
        }
    }
    for lib in &wlibs {
        let used = files
            .iter()
            .any(|(s, _)| wowlua_sema::package_of(&s.src_rel) == Some(lib.name.as_str()));
        if !used {
            continue;
        }
        match crate::embed::wowlua_license_text(lib) {
            Some(text) => lib_licenses.push((format!("{} ({})", lib.name, lib.out), text)),
            None => unlicensed.push(lib.name.clone()),
        }
    }
    if !unlicensed.is_empty() {
        session.push(
            Diagnostic::warning(
                "W4306",
                format!(
                    "embedded {} without a recognizable license file or header: {}",
                    if unlicensed.len() == 1 { "library" } else { "libraries" },
                    unlicensed.join(", ")
                ),
            )
            .with_help("check their licenses before publishing; `lua-library.license-files` can name the file"),
        );
    }
    out.insert(
        "LICENSES.txt".into(),
        Content::Bytes(licenses_text(profile.dev_console, &lib_licenses).into_bytes()),
    );
    // The dev window's fonts (spec §7.7.1).
    if profile.dev_console {
        for (name, bytes) in wowlua_runtime::DEV_FONTS {
            out.insert(
                format!("wowlua/fonts/{name}"),
                Content::Bytes(bytes.to_vec()),
            );
        }
    }
    let hand_written = bindings_files(session, &root);
    // `wowlua:bindings` (spec stdlib "bindings"): every flavor's bindings in
    // one generated Bindings.xml.
    let mut declared: Vec<&wowlua_sema::stdlib::BindingDecl> = Vec::new();
    for b in &builds {
        for d in &b.output.bindings {
            if !declared.iter().any(|x| x.name == d.name) {
                declared.push(d);
            }
        }
    }
    if let Some(first) = declared.first() {
        if let Some((name, _)) = hand_written.first() {
            session.push(
                Diagnostic::error(
                    "E4334",
                    format!("`{name}` is hand-written, and `wowlua:bindings` would generate one"),
                )
                .with_label(first.span, "a `@binding`")
                .with_help("declare every binding with `@binding`, or keep the hand-written file and drop `wowlua:bindings`"),
            );
        } else {
            let clients: Vec<&wowlua_flavors::Client> = flavors
                .iter()
                .flat_map(|f| wowlua_flavors::table().clients_of(f))
                .collect();
            let lacking: Vec<&str> = clients
                .iter()
                .filter(|c| !c.features.binding_defaults)
                .map(|c| c.name.as_str())
                .collect();
            let defaults = lacking.len() < clients.len();
            for d in &declared {
                let Some((key, span)) = &d.default else {
                    continue;
                };
                let message = if defaults {
                    format!(
                        "the default key `{key}` of `{}` is ignored on {}",
                        d.name,
                        lacking.join(", ")
                    )
                } else {
                    format!(
                        "the default key `{key}` of `{}` is ignored: no targeted client supports default keys",
                        d.name
                    )
                };
                if !lacking.is_empty() {
                    session.push(
                        Diagnostic::warning("W8354", message)
                            .with_label(*span, "no default key there"),
                    );
                }
            }
            out.insert(
                "Bindings.xml".into(),
                Content::Bytes(bindings_xml(&declared, &addon_name, defaults).into_bytes()),
            );
        }
    }
    for (name, mut bytes) in hand_written {
        // The packager processes them like the TOC's XML files.
        if let Some(info) = &keyword_info
            && !crate::keywords::plain_copy(&manifest.manifest.compat.plain_copy, &[&name])
        {
            bytes = crate::keywords::lf(&bytes);
            if let Ok(text) = std::str::from_utf8(&bytes) {
                let path = Path::new(&name);
                bytes = crate::keywords::apply(text, info, path, path, true).into_bytes();
            }
        }
        out.insert(name, Content::Bytes(bytes));
    }
    if needs_runtime {
        if runtime_per_flavor {
            for (b, text) in builds.iter().zip(&runtimes) {
                if let Some(f) = &b.flavor {
                    out.insert(
                        format!("wowlua/flavors/{f}/{RUNTIME_PATH}"),
                        Content::Bytes(text.clone().into_bytes()),
                    );
                }
            }
        } else if let Some(text) = runtimes.first() {
            out.insert(
                RUNTIME_PATH.into(),
                Content::Bytes(text.clone().into_bytes()),
            );
        }
    }
    for file in &locale_files {
        out.insert(
            file.path.clone(),
            Content::Bytes(file.code.clone().into_bytes()),
        );
    }
    for asset in &assets {
        // A converted project's library folders hold what the libraries
        // ship (their files win, like the packager's externals).
        if keywords && lib_outputs.contains(&asset.rel) {
            continue;
        }
        // `build.flavor-files` restricts assets too.
        if restriction(&asset.rel).is_some_and(|list| !list.iter().any(|f| flavors.contains(f))) {
            continue;
        }
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
        // Shipped text files get the packager's keywords and line endings.
        if let Some(info) = &keyword_info
            && crate::keywords::processed_extension(&asset.rel)
            && !crate::keywords::plain_copy(&manifest.manifest.compat.plain_copy, &[&asset.rel])
            && let Ok(bytes) = std::fs::read(&asset.abs)
        {
            out.insert(
                asset.rel.clone(),
                Content::Bytes(packaged_text(&bytes, info, &asset.rel)),
            );
            continue;
        }
        out.insert(asset.rel.clone(), Content::Copy(asset.abs.clone()));
    }
    // What the packager adds to the package (decision T146).
    if let Some(info) = &keyword_info {
        package_files(project, &root, info, &addon_name, &mut out, &mut raw_paths);
    }
    // The workspace root's license in a member that has none (`.pkgmeta`
    // `license-output`, copied into `move-folders` addons by the packager).
    if let Some(ws) = &project.workspace_root
        && std::fs::canonicalize(ws).ok() != std::fs::canonicalize(&root).ok()
        && let Some(license) = std::fs::read_to_string(ws.join(wowlua_manifest::MANIFEST_FILE))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|m| {
                m.pointer("/compat/license-output")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
        && !out.contains_key(&license)
        && let Some((bytes, downloaded)) = license_bytes(ws, &license)
    {
        // A license of the repository is copied as the packager processed
        // it; a downloaded one as it was saved.
        let bytes = match &keyword_info {
            Some(info) if !downloaded => packaged_text(&bytes, info, &license),
            _ => bytes,
        };
        if downloaded {
            raw_paths.insert(license.clone());
        }
        out.insert(license.clone(), Content::Bytes(bytes));
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
    let mut roots: BTreeSet<String> = emitted
        .iter()
        .filter(|s| shared.contains(&s.rel))
        .map(|s| s.output_path())
        .collect();
    roots.extend(flavor_roots);
    // Every other file of the project the packager would copy (decision
    // T155): its tests, docs and unloaded files, processed like its own.
    // The game loads none of them, so they need not be reachable.
    let mut shipped_as_is: BTreeSet<String> = BTreeSet::new();
    if let Some(info) = &keyword_info {
        let tracked = (!profile.debug).then(|| sources::tracked_files(&root));
        let members: Vec<PathBuf> = manifest
            .manifest
            .workspace
            .iter()
            .flat_map(|w| w.members.iter())
            .map(|m| root.join(m))
            .collect();
        let mut skip = vec![root.join(&build_cfg.src), root.join(&build_cfg.out)];
        // A nolib package leaves the libraries' folders out entirely.
        if request.nolib {
            skip.extend(libs.iter().map(|l| root.join(&l.install)));
        }
        ship_project_files(
            &root,
            &members,
            &skip,
            &exclude,
            &manifest.manifest.compat.plain_copy,
            tracked.as_ref(),
            info,
            &mut out,
            &mut shipped_as_is,
        );
        // A workspace without a root addon: its own files go into the AddOn
        // its package folder becomes (the AddOn's own files win, like the
        // packager's `move-folders`).
        if let Some((ws, ws_manifest)) = suite_package_root(project, &root, &addon_name) {
            let ws_members: Vec<PathBuf> = ws_manifest
                .pointer("/workspace/members")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|m| m.as_str())
                .map(|m| ws.join(m))
                .collect();
            let strings = |pointer: &str| -> Vec<String> {
                ws_manifest
                    .pointer(pointer)
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            };
            let ws_tracked = (!profile.debug).then(|| sources::tracked_files(&ws));
            let skip = vec![ws.join(&build_cfg.out)];
            ship_project_files(
                &ws,
                &ws_members,
                &skip,
                &sources::glob_set(&strings("/build/exclude")),
                &strings("/compat/plain-copy"),
                ws_tracked.as_ref(),
                info,
                &mut out,
                &mut shipped_as_is,
            );
        }
    }
    // CR LF for what the packager processes, unless its runs pass `-u`
    // (decision T154).
    if keywords
        && manifest.manifest.compat.line_endings == Some(wowlua_manifest::model::LineEndings::Crlf)
    {
        let plain = &manifest.manifest.compat.plain_copy;
        for (path, content) in out.iter_mut() {
            let generated = path == RUNTIME_PATH
                || path.starts_with(LOCALES_PATH)
                || path == "LICENSES.txt"
                || path.starts_with("wowlua/std/")
                || path.starts_with("wowlua/fonts/")
                || path.ends_with(&format!("/{RUNTIME_PATH}"));
            let in_src = path.strip_prefix("src/").unwrap_or(path);
            if generated
                || raw_paths.contains(path)
                || !crate::keywords::processed_extension(path)
                || crate::keywords::plain_copy(plain, &[path, in_src])
            {
                continue;
            }
            let bytes = match content {
                Content::Bytes(b) => std::mem::take(b),
                Content::Copy(p) => std::fs::read(&*p).unwrap_or_default(),
            };
            *content = Content::Bytes(crate::keywords::crlf(&bytes));
        }
    }
    roots.extend(shipped_as_is);
    // Bundling (spec optimization §15.3) rewrites the TOC's file list.
    let (toc_items, flavor_orders) = if profile.bundle {
        bundle_output(
            session,
            &BundleInput {
                addon: &addon_name,
                build_id: &build_id,
                flavors: &flavors,
                libs: &libs,
                files: &files,
                raw: &raw,
                source_maps,
                known: &known,
            },
            toc_items,
            flavor_orders,
            &mut out,
            &mut maps,
        )?
    } else {
        (toc_items, flavor_orders)
    };
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
            // A nolib package names its libraries' standalone addons
            // (not in a converted project: the packager adds none).
            &if request.nolib && !keywords {
                libs.iter()
                    .map(|l| {
                        l.addon_name.clone().unwrap_or_else(|| {
                            l.install
                                .rsplit('/')
                                .next()
                                .unwrap_or(&l.install)
                                .to_string()
                        })
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            },
            // The dev window's state (spec §7.7.1).
            profile.dev_console.then(|| {
                let safe: String = addon_name
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                format!("WowluaDevDB_{safe}")
            }),
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
        flavor_orders,
    };
    let mut tocs = match wowlua_toc::generate(&toc_input) {
        Ok(generated) => generated.files,
        Err(errors) => {
            for error in errors {
                session.push(toc_error(error, &manifest.path));
            }
            return None;
        }
    };
    // The packager substitutes keywords in the whole TOC (`## Notes: …
    // @project-version@`).
    if let Some(info) = &keyword_info {
        for toc in &mut tocs {
            if toc.content.contains('@') {
                toc.content = crate::keywords::substitute(&toc.content, info);
            }
        }
    }

    for toc in &tocs {
        out.insert(
            toc.name.clone(),
            Content::Bytes(toc.content.clone().into_bytes()),
        );
    }
    validate_output(
        session,
        &out,
        &tocs,
        &xml,
        &output_paths,
        &variable_outputs,
        &roots,
        &addon_name,
    );
    if session.has_errors() {
        return None;
    }

    let dir = request.out_dir.clone().unwrap_or_else(|| {
        wowlua_pkg::source::normalize(&manifest.out_dir().join(&profile.name).join(&addon_name))
    });
    let written = match cache {
        // `wowlua dev` (#7 stage 4): only what changed is written.
        Some(cache) if request.write => {
            let previous = cache.written.take();
            crate::incremental::write_changed(&dir, &out, &addon_name, &maps, previous.as_ref())
                .map(|w| cache.written = Some(w))
        }
        _ if request.write => {
            write_tree(&dir, &out).and_then(|()| write_maps(&dir, &addon_name, &maps))
        }
        _ => Ok(()),
    };
    if let Err(error) = written {
        session.push(
            Diagnostic::plain_error(format!("cannot write the build output: {error}"))
                .with_path(&dir),
        );
        return None;
    }

    let tests = test_files(&files, &edges, &test_side, &out);
    Some(BuildOutput {
        addon_name,
        dir,
        profile: profile.name,
        flavors,
        version,
        build_id,
        tocs,
        files: out,
        tests,
    })
}

/// The test side of a build (spec Tooling §16.1.1): test files, mocks and
/// helpers, and the standard-library modules only they import.
fn test_side(
    files: &[(Source, FileId)],
    edges: &[(String, String)],
) -> HashMap<String, sources::TestRole> {
    let mut side: HashMap<String, sources::TestRole> = files
        .iter()
        .filter_map(|(s, _)| Some((s.rel.clone(), sources::TestRole::of(&s.rel)?)))
        .collect();
    if side.is_empty() {
        return side;
    }
    // What the addon's own files need, directly or not.
    let mut needed: HashSet<&str> = files
        .iter()
        .map(|(s, _)| s.rel.as_str())
        .filter(|rel| !side.contains_key(*rel) && !rel.starts_with("<wowlua>/"))
        .collect();
    loop {
        let before = needed.len();
        for (a, b) in edges {
            if needed.contains(b.as_str()) {
                needed.insert(a.as_str());
            }
        }
        if needed.len() == before {
            break;
        }
    }
    for (s, _) in files {
        if s.rel.starts_with("<wowlua>/") && !needed.contains(s.rel.as_str()) {
            side.insert(s.rel.clone(), sources::TestRole::Std);
        }
    }
    side
}

/// The test side's compiled files, each after the test-side files it
/// imports (otherwise by path).
fn test_files(
    files: &[(Source, FileId)],
    edges: &[(String, String)],
    side: &HashMap<String, sources::TestRole>,
    out: &BTreeMap<String, Content>,
) -> Vec<TestFile> {
    let output_of: HashMap<&str, String> = files
        .iter()
        .map(|(s, _)| (s.rel.as_str(), s.output_path()))
        .collect();
    let mut pending: Vec<&Source> = files
        .iter()
        .map(|(s, _)| s)
        .filter(|s| side.contains_key(&s.rel) && out.contains_key(&s.output_path()))
        .collect();
    pending.sort_by(|a, b| a.rel.cmp(&b.rel));
    let deps_of = |rel: &str| -> Vec<&str> {
        edges
            .iter()
            .filter(|(_, b)| b == rel)
            .map(|(a, _)| a.as_str())
            .collect()
    };
    let mut ordered: Vec<TestFile> = Vec::new();
    let mut placed: HashSet<String> = HashSet::new();
    while !pending.is_empty() {
        // The first whose test-side imports are placed; on a cycle, the
        // first.
        let next = pending
            .iter()
            .position(|s| {
                deps_of(&s.rel)
                    .iter()
                    .all(|d| !side.contains_key(*d) || placed.contains(*d))
            })
            .unwrap_or(0);
        let s = pending.remove(next);
        placed.insert(s.rel.clone());
        ordered.push(TestFile {
            path: s.output_path(),
            rel: s.rel.clone(),
            role: side[&s.rel],
            deps: deps_of(&s.rel)
                .iter()
                .filter_map(|d| output_of.get(d).cloned())
                .collect(),
        });
    }
    ordered
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
    /// Files of embedded libraries the package's XML loads, by library
    /// index (paths relative to the library).
    lib_refs: BTreeMap<usize, BTreeSet<String>>,
}

/// Resolves the XML references of the sources. A reference into
/// `<libs-dir>/<install-path>/` (an `embeds.xml` of a migrated addon) names
/// a file of that embedded library; it is rewritten to the library's
/// in-game path.
fn resolve_xml(
    session: &mut Session,
    files: &[(Source, FileId)],
    libs: &[crate::embed::LuaLib],
    addon: &str,
    packager_debug: bool,
    variants: (&BTreeSet<String>, &[String]),
) -> XmlInfo {
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
        // References a dev build cannot resolve inside the packager's
        // `@debug@` blocks are left out (a member's files, say).
        let debug_blocks = if packager_debug {
            wowlua_xml::debug_blocks(text)
        } else {
            Vec::new()
        };
        let mut dropped: Vec<wowlua_xml::XmlRef> = Vec::new();
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
            let library = if target.is_none() {
                let lower = joined.to_ascii_lowercase();
                libs.iter().enumerate().find_map(|(i, lib)| {
                    let rest =
                        lower.strip_prefix(&format!("{}/", lib.install.to_ascii_lowercase()))?;
                    lib.index.get(rest).map(|rel| (i, rel.clone()))
                })
            } else {
                None
            };
            if let Some((i, rel)) = library {
                let conditions = reference.file[reference.path().len()..].to_string();
                replacements.push((
                    reference,
                    format!(
                        "Interface\\AddOns\\{addon}\\{}{conditions}",
                        libs[i].output(&rel).replace('/', "\\")
                    ),
                ));
                info.lib_refs.entry(i).or_default().insert(rel);
                continue;
            }
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
                None if debug_blocks
                    .iter()
                    .any(|b| b.contains(&reference.span.start)) =>
                {
                    diagnostics.push(
                        Diagnostic::warning(
                            "W5003",
                            format!(
                                "XML reference `{}` in a `@debug@` block does not resolve to a source file; left out of this build",
                                reference.path()
                            ),
                        )
                        .with_label(Span::new(*file, reference.span.clone()), "not found")
                        .with_note("release builds comment the block out, like the packager"),
                    );
                    dropped.push(wowlua_xml::XmlRef {
                        span: wowlua_xml::element_range(text, reference),
                        ..reference.clone()
                    });
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
        // Files only some flavors' variants load (packager flavor blocks).
        if variants.0.contains(&source.rel) {
            for flavor in variants.1 {
                let variant = crate::keywords::apply_flavor(text, flavor, true);
                for reference in wowlua_xml::references(&variant).unwrap_or_default() {
                    let joined = normalize(&if dir.is_empty() {
                        reference.normalized()
                    } else {
                        format!("{dir}/{}", reference.normalized())
                    });
                    let Some(target) = by_src_rel.get(&joined.to_ascii_lowercase()) else {
                        continue;
                    };
                    let list = info.edges.entry(source.rel.clone()).or_default();
                    if !list.contains(&target.rel) {
                        list.push(target.rel.clone());
                        info.loaded_by_xml.insert(target.rel.clone());
                    }
                }
            }
        }
        replacements.extend(dropped.iter().map(|r| (r, String::new())));
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
    /// An entry file of an embedded library: loads before the package's
    /// own files unless `build.load-order` names it.
    dependency: bool,
}

/// Orders TOC entries: `build.load-order` entries first, in their order (an
/// entry naming an item exactly matches it, even when it contains `[…]`;
/// other entries are globs expanding in path order), then everything else by
/// path (spec §5.4; the load-order analysis of milestone M2 refines this).
/// Applies dependency edges between sources (imports; spec §5.4) to the TOC
/// order, keeping the current order wherever the edges allow. A file loaded
/// through XML is represented by the TOC-listed XML file that loads it.
/// When the flavors' edges together form a cycle although each flavor's
/// own edges do not, the flavors need different orders: returns them (by
/// `TocItem::rel`) and leaves `entries` unchanged.
fn order_by_dependencies(
    session: &mut Session,
    entries: &mut Vec<TocItem>,
    edges: &[(String, String)],
    reasons: &HashMap<(String, String), (wowlua_span::Span, String)>,
    flavor_edges: &[FlavorEdges],
    xml: &XmlInfo,
    load_order: &[String],
) -> Option<HashMap<String, Vec<String>>> {
    if edges.is_empty() {
        return None;
    }
    let index: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.rel.as_str(), i))
        .collect();
    // A file may be loaded by several XML files (one per flavor, say).
    let mut parents: HashMap<&str, Vec<&str>> = HashMap::new();
    for (xml_file, targets) in &xml.edges {
        for t in targets {
            let list = parents.entry(t.as_str()).or_default();
            if !list.contains(&xml_file.as_str()) {
                list.push(xml_file.as_str());
            }
        }
    }
    // The TOC entries that load `rel`, directly or through XML.
    let find_items = |rel: &str| -> Vec<usize> {
        let mut found = Vec::new();
        let mut queue = vec![rel];
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(current) = queue.pop() {
            if !seen.insert(current) || seen.len() > 256 {
                continue;
            }
            if let Some(&i) = index.get(current) {
                if !found.contains(&i) {
                    found.push(i);
                }
                continue;
            }
            if let Some(ps) = parents.get(current) {
                queue.extend(ps.iter().copied());
            }
        }
        found.sort_unstable();
        found
    };
    let mut items: HashMap<&str, Vec<usize>> = HashMap::new();
    for (a, b) in edges
        .iter()
        .chain(flavor_edges.iter().flat_map(|f| f.1.iter()))
    {
        for rel in [a.as_str(), b.as_str()] {
            if !items.contains_key(rel) {
                items.insert(rel, find_items(rel));
            }
        }
    }
    let items_of = |rel: &str| -> &[usize] { items.get(rel).map_or(&[], Vec::as_slice) };
    // An edge (a loads before b) between entries: from the first entry
    // loading `a` to each entry loading `b`.
    let item_edges = |edges: &[(String, String)]| -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut known = HashSet::new();
        for (a, b) in edges {
            let Some(&x) = items_of(a).first() else {
                continue;
            };
            for &y in items_of(b) {
                if x != y && known.insert((x, y)) {
                    out.push((x, y));
                }
            }
        }
        out
    };
    let union = item_edges(edges);
    let pos = |rel: &str| load_order.iter().position(|l| l == rel);
    // Whether an entry loads for `flavor` (`None`: any).
    let active = |i: usize, flavor: Option<&str>| match (flavor, &entries[i].flavors) {
        (Some(f), Some(fs)) => fs.iter().any(|x| x == f),
        _ => true,
    };
    let mut flavors: Vec<Option<&str>> = entries
        .iter()
        .filter_map(|e| e.flavors.as_ref())
        .flatten()
        .map(|f| Some(f.as_str()))
        .collect();
    flavors.sort();
    flavors.dedup();
    if flavors.is_empty() {
        flavors.push(None);
    }
    let mut reported = HashSet::new();
    for (a, b) in edges {
        let (xs, ys) = (items_of(a), items_of(b));
        if xs.is_empty() || ys.is_empty() {
            continue;
        }
        // A `build.load-order` that lists b's first entry before a's (for
        // a flavor loading both) contradicts it.
        for flavor in &flavors {
            let first = |items: &[usize]| {
                items
                    .iter()
                    .copied()
                    .filter(|&i| active(i, *flavor))
                    .filter_map(|i| Some((pos(&entries[i].rel)?, i)))
                    .min()
            };
            let (Some((pa, x)), Some((pb, y))) = (first(xs), first(ys)) else {
                continue;
            };
            if pb < pa && x != y && reported.insert((x, y)) {
                let mut d = Diagnostic::error(
                    "E1035",
                    format!(
                        "`build.load-order` lists `{}` before `{}`, but it depends on it",
                        entries[y].rel, entries[x].rel
                    ),
                );
                if let Some((span, text)) = reasons.get(&(a.clone(), b.clone())) {
                    d = d.with_label(*span, text.clone());
                }
                session.push(d);
            }
        }
    }
    let rank: Vec<usize> = (0..entries.len()).collect();
    let report_cycle = |session: &mut Session, entries: &[TocItem], cycle: &[usize]| {
        let names: Vec<&str> = cycle.iter().map(|&i| entries[i].rel.as_str()).collect();
        let mut shown = names.join(" → ");
        if let Some(first) = names.first() {
            shown.push_str(&format!(" → {first}"));
        }
        session.push(
            Diagnostic::error(
                "E1014",
                format!("the TOC entries cannot be ordered by their dependencies: {shown}"),
            )
            .with_note("files loaded by one XML file load together")
            .with_help("move the use into a function that runs later, or split a module"),
        );
    };
    match wowlua_sema::order::stable_topo(entries.len(), &rank, &union) {
        Ok(order) => {
            let mut old: Vec<Option<TocItem>> =
                std::mem::take(entries).into_iter().map(Some).collect();
            *entries = order.into_iter().filter_map(|i| old[i].take()).collect();
            None
        }
        Err(cycle) => {
            let mut orders = HashMap::new();
            for (flavor, own) in flavor_edges {
                let Some(flavor) = flavor else {
                    report_cycle(session, entries, &cycle);
                    return None;
                };
                match wowlua_sema::order::stable_topo(entries.len(), &rank, &item_edges(own)) {
                    Ok(order) => {
                        orders.insert(
                            flavor.clone(),
                            order.into_iter().map(|i| entries[i].rel.clone()).collect(),
                        );
                    }
                    Err(own_cycle) => {
                        report_cycle(session, entries, &own_cycle);
                        return None;
                    }
                }
            }
            Some(orders)
        }
    }
}

/// The load position of each source listed in `build.load-order`: listed
/// entries in order (a glob's matches by path, TOC variables expanded for
/// `flavors`), each followed by the sources its XML loads, depth first.
/// Unlisted sources have none.
/// Adds and parses an embedded WoW API file. Without `keep_tokens` its tokens,
/// trivia and line starts are freed at once: only rename-private
/// (`private_names::plan`) reads them, for the API's method names.
fn parse_api(
    db: &mut wowlua_span::SourceDb,
    f: wowlua_api::File,
    keep_tokens: bool,
) -> crate::compile::ParsedDeclaration {
    let file = db.add_static(f.path, f.text);
    let mut parse = wowlua_syntax::parse(f.text.as_bytes(), file);
    if !keep_tokens {
        parse.lexed.tokens = Vec::new();
        parse.lexed.trivia = Vec::new();
        parse.lexed.line_starts = Vec::new();
    }
    crate::compile::ParsedDeclaration {
        source: sources::Source {
            rel: f.path.to_string(),
            src_rel: f.path.to_string(),
            abs: PathBuf::new(),
            kind: SourceKind::Wowlua,
            output: None,
        },
        file,
        parse,
        globals: f.globals,
    }
}

fn listed_positions(
    files: &[(Source, FileId)],
    load_order: &[String],
    xml: &XmlInfo,
    flavors: &[String],
) -> Vec<Option<usize>> {
    let by_lower: HashMap<String, &str> = files
        .iter()
        .map(|(s, _)| (s.rel.to_ascii_lowercase(), s.rel.as_str()))
        .collect();
    let index: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, (s, _))| (s.rel.as_str(), i))
        .collect();
    let mut positions = vec![None; files.len()];
    let mut next = 0;
    fn visit(
        rel: &str,
        index: &HashMap<&str, usize>,
        xml: &XmlInfo,
        positions: &mut [Option<usize>],
        next: &mut usize,
    ) {
        let Some(&i) = index.get(rel) else {
            return;
        };
        if positions[i].is_some() {
            return;
        }
        positions[i] = Some(*next);
        *next += 1;
        for target in xml.edges.get(rel).into_iter().flatten() {
            visit(target, index, xml, positions, next);
        }
    }
    for pattern in load_order {
        let lower = pattern.to_ascii_lowercase();
        let variable = ["[game]", "[family]", "[textlocale]"]
            .iter()
            .any(|v| lower.contains(v));
        let mut matched: Vec<&str> = if variable {
            // `db/[Game]/Database.xml`: this flavor's file (each locale's).
            let locales: Vec<&str> = if lower.contains("[textlocale]") {
                wowlua_manifest::model::LOCALES.to_vec()
            } else {
                vec![""]
            };
            let mut out = Vec::new();
            for f in flavors {
                for l in &locales {
                    let concrete = substitute(pattern, f, l).to_ascii_lowercase();
                    if let Some(rel) = by_lower.get(&concrete)
                        && !out.contains(rel)
                    {
                        out.push(*rel);
                    }
                }
            }
            out
        } else if index.contains_key(pattern.as_str()) {
            vec![pattern.as_str()]
        } else if let Ok(glob) = globset::Glob::new(pattern) {
            let matcher = glob.compile_matcher();
            files
                .iter()
                .map(|(s, _)| s.rel.as_str())
                .filter(|rel| matcher.is_match(rel) && !xml.loaded_by_xml.contains(*rel))
                .collect()
        } else {
            Vec::new()
        };
        matched.sort();
        for rel in matched {
            visit(rel, &index, xml, &mut positions, &mut next);
        }
    }
    positions
}

fn order_by_load_order(entries: &mut Vec<TocItem>, load_order: &[String]) {
    let mut ordered = Vec::with_capacity(entries.len());
    // Library entries load first, unless the load order names them exactly.
    let (mut deps, mut remaining): (Vec<TocItem>, Vec<TocItem>) = std::mem::take(entries)
        .into_iter()
        .partition(|i| i.dependency);
    for pattern in load_order {
        if deps.iter().any(|i| i.rel.eq_ignore_ascii_case(pattern)) {
            let (matched, rest): (Vec<_>, Vec<_>) = deps
                .into_iter()
                .partition(|i| i.rel.eq_ignore_ascii_case(pattern));
            ordered.extend(matched);
            deps = rest;
            continue;
        }
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
    deps.extend(ordered);
    *entries = deps;
}

/// Whether `rel` lies in a folder a variable file names (`db/[Game]/…`,
/// `[Family]/…`) other than this flavor's: the client loads only its own
/// game's folder, so the others (and what they include) do not load.
fn in_other_variant(
    rel: &str,
    flavor: &str,
    entries: &[wowlua_manifest::model::VariableFile],
) -> bool {
    let lower = rel.to_ascii_lowercase();
    for entry in entries {
        let path = entry.path.replace('\\', "/");
        let segments: Vec<&str> = path.split('/').collect();
        // The folders up to the last one named by a variable.
        let Some(last) = segments[..segments.len().saturating_sub(1)]
            .iter()
            .rposition(|s| s.contains('['))
        else {
            continue;
        };
        let prefix = segments[..=last].join("/");
        if prefix.to_ascii_lowercase().contains("[textlocale]") {
            continue;
        }
        // The folder holding the variants (`db/`), and this flavor's.
        let parent = segments[..last].join("/").to_ascii_lowercase();
        let parent = if parent.is_empty() {
            parent
        } else {
            format!("{parent}/")
        };
        let own = format!("{}/", substitute(&prefix, flavor, "").to_ascii_lowercase());
        if lower.starts_with(&parent) && !lower.starts_with(&own) {
            // Only folders below the variable's position: `db/X/…`.
            if lower[parent.len()..].contains('/') {
                return true;
            }
        }
    }
    false
}

/// Every file a variable file expands to for some known flavor (and
/// locale), whether or not this build targets that flavor.
fn variant_files(
    entries: &[wowlua_manifest::model::VariableFile],
    files: &[(Source, FileId)],
) -> BTreeSet<String> {
    const ALL: &[&str] = &[
        "standard", "camelot", "mists", "cata", "wrath", "tbc", "vanilla",
    ];
    let by_lower: HashMap<String, &str> = files
        .iter()
        .map(|(s, _)| (s.rel.to_ascii_lowercase(), s.rel.as_str()))
        .collect();
    let mut out = BTreeSet::new();
    for entry in entries {
        let locales: Vec<String> = if entry.path.to_ascii_lowercase().contains("[textlocale]") {
            wowlua_manifest::model::LOCALES
                .iter()
                .map(|l| l.to_string())
                .collect()
        } else {
            vec![String::new()]
        };
        for flavor in ALL {
            for locale in &locales {
                let concrete = substitute(&entry.path, flavor, locale).to_ascii_lowercase();
                if let Some(rel) = by_lower.get(&concrete) {
                    out.insert(rel.to_string());
                }
            }
        }
    }
    out
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
pub fn substitute(path: &str, flavor: &str, locale: &str) -> String {
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
                dependency: false,
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
                            locales: if has_locale {
                                Some(vec![locale.clone()])
                            } else {
                                line_locales.clone()
                            },
                            dependency: false,
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
    // A converted packager project is versioned by its tags, like the
    // packager's `@project-version@` (decision T157).
    if project.manifest.manifest.compat.packager_keywords {
        return Some(
            crate::keywords::head_tag(&project.manifest.root)
                .unwrap_or_else(|| vcs.tags[0].clone()),
        );
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

#[allow(clippy::too_many_arguments)]
fn directives(
    project: &Project,
    flavors: &[String],
    profile: &str,
    version: &str,
    build_id: &str,
    toc_data: TocData,
    nolib_optional: &[String],
    dev_saved: Option<String>,
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
    // A converted project's TOC keeps its own version line (decision T153).
    match &manifest.manifest.compat.toc_version {
        Some(v) => {
            let mut groups: indexmap::IndexMap<String, Vec<String>> = indexmap::IndexMap::new();
            for f in flavors {
                if let Some(value) = v.of(f).filter(|value| !value.is_empty()) {
                    groups.entry(value.to_string()).or_default().push(f.clone());
                }
            }
            for (value, group) in groups {
                out.push(Directive {
                    key: "Version".into(),
                    value,
                    flavors: (group.len() < flavors.len()).then_some(group),
                });
            }
        }
        None => out.push(Directive::new("Version", version)),
    }
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

    // A nolib package's libraries, as standalone addons (Publishing
    // §11.2.2), join the optional dependencies.
    let listed: Vec<String> = manifest
        .manifest
        .addon_dependencies
        .iter()
        .flat_map(|d| d.optional.iter())
        .filter(|d| d.flavors().is_empty())
        .map(|d| d.name().to_string())
        .collect();
    let nolib_extra: Vec<&str> = nolib_optional
        .iter()
        .filter(|n| !listed.contains(n))
        .map(String::as_str)
        .collect();
    if !nolib_extra.is_empty() && manifest.manifest.addon_dependencies.is_none() {
        out.push(Directive::new("OptionalDeps", nolib_extra.join(", ")));
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
            if key == "OptionalDeps" && !nolib_extra.is_empty() {
                match runs.iter_mut().find(|(r, _)| r.is_none()) {
                    Some((_, names)) => names.extend(nolib_extra.iter().copied()),
                    None => runs.push((None, nolib_extra.clone())),
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

    // SavedVariables, per flavor when overrides differ (decision T128).
    let grouped = |out: &mut Vec<Directive>, key: &str, per_flavor: Vec<(String, String)>| {
        let mut groups: indexmap::IndexMap<String, Vec<String>> = indexmap::IndexMap::new();
        for (flavor, value) in per_flavor {
            if !value.is_empty() {
                groups.entry(value).or_default().push(flavor);
            }
        }
        for (value, group) in groups {
            out.push(Directive {
                key: key.to_string(),
                value,
                flavors: (group.len() < flavors.len()).then_some(group),
            });
        }
    };
    let override_of = |f: &str| overrides.iter().find(|(id, _)| id == f).map(|(_, o)| o);
    let saved: Vec<(String, String)> = flavors
        .iter()
        .map(|f| {
            let list = override_of(f)
                .and_then(|o| o.saved_variables.clone())
                .unwrap_or_else(|| addon.saved_variables.clone());
            let all: Vec<String> = list.into_iter().chain(dev_saved.clone()).collect();
            (f.clone(), all.join(", "))
        })
        .collect();
    grouped(&mut out, "SavedVariables", saved);
    let per_character: Vec<(String, String)> = flavors
        .iter()
        .map(|f| {
            let list = override_of(f)
                .and_then(|o| o.saved_variables_per_character.clone())
                .unwrap_or_else(|| addon.saved_variables_per_character.clone());
            (f.clone(), list.join(", "))
        })
        .collect();
    grouped(&mut out, "SavedVariablesPerCharacter", per_character);
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
    // Publish IDs: per flavor, the first target of each site that serves
    // it (targets restricted with `flavors` give per-flavor IDs, T129).
    if let Some(publish) = &manifest.manifest.publish {
        use wowlua_manifest::model::{ProjectId, TargetType};
        for (site, key) in [
            (TargetType::Curseforge, "X-Curse-Project-ID"),
            (TargetType::Wago, "X-Wago-ID"),
            (TargetType::Wowinterface, "X-WoWI-ID"),
        ] {
            let per_flavor: Vec<(String, String)> = flavors
                .iter()
                .filter_map(|f| {
                    let target = publish.targets.values().find(|t| {
                        t.target_type == site
                            && t.enabled
                            && t.project_id.is_some()
                            && t.flavors.as_ref().is_none_or(|list| {
                                list.iter().any(|x| {
                                    wowlua_flavors::table()
                                        .resolve(x)
                                        .is_some_and(|fl| fl.id == *f)
                                })
                            })
                    })?;
                    let value = match target.project_id.as_ref()? {
                        ProjectId::Number(n) => n.to_string(),
                        ProjectId::String(s) => s.clone(),
                    };
                    Some((f.clone(), value))
                })
                .collect();
            grouped(&mut out, key, per_flavor);
        }
    }
    out
}

/// A text file as the packager ships it: LF line endings and repository
/// keywords.
fn packaged_text(bytes: &[u8], info: &crate::keywords::KeywordInfo, rel: &str) -> Vec<u8> {
    let normalized = crate::keywords::lf(bytes);
    match std::str::from_utf8(&normalized) {
        Ok(text) => {
            crate::keywords::apply_text(text, info, Path::new(rel), Path::new(rel)).into_bytes()
        }
        Err(_) => normalized,
    }
}

/// The manifest JSON of a package folder.
fn manifest_json(dir: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(dir.join(wowlua_manifest::MANIFEST_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The CurseForge project ID of a manifest's publish targets.
fn curseforge_id(manifest: &serde_json::Value) -> Option<String> {
    manifest
        .pointer("/publish/targets")?
        .as_object()?
        .values()
        .find(|t| t["type"] == "curseforge")
        .and_then(|t| match &t["project-id"] {
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::String(s) => Some(s.clone()),
            _ => None,
        })
}

/// `license-output` of the package in `dir`: its file, else the license of
/// its CurseForge project, like the packager downloads it.
fn license_bytes(dir: &Path, license: &str) -> Option<(Vec<u8>, bool)> {
    if let Ok(bytes) = std::fs::read(dir.join(license)) {
        return Some((bytes, false));
    }
    let id = curseforge_id(&manifest_json(dir)?)?;
    crate::packaging::downloaded_license(&id).map(|b| (b, true))
}

/// The files the packager adds to the AddOn its package directory becomes
/// (the root addon, or the member named `compat.package-as` of a workspace
/// without one): the manual changelog or the one it generates, and the
/// `license-output` license when the repository has none.
fn package_files(
    project: &Project,
    root: &Path,
    info: &crate::keywords::KeywordInfo,
    addon_name: &str,
    out: &mut BTreeMap<String, Content>,
    raw_paths: &mut BTreeSet<String>,
) {
    let same = |a: &Path, b: &Path| std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok();
    let (repo, repo_manifest) = match &project.workspace_root {
        Some(ws) if !same(ws, root) => {
            let Some(m) = manifest_json(ws) else { return };
            let package_root = m.get("package").is_none()
                && m.pointer("/compat/package-as").and_then(|v| v.as_str()) == Some(addon_name);
            if !package_root {
                return;
            }
            (ws.clone(), m)
        }
        _ => match manifest_json(root) {
            Some(m) => (root.to_path_buf(), m),
            None => return,
        },
    };
    match repo_manifest
        .pointer("/publish/changelog")
        .and_then(|v| v.as_str())
        // A manual changelog the repository lacks: the packager generates one.
        .filter(|file| repo.join(file).is_file())
    {
        Some(file) => {
            if !out.contains_key(file)
                && let Ok(bytes) = std::fs::read(repo.join(file))
            {
                out.insert(
                    file.to_string(),
                    Content::Bytes(packaged_text(&bytes, info, file)),
                );
            }
        }
        None => {
            let title = repo_manifest
                .pointer("/compat/changelog-title")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| crate::packaging::plain_title(&project.manifest.title()));
            // It replaces a `CHANGELOG.md` of the repository, like the
            // packager's.
            if let Some(text) = crate::packaging::generated_changelog(&repo, &title, info) {
                // A `changelog.md` of the repository: one file on the game's
                // case-insensitive file systems, where the packager's
                // generated one replaces it.
                out.retain(|path, _| {
                    path == "CHANGELOG.md" || !path.eq_ignore_ascii_case("CHANGELOG.md")
                });
                out.insert("CHANGELOG.md".into(), Content::Bytes(text.into_bytes()));
            }
        }
    }
    if let Some(license) = repo_manifest
        .pointer("/compat/license-output")
        .and_then(|v| v.as_str())
        && !out.contains_key(license)
        && !repo.join(license).is_file()
        && let Some((bytes, _)) = license_bytes(&repo, license)
    {
        // Saved after the packager's processing: as downloaded.
        raw_paths.insert(license.to_string());
        out.insert(license.to_string(), Content::Bytes(bytes));
    }
}

/// The workspace root of a workspace without a root addon whose package
/// folder becomes this AddOn (`compat.package-as`), and its manifest.
fn suite_package_root(
    project: &Project,
    root: &Path,
    addon_name: &str,
) -> Option<(PathBuf, serde_json::Value)> {
    let ws = project.workspace_root.as_ref()?;
    if std::fs::canonicalize(ws).ok() == std::fs::canonicalize(root).ok() {
        return None;
    }
    let m = manifest_json(ws)?;
    (m.get("package").is_none()
        && m.pointer("/compat/package-as").and_then(|v| v.as_str()) == Some(addon_name))
    .then(|| (ws.clone(), m))
}

/// Ships the files of `dir` the packager copies and wowlua has not placed
/// yet: all but dotfiles, `skip` folders, `members`, wowlua's own files and
/// the TOCs it generates, excluded and (release) untracked files; text
/// files get the packager's processing unless `plain-copy` says otherwise.
#[allow(clippy::too_many_arguments)]
fn ship_project_files(
    dir: &Path,
    members: &[PathBuf],
    skip: &[PathBuf],
    exclude: &globset::GlobSet,
    plain: &[String],
    tracked: Option<&std::collections::HashSet<PathBuf>>,
    info: &crate::keywords::KeywordInfo,
    out: &mut BTreeMap<String, Content>,
    shipped: &mut BTreeSet<String>,
) {
    // Output paths compare case-insensitively, like the game's file system:
    // what is already there wins.
    let mut taken: std::collections::HashSet<String> =
        out.keys().map(|k| k.to_ascii_lowercase()).collect();
    for entry in walkdir::WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !(e.file_name().to_string_lossy().starts_with('.')
                    || skip.iter().any(|d| d == e.path())
                    || members.iter().any(|m| m == e.path()))
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
        let lower = rel.to_ascii_lowercase();
        // wowlua's files, and the TOCs it generates in their place.
        let own = !rel.contains('/')
            && (lower == "wowlua.json" || lower == "wowlua.lock" || lower.ends_with(".toc"));
        let untracked = tracked.is_some_and(|t| !t.is_empty() && !t.contains(entry.path()));
        if own || untracked || taken.contains(&lower) || exclude.is_match(&rel) {
            continue;
        }
        taken.insert(lower.clone());
        let content = match std::fs::read(entry.path()) {
            Ok(bytes)
                if !crate::keywords::plain_copy(plain, &[&rel])
                    && crate::keywords::processed_extension(&rel) =>
            {
                let normalized = crate::keywords::lf(&bytes);
                match std::str::from_utf8(&normalized) {
                    Ok(text) if lower.ends_with(".lua") || lower.ends_with(".xml") => {
                        let path = Path::new(&rel);
                        Content::Bytes(
                            crate::keywords::apply(text, info, path, path, lower.ends_with(".xml"))
                                .into_bytes(),
                        )
                    }
                    _ => Content::Bytes(packaged_text(&bytes, info, &rel)),
                }
            }
            _ => Content::Copy(entry.path().to_path_buf()),
        };
        shipped.insert(rel.clone());
        out.insert(rel, content);
    }
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
        TocError::IncompatibleOrders => Diagnostic::error(
            "E1036",
            "the flavors need incompatible load orders, but `build.toc-layout` is `single`",
        )
        .with_path(manifest)
        .with_help("use `\"toc-layout\": \"auto\"` to get per-flavor TOC files"),
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

/// The global table Bindings.xml calls through: the addon's name as a Lua
/// name (the `wowlua:bindings` module derives the same one).
fn bindings_global(addon: &str) -> String {
    let mut name: String = addon
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'_' {
                b as char
            } else {
                '_'
            }
        })
        .collect();
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name + "_Bindings"
}

fn xml_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

/// The generated Bindings.xml: headerless bindings first, then each
/// header's bindings together (the attribute on the first), in order of
/// first use; `defaults`: emit the default keys.
fn bindings_xml(
    bindings: &[&wowlua_sema::stdlib::BindingDecl],
    addon: &str,
    defaults: bool,
) -> String {
    let mut groups: Vec<(Option<&str>, Vec<&wowlua_sema::stdlib::BindingDecl>)> =
        vec![(None, Vec::new())];
    for b in bindings {
        let header = b.header.as_deref();
        match groups.iter_mut().find(|(h, _)| *h == header) {
            Some((_, list)) => list.push(b),
            None => groups.push((header, vec![b])),
        }
    }
    let dispatch = bindings_global(addon);
    let mut xml = String::from("<Bindings>\n");
    for (header, list) in groups {
        for (i, b) in list.iter().enumerate() {
            xml.push_str(&format!("  <Binding name=\"{}\"", b.name));
            if let Some(h) = header
                && i == 0
            {
                xml.push_str(&format!(" header=\"{}\"", xml_attribute(h)));
            }
            if b.run_on_up {
                xml.push_str(" runOnUp=\"true\"");
            }
            if defaults && let Some((key, _)) = &b.default {
                xml.push_str(&format!(" default=\"{}\"", xml_attribute(key)));
            }
            xml.push_str(&format!(
                ">{dispatch}[\"{}\"](keystate)</Binding>\n",
                b.name
            ));
        }
    }
    xml.push_str("</Bindings>\n");
    xml
}

/// The binding names of the hand-written keybinding files in `root`.
fn xml_binding_names(root: &Path) -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return HashSet::new();
    };
    let mut names = HashSet::new();
    for e in entries.filter_map(Result::ok) {
        if !is_bindings_file(&e.file_name().to_string_lossy()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        for tag in text.split("<Binding").skip(1) {
            let Some(rest) = tag.split_once("name=").map(|(_, r)| r) else {
                continue;
            };
            let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                continue;
            };
            if let Some(name) = rest[1..].split(quote).next() {
                names.insert(name.to_string());
            }
        }
    }
    names
}

fn licenses_text(dev_fonts: bool, libs: &[(String, String)]) -> String {
    let mut embedded = String::new();
    for (title, text) in libs {
        embedded.push_str(&format!("== {title} ==\n\n{text}\n\n"));
    }
    if dev_fonts {
        embedded.push_str(
            "The dev window's fonts in wowlua/fonts/ (dev builds only):\n\
             - Marcellus SC, Copyright (c) 2012 Brian J. Bonislawsky DBA Astigmatic (AOETI),\n  \
               SIL Open Font License 1.1 (wowlua/fonts/MarcellusSC-OFL.txt)\n\
             - Figtree, Copyright 2022 The Figtree Project Authors,\n  \
               SIL Open Font License 1.1 (wowlua/fonts/Figtree-OFL.txt)\n",
        );
    }
    if embedded.is_empty() {
        embedded.push_str("This build embeds no third-party components.\n");
    }
    format!(
        "Licenses of components embedded in this AddOn.\n\
         Generated by wowlua {}.\n\n{embedded}",
        crate::COMPILER_VERSION
    )
}

/// Output validation (spec §13.8).
#[allow(clippy::too_many_arguments)]
fn validate_output(
    session: &mut Session,
    out: &BTreeMap<String, Content>,
    tocs: &[TocFile],
    xml: &XmlInfo,
    output_paths: &HashMap<String, String>,
    variable_outputs: &BTreeSet<String>,
    roots: &BTreeSet<String>,
    addon: &str,
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
    // The package's own XML loads only files of the output (W5004): a file
    // `build.flavor-files` leaves out of a build may still be referenced.
    let lower_paths: HashSet<String> = out.keys().map(|p| p.to_ascii_lowercase()).collect();
    let prefix = format!("interface/addons/{}/", addon.to_ascii_lowercase());
    let mut missing: BTreeSet<(String, String)> = BTreeSet::new();
    for (path, content) in out {
        let lower = path.to_ascii_lowercase();
        if !lower.ends_with(".xml")
            || !(lower.starts_with("src/") || lower.starts_with("wowlua/flavors/"))
        {
            continue;
        }
        let bytes = match content {
            Content::Bytes(b) => b.clone(),
            Content::Copy(p) => std::fs::read(p).unwrap_or_default(),
        };
        let Ok(refs) = wowlua_xml::references(&String::from_utf8_lossy(&bytes)) else {
            continue;
        };
        let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for r in refs {
            let target = r.normalized().to_ascii_lowercase();
            let target = if let Some(rest) = target.strip_prefix(&prefix) {
                normalize(rest)
            } else if target.starts_with("interface/") {
                continue; // another AddOn's file
            } else {
                normalize(&format!("{dir}/{target}")).to_ascii_lowercase()
            };
            if !lower_paths.contains(&target) {
                // Per-flavor copies report the shared path once.
                let shown = path
                    .strip_prefix("wowlua/flavors/")
                    .and_then(|r| r.split_once('/'))
                    .map_or(path.as_str(), |(_, r)| r);
                missing.insert((shown.to_string(), r.path()));
            }
        }
    }
    for (path, reference) in missing {
        session.push(
            Diagnostic::warning(
                "W5004",
                format!("`{path}` loads `{reference}`, which is not part of this build's output"),
            )
            .with_help("check `build.exclude` and `build.flavor-files`"),
        );
    }
    for path in out.keys() {
        let lower = path.to_ascii_lowercase();
        let is_code = lower.ends_with(".lua") || lower.ends_with(".xml");
        // Test files (`wowlua test`) load outside the TOC.
        let exempt =
            path == "Bindings.xml" || !path.starts_with("src/") || lower.ends_with(".test.lua");
        if is_code && !exempt && !reachable.contains(path) {
            session.push(Diagnostic::error(
                "E5002",
                format!("`{path}` is part of the output but is not loaded by any TOC or XML file"),
            ));
        }
    }
}

/// What `wowlua build --stats` reports (spec optimization §15.3.2).
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub files: usize,
    pub bytes: u64,
    pub lua_files: usize,
    pub lua_bytes: u64,
    /// How long the embedded Lua 5.1 takes to compile the Lua files: the
    /// parse cost the client pays at load, without its per-file overhead.
    pub compile: std::time::Duration,
}

/// The statistics of a build's output.
pub fn stats(build: &BuildOutput) -> Stats {
    let lua = mlua::Lua::new();
    let mut stats = Stats::default();
    for (path, content) in &build.files {
        let bytes = content_bytes(content);
        stats.files += 1;
        stats.bytes += bytes.len() as u64;
        if path.to_ascii_lowercase().ends_with(".lua") {
            stats.lua_files += 1;
            stats.lua_bytes += bytes.len() as u64;
            let code = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            let started = std::time::Instant::now();
            let _ = lua.load(code).into_function();
            stats.compile += started.elapsed();
        }
    }
    stats
}

/// What bundling needs to know about the build.
struct BundleInput<'a> {
    addon: &'a str,
    build_id: &'a str,
    flavors: &'a [String],
    libs: &'a [crate::embed::LuaLib],
    files: &'a [(Source, wowlua_span::FileId)],
    raw: &'a HashMap<String, Arc<[u8]>>,
    source_maps: bool,
    /// Compiled code known to compile, by output path.
    known: &'a HashMap<String, std::sync::Arc<crate::compile::Compiled>>,
}

fn content_bytes(content: &Content) -> Vec<u8> {
    match content {
        Content::Bytes(b) => b.clone(),
        Content::Copy(p) => std::fs::read(p).unwrap_or_default(),
    }
}

/// The key of an output file's source map in `maps`.
fn map_key(path: &str) -> String {
    match path
        .strip_prefix("wowlua/flavors/")
        .and_then(|rest| rest.split_once('/'))
    {
        Some((flavor, rest)) => format!("{flavor}/{rest}.map"),
        None => format!("{path}.map"),
    }
}

/// Calls of `debugstack` or `debuglocals` in Lua text: code that may look at
/// its own file name (spec optimization §15.3.1).
fn inspects_own_file(text: &[u8]) -> bool {
    let file = wowlua_span::SourceDb::new().add("lexed", String::new());
    let lexed = wowlua_syntax::lexer::lex(text, file);
    lexed.tokens.iter().any(|t| {
        t.kind == wowlua_syntax::token::TokenKind::Name
            && matches!(
                text.get(t.start as usize..t.end as usize),
                Some(b"debugstack" | b"debuglocals")
            )
    })
}

/// Bundles the TOC's Lua files (spec optimization §15.3): returns the new
/// TOC entries and per-flavor orders; the bundles replace their files in
/// `out` and `maps`.
#[allow(clippy::type_complexity)]
fn bundle_output(
    session: &mut Session,
    input: &BundleInput,
    items: Vec<TocItem>,
    orders: Option<HashMap<String, Vec<usize>>>,
    out: &mut BTreeMap<String, Content>,
    maps: &mut BTreeMap<String, Arc<EncodedMap>>,
) -> Option<(Vec<TocItem>, Option<HashMap<String, Vec<usize>>>)> {
    let lib_of = |path: &str| {
        input
            .libs
            .iter()
            .find(|l| path.starts_with(&format!("{}/", l.install)))
    };
    // Libraries that stay separate: `"bundle": false`, or code that looks at
    // its own file name (decision T163).
    let mut separate: BTreeSet<String> = BTreeSet::new();
    for lib in input.libs {
        if !lib.bundle {
            separate.insert(lib.name.clone());
            continue;
        }
        let inspects = lib.index.values().any(|rel| {
            rel.to_ascii_lowercase().ends_with(".lua")
                && std::fs::read(lib.dir.join(rel)).is_ok_and(|t| inspects_own_file(&t))
        });
        if inspects {
            separate.insert(lib.name.clone());
            session.push(Diagnostic::note(format!(
                "the library `{}` is not bundled: it calls `debugstack` or `debuglocals`, which see the file name",
                lib.name
            )));
        }
    }
    let bundled_lib = |path: &str| lib_of(path).is_none_or(|l| !separate.contains(&l.name));
    // An XML file of the TOC that only loads Lua files is flattened into them
    // (decision T167), unless another XML file loads any of them.
    let referrers = xml_referrers(out, input.addon);
    let mut expanded: Vec<TocItem> = Vec::new();
    let mut origin: Vec<Vec<usize>> = Vec::new();
    let mut flattened_xml: Vec<String> = Vec::new();
    for item in items {
        let mut xmls = Vec::new();
        let flat = (item.path.to_ascii_lowercase().ends_with(".xml") && bundled_lib(&item.path))
            .then(|| flatten_xml(out, &item.path, &mut xmls, 0))
            .flatten()
            .filter(|paths| {
                let inside: BTreeSet<&String> = xmls.iter().collect();
                paths.iter().chain(&xmls).all(|p| {
                    referrers
                        .get(&p.to_ascii_lowercase())
                        .is_none_or(|from| from.iter().all(|x| inside.contains(x)))
                }) && paths.iter().all(|p| bundled_lib(p))
            });
        let start = expanded.len();
        match flat {
            Some(paths) => {
                flattened_xml.extend(xmls);
                for path in paths {
                    expanded.push(TocItem {
                        rel: path.clone(),
                        path,
                        ..item.clone()
                    });
                }
            }
            None => expanded.push(item),
        }
        origin.push((start..expanded.len()).collect());
    }
    let orders = orders.map(|o| {
        o.into_iter()
            .map(|(f, idx)| (f, idx.iter().flat_map(|&i| origin[i].clone()).collect()))
            .collect::<HashMap<_, _>>()
    });
    let lua = mlua::Lua::new();
    let compiles = |bytes: &[u8]| {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        crate::luacheck::no_memory_error(lua.load(bytes).into_function()).is_ok()
    };
    let entries: Vec<(bool, Option<Vec<String>>)> = expanded
        .iter()
        .map(|item| {
            let ok = item.path.to_ascii_lowercase().ends_with(".lua")
                && !item.path.starts_with("../")
                && !item.path.contains('[')
                && item.locales.is_none()
                // Locale files load alone, before everything (spec stdlib
                // "locale").
                && !item.path.starts_with(LOCALES_PATH)
                && bundled_lib(&item.path)
                && out.get(&item.path).is_some_and(|c| {
                    // The bytes the compiler compiled, still unchanged.
                    let known = input.known.get(&item.path).is_some_and(|k| {
                        matches!(c, Content::Bytes(b) if b[..] == k.code[..])
                    });
                    debug_assert!(!known || compiles(&content_bytes(c)));
                    known || compiles(&content_bytes(c))
                });
            (ok, item.flavors.clone())
        })
        .collect();
    let (planned, orders) =
        crate::bundle::plan(input.addon, &entries, input.flavors, orders.as_ref());
    // Bundled project code that looks at its own file name (W6001).
    let sources: HashMap<&str, &Source> = input
        .files
        .iter()
        .map(|(s, _)| (s.rel.as_str(), s))
        .collect();
    let mut warned: BTreeSet<String> = BTreeSet::new();
    let mut result: Vec<TocItem> = Vec::new();
    for p in planned {
        match p {
            crate::bundle::Planned::Keep(i) => result.push(expanded[i].clone()),
            crate::bundle::Planned::Bundle {
                path,
                flavors,
                members,
            } => {
                if out.contains_key(&path) {
                    session.push(
                        Diagnostic::error(
                            "E4311",
                            format!("`{path}` is the bundle's path and also a file of the output"),
                        )
                        .with_help("rename the file, or turn bundling off with `bundle: false`"),
                    );
                    return None;
                }
                let mut parts = Vec::new();
                for i in members {
                    let item = &expanded[i];
                    let text = out
                        .remove(&item.path)
                        .map(|c| content_bytes(&c))
                        .unwrap_or_default();
                    // A map that does not decode is as none: its lines map to the file.
                    let own_map = maps.remove(&map_key(&item.path)).filter(|m| m.parses());
                    if let Some(source) = sources.get(item.rel.as_str())
                        && lib_of(&item.path).is_none()
                        && !wowlua_sema::is_compiler_known(&source.src_rel)
                        && input
                            .raw
                            .get(&source.rel)
                            .is_some_and(|t| inspects_own_file(t))
                        && warned.insert(source.rel.clone())
                    {
                        session.push(
                            Diagnostic::warning(
                                "W6001",
                                "this file calls `debugstack` or `debuglocals`, which see the bundle's file name, not this file's",
                            )
                            .with_path(&source.rel)
                            .with_help("`wowlua trace` maps bundle locations back; or turn bundling off with `bundle: false`"),
                        );
                    }
                    let unprefixed = item
                        .path
                        .strip_prefix("wowlua/flavors/")
                        .and_then(|r| r.split_once('/'))
                        .map_or(item.path.as_str(), |(_, r)| r);
                    parts.push(crate::bundle::Member {
                        id: unprefixed
                            .strip_suffix(".lua")
                            .unwrap_or(unprefixed)
                            .to_string(),
                        text,
                        map: own_map,
                        source: item.rel.clone(),
                    });
                }
                let (text, map) =
                    crate::bundle::assemble(&path, input.build_id, parts, input.source_maps);
                // The chunk name appears in the error below. mlua's default is
                // the Rust file and line of this call, so the name is pinned
                // to the one it has always had: edits above leave the message
                // unchanged.
                if let Err(error) = crate::luacheck::no_memory_error(
                    lua.load(&text[..])
                        .set_name("@crates/wowlua-driver/src/build.rs:4316")
                        .into_function(),
                ) {
                    session.push(Diagnostic::plain_error(format!(
                        "internal compiler error: the bundle `{path}` does not compile: {error}"
                    )));
                    return None;
                }
                if let Some(map) = map {
                    maps.insert(format!("{path}.map"), Arc::new(map));
                }
                out.insert(path.clone(), Content::Bytes(text));
                result.push(TocItem {
                    rel: path.clone(),
                    path,
                    flavors,
                    locales: None,
                    dependency: false,
                });
            }
        }
    }
    for xml in flattened_xml {
        out.remove(&xml);
    }
    Some((result, orders))
}

/// The XML files of the output that load each file (by lowercase path).
fn xml_referrers(out: &BTreeMap<String, Content>, addon: &str) -> HashMap<String, Vec<String>> {
    let prefix = format!("interface/addons/{}/", addon.to_ascii_lowercase());
    let mut referrers: HashMap<String, Vec<String>> = HashMap::new();
    for (path, content) in out {
        if !path.to_ascii_lowercase().ends_with(".xml") {
            continue;
        }
        let Ok(refs) = wowlua_xml::references(&String::from_utf8_lossy(&content_bytes(content)))
        else {
            continue;
        };
        let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for r in refs {
            let target = r.normalized().to_ascii_lowercase();
            let target = match target.strip_prefix(&prefix) {
                Some(rest) => normalize(rest),
                None if target.starts_with("interface/") => continue,
                None if dir.is_empty() => normalize(&target),
                None => normalize(&format!("{dir}/{target}")),
            };
            referrers
                .entry(target.to_ascii_lowercase())
                .or_default()
                .push(path.clone());
        }
    }
    referrers
}

/// The Lua files an XML file of the output loads, when it (and every file it
/// includes) loads nothing else; `xmls` collects the XML files involved.
fn flatten_xml(
    out: &BTreeMap<String, Content>,
    path: &str,
    xmls: &mut Vec<String>,
    depth: u32,
) -> Option<Vec<String>> {
    if depth > 16 {
        return None;
    }
    let text = String::from_utf8(content_bytes(out.get(path)?)).ok()?;
    let refs = wowlua_xml::script_only(&text)?;
    xmls.push(path.to_string());
    let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let lower: HashMap<String, &String> = out.keys().map(|k| (k.to_ascii_lowercase(), k)).collect();
    let mut files = Vec::new();
    for r in refs {
        let joined = if dir.is_empty() {
            r.normalized()
        } else {
            format!("{dir}/{}", r.normalized())
        };
        let target = (*lower.get(&normalize(&joined).to_ascii_lowercase())?).clone();
        match r.kind {
            wowlua_xml::RefKind::Script if target.to_ascii_lowercase().ends_with(".lua") => {
                files.push(target)
            }
            wowlua_xml::RefKind::Include => {
                files.extend(flatten_xml(out, &target, xmls, depth + 1)?)
            }
            _ => return None,
        }
    }
    Some(files)
}

/// Writes the output tree to a temporary directory and swaps it into place.
/// Writes source maps outside the AddOn folder (spec codegen §14.6):
/// `<parent>/.maps/<Addon>/<output path>.map`. Maps of per-flavor files
/// (milestone M2) add a `<flavor>/` level.
pub(crate) fn write_maps(
    dir: &Path,
    addon: &str,
    maps: &BTreeMap<String, Arc<EncodedMap>>,
) -> std::io::Result<()> {
    let Some(parent) = dir.parent() else {
        return Ok(());
    };
    let root = parent.join(".maps").join(addon);
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    for (rel, map) in maps {
        let path = root.join(rel);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(path, map.to_json())?;
    }
    Ok(())
}

pub(crate) fn write_tree(dir: &Path, out: &BTreeMap<String, Content>) -> std::io::Result<()> {
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

/// The runtime slices of a build: those the compiled code uses, plus the
/// tools the profile builds in (decision T96).
fn tool_slices(
    profile: &wowlua_manifest::profile::Profile,
    used: &BTreeSet<wowlua_runtime::Slice>,
) -> BTreeSet<wowlua_runtime::Slice> {
    use wowlua_runtime::Slice;
    let mut out = used.clone();
    if profile.dev_console {
        out.insert(Slice::DevConsole);
    }
    if profile.monitor {
        out.insert(Slice::Monitor);
    }
    if profile.capture_errors {
        out.insert(Slice::Errors);
    }
    if profile.instrument != wowlua_manifest::model::Instrument::None && profile.dev_console {
        out.insert(Slice::Prof);
    }
    out
}

/// The runtime's version number (`0.6.0` → 600): the newest runtime among
/// the loaded dev builds serves the dev console (decision T98).
fn runtime_version() -> u32 {
    let mut parts = crate::COMPILER_VERSION
        .split('.')
        .map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    major * 1_000_000 + minor * 1_000 + patch
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
