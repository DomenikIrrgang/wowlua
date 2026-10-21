//! Widget methods `methods3.rs` answers from the setters' stored values,
//! the engine's models and the documented or XML defaults.

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

#[test]
fn cooldown_texture_and_line_settings_read_back() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
local cd = CreateFrame('Cooldown', nil, f)
local swipe = cd:GetDrawSwipe()
cd:SetDrawSwipe(false)
local t = f:CreateTexture()
local blend = t:GetBlendMode()
t:SetBlendMode('ADD')
t:SetVertexOffset(2, 3, 4)
local x, y = t:GetVertexOffset(2)
local x1 = t:GetVertexOffset(1)
local l = f:CreateLine()
return swipe, cd:GetDrawSwipe(), blend, t:GetBlendMode(), x, y, x1, l:GetThickness()",
    );
    assert_eq!(r, ["true", "false", "BLEND", "ADD", "3", "4", "0", "4"]);
}

#[test]
fn model_scene_cameras_and_html_text_types() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local s = CreateFrame('ModelScene')
s:SetCameraFarClip(100)
local fx, fy, fz = s:GetCameraForward()
local h = CreateFrame('SimpleHTML')
h:SetTextColor('H1', 1, 0, 0)
local r, g, b, a = h:GetTextColor('H1')
local pr = h:GetTextColor('P')
return s:GetCameraFarClip(), fx, fy, fz == 0, r, g, a, pr",
    );
    assert_eq!(r, ["100", "1", "0", "true", "1", "0", "1", "1"]);
}

#[test]
fn animations_know_their_region_and_settings() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
local g = f:CreateAnimationGroup()
local rot = g:CreateAnimation('Rotation')
rot:SetDegrees(90)
local a = g:CreateAnimation('Alpha')
return rot:GetRegionParent() == f, math.abs(rot:GetRadians() - math.pi / 2) < 1e-6,
  a:GetFromAlpha(), a:GetToAlpha(), g:GetLoopState()",
    );
    assert_eq!(r, ["true", "true", "0", "1", "NONE"]);
}

#[test]
fn frame_levels_and_edit_box_display() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local p = CreateFrame('Frame')
p:SetFrameLevel(5)
local c = CreateFrame('Frame', nil, p)
c:SetFrameLevel(9)
local e = CreateFrame('EditBox')
e:SetPassword(true)
e:SetText('abc')
return p:GetHighestFrameLevel(), e:GetDisplayText(), e:GetBlinkSpeed()",
    );
    assert_eq!(r, ["9", "***", "0.5"]);
}

/// A model's or actor's `GetItemTransmogInfoList()` has an entry for every
/// equipment slot (Blizzard's dressing room reads the main hand's
/// unchecked); without Blizzard's `ItemUtil`, plain tables of the fields;
/// with it, its objects.
#[test]
fn transmog_info_lists_have_every_slot() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local s = CreateFrame('ModelScene')
local a = s:CreateActor()
local list = a:GetItemTransmogInfoList()
local n = 0
for slot = 1, 19 do if list[slot] then n = n + 1 end end
local plain = list[16].appearanceID
ItemUtil = { CreateItemTransmogInfo = function(id) return { appearanceID = id, made = true } end }
local made = CreateFrame('DressUpModel'):GetItemTransmogInfoList()[16].made
return n, plain, tostring(made), tostring(list[20])",
    );
    assert_eq!(r, ["19", "0", "true", "nil"]);
}
