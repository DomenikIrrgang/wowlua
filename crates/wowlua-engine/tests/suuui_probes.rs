//! What SuuUI's probe diagnostics recorded of WoW Forever 1.60.1 (70450)
//! that the engine answers: function and event answers, widget methods,
//! the XML's frames.

#![allow(clippy::disallowed_methods)] // (tests set up their files on the disk)

use std::path::PathBuf;

use wowlua_vm::{Value, Vm};

fn vm() -> Vm {
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    wowlua_engine::controls::install(&mut vm, "mock");
    vm
}

/// The engine with Blizzard's UI loaded and logged in, if `cargo xtask
/// types` fetched the UI code.
fn ui_vm() -> Option<Vm> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let interface = repo.join("target/types-src/camelot/Interface");
    if !interface.exists() {
        eprintln!("skipped: {} is missing", interface.display());
        return None;
    }
    let xtask = std::fs::read_to_string(repo.join("xtask/src/types.rs")).unwrap_or_default();
    let commit = xtask
        .split("\"camelot\",")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(3))
        .unwrap_or("");
    let strings = repo.join(format!("target/types-src/strings/{commit}.lua"));
    let mut vm = Vm::wow();
    wowlua_engine::install(&mut vm, "camelot").expect("installs");
    let report = wowlua_engine::ui::load(
        &mut vm,
        &interface,
        strings.exists().then_some(strings.as_path()),
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    wowlua_engine::controls::install(&mut vm, "mock");
    wowlua_engine::login::login(&mut vm, "MyAddon", false, false);
    wowlua_engine::clock::advance_frames(&mut vm, 60, 1000.0 / 60.0);
    Some(vm)
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
fn the_preferred_play_settings_arrive_as_their_event() {
    // Logged in either way: the request is answered only after the login
    // (without the UI code, as on CI, the plain engine logs in here).
    let mut vm = ui_vm().unwrap_or_else(|| {
        let mut vm = vm();
        wowlua_engine::login::login(&mut vm, "MyAddon", false, false);
        vm
    });
    let r = run(
        &mut vm,
        "local f = CreateFrame('Frame')
local registered = pcall(f.RegisterEvent, f, 'PREFERRED_PLAY_SETTINGS_UPDATED')
local got
f:SetScript('OnEvent', function(_, event, ...) got = table.concat({ event, ... }, ' ') end)
local started = C_AccountServices.RequestPreferredPlaySettings()
mock.advanceTime(1)
return registered, started, got",
    );
    assert_eq!(
        r,
        ["true", "true", "PREFERRED_PLAY_SETTINGS_UPDATED 1 1 1 0 0"]
    );
}

#[test]
fn aura_sounds_need_a_sound_file() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local U = C_UnitAuras
local function sound(extra)
    local t = { spellID = 1126, unitToken = 'player' }
    for k, v in pairs(extra) do t[k] = v end
    return t
end
local kit = U.AddAuraSound(Enum.UnitAuraSoundTrigger.Added, sound({ soundKitID = 8960 }))
local file = U.AddAuraSound(Enum.UnitAuraSoundTrigger.Added, sound({ soundFileID = 567478 }))
U.RemoveAuraSound(file)
local named = U.AddAuraSound(Enum.UnitAuraSoundTrigger.Added, sound({ soundFileName = 'Sound\\\\Interface\\\\RaidWarning.ogg', outputChannel = 'Master' }))
local _, badSpell = pcall(U.AddAuraSound, 0, { spellID = 'Mark of the Wild', unitToken = 'player', soundKitID = 8960 })
local _, noUnit = pcall(U.AddAuraSound, 0, {})
return kit, file, named, badSpell, noUnit",
    );
    assert_eq!(
        r,
        [
            "nil",
            "1",
            "2",
            "bad argument #3 to '?' (Current Field: [spellID], outside of expected range -2147483648 to 2147483647 - Usage: local auraSoundID = C_UnitAuras.AddAuraSound(trigger, sound))",
            "bad argument #3 to '?' (Current Field: [unitToken], Usage: local auraSoundID = C_UnitAuras.AddAuraSound(trigger, sound))",
        ]
    );
}

