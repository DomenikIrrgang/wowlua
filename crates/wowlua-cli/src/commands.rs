//! Command implementations. Each returns `true` on success.

use crate::output::Output;
use crate::{AddArgs, BuildArgs, CheckArg, GlobalArgs, InstallTarget, Kind};
use notify::{EventKind, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use wowlua_diagnostics::Diagnostic;
use wowlua_driver::install::{self, InstallRequest};
use wowlua_driver::scaffold::{self, InitRequest, NewRequest};
use wowlua_driver::{BuildOutput, BuildRequest, LoadOptions, Project, Session};
use wowlua_manifest::model::{CheckMode, PackageKind};

pub(crate) fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub(crate) fn load_options(out: &Output, global: &GlobalArgs) -> LoadOptions {
    let cwd = cwd();
    LoadOptions {
        manifest: global.manifest.as_ref().map(|m| cwd.join(m)),
        cwd,
        package: global.package.clone(),
        locked: global.locked || global.frozen,
        lock_in_memory: false,
        offline: global.offline || global.frozen,
        skip_dependencies: false,
        update: None,
        precise: None,
        refresh: false,
        status: Some(out.status_sink()),
        bundled: false,
        all_members: false,
    }
}

/// Loads the selected projects with the command's features (`--features`,
/// `--no-default-features`; spec Manifest §9.6).
fn load_projects(
    global: &GlobalArgs,
    session: &mut Session,
    options: &LoadOptions,
) -> Option<Vec<Project>> {
    let mut projects = wowlua_driver::load_projects(session, options)?;
    wowlua_driver::select_features(session, &mut projects, &global.feature_selection())
        .then_some(projects)
}

/// `load_projects` for commands that work on one project.
pub(crate) fn load_project(
    global: &GlobalArgs,
    session: &mut Session,
    options: &LoadOptions,
) -> Option<Project> {
    let project = wowlua_driver::load_project(session, options)?;
    let mut projects = [project];
    wowlua_driver::select_features(session, &mut projects, &global.feature_selection()).then(|| {
        let [project] = projects;
        project
    })
}

/// Applies `check.lints` from the manifest to the collected diagnostics.
fn apply_lints(session: &mut Session, project: &Project) {
    project.apply_lints(session);
}

pub(crate) fn relative(path: &Path) -> String {
    let cwd = cwd();
    path.strip_prefix(&cwd)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn describe(build: &BuildOutput) -> String {
    format!(
        "{} {} ({}: {})",
        build.addon_name,
        build.version,
        build.profile,
        build.flavors.join(", ")
    )
}

/// Loads the selected projects (every member of a workspace) and builds
/// the addons among them; with `write: false`, libraries are checked too.
/// Diagnostics stay in the session.
fn load_and_build_all(
    out: &Output,
    session: &mut Session,
    global: &GlobalArgs,
    request: &BuildRequest,
) -> Option<Built> {
    let options = load_options(out, global);
    let projects = load_projects(global, session, &options)?;
    let single = projects.len() == 1;
    let whole = whole_workspace(global, &projects);
    let mut builds = Vec::new();
    for project in projects {
        let buildable = match project.kind() {
            PackageKind::Addon => true,
            PackageKind::Library => !request.write,
            PackageKind::LuaLibrary | PackageKind::Types if !request.write => {
                // Checked as an addon that uses it checks it; its license
                // is the consumers' concern (W4306).
                let Some(consumer) = wowlua_driver::checking_consumer(session, &options, &project)
                else {
                    continue;
                };
                let build = wowlua_driver::build(session, &consumer, request);
                session.diagnostics.retain(|d| d.code != Some("W4306"));
                apply_lints(session, &project);
                if let Some(b) = build {
                    builds.push((project, b));
                }
                continue;
            }
            _ => false,
        };
        if !buildable && !single {
            continue;
        }
        let build = wowlua_driver::build(session, &project, request);
        apply_lints(session, &project);
        if let Some(b) = build {
            builds.push((project, b));
        }
    }
    // The workspace's external addons, built with the whole workspace.
    let mut externals = Vec::new();
    if whole
        && request.write
        && let Some((first, _)) = builds.first()
        && let Ok(profile) = first.manifest.profile(&request.profile)
    {
        let out_root = first.manifest.out_dir();
        for ext in &first.external_addons {
            if let Some(b) =
                wowlua_driver::external::build_external(session, ext, &profile, &out_root, true)
            {
                externals.push(b);
            }
        }
    }
    (!session.has_errors()).then_some(Built { builds, externals })
}

/// What `load_and_build_all` built.
struct Built {
    builds: Vec<(Project, BuildOutput)>,
    /// The workspace's external addons (installed with the first member's
    /// client folders).
    externals: Vec<BuildOutput>,
}

/// Whether the command works on the whole workspace: no `-p`, run at the
/// workspace root.
fn whole_workspace(global: &GlobalArgs, projects: &[Project]) -> bool {
    if global.package.is_some() {
        return false;
    }
    let cwd = cwd();
    let manifest = global.manifest.as_ref().map(|m| cwd.join(m));
    let found = wowlua_manifest::find_manifest(&cwd, manifest.as_deref())
        .and_then(|m| m.parent().map(Path::to_path_buf));
    projects
        .first()
        .and_then(|p| p.workspace_root.clone())
        .is_some_and(|root| found.is_some_and(|f| f == root))
}

/// Loads the project and builds it. Diagnostics stay in the session.
fn load_and_build(
    out: &Output,
    session: &mut Session,
    global: &GlobalArgs,
    request: &BuildRequest,
    cache: Option<&mut wowlua_driver::BuildCache>,
) -> Option<(Project, BuildOutput)> {
    let project = load_project(global, session, &load_options(out, global))?;
    let build = match cache {
        Some(cache) => wowlua_driver::build_with_cache(session, &project, request, cache),
        None => wowlua_driver::build(session, &project, request),
    };
    apply_lints(session, &project);
    build.map(|b| (project, b))
}

/// `wowlua build`; with `--watch`, again after every change (spec CLI
/// §12.2).
pub fn build(out: &Output, global: &GlobalArgs, args: &BuildArgs) -> bool {
    let (ok, watched) = build_once(out, global, args);
    if !args.watch {
        return ok;
    }
    // The folder of the manifest the command found (a workspace's root
    // covers its members), else the first project's.
    let found = wowlua_manifest::find_manifest(&cwd(), global.manifest.as_deref())
        .and_then(|m| m.parent().map(Path::to_path_buf));
    let Some(root) = found.or_else(|| watched.as_ref().map(|(root, _)| root.clone())) else {
        return ok;
    };
    // What the build writes is not a change: its output folder, the one
    // `--out` names and the source maps beside that (as the watcher names
    // them: symbolic links resolved).
    let out_dirs = |dist: PathBuf| {
        let mut dirs = vec![dist];
        if let Some(o) = &args.out {
            let o = cwd().join(o);
            dirs.extend(o.parent().map(|p| p.join(".maps")));
            dirs.push(o);
        }
        let real: Vec<PathBuf> = dirs.iter().filter_map(|d| d.canonicalize().ok()).collect();
        dirs.extend(real);
        dirs
    };
    let dist = watched.map_or_else(|| root.join("dist"), |(_, dist)| dist);
    watch(out, &root, out_dirs(dist), |dirs| {
        if let (_, Some((_, dist))) = build_once(out, global, args) {
            *dirs = out_dirs(dist);
        }
        true
    })
}

/// One `wowlua build`: whether it succeeded, and the first project's root
/// and output folder.
fn build_once(
    out: &Output,
    global: &GlobalArgs,
    args: &BuildArgs,
) -> (bool, Option<(PathBuf, PathBuf)>) {
    let started = Instant::now();
    let mut session = Session::new();
    let request = BuildRequest {
        profile: args.profile.name("dev"),
        flavors: (!args.flavor.is_empty()).then(|| args.flavor.clone()),
        out_dir: args.out.as_ref().map(|o| cwd().join(o)),
        write: true,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: global.jobs(),
        keep_code: true,
        tests: false,
        left_out: Vec::new(),
        reproducible: args.reproducible,
    };
    let result = load_and_build_all(out, &mut session, global, &request);
    let ok = out.flush(&mut session);
    match result {
        Some(built) if ok => {
            let watched = built
                .builds
                .first()
                .map(|(p, _)| (p.root().to_path_buf(), p.manifest.out_dir()));
            let all = built.builds.iter().map(|(_, b)| b).chain(&built.externals);
            for build in all {
                out.status(
                    "Finished",
                    format!(
                        "{} in {:.2}s → {}",
                        describe(build),
                        started.elapsed().as_secs_f64(),
                        relative(&build.dir)
                    ),
                );
                if args.stats {
                    let s = wowlua_driver::build::stats(build);
                    let count = |n: usize, what: &str| {
                        format!("{n} {what}{}", if n == 1 { "" } else { "s" })
                    };
                    out.status(
                        "Stats",
                        format!(
                            "{} ({} KiB), {} ({} KiB), compiled by Lua 5.1 in {:.1} ms",
                            count(s.files, "file"),
                            s.bytes.div_ceil(1024),
                            count(s.lua_files, "Lua file"),
                            s.lua_bytes.div_ceil(1024),
                            s.compile.as_secs_f64() * 1000.0
                        ),
                    );
                }
            }
            (true, watched)
        }
        _ => (false, None),
    }
}

/// `wowlua package`: builds every publish target and packs its zips.
pub fn package(
    out: &Output,
    global: &GlobalArgs,
    targets: Vec<String>,
    flavors: Vec<String>,
) -> bool {
    let started = Instant::now();
    let mut session = Session::new();
    let options = LoadOptions {
        bundled: true,
        all_members: false,
        ..load_options(out, global)
    };
    let Some(projects) = load_projects(global, &mut session, &options) else {
        out.flush(&mut session);
        return false;
    };
    let request = wowlua_driver::package::PackageRequest {
        targets,
        flavors: (!flavors.is_empty()).then_some(flavors),
        ..Default::default()
    };
    let packaged = wowlua_driver::package::package(&mut session, &projects, &request);
    for project in &projects {
        apply_lints(&mut session, project);
    }
    let ok = out.flush(&mut session);
    match packaged {
        Some(p) if ok => {
            if p.zips.is_empty() {
                out.status(
                    "Note",
                    "no publish targets: add one under `publish.targets` (see `wowlua explain` or the manual)",
                );
            }
            for zip in &p.zips {
                out.status(
                    "Packaged",
                    format!(
                        "{} ({}{}; {}) → {}",
                        zip.file_name,
                        p.release.as_str(),
                        if zip.nolib { ", nolib" } else { "" },
                        zip.targets.join(", "),
                        relative(&zip.path)
                    ),
                );
            }
            out.status(
                "Finished",
                format!("packaging in {:.2}s", started.elapsed().as_secs_f64()),
            );
            true
        }
        _ => false,
    }
}

/// `wowlua package --registry`: the registry archive of each selected
/// library package (spec Package manager §10.10.3).
pub fn package_registry(out: &Output, global: &GlobalArgs) -> bool {
    let started = Instant::now();
    let mut session = Session::new();
    let Some(projects) = load_projects(global, &mut session, &load_options(out, global)) else {
        out.flush(&mut session);
        return false;
    };
    let archives: Vec<_> = projects
        .iter()
        .filter(|p| p.selected)
        .filter_map(|p| wowlua_driver::publish::registry::archive(&mut session, p))
        .collect();
    let ok = out.flush(&mut session);
    if !ok {
        return false;
    }
    for a in &archives {
        out.status(
            "Packaged",
            format!(
                "{} (registry; {} files, {}) → {}",
                a.file_name,
                a.files.len(),
                a.checksum,
                relative(&a.path)
            ),
        );
    }
    out.status(
        "Finished",
        format!("packaging in {:.2}s", started.elapsed().as_secs_f64()),
    );
    true
}

pub fn publish(out: &Output, global: &GlobalArgs, args: crate::PublishArgs) -> bool {
    use wowlua_manifest::model::ReleaseType;
    let started = Instant::now();
    let mut session = Session::new();
    // A publish never changes the lockfile (§11.1.1): an addon's must be
    // up to date; a library's is not committed, so it resolves without
    // writing one.
    let mut options = load_options(out, global);
    let library = library_manifest(&options);
    if library {
        options.lock_in_memory = true;
    } else {
        options.locked = true;
    }
    options.bundled = true;
    let Some(projects) = load_projects(global, &mut session, &options) else {
        out.flush(&mut session);
        return false;
    };
    // Step 3 (§11.9): `wowlua test` on the same packages, with the lockfile
    // as the publish has it.
    let tests = (!args.no_test).then(|| {
        let mut test_global = global.clone();
        test_global.locked |= !library;
        let flavor = args.flavor.clone();
        wowlua_driver::publish::Tests(std::rc::Rc::new(move || {
            let out = Output::new(&test_global);
            let test_args = crate::TestArgs {
                filter: Vec::new(),
                flavor: flavor.clone(),
                update_snapshots: false,
                coverage: false,
                coverage_per_flavor: false,
                reporter: Vec::new(),
                output: None,
                watch: false,
                profile_out: None,
                game_data: false,
            };
            test_once(&out, &test_global, &test_args).0
        }))
    });
    let request = wowlua_driver::publish::PublishRequest {
        package: wowlua_driver::package::PackageRequest {
            targets: args.targets,
            flavors: (!args.flavor.is_empty()).then_some(args.flavor),
            release_type: args.release_type.map(|r| match r {
                crate::ReleaseTypeArg::Alpha => ReleaseType::Alpha,
                crate::ReleaseTypeArg::Beta => ReleaseType::Beta,
                crate::ReleaseTypeArg::Release => ReleaseType::Release,
            }),
            allow_dirty: args.allow_dirty,
            tag: None,
        },
        dry_run: args.dry_run,
        tag: args.tag,
        deny_warnings: global.deny_warnings,
        status: Some(out.status_sink()),
        tests,
    };
    let report = wowlua_driver::publish::publish(&mut session, &projects, &request);
    for project in &projects {
        apply_lints(&mut session, project);
    }
    // The "Packaged" lines were said as soon as the zips were ready.
    let Some(report) = report else {
        out.flush(&mut session);
        return false;
    };
    for o in &report.outcomes {
        let (label, text) = match (o.ok, o.skipped, args.dry_run) {
            (true, true, _) => (
                "Skipped",
                format!("{} ({}): {}", o.target, o.file, o.message),
            ),
            (true, false, true) => (
                "Dry run",
                format!("{} ({}): {}", o.target, o.file, o.message),
            ),
            (true, false, false) => (
                "Uploaded",
                format!("{} → {}: {}", o.file, o.target, o.message),
            ),
            (false, _, _) => ("Failed", format!("{} ({})", o.target, o.file)),
        };
        out.status(label, text);
    }
    for note in &report.notes {
        out.status("Note", note);
    }
    let ok = out.flush(&mut session) && report.ok();
    if ok {
        out.status(
            "Finished",
            format!("publishing in {:.2}s", started.elapsed().as_secs_f64()),
        );
    }
    ok
}

/// Whether the command's manifest is a `library`, `lua-library` or `types`
/// package's.
fn library_manifest(options: &LoadOptions) -> bool {
    let Some(path) = wowlua_manifest::find_manifest(&options.cwd, options.manifest.as_deref())
    else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    matches!(
        value.pointer("/package/kind").and_then(|k| k.as_str()),
        Some("library" | "lua-library" | "types")
    )
}

/// Prints one diagnostic.
fn report(out: &Output, diagnostic: Diagnostic) {
    let mut session = Session::new();
    session.push(diagnostic);
    out.flush(&mut session);
}

pub fn login(out: &Output, site: crate::Site, token: Option<String>, no_verify: bool) -> bool {
    use std::io::IsTerminal;
    use wowlua_driver::credentials::{self, Saved, Verified};
    let token = match token {
        Some(t) => t,
        None if std::io::stdin().is_terminal() => {
            match rpassword::prompt_password(format!("Paste the {} API token: ", site.title())) {
                Ok(t) => t,
                Err(e) => {
                    report(
                        out,
                        Diagnostic::plain_error(format!("cannot read the token: {e}")),
                    );
                    return false;
                }
            }
        }
        None => {
            let mut line = String::new();
            if let Err(e) = std::io::stdin().read_line(&mut line) {
                report(
                    out,
                    Diagnostic::plain_error(format!("cannot read the token: {e}")),
                );
                return false;
            }
            line
        }
    };
    let token = token.trim();
    if token.is_empty() {
        report(out, Diagnostic::plain_error("no token given"));
        return false;
    }
    if !no_verify {
        match credentials::verify(site.id(), token) {
            Verified::Accepted => out.status("Verified", format!("the {} token", site.title())),
            Verified::Unchecked(why) => out.status("Note", format!("not checked: {why}")),
            Verified::Rejected(why) => {
                report(
                    out,
                    Diagnostic::plain_error(format!("{}: {why}; not saved", site.title()))
                        .with_help("check the token, or save it anyway with `--no-verify`"),
                );
                return false;
            }
        }
    }
    match credentials::set(site.id(), token) {
        Ok(Saved::CredentialStore) => {
            out.status(
                "Saved",
                format!("the {} token in the OS credential store", site.title()),
            );
            true
        }
        Ok(Saved::ConfigNoTerminal) => {
            out.status(
                "Saved",
                format!(
                    "the {} token in config.json (readable only by you): without a terminal, wowlua does not use the OS credential store",
                    site.title()
                ),
            );
            true
        }
        Ok(Saved::Config) => {
            report(
                out,
                Diagnostic::warning(
                    "W4326",
                    "no OS credential store; the token is saved in `config.json` (readable only by you)",
                ),
            );
            out.status(
                "Saved",
                format!("the {} token in config.json", site.title()),
            );
            true
        }
        Err(e) => {
            report(
                out,
                Diagnostic::plain_error(format!("cannot save the token: {e}")),
            );
            false
        }
    }
}

pub fn logout(out: &Output, site: crate::Site) -> bool {
    match wowlua_driver::credentials::remove(site.id()) {
        Ok(true) => {
            out.status("Removed", format!("the {} token", site.title()));
            true
        }
        Ok(false) => {
            out.status("Note", format!("no saved {} token", site.title()));
            true
        }
        Err(e) => {
            report(
                out,
                Diagnostic::plain_error(format!("cannot remove the token: {e}")),
            );
            false
        }
    }
}

/// `wowlua yank <name>@<version> [--undo]` (spec CLI §12.4).
pub fn yank(out: &Output, package: &str, undo: bool, registry: Option<&str>) -> bool {
    use wowlua_driver::publish::registry as reg;
    let Some((name, version)) = package
        .split_once('@')
        .filter(|(n, v)| !n.is_empty() && semver::Version::parse(v).is_ok())
    else {
        report(
            out,
            Diagnostic::plain_error(format!("`{package}` is not `<name>@<version>`"))
                .with_help("for example `wowlua yank lib-tooltip@1.2.3`"),
        );
        return false;
    };
    let registry = reg::chosen_registry(registry);
    let status = out.status_sink();
    let http = wowlua_driver::publish::http::Http::default();
    let done = reg::token(&registry, Some(&status))
        .and_then(|t| reg::yank(&http, &registry, &t, name, version, undo));
    match done {
        Ok(()) => {
            out.status(
                if undo { "Unyanked" } else { "Yanked" },
                format!("{name} {version} ({registry})"),
            );
            true
        }
        Err(d) => {
            report(out, d);
            false
        }
    }
}

/// `wowlua owner list|add|remove <name> [logins]`.
pub fn owner(out: &Output, action: crate::OwnerAction) -> bool {
    use wowlua_driver::publish::registry::{self as reg, OwnerChange};
    let (change, name, logins, registry) = match action {
        crate::OwnerAction::List { name, registry } => {
            (OwnerChange::List, name, Vec::new(), registry)
        }
        crate::OwnerAction::Add {
            name,
            logins,
            registry,
        } => (OwnerChange::Add, name, logins, registry),
        crate::OwnerAction::Remove {
            name,
            logins,
            registry,
        } => (OwnerChange::Remove, name, logins, registry),
    };
    let registry = reg::chosen_registry(registry.as_deref());
    let http = wowlua_driver::publish::http::Http::default();
    let token = match change {
        OwnerChange::List => None,
        _ => match reg::token(&registry, Some(&out.status_sink())) {
            Ok(t) => Some(t),
            Err(d) => {
                report(out, d);
                return false;
            }
        },
    };
    match reg::owners(&http, &registry, token.as_deref(), &name, change, &logins) {
        Ok(owners) => {
            if change == OwnerChange::List {
                let mut text = String::new();
                for o in &owners {
                    text.push_str(o);
                    text.push('\n');
                }
                out.print(text);
            } else {
                out.status("Owners", format!("{name}: {}", owners.join(", ")));
            }
            true
        }
        Err(d) => {
            report(out, d);
            false
        }
    }
}

/// `wowlua search <words>`.
pub fn search(
    out: &Output,
    words: &str,
    page: usize,
    limit: usize,
    registry: Option<&str>,
) -> bool {
    use wowlua_driver::publish::registry as reg;
    let registry = reg::chosen_registry(registry);
    let http = wowlua_driver::publish::http::Http::default();
    let found = match reg::search(&http, &registry, words, page.max(1), limit.clamp(1, 100)) {
        Ok(f) => f,
        Err(d) => {
            report(out, d);
            return false;
        }
    };
    if found.packages.is_empty() {
        out.status(
            "Found",
            if found.total == 0 {
                "no packages match".to_string()
            } else {
                format!("{} packages, none on page {}", found.total, found.page)
            },
        );
        return true;
    }
    let wn = found
        .packages
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(0);
    let wv = found
        .packages
        .iter()
        .map(|p| p.vers.len())
        .max()
        .unwrap_or(0);
    let mut text = String::new();
    for p in &found.packages {
        text.push_str(format!("{:wn$}  {:wv$}  {}", p.name, p.vers, p.description).trim_end());
        text.push('\n');
    }
    out.print(text);
    let first = (found.page - 1) * found.per_page + 1;
    let last = first + found.packages.len() - 1;
    let more = if last < found.total {
        format!("; `--page {}` for more", found.page + 1)
    } else {
        String::new()
    };
    out.status(
        "Found",
        format!(
            "{} package{}, {first}–{last} shown{more}",
            found.total,
            if found.total == 1 { "" } else { "s" }
        ),
    );
    true
}

pub fn toc(out: &Output, global: &GlobalArgs, profile: &str, flavors: Vec<String>) -> bool {
    let mut session = Session::new();
    let request = BuildRequest {
        profile: profile.into(),
        flavors: (!flavors.is_empty()).then_some(flavors),
        out_dir: None,
        write: false,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: global.jobs(),
        keep_code: true,
        tests: false,
        left_out: Vec::new(),
        reproducible: false,
    };
    let result = load_and_build(out, &mut session, global, &request, None);
    let ok = out.flush(&mut session);
    let Some((_, build)) = result.filter(|_| ok) else {
        return false;
    };
    let several = build.tocs.len() > 1;
    for toc in &build.tocs {
        if several {
            out.print(format!("==> {} <==\n", toc.name));
        }
        out.print(&toc.content);
        if several {
            out.print("\n");
        }
    }
    true
}

/// `wowlua check`: the build pipeline without writing; with `--watch`,
/// again after every change (spec CLI §12.2).
pub fn check(
    out: &Output,
    global: &GlobalArgs,
    profile: &str,
    flavors: Vec<String>,
    watching: bool,
) -> bool {
    let (ok, watched) = check_once(out, global, profile, &flavors);
    if !watching {
        return ok;
    }
    // The folder of the manifest the command found (a workspace's root
    // covers its members), else the first project's.
    let found = wowlua_manifest::find_manifest(&cwd(), global.manifest.as_deref())
        .and_then(|m| m.parent().map(Path::to_path_buf));
    let Some(root) = found.or_else(|| watched.as_ref().map(|(root, _)| root.clone())) else {
        return ok;
    };
    let out_dir = watched.map_or_else(|| root.join("dist"), |(_, out_dir)| out_dir);
    watch(out, &root, vec![out_dir], |out_dirs| {
        if let (_, Some((_, dir))) = check_once(out, global, profile, &flavors) {
            *out_dirs = vec![dir];
        }
        true
    })
}

/// One `wowlua check` run: whether it passed, and the first project's root
/// and output folder.
fn check_once(
    out: &Output,
    global: &GlobalArgs,
    profile: &str,
    flavors: &[String],
) -> (bool, Option<(PathBuf, PathBuf)>) {
    let started = Instant::now();
    let mut session = Session::new();
    let request = BuildRequest {
        profile: profile.into(),
        flavors: (!flavors.is_empty()).then(|| flavors.to_vec()),
        out_dir: None,
        write: false,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: global.jobs(),
        keep_code: false,
        // The test side too (test files, mocks, helpers), as `wowlua test`
        // compiles it.
        tests: true,
        left_out: Vec::new(),
        reproducible: false,
    };
    let result = load_and_build_all(out, &mut session, global, &request);
    let ok = out.flush(&mut session);
    match result {
        Some(built) if ok => {
            for (_, build) in &built.builds {
                out.status(
                    "Checked",
                    format!(
                        "{} in {:.2}s",
                        describe(build),
                        started.elapsed().as_secs_f64()
                    ),
                );
            }
            let watched = built
                .builds
                .first()
                .map(|(p, _)| (p.root().to_path_buf(), p.manifest.out_dir()));
            (true, watched)
        }
        _ => (false, None),
    }
}

/// `wowlua test` (spec Tooling §16.1): each addon's test files, once per
/// flavor, in the emulated client; reports and coverage (§16.1.5, §16.1.6);
/// with `--watch`, again after every change.
pub fn test(out: &Output, global: &GlobalArgs, args: &crate::TestArgs) -> bool {
    let (ok, watched) = test_once(out, global, args);
    if !args.watch {
        return ok;
    }
    let Some((root, out_dir)) = watched else {
        return ok;
    };
    watch(out, &root, vec![out_dir], |_| {
        test_once(out, global, args);
        true
    })
}

/// The folder a status line shows: relative to the current directory, `.`
/// for the current directory itself.
fn shown_folder(path: &Path) -> String {
    let shown = relative(path);
    if shown.is_empty() { ".".into() } else { shown }
}

/// Watches `root` and calls `run` once per batch of changes (until 300 ms
/// pass without one): changes saved while it runs make one more run, not
/// one each. Changes in the output folders (which `run` may change), in
/// `.git` and in temporary files are not changes, nor are files read (inotify
/// reports opening one) or a change reported again (the file is as the last
/// run saw it). Runs until `run` returns false; false when the folder cannot
/// be watched.
fn watch(
    out: &Output,
    root: &Path,
    mut out_dirs: Vec<PathBuf>,
    mut run: impl FnMut(&mut Vec<PathBuf>) -> bool,
) -> bool {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(w) => w,
        Err(error) => {
            let mut session = Session::new();
            session.push(Diagnostic::plain_error(format!(
                "cannot watch files: {error}"
            )));
            out.flush(&mut session);
            return false;
        }
    };
    if let Err(error) = watcher.watch(root, RecursiveMode::Recursive) {
        let mut session = Session::new();
        session.push(Diagnostic::plain_error(format!(
            "cannot watch `{}`: {error}",
            root.display()
        )));
        out.flush(&mut session);
        return false;
    }
    out.status(
        "Watching",
        format!("{} for changes (Ctrl+C to stop)", shown_folder(root)),
    );
    let changes = |result: notify::Result<notify::Event>, into: &mut Vec<PathBuf>| {
        let Ok(event) = result else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        into.extend(event.paths);
    };
    // Each changed path as the last run saw it: modification time and size,
    // `None` when gone. The watcher can report one save twice (under load,
    // in two batches), and the second report must not run again.
    let mut seen: HashMap<PathBuf, Option<(Option<SystemTime>, u64)>> = HashMap::new();
    while let Ok(first) = rx.recv() {
        let mut paths = Vec::new();
        changes(first, &mut paths);
        // The rest of the batch, with what arrived during the last run.
        while let Ok(result) = rx.recv_timeout(Duration::from_millis(300)) {
            changes(result, &mut paths);
        }
        paths.sort();
        paths.dedup();
        let mut changed = false;
        for path in paths {
            // (a folder that holds an output folder changes with it)
            if out_dirs
                .iter()
                .any(|d| path.starts_with(d) || d.starts_with(&path))
                || path.components().any(|c| c.as_os_str() == ".git")
                || path.to_string_lossy().contains(".wowlua-tmp")
                // (the build's previous output, moved aside)
                || path.to_string_lossy().contains(".wowlua-old")
            {
                continue;
            }
            let now = std::fs::metadata(&path)
                .ok()
                .map(|m| (m.modified().ok(), m.len()));
            // (without modification times, every report is a change)
            let unknown = now.is_some_and(|(modified, _)| modified.is_none());
            changed |= seen.insert(path, now) != Some(now) || unknown;
        }
        if changed && !run(&mut out_dirs) {
            break;
        }
    }
    true
}

/// One `wowlua test` run: whether it passed, and the package root and
/// output folder to watch.
fn test_once(
    out: &Output,
    global: &GlobalArgs,
    args: &crate::TestArgs,
) -> (bool, Option<(PathBuf, PathBuf)>) {
    use termcolor::Color;
    let started = Instant::now();
    let filters = args.filter.as_slice();
    let mut session = Session::new();
    let Some(projects) = load_projects(global, &mut session, &load_options(out, global)) else {
        out.flush(&mut session);
        return (false, None);
    };
    let watched = projects
        .first()
        .map(|p| (p.root().to_path_buf(), p.manifest.out_dir()));
    let wanted: Vec<String> = args
        .flavor
        .iter()
        .filter_map(|f| wowlua_flavors::table().resolve(f).map(|f| f.id.clone()))
        .collect();
    // Test files at once: `--jobs` (or `WOWLUA_JOBS`, `build.jobs`) when
    // given, else one a core (each is a whole emulated client).
    let jobs = global.jobs();
    let threads = if jobs.explicit {
        jobs.threads.get()
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    };
    let mut ok = true;
    let (mut passed, mut failed, mut skipped, mut files) = (0, 0, 0, 0);
    let mut snaps = wowlua_test::SnapshotSummary::default();
    let mut all: Vec<wowlua_test::FileResult> = Vec::new();
    // Coverage by flavor ("" merged), the manifest's thresholds and
    // exclusions, the report folder and the root.
    let mut coverage: std::collections::BTreeMap<String, wowlua_test::coverage::Report> =
        std::collections::BTreeMap::new();
    let mut settings: Option<wowlua_manifest::model::CoverageSettings> = None;
    let mut reporters: Vec<String> = args.reporter.clone();
    let mut output: Option<PathBuf> = args.output.as_ref().map(|o| cwd().join(o));
    // The addons' roots (a report's paths are relative to their package)
    // and the first one's name, as the reports' title: in a workspace, the
    // root package's, not the last member's.
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut title: Option<String> = None;
    for project in projects {
        // A library's tests run against it built as an addon.
        if !matches!(project.kind(), PackageKind::Addon | PackageKind::Library) {
            continue;
        }
        let test_settings = project.manifest.manifest.test.clone();
        if reporters.is_empty() {
            reporters = test_settings.reporters.clone();
        }
        if settings.is_none() {
            settings = test_settings.coverage.clone();
        }
        if output.is_none() {
            output = Some(project.manifest.out_dir().join("test"));
        }
        roots.push(project.root().to_path_buf());
        title.get_or_insert_with(|| project.addon_name());
        let selected: Vec<String> = project
            .manifest
            .flavors
            .iter()
            .filter(|f| wanted.is_empty() || wanted.contains(f))
            .cloned()
            .collect();
        // A shared-mode package is one build for all its flavors (its
        // flavor guards need them all); split mode builds each.
        let shared_mode = project.manifest.manifest.build.flavor_mode
            == wowlua_manifest::model::FlavorMode::Shared;
        let mut shared: Option<Option<(BuildOutput, Vec<String>)>> = None;
        for flavor in selected.clone() {
            let owned: Option<(BuildOutput, Vec<String>)>;
            let (build, left_out): (&BuildOutput, &[String]) = if shared_mode {
                if shared.is_none() {
                    // All the package's flavors: shared code compiles for them all.
                    let all = project.manifest.flavors.clone();
                    shared = Some(test_build(
                        out,
                        global,
                        &mut session,
                        &project,
                        all,
                        "shared",
                    ));
                }
                match shared.as_ref().and_then(Option::as_ref) {
                    Some((b, l)) => (b, l.as_slice()),
                    None => {
                        ok = false;
                        continue;
                    }
                }
            } else {
                owned = test_build(
                    out,
                    global,
                    &mut session,
                    &project,
                    vec![flavor.clone()],
                    &flavor,
                );
                match &owned {
                    Some((b, l)) => (b, l.as_slice()),
                    None => {
                        ok = false;
                        continue;
                    }
                }
            };
            // A flavor without test files (`build.flavor-files` keeps them to
            // other flavors) runs none and needs no UI code.
            let has_tests = build
                .tests
                .iter()
                .any(|t| t.role == wowlua_driver::TestRole::Test);
            let blizzard_ui = if test_settings.blizzard_ui && has_tests {
                match wowlua_test::ui_source(&flavor, &|what| out.status("Fetching", what)) {
                    Ok(ui) => Some(ui),
                    Err(e) => {
                        out.colored(&[
                            (Some(Color::Red), true, "error"),
                            (None, false, &format!(": test.blizzard-ui: {e}\n")),
                        ]);
                        ok = false;
                        continue;
                    }
                }
            } else {
                None
            };
            let start_profile: &(dyn Fn(&mut wowlua_vm::Vm) + Sync) = &|vm: &mut wowlua_vm::Vm| {
                let start = Instant::now();
                vm.start_profiling(Box::new(move || start.elapsed().as_nanos() as u64));
            };
            let mut suite = make_suite(
                &project,
                build,
                &flavor,
                blizzard_ui,
                SuiteSwitches {
                    update_snapshots: args.update_snapshots,
                    coverage: args.coverage,
                    game_data: args.game_data,
                },
                args.profile_out.as_ref().map(|_| start_profile),
            );
            // `--game-data`: the game data `wowlua data pull` read, else the
            // installation's.
            if args.game_data {
                let (camera, pulled) = crate::data::with_pulled(suite.camera.take(), &flavor);
                suite.camera = camera;
                if !pulled {
                    out.status(
                        "Note",
                        format!("no game data pulled for {flavor} (`wowlua data pull`): reading the installation's"),
                    );
                }
            }
            if let Some(dir) = &args.profile_out {
                let dir = cwd().join(dir);
                let mapper = wowlua_test::chunk_mapper(&suite);
                let flavor = flavor.clone();
                suite.after_file = Some(std::sync::Arc::new(
                    move |vm: &mut wowlua_vm::Vm, rel: &str| {
                        let Some(p) = vm.stop_profiling() else { return };
                        let map = |chunk: &[u8]| mapper(&String::from_utf8_lossy(chunk));
                        let json = wowlua_debug::profile::speedscope(
                            &p,
                            &map,
                            &format!("{rel} ({flavor})"),
                        );
                        let name =
                            format!("{}.{flavor}.speedscope.json", rel.replace(['/', '\\'], "_"));
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::fs::write(
                            dir.join(name),
                            serde_json::to_vec(&json).unwrap_or_default(),
                        );
                    },
                ));
            }
            let mut results = wowlua_test::run(&suite, filters, threads);
            results.extend(not_compiling(out, left_out, filters, &flavor));
            for result in &results {
                if let Some(c) = &result.coverage {
                    let key = if args.coverage_per_flavor {
                        flavor.clone()
                    } else {
                        String::new()
                    };
                    coverage.entry(key).or_default().merge(c);
                }
            }
            for result in results {
                files += 1;
                snaps.written += result.snapshots.written;
                snaps.updated += result.snapshots.updated;
                snaps.removed += result.snapshots.removed;
                let bad = result.failed();
                if bad {
                    ok = false;
                }
                for t in &result.tests {
                    match &t.outcome {
                        wowlua_test::Outcome::Pass => passed += 1,
                        wowlua_test::Outcome::Skip(_) => skipped += 1,
                        wowlua_test::Outcome::Fail { .. } => failed += 1,
                    }
                }
                if reporters.is_empty() || reporters.iter().any(|r| r == "pretty") {
                    print_result(out, &result);
                }
                all.push(result);
            }
        }
    }
    let output = output.unwrap_or_else(|| cwd().join("dist").join("test"));
    // Report files.
    let write = |name: &str, text: &str| -> bool {
        let path = output.join(name);
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, text));
        match written {
            Ok(()) => {
                out.status("Wrote", path.display().to_string());
                true
            }
            Err(e) => {
                out.colored(&[
                    (Some(Color::Red), true, "error"),
                    (None, false, &format!(": writing {}: {e}\n", path.display())),
                ]);
                false
            }
        }
    };
    for r in &reporters {
        let done = match r.as_str() {
            "pretty" | "lcov" | "cobertura" | "coverage-json" | "html" => true,
            "junit" => write("junit.xml", &wowlua_test::reports::junit(&all)),
            "tap" => write("results.tap", &wowlua_test::reports::tap(&all)),
            "json" => write("results.json", &wowlua_test::reports::tests_json(&all)),
            "github" => {
                print!("{}", wowlua_test::reports::github(&all));
                true
            }
            other => {
                out.colored(&[
                    (Some(Color::Red), true, "error"),
                    (None, false, &format!(": unknown reporter `{other}`\n")),
                ]);
                false
            }
        };
        ok &= done;
    }
    // Coverage: its exclusions, reports and thresholds.
    if args.coverage {
        let exclude = wowlua_driver::glob_set(
            &settings
                .as_ref()
                .map(|s| s.exclude.clone())
                .unwrap_or_default(),
        );
        for (flavor, report) in &mut coverage {
            report.files.retain(|path, _| !exclude.is_match(path));
            let prefix = if flavor.is_empty() {
                String::new()
            } else {
                format!("{flavor}/")
            };
            out.colored(&[(None, false, &wowlua_test::reports::summary(report))]);
            for r in &reporters {
                let done = match r.as_str() {
                    "lcov" => write(
                        &format!("{prefix}lcov.info"),
                        &wowlua_test::reports::lcov(report),
                    ),
                    "cobertura" => write(
                        &format!("{prefix}cobertura.xml"),
                        &wowlua_test::reports::cobertura(report),
                    ),
                    "coverage-json" => write(
                        &format!("{prefix}coverage.json"),
                        &wowlua_test::reports::coverage_json(report),
                    ),
                    "html" => write(
                        &format!("{prefix}coverage.html"),
                        &wowlua_test::reports::html(
                            report,
                            &roots,
                            title.as_deref().unwrap_or("wowlua"),
                        ),
                    ),
                    _ => true,
                };
                ok &= done;
            }
            // (a run with filters checks part of the suite: its totals still
            // count every source line, so the thresholds are the whole
            // suite's alone)
            if let Some(s) = settings.as_ref().filter(|_| filters.is_empty()) {
                for below in wowlua_test::runs::below_minimums(report, s) {
                    let wowlua_test::runs::Below {
                        kind,
                        percent: p,
                        minimum: min,
                    } = below;
                    ok = false;
                    out.colored(&[
                        (Some(Color::Red), true, "error"),
                        (
                            None,
                            false,
                            &format!(": {kind} coverage {p:.1}% is below the minimum {min}%\n"),
                        ),
                    ]);
                }
            }
        }
    }
    let mut summary = format!(
        "{passed} passed, {failed} failed, {skipped} skipped ({files} file{}) in {:.2}s",
        if files == 1 { "" } else { "s" },
        started.elapsed().as_secs_f64()
    );
    let parts: Vec<String> = [
        (snaps.written, "written"),
        (snaps.updated, "updated"),
        (snaps.removed, "removed"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if !parts.is_empty() {
        summary.push_str(&format!("; snapshots: {}", parts.join(", ")));
    }
    if ok {
        out.status("Tested", summary);
    } else {
        out.colored(&[
            (Some(Color::Red), true, "error"),
            (None, false, &format!(": tests failed: {summary}\n")),
        ]);
    }
    (ok, watched)
}

/// How a suite runs: `-u`, `--coverage`, `--game-data`.
use wowlua_test::runs::Switches as SuiteSwitches;

/// The test runner's suite for a test build of `project`, with the
/// renderer's camera.
fn make_suite<'a>(
    project: &Project,
    build: &'a BuildOutput,
    flavor: &str,
    blizzard_ui: Option<wowlua_test::UiSource>,
    switches: SuiteSwitches,
    instrument: Option<&'a (dyn Fn(&mut wowlua_vm::Vm) + Sync)>,
) -> wowlua_test::Suite<'a> {
    let camera = crate::screenshot::camera(project, build, flavor);
    wowlua_test::runs::suite(
        project,
        build,
        flavor,
        blizzard_ui,
        switches,
        instrument,
        camera,
    )
}

