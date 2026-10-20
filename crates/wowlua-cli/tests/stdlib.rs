//! The standard library (spec chapter 8) at run time: addons that import
//! `wowlua:*` modules are built and run against a model of the WoW client
//! (`wowmock.wowlua`), driven by a test script, in wowlua's own VM
//! (`wowlua-vm`, the client's Lua). Every runtime test runs the addon in
//! three builds (dev, release, and release fully minified with the
//! libraries minified too), which must behave alike.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use wowlua_vm::{Value, Vm};

/// The model of the client, compiled like a test snippet.
const MOCK: &str = include_str!("wowmock.wowlua");

/// A compiled snippet: the runtime (when its code needs one) and its Lua.
#[derive(Clone)]
struct Snippet {
    runtime: Option<String>,
    lua: String,
}

/// Compiles a test snippet (a setup or a script driving the mock client)
/// as wowlua, so it is type-checked like any wowlua code: an error fails
/// the test with its diagnostics. Snippets are plain scripts (no imports);
/// `Mock` is typed by `wowmock.d.wowlua`. Compiled once per source.
fn wowlua_snippet(source: &str) -> Snippet {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Snippet>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(lua) = cache.lock().unwrap().get(source) {
        return lua.clone();
    }
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("wowlua.json"),
        // Dev's line numbers and checks, without anything that needs the
        // runtime (snippets run on their own).
        "{ \"package\": { \"name\": \"snippet\", \"version\": \"1.0.0\" },\n  \"profiles\": { \"snippet\": { \"inherits\": \"dev\", \"instrument\": \"none\", \"runtime-checks\": \"none\", \"dev-console\": false, \"monitor\": false, \"capture-errors\": false } } }\n",
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/mock.d.wowlua"),
        include_str!("wowmock.d.wowlua"),
    )
    .unwrap();
    fs::write(dir.path().join("src/main.wowlua"), source).unwrap();
    let mut session = wowlua_driver::Session::new();
    let options = wowlua_driver::LoadOptions {
        manifest: None,
        cwd: dir.path().to_path_buf(),
        ..Default::default()
    };
    let project = wowlua_driver::load_projects(&mut session, &options)
        .and_then(|ps| ps.into_iter().next())
        .expect("the snippet project loads");
    let mut request = wowlua_driver::BuildRequest::new("snippet");
    request.write = false;
    let built = wowlua_driver::build(&mut session, &project, &request);
    let renderer = wowlua_diagnostics::Renderer::new(
        wowlua_diagnostics::MessageFormat::Short,
        wowlua_diagnostics::ColorChoice::Never,
    );
    let errors: Vec<String> = session
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| renderer.render_to_string(&session.db, d))
        .collect();
    assert!(
        errors.is_empty(),
        "the test snippet does not compile:\n{}\n{source}",
        errors.join("\n")
    );
    let built = built.expect("the snippet builds");
    let runtime = match built.files.get("wowlua/runtime.lua") {
        Some(wowlua_driver::build::Content::Bytes(b)) => {
            Some(String::from_utf8(b.clone()).unwrap())
        }
        _ => None,
    };
    let lua = match built.files.get("src/main.lua") {
        Some(wowlua_driver::build::Content::Bytes(b)) => String::from_utf8(b.clone()).unwrap(),
        _ => panic!(
            "no src/main.lua: {:?}",
            built.files.keys().collect::<Vec<_>>()
        ),
    };
    let snippet = Snippet { runtime, lua };
    cache
        .lock()
        .unwrap()
        .insert(source.to_string(), snippet.clone());
    snippet
}

/// A VM error's message.
fn message(vm: &Vm, e: &wowlua_vm::LuaError) -> String {
    match e.value {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        other => format!("{other:?}"),
    }
}

/// Runs a chunk of `code` named `name` with `args`, as the client runs an
/// addon's file.
fn exec(vm: &mut Vm, code: &[u8], name: &str, args: &[Value]) -> Result<(), String> {
    let f = vm.load(code, name)?;
    vm.call(f, args).map(|_| ()).map_err(|e| message(vm, &e))
}

/// Runs a snippet in `vm` (its runtime first, if any), with an addon table
/// of its own, as the client runs an addon's files.
fn exec_snippet(vm: &mut Vm, source: &str, name: &str) -> Result<(), String> {
    let snippet = wowlua_snippet(source);
    let ns = Value::Table(vm.new_table());
    let label = vm.str("Snippet");
    if let Some(runtime) = &snippet.runtime {
        exec(
            vm,
            runtime.as_bytes(),
            &format!("{name} (runtime)"),
            &[label, ns],
        )?;
    }
    exec(vm, snippet.lua.as_bytes(), name, &[label, ns])
}

/// A global string (`Mock.saved`) read back, by an expression.
fn eval_string(vm: &mut Vm, expr: &str) -> String {
    let f = vm
        .load(format!("return {expr}").as_bytes(), "=eval")
        .expect("compiles");
    match vm.call(f, &[]) {
        Ok(r) => match r.first() {
            Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(*s)).into_owned(),
            other => panic!("{expr}: {other:?}"),
        },
        Err(e) => panic!("{expr}: {}", message(vm, &e)),
    }
}

/// The builds every runtime test runs in.
const MODES: &[&str] = &["dev", "release", "max"];

fn wowlua(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", dir.join(".wowlua-home"))
        .env_remove("WOWLUA_WOW_PATH")
        .env_remove("WOWLUA_STDLIB_DIR");
    cmd
}

/// A package `Std` with these sources; returns its directory.
fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = r#"{ "package": { "name": "std", "version": "1.0.0" },
  "addon": { "saved-variables": ["StdDB", "StdOtherDB"], "saved-variables-per-character": ["StdDBPerChar", "StdCharDB"] },
  "profiles": { "max": { "inherits": "release", "minify": "max", "minify-libraries": true } } }
"#;
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    // The mock client's controls, for checks inside the addon.
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/mock.d.wowlua"),
        include_str!("wowmock.d.wowlua"),
    )
    .unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

/// Builds the project with `profile`; returns the build's stderr.
fn build(dir: &Path, profile: &str) -> String {
    let assert = wowlua(dir)
        .args(["build", "--profile", profile])
        .assert()
        .success();
    String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
}

/// Runs `setup` (before the addon's files, as WoW loads SavedVariables),
/// loads the built addon into the mock client and runs `script`; returns
/// what was printed, one line each.
fn run(dir: &Path, profile: &str, setup: &str, script: &str) -> Vec<String> {
    let out = dir.join("dist").join(profile).join("Std");
    load(
        &out,
        "Std",
        "Std.toc",
        &format!("{profile}: "),
        setup,
        script,
    )
}

/// Loads the files `toc` (in AddOn folder `out`) lists into the mock
/// client as addon `name`, between `setup` and `script`.
fn load(out: &Path, name: &str, toc: &str, label: &str, setup: &str, script: &str) -> Vec<String> {
    let mut vm = client(out, name, toc, label, setup);
    exec_snippet(&mut vm, script, "@test").unwrap_or_else(|e| panic!("{label}{e}"));
    printed(&mut vm)
}

/// A mock client with `setup` run and the addon's files loaded.
fn client(out: &Path, name: &str, toc: &str, label: &str, setup: &str) -> Vm {
    let toc = fs::read_to_string(out.join(toc)).unwrap();
    let mut vm = Vm::wow();
    // The engine (every documented function), with the mock's Lua on top
    // where it still stands in.
    wowlua_engine::install(&mut vm, "standard").expect("the engine installs");
    wowlua_engine::controls::install(&mut vm, "__engine");
    exec_snippet(&mut vm, MOCK, "@wowmock.wowlua").unwrap();
    exec_snippet(&mut vm, setup, "@setup").unwrap_or_else(|e| panic!("{label}setup: {e}"));
    let ns = Value::Table(vm.new_table());
    let addon = vm.str(name);
    for line in toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let path = line.split(" [").next().unwrap().replace('\\', "/");
        if !path.ends_with(".lua") {
            continue;
        }
        let code = fs::read(out.join(&path)).unwrap();
        exec(&mut vm, &code, &format!("@{path}"), &[addon, ns])
            .unwrap_or_else(|e| panic!("{label}{path}: {e}"));
    }
    // The client reads Bindings.xml from the AddOn root: each body runs
    // with `keystate`, key releases only with `runOnUp`.
    if let Ok(xml) = fs::read_to_string(out.join("Bindings.xml")) {
        for tag in xml.split("<Binding ").skip(1) {
            let (attrs, rest) = tag.split_once('>').expect("a binding tag");
            let body = rest.split("</Binding>").next().unwrap_or("");
            let binding = attrs.split('"').nth(1).expect("a binding name");
            let on_up = attrs.contains("runOnUp=\"true\"");
            let code = format!(
                "Mock.addBinding({binding:?}, function(keystate) if keystate == 'up' and not {on_up} then return end {body} end)"
            );
            exec(&mut vm, code.as_bytes(), "@Bindings.xml", &[])
                .unwrap_or_else(|e| panic!("{label}Bindings.xml: {e}"));
        }
    }
    vm
}

/// What the mock client printed, one line each.
fn printed(vm: &mut Vm) -> Vec<String> {
    let f = vm.load(b"return Mock.out", "=printed").expect("compiles");
    let r = vm
        .call(f, &[])
        .unwrap_or_else(|e| panic!("{}", message(vm, &e)));
    let Some(Value::Table(t)) = r.first().copied() else {
        panic!("Mock.out is not a table")
    };
    let n = vm.heap.table(t).border();
    (1..=n)
        .map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("{other:?}"),
        })
        .collect()
}

fn is_error(line: &str) -> bool {
    line.contains("ERROR:") || line.contains("error captured")
}

/// The printed lines without error reports (dev builds also capture errors
/// in the dev console and name the failing handler).
fn plain(lines: &[String]) -> Vec<String> {
    lines.iter().filter(|l| !is_error(l)).cloned().collect()
}

/// Builds the addon in every mode and runs it; every mode must print the
/// same and report the same number of errors, and no `Mock.check` may
/// fail. Returns the dev build's output.
fn modes(files: &[(&str, &str)], setup: &str, script: &str) -> Vec<String> {
    modes_in(&project(files), setup, script)
}

/// [`modes`] for a prepared project.
fn modes_in(dir: &tempfile::TempDir, setup: &str, script: &str) -> Vec<String> {
    let mut outputs: Vec<(&str, Vec<String>)> = Vec::new();
    for mode in MODES {
        build(dir.path(), mode);
        outputs.push((mode, run(dir.path(), mode, setup, script)));
    }
    let (_, dev) = &outputs[0];
    for (mode, out) in &outputs {
        let failed: Vec<&String> = out.iter().filter(|l| l.starts_with("FAIL")).collect();
        assert!(failed.is_empty(), "{mode}: {failed:?}\n{out:?}");
        assert_eq!(plain(dev), plain(out), "dev and {mode} differ");
        let count = |o: &[String]| o.iter().filter(|l| l.contains("ERROR:")).count();
        assert_eq!(
            count(dev),
            count(out),
            "dev and {mode} report different errors:\n{dev:?}\n{out:?}"
        );
    }
    dev.clone()
}

/// The error reports (as the error handler received them) of an output.
fn errors(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix("ERROR: ").map(str::to_string))
        .collect()
}

// ----------------------------------------------------------------- errors --

#[test]
fn errors_prefix_rate_limit_and_listeners() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as errors from "wowlua:errors"
local C = Mock.check

-- Prefixed once, never twice.
errors.report("plain message")
errors.report("Std: already prefixed")
-- Non-string errors are converted.
errors.report({})
errors.report(Mock.secret())

-- The same message within the rate limit (10 s) is counted, not repeated.
errors.report("repeated")
errors.report("repeated")
errors.report("repeated")
_G.StdAgain = function() errors.report("repeated") end

-- Listeners see every report; their own errors go to the handler directly
-- and are not reported again.
local seen = 0
local sub = errors.onError(function(msg: string) seen += 1 end)
errors.onError(function(msg: string) error("listener broke") end)
errors.report("for listeners")
C(seen == 1, "listener called once")
sub:cancel()
errors.report("after cancel")
C(seen == 1, "cancelled listener not called")

-- call returns the results, or false; wrap drops results.
local ok, a, b = errors.call(function(x: number, y: number?) return x, y end, 1, nil)
C(ok == true and a == 1 and b == nil, "call returns results")
local failed = errors.call(function() error("in call") end)
C(failed == false, "call returns false on error")
local wrapped = errors.wrap(function(x: number) return x * 2 end)
C(wrapped(2) == nil, "wrap drops results")

errors.configure({ rateLimit = 0 })
errors.report("unlimited")
errors.report("unlimited")
_G.StdCount = errors.count
"#,
        )],
        "",
        r#"Mock.advance(11)
StdAgain()
-- Repeats held back by the rate limit are not counted.
print("count", StdCount())
"#,
    );
    let reported = errors(&out);
    let has = |m: &str| reported.iter().filter(|r| r.starts_with(m)).count();
    assert_eq!(has("Std: plain message"), 1, "{reported:?}");
    assert_eq!(has("Std: already prefixed"), 1, "{reported:?}");
    assert!(
        !reported.iter().any(|r| r.contains("Std: Std:")),
        "{reported:?}"
    );
    assert_eq!(has("Std: table: "), 1, "{reported:?}");
    assert_eq!(has("Std: <secret>"), 1, "{reported:?}");
    // Reported once, then after the window with the count of repeats.
    assert_eq!(has("Std: repeated"), 2, "{reported:?}");
    // (Debug builds name the module that reported it.)
    assert!(
        reported.contains(&"Std: repeated [main] (2 more)".to_string()),
        "{reported:?}"
    );
    // The broken listener's error reaches the handler for each of the six
    // reports after it subscribed, unprefixed and never reported again.
    assert_eq!(
        reported
            .iter()
            .filter(|r| r.contains("listener broke"))
            .count(),
        6,
        "{reported:?}"
    );
    assert!(
        reported.iter().any(|r| r.contains("in call")),
        "{reported:?}"
    );
    assert_eq!(has("Std: unlimited"), 2, "{reported:?}");
    assert!(out.contains(&"count 11".to_string()), "{out:?}");
}

/// Listeners get the component, the handler's owner and the stack; debug
/// builds append the module ID (from the error's location, else the stack,
/// or `context.module`) and the component to the report.
#[test]
fn errors_listeners_learn_component_owner_and_stack() {
    let out = modes(
        &[
            (
                "src/main.wowlua",
                r#"import * as errors from "wowlua:errors"
import on, send, watchUnit from "wowlua:events"
import hook from "wowlua:hooks"
import after from "wowlua:timer"
import * as callbacks from "wowlua:callbacks"
import fail from "./sub/thing"

local owner = { name = "owner" }
errors.onError(function(msg: string, info: errors.ErrorInfo)
    local stack = info.stack
    print("info", tostring(info.component), info.owner == owner, type(stack) == "string" and #stack > 0)
end)
-- One-parameter listeners still fit.
errors.onError(function(msg: string) end)

declare message STD_OWNED()
on("STD_OWNED", function() error("owned boom") end, owner)
send("STD_OWNED")
local t = { M = function(_self: any) end }
hook(t, "M", function() error("hook boom") end, { owner = owner })
t:M()
errors.report("direct", { component = "mine", owner = owner })
errors.report("named", { module = "sync" })
errors.report({})
local w = watchUnit({ "UNIT_HEALTH" }, function() error("watch boom") end, owner)
w:setUnit("player")
after(1, function() error("timer boom") end)
errors.call(fail)
interface Ev
    Boom: () -> ()
end
local registry: callbacks.Registry<Ev> = callbacks.registry()
registry:on("Boom", function() error("callback boom") end, owner)
registry:fire("Boom")
"#,
            ),
            (
                "src/sub/thing.wowlua",
                "export function fail()\n    error(\"sub boom\")\nend\n",
            ),
        ],
        "",
        r#"Mock.fire("UNIT_HEALTH", "player")
Mock.advance(1)
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "info events: STD_OWNED handler true true",
            "info hooks: M true true",
            "info mine true true",
            "info nil false true",
            "info nil false true",
            "info nil false true",
            "info callbacks: Boom true true",
            "info events: UNIT_HEALTH handler true true",
            "info timer: after false true",
        ],
        "{out:?}"
    );
    // The dev build's reports.
    let reported = errors(&out);
    let has = |text: &str, tail: &str| {
        reported
            .iter()
            .any(|r| r.contains(text) && r.ends_with(tail))
    };
    assert!(
        has("owned boom", " [main] [events: STD_OWNED handler]"),
        "{reported:?}"
    );
    assert!(has("hook boom", " [main] [hooks: M]"), "{reported:?}");
    // Without a location in the message, the module comes from the stack.
    assert!(has("Std: direct", "direct [main] [mine]"), "{reported:?}");
    assert!(has("Std: table: ", " [main]"), "{reported:?}");
    assert!(has("Std: named", "named [sync]"), "{reported:?}");
    assert!(has("sub boom", "sub boom [sub/thing]"), "{reported:?}");
    assert!(has("timer boom", " [main] [timer: after]"), "{reported:?}");
}

/// The isolated calls of events, hooks and timers allocate nothing when
/// the callback succeeds (the client's `xpcall` passes the arguments on);
/// a failing one is reported with its own context, also when nested.
#[test]
fn isolated_calls_allocate_nothing() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import on, onUnit, send, watchUnit from "wowlua:events"
import call, callIn from "wowlua:errors"
import hook from "wowlua:hooks"
import every, repeating from "wowlua:timer"
local C = Mock.check

declare message STD_ALLOC(a: number, b: number)
local n = 0
local function count(_a: any, _b: any)
    n += 1
end
on("STD_ALLOC", count)
on("STD_ALLOC", count)
-- The mock's own allocations (its `hooksecurefunc`, `Mock.fire`) are
-- measured on code without the standard library.
onUnit("UNIT_HEALTH", { "player" }, count, {})
local raw = CreateFrame("Frame")
raw:RegisterUnitEvent("UNIT_MAXHEALTH", "player")
raw:SetScript("OnEvent", count)
watchUnit({ "UNIT_LEVEL" }, count, {}):setUnit("player")
local rawLevel = CreateFrame("Frame")
rawLevel:RegisterUnitEvent("UNIT_NAME_UPDATE", "player")
rawLevel:SetScript("OnEvent", count)
local t = { M = function(_self: any, _a: any) end }
hook(t, "M", count)
local u = { M = function(_self: any, _a: any) end }
hooksecurefunc(u, "M", count)
_G.StdRawTicker = function(): TickerHandle
    return C_Timer.NewTicker(1, function() n += 1 end)
end
_G.StdTicker = function(): TickerHandle
    return every(1, function() n += 1 end)
end
_G.StdRepeating = function(): () -> ()
    local r = repeating(1, function() n += 1 end)
    r:start()
    return function() r:stop() end
end
local context = { component = "test" }

-- What `fn` allocates, run 100 times with the collector stopped.
local function garbage(fn: () -> ()): number
    fn()
    collectgarbage("collect")
    collectgarbage("stop")
    local before = collectgarbage("count")
    for _ = 1, 100 do
        fn()
    end
    local after = collectgarbage("count")
    collectgarbage("restart")
    return after - before
end
_G.StdGarbage = garbage

C(garbage(function() send("STD_ALLOC", 1, 2) end) == 0, "a message dispatch")
C(garbage(function() callIn(context, count, 1, 2) end) == 0, "callIn")
C(garbage(function() call(count, 1, 2) end) == 0, "call")
C(garbage(function() t:M(1) end) == garbage(function() u:M(1) end), "a hooked call")
local ok, a, b = callIn(context, function(x: number, y: number?) return x, y end, 1, nil)
C(ok == true and a == 1 and b == nil, "callIn returns the results")

declare message STD_FAIL()
on("STD_FAIL", function() error("handler boom") end)
on("STD_FAIL", function() print("next handler") end)
send("STD_FAIL")
t.M = function(_self: any) end
hook(t, "M", function() error("hook boom") end)
t:M()
callIn({ component = "outer" }, function()
    call(function() error("inner boom") end)
    error("outer boom")
end)
"#,
        )],
        "",
        r#"local C = Mock.check
local unit = StdGarbage(function() Mock.fire("UNIT_HEALTH", "player") end)
local rawUnit = StdGarbage(function() Mock.fire("UNIT_MAXHEALTH", "player") end)
C(unit == rawUnit, "a unit event dispatch")
local watched = StdGarbage(function() Mock.fire("UNIT_LEVEL", "player") end)
local rawWatched = StdGarbage(function() Mock.fire("UNIT_NAME_UPDATE", "player") end)
C(watched == rawWatched, "a watch's dispatch")
local ticker = StdRawTicker()
local rawTick = StdGarbage(function() Mock.advance(1) end)
ticker:Cancel()
ticker = StdTicker()
local tick = StdGarbage(function() Mock.advance(1) end)
-- (The engine's own timers vary by a few bytes.)
C(tick <= rawTick, "a ticker's tick")
ticker:Cancel()
local stop = StdRepeating()
local repeated = StdGarbage(function() Mock.advance(1) end)
C(repeated <= rawTick, "a repeating timer's tick")
stop()
"#,
    );
    assert!(out.contains(&"next handler".to_string()), "{out:?}");
    let reported = errors(&out);
    let one = |text: &str, context: &str| {
        reported
            .iter()
            .any(|r| r.starts_with("Std: ") && r.contains(text) && r.ends_with(context))
    };
    assert!(
        one("handler boom", " [main] [events: STD_FAIL handler]"),
        "{reported:?}"
    );
    assert!(one("hook boom", " [main] [hooks: M]"), "{reported:?}");
    // Without a context, only the module is named.
    assert!(one("inner boom", "inner boom [main]"), "{reported:?}");
    assert!(one("outer boom", " [outer]"), "{reported:?}");
}

// ----------------------------------------------------------------- events --

#[test]
fn events_dispatch_in_order_and_isolated() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import event, unitEvent, on, once, off, offAll, send from "wowlua:events"

@event("PLAYER_ENTERING_WORLD")
local function first(isInitialLogin: boolean, isReloadingUi: boolean)
    print("first", isInitialLogin, isReloadingUi)
end

@event("PLAYER_ENTERING_WORLD")
local function broken()
    error("boom")
end

@event("PLAYER_ENTERING_WORLD")
local function third()
    print("third")
end

@event("BAG_UPDATE", "BAG_UPDATE_DELAYED")
local function bags(name: string, ...: any)
    print("bags", name, ...)
end

@unitEvent("UNIT_HEALTH", "player")
local function playerHealth(unit: string)
    print("health", unit)
end

local owner = {}
on("UNIT_HEALTH", function(unit: string) print("any health", unit) end, owner)
once("PLAYER_LOGIN", function() print("login once") end)

declare message STD_PING(n: number)
local sub = on("STD_PING", function(n: number) print("ping", n) end)
_G.StdPing = function(n: number) send("STD_PING", n) end
_G.StdUnsub = function() off(sub) end
_G.StdOffAll = function() offAll(owner) end
"#,
        )],
        "",
        r#"Mock.fire("PLAYER_ENTERING_WORLD", true, false)
Mock.fire("BAG_UPDATE", 3)
Mock.fire("BAG_UPDATE_DELAYED")
Mock.fire("UNIT_HEALTH", "target")
Mock.fire("UNIT_HEALTH", "player")
Mock.fire("PLAYER_LOGIN")
Mock.fire("PLAYER_LOGIN")
StdPing(2)
StdUnsub()
StdPing(5)
StdOffAll()
Mock.fire("UNIT_HEALTH", "target")
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "first true false",
            "third",
            "bags BAG_UPDATE 3",
            "bags BAG_UPDATE_DELAYED",
            "any health target",
            // The unit handler and the plain one live on different frames,
            // which get events in the order they first registered one (the
            // client's): the plain handlers' frame first.
            "any health player",
            "health player",
            "login once",
            "ping 2",
        ],
        "{out:?}"
    );
    assert!(errors(&out)[0].contains("boom"), "{out:?}");
}

#[test]
fn events_registration_follows_the_handlers() {
    modes(
        &[(
            "src/main.wowlua",
            r#"import on, onUnit, send, watchUnit from "wowlua:events"
local C = Mock.check

-- The shared frame registers plain events (without a unit filter).
local function registered(name: string): boolean
    for _, f in ipairs(Mock.frames()) do
        local on, unit = f:IsEventRegistered(name)
        if on and unit == nil then return true end
    end
    return false
end

C(not registered("BAG_UPDATE"), "nothing registered before a handler")
local a = on("BAG_UPDATE", function() end)
local b = on("BAG_UPDATE", function() end)
C(registered("BAG_UPDATE"), "registered with a handler")
a:cancel()
C(registered("BAG_UPDATE"), "still registered with one handler left")
C(a.active == false and b.active == true, "subscription states")
b:cancel()
C(not registered("BAG_UPDATE"), "unregistered without handlers")
b:cancel()
C(not registered("BAG_UPDATE"), "cancelling twice is harmless")

-- Messages are never registered with the game.
declare message STD_LOCAL()
on("STD_LOCAL", function() end)
C(not registered("STD_LOCAL"), "a message is not registered")

-- Unit events: one frame per event and unit set, shared by equal sets.
local frames = #Mock.frames()
local u1 = onUnit("UNIT_HEALTH", { "player" }, function() end)
local u2 = onUnit("UNIT_HEALTH", { "player" }, function() end)
C(#Mock.frames() == frames + 1, "equal unit sets share a frame")
local u3 = onUnit("UNIT_HEALTH", { "target", "focus" }, function() end)
C(#Mock.frames() == frames + 2, "another unit set gets its own frame")
local frame = Mock.frames()[frames + 1]
C(select(2, frame:IsEventRegistered("UNIT_HEALTH")) == "player", "unit filter set")
u1:cancel()
C(frame:IsEventRegistered("UNIT_HEALTH"), "still registered for the second handler")
u2:cancel()
C(not frame:IsEventRegistered("UNIT_HEALTH"), "unregistered without handlers")
u3:cancel()

-- More than two units: a frame per pair (WoW filters by two at most).
frames = #Mock.frames()
local u4 = onUnit("UNIT_HEALTH", { "party1", "party2", "party3" }, function() end)
C(#Mock.frames() == frames + 2, "a frame per pair")
C(select(3, Mock.frames()[frames + 1]:IsEventRegistered("UNIT_HEALTH")) == "party2", "the first pair")
C(select(2, Mock.frames()[frames + 2]:IsEventRegistered("UNIT_HEALTH")) == "party3", "the unit left over")
u4:cancel()
C(not Mock.frames()[frames + 2]:IsEventRegistered("UNIT_HEALTH"), "every pair unregistered")
local none: string[] = {}
C(not pcall(onUnit, "UNIT_HEALTH", none, function() end), "no units is refused")
-- An event this client lacks (shared flavor mode): never called, no error.
local missing = "UNIT_" .. "MISSING"
C(pcall(onUnit, missing, { "player" }, function() end), "onUnit of a missing event")
C(pcall(function() watchUnit({ missing, "UNIT_HEALTH" }, function() end):setUnit("player") end), "a watch of a missing event")

-- Sending a WoW event is an error (a name the compiler cannot see).
local name = "BAG_" .. "UPDATE"
C(not pcall(send, name), "send refuses WoW events")
print("done")
"#,
        )],
        "",
        "",
    );
}

#[test]
fn events_changes_during_a_dispatch_apply_to_the_next() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r##"import on, once, off, send from "wowlua:events"

local later: any = nil
on("PLAYER_STARTED_MOVING", function()
    print("first")
    -- Added during the dispatch: runs from the next one.
    later = later or on("PLAYER_STARTED_MOVING", function() print("added") end)
end)
local second = on("PLAYER_STARTED_MOVING", function()
    print("second")
end)
on("PLAYER_STARTED_MOVING", function()
    -- Removed during the dispatch, after it ran.
    off(second)
end)
once("PLAYER_STARTED_MOVING", function()
    print("once")
end)

-- nil values in a payload arrive as sent.
declare message STD_HOLES(a: any, b: any, c: any)
on("STD_HOLES", function(...: any) print("holes", select("#", ...), ...) end)
_G.StdHoles = function() send("STD_HOLES", nil, 2, nil) end
"##,
        )],
        "",
        r#"Mock.fire("PLAYER_STARTED_MOVING")
Mock.fire("PLAYER_STARTED_MOVING")
StdHoles()
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "first",
            "second",
            "once",
            "first",
            "added",
            "holes 3 nil 2 nil"
        ],
        "{out:?}"
    );
}