#[test]
fn the_minimap_and_the_bank_frame_as_the_client_has_them() {
    let Some(mut vm) = ui_vm() else { return };
    let r = run(
        &mut vm,
        "return type(Minimap.SetPlayerTexture), type(Minimap.SetBlipTexture), type(Minimap.SetZoom),
    type(Minimap.SetTaskBlobRingTexture), type(BankFramePurchaseButton),
    BankFrame.BankPanel.PurchaseButton:GetObjectType()",
    );
    assert_eq!(r, ["nil", "nil", "function", "function", "nil", "Button"]);
}

#[test]
fn addon_code_is_refused_what_the_client_refused() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ADDON_ACTION_BLOCKED')
f:RegisterEvent('ADDON_ACTION_FORBIDDEN')
f:SetScript('OnEvent', function(_, event, _, action) seen[#seen + 1] = event:sub(14) .. ' ' .. action end)
pcall(AcceptTrade)
pcall(CancelLogout)
pcall(DeleteCursorItem)
pcall(AcceptProposal)
pcall(AcceptBattlefieldPort, 1, false)
Logout()
pcall(GuildControlSetRank, 1)
pcall(C_GuildInfo.SetMOTD, 'x')
pcall(AcceptGroup)
pcall(CancelTrade)
return table.concat(seen, ', ')",
    );
    assert_eq!(
        r,
        [
            "BLOCKED UNKNOWN(), BLOCKED UNKNOWN(), BLOCKED UNKNOWN(), BLOCKED UNKNOWN(), \
          FORBIDDEN UNKNOWN(), FORBIDDEN Logout(), FORBIDDEN UNKNOWN(), FORBIDDEN UNKNOWN()"
        ]
    );
}

#[test]
fn answers_the_probes_recorded() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local got
local f = CreateFrame('Frame')
f:RegisterEvent('START_PLAYER_COUNTDOWN')
f:SetScript('OnEvent', function(_, _, guid, left, total, chat, name)
    got = table.concat({ tostring(guid == UnitGUID('player')), left, total, tostring(chat), tostring(name == UnitName('player')) }, ' ')
end)
C_PartyInfo.DoCountdown(10)
local wp = C_Map.GetUserWaypointFromHyperlink('|cffffff00|Hworldmap:1454:5657:3315|h[x]|h|r')
local cost = C_Spell.GetSpellPowerCost(647)
local _, bookError = pcall(C_SpellBook.GetSpellBookItemLevelLearned, 647)
return got, C_Bank.BankBagTypeAndIDToInvSlot(0, 1), C_Bank.BankBagTypeAndIDToInvSlot(0, 9),
    C_BattleNet.AreTitleFriendsEnabled(), C_BattleNet.AreTitleFriendCustomNamesEnabled(),
    (C_DamageMeter.IsDamageMeterAvailable()), C_Minimap.GetUiMapID(), C_QuestLog.GetRequiredMoney(1),
    C_Spell.GetSpellSubtext(647), cost[1].cost, bookError, ('%.14g'):format(wp.position.x),
    type(C_EncounterTimeline.GetCurrentTime())",
    );
    assert_eq!(
        r,
        [
            "true 10 10 true true",
            "63",
            "71",
            "true",
            "true",
            "true",
            "nil",
            "-1",
            "Rank 3",
            "110",
            // The client's usage names the slot and bank as one argument
            // (`/stdtest engine fullprobe`).
            "bad argument #1 to '?' (not a numerical value - Usage: local levelLearned = C_SpellBook.GetSpellBookItemLevelLearned(spellBookItem))",
            "0.56569999456406",
            "number",
        ]
    );
}

