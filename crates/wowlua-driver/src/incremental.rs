//! What `wowlua dev` keeps between builds of one project (#7): each file's
//! compiled output, by its source, `FileId`, bytes and context. Every
//! rebuild gives what a fresh build of the same tree gives; the cache only
//! skips compiling a file again where that would give the same.

use crate::compile::{CacheEntry, FlavorCache};
use std::collections::HashMap;
use std::sync::Arc;
use wowlua_diagnostics::Diagnostic;
use wowlua_sema::usage::FileUsage;
use wowlua_span::{FileId, SourceDb};

/// The cache of one `wowlua dev` session. One-shot commands never make one.
#[derive(Default)]
pub struct BuildCache {
    /// The hash of the build-wide inputs of the last build; another value
    /// empties the cache.
    config: Option<blake3::Hash>,
    /// Counts builds. Entries the latest compile did not use are dropped.
    generation: u64,
    /// Compiled files by `Source::rel`.
    files: HashMap<String, Vec<CacheEntry>>,
    /// The Lua 5.1 check of plain Lua sources by `Source::rel`.
    lua_checks: HashMap<String, LuaCheck>,
    /// The hashes of the files' bytes in this build, by `Source::rel`.
    hashes: HashMap<String, blake3::Hash>,
    /// The game's API files, parsed: by path, the `FileId` and whether
    /// tokens were kept.
    pub(crate) api: HashMap<&'static str, (FileId, bool, Arc<crate::compile::ParsedDeclaration>)>,
    /// The repository's state, read by the first build.
    pub(crate) vcs: Option<Option<crate::vcs::VcsInfo>>,
    /// Each source's load-order usage, by `Source::rel`.
    usages: HashMap<String, crate::compile::UsageEntry>,
    /// Modules' exports (decision T375).
    modules: wowlua_sema::check::portable::ModuleCache,
    /// Plain Lua files a rebuild does not parse again, by `Source::rel`.
    fronts: HashMap<String, FrontEntry>,
    /// What the last build wrote (stage 4).
    pub(crate) written: Option<Written>,
    /// What the last build took from the cache.
    pub stats: CacheStats,
    /// The hash of the modules' bytes as last read from or written to disk.
    persisted: Option<blake3::Hash>,
}

/// What `luacheck::check` gave for a file: it reads only the bytes, and the
/// path and text of its `FileId`.
struct LuaCheck {
    file: FileId,
    hash: blake3::Hash,
    result: Option<Diagnostic>,
    generation: u64,
}

/// An unchanged plain Lua source a rebuild does not parse again (stage 3):
/// one that parsed cleanly and that is neither type-checked nor linted.
/// The build reads nothing else of its parse but its load-order usage
/// (which is the same in every flavor: it reads only the flavor table's
/// deferred-call markers) and `--!nolint`; lowering parses it again where
/// a flavor's compile misses.
struct FrontEntry {
    file: FileId,
    hash: blake3::Hash,
    usage: Arc<FileUsage>,
    nolint: bool,
    generation: u64,
}

/// What a build took from the cache: per file and flavor compiled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: usize,
    pub misses: usize,
    /// Plain Lua sources whose Lua 5.1 check was reused.
    pub lua_checks: usize,
    /// Plain Lua sources whose type check (`check.lua`) a later flavor
    /// took from an earlier one of the same build.
    pub type_checks: usize,
    /// Plain Lua sources that were not parsed again.
    pub summarized: usize,
    /// The build-wide inputs changed, so the cache was emptied.
    pub cleared: bool,
    /// Modules whose exports a focused check took instead of checking them
    /// (decision T375).
    pub modules: usize,
}

impl BuildCache {
    /// Starts a build with these build-wide inputs.
    pub(crate) fn begin(&mut self, config: blake3::Hash) {
        self.stats = CacheStats::default();
        if self.config != Some(config) {
            self.stats.cleared = self.config.is_some();
            self.files.clear();
            self.usages.clear();
            self.modules = Default::default();
            self.lua_checks.clear();
            self.fronts.clear();
            self.written = None;
            self.config = Some(config);
        }
        self.hashes.clear();
        self.generation += 1;
    }

