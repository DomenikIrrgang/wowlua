//! Fetching sources into the cache: git through the system `git` (bare
//! clones, exported per commit), SVN through the system `svn` (exported per
//! revision), archives over HTTPS (stored by SHA-256, extracted once).

use crate::cache::{self, Cache};
use crate::safety;
use crate::source::GitRef;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A fetch problem, turned into a diagnostic by the caller.
#[derive(Debug, Clone)]
pub struct FetchError {
    pub code: Option<&'static str>,
    pub message: String,
    pub help: Option<String>,
}

impl FetchError {
    fn plain(message: impl Into<String>) -> Self {
        FetchError {
            code: None,
            message: message.into(),
            help: None,
        }
    }

    fn coded(code: &'static str, message: impl Into<String>) -> Self {
        FetchError {
            code: Some(code),
            message: message.into(),
            help: None,
        }
    }

    fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    fn offline(what: &str) -> Self {
        FetchError::coded(
            "E4105",
            format!("{what} is not in the cache, and the network may not be used"),
        )
        .help("run `wowlua fetch` without `--offline` / `--frozen` first")
    }
}

type Status = Box<dyn Fn(&str, &str)>;

pub struct Fetcher {
    pub cache: Cache,
    /// `--offline` / `--frozen`: only the cache.
    pub offline: bool,
    status: Status,
    /// Repositories already fetched by this command.
    fetched: RefCell<HashSet<String>>,
}

impl std::fmt::Debug for Fetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetcher")
            .field("cache", &self.cache)
            .field("offline", &self.offline)
            .finish()
    }
}

/// Output of a finished command, or why it failed.
fn run(mut command: Command, what: &str) -> Result<String, FetchError> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                let install = match program.as_str() {
                    "svn" => "install Subversion (e.g. `brew install subversion`, `apt install subversion`, or TortoiseSVN with command-line tools)",
                    _ => "install Git and make sure `git` is on the PATH",
                };
                FetchError::plain(format!("{what}: `{program}` is not installed")).help(install)
            } else {
                FetchError::plain(format!("{what}: cannot run `{program}`: {e}"))
            }
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("failed")
            .to_string();
        Err(FetchError::plain(format!("{what}: {detail}")))
    }
}

/// `run` for network operations: up to three attempts (like the
/// packager's `retry`), because repository hosts fail transiently.
fn run_net(url: &str, make: impl Fn() -> Command, what: &str) -> Result<String, FetchError> {
    let local = url.starts_with("file:") || !url.contains("://");
    let mut delay = std::time::Duration::from_secs(2);
    for attempt in 1.. {
        match run(make(), what) {
            Err(e) if !local && attempt < 3 && !e.message.contains("is not installed") => {
                std::thread::sleep(delay);
                delay *= 2;
            }
            result => return result,
        }
    }
    unreachable!("the loop returns")
}

fn git(dir: Option<&Path>) -> Command {
    let mut c = Command::new("git");
    // Exports keep the repository's bytes on every platform (checksums and
    // builds must not depend on a user's line-ending settings).
    c.args(["-c", "core.autocrlf=false", "-c", "core.eol=lf"]);
    if let Some(d) = dir {
        c.arg("--git-dir").arg(d);
    }
    c
}

impl Fetcher {
    pub fn new(cache: Cache, offline: bool, status: impl Fn(&str, &str) + 'static) -> Self {
        Fetcher {
            cache,
            offline,
            status: Box::new(status),
            fetched: RefCell::new(HashSet::new()),
        }
    }

    fn say(&self, word: &str, message: &str) {
        (self.status)(word, message);
    }

    // ---- git ---------------------------------------------------------------

