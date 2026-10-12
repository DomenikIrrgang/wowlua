//! `wowlua package` (spec Publishing §11.2): builds every publish target's
//! configuration into `dist/targets/<target>/` and packs reproducible zips
//! into `dist/packages/`.
//!
//! A converted packager project (`compat.packager-keywords`) packages like
//! the BigWigs packager (decision T157): one zip with every AddOn folder of
//! the repository (one per game for `compat.per-game` projects), named from
//! the packager's templates, plus its nolib zip with `compat.nolib-package`.

use crate::build::{BuildOutput, BuildRequest};
use crate::{Project, Session};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::model::{ReleaseType, Target, TargetType};

/// What to package.
#[derive(Clone, Debug, Default)]
pub struct PackageRequest {
    /// `--target`: only these targets (all enabled ones when empty).
    pub targets: Vec<String>,
    /// `--flavor`: only these flavors.
    pub flavors: Option<Vec<String>>,
    /// `--release-type` over the manifest's.
    pub release_type: Option<ReleaseType>,
    /// `--allow-dirty`: a dirty working tree is allowed (an alpha build).
    pub allow_dirty: bool,
    /// `publish --tag` in a dry run: the tag the build is treated as on.
    pub tag: Option<String>,
}

/// The release type of a build (`alpha`, `beta`, `release`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    Alpha,
    Beta,
    Release,
}

impl Release {
    pub fn as_str(self) -> &'static str {
        match self {
            Release::Alpha => "alpha",
            Release::Beta => "beta",
            Release::Release => "release",
        }
    }
}

/// One packaged zip.
#[derive(Clone, Debug)]
pub struct Zip {
    pub path: PathBuf,
    pub file_name: String,
    /// The display name or upload label on the sites.
    pub label: String,
    /// The targets that upload it.
    pub targets: Vec<String>,
    pub nolib: bool,
    /// The flavors it contains, with their interface numbers.
    pub interfaces: BTreeMap<String, Vec<u32>>,
    /// The AddOn folders it contains.
    pub addons: Vec<String>,
    /// The builds in it (source maps, versions).
    pub builds: Vec<BuildOutput>,
    /// The package's version string (`build.version`, or the packager's
    /// `@project-version@`).
    pub version: String,
    /// Bundled by the workspace root's `publish.bundle-addons` (its targets
    /// upload it).
    pub bundle: bool,
}