    /// The usage and `--!nolint` of an unchanged plain Lua source kept by
    /// an earlier build, when its path, `FileId` and bytes are the same.
    pub(crate) fn front(
        &mut self,
        rel: &str,
        file: FileId,
        bytes: &[u8],
    ) -> Option<(Arc<FileUsage>, bool)> {
        let hash = self.hash(rel, bytes);
        let generation = self.generation;
        let e = self.fronts.get_mut(rel)?;
        if e.file != file || e.hash != hash {
            return None;
        }
        e.generation = generation;
        self.stats.summarized += 1;
        Some((e.usage.clone(), e.nolint))
    }

    /// The hash of a source's bytes in this build.
    pub(crate) fn source_hash(&mut self, rel: &str, bytes: &[u8]) -> blake3::Hash {
        self.hash(rel, bytes)
    }

    /// Keeps a cleanly parsed plain Lua source for later builds.
    pub(crate) fn keep_front(
        &mut self,
        rel: &str,
        file: FileId,
        hash: blake3::Hash,
        usage: Arc<FileUsage>,
        nolint: bool,
    ) {
        let generation = self.generation;
        self.fronts.insert(
            rel.to_string(),
            FrontEntry {
                file,
                hash,
                usage,
                nolint,
                generation,
            },
        );
    }

    fn hash(&mut self, rel: &str, bytes: &[u8]) -> blake3::Hash {
        if let Some(hash) = self.hashes.get(rel) {
            return *hash;
        }
        let hash = blake3::hash(bytes);
        self.hashes.insert(rel.to_string(), hash);
        hash
    }

    /// `luacheck::check` of a source, taken from an earlier build when its
    /// path, `FileId` and bytes are the same.
    pub(crate) fn lua_check(
        &mut self,
        db: &SourceDb,
        rel: &str,
        file: FileId,
        bytes: &[u8],
    ) -> Option<Diagnostic> {
        self.lua_check_with(db, rel, file, bytes, None)
    }

    /// Whether the Lua 5.1 check of a source is kept (without using it).
    pub(crate) fn has_lua_check(&mut self, rel: &str, file: FileId, bytes: &[u8]) -> bool {
        let hash = self.hash(rel, bytes);
        self.lua_checks
            .get(rel)
            .is_some_and(|c| c.file == file && c.hash == hash)
    }

    /// [`BuildCache::lua_check`], with the check computed already when it
    /// is not kept (`computed`, by a parallel job).
    pub(crate) fn lua_check_with(
        &mut self,
        db: &SourceDb,
        rel: &str,
        file: FileId,
        bytes: &[u8],
        computed: Option<Option<Diagnostic>>,
    ) -> Option<Diagnostic> {
        let hash = self.hash(rel, bytes);
        let generation = self.generation;
        if let Some(c) = self.lua_checks.get_mut(rel)
            && c.file == file
            && c.hash == hash
        {
            c.generation = generation;
            self.stats.lua_checks += 1;
            // Debug builds (and so the tests) check it again and compare.
            debug_assert_eq!(
                format!("{:?}", c.result),
                format!("{:?}", crate::luacheck::check(db, file, bytes)),
                "{rel} is reused but checks differently"
            );
            return c.result.clone();
        }
        let result = computed.unwrap_or_else(|| crate::luacheck::check(db, file, bytes));
        self.lua_checks.insert(
            rel.to_string(),
            LuaCheck {
                file,
                hash,
                result: result.clone(),
                generation,
            },
        );
        result
    }

    /// The compile cache of this build, holding the kept entries.
    pub(crate) fn lend(&mut self) -> FlavorCache {
        FlavorCache::kept(
            std::mem::take(&mut self.files),
            std::mem::take(&mut self.hashes),
            std::mem::take(&mut self.usages),
            {
                let mut modules = std::mem::take(&mut self.modules);
                modules.hits = 0;
                modules.misses = 0;
                modules
            },
            self.generation,
        )
    }

