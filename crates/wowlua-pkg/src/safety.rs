//! Archive and checkout safety (spec §10.11, E4323): no absolute paths, no
//! `..` components, and no symlinks or hardlinks that leave the package.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Why an entry is unsafe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsafe {
    pub path: String,
    pub reason: &'static str,
}

/// The relative path an archive entry may be written to, or why not.
pub fn entry_path(name: &str) -> Result<PathBuf, Unsafe> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') && is_drive(&normalized) {
        return Err(Unsafe {
            path: name.into(),
            reason: "absolute path",
        });
    }
    let mut out = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(Unsafe {
                    path: name.into(),
                    reason: "path contains `..`",
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Unsafe {
                    path: name.into(),
                    reason: "absolute path",
                });
            }
        }
    }
    Ok(out)
}

fn is_drive(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Whether a link at `link` (relative to the package root) pointing at
/// `target` stays inside the package.
pub fn link_stays_inside(link: &Path, target: &str) -> bool {
    let target = target.replace('\\', "/");
    if target.starts_with('/') || is_drive(&target) {
        return false;
    }
    let mut depth: Vec<std::ffi::OsString> = link
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| match c {
                    Component::Normal(c) => Some(c.to_os_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    for component in Path::new(&target).components() {
        match component {
            Component::Normal(c) => depth.push(c.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth.pop().is_none() {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// Checks an extracted or checked-out tree: every symlink must stay inside.
pub fn check_tree(dir: &Path) -> Result<(), Unsafe> {
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if !entry.path_is_symlink() {
            continue;
        }
        let rel = entry.path().strip_prefix(dir).unwrap_or(entry.path());
        let target = std::fs::read_link(entry.path())
            .map(|t| t.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !link_stays_inside(rel, &target) {
            return Err(Unsafe {
                path: rel.to_string_lossy().replace('\\', "/"),
                reason: "symlink points outside the package",
            });
        }
    }
    Ok(())
}

/// Extracts a `.tar` stream into `dest` with the safety rules. Symlinks are
/// recreated only where the platform allows it; otherwise skipped.
pub fn extract_tar(reader: impl Read, dest: &Path) -> Result<(), ExtractError> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(ExtractError::Io)? {
        let mut entry = entry.map_err(ExtractError::Io)?;
        let name = entry
            .path()
            .map_err(ExtractError::Io)?
            .to_string_lossy()
            .into_owned();
        // `git archive` adds a global pax header.
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        let rel = entry_path(&name).map_err(ExtractError::Unsafe)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&rel);
        if kind.is_dir() {
            std::fs::create_dir_all(&out).map_err(ExtractError::Io)?;
        } else if kind.is_symlink() || kind.is_hard_link() {
            let target = entry
                .link_name()
                .map_err(ExtractError::Io)?
                .map(|t| t.to_string_lossy().into_owned())
                .unwrap_or_default();
            let inside = if kind.is_hard_link() {
                entry_path(&target).is_ok()
            } else {
                link_stays_inside(&rel, &target)
            };
            if !inside {
                return Err(ExtractError::Unsafe(Unsafe {
                    path: name,
                    reason: if kind.is_hard_link() {
                        "hardlink points outside the package"
                    } else {
                        "symlink points outside the package"
                    },
                }));
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(ExtractError::Io)?;
            }
            if kind.is_hard_link() {
                let source = dest.join(entry_path(&target).map_err(ExtractError::Unsafe)?);
                std::fs::copy(source, &out).map_err(ExtractError::Io)?;
            } else {
                make_symlink(&target, &out);
            }
        } else if kind.is_file() || kind == tar::EntryType::Continuous {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(ExtractError::Io)?;
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(ExtractError::Io)?;
            std::fs::write(&out, bytes).map_err(ExtractError::Io)?;
        }
    }
    Ok(())
}

/// Extracts a `.zip` archive into `dest` with the safety rules.
pub fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), ExtractError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| ExtractError::Io(std::io::Error::other(e)))?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| ExtractError::Io(std::io::Error::other(e)))?;
        let name = file
            .name()
            .map_err(|e| ExtractError::Io(std::io::Error::other(e)))?
            .into_owned();
        let rel = entry_path(&name).map_err(ExtractError::Unsafe)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&rel);
        if file.is_dir() {
            std::fs::create_dir_all(&out).map_err(ExtractError::Io)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(ExtractError::Io)?;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).map_err(ExtractError::Io)?;
        if file.is_symlink() {
            let target = String::from_utf8_lossy(&contents).into_owned();
            if !link_stays_inside(&rel, &target) {
                return Err(ExtractError::Unsafe(Unsafe {
                    path: name,
                    reason: "symlink points outside the package",
                }));
            }
            make_symlink(&target, &out);
            continue;
        }
        std::fs::write(&out, contents).map_err(ExtractError::Io)?;
    }
    Ok(())
}

#[cfg(unix)]
fn make_symlink(target: &str, link: &Path) {
    let _ = std::os::unix::fs::symlink(target, link);
}

#[cfg(not(unix))]
fn make_symlink(_target: &str, _link: &Path) {}

#[derive(Debug)]
pub enum ExtractError {
    Io(std::io::Error),
    Unsafe(Unsafe),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_paths_reject_escapes() {
        assert!(entry_path("a/b.lua").is_ok());
        assert!(entry_path("./a/b.lua").is_ok());
        assert_eq!(entry_path("../x").unwrap_err().reason, "path contains `..`");
        assert_eq!(
            entry_path("a/../../x").unwrap_err().reason,
            "path contains `..`"
        );
        assert_eq!(
            entry_path("/etc/passwd").unwrap_err().reason,
            "absolute path"
        );
        assert_eq!(entry_path("C:\\x").unwrap_err().reason, "absolute path");
    }

    #[test]
    fn links_must_stay_inside() {
        assert!(link_stays_inside(Path::new("a/link"), "../b.lua"));
        assert!(link_stays_inside(Path::new("a/link"), "c/d.lua"));
        assert!(!link_stays_inside(Path::new("a/link"), "../../x"));
        assert!(!link_stays_inside(Path::new("link"), "/etc/passwd"));
    }

    #[test]
    fn tar_with_escaping_entry_is_rejected() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_entry_type(tar::EntryType::Regular);
        // `set_path` refuses `..`, so write the name bytes directly.
        header.as_old_mut().name[..9].copy_from_slice(b"../evil.x");
        header.set_cksum();
        builder.append(&header, &b"x"[..]).unwrap();
        let bytes = builder.into_inner().unwrap();
        let dir = tempfile::tempdir().unwrap();
        match extract_tar(&bytes[..], dir.path()) {
            Err(ExtractError::Unsafe(u)) => assert_eq!(u.reason, "path contains `..`"),
            other => panic!("expected an unsafe entry, got {other:?}"),
        }
    }
}
