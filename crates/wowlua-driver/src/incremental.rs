//! What `wowlua dev` keeps between builds of one project (#7): each file's
//! compiled output, by its source, `FileId`, bytes and context. Every
//! rebuild gives what a fresh build of the same tree gives; the cache only
//! skips compiling a file again where that would give the same.

use crate::compile::{CacheEntry, FlavorCache};
use std::collections::HashMap;
use wowlua_diagnostics::Diagnostic;
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

/// What a build took from the cache: per file and flavor compiled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: usize,
    pub misses: usize,
    /// Plain Lua sources whose Lua 5.1 check was reused.
    pub lua_checks: usize,
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
            self.config = Some(config);
        }
        self.hashes.clear();
        self.generation += 1;
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
        let result = crate::luacheck::check(db, file, bytes);
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
        let generation = self.generation;
        let mut files = cache.entries;
        files.retain(|_, entries| {
            entries.retain(|e| e.generation == generation);
            !entries.is_empty()
        });
        self.files = files;
        self.lua_checks.retain(|_, c| c.generation == generation);
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
