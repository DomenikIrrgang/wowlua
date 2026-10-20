//! `wowlua publish` (spec Publishing §11.9): package, then upload to every
//! publish target.
//!
//! Natively, the targets upload in parallel and a failing target does not
//! stop the others. A converted packager project (`compat.packager-keywords`)
//! publishes like the BigWigs packager (decision T157): per package, in the
//! order CurseForge, WoWInterface, Wago, GitHub; a failed CurseForge upload
//! is retried and then stops the publish; targets without a token are
//! skipped; the tokens come from the packager's variables and `.env` file.

pub mod changelog;
pub mod http;
pub mod sites;
pub mod versions;

use crate::package::{PackageRequest, Packaged, Zip};
use crate::{Project, Session};
use changelog::Changelog;
use sites::{GithubRelease, Outcome, Upload};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use wowlua_diagnostics::Diagnostic;
use wowlua_manifest::model::{
    ChangelogFormat, ChangelogOutput, Dependency, PackageKind, Target, TargetType,
};

/// What to publish.
#[derive(Clone, Debug, Default)]
pub struct PublishRequest {
    pub package: PackageRequest,
    /// `--dry-run`: package, print what each target would receive.
    pub dry_run: bool,
    /// `--tag`: tag `HEAD` (locally) and publish it as a tagged build.
    pub tag: Option<String>,
    /// `--deny-warnings`: W6010 (minified code for an addon site) stops the
    /// publish before any upload.
    pub deny_warnings: bool,
    /// Progress lines: a token read from what `wowlua login` saved (the OS
    /// credential store may wait for the user to allow it).
    pub status: Option<crate::Status>,
}

/// What happened.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub packaged: Option<Packaged>,
    pub outcomes: Vec<Outcome>,
    /// Notes for the summary.
    pub notes: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.outcomes.iter().all(|o| o.ok)
    }
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