/// A handler cancelled during a dispatch (by an earlier handler) is skipped
/// for the rest of it, on every frame of an `onUnit` subscription too.
#[test]
fn events_cancelled_during_a_dispatch_are_skipped() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import on, onUnit, offAll, send from "wowlua:events"
import type Subscription from "wowlua:events"

declare message STD_SKIP()
local later: Subscription? = nil
on("STD_SKIP", function()
    print("first")
    if later then
        later:cancel()
    end
end)
later = on("STD_SKIP", function() print("later") end)
on("STD_SKIP", function() print("last") end)
send("STD_SKIP")
send("STD_SKIP")

-- An owner's handlers, cancelled by offAll from an earlier handler.
local owner = {}
on("UNIT_HEALTH", function() offAll(owner) end)
on("UNIT_HEALTH", function(unit: string) print("owned", unit) end, owner)
onUnit("UNIT_HEALTH", { "player", "target", "focus" }, function(unit: string) print("unit", unit) end, owner)
"#,
        )],
        "",
        r#"Mock.fire("UNIT_HEALTH", "focus")
"#,
    );
    assert_eq!(plain(&out), ["first", "last", "first", "last"], "{out:?}");
}

/// `onUnit` and `@unitEvent` take any number of units; `offAll` takes a
/// name; a unit listed twice gets its events once.
#[test]
fn events_any_number_of_units_and_offall_by_name() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import on, onUnit, offAll, send, unitEvent, watchUnit from "wowlua:events"

@unitEvent("UNIT_LEVEL", "player", "target", "focus")
local function level(unit: string)
    print("level", unit)
end

local owner = {}
onUnit("UNIT_HEALTH", { "party1", "party2", "party3", "party1" }, function(unit: string) print("party", unit) end, owner)
on("BAG_UPDATE", function(bag: number) print("bag", bag) end, owner)
declare message STD_A()
on("STD_A", function() print("message") end, owner)
local w = watchUnit({ "UNIT_HEALTH", "UNIT_MAXHEALTH" }, function(event: string, unit: string) print("watch", event, unit) end, owner)
w:setUnit("party1")
_G.StdDropHealth = function() offAll(owner, "UNIT_HEALTH") end
_G.StdDropAll = function()
    offAll(owner)
    Mock.check(w.active == false, "offAll cancels the owner's watches")
end
_G.StdSend = function() send("STD_A") end
"#,
        )],
        "",
        r#"Mock.fire("UNIT_LEVEL", "focus")
Mock.fire("UNIT_HEALTH", "party1")
Mock.fire("UNIT_HEALTH", "party3")
StdDropHealth()
Mock.fire("UNIT_HEALTH", "party1")
Mock.fire("UNIT_MAXHEALTH", "party1")
Mock.fire("BAG_UPDATE", 2)
StdSend()
StdDropAll()
Mock.fire("UNIT_MAXHEALTH", "party1")
Mock.fire("BAG_UPDATE", 3)
StdSend()
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "level focus",
            "party party1",
            "watch UNIT_HEALTH party1",
            "party party3",
            // offAll(owner, name) leaves the owner's other events alone.
            "watch UNIT_MAXHEALTH party1",
            "bag 2",
            "message",
        ],
        "{out:?}"
    );
}

/// A watch registers its events on one frame of its own, re-pointed by
/// `setUnit`; a cancelled watch's frame serves the next watch.
#[test]
fn events_watch_unit_repoints_one_frame() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r##"import watchUnit from "wowlua:events"
local C = Mock.check

local frames = #Mock.frames()
local w = watchUnit({ "UNIT_HEALTH", "UNIT_MAXHEALTH" }, function(event: string, unit: string, ...: any)
    print("watch", event, unit, select("#", ...))
end)
C(#Mock.frames() == frames, "no frame before a unit")
w:setUnit("player")
C(#Mock.frames() == frames + 1, "one frame")
local frame = Mock.frames()[frames + 1]
C(select(2, frame:IsEventRegistered("UNIT_MAXHEALTH")) == "player", "every event registered")
_G.StdWatch = w
_G.StdCheck = function(step: string)
    if step == "repointed" then
        C(#Mock.frames() == frames + 1, "re-pointing keeps the frame")
        C(select(3, frame:IsEventRegistered("UNIT_HEALTH")) == "focus", "both units")
    elseif step == "cleared" then
        C(not frame:IsEventRegistered("UNIT_HEALTH"), "nil unregisters")
    elseif step == "cancelled" then
        C(w.active == false and not frame:IsEventRegistered("UNIT_HEALTH"), "cancelled")
        local again = watchUnit({ "UNIT_HEALTH" }, function() print("again") end)
        again:setUnit("player")
        C(#Mock.frames() == frames + 1, "a cancelled watch's frame is reused")
        C(select(2, frame:IsEventRegistered("UNIT_HEALTH")) == "player", "for the next watch")
        again:cancel()
    end
end
"##,
        )],
        "",
        r#"Mock.fire("UNIT_HEALTH", "player")
Mock.fire("UNIT_HEALTH", "target")
StdWatch:setUnit("target", "focus")
StdCheck("repointed")
Mock.fire("UNIT_MAXHEALTH", "focus")
Mock.fire("UNIT_HEALTH", "player")
StdWatch:setUnit(nil)
StdCheck("cleared")
Mock.fire("UNIT_HEALTH", "target")
StdWatch:setUnit("player")
StdWatch:cancel()
StdWatch:cancel()
StdWatch:setUnit("target")
StdCheck("cancelled")
Mock.fire("UNIT_HEALTH", "target")
"#,
    );
    assert_eq!(
        plain(&out),
        ["watch UNIT_HEALTH player 0", "watch UNIT_MAXHEALTH focus 0"],
        "{out:?}"
    );
}

#[test]
fn events_on_static_methods_and_with_owners() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import event, on, offAll from "wowlua:events"

class Tracker
    count: number = 0

    @event("PLAYER_LOGIN")
    static function onLogin()
        print("static login")
    end

    constructor()
        on("BAG_UPDATE", function(bag: number) self:bag(bag) end, self)
    end

    function bag(n: number)
        self.count += 1
        print("bag", n, self.count)
    end
end

local a, b = Tracker.new(), Tracker.new()
_G.StdDrop = function() offAll(a) end
"#,
        )],
        "",
        r#"Mock.fire("PLAYER_LOGIN")
Mock.fire("BAG_UPDATE", 1)
StdDrop()
Mock.fire("BAG_UPDATE", 2)
"#,
    );
    assert_eq!(
        plain(&out),
        ["static login", "bag 1 1", "bag 1 1", "bag 2 2"],
        "{out:?}"
    );
}

// -------------------------------------------------------------- lifecycle --

#[test]
fn lifecycle_runs_its_phases_in_order() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import onLoad, onLogin, onEnterWorld, onLogout, onAddonLoaded, isLoaded, isLoggedIn from "wowlua:lifecycle"

print("top", isLoaded(), isLoggedIn())

@onLoad
local function load1() print("load 1", isLoaded(), isLoggedIn()) end

@onLoad
local function loadFails() error("load failed") end

@onLoad
local function load2() print("load 2") end

@onLogin
local function login()
    print("login", isLoggedIn())
    onAddonLoaded("Other", function() print("already loaded") end)
end

@onEnterWorld
local function world(initial: boolean, reload: boolean) print("world", initial, reload) end

@onLogout
local function logout() print("logout") end

onAddonLoaded("Blizzard_Later", function() print("later loaded") end)
onAddonLoaded("Blizzard_Later", function() error("later failed") end)
onAddonLoaded("Blizzard_Later", function() print("later loaded too") end)
"#,
        )],
        "",
        r#"Mock.loadAddon("Other")
print("before")
Mock.loadAddon("Std")
-- Once only, whatever the client sends again.
Mock.loadAddon("Std")
Mock.fire("PLAYER_LOGIN")
Mock.fire("PLAYER_LOGIN")
Mock.fire("PLAYER_ENTERING_WORLD", true, false)
Mock.loadAddon("Blizzard_Later")
Mock.loadAddon("Blizzard_Later")
Mock.fire("PLAYER_ENTERING_WORLD", false, true)
Mock.fire("PLAYER_LOGOUT")
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "top false false",
            "before",
            "load 1 true false",
            "load 2",
            "login true",
            "already loaded",
            "world true false",
            "later loaded",
            "later loaded too",
            "world false true",
            "logout",
        ],
        "{out:?}"
    );
    let reported = errors(&out);
    assert_eq!(reported.len(), 2, "{out:?}");
    assert!(reported[0].contains("load failed"), "{out:?}");
    assert!(reported[1].contains("later failed"), "{out:?}");
}

#[test]
fn lifecycle_of_a_load_on_demand_addon() {
    // Loaded after login: @onLogin follows @onLoad at once, and the later
    // PLAYER_LOGIN of a reload does not repeat it.
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import onLoad, onLogin, isLoggedIn from "wowlua:lifecycle"

@onLoad
local function load() print("load", isLoggedIn()) end

@onLogin
local function login() print("login", isLoggedIn()) end
"#,
        )],
        "Mock.loggedIn = true",
        r#"Mock.loadAddon("Std")
Mock.fire("PLAYER_LOGIN")
"#,
    );
    assert_eq!(plain(&out), ["load false", "login true"], "{out:?}");
    // Logged in by the event before the addon loaded.
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import onLoad, onLogin from "wowlua:lifecycle"

@onLogin
local function login() print("login") end

@onLoad
local function load() print("load") end
"#,
        )],
        "",
        r#"Mock.fire("PLAYER_LOGIN")
Mock.loggedIn = false
print("loading")
Mock.loadAddon("Std")
"#,
    );
    assert_eq!(plain(&out), ["loading", "load", "login"], "{out:?}");
}

// ------------------------------------------------------------------ timer --

#[test]
fn timers_throttle_debounce_and_next_frame() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import after, every, throttle, debounce, nextFrame from "wowlua:timer"

local t = after(2, function() print("after 2") end)
local cancelled = after(1, function() print("never") end)
cancelled:Cancel()
Mock.check(cancelled:IsCancelled(), "cancelled")
every(1, function(ticker: any) print("tick", GetTime()) end, 3)

@throttle(1)
local function resize(w: number) print("resize", w, GetTime()) end

@debounce(0.5)
local function search(text: string) print("search", text, GetTime()) end

class Window
    name: string
    constructor(name: string) self.name = name end

    @nextFrame
    function refresh(n: number) print("refresh", self.name, n) end
end

local a, b = Window.new("a"), Window.new("b")
_G.StdT = { resize = resize, search = search, a = a, b = b }
nextFrame(function() print("next frame") end)
"#,
        )],
        "",
        r#"StdT.resize(1); StdT.resize(2); StdT.resize(3)
StdT.search("x"); Mock.advance(0.2); StdT.search("xy")
StdT.a:refresh(1); StdT.a:refresh(2); StdT.b:refresh(9)
Mock.advance(0)
Mock.advance(5)
"#,
    );
    assert_eq!(
        out,
        [
            "resize 1 0",
            // Same-time timers run in the order they were scheduled.
            "next frame",
            "refresh a 2",
            "refresh b 9",
            "search xy 0.7",
            "tick 1",
            "resize 3 1",
            "after 2",
            "tick 2",
            "tick 3",
        ],
        "{out:?}"
    );
}

/// `repeating`: stopped until started, the first tick one interval after
/// each start, a start while running changes nothing, errors isolated.
#[test]
fn timers_repeating_start_and_stop() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import repeating from "wowlua:timer"

local tick = repeating(1, function() print("tick", GetTime()) end)
Mock.check(not tick:isRunning(), "stopped at first")
local failed = 0
local broken = repeating(1, function()
    failed += 1
    error("tick boom")
end)
broken:start()
_G.StdTick = tick
_G.StdBroken = broken
_G.StdFailed = function(): number return failed end
"#,
        )],
        "",
        r#"StdTick:start()
Mock.advance(0.5)
StdTick:start()
Mock.advance(2)
StdTick:setRunning(false)
print("running", StdTick:isRunning())
Mock.advance(3)
StdTick:setRunning(true)
print("running", StdTick:isRunning())
Mock.advance(1)
StdTick:stop()
StdTick:stop()
StdBroken:stop()
Mock.advance(2)
print("failed", StdFailed())
"#,
    );
    // The failing timer kept ticking until stopped (at 6.5).
    assert_eq!(
        plain(&out),
        [
            "tick 1",
            "tick 2",
            "running false",
            "running true",
            "tick 6.5",
            "failed 6"
        ],
        "{out:?}"
    );
    assert!(errors(&out)[0].contains("tick boom"), "{out:?}");
}

#[test]
fn timers_edge_cases() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import after, every, throttle, debounce, nextFrame from "wowlua:timer"

-- A ticker gets itself, and can stop itself.
local n = 0
every(1, function(ticker: any)
    n += 1
    print("ticker", n)
    if n == 2 then ticker:Cancel() end
end)

-- Errors in timer callbacks are reported; the timers carry on.
after(0.5, function() error("timer failed") end)
after(0.5, function() print("after the failure") end)
local failing = 0
every(3, function()
    failing += 1
    error("ticker failed " .. failing)
end, 2)

-- Plain nextFrame calls are not coalesced.
nextFrame(function() print("frame a") end)
nextFrame(function() print("frame a") end)

-- @nextFrame on a function coalesces, and runs again after it ran.
@nextFrame
local function redraw(why: string) print("redraw", why) end

class Unit
    name: string
    constructor(name: string) self.name = name end

    -- Throttled per instance.
    @throttle(2)
    function update(v: number) print("update", self.name, v, GetTime()) end
end
local u1, u2 = Unit.new("u1"), Unit.new("u2")

-- Debounce restarts with every call.
@debounce(1)
local function save(v: number) print("save", v, GetTime()) end

-- A throttled trailing call that fails is reported.
@throttle(1)
local function risky(v: number)
    if v > 1 then error("risky " .. v) end
    print("risky", v)
end

_G.StdT = { redraw = redraw, u1 = u1, u2 = u2, save = save, risky = risky }
"#,
        )],
        "",
        r#"StdT.redraw("a"); StdT.redraw("b")
Mock.advance(0)
StdT.redraw("c")
Mock.advance(0)
StdT.u1:update(1); StdT.u2:update(1); StdT.u1:update(2)
StdT.save(1); Mock.advance(0.6); StdT.save(2); Mock.advance(0.6); StdT.save(3)
StdT.risky(1); StdT.risky(2)
Mock.advance(10)
-- After the window, a call runs at once again.
StdT.u1:update(3)
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "frame a",
            "frame a",
            "redraw b",
            "redraw c",
            "update u1 1 0",
            "update u2 1 0",
            // At 0.5 and 1, while the clock advances for `save`.
            "after the failure",
            "ticker 1",
            "risky 1",
            // Both due at 2: the ticker was scheduled first.
            "ticker 2",
            "update u1 2 2",
            "save 3 2.2",
            "update u1 3 11.2",
        ],
        "{out:?}"
    );
    let reported = errors(&out);
    assert_eq!(reported.len(), 4, "{out:?}");
    assert!(reported[0].contains("timer failed"), "{out:?}");
    assert!(reported[1].contains("risky 2"), "{out:?}");
    assert!(reported[2].contains("ticker failed 1"), "{out:?}");
    assert!(reported[3].contains("ticker failed 2"), "{out:?}");
}

// ----------------------------------------------------------------- combat --

#[test]
fn combat_queues_until_combat_ends() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import outOfCombat, defer, inCombat, onCombatChange from "wowlua:combat"

class Bar
    name: string
    constructor(name: string) self.name = name end

    @outOfCombat
    function layout(offset: number) print("layout", self.name, offset, inCombat()) end
end

local one, two = Bar.new("one"), Bar.new("two")
onCombatChange(function(entering: boolean) print("combat", entering) end)
_G.StdC = { one = one, two = two, defer = defer }
"#,
        )],
        "",
        r#"StdC.one:layout(1)
Mock.fire("PLAYER_REGEN_DISABLED")
StdC.one:layout(2)
StdC.two:layout(5)
StdC.defer(function() print("deferred") end)
StdC.one:layout(3)
Mock.fire("PLAYER_REGEN_ENABLED")
"#,
    );
    assert_eq!(
        out,
        [
            "layout one 1 false",
            "combat true",
            // Coalesced: one's first place, its latest arguments.
            "layout one 3 false",
            "layout two 5 false",
            "deferred",
            "combat false",
        ],
        "{out:?}"
    );
}

#[test]
fn combat_edge_cases() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import outOfCombat, defer, onCombatChange from "wowlua:combat"

@outOfCombat
local function apply(v: number) print("apply", v) end

@outOfCombat
local function fails() error("queued failure") end

local function step(name: string): () -> ()
    return function() print("step", name) end
end
local restart = function()
    print("restart")
    Mock.combat = true
end

local sub = onCombatChange(function(entering: boolean) print("combat", entering) end)
_G.StdC = { apply = apply, fails = fails, defer = defer, step = step, restart = restart, sub = sub }
"#,
        )],
        "",
        r#"-- Out of combat everything runs at once.
StdC.defer(StdC.step("now"))
StdC.apply(0)
Mock.fire("PLAYER_REGEN_DISABLED")
StdC.apply(1)
StdC.fails()
-- defer is not coalesced, even for one function.
local s = StdC.step("twice")
StdC.defer(s)
StdC.defer(s)
StdC.apply(2)
-- With a key: the first call's place, the latest function.
StdC.defer(StdC.step("key a1"), "a")
StdC.defer(StdC.step("key b"), "b")
StdC.defer(StdC.step("key a2"), "a")
-- Combat starts again in the middle of the flush: the rest waits.
StdC.defer(StdC.restart)
StdC.defer(StdC.step("after restart"))
StdC.sub:cancel()
Mock.fire("PLAYER_REGEN_ENABLED")
print("in combat again", Mock.combat)
-- Queued again after a flush (the coalescing entry is gone).
StdC.apply(3)
Mock.fire("PLAYER_REGEN_ENABLED")
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "step now",
            "apply 0",
            "combat true",
            "apply 2",
            "step twice",
            "step twice",
            "step key a2",
            "step key b",
            "restart",
            "in combat again true",
            "step after restart",
            "apply 3",
        ],
        "{out:?}"
    );
    let reported = errors(&out);
    assert_eq!(reported.len(), 1, "{out:?}");
    assert!(reported[0].contains("queued failure"), "{out:?}");
}

// ------------------------------------------------------------------ hooks --

#[test]
fn hooks_post_hook_globals_methods_and_scripts() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import hook, hookScript from "wowlua:hooks"

@hook("ToggleGameMenu")
local function afterMenu() print("after menu") end

@hook(GameTooltip, "SetUnit")
local function afterSetUnit(tip: any, unit: string) print("after SetUnit", unit) end

@hookScript("PlayerFrame", "OnShow")
local function shown(frame: any) print("shown", frame:GetName()) end

@hook("LateFrame", "Show", { addon = "Blizzard_Late" })
local function lateShown(frame: any) print("late shown") end
"#,
        )],
        "",
        r#"Mock.loadAddon("Std")
ToggleGameMenu()
GameTooltip:SetUnit("target")
PlayerFrame:Hide()
PlayerFrame:Show()
CreateFrame("Frame", "LateFrame")
Mock.loadAddon("Blizzard_Late")
LateFrame:Show()
"#,
    );
    assert_eq!(
        out,
        [
            "menu",
            "after menu",
            "SetUnit target",
            "after SetUnit target",
            "shown PlayerFrame",
            "late shown"
        ],
        "{out:?}"
    );
}

#[test]
fn hooks_edge_cases() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import hook, hookScript from "wowlua:hooks"

-- Plain forms; the hooked function keeps its results, and a failing hook
-- is reported without breaking the caller.
hook("StdAdd", function(a: number, b: number) print("added", a, b) end)
hook("StdAdd", function() error("hook failed") end)
hook(StdObject, "Get", function(self: any, k: string) print("got", k) end)
hookScript(PlayerFrame, "OnHide", function(frame: any) print("hidden") end)

-- A missing target is an error where the hook is installed.
Mock.check(not pcall(hook, "NoSuchFrame", "Show", function() end), "missing target refused")
Mock.check(not pcall(hookScript, "NoSuchFrame", "OnShow", function() end), "missing frame refused")

-- An addon that has loaded already: installed at once.
hook("OtherFrame", "Show", function() print("other shown") end, { addon = "Other" })
-- Missing when its addon loads: reported then.
hook("NeverFrame", "Show", function() end, { addon = "Blizzard_Never" })
"#,
        )],
        r#"function StdAdd(a, b) return a + b, "sum" end
StdObject = { Get = function(self, k) return k .. "!" end }
CreateFrame("Frame", "OtherFrame")
Mock.loaded.Other = true
"#,
        r#"local sum, label = StdAdd(1, 2)
print("result", sum, label)
print("get", StdObject:Get("k"))
PlayerFrame:Hide()
OtherFrame:Show()
Mock.loadAddon("Blizzard_Never")
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "added 1 2",
            "result 3 sum",
            "got k",
            "get k!",
            "hidden",
            "other shown",
        ],
        "{out:?}"
    );
    let reported = errors(&out);
    assert_eq!(reported.len(), 2, "{out:?}");
    assert!(reported[0].contains("hook failed"), "{out:?}");
    assert!(
        reported[1].contains("hook: `NeverFrame` does not exist"),
        "{out:?}"
    );
}

// ------------------------------------------------------------------ saved --

const SAVED_MAIN: &str = r#"import * as saved from "wowlua:saved"
import onLoad, onLogout from "wowlua:lifecycle"

interface Settings
    scale: number
    enabled: boolean
    hidden: { [string]: boolean }
    window: { x: number, y: number }
    custom?: { [string]: any }
end

local defaults = { scale = 1, enabled = false, hidden = {}, window = { x = 0, y = 0 } } :: Settings
local db = saved.bind("StdDB", {
    version = 3,
    defaults = defaults,
    migrate = {
        [2] = function(old: any) old.scale = old.uiScale; old.uiScale = nil end,
        [3] = function(old: any) old.hidden = old.hidden or {} end,
    },
})

-- Locked until loaded, for reads and writes.
local ok, err = pcall(function() return db.scale end)
Mock.check(not ok and tostring(err):match("Std: StdDB accessed before it was loaded; use @onLoad") ~= nil, "early read")
ok, err = pcall(function() db.scale = 2 end)
Mock.check(not ok and tostring(err):match("accessed before it was loaded") ~= nil, "early write")
Mock.check(not saved.handle(db):isLoaded(), "not loaded yet")
Mock.check(saved.handle(db).profiles == nil, "no profiles")

@onLoad
local function init()
    print("scale", db.scale, "x", db.window.x, "enabled", db.enabled)
    db.window.y = 7
    db.hidden.minimap = true
    db.enabled = false
    db.custom = {}
    Mock.check(saved.handle(db):isLoaded(), "loaded")
    Mock.check(_G.StdDB == db, "the global is the returned table")
    -- Writes never reach the defaults.
    Mock.check(defaults.window.y == 0 and defaults.hidden.minimap == nil, "defaults untouched")
end

@onLogout
local function bye() print("logout", db.scale) end
"#;

const SAVED_SCRIPT: &str = r#"
Mock.loadAddon("Std")
Mock.fire("PLAYER_LOGOUT")
local d = StdDB
print("saved", d.__version, rawget(d, "scale"), d.uiScale, rawget(d, "enabled"))
print("window", d.window and rawget(d.window, "y"), d.window and rawget(d.window, "x"))
print("hidden", d.hidden and d.hidden.minimap, type(rawget(d, "custom")))
"#;

#[test]
fn saved_variables_load_migrate_default_and_strip() {
    let dir = project(&[("src/main.wowlua", SAVED_MAIN)]);
    // Stored at version 1: migrations 2 and 3 run; values equal to their
    // defaults go (also `false`); tables without defaults stay.
    let out = modes_in(&dir, "StdDB = { __version = 1, uiScale = 2 }", SAVED_SCRIPT);
    assert_eq!(
        out,
        [
            "scale 2 x 0 enabled false",
            "logout 2",
            "saved 3 2 nil nil",
            "window 7 nil",
            "hidden true table",
        ],
        "{out:?}"
    );
    // A fresh install runs no migrations; a value back at its default goes.
    let out = modes_in(&dir, "", SAVED_SCRIPT);
    assert_eq!(
        out,
        [
            "scale 1 x 0 enabled false",
            "logout 1",
            "saved 3 nil nil nil",
            "window 7 nil",
            "hidden true table",
        ],
        "{out:?}"
    );
    // Saved by a newer version: kept unchanged, nothing stripped.
    let out = modes_in(&dir, "StdDB = { __version = 9, scale = 1 }", SAVED_SCRIPT);
    assert!(
        errors(&out)
            .iter()
            .any(|l| l.contains("StdDB was saved by a newer version (9 > 3)")),
        "{out:?}"
    );
    assert!(out.contains(&"saved 9 1 nil false".to_string()), "{out:?}");
    // Not stripped, but defaults never read were not created either.
    assert!(out.contains(&"window 7 nil".to_string()), "{out:?}");
}

#[test]
fn saved_migrations() {
    let dir = project(&[(
        "src/main.wowlua",
        r#"import * as saved from "wowlua:saved"
import onLoad from "wowlua:lifecycle"

local runs: number[] = {}
local db = saved.bind("StdDB", {
    version = 4,
    defaults = { a = 0 },
    -- Steps may be missing; each runs once, on the whole table.
    migrate = {
        [2] = function(old: any)
            runs[#runs + 1] = 2
            for _, p in pairs(old.profiles) do p.a *= 10 end
        end,
        [4] = function(old: any)
            runs[#runs + 1] = 4
            old.profiles.Tank.b = old.__version
        end,
    },
    profiles = true,
})

@onLoad
local function init()
    print("runs", table.concat(runs, ","))
    print("a", db.a, "tank", StdDB.profiles.Tank.a, StdDB.profiles.Tank.b)
    print("version", StdDB.__version)
end
"#,
    )]);
    let out = modes_in(
        &dir,
        "StdDB = { __version = 1, profiles = { Default = { a = 1 }, Tank = { a = 2 } } }",
        "Mock.loadAddon(\"Std\")",
    );
    // The stored version stays 1 for the steps (it is set at the end).
    assert_eq!(out, ["runs 2,4", "a 10 tank 20 1", "version 4"], "{out:?}");

    // A failing step: nothing changes, the data stays locked with the reason.
    let dir = project(&[(
        "src/main.wowlua",
        r#"import * as saved from "wowlua:saved"
import onLoad from "wowlua:lifecycle"

local db = saved.bind("StdDB", {
    version = 3,
    defaults = { scale = 1 },
    migrate = {
        [2] = function(old: any) old.scale = 5 end,
        [3] = function(old: any) old.scale = 6; error("bad migration") end,
    },
})

@onLoad
local function init()
    local ok, err = pcall(function() return db.scale end)
    print("access", ok, tostring(err):match("Std: StdDB failed to migrate to version 3; data preserved") ~= nil)
    print("handle", saved.handle(db):isLoaded())
end
"#,
    )]);
    let out = modes_in(
        &dir,
        "StdDB = { __version = 1, scale = 3 }",
        "Mock.loadAddon(\"Std\")\nMock.fire(\"PLAYER_LOGOUT\")\nprint(\"stored\", StdDB.__version, StdDB.scale)",
    );
    assert_eq!(
        plain(&out),
        ["access false true", "handle false", "stored 1 3"],
        "{out:?}"
    );
    assert!(errors(&out)[0].contains("bad migration"), "{out:?}");
}

#[test]
fn saved_reset_strip_options_and_late_binds() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as saved from "wowlua:saved"
import onLoad from "wowlua:lifecycle"

local db = saved.bind("StdDB", { version = 2, defaults = { scale = 1, list = { 1, 2 } } })
local keep = saved.bind("StdOtherDB", { version = 1, defaults = { scale = 1, nested = { a = 1 } }, stripDefaults = false })

@onLoad
local function init()
    db.scale = 3
    db.extra = "x"
    saved.handle(db):reset()
    print("reset", db.scale, rawget(db, "extra"), rawget(db, "__version"))
    keep.scale = 1
    print("nested", keep.nested.a)
end

-- Bound after loading: usable at once.
_G.StdLate = function()
    local late = saved.bind("StdCharDB", { version = 1, defaults = { n = 5 } })
    print("late", late.n, saved.handle(late):isLoaded())
    late.n = 6
end

-- handle() takes only bound tables.
Mock.check(not pcall(saved.handle, {}), "handle of another table")
"#,
        )],
        "StdCharDB = { n = 1 }",
        r#"Mock.loadAddon("Std")