/// The result of packaging.
#[derive(Clone, Debug)]
pub struct Packaged {
    pub zips: Vec<Zip>,
    pub release: Release,
    /// The tag on `HEAD`.
    pub tag: Option<String>,
    /// Packaged like the BigWigs packager.
    pub packager: bool,
    /// The repository root.
    pub repo: PathBuf,
    /// The project title (`release.json` names, changelogs): the packager's
    /// `changelog-title` or plain TOC title in packager mode.
    pub title: String,
    /// The AddOn whose folder holds the package files (changelog).
    pub main: String,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Whether the working tree has uncommitted changes (the build output
/// folder `out` aside).
pub fn dirty(dir: &Path, out: &Path) -> bool {
    let mut args = vec![
        "status".to_string(),
        "--porcelain".into(),
        "--".into(),
        ".".into(),
    ];
    if let Ok(rel) = normalized(out).strip_prefix(normalized(dir)) {
        args.push(format!(
            ":(exclude){}",
            rel.to_string_lossy().replace('\\', "/")
        ));
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(dir, &args).is_some_and(|s| !s.is_empty())
}

/// A path with `.` and `..` components resolved lexically.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The output root of a project (`build.out`; a workspace member's is the
/// workspace's).
pub fn out_root(project: &Project) -> PathBuf {
    normalized(&project.manifest.out_dir())
}

/// The release type (Publishing §11.1.2): explicit, else from the tag on
/// `HEAD` (untagged or, natively, dirty builds are alphas).
pub fn release_type(
    explicit: Option<ReleaseType>,
    tag: Option<&str>,
    dirty: bool,
    packager: bool,
) -> Release {
    match explicit {
        Some(ReleaseType::Release) => return Release::Release,
        Some(ReleaseType::Beta) => return Release::Beta,
        Some(ReleaseType::Alpha) => return Release::Alpha,
        _ => {}
    }
    let Some(tag) = tag else {
        return Release::Alpha;
    };
    // The packager does not look at the working tree.
    if dirty && !packager {
        return Release::Alpha;
    }
    let lower = tag.to_ascii_lowercase();
    if lower.contains("alpha") {
        Release::Alpha
    } else if lower.contains("beta") {
        Release::Beta
    } else {
        Release::Release
    }
}

/// A target's effective build settings; targets with equal settings share
/// one build and one zip.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Config {
    profile: String,
    minify: Option<String>,
    bundle: Option<bool>,
    flavors: Option<Vec<String>>,
    nolib: bool,
    split_flavors: bool,
}

fn config_of(target: &Target) -> Config {
    Config {
        profile: target.profile.clone().unwrap_or_else(|| "release".into()),
        minify: target.minify.clone(),
        bundle: target.bundle,
        flavors: target.flavors.clone(),
        nolib: target.nolib,
        split_flavors: target.split_flavors,
    }
}

/// Zip timestamps: `SOURCE_DATE_EPOCH`, else the time of `HEAD`.
pub(crate) fn zip_time(repo: &Path) -> zip::DateTime {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| git(repo, &["log", "-1", "--format=%ct"]).and_then(|s| s.parse().ok()))
        .unwrap_or(315_532_800);
    let iso = crate::keywords::iso_date(secs.max(315_532_800));
    let num = |a: usize, b: usize| iso[a..b].parse::<u16>().unwrap_or(1);
    zip::DateTime::from_date_and_time(
        num(0, 4),
        num(5, 7) as u8,
        num(8, 10) as u8,
        num(11, 13) as u8,
        num(14, 16) as u8,
        num(17, 19) as u8,
    )
    .unwrap_or_default()
}

/// Writes a zip of AddOn folders (`(name in the zip, folder)`), with sorted
/// `/` entries and normalized timestamps, so packages are reproducible.
pub fn write_zip(
    path: &Path,
    folders: &[(String, PathBuf)],
    extra: &[(String, PathBuf)],
    time: zip::DateTime,
) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut entries: BTreeMap<String, Option<PathBuf>> = BTreeMap::new();
    for (name, dir) in folders {
        entries.insert(format!("{name}/"), None);
        for e in walkdir::WalkDir::new(dir)
            .min_depth(1)
            .into_iter()
            .flatten()
        {
            let Ok(rel) = e.path().strip_prefix(dir) else {
                continue;
            };
            let rel = format!("{name}/{}", rel.to_string_lossy().replace('\\', "/"));
            if e.file_type().is_dir() {
                entries.insert(format!("{rel}/"), None);
            } else if e.file_type().is_file() {
                entries.insert(rel, Some(e.path().to_path_buf()));
            }
        }
    }
    for (rel, file) in extra {
        entries.insert(rel.clone(), Some(file.clone()));
    }
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(time);
    for (name, source) in entries {
        match source {
            None => zip
                .add_directory(name.trim_end_matches('/'), options.unix_permissions(0o755))
                .map_err(std::io::Error::other)?,
            Some(source) => {
                zip.start_file(name, options.unix_permissions(0o644))
                    .map_err(std::io::Error::other)?;
                zip.write_all(&std::fs::read(source)?)?;
            }
        }
    }
    zip.finish().map_err(std::io::Error::other)?;
    Ok(())
}

/// The packager's game type of an interface number (`toc_to_type`).
pub fn packager_type(interface: u32) -> &'static str {
    match interface {
        11000..=11999 => "classic",
        16000..=16999 => "forever",
        20000..=20999 => "bcc",
        38000..=38099 => "titan",
        30000..=30999 => "wrath",
        40000..=40999 => "cata",
        50000..=50999 => "mists",
        _ => "retail",
    }
}

