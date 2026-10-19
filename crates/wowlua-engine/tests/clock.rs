//! The clock, timers and OnUpdate, driven by the controls.

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
        other => format!("{other:?}"),
    };
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

#[test]
fn timers_run_in_order_on_the_virtual_clock() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local log = {}
C_Timer.After(2, function() log[#log + 1] = 'a@' .. GetTime() end)
C_Timer.After(1, function() log[#log + 1] = 'b@' .. GetTime() end)
local ticks = 0
local t = C_Timer.NewTicker(1.5, function(self) ticks = ticks + 1 if ticks == 2 then self:Cancel() end end)
local timer = C_Timer.NewTimer(10, function() log[#log + 1] = 'never' end)
timer:Cancel()
mock.advanceTime(5)
return table.concat(log, ' '), ticks, tostring(t:IsCancelled()), tostring(timer:IsCancelled()), GetTime()",
    );
    assert_eq!(r, ["b@1 a@2", "2", "true", "true", "5"]);
}

#[test]
fn on_update_runs_per_frame_for_visible_frames() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local n, total = 0, 0
local f = CreateFrame('Frame')
f:SetScript('OnUpdate', function(self, elapsed) n = n + 1 total = total + elapsed end)
mock.advanceFrames(3, 100)
f:Hide()
mock.nextFrame()
return n, string.format('%.1f', total)",
    );
    assert_eq!(r, ["3", "0.3"]);
}

/// An `OnUpdate` given during a frame: a widget after the one running gets
/// its turn in that frame, one before it and one made in it the next.
#[test]
fn on_update_given_during_a_frame_runs_in_creation_order() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local log = {}
local a, b, c = CreateFrame('Frame'), CreateFrame('Frame'), CreateFrame('Frame')
local function logger(name) return function() log[#log + 1] = name end end
b:SetScript('OnUpdate', function()
    log[#log + 1] = 'b'
    if not a:GetScript('OnUpdate') then
        a:SetScript('OnUpdate', logger('a'))
        c:SetScript('OnUpdate', logger('c'))
        CreateFrame('Frame'):SetScript('OnUpdate', logger('d'))
    end
end)
mock.nextFrame()
log[#log + 1] = '|'
mock.nextFrame()
return table.concat(log, ' ')",
    );
    assert_eq!(r, ["b c | a b c d"]);
}

#[test]
fn fire_event_checks_the_documentation() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local got
local f = CreateFrame('Frame')
f:RegisterEvent('ADDON_LOADED')
f:SetScript('OnEvent', function(_, e, name) got = e .. ':' .. name end)
mock.fireEvent('ADDON_LOADED', 'MyAddon', false)
return got, select(2, pcall(mock.fireEvent, 'NOT_AN_EVENT')), select(2, pcall(mock.fireEvent, 'ADDON_LOADED', 5))",
    );
    assert_eq!(r[0], "ADDON_LOADED:MyAddon");
    assert!(
        r[1].ends_with("\"NOT_AN_EVENT\" is not an event of this flavor"),
        "{}",
        r[1]
    );
    assert!(
        r[2].contains("ADDON_LOADED's `addOnName` is cstring, got number"),
        "{}",
        r[2]
    );
}

#[test]
fn timer_callbacks_must_be_functions() {
    let mut vm = vm();
    let r = run(&mut vm, "return select(2, pcall(C_Timer.After, 1, 'x'))");
    assert!(
        r[0].contains("Usage: C_Timer.After(seconds, callback)"),
        "{}",
        r[0]
    );
}
