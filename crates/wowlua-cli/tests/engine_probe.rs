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
    for file in [
        "engineprobe.wowlua",
        "answers.wowlua",
        "maps.wowlua",
        "collections.wowlua",
        "startup.wowlua",
    ] {
        fs::copy(repo().join("tests/ingame/src").join(file), src.join(file)).unwrap();
    }
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
    client_in(vm, out)
}

/// StdTest loaded in an engine (with Blizzard's UI or not).
fn client_in(mut vm: Vm, out: &Path) -> Vm {
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
        "probe.loadCalls.C_Navigation.GetFrame",
        "probe.loadCalls.C_SuperTrack.GetSuperTrackedQuestID",
        "probe.loadCalls.GetSubZoneText",
        "probe.loadCalls.C_DateAndTime.IsDayTime",
        // The player's own setting (and where the server stores it).
        "probe.cvarFlags.GetCVarInfo(SoftTargetEnemy)",
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

/// A text's numbers, and the text between them.
fn split_numbers(s: &str) -> (Vec<String>, Vec<f64>) {
    let (mut text, mut numbers) = (Vec::new(), Vec::new());
    let (mut cur, mut num) = (String::new(), String::new());
    for ch in s.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() || (ch == '.' && !num.is_empty()) {
            num.push(ch);
            continue;
        }
        if !num.is_empty() {
            numbers.push(num.parse().unwrap_or(f64::NAN));
            text.push(std::mem::take(&mut cur));
            num.clear();
        }
        cur.push(ch);
    }
    text.push(cur);
    (text, numbers)
}

