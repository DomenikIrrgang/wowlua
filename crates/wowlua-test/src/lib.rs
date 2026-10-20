//! `wowlua test` (spec Tooling §16.1): a test build's test files run in the
//! engine, each file in a fresh VM, in parallel. A file's VM loads the
//! runtime, the project mocks, the whole addon in its TOC's order, then the
//! test file with its helpers (§16.1.2); its tests run as `wowlua:test`
//! registered them, each in a coroutine the runner steps, advancing the
//! game's frames while a test waits.

mod blizzard;
pub mod coverage;
mod natives;
pub mod profile;
pub mod reports;
mod saved;
pub mod scenario;
mod snapshots;

pub use snapshots::Summary as SnapshotSummary;

pub use blizzard::{UiSource, ui_source, ui_source_in};

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;
use wowlua_driver::{BuildOutput, TestFile, TestRole};
use wowlua_vm::{TableRef, Value, Vm};

/// What a test run needs of a test build.
pub struct Suite<'a> {
    pub build: &'a BuildOutput,
    /// The flavor the build is for.
    pub flavor: String,
    /// The package root and its `build.src`, to show source paths.
    pub root: PathBuf,
    pub src: String,
    /// `test.blizzard-ui`: Blizzard's UI code, loaded before the addon.
    pub blizzard_ui: Option<UiSource>,
    /// `test.profile`: the world profile of files that name none.
    pub profile: Option<String>,
    /// `test.addons`: other addons' folders, loaded before the addon.
    pub addons: Vec<PathBuf>,
    /// `-u`: rewrite the snapshots that differ.
    pub update_snapshots: bool,
    /// `--coverage`: count what the addon's files run.
    pub coverage: bool,
}

fn widgets_keep(vm: &mut Vm, v: Value) {
    wowlua_engine::widgets::keep(vm, v);
}

/// One test file's run for one flavor.
#[derive(Clone, Debug)]
pub struct FileResult {
    /// The test file, relative to the package root.
    pub file: String,
    pub flavor: String,
    pub tests: Vec<TestResult>,
    /// The file could not run: an error while loading the addon, the
    /// mocks or the test file.
    pub error: Option<String>,
    /// What the file's code printed.
    pub printed: String,
    pub seconds: f64,
    pub snapshots: SnapshotSummary,
    /// With `--coverage`: what the file's tests ran of the addon.
    pub coverage: Option<coverage::Report>,
}

impl FileResult {
    pub fn failed(&self) -> bool {
        self.error.is_some()
            || self
                .tests
                .iter()
                .any(|t| matches!(t.outcome, Outcome::Fail { .. }))
    }
}

#[derive(Clone, Debug)]
pub struct TestResult {
    pub name: String,
    pub outcome: Outcome,
    pub seconds: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Pass,
    /// The message and where it happened (source locations).
    Fail {
        message: String,
        stack: String,
    },
    Skip(String),
}

