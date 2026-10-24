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

/// The files under `dir`, relative to it with `/`, sorted.
fn files_in(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
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
        end
        -- WoW's millisecond clock (tests may replace it).
        debugprofilestop = function() return os.clock() * 1000 end"##,
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
        ("import _ from \"wowlua:nonexistent\"\n", "", "E1015"),
        (
            "print(f())\nexport function f() return 1 end\n",
            "",
            "E1037",
        ),
        // A module's own `function f()` is hoisted like an export.
        (
            "print(f())\nfunction f() return 1 end\nexport local a = 1\n",
            "",
            "E1037",
        ),
        (
            "local items = {}\nexport function items() return 1 end\n",
            "",
            "E1016",
        ),
        (
            "export function items() return 1 end\nlocal items = {}\n",
            "",
            "E1016",
        ),
        ("export local a = 1\nexport local a = 2\n", "", "E1016"),
        (
            "export function f() end\nexport function f() end\n",
            "",
            "E1016",
        ),
        (
            "import a from \"/b\"\nexport function a() end\n",
            "export local a = 1\n",
            "E1016",
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

/// An export may share its name with a declaration `@if` removes from the
/// build, not with one it keeps; a forward-declared local is defined by
/// `function name()`, not redeclared (spec §5.1).
#[test]
fn exports_beside_conditional_declarations() {
    let build = |main: &str| {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &[
                (
                    "wowlua.json",
                    "{ \"package\": { \"name\": \"cond\", \"version\": \"1.0.0\" } }\n",
                ),
                ("src/main.wowlua", main),
            ],
        );
        let out = wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .output()
            .unwrap();
        let ran = out
            .status
            .success()
            .then(|| run_addon(&dir.path().join("dist/dev/Cond"), "Cond"));
        (ran, String::from_utf8_lossy(&out.stderr).into_owned())
    };
    let (ran, stderr) = build(
        "import build from \"wowlua:meta\"\n@if(not build.debug)\nlocal items = {}\nexport function items() return 1 end\nlocal g\nfunction g() return 3 end\nprint(items(), g())\n",
    );
    assert_eq!(ran.as_deref(), Some("1 3"), "{stderr}");
    // Alternatives for different builds.
    let (ran, stderr) = build(
        "import build from \"wowlua:meta\"\n@if(build.debug)\nexport function f() return 1 end\n@if(not build.debug)\nexport function f() return 2 end\nprint(f())\n",
    );
    assert_eq!(ran.as_deref(), Some("1"), "{stderr}");
    let (ran, stderr) = build(
        "import build from \"wowlua:meta\"\n@if(build.debug)\nlocal items = {}\nexport function items() return 1 end\n",
    );
    assert!(ran.is_none() && stderr.contains("error[E1016]"), "{stderr}");
}

/// A module's top-level `function name()` is a local of the module,
/// hoisted, and never a global; a script's is a global (spec §5.1).
#[test]
fn module_functions_are_module_local() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"loc\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/main.wowlua",
                "export function run(): number return helper() + later() end\nfunction helper(): number return 1 end\nfunction later(): number return 2 end\nprint(\"module\", run(), rawget(_G, \"helper\"), rawget(_G, \"later\"))\n",
            ),
            (
                "src/plain.wowlua",
                "function scriptGlobal() return 1 end\nprint(\"script\", rawget(_G, \"scriptGlobal\") == scriptGlobal)\n",
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let out = run_addon(&dir.path().join("dist/dev/Loc"), "Loc");
    let mut lines: Vec<&str> = out.lines().collect();
    lines.sort_unstable();
    assert_eq!(lines, ["module 3 nil nil", "script true"], "{out}");
}

/// An import cycle anywhere in the package leaves every other import its
/// type: standard-library modules and the modules a cycle imports are
/// checked before their importers, and the modules of a cycle see each
/// other's exports as their annotations declare them (spec §5.4.4).
#[test]
fn import_cycles_keep_import_types() {
    let manifest = "{ \"package\": { \"name\": \"cyc\", \"version\": \"1.0.0\" }, \"check\": { \"mode\": \"strict\" } }\n";
    // The errors `wowlua check` reports, as `file:line: code`.
    let check = |files: &[(&str, &str)]| -> Vec<String> {
        let dir = tempfile::tempdir().unwrap();
        let mut all = vec![("wowlua.json", manifest)];
        all.extend_from_slice(files);
        write(dir.path(), &all);
        let out = wowlua(dir.path())
            .args(["check", "--message-format", "short"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let mut found: Vec<String> = stderr
            .lines()
            .filter_map(|l| {
                let (at, rest) = l.split_once(": error[")?;
                let (file, line) = at.rsplit_once(':')?.0.rsplit_once(':')?;
                let code = &rest[..rest.find(']')?];
                Some(format!("{file}:{line}: {code}"))
            })
            .collect();
        found.sort();
        found
    };
    // A run-time cycle between two modules, and a third file that imports
    // the standard library.
    let cycle = [
        (
            "src/P.wowlua",
            "import q from \"./Q\"\nexport function p(): number return q() end\nexport function s(): string local early: string = q return early end\n",
        ),
        (
            "src/Q.wowlua",
            "import p from \"./P\"\nexport function q(): number return 1 end\nexport function r(): number return p() end\nexport function t(): string local late: string = p return late end\n",
        ),
    ];
    let mut files = cycle.to_vec();
    files.push((
        "src/A.wowlua",
        "import after from \"wowlua:timer\"\nimport isSecret from \"wowlua:secret\"\nlocal n: number = after\nlocal function f(v: string | Secret<string>): string\n    if isSecret(v) then return \"?\" end\n    return v:lower()\nend\nprint(n, f(\"x\"))\n",
    ));
    assert_eq!(
        check(&files),
        [
            "src/A.wowlua:3: E2001",
            "src/P.wowlua:3: E2001",
            "src/Q.wowlua:4: E2001"
        ]
    );
    // The standard library imported inside a module of the cycle: by
    // namespace, and its predicate through a local alias.
    let mut files = cycle.to_vec();
    files[1] = (
        "src/Q.wowlua",
        "import p from \"./P\"\nimport * as secret from \"wowlua:secret\"\nexport function q(): number return 1 end\nexport function r(): number return p() end\nconst isSecret = secret.isSecret\nlocal function f(v: string | Secret<string>): string\n    if isSecret(v) then return \"?\" end\n    return v:lower()\nend\nlocal function g(v: string | Secret<string>): string\n    if not secret.isSecret(v) then return v:upper() end\n    return \"?\"\nend\nlocal n: number = secret.isSecret\nprint(n, f(\"x\"), g(\"y\"))\n",
    );
    assert_eq!(
        check(&files),
        ["src/P.wowlua:3: E2001", "src/Q.wowlua:14: E2001"]
    );
    // A load-time cycle (E1014) does not hide the types either.
    let files = [
        (
            "src/main.wowlua",
            "import a from \"/b\"\nexport local m: number = a\nlocal s: string = a\nprint(s)\n",
        ),
        (
            "src/b.wowlua",
            "import m from \"/main\"\nimport after from \"wowlua:timer\"\nexport local a: number = m\nlocal t: string = m\nlocal n: number = after\nprint(t, n)\n",
        ),
    ];
    assert_eq!(
        check(&files),
        [
            "src/b.wowlua:3: E1014",
            "src/b.wowlua:4: E2001",
            "src/b.wowlua:5: E2001",
            "src/main.wowlua:3: E2001"
        ]
    );
}

/// Inside an import cycle, a module checked before another of the cycle
/// reads that one's exports without a type annotation as `any`: W1036
/// where they are imported or read, once (spec §5.4.4), also a `typeof`
/// alias through a namespace import (`qn.Conf`) and a file-level local
/// `import internal` reads. A `typeof` alias gets its type once its module
/// is checked (spec §2.2.7).
#[test]
fn import_cycles_warn_about_untyped_exports() {
    let manifest = "{ \"package\": { \"name\": \"cyc\", \"version\": \"1.0.0\" }, \"check\": { \"mode\": \"strict\" } }\n";
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            ("wowlua.json", manifest),
            (
                "src/P.wowlua",
                "import q, items from \"./Q\"\nimport * as qn from \"./Q\"\nimport type Conf from \"./Q\"\nexport function p(): number return q() end\nlocal function f() return items, qn.items, qn.items, qn.r(), qn.q() end\nlocal c: Conf = 1\nlocal d: qn.Conf = 1\nlocal e: qn.Conf = 2\nprint(f, c, d, e)\n",
            ),
            (
                "src/Q.wowlua",
                "import p from \"./P\"\nexport function q(): number return 1 end\nexport function r(): number return p() end\nexport local items = { a = 1 }\nlocal defaults = { n = 1 }\nexport type Conf = typeof(defaults)\nlocal bad: string = p\nlocal ok: Conf = { n = 2 }\nlocal c: Conf = 1\nprint(bad, ok, c)\n",
            ),
            (
                "src/main.wowlua",
                "import type Conf from \"./Q\"\nimport items from \"./Q\"\nlocal c: Conf = 1\nlocal n: string = items\nprint(c, n)\n",
            ),
        ],
    );
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut found: Vec<String> = stderr
        .lines()
        .filter_map(|l| {
            let (at, rest) = l
                .split_once(": error[")
                .or_else(|| l.split_once(": warning["))?;
            let (file, line) = at.rsplit_once(':')?.0.rsplit_once(':')?;
            let code = &rest[..rest.find(']')?];
            Some(format!("{file}:{line}: {code}"))
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            "src/P.wowlua:1: W1036",
            "src/P.wowlua:3: W1036",
            "src/P.wowlua:5: W1036",
            "src/P.wowlua:7: W1036",
            "src/Q.wowlua:7: E2001",
            "src/Q.wowlua:9: E2001",
            "src/main.wowlua:3: E2001",
            "src/main.wowlua:4: E2001",
        ],
        "{stderr}"
    );
    // `import internal` in a cycle of test helpers: an annotated local or a
    // function has its type, an unannotated local is `any` (W1036).
    fs::remove_dir_all(dir.path().join("src")).unwrap();
    write(
        dir.path(),
        &[
            ("src/main.wowlua", "print(1)\n"),
            (
                "tests/a.wowlua",
                "import internal hidden, typed, helper from \"./b\"\nexport function a(): number return 1 end\nlocal s: string = typed\nlocal h: string = helper()\nprint(hidden, s, h)\n",
            ),
            (
                "tests/b.wowlua",
                "import a from \"./a\"\nlocal hidden = { n = 1 }\nlocal typed: number = 2\nlocal function helper(): number return a() end\nprint(hidden, typed, helper)\n",
            ),
            (
                "tests/x.test.wowlua",
                "import test from \"wowlua:test\"\nimport a from \"./a\"\n@test\nlocal function t() print(a()) end\n",
            ),
        ],
    );
    let out = wowlua(dir.path())
        .args(["test", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut found: Vec<String> = stderr
        .lines()
        .filter_map(|l| {
            let (at, rest) = l
                .split_once(": error[")
                .or_else(|| l.split_once(": warning["))?;
            let (file, line) = at.rsplit_once(':')?.0.rsplit_once(':')?;
            let code = &rest[..rest.find(']')?];
            Some(format!("{file}:{line}: {code}"))
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            "tests/a.wowlua:1: W1036",
            "tests/a.wowlua:3: E2001",
            "tests/a.wowlua:4: E2001",
        ],
        "{stderr}"
    );
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
    // Only the file that reads `wowlua:meta` is stored per flavor, with the
    // runtime, which carries each flavor's metadata.
    assert_eq!(
        files_in(&dist.join("wowlua/flavors")),
        [
            "standard/src/main.lua",
            "standard/wowlua/runtime.lua",
            "vanilla/src/main.lua",
            "vanilla/wowlua/runtime.lua"
        ]
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

/// `--jobs 4` (parallel regions, also for a small project when explicit)
/// gives what `--jobs 1` gives: the same warnings in the same order, and
/// the same files.
#[test]
fn parallel_builds_equal_sequential_builds() {
    let dir = tempfile::tempdir().unwrap();
    let mut files: Vec<(String, String)> = vec![(
        "wowlua.json".into(),
        r#"{ "package": { "name": "par", "version": "1.0.0" },
  "flavors": { "standard": {}, "vanilla": {} }, "check": { "lua": true } }
"#
        .into(),
    )];
    for n in 0..12 {
        files.push((
            format!("src/f{n:02}.lua"),
            format!("local _, ns = ...\nlocal unused{n} = 1\nns.v{n} = {{ {n}, \"{n}\" }}\nprint(ns.v{n}.x, C_Seasons)\n"),
        ));
    }
    files.push((
        "src/m.wowlua".into(),
        "@deprecated(\"no\")\nlocal function old(): number return 1 end\nprint(old())\n".into(),
    ));
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    write(dir.path(), &refs);
    let run = |jobs: &str, profile: &str| {
        let _ = std::fs::remove_dir_all(dir.path().join("dist"));
        let assert = wowlua(dir.path())
            .args([
                "build",
                "--message-format",
                "short",
                "-j",
                jobs,
                "--profile",
                profile,
            ])
            .assert()
            .success();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        let warnings: Vec<String> = stderr
            .lines()
            .filter(|l| l.starts_with("src/"))
            .map(str::to_string)
            .collect();
        let dist = dir.path().join("dist").join(profile).join("Par");
        let tree: Vec<(String, Vec<u8>)> = files_in(&dist)
            .into_iter()
            .map(|f| {
                let bytes = std::fs::read(dist.join(&f)).unwrap();
                (f, bytes)
            })
            .collect();
        (warnings, tree)
    };
    for profile in ["dev", "release"] {
        let sequential = run("1", profile);
        assert!(!sequential.0.is_empty());
        assert_eq!(run("4", profile), sequential, "{profile}");
    }
}

/// Long operator chains are valid Lua 5.1 (its parser builds them in a
/// loop, and so does wowlua's), however deep the tree they make: every pass
/// walks, clones and drops them without a stack overflow, and `and`/`or`
/// chains lower in linear time.
#[test]
fn deep_operator_chains_compile() {
    // 999 links, just below `deep::DEEP`, where no pass grows its stack (it
    // overflowed Windows' 1 MiB main thread); and far past it.
    for terms in [500, 50_000] {
        let dir = tempfile::tempdir().unwrap();
        let sum = vec!["1"; terms].join(" + ");
        let test = vec!["true"; terms].join(" and ") + " or " + &vec!["false"; terms].join(" or ");
        write(
            dir.path(),
            &[
                (
                    "wowlua.json",
                    r#"{ "package": { "name": "deep", "version": "1.0.0" } }
"#,
                ),
                ("src/sum.lua", &format!("local x = {sum}\nprint(x)\n")),
                ("src/test.lua", &format!("local b = {test}\nprint(b)\n")),
            ],
        );
        for args in [&["check"][..], &["build"], &["build", "--release"]] {
            wowlua(dir.path()).args(args).assert().success();
        }
    }
}

/// With `check.lua`, a later flavor takes a plain Lua file's type check
/// from an earlier one where what the file read of the declarations is the
/// same (debug builds, and so this test, check every reused file again and
/// compare): each warning is reported once, at its place among the others,
/// and a file that reads an API that differs between flavors reports what
/// each flavor finds.
#[test]
fn check_lua_flavors_share_what_checks_the_same() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "reuse", "version": "1.0.0" },
  "flavors": { "standard": {}, "vanilla": {}, "camelot": {} },
  "check": { "lua": true } }
"#,
            ),
            // Reads no API.
            (
                "src/data.lua",
                "local t = { x = 1 }\nlocal n = 1\nprint(n .. {}, t.x())\n",
            ),
            // Only vanilla declares `C_Seasons`.
            ("src/seasons.lua", "print(C_Seasons.HasActiveSeason())\n"),
            // One result in standard and vanilla, two in camelot.
            (
                "src/haste.lua",
                "local a, b = GetRangedHaste()\nprint(a + b)\n",
            ),
            // A union of two declared types: never stored.
            (
                "src/union.lua",
                "local f = math.random() > 0.5 and UIParent or GameTooltip\nprint(string.rep(f, 2))\n",
            ),
            (
                "src/typename.lua",
                "local v = { a = 1 }\nprint(typeName(v), v.b)\n",
            ),
        ],
    );
    let assert = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    let warnings: Vec<&str> = stderr.lines().filter(|l| l.starts_with("src/")).collect();
    assert_eq!(
        warnings,
        [
            "src/data.lua:3:16: warning[W2004]: a value of type `number` is not callable",
            "src/haste.lua:2:7: warning[W2040]: `number | Secret<number>` may be a secret value; narrow it with `canaccessvalue` or `issecretvalue` first",
            "src/seasons.lua:1:7: warning[W1001]: `C_Seasons` is not a declared global",
            "src/typename.lua:2:7: warning[W1001]: `typeName` is not a declared global",
            "src/union.lua:2:18: warning[W2001]: `GameTooltip | UIParent` is not assignable to `string`",
            "src/haste.lua:2:11: warning[W2040]: `number | Secret<number>` may be a secret value; narrow it with `canaccessvalue` or `issecretvalue` first",
        ],
        "{stderr}"
    );
}