/// Builds the package's test build for `flavor` (its first flavor by
/// default) and runs `f` with its suite (a debug session's, `wowlua
/// screenshot`'s); an error when it does not build.
pub fn with_suite(
    out: &Output,
    global: &GlobalArgs,
    flavor: Option<&str>,
    instrument: Option<&(dyn Fn(&mut wowlua_vm::Vm) + Sync)>,
    f: &mut dyn FnMut(&mut wowlua_test::Suite),
) -> Result<(), String> {
    let mut session = Session::new();
    let Some(projects) = load_projects(global, &mut session, &load_options(out, global)) else {
        out.flush(&mut session);
        return Err("the package does not load (its errors are above)".into());
    };
    let Some(project) = projects
        .into_iter()
        .find(|p| p.kind() == PackageKind::Addon)
    else {
        return Err("no addon package here".into());
    };
    let flavor = match flavor {
        Some(f) => wowlua_flavors::table()
            .resolve(f)
            .map(|f| f.id.clone())
            .ok_or_else(|| format!("unknown flavor {f}"))?,
        None => project
            .manifest
            .flavors
            .first()
            .cloned()
            .ok_or("the package has no flavors")?,
    };
    let shared_mode =
        project.manifest.manifest.build.flavor_mode == wowlua_manifest::model::FlavorMode::Shared;
    let (flavors, dir) = if shared_mode {
        (project.manifest.flavors.clone(), "shared".to_string())
    } else {
        (vec![flavor.clone()], flavor.clone())
    };
    let Some((build, _)) = test_build(out, global, &mut session, &project, flavors, &dir) else {
        return Err("the package does not build (its errors are above)".into());
    };
    let blizzard_ui = if project.manifest.manifest.test.blizzard_ui {
        Some(wowlua_test::ui_source(&flavor, &|what| {
            out.status("Fetching", what)
        })?)
    } else {
        None
    };
    let mut suite = make_suite(
        &project,
        &build,
        &flavor,
        blizzard_ui,
        // a debug session and `wowlua screenshot` show the game as the
        // installation has it
        SuiteSwitches {
            game_data: true,
            ..SuiteSwitches::default()
        },
        instrument,
    );
    f(&mut suite);
    Ok(())
}

