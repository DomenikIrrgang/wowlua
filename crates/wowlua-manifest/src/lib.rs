//! The `wowlua.json` manifest (spec chapter 9): typed model, loading with
//! precise diagnostics, semantic validation, profile resolution and the JSON
//! schema.

pub mod edit;
mod load;
pub mod locate;
pub mod model;
pub mod profile;
mod validate;
pub mod world;

use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostics;
use wowlua_span::FileId;

pub use load::{load, parse};
pub use model::Manifest;
pub use profile::{BuiltinProfile, Profile, ProfileError};
pub use validate::is_generated_directive;

/// File name of the manifest.
pub const MANIFEST_FILE: &str = "wowlua.json";

/// The schema version written to `$schema` (spec §9.16).
pub const SCHEMA_VERSION: &str = "0.1";

/// The `$schema` URL of the current schema version (decision T46).
pub fn schema_url() -> String {
    format!(
        "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/wowlua-{SCHEMA_VERSION}.json"
    )
}

/// The JSON schema of the manifest, pretty-printed.
pub fn schema_json() -> String {
    let mut schema = serde_json::to_value(schemars::schema_for!(Manifest))
        .expect("the manifest schema serializes");
    schema["$id"] = schema_url().into();
    let mut text = serde_json::to_string_pretty(&schema).expect("schema serializes");
    text.push('\n');
    text
}

/// Minification levels built into the compiler (spec §15.4.2).
pub const BUILTIN_MINIFY_LEVELS: &[&str] = &["none", "strip", "compact", "rename", "max"];

/// Minification passes known to the compiler (spec §15.4.1).
pub const MINIFY_PASSES: &[&str] = &[
    "strip-comments",
    "strip-whitespace",
    "compact",
    "shorten-literals",
    "simplify",
    "rename-locals",
    "rename-private",
    "dedupe-strings",
];

