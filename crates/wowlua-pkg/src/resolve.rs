//! Resolution (spec §10.4): every dependency of every root (an addon or
//! library of the workspace), over the union of features, flavors and
//! `dev-dependencies`, is materialized in the cache; PubGrub then picks one
//! version per package per root and explains conflicts (E4101).

use crate::checksum;
use crate::fetch::{FetchError, Fetcher};
use crate::lock::LockedPackage;
use crate::source::Source;
use pubgrub::{DefaultStringReporter, OfflineDependencyProvider, PubGrubError, Ranges, Reporter};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::model::{Dependency, DependencyDetail, Manifest, PackageKind};
use wowlua_span::SourceDb;

/// A package whose dependencies are resolved: an addon or library of the
/// workspace.
#[derive(Clone, Debug)]
pub struct Root {
    /// Its package name (`used-by` in the lockfile).
    pub name: String,
    pub version: semver::Version,
    /// The directory of its manifest.
    pub dir: PathBuf,
    pub manifest_path: PathBuf,
    /// `dependencies` and `dev-dependencies`, all features and flavors.
    pub deps: Vec<(String, Dependency)>,
}

/// Which locked pins may move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// Keep every pin that still matches its declaration.
    None,
    /// `wowlua update`.
    All,
    /// `wowlua update <names>`.
    Only(Vec<String>),
}