/// Packages and uploads the projects.
pub fn publish(
    session: &mut Session,
    projects: &[Project],
    request: &PublishRequest,
) -> Option<Report> {
    let first = projects.first()?;
    let repo = first
        .workspace_root
        .clone()
        .unwrap_or_else(|| first.manifest.root.clone());
    let packager = first.manifest.manifest.compat.packager_keywords;
    let mut report = Report::default();
    // The packager's GitHub Actions guard: a branch push of a tagged commit
    // leaves the publish to the tag's own run.
    if packager
        && std::env::var("GITHUB_ACTIONS").is_ok_and(|v| !v.is_empty())
        && std::env::var("GITHUB_EVENT_NAME").as_deref() == Ok("push")
        && std::env::var("GITHUB_REF").is_ok_and(|r| r.starts_with("refs/heads"))
        && let Some(tag) = git(&repo, &["tag", "--points-at", "HEAD"]).filter(|t| !t.is_empty())
    {
        report.notes.push(format!(
            "found future tag `{}`, not publishing (the tag's run publishes it)",
            tag.lines().next().unwrap_or_default()
        ));
        return Some(report);
    }
    if !packager
        && !request.package.allow_dirty
        && crate::package::dirty(&repo, &crate::package::out_root(first))
    {
        session.push(
            Diagnostic::error(
                "E4336",
                "the working tree has uncommitted changes; a publish needs a clean tree",
            )
            .with_path(&repo)
            .with_help("commit or stash them, or pass `--allow-dirty` (an alpha build)"),
        );
        return None;
    }
    let mut package_request = request.package.clone();
    let mut commit = None;
    if let Some(tag) = &request.tag {
        if !packager {
            let version = first
                .manifest
                .package()
                .map(|p| p.version.clone())
                .unwrap_or_default();
            if tag.strip_prefix('v').unwrap_or(tag) != version {
                session.push(
                    Diagnostic::error(
                        "E4319",
                        format!("`--tag {tag}` does not match the package version `{version}`"),
                    )
                    .with_path(&first.manifest.path)
                    .with_help(format!("tag `{version}` or `v{version}`")),
                );
                return None;
            }
        }
        let head = git(&repo, &["rev-parse", "HEAD"]);
        match git(
            &repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{tag}^{{commit}}"),
            ],
        ) {
            Some(c) if Some(&c) != head.as_ref() => {
                session.push(
                    Diagnostic::error(
                        "E4330",
                        format!("the tag `{tag}` exists and points at another commit ({c:.10})"),
                    )
                    .with_path(&repo),
                );
                return None;
            }
            Some(_) => {}
            None if request.dry_run => {
                report.notes.push(format!(
                    "a dry run does not create the tag `{tag}`; the build shows the untagged version"
                ));
                package_request.tag = Some(tag.clone());
            }
            None => {
                // An annotated tag needs a tagger; CI runners often have none.
                let identity = git(&repo, &["config", "user.email"]).is_some();
                let mut args = vec![];
                if !identity {
                    args.extend([
                        "-c",
                        "user.name=wowlua",
                        "-c",
                        "user.email=wowlua@localhost",
                    ]);
                }
                args.extend(["tag", "-a", tag, "-m", tag]);
                if git(&repo, &args).is_none() {
                    session.push(
                        Diagnostic::plain_error(format!("cannot create the tag `{tag}`"))
                            .with_path(&repo),
                    );
                    return None;
                }
                report.notes.push(format!(
                    "created the tag `{tag}` (local; the GitHub release creates it remotely)"
                ));
            }
        }
        commit = head;
    }
    let packaged = crate::package::package(session, projects, &package_request)?;
    if session.has_errors() {
        return None;
    }
    if minified_site_targets(session, projects, &packaged, &repo) && request.deny_warnings {
        return None;
    }
    let http = http::Http::default();
    let tokens = Tokens::new(&repo, packager, request.status.clone());
    if !request.dry_run {
        archive_maps(&packaged);
    }
    if packager {
        publish_like_the_packager(projects, &packaged, &http, &tokens, request, &mut report);
    } else {
        publish_native(
            session,
            projects,
            &packaged,
            &http,
            &tokens,
            request,
            commit.as_deref(),
            &mut report,
        );
    }
    for o in &report.outcomes {
        for w in &o.warnings {
            session.push(w.clone().with_path(&repo));
        }
        if !o.ok {
            session.push(
                Diagnostic::error(o.code, format!("{}: {}", o.target, o.message)).with_path(&repo),
            );
        }
    }
    report.packaged = Some(packaged);
    Some(report)
}

/// API tokens (Publishing §11.1.3): environment (and the packager's `.env`),
/// then what `wowlua login` saved.
struct Tokens {
    /// The packager's `.env` file, in packager mode.
    dotenv: BTreeMap<String, String>,
    packager: bool,
    status: Option<crate::Status>,
}

impl Tokens {
    fn new(repo: &Path, packager: bool, status: Option<crate::Status>) -> Self {
        let dotenv = if packager {
            [repo.join(".env"), PathBuf::from(".env")]
                .iter()
                .find_map(|p| std::fs::read_to_string(p).ok())
                .map(|t| parse_dotenv(&t))
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        Tokens {
            dotenv,
            packager,
            status,
        }
    }

    /// A variable of the `.env` file or the environment. The packager's
    /// own lowercase variables (`cf_token`, …) come from `.env` only (the
    /// environment's names are case-insensitive on Windows).
    fn var(&self, name: &str) -> Option<String> {
        let environment = name.bytes().all(|b| !b.is_ascii_lowercase());
        self.dotenv
            .get(name)
            .cloned()
            .or_else(|| environment.then(|| std::env::var(name).ok()).flatten())
            .filter(|v| !v.is_empty())
    }

    /// The variables a target type reads, in order.
    fn names(&self, ty: TargetType) -> &'static [&'static str] {
        match (ty, self.packager) {
            (TargetType::Curseforge, true) => &["cf_token", "CF_API_TOKEN", "CF_API_KEY"],
            (TargetType::Curseforge, false) => &["CF_API_TOKEN", "CF_API_KEY"],
            (TargetType::Wago, true) => &["wago_token", "WAGO_API_TOKEN"],
            (TargetType::Wago, false) => &["WAGO_API_TOKEN"],
            (TargetType::Wowinterface, true) => &["wowi_token", "WOWI_API_TOKEN"],
            (TargetType::Wowinterface, false) => &["WOWI_API_TOKEN"],
            (TargetType::Github, true) => &[
                "github_token",
                "GH_API_TOKEN",
                "GITHUB_API_TOKEN",
                "GITHUB_OAUTH",
            ],
            (TargetType::Github, false) => &[
                "GITHUB_TOKEN",
                "GH_API_TOKEN",
                "GITHUB_API_TOKEN",
                "GITHUB_OAUTH",
            ],
            (TargetType::Registry, _) => &["WOWLUA_REGISTRY_TOKEN"],
        }
    }

