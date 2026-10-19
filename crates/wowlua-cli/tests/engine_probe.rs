//! `/stdtest engine` in the engine: StdTest's probe battery
//! (`tests/ingame/src/engineprobe.wowlua`), built with stand-ins for its
//! state and harness modules, runs in wowlua's VM with camelot's engine;
//! its results must be what WoW Forever recorded
//! (`crates/wowlua-engine/tests/client/forever-camelot-engine.lua`), but
//! for what depends on the character, the moment or Blizzard's UI code.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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
        "traits.wowlua",
        "tradeskill.wowlua",
        "achievements.wowlua",
        "trainer.wowlua",
        "group.wowlua",
        "startup.wowlua",
        "scenario.wowlua",
        "snapshot.wowlua",
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
        // The class's: the engine's per class, checked by
        // `the_hidden_group_buffs_are_the_recorded_class_s`.
        "probe.loadCalls.C_UnitAuras.GetHiddenGroupBuffs",
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

/// The engine's hidden group buffs for the battery's character's class are
/// the ones the client gave it (`data/camelot-hidden-group-buffs.txt`).
#[test]
fn the_hidden_group_buffs_are_the_recorded_class_s() {
    let theirs = fixture();
    let class = theirs["probe.units.UnitClass(player)"]
        .split(", ")
        .nth(1)
        .and_then(|c| c.strip_prefix("string "))
        .expect("the class")
        .to_string();
    let ids = |s: &str| -> std::collections::BTreeSet<String> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|x| !x.is_empty())
            .map(String::from)
            .collect()
    };
    let recorded = &theirs["probe.loadCalls.C_UnitAuras.GetHiddenGroupBuffs"];
    let recorded: std::collections::BTreeSet<String> = recorded
        .split("number ")
        .skip(1)
        .filter_map(|x| x.split(|c: char| !c.is_ascii_digit()).next())
        .map(String::from)
        .collect();
    let data =
        fs::read_to_string(repo().join("crates/wowlua-engine/data/camelot-hidden-group-buffs.txt"))
            .expect("the data");
    let ours = data
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .find(|(c, _)| *c == class)
        .map(|(_, list)| ids(list))
        .unwrap_or_default();
    assert_eq!(ours, recorded, "{class}");
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

