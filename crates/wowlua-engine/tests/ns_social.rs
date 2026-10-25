//! The player's social world: no friends or clubs but the guild's unless
//! the test says so; names without the player's realm; the voice CVars.

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
fn ambiguate_drops_the_players_own_realm() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local realm = GetNormalizedRealmName()
return Ambiguate('Someone-' .. realm, 'none'), Ambiguate('Someone-Elsewhere', 'none'),
    Ambiguate('Someone-Elsewhere', 'short'), Ambiguate('Someone-' .. realm, 'mail'),
    Ambiguate('First Last', 'none')",
    );
    let realm = run(&mut vm, "return GetNormalizedRealmName()")[0].clone();
    assert_eq!(
        r,
        [
            "Someone".to_string(),
            "Someone-Elsewhere".into(),
            "Someone".into(),
            format!("Someone-{realm}"),
            "First Last".into(),
        ]
    );
}

#[test]
fn a_guild_but_no_friends() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "return (GetGuildInfo('player')), GetNumGuildMembers(), C_FriendList.GetNumFriends(),
    #C_Club.GetSubscribedClubs(), CanGuildInvite(), BNConnected()",
    );
    assert_eq!(r, ["Guild", "27", "0", "1", "true", "true"]);
    let r = run(
        &mut vm,
        "mock.leaveGuild()
local name, rank, index = GetGuildInfo('player')
return name, rank, index, GetNumGuildMembers(), #C_Club.GetSubscribedClubs(), CanGuildInvite()",
    );
    assert_eq!(r, ["nil", "nil", "0", "0", "0", "false"]);
}

#[test]
fn voice_settings_are_the_cvars() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "local before = C_VoiceChat.GetOutputVolume()
C_VoiceChat.SetOutputVolume(30)
C_VoiceChat.ToggleDeafened()
return before, C_VoiceChat.GetOutputVolume(), C_VoiceChat.IsDeafened(),
    C_VoiceChat.GetCommunicationMode()",
    );
    assert_eq!(r, ["50", "30", "true", "0"]);
}

#[test]
fn raid_target_expressions_become_icons() {
    let mut vm = vm();
    let r = run(
        &mut vm,
        "return C_ChatInfo.ReplaceIconAndGroupExpressions('a {rt8} b {Star} {g1}')",
    );
    assert_eq!(
        r,
        [
            "a |TInterface\\TargetingFrame\\UI-RaidTargetingIcon_8:0|t b |TInterface\\TargetingFrame\\UI-RaidTargetingIcon_1:0|t {g1}"
        ]
    );
}
