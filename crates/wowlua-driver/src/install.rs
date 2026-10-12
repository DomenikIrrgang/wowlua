//! Installing builds into WoW (spec §12.5): locating the installation, the
//! client folders of each flavor, safe replacement and links.

use crate::build::BuildOutput;
use crate::project::Project;
use crate::{Session, fsutil};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;

#[derive(Clone, Debug, Default)]
pub struct InstallRequest {
    /// `--wow`: the WoW installation directory.
    pub wow: Option<PathBuf>,
    /// `--client`: restrict to these client folders (e.g. `_retail_`).
    pub clients: Option<Vec<String>>,
    /// `--flavor`: restrict to these flavors (IDs or aliases).
    pub flavors: Option<Vec<String>>,
    /// `--link`: link to the build folder instead of copying.
    pub link: bool,
    /// `--force`: replace folders that were not installed by wowlua.
    pub force: bool,
}

/// One AddOn folder written (or linked) into a client.
#[derive(Clone, Debug)]
pub struct Installed {
    pub client_folder: String,
    pub path: PathBuf,
    pub linked: bool,
}

/// `$WOWLUA_HOME`, default `~/.wowlua`.
pub fn wowlua_home() -> Option<PathBuf> {
    std::env::var_os("WOWLUA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".wowlua")))
}

fn config_wow_path() -> Option<PathBuf> {
    let text = std::fs::read_to_string(wowlua_home()?.join("config.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.pointer("/wow/path")?.as_str().map(PathBuf::from)
}

fn default_install_paths() -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![PathBuf::from("/Applications/World of Warcraft")]
    } else if cfg!(windows) {
        vec![
            PathBuf::from(r"C:\Program Files (x86)\World of Warcraft"),
            PathBuf::from(r"C:\Program Files\World of Warcraft"),
        ]
    } else {
        Vec::new()
    }
}

/// Finds the WoW installation: `--wow`, `WOWLUA_WOW_PATH`, the global
/// config, then the Battle.net default locations.
pub fn find_wow(explicit: Option<&Path>) -> Result<PathBuf, Diagnostic> {
    let candidate = explicit
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("WOWLUA_WOW_PATH").map(PathBuf::from))
        .or_else(config_wow_path);
    if let Some(path) = candidate {
        return if path.is_dir() {
            Ok(path)
        } else {
            Err(Diagnostic::plain_error(format!(
                "the WoW installation `{}` does not exist",
                path.display()
            )))
        };
    }
    default_install_paths()
        .into_iter()
        .find(|p| p.is_dir())
        .ok_or_else(|| {
            Diagnostic::plain_error("World of Warcraft installation not found").with_help(
                "pass `--wow <path>`, set WOWLUA_WOW_PATH, or set `wow.path` in ~/.wowlua/config.json",
            )
        })
}

/// Client folders of a flavor: the manifest's `client-folders` or the flavor
/// table's.
fn client_folders(project: &Project, flavor: &str) -> Vec<String> {
    let explicit = project
        .manifest
        .manifest
        .flavors
        .iter()
        .flatten()
        .find(|(name, _)| {
            wowlua_flavors::table()
                .resolve(name)
                .is_some_and(|f| f.id == flavor)
        })
        .and_then(|(_, settings)| settings.client_folders.clone());
    explicit.unwrap_or_else(|| {
        wowlua_flavors::table()
            .clients_of(flavor)
            .flat_map(|c| c.folders.clone())
            .collect()
    })
}

fn folder_matches(folder: &str, wanted: &str) -> bool {
    folder
        .trim_matches('_')
        .eq_ignore_ascii_case(wanted.trim_matches('_'))
}

/// The AddOn folders this addon would occupy, as `(client folder, path)`.
fn targets(
    session: &mut Session,
    project: &Project,
    addon: &str,
    wow: &Path,
    flavors: &[String],
    clients: Option<&[String]>,
) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for flavor in flavors {
        let folders: Vec<String> = client_folders(project, flavor)
            .into_iter()
            .filter(|f| clients.is_none_or(|wanted| wanted.iter().any(|w| folder_matches(f, w))))
            .filter(|f| wow.join(f).is_dir())
            .collect();
        if folders.is_empty() && clients.is_none() {
            session.push(Diagnostic::note(format!(
                "flavor `{flavor}` has no client folder in `{}`",
                wow.display()
            )));
        }
        for folder in folders {
            let path = wow
                .join(&folder)
                .join("Interface")
                .join("AddOns")
                .join(addon);
            if !out.iter().any(|(_, p)| *p == path) {
                out.push((folder, path));
            }
        }
    }
    out
}

/// Whether an existing AddOn folder was installed by wowlua: a link, or a
/// folder whose TOC carries `X-Wowlua-Build`.
pub fn is_wowlua_install(path: &Path) -> bool {
    // External addons keep their own TOC; the install leaves a marker.
    if path.join(INSTALL_MARKER).is_file() {
        return true;
    }
    if fsutil::is_link(path) {
        return fsutil::link_target(path).is_some_and(|target| {
            has_wowlua_toc(&target) || target.components().any(|c| c.as_os_str() == "dist")
        });
    }
    has_wowlua_toc(path)
}

