//! Command implementations. Each returns `true` on success.

use crate::output::Output;
use crate::{AddArgs, BuildArgs, GlobalArgs, InstallTarget, Kind};
use notify_debouncer_mini::{DebouncedEvent, new_debouncer, notify::RecursiveMode};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use wowlua_diagnostics::Diagnostic;
use wowlua_driver::install::{self, InstallRequest};
use wowlua_driver::scaffold::{self, InitRequest, NewRequest};
use wowlua_driver::{BuildOutput, BuildRequest, LoadOptions, Project, Session};
use wowlua_manifest::model::PackageKind;

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn load_options(out: &Output, global: &GlobalArgs) -> LoadOptions {
    let cwd = cwd();
    LoadOptions {
        manifest: global.manifest.as_ref().map(|m| cwd.join(m)),
        cwd,
        package: global.package.clone(),
        locked: global.locked || global.frozen,
        offline: global.offline || global.frozen,
        skip_dependencies: false,
        update: None,
        precise: None,
        status: Some(out.status_sink()),
        bundled: false,
    }
}

/// Applies `check.lints` from the manifest to the collected diagnostics.
fn apply_lints(session: &mut Session, project: &Project) {
    project.apply_lints(session);
}

fn relative(path: &Path) -> String {
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
    let projects = wowlua_driver::load_projects(session, &load_options(out, global))?;
    let single = projects.len() == 1;
    let whole = whole_workspace(global, &projects);
    let mut builds = Vec::new();
    for project in projects {
        let buildable = match project.kind() {
            PackageKind::Addon => true,
            PackageKind::Library => !request.write,
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
    let project = wowlua_driver::load_project(session, &load_options(out, global))?;
    let build = match cache {
        Some(cache) => wowlua_driver::build_with_cache(session, &project, request, cache),
        None => wowlua_driver::build(session, &project, request),
    };
    apply_lints(session, &project);
    build.map(|b| (project, b))
}

pub fn build(out: &Output, global: &GlobalArgs, args: &BuildArgs) -> bool {
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
    };
    let result = load_and_build_all(out, &mut session, global, &request);
    let ok = out.flush(&mut session);
    match result {
        Some(built) if ok => {
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
            true
        }
        _ => false,
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
        ..load_options(out, global)
    };
    let Some(projects) = wowlua_driver::load_projects(&mut session, &options) else {
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

pub fn publish(out: &Output, global: &GlobalArgs, args: crate::PublishArgs) -> bool {
    use wowlua_manifest::model::ReleaseType;
    let started = Instant::now();
    let mut session = Session::new();
    // A publish never changes the lockfile (§11.1.1).
    let mut options = load_options(out, global);
    options.locked = true;
    options.bundled = true;
    let Some(projects) = wowlua_driver::load_projects(&mut session, &options) else {
        out.flush(&mut session);
        return false;
    };
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

/// `wowlua check`: the build pipeline without writing (type checking
/// arrives with M4).
pub fn check(out: &Output, global: &GlobalArgs, profile: &str, flavors: Vec<String>) -> bool {
    let started = Instant::now();
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
        keep_code: false,
        // The test side too (test files, mocks, helpers), as `wowlua test`
        // compiles it.
        tests: true,
        left_out: Vec::new(),
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
            true
        }
        _ => false,
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
    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = match new_debouncer(Duration::from_millis(300), tx) {
        Ok(d) => d,
        Err(error) => {
            let mut session = Session::new();
            session.push(Diagnostic::plain_error(format!(
                "cannot watch files: {error}"
            )));
            out.flush(&mut session);
            return false;
        }
    };
    if let Err(error) = debouncer.watcher().watch(&root, RecursiveMode::Recursive) {
        let mut session = Session::new();
        session.push(Diagnostic::plain_error(format!(
            "cannot watch `{}`: {error}",
            root.display()
        )));
        out.flush(&mut session);
        return false;
    }
    let shown = relative(&root);
    out.status(
        "Watching",
        format!(
            "{} for changes (Ctrl+C to stop)",
            if shown.is_empty() { "." } else { &shown }
        ),
    );
    let relevant = |result: &Result<Vec<DebouncedEvent>, _>| {
        result.as_ref().is_ok_and(|events| {
            events.iter().any(|event| {
                let path = &event.path;
                !path.starts_with(&out_dir)
                    && !path.components().any(|c| c.as_os_str() == ".git")
                    && !path.to_string_lossy().contains(".wowlua-tmp")
            })
        })
    };
    while let Ok(result) = rx.recv() {
        let mut changed = relevant(&result);
        // Changes during the last run: one run for all.
        while let Ok(result) = rx.try_recv() {
            changed |= relevant(&result);
        }
        if changed {
            test_once(out, global, args);
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
    let Some(projects) = wowlua_driver::load_projects(&mut session, &load_options(out, global))
    else {
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
        if project.kind() != PackageKind::Addon {
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
            let blizzard_ui = if test_settings.blizzard_ui {
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
            let suite = make_suite(
                &project,
                build,
                &flavor,
                blizzard_ui,
                args.update_snapshots,
                args.coverage,
                None,
            );
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
                let t = report.totals();
                for (kind, min, got) in [
                    ("lines", s.lines, t.lines),
                    ("statements", s.statements, t.lines),
                    ("branches", s.branches, t.branches),
                    ("functions", s.functions, t.functions),
                    ("conditions", s.conditions, t.conditions),
                ] {
                    let p = wowlua_test::coverage::percent(got);
                    if let Some(min) = min
                        && p < min
                    {
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

/// The test runner's suite for a test build of `project`.
fn make_suite<'a>(
    project: &Project,
    build: &'a BuildOutput,
    flavor: &str,
    blizzard_ui: Option<wowlua_test::UiSource>,
    update_snapshots: bool,
    coverage: bool,
    instrument: Option<&'a (dyn Fn(&mut wowlua_vm::Vm) + Sync)>,
) -> wowlua_test::Suite<'a> {
    let test_settings = &project.manifest.manifest.test;
    wowlua_test::Suite {
        build,
        flavor: flavor.to_string(),
        root: project.root().to_path_buf(),
        src: project.manifest.manifest.build.src.clone(),
        blizzard_ui,
        profile: test_settings.profile.clone(),
        platform: match test_settings.platform {
            Some(wowlua_manifest::model::TestPlatform::Windows) => wowlua_test::Platform::Windows,
            _ => wowlua_test::Platform::Mac,
        },
        screen: match test_settings.screen {
            Some(s) => wowlua_test::Screen {
                width: s.width,
                height: s.height,
                dpi_scale: s.dpi_scale.unwrap_or(1.0),
                ui_scale: s.ui_scale,
            },
            None => wowlua_test::Screen::default(),
        },
        // Relative to the package root, or absolute.
        addons: test_settings
            .addons
            .iter()
            .map(|a| project.root().join(a))
            .collect(),
        update_snapshots,
        coverage,
        instrument,
        camera: crate::screenshot::camera(project, build, flavor),
        after_test: None,
    }
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
    let Some(projects) = wowlua_driver::load_projects(&mut session, &load_options(out, global))
    else {
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
        false,
        false,
        instrument,
    );
    f(&mut suite);
    Ok(())
}

/// The package root of the current directory's addon.
pub fn package_root(out: &Output, global: &GlobalArgs) -> Option<PathBuf> {
    let mut session = Session::new();
    let projects = wowlua_driver::load_projects(&mut session, &load_options(out, global))?;
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
    let mut left_out: Vec<String> = Vec::new();
    loop {
        let request = BuildRequest {
            profile: "test".into(),
            flavors: Some(flavors.clone()),
            out_dir: Some(wowlua_test::build_dir(
                &project.manifest.out_dir(),
                dir,
                &project.addon_name(),
            )),
            write: true,
            nolib: false,
            minify: None,
            bundle: None,
            only: None,
            jobs: global.jobs(),
            keep_code: true,
            tests: true,
            left_out: left_out.clone(),
        };
        let build = wowlua_driver::build(session, project, &request);
        apply_lints(session, project);
        if let Some(broken) = broken_test_files(session)
            && !broken.iter().all(|b| left_out.contains(b))
        {
            out.show(session);
            left_out.extend(broken);
            continue;
        }
        if !out.flush(session) {
            return None;
        }
        return build.map(|b| (b, left_out));
    }
}

/// The test files left out of a run because they do not compile: a failed
/// file each when the run selects it (no filter, or one in its path), else
/// a note (a filtered run is not held up by another's half-written file).
fn not_compiling(
    out: &Output,
    left_out: &[String],
    filters: &[String],
    flavor: &str,
) -> Vec<wowlua_test::FileResult> {
    let mut results = Vec::new();
    for file in left_out {
        if filters.is_empty() || filters.iter().any(|f| file.contains(f.as_str())) {
            results.push(wowlua_test::FileResult {
                file: file.clone(),
                flavor: flavor.to_string(),
                tests: Vec::new(),
                error: Some(
                    "it does not compile (its errors are above); the other test files ran".into(),
                ),
                printed: String::new(),
                seconds: 0.0,
                snapshots: wowlua_test::SnapshotSummary::default(),
                coverage: None,
            });
        } else {
            out.status("Skipped", format!("{file}, which does not compile"));
        }
    }
    results
}

/// The test files (`*.test.wowlua`, package-relative) the session's errors
/// are in, when every error is in one; `None` without errors, or with one
/// elsewhere (the addon, a mock or helper: nothing could run).
fn broken_test_files(session: &Session) -> Option<Vec<String>> {
    let mut files: Vec<String> = Vec::new();
    for d in session.diagnostics.iter().filter(|d| d.is_error()) {
        let path = d
            .labels
            .first()
            .map(|l| session.db.get(l.span.file).path().to_path_buf())
            .or_else(|| d.path.clone())?;
        let path = path.to_string_lossy().replace('\\', "/");
        let path = path.strip_prefix("./").unwrap_or(&path).to_string();
        if !path.ends_with(".test.wowlua") {
            return None;
        }
        if !files.contains(&path) {
            files.push(path);
        }
    }
    (!files.is_empty()).then_some(files)
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
        wowlua_driver::load_project(&mut session, &options)
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
    let Some(projects) = wowlua_driver::load_projects(
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
            "{} of {} {} {verb}{skipped}",
            report.changed.len(),
            total,
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
    let Some(projects) = wowlua_driver::load_projects(
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
    let Some(projects) = wowlua_driver::load_projects(
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
    match wowlua_diagnostics::lookup(code) {
        Some(info) => {
            let kind = if info.code.starts_with('E') {
                "error"
            } else {
                "warning"
            };
            out.print(format!(
                "{} ({kind}): {}\n\nSee the diagnostics reference: spec/appendix/diagnostics.md\n",
                info.code,
                info.summary.replace('`', "")
            ));
            true
        }
        None => {
            let mut session = Session::new();
            session.push(Diagnostic::plain_error(format!(
                "unknown diagnostic code `{code}`"
            )));
            out.flush(&mut session);
            false
        }
    }
}

pub fn new(
    out: &Output,
    path: PathBuf,
    kind: Kind,
    name: Option<String>,
    flavors: Vec<String>,
    ci_github: bool,
) -> bool {
    let kind = match kind {
        Kind::Addon => PackageKind::Addon,
        Kind::Library => PackageKind::Library,
        Kind::LuaLibrary => PackageKind::LuaLibrary,
        Kind::Types => PackageKind::Types,
    };
    let request = NewRequest {
        path: cwd().join(&path),
        kind,
        name,
        flavors,
        ci_github,
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

fn plural(count: usize, one: &str, many: &str) -> String {
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

pub fn init(out: &Output, from_toc: Option<PathBuf>, from_pkgmeta: Option<String>) -> bool {
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
    let mut out_dir = first.map_or_else(|| root.join("dist"), |(_, out_dir)| out_dir);

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = match new_debouncer(Duration::from_millis(300), tx) {
        Ok(d) => d,
        Err(error) => {
            let mut session = Session::new();
            session.push(Diagnostic::plain_error(format!(
                "cannot watch files: {error}"
            )));
            out.flush(&mut session);
            return false;
        }
    };
    if let Err(error) = debouncer.watcher().watch(&root, RecursiveMode::Recursive) {
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
        format!("{} for changes (Ctrl+C to stop)", relative(&root)),
    );
    let relevant = |out_dir: &Path, result: &Result<Vec<DebouncedEvent>, _>| {
        result.as_ref().is_ok_and(|events| {
            events.iter().any(|event| {
                let path = &event.path;
                !path.starts_with(out_dir)
                    && !path.components().any(|c| c.as_os_str() == ".git")
                    && !path.to_string_lossy().contains(".wowlua-tmp")
            })
        })
    };
    while let Ok(result) = rx.recv() {
        let mut changed = relevant(&out_dir, &result);
        // Changes that arrived during the last build: one build for all.
        while let Ok(result) = rx.try_recv() {
            changed |= relevant(&out_dir, &result);
        }
        if !changed {
            continue;
        }
        if let Some((_, dir)) = cycle(false) {
            out_dir = dir;
        }
        builds += 1;
        if exit_after.is_some_and(|n| builds >= n) {
            break;
        }
    }
    true
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
    };
    let mut total = 0;
    let mut files: std::collections::BTreeSet<PathBuf> = Default::default();
    for _ in 0..10 {
        let mut session = Session::new();
        let Some(project) = wowlua_driver::load_project(&mut session, &load_options(out, global))
        else {
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
    };
    let Some(project) = wowlua_driver::load_project(&mut session, &load_options(out, global))
    else {
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
pub fn fix(out: &Output, global: &GlobalArgs, manifest: bool) -> bool {
    if manifest {
        let mut session = Session::new();
        let Some(project) = wowlua_driver::load_project(
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
    };
    let _ = load_and_build(out, &mut session, global, &request, None);
    out.flush(&mut session)
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
    let Some(project) = wowlua_driver::load_project(
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
    let options = load_options(out, global);
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
            features: args.features.clone(),
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
        ..load_options(out, global)
    };
    let project = wowlua_driver::load_project(&mut session, &options);
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
    let project = wowlua_driver::load_project(&mut session, &load_options(out, global));
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
    let project = wowlua_driver::load_project(&mut session, &load_options(out, global));
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
    let options = load_options(out, global);
    let project = wowlua_driver::load_project(&mut session, &options);
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