/// The packager's `filename_filter`: template tokens and flags, and every
/// character outside `[A-Za-z0-9._!-]` replaced by `_`.
pub struct NameValues<'a> {
    pub package: &'a str,
    pub info: &'a crate::keywords::KeywordInfo,
    pub game_type: Option<&'a str>,
    pub release: Release,
    pub nolib: bool,
}

pub fn packager_name(template: &str, v: &NameValues) -> String {
    let version_lower = v.info.version.to_ascii_lowercase();
    let classic = match v.game_type {
        Some(t) if t != "retail" && !version_lower.contains(&format!("-{t}")) => format!("-{t}"),
        _ => String::new(),
    };
    let (iso, int) = crate::keywords::date_pair(v.info.timestamp);
    let flag = |on: bool, s: &str| if on { s.to_string() } else { String::new() };
    let text = template
        .replace("{package-name}", v.package)
        .replace("{project-revision}", &v.info.revision)
        .replace("{project-hash}", &v.info.hash)
        .replace("{project-abbreviated-hash}", &v.info.short_hash)
        .replace("{project-author}", &v.info.author)
        .replace("{project-date-iso}", &iso)
        .replace("{project-date-integer}", &int)
        .replace("{project-timestamp}", &v.info.timestamp.to_string())
        .replace("{project-version}", &v.info.version)
        .replace("{game-type}", v.game_type.unwrap_or(""))
        .replace("{release-type}", v.release.as_str())
        .replace("{alpha}", &flag(v.release == Release::Alpha, "-alpha"))
        .replace("{beta}", &flag(v.release == Release::Beta, "-beta"))
        .replace("{nolib}", &flag(v.nolib, "-nolib"))
        .replace("{classic}", &classic);
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '!' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The packager's default file and label templates, or `-n "{file}:{label}"`.
fn packager_templates(archive_name: Option<&str>) -> (String, String) {
    const FILE: &str = "{package-name}-{project-version}{nolib}{classic}";
    const LABEL: &str = "{project-version}{classic}{nolib}";
    match archive_name {
        None => (FILE.into(), LABEL.into()),
        Some(n) => {
            let file = n.split(':').next().unwrap_or(n);
            let label = n.rsplit(':').next().unwrap_or(n);
            (
                if file.is_empty() {
                    FILE.into()
                } else {
                    file.into()
                },
                if label.is_empty() {
                    LABEL.into()
                } else {
                    label.into()
                },
            )
        }
    }
}

/// An enabled publish target selected by `--target`.
fn selected(name: &String, t: &Target, request: &PackageRequest) -> bool {
    t.enabled
        && t.target_type != TargetType::Registry
        && (request.targets.is_empty() || request.targets.contains(name))
}

/// The enabled publish targets of a manifest selected by `--target`.
fn selected_targets(project: &Project, request: &PackageRequest) -> Vec<(String, Target)> {
    project
        .manifest
        .manifest
        .publish
        .iter()
        .flat_map(|p| p.targets.iter())
        .filter(|(name, t)| selected(name, t, request))
        .map(|(n, t)| (n.clone(), t.clone()))
        .collect()
}

/// Builds and packages the projects (every member of a workspace).
pub fn package(
    session: &mut Session,
    projects: &[Project],
    request: &PackageRequest,
) -> Option<Packaged> {
    let first = projects.first()?;
    let repo = first
        .workspace_root
        .clone()
        .unwrap_or_else(|| first.manifest.root.clone());
    let tag = request
        .tag
        .clone()
        .or_else(|| crate::keywords::head_tag(&repo));
    let dirty = dirty(&repo, &out_root(first));
    let packager = first.manifest.manifest.compat.packager_keywords;
    if packager {
        package_like_the_packager(session, projects, request, &repo, tag)
    } else {
        package_native(session, projects, request, &repo, tag, dirty)
    }
}

/// AddOns packaged together: one project, or the members bundled by the
/// workspace root's `publish.bundle-addons` (the main one first).
struct Unit<'a> {
    members: Vec<&'a Project>,
    targets: Vec<(String, Target)>,
    bundle: bool,
}

