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
}

/// A loaded, validated project.
#[derive(Debug)]
pub struct Project {
    pub manifest: LoadedManifest,
    pub lock: Lockfile,
    /// Interface numbers per targeted flavor: explicit in the manifest, or
    /// pinned in the lockfile.
    pub interfaces: BTreeMap<String, Vec<u32>>,
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
    let lock = lockfile::required(pinned.as_ref(), &pinned_flavors);
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
    })
}

/// A path relative to `cwd` when possible, for messages.
pub fn display_path(path: &std::path::Path, cwd: &std::path::Path) -> PathBuf {
    path.strip_prefix(cwd)
        .map(PathBuf::from)
        .unwrap_or_else(|_| path.to_path_buf())
}
