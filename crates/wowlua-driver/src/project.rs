//! Loading a project: finding and validating `wowlua.json` (and the
//! workspace around it), resolving dependencies, and bringing `wowlua.lock`
//! up to date.

use crate::Session;
use crate::lockfile::{self, LockError, Lockfile};
use crate::workspace::{self, WorkspaceAddon};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::LoadedManifest;
use wowlua_manifest::model::PackageKind;

#[derive(Clone, Debug, Default)]
pub struct LoadOptions {
    /// `--manifest`.
    pub manifest: Option<PathBuf>,
    /// Directory to search from (normally the current directory).
    pub cwd: PathBuf,
    /// `-p, --package`: the workspace member to use.
    pub package: Option<String>,
    /// `--locked` / `--frozen`: fail instead of changing `wowlua.lock`.
    pub locked: bool,
    /// The editor (`wowlua lsp`): resolve as the lockfile would change, but
    /// leave `wowlua.lock` as it is on disk.
    pub lock_in_memory: bool,
    /// `--offline` / `--frozen`: only the cache, never the network.
    pub offline: bool,
    /// Commands that never look at dependencies (`fmt`, `trace`) skip
    /// resolving and fetching them.
    pub skip_dependencies: bool,
    /// Which locked pins may move (`wowlua update`).
    pub update: Option<wowlua_pkg::Update>,
    /// `wowlua update <name> --precise <pin>`.
    pub precise: Option<(String, String)>,
    /// Progress lines (`Cloning …`, `Fetching …`).
    pub status: Option<Status>,
    /// `package` / `publish` at a workspace root: also load the members the
    /// root's `publish.bundle-addons` bundles.
    pub bundled: bool,
    /// Every member of the workspace, from a member's folder too (the
    /// language server's checks of a whole workspace).
    pub all_members: bool,
}

/// A progress callback: status word and message.
pub type StatusFn = dyn Fn(&str, &str);

/// A sink for progress lines.
#[derive(Clone)]
pub struct Status(pub std::rc::Rc<StatusFn>);

impl std::fmt::Debug for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Status")
    }
}

/// A loaded, validated project.
#[derive(Debug)]
pub struct Project {
    pub manifest: LoadedManifest,
    pub lock: Lockfile,
    /// Interface numbers per targeted flavor: explicit in the manifest, or
    /// pinned in the lockfile.
    pub interfaces: BTreeMap<String, Vec<u32>>,
    /// The resolved dependencies, fetched into the cache (empty when the
    /// command skips them). In a workspace: those of every member.
    pub deps: wowlua_pkg::Resolution,
    /// The workspace root, when the package is a workspace member.
    pub workspace_root: Option<PathBuf>,
    /// The workspace's other addons (generated declaration files).
    pub workspace_addons: Vec<WorkspaceAddon>,
    /// The workspace's external addons, fetched (`workspace.external-addons`).
    pub external_addons: Vec<ExternalAddon>,
    /// Selected by the command (not only loaded for a bundle).
    pub selected: bool,
    /// Bundled into the workspace root's zip (`publish.bundle-addons`).
    pub bundled: bool,
    /// `--features` and `--no-default-features` (spec Manifest §9.6).
    pub features: FeatureSelection,
}

/// The features a command selects: the manifest's `default` ones unless
/// `no_default`, and `features` besides (spec Manifest §9.6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureSelection {
    /// `--features a,b`.
    pub features: Vec<String>,
    /// `--no-default-features`.
    pub no_default: bool,
}

/// Applies the command's feature selection to the selected projects. A
/// feature that none of them defines is error E4314; each project enables
/// the named features it defines.
pub fn select_features(
    session: &mut Session,
    projects: &mut [Project],
    selection: &FeatureSelection,
) -> bool {
    let mut ok = true;
    for name in &selection.features {
        let known = projects
            .iter()
            .filter(|p| p.selected)
            .any(|p| p.manifest.manifest.features.contains_key(name) && name != "default");
        if !known {
            let mut known: Vec<&str> = projects
                .iter()
                .filter(|p| p.selected)
                .flat_map(|p| p.manifest.manifest.features.keys())
                .map(String::as_str)
                .filter(|k| *k != "default")
                .collect();
            known.sort_unstable();
            known.dedup();
            let help = if known.is_empty() {
                "the manifest's `features` defines no features".to_string()
            } else {
                format!("the features are: {}", known.join(", "))
            };
            session.push(
                Diagnostic::error(
                    "E4314",
                    format!("`--features` names the unknown feature `{name}`"),
                )
                .with_help(help),
            );
            ok = false;
        }
    }
    for project in projects.iter_mut() {
        let own = &project.manifest.manifest.features;
        project.features = FeatureSelection {
            features: selection
                .features
                .iter()
                .filter(|f| own.contains_key(*f))
                .cloned()
                .collect(),
            no_default: selection.no_default,
        };
    }
    ok
}