/// Camelot's declarations take the calls Blizzard's code makes and the
/// values the client answers (SuuUI's port, report C25 to C37): money and
/// sizes are numbers, tooltip data methods take their `C_TooltipInfo`
/// function's parameters, a spell cooldown is a readable table with possibly
/// secret fields (C34: an item cooldown's are plain), the client's own
/// number globals are declared, named frames have their mixins' methods
/// (C35), a unit's name may be absent, the cursor's info is its kind and
/// details.
#[test]
fn camelot_declarations_follow_the_client() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "client", "version": "1.0.0" },
  "flavors": { "camelot": {} },
  "check": { "mode": "strict" } }
"#,
            ),
            (
                "src/main.wowlua",
                r#"local _, _, _, _, _, _, _, _, _, buyout = C_AuctionHouse.GetReplicateItemInfo(0)
local cancel: number = C_AuctionHouse.GetCancelCost(1) + buyout
GameTooltip:SetInboxItem(1, 2)
GameTooltip:SetSendMailItem(1)
local zoom: number = Minimap:GetZoom()
local maxZoom = Minimap:GetZoomLevels() - 1
C_NamePlate.SetNamePlateSize(140, 30)
local list: number[] = {}
local secretTable: boolean = issecrettable(list)
SelectActiveQuest(1)
local completion = GetQuestLogCompletionText(1)
JoinChannelByName("LookingForGroup")
SetTrainerServiceTypeFilter("available", true)
C_ChatInfo.SendChatMessage("hi", "CHANNEL", nil, 1)
local known: boolean = IsSpellKnown(1)
local tab = C_Bank.FetchNextPurchasableBankTabData(0)
local tabCost: number = tab and tab.tabCost or 0
local condition = C_Traits.GetConditionInfo(1, 2, true)
local conditionText: string? = condition and condition.tooltipText
local cooldown = C_Spell.GetSpellCooldown(1)
local active: boolean = cooldown ~= nil and cooldown.isActive
local started = cooldown and cooldown.startTime > 0
local party: number = LE_PARTY_CATEGORY_INSTANCE
local location = ItemLocation:CreateFromBagAndSlot(0, 1)
NamePlateDriverFrame:Show()
local itemCooldown = C_Spell.GetItemCooldown(1)
local itemStarted = itemCooldown and itemCooldown.startTime > 0
ChatFrame1:ApplySystemAnchor()
ChatFrame1:AddMessage("hi", 1, 1, 1)
PlayerFrame:ApplySystemAnchor()
local unitName, unitRealm = UnitName("party2")
local named: string = unitName
local cursorKind, cursorID = GetCursorInfo()
local held: string? = cursorKind
local heldID: number = cursorID
local binding = C_DurationUtil.CreateDurationTextBinding()
binding:SetFormatter(C_StringUtil.CreateNumericRuleFormatter())
local counts: CustomAuraButtonApplicationCountOptions = { formatter = C_StringUtil.CreateAbbreviatedNumberFormatter() }
local enabled: string = binding:IsEnabled()
local fade = Minimap:CreateAnimationGroup():CreateAnimation("Alpha")
fade:SetFromAlpha(1)
local fadeTo: string = fade:GetToAlpha()
local outfit = C_TransmogCollection.GetCustomSetItemTransmogInfoList(1)
local firstPiece = outfit and outfit[1]
local appearance: string = firstPiece and firstPiece.appearanceID or ""
local bagItem = ItemLocation:CreateFromBagAndSlot(0, 1)
local sellable: boolean = C_AuctionHouse.IsSellItemValid(bagItem, false) and bagItem.bagID == 0
local log = CreateFrame("ScrollingMessageFrame")
log:SetFading(false)
log:AddMessage("hi", 1, 1, 1)
local offset: string = log:GetScrollOffset()
local stored: string = ChatFrame1:GetNumMessages()
print(cancel, zoom, maxZoom, secretTable, completion, known, tabCost, conditionText, active, started, party, location, itemStarted, unitRealm, named, held, heldID, counts, enabled, fadeTo, appearance, sellable, offset, stored)
"#,
            ),
        ],
    );
    let assert = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    let diagnostics: Vec<&str> = stderr.lines().filter(|l| l.starts_with("src/")).collect();
    // Only the spell cooldown's start time may be secret (an item's never
    // is), and a unit's name may be absent.
    assert_eq!(
        diagnostics,
        [
            "src/main.wowlua:22:30: error[E2040]: `number | Secret<number>` may be a secret value; narrow it with `canaccessvalue` or `issecretvalue` first",
            "src/main.wowlua:22:30: error[E2001]: cannot compare `number | Secret<number>` with `0`",
            "src/main.wowlua:32:7: error[E2001]: `nil | string | Secret<string>` is not assignable to `string`",
            "src/main.wowlua:39:7: error[E2001]: `boolean` is not assignable to `string`",
            "src/main.wowlua:42:7: error[E2001]: `number` is not assignable to `string`",
            "src/main.wowlua:45:7: error[E2001]: `number | \"\"` is not assignable to `string`",
            "src/main.wowlua:51:7: error[E2001]: `number` is not assignable to `string`",
            "src/main.wowlua:52:7: error[E2001]: `number` is not assignable to `string`",
        ],
        "{stderr}"
    );
}