#[test]
fn aura_filters_by_kind_as_the_game_has_them() {
    // WoW Forever 1.60.1 (2026-11-08): `HARMFUL|DISPELLABLE` kept a typed
    // debuff (Corrosive Poison, Poison) and not Forbearance;
    // `HARMFUL|RAID_IN_COMBAT` kept neither; `HARMFUL|CROWD_CONTROL` kept
    // neither (no crowd control on the player).
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.player:applyAura({ spellId = 25771, name = 'Forbearance', harmful = true })
mock.player:applyAura({ spellId = 3396, name = 'Corrosive Poison', harmful = true, dispelName = 'Poison' })
mock.player:applyAura({ spellId = 853, name = 'Hammer of Justice', harmful = true, dispelName = 'Magic', stuns = true })
mock.player:applyAura({ spellId = 1090, name = 'Sleep', harmful = true, crowdControl = true })
local function names(filter)
    local out = {}
    for _, a in ipairs(C_UnitAuras.GetUnitAuras('player', filter)) do out[#out + 1] = a.name end
    return table.concat(out, ',')
end
return names('HARMFUL|DISPELLABLE'), names('HARMFUL|!DISPELLABLE'), names('HARMFUL|CROWD_CONTROL'),
    names('HARMFUL|CROWD_CONTROL|!DISPELLABLE'), names('HARMFUL|RAID_IN_COMBAT')",
    );
    assert_eq!(
        r,
        [
            "Corrosive Poison,Hammer of Justice",
            "Forbearance,Sleep",
            "Hammer of Justice,Sleep",
            "Sleep",
            ""
        ]
    );
}

#[test]
fn aura_filters_negate_and_know_their_words() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local other = mock.spawnUnit({ name = 'Mage', level = 20 })
mock.player:applyAura({ spellId = 643, name = 'Devotion Aura', source = mock.player })
mock.player:applyAura({ spellId = 1461, name = 'Arcane Intellect', source = other })
mock.player:applyAura({ spellId = 25771, name = 'Forbearance', harmful = true, source = mock.player })
local function names(filter)
    local out = {}
    for _, a in ipairs(C_UnitAuras.GetUnitAuras('player', filter)) do out[#out + 1] = a.name end
    return table.concat(out, ',')
end
return names('HELPFUL|!PLAYER'), names('HELPFUL|PLAYER'), names('HELPFUL|!CANCELABLE'),
    names('HELPFUL|EXTERNAL_DEFENSIVE'), names('HELPFUL|!EXTERNAL_DEFENSIVE|!BIG_DEFENSIVE'),
    names('HARMFUL|RAID_PLAYER_DISPELLABLE'), names('HARMFUL|PLAYER')",
    );
    assert_eq!(
        r,
        [
            "Arcane Intellect",
            "Devotion Aura",
            "",
            "",
            "Devotion Aura,Arcane Intellect",
            "",
            "Forbearance"
        ]
    );
}

#[test]
fn a_secret_number_abbreviates_to_a_secret() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local s = secretwrap(12345)
return issecretvalue(AbbreviateNumbers(s)), AbbreviateNumbers(12345)",
    );
    assert_eq!(r, ["true", "12K"]);
}

#[test]
fn blizzard_xml_as_the_client_builds_it() {
    let Some(mut vm) = ui_vm() else { return };
    let r = run(
        &mut vm,
        "local _, missing = pcall(CreateFrame, 'Button', nil, UIParent, 'BankItemButtonGenericTemplate')
local t = UIParent:CreateTexture()
t:SetDrawLayer('OVERLAY')
-- Placed once the window is (unplaced, the slots have no rectangle).
ToggleCharacter('PaperDollFrame')
local icon = CharacterHeadSlotIconTexture
return missing, ChatFrame1EditBox:GetHistoryLines(), select(2, t:GetDrawLayer()),
    CharacterFrame.NineSlice.TopLeftCorner:GetDrawLayer(),
    PaperDollFrame:GetFrameLevel() == CharacterFrame:GetFrameLevel(),
    icon:GetNumPoints(), icon:GetWidth() == CharacterHeadSlot:GetWidth()",
    );
    assert_eq!(
        r,
        [
            "CreateFrame(): Couldn't find inherited node 'BankItemButtonGenericTemplate'\nLua Taint: MyAddon",
            "32",
            "0",
            "OVERLAY",
            "true",
            "2",
            "true"
        ]
    );
}