    /// Takes the entries back after a build that compiled part of the
    /// package (the language server's first pass): those it did not touch
    /// stay, for the next full build to use or drop.
    pub(crate) fn keep_partial(&mut self, cache: FlavorCache) {
        self.stats.hits += cache.hits;
        self.stats.misses += cache.misses;
        self.files = cache.entries;
        self.usages = cache.usages;
        self.stats.modules += cache.modules.hits;
        self.modules = cache.modules;
    }

    /// Takes the entries back after compiling, without those this build
    /// did not use: one per file and context of the latest build.
    pub(crate) fn keep(&mut self, cache: FlavorCache) {
        self.stats.hits += cache.hits;
        self.stats.misses += cache.misses;
        self.stats.type_checks += cache.checks.hits;
        let generation = self.generation;
        let mut files = cache.entries;
        files.retain(|_, entries| {
            entries.retain(|e| e.generation == generation);
            !entries.is_empty()
        });
        self.files = files;
        let mut usages = cache.usages;
        usages.retain(|_, u| u.generation == generation);
        self.usages = usages;
        self.stats.modules += cache.modules.hits;
        self.modules = cache.modules;
        self.lua_checks.retain(|_, c| c.generation == generation);
        self.fronts.retain(|_, f| f.generation == generation);
    }
}

/// The start of a module cache file: what it is and its format.
const MODULES_MAGIC: &[u8; 8] = b"WLMODS01";

/// What wrote a module cache file: this wowlua, down to its executable
/// (a rebuilt wowlua of the same version reads none of another's).
fn identity() -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    if let Ok(meta) = std::env::current_exe().and_then(std::fs::metadata) {
        h.update(&meta.len().to_le_bytes());
        if let Ok(t) = meta.modified()
            && let Ok(since) = t.duration_since(std::time::UNIX_EPOCH)
        {
            h.update(&since.as_nanos().to_le_bytes());
        }
    }
    h.finalize()
}

/// Where the language server keeps a package's module cache between runs
/// (spec compiler §13.9 Phase 2): `$WOWLUA_HOME/cache/check/<hash of the
/// manifest's path>`; `None` with `WOWLUA_NO_CHECK_CACHE` set (to anything
/// but `0`) or without a wowlua home.
pub fn module_cache_path(manifest: &std::path::Path) -> Option<std::path::PathBuf> {
    if std::env::var_os("WOWLUA_NO_CHECK_CACHE").is_some_and(|v| !v.is_empty() && v != "0") {
        return None;
    }
    let cache = wowlua_pkg::Cache::new()?;
    Some(module_cache_in(&cache.root.join("check"), manifest))
}

/// The module cache file of `manifest` in `dir`.
pub fn module_cache_in(dir: &std::path::Path, manifest: &std::path::Path) -> std::path::PathBuf {
    let name = blake3::hash(manifest.to_string_lossy().as_bytes()).to_hex();
    dir.join(&name.as_str()[..32])
}

