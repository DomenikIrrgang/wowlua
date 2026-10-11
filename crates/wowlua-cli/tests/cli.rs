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
        .stderr(predicate::str::contains("src/main.lua:1:1: error[E0101]"));
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
