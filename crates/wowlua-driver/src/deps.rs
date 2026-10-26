//! Dependency commands (spec CLI §12.3): `add` and `remove` edit
//! `wowlua.json` in place and bring the lockfile up to date; `tree` and
//! `outdated` report on the resolved graph.

use crate::Session;
use crate::project::{LoadOptions, Project, load_project};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::model::Dependency;
use wowlua_pkg::{GitRef, Source};

/// `wowlua add <name> …`.
#[derive(Clone, Debug, Default)]
pub struct AddRequest {
    pub name: String,
    /// `name@<req>`: a registry requirement.
    pub requirement: Option<String>,
    pub git: Option<String>,
    pub tag: Option<String>,
    pub branch: Option<String>,
    pub rev: Option<String>,
    pub svn: Option<String>,
    pub path: Option<String>,
    pub archive: Option<String>,
    pub sha256: Option<String>,
    pub strip_prefix: Option<String>,
    pub subdir: Option<String>,
    pub kind: Option<String>,
    pub files: Vec<String>,
    pub install_path: Option<String>,
    pub dev: bool,
    pub optional: bool,
    pub flavors: Vec<String>,
    pub features: Vec<String>,
    /// `default-features: false`.
    pub no_default_features: bool,
}

impl AddRequest {
    /// The dependency entry as JSON.
    fn entry(&self) -> Value {
        let mut m = Map::new();
        let mut put = |k: &str, v: &Option<String>| {
            if let Some(v) = v {
                m.insert(k.into(), json!(v));
            }
        };
        put("git", &self.git);
        put("tag", &self.tag);
        put("branch", &self.branch);
        put("svn", &self.svn);
        put("rev", &self.rev);
        put("subdir", &self.subdir);
        put("path", &self.path);
        put("archive", &self.archive);
        put("sha256", &self.sha256);
        put("strip-prefix", &self.strip_prefix);
        put("kind", &self.kind);
        put("install-path", &self.install_path);
        if !self.files.is_empty() {
            m.insert("files".into(), json!(self.files));
        }
        if self.optional {
            m.insert("optional".into(), json!(true));
        }
        if !self.flavors.is_empty() {
            m.insert("flavors".into(), json!(self.flavors));
        }
        if !self.features.is_empty() {
            m.insert("features".into(), json!(self.features));
        }
        if self.no_default_features {
            m.insert("default-features".into(), json!(false));
        }
        if m.is_empty() {
            return json!(self.requirement.clone().unwrap_or_else(|| "*".into()));
        }
        if let Some(req) = &self.requirement {
            m.insert("version".into(), json!(req));
        }
        Value::Object(m)
    }
}

/// The manifest the command edits: the package, or the selected workspace
/// member.
fn manifest_path(session: &mut Session, options: &LoadOptions) -> Option<PathBuf> {
    let project = load_project(
        session,
        &LoadOptions {
            skip_dependencies: true,
            ..options.clone()
        },
    )?;
    Some(project.manifest.path.clone())
}

/// Applies `edit` to the manifest text, then loads the project (resolving
/// and fetching). On failure the manifest is restored.
fn edit_and_load(
    session: &mut Session,
    options: &LoadOptions,
    edit: impl FnOnce(&str) -> Result<Option<String>, Diagnostic>,
) -> Option<Project> {
    let path = manifest_path(session, options)?;
    let original = match crate::host::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            session.push(
                Diagnostic::plain_error(format!("cannot read the manifest: {e}")).with_path(&path),
            );
            return None;
        }
    };
    let edited = match edit(&original) {
        Ok(Some(t)) => t,
        Ok(None) => return load_project(session, options),
        Err(d) => {
            session.push(d.with_path(&path));
            return None;
        }
    };
    if let Err(e) = crate::host::write(&path, &edited) {
        session.push(
            Diagnostic::plain_error(format!("cannot write the manifest: {e}")).with_path(&path),
        );
        return None;
    }
    let project = load_project(session, options);
    if project.is_none() {
        let _ = crate::host::write(&path, &original);
        session.push(Diagnostic::note("wowlua.json was left unchanged"));
    }
    project
}

fn edit_error(e: String) -> Diagnostic {
    Diagnostic::plain_error(format!("cannot edit the manifest: {e}"))
}

/// `wowlua add`: adds or replaces a dependency entry.
pub fn add(session: &mut Session, options: &LoadOptions, request: &AddRequest) -> Option<Project> {
    let section = if request.dev {
        "dev-dependencies"
    } else {
        "dependencies"
    };
    let entry = request.entry();
    edit_and_load(session, options, |text| {
        wowlua_manifest::edit::set(text, &[section], &request.name, &entry)
            .map(Some)
            .map_err(edit_error)
    })
}