StdLate()
Mock.fire("PLAYER_LOGOUT")
print("db", rawget(StdDB, "scale"), StdDB.__version)
print("keep", rawget(StdOtherDB, "scale"), StdOtherDB.nested and StdOtherDB.nested.a)
print("char", StdCharDB.n, StdCharDB.__version)
"#,
    );
    assert_eq!(
        out,
        [
            "reset 1 nil 2",
            "nested 1",
            "late 1 true",
            "db nil 2",
            "keep 1 1",
            "char 6 1",
        ],
        "{out:?}"
    );
}

#[test]
fn saved_secrets_are_dropped() {
    let dir = project(&[(
        "src/main.wowlua",
        r#"import * as saved from "wowlua:saved"
import onLoad from "wowlua:lifecycle"

local db = saved.bind("StdDB", { version = 1, defaults = { opts = { a = 1 } } })

@onLoad
local function init()
    db.secret = Mock.secret()
    db.opts.secret = Mock.secret()
    db.free = { secret = Mock.secret(), keep = 1 }
end
"#,
    )]);
    let script = r#"Mock.loadAddon("Std")
Mock.fire("PLAYER_LOGOUT")
print("saved", rawget(StdDB, "secret"), rawget(StdDB, "opts"), StdDB.free.secret, StdDB.free.keep)
"#;
    for mode in MODES {
        build(dir.path(), mode);
        let out = run(dir.path(), mode, "", script);
        assert_eq!(plain(&out), ["saved nil nil nil 1"], "{mode}: {out:?}");
        // Reported in debug builds only.
        let reported = errors(&out);
        if *mode == "dev" {
            assert_eq!(
                reported,
                ["Std: StdDB: secret values were not saved"],
                "{out:?}"
            );
        } else {
            assert!(reported.is_empty(), "{mode}: {out:?}");
        }
    }
}

#[test]
fn saved_variables_loaded_first() {
    let dir = project(&[(
        "src/main.wowlua",
        r#"import * as saved from "wowlua:saved"

-- `addon.load-saved-variables-first`: usable at top level.
local db = saved.bind("StdDB", { version = 1, defaults = { scale = 1 } })
print("top", db.scale, db.count)
db.count = (db.count ?? 0) + 1
"#,
    )]);
    let manifest = fs::read_to_string(dir.path().join("wowlua.json")).unwrap();
    let manifest = manifest.replace(
        "\"addon\": {",
        "\"addon\": { \"load-saved-variables-first\": true,",
    );
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    let out = modes_in(
        &dir,
        "StdDB = { __version = 1, count = 4 }",
        "Mock.loadAddon(\"Std\")\nprint(\"count\", StdDB.count)",
    );
    assert_eq!(out, ["top 1 4", "count 5"], "{out:?}");
    let toc = fs::read_to_string(dir.path().join("dist/dev/Std/Std.toc")).unwrap();
    assert!(toc.contains("## LoadSavedVariablesFirst: 1"), "{toc}");
}

#[test]
fn saved_profiles_switch_per_character() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as saved from "wowlua:saved"
import onLoad from "wowlua:lifecycle"

local db = saved.bind("StdDB", { version = 1, defaults = { scale = 1, bars = { n = 1 } }, profiles = true })
local profiles = saved.handle(db).profiles

-- Profile operations need the data.
Mock.check(not pcall(profiles.use, profiles, "X"), "use before load")
Mock.check(not pcall(profiles.list, profiles), "list before load")
_G.StdUseDot = function() (profiles :: any).use("Tank") end

@onLoad
local function init()
    local changes = profiles:onChange(function(name: string) print("changed to", name) end)
    print("current", profiles:current(), db.scale)
    db.scale = 2
    db.bars.n = 4
    profiles:use("Healer")
    print("healer", db.scale, db.bars.n)
    profiles:copy("Default")
    print("copied", db.scale, db.bars.n)
    -- A copy is not shared with its source.
    db.bars.n = 5
    print("source", StdDB.profiles.Default.bars.n)
    -- Copying the active profile keeps it.
    profiles:copy("Healer")
    print("self copy", db.scale, db.bars.n)
    print("list", table.concat(profiles:list(), ","))
    Mock.check(not pcall(profiles.copy, profiles, "Missing"), "copy of a missing profile")
    Mock.check(not pcall(profiles.delete, profiles, "Healer"), "delete of the active profile")
    changes:cancel()
    profiles:use("Default")
    profiles:delete("Healer")
    print("after delete", table.concat(profiles:list(), ","), profiles:current(), db.scale)
    saved.handle(db):reset()
    print("reset", db.scale, StdDB.__version)
    profiles:use("Tank")
end
"#,
        )],
        "",
        "Mock.loadAddon(\"Std\")\nlocal ok, err = pcall(StdUseDot)\nMock.check(not ok and err:match(\"profiles:use: the profile name must be a string, not nil\"), \"dot call\")\nMock.fire(\"PLAYER_LOGOUT\")\nprint(\"char\", StdDBPerChar.profile, StdDB.profiles.Tank and next(StdDB.profiles.Tank), rawget(StdDB.profiles.Default, \"scale\"))",
    );
    assert_eq!(
        out,
        [
            "current Default 1",
            "changed to Healer",
            "healer 1 1",
            "copied 2 4",
            "source 4",
            "self copy 2 5",
            "list Default,Healer",
            "after delete Default Default 2",
            "reset 1 1",
            "char Tank nil nil",
        ],
        "{out:?}"
    );
}

#[test]
fn saved_profiles_are_restored_per_character() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as saved from "wowlua:saved"
import onLoad from "wowlua:lifecycle"

local db = saved.bind("StdDB", { version = 1, defaults = { scale = 1 }, profiles = true })

@onLoad
local function init()
    print("profile", saved.handle(db).profiles:current(), db.scale)
end
"#,
        )],
        r#"StdDB = { __version = 1, profiles = { Default = { scale = 2 }, Tank = { scale = 3 } } }
StdDBPerChar = { profile = "Tank" }"#,
        "Mock.loadAddon(\"Std\")",
    );
    assert_eq!(out, ["profile Tank 3"], "{out:?}");
}

// ------------------------------------------------------------------ slash --

const ROUTER: &str = r#"import slash, router from "wowlua:slash"

local commands = router({
    show = { run = function() print("show") end, help = "Show the window", aliases = { "open" } },
    scale = { args = { "number" }, run = function(v: number) print("scale", v) end, help = "Set the scale" },
    mode = { args = { { "tank", "heal" }, "boolean?" }, run = function(m: string, on: boolean?) print("mode", m, on) end },
    say = { args = { "string", "rest" }, flags = { "loud" }, run = function(who: string, text: string, flags: { loud: boolean }) print("say", who, "[" .. text .. "]", flags.loud) end },
    reset = { flags = { "all" }, run = function(flags: { all: boolean }) print("reset", flags.all) end },
    move = { args = { "number", "number?" }, flags = { "smooth" }, run = function(x: number, y: number?, flags: { smooth: boolean }) print("move", x, y, flags.smooth) end },
    note = { args = { "rest" }, run = function(text: string) print("note", "[" .. text .. "]") end },
    boom = { run = function() error("command failed") end },
})

@slash("/std", "/stdlib")
local function onCommand(msg: string) print("->", commands:dispatch(msg)) end

@slash("/stdping")
local function ping(msg: string, editBox: any) print("ping", "[" .. msg .. "]", type(editBox)) end
"#;

#[test]
fn slash_commands_and_the_router() {
    let out = modes(
        &[("src/main.wowlua", ROUTER)],
        "",
        r#"Mock.slash("/std show")
Mock.slash("/STDLIB OPEN")
Mock.slash("/std scale 1.5")
Mock.slash("/std scale -2")
Mock.slash("/std scale big")
Mock.slash("/std scale 1 2")
Mock.slash("/std mode HEAL yes")
Mock.slash("/std mode tank OFF")
Mock.slash("/std mode tank 0")
Mock.slash("/std mode tank")
Mock.slash("/std mode dps")
Mock.slash("/std mode tank maybe")
Mock.slash([[/std say "two words" hello there  friend  ]])
Mock.slash([[/std say 'it\'s' "a \"quoted\" word"]])
Mock.slash([[/std say --loud bob hello --loud]])
Mock.slash("/std say bob")
Mock.slash("/std say")
Mock.slash("/std reset --all")
Mock.slash("/std reset")
Mock.slash("/std reset --nope")
Mock.slash("/std reset extra")
Mock.slash("/std move --smooth 1")
Mock.slash("/std move 1 2 --smooth")
Mock.slash("/std note   ")
Mock.slash("/std boom")
Mock.slash("/std nothing")
Mock.slash("/stdping 42")
Mock.slash("/stdping")
print("keys", SLASH_WOWLUA_STD_11, SLASH_WOWLUA_STD_12, SLASH_WOWLUA_STD_21)
"#,
    );
    let out: Vec<String> = plain(&out)
        .iter()
        .map(|l| l.replace("|cff33ff99Std|r: ", ""))
        .collect();
    let help = [
        "boom",
        "mode <tank|heal> [boolean]",
        "move <number> [number] [--smooth]",
        "note <text...>",
        "reset [--all]",
        "say <string> <text...> [--loud]",
        "scale <number> - Set the scale",
        "show - Show the window",
    ];
    let mut expected: Vec<&str> = vec![
        "show",
        "-> true",
        "show",
        "-> true",
        "scale 1.5",
        "-> true",
        "scale -2",
        "-> true",
        "Usage: scale <number>",
        "-> true",
        "Usage: scale <number>",
        "-> true",
        // Choices match in any case and give the declared spelling.
        "mode heal true",
        "-> true",
        "mode tank false",
        "-> true",
        "mode tank false",
        "-> true",
        "mode tank nil",
        "-> true",
        "Usage: mode <tank|heal> [boolean]",
        "-> true",
        "Usage: mode <tank|heal> [boolean]",
        "-> true",
        // The rest as typed, trimmed at the end.
        "say two words [hello there  friend] false",
        "-> true",
        "say it's [\"a \\\"quoted\\\" word\"] false",
        "-> true",
        // Flags before the rest; in it, they are text.
        "say bob [hello --loud] true",
        "-> true",
        "say bob [] false",
        "-> true",
        "Usage: say <string> <text...> [--loud]",
        "-> true",
        "reset true",
        "-> true",
        "reset false",
        "-> true",
        "Usage: reset [--all]",
        "-> true",
        "Usage: reset [--all]",
        "-> true",
        "move 1 nil true",
        "-> true",
        "move 1 2 true",
        "-> true",
        "note []",
        "-> true",
        // A failing command is reported; it was dispatched.
        "-> true",
    ];
    // An unknown command lists the commands and is not dispatched.
    expected.extend(help);
    expected.extend([
        "-> false",
        "ping [42] table",
        "ping [] table",
        "keys /std /stdlib /stdping",
    ]);
    assert_eq!(out, expected, "{out:?}");
}

#[test]
fn slash_router_help() {
    let out = modes(
        &[("src/main.wowlua", ROUTER)],
        "",
        "Mock.slash(\"/std\")\nMock.slash(\"/std HELP\")",
    );
    // Both list every command, sorted, and count as handled.
    assert_eq!(out.len(), 18, "{out:?}");
    assert_eq!(out[0], "|cff33ff99Std|r: boom", "{out:?}");
    assert_eq!(out[8], "-> true", "{out:?}");
    assert_eq!(out[0..9], out[9..18], "{out:?}");
}

/// Commands added and removed at run time, with owners; help and
/// `commands()` in the order they were added (`order = "added"`).
#[test]
fn slash_router_commands_come_and_go() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import slash, router from "wowlua:slash"
local owner = {}
local cmds = router({ zeta = { run = function() print("zeta") end, help = "Defined first" } }, { order = "added" })
cmds:add("beta", { args = { "number" }, run = function(n: number) print("beta", n) end, help = "B", aliases = { "b" } }, owner)
cmds:add("alpha", { run = function() print("alpha") end }, owner)
cmds:add("solo", { run = function() print("solo") end })
_G.StdCmds = cmds
_G.StdOwner = owner
@slash("/std")
local function onCommand(msg: string) print("->", cmds:dispatch(msg)) end
"#,
        )],
        "",
        r#"Mock.slash("/std b 2")
Mock.slash("/std help")
for _, c in ipairs(StdCmds:commands()) do print(c.name, c.usage, c.help, #c.aliases, c.owner == StdOwner) end
print(pcall(StdCmds.add, StdCmds, "other", { run = function() end, aliases = { "B" } }))
StdCmds:removeAll(StdOwner)
StdCmds:remove("solo")
StdCmds:remove("nothing")
Mock.slash("/std b 2")
"#,
    );
    let out: Vec<String> = plain(&out)
        .iter()
        .map(|l| l.replace("|cff33ff99Std|r: ", ""))
        .collect();
    assert_eq!(
        out,
        [
            "beta 2",
            "-> true",
            "zeta - Defined first",
            "beta <number> - B",
            "alpha",
            "solo",
            "-> true",
            "zeta zeta Defined first 0 false",
            "beta beta <number> B 1 true",
            "alpha alpha nil 0 true",
            "solo solo nil 0 false",
            "false router: \"B\" is taken by the command \"beta\"",
            // Only zeta is left: the help, and not dispatched.
            "zeta - Defined first",
            "-> false",
        ],
        "{out:?}"
    );
}
// -------------------------------------------------------------- callbacks --

#[test]
fn callback_registries() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as callbacks from "wowlua:callbacks"

local events = callbacks.registry({ compat = "CallbackHandler" })
local owner = {}
events:on("Kill", function(n: number) print("kill", n) end, owner)
events:on("Kill", function() error("handler failed") end)
events:once("Kill", function(n: number) print("first kill", n) end)
print("has", events:has("Kill"), events:has("Reset"))
events:fire("Kill", 1)
events:fire("Kill", 2)
events:offAll(owner)
events:fire("Kill", 3)
events:fire("Nobody", 1)

-- A handler added during a fire waits for the next one; one cancelled
-- during a fire is skipped at once.
local late: any = nil
local second: any = nil
events:on("Tick", function()
    print("tick 1")
    late = late or events:on("Tick", function() print("tick late") end)
    if second then events:off(second) end
end)
second = events:on("Tick", function() print("tick 2") end)
events:fire("Tick")
events:fire("Tick")

-- Unsubscribing everything removes the name.
local sub = events:on("Solo", function() end)
sub:cancel()
print("solo", events:has("Solo"), sub.active)

-- Removal by name and function (every subscription of it, `once` too),
-- and by owner and name.
local onKill = function(n: number) print("by function", n) end
events:on("Kill", onKill)
events:once("Kill", onKill)
events:on("Reset", function() print("reset kept") end, owner)
events:on("Kill", function(n: number) print("owner's kill", n) end, owner)
events:fire("Kill", 20)
events:off("Kill", onKill)
events:offAll(owner, "Kill")
events:fire("Kill", 21)
events:fire("Reset")
events:offAll(owner)

-- A plain Lua consumer, CallbackHandler style.
local consumer: any = {}
function consumer.OnKill(self: any, name: string, n: number) print("method", self == consumer, name, n) end
function consumer.OnKillArg(self: any, arg: any, name: string, n: number) print("method arg", arg, name, n) end
function consumer.Kill(self: any, name: string, n: number) print("default method", name, n) end
events.RegisterCallback(consumer, "Kill", "OnKill")
events:fire("Kill", 4)
-- Registering the same owner and name again replaces the handler.
events.RegisterCallback(consumer, "Kill", "OnKillArg", "x")
events:fire("Kill", 5)
events.RegisterCallback(consumer, "Kill")
events:fire("Kill", 6)
events.RegisterCallback(consumer, "Kill", function(name: string, n: number) print("function", name, n) end)
events:fire("Kill", 7)
events.RegisterCallback(consumer, "Kill", function(arg: any, name: string, n: number) print("function arg", arg, name, n) end, nil)
events:fire("Kill", 8)
events.RegisterCallback("SomeAddon", "Kill", function(name: string, n: number) print("string owner", n) end)
events:fire("Kill", 9)
events.UnregisterCallback(consumer, "Kill")
events.UnregisterAllCallbacks("SomeAddon")
events:fire("Kill", 10)
print("kill", events:has("Kill"))

-- Without compat, no CallbackHandler face.
local plainRegistry = callbacks.registry()
print("compat", plainRegistry.RegisterCallback == nil)
"#,
        )],
        "",
        "",
    );
    assert_eq!(
        plain(&out),
        [
            "has true false",
            "kill 1",
            "first kill 1",
            "kill 2",
            "tick 1",
            "tick 1",
            "tick late",
            "solo false false",
            "by function 20",
            "by function 20",
            "owner's kill 20",
            "reset kept",
            "method true Kill 4",
            "method arg x Kill 5",
            "default method Kill 6",
            "function Kill 7",
            "function arg nil Kill 8",
            "function arg nil Kill 9",
            "string owner 9",
            // The failing handler without an owner is still there.
            "kill true",
            "compat true",
        ],
        "{out:?}"
    );
    // The failing handler: the second failure is within the rate limit.
    let reported = errors(&out);
    assert_eq!(reported.len(), 1, "{out:?}");
    assert!(reported[0].contains("handler failed"), "{out:?}");
}

// ---------------------------------------------------------- compile checks --

/// Builds `source` as the addon's only module; returns the build's stderr.
fn check(source: &str) -> String {
    let dir = project(&[("src/main.wowlua", source)]);
    let assert = wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert();
    String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
}

/// The core modules' compile-time checks (spec stdlib, "core").
#[test]
fn core_modules_are_checked_at_compile_time() {
    let cases: &[(&str, &str)] = &[
        (
            "import event from \"wowlua:events\"\n@event(\"PLAYER_LOGN\")\nlocal function f() end\n",
            "E8003",
        ),
        // Through a namespace import and an alias, too.
        (
            "import * as events from \"wowlua:events\"\n@events.event(\"PLAYER_LOGN\")\nlocal function f() end\n",
            "E8003",
        ),
        (
            "import event as ev from \"wowlua:events\"\n@ev(\"PLAYER_LOGN\")\nlocal function f() end\n",
            "E8003",
        ),
        (
            "import * as events from \"wowlua:events\"\nevents.on(\"BAG_UPDAET\", function() end)\n",
            "E8003",
        ),
        (
            "import on from \"wowlua:events\"\non(\"BAG_UPDAET\", function() end)\n",
            "E8003",
        ),
        (
            "import once from \"wowlua:events\"\nonce(\"BAG_UPDAET\", function() end)\n",
            "E8003",
        ),
        (
            "import onUnit from \"wowlua:events\"\nonUnit(\"UNIT_HELTH\", { \"player\", \"target\", \"focus\" }, function() end)\n",
            "E8003",
        ),
        (
            "import watchUnit from \"wowlua:events\"\nwatchUnit({ \"UNIT_HEALTH\", \"UNIT_HELTH\" }, function() end)\n",
            "E8003",
        ),
        (
            "import * as events from \"wowlua:events\"\ndeclare message STD_M()\nevents.watchUnit({ \"STD_M\" }, function() end)\n",
            "E8003",
        ),
        // The handler gets one of the names first.
        (
            "import watchUnit from \"wowlua:events\"\nwatchUnit({ \"UNIT_HEALTH\" }, function(event: number) end)\n",
            "E2001",
        ),
        (
            "import offAll from \"wowlua:events\"\noffAll({}, \"BAG_UPDAET\")\n",
            "E8003",
        ),
        // `errors.call` returns the function's results, typed.
        (
            "import * as errors from \"wowlua:errors\"\nlocal ok, n = errors.call(function(a: number): number return a end, 1)\nlocal s: string = n\nprint(ok, s)\n",
            "E2001",
        ),
        (
            "import call from \"wowlua:errors\"\ncall(function(a: number) end, \"one\")\n",
            "E2001",
        ),
        (
            "import send from \"wowlua:events\"\nsend(\"PLAYER_LOGIN\")\n",
            "E8001",
        ),
        (
            "import send from \"wowlua:events\"\nsend(\"STD_UNDECLARED\")\n",
            "E8003",
        ),
        (
            "import event from \"wowlua:events\"\nclass A\n    @event(\"PLAYER_LOGIN\")\n    function m() end\nend\n",
            "E8002",
        ),
        (
            "import unitEvent from \"wowlua:events\"\n@unitEvent(\"UNIT_HELTH\", \"player\", \"target\", \"focus\")\nlocal function f() end\n",
            "E8003",
        ),
        (
            "import slash from \"wowlua:slash\"\n@slash(\"std\")\nlocal function f() end\n",
            "E8101",
        ),
        (
            "import slash from \"wowlua:slash\"\n@slash(\"/std\")\nlocal function f() end\n@slash(\"/STD\")\nlocal function g() end\n",
            "E8010",
        ),
        (
            "import slash from \"wowlua:slash\"\n@slash(\"/reload\")\nlocal function f() end\n",
            "W8011",
        ),
        (
            "import router from \"wowlua:slash\"\nlocal r = router({ go = { args = { \"number\", \"text\" }, run = function(a, b) end } })\n",
            "E8112",
        ),
        (
            "import router from \"wowlua:slash\"\nlocal r = router({ go = { args = { \"number\" }, flags = { \"all\" }, run = function(a) end } })\n",
            "E8112",
        ),
        // Commands added later are checked alike.
        (
            "import router from \"wowlua:slash\"\nlocal r = router({})\nr:add(\"go\", { args = { \"number\" }, run = function(a: number, b: string) end })\n",
            "E8112",
        ),
        (
            "import router from \"wowlua:slash\"\nlocal r = router({})\nr:add(\"go\", { args = { \"rest\", \"number\" }, run = function(a: string, b: number) end })\n",
            "E8112",
        ),
        // `known`'s fallback joins the result's type.
        (
            "import * as secret from \"wowlua:secret\"\nlocal function f(name: string | Secret<string>)\n    local b: boolean = secret.known(name, \"?\")\n    print(b)\nend\nf(\"x\")\n",
            "E2001",
        ),
        // A registry's names and handlers in `off` and `offAll`.
        (
            "import * as callbacks from \"wowlua:callbacks\"\ninterface Ev\n    Kill: (n: number) -> ()\nend\nlocal r: callbacks.Registry<Ev> = callbacks.registry()\nr:off(\"Kil\", function(n: number) end)\n",
            "E8420",
        ),
        (
            "import * as callbacks from \"wowlua:callbacks\"\ninterface Ev\n    Kill: (n: number) -> ()\nend\nlocal r: callbacks.Registry<Ev> = callbacks.registry()\nr:offAll({}, \"Kil\")\n",
            "E8420",
        ),
        (
            "import * as callbacks from \"wowlua:callbacks\"\ninterface Ev\n    Kill: (n: number) -> ()\nend\nlocal r: callbacks.Registry<Ev> = callbacks.registry()\nr:off(\"Kill\", function(n: string) end)\n",
            "E2001",
        ),
        (
            "import * as callbacks from \"wowlua:callbacks\"\ninterface Ev\n    Kill: (n: number) -> ()\nend\nlocal r: callbacks.Registry<Ev> = callbacks.registry()\nr:off(\"Kill\")\n",
            "E2007",
        ),
        (
            "import * as callbacks from \"wowlua:callbacks\"\ninterface Ev\n    Kill: (n: number) -> ()\nend\nlocal r: callbacks.Registry<Ev> = callbacks.registry()\nlocal sub = r:on(\"Kill\", function() end)\nr:off(sub, function() end)\n",
            "E2006",
        ),
        (
            "import * as callbacks from \"wowlua:callbacks\"\ninterface Ev\n    Kill: (n: number) -> ()\nend\nlocal r: callbacks.Registry<Ev> = callbacks.registry()\nlocal name = \"Kill\"\nr:off(name, function() end)\n",
            "E2001",
        ),
        (
            "import * as saved from \"wowlua:saved\"\nlocal n = \"StdDB\"\nlocal db = saved.bind(n, { version = 1, defaults = {} })\n",
            "E8102",
        ),
        (
            "import * as saved from \"wowlua:saved\"\nlocal db = saved.bind(\"OtherDB\", { version = 1, defaults = {} })\n",
            "E8020",
        ),
        (
            "import bind from \"wowlua:saved\"\nlocal a = bind(\"StdDB\", { version = 1, defaults = {} })\nlocal b = bind(\"StdDB\", { version = 1, defaults = {} })\n",
            "E8103",
        ),
        (
            "import * as saved from \"wowlua:saved\"\nlocal db = saved.bind(\"StdOtherDB\", { version = 1, defaults = {}, profiles = true })\n",
            "E8021",
        ),
        (
            "import * as saved from \"wowlua:saved\"\nlocal db = saved.bind(\"StdDBPerChar\", { version = 1, defaults = {}, profiles = true })\n",
            "E8117",
        ),
        // Handlers are typed by their event or message.
        (
            "import on from \"wowlua:events\"\non(\"UNIT_HEALTH\", function(unit: number) end)\n",
            "E2001",
        ),
        (
            "import * as events from \"wowlua:events\"\nevents.once(\"UNIT_HEALTH\", function(unit) local n: number = unit end)\n",
            "E2001",
        ),
        (
            "import event from \"wowlua:events\"\n@event(\"UNIT_HEALTH\")\nlocal function f(unit: number) end\n",
            "E2001",
        ),
        (
            "import unitEvent from \"wowlua:events\"\n@unitEvent(\"UNIT_HEALTH\", \"player\")\nlocal function f(unit: boolean) end\n",
            "E2001",
        ),
        (
            "import send from \"wowlua:events\"\ndeclare message STD_PING(n: number)\nsend(\"STD_PING\", \"one\")\n",
            "E2001",
        ),
        (
            "import send from \"wowlua:events\"\ndeclare message STD_PING(n: number)\nsend(\"STD_PING\")\n",
            "E2007",
        ),
        // Hook targets.
        (
            "import hook from \"wowlua:hooks\"\nhook(\"NoSuchGlobalFunction\", function() end)\n",
            "W8040",
        ),
        (
            "import hook from \"wowlua:hooks\"\n@hook(GameTooltip, \"NoSuchMethod\")\nlocal function f() end\n",
            "W8040",
        ),
        (
            "import hookScript from \"wowlua:hooks\"\nhookScript(\"NoSuchFrame\", \"OnShow\", function() end)\n",
            "W8040",
        ),
        (
            "import hook from \"wowlua:hooks\"\nhook(\"PlayerSpellsFrame\", \"Show\", function() end)\n",
            "W8102",
        ),
        (
            "import hook from \"wowlua:hooks\"\nhook(PlayerSpellsFrame, \"Show\", function() end, { addon = \"Blizzard_PlayerSpells\" })\n",
            "W8102",
        ),
        (
            "import * as saved from \"wowlua:saved\"\ninterface S\n    name: Secret<string>\nend\nlocal db = saved.bind(\"StdDB\", { version = 1, defaults = { inner = {} } :: { inner: S } })\n",
            "E8104",
        ),
        // Settings controls against the store's type.
        (
            "import * as settings from \"wowlua:settings\"\nimport * as saved from \"wowlua:saved\"\ninterface O\n    on: boolean\n    n: number\n    mode: \"a\" | \"b\"\nend\nlocal db = saved.bind(\"StdDB\", { version = 1, defaults = { on = true, n = 1, mode = \"a\" } :: O })\nsettings.register({ title = \"x\", store = db, controls = { settings.checkbox(\"of\", { label = \"x\" }) } })\n",
            "E8113",
        ),
        (
            "import * as settings from \"wowlua:settings\"\nimport * as saved from \"wowlua:saved\"\ninterface O\n    on: boolean\n    n: number\n    mode: \"a\" | \"b\"\nend\nlocal db = saved.bind(\"StdDB\", { version = 1, defaults = { on = true, n = 1, mode = \"a\" } :: O })\nsettings.register({ title = \"x\", store = db, controls = { settings.slider(\"on\", { label = \"x\", min = 0, max = 1, step = 1 }) } })\n",
            "E8070",
        ),
        (
            "import * as settings from \"wowlua:settings\"\nimport * as saved from \"wowlua:saved\"\ninterface O\n    on: boolean\n    n: number\n    mode: \"a\" | \"b\"\nend\nlocal db = saved.bind(\"StdDB\", { version = 1, defaults = { on = true, n = 1, mode = \"a\" } :: O })\nsettings.register({ title = \"x\", store = db, controls = { settings.dropdown(\"mode\", { label = \"x\", options = { { value = \"c\", label = \"c\" } } }) } })\n",
            "E8107",
        ),
        (
            "import * as settings from \"wowlua:settings\"\nimport * as saved from \"wowlua:saved\"\ninterface O\n    on: boolean\n    n: number\n    mode: \"a\" | \"b\"\nend\nlocal db = saved.bind(\"StdDB\", { version = 1, defaults = { on = true, n = 1, mode = \"a\" } :: O })\nsettings.register({ title = \"x\", store = db, controls = {} })\nsettings.register({ title = \"y\", store = db, controls = {} })\n",
            "E8114",
        ),
        (
            "import * as launcher from \"wowlua:launcher\"\nlauncher.register({ icon = \"i\", label = \"l\", onClick = function() end })\nlauncher.register({ icon = \"i\", label = \"l\", onClick = function() end })\n",
            "E8080",
        ),
        (
            "import * as comm from \"wowlua:comm\"\nlocal c = comm.open(\"ThisPrefixIsTooLong\")\n",
            "E8050",
        ),
        // Pools are typed by the class they hold.
        (
            "import FramePool from \"wowlua:pool\"\nclass Row\n    constructor(label: string) end\nend\nlocal rows = FramePool.new(Row, 1)\n",
            "E2001",
        ),
        (
            "import FramePool from \"wowlua:pool\"\nclass Row\n    constructor(label: string) end\nend\nlocal rows = FramePool.new(Row, \"x\")\nlocal n: number = rows:acquire()\n",
            "E2001",
        ),
        // The handle's methods are typed (spec stdlib "saved").
        (
            "import * as saved from \"wowlua:saved\"\nlocal db = saved.bind(\"StdDB\", { version = 1, defaults = {}, profiles = true })\nsaved.handle(db).reset()\n",
            "E2008",
        ),
        (
            "import * as saved from \"wowlua:saved\"\nlocal db = saved.bind(\"StdDB\", { version = 1, defaults = {}, profiles = true })\nsaved.handle(db).profiles?.use(\"Tank\")\n",
            "E2007",
        ),
        (
            "import after from \"wowlua:timer\"\nafter(\"soon\", function() end)\n",
            "E2001",
        ),
        (
            "import debounce from \"wowlua:timer\"\n@debounce(1)\nlocal function f(): number return 1 end\n",
            "E8030",
        ),
        (
            "import throttle from \"wowlua:timer\"\n@throttle(1)\nlocal function f(): string return \"\" end\n",
            "E8030",
        ),
        (
            "import nextFrame from \"wowlua:timer\"\n@nextFrame\nlocal function f(): boolean return true end\n",
            "E8030",
        ),
        (
            "import outOfCombat from \"wowlua:combat\"\nclass A\n    @outOfCombat\n    function m(): boolean return true end\nend\n",
            "E8030",
        ),
    ];
    for (source, code) in cases {
        let stderr = check(source);
        let kind = if code.starts_with('W') {
            "warning"
        } else {
            "error"
        };
        assert!(
            stderr.contains(&format!("{kind}[{code}]")),
            "{code} expected for:\n{source}\n{stderr}"
        );
    }
}