/// The package root of the current directory's addon; the errors of a
/// package that does not load are shown.
pub fn package_root(out: &Output, global: &GlobalArgs) -> Option<PathBuf> {
    let mut session = Session::new();
    let Some(projects) = load_projects(global, &mut session, &load_options(out, global)) else {
        out.flush(&mut session);
        return None;
    };
    projects
        .into_iter()
        .find(|p| p.kind() == PackageKind::Addon)
        .map(|p| p.root().to_path_buf())
}

/// A test build of `project` for `flavors` in `<out>/.test/<dir>/<Name>`
/// (its diagnostics shown); `None` when it fails. A test file that does
/// not compile, when the errors are all in test files, is left out and the
/// rest built again (its errors shown): the build and the files left out.
fn test_build(
    out: &Output,
    global: &GlobalArgs,
    session: &mut Session,
    project: &Project,
    flavors: Vec<String>,
    dir: &str,
) -> Option<(BuildOutput, Vec<String>)> {
    test_build_as(out, global, session, project, flavors, dir, "test")
}

/// [`test_build`] with another profile (`wowlua bench`'s builds).
fn test_build_as(
    out: &Output,
    global: &GlobalArgs,
    session: &mut Session,
    project: &Project,
    flavors: Vec<String>,
    dir: &str,
    profile: &str,
) -> Option<(BuildOutput, Vec<String>)> {
    let built = wowlua_test::runs::build(
        session,
        project,
        flavors,
        dir,
        profile,
        global.jobs(),
        &mut |session| out.show(session),
    );
    if !out.flush(session) {
        return None;
    }
    built
}

