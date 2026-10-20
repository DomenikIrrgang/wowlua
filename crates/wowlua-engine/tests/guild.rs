//! The player's guild: the camelot character leads the recorded guild;
//! the mock's controls put it in another, at another rank, or in none, and
//! change the roster.

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
fn the_camelot_character_leads_the_recorded_guild() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local name, rank, index = GetGuildInfo('player')
local total, online = GetNumGuildMembers()
local _, rank2, index2, level2, _, _, _, _, online2 = GetGuildRosterInfo(2)
return name, rank, index, total, online, IsGuildLeader(), GuildControlGetNumRanks(),
    rank2, index2, level2, online2, select(3, GetGuildRosterLastOnline(2)),
    GetNumMembersInRank(1), GetPromotionRank(2), C_Club.GetGuildClubId(),
    GetGuildBankWithdrawMoney() == 2 ^ 64, GetGuildBankTabCost()",
    );
    assert_eq!(
        r,
        [
            "Guild",
            "Guild Master",
            "0",
            "27",
            "1",
            "true",
            "5",
            "Initiate",
            "4",
            "3",
            "false",
            "24",
            "1",
            "3",
            "22684421",
            "true",
            "1000000"
        ]
    );
}

#[test]
fn a_member_of_a_lower_rank_has_its_rights() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.joinGuild({ name = 'Other Guild', rank = 3, members = {
    { name = 'Otherplayer', lastName = 'Othername', rank = 0, online = true, class = 'MAGE', level = 60 },
} })
local name, rank, index = GetGuildInfo('player')
local total, online = GetNumGuildMembers()
return name, rank, index, total, online, IsGuildLeader(), CanGuildInvite(),
    C_GuildInfo.CanSpeakInGuildChat(), CanEditMOTD(), C_GuildInfo.IsGuildOfficer(),
    GetGuildBankWithdrawMoney(), (GetGuildRosterInfo(2)), GetPromotionRank(2),
    C_GuildInfo.MemberExistsByName('Otherplayer Othername'), #C_Club.GetStreams(C_Club.GetGuildClubId())",
    );
    assert_eq!(
        r,
        [
            "Other Guild",
            "Member",
            "3",
            "2",
            "2",
            "false",
            "false",
            "true",
            "false",
            "false",
            "0",
            "Otherplayer Othername",
            "nil",
            "true",
            "1"
        ]
    );
}

#[test]
fn the_roster_changes_with_the_controls() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.joinGuild({})
local seen = 0
local f = CreateFrame('Frame')
f:RegisterEvent('GUILD_ROSTER_UPDATE')
f:SetScript('OnEvent', function() seen = seen + 1 end)
local h = mock.defineGuildMember({ name = 'Otherplayer', lastName = 'Othername', class = 'PRIEST', level = 12 })
local before = GetNumGuildMembers()
mock.updateGuildMember(h, { online = true })
local _, online = GetNumGuildMembers()
GuildPromote('Otherplayer Othername')
local _, _, promoted = GetGuildRosterInfo(2)
mock.removeGuildMember(h)
local after = GetNumGuildMembers()
mock.advanceTime(1)
return before, online, promoted, after, seen > 0",
    );
    assert_eq!(r, ["2", "2", "3", "1", "true"]);
}

#[test]
fn outside_a_guild_nothing_is_the_guilds() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "mock.leaveGuild()
return IsInGuild(), GetNumGuildMembers(), GuildControlGetNumRanks(), C_Club.GetGuildClubId(),
    #C_Club.GetSubscribedClubs(), CanGuildInvite(), C_ChatInfo.SendAddonMessage('X', 'y', 'GUILD')",
    );
    assert_eq!(r, ["false", "0", "0", "nil", "0", "false", "10"]);
}