    /// A target's token and the variable to name when it is missing.
    fn of(&self, target: &Target) -> Result<String, String> {
        if let Some(env) = &target.token_env {
            return self.var(env).ok_or_else(|| env.clone());
        }
        let names = self.names(target.target_type);
        let kind = type_name(target.target_type);
        names
            .iter()
            .find_map(|n| self.var(n))
            .or_else(|| {
                // Said first: the OS credential store may wait for the user
                // without a word (a macOS Keychain dialog, when another
                // wowlua binary saved the token).
                if let Some(status) = &self.status {
                    let store = if crate::credentials::uses_store() {
                        " (the OS credential store may ask you to allow wowlua)"
                    } else {
                        ""
                    };
                    (status.0)(
                        "Reading",
                        &format!("the {kind} token `wowlua login` saved{store}"),
                    );
                }
                crate::credentials::get(kind)
            })
            .ok_or_else(|| {
                names
                    .iter()
                    .find(|n| n.contains("_API_") || **n == "GITHUB_TOKEN")
                    .unwrap_or(&names[0])
                    .to_string()
            })
    }
}

fn type_name(ty: TargetType) -> &'static str {
    match ty {
        TargetType::Curseforge => "curseforge",
        TargetType::Wago => "wago",
        TargetType::Wowinterface => "wowinterface",
        TargetType::Github => "github",
        TargetType::Registry => "registry",
    }
}

/// `KEY=value` lines of a shell `.env` file (`export`, quotes, comments).
fn parse_dotenv(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let value = value.trim();
        let value = if let Some(v) = value.strip_prefix('"').and_then(|v| v.split_once('"')) {
            v.0.to_string()
        } else if let Some(v) = value.strip_prefix('\'').and_then(|v| v.split_once('\'')) {
            v.0.to_string()
        } else {
            value
                .split(" #")
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        };
        out.insert(key.to_string(), value);
    }
    out
}

/// Copies the source maps of the builds to `$WOWLUA_HOME/maps/<build id>/`
/// (step 6), so `wowlua trace` decodes reports of published versions.
fn archive_maps(packaged: &Packaged) {
    let Some(home) = crate::install::wowlua_home() else {
        return;
    };
    for build in packaged.zips.iter().flat_map(|z| &z.builds) {
        let Some(maps) = maps_dir(build) else {
            continue;
        };
        let dest = home
            .join("maps")
            .join(&build.build_id)
            .join(&build.addon_name);
        for e in walkdir::WalkDir::new(&maps).into_iter().flatten() {
            let Ok(rel) = e.path().strip_prefix(&maps) else {
                continue;
            };
            if e.file_type().is_file() {
                let to = dest.join(rel);
                if let Some(p) = to.parent() {
                    let _ = std::fs::create_dir_all(p);
                }
                let _ = std::fs::copy(e.path(), to);
            }
        }
    }
}

/// `<parent>/.maps/<Addon>/` of a build, when it has source maps.
fn maps_dir(build: &crate::build::BuildOutput) -> Option<PathBuf> {
    let dir = build.dir.parent()?.join(".maps").join(&build.addon_name);
    dir.is_dir().then_some(dir)
}