/// Runs the suite's test files whose path or tests' names contain
/// `filter` (all without one), on `jobs` threads.
pub fn run(suite: &Suite, filter: Option<&str>, jobs: usize) -> Vec<FileResult> {
    let files: Vec<&TestFile> = suite
        .build
        .tests
        .iter()
        .filter(|t| t.role == TestRole::Test)
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, files.len().max(1)) {
            let builder = std::thread::Builder::new().stack_size(64 << 20);
            let _ = builder.spawn_scoped(scope, || {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(file) = files.get(i) else { break };
                    let result = run_file(suite, file, filter);
                    if let Ok(mut r) = results.lock() {
                        r.push((i, result));
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_default();
    results.sort_by_key(|(i, _)| *i);
    results
        .into_iter()
        .map(|(_, r)| r)
        .filter(|r| r.error.is_some() || !r.tests.is_empty() || filter.is_none())
        .collect()
}

/// What the addon's files' chunk names start with: the client names an
/// addon's files `Interface/AddOns/<Name>/<path>` (`debugstack` in a
/// recording: `Interface/AddOns/StdTest/src/engineprobe.lua:806`).
fn chunk_prefix(build: &BuildOutput) -> String {
    format!("Interface/AddOns/{}/", build.addon_name)
}

/// Loads files of the addon with the names the client gives them
/// ([`chunk_prefix`]); what loads later (Blizzard's addons on demand) is
/// named as before.
fn named_as_client(vm: &mut Vm, build: &BuildOutput, load: impl FnOnce(&mut Vm)) {
    let saved = wowlua_engine::engine_mut(vm)
        .world
        .ui
        .chunk_prefix
        .replace(chunk_prefix(build));
    load(vm);
    wowlua_engine::engine_mut(vm).world.ui.chunk_prefix = saved;
}

/// Whether coverage counts a chunk: the addon's own files (`src/…`), not
/// test files.
fn counted(prefix: &str, chunk: &str) -> bool {
    chunk
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.starts_with("src/") && !rest.ends_with(".test.lua"))
}

/// The source path of an output file (`src/x.lua` → `src/x.wowlua`), by
/// its path in the AddOn folder or its chunk name.
fn source_of(suite: &Suite, output: &str) -> String {
    let output = output
        .strip_prefix(chunk_prefix(suite.build).as_str())
        .unwrap_or(output);
    let src = suite.src.trim_end_matches('/');
    let (prefix, rest) = if let Some(rest) = output.strip_prefix("src/") {
        (
            if src.is_empty() || src == "." {
                String::new()
            } else {
                format!("{src}/")
            },
            rest,
        )
    } else if let Some(rest) = output.strip_prefix("tests/") {
        ("tests/".to_string(), rest)
    } else {
        return output.to_string();
    };
    let stem = rest.strip_suffix(".lua").unwrap_or(rest);
    let wowlua = format!("{prefix}{stem}.wowlua");
    if suite.root.join(&wowlua).exists() {
        wowlua
    } else {
        format!("{prefix}{rest}")
    }
}

/// The first source location in a stack (`tests/x.test.wowlua:12`) that
/// is not the standard library's.
pub fn location(stack: &str) -> Option<String> {
    let re = regex::Regex::new(r"[\w./@-]+\.(?:wowlua|lua):\d+").expect("a valid pattern");
    re.find_iter(stack)
        .map(|m| m.as_str())
        .find(|l| !l.starts_with("wowlua/"))
        .map(str::to_string)
}

/// A failure's message without the location an error put first, and the
/// location: that one, else the stack's.
pub fn split_failure(message: &str, stack: &str) -> (String, Option<String>) {
    let re = regex::Regex::new(r"^([\w./@-]+\.(?:wowlua|lua):\d+): ").expect("a valid pattern");
    if let Some(c) = re.captures(message)
        && !c[1].starts_with("wowlua/")
    {
        return (message[c[0].len()..].to_string(), Some(c[1].to_string()));
    }
    (message.to_string(), location(stack))
}

/// Error messages and stacks with source paths: the test profile keeps
/// lines, so `Interface/AddOns/Demo/src/x.lua:12` is `src/x.wowlua:12`.
/// A chunk name Lua shortened (`...ns/Demo/src/x.lua`, past 52 bytes) is
/// the addon's file it ends, when only one does; other addons' files stay.
fn to_sources(suite: &Suite, text: &str) -> String {
    let re = regex::Regex::new(r"\[?(\.\.\.)?([\w./@-]+\.lua)\]?(:\d+)").expect("a valid pattern");
    let prefix = chunk_prefix(suite.build);
    let own = |path: &str| -> Option<String> {
        if let Some(rest) = path.strip_prefix(prefix.as_str()) {
            return Some(rest.to_string());
        }
        ["src/", "tests/", "wowlua/"]
            .iter()
            .any(|p| path.starts_with(p))
            .then(|| path.to_string())
    };
    let ending = |tail: &str| -> Option<String> {
        let files = suite.build.files.keys();
        let tests = suite.build.tests.iter().map(|t| &t.path);
        let mut found = files
            .chain(tests)
            .filter(|f| f.ends_with(".lua") && format!("{prefix}{f}").ends_with(tail));
        let first = found.next()?;
        found.all(|f| f == first).then(|| first.clone())
    };
    re.replace_all(text, |c: &regex::Captures| {
        let path = if c.get(1).is_some() {
            ending(&c[2])
        } else {
            own(&c[2])
        };
        match path {
            Some(path) => format!("{}{}", source_of(suite, &path), &c[3]),
            None => c[0].to_string(),
        }
    })
    .into_owned()
}

fn get(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.heap.intern(key.as_bytes());
    vm.heap.table(t).get_str(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.heap.intern(key.as_bytes());
    vm.heap.table_mut(t).set_str(k, v);
}

fn text(vm: &mut Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Nil => String::new(),
        other => vm
            .tostring(other)
            .ok()
            .map(|s| text(vm, s))
            .unwrap_or_default(),
    }
}

/// A Lua error's message (a test failure's own, for tables).
fn error_text(vm: &mut Vm, e: &wowlua_vm::LuaError) -> String {
    text(vm, e.value)
}

/// The output paths a file needs loaded first, itself last.
fn closure<'a>(build: &'a BuildOutput, path: &'a str, out: &mut Vec<&'a str>) {
    if out.contains(&path) {
        return;
    }
    if let Some(t) = build.tests.iter().find(|t| t.path == path) {
        // Placed before its imports are, so a cycle ends.
        out.push(path);
        let at = out.len() - 1;
        for d in &t.deps {
            closure(build, d, out);
        }
        let me = out.remove(at);
        out.push(me);
    }
}

fn run_file(suite: &Suite, file: &TestFile, filter: Option<&str>) -> FileResult {
    let started = Instant::now();
    let rel = file.rel.clone();
    let mut result = FileResult {
        file: rel.clone(),
        flavor: suite.flavor.clone(),
        tests: Vec::new(),
        error: None,
        printed: String::new(),
        seconds: 0.0,
        snapshots: SnapshotSummary::default(),
        coverage: None,
    };
    snapshots::begin(&suite.root.join(&file.rel), suite.update_snapshots);
    natives::reset();
    match load(suite, file, None) {
        Ok(mut state) => {
            run_tests(suite, file, &mut state, filter, &mut result);
            if let Some(mut report) = coverage_of(suite, &state.vm) {
                for extra in &state.reloaded_coverage {
                    report.merge(extra);
                }
                result.coverage = Some(report);
            }
            let all_ran = filter.is_none() && result.error.is_none();
            match snapshots::finish(all_ran) {
                Ok(s) => result.snapshots = s,
                Err(e) => result.error = Some(format!("writing the snapshots: {e}")),
            }
        }
        Err(e) => result.error = Some(to_sources(suite, &e)),
    }
    result.printed = natives::take_printed();
    result.seconds = started.elapsed().as_secs_f64();
    result
}

/// The counts of a VM's run, by source file.
fn coverage_of(suite: &Suite, vm: &Vm) -> Option<coverage::Report> {
    let cov = vm.coverage.as_ref()?;
    let prefix = chunk_prefix(suite.build);
    let map = |chunk: &str| -> Option<String> {
        counted(&prefix, chunk).then(|| source_of(suite, chunk))
    };
    Some(coverage::collect(vm, cov, &suite.root, &map))
}

/// A test file's VM, loaded: the registry `wowlua:test` filled.
struct Loaded {
    vm: Vm,
    tests: TableRef,
    /// The variables the TOC saves.
    saved: Vec<String>,
    /// The coverage of the VMs `mock.reloadUI` made and dropped.
    reloaded_coverage: Vec<coverage::Report>,
}

/// After `mock.reloadUI`: the pass, and the clock's time, which goes on.
struct Reload {
    pass: usize,
    now: f64,
    elapsed_ms: f64,
}

/// What fails a test on an error the client drops without a report
/// (`secureexecuterange`'s callbacks): code that failed, though no error
/// handler heard it.
const DROPPED_FAILURE: &str = "an error was dropped without a report (as the client drops it)";

fn load(suite: &Suite, file: &TestFile, reload: Option<&Reload>) -> Result<Loaded, String> {
    let build = suite.build;
    let dir = &build.dir;
    let name = build.addon_name.clone();
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, &suite.flavor)?;
    if let Some(r) = reload {
        vm.clock.now = r.now;
        vm.clock.elapsed_ms = r.elapsed_ms;
    }
    if suite.coverage {
        let prefix = chunk_prefix(build);
        vm.enable_coverage(Box::new(move |chunk: &[u8]| {
            counted(
                &prefix,
                &String::from_utf8_lossy(chunk.strip_prefix(b"@").unwrap_or(chunk)),
            )
        }));
    }
    if let Some(ui) = &suite.blizzard_ui {
        let report = wowlua_engine::ui::load(&mut vm, &ui.interface, ui.strings.as_deref());
        if !report.errors.is_empty() {
            return Err(format!(
                "Blizzard's UI code raised {} errors while loading:\n{}",
                report.errors.len(),
                report.errors.join("\n")
            ));
        }
    }
    vm.set_output(natives::print_sink);
    natives::install_print(&mut vm);
    vm.error_handler = vm.native("errorhandler", natives::error_handler);
    vm.on_dropped_error = Some(natives::dropped_error);
    // The addon table, with the runtime's table holding the mock's engine.
    let ns = vm.new_table();
    let wl = vm.new_table();
    set(&mut vm, ns, "__wowlua", Value::Table(wl));
    let controls = wowlua_engine::controls::table(&mut vm);
    natives::extend(&mut vm, controls, &name);
    set(&mut vm, wl, "mockEngine", Value::Table(controls));
    if let Some(r) = reload {
        set(&mut vm, wl, "reloads", Value::Num(r.pass as f64));
    }
    wowlua_engine::widgets::keep(&mut vm, Value::Table(ns));
    let table = Value::Table(ns);
    // The world profile, before the addon (decision T213).
    let source = std::fs::read(suite.root.join(&file.rel)).unwrap_or_default();
    if let Some(name) = profile::named_in(&source).or_else(|| suite.profile.clone()) {
        let path = suite
            .root
            .join("tests")
            .join("profiles")
            .join(format!("{name}.json"));
        profile::apply(&mut vm, controls, &path)?;
    } else if let Some(text) = profile::default_for(&suite.flavor) {
        profile::apply_text(&mut vm, controls, text, "of the fresh character")?;
    }

    // The addon's key bindings, which the client reads at startup.
    let bindings = dir.join("Bindings.xml");
    natives::set_has_bindings(bindings.exists());
    if bindings.exists() {
        wowlua_engine::bindings::load_bindings_xml(&mut vm, &bindings, Some(&name))?;
    }
    let toc = toc_path(build, &suite.flavor).ok_or("the build has no TOC file")?;
    let toc_text = std::fs::read_to_string(dir.join(&toc)).map_err(|e| e.to_string())?;
    let mut saved = saved::names(&wowlua_toc::parse(&toc_text));
    let listed: Vec<String> = wowlua_toc::parse(&toc_text)
        .files
        .iter()
        .map(|f| f.path.replace('\\', "/"))
        .collect();
    let mut loaded: HashSet<String> = HashSet::new();
    let errors_before = |vm: &Vm| wowlua_engine::engine(vm).world.ui.errors.len();
    let check = |vm: &mut Vm, from: usize, what: &str| -> Result<(), String> {
        let errors = &wowlua_engine::engine(vm).world.ui.errors;
        if errors.len() > from {
            return Err(format!("{what}: {}", errors[from..].join("\n")));
        }
        Ok(())
    };
    // The TOC's files up to the runtime (locale files come first), then
    // the mocks (with what they import), then the rest of the addon.
    if let Some(runtime) = listed.iter().position(|p| p == "wowlua/runtime.lua") {
        let before = errors_before(&vm);
        for path in &listed[..=runtime] {
            named_as_client(&mut vm, build, |vm| {
                wowlua_engine::ui::load_user_file(vm, dir, path, &name, table)
            });
            loaded.insert(path.clone());
        }
        check(&mut vm, before, "loading the runtime")?;
    }
    let before = errors_before(&vm);
    for mock in build.tests.iter().filter(|t| t.role == TestRole::Mock) {
        let mut order = Vec::new();
        closure(build, &mock.path, &mut order);
        for path in order {
            if loaded.insert(path.to_string()) {
                named_as_client(&mut vm, build, |vm| {
                    wowlua_engine::ui::load_user_file(vm, dir, path, &name, table)
                });
            }
        }
    }
    check(&mut vm, before, "loading the mocks")?;
    // The other addons (`test.addons`), as the client loads them first.
    for addon in other_addons(suite)? {
        saved.extend(addon.saved.iter().cloned());
        let before = errors_before(&vm);
        load_other_addon(&mut vm, &addon)?;
        check(
            &mut vm,
            before,
            &format!("loading the addon {}", addon.name),
        )?;
    }
    // The addon itself, known to the addon functions as its TOC says.
    let own = wowlua_engine::ui::read_user_toc(&suite.flavor, &name, dir, &dir.join(&toc))?;
    wowlua_engine::bindings::add_own_addon(&mut vm, &own, table, bindings.exists());
    let before = errors_before(&vm);
    named_as_client(&mut vm, build, |vm| {
        wowlua_engine::ui::load_user_addon(vm, dir, &dir.join(&toc), &name, table, &loaded)
    });
    loaded.extend(listed);
    check(&mut vm, before, "loading the addon")?;
    let before = errors_before(&vm);
    let mut order = Vec::new();
    closure(build, &file.path, &mut order);
    for path in order {
        if loaded.insert(path.to_string()) {
            named_as_client(&mut vm, build, |vm| {
                wowlua_engine::ui::load_user_file(vm, dir, path, &name, table)
            });
        }
    }
    check(&mut vm, before, "loading the test file")?;
    let tests = match get(&mut vm, wl, "tests") {
        Value::Table(t) => t,
        _ => return Err("the test file does not import `wowlua:test`".into()),
    };
    // The lifecycle (step 5), unless the file drives it.
    let settings = match get(&mut vm, tests, "settings") {
        Value::Table(t) => Some(t),
        _ => None,
    };
    let manual = settings.is_some_and(|s| {
        let v = get(&mut vm, s, "lifecycle");
        text(&mut vm, v) == "manual"
    });
    if !manual {
        natives::login(&mut vm, &name);
    }
    Ok(Loaded {
        vm,
        tests,
        saved,
        reloaded_coverage: Vec::new(),
    })
}

