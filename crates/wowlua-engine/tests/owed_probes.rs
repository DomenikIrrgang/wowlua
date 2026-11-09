//! What WoW Forever answered to the owed in-game probes (`/stdtest engine
//! owed` and `/stdtest engine scenario mailwalk`, client 1.60.1.70862,
//! 2026-11-09; `tests/ingame/PROBES.md`), where the engine answered
//! otherwise before.

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

/// Walking away from an open mailbox ends the interaction: the transfer
/// cancelled and the mailbox's hide, each told twice, the interaction on
/// during the first hide; no `MAIL_CLOSED` (`mailwalk`).
#[test]
fn walking_away_from_a_mailbox_ends_the_interaction() {
    let mut vm = vm();
    run(
        &mut vm,
        "Box = mock.spawnUnit({ name = 'Mailbox', distance = 2 })
mock.defineMailbox(Box, { letters = {} })
mock.player:interact(Box)",
    );
    wowlua_engine::clock::advance(&mut vm, 1.0);
    let r = run(
        &mut vm,
        "Seen = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'MAIL_CLOSED', 'SECURE_TRANSFER_CANCEL', 'PLAYER_INTERACTION_MANAGER_FRAME_HIDE' }) do
    f:RegisterEvent(e)
end
f:SetScript('OnEvent', function(_, e, ...)
    Seen[#Seen + 1] = table.concat({ e, tostring((...)), tostring(C_PlayerInteractionManager.IsInteractingWithNpcOfType(17)) }, ':')
    -- (Blizzard's mail frame closes the mail on the hide)
    if e == 'PLAYER_INTERACTION_MANAGER_FRAME_HIDE' then CloseMail() end
end)
local open = C_PlayerInteractionManager.IsInteractingWithNpcOfType(17)
Box:setDistance(4)
local near = table.concat(Seen, ' ')
Box:setDistance(8)
return tostring(open), near, table.concat(Seen, ' '), tostring(C_PlayerInteractionManager.IsInteractingWithNpcOfType(17))",
    );
    assert_eq!(
        r,
        [
            "true",
            "",
            "SECURE_TRANSFER_CANCEL:nil:true SECURE_TRANSFER_CANCEL:nil:true \
             PLAYER_INTERACTION_MANAGER_FRAME_HIDE:17:true PLAYER_INTERACTION_MANAGER_FRAME_HIDE:17:false",
            "false"
        ]
    );
}

/// The heal prediction calculator's incoming heals overflow 1.05 (the
/// float's), and its maximum incoming heals with it.
#[test]
fn heal_calculators_overflow_by_five_percent() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local c = CreateUnitHealPredictionCalculator()
local overflow = c:GetIncomingHealOverflowPercent()
c:SetPredictedValues({ health = 60000, healthMax = 100000, totalDamageAbsorbs = 36000, totalHealAbsorbs = 5000,
    totalIncomingHeals = 12000, totalIncomingHealsFromHealer = 8000 })
return string.format('%.13g', overflow), string.format('%.14g', (c:GetMaximumIncomingHeals())),
    c:GetMaximumDamageAbsorbs(), c:GetMaximumHealAbsorbs()",
    );
    assert_eq!(r, ["1.049999952316", "44999.995231628", "33000", "60000"]);
}

/// A processed option keeps a structure given as it was given
/// (`backdropInfo = {}` stays empty), one left out filled.
#[test]
fn processed_options_keep_what_was_given() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local t = C_AuraContainerUtil.ProcessAuraTooltipBackdropOptions({ backdropInfo = {} })
return tostring(next(t.backdropInfo)), t.anchorOffsets.left, t.anchorOffsets.bottom",
    );
    assert_eq!(r, ["nil", "0", "0"]);
}

/// A mount the character lacks answers its info, not collected (mount 6,
/// the Brown Horse).
#[test]
fn a_mount_not_collected_answers_its_info() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local name, spell, icon, active, usable, source, favorite, factionSpecific, faction, hidden, collected, id =
    C_MountJournal.GetMountInfoByID(6)
return name, spell, icon, tostring(factionSpecific), faction, tostring(collected), id",
    );
    assert_eq!(
        r,
        ["Brown Horse", "458", "132261", "true", "1", "false", "6"]
    );
}

