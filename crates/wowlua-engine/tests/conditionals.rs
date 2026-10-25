//! Macro conditionals: `SecureCmdOptionParse` over the engine's state, and
//! Blizzard's state driver following it.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use std::path::{Path, PathBuf};

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
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

fn run(vm: &mut Vm, src: &str) -> Vec<String> {
    let f = vm
        .load_as(src.as_bytes(), "@MyAddon/main.lua", "MyAddon")
        .expect("compiles");
    match vm.call(f, &[]) {
        Ok(r) => r.into_iter().map(|v| text(vm, v)).collect(),
        Err(e) => panic!("{}", text(vm, e.value)),
    }
}

/// Each line's answer as `value|target`.
const PARSE: &str = "local out = {}
local function p(s)
    local v, t = SecureCmdOptionParse(s)
    out[#out + 1] = tostring(v) .. '|' .. tostring(t)
end
for _, s in ipairs(LINES) do p(s) end
return unpack(out)";

fn parse(vm: &mut Vm, lines: &[&str]) -> Vec<String> {
    let list: Vec<String> = lines.iter().map(|l| format!("{l:?}")).collect();
    let src = format!("local LINES = {{ {} }}\n{PARSE}", list.join(", "));
    run(vm, &src)
}

#[test]
fn clauses_values_and_targets() {
    let mut vm = vm();
    let r = parse(
        &mut vm,
        &[
            "",
            "  plain value  ",
            "[combat] a; b",
            "[nocombat] a; b",
            "[combat]",
            "[nocombat]",
            "[] always",
            "[combat][nocombat] either",
            "[@player] me",
            "[target=focus] f",
            "[@focus,exists] f; nobody",
            "[exists] t; none",
            "[@player,exists,nodead] alive",
            "[frobnicate] x; [nofrobnicate] y; z",
            "[combat] a;",
            "[bar:2] 2; [bar:1/3] 13; 0",
            "[nostance] none; any",
            "[group] g; [nogroup:party/raid] solo",
            "[nopet] nopet",
            "[button:1] one; other",
            "[nocursor,noflying,noswimming,nomounted,nochanneling] ground",
        ],
    );
    assert_eq!(
        r,
        [
            "|nil",
            "plain value|nil",
            "b|nil",
            "a|nil",
            "nil|nil",
            "|nil",
            "always|nil",
            "either|nil",
            "me|player",
            "f|focus",
            "nobody|nil",
            "none|nil",
            "alive|player",
            // An unknown condition holds (the client's `[frobnicate] a; b`
            // is "a").
            "x|nil",
            "|nil",
            "13|nil",
            "none|nil",
            "solo|nil",
            "nopet|nil",
            "one|nil",
            "ground|nil",
        ]
    );
}

#[test]
fn conditions_follow_the_world() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local out = {}
local function p(s) out[#out + 1] = tostring((SecureCmdOptionParse(s))) end
local hogger = mock.spawnUnit({ name = 'Hogger', level = 11, reaction = 'hostile' })
p('[harm] harm; [help] help; none')
mock.player:target(hogger)
p('[harm,nodead] harm; [help] help; none')
p('[@player,help] self; none')
mock.player:enterCombat()
p('[combat] fight; calm')
mock.setModifiers({ shift = true })
p('[mod:shift] s; n')
p('[mod:ctrl/alt] ca; n')
p('[mod] any; n')
p('[nomod] none; held')
mock.setModifiers({})
p('[nomod] none; held')
local seen = 0
local f = CreateFrame('Frame')
f:RegisterEvent('ACTIONBAR_PAGE_CHANGED')
f:SetScript('OnEvent', function() seen = seen + 1 end)
p('[bar:2] 2; 1')
C_ActionBar.SetActionBarPage(2)
p('[bar:2] 2; 1')
C_ActionBar.SetActionBarPage(2)
p(C_ActionBar.GetActionBarPage() .. ' ' .. seen)
return unpack(out)",
    );
    assert_eq!(
        r,
        [
            "none", "harm", "self", "fight", "s", "n", "any", "held", "none", "1", "2", "2 1",
        ]
    );
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A VM with camelot's UI code, or nothing when it is not on disk.
fn client() -> Option<Vm> {
    let interface = repo().join("target/types-src/camelot/Interface");
    if !interface.exists() {
        return None;
    }
    let xtask = std::fs::read_to_string(repo().join("xtask/src/types.rs")).unwrap_or_default();
    let commit = xtask
        .split("\"camelot\",")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(3))
        .unwrap_or("");
    let strings = repo().join(format!("target/types-src/strings/{commit}.lua"));
    let mut vm = vm();
    wowlua_engine::ui::load(
        &mut vm,
        &interface,
        strings.exists().then_some(Path::new(&strings)),
    );
    Some(vm)
}

