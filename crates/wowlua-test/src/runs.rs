//! A test run's steps, as `wowlua test` and wowlua studio take them (spec
//! Tooling §16.1): a package's test build for its flavors (the test files
//! that do not compile left out), the runner's suite of it, those files'
//! results, and the coverage minimums a run is held to.

use std::sync::Arc;

use wowlua_driver::{BuildOutput, BuildRequest, Project, Session};
use wowlua_manifest::model::{CoverageSettings, TestPlatform};

use crate::coverage::{Report, percent};
use crate::{FileResult, SnapshotSummary, Suite, UiSource};

/// How a suite runs: `-u`, `--coverage`, `--game-data`, `--world`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Switches {
    pub update_snapshots: bool,
    pub coverage: bool,
    pub game_data: bool,
    pub world: bool,
}

/// The runner's suite for a test build of `project` for `flavor`, with
/// what `wowlua.json`'s `test` sets: the profile, platform, screen and
/// other addons.
pub fn suite<'a>(
    project: &Project,
    build: &'a BuildOutput,
    flavor: &str,
    blizzard_ui: Option<UiSource>,
    switches: Switches,
    instrument: Option<&'a (dyn Fn(&mut wowlua_vm::Vm) + Sync)>,
    camera: Option<Arc<dyn crate::screenshots::Camera>>,
) -> Suite<'a> {
    let Switches {
        update_snapshots,
        coverage,
        game_data,
        world,
    } = switches;
    let test_settings = &project.manifest.manifest.test;
    Suite {
        build,
        flavor: flavor.to_string(),
        root: project.root().to_path_buf(),
        src: project.manifest.manifest.build.src.clone(),
        blizzard_ui,
        profile: test_settings.profile.clone(),
        profile_text: None,
        platform: match test_settings.platform {
            Some(TestPlatform::Windows) => crate::Platform::Windows,
            _ => crate::Platform::Mac,
        },
        screen: match test_settings.screen {
            Some(s) => crate::Screen {
                width: s.width,
                height: s.height,
                dpi_scale: s.dpi_scale.unwrap_or(1.0),
                ui_scale: s.ui_scale,
            },
            None => crate::Screen::default(),
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
        camera,
        game_data,
        world,
        after_test: None,
        after_file: None,
    }
}

/// Builds `project`'s test build (`profile`, `test` for a run) for
/// `flavors` into `<out>/.test/<dir>`. A test file that does not compile
/// is left out and the build made again without it (`show` is given the
/// session with its errors first): the others run. The session keeps the
/// last build's diagnostics; with errors in them the build is not run.
/// The build and the test files left out.
pub fn build(
    session: &mut Session,
    project: &Project,
    flavors: Vec<String>,
    dir: &str,
    profile: &str,
    jobs: wowlua_driver::parallel::Jobs,
    show: &mut dyn FnMut(&mut Session),
) -> Option<(BuildOutput, Vec<String>)> {
    let mut left_out: Vec<String> = Vec::new();
    loop {
        let request = BuildRequest {
            profile: profile.into(),
            flavors: Some(flavors.clone()),
            out_dir: Some(crate::build_dir(
                &project.manifest.out_dir(),
                dir,
                &project.addon_name(),
            )),
            write: true,
            nolib: false,
            minify: None,
            bundle: None,
            only: None,
            jobs,
            keep_code: true,
            tests: true,
            left_out: left_out.clone(),
            reproducible: false,
            progress: None,
        };
        let build = wowlua_driver::build(session, project, &request);
        project.apply_lints(session);
        if let Some(broken) = broken_test_files(session)
            && !broken.iter().all(|b| left_out.contains(b))
        {
            show(session);
            left_out.extend(broken);
            continue;
        }
        return build.map(|b| (b, left_out));
    }
}

/// The test files left out of a run because they do not compile: a failed
/// file each when the run selects it (no filter, or one in its path); the
/// others' names (a filtered run is not held up by another's half-written
/// file).
pub fn not_compiling(
    left_out: &[String],
    filters: &[String],
    flavor: &str,
) -> (Vec<FileResult>, Vec<String>) {
    let mut results = Vec::new();
    let mut skipped = Vec::new();
    for file in left_out {
        // (a source's examples are named by that source)
        let shown = wowlua_driver::examples::source_of(file);
        if filters.is_empty()
            || filters
                .iter()
                .any(|f| file.contains(f.as_str()) || shown.contains(f.as_str()))
        {
            results.push(FileResult {
                file: shown.clone(),
                flavor: flavor.to_string(),
                tests: Vec::new(),
                error: Some(
                    "it does not compile (its errors are above); the other test files ran".into(),
                ),
                printed: String::new(),
                seconds: 0.0,
                snapshots: SnapshotSummary::default(),
                coverage: None,
            });
        } else {
            skipped.push(shown);
        }
    }
    (results, skipped)
}

/// The test files (`*.test.wowlua`, package-relative) the session's errors
/// are in, when every error is in one; `None` without errors, or with one
/// elsewhere (the addon, a mock or helper: nothing could run).
fn broken_test_files(session: &Session) -> Option<Vec<String>> {
    let mut files: Vec<String> = Vec::new();
    for d in session.diagnostics.iter().filter(|d| d.is_error()) {
        // (an error in a doc comment's example is its examples file's)
        let path = d
            .labels
            .first()
            .map(|l| match session.examples.get(&l.span.file) {
                Some(examples) => std::path::PathBuf::from(examples),
                None => session.db.get(l.span.file).path().to_path_buf(),
            })
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

/// A coverage kind below its minimum: the kind, the run's percentage, the
/// minimum.
#[derive(Clone, Debug, PartialEq)]
pub struct Below {
    pub kind: &'static str,
    pub percent: f64,
    pub minimum: f64,
}

/// The kinds of `report` below `settings`' minimums (`statements` are the
/// lines). A run with filters checks part of the suite while its totals
/// still count every source line: hold only a whole suite's run to them.
pub fn below_minimums(report: &Report, settings: &CoverageSettings) -> Vec<Below> {
    let t = report.totals();
    [
        ("lines", settings.lines, t.lines),
        ("statements", settings.statements, t.lines),
        ("branches", settings.branches, t.branches),
        ("functions", settings.functions, t.functions),
        ("conditions", settings.conditions, t.conditions),
    ]
    .into_iter()
    .filter_map(|(kind, min, got)| {
        let p = percent(got);
        min.filter(|m| p < *m).map(|minimum| Below {
            kind,
            percent: p,
            minimum,
        })
    })
    .collect()
}