/// Warns (W6010) about addon-site targets that publish minified code; true
/// if any does (decision T170).
fn minified_site_targets(
    session: &mut Session,
    projects: &[Project],
    packaged: &crate::package::Packaged,
    repo: &Path,
) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    for zip in &packaged.zips {
        let project = project_of(projects, zip);
        let Some(publish) = publish_of(projects, zip, repo) else {
            continue;
        };
        for name in &zip.targets {
            let Some(target) = publish.targets.get(name) else {
                continue;
            };
            // Addon sites build unminified unless the target sets a level.
            let Some(level) = target
                .minify
                .as_ref()
                .filter(|_| crate::package::addon_site(target))
            else {
                continue;
            };
            let passes = wowlua_manifest::minify_passes(&project.manifest.manifest, level)
                .unwrap_or_default();
            if !passes.is_empty() && seen.insert(name.clone()) {
                session.push(
                    Diagnostic::warning(
                        "W6010",
                        format!(
                            "target `{name}` publishes minified code (level `{level}`) to an addon site"
                        ),
                    )
                    .with_path(&project.manifest.path)
                    .with_note("Blizzard's UI Add-On Development Policy: \"the programming code of an add-on must in no way be hidden or obfuscated, and must be freely accessible to and viewable by the general public\"; CurseForge, Wago and WoWInterface take readable code only")
                    .with_help("remove the target's `minify` (addon sites build unminified), or publish minified builds to GitHub"),
                );
            }
        }
    }
    !seen.is_empty()
}

/// The project a zip was built from (a bundle's main member).
fn project_of<'a>(projects: &'a [Project], zip: &Zip) -> &'a Project {
    projects
        .iter()
        .find(|p| zip.addons.first() == Some(&p.addon_name()))
        .unwrap_or(&projects[0])
}

/// The publish settings a zip's targets come from: its project's, or the
/// workspace root's for a bundle.
fn publish_of(
    projects: &[Project],
    zip: &Zip,
    repo: &Path,
) -> Option<wowlua_manifest::model::Publish> {
    if zip.bundle {
        return crate::package::bundle_publish(repo).map(|(p, _)| p);
    }
    project_of(projects, zip).manifest.manifest.publish.clone()
}

/// A native changelog (§11.2.1) for a target.
fn native_changelog(
    session: &mut Session,
    project: &Project,
    target: &Target,
    version: &str,
) -> Option<Changelog> {
    let publish = project.manifest.manifest.publish.as_ref();
    let setting = target
        .changelog
        .clone()
        .or_else(|| publish.and_then(|p| p.changelog.clone()));
    let format = target
        .changelog_format
        .or_else(|| publish.and_then(|p| p.changelog_format))
        .unwrap_or(ChangelogFormat::Markdown);
    let root = &project.manifest.root;
    let package_version = project
        .manifest
        .package()
        .map(|p| p.version.clone())
        .unwrap_or_default();
    let text = match setting {
        Some(None) => return None,
        Some(Some(git)) if git == "git" => {
            let repo = project.workspace_root.as_deref().unwrap_or(root);
            return Some(Changelog {
                text: changelog::from_git(repo, &project.addon_name(), version),
                format: ChangelogFormat::Markdown,
            });
        }
        Some(Some(file)) => match std::fs::read_to_string(root.join(&file)) {
            Ok(t) => t,
            Err(e) => {
                session.push(
                    Diagnostic::plain_error(format!("cannot read the changelog `{file}`: {e}"))
                        .with_path(&project.manifest.path),
                );
                return None;
            }
        },
        None => std::fs::read_to_string(root.join("CHANGELOG.md")).ok()?,
    };
    Some(Changelog {
        text: changelog::section(&text, &package_version),
        format,
    })
}