/// Warnings a project turns on in `check.lints` (W2063, implicit `any`).
pub const OFF_BY_DEFAULT: &[&str] = &["W2063"];

impl Project {
    /// Applies the manifest's `check.lints` severities to the session's
    /// diagnostics.
    pub fn apply_lints(&self, session: &mut Session) {
        use wowlua_diagnostics::LintLevel;
        use wowlua_manifest::model::LintSetting;
        let lints = &self.manifest.manifest.check.lints;
        session.diagnostics.apply_lint_levels(|code| {
            match lints.get(code) {
                Some(LintSetting::Allow) => Some(LintLevel::Allow),
                Some(LintSetting::Warn) => Some(LintLevel::Warn),
                Some(LintSetting::Deny) => Some(LintLevel::Deny),
                // Off unless the manifest turns it on (decision T274).
                None if OFF_BY_DEFAULT.contains(&code) => Some(LintLevel::Allow),
                None => None,
            }
        });
    }
}

/// An AddOn fetched and shipped as it is (`workspace.external-addons`).
#[derive(Debug, Clone)]
pub struct ExternalAddon {
    /// The AddOn folder name.
    pub name: String,
    /// The fetched package (`None` when the command skipped dependencies).
    pub package: Option<wowlua_pkg::ResolvedPackage>,
    /// The packager's processing applies (the workspace root's
    /// `compat.packager-keywords`).
    pub packager: bool,
    /// Processed text files get CR LF (`compat.line-endings: "crlf"`).
    pub crlf: bool,
    /// The flavors the workspace targets (install folders).
    pub flavors: Vec<String>,
}

/// The dependency name an external addon is resolved under.
pub fn external_dependency_name(addon: &str) -> String {
    let mut out = String::from("addon-");
    for c in addon.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(64).collect()
}

impl Project {
    pub fn root(&self) -> &std::path::Path {
        &self.manifest.root
    }

    /// The enabled features (without `default`) and the optional
    /// dependencies they turn on: the `default` ones unless
    /// `--no-default-features`, `--features`, and what they include.
    pub fn enabled_features(&self) -> (BTreeSet<String>, BTreeSet<String>) {
        let features = &self.manifest.manifest.features;
        let mut stack: Vec<String> = self.features.features.clone();
        if !self.features.no_default {
            stack.extend(features.get("default").cloned().unwrap_or_default());
        }
        let (mut enabled, mut deps) = (BTreeSet::new(), BTreeSet::new());
        while let Some(f) = stack.pop() {
            if let Some(dep) = f.strip_prefix("dep:") {
                deps.insert(dep.to_string());
            } else if enabled.insert(f.clone())
                && let Some(more) = features.get(&f)
            {
                stack.extend(more.iter().cloned());
            }
        }
        enabled.remove("default");
        (enabled, deps)
    }

    pub fn addon_name(&self) -> String {
        self.manifest.addon_name()
    }

    /// Where `wowlua.lock` lives.
    pub fn lock_root(&self) -> &Path {
        self.workspace_root
            .as_deref()
            .unwrap_or(&self.manifest.root)
    }

    pub fn kind(&self) -> PackageKind {
        self.manifest.package().map(|p| p.kind).unwrap_or_default()
    }
}

/// Loads the one project a command works on. In a workspace root without
/// `-p`, this needs a single selected member.
pub fn load_project(session: &mut Session, options: &LoadOptions) -> Option<Project> {
    let mut projects = load_projects(session, options)?;
    if projects.len() == 1 {
        return projects.pop();
    }
    let names: Vec<String> = projects
        .iter()
        .filter_map(|p| p.manifest.package().map(|pk| pk.name.clone()))
        .collect();
    session.push(
        Diagnostic::plain_error(
            "this command works on one package, but the workspace selects several",
        )
        .with_note(format!("members: {}", names.join(", ")))
        .with_help("choose one with `-p <package>`, or run the command inside the member's folder"),
    );
    None
}

/// The `wowlua.json` in `start` or the nearest folder above it.
pub fn find_manifest(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(wowlua_manifest::MANIFEST_FILE))
        .find(|candidate| crate::host::is_file(candidate))
}

