//! The standard library (spec chapter 8) at run time: addons that import
//! `wowlua:*` modules are built and run against a model of the WoW client
//! (`wowmock.wowlua`), driven by a test script. Every runtime test runs the
//! addon in three builds (dev, release, and release fully minified with the
//! libraries minified too), which must behave alike.

use assert_cmd::Command;
use std::fs;
use std::path::Path;

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

/// Runs a snippet in `lua` (its runtime first, if any), with an addon
/// table of its own, as the client runs an addon's files.
fn exec_snippet(lua: &mlua::Lua, source: &str, name: &str) -> mlua::Result<()> {
    let snippet = wowlua_snippet(source);
    let ns = lua.create_table()?;
    if let Some(runtime) = &snippet.runtime {
        lua.load(runtime.as_str())
            .set_name(format!("{name} (runtime)"))
            .call::<()>(("Snippet", ns.clone()))?;
    }
    lua.load(snippet.lua.as_str())
        .set_name(name)
        .call::<()>(("Snippet", ns))
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
    let lua = client(out, name, toc, label, setup);
    exec_snippet(&lua, script, "@test").unwrap_or_else(|e| panic!("{label}{e}"));
    printed(&lua)
}

/// A mock client with `setup` run and the addon's files loaded.
fn client(out: &Path, name: &str, toc: &str, label: &str, setup: &str) -> mlua::Lua {
    let toc = fs::read_to_string(out.join(toc)).unwrap();
    let lua = mlua::Lua::new();
    exec_snippet(&lua, MOCK, "@wowmock.wowlua").unwrap();
    exec_snippet(&lua, setup, "@setup").unwrap_or_else(|e| panic!("{label}setup: {e}"));
    let ns = lua.create_table().unwrap();
    for line in toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let path = line.split(" [").next().unwrap().replace('\\', "/");
        if !path.ends_with(".lua") {
            continue;
        }
        let code = fs::read(out.join(&path)).unwrap();
        lua.load(code)
            .set_name(format!("@{path}"))
            .call::<()>((name, ns.clone()))
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
            lua.load(format!(
                "Mock.addBinding({binding:?}, function(keystate) if keystate == 'up' and not {on_up} then return end {body} end)"
            ))
            .set_name("@Bindings.xml")
            .exec()
            .unwrap_or_else(|e| panic!("{label}Bindings.xml: {e}"));
        }
    }
    lua
}

