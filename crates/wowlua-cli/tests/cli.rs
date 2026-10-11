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
    assert!(mainline.starts_with("## Interface: 120100\n"), "{mainline}");
    assert!(mainline.contains("## Version: 1.2.3-dev\n"));
    assert!(mainline.contains("## OptionalDeps: Details\n"));
    assert!(mainline.contains("## X-Curse-Project-ID: 12345\n"));
    assert!(
        mainline.ends_with("src\\embeds.xml\nsrc\\Core.lua\nsrc\\UI\\Frames.xml\n"),
        "{mainline}"
    );
    assert!(classic.starts_with("## Interface: 11509\n"), "{classic}");
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
    assert!(!manifest.contains("AllowLoadGameType"), "{manifest}");
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

/// Milestone M1: `.wowlua` files compile (types erased, lines preserved),
/// with source maps; other wowlua syntax names its milestone.
#[test]
fn wowlua_sources_compile_with_types_erased() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"typed\", \"version\": \"1.0.0\" },\n  \"profiles\": { \"dev\": { \"source-maps\": true } } }\n",
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

    fs::write(dir.path().join("src/main.wowlua"), "class A\nend\n").unwrap();
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "src/main.wowlua:1:1: error: not available yet: classes (arrives with milestone M3)",
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
            "import m from \"/main\"\nexport local a = 1\n",
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
    wowlua(dir.path()).arg("build").assert().success();
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
