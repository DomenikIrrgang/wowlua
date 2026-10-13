//! End-to-end tests of the `wowlua` binary (milestone M0 exit criterion: an
//! existing Lua addon can be adopted, built for several flavors and
//! installed).

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn wowlua(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", dir.join(".wowlua-home"))
        .env_remove("WOWLUA_WOW_PATH");
    cmd
}

fn write(root: &Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}

/// An addon as it exists before wowlua: two TOCs, an embedded library loaded
/// through XML, a nested XML include, an asset and an in-game path.
fn legacy_addon(root: &Path) {
    write(
        root,
        &[
            (
                "MyAddon.toc",
                "## Interface: 110200\n## Title: My Addon\n## Author: Jane Doe\n## Version: v1.2.3\n## SavedVariables: MyAddonDB\n## OptionalDeps: Details\n## X-Curse-Project-ID: 12345\n\nembeds.xml\nCore.lua\nUI\\Frames.xml\n",
            ),
            (
                "MyAddon_Vanilla.toc",
                "## Interface: 11507\n## Title: My Addon Classic\n## Version: v1.2.3\nembeds.xml\nCore.lua\nClassic.lua\n",
            ),
            (
                "embeds.xml",
                "<Ui>\n  <Script file=\"Libs\\LibStub\\LibStub.lua\"/>\n</Ui>\n",
            ),
            ("Libs/LibStub/LibStub.lua", "LibStub = LibStub or {}\n"),
            (
                "Core.lua",
                "local _, ns = ...\nns.icon = \"Interface\\\\AddOns\\\\MyAddon\\\\Media\\\\icon.tga\"\nns.frames = \"Interface\\\\AddOns\\\\MyAddon\\\\UI\\\\Frames.lua\"\n",
            ),
            (
                "UI/Frames.xml",
                "<Ui>\n  <Script file=\"Frames.lua\"/>\n</Ui>\n",
            ),
            ("UI/Frames.lua", "local _, ns = ...\nns.ready = true\n"),
            ("Classic.lua", "local _, ns = ...\nns.classic = true\n"),
            ("Media/icon.tga", "TGA"),
            (
                "Bindings.xml",
                "<Bindings>\n  <Binding name=\"MYADDON_TOGGLE\">MyAddon_Toggle()</Binding>\n</Bindings>\n",
            ),
        ],
    );
}

#[test]
fn new_package_builds_for_several_flavors() {
    let dir = tempfile::tempdir().unwrap();
    wowlua(dir.path())
        .args(["new", "kill-tracker", "--flavors", "retail,mists,vanilla"])
        .assert()
        .success();
    let project = dir.path().join("kill-tracker");
    wowlua(&project).arg("build").assert().success();

    let toc = fs::read_to_string(project.join("dist/dev/KillTracker/KillTracker.toc")).unwrap();
    assert!(
        toc.starts_with("## Interface: 120100, 50504, 11509\n"),
        "{toc}"
    );
    assert!(toc.contains("## X-Wowlua-Build: dev "));
    assert!(toc.ends_with("src\\main.lua\n"));
    assert!(project.join("dist/dev/KillTracker/LICENSES.txt").exists());
    assert!(project.join("wowlua.lock").exists());
}

#[test]
fn init_adopts_an_existing_addon() {
    let dir = tempfile::tempdir().unwrap();
    let addon = dir.path().join("MyAddon");
    legacy_addon(&addon);

    wowlua(&addon)
        .arg("init")
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "6 files moved into src/, 1 in-game path updated",
        ));
    assert!(addon.join("src/Libs/LibStub/LibStub.lua").exists());
    assert!(addon.join("Media/icon.tga").exists());
    let core = fs::read_to_string(addon.join("src/Core.lua")).unwrap();
    assert!(core.contains(r"MyAddon\\src\\UI\\Frames.lua"));
    assert!(core.contains(r"MyAddon\\Media\\icon.tga"));

    // `OptionalDeps: Details` only appears in the Retail TOC, so it becomes a
    // flavor-specific dependency. Conditions on `##` lines need 12.0.7+
    // clients, which the Retail beta client is not, so auto picks per-flavor
    // TOCs.
    let manifest = fs::read_to_string(addon.join("wowlua.json")).unwrap();
    assert!(manifest.contains("\"name\": \"Details\""), "{manifest}");

    wowlua(&addon).arg("build").assert().success();
    let dist = addon.join("dist/dev/MyAddon");
    let mainline = fs::read_to_string(dist.join("MyAddon_Mainline.toc")).unwrap();
    let classic = fs::read_to_string(dist.join("MyAddon_Classic.toc")).unwrap();
    // init pins the interface numbers the TOCs list (`types sync` moves
    // them on).
    assert!(mainline.starts_with("## Interface: 110200\n"), "{mainline}");
    assert!(mainline.contains("## Version: 1.2.3-dev\n"));
    assert!(mainline.contains("## OptionalDeps: Details\n"));
    assert!(mainline.contains("## X-Curse-Project-ID: 12345\n"));
    assert!(
        mainline.ends_with("src\\embeds.xml\nsrc\\Core.lua\nsrc\\UI\\Frames.xml\n"),
        "{mainline}"
    );
    assert!(classic.starts_with("## Interface: 11507\n"), "{classic}");
    assert!(
        classic.contains("## Title: My Addon Classic\n"),
        "{classic}"
    );
    assert!(mainline.contains("## Title: My Addon\n"), "{mainline}");
    assert!(manifest.contains("\"flavor-overrides\""), "{manifest}");
    // Keybinding files are loaded by the client from the AddOn root.
    assert!(
        addon.join("Bindings.xml").exists(),
        "init leaves Bindings.xml in place"
    );
    assert!(dist.join("Bindings.xml").exists());
    assert!(!classic.contains("OptionalDeps"));
    assert!(
        classic.ends_with("src\\embeds.xml\nsrc\\Core.lua\nsrc\\Classic.lua\n"),
        "{classic}"
    );
    assert!(
        !mainline.contains("LibStub"),
        "XML-loaded files are not listed in the TOC"
    );
    for file in [
        "src/embeds.xml",
        "src/Libs/LibStub/LibStub.lua",
        "src/UI/Frames.lua",
        "Media/icon.tga",
    ] {
        assert!(dist.join(file).exists(), "{file}");
    }
}

#[test]
fn install_is_safe_and_uninstall_removes_only_own_folders() {
    let dir = tempfile::tempdir().unwrap();
    let addon = dir.path().join("MyAddon");
    legacy_addon(&addon);
    wowlua(&addon).arg("init").assert().success();
    let wow = dir.path().join("World of Warcraft");
    fs::create_dir_all(wow.join("_retail_")).unwrap();
    fs::create_dir_all(wow.join("_classic_era_")).unwrap();

    wowlua(&addon)
        .args(["install", "--wow"])
        .arg(&wow)
        .assert()
        .success();
    let retail = wow.join("_retail_/Interface/AddOns/MyAddon");
    assert!(retail.join("MyAddon_Mainline.toc").exists());
    assert!(
        wow.join("_classic_era_/Interface/AddOns/MyAddon/MyAddon_Classic.toc")
            .exists()
    );

    // A folder from somewhere else is left alone.
    fs::remove_dir_all(&retail).unwrap();
    write(&retail, &[("MyAddon.toc", "## Interface: 110200\n")]);
    wowlua(&addon)
        .args(["install", "--message-format", "short", "--wow"])
        .arg(&wow)
        .assert()
        .failure()
        .stderr(predicate::str::contains("error[E4331]"));

    wowlua(&addon)
        .args(["uninstall", "--wow"])
        .arg(&wow)
        .assert()
        .success();
    assert!(retail.exists());
    assert!(!wow.join("_classic_era_/Interface/AddOns/MyAddon").exists());
}

#[test]
fn manifest_errors_fail_with_codes() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"x-y\", \"version\": \"1.0.0\" }, \"addon\": { \"titel\": \"X\" } }\n",
            ),
            ("src/main.lua", "print('hi')\n"),
        ],
    );
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "wowlua.json:1:64: error[E4002]: unknown key `titel` in `addon`",
        ));
}

#[test]
fn lua_syntax_errors_fail_the_build() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"x-y\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.lua", "local = 1\n"),
        ],
    );
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("src/main.lua:1:7: error[E0101]"));
    assert!(
        !dir.path().join("dist").exists(),
        "failed builds write nothing"
    );
}

#[test]
fn locked_fails_when_the_lockfile_would_change() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"x-y\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.lua", "print('hi')\n"),
        ],
    );
    wowlua(dir.path())
        .args(["build", "--locked", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error[E4104]"));
    wowlua(dir.path()).arg("build").assert().success();
    wowlua(dir.path())
        .args(["build", "--locked"])
        .assert()
        .success();
}

#[test]
fn explain_and_clean() {
    let dir = tempfile::tempdir().unwrap();
    wowlua(dir.path())
        .args(["explain", "e4002"])
        .assert()
        .success()
        .stdout(predicate::str::contains("E4002 (error)"));
    wowlua(dir.path())
        .args(["explain", "E9999"])
        .assert()
        .code(1);

    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"x-y\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.lua", "print('hi')\n"),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    wowlua(dir.path()).arg("clean").assert().success();
    assert!(!dir.path().join("dist").exists());
}

/// TOC file variables and bulk TOC metadata (seen in BigWigs and QuestieDB).
#[test]
fn init_handles_file_variables_and_toc_data() {
    let dir = tempfile::tempdir().unwrap();
    let addon = dir.path().join("DataAddon");
    let mut toc =
        String::from("## Interface: 120100, 11509\n## Title: Data Addon\n## Version: 2.0\n");
    for i in 0..250 {
        toc.push_str(&format!("## X-Entry-{i}: value{i}\n"));
    }
    toc.push_str("Init_[Game].lua\nLocales\\enUS.lua\nLocales\\[TextLocale].lua [AllowLoadTextLocale deDE, frFR]\n");
    write(
        &addon,
        &[
            ("DataAddon.toc", toc.as_str()),
            ("Init_Standard.lua", "print('retail')\n"),
            ("Init_Vanilla.lua", "print('era')\n"),
            ("Locales/enUS.lua", "L = {}\n"),
            ("Locales/deDE.lua", "L.x = 1\n"),
            ("Locales/frFR.lua", "L.x = 2\n"),
        ],
    );

    wowlua(&addon).arg("init").assert().success();
    let manifest = fs::read_to_string(addon.join("wowlua.json")).unwrap();
    assert!(manifest.contains("\"variable-files\""), "{manifest}");
    assert!(
        manifest.contains("src/Locales/[TextLocale].lua"),
        "{manifest}"
    );
    assert!(manifest.contains("\"toc-data\""), "{manifest}");
    assert!(
        !manifest.contains("X-Entry-1"),
        "bulk data stays out of the manifest"
    );
    assert!(addon.join("src/Init_Vanilla.lua").exists());
    assert!(addon.join("src/Locales/frFR.lua").exists());

    wowlua(&addon).arg("build").assert().success();
    let dist = addon.join("dist/dev/DataAddon");
    let built = fs::read_to_string(dist.join("DataAddon.toc")).unwrap();
    assert!(built.contains("## X-Entry-249: value249\n"), "{built}");
    assert!(built.contains("src\\Init_[Game].lua\n"), "{built}");
    assert!(
        built.contains("src\\Locales\\[TextLocale].lua [AllowLoadTextLocale deDE, frFR]\n"),
        "{built}"
    );
    for file in [
        "src/Init_Standard.lua",
        "src/Init_Vanilla.lua",
        "src/Locales/deDE.lua",
        "src/Locales/frFR.lua",
    ] {
        assert!(dist.join(file).exists(), "{file}");
    }
}