/// The `test.addons` the client loads in this game, in its load order.
fn other_addons(suite: &Suite) -> Result<Vec<wowlua_engine::ui::UserAddon>, String> {
    let mut list = Vec::new();
    for dir in &suite.addons {
        let addon = wowlua_engine::ui::read_user_addon(&suite.flavor, dir)
            .map_err(|e| format!("test.addons: {e}"))?;
        // The client leaves out an addon for other games (unverified:
        // whether its addon list shows it).
        if addon.allowed {
            list.push(addon);
        }
    }
    Ok(wowlua_engine::ui::user_addon_order(list))
}

/// One of the `test.addons`, known to the client with its own addon table:
/// its `Bindings.xml` read, then its files (unless on demand), after the
/// Blizzard addons it depends on.
fn load_other_addon(vm: &mut Vm, addon: &wowlua_engine::ui::UserAddon) -> Result<(), String> {
    let table = Value::Table(vm.new_table());
    let bindings = addon.dir.join("Bindings.xml");
    if bindings.exists() {
        wowlua_engine::bindings::load_bindings_xml(vm, &bindings, Some(&addon.name))?;
    }
    wowlua_engine::bindings::add_user_addon(vm, addon, table, bindings.exists());
    if addon.on_demand {
        return Ok(());
    }
    for dep in &addon.deps {
        wowlua_engine::ui::load_addon(vm, dep);
    }
    wowlua_engine::ui::load_other_user_addon(vm, &addon.dir, &addon.toc, &addon.name, table);
    Ok(())
}

