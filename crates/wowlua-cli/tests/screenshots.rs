//! Screenshots in tests and `wowlua screenshot` (spec debugger §18.12.3,
//! §18.12.4). Skipped where there is no GPU adapter.

use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn wowlua(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("wowlua").unwrap();
    cmd.current_dir(dir)
        .env("NO_COLOR", "1")
        .env("WOWLUA_HOME", dir.join(".wowlua-home"))
        .env("WOWLUA_WOW_PATH", dir.join("no-wow-here"));
    cmd
}

fn box_src(r: f64) -> String {
    format!(
        "local f = CreateFrame('Frame', 'DemoBox', UIParent)
f:SetSize(120, 60)
f:SetPoint('CENTER')
local t = f:CreateTexture(nil, 'BACKGROUND')
t:SetAllPoints()
t:SetColorTexture({r}, 0.5, 0.2, 1)
"
    )
}

const TEST: &str = r#"import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("the box")
local function looks()
    expect(_G.DemoBox :: any).toMatchScreenshot("box")
    expect(mock.screen).toMatchScreenshot("screen")
    local path = mock.screenshot("shot")
    expect(path:find("shot.png", 1, true) ~= nil).toBe(true)
end
"#;

fn run(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = wowlua(dir).args(args).output().unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn screenshots_are_written_compared_and_updated() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("wowlua.json"),
        r#"{ "package": { "name": "demo", "version": "1.0.0" }, "flavors": { "standard": {} }, "addon": { "title": "Demo" } }"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::write(root.join("src/box.lua"), box_src(0.9)).unwrap();
    fs::write(root.join("tests/box.test.wowlua"), TEST).unwrap();

    let (ok, text) = run(root, &["test"]);
    if text.contains("no GPU adapter") {
        eprintln!("skipped: {text}");
        return;
    }
    assert!(ok, "{text}");
    let shots = root.join("tests/__screenshots__/box.test.wowlua");
    let stored = shots.join("the-box-box.png");
    assert!(stored.exists(), "{text}");
    assert!(shots.join("the-box-screen.png").exists());
    let first = fs::read(&stored).unwrap();
    // The frame's image is its size: 120 by 60 units at the screen's scale.
    let img = wowlua_render_size(&first);
    assert!(img.0 > 100 && img.1 > 50 && img.0 < 300, "{img:?}");

    // Unchanged: it matches.
    let (ok, text) = run(root, &["test"]);
    assert!(ok, "{text}");

    // Another color: it fails, with the images beside it.
    fs::write(root.join("src/box.lua"), box_src(0.1)).unwrap();
    let (ok, text) = run(root, &["test"]);
    assert!(!ok);
    assert!(
        text.contains("the screenshot \"the box box\" differs"),
        "{text}"
    );
    assert!(shots.join("the-box-box.diff.png").exists());
    assert!(shots.join("the-box-box.actual.png").exists());

    // -u takes the new one.
    let (ok, text) = run(root, &["test", "-u"]);
    assert!(ok, "{text}");
    assert_ne!(fs::read(&stored).unwrap(), first);
    assert!(!shots.join("the-box-box.diff.png").exists());

    // `wowlua screenshot`: the frame as the test left it.
    let out = root.join("box.png");
    let (ok, text) = run(
        root,
        &[
            "screenshot",
            "tests/box.test.wowlua",
            "--frame",
            "DemoBox",
            "--scale",
            "2",
            "-o",
            out.to_str().unwrap(),
        ],
    );
    assert!(ok, "{text}");
    let size = wowlua_render_size(&fs::read(&out).unwrap());
    assert_eq!(size.0, img.0 * 2, "{size:?} vs {img:?}");
}

/// A PNG's width and height (its IHDR).
fn wowlua_render_size(png: &[u8]) -> (u32, u32) {
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
    (w, h)
}