fn has_wowlua_toc(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("toc"))
            && std::fs::read_to_string(&path)
                .is_ok_and(|text| text.lines().any(|l| l.starts_with("## X-Wowlua-Build:")))
    })
}

/// Installs a finished build into every selected client folder.
pub fn install(
    session: &mut Session,
    project: &Project,
    build: &BuildOutput,
    request: &InstallRequest,
) -> Vec<Installed> {
    let wow = match find_wow(request.wow.as_deref()) {
        Ok(wow) => wow,
        Err(diagnostic) => {
            session.push(diagnostic);
            return Vec::new();
        }
    };
    let mut installed = Vec::new();
    for (folder, path) in targets(
        session,
        project,
        &build.addon_name,
        &wow,
        &build.flavors,
        request.clients.as_deref(),
    ) {
        let exists = path.exists() || fsutil::is_link(&path);
        if exists && !request.force && !is_wowlua_install(&path) {
            session.push(
                Diagnostic::error(
                    "E4331",
                    format!("`{}` exists and was not installed by wowlua", path.display()),
                )
                .with_help("remove it (for example, the same addon from an addon manager) or pass `--force`"),
            );
            continue;
        }
        let result = if request.link {
            link(&build.dir, &path).or_else(|error| {
                session.push(Diagnostic::warning(
                    "W4309",
                    format!(
                        "cannot link `{}` ({error}); copying instead",
                        path.display()
                    ),
                ));
                copy(&build.dir, &path)
            })
        } else {
            copy(&build.dir, &path)
        };
        match result {
            Ok(linked) => installed.push(Installed {
                client_folder: folder,
                path,
                linked,
            }),
            Err(error) => session.push(Diagnostic::plain_error(format!(
                "cannot install into `{}`: {error}",
                path.display()
            ))),
        }
    }
    installed
}

/// Left in AddOn folders `install` copies (the client ignores it).
pub const INSTALL_MARKER: &str = ".wowlua-install";

fn copy(build_dir: &Path, target: &Path) -> std::io::Result<bool> {
    let staged = fsutil::temp_sibling(target)?;
    fsutil::copy_dir(build_dir, &staged)?;
    std::fs::write(staged.join(INSTALL_MARKER), b"installed by wowlua\n")?;
    fsutil::replace_dir(&staged, target)?;
    Ok(false)
}

/// Links `target` to the build folder (the caller falls back to copying).
fn link(build_dir: &Path, target: &Path) -> std::io::Result<bool> {
    let build_dir = build_dir.canonicalize()?;
    if fsutil::link_target(target).is_some_and(|t| t == build_dir) {
        return Ok(true);
    }
    fsutil::remove_any(target)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    fsutil::create_dir_link(&build_dir, target)?;
    Ok(true)
}

/// Removes installed folders (or links) of this addon that carry
/// `X-Wowlua-Build` from the selected clients.
pub fn uninstall(
    session: &mut Session,
    project: &Project,
    addon: &str,
    request: &InstallRequest,
) -> Vec<PathBuf> {
    let wow = match find_wow(request.wow.as_deref()) {
        Ok(wow) => wow,
        Err(diagnostic) => {
            session.push(diagnostic);
            return Vec::new();
        }
    };
    let flavors = request
        .flavors
        .clone()
        .unwrap_or_else(|| project.manifest.flavors.clone());
    let mut removed = Vec::new();
    for (_, path) in targets(
        session,
        project,
        addon,
        &wow,
        &flavors,
        request.clients.as_deref(),
    ) {
        if !(path.exists() || fsutil::is_link(&path)) {
            continue;
        }
        if !is_wowlua_install(&path) {
            session.push(Diagnostic::plain_warning(format!(
                "left `{}` in place: it was not installed by wowlua",
                path.display()
            )));
            continue;
        }
        match fsutil::remove_any(&path) {
            Ok(()) => removed.push(path),
            Err(error) => session.push(Diagnostic::plain_error(format!(
                "cannot remove `{}`: {error}",
                path.display()
            ))),
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_wowlua_installs() {
        let dir = tempfile::tempdir().unwrap();
        let addon = dir.path().join("MyAddon");
        std::fs::create_dir_all(&addon).unwrap();
        std::fs::write(addon.join("MyAddon.toc"), "## Interface: 120100\n").unwrap();
        assert!(!is_wowlua_install(&addon));
        std::fs::write(
            addon.join("MyAddon.toc"),
            "## Interface: 120100\n## X-Wowlua-Build: dev 1234abcd\n",
        )
        .unwrap();
        assert!(is_wowlua_install(&addon));
    }

    #[test]
    fn client_folder_names_match_loosely() {
        assert!(folder_matches("_retail_", "retail"));
        assert!(folder_matches("_classic_era_", "_classic_era_"));
        assert!(!folder_matches("_classic_", "classic_era"));
    }
}