/// In split mode a file compiles the same for every flavor unless it reads
/// `wowlua:meta` or imports a value that differs: only those are stored per
/// flavor, and what the others report is reported once.
#[test]
fn split_flavors_share_what_compiles_the_same() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "share", "version": "1.0.0" },
  "flavors": { "standard": {}, "vanilla": {} },
  "build": { "load-order": ["src/plain.lua", "src/util.wowlua", "src/flavored.wowlua", "src/label.wowlua", "src/main.wowlua"] },
  "profiles": { "release": { "bundle": false } } }
"#,
            ),
            ("src/plain.lua", "local _, ns = ...\nns.plain = \"plain\"\n"),
            (
                "src/util.wowlua",
                "@deprecated(\"use twice\")\nexport function double(n: number): number return n * 2 end\nexport function twice(n: number): number return n * 2 end\n",
            ),
            (
                "src/flavored.wowlua",
                "import flavor from \"wowlua:meta\"\nexport const NAME = flavor\n",
            ),
            (
                "src/label.wowlua",
                "import NAME from \"./flavored\"\nprint(NAME)\n",
            ),
            (
                "src/main.wowlua",
                "import double, twice from \"./util\"\n@inline\nlocal function countdown(n: number): number\n    if n <= 0 then return 0 end\n    return countdown(n - 1)\nend\nlocal _, ns = ...\nprint(ns.plain, double(1), twice(2), countdown(3))\n",
            ),
        ],
    );
    // Dev builds also warn about `@deprecated` at run time.
    let deprecated = "Share: double is deprecated: use twice\n";
    for (profile, warnings, at_run_time) in [
        ("dev", &["W3030"][..], deprecated),
        ("release", &["W3030", "W3031"], ""),
    ] {
        let assert = wowlua(dir.path())
            .args(["build", "--profile", profile])
            .assert()
            .success();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        for code in warnings {
            assert_eq!(
                stderr.matches(&format!("warning[{code}]")).count(),
                1,
                "{profile}: {stderr}"
            );
        }
        let dist = dir.path().join(format!("dist/{profile}/Share"));
        assert_eq!(
            files_in(&dist.join("wowlua/flavors")),
            [
                "standard/src/flavored.lua",
                "standard/src/label.lua",
                "vanilla/src/flavored.lua",
                "vanilla/src/label.lua"
            ],
            "{profile}"
        );
        for (flavor, family) in [("standard", "mainline"), ("vanilla", "classic")] {
            assert_eq!(
                run_addon_as(&dist, "Share", Some((flavor, family))),
                format!("{flavor}\n{at_run_time}plain 2 4 0"),
                "{profile}"
            );
        }
    }
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
                "local _, ns = ...\nns.order = { \"first\" }\nns.where = debugstack and debugstack(1):match(\"([^/]+)%.lua\")\n",
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

/// A build that writes nothing (`check`, `toc`, `compile`) makes no source
/// maps; its files are still exactly the ones a written build writes.
#[test]
fn unwritten_builds_make_the_written_files() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "same", "version": "1.0.0" },
  "build": { "load-order": ["src/first.lua", "src/util.wowlua", "src/main.wowlua"] } }
"#,
            ),
            (
                "src/first.lua",
                "local _, ns = ...\nns.order = { \"first\" }\n",
            ),
            (
                "src/util.wowlua",
                "export function twice(n: number): number\n\treturn n * 2\nend\n",
            ),
            (
                "src/main.wowlua",
                "import twice from \"./util\"\nlocal _, ns = ...\nprint(twice(#ns.order))\n",
            ),
        ],
    );
    for profile in ["dev", "release"] {
        let mut cmd = wowlua(dir.path());
        cmd.arg("build");
        if profile == "release" {
            cmd.arg("--release");
        }
        cmd.assert().success();
        assert!(
            dir.path()
                .join(format!("dist/{profile}/.maps/Same"))
                .is_dir()
        );
        let mut session = wowlua_driver::Session::new();
        let options = wowlua_driver::LoadOptions {
            manifest: None,
            cwd: dir.path().to_path_buf(),
            ..Default::default()
        };
        let project = wowlua_driver::load_projects(&mut session, &options)
            .and_then(|ps| ps.into_iter().next())
            .expect("the project loads");
        let mut request = wowlua_driver::BuildRequest::new(profile);
        request.write = false;
        let built = wowlua_driver::build(&mut session, &project, &request).expect("it builds");
        let addon = dir.path().join(format!("dist/{profile}/Same"));
        let written = files_in(&addon);
        assert_eq!(
            built.files.keys().cloned().collect::<Vec<_>>(),
            written,
            "{profile}"
        );
        for (path, content) in &built.files {
            let bytes = match content {
                wowlua_driver::build::Content::Bytes(b) => b.clone(),
                wowlua_driver::build::Content::Copy(from) => fs::read(from).unwrap(),
            };
            assert_eq!(
                bytes,
                fs::read(addon.join(path)).unwrap(),
                "{profile}: {path}"
            );
        }
    }
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

/// What finishing a file reports (W3031 from inlining, E5001 from checking
/// the emitted code) comes after what lowering reports (W3030), for every
/// file: whether each file is finished right after it is lowered, or all
/// are lowered first because `rename-private` has private methods to rename.
#[test]
fn finishing_diagnostics_follow_lowering_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    // More than 200 locals in one function, all of them used.
    let names: Vec<String> = (0..210).map(|n| format!("v{n}")).collect();
    let locals: String = names
        .iter()
        .enumerate()
        .map(|(n, v)| format!("    local {v} = seed + {n}\n"))
        .collect();
    let first = format!(
        "@deprecated(\"use wide\")\nlocal function narrow(): number return 1 end\n@inline\nlocal function countdown(n: number): number\n    if n <= 0 then return 0 end\n    return countdown(n - 1)\nend\nlocal function wide(seed: number): number\n{locals}    local all = {{ {} }}\n    return #all\nend\nprint(wide(1), countdown(3), narrow())\n",
        names.join(", ")
    );
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "order", "version": "1.0.0" },
  "build": { "load-order": ["src/first.wowlua", "src/second.wowlua"] },
  "profiles": { "renamed": { "inherits": "release", "minify": "max", "bundle": false } } }
"#,
            ),
            ("src/first.wowlua", first.as_str()),
            (
                "src/second.wowlua",
                "import old from \"./old\"\nclass Counter\n    total: number = 0\n    private function bump() self.total = self.total + 1 end\n    function run(): number\n        self:bump()\n        return self.total\n    end\nend\nold()\nprint(Counter.new():run())\n",
            ),
            (
                "src/old.wowlua",
                "@deprecated(\"use something else\")\nexport function old() print(\"old\") end\n",
            ),
        ],
    );
    let reported = |args: &[&str]| -> Vec<String> {
        let assert = wowlua(dir.path())
            .args(args)
            .args(["--message-format", "short"])
            .assert()
            .failure();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        stderr
            .lines()
            .filter_map(|line| {
                let (path, rest) = line.split_once(':')?;
                let code = rest.split_once('[')?.1.split_once(']')?.0;
                Some(format!("{path} {code}"))
            })
            .collect()
    };
    let lowered = ["src/first.wowlua W3030", "src/second.wowlua W3030"];
    assert_eq!(
        reported(&["check"]),
        [&lowered[..], &["src/first.wowlua E5001"]].concat()
    );
    let finished = ["src/first.wowlua W3031", "src/first.wowlua E5001"];
    for profile in ["release", "renamed"] {
        assert_eq!(
            reported(&["build", "--profile", profile]),
            [&lowered[..], &finished[..]].concat(),
            "{profile}"
        );
    }
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

/// `rename-private` leaves the methods of classes other addons can reach
/// alone: `@global` classes, classes in `@global` signatures, and classes
/// whose instances are stored in globals (with their whole hierarchy).
#[test]
fn rename_private_keeps_classes_reachable_from_globals() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "reach", "version": "1.0.0" },
  "profiles": { "release": { "minify": "max", "bundle": false } } }
