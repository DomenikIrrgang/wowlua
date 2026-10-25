//! The upload clients (Publishing §11.4–§11.7): CurseForge, Wago Addons,
//! WoWInterface and GitHub Releases. Endpoints can be redirected with the
//! target's `api-url`.

// Only a computer has this (spec M14, decision T394).
#![allow(clippy::disallowed_methods)]

use super::changelog::Changelog;
use super::http::{Http, Multipart, Response};
use super::versions::{self, Choice};
use crate::package::{Release, Zip};
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::model::{Prerelease, Target};

/// What one upload needs.
pub struct Upload<'a> {
    pub http: &'a Http,
    pub name: &'a str,
    pub target: &'a Target,
    pub token: &'a str,
    pub zip: &'a Zip,
    pub release: Release,
    pub changelog: Option<&'a Changelog>,
    /// Packaged like the BigWigs packager.
    pub packager: bool,
    pub dry_run: bool,
}

/// The result of one upload.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub target: String,
    pub file: String,
    pub ok: bool,
    /// Skipped on purpose (untagged builds for WoWInterface and GitHub).
    pub skipped: bool,
    pub message: String,
    /// The error code of a failure (E4202, or E4210 without a changelog).
    pub code: &'static str,
    pub warnings: Vec<Diagnostic>,
}

impl Outcome {
    fn new(u: &Upload) -> Self {
        Outcome {
            target: u.name.to_string(),
            file: u.zip.file_name.clone(),
            ok: false,
            skipped: false,
            message: String::new(),
            code: "E4202",
            warnings: Vec::new(),
        }
    }

    /// A target skipped on purpose.
    pub fn skipped(target: &str, file: &str, message: &str) -> Self {
        Outcome {
            target: target.to_string(),
            file: file.to_string(),
            ok: true,
            skipped: true,
            message: message.to_string(),
            code: "E4202",
            warnings: Vec::new(),
        }
    }

    /// A failed upload.
    pub fn failed(target: &str, file: &str, message: &str) -> Self {
        Outcome {
            ok: false,
            skipped: false,
            ..Outcome::skipped(target, file, message)
        }
    }

    fn skip(mut self, message: impl Into<String>) -> Self {
        self.ok = true;
        self.skipped = true;
        self.message = message.into();
        self
    }

    fn fail(mut self, message: impl Into<String>) -> Self {
        self.ok = false;
        self.message = message.into();
        self
    }

    fn done(mut self, message: impl Into<String>) -> Self {
        self.ok = true;
        self.message = message.into();
        self
    }
}

fn base<'a>(target: &'a Target, default: &'a str) -> &'a str {
    target
        .api_url
        .as_deref()
        .unwrap_or(default)
        .trim_end_matches('/')
}

fn project_id(target: &Target) -> String {
    match &target.project_id {
        Some(wowlua_manifest::model::ProjectId::Number(n)) => n.to_string(),
        Some(wowlua_manifest::model::ProjectId::String(s)) => s.clone(),
        None => String::new(),
    }
}

fn rejected(site: &str, r: &Response) -> String {
    let body: String = r.body.chars().take(400).collect();
    format!("{site} answered HTTP {}: {}", r.status, body.trim())
}

/// The game versions of a zip on one site, with W4328 / W4220 warnings.
fn chosen<T: Clone>(
    u: &Upload,
    site: &str,
    pick: impl Fn(&versions::GameType, &str) -> Choice<T>,
    warnings: &mut Vec<Diagnostic>,
) -> Vec<(&'static versions::GameType, T, String)> {
    let mut out = Vec::new();
    for (ty, list) in versions::versions(&u.zip.interfaces) {
        let Some(game) = versions::game_type(ty) else {
            continue;
        };
        for (version, _) in list {
            match pick(game, &version) {
                Choice::Exact(v) => out.push((game, v, version)),
                Choice::Fallback {
                    wanted,
                    chosen,
                    shown,
                } => {
                    warnings.push(Diagnostic::warning(
                        "W4328",
                        format!("{site} does not know {ty} version {wanted}; using {shown}"),
                    ));
                    out.push((game, chosen, shown));
                }
                Choice::Unknown(v) => warnings.push(Diagnostic::warning(
                    "W4220",
                    format!("{site} has no game version for {ty} {v}; left out of the upload"),
                )),
            }
        }
    }
    out
}

