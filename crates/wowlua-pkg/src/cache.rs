//! The global cache (spec §10.7): `$WOWLUA_HOME/cache/`, default
//! `~/.wowlua/cache/`. Every entry is created in a temporary sibling and
//! renamed into place, so concurrent builds never see half-written entries.

use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// `$WOWLUA_HOME`, default `~/.wowlua`.
pub fn wowlua_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("WOWLUA_HOME").filter(|h| !h.is_empty()) {
        return Some(plain(PathBuf::from(home)));
    }
    dirs::home_dir().map(|h| plain(h.join(".wowlua")))
}

/// Without the Windows verbatim prefix (`\\?\C:\…`), which git and svn
/// cannot write to.
pub fn plain(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest.to_string()),
        _ => path,
    }
}

/// A short, stable file-name key for a URL or other text.
pub fn key(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// A readable prefix for cache entry names: the last path segment of a URL.
pub fn label(url: &str) -> String {
    let last = url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("pkg");
    let clean: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    if clean.is_empty() {
        "pkg".into()
    } else {
        clean
    }
}

#[derive(Clone, Debug)]
pub struct Cache {
    pub root: PathBuf,
}

impl Cache {
    /// The cache under `$WOWLUA_HOME`.
    pub fn new() -> Option<Cache> {
        wowlua_home().map(|h| Cache {
            root: h.join("cache"),
        })
    }

    pub fn at(root: impl Into<PathBuf>) -> Cache {
        Cache {
            root: plain(root.into()),
        }
    }

    /// Bare git clones.
    pub fn git_dir(&self, url: &str) -> PathBuf {
        self.root
            .join("git")
            .join(format!("{}-{}", label(url), key(url)))
    }

    /// Exported checkouts: one per commit, revision or archive.
    pub fn checkout_dir(&self, name: &str) -> PathBuf {
        self.root.join("checkouts").join(name)
    }

    /// Downloaded archives by their SHA-256.
    pub fn archive_file(&self, sha256: &str) -> PathBuf {
        self.root.join("archives").join(sha256)
    }

    /// Marks an entry as used now (for `cache clean`).
    pub fn touch(&self, entry: &Path) {
        let _ = std::fs::write(stamp(entry), b"");
    }

    /// Removes entries not used for `older_than` (everything with `None`).
    /// Returns the removed entries and the bytes freed.
    pub fn clean(&self, older_than: Option<Duration>) -> io::Result<(Vec<PathBuf>, u64)> {
        let mut removed = Vec::new();
        let mut freed = 0;
        let now = SystemTime::now();
        for group in ["git", "checkouts", "archives"] {
            let dir = self.root.join(group);
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".used") || name.starts_with('.') {
                    continue;
                }
                let used = std::fs::metadata(stamp(&path))
                    .or_else(|_| std::fs::metadata(&path))
                    .and_then(|m| m.modified())
                    .unwrap_or(now);
                let stale = match older_than {
                    None => true,
                    Some(limit) => now.duration_since(used).unwrap_or_default() > limit,
                };
                if stale {
                    freed += size(&path);
                    remove(&path)?;
                    let _ = std::fs::remove_file(stamp(&path));
                    removed.push(path);
                }
            }
        }
        // Lua compile results (`compile/<hash>`) and the language server's
        // module caches (`check/<hash>`): each entry's modification time is
        // its last use.
        for group in ["compile", "check"] {
            let dir = self.root.join(group);
            for entry in walkdir::WalkDir::new(&dir)
                .min_depth(1)
                .max_depth(1)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
            {
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let stale = match older_than {
                    None => true,
                    Some(limit) => meta
                        .modified()
                        .is_ok_and(|t| now.duration_since(t).unwrap_or_default() > limit),
                };
                if stale && std::fs::remove_file(entry.path()).is_ok() {
                    freed += meta.len();
                    removed.push(entry.into_path());
                }
            }
            if older_than.is_none() {
                remove(&dir)?;
            }
        }
        Ok((removed, freed))
    }
}

fn stamp(entry: &Path) -> PathBuf {
    let mut name = entry.file_name().unwrap_or_default().to_os_string();
    name.push(".used");
    entry.with_file_name(name)
}

fn size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

pub(crate) fn remove(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            make_writable(path);
            std::fs::remove_dir_all(path)
        }
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Git marks its objects read-only, which blocks removal on Windows.
fn make_writable(path: &Path) {
    for entry in walkdir::WalkDir::new(path).into_iter().flatten() {
        if let Ok(meta) = entry.metadata() {
            let mut perms = meta.permissions();
            if perms.readonly() {
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                let _ = std::fs::set_permissions(entry.path(), perms);
            }
        }
    }
}

/// A fresh temporary sibling of `target` for staging an entry.
pub(crate) fn staging(target: &Path) -> io::Result<PathBuf> {
    let parent = target.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    for i in 0..1000u32 {
        let candidate = parent.join(format!(".{name}.tmp{}-{i}", std::process::id()));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::other("no free temporary name"))
}

/// Moves a staged entry into place. If another process won the race, its
/// entry is kept and the staged one discarded.
pub(crate) fn commit(staged: &Path, target: &Path) -> io::Result<()> {
    match std::fs::rename(staged, target) {
        Ok(()) => Ok(()),
        Err(_) if target.exists() => remove(staged),
        Err(e) => {
            let _ = remove(staged);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile results and module caches unused for longer than the limit
    /// go; `--all` takes the whole directories.
    #[test]
    fn clean_removes_old_compile_results() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let old = cache.root.join("compile/old");
        let new = cache.root.join("compile/new");
        let old_check = cache.root.join("check/old");
        let new_check = cache.root.join("check/new");
        for path in [&old, &new, &old_check, &new_check] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"ok\n").unwrap();
        }
        let long_ago = SystemTime::now() - Duration::from_secs(40 * 24 * 60 * 60);
        for path in [&old, &old_check] {
            std::fs::File::options()
                .append(true)
                .open(path)
                .unwrap()
                .set_modified(long_ago)
                .unwrap();
        }
        let (removed, _) = cache
            .clean(Some(Duration::from_secs(30 * 24 * 60 * 60)))
            .unwrap();
        assert_eq!(removed, vec![old.clone(), old_check.clone()]);
        assert!(new.exists() && new_check.exists());
        cache.clean(None).unwrap();
        assert!(!cache.root.join("compile").exists());
        assert!(!cache.root.join("check").exists());
    }
}