/// The edit box's `Insert`: its `OnTextChanged` tells user input, once in
/// the frame's update also after a `SetText` in the same frame; a
/// selection given backwards runs to the text's end.
#[test]
fn an_inserts_text_change_is_the_users() {
    let mut vm = vm();
    run(
        &mut vm,
        "Changed = {}
E = CreateFrame('EditBox', nil, UIParent)
E:SetSize(200, 20)
E:SetPoint('CENTER')
E:SetAutoFocus(false)
E:SetText('hello world')
E:SetScript('OnTextChanged', function(_, user) Changed[#Changed + 1] = tostring(user) end)",
    );
    wowlua_engine::clock::advance_frames(&mut vm, 1, 1000.0 / 60.0);
    run(&mut vm, "Changed = {} E:Insert('!')");
    wowlua_engine::clock::advance_frames(&mut vm, 1, 1000.0 / 60.0);
    run(&mut vm, "E:SetCursorPosition(1) E:SetText('abc')");
    wowlua_engine::clock::advance_frames(&mut vm, 1, 1000.0 / 60.0);
    let r = run(
        &mut vm,
        "E:SetText('abcdef') E:HighlightText(4, 1) E:Insert('Z') return E:GetText()",
    );
    assert_eq!(r, ["abcdZ"]);
    wowlua_engine::clock::advance_frames(&mut vm, 1, 1000.0 / 60.0);
    let r = run(&mut vm, "return table.concat(Changed, ' ')");
    assert_eq!(r, ["true false true"]);
}

/// The clears: a button's cleared normal texture stays its own and shown,
/// emptied; a model cleared has no file.
#[test]
fn cleared_textures_stay_shown_and_cleared_models_have_no_file() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local b = CreateFrame('Button', nil, UIParent)
b:SetNormalTexture('Interface\\\\Buttons\\\\WHITE8X8')
local t = b:GetNormalTexture()
b:ClearNormalTexture()
local m = CreateFrame('PlayerModel', nil, UIParent)
local fresh = m:GetModelFileID()
m:SetModel(130737)
m:ClearModel()
return tostring(b:GetNormalTexture() == t), tostring(t:GetTexture()), tostring(t:IsShown()), fresh,
    tostring(m:GetModelFileID())",
    );
    assert_eq!(r, ["true", "nil", "true", "0", "nil"]);
}

/// `GetTextureFilePath`: nil for an addon's file that is not there, an
/// atlas's file ID.
#[test]
fn a_textures_file_path_is_an_atlass_file_or_nothing_missing() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local t = UIParent:CreateTexture()
local set = t:SetTexture('Interface\\\\AddOns\\\\NoSuchAddon\\\\media\\\\logo')
local missing = { t:GetTexture(), t:GetTextureFileID(), t:GetTextureFilePath() }
t:SetAtlas('128-RedButton-Exit')
return tostring(set), #missing, t:GetTexture(), t:GetTextureFilePath()",
    );
    assert_eq!(r, ["true", "0", "8063723", "8063723"]);
}

/// A rectangle laid out is valid: a chain made in one call, the last
/// one's width asked for, answers it at once, its parent's too.
#[test]
fn a_rectangle_asked_for_is_valid() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local p = CreateFrame('Frame', nil, UIParent)
p:SetPoint('CENTER')
p:SetSize(100, 50)
local c = CreateFrame('Frame', nil, p)
c:SetAllPoints()
local g = CreateFrame('Frame', nil, c)
g:SetAllPoints()
local loose = CreateFrame('Frame', nil, UIParent)
loose:SetSize(10, 10)
return g:GetWidth(), tostring(g:IsRectValid()), tostring(c:IsRectValid()), tostring(loose:IsRectValid()),
    tostring(UIParent:IsRectValid())",
    );
    assert_eq!(r, ["100", "true", "true", "false", "true"]);
}

/// A mouse script turns the mouse on, its clicks and its motion alike;
/// `EnableMouse` too.
#[test]
fn mouse_scripts_turn_clicks_and_motion_on() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
f:SetScript('OnMouseUp', function() end)
local g = CreateFrame('Frame')
g:EnableMouse(true)
return tostring(f:IsMouseEnabled()), tostring(f:IsMouseClickEnabled()), tostring(f:IsMouseMotionEnabled()),
    tostring(g:IsMouseClickEnabled()), tostring(g:IsMouseMotionEnabled())",
    );
    assert_eq!(r, ["true", "true", "true", "true", "true"]);
}

/// A button's font object setters refuse a name that is no font in their
/// own words.
#[test]
fn a_buttons_font_setter_refuses_an_unknown_name_in_its_words() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local b = CreateFrame('Button', nil, UIParent)
return select(2, pcall(b.SetHighlightFontObject, b, 'NoSuchFont'))",
    );
    assert_eq!(
        r,
        ["Button:SetHighlightFontObject(): Usage: (\"fontname\" or fontObject)"]
    );
}