/// A load-on-demand addon whose TOC loads files stored in its main addon's
/// folder (TellMeWhen_Options, MythicDungeonTools_UI).
#[test]
fn init_handles_files_loaded_from_a_sibling_addon() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("Main");
    let options = dir.path().join("Main_Options");
    write(
        &main,
        &[
            (
                "Main.toc",
                "## Interface: 120100\n## Title: Main\n## Version: 1.0\nlibs\\libs.xml\nCore.lua\n",
            ),
            (
                "libs/libs.xml",
                "<Ui>\n  <Script file=\"LibStub.lua\"/>\n</Ui>\n",
            ),
            ("libs/LibStub.lua", "LibStub = {}\n"),
            (
                "libs/options_libs.xml",
                "<Ui>\n  <Script file=\"LibStub.lua\"/>\n  <Script file=\"AceGUI.lua\"/>\n</Ui>\n",
            ),
            ("libs/AceGUI.lua", "AceGUI = {}\n"),
            ("Core.lua", "print('core')\n"),
            ("Options/Config.lua", "print('config')\n"),
        ],
    );
    write(
        &options,
        &[
            (
                "Main_Options.toc",
                "## Interface: 120100\n## Title: Main Options\n## Version: 1.0\n## LoadOnDemand: true\n## Dependencies: Main\n..\\Main\\libs\\options_libs.xml\nBoot.lua\n..\\Main\\Options\\Config.lua\n",
            ),
            ("Boot.lua", "print('boot')\n"),
        ],
    );

    wowlua(&main).arg("init").assert().success();
    wowlua(&options).arg("init").assert().success();
    let manifest = fs::read_to_string(main.join("wowlua.json")).unwrap();
    assert!(manifest.contains("\"shared-files\""), "{manifest}");
    assert!(manifest.contains("src/Options/Config.lua"), "{manifest}");
    assert!(main.join("src/libs/options_libs.xml").exists());
    assert!(main.join("src/libs/AceGUI.lua").exists());
    let manifest = fs::read_to_string(options.join("wowlua.json")).unwrap();
    assert!(manifest.contains("\"external-files\""), "{manifest}");
    assert!(manifest.contains("\"load-on-demand\": true"), "{manifest}");

    wowlua(&main).arg("build").assert().success();
    wowlua(&options).arg("build").assert().success();
    let toc = fs::read_to_string(main.join("dist/dev/Main/Main.toc")).unwrap();
    assert!(
        !toc.contains("Config.lua") && !toc.contains("options_libs"),
        "{toc}"
    );
    assert!(main.join("dist/dev/Main/src/Options/Config.lua").exists());
    assert!(main.join("dist/dev/Main/src/libs/AceGUI.lua").exists());
    let toc = fs::read_to_string(options.join("dist/dev/Main_Options/Main_Options.toc")).unwrap();
    assert!(
        toc.contains("..\\Main\\src\\libs\\options_libs.xml\nsrc\\Boot.lua\n..\\Main\\src\\Options\\Config.lua\n"),
        "{toc}"
    );
}

/// Folder names decide the addon name; an existing `src/` folder, `true`
/// flags, addon-wide game types, the `[Directive: values]` form and inline
/// directives in XML (seen in LittleWigs, RealUI, AllTheThings, Total RP 3).
#[test]
fn init_handles_unusual_layouts() {
    let dir = tempfile::tempdir().unwrap();
    let addon = dir.path().join("!Pack_Classic");
    write(
        &addon,
        &[
            (
                "!pack_classic.toc",
                "## Interface: 120100, 11509, 50504\n## AllowLoadGameType: classic\n## Title: Pack\n## Version: 1.0\n## LoadOnDemand: true\nsrc\\Core.lua\ndata.xml\nEra.lua [AllowLoadGameType: vanilla]\n",
            ),
            ("src/Core.lua", "print('core')\n"),
            ("src/Unused.lua", "error('never loaded')\n"),
            (
                "data.xml",
                "<Ui>\n  <Script file=\"Data_Mists.lua [AllowLoadGameType mists]\"/>\n  <Script file=\"data_era.lua [AllowLoadGameType: vanilla]\"/>\n</Ui>\n",
            ),
            ("Data_Mists.lua", "print('mists')\n"),
            ("Data_Era.lua", "print('era')\n"),
            ("Era.lua", "print('era')\n"),
        ],
    );

    wowlua(&addon).arg("init").assert().success();
    let manifest = fs::read_to_string(addon.join("wowlua.json")).unwrap();
    assert!(
        manifest.contains("\"name\": \"!Pack_Classic\""),
        "{manifest}"
    );
    assert!(manifest.contains("\"src\": \"wowlua-src\""), "{manifest}");
    assert!(manifest.contains("\"load-on-demand\": true"), "{manifest}");
    assert!(
        !manifest.contains("\"standard\""),
        "classic only: {manifest}"
    );
    // The addon-wide load condition stays in the TOC (decision T122).
    assert!(
        manifest.contains("\"AllowLoadGameType\": \"classic\""),
        "{manifest}"
    );
    assert!(addon.join("wowlua-src/src/Core.lua").exists());
    assert!(addon.join("wowlua-src/Data_Era.lua").exists());
    assert!(addon.join("src/Unused.lua").exists(), "unloaded files stay");

    wowlua(&addon)
        .arg("build")
        .assert()
        .success()
        .stderr(predicate::str::contains("W4310"));
    let dist = addon.join("dist/dev/!Pack_Classic");
    let toc = fs::read_to_string(dist.join("!Pack_Classic.toc")).unwrap();
    assert!(toc.contains("## Interface: 11509, 50504\n"), "{toc}");
    // The output layout is always `src/…`.
    assert!(toc.contains("src\\src\\Core.lua\n"), "{toc}");
    assert!(
        toc.contains("src\\Era.lua [AllowLoadGameType vanilla]"),
        "{toc}"
    );
    assert!(!dist.join("src/src/Unused.lua").exists());
    let xml = fs::read_to_string(dist.join("src/data.xml")).unwrap();
    assert!(
        xml.contains("Data_Mists.lua [AllowLoadGameType mists]"),
        "kept verbatim: {xml}"
    );
}

/// `.wowlua` files compile (types erased, lines preserved), with source
/// maps; features of later milestones name their milestone.
#[test]
fn wowlua_sources_compile_with_types_erased() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"typed\", \"version\": \"1.0.0\" },\n  \"profiles\": { \"dev\": { \"source-maps\": true, \"instrument\": \"none\" } } }\n",
            ),
            (
                "src/main.wowlua",
                "-- greeting\ntype Name = string\nlocal function greet(who: Name, times: number?): string\n\treturn \"hi \" .. (who :: string)\nend\nprint(greet(\"you\"))\n",
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let out = fs::read_to_string(dir.path().join("dist/dev/Typed/src/main.lua")).unwrap();
    assert_eq!(
        out,
        "-- greeting\n\nlocal function greet(who, times)\n\treturn \"hi \" .. (who)\nend\nprint(greet(\"you\"))\n"
    );
    let map = fs::read_to_string(dir.path().join("dist/dev/.maps/Typed/src/main.lua.map")).unwrap();
    assert!(map.contains("\"sources\":[\"src/main.wowlua\"]"), "{map}");
    assert!(
        !dir.path().join("dist/dev/Typed/.maps").exists(),
        "maps stay outside the AddOn"
    );

    fs::write(dir.path().join("src/main.wowlua"), "@trace\nclass C\nend\n").unwrap();
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "src/main.wowlua:1:1: error[E3021]: `@trace` cannot be applied to classes",
        ));
}

#[test]
fn check_and_fmt() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"fmt-me\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/main.wowlua",
                "local t={a=1,b='x'} -- keep\nif t.a then print( t.b ) end\n",
            ),
            ("src/plain.lua", "local   x=1\n"),
        ],
    );
    wowlua(dir.path())
        .arg("check")
        .assert()
        .success()
        .stderr(predicate::str::contains("Checked"));
    assert!(!dir.path().join("dist").exists(), "check writes nothing");

    wowlua(dir.path())
        .args(["fmt", "--check"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("would reformat src/main.wowlua"));
    wowlua(dir.path()).arg("fmt").assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("src/main.wowlua")).unwrap(),
        "local t = { a = 1, b = \"x\" } -- keep\nif t.a then\n\tprint(t.b)\nend\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("src/plain.lua")).unwrap(),
        "local   x=1\n",
        ".lua files are formatted only with fmt.lua"
    );
    wowlua(dir.path())
        .args(["fmt", "--check"])
        .assert()
        .success();
}

/// Loads a built AddOn into the embedded Lua 5.1 VM like WoW does: the
/// TOC's Lua files in order, each with `(addonName, addonTable)` as `...`.
/// Returns what it printed.
fn run_addon(dir: &Path, name: &str) -> String {
    run_addon_as(dir, name, None)
}

/// Like [`run_addon`], as the client of `flavor` (game type, family) would
/// load a single TOC: lines whose `[AllowLoadGameType …]` excludes the
/// flavor are skipped.
fn run_addon_as(dir: &Path, name: &str, flavor: Option<(&str, &str)>) -> String {
    run_toc(dir, &format!("{name}.toc"), name, flavor)
}

/// Loads the Lua files of one TOC file (see [`run_addon`]).
fn run_toc(dir: &Path, toc_file: &str, name: &str, flavor: Option<(&str, &str)>) -> String {
    let toc = fs::read_to_string(dir.join(toc_file)).unwrap();
    let lua = mlua::Lua::new();
    lua.load(
        r##"__out = {}
        print = function(...)
          local parts = {}
          for i = 1, select("#", ...) do parts[i] = tostring((select(i, ...))) end
          __out[#__out + 1] = table.concat(parts, " ")
        end"##,
    )
    .exec()
    .unwrap();
    let ns = lua.create_table().unwrap();
    for line in toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        if let (Some((game, family)), Some(cond)) =
            (flavor, line.split("[AllowLoadGameType ").nth(1))
        {
            let allowed: Vec<&str> = cond
                .trim_end_matches(']')
                .split(',')
                .map(str::trim)
                .collect();
            if !allowed.contains(&game) && !allowed.contains(&family) {
                continue;
            }
        }
        let path = line.split(" [").next().unwrap().replace('\\', "/");
        if !path.ends_with(".lua") {
            continue;
        }
        let code = fs::read(dir.join(&path)).unwrap();
        let chunk = lua.load(code).set_name(format!("@{path}"));
        if let Err(e) = chunk.call::<()>((name, ns.clone())) {
            panic!("{path}: {e}");
        }
    }
    let out: mlua::Table = lua.globals().get("__out").unwrap();
    out.sequence_values::<String>()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn modules_build_and_run_in_load_order() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"mods\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/core/math.wowlua",
                "export const SCALE = 2\nexport function clamp(v: number, lo: number, hi: number): number\n\treturn math.max(lo, math.min(hi, v))\nend\nexport local counter = { n = 0 }\nexport type Num = number\n",
            ),
            (
                "src/main.wowlua",
                "import clamp, SCALE as S, counter from \"/core/math\"\nimport type Num from \"/core/math\"\nimport * as Str from \"./util/str\"\nimport round from \"/legacy\"\ncounter.n += 1\nlocal x: Num = clamp(5, 0, 1)\nprint(x, S, Str.shout(\"hi\"), counter.n, round(2.6))\n",
            ),
            (
                "src/util/str.wowlua",
                "import counter from \"/core/math\"\nexport function shout(s: string): string\n\tcounter.n += 1\n\treturn s:upper() .. \"!\"\nend\n",
            ),
            (
                "src/legacy.lua",
                "local _, ns = ...\nns.__wowlua.define(\"legacy\", { round = function(x) return math.floor(x + 0.5) end })\n",
            ),
            (
                "src/legacy.d.wowlua",
                "declare module \"/legacy\"\n\tfunction round(x: number): number\nend\n",
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let dist = dir.path().join("dist/dev/Mods");
    let toc = fs::read_to_string(dist.join("Mods.toc")).unwrap();
    let files: Vec<&str> = toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert_eq!(
        files,
        [
            "wowlua\\runtime.lua",
            "src\\core\\math.lua",
            "src\\legacy.lua",
            "src\\util\\str.lua",
            "src\\main.lua"
        ],
        "{toc}"
    );
    assert_eq!(run_addon(&dist, "Mods"), "1 2 HI! 2 3");
    let main = fs::read_to_string(dist.join("src/main.lua")).unwrap();
    assert_eq!(main.lines().count(), 7, "lines are preserved:\n{main}");
    assert!(
        main.contains("print(x, 2, "),
        "constants are inlined:\n{main}"
    );
}

#[test]
fn minified_builds_run_the_same() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "mini", "version": "1.0.0" },
  "minify-levels": { "small": { "extends": "compact", "passes": ["rename-locals"] } },
  "profiles": { "release": { "minify": "small", "bundle": false } } }
"#,
            ),
            (
                "src/core/math.wowlua",
                "-- Clamps a value.\nexport function clamp(value: number, lowest: number, highest: number): number\n\treturn math.max(lowest, math.min(highest, value))\nend\n",
            ),
            (
                "src/main.wowlua",
                "import clamp from \"/core/math\"\nimport build from \"wowlua:meta\"\nlocal result = clamp(5, 0, 1000000)\nprint(result, (\"%d\"):format(1), build.minify)\n",
            ),
            (
                "src/plain.lua",
                "-- A plain Lua file.\nlocal message = \"plain\"\nprint(message)\n",
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success();
    let dev = dir.path().join("dist/dev/Mini");
    let release = dir.path().join("dist/release/Mini");
    assert_eq!(run_addon(&dev, "Mini"), "5 1 none\nplain");
    assert_eq!(run_addon(&release, "Mini"), "5 1 small\nplain");
    let math = fs::read_to_string(release.join("src/core/math.lua")).unwrap();
    assert!(
        !math.contains("Clamps") && !math.contains("lowest"),
        "{math}"
    );
    let plain = fs::read_to_string(release.join("src/plain.lua")).unwrap();
    assert_eq!(plain, "\nlocal a=\"plain\"\nprint(a)\n");
    let main = fs::read_to_string(release.join("src/main.lua")).unwrap();
    assert!(main.contains("1e6"), "{main}");
    // Unbundled output keeps every line.
    assert_eq!(main.lines().count(), 4, "{main}");
    let size = |d: &Path| fs::metadata(d.join("wowlua/runtime.lua")).unwrap().len();
    assert!(size(&release) < size(&dev), "the runtime is minified too");
}