impl BuildCache {
    /// Reads the modules' exports a language server kept on disk for this
    /// package (`path`), into a cache that has built nothing yet: the next
    /// build takes them when its build-wide inputs are the ones they were
    /// checked with, and drops them otherwise. Whether it read any.
    pub fn load_modules(&mut self, path: &std::path::Path) -> bool {
        if self.config.is_some() {
            return false;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        let head = MODULES_MAGIC.len() + 64;
        if bytes.len() < head
            || &bytes[..MODULES_MAGIC.len()] != MODULES_MAGIC
            || bytes[MODULES_MAGIC.len()..MODULES_MAGIC.len() + 32] != *identity().as_bytes()
        {
            return false;
        }
        let mut config = [0u8; 32];
        config.copy_from_slice(&bytes[MODULES_MAGIC.len() + 32..head]);
        let Some(modules) = wowlua_sema::check::portable::ModuleCache::from_bytes(&bytes[head..])
        else {
            return false;
        };
        self.persisted = Some(blake3::hash(&bytes[head..]));
        self.config = Some(blake3::Hash::from_bytes(config));
        self.modules = modules;
        // (used now: kept from pruning)
        touch(path);
        true
    }

    /// Writes the modules' exports to `path` for a later language server,
    /// unless they are as read or last written. Whether it wrote them.
    pub fn save_modules(&mut self, path: &std::path::Path) -> std::io::Result<bool> {
        let Some(config) = self.config else {
            return Ok(false);
        };
        let body = self.modules.to_bytes();
        let hash = blake3::hash(&body);
        if self.persisted == Some(hash) {
            return Ok(false);
        }
        let mut bytes = Vec::with_capacity(MODULES_MAGIC.len() + 64 + body.len());
        bytes.extend_from_slice(MODULES_MAGIC);
        bytes.extend_from_slice(identity().as_bytes());
        bytes.extend_from_slice(config.as_bytes());
        bytes.extend_from_slice(&body);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // (written whole, then renamed: another server reads all or none)
        let partial = path.with_extension(format!("{}.partial", std::process::id()));
        std::fs::write(&partial, &bytes)?;
        if let Err(e) = std::fs::rename(&partial, path) {
            let _ = std::fs::remove_file(&partial);
            return Err(e);
        }
        self.persisted = Some(hash);
        if let Some(dir) = path.parent() {
            prune(dir);
        }
        Ok(true)
    }

    /// The modules whose exports are kept.
    pub fn kept_modules(&self) -> usize {
        self.modules.len()
    }
}

/// How long a package's module cache is kept unused.
const MODULES_UNUSED: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// Marks a module cache file as used now, at most once a day (its
/// modification time is its last use).
fn touch(path: &std::path::Path) {
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    let old = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age >= day));
    if old && let Ok(file) = std::fs::File::options().append(true).open(path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

/// Removes the module caches of packages no server checked for 30 days
/// (and files a server left half-written), once per process.
fn prune(dir: &std::path::Path) {
    static PRUNED: std::sync::Once = std::sync::Once::new();
    PRUNED.call_once(|| {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let partial = path.extension().is_some_and(|e| e == "partial");
            let limit = if partial {
                std::time::Duration::from_secs(60 * 60)
            } else {
                MODULES_UNUSED
            };
            let unused = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age >= limit));
            if unused {
                let _ = std::fs::remove_file(&path);
            }
        }
    });
}

/// The hash of what every file of a build depends on: the request, the
/// texts loaded before the build (manifests), the lockfile, the resolved
/// dependencies and the workspace. A file's own bytes, `FileId` and
/// context are compared per file.
pub(crate) fn config_hash(
    db: &SourceDb,
    project: &crate::Project,
    request: &crate::BuildRequest,
) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    let mut add = |part: &[u8]| {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part);
    };
    add(env!("CARGO_PKG_VERSION").as_bytes());
    add(format!("{request:?}").as_bytes());
    for (_, file) in db.files() {
        add(file.path().to_string_lossy().as_bytes());
        add(file.text().as_bytes());
    }
    add(format!("{:?}", project.lock).as_bytes());
    add(format!("{:?}", project.interfaces).as_bytes());
    add(format!("{:?}", project.deps).as_bytes());
    add(format!("{:?}", project.workspace_root).as_bytes());
    add(format!("{:?}", project.workspace_addons).as_bytes());
    add(format!("{:?}", project.external_addons).as_bytes());
    h.finalize()
}

/// A file on disk as a build left it: its length and modification time.
/// A file that still has them is the one the build wrote (nothing else
/// writes into the output folder).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp(u64, std::time::SystemTime);

fn stamp(path: &std::path::Path) -> Option<Stamp> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    meta.is_file().then_some(())?;
    Some(Stamp(meta.len(), meta.modified().ok()?))
}

/// What a `wowlua dev` build wrote: the AddOn folder's files (a written
/// file by its bytes' hash, a copied one by its source) and the source maps.
pub(crate) struct Written {
    dir: std::path::PathBuf,
    files: HashMap<String, (Kept, Stamp)>,
    maps_root: std::path::PathBuf,
    maps: HashMap<String, (Arc<wowlua_luair::sourcemap::EncodedMap>, Stamp)>,
}

