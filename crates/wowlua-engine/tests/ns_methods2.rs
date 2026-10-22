//! Widget methods `methods2.rs` answers from the engine's models and the
//! setters' stored values.

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
fn parent_keys_max_lines_and_font_families() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
local fs = f:CreateFontString()
f.Label = fs
local t = f:CreateTexture()
t:SetParentKey('Icon')
local font = CreateFont('MyAddonFont')
local before = fs:GetMaxLines()
fs:SetMaxLines(3)
return fs:GetParentKey(), t:GetParentKey(), before, fs:GetMaxLines(),
  font:GetFontObjectForAlphabet('roman') == font",
    );
    assert_eq!(r, ["Label", "Icon", "0", "3", "true"]);
}

#[test]
fn texture_coordinates_and_text_colors() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
local t = f:CreateTexture()
t:SetTexCoord(0.1, 0.9, 0.2, 0.8)
local ulx, uly, llx, lly, urx, ury, lrx, lry = t:GetTexCoord()
local fs = f:CreateFontString()
fs:SetTextColor(1, 0, 0)
local _, _, _, a = fs:GetTextColor()
return ulx, uly, lly, urx, lry, a",
    );
    assert_eq!(r, ["0.1", "0.2", "0.8", "0.9", "0.8", "1"]);
}

#[test]
fn strata_alpha_and_letters_follow_the_models() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local p = CreateFrame('Frame')
p:SetFrameStrata('HIGH')
p:SetAlpha(0.5)
local c = CreateFrame('Frame', nil, p)
local e = CreateFrame('EditBox', nil, p)
e:SetText('héllo')
return c:GetFrameStrata(), c:GetEffectiveAlpha() == p:GetAlpha(), e:GetNumLetters(),
  e:HasText(), e:GetUTF8CursorPosition()",
    );
    assert_eq!(r, ["HIGH", "true", "5", "true", "5"]);
}

/// An alpha set from a secret keeps reading back secret after a plain one
/// is set (decision T251), its new value applied.
#[test]
fn a_secret_alpha_stays_secret() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
local plain = CreateFrame('Frame')
plain:SetAlpha(0.5)
-- (your health's share, secret: 1)
f:SetAlpha(UnitHealthPercent('player'))
local before = issecretvalue(f:GetAlpha())
f:SetAlpha(1)
local g = CreateFrame('Frame')
g:SetAlphaFromBoolean(UnitHealthPercent('player') ~= nil, 1, 0)
g:SetAlpha(UnitHealthPercent('player'))
g:SetAlpha(0.25)
return tostring(issecretvalue(plain:GetAlpha())), tostring(before), tostring(issecretvalue(f:GetAlpha())),
  tostring(f:HasSecretAspect(Enum.SecretAspect.Alpha)), tostring(issecretvalue(g:GetAlpha()))",
    );
    assert_eq!(r, ["false", "true", "true", "true", "true"]);
}
