//! Discovering source files under `build.src` and asset files matched by
//! `build.assets`. `.gitignore` rules and `build.exclude` apply to both.

use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Lua,
    Wowlua,
    Xml,
}

impl SourceKind {
    fn of(path: &Path) -> Option<SourceKind> {
        let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
        if name.ends_with(".test.wowlua") || name.ends_with(".d.wowlua") {
            return None;
        }
        match path
            .extension()?
            .to_string_lossy()
            .to_ascii_lowercase()
            .as_str()
        {
            "lua" => Some(SourceKind::Lua),
            "wowlua" => Some(SourceKind::Wowlua),
            "xml" => Some(SourceKind::Xml),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Source {
    /// Path relative to the package root, `/` separators.
    pub rel: String,
    /// Path relative to `build.src`, `/` separators.
    pub src_rel: String,
    pub abs: PathBuf,
    pub kind: SourceKind,
}

impl Source {
    /// Output path inside the AddOn folder: `src/<path>`, with `.wowlua`
    /// compiled to `.lua` (spec §14.8).
    pub fn output_path(&self) -> String {
        let path = match self.kind {
            SourceKind::Wowlua => {
                format!(
                    "{}.lua",
                    self.src_rel
                        .strip_suffix(".wowlua")
                        .unwrap_or(&self.src_rel)
                )
            }
            _ => self.src_rel.clone(),
        };
        format!("src/{path}")
    }
}

#[derive(Clone, Debug)]
pub struct Asset {
    /// Path relative to the package root (= output path), `/` separators.
    pub rel: String,
    pub abs: PathBuf,
}

pub fn glob_set(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        // Patterns are validated with the manifest.
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Walks `dir` (respecting `.gitignore` and hidden files) and returns files as
/// `(path relative to root, absolute path)`, skipping `skip_dirs`.
fn walk(root: &Path, dir: &Path, skip_dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let skip = skip_dirs.to_vec();
    let walker = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(move |entry| !skip.iter().any(|s| entry.path() == s))
        .build();
    let mut files: Vec<(String, PathBuf)> = walker
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|entry| {
            let rel = crate::fsutil::relative_slash(entry.path(), root)?;
            Some((rel, entry.into_path()))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Directories never scanned: the output root and the lockfile's neighbours
/// such as `.git` are handled by the hidden-file rule.
fn skip_dirs(root: &Path, out: &str) -> Vec<PathBuf> {
    vec![root.join(out)]
}

/// Source files under `src`, sorted by path.
pub fn discover_sources(root: &Path, src: &str, out: &str, exclude: &GlobSet) -> Vec<Source> {
    let src_dir = root.join(src);
    walk(root, &src_dir, &skip_dirs(root, out))
        .into_iter()
        .filter(|(rel, _)| !exclude.is_match(rel))
        .filter_map(|(rel, abs)| {
            let kind = SourceKind::of(&abs)?;
            let src_rel = crate::fsutil::relative_slash(&abs, &src_dir)?;
            Some(Source {
                rel,
                src_rel,
                abs,
                kind,
            })
        })
        .collect()
}

/// Declaration files (`.d.wowlua`) under `src`: types only, never part of
/// the output; they type plain-Lua modules for imports (spec §5.3).
pub fn discover_declarations(root: &Path, src: &str, out: &str, exclude: &GlobSet) -> Vec<Source> {
    let src_dir = root.join(src);
    walk(root, &src_dir, &skip_dirs(root, out))
        .into_iter()
        .filter(|(rel, _)| {
            !exclude.is_match(rel) && rel.to_ascii_lowercase().ends_with(".d.wowlua")
        })
        .filter_map(|(rel, abs)| {
            let src_rel = crate::fsutil::relative_slash(&abs, &src_dir)?;
            Some(Source {
                rel,
                src_rel,
                abs,
                kind: SourceKind::Wowlua,
            })
        })
        .collect()
}

/// Files matched by `build.assets`, sorted by path.
pub fn discover_assets(root: &Path, out: &str, assets: &GlobSet, exclude: &GlobSet) -> Vec<Asset> {
    if assets.is_empty() {
        return Vec::new();
    }
    walk(root, root, &skip_dirs(root, out))
        .into_iter()
        .filter(|(rel, _)| assets.is_match(rel) && !exclude.is_match(rel))
        .map(|(rel, abs)| Asset { rel, abs })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discovers_sources_and_assets() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for path in [
            "src/main.wowlua",
            "src/core/a.lua",
            "src/ui/frames.xml",
            "src/types.d.wowlua",
            "src/main.test.wowlua",
            "src/notes.txt",
            "src/scratch/x.lua",
            "media/icon.tga",
            "dist/dev/old.lua",
        ] {
            let p = root.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "").unwrap();
        }
        let exclude = glob_set(&["src/scratch/**".into()]);
        let sources = discover_sources(root, "src", "dist", &exclude);
        let rels: Vec<&str> = sources.iter().map(|s| s.rel.as_str()).collect();
        assert_eq!(
            rels,
            ["src/core/a.lua", "src/main.wowlua", "src/ui/frames.xml"]
        );
        assert_eq!(sources[1].output_path(), "src/main.lua");

        let assets = discover_assets(root, "dist", &glob_set(&["media/**".into()]), &exclude);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].rel, "media/icon.tga");
    }
}