"#,
            ),
            (
                "src/main.wowlua",
                "@global(\"ReachPanel\")\nclass Panel\n    private function refreshPanel(): string return \"panel\" end\n    function show(): string return self:refreshPanel() end\nend\nclass Base\n    protected function redrawBase(): string return \"base\" end\nend\nclass Widget extends Base\n    function draw(): string return self:redrawBase() end\nend\nReachWidget = Widget.new()\nclass Report\n    private function formatReport(): string return \"report\" end\n    function text(): string return self:formatReport() end\nend\n@global\nfunction ReachReport(): Report return Report.new() end\nclass Hidden\n    private function computeHidden(): string return \"hidden\" end\n    function value(): string return self:computeHidden() end\nend\nprint(Panel.new():show(), ReachWidget:draw(), ReachReport():text(), Hidden.new():value())\n",
            ),
        ],
    );
    wowlua(dir.path())
        .args(["build", "--release"])
        .assert()
        .success();
    let release = dir.path().join("dist/release/Reach");
    assert_eq!(run_addon(&release, "Reach"), "panel base report hidden");
    let main = fs::read_to_string(release.join("src/main.lua")).unwrap();
    for kept in ["refreshPanel", "redrawBase", "formatReport"] {
        assert!(
            main.contains(kept),
            "{kept} is reachable from a global:\n{main}"
        );
    }
    assert!(!main.contains("computeHidden"), "{main}");
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
                "import helper from \"wowlua:helper\"\nexport function wrap(tag: string)\n    return function(fn: (...any) -> string, ctx: DecoratorContext) return function(...: any): string return tag .. fn(...) end end\nend\nexport function register(name: string)\n    return function(fn: () -> any, ctx: DecoratorContext) print(\"registered\", name, ctx.name, fn()) end\nend\nexport local version = helper()\n",
            ),
            (
                "fixture.d.wowlua",
                "@intrinsic(\"internal.wrap\")\ndeclare function wrap(tag: string): any\n@intrinsic(\"internal.register\")\ndeclare function register(name: string): any\n",
            ),
            (
                "helper.wowlua",
                "export function helper(): number return 7 end\n",
            ),
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
    let main = r#"import log, warn, history, setLevel, sink, configure, print as chat from "wowlua:log"
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
-- A prefix of its own, and info and above to chat; then nothing to chat.
setLevel("debug")
configure({ prefix = "|cffaabbccL|r ", chatLevel = "info" })
log.info("shown")
log.debug("quiet")
chat("Hi")
configure({ chatLevel = "off" })
warn("silent")
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
            "|cffaabbccL|r |cfff1eadb[info]|r shown",
            "sink info shown",
            "sink debug quiet",
            "|cffaabbccL|r Hi",
            "sink warn silent",
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
            "|cffaabbccL|r Hi",
            "sink warn silent",
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
-- Wrong arguments on purpose, past the static checks (`pcall` checks its
-- arguments against the function's parameters).
local record: any, unchecked: any = t.record, place
show(pcall(record, t, "5"))
show(pcall(record, t, 5, 7))
show(pcall(unchecked, "UP", { x = 1, y = 2 }))
show(pcall(unchecked, "LEFT", { x = 1 }))
show(pcall(unchecked, "LEFT", { x = 1, y = 2 }, nil, 1, "two"))
show(pcall(unchecked, "RIGHT", { x = 1, y = 2 }))
show(pcall(strict, {}))
show(pcall(loose :: any, "x"))
show(pcall(inner :: any, "x"))
show(pcall(function() return (_G.missing :: number) end))
-- Two parameters of one name: the later one's value is the name's.
export function pair(_: number, _: string) end
show(pcall(pair, 1, "a"))
show(pcall(pair :: any, 1, 2))
-- The game's objects: a table or a userdata (T277).
export function takeDuration(_d: DurationObject) end
local proxy: any = (_G :: any).newproxy
show(pcall(takeDuration, proxy(false)))
show(pcall(takeDuration :: any, 5))
-- A union of many literals (every event's name): checked against a set.
export function onEvent(event: EventName) end
show(pcall(onEvent, "PLAYER_LOGIN"))
show(pcall(onEvent :: any, "NOT_AN_EVENT"))
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
            "true nil",
            "false Chk: src/main.wowlua:45: bad argument #2 '_' to 'pair' (string expected, got number)",
            "true nil",
            "false Chk: src/main.wowlua:49: bad argument #1 '_d' to 'takeDuration' (DurationObject expected, got number)",
            "true nil",
            "false Chk: src/main.wowlua:54: bad argument #1 'event' to 'onEvent' (\"ACCOUNT_STORE_CURRENCY_AVAILABLE_UPDATED\" | \"ACCOUNT_STORE_FRONT_UPDATED\" | \"ACCOUNT_STORE_ITEM_INFO_UPDATED\" | \"ACCOUNT_STORE_TRANSACTION_ERROR\" | \"STORE_FRONT_STATE_UPDATED\" | \"ACHIEVEMENT_EARNED\" | \"ACHIEVEMENT_PLAYER_NAME\" | \"ACHIEVEMENT_SEARCH_UPDATED\" | … (1774 more) expected, got string)",
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
        "false Chk: src/main.wowlua:43: bad value for '::' (number expected, got nil)"
    );
}

/// `EventArgs<E>` is the payload of the event or message a call names
/// (decision L44): a handler's arguments and a sender's values are checked,
/// a constrained type parameter keeps the literal (`E` is "UNIT_HEALTH",
/// not `string`) and satisfies its constraint where it is passed on, and a
/// long union shows its first members.
#[test]
fn event_args_types() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"declare message PROBE_CHANGED(key: string, value: number)

local function onGame<E extends EventName>(event: E, fn: (name: E, EventArgs<E>...) -> ())
    print(event, fn)
end
local function send<M extends MessageName>(name: M, ...: EventArgs<M>...)
    print(name, ...)
end
local function relay<M extends MessageName>(name: M, ...: EventArgs<M>...)
    send(name, ...)
end
local function id<S extends string>(s: S): S
    return s
end
onGame("UNIT_HEALTH", function(name, unit)
    local n: number = unit
    local e: "UNIT_HEALTH" = name
    print(e, n)
end)
send("PROBE_CHANGED", "x", 1)
send("PROBE_CHANGED", 1, "x")
relay("PROBE_CHANGED", "a", 2)
local k: "a" = id("a")
print(k)
onGame("NOT_AN_EVENT", function() end)
local function wrap<E extends EventName>(event: E, fn: (name: E, EventArgs<E>...) -> ())
    onGame(event, fn)
end
wrap("UNIT_HEALTH", function(_, unit) print(unit) end)
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"eventargs\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let errors: Vec<&str> = stderr.lines().filter(|l| l.contains("error[")).collect();
    assert_eq!(errors.len(), 4, "{stderr}");
    assert_eq!(
        errors[..3],
        [
            "src/main.wowlua:16:11: error[E2001]: `string` is not assignable to `number`",
            "src/main.wowlua:21:23: error[E2001]: `1` is not assignable to `string`",
            "src/main.wowlua:21:26: error[E2001]: `\"x\"` is not assignable to `number`",
        ],
        "{stderr}"
    );
    assert!(errors[3].starts_with("src/main.wowlua:25:1: error[E2014]: `\"NOT_AN_EVENT\"`"));
    assert!(errors[3].contains(" more)` of `E`"), "{stderr}");
    assert!(errors[3].len() < 1000, "{stderr}");
}

/// `pairs` over a record with named fields and an `unknown` indexer: its
/// values are `unknown`, which fits the union of the values (spec §2.6).
#[test]
fn pairs_over_record_with_unknown_indexer() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"type Tags = { module?: string, skinned?: Frame, [string]: unknown }
local function f(t: Tags)
    for k, v in pairs(t) do print(k, v) end
end
f({ module = "a" })
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"pairsrec\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
}

/// `keyof T` and `T[K]` (decision L45): a field-name union and a field's
/// type, worked out when a generic call binds them.
#[test]
fn keyof_and_indexed_access() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"type Defaults = { line: number, figure: boolean, note?: string }
local k: keyof Defaults = "figure"
local n: Defaults["line"] = 3
local o: Defaults["note"] = nil
local u: Defaults["line" | "figure"] = true
print(k, n, o, u)

local function get<D extends { [string]: unknown }, K extends keyof D>(values: D, key: K): D[K]
    return values[key] :: D[K]
end
local line = get({ line = 20, figure = true }, "line")
local s: string = line
local f: string = get({ line = 20, figure = true }, "figure")
get({ line = 20 }, "lnie")
local bad: keyof Defaults = "nope"
print(s, f, bad)
type Both = Defaults & { enabled: boolean }
local e: Both["enabled"] = 1
local kk: keyof Both = "enabled"
print(e, kk)
local function each<K extends keyof Defaults>(keys: K[]) print(keys) end
each({ "line", "figure" })
each({ "line", "nope" })
type Loose = { [string]: unknown } & { enabled: boolean }
local z: string = (5 :: unknown) :: Loose["x"]
print(z)
local function set<K extends keyof Defaults>(key: K, value: Defaults[K]) print(key, value) end
set("note", "x")
set("line", "x")
type Place = { point: "TOP" | "BOTTOM", x: number }
local function put<K extends keyof { at: Place }>(key: K, value: ({ at: Place })[K]) print(key, value) end
put("at", { point = "TOP", x = 1 })
put("at", { point = "LEFT", x = 1 })
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"keyofs\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let errors: Vec<&str> = stderr.lines().filter(|l| l.contains("error[")).collect();
    assert_eq!(errors.len(), 10, "{stderr}");
    assert!(
        errors[8].starts_with("src/main.wowlua:33:21: error[E2001]: `\"LEFT\"`"),
        "{stderr}"
    );
    assert!(
        errors[7].starts_with(
            "src/main.wowlua:29:13: error[E2001]: `\"x\"` is not assignable to `number`"
        ),
        "{stderr}"
    );
    assert!(
        errors[6].starts_with(
            "src/main.wowlua:25:7: error[E2001]: `unknown?` is not assignable to `string`"
        ),
        "{stderr}"
    );
    assert!(
        errors[5].starts_with("src/main.wowlua:23:1: error[E2014]: `\"line\" | \"nope\"`"),
        "{stderr}"
    );
    assert!(
        errors[4]
            .starts_with("src/main.wowlua:18:7: error[E2001]: `1` is not assignable to `boolean`"),
        "{stderr}"
    );
    assert!(
        errors[0].starts_with(
            "src/main.wowlua:12:7: error[E2001]: `number` is not assignable to `string`"
        ),
        "{stderr}"
    );
    assert!(
        errors[1].starts_with(
            "src/main.wowlua:13:7: error[E2001]: `boolean` is not assignable to `string`"
        ),
        "{stderr}"
    );
    assert!(
        errors[2].starts_with("src/main.wowlua:14:1: error[E2014]: `\"lnie\"`"),
        "{stderr}"
    );
    assert!(errors[3].starts_with("src/main.wowlua:15:7: error[E2001]: `\"nope\"` is not assignable to `\"line\" | \"figure\" | \"note\"`"), "{stderr}");
}

/// A generic class's generic method reads the class's arguments in its
/// constraints (`K extends keyof D` of `Module<D>`), and a base written
/// with `typeof` (directly or through an alias) is worked out where the
/// file reaches the class (decision L45).
#[test]
fn keyof_in_generic_class_methods() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"class Module<D extends { [string]: unknown }>
    protected readonly defaults: D
    constructor(defaults: D)
        self.defaults = defaults
    end
    function Get<K extends keyof D>(key: K): D[K]
        return self.defaults[key] :: D[K]
    end
end
const DEFAULTS = { line = 20, figure = true, position = { x = 1, y = 2 } }
type Defs = typeof(DEFAULTS)
local probe: Defs = DEFAULTS
local k: keyof Defs = "nope"
class Sub extends Module<Defs>
    constructor() super(DEFAULTS) end
    function T()
        local c: string = self:Get("line")
        print(c)
    end
end
class Sub2 extends Module<typeof(DEFAULTS)>
    constructor() super(DEFAULTS) end
    function T()
        local c: string = self:Get("line")
        print(c)
    end
end
print(probe, k)
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"classkeyof\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let errors: Vec<&str> = stderr.lines().filter(|l| l.contains("error[")).collect();
    assert_eq!(
        errors,
        [
            "src/main.wowlua:13:7: error[E2001]: `\"nope\"` is not assignable to `\"line\" | \"figure\" | \"position\"`",
            "src/main.wowlua:17:15: error[E2001]: `number` is not assignable to `string`",
            "src/main.wowlua:24:15: error[E2001]: `number` is not assignable to `string`",
        ],
        "{stderr}"
    );
}

/// A declared class's `Self` is the receiver (decision L46): a script
/// handler set on a widget class's instance gets that class, so its fields
/// are typed.
#[test]
fn declared_self_is_the_receiver() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"class Toggle extends Button
    knob: number = 1
    constructor() super(nil, UIParent) end
    function Go()
        self:SetScript("OnUpdate", function(b, elapsed)
            local k: string = b.knob
            print(k, elapsed)
        end)
    end