/// The test files left out of a run because they do not compile: a failed
/// file each the run selects, a note for each other.
fn not_compiling(
    out: &Output,
    left_out: &[String],
    filters: &[String],
    flavor: &str,
) -> Vec<wowlua_test::FileResult> {
    let (results, skipped) = wowlua_test::runs::not_compiling(left_out, filters, flavor);
    for shown in skipped {
        out.status("Skipped", format!("{shown}, which does not compile"));
    }
    results
}

/// One test file's results, as the `pretty` reporter shows them.
fn print_result(out: &Output, result: &wowlua_test::FileResult) {
    use termcolor::Color;
    let bad = result.failed();
    let (word, color) = if bad {
        ("FAIL", Color::Red)
    } else {
        ("PASS", Color::Green)
    };
    out.colored(&[
        (Some(color), true, &format!(" {word} ")),
        (None, true, &result.file),
        (
            None,
            false,
            &format!(" ({}, {:.2}s)\n", result.flavor, result.seconds),
        ),
    ]);
    if let Some(error) = &result.error {
        out.colored(&[(Some(Color::Red), false, &indent(error, "     "))]);
    }
    for t in &result.tests {
        match &t.outcome {
            wowlua_test::Outcome::Pass => {
                out.colored(&[
                    (Some(Color::Green), false, "   ✓ "),
                    (None, false, &format!("{}\n", t.name)),
                ]);
            }
            wowlua_test::Outcome::Skip(reason) => {
                let why = if reason.is_empty() {
                    String::new()
                } else {
                    format!(" ({reason})")
                };
                out.colored(&[
                    (Some(Color::Yellow), false, "   - "),
                    (None, false, &format!("{}{why}\n", t.name)),
                ]);
            }
            wowlua_test::Outcome::Fail { message, stack } => {
                let (message, at) = wowlua_test::split_failure(message, stack);
                out.colored(&[
                    (Some(Color::Red), true, "   ✗ "),
                    (None, true, &format!("{}\n", t.name)),
                    (Some(Color::Red), false, &indent(&message, "       ")),
                ]);
                if let Some(at) = at {
                    out.colored(&[(None, false, &format!("       at {at}\n"))]);
                }
            }
        }
    }
    if bad && !result.printed.is_empty() {
        out.colored(&[
            (None, false, "     printed:\n"),
            (None, false, &indent(result.printed.trim_end(), "       ")),
        ]);
    }
}