/// CurseForge relations of a native project (§11.4).
fn native_relations(project: &Project, target: &Target, nolib: bool) -> Vec<(String, String)> {
    let m = &project.manifest.manifest;
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |slug: &str, kind: &str| {
        out.retain(|(s, _)| s != slug);
        out.push((slug.to_string(), kind.to_string()));
    };
    for (name, dep) in &m.dependencies {
        let Dependency::Detailed(d) = dep else {
            continue;
        };
        let Some(slug) = &d.curseforge_slug else {
            continue;
        };
        if d.load == Some(false) {
            continue;
        }
        let lua_library = project
            .deps
            .packages
            .iter()
            .any(|p| p.name == *name && p.kind == PackageKind::LuaLibrary);
        let kind = if nolib && lua_library {
            "requiredDependency"
        } else {
            "embeddedLibrary"
        };
        add(slug, kind);
    }
    if let Some(a) = &m.addon_dependencies {
        for (list, kind) in [
            (&a.required, "requiredDependency"),
            (&a.optional, "optionalDependency"),
        ] {
            for d in list {
                if let wowlua_manifest::model::AddonDependency::Detailed(d) = d
                    && let Some(slug) = &d.curseforge_slug
                {
                    add(slug, kind);
                }
            }
        }
    }
    for r in target.relations.iter().flatten() {
        add(&r.slug, &r.relation_type);
    }
    out
}

/// The changelog per site: Wago gets Markdown, WoWInterface BBCode unless
/// `changelog-output: "markdown"`.
fn wago_changelog(c: &Changelog) -> Changelog {
    match c.format {
        ChangelogFormat::Html => Changelog {
            text: changelog::markdown_of_html(&c.text),
            format: ChangelogFormat::Markdown,
        },
        _ => c.clone(),
    }
}

