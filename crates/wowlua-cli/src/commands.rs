//! Command implementations. Each returns `true` on success.

use crate::output::Output;
use crate::{AddArgs, BuildArgs, GlobalArgs, InstallTarget, Kind};
use notify_debouncer_mini::{DebouncedEvent, new_debouncer, notify::RecursiveMode};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use wowlua_diagnostics::{Diagnostic, LintLevel};
use wowlua_driver::install::{self, InstallRequest};
use wowlua_driver::scaffold::{self, InitRequest, NewRequest};
use wowlua_driver::{BuildOutput, BuildRequest, LoadOptions, Project, Session};
use wowlua_manifest::model::{LintSetting, PackageKind};

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
    }
}

/// Applies `check.lints` from the manifest to the collected diagnostics.
fn apply_lints(session: &mut Session, project: &Project) {
    let lints = project.manifest.manifest.check.lints.clone();
    session.diagnostics.apply_lint_levels(|code| {
        lints.get(code).map(|level| match level {
            LintSetting::Allow => LintLevel::Allow,
            LintSetting::Warn => LintLevel::Warn,
            LintSetting::Deny => LintLevel::Deny,
        })
    });
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
) -> Option<Vec<(Project, BuildOutput)>> {
    let projects = wowlua_driver::load_projects(session, &load_options(out, global))?;
    let single = projects.len() == 1;
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
    (!session.has_errors()).then_some(builds)
}

/// Loads the project and builds it. Diagnostics stay in the session.
fn load_and_build(
    out: &Output,
    session: &mut Session,
    global: &GlobalArgs,
    request: &BuildRequest,
) -> Option<(Project, BuildOutput)> {
    let project = wowlua_driver::load_project(session, &load_options(out, global))?;
    let build = wowlua_driver::build(session, &project, request);
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
    };
    let result = load_and_build_all(out, &mut session, global, &request);
    let ok = out.flush(&mut session);
    match result {
        Some(builds) if ok => {
            for (_, build) in &builds {
                out.status(
                    "Finished",
                    format!(
                        "{} in {:.2}s → {}",
                        describe(build),
                        started.elapsed().as_secs_f64(),
                        relative(&build.dir)
                    ),
                );
            }
            true
        }
        _ => false,
    }
}

pub fn toc(out: &Output, global: &GlobalArgs, profile: &str, flavors: Vec<String>) -> bool {
    let mut session = Session::new();
    let request = BuildRequest {
        profile: profile.into(),
        flavors: (!flavors.is_empty()).then_some(flavors),
        out_dir: None,
        write: false,
    };
    let result = load_and_build(out, &mut session, global, &request);
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
    };
    let result = load_and_build_all(out, &mut session, global, &request);
    let ok = out.flush(&mut session);
    match result {
        Some(builds) if ok => {
            for (_, build) in &builds {
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
    };
    let Some(builds) = load_and_build_all(out, &mut session, global, &request) else {
        out.flush(&mut session);
        return false;
    };
    if !out.flush(&mut session) {
        return false;
    }
    let mut ok = true;
    for (project, build) in &builds {
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
    for project in projects.iter().filter(|p| p.kind() == PackageKind::Addon) {
        let removed = install::uninstall(&mut session, project, &request);
        ok &= out.flush(&mut session);
        for path in &removed {
            out.status("Removed", path.display().to_string());
        }
        if removed.is_empty() && ok {
            out.status(
                "Nothing",
                format!("no installed build of {} found", project.addon_name()),
            );
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

pub fn init(out: &Output, from_toc: Option<PathBuf>, from_pkgmeta: bool) -> bool {
    let mut session = Session::new();
    if from_pkgmeta {
        session.push(
            Diagnostic::plain_error("`--from-pkgmeta` is not available yet")
                .with_note("converting .pkgmeta arrives with the package manager (milestone M6)"),
        );
        out.flush(&mut session);
        return false;
    }
    let request = InitRequest {
        dir: cwd(),
        from_toc: from_toc.map(|t| cwd().join(t)),
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
            out.status(
                "Note",
                format!(
                    "TOC files are generated from wowlua.json now; the old {} can be deleted",
                    report.old_tocs.join(", ")
                ),
            );
            true
        }
        Err(diagnostic) => {
            session.push(diagnostic);
            out.flush(&mut session);
            false
        }
    }
}

pub fn dev(out: &Output, global: &GlobalArgs, target: &InstallTarget, link: bool) -> bool {
    let request = BuildRequest {
        profile: "dev".into(),
        flavors: (!target.flavor.is_empty()).then(|| target.flavor.clone()),
        out_dir: None,
        write: true,
    };
    let install = install_request(target, link);

    let cycle = |first: bool| -> Option<PathBuf> {
        let started = Instant::now();
        let mut session = Session::new();
        let result = load_and_build(out, &mut session, global, &request);
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
        Some(project.root().to_path_buf())
    };

    let root = cycle(true).or_else(|| {
        wowlua_manifest::find_manifest(&cwd(), global.manifest.as_deref())
            .and_then(|m| m.parent().map(Path::to_path_buf))
    });
    let Some(root) = root else { return false };
    let out_dir = root.join("dist");

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
    let relevant = |event: &DebouncedEvent| {
        let path = &event.path;
        !path.starts_with(&out_dir)
            && !path.components().any(|c| c.as_os_str() == ".git")
            && !path.to_string_lossy().contains(".wowlua-tmp")
    };
    while let Ok(result) = rx.recv() {
        if let Ok(events) = result
            && events.iter().any(relevant)
        {
            cycle(false);
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
    };
    let _ = load_and_build(out, &mut session, global, &request);
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
    if build.is_some() {
        // Every build keeps its source lines until bundling (milestone M7):
        // locations decode without the build's source maps.
        session.push(Diagnostic::note(
            "builds are line-preserved: `--build` is not needed yet",
        ));
    }
    out.print(wowlua_driver::trace::decode(&project, &input));
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
