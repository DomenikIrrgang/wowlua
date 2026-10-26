//! Resolution (spec §10.4): every dependency of every root (an addon or
//! library of the workspace), over the union of features, flavors and
//! `dev-dependencies`, is materialized in the cache; PubGrub then picks one
//! version per package per root and explains conflicts (E4101).

use crate::checksum;
use crate::fetch::{FetchError, Fetcher};
use crate::lock::LockedPackage;
use crate::registry::IndexEntry;
use crate::source::Registries;
use crate::source::Source;
use pubgrub::{
    DefaultStringReporter, DependencyProvider, OfflineDependencyProvider, PubGrubError, Ranges,
    Reporter,
};
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
    /// The packager project file (`.pkgmeta`) of the repository it comes
    /// from, and the package's path inside that repository: the packager
    /// applies its `ignore` and `plain-copy` to an external.
    pub project_file: Option<(PathBuf, String)>,
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
    /// Warnings of the resolution: a locked version that was yanked (W4322).
    pub warnings: Vec<Diagnostic>,
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
    /// `wowlua update <name> --precise <commit|revision|version>`.
    pub precise: Option<(&'a str, &'a str)>,
    /// The default and the named registries (`config.json`).
    pub registries: &'a Registries,
    /// Read every registry index again (`fetch`, `update`, `add`,
    /// `outdated`); otherwise a cached index that has the locked versions
    /// is used as it is.
    pub refresh: bool,
}