    /// The bare clone of `url`, cloned or updated (once per command).
    fn git_repo(&self, url: &str, update: bool) -> Result<PathBuf, FetchError> {
        let dir = self.cache.git_dir(url);
        if dir.join("HEAD").exists() {
            self.cache.touch(&dir);
            if update && !self.offline && self.fetched.borrow_mut().insert(url.to_string()) {
                self.say("Updating", url);
                run_net(
                    url,
                    || {
                        let mut c = git(Some(&dir));
                        c.args([
                            "fetch",
                            "--quiet",
                            "--force",
                            "--prune",
                            "--tags",
                            "origin",
                            "+refs/heads/*:refs/heads/*",
                        ]);
                        c
                    },
                    &format!("cannot update `{url}`"),
                )?;
            }
            return Ok(dir);
        }
        if self.offline {
            return Err(FetchError::offline(&format!("git repository `{url}`")));
        }
        // A damaged entry (no `HEAD`) is replaced.
        if dir.exists() {
            let _ = cache::remove(&dir);
        }
        self.say("Cloning", url);
        let staged = cache::staging(&dir).map_err(|e| FetchError::plain(e.to_string()))?;
        let clone = || {
            let _ = cache::remove(&staged);
            let mut c = git(None);
            c.args(["clone", "--bare", "--quiet", url]).arg(&staged);
            c
        };
        if let Err(e) = run_net(url, clone, &format!("cannot clone `{url}`")) {
            let _ = cache::remove(&staged);
            return Err(FetchError {
                code: Some("E4102"),
                ..e
            });
        }
        if !staged.join("HEAD").exists() {
            let listing: Vec<String> = std::fs::read_dir(&staged)
                .map(|d| {
                    d.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            let _ = cache::remove(&staged);
            return Err(FetchError::coded(
                "E4102",
                format!("cloning `{url}` produced no repository (the folder holds {listing:?})"),
            ));
        }
        cache::commit(&staged, &dir).map_err(|e| FetchError::plain(e.to_string()))?;
        self.cache.touch(&dir);
        self.fetched.borrow_mut().insert(url.to_string());
        Ok(dir)
    }

    fn has_commit(dir: &Path, commit: &str) -> bool {
        let mut c = git(Some(dir));
        c.args(["cat-file", "-e", &format!("{commit}^{{commit}}")])
            .stderr(Stdio::null());
        run(c, "").is_ok()
    }

    fn rev_parse(dir: &Path, spec: &str) -> Option<String> {
        let mut c = git(Some(dir));
        c.args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{spec}^{{commit}}"),
        ]);
        run(c, "").ok().filter(|s| !s.is_empty())
    }

