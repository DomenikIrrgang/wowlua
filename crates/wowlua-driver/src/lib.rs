//! Orchestration of the wowlua toolchain: loading a project, building it into
//! a complete AddOn, validating the output, and installing it into WoW.
//!
//! All functions report problems by pushing diagnostics into a [`Session`];
//! the CLI renders them.

pub mod build;
mod compile;
pub mod format;
mod fsutil;
pub mod install;
pub mod lockfile;
mod luacheck;
pub mod project;
pub mod scaffold;
mod sources;
pub mod vcs;

use wowlua_diagnostics::{Diagnostic, Diagnostics};
use wowlua_span::SourceDb;

pub use build::{BuildOutput, BuildRequest, build};
pub use project::{LoadOptions, Project, load_project};

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