/// Reads and validates the manifest at `path` ([`wowlua_manifest::load`]
/// through the host).
pub fn load_manifest(db: &mut wowlua_span::SourceDb, path: &Path) -> wowlua_manifest::LoadOutput {
    let text = match crate::host::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            let mut diagnostics = wowlua_diagnostics::Diagnostics::new();
            diagnostics.push(
                Diagnostic::plain_error(format!("cannot read the manifest: {error}"))
                    .with_path(path),
            );
            return wowlua_manifest::LoadOutput {
                manifest: None,
                diagnostics,
            };
        }
    };
    let mut output = wowlua_manifest::parse(db, path, text, &crate::compiler_version());
    if let Some(manifest) = &mut output.manifest {
        manifest.path = path.to_path_buf();
        manifest.root = path.parent().map(Path::to_path_buf).unwrap_or_default();
    }
    output
}

fn no_manifest(options: &LoadOptions) -> Diagnostic {
    Diagnostic::plain_error(format!(
        "no {} found in `{}` or any parent directory",
        wowlua_manifest::MANIFEST_FILE,
        options.cwd.display()
    ))
    .with_help(
        "run `wowlua new <path>` to create a package or `wowlua init` to adopt an existing addon",
    )
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&crate::host::read_to_string(path).ok()?).ok()
}

fn member_globs(patterns: &[String]) -> globset::GlobSet {
    let mut b = globset::GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = globset::GlobBuilder::new(p.trim_end_matches('/'))
            .literal_separator(true)
            .build()
        {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| globset::GlobSet::empty())
}