/// `wowlua add --addon <Name> [--optional]`.
pub fn add_addon(
    session: &mut Session,
    options: &LoadOptions,
    name: &str,
    optional: bool,
) -> Option<Project> {
    let list = if optional { "optional" } else { "required" };
    edit_and_load(session, options, |text| {
        let current: Value = serde_json::from_str(text).map_err(|e| edit_error(e.to_string()))?;
        let present = current["addon-dependencies"][list]
            .as_array()
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|i| i.as_str() == Some(name) || i["name"].as_str() == Some(name))
            });
        if present {
            return Ok(None);
        }
        wowlua_manifest::edit::push(text, &["addon-dependencies", list], &json!(name))
            .map(Some)
            .map_err(edit_error)
    })
}

/// `wowlua remove`.
pub fn remove(
    session: &mut Session,
    options: &LoadOptions,
    name: &str,
    dev: bool,
) -> Option<Project> {
    let section = if dev {
        "dev-dependencies"
    } else {
        "dependencies"
    };
    edit_and_load(
        session,
        options,
        |text| match wowlua_manifest::edit::remove(text, &[section], name).map_err(edit_error)? {
            Some(t) => Ok(Some(t)),
            None => Err(Diagnostic::plain_error(format!(
                "`{name}` is not in `{section}`"
            ))),
        },
    )
}

/// `wowlua tree`: the dependency graph, as text.
pub fn tree(project: &Project, flavor: Option<&str>, invert: Option<&str>) -> String {
    let manifest = &project.manifest.manifest;
    let root = project
        .manifest
        .package()
        .map(|p| format!("{} {}", p.name, p.version))
        .unwrap_or_default();
    let packages: BTreeMap<&str, &wowlua_pkg::ResolvedPackage> = project
        .deps
        .packages
        .iter()
        .map(|p| (p.name.as_str(), p))
        .collect();
    let applies = |dep: &Dependency| -> bool {
        let Some(f) = flavor else { return true };
        match dep {
            Dependency::Detailed(d) if !d.flavors.is_empty() => d.flavors.iter().any(|x| {
                wowlua_flavors::table()
                    .resolve(x)
                    .is_some_and(|fl| fl.id == f)
            }),
            _ => true,
        }
    };
    let label = |name: &str| -> String {
        match packages.get(name) {
            Some(p) => format!("{name} {} ({})", p.version, p.source.describe()),
            None => name.to_string(),
        }
    };
    let mut out = String::new();
    if let Some(target) = invert {
        // Every path from a direct dependency down to `target`, upside down.
        out.push_str(&format!("{}\n", label(target)));
        fn users(
            name: &str,
            packages: &BTreeMap<&str, &wowlua_pkg::ResolvedPackage>,
            direct: &BTreeSet<String>,
            root: &str,
            depth: usize,
            out: &mut String,
        ) {
            if depth > 32 {
                return;
            }
            for (other, p) in packages {
                if p.dependencies.iter().any(|d| d == name) {
                    out.push_str(&format!(
                        "{}└── {other} {}\n",
                        "    ".repeat(depth),
                        p.version
                    ));
                    users(other, packages, direct, root, depth + 1, out);
                }
            }
            if direct.contains(name) {
                out.push_str(&format!("{}└── {root}\n", "    ".repeat(depth)));
            }
        }
        let direct: BTreeSet<String> = manifest
            .dependencies
            .iter()
            .chain(&manifest.dev_dependencies)
            .filter(|(_, d)| applies(d))
            .map(|(n, _)| n.clone())
            .collect();
        users(target, &packages, &direct, &root, 0, &mut out);
        return out;
    }
    out.push_str(&format!("{root}\n"));
    #[allow(clippy::too_many_arguments)]
    fn walk(
        name: &str,
        prefix: &str,
        last: bool,
        suffix: &str,
        packages: &BTreeMap<&str, &wowlua_pkg::ResolvedPackage>,
        label: &dyn Fn(&str) -> String,
        seen: &mut Vec<String>,
        out: &mut String,
    ) {
        let branch = if last { "└── " } else { "├── " };
        let repeated = seen.iter().any(|s| s == name);
        out.push_str(&format!(
            "{prefix}{branch}{}{suffix}{}\n",
            label(name),
            if repeated { " (*)" } else { "" }
        ));
        if repeated {
            return;
        }
        seen.push(name.to_string());
        let children = packages
            .get(name)
            .map(|p| p.dependencies.clone())
            .unwrap_or_default();
        let next = format!("{prefix}{}", if last { "    " } else { "│   " });
        for (i, c) in children.iter().enumerate() {
            walk(
                c,
                &next,
                i + 1 == children.len(),
                "",
                packages,
                label,
                seen,
                out,
            );
        }
    }
    let direct: Vec<(String, &str)> = manifest
        .dependencies
        .iter()
        .filter(|(_, d)| applies(d))
        .map(|(n, d)| (n.clone(), if d.is_optional() { " [optional]" } else { "" }))
        .chain(
            manifest
                .dev_dependencies
                .iter()
                .filter(|(_, d)| applies(d))
                .map(|(n, _)| (n.clone(), " [dev]")),
        )
        .collect();
    let mut seen = Vec::new();
    for (i, (name, suffix)) in direct.iter().enumerate() {
        walk(
            name,
            "",
            i + 1 == direct.len(),
            suffix,
            &packages,
            &label,
            &mut seen,
            &mut out,
        );
    }
    out
}

