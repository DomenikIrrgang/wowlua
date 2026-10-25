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
/// Walks remembered by a long-lived process (`remember_walks`): by the
/// directory, its skipped directories and its root, the files found and
/// what shows they are still the files there.
type Walks = std::collections::HashMap<(PathBuf, Vec<PathBuf>, PathBuf), Walked>;
static WALKS: std::sync::Mutex<Option<Walks>> = std::sync::Mutex::new(None);

/// A walk's files and the modification times that keep it valid: of every
/// directory it went through (adding, removing or renaming an entry
/// changes its directory's), of the ignore files it read, and of git's
/// index (which files git tracks).
struct Walked {
    files: Vec<(String, PathBuf)>,
    stamps: Stamps,
}

/// Paths and their modification times (`None`: not there).
type Stamps = Vec<(PathBuf, Option<std::time::SystemTime>)>;

/// A long-lived process (the language server's checker) remembers walks
/// for as long as nothing they saw changed.
pub fn remember_walks() {
    let mut w = WALKS.lock().unwrap_or_else(|e| e.into_inner());
    w.get_or_insert_with(Default::default);
}

/// Stops remembering walks (for tests: the switch is the process's).
#[cfg(test)]
fn stop_remembering_walks() {
    *WALKS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Forgets the remembered walks (files were created or deleted).
pub fn forget_walks() {
    let mut w = WALKS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(w) = w.as_mut() {
        w.clear();
    }
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    crate::host::modified(path)
}

fn walk(root: &Path, dir: &Path, skip_dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let key = (dir.to_path_buf(), skip_dirs.to_vec(), root.to_path_buf());
    {
        let w = WALKS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(walks) = w.as_ref() else {
            drop(w);
            return walk_now(root, dir, skip_dirs).0;
        };
        if let Some(walked) = walks.get(&key)
            && walked.stamps.iter().all(|(p, t)| modified(p) == *t)
        {
            return walked.files.clone();
        }
    }
    let (files, stamps) = walk_now(root, dir, skip_dirs);
    let mut w = WALKS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(walks) = w.as_mut() {
        walks.insert(
            key,
            Walked {
                files: files.clone(),
                stamps,
            },
        );
    }
    files
}

/// Walks `dir`: its files, and the stamps that keep the walk valid.
fn walk_now(root: &Path, dir: &Path, skip_dirs: &[PathBuf]) -> (Vec<(String, PathBuf)>, Stamps) {
    let stamps = std::cell::RefCell::new(Vec::new());
    // An installed host (the browser's) has no ignore files and no git:
    // every file that is not hidden.
    if crate::host::installed().is_some() {
        let files = crate::host::walk(dir, false, &|d| !skip_dirs.iter().any(|s| d == s));
        let mut files: Vec<(String, PathBuf)> = files
            .into_iter()
            .filter_map(|path| Some((crate::fsutil::relative_slash(&path, root)?, path)))
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        return (files, Vec::new());
    }
    let walk_with = |git_ignore: bool| -> Vec<PathBuf> {
        let skip = skip_dirs.to_vec();
        let mut files = Vec::new();
        #[allow(clippy::disallowed_methods)] // (no host installed: the disk's)
        for entry in ignore::WalkBuilder::new(dir)
            .hidden(true)
            .git_ignore(git_ignore)
            .git_global(false)
            .git_exclude(git_ignore)
            .ignore(git_ignore)
            .require_git(false)
            .filter_entry(move |entry| !skip.iter().any(|s| entry.path() == s))
            .build()
            .filter_map(Result::ok)
        {
            match entry.file_type() {
                Some(t) if t.is_dir() => {
                    let path = entry.path();
                    let mut st = stamps.borrow_mut();
                    st.push((path.to_path_buf(), modified(path)));
                    for name in [".gitignore", ".ignore"] {
                        let f = path.join(name);
                        st.push((f.clone(), modified(&f)));
                    }
                }
                Some(t) if t.is_file() => files.push(entry.into_path()),
                _ => {}
            }
        }
        files
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
    let mut stamps = stamps.into_inner();
    // (what git tracks changes with its index)
    if let Some(git) = git_dir(dir) {
        let index = git.join("index");
        stamps.push((index.clone(), modified(&index)));
    }
    stamps.sort();
    stamps.dedup();
    (files, stamps)
}

/// The `.git` directory of the repository `dir` is in, if any (`.git` may
/// be a file naming it, in a worktree).
fn git_dir(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(dir);
    while let Some(here) = d {
        let dot = here.join(".git");
        if crate::host::is_dir(&dot) {
            return Some(dot);
        }
        if let Ok(text) = crate::host::read_to_string(&dot)
            && let Some(rest) = text.trim().strip_prefix("gitdir:")
        {
            return Some(here.join(rest.trim()));
        }
        d = here.parent();
    }
    None
}

/// The files under `dir` git tracks (absolute paths); empty outside a
/// repository.
pub(crate) fn tracked_files(dir: &Path) -> HashSet<PathBuf> {
    if crate::host::installed().is_some() {
        return HashSet::new();
    }
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

/// Directories never scanned: the output root and the test side
/// (`tests/`, never part of a build); the lockfile's neighbours such as
/// `.git` are handled by the hidden-file rule.
fn skip_dirs(root: &Path, out: &str) -> Vec<PathBuf> {
    vec![root.join(out), root.join(TESTS_DIR)]
}

/// The test side's folder (spec Tooling §16.1.1).
pub const TESTS_DIR: &str = "tests";

/// What a file of the test side is (spec Tooling §16.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestRole {
    /// `*.test.wowlua`: a test file.
    Test,
    /// `tests/mocks/**`: loaded before the addon.
    Mock,
    /// Other files under `tests/`: importable from test files.
    Helper,
    /// A standard-library module only the test side imports.
    Std,
}

impl TestRole {
    /// The role of a source by its package-relative path (`None`: the
    /// addon's own).
    pub fn of(rel: &str) -> Option<TestRole> {
        if rel.ends_with(".test.wowlua") {
            Some(TestRole::Test)
        } else if rel.starts_with(&format!("{TESTS_DIR}/mocks/")) {
            Some(TestRole::Mock)
        } else if rel.starts_with(&format!("{TESTS_DIR}/")) {
            Some(TestRole::Helper)
        } else {
            None
        }
    }
}

/// The test side's sources: `*.test.wowlua` files under `src` and every
/// `.wowlua` file under `tests/` (as `@tests/<path>`, output
/// `tests/<path>.lua`), sorted by path.
pub fn discover_tests(root: &Path, src: &str, out: &str, exclude: &GlobSet) -> Vec<Source> {
    let src_dir = root.join(src);
    let mut found: Vec<Source> = walk(root, &src_dir, &skip_dirs(root, out))
        .into_iter()
        .filter(|(rel, _)| !exclude.is_match(rel) && rel.ends_with(".test.wowlua"))
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
        .collect();
    let tests_dir = root.join(TESTS_DIR);
    if crate::host::is_dir(&tests_dir) {
        found.extend(
            walk(root, &tests_dir, &[root.join(out)])
                .into_iter()
                .filter(|(rel, _)| {
                    let lower = rel.to_ascii_lowercase();
                    !exclude.is_match(rel)
                        && lower.ends_with(".wowlua")
                        && !lower.ends_with(".d.wowlua")
                })
                .filter_map(|(rel, abs)| {
                    let inner = crate::fsutil::relative_slash(&abs, &tests_dir)?;
                    let stem = inner.strip_suffix(".wowlua").unwrap_or(&inner).to_string();
                    Some(Source {
                        rel,
                        src_rel: format!("{}{inner}", wowlua_sema::TESTS_PREFIX),
                        abs,
                        kind: SourceKind::Wowlua,
                        output: Some(format!("{TESTS_DIR}/{stem}.lua")),
                    })
                }),
        );
    }
    found.sort_by(|a, b| a.rel.cmp(&b.rel));
    found
}

/// The file-level locals the test side reads through `import internal`
/// (spec Tooling §16.1.3.1), by the module's source path (`src_rel`): a
/// local only tests read is not unused (W1003), in any build.
pub fn internal_reads(
    root: &Path,
    src: &str,
    out: &str,
    exclude: &GlobSet,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut reads: std::collections::HashMap<String, Vec<String>> = Default::default();
    let remember = WALKS.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    for test in discover_tests(root, src, out, exclude) {
        let Ok(bytes) = crate::overlay::read(&test.abs) else {
            continue;
        };
        if !bytes.windows(8).any(|w| w == b"internal") {
            continue;
        }
        // (a long-lived process keeps each test file's, by its bytes)
        let found = if remember {
            let hash = blake3::hash(&bytes);
            let kept = INTERNAL
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&test.abs)
                .filter(|(h, _)| *h == hash)
                .map(|(_, f)| f.clone());
            kept.unwrap_or_else(|| {
                let found = internal_reads_of(&test, &bytes);
                INTERNAL
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(test.abs.clone(), (hash, found.clone()));
                found
            })
        } else {
            internal_reads_of(&test, &bytes)
        };
        for (module, names) in found {
            reads.entry(module).or_default().extend(names);
        }
    }
    reads
}

/// Each test file's `import internal` reads, by its path and bytes' hash,
/// in a long-lived process.
type InternalReads = std::collections::HashMap<PathBuf, (blake3::Hash, Vec<(String, Vec<String>)>)>;
static INTERNAL: std::sync::LazyLock<std::sync::Mutex<InternalReads>> =
    std::sync::LazyLock::new(Default::default);

/// What one test file reads through `import internal`: by module path,
/// the names.
fn internal_reads_of(test: &Source, bytes: &[u8]) -> Vec<(String, Vec<String>)> {
    use wowlua_syntax::view::{ImportClause, StmtKind};
    let mut out = Vec::new();
    let mut db = wowlua_span::SourceDb::new();
    let file = db.add(test.rel.clone(), String::new());
    let parse = wowlua_syntax::parse(bytes, file);
    for s in parse.view().block().stmts() {
        let StmtKind::Import(i) = s.kind() else {
            continue;
        };
        if !i.internal() {
            continue;
        }
        let ImportClause::Names(specs) = i.clause() else {
            continue;
        };
        let spec = String::from_utf8_lossy(&i.from().value()).into_owned();
        let base = if let Some(rest) = spec.strip_prefix('/') {
            rest.to_string()
        } else if spec.starts_with("./") || spec.starts_with("../") {
            let dir = test.src_rel.rsplit_once('/').map_or("", |(d, _)| d);
            format!("{dir}/{spec}")
        } else {
            continue;
        };
        let mut parts: Vec<&str> = Vec::new();
        for part in base.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                p => parts.push(p),
            }
        }
        let path = parts.join("/");
        let names: Vec<String> = specs.iter().map(|s| s.name.text().to_string()).collect();
        for module in [format!("{path}.wowlua"), format!("{path}/init.wowlua")] {
            out.push((module, names.clone()));
        }
    }
    out
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
    let root = &crate::host::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let src_dir = root.join(src);
    let src_dir = crate::host::canonicalize(&src_dir).unwrap_or(src_dir);
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

    #[test]
    fn a_remembered_walk_sees_files_added_and_removed() {
        super::remember_walks();
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        crate::host::create_dir_all(src.join("ui")).unwrap();
        crate::host::write(src.join("a.wowlua"), "").unwrap();
        let walk = || {
            super::walk(dir.path(), &src, &[])
                .into_iter()
                .map(|(rel, _)| rel)
                .collect::<Vec<_>>()
        };
        assert_eq!(walk(), ["src/a.wowlua"]);
        // remembered: the same again
        assert_eq!(walk(), ["src/a.wowlua"]);
        // a file in a subfolder, then one removed
        crate::host::write(src.join("ui/b.wowlua"), "").unwrap();
        assert_eq!(walk(), ["src/a.wowlua", "src/ui/b.wowlua"]);
        crate::host::remove_file(src.join("a.wowlua")).unwrap();
        assert_eq!(walk(), ["src/ui/b.wowlua"]);
        // forgotten: walked anew
        super::forget_walks();
        assert_eq!(walk(), ["src/ui/b.wowlua"]);
        super::stop_remembering_walks();
    }

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