fn is_member(root_value: &serde_json::Value, rel: &str) -> bool {
    let list = |key: &str| -> Vec<String> {
        root_value["workspace"][key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    member_globs(&list("members")).is_match(rel) && !member_globs(&list("exclude")).is_match(rel)
}

/// The workspace root above `dir` that lists it as a member.
fn workspace_root_above(dir: &Path) -> Option<PathBuf> {
    for ancestor in dir.ancestors().skip(1) {
        let candidate = ancestor.join(wowlua_manifest::MANIFEST_FILE);
        if !crate::host::is_file(&candidate) {
            continue;
        }
        let value = read_json(&candidate)?;
        if value.get("workspace").is_none() {
            continue;
        }
        let rel = dir
            .strip_prefix(ancestor)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        return is_member(&value, &rel).then(|| ancestor.to_path_buf());
    }
    None
}

/// Parses a manifest text into a loaded manifest at `path`.
fn parse_manifest(
    session: &mut Session,
    options: &LoadOptions,
    path: &Path,
    text: String,
) -> Option<LoadedManifest> {
    // Diagnostics show the manifest relative to the current directory.
    let display = display_path(path, &options.cwd);
    let mut output =
        wowlua_manifest::parse(&mut session.db, &display, text, &crate::compiler_version());
    session.diagnostics.extend(output.diagnostics);
    let mut manifest = output.manifest.take()?;
    manifest.path = path.to_path_buf();
    manifest.root = path.parent().map(PathBuf::from).unwrap_or_default();
    // `.pkgmeta` as the source of dependencies (spec CLI §12.6).
    if manifest.manifest.compat.pkgmeta {
        let mut problems = Vec::new();
        crate::pkgmeta::apply_compat(&mut manifest, &mut problems);
        let failed = problems.iter().any(|d| d.is_error());
        session.diagnostics.extend(problems);
        if failed {
            return None;
        }
    }
    Some(manifest)
}

fn read_lock(session: &mut Session, root: &Path) -> Option<Option<Lockfile>> {
    match Lockfile::read(root) {
        Ok(lock) => Some(lock),
        Err(LockError::TooNew(version)) => {
            session.push(
                Diagnostic::error(
                    "E4320",
                    format!(
                        "wowlua.lock has version {version}, which this compiler does not understand"
                    ),
                )
                .with_path(root.join(lockfile::LOCKFILE))
                .with_help("update wowlua"),
            );
            None
        }
        Err(LockError::Invalid(message)) => {
            session.push(
                Diagnostic::plain_error(format!("invalid wowlua.lock: {message}"))
                    .with_path(root.join(lockfile::LOCKFILE))
                    .with_help("delete it to regenerate it"),
            );
            None
        }
    }
}

/// Interface numbers set explicitly in the manifest.
fn explicit_interfaces(manifest: &LoadedManifest) -> BTreeMap<String, Vec<u32>> {
    manifest
        .manifest
        .flavors
        .iter()
        .flatten()
        .filter_map(|(name, settings)| {
            let id = wowlua_flavors::table().resolve(name)?.id.clone();
            settings.interface.clone().map(|i| (id, i))
        })
        .collect()
}

fn interfaces_of(manifest: &LoadedManifest, lock: &Lockfile) -> BTreeMap<String, Vec<u32>> {
    let explicit = explicit_interfaces(manifest);
    manifest
        .flavors
        .iter()
        .map(|id| {
            let numbers = explicit
                .get(id)
                .or_else(|| lock.flavor_table.interfaces.get(id))
                .cloned()
                .unwrap_or_default();
            (id.clone(), numbers)
        })
        .collect()
}

/// Writes the lockfile when it changed (E4104 with `--locked`).
fn store_lock(
    session: &mut Session,
    options: &LoadOptions,
    root: &Path,
    pinned: Option<&Lockfile>,
    lock: &Lockfile,
) -> Option<()> {
    if pinned == Some(lock) || options.lock_in_memory {
        return Some(());
    }
    if options.locked {
        session.push(
            Diagnostic::error(
                "E4104",
                "wowlua.lock needs to be updated, but `--locked` was given",
            )
            .with_path(root.join(lockfile::LOCKFILE))
            .with_help("run the command without `--locked` and commit the updated lockfile"),
        );
        return None;
    }
    if let Err(error) = lock.write(root) {
        session.push(
            Diagnostic::plain_error(format!("cannot write wowlua.lock: {error}"))
                .with_path(root.join(lockfile::LOCKFILE)),
        );
        return None;
    }
    Some(())
}

/// `wowlua types sync [--flavor <f>…]` (spec Package manager §10.9): pins
/// the loaded projects' lockfile to the compiler's flavor table: its
/// version, the pinned interface numbers (of `only`'s flavors, when given)
/// and the client aliases, with W4313 for each alias the table moved to
/// another ruleset. The `wow-api-<flavor>` declarations ship with the
/// compiler until the registry provides them (decision T83), so no types
/// package is locked. Returns the changes, one line each (`None` on an
/// error: `--locked` with a change is E4104).
pub fn types_sync(
    session: &mut Session,
    options: &LoadOptions,
    projects: &[Project],
    only: Option<&[String]>,
) -> Option<Vec<String>> {
    let first = projects.first()?;
    let root = first
        .workspace_root
        .clone()
        .unwrap_or_else(|| first.root().to_path_buf());
    let mut aliases: Vec<String> = projects
        .iter()
        .flat_map(|p| lockfile::client_aliases(&p.manifest))
        .collect();
    aliases.sort();
    aliases.dedup();
    let pinned = &first.lock;
    let (lock, moved) = lockfile::sync(pinned, wowlua_flavors::table(), only, &aliases);
    for m in &moved {
        session.push(
            Diagnostic::warning(
                "W4313",
                format!(
                    "the flavor alias `{}` now names `{}`, not `{}`",
                    m.alias, m.to, m.from
                ),
            )
            .with_path(&first.manifest.path)
            .with_help(format!(
                "commit the updated wowlua.lock to build `{}` for `{}`; to keep building for `{}`, name it `{}` in `flavors`",
                m.alias, m.to, m.from, m.from
            )),
        );
    }
    let mut changes = Vec::new();
    if lock.flavor_table.version != pinned.flavor_table.version {
        changes.push(format!(
            "flavor table {} -> {}",
            pinned.flavor_table.version, lock.flavor_table.version
        ));
    }
    for (id, pins) in &lock.flavor_table.interfaces {
        let before = pinned.flavor_table.interfaces.get(id);
        if before != Some(pins) {
            let list = |l: &[u32]| l.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
            changes.push(format!(
                "{id} interface {} -> {}",
                before.map_or_else(String::new, |b| list(b)),
                list(pins)
            ));
        }
    }
    for m in &moved {
        changes.push(format!("alias {} {} -> {}", m.alias, m.from, m.to));
    }
    store_lock(session, options, &root, Some(pinned), &lock)?;
    Some(changes)
}

/// Loads the projects a command works on: the package, or the selected
/// members of a workspace (`-p`, the member around the current directory,
/// `default-members`, or all members).
pub fn load_projects(session: &mut Session, options: &LoadOptions) -> Option<Vec<Project>> {
    let found = match &options.manifest {
        Some(path) => Some(path.clone()),
        None => find_manifest(&options.cwd),
    };
    let Some(path) = found else {
        session.push(no_manifest(options));
        return None;
    };
    let dir = path.parent().map(PathBuf::from).unwrap_or_default();
    let is_root = read_json(&path).is_some_and(|v| v.get("workspace").is_some());
    let root = if is_root {
        Some(dir.clone())
    } else {
        workspace_root_above(&dir)
    };
    match root {
        None => {
            if let Some(name) = &options.package {
                session.push(
                    Diagnostic::plain_error(format!(
                        "`-p {name}` selects a workspace member, but this package is not in a workspace"
                    ))
                    .with_path(&path),
                );
                return None;
            }
            load_single(session, options, &path).map(|p| vec![p])
        }
        Some(root) => load_workspace(session, options, &root, (!is_root).then_some(dir.as_path())),
    }
}

/// The project of a standalone `wowlua compile` (no `wowlua.json`): an
/// addon at `root` with the built-in defaults, its sources there. Nothing
/// is written (no lockfile) and there are no dependencies.
pub fn standalone_project(
    session: &mut Session,
    options: &LoadOptions,
    root: &Path,
) -> Option<Project> {
    let text = "{ \"package\": { \"name\": \"compiled\", \"version\": \"0.0.0\" }, \"build\": { \"src\": \".\" } }\n";
    let path = root.join(wowlua_manifest::MANIFEST_FILE);
    let manifest = parse_manifest(session, options, &path, text.to_string())?;
    let lock = lockfile::required(None, &manifest.flavors, &[]);
    let interfaces = interfaces_of(&manifest, &lock);
    Some(Project {
        manifest,
        lock,
        interfaces,
        deps: wowlua_pkg::Resolution::default(),
        workspace_root: None,
        workspace_addons: Vec::new(),
        external_addons: Vec::new(),
        selected: true,
        bundled: false,
        features: FeatureSelection::default(),
    })
}

/// A package outside any workspace.
fn load_single(session: &mut Session, options: &LoadOptions, path: &Path) -> Option<Project> {
    let text = match crate::host::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            session.push(
                Diagnostic::plain_error(format!("cannot read the manifest: {error}"))
                    .with_path(path),
            );
            return None;
        }
    };
    let manifest = parse_manifest(session, options, path, text)?;
    let pinned = read_lock(session, &manifest.root)?;
    let explicit = explicit_interfaces(&manifest);
    let pinned_flavors: Vec<String> = manifest
        .flavors
        .iter()
        .filter(|id| !explicit.contains_key(*id))
        .cloned()
        .collect();
    let mut lock = lockfile::required(
        pinned.as_ref(),
        &pinned_flavors,
        &lockfile::client_aliases(&manifest),
    );
    let deps = if options.skip_dependencies {
        wowlua_pkg::Resolution::default()
    } else {
        let deps = resolve(
            session,
            options,
            &[&manifest],
            None,
            &manifest.root,
            &lock,
            &HashMap::new(),
        )?;
        lock.packages = deps.lock_entries(&manifest.root);
        deps
    };
    store_lock(session, options, &manifest.root, pinned.as_ref(), &lock)?;
    let interfaces = interfaces_of(&manifest, &lock);
    Some(Project {
        manifest,
        lock,
        interfaces,
        deps,
        workspace_root: None,
        workspace_addons: Vec::new(),
        external_addons: Vec::new(),
        selected: true,
        bundled: false,
        features: FeatureSelection::default(),
    })
}

