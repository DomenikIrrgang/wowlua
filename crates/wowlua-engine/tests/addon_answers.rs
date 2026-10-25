//! What addons ask the client beyond what Blizzard's UI asks: timed signal
//! maps, heal prediction calculators, addons' enable states, CVar
//! bitfields, masks, zero-delay timers set by timers.

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

/// Chat the player sends gets the server's answer on the next frame: what
/// they say, a whisper to themselves (sent, then heard), the error for a
/// name nobody has, and for party chat outside a party (decision T360).
#[test]
fn chat_messages_get_the_servers_answer() {
    let mut vm = vm();
    run(
        &mut vm,
        "Got = {}
local f = CreateFrame('Frame')
for _, e in ipairs({ 'CHAT_MSG_SAY', 'CHAT_MSG_WHISPER', 'CHAT_MSG_WHISPER_INFORM', 'CHAT_MSG_SYSTEM' }) do f:RegisterEvent(e) end
f:SetScript('OnEvent', function(_, e, text, author) Got[#Got + 1] = e .. '|' .. text .. '|' .. author end)
local me = UnitName('player')
C_ChatInfo.SendChatMessage('hello', 'SAY')
C_ChatInfo.SendChatMessage('psst', 'WHISPER', nil, me)
C_ChatInfo.SendChatMessage('anyone', 'WHISPER', nil, 'Nobody')
C_ChatInfo.SendChatMessage('team', 'PARTY')",
    );
    let before = run(&mut vm, "return #Got");
    assert_eq!(before, ["0"], "answered at once, not on the next frame");
    wowlua_engine::clock::advance_frames(&mut vm, 1, 1000.0 / 60.0);
    let r = run(
        &mut vm,
        "return table.concat(Got, '\\n'), UnitName('player')",
    );
    let me = &r[1];
    let lines: Vec<&str> = r[0].lines().collect();
    assert_eq!(lines.len(), 5, "{lines:?}");
    assert!(
        lines[0].starts_with(&format!("CHAT_MSG_SAY|hello|{me}")),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with(&format!("CHAT_MSG_WHISPER_INFORM|psst|{me}")),
        "{lines:?}"
    );
    assert!(
        lines[2].starts_with(&format!("CHAT_MSG_WHISPER|psst|{me}")),
        "{lines:?}"
    );
    assert_eq!(
        lines[3],
        "CHAT_MSG_SYSTEM|No player named 'Nobody' is currently playing.|"
    );
    assert_eq!(lines[4], "CHAT_MSG_SYSTEM|You aren't in a party.|");
}

/// What the player types gets the server's answer a round trip later, in
/// Blizzard's words (decision T362): a roll in its range, away and back,
/// names the world does not have, the guild master's leave refused, a
/// who's count; an emote is not restricted.
#[test]
fn commands_get_the_servers_answers() {
    let mut vm = vm();
    run(
        &mut vm,
        "Lines = {}
local f = CreateFrame('Frame')
f:RegisterEvent('CHAT_MSG_SYSTEM')
f:SetScript('OnEvent', function(_, _, text) Lines[#Lines + 1] = text end)
RandomRoll(1, 6)
C_ChatInfo.SendChatMessage('', 'AFK')
C_FriendList.AddOrDelIgnore('Some Body')
C_PartyInfo.InviteUnit('Some Body')
C_PartyInfo.UninviteUnit('Some Body')
C_GuildInfo.Invite('Some Body')
C_GuildInfo.Promote('Some Body')
C_GuildInfo.Leave()
C_FriendList.SendWho('Some')
Emote = C_ChatInfo.PerformEmote('DANCE')",
    );
    wowlua_engine::clock::advance_frames(&mut vm, 30, 1000.0 / 60.0);
    let r = run(
        &mut vm,
        "return table.concat(Lines, '\\n'), tostring(UnitIsAFK('player')), tostring(Emote), UnitName('player')",
    );
    let lines: Vec<&str> = r[0].lines().collect();
    let me = &r[3];
    assert!(
        lines.iter().any(|l| {
            l.strip_prefix(&format!("{me} rolls "))
                .and_then(|rest| rest.strip_suffix(" (1-6)"))
                .and_then(|n| n.parse::<u32>().ok())
                .is_some_and(|n| (1..=6).contains(&n))
        }),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"You are now AFK: Away from Keyboard"),
        "{lines:?}"
    );
    assert!(lines.contains(&"Player not found."), "{lines:?}");
    assert!(lines.contains(&"Cannot find 'Some Body'."), "{lines:?}");
    assert!(lines.contains(&"You aren't in a party."), "{lines:?}");
    assert!(lines.contains(&"\"Some Body\" not found."), "{lines:?}");
    assert!(lines.contains(&"0 |4player:players; total"), "{lines:?}");
    assert_eq!(r[1], "true", "away");
    assert_eq!(r[2], "false", "the emote is not restricted");
}

/// `|4one:many;` reads the number before it, as the client writes "1 day"
/// and "2 days".
#[test]
fn plural_escapes_follow_the_number_before() {
    assert_eq!(
        wowlua_engine::text::visible("1 |4day:days;, 2 |4hour:hours;"),
        "1 day, 2 hours"
    );
    assert_eq!(
        wowlua_engine::text::visible("0 |4player:players; total"),
        "0 players total"
    );
}

/// The debugger's console shows a chat line as the chat frame does.
#[test]
fn plain_text_leaves_out_colours_links_and_textures() {
    assert_eq!(
        wowlua_engine::text::plain(
            "|cff33ff99Demo|r: |HurlIndex:64|h|cnVISITABLE_URL_DEFAULT_CHAT_LINK_COLOR:a link|r|h |TInterface\\Icons\\Spell_Nature_Rejuvenation:0|t, 2 |4day:days;"
        ),
        "Demo: a link , 2 days"
    );
}

/// Other players of the world who accept invitations join a moment after
/// the player's `/invite` (decision T363): a party of five, then full; a
/// raid after converting, its second group filling; the leader removes and
/// promotes; a who counts the online players; a guild invitation joins the
/// roster. One who does not accept stays out.
#[test]
fn online_players_accept_invitations() {
    let mut vm = vm();
    let step = |vm: &mut Vm, seconds: f64| {
        wowlua_engine::clock::advance_frames(vm, (seconds * 60.0) as usize, 1000.0 / 60.0);
    };
    run(
        &mut vm,
        "Lines = {}
local f = CreateFrame('Frame')
f:RegisterEvent('CHAT_MSG_SYSTEM')
f:SetScript('OnEvent', function(_, _, text) Lines[#Lines + 1] = text end)
for i, n in ipairs({ 'Ana', 'Bo', 'Cy', 'Di', 'Ed', 'Fay' }) do
    mock.spawnUnit({ name = n, lastName = 'Test', player = true, class = 'MAGE', level = 60, acceptsInvites = true })
end
mock.spawnUnit({ name = 'Shy', lastName = 'Test', player = true, class = 'MAGE', level = 60 })
for _, n in ipairs({ 'Ana Test', 'Bo Test', 'Cy Test', 'Di Test', 'Shy Test' }) do C_PartyInfo.InviteUnit(n) end",
    );
    step(&mut vm, 3.0);
    let r = run(&mut vm, "return GetNumGroupMembers(), tostring(IsInRaid())");
    assert_eq!(r, ["5", "false"], "four joined, the shy one not");
    run(&mut vm, "Lines = {} C_PartyInfo.InviteUnit('Ed Test')");
    step(&mut vm, 1.0);
    assert_eq!(
        run(&mut vm, "return table.concat(Lines, '|')"),
        ["Your party is full."]
    );
    run(
        &mut vm,
        "C_PartyInfo.ConvertToRaid() C_PartyInfo.InviteUnit('Ed Test') C_PartyInfo.InviteUnit('Fay Test')",
    );
    step(&mut vm, 3.0);
    let r = run(
        &mut vm,
        "local _, _, g = GetRaidRosterInfo(GetNumGroupMembers()) return GetNumGroupMembers(), tostring(IsInRaid()), g",
    );
    assert_eq!(r, ["7", "true", "2"], "the raid's second group");
    run(
        &mut vm,
        "Lines = {} C_PartyInfo.UninviteUnit('Fay Test') C_PartyInfo.PromoteToLeader('Ana Test')",
    );
    step(&mut vm, 1.0);
    let r = run(
        &mut vm,
        "local leader for i = 1, GetNumGroupMembers() do local n, rank = GetRaidRosterInfo(i) if rank == 2 then leader = n end end
return GetNumGroupMembers(), tostring(UnitIsGroupLeader('player')), leader, table.concat(Lines, '|')",
    );
    assert_eq!(r[0], "6");
    assert_eq!(r[1], "false");
    assert_eq!(r[2], "Ana Test");
    assert!(
        r[3].contains("Fay Test has left the raid group"),
        "{}",
        r[3]
    );
    assert!(
        r[3].contains("Ana Test is now the group leader."),
        "{}",
        r[3]
    );
    run(
        &mut vm,
        "Lines = {} Before = GetNumGuildMembers() C_FriendList.SendWho('Test') C_GuildInfo.Invite('Ed Test')",
    );
    step(&mut vm, 3.0);
    let r = run(
        &mut vm,
        "return table.concat(Lines, '|'), GetNumGuildMembers() - Before, (C_FriendList.GetNumWhoResults())",
    );
    assert!(r[0].contains("7 |4player:players; total"), "{}", r[0]);
    assert!(r[0].contains("Ed Test has joined the guild."), "{}", r[0]);
    assert_eq!(r[1], "1", "the guild has one more member");
    assert_eq!(r[2], "7");
}