/// Every line of `text` indented, newline-terminated.
fn indent(text: &str, by: &str) -> String {
    text.lines().map(|l| format!("{by}{l}\n")).collect()
}

/// `wowlua compile <files…>`: the build of only these files, in the
/// project's settings when run inside one, else the defaults (rooted at
/// the files' common folder). Writes the Lua outputs: to stdout when there
/// is one, else into `--out`, printing the load order.
pub fn compile(out: &Output, global: &GlobalArgs, args: &crate::CompileArgs) -> bool {
    let started = Instant::now();
    let mut session = Session::new();
    let cwd = cwd();
    let mut files = Vec::new();
    for file in &args.files {
        match cwd.join(file).canonicalize() {
            Ok(path) => files.push(path),
            Err(error) => session.push(Diagnostic::plain_error(format!(
                "cannot read `{}`: {error}",
                file.display()
            ))),
        }
    }
    if !out.flush(&mut session) {
        return false;
    }
    let options = load_options(out, global);
    let in_project =
        wowlua_manifest::find_manifest(&options.cwd, options.manifest.as_deref()).is_some();
    let project = if in_project {
        load_project(global, &mut session, &options)
    } else {
        let root = common_folder(&files);
        wowlua_driver::standalone_project(&mut session, &options, &root)
    };
    let Some(project) = project else {
        out.flush(&mut session);
        return false;
    };
    let request = BuildRequest {
        profile: args.profile.name("dev"),
        flavors: (!args.flavor.is_empty()).then(|| args.flavor.clone()),
        out_dir: None,
        write: false,
        nolib: false,
        minify: args.minify.clone(),
        bundle: (args.bundle || args.no_bundle).then_some(args.bundle),
        only: Some(wowlua_driver::Only {
            files,
            project_declarations: in_project,
        }),
        jobs: global.jobs(),
        keep_code: true,
        tests: false,
        left_out: Vec::new(),
        reproducible: false,
    };
    let build = wowlua_driver::build(&mut session, &project, &request);
    apply_lints(&mut session, &project);
    let ok = out.flush(&mut session);
    let Some(build) = build.filter(|_| ok) else {
        return false;
    };
    let lua = wowlua_driver::build::lua_outputs(&build);
    let order: Vec<&String> = lua.iter().map(|(path, _)| path).collect();
    let content = |path: &str| -> Option<Vec<u8>> {
        lua.iter().find(|(p, _)| p == path).map(|(_, b)| b.clone())
    };
    let target = args.out.as_ref().map(|o| cwd.join(o));
    let to_file = |path: &Path, bytes: &[u8]| -> bool {
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, bytes));
        if let Err(error) = written {
            session_error(out, format!("cannot write `{}`: {error}", relative(path)));
            return false;
        }
        true
    };
    match (order.as_slice(), &target) {
        ([], _) => {
            out.status("Compiled", "nothing (no Lua output)");
            true
        }
        ([one], None) => {
            let bytes = content(one).unwrap_or_default();
            out.print(String::from_utf8_lossy(&bytes));
            true
        }
        ([one], Some(path)) if !path.is_dir() && !names_folder(args.out.as_deref()) => {
            if !to_file(path, &content(one).unwrap_or_default()) {
                return false;
            }
            out.status(
                "Compiled",
                format!(
                    "in {:.2}s → {}",
                    started.elapsed().as_secs_f64(),
                    relative(path)
                ),
            );
            true
        }
        (_, None) => {
            session_error(
                out,
                format!(
                    "the compilation has {} output files; pass `-o <dir>` to write them",
                    order.len()
                ),
            );
            false
        }
        (_, Some(dir)) => {
            for path in &order {
                if !to_file(&dir.join(path), &content(path).unwrap_or_default()) {
                    return false;
                }
            }
            out.status(
                "Compiled",
                format!(
                    "{} files in {:.2}s → {}",
                    order.len(),
                    started.elapsed().as_secs_f64(),
                    relative(dir)
                ),
            );
            out.print("Load order:\n");
            for path in &order {
                out.print(format!("  {path}\n"));
            }
            true
        }
    }
}

/// `-o dir/`: a trailing separator names a folder.
fn names_folder(path: Option<&Path>) -> bool {
    path.is_some_and(|p| {
        let s = p.to_string_lossy();
        s.ends_with('/') || s.ends_with('\\')
    })
}

/// The deepest folder containing every file.
fn common_folder(files: &[PathBuf]) -> PathBuf {
    let mut common: Option<PathBuf> = None;
    for file in files {
        let parent = file.parent().map(Path::to_path_buf).unwrap_or_default();
        common = Some(match common {
            None => parent,
            Some(c) => {
                let mut shared = PathBuf::new();
                for (a, b) in c.components().zip(parent.components()) {
                    if a != b {
                        break;
                    }
                    shared.push(a);
                }
                shared
            }
        });
    }
    common.unwrap_or_else(cwd)
}

fn session_error(out: &Output, message: String) {
    let mut session = Session::new();
    session.push(Diagnostic::plain_error(message));
    out.flush(&mut session);
}

/// `wowlua fmt [--check] [paths…]`.
pub fn fmt(out: &Output, global: &GlobalArgs, check: bool, paths: Vec<PathBuf>) -> bool {
    let mut session = Session::new();
    let Some(projects) = load_projects(
        global,
        &mut session,
        &LoadOptions {
            skip_dependencies: true,
            ..load_options(out, global)
        },
    ) else {
        out.flush(&mut session);
        return false;
    };
    let cwd = cwd();
    let request = wowlua_driver::format::FormatRequest {
        paths: paths.iter().map(|p| cwd.join(p)).collect(),
        check,
    };
    let mut report = wowlua_driver::format::FormatReport::default();
    for project in &projects {
        let r = wowlua_driver::format::format(&mut session, project, &request);
        report.changed.extend(r.changed);
        report.unchanged += r.unchanged;
        report.skipped += r.skipped;
    }
    let ok = out.flush(&mut session);
    for file in &report.changed {
        if check {
            out.print(format!("would reformat {file}\n"));
        } else {
            out.status("Formatted", file);
        }
    }
    let total = report.changed.len() + report.unchanged;
    let skipped = if report.skipped > 0 {
        format!(", {} skipped", report.skipped)
    } else {
        String::new()
    };
    let verb = if check { "would change" } else { "changed" };
    out.status(
        "Finished",
        format!(
            "{} of {} {verb}{skipped}",
            report.changed.len(),
            plural(total, "file", "files")
        ),
    );
    ok && !(check && !report.changed.is_empty())
}

fn install_request(target: &InstallTarget, link: bool) -> InstallRequest {
    InstallRequest {
        wow: target.wow.as_ref().map(|w| cwd().join(w)),
        clients: (!target.client.is_empty()).then(|| target.client.clone()),
        flavors: (!target.flavor.is_empty()).then(|| target.flavor.clone()),
        link,
        force: target.force,
    }
}

fn report_installed(out: &Output, installed: &[install::Installed]) {
    for item in installed {
        let verb = if item.linked { "Linked" } else { "Installed" };
        out.status(verb, item.path.display().to_string());
    }
}

pub fn install(out: &Output, global: &GlobalArgs, profile: &str, target: &InstallTarget) -> bool {
    let mut session = Session::new();
    let request = BuildRequest {
        profile: profile.into(),
        flavors: (!target.flavor.is_empty()).then(|| target.flavor.clone()),
        out_dir: None,
        write: true,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: global.jobs(),
        keep_code: true,
        tests: false,
        left_out: Vec::new(),
        reproducible: false,
    };
    let Some(built) = load_and_build_all(out, &mut session, global, &request) else {
        out.flush(&mut session);
        return false;
    };
    if !out.flush(&mut session) {
        return false;
    }
    let mut ok = true;
    let pairs = built.builds.iter().map(|(p, b)| (p, b)).chain(
        built
            .builds
            .first()
            .map(|(p, _)| p)
            .into_iter()
            .flat_map(|p| built.externals.iter().map(move |b| (p, b))),
    );
    for (project, build) in pairs {
        out.status("Built", describe(build));
        let installed = install::install(
            &mut session,
            project,
            build,
            &install_request(target, false),
        );
        ok &= out.flush(&mut session);
        report_installed(out, &installed);
        if installed.is_empty() && ok {
            out.status("Skipped", "no client folder to install into");
        }
    }
    ok
}

pub fn uninstall(
    out: &Output,
    global: &GlobalArgs,
    client: Vec<String>,
    wow: Option<PathBuf>,
) -> bool {
    let mut session = Session::new();
    let Some(projects) = load_projects(
        global,
        &mut session,
        &LoadOptions {
            skip_dependencies: true,
            ..load_options(out, global)
        },
    ) else {
        out.flush(&mut session);
        return false;
    };
    let request = InstallRequest {
        wow: wow.map(|w| cwd().join(w)),
        clients: (!client.is_empty()).then_some(client),
        ..InstallRequest::default()
    };
    let mut ok = true;
    let whole = whole_workspace(global, &projects);
    let mut names: Vec<(&Project, String, Option<Vec<String>>)> = projects
        .iter()
        .filter(|p| p.kind() == PackageKind::Addon)
        .map(|p| (p, p.addon_name(), None))
        .collect();
    if whole && let Some(first) = projects.first() {
        for ext in &first.external_addons {
            names.push((first, ext.name.clone(), Some(ext.flavors.clone())));
        }
    }
    for (project, name, flavors) in names {
        let request = InstallRequest {
            flavors: flavors.or_else(|| request.flavors.clone()),
            ..request.clone()
        };
        let removed = install::uninstall(&mut session, project, &name, &request);
        ok &= out.flush(&mut session);
        for path in &removed {
            out.status("Removed", path.display().to_string());
        }
        if removed.is_empty() && ok {
            out.status("Nothing", format!("no installed build of {name} found"));
        }
    }
    ok
}

pub fn clean(out: &Output, global: &GlobalArgs) -> bool {
    let mut session = Session::new();
    let Some(projects) = load_projects(
        global,
        &mut session,
        &LoadOptions {
            skip_dependencies: true,
            ..load_options(out, global)
        },
    ) else {
        out.flush(&mut session);
        return false;
    };
    out.flush(&mut session);
    // Workspace members share one output folder.
    let mut dirs: Vec<PathBuf> = projects
        .iter()
        .map(|p| wowlua_pkg::source::normalize(&p.manifest.out_dir()))
        .collect();
    dirs.sort();
    dirs.dedup();
    let mut ok = true;
    for dir in dirs.iter().filter(|d| d.exists()) {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => out.status("Removed", relative(dir)),
            Err(error) => {
                session.push(Diagnostic::plain_error(format!(
                    "cannot remove `{}`: {error}",
                    dir.display()
                )));
                out.flush(&mut session);
                ok = false;
            }
        }
    }
    ok
}

