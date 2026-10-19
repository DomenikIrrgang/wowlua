//! `/stdtest engine fullprobe` (tests/ingame/src/fullprobe.wowlua) run in
//! the engine with a tiny plan: it runs a few calls a frame, reloads after
//! each segment, resumes after the reload, runs a crashed segment again in
//! smaller steps and skips the call that crashed (and its function's other
//! calls), and puts every setting back. A session is a fresh VM given the
//! saved variables the last one wrote (a crash writes none).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use wowlua_vm::{Value, Vm};

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// The runner as an addon `StdTest` for camelot: `StdTestFullProbe(arg)`,
/// `StdTestFullProbeReplay(keys)`.
fn build() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = r#"{ "package": { "name": "std-test", "version": "0.1.0" },
  "addon": { "saved-variables": ["StdTestState"] },
  "flavors": { "camelot": {} } }
"#;
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    for file in [
        "fullprobe.wowlua",
        "fullprobeplan.lua",
        "fullprobeplan.d.wowlua",
    ] {
        fs::copy(repo().join("tests/ingame/src").join(file), src.join(file)).unwrap();
    }
    fs::write(
        src.join("state.wowlua"),
        "export function state(): table\n    if type(StdTestState) ~= \"table\" then StdTestState = {} end\n    return StdTestState\nend\n\
         export function stamp(part: string)\n    local st = state() :: any\n    st.engineStamps = st.engineStamps or {}\n    st.engineStamps[part] = { at = date(\"%Y-%m-%d %H:%M:%S\") }\nend\n",
    )
    .unwrap();
    fs::write(
        src.join("harness.wowlua"),
        "export function say(...: any)\n    print(...)\nend\n",
    )
    .unwrap();
    fs::write(
        src.join("main.wowlua"),
        "import fullProbe, replay, literalCalls from \"/fullprobe\"\n_G.StdTestFullProbe = fullProbe\n_G.StdTestFullProbeReplay = replay\n_G.StdTestFullProbeLiteral = literalCalls\n",
    )
    .unwrap();
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir.path())
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", dir.path().join(".wowlua-home"))
        .env_remove("WOWLUA_WOW_PATH")
        .env_remove("WOWLUA_STDLIB_DIR")
        .args(["build", "--profile", "dev"])
        .assert()
        .success();
    dir
}

fn text(vm: &Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        other => format!("{other:?}"),
    }
}

