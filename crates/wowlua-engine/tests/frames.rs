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
            "function", "false"
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
            "Frame:RegisterEvent(): Frame:RegisterEvent(): Attempt to register unknown event \"NO_SUCH_EVENT\"",
            "Frame:SetScript(): Cannot assign script handler for 'onclick' (script type not supported by this object)\nLua Taint: MyAddon"
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
p:Hide()        -- c-, p- (children first; c was visible)
c:Hide()        -- nothing: c not visible
p:Show()        -- p+
c:Show()        -- c+
p:Show()        -- nothing
return table.concat(log, ' '), tostring(c:IsVisible())",
    );
    assert_eq!(r, ["c- p- p+ c+", "true"]);
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
local noFont = select(2, pcall(fs.SetText, fs, 'hi'))
fs:SetFont('Fonts\\FRIZQT__.TTF', 12, '')
fs:SetText('hi')
return f:GetLeft(), f:GetTop(), f:GetFrameStrata(), fs:GetText(), fs:GetObjectType(), select(2, pcall(f.GetBoundsRect, f)), f:GetFrameLevel(), noFont",
    );
    assert_eq!(
        r,
        [
            "10",
            "70",
            "HIGH",
            "hi",
            "FontString",
            "10",
            "1",
            "FontString:SetText(): Font not set\nLua Taint: MyAddon"
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
    // A 14 high font keeps 13.999999046326 on camelot (`/stdtest engine`).
    assert_eq!(
        r,
        [
            "true",
            "true",
            "true",
            "true",
            "13.999999046326",
            "false",
            "off",
            "1001"
        ]
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
local f = CreateFrame('Frame', nil, UIParent, 'MyTemplate, BackdropTemplate')
local unknown = select(2, pcall(CreateFrame, 'Frame', nil, nil, 'NoSuchTemplate'))
local c = CreateFrame('mytype')
CreateFont('MyFont')
local fs = f:CreateFontString()
fs:SetFontObject('MyFont')
local ok = pcall(fs.SetFontObject, fs, 'NoSuchFont')
return tostring(f.fromTemplate), tostring(c.custom), c:GetObjectType(), #E.warnings(), tostring(ok),
    tostring(MyFont:IsObjectType('Font')), unknown",
    );
    // BackdropTemplate is Blizzard's (not loaded: a warning); a name nobody
    // defines fails as in the client.
    assert_eq!(
        r,
        [
            "true",
            "true",
            "Button",
            "1",
            "false",
            "true",
            "CreateFrame(): Couldn't find inherited node 'NoSuchTemplate'\nLua Taint: MyAddon"
        ]
    );
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
f:SetMovable(true)
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
    // Centered 100 x 50 on 1920 x 1080: left 910, bottom 515; moved by (10, 20).
    // The top stays at 585 as the frame grows to its maximum width.
    assert_eq!(r, ["920", "535", "120", "55", "TOPLEFT", "585", "4"]);
}

#[test]
fn animation_groups_play_and_finish_on_the_frame_clock() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "Seen = {}
local f = CreateFrame('Frame', nil, UIParent)
local g = f:CreateAnimationGroup()
local a = g:CreateAnimation('Alpha')
a:SetDuration(0.5)
local b = g:CreateAnimation('Translation')
b:SetDuration(0.25)
b:SetOrder(2)
g:SetScript('OnPlay', function() Seen[#Seen + 1] = 'play' end)
g:SetScript('OnFinished', function() Seen[#Seen + 1] = 'finished' end)
g:Play()
return tostring(g:IsPlaying()), g:GetDuration(), a:GetObjectType(), #{ g:GetAnimations() }, #{ f:GetAnimationGroups() }",
    );
    assert_eq!(r, ["true", "0.75", "Alpha", "2", "1"]);
    wowlua_engine::clock::advance_frames(&mut vm, 30, 1000.0 / 60.0);
    let mid = run(&mut vm, "return table.concat(Seen, ',')");
    wowlua_engine::clock::advance_frames(&mut vm, 30, 1000.0 / 60.0);
    let end = run(&mut vm, "return table.concat(Seen, ',')");
    assert_eq!(mid, ["play"]);
    assert_eq!(end, ["play,finished"]);
}

/// A hidden edit box's `SetText`: its `OnTextChanged` waits until the box
/// shows, then comes in that frame after `OnUpdate` (as `/stdtest engine
/// textchange` recorded on Forever).
#[test]
fn a_hidden_edit_box_changes_text_once_it_shows() {
    let mut vm = vm();
    wowlua_engine::controls::install(&mut vm, "mock");
    run(
        &mut vm,
        "local log = {}
_G.textLog = log
local parent = CreateFrame('Frame', nil, UIParent)
parent:Hide()
local box = CreateFrame('EditBox', nil, parent)
box:SetAutoFocus(false)
box:SetScript('OnTextChanged', function(self, userInput)
    log[#log + 1] = 'changed(' .. tostring(userInput) .. ', ' .. self:GetText() .. ', visible ' .. tostring(self:IsVisible()) .. ')'
end)
box:SetText('hidden')
log[#log + 1] = 'set while hidden'
local frames = 0
local ticker = CreateFrame('Frame')
ticker:SetScript('OnUpdate', function()
    frames = frames + 1
    log[#log + 1] = 'update' .. frames
    if frames == 3 then
        parent:Show()
        log[#log + 1] = 'parent shown'
    elseif frames == 6 then
        parent:Hide()
        box:SetText('')
        log[#log + 1] = 'hidden, set empty'
        parent:Show()
        log[#log + 1] = 'shown again in the same frame'
    elseif frames == 9 then
        ticker:SetScript('OnUpdate', nil)
    end
end)
mock.advanceFrames(10)",
    );
    let r = run(&mut vm, "return table.concat(textLog, ' | ')");
    assert_eq!(
        r,
        [
            "set while hidden | update1 | update2 | update3 | parent shown | changed(false, hidden, visible true) | update4 | update5 | update6 | hidden, set empty | shown again in the same frame | changed(false, , visible true) | update7 | update8 | update9"
        ]
    );
}

/// `CreateFrame`'s fifth argument is the frame's ID (`GetID`), set before
/// the templates run (Blizzard's action buttons read it in `OnLoad`).
#[test]
fn create_frame_sets_the_id() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('CheckButton', nil, UIParent, nil, 7)
local g = CreateFrame('Frame')
return f:GetID(), g:GetID()",
    );
    assert_eq!(r, ["7", "0"]);
}

/// `SetDrawLayer(layer)` without a sublevel sets sublevel 0 (documented),
/// which `GetDrawLayer` answers (Blizzard's menus raise a pooled texture a
/// sublevel above it); `SetTextToFit` sets the text.
#[test]
fn a_draw_layer_set_alone_has_sublevel_zero_and_text_fits() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame', nil, UIParent)
local t = f:CreateTexture(nil, 'BACKGROUND', nil, 3)
t:SetDrawLayer('ARTWORK')
local layer, sublevel = t:GetDrawLayer()
local s = f:CreateFontString()
s:SetFont('Fonts\\\\FRIZQT__.TTF', 12, '')
s:SetTextToFit('Learning')
return layer, sublevel, s:GetText()",
    );
    assert_eq!(r, ["ARTWORK", "0", "Learning"]);
}
