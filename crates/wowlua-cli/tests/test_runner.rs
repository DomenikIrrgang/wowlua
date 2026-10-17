//! `wowlua test` (spec Tooling §16.1): test files of a package run in the
//! emulated client, per flavor.

use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn wowlua(dir: &Path) -> Command {
    // The examples are in the repository: their home goes elsewhere.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let home = if dir.starts_with(&repo) {
        std::env::temp_dir().join(format!("wowlua-home-{}", std::process::id()))
    } else {
        dir.join(".wowlua-home")
    };
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", home)
        .env_remove("WOWLUA_WOW_PATH")
        .env_remove("WOWLUA_STDLIB_DIR");
    cmd
}

/// A package `Demo` (`flavors`: the manifest's entries) with these files.
fn project(flavors: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = format!(
        r#"{{ "package": {{ "name": "demo", "version": "1.0.0" }}, "flavors": {{ {flavors} }},
  "addon": {{ "title": "Demo", "saved-variables": ["DemoDB"] }} }}"#
    );
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

/// Runs `wowlua test` with `args`: success and stdout + stderr.
fn test(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = wowlua(dir).arg("test").args(args).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

const TRACKER: &str = r#"
export class Tracker
    kills: number = 0
    static function create(): Tracker
        return Tracker()
    end
    function record(n: number?)
        self.kills = self.kills + (n ?? 1)
    end
end
export function double(x: number): number
    return x * 2
end
"#;

#[test]
fn runs_tests_hooks_and_cases() {
    let dir = project(
        r#""standard": {}"#,
        &[
            ("src/tracker.wowlua", TRACKER),
            (
                "tests/tracker.test.wowlua",
                r#"
import test, beforeEach, afterEach, beforeAll, afterAll, expect, cases, skip from "wowlua:test"
import Tracker, double from "/tracker"

local tracker: Tracker
local order: string[] = {}

@beforeAll
local function first() order[#order + 1] = "all" end
@beforeEach
local function setup()
    order[#order + 1] = "each"
    tracker = Tracker.create()
end
@afterEach
local function teardown() order[#order + 1] = "after" end
@afterAll
local function last() print(table.concat(order, " ")) end

@test("records kills")
local function recordsKills()
    tracker:record(2)
    expect(tracker.kills).toBe(2)
    expect(tracker.kills).never.toBe(3)
end

@test("doubles")
@cases({ { 1, 2 }, { 2, 4 } })
local function doubles(n: number, expected: number)
    expect(double(n)).toBe(expected)
end

@skip("not yet")
@test("later")
local function later() end

@test
local function deepEquality()
    expect({ a = { 1, 2 }, b = "x" }).toEqual({ b = "x", a = { 1, 2 } })
    expect("hello").toContain("ell")
    expect({ 1, 2, 3 }).toHaveLength(3)
    expect(0.1 + 0.2).toBeCloseTo(0.3)
    expect(function() error("boom") end).toThrow("boom")
end
"#,
            ),
        ],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    for line in [
        "PASS tests/tracker.test.wowlua (standard",
        "✓ records kills",
        "✓ doubles [1, 2]",
        "✓ doubles [2, 4]",
        "- later (not yet)",
        "✓ deepEquality",
        "4 passed, 0 failed, 1 skipped (1 file)",
    ] {
        assert!(out.contains(line), "missing {line:?} in:\n{out}");
    }
}

#[test]
fn failures_name_the_source_line_and_fail_the_run() {
    let dir = project(
        r#""standard": {}"#,
        &[
            ("src/tracker.wowlua", TRACKER),
            (
                "tests/fail.test.wowlua",
                r#"import test, expect from "wowlua:test"
import double from "/tracker"

@test("wrong")
local function wrong()
    expect(double(2)).toBe(5)
end

@test("raises")
local function raises()
    local t: any = nil
    print(t.field)
end
"#,
            ),
        ],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("expected 4 to be 5"), "{out}");
    assert!(out.contains("at tests/fail.test.wowlua:6"), "{out}");
    assert!(out.contains("attempt to index"), "{out}");
    assert!(out.contains("at tests/fail.test.wowlua:12"), "{out}");
    assert!(out.contains("0 passed, 2 failed"), "{out}");
}

#[test]
fn filter_only_and_flavors_select_tests() {
    let dir = project(
        r#""standard": {}, "vanilla": {}"#,
        &[
            (
                "tests/a.test.wowlua",
                r#"import test, expect, flavors from "wowlua:test"

@test("everywhere")
local function everywhere() expect(1).toBe(1) end

@test("retail only")
@flavors("standard")
local function retail() expect(1).toBe(1) end
"#,
            ),
            (
                "tests/b.test.wowlua",
                r#"import test, only, expect from "wowlua:test"

@test("not this")
local function notThis() expect(1).toBe(2) end

@only
@test("this")
local function this() expect(1).toBe(1) end
"#,
            ),
        ],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    // a: 2 tests on standard, 1 on classic; b: 1 per flavor.
    assert!(
        out.contains("5 passed, 0 failed, 0 skipped (4 files)"),
        "{out}"
    );
    assert!(!out.contains("not this"), "{out}");
    let (ok, out) = test(dir.path(), &["everywhere", "--flavor", "vanilla"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("1 passed, 0 failed, 0 skipped (1 file)"),
        "{out}"
    );
}

#[test]
fn lifecycle_mocks_helpers_and_the_world() {
    let dir = project(
        r#""standard": {}"#,
        &[
            (
                "src/main.wowlua",
                r#"local addonName: string = ...
local f = CreateFrame("Frame")
f:RegisterEvent("ADDON_LOADED")
f:RegisterEvent("PLAYER_LOGIN")
f:SetScript("OnEvent", function(_: Frame, event: string, name: string?)
    if event == "ADDON_LOADED" and name == addonName then
        DemoDB = DemoDB or { logins = 0 }
    elseif event == "PLAYER_LOGIN" then
        DemoDB.logins = DemoDB.logins + 1
        print("hello " .. (MockedName ?? "?"))
    end
end)
"#,
            ),
            (
                "tests/mocks/name.wowlua",
                "MockedName = \"from the mock\"\n",
            ),
            (
                "tests/helpers/numbers.wowlua",
                "export function answer(): number\n    return 42\nend\n",
            ),
            (
                "tests/auto.test.wowlua",
                r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"
import answer from "./helpers/numbers"

@test("logged in once, mocks first")
local function loggedIn()
    expect(DemoDB.logins).toBe(1)
    expect(mock.printed()).toContain("hello from the mock")
    expect(answer()).toBe(42)
end

@test("spies and stubs are restored")
local function stubs()
    local spy = mock.stub(_G, "GetTime", function(): number return 5 end)
    expect(GetTime()).toBe(5)
    expect(spy).toHaveBeenCalledTimes(1)
end

@test("after a stub")
local function afterStub()
    expect(GetTime()).never.toBe(5)
end

@test("timers and waiting")
local function waiting()
    local spy = mock.spy()
    C_Timer.NewTicker(1, spy)
    mock.advanceTime(3.5)
    expect(spy).toHaveBeenCalledTimes(3)
    local fired = false
    C_Timer.After(2, function() fired = true end)
    mock.waitSeconds(2.5)
    expect(fired).toBe(true)
    C_Timer.After(1, function() mock.fireEvent("PLAYER_REGEN_ENABLED") end)
    mock.waitFor("PLAYER_REGEN_ENABLED")
end

@test("a wait times out")
local function timesOut()
    mock.waitUntil(function(): boolean return false end, "nothing")
end
"#,
            ),
            (
                "tests/manual.test.wowlua",
                r#"import test, expect, config from "wowlua:test"
import * as mock from "wowlua:mock"
config({ lifecycle = "manual" })

@test("logs in itself")
local function manual()
    expect(DemoDB).toBeNil()
    mock.login()
    expect(DemoDB.logins).toBe(1)
end
"#,
            ),
        ],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(
        out.contains("timed out after 10 s waiting for nothing"),
        "{out}"
    );
    assert!(out.contains("5 passed, 1 failed"), "{out}");
}

#[test]
fn errors_reaching_the_handler_fail_unless_read() {
    let dir = project(
        r#""standard": {}"#,
        &[(
            "tests/errors.test.wowlua",
            r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

local function failLater()
    C_Timer.After(1, function() error("late failure") end)
    mock.advanceTime(1)
end

@test("unread")
local function unread() failLater() end

@test("read")
local function read()
    failLater()
    expect(mock.errors()[1]).toContain("late failure")
end
"#,
        )],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("an error reached the error handler"), "{out}");
    assert!(out.contains("1 passed, 1 failed"), "{out}");
}

#[test]
fn test_modules_are_only_for_test_files() {
    let dir = project(
        r#""standard": {}"#,
        &[(
            "src/main.wowlua",
            "import expect from \"wowlua:test\"\nexpect(1).toBe(1)\n",
        )],
    );
    let out = wowlua(dir.path()).arg("check").output().unwrap();
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(text.contains("E8108"), "{text}");
}

#[test]
fn test_files_stay_out_of_builds() {
    let dir = project(
        r#""standard": {}"#,
        &[
            ("src/main.wowlua", "print(1)\n"),
            (
                "src/main.test.wowlua",
                "import test from \"wowlua:test\"\n@test\nlocal function t() end\n",
            ),
            ("tests/helper.wowlua", "export const x = 1\n"),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let out = dir.path().join("dist/dev/Demo");
    let toc = fs::read_to_string(out.join("Demo.toc")).unwrap();
    assert!(!toc.contains("test"), "{toc}");
    assert!(!out.join("src/main.test.lua").exists());
    assert!(!out.join("wowlua/std/test.lua").exists());
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("PASS src/main.test.wowlua"), "{out}");
}

#[test]
fn a_new_addon_passes_its_tests() {
    let dir = tempfile::tempdir().unwrap();
    wowlua(dir.path())
        .args(["new", "greeter", "--name", "Greeter"])
        .assert()
        .success();
    let (ok, out) = test(&dir.path().join("greeter"), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("✓ greets at login"), "{out}");
}

/// `test.blizzard-ui`: Blizzard's UI code (from `cargo xtask types fetch`,
/// skipped without it) loads before the addon; `print` goes through its
/// print handler.
#[test]
fn blizzard_ui_loads_before_the_addon() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/types-src");
    if !source.join("camelot/Interface").exists() {
        eprintln!("skipped: no Blizzard UI code (run `cargo xtask types fetch`)");
        return;
    }
    let dir = project(
        r#""camelot": {}"#,
        &[
            ("src/main.wowlua", "print(\"loaded\", 1)\n"),
            (
                "tests/ui.test.wowlua",
                r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("Blizzard's frames")
local function frames()
    expect(GameTooltip).never.toBeNil()
    local b = CreateFrame("Button", nil, UIParent, "UIPanelButtonTemplate")
    b:SetText("Hi")
    expect(b:GetText()).toBe("Hi")
    expect(mock.printed()).toContain("loaded 1")
    GameTooltip:SetOwner(UIParent, "ANCHOR_NONE")
    GameTooltip:AddLine("Hello")
    GameTooltip:AddDoubleLine("Left", "Right")
    GameTooltip:Show()
    expect(GameTooltip).toHaveLines({ "Hello", { "Left", "Right" } })
end
"#,
            ),
        ],
    );
    let manifest = dir.path().join("wowlua.json");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replacen('{', r#"{ "test": { "blizzard-ui": true },"#, 1),
    )
    .unwrap();
    let out = wowlua(dir.path())
        .arg("test")
        .env("WOWLUA_UI_SOURCE", &source)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.contains("1 passed, 0 failed"), "{text}");
    assert!(!text.contains("raised"), "{text}");
}

/// Typed matchers (decision T214) and cases (T215) are checked when the
/// tests compile.
#[test]
fn matchers_and_cases_are_type_checked() {
    let dir = project(
        r#""standard": {}"#,
        &[(
            "tests/typed.test.wowlua",
            r#"import test, expect, cases from "wowlua:test"
import * as mock from "wowlua:mock"

@test
local function wrong()
    local count = 3
    expect(count).toBe("3")
    expect(count).toHaveLength(3)
    local s = mock.spy(function(_a: number, _b: string) end)
    expect(s).toHaveBeenCalledWith("x", 1)
end

@test
@cases({ { 1, 2 }, { "a", 2 }, { 1 }, { 1, 2, 3 } })
local function sums(a: number, b: number)
    expect(a + b).never.toBe(0)
end
"#,
        )],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    for wanted in [
        "`\"3\"` is not assignable to `number`",
        "`toHaveLength` needs a string or a table; the value is `number`",
        "`\"x\"` is not assignable to `number`",
        "case 2: `\"a\"` is not assignable to `number` (parameter `a`)",
        "case 3 has no value for `b`",
        "case 4 has 3 values; the test takes 2",
    ] {
        assert!(out.contains(wanted), "missing {wanted:?} in:\n{out}");
    }
}

/// World profiles (decision T213): the character, gear, bags, money,
/// place, the units around, CVars, bindings and saved variables, loaded
/// before the addon; changes fire the client's events.
#[test]
fn world_profiles() {
    let profile = r#"{
  "character": { "name": "Thrall", "realm": "Realm", "class": "SHAMAN", "race": "Orc", "level": 60, "health": 3000, "maxHealth": 3000 },
  "money": 123456,
  "items": { "19019": { "name": "Thunderfury, Blessed Blade of the Windseeker", "quality": 5, "icon": 135349 }, "159": { "name": "Refreshing Spring Water", "quality": 1 } },
  "equipment": { "MainHandSlot": 19019 },
  "bags": { "0": { "slots": 16, "items": { "1": { "id": 159, "count": 5 } } } },
  "position": { "mapID": 1453, "zone": "Stormwind City", "subZone": "Trade District", "x": 0.5, "y": 0.6 },
  "units": [ { "name": "Hogger", "level": 11, "reaction": "hostile", "distance": 12, "target": true } ],
  "cvars": { "nameplateShowEnemies": "1" },
  "savedVariables": { "DemoDB": { "logins": 41 } }
}"#;
    let dir = project(
        r#""standard": {}"#,
        &[
            (
                "src/main.wowlua",
                r#"local f = CreateFrame("Frame")
f:RegisterEvent("PLAYER_LOGIN")
f:SetScript("OnEvent", function()
    DemoDB.logins = DemoDB.logins + 1
    print(UnitName("player"), GetMoney(), GetZoneText(), (UnitName("target")))
end)
"#,
            ),
            ("tests/profiles/stormwind.json", profile),
            (
                "tests/world.test.wowlua",
                r#"import test, expect, config from "wowlua:test"
import * as mock from "wowlua:mock"
config({ profile = "stormwind" })

@test("the profile is the world")
local function world()
    expect(DemoDB.logins).toBe(42)
    expect(mock.printed()).toContain("Thrall 123456 Stormwind City Hogger")
    expect(UnitClass("player")).toBe("Shaman")
    expect(GetInventoryItemID("player", 16)).toBe(19019)
    expect(GetInventoryItemLink("player", 16)).toContain("[Thunderfury, Blessed Blade of the Windseeker]")
    expect(C_Container.GetContainerItemInfo(0, 1).stackCount).toBe(5)
    expect(C_Container.GetContainerNumFreeSlots(0)).toBe(15)
    expect(GetSubZoneText()).toBe("Trade District")
    expect(C_Map.GetBestMapForUnit("player")).toBe(1453)
    expect(CheckInteractDistance("target", 2)).toBe(false)
    expect(CheckInteractDistance("target", 1)).toBe(true)
    expect(C_CVar.GetCVar("nameplateShowEnemies")).toBe("1")
end

@test("changes fire events")
local function changes()
    local seen: string[] = {}
    local f = CreateFrame("Frame")
    for _, e in ipairs({ "PLAYER_MONEY", "PLAYER_EQUIPMENT_CHANGED", "BAG_UPDATE", "ZONE_CHANGED", "ZONE_CHANGED_NEW_AREA" }) do
        f:RegisterEvent(e)
    end
    f:SetScript("OnEvent", function(_: Frame, e: string) seen[#seen + 1] = e end)
    mock.player:setMoney(10)
    mock.player:equip(16, nil)
    mock.setBag(0, { slots = 16, items = {} })
    mock.moveTo({ subZone = "Old Town" })
    mock.moveTo({ zone = "Elwynn Forest", mapID = 1429 })
    expect(seen).toEqual({ "PLAYER_MONEY", "PLAYER_EQUIPMENT_CHANGED", "BAG_UPDATE", "BAG_UPDATE", "ZONE_CHANGED", "ZONE_CHANGED_NEW_AREA" })
    expect(GetMoney()).toBe(10)
    expect(GetInventoryItemID("player", 16)).toBeNil()
end
"#,
            ),
        ],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("2 passed, 0 failed"), "{out}");
}

/// Widget matchers (decision T217) and custom matchers (T220), typed.
#[test]
fn widget_and_custom_matchers() {
    let dir = project(
        r#""standard": {}"#,
        &[
            (
                "tests/matchers.wowlua",
                r#"import matcher from "wowlua:test"

export const toBeEven = matcher("toBeEven", function(n: number, by: number?): (boolean, string)
    return n % (by ?? 2) == 0, `expected {n} to be even`
end)
"#,
            ),
            (
                "tests/ui.test.wowlua",
                r#"import test, expect from "wowlua:test"
import toBeEven from "./matchers"

@test("frames")
local function frames()
    local parent = CreateFrame("Frame", nil, UIParent)
    parent:SetSize(200, 100)
    parent:SetPoint("CENTER")
    local child = CreateFrame("Frame", nil, parent)
    child:SetPoint("TOPLEFT", parent, "TOPLEFT", 4, -4)
    local label = parent:CreateFontString(nil, "OVERLAY")
    label:SetFont("Fonts\\FRIZQT__.TTF", 12, "")
    label:SetText("Hello")
    expect(parent).toBeShown()
    expect(parent).toBeVisible()
    expect(parent).toHaveSize(200, 100)
    expect(parent).toHaveText("Hello")
    expect(label).toHaveText("Hello")
    expect(child).toBeAnchoredTo(parent, "TOPLEFT")
    expect(child).never.toBeAnchoredTo(UIParent)
    parent:Hide()
    expect(child).toBeShown()
    expect(child).never.toBeVisible()
end

@test("custom")
local function customMatcher()
    expect(4).toBeEven()
    expect(9).toBeEven(3)
    expect(3).never.toBeEven()
end

@test("a custom failure")
local function customFailure()
    expect(3).toBeEven()
end
"#,
            ),
        ],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("2 passed, 1 failed"), "{out}");
    assert!(out.contains("expected 3 to be even"), "{out}");
    // Typed: the value against the matcher's first parameter, the
    // arguments against the rest.
    fs::write(
        dir.path().join("tests/ui.test.wowlua"),
        r#"import test, expect from "wowlua:test"
import toBeEven from "./matchers"

@test
local function typed()
    expect("x").toBeEven()
    expect(4).toBeEven("two")
    expect(4).toBeShown()
end
"#,
    )
    .unwrap();
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(
        out.contains("`toBeEven` takes `number`; the value is `\"x\"`"),
        "{out}"
    );
    assert!(
        out.contains("`\"two\"` is not assignable to `number?`"),
        "{out}"
    );
    assert!(
        out.contains("`toBeShown` needs a widget; the value is `4`"),
        "{out}"
    );
    assert!(!out.contains("unused import"), "{out}");
}

/// Snapshots (decision T218): written when missing, compared after, shown
/// as a diff when they differ, rewritten with `-u`.
#[test]
fn snapshots() {
    let file = |kills: u32| {
        format!(
            r#"import test, expect from "wowlua:test"

@test("state")
local function state()
    expect({{ kills = {kills}, name = "x", list = {{ 1, 2 }} }}).toMatchSnapshot()
end

@test("frame")
local function frame()
    local f = CreateFrame("Frame", "SnapFrame", UIParent)
    f:SetSize(100, 50)
    f:SetPoint("TOPLEFT", 10, -20)
    local t = f:CreateFontString(nil, "OVERLAY")
    t:SetFont("Fonts\\FRIZQT__.TTF", 12, "")
    t:SetText("Hi")
    expect(f).toMatchSnapshot("layout")
end
"#
        )
    };
    let dir = project(r#""standard": {}"#, &[("tests/snap.test.wowlua", &file(1))]);
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("snapshots: 2 written"), "{out}");
    let snap =
        fs::read_to_string(dir.path().join("tests/__snapshots__/snap.test.wowlua.snap")).unwrap();
    for line in [
        "# state › 1",
        "| {",
        "|   kills = 1,",
        "|   name = \"x\",",
        "# frame › layout",
        "| Frame SnapFrame",
        "|   size 100 x 50",
        "|   TOPLEFT on UIParent TOPLEFT (10, -20)",
        "|   FontString <FontString>",
        "|     text \"Hi\"",
    ] {
        assert!(snap.contains(line), "missing {line:?} in:\n{snap}");
    }
    // The same again: they match.
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok && !out.contains("snapshots:"), "{out}");
    // A change fails with a diff; -u takes it.
    fs::write(dir.path().join("tests/snap.test.wowlua"), file(2)).unwrap();
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("- stored, + now"), "{out}");
    assert!(
        out.contains("-   kills = 1,") && out.contains("+   kills = 2,"),
        "{out}"
    );
    let (ok, out) = test(dir.path(), &["-u"]);
    assert!(ok, "{out}");
    assert!(out.contains("snapshots: 1 updated"), "{out}");
    // A snapshot no test matches goes with -u.
    fs::write(
        dir.path().join("tests/snap.test.wowlua"),
        file(2).replace("toMatchSnapshot(\"layout\")", "toBeShown()"),
    )
    .unwrap();
    let (ok, out) = test(dir.path(), &["-u"]);
    assert!(ok, "{out}");
    assert!(out.contains("snapshots: 1 removed"), "{out}");
}

/// Compile-time tests (decision T219): `@expectError` passes when the
/// function gives exactly these diagnostics, which are dropped; otherwise
/// E8122.
#[test]
fn compile_time_tests() {
    let body = |codes: &str| {
        format!(
            r#"import test, expect, expectError from "wowlua:test"

local function record(n: number): number
    return n
end

@expectError({codes})
local function wrongArgument()
    record("two")
end

@test
local function runs()
    expect(record(2)).toBe(2)
end
"#
        )
    };
    let dir = project(
        r#""standard": {}"#,
        &[("tests/types.test.wowlua", &body("\"E2001\""))],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("✓ wrongArgument"), "{out}");
    assert!(out.contains("2 passed, 0 failed"), "{out}");
    fs::write(
        dir.path().join("tests/types.test.wowlua"),
        body("\"E2005\""),
    )
    .unwrap();
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(
        out.contains("`wrongArgument` should not compile with E2005; it gives E2001"),
        "{out}"
    );
}

/// `import internal` (decision T223): a module's file-level locals and
/// types in test files only (E8119 elsewhere); the addon build is
/// unchanged.
#[test]
fn import_internal() {
    let tracker = r#"
type Entry = { kills: number }

local function parse(text: string): number
    return tonumber(text) or 0
end

const LIMIT = 10

local cache: { [string]: Entry } = {}

export function record(name: string, text: string)
    cache[name] = { kills = math.min(parse(text), LIMIT) }
end
"#;
    let dir = project(
        r#""standard": {}"#,
        &[
            ("src/tracker.wowlua", tracker),
            (
                "tests/tracker.test.wowlua",
                r#"import test, expect from "wowlua:test"
import record from "/tracker"
import internal parse, LIMIT, cache, Entry from "/tracker"

@test
local function internals()
    expect(parse("12")).toBe(12)
    expect(LIMIT).toBe(10)
    record("hogger", "99")
    local e: Entry = cache["hogger"]
    expect(e.kills).toBe(10)
end
"#,
            ),
        ],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("1 passed, 0 failed"), "{out}");
    // Typed: the local's type.
    fs::write(
        dir.path().join("tests/tracker.test.wowlua"),
        "import internal parse from \"/tracker\"\nlocal s: string = parse(\"1\")\nprint(s)\n",
    )
    .unwrap();
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(
        out.contains("`number` is not assignable to `string`"),
        "{out}"
    );
    // Not in the addon's own files.
    fs::write(
        dir.path().join("tests/tracker.test.wowlua"),
        "import test from \"wowlua:test\"\n@test\nlocal function t() end\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/main.wowlua"),
        "import internal parse from \"./tracker\"\nprint(parse(\"1\"))\n",
    )
    .unwrap();
    let out = wowlua(dir.path()).arg("check").output().unwrap();
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("E8119"), "{text}");
    // The addon build has no internals.
    fs::remove_file(dir.path().join("src/main.wowlua")).unwrap();
    wowlua(dir.path()).arg("build").assert().success();
    let built = fs::read_to_string(dir.path().join("dist/dev/Demo/src/tracker.lua")).unwrap();
    assert!(!built.contains("#internal"), "{built}");
}

/// `mock.module` (decision T224): the module's exports replaced for every
/// importer, the rest kept; replacements checked against the exports.
#[test]
fn mock_module() {
    let dir = project(
        r#""standard": {}"#,
        &[
            (
                "src/clock.wowlua",
                "export function now(): number\n    return 1\nend\nexport function zone(): string\n    return \"here\"\nend\n",
            ),
            (
                "src/report.wowlua",
                "import now, zone from \"./clock\"\nexport function line(): string\n    return zone() .. \" at \" .. now()\nend\n",
            ),
            (
                "tests/report.test.wowlua",
                r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"
import * as clock from "/clock"
import line from "/report"

@test("the real one")
local function real()
    expect(line()).toBe("here at 1")
end

@test("replaced")
local function replaced()
    mock.module(clock, { now = function(): number return 42 end })
    expect(line()).toBe("here at 42")
end
"#,
            ),
        ],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("2 passed, 0 failed"), "{out}");
    fs::write(
        dir.path().join("tests/report.test.wowlua"),
        r#"import * as mock from "wowlua:mock"
import * as clock from "/clock"
mock.module(clock, { now = function(): string return "x" end, later = 1 })
"#,
    )
    .unwrap();
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("the module has no export `later`"), "{out}");
    assert!(out.contains("is not assignable to `() -> number`"), "{out}");
}

/// `mock.reloadUI()`: the test runs again in a fresh VM with the
/// SavedVariables as the client writes them (`/stdtest sv`), logged in as
/// a reload; the file's next test runs where the file was.
#[test]
fn reload_ui() {
    let dir = project(
        r#""camelot": {}"#,
        &[
            ("src/main.wowlua", "_G.Loaded = (_G.Loaded or 0) + 1\n"),
            (
                "tests/reload.test.wowlua",
                r#"import test, expect, afterEach from "wowlua:test"
import * as mock from "wowlua:mock"

local entering: any[] = {}
local f = CreateFrame("Frame")
f:RegisterEvent("PLAYER_ENTERING_WORLD")
f:SetScript("OnEvent", function(_, _, initial: boolean, reloading: boolean)
    entering = { initial, reloading }
end)
local teardowns = 0

@afterEach
local function count()
    teardowns = teardowns + 1
end

@test("the SavedVariables survive a reload")
local function survives()
    const db = _G :: any
    if mock.reloads() == 0 then
        expect(db.DemoDB).toBeNil()
        expect(entering).toEqual({ true, false })
        local shared = { tag = "shared" }
        db.DemoDB = {
            count = 1,
            third = 1 / 3,
            seventeen = 0.12345678901234567,
            fn = print,
            frame = CreateFrame("Frame"),
            nested = { deep = true },
            keys = { [1.5] = "float", [true] = "true", [-1] = "negative" },
            a = shared,
            b = shared,
            time = GetTime(),
        }
        mock.reloadUI()
        error("not reached")
    end
    expect(mock.reloads()).toBe(1)
    expect(db.Loaded).toBe(1)
    expect(entering).toEqual({ false, true })
    expect(db.DemoDB.count).toBe(1)
    expect(db.DemoDB.third).toBe(1 / 3)
    expect(db.DemoDB.seventeen).toBe(0.1234567890123457)
    expect(db.DemoDB.fn).toBeNil()
    expect(db.DemoDB.frame[0]).toBeNil()
    expect(db.DemoDB.nested.deep).toBe(true)
    expect(db.DemoDB.keys).toEqual({ [1.5] = "float", [true] = "true", [-1] = "negative" })
    expect(db.DemoDB.a == db.DemoDB.b).toBe(false)
    expect(db.DemoDB.a.tag).toBe("shared")
    expect(GetTime() >= db.DemoDB.time).toBe(true)
end

@test("twice")
local function twice()
    const db = _G :: any
    db.DemoDB.passes = db.DemoDB.passes ?? {}
    table.insert(db.DemoDB.passes, mock.reloads())
    if mock.reloads() < 2 then
        mock.reloadUI()
    end
    expect(db.DemoDB.passes).toEqual({ 0, 1, 2 })
end

@test("the next test runs where the file was")
local function after()
    expect(mock.reloads()).toBe(0)
    expect((_G :: any).DemoDB.count).toBe(1)
    -- Each test's first pass ran its teardown in this VM.
    expect(teardowns).toBe(2)
end
"#,
            ),
        ],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("3 passed, 0 failed"), "{out}");
}

/// GUI tests act as the user: hover, click, type and keys.
#[test]
fn gui_input() {
    let dir = project(
        r#""standard": {}"#,
        &[(
            "tests/gui.test.wowlua",
            r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("hover and click")
local function hoverClick()
    local seen: string[] = {}
    local b = CreateFrame("Button", "GuiButton", UIParent)
    b:SetSize(80, 20)
    b:SetPoint("CENTER")
    for _, s in ipairs({ "OnEnter", "OnLeave", "OnMouseDown", "OnMouseUp", "PreClick", "OnClick", "PostClick" }) do
        b:SetScript(s, function() seen[#seen + 1] = s end)
    end
    mock.hover(b)
    expect(b:IsMouseOver()).toBe(true)
    expect(GetMouseFoci()[1]).toBe(b)
    mock.click(b)
    mock.hover(nil)
    expect(seen).toEqual({ "OnEnter", "OnMouseDown", "OnMouseUp", "PreClick", "OnClick", "PostClick", "OnLeave" })
    b:Disable()
    seen = {}
    mock.click(b)
    expect(seen).toEqual({ "OnMouseDown", "OnMouseUp" })
    b:Hide()
    expect(function() mock.click(b) end).toThrow("GuiButton is not visible")
end

@test("type and press enter")
local function typing()
    local box = CreateFrame("EditBox", nil, UIParent)
    local changes = 0
    local entered: string? = nil
    box:SetScript("OnTextChanged", function(_: EditBox, user: boolean) if user then changes = changes + 1 end end)
    box:SetScript("OnEnterPressed", function(self: EditBox) entered = self:GetText() end)
    mock.typeText(box, "hello")
    expect(box:HasFocus()).toBe(true)
    expect(changes).toBe(5)
    mock.pressKey("ENTER")
    expect(entered).toBe("hello")
end
"#,
        )],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("2 passed, 0 failed"), "{out}");
}

/// Coverage (decision T188): counted by the VM in the addon's own files,
/// with reports and thresholds.
#[test]
fn coverage_reports_and_thresholds() {
    let dir = project(
        r#""standard": {}"#,
        &[
            (
                "src/math.wowlua",
                r#"export function sign(n: number): number
    if n > 0 then
        return 1
    elseif n < 0 then
        return -1
    end
    return 0
end

export function unused(): number
    return 42
end

-- wowlua: coverage ignore
local function debugOnly() print("never") end
"#,
            ),
            (
                "tests/math.test.wowlua",
                r#"import test, expect from "wowlua:test"
import sign from "/math"

@test
local function positive()
    expect(sign(5)).toBe(1)
end
"#,
            ),
        ],
    );
    let (ok, out) = test(
        dir.path(),
        &[
            "--coverage",
            "--reporter",
            "pretty,lcov,cobertura,coverage-json,html,junit,tap,json",
        ],
    );
    assert!(ok, "{out}");
    assert!(out.contains("coverage: lines"), "{out}");
    assert!(out.contains("src/math.wowlua: lines"), "{out}");
    let report = dir.path().join("dist/test");
    let lcov = fs::read_to_string(report.join("lcov.info")).unwrap();
    for line in [
        "SF:src/math.wowlua",
        "FN:1,sign#0",
        "FNDA:1,sign#0",
        "FNDA:0,unused#0",
        "DA:11,0",
        "BRDA:2,0,",
    ] {
        assert!(lcov.contains(line), "missing {line:?} in:\n{lcov}");
    }
    // The ignored function is not counted.
    assert!(!lcov.contains("debugOnly"), "{lcov}");
    assert!(!lcov.contains("SF:tests/"), "{lcov}");
    let html = fs::read_to_string(report.join("coverage.html")).unwrap();
    assert!(html.contains("<title>Demo coverage</title>") && html.contains("prefers-color-scheme"));
    assert!(
        fs::read_to_string(report.join("cobertura.xml"))
            .unwrap()
            .contains("filename=\"src/math.wowlua\"")
    );
    assert!(
        fs::read_to_string(report.join("coverage.json"))
            .unwrap()
            .contains("\"functions\"")
    );
    assert!(
        fs::read_to_string(report.join("junit.xml"))
            .unwrap()
            .contains("<testcase name=\"positive\"")
    );
    assert!(
        fs::read_to_string(report.join("results.tap"))
            .unwrap()
            .contains("ok 1 - tests/math.test.wowlua (standard) › positive")
    );
    assert!(
        fs::read_to_string(report.join("results.json"))
            .unwrap()
            .contains("\"status\": \"passed\"")
    );
    // A minimum the run does not reach fails it.
    let manifest = dir.path().join("wowlua.json");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replacen('{', r#"{ "test": { "coverage": { "functions": 90 } },"#, 1),
    )
    .unwrap();
    let (ok, out) = test(dir.path(), &["--coverage"]);
    assert!(!ok, "{out}");
    assert!(
        out.contains("functions coverage 50.0% is below the minimum 90%"),
        "{out}"
    );
}

/// GitHub annotations for failures.
#[test]
fn github_reporter() {
    let dir = project(
        r#""standard": {}"#,
        &[(
            "tests/a.test.wowlua",
            "import test, expect from \"wowlua:test\"\n@test(\"bad\")\nlocal function bad()\n    expect(1).toBe(2)\nend\n",
        )],
    );
    let (ok, out) = test(dir.path(), &["--reporter", "github"]);
    assert!(!ok, "{out}");
    assert!(
        out.contains(
            "::error file=tests/a.test.wowlua,line=4,title=bad (standard)::expected 1 to be 2"
        ),
        "{out}"
    );
}

/// Taint in tests (spec Tooling §16.1.4): a protected action refused
/// because of taint fails the test with its taint chain unless the test
/// expects it (`toBeBlocked`); `toBeSecure` tells untainted variables.
#[test]
fn taint_in_tests() {
    let dir = project(
        r#""standard": {}"#,
        &[(
            "tests/taint.test.wowlua",
            r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

local function protectedFrame(name: string): Frame
    local f = CreateFrame("Button", name, UIParent, "SecureActionButtonTemplate")
    mock.player:enterCombat()
    return f
end

@test("expected")
local function expected()
    local f = protectedFrame("ProtA")
    expect(function() f:Hide() end).toBeBlocked()
    mock.player:leaveCombat()
end

@test("unexpected")
local function unexpected()
    TargetUnit("player")
end

@test("variables")
local function variables()
    expect("UIParent").toBeSecure()
    _G.MyAddonGlobal = 1
    expect("MyAddonGlobal").never.toBeSecure()
end
"#,
        )],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("✓ expected"), "{out}");
    assert!(out.contains("✓ variables"), "{out}");
    assert!(out.contains("✗ unexpected"), "{out}");
    assert!(out.contains("because of taint"), "{out}");
    assert!(out.contains("2 passed, 1 failed"), "{out}");
}

/// Casting, modifier keys, load-on-demand addons and libraries.
#[test]
fn casting_modifiers_addons_libraries() {
    let dir = project(
        r#""standard": {}"#,
        &[(
            "tests/world.test.wowlua",
            r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("a cast runs and ends")
local function cast()
    mock.defineSpell(116, { name = "Frostbolt", iconID = 135846, castTime = 2000 })
    local seen: string[] = {}
    local f = CreateFrame("Frame")
    for _, e in ipairs({ "UNIT_SPELLCAST_START", "UNIT_SPELLCAST_SUCCEEDED", "UNIT_SPELLCAST_STOP", "UNIT_SPELLCAST_INTERRUPTED" }) do
        f:RegisterUnitEvent(e, "player")
    end
    f:SetScript("OnEvent", function(_: Frame, e: string) seen[#seen + 1] = e end)
    mock.player:castSpell(116)
    local name, _, icon, startMS, endMS = UnitCastingInfo("player")
    expect(name).toBe("Frostbolt")
    expect(icon).toBe(135846)
    expect(endMS - startMS).toBe(2000)
    mock.advanceTime(2)
    expect(UnitCastingInfo("player")).toBeNil()
    mock.player:castSpell(116)
    mock.player:interruptCast()
    expect(seen).toEqual({
        "UNIT_SPELLCAST_START", "UNIT_SPELLCAST_SUCCEEDED", "UNIT_SPELLCAST_STOP",
        "UNIT_SPELLCAST_START", "UNIT_SPELLCAST_INTERRUPTED", "UNIT_SPELLCAST_STOP",
    })
end

@test("modifiers")
local function modifiers()
    expect(IsShiftKeyDown()).toBe(false)
    mock.setModifiers({ shift = true })
    expect(IsShiftKeyDown()).toBe(true)
    expect(IsModifierKeyDown()).toBe(true)
    mock.setModifiers({})
    expect(IsModifierKeyDown()).toBe(false)
end

@test("addons and libraries")
local function addonsAndLibraries()
    local loaded = false
    mock.defineAddon("Extra", { loadOnDemand = true, load = function() loaded = true end })
    mock.loadAddon("Extra")
    expect(loaded).toBe(true)
    mock.provideLibrary("LibThing-1.0", { answer = 42 })
    local lib: any = LibStub("LibThing-1.0")
    expect(lib.answer).toBe(42)
end
"#,
        )],
    );
    let (ok, out) = test(dir.path(), &[]);
    assert!(ok, "{out}");
    assert!(out.contains("3 passed, 0 failed"), "{out}");
}

/// The examples' tests: the addons with Blizzard's UI (skipped without
/// the UI code of `cargo xtask types fetch`).
#[test]
fn example_tests() {
    example("core-demo", "8 passed, 0 failed");
    example("widget-demo", "3 passed, 0 failed");
}

/// The typed-demo example's tests: modules, world profiles, compile-time
/// tests, every flavor of a shared-mode package (no Blizzard UI needed).
#[test]
fn typed_demo_example() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/typed-demo");
    let (ok, out) = test(&root, &[]);
    assert!(ok, "{out}");
    assert!(out.contains("36 passed, 0 failed"), "{out}");
}

fn example(name: &str, summary: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join("target/types-src");
    if !source.join("camelot/Interface").exists() {
        eprintln!("skipped: no Blizzard UI code (run `cargo xtask types fetch`)");
        return;
    }
    let out = wowlua(&root.join("examples").join(name))
        .args(["test", "--flavor", "camelot"])
        .env("WOWLUA_UI_SOURCE", &source)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{name}: {text}");
    assert!(text.contains(summary), "{name}: {text}");
}

/// Blizzard's UI through the world's changes, without an error (it runs
/// without one in the client): casts, money, gear, bags, auras in and out
/// of combat (their secret updates), a target's death.
#[test]
fn blizzard_ui_through_world_changes() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/types-src");
    if !source.join("camelot/Interface").exists() {
        eprintln!("skipped: no Blizzard UI code (run `cargo xtask types fetch`)");
        return;
    }
    let dir = project(
        r#""camelot": {}"#,
        &[
            ("src/main.wowlua", "print(\"loaded\")\n"),
            (
                "tests/world.test.wowlua",
                r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("the world changes")
local function world()
    mock.nextFrame(60)
    local p = mock.player :: any
    p:castSpell(647, { castTime = 1 })
    p:castSpell(647, { castTime = 1 })
    mock.advanceTime(2)
    p:castSpell(19750, { castTime = 0 })
    p:castSpell(5143, { castTime = 1, channel = true })
    p:interruptCast()
    p:setMoney(40000)
    p:equip(16, { id = 6948 })
    p:equip(16, nil)
    mock.setBag(0, { slots = 16, items = { [1] = { id = 6948 } } })
    mock.setBag(2, { slots = 6, items = {} })
    mock.setBag(2, nil)
    local id = p:applyAura({ spellId = 20217, name = "Blessing of Kings", duration = 300 })
    mock.nextFrame(5)
    p:removeAura(id)
    local hogger = mock.spawnUnit({ name = "Hogger", level = 11, reaction = "hostile", health = 666 })
    p:target(hogger)
    p:enterCombat()
    mock.nextFrame(30)
    local id2 = p:applyAura({ spellId = 20217, name = "Blessing of Kings", duration = 300 })
    hogger:applyAura({ spellId = 772, name = "Rend", duration = 15, harmful = true, source = "player" })
    mock.nextFrame(5)
    p:removeAura(id2)
    hogger:castSpell(133, { castTime = 1 })
    mock.advanceTime(0.5)
    hogger:kill({ by = mock.player, lootable = true })
    mock.nextFrame(5)
    p:leaveCombat()
    mock.nextFrame(30)
    expect(UnitExists("target")).toBe(false)
    -- Soft targets.
    local wolf = mock.spawnUnit({ name = "Swoop", level = 4, reaction = "hostile", health = 50 })
    local friend = mock.spawnUnit({ name = "Harn Longcast", level = 30, reaction = "friendly" })
    p:softTarget(wolf)
    p:softTarget(friend)
    p:softTarget(friend, "interact")
    mock.nextFrame(10)
    p:softTarget(nil, "enemy")
    p:softTarget(nil, "friend")
    p:softTarget(nil, "interact")
    -- A vendor: the merchant frame opens, buys, sells and buys back.
    mock.defineItem(6256, { name = "Fishing Pole", icon = 132932, quality = 1 })
    mock.defineItem(6529, { name = "Shiny Bauble", icon = 134335, quality = 1, sellPrice = 12, stackCount = 20 })
    mock.openMerchant({ name = "Harn Longcast", items = { { id = 6256, price = 23 }, { id = 6529, price = 50 } } })
    mock.nextFrame(10)
    expect(MerchantFrame).toBeShown()
    BuyMerchantItem(2, 1)
    mock.waitSeconds(0.5)
    mock.sell(0, 2)
    mock.waitSeconds(0.5)
    BuybackItem(1)
    mock.waitSeconds(0.5)
    mock.closeMerchant()
    mock.nextFrame(10)
    expect(MerchantFrame).never.toBeShown()
end
"#,
            ),
        ],
    );
    let manifest = dir.path().join("wowlua.json");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replacen('{', r#"{ "test": { "blizzard-ui": true },"#, 1),
    )
    .unwrap();
    let out = wowlua(dir.path())
        .arg("test")
        .env("WOWLUA_UI_SOURCE", &source)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.contains("1 passed, 0 failed"), "{text}");
}

/// Every panel's key binding with Blizzard's UI, opened and closed, as the
/// fresh character (the default) and as the recorded one (its profile):
/// not a single Blizzard error, nothing refused.
#[test]
fn blizzard_ui_every_panel_binding() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/types-src");
    if !source.join("camelot/Interface").exists() {
        eprintln!("skipped: no Blizzard UI code (run `cargo xtask types fetch`)");
        return;
    }
    let panels = |profile: &str| {
        format!(
            r#"import test, expect, config from "wowlua:test"
import * as mock from "wowlua:mock"
{profile}
const BINDINGS = {{
    "TOGGLEGAMEMENU", "TOGGLEBACKPACK", "TOGGLEBAG1", "TOGGLEBAG2", "TOGGLEBAG3", "TOGGLEBAG4", "OPENALLBAGS",
    "TOGGLECHARACTER0", "TOGGLECHARACTER2", "TOGGLECURRENCY", "TOGGLECHARACTER4", "TOGGLESPELLBOOK",
    "TOGGLEPROFESSIONBOOK", "TOGGLEPETBOOK", "TOGGLETALENTS", "TOGGLECOOLDOWNVIEWERSETTINGS", "TOGGLEQUESTLOG",
    "TOGGLEWORLDMAP", "TOGGLEWORLDMAPSIZE", "TOGGLEMINIMAP", "TOGGLEBATTLEFIELDMINIMAP", "TOGGLEWORLDSTATESCORES",
    "TOGGLEGUILDTAB", "TOGGLESOCIAL", "TOGGLEFRIENDSTAB", "TOGGLEWHOTAB", "TOGGLECHATTAB", "TOGGLERAIDTAB",
    "TOGGLECOLLECTIONS", "TOGGLECOLLECTIONSMOUNTJOURNAL", "TOGGLECOLLECTIONSPETJOURNAL", "TOGGLECOLLECTIONSTOYBOX",
    "TOGGLECOLLECTIONSWARDROBE", "TOGGLEUI", "TOGGLEFPS",
}}

@test("every panel, opened and closed")
local function panels()
    mock.nextFrame(60)
    for _, name in ipairs(BINDINGS) do
        mock.pressBinding(name)
        mock.nextFrame(5)
        mock.pressBinding(name)
        mock.nextFrame(3)
    end
    expect(#mock.refusals()).toBe(0)
end
"#
        )
    };
    let recorded = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../wowlua-engine/tests/client/forever-camelot-character.json"),
    )
    .unwrap();
    let fresh = panels("");
    let as_recorded = panels(r#"config({ profile = "recorded" })"#);
    let dir = project(
        r#""camelot": {}"#,
        &[
            ("src/main.wowlua", "print(\"loaded\")\n"),
            ("tests/fresh.test.wowlua", &fresh),
            ("tests/recorded.test.wowlua", &as_recorded),
            ("tests/profiles/recorded.json", &recorded),
        ],
    );
    let manifest = dir.path().join("wowlua.json");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replacen('{', r#"{ "test": { "blizzard-ui": true },"#, 1),
    )
    .unwrap();
    let out = wowlua(dir.path())
        .arg("test")
        .env("WOWLUA_UI_SOURCE", &source)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.contains("2 passed, 0 failed"), "{text}");
}

/// Every button in every panel with Blizzard's UI, hovered and clicked
/// (each tab's content too), scroll areas wheeled, as the fresh and the
/// recorded character: not a single Blizzard error, nothing refused.
#[test]
fn blizzard_ui_every_button_in_every_panel() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/types-src");
    if !source.join("camelot/Interface").exists() {
        eprintln!("skipped: no Blizzard UI code (run `cargo xtask types fetch`)");
        return;
    }
    let clicks = |profile: &str| {
        format!(
            r#"import test, expect, config from "wowlua:test"
import * as mock from "wowlua:mock"
{profile}
const BINDINGS = {{
    "TOGGLEGAMEMENU", "TOGGLEBACKPACK", "OPENALLBAGS",
    "TOGGLECHARACTER0", "TOGGLECHARACTER2", "TOGGLECURRENCY", "TOGGLECHARACTER4", "TOGGLESPELLBOOK",
    "TOGGLEPROFESSIONBOOK", "TOGGLEPETBOOK", "TOGGLETALENTS", "TOGGLECOOLDOWNVIEWERSETTINGS", "TOGGLEQUESTLOG",
    "TOGGLEWORLDMAP", "TOGGLEBATTLEFIELDMINIMAP", "TOGGLEWORLDSTATESCORES",
    "TOGGLEGUILDTAB", "TOGGLESOCIAL", "TOGGLEWHOTAB", "TOGGLECHATTAB", "TOGGLERAIDTAB",
    "TOGGLECOLLECTIONS", "TOGGLECOLLECTIONSMOUNTJOURNAL", "TOGGLECOLLECTIONSPETJOURNAL", "TOGGLECOLLECTIONSTOYBOX",
    "TOGGLECOLLECTIONSWARDROBE",
}}

const G = _G :: any

--- The visible top-level frames under UIParent.
local function shown(): {{ [any]: boolean }}
    const out: {{ [any]: boolean }} = {{}}
    for _, f in ipairs({{ G.UIParent:GetChildren() }}) do
        if not f:IsForbidden() and f:IsVisible() then
            out[f] = true
        end
    end
    return out
end

local function descendants(f: any, out: any[])
    for _, c in ipairs({{ f:GetChildren() }}) do
        if not c:IsForbidden() then
            out[#out + 1] = c
            descendants(c, out)
        end
    end
end

local function closeNew(before: {{ [any]: boolean }}, keep: any[])
    for f in pairs(shown()) do
        local kept = false
        for _, r in ipairs(keep) do
            kept = kept or r == f
        end
        if not before[f] and not kept and f:IsVisible() then
            G.HideUIPanel(f)
            if f:IsVisible() then
                f:Hide()
            end
        end
    end
end

@test("every button in every panel, hovered and clicked")
local function clicks()
    mock.nextFrame(60)
    for _, binding in ipairs(BINDINGS) do
        const before = shown()
        mock.pressBinding(binding)
        mock.nextFrame(5)
        const roots: any[] = {{}}
        for f in pairs(shown()) do
            if not before[f] then
                roots[#roots + 1] = f
            end
        end
        -- Each visible, enabled button not yet clicked, looking again after
        -- every click (a tab shows its content); scroll areas wheeled.
        const done: {{ [any]: boolean }} = {{}}
        local clicked = 0
        while clicked < 400 do
            const frames: any[] = {{}}
            for _, r in ipairs(roots) do
                if r:IsVisible() then
                    descendants(r, frames)
                end
            end
            local next: any = nil
            for _, f in ipairs(frames) do
                if not done[f] and f:IsVisible() then
                    if f:IsObjectType("Button") and f:IsEnabled() then
                        const name = f:GetName() ?? ""
                        if name:find("Logout") or name:find("Quit") or name:find("ExitGame") then
                            done[f] = true
                        else
                            next = f
                            break
                        end
                    elseif f:IsObjectType("ScrollFrame") or f.GetScrollPercentage ~= nil then
                        done[f] = true
                        mock.wheel(f, -1)
                        mock.wheel(f, 1)
                        mock.nextFrame(1)
                    end
                end
            end
            if next == nil then
                break
            end
            done[next] = true
            mock.hover(next)
            mock.click(next)
            mock.nextFrame(2)
            mock.hover(nil)
            clicked += 1
            local open = false
            for _, r in ipairs(roots) do
                open = open or r:IsVisible()
            end
            if not open then
                mock.pressBinding(binding)
                mock.nextFrame(3)
            end
            closeNew(before, roots)
        end
        mock.pressBinding(binding)
        mock.nextFrame(2)
        closeNew(before, {{}})
        mock.nextFrame(3)
    end
    expect(#mock.refusals()).toBe(0)
end
"#
        )
    };
    let recorded = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../wowlua-engine/tests/client/forever-camelot-character.json"),
    )
    .unwrap();
    let fresh = clicks("");
    let as_recorded = clicks(r#"config({ profile = "recorded" })"#);
    let dir = project(
        r#""camelot": {}"#,
        &[
            ("src/main.wowlua", "print(\"loaded\")\n"),
            ("tests/fresh.test.wowlua", &fresh),
            ("tests/recorded.test.wowlua", &as_recorded),
            ("tests/profiles/recorded.json", &recorded),
        ],
    );
    let manifest = dir.path().join("wowlua.json");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replacen('{', r#"{ "test": { "blizzard-ui": true },"#, 1),
    )
    .unwrap();
    let out = wowlua(dir.path())
        .arg("test")
        .env("WOWLUA_UI_SOURCE", &source)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.contains("2 passed, 0 failed"), "{text}");
}

/// Tooltips with Blizzard's UI: the engine's tooltip data from the tests'
/// game data, Blizzard's tooltip code, and an addon's post-call.
#[test]
fn tooltips_with_blizzard_ui() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/types-src");
    if !source.join("camelot/Interface").exists() {
        eprintln!("skipped: no Blizzard UI code (run `cargo xtask types fetch`)");
        return;
    }
    let dir = project(
        r#""camelot": {}"#,
        &[
            (
                "src/main.wowlua",
                r#"TooltipDataProcessor.AddTooltipPostCall(Enum.TooltipDataType.Item, function(tip: any, data: any)
    tip:AddLine("Demo: item " .. data.id)
end)
"#,
            ),
            (
                "tests/tip.test.wowlua",
                r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("an item tooltip with the addon's line")
local function item()
    mock.defineItem(1, { name = "Not loaded" })
    expect(mock.showTooltip("item", { id = 1 })[1]).toContain("Retrieving item information")
    mock.defineItem(19019, { name = "Thunderfury, Blessed Blade of the Windseeker", quality = 5, itemLevel = 80 }, { cached = true })
    local lines = mock.showTooltip("item", { id = 19019 })
    expect(lines).toEqual({ "Thunderfury, Blessed Blade of the Windseeker", "Item Level 80", "Demo: item 19019" })
    expect(GameTooltip).toBeShown()
end

@test("a unit tooltip")
local function unit()
    local hogger = mock.spawnUnit({ name = "Hogger", level = 11, reaction = "hostile" })
    mock.player:target(hogger)
    -- The beta client's issue reporter (Blizzard_PTRFeedback, loaded on
    -- beta and PTR clients only) adds its line to unit tooltips.
    expect(mock.showTooltip("unit", { unit = "target" })).toEqual({
        "Hogger",
        "Level 11",
        " ",
        "|c0042b1feThe 'Open Issue Report' keybind is not bound.\nBind via the PTR section in the Key Bindings Menu.",
    })
end
"#,
            ),
        ],
    );
    let manifest = dir.path().join("wowlua.json");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replacen('{', r#"{ "test": { "blizzard-ui": true },"#, 1),
    )
    .unwrap();
    let out = wowlua(dir.path())
        .arg("test")
        .env("WOWLUA_UI_SOURCE", &source)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.contains("2 passed, 0 failed"), "{text}");
}
