//! The wowlua registry as a publish target (Publishing §11.8): the package
//! archive of `wowlua package --registry` (Package manager §10.10.3) and
//! its upload, `PUT {api}/v1/packages/<name>/<version>`.
//!
//! Only `library`, `lua-library` and `types` packages go to the registry
//! (E4230), and only with dependencies the registry can resolve (E4231). A
//! library package that declares no registry target has an implicit one
//! named `registry` (decision T104's consequences).

// Only a computer has this (spec M14, decision T394).
#![allow(clippy::disallowed_methods)]

use super::http::{Http, Multipart};
use super::sites::Outcome;
use crate::{Project, Session};
use std::collections::BTreeMap;
use std::path::PathBuf;
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::model::{Dependency, PackageKind, Target, TargetType};

/// The name of a library's implicit registry target.
pub const IMPLICIT_TARGET: &str = "registry";

/// A package archive for the registry.
#[derive(Clone, Debug)]
pub struct Archive {
    pub name: String,
    pub version: String,
    /// `dist/packages/<name>-<version>.tar.gz`.
    pub path: PathBuf,
    pub file_name: String,
    pub bytes: Vec<u8>,
    /// The normalized `wowlua.json`, as in the archive.
    pub manifest: String,
    /// The checksum of its files (the registry's `cksum`).
    pub checksum: String,
    /// The files in it, sorted.
    pub files: Vec<String>,
}

/// The `wowlua.json` the registry gets: the package's, without what only
/// its own repository uses (`dev-dependencies`, `publish`, `workspace`),
/// and with a dependency's registry name (from `config.json`) replaced by
/// its URL.
pub fn normalized_manifest(
    text: &str,
    registries: &wowlua_pkg::source::Registries,
) -> Result<String, String> {
    let mut value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if let Some(map) = value.as_object_mut() {
        for key in ["dev-dependencies", "publish", "workspace"] {
            map.remove(key);
        }
        for dep in map
            .get_mut("dependencies")
            .and_then(|d| d.as_object_mut())
            .into_iter()
            .flat_map(|d| d.values_mut())
        {
            if let Some(registry) = dep.get_mut("registry")
                && let Some(name) = registry.as_str()
            {
                *registry = serde_json::Value::String(registries.url(Some(name)));
            }
        }
    }
    let mut out = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    out.push('\n');
    Ok(out)
}

/// E4230 for an addon.
pub fn addon_error(project: &Project) -> Diagnostic {
    Diagnostic::error(
        "E4230",
        format!(
            "`{}` is an addon; addons cannot be published to the wowlua registry",
            project.manifest.package().map_or("", |p| p.name.as_str())
        ),
    )
    .with_path(&project.manifest.path)
    .with_help("the registry is for `library`, `lua-library` and `types` packages; publish an addon to CurseForge, Wago, WoWInterface or GitHub")
}

/// Whether a dependency comes from a registry: a requirement, or an entry
/// with `version` / `registry` and no other source.
fn from_registry(dep: &Dependency) -> bool {
    match dep {
        Dependency::Version(_) => true,
        Dependency::Detailed(d) => {
            d.git.is_none() && d.svn.is_none() && d.path.is_none() && d.archive.is_none()
        }
    }
}

/// The time archive entries carry: `SOURCE_DATE_EPOCH`, else the commit
/// time of `HEAD`, else 0.
fn archive_time(dir: &std::path::Path) -> u64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .or_else(|| {
            let out = std::process::Command::new("git")
                .args(["log", "-1", "--format=%ct"])
                .current_dir(dir)
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()?;
            String::from_utf8_lossy(&out.stdout).trim().parse().ok()
        })
        .unwrap_or(0)
}

