//! Frames, scripts and events in the engine.

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    vm
}

fn text(vm: &Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => n.to_string(),
        other => format!("{other:?}"),
    }
}

/// Runs `src` as addon code; its results as text.
fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

#[test]
fn frames_are_tables_with_their_class_methods() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame', 'MyFrame', UIParent)
local b = CreateFrame('button', nil, f)
return type(f), type(f[0]), f == MyFrame and 'named' or 'not', f:GetName(), f:GetObjectType(), b:GetObjectType(),
    tostring(b:IsObjectType('Frame')), tostring(b:GetParent() == f), tostring(f:IsShown()), type(f.Show), tostring(f.Show == b.Show)",
    );
    assert_eq!(
        r,
        [
            "table", "userdata", "named", "MyFrame", "Frame", "Button", "true", "true", "true",
            "function", "true"
        ]
    );
}

#[test]
fn events_reach_registered_frames_in_order() {
    let mut vm = vm();
    run(
        &mut vm,
        "Seen = {}
for i = 1, 3 do
    local f = CreateFrame('Frame')
    f:SetScript('OnEvent', function(self, event, arg) Seen[#Seen + 1] = i .. event .. tostring(arg) end)
    if i ~= 2 then f:RegisterEvent('PLAYER_LOGIN') end
    if i == 2 then f:RegisterUnitEvent('UNIT_HEALTH', 'target') end
end",
    );
    let unit = vm.str("player");
    wowlua_engine::widgets::fire_event(&mut vm, "PLAYER_LOGIN", &[]);
    wowlua_engine::widgets::fire_event(&mut vm, "UNIT_HEALTH", &[unit]);
    let target = vm.str("target");
    wowlua_engine::widgets::fire_event(&mut vm, "UNIT_HEALTH", &[target]);
    let r = run(&mut vm, "return table.concat(Seen, ',')");
    assert_eq!(r, ["1PLAYER_LOGINnil,3PLAYER_LOGINnil,2UNIT_HEALTHtarget"]);
}

#[test]
fn a_failing_handler_does_not_stop_the_others() {
    let mut vm = vm();
    run(
        &mut vm,
        "Errors, Ran = {}, {}
seterrorhandler(function(e) Errors[#Errors + 1] = e end)
for i = 1, 2 do
    local f = CreateFrame('Frame')
    f:RegisterEvent('PLAYER_LOGIN')
    f:SetScript('OnEvent', function() Ran[#Ran + 1] = i if i == 1 then error('boom') end end)
end",
    );
    wowlua_engine::widgets::fire_event(&mut vm, "PLAYER_LOGIN", &[]);
    let r = run(&mut vm, "return #Ran, #Errors, Errors[1]");
    assert_eq!(r[0], "2");
    assert_eq!(r[1], "1");
    assert!(r[2].ends_with("boom"), "{}", r[2]);
}

#[test]
fn unknown_events_and_scripts_fail() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
return select(2, pcall(f.RegisterEvent, f, 'NO_SUCH_EVENT')), select(2, pcall(f.SetScript, f, 'OnClick', print))",
    );
    assert_eq!(
        r,
        [
            "Frame:RegisterEvent(): Attempt to register unknown event \"NO_SUCH_EVENT\"",
            "Frame:SetScript(): Frame doesn't have a \"OnClick\" script"
        ]
    );
}

#[test]
fn show_and_hide_run_scripts_when_visibility_changes() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local log = {}
local p = CreateFrame('Frame')
local c = CreateFrame('Frame', nil, p)
for name, f in pairs({ p = p, c = c }) do
    f:SetScript('OnShow', function() log[#log + 1] = name .. '+' end)
    f:SetScript('OnHide', function() log[#log + 1] = name .. '-' end)
end
p:Hide()        -- p-, c- (c was visible)
c:Hide()        -- nothing: c not visible
p:Show()        -- p+
c:Show()        -- c+
p:Show()        -- nothing
return table.concat(log, ' '), tostring(c:IsVisible())",
    );
    assert_eq!(r, ["p- c- p+ c+", "true"]);
}

#[test]
fn layout_and_stored_setters() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame', nil, UIParent)
f:SetSize(100, 50)
f:SetPoint('BOTTOMLEFT', 10, 20)
f:SetFrameStrata('HIGH')
local fs = f:CreateFontString(nil, 'OVERLAY')
fs:SetText('hi')
return f:GetLeft(), f:GetTop(), f:GetFrameStrata(), fs:GetText(), fs:GetObjectType(), select(2, pcall(f.GetHitRectInsets, f)), f:GetFrameLevel()",
    );
    assert_eq!(
        r,
        [
            "10",
            "70",
            "HIGH",
            "hi",
            "FontString",
            "Frame:GetHitRectInsets is not mocked",
            "1"
        ]
    );
}

#[test]
fn stored_methods_is_enable_and_no_ops() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
f:SetMovable(true)
f:EnableMouse(true)
f:SetToplevel(true)
local fs = f:CreateFontString()
local ok = fs:SetFont('Fonts\\\\FRIZQT__.TTF', 14, '')
f:RegisterForDrag('LeftButton')
local b = CreateFrame('Button')
Seen = {}
b:SetScript('OnDisable', function() Seen[#Seen + 1] = 'off' end)
b:Disable()
b:Disable()
local t = f:CreateTexture()
t:SetVertexColor(1, 0, 0)
local r, g, bb, a = t:GetVertexColor()
return tostring(f:IsMovable()), tostring(f:IsMouseEnabled()), tostring(f:IsToplevel()), tostring(ok),
    select(2, fs:GetFont()), tostring(b:IsEnabled()), table.concat(Seen, ','), r .. g .. bb .. a",
    );
    assert_eq!(
        r,
        ["true", "true", "true", "true", "14", "false", "off", "1001"]
    );
}

#[test]
fn unit_events_report_their_units() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
f:RegisterUnitEvent('UNIT_HEALTH', 'player', 'target')
f:RegisterEvent('PLAYER_LOGIN')
local a, u1, u2 = f:IsEventRegistered('UNIT_HEALTH')
local b, none = f:IsEventRegistered('PLAYER_LOGIN')
return tostring(a), u1, u2, tostring(b), tostring(none), tostring(f:IsEventRegistered('BAG_UPDATE')),
    tostring(GetFramesRegisteredForEvent('PLAYER_LOGIN') == f), tostring(C_EventUtils.IsEventValid('PLAYER_LOGIN')),
    tostring(C_EventUtils.IsEventValid('NOPE'))",
    );
    assert_eq!(
        r,
        [
            "true", "player", "target", "true", "nil", "false", "true", "true", "false"
        ]
    );
}

#[test]
fn templates_frame_types_and_fonts() {
    let mut vm = vm();
    wowlua_engine::controls::install(&mut vm, "E");
    let r = run(
        &mut vm,
        "E.defineTemplate('MyTemplate', function(f) f.fromTemplate = true end)
E.defineFrameType('MyType', 'Button', function(f) f.custom = true end)
local f = CreateFrame('Frame', nil, UIParent, 'MyTemplate, OtherTemplate')
local c = CreateFrame('mytype')
CreateFont('MyFont')
local fs = f:CreateFontString()
fs:SetFontObject('MyFont')
local ok = pcall(fs.SetFontObject, fs, 'NoSuchFont')
return tostring(f.fromTemplate), tostring(c.custom), c:GetObjectType(), #E.warnings(), tostring(ok),
    tostring(MyFont:IsObjectType('Font'))",
    );
    assert_eq!(r, ["true", "true", "Button", "1", "false", "true"]);
}

#[test]
fn drags_and_resize_grips() {
    let mut vm = vm();
    wowlua_engine::controls::install(&mut vm, "E");
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame', nil, UIParent)
f:SetSize(100, 50)
f:SetPoint('CENTER')
f:SetResizeBounds(80, 40, 120, 60)
f:SetScript('OnDragStart', function(self) self:StartMoving() end)
f:SetScript('OnDragStop', function(self) self:StopMovingOrSizing() end)
E.drag(f, 10, 20)
local left, bottom = f:GetLeft(), f:GetBottom()
local grip = CreateFrame('Button', nil, f)
grip:SetScript('OnMouseDown', function() f:StartSizing('BOTTOMRIGHT') end)
grip:SetScript('OnMouseUp', function() f:StopMovingOrSizing() end)
E.sizeBy(grip, 50, 5)
local p = f:GetPoint(1)
return left, bottom, f:GetWidth(), f:GetHeight(), p, f:GetTop(), #E.frames()",
    );
    // Centered 100 x 50 on 1024 x 768: left 462, bottom 359; moved by (10, 20).
    // The top stays at 429 + 0 as the frame grows to its maximum width.
    assert_eq!(r, ["472", "379", "120", "55", "TOPLEFT", "429", "4"]);
}