#[test]
fn module_errors() {
    let cases: &[(&str, &str, &str)] = &[
        ("import nope from \"/b\"\n", "export local a = 1\n", "E1012"),
        ("import a from \"/missing\"\n", "", "E1015"),
        ("import a from \"/B\"\n", "export local a = 1\n", "E1013"),
        (
            "import a from \"/b\"\na = 2\n",
            "export local a = 1\n",
            "E1010",
        ),
        (
            "local x = 1\nimport a from \"/b\"\n",
            "export local a = 1\n",
            "E1011",
        ),
        ("export local v = 1\nreturn v\n", "", "E1032"),
        (
            "import a from \"/b\"\nexport local m = a\n",
            "import m from \"/main\"\nexport local a = m\n",
            "E1014",
        ),
        ("import _ from \"wowlua:events\"\n", "", "E1015"),
        (
            "print(f())\nexport function f() return 1 end\n",
            "",
            "E1037",
        ),
    ];
    for (main, b, code) in cases {
        let dir = tempfile::tempdir().unwrap();
        let mut files = vec![
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"err\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", *main),
        ];
        if !b.is_empty() {
            files.push(("src/b.wowlua", *b));
        }
        write(dir.path(), &files);
        wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(format!("error[{code}]")));
    }
}

/// Build metadata, static conditions, `@if` and per-flavor output (spec
/// chapter 6, codegen §14.8.3).
#[test]
fn build_metadata_and_flavor_split_builds() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"import addon, build, flavor, config, features, asset from "wowlua:meta"
@if(flavor == "standard")
import * as retail from "./retail"
if flavor == "standard" then
	print("retail", retail.name)
elseif flavor == "vanilla" then
	print("classic")
end
print(addon.name, build.debug, config.maxHistory, features.minimap, features.comms)
print(asset("media/icon.tga"))
local function nameOf(t) return t.name end
print(#addon.authors, addon.flavors[2], nameOf(addon), addon.interface[1])
@if(build.debug)
local function debugOnly() return "dbg" end
print(build.debug and debugOnly() or "release")
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "meta-demo", "version": "1.2.3", "authors": ["Jane <j@x.org>"] },
  "flavors": { "standard": {}, "vanilla": {} },
  "build": { "assets": ["media/**"] },
  "config": { "maxHistory": 50 },
  "features": { "default": ["minimap"], "minimap": [], "comms": [] },
  "profiles": { "dev": { "config": { "maxHistory": 100 } } } }
"#,
            ),
            ("src/main.wowlua", main),
            ("src/retail.wowlua", "export local name = \"Retail!\"\n"),
            ("media/icon.tga", "tga"),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let dist = dir.path().join("dist/dev/MetaDemo");
    let toc = fs::read_to_string(dist.join("MetaDemo.toc")).unwrap();
    assert!(
        toc.contains(r"wowlua\flavors\standard\src\main.lua [AllowLoadGameType mainline]"),
        "{toc}"
    );
    assert!(
        toc.contains(r"wowlua\flavors\vanilla\src\main.lua [AllowLoadGameType classic]"),
        "{toc}"
    );
    assert!(
        toc.contains("src\\retail.lua\n"),
        "identical files are stored once: {toc}"
    );
    let standard = run_addon_as(&dist, "MetaDemo", Some(("standard", "mainline")));
    let vanilla = run_addon_as(&dist, "MetaDemo", Some(("vanilla", "classic")));
    let icon = r"Interface\AddOns\MetaDemo\media\icon.tga";
    assert_eq!(
        standard,
        format!(
            "retail Retail!\nMetaDemo true 100 true false\n{icon}\n1 vanilla MetaDemo 120100\ndbg"
        )
    );
    assert_eq!(
        vanilla,
        format!("classic\nMetaDemo true 100 true false\n{icon}\n1 vanilla MetaDemo 11509\ndbg")
    );
    let main_out = fs::read_to_string(dist.join("wowlua/flavors/vanilla/src/main.lua")).unwrap();
    assert_eq!(
        main_out.lines().count(),
        15,
        "lines are preserved:\n{main_out}"
    );
    assert!(
        !main_out.contains("retail"),
        "removed branches are not emitted:\n{main_out}"
    );

    // Release: the debug-only function and its use are gone.
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success();
    let release = dir.path().join("dist/release/MetaDemo");
    let out = run_addon_as(&release, "MetaDemo", Some(("standard", "mainline")));
    assert!(out.ends_with("release"), "{out}");
}

#[test]
fn build_metadata_errors() {
    let cases: &[(&str, &str, &str)] = &[
        ("@if(false)\nlocal x = 1\nprint(x)\n", "", "E1020"),
        ("@if(someGlobal)\nlocal x = 1\n", "", "E1040"),
        (
            "import config from \"wowlua:meta\"\nprint(config.missing)\n",
            "",
            "E2003",
        ),
        (
            "import build from \"wowlua:meta\"\nbuild.debug = true\n",
            "",
            "E2010",
        ),
        (
            "import asset from \"wowlua:meta\"\nprint(asset(\"nope.tga\"))\n",
            "",
            "E1038",
        ),
        ("import nope from \"wowlua:meta\"\n", "", "E1012"),
        (
            "import flavor from \"wowlua:meta\"\n@if(flavor == \"standard\")\nlocal x = 1\n",
            ", \"build\": { \"flavor-mode\": \"shared\" }",
            "E1039",
        ),
    ];
    for (main, extra, code) in cases {
        let dir = tempfile::tempdir().unwrap();
        let manifest = format!(
            "{{ \"package\": {{ \"name\": \"err\", \"version\": \"1.0.0\" }}, \"flavors\": {{ \"standard\": {{}}, \"vanilla\": {{}} }}{extra} }}\n"
        );
        write(
            dir.path(),
            &[
                ("wowlua.json", manifest.as_str()),
                ("src/main.wowlua", *main),
            ],
        );
        wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(format!("error[{code}]")));
    }
}

/// Flavors that need opposite orders get per-flavor TOC files (spec §5.4.6).
#[test]
fn incompatible_flavor_orders() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "orders", "version": "1.0.0" }, "flavors": { "standard": {}, "vanilla": {} } }
"#,
            ),
            (
                "src/a.wowlua",
                "import flavor from \"wowlua:meta\"\n@if(flavor == \"standard\")\nimport bv from \"./b\"\nexport local av = \"a\"\nprint(\"a\", flavor)\n",
            ),
            (
                "src/b.wowlua",
                "import flavor from \"wowlua:meta\"\n@if(flavor == \"vanilla\")\nimport av from \"./a\"\nexport local bv = \"b\"\nprint(\"b\", flavor)\n",
            ),
        ],
    );
    wowlua(dir.path())
        .arg("build")
        .assert()
        .success()
        .stderr(predicate::str::contains("W4304"));
    let dist = dir.path().join("dist/dev/Orders");
    assert_eq!(
        run_toc(&dist, "Orders_Mainline.toc", "Orders", None),
        "b standard\na standard"
    );
    assert_eq!(
        run_toc(&dist, "Orders_Classic.toc", "Orders", None),
        "a vanilla\nb vanilla"
    );
    let manifest = fs::read_to_string(dir.path().join("wowlua.json"))
        .unwrap()
        .replace("} }\n", "}, \"build\": { \"toc-layout\": \"single\" } }\n");
    fs::write(dir.path().join("wowlua.json"), manifest).unwrap();
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error[E1036]"));
}

/// Classes, enums and decorators across modules, run in the VM (spec
/// chapters 3 and 4).
#[test]
fn classes_across_modules_run() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"shapes\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/main.wowlua",
                "import Square from \"./shapes/square\"\nenum Size { Small = 1, Large = 3 }\nexport enum Color { Red = \"r\" }\n@global(\"DemoSquare\")\nlocal s = Square.new(Size.Large)\nprint(s:describe(), Color.Red, DemoSquare == s)\n",
            ),
            (
                "src/shapes/base.wowlua",
                "export abstract class Shape\n    abstract function area(): number\n    function describe(): string return `{self:name()} {self:area()}` end\n    function name(): string return \"shape\" end\nend\n",
            ),
            (
                "src/shapes/square.wowlua",
                "import Shape from \"./base\"\nexport class Square extends Shape\n    side: number\n    constructor(side: number)\n        super()\n        self.side = side\n    end\n    override function area(): number return self.side ^ 2 end\n    override function name(): string return \"square\" end\nend\n",
            ),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("warning"), "{stderr}");
    let dist = dir.path().join("dist/dev/Shapes");
    let toc = fs::read_to_string(dist.join("Shapes.toc")).unwrap();
    let runtime = fs::read_to_string(dist.join("wowlua/runtime.lua")).unwrap();
    assert!(
        runtime.contains("function wl.class(") && !runtime.contains("wl.widgetClass"),
        "slices are tree-shaken"
    );
    assert_eq!(run_addon(&dist, "Shapes"), "square 9 r true", "{toc}");
}

/// Bundling (spec optimization §15.3): the TOC's Lua files become segments
/// around XML files; each file runs protected, so an error stops only it.
#[test]
fn bundled_builds_run_the_same() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "bun", "version": "1.0.0" },
  "build": { "load-order": ["src/first.lua", "src/embeds.xml", "src/broken.lua", "src/frames.xml", "src/main.wowlua"] } }
"#,
            ),
            (
                "src/first.lua",
                "local _, ns = ...\nns.order = { \"first\" }\nns.where = debugstack and debugstack(1)\n",
            ),
            (
                "src/broken.lua",
                "local _, ns = ...\nns.order[#ns.order + 1] = \"broken\"\nerror(\"boom\")\n",
            ),
            (
                "src/frames.xml",
                "<Ui><Frame name=\"BunFrame\"/><Script file=\"framecode.lua\"/></Ui>\n",
            ),
            // Only loads Lua: flattened into the bundle (decision T167).
            (
                "src/embeds.xml",
                "<Ui>\n<Script file=\"embedded.lua\"/>\n</Ui>\n",
            ),
            (
                "src/embedded.lua",
                "local _, ns = ...\nns.order[#ns.order + 1] = \"embedded\"\n",
            ),
            (
                "src/framecode.lua",
                "local _, ns = ...\nns.order[#ns.order + 1] = \"xml\"\n",
            ),
            (
                "src/main.wowlua",
                "import build from \"wowlua:meta\"\nlocal _, ns = ...\nns.order[#ns.order + 1] = \"main\"\nprint(table.concat(ns.order, \" \"), build.bundled)\n",
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    wowlua(dir.path())
        .args(["build", "--release", "--stats"])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("warning[W6001]")
                .and(predicate::str::contains("= in src/first.lua")),
        )
        .stderr(predicate::str::contains("Stats").and(predicate::str::contains("3 Lua files")));
    let release = dir.path().join("dist/release/Bun");
    let toc = fs::read_to_string(release.join("Bun.toc")).unwrap();
    let files: Vec<&str> = toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert_eq!(files, ["Bun.lua", "src\\frames.xml", "Bun.2.lua"], "{toc}");
    assert!(!release.join("src/first.lua").exists());
    assert!(release.join("src/framecode.lua").exists(), "XML loads it");
    let out = run_addon(&release, "Bun");
    assert!(out.contains("Bun (src/broken, build "), "{out}");
    // The test runner does not run XML scripts.
    assert!(out.ends_with("first embedded broken main true"), "{out}");
    assert!(!release.join("src/embeds.xml").exists() && !release.join("src/embedded.lua").exists());
    // The source map leads back to the files.
    let map = fs::read_to_string(dir.path().join("dist/release/.maps/Bun/Bun.lua.map")).unwrap();
    assert!(map.contains("src/broken.lua"), "{map}");
}

/// `dead-code` (spec optimization §15.2.1): side-effect-free modules nothing
/// uses go (W6002); modules with effects, imported ones and `build.keep`
/// stay.
#[test]
fn dead_code_removes_unused_modules() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "dead", "version": "1.0.0" },
  "build": { "keep": ["kept"] },
  "profiles": { "release": { "bundle": false } } }