/// What the mock client printed, one line each.
fn printed(lua: &mlua::Lua) -> Vec<String> {
    let printed: mlua::Table = lua.load("return Mock.out").eval().unwrap();
    printed
        .sequence_values::<String>()
        .map(Result::unwrap)
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
    assert!(
        reported.contains(&"Std: repeated (2 more)".to_string()),
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
            // The unit handler and the plain one live on different frames.
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
            r#"import on, onUnit, send from "wowlua:events"
local C = Mock.check

-- The shared frame registers plain events (without a unit filter).
local function registered(name: string): boolean
    for _, f in ipairs(Mock.frames) do
        if f.events[name] and not f.units[name] then return true end
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
local frames = #Mock.frames
local u1 = onUnit("UNIT_HEALTH", { "player" }, function() end)
local u2 = onUnit("UNIT_HEALTH", { "player" }, function() end)
C(#Mock.frames == frames + 1, "equal unit sets share a frame")
local u3 = onUnit("UNIT_HEALTH", { "target", "focus" }, function() end)
C(#Mock.frames == frames + 2, "another unit set gets its own frame")
local frame = Mock.frames[frames + 1]
C(frame.units.UNIT_HEALTH.player == true, "unit filter set")
u1:cancel()
C(frame.events.UNIT_HEALTH == true, "still registered for the second handler")
u2:cancel()
C(frame.events.UNIT_HEALTH == nil, "unregistered without handlers")
u3:cancel()

-- WoW's limit of two units, at run time too.
local units = { "a", "b", "c" }
C(not pcall(onUnit, "UNIT_HEALTH", units, function() end), "three units are refused")
local none: string[] = {}
C(not pcall(onUnit, "UNIT_HEALTH", none, function() end), "no units is refused")

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
    -- Removed during the dispatch: still ran this time.
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

-- Changes during a fire apply to the next one.
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
            "tick 2",
            "tick 1",
            "tick late",
            "solo false false",
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
            "import onUnit from \"wowlua:events\"\nonUnit(\"UNIT_HEALTH\", { \"player\", \"target\", \"focus\" }, function() end)\n",
            "E8100",
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
            "import unitEvent from \"wowlua:events\"\n@unitEvent(\"UNIT_HEALTH\", \"player\", \"target\", \"focus\")\nlocal function f() end\n",
            "E8100",
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
    Mock.advance(0.05)
end
"#,
    );
    let text = printed.join("\n");
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
        Mock.advance(0.05)
    end
    error("never printed: " .. text)
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
    let lua = client(&out, "StdTest", toc, "", &setup);
    exec_snippet(
        &lua,
        r#"Mock.loadAddon("StdTest")
Mock.fire("PLAYER_LOGIN")
Mock.fire("PLAYER_ENTERING_WORLD", true, false)
Mock.slash("/stdtest")
Mock.awaitText("Press and release CTRL-SHIFT-K")
Mock.pressKey("CTRL-SHIFT-K")
Mock.awaitText("Enter combat")
Mock.fire("PLAYER_REGEN_DISABLED")
Mock.awaitText("Now leave combat")
Mock.fire("PLAYER_REGEN_ENABLED")
Mock.awaitText("Change your target")
Mock.fire("PLAYER_TARGET_CHANGED")
Mock.fire("UNIT_TARGET", "player")
Mock.fire("UNIT_TARGET", "target")
Mock.awaitText("Drag the StdTest box")
Mock.modifiers = {}
Mock.drag(StdTestMovableBox, 400, 300)
Mock.awaitText("Resize the box")
for _, f in ipairs(Mock.frames) do
    if f.parent == StdTestMovableBox and f.kind == "Button" then Mock.sizeBy(f, 60, 30) end
end
Mock.awaitText("Hold Ctrl")
Mock.modifiers.CTRL = true
Mock.wheel(StdTestMovableBox, 1)
Mock.modifiers.CTRL = nil
Mock.awaitText("Click the question-mark button")
Mock.script(StdTestMinimapButton, "OnClick", "LeftButton")
Mock.awaitText("Mouse over any item in your bags")
Mock.showTooltip("Item", { id = 6948, name = "Hearthstone", link = "|Hitem:6948::|h[Hearthstone]|h" })
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
    let before = printed(&lua).join("\n");
    let saved: String = lua.load("return Mock.saved").eval().unwrap();
    drop(lua);
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
C(made and f.kind == "Button", "a new button")
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
for _, f in ipairs(Mock.frames) do
    if f.parent == StdWindow and f.kind == "Button" then grip = f end
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
print("locked", StdMove:isLocked(), grip.shown)
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
StdGuarded.protected = true
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
        r#"print("before", StdWindow:GetPoint(1))
Mock.loadAddon("Std")
local point, rel, relPoint, x, y = StdWindow:GetPoint(1)
print("after", point, rel:GetName(), relPoint, x, y, StdWindow:GetWidth(), StdWindow:GetHeight(), StdWindow:GetScale())
"#,
    );
    // Offsets are in the frame's own (scaled) units: 40 / 0.8, -60 / 0.8.
    assert_eq!(
        plain(&out),
        [
            "before CENTER",
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
            "dragged TOPRIGHT TOPRIGHT -62 -34 300 200 1",
            // The top left stays: the frame now reaches past the screen.
            "resized TOPRIGHT TOPRIGHT 138 -34 500 250 1",
            "no ctrl TOPRIGHT TOPRIGHT 138 -34 500 250 1",
            "scaled TOPRIGHT TOPRIGHT 138 -34 500 250 1.1",
            "locked true false",
            "still TOPRIGHT TOPRIGHT 138 -34 500 250 1.1",
            "reset nil",
            "anchor CENTER 300 200 1",
            "changes 4",
            "no shift nil",
            "shift BOTTOM BOTTOM -2 10 - - 1",
            "in combat BOTTOMLEFT UIParent BOTTOMLEFT 460 10",
            "after combat TOPLEFT UIParent TOPLEFT 20 -20",
        ],
        "{out:?}"
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
C(b.icon.texture == "icon-2", "the button's icon")
-- The compartment entry.
local entry = AddonCompartmentFrame.registeredAddons[1]
C(entry and entry.text == "Std" and entry.icon == "icon-2", "the compartment entry")
entry.func(nil, { buttonName = "MiddleButton" })
StdLauncher.launcher.setLabel("Std 2")
C(entry.text == "Std 2", "the entry's label")
C(not pcall(StdLauncher.again, { icon = "x", label = "y", onClick = function() end }), "register once")
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
        let lua = client(&out, "Std", "Std.toc", "", "");
        let ser: mlua::Table = lua.globals().get("Ser").unwrap();
        let compress: mlua::Function = ser.get("compress").unwrap();
        let decompress: mlua::Function = ser.get("decompress").unwrap();
        for (n, input) in compression_inputs().iter().enumerate() {
            for level in [0, 1, 6, 9] {
                // Ours, inflated by zlib.
                let packed: mlua::LuaString = compress
                    .call((lua.create_string(input).unwrap(), level))
                    .unwrap_or_else(|e| panic!("{mode} input {n} level {level}: {e}"));
                let packed = packed.as_bytes().to_vec();
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
                let ours: Option<mlua::LuaString> = decompress
                    .call(lua.create_string(&packed).unwrap())
                    .unwrap();
                assert!(
                    ours.is_some_and(|s| *s.as_bytes() == **input),
                    "{mode} input {n} level {level}: round trip"
                );
            }
            // zlib's, inflated by us.
            for level in [1, 6, 9] {
                let mut packed = Vec::new();
                DeflateEncoder::new(&input[..], Compression::new(level))
                    .read_to_end(&mut packed)
                    .unwrap();
                let back: Option<mlua::LuaString> = decompress
                    .call(lua.create_string(&packed).unwrap())
                    .unwrap();
                assert!(
                    back.is_some_and(|s| *s.as_bytes() == **input),
                    "{mode} input {n}: zlib level {level} not inflated"
                );
            }
        }
        // Corrupt data: nil, never an error.
        for bad in [&b"\xff\xff\xff"[..], b"\x07", b"", b"\x05\x00\x00\x00"] {
            let r: Option<mlua::LuaString> =
                decompress.call(lua.create_string(bad).unwrap()).unwrap();
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
local ok, nan = serialize.decode(serialize.encode(0 / 0))
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
Mock.fire("CHAT_MSG_ADDON", "StdComm", "\003orphan", "PARTY", "Someone-MockRealm")
Mock.fire("CHAT_MSG_ADDON", "StdComm", "\009future", "PARTY", "Someone-MockRealm")
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