/// Runs Lua as StdTest's code; its first result as text.
fn run(vm: &mut Vm, src: &str) -> String {
    let f = vm
        .load_as(
            src.as_bytes(),
            "@Interface/AddOns/StdTest/test.lua",
            "StdTest",
        )
        .expect("compiles");
    match vm.call(f, &[]) {
        Ok(r) => r.first().map(|v| text(vm, *v)).unwrap_or_default(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// The tiny plan, the functions it calls and the setting that outlives a
/// session (as a CVar would): `StdTestFake`, given by the test.
const SETUP: &str = r#"
StdTestFullProbePlan = {
    segmentSize = 3,
    calls = {
        { "StdTestEcho", "1", "\"a\"", "{NOSUCHVALUE}" },
        { "StdTestCrash", "", "1" },
        { "StdTestEcho", "2", "3" },
    },
    setters = {
        { get = "StdTestGetFake()", restore = "function(o) StdTestSetFake(o) end",
          sets = { "StdTestSetFake(\"one\")", "StdTestSetFakeAndCrash(\"two\")", "StdTestSetFake(\"three\")" } },
    },
    skipped = { { "Nothing", "a test" } },
}
function StdTestEcho(...) return ... end
function StdTestCrash() StdTestCrashedNow = true return "survived" end
function StdTestGetFake() return StdTestFake end
function StdTestSetFake(v) StdTestFake = v end
function StdTestSetFakeAndCrash(v) StdTestFake = v StdTestCrashedNow = true end
"#;

/// How a session ended.
#[derive(Debug, PartialEq)]
enum End {
    /// Reloaded or logged out: the saved variables written.
    Saved,
    Crashed,
    /// Still running after the frames given.
    Running,
}

struct Client {
    out: PathBuf,
    /// The plan and functions given each session.
    setup: String,
    /// The saved variables the last session wrote (`return {…}`).
    saved: String,
    /// The setting that outlives sessions and crashes.
    fake: String,
    sessions: usize,
    crashes: usize,
}

const SERIALIZE: &str = r#"
local function ser(v)
    local t = type(v)
    if t == "string" then return string.format("%q", v) end
    if t == "number" or t == "boolean" then return tostring(v) end
    if t ~= "table" then return "nil" end
    local parts = {}
    for k, x in pairs(v) do
        local tk = type(k)
        if (tk == "string" or tk == "number") and (type(x) ~= "function" and type(x) ~= "userdata") then
            parts[#parts + 1] = "[" .. ser(k) .. "]=" .. ser(x)
        end
    end
    return "{" .. table.concat(parts, ",") .. "}"
end
return "return " .. ser(StdTestState)
"#;

impl Client {
    fn new(out: PathBuf, setup: &str) -> Client {
        Client {
            out,
            setup: setup.to_string(),
            saved: "return {}".into(),
            fake: "\"orig\"".into(),
            sessions: 0,
            crashes: 0,
        }
    }

    /// A session: StdTest loaded with the saved variables, logged in
    /// (`initial`, or a reload), `command` typed, up to `frames` frames.
    fn session(&mut self, initial: bool, command: Option<&str>, frames: usize) -> (Vm, End) {
        self.sessions += 1;
        let mut vm = Vm::wow();
        wowlua_engine::install(&mut vm, "camelot").expect("installs");
        wowlua_engine::controls::install(&mut vm, "__engine");
        run(
            &mut vm,
            &format!(
                "StdTestState = (function() {} end)()\nStdTestFake = {}\n{}",
                self.saved, self.fake, self.setup
            ),
        );
        let toc = fs::read_to_string(self.out.join("StdTest_Camelot.toc"))
            .or_else(|_| fs::read_to_string(self.out.join("StdTest.toc")))
            .expect("a TOC");
        let ns = Value::Table(vm.new_table());
        let name = vm.str("StdTest");
        for line in toc
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        {
            let path = line.split(" [").next().unwrap().replace('\\', "/");
            if !path.ends_with(".lua") {
                continue;
            }
            let code = fs::read(self.out.join(&path)).unwrap();
            let f = vm
                .load_as(
                    &code,
                    &format!("@Interface/AddOns/StdTest/{path}"),
                    "StdTest",
                )
                .expect("compiles");
            if let Err(e) = vm.call(f, &[name, ns]) {
                panic!("{path}: {}", text(&vm, e.value));
            }
        }
        let name = vm.str("StdTest");
        wowlua_engine::widgets::fire_event(&mut vm, "ADDON_LOADED", &[name, Value::Bool(false)]);
        wowlua_engine::widgets::fire_event(&mut vm, "PLAYER_LOGIN", &[]);
        wowlua_engine::widgets::fire_event(
            &mut vm,
            "PLAYER_ENTERING_WORLD",
            &[Value::Bool(initial), Value::Bool(!initial)],
        );
        if let Some(c) = command {
            run(&mut vm, &format!("StdTestFullProbe({c:?})"));
        }
        let end = self.frames(&mut vm, frames);
        (vm, end)
    }

    /// Frames until the session ends; a crash keeps the saved variables of
    /// before, an end writes them.
    fn frames(&mut self, vm: &mut Vm, frames: usize) -> End {
        for _ in 0..=frames {
            let crashed = run(vm, "return tostring(StdTestCrashedNow == true)") == "true";
            self.fake = run(vm, "return string.format('%q', tostring(StdTestFake))");
            if crashed {
                self.crashes += 1;
                return End::Crashed;
            }
            let ended = run(
                vm,
                "local fp = StdTestState.fullProbe return tostring(fp ~= nil and fp.endKind ~= nil)",
            ) == "true";
            if ended {
                self.saved = run(vm, SERIALIZE);
                return End::Saved;
            }
            wowlua_engine::clock::advance_frames(vm, 1, 1000.0 / 60.0);
        }
        End::Running
    }

    /// Sessions until the run is done: after a reload a reload's login,
    /// after a crash a fresh one.
    fn until_done(&mut self, mut end: End, sessions: usize) -> Vm {
        for _ in 0..sessions {
            let initial = end == End::Crashed;
            let (mut vm, e) = self.session(initial, None, 600);
            let done = run(
                &mut vm,
                "local fp = StdTestState.fullProbe return tostring(fp.done == true)",
            );
            if done == "true" && e == End::Saved {
                return vm;
            }
            assert_ne!(e, End::Running, "a session neither reloaded nor crashed");
            end = e;
        }
        panic!("the run never finished");
    }
}

/// `StdTestState.engineFullProbe` of a session.
fn results(vm: &mut Vm) -> BTreeMap<String, String> {
    let all = run(
        vm,
        "local out = {} for k, v in pairs(StdTestState.engineFullProbe or {}) do out[#out + 1] = k .. '\\0' .. v end table.sort(out) return table.concat(out, '\\1')",
    );
    all.split('\u{1}')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.split_once('\0'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn the_full_probe_resumes_skips_a_crash_and_restores_settings() {
    let dir = build();
    let mut client = Client::new(dir.path().join("dist/dev/StdTest"), SETUP);
    // The start: the values and settings read, then the first reload.
    let (_, end) = client.session(false, Some(""), 10);
    assert_eq!(end, End::Saved, "the start reloads");
    let mut vm = client.until_done(end, 60);
    let r = results(&mut vm);
    assert_eq!(
        r.get("StdTestEcho(1)").map(String::as_str),
        Some("1: number 1")
    );
    assert_eq!(
        r.get("StdTestEcho(\"a\")").map(String::as_str),
        Some("1: string a")
    );
    assert_eq!(
        r.get("StdTestEcho({NOSUCHVALUE})").map(String::as_str),
        Some("unresolved: {NOSUCHVALUE}")
    );
    assert_eq!(r.get("StdTestCrash()").map(String::as_str), Some("crashed"));
    assert_eq!(
        r.get("StdTestCrash(1)").map(String::as_str),
        Some("skipped: StdTestCrash crashed the client on StdTestCrash()")
    );
    assert_eq!(
        r.get("StdTestEcho(3)").map(String::as_str),
        Some("1: number 3")
    );
    let set = |s: &str| {
        format!("set {s} || get StdTestGetFake() || restore function(o) StdTestSetFake(o) end")
    };
    assert_eq!(
        r.get(&set("StdTestSetFake(\"one\")")).map(String::as_str),
        Some("was: 1: string orig | set: 0:  | read: 1: string one | restored: ok")
    );
    assert_eq!(
        r.get(&set("StdTestSetFakeAndCrash(\"two\")"))
            .map(String::as_str),
        Some("crashed")
    );
    assert!(
        r.get(&set("StdTestSetFake(\"three\")"))
            .is_some_and(|v| v.ends_with("restored: ok")),
        "{r:?}"
    );
    // Two crashes for each crashing call: in its segment, then alone.
    assert_eq!(client.crashes, 4, "the crashes");
    // The setting the crash left changed is put back.
    assert_eq!(client.fake, "\"orig\"");
    let summary = run(
        &mut vm,
        "local fp = StdTestState.fullProbe return #fp.crashed .. ' ' .. tostring(fp.running) .. ' ' .. fp.total .. ' ' .. tostring(next(fp.restoreFailures))",
    );
    assert_eq!(summary, "2 false 10 nil");
    // The engine answers recorded keys as the run did.
    let replayed = run(
        &mut vm,
        &format!(
            "local r = StdTestFullProbeReplay({{ 'StdTestEcho(1)', {:?} }}) return r['StdTestEcho(1)'] .. '|' .. r[{:?}]",
            set("StdTestSetFake(\"one\")"),
            set("StdTestSetFake(\"one\")")
        ),
    );
    assert_eq!(
        replayed,
        "1: number 1|was: 1: string orig | set: 0:  | read: 1: string one | restored: ok"
    );
    // A stamp for ingest.
    assert_ne!(
        run(
            &mut vm,
            "return tostring(StdTestState.engineStamps.fullprobe)"
        ),
        "nil"
    );
}

#[test]
fn the_full_probe_runs_a_few_calls_a_frame_after_a_planned_reload() {
    let dir = build();
    let calls: Vec<String> = (1..=90).map(|i| format!("\"{i}\"")).collect();
    let setup = format!(
        "StdTestFullProbePlan = {{ segmentSize = 100, calls = {{ {{ \"StdTestEcho\", {} }} }}, setters = {{}}, skipped = {{}} }}\n\
         function StdTestEcho(...) return ... end",
        calls.join(", ")
    );
    let mut client = Client::new(dir.path().join("dist/dev/StdTest"), &setup);
    let (_, end) = client.session(false, Some(""), 10);
    assert_eq!(end, End::Saved, "the start reloads");
    // A fresh login that follows no crash (the last session wrote its
    // variables: a logout) reloads before it runs anything.
    let planned = client.saved.clone();
    client.saved = planned.replace("[\"endKind\"]=\"planned\"", "[\"endKind\"]=\"other\"");
    assert_ne!(client.saved, planned, "the saved variables name the reload");
    let (mut vm, end) = client.session(true, None, 600);
    assert_eq!(end, End::Saved);
    assert!(results(&mut vm).is_empty(), "nothing ran before the reload");
    // After the planned reload: at most 40 calls a frame, all 90 in one
    // segment.
    let (mut vm, end) = client.session(false, None, 0);
    assert_eq!(end, End::Running);
    let cursor = |vm: &mut Vm| -> usize {
        run(vm, "return tostring(StdTestState.fullProbe.cursor)")
            .parse()
            .unwrap()
    };
    let mut steps = Vec::new();
    let mut last = cursor(&mut vm);
    for _ in 0..200 {
        wowlua_engine::clock::advance_frames(&mut vm, 1, 1000.0 / 60.0);
        let now = cursor(&mut vm);
        if now != last {
            steps.push(now - last);
            last = now;
        }
        if run(
            &mut vm,
            "return tostring(StdTestState.fullProbe.done == true)",
        ) == "true"
        {
            break;
        }
    }
    assert_eq!(steps, vec![40, 40, 10], "calls a frame");
    let r = results(&mut vm);
    assert_eq!(r.len(), 90);
    assert_eq!(
        r.get("StdTestEcho(90)").map(String::as_str),
        Some("1: number 90")
    );
    // The commands.
    run(&mut vm, "StdTestFullProbe('status')");
    run(&mut vm, "StdTestFullProbe('reset')");
    assert_eq!(
        run(&mut vm, "return tostring(StdTestState.fullProbe)"),
        "nil"
    );
    run(&mut vm, "StdTestFullProbe('stop')");
}

/// The generated plan runs to its end in the engine: every call compiles,
/// none ends the session, every setting is put back. `GetCurrentEnvironment`
/// answers a small table here: formatting `_G` after the plan's earlier
/// calls makes the VM's collector meet a stale function handle (a panic in
/// heap.rs, "stale handle … Function"; not reproduced alone: an engine bug
/// left open).
#[test]
fn the_generated_plan_runs_in_the_engine() {
    let dir = build();
    let mut client = Client::new(
        dir.path().join("dist/dev/StdTest"),
        "StdTestFullProbePlan = nil GetCurrentEnvironment = function() return {} end",
    );
    let (_, end) = client.session(false, Some(""), 10);
    assert_eq!(end, End::Saved, "the start reloads");
    let started = std::time::Instant::now();
    let mut vm = client.until_done(end, 400);
    let r = results(&mut vm);
    let syntax: Vec<&String> = r
        .iter()
        .filter(|(_, v)| *v == "syntax")
        .map(|(k, _)| k)
        .collect();
    assert!(syntax.is_empty(), "calls that do not compile: {syntax:?}");
    let summary = run(
        &mut vm,
        "local fp = StdTestState.fullProbe local n = 0 for _ in pairs(fp.restoreFailures) do n = n + 1 end return #fp.crashed .. ' ' .. n .. ' ' .. fp.total",
    );
    let unresolved = r.values().filter(|v| v.starts_with("unresolved")).count();
    let errors = r.values().filter(|v| v.starts_with("error")).count();
    eprintln!(
        "{} answers ({unresolved} unresolved, {errors} errors) in {} sessions, {:.1} s; crashed, not restored, calls: {summary}",
        r.len(),
        client.sessions,
        started.elapsed().as_secs_f64()
    );
    assert!(summary.starts_with("0 0 "), "{summary}");
}