/// A secure button's macro runs its lines as typed into chat, through the
/// line callback Blizzard's UI gives (decision T601): `/click <name>
/// <button>` clicks that button with that button, in combat too, as
/// Blizzard's handler does (a frame that is no button and a name nothing
/// has: nothing); `#` lines run nothing, `/stopmacro` stops the rest; a
/// saved macro (`macro`) runs as its text does; `/click` typed in chat
/// clicks too. (SuuUI's tests found the macro doing nothing.)
#[test]
fn a_macro_clicks_with_click() {
    let Some(mut vm) = ui_vm() else { return };
    let r = run(
        &mut vm,
        "local errs = {}
seterrorhandler(function(e) errs[#errs + 1] = e end)
local got = {}
local target = CreateFrame('Button', 'MyClickTarget', UIParent)
target:SetScript('OnClick', function(_, button, down) got[#got + 1] = button .. ' ' .. tostring(down) end)
local plain = CreateFrame('Frame', 'MyPlainFrame', UIParent)
plain:SetScript('OnMouseUp', function() got[#got + 1] = 'plain' end)
local b = CreateFrame('Button', 'MyMacroButton', UIParent, 'SecureActionButtonTemplate')
b:RegisterForClicks('AnyDown', 'AnyUp')
b:SetAttribute('type1', 'macro')
b:SetAttribute('macrotext1', '#showtooltip\\n/click MyClickTarget\\n/stopmacro\\n/click MyClickTarget Button4')
b:SetAttribute('type2', 'macro')
b:SetAttribute('macrotext2', '/click MyClickTarget RightButton')
b:SetSize(20, 20); b:SetPoint('CENTER'); b:Show()
mock.click(b, 'RightButton')
mock.player:enterCombat()
mock.click(b, 'RightButton')
mock.click(b)
mock.player:leaveCombat()
local fought = table.concat(got, ',')
got = {}
b:SetAttribute('macrotext2', '/click MyPlainFrame RightButton\\n/click NoSuchFrame RightButton')
mock.click(b, 'RightButton')
mock.slash('/click MyClickTarget MiddleButton')
mock.setMacros({ account = { { name = 'Clicker', icon = 1, body = '/click MyClickTarget Button5' } } })
b:SetAttribute('macro2', 'Clicker')
mock.click(b, 'RightButton')
return fought, table.concat(got, ','), #mock.refusals(), tostring(errs[1])",
    );
    assert_eq!(
        r,
        [
            "RightButton false,RightButton false,LeftButton false",
            "MiddleButton false,Button5 false",
            "0",
            "nil"
        ]
    );
}

/// The game's pet bar button clicked by a macro's `/click PetActionButton4
/// RightButton` switches that slot's autocast, out of combat and in
/// (seen in game, WoW Forever 1.60.1; decisions T600, T601).
#[test]
fn a_macro_switches_the_pets_autocast() {
    let Some(mut vm) = ui_vm() else { return };
    let r = run(
        &mut vm,
        "mock.setPlayer({ name = 'Tamsin', level = 20, class = 'HUNTER', race = 'Dwarf' })
local wolf = mock.spawnUnit({ name = 'Fang', level = 19, reaction = 'friendly', creatureFamily = 'Wolf',
    creatureFamilyID = 1 })
mock.defineSpell(17253, { name = 'Bite', icon = 132278 })
mock.setPet(wolf, { actions = { [4] = { spell = 17253, name = 'Bite', icon = 132278, autocast = true } } })
mock.nextFrame(10)
local b = CreateFrame('Button', 'MyAutocastButton', UIParent, 'SecureActionButtonTemplate')
b:RegisterForClicks('AnyDown', 'AnyUp')
b:SetAttribute('type2', 'macro')
b:SetAttribute('macrotext2', '/click PetActionButton4 RightButton')
b:SetSize(20, 20); b:SetPoint('CENTER'); b:Show()
local seen = {}
local function note() seen[#seen + 1] = tostring(select(6, GetPetActionInfo(4))) end
note()
mock.click(b, 'RightButton')
note()
mock.player:enterCombat()
mock.click(b, 'RightButton')
note()
return table.concat(seen, ','), #mock.refusals()",
    );
    assert_eq!(r, ["true,false,true", "0"]);
}

/// An addon's own call of `C_Macro.RunMacroText` is forbidden, as the
/// global `RunMacroText` (decision T601).
#[test]
fn an_addon_may_not_run_a_macro_itself() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local seen = {}
local f = CreateFrame('Frame')
f:RegisterEvent('ADDON_ACTION_FORBIDDEN')
f:SetScript('OnEvent', function(_, _, addon, action) seen[#seen + 1] = addon .. ' ' .. action end)
C_Macro.RunMacroText('/click MyClickTarget', 'LeftButton')
return table.concat(seen, ','), #mock.refusals()",
    );
    assert_eq!(r, ["MyAddon RunMacroText()", "1"]);
}