fn wowi_changelog(c: &Changelog, target: &Target) -> String {
    match (
        target.changelog_output.unwrap_or(ChangelogOutput::Bbcode),
        c.format,
    ) {
        (ChangelogOutput::Bbcode, ChangelogFormat::Markdown) => changelog::bbcode(&c.text),
        _ => c.text.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn publish_native(
    session: &mut Session,
    projects: &[Project],
    packaged: &Packaged,
    http: &http::Http,
    tokens: &Tokens,
    request: &PublishRequest,
    commit: Option<&str>,
    report: &mut Report,
) {
    let tagged = packaged.tag.is_some();
    // Upload jobs: one per (target, zip); GitHub one per target.
    struct Job<'a> {
        name: String,
        target: Target,
        token: String,
        zips: Vec<&'a Zip>,
        changelog: Option<Changelog>,
        relations: Vec<(String, String)>,
        archive_maps: bool,
    }
    let mut jobs: Vec<Job> = Vec::new();
    let mut missing_token = false;
    // Targets by their owner (a project, or the bundle) and name: members
    // may name their targets alike.
    let owner = |z: &Zip| (z.bundle, z.addons.first().cloned().unwrap_or_default());
    let mut keys: Vec<((bool, String), String)> = Vec::new();
    for zip in &packaged.zips {
        for name in &zip.targets {
            let key = (owner(zip), name.clone());
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    for (key, name) in &keys {
        let zips: Vec<&Zip> = packaged
            .zips
            .iter()
            .filter(|z| owner(z) == *key && z.targets.contains(name))
            .collect();
        let Some(publish) = publish_of(projects, zips[0], &packaged.repo) else {
            continue;
        };
        let Some(target) = publish.targets.get(name).cloned() else {
            continue;
        };
        let token = match tokens.of(&target) {
            Ok(t) => t,
            Err(var) => {
                session.push(
                    Diagnostic::error(
                        "E4201",
                        format!("no API token for the publish target `{name}`"),
                    )
                    .with_help(format!(
                        "set `{var}`, or run `wowlua login {}`",
                        type_name(target.target_type)
                    )),
                );
                missing_token = true;
                String::new()
            }
        };
        let project = project_of(projects, zips[0]);
        let changelog = native_changelog(session, project, &target, &zips[0].version);
        let archive_maps = publish.archive_maps.unwrap_or(true);
        if target.target_type == TargetType::Github {
            jobs.push(Job {
                name: name.clone(),
                target,
                token,
                zips,
                changelog,
                relations: Vec::new(),
                archive_maps,
            });
        } else {
            for zip in zips {
                // Every AddOn's relations (a bundle has several).
                let mut relations: Vec<(String, String)> = Vec::new();
                for p in projects
                    .iter()
                    .filter(|p| zip.addons.contains(&p.addon_name()))
                {
                    for r in native_relations(p, &target, zip.nolib) {
                        if !relations.contains(&r) {
                            relations.push(r);
                        }
                    }
                }
                jobs.push(Job {
                    name: name.clone(),
                    target: target.clone(),
                    token: token.clone(),
                    zips: vec![zip],
                    relations,
                    changelog: changelog.clone(),
                    archive_maps,
                });
            }
        }
    }
    // Nothing is uploaded while a token is missing; a dry run still shows
    // what the targets would receive.
    if missing_token && !request.dry_run {
        return;
    }
    let title = |z: &Zip| crate::packaging::plain_title(&project_of(projects, z).manifest.title());
    let release = packaged.release;
    let dry_run = request.dry_run;
    let run = |job: &Job| -> Outcome {
        let upload = Upload {
            http,
            name: &job.name,
            target: &job.target,
            token: &job.token,
            zip: job.zips[0],
            release,
            changelog: job.changelog.as_ref(),
            packager: false,
            dry_run,
        };
        match job.target.target_type {
            TargetType::Curseforge => sites::curseforge(&upload, &job.relations),
            TargetType::Wago => {
                let c = job.changelog.as_ref().map(wago_changelog);
                sites::wago(&Upload {
                    changelog: c.as_ref(),
                    ..upload
                })
            }
            TargetType::Wowinterface => {
                let c = job
                    .changelog
                    .as_ref()
                    .map(|c| wowi_changelog(c, &job.target));
                sites::wowinterface(&upload, tagged, c.as_deref())
            }
            TargetType::Github => {
                let Some(tag) = packaged.tag.as_deref() else {
                    let mut o = Outcome::skipped(
                        &job.name,
                        &job.zips[0].file_name,
                        "the build is not on a tag; no GitHub release (pass `--tag`)",
                    );
                    o.warnings = vec![Diagnostic::warning(
                        "W4329",
                        format!(
                            "GitHub target `{}` skipped: the build is not on a tag and no `--tag` was given",
                            job.name
                        ),
                    )];
                    return o;
                };
                let mut extra = Vec::new();
                if job.archive_maps {
                    for zip in &job.zips {
                        if let Some(m) = maps_zip(zip, &packaged.repo) {
                            extra.push(m);
                        }
                    }
                }
                let entries = job.target.release_json.unwrap_or(true).then(|| {
                    job.zips
                        .iter()
                        .map(|z| sites::release_entry(&title(z), &z.version, z))
                        .collect()
                });
                let gh = GithubRelease {
                    tag,
                    commit,
                    name: tag,
                    body: job.changelog.as_ref().map_or("", |c| c.text.as_str()),
                    zips: job.zips.clone(),
                    extra,
                    release_json: entries,
                };
                sites::github(&upload, &gh)
            }
            TargetType::Registry => Outcome::skipped(
                &job.name,
                &job.zips[0].file_name,
                "registry publishing is not available yet",
            ),
        }
    };
    let outcomes: Vec<Outcome> = std::thread::scope(|s| {
        let handles: Vec<_> = jobs.iter().map(|job| s.spawn(|| run(job))).collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Outcome::failed("publish", "", "an upload panicked"))
            })
            .collect()
    });
    report.outcomes.extend(outcomes);
}

