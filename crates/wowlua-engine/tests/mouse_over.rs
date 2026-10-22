//! `IsMouseOver` measures the cursor against the region (`input.rs`,
//! decision T257).

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
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
fn the_mouse_over_a_frame_is_measured_against_its_rectangle_and_offsets() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local a = CreateFrame('Frame', nil, UIParent)
a:SetSize(100, 40)
a:SetPoint('BOTTOMLEFT', 200, 300)
local child = CreateFrame('Frame', nil, a)
child:SetSize(20, 20)
child:SetPoint('LEFT', a, 'RIGHT', 30, 0)
local b = CreateFrame('Frame', nil, UIParent)
b:SetSize(50, 50)
b:SetPoint('BOTTOMLEFT', 600, 300)
local out = {}
local function over() return tostring(a:IsMouseOver()) .. ' ' .. tostring(b:IsMouseOver()) end
out[#out + 1] = over()                       -- the mouse off every frame at first
mock.hover(a)
local x, y = GetCursorPosition()
local s = UIParent:GetEffectiveScale()
out[#out + 1] = over() .. ' ' .. x / s .. ' ' .. y / s
mock.hover(child)                            -- a child outside its parent
out[#out + 1] = over() .. ' ' .. tostring(child:IsMouseOver())
-- just right of the frame: over it only with the right edge moved out
mock.setCursorPosition(305 * s, 320 * s)
out[#out + 1] = tostring(a:IsMouseOver()) .. ' ' .. tostring(a:IsMouseOver(0, 0, 0, 10)) .. ' ' .. tostring(a:IsMouseOver(0, 0, 0, 2))
-- just below it: the bottom edge moved down
mock.setCursorPosition(250 * s, 290 * s)
out[#out + 1] = tostring(a:IsMouseOver()) .. ' ' .. tostring(a:IsMouseOver(0, -12, 0, 0))
mock.hover(b)
out[#out + 1] = over()
b:Hide()                                     -- hidden: still its rectangle (unverified)
out[#out + 1] = tostring(b:IsMouseOver())
mock.hover(nil)
out[#out + 1] = over()
local loose = CreateFrame('Frame', nil, UIParent)
loose:SetSize(10, 10)
mock.hover(loose)                            -- no points: no rectangle
out[#out + 1] = tostring(loose:IsMouseOver())
return table.concat(out, ' | ')",
    );
    assert_eq!(
        r,
        [
            "false false | true false 250 320 | false false true | false true false | false true | false true | true | false false | false"
        ]
    );
}

#[test]
fn a_frames_on_leave_sees_the_mouse_gone() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local a = CreateFrame('Button', nil, UIParent)
a:SetSize(100, 40)
a:SetPoint('CENTER')
local seen = {}
a:SetScript('OnEnter', function(self) seen[#seen + 1] = 'enter ' .. tostring(self:IsMouseOver()) end)
a:SetScript('OnLeave', function(self) seen[#seen + 1] = 'leave ' .. tostring(self:IsMouseOver()) end)
mock.click(a)
mock.hover(nil)
return table.concat(seen, ', ')",
    );
    assert_eq!(r, ["enter true, leave false"]);
}

#[test]
fn clear_point_takes_one_point_away() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local owner = CreateFrame('Frame', nil, UIParent)
owner:SetSize(100, 20)
owner:SetPoint('BOTTOMLEFT', 100, 100)
local menu = CreateFrame('Frame', nil, UIParent)
menu:SetSize(80, 60)
menu:SetPoint('TOPLEFT', owner, 'BOTTOMLEFT')
-- flipped to open above, as Blizzard's menus do near the screen's bottom
menu:ClearPoint('TOPLEFT')
menu:SetPoint('BOTTOMLEFT', owner, 'TOPLEFT')
local point, _, relativePoint = menu:GetPoint(1)
menu:ClearPoint('CENTER')
return menu:GetNumPoints(), point, relativePoint, menu:GetHeight(), menu:GetBottom()",
    );
    assert_eq!(r, ["1", "BOTTOMLEFT", "TOPLEFT", "60", "120"]);
}

#[test]
fn a_drag_over_frames_runs_them_as_it_moves() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame', nil, UIParent)
f:SetSize(40, 40)
f:SetPoint('BOTTOMLEFT', 100, 100)
f:SetMovable(true)
f:EnableMouse(true)
f:RegisterForDrag('LeftButton')
f:SetScript('OnDragStart', function(self) self:StartMoving() end)
f:SetScript('OnDragStop', function(self) self:StopMovingOrSizing() end)
local seen = {}
f:SetScript('OnUpdate', function(self)
    local x = GetCursorPosition() / UIParent:GetEffectiveScale()
    seen[#seen + 1] = math.floor(self:GetLeft() + 0.5) .. '@' .. math.floor(x + 0.5)
end)
local s = UIParent:GetEffectiveScale()
mock.drag(f, 30 * s, 0, 3)
local stepped = table.concat(seen, ' ')
seen = {}
mock.drag(f, 10 * s, 0)
return stepped, #seen, math.floor(f:GetLeft() + 0.5)",
    );
    assert_eq!(r, ["110@130 120@140 130@150", "0", "140"]);
}

#[test]
fn a_region_upside_down_has_no_size_at_its_left_and_bottom_points() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame', nil, UIParent)
f:SetPoint('TOP', UIParent, 'CENTER', 0, -50)
f:SetPoint('BOTTOM', UIParent, 'CENTER', 0, 50)
f:SetPoint('LEFT', UIParent, 'CENTER', 50, 0)
f:SetPoint('RIGHT', UIParent, 'CENTER', -50, 0)
local cx, cy = UIParent:GetCenter()
local l, b, w, h = f:GetRect()
return f:GetWidth(), f:GetHeight(), l - cx, b - cy, w, h",
    );
    assert_eq!(r, ["0", "0", "50", "50", "0", "0"]);
}

#[test]
fn a_child_of_a_frame_never_anchored_has_no_rectangle() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local b = CreateFrame('Button', nil, UIParent)
b:SetSize(200, 30)
local t = b:CreateTexture()
t:SetAllPoints()
return tostring(t:GetWidth()), tostring(t:GetLeft()), select('#', t:GetRect())",
    );
    assert_eq!(r, ["0", "nil", "0"]);
}
