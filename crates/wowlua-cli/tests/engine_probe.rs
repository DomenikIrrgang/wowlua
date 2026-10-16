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
    ];
    !SKIP.iter().any(|s| key.starts_with(s))
        && !key.ends_with(".GetDebugName")
        && !key.ends_with(".GetEffectiveScale")
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
    for (key, expected) in theirs.iter().filter(|(k, _)| compared(k)) {
        match ours.get(key) {
            Some(mine) if mine == expected => {}
            mine => differences.push(format!(
                "{key}\n    client: {expected}\n    ours:   {}",
                mine.map_or("<missing>", String::as_str)
            )),
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} differ:\n{}",
        differences.len(),
        theirs.keys().filter(|k| compared(k)).count(),
        differences.join("\n")
    );
}
