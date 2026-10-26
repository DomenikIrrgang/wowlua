//! Dependency sources (spec §10.3) and how they are written in the lockfile.

use std::path::{Path, PathBuf};
use wowlua_manifest::model::{Dependency, DependencyDetail};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum GitRef {
    Tag(String),
    Branch(String),
    Rev(String),
    /// The remote's default branch.
    Head,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Relative to the declaring manifest; `dir` is absolute.
    Path { path: String, dir: PathBuf },
    Git {
        url: String,
        reference: GitRef,
        subdir: Option<String>,
        submodules: bool,
    },
    Svn {
        url: String,
        rev: Option<u64>,
        externals: bool,
    },
    Archive {
        url: String,
        sha256: String,
        strip_prefix: Option<String>,
    },
    /// A version requirement against a registry (spec §10.10); `url` is
    /// the registry's, without a trailing `/`.
    Registry { req: String, url: String },
}

/// The registries dependencies may name: the default one and the named ones
/// of `config.json`'s `registries` (spec CLI §12.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registries {
    pub default: String,
    pub named: std::collections::BTreeMap<String, String>,
}

impl Default for Registries {
    fn default() -> Self {
        Registries {
            default: crate::registry::DEFAULT_REGISTRY.into(),
            named: Default::default(),
        }
    }
}

impl Registries {
    /// The URL of a dependency's `registry` (a URL, or a name in
    /// `config.json`); a name that is neither stays as it is, and
    /// resolution reports it.
    pub fn url(&self, declared: Option<&str>) -> String {
        match declared {
            None => self.default.clone(),
            Some(d) if d.contains("://") => crate::registry::normalize_url(d),
            Some(d) => self
                .named
                .get(d)
                .map(|u| crate::registry::normalize_url(u))
                .unwrap_or_else(|| d.to_string()),
        }
    }
}

impl Source {
    /// The source of a dependency entry declared in the manifest at `base`.
    pub fn of(dep: &Dependency, base: &Path, registries: &Registries) -> Source {
        let d = match dep {
            Dependency::Version(req) => {
                return Source::Registry {
                    req: req.clone(),
                    url: registries.url(None),
                };
            }
            Dependency::Detailed(d) => d,
        };
        Self::of_detail(d, base, registries)
    }

    fn of_detail(d: &DependencyDetail, base: &Path, registries: &Registries) -> Source {
        if let Some(url) = &d.git {
            let reference = if let Some(t) = &d.tag {
                GitRef::Tag(t.clone())
            } else if let Some(b) = &d.branch {
                GitRef::Branch(b.clone())
            } else if let Some(r) = &d.rev {
                GitRef::Rev(r.clone())
            } else {
                GitRef::Head
            };
            return Source::Git {
                url: url.clone(),
                reference,
                subdir: d.subdir.clone().map(|s| s.trim_matches('/').to_string()),
                submodules: d.submodules,
            };
        }
        if let Some(url) = &d.svn {
            return Source::Svn {
                url: url.trim_end_matches('/').to_string(),
                rev: d.rev.as_deref().and_then(|r| r.parse().ok()),
                externals: d.submodules,
            };
        }
        if let Some(url) = &d.archive {
            return Source::Archive {
                url: url.clone(),
                sha256: d.sha256.clone().unwrap_or_default().to_ascii_lowercase(),
                strip_prefix: d
                    .strip_prefix
                    .clone()
                    .map(|s| s.trim_matches('/').to_string())
                    .filter(|s| !s.is_empty()),
            };
        }
        if let Some(path) = &d.path {
            return Source::Path {
                path: path.clone(),
                dir: normalize(&base.join(path)),
            };
        }
        Source::Registry {
            req: d.version.clone().unwrap_or_else(|| "*".into()),
            url: registries.url(d.registry.as_deref()),
        }
    }

    /// What makes two packages of the same name the same package (spec
    /// §10.4): the repository, path or URL, without the ref.
    pub fn identity(&self) -> String {
        match self {
            Source::Path { dir, .. } => format!("path+{}", dir.display()),
            Source::Git { url, subdir, .. } => match subdir {
                Some(s) => format!("git+{}#{s}", normalize_url(url)),
                None => format!("git+{}", normalize_url(url)),
            },
            Source::Svn { url, .. } => format!("svn+{}", normalize_url(url)),
            Source::Archive { url, .. } => format!("archive+{url}"),
            Source::Registry { url, .. } => format!("registry+{url}"),
        }
    }