impl Update {
    fn allows(&self, name: &str) -> bool {
        match self {
            Update::None => false,
            Update::All => true,
            Update::Only(names) => names.iter().any(|n| n == name),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: semver::Version,
    pub source: Source,
    /// The commit, revision or archive checksum.
    pub pin: Option<String>,
    /// The package root (after `subdir` / `strip-prefix`).
    pub dir: PathBuf,
    pub checksum: Option<String>,
    /// Its `wowlua.json`, if it has one.
    pub manifest: Option<Manifest>,
    pub kind: PackageKind,
    /// The declaration that brought it in (a root's entry when there is one).
    pub entry: DependencyDetail,
    /// Names of its own resolved dependencies.
    pub dependencies: Vec<String>,
    pub used_by: Vec<String>,
}

impl ResolvedPackage {
    pub fn lock_entry(&self, lock_root: &Path) -> LockedPackage {
        let key = self.source.lock_key(lock_root);
        LockedPackage {
            name: self.name.clone(),
            version: self.version.to_string(),
            source: match &self.pin {
                Some(pin) => format!("{key}#{pin}"),
                None => key,
            },
            checksum: self.checksum.clone(),
            dependencies: self.dependencies.clone(),
            used_by: self.used_by.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Resolution {
    /// Sorted by name, then version.
    pub packages: Vec<ResolvedPackage>,
}

impl Resolution {
    pub fn lock_entries(&self, lock_root: &Path) -> Vec<LockedPackage> {
        self.packages
            .iter()
            .map(|p| p.lock_entry(lock_root))
            .collect()
    }

    /// The packages one root uses.
    pub fn of_root<'a>(&'a self, root: &'a str) -> impl Iterator<Item = &'a ResolvedPackage> {
        self.packages
            .iter()
            .filter(move |p| p.used_by.iter().any(|u| u == root))
    }
}

pub struct Options<'a> {
    pub lock: &'a [LockedPackage],
    /// Where the lockfile lives (path sources are written relative to it).
    pub lock_root: &'a Path,
    pub update: &'a Update,
    pub compiler: &'a semver::Version,
    /// Workspace members by package name: dependencies with these names
    /// resolve to them (spec §9.15).
    pub members: &'a HashMap<String, PathBuf>,
    /// `wowlua update <name> --precise <commit|revision>`.
    pub precise: Option<(&'a str, &'a str)>,
}

/// One materialized package version.
#[derive(Clone, Debug)]
struct Materialized {
    version: semver::Version,
    pin: Option<String>,
    dir: PathBuf,
    checksum: Option<String>,
    manifest: Option<Manifest>,
}

/// A dependency request, while walking the graph.
struct Request {
    name: String,
    dep: Dependency,
    /// Directory of the declaring manifest.
    base: PathBuf,
    /// The package declaring it (`None`: the root).
    requester: Option<(String, semver::Version)>,
}

fn diag(error: FetchError, name: &str) -> Diagnostic {
    let message = format!("dependency `{name}`: {}", error.message);
    let mut d = match error.code {
        Some(code) => Diagnostic::error(code, message),
        None => Diagnostic::plain_error(message),
    };
    if let Some(help) = error.help {
        d = d.with_help(help);
    }
    d
}

/// Resolves and fetches the dependencies of every root.
pub fn resolve(
    db: &mut SourceDb,
    roots: &[Root],
    fetcher: &Fetcher,
    options: &Options,
) -> Result<Resolution, Vec<Diagnostic>> {
    let mut errors: Vec<Diagnostic> = Vec::new();
    // (identity, lock key) → the materialized package.
    let mut materialized: HashMap<(String, String), Materialized> = HashMap::new();
    // name → (identity, who asked first) for E4321.
    let mut identities: HashMap<String, (String, String, String)> = HashMap::new();
    // name → version → (source, deps (name, version), entry, kind).
    type Node = (
        Source,
        Vec<(String, semver::Version)>,
        DependencyDetail,
        PackageKind,
    );
    let mut nodes: BTreeMap<String, BTreeMap<semver::Version, Node>> = BTreeMap::new();
    let mut root_deps: Vec<Vec<(String, semver::Version)>> = Vec::new();

    for root in roots {
        let mut direct: Vec<(String, semver::Version)> = Vec::new();
        let mut queue: VecDeque<Request> = root
            .deps
            .iter()
            .map(|(name, dep)| Request {
                name: name.clone(),
                dep: dep.clone(),
                base: root.dir.clone(),
                requester: None,
            })
            .collect();
        let mut visited: BTreeSet<(String, String, String)> = BTreeSet::new();
        while let Some(req) = queue.pop_front() {
            let mut source = Source::of(&req.dep, &req.base);
            if let Some(dir) = options.members.get(&req.name)
                && matches!(source, Source::Registry { .. })
            {
                source = Source::Path {
                    path: crate::source::relative(dir, &req.base)
                        .unwrap_or_else(|| dir.display().to_string()),
                    dir: dir.clone(),
                };
            }
            let requester_name = req
                .requester
                .as_ref()
                .map(|(n, _)| n.clone())
                .unwrap_or_else(|| root.name.clone());
            if let Source::Registry { req: requirement } = &source {
                errors.push(
                    Diagnostic::error(
                        "E4102",
                        format!(
                            "dependency `{}` (`{requirement}`, required by `{requester_name}`) needs the wowlua registry, which is not available yet",
                            req.name
                        ),
                    )
                    .with_path(&root.manifest_path)
                    .with_help("use a `git`, `svn`, `path` or `archive` source for now"),
                );
                continue;
            }
            let identity = source.identity();
            match identities.get(&req.name) {
                Some((other, first_by, other_desc)) if *other != identity => {
                    errors.push(
                        Diagnostic::error(
                            "E4321",
                            format!("package `{}` comes from two different sources", req.name),
                        )
                        .with_path(&root.manifest_path)
                        .with_note(format!("`{first_by}` uses {other_desc}"))
                        .with_note(format!("`{requester_name}` uses {}", source.describe()))
                        .with_help("make every package that depends on it use the same source"),
                    );
                    continue;
                }
                Some(_) => {}
                None => {
                    identities.insert(
                        req.name.clone(),
                        (identity.clone(), requester_name.clone(), source.describe()),
                    );
                }
            }
            let key = source.lock_key(options.lock_root);
            let m = match materialized.get(&(identity.clone(), key.clone())) {
                Some(m) => m.clone(),
                None => match materialize(db, &req.name, &source, &key, fetcher, options) {
                    Ok(m) => {
                        materialized.insert((identity.clone(), key.clone()), m.clone());
                        m
                    }
                    Err(mut ds) => {
                        for d in &mut ds {
                            if d.labels.is_empty() && d.path.is_none() {
                                *d = d.clone().with_path(&root.manifest_path);
                            }
                        }
                        errors.extend(ds);
                        continue;
                    }
                },
            };
            let entry = match &req.dep {
                Dependency::Detailed(d) => (**d).clone(),
                Dependency::Version(_) => DependencyDetail::default(),
            };
            let kind = match entry.kind.or_else(|| {
                m.manifest
                    .as_ref()
                    .and_then(|mf| mf.package.as_ref().map(|p| p.kind))
            }) {
                Some(kind) => kind,
                None => {
                    errors.push(
                        Diagnostic::plain_error(format!(
                            "dependency `{}` has no wowlua.json, so its kind is unknown",
                            req.name
                        ))
                        .with_path(&root.manifest_path)
                        .with_help(
                            "add `\"kind\": \"lua-library\"` (or `types`) to the dependency",
                        ),
                    );
                    continue;
                }
            };
            if kind == PackageKind::Addon {
                errors.push(
                    Diagnostic::plain_error(format!(
                        "dependency `{}` is an addon; addons cannot be embedded",
                        req.name
                    ))
                    .with_path(&root.manifest_path)
                    .with_help("list it in `addon-dependencies` instead"),
                );
                continue;
            }
            match &req.requester {
                None => direct.push((req.name.clone(), m.version.clone())),
                Some((pname, pversion)) => {
                    if let Some((_, deps, _, _)) =
                        nodes.get_mut(pname).and_then(|v| v.get_mut(pversion))
                        && !deps.iter().any(|(n, v)| n == &req.name && v == &m.version)
                    {
                        deps.push((req.name.clone(), m.version.clone()));
                    }
                }
            }
            let fresh = !nodes
                .get(&req.name)
                .is_some_and(|v| v.contains_key(&m.version));
            if fresh {
                nodes
                    .entry(req.name.clone())
                    .or_default()
                    .insert(m.version.clone(), (source.clone(), Vec::new(), entry, kind));
            }
            if visited.insert((req.name.clone(), m.version.to_string(), key)) {
                for (n, d) in m
                    .manifest
                    .as_ref()
                    .map(|mf| mf.dependencies.clone())
                    .unwrap_or_default()
                {
                    queue.push_back(Request {
                        name: n,
                        dep: d,
                        base: m.dir.clone(),
                        requester: Some((req.name.clone(), m.version.clone())),
                    });
                }
            }
        }
        root_deps.push(direct);
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // One version per package per root (PubGrub over the materialized graph).
    let mut used_by: BTreeMap<(String, semver::Version), BTreeSet<String>> = BTreeMap::new();
    for (root, direct) in roots.iter().zip(&root_deps) {
        let mut provider: OfflineDependencyProvider<String, Ranges<Ver>> =
            OfflineDependencyProvider::new();
        let root_key = format!("{} (root)", root.name);
        provider.add_dependencies(
            root_key.clone(),
            Ver(root.version.clone()),
            direct
                .iter()
                .map(|(n, v)| (n.clone(), Ranges::singleton(Ver(v.clone())))),
        );
        for (name, versions) in &nodes {
            for (version, (_, deps, _, _)) in versions {
                provider.add_dependencies(
                    name.clone(),
                    Ver(version.clone()),
                    deps.iter()
                        .map(|(n, v)| (n.clone(), Ranges::singleton(Ver(v.clone())))),
                );
            }
        }
        match pubgrub::resolve(&provider, root_key.clone(), Ver(root.version.clone())) {
            Ok(selected) => {
                for (name, version) in selected {
                    if name != root_key {
                        used_by
                            .entry((name, version.0))
                            .or_default()
                            .insert(root.name.clone());
                    }
                }
            }
            Err(PubGrubError::NoSolution(mut tree)) => {
                tree.collapse_no_versions();
                let mut d = Diagnostic::error(
                    "E4101",
                    format!("the dependencies of `{}` conflict", root.name),
                )
                .with_path(&root.manifest_path)
                .with_note(DefaultStringReporter::report(&tree).replace(&root_key, &root.name));
                // What the commit versions stand for.
                for (name, versions) in nodes.iter().filter(|(_, v)| v.len() > 1) {
                    for (version, (source, ..)) in versions {
                        d = d.with_note(format!("{name} {version} is {}", source.describe()));
                    }
                }
                errors.push(
                    d.with_help("make the declarations agree on one tag, branch or revision"),
                );
            }
            Err(e) => errors.push(Diagnostic::plain_error(format!(
                "cannot resolve the dependencies of `{}`: {e}",
                root.name
            ))),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut packages = Vec::new();
    for ((name, version), users) in used_by {
        let (source, deps, entry, kind) = nodes[&name][&version].clone();
        let key = source.lock_key(options.lock_root);
        let m = &materialized[&(source.identity(), key)];
        let mut dependencies: Vec<String> = deps.into_iter().map(|(n, _)| n).collect();
        dependencies.sort();
        dependencies.dedup();
        packages.push(ResolvedPackage {
            name,
            version,
            source,
            pin: m.pin.clone(),
            dir: m.dir.clone(),
            checksum: m.checksum.clone(),
            manifest: m.manifest.clone(),
            kind,
            entry,
            dependencies,
            used_by: users.into_iter().collect(),
        });
    }
    Ok(Resolution { packages })
}

/// Fetches one package version into the cache (or finds it there).
fn materialize(
    db: &mut SourceDb,
    name: &str,
    source: &Source,
    key: &str,
    fetcher: &Fetcher,
    options: &Options,
) -> Result<Materialized, Vec<Diagnostic>> {
    let locked = options
        .lock
        .iter()
        .find(|l| l.name == name && l.key() == key)
        .filter(|_| !options.update.allows(name));
    let mut pin = locked.and_then(|l| l.pin()).map(str::to_string);
    if let Some((_, precise)) = options.precise.filter(|(n, _)| *n == name) {
        pin = Some(match source {
            Source::Git {
                reference: crate::source::GitRef::Tag(_) | crate::source::GitRef::Rev(_),
                ..
            }
            | Source::Svn { rev: Some(_), .. } => {
                return Err(vec![Diagnostic::plain_error(format!(
                    "dependency `{name}` is pinned by its declaration (`tag` / `rev`); change the manifest instead of using `--precise`"
                ))]);
            }
            Source::Git { url, .. } => fetcher
                .git_commit(url, &crate::source::GitRef::Rev(precise.to_string()), None)
                .map_err(|e| vec![diag(e, name)])?,
            Source::Svn { .. } if precise.parse::<u64>().is_ok() => precise.to_string(),
            _ => {
                return Err(vec![Diagnostic::plain_error(format!(
                    "`--precise {precise}` needs a git commit or an SVN revision number for dependency `{name}`"
                ))]);
            }
        });
    }
    let pin = pin.as_deref();
    let (dir, pin) = match source {
        Source::Path { path, dir } => {
            if !dir.join("wowlua.json").exists() && !dir.is_dir() {
                return Err(vec![Diagnostic::error(
                    "E4102",
                    format!("dependency `{name}`: path `{path}` does not exist"),
                )]);
            }
            (dir.clone(), None)
        }
        Source::Git {
            url,
            reference,
            subdir,
            submodules,
        } => {
            let commit = fetcher
                .git_commit(url, reference, pin)
                .map_err(|e| vec![diag(e, name)])?;
            let checkout = fetcher
                .git_checkout(url, &commit, *submodules)
                .map_err(|e| vec![diag(e, name)])?;
            let dir = match subdir {
                Some(s) => checkout.join(s),
                None => checkout,
            };
            if !dir.is_dir() {
                return Err(vec![Diagnostic::error(
                    "E4102",
                    format!(
                        "dependency `{name}`: `{url}` has no folder `{}` at commit {commit}",
                        subdir.as_deref().unwrap_or("")
                    ),
                )]);
            }
            (dir, Some(commit))
        }
        Source::Svn {
            url,
            rev,
            externals,
        } => {
            let locked_rev = pin.and_then(|p| p.parse().ok());
            let revision = fetcher
                .svn_revision(url, *rev, locked_rev)
                .map_err(|e| vec![diag(e, name)])?;
            let dir = fetcher
                .svn_checkout(url, revision, *externals)
                .map_err(|e| vec![diag(e, name)])?;
            (dir, Some(revision.to_string()))
        }
        Source::Archive {
            url,
            sha256,
            strip_prefix,
        } => {
            let extracted = fetcher
                .archive(url, sha256)
                .map_err(|e| vec![diag(e, name)])?;
            let dir = match strip_prefix {
                Some(p) => extracted.join(p),
                None => extracted,
            };
            if !dir.is_dir() {
                return Err(vec![
                    Diagnostic::error(
                        "E4102",
                        format!(
                            "dependency `{name}`: `{url}` has no folder `{}`",
                            strip_prefix.as_deref().unwrap_or("")
                        ),
                    )
                    .with_help("check `strip-prefix`"),
                ]);
            }
            (dir, Some(sha256.clone()))
        }
        Source::Registry { .. } => unreachable!("registry sources are rejected earlier"),
    };
    let checksum = match source {
        Source::Path { .. } => None,
        _ => {
            let sum = checksum::tree(&dir).map_err(|e| {
                vec![Diagnostic::plain_error(format!(
                    "dependency `{name}`: cannot read `{}`: {e}",
                    dir.display()
                ))]
            })?;
            if let Some(expected) = locked.and_then(|l| l.checksum.as_deref())
                && expected != sum
            {
                return Err(vec![Diagnostic::error(
                    "E4103",
                    format!("checksum mismatch for dependency `{name}`: the lockfile has {expected}, the fetched files have {sum}"),
                )
                .with_note(format!("cached at {}", dir.display()))
                .with_help("if the source legitimately changed, run `wowlua update` for it; otherwise run `wowlua cache clean --all` and fetch again")]);
            }
            Some(sum)
        }
    };
    let manifest_path = dir.join("wowlua.json");
    let manifest = if manifest_path.is_file() {
        let output = wowlua_manifest::load(db, &manifest_path, options.compiler);
        let errors: Vec<Diagnostic> = output
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        output.manifest.map(|m| m.manifest)
    } else {
        None
    };
    if let Some(package) = manifest.as_ref().and_then(|m| m.package.as_ref())
        && package.name != name
    {
        return Err(vec![
            Diagnostic::error(
                "E4102",
                format!("dependency `{name}` points at package `{}`", package.name),
            )
            .with_help(format!("rename the dependency to `{}`", package.name)),
        ]);
    }
    let version = match manifest.as_ref().and_then(|m| m.package.as_ref()) {
        Some(p) => semver::Version::parse(&p.version).unwrap_or(semver::Version::new(0, 0, 0)),
        None => {
            let mut v = semver::Version::new(0, 0, 0);
            if let Some(p) = &pin {
                let short: String = p
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .take(12)
                    .collect();
                v.build = semver::BuildMetadata::new(&short).unwrap_or_default();
            }
            v
        }
    };
    Ok(Materialized {
        version,
        pin,
        dir,
        checksum,
        manifest,
    })
}

/// Versions for PubGrub: SemVer with build metadata (`0.0.0+<commit>`), so
/// two commits of one repository are different versions.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ver(pub semver::Version);

impl std::fmt::Display for Ver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
