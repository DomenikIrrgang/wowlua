//! What addons ask the client beyond what Blizzard's UI asks: timed signal
//! maps, heal prediction calculators, addons' enable states, CVar
//! bitfields, masks, zero-delay timers set by timers.

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
fn timed_signal_maps_signal_due_keys() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "Got = {}
Map = C_Timer.NewTimedSignalMap(function(key) Got[#Got + 1] = key end)
function Map:RegisterCallback() return 1 end
Map:SignalAfter(1, 0.5)
Map:SignalAfter(2, 0.2)
Map:SignalAt(3, GetTime() + 1)
Map:SignalAfter(3, 0.3)
Map:SignalAfter(4, 2)
Map:CancelSignal(4)
local key, time = Map:GetNextSignal()
return Map:GetSignalCount(), key, time, tostring(Map:HasSignal(4)), Map:RegisterCallback()",
    );
    assert_eq!(r, ["3", "2", "0.2", "false", "1"]);
    wowlua_engine::clock::advance(&mut vm, 1.0);
    let r = run(
        &mut vm,
        "return table.concat(Got, ' '), Map:GetSignalCount(), tostring(Map:GetSignalTime(1))",
    );
    assert_eq!(r, ["2 3 1", "0", "nil"]);
}

#[test]
fn a_zero_delay_timer_set_by_a_timer_waits_for_the_next_frame() {
    let mut vm = vm();
    run(
        &mut vm,
        "Runs = 0
local function again() Runs = Runs + 1; C_Timer.After(0, again) end
C_Timer.After(0, again)",
    );
    wowlua_engine::clock::advance(&mut vm, 5.0);
    assert_eq!(run(&mut vm, "return Runs"), ["1"]);
    wowlua_engine::clock::advance_frames(&mut vm, 3, 1000.0 / 60.0);
    assert_eq!(run(&mut vm, "return Runs"), ["4"]);
}

#[test]
fn heal_prediction_calculators_read_the_unit() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local calc = CreateUnitHealPredictionCalculator()
local before, plain = calc:GetMaximumHealth(), calc:HasSecretValues()
calc:SetMaximumHealthMode(1)
UnitGetDetailedHealPrediction('player', nil, calc)
local _, clamped = calc:GetDamageAbsorbs()
return before, tostring(plain), tostring(calc:HasSecretValues()),
    tostring(issecretvalue(calc:GetCurrentHealth())), tostring(clamped),
    calc:GetMaximumHealthMode(), calc:GetHealAbsorbMode()",
    );
    assert_eq!(r, ["0", "false", "true", "true", "false", "1", "0"]);
}

#[test]
fn addons_enable_states_and_existence() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineAddon('Extra', { loadOnDemand = true })
local known, unknown = C_AddOns.DoesAddOnExist('Extra'), C_AddOns.DoesAddOnExist('Nope')
local before = C_AddOns.GetAddOnEnableState('Extra')
C_AddOns.DisableAddOn('Extra')
local disabled = C_AddOns.GetAddOnEnableState('Extra')
C_AddOns.EnableAddOn('Extra')
C_AddOns.SaveAddOns()
return tostring(known), tostring(unknown), before, disabled,
    C_AddOns.GetAddOnEnableState('Extra'), C_AddOns.GetAddOnEnableState('Nope')",
    );
    assert_eq!(r, ["true", "false", "2", "0", "2", "0"]);
    // What Blizzard's addon list reads of each.
    let r = run(
        &mut vm,
        "mock.defineAddon('Second', { metadata = { Title = 'The Second', Group = 'Extra' } })
return C_AddOns.GetAddOnMetadata(1, 'Group'), C_AddOns.GetAddOnMetadata('Second', 'Group'),
    C_AddOns.GetAddOnName(2), C_AddOns.GetAddOnTitle(1), C_AddOns.GetAddOnTitle(2),
    C_AddOns.GetAddOnSecurity(1), tostring(C_AddOns.IsAddOnLoadable(1)),
    tostring(C_AddOns.IsAddOnLoaded(1))",
    );
    assert_eq!(
        r,
        [
            "Extra",
            "Extra",
            "Second",
            "Extra",
            "The Second",
            "1",
            "true",
            "false"
        ]
    );
}

#[test]
fn cvar_bitfields_tell_only_changes() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "Told = 0
local f = CreateFrame('Frame')
f:RegisterEvent('CVAR_UPDATE')
f:SetScript('OnEvent', function() Told = Told + 1 end)
local before = C_CVar.GetCVarBitfield('closedInfoFrames', 8)
C_CVar.SetCVarBitfield('closedInfoFrames', 8, true)
local set = C_CVar.GetCVarBitfield('closedInfoFrames', 8)
local other = C_CVar.GetCVarBitfield('closedInfoFrames', 7)
C_CVar.SetCVarBitfield('closedInfoFrames', 8, true)
C_CVar.SetCVarBitfield('closedInfoFrames', 8, false)
return tostring(before), tostring(set), tostring(other),
    tostring(C_CVar.GetCVarBitfield('closedInfoFrames', 8)), Told",
    );
    // Told for the two changes, not for setting the bit already set.
    assert_eq!(r, ["false", "true", "false", "false", "2"]);
}

#[test]
fn textures_keep_their_masks_and_tiling() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
local t = f:CreateTexture()
local m1, m2 = f:CreateMaskTexture(), f:CreateMaskTexture()
t:AddMaskTexture(m1)
t:AddMaskTexture(m2)
t:AddMaskTexture(m1)
local two = t:GetNumMaskTextures()
t:RemoveMaskTexture(m1)
local tile = t:GetVertTile()
t:SetVertTile(true)
return two, t:GetNumMaskTextures(), t:GetMaskTexture(1) == m2, tostring(tile), tostring(t:GetVertTile())",
    );
    assert_eq!(r, ["2", "1", "true", "false", "true"]);
}

/// An addon's texture copied by its ID (`b:SetTexture(a:GetTexture())`)
/// still shows the addon's file.
#[test]
fn a_loose_files_id_shows_its_file() {
    use wowlua_engine::scene::{Image, Item};
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.defineAddon('Mine', {})
local a = UIParent:CreateTexture()
a:SetTexture('Interface\\\\AddOns\\\\Mine\\\\logo.tga')
local b = UIParent:CreateTexture()
b:SetSize(32, 32)
b:SetPoint('CENTER')
b:SetTexture(a:GetTexture())
return tostring(a:GetTexture() < 0), tostring(b:GetTexture() == a:GetTexture())",
    );
    assert_eq!(r, ["true", "true"]);
    let scene = wowlua_engine::scene::build(&mut vm);
    assert!(scene.items.iter().any(|i| matches!(i,
        Item::Quad(q) if matches!(&q.image, Image::File(p) if p == "interface\\addons\\mine\\logo"))));
}

/// The skill line selected is the one `SetSelectedSkill` chose (the skills
/// window selects its first skill so and shows its details).
#[test]
fn the_selected_skill_is_kept() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local before = C_SkillInfo.GetSelectedSkill()
C_SkillInfo.SetSelectedSkill(3)
return before, C_SkillInfo.GetSelectedSkill()",
    );
    assert_eq!(r, ["0", "3"]);
}