/// `<zip name>-maps.zip` of a zip's source maps, for GitHub.
fn maps_zip(zip: &Zip, repo: &Path) -> Option<(String, PathBuf, &'static str)> {
    let folders: Vec<(String, PathBuf)> = zip
        .builds
        .iter()
        .filter_map(|b| Some((b.addon_name.clone(), maps_dir(b)?)))
        .collect();
    if folders.is_empty() {
        return None;
    }
    let name = format!("{}-maps.zip", zip.file_name.trim_end_matches(".zip"));
    let path = zip.path.with_file_name(&name);
    crate::package::write_zip(&path, &folders, &[], crate::package::zip_time(repo)).ok()?;
    Some((name, path, "application/zip"))
}

/// The packager's changelog of a package: its manual changelog (with its
/// markup type, default text) or the `CHANGELOG.md` it generated, read from
/// the built package folder.
fn packager_changelog(
    projects: &[Project],
    packaged: &Packaged,
    zip: &Zip,
) -> Option<(Changelog, PathBuf, bool)> {
    let main = projects
        .iter()
        .find(|p| p.addon_name() == packaged.main)
        .unwrap_or(&projects[0]);
    let publish = main.manifest.manifest.publish.as_ref();
    let dir = &zip
        .builds
        .iter()
        .find(|b| b.addon_name == packaged.main)
        .or(zip.builds.first())?
        .dir;
    let manual = publish
        .and_then(|p| p.changelog.clone().flatten())
        .map(|f| dir.join(f))
        .filter(|p| p.is_file());
    let (path, format, manual) = match manual {
        Some(p) => (
            p,
            publish
                .and_then(|p| p.changelog_format)
                .unwrap_or(ChangelogFormat::Text),
            true,
        ),
        None => (dir.join("CHANGELOG.md"), ChangelogFormat::Markdown, false),
    };
    let text = std::fs::read_to_string(&path).ok()?;
    Some((Changelog { text, format }, path, manual))
}

fn publish_like_the_packager(
    projects: &[Project],
    packaged: &Packaged,
    http: &http::Http,
    tokens: &Tokens,
    request: &PublishRequest,
    report: &mut Report,
) {
    let main = projects
        .iter()
        .find(|p| p.addon_name() == packaged.main)
        .unwrap_or(&projects[0]);
    let Some(targets) = main.manifest.manifest.publish.as_ref().map(|p| &p.targets) else {
        return;
    };
    let tagged = packaged.tag.is_some();
    let info = crate::keywords::KeywordInfo::from_git(&packaged.repo, "", false, false, 0);
    let order = [
        TargetType::Curseforge,
        TargetType::Wowinterface,
        TargetType::Wago,
        TargetType::Github,
    ];
    // One packager run per package (per game), each with its nolib zip.
    let runs: Vec<&Zip> = packaged.zips.iter().filter(|z| !z.nolib).collect();
    for zip in &runs {
        let nolib = packaged
            .zips
            .iter()
            .find(|z| z.nolib && z.interfaces.keys().eq(zip.interfaces.keys()));
        let changelog = packager_changelog(projects, packaged, zip);
        for ty in order {
            for (name, target) in targets
                .iter()
                .filter(|(_, t)| t.target_type == ty && t.enabled)
            {
                if !zip.targets.contains(name) {
                    continue;
                }
                let skipped = |why: String| Outcome::skipped(name, &zip.file_name, &why);
                let token = match tokens.of(target) {
                    Ok(t) => t,
                    Err(var) => {
                        // The packager skips sites it has no token for.
                        report.outcomes.push(skipped(format!("no token (`{var}`)")));
                        continue;
                    }
                };
                let upload = Upload {
                    http,
                    name,
                    target,
                    token: &token,
                    zip,
                    release: packaged.release,
                    changelog: changelog.as_ref().map(|c| &c.0),
                    packager: true,
                    dry_run: request.dry_run,
                };
                let outcome = match ty {
                    TargetType::Curseforge => {
                        // The packager's `retry upload_curseforge`.
                        let relations: Vec<(String, String)> = target
                            .relations
                            .iter()
                            .flatten()
                            .map(|r| (r.slug.clone(), r.relation_type.clone()))
                            .collect();
                        let mut o = sites::curseforge(&upload, &relations);
                        let mut attempt = 1;
                        while !o.ok && !request.dry_run && attempt < 10 && o.code != "E4210" {
                            std::thread::sleep(http.backoff);
                            attempt += 1;
                            o = sites::curseforge(&upload, &relations);
                        }
                        o
                    }
                    TargetType::Wowinterface => {
                        let text = changelog.as_ref().and_then(|(c, path, manual)| {
                            packager_wowi_changelog(c, path, *manual, target, packaged, &info)
                        });
                        sites::wowinterface(&upload, tagged, text.as_deref())
                    }
                    TargetType::Wago => sites::wago(&upload),
                    TargetType::Github => {
                        let Some(tag) = packaged.tag.as_deref() else {
                            report
                                .outcomes
                                .push(skipped("only tagged builds get a GitHub release".into()));
                            continue;
                        };
                        let mut zips = vec![*zip];
                        zips.extend(nolib);
                        let entries = target.release_json.unwrap_or(true).then(|| {
                            zips.iter()
                                .map(|z| sites::release_entry(&packaged.title, &info.version, z))
                                .collect()
                        });
                        let mut extra = Vec::new();
                        if main
                            .manifest
                            .manifest
                            .publish
                            .as_ref()
                            .and_then(|p| p.archive_maps)
                            == Some(true)
                        {
                            extra.extend(maps_zip(zip, &packaged.repo));
                        }
                        let gh = GithubRelease {
                            tag,
                            commit: None,
                            name: tag,
                            body: changelog.as_ref().map_or("", |c| c.0.text.as_str()),
                            zips,
                            extra,
                            release_json: entries,
                        };
                        sites::github(&upload, &gh)
                    }
                    TargetType::Registry => continue,
                };
                let failed_curseforge = ty == TargetType::Curseforge && !outcome.ok;
                report.outcomes.push(outcome);
                if failed_curseforge {
                    // Like the packager: the other uploads are aborted.
                    report
                        .notes
                        .push("uploading to CurseForge failed, aborting the other uploads".into());
                    return;
                }
            }
        }
    }
}