    /// The commit `reference` names. A `locked` commit that is still in the
    /// cache is used without network access.
    pub fn git_commit(
        &self,
        url: &str,
        reference: &GitRef,
        locked: Option<&str>,
    ) -> Result<String, FetchError> {
        if let Some(commit) = locked {
            let dir = self.git_repo(url, false)?;
            if Self::has_commit(&dir, commit) {
                return Ok(commit.to_string());
            }
            if self.offline {
                return Err(FetchError::offline(&format!("commit {commit} of `{url}`")));
            }
            let dir = self.git_repo(url, true)?;
            if Self::has_commit(&dir, commit) {
                return Ok(commit.to_string());
            }
            return Err(FetchError::coded(
                "E4102",
                format!("locked commit {commit} no longer exists in `{url}`"),
            )
            .help("run `wowlua update` to pick a new commit"));
        }
        let dir = self.git_repo(url, true)?;
        let found = match reference {
            // The packager's `tag: latest`: the newest tag in a 50-commit
            // clone of the default branch, else that branch.
            GitRef::Tag(t) if t == LATEST_TAG => latest_tag(&dir)
                .and_then(|(tag, _)| Self::rev_parse(&dir, &format!("refs/tags/{tag}")))
                .or_else(|| Self::rev_parse(&dir, "HEAD")),
            GitRef::Tag(t) => Self::rev_parse(&dir, &format!("refs/tags/{t}")),
            GitRef::Branch(b) => Self::rev_parse(&dir, &format!("refs/heads/{b}")),
            GitRef::Rev(r) => Self::rev_parse(&dir, r),
            GitRef::Head => Self::rev_parse(&dir, "HEAD"),
        };
        found.ok_or_else(|| {
            let what = match reference {
                GitRef::Tag(t) => format!("tag `{t}`"),
                GitRef::Branch(b) => format!("branch `{b}`"),
                GitRef::Rev(r) => format!("revision `{r}`"),
                GitRef::Head => "a default branch".into(),
            };
            // What git says, for diagnosis.
            let mut c = git(Some(&dir));
            c.args(["symbolic-ref", "HEAD"]);
            let head = run(c, "HEAD").unwrap_or_else(|e| e.message);
            let listing: Vec<String> = std::fs::read_dir(dir.parent().unwrap_or(&dir))
                .map(|d| {
                    d.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            FetchError::coded("E4102", format!("`{url}` has no {what}")).help(format!(
                "the cached clone at {} has HEAD {head}; its folder holds {listing:?}",
                dir.display()
            ))
        })
    }

    /// The files of `commit`, exported into the cache.
    pub fn git_checkout(
        &self,
        url: &str,
        commit: &str,
        submodules: bool,
    ) -> Result<PathBuf, FetchError> {
        let name = format!(
            "git-{}-{}-{}{}",
            cache::label(url),
            cache::key(url),
            &commit[..commit.len().min(12)],
            if submodules { "-sub" } else { "" }
        );
        let target = self.cache.checkout_dir(&name);
        if target.exists() {
            self.cache.touch(&target);
            return Ok(target);
        }
        let repo = self.git_repo(url, false)?;
        let staged = cache::staging(&target).map_err(|e| FetchError::plain(e.to_string()))?;
        let result = if submodules {
            self.git_export_with_submodules(url, commit, &staged)
        } else {
            Self::git_export(&repo, commit, &staged)
        };
        if let Err(e) = result {
            let _ = cache::remove(&staged);
            return Err(e);
        }
        if let Err(u) = safety::check_tree(&staged) {
            let _ = cache::remove(&staged);
            return Err(unsafe_error(url, u));
        }
        cache::commit(&staged, &target).map_err(|e| FetchError::plain(e.to_string()))?;
        self.cache.touch(&target);
        Ok(target)
    }

    fn git_export(repo: &Path, commit: &str, dest: &Path) -> Result<(), FetchError> {
        std::fs::create_dir_all(dest).map_err(|e| FetchError::plain(e.to_string()))?;
        let output = git(Some(repo))
            .args(["archive", "--format=tar", commit])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| FetchError::plain(format!("cannot run `git archive`: {e}")))?;
        if !output.status.success() {
            return Err(FetchError::plain(format!(
                "cannot export commit {commit}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        safety::extract_tar(&output.stdout[..], dest).map_err(|e| match e {
            safety::ExtractError::Io(e) => FetchError::plain(format!("cannot export: {e}")),
            safety::ExtractError::Unsafe(u) => unsafe_error(&repo.display().to_string(), u),
        })
    }

    /// Submodules (opt-in, spec §10.3): a full clone at `commit`, with the
    /// submodule commits the parent pins.
    fn git_export_with_submodules(
        &self,
        url: &str,
        commit: &str,
        dest: &Path,
    ) -> Result<(), FetchError> {
        if self.offline {
            return Err(FetchError::offline(&format!("submodules of `{url}`")));
        }
        self.say("Cloning", &format!("{url} (with submodules)"));
        run_net(
            url,
            || {
                let _ = cache::remove(dest);
                let mut c = git(None);
                c.args(["clone", "--quiet", "--no-checkout", url]).arg(dest);
                c
            },
            &format!("cannot clone `{url}`"),
        )?;
        let mut c = git(None);
        c.arg("-C")
            .arg(dest)
            .args(["checkout", "--quiet", "--detach", commit]);
        run(c, &format!("cannot check out {commit}"))?;
        run_net(
            url,
            || {
                let mut c = git(None);
                c.arg("-C").arg(dest).args([
                    "submodule",
                    "update",
                    "--quiet",
                    "--init",
                    "--recursive",
                ]);
                c
            },
            &format!("cannot fetch the submodules of `{url}`"),
        )?;
        // Version-control metadata is not part of the package.
        for entry in walkdir::WalkDir::new(dest).into_iter().flatten() {
            if entry.file_name() == ".git" {
                let _ = cache::remove(entry.path());
            }
        }
        Ok(())
    }

    /// Tags of a repository (for `outdated`).
    pub fn git_tags(&self, url: &str) -> Result<Vec<String>, FetchError> {
        let dir = self.git_repo(url, true)?;
        let mut c = git(Some(&dir));
        c.args(["tag", "--list"]);
        Ok(run(c, &format!("cannot list the tags of `{url}`"))?
            .lines()
            .map(str::to_string)
            .collect())
    }

    // ---- svn ---------------------------------------------------------------

    /// The revision to use: the explicit one, the `locked` one, or the last
    /// revision that changed `url`.
    pub fn svn_revision(
        &self,
        url: &str,
        rev: Option<u64>,
        locked: Option<u64>,
    ) -> Result<u64, FetchError> {
        if let Some(r) = rev.or(locked) {
            return Ok(r);
        }
        if self.offline {
            return Err(FetchError::offline(&format!(
                "the latest revision of `{url}`"
            )));
        }
        self.say("Updating", url);
        let info = || {
            let mut c = Command::new("svn");
            c.args([
                "info",
                "--non-interactive",
                "--show-item",
                "last-changed-revision",
                url,
            ]);
            c
        };
        let text = run_net(url, info, &format!("cannot read `{url}`")).map_err(|e| FetchError {
            code: Some("E4102"),
            ..e
        })?;
        text.trim()
            .parse()
            .map_err(|_| FetchError::plain(format!("`svn info` gave no revision for `{url}`")))
    }

    /// The files of `url` at `rev`, exported into the cache. SVN externals
    /// are fetched only when opted in, like git submodules.
    pub fn svn_checkout(
        &self,
        url: &str,
        rev: u64,
        externals: bool,
    ) -> Result<PathBuf, FetchError> {
        let name = format!(
            "svn-{}-{}-r{rev}{}",
            cache::label(url),
            cache::key(url),
            if externals { "-ext" } else { "" }
        );
        let target = self.cache.checkout_dir(&name);
        if target.exists() {
            self.cache.touch(&target);
            return Ok(target);
        }
        if self.offline {
            return Err(FetchError::offline(&format!("revision {rev} of `{url}`")));
        }
        self.say("Fetching", &format!("{url}@{rev}"));
        let staged = cache::staging(&target).map_err(|e| FetchError::plain(e.to_string()))?;
        let export = || {
            let _ = cache::remove(&staged);
            let mut c = Command::new("svn");
            c.args(["export", "--quiet", "--non-interactive"]);
            if !externals {
                c.arg("--ignore-externals");
            }
            c.arg(format!("{url}@{rev}")).arg(&staged);
            c
        };
        if let Err(e) = run_net(url, export, &format!("cannot export `{url}`@{rev}")) {
            let _ = cache::remove(&staged);
            return Err(FetchError {
                code: Some("E4102"),
                ..e
            });
        }
        // A single-file URL exports a file, not a directory.
        if staged.is_file() {
            let file = cache::staging(&target).map_err(|e| FetchError::plain(e.to_string()))?;
            std::fs::rename(&staged, &file).map_err(|e| FetchError::plain(e.to_string()))?;
            std::fs::create_dir_all(&staged).map_err(|e| FetchError::plain(e.to_string()))?;
            let base = url.rsplit('/').next().unwrap_or("file.lua");
            std::fs::rename(&file, staged.join(base))
                .map_err(|e| FetchError::plain(e.to_string()))?;
        }
        if let Err(u) = safety::check_tree(&staged) {
            let _ = cache::remove(&staged);
            return Err(unsafe_error(url, u));
        }
        cache::commit(&staged, &target).map_err(|e| FetchError::plain(e.to_string()))?;
        self.cache.touch(&target);
        Ok(target)
    }

    /// The packager project file (`.pkgmeta`) of the SVN project a
    /// `…/trunk/<sub>` or `…/tags/<tag>/<sub>` URL points into, with
    /// `<sub>`: the packager reads it for an external's `ignore` and
    /// `plain-copy`. Cached per revision; `None` when there is none.
    pub fn svn_root_pkgmeta(&self, url: &str, rev: u64) -> Option<(PathBuf, String)> {
        let (root, sub) = svn_project_root(url)?;
        let entry = self.cache.checkout_dir(&format!(
            "svn-{}-{}-r{rev}-pkgmeta",
            cache::label(root),
            cache::key(root)
        ));
        if !entry.exists() {
            if self.offline {
                return None;
            }
            let output = Command::new("svn")
                .args(["cat", "--non-interactive"])
                .arg(format!("{root}/.pkgmeta@{rev}"))
                .stdin(Stdio::null())
                .output()
                .ok()?;
            let missing = String::from_utf8_lossy(&output.stderr).contains("160013");
            if !output.status.success() && !missing {
                return None;
            }
            let staged = cache::staging(&entry).ok()?;
            std::fs::create_dir_all(&staged).ok()?;
            if output.status.success() {
                std::fs::write(staged.join(".pkgmeta"), &output.stdout).ok()?;
            }
            cache::commit(&staged, &entry).ok()?;
        }
        self.cache.touch(&entry);
        let file = entry.join(".pkgmeta");
        file.is_file().then(|| (file, sub.to_string()))
    }

    /// The newest revision of `url` (for `outdated`).
    pub fn svn_latest(&self, url: &str) -> Result<u64, FetchError> {
        self.svn_revision(url, None, None)
    }

    // ---- archives -------------------------------------------------------------

    /// The extracted archive (before `strip-prefix`), downloaded and
    /// verified against `sha256` if not cached.
    pub fn archive(&self, url: &str, sha256: &str) -> Result<PathBuf, FetchError> {
        let file = self.cache.archive_file(sha256);
        let mut extracted = file.clone().into_os_string();
        extracted.push(".d");
        let extracted = PathBuf::from(extracted);
        if extracted.exists() {
            self.cache.touch(&extracted);
            return Ok(extracted);
        }
        let bytes = if file.exists() {
            std::fs::read(&file).map_err(|e| FetchError::plain(e.to_string()))?
        } else {
            if self.offline {
                return Err(FetchError::offline(&format!("archive `{url}`")));
            }
            let bytes = self.download(url)?;
            let actual = crate::checksum::sha256_hex(&bytes);
            if actual != sha256 {
                return Err(FetchError::coded(
                    "E4103",
                    format!(
                        "checksum mismatch for `{url}`: expected sha256 {sha256}, got {actual}"
                    ),
                )
                .help("check the URL, or update `sha256` if the archive legitimately changed"));
            }
            let staged = cache::staging(&file).map_err(|e| FetchError::plain(e.to_string()))?;
            std::fs::write(&staged, &bytes).map_err(|e| FetchError::plain(e.to_string()))?;
            cache::commit(&staged, &file).map_err(|e| FetchError::plain(e.to_string()))?;
            bytes
        };
        let staged = cache::staging(&extracted).map_err(|e| FetchError::plain(e.to_string()))?;
        std::fs::create_dir_all(&staged).map_err(|e| FetchError::plain(e.to_string()))?;
        let result = if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
            safety::extract_zip(&bytes, &staged)
        } else if bytes.starts_with(&[0x1f, 0x8b]) {
            safety::extract_tar(flate2::read::GzDecoder::new(&bytes[..]), &staged)
        } else {
            let _ = cache::remove(&staged);
            return Err(FetchError::plain(format!(
                "`{url}` is neither a `.zip` nor a `.tar.gz` archive"
            )));
        };
        if let Err(e) = result {
            let _ = cache::remove(&staged);
            return Err(match e {
                safety::ExtractError::Io(e) => {
                    FetchError::plain(format!("cannot extract `{url}`: {e}"))
                }
                safety::ExtractError::Unsafe(u) => unsafe_error(url, u),
            });
        }
        cache::commit(&staged, &extracted).map_err(|e| FetchError::plain(e.to_string()))?;
        self.cache.touch(&extracted);
        Ok(extracted)
    }

    /// Downloads over HTTPS; `file://` URLs read local files.
    fn download(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        if let Some(path) = url.strip_prefix("file://") {
            // `file:///C:/x` names `C:/x` on Windows.
            let b = path.as_bytes();
            let path = if b.len() > 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':'
            {
                &path[1..]
            } else {
                path
            };
            return std::fs::read(path)
                .map_err(|e| FetchError::coded("E4102", format!("cannot read `{url}`: {e}")));
        }
        if !url.starts_with("https://") {
            return Err(FetchError::plain(format!(
                "archive `{url}` must be downloaded over HTTPS"
            )));
        }
        self.download_https(url)
    }

    /// Without downloads (the browser's build): an archive must be in the
    /// cache or on disk.
    #[cfg(not(feature = "net"))]
    fn download_https(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        Err(FetchError::coded(
            "E4102",
            format!("cannot download `{url}`: this build of wowlua has no downloads"),
        ))
    }

    #[cfg(feature = "net")]
    fn download_https(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        self.say("Downloading", url);
        let mut delay = std::time::Duration::from_secs(2);
        let mut attempt = 1;
        let response = loop {
            match ureq::get(url).call() {
                Ok(r) => break r,
                Err(_) if attempt < 3 => {
                    std::thread::sleep(delay);
                    delay *= 2;
                    attempt += 1;
                }
                Err(e) => {
                    return Err(FetchError::coded(
                        "E4102",
                        format!("cannot download `{url}`: {e}"),
                    ));
                }
            }
        };
        response
            .into_body()
            .with_config()
            .limit(512 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| FetchError::plain(format!("cannot download `{url}`: {e}")))
    }
}

/// Downloads `url` and returns its SHA-256 (for converting `.pkgmeta` zip
/// externals, which carry no checksum).
pub fn sha256_of_url(url: &str) -> Result<String, String> {
    let fetcher = Fetcher::new(Cache::at(std::env::temp_dir()), false, |_, _| {});
    let bytes = fetcher.download(url).map_err(|e| e.message)?;
    Ok(crate::checksum::sha256_hex(&bytes))
}

fn unsafe_error(source: &str, u: safety::Unsafe) -> FetchError {
    FetchError::coded(
        "E4323",
        format!(
            "`{source}` contains an unsafe entry `{}`: {}",
            u.path, u.reason
        ),
    )
    .help("wowlua refuses to extract packages that could write outside their folder")
}

/// Repository values of a fetched package, for packager keywords in its
/// files (like the packager's `set_info_git` / `set_info_svn`).
#[derive(Clone, Debug, Default)]
pub struct PackageVcs {
    pub version: String,
    pub hash: String,
    pub short_hash: String,
    pub revision: String,
    pub author: String,
    pub timestamp: i64,
}

/// The repository values of `source` at `pin`. Git values come from the
/// cached bare clone; SVN author and date need `svn info` (`detail`).
pub fn package_vcs(cache: &Cache, source: &crate::Source, pin: &str, detail: bool) -> PackageVcs {
    let mut v = PackageVcs::default();
    match source {
        crate::Source::Git { url, reference, .. } => {
            let dir = cache.git_dir(url);
            let q = |args: &[&str]| -> String {
                let mut c = git(Some(&dir));
                c.args(args);
                run(c, "").unwrap_or_default()
            };
            v.hash = pin.to_string();
            v.short_hash = pin.chars().take(7).collect();
            let line = q(&["log", "-1", "--format=%an%x09%at", pin]);
            if let Some((a, t)) = line.split_once('\t') {
                v.author = a.to_string();
                v.timestamp = t.parse().unwrap_or(0);
            }
            // The packager clones branches and tags with `--depth 1`: no
            // history, so the revision is 1 and the version the tag or the
            // short hash; only `commit` externals are full clones.
            match reference {
                crate::GitRef::Tag(t) if t == LATEST_TAG => {
                    // Checked out in a 50-commit clone: the version is the
                    // tag, the revision what that clone holds of its history.
                    match latest_tag(&dir).filter(|(_, commit)| commit == pin) {
                        Some((tag, _)) => {
                            v.version = tag;
                            let recent = q(&["rev-list", "--max-count=50", "HEAD"]);
                            let depth = recent.lines().position(|c| c == pin).unwrap_or(0);
                            v.revision = (50 - depth).to_string();
                        }
                        None => {
                            v.version = v.short_hash.clone();
                            v.revision = q(&["rev-list", "--count", "--max-count=50", pin]);
                        }
                    }
                }
                crate::GitRef::Tag(t) => {
                    v.revision = "1".into();
                    v.version = t.clone();
                }
                crate::GitRef::Branch(_) | crate::GitRef::Head => {
                    v.revision = "1".into();
                    // A shallow clone still has the tags on its commit.
                    let tags = q(&[
                        "tag",
                        "--points-at",
                        pin,
                        "--sort=-committerdate",
                        "--sort=-v:refname",
                        "--sort=-taggerdate",
                    ]);
                    v.version = match tags.lines().next() {
                        Some(t) if !t.is_empty() => t.to_string(),
                        _ => v.short_hash.clone(),
                    };
                }
                crate::GitRef::Rev(_) => {
                    v.revision = q(&["rev-list", "--count", pin]);
                    let tags = q(&["tag", "--points-at", pin, "--sort=-v:refname"]);
                    v.version = match tags.lines().next() {
                        Some(t) if !t.is_empty() => t.to_string(),
                        _ => {
                            let d = q(&["describe", "--tags", "--abbrev=7", pin]);
                            if d.is_empty() {
                                v.short_hash.clone()
                            } else {
                                d
                            }
                        }
                    };
                }
            }
        }
        crate::Source::Svn { url, .. } => {
            v.revision = pin.to_string();
            v.version = match url.split_once("/tags/") {
                Some((_, rest)) => rest.split('/').next().unwrap_or(rest).to_string(),
                None => format!("r{pin}"),
            };
            if detail {
                let mut c = Command::new("svn");
                c.args(["info", "--non-interactive"])
                    .arg(format!("{url}@{pin}"));
                let mut root = None;
                for line in run(c, "").unwrap_or_default().lines() {
                    if let Some(a) = line.strip_prefix("Last Changed Author: ") {
                        v.author = a.to_string();
                    }
                    if let Some(d) = line.strip_prefix("Last Changed Date: ") {
                        v.timestamp = svn_date(d).unwrap_or(0);
                    }
                    if let Some(r) = line.strip_prefix("Repository Root: ") {
                        root = Some(r.to_string());
                    }
                }
                // A trunk checkout whose revision the newest tag was copied
                // from is that tag (the packager's `set_info_svn`).
                if let Some(root) = root.filter(|_| !url.contains("/tags/")) {
                    let mut c = Command::new("svn");
                    c.args(["log", "--non-interactive", "--verbose", "--limit", "1"])
                        .arg(format!("{root}/tags"));
                    let log = run(c, "").unwrap_or_default();
                    if let Some(line) = log.lines().find(|l| l.starts_with("   A ")) {
                        let path = line[5..].split_whitespace().next().unwrap_or("");
                        let from = line
                            .rsplit_once(':')
                            .map(|(_, r)| r.trim_end_matches(')'))
                            .unwrap_or("");
                        if from == pin
                            && let Some(tag) = path.rsplit('/').next().filter(|t| !t.is_empty())
                        {
                            v.version = tag.to_string();
                            // The revision becomes the tag's (its copy).
                            let mut c = Command::new("svn");
                            c.args([
                                "info",
                                "--non-interactive",
                                "--show-item",
                                "last-changed-revision",
                            ])
                            .arg(format!("{root}/tags/{tag}"));
                            if let Ok(rev) = run(c, "") {
                                let rev = rev.trim();
                                if !rev.is_empty() {
                                    v.revision = rev.to_string();
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    v
}

/// A git `tag` of `latest`: the newest tag, like the packager's.
pub const LATEST_TAG: &str = "latest";

/// The newest tag (by creation date) among the last 50 commits of the
/// default branch of a cached clone, and its commit.
fn latest_tag(dir: &Path) -> Option<(String, String)> {
    let q = |args: &[&str]| -> String {
        let mut c = git(Some(dir));
        c.args(args);
        run(c, "").unwrap_or_default()
    };
    let recent: HashSet<String> = q(&["rev-list", "--max-count=50", "HEAD"])
        .lines()
        .map(str::to_string)
        .collect();
    q(&[
        "for-each-ref",
        "refs/tags",
        "--sort=-creatordate",
        "--format=%(refname:short)",
    ])
    .lines()
    .find_map(|tag| {
        let commit = q(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/tags/{tag}^{{commit}}"),
        ]);
        recent.contains(&commit).then(|| (tag.to_string(), commit))
    })
}

/// The `@file-*@` values of one file of a package.
#[derive(Clone, Debug, Default)]
pub struct FileVcs {
    pub revision: String,
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub timestamp: i64,
}

/// The `@file-*@` values of `rel` (a path inside the package) as the
/// packager's `set_info_file` computes them for an external: in a shallow
/// clone (tag and branch externals) every file has the checked-out commit
/// and revision 1; `commit` externals have their full history; SVN files
/// their last change and checksum.
pub fn package_file_vcs(
    cache: &Cache,
    source: &crate::Source,
    pin: &str,
    rel: &str,
    bytes: &[u8],
    project: &PackageVcs,
) -> FileVcs {
    let mut f = FileVcs::default();
    match source {
        crate::Source::Git {
            url,
            reference,
            subdir,
            ..
        } => {
            if !matches!(reference, crate::GitRef::Rev(_)) {
                return FileVcs {
                    revision: "1".into(),
                    hash: project.hash.clone(),
                    short_hash: project.short_hash.clone(),
                    author: project.author.clone(),
                    timestamp: project.timestamp,
                };
            }
            let dir = cache.git_dir(url);
            let q = |args: &[&str]| -> String {
                let mut c = git(Some(&dir));
                c.args(args);
                run(c, "").unwrap_or_default()
            };
            let path = match subdir {
                Some(s) => format!("{s}/{rel}"),
                None => rel.to_string(),
            };
            let line = q(&[
                "log",
                "-1",
                "--abbrev=7",
                "--format=%H%x09%h%x09%an%x09%at",
                pin,
                "--",
                &path,
            ]);
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() == 4 {
                f.hash = parts[0].into();
                f.short_hash = parts[1].into();
                f.author = parts[2].into();
                f.timestamp = parts[3].parse().unwrap_or(0);
                f.revision = q(&["rev-list", "--count", parts[0]]);
            }
        }
        crate::Source::Svn { url, .. } => {
            let mut c = Command::new("svn");
            c.args(["info", "--non-interactive"])
                .arg(format!("{url}/{rel}@{pin}"));
            for line in run(c, "").unwrap_or_default().lines() {
                if let Some(a) = line.strip_prefix("Last Changed Author: ") {
                    f.author = a.to_string();
                } else if let Some(r) = line.strip_prefix("Last Changed Rev: ") {
                    f.revision = r.to_string();
                } else if let Some(d) = line.strip_prefix("Last Changed Date: ") {
                    f.timestamp = svn_date(d).unwrap_or(0);
                }
            }
            use sha1::Digest;
            f.hash = sha1::Sha1::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            f.short_hash = f.hash.chars().take(7).collect();
        }
        _ => {}
    }
    f
}

/// Unix seconds of an `svn info` date: `2020-01-02 03:04:05 +0100 (…)`.
fn svn_date(text: &str) -> Option<i64> {
    let mut parts = text.split_whitespace();
    let date = parts.next()?;
    let time = parts.next()?;
    let zone = parts.next()?;
    let d: Vec<i64> = date
        .split('-')
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    let t: Vec<i64> = time
        .split(':')
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    if d.len() != 3 || t.len() != 3 || zone.len() != 5 {
        return None;
    }
    let (y, m, day) = (d[0] - i64::from(d[1] <= 2), d[1], d[2]);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let offset =
        sign * (zone[1..3].parse::<i64>().ok()? * 3600 + zone[3..5].parse::<i64>().ok()? * 60);
    Some(days * 86_400 + t[0] * 3600 + t[1] * 60 + t[2] - offset)
}

/// The project root of an SVN URL inside `trunk` or a tag, and the path
/// below it: `…/wow/ace3/trunk/AceGUI-3.0` → (`…/wow/ace3/trunk`,
/// `AceGUI-3.0`).
pub fn svn_project_root(url: &str) -> Option<(&str, &str)> {
    if let Some(i) = url.find("/trunk/") {
        return Some((&url[..i + "/trunk".len()], &url[i + "/trunk/".len()..]));
    }
    let i = url.find("/tags/")? + "/tags/".len();
    let j = url[i..].find('/')? + i;
    Some((&url[..j], &url[j + 1..]))
}

#[cfg(test)]
mod svn_date_tests {
    #[test]
    fn parses_svn_dates() {
        assert_eq!(
            super::svn_date("2008-05-01 12:34:56 +0000 (Thu, 01 May 2008)"),
            Some(1_209_645_296)
        );
        assert_eq!(
            super::svn_date("2008-05-01 14:34:56 +0200"),
            Some(1_209_645_296)
        );
    }
}