/// Whether a result matches the client's: exactly, but layout sizes (and
/// the sizes `OnSizeChanged` gets) up to float precision (the client's last bits follow the session's screen and
/// UI scale) and a list the client returns in its session's hash order as
/// a set.
fn same(key: &str, expected: &str, mine: &str) -> bool {
    if expected == mine {
        return true;
    }
    if key.starts_with("probe.layoutSizes.") || key == "probe.timers.sizeChanged" {
        // The same text around numbers within a float's last bits.
        let (ta, a) = split_numbers(expected);
        let (tb, b) = split_numbers(mine);
        return ta == tb
            && a.len() == b.len()
            && a.iter()
                .zip(&b)
                .all(|(x, y)| (x - y).abs() <= x.abs() * 1e-5);
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
    // The client's CVars without a default are not the engine's (as above):
    // the flag lists leave them out.
    let no_default = |name: &str| {
        theirs
            .get(&format!("probe.allCVars.{name}"))
            .is_some_and(|v| v == "0: ")
    };
    for (key, expected) in theirs
        .iter()
        .filter(|(k, v)| compared(k) && !(k.starts_with("probe.allCVars.") && v.as_str() == "0: "))
    {
        let filtered;
        let expected = if key.starts_with("probe.cvarFlags.") && !key.contains('(') {
            filtered = expected
                .split(' ')
                .filter(|n| !no_default(n))
                .collect::<Vec<_>>()
                .join(" ");
            &filtered
        } else {
            expected
        };
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

/// `/stdtest engine input` (before its slider) replayed with the engine's
/// controls: the same scripts with the same arguments, in the client's
/// order.
#[test]
fn the_engine_takes_input_as_the_client_did() {
    let src =
        fs::read(repo().join("crates/wowlua-engine/tests/client/forever-camelot-input-typing.lua"))
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

/// `/stdtest engine input` with its slider replayed with the engine's
/// controls: the global mouse events around each click's scripts, an edit
/// box losing the focus to a click elsewhere, the slider's thumb dragged
/// (a drag's moves compared by where they end), the wheel over it.
#[test]
fn the_engine_takes_mouse_input_as_the_client_did() {
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
    // Without the time, and the player's own Cmd presses (not replayed).
    let client: Vec<String> = (1..=n)
        .map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s))
                .split_once(' ')
                .expect("a time")
                .1
                .to_string(),
            _ => panic!("a string"),
        })
        .filter(|l| !l.starts_with("MODIFIER_STATE_CHANGED(LMETA"))
        .collect();
    // The wheel's turns, in order, replayed as they came.
    let wheel: Vec<String> = client
        .iter()
        .filter_map(|l| l.strip_prefix("slider OnMouseWheel("))
        .map(|l| l.split(')').next().unwrap_or("0").to_string())
        .collect();
    // Where the thumb was let go.
    let to = client
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("slider OnValueChanged("))
        .and_then(|l| l.split(',').next())
        .expect("a drag")
        .to_string();
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
local sl = CreateFrame("Slider", "StdTestInputSlider", UIParent)
sl:SetSize(200, 17)
sl:SetPoint("CENTER", 0, -20)
sl:SetMinMaxValues(0, 10)
sl:SetValueStep(2)
sl:SetValue(0)
sl:EnableMouseWheel(true)
sl:Show()
for _, script in ipairs({ "OnMouseDown", "OnMouseUp", "OnValueChanged", "OnMouseWheel" }) do
    sl:SetScript(script, function(self, ...)
        local args = {}
        for i = 1, select("#", ...) do args[#args + 1] = tostring((select(i, ...))) end
        note(("slider %s(%s) value=%s dragging=%s"):format(script, table.concat(args, ", "), tostring(self:GetValue()), tostring(self:IsDraggingThumb())))
    end)
end
local m = CreateFrame("Frame")
m:RegisterEvent("MODIFIER_STATE_CHANGED")
m:RegisterEvent("GLOBAL_MOUSE_DOWN")
m:RegisterEvent("GLOBAL_MOUSE_UP")
m:SetScript("OnEvent", function(_, event, key, down) note(("%s(%s, %s)"):format(event, key, tostring(down))) end)"##,
    );
    wowlua_engine::clock::advance_frames(&mut vm, 1, 1000.0 / 60.0);
    let turns: Vec<String> = wheel
        .iter()
        .map(|d| format!("__engine.wheel(sl, {d})"))
        .collect();
    addon(
        &mut vm,
        &format!(
            r#"local b, e, sl = StdTestInputButton, StdTestInputBox, StdTestInputSlider
__engine.hover(b)
__engine.click(b)
__engine.hover(nil)
__engine.click(e)
__engine.slide(sl, {to})
{}"#,
            turns.join("\n")
        ),
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
    // A drag's moves (after its mouse down) by where they end.
    let moves = |log: &[String]| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for l in log {
            let is_move = l.starts_with("slider OnValueChanged(")
                && out.last().is_some_and(|p| {
                    p.starts_with("slider OnValueChanged(") || p.starts_with("slider OnMouseDown(")
                })
                && out.iter().any(|p| p.starts_with("slider OnMouseDown("));
            if is_move
                && out
                    .last()
                    .is_some_and(|p| p.starts_with("slider OnValueChanged("))
            {
                out.pop();
            }
            out.push(l.clone());
        }
        out
    };
    assert_eq!(moves(&ours), moves(&client));
}