"#,
            ),
            (
                "src/unused.wowlua",
                "export function helper(): number return 1 end\nlocal function inner() end\n",
            ),
            ("src/kept.wowlua", "export const VALUE = 2\n"),
            (
                "src/effects.wowlua",
                "export local x = 1\nprint(\"effects\")\n",
            ),
            (
                "src/used.wowlua",
                "export function twice(n: number): number return n * 2 end\n",
            ),
            (
                "src/main.wowlua",
                "import twice from \"./used\"\nlocal function neverCalled() return 3 end\nprint(twice(2))\n",
            ),
        ],
    );
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("warning[W6002]")
                .and(predicate::str::contains("= in src/unused.wowlua")),
        )
        .stderr(predicate::str::contains("src/kept.wowlua").not())
        .stderr(predicate::str::contains("src/effects.wowlua").not());
    let release = dir.path().join("dist/release/Dead");
    assert!(!release.join("src/unused.lua").exists());
    assert!(release.join("src/kept.lua").exists());
    assert!(release.join("src/effects.lua").exists());
    let toc = fs::read_to_string(release.join("Dead.toc")).unwrap();
    assert!(!toc.contains("unused"), "{toc}");
    let main = fs::read_to_string(release.join("src/main.lua")).unwrap();
    assert!(
        !main.contains("neverCalled"),
        "unused local functions go too:\n{main}"
    );
    let out = run_addon(&release, "Dead");
    assert!(out.ends_with('4'), "{out}");
}

/// `optimize.inline` (spec optimization §15.2): expression helpers are
/// inlined; `@inline` on anything else is W3031.
#[test]
fn release_builds_inline_expression_helpers() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "inl", "version": "1.0.0" },
  "profiles": { "release": { "bundle": false } } }
"#,
            ),
            (
                "src/main.wowlua",
                "local function half(n: number): number return n / 2 end\n@inline\nlocal function countdown(n: number): number\n    if n <= 0 then return 0 end\n    return countdown(n - 1)\nend\nlocal size = 10\nprint(half(size), countdown(3))\n",
            ),
        ],
    );
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "warning[W3031]: `@inline` is ignored: its body is not a single `return` expression",
        ));
    let release = dir.path().join("dist/release/Inl");
    let main = fs::read_to_string(release.join("src/main.lua")).unwrap();
    assert!(main.contains("print(size / 2,"), "{main}");
    assert!(!main.contains("half"), "{main}");
    assert_eq!(run_addon(&release, "Inl"), "5 0");
}

/// `rename-private` (spec optimization §15.4.1): private and protected
/// methods get short names unless their names occur anywhere else.
#[test]
fn rename_private_renames_only_provably_private_methods() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "priv", "version": "1.0.0" },
  "profiles": { "release": { "minify": "max", "bundle": false } } }
"#,
            ),
            (
                "src/base.wowlua",
                "export class Counter\n    total: number = 0\n    protected function incrementTotal(by: number) self.total = self.total + by end\n    private function describeTotal(): string return `total {self.total}` end\n    private function lookedUpByName(): string return \"dynamic\" end\n    function run(other: Counter): string\n        self:incrementTotal(2)\n        other:incrementTotal(1)\n        return self:describeTotal() .. \" \" .. self[\"lookedUpByName\"](self)\n    end\nend\n",
            ),
            (
                "src/main.wowlua",
                "import Counter from \"./base\"\nclass Double extends Counter\n    protected override function incrementTotal(by: number) self.total = self.total + by * 2 end\nend\nlocal d = Double.new()\nprint(d:run(Counter.new()))\n",
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success();
    let release = dir.path().join("dist/release/Priv");
    assert_eq!(
        run_addon(&dir.path().join("dist/dev/Priv"), "Priv"),
        "total 4 dynamic"
    );
    assert_eq!(run_addon(&release, "Priv"), "total 4 dynamic");
    let base = fs::read_to_string(release.join("src/base.lua")).unwrap();
    let main = fs::read_to_string(release.join("src/main.lua")).unwrap();
    assert!(
        !base.contains("incrementTotal") && !base.contains("describeTotal"),
        "{base}"
    );
    assert!(
        !main.contains("incrementTotal"),
        "the override is renamed alike:\n{main}"
    );
    // Its name is in a string: it stays.
    assert!(base.contains("lookedUpByName"), "{base}");
}

/// Standard-library infrastructure (spec §8.1, compiler §13.2): `wowlua:*`
/// modules compile into the addon when imported, and `@intrinsic`
/// declarations give framework decorators their behavior.
#[test]
fn standard_library_modules_and_intrinsics() {
    let stdlib = tempfile::tempdir().unwrap();
    write(
        stdlib.path(),
        &[
            (
                "fixture.wowlua",
                "import helper from \"wowlua:helper\"\nexport function wrap(tag: string)\n    return function(fn, ctx) return function(...) return tag .. fn(...) end end\nend\nexport function register(name: string)\n    return function(fn, ctx) print(\"registered\", name, ctx.name, fn()) end\nend\nexport local version = helper()\n",
            ),
            (
                "fixture.d.wowlua",
                "@intrinsic(\"internal.wrap\")\ndeclare function wrap(tag: string): any\n@intrinsic(\"internal.register\")\ndeclare function register(name: string): any\n",
            ),
            ("helper.wowlua", "export function helper() return 7 end\n"),
            ("unused.wowlua", "print(\"never loaded\")\n"),
        ],
    );
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"std\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/main.wowlua",
                "import wrap, register, version from \"wowlua:fixture\"\n@register(\"r\")\n@wrap(\"<\")\nlocal function f() return \"x\" end\nprint(f(), version)\n",
            ),
        ],
    );
    wowlua(dir.path())
        .env("WOWLUA_STDLIB_DIR", stdlib.path())
        .arg("build")
        .assert()
        .success();
    let dist = dir.path().join("dist/dev/Std");
    let toc = fs::read_to_string(dist.join("Std.toc")).unwrap();
    let files: Vec<&str> = toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    let pos = |name: &str| {
        files
            .iter()
            .position(|f| f.replace('\\', "/") == name)
            .unwrap_or_else(|| panic!("{name}: {toc}"))
    };
    assert!(
        pos("wowlua/std/helper.lua") < pos("wowlua/std/fixture.lua"),
        "{toc}"
    );
    assert!(pos("wowlua/std/fixture.lua") < pos("src/main.lua"), "{toc}");
    assert!(
        !toc.contains("unused") && !dist.join("wowlua/std/unused.lua").exists(),
        "tree-shaken: {toc}"
    );
    assert_eq!(run_addon(&dist, "Std"), "registered r f <x\n<x 7");
    // Unknown modules, and `@intrinsic` outside the standard library.
    fs::write(
        dir.path().join("src/main.wowlua"),
        "import x from \"wowlua:nope\"\n",
    )
    .unwrap();
    wowlua(dir.path())
        .env("WOWLUA_STDLIB_DIR", stdlib.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "unknown standard-library module `wowlua:nope`",
        ));
    fs::write(dir.path().join("src/main.wowlua"), "print(1)\n").unwrap();
    fs::write(
        dir.path().join("src/api.d.wowlua"),
        "@intrinsic(\"x\")\ndeclare function f(): number\n",
    )
    .unwrap();
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("E3033"));
}

/// `wowlua:debug` (spec §7.1): folded helpers, source locations and text,
/// profiler-backed timing, and removal with the arguments by profile.
#[test]
fn debug_module() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"import assert, dump, location, nameof, typeName, measure, stats, watch from "wowlua:debug"
import * as debug from "wowlua:debug"

local Tracker = { total = 0 }
local function sideEffect(): number
    print("evaluated")
    return 1
end
local function onLogin(amount: number)
    assert(amount > 0, "positive")
    dump(amount + 1, Tracker)
    print(location(), nameof(Tracker.total), typeName(amount), debug.typeName(Tracker))
    local doubled = measure("double", function(x: number): number return x * 2 end, amount)
    print(doubled, stats().double ~= nil)
    watch("total", function() return sideEffect() end)
    print(pcall(function() debug.assert(sideEffect() > 1, "boom") end))
end
onLogin(3)
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"dbg\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    assert_eq!(
        run_addon(&dir.path().join("dist/dev/Dbg"), "Dbg"),
        [
            "src/main.wowlua:11 (onLogin) amount + 1 = 4",
            "src/main.wowlua:11 (onLogin) Tracker = { total = 0 }",
            "src/main.wowlua:12 (onLogin) total number { total: number }",
            "6 true",
            "evaluated",
            "false Dbg: src/main.wowlua:16 (onLogin): assertion failed: boom",
        ]
        .join("\n")
    );
    // Release: assertions, dumps and watches are removed with their
    // arguments (`sideEffect()` never runs), `measure` is a direct call.
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success();
    assert_eq!(
        run_addon(&dir.path().join("dist/release/Dbg"), "Dbg"),
        [
            "src/main.wowlua:12 (onLogin) total number { total: number }",
            "6 false",
            "true"
        ]
        .join("\n")
    );
    // Statement-only helpers as values, `nameof` of an expression, `todo`.
    fs::write(
        dir.path().join("src/main.wowlua"),
        "import assert, nameof, todo from \"wowlua:debug\"\nlocal ok = assert(true)\nprint(ok, nameof(1 + 2))\nlocal function f() todo() end\nf()\n",
    )
    .unwrap();
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(
            predicate::str::contains("error[E3045]").and(predicate::str::contains("error[E7001]")),
        );
}

/// `wowlua:log` (spec stdlib "log", §7.2): level calls below the profile's
/// log level are removed with their arguments; kept ones carry their
/// source location in debug builds and go to the ring buffer and sinks.
#[test]
fn log_module() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"import log, warn, history, setLevel, sink, print as chat from "wowlua:log"
import * as Log from "wowlua:log"

local function costly(): string
    _G.print("evaluated")
    return "state"
end
sink(function(level: string, message: string) _G.print("sink", level, message) end)
log.trace(costly())
log.debug("cache", 42, true)
Log.info("ready")
warn("low", "mana")
setLevel("error")
log.info("hidden")
for _, e in ipairs(history()) do
    _G.print(e.level, e.message, e.location)
end
chat("Hello")
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"logs\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    // dev: log-level "debug" (trace calls removed, `costly()` never runs).
    assert_eq!(
        run_addon(&dir.path().join("dist/dev/Logs"), "Logs"),
        [
            "sink debug cache 42 true",
            "sink info ready",
            "|cff33ff99Logs|r |cffffb23f[warn]|r low mana",
            "sink warn low mana",
            "debug cache 42 true src/main.wowlua:10",
            "info ready src/main.wowlua:11",
            "warn low mana src/main.wowlua:12",
            "|cff33ff99Logs|r: Hello",
        ]
        .join("\n")
    );
    // release: log-level "warn", no locations.
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success();
    assert_eq!(
        run_addon(&dir.path().join("dist/release/Logs"), "Logs"),
        [
            "|cff33ff99Logs|r |cffffb23f[warn]|r low mana",
            "sink warn low mana",
            "warn low mana nil",
            "|cff33ff99Logs|r: Hello",
        ]
        .join("\n")
    );
    // A level function as a value cannot be stripped (E8118).
    fs::write(
        dir.path().join("src/main.wowlua"),
        "import info from \"wowlua:log\"\nlocal f = info\nf(\"x\")\n",
    )
    .unwrap();
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error[E8118]"));
}

/// Profiling and tracing (spec §7.3, §7.4): instrument levels, `@profile`,
/// `@noprofile`, class methods, plain Lua files in instrumented profiles,
/// error counting and `@trace`.
#[test]
fn profiling_and_tracing() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = r#"{ "package": { "name": "prof", "version": "1.0.0" },
  "profiles": {
    "decorated": { "inherits": "dev", "instrument": "decorated" },
    "exports": { "inherits": "dev", "instrument": "exports" },
    "traced": { "inherits": "test", "instrument": "none" }
  } }
"#;
    let main = r#"import stats from "wowlua:debug"
import add from "./math"

class Counter
    n: number = 0
    function bump(): number
        self.n += 1
        return self.n
    end
end

local function helper(x: number): number return x + 1 end

@noprofile
local function hidden() end

@profile({ errors = true })
local function risky(fail: boolean)
    if fail then error("boom", 0) end
end

@trace
local function traced(a: number, b: string): number return a * 2 end

