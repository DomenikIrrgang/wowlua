//! The encounter timeline's script events (`timeline.rs`, decision T255).

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

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
fn an_addons_timer_rides_the_timeline_until_it_runs_out() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'ENCOUNTER_TIMELINE_EVENT_ADDED', 'ENCOUNTER_TIMELINE_EVENT_STATE_CHANGED', 'ENCOUNTER_TIMELINE_EVENT_REMOVED' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, a) seen[#seen + 1] = e:match('EVENT_(.*)') .. ' ' .. (type(a) == 'table' and a.spellName or tostring(a)) end)
local ET = C_EncounterTimeline
local pull = ET.AddScriptEvent({ spellID = 100, iconFileID = 132337, duration = 10, overrideName = 'Pull' })
local other = ET.AddScriptEvent({ spellID = 100, iconFileID = 1, duration = 60 })
local info = ET.GetEventInfo(pull)
local list = ET.GetEventList()
mock.advanceTime(4)
local left = ET.GetEventTimeRemaining(pull)
ET.PauseScriptEvent(pull)
mock.advanceTime(20)
local paused = ET.GetEventState(pull) .. ' ' .. ET.GetEventTimeRemaining(pull) .. ' ' .. tostring(ET.HasPausedEvents())
ET.ResumeScriptEvent(pull)
mock.advanceTime(6.1)
-- (one gone: the calls that need an event answer nothing for it)
local afterRun = #ET.GetEventList() .. ' ' .. select('#', ET.GetEventState(pull))
ET.CancelScriptEvent(other)
return pull, info.spellName .. ' ' .. info.source .. ' ' .. info.duration, #list, left, paused, afterRun,
    tostring(ET.HasAnyEvents()), table.concat(seen, ', ')",
    );
    assert_eq!(
        r,
        [
            "41",
            "Pull 1 10",
            "2",
            "6",
            "1 6 true",
            "1 0",
            "false",
            "ADDED Pull, ADDED Charge, STATE_CHANGED 41, STATE_CHANGED 41, STATE_CHANGED 41, REMOVED 41, STATE_CHANGED 42, REMOVED 42"
        ]
    );
}