/// `/stdtest engine keys` replayed: keyboard frames passing keys on, an
/// override binding of J clicking a button; J, Shift-J, J kept by the top
/// frame (it stops propagation in its handler), J past a keyboard frame
/// without a key script. The same scripts, clicks and order as the client.
#[test]
fn the_engine_sends_keys_as_the_client_did() {
    let src = fs::read(repo().join("crates/wowlua-engine/tests/client/forever-camelot-keys.lua"))
        .expect("the fixture");
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "__engine");
    let f = vm.load(&src, "=fixture").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let n = vm.heap.table(t).entries().count();
    let client: Vec<String> = (1..=n)
        .map(|i| {
            let k = vm.str(&format!("{i:03}"));
            match vm.heap.table(t).get(k) {
                Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                _ => panic!("a string"),
            }
        })
        .collect();
    addon(
        &mut vm,
        r##"KeyLog = {}
Phase = 1
local function note(what) KeyLog[#KeyLog + 1] = ("p%d %s"):format(Phase, what) end
local owner = CreateFrame("Frame")
local target = CreateFrame("Button", "StdTestKeyTarget", UIParent)
target:RegisterForClicks("AnyDown", "AnyUp")
target:SetScript("OnClick", function(_, button, down) note(("binding click %s down=%s"):format(button, tostring(down))) end)
SetOverrideBindingClick(owner, true, "J", "StdTestKeyTarget")
local function keyFrame(name, strata, level)
    local f = CreateFrame("Frame", nil, UIParent)
    f:SetFrameStrata(strata)
    f:SetFrameLevel(level)
    f:SetSize(10, 10)
    f:SetPoint("TOPLEFT")
    f:EnableKeyboard(true)
    f:SetPropagateKeyboardInput(true)
    f:SetScript("OnKeyDown", function(self, key)
        if Phase == 3 and name == "top" and key == "J" then self:SetPropagateKeyboardInput(false) end
        note(("%s down %s shift=%s propagate=%s"):format(name, key, tostring(IsShiftKeyDown()), tostring(self:GetPropagateKeyboardInput())))
    end)
    f:SetScript("OnKeyUp", function(_, key) note(("%s up %s"):format(name, key)) end)
    return f
end
Top = keyFrame("top", "DIALOG", 5)
keyFrame("high", "HIGH", 10)
Bare = CreateFrame("Frame", nil, UIParent)
Bare:SetFrameStrata("MEDIUM")
Bare:SetSize(10, 10)
Bare:SetPoint("TOPLEFT")
Bare:SetScript("OnKeyDown", function() end)
Bare:SetScript("OnKeyDown", nil)
Bare:SetPropagateKeyboardInput(false)
Bare:Hide()"##,
    );
    addon(
        &mut vm,
        r#"__engine.pressKey("J")
Phase = 2
__engine.setModifiers({ shift = true })
__engine.pressKey("J")
__engine.setModifiers({})
Top:SetPropagateKeyboardInput(true)
Phase = 3
__engine.pressKey("J")
Top:SetPropagateKeyboardInput(true)
Phase = 4
Bare:Show()
KeyLog[#KeyLog + 1] = ("p4 bare keyboard %s propagate %s"):format(tostring(Bare:IsKeyboardEnabled()), tostring(Bare:GetPropagateKeyboardInput()))
__engine.pressKey("J")"#,
    );
    let ours: Vec<String> = {
        let Value::Table(t) = vm.get_global("KeyLog") else {
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

/// How many of the client's answers (`/stdtest engine answers`) the engine
/// gives as well: the calls Blizzard's UI made that the engine could not
/// answer, asked again with the recorded character's world. Only ever
/// raised (decision: the engine answers every call as the client does).
const ANSWERS_MATCHED: usize = 5151;

#[test]
fn the_engine_answers_blizzards_calls_as_the_client_did() {
    // The calls are Blizzard's UI's: its code (fonts, mixins) is loaded too.
    let Some(ui) = wowlua_test::ui_source_in(&repo().join("target/types-src"), "camelot") else {
        eprintln!("skipped: no Blizzard UI code (run `cargo xtask types fetch`)");
        return;
    };
    let dir = build();
    let out = dir.path().join("dist/dev/StdTest");
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let report = wowlua_engine::ui::load(&mut vm, &ui.interface, ui.strings.as_deref());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut vm = client_in(vm, &out);
    let controls = wowlua_engine::controls::table(&mut vm);
    wowlua_test::profile::apply(
        &mut vm,
        controls,
        &repo().join("crates/wowlua-engine/tests/client/forever-camelot-character.json"),
    )
    .expect("the character's profile");
    // Logged in, as the client was (Blizzard's code finishes setting up
    // then: its text size manager scales the fonts).
    let name = vm.str("StdTest");
    wowlua_engine::widgets::fire_event(&mut vm, "ADDON_LOADED", &[name, Value::Bool(false)]);
    wowlua_engine::widgets::fire_event(&mut vm, "PLAYER_LOGIN", &[]);
    wowlua_engine::widgets::fire_event(
        &mut vm,
        "PLAYER_ENTERING_WORLD",
        &[Value::Bool(true), Value::Bool(false)],
    );
    // Its place in the login unverified (`/stdtest engine startup`).
    wowlua_engine::widgets::fire_event(&mut vm, "VARIABLES_LOADED", &[]);
    wowlua_engine::clock::advance_frames(&mut vm, 10, 1000.0 / 60.0);
    addon(&mut vm, "StdTestEngineProbe('answers')");
    let f = vm
        .load(
            b"local out = {} for k, v in pairs(StdTestState.engineAnswers) do out[k] = v end return out",
            "=answers",
        )
        .expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let ours: BTreeMap<String, String> = vm
        .heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(vm.str_bytes(v)).into_owned(),
            )),
            _ => None,
        })
        .collect();
    let src =
        fs::read(repo().join("crates/wowlua-engine/tests/client/forever-camelot-answers.lua"))
            .expect("the fixture");
    let mut fixture_vm = Vm::new();
    let f = fixture_vm.load(&src, "=fixture").expect("compiles");
    let r = fixture_vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let theirs: BTreeMap<String, String> = fixture_vm
        .heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(fixture_vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(fixture_vm.str_bytes(v)).into_owned(),
            )),
            _ => None,
        })
        .collect();
    let address = regex::Regex::new(r"(function|table|userdata|thread): 0x[0-9a-f]+").unwrap();
    let ours: BTreeMap<String, String> = ours
        .into_iter()
        .map(|(k, v)| (k, address.replace_all(&v, "$1").into_owned()))
        .collect();
    // `WOWLUA_ANSWERS=GetFontInfo` shows that function's differences (`*`:
    // every function's).
    let show = std::env::var("WOWLUA_ANSWERS").ok();
    let mut shown = 0;
    let mut matched = 0;
    let mut by_function: BTreeMap<String, usize> = BTreeMap::new();
    for (call, answer) in &theirs {
        if ours.get(call) == Some(answer) {
            matched += 1;
        } else {
            if show
                .as_deref()
                .is_some_and(|f| f == "*" || call.starts_with(&format!("{f}(")))
                && shown
                    < std::env::var("WOWLUA_ANSWERS_LIMIT")
                        .ok()
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(8)
            {
                shown += 1;
                let mine = ours.get(call).map_or("<missing>", String::as_str);
                // From where they part.
                let at = answer
                    .bytes()
                    .zip(mine.bytes())
                    .take_while(|(a, b)| a == b)
                    .count()
                    .saturating_sub(40);
                let from = |s: &str| {
                    s.get(at..)
                        .unwrap_or(s)
                        .chars()
                        .take(240)
                        .collect::<String>()
                };
                eprintln!(
                    "{call} (from byte {at})\n    client: {}\n    ours:   {}",
                    from(answer),
                    from(mine)
                );
            }
            let name = call.split('(').next().unwrap_or(call).to_string();
            *by_function.entry(name).or_default() += 1;
        }
    }
    let mut worst: Vec<_> = by_function.into_iter().collect();
    worst.sort_by_key(|a| std::cmp::Reverse(a.1));
    // `WOWLUA_ANSWERS_TOP=1000` lists more of them.
    let top = std::env::var("WOWLUA_ANSWERS_TOP")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(40);
    let summary: Vec<String> = worst
        .iter()
        .take(top)
        .map(|(n, c)| format!("{c:5} {n}"))
        .collect();
    eprintln!(
        "{matched} of {} answered as the client did; the most missed:\n{}",
        theirs.len(),
        summary.join("\n")
    );
    // The checks (rules the call form cannot ask) as the client answered
    // them; addresses compared by their form.
    let f = vm
        .load(
            b"local out = {} for k, v in pairs(StdTestState.engineChecks) do out[k] = v end return out",
            "=checks",
        )
        .expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let our_checks: BTreeMap<String, String> = vm
        .heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(vm.str_bytes(v)).into_owned(),
            )),
            _ => None,
        })
        .collect();
    let src = fs::read(repo().join("crates/wowlua-engine/tests/client/forever-camelot-checks.lua"))
        .expect("the checks");
    let f = fixture_vm.load(&src, "=checks").expect("compiles");
    let r = fixture_vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let their_checks: BTreeMap<String, String> = fixture_vm
        .heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(fixture_vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(fixture_vm.str_bytes(v)).into_owned(),
            )),
            _ => None,
        })
        .collect();
    // Not yet answered as the client does, with the reason.
    const KNOWN: &[&str] = &[
        // Differs between the client's sessions (380, 500, then 820) though
        // it lists 20: no stable answer to give.
        "GetNumModifiedClickActions",
    ];
    let hex = regex::Regex::new(r"[0-9a-f]{10}").unwrap();
    let mut differ = Vec::new();
    for (label, answer) in &their_checks {
        if KNOWN.contains(&label.as_str()) {
            continue;
        }
        let ours = our_checks.get(label).map_or("<missing>", String::as_str);
        if hex.replace_all(ours, "ADDR") != hex.replace_all(answer, "ADDR") {
            differ.push(format!("{label}: client {answer:?}, ours {ours:?}"));
        }
    }
    assert!(differ.is_empty(), "checks differ:\n{}", differ.join("\n"));
    assert!(
        matched >= ANSWERS_MATCHED,
        "{matched} of {} answered as the client did (was {ANSWERS_MATCHED})",
        theirs.len()
    );
}

