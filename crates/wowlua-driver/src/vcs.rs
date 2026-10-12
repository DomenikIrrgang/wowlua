//! Version-control information from the system `git`, used for
//! `build.version` (spec §9.9.2). Everything is optional: without git or
//! outside a repository the build still works.

use std::path::Path;
use std::process::{Command, Stdio};

/// State of the repository at the package root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VcsInfo {
    /// Abbreviated commit hash of `HEAD`.
    pub short_hash: String,
    /// Tags pointing exactly at `HEAD`.
    pub tags: Vec<String>,
    /// Uncommitted changes to tracked files.
    pub dirty: bool,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Reads the repository state, or `None` without git or a repository.
pub fn info(dir: &Path) -> Option<VcsInfo> {
    let short_hash = git(dir, &["rev-parse", "--short=7", "HEAD"])?;
    let tags = git(dir, &["tag", "--points-at", "HEAD"])
        .map(|out| out.lines().map(str::to_string).collect())
        .unwrap_or_default();
    let dirty = git(dir, &["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|out| !out.is_empty());
    Some(VcsInfo {
        short_hash,
        tags,
        dirty,
    })
}

/// Whether `dir` is inside a Git work tree.
pub fn inside_work_tree(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"]).as_deref() == Some("true")
}

/// Whether the `git` executable is available.
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Moves a tracked file with `git mv`, so history follows it.
pub fn git_mv(dir: &Path, from: &str, to: &str) -> bool {
    git(dir, &["mv", "-k", from, to]).is_some() && !dir.join(from).exists()
}

/// Runs `git init` in `dir`.
pub fn git_init(dir: &Path) -> bool {
    git(dir, &["init", "-q"]).is_some()
}

/// The user's configured Git name, for templates.
pub fn user_name(dir: &Path) -> Option<String> {
    git(dir, &["config", "user.name"]).filter(|n| !n.is_empty())
}

/// The most recent tag reachable from `HEAD`.
pub fn latest_tag(dir: &Path) -> Option<String> {
    git(dir, &["describe", "--tags", "--abbrev=0"]).filter(|t| !t.is_empty())
}

/// `owner/repo` of the `origin` remote when it is on GitHub.
pub fn github_repository(dir: &Path) -> Option<String> {
    let url = git(dir, &["remote", "get-url", "origin"])?;
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty()).then(|| format!("{owner}/{repo}"))
}