/// The session's clock (`GetTime()`) as the recording began: recorded
/// (`startTime`), else the first global cooldown's start where it is read
/// (the frame the cast began; recordings before `startTime`), else the
/// player's first cast's start as its `UNIT_SPELLCAST_START` reads it.
fn session_start(recording: &str) -> Option<f64> {
    let recorded = regex::Regex::new(r#"\["startTime"\] = "([0-9.]+)""#).unwrap();
    if let Some(c) = recorded.captures(recording) {
        return c[1].parse().ok();
    }
    let gcd = regex::Regex::new(
        r#"= "([0-9.]+) = C_ActionBar\.GetActionCooldown\(\d+\) -> 1: \{activeCategory=number 133,[^}]*isOnGCD=boolean true,[^}]*startTime=number ([0-9.]+),"#,
    )
    .unwrap();
    let cast = regex::Regex::new(
        r#"= "([0-9.]+) = UnitCastingInfo\(\\"player\\"\) -> 11: string [^,]*, string [^,]*, number [0-9]+, number ([0-9]+),"#,
    )
    .unwrap();
    let first = |re: &regex::Regex, scale: f64| {
        re.captures_iter(recording)
            .filter_map(|c| Some((c[1].parse::<f64>().ok()?, c[2].parse::<f64>().ok()?)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(at, started)| started / scale - at)
    };
    first(&gcd, 1.0).or_else(|| first(&cast, 1000.0))
}

/// A scenario (`/stdtest engine scenario <name>`) played in the engine:
/// Blizzard's UI loaded, the character's world as the recording began, the
/// scenario's play script (`tests/scenarios/<name>.lua`) run with the
/// engine's controls while StdTest's recorder runs as in the client. The
/// recorder's log and the client's, each line's time aside, the noise of
/// the world around (nameplates, the cursor, the mouse) and the
/// recordings' own mistakes left out; with Blizzard's errors.
fn play_scenario(name: &str) -> Option<(Vec<String>, Vec<String>, Vec<String>)> {
    let ui = wowlua_test::ui_source_in(&repo().join("target/types-src"), "camelot")?;
    let dir = build();
    let out = dir.path().join("dist/dev/StdTest");
    let fixtures = repo().join("crates/wowlua-engine/tests/client/scenarios");
    let recording = fs::read_to_string(fixtures.join(format!("{name}.lua"))).unwrap_or_default();
    // The session's clock as the client's (its answers' times are the
    // session's): begun as long before the recording as logging in and
    // ten frames take.
    let session_start = session_start(&recording);
    let mut vm = Vm::wow();
    if let Some(start) = session_start {
        let before = wowlua_engine::login::duration(false) + 10.0 / 60.0;
        vm.clock.elapsed_ms = (start - before) * 1000.0;
    }
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let report = wowlua_engine::ui::load(&mut vm, &ui.interface, ui.strings.as_deref());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut vm = client_in(vm, &out);
    let controls = wowlua_engine::controls::table(&mut vm);
    wowlua_test::profile::apply(
        &mut vm,
        controls,
        &fixtures.join(format!("{name}-before.json")),
    )
    .expect("the character's world");
    // Logged in as the client logs in (the recorded login: the chat
    // windows set up, …), as `wowlua test` does.
    wowlua_engine::login::login(&mut vm, "StdTest", true, false);
    wowlua_engine::clock::advance_frames(&mut vm, 10, 1000.0 / 60.0);
    if let Some(start) = session_start {
        let now = wowlua_engine::clock::now(&vm);
        assert!(
            (now - start).abs() < 1e-6,
            "the session's clock at the recording's start: {now}, not {start}"
        );
    }
    let errors_before = wowlua_engine::engine(&vm).world.ui.errors.len();
    let refusals_before = wowlua_engine::engine(&vm).refusals.len();
    addon(&mut vm, &format!("StdTestEngineProbe('scenario {name}')"));
    // What the UI shows, checked by the play script: `check(label, ok)`
    // collects the failures, `note(...)` what it saw.
    addon(
        &mut vm,
        "ScenarioFailures, ScenarioNotes = {}, {}
function check(label, ok) if not ok then ScenarioFailures[#ScenarioFailures + 1] = label end end
function note(...) local t = {} for i = 1, select('#', ...) do t[i] = tostring((select(i, ...))) end ScenarioNotes[#ScenarioNotes + 1] = table.concat(t, ' ') end",
    );
    // The recording's ping (`GetNetStats`' world latency, ms): the server's
    // answers come a round trip later; a play script may set its own.
    if let Some(ms) = regex::Regex::new(r#"\["latency"\] = "[^,]*, [^,]*, [0-9.]+, ([0-9.]+)""#)
        .unwrap()
        .captures(&recording)
        .and_then(|c| c[1].parse::<f64>().ok())
    {
        addon(&mut vm, &format!("__engine.setLatency({})", ms / 1000.0));
    }
    let script =
        fs::read_to_string(repo().join(format!("crates/wowlua-cli/tests/scenarios/{name}.lua")))
            .expect("the play script");
    // A full collection every N frames and before each other step of time
    // (`WOWLUA_SCENARIO_GC=N`, 1 by default): what the engine holds
    // unrooted goes stale.
    if let Ok(every) = std::env::var("WOWLUA_SCENARIO_GC") {
        let every: usize = every.parse().unwrap_or(1).max(1);
        addon(
            &mut vm,
            &format!(
                "local e = __engine local frames, time = e.advanceFrames, e.advanceTime \
                 e.advanceFrames = function(n, ms) \
                     local i = 0 \
                     while i < n do collectgarbage('collect') \
                         local k = math.min({every}, n - i) frames(k, ms) i = i + k end end \
                 e.advanceTime = function(...) collectgarbage('collect') return time(...) end"
            ),
        );
    }
    addon(&mut vm, &script);
    addon(&mut vm, "StdTestEngineProbe('scenario stop')");
    let f = vm
        .load(
            format!("local out = {{}} for i, v in ipairs(StdTestState.engineScenarios[{name:?}].log) do out[i] = v end return out").as_bytes(),
            "=log",
        )
        .expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a log")
    };
    let n = vm.heap.table(t).border();
    let ours: Vec<String> = (1..=n)
        .map(|i| text(&vm, vm.heap.table(t).get(Value::Num(i as f64))))
        .collect();
    let src = fs::read(fixtures.join(format!("{name}.lua"))).expect("the recording");
    let f = vm.load(&src, "=recording").expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let mut theirs: Vec<(String, String)> = vm
        .heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(_)) => Some((text(&vm, Value::Str(k)), text(&vm, v))),
            _ => None,
        })
        .filter(|(k, _)| k.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    theirs.sort();
    let theirs: Vec<String> = theirs.into_iter().map(|(_, v)| v).collect();
    let mut errors: Vec<String> =
        wowlua_engine::engine(&vm).world.ui.errors[errors_before..].to_vec();
    // The stacks of errors from tainted code (`WOWLUA_SCENARIO_TAINT`;
    // `=all`: every error's).
    if let Ok(which) = std::env::var("WOWLUA_SCENARIO_TAINT") {
        for stack in &wowlua_engine::engine(&vm).world.ui.error_stacks {
            if which == "all" || stack.contains("tainted") {
                eprintln!("tainted error:\n{stack}");
            }
        }
    }
    for (global, label) in [
        ("ScenarioNotes", "note"),
        ("ScenarioFailures", "UI check failed"),
    ] {
        let Value::Table(t) = vm.get_global(global) else {
            continue;
        };
        let n = vm.heap.table(t).border();
        for i in 1..=n {
            let line = text(&vm, vm.heap.table(t).get(Value::Num(i as f64)));
            if label == "note" {
                eprintln!("note: {line}");
            } else {
                errors.push(format!("{label}: {line}"));
            }
        }
    }
    // An action forbidden to the addon: Blizzard's popup (a blocked one
    // in combat is the recorder's own, as recorded).
    for r in &wowlua_engine::engine(&vm).refusals[refusals_before..] {
        if r.event == "ADDON_ACTION_FORBIDDEN" {
            errors.push(format!("forbidden: {} {}", r.addon, r.action));
            if std::env::var("WOWLUA_SCENARIO_TAINT").is_ok() {
                eprintln!("forbidden {}:\n{}", r.action, r.report);
            }
        }
    }
    // A script that plays only part of its recording (`ScenarioPartial =
    // "why"`) cannot end as the client did: its end's differences are notes.
    let end = end_state_differences(&vm, &fixtures.join(format!("{name}-after.json")));
    match vm.get_global("ScenarioPartial") {
        Value::Str(why) => {
            let why = text(&vm, Value::Str(why));
            for d in end {
                eprintln!("note: {d} (partial: {why})");
            }
        }
        _ => errors.extend(end),
    }
    // The calls the engine answered with defaults (`WOWLUA_SCENARIO_UNANSWERED`,
    // optionally a name's start): what to model next.
    if let Ok(prefix) = std::env::var("WOWLUA_SCENARIO_UNANSWERED") {
        let mut calls: Vec<&String> = wowlua_engine::engine(&vm)
            .defaulted_calls
            .iter()
            .filter(|c| c.starts_with(&prefix))
            .collect();
        calls.sort();
        eprintln!(
            "unanswered ({}):\n{}",
            calls.len(),
            calls
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    // Both logs with their times, for fitting a script's steps.
    if let Ok(dir) = std::env::var("WOWLUA_SCENARIO_DUMP") {
        let dir = Path::new(&dir);
        fs::write(
            dir.join(format!("{name}-engine-timed.txt")),
            ours.join("\n"),
        )
        .expect("writes");
        fs::write(
            dir.join(format!("{name}-client-timed.txt")),
            theirs.join("\n"),
        )
        .expect("writes");
    }
    let compared = |log: &[String]| -> Vec<String> { scenario_lines(log) };
    let theirs = compared(&theirs);
    let ours = asked_as_recorded(compared(&ours), &theirs);
    Some((ours, theirs, errors))
}

/// The engine's answers less those the recording's recorder did not ask:
/// a call after an event the client's log has, but never after it (the
/// recorder's asks added since, e.g. `UnitOnTaxi` on `UNIT_FLAGS`).
fn asked_as_recorded(ours: Vec<String>, theirs: &[String]) -> Vec<String> {
    fn pairs(log: &[String]) -> Vec<(String, String)> {
        let mut event = String::new();
        let mut out = Vec::new();
        for l in log {
            match l.strip_prefix("= ") {
                Some(answer) => {
                    let call = answer.split_once(" -> ").map_or(answer, |(c, _)| c);
                    out.push((event.clone(), call.to_string()));
                }
                None => event = l.split('(').next().unwrap_or(l).to_string(),
            }
        }
        out
    }
    let asked: std::collections::HashSet<(String, String)> = pairs(theirs).into_iter().collect();
    let events: std::collections::HashSet<&str> = theirs
        .iter()
        .filter(|l| !l.starts_with("= "))
        .map(|l| l.split('(').next().unwrap_or(l))
        .collect();
    let mut event = String::new();
    ours.into_iter()
        .filter(|l| match l.strip_prefix("= ") {
            Some(answer) => {
                let call = answer.split_once(" -> ").map_or(answer, |(c, _)| c);
                !events.contains(event.as_str())
                    || asked.contains(&(event.clone(), call.to_string()))
            }
            None => {
                event = l.split('(').next().unwrap_or(l).to_string();
                true
            }
        })
        .collect()
}

/// Where the engine's world at a scenario's end differs from the client's
/// (its `-after.json` snapshot): the money, the experience, each bag
/// slot's item and count.
fn end_state_differences(vm: &Vm, after: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(after) else {
        return vec![];
    };
    let json: serde_json::Value = serde_json::from_str(&text).expect("a profile");
    let e = wowlua_engine::engine(vm);
    let mut out = Vec::new();
    let money = json["money"].as_f64().unwrap_or(0.0);
    if e.world.character.money != money {
        out.push(format!(
            "end: money {} (client {money})",
            e.world.character.money
        ));
    }
    if let Some(xp) = json["stats"]["xp"][0].as_f64() {
        let ours = match e.world.character.stats.get("xp") {
            Some(wowlua_engine::stats::Entry::One(a)) => match a.first() {
                Some(wowlua_engine::stats::Answer::Num(n)) => Some(*n),
                _ => None,
            },
            _ => None,
        };
        if ours != Some(xp) {
            out.push(format!("end: xp {ours:?} (client {xp})"));
        }
    }
    if let Some(bags) = json["bags"].as_object() {
        for (bag, b) in bags {
            let bag: i64 = bag.parse().unwrap_or(0);
            // A list, or by slot (the newer snapshots).
            let items: Vec<&serde_json::Value> = match &b["items"] {
                serde_json::Value::Array(list) => list.iter().collect(),
                serde_json::Value::Object(by_slot) => {
                    let mut slots: Vec<(i64, &serde_json::Value)> = by_slot
                        .iter()
                        .map(|(k, v)| (k.parse().unwrap_or(0), v))
                        .collect();
                    slots.sort_by_key(|(k, _)| *k);
                    slots.into_iter().map(|(_, v)| v).collect()
                }
                _ => vec![],
            };
            let theirs: Vec<(i64, f64)> = items
                .iter()
                .map(|i| {
                    (
                        i["id"].as_i64().unwrap_or(0),
                        i["count"].as_f64().unwrap_or(1.0),
                    )
                })
                .collect();
            let ours: Vec<(i64, f64)> = e
                .world
                .character
                .bags
                .get(&bag)
                .map(|b| b.items.values().map(|h| (h.id, h.count)).collect())
                .unwrap_or_default();
            if ours != theirs {
                out.push(format!("end: bag {bag} {ours:?} (client {theirs:?})"));
            }
        }
    }
    out
}

/// A scenario log's line as compared: without its time; none for the
/// world's noise and the recordings' own mistakes.
/// A log's lines as compared: each `scenario_line`'s, the answers to the
/// player's moving left out with it (`UnitOnTaxi` after
/// `PLAYER_STARTED_MOVING`, every frame while following).
fn scenario_lines(log: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut after_moving = false;
    for l in log {
        let body = l.split_once(' ').map_or(l.as_str(), |(_, b)| b);
        if body.starts_with("= ") {
            if after_moving {
                continue;
            }
        } else {
            after_moving = body.starts_with("PLAYER_STARTED_MOVING")
                || body.starts_with("PLAYER_STOPPED_MOVING");
        }
        if let Some(line) = scenario_line(l) {
            out.push(line);
        }
    }
    out
}

/// A pattern compiled on first use.
type Pattern = std::sync::LazyLock<regex::Regex>;

fn scenario_line(line: &str) -> Option<String> {
    let body = line.split_once(' ').map_or(line, |(_, b)| b);
    const NOISE: &[&str] = &[
        "NAME_PLATE_",
        "UNIT_POWER_FREQUENT",
        "SPELL_UPDATE_USABLE",
        "DAMAGE_METER_",
        "WORLD_CURSOR_",
        "CURSOR_CHANGED",
        "UPDATE_MOUSEOVER_UNIT",
        "GLOBAL_",
        "MODIFIER_STATE_CHANGED",
        "CONSOLE_MESSAGE",
        "PLAYER_SOFT_",
        "ACTION_RANGE_CHECK_UPDATE",
        "CRITERIA_UPDATE",
        "PLAYER_STARTED_",
        "PLAYER_STOPPED_",
        "ITEM_DATA_LOAD_RESULT",
        "SPELL_DATA_LOAD_RESULT",
        "QUEST_DATA_LOAD_RESULT",
        "ADDON_ACTION_BLOCKED",
        "SPELL_ACTIVATION_OVERLAY_HIDE",
        "UNIT_HAPPINESS",
    ];
    if NOISE.iter().any(|n| body.starts_with(n))
        || body.contains("nameplate")
        || body.contains("softinteract")
    {
        return None;
    }
    // The recordings' mistaken asks (fixed in the recorder since), and the
    // asks a protected call blocks in combat.
    const MISTAKEN: &[&str] = &[
        "= UnitIsEnemy(",
        "= UnitIsFriend(",
        "= UnitCanAttack(",
        "= UnitCanAssist(",
        "= UnitIsCivilian(",
        "= GetSpellCooldown(",
        "= C_Spell.GetSpellCooldown(",
        "= GetLootMethod(",
        "= C_PartyInfo.GetLootMethod(",
        "= CheckInteractDistance(",
    ];
    if MISTAKEN.iter().any(|m| body.starts_with(m)) {
        return None;
    }
    // Players' GUIDs as the recordings have them (scrubbed); a chat
    // message's line number (the session's). (Each pattern compiled once:
    // a scenario has thousands of lines.)
    static GUID: Pattern = Pattern::new(|| regex::Regex::new(r"Player-\d+-[0-9A-F]{8}").unwrap());
    static LINE: Pattern = Pattern::new(|| {
        regex::Regex::new(r"^(CHAT_MSG_.*; number )\d+(; (?:nil nil|string [^;]*); number 0;)")
            .unwrap()
    });
    let (guid, line) = (&*GUID, &*LINE);
    let body = guid.replace_all(body, "Player-0000-00000000");
    // A cast's GUID by its kind and spell (its server, map and number are
    // the session's).
    static CAST: Pattern =
        Pattern::new(|| regex::Regex::new(r"Cast-(\d+)-\d+-\d+-\d+-(\d+)-[0-9A-F]+").unwrap());
    let cast = &*CAST;
    let body = cast.replace_all(&body, "Cast-$1-$2");
    // A party's ID, the session's.
    static PARTY: Pattern = Pattern::new(|| regex::Regex::new(r"Party-\d+-\d+-[0-9A-F]+").unwrap());
    let party = &*PARTY;
    let body = party.replace_all(&body, "Party-<id>");
    // A cast bar's ID, the session's.
    static BAR: Pattern = Pattern::new(|| regex::Regex::new(r"CastBar-[0-9A-F]+").unwrap());
    let bar = &*BAR;
    let body = bar.replace_all(&body, "CastBar-<id>");
    // An item's GUID, the server's (a made item's: the session's).
    static ITEM: Pattern = Pattern::new(|| regex::Regex::new(r"Item-\d+-0-[0-9A-F]{16}").unwrap());
    let item = &*ITEM;
    let body = item.replace_all(&body, "Item-<id>");
    // A value's address, the session's (the ingest drops the client's).
    static ADDRESS: Pattern = Pattern::new(|| {
        regex::Regex::new(r"(function|table|userdata|thread): 0x[0-9a-f]+").unwrap()
    });
    let body = ADDRESS.replace_all(&body, "$1");
    let body = line.replace(&body, "${1}<line>${2}").into_owned();
    // An answer's trailing nils dropped (the first recordings' recorder
    // lost them).
    if let Some((call, answer)) = body.split_once(" -> ")
        && let Some((n, values)) = answer.split_once(": ")
        && n.parse::<usize>().is_ok()
    {
        let mut list: Vec<&str> = if values.is_empty() {
            vec![]
        } else {
            values.split(", ").collect()
        };
        while list.last() == Some(&"nil nil") {
            list.pop();
        }
        return Some(format!("{call} -> {}: {}", list.len(), list.join(", ")));
    }
    Some(body)
}

/// The longest common subsequence of two logs: the lines in both, in
/// order; and those only in each.
fn align(ours: &[String], theirs: &[String]) -> (usize, Vec<String>, Vec<String>) {
    let (n, m) = (ours.len(), theirs.len());
    // u16: a scenario's log stays below 65536 lines.
    let mut table = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if ours[i] == theirs[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let (mut only_ours, mut only_theirs) = (Vec::new(), Vec::new());
    while i < n && j < m {
        if ours[i] == theirs[j] {
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            only_ours.push(format!("{i:5} {}", ours[i]));
            i += 1;
        } else {
            only_theirs.push(format!("{j:5} {}", theirs[j]));
            j += 1;
        }
    }
    only_ours.extend((i..n).map(|i| format!("{i:5} {}", ours[i])));
    only_theirs.extend((j..m).map(|j| format!("{j:5} {}", theirs[j])));
    (table[0][0] as usize, only_ours, only_theirs)
}

/// How many of the combat scenario's lines the engine plays as the client
/// did: only ever up.
const COMBAT_MATCHED: usize = 733;

/// The combat scenario: a wolf targeted, fought, killed and looted.
#[test]
fn the_engine_plays_combat_as_the_client_did() {
    plays_as_the_client_did("combat", COMBAT_MATCHED);
}

/// How many of the death scenario's lines the engine plays as the client
/// did: only ever up.
const DEATH_MATCHED: usize = 1398;

/// The death scenario: a fight lost, the spirit released, the corpse taken.
#[test]
fn the_engine_plays_death_as_the_client_did() {
    plays_as_the_client_did("death", DEATH_MATCHED);
}

/// How many of the flight scenario's lines the engine plays as the client
/// did: only ever up.
const FLIGHT_MATCHED: usize = 259;

/// The flight scenario: a flight master's gossip, the taxi map looked at.
#[test]
fn the_engine_plays_flight_as_the_client_did() {
    plays_as_the_client_did("flight", FLIGHT_MATCHED);
}

/// How many of the auradeath scenario's lines the engine plays as the
/// client did: only ever up.
const AURADEATH_MATCHED: usize = 992;

/// The auradeath scenario: a fight with a seal, Consecration, a stun, a
/// Judgement's kill; the stunned cat's death taking its auras.
#[test]
fn the_engine_plays_auradeath_as_the_client_did() {
    plays_as_the_client_did("auradeath", AURADEATH_MATCHED);
}

/// How many of the gossip2 scenario's lines the engine plays as the client
/// did: only ever up.
const GOSSIP2_MATCHED: usize = 166;

/// The gossip2 scenario: a merchant's gossip, his goods browsed, his gossip
/// again, closed with Escape.
#[test]
fn the_engine_plays_gossip2_as_the_client_did() {
    plays_as_the_client_did("gossip2", GOSSIP2_MATCHED);
}

/// How many of the trainer scenario's lines the engine plays as the client
/// did: only ever up.
const TRAINER_MATCHED: usize = 171;

/// The trainer scenario: its gossip, its services, one bought.
#[test]
fn the_engine_plays_trainer_as_the_client_did() {
    plays_as_the_client_did("trainer", TRAINER_MATCHED);
}

/// How many of the vendor scenario's lines the engine plays as the client
/// did: only ever up.
const VENDOR_MATCHED: usize = 536;

/// The vendor scenario: goods sold, bought, bought back; too little money;
/// a repair.
#[test]
fn the_engine_plays_vendor_as_the_client_did() {
    plays_as_the_client_did("vendor", VENDOR_MATCHED);
}

/// How many of the hearth scenario's lines the engine plays as the client
/// did: only ever up. Not yet: the inventory's packets after the loading
/// screen (one per item, in the server's order: the items' GUIDs, not in
/// the snapshot), `Blizzard_PlayerChoice` loading at the first loading
/// screen, the active talent config, the account's Edit Mode layouts, the
/// aura instance IDs (not in the snapshot).
const HEARTH_MATCHED: usize = 1395;

/// The hearth scenario: a Hearthstone cast, the loading screen.
#[test]
fn the_engine_plays_hearth_as_the_client_did() {
    plays_as_the_client_did("hearth", HEARTH_MATCHED);
}

/// How many of the taxi scenario's lines the engine plays as the client
/// did: only ever up.
const TAXI_MATCHED: usize = 1228;

/// The taxi scenario: a flight from Orgrimmar to the Crossroads.
#[test]
fn the_engine_plays_taxi_as_the_client_did() {
    plays_as_the_client_did("taxi", TAXI_MATCHED);
}

/// How many of the mail scenario's lines the engine plays as the client
/// did: only ever up.
const MAIL_MATCHED: usize = 369;

/// The mail scenario: the inbox, an item taken, a letter sent.
#[test]
fn the_engine_plays_mail_as_the_client_did() {
    plays_as_the_client_did("mail", MAIL_MATCHED);
}

/// How many of the bank scenario's lines the engine plays as the client
/// did: only ever up.
const BANK_MATCHED: usize = 553;

/// The bank scenario: items moved in and out, a tab bought.
#[test]
fn the_engine_plays_bank_as_the_client_did() {
    plays_as_the_client_did("bank", BANK_MATCHED);
}

/// How many of the auction scenario's lines the engine plays as the
/// client did: only ever up.
const AUCTION_MATCHED: usize = 365;

/// The auction scenario: browsing, buying a commodity, posting an item.
#[test]
fn the_engine_plays_auction_as_the_client_did() {
    plays_as_the_client_did("auction", AUCTION_MATCHED);
}

/// How many of the group scenario's lines the engine plays as the client
/// did: only ever up.
const GROUP_MATCHED: usize = 251;

/// The group scenario: an invitation, a party, following, a trade.
#[test]
fn the_engine_plays_group_as_the_client_did() {
    plays_as_the_client_did("group", GROUP_MATCHED);
}

/// How many of the inspect scenario's lines the engine plays as the
/// client did: only ever up.
const INSPECT_MATCHED: usize = 283;

/// The inspect scenario: another player's gear and talents.
#[test]
fn the_engine_plays_inspect_as_the_client_did() {
    plays_as_the_client_did("inspect", INSPECT_MATCHED);
}

/// How many of the talents scenario's lines the engine plays as the client
/// did: only ever up.
const TALENTS_MATCHED: usize = 69;

/// The talents scenario: a class talent bought and applied.
#[test]
fn the_engine_plays_talents_as_the_client_did() {
    plays_as_the_client_did("talents", TALENTS_MATCHED);
}

/// How many of the profession scenario's lines the engine plays as the
/// client did: only ever up.
const PROFESSION_MATCHED: usize = 2141;

/// The profession scenario: a vein mined, a profession's window, crafts.
#[test]
fn the_engine_plays_profession_as_the_client_did() {
    plays_as_the_client_did("profession", PROFESSION_MATCHED);
}

/// How many of the legacy scenario's lines the engine plays as the client
/// did: only ever up.
const LEGACY_MATCHED: usize = 119;

/// The legacy scenario: the legacy system's tree, five ranks bought and
/// applied.
#[test]
fn the_engine_plays_legacy_as_the_client_did() {
    plays_as_the_client_did("legacy", LEGACY_MATCHED);
}

/// How many of the items scenario's lines the engine plays as the client
/// did: only ever up.
const ITEMS_MATCHED: usize = 350;

/// The items scenario: gear worn and taken off by right-click and drag,
/// a two-handed weapon refused (bags full), food eaten, an item deleted.
#[test]
fn the_engine_plays_items_as_the_client_did() {
    plays_as_the_client_did("items", ITEMS_MATCHED);
}

/// How many of the quest scenario's lines the engine plays as the client
/// did: only ever up.
const QUEST_MATCHED: usize = 4979;

/// The quest scenario: a quest taken, eight wolves hunted for its item (a
/// level gained), the quest turned in.
#[test]
fn the_engine_plays_quest_as_the_client_did() {
    plays_as_the_client_did("quest", QUEST_MATCHED);
}

/// A scenario played: no error of Blizzard's, every UI check passed, the
/// end as the client's, and at least `ratchet` of its lines as the
/// client's (`WOWLUA_SCENARIO_LIMIT`: how many differences to print;
/// `WOWLUA_SCENARIO_EVENTS`: the engine's events; `WOWLUA_SCENARIO_DUMP`:
/// a directory for both logs).
fn plays_as_the_client_did(name: &str, ratchet: usize) {
    let Some((ours, theirs, errors)) = play_scenario(name) else {
        eprintln!("skipped: no Blizzard UI code (run `cargo xtask types fetch`)");
        return;
    };
    let (matched, only_ours, only_theirs) = align(&ours, &theirs);
    let limit: usize = std::env::var("WOWLUA_SCENARIO_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    eprintln!(
        "{matched} of {} client lines matched ({} engine lines)\nonly the client's:\n{}\nonly the engine's:\n{}",
        theirs.len(),
        ours.len(),
        only_theirs
            .iter()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
        only_ours
            .iter()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
    );
    // Both logs as compared, for a diff (`WOWLUA_SCENARIO_DUMP=<dir>`).
    if let Ok(dir) = std::env::var("WOWLUA_SCENARIO_DUMP") {
        let dir = Path::new(&dir);
        fs::write(dir.join(format!("{name}-engine.txt")), ours.join("\n")).expect("writes");
        fs::write(dir.join(format!("{name}-client.txt")), theirs.join("\n")).expect("writes");
    }
    if std::env::var("WOWLUA_SCENARIO_EVENTS").is_ok() {
        eprintln!(
            "the engine's events:\n{}",
            ours.iter()
                .filter(|l| !l.starts_with("= "))
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    assert!(errors.is_empty(), "Blizzard's errors: {errors:#?}");
    assert!(
        matched >= ratchet,
        "{matched} of the {name} scenario's lines played as the client did (was {ratchet})"
    );
}

/// How many of the client's answers (`/stdtest engine answers`) the engine
/// gives as well: the calls Blizzard's UI made that the engine could not
/// answer, asked again with the recorded character's world. Only ever
/// raised (decision: the engine answers every call as the client does).
const ANSWERS_MATCHED: usize = 5924;

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

/// How many of the client's map answers (`/stdtest engine maps`: each
/// map's data and what it holds) the engine gives as well, the probe run
/// in the engine with the recorded character's world. Only ever raised.
const MAPS_MATCHED: usize = 1272;

#[test]
fn the_engine_answers_the_maps_as_the_client_did() {
    let dir = build();
    let out = dir.path().join("dist/dev/StdTest");
    let mut vm = client(&out);
    let controls = wowlua_engine::controls::table(&mut vm);
    wowlua_test::profile::apply(
        &mut vm,
        controls,
        &repo().join("crates/wowlua-engine/tests/client/forever-camelot-character.json"),
    )
    .expect("the character's profile");
    addon(&mut vm, "StdTestEngineProbe('maps')");
    let f = vm
        .load(
            b"local out = {} for k, v in pairs(StdTestState.engineMapAnswers) do out[k] = v end return out",
            "=maps",
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
            (Value::Str(k), Value::Str(v)) => {
                Some((text(&vm, Value::Str(k)), text(&vm, Value::Str(v))))
            }
            _ => None,
        })
        .collect();
    let src = fs::read(repo().join("crates/wowlua-engine/tests/client/forever-camelot-maps.lua"))
        .expect("the fixture");
    let mut fixture_vm = Vm::new();
    let f = fixture_vm.load(&src, "=fixture").expect("compiles");
    let r = fixture_vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let theirs: Vec<(String, String)> = fixture_vm
        .heap
        .table(t)
        .entries()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(k), Value::Str(v)) => Some((
                text(&fixture_vm, Value::Str(k)),
                text(&fixture_vm, Value::Str(v)),
            )),
            _ => None,
        })
        .collect();
    let missed: Vec<String> = theirs
        .iter()
        .filter(|(call, answer)| ours.get(call) != Some(answer))
        .map(|(call, answer)| {
            let mine = ours.get(call).map_or("<not asked>", String::as_str);
            format!("{call}\n    client: {answer}\n    ours:   {mine}")
        })
        .collect();
    let matched = theirs.len() - missed.len();
    eprintln!(
        "{matched} of {} map answers as the client gave them; missed:\n{}",
        theirs.len(),
        missed
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        matched >= MAPS_MATCHED,
        "{matched} of {} map answers as the client gave them (was {MAPS_MATCHED})",
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

/// The cursor recording's character: its own snapshot once the probe
/// records one (`forever-camelot-cursor-character.json`), else the shared
/// one it was recorded with.
fn cursor_character() -> PathBuf {
    let client = repo().join("crates/wowlua-engine/tests/client");
    let own = client.join("forever-camelot-cursor-character.json");
    if own.is_file() {
        own
    } else {
        client.join("forever-camelot-character.json")
    }
}

/// The cursor (`/stdtest engine cursor`): each step's state and the events
/// between, as the client recorded them for its character.
#[test]
fn the_cursor_moves_as_the_client_recorded() {
    let dir = build();
    let out = dir.path().join("dist/dev/StdTest");
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let mut vm = client_in(vm, &out);
    let controls = wowlua_engine::controls::table(&mut vm);
    wowlua_test::profile::apply(&mut vm, controls, &cursor_character())
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

fn yes(_vm: &mut Vm, _c: wowlua_vm::NativeCall) -> wowlua_vm::stdlib::Ret {
    Ok(vec![Value::Bool(true)])
}

/// Blizzard's UI logs in without an error with tutorials on (a fresh
/// account's: `GetTutorialsEnabled()`, `showNPETutorials`): the new player
/// experience loads, as it did on such an account's client (recorded: no
/// Blizzard error), though its keyboard frame's mixin is missing from the
/// client's files.
#[test]
fn blizzard_ui_logs_in_with_tutorials_on() {
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
    let tutorials = vm.native("GetTutorialsEnabled", yes);
    vm.set_global("GetTutorialsEnabled", tutorials);
    addon(&mut vm, "__engine.setCVar('showNPETutorials', '1')");
    let errors_before = wowlua_engine::engine(&vm).world.ui.errors.len();
    wowlua_engine::login::login(&mut vm, "StdTest", true, false);
    wowlua_engine::clock::advance_frames(&mut vm, 60, 1000.0 / 60.0);
    let errors = wowlua_engine::engine(&vm).world.ui.errors[errors_before..].to_vec();
    assert!(errors.is_empty(), "{errors:#?}");
    let f = vm
        .load(
            b"return C_AddOns.IsAddOnLoaded('Blizzard_NewPlayerExperience')",
            "=check",
        )
        .expect("compiles");
    let loaded = vm.call(f, &[]).expect("runs");
    assert!(
        matches!(loaded.first(), Some(Value::Bool(true))),
        "the new player experience loaded"
    );
}

/// The recorded group answers: each label's (`party`, `raid`) calls and
/// answers.
fn group_fixture() -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for label in ["party", "raid"] {
        let src = fs::read(repo().join(format!(
            "crates/wowlua-engine/tests/client/forever-camelot-group-{label}.lua"
        )))
        .expect("the fixture");
        let mut vm = Vm::new();
        let f = vm.load(&src, "=fixture").expect("compiles");
        let r = vm.call(f, &[]).expect("runs");
        let Value::Table(t) = r[0] else {
            panic!("a table")
        };
        let calls: BTreeMap<String, String> = vm
            .heap
            .table(t)
            .entries()
            .map(|(k, v)| (text(&vm, k), text(&vm, v)))
            .collect();
        out.insert(label.to_string(), calls);
    }
    out
}

/// `/stdtest engine group` played in the engine as the client recorded it
/// (70450): the paladin, dead, leading a party with an offline druid of the
/// user's (both on damage), then the party made a raid; every call's
/// answer the client's.
#[test]
fn the_engine_answers_the_group_as_the_client_did() {
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
        &repo().join("crates/wowlua-engine/tests/client/scenarios/group-before.json"),
    )
    .expect("the character's world");
    wowlua_engine::login::login(&mut vm, "StdTest", true, false);
    wowlua_engine::clock::advance_frames(&mut vm, 10, 1000.0 / 60.0);
    // As recorded (16:55): a nameplate shown, the player dead, the druid
    // offline, both on damage; probed as a party, made a raid, probed.
    addon(
        &mut vm,
        r#"local e = __engine
e.spawnUnit({ name = "Plainstrider", level = 12, reaction = "hostile" }):showNameplate()
e.player:setName("Player", "Lastname")
e.player:setRole("DAMAGER")
GroupDruid = e.spawnUnit({
    name = "Member1", lastName = "Member1Last", player = true, level = 21, class = "DRUID", classID = 11,
    reaction = "friendly", role = "DAMAGER", online = false,
})
e.addToGroup(GroupDruid)
e.player:die()
e.advanceFrames(10, 1000 / 60)
StdTestEngineProbe("group")
C_PartyInfo.ConvertToRaid()
e.advanceFrames(60, 1000 / 60)
StdTestEngineProbe("group")"#,
    );
    let f = vm
        .load(
            b"local out = {} for label, calls in pairs(StdTestState.engineGroupAnswers) do local t = {} for k, v in pairs(calls) do t[k] = v end out[label] = t end return out",
            "=group",
        )
        .expect("compiles");
    let r = vm.call(f, &[]).expect("runs");
    let Value::Table(t) = r[0] else {
        panic!("a table")
    };
    let mut ours: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (label, calls) in vm.heap.table(t).entries() {
        let Value::Table(calls) = calls else { continue };
        ours.insert(
            text(&vm, label),
            vm.heap
                .table(calls)
                .entries()
                .map(|(k, v)| (text(&vm, k), text(&vm, v)))
                .collect(),
        );
    }
    let mut differences = Vec::new();
    for (label, calls) in group_fixture() {
        let mine = ours.get(&label);
        for (call, answer) in calls {
            let got = mine
                .and_then(|m| m.get(&call))
                .map_or("<missing>", String::as_str);
            if got != answer {
                differences.push(format!(
                    "{label} {call}\n    client: {answer}\n    ours:   {got}"
                ));
            }
        }
    }
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
}
