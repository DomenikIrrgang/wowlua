//! `/stdtest engine` in the engine: StdTest's probe battery
//! (`tests/ingame/src/engineprobe.wowlua`), built with stand-ins for its
//! state and harness modules, runs in wowlua's VM with camelot's engine;
//! its results must be what WoW Forever recorded
//! (`crates/wowlua-engine/tests/client/forever-camelot-engine.lua`), but
//! for what depends on the character, the moment or Blizzard's UI code.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use assert_cmd::Command;
use wowlua_vm::{Value, Vm};

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// The probe as an addon `StdTest` for camelot: `StdTestEngineProbe(arg)`.
fn build() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = r#"{ "package": { "name": "std-test", "version": "0.1.0" },
  "addon": { "saved-variables": ["StdTestState"] },
  "flavors": { "camelot": {} } }
"#;
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::copy(
        repo().join("tests/ingame/src/engineprobe.wowlua"),
        src.join("engineprobe.wowlua"),
    )
    .unwrap();
    fs::write(
        src.join("state.wowlua"),
        "export function state(): table\n    if type(StdTestState) ~= \"table\" then StdTestState = {} end\n    return StdTestState\nend\n",
    )
    .unwrap();
    fs::write(
        src.join("harness.wowlua"),
        "export function say(...: any)\n    print(...)\nend\n",
    )
    .unwrap();
    fs::write(
        src.join("main.wowlua"),
        "import engineProbe from \"/engineprobe\"\n_G.StdTestEngineProbe = engineProbe\n",
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

/// Runs Lua as StdTest's code.
fn addon(vm: &mut Vm, src: &str) {
    let f = vm
        .load_as(
            src.as_bytes(),
            "@Interface/AddOns/StdTest/test.lua",
            "StdTest",
        )
        .expect("compiles");
    if let Err(e) = vm.call(f, &[]) {
        panic!("{}", text(vm, e.value));
    }
}

/// The client: camelot's engine with StdTest loaded.
fn client(out: &Path) -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "__engine");
    addon(&mut vm, "StdTestState = {}");
    let toc = fs::read_to_string(out.join("StdTest_Camelot.toc"))
        .or_else(|_| fs::read_to_string(out.join("StdTest.toc")))
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
        let code = fs::read(out.join(&path)).unwrap();
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
    vm
}

/// `StdTestState`'s probe results, flattened as the fixture is.
fn flatten(vm: &mut Vm) -> BTreeMap<String, String> {
    let f = vm
        .load(
            br#"local out = {}
local function dump(v, prefix)
    if type(v) ~= "table" then out[prefix] = tostring(v) return end
    for k, inner in pairs(v) do dump(inner, prefix == "" and tostring(k) or (prefix .. "." .. tostring(k))) end
end
local s = StdTestState
dump(s.engineProbe, "probe")
dump(s.engineOrder, "order")
dump(s.engineProtected, "protected")
dump(s.engineBlocked, "blocked")
return out"#,
            "=flatten",
        )
        .expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    vm.heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(vm.str_bytes(v)).into_owned(),
            )),
            _ => None,
        })
        .collect()
}

fn fixture() -> BTreeMap<String, String> {
    let src = fs::read(repo().join("crates/wowlua-engine/tests/client/forever-camelot-engine.lua"))
        .expect("the fixture");
    let mut vm = Vm::new();
    let f = vm.load(&src, "=fixture").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    vm.heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(vm.str_bytes(v)).into_owned(),
            )),
            _ => None,
        })
        .collect()
}

/// The number globals the client defines itself (the engine's data).
fn client_globals() -> &'static std::collections::HashSet<String> {
    static NAMES: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        fs::read_to_string(repo().join("crates/wowlua-engine/data/camelot-globals.txt"))
            .expect("the globals")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split('\t').next().map(str::to_string))
            .collect()
    })
}