end
local t = Toggle.new()
t:SetScript("OnUpdate", function(b) local k: string = b print(k) end)
local f = CreateFrame("Frame")
f:SetScript("OnShow", function(self) local n: number = self print(n) end)
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"selfrecv\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let errors: Vec<&str> = stderr.lines().filter(|l| l.contains("error[")).collect();
    assert_eq!(
        errors,
        [
            "src/main.wowlua:6:19: error[E2001]: `number` is not assignable to `string`",
            "src/main.wowlua:12:43: error[E2001]: `Toggle` is not assignable to `string`",
            "src/main.wowlua:14:44: error[E2001]: `Frame` is not assignable to `number`",
        ],
        "{stderr}"
    );
}

/// A field added to a table held in a field of an open table is part of the
/// type the file exports, and the nested table is sealed with it (T275).
#[test]
fn nested_fields_are_exported() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"nested\", \"version\": \"1.0.0\" }, \"check\": { \"mode\": \"strict\" } }\n",
            ),
            (
                "src/api.wowlua",
                "local API = {}\nAPI.UI = { A = 1 }\nAPI.UI.B = \"x\"\nexport const Proto = API\n",
            ),
            (
                "src/main.wowlua",
                "import Proto from \"./api\"\nlocal n: number = Proto.UI.B\nprint(n, Proto.UI.Z)\n",
            ),
        ],
    );
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let errors: Vec<&str> = stderr.lines().filter(|l| l.contains("error[")).collect();
    assert_eq!(
        errors,
        [
            "src/main.wowlua:2:7: error[E2001]: `string` is not assignable to `number`",
            "src/main.wowlua:3:19: error[E2003]: `{ A: number, B: string }` has no field `Z`",
        ],
        "{stderr}"
    );
}

/// A value typed elsewhere keeps its declared type when it binds a type
/// parameter, a union of literals included; a literal written there widens
/// (decision L44).
#[test]
fn typed_values_keep_literal_unions() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"type Token = "a" | "b"
local tokens: { [string]: Token } = { x = "a" }
local t: Token = assert(tokens.x)
local function id<T>(v: T): T return v end
local s = id("lit")
local u: "lit" = s
print(t, u)
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"literals\", \"version\": \"1.0.0\" }, \"check\": { \"mode\": \"strict\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let errors: Vec<&str> = stderr.lines().filter(|l| l.contains("error[")).collect();
    assert_eq!(
        errors,
        ["src/main.wowlua:6:7: error[E2001]: `string` is not assignable to `\"lit\"`"],
        "{stderr}"
    );
}

/// `W2063` (decision T274): off unless `check.lints` turns it on; then an
/// unannotated parameter nothing types, a local whose value is `any`, a
/// generic type written without its type arguments (T276), and a field a
/// widget class or a game frame doesn't declare (L51, L79).
#[test]
fn implicit_any_lint() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"local function takes(fn: (x: number) -> ()) fn(1) end
local function loose(fn: (...any) -> ()) fn(1) end
takes(function(x) print(x) end)
loose(function(y) print(y) end)
local t: any = {}
local a = t.field
local _b = t.other
local frame = CreateFrame("Frame")
frame:SetScript("OnUpdate", function(self, elapsed) print(self, elapsed) end)
print(a)
type Box<T> = { item: T }
local boxed: Box = { item = 1 }
local typed: Box<number> = { item = 1 }
print(boxed, typed)
class Switch extends Button
    on: boolean = false
end
local sw = Switch.new(nil, UIParent)
sw.extra = 1
print(sw.on, sw.other, frame.anything)
frame:Paint()
local function paintable(f: Frame): f is Frame & { Paint: (self: Frame) -> () } return f ~= nil end
if paintable(frame) then frame:Paint() end
local made, slider = pcall(CreateFrame, "Slider", nil, UIParent)
if made and slider then slider:SetMinMaxValues(0, 1) end
local owned = CreateFrame("Button") :: Button & { owner: Frame? }
owned:SetScript("OnClick", function(self) if self.owner then self.owner:Hide() end end)
"#;
    let manifest = |lints: &str| {
        format!(
            "{{ \"package\": {{ \"name\": \"implicitany\", \"version\": \"1.0.0\" }}, \"check\": {{ \"mode\": \"strict\"{lints} }} }}\n"
        )
    };
    write(
        dir.path(),
        &[
            ("wowlua.json", manifest("").as_str()),
            ("src/main.wowlua", main),
        ],
    );
    let run = |dir: &std::path::Path| {
        let out = wowlua(dir)
            .args(["check", "--message-format", "short"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let off = run(dir.path());
    assert!(!off.contains("W2063"), "{off}");
    write(
        dir.path(),
        &[(
            "wowlua.json",
            manifest(", \"lints\": { \"W2063\": \"warn\" }").as_str(),
        )],
    );
    let on = run(dir.path());
    let warnings: Vec<&str> = on.lines().filter(|l| l.contains("W2063")).collect();
    assert_eq!(
        warnings,
        [
            "src/main.wowlua:4:16: warning[W2063]: parameter `y` is `any`: nothing gives it a type (annotate it)",
            "src/main.wowlua:6:7: warning[W2063]: `a` is `any`: its value has no type (annotate it, or type where it comes from)",
            "src/main.wowlua:19:4: warning[W2063]: `Switch` doesn't declare `extra`, so it is `any` (declare it in the class)",
            "src/main.wowlua:20:17: warning[W2063]: `Switch` doesn't declare `other`, so it is `any` (declare it in the class)",
            "src/main.wowlua:20:30: warning[W2063]: `Frame` doesn't declare `anything`, so it is `any` (give the value a type that has it: `Frame & { anything: … }`)",
            "src/main.wowlua:21:7: warning[W2063]: `Frame` doesn't declare `Paint`, so it is `any` (give the value a type that has it: `Frame & { Paint: … }`)",
            "src/main.wowlua:12:14: warning[W2063]: `Box` is written without its type arguments, so they are `any` (write `Box<T>`)",
        ],
        "{on}"
    );
}

/// `wowlua:reflect`'s results are typed (decision L36): `isInstance(v, Dog)`
/// narrows `v` to `Dog`, `classOf` gives `ClassOf<Dog>?`, and `enumFrom`
/// and `enumEntries` give the enum.
#[test]
fn reflection_types() {
    let dir = tempfile::tempdir().unwrap();
    let main = r#"import * as reflect from "wowlua:reflect"
class Dog
    name: string = "rex"
end
enum Anchor { Top = "TOP", Bottom = "BOTTOM" }

local function name(v: Dog | number): string
    if reflect.isInstance(v, Dog) then return v.name end
    return tostring(v)
end
local a: Anchor? = reflect.enumFrom(Anchor, "TOP")
local first: Anchor = reflect.enumEntries(Anchor)[1].value
local cls: ClassOf<Dog>? = reflect.classOf(Dog.new())
print(name(Dog.new()), name(4), a, first, cls == Dog)
local wrong: number = reflect.enumFrom(Anchor, "TOP")
print(wrong)
"#;
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"refltypes\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", main),
        ],
    );
    let out = wowlua(dir.path())
        .args(["check", "--message-format", "short"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let errors: Vec<&str> = stderr.lines().filter(|l| l.contains("error[")).collect();
    assert_eq!(
        errors,
        ["src/main.wowlua:15:7: error[E2001]: `Anchor?` is not assignable to `number`"],
        "{stderr}"
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
    // `wowlua fix` applies every fix; `--migrate` brings the manifest to the
    // current schema (the global `--manifest` names the file, as for every
    // command).
    fs::write(dir.path().join("src/quiet.wowlua"), "local unused = 1\n").unwrap();
    // (the type error is left: it has no fix, and fails the command)
    wowlua(dir.path())
        .args(["fix"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Fixed 1 problem in 1 file"));
    assert_eq!(
        fs::read_to_string(dir.path().join("src/quiet.wowlua")).unwrap(),
        ""
    );
    wowlua(dir.path())
        .args(["fix", "--migrate", "--manifest", "wowlua.json"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "wowlua.json uses the current schema",
        ));
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
function methods:CreateFontString() return setmetatable({ objectType = "FontString" }, methods) end
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
    function describe(): string
        return "counter"
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
-- a method replaced on the class reaches frames built afterwards, a
-- subclass's too; a frame built before keeps its own (T268)
local early = FancyButton.new(UIParent)
local C: any = CounterButton
C.describe = function(): string return "patched" end
print(early:describe(), CounterButton.new(UIParent):describe(), FancyButton.new(UIParent):describe())
-- a frame that exists (a secure header's button): the class built on it,
-- its constructor run, no frame created (L42)
local existing = CreateFrame("Button", "Existing", UIParent)
local a = FancyButton.adopt(existing, UIParent)
a:Click()
print(a == existing, a:GetText(), a.count, Plain.adopt(CreateFrame("Frame")).__class == Plain, CounterButton.new(UIParent) ~= existing)
-- a region (a frame makes it): built with adopt only (L43)
class Label extends FontString
    prefix: string = "> "
    constructor(text: string)
        super()
        self:SetText(self.prefix .. text)
    end
end
local label = Label.adopt(CreateFrame("Frame"):CreateFontString(), "hi")
print(label:GetText(), label:GetObjectType())
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
        "Clicked 1x Clicked 2x *3 2 3 Button UIPanelButtonTemplate\nonload row\nFrame true true\ncounter patched patched\ntrue Clicked 1x *3 1 true true\n> hi FontString"
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
        // `adopt` takes a frame of the class's native type (L42).
        (
            "class B extends Button\nend\nprint(B.adopt(CreateFrame(\"Frame\")))\n",
            "E2001",
        ),
        // A region class has no `new`: CreateFrame makes no regions (L43).
        ("class L extends FontString\nend\nprint(L.new())\n", "E2003"),
    ] {
        fs::write(dir.path().join("src/main.wowlua"), src).unwrap();
        wowlua(dir.path())
            .args(["build", "--message-format", "short"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(code));
    }
}

/// Parameter properties (decision L35): a constructor's `private sink:
/// Sink` declares the field and sets it from the argument, before the field
/// initializers, in plain, derived and widget classes.
#[test]
fn parameter_properties_run() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"props\", \"version\": \"1.0.0\" }, \"check\": { \"mode\": \"strict\" } }\n",
            ),
            (
                "src/main.wowlua",
                r#"interface Sink
    function say(text: string)
end
local printer: Sink = { say = function(_: any, text: string) print(text) end }
class Log
    lines: string[] = {}
    constructor(private sink: Sink = printer, public readonly name: string = "log")
    end
    function add(text: string)
        self.lines[#self.lines + 1] = text
        self.sink:say(`{self.name}: {text}`)
    end
end
class Timed extends Log
    constructor(private started: number)
        super(printer, "timed")
    end
    function since(): number
        return self.started
    end
end
class Label extends Frame
    constructor(parent: Frame, private text: string)
        super(nil, parent)
    end
    function shown(): string
        return self.text
    end
end
local log = Log.new()
log:add("a")
local t = Timed.new(7)
t:add("b")
print(t:since(), #t.lines, Label.new(UIParent, "hi"):shown())
"#,
            ),
        ],
    );
    wowlua(dir.path()).arg("build").assert().success();
    let dist = dir.path().join("dist/dev/Props");
    let toc = fs::read_to_string(dist.join("Props.toc")).unwrap();
    fs::write(dist.join("mock.lua"), MOCK_FRAMES).unwrap();
    fs::write(dist.join("Props.toc"), format!("mock.lua\n{toc}")).unwrap();
    assert_eq!(run_addon(&dist, "Props"), "log: a\ntimed: b\n7 1 hi");
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
        (
            "class A\n constructor(f: () -> ())\n  f()\n end\nend\nclass B extends A\n n: number = 1\n constructor()\n  super(function() print(self.n) end)\n end\nend\n",
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

/// One definition that runs while several files load (a function they all
/// call) is not an ambiguity between them (no W1033).
#[test]
fn load_order_shared_definitions_are_not_ambiguous() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"shared-def\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/a_lib.lua",
                "local _, ns = ...\nfunction ns.make()\n  local t = {}\n  t.ready = true\n  return t.ready\nend\n",
            ),
            ("src/b_one.lua", "local _, ns = ...\nns.one = ns.make()\n"),
            ("src/c_two.lua", "local _, ns = ...\nns.two = ns.make()\n"),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("W1033"), "{stderr}");
}

/// Test files load after the addon (outside its TOC): a test that takes a
/// game table away and back (`_G.C_EventUtils = nil`, `_G[name] = nil`)
/// defines nothing for the addon, so the addon's load-time read of it is
/// no W1034 and the dynamic write no W1033. The same writes in a source
/// file still are.
#[test]
fn load_order_ignores_test_files_writes() {
    let dir = tempfile::tempdir().unwrap();
    let read = "local valid = not (C_EventUtils and C_EventUtils.IsEventValid) or C_EventUtils.IsEventValid(\"X\")\nprint(valid)\n";
    let writes = "local function strip(name: string)\n    local saved = _G[name]\n    _G[name] = nil\n    _G[name] = saved\nend\nlocal function away()\n    local utils = _G.C_EventUtils\n    _G.C_EventUtils = nil\n    _G.C_EventUtils = utils\nend\n";
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"core\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/main.wowlua", read),
            (
                "tests/main.test.wowlua",
                &format!(
                    "import test from \"wowlua:test\"\n{writes}@test\nlocal function t() strip(\"GetTime\"); away() end\n"
                ),
            ),
        ],
    );
    let check = |dir: &Path| {
        let out = wowlua(dir)
            .args(["check", "--message-format", "short"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let stderr = check(dir.path());
    assert!(
        !stderr.contains("W1033") && !stderr.contains("W1034"),
        "{stderr}"
    );
    // The same writes in a source file.
    write(
        dir.path(),
        &[(
            "src/reset.wowlua",
            &format!("{writes}export function reset() strip(\"GetTime\"); away() end\n"),
        )],
    );
    let stderr = check(dir.path());
    assert!(
        stderr.contains("W1034") && stderr.contains("W1033"),
        "{stderr}"
    );
}

/// Files another AddOn loads (`build.shared-files`) are shipped, not
/// loaded by this one: their reads do not order or warn here.
#[test]
fn load_order_ignores_shared_files() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"core\", \"version\": \"1.0.0\" }, \"build\": { \"shared-files\": [\"src/options.lua\"] } }\n",
            ),
            (
                "src/core.lua",
                "Core = {}\nfunction Core.Init() Core.db = {} end\n",
            ),
            // Loaded by the options addon, after Core.Init has run.
            ("src/options.lua", "print(Core.db)\n"),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("W1033") && !stderr.contains("W1034"),
        "{stderr}"
    );
}