/// Correct uses of the core modules pass their checks.
#[test]
fn core_modules_accept_correct_uses() {
    let dir = project(&[
        (
            "src/main.wowlua",
            r#"import event, unitEvent, send, on, onUnit from "wowlua:events"
import * as events from "wowlua:events"
import slash, router from "wowlua:slash"
import * as saved from "wowlua:saved"
import throttle from "wowlua:timer"
import outOfCombat from "wowlua:combat"
import onLoad from "wowlua:lifecycle"
import hook, hookScript from "wowlua:hooks"

@event("PLAYER_LOGIN", "STD_PING")
local function f(name: string, ...: any) end

@events.event("PLAYER_LOGOUT")
local function g() end

@unitEvent("UNIT_HEALTH", "player", "target")
local function h(unit: string) end

-- Any number of units.
@unitEvent("UNIT_MAXHEALTH", "party1", "party2", "party3", "party4")
local function party(unit: string) end
onUnit("UNIT_POWER_UPDATE", { "player", "target", "focus" }, function() end)
local watch = events.watchUnit({ "UNIT_HEALTH", "UNIT_MAXHEALTH" }, function(event: "UNIT_HEALTH" | "UNIT_MAXHEALTH", unit: string) end, {})
watch:setUnit("player")
watch:setUnit("target", "focus")
watch:setUnit(nil)
events.offAll(watch, "UNIT_HEALTH")

-- A message declared in another module.
send("STD_PING", 1)
on("BAG_UPDATE", function() end)
onUnit("UNIT_POWER_UPDATE", { "player" }, function() end)

class Bars
    @event("PLAYER_LOGIN")
    static function login() end

    @outOfCombat
    function layout() end

    @throttle(1)
    function refresh() end
end

-- Per-character data without profiles, and data with them.
local char = saved.bind("StdCharDB", { version = 1, defaults = { n = 1 } })
local db = saved.bind("StdDB", { version = 1, defaults = { n = 1 }, profiles = true })

local r = router({ go = { args = { "number", "rest" }, flags = { "all" }, run = function(a: number, b: string, flags: any) end } })
@slash("/stdgo")
local function cmd(msg: string) r:dispatch(msg) end

-- Handlers get their event's types; extra parameters are fine.
on("UNIT_HEALTH", function(unit) local s: string = unit end)
on("UNIT_HEALTH", function(unit: string, extra: any) end)
send("STD_PING", 2)
@event("UNIT_HEALTH")
local function health(unit) local s: string = unit end

-- Hooks of declared targets, of load-on-demand addons with `addon`, and of
-- named frames the addon creates.
hook("ToggleGameMenu", function() end)
hook(GameTooltip, "SetUnit", function() end)
hook("PlayerSpellsFrame", "Show", function() end, { addon = "Blizzard_PlayerSpells" })
@hookScript("PlayerSpellsFrame", "OnShow", { addon = "Blizzard_PlayerSpells" })
local function shown() end
local frame = CreateFrame("Frame", "StdOwnFrame")
hookScript("StdOwnFrame", "OnShow", function() end)
"#,
        ),
        (
            "src/later.wowlua",
            r#"import router from "wowlua:slash"
import * as callbacks from "wowlua:callbacks"
import * as secret from "wowlua:secret"
import * as tbl from "wowlua:table"
import * as log from "wowlua:log"
import * as secure from "wowlua:secure"
import defer from "wowlua:combat"
import * as errors from "wowlua:errors"
import repeating from "wowlua:timer"
import hook from "wowlua:hooks"

-- Typed results of an isolated call; listeners learn about the error.
local ok, n, s = errors.call(function(a: number): (number, string) return a, "" end, 1)
local typed: number, text: string, okay: boolean = n, s, ok
print(typed, text, okay)
errors.wrap(function(a: number, b: string) end)(1, "")
errors.onError(function(msg: string, info: errors.ErrorInfo)
    local component: string? = info.component
    local stack: string? = info.stack
    print(msg, component, stack, info.owner)
end)
errors.onError(function(msg: string) end)
local tick = repeating(0.5, function() end)
tick:setRunning(true)
if tick:isRunning() then tick:stop() end
tick:start()
hook(GameTooltip, "SetUnit", function() end, { owner = tick })

-- Commands that come and go, listed in the order they were added.
local owner = {}
local r = router({}, { order = "added" })
r:add("go", { args = { "number", "rest" }, run = function(a: number, b: string) end }, owner)
r:add("stop", { run = function() end, help = "Stop" })
for _, c in ipairs(r:commands()) do print(c.name, c.usage, c.help, #c.aliases) end
r:remove("stop")
r:removeAll(owner)

interface Ev
    Kill: (n: number) -> ()
end
local events: callbacks.Registry<Ev> = callbacks.registry()
local onKill = function(n: number) end
local sub = events:on("Kill", onKill, owner)
events:off("Kill", onKill)
events:off(sub)
events:offAll(owner, "Kill")
events:offAll(owner)

-- `known`: the value's type, joined with the fallback's.
local function show(name: string | Secret<string>, hp: number | Secret<number>)
    local maybe: string? = secret.known(name, nil)
    local sure: string = secret.known(name, "?")
    local n: number = secret.known(hp, 0)
    print(maybe, sure, n)
end
show("x", 1)

-- Truthy flags from legacy answers, and the readable narrowing.
local inRaid: boolean = secret.flag(UnitInRaid("player"))
local level = UnitLevel("player")
if secret.readable(level) and level > 10 then print(inRaid) end

local copied = tbl.copy({ a = { 1 } }, { shared = false })
local names = tbl.keys({ [1] = true, b = true }, function(a: number | string, b: number | string) return tostring(a) < tostring(b) end)
print(copied, names)

log.configure({ prefix = "|cff9fd3c7Std|r ", chatLevel = "debug" })
local header = CreateFrame("Frame", nil, UIParent, "SecureHandlerStateTemplate")
local paging = secure.state(header, "page", "[mod:shift] 2; 1")
paging:cancel()
defer(function() header:Show() end, "header")
"#,
        ),
        (
            "src/messages.wowlua",
            "declare message STD_PING(n: number)\n",
        ),
    ]);
    let stderr = build(dir.path(), "dev");
    assert!(
        !stderr.contains("[E8") && !stderr.contains("[W8"),
        "{stderr}"
    );
}

/// An enclosing existence test of the hooked member stands for its
/// declaration (no W8040), but a target of a load-on-demand addon still
/// needs `opts.addon` (W8102): before the addon loads, the test fails and
/// the hook is never installed.
#[test]
fn hooks_inside_existence_tests() {
    let stderr = check(
        r#"import hook, hookScript from "wowlua:hooks"
if _G.StdNoSuchFunction then
    hook("StdNoSuchFunction", function() end)
end
if ChatFrame1.StdNoSuchMethod then
    hook(ChatFrame1, "StdNoSuchMethod", function() end)
end
local frame = ChatFrame1
if frame.StdOtherMethod ~= nil then
    hook(frame, "StdOtherMethod", function() end)
end
if _G.CollectionsJournal_UpdateSelectedTab then
    hook("CollectionsJournal_UpdateSelectedTab", function() end)
end
hook(ChatFrame1, "StdNoSuchMethod", function() end)
if ChatFrame1 then
    hook(ChatFrame1, "StdNoSuchMethod", function() end)
end
if _G.StdNoSuchFunction then
    hookScript("StdNoSuchFrame", "OnShow", function() end)
end
"#,
    );
    let found: Vec<String> = stderr
        .lines()
        .filter(|l| l.starts_with("src/main.wowlua"))
        .filter_map(|l| {
            let line = l.split(':').nth(1)?;
            let code = l.split("warning[").nth(1)?.split(']').next()?;
            Some(format!("{line} {code}"))
        })
        .collect();
    assert_eq!(
        found,
        ["13 W8102", "15 W8040", "17 W8040", "20 W8040"],
        "{stderr}"
    );
}

/// The standard library's modules type-check clean, also in a strict
/// project, and their exports are typed for the code importing them.
#[test]
fn standard_library_is_typed_and_checks_clean() {
    let dir = project(&[
        ("locales/enUS.json", "{ \"TITLE\": \"Std\" }"),
        (
            "src/main.wowlua",
            r#"--!strict
import * as errors from "wowlua:errors"
import * as events from "wowlua:events"
import * as lifecycle from "wowlua:lifecycle"
import * as timer from "wowlua:timer"
import * as combat from "wowlua:combat"
import * as hooks from "wowlua:hooks"
import * as slash from "wowlua:slash"
import * as saved from "wowlua:saved"
import * as callbacks from "wowlua:callbacks"
import * as log from "wowlua:log"
import * as debug from "wowlua:debug"
import * as reflect from "wowlua:reflect"
import * as locale from "wowlua:locale"
import * as pool from "wowlua:pool"
import * as settings from "wowlua:settings"
import * as launcher from "wowlua:launcher"
import * as serialize from "wowlua:serialize"
import * as comm from "wowlua:comm"
print(comm)
print(errors, events, lifecycle, timer, combat, hooks, slash, saved, callbacks, log, debug, reflect)
print(locale, pool, settings, launcher, serialize)
local n: number = errors.count()
local loaded: boolean = lifecycle.isLoaded()
print(n, loaded)
"#,
        ),
    ]);
    let manifest = fs::read_to_string(dir.path().join("wowlua.json")).unwrap();
    let manifest = manifest.replacen('{', "{ \"check\": { \"mode\": \"strict\" },", 1);
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    let stderr = build(dir.path(), "dev");
    assert!(
        !stderr.contains("error") && !stderr.contains("warning"),
        "{stderr}"
    );
    // A wrong use of a typed export is an error.
    fs::write(
        dir.path().join("src/main.wowlua"),
        "import count from \"wowlua:errors\"\nlocal s: string = count()\nprint(s)\n",
    )
    .unwrap();
    let assert = wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("error[E2001]"), "{stderr}");
}

/// Functions under the standard library's decorators run later, not while
/// their files load: what they define does not order files (W1033).
#[test]
fn stdlib_decorators_do_not_run_at_load_time() {
    let dir = project(&[
        (
            "src/state.wowlua",
            "export function state(): any\n    if type(StdDB) ~= \"table\" then\n        StdDB = {}\n    end\n    return StdDB\nend\n",
        ),
        (
            "src/a.wowlua",
            "import onLogin from \"wowlua:lifecycle\"\nimport state from \"/state\"\n@onLogin\nlocal function login() state().a = 1 end\n",
        ),
        (
            "src/b.wowlua",
            "import * as events from \"wowlua:events\"\nimport state from \"/state\"\n@events.event(\"PLAYER_LOGIN\")\nlocal function login() state().b = 1 end\n",
        ),
    ]);
    let stderr = build(dir.path(), "dev");
    assert!(
        !stderr.contains("W1033") && !stderr.contains("W1034"),
        "{stderr}"
    );
}

/// Copies a directory tree (the in-game test addon, built outside the
/// repository).
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_name() == "dist" {
            continue;
        }
        if path.is_dir() {
            copy_tree(&path, &to.join(entry.file_name()));
        } else {
            fs::copy(&path, to.join(entry.file_name())).unwrap();
        }
    }
}

/// The in-game test addon (`tests/ingame`, `/stdtest` in WoW) builds
/// without diagnostics, and its automatic suites pass in the mock client.
#[test]
fn ingame_suite_builds_and_passes_in_the_mock() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ingame");
    let dir = tempfile::tempdir().unwrap();
    copy_tree(&source, dir.path());
    let stderr = build(dir.path(), "dev");
    // W8062: its German locale lacks a key on purpose (the fallback test).
    let unexpected = stderr
        .lines()
        .any(|l| (l.contains("warning[") && !l.contains("warning[W8062]")) || l.contains("error"));
    assert!(!unexpected, "{stderr}");
    let out = dir.path().join("dist/dev/StdTest");
    let toc = fs::read_to_string(out.join("StdTest_Camelot.toc")).unwrap();
    let field = |name: &str| {
        toc.lines()
            .find_map(|l| l.strip_prefix(&format!("## {name}: ")))
            .unwrap()
            .to_string()
    };
    let setup = format!(
        r#"Mock.interface = {}
Mock.metadata.StdTest = {{ Version = "{}" }}
Mock.onDemand.Blizzard_DebugTools = function() DevTools_Dump = function() end end
-- In a guild, whose messages come back to the sender.
Mock.guild = true
Mock.echoAs = "Tester"
"#,
        field("Interface"),
        field("Version")
    );
    let printed = load(
        &out,
        "StdTest",
        "StdTest_Camelot.toc",
        "",
        &setup,
        r#"Mock.loadAddon("StdTest")
Mock.fire("PLAYER_LOGIN")
Mock.fire("PLAYER_ENTERING_WORLD", true, false)
Mock.out = {}
Mock.slash("/stdtest auto")
for _ = 1, 1200 do
    Mock.advanceFrames(3, 50 / 3)
end
-- The Lua environment probe records without failing (wowlua's VM has the
-- client's additions: math.clamp returns its value).
Mock.slash("/stdtest probe")
local env, additions = StdTestState.luaEnv, StdTestState.luaEnvAdditions
print("probe recorded", env.version, env.libraries.os, env.calls["time()"], additions["math.clamp(5, 1, 3)"]:sub(1, 6))
"#,
    );
    let text = printed.join("\n");
    assert!(
        printed
            .iter()
            .any(|l| l == "probe recorded Lua 5.1 false ok: number 1 valu"),
        "{text}"
    );
    let summary = printed
        .iter()
        .find(|l| l.contains(" passed, "))
        .unwrap_or_else(|| panic!("no summary:\n{text}"));
    assert!(summary.contains(" 0 failed"), "{text}");
    for suite in [
        "errors",
        "events",
        "lifecycle",
        "timer",
        "combat",
        "hooks",
        "slash",
        "saved",
        "callbacks",
        "log",
        "debug",
        "reflect",
        "meta",
        "locale",
        "pool",
        "settings",
        "launcher",
        "serialize",
        "comm",
    ] {
        assert!(
            printed
                .iter()
                .any(|l| l.contains(&format!(" {suite} .")) && l.contains("PASS")),
            "{suite}:\n{text}"
        );
    }
}