/// The TOC the client would read for the flavor: the flavor's own
/// (`<Name>_Mainline.toc`), else the plain one.
fn toc_path(build: &BuildOutput, flavor: &str) -> Option<String> {
    let tocs: Vec<&str> = build
        .files
        .keys()
        .filter(|p| !p.contains('/') && p.to_ascii_lowercase().ends_with(".toc"))
        .map(String::as_str)
        .collect();
    let suffixes = wowlua_flavors::table()
        .clients
        .iter()
        .find(|c| c.flavor == flavor)
        .map(|c| c.suffixes.clone())
        .unwrap_or_default();
    for suffix in suffixes {
        if let Some(t) = tocs.iter().find(|t| {
            t.to_ascii_lowercase()
                .ends_with(&format!("{}.toc", suffix.to_ascii_lowercase()))
        }) {
            return Some(t.to_string());
        }
    }
    let plain = format!("{}.toc", build.addon_name);
    tocs.iter()
        .find(|t| t.eq_ignore_ascii_case(&plain))
        .or(tocs.first())
        .map(|t| t.to_string())
}

/// The longest a single wait may take, in frames (a test's timeout).
fn timeout_of(vm: &mut Vm, tests: TableRef) -> f64 {
    let f = get(vm, tests, "timeout");
    vm.call(f, &[])
        .ok()
        .and_then(|r| match r.first() {
            Some(Value::Num(n)) => Some(*n),
            _ => None,
        })
        .unwrap_or(10.0)
}

