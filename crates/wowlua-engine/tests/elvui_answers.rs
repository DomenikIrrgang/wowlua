//! What ElvUI v16 asked of the engine at login: undocumented events, aura
//! containers as frames, the regions widgets are made with, curve colors,
//! explicit protection from XML and secure addons' unforbidden frames.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use std::path::PathBuf;

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    vm
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    let text = |vm: &Vm, v: Value| match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// Writes `files` under a fresh directory; returns its `Interface` folder.
fn tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("wowlua-elvui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in files {
        let p = root.join("Interface").join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    root.join("Interface")
}

#[test]
fn expansion_level_events_and_bling_textures() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
f:RegisterEvent('UPDATE_EXPANSION_LEVEL')
local cd = CreateFrame('Cooldown', nil, f)
cd:SetBlingTexture(131010)
return f:IsEventRegistered('UPDATE_EXPANSION_LEVEL'), C_EventUtils.IsEventValid('UPDATE_EXPANSION_LEVEL')",
    );
    assert_eq!(r, ["true", "true"]);
}

#[test]
fn aura_containers_share_the_frame_methods() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
getmetatable(f).__index.MyAddonSize = function() return 'sized' end
local a = CreateFrame('AuraContainer')
return a:GetObjectType(), a:MyAddonSize(), getmetatable(a) == getmetatable(f)",
    );
    assert_eq!(r, ["Frame", "sized", "true"]);
}

#[test]
fn edit_boxes_and_cooldowns_come_with_their_regions() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local e = CreateFrame('EditBox')
local text, highlight = e:GetRegions()
local cd = CreateFrame('Cooldown')
return select('#', e:GetRegions()), text:GetObjectType(), highlight:GetObjectType(),
  cd:GetRegions() == cd:GetCountdownFontString()",
    );
    assert_eq!(r, ["2", "FontString", "Texture", "true"]);
}

#[test]
fn a_curve_color_stays_readable_with_secret_channels() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local curve = C_CurveUtil.CreateColorCurve()
curve:AddPoint(0, { r = 1, g = 0, b = 0, a = 1 })
curve:AddPoint(1, { r = 0, g = 1, b = 0, a = 1 })
local color = UnitHealthPercent('player', true, curve)
return issecretvalue(color), issecretvalue(color.r)",
    );
    assert_eq!(r, ["false", "true"]);
}

#[test]
fn xml_protection_is_explicit_and_only_scoped_secure_frames_are_forbidden() {
    let interface = tree(
        "scoped",
        &[
            (
                "AddOns/Blizzard_Shop/Blizzard_Shop.toc",
                "## UseSecureEnvironment: 1\nShop.xml\n",
            ),
            (
                "AddOns/Blizzard_Shop/Shop.xml",
                r#"<Ui>
  <Frame name="ShopFrame"><Frames><Frame parentKey="Inner"/></Frames></Frame>
  <ScopedModifier forbidden="true">
    <Frame name="StoreLikeFrame"><Frames><Frame parentKey="Inner"/></Frames></Frame>
  </ScopedModifier>
</Ui>"#,
            ),
            (
                "AddOns/Blizzard_Secure/Blizzard_Secure.toc",
                "## LoadFirst: 1\nSecure.xml\n",
            ),
            (
                "AddOns/Blizzard_Secure/Secure.xml",
                r#"<Ui><Frame name="ProtectedTemplate" protected="true" virtual="true"/></Ui>"#,
            ),
        ],
    );
    let mut vm = vm();
    let report = wowlua_engine::ui::load(&mut vm, &interface, None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let r = run(
        &mut vm,
        "local p = CreateFrame('Frame', nil, UIParent, 'ProtectedTemplate')
local _, explicit = p:IsProtected()
return ShopFrame:IsForbidden(), ShopFrame.Inner:IsForbidden(), StoreLikeFrame:IsForbidden(), explicit",
    );
    assert_eq!(r, ["false", "false", "true", "true"]);
}