/// Classic's quest objective functions (`GetNumQuestLeaderBoards`,
/// `GetQuestLogLeaderBoard`), which the engine derives from the recorded
/// quest log, answer as the client did for every entry of the recorded
/// character's log (`/stdtest snapshot`'s `leaderBoards`).
#[test]
fn quest_objectives_derive_as_the_client_gave_them() {
    let path = repo().join("crates/wowlua-engine/tests/client/forever-camelot-character.json");
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("the profile")).expect("JSON");
    let Some(recorded) = json["leaderBoards"].as_array() else {
        panic!("the profile records the leader boards");
    };
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let controls = wowlua_engine::controls::table(&mut vm);
    wowlua_test::profile::apply(&mut vm, controls, &path).expect("the profile applies");
    let as_text = |v: &serde_json::Value| match v {
        serde_json::Value::Null => "nil".to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    };
    let mut expected = Vec::new();
    for (i, entry) in recorded.iter().enumerate() {
        expected.push(format!("{} {}", i + 1, entry["count"]));
        for board in entry["boards"].as_array().into_iter().flatten() {
            let parts: Vec<String> = board
                .as_array()
                .into_iter()
                .flatten()
                .map(as_text)
                .collect();
            expected.push(parts.join(" | "));
        }
    }
    let src = format!(
        "local out = {{}}
for i = 1, {} do
    local n = GetNumQuestLeaderBoards(i)
    out[#out + 1] = i .. ' ' .. n
    for j = 1, n do
        local text, kind, finished = GetQuestLogLeaderBoard(j, i)
        out[#out + 1] = tostring(text) .. ' | ' .. tostring(kind) .. ' | ' .. tostring(finished)
    end
end
return table.concat(out, '\\n')",
        recorded.len()
    );
    let f = vm.load(src.as_bytes(), "=objectives").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let ours = text(&vm, r[0]);
    assert_eq!(ours.lines().collect::<Vec<_>>(), expected);
}

/// The cursor (`/stdtest engine cursor`): each step's state and the events
/// between, as the client recorded them for the recorded character.
#[test]
fn the_cursor_moves_as_the_client_recorded() {
    let dir = build();
    let out = dir.path().join("dist/dev/StdTest");
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let mut vm = client_in(vm, &out);
    let controls = wowlua_engine::controls::table(&mut vm);
    wowlua_test::profile::apply(
        &mut vm,
        controls,
        &repo().join("crates/wowlua-engine/tests/client/forever-camelot-character.json"),
    )
    .expect("the character's profile");
    addon(&mut vm, "StdTestEngineProbe('cursor')");
    wowlua_engine::clock::advance_frames(&mut vm, 90, 1000.0 / 60.0);
    let f = vm
        .load(
            b"return table.concat(StdTestState.engineCursor, '\\n')",
            "=cursor",
        )
        .expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let ours = text(&vm, r[0]);
    let src = fs::read(repo().join("crates/wowlua-engine/tests/client/forever-camelot-cursor.lua"))
        .expect("the fixture");
    let mut fixture_vm = Vm::new();
    let f = fixture_vm.load(&src, "=fixture").expect("compiles");
    let r = fixture_vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let mut lines: Vec<(String, String)> = fixture_vm
        .heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                String::from_utf8_lossy(fixture_vm.str_bytes(k)).into_owned(),
                String::from_utf8_lossy(fixture_vm.str_bytes(v)).into_owned(),
            )),
            _ => None,
        })
        .collect();
    lines.sort();
    let theirs: Vec<String> = lines.into_iter().map(|(_, v)| v).collect();
    assert_eq!(ours.lines().collect::<Vec<_>>(), theirs);
}
