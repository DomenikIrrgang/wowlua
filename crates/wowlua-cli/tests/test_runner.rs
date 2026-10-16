//! `wowlua test` (spec Tooling §16.1): test files of a package run in the
//! emulated client, per flavor.

use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn wowlua(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", dir.join(".wowlua-home"))
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
    expect(seen).toEqual({ "PLAYER_MONEY", "PLAYER_EQUIPMENT_CHANGED", "BAG_UPDATE", "ZONE_CHANGED", "ZONE_CHANGED_NEW_AREA" })
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