local c = Counter.new()
c:bump()
c:bump()
print(helper(1), add(2, 3), hidden(), traced(4, "x"))
risky(false)
print(pcall(risky, true))
legacy()
local names = {}
for name, s in pairs(stats()) do
    names[#names + 1] = name .. "=" .. s.calls .. (s.errors > 0 and ("!" .. s.errors) or "")
end
table.sort(names)
print(table.concat(names, " "))
"#;
    write(
        dir.path(),
        &[
            ("wowlua.json", manifest),
            (
                "src/math.wowlua",
                "export function add(a: number, b: number): number return a + b end\nexport function unused() end\n",
            ),
            ("src/main.wowlua", main),
            ("src/legacy.lua", "function legacy() return 1 end\n"),
        ],
    );
    let run = |profile: &str, dist: &str| -> String {
        wowlua(dir.path())
            .args(["build", "--profile", profile])
            .assert()
            .success();
        run_addon(&dir.path().join(format!("dist/{dist}/Prof")), "Prof")
    };
    // dev: every named function, plain Lua too; `@noprofile` excluded.
    assert_eq!(
        run("dev", "dev"),
        "2 5 nil 8\nfalse boom\nCounter.new=1 Counter:bump=2 add=1 helper=1 legacy=1 risky=2!1 traced=1"
    );
    // decorated: only `@profile`.
    assert_eq!(
        run("decorated", "decorated"),
        "2 5 nil 8\nfalse boom\nrisky=2!1"
    );
    // exports: plus exported functions and public methods of exported classes.
    assert_eq!(
        run("exports", "exports"),
        "2 5 nil 8\nfalse boom\nadd=1 risky=2!1"
    );
    // A trace-level profile: `@trace` logs entry and exit.
    assert_eq!(
        run("traced", "traced"),
        "Prof: > traced(4, \"x\")\nProf: < traced = 8\n2 5 nil 8\nfalse boom\n"
    );
    // The plain Lua file keeps its lines and gets its wrapper on its `end` line.
    wowlua(dir.path()).arg("build").assert().success();
    let legacy = fs::read_to_string(dir.path().join("dist/dev/Prof/src/legacy.lua")).unwrap();
    assert!(
        legacy.ends_with("function legacy() return 1 end legacy = __wl.prof.wrap(legacy, \"legacy\", \"src/legacy.lua:1\")\n"),
        "{legacy}"
    );
}

/// Runtime type checks (spec §7.5): entry checks of exported functions,
/// public methods of exported classes and `@checked` functions in `dev`
/// (`exports`), every annotated function and `::` in `test` (`all`).
#[test]
fn runtime_checks() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"export class Tracker
    total: number = 0
    function record(amount: number, note: string?)
        self.total += amount
    end
end

export enum Side { Left = "LEFT", Right = "RIGHT" }

export interface Point
    x: number
    y: number
end

export function place(side: Side, p: Point, count: number = 1, ...: number): Side
    return side
end

@checked
local function strict(t: Tracker) end

@unchecked
export function loose(x: number) end

local function inner(x: number) end

local function show(ok: boolean, message: any)
    print(ok, message)
end
local t = Tracker.new()
show(pcall(t.record, t, "5"))
show(pcall(t.record, t, 5, 7))
show(pcall(place, "UP", { x = 1, y = 2 }))
show(pcall(place, "LEFT", { x = 1 }))
show(pcall(place, "LEFT", { x = 1, y = 2 }, nil, 1, "two"))
show(pcall(place, "RIGHT", { x = 1, y = 2 }))
show(pcall(strict, {}))
show(pcall(loose, "x"))
show(pcall(inner, "x"))
show(pcall(function() return (_G.missing :: number) end))
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"chk\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    assert_eq!(
        run_addon(&dir.path().join("dist/dev/Chk"), "Chk"),
        [
            "false Chk: src/main.wowlua:3: bad argument #1 'amount' to 'Tracker:record' (number expected, got string)",
            "false Chk: src/main.wowlua:3: bad argument #2 'note' to 'Tracker:record' (string? expected, got number)",
            "false Chk: src/main.wowlua:15: bad argument #1 'side' to 'place' (Side expected, got string)",
            "false Chk: src/main.wowlua:15: bad argument #2 'p' to 'place' (Point expected, got table)",
            "false Chk: src/main.wowlua:15: bad argument #5 '...' to 'place' (number expected, got string)",
            "true RIGHT",
            "false Chk: src/main.wowlua:20: bad argument #1 't' to 'strict' (Tracker expected, got table)",
            "true nil",
            "true nil",
            "true nil",
        ]
        .join("\n")
    );
    // `test` checks every annotated function and `::` assertions.
    wowlua(dir.path())
        .args(["build", "--profile", "test"])
        .assert()
        .success();
    let out = run_addon(&dir.path().join("dist/test/Chk"), "Chk");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[7], "true nil", "{out}");
    assert_eq!(
        lines[8],
        "false Chk: src/main.wowlua:25: bad argument #1 'x' to 'inner' (number expected, got string)"
    );
    assert_eq!(
        lines[9],
        "false Chk: src/main.wowlua:40: bad value for '::' (number expected, got nil)"
    );
}

/// `wowlua:reflect` (spec §7.8): class identity, enum order, and the full
/// metadata of `@reflect` classes and functions.
#[test]
fn reflection() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"import * as reflect from "wowlua:reflect"

interface Named
    name: string
end

class Animal
    name: string = "?"
end

@reflect
class Dog extends Animal implements Named
    scale: number = 1
    private locked: boolean = false
    nickname?: string
    get label(): string return self.name end
    function bark(times: number, loud: boolean?): string return "woof" end
end

enum Direction { Up, Down, Left, Right }

@reflect
local function greet(who: string, times: number = 1): string return "hi " .. who end

local d = Dog.new()
print(reflect.isInstance(d, Dog), reflect.isInstance(d, Animal), reflect.className(d), reflect.className(reflect.superclass(Dog)))
print(reflect.isSubclass(Dog, Animal), reflect.isSubclass(Animal, Dog), reflect.classOf(d) == Dog)
print(reflect.enumName(Direction, 2), reflect.enumFrom(Direction, 9), reflect.enumFrom(Direction, 4))
local order = {}
for _, e in ipairs(reflect.enumEntries(Direction)) do order[#order + 1] = e.name .. "=" .. e.value end
print(table.concat(order, " "))
for _, f in ipairs(reflect.fields(Dog)) do
    print(f.name, f.type, f.visibility, f.optional, f.hasDefault)
end
local m = reflect.methods(Dog)[1]
print(m.name, m.params[1].name, m.params[2].type, m.params[2].optional, m.returns)
print(reflect.properties(Dog)[1].name, reflect.interfaces(Dog)[1], reflect.getClass("Dog") == Dog, reflect.fields(Animal))
local info = reflect.functionInfo(greet)
print(info.name, info.location, info.params[2].optional, info.returns)
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"refl\", \"version\": \"1.0.0\" }, \"profiles\": { \"bare\": { \"inherits\": \"dev\", \"reflection\": \"none\" } } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    assert_eq!(
        run_addon(&dir.path().join("dist/dev/Refl"), "Refl"),
        [
            "true true Dog Animal",
            "true false true",
            "Down nil 4",
            "Up=1 Down=2 Left=3 Right=4",
            "scale number public false true",
            "locked boolean private false true",
            "nickname string public true false",
            "bark times boolean? true string",
            "label Named true nil",
            "greet src/main.wowlua:23 true string",
        ]
        .join("\n")
    );
    // Without reflection, full-metadata calls warn (W7010) and return nil.
    wowlua(dir.path())
        .args(["build", "--profile", "bare", "--message-format", "short"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning[W7010]"));
}

/// The dev console (spec §7.7), monitor (§7.9) and error capture (§7.10)
/// are built into dev builds without code changes.
#[test]
fn dev_console_monitor_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"local addonName, ns = ...
ns.answer = 42
local frame = CreateFrame("Frame", "MonFrame")
frame:RegisterEvent("PLAYER_LOGIN")
frame:SetScript("OnEvent", function(self, event)
    ns.seen = event
end)
frame:HookScript("OnShow", function() end)
C_Timer.After(1, function() ns.timer = true end)
local function work(n) return n * 2 end
work(1)
work(2)
"#;
    let mock = r#"__chat = {}
DEFAULT_CHAT_FRAME = { AddMessage = function(_, text) __chat[#__chat + 1] = text end }
SlashCmdList = {}
local handler = function(message) __chat[#__chat + 1] = "default handler: " .. message end
function geterrorhandler() return handler end
function seterrorhandler(h) handler = h end
function debugstack() return "Interface/AddOns/Mon/src/main.lua:12: in main chunk" end
function date() return "12:00:00" end
local now = 0
function debugprofilestop() now = now + 1 return now end
C_Timer = { After = function(_, fn) fn() end, NewTicker = function() end }
local frames = {}
function CreateFrame(_, name)
  local f = { scripts = {}, name = name }
  function f:GetName() return self.name end
  function f:RegisterEvent() end
  function f:SetScript(s, fn) self.scripts[s] = fn end
  function f:HookScript(s, fn) self.scripts[s] = fn end
  frames[#frames + 1] = f
  return f
end
function __fire(event) for _, f in ipairs(frames) do if f.scripts.OnEvent then f.scripts.OnEvent(f, event) end end end
"#;
    let sim = r#"__fire("PLAYER_LOGIN")
__fire("PLAYER_LOGIN")
geterrorhandler()("Interface/AddOns/Mon/src/main.lua:9: boom")
geterrorhandler()("Interface/AddOns/Mon/src/main.lua:9: boom")
geterrorhandler()("Interface/AddOns/Other/x.lua:1: not mine")
geterrorhandler()("Mon: src/main.lua:10: bad argument #1 'n' to 'work' (number expected, got nil)")
SlashCmdList.WOWLUA("list")
SlashCmdList.WOWLUA("mon mon")
SlashCmdList.WOWLUA("prof Mon top 2")
SlashCmdList.WOWLUA("errors")
SlashCmdList.WOWLUA("watch Mon ns.answer + 1")
for _, line in ipairs(__chat) do print((line:gsub("|c%x%x%x%x%x%x%x%x", ""):gsub("|r", ""))) end
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"mon\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.lua", main),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let dist = dir.path().join("dist/dev/Mon");
    let toc = fs::read_to_string(dist.join("Mon.toc")).unwrap();
    fs::write(dist.join("mock.lua"), mock).unwrap();
    fs::write(dist.join("sim.lua"), sim).unwrap();
    fs::write(dist.join("Mon.toc"), format!("mock.lua\n{toc}\nsim.lua\n")).unwrap();
    let out = run_addon(&dist, "Mon");
    let lines: Vec<&str> = out.lines().collect();
    let has = |needle: &str| lines.iter().any(|l| l.contains(needle));
    assert!(
        has("default handler: Interface/AddOns/Mon/src/main.lua:9: boom"),
        "{out}"
    );
    assert!(has("wowlua Mon 1.0.0-dev"), "{out}");
    assert!(
        has("wowlua Mon events:")
            && has("PLAYER_LOGIN")
            && lines
                .iter()
                .any(|l| l.contains("PLAYER_LOGIN") && l.contains(" 2 ")),
        "{out}"
    );
    assert!(has("MonFrame · OnShow") || has("Mon scripts:"), "{out}");
    assert!(has("C_Timer.After at src/main.lua:9"), "{out}");
    assert!(has("work") && has("Mon profiler (ms), by total:"), "{out}");
    assert!(has("src/main.lua:9: boom ×2"), "{out}");
    assert!(!has("not mine ×"), "{out}");
    assert!(
        has("bad argument #1 'n' to 'work' (number expected, got nil) ×1"),
        "{out}"
    );
    assert!(
        has("Mon error captured: src/main.lua:9: boom (/wl errors Mon)"),
        "{out}"
    );
    assert!(has("wowlua Mon watch ns.answer + 1 = 43"), "{out}");
}

/// The dev window (spec §7.7.1) opens with `/wl` and renders every tab
/// against a permissive mock of the frame API; dev builds ship its fonts
/// and save its state per addon.
#[test]
fn dev_window_renders() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"import watch from "wowlua:debug"
import info, warn from "wowlua:log"
local addonName, ns = ...
ns.db = { profile = { scale = 1.25 }, list = { 1, 2, 3 } }
ns.db.self = ns.db
local function work(n: number): number return n * 2 end
work(1)
info("ready")
warn("careful")
watch("scale", function() return ns.db.profile.scale end)
"#;
    let mock = r#"local function object(kind)
  local o = { __kind = kind, __shown = true, __scripts = {}, __text = "" }
  return setmetatable(o, { __index = function(self, key)
    if key == "GetWidth" or key == "GetHeight" then return function() return 400 end end
    if key == "GetStringWidth" then return function() return 50 end end
    if key == "GetPoint" then return function() return "CENTER", nil, "CENTER", 0, 0 end end
    if key == "GetLeft" or key == "GetVerticalScroll" then return function() return 0 end end
    if key == "GetEffectiveScale" then return function() return 1 end end
    if key == "SetFont" then return function() return true end end
    if key == "IsShown" then return function(s) return s.__shown end end
    if key == "Show" then return function(s) s.__shown = true end end
    if key == "Hide" then return function(s)
      local was = s.__shown
      s.__shown = false
      if was and s.__scripts.OnHide then s.__scripts.OnHide(s) end
    end end
    if key == "SetShown" then return function(s, v) s.__shown = v and true or false end end
    if key == "SetScript" then return function(s, name, fn) s.__scripts[name] = fn end end
    if key == "GetScript" then return function(s, name) return s.__scripts[name] end end
    if key == "SetText" then return function(s, t) s.__text = t end end
    if key == "GetText" then return function(s) return s.__text end end
    if key == "CreateFontString" or key == "CreateTexture" then return function() return object(key) end end
    return function() end
  end })
end
UIParent = object("UIParent")
function CreateFrame(kind) return object(kind) end
function GetCursorPosition() return 10, 10 end
SlashCmdList = {}
UISpecialFrames = {}
DEFAULT_CHAT_FRAME = { AddMessage = function() end }
"#;
    let sim = r#"SlashCmdList.WOWLUA("")
local console = WOWLUA_DEV.console
assert(console.window and console.window:IsShown(), "the window opens")
for tab = 1, 7 do console.setTab(tab) end
console.window:GetScript("OnUpdate")(console.window, 1)
SlashCmdList.WOWLUA("")
print("shown", console.window:IsShown(), type(WowluaDevDB_Win))
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"win\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let dist = dir.path().join("dist/dev/Win");
    let toc = fs::read_to_string(dist.join("Win.toc")).unwrap();
    assert!(toc.contains("## SavedVariables: WowluaDevDB_Win"), "{toc}");
    assert!(dist.join("wowlua/fonts/MarcellusSC-Regular.ttf").exists());
    assert!(dist.join("wowlua/fonts/Figtree-OFL.txt").exists());
    let licenses = fs::read_to_string(dist.join("LICENSES.txt")).unwrap();
    assert!(
        licenses.contains("Marcellus SC") && licenses.contains("Figtree"),
        "{licenses}"
    );
    fs::write(dist.join("mock.lua"), mock).unwrap();
    fs::write(dist.join("sim.lua"), sim).unwrap();
    fs::write(dist.join("Win.toc"), format!("mock.lua\n{toc}\nsim.lua\n")).unwrap();
    assert_eq!(run_addon(&dist, "Win"), "shown false table");
    // Release builds have none of it.
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success();
    let release = dir.path().join("dist/release/Win");
    let toc = fs::read_to_string(release.join("Win.toc")).unwrap();
    assert!(
        !toc.contains("WowluaDevDB") && !release.join("wowlua/fonts").exists(),
        "{toc}"
    );
}

/// `wowlua trace` (spec tooling §16.4): locations from the game mapped to
/// source files, with the enclosing function.
#[test]
fn trace_decodes_locations() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"tr\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/ui/window.wowlua",
                "class Window\n    frame: any = nil\n    function refresh()\n        print(self.frame.x)\n    end\nend\nlocal function onLogin()\n    Window.new():refresh()\nend\nonLogin()\n",
            ),
            (
                "src/legacy.lua",
                "local t = {}\nfunction t.load()\n  error(\"x\")\nend\n",
            ),
        ],
    );
    let input = "Message: Interface/AddOns/Tr/src/ui/window.lua:4: attempt to index field 'frame' (a nil value)\n\
[string \"@Interface/AddOns/Tr/src/ui/window.lua\"]:8: in function <Interface/AddOns/Tr/src/ui/window.lua:7>\n\
Interface\\AddOns\\Tr\\src\\legacy.lua:3: x\n\
Interface/AddOns/Other/x.lua:1: not ours\n";
    let assert = wowlua(dir.path())
        .arg("trace")
        .write_stdin(input)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(stdout.contains("window.lua:4: attempt to index field 'frame' (a nil value)\n  → src/ui/window.wowlua:4 in Window:refresh\n"), "{stdout}");
    assert!(
        stdout.contains(
            "  → src/ui/window.wowlua:8 in onLogin\n  → src/ui/window.wowlua:7 in onLogin\n"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("  → src/legacy.lua:3 in t.load\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("not ours\n") && !stdout.contains("x.lua:1\n  →"),
        "{stdout}"
    );
}

/// `wowlua trace` through a bundle's source map, with renamed locals.
#[test]
fn trace_decodes_bundled_builds() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "tr", "version": "1.0.0" },
  "profiles": { "release": { "minify": "rename" } } }
