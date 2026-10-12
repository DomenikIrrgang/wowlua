//! Loading a project: finding and validating `wowlua.json`, and bringing
//! `wowlua.lock` up to date.

use crate::Session;
use crate::lockfile::{self, LockError, Lockfile};
use std::collections::BTreeMap;
use std::path::PathBuf;
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::LoadedManifest;

#[derive(Clone, Debug, Default)]
pub struct LoadOptions {
    /// `--manifest`.
    pub manifest: Option<PathBuf>,
    /// Directory to search from (normally the current directory).
    pub cwd: PathBuf,
    /// `--locked` / `--frozen`: fail instead of changing `wowlua.lock`.
    pub locked: bool,
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
    /// command skips them).
    pub deps: wowlua_pkg::Resolution,
}

impl Project {
    pub fn root(&self) -> &std::path::Path {
        &self.manifest.root
    }

    pub fn addon_name(&self) -> String {
        self.manifest.addon_name()
    }
}

/// Loads the project for a command. Problems are reported to the session;
/// `None` means the project cannot be used.
pub fn load_project(session: &mut Session, options: &LoadOptions) -> Option<Project> {
    let Some(path) = wowlua_manifest::find_manifest(&options.cwd, options.manifest.as_deref())
    else {
        session.push(
            Diagnostic::plain_error(format!(
                "no {} found in `{}` or any parent directory",
                wowlua_manifest::MANIFEST_FILE,
                options.cwd.display()
            ))
            .with_help("run `wowlua new <path>` to create a package or `wowlua init` to adopt an existing addon"),
        );
        return None;
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            session.push(
                Diagnostic::plain_error(format!("cannot read the manifest: {error}"))
                    .with_path(&path),
            );
            return None;
        }
    };
    // Diagnostics show the manifest relative to the current directory.
    let display = display_path(&path, &options.cwd);
    let mut output =
        wowlua_manifest::parse(&mut session.db, &display, text, &crate::compiler_version());
    session.diagnostics.extend(output.diagnostics);
    let mut manifest = output.manifest.take()?;
    manifest.path = path.clone();
    manifest.root = path.parent().map(PathBuf::from).unwrap_or_default();

    if manifest.is_workspace_root() {
        session.push(
            Diagnostic::plain_error("workspaces are not supported yet")
                .with_path(&path)
                .with_note("workspace support arrives with the package manager (milestone M6)"),
        );
        return None;
    }

    let pinned = match Lockfile::read(&manifest.root) {
        Ok(lock) => lock,
        Err(LockError::TooNew(version)) => {
            session.push(
                Diagnostic::error(
                    "E4320",
                    format!(
                        "wowlua.lock has version {version}, which this compiler does not understand"
                    ),
                )
                .with_path(manifest.root.join(lockfile::LOCKFILE))
                .with_help("update wowlua"),
            );
            return None;
        }
        Err(LockError::Invalid(message)) => {
            session.push(
                Diagnostic::plain_error(format!("invalid wowlua.lock: {message}"))
                    .with_path(manifest.root.join(lockfile::LOCKFILE))
                    .with_help("delete it to regenerate it"),
            );
            return None;
        }
    };

    let explicit: BTreeMap<String, Vec<u32>> = manifest
        .manifest
        .flavors
        .iter()
        .flatten()
        .filter_map(|(name, settings)| {
            let id = wowlua_flavors::table().resolve(name)?.id.clone();
            settings.interface.clone().map(|i| (id, i))
        })
        .collect();
    let pinned_flavors: Vec<String> = manifest
        .flavors
        .iter()
        .filter(|id| !explicit.contains_key(*id))
        .cloned()
        .collect();
    let mut lock = lockfile::required(pinned.as_ref(), &pinned_flavors);
    let deps = if options.skip_dependencies {
        wowlua_pkg::Resolution::default()
    } else {
        let deps = resolve_dependencies(session, &manifest, &lock, options)?;
        lock.packages = deps.lock_entries(&manifest.root);
        deps
    };
    if pinned.as_ref() != Some(&lock) {
        if options.locked {
            session.push(
                Diagnostic::error(
                    "E4104",
                    "wowlua.lock needs to be updated, but `--locked` was given",
                )
                .with_path(manifest.root.join(lockfile::LOCKFILE))
                .with_help("run the command without `--locked` and commit the updated lockfile"),
            );
            return None;
        }
        if let Err(error) = lock.write(&manifest.root) {
            session.push(
                Diagnostic::plain_error(format!("cannot write wowlua.lock: {error}"))
                    .with_path(manifest.root.join(lockfile::LOCKFILE)),
            );
            return None;
        }
    }

    let interfaces = manifest
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
        .collect();

    Some(Project {
        manifest,
        lock,
        interfaces,
        deps,
    })
}

/// Resolves the package's dependencies (all features, flavors and
/// dev-dependencies) and fetches them into the cache.
fn resolve_dependencies(
    session: &mut Session,
    manifest: &LoadedManifest,
    lock: &Lockfile,
    options: &LoadOptions,
) -> Option<wowlua_pkg::Resolution> {
    let m = &manifest.manifest;
    if m.dependencies.is_empty() && m.dev_dependencies.is_empty() {
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
    let package = manifest.package();
    let root = wowlua_pkg::Root {
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
    };
    let update = options.update.clone().unwrap_or(wowlua_pkg::Update::None);
    let resolved = wowlua_pkg::resolve(
        &mut session.db,
        &[root],
        &fetcher,
        &wowlua_pkg::resolve::Options {
            lock: &lock.packages,
            lock_root: &manifest.root,
            update: &update,
            compiler: &crate::compiler_version(),
            members: &Default::default(),
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