/// Results that depend on the character, the moment, the session or
/// Blizzard's UI code (not loaded here): not compared.
fn compared(key: &str) -> bool {
    const SKIP: &[&str] = &[
        "probe.client",
        "probe.getTime",
        "probe.units.",
        "probe.widgets.AuraContainer.",
        // Session values: addresses, the UI scale, the screen.
        "probe.widgets.UIParent.getters.Get",
        "probe.widgets.FontString.getters.GetFont",
        "probe.widgets.SimpleHTML.getters.GetTextColor",
        "probe.cvarDefaults.",
        "probe.blizzard",
        "probe.behaviours.CreateFrame('EventFrame')",
        "probe.behaviours.GetFontObject after a name",
        "probe.behaviours.SetFontObject('GameFontNormal')",
        "probe.behaviours.GetFont fresh FontString",
        "protected.calls.17 UnitName",
        "probe.errors.GetScript",
        // The probed character, place and session; the engine's world is
        // neutral (and its fonts are Blizzard's).
        "probe.loadCalls.C_AddOns.GetNumAddOns",
        "probe.loadCalls.C_Container.CalculateTotalNumberOfFreeBagSlots",
        "probe.loadCalls.C_PvP.GetZonePVPInfo",
        "probe.loadCalls.C_QuestLog.GetNumQuestLogEntries",
        "probe.loadCalls.C_VoiceChat.GetTtsVoices",
        "probe.loadCalls.GetCursorPosition",
        "probe.loadCalls.GetFonts",
        "probe.loadCalls.GetGameTime",
        "probe.loadCalls.GetInstanceInfo",
        "probe.loadCalls.GetMinimapZoneText",
        "probe.loadCalls.GetPhysicalScreenSize",
        "probe.loadCalls.GetRealmName",
        "probe.loadCalls.GetRestState",
        "probe.loadCalls.GetScreenHeight",
        "probe.loadCalls.GetXPExhaustion",
        "probe.loadCalls.IsGuildLeader",
        "probe.loadCalls.GetNumLanguages",
        "probe.loadCalls.GetNumShapeshiftForms",
        "probe.loadCalls.C_LFGListRoles.GetSavedRoles",
        "probe.loadCalls.GetNetStats",
        "probe.loadCalls.GuildControlGetNumRanks",
        "probe.loadCalls.BNGetNumFriends",
        // Blizzard's UI code makes these (not loaded here): the intrinsic
        // types and its secure store frame.
        "probe.widgets.EventFrame.",
        "probe.widgets.ScrollingMessageFrame.",
        "probe.widgets.DropdownButton.",
        "probe.widgets.ContainedAlertFrame.",
        "probe.forbidden.",
    ];
    // The client's other console commands are unknown here, and so are
    // the CVars it has without a default.
    if key.starts_with("probe.allCVars.#type") {
        return false;
    }
    // Of the globals, the client's own (`LE_*`); Blizzard's code sets the
    // rest.
    if let Some(name) = key.strip_prefix("probe.scalars.") {
        return client_globals().contains(name);
    }
    !SKIP.iter().any(|s| key.starts_with(s))
        && !key.ends_with(".GetDebugName")
        && !key.ends_with(".GetEffectiveScale")
}

/// Whether a result matches the client's: exactly, but layout sizes up to
/// float precision (the client's last bits follow the session's screen and
/// UI scale) and a list the client returns in its session's hash order as
/// a set.
fn same(key: &str, expected: &str, mine: &str) -> bool {
    if expected == mine {
        return true;
    }
    if key.starts_with("probe.layoutSizes.") {
        let numbers =
            |s: &str| -> Vec<f64> { s.split(' ').filter_map(|n| n.parse().ok()).collect() };
        let (a, b) = (numbers(expected), numbers(mine));
        return a.len() == b.len()
            && a.iter()
                .zip(&b)
                .all(|(x, y)| (x - y).abs() <= x.abs() * 1e-6);
    }
    if key == "probe.loadCalls.C_UnitAuras.GetHiddenGroupBuffs" {
        let values = |s: &str| -> std::collections::BTreeSet<String> {
            s.split(['{', ',', '}'])
                .filter_map(|e| e.split_once('=').map(|(_, v)| v.to_string()))
                .collect()
        };
        let count = |s: &str| s.matches('=').count();
        return count(expected) == count(mine) && values(expected) == values(mine);
    }
    false
}

#[test]
fn the_engine_answers_the_probe_battery_as_the_client_did() {
    let dir = build();
    let out = dir.path().join("dist/dev/StdTest");
    let mut vm = client(&out);
    // The client's font objects (Blizzard's UI code is not loaded here).
    addon(
        &mut vm,
        "CreateFont('GameFontNormal'):SetFont('Fonts\\\\FRIZQT__.TTF', 12, '')",
    );
    // Blizzard's `ColorMixin` (colors are made with it), its methods.
    addon(
        &mut vm,
        "ColorMixin = {} for _, m in ipairs({ 'GenerateHexColor', 'GenerateHexColorMarkup', \
         'GenerateHexColorNoAlpha', 'GetHSL', 'GetRGB', 'GetRGBA', 'GetRGBAAsBytes', 'GetRGBAsBytes', \
         'IsEqualTo', 'IsRGBEqualTo', 'OnLoad', 'SetRGB', 'SetRGBA', 'WrapTextInColorCode' }) do \
         ColorMixin[m] = function() end end",
    );
    addon(&mut vm, "StdTestEngineProbe('')");
    wowlua_engine::clock::advance_frames(&mut vm, 60, 1000.0 / 60.0);
    addon(&mut vm, "StdTestEngineProbe('order')");
    let shift = [vm.str("LSHIFT"), Value::Num(1.0)];
    wowlua_engine::widgets::fire_event(&mut vm, "MODIFIER_STATE_CHANGED", &shift);
    addon(&mut vm, "StdTestEngineProbe('protected')");
    wowlua_engine::clock::advance_frames(&mut vm, 60, 1000.0 / 60.0);
    addon(&mut vm, "StdTestEngineProbe('blocked')");
    addon(&mut vm, "__engine.player:enterCombat()");
    wowlua_engine::clock::advance_frames(&mut vm, 90, 1000.0 / 60.0);
    let ours = flatten(&mut vm);
    let theirs = fixture();
    let mut differences = Vec::new();
    for (key, expected) in theirs
        .iter()
        .filter(|(k, v)| compared(k) && !(k.starts_with("probe.allCVars.") && v.as_str() == "0: "))
    {
        match ours.get(key) {
            Some(mine) if same(key, expected, mine) => {}
            mine => differences.push(format!(
                "{key}\n    client: {expected}\n    ours:   {}",
                mine.map_or("<missing>", String::as_str)
            )),
        }
    }
    // `Enum`, `Constants`, the number globals and the CVars were recorded
    // whole: nothing more either.
    for key in ours.keys().filter(|k| {
        [
            "probe.enums.",
            "probe.constants.",
            "probe.scalars.",
            "probe.allCVars.",
        ]
        .iter()
        .any(|p| k.starts_with(p))
            && !theirs.contains_key(*k)
    }) {
        differences.push(format!(
            "{key}\n    client: <missing>\n    ours:   {}",
            ours[key]
        ));
    }
    assert!(
        differences.is_empty(),
        "{} of {} differ:\n{}",
        differences.len(),
        theirs.keys().filter(|k| compared(k)).count(),
        differences.join("\n")
    );
}