#[derive(PartialEq, Eq)]
enum Kept {
    Bytes(blake3::Hash),
    Copy(std::path::PathBuf, Stamp),
}

/// `write_tree` and `write_maps` writing only what changed since the last
/// build (#7 stage 4). The folder is still staged and swapped in whole; a
/// file whose bytes (or copied source) are the same, and that is on disk as
/// the last build left it, is hard-linked from the old folder instead of
/// written. A map that is the same (`Arc`) and on disk as left stays; the
/// others are written, and maps of files no longer built are removed. The
/// result is what writing everything gives.
pub(crate) fn write_changed(
    dir: &std::path::Path,
    out: &std::collections::BTreeMap<String, crate::build::Content>,
    addon: &str,
    maps: &std::collections::BTreeMap<String, Arc<wowlua_luair::sourcemap::EncodedMap>>,
    previous: Option<&Written>,
) -> std::io::Result<Written> {
    use crate::build::Content;
    let previous = previous.filter(|p| p.dir == dir);
    let staged = crate::fsutil::temp_sibling(dir)?;
    let mut files = HashMap::new();
    for (path, content) in out {
        let dest = staged.join(path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let kept = match content {
            Content::Bytes(bytes) => Kept::Bytes(blake3::hash(bytes)),
            Content::Copy(from) => match stamp(from) {
                Some(s) => Kept::Copy(from.clone(), s),
                None => {
                    // A source without a stamp is copied as always.
                    std::fs::copy(from, &dest)?;
                    continue;
                }
            },
        };
        let old = dir.join(path);
        let linked = previous
            .and_then(|p| p.files.get(path))
            .is_some_and(|(k, s)| *k == kept && stamp(&old) == Some(*s))
            && std::fs::hard_link(&old, &dest).is_ok();
        if !linked {
            match content {
                Content::Bytes(bytes) => std::fs::write(&dest, bytes)?,
                Content::Copy(from) => {
                    std::fs::copy(from, &dest)?;
                }
            }
        }
        if let Some(s) = stamp(&dest) {
            files.insert(path.clone(), (kept, s));
        }
    }
    crate::fsutil::replace_dir(&staged, dir)?;
    let maps_root = dir
        .parent()
        .map(|p| p.join(".maps").join(addon))
        .unwrap_or_default();
    let Some(previous) = previous.filter(|p| p.maps_root == maps_root) else {
        crate::build::write_maps(dir, addon, maps)?;
        let maps = maps
            .iter()
            .filter_map(|(rel, m)| Some((rel.clone(), (m.clone(), stamp(&maps_root.join(rel))?))))
            .collect();
        return Ok(Written {
            dir: dir.to_path_buf(),
            files,
            maps_root,
            maps,
        });
    };
    let mut kept_maps = HashMap::new();
    for (rel, map) in maps {
        let path = maps_root.join(rel);
        let same = previous
            .maps
            .get(rel)
            .is_some_and(|(m, s)| Arc::ptr_eq(m, map) && stamp(&path) == Some(*s));
        if !same {
            crate::fsutil::write_file(&path, map.to_json().as_bytes())?;
        }
        if let Some(s) = stamp(&path) {
            kept_maps.insert(rel.clone(), (map.clone(), s));
        }
    }
    // Maps of files no longer built, and anything else there, go.
    remove_others(&maps_root, &maps_root, maps)?;
    Ok(Written {
        dir: dir.to_path_buf(),
        files,
        maps_root,
        maps: kept_maps,
    })
}

/// Removes the files below `dir` that are not in `keep` (by path relative
/// to `root`, `/` separators), and directories left empty.
fn remove_others(
    root: &std::path::Path,
    dir: &std::path::Path,
    keep: &std::collections::BTreeMap<String, Arc<wowlua_luair::sourcemap::EncodedMap>>,
) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            remove_others(root, &path, keep)?;
            let _ = std::fs::remove_dir(&path);
        } else if crate::fsutil::relative_slash(&path, root)
            .is_none_or(|rel| !keep.contains_key(&rel))
        {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}