/// A workspace: every member is loaded and resolved together; the selected
/// ones are returned.
fn load_workspace(
    session: &mut Session,
    options: &LoadOptions,
    root: &Path,
    inside: Option<&Path>,
) -> Option<Vec<Project>> {
    let root_path = root.join(wowlua_manifest::MANIFEST_FILE);
    let root_text = match crate::host::read_to_string(&root_path) {
        Ok(t) => t,
        Err(error) => {
            session.push(
                Diagnostic::plain_error(format!("cannot read the workspace manifest: {error}"))
                    .with_path(&root_path),
            );
            return None;
        }
    };
    let root_manifest = parse_manifest(session, options, &root_path, root_text.clone())?;
    let root_value: serde_json::Value = serde_json::from_str(&root_text).ok()?;
    let ws = root_manifest.manifest.workspace.clone().unwrap_or_default();
    let out_root = root.join(&root_manifest.manifest.build.out);

    // Member directories: the root itself when it is a package, and every
    // folder with a manifest that `members` matches.
    let mut dirs: Vec<PathBuf> = Vec::new();
    if root_manifest.package().is_some() {
        dirs.push(root.to_path_buf());
    }
    let enter = |p: &Path| p != out_root && p.file_name().is_none_or(|n| n != "node_modules");
    for (dir, kind) in crate::host::walk_entries(root, false, &enter) {
        if kind != crate::host::Kind::Dir
            || !crate::host::is_file(dir.join(wowlua_manifest::MANIFEST_FILE))
        {
            continue;
        }
        let rel = dir
            .strip_prefix(root)
            .unwrap_or(&dir)
            .to_string_lossy()
            .replace('\\', "/");
        if is_member(&root_value, &rel) {
            dirs.push(dir);
        }
    }
    dirs.sort();
    if dirs.is_empty() {
        session.push(
            Diagnostic::plain_error("the workspace has no members")
                .with_path(&root_path)
                .with_help("list member folders in `workspace.members`, e.g. `[\"addons/*\"]`"),
        );
        return None;
    }

    let mut manifests: Vec<LoadedManifest> = Vec::new();
    for dir in &dirs {
        let path = dir.join(wowlua_manifest::MANIFEST_FILE);
        if dir == root {
            let mut m = root_manifest.clone();
            m.manifest.workspace = None;
            manifests.push(m);
            continue;
        }
        let Ok(text) = crate::host::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            // Reported with positions by the parser.
            parse_manifest(session, options, &path, text);
            continue;
        };
        if value["build"].get("out").is_some() {
            session.push(
                Diagnostic::error("E4316", "a workspace member may not set `build.out`")
                    .with_path(&path)
                    .with_help("the workspace root's `build.out` holds every member's build"),
            );
            continue;
        }
        let merged = workspace::with_defaults(&value, &root_value);
        let text = if merged == value {
            text
        } else {
            let mut t = serde_json::to_string_pretty(&merged).unwrap_or(text);
            t.push('\n');
            t
        };
        let Some(mut m) = parse_manifest(session, options, &path, text) else {
            continue;
        };
        // Every member builds into the root's `build.out`.
        m.manifest.build.out = wowlua_pkg::source::relative(&out_root, &m.root)
            .unwrap_or_else(|| out_root.display().to_string());
        manifests.push(m);
    }
    if session.has_errors() {
        return None;
    }

    check_members(session, &manifests);
    if session.has_errors() {
        return None;
    }

    let pinned = read_lock(session, root)?;
    let mut pinned_flavors: BTreeSet<String> = BTreeSet::new();
    for m in &manifests {
        let explicit = explicit_interfaces(m);
        pinned_flavors.extend(
            m.flavors
                .iter()
                .filter(|f| !explicit.contains_key(*f))
                .cloned(),
        );
    }
    let mut aliases: Vec<String> = manifests
        .iter()
        .flat_map(lockfile::client_aliases)
        .collect();
    aliases.sort();
    aliases.dedup();
    let mut lock = lockfile::required(
        pinned.as_ref(),
        &pinned_flavors.into_iter().collect::<Vec<_>>(),
        &aliases,
    );
    let members: HashMap<String, PathBuf> = manifests
        .iter()
        .filter_map(|m| {
            let p = m.package()?;
            (p.kind != PackageKind::Addon).then(|| (p.name.clone(), m.root.clone()))
        })
        .collect();
    // External addons resolve like dependencies of the workspace itself.
    let externals: Vec<(String, String, wowlua_manifest::model::DependencyDetail)> = ws
        .external_addons
        .iter()
        .map(|(name, d)| {
            let mut d = d.clone();
            d.kind = Some(PackageKind::LuaLibrary);
            (name.clone(), external_dependency_name(name), d)
        })
        .collect();
    let deps = if options.skip_dependencies {
        wowlua_pkg::Resolution::default()
    } else {
        let refs: Vec<&LoadedManifest> = manifests.iter().collect();
        let extra = (!externals.is_empty()).then(|| wowlua_pkg::Root {
            name: "workspace".into(),
            version: semver::Version::new(0, 0, 0),
            dir: root.to_path_buf(),
            manifest_path: root_path.clone(),
            deps: externals
                .iter()
                .map(|(_, key, d)| {
                    (
                        key.clone(),
                        wowlua_manifest::model::Dependency::Detailed(Box::new(d.clone())),
                    )
                })
                .collect(),
        });
        let deps = resolve(session, options, &refs, extra, root, &lock, &members)?;
        lock.packages = deps.lock_entries(root);
        deps
    };
    let all_flavors: Vec<String> = {
        let mut f: Vec<String> = Vec::new();
        for m in &manifests {
            for id in &m.flavors {
                if !f.contains(id) {
                    f.push(id.clone());
                }
            }
        }
        f
    };
    let external_addons: Vec<ExternalAddon> = externals
        .iter()
        .map(|(name, key, _)| {
            let package = deps
                .packages
                .iter()
                .find(|p| &p.name == key && p.used_by.iter().any(|u| u == "workspace"))
                .cloned();
            ExternalAddon {
                name: name.clone(),
                package,
                packager: root_manifest.manifest.compat.packager_keywords,
                crlf: root_manifest.manifest.compat.line_endings
                    == Some(wowlua_manifest::model::LineEndings::Crlf),
                flavors: all_flavors.clone(),
            }
        })
        .collect();
    store_lock(session, options, root, pinned.as_ref(), &lock)?;

    let addons: Vec<WorkspaceAddon> = manifests
        .iter()
        .filter(|m| m.package().is_some_and(|p| p.kind == PackageKind::Addon))
        .map(|m| WorkspaceAddon {
            name: m.addon_name(),
            package: m.package().map(|p| p.name.clone()).unwrap_or_default(),
            src: m.root.join(&m.manifest.build.src),
            manifest: m.manifest.clone(),
        })
        .collect();

    // Selection.
    let selected: Vec<usize> = if options.all_members {
        (0..manifests.len()).collect()
    } else if let Some(name) = &options.package {
        match manifests
            .iter()
            .position(|m| m.package().is_some_and(|p| &p.name == name))
        {
            Some(i) => vec![i],
            None => {
                let names: Vec<String> = manifests
                    .iter()
                    .filter_map(|m| m.package().map(|p| p.name.clone()))
                    .collect();
                session.push(
                    Diagnostic::plain_error(format!("the workspace has no member `{name}`"))
                        .with_path(&root_path)
                        .with_note(format!("members: {}", names.join(", "))),
                );
                return None;
            }
        }
    } else if let Some(dir) = inside {
        manifests
            .iter()
            .position(|m| m.root == dir)
            .into_iter()
            .collect()
    } else if !ws.default_members.is_empty() {
        let set = member_globs(&ws.default_members);
        manifests
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                let rel = m
                    .root
                    .strip_prefix(root)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                set.is_match(if rel.is_empty() { "." } else { &rel })
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        (0..manifests.len()).collect()
    };

    // At the root, `package` and `publish` also load the bundled members.
    let bundled: Vec<usize> = if options.bundled && options.package.is_none() && inside.is_none() {
        let addon = |m: &LoadedManifest| m.package().is_some_and(|p| p.kind == PackageKind::Addon);
        match root_manifest
            .manifest
            .publish
            .as_ref()
            .and_then(|p| p.bundle_addons.as_ref())
        {
            Some(wowlua_manifest::model::BundleAddons::All(true)) => manifests
                .iter()
                .enumerate()
                .filter(|(_, m)| addon(m))
                .map(|(i, _)| i)
                .collect(),
            Some(wowlua_manifest::model::BundleAddons::Listed(names)) => manifests
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    addon(m)
                        && names.iter().any(|n| {
                            *n == m.addon_name() || m.package().is_some_and(|p| p.name == *n)
                        })
                })
                .map(|(i, _)| i)
                .collect(),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };
    let mut projects = Vec::new();
    for (i, manifest) in manifests.into_iter().enumerate() {
        if !selected.contains(&i) && !bundled.contains(&i) {
            continue;
        }
        let interfaces = interfaces_of(&manifest, &lock);
        let own = manifest.addon_name();
        projects.push(Project {
            interfaces,
            lock: lock.clone(),
            deps: wowlua_pkg::Resolution {
                packages: deps.packages.clone(),
            },
            workspace_root: Some(root.to_path_buf()),
            workspace_addons: addons.iter().filter(|a| a.name != own).cloned().collect(),
            external_addons: external_addons.clone(),
            manifest,
            selected: selected.contains(&i),
            bundled: bundled.contains(&i),
            features: FeatureSelection::default(),
        });
    }
    Some(projects)
}