/// `/stdtest engine input` replayed with the engine's controls: the same
/// scripts with the same arguments, in the client's order.
#[test]
fn the_engine_takes_input_as_the_client_did() {
    let src = fs::read(repo().join("crates/wowlua-engine/tests/client/forever-camelot-input.lua"))
        .expect("the fixture");
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "__engine");
    let f = vm.load(&src, "=fixture").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let n = vm.heap.table(t).border();
    let client: Vec<String> = (1..=n)
        .map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            // Without the time: "19.298 button OnClick(...)".
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s))
                .split_once(' ')
                .expect("a time")
                .1
                .to_string(),
            _ => panic!("a string"),
        })
        .collect();
    // The probe's frames and scripts (its templates' looks aside).
    addon(
        &mut vm,
        r##"InputLog = {}
local function note(what) InputLog[#InputLog + 1] = what end
local b = CreateFrame("Button", "StdTestInputButton", UIParent)
b:SetSize(160, 28)
b:SetPoint("CENTER", 0, 60)
b:RegisterForClicks("AnyUp")
for _, script in ipairs({ "OnEnter", "OnLeave", "OnMouseDown", "OnMouseUp", "PreClick", "OnClick", "PostClick" }) do
    b:SetScript(script, function(_, ...)
        local args = {}
        for i = 1, select("#", ...) do args[#args + 1] = tostring((select(i, ...))) end
        note(("button %s(%s) shift=%s"):format(script, table.concat(args, ", "), tostring(IsShiftKeyDown())))
    end)
end
local e = CreateFrame("EditBox", "StdTestInputBox", UIParent)
e:SetSize(160, 24)
e:SetPoint("CENTER", 0, 20)
e:SetAutoFocus(false)
e:SetText("")
e:Show()
for _, script in ipairs({ "OnEditFocusGained", "OnEditFocusLost", "OnChar", "OnTextChanged", "OnEnterPressed", "OnEscapePressed", "OnTabPressed" }) do
    e:SetScript(script, function(self, ...)
        local args = {}
        for i = 1, select("#", ...) do args[#args + 1] = tostring((select(i, ...))) end
        note(("box %s(%s) text=%s focus=%s"):format(script, table.concat(args, ", "), self:GetText(), tostring(self:HasFocus())))
        if script == "OnEnterPressed" or script == "OnEscapePressed" then self:ClearFocus() end
    end)
end
local m = CreateFrame("Frame")
m:RegisterEvent("MODIFIER_STATE_CHANGED")
m:SetScript("OnEvent", function(_, event, key, down) note(("%s(%s, %s)"):format(event, key, tostring(down))) end)"##,
    );
    wowlua_engine::clock::advance_frames(&mut vm, 1, 1000.0 / 60.0);
    addon(
        &mut vm,
        r#"local b, e = StdTestInputButton, StdTestInputBox
__engine.hover(b) __engine.hover(nil)
__engine.hover(b) __engine.hover(nil)
__engine.hover(b)
__engine.click(b)
__engine.setModifiers({ shift = true })
__engine.click(b)
__engine.setModifiers({})
__engine.hover(nil)
__engine.click(e)
__engine.type(e, "ab")
__engine.pressKey("ENTER")"#,
    );
    let ours: Vec<String> = {
        let log = vm.get_global("InputLog");
        let Value::Table(t) = log else {
            panic!("a log")
        };
        let n = vm.heap.table(t).border();
        (1..=n)
            .map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
                Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                _ => panic!("a string"),
            })
            .collect()
    };
    assert_eq!(ours, client);
}