/// CurseForge (§11.4): `POST /api/projects/{id}/upload-file`.
pub fn curseforge(u: &Upload, relations: &[(String, String)]) -> Outcome {
    let mut o = Outcome::new(u);
    let base = base(u.target, "https://wow.curseforge.com");
    let auth = [("x-api-token", u.token.to_string())];
    let Some(changelog) = u.changelog else {
        o.code = "E4210";
        return o.fail("CurseForge needs a changelog for every file");
    };
    let list = match u
        .http
        .retried("GET", &format!("{base}/api/game/wow/versions"), &auth, None)
    {
        Ok(r) if r.ok() => r.json().unwrap_or_default(),
        Ok(r) => return o.fail(rejected("CurseForge", &r)),
        Err(e) => return o.fail(format!("cannot reach CurseForge: {e}")),
    };
    let mut warnings = Vec::new();
    let picked = chosen(
        u,
        "CurseForge",
        |g, v| versions::curseforge(&list, g, v),
        &mut warnings,
    );
    o.warnings = warnings;
    let mut ids: Vec<u64> = Vec::new();
    for (_, id, _) in &picked {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    if ids.is_empty() {
        return o.fail("no game version CurseForge knows");
    }
    let mut metadata = serde_json::json!({
        "displayName": u.zip.label,
        "gameVersions": ids,
        "releaseType": u.release.as_str(),
        "changelog": changelog.text,
        "changelogType": changelog.curseforge_type(),
    });
    if !relations.is_empty() {
        let projects: Vec<serde_json::Value> = relations
            .iter()
            .map(|(slug, kind)| serde_json::json!({ "slug": slug, "type": kind }))
            .collect();
        metadata["relations"] = serde_json::json!({ "projects": projects });
    }
    if u.dry_run {
        return o.done(format!("would upload with {metadata}"));
    }
    let Ok(bytes) = std::fs::read(&u.zip.path) else {
        return o.fail(format!("cannot read {}", u.zip.path.display()));
    };
    let (content_type, body) = Multipart::default()
        .text("metadata", &metadata.to_string())
        .file("file", &u.zip.file_name, "application/octet-stream", &bytes)
        .finish();
    let url = format!("{base}/api/projects/{}/upload-file", project_id(u.target));
    match u.http.retried(
        "POST",
        &url,
        &[auth[0].clone(), ("content-type", content_type)],
        Some(&body),
    ) {
        Ok(r) if r.status == 200 => {
            let id = r.json().and_then(|j| j["id"].as_u64()).unwrap_or_default();
            o.done(format!("file {id}"))
        }
        Ok(r) if r.status == 404 => {
            o.fail(format!("no CurseForge project {}", project_id(u.target)))
        }
        Ok(r) => o.fail(rejected("CurseForge", &r)),
        Err(e) => o.fail(format!("cannot reach CurseForge: {e}")),
    }
}

/// Wago Addons (§11.5): `POST /api/projects/{id}/version`.
pub fn wago(u: &Upload) -> Outcome {
    let mut o = Outcome::new(u);
    let base = base(u.target, "https://addons.wago.io");
    let game = match u
        .http
        .retried("GET", &format!("{base}/api/data/game"), &[], None)
    {
        Ok(r) if r.ok() => r.json().unwrap_or_default(),
        Ok(r) => return o.fail(rejected("Wago", &r)),
        Err(e) => return o.fail(format!("cannot reach Wago: {e}")),
    };
    let mut warnings = Vec::new();
    let picked = chosen(u, "Wago", |g, v| versions::wago(&game, g, v), &mut warnings);
    o.warnings = warnings;
    let mut metadata = serde_json::Map::new();
    metadata.insert("label".into(), serde_json::json!(u.zip.label));
    for (g, v, _) in &picked {
        let key = format!("supported_{}_patches", g.wago);
        let list = metadata
            .entry(key)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .expect("an array");
        if !list.iter().any(|x| x == v) {
            list.push(serde_json::json!(v));
        }
    }
    if picked.is_empty() {
        return o.fail("no game version Wago knows");
    }
    let stability = match u.release {
        Release::Release => "stable",
        Release::Beta => "beta",
        Release::Alpha => "alpha",
    };
    metadata.insert("stability".into(), serde_json::json!(stability));
    metadata.insert(
        "changelog".into(),
        serde_json::json!(u.changelog.map(|c| c.text.as_str()).unwrap_or_default()),
    );
    let metadata = serde_json::Value::Object(metadata);
    if u.dry_run {
        return o.done(format!("would upload with {metadata}"));
    }
    let Ok(bytes) = std::fs::read(&u.zip.path) else {
        return o.fail(format!("cannot read {}", u.zip.path.display()));
    };
    let (content_type, body) = Multipart::default()
        .text("metadata", &metadata.to_string())
        .file("file", &u.zip.file_name, "application/octet-stream", &bytes)
        .finish();
    let url = format!("{base}/api/projects/{}/version", project_id(u.target));
    let headers = [
        ("authorization", format!("Bearer {}", u.token)),
        ("accept", "application/json".to_string()),
        ("content-type", content_type),
    ];
    match u.http.retried("POST", &url, &headers, Some(&body)) {
        Ok(r) if r.status == 200 || r.status == 201 => o.done("uploaded"),
        Ok(r) if r.status == 404 => o.fail(format!("no Wago project {}", project_id(u.target))),
        Ok(r) => o.fail(rejected("Wago", &r)),
        Err(e) => o.fail(format!("cannot reach Wago: {e}")),
    }
}

/// WoWInterface (§11.6): `POST /addons/update`. `changelog` is what to send
/// (BBCode or Markdown), if anything.
pub fn wowinterface(u: &Upload, tagged: bool, changelog: Option<&str>) -> Outcome {
    let mut o = Outcome::new(u);
    if !tagged && !(u.target.upload_untagged.unwrap_or(false) && !u.packager) {
        return o.skip("only tagged builds go to WoWInterface (`upload-untagged` uploads others)");
    }
    let base = base(u.target, "https://api.wowinterface.com");
    let list = match u
        .http
        .retried("GET", &format!("{base}/addons/compatible.json"), &[], None)
    {
        Ok(r) if r.ok() => r.json().unwrap_or_default(),
        Ok(r) => return o.fail(rejected("WoWInterface", &r)),
        Err(e) => return o.fail(format!("cannot reach WoWInterface: {e}")),
    };
    let mut warnings = Vec::new();
    let picked = chosen(
        u,
        "WoWInterface",
        |g, v| versions::wowinterface(&list, g, v),
        &mut warnings,
    );
    o.warnings = warnings;
    let mut compatible: Vec<String> = Vec::new();
    for (_, v, _) in picked {
        if !compatible.contains(&v) {
            compatible.push(v);
        }
    }
    versions::sort_desc(&mut compatible);
    if compatible.is_empty() {
        return o.fail("no game version WoWInterface knows");
    }
    let compatible = compatible.join(",");
    if u.dry_run {
        return o.done(format!(
            "would upload id={} version={} compatible={compatible} changelog={}",
            project_id(u.target),
            u.zip.version,
            changelog.map_or(0, str::len)
        ));
    }
    let Ok(bytes) = std::fs::read(&u.zip.path) else {
        return o.fail(format!("cannot read {}", u.zip.path.display()));
    };
    let mut form = Multipart::default()
        .text("id", &project_id(u.target))
        .text("version", &u.zip.version)
        .text("compatible", &compatible);
    if let Some(c) = changelog {
        form = form.text("changelog", c);
    }
    if !u.target.archive_previous.unwrap_or(true) {
        form = form.text("archive", "No");
    }
    let (content_type, body) = form
        .file(
            "updatefile",
            &u.zip.file_name,
            "application/octet-stream",
            &bytes,
        )
        .finish();
    let headers = [
        ("x-api-token", u.token.to_string()),
        ("content-type", content_type),
    ];
    match u.http.retried(
        "POST",
        &format!("{base}/addons/update"),
        &headers,
        Some(&body),
    ) {
        Ok(r) if r.status == 202 => o.done("uploaded"),
        Ok(r) if r.status == 401 || r.status == 403 => o.fail(format!(
            "WoWInterface refused the upload (HTTP {}): wrong token, or no permission for addon {}",
            r.status,
            project_id(u.target)
        )),
        Ok(r) => o.fail(rejected("WoWInterface", &r)),
        Err(e) => o.fail(format!("cannot reach WoWInterface: {e}")),
    }
}

/// A `release.json` entry (§11.7).
pub fn release_entry(title: &str, version: &str, zip: &Zip) -> serde_json::Value {
    let mut metadata = Vec::new();
    for (ty, list) in versions::versions(&zip.interfaces) {
        let flavor = versions::game_type(ty).map_or(ty, |g| g.release_json);
        for (_, interface) in list {
            metadata.push(serde_json::json!({ "flavor": flavor, "interface": interface }));
        }
    }
    serde_json::json!({
        "name": title,
        "version": version,
        "filename": zip.file_name,
        "nolib": zip.nolib,
        "metadata": metadata,
    })
}

/// What a GitHub release gets.
pub struct GithubRelease<'a> {
    pub tag: &'a str,
    pub commit: Option<&'a str>,
    pub name: &'a str,
    pub body: &'a str,
    /// Zips (and their files) to attach, plus extra assets
    /// (`release.json`, archived source maps) as (name, path, type).
    pub zips: Vec<&'a Zip>,
    pub extra: Vec<(String, std::path::PathBuf, &'static str)>,
    /// `release.json` entries; merged with the release's existing ones.
    pub release_json: Option<Vec<serde_json::Value>>,
}

/// GitHub Releases (§11.7): create or update the release of the tag and
/// replace its assets.
pub fn github(u: &Upload, gh: &GithubRelease) -> Outcome {
    let o = Outcome::new(u);
    let api = base(u.target, "https://api.github.com");
    let Some(repo) = u.target.repository.as_deref() else {
        return o.fail("the GitHub target has no `repository`");
    };
    let scheme = if u.packager { "token" } else { "Bearer" };
    let auth = vec![
        ("authorization", format!("{scheme} {}", u.token)),
        ("accept", "application/vnd.github.v3+json".to_string()),
    ];
    let prerelease = match &u.target.prerelease {
        Some(Prerelease::Bool(b)) => *b,
        _ => u.release != Release::Release,
    };
    let mut payload = serde_json::json!({
        "tag_name": gh.tag,
        "name": gh.name,
        "body": gh.body,
        "draft": u.target.draft.unwrap_or(false),
        "prerelease": prerelease,
    });
    if let Some(c) = gh.commit {
        payload["target_commitish"] = serde_json::json!(c);
    }
    if u.dry_run {
        // A read-only request checks the token.
        if !u.token.is_empty() {
            match u
                .http
                .retried("GET", &format!("{api}/repos/{repo}"), &auth, None)
            {
                Ok(r) if r.ok() => {}
                Ok(r) => return o.fail(rejected("GitHub", &r)),
                Err(e) => return o.fail(format!("cannot reach GitHub: {e}")),
            }
        }
        return o.done(format!(
            "would publish release {payload} with {}",
            gh.zips
                .iter()
                .map(|z| z.file_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let existing = match u.http.retried(
        "GET",
        &format!("{api}/repos/{repo}/releases/tags/{}", gh.tag),
        &auth,
        None,
    ) {
        Ok(r) if r.ok() => r.json().and_then(|j| j["id"].as_u64()),
        Ok(_) => None,
        Err(e) => return o.fail(format!("cannot reach GitHub: {e}")),
    };
    let mut headers = auth.clone();
    headers.push(("content-type", "application/json".into()));
    let body = payload.to_string();
    let response = match existing {
        Some(id) => u.http.retried(
            "PATCH",
            &format!("{api}/repos/{repo}/releases/{id}"),
            &headers,
            Some(body.as_bytes()),
        ),
        None => u.http.retried(
            "POST",
            &format!("{api}/repos/{repo}/releases"),
            &headers,
            Some(body.as_bytes()),
        ),
    };
    let release = match response {
        Ok(r) if r.status == 200 || r.status == 201 => r.json().unwrap_or_default(),
        Ok(r) => return o.fail(rejected("GitHub", &r)),
        Err(e) => return o.fail(format!("cannot reach GitHub: {e}")),
    };
    let Some(id) = release["id"].as_u64().or(existing) else {
        return o.fail("GitHub returned no release id");
    };
    // Upload URL: the release's (`…/assets{?name,label}`), else the default.
    let uploads = release["upload_url"]
        .as_str()
        .map(|u| u.split('{').next().unwrap_or(u).to_string())
        .unwrap_or_else(|| format!("https://uploads.github.com/repos/{repo}/releases/{id}/assets"));
    let assets: Vec<(String, u64, String)> = u
        .http
        .retried(
            "GET",
            &format!("{api}/repos/{repo}/releases/{id}/assets"),
            &auth,
            None,
        )
        .ok()
        .and_then(|r| r.json())
        .and_then(|j| j.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|a| {
            Some((
                a["name"].as_str()?.to_string(),
                a["id"].as_u64()?,
                a["url"].as_str().unwrap_or_default().to_string(),
            ))
        })
        .collect();
    // `release.json`: merged with the release's by file name.
    let mut extra = gh.extra.clone();
    let json_path;
    if let Some(entries) = &gh.release_json {
        let mut all: Vec<serde_json::Value> = entries.clone();
        if let Some((_, _, url)) = assets.iter().find(|(n, _, _)| n == "release.json") {
            let mut h = auth.clone();
            h[1] = ("accept", "application/octet-stream".into());
            if let Some(old) = u
                .http
                .retried("GET", url, &h, None)
                .ok()
                .and_then(|r| r.json())
            {
                all.extend(old["releases"].as_array().cloned().unwrap_or_default());
            }
        }
        let mut seen = std::collections::BTreeMap::new();
        for e in all {
            let key = e["filename"].as_str().unwrap_or_default().to_string();
            seen.entry(key).or_insert(e);
        }
        let merged = serde_json::json!({ "releases": seen.into_values().collect::<Vec<_>>() });
        json_path =
            std::env::temp_dir().join(format!("wowlua-release-{}-{id}.json", std::process::id()));
        if std::fs::write(&json_path, merged.to_string()).is_ok() {
            extra.push(("release.json".into(), json_path.clone(), "application/json"));
        }
    }
    let mut files: Vec<(String, std::path::PathBuf, &str)> = gh
        .zips
        .iter()
        .map(|z| (z.file_name.clone(), z.path.clone(), "application/zip"))
        .collect();
    files.extend(extra);
    for (name, path, content_type) in &files {
        if let Some((_, asset, _)) = assets.iter().find(|(n, _, _)| n == name) {
            let _ = u.http.request(
                "DELETE",
                &format!("{api}/repos/{repo}/releases/assets/{asset}"),
                &auth,
                None,
            );
        }
        let Ok(bytes) = std::fs::read(path) else {
            return o.fail(format!("cannot read {}", path.display()));
        };
        let mut h = auth.clone();
        h.push(("content-type", content_type.to_string()));
        let url = format!("{uploads}?name={}", query_escape(name));
        match u.http.retried("POST", &url, &h, Some(&bytes)) {
            Ok(r) if r.status == 201 => {}
            Ok(r) => return o.fail(rejected("GitHub", &r)),
            Err(e) => return o.fail(format!("cannot reach GitHub: {e}")),
        }
    }
    let url = release["html_url"].as_str().unwrap_or_default().to_string();
    o.done(if url.is_empty() {
        format!("release {id}")
    } else {
        url
    })
}

/// A query parameter value, percent-encoded.
fn query_escape(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