/// One materialized package version.
#[derive(Clone, Debug)]
struct Materialized {
    version: semver::Version,
    pin: Option<String>,
    dir: PathBuf,
    checksum: Option<String>,
    manifest: Option<Manifest>,
    project_file: Option<(PathBuf, String)>,
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

/// One version of a package in the graph.
#[derive(Clone, Debug)]
struct Node {
    source: Source,
    /// Its dependencies: the versions each may take.
    deps: Vec<(String, Ranges<Ver>)>,
    /// The declaration that brought it in.
    entry: DependencyDetail,
    /// Known once it is fetched (a registry version: after the selection).
    kind: Option<PackageKind>,
    /// A registry version's index line.
    index: Option<IndexEntry>,
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

/// The versions of `name` a registry offers a resolution: those not
/// yanked, and a locked one that is (it keeps working, W4322) unless it may
/// move.
fn registry_candidates(
    index: &[IndexEntry],
    name: &str,
    key: &str,
    options: &Options,
) -> Vec<(semver::Version, IndexEntry)> {
    let locked: Vec<semver::Version> = if options.update.allows(name) {
        Vec::new()
    } else {
        options
            .lock
            .iter()
            .filter(|l| l.name == name && l.key() == key)
            .filter_map(|l| semver::Version::parse(&l.version).ok())
            .collect()
    };
    index
        .iter()
        .filter_map(|e| Some((e.version()?, e.clone())))
        .filter(|(v, e)| !e.yanked || locked.contains(v))
        .collect()
}

/// Resolves and fetches the dependencies of every root.
pub fn resolve(
    db: &mut SourceDb,
    roots: &[Root],
    fetcher: &Fetcher,
    options: &Options,
) -> Result<Resolution, Vec<Diagnostic>> {
    let mut errors: Vec<Diagnostic> = Vec::new();
    let mut warnings: Vec<Diagnostic> = Vec::new();
    // (identity, lock key) → the materialized package.
    let mut materialized: HashMap<(String, String), Materialized> = HashMap::new();
    // name → (identity, who asked first) for E4321.
    let mut identities: HashMap<String, (String, String, String)> = HashMap::new();
    let mut nodes: BTreeMap<String, BTreeMap<semver::Version, Node>> = BTreeMap::new();
    let mut root_deps: Vec<Vec<(String, Ranges<Ver>)>> = Vec::new();

    for root in roots {
        let mut direct: Vec<(String, Ranges<Ver>)> = Vec::new();
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
            let mut source = Source::of(&req.dep, &req.base, options.registries);
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
            let entry = match &req.dep {
                Dependency::Detailed(d) => (**d).clone(),
                Dependency::Version(_) => DependencyDetail::default(),
            };

            // A registry requirement: every version of the index it matches.
            if let Source::Registry {
                req: requirement,
                url,
            } = &source
            {
                let versions = match registry_versions(
                    &req.name,
                    requirement,
                    url,
                    &key,
                    &requester_name,
                    fetcher,
                    options,
                ) {
                    Ok(v) => v,
                    Err(d) => {
                        errors.push(d.with_path(&root.manifest_path));
                        continue;
                    }
                };
                let range = versions.iter().fold(Ranges::empty(), |r, (v, _)| {
                    r.union(&Ranges::singleton(Ver(v.clone())))
                });
                record(&mut nodes, &mut direct, &req, range);
                for (version, line) in versions {
                    let fresh = !nodes
                        .get(&req.name)
                        .is_some_and(|v| v.contains_key(&version));
                    if fresh {
                        nodes.entry(req.name.clone()).or_default().insert(
                            version.clone(),
                            Node {
                                source: source.clone(),
                                deps: Vec::new(),
                                entry: entry.clone(),
                                kind: entry.kind,
                                index: Some(line.clone()),
                            },
                        );
                    }
                    if visited.insert((req.name.clone(), version.to_string(), key.clone())) {
                        for d in &line.deps {
                            let detail = DependencyDetail {
                                version: Some(d.req.clone()),
                                registry: Some(d.registry.clone().unwrap_or_else(|| url.clone())),
                                optional: d.optional,
                                ..Default::default()
                            };
                            queue.push_back(Request {
                                name: d.name.clone(),
                                dep: Dependency::Detailed(Box::new(detail)),
                                base: req.base.clone(),
                                requester: Some((req.name.clone(), version.clone())),
                            });
                        }
                    }
                }
                continue;
            }

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
                errors.push(addon_error(&req.name, &root.manifest_path));
                continue;
            }
            record(
                &mut nodes,
                &mut direct,
                &req,
                Ranges::singleton(Ver(m.version.clone())),
            );
            let fresh = !nodes
                .get(&req.name)
                .is_some_and(|v| v.contains_key(&m.version));
            if fresh {
                nodes.entry(req.name.clone()).or_default().insert(
                    m.version.clone(),
                    Node {
                        source: source.clone(),
                        deps: Vec::new(),
                        entry,
                        kind: Some(kind),
                        index: None,
                    },
                );
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

    // One version per package per root (PubGrub over the graph), preferring
    // the root's locked versions.
    let mut used_by: BTreeMap<(String, semver::Version), BTreeSet<String>> = BTreeMap::new();
    for (root, direct) in roots.iter().zip(&root_deps) {
        let mut provider: OfflineDependencyProvider<String, Ranges<Ver>> =
            OfflineDependencyProvider::new();
        let root_key = format!("{} (root)", root.name);
        provider.add_dependencies(root_key.clone(), Ver(root.version.clone()), direct.clone());
        // A yanked version only for the roots that locked it.
        let locked_by_root = |name: &str, version: &semver::Version| {
            options.lock.iter().any(|l| {
                l.name == name
                    && l.version == version.to_string()
                    && (l.used_by.is_empty() || l.used_by.contains(&root.name))
            })
        };
        for (name, versions) in &nodes {
            for (version, node) in versions {
                if node.index.as_ref().is_some_and(|i| i.yanked) && !locked_by_root(name, version) {
                    continue;
                }
                provider.add_dependencies(name.clone(), Ver(version.clone()), node.deps.clone());
            }
        }
        let mut preferred: HashMap<String, Ver> = HashMap::new();
        for l in options.lock {
            if options.update.allows(&l.name)
                || !(l.used_by.is_empty() || l.used_by.contains(&root.name))
            {
                continue;
            }
            if let Ok(v) = semver::Version::parse(&l.version) {
                preferred.insert(l.name.clone(), Ver(v));
            }
        }
        if let Some((name, precise)) = options.precise
            && let Ok(v) = semver::Version::parse(precise)
        {
            preferred.insert(name.to_string(), Ver(v));
        }
        let provider = Preferring {
            inner: provider,
            preferred,
        };
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
                    for (version, node) in versions {
                        if node.index.is_none() {
                            d = d.with_note(format!(
                                "{name} {version} is {}",
                                node.source.describe()
                            ));
                        }
                    }
                }
                errors.push(d.with_help(
                    "make the declarations agree on one version, tag, branch or revision",
                ));
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
        let node = nodes[&name][&version].clone();
        let key = node.source.lock_key(options.lock_root);
        let m = match &node.index {
            Some(line) => {
                let Source::Registry { url, .. } = &node.source else {
                    unreachable!("index lines are registry versions")
                };
                match materialize_registry(db, &name, url, line, &key, fetcher, options) {
                    Ok(m) => m,
                    Err(ds) => {
                        errors.extend(ds);
                        continue;
                    }
                }
            }
            None => materialized[&(node.source.identity(), key)].clone(),
        };
        let kind = match node.kind.or_else(|| {
            m.manifest
                .as_ref()
                .and_then(|mf| mf.package.as_ref().map(|p| p.kind))
        }) {
            Some(PackageKind::Addon) | None => {
                let path = roots
                    .iter()
                    .find(|r| users.contains(&r.name))
                    .map(|r| r.manifest_path.clone())
                    .unwrap_or_default();
                errors.push(addon_error(&name, &path));
                continue;
            }
            Some(kind) => kind,
        };
        if let Some(line) = node.index.as_ref().filter(|l| l.yanked) {
            let mut w = Diagnostic::warning(
                "W4322",
                format!(
                    "`{name}` {} is locked, but it has been yanked from the registry",
                    line.vers
                ),
            )
            .with_help(format!(
                "run `wowlua update {name}` to move to a version that is not yanked"
            ));
            if let Some(root) = roots.iter().find(|r| users.contains(&r.name)) {
                w = w.with_path(&root.manifest_path);
            }
            warnings.push(w);
        }
        let mut dependencies: Vec<String> = node.deps.into_iter().map(|(n, _)| n).collect();
        dependencies.sort();
        dependencies.dedup();
        packages.push(ResolvedPackage {
            name,
            version,
            source: node.source,
            pin: m.pin.clone(),
            dir: m.dir.clone(),
            checksum: m.checksum.clone(),
            manifest: m.manifest.clone(),
            project_file: m.project_file.clone(),
            kind,
            entry: node.entry,
            dependencies,
            used_by: users.into_iter().collect(),
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Resolution { packages, warnings })
}

fn addon_error(name: &str, manifest: &Path) -> Diagnostic {
    Diagnostic::plain_error(format!(
        "dependency `{name}` is an addon; addons cannot be embedded"
    ))
    .with_path(manifest)
    .with_help("list it in `addon-dependencies` instead")
}

/// Adds a dependency edge: from the root, or from the package version that
/// declares it.
fn record(
    nodes: &mut BTreeMap<String, BTreeMap<semver::Version, Node>>,
    direct: &mut Vec<(String, Ranges<Ver>)>,
    req: &Request,
    range: Ranges<Ver>,
) {
    let edge = (req.name.clone(), range);
    match &req.requester {
        None => {
            if !direct.contains(&edge) {
                direct.push(edge);
            }
        }
        Some((pname, pversion)) => {
            if let Some(node) = nodes.get_mut(pname).and_then(|v| v.get_mut(pversion))
                && !node.deps.contains(&edge)
            {
                node.deps.push(edge);
            }
        }
    }
}

/// The versions of a registry package a requirement takes, with their index
/// lines (E4102 when there are none).
fn registry_versions(
    name: &str,
    requirement: &str,
    url: &str,
    key: &str,
    requester: &str,
    fetcher: &Fetcher,
    options: &Options,
) -> Result<Vec<(semver::Version, IndexEntry)>, Diagnostic> {
    let parsed = semver::VersionReq::parse(requirement).map_err(|e| {
        Diagnostic::error(
            "E4102",
            format!("dependency `{name}`: `{requirement}` (required by `{requester}`) is not a version requirement: {e}"),
        )
    })?;
    let locked: Vec<String> = options
        .lock
        .iter()
        .filter(|l| l.name == name && l.key() == key)
        .map(|l| l.version.clone())
        .collect();
    // A cached index serves when it has the locked version; anything new
    // reads the registry.
    let refresh = options.refresh || options.update.allows(name) || locked.is_empty();
    let read = |refresh: bool| {
        fetcher
            .registry_index(url, name, refresh)
            .map_err(|e| diag(e, name))
    };
    let mut index = read(refresh)?;
    let has_locked = |index: &Option<Vec<IndexEntry>>| {
        index.as_ref().is_some_and(|i| {
            i.iter().any(|e| {
                locked.contains(&e.vers) && e.version().is_some_and(|v| parsed.matches(&v))
            })
        })
    };
    if !refresh && !has_locked(&index) {
        index = read(true)?;
    }
    let Some(index) = index else {
        return Err(Diagnostic::error(
            "E4102",
            format!(
                "dependency `{name}` (required by `{requester}`) is not in the registry `{url}`"
            ),
        )
        .with_help("check the name; a library the registry does not have can come from a `git`, `svn`, `path` or `archive` source"));
    };
    let mut candidates = registry_candidates(&index, name, key, options);
    if let Some((_, precise)) = options.precise.filter(|(n, _)| *n == name) {
        let Ok(v) = semver::Version::parse(precise) else {
            return Err(Diagnostic::plain_error(format!(
                "`--precise {precise}` needs a version of the registry package `{name}`"
            )));
        };
        if !candidates.iter().any(|(c, _)| *c == v) {
            return Err(Diagnostic::error(
                "E4102",
                format!("the registry has no version {v} of `{name}` that is not yanked"),
            ));
        }
    }
    let matching: Vec<(semver::Version, IndexEntry)> = candidates
        .drain(..)
        .filter(|(v, _)| parsed.matches(v))
        .collect();
    if matching.is_empty() {
        let mut d = Diagnostic::error(
            "E4102",
            format!(
                "no version of `{name}` in the registry matches `{requirement}` (required by `{requester}`)"
            ),
        );
        let mut available: Vec<semver::Version> = index
            .iter()
            .filter(|e| !e.yanked)
            .filter_map(IndexEntry::version)
            .collect();
        available.sort();
        if !available.is_empty() {
            let list: Vec<String> = available
                .iter()
                .rev()
                .take(5)
                .map(|v| v.to_string())
                .collect();
            d = d.with_note(format!("the newest versions are {}", list.join(", ")));
        }
        let yanked: Vec<&str> = index
            .iter()
            .filter(|e| e.yanked && e.version().is_some_and(|v| parsed.matches(&v)))
            .map(|e| e.vers.as_str())
            .collect();
        if !yanked.is_empty() {
            d = d.with_note(format!(
                "{} would match, but yanked versions are never chosen for a new resolution",
                yanked.join(", ")
            ));
        }
        return Err(d.with_help("change the requirement"));
    }
    Ok(matching)
}

/// Downloads (or finds in the cache) a selected registry version and checks
/// it: its checksum against the index and the lockfile (E4103), and its
/// manifest's name and version.
fn materialize_registry(
    db: &mut SourceDb,
    name: &str,
    url: &str,
    line: &IndexEntry,
    key: &str,
    fetcher: &Fetcher,
    options: &Options,
) -> Result<Materialized, Vec<Diagnostic>> {
    let dir = fetcher
        .registry_package(url, line)
        .map_err(|e| vec![diag(e, name)])?;
    let sum = checksum::tree(&dir).map_err(|e| {
        vec![Diagnostic::plain_error(format!(
            "dependency `{name}`: cannot read `{}`: {e}",
            dir.display()
        ))]
    })?;
    let locked = options
        .lock
        .iter()
        .find(|l| l.name == name && l.key() == key && l.version == line.vers)
        .and_then(|l| l.checksum.as_deref());
    for (what, expected) in [
        ("the registry's index", Some(line.cksum.as_str())),
        ("the lockfile", locked),
    ] {
        if let Some(expected) = expected
            && expected != sum
        {
            return Err(vec![Diagnostic::error(
                "E4103",
                format!("checksum mismatch for dependency `{name}` {}: {what} has {expected}, the fetched files have {sum}", line.vers),
            )
            .with_note(format!("cached at {}", dir.display()))
            .with_help("run `wowlua cache clean --all` and fetch again; a version of the registry never changes")]);
        }
    }
    let manifest_path = dir.join("wowlua.json");
    let mut output = wowlua_manifest::load(db, &manifest_path, options.compiler);
    if let Some(m) = &mut output.manifest {
        m.root.clone_from(&dir);
    }
    let errors: Vec<Diagnostic> = output
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .cloned()
        .collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    let manifest = output.manifest.map(|m| m.manifest);
    let package = manifest.as_ref().and_then(|m| m.package.as_ref());
    if package.is_none_or(|p| p.name != name || p.version != line.vers) {
        return Err(vec![Diagnostic::error(
            "E4102",
            format!(
                "dependency `{name}` {}: the registry's archive holds another package",
                line.vers
            ),
        )]);
    }
    Ok(Materialized {
        version: line.version().unwrap_or(semver::Version::new(0, 0, 0)),
        pin: None,
        dir,
        checksum: Some(sum),
        manifest,
        project_file: None,
    })
}

/// PubGrub's offline provider, choosing a root's locked version when its
/// range allows it (spec §10.4) and the highest otherwise.
struct Preferring {
    inner: OfflineDependencyProvider<String, Ranges<Ver>>,
    preferred: HashMap<String, Ver>,
}

impl DependencyProvider for Preferring {
    type P = String;
    type V = Ver;
    type VS = Ranges<Ver>;
    type M = String;
    type Err = std::convert::Infallible;
    type Priority =
        <OfflineDependencyProvider<String, Ranges<Ver>> as DependencyProvider>::Priority;

    fn prioritize(
        &self,
        package: &String,
        range: &Ranges<Ver>,
        stats: &pubgrub::PackageResolutionStatistics,
    ) -> Self::Priority {
        self.inner.prioritize(package, range, stats)
    }

    fn choose_version(
        &self,
        package: &String,
        range: &Ranges<Ver>,
    ) -> Result<Option<Ver>, Self::Err> {
        if let Some(v) = self.preferred.get(package)
            && range.contains(v)
            && self
                .inner
                .versions(package)
                .is_some_and(|mut vs| vs.any(|x| x == v))
        {
            return Ok(Some(v.clone()));
        }
        self.inner.choose_version(package, range)
    }

    fn get_dependencies(
        &self,
        package: &String,
        version: &Ver,
    ) -> Result<pubgrub::Dependencies<String, Ranges<Ver>, String>, Self::Err> {
        self.inner.get_dependencies(package, version)
    }
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
    // Through the host: a path dependency of a project in memory (the
    // browser's) is in memory too.
    let own_pkgmeta = |root: &Path, sub: &str| {
        let file = root.join(".pkgmeta");
        wowlua_host::is_file(&file).then(|| (file, sub.to_string()))
    };
    let mut project_file = None;
    let (dir, pin) = match source {
        Source::Path { path, dir } => {
            if !wowlua_host::exists(dir.join("wowlua.json")) && !wowlua_host::is_dir(dir) {
                return Err(vec![Diagnostic::error(
                    "E4102",
                    format!("dependency `{name}`: path `{path}` does not exist"),
                )]);
            }
            project_file = own_pkgmeta(dir, "");
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
            project_file = own_pkgmeta(&checkout, subdir.as_deref().unwrap_or(""));
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
            // A checkout of a folder of the project leaves its root's
            // `.pkgmeta` behind; the packager fetches that one.
            project_file =
                own_pkgmeta(&dir, "").or_else(|| fetcher.svn_root_pkgmeta(url, revision));
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
            // Without `strip-prefix`, a lone top folder (a GitHub archive's)
            // is the root, like the packager uses it.
            let dir = match strip_prefix {
                Some(p) => extracted.join(p),
                None => {
                    let entries: Vec<PathBuf> = std::fs::read_dir(&extracted)
                        .map(|d| d.flatten().map(|e| e.path()).collect())
                        .unwrap_or_default();
                    match entries.as_slice() {
                        [only] if only.is_dir() => only.clone(),
                        _ => extracted,
                    }
                }
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
        Source::Registry { .. } => {
            unreachable!("registry versions are materialized after the selection")
        }
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
    let manifest = if wowlua_host::is_file(&manifest_path) {
        let output = match wowlua_host::read_to_string(&manifest_path) {
            Ok(text) => {
                let mut output = wowlua_manifest::parse(db, &manifest_path, text, options.compiler);
                if let Some(m) = &mut output.manifest {
                    m.path.clone_from(&manifest_path);
                    m.root.clone_from(&dir);
                }
                output
            }
            Err(_) => wowlua_manifest::load(db, &manifest_path, options.compiler),
        };
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
        project_file,
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
