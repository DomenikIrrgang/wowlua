//! The in-game dev tools (spec Debugging §7.3.1–7.3.3, §7.7, §7.10) in the
//! emulated client: a package whose `test` profile builds them in, tested
//! with `wowlua test` (the engine's frames, events, combat, encounters,
//! clock and reloads).

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

/// The addon `Probe` built with every dev tool, `profile` merged into its
/// test profile.
const MAIN: &str = r#"
--- What the tests make the addon's work do (spend time, raise errors).
export const hooks = { work = function() end }
export function heavy(work: () -> ())
    work()
end
export function alloc(n: number): number
    local t = {}
    for i = 1, n do
        t[i] = { i }
    end
    return #t
end
export function handler()
    heavy(hooks.work)
    alloc(200)
end
local f = CreateFrame("Frame")
f:RegisterEvent("BAG_UPDATE")
f:SetScript("OnEvent", function()
    handler()
end)
_G.SLASH_PROBE1 = "/probe"
SlashCmdList.PROBE = function()
    handler()
end
-- A frame that only runs code: no size, nothing in it.
export const ticker = CreateFrame("Frame")
export function poll() end
"#;

fn run(profile: &str, test: &str) {
    package(profile, test);
}

/// The package of `run`, after its tests passed.
fn package(profile: &str, test: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = format!(
        r#"{{ "package": {{ "name": "probe", "version": "1.0.0" }}, "flavors": {{ "camelot": {{}} }},
  "check": {{ "lints": {{ "W1003": "allow" }} }},
  "profiles": {{ "test": {{ "dev-console": true, "monitor": true, "capture-errors": true,
    "instrument": "all", "runtime-checks": "none" {profile} }} }} }}"#
    );
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::create_dir_all(dir.path().join("tests")).unwrap();
    fs::write(dir.path().join("src/main.wowlua"), MAIN).unwrap();
    fs::write(
        dir.path().join("tests/devtools.test.wowlua"),
        format!(
            "import test, expect from \"wowlua:test\"\nimport * as mock from \"wowlua:mock\"\n\
             import hooks, ticker, poll, heavy from \"/main\"\n\
             local dev: any = _G.WOWLUA_DEV\nlocal function probe(): any return dev.addons.Probe end\n{test}"
        ),
    )
    .unwrap();
    let out = wowlua(dir.path()).arg("test").output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    dir
}