/// Makes the registry archive of a package (§10.10.3) and writes it to
/// `dist/packages/`: `wowlua.json` (normalized), the sources, declaration
/// and license files: every file of the package except its build output,
/// its tests (`tests/`, `*.test.wowlua`), dev files (names starting with
/// `.`, `wowlua.lock`) and what `build.exclude` excludes.
pub fn archive(session: &mut Session, project: &Project) -> Option<Archive> {
    let manifest = &project.manifest;
    let Some(package) = manifest.package() else {
        session.push(
            Diagnostic::plain_error("a workspace root without a package has no registry archive")
                .with_path(&manifest.path),
        );
        return None;
    };
    if package.kind == PackageKind::Addon {
        session.push(addon_error(project));
        return None;
    }
    let mut ok = true;
    for (name, dep) in &manifest.manifest.dependencies {
        if !from_registry(dep) {
            ok = false;
            session.push(
                Diagnostic::error(
                    "E4231",
                    format!("dependency `{name}` does not come from a registry, so a package on the registry cannot depend on it"),
                )
                .with_path(&manifest.path)
                .with_help(format!("depend on `{name}` by version (`\"{name}\": \"^1.0\"`) once it is on the registry, or embed its files in the package")),
            );
        }
    }
    if !ok {
        return None;
    }
    let text = match std::fs::read_to_string(&manifest.path) {
        Ok(t) => t,
        Err(e) => {
            session.push(
                Diagnostic::plain_error(format!("cannot read the manifest: {e}"))
                    .with_path(&manifest.path),
            );
            return None;
        }
    };
    let normalized = match normalized_manifest(&text, &crate::project::registries()) {
        Ok(n) => n,
        Err(e) => {
            session.push(
                Diagnostic::plain_error(format!("cannot read the manifest: {e}"))
                    .with_path(&manifest.path),
            );
            return None;
        }
    };
    let root = &manifest.root;
    let out = crate::package::out_root(project);
    let exclude = crate::sources::glob_set(&manifest.manifest.build.exclude);
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    files.insert(
        wowlua_manifest::MANIFEST_FILE.into(),
        normalized.clone().into_bytes(),
    );
    let enter = |dir: &std::path::Path| {
        let rel = dir.strip_prefix(root).unwrap_or(dir);
        dir != out && rel != std::path::Path::new("tests")
    };
    for path in crate::host::walk(root, false, &enter) {
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let lower = rel.to_ascii_lowercase();
        if rel == wowlua_manifest::MANIFEST_FILE
            || rel == "wowlua.lock"
            || lower.ends_with(".test.wowlua")
            || exclude.is_match(&rel)
        {
            continue;
        }
        match std::fs::read(&path) {
            Ok(bytes) => {
                files.insert(rel, bytes);
            }
            Err(e) => {
                session.push(
                    Diagnostic::plain_error(format!("cannot read `{rel}`: {e}")).with_path(&path),
                );
                return None;
            }
        }
    }
    if files.len() > wowlua_pkg::registry::MAX_FILES {
        session.push(
            Diagnostic::plain_error(format!(
                "the package has {} files; a registry archive holds at most {}",
                files.len(),
                wowlua_pkg::registry::MAX_FILES
            ))
            .with_path(&manifest.path)
            .with_help("leave files out with `build.exclude`"),
        );
        return None;
    }
    let bytes = match wowlua_pkg::registry::write_archive(&files, archive_time(root)) {
        Ok(b) => b,
        Err(e) => {
            session.push(Diagnostic::plain_error(format!(
                "cannot write the archive: {e}"
            )));
            return None;
        }
    };
    let file_name = format!("{}-{}.tar.gz", package.name, package.version);
    let path = out.join("packages").join(&file_name);
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, &bytes));
    if let Err(e) = written {
        session.push(
            Diagnostic::plain_error(format!("cannot write `{}`: {e}", path.display()))
                .with_path(&manifest.path),
        );
        return None;
    }
    Some(Archive {
        name: package.name.clone(),
        version: package.version.clone(),
        path,
        file_name,
        checksum: wowlua_pkg::registry::checksum(&files),
        files: files.keys().cloned().collect(),
        bytes,
        manifest: normalized,
    })
}

/// A registry upload of one package.
#[derive(Clone, Debug)]
pub struct Job {
    /// Index into the projects.
    pub project: usize,
    pub target_name: String,
    pub target: Target,
}

fn implicit_target() -> Target {
    serde_json::from_value(serde_json::json!({ "type": "registry" }))
        .expect("a registry target needs only its type")
}

