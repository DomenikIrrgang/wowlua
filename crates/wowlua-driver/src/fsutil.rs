//! File-system helpers: atomic directory replacement (spec §13.12), copies,
//! and links (symlinks on Unix, junctions on Windows).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Suffix of temporary directories while a replacement is in progress.
const TMP_SUFFIX: &str = ".wowlua-tmp";
/// Suffix of the old directory during a replacement on Windows.
const OLD_SUFFIX: &str = ".wowlua-old";

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// A fresh temporary directory next to `target`, for building a replacement.
pub fn temp_sibling(target: &Path) -> io::Result<PathBuf> {
    let tmp = sibling(target, TMP_SUFFIX);
    remove_any(&tmp)?;
    if let Some(parent) = tmp.parent() {
        crate::host::create_dir_all(parent)?;
    }
    crate::host::create_dir_all(&tmp)?;
    Ok(tmp)
}

/// Replaces `target` with the directory `staged` as atomically as the
/// platform allows. Leftovers of interrupted replacements are cleaned up.
pub fn replace_dir(staged: &Path, target: &Path) -> io::Result<()> {
    let old = sibling(target, OLD_SUFFIX);
    remove_any(&old)?;
    if is_link(target) {
        remove_any(target)?;
    }
    if cfg!(windows) || crate::host::exists(target) {
        if crate::host::exists(target) {
            crate::host::rename(target, &old)?;
        }
        crate::host::rename(staged, target)?;
        remove_any(&old)?;
    } else {
        crate::host::rename(staged, target)?;
    }
    Ok(())
}

/// Removes a file, a directory tree or a link (without following it).
pub fn remove_any(path: &Path) -> io::Result<()> {
    if crate::host::installed().is_some() {
        return match crate::host::kind(path) {
            Some(crate::host::Kind::Dir) => crate::host::remove_dir_all(path),
            Some(crate::host::Kind::File) => crate::host::remove_file(path),
            None => Ok(()),
        };
    }
    #[allow(clippy::disallowed_methods)] // (no host installed: the disk's)
    let Ok(meta) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if meta.file_type().is_symlink() || is_junction(path) {
        // Directory symlinks on Windows and junctions are removed as directories.
        #[allow(clippy::disallowed_methods)]
        return crate::host::remove_file(path).or_else(|_| fs::remove_dir(path));
    }
    if meta.is_dir() {
        crate::host::remove_dir_all(path)
    } else {
        crate::host::remove_file(path)
    }
}

/// Copies a directory tree. Links inside it are copied as their targets.
pub fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    crate::host::create_dir_all(to)?;
    if crate::host::installed().is_some() {
        for (path, kind) in crate::host::walk_entries(from, true, &|_| true) {
            let dest = to.join(path.strip_prefix(from).unwrap_or(&path));
            match kind {
                crate::host::Kind::Dir => crate::host::create_dir_all(&dest)?,
                crate::host::Kind::File => drop(crate::host::copy(&path, &dest)?),
            }
        }
        return Ok(());
    }
    #[allow(clippy::disallowed_methods)] // (no host installed: the disk's)
    for entry in walkdir::WalkDir::new(from).min_depth(1).follow_links(true) {
        let entry = entry.map_err(io::Error::other)?;
        let relative = entry
            .path()
            .strip_prefix(from)
            .expect("walkdir yields children");
        let dest = to.join(relative);
        if entry.file_type().is_dir() {
            crate::host::create_dir_all(&dest)?;
        } else {
            crate::host::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

pub fn is_link(path: &Path) -> bool {
    if crate::host::installed().is_some() {
        return false;
    }
    #[allow(clippy::disallowed_methods)] // (no host installed: the disk's)
    let link = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink());
    link || is_junction(path)
}

#[cfg(windows)]
fn is_junction(path: &Path) -> bool {
    junction::exists(path).unwrap_or(false)
}

#[cfg(not(windows))]
fn is_junction(_path: &Path) -> bool {
    false
}

/// Where a link points, if `path` is a link.
pub fn link_target(path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    if is_junction(path) {
        return junction::get_target(path).ok();
    }
    fs::read_link(path).ok()
}

/// Creates a directory link at `link` pointing to `target`.
pub fn create_dir_link(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        junction::create(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this system has no directory links",
        ))
    }
}

/// Writes a file, creating parent directories.
pub fn write_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        crate::host::create_dir_all(parent)?;
    }
    crate::host::write(path, contents)
}

/// `a/b/c` with `/` separators, relative to `base`.
pub fn relative_slash(path: &Path, base: &Path) -> Option<String> {
    let relative = path.strip_prefix(base).ok()?;
    Some(
        relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_dir_swaps_contents() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("out");
        crate::host::create_dir_all(&target).unwrap();
        crate::host::write(target.join("old.txt"), "old").unwrap();

        let staged = temp_sibling(&target).unwrap();
        crate::host::write(staged.join("new.txt"), "new").unwrap();
        replace_dir(&staged, &target).unwrap();

        assert!(target.join("new.txt").exists());
        assert!(!target.join("old.txt").exists());
        assert!(!staged.exists());
    }

    #[test]
    fn links_are_replaced_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        crate::host::create_dir_all(&real).unwrap();
        crate::host::write(real.join("keep.txt"), "keep").unwrap();
        let link = root.path().join("link");
        create_dir_link(&real, &link).unwrap();
        assert!(is_link(&link));
        assert_eq!(link_target(&link).unwrap(), real);

        remove_any(&link).unwrap();
        assert!(!link.exists());
        assert!(real.join("keep.txt").exists());
    }

    #[test]
    fn relative_paths_use_slashes() {
        let base = Path::new("/a");
        assert_eq!(
            relative_slash(Path::new("/a/b/c.lua"), base).unwrap(),
            "b/c.lua"
        );
    }
}