    /// The source as the lockfile writes it, before the `#<pin>` part:
    /// `git+<url>?tag=1.0`, `svn+<url>`, `archive+<url>`, `path+<path>`.
    pub fn lock_key(&self, lock_root: &Path) -> String {
        match self {
            Source::Path { dir, .. } => {
                let rel = relative(dir, lock_root).unwrap_or_else(|| dir.display().to_string());
                format!("path+{rel}")
            }
            Source::Git {
                url,
                reference,
                subdir,
                submodules,
            } => {
                let mut query = Vec::new();
                match reference {
                    GitRef::Tag(t) => query.push(format!("tag={t}")),
                    GitRef::Branch(b) => query.push(format!("branch={b}")),
                    GitRef::Rev(r) => query.push(format!("rev={r}")),
                    GitRef::Head => {}
                }
                if let Some(s) = subdir {
                    query.push(format!("subdir={s}"));
                }
                if *submodules {
                    query.push("submodules=true".into());
                }
                if query.is_empty() {
                    format!("git+{url}")
                } else {
                    format!("git+{url}?{}", query.join("&"))
                }
            }
            Source::Svn {
                url,
                rev,
                externals,
            } => {
                let mut query = Vec::new();
                if let Some(r) = rev {
                    query.push(format!("rev={r}"));
                }
                if *externals {
                    query.push("externals=true".into());
                }
                if query.is_empty() {
                    format!("svn+{url}")
                } else {
                    format!("svn+{url}?{}", query.join("&"))
                }
            }
            Source::Archive { url, .. } => format!("archive+{url}"),
            Source::Registry { url, .. } => format!("registry+{url}"),
        }
    }

    /// A short description for messages: `git https://…?tag=1.0`.
    pub fn describe(&self) -> String {
        match self {
            Source::Path { path, .. } => format!("path {path}"),
            Source::Git { url, reference, .. } => match reference {
                GitRef::Tag(t) => format!("git {url} (tag {t})"),
                GitRef::Branch(b) => format!("git {url} (branch {b})"),
                GitRef::Rev(r) => format!("git {url} (rev {r})"),
                GitRef::Head => format!("git {url}"),
            },
            Source::Svn { url, rev, .. } => match rev {
                Some(r) => format!("svn {url}@{r}"),
                None => format!("svn {url}"),
            },
            Source::Archive { url, .. } => format!("archive {url}"),
            Source::Registry { req, url } if url == crate::registry::DEFAULT_REGISTRY => {
                format!("registry {req}")
            }
            Source::Registry { req, url } => format!("registry {url} {req}"),
        }
    }
}

/// URLs compared for identity: case-insensitive scheme and host, no
/// trailing `/` or `.git`.
pub fn normalize_url(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    match trimmed.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
            format!(
                "{}://{}/{}",
                scheme.to_ascii_lowercase(),
                host.to_ascii_lowercase(),
                path
            )
        }
        None => trimmed.to_string(),
    }
}

/// Lexically normalized path (`a/./b/../c` → `a/c`).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `path` relative to `base` with `/` separators (may use `..`).
pub fn relative(path: &Path, base: &Path) -> Option<String> {
    let path = normalize(path);
    let base = normalize(base);
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common == 0 && (p.first() != b.first()) {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in common..b.len() {
        parts.push("..".into());
    }
    for c in &p[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    Some(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_ignores_refs_and_url_spelling() {
        let a = Source::Git {
            url: "https://GitHub.com/x/LibStub.git".into(),
            reference: GitRef::Tag("1.0".into()),
            subdir: None,
            submodules: false,
        };
        let b = Source::Git {
            url: "https://github.com/x/LibStub/".into(),
            reference: GitRef::Head,
            subdir: None,
            submodules: false,
        };
        assert_eq!(a.identity(), b.identity());
        assert_eq!(
            a.lock_key(Path::new("/p")),
            "git+https://GitHub.com/x/LibStub.git?tag=1.0"
        );
    }

    #[test]
    fn relative_paths() {
        assert_eq!(
            relative(Path::new("/a/b/c"), Path::new("/a/d")).unwrap(),
            "../b/c"
        );
        assert_eq!(relative(Path::new("/a/b"), Path::new("/a/b")).unwrap(), ".");
    }
}