/// The guided run of the in-game suite, with the mock client playing the
/// player: combat, a target change and a /reload (the SavedVariables are
/// written at logout and loaded into a fresh client, as WoW does).
#[test]
fn ingame_suite_guided_run_through_a_reload() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ingame");
    let dir = tempfile::tempdir().unwrap();
    copy_tree(&source, dir.path());
    build(dir.path(), "dev");
    let out = dir.path().join("dist/dev/StdTest");
    let toc = "StdTest_Camelot.toc";
    let toc_text = fs::read_to_string(out.join(toc)).unwrap();
    let field = |name: &str| {
        toc_text
            .lines()
            .find_map(|l| l.strip_prefix(&format!("## {name}: ")))
            .unwrap()
            .to_string()
    };
    let setup = format!(
        r#"Mock.interface = {}
Mock.metadata.StdTest = {{ Version = "{}" }}
Mock.onDemand.Blizzard_DebugTools = function() DevTools_Dump = function() end end
Mock.guild = true
Mock.echoAs = "Tester"
-- Advances the clock until a line containing `text` is printed.
function Mock.awaitText(text)
    for _ = 1, 4000 do
        for _, line in ipairs(Mock.out) do
            if line:find(text, 1, true) then return end
        end
        Mock.advanceFrames(3, 50 / 3)
    end
    error("never printed: " .. text .. "\n" .. table.concat(Mock.out, "\n", math.max(1, #Mock.out - 40)))
end
-- SavedVariables as WoW writes them (raw tables).
function Mock.serialize(v)
    if type(v) == "table" then
        local parts = {{}}
        for k, x in pairs(v) do
            parts[#parts + 1] = "[" .. Mock.serialize(k) .. "] = " .. Mock.serialize(x)
        end
        return "{{ " .. table.concat(parts, ", ") .. " }}"
    elseif type(v) == "string" then
        return string.format("%q", v)
    end
    return tostring(v)
end
"#,
        field("Interface"),
        field("Version")
    );
    let mut vm = client(&out, "StdTest", toc, "", &setup);
    exec_snippet(
        &mut vm,
        r#"Mock.loadAddon("StdTest")
Mock.fire("PLAYER_LOGIN")
Mock.fire("PLAYER_ENTERING_WORLD", true, false)
Mock.slash("/stdtest")
Mock.awaitText("Have any buff on you")
Mock.auras.player = { { name = "Well Fed", spellId = 19705, auraInstanceID = 9, isHelpful = true, applications = 0 } }
Mock.auraChanged("player")
Mock.awaitText("Press and release CTRL-SHIFT-K")
Mock.pressKey("CTRL-SHIFT-K")
Mock.awaitText("Enter combat")
Mock.fire("PLAYER_REGEN_DISABLED")
Mock.awaitText("Now leave combat")
Mock.fire("PLAYER_REGEN_ENABLED")
Mock.awaitText("Type wowlua into the StdTest prompt")
Mock.dialogAccept("wowlua")
Mock.awaitText("Change your target")
Mock.fire("PLAYER_TARGET_CHANGED")
Mock.fire("UNIT_TARGET", "player")
Mock.fire("UNIT_TARGET", "target")
Mock.awaitText("Drag the StdTest box")
Mock.modifiers = {}
Mock.drag(StdTestMovableBox, 400, 300)
Mock.awaitText("Resize the box")
for _, f in ipairs(Mock.frames()) do
    if f:GetParent() == StdTestMovableBox and f:GetObjectType() == "Button" then Mock.sizeBy(f, 60, 30) end
end
Mock.awaitText("Hold Ctrl")
Mock.modifiers.CTRL = true
Mock.wheel(StdTestMovableBox, 1)
Mock.modifiers.CTRL = nil
Mock.awaitText("Open Edit Mode")
Mock.editMode(true)
Mock.drag(Mock.selectionOf(StdTestEditModeBox), 50, 0)
Mock.awaitText("Close Edit Mode")
Mock.editMode(false)
Mock.awaitText("Click the question-mark button")
Mock.script(StdTestMinimapButton, "OnClick", "LeftButton")
Mock.awaitText("Click StdTest entry 3")
for _, f in ipairs(Mock.frames()) do
    if f.label and f.label:GetText() == "StdTest entry 3" then Mock.script(f, "OnMouseUp", "LeftButton") end
end
Mock.awaitText("Right-click the StdTest menu button")
Mock.script(StdTestMenuButton, "OnMouseUp", "RightButton")
Mock.clickMenu("Pick me")
Mock.awaitText("Move any item to another slot in your bags")
Mock.fire("BAG_UPDATE", 0)
Mock.fire("BAG_UPDATE", 1)
Mock.awaitText("Hold Shift for a moment")
-- Held for a moment, as the step asks.
Mock.modifiers.SHIFT = true
Mock.updateStateDrivers()
Mock.advanceFrames(10)
Mock.modifiers.SHIFT = nil
Mock.awaitText("Enter combat again")
Mock.fire("PLAYER_REGEN_DISABLED")
Mock.awaitText("Leave combat again")
Mock.fire("PLAYER_REGEN_ENABLED")
Mock.awaitText("Mouse over any item in your bags")
Mock.showTooltip("Item", { id = 6948, name = "Hearthstone", link = "|Hitem:6948::|h[Hearthstone]|h" })
Mock.awaitText("Click the [StdTest link]")
SetItemRef("addon:StdTest:guided", "[StdTest link]", "LeftButton")
Mock.awaitText("Type /reload")
Mock.fire("PLAYER_LOGOUT")
local saved = {}
for _, name in ipairs({ "StdTestState", "StdTestDB", "StdTestMigrateDB", "StdTestDBPerChar", "StdTestCharDB" }) do
    saved[#saved + 1] = name .. " = " .. Mock.serialize(_G[name])
end
Mock.saved = table.concat(saved, "\n")
"#,
        "@test",
    )
    .unwrap();
    let before = printed(&mut vm).join("\n");
    let saved = eval_string(&mut vm, "Mock.saved");
    drop(vm);
    // After the reload: the run continues by itself.
    let after = load(
        &out,
        "StdTest",
        toc,
        "",
        &format!("{setup}\n{saved}"),
        r#"Mock.loadAddon("StdTest")
Mock.fire("PLAYER_LOGIN")
Mock.fire("PLAYER_ENTERING_WORLD", false, true)
Mock.awaitText(" passed, ")
"#,
    );
    let text = after.join("\n");
    let summary = after.iter().find(|l| l.contains(" passed, ")).unwrap();
    assert!(summary.contains(" 0 failed"), "{before}\n----\n{text}");
    assert!(!summary.contains("skipped"), "{before}\n----\n{text}");
    for suite in [
        "combat (guided)",
        "events (target)",
        "saved (reload)",
        "saved (after reload)",
        "launcher (guided)",
        "timer",
    ] {
        assert!(
            after
                .iter()
                .any(|l| l.contains(&format!(" {suite} .")) && l.contains("PASS")),
            "{suite}:\n{before}\n----\n{text}"
        );
    }
}

/// W8101 (spec stdlib "events"): in shared flavor mode, an event some
/// selected flavor lacks; a flavor guard covers it.
#[test]
fn events_missing_from_a_flavor_warn_in_shared_mode() {
    let dir = project(&[(
        "src/main.wowlua",
        r#"import on, event from "wowlua:events"
on("PLAYER_LOGIN", function() end)
on("ACTIVE_COMBAT_CONFIG_CHANGED", function() end)
@event("ACTIVE_COMBAT_CONFIG_CHANGED")
local function f() end
if WOW_PROJECT_ID == WOW_PROJECT_MAINLINE then
    on("ACTIVE_COMBAT_CONFIG_CHANGED", function() end)
end
"#,
    )]);
    let manifest = fs::read_to_string(dir.path().join("wowlua.json")).unwrap();
    let manifest = manifest.replacen(
        '{',
        "{ \"flavors\": { \"standard\": {}, \"vanilla\": {} }, \"build\": { \"flavor-mode\": \"shared\" },",
        1,
    );
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    let stderr = build(dir.path(), "dev");
    let lines: Vec<&str> = stderr.lines().filter(|l| l.contains("W8101")).collect();
    assert_eq!(lines.len(), 2, "{stderr}");
    assert!(stderr.contains("is not available in `vanilla`"), "{stderr}");
}

// ----------------------------------------------------------------- locale --

const LOCALE_FILES: &[(&str, &str)] = &[
    (
        "locales/enUS.json",
        r#"{ "TITLE": "Kill Tracker", "WELCOME": "Welcome back, {name}!", "KILLS": "{count} kills in {zone}", "BRACES": "{{x}} \"q\"\nline", "COLOR": "|cffff0000red|r" }"#,
    ),
    (
        "locales/deDE.json",
        r#"{ "TITLE": "Abschusszähler", "WELCOME": "Willkommen zurück, {name}!", "KILLS": "{zone}: {count} Abschüsse ({count})" }"#,
    ),
    ("locales/enGB.json", r#"{ "TITLE": "Kill Tracker (GB)" }"#),
];

const LOCALE_MAIN: &str = r#"import L from "wowlua:locale"
print(L.TITLE)
print(L.WELCOME({ name = "Sua" }))
print(L.KILLS({ count = 3, zone = "Elwynn" }))
print(L.BRACES)
print(L.COLOR)
"#;

#[test]
fn locale_strings_follow_the_client_language() {
    let mut files = LOCALE_FILES.to_vec();
    files.push(("src/main.wowlua", LOCALE_MAIN));
    let out = modes(&files, "", "");
    assert_eq!(
        out,
        [
            "Kill Tracker",
            "Welcome back, Sua!",
            "3 kills in Elwynn",
            "{x} \"q\"\nline",
            "|cffff0000red|r"
        ],
        "{out:?}"
    );
    // German, with the base strings where the translation lacks one, and
    // the placeholders in its own order.
    let out = modes(&files, "Mock.locale = \"deDE\"", "");
    assert_eq!(
        out[..3],
        [
            "Abschusszähler",
            "Willkommen zurück, Sua!",
            "Elwynn: 3 Abschüsse (3)"
        ],
        "{out:?}"
    );
    assert_eq!(out[4], "|cffff0000red|r", "{out:?}");
    // British clients: enUS, then enGB over it.
    let out = modes(&files, "Mock.locale = \"enGB\"", "");
    assert_eq!(
        out[..2],
        ["Kill Tracker (GB)", "Welcome back, Sua!"],
        "{out:?}"
    );
    // Another language without a file: the base locale.
    let out = modes(&files, "Mock.locale = \"frFR\"", "");
    assert_eq!(out[0], "Kill Tracker", "{out:?}");
}

#[test]
fn locale_files_load_first_with_their_client_tags() {
    let mut files = LOCALE_FILES.to_vec();
    files.push(("src/main.wowlua", LOCALE_MAIN));
    let dir = project(&files);
    let stderr = build(dir.path(), "dev");
    // deDE lacks two keys.
    assert!(
        stderr.contains("W8062") && stderr.contains("`deDE` lacks 2 of 5"),
        "{stderr}"
    );
    let toc = fs::read_to_string(dir.path().join("dist/dev/Std/Std.toc")).unwrap();
    let lines: Vec<&str> = toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert_eq!(
        lines[..4],
        [
            "wowlua\\locales\\enUS.lua",
            "wowlua\\locales\\deDE.lua [AllowLoadTextLocale deDE]",
            "wowlua\\locales\\enGB.lua [AllowLoadTextLocale enGB]",
            "wowlua\\runtime.lua"
        ],
        "{toc}"
    );
    // Release builds bundle the code, never the locale files.
    build(dir.path(), "release");
    let toc = fs::read_to_string(dir.path().join("dist/release/Std/Std.toc")).unwrap();
    assert!(
        toc.contains("wowlua\\locales\\enUS.lua\nwowlua\\locales\\deDE.lua"),
        "{toc}"
    );
    assert!(
        dir.path()
            .join("dist/release/Std/wowlua/locales/enUS.lua")
            .exists()
    );
}

#[test]
fn locales_all_in_one_file() {
    let mut files = LOCALE_FILES.to_vec();
    files.push(("src/main.wowlua", LOCALE_MAIN));
    let dir = project(&files);
    let manifest = fs::read_to_string(dir.path().join("wowlua.json")).unwrap();
    fs::write(
        dir.path().join("wowlua.json"),
        manifest.replacen('{', "{ \"build\": { \"locales\": \"all\" },", 1),
    )
    .unwrap();
    for (locale, title) in [
        ("deDE", "Abschusszähler"),
        ("enGB", "Kill Tracker (GB)"),
        ("frFR", "Kill Tracker"),
    ] {
        let out = modes_in(&dir, &format!("Mock.locale = \"{locale}\""), "");
        assert_eq!(out[0], title, "{locale}: {out:?}");
    }
    let toc = fs::read_to_string(dir.path().join("dist/dev/Std/Std.toc")).unwrap();
    assert!(toc.contains("wowlua\\locales\\locales.lua\n"), "{toc}");
}

/// The locale files' checks, and `L`'s type.
#[test]
fn locale_checks() {
    let main = "import L from \"wowlua:locale\"\nprint(L.TITLE)\n";
    type Case<'a> = (&'a [(&'a str, &'a str)], &'a str, &'a str);
    let cases: &[Case] = &[
        (
            &[],
            "E8106",
            "the base locale `locales/enUS.json` is missing",
        ),
        (
            &[
                ("locales/enUS.json", "{ \"TITLE\": \"x\" }"),
                ("locales/english.json", "{}"),
            ],
            "E8105",
            "`english` is no locale code",
        ),
        (
            &[(
                "locales/enUS.json",
                "{ \"TITLE\": \"x\", \"bad key\": \"y\" }",
            )],
            "E8060",
            "`bad key`",
        ),
        (
            &[
                ("locales/enUS.json", "{ \"TITLE\": \"x\" }"),
                (
                    "locales/deDE.json",
                    "{ \"TITLE\": \"x\", \"EXTRA\": \"y\" }",
                ),
            ],
            "E8061",
            "`EXTRA` in `locales/deDE.json`",
        ),
        (
            &[
                ("locales/enUS.json", "{ \"TITLE\": \"{name}\" }"),
                ("locales/deDE.json", "{ \"TITLE\": \"{nom}\" }"),
            ],
            "E8063",
            "placeholders [{nom}], the base locale [{name}]",
        ),
        (
            &[("locales/enUS.json", "{ \"TITLE\": 1 }")],
            "E8115",
            "must be a string",
        ),
        (
            &[("locales/enUS.json", "[ \"x\" ]")],
            "E8115",
            "must be an object",
        ),
        (
            &[("locales/enUS.json", "{ nope")],
            "E8115",
            "is not valid JSON",
        ),
    ];
    for (locales, code, text) in cases {
        let mut files = locales.to_vec();
        files.push(("src/main.wowlua", main));
        let dir = project(&files);
        let assert = wowlua(dir.path()).arg("build").assert().failure();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(
            stderr.contains(&format!("error[{code}]")) && stderr.contains(text),
            "{code}: {stderr}"
        );
    }
    // `L` is typed by the base locale.
    let typed: &[(&str, &str)] = &[
        ("print(L.TITEL)\n", "E2003"),
        ("print(L.WELCOME())\n", "E2007"),
        ("local n: number = L.TITLE\nprint(n)\n", "E2001"),
        // A secret value makes a secret string.
        (
            "local s: string = L.WELCOME({ name = UnitName(\"player\") })\nprint(s)\n",
            "E2001",
        ),
    ];
    for (code, expected) in typed {
        let mut files = LOCALE_FILES.to_vec();
        let source = format!("import L from \"wowlua:locale\"\n{code}");
        files.push(("src/main.wowlua", &source));
        let dir = project(&files);
        let assert = wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .assert()
            .failure();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(
            stderr.contains(&format!("error[{expected}]")),
            "{code}: {stderr}"
        );
    }
}

/// Without `wowlua:locale` the locale folder is not read (adopted addons
/// keep their own `locales/` of Lua files).
#[test]
fn locales_unused_without_the_module() {
    let dir = project(&[
        ("locales/enUS.json", "{ nope"),
        (
            "locales/deDE.lua",
            "print(\"an addon's own locale file\")\n",
        ),
        ("src/main.wowlua", "print(\"main\")\n"),
    ]);
    let stderr = build(dir.path(), "dev");
    assert!(
        !stderr.contains("E81") && !stderr.contains("W8062"),
        "{stderr}"
    );
    assert!(!dir.path().join("dist/dev/Std/wowlua/locales").exists());
}

// ------------------------------------------------------------------- pool --

#[test]
fn pools_reuse_tables_and_frames() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import FramePool, TablePool from "wowlua:pool"
local C = Mock.check

-- Tables: wiped on release, reused, capped by `max`.
local tables = TablePool.new({ max = 1 })
local a, newA = tables:acquire()
a.x = 1
local b = tables:acquire()
C(newA, "a new table")
print("count", tables:count())
tables:release(a)
tables:release(b)
print("after release", tables:count())
local again, newAgain = tables:acquire()
C(again == a and not newAgain and next(again) == nil, "the wiped table is reused")

-- Frames of a class: made with the constructor's arguments, reset on release.
class Row
    label: string
    released: number = 0
    frame: any
    constructor(label: string, n: number)
        self.label = label .. n
        self.frame = CreateFrame("Frame")
    end
    function Hide() self.frame:Hide() end
    function ClearAllPoints() print("points cleared", self.label) end
    function onRelease() self.released += 1 end
end

local rows = FramePool.new(Row, "row", 1)
local r1 = rows:acquire()
local r2 = rows:acquire()
print("rows", r1.label, rows:count())
local seen = 0
for row in rows:active() do
    seen += 1
end
C(seen == 2, "active iterates the acquired rows")
rows:releaseAll()
C(r1.released == 1 and r2.released == 1, "onRelease ran")
print("rows after releaseAll", rows:count())
C(rows:acquire() ~= nil, "reacquired")

-- Plain frames from a template.
local frames = FramePool.fromTemplate("Button", UIParent)
local f, made = frames:acquire()
C(made and f:GetObjectType() == "Button", "a new button")
f:Show()
frames:release(f)
C(not f:IsShown(), "hidden on release")
C(frames:acquire() == f, "reused")

-- A protected frame released in combat: released once combat ends.
local guarded = FramePool.fromTemplate("Frame")
local g = guarded:acquire()
g.IsProtected = function() return true end
_G.StdPool = { pool = guarded, frame = g }
"#,
        )],
        "",
        r#"Mock.fire("PLAYER_REGEN_DISABLED")
StdPool.pool:release(StdPool.frame)
print("in combat", StdPool.pool:count())
Mock.fire("PLAYER_REGEN_ENABLED")
print("after combat", StdPool.pool:count())
"#,
    );
    assert_eq!(
        out,
        [
            "count 2 0",
            "after release 0 1",
            "rows row1 2 0",
            "points cleared row1",
            "points cleared row1",
            "rows after releaseAll 0 2",
            "in combat 1 0",
            "after combat 0 1",
        ],
        "{out:?}"
    );
}

/// Releasing twice or a foreign object is an error in debug builds only.
#[test]
fn pool_misuse_raises_in_debug_builds() {
    let dir = project(&[(
        "src/main.wowlua",
        r#"import TablePool from "wowlua:pool"
local tables = TablePool.new()
local t = tables:acquire()
tables:release(t)
print("twice", pcall(tables.release, tables, t))
print("foreign", pcall(tables.release, tables, {}))
"#,
    )]);
    for mode in MODES {
        build(dir.path(), mode);
        let out = run(dir.path(), mode, "", "");
        let expected = if *mode == "dev" { "false" } else { "true" };
        assert!(
            out[0].starts_with(&format!("twice {expected}")),
            "{mode}: {out:?}"
        );
        assert!(
            out[1].starts_with(&format!("foreign {expected}")),
            "{mode}: {out:?}"
        );
    }
}

// --------------------------------------------------------------- bindings --

const BINDINGS: &str = r#"import binding, header from "wowlua:bindings"
import KeyState from "wowlua:bindings"
local C = Mock.check

const MAIN = "STD"
const NEVER = false
header(MAIN, "Std Addon")

@binding("STD_PLAIN", { label = "Plain" })
local function plain()
    print("plain")
end

@binding("STD_TOGGLE", { label = "Toggle", header = MAIN })
local function toggle()
    print("toggle")
end

@binding("STD_PUSH", { label = `Push {1 + 1}`, header = MAIN, runOnUp = true })
local function push(state: KeyState)
    print(`push {state}`)
end

@binding("STD_BOOM", { label = "Boom", header = "DEV" })
local function boom()
    error("boom")
end

@if(NEVER)
@binding("STD_NEVER", { label = "Never", header = MAIN })
local function never()
    print("never")
end

C(BINDING_HEADER_STD == "Std Addon", "the header's label")
C(BINDING_NAME_STD_PUSH == "Push 2", "a computed label")
"#;

/// `wowlua:bindings`: the generated Bindings.xml calls the handlers through
/// the addon's dispatch table, in every build mode.
#[test]
fn bindings_run_through_the_generated_file() {
    let dir = project(&[("src/main.wowlua", BINDINGS)]);
    build(dir.path(), "dev");
    let xml = fs::read_to_string(dir.path().join("dist/dev/Std/Bindings.xml")).unwrap();
    assert_eq!(
        xml,
        "<Bindings>\n  <Binding name=\"STD_PLAIN\">Std_Bindings[\"STD_PLAIN\"](keystate)</Binding>\n  <Binding name=\"STD_TOGGLE\" header=\"STD\">Std_Bindings[\"STD_TOGGLE\"](keystate)</Binding>\n  <Binding name=\"STD_PUSH\" runOnUp=\"true\">Std_Bindings[\"STD_PUSH\"](keystate)</Binding>\n  <Binding name=\"STD_BOOM\" header=\"DEV\">Std_Bindings[\"STD_BOOM\"](keystate)</Binding>\n</Bindings>\n"
    );
    let out = modes_in(
        &dir,
        "",
        r#"Mock.pressBinding("STD_PLAIN")
Mock.pressBinding("STD_PLAIN", "up")
Mock.pressBinding("STD_TOGGLE")
Mock.pressBinding("STD_PUSH")
Mock.pressBinding("STD_PUSH", "up")
Mock.pressBinding("STD_BOOM")
Std_Bindings.STD_NEVER("down")
print("labels", BINDING_NAME_STD_PLAIN, BINDING_NAME_STD_TOGGLE)
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "plain",
            "toggle",
            "push down",
            "push up",
            "labels Plain Toggle"
        ]
    );
    // A failing handler is reported, not raised into the client.
    assert!(errors(&out).iter().any(|e| e.contains("boom")), "{out:?}");
}

/// `wowlua:bindings`' compile-time checks (E8350–E8353, W8354, E4334).
#[test]
fn bindings_are_checked_at_compile_time() {
    let import = "import binding, header from \"wowlua:bindings\"\n";
    let cases: &[(&str, &str)] = &[
        (
            "local n = \"STD_X\"\n@binding(n, { label = \"x\" })\nlocal function f() end\n",
            "E8350",
        ),
        (
            "local h = \"H\"\n@binding(\"STD_X\", { label = \"x\", header = h })\nlocal function f() end\n",
            "E8350",
        ),
        (
            "do\n    @binding(\"STD_X\", { label = \"x\" })\n    local function f() end\nend\n",
            "E8350",
        ),
        (
            "class A\n    @binding(\"STD_X\", { label = \"x\" })\n    static function f() end\nend\n",
            "E8350",
        ),
        ("if true then\n    header(\"H\", \"x\")\nend\n", "E8350"),
        (
            "@binding(\"std_x\", { label = \"x\" })\nlocal function f() end\n",
            "E8352",
        ),
        (
            "@binding(\"STD_X\", { label = \"x\" })\nlocal function f() end\n@binding(\"STD_X\", { label = \"y\" })\nlocal function g() end\n",
            "E8351",
        ),
        (
            "@binding(\"STD_X\", { label = \"x\", default = \"CTRL-X\" })\nlocal function f() end\n",
            "W8354",
        ),
    ];
    for (body, code) in cases {
        let out = check(&format!("{import}{body}"));
        assert!(out.contains(code), "{code} expected for:\n{body}\n{out}");
    }
    let out = check(
        "import * as settings from \"wowlua:settings\"\nlocal c = settings.keybinding(\"STD_NOPE\", { label = \"x\" })\nprint(c)\n",
    );
    assert!(out.contains("E8353"), "{out}");
    // Constants count as constant arguments.
    let out = check(&format!(
        "{import}const N = \"STD_X\"\nconst H = \"STD\"\nheader(H, \"x\")\n@binding(N, {{ label = \"x\", header = H, runOnUp = not false }})\nlocal function f() end\n"
    ));
    assert!(!out.contains("E83"), "{out}");
    // A hand-written Bindings.xml and `@binding` together.
    let dir = project(&[(
        "src/main.wowlua",
        &format!("{import}@binding(\"STD_X\", {{ label = \"x\" }})\nlocal function f() end\n"),
    )]);
    fs::write(dir.path().join("Bindings.xml"), "<Bindings></Bindings>\n").unwrap();
    let assert = wowlua(dir.path()).args(["build"]).assert().failure();
    assert!(String::from_utf8_lossy(&assert.get_output().stderr).contains("E4334"));
}

// ---------------------------------------------------------------- tooltip --

const TOOLTIP: &str = r#"import onTooltip, attach from "wowlua:tooltip"
import * as tooltip from "wowlua:tooltip"
import type TooltipData from "wowlua:tooltip"

@onTooltip("Item")
local function item(tip: GameTooltip, data: TooltipData)
    tip:AddLine(`item {data.id} {data.link}`)
end

@onTooltip("Unit")
local function unit(tip: GameTooltip, data: TooltipData)
    tip:AddLine(`unit {data.unit} {tostring(data.guid)}`)
end

@onTooltip("Spell")
local function boom()
    error("boom")
end

@onTooltip("Spell")
local function spell(tip: GameTooltip, data: TooltipData)
    tip:AddLine(`spell {data.id}`)
end

_G.StdTip = tooltip.hook("Item", function(tip: GameTooltip) tip:AddLine("second") end)
_G.StdAttach = attach
"#;

const TOOLTIP_SCRIPT: &str = r#"local thunderfury = { id = 19019, name = "Thunderfury", link = "|cffff8000|Hitem:19019::|h[Thunderfury]|h|r" }
local function show(kind, data, tip)
    print(kind .. ": " .. table.concat(Mock.showTooltip(kind, data, tip), " | "))
end
show("Item", thunderfury)
show("Unit", { unit = "target", name = "Hogger", guid = "GUID-target" })
show("Spell", { id = 133, name = "Fireball" })
StdTip:disable()
show("Item", thunderfury)
print("enabled", StdTip:isEnabled())
StdTip:enable()
show("Item", thunderfury, ItemRefTooltip)
local custom = Mock.tooltip("StdCustomTip")
StdAttach(custom)
StdAttach(custom)
show("Item", thunderfury, custom)
"#;

/// `wowlua:tooltip`: the same post-hooks with the tooltip data system and
/// through the tooltips' scripts, in every build mode.
#[test]
fn tooltip_post_hooks_with_and_without_the_data_system() {
    let link = "|cffff8000|Hitem:19019::|h[Thunderfury]|h|r";
    let expected = [
        format!("Item: item 19019 {link} | second"),
        "Unit: unit target GUID-target".to_string(),
        "Spell: spell 133".to_string(),
        format!("Item: item 19019 {link}"),
        "enabled false".to_string(),
        format!("Item: item 19019 {link} | second"),
        format!("Item: item 19019 {link} | second"),
    ];
    for setup in ["", "TooltipDataProcessor = nil"] {
        let out = modes(&[("src/main.wowlua", TOOLTIP)], setup, TOOLTIP_SCRIPT);
        assert_eq!(plain(&out), expected, "setup {setup:?}");
        // A failing handler is reported; the others still run.
        assert_eq!(
            errors(&out).iter().filter(|e| e.contains("boom")).count(),
            1,
            "{out:?}"
        );
    }
}

/// Tooltip kinds are checked per targeted flavor (E8330, E8300).
#[test]
fn tooltip_kinds_are_checked_per_flavor() {
    let check_in = |flavors: &str, body: &str| {
        let dir = project(&[(
            "src/main.wowlua",
            &format!("import onTooltip, hook from \"wowlua:tooltip\"\n{body}"),
        )]);
        fs::write(
            dir.path().join("wowlua.json"),
            format!(r#"{{ "package": {{ "name": "std", "version": "1.0.0" }}, "flavors": {{ {flavors} }}, "build": {{ "flavor-mode": "split" }} }}"#),
        )
        .unwrap();
        let assert = wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .assert();
        String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
    };
    let both = r#""camelot": {}, "vanilla": {}"#;
    let out = check_in(both, "hook(\"Itme\", function() end)\n");
    assert!(out.contains("E8330"), "{out}");
    let out = check_in(both, "hook(\"Mount\", function() end)\n");
    assert!(out.contains("E8300") && out.contains("vanilla"), "{out}");
    let out = check_in(r#""camelot": {}"#, "hook(\"Mount\", function() end)\n");
    assert!(!out.contains("E83"), "{out}");
    // Guarded by `@if`, a kind only some flavors have is fine.
    let out = check_in(
        both,
        "import flavor from \"wowlua:meta\"\n@if(flavor == \"camelot\")\n@onTooltip(\"Currency\")\nlocal function f() end\nhook(\"Item\", function() end)\n",
    );
    assert!(!out.contains("E83"), "{out}");
}

// ----------------------------------------------------------------- frames --

const FRAMES: &str = r#"import * as frames from "wowlua:frames"
import type FramePosition from "wowlua:frames"
import * as saved from "wowlua:saved"

interface Layout
    window?: FramePosition
    guarded?: FramePosition
end

local db = saved.bind("StdDB", { version = 1, defaults = {} :: Layout })

local window = CreateFrame("Frame", "StdWindow", UIParent)
window:SetSize(300, 200)
window:SetPoint("CENTER")
local changes = 0
_G.StdMove = frames.movable(window, {
    store = db,
    key = "window",
    resize = { min = { 200, 120 }, max = { 500, 400 } },
    scaleWheel = "CTRL",
    onChange = function(pos: FramePosition?) changes += 1 end,
})

local guarded = CreateFrame("Frame", "StdGuarded", UIParent)
guarded:SetSize(100, 100)
guarded:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10)
_G.StdGuardedMove = frames.movable(guarded, { store = db, key = "guarded", modifier = "SHIFT" })

_G.StdFrames = { db = db, changes = function() return changes end }
"#;

const FRAMES_SCRIPT: &str = r#"Mock.loadAddon("Std")
local db = StdFrames.db
local function show(label, pos)
    if not pos then
        print(label, "nil")
        return
    end
    print(label, pos.point, pos.relativePoint, math.floor(pos.x + 0.5), math.floor(pos.y + 0.5), pos.width or "-", pos.height or "-", pos.scale or "-")
end
-- Dragged into the top right third: anchored there.
Mock.drag(StdWindow, 300, 250)
show("dragged", db.window)
-- Resized by the built-in grip (clamped to the bounds).
local grip = StdWindow.grip
for _, f in ipairs(Mock.frames()) do
    if f:GetParent() == StdWindow and f:GetObjectType() == "Button" then grip = f end
end
Mock.sizeBy(grip, 300, 50)
show("resized", db.window)
-- Ctrl + wheel scales around the anchor.
Mock.wheel(StdWindow, 1)
show("no ctrl", db.window)
Mock.modifiers.CTRL = true
Mock.wheel(StdWindow, 2)
Mock.modifiers.CTRL = nil
show("scaled", db.window)
-- Locked: nothing moves.
StdMove:lock(true)
Mock.drag(StdWindow, -300, 0)
Mock.sizeBy(grip, -100, 0)
print("locked", StdMove:isLocked(), grip:IsShown())
show("still", db.window)
StdMove:lock(false)
-- Back to the original anchor, size and scale.
StdMove:reset()
show("reset", db.window)
print("anchor", StdWindow:GetPoint(1), StdWindow:GetWidth(), StdWindow:GetHeight(), StdWindow:GetScale())
print("changes", StdFrames.changes())
-- A modifier is needed to drag the second frame.
Mock.drag(StdGuarded, 500, 0)
show("no shift", db.guarded)
Mock.modifiers.SHIFT = true
Mock.drag(StdGuarded, 450, 0)
Mock.modifiers.SHIFT = nil
show("shift", db.guarded)
-- Protected frames stay put in combat; a restore waits for its end.
Mock.protect(StdGuarded)
Mock.fire("PLAYER_REGEN_DISABLED")
Mock.modifiers.SHIFT = true
Mock.drag(StdGuarded, -200, 0)
Mock.modifiers.SHIFT = nil
db.guarded = { point = "TOPLEFT", relativePoint = "TOPLEFT", x = 20, y = -20 }
StdGuardedMove:restore()
local function anchor(label, frame)
    local point, rel, relPoint, x, y = frame:GetPoint(1)
    print(label, point, rel:GetName(), relPoint, x, y)
end
anchor("in combat", StdGuarded)
Mock.fire("PLAYER_REGEN_ENABLED")
anchor("after combat", StdGuarded)
"#;

/// `wowlua:frames`: the store's field is `FramePosition?` (E8340).
#[test]
fn frames_store_field_is_checked() {
    let source = |field: &str, key: &str| {
        format!(
            "import * as frames from \"wowlua:frames\"\nimport type FramePosition from \"wowlua:frames\"\nimport * as saved from \"wowlua:saved\"\ninterface L\n    {field}\nend\nlocal db = saved.bind(\"StdDB\", {{ version = 1, defaults = {{}} :: L }})\nlocal f = CreateFrame(\"Frame\", nil, UIParent)\nframes.movable(f, {{ store = db, key = \"{key}\" }})\n"
        )
    };
    let out = check(&source("pos?: FramePosition", "pos"));
    assert!(!out.contains("E8340"), "{out}");
    let out = check(&source("pos?: number", "pos"));
    assert!(out.contains("E8340"), "{out}");
    let out = check(&source("pos: FramePosition", "pos"));
    assert!(
        out.contains("E8340"),
        "a required field cannot start empty: {out}"
    );
    let out = check(&source("pos?: FramePosition", "other"));
    assert!(out.contains("E8340"), "{out}");
}

/// `wowlua:frames`: a saved position is applied once the store loads.
#[test]
fn frames_restore_the_saved_position_at_load() {
    let out = modes(
        &[("src/main.wowlua", FRAMES)],
        r#"StdDB = { __version = 1, window = { point = "TOPLEFT", relativePoint = "TOPLEFT", x = 40, y = -60, width = 420, height = 260, scale = 0.8 } }"#,
        r#"local p, r, rp, x, y = StdWindow:GetPoint(1)
print("before", p, r:GetName(), rp, x, y)
Mock.loadAddon("Std")
local point, rel, relPoint, x, y = StdWindow:GetPoint(1)
print("after", point, rel:GetName(), relPoint, x, y, StdWindow:GetWidth(), StdWindow:GetHeight(), StdWindow:GetScale())
"#,
    );
    // Offsets are in the frame's own (scaled) units: 40 / 0.8, -60 / 0.8.
    assert_eq!(
        plain(&out),
        [
            "before CENTER UIParent CENTER 0 0",
            "after TOPLEFT UIParent TOPLEFT 50 -75 420 260 0.8"
        ]
    );
}

/// `wowlua:frames`: dragging, resizing, wheel scaling, locking, resetting
/// and restoring, with positions anchored at the frame's screen third.
#[test]
fn frames_move_resize_scale_and_save() {
    let out = modes(&[("src/main.wowlua", FRAMES)], "", FRAMES_SCRIPT);
    assert_eq!(
        plain(&out),
        [
            // The screen's middle third (1920 x 1080): anchored at its top.
            "dragged TOP TOP 300 -190 300 200 1",
            // Grown into the right third: anchored there.
            "resized TOPRIGHT TOPRIGHT -310 -190 500 250 1",
            "no ctrl TOPRIGHT TOPRIGHT -310 -190 500 250 1",
            "scaled TOPRIGHT TOPRIGHT -310 -190 500 250 1.1",
            "locked true false",
            "still TOPRIGHT TOPRIGHT -310 -190 500 250 1.1",
            "reset nil",
            "anchor CENTER 300 200 1",
            "changes 4",
            "no shift nil",
            "shift BOTTOMLEFT BOTTOMLEFT 460 10 - - 1",
            "in combat BOTTOMLEFT UIParent BOTTOMLEFT 460 10",
            "after combat TOPLEFT UIParent TOPLEFT 20 -20",
        ],
        "{out:?}"
    );
}

/// `frames.movable` with `editMode`: the client's selection overlay in Edit
/// Mode, a position per layout, locked outside Edit Mode unless
/// `lock = false`.
#[test]
fn frames_join_edit_mode() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as frames from "wowlua:frames"
import type FramePosition from "wowlua:frames"
import * as saved from "wowlua:saved"

interface Layout
    box?: FramePosition
    free?: FramePosition
end

local db = saved.bind("StdDB", { version = 1, defaults = {} :: Layout })
local box = CreateFrame("Frame", "StdBox", UIParent)
box:SetSize(100, 100)
box:SetPoint("CENTER")
frames.movable(box, { store = db, key = "box", editMode = { label = "Std Box" } })
local free = CreateFrame("Frame", "StdFree", UIParent)
free:SetSize(100, 100)
free:SetPoint("CENTER")
frames.movable(free, { store = db, key = "free", editMode = { label = "Std Free", lock = false } })
"#,
        )],
        "",
        r#"Mock.loadAddon("Std")