/// The workspace root's `publish`, when it bundles members.
pub(crate) fn bundle_publish(
    repo: &Path,
) -> Option<(wowlua_manifest::model::Publish, Vec<String>)> {
    let text = std::fs::read_to_string(repo.join(wowlua_manifest::MANIFEST_FILE)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("workspace")?;
    let publish: wowlua_manifest::model::Publish =
        serde_json::from_value(value.get("publish")?.clone()).ok()?;
    let default_members = value
        .pointer("/workspace/default-members")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    match publish.bundle_addons {
        None | Some(wowlua_manifest::model::BundleAddons::All(false)) => None,
        Some(_) => Some((publish, default_members)),
    }
}

fn package_units<'a>(
    session: &mut Session,
    projects: &'a [Project],
    request: &PackageRequest,
    repo: &Path,
) -> Option<Vec<Unit<'a>>> {
    let addons: Vec<&Project> = projects
        .iter()
        .filter(|p| p.kind() == wowlua_manifest::model::PackageKind::Addon)
        .collect();
    let mut units = Vec::new();
    let bundled: Vec<&Project> = addons.iter().copied().filter(|p| p.bundled).collect();
    // The bundle, when the command runs at the workspace root.
    if !bundled.is_empty()
        && let Some((publish, default_members)) = bundle_publish(repo)
    {
        let same =
            |a: &Path, b: &Path| std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok();
        let mut members: Vec<&Project> = Vec::new();
        match &publish.bundle_addons {
            Some(wowlua_manifest::model::BundleAddons::Listed(names)) => {
                for name in names {
                    match bundled.iter().find(|p| {
                        p.addon_name() == *name
                            || p.manifest.package().is_some_and(|k| k.name == *name)
                    }) {
                        Some(p) => members.push(*p),
                        None => {
                            session.push(
                                Diagnostic::error(
                                    "E4006",
                                    format!(
                                        "`publish.bundle-addons` names `{name}`, which is no addon of the workspace"
                                    ),
                                )
                                .with_path(repo.join(wowlua_manifest::MANIFEST_FILE)),
                            );
                            return None;
                        }
                    }
                }
            }
            _ => {
                members = bundled.clone();
                // The main AddOn: the first default member.
                if let Some(i) = default_members.iter().find_map(|d| {
                    members
                        .iter()
                        .position(|p| same(&p.manifest.root, &repo.join(d)))
                }) {
                    let main = members.remove(i);
                    members.insert(0, main);
                }
            }
        }
        members.dedup_by(|a, b| std::ptr::eq(*a, *b));
        let targets: Vec<(String, Target)> = publish
            .targets
            .iter()
            .filter(|(name, t)| selected(name, t, request))
            .map(|(n, t)| (n.clone(), t.clone()))
            .collect();
        if !targets.is_empty() {
            units.push(Unit {
                members,
                targets,
                bundle: true,
            });
        }
    }
    for project in addons {
        // A root package's targets are the bundle's.
        let root_package =
            project.workspace_root.as_deref() == Some(project.manifest.root.as_path());
        if !project.selected || (!bundled.is_empty() && root_package) {
            continue;
        }
        let targets = selected_targets(project, request);
        if !targets.is_empty() {
            units.push(Unit {
                members: vec![project],
                targets,
                bundle: false,
            });
        }
    }
    Some(units)
}