/// The registry targets of the projects that `--target` selects (all
/// enabled ones without it), with a library's implicit `registry` target;
/// E4230 for an addon with one.
pub fn plan(session: &mut Session, projects: &[Project], requested: &[String]) -> Vec<Job> {
    let mut jobs = Vec::new();
    for (i, project) in projects.iter().enumerate() {
        if !project.selected {
            continue;
        }
        let declared: Vec<(String, Target)> = project
            .manifest
            .manifest
            .publish
            .iter()
            .flat_map(|p| p.targets.iter())
            .filter(|(_, t)| t.target_type == TargetType::Registry)
            .map(|(n, t)| (n.clone(), t.clone()))
            .collect();
        let addon = project.kind() == PackageKind::Addon;
        let mut targets: Vec<(String, Target)> = declared
            .into_iter()
            .filter(|(n, t)| {
                if requested.is_empty() {
                    t.enabled
                } else {
                    requested.contains(n)
                }
            })
            .collect();
        let declares_any = project
            .manifest
            .manifest
            .publish
            .iter()
            .flat_map(|p| p.targets.values())
            .any(|t| t.target_type == TargetType::Registry);
        let names_implicit = requested.iter().any(|r| r == IMPLICIT_TARGET)
            && !project
                .manifest
                .manifest
                .publish
                .iter()
                .any(|p| p.targets.contains_key(IMPLICIT_TARGET));
        if !declares_any && (names_implicit || (requested.is_empty() && !addon)) {
            targets.push((IMPLICIT_TARGET.into(), implicit_target()));
        }
        if targets.is_empty() {
            continue;
        }
        if addon {
            session.push(addon_error(project));
            continue;
        }
        for (name, target) in targets {
            jobs.push(Job {
                project: i,
                target_name: name,
                target,
            });
        }
    }
    jobs
}

/// The registry a target publishes to.
pub fn registry_url(target: &Target) -> String {
    match &target.registry {
        Some(r) => crate::project::registries().url(Some(r)),
        None => crate::project::registries().default,
    }
}

/// `{"errors": [{"code", "detail"}]}` of an error response.
fn error_text(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let errors: Vec<String> = value["errors"]
        .as_array()?
        .iter()
        .map(|e| match (e["code"].as_str(), e["detail"].as_str()) {
            (Some(c), Some(d)) => format!("{d} ({c})"),
            (None, Some(d)) => d.to_string(),
            (Some(c), None) => c.to_string(),
            (None, None) => String::new(),
        })
        .filter(|e| !e.is_empty())
        .collect();
    (!errors.is_empty()).then(|| errors.join("; "))
}

/// Uploads an archive: `GET {registry}/config.json`, then the multipart
/// `PUT {api}/v1/packages/<name>/<version>`.
pub fn upload(
    http: &Http,
    target_name: &str,
    target: &Target,
    token: &str,
    archive: &Archive,
    dry_run: bool,
) -> Outcome {
    let registry = registry_url(target);
    let failed = |message: String| Outcome::failed(target_name, &archive.file_name, &message);
    if !wowlua_pkg::registry::allowed_url(&registry) {
        return failed(format!(
            "the registry `{registry}` must be reached over HTTPS"
        ));
    }
    let api = match &target.api_url {
        Some(api) => wowlua_pkg::registry::normalize_url(api),
        None if dry_run => registry.clone(),
        None => match http.retried("GET", &format!("{registry}/config.json"), &[], None) {
            Ok(r) if r.ok() => match r.json().and_then(|j| j["api"].as_str().map(str::to_string)) {
                Some(api) => wowlua_pkg::registry::normalize_url(&api),
                None => {
                    return failed(format!(
                        "`{registry}/config.json` is not a registry configuration"
                    ));
                }
            },
            Ok(r) => {
                return failed(format!(
                    "cannot read `{registry}/config.json` (HTTP {})",
                    r.status
                ));
            }
            Err(e) => return failed(format!("cannot reach the registry `{registry}`: {e}")),
        },
    };
    let url = format!("{api}/v1/packages/{}/{}", archive.name, archive.version);
    if dry_run {
        let mut o = Outcome::skipped(
            target_name,
            &archive.file_name,
            &format!(
                "PUT {url}: {} {} ({} files, {} bytes, {})",
                archive.name,
                archive.version,
                archive.files.len(),
                archive.bytes.len(),
                archive.checksum
            ),
        );
        o.skipped = false;
        return o;
    }
    let (content_type, body) = Multipart::default()
        .text("manifest", &archive.manifest)
        .file(
            "archive",
            &archive.file_name,
            "application/gzip",
            &archive.bytes,
        )
        .finish();
    let headers = [
        ("authorization", format!("Bearer {token}")),
        ("accept", "application/json".to_string()),
        ("content-type", content_type),
    ];
    match http.retried("PUT", &url, &headers, Some(&body)) {
        Ok(r) if r.status == 201 || r.ok() => {
            let cksum = r
                .json()
                .and_then(|j| j["cksum"].as_str().map(str::to_string))
                .unwrap_or_else(|| archive.checksum.clone());
            let mut o = Outcome::skipped(
                target_name,
                &archive.file_name,
                &format!("{} {} ({cksum})", archive.name, archive.version),
            );
            o.skipped = false;
            o
        }
        Ok(r) => failed(format!(
            "the registry refused {} {} (HTTP {}): {}",
            archive.name,
            archive.version,
            r.status,
            error_text(&r.body).unwrap_or_else(|| r.body.trim().to_string())
        )),
        Err(e) => failed(format!("cannot reach the registry `{registry}`: {e}")),
    }
}