pub fn explain(out: &Output, code: &str) -> bool {
    // The plain-Lua warning form of a name or type error (`W2001`) is
    // explained as that error (appendix B.2).
    let (info, intro) = match (
        wowlua_diagnostics::lookup(code),
        wowlua_diagnostics::plain_lua_error(code),
    ) {
        (Some(info), _) => (info, String::new()),
        (None, Some(error)) => {
            let warning = code.to_ascii_uppercase();
            (
                error,
                format!(
                    "{warning} (warning): the plain-Lua warning form of {e}\n\n\
                     In a file of plain Lua (no wowlua syntax and no `--!strict`), {e}\n\
                     is reported as the warning {warning}. It means the same; set its\n\
                     level in `check.lints` as \"{warning}\".\n\n",
                    e = error.code
                ),
            )
        }
        (None, None) => {
            let mut session = Session::new();
            session.push(Diagnostic::plain_error(format!(
                "unknown diagnostic code `{code}`"
            )));
            out.flush(&mut session);
            return false;
        }
    };
    let kind = match info.code.as_bytes()[0] {
        b'E' => "error",
        b'W' => "warning",
        _ => "hint",
    };
    let mut text = format!(
        "{intro}{} ({kind}): {}\n",
        info.code,
        info.summary.replace('`', "")
    );
    if let Some(e) = wowlua_diagnostics::explanation(info.code) {
        text.push_str(&format!("\n{}\n", e.text));
        for example in &e.examples {
            let verdict = if example.compiles {
                "Compiles"
            } else {
                "Reports it"
            };
            text.push_str(&format!("\n{verdict}:\n"));
            for file in &example.files {
                if example.files.len() > 1 || file.path != "src/main.wowlua" {
                    text.push_str(&format!("  {}\n", file.path));
                }
                for line in file.text.lines() {
                    text.push_str(format!("    {line}").trim_end());
                    text.push('\n');
                }
            }
        }
        if !e.related.is_empty() {
            text.push_str(&format!("\nRelated: {}\n", e.related.join(", ")));
        }
    }
    text.push_str(&format!(
        "\nhttps://wowlua.dev/docs/reference/diagnostics/{}\n",
        info.code
    ));
    out.print(text);
    true
}

/// `wowlua new`'s arguments.
pub struct NewArgs {
    pub path: PathBuf,
    pub kind: Kind,
    pub name: Option<String>,
    pub flavors: Vec<String>,
    pub ci_github: bool,
    pub check: Option<CheckArg>,
    pub git: bool,
    pub world_profile: bool,
}

pub fn new(out: &Output, args: NewArgs) -> bool {
    let NewArgs {
        path,
        kind,
        name,
        flavors,
        ci_github,
        check,
        git,
        world_profile,
    } = args;
    let kind = match kind {
        Kind::Addon => PackageKind::Addon,
        Kind::Library => PackageKind::Library,
        Kind::LuaLibrary => PackageKind::LuaLibrary,
        Kind::Types => PackageKind::Types,
    };
    let check = check.map(|c| match c {
        CheckArg::Nocheck => CheckMode::Nocheck,
        CheckArg::Basic => CheckMode::Basic,
        CheckArg::Strict => CheckMode::Strict,
    });
    let request = NewRequest {
        path: cwd().join(&path),
        kind,
        name,
        flavors,
        ci_github,
        check,
        git,
        world_profile,
    };
    match scaffold::new_package(&request) {
        Ok(_) => {
            out.status(
                "Created",
                format!("{} package in {}", kind_name(kind), path.display()),
            );
            true
        }
        Err(diagnostic) => {
            let mut session = Session::new();
            session.push(diagnostic);
            out.flush(&mut session);
            false
        }
    }
}

pub(crate) fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn kind_name(kind: PackageKind) -> &'static str {
    match kind {
        PackageKind::Addon => "addon",
        PackageKind::Library => "library",
        PackageKind::LuaLibrary => "lua-library",
        PackageKind::Types => "types",
    }
}

pub fn init(
    out: &Output,
    from_toc: Option<PathBuf>,
    from_pkgmeta: Option<String>,
    no_pkgmeta: bool,
) -> bool {
    let mut session = Session::new();
    let pkgmeta_file = from_pkgmeta
        .as_ref()
        .filter(|p| !p.is_empty())
        .map(|p| cwd().join(p));
    let request = InitRequest {
        dir: cwd(),
        from_toc: from_toc.map(|t| cwd().join(t)),
        from_pkgmeta: from_pkgmeta.is_some(),
        pkgmeta_file,
        no_pkgmeta,
        inherited: None,
        addon_name: None,
        neighbors: Vec::new(),
    };
    match scaffold::init_package(&request) {
        Ok(report) => {
            session.diagnostics.extend(report.warnings.clone());
            out.flush(&mut session);
            out.status(
                "Adopted",
                format!(
                    "{} for {} ({} moved into src/, {} updated)",
                    report.addon_name,
                    report.flavors.join(", "),
                    plural(report.moved, "file", "files"),
                    plural(report.rewritten_paths, "in-game path", "in-game paths")
                ),
            );
            if report.dependencies > 0 {
                out.status(
                    "Converted",
                    format!(
                        "{} from .pkgmeta",
                        plural(report.dependencies, "external", "externals")
                    ),
                );
            }
            for member in &report.members {
                out.status("Adopted", format!("{member} as a workspace member"));
            }
            for (name, _) in &report.external_addons {
                out.status("Adopted", format!("{name} as an external addon"));
            }
            let old = if report.old_tocs.is_empty() {
                "TOC files of the members".to_string()
            } else {
                report.old_tocs.join(", ")
            };
            out.status(
                "Note",
                format!(
                    "TOC files are generated from wowlua.json now; the old {old} can be deleted"
                ),
            );
            // Packager workflows keep working; how to switch them (T159).
            for (file, pandoc) in wowlua_driver::pkgmeta::packager_workflows(&cwd()) {
                let mut steps = String::from(
                    "      - uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main\n",
                );
                if pandoc {
                    steps.push_str("      - run: sudo apt-get install -y pandoc\n");
                }
                steps.push_str("      - run: wowlua publish\n        env: # the packager step's env (CF_API_TOKEN, WOWI_API_TOKEN, WAGO_API_TOKEN, GITHUB_OAUTH)");
                out.status(
                    "Note",
                    format!(
                        ".github/workflows/{file} publishes with the BigWigs packager, which keeps working; \
                         to publish with wowlua, replace its packager steps with one\n{steps}\n\
                         (runs with `-d`, which upload nothing, become `wowlua package`)"
                    ),
                );
            }
            true
        }
        Err(diagnostic) => {
            session.push(diagnostic);
            out.flush(&mut session);
            false
        }
    }
}

pub fn dev(
    out: &Output,
    global: &GlobalArgs,
    target: &InstallTarget,
    link: bool,
    exit_after: Option<usize>,
) -> bool {
    let request = BuildRequest {
        profile: "dev".into(),
        flavors: (!target.flavor.is_empty()).then(|| target.flavor.clone()),
        out_dir: None,
        write: true,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: global.jobs(),
        keep_code: true,
        tests: false,
        left_out: Vec::new(),
        reproducible: false,
    };
    let install = install_request(target, link);
    // What one build compiled, for the next ones.
    let mut cache = wowlua_driver::BuildCache::default();

    // The project's root and output folder, after a build that succeeded.
    let mut cycle = |first: bool| -> Option<(PathBuf, PathBuf)> {
        let started = Instant::now();
        let mut session = Session::new();
        let result = load_and_build(out, &mut session, global, &request, Some(&mut cache));
        let ok = out.flush(&mut session);
        let (project, build) = result.filter(|_| ok)?;
        out.status(
            "Built",
            format!(
                "{} in {:.2}s",
                describe(&build),
                started.elapsed().as_secs_f64()
            ),
        );
        // With --link, later builds update the linked folder in place.
        if first || !link {
            let installed = install::install(&mut session, &project, &build, &install);
            out.flush(&mut session);
            report_installed(out, &installed);
        }
        out.status("Ready", "type /reload in game to load the changes");
        Some((project.root().to_path_buf(), project.manifest.out_dir()))
    };

    let first = cycle(true);
    let mut builds = 1;
    let root = first.as_ref().map(|(root, _)| root.clone()).or_else(|| {
        wowlua_manifest::find_manifest(&cwd(), global.manifest.as_deref())
            .and_then(|m| m.parent().map(Path::to_path_buf))
    });
    let Some(root) = root else { return false };
    if exit_after.is_some_and(|n| builds >= n) {
        return first.is_some();
    }
    // The build writes here: its own writes are not changes. Until a build
    // succeeds, the manifest is not known to be readable.
    let out_dir = first.map_or_else(|| root.join("dist"), |(_, out_dir)| out_dir);

    watch(out, &root, vec![out_dir], |out_dirs| {
        if let Some((_, dir)) = cycle(false) {
            *out_dirs = vec![dir];
        }
        builds += 1;
        exit_after.is_none_or(|n| builds < n)
    })
}

/// Whether `wowlua lint` shows a diagnostic: lints, and errors that stop
/// the analysis (syntax, manifest, I/O).
fn shown_by_lint(d: &Diagnostic) -> bool {
    d.is_lint()
        || (d.is_error()
            && d.code
                .is_none_or(|c| c.starts_with("E0") || c.starts_with("E4")))
}

/// Applies machine-applicable fixes until none is left (at most 10 rounds,
/// as a fix can make another one possible). `lints_only`: only fixes of
/// lint diagnostics. `None` when the project does not load.
fn apply_fixes(out: &Output, global: &GlobalArgs, lints_only: bool) -> Option<usize> {
    let request = BuildRequest {
        profile: "dev".into(),
        flavors: None,
        out_dir: None,
        write: false,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: global.jobs(),
        keep_code: true,
        tests: false,
        left_out: Vec::new(),
        reproducible: false,
    };
    let mut total = 0;
    let mut files: std::collections::BTreeSet<PathBuf> = Default::default();
    for _ in 0..10 {
        let mut session = Session::new();
        let Some(project) = load_project(global, &mut session, &load_options(out, global)) else {
            out.flush(&mut session);
            return None;
        };
        let _ = wowlua_driver::build(&mut session, &project, &request);
        apply_lints(&mut session, &project);
        let diagnostics = session.take_diagnostics().into_vec();
        let with_fixes: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|d| !d.fixes.is_empty() && (!lints_only || d.is_lint()))
            .collect();
        match wowlua_driver::fix::apply(&session.db, &with_fixes, project.root()) {
            Ok(applied) if applied.fixes > 0 => {
                total += applied.fixes;
                files.extend(applied.files);
            }
            Ok(_) => break,
            Err(e) => {
                session.push(Diagnostic::plain_error(format!("cannot apply fixes: {e}")));
                out.flush(&mut session);
                return None;
            }
        }
    }
    if total > 0 {
        let s = if total == 1 { "" } else { "s" };
        let f = if files.len() == 1 { "" } else { "s" };
        out.status(
            "Fixed",
            format!("{total} problem{s} in {} file{f}", files.len()),
        );
    }
    Some(total)
}

/// `wowlua lint [--fix]`.
pub fn lint(out: &Output, global: &GlobalArgs, fix: bool) -> bool {
    if fix && apply_fixes(out, global, true).is_none() {
        return false;
    }
    let started = Instant::now();
    let mut session = Session::new();
    let request = BuildRequest {
        profile: "dev".into(),
        flavors: None,
        out_dir: None,
        write: false,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: global.jobs(),
        keep_code: true,
        tests: false,
        left_out: Vec::new(),
        reproducible: false,
    };
    let Some(project) = load_project(global, &mut session, &load_options(out, global)) else {
        out.flush(&mut session);
        return false;
    };
    let build = wowlua_driver::build(&mut session, &project, &request);
    apply_lints(&mut session, &project);
    session.diagnostics.retain(shown_by_lint);
    let ok = out.flush(&mut session);
    if ok {
        let name = build.map_or_else(String::new, |b| format!("{} ", describe(&b)));
        out.status(
            "Linted",
            format!("{name}in {:.2}s", started.elapsed().as_secs_f64()),
        );
    }
    ok
}