/// Workspace-wide rules: unique addon names (E4325) and SavedVariables
/// (E4120), and no cycle of required addon dependencies (E4324).
fn check_members(session: &mut Session, manifests: &[LoadedManifest]) {
    let addons: Vec<&LoadedManifest> = manifests
        .iter()
        .filter(|m| m.package().is_some_and(|p| p.kind == PackageKind::Addon))
        .collect();
    let mut names: HashMap<String, &Path> = HashMap::new();
    for m in &addons {
        let name = m.addon_name();
        if let Some(other) = names.insert(name.to_ascii_lowercase(), &m.path) {
            session.push(
                Diagnostic::error(
                    "E4325",
                    format!("two workspace members are both the addon `{name}`"),
                )
                .with_path(&m.path)
                .with_note(format!("also {}", other.display())),
            );
        }
    }
    let mut saved: HashMap<String, String> = HashMap::new();
    for m in &addons {
        let Some(addon) = &m.manifest.addon else {
            continue;
        };
        for var in addon.all_saved_variables().iter() {
            if let Some(other) = saved.insert(var.clone(), m.addon_name()) {
                session.push(
                    Diagnostic::error(
                        "E4120",
                        format!(
                            "SavedVariables `{var}` is used by both `{other}` and `{}`",
                            m.addon_name()
                        ),
                    )
                    .with_path(&m.path),
                );
            }
        }
    }
    // Required addon dependencies between members.
    let index: HashMap<String, usize> = addons
        .iter()
        .enumerate()
        .map(|(i, m)| (m.addon_name().to_ascii_lowercase(), i))
        .collect();
    let edges: Vec<Vec<usize>> = addons
        .iter()
        .map(|m| {
            m.manifest
                .addon_dependencies
                .as_ref()
                .map(|d| {
                    d.required
                        .iter()
                        .filter_map(|r| index.get(&r.name().to_ascii_lowercase()).copied())
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    let mut state = vec![0u8; addons.len()];
    fn dfs(
        i: usize,
        edges: &[Vec<usize>],
        state: &mut [u8],
        stack: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        state[i] = 1;
        stack.push(i);
        for &j in &edges[i] {
            if state[j] == 1 {
                let start = stack.iter().position(|&s| s == j).unwrap_or(0);
                return Some(stack[start..].to_vec());
            }
            if state[j] == 0
                && let Some(c) = dfs(j, edges, state, stack)
            {
                return Some(c);
            }
        }
        stack.pop();
        state[i] = 2;
        None
    }
    for i in 0..addons.len() {
        if state[i] == 0
            && let Some(cycle) = dfs(i, &edges, &mut state, &mut Vec::new())
        {
            let mut names: Vec<String> = cycle.iter().map(|&c| addons[c].addon_name()).collect();
            names.push(names[0].clone());
            session.push(
                Diagnostic::error(
                    "E4324",
                    format!(
                        "the workspace's required addon dependencies form a cycle: {}",
                        names.join(" → ")
                    ),
                )
                .with_path(&addons[cycle[0]].path)
                .with_help("make one of them an optional dependency"),
            );
            break;
        }
    }
}

/// Resolves the dependencies of `manifests` together (all features, flavors
/// and dev-dependencies) and fetches them into the cache.
fn resolve(
    session: &mut Session,
    options: &LoadOptions,
    manifests: &[&LoadedManifest],
    extra: Option<wowlua_pkg::Root>,
    lock_root: &Path,
    lock: &Lockfile,
    members: &HashMap<String, PathBuf>,
) -> Option<wowlua_pkg::Resolution> {
    let mut roots: Vec<wowlua_pkg::Root> = manifests
        .iter()
        .filter(|m| !m.manifest.dependencies.is_empty() || !m.manifest.dev_dependencies.is_empty())
        .map(|manifest| {
            let package = manifest.package();
            let m = &manifest.manifest;
            wowlua_pkg::Root {
                name: package.map(|p| p.name.clone()).unwrap_or_default(),
                version: package
                    .and_then(|p| semver::Version::parse(&p.version).ok())
                    .unwrap_or(semver::Version::new(0, 0, 0)),
                dir: manifest.root.clone(),
                manifest_path: manifest.path.clone(),
                deps: m
                    .dependencies
                    .iter()
                    .chain(&m.dev_dependencies)
                    .map(|(n, d)| (n.clone(), d.clone()))
                    .collect(),
            }
        })
        .collect();
    roots.extend(extra);
    if roots.is_empty() {
        return Some(wowlua_pkg::Resolution::default());
    }
    let Some(cache) = wowlua_pkg::Cache::new() else {
        session.push(
            Diagnostic::plain_error("cannot find a home directory for the package cache")
                .with_help("set `WOWLUA_HOME`"),
        );
        return None;
    };
    let status = options.status.clone();
    let fetcher = wowlua_pkg::Fetcher::new(cache, options.offline, move |word, message| {
        if let Some(s) = &status {
            (s.0)(word, message);
        }
    });
    let update = options.update.clone().unwrap_or(wowlua_pkg::Update::None);
    let resolved = wowlua_pkg::resolve(
        &mut session.db,
        &roots,
        &fetcher,
        &wowlua_pkg::resolve::Options {
            lock: &lock.packages,
            lock_root,
            update: &update,
            compiler: &crate::compiler_version(),
            members,
            precise: options
                .precise
                .as_ref()
                .map(|(n, p)| (n.as_str(), p.as_str())),
        },
    );
    match resolved {
        Ok(r) => Some(r),
        Err(errors) => {
            session.diagnostics.extend(errors);
            None
        }
    }
}

/// A path relative to `cwd` when possible, for messages.
pub fn display_path(path: &std::path::Path, cwd: &std::path::Path) -> PathBuf {
    path.strip_prefix(cwd)
        .map(PathBuf::from)
        .unwrap_or_else(|_| path.to_path_buf())
}