const FRAME_MS: f64 = 1000.0 / 60.0;

/// Steps a job until it finishes (a reload outside a test fails it).
fn drive(vm: &mut Vm, tests: TableRef, job: Value) -> Outcome {
    drive_inner(vm, tests, job).unwrap_or_else(|| Outcome::Fail {
        message: "mock.reloadUI runs only in a test".into(),
        stack: String::new(),
    })
}

/// Steps a job until it finishes (`None`: at `mock.reloadUI`; resumed,
/// the job ends the pass and runs its teardown).
fn drive_inner(vm: &mut Vm, tests: TableRef, job: Value) -> Option<Outcome> {
    let step = get(vm, tests, "step");
    loop {
        let r = match vm.call(step, &[job]) {
            Ok(r) => r,
            Err(e) => {
                return Some(Outcome::Fail {
                    message: error_text(vm, &e),
                    stack: e.traceback.clone(),
                });
            }
        };
        let status = r.first().map(|&v| text(vm, v)).unwrap_or_default();
        match status.as_str() {
            "pass" => return Some(Outcome::Pass),
            "wait" => {
                let kind = r.get(1).map(|&v| text(vm, v)).unwrap_or_default();
                if kind == "reload" {
                    return None;
                }
                let amount = match r.get(2) {
                    Some(Value::Num(n)) => *n,
                    _ => 1.0,
                };
                let frames = if kind == "seconds" {
                    (amount * 1000.0 / FRAME_MS).ceil()
                } else {
                    amount.ceil()
                };
                let limit = timeout_of(vm, tests) * 1000.0 / FRAME_MS;
                if frames > limit.ceil() {
                    return Some(Outcome::Fail {
                        message: format!(
                            "a wait of {amount} {kind} is longer than the test's timeout"
                        ),
                        stack: String::new(),
                    });
                }
                let target = wowlua_engine::clock::now(vm) + amount;
                wowlua_engine::clock::advance_frames(vm, frames.max(0.0) as usize, FRAME_MS);
                // Frames of 1000/60 ms add up to a hair less than the
                // seconds asked for: the clock reaches them.
                let now = wowlua_engine::clock::now(vm);
                if kind == "seconds" && now < target {
                    wowlua_engine::clock::advance(vm, target - now);
                }
            }
            _ => {
                let message = r.get(1).map(|&v| text(vm, v)).unwrap_or_default();
                let stack = r.get(2).map(|&v| text(vm, v)).unwrap_or_default();
                return Some(Outcome::Fail { message, stack });
            }
        }
    }
}