local box, sel = StdBox, Mock.selectionOf(StdBox)
local function at(f) local p, _, _, x, y = f:GetPoint(1) return p .. " " .. x .. " " .. y end
print("overlay", sel ~= nil, sel:IsShown(), sel.system:GetSystemName())
Mock.drag(box, 50, 0)
print("locked outside", StdDB.box)
Mock.drag(StdFree, 50, 0)
print("lock = false", StdDB.free ~= nil)
Mock.editMode(true)
print("in edit mode", sel:IsShown(), sel.mode)
Mock.drag(sel, 100, 0)
print("Modern", at(box), StdDB.box.layouts.Modern.x, StdDB.box.x)
Mock.script(sel, "OnMouseDown", "LeftButton")
print("selected", sel.mode, Mock.editModeCleared)
Mock.editModeSwitch("Raid")
print("Raid (no own position)", at(box))
Mock.drag(sel, 0, -100)
print("Raid moved", at(box), StdDB.box.layouts.Raid.y)
Mock.editModeSwitch("Modern")
print("Modern again", at(box))
Mock.editMode(false)
print("left", sel:IsShown())
Mock.drag(box, 50, 0)
print("locked again", at(box))
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "overlay true false Std Box",
            "locked outside nil",
            "lock = false true",
            "in edit mode true highlighted",
            "Modern BOTTOMLEFT 1010 490 100 100",
            "selected selected 1",
            // A layout without its own position takes the last one saved.
            "Raid (no own position) CENTER 100 0",
            "Raid moved BOTTOMLEFT 1010 390 -100",
            "Modern again CENTER 100 0",
            "left false",
            "locked again CENTER 100 0",
        ]
    );
}

// ----------------------------------------------------------------- secret --

/// `secret.known(v, fallback)` is typed `T | F`: a nil fallback keeps the
/// result optional, a value of `T` makes it plain (strict mode).
#[test]
fn secret_known_joins_the_fallback_type() {
    let dir = project(&[(
        "src/main.wowlua",
        r#"import * as secret from "wowlua:secret"
local function show(name: string | Secret<string>)
    local maybe: string? = secret.known(name, nil)
    local sure: string = secret.known(name, "?")
    local wrong: string = secret.known(name, nil)
    print(maybe, sure, wrong)
end
show("x")
"#,
    )]);
    let manifest = dir.path().join("wowlua.json");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replacen('{', r#"{ "check": { "mode": "strict" },"#, 1),
    )
    .unwrap();
    let assert = wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    let errors: Vec<&str> = stderr.lines().filter(|l| l.contains("error[")).collect();
    assert_eq!(errors.len(), 1, "{stderr}");
    assert!(
        errors[0].contains("E2001")
            && errors[0].contains("main.wowlua:5")
            && errors[0].contains("string?"),
        "{stderr}"
    );
}

/// `wowlua:secret`: helpers that test for secrecy before anything else.
#[test]
fn secret_helpers() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as secret from "wowlua:secret"
_G.StdSecret = secret
"#,
        )],
        "",
        r#"Mock.loadAddon("Std")
local secret = StdSecret
local s = Mock.secret()
print(secret.isSecret(s), secret.isSecret(1), secret.known(s, 5), secret.known(nil, 6), secret.known(7, 0))
print(secret.flag(s), secret.flag(true), secret.flag(nil), secret.flag(false))
print(secret.amount(s) == s, secret.amount(nil), secret.amount(3))
-- `flag` by truthiness (legacy 1/nil answers); `readable`: present, not secret.
print(secret.flag(1), secret.flag(0), secret.flag("yes"), secret.readable(s), secret.readable(nil), secret.readable(false), secret.readable(0))
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "true false 5 6 7",
            "false true false false",
            "true 0 3",
            "true true true false false true true",
        ]
    );
}

// ------------------------------------------------------------------- unit --

/// `wowlua:unit`: unit records (secrets passed through), names with realms,
/// health, class colors and group iteration.
#[test]
fn unit_info_health_and_groups() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as unit from "wowlua:unit"
_G.StdUnit = unit
"#,
        )],
        "",
        r#"Mock.loadAddon("Std")
local unit = StdUnit
local function show(i)
    print(i.name, i.realm, i.fullName, i.class, i.classFile, i.level, i.isPlayer, i.isConnected, i.faction, i.race, i.role)
end
show(unit.info("player"))
print(unit.info("target"), unit.exists("target"), unit.fullName("target"))
Mock.units.party1 = { name = "Ally", realm = "Other", guid = "Player-2-1", class = "Warrior", classFile = "WARRIOR", level = 60, health = 10, healthMax = 40, power = 0, powerMax = 100, isPlayer = true, connected = true }
show(unit.info("party1"))
-- A secret name: passed through, no full name.
Mock.units.target = { name = Mock.secret(), guid = Mock.secret(), class = "Mage", classFile = Mock.secret(), level = 70, health = Mock.secret(), healthMax = Mock.secret(), power = 0, powerMax = 0, isPlayer = false, connected = true }
local t = unit.info("target")
print(issecretvalue(t.name), t.fullName, unit.fullName("target"), unit.classColor("target"))
print(unit.health("player"))
print(unit.healthPercent("party1"), unit.healthPercent("target"))
print(issecretvalue((unit.health("target"))))
print(unit.classColor("player"):GenerateHexColor())
local function members(opts)
    local out = {}
    for token in unit.group(opts) do out[#out + 1] = token end
    return table.concat(out, " ")
end
print("solo", members(), members({ includePlayer = true }), unit.groupSize())
Mock.group = "party"
Mock.units.party2 = Mock.units.target
Mock.units.partypet1 = { name = "Wolf", guid = "Pet-1", class = "", classFile = "", level = 1, health = 1, healthMax = 1, power = 0, powerMax = 0, isPlayer = false, connected = true }
print("party", members(), members({ includePlayer = true, includePets = true }), unit.groupSize())
Mock.group = "raid"
Mock.units.raid1 = Mock.units.player
Mock.units.raid2 = Mock.units.party1
print("raid", members(), members({ includePlayer = true }), unit.groupSize())
print(unit.isSame("raid1", "player"), unit.isSame("raid2", "player"))
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "Tester nil Tester-MockRealm Mage MAGE 70 true true Alliance Human NONE",
            "nil false nil",
            "Ally Other Ally-Other Warrior WARRIOR 60 true true nil nil NONE",
            "true nil nil nil",
            "800 1000",
            "25 nil",
            "true",
            "ff40c7eb",
            "solo player player 1",
            "party party1 party2 player party1 partypet1 party2 3",
            "raid raid2 raid1 raid2 2",
            "true false",
        ]
    );
}

/// `unit.healthPercent` uses the client's own percentage where it has one
/// (scaled to 100), which may be secret.
#[test]
fn unit_health_percent_from_the_client() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as unit from \"wowlua:unit\"\n_G.StdUnit = unit\n",
        )],
        r#"CurveConstants = { ScaleTo100 = "scale" }
UnitHealthPercent = function(u, predicted, curve) print("native", u, curve) return Mock.secret() end"#,
        "Mock.loadAddon(\"Std\")\nprint(issecretvalue(StdUnit.healthPercent(\"player\")))\n",
    );
    assert_eq!(plain(&out), ["native player scale", "true"]);
}

// ------------------------------------------------------------------- aura --

const AURA_SETUP: &str = r#"Mock.auras.player = {
    { name = "Arcane Intellect", spellId = 1459, auraInstanceID = 1, isHelpful = true, isFromPlayerOrPlayerPet = true, applications = 0 },
    { name = "Thorns", spellId = 467, auraInstanceID = 2, isHelpful = true, isFromPlayerOrPlayerPet = false, applications = 0 },
    { name = Mock.secret(), spellId = Mock.secret(), auraInstanceID = 3, isHelpful = true, isFromPlayerOrPlayerPet = false, applications = 0 },
    { name = "Curse of Agony", spellId = 980, auraInstanceID = 4, isHarmful = true, dispelName = "Curse", applications = 3 },
    { name = "Frostbolt", spellId = 116, auraInstanceID = 5, isHarmful = true, dispelName = "Magic", applications = 0 },
}"#;

/// Literal aura filters are checked per targeted flavor (E8310), and
/// containers need a flavor with aura containers (E8300).
#[test]
fn aura_filters_and_containers_are_checked_per_flavor() {
    let check_in = |flavors: &str, body: &str| {
        let dir = project(&[(
            "src/main.wowlua",
            &format!("import * as aura from \"wowlua:aura\"\n{body}"),
        )]);
        fs::write(
            dir.path().join("wowlua.json"),
            format!(r#"{{ "package": {{ "name": "std", "version": "1.0.0" }}, "flavors": {{ {flavors} }}, "build": {{ "flavor-mode": "split" }} }}"#),
        )
        .unwrap();
        let assert = wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .assert();
        String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
    };
    let both = r#""camelot": {}, "vanilla": {}"#;
    let out = check_in(both, "print(aura.find(\"player\", 1, \"HELPFUL|BOGUS\"))\n");
    assert!(out.contains("E8310") && out.contains("BOGUS"), "{out}");
    let out = check_in(both, "print(aura.each(\"player\", \"HARMFUL|!PLAYER\"))\n");
    assert!(out.contains("E8310") && out.contains("vanilla"), "{out}");
    let out = check_in(
        r#""camelot": {}"#,
        "print(aura.each(\"player\", \"HARMFUL|!PLAYER|DISPELLABLE\"))\n",
    );
    assert!(!out.contains("E8310"), "{out}");
    let out = check_in(
        both,
        "local c = aura.container(UIParent, { unit = \"player\", groups = { { key = \"b\", filter = \"HELPFUL\" } } })\nprint(c)\n",
    );
    assert!(out.contains("E8300") && out.contains("vanilla"), "{out}");
    let out = check_in(
        both,
        "import flavor from \"wowlua:meta\"\n@if(flavor == \"camelot\")\nlocal function show()\n    aura.container(UIParent, { unit = \"player\", groups = { { key = \"b\", filter = \"HELPFUL|IMPORTANT\" } } })\nend\n",
    );
    assert!(!out.contains("E8300") && !out.contains("E8310"), "{out}");
}

/// `wowlua:aura`: guarded reads (nothing found when the client refuses),
/// secret IDs skipped, change notifications.
#[test]
fn aura_reads_are_guarded() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as aura from \"wowlua:aura\"\n_G.StdAura = aura\n",
        )],
        AURA_SETUP,
        r#"Mock.loadAddon("Std")
local aura = StdAura
local names = {}
for a in aura.each("player", "HELPFUL") do names[#names + 1] = issecretvalue(a.name) and "<secret>" or a.name end
print(table.concat(names, ", "))
local mine = 0
aura.forEach("player", "HELPFUL|PLAYER", function(a) mine = mine + 1 end)
print("mine", mine)
print(aura.find("player", 467, "HELPFUL").name, aura.find("player", 999, "HELPFUL"), aura.findByName("player", "Curse of Agony", "HARMFUL").applications)
print(aura.byInstanceId("player", 5).name, aura.readable("player"))
local stops = 0
aura.forEach("player", "HARMFUL", function(a) stops = stops + 1 return true end)
print("stopped after", stops)
-- In combat on restricted clients every read refuses: nothing, no error.
Mock.aurasRestricted = true
print(aura.readable("player"), aura.each("player", "HELPFUL")(), aura.find("player", 467, "HELPFUL"), aura.byInstanceId("player", 1))
Mock.aurasRestricted = false
aura.onChange("player", function(u, changes) print("changed", u, changes and changes.isFullUpdate) end)
Mock.fire("UNIT_AURA", "player", { isFullUpdate = true })
Mock.fire("UNIT_AURA", "target", { isFullUpdate = true })
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "Arcane Intellect, Thorns, <secret>",
            "mine 1",
            "Thorns nil 3",
            "Frostbolt true",
            "stopped after 1",
            "false nil nil nil",
            "changed player true",
        ]
    );
}

/// `aura.container`: the engine's container with filter groups, spell and
/// dispel-type lists, the client look, working while reads refuse.
#[test]
fn aura_containers_are_filled_by_the_engine() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as aura from "wowlua:aura"
import type AuraButton from "wowlua:aura"
local c = aura.container(UIParent, {
    unit = "player",
    groups = {
        { key = "buffs", filter = "PLAYER|HELPFUL" },
        { key = "tracked", filter = "HELPFUL", spells = { 467 } },
        { key = "curses", filter = "HARMFUL", dispelTypes = { "Curse" } },
        { key = "custom", filter = "HARMFUL", max = 1, initialize = function(_: AuraButton) print("custom button") end },
    },
})
_G.StdContainer = c
"#,
        )],
        AURA_SETUP,
        r#"Mock.loadAddon("Std")
local c = StdContainer
local engine = c.frame
for _, g in ipairs(engine.groups) do print(g.key, g.filter) end
local function show()
    for _, key in ipairs({ "buffs", "tracked", "curses", "custom" }) do
        print(key, table.concat(engine.showing[key], " "))
    end
end
show()
local curse = engine.groups[3].buttons[1]
print("look", curse.icon ~= nil, curse.count ~= nil, curse.durationText ~= nil, #curse.dispel, engine.groups[1].buttons[1].cancel)
-- The engine keeps working when addon reads refuse.
Mock.aurasRestricted = true
c:setGroupSpells("tracked", { 1459, 467 })
c:refresh()
print("tracked", table.concat(engine.showing.tracked, " "))
c:setGroupMax("buffs", 0)
c:refresh()
print("buffs", #engine.showing.buffs)
c:release()
print("released", engine.unit, engine:IsShown())
c:setUnit("target")
print("again", engine.unit, engine:IsShown())
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "custom button",
            "buffs HELPFUL|PLAYER",
            "tracked HELPFUL",
            "curses HARMFUL",
            "custom HARMFUL",
            "buffs 1459",
            "tracked 467",
            "curses 980",
            "custom 980",
            "look true true true 1 RightButtonUp",
            "tracked 1459 467",
            "buffs 0",
            "released none false",
            "again target true",
        ]
    );
}

/// `aura.container`'s group options and setters: `exclude`, per-group
/// size, spacing, order and gap, lazy groups (added once they have room,
/// keeping their place and what was set meanwhile), a new filter, spells
/// that keep the dispel types, and sizes applied out of combat;
/// `hasContainers`.
#[test]
fn aura_container_groups_change_later() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as aura from "wowlua:aura"
_G.StdAura = aura
_G.StdContainer = aura.container(UIParent, {
    unit = "player",
    lazy = true,
    size = 20,
    spacing = 3,
    groups = {
        { key = "debuffs", filter = "HARMFUL", exclude = { 116 } },
        { key = "later", filter = "HELPFUL", max = 0, dispelTypes = { "Curse" }, spacing = 1, gap = 5 },
        { key = "first", filter = "HELPFUL|PLAYER", order = 0, size = 16 },
    },
})
"#,
        )],
        AURA_SETUP,
        r#"Mock.loadAddon("Std")
local c = StdContainer
local engine = c.frame
print("has", StdAura.hasContainers())
local function groups()
    local parts = {}
    for _, g in ipairs(engine.groups) do
        local l = g.options.layout
        parts[#parts + 1] = `{g.key}:{g.filter}:{l.layoutIndex}:{l.elementWidth}:{l.elementSpacing}:{tostring(l.groupSpacing)}`
    end
    print(table.concat(parts, " "))
end
groups()
c:refresh()
print("debuffs", table.concat(engine.showing.debuffs, " "))
-- What a lazy group gets before it has room waits for it.
c:setGroupFilter("later", "HARMFUL")
c:setGroupSpells("later", { 980, 116 })
c:setGroupMax("later", 2)
groups()
c:refresh()
print("later", table.concat(engine.showing.later, " "))
c:setGroupFilter("debuffs", "HELPFUL|PLAYER")
c:refresh()
print("debuffs now", engine.groups[1].filter, table.concat(engine.showing.debuffs, " "))
-- Sizes wait for the end of combat; new buttons take the latest.
local button = engine.groups[2].buttons[1]
print("size", button:GetSize())
Mock.fire("PLAYER_REGEN_DISABLED")
c:setGroupSize("first", 30)
c:setGroupSize("first", 32)
print("in combat", button:GetWidth(), engine.groups[2].options.layout.elementWidth)
Mock.fire("PLAYER_REGEN_ENABLED")
print("after combat", button:GetSize())
Mock.auras.player[#Mock.auras.player + 1] = { name = "Mark", spellId = 1126, auraInstanceID = 6, isHelpful = true, isFromPlayerOrPlayerPet = true, applications = 0 }
c:refresh()
print("new button", engine.groups[2].buttons[2]:GetSize())
Mock.noAuraContainers = true
print("has", StdAura.hasContainers())
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "has true",
            "debuffs:HARMFUL:1:20:3:nil first:HELPFUL|PLAYER:0:16:3:nil",
            "debuffs 980",
            "debuffs:HARMFUL:1:20:3:nil first:HELPFUL|PLAYER:0:16:3:nil later:HARMFUL:2:20:1:5",
            "later 980",
            "debuffs now HELPFUL|PLAYER 1459",
            "size 16 16",
            "in combat 16 32",
            "after combat 32 32",
            "new button 32 32",
            "has false",
        ]
    );
}
// ------------------------------------------------------------------ spell --

/// `wowlua:spell`: spell data, cooldowns with secret times (still telling
/// whether they run), the legacy cooldown API.
#[test]
fn spell_info_and_cooldowns() {
    let source = "import * as spell from \"wowlua:spell\"\n_G.StdSpell = spell\n";
    let script = r#"Mock.loadAddon("Std")
local spell = StdSpell
local i = spell.info(133)
print(i.name, i.iconID, spell.name("Fireball"), spell.icon(133), spell.link(133), spell.description(133))
print(spell.info(1), spell.onCooldown(133), spell.isKnown(133), spell.isKnown(1), spell.isKnown(133, true))
Mock.spells[133].cooldown = { startTime = Mock.secret(), duration = Mock.secret(), modRate = Mock.secret(), isEnabled = true, isActive = true }
local cd = spell.cooldown(133)
print(issecretvalue(cd.duration), cd.isActive, cd.isEnabled, spell.onCooldown(133), spell.cooldownDuration(133))
Mock.spells[133].charges = { currentCharges = 1, maxCharges = 2, cooldownStartTime = 0, cooldownDuration = 8, chargeModRate = 1, isActive = true }
print(spell.charges(133).currentCharges)
"#;
    let out = modes(&[("src/main.wowlua", source)], "", script);
    assert_eq!(
        plain(&out),
        [
            "Fireball 135812 Fireball 135812 |cff71d5ff|Hspell:133|h[Fireball]|h|r Hurls a fiery ball.",
            "nil false true false false",
            "true true true true nil",
            "1",
        ]
    );
    // Clients with the global API only.
    let out = modes(
        &[("src/main.wowlua", source)],
        "C_Spell.GetSpellCooldown = nil\nfunction GetSpellCooldown(id) if id == 133 then return 100, 8, 1 end return 0, 0, 1 end",
        "Mock.loadAddon(\"Std\")\nlocal cd = StdSpell.cooldown(133)\nprint(cd.startTime, cd.duration, cd.isEnabled, cd.isActive, cd.modRate, StdSpell.onCooldown(1))\n",
    );
    assert_eq!(plain(&out), ["100 8 true true 1 false"]);
}

// ------------------------------------------------------------------- item --

/// `wowlua:item`: cached and loading items, shared requests, failures,
/// timeouts, cancelling, several items at once, spell data.
#[test]
fn items_load_and_cache() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as item from \"wowlua:item\"\n_G.StdItem = item\n",
        )],
        r#"Mock.items = {
    [6948] = { cached = true, info = { name = "Hearthstone", link = "|Hitem:6948|h[Hearthstone]|h", quality = 1 } },
    [19019] = { delay = 0.3, info = { name = "Thunderfury", link = "|Hitem:19019|h[Thunderfury]|h", quality = 5, itemLevel = 80 } },
    [404] = { missing = true },
    [17182] = { delay = 30, info = { name = "Sulfuras", link = "|Hitem:17182|h[Sulfuras]|h", quality = 5 } },
    [12345] = { delay = 0.3, info = { name = "Cancelled", link = "|Hitem:12345|h[Cancelled]|h" } },
}"#,
        r#"Mock.loadAddon("Std")
local item = StdItem
print(item.get(6948).name, item.get("|Hitem:6948|h[Hearthstone]|h").quality, item.get("Hearthstone").name, item.get(19019))
item.load(6948, function(d) print("sync", d.name) end)
print("after load")
item.load(19019, function(d) print("first", d.name, d.itemLevel) end)
item.load(19019, function(d) print("second", d.name) end)
item.load(19019, function(d) error("boom") end)
item.load(404, function(d) print("missing", d) end)
item.load(17182, function(d) print("timed out", d) end, { timeout = 1 })
local h = item.load(12345, function(d) print("never") end)
h:cancel()
print("done?", h:isDone())
Mock.advance(0.5)
local requests = 0
for _, id in ipairs(Mock.itemRequests) do if id == 19019 then requests = requests + 1 end end
print("requests for 19019", requests)
Mock.advance(1)
item.loadMany({ 6948, 19019, 404 }, function(all) print("many", all[6948].name, all[19019].name, all[404]) end)
Mock.advance(0.5)
item.loadMany({}, function(all) print("none", next(all)) end)
Mock.spells[133].cached = false
item.loadSpell(133, function(ok) print("spell", ok) end)
item.loadSpell(999, function(ok) print("no spell", ok) end)
Mock.advance(0.5)
item.loadSpell(133, function(ok) print("spell cached", ok) end)
Mock.advance(40)
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "Hearthstone 1 Hearthstone nil",
            "sync Hearthstone",
            "after load",
            "done? true",
            "missing nil",
            "first Thunderfury 80",
            "second Thunderfury",
            "requests for 19019 1",
            "timed out nil",
            "many Hearthstone Thunderfury nil",
            "none nil",
            "spell true",
            "no spell false",
            "spell cached true",
        ]
    );
    assert_eq!(
        errors(&out).iter().filter(|e| e.contains("boom")).count(),
        1,
        "{out:?}"
    );
}

// ------------------------------------------------------------------ media --

/// `wowlua:media`: an own registry without LibSharedMedia; with it (loaded
/// later by another addon) the waiting media is handed over.
#[test]
fn media_registry_and_shared_media() {
    let source = r#"import * as media from "wowlua:media"
media.register("statusbar", "Std Flat", "Interface\\AddOns\\Std\\flat")
_G.StdMedia = media
"#;
    let out = modes(
        &[("src/main.wowlua", source)],
        "STANDARD_TEXT_FONT = \"Fonts\\\\FRIZQT__.TTF\"",
        r#"Mock.loadAddon("Std")
local media = StdMedia
media.onRegistered(function(kind, name) print("registered", kind, name) end)
media.register("font", "Std Bold", "Interface\\AddOns\\Std\\bold.ttf")
media.register("font", "Std Alpha", 12345)
print(media.fetch("statusbar", "Std Flat"), media.fetch("font", "Nope"), media.fetch("font", "Nope", { noDefault = true }), media.fetch("sound", nil))
print(table.concat(media.list("font"), ", "), #media.list("border"))
-- Another addon loads LibSharedMedia.
local lsm = Mock.sharedMedia()
Mock.provideLibrary("LibSharedMedia-3.0", lsm)
Mock.fire("ADDON_LOADED", "Other", false)
print(table.concat(media.list("font"), ", "), media.fetch("statusbar", "Std Flat"), media.fetch("font", "Nope"))
lsm:Register("font", "Other Font", "x.ttf")
media.register("sound", "Std Ping", 567)
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "registered font Std Bold",
            "registered font Std Alpha",
            "Interface\\AddOns\\Std\\flat Fonts\\FRIZQT__.TTF nil nil",
            "Std Alpha, Std Bold 0",
            "Std Alpha, Std Bold Interface\\AddOns\\Std\\flat lsm-default-font",
            "registered font Other Font",
            "registered sound Std Ping",
        ]
    );
}

// ------------------------------------------------------------------- jobs --

/// `wowlua:jobs`: work spread over frames within the budget, priorities,
/// sleeping, waiting for events, progress, results, errors, cancelling and
/// combat pauses.
#[test]
fn jobs_spread_work_over_frames() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as jobs from \"wowlua:jobs\"\n_G.StdJobs = jobs\n",
        )],
        "",
        r#"Mock.loadAddon("Std")
local jobs = StdJobs
local frames, perFrame = 0, {}
-- 10 items of 1 ms each with a 4 ms budget: 3 frames.
local j = jobs.start(function(ctx)
    for i = 1, 10 do
        Mock.spend(1)
        perFrame[frames] = (perFrame[frames] or 0) + 1
        ctx:progress(i / 10)
        ctx:check()
    end
    return "indexed", 10
end, { name = "index", onDone = function(a, b) print("done", a, b) end, onProgress = function(p) end })
print(j:status())
for _ = 1, 5 do frames = frames + 1; Mock.advanceFrames(1) end
print(j:status(), j:progress(), j:result())
print("frames", perFrame[1], perFrame[2], perFrame[3])
-- Priorities: high runs about four times as often as low.
local runs = { high = 0, low = 0 }
local function busy(kind)
    return function(ctx)
        while true do runs[kind] = runs[kind] + 1; Mock.spend(1); ctx:check() end
    end
end
local high = jobs.start(busy("high"), { priority = "high" })
local low = jobs.start(busy("low"), { priority = "low" })
Mock.advanceFrames(20)
print("ratio", math.floor(runs.high / runs.low + 0.5))
high:cancel(); low:cancel()
print(high:status())
-- Sleeping and waiting for an event.
local s = jobs.start(function(ctx)
    ctx:sleep(1)
    print("woke")
    local a, b = ctx:waitFor("GET_ITEM_INFO_RECEIVED")
    print("event", a, b)
end)
Mock.advanceFrames(10)
print("still", s:status())
Mock.advanceFrames(60)
Mock.fire("GET_ITEM_INFO_RECEIVED", 3, true)
Mock.advanceFrames(2)
print(s:status())
-- Errors end the job, others go on.
local bad = jobs.start(function(ctx) error("broken") end, { onError = function(e) print("onError", e:find("broken") ~= nil) end })
Mock.advanceFrames(1)
print(bad:status())
-- Paused in combat.
local c = jobs.start(function(ctx) while true do ctx:yield() end end, { pauseInCombat = true })
Mock.advanceFrames(1)
Mock.fire("PLAYER_REGEN_DISABLED")
print(c:status())
Mock.fire("PLAYER_REGEN_ENABLED")
Mock.advanceFrames(1)
print(c:status())
c:cancel()
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "pending",
            "done indexed 10",
            "done 1 indexed 10",
            "frames 4 4 2",
            "ratio 4",
            "cancelled",
            "still running",
            "woke",
            "event 3 true",
            "done",
            "onError true",
            "failed",
            "paused",
            "running",
        ]
    );
    assert_eq!(
        errors(&out).iter().filter(|e| e.contains("broken")).count(),
        1,
        "{out:?}"
    );
}

/// `item.await` inside a job: the job waits for the item's data.
#[test]
fn items_await_in_jobs() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as jobs from "wowlua:jobs"
import * as item from "wowlua:item"
import type JobContext from "wowlua:jobs"
_G.StdRun = function()
    return jobs.start(function(ctx: JobContext)
        const cached = item.await(ctx, 6948)
        print("cached", cached?.name)
        const loaded = item.await(ctx, 19019)
        print("loaded", loaded?.name)
        return item.await(ctx, 404)
    end, { onDone = function(missing: any) print("missing", missing) end })
end
"#,
        )],
        "Mock.items[404] = { missing = true }",
        "Mock.loadAddon(\"Std\")\nlocal j = StdRun()\nMock.advanceFrames(60)\nprint(j:status())\n",
    );
    assert_eq!(
        plain(&out),
        [
            "cached Hearthstone",
            "loaded Thunderfury",
            "missing nil",
            "done"
        ]
    );
}