fn package_native(
    session: &mut Session,
    projects: &[Project],
    request: &PackageRequest,
    repo: &Path,
    tag: Option<String>,
    dirty: bool,
) -> Option<Packaged> {
    let time = zip_time(repo);
    let mut zips = Vec::new();
    let mut release_setting = None;
    for unit in package_units(session, projects, request, repo)? {
        let main = unit.members[0];
        release_setting = release_setting.or_else(|| {
            main.manifest
                .manifest
                .publish
                .as_ref()
                .and_then(|p| p.release_type)
        });
        // Targets with equal settings share a build.
        let mut configs: Vec<(Config, Vec<String>)> = Vec::new();
        for (name, target) in &unit.targets {
            let c = config_of(target);
            match configs.iter_mut().find(|(x, _)| *x == c) {
                Some((_, names)) => names.push(name.clone()),
                None => configs.push((c, vec![name.clone()])),
            }
        }
        let several = configs.len() > 1;
        let out_root = out_root(main);
        for (config, names) in &configs {
            let flavors = request.flavors.clone().or_else(|| config.flavors.clone());
            let all = main.manifest.flavors.clone();
            let groups: Vec<Option<Vec<String>>> = if config.split_flavors {
                flavors
                    .clone()
                    .unwrap_or(all)
                    .into_iter()
                    .map(|f| Some(vec![f]))
                    .collect()
            } else {
                vec![flavors.clone()]
            };
            for group in groups {
                let dir_name = match &group {
                    Some(g) if config.split_flavors => format!("{}-{}", names[0], g[0]),
                    _ => names[0].clone(),
                };
                let mut builds = Vec::new();
                let mut folders = Vec::new();
                let mut license: Vec<(String, PathBuf)> = Vec::new();
                for project in &unit.members {
                    let addon = project.addon_name();
                    let out_dir = out_root.join("targets").join(&dir_name).join(&addon);
                    let build_request = BuildRequest {
                        profile: config.profile.clone(),
                        flavors: group.clone(),
                        out_dir: Some(out_dir.clone()),
                        write: true,
                        nolib: config.nolib,
                        minify: config.minify.clone(),
                        bundle: config.bundle,
                    };
                    let build = crate::build(session, project, &build_request)?;
                    // The package's own license files ship next to the AddOn.
                    license.extend(
                        std::fs::read_dir(&project.manifest.root)
                            .into_iter()
                            .flatten()
                            .flatten()
                            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                            .filter(|e| {
                                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                                n == "license" || n.starts_with("license.")
                            })
                            .filter(|e| {
                                !build
                                    .files
                                    .contains_key(&e.file_name().to_string_lossy().into_owned())
                            })
                            .map(|e| {
                                (
                                    format!("{addon}/{}", e.file_name().to_string_lossy()),
                                    e.path(),
                                )
                            }),
                    );
                    folders.push((addon, out_dir));
                    builds.push(build);
                }
                let version = builds[0].version.clone();
                let mut file_name = format!("{}-{version}", builds[0].addon_name);
                if several {
                    file_name.push_str(&format!("-{}", names[0]));
                }
                if config.split_flavors
                    && let Some(g) = &group
                {
                    file_name.push_str(&format!("-{}", g[0]));
                }
                if config.nolib {
                    file_name.push_str("-nolib");
                }
                file_name.push_str(".zip");
                let path = out_root.join("packages").join(&file_name);
                if let Err(e) = write_zip(&path, &folders, &license, time) {
                    session.push(
                        Diagnostic::plain_error(format!("cannot write `{}`: {e}", path.display()))
                            .with_path(&path),
                    );
                    return None;
                }
                // Every member's interface numbers per flavor.
                let mut interfaces: BTreeMap<String, Vec<u32>> = BTreeMap::new();
                for (project, build) in unit.members.iter().zip(&builds) {
                    for f in &build.flavors {
                        let list = interfaces.entry(f.clone()).or_default();
                        for i in project.interfaces.get(f).into_iter().flatten() {
                            if !list.contains(i) {
                                list.push(*i);
                            }
                        }
                    }
                }
                zips.push(Zip {
                    path,
                    file_name,
                    label: version.clone(),
                    targets: names.clone(),
                    nolib: config.nolib,
                    interfaces,
                    addons: builds.iter().map(|b| b.addon_name.clone()).collect(),
                    version,
                    builds,
                    bundle: unit.bundle,
                });
            }
        }
    }
    Some(Packaged {
        release: release_type(
            request.release_type.or(release_setting),
            tag.as_deref(),
            dirty,
            false,
        ),
        zips,
        tag,
        packager: false,
        repo: repo.to_path_buf(),
        title: crate::packaging::plain_title(&projects[0].manifest.title()),
        main: projects[0].addon_name(),
    })
}