fn start(
    vm: &mut Vm,
    tests: TableRef,
    kind: &str,
    i: Option<usize>,
    case: Option<usize>,
) -> Result<Value, String> {
    let f = get(vm, tests, "start");
    let kind = vm.str(kind);
    let args = [
        kind,
        i.map_or(Value::Nil, |i| Value::Num(i as f64)),
        case.map_or(Value::Nil, |c| Value::Num(c as f64)),
    ];
    match vm.call(f, &args) {
        Ok(r) => Ok(r.first().copied().unwrap_or(Value::Nil)),
        Err(e) => Err(error_text(vm, &e)),
    }
}

/// One planned test: its number, case and name.
struct Planned {
    index: usize,
    case: Option<usize>,
    name: String,
    skip: Option<String>,
}

fn plan(
    vm: &mut Vm,
    tests: TableRef,
    flavor: &str,
    file: &str,
    filter: Option<&str>,
) -> Vec<Planned> {
    let count = get(vm, tests, "count");
    let n = match vm.call(count, &[]).ok().and_then(|r| r.first().copied()) {
        Some(Value::Num(n)) => n as usize,
        _ => 0,
    };
    let info = get(vm, tests, "info");
    let case_name = get(vm, tests, "caseName");
    let flavor_v = vm.str(flavor);
    let mut planned = Vec::new();
    let mut any_only = false;
    let mut entries = Vec::new();
    for i in 1..=n {
        let Ok(r) = vm.call(info, &[Value::Num(i as f64), flavor_v]) else {
            continue;
        };
        let name = r.first().map(|&v| text(vm, v)).unwrap_or_default();
        let skip = match r.get(3) {
            Some(&v @ Value::Str(_)) => Some(text(vm, v)),
            _ => None,
        };
        let only = matches!(r.get(4), Some(Value::Bool(true)));
        let runs = matches!(r.get(5), Some(Value::Bool(true)));
        let cases = match r.get(6) {
            Some(Value::Num(c)) => *c as usize,
            _ => 0,
        };
        any_only |= only;
        entries.push((i, name, skip, only, runs, cases));
    }
    for (i, name, skip, only, runs, cases) in entries {
        if !runs || (any_only && !only) {
            continue;
        }
        let names: Vec<(Option<usize>, String)> = if cases == 0 {
            vec![(None, name)]
        } else {
            (1..=cases)
                .map(|c| {
                    let n = vm
                        .call(case_name, &[Value::Num(i as f64), Value::Num(c as f64)])
                        .ok()
                        .and_then(|r| r.first().copied())
                        .map(|v| text(vm, v))
                        .unwrap_or_default();
                    (Some(c), n)
                })
                .collect()
        };
        for (case, name) in names {
            if let Some(f) = filter
                && !name.contains(f)
                && !file.contains(f)
            {
                continue;
            }
            planned.push(Planned {
                index: i,
                case,
                name,
                skip: skip.clone(),
            });
        }
    }
    planned
}

