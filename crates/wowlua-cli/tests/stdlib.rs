//! The standard library (spec chapter 8) at run time: addons that import
//! `wowlua:*` modules are built and run against a model of the WoW client
//! (`wowmock.lua`), driven by a test script.

use assert_cmd::Command;
use std::fs;
use std::path::Path;

const MOCK: &str = include_str!("wowmock.lua");

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
  "addon": { "saved-variables": ["StdDB"] } }
"#;
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
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

/// Loads the built addon into the mock client and runs `script`; returns
/// what was printed, one line each.
fn run(dir: &Path, profile: &str, script: &str) -> Vec<String> {
    let out = dir.join("dist").join(profile).join("Std");
    let toc = fs::read_to_string(out.join("Std.toc")).unwrap();
    let lua = mlua::Lua::new();
    lua.load(MOCK).set_name("@wowmock.lua").exec().unwrap();
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
            .call::<()>(("Std", ns.clone()))
            .unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    lua.load(script)
        .set_name("@test")
        .exec()
        .unwrap_or_else(|e| panic!("{e}"));
    let printed: mlua::Table = lua.load("return Mock.out").eval().unwrap();
    printed
        .sequence_values::<String>()
        .map(Result::unwrap)
        .collect()
}

/// Builds in dev and release and checks both behave the same.
fn both(files: &[(&str, &str)], script: &str) -> Vec<String> {
    let dir = project(files);
    build(dir.path(), "dev");
    build(dir.path(), "release");
    let dev = run(dir.path(), "dev", script);
    let release = run(dir.path(), "release", script);
    // Error messages differ: dev builds capture them and name the handler.
    let plain = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .filter(|l| !l.contains("ERROR:") && !l.contains("error captured"))
            .cloned()
            .collect()
    };
    assert_eq!(
        plain(&dev),
        plain(&release),
        "dev and release builds differ"
    );
    assert_eq!(
        dev.iter().filter(|l| l.contains("ERROR:")).count(),
        release.iter().filter(|l| l.contains("ERROR:")).count(),
        "dev and release report different errors"
    );
    dev
}

#[test]
fn events_dispatch_isolated_in_order() {
    let out = both(
        &[(
            "src/main.wowlua",
            r#"import event, unitEvent, on, once, off, offAll, send, onUnit from "wowlua:events"

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

-- a message, dispatched in Lua only
local count = 0
local sub = on("STD_PING", function(n: number) count += n; print("ping", n) end)
local function sendPing(n: number) send("STD_PING", n) end
_G.StdPing = sendPing
_G.StdUnsub = function() off(sub) end
_G.StdOffAll = function() offAll(owner) end

-- registering during a dispatch applies to the next one
on("PLAYER_STARTED_MOVING", function()
    print("moving")
    on("PLAYER_STARTED_MOVING", function() print("added later") end)
end)
"#,
        )],
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
Mock.fire("PLAYER_STARTED_MOVING")
Mock.fire("PLAYER_STARTED_MOVING")
"#,
    );
    let out = out.join("\n");
    let expected = [
        "first true false",
        "ERROR: Std: ",
        "third",
        "bags BAG_UPDATE 3",
        "bags BAG_UPDATE_DELAYED",
        "any health target",
        "login once",
        "ping 2",
        "moving",
        "moving\nadded later",
    ];
    let mut at = 0;
    for e in expected {
        let found = out[at..]
            .find(e)
            .unwrap_or_else(|| panic!("`{e}` not found in order:\n{out}"));
        at += found + e.len();
    }
    assert!(out.contains("boom"), "{out}");
    assert_eq!(out.matches("login once").count(), 1, "{out}");
    assert_eq!(out.matches("ping").count(), 1, "{out}");
    assert_eq!(out.matches("any health target").count(), 1, "{out}");
    // The unit filter: only the player's health reaches the unit handler.
    assert_eq!(out.matches("health player").count(), 2, "{out}");
    assert!(!out.contains("\nhealth target"), "{out}");
    assert!(
        out.contains("boom [events: PLAYER_ENTERING_WORLD handler]"),
        "{out}"
    );
}

#[test]
fn lifecycle_runs_its_phases_in_order() {
    let out = both(
        &[(
            "src/main.wowlua",
            r#"import onLoad, onLogin, onEnterWorld, onLogout, onAddonLoaded, isLoaded, isLoggedIn from "wowlua:lifecycle"

@onLoad
local function load1() print("load 1", isLoaded()) end

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
"#,
        )],
        r#"Mock.loadAddon("Other")
print("before")
Mock.loadAddon("Std")
Mock.fire("PLAYER_LOGIN")
Mock.fire("PLAYER_ENTERING_WORLD", true, false)
Mock.loadAddon("Blizzard_Later")
Mock.loadAddon("Blizzard_Later")
Mock.fire("PLAYER_ENTERING_WORLD", false, true)
Mock.fire("PLAYER_LOGOUT")
"#,
    );
    let plain: Vec<&String> = out
        .iter()
        .filter(|l| !l.contains("ERROR") && !l.contains("error captured"))
        .collect();
    assert_eq!(
        plain,
        [
            "before",
            "load 1 true",
            "load 2",
            "login true",
            "already loaded",
            "world true false",
            "later loaded",
            "world false true",
            "logout",
        ],
        "{out:?}"
    );
    assert!(out.iter().any(|l| l.contains("load failed")), "{out:?}");
}