/// `wowlua fix [--manifest]`.
pub fn fix(out: &Output, global: &GlobalArgs, migrate: bool) -> bool {
    if migrate {
        let mut session = Session::new();
        let Some(project) = load_project(
            global,
            &mut session,
            &LoadOptions {
                skip_dependencies: true,
                ..load_options(out, global)
            },
        ) else {
            out.flush(&mut session);
            return false;
        };
        // There is one manifest schema so far: nothing to migrate.
        out.status(
            "Fresh",
            format!(
                "{} uses the current schema",
                relative(&project.manifest.path)
            ),
        );
        return out.flush(&mut session);
    }
    let Some(total) = apply_fixes(out, global, false) else {
        return false;
    };
    if total == 0 {
        out.status("Fresh", "nothing to fix");
    }
    // What is left.
    let mut session = Session::new();
    let request = BuildRequest {
        profile: "dev".into(),
        flavors: None,
        out_dir: None,
        write: false,
        nolib: false,
        minify: None,
        bundle: None,
        only: None,
        jobs: global.jobs(),
        keep_code: true,
        tests: false,
        left_out: Vec::new(),
        reproducible: false,
    };
    let _ = load_and_build(out, &mut session, global, &request, None);
    out.flush(&mut session)
}

/// `wowlua types sync [--flavor <f>…]` (spec Package manager §10.9).
pub fn types_sync(out: &Output, global: &GlobalArgs, flavors: &[String]) -> bool {
    let mut session = Session::new();
    let options = load_options(out, global);
    let Some(projects) = load_projects(global, &mut session, &options) else {
        out.flush(&mut session);
        return false;
    };
    let mut only = Vec::new();
    for f in flavors {
        match wowlua_flavors::table().resolve(f) {
            Some(flavor) => only.push(flavor.id.clone()),
            None => session.push(Diagnostic::error("E4004", format!("unknown flavor `{f}`"))),
        }
    }
    if session.has_errors() {
        out.flush(&mut session);
        return false;
    }
    let changes = wowlua_driver::types_sync(
        &mut session,
        &options,
        &projects,
        (!flavors.is_empty()).then_some(only.as_slice()),
    );
    let ok = out.flush(&mut session);
    let Some(changes) = changes.filter(|_| ok) else {
        return false;
    };
    if changes.is_empty() {
        out.status(
            "Fresh",
            format!(
                "wowlua.lock is pinned to the flavor table {}",
                wowlua_flavors::table().version
            ),
        );
    }
    for change in changes {
        out.status("Updating", change);
    }
    true
}

/// `wowlua types convert <path> [--out <dir>]`.
pub fn types_convert(out: &Output, path: &Path, out_dir: Option<&Path>) -> bool {
    let cwd = cwd();
    let out_dir = out_dir.map_or_else(|| cwd.clone(), |d| cwd.join(d));
    match wowlua_driver::types::convert(&cwd.join(path), &out_dir) {
        Ok((written, inputs)) => {
            let s = if inputs == 1 { "" } else { "s" };
            out.status(
                "Converted",
                format!("{inputs} file{s} → {}", relative(&written)),
            );
            true
        }
        Err(e) => {
            let mut session = Session::new();
            session.push(Diagnostic::plain_error(e));
            out.flush(&mut session);
            false
        }
    }
}

/// The text of the clipboard, through the platform's tool.
fn clipboard_text() -> Result<String, String> {
    let attempts: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else if cfg!(windows) {
        &[("powershell", &["-NoProfile", "-Command", "Get-Clipboard"])]
    } else {
        &[
            ("wl-paste", &["--no-newline"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["--clipboard", "--output"]),
        ]
    };
    for (program, args) in attempts {
        if let Ok(output) = std::process::Command::new(program).args(*args).output()
            && output.status.success()
        {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }
    Err("cannot read the clipboard (no clipboard tool found)".into())
}

/// `wowlua trace [file] [--clipboard] [--build <id>]`.
pub fn trace(
    out: &Output,
    global: &GlobalArgs,
    file: Option<&Path>,
    clipboard: bool,
    build: Option<&str>,
) -> bool {
    let mut session = Session::new();
    let input = if clipboard {
        clipboard_text()
    } else if let Some(path) = file {
        std::fs::read_to_string(cwd().join(path))
            .map_err(|e| format!("cannot read `{}`: {e}", path.display()))
    } else {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map(|_| text)
            .map_err(|e| format!("cannot read standard input: {e}"))
    };
    let input = match input {
        Ok(text) => text,
        Err(e) => {
            session.push(Diagnostic::plain_error(e));
            out.flush(&mut session);
            return false;
        }
    };
    let Some(project) = load_project(
        global,
        &mut session,
        &LoadOptions {
            skip_dependencies: true,
            ..load_options(out, global)
        },
    ) else {
        out.flush(&mut session);
        return false;
    };
    out.print(wowlua_driver::trace::decode(&project, &input, build));
    out.flush(&mut session)
}

fn cli_kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Addon => "addon",
        Kind::Library => "library",
        Kind::LuaLibrary => "lua-library",
        Kind::Types => "types",
    }
}

/// Lockfile changes after a dependency command: added, changed and
/// removed packages.
fn report_lock_changes(out: &Output, before: &[wowlua_pkg::LockedPackage], project: &Project) {
    let after = &project.lock.packages;
    for p in after {
        match before.iter().find(|b| b.name == p.name) {
            None => out.status("Locking", format!("{} {}", p.name, p.version)),
            Some(b) if b.source != p.source => out.status(
                "Updating",
                format!("{} {} -> {}", p.name, b.version, p.version),
            ),
            Some(_) => {}
        }
    }
    for b in before {
        if !after.iter().any(|p| p.name == b.name) {
            out.status("Removing", format!("{} {}", b.name, b.version));
        }
    }
}

fn current_lock(global: &GlobalArgs) -> Vec<wowlua_pkg::LockedPackage> {
    let cwd = cwd();
    let manifest = global.manifest.as_ref().map(|m| cwd.join(m));
    // A workspace member's lockfile is at the workspace root.
    wowlua_manifest::find_manifest(&cwd, manifest.as_deref())
        .and_then(|m| {
            m.ancestors()
                .skip(1)
                .find(|d| d.join(wowlua_driver::lockfile::LOCKFILE).is_file())
                .map(Path::to_path_buf)
        })
        .and_then(|root| {
            wowlua_driver::lockfile::Lockfile::read(&root)
                .ok()
                .flatten()
        })
        .map(|l| l.packages)
        .unwrap_or_default()
}

/// `wowlua add`.
pub fn add(out: &Output, global: &GlobalArgs, args: &AddArgs) -> bool {
    let mut session = Session::new();
    let before = current_lock(global);
    let options = LoadOptions {
        refresh: true,
        ..load_options(out, global)
    };
    let project = if args.addon {
        wowlua_driver::deps::add_addon(&mut session, &options, &args.name, args.optional)
    } else {
        let (name, requirement) = match args.name.split_once('@') {
            Some((n, r)) => (n.to_string(), Some(r.to_string())),
            None => (args.name.clone(), None),
        };
        let request = wowlua_driver::deps::AddRequest {
            name,
            requirement,
            git: args.git.clone(),
            tag: args.tag.clone(),
            branch: args.branch.clone(),
            rev: args.rev.clone(),
            svn: args.svn.clone(),
            path: args.path.clone(),
            archive: args.archive.clone(),
            sha256: args.sha256.clone(),
            strip_prefix: args.strip_prefix.clone(),
            subdir: args.subdir.clone(),
            kind: args.kind.map(|k| cli_kind_name(k).to_string()),
            files: args.files.clone(),
            install_path: args.install_path.clone(),
            dev: args.dev,
            optional: args.optional,
            flavors: args.flavors.clone(),
            features: global.features.clone(),
            no_default_features: global.no_default_features,
        };
        wowlua_driver::deps::add(&mut session, &options, &request)
    };
    let ok = out.flush(&mut session);
    match project {
        Some(p) if ok => {
            out.status(
                "Adding",
                format!(
                    "{} to {}",
                    args.name,
                    if args.addon {
                        if args.optional {
                            "addon-dependencies.optional"
                        } else {
                            "addon-dependencies.required"
                        }
                    } else if args.dev {
                        "dev-dependencies"
                    } else {
                        "dependencies"
                    }
                ),
            );
            report_lock_changes(out, &before, &p);
            true
        }
        _ => false,
    }
}

/// `wowlua remove`.
pub fn remove(out: &Output, global: &GlobalArgs, name: &str, dev: bool) -> bool {
    let mut session = Session::new();
    let before = current_lock(global);
    let project = wowlua_driver::deps::remove(&mut session, &load_options(out, global), name, dev);
    let ok = out.flush(&mut session);
    match project {
        Some(p) if ok => {
            report_lock_changes(out, &before, &p);
            true
        }
        _ => false,
    }
}

/// `wowlua update [names] [--precise <pin>]`.
pub fn update(
    out: &Output,
    global: &GlobalArgs,
    names: Vec<String>,
    precise: Option<String>,
) -> bool {
    let mut session = Session::new();
    if precise.is_some() && names.len() != 1 {
        session.push(Diagnostic::plain_error(
            "`--precise` needs exactly one dependency name",
        ));
        out.flush(&mut session);
        return false;
    }
    let before = current_lock(global);
    let options = LoadOptions {
        update: Some(if names.is_empty() {
            wowlua_pkg::Update::All
        } else {
            wowlua_pkg::Update::Only(names.clone())
        }),
        precise: precise.map(|p| (names[0].clone(), p)),
        refresh: true,
        ..load_options(out, global)
    };
    let project = load_project(global, &mut session, &options);
    let ok = out.flush(&mut session);
    match project {
        Some(p) if ok => {
            for name in &names {
                if !p.lock.packages.iter().any(|l| &l.name == name) {
                    out.status("Warning", format!("`{name}` is not a dependency"));
                }
            }
            report_lock_changes(out, &before, &p);
            true
        }
        _ => false,
    }
}

/// `wowlua fetch`.
pub fn fetch(out: &Output, global: &GlobalArgs) -> bool {
    let mut session = Session::new();
    let options = LoadOptions {
        refresh: true,
        ..load_options(out, global)
    };
    let project = load_project(global, &mut session, &options);
    let ok = out.flush(&mut session);
    match project {
        Some(p) if ok => {
            out.status(
                "Fetched",
                format!(
                    "{} package{}",
                    p.deps.packages.len(),
                    if p.deps.packages.len() == 1 { "" } else { "s" }
                ),
            );
            true
        }
        _ => false,
    }
}

/// `wowlua tree`.
pub fn tree(out: &Output, global: &GlobalArgs, flavor: Option<&str>, invert: Option<&str>) -> bool {
    let mut session = Session::new();
    let project = load_project(global, &mut session, &load_options(out, global));
    let ok = out.flush(&mut session);
    let Some(project) = project.filter(|_| ok) else {
        return false;
    };
    let flavor_id = match flavor {
        Some(f) => match wowlua_flavors::table().resolve(f) {
            Some(fl) => Some(fl.id.clone()),
            None => {
                session.push(Diagnostic::error("E4004", format!("unknown flavor `{f}`")));
                out.flush(&mut session);
                return false;
            }
        },
        None => None,
    };
    if let Some(name) = invert
        && !project.deps.packages.iter().any(|p| p.name == name)
    {
        session.push(Diagnostic::plain_error(format!(
            "`{name}` is not a dependency"
        )));
        out.flush(&mut session);
        return false;
    }
    out.print(wowlua_driver::deps::tree(
        &project,
        flavor_id.as_deref(),
        invert,
    ));
    true
}

