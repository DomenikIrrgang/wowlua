//! Discovering source files under `build.src` and asset files matched by
//! `build.assets`. `.gitignore` rules and `build.exclude` apply to both.

use globset::{Glob, GlobSet, GlobSetBuilder};
use std::collections::HashSet;
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
    /// The output path when it is not derived from `src_rel` (files of
    /// wowlua library dependencies: `libs/<name>/…`).
    pub output: Option<String>,
}

impl Source {
    /// Output path inside the AddOn folder: `src/<path>`, with `.wowlua`
    /// compiled to `.lua` (spec §14.8).
    pub fn output_path(&self) -> String {
        if let Some(output) = &self.output {
            return output.clone();
        }
        // Standard-library modules: `wowlua/std/<name>.lua`.
        if let Some(name) = self
            .src_rel
            .strip_prefix(wowlua_sema::STDLIB_PREFIX)
            .and_then(|n| n.strip_suffix(".wowlua"))
        {
            return format!("wowlua/std/{name}.lua");
        }
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

/// Like [`glob_set`], but case-insensitive (asset paths: the client reads
/// `.TTF` as well as `.ttf`).
pub fn glob_set_ci(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        if let Ok(glob) = globset::GlobBuilder::new(pattern)
            .case_insensitive(true)
            .build()
        {
            builder.add(glob);
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Walks `dir` (respecting `.gitignore` and hidden files) and returns files as
/// `(path relative to root, absolute path)`, skipping `skip_dirs`. Like git,
/// `.gitignore` does not apply to files git tracks.
fn walk(root: &Path, dir: &Path, skip_dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let walk_with = |git_ignore: bool| -> Vec<PathBuf> {
        let skip = skip_dirs.to_vec();
        ignore::WalkBuilder::new(dir)
            .hidden(true)
            .git_ignore(git_ignore)
            .git_global(false)
            .git_exclude(git_ignore)
            .ignore(git_ignore)
            .require_git(false)
            .filter_entry(move |entry| !skip.iter().any(|s| entry.path() == s))
            .build()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
            .map(|entry| entry.into_path())
            .collect()
    };
    let mut paths = walk_with(true);
    let tracked = tracked_files(dir);
    if !tracked.is_empty() {
        let found: HashSet<PathBuf> = paths.iter().cloned().collect();
        paths.extend(
            walk_with(false)
                .into_iter()
                .filter(|p| !found.contains(p) && tracked.contains(p)),
        );
    }
    let mut files: Vec<(String, PathBuf)> = paths
        .into_iter()
        .filter_map(|path| Some((crate::fsutil::relative_slash(&path, root)?, path)))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// The files under `dir` git tracks (absolute paths); empty outside a
/// repository.
pub(crate) fn tracked_files(dir: &Path) -> HashSet<PathBuf> {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "-z"])
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return HashSet::new();
    };
    if !output.status.success() {
        return HashSet::new();
    }
    output
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| dir.join(String::from_utf8_lossy(p).as_ref()))
        .collect()
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
                output: None,
            })
        })
        .collect()
}

/// The files given to `wowlua compile` (absolute, under `root`): sources
/// and declaration files. Paths are relative to `src` when inside it, else
/// to the root.
pub fn given(
    session: &mut crate::Session,
    files: &[PathBuf],
    root: &Path,
    src: &str,
) -> (Vec<Source>, Vec<Source>) {
    // Compared canonically (the given paths are canonical).
    let root = &root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let src_dir = root.join(src);
    let src_dir = src_dir.canonicalize().unwrap_or(src_dir);
    let mut sources: Vec<Source> = Vec::new();
    let mut declarations = Vec::new();
    for abs in files {
        let Some(rel) = crate::fsutil::relative_slash(abs, root) else {
            session.push(wowlua_diagnostics::Diagnostic::plain_error(format!(
                "`{}` is outside the package at `{}`",
                abs.display(),
                root.display()
            )));
            continue;
        };
        let src_rel = crate::fsutil::relative_slash(abs, &src_dir).unwrap_or_else(|| rel.clone());
        let lower = rel.to_ascii_lowercase();
        let kind = if lower.ends_with(".d.wowlua") {
            None
        } else {
            match SourceKind::of(abs) {
                Some(SourceKind::Xml) | None => {
                    session.push(
                        wowlua_diagnostics::Diagnostic::plain_error(
                            "compiles `.wowlua` and `.lua` files (and `.d.wowlua` declarations)",
                        )
                        .with_path(&rel),
                    );
                    continue;
                }
                kind => kind,
            }
        };
        if sources
            .iter()
            .chain(&declarations)
            .any(|s: &Source| s.abs == *abs)
        {
            continue;
        }
        let source = Source {
            rel,
            src_rel,
            abs: abs.clone(),
            kind: kind.unwrap_or(SourceKind::Wowlua),
            output: None,
        };
        if kind.is_some() {
            sources.push(source);
        } else {
            declarations.push(source);
        }
    }
    sources.sort_by(|a, b| a.rel.cmp(&b.rel));
    (sources, declarations)
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
                output: None,
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