fn package_like_the_packager(
    session: &mut Session,
    projects: &[Project],
    request: &PackageRequest,
    repo: &Path,
    tag: Option<String>,
) -> Option<Packaged> {
    let first = &projects[0];
    let compat = &first.manifest.manifest.compat;
    // The AddOn the package folder becomes: the root addon, or the member
    // named `compat.package-as` of a workspace without one.
    let package_name = std::fs::read_to_string(repo.join(wowlua_manifest::MANIFEST_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|m| {
            m.pointer("/compat/package-as")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| first.addon_name());
    let main = projects
        .iter()
        .find(|p| p.addon_name() == package_name)
        .unwrap_or(first);
    let targets = selected_targets(main, request);
    let release_setting = main
        .manifest
        .manifest
        .publish
        .as_ref()
        .and_then(|p| p.release_type);
    let release = release_type(
        request.release_type.or(release_setting),
        tag.as_deref(),
        false,
        true,
    );
    let info = crate::keywords::KeywordInfo::from_git(repo, "", false, false, 0);
    let (file_template, label_template) = packager_templates(compat.archive_name.as_deref());
    let time = zip_time(repo);
    let out_root = out_root(main);
    let all_targets: Vec<String> = targets.iter().map(|(n, _)| n.clone()).collect();
    let github: Vec<String> = targets
        .iter()
        .filter(|(_, t)| t.target_type == TargetType::Github)
        .map(|(n, _)| n.clone())
        .collect();
    // One package per game for projects released game by game.
    let flavors = request
        .flavors
        .clone()
        .unwrap_or_else(|| main.manifest.flavors.clone());
    let games: Vec<Option<Vec<String>>> = if compat.per_game() {
        // The games the packager runs release.
        flavors
            .iter()
            .filter(|f| {
                compat
                    .per_game_flavors()
                    .is_none_or(|list| list.contains(f))
            })
            .map(|f| Some(vec![f.clone()]))
            .collect()
    } else {
        vec![request.flavors.clone()]
    };
    let mut zips = Vec::new();
    for game in games {
        for nolib in [false, true] {
            if nolib && !compat.nolib_package {
                continue;
            }
            let variant = match (&game, nolib) {
                (Some(g), false) => format!("package-{}", g[0]),
                (Some(g), true) => format!("package-{}-nolib", g[0]),
                (None, false) => "package".to_string(),
                (None, true) => "package-nolib".to_string(),
            };
            let dir = out_root.join("targets").join(&variant);
            let mut builds = Vec::new();
            for project in projects {
                if project.kind() != wowlua_manifest::model::PackageKind::Addon {
                    continue;
                }
                let build_request = BuildRequest {
                    profile: "release".into(),
                    flavors: game.clone(),
                    out_dir: Some(dir.join(project.addon_name())),
                    write: true,
                    nolib,
                    minify: None,
                    bundle: None,
                };
                builds.push(crate::build(session, project, &build_request)?);
            }
            // External addons (fetched externals) are not part of a nolib
            // package, like the packager's nolib exclusions.
            if !nolib && let Ok(profile) = main.manifest.profile("release") {
                for ext in &main.external_addons {
                    if let Some(b) =
                        crate::external::build_external(session, ext, &profile, &dir, true)
                    {
                        // `build_external` writes to `<dir>/<profile>/<name>`.
                        let target = dir.join(&b.addon_name);
                        let _ = std::fs::remove_dir_all(&target);
                        if std::fs::rename(&b.dir, &target).is_ok() {
                            let _ = std::fs::remove_dir(dir.join("release"));
                            builds.push(BuildOutput { dir: target, ..b });
                        }
                    }
                }
            }
            // The game type: the game of a single-game package, or the only
            // one the interface numbers name.
            let mut interfaces: BTreeMap<String, Vec<u32>> = BTreeMap::new();
            let shown: Vec<String> = game.clone().unwrap_or_else(|| flavors.clone());
            for f in &shown {
                interfaces.insert(
                    f.clone(),
                    main.interfaces.get(f).cloned().unwrap_or_default(),
                );
            }
            let types: BTreeSet<&str> = interfaces
                .values()
                .flatten()
                .map(|i| packager_type(*i))
                .collect();
            let game_type = (types.len() == 1).then(|| *types.iter().next().unwrap_or(&""));
            let values = NameValues {
                package: &package_name,
                info: &info,
                game_type,
                release,
                nolib,
            };
            let mut file_name = packager_name(&file_template, &values);
            let mut label = packager_name(&label_template, &values);
            if nolib {
                let plain = NameValues {
                    nolib: false,
                    ..values
                };
                // A template without `{nolib}`: the packager appends it.
                if packager_name(&file_template, &plain) == file_name {
                    file_name.push_str("-nolib");
                }
                if packager_name(&label_template, &plain) == label {
                    label.push_str("-nolib");
                }
            }
            file_name.push_str(".zip");
            let path = out_root.join("packages").join(&file_name);
            let folders: Vec<(String, PathBuf)> = builds
                .iter()
                .map(|b| (b.addon_name.clone(), b.dir.clone()))
                .collect();
            if let Err(e) = write_zip(&path, &folders, &[], time) {
                session.push(
                    Diagnostic::plain_error(format!("cannot write `{}`: {e}", path.display()))
                        .with_path(&path),
                );
                return None;
            }
            zips.push(Zip {
                path,
                file_name,
                label,
                // The packager uploads its nolib zip only to GitHub.
                targets: if nolib {
                    github.clone()
                } else {
                    all_targets.clone()
                },
                nolib,
                interfaces,
                addons: builds.iter().map(|b| b.addon_name.clone()).collect(),
                version: info.version.clone(),
                builds,
                bundle: false,
            });
        }
    }
    Some(Packaged {
        zips,
        release,
        tag,
        packager: true,
        repo: repo.to_path_buf(),
        // Like the package files the build adds: the repository manifest's.
        title: std::fs::read_to_string(repo.join(wowlua_manifest::MANIFEST_FILE))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|m| {
                m.pointer("/compat/changelog-title")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| crate::packaging::plain_title(&main.manifest.title())),
        main: package_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_zips_like_the_packager() {
        let info = crate::keywords::KeywordInfo {
            version: "v1.2.3".into(),
            ..Default::default()
        };
        let values = NameValues {
            package: "My Addon",
            info: &info,
            game_type: Some("classic"),
            release: Release::Release,
            nolib: false,
        };
        let (file, label) = packager_templates(None);
        assert_eq!(packager_name(&file, &values), "My_Addon-v1.2.3-classic");
        assert_eq!(packager_name(&label, &values), "v1.2.3-classic");
        let retail = NameValues {
            game_type: Some("retail"),
            nolib: true,
            ..values
        };
        assert_eq!(packager_name(&file, &retail), "My_Addon-v1.2.3-nolib");
        assert_eq!(
            packager_templates(Some("{project-version}:{project-version}")),
            ("{project-version}".into(), "{project-version}".into())
        );
        assert_eq!(
            release_type(None, Some("v1.0-beta2"), false, true),
            Release::Beta
        );
        assert_eq!(
            release_type(None, Some("v1.0"), true, false),
            Release::Alpha
        );
        assert_eq!(
            release_type(None, Some("v1.0"), true, true),
            Release::Release
        );
        assert_eq!(packager_type(38002), "titan");
        assert_eq!(packager_type(120100), "retail");
    }
}
