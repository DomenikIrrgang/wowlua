//! Orchestration of the wowlua toolchain: loading a project, building it into
//! a complete AddOn, validating the output, and installing it into WoW.
//!
//! All functions report problems by pushing diagnostics into a [`Session`];
//! the CLI renders them.

pub mod build;
mod bundle;
mod compile;
pub mod compile_cache;
pub mod credentials;
pub mod deps;
mod embed;
pub mod external;
pub mod fix;
mod foreign;
pub mod format;
mod fsutil;
pub mod incremental;
pub mod install;
pub mod keywords;
mod locale;
pub mod lockfile;
mod luacheck;
mod meta;
pub mod overlay;
pub mod package;
pub mod packaging;
pub mod parallel;
pub mod pkgmeta;
mod private_names;
pub mod project;
pub mod publish;
pub mod scaffold;
mod sources;
pub mod trace;
pub mod types;
pub mod vcs;
pub mod workspace;

use wowlua_diagnostics::{Diagnostic, Diagnostics};
use wowlua_span::SourceDb;

pub use build::{BuildOutput, BuildRequest, Only, TestFile, build, build_with_cache};
pub use compile::{Optimizations, Optimized, optimize_and_emit};
pub use incremental::{BuildCache, CacheStats};
pub use luacheck::LuaOutOfMemory;
pub use project::{
    ExternalAddon, LoadOptions, Project, Status, load_project, load_projects, standalone_project,
};
pub use sources::{TestRole, glob_set};

/// wowlua's cache folder (`$WOWLUA_HOME/cache`).
pub fn cache_root() -> Option<std::path::PathBuf> {
    wowlua_pkg::Cache::new().map(|c| c.root)
}

/// Version of this compiler.
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn compiler_version() -> semver::Version {
    semver::Version::parse(COMPILER_VERSION).expect("the crate version is valid semver")
}

/// Shared state of one command: the source files and all diagnostics.
#[derive(Debug, Default)]
pub struct Session {
    pub db: SourceDb,
    pub diagnostics: Diagnostics,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.has_errors()
    }

    /// Takes the diagnostics collected so far (for incremental rendering).
    pub fn take_diagnostics(&mut self) -> Diagnostics {
        std::mem::take(&mut self.diagnostics)
    }
}