/// A function passed to a known API function (`type`, `tinsert`) is not
/// called by it: a registration helper that checks and stores its handler
/// does not make the handler run at load time.
#[test]
fn load_order_api_functions_do_not_call_their_arguments() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"handlers\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/a_api.lua",
                "local _, ns = ...\nlocal list = {}\nfunction ns.Register(fn)\n  if type(fn) ~= \"function\" then print(\"not a function\", fn) return end\n  tinsert(list, fn)\nend\n",
            ),
            (
                "src/b_register.lua",
                "local _, ns = ...\nns.Register(function() print(ns.Late) end)\n",
            ),
            ("src/c_late.lua", "local _, ns = ...\nns.Late = 1\n"),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("W1034") && !stderr.contains("W1035"),
        "{stderr}"
    );
    let toc = fs::read_to_string(dir.path().join("dist/dev/Handlers/Handlers.toc")).unwrap();
    let pos = |name: &str| toc.find(name).unwrap_or_else(|| panic!("{name}: {toc}"));
    assert!(pos("a_api.lua") < pos("b_register.lua"), "{toc}");
    assert!(
        pos("b_register.lua") < pos("c_late.lua"),
        "the handler does not run while b_register loads: {toc}"
    );
}

/// A callback passed through a class (`bus:listen` → `self:register` →
/// `self.dispatcher:on`) to an interface method whose parameter is
/// `@deferred` runs later: no load-time cycle (decision T269). Without the
/// mark, the call through the interface is unknown and the callback counts
/// as called at load.
#[test]
fn load_order_follows_classes_and_deferred_interfaces() {
    let dir = tempfile::tempdir().unwrap();
    let bus = |deferred: &str| {
        format!(
            r#"export interface Dispatcher
    function on(name: string, {deferred}fn: () -> ())
end
class Store implements Dispatcher
    handlers: {{ [string]: () -> () }} = {{}}
    function on(name: string, fn: () -> ())
        self.handlers[name] = fn
    end
end
export class Bus
    constructor(private dispatcher: Dispatcher = Store.new()) end
    private function register(name: string, fn: () -> ())
        self.dispatcher:on(name, fn)
    end
    function listen(name: string, fn: () -> ())
        self:register(name, fn)
    end
end
export const bus = Bus.new()
export function on(name: string, fn: () -> ())
    bus:listen(name, fn)
end
"#
        )
    };
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"busorder\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/Bus.wowlua", &bus("@deferred ")),
            (
                "src/Register.wowlua",
                "import on from \"/Bus\"\nimport * as late from \"/Late\"\nexport function ready(): boolean return true end\non(\"X\", function() print(late.value) end)\n",
            ),
            (
                "src/Late.wowlua",
                "import ready from \"/Register\"\nexport const value = ready() and 1 or 0\n",
            ),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("E1014") && !stderr.contains("W1034"),
        "{stderr}"
    );
    // Without `@deferred` the callback may run at load: a cycle.
    fs::write(dir.path().join("src/Bus.wowlua"), bus("")).unwrap();
    wowlua(dir.path())
        .args(["build", "--message-format", "short"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("E1014"));
}

/// `C.new(…)` at load runs C's constructor and field initializers (its
/// superclass's too): what they read orders the files (decision T270). A
/// default instance whose constructor reads another module's export loads
/// after that module, though its name sorts first.
#[test]
fn load_order_follows_constructors() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"ctororder\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/a_queue.wowlua",
                "import * as log from \"/b_log\"\nclass Base\n    seen: string = log.name\nend\nexport class Queue extends Base\n    constructor(private reporter: string = log.name)\n        super()\n    end\nend\nexport const queue = Queue.new()\n",
            ),
            (
                "src/b_log.wowlua",
                "import * as q from \"/a_queue\"\nexport const name = \"log\"\nexport function size(): number return q.queue and 1 or 0 end\n",
            ),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("W1034"), "{stderr}");
    let toc = fs::read_to_string(dir.path().join("dist/dev/Ctororder/Ctororder.toc")).unwrap();
    let pos = |name: &str| toc.find(name).unwrap_or_else(|| panic!("{name}: {toc}"));
    assert!(pos("b_log") < pos("a_queue"), "{toc}");
}

/// Files in another flavor's folder of a variable file (`db/[Game]/…`) do
/// not load in this flavor: they define nothing for its load order (no
/// W1033), and a build of some flavors leaves the others' out of the TOC.
#[test]
fn load_order_variable_file_folders() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "games", "version": "1.0.0" }, "flavors": { "camelot": {}, "vanilla": {} },
  "build": { "variable-files": [ { "path": "src/db/[Game]/Data.lua" } ] } }