/// `wowlua outdated`.
pub fn outdated(out: &Output, global: &GlobalArgs) -> bool {
    let mut session = Session::new();
    let options = LoadOptions {
        refresh: true,
        ..load_options(out, global)
    };
    let project = load_project(global, &mut session, &options);
    let ok = out.flush(&mut session);
    let Some(project) = project.filter(|_| ok) else {
        return false;
    };
    let rows = wowlua_driver::deps::outdated(
        &mut session,
        &project,
        options.offline,
        options.status.clone(),
    );
    out.flush(&mut session);
    if rows.is_empty() {
        out.status("Fresh", "every dependency is up to date");
        return true;
    }
    let width = |f: &dyn Fn(&wowlua_driver::deps::Outdated) -> &str, head: &str| {
        rows.iter()
            .map(|r| f(r).len())
            .max()
            .unwrap_or(0)
            .max(head.len())
    };
    let (wn, wc, wl) = (
        width(&|r| &r.name, "name"),
        width(&|r| &r.current, "current"),
        width(&|r| &r.latest, "latest"),
    );
    let mut text = format!(
        "{:wn$}  {:wc$}  {:wl$}  to update\n",
        "name", "current", "latest"
    );
    for r in &rows {
        text.push_str(&format!(
            "{:wn$}  {:wc$}  {:wl$}  {}\n",
            r.name, r.current, r.latest, r.action
        ));
    }
    out.print(text);
    true
}

/// `wowlua cache clean [--all]`.
pub fn cache_clean(out: &Output, all: bool) -> bool {
    let mut session = Session::new();
    let Some(cache) = wowlua_pkg::Cache::new() else {
        session.push(
            Diagnostic::plain_error("cannot find the package cache").with_help("set `WOWLUA_HOME`"),
        );
        out.flush(&mut session);
        return false;
    };
    let limit = (!all).then(|| Duration::from_secs(30 * 24 * 60 * 60));
    match cache.clean(limit) {
        Ok((removed, freed)) => {
            out.status(
                "Removed",
                format!(
                    "{} cache entr{} ({:.1} MB)",
                    removed.len(),
                    if removed.len() == 1 { "y" } else { "ies" },
                    freed as f64 / 1_048_576.0
                ),
            );
            true
        }
        Err(e) => {
            session.push(Diagnostic::plain_error(format!(
                "cannot clean the cache: {e}"
            )));
            out.flush(&mut session);
            false
        }
    }
}

/// The profiles `wowlua bench` builds with (spec Tooling §16.1.7): the test
/// build, with the chosen profile's debugging settings that cost time
/// (`runtime-checks`, `instrument`, `assertions`, `log-level`) and its
/// line mode, and its optimizations or none.
const BENCH_OPTIMIZED: &str = "wowlua-bench-optimized";
const BENCH_UNOPTIMIZED: &str = "wowlua-bench-unoptimized";

fn bench_profiles(
    project: &mut Project,
    profile: &str,
) -> Result<(), wowlua_manifest::profile::ProfileError> {
    use wowlua_manifest::model::{OptimizeSettings, ProfileSettings};
    let manifest = &mut project.manifest.manifest;
    let chosen = wowlua_manifest::profile::resolve(manifest, profile)?;
    let settings = |o: &wowlua_manifest::profile::Optimize| ProfileSettings {
        inherits: Some("test".into()),
        runtime_checks: Some(chosen.runtime_checks),
        instrument: Some(chosen.instrument),
        assertions: Some(chosen.assertions),
        log_level: Some(chosen.log_level),
        // The profile's lines: a bundled build's code is not kept on its
        // source lines, which some optimizations need.
        line_mode: Some(chosen.effective_line_mode()),
        optimize: Some(OptimizeSettings {
            inline: Some(o.inline),
            cache_globals: Some(o.cache_globals),
            dead_code: Some(o.dead_code),
            constants: Some(o.constants),
            cache_fields: Some(o.cache_fields),
            cache_upvalues: Some(o.cache_upvalues),
            numeric_loops: Some(o.numeric_loops),
            table_insert: Some(o.table_insert),
            hoist_closures: Some(o.hoist_closures),
        }),
        ..ProfileSettings::default()
    };
    let optimized = settings(&chosen.optimize);
    let unoptimized = settings(&wowlua_manifest::profile::Optimize::none());
    manifest.profiles.insert(BENCH_OPTIMIZED.into(), optimized);
    manifest
        .profiles
        .insert(BENCH_UNOPTIMIZED.into(), unoptimized);
    Ok(())
}

/// `wowlua bench` (spec Tooling §16.1.7): the `@bench` functions of each
/// addon's test files, in the build the profile optimizes and in the same
/// build with every optimization off, for one flavor (the first, or each
/// `--flavor`).
pub fn bench(out: &Output, global: &GlobalArgs, args: &crate::BenchArgs) -> bool {
    use termcolor::Color;
    let mut session = Session::new();
    let Some(projects) = load_projects(global, &mut session, &load_options(out, global)) else {
        out.flush(&mut session);
        return false;
    };
    let profile = args.profile.clone().unwrap_or_else(|| "release".into());
    let wanted: Vec<String> = args
        .flavor
        .iter()
        .filter_map(|f| wowlua_flavors::table().resolve(f).map(|f| f.id.clone()))
        .collect();
    let mut options = wowlua_test::bench::Options::default();
    if let Some(n) = args.samples {
        options.samples = n.max(1);
    }
    if let Some(ms) = args.batch_ms {
        options.batch = Duration::from_millis(ms.max(1));
    }
    let mut ok = true;
    let mut all: Vec<wowlua_test::bench::BenchFile> = Vec::new();
    for mut project in projects {
        if !matches!(project.kind(), PackageKind::Addon | PackageKind::Library) {
            continue;
        }
        if let Err(e) = bench_profiles(&mut project, &profile) {
            let message = match e {
                wowlua_manifest::profile::ProfileError::Unknown(p) => format!("no profile `{p}`"),
                other => format!("the profile `{profile}` does not resolve: {other:?}"),
            };
            session.push(Diagnostic::error("E4006", message));
            out.flush(&mut session);
            return false;
        }
        let flavors: Vec<String> = if wanted.is_empty() {
            project.manifest.flavors.iter().take(1).cloned().collect()
        } else {
            project
                .manifest
                .flavors
                .iter()
                .filter(|f| wanted.contains(f))
                .cloned()
                .collect()
        };
        let shared_mode = project.manifest.manifest.build.flavor_mode
            == wowlua_manifest::model::FlavorMode::Shared;
        for flavor in flavors {
            let built: Vec<String> = if shared_mode {
                project.manifest.flavors.clone()
            } else {
                vec![flavor.clone()]
            };
            out.status(
                "Building",
                format!(
                    "{} ({flavor}): optimized as `{profile}` and unoptimized",
                    project.addon_name()
                ),
            );
            let plain = test_build_as(
                out,
                global,
                &mut session,
                &project,
                built.clone(),
                &format!("bench-unoptimized-{flavor}"),
                BENCH_UNOPTIMIZED,
            );
            let fast = test_build_as(
                out,
                global,
                &mut session,
                &project,
                built,
                &format!("bench-optimized-{flavor}"),
                BENCH_OPTIMIZED,
            );
            let (Some((plain, _)), Some((fast, _))) = (plain, fast) else {
                ok = false;
                continue;
            };
            let blizzard_ui = if project.manifest.manifest.test.blizzard_ui {
                match wowlua_test::ui_source(&flavor, &|what| out.status("Fetching", what)) {
                    Ok(ui) => Some(ui),
                    Err(e) => {
                        out.colored(&[
                            (Some(Color::Red), true, "error"),
                            (None, false, &format!(": test.blizzard-ui: {e}\n")),
                        ]);
                        ok = false;
                        continue;
                    }
                }
            } else {
                None
            };
            let switches = || SuiteSwitches {
                update_snapshots: false,
                coverage: false,
                game_data: false,
            };
            let plain_suite = make_suite(
                &project,
                &plain,
                &flavor,
                blizzard_ui.clone(),
                switches(),
                None,
            );
            let fast_suite = make_suite(&project, &fast, &flavor, blizzard_ui, switches(), None);
            out.status("Running", format!("benchmarks ({flavor})"));
            let results =
                wowlua_test::bench::run(&plain_suite, &fast_suite, &args.filter, &options);
            ok &= results
                .iter()
                .all(|f| f.error.is_none() && f.benches.iter().all(|b| b.error.is_none()));
            all.extend(results);
        }
    }
    if args.json {
        println!("{}", bench_json(&all, &profile, &options));
    } else {
        bench_table(out, &all, &profile);
    }
    ok
}

fn bench_json(
    files: &[wowlua_test::bench::BenchFile],
    profile: &str,
    options: &wowlua_test::bench::Options,
) -> String {
    let timing = |t: &wowlua_test::bench::Timing| serde_json::json!({ "nanos": t.nanos, "spread": t.spread });
    let files: Vec<serde_json::Value> = files
        .iter()
        .map(|f| {
            serde_json::json!({
                "file": f.file,
                "flavor": f.flavor,
                "error": f.error,
                "benches": f.benches.iter().map(|b| serde_json::json!({
                    "name": b.name,
                    "skipped": b.skipped,
                    "error": b.error,
                    "calls": b.calls,
                    "unoptimized": timing(&b.unoptimized),
                    "optimized": timing(&b.optimized),
                    "speedup": b.speedup(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "profile": profile,
        "samples": options.samples,
        "batch_ms": options.batch.as_millis() as u64,
        "files": files,
    }))
    .expect("JSON")
}

/// A time per call, in the unit that suits it.
fn per_call(nanos: f64) -> String {
    if nanos >= 1_000_000.0 {
        format!("{:.2} ms", nanos / 1_000_000.0)
    } else if nanos >= 1_000.0 {
        format!("{:.2} µs", nanos / 1_000.0)
    } else {
        format!("{nanos:.0} ns")
    }
}

fn bench_table(out: &Output, files: &[wowlua_test::bench::BenchFile], profile: &str) {
    use termcolor::Color;
    let width = files
        .iter()
        .flat_map(|f| f.benches.iter().map(|b| b.name.chars().count()))
        .max()
        .unwrap_or(0)
        .max(9);
    for f in files {
        out.colored(&[(None, true, &format!("\n{} ({})\n", f.file, f.flavor))]);
        if let Some(e) = &f.error {
            out.colored(&[
                (Some(Color::Red), true, "  error"),
                (None, false, &format!(": {e}\n")),
            ]);
            continue;
        }
        out.colored(&[(
            None,
            false,
            &format!(
                "  {:<width$}  {:>20}  {:>20}  speed-up\n",
                "benchmark", "unoptimized", profile
            ),
        )]);
        for b in &f.benches {
            if let Some(reason) = &b.skipped {
                out.colored(&[(
                    Some(Color::Yellow),
                    false,
                    &format!("  {:<width$}  skipped {reason}\n", b.name),
                )]);
                continue;
            }
            if let Some(e) = &b.error {
                out.colored(&[
                    (None, false, &format!("  {:<width$}  ", b.name)),
                    (Some(Color::Red), true, "error"),
                    (None, false, &format!(": {e}\n")),
                ]);
                continue;
            }
            let cell = |t: &wowlua_test::bench::Timing| {
                format!("{} ± {:.1}%", per_call(t.nanos), t.spread * 100.0)
            };
            let speedup = b.speedup();
            let color = if speedup >= 1.05 {
                Some(Color::Green)
            } else if speedup <= 0.95 {
                Some(Color::Red)
            } else {
                None
            };
            out.colored(&[
                (
                    None,
                    false,
                    &format!(
                        "  {:<width$}  {:>20}  {:>20}  ",
                        b.name,
                        cell(&b.unoptimized),
                        cell(&b.optimized)
                    ),
                ),
                (color, false, &format!("{speedup:.2}x\n")),
            ]);
        }
    }
}
