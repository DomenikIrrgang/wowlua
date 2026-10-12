//! Package checksums (spec §10.6): SHA-256 over the normalized file tree:
//! sorted `/`-separated paths and their contents; the executable bit and
//! timestamps are ignored. Version-control metadata is not part of it.

use sha2::{Digest, Sha256};
use std::path::Path;

/// Directories never part of a package tree.
pub const VCS_DIRS: &[&str] = &[".git", ".svn", ".hg"];

/// `sha256-<hex>` of the tree under `dir`.
pub fn tree(dir: &Path) -> std::io::Result<String> {
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            !(e.file_type().is_dir()
                && VCS_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
    {
        let entry = entry.map_err(std::io::Error::other)?;
        if entry.file_type().is_dir() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(dir)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        files.push((rel, entry.path().to_path_buf()));
    }
    files.sort();
    let mut hasher = Sha256::new();
    for (rel, path) in files {
        let contents = if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            std::fs::read_link(&path)?
                .to_string_lossy()
                .replace('\\', "/")
                .into_bytes()
        } else {
            std::fs::read(&path)?
        };
        hasher.update((rel.len() as u64).to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update((contents.len() as u64).to_le_bytes());
        hasher.update(&contents);
    }
    Ok(format!("sha256-{}", hex(&hasher.finalize())))
}

/// SHA-256 of bytes, as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_ignores_vcs_dirs_and_depends_on_paths_and_contents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.lua"), "x").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b.lua"), "y").unwrap();
        let first = tree(dir.path()).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "z").unwrap();
        assert_eq!(tree(dir.path()).unwrap(), first);
        std::fs::write(dir.path().join("sub/b.lua"), "Y").unwrap();
        assert_ne!(tree(dir.path()).unwrap(), first);
        assert!(first.starts_with("sha256-"));
    }
}