/// The passes of minification level `name` (spec §15.4.2–15.4.3): the
/// project's own definition of the name, else the preset. Presets build on
/// each other by name, so a redefined `strip` also changes `compact`.
/// `None` for an unknown level.
pub fn minify_passes(
    manifest: &Manifest,
    name: &str,
) -> Option<std::collections::BTreeSet<String>> {
    fn preset(name: &str) -> Option<(Option<&'static str>, &'static [&'static str])> {
        Some(match name {
            "none" => (None, &[]),
            "strip" => (None, &["strip-comments", "strip-whitespace", "simplify"]),
            "compact" => (Some("strip"), &["compact", "shorten-literals"]),
            "rename" => (Some("compact"), &["rename-locals", "dedupe-strings"]),
            "max" => (Some("rename"), &["rename-private"]),
            _ => return None,
        })
    }
    let mut passes = std::collections::BTreeSet::new();
    let mut seen: Vec<String> = Vec::new();
    let mut current = Some(name.to_string());
    while let Some(level) = current {
        // Cycles are reported by validation (E4007).
        if seen.contains(&level) {
            break;
        }
        seen.push(level.clone());
        current = match manifest.minify_levels.get(&level) {
            Some(own) => {
                passes.extend(own.passes.iter().cloned());
                own.extends.clone()
            }
            None => {
                let (extends, own) = preset(&level)?;
                passes.extend(own.iter().map(|p| p.to_string()));
                extends.map(str::to_string)
            }
        };
    }
    Some(passes)
}

/// A successfully parsed and validated manifest.
#[derive(Debug, Clone)]
pub struct LoadedManifest {
    /// Path of `wowlua.json`.
    pub path: PathBuf,
    /// The package root (the directory containing the manifest).
    pub root: PathBuf,
    /// The manifest's file in the [`wowlua_span::SourceDb`].
    pub file: FileId,
    pub manifest: Manifest,
    /// Targeted flavor IDs, normalized from aliases, in manifest order.
    pub flavors: Vec<String>,
}

impl LoadedManifest {
    pub fn package(&self) -> Option<&model::Package> {
        self.manifest.package.as_ref()
    }

    pub fn is_workspace_root(&self) -> bool {
        self.manifest.workspace.is_some()
    }

    /// The AddOn folder/TOC name: `addon.name` or PascalCase of `package.name`.
    pub fn addon_name(&self) -> String {
        self.manifest
            .addon
            .as_ref()
            .and_then(|a| a.name.clone())
            .unwrap_or_else(|| {
                self.package()
                    .map(|p| pascal_case(&p.name))
                    .unwrap_or_default()
            })
    }

    /// `addon.title`, defaulting to the addon name.
    pub fn title(&self) -> String {
        self.manifest
            .addon
            .as_ref()
            .and_then(|a| a.title.clone())
            .unwrap_or_else(|| self.addon_name())
    }

    /// `addon.author`, defaulting to the package authors without e-mail parts.
    pub fn author(&self) -> Option<String> {
        if let Some(author) = self.manifest.addon.as_ref().and_then(|a| a.author.clone()) {
            return Some(author);
        }
        let authors: Vec<String> = self
            .package()?
            .authors
            .iter()
            .map(|a| strip_email(a))
            .filter(|a| !a.is_empty())
            .collect();
        (!authors.is_empty()).then(|| authors.join(", "))
    }

    /// Resolves a profile by name (built-in or custom).
    pub fn profile(&self, name: &str) -> Result<Profile, ProfileError> {
        profile::resolve(&self.manifest, name)
    }

    /// Source root (`build.src`), absolute.
    pub fn src_dir(&self) -> PathBuf {
        self.root.join(&self.manifest.build.src)
    }

    /// Output root (`build.out`), absolute.
    pub fn out_dir(&self) -> PathBuf {
        self.root.join(&self.manifest.build.out)
    }
}

/// Result of loading a manifest: the manifest if it is usable, plus all
/// diagnostics (warnings may accompany a usable manifest).
#[derive(Debug)]
pub struct LoadOutput {
    pub manifest: Option<LoadedManifest>,
    pub diagnostics: Diagnostics,
}

/// `my-addon` → `MyAddon`.
pub fn pascal_case(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// `"Jane Doe <jane@example.com>"` → `"Jane Doe"`.
pub fn strip_email(author: &str) -> String {
    match author.find('<') {
        Some(i) => author[..i].trim().to_string(),
        None => author.trim().to_string(),
    }
}

/// Finds the manifest for a command: `explicit`, or the nearest
/// `wowlua.json` in `start` or its ancestors.
pub fn find_manifest(start: &Path, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    start
        .ancestors()
        .map(|dir| dir.join(MANIFEST_FILE))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(pascal_case("my-addon"), "MyAddon");
        assert_eq!(pascal_case("kill_tracker-x"), "KillTrackerX");
        assert_eq!(strip_email("Jane Doe <jane@example.com>"), "Jane Doe");
        assert_eq!(strip_email("Solo"), "Solo");
    }

    #[test]
    fn resolves_minify_levels() {
        let manifest: Manifest = serde_json::from_str(
            r#"{ "minify-levels": {
                "strip": { "passes": ["strip-comments"] },
                "small": { "extends": "compact", "passes": ["dedupe-strings"] }
            } }"#,
        )
        .unwrap();
        let passes =
            |name: &str| minify_passes(&manifest, name).map(|p| p.into_iter().collect::<Vec<_>>());
        assert_eq!(passes("none"), Some(vec![]));
        // The redefined `strip` flows into the presets built on it.
        assert_eq!(
            passes("small").unwrap(),
            [
                "compact",
                "dedupe-strings",
                "shorten-literals",
                "strip-comments"
            ]
        );
        assert_eq!(
            passes("max").unwrap(),
            [
                "compact",
                "dedupe-strings",
                "rename-locals",
                "rename-private",
                "shorten-literals",
                "strip-comments"
            ]
        );
        assert_eq!(passes("nope"), None);
    }

    #[test]
    fn schema_is_generated() {
        let schema = schema_json();
        assert!(schema.contains("\"saved-variables\""));
        assert!(schema.contains(&schema_url()));
    }
}