"#,
            ),
            ("src/first.wowlua", "print(\"first\")\n"),
            (
                "src/calc.wowlua",
                "local function addOne(values: any)\n    local runningTotal = values.missing\n    return runningTotal + 1\nend\naddOne({})\n",
            ),
        ],
    );
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success();
    let bundle = fs::read_to_string(dir.path().join("dist/release/Tr/Tr.lua")).unwrap();
    let (index, line) = bundle
        .lines()
        .enumerate()
        .find(|(_, l)| l.starts_with("return") && l.ends_with("+1"))
        .unwrap();
    let short = line
        .trim_start_matches("return")
        .trim_end_matches("+1")
        .trim();
    let input = format!(
        "Tr (src/calc, build 0): Interface/AddOns/Tr/Tr.lua:{}: attempt to perform arithmetic on local '{short}' (a nil value)\n",
        index + 1
    );
    let assert = wowlua(dir.path())
        .arg("trace")
        .write_stdin(input)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        stdout.contains(&format!(
            "  → src/calc.wowlua:3 in addOne   ('{short}' is 'runningTotal')"
        )),
        "{stdout}\n{bundle}"
    );
}

/// `wowlua lint` reports the lints only; `--fix` applies their fixes;
/// `--!nolint` and `check.lints` turn them off.
#[test]
fn lint_and_fix() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = "{ \"package\": { \"name\": \"lints\", \"version\": \"1.0.0\" }, \"check\": { \"lints\": { \"W1004\": \"deny\" } } }\n";
    let main = "local unused = 1\nlocal n: number = \"type error\"\nlocal function f(a, b)\n    return a\nend\nprint(f(n))\n";
    let quiet = "--!nolint\nlocal unused = 1\n";
    write(
        dir.path(),
        &[
            ("wowlua.json", manifest),
            ("src/main.wowlua", main),
            ("src/quiet.wowlua", quiet),
        ],
    );
    // Lints only: the type error (E2001) is not shown.
    let assert = wowlua(dir.path())
        .args(["lint", "--message-format", "short"])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("main.wowlua:1:7: warning[W1003]: unused local `unused`"),
        "{stderr}"
    );
    assert!(
        stderr.contains("main.wowlua:3:21: warning[W1003]: unused parameter `b`"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("E2001") && !stderr.contains("quiet.wowlua"),
        "{stderr}"
    );
    wowlua(dir.path())
        .args(["lint", "--fix"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Fixed 2 problems in 1 file"));
    assert_eq!(
        fs::read_to_string(dir.path().join("src/main.wowlua")).unwrap(),
        "local n: number = \"type error\"\nlocal function f(a, _b)\n    return a\nend\nprint(f(n))\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("src/quiet.wowlua")).unwrap(),
        quiet
    );
    // `check.lints` severities.
    fs::write(
        dir.path().join("src/main.wowlua"),
        "local x = 1\ndo local x = 2 print(x) end\nprint(x)\n",
    )
    .unwrap();
    wowlua(dir.path())
        .args(["lint", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error[W1004]"));
}

/// `wowlua types convert` turns LuaLS annotations into declarations.
#[test]
fn types_convert() {
    let dir = tempfile::tempdir().unwrap();
    let lib = "---@meta\n---@class MyLib\n---@field version number\nlocal MyLib = {}\n\n---@param name string\n---@return boolean\nfunction MyLib:Register(name) end\n";
    write(dir.path(), &[("annotations/MyLib.lua", lib)]);
    wowlua(dir.path())
        .args(["types", "convert", "annotations", "--out", "types"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Converted 1 file → types/annotations.d.wowlua",
        ));
    let out = fs::read_to_string(dir.path().join("types/annotations.d.wowlua")).unwrap();
    assert!(
        out.contains("declare class MyLib")
            && out.contains("version: number")
            && out.contains("Register(name: string): boolean"),
        "{out}"
    );
}

/// Shared flavor mode: APIs some flavor lacks need a flavor guard (spec
/// §6.5, E2030).
#[test]
fn flavor_guards_in_shared_mode() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = "{ \"package\": { \"name\": \"guards\", \"version\": \"1.0.0\" }, \"flavors\": { \"standard\": {}, \"vanilla\": {} }, \"build\": { \"flavor-mode\": \"shared\" } }\n";
    let guarded = r#"import flavor from "wowlua:meta"
if C_Garrison then
    C_Garrison.GetGarrisonInfo(1)
end
if WOW_PROJECT_ID == WOW_PROJECT_MAINLINE then
    C_Garrison.GetGarrisonInfo(1)
end
if flavor == "standard" then
    C_Garrison.GetGarrisonInfo(1)
end
local function f()
    if not C_Garrison then return end
    C_Garrison.GetGarrisonInfo(1)
end
print(GetTime())
"#;
    write(
        dir.path(),
        &[("wowlua.json", manifest), ("src/main.wowlua", guarded)],
    );
    let assert = wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("E2030") && !stderr.contains("W1001"),
        "{stderr}"
    );
    fs::write(
        dir.path().join("src/main.wowlua"),
        "import flavor from \"wowlua:meta\"\nC_Garrison.GetGarrisonInfo(1)\n",
    )
    .unwrap();
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(
            predicate::str::contains("error[E2030]").and(predicate::str::contains("`vanilla`")),
        );
}

/// The typed sample addon checks in strict mode without diagnostics
/// (milestone M4 exit criterion) and runs on both of its flavors: the
/// mainline-only API is behind a flavor guard.
#[test]
fn typed_demo_example_runs() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/typed-demo");
    let dir = tempfile::tempdir().unwrap();
    for entry in [
        "wowlua.json",
        "src/main.wowlua",
        "src/ring.wowlua",
        "src/money.wowlua",
        "src/ledger.wowlua",
        "src/saved.d.wowlua",
    ] {
        let target = dir.path().join(entry);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(example.join(entry), &target).unwrap();
    }
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("warning"), "{stderr}");
    let dist = dir.path().join("dist/dev/TypedDemo");
    let toc = fs::read_to_string(dist.join("TypedDemo.toc")).unwrap();
    let mock = |project: u32, bank: bool| {
        format!(
            r#"WOW_PROJECT_MAINLINE = 1
WOW_PROJECT_ID = {project}
local money = 10000
function GetMoney() return money end
function __setMoney(m) money = m end
local now = 0
function GetTime() now = now + 1 return now end
SlashCmdList = {{}}
C_Timer = {{ NewTicker = function() end }}
local frames = {{}}
function CreateFrame()
  local f = {{ events = {{}} }}
  function f:RegisterEvent(e) self.events[e] = true end
  function f:SetScript(name, handler) self[name] = handler end
  frames[#frames + 1] = f
  return f
end
function __fire(event, ...)
  for _, f in ipairs(frames) do
    if f.events[event] and f.OnEvent then f.OnEvent(f, event, ...) end
  end
end
if {bank} then
  C_Bank = {{ FetchDepositedMoney = function(t) return t == 2 and 1234567 or 0 end }}
  Enum = {{ BankType = {{ Account = 2 }} }}
end
"#
        )
    };
    fs::write(dist.join("mock_standard.lua"), mock(1, true)).unwrap();
    fs::write(dist.join("mock_vanilla.lua"), mock(2, false)).unwrap();
    fs::write(
        dist.join("sim.lua"),
        r#"__fire("PLAYER_LOGIN")
__fire("LOOT_OPENED"); __setMoney(10550); __fire("PLAYER_MONEY"); __fire("LOOT_CLOSED")
__fire("MERCHANT_SHOW"); __setMoney(8000); __fire("PLAYER_MONEY")
__setMoney(9000); __fire("PLAYER_MONEY"); __fire("MERCHANT_CLOSED")
SlashCmdList.TYPEDDEMO()
"#,
    )
    .unwrap();
    let summary = [
        "Typed Demo: session 1, 1g 0s 0c. Type /tdemo.",
        "earned 0g 15s 50c, spent 0g 25s 50c, net -0g 10s 0c",
        "  + 0g 5s 50c (loot)",
        "  - 0g 25s 50c",
        "  + 0g 10s 0c (vendor)",
    ]
    .join("\n");
    for (flavor, expected) in [
        ("standard", format!("{summary}\nwarband bank: 123g 45s 67c")),
        ("vanilla", summary.clone()),
    ] {
        let toc_file = format!("TypedDemo_{flavor}_test.toc");
        fs::write(
            dist.join(&toc_file),
            format!("mock_{flavor}.lua\n{toc}\nsim.lua\n"),
        )
        .unwrap();
        assert_eq!(
            run_toc(&dist, &toc_file, "TypedDemo", None),
            expected,
            "{flavor}"
        );
    }
}

/// A mock of the frame API for widget classes in the VM.
const MOCK_FRAMES: &str = r##"UIParent = {}
local methods = {}
methods.__index = methods
function methods:SetSize(w, h) self.w, self.h = w, h end
function methods:SetScript(name, fn) self.scripts = self.scripts or {}; self.scripts[name] = fn end
function methods:SetText(t) self.text = t end
function methods:GetText() return self.text end
function methods:Click() local f = self.scripts and self.scripts.OnClick; if f then f(self) end end
function methods:GetObjectType() return self.objectType end
function CreateFrame(kind, name, parent, template)
  local f = setmetatable({ objectType = kind, template = template }, methods)
  if name then _G[name] = f end
  return f
end
function Mixin(obj, ...)
  for i = 1, select("#", ...) do for k, v in pairs((select(i, ...))) do obj[k] = v end end
  return obj
end
"##;

/// Widget classes are mixins applied to frames (spec §3.2).
#[test]
fn widget_classes_run() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"widgets\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/main.wowlua",
                r#"export class CounterButton extends Button
    count: number = 0
    constructor(parent: Frame)
        super(nil, parent, "UIPanelButtonTemplate")
        self:SetSize(120, 24)
        self:SetScript("OnClick", function() self:increment() end)
        self:refresh()
    end
    function increment()
        self.count += 1
        self:refresh()
    end
    function refresh()
        self:SetText(`Clicked {self.count}x`)
    end
end
class FancyButton extends CounterButton
    stars: number = 3
    override function refresh()
        super:refresh()
        self:SetText(self:GetText() .. " *" .. (self.stars or 0))
    end
end
@global("DemoRowMixin")
class Row extends Frame
    label: string = "row"
    function OnLoad()
        print("onload", self.label)
    end
end
class Plain extends Frame
end
local b = CounterButton.new(UIParent)
b:Click()
local f = FancyButton.new(UIParent)
f:Click()
f:Click()
print(b:GetText(), f:GetText(), f.count, f.stars, f:GetObjectType(), f.template)
local frame = CreateFrame("Frame")
Mixin(frame, DemoRowMixin)
frame:OnLoad()
local p = Plain.new("Named")
print(p:GetObjectType(), Named == p, p.__class == Plain)
"#,
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let dist = dir.path().join("dist/dev/Widgets");
    let toc = fs::read_to_string(dist.join("Widgets.toc")).unwrap();
    fs::write(dist.join("mock.lua"), MOCK_FRAMES).unwrap();
    fs::write(dist.join("Widgets.toc"), format!("mock.lua\n{toc}")).unwrap();
    assert_eq!(
        run_addon(&dist, "Widgets"),
        "Clicked 1x Clicked 2x *3 2 3 Button UIPanelButtonTemplate\nonload row\nFrame true true"
    );
    // Widget classes cannot define metamethods or accessors.
    for (src, code) in [
        (
            "class B extends Button\n function __tostring() return \"\" end\nend\n",
            "E3007",
        ),
        (
            "class B extends Frame\n get x(): number return 1 end\nend\n",
            "E3043",
        ),
    ] {
        fs::write(dir.path().join("src/main.wowlua"), src).unwrap();
        wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(code));
    }
}