/// What the packager sends WoWInterface: its BBCode of a Markdown manual
/// changelog (with `pandoc`), the manual changelog as it is, its generated
/// BBCode changelog, or (`wowi-convert-changelog: no`) the generated
/// Markdown; nothing with `wowi-create-changelog: no`.
fn packager_wowi_changelog(
    c: &Changelog,
    path: &Path,
    manual: bool,
    target: &Target,
    packaged: &Packaged,
    info: &crate::keywords::KeywordInfo,
) -> Option<String> {
    let convert = target.changelog_output != Some(ChangelogOutput::Markdown);
    let create = target.changelog != Some(None);
    if manual {
        if c.format == ChangelogFormat::Markdown
            && convert
            && let Some(b) = changelog::packager_bbcode(path)
        {
            return Some(b);
        }
        return Some(c.text.clone());
    }
    if !create {
        return None;
    }
    if !convert {
        return Some(c.text.clone());
    }
    let tag = packaged.tag.as_deref()?;
    let token = ["GH_API_TOKEN", "GITHUB_API_TOKEN", "GITHUB_OAUTH"]
        .iter()
        .any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()));
    Some(changelog::packager_wowi_generated(
        &packaged.repo,
        &packaged.title,
        &info.version,
        info.timestamp,
        crate::packaging::previous_tag(&packaged.repo, Some(tag)).as_deref(),
        tag,
        crate::packaging::github_url(&packaged.repo).as_deref(),
        token,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_dotenv_files_like_the_shell() {
        let env = parse_dotenv(
            "# secrets\nexport CF_API_TOKEN=\"abc\"\nWAGO_API_TOKEN='x y'\nwowi_token=plain # note\nbad line\n",
        );
        assert_eq!(env["CF_API_TOKEN"], "abc");
        assert_eq!(env["WAGO_API_TOKEN"], "x y");
        assert_eq!(env["wowi_token"], "plain");
        assert_eq!(env.len(), 3);
    }
}