// ---- yank, owners and search (CLI §12.4) --------------------------------------

/// The registry a command names with `--registry` (a URL or a name in
/// `config.json`), else the default.
pub fn chosen_registry(declared: Option<&str>) -> String {
    crate::project::registries().url(declared)
}

/// The registry's API base, from its `config.json`.
pub fn api_base(http: &Http, registry: &str) -> Result<String, Diagnostic> {
    if !wowlua_pkg::registry::allowed_url(registry) {
        return Err(Diagnostic::error(
            "E4102",
            if registry.contains("://") {
                format!("the registry `{registry}` must be reached over HTTPS")
            } else {
                format!("unknown registry `{registry}`")
            },
        ));
    }
    match http.retried("GET", &format!("{registry}/config.json"), &[], None) {
        Ok(r) if r.ok() => r
            .json()
            .and_then(|j| j["api"].as_str().map(wowlua_pkg::registry::normalize_url))
            .ok_or_else(|| {
                Diagnostic::plain_error(format!(
                    "`{registry}/config.json` is not a registry configuration"
                ))
            }),
        Ok(r) => Err(refused(registry, &r)),
        Err(e) => Err(Diagnostic::plain_error(format!(
            "cannot reach the registry `{registry}`: {e}"
        ))),
    }
}

/// The registry token: `WOWLUA_REGISTRY_TOKEN`, else what `wowlua login
/// registry` saved; E4201 without one.
pub fn token(registry: &str, status: Option<&crate::Status>) -> Result<String, Diagnostic> {
    if let Some(t) = std::env::var("WOWLUA_REGISTRY_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
    {
        return Ok(t.trim().to_string());
    }
    if let Some(s) = status
        && crate::credentials::reads_store()
    {
        (s.0)(
            "Reading",
            "the registry token `wowlua login` saved (the OS credential store may ask you to allow wowlua)",
        );
    }
    crate::credentials::get("registry").map_err(|_| {
        Diagnostic::error("E4201", "no API token for the wowlua registry").with_help(format!(
            "set `WOWLUA_REGISTRY_TOKEN`, or run `wowlua login registry`; sign in at {registry}/login to create a token"
        ))
    })
}

fn refused(registry: &str, r: &super::http::Response) -> Diagnostic {
    Diagnostic::plain_error(format!(
        "the registry `{registry}` refused it (HTTP {}): {}",
        r.status,
        error_text(&r.body).unwrap_or_else(|| r.body.trim().to_string())
    ))
}

/// `wowlua yank <name>@<version> [--undo]`.
pub fn yank(
    http: &Http,
    registry: &str,
    token: &str,
    name: &str,
    version: &str,
    undo: bool,
) -> Result<(), Diagnostic> {
    let api = api_base(http, registry)?;
    let (method, url) = if undo {
        ("PUT", format!("{api}/v1/packages/{name}/{version}/unyank"))
    } else {
        ("DELETE", format!("{api}/v1/packages/{name}/{version}"))
    };
    let headers = [
        ("authorization", format!("Bearer {token}")),
        ("accept", "application/json".to_string()),
    ];
    match http.retried(method, &url, &headers, None) {
        Ok(r) if r.ok() => Ok(()),
        Ok(r) => Err(refused(registry, &r)),
        Err(e) => Err(Diagnostic::plain_error(format!(
            "cannot reach the registry `{registry}`: {e}"
        ))),
    }
}

/// What `wowlua owner` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerChange {
    List,
    Add,
    Remove,
}