/// Frame time, slow calls with their root and stack, call paths and garbage
/// per function (§7.3.1–7.3.3), from the profile's settings (§9.9).
#[test]
fn frame_time_slow_calls_paths_and_garbage() {
    run(
        r#", "profile-call-paths": true, "profile-garbage": true, "frame-budget": 1, "slow-call": 3"#,
        r#"
-- (The test itself is not profiled: work it starts would have the root
-- `other`.)
@test
@noprofile
local function measures()
    local perf = probe().runtime.perf
    expect(perf.callPaths).toBe(true)
    expect(perf.garbage).toBe(true)
    expect(perf.slowCall).toBe(3)
    hooks.work = function() mock.spend(5) end
    local rec = perf.start("manual")
    mock.fireEvent("BAG_UPDATE", 0)
    mock.nextFrame()
    perf.stop(rec)
    -- Only the deepest call over 3 ms is a slow call, with its root; its
    -- callers (handler, the event) are in its stack.
    local slow = perf.slow[#perf.slow]
    expect(#rec.data.slow).toBe(1)
    expect(slow.name).toBe("heavy")
    expect(slow.ms >= 5).toBe(true)
    expect(slow.root).toBe("BAG_UPDATE")
    expect(type(slow.stack)).toBe("string")
    -- The last frame took its time.
    expect(perf.lastFrame >= 5).toBe(true)
    expect(perf.over > 0).toBe(true)
    local data = rec.data
    expect(data.functions.heavy.over).toBe(1)
    expect(data.roots.BAG_UPDATE.calls).toBe(1)
    expect(data.roots.BAG_UPDATE.top).toBe("heavy")
    local found = false
    for _, p in ipairs(data.paths) do
        if table.concat(p.path, ">") == "BAG_UPDATE>handler>heavy" then
            found = p.calls == 1 and p.total >= 5
        end
    end
    expect(found).toBe(true)
    expect(data.functions.alloc.selfGarbage > 0).toBe(true)
    -- The addon's own garbage, besides all addons'.
    expect(data.garbageAddon >= data.functions.alloc.selfGarbage).toBe(true)
    expect(#data.seconds.garbageAll).toBe(#data.seconds.garbage)
    expect(data.functions.handler.garbage >= data.functions.alloc.garbage).toBe(true)
    -- A slash command is a root too.
    mock.slash("/probe")
    local live = perf.data(perf.recordings[1])
    expect(live.roots["/probe"].kind).toBe("slash")
    expect(live.commands["/probe"].count).toBe(1)
    -- Small work is no slow call.
    hooks.work = function() mock.spend(0.5) end
    local before = #perf.slow
    mock.fireEvent("BAG_UPDATE", 0)
    expect(#perf.slow).toBe(before)
end
"#,
    );
}

/// Combats, boss pulls and manual recordings; comparing per minute; export
/// through the SavedVariables across a reload (§7.7.4, §7.7.9).
#[test]
fn recordings_and_export() {
    run(
        "",
        r#"
-- (Not profiled: the addon's garbage counts at the outermost measured call,
-- which would be the test.)
@test
@noprofile
local function records()
    local perf = probe().runtime.perf
    if mock.reloads() == 0 then
        mock.player:enterCombat()
        mock.fireEvent("BAG_UPDATE", 0)
        mock.fireEvent("BAG_UPDATE", 0)
        mock.nextFrame()
        mock.player:leaveCombat()
        local combat = perf.recordings[2]
        expect(combat.kind).toBe("combat")
        expect(combat.name).toBe("combat 1")
        expect(combat.running).toBe(false)
        expect(combat.data.functions.handler.calls).toBe(2)
        expect(combat.data.events.BAG_UPDATE.count).toBe(2)
        -- The addon's garbage is measured without garbage per function.
        expect(combat.data.garbageAddon > 0).toBe(true)
        mock.startEncounter({ id = 2902, name = "Ulgrax", difficultyID = 16, groupSize = 20 })
        mock.fireEvent("BAG_UPDATE", 0)
        mock.endEncounter(true)
        local pull = perf.recordings[#perf.recordings]
        expect(pull.name).toBe("pull 1")
        expect(pull.detail).toBe("Ulgrax")
        expect(pull.data.functions.handler.calls).toBe(1)
        -- Manual, named when stopped.
        mock.slash("/wl record Probe")
        mock.fireEvent("BAG_UPDATE", 0)
        mock.advanceFrames(60)
        local manual = perf.running("manual")
        perf.stop(manual, "my run")
        expect(manual.name).toBe("my run")
        expect(manual.data.functions.handler.calls).toBe(1)
        expect(manual.data.duration >= 0.9).toBe(true)
        -- Reset keeps what the open recordings counted.
        mock.player:enterCombat()
        mock.fireEvent("BAG_UPDATE", 0)
        perf.reset()
        mock.fireEvent("BAG_UPDATE", 0)
        mock.player:leaveCombat()
        expect(perf.recordings[#perf.recordings].data.functions.handler.calls).toBe(2)
        expect(perf.data(perf.recordings[1]).functions.handler.calls).toBe(1)
        perf.setExport(combat, true)
        local db: any = _G.WowluaDevDB_Probe
        expect(#db.exports).toBe(1)
        expect(db.exports[1].format).toBe(1)
        expect(db.exports[1].addon).toBe("Probe")
        expect(db.exports[1].name).toBe("combat 1")
        expect(db.exports[1].functions.handler.calls).toBe(2)
        expect(type(db.exports[1].seconds.frames)).toBe("table")
        expect(type(db.exports[1].hints)).toBe("table")
        mock.reloadUI()
    else
        local db: any = _G.WowluaDevDB_Probe
        expect(db.exports[1].name).toBe("combat 1")
        local saved = perf.recordings[2]
        expect(saved.name).toBe("combat 1")
        expect(saved.export).toBe(true)
        expect(perf.data(saved).functions.handler.calls).toBe(2)
        perf.setExport(saved, false)
        expect(#db.exports).toBe(0)
    end
end

@test
local function automaticRecordingsAreBounded()
    local perf = probe().runtime.perf
    for _ = 1, 25 do
        mock.player:enterCombat()
        mock.player:leaveCombat()
    end
    local auto = 0
    for _, r in ipairs(perf.recordings) do
        if (r.kind == "combat" or r.kind == "pull") and not r.export then
            auto += 1
        end
    end
    expect(auto).toBe(20)
    expect(perf.recordings[1].kind).toBe("live")
end
"#,
    );
}

/// The window's switches and thresholds win over the profile and are
/// saved per addon until reset (§7.7.5).
#[test]
fn switches_are_saved_per_addon() {
    run(
        r#", "frame-budget": 2"#,
        r#"
@test
local function saved()
    local perf = probe().runtime.perf
    if mock.reloads() == 0 then
        expect(perf.frameBudget).toBe(2)
        expect(perf.callPaths).toBe(false)
        perf.set("callPaths", true)
        perf.set("frameBudget", 4)
        mock.reloadUI()
    elseif mock.reloads() == 1 then
        expect(perf.callPaths).toBe(true)
        expect(perf.frameBudget).toBe(4)
        perf.resetSettings()
        expect(perf.frameBudget).toBe(2)
        mock.reloadUI()
    else
        expect(perf.callPaths).toBe(false)
        expect(perf.frameBudget).toBe(2)
    end
end
"#,
    );
}

/// The hints (§7.7.7): P7101, P7102, P7201 and P7301 from the live
/// recording while the window is open, with the count on the tab; a
/// dismissed hint comes back when it gets worse.
#[test]
fn hints() {
    run(
        r#", "profile-garbage": true"#,
        r#"
local function find(list: any[], code: string, subject: string): any?
    for _, x in ipairs(list) do
        if x.code == code and x.subject == subject then
            return x
        end
    end
    return nil
end

@test
local function rules()
    ticker:SetScript("OnUpdate", function() poll() end)
    hooks.work = function() mock.spend(4) end
    -- Bursts: the event six times a frame, in ten frames.
    for _ = 1, 10 do
        for _ = 1, 6 do
            mock.fireEvent("BAG_UPDATE", 0)
        end
        mock.nextFrame()
    end
    hooks.work = function() end
    mock.advanceFrames(700)
    mock.slash("/wl")
    mock.nextFrame()
    local d = probe()
    local list = d.hintState.list
    local burst = find(list, "P7101", "BAG_UPDATE")
    expect(burst ~= nil).toBe(true)
    expect(burst.evidence:match("in bursts of 5 or more in %d+ frames") ~= nil).toBe(true)
    expect(burst.recording).toBe("live")
    expect(burst.fix).toContain("@throttle")
    local update = nil
    for _, x in ipairs(list) do
        if x.code == "P7102" then
            update = x
        end
    end
    expect(update ~= nil).toBe(true)
    expect(update.evidence).toContain("no size")
    expect(find(list, "P7201", "alloc") ~= nil).toBe(true)
    local over = find(list, "P7301", "heavy")
    expect(over ~= nil).toBe(true)
    expect(over.evidence).toContain("4.00 ms")
    -- The count of new hints on the tab, until the tab is seen.
    local console = dev.console
    expect(console.newHints(d)).toBe(#list)
    console.setTab(8)
    expect(console.newHints(d)).toBe(0)
    -- Dismissed, and back when the slowest call is half as slow again.
    console.dismissHint(d, over)
    expect(find(d.hintState.list, "P7301", "heavy")).toBe(nil)
    hooks.work = function() mock.spend(5) end
    mock.fireEvent("BAG_UPDATE", 0)
    console.runHints(d)
    expect(find(d.hintState.list, "P7301", "heavy")).toBe(nil)
    hooks.work = function() mock.spend(7) end
    mock.fireEvent("BAG_UPDATE", 0)
    console.runHints(d)
    expect(find(d.hintState.list, "P7301", "heavy") ~= nil).toBe(true)
    expect(console.newHints(d)).toBe(1)
    mock.slash("/wl hints Probe")
    local printed = table.concat(mock.printed(), "\n")
    expect(printed).toContain("hint[P7301]")
end
"#,
    );
}

/// The window draws every tab and performance view, plain and compared.
#[test]
fn window_draws_every_view() {
    run(
        r#", "profile-call-paths": true, "profile-garbage": true"#,
        r#"
@test
local function draws()
    hooks.work = function() mock.spend(3) end
    mock.player:enterCombat()
    for _ = 1, 3 do
        mock.fireEvent("BAG_UPDATE", 0)
        mock.nextFrame()
    end
    mock.player:leaveCombat()
    mock.fireEvent("BAG_UPDATE", 0)
    mock.advanceFrames(10)
    mock.slash("/wl")
    local console = dev.console
    local w: any = _G.WowluaDevWindow
    expect(w:IsShown()).toBe(true)
    local perf = probe().runtime.perf
    local S = console.U.S
    for pass = 1, 2 do
        if pass == 2 then
            S.compareOf.Probe = perf.recordings[2]
        end
        for tab = 1, 8 do
            console.setTab(tab)
        end
        for _, v in ipairs({ "functions", "tree", "flame", "events", "timeline", "slow", "garbage", "bench" }) do
            S.view = v
            console.setTab(2)
        end
    end
    -- A recording of the session before: no frames in the ring.
    S.recOf.Probe = perf.recordings[2]
    perf.recordings[2].saved = true
    S.view = "timeline"
    console.setTab(2)
    mock.nextFrame()
    w:Hide()
end
"#,
    );
}

/// The window used with the mouse: a recording's menu marks it for export
/// and compares it, the switches switch, a hint is dismissed.
#[test]
fn window_by_mouse() {
    run(
        "",
        r#"
-- The shown button whose text (a region's) is `text`.
local function button(text: string): any
    for _, f in ipairs(mock.frames()) do
        local b: any = f
        if b:IsVisible() and b.GetRegions then
            for _, r in ipairs({ b:GetRegions() }) do
                local region: any = r
                if region.GetText and region:GetText() == text then
                    return b
                end
            end
        end
    end
    error("no button " .. text)
end

@test
local function clicks()
    hooks.work = function() mock.spend(3) end
    mock.player:enterCombat()
    mock.fireEvent("BAG_UPDATE", 0)
    mock.player:leaveCombat()
    mock.slash("/wl")
    local console = dev.console
    console.setTab(2)
    local perf = probe().runtime.perf
    local combat = perf.recordings[2]
    mock.click(button("combat 1"), "RightButton")
    mock.click(button("Mark for export"))
    expect(combat.export).toBe(true)
    mock.click(button("combat 1"), "RightButton")
    mock.click(button("Compare with this"))
    expect(console.U.S.compareOf.Probe).toBe(combat)
    console.refresh(true)
    mock.click(button("Call paths"))
    expect(perf.callPaths).toBe(true)
    mock.click(button("✓ Call paths"))
    expect(perf.callPaths).toBe(false)
    -- A manual recording, named when stopped.
    mock.click(button("Record"))
    expect(perf.running("manual") ~= nil).toBe(true)
    mock.click(button("Stop"))
    expect(perf.running("manual")).toBe(nil)
    -- The timeline's frames are clickable.
    console.U.S.compareOf.Probe = nil
    console.U.S.view = "timeline"
    console.refresh(true)
    mock.click(console.U.canvas)
    console.refresh(true)
    -- The footer's HUD switch.
    mock.click(button("HUD"))
    expect(console.hudShown()).toBe(true)
    -- A hint dismissed from its menu.
    mock.nextFrame()
    console.setTab(8)
    mock.click(button("hint[P7301] heavy"))
    mock.click(button("Dismiss"))
    for _, x in ipairs(probe().hintState.list) do
        expect(x.id ~= "P7301 heavy").toBe(true)
    end
end
"#,
    );
}

/// Bench (§7.7.6): n runs in chunks of at most 50 ms per frame, not in
/// combat, the last 20 runs kept.
#[test]
fn bench() {
    run(
        "",
        r#"
@test
local function runs()
    _G.spendMs = function(ms: number) mock.spend(ms) end
    mock.slash("/wl bench Probe 1 + 1 100")
    local d = probe()
    expect(d.benches[1].status).toBe("done")
    expect(d.benches[1].n).toBe(100)
    expect(d.benches[1].min <= d.benches[1].median).toBe(true)
    -- 10 runs of 20 ms: three frames.
    mock.slash("/wl bench Probe spendMs(20) 10")
    local run = d.benches[1]
    expect(run.status).toBe("running")
    expect(run.done).toBe(3)
    mock.nextFrame()
    mock.nextFrame()
    mock.nextFrame()
    expect(run.status).toBe("done")
    expect(run.median).toBe(20)
    mock.player:enterCombat()
    mock.slash("/wl bench Probe 1 + 1")
    expect(#d.benches).toBe(2)
    mock.player:leaveCombat()
    -- A last number belongs to the expression when the rest is none.
    mock.slash("/wl bench Probe 1 + 1")
    expect(d.benches[1].expr).toBe("1 + 1")
    expect(d.benches[1].n).toBe(10000)
    for _ = 1, 25 do
        mock.slash("/wl bench Probe 1 10")
    end
    expect(#d.benches).toBe(20)
end
"#,
    );
}

/// The snapshot (§7.7.10): `/wl snapshot` keeps the world, as a world
/// profile's data, in the addon's dev data half a second later.
#[test]
fn snapshot() {
    run(
        "",
        r#"
@test
local function taken()
    mock.player:setMoney(12345)
    mock.player:target(mock.spawnUnit({ name = "Hogger", level = 11, reaction = "hostile" }))
    mock.slash("/wl snapshot")
    local db: any = _G.WowluaDevDB_Probe
    expect(db.snapshot).toBe(nil)
    mock.advanceTime(1)
    local s = db.snapshot
    expect(type(s.at)).toBe("number")
    expect(s.profile.money).toBe(12345)
    expect(s.profile.character.class).toBe("WARRIOR")
    expect(s.profile.units[1].name).toBe("Hogger")
    expect(s.profile.units[1].target).toBe(true)
    expect(s.profile.errors).toBe(nil)
end
"#,
    );
}

/// The HUD (§7.7.8): a row per dev build; hidden while the window is open
/// and in cinematics; its choice saved.
#[test]
fn hud() {
    run(
        "",
        r#"
@test
local function rows()
    local console = dev.console
    if mock.reloads() == 1 then
        -- Shown again after the reload, where it was.
        expect(console.hud:IsShown()).toBe(true)
        local point, _, _, x = console.hud:GetPoint(1)
        expect(point).toBe("TOPRIGHT")
        expect(x).toBe(-220)
        return
    end
    mock.slash("/wl hud")
    expect(console.hud:IsShown()).toBe(true)
    mock.nextFrame()
    local db: any = _G.WowluaDevDB_Probe
    expect(db.hud.shown).toBe(true)
    mock.slash("/wl")
    expect(console.hud:IsShown()).toBe(false)
    mock.slash("/wl")
    expect(console.hud:IsShown()).toBe(true)
    mock.fireEvent("CINEMATIC_START", false, 0)
    expect(console.hud:IsShown()).toBe(false)
    mock.fireEvent("CINEMATIC_STOP")
    expect(console.hud:IsShown()).toBe(true)
    mock.slash("/wl hud")
    expect(console.hud:IsShown()).toBe(false)
    mock.slash("/wl hud")
    mock.reloadUI()
end
"#,
    );
}

/// The error toast (§7.10): new errors while the window is closed, bursts
/// collapsed, gone after 8 seconds unless hovered, after combat in combat.
#[test]
fn error_toast() {
    run(
        "",
        r#"
@test
local function toast()
    local console = dev.console
    hooks.work = function() error("boom") end
    mock.fireEvent("BAG_UPDATE", 0)
    expect(console.toast:IsShown()).toBe(true)
    hooks.work = function() error("bang") end
    mock.fireEvent("BAG_UPDATE", 0)
    local text = ""
    for _, r in ipairs({ console.toast:GetRegions() }) do
        if r.GetText and type(r:GetText()) == "string" then
            text ..= r:GetText() .. "|"
        end
    end
    expect(text).toContain("Probe · 2 new errors")
    -- The cursor on it pauses its timer.
    mock.hover(console.toast)
    mock.advanceFrames(9 * 60)
    expect(console.toast:IsShown()).toBe(true)
    mock.hover(nil)
    mock.advanceFrames(9 * 60)
    expect(console.toast:IsShown()).toBe(false)
    -- In combat it waits.
    mock.player:enterCombat()
    hooks.work = function() error("later") end
    mock.fireEvent("BAG_UPDATE", 0)
    expect(console.toast:IsShown()).toBe(false)
    mock.player:leaveCombat()
    expect(console.toast:IsShown()).toBe(true)
    -- Clicking opens the errors tab.
    mock.click(console.toast)
    expect(console.toast:IsShown()).toBe(false)
    expect(console.window:IsShown()).toBe(true)
    expect(console.U.S.tab).toBe(5)
    -- No toast while the window is open.
    hooks.work = function() error("open") end
    mock.fireEvent("BAG_UPDATE", 0)
    expect(console.toast:IsShown()).toBe(false)
    expect(#mock.errors()).toBe(4)
end
"#,
    );
}

/// `wowlua profile snapshot` (decision T498): the snapshot `/wl snapshot`
/// kept in the addon's dev data, found in the installation, written as a
/// world profile with the names scrubbed, checked clean; an existing
/// profile is kept unless `--force`.
#[test]
fn profile_snapshot_writes_a_world_profile() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("wowlua.json"),
        r#"{ "package": { "name": "probe", "version": "1.0.0" }, "flavors": { "camelot": {} } }"#,
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.wowlua"), "print(\"hi\")\n").unwrap();
    let sv = dir
        .path()
        .join("wow/_classic_era_/WTF/Account/ACC/SavedVariables");
    fs::create_dir_all(&sv).unwrap();
    fs::write(
        sv.join("WowluaDevDB_Probe.lua"),
        r#"
WowluaDevDB_Probe = {
	["snapshot"] = {
		["at"] = 1793170000,
		["failed"] = {
			"toys: no toy box", -- [1]
		},
		["profile"] = {
			["character"] = {
				["name"] = "Suuna",
				["realm"] = "Golemagg",
				["class"] = "SHAMAN",
				["level"] = 60,
			},
			["money"] = 123456,
			["actions"] = {
				[1] = { ["type"] = "spell", ["id"] = 403 },
				[5] = { ["type"] = "item", ["id"] = 159 },
			},
			["units"] = {
				{ ["name"] = "Hogger", ["level"] = 11, ["target"] = true }, -- [1]
			},
		},
	},
}
"#,
    )
    .unwrap();
    let snapshot = |args: &[&str]| {
        let out = wowlua(dir.path())
            .args(["profile", "snapshot", "raid", "--wow", "wow"])
            .args(args)
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    };
    let (ok, text) = snapshot(&[]);
    assert!(ok, "{text}");
    assert!(text.contains("toys: no toy box"), "{text}");
    assert!(text.contains("tests/profiles/raid.json"), "{text}");
    let profile = fs::read_to_string(dir.path().join("tests/profiles/raid.json")).unwrap();
    assert!(
        !profile.contains("Suuna") && !profile.contains("Golemagg"),
        "{profile}"
    );
    let v: serde_json::Value = serde_json::from_str(&profile).unwrap();
    assert_eq!(v["character"]["name"], "Player");
    assert_eq!(v["actions"]["5"]["id"], 159);
    assert_eq!(v["units"][0]["name"], "Hogger");
    // There already: kept, unless --force.
    let (ok, text) = snapshot(&[]);
    assert!(!ok && text.contains("exists"), "{text}");
    let (ok, text) = snapshot(&["--force"]);
    assert!(ok, "{text}");
    // The package checks clean with it.
    let out = wowlua(dir.path()).arg("check").output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `wowlua prof report` (Tooling §16.5, decisions T175 and T179) on what the
/// runtime exported: a boss pull marked for export, written to the
/// SavedVariables file as the client writes it, found in the installation's
/// accounts and clients, its stacks mapped to the source, as an HTML page.
#[test]
fn prof_report_of_an_exported_pull() {
    let dir = package(
        r#", "profile-call-paths": true, "profile-garbage": true, "slow-call": 3"#,
        r#"
-- WowluaDevDB_Probe as the client writes SavedVariables.
local function sv(v: any, indent: string): string
    local t = type(v)
    if t == "string" then
        return (string.format("%q", v):gsub("\\\n", "\\n"))
    elseif t == "number" then
        if v == math.floor(v) and math.abs(v) < 1e15 then
            return string.format("%d", v)
        end
        return string.format("%.16g", v)
    elseif t ~= "table" then
        return tostring(v)
    end
    local inner = indent .. "\t"
    local out = { "{\n" }
    local n = #v
    for i = 1, n do
        out[#out + 1] = `{inner}{sv(v[i], inner)}, -- [{i}]\n`
    end
    local keys = {}
    for k in pairs(v) do
        if not (type(k) == "number" and k >= 1 and k <= n and k == math.floor(k)) then
            keys[#keys + 1] = k
        end
    end
    table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
    for _, k in ipairs(keys) do
        if type(v[k]) ~= "function" then
            local key = type(k) == "string" and string.format("%q", k) or tostring(k)
            out[#out + 1] = `{inner}[{key}] = {sv(v[k], inner)},\n`
        end
    end
    out[#out + 1] = indent .. "}"
    return table.concat(out)
end

@test
@noprofile
local function exportsAPull()
    local perf = probe().runtime.perf
    hooks.work = function() mock.spend(5) end
    mock.startEncounter({ id = 2902, name = "Ulgrax", difficultyID = 16, groupSize = 20 })
    mock.fireEvent("BAG_UPDATE", 0)
    mock.advanceFrames(120)
    mock.slash("/probe")
    mock.endEncounter(true)
    local pull = perf.recordings[#perf.recordings]
    expect(pull.name).toBe("pull 1")
    perf.setExport(pull, true)
    local db: any = _G.WowluaDevDB_Probe
    expect(#db.exports).toBe(1)
    expect(#db.exports[1].slow > 0).toBe(true)
    expect("\nWowluaDevDB_Probe = " .. sv(db, "") .. "\n").toMatchSnapshot("SavedVariables")
end
"#,
    );
    // The snapshot holds the file's text as a Lua string.
    let snap = fs::read_to_string(
        dir.path()
            .join("tests/__snapshots__/devtools.test.wowlua.snap"),
    )
    .unwrap();
    let mut literal = String::new();
    let mut inside = false;
    for line in snap.lines() {
        if line.starts_with("# ") {
            inside = line.ends_with(" SavedVariables");
        } else if inside && let Some(rest) = line.strip_prefix('|') {
            literal.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            literal.push('\n');
        }
    }
    let lua = mlua::Lua::new();
    let file: mlua::LuaString = lua.load(format!("return {literal}")).eval().unwrap();
    let file = file.as_bytes().to_vec();
    let wow = dir.path().join("wow");
    let place = |client: &str, account: &str| {
        let sv = wow
            .join(client)
            .join("WTF/Account")
            .join(account)
            .join("SavedVariables");
        fs::create_dir_all(&sv).unwrap();
        fs::write(sv.join("WowluaDevDB_Probe.lua"), &file).unwrap();
    };
    place("_retail_", "ACCOUNT1");
    // An account without exports, and another addon's data, do not count.
    fs::create_dir_all(wow.join("_retail_/WTF/Account/EMPTY/SavedVariables")).unwrap();
    fs::write(
        wow.join("_retail_/WTF/Account/EMPTY/SavedVariables/WowluaDevDB_Probe.lua"),
        "WowluaDevDB_Probe = {\n[\"settings\"] = {\n},\n}\n",
    )
    .unwrap();
    let report = |args: &[&str]| {
        let out = wowlua(dir.path())
            .args(["prof", "report", "--no-open", "--wow", "wow"])
            .args(args)
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    };
    let page_data = |path: &Path| -> serde_json::Value {
        let page = fs::read_to_string(path).unwrap();
        let start = page
            .find("<script type=\"application/json\" id=\"data\">")
            .unwrap();
        let rest = &page[start..];
        let json = &rest[rest.find('>').unwrap() + 1..rest.find("</script>").unwrap()];
        serde_json::from_str(json).unwrap()
    };

    // One place with recordings: the default page.
    let (ok, text) = report(&[]);
    assert!(ok, "{text}");
    assert!(text.contains("Read 1 recording of Probe"), "{text}");
    let reports: Vec<_> = fs::read_dir(dir.path().join("dist/reports"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(reports.len(), 1, "{reports:?}");
    let name = reports[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        regex::Regex::new(r"^Probe-\d{4}-\d{2}-\d{2}\.html$")
            .unwrap()
            .is_match(&name),
        "{name}"
    );
    let page = fs::read_to_string(&reports[0]).unwrap();
    // Self-contained: the design's fonts inside, nothing from the network.
    assert!(page.contains("font-family: \"Martian Mono\""));
    for load in ["src=\"http", "href=\"http", "url(http", "@import"] {
        assert!(!page.contains(load), "{load}");
    }
    let data = page_data(&reports[0]);
    assert_eq!(data["client"], "_retail_");
    assert_eq!(data["account"], "ACCOUNT1");
    let rec = &data["recordings"][0];
    assert_eq!(rec["format"], 1);
    assert_eq!(rec["addon"], "Probe");
    assert_eq!(rec["name"], "pull 1");
    assert_eq!(rec["kind"], "pull");
    assert_eq!(rec["detail"], "Ulgrax");
    assert_eq!(rec["callPaths"], true);
    assert_eq!(rec["functions"]["heavy"]["calls"], 2);
    assert!(
        rec["functions"]["heavy"]["location"]
            .as_str()
            .unwrap()
            .starts_with("src/main.wowlua:"),
        "{rec}"
    );
    assert!(rec["frames"].as_i64().unwrap() >= 120, "{rec}");
    assert!(
        rec["seconds"]["frames"].as_array().unwrap().len() >= 2,
        "{rec}"
    );
    let paths: Vec<String> = rec["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            p["path"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
                .join(">")
        })
        .collect();
    assert!(
        paths.contains(&"BAG_UPDATE>handler>heavy".to_string()),
        "{paths:?}"
    );
    assert!(
        paths.contains(&"/probe>handler>heavy".to_string()),
        "{paths:?}"
    );
    let slow = rec["slow"].as_array().unwrap();
    assert_eq!(slow.len(), 2, "{slow:?}");
    assert_eq!(slow[0]["root"], "/probe");
    assert_eq!(slow[1]["root"], "BAG_UPDATE");
    // The stack, with its lines mapped to the source.
    let stack = slow[0]["stack"].as_str().unwrap();
    assert!(stack.contains("→ src/main.wowlua:"), "{stack}");
    let hints: Vec<&str> = rec["hints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["code"].as_str().unwrap())
        .collect();
    assert!(hints.contains(&"P7301"), "{hints:?}");

    // Several accounts and clients: without a terminal, the command lists
    // them and asks for --client and --account.
    place("_classic_", "OTHER");
    let (ok, text) = report(&[]);
    assert!(!ok, "{text}");
    assert!(text.contains("several accounts or clients"), "{text}");
    assert!(
        text.contains("_retail_  ACCOUNT1  1 recording (pull 1)"),
        "{text}"
    );
    assert!(text.contains("_classic_  OTHER"), "{text}");
    assert!(text.contains("--client"), "{text}");
    let (ok, text) = report(&[
        "--account",
        "other",
        "--out",
        "pull.html",
        "--recording",
        "pull 1",
    ]);
    assert!(ok, "{text}");
    assert_eq!(
        page_data(&dir.path().join("pull.html"))["client"],
        "_classic_"
    );
    let (ok, text) = report(&["--client", "retail", "--recording", "pull 9"]);
    assert!(!ok, "{text}");
    assert!(
        text.contains("no exported recording named `pull 9`"),
        "{text}"
    );
    assert!(text.contains("exported: pull 1"), "{text}");
    let (ok, text) = report(&["--account", "EMPTY"]);
    assert!(!ok, "{text}");
    assert!(text.contains("no exported recordings of Probe"), "{text}");
    assert!(text.contains("Mark for export"), "{text}");
}