"#,
            ),
            ("src/main.lua", "local _, ns = ...\nns.L = ns.L or {}\n"),
            (
                "src/db/Camelot/Data.lua",
                "local _, ns = ...\nns.L.TITLE = \"camelot\"\n",
            ),
            (
                "src/db/Vanilla/Data.lua",
                "local _, ns = ...\nns.L.TITLE = \"vanilla\"\n",
            ),
            // A folder no flavor names (loaded by no TOC line).
            (
                "src/db/VanillaSOD/Data.lua",
                "local _, ns = ...\nns.L.TITLE = \"sod\"\n",
            ),
            ("src/z_use.lua", "local _, ns = ...\nprint(ns.L.TITLE)\n"),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("W1033"), "{stderr}");
    wowlua(dir.path())
        .args(["build", "--flavor", "camelot"])
        .assert()
        .success();
    let tocs: Vec<String> = fs::read_dir(dir.path().join("dist/dev/Games"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toc"))
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    let toc = tocs.join("\n");
    assert!(toc.contains("[Game]"), "{toc}");
    assert!(!toc.contains("Vanilla\\Data.lua"), "{toc}");
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

/// Fields of global and `ns` tables order files like globals do (issue #1).
#[test]
fn load_order_from_table_fields() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"fields\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/a_init.lua",
                "local _, ns = ...\nLib = {}\nSuuUI = {}\nns.API = SuuUI\nns.Tools = {}\nns.UI = {}\n",
            ),
            // `function X.f` and `X.f = value`.
            ("src/b_user.lua", "print(Lib.Register(1))\n"),
            ("src/c_api.lua", "function Lib.Register(x) return x end\n"),
            ("src/d_value_user.lua", "print(Lib.value)\n"),
            ("src/e_value_api.lua", "Lib.value = 42\n"),
            // Below an `ns` key.
            (
                "src/f_ns_user.lua",
                "local _, ns = ...\nprint(ns.Tools.Run())\n",
            ),
            (
                "src/g_ns_api.lua",
                "local _, ns = ...\nfunction ns.Tools.Run() return \"run\" end\n",
            ),
            // Through a local alias of a global, and of an `ns` table.
            ("src/h_alias_user.lua", "print(Lib.Alias(), Lib:Method())\n"),
            (
                "src/i_alias_api.lua",
                "local API = Lib\nfunction API.Alias() return \"alias\" end\nfunction API:Method() return \"method\" end\n",
            ),
            (
                "src/j_ui_user.lua",
                "local _, ns = ...\nprint(ns.UI.Icon())\n",
            ),
            (
                "src/k_ui_api.lua",
                "local _, ns = ...\nlocal UI = ns.UI\nfunction UI.Icon() return \"icon\" end\n",
            ),
            // `ns.API = SuuUI`: both names hold one table.
            (
                "src/l_module.lua",
                "print(SuuUI.RegisterModule({ name = \"module\" }))\n",
            ),
            (
                "src/m_api.lua",
                "local _, ns = ...\nlocal API = ns.API\nlocal Modules = {}\nfunction Modules.Register(m) return m.name end\nAPI.RegisterModule = Modules.Register\n",
            ),
            // A local table that an `ns` key holds (`ns.Roles = Roles`).
            (
                "src/p_roles_user.lua",
                "local _, ns = ...\nprint(ns.Roles.Current())\n",
            ),
            (
                "src/q_roles_api.lua",
                "local _, ns = ...\nlocal Roles = { label = \"roles\" }\nns.Roles = Roles\nfunction Roles.Current() return Roles.label end\n",
            ),
            // Guarded field uses do not order files.
            (
                "src/n_guarded.lua",
                "if Lib.Maybe then Lib.Maybe() end\nlocal y = Lib.Other and Lib.Other.y\n",
            ),
            (
                "src/o_maybe.lua",
                "Lib.Maybe = function() end\nLib.Other = { y = 1 }\n",
            ),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("W1033") && !stderr.contains("W1034"),
        "{stderr}"
    );
    let dist = dir.path().join("dist/dev/Fields");
    let toc = fs::read_to_string(dist.join("Fields.toc")).unwrap();
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
    for (definer, user) in [
        ("c_api.lua", "b_user.lua"),
        ("e_value_api.lua", "d_value_user.lua"),
        ("g_ns_api.lua", "f_ns_user.lua"),
        ("i_alias_api.lua", "h_alias_user.lua"),
        ("k_ui_api.lua", "j_ui_user.lua"),
        ("m_api.lua", "l_module.lua"),
        ("q_roles_api.lua", "p_roles_user.lua"),
    ] {
        assert!(pos(definer) < pos(user), "{definer} before {user}: {toc}");
    }
    assert!(
        pos("n_guarded.lua") < pos("o_maybe.lua"),
        "guarded field uses do not order: {toc}"
    );
    let mut lines: Vec<String> = run_addon(&dist, "Fields")
        .lines()
        .map(str::to_string)
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        ["1", "42", "alias method", "icon", "module", "roles", "run"]
    );
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
            // So are optional links and `??`: they expect nil.
            (
                "src/a_probe.wowlua",
                "declare global Later: { x: number }?\nprint(Later?.x, Later ?? 0)\n",
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
    assert!(
        pos("a_probe.lua") < pos("z_later.lua"),
        "probes do not order: {toc}"
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
        // Fields: several load-time definers, a late definition, and a
        // write through a table the analysis cannot trace.
        (
            &[
                ("src/a.lua", "Lib = {}\n"),
                ("src/b.lua", "function Lib.f() end\n"),
                ("src/c.lua", "Lib.f = 1\n"),
                ("src/d.lua", "print(Lib.f)\n"),
            ],
            "W1033",
            "files defining `Lib.f` cannot be determined",
        ),
        (
            &[
                ("src/a.lua", "Lib = {}\nprint(Lib.late)\n"),
                ("src/b.lua", "function init_lib() Lib.late = 1 end\n"),
            ],
            "W1034",
            "`Lib.late` is read while the file loads",
        ),
        (
            &[
                ("src/a.lua", "Lib = {}\n"),
                ("src/b.lua", "print(Lib.f)\n"),
                (
                    "src/c.lua",
                    "local function get() return Lib end\nget().f = 1\n",
                ),
            ],
            "W1033",
            "the definition of `Lib.f` cannot be traced",
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

/// Files whose order the layout fixes are not ambiguous: an XML file loads
/// its scripts in document order, so two of them defining the same name at
/// load time (a locale file overriding the default) get no W1033. Two files
/// whose order nothing fixes still do (`load_order_diagnostics`).
#[test]
fn load_order_files_one_xml_loads_are_ordered() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"xmlpair\", \"version\": \"1.0.0\" } }\n",
            ),
            (
                "src/locales.xml",
                "<Ui>\n  <Script file=\"loc/enUS.lua\"/>\n  <Script file=\"loc/deDE.lua\"/>\n</Ui>\n",
            ),
            ("src/loc/enUS.lua", "Greeting = \"Hello\"\n"),
            (
                "src/loc/deDE.lua",
                "if GetLocale() ~= \"deDE\" then return end\nGreeting = \"Hallo\"\n",
            ),
            ("src/main.lua", "print(Greeting)\n"),
        ],
    );
    let assert = wowlua(dir.path()).arg("build").assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("W1033"), "{stderr}");
}

/// A load-on-demand addon loads after its required addons have run, so a
/// field it reads at load time of a table one of them owns (a global the
/// addon never defines) may be set by that addon, even if the addon itself
/// only writes it at run time: no W1034. Its own SavedVariables are set
/// only after its files load, so reading them still warns, as does the
/// same read in an addon that loads with the others.
#[test]
fn load_order_load_on_demand_reads_its_dependencies_tables() {
    let manifest = |lod: bool| {
        format!(
            "{{ \"package\": {{ \"name\": \"gui\", \"version\": \"1.0.0\" }},\n  \"addon\": {{ \"name\": \"Gui\", \"load-on-demand\": {lod}, \"saved-variables\": [\"GuiDB\"] }},\n  \"addon-dependencies\": {{ \"required\": [\"Core\"] }} }}\n"
        )
    };
    let files = [
        ("src/a.lua", "print(Core.Options.x, GuiDB.size)\n"),
        (
            "src/b.lua",
            "function import_profile(t) Core.Options = t GuiDB.size = 1 end\n",
        ),
    ];
    for lod in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let manifest = manifest(lod);
        let mut all = vec![("wowlua.json", manifest.as_str())];
        all.extend(files);
        write(dir.path(), &all);
        let assert = wowlua(dir.path()).arg("build").assert().success();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(
            stderr.contains("`GuiDB.size` is read while the file loads"),
            "{lod}: {stderr}"
        );
        assert_eq!(
            stderr.contains("`Core.Options` is read while the file loads"),
            !lod,
            "{lod}: {stderr}"
        );
    }
}

/// Code in a branch that runs only when a parameter is not a function
/// (`if type(p) ~= "function"`, `if not p`, the `else` of
/// `if type(p) == "function"`) cannot run while every call reaching the
/// function passes a function for it: the late `Late` it reads is no W1034.
/// A table or an untraceable value may be anything, and a parameter the
/// function reassigns says nothing about the caller: those still warn.
#[test]
fn load_order_branches_for_non_function_arguments() {
    let check = "if type(handler) ~= \"function\" then print(Late) return end";
    let cases: &[(String, &str, bool)] = &[
        (check.to_string(), "Register(\"x\", function() end)", false),
        (
            check.to_string(),
            "local function on() end\nRegister(\"x\", on)",
            false,
        ),
        (check.to_string(), "Register(\"x\", Handlers.OnLoad)", false),
        (check.to_string(), "Register(\"x\", {})", true),
        (check.to_string(), "Register(\"x\", Unknown())", true),
        (
            format!("handler = handler or nil\n{check}"),
            "Register(\"x\", function() end)",
            true,
        ),
        (
            "if not handler then print(Late) end".to_string(),
            "Register(\"x\", function() end)",
            false,
        ),
        (
            "if type(handler) == \"function\" then handler() else print(Late) end".to_string(),
            "Register(\"x\", function() end)",
            false,
        ),
    ];
    for (body, call, warns) in cases {
        let dir = tempfile::tempdir().unwrap();
        let api = format!("function Register(name, handler)\n{body}\nend\n");
        write(
            dir.path(),
            &[
                (
                    "wowlua.json",
                    "{ \"package\": { \"name\": \"branches\", \"version\": \"1.0.0\" } }\n",
                ),
                ("src/a_api.lua", api.as_str()),
                (
                    "src/b_late.lua",
                    "Handlers = {}\nfunction Handlers.OnLoad() end\nfunction init() Late = 1 end\n",
                ),
                ("src/c_use.lua", call),
            ],
        );
        let assert = wowlua(dir.path()).arg("build").assert().success();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert_eq!(
            stderr.contains("`Late` is read while the file loads"),
            *warns,
            "{body} / {call}: {stderr}"
        );
    }
}