/// `extends` reads the imported class while the module loads, so a cycle
/// through a run-time use elsewhere still loads the base first.
#[test]
fn class_inheritance_orders_modules() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"cycle\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/a.wowlua",
                "import Base from \"./b\"\nexport class A<T> extends Base\n    item?: T\n    override function name(): string return \"a\" end\nend\nexport function make(): A<number> return A.new() end\n",
            ),
            (
                "src/b.wowlua",
                "import make from \"./a\"\nexport class Base\n    function name(): string return \"base\" end\n    static function demo(): string return make():name() end\nend\n",
            ),
            (
                "src/main.wowlua",
                "import Base from \"./b\"\nimport _ from \"./a\"\nprint(Base.demo())\n",
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let dist = dir.path().join("dist/dev/Cycle");
    let toc = fs::read_to_string(dist.join("Cycle.toc")).unwrap();
    assert!(
        toc.find("src\\b.lua").unwrap() < toc.find("src\\a.lua").unwrap(),
        "{toc}"
    );
    assert_eq!(run_addon(&dist, "Cycle"), "a");
}

/// A fuller mock of the frame API: unknown methods are no-ops; the XML
/// template of the widget demo is simulated.
const MOCK_UI: &str = r##"UIParent = {}
UISpecialFrames = {}
SlashCmdList = {}
tinsert = table.insert
__frames = {}
local function noop() end
local methods = {}
local function object(t)
  return setmetatable(t, { __index = function(_, k)
    if methods[k] then return methods[k] end
    if type(k) == "string" and k:match("^%u") then return noop end
  end })
end
function methods:SetText(t) self._text = t end
function methods:GetText() return self._text or "" end
function methods:SetScript(name, fn) self._scripts = self._scripts or {}; self._scripts[name] = fn end
function methods:GetScript(name) return self._scripts and self._scripts[name] end
function methods:HookScript(name, fn)
  local old = self:GetScript(name)
  self:SetScript(name, function(...) if old then old(...) end fn(...) end)
end
function methods:Click() local f = self:GetScript("OnClick"); if f then f(self) end end
function methods:SetValue(v) self._value = v end
function methods:GetValue() return self._value end
function methods:SetChecked(v) self._checked = v end
function methods:GetChecked() return self._checked end
function methods:Show() self._shown = true end
function methods:Hide() self._shown = false end
function methods:SetShown(v) self._shown = v end
function methods:IsShown() return self._shown ~= false end
function methods:CreateTexture() return object({}) end
function methods:CreateFontString() return object({}) end
function methods:GetObjectType() return self._type end
function CreateFrame(kind, name, parent, template)
  local f = object({ _type = kind, _template = template, _parent = parent })
  if template == "UICheckButtonTemplate" then f.text = object({}) end
  if template == "WidgetDemoRowTemplate" then
    f.Label = object({})
    Mixin(f, WidgetDemoRowMixin)
    f:OnLoad()
  end
  if name then _G[name] = f end
  __frames[#__frames + 1] = f
  return f
end
function Mixin(obj, ...)
  for i = 1, select("#", ...) do for k, v in pairs((select(i, ...))) do obj[k] = v end end
  return obj
end
"##;

/// `@script` registers widget class methods as script handlers (spec
/// §3.2.1).
#[test]
fn script_decorator_registers_handlers() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"scripts\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/main.wowlua",
                r#"class Clicker extends Button
    clicks: number = 0
    constructor()
        super(nil, UIParent)
        -- Replaced: handlers are installed after the constructor.
        self:SetScript("OnClick", function() print("constructor handler") end)
        self:SetScript("OnShow", function() print("template OnShow") end)
    end
    @script("OnClick")
    function clicked(button: string)
        self.clicks += 1
        print("clicked", button, self.clicks)
    end
    @script("OnShow", { hook = true })
    function shown()
        print("hooked OnShow")
    end
end
class Loud extends Clicker
    override function clicked(button: string)
        super:clicked(button)
        print("loud")
    end
end
@global("ScriptsRowMixin")
class Row extends Frame
    label: string = "row"
    @script("OnEnter")
    function enter() print("enter", self.label) end
    function OnLoad() print("onload", self:GetScript("OnEnter") ~= nil) end
end
local c = Clicker.new()
c:GetScript("OnClick")(c, "LeftButton")
c:GetScript("OnShow")(c)
local l = Loud.new()
l:GetScript("OnClick")(l, "RightButton")
local f = CreateFrame("Frame")
Mixin(f, ScriptsRowMixin)
f:OnLoad()
f:GetScript("OnEnter")(f)
"#,
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let dist = dir.path().join("dist/dev/Scripts");
    let toc = fs::read_to_string(dist.join("Scripts.toc")).unwrap();
    fs::write(dist.join("mock.lua"), MOCK_UI).unwrap();
    fs::write(dist.join("Scripts.toc"), format!("mock.lua\n{toc}")).unwrap();
    assert_eq!(
        run_addon(&dist, "Scripts"),
        "clicked LeftButton 1\ntemplate OnShow\nhooked OnShow\nclicked RightButton 1\nloud\nonload true\nenter row"
    );
    for src in [
        "class A\n @script(\"OnClick\")\n function f() end\nend\n",
        "class A extends Button\n @script(\"OnClick\")\n static function f() end\nend\n",
        "class A extends Button\n @script\n function f() end\nend\n",
        "class A extends Button\n @script(\"OnShow\", { hook = 1 })\n function f() end\nend\n",
        "@script(\"OnClick\")\nlocal function f() end\n",
    ] {
        fs::write(dir.path().join("src/main.wowlua"), src).unwrap();
        wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("E3021"));
    }
}

/// The UI sample addon checks in strict mode without diagnostics (milestone
/// M4 exit criterion), builds for its flavors and works against the mock
/// frame API (the milestone M3 exit criterion is checked in game).
#[test]
fn widget_demo_example_runs() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/widget-demo");
    let dir = tempfile::tempdir().unwrap();
    for entry in [
        "wowlua.json",
        "src/main.wowlua",
        "src/ui/theme.wowlua",
        "src/ui/widgets.wowlua",
        "src/ui/row.wowlua",
        "src/ui/window.wowlua",
        "src/ui/templates.xml",
    ] {
        let target = dir.path().join(entry);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(example.join(entry), &target).unwrap();
    }
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("warning"), "{stderr}");
    let dist = dir.path().join("dist/dev/WidgetDemo");
    let toc = fs::read_to_string(dist.join("WidgetDemo.toc")).unwrap();
    assert!(toc.contains("## Interface: 120100, 16001, 11509"), "{toc}");
    fs::write(dist.join("mock.lua"), MOCK_UI).unwrap();
    fs::write(
        dist.join("sim.lua"),
        r#"local function find(template, n)
  local seen = 0
  for _, f in ipairs(__frames) do
    if f._template == template then seen = seen + 1; if seen == (n or 1) then return f end end
  end
end
SlashCmdList.WIDGETDEMO()
local w = WidgetDemoWindow
print("shown", w:IsShown(), w.__class ~= nil)
local limited = find("UIPanelButtonTemplate", 1)
for _ = 1, 3 do limited:Click() end
print(limited:GetText(), w.progress:GetValue(), w.status:GetText())
find("UIPanelButtonTemplate", 2):Click()
print(w.status:GetText())
local input = find("InputBoxTemplate")
input:SetText("hello")
input:GetScript("OnEnterPressed")(input)
input:SetText("world")
input:GetScript("OnEnterPressed")(input)
print(w.rows[1].Label:GetText(), w.rows[2].Label:GetText(), w.rows[1].index, w.status:GetText())
local toggle = find("UICheckButtonTemplate")
toggle:SetChecked(false)
toggle:Click()
print("list", w.list:IsShown(), toggle.text:GetText())
for _ = 1, 8 do limited:Click() end
print(limited:GetText())
SlashCmdList.WIDGETDEMO()
print("shown", w:IsShown())
"#,
    )
    .unwrap();
    fs::write(
        dist.join("WidgetDemo.toc"),
        format!("mock.lua\n{toc}\nsim.lua\n"),
    )
    .unwrap();
    let out = run_addon(&dist, "WidgetDemo");
    assert_eq!(
        out,
        [
            "|cff33ff99Widget Demo|r 0.1.0 loaded. Type /wdemo.",
            "|cff33ff99Widget Demo|r toggleWindow",
            "shown true true",
            "Clicks: 3 3 |cff9d9d9dclicked 3 times|r",
            "|cff9d9d9dplain counter at 1|r",
            "1. world 2. hello 1 |cff9d9d9d2 rows|r",
            "list false Show list",
            "Clicks: 1",
            "|cff33ff99Widget Demo|r toggleWindow",
            "shown false",
        ]
        .join("\n")
    );
}