// ----------------------------------------------------------------- dialog --

/// `wowlua:dialog`: confirm, prompt (validated), export (read-only),
/// import (errors keep it open), one at a time, Escape, themes.
#[test]
fn dialogs_confirm_prompt_export_import() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as dialog from \"wowlua:dialog\"\n_G.StdDialog = dialog\n",
        )],
        "ACCEPT, CANCEL, CLOSE, BACKDROP_DIALOG_32_32 = \"Accept\", \"Cancel\", \"Close\", {}",
        r#"Mock.loadAddon("Std")
local dialog = StdDialog
dialog.confirm({ text = "Reset?", onAccept = function() print("reset") end, onCancel = function() print("kept") end })
local d = Mock.dialog()
print(d.text:GetText(), d.frame:GetName(), UISpecialFrames[1])
Mock.dialogAccept()
print("open after accept", Mock.dialog())
dialog.confirm({ text = "Again?", onAccept = function() print("never") end, onCancel = function() print("cancelled") end })
Mock.escape()
dialog.prompt({ text = "Name?", default = "x", validate = function(v) return #v >= 3 end, onAccept = function(v) print("named", v) end })
d = Mock.dialog()
print(d.edit:GetText(), d.accept:IsEnabled())
Mock.dialogAccept("ab")
print("still open", Mock.dialog() ~= nil)
Mock.dialogAccept("abc")
-- Opening another cancels the current one.
dialog.prompt({ text = "First", onAccept = function() end, onCancel = function() print("first cancelled") end })
dialog.export({ title = "Export", value = "!WL1!data" })
d = Mock.dialog()
print(d.text:GetText(), d.edit:GetText(), d.accept:IsShown())
d.edit:SetText("typed")
Mock.script(d.edit, "OnTextChanged", true)
print("still", d.edit:GetText())
Mock.dialogCancel()
dialog.import({ title = "Import", onImport = function(text) if text ~= "good" then return "Invalid string" end print("imported", text) end })
Mock.dialogAccept("bad")
d = Mock.dialog()
print("error", d.error:GetText())
Mock.dialogAccept("good")
print("closed", Mock.dialog())
dialog.setTheme({ button = "flat", colors = { background = { 0, 0, 0, 1 } } })
dialog.confirm({ text = "Themed", onAccept = function() end })
dialog.close()
print("closed", Mock.dialog())
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "Reset? StdDialog1 StdDialog1",
            "reset",
            "open after accept nil",
            "cancelled",
            "x false",
            "still open true",
            "named abc",
            "first cancelled",
            "Export !WL1!data false",
            "still !WL1!data",
            "error Invalid string",
            "imported good",
            "closed nil",
            "closed nil",
        ]
    );
}

// ------------------------------------------------------------------- menu --

/// `wowlua:menu`: menus built through MenuUtil each time they open, with
/// checkboxes, radios, submenus and disabled entries.
#[test]
fn menus_build_through_menu_util() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as menu from "wowlua:menu"
import type MenuBuilder from "wowlua:menu"
local pinned = false
local folder = "bags"
const row = CreateFrame("Button", "StdRow", UIParent)
menu.attach(row, function(m: MenuBuilder)
    m:title("Thunderfury")
        :button("Open", function() print("opened") end)
        :checkbox("Pinned", function() return pinned end, function() pinned = not pinned end)
        :divider()
        :submenu("Move to", function(sub: MenuBuilder)
            for _, f in ipairs({ "bags", "bank" }) do
                sub:radio(f, function() return folder == f end, function() folder = f end)
            end
        end)
        :disabled("Sell")
        :button("Boom", function() error("boom") end)
end)
"#,
        )],
        "",
        r#"Mock.loadAddon("Std")
Mock.script(StdRow, "OnMouseUp", "LeftButton")
print("left click", Mock.menu)
Mock.script(StdRow, "OnMouseUp", "RightButton")
print(Mock.lastMenu())
Mock.clickMenu("Open")
Mock.clickMenu("Pinned")
Mock.clickMenu("Move to/bank")
Mock.clickMenu("Sell")
Mock.clickMenu("Boom")
Mock.script(StdRow, "OnMouseUp", "RightButton")
print(Mock.lastMenu())
"#,
    );
    let menu = |pinned: &str, bags: &str, bank: &str| {
        format!(
            "Thunderfury\nOpen\n{pinned} Pinned\n---\nMove to\n  {bags} bags\n  {bank} bank\nSell (disabled)\nBoom"
        )
    };
    let expected = [
        "left click nil".to_string(),
        menu("[ ]", "(o)", "( )"),
        "opened".to_string(),
        menu("[x]", "( )", "(o)"),
    ];
    assert_eq!(plain(&out), expected);
    assert_eq!(
        errors(&out).iter().filter(|e| e.contains("boom")).count(),
        1,
        "{out:?}"
    );
}

// ------------------------------------------------------------------- list --

/// `wowlua:list`: only visible rows exist, rows are instances of the widget
/// class recycled through a pool, selection, refresh and scrolling.
#[test]
fn lists_recycle_rows() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as list from "wowlua:list"
type Entry = { name: string }
local made, resets = 0, 0
class RowButton extends Button
    readonly label: FontString
    constructor(parent: Frame)
        super(nil, parent)
        made += 1
        self.label = self:CreateFontString(nil, "ARTWORK", "GameFontHighlight")
    end
end
const entries: Entry[] = {}
for i = 1, 20 do entries[i] = { name = `entry {i}` } end
local shown: string[] = {}
local view = list.create(UIParent, {
    rowHeight = 20,
    row = RowButton,
    init = function(row: RowButton, entry: Entry, index: number)
        row.label:SetText(entry.name)
        shown[#shown + 1] = `{index}`
    end,
    reset = function(_: RowButton) resets += 1 end,
})
view:onSelect(function(entry: Entry, index: number) print("selected", entry.name, index) end)
_G.StdList = { view = view, stats = function() return made, resets, table.concat(shown, " ") end, clear = function() shown = {} end }
"#,
        )],
        "",
        r#"Mock.loadAddon("Std")
local l = StdList
l.view:setData((function() local t = {} for i = 1, 20 do t[i] = { name = "entry " .. i } end return t end)())
print(l.stats())
l.clear()
l.view:scrollTo(12)
print(l.stats())
-- Clicking a row selects its entry.
for _, f in ipairs(Mock.frames()) do
    if f.label and f.label:GetText() == "entry 9" then Mock.script(f, "OnMouseUp", "LeftButton") end
end
print(l.view:selected().name, #l.view:data())
l.clear()
Mock.scroll(l.view, 15)
print(l.stats())
l.clear()
l.view:refresh()
print(l.stats())
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "5 0 1 2 3 4 5",
            "5 5 8 9 10 11 12",
            "selected entry 9 9",
            "entry 9 20",
            "5 10 16 17 18 19 20",
            "5 10 16 17 18 19 20",
        ]
    );
}

// -------------------------------------------------------------- core-demo --

/// M8's exit criterion: `examples/core-demo` has events, saved variables,
/// a settings page, a minimap button, keybindings and addon messages, all
/// from the standard library.
#[test]
fn core_demo_needs_no_third_party_libraries() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/core-demo");
    let dir = tempfile::tempdir().unwrap();
    copy_tree(&source, dir.path());
    let stderr = build(dir.path(), "dev");
    assert!(
        !stderr.contains("warning[") && !stderr.contains("error"),
        "{stderr}"
    );
    let out = dir.path().join("dist/dev/CoreDemo");
    // No embedded libraries.
    assert!(!out.join("libs").exists());
    let printed = load(
        &out,
        "CoreDemo",
        "CoreDemo.toc",
        "",
        "Mock.guild = true\nMock.echoAs = \"Friend\"",
        r#"Mock.loadAddon("CoreDemo")
Mock.fire("PLAYER_LOGIN")
Mock.advance(4)
local C = Mock.check
C(#Mock.settings.categories == 1, "a settings page")
C(CoreDemoMinimapButton ~= nil, "a minimap button")
C(AddonCompartmentFrame.registeredAddons[1].text == "Core Demo", "an Addon Compartment entry")
Mock.pressBinding("COREDEMO_TOGGLE_BOX")
Mock.slash("/coredemo ping")
Mock.advance(2)
Mock.slash("/coredemo status")
"#,
    );
    let text = plain(&printed).join("\n");
    assert!(!text.contains("FAIL"), "{text}");
    for expected in [
        "login #1",
        "box toggled",
        "ping sent",
        "Friend answered: pong 1 (WHISPER)",
        "logins 1",
    ] {
        assert!(text.contains(expected), "{expected:?} missing:\n{text}");
    }
}

// ---------------------------------------------------------- table, format --

/// `wowlua:table`: copies (cycles kept), merging, defaults, equality, keys,
/// list helpers.
#[test]
fn table_helpers() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as tbl from \"wowlua:table\"\n_G.StdTable = tbl\n",
        )],
        "",
        r#"Mock.loadAddon("Std")
local tbl = StdTable
local t = { a = 1, nested = { b = 2 }, list = { 1, 2, 3 } }
t.self = t
local c = tbl.copy(t)
print(c ~= t, c.nested ~= t.nested, c.nested.b, c.self == c, tbl.equal(t.nested, c.nested))
local target = { a = 1, keep = { x = 1 } }
tbl.merge(target, { a = 2, keep = { y = 2 }, new = { z = 3 } })
print(target.a, target.keep.x, target.keep.y, target.new.z)
local settings = { scale = 2, colors = { r = 1 } }
tbl.defaults(settings, { scale = 1, greet = true, colors = { r = 0, g = 0 } })
print(settings.scale, settings.greet, settings.colors.r, settings.colors.g)
print(tbl.equal({ 1, { 2 } }, { 1, { 2 } }), tbl.equal({ 1 }, { 1, 2 }), tbl.equal(Mock.secret(), 1))
print(table.concat(tbl.keys({ b = 1, a = 2, c = 3 }), ","), tbl.count({ a = 1, 2, 3 }), tbl.isEmpty({}))
local doubled = tbl.map({ 1, 2, 3 }, function(v) return v * 2 end)
local odd = tbl.filter({ 1, 2, 3, 4 }, function(v) return v % 2 == 1 end)
local found, at = tbl.find({ "a", "b" }, function(v) return v == "b" end)
print(table.concat(doubled, ","), table.concat(odd, ","), found, at, tbl.contains({ "x" }, "x"))
-- Keys are kept as they are (a frame or a table as a key stays itself).
local frame = CreateFrame("Frame")
local key = {}
local byKey = tbl.copy({ [frame] = 1, [key] = { 2 } })
print(byKey[frame], byKey[key][1], byKey[key] ~= nil)
-- `shared = false`: no two places share a table; a cycle is an error.
local shared = { 1 }
local both = { a = shared, b = shared }
local kept, apart = tbl.copy(both), tbl.copy(both, { shared = false })
print(kept.a == kept.b, apart.a == apart.b, apart.a[1], apart.a ~= shared)
print(pcall(tbl.copy, t, { shared = false }))
print(table.concat(tbl.keys({ [2] = 1, b = 2, [1] = 3 }, function(x, y) return tostring(x) < tostring(y) end), ","))
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "true true 2 true true",
            "2 1 2 3",
            "2 true 1 0",
            "true false false",
            "a,b,c 3 true",
            "2,4,6 1,3 b 2 true",
            "1 2 true",
            "true false 1 true",
            "false copy: the table contains itself, which `shared = false` cannot copy",
            "1,2,b",
        ]
    );
}

/// `wowlua:format`: the client's localized formats (the mock's are
/// English), secret numbers stay secret.
#[test]
fn format_uses_the_client_formats() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as format from \"wowlua:format\"\n_G.StdFormat = format\n",
        )],
        "",
        r#"Mock.loadAddon("Std")
local format = StdFormat
print(format.duration(3725), format.duration(3725, { long = true, units = 3 }), format.duration(125, { noSeconds = true }))
print(format.short(300), format.clock(187), format.clock(3787), format.clock(65, true))
print(format.money(123456), format.money(123456, { text = true }), format.money(0, { text = true }))
print(format.number(1234567), format.abbreviate(8500), format.percent(0.456), format.percent(0.456, 1))
local s = format.abbreviate(Mock.secret())
print(type(format.date()))
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "1 Hr 2 Min 1 Hours 2 Minutes 5 Seconds 2 Min",
            "5 m 3:07 1:03:07 0:01:05",
            "12g 34s 56c 12 Gold 34 Silver 56 Copper 0 Copper",
            "1,234,567 8.5K 46% 45.6%",
            "string",
        ]
    );
}

// ---------------------------------------------------------------- version --

/// `wowlua:version`: semantic version order, the shared registry of
/// installed addons, and newer versions learned over addon messages
/// (callback only).
#[test]
fn versions_registry_and_newer_versions() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as version from \"wowlua:version\"\nimport * as comm from \"wowlua:comm\"\n_G.StdVersion = version\n_G.StdComm = comm\n",
        )],
        "WowluaVersions = { Other = \"2.1.0\" }\nMock.guild = true\nMock.echoAs = \"Friend\"",
        r#"Mock.loadAddon("Std")
local version = StdVersion
local c = version.compare
print(c("1.0.0", "1.0.0"), c("1.0.1", "1.0.0"), c("1.0.0", "1.0.0-beta.2"), c("1.0.0-beta.2", "1.0.0-beta.10"), c("1.0.0-alpha", "1.0.0-beta"), c("1.0.0+abc", "1.0.0+def"), c("2.0", "1.9.9"))
local mine = version.current()
print(version.of("Std") == mine, version.of("Other"), version.of("Nope"), version.installed().Other)
version.onNewer(function(newest, sender) print("newer", newest, sender) end)
Mock.fire("PLAYER_LOGIN")
Mock.advance(11)
print("announced", Mock.sent[1] and Mock.sent[1].chatType)
-- Another player with a newer version, then an older one.
local channel = StdComm.open("WowluaVersion")
channel:send("1:Std:9.0.0", "GUILD")
Mock.advance(1)
channel:send("1:Other:99.0.0", "GUILD")
Mock.advance(1)
local before = #Mock.sent
channel:send("1:Std:0.0.1", "GUILD")
Mock.advance(1)
local reply = Mock.sent[before + 2]
print("told", reply and reply.chatType, reply and reply.target)
-- The echoed whisper carried our own version back.
print(version.newest(), version.peers().Friend == mine)
version.onNewer(function(newest) print("late", newest) end)
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "0 1 1 -1 -1 0 1",
            "true 2.1.0 nil 2.1.0",
            "announced GUILD",
            "newer 9.0.0 Friend",
            "told WHISPER Friend",
            "9.0.0 true",
            "late 9.0.0",
        ]
    );
}

// ----------------------------------------------------------- alert, links --

/// `wowlua:alert`: sound kits, files and media names, raid warnings,
/// flashing, throttled warnings.
#[test]
fn alerts_sounds_and_warnings() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as alert from \"wowlua:alert\"\nimport * as media from \"wowlua:media\"\nimport * as color from \"wowlua:color\"\nmedia.register(\"sound\", \"Std Ping\", \"Interface\\\\AddOns\\\\Std\\\\ping.ogg\")\n_G.StdAlert = { alert = alert, color = color }\n",
        )],
        "",
        r#"Mock.loadAddon("Std")
local alert, color = StdAlert.alert, StdAlert.color
alert.sound(8959)
alert.sound("Std Ping", "SFX")
alert.sound(567890123)
alert.raidWarning("Pull in 5")
alert.warn("Boss!", { sound = 8959, flash = true, color = color.rgb(0, 1, 0) })
alert.warn("Boss!", { sound = 8959 })
Mock.advance(2)
alert.warn("Boss!")
print(table.concat(Mock.sounds, " | "))
print(table.concat(Mock.warnings, " | "))
print("flashes", Mock.flashes)
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "kit 8959 Master | file Interface\\AddOns\\Std\\ping.ogg SFX | file 567890123 Master | kit 8959 Master",
            "Pull in 5 (1.00 0.28 0.00) | Boss! (0.00 1.00 0.00) | Boss! (1.00 0.28 0.00)",
            "flashes 1",
        ]
    );
}

/// `wowlua:links`: addon links in chat, clicks reach this addon only.
#[test]
fn links_in_chat() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as links from \"wowlua:links\"\n_G.StdLinks = links\n",
        )],
        "",
        r#"Mock.loadAddon("Std")
local links = StdLinks
local text = links.link("Open the log", "log:42")
print(text)
local sub = links.onClick(function(data, button) print("clicked", data, button) end)
links.onClick(function(data) print("second", data) end)
SetItemRef("addon:Std:log:42", "[Open the log]", "LeftButton")
SetItemRef("addon:Other:x", "[x]", "LeftButton")
sub:cancel()
SetItemRef("addon:Std:again", "[again]", "RightButton")
print(pcall(links.link, "x", "a|b"))
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "|cff71d5ff|Haddon:Std:log:42|h[Open the log]|h|r",
            "clicked log:42 LeftButton",
            "second log:42",
            "second again",
            "false links.link: the data may not contain `|`",
        ]
    );
}

// ----------------------------------------------------- roster, bucket, LDB --

/// `unit.roster` maps GUIDs to group members, coalesces roster events and
/// reports who joined and left; secret GUIDs keep the last known member.
#[test]
fn unit_roster_tracks_joins_and_leaves() {
    let out = modes(
        &[(
            "src/main.wowlua",
            "import * as unit from \"wowlua:unit\"\n_G.StdUnit = unit\n",
        )],
        "",
        r#"Mock.loadAddon("Std")
local roster = StdUnit.roster
local function names(list)
    local out = {}
    for _, m in ipairs(list) do out[#out + 1] = m.name .. "@" .. m.unit end
    table.sort(out)
    return table.concat(out, ",")
end
print("solo", names(roster.members()), roster.byGuid("Player-1-00000001").fullName)
roster.onChange(function(joined, left) print("change", names(joined), names(left)) end)
local ally = { name = "Ally", realm = "Other", guid = "Player-2-1", class = "Warrior", classFile = "WARRIOR", level = 60, health = 1, healthMax = 1, power = 0, powerMax = 1, isPlayer = true, connected = true }
local friend = { name = "Friend", guid = "Player-3-1", class = "Priest", classFile = "PRIEST", level = 60, health = 1, healthMax = 1, power = 0, powerMax = 1, isPlayer = true, connected = true }
Mock.group = "party"
Mock.units.party1 = ally
Mock.fire("GROUP_ROSTER_UPDATE")
Mock.units.party2 = friend
Mock.fire("GROUP_ROSTER_UPDATE")
Mock.fire("GROUP_ROSTER_UPDATE")
print("coalesced")
Mock.advance(0.3)
print(roster.byUnit("party2").fullName, roster.byGuid("Player-2-1").unit)
-- Into a raid: the same people, other tokens; no change reported.
Mock.group = "raid"
Mock.units.party1, Mock.units.party2 = nil, nil
Mock.units.raid1, Mock.units.raid2, Mock.units.raid3 = Mock.units.player, friend, ally
Mock.fire("GROUP_ROSTER_UPDATE")
Mock.advance(0.3)
print(names(roster.members()), roster.byGuid("Player-2-1").unit)
-- A secret GUID keeps the member; leaving is reported.
Mock.units.raid3 = { name = Mock.secret(), guid = Mock.secret(), class = "Warrior", classFile = "WARRIOR", level = 60, health = 1, healthMax = 1, power = 0, powerMax = 1, isPlayer = true, connected = true }
Mock.units.raid2 = nil
Mock.fire("GROUP_ROSTER_UPDATE")
Mock.advance(0.3)
print(roster.byGuid("Player-2-1") ~= nil, roster.byGuid("Player-3-1"))
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "solo Tester@player Tester-MockRealm",
            "coalesced",
            "change Ally@party1,Friend@party2 ",
            "Friend-MockRealm party1",
            "Ally@raid3,Friend@raid2,Tester@player raid3",
            "change  Friend@raid2",
            "true nil",
        ]
    );
}

/// `events.onBucket` and `@bucket` coalesce bursts and count them by first
/// argument and by event.
#[test]
fn events_buckets_coalesce_bursts() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as events from "wowlua:events"
import bucket from "wowlua:events"
import type BucketCounts from "wowlua:events"
_G.StdEvents = events

@bucket(0.5, "BAG_UPDATE")
function bags(counts: BucketCounts)
    local ids: string[] = {}
    for id, n in pairs(counts.args) do ids[#ids + 1] = `{id}x{n}` end
    table.sort(ids)
    print("bags", table.concat(ids, " "))
end
"#,
        )],
        "",
        r#"Mock.loadAddon("Std")
local events = StdEvents
local sub = events.onBucket({ "UNIT_HEALTH", "UNIT_POWER_UPDATE" }, 1, function(c)
    print("units", c.args.player, c.args.target, c.events.UNIT_HEALTH, c.events.UNIT_POWER_UPDATE)
end)
Mock.fire("BAG_UPDATE", 0)
Mock.fire("BAG_UPDATE", 0)
Mock.fire("BAG_UPDATE", 2)
Mock.fire("UNIT_HEALTH", "player")
Mock.fire("UNIT_HEALTH", "target")
Mock.fire("UNIT_POWER_UPDATE", "player", "MANA")
Mock.fire("UNIT_HEALTH", Mock.secret())
Mock.advance(0.6)
print("half")
Mock.advance(0.5)
Mock.fire("BAG_UPDATE", 1)
sub:cancel()
Mock.fire("UNIT_HEALTH", "player")
Mock.advance(2)
"#,
    );
    assert_eq!(
        plain(&out),
        ["bags 0x2 2x1", "half", "units 2 1 3 1", "bags 1x1"]
    );
}

/// Bucket event names are checked like `on`'s (E8003).
#[test]
fn bucket_event_names_are_checked() {
    for src in [
        "import onBucket from \"wowlua:events\"\nonBucket({ \"BAG_UPDATE\", \"NOT_AN_EVENT\" }, 1, function() end)\n",
        "import onBucket from \"wowlua:events\"\nonBucket(\"NOT_AN_EVENT\", 1, function() end)\n",
        "import bucket from \"wowlua:events\"\n@bucket(1, \"BAG_UPDATE\", \"NOT_AN_EVENT\")\nlocal function f() end\n",
    ] {
        let out = check(src);
        assert!(
            out.contains("E8003") && out.contains("NOT_AN_EVENT"),
            "{src}: {out}"
        );
    }
    let out = check(
        "import bucket, onBucket from \"wowlua:events\"\nonBucket({ \"BAG_UPDATE\" }, 1, function() end)\n@bucket(1, \"BAG_UPDATE\")\nlocal function f() end\n",
    );
    assert!(!out.contains("E8003"), "{out}");
}

/// `launcher.dataSource` publishes an LDB data source, waiting for
/// LibDataBroker when another addon embeds it later.
#[test]
fn launcher_data_sources() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as launcher from "wowlua:launcher"
_G.StdLauncher = launcher
_G.StdGold = launcher.dataSource("Std Gold", {
    text = "12g",
    icon = "Interface\\Icons\\INV_Misc_Coin_01",
    onClick = function(button: string) print("clicked", button) end,
    tooltip = function(tip: GameTooltip) tip:AddLine("Gold") end,
})
"#,
        )],
        "_G.LibStub = nil",
        r#"Mock.loadAddon("Std")
StdGold:setText("13g")
local objects = {}
Mock.provideLibrary("LibDataBroker-1.1", {
    NewDataObject = function(_, name, obj)
        if objects[name] then return nil end
        objects[name] = obj
        return obj
    end,
})
Mock.fire("ADDON_LOADED", "Titan", false)
local obj = objects["Std Gold"]
print(obj.type, obj.text, obj.label, obj.icon)
StdGold:setText("14g")
StdGold:setLabel("Gold")
print(obj.text, obj.label)
obj.OnClick(nil, "RightButton")
local now = StdLauncher.dataSource("Std Bags", { text = "5/20" })
print(objects["Std Bags"].text, now.object == objects["Std Bags"])
StdLauncher.dataSource("Std Bags", { text = "again" })
print(#Mock.errors)
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "data source 13g Std Gold Interface\\Icons\\INV_Misc_Coin_01",
            "14g Gold",
            "clicked RightButton",
            "5/20 true",
            "1",
        ]
    );
}

// ----------------------------------------------------------------- secure --

/// `secure.visibility` and `secure.state` drive frames through the client's
/// state drivers; registering in combat waits for its end.
#[test]
fn secure_state_drivers() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as secure from "wowlua:secure"
_G.StdSecure = secure
_G.StdBox = CreateFrame("Frame")
"#,
        )],
        "",
        r#"Mock.loadAddon("Std")
local secure, box = StdSecure, StdBox
secure.visibility(box, "[combat] show; hide")
print("out of combat", box:IsShown())
Mock.fire("PLAYER_REGEN_DISABLED")
print("in combat", box:IsShown())
local sub = secure.state(box, "Mode", "[mod:shift] fast; [group] group; solo", function(v) print("state", v) end)
print("registered in combat:", #Mock.stateDrivers)
Mock.fire("PLAYER_REGEN_ENABLED")
print("after combat:", #Mock.stateDrivers, box:IsShown())
Mock.group = "party"
Mock.fire("GROUP_ROSTER_UPDATE")
Mock.modifiers.SHIFT = true
Mock.updateStateDrivers()
Mock.modifiers.SHIFT = nil
sub:cancel()
Mock.group = nil
Mock.fire("GROUP_ROSTER_UPDATE")
secure.visibility(box, nil)
Mock.fire("PLAYER_REGEN_DISABLED")
print("no driver", box:IsShown(), #Mock.stateDrivers)
-- Without `fn`, only the driver (for an `_onstate-<name>` snippet).
Mock.fire("PLAYER_REGEN_ENABLED")
local page = secure.state(box, "page", "[mod:shift] 2; 1")
print("driver only", #Mock.stateDrivers, box:GetAttribute("state-page"))
page:cancel()
print("removed", #Mock.stateDrivers)
print(secure.evaluate("[combat] a; b"), secure.evaluate("[nocombat] a"))
print(#Mock.errors)
"#,
    );
    assert_eq!(
        plain(&out),
        [
            "out of combat false",
            "in combat true",
            "registered in combat: 1",
            "state solo",
            "after combat: 2 false",
            "state group",
            "state fast",
            "no driver false 0",
            "driver only 1 1",
            "removed 0",
            "b a",
            "0",
        ]
    );
}

/// Literal macro conditionals are checked: brackets and conditions (E8440),
/// and a visibility driver's values (E8441).
#[test]
fn secure_conditionals_are_checked() {
    let cases = [
        ("secure.visibility(f, \"[combat show; hide\")", "E8440"),
        ("secure.visibility(f, \"[comabt] show; hide\")", "E8440"),
        (
            "secure.state(f, \"x\", \"[@] a; b\", function(v: string) end)",
            "E8440",
        ),
        ("secure.visibility(f, \"[combat] shown; hide\")", "E8441"),
        ("secure.evaluate(\"a ] b\")", "E8440"),
    ];
    for (call, code) in cases {
        let out = check(&format!(
            "import * as secure from \"wowlua:secure\"\nlocal f = CreateFrame(\"Frame\")\n{call}\n"
        ));
        assert!(out.contains(code), "{call}: {out}");
    }
    let out = check(
        "import * as secure from \"wowlua:secure\"\nlocal f = CreateFrame(\"Frame\")\n\
         secure.visibility(f, \"[petbattle][vehicleui] hide; [@target,exists,nodead,harm][combat,mod:shift/ctrl] show; hide;\")\n\
         secure.state(f, \"page\", \"[bonusbar:5] 11; [form:1/2,target=focus] 7; 1\", function(v: string) end)\n\
         secure.visibility(f, nil)\n",
    );
    assert!(!out.contains("E844"), "{out}");
}

// ------------------------------------------------------------------ color --

/// A literal `color.hex` is validated at compile time (E8390).
#[test]
fn color_hex_literals_are_checked() {
    for bad in ["\"ff88\"", "\"#gg8800\"", "\"ff88001122\""] {
        let out = check(&format!(
            "import * as color from \"wowlua:color\"\nlocal c = color.hex({bad})\nprint(c)\n"
        ));
        assert!(out.contains("E8390"), "{bad}: {out}");
    }
    let out = check("import * as color from \"wowlua:color\"\nprint(color.hex(\"#80ff8800\"))\n");
    assert!(!out.contains("E8390"), "{out}");
}

/// `wowlua:color`: hex colors (folded when constant), escapes, class and
/// quality colors, interpolation.
#[test]
fn colors_hex_escapes_class_and_quality() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r##"import * as color from "wowlua:color"

const ORANGE = "#ff8800"
local orange = color.hex(ORANGE)
local runtime = "80ff0000"
local red = color.hex(runtime)
print(color.toHex(orange), color.toHex(red, true))
print(color.wrap("Warning", orange))
print(color.colorize("00ff00", "ok"), color.strip("|cff00ff00ok|r and |cnRED_FONT_COLOR:red|r"))
print(color.toHex(color.lerp(color.hex("000000"), color.hex("ffffff"), 0.5)))
print(color.toHex(assert(color.class("MAGE"))), color.class("NOPE"), color.toHex(assert(color.quality(4))))
_G.CUSTOM_CLASS_COLORS = { MAGE = { r = 1, g = 0, b = 1 } }
print(color.toHex(assert(color.class("MAGE"))), color.toHex(color.rgb(0, 0, 1, 0.5), true))
print(pcall(color.hex, "nope"))
"##,
        )],
        "",
        "Mock.loadAddon(\"Std\")",
    );
    assert_eq!(
        plain(&out),
        [
            "ff8800 80ff0000",
            "|cffff8800Warning|r",
            "|cff00ff00ok|r ok and red",
            "808080",
            "40c7eb nil a336ed",
            "ff00ff 800000ff",
            "false color.hex: \"nope\" is not rrggbb or aarrggbb",
        ]
    );
}