/// The most passes a test may reload into.
const MAX_RELOADS: usize = 10;

fn failed(message: String) -> Outcome {
    Outcome::Fail {
        message,
        stack: String::new(),
    }
}

/// A test, with its passes after `mock.reloadUI`: each in a fresh VM with
/// the SavedVariables kept, the clock going on; the file's VM stays the
/// first pass's.
fn run_one(suite: &Suite, file: &TestFile, state: &mut Loaded, p: &Planned) -> Outcome {
    let tests = state.tests;
    let job = match start(&mut state.vm, tests, "test", Some(p.index), p.case) {
        Ok(job) => job,
        Err(e) => return failed(e),
    };
    if let Some(o) = drive_inner(&mut state.vm, tests, job) {
        return o;
    }
    // The first pass ends: its teardown runs in the file's VM. An addon's
    // `ReloadUI()` ended its session (`PLAYER_LOGOUT`, nothing running
    // after): the file's next tests go on in it as after `mock.reloadUI`.
    let mut saved = saved::capture(&mut state.vm, &state.saved);
    wowlua_engine::engine_mut(&mut state.vm).world.logged_out = false;
    let mut clock = (state.vm.clock.now, state.vm.clock.elapsed_ms);
    if let Some(o @ Outcome::Fail { .. }) = drive_inner(&mut state.vm, tests, job) {
        return o;
    }
    let mut pass = 0;
    let outcome = loop {
        pass += 1;
        if pass > MAX_RELOADS {
            break failed(format!(
                "the test reloaded the UI more than {MAX_RELOADS} times"
            ));
        }
        natives::keep_saved(saved.clone());
        let reload = Reload {
            pass,
            now: clock.0,
            elapsed_ms: clock.1,
        };
        let mut fresh = match load(suite, file, Some(&reload)) {
            Ok(l) => l,
            Err(e) => break failed(format!("reloading the UI: {e}")),
        };
        let pending = natives::unread_errors();
        if !pending.is_empty() {
            break failed(format!(
                "an error reached the error handler while reloading:\n{}",
                pending.join("\n")
            ));
        }
        let dropped = natives::unread_dropped();
        if !dropped.is_empty() {
            break failed(format!(
                "{DROPPED_FAILURE} while reloading:\n{}",
                dropped.join("\n")
            ));
        }
        let tests = fresh.tests;
        let vm = &mut fresh.vm;
        let ended = match start(vm, tests, "beforeAll", None, None) {
            Ok(job) => match drive(vm, tests, job) {
                Outcome::Fail { message, .. } => Some(failed(format!(
                    "@beforeAll failed after the reload: {message}"
                ))),
                _ => None,
            },
            Err(e) => Some(failed(e)),
        };
        let ended = ended.or_else(|| {
            let job = match start(vm, tests, "test", Some(p.index), p.case) {
                Ok(job) => job,
                Err(e) => return Some(failed(e)),
            };
            let done = drive_inner(vm, tests, job);
            if done.is_none() {
                saved = saved::capture(vm, &fresh.saved);
                clock = (vm.clock.now, vm.clock.elapsed_ms);
                if let Some(o @ Outcome::Fail { .. }) = drive_inner(vm, tests, job) {
                    return Some(o);
                }
            }
            done
        });
        if let Ok(job) = start(vm, tests, "afterAll", None, None) {
            let _ = drive(vm, tests, job);
        }
        if let Some(report) = coverage_of(suite, &fresh.vm) {
            state.reloaded_coverage.push(report);
        }
        if let Some(o) = ended {
            break o;
        }
    };
    // A reload's SavedVariables a manual lifecycle never logged in with.
    natives::drop_saved();
    outcome
}

