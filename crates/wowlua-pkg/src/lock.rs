//! Package entries of `wowlua.lock` (spec §10.6). Keys are written in a
//! fixed order and entries sorted by name, so diffs stay minimal.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    /// `<source>#<pin>`: `git+<url>?tag=1.0#<commit>`, `svn+<url>#<rev>`,
    /// `archive+<url>#<sha256>`, or `path+<path>` (never pinned).
    pub source: String,
    /// Checksum of the package's file tree (not for path sources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// The packages (addons and libraries of the workspace) using it.
    #[serde(default)]
    pub used_by: Vec<String>,
}

impl LockedPackage {
    /// The source without the pin.
    pub fn key(&self) -> &str {
        self.source.split_once('#').map_or(&self.source, |(k, _)| k)
    }

    /// The commit, revision or archive checksum.
    pub fn pin(&self) -> Option<&str> {
        self.source.split_once('#').map(|(_, p)| p)
    }
}
