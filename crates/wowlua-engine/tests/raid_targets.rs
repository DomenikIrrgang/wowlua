//! Raid target markers (decision T631): set, moved and taken off a round
//! trip later with `RAID_TARGET_UPDATE`; on camelot forbidden to an
//! addon in combat (found in game), not to Blizzard's untainted code.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use wowlua_vm::{Value, Vm};

fn vm(flavor: &str) -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, flavor).expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

fn text(vm: &Vm, v: Value) -> String {
    match v {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        Value::Bool(b) => format!("{b}"),
        Value::Nil => "nil".into(),
        other => format!("{other:?}"),
    }
}

/// Runs `src` as MyAddon's code, or as Blizzard's (secure) without an
/// addon.
fn run_as(vm: &mut Vm, src: &str, addon: Option<&str>) -> Vec<String> {
    let r = match addon {
        Some(a) => {
            let f = vm
                .load_as(src.as_bytes(), &format!("@{a}/main.lua"), a)
                .expect("compiles");
            vm.call(f, &[])
        }
        // Blizzard's code loads and starts untainted, as the client runs
        // each event.
        None => vm.with_execution_taint(wowlua_vm::Taint::SECURE, |vm| {
            let f = vm
                .load(
                    src.as_bytes(),
                    "@Interface/AddOns/Blizzard_UnitFrame/Test.lua",
                )
                .expect("compiles");
            vm.call(f, &[])
        }),
    };
    match r {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    run_as(vm, src, Some("MyAddon"))
}

/// The setup: two mobs, the first targeted, and a log of the events.
const SETUP: &str = "Seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('RAID_TARGET_UPDATE')
f:RegisterEvent('ADDON_ACTION_FORBIDDEN')
f:SetScript('OnEvent', function(_, e, addon, action)
    Seen[#Seen + 1] = action and (e .. ' ' .. addon .. ' ' .. action) or e
end)
Mob = mock.spawnUnit({ name = 'Mob' })
Other = mock.spawnUnit({ name = 'Other' })
mock.player:target(Mob)";

/// The marker a unit has, through the secret camelot answers.
const MARKER: &str = "local function marker(unit)
    local m = GetRaidTargetIndex(unit)
    if m == nil then return 'none' end
    for i = 1, 8 do
        if mock.sameSecret(m, i) then return tostring(i) end
    end
    return '?'
end";

#[test]
fn an_addon_marks_out_of_combat() {
    let mut vm = vm("camelot");
    run(&mut vm, SETUP);
    let r = run(
        &mut vm,
        &format!(
            "{MARKER}
local before = select('#', GetRaidTargetIndex('target'))
SetRaidTarget('target', 8)
local asked = marker('target')
mock.advanceTime(1)
local set = marker('target')
local events = table.concat(Seen, ',')
-- The same marker on another unit moves it.
local other = Other
mock.player:target(other)
SetRaidTarget('target', 8)
mock.advanceTime(1)
local moved = marker('target')
mock.player:target(Mob)
local left = marker('target')
-- 0 takes it off.
mock.player:target(other)
SetRaidTarget('target', 0)
mock.advanceTime(1)
return before, asked, set, events, moved, left, marker('target'), #Seen, #mock.refusals()"
        ),
    );
    assert_eq!(
        r,
        [
            "1",
            "none",
            "8",
            "RAID_TARGET_UPDATE",
            "8",
            "none",
            "none",
            "3",
            "0"
        ]
    );
}

#[test]
fn an_addon_is_forbidden_to_mark_in_combat_on_camelot() {
    let mut vm = vm("camelot");
    run(&mut vm, SETUP);
    let r = run(
        &mut vm,
        &format!(
            "{MARKER}
mock.player:enterCombat()
SetRaidTarget('target', 8)
local direct = table.concat(Seen, ',')
local ok = pcall(SetRaidTarget, 'target', 8)
mock.advanceTime(1)
local refused = mock.refusals()
return direct, tostring(ok), marker('target'), #Seen, refused[1].action, refused[2].action"
        ),
    );
    assert_eq!(
        r,
        [
            "ADDON_ACTION_FORBIDDEN MyAddon SetRaidTarget()",
            "true",
            "none",
            "2",
            "SetRaidTarget()",
            "UNKNOWN()"
        ]
    );
}

/// Blizzard's unit menu marks in combat when nothing tainted it: its
/// `SetRaidTargetIcon` (TargetFrame.lua), untainted, sets and toggles.
#[test]
fn blizzards_untainted_call_marks_in_combat() {
    let mut vm = vm("camelot");
    run(&mut vm, SETUP);
    run(&mut vm, "mock.player:enterCombat()");
    let secure = "function TestSetRaidTargetIcon(unit, index)
    if GetRaidTargetIndex(unit) and GetRaidTargetIndex(unit) == index then
        SetRaidTarget(unit, 0)
    else
        SetRaidTarget(unit, index)
    end
end
TestSetRaidTargetIcon('target', 8)";
    run_as(&mut vm, secure, None);
    let r = run(
        &mut vm,
        &format!(
            "{MARKER}
mock.advanceTime(1)
return marker('target'), table.concat(Seen, ','), #mock.refusals(), tostring(InCombatLockdown())"
        ),
    );
    assert_eq!(r, ["8", "RAID_TARGET_UPDATE", "0", "true"]);
}

/// Other flavors keep it unrestricted (unverified there: the probe).
#[test]
fn other_flavors_mark_in_combat() {
    let mut vm = vm("vanilla");
    run(&mut vm, SETUP);
    let r = run(
        &mut vm,
        "mock.player:enterCombat()
SetRaidTarget('target', 3)
mock.advanceTime(1)
return GetRaidTargetIndex('target'), table.concat(Seen, ','), #mock.refusals()",
    );
    assert_eq!(r, ["3", "RAID_TARGET_UPDATE", "0"]);
}