/// Lua compile results are kept in `$WOWLUA_HOME/cache/compile/` (#50): a
/// second run takes them from there and reports exactly the same (debug
/// builds, and so this test, also compile again and compare), and
/// `WOWLUA_NO_COMPILE_CACHE` leaves the cache alone.
#[test]
fn compile_results_are_kept_between_runs() {
    let dir = tempfile::tempdir().unwrap();
    let locals: String = (0..201).map(|i| format!("local v{i} = {i}\n")).collect();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"cached\", \"version\": \"1.0.0\" } }\n",
            ),
            ("src/ok.lua", "local a = 1\nprint(a)\n"),
            ("src/limit.lua", locals.as_str()),
        ],
    );
    let entries = |home: &Path| {
        fs::read_dir(home.join("cache/compile"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .count()
    };
    let run = |envs: &[(&str, &str)]| {
        let mut cmd = wowlua(dir.path());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let output = cmd.arg("check").assert().code(1).get_output().clone();
        // Without the summary line, which has the time.
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|l| !l.contains(" in ") || l.contains('['))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let cold = run(&[]);
    assert!(cold.contains("E5001"), "{cold}");
    let home = dir.path().join(".wowlua-home");
    let stored = entries(&home);
    assert!(stored >= 2, "{stored}");
    let warm = run(&[]);
    assert_eq!(warm, cold);
    assert_eq!(entries(&home), stored, "a warm run adds nothing");
    std::fs::remove_dir_all(home.join("cache/compile")).unwrap();
    let off = run(&[("WOWLUA_NO_COMPILE_CACHE", "1")]);
    assert_eq!(off, cold);
    assert!(!home.join("cache/compile").exists());
}

/// Runs compiled files in order in one Lua state, each as a chunk called
/// with `("Compiled", ns)` like the client does; returns what they printed.
fn run_compiled(files: &[std::path::PathBuf]) -> String {
    let lua = mlua::Lua::new();
    lua.load("OUT = {} print = function(...) local t = {} for i = 1, select('#', ...) do t[i] = tostring((select(i, ...))) end OUT[#OUT + 1] = table.concat(t, ' ') end debugprofilestop = function() return os.clock() * 1000 end")
        .exec()
        .unwrap();
    let ns = lua.create_table().unwrap();
    for file in files {
        let code = fs::read_to_string(file).unwrap();
        lua.load(&code)
            .set_name(file.to_string_lossy())
            .call::<()>(("Compiled", ns.clone()))
            .unwrap();
    }
    lua.load("return table.concat(OUT, '\\n')").eval().unwrap()
}

/// `wowlua compile`: files without a project compile with the defaults;
/// one output goes to stdout, several into `-o <dir>` with the load order;
/// `--bundle` gives one file in the build's bundle format.
#[test]
fn compile_without_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        &[
            (
                "x/plain.wowlua",
                "local function add(a: number, b: number): number\n    return a + b\nend\nlocal total = 0\ntotal += add(1, 2)\nprint(total)\n",
            ),
            (
                "x/util.wowlua",
                "export function double(n: number): number\n    return n * 2\nend\n",
            ),
            (
                "x/main.wowlua",
                "import double from \"./util\"\nprint(double(21))\n",
            ),
            ("x/bad.wowlua", "local n: number = \"no\"\nprint(n)\n"),
        ],
    );
    // One plain file: plain Lua on stdout, no runtime.
    let out = wowlua(root)
        .args(["compile", "--release", "--no-bundle", "x/plain.wowlua"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let lua = String::from_utf8(out).unwrap();
    assert!(lua.contains("total = total + "), "{lua}");
    assert!(!lua.contains(": number"), "{lua}");

    // Several outputs need a directory.
    wowlua(root)
        .args(["compile", "x/util.wowlua", "x/main.wowlua"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("pass `-o <dir>`"));
    wowlua(root)
        .args(["compile", "x/util.wowlua", "x/main.wowlua", "-o", "out"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Load order:\n  wowlua/runtime.lua\n  src/util.lua\n  src/main.lua\n",
        ));
    let order: Vec<_> = ["wowlua/runtime.lua", "src/util.lua", "src/main.lua"]
        .iter()
        .map(|p| root.join("out").join(p))
        .collect();
    assert_eq!(run_compiled(&order), "42");

    // A bundle runs on its own.
    wowlua(root)
        .args([
            "compile",
            "--bundle",
            "x/util.wowlua",
            "x/main.wowlua",
            "-o",
            "all.lua",
        ])
        .assert()
        .success();
    let bundle = fs::read_to_string(root.join("all.lua")).unwrap();
    assert!(bundle.contains("__wl_load(\"src/main\""), "{bundle}");
    assert_eq!(run_compiled(&[root.join("all.lua")]), "42");

    // Type errors fail.
    wowlua(root)
        .args(["compile", "x/bad.wowlua"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("E2001"));
}

/// Inside a project, `wowlua compile` uses its manifest: its profiles and
/// its declaration files.
#[test]
fn compile_in_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        &[
            (
                "wowlua.json",
                "{ \"package\": { \"name\": \"demo\", \"version\": \"1.0.0\" },\n  \"profiles\": { \"plain\": { \"inherits\": \"release\", \"bundle\": false, \"minify\": \"none\", \"optimize\": { \"cache-globals\": \"off\" } } } }\n",
            ),
            (
                "src/api.d.wowlua",
                "declare global Greet: (name: string) -> string\n",
            ),
            ("src/main.wowlua", "print(Greet(\"you\"))\n"),
            ("src/other.wowlua", "local s: string = Greet(1)\nprint(s)\n"),
        ],
    );
    wowlua(root)
        .args(["compile", "--profile", "plain", "src/main.wowlua"])
        .assert()
        .success()
        .stdout("print(Greet(\"you\"))\n");
    // Only the given files are compiled.
    wowlua(root)
        .args(["compile", "--profile", "plain", "src/other.wowlua"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("E2001").or(predicate::str::contains("E2012")));
}

/// Source maps keep their bytes: a dev build's maps of wowlua modules, and a
/// release bundle's map with renamed names and a member without a map.
#[test]
fn source_maps_keep_their_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let dev = dir.path().join("dev");
    write(
        &dev,
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "maps", "version": "1.0.0" },
  "flavors": { "standard": {} },
  "profiles": { "dev": { "instrument": "none", "monitor": false } } }
"#,
            ),
            (
                "src/util.wowlua",
                "export function greet(name: string): string\n    local message = \"hello, \" .. name\n    return message\nend\n",
            ),
            (
                "src/main.wowlua",
                "import greet from \"./util\"\nlocal function shout(text: string): string\n    local loud = text:upper()\n    return loud\nend\nprint(shout(greet(\"map\")))\n",
            ),
        ],
    );
    wowlua(&dev).arg("build").assert().success();
    let maps = dev.join("dist/dev/.maps/Maps/src");
    assert_eq!(
        fs::read_to_string(maps.join("main.lua.map")).unwrap(),
        DEV_MAIN
    );
    assert_eq!(
        fs::read_to_string(maps.join("util.lua.map")).unwrap(),
        DEV_UTIL
    );
    let release = dir.path().join("release");
    write(
        &release,
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "maps", "version": "1.0.0" },
  "flavors": { "standard": {} },
  "profiles": { "release": { "minify": "rename", "runtime-checks": "none" } } }
"#,
            ),
            (
                "src/a.lua",
                "local counter = 0\nfunction PlainTick()\n    local step = 1\n    counter = counter + step\n    return counter\nend\n",
            ),
            (
                "src/b.lua",
                "local function twice(value)\n    return value * 2\nend\nPlainTwice = twice\n",
            ),
            (
                "src/libs/Tiny/Tiny.lua",
                "Tiny = Tiny or {}\nfunction Tiny.id(x) return x end\n",
            ),
        ],
    );
    wowlua(&release)
        .args(["build", "--release"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(release.join("dist/release/.maps/Maps/Maps.lua.map")).unwrap(),
        RELEASE_BUNDLE
    );
}

const DEV_MAIN: &str = r#"{"version":3,"file":"main.lua","sources":["src/main.wowlua"],"names":[],"mappings":"AAAA,MAAA,OAAA,OAAA,GAAA,cAAA,MAAO,QAAP,aAAA;AACA,eAAe,KAAK,CAAC;CACjB,MAAM,OAAO,KAAK,MAAM;CACxB,OAAO;AACX;AACA,MAAM,MAAM,MAAM,KAAK,CAAC,CAAC"}
"#;
const DEV_UTIL: &str = r#"{"version":3,"file":"util.lua","sources":["src/util.wowlua"],"names":[],"mappings":"AAAA,MAAA,OAAA,OAAA,GAAA,cAAA,MAAA,eAAA,YAAA,QAAA,MAAA,MAAgB,QAAT,QAAc,CAAC,MAAD,GAAA,IAAA,CAAA,KAAC,SAAD,eAAA,gBAAA,GAAA,QAAA,SAAA,qBAAA,UAAC;CAClB,MAAM,UAAU,UAAU,GAAG;CAC7B,OAAO;AACX,IAHA,qBAAgB"}
"#;
const RELEASE_BUNDLE: &str = r#"{"version":3,"file":"Maps.lua","sources":["src/a.lua","src/b.lua","src/libs/Tiny/Tiny.lua"],"names":["counter","step","twice","value"],"mappings":";;;;;;;;;AAAA,MAAMA,EAAU;AAChB,SAAS,SAAS;AACd,MAAMC,EAAO;AACbD,EAAUA,CAAQ,CAAEC;AACpB,OAAOD;AACX;;;ACLA,eAAeE,CAAK,CAACC;AACjB,OAAOA,CAAM,CAAE;AACnB;AACA,WAAaD;;;ACHb;AACA;AACA"}
"#;
/// `wowlua dev` keeps what it compiled between builds: after an edit, its
/// `dist/` is what a fresh `wowlua build` of the same tree writes.
#[test]
fn dev_rebuilds_match_a_fresh_build() {
    use std::io::{BufRead, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let addon = dir.path().join("DevAddon");
    write(
        &addon,
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "dev-addon", "version": "1.0.0" },
  "flavors": { "standard": {}, "vanilla": {} } }
"#,
            ),
            ("src/a.lua", "local _, ns = ...\nns.a = 1\n"),
            ("src/b.lua", "local _, ns = ...\nprint(ns.a)\n"),
            (
                "src/util.wowlua",
                "export function double(n: number): number return n * 2 end\n",
            ),
            (
                "src/main.wowlua",
                "import double from \"./util\"\nprint(double(2))\n",
            ),
        ],
    );
    let wow = dir.path().join("World of Warcraft");
    fs::create_dir_all(wow.join("_retail_")).unwrap();
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("wowlua"))
        .current_dir(&addon)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", dir.path().join(".wowlua-home"))
        .env_remove("WOWLUA_WOW_PATH")
        .args(["dev", "--link", "--exit-after", "2", "--wow"])
        .arg(&wow)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // A missed file event would leave `dev` waiting: fail instead of hanging.
    let (done, finished) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        if finished
            .recv_timeout(std::time::Duration::from_secs(120))
            .is_err()
        {
            eprintln!("wowlua dev did not finish its second build");
            std::process::abort();
        }
    });
    let mut log = String::new();
    let mut ready = 0;
    for line in BufReader::new(child.stderr.take().unwrap()).lines() {
        let line = line.unwrap();
        log.push_str(&line);
        log.push('\n');
        if line.contains("Ready") {
            ready += 1;
        }
        if line.contains("Watching") {
            fs::write(addon.join("src/a.lua"), "local _, ns = ...\nns.a = 2\n").unwrap();
        }
    }
    assert!(child.wait().unwrap().success(), "{log}");
    let _ = done.send(());
    assert_eq!(ready, 2, "{log}");
    let tree = |root: &Path| -> Vec<(String, Vec<u8>)> {
        files_in(root)
            .into_iter()
            .map(|f| {
                let bytes = fs::read(root.join(&f)).unwrap();
                (f, bytes)
            })
            .collect()
    };
    let dev = tree(&addon.join("dist"));
    assert!(
        dev.iter().any(|(f, bytes)| f.ends_with("src/a.lua")
            && String::from_utf8_lossy(bytes).contains("ns.a = 2")),
        "the edit is built"
    );
    fs::remove_dir_all(addon.join("dist")).unwrap();
    wowlua(&addon)
        .env("WOWLUA_HOME", dir.path().join(".wowlua-home"))
        .arg("build")
        .assert()
        .success();
    assert!(
        dev == tree(&addon.join("dist")),
        "dev and a fresh build differ"
    );
}

/// Runs end with a count of their errors and warnings, as rustc's do, in
/// the human and short formats; `-q` and the machine formats leave it out.
#[test]
fn runs_end_with_a_summary_of_their_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &[
            (
                "wowlua.json",
                r#"{ "package": { "name": "summary", "version": "1.0.0" },
  "flavors": { "standard": {} }, "check": { "lua": true } }
"#,
            ),
            ("src/a.lua", "local unused = 1\nlocal x = y\nprint(x)\n"),
        ],
    );
    let stderr = |args: &[&str]| {
        let out = wowlua(dir.path()).arg("check").args(args).output().unwrap();
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    for args in [&[][..], &["--message-format", "short"]] {
        let text = stderr(args);
        assert!(
            text.contains("\nwarning: 2 warnings emitted\n"),
            "{args:?}: {text}"
        );
    }
    for args in [
        &["-q"][..],
        &["--message-format", "json"],
        &["--message-format", "github"],
    ] {
        let text = stderr(args);
        assert!(!text.contains("emitted"), "{args:?}: {text}");
    }
    write(dir.path(), &[("src/b.lua", "x = = 1\n")]);
    let text = stderr(&["--message-format", "short"]);
    assert!(
        text.ends_with("error: aborting due to 1 previous error; 2 warnings emitted\n"),
        "{text}"
    );
}
