//! A persistent cache of Lua 5.1 compile results (#50, spec §10.7):
//! `$WOWLUA_HOME/cache/compile/<hash>`. Compiling a file with the embedded Lua
//! only to learn whether it compiles (and Lua's message when it does not)
//! is a large part of a build; the result depends only on the bytes, the
//! chunk name and the Lua build, so a later build of the same bytes takes
//! it from here.
//!
//! The CLI turns the cache on ([`enable`]); a library build of the driver
//! never reads or writes it unless its caller does. An entry is created once
//! and records its own length, so one another build is still writing, or
//! one cut short, is a miss (and a damaged one is removed and written
//! again); concurrent builds write the same bytes for the same key. The
//! cache stays under its size by removing the least recently used entries
//! (an entry's modification time is its last use), at most once a day.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// Part of every key: entries of another wowlua or Lua build never match.
/// Bump the number when the embedded Lua or how it is loaded changes.
const BUILD: &str = concat!("wowlua ", env!("CARGO_PKG_VERSION"), " lua5.1 compile 1");

/// The default size limit: 256 MiB.
const DEFAULT_LIMIT: u64 = 256 << 20;

/// How often the cache is brought back under its size limit.
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

static CACHE: OnceLock<CompileCache> = OnceLock::new();

/// A compile cache in a directory, kept under `limit` bytes.
#[derive(Clone, Debug)]
pub struct CompileCache {
    dir: PathBuf,
    limit: u64,
}

/// What compiling gave: compiled, or Lua's message.
pub(crate) type Outcome = Result<(), String>;

impl CompileCache {
    pub fn new(dir: impl Into<PathBuf>, limit: u64) -> CompileCache {
        CompileCache {
            dir: dir.into(),
            limit,
        }
    }

    /// The cache under `$WOWLUA_HOME/cache/compile/`, unless
    /// `WOWLUA_NO_COMPILE_CACHE` is set (to anything but `0`) or
    /// `build.compile-cache` is `false` in `$WOWLUA_HOME/config.json`; its
    /// limit is `build.compile-cache-size` (MiB), else 256 MiB.
    pub fn from_env() -> Option<CompileCache> {
        if std::env::var_os("WOWLUA_NO_COMPILE_CACHE").is_some_and(|v| !v.is_empty() && v != "0") {
            return None;
        }
        let cache = wowlua_pkg::Cache::new()?;
        let config: Option<serde_json::Value> = crate::install::wowlua_home()
            .and_then(|home| std::fs::read_to_string(home.join("config.json")).ok())
            .and_then(|text| serde_json::from_str(&text).ok());
        let setting = |key: &str| config.as_ref().and_then(|c| c.pointer(key));
        if setting("/build/compile-cache").and_then(|v| v.as_bool()) == Some(false) {
            return None;
        }
        let limit = setting("/build/compile-cache-size")
            .and_then(|v| v.as_u64())
            .map_or(DEFAULT_LIMIT, |mib| mib.saturating_mul(1 << 20));
        Some(CompileCache::new(cache.root.join("compile"), limit))
    }

    /// The entry of `code` compiled as `name` (`None`: Lua's default name).
    fn path(&self, name: Option<&str>, code: &[u8]) -> PathBuf {
        let mut hasher = blake3::Hasher::new();
        hasher.update(BUILD.as_bytes());
        hasher.update(&[0]);
        match name {
            Some(name) => {
                hasher.update(&[1]);
                hasher.update(&(name.len() as u64).to_le_bytes());
                hasher.update(name.as_bytes());
            }
            None => {
                hasher.update(&[0]);
            }
        }
        hasher.update(code);
        self.dir.join(hasher.finalize().to_hex().as_str())
    }

    /// The stored outcome, if there is a readable one; a hit marks the
    /// entry as used now, unless it was within the pruning interval (which
    /// is all the precision pruning needs).
    pub(crate) fn get(&self, name: Option<&str>, code: &[u8]) -> Option<Outcome> {
        use std::io::Read;
        let path = self.path(name, code);
        let mut file = std::fs::File::open(&path).ok()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).ok()?;
        let Some(outcome) = decode(&bytes) else {
            // Cut short, or still being written: written again later.
            drop(file);
            let _ = std::fs::remove_file(&path);
            return None;
        };
        let stale = file
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age >= PRUNE_EVERY));
        if stale && let Ok(file) = std::fs::File::options().append(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(outcome)
    }

    /// Stores an outcome; a failure to write only loses the entry.
    pub(crate) fn put(&self, name: Option<&str>, code: &[u8], outcome: &Outcome) {
        use std::io::Write;
        let path = self.path(name, code);
        let create = || {
            std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(&path)
        };
        let mut file = create();
        if file
            .as_ref()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            // The first entry: the directory is made once.
            file = std::fs::create_dir_all(&self.dir).and_then(|()| create());
        }
        // An entry that exists already holds the same outcome.
        if let Ok(mut file) = file {
            let _ = file.write_all(&encode(outcome));
        }
    }

    /// Removes the least recently used entries while the cache is over its
    /// limit, down to three quarters of it. Returns the bytes removed.
    pub fn prune(&self) -> u64 {
        let mut entries: Vec<(SystemTime, u64, PathBuf)> = walkdir::WalkDir::new(&self.dir)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .flatten()
            .filter(|e| {
                e.file_type().is_file() && !e.file_name().to_string_lossy().starts_with('.')
            })
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                Some((meta.modified().ok()?, meta.len(), e.into_path()))
            })
            .collect();
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        if total <= self.limit {
            return 0;
        }
        entries.sort_by_key(|e| e.0);
        let target = self.limit / 4 * 3;
        let mut removed = 0;
        for (_, len, path) in entries {
            if total <= target {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total -= len;
                removed += len;
            }
        }
        removed
    }

    /// [`CompileCache::prune`], unless the cache was pruned within a day.
    fn prune_now_and_then(&self) {
        let marker = self.dir.join(".pruned");
        let recent = std::fs::metadata(&marker)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age < PRUNE_EVERY));
        if recent {
            return;
        }
        self.prune();
        if std::fs::create_dir_all(&self.dir).is_ok() {
            let _ = std::fs::write(&marker, b"");
        }
    }
}

