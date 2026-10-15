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
    assert_eq!(r[0], "Num(2.0)");
    assert_eq!(r[1], "Num(1.0)");
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
return f:GetLeft(), f:GetTop(), f:GetFrameStrata(), fs:GetText(), fs:GetObjectType(), select(2, pcall(f.GetFrameLevel, f))",
    );
    assert_eq!(
        r,
        [
            "Num(10.0)",
            "Num(70.0)",
            "HIGH",
            "hi",
            "FontString",
            "Frame:GetFrameLevel is not mocked"
        ]
    );
}