#[test]
fn timers_throttle_debounce_and_next_frame() {
    let out = both(
        &[(
            "src/main.wowlua",
            r#"import after, every, throttle, debounce, nextFrame from "wowlua:timer"

local t = after(2, function() print("after 2") end)
local cancelled = after(1, function() print("never") end)
cancelled:Cancel()
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
fn combat_queues_until_combat_ends() {
    let out = both(
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
            "layout one 3 false",
            "layout two 5 false",
            "deferred",
            "combat false",
        ],
        "{out:?}"
    );
}

#[test]
fn hooks_post_hook_globals_methods_and_scripts() {
    let out = both(
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

/// Runs `script` with saved data set up by `setup` (before the addon's
/// files run, as WoW loads SavedVariables).
fn run_with(dir: &Path, profile: &str, setup: &str, script: &str) -> Vec<String> {
    run(dir, profile, &format!("{setup}\n{script}"))
}

#[test]
fn saved_variables_load_migrate_default_and_strip() {
    let dir = project(&[(
        "src/main.wowlua",
        r#"import * as saved from "wowlua:saved"
import onLoad, onLogout from "wowlua:lifecycle"

interface Settings
    scale: number
    hidden: { [string]: boolean }
    window: { x: number, y: number }
end

local db = saved.bind("StdDB", {
    version = 3,
    defaults = { scale = 1, hidden = {}, window = { x = 0, y = 0 } } :: Settings,
    migrate = {
        [2] = function(old: any) old.scale = old.uiScale; old.uiScale = nil end,
        [3] = function(old: any) old.hidden = old.hidden or {} end,
    },
})

local ok, err = pcall(function() return db.scale end)
print("early", ok, tostring(err):match("accessed before it was loaded") ~= nil)

@onLoad
local function init()
    print("scale", db.scale, "x", db.window.x)
    db.window.y = 7
    db.hidden.minimap = true
    print("handle loaded", saved.handle(db).isLoaded())
end

@onLogout
local function bye() print("logout", db.scale) end
_G.StdDb = db
"#,
    )]);
    build(dir.path(), "dev");
    let script = r#"
Mock.loadAddon("Std")
Mock.fire("PLAYER_LOGOUT")
local d = StdDB
print("saved", d.__version, rawget(d, "scale"), d.uiScale, d.window and rawget(d.window, "y"), d.window and rawget(d.window, "x"), d.hidden and d.hidden.minimap)
print("same table", StdDb == StdDB)
"#;
    // Stored at version 1: migrations 2 and 3 run; defaults equal values go.
    let out = run_with(
        dir.path(),
        "dev",
        "StdDB = { __version = 1, uiScale = 2 }",
        script,
    );
    assert_eq!(
        out,
        [
            "early false true",
            "scale 2 x 0",
            "handle loaded true",
            "logout 2",
            "saved 3 2 nil 7 nil true",
            "same table true",
        ],
        "{out:?}"
    );
    // A fresh install runs no migrations; a value back at its default goes.
    let out = run_with(dir.path(), "dev", "", script);
    assert_eq!(out[1], "scale 1 x 0", "{out:?}");
    assert_eq!(out[4], "saved 3 nil nil 7 nil true", "{out:?}");
    // Saved by a newer version: kept unchanged, nothing stripped.
    let out = run_with(
        dir.path(),
        "dev",
        "StdDB = { __version = 9, scale = 1 }",
        script,
    );
    assert!(
        out.iter().any(|l| l.contains("saved by a newer version")),
        "{out:?}"
    );
    assert!(out.iter().any(|l| l.starts_with("saved 9 1 ")), "{out:?}");
}

#[test]
fn saved_migration_failure_preserves_the_data() {
    let dir = project(&[(
        "src/main.wowlua",
        r#"import * as saved from "wowlua:saved"
import onLoad from "wowlua:lifecycle"

local db = saved.bind("StdDB", {
    version = 2,
    defaults = { scale = 1 },
    migrate = { [2] = function(old: any) old.scale = 5; error("bad migration") end },
})

@onLoad
local function init()
    local ok, err = pcall(function() return db.scale end)
    print("access", ok, tostring(err):match("failed to migrate to version 2; data preserved") ~= nil)
end
"#,
    )]);
    build(dir.path(), "dev");
    let out = run_with(
        dir.path(),
        "dev",
        "StdDB = { __version = 1, scale = 3 }",
        "Mock.loadAddon(\"Std\")\nMock.fire(\"PLAYER_LOGOUT\")\nprint(\"stored\", StdDB.__version, StdDB.scale)",
    );
    assert!(out.iter().any(|l| l.contains("bad migration")), "{out:?}");
    assert!(out.contains(&"access false true".to_string()), "{out:?}");
    assert!(out.contains(&"stored 1 3".to_string()), "{out:?}");
}

#[test]
fn saved_profiles_switch_per_character() {
    let dir = project(&[(
        "src/main.wowlua",
        r#"import * as saved from "wowlua:saved"
import onLoad from "wowlua:lifecycle"

local db = saved.bind("StdDB", { version = 1, defaults = { scale = 1 }, profiles = true })

@onLoad
local function init()
    local profiles = saved.handle(db).profiles
    profiles:onChange(function(name: string) print("changed to", name) end)
    print("current", profiles:current(), db.scale)
    db.scale = 2
    profiles:use("Healer")
    print("healer", db.scale)
    profiles:copy("Default")
    print("copied", db.scale)
    print("list", table.concat(profiles:list(), ","))
end
"#,
    )]);
    build(dir.path(), "dev");
    let out = run_with(
        dir.path(),
        "dev",
        "",
        "Mock.loadAddon(\"Std\")\nMock.fire(\"PLAYER_LOGOUT\")\nprint(\"char\", StdDBPerChar.profile, StdDB.profiles.Default.scale)",
    );
    assert_eq!(
        out,
        [
            "current Default 1",
            "changed to Healer",
            "healer 1",
            "copied 2",
            "list Default,Healer",
            "char Healer 2",
        ],
        "{out:?}"
    );
}

#[test]
fn slash_commands_and_the_router() {
    let out = both(
        &[(
            "src/main.wowlua",
            r#"import slash, router from "wowlua:slash"

local commands = router({
    show = { run = function() print("show") end, help = "Show the window", aliases = { "open" } },
    scale = { args = { "number" }, run = function(v: number) print("scale", v) end, help = "Set the scale" },
    mode = { args = { { "tank", "heal" }, "boolean?" }, run = function(m: string, on: boolean?) print("mode", m, on) end },
    say = { args = { "string", "rest" }, run = function(who: string, text: string) print("say", who, "[" .. text .. "]") end },
    reset = { flags = { "all" }, run = function(flags: { all: boolean }) print("reset", flags.all) end },
})

@slash("/std", "/stdlib")
local function onCommand(msg: string) commands:dispatch(msg) end

@slash("/stdping")
local function ping(msg: string) print("ping", msg) end
"#,
        )],
        r#"Mock.slash("/std show")
Mock.slash("/STDLIB OPEN")
Mock.slash("/std scale 1.5")
Mock.slash("/std scale big")
Mock.slash("/std mode HEAL yes")
Mock.slash("/std mode tank")
Mock.slash([[/std say "two words" hello there  friend]])
Mock.slash("/std reset --all")
Mock.slash("/std reset")
Mock.slash("/std reset --nope")
Mock.slash("/stdping 42")
Mock.slash("/std")
print("keys", SLASH_WOWLUA_STD_11, SLASH_WOWLUA_STD_12, SLASH_WOWLUA_STD_21)
"#,
    );
    let out: Vec<String> = out
        .iter()
        .map(|l| l.replace("|cff33ff99Std|r: ", ""))
        .collect();
    assert_eq!(
        out,
        [
            "show",
            "show",
            "scale 1.5",
            "Usage: scale <number>",
            "mode heal true",
            "mode tank nil",
            "say two words [hello there  friend]",
            "reset true",
            "reset false",
            "Usage: reset [--all]",
            "ping 42",
            "mode <tank|heal> [boolean]",
            "reset [--all]",
            "say <string> <text...>",
            "scale <number> - Set the scale",
            "show - Show the window",
            "keys /std /stdlib /stdping",
        ],
        "{out:?}"
    );
}

#[test]
fn callback_registries_and_errors_api() {
    let out = both(
        &[(
            "src/main.wowlua",
            r#"import * as callbacks from "wowlua:callbacks"
import * as errors from "wowlua:errors"

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

-- a plain Lua consumer, CallbackHandler style
local consumer = { OnKill = function(self: any, name: string, n: number) print("consumer", name, n) end }
events.RegisterCallback(consumer, "Kill", "OnKill")
events:fire("Kill", 4)
events.UnregisterAllCallbacks(consumer)
events:fire("Kill", 5)

errors.onError(function(msg: string) print("noticed", msg:match("wrapped failure") ~= nil) end)
local safe = errors.wrap(function() error("wrapped failure") end)
safe()
local ok, a, b = errors.call(function(x: number) return x, x * 2 end, 21)
print("call", ok, a, b)
print("count", errors.count())
"#,
        )],
        "",
    );
    let plain: Vec<&String> = out
        .iter()
        .filter(|l| !l.contains("ERROR") && !l.contains("error captured"))
        .collect();
    assert_eq!(
        plain,
        [
            "has true false",
            "kill 1",
            "first kill 1",
            "kill 2",
            "consumer Kill 4",
            "noticed true",
            "call true 21 42",
            // The repeated handler failure is rate limited: reported once.
            "count 2",
        ],
        "{out:?}"
    );
}