/// One row of `wowlua outdated`.
#[derive(Clone, Debug)]
pub struct Outdated {
    pub name: String,
    /// The locked tag, branch commit or revision.
    pub current: String,
    pub latest: String,
    /// What would bring it up to date.
    pub action: String,
}

fn short(commit: &str) -> String {
    commit.chars().take(10).collect()
}

/// Orders tags by SemVer when they parse (a leading `v` allowed), else
/// lexically after the SemVer ones.
fn tag_key(tag: &str) -> (Option<semver::Version>, String) {
    let t = tag.strip_prefix('v').unwrap_or(tag);
    let padded = match t.matches('.').count() {
        0 => format!("{t}.0.0"),
        1 => format!("{t}.0"),
        _ => t.to_string(),
    };
    (semver::Version::parse(&padded).ok(), tag.to_string())
}

/// `wowlua outdated`: dependencies with newer tags, commits or revisions.
pub fn outdated(
    session: &mut Session,
    project: &Project,
    offline: bool,
    status: Option<crate::project::Status>,
) -> Vec<Outdated> {
    let Some(cache) = wowlua_pkg::Cache::new() else {
        return Vec::new();
    };
    let fetcher = wowlua_pkg::Fetcher::new(cache, offline, move |w, m| {
        if let Some(s) = &status {
            (s.0)(w, m);
        }
    });
    let mut rows = Vec::new();
    for p in &project.deps.packages {
        let pin = p.pin.clone().unwrap_or_default();
        let row = match &p.source {
            Source::Git { url, reference, .. } => match reference {
                GitRef::Tag(tag) => match fetcher.git_tags(url) {
                    Ok(tags) => {
                        let current = tag_key(tag);
                        tags.iter()
                            .map(|t| tag_key(t))
                            .filter(|k| k.0.is_some() && current.0.is_some() && k.0 > current.0)
                            .max()
                            .map(|newest| Outdated {
                                name: p.name.clone(),
                                current: tag.clone(),
                                latest: newest.1.clone(),
                                action: format!("change `tag` to \"{}\"", newest.1),
                            })
                    }
                    Err(e) => {
                        session.push(Diagnostic::plain_warning(format!(
                            "{}: {}",
                            p.name, e.message
                        )));
                        None
                    }
                },
                GitRef::Rev(_) => None,
                GitRef::Branch(_) | GitRef::Head => {
                    match fetcher.git_commit(url, reference, None) {
                        Ok(latest) if latest != pin => Some(Outdated {
                            name: p.name.clone(),
                            current: short(&pin),
                            latest: short(&latest),
                            action: "`wowlua update`".into(),
                        }),
                        Ok(_) => None,
                        Err(e) => {
                            session.push(Diagnostic::plain_warning(format!(
                                "{}: {}",
                                p.name, e.message
                            )));
                            None
                        }
                    }
                }
            },
            Source::Svn { url, rev, .. } => match fetcher.svn_latest(url) {
                Ok(latest) if latest.to_string() != pin => Some(Outdated {
                    name: p.name.clone(),
                    current: format!("r{pin}"),
                    latest: format!("r{latest}"),
                    action: if rev.is_some() {
                        format!("change `rev` to \"{latest}\"")
                    } else {
                        "`wowlua update`".into()
                    },
                }),
                Ok(_) => None,
                Err(e) => {
                    session.push(Diagnostic::plain_warning(format!(
                        "{}: {}",
                        p.name, e.message
                    )));
                    None
                }
            },
            // Archives and paths have no notion of newer versions.
            _ => None,
        };
        rows.extend(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_order_by_semver() {
        let mut tags = vec!["v1.10", "v1.2", "1.9.1", "beta"];
        tags.sort_by_key(|t| tag_key(t));
        assert_eq!(tags, vec!["beta", "v1.2", "1.9.1", "v1.10"]);
    }
}