/// `RegisterStateDriver` sets the state attribute at once, and again when
/// the state changes: on the manager's next frame after an event it
/// listens for (`ACTIONBAR_PAGE_CHANGED`, `MODIFIER_STATE_CHANGED`,
/// `PLAYER_REGEN_DISABLED`), else on its throttled update (0.2 seconds).
/// The visibility driver shows a protected frame in combat (the manager's
/// code is secure).
#[test]
fn the_state_driver_follows_the_conditions() {
    let Some(mut vm) = client() else {
        eprintln!("skipped: the UI code is missing");
        return;
    };
    let r = run(
        &mut vm,
        "local out = {}
local f = CreateFrame('Frame', 'PagerFrame', UIParent, 'SecureHandlerStateTemplate')
local shown = CreateFrame('Frame', 'ShownFrame', UIParent, 'SecureHandlerStateTemplate')
local function note()
    out[#out + 1] = tostring(f:GetAttribute('state-page')) .. ' ' .. tostring(shown:IsShown())
end
f:SetAttribute('_onstate-page', [[ self:SetAttribute('seen', newstate) ]])
RegisterStateDriver(f, 'page', '[bar:2] 2; [mod:shift] 3; [@player,nomounted] 1; 4')
RegisterStateDriver(shown, 'visibility', '[combat] show; hide')
note()
C_ActionBar.SetActionBarPage(2)
note()
mock.nextFrame()
note()
C_ActionBar.SetActionBarPage(1)
mock.setModifiers({ shift = true })
mock.nextFrame()
note()
mock.setModifiers({})
mock.nextFrame()
note()
mock.player:enterCombat()
mock.nextFrame()
note()
out[#out + 1] = tostring(f:GetAttribute('seen')) .. ' ' .. #mock.refusals()
mock.player:leaveCombat()
local resting = CreateFrame('Frame', nil, UIParent, 'SecureHandlerStateTemplate')
RegisterStateDriver(resting, 'rest', '[resting] rest; out')
mock.nextFrame()
mock.setResting(true)
mock.nextFrame()
out[#out + 1] = resting:GetAttribute('state-rest')
mock.advanceFrames(15)
out[#out + 1] = resting:GetAttribute('state-rest')
return unpack(out)",
    );
    assert_eq!(
        r,
        [
            "1 false", "1 false", "2 false", "3 false", "1 false", "1 true", "1 0", "out", "rest"
        ]
    );
}

/// A secure click handler's snippet runs on the user's click
/// (`SecureHandlerClickTemplate`, an addon's button: Blizzard's handler
/// code is secure).
#[test]
fn a_secure_click_runs_its_snippet() {
    let Some(mut vm) = client() else {
        eprintln!("skipped: the UI code is missing");
        return;
    };
    let r = run(
        &mut vm,
        "local errs = {}
seterrorhandler(function(e) errs[#errs + 1] = e end)
local b = CreateFrame('Button', 'ToggleButton', UIParent, 'SecureHandlerClickTemplate')
b:SetSize(20, 20)
b:SetPoint('CENTER')
b:RegisterForClicks('LeftButtonUp')
b:SetAttribute('_onclick', [[ self:SetAttribute('ran', (self:GetAttribute('ran') or 0) + 1) ]])
mock.click(b)
return tostring(b:GetAttribute('ran')), tostring(errs[1])",
    );
    assert_eq!(r, ["1", "nil"]);
}