fn run_tests(
    suite: &Suite,
    file: &TestFile,
    state: &mut Loaded,
    filter: Option<&str>,
    result: &mut FileResult,
) {
    let tests = state.tests;
    let vm = &mut state.vm;
    let planned = plan(vm, tests, &suite.flavor, &result.file, filter);
    if planned.is_empty() {
        return;
    }
    let fail_all = |result: &mut FileResult, planned: &[Planned], message: String| {
        for p in planned {
            result.tests.push(TestResult {
                name: p.name.clone(),
                outcome: Outcome::Fail {
                    message: message.clone(),
                    stack: String::new(),
                },
                seconds: 0.0,
            });
        }
    };
    // Errors and refusals so far (loading, logging in) belong to no test:
    // they fail the file, Blizzard's code's included (it runs without an
    // error in the client).
    let _ = natives::unread_refusals(vm);
    let pending = natives::unread_errors();
    if !pending.is_empty() {
        result.error = Some(to_sources(
            suite,
            &format!(
                "an error reached the error handler while loading:\n{}",
                pending.join("\n")
            ),
        ));
        return;
    }
    let dropped = natives::unread_dropped();
    if !dropped.is_empty() {
        result.error = Some(to_sources(
            suite,
            &format!("{DROPPED_FAILURE} while loading:\n{}", dropped.join("\n")),
        ));
        return;
    }
    match start(vm, tests, "beforeAll", None, None) {
        Ok(job) => {
            if let Outcome::Fail { message, .. } = drive(vm, tests, job) {
                fail_all(
                    result,
                    &planned,
                    to_sources(suite, &format!("@beforeAll failed: {message}")),
                );
                return;
            }
        }
        Err(e) => {
            fail_all(result, &planned, e);
            return;
        }
    }
    for p in &planned {
        if let Some(reason) = &p.skip {
            result.tests.push(TestResult {
                name: p.name.clone(),
                outcome: Outcome::Skip(reason.clone()),
                seconds: 0.0,
            });
            continue;
        }
        let started = Instant::now();
        let mut outcome = run_one(suite, file, state, p);
        let vm = &mut state.vm;
        // An error no test read (`mock.errors()`) fails the test, and so
        // does a protected action refused because of taint (with its taint
        // chain) that no test expected (`toBeBlocked`, `mock.refusals()`).
        let unread = natives::unread_errors();
        let dropped = natives::unread_dropped();
        let refused = natives::unread_refusals(vm);
        if outcome == Outcome::Pass && !unread.is_empty() {
            outcome = Outcome::Fail {
                message: format!("an error reached the error handler: {}", unread.join("\n")),
                stack: String::new(),
            };
        } else if outcome == Outcome::Pass && !dropped.is_empty() {
            outcome = Outcome::Fail {
                message: format!("{DROPPED_FAILURE}: {}", dropped.join("\n")),
                stack: String::new(),
            };
        } else if outcome == Outcome::Pass && !refused.is_empty() {
            let (action, chain) = &refused[0];
            outcome = Outcome::Fail {
                message: format!("the client refused {action} because of taint:\n{chain}"),
                stack: String::new(),
            };
        }
        if let Outcome::Fail { message, stack } = outcome {
            outcome = Outcome::Fail {
                message: to_sources(suite, &message),
                stack: to_sources(suite, &stack),
            };
        }
        result.tests.push(TestResult {
            name: p.name.clone(),
            outcome,
            seconds: started.elapsed().as_secs_f64(),
        });
    }
    let vm = &mut state.vm;
    if let Ok(job) = start(vm, tests, "afterAll", None, None)
        && let Outcome::Fail { message, .. } = drive(vm, tests, job)
    {
        result.error = Some(to_sources(suite, &format!("@afterAll failed: {message}")));
    }
}

/// Where test builds go: `<out>/.test/<flavor>/<Name>`.
pub fn build_dir(out: &Path, flavor: &str, addon: &str) -> PathBuf {
    out.join(".test").join(flavor).join(addon)
}