/// `wowlua owner list|add|remove <name> [logins]`: the owners afterwards.
pub fn owners(
    http: &Http,
    registry: &str,
    token: Option<&str>,
    name: &str,
    change: OwnerChange,
    users: &[String],
) -> Result<Vec<String>, Diagnostic> {
    let api = api_base(http, registry)?;
    let url = format!("{api}/v1/packages/{name}/owners");
    let mut headers = vec![("accept", "application/json".to_string())];
    if let Some(t) = token {
        headers.push(("authorization", format!("Bearer {t}")));
    }
    let body = serde_json::json!({ "users": users }).to_string();
    let result = match change {
        OwnerChange::List => http.retried("GET", &url, &headers, None),
        OwnerChange::Add | OwnerChange::Remove => {
            headers.push(("content-type", "application/json".to_string()));
            let method = if change == OwnerChange::Add {
                "PUT"
            } else {
                "DELETE"
            };
            http.retried(method, &url, &headers, Some(body.as_bytes()))
        }
    };
    let r = result.map_err(|e| {
        Diagnostic::plain_error(format!("cannot reach the registry `{registry}`: {e}"))
    })?;
    if !r.ok() {
        return Err(refused(registry, &r));
    }
    let json = r.json().unwrap_or_default();
    Ok(json["users"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|u| {
                    u.as_str()
                        .or_else(|| u["login"].as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default())
}

/// One search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub vers: String,
    pub description: String,
}

/// A page of results and how many there are.
#[derive(Clone, Debug, Default)]
pub struct SearchPage {
    pub packages: Vec<Found>,
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
}

fn encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `wowlua search <words>`: `GET {api}/v1/search`.
pub fn search(
    http: &Http,
    registry: &str,
    words: &str,
    page: usize,
    per_page: usize,
) -> Result<SearchPage, Diagnostic> {
    let api = api_base(http, registry)?;
    let url = format!(
        "{api}/v1/search?q={}&page={page}&per_page={per_page}",
        encode(words)
    );
    let r = http
        .retried(
            "GET",
            &url,
            &[("accept", "application/json".to_string())],
            None,
        )
        .map_err(|e| {
            Diagnostic::plain_error(format!("cannot reach the registry `{registry}`: {e}"))
        })?;
    if !r.ok() {
        return Err(refused(registry, &r));
    }
    let json = r.json().unwrap_or_default();
    let field = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    Ok(SearchPage {
        packages: json["packages"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| Found {
                        name: field(p, "name"),
                        vers: field(p, "vers"),
                        description: field(p, "description"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        total: json["total"].as_u64().unwrap_or(0) as usize,
        page: json["page"].as_u64().unwrap_or(page as u64) as usize,
        per_page: json["per_page"].as_u64().unwrap_or(per_page as u64) as usize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_normalized_manifest_drops_what_only_the_repository_uses() {
        let text = r#"{
  "package": { "name": "lib-a", "version": "1.0.0", "kind": "library" },
  "dependencies": { "lib-b": "^1", "lib-c": { "version": "^2", "registry": "work" } },
  "dev-dependencies": { "mocks": { "path": "../mocks" } },
  "publish": { "targets": {} }
}"#;
        let mut registries = wowlua_pkg::source::Registries::default();
        registries
            .named
            .insert("work".into(), "https://registry.example.com".into());
        let out = normalized_manifest(text, &registries).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["package"]["name"], "lib-a");
        assert_eq!(value["dependencies"]["lib-b"], "^1");
        assert_eq!(
            value["dependencies"]["lib-c"]["registry"],
            "https://registry.example.com"
        );
        assert!(value.get("dev-dependencies").is_none());
        assert!(value.get("publish").is_none());
        assert!(out.ends_with("}\n"));
    }

    #[test]
    fn registry_errors_read_as_text() {
        assert_eq!(
            error_text(r#"{"errors":[{"code":"version-exists","detail":"lib-a 1.0.0 exists"}]}"#)
                .as_deref(),
            Some("lib-a 1.0.0 exists (version-exists)")
        );
        assert_eq!(error_text("nope"), None);
    }
}