// --------------------------------------------------------------- settings --

#[test]
fn settings_pages_bind_controls_to_the_store() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"
import binding from "wowlua:bindings"
local C = Mock.check

@binding("STD_TOGGLE", { label = "Toggle" })
local function toggle() end

interface Options
    showMinimap: boolean
    scale: number
    anchor: "TOPLEFT" | "CENTER"
    tint: { r: number, g: number, b: number, a: number? }
end

local db = saved.bind("StdDB", {
    version = 1,
    defaults = { showMinimap = true, scale = 1, anchor = "CENTER", tint = { r = 1, g = 0, b = 0 } } :: Options,
    profiles = true,
})

local built = 0
local page = settings.register({
    title = "Std",
    store = db,
    controls = {
        settings.header("General"),
        settings.text("Some text"),
        settings.checkbox("showMinimap", { label = "Minimap", tooltip = "Show it" }),
        settings.slider("scale", { label = "Scale", min = 0.5, max = 2, step = 0.1, format = function(v: number) return v * 100 .. "%" end }),
        settings.dropdown("anchor", { label = "Anchor", options = { { value = "TOPLEFT", label = "Top left" }, { value = "CENTER", label = "Center" } } }),
        settings.color("tint", { label = "Tint" }),
        settings.keybinding("STD_TOGGLE", { label = "Toggle" }),
        settings.button("Reset", function() print("reset clicked") end),
        settings.custom(function(parent: Frame) built += 1; return parent end, { height = 40 }),
    },
    subcategories = { { title = "More", controls = { settings.checkbox("showMinimap", { label = "Again" }) } } },
    onChange = function(key: string, value: any) print("changed", key, type(value) == "table" and "table" or tostring(value)) end,
})
C(#Mock.settings.categories == 0, "nothing before the store loads")
C(not pcall(settings.register, { title = "x", store = db, controls = {} }), "register once")
_G.StdSettings = { page = page, db = db, built = function() return built end, profiles = saved.handle(db).profiles }
"#,
        )],
        // The client's own binding first; the addon's comes from its
        // Bindings.xml.
        r#"Mock.bindings = { "OTHER" }"#,
        r#"Mock.loadAddon("Std")
local C = Mock.check
local cats = Mock.settings.categories
C(#cats == 2 and cats[1].registered and cats[2].parent == cats[1], "a category and its subcategory")
C(StdSettings.page.id == cats[1].id, "the page knows its id")
local items = cats[1].layout.items
local kinds = {}
for _, i in ipairs(items) do kinds[#kinds + 1] = i.kind end
print(table.concat(kinds, " "))
local vars = Mock.settings.variables
local minimap = vars.WOWLUA_STD_showMinimap
C(minimap.varType == "boolean" and minimap.default == true, "a boolean proxy with the store's default")
C(minimap:GetValue() == true, "reads the store")
minimap:SetValue(false)
C(StdSettings.db.showMinimap == false, "writes the store")
C(items[4].a.format(1.5) == "150%", "the slider formatter")
local anchors = {}
for _, o in ipairs(items[5].a()) do anchors[#anchors + 1] = o.value .. "=" .. o.label end
print(table.concat(anchors, ","))
local tint = vars.WOWLUA_STD_tint
print("tint", tint:GetValue())
tint:SetValue("ff00ff00")
print("tint now", StdSettings.db.tint.r, StdSettings.db.tint.g, StdSettings.db.tint.b)
C(items[7].index == 2, "the binding's index")
items[8].click()
-- Custom rows build their content once, when first shown.
local frame = CreateFrame("Frame")
items[9]:InitFrame(frame)
items[9]:InitFrame(frame)
C(StdSettings.built() == 1 and items[9]:GetExtent() == 40, "custom content built once")
items[2]:InitFrame(frame)
-- A profile switch re-reads the controls.
StdSettings.profiles:use("Other")
C((minimap.updates or 0) >= 1, "refreshed after a profile switch")
C(minimap:GetValue() == true, "the new profile's value")
-- The subcategory's control for the same field: its own variable.
local again = vars.WOWLUA_STD_showMinimap_2
C(again ~= nil and again:GetValue() == true, "a second control of a field")
-- Opening: now, or after combat.
StdSettings.page:open()
Mock.fire("PLAYER_REGEN_DISABLED")
StdSettings.page:open()
print("opened in combat", #Mock.settings.opened)
Mock.fire("PLAYER_REGEN_ENABLED")
print("opened", #Mock.settings.opened)
"#,
    );
    let plain: Vec<String> = out
        .iter()
        .map(|l| l.replace("|cff33ff99Std|r: ", ""))
        .collect();
    assert_eq!(
        plain,
        [
            "header element checkbox slider dropdown color keybinding button element",
            "changed showMinimap false",
            "TOPLEFT=Top left,CENTER=Center",
            "tint ffff0000",
            "changed tint table",
            "tint now 0 1 0",
            "reset clicked",
            "The options open once combat ends.",
            "opened in combat 1",
            "opened 2",
        ],
        "{out:?}"
    );
}

// --------------------------------------------------------------- launcher --

const LAUNCHER_MAIN: &str = r#"import * as launcher from "wowlua:launcher"
import * as saved from "wowlua:saved"

local db = saved.bind("StdDB", { version = 1, defaults = { minimap = { hide = false } } })
launcher.register({
    icon = "icon-1",
    label = "Std",
    onClick = function(button: string) print("click", button) end,
    tooltip = function(tip: GameTooltip) print("tooltip", tip == GameTooltip) end,
    minimap = { store = db, key = "minimap" },
    compartment = true,
    ldb = true,
})
local again = launcher.register
_G.StdLauncher = { launcher = launcher, db = db, again = again }
"#;

#[test]
fn launcher_builtin_button_and_compartment() {
    let out = modes(
        &[("src/main.wowlua", LAUNCHER_MAIN)],
        "",
        r#"local C = Mock.check
C(StdMinimapButton == nil, "nothing before login")
Mock.loadAddon("Std")
Mock.fire("PLAYER_LOGIN")
local b = StdMinimapButton
C(b ~= nil and b:GetParent() == nil or true, "the built-in button")
local function at() local _, _, _, x, y = b:GetPoint(); return math.floor(x + 0.5), math.floor(y + 0.5) end
print("default", at())
Mock.script(b, "OnClick", "RightButton")
Mock.script(b, "OnEnter")
-- Dragging: the cursor right of the minimap's center.
Mock.cursor = { 600, 500 }
Mock.script(b, "OnDragStart")
Mock.script(b, "OnUpdate")
Mock.script(b, "OnDragStop")
print("dragged", StdLauncher.db.minimap.minimapPos, at())
-- A square minimap: the button follows its edge.
function GetMinimapShape() return "SQUARE" end
StdLauncher.db.minimap.minimapPos = 45
StdLauncher.launcher.refresh()
print("square", at())
StdLauncher.launcher.showMinimap(false)
C(not b:IsShown() and StdLauncher.db.minimap.hide == true, "hidden and stored")
StdLauncher.launcher.setIcon("icon-2")
C(b.icon:GetTexture() == "icon-2", "the button's icon")
-- The compartment entry.
local entry = AddonCompartmentFrame.registeredAddons[1]
C(entry and entry.text == "Std" and entry.icon == "icon-2", "the compartment entry")
entry.func(nil, { buttonName = "MiddleButton" })
StdLauncher.launcher.setLabel("Std 2")
C(entry.text == "Std 2", "the entry's label")
C(not pcall(StdLauncher.again, { icon = "x", label = "y", onClick = function() end }), "register once")
-- Themes: tinted, flat, and back.
local function vertex(t: any): number[] return { t:GetVertexColor() } end
C(b.ring:IsShown() and b.disc:IsShown() and b.flatDisc == nil, "the client's look")
StdLauncher.launcher.setTheme({ colors = { border = { 1, 0, 0 }, background = { 0, 0, 1, 0.5 } } })
-- Colors are kept in eight bits (0.5 reads back 128/255, as in the client).
C(vertex(b.ring)[1] == 1 and vertex(b.ring)[2] == 0 and vertex(b.disc)[3] == 1 and math.abs(vertex(b.disc)[4] - 128 / 255) < 1e-6, "tinted ring and disc")
StdLauncher.launcher.setTheme({ button = "flat", colors = { border = { 0, 1, 0 } } })
C(not b.ring:IsShown() and not b.disc:IsShown() and b.flatDisc:IsShown() and vertex(b.flatBorder)[2] == 1, "flat: a disc and a border")
StdLauncher.launcher.setTheme(nil)
C(b.ring:IsShown() and b.disc:IsShown() and not b.flatDisc:IsShown() and vertex(b.ring)[2] == 1, "back to the client's look")
"#,
    );
    assert_eq!(
        out,
        [
            "default -57 -57",
            "click RightButton",
            "tooltip true",
            "dragged 0 80 0",
            "square 80 80",
            "click MiddleButton",
        ],
        "{out:?}"
    );
}

#[test]
fn launcher_uses_libdbicon_and_ldb() {
    let out = modes(
        &[("src/main.wowlua", LAUNCHER_MAIN)],
        r#"local objects, icons = {}, {}
local ldb = { NewDataObject = function(_, name, obj) objects[name] = obj; return obj end }
local dbicon = {
    Register = function(_, name, obj, db) icons[name] = { obj = obj, db = db } end,
    Show = function(_, name) print("dbicon show", name) end,
    Hide = function(_, name) print("dbicon hide", name) end,
    Refresh = function(_, name) print("dbicon refresh", name) end,
}
function LibStub(name, silent)
    return ({ ["LibDataBroker-1.1"] = ldb, ["LibDBIcon-1.0"] = dbicon })[name]
end
Mock.ldb = { objects = objects, icons = icons }
AddonCompartmentFrame = nil"#,
        r#"local C = Mock.check
Mock.loadAddon("Std")
Mock.fire("PLAYER_LOGIN")
local obj = Mock.ldb.objects.Std
C(obj and obj.type == "launcher" and obj.label == "Std", "an LDB launcher named by the addon")
C(Mock.ldb.icons.Std and Mock.ldb.icons.Std.obj == obj and Mock.ldb.icons.Std.db == StdLauncher.db.minimap, "LibDBIcon with the stored position")
C(StdMinimapButton == nil, "no built-in button")
obj.OnClick(nil, "LeftButton")
StdLauncher.launcher.showMinimap(false)
StdLauncher.launcher.setLabel("New")
C(obj.label == "New", "the LDB label")
StdLauncher.launcher.refresh()
"#,
    );
    assert_eq!(
        out,
        [
            "dbicon show Std",
            "click LeftButton",
            "dbicon hide Std",
            "dbicon refresh Std",
            "dbicon hide Std"
        ],
        "{out:?}"
    );
}

// -------------------------------------------------------------- serialize --

/// Test inputs for compression: empty, tiny, repetitive, pseudo-random,
/// every byte value, and real text.
fn compression_inputs() -> Vec<Vec<u8>> {
    let mut random = Vec::new();
    let mut x: u32 = 12345;
    for _ in 0..20000 {
        x = x.wrapping_mul(1103515245).wrapping_add(12345);
        random.push((x >> 16) as u8);
    }
    let text =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/stdlib/core.md")).unwrap();
    let mut big = Vec::new();
    while big.len() < 100_000 {
        big.extend_from_slice(&text);
    }
    vec![
        Vec::new(),
        b"a".to_vec(),
        b"hello hello hello hello".to_vec(),
        b"abc".repeat(5000),
        (0..=255u8).collect(),
        (0..=255u8).cycle().take(70_000).collect(),
        random,
        text,
        big,
    ]
}

#[test]
fn serialize_deflate_interoperates_with_zlib() {
    use flate2::Compression;
    use flate2::read::{DeflateDecoder, DeflateEncoder};
    use std::io::Read;
    let dir = project(&[(
        "src/main.wowlua",
        "import * as serialize from \"wowlua:serialize\"\n_G.Ser = serialize\n",
    )]);
    for mode in ["dev", "max"] {
        build(dir.path(), mode);
        let out = dir.path().join("dist").join(mode).join("Std");
        let mut vm = client(&out, "Std", "Std.toc", "", "");
        let ser = vm.get_global("Ser");
        let field = |vm: &mut Vm, name: &str| {
            let Value::Table(t) = ser else { panic!("Ser") };
            let k = vm.str(name);
            vm.heap.table(t).get(k)
        };
        let compress = field(&mut vm, "compress");
        let decompress = field(&mut vm, "decompress");
        // One call's first result: its bytes, or None for nil.
        let call = |vm: &mut Vm, f: Value, args: &[Value]| -> Option<Vec<u8>> {
            let r = vm
                .call(f, args)
                .unwrap_or_else(|e| panic!("{}", message(vm, &e)));
            match r.first() {
                Some(Value::Str(s)) => Some(vm.str_bytes(*s).to_vec()),
                _ => None,
            }
        };
        for (n, input) in compression_inputs().iter().enumerate() {
            for level in [0, 1, 6, 9] {
                // Ours, inflated by zlib.
                let arg = vm.bytes(input);
                let packed = call(&mut vm, compress, &[arg, Value::Num(f64::from(level))])
                    .unwrap_or_else(|| panic!("{mode} input {n} level {level}: no result"));
                let mut back = Vec::new();
                DeflateDecoder::new(&packed[..])
                    .read_to_end(&mut back)
                    .unwrap_or_else(|e| panic!("{mode} input {n} level {level}: zlib: {e}"));
                assert!(
                    back == *input,
                    "{mode} input {n} level {level}: zlib reads other bytes"
                );
                if level > 0 && input.len() > 1000 && n != 6 {
                    assert!(
                        packed.len() < input.len() / 2,
                        "{mode} input {n}: {} of {}",
                        packed.len(),
                        input.len()
                    );
                }
                // Ours, inflated by us.
                let arg = vm.bytes(&packed);
                let ours = call(&mut vm, decompress, &[arg]);
                assert!(
                    ours.is_some_and(|s| s == *input),
                    "{mode} input {n} level {level}: round trip"
                );
            }
            // zlib's, inflated by us.
            for level in [1, 6, 9] {
                let mut packed = Vec::new();
                DeflateEncoder::new(&input[..], Compression::new(level))
                    .read_to_end(&mut packed)
                    .unwrap();
                let arg = vm.bytes(&packed);
                let back = call(&mut vm, decompress, &[arg]);
                assert!(
                    back.is_some_and(|s| s == *input),
                    "{mode} input {n}: zlib level {level} not inflated"
                );
            }
        }
        // Corrupt data: nil, never an error.
        for bad in [&b"\xff\xff\xff"[..], b"\x07", b"", b"\x05\x00\x00\x00"] {
            let arg = vm.bytes(bad);
            let r = call(&mut vm, decompress, &[arg]);
            assert!(r.is_none(), "{mode}: {bad:?}");
        }
    }
}

#[test]
fn serialize_round_trips_and_rejects() {
    let out = modes(
        &[(
            "src/main.wowlua",
            r#"import * as serialize from "wowlua:serialize"
local C = Mock.check

local function same(a: any, b: any): boolean
    if type(a) ~= type(b) then return false end
    if type(a) ~= "table" then
        return a == b or (a ~= a and b ~= b)
    end
    for k, v in pairs(a) do
        if not same(v, b[k]) then return false end
    end
    for k in pairs(b) do
        if a[k] == nil then return false end
    end
    return true
end

local values: any[] = {
    0, 1, -1, 127, 128, 300, -300, 2 ^ 40, -(2 ^ 40), 2 ^ 53, 0.5, -1.25, 1 / 3, 1e300, -1e-300,
    math.huge, -math.huge, true, false, "", "a", "hello", string.rep("x", 1000),
    "\000\001\255 bytes",
    { 1, 2, 3 },
    { a = 1, b = { c = "deep", d = { true, false } } },
    { "same", "same", "same", key = "same" },
    { [1] = "one", [3] = "three", [1.5] = "half", [true] = "yes" },
}
for i, v in ipairs(values) do
    local ok, back = serialize.decode(serialize.encode(v))
    C(ok and same(v, back), "round trip " .. i)
end
-- Not `0 / 0`: the client raises "Division by zero" (decision S38).
local ok, nan = serialize.decode(serialize.encode(math.huge - math.huge))
C(ok and nan ~= nan, "NaN")
local nilOk, nothing = serialize.decode(serialize.encode(nil))
C(nilOk and nothing == nil, "nil")
-- Deduplicated strings make repeats short.
local repeated = {}
for i = 1, 100 do repeated[i] = "a repeated string" end
C(#serialize.encode(repeated) < 300, "strings deduplicated")

-- What cannot be encoded.
local cyclic: any = {}
cyclic.self = cyclic
C(not pcall(serialize.encode, cyclic), "a cycle")
C(not pcall(serialize.encode, { f = print }), "a function")
C(not pcall(serialize.encode, Mock.secret()), "a secret value")
local deep: any = {}
local node = deep
for _ = 1, 150 do node.child = {}; node = node.child end
C(not pcall(serialize.encode, deep), "too deep")

-- Decoding damaged or hostile input: false and a reason, never an error.
local good = serialize.encode({ 1, 2, { 3 } })
for _, bad in ipairs({ "", "\002", good:sub(1, -2), good .. "x", "\001\011\255\255\255\255\015", "\001\010\005" }) do
    local okBad, why = serialize.decode(bad)
    C(okBad == false and type(why) == "string", "rejects " .. #bad)
end
-- Decoding never runs code.
local evil = serialize.encode("os.exit()")
local _, text = serialize.decode(evil)
C(text == "os.exit()", "a string stays a string")

-- Printable and addon-channel encodings.
local all = {}
for i = 0, 255 do all[#all + 1] = string.char(i) end
local bytes = table.concat(all)
for _, s in ipairs({ "", "a", "ab", "abc", "abcd", bytes }) do
    local p = serialize.toPrintable(s)
    C(not p:find("[^%w%(%)]"), "printable characters")
    C(serialize.fromPrintable(p) == s, "printable round trip " .. #s)
    local a = serialize.toAddonChannel(s)
    C(not a:find("%z"), "no NUL")
    C(serialize.fromAddonChannel(a) == s, "addon channel round trip " .. #s)
end
C(serialize.fromPrintable("abc!") == nil, "invalid characters")
C(serialize.fromPrintable("  " .. serialize.toPrintable("x") .. "\n") == "x", "surrounding space")

-- Export strings.
local profile = { name = "Main", bars = { { id = 1, x = 10.5 }, { id = 2, x = -3 } }, enabled = true }
local exported = serialize.exportString(profile, { version = 3, prefix = "STDTEST" })
print(exported:sub(1, 11))
local okImport, imported = serialize.importString(exported)
C(okImport and same(profile, imported), "export round trip")
local okDefault = serialize.importString(serialize.exportString(1, { version = 1 }))
C(okDefault, "the default prefix")
local damaged = exported:sub(1, 20) .. (exported:sub(21, 21) == "a" and "b" or "a") .. exported:sub(22)
local okDamaged, why = serialize.importString(damaged)
print("damaged", okDamaged, why)
print("garbage", serialize.importString("hello"))
"#,
        )],
        "",
        "",
    );
    assert_eq!(out[0], "!STDTEST:3!", "{out:?}");
    assert!(
        out[1].starts_with("damaged false the export string is damaged"),
        "{out:?}"
    );
    assert_eq!(out[2], "garbage false not an export string", "{out:?}");
}

// ------------------------------------------------------------------- comm --

const COMM_MAIN: &str = r#"import * as comm from "wowlua:comm"

local got: string[] = {}
local ch = comm.open("StdComm")
ch:onMessage(function(msg: string, sender: string, dist: string)
    got[#got + 1] = `{sender} {dist} {#msg} {msg:sub(1, 12)}`
    _G.StdLast = msg
end)
ch:onObject(function(value: any, sender: string) got[#got + 1] = `object from {sender}: {value.name} {#value.list}` end)
local own = comm.open("StdOwn", { receiveOwn = true })
own:onMessage(function(msg: string, sender: string) got[#got + 1] = `own {sender} {msg}` end)
local ace = comm.open("StdAce", { format = "acecomm" })
ace:onMessage(function(msg: string, sender: string) got[#got + 1] = `ace {sender} {#msg} {msg:byte(1)}` end)
ace:onObject(function(value: any) got[#got + 1] = `ace object {value.name}` end)
_G.StdComm = { comm = comm, ch = ch, own = own, ace = ace, got = got }
"#;

#[test]
fn comm_sends_chunks_and_reassembles() {
    let out = modes(
        &[("src/main.wowlua", COMM_MAIN)],
        r#"Mock.group = "party"; Mock.echoAs = "Other""#,
        r#"local C = Mock.check
local S = StdComm
-- A short message.
C(S.ch:send("hello", "PARTY") == "sent", "sent at once")
Mock.advance(0.1)
-- A long one with NUL bytes, in chunks, with progress.
local parts = {}
for i = 1, 600 do parts[i] = string.char(i % 256) end
local long = table.concat(parts)
local progress = {}
S.ch:send(long, "GUILD", nil, { callback = function(sent, total) progress[#progress + 1] = sent .. "/" .. total end })
Mock.advance(0.5)
C(StdLast == long, "the long message reassembled")
for _, m in ipairs(Mock.sent) do C(#m.msg <= 255 and not m.msg:find("%z"), "chunks fit the channel") end
C(progress[#progress]:match("^(%d+)/(%d+)$"), "progress reported")
-- GROUP picks the channel.
Mock.group = "raid"
S.ch:send("to the raid", "GROUP")
C(Mock.sent[#Mock.sent].chatType == "RAID", "GROUP in a raid")
Mock.group = nil
C(S.ch:send("alone", "GROUP") == "dropped", "GROUP out of a group")
Mock.group = "party"
-- The player's own echo: only on a receiveOwn channel.
Mock.echoAs = "Tester"
S.ch:send("not for me", "PARTY")
S.own:send("for me", "PARTY")
Mock.advance(0.2)
Mock.echoAs = "Other"
-- Objects.
S.ch:sendObject({ name = "profile", list = { 1, 2, 3 } }, "PARTY")
Mock.advance(0.2)
print(table.concat(S.got, " | "))
"#,
    );
    assert_eq!(
        out,
        [
            "Other PARTY 5 hello | Other GUILD 600 \u{1}\u{2}\u{3}\u{4}\u{5}\u{6}\u{7}\u{8}\t\n\u{b}\u{c} | Other RAID 11 to the raid | own Tester for me | object from Other: profile 3"
        ],
        "{out:?}"
    );
}

#[test]
fn comm_throttles_and_waits() {
    let out = modes(
        &[("src/main.wowlua", COMM_MAIN)],
        r#"Mock.group = "party"; Mock.echoAs = "Other""#,
        r#"local C = Mock.check
local S = StdComm
-- More than the built-in budgets allow at once: queued, then sent.
local big = string.rep("x", 6000)
C(S.ch:send(big, "PARTY") == "queued", "queued over the budget")
local first = #Mock.sent
C(first > 0 and first < 25, "a first burst " .. first)
Mock.advance(30)
C(StdLast == big, "all sent and reassembled")
-- The server throttles: the chunk is retried.
Mock.sendResults = { 3 }
C(S.ch:send("retry me", "PARTY") == "queued", "queued after a throttle")
Mock.advance(2)
C(StdLast == "retry me", "sent after the throttle")
-- Messaging lockdown: queued, or dropped on request; sent when it ends.
Mock.lockdown = true
C(S.ch:send("later", "PARTY") == "queued", "queued in lockdown")
C(S.ch:send("never", "PARTY", nil, { dropInLockdown = true }) == "dropped", "dropped in lockdown")
Mock.advance(3)
C(StdLast ~= "later", "nothing during lockdown")
Mock.lockdown = false
Mock.advance(2)
C(StdLast == "later", "sent once lockdown ends")
-- A failure: reported, the message dropped.
Mock.sendResults = { 5 }
C(S.ch:send("fails", "PARTY") == "dropped", "a failed send")
print("done")
"#,
    );
    assert_eq!(plain(&out), ["done"], "{out:?}");
    assert!(
        errors(&out)
            .iter()
            .any(|e| e.contains("sending on StdComm failed (result 5)")),
        "{out:?}"
    );
}

#[test]
fn comm_acecomm_format_and_chatthrottlelib() {
    let out = modes(
        &[("src/main.wowlua", COMM_MAIN)],
        r#"Mock.group = "party"; Mock.echoAs = "Other"
Mock.ctl = {}
ChatThrottleLib = {
    SendAddonMessage = function(self, prio, prefix, text, chattype, target, queueName, callback, arg)
        table.insert(Mock.ctl, prio .. " " .. queueName)
        C_ChatInfo.SendAddonMessage(prefix, text, chattype, target)
        callback(arg, true)
    end,
}"#,
        r#"local C = Mock.check
local S = StdComm
-- Through ChatThrottleLib, with the priority.
C(S.ace:send("\002starts with a control byte", "PARTY", nil, { priority = "bulk" }) == "sent", "sent through ChatThrottleLib")
C(Mock.ctl[1] == "BULK StdAce", "priority and queue " .. tostring(Mock.ctl[1]))
C(Mock.sent[1].msg:sub(1, 1) == "\004", "AceComm escapes a leading control byte")
local long = string.rep("y", 600)
S.ace:send(long, "PARTY")
local markers = {}
for i = 2, #Mock.sent do markers[#markers + 1] = Mock.sent[i].msg:byte(1) end
print("markers", table.concat(markers, " "))
S.ace:sendObject({ name = "ace profile" }, "PARTY")
-- A chunk without its start, and an unknown control byte: ignored.
Mock.addonMessage("StdComm", "\003orphan", "PARTY", "Someone-MockRealm")
Mock.addonMessage("StdComm", "\009future", "PARTY", "Someone-MockRealm")
Mock.advance(0.5)
print(table.concat(S.got, " | "))
"#,
    );
    assert_eq!(
        out,
        [
            "markers 1 2 3",
            "ace Other 27 2 | ace Other 600 121 | ace object ace profile"
        ],
        "{out:?}"
    );
}