/// Turns the cache on for this process (the CLI does at startup); the first
/// call wins. Brings it under its limit if that was not done today.
pub fn enable(cache: CompileCache) {
    cache.prune_now_and_then();
    let _ = CACHE.set(cache);
}

/// The cache, if [`enable`] turned it on.
pub(crate) fn get() -> Option<&'static CompileCache> {
    CACHE.get()
}

/// `ok`, or `error <bytes>` and the message, each line ending in `\n`.
fn encode(outcome: &Outcome) -> Vec<u8> {
    match outcome {
        Ok(()) => b"ok\n".to_vec(),
        Err(message) => format!("error {}\n{message}\n", message.len()).into_bytes(),
    }
}

/// The outcome [`encode`] wrote, if it is all there.
fn decode(bytes: &[u8]) -> Option<Outcome> {
    if bytes == b"ok\n" {
        return Some(Ok(()));
    }
    let rest = bytes.strip_prefix(b"error ")?;
    let newline = rest.iter().position(|&b| b == b'\n')?;
    let len: usize = std::str::from_utf8(&rest[..newline]).ok()?.parse().ok()?;
    let message = rest[newline + 1..].strip_suffix(b"\n")?;
    (message.len() == len).then(|| Some(Err(String::from_utf8(message.to_vec()).ok()?)))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(limit: u64) -> (tempfile::TempDir, CompileCache) {
        let dir = tempfile::tempdir().unwrap();
        let cache = CompileCache::new(dir.path().join("compile"), limit);
        (dir, cache)
    }

    /// An outcome comes back as it was stored, for the same name and bytes
    /// only.
    #[test]
    fn outcomes_come_back_for_the_same_name_and_bytes() {
        let (_dir, cache) = cache(DEFAULT_LIMIT);
        let error: Outcome = Err("=src/a.lua:3: 'end' expected".into());
        cache.put(Some("=src/a.lua"), b"x", &error);
        cache.put(None, b"y", &Ok(()));
        assert_eq!(cache.get(Some("=src/a.lua"), b"x"), Some(error));
        assert_eq!(cache.get(None, b"y"), Some(Ok(())));
        assert_eq!(cache.get(Some("=src/b.lua"), b"x"), None);
        assert_eq!(cache.get(Some("=src/a.lua"), b"x "), None);
        assert_eq!(
            cache.get(Some(""), b"y"),
            None,
            "no name is not an empty name"
        );
    }

    /// An entry that is not a stored outcome is a miss, and storing writes
    /// it again.
    #[test]
    fn unreadable_entries_are_misses() {
        let (_dir, cache) = cache(DEFAULT_LIMIT);
        let path = cache.path(None, b"z");
        std::fs::create_dir_all(&cache.dir).unwrap();
        std::fs::write(&path, b"garbage").unwrap();
        assert_eq!(cache.get(None, b"z"), None);
        cache.put(None, b"z", &Ok(()));
        assert_eq!(cache.get(None, b"z"), Some(Ok(())));
        // An error cut short.
        let full = encode(&Err("=a.lua:1: unexpected symbol".into()));
        let path = cache.path(None, b"w");
        std::fs::write(&path, &full[..full.len() - 4]).unwrap();
        assert_eq!(cache.get(None, b"w"), None);
        cache.put(None, b"w", &Err("=a.lua:1: unexpected symbol".into()));
        assert_eq!(
            cache.get(None, b"w"),
            Some(Err("=a.lua:1: unexpected symbol".into()))
        );
    }

    /// Over its limit, the least recently used entries go first.
    #[test]
    fn pruning_removes_the_least_recently_used() {
        let (_dir, cache) = cache(4 * 1000);
        let message = "m".repeat(989);
        for i in 0..8u8 {
            cache.put(None, &[i], &Err(message.clone()));
            let path = cache.path(None, &[i]);
            let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + u64::from(i));
            std::fs::File::options()
                .append(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        assert!(cache.prune() > 0);
        let kept: Vec<u8> = (0..8u8)
            .filter(|i| cache.path(None, &[*i]).exists())
            .collect();
        assert_eq!(
            kept,
            vec![5, 6, 7],
            "the newest stay, within 3/4 of the limit"
        );
    }
}
