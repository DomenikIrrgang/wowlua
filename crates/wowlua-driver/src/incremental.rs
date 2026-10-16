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
    /// Plain Lua files a rebuild does not parse again, by `Source::rel`.
    fronts: HashMap<String, FrontEntry>,
    /// What the last build took from the cache.
    pub stats: CacheStats,
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
}

impl BuildCache {
    /// Starts a build with these build-wide inputs.
    pub(crate) fn begin(&mut self, config: blake3::Hash) {
        self.stats = CacheStats::default();
        if self.config != Some(config) {
            self.stats.cleared = self.config.is_some();
            self.files.clear();
            self.lua_checks.clear();
            self.fronts.clear();
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
            self.generation,
        )
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
        self.lua_checks.retain(|_, c| c.generation == generation);
        self.fronts.retain(|_, f| f.generation == generation);
    }
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