/// Structural class, enum and decorator checks (spec chapters 3 and 4,
/// decision T75).
#[test]
fn class_enum_and_decorator_errors() {
    let cases: &[(&str, &str, &str)] = &[
        ("class A\n function new() end\nend\n", "", "E3004"),
        (
            "class A\n function f() end\nend\nclass B extends A\n function f() end\nend\n",
            "",
            "E3005",
        ),
        ("class A\n override function f() end\nend\n", "", "E3005"),
        (
            "abstract class A\n abstract function f()\nend\nclass B extends A\nend\n",
            "",
            "E3003",
        ),
        ("class A\n abstract function f()\nend\n", "", "E3003"),
        ("abstract class A\nend\nA.new()\n", "", "E3002"),
        (
            "import A from \"/b\"\nA()\n",
            "export abstract class A\nend\n",
            "E3002",
        ),
        (
            "import A from \"/b\"\nclass B extends A\n function f() end\nend\n",
            "export class A\n function f() end\nend\n",
            "E3005",
        ),
        (
            "class A\nend\nclass B extends A\n constructor()\n end\nend\n",
            "",
            "E3001",
        ),
        (
            "class A\nend\nclass B extends A\n constructor()\n  self.x = 1\n  super()\n end\nend\n",
            "",
            "E3001",
        ),
        (
            "class A\n constructor()\n  super()\n end\nend\n",
            "",
            "E3001",
        ),
        ("class A\n x: number = ...\nend\n", "", "E3041"),
        ("class A\n x: number\n function x() end\nend\n", "", "E3042"),
        (
            "class A\n function f()\n  local super = 1\n end\nend\n",
            "",
            "E3040",
        ),
        ("enum E { A = 1, B = \"x\" }\n", "", "E3010"),
        ("enum E { A = 1, B = 1 }\n", "", "E3011"),
        ("enum E { A }\nprint(E.B)\n", "", "E2003"),
        ("enum E { A }\nE.A = 2\n", "", "E2010"),
        ("@nope\nlocal function f() end\n", "", "E3020"),
        ("local function d() end\n@d\nlocal x = 1\n", "", "E3021"),
        ("class A\n @global\n function f() end\nend\n", "", "E3021"),
        ("@debugOnly\nlocal function f() return 1 end\n", "", "E3032"),
        (
            "@debugOnly\nlocal function f() end\nlocal g = f\n",
            "",
            "E3032",
        ),
        (
            "@intrinsic(\"x\")\ndeclare function f(): number\n",
            "",
            "E3033",
        ),
        ("local x = A.new()\nclass A\nend\n", "", "E1037"),
    ];
    for (a, b, code) in cases {
        let dir = tempfile::tempdir().unwrap();
        let mut files = vec![
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"cls\", \"version\": \"1.0.0\" }, \"profiles\": { \"dev\": { \"debug\": false } } }\n",
            ),
            ("src/a.wowlua", *a),
        ];
        if !b.is_empty() {
            files.push(("src/b.wowlua", *b));
        }
        write(dir.path(), &files);
        wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .assert()
            .code(1)
            .stderr(
                predicate::str::contains(format!("error[{code}]"))
                    .or(predicate::str::contains(format!("error: {code}")))
                    .or(predicate::str::contains(*code)),
            );
    }
    // `@deprecated` warns at uses, also across modules (W3030).
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"dep\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/a.wowlua",
                "import old from \"/b\"\n@deprecated(\"use g\")\nlocal function f() end\nf()\nold()\n",
            ),
            ("src/b.wowlua", "@deprecated\nexport function old() end\n"),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert_eq!(stderr.matches("W3030").count(), 2, "{stderr}");
    assert!(stderr.contains("`f` is deprecated: use g"), "{stderr}");
}

/// XML files take part in the load order (spec §5.4.2).
#[test]
fn load_order_with_xml() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"xmlorder\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/a_early.lua", "print(MyAddonFrame)\n"),
            (
                "src/b_ui.xml",
                "<Ui>\n  <Frame name=\"MyAddonFrame\" mixin=\"MyMixin\" inherits=\"MyTemplate\">\n    <Scripts><OnLoad>MyAddon_Init(self)</OnLoad><OnShow>print(Later)</OnShow></Scripts>\n  </Frame>\n</Ui>\n",
            ),
            (
                "src/c_templates.xml",
                "<Ui>\n  <Frame name=\"MyTemplate\" virtual=\"true\"><Scripts><OnLoad>TemplateSetup(self)</OnLoad></Scripts></Frame>\n</Ui>\n",
            ),
            ("src/d_mixin.lua", "MyMixin = {}\n"),
            (
                "src/e_init.lua",
                "function MyAddon_Init(f) return Config end\n",
            ),
            ("src/f_setup.lua", "function TemplateSetup(f) end\n"),
            ("src/g_config.lua", "Config = {}\n"),
            ("src/h_later.lua", "Later = 1\n"),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    let toc = fs::read_to_string(dir.path().join("dist/dev/Xmlorder/Xmlorder.toc")).unwrap();
    let files: Vec<&str> = toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    let pos = |name: &str| {
        files
            .iter()
            .position(|f| f.ends_with(name))
            .unwrap_or_else(|| panic!("{name}: {toc}"))
    };
    for before in [
        "c_templates.xml",
        "d_mixin.lua",
        "e_init.lua",
        "f_setup.lua",
        "g_config.lua",
    ] {
        assert!(pos(before) < pos("b_ui.xml"), "{before}: {toc}\n{stderr}");
    }
    assert!(
        pos("b_ui.xml") < pos("a_early.lua"),
        "a named frame is a global: {toc}"
    );
    assert!(
        pos("b_ui.xml") < pos("h_later.lua"),
        "OnShow runs later: {toc}"
    );
}

/// Files of different flavors never order each other.
#[test]
fn load_order_per_flavor_files() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "split", "version": "1.0.0" }, "flavors": { "standard": {}, "vanilla": {} },
  "build": { "flavor-files": { "src/a_retail.lua": ["standard"], "src/b_classic.lua": ["vanilla"] } } }
"#,
            ),
            ("src/a_retail.lua", "print(Loader)\n"),
            ("src/b_classic.lua", "Loader = 1\n"),
            ("src/c_both.lua", "print(Loader)\n"),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("W1035") && !stderr.contains("E1014"),
        "{stderr}"
    );
    let toc = fs::read_to_string(dir.path().join("dist/dev/Split/Split.toc")).unwrap();
    let files: Vec<&str> = toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    let pos = |name: &str| {
        files
            .iter()
            .position(|f| f.contains(name))
            .unwrap_or_else(|| panic!("{name}: {toc}"))
    };
    assert!(pos("a_retail.lua") < pos("b_classic.lua"), "{toc}");
    assert!(pos("b_classic.lua") < pos("c_both.lua"), "{toc}");
}

/// Usage analysis orders files without imports (spec §5.4.2–§5.4.4).
#[test]
fn load_order_from_usage_analysis() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"usage\", \"version\": \"1.0.0\" } }\n",
            ),
            // Path order would load these first; they use what later files define.
            (
                "src/a_main.lua",
                "local _, ns = ...\nlocal function init() return ns.Config.greeting end\nprint(ns.Util.round(2.6), MyGlobal.ok, init())\n",
            ),
            (
                "src/b_late.lua",
                "local _, ns = ...\nlocal f = CreateFrame(\"Frame\")\nf:SetScript(\"OnEvent\", function() print(ns.Later.x) end)\n",
            ),
            (
                "src/c_config.lua",
                "local _, ns = ...\nns.Config = { greeting = \"hi\" }\n",
            ),
            (
                "src/d_util.lua",
                "local _, ns = ...\nns.Util = { round = function(x) return math.floor(x + 0.5) end }\nMyGlobal = { ok = true }\n",
            ),
            (
                "src/z_later.lua",
                "local _, ns = ...\nns.Later = { x = 1 }\n",
            ),
            // Modules that import each other, using the imports only later.
            (
                "src/m/a.wowlua",
                "import b from \"./b\"\nexport function a() return \"a\" .. b() end\n",
            ),
            (
                "src/m/b.wowlua",
                "import a from \"./a\"\nexport function b() return \"b\" end\nexport function both() return a() end\n",
            ),
            (
                "src/m/main.wowlua",
                "import a from \"./a\"\nimport both from \"./b\"\nprint(a(), both())\n",
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let dist = dir.path().join("dist/dev/Usage");
    let toc = fs::read_to_string(dist.join("Usage.toc")).unwrap();
    let files: Vec<&str> = toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    let pos = |name: &str| {
        files
            .iter()
            .position(|f| f.ends_with(name))
            .unwrap_or_else(|| panic!("{name}: {toc}"))
    };
    assert!(
        pos("c_config.lua") < pos("a_main.lua"),
        "a call at load time is followed: {toc}"
    );
    assert!(pos("d_util.lua") < pos("a_main.lua"), "{toc}");
    assert!(
        pos("b_late.lua") < pos("z_later.lua"),
        "deferred handlers do not order files: {toc}"
    );
    // A mock `CreateFrame` for the deferred handler file.
    fs::write(
        dist.join("mock.lua"),
        "CreateFrame = function() return { SetScript = function() end } end\n",
    )
    .unwrap();
    fs::write(dist.join("Usage.toc"), format!("mock.lua\n{toc}")).unwrap();
    assert_eq!(run_addon(&dist, "Usage"), "3 true hi\nab ab");
}

#[test]
fn load_order_guards_callbacks_and_listed_order() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"guards\", \"version\": \"1.0.0\" },\n  \"build\": { \"load-order\": [\"src/p_first.lua\", \"src/q_second.lua\"] } }\n",
            ),
            // Existence tests and the uses they guard do not order files.
            (
                "src/a_guarded.lua",
                "if Later and Later.x then print(Later.x) end\nlocal y = Later and Later.y\nif type(Later) == \"table\" then print(Later.z) end\n",
            ),
            // `ns.On` only stores the handler; `ns.Run` calls it.
            (
                "src/b_register.lua",
                "local _, ns = ...\nns.On(\"x\", function() print(ns.Late1) end)\nns.Run(function() return ns.Late2 end)\n",
            ),
            (
                "src/c_api.lua",
                "local _, ns = ...\nlocal handlers = {}\nfunction ns.On(name, fn) handlers[name] = fn end\nlocal function call(f) return f() end\nfunction ns.Run(fn) return call(fn) end\n",
            ),
            (
                "src/d_late.lua",
                "local _, ns = ...\nns.Late1 = 1\nns.Late2 = 2\n",
            ),
            ("src/z_later.lua", "Later = { x = 1, y = 2, z = 3 }\n"),
            // The listed order wins over a usage edge (W1035).
            ("src/p_first.lua", "print(Second)\n"),
            ("src/q_second.lua", "Second = 1\n"),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("W1035") && stderr.contains("keeping the listed order"),
        "{stderr}"
    );
    assert!(!stderr.contains("W1034"), "{stderr}");
    let toc = fs::read_to_string(dir.path().join("dist/dev/Guards/Guards.toc")).unwrap();
    let files: Vec<&str> = toc
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    let pos = |name: &str| {
        files
            .iter()
            .position(|f| f.ends_with(name))
            .unwrap_or_else(|| panic!("{name}: {toc}"))
    };
    assert!(
        pos("a_guarded.lua") < pos("z_later.lua"),
        "guarded uses do not order: {toc}"
    );
    assert!(pos("c_api.lua") < pos("b_register.lua"), "{toc}");
    assert!(
        pos("d_late.lua") < pos("b_register.lua"),
        "a called callback is load-time: {toc}"
    );
    assert!(
        pos("p_first.lua") < pos("q_second.lua"),
        "the listed order wins: {toc}"
    );
}

#[test]
fn load_order_diagnostics() {
    type Case<'a> = (&'a [(&'a str, &'a str)], &'a str, &'a str);
    let cases: &[Case] = &[
        (
            &[
                ("src/a.lua", "local _, ns = ...\nprint(ns.Late)\n"),
                (
                    "src/b.lua",
                    "local _, ns = ...\nfunction ns_init() ns.Late = 1 end\n",
                ),
            ],
            "W1034",
            "only defined later",
        ),
        (
            &[
                ("src/a.lua", "Shared = 1\n"),
                ("src/b.lua", "Shared = 2\n"),
                ("src/c.lua", "print(Shared)\n"),
            ],
            "W1033",
            "cannot be determined",
        ),
        (
            &[
                (
                    "src/a.lua",
                    "A = 1\nlocal function check() return B end\nprint(check())\n",
                ),
                ("src/b.lua", "B = 2\nprint(A)\n"),
            ],
            "E1014",
            "called from top-level code",
        ),
    ];
    for (files, code, text) in cases {
        let dir = tempfile::tempdir().unwrap();
        let mut all = vec![(
            "wowlua.json",
            "{ \"package\": { \"name\": \"diag\", \"version\": \"1.0.0\" } }\n",
        )];
        all.extend(files.iter().copied());
        write(dir.path(), &all);
        let assert = wowlua(dir.path()).arg("build").assert();
        let assert = if code.starts_with('E') {
            assert.code(1)
        } else {
            assert.success()
        };
        assert.stderr(predicate::str::contains(*code).and(predicate::str::contains(*text)));
    }
}
