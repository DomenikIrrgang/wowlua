//! The player's social world: guild, friends, Battle.net, communities,
//! voice chat, the who list, chat lines and channels' odds and ends.
//!
//! The engine's character has no friends, no Battle.net friends, no
//! ignores, no communities but its guild's, no voice channels; its guild
//! is `guild.rs`'s. Battle.net is connected (recorded: `BNFeaturesEnabled`
//! true, `BNGetInfo` with a BattleTag). Voice chat is enabled and not
//! logged in (recorded: `C_VoiceChat.IsLoggedIn` false); its settings are
//! the `Voice…` CVars (recorded defaults: volumes 50, VAD sensitivity 43,
//! master volume scale 1, push to talk).
//!
//! Functions of systems with nothing in them answer the documentation's
//! "nothing": no values for a function that may return nothing, nil for a
//! nilable result, 0, false, an empty list; actions are accepted and do
//! nothing (unverified, each: the client's answers with the player in
//! such systems were not recorded).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, units};

// ---------------------------------------------------------------- shared

/// No values: an action, or a function that may return nothing.
fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn nil(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn yes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn zeros2(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0)])
}

fn no2(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false), Value::Bool(false)])
}

fn empty_string(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

fn empty_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

/// The documentation's empty values for a documented function (a
/// structure with its fields false, 0, "" and empty lists); nothing for
/// an undocumented one.
fn documented_empty(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Num(i)) = c.upvals.first().copied() else {
        return Ok(vec![]);
    };
    let api = engine(vm).api;
    let Some(f) = api.functions.get(i as usize) else {
        return Ok(vec![]);
    };
    Ok(crate::defaults(vm, api, &f.doc))
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// The player's realm as `GetNormalizedRealmName` gives it (the engine's
/// placeholder "Realm" when the world gives none; no spaces).
fn player_realm(vm: &Vm) -> String {
    engine(vm).units.list[units::PLAYER.0]
        .realm
        .clone()
        .unwrap_or_else(|| "Realm".into())
        .replace([' ', '-'], "")
}

/// A `Voice…` CVar as a number, else its recorded default.
fn cvar_number(vm: &mut Vm, name: &str, default: f64) -> f64 {
    crate::framescript::cvar_get(vm, name)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(default)
}

fn cvar_flag(vm: &mut Vm, name: &str) -> bool {
    cvar_number(vm, name, 0.0) != 0.0
}

fn set_cvar_text(vm: &mut Vm, name: &str, v: Value) {
    let value = match vm.hidden(v) {
        Value::Num(n) if n.fract() == 0.0 => format!("{}", n as i64),
        Value::Num(n) => n.to_string(),
        Value::Bool(b) => if b { "1" } else { "0" }.to_string(),
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => return,
    };
    crate::framescript::cvar_set(vm, name, value);
}

// ---------------------------------------------------------------- names

/// `Ambiguate(fullName, context)`: the name with its realm or without, as
/// warcraft.wiki.gg's table has it (unverified): "short" drops the realm
/// always; "none", "all" and "guild" drop the player's own realm (the
/// engine's guild has no one from another realm); "mail" and
/// "craftingorder" keep it. A name without a realm (WoW Forever's "First
/// Last") stays as it is.
fn ambiguate(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(full) = text(vm, c.arg(1)) else {
        return Err(vm.error("bad argument #1 to 'Ambiguate' (string expected)"));
    };
    let context = text(vm, c.arg(2)).unwrap_or_default().to_ascii_lowercase();
    let Some((name, realm)) = full.split_once('-') else {
        return Ok(vec![vm.str(&full)]);
    };
    let own = realm
        .replace([' ', '-'], "")
        .eq_ignore_ascii_case(&player_realm(vm));
    let short = match context.as_str() {
        "short" => true,
        "mail" | "craftingorder" => false,
        _ => own,
    };
    Ok(vec![vm.str(if short { name } else { &full })])
}

// ---------------------------------------------------------------- guild

/// `BNGetDisplayName(id)`: a text for any account ID, empty here with no
/// friends (the full probe: -1, 0, 1, 2 answered a string, its text
/// blanked; unverified: empty).
fn bn_get_display_name(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

/// `GuildControlGetRank()`: the rank the guild control panel edits, as
/// `GuildControlSetRank(rankOrder)` set it, 2 before (the full probe: 2
/// with any argument; Blizzard's `GuildControlUI` sets 2 on load; the
/// client's own start unverified).
fn guild_control_get_rank(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![crate::globalsinfo::get_or(
        vm,
        "guildControlRank",
        Value::Num(2.0),
    )])
}

/// `GuildControlSetRank(rankOrder)`: the rank `GuildControlGetRank` tells.
fn guild_control_set_rank(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Value::Num(n) = vm.hidden(c.arg(1)) {
        crate::globalsinfo::put(vm, "guildControlRank", Value::Num(n.trunc()));
    }
    Ok(vec![])
}

/// `GetGuildFactionGroup()`: the player's faction, 0 for the Horde, 1 for
/// the Alliance (recorded 0 for a Horde character in a guild; unverified
/// outside one).
fn get_guild_faction_group(vm: &mut Vm, _c: NativeCall) -> Ret {
    let f = vm.get_global("UnitFactionGroup");
    let player = vm.str("player");
    let faction = match vm.call(f, &[player])?.first().copied() {
        Some(v) => text(vm, v).unwrap_or_default(),
        None => String::new(),
    };
    Ok(vec![Value::Num(if faction == "Alliance" {
        1.0
    } else {
        0.0
    })])
}

/// `GetGuildCharterCost()`: 0 away from a guild registrar (recorded;
/// unverified: the price at one, warcraft.wiki.gg's 10 silver).
fn get_guild_charter_cost(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

// ---------------------------------------------------------------- Battle.net

/// `BNGetInfo()`: presence ID (nil), BattleTag, toon ID 1, broadcast,
/// away, busy, Real ID enabled, and an eighth false (recorded, the strings
/// scrubbed: the BattleTag here the player's name and #0000, the
/// broadcast empty; unverified).
fn bn_get_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    let tag = format!("{}#0000", engine(vm).units.list[units::PLAYER.0].name);
    Ok(vec![
        Value::Nil,
        vm.str(&tag),
        Value::Num(1.0),
        vm.str(""),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(true),
        Value::Bool(false),
    ])
}

/// `C_AccountInfo.IsGUIDBattleNetAccountType(guid)`: a `BNetAccount-`
/// GUID (by its form; unverified).
fn is_guid_bnet_account(vm: &mut Vm, c: NativeCall) -> Ret {
    let guid = text(vm, c.arg(1)).unwrap_or_default();
    Ok(vec![Value::Bool(guid.starts_with("BNetAccount-"))])
}

/// `C_AccountInfo.IsGUIDRelatedToLocalAccount(guid)`: the player's own
/// character (unverified: the account's other characters are unknown).
fn is_guid_local_account(vm: &mut Vm, c: NativeCall) -> Ret {
    let guid = text(vm, c.arg(1)).unwrap_or_default();
    let me = guid == engine(vm).units.list[units::PLAYER.0].guid;
    Ok(vec![Value::Bool(me)])
}

// ---------------------------------------------------------------- chat

/// `C_ChatInfo.IsPartyChannelType(channelType)`: the private and public
/// party channels (`Enum.ChatChannelType` 2 and 3; recorded false for
/// Custom; the others by their names, unverified).
fn is_party_channel_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let party = matches!(vm.hidden(c.arg(1)), Value::Num(n) if n == 2.0 || n == 3.0);
    Ok(vec![Value::Bool(party)])
}

/// `C_ChatInfo.GetChannelShortcut(channelIndex)`: the joined channel's
/// shortcut (its name when it has none), "" for none (unverified).
fn get_channel_shortcut(vm: &mut Vm, c: NativeCall) -> Ret {
    let index = match vm.hidden(c.arg(1)) {
        Value::Num(n) if n >= 1.0 => n as usize,
        _ => 0,
    };
    // The channel's place in the list, not its number (the full probe,
    // 1.60.1: 1 answered a shortcut with number 1 free, LookingForGroup
    // left; which channel's unverified, the answer scrubbed).
    let joined = &engine(vm).world.character.channels.joined;
    let shortcut = index
        .checked_sub(1)
        .and_then(|i| joined.get(i))
        .map(|ch| ch.shortcut.clone().unwrap_or_else(|| ch.name.clone()));
    // No channel there: nil (the full probe, 1.60.1: 0 and 2147483647).
    Ok(vec![shortcut.map_or(Value::Nil, |s| vm.str(&s))])
}

/// `C_ChatInfo.GetNumActiveChannels()`: the channels joined.
fn get_num_active_channels(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = engine(vm).world.character.channels.joined.len();
    Ok(vec![Value::Num(n as f64)])
}

/// `C_ChatInfo.SwapChatChannelsByChannelIndex(first, second)`: the two
/// joined channels change numbers (`CHANNEL_UI_UPDATE`; unverified: the
/// client's events).
fn swap_chat_channels(vm: &mut Vm, c: NativeCall) -> Ret {
    let number = |v: Value| match v {
        Value::Num(n) if n >= 1.0 => Some(n as usize),
        _ => None,
    };
    let (Some(a), Some(b)) = (number(vm.hidden(c.arg(1))), number(vm.hidden(c.arg(2)))) else {
        return Ok(vec![]);
    };
    let joined = &mut crate::engine_mut(vm).world.character.channels.joined;
    let place = |n: usize| joined.iter().position(|ch| ch.number == n);
    if let (Some(i), Some(j)) = (place(a), place(b))
        && i != j
    {
        joined.swap(i, j);
        joined[i].number = a;
        joined[j].number = b;
        crate::widgets::fire_event(vm, "CHANNEL_UI_UPDATE", &[]);
    }
    Ok(vec![])
}

/// `C_ChatInfo.ReplaceIconAndGroupExpressions(input [, noIconReplacement,
/// noGroupReplacement])`: raid target expressions (`{rt1}` to `{rt8}`, and
/// their English names) as the target icons' textures; group expressions
/// (`{g1}`) stay (unverified: the client's markup, its localized names).
fn replace_icon_expressions(vm: &mut Vm, c: NativeCall) -> Ret {
    let input = text(vm, c.arg(1)).unwrap_or_default();
    if c.arg(2).truthy() || !input.contains('{') {
        return Ok(vec![vm.str(&input)]);
    }
    const NAMES: [&[&str]; 8] = [
        &["rt1", "star"],
        &["rt2", "circle", "coin"],
        &["rt3", "diamond"],
        &["rt4", "triangle"],
        &["rt5", "moon"],
        &["rt6", "square"],
        &["rt7", "cross", "x"],
        &["rt8", "skull"],
    ];
    let mut out = String::with_capacity(input.len());
    let mut rest = input.as_str();
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let word = after[..close].to_ascii_lowercase();
        match NAMES
            .iter()
            .position(|names| names.contains(&word.as_str()))
        {
            Some(i) => out.push_str(&format!(
                "|TInterface\\TargetingFrame\\UI-RaidTargetingIcon_{}:0|t",
                i + 1
            )),
            None => out.push_str(&rest[open..open + close + 2]),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(vec![vm.str(&out)])
}

// ---------------------------------------------------------------- voice

macro_rules! voice_cvar_numbers {
    ($($name:ident: $cvar:literal, $default:literal;)*) => {
        $(
            fn $name(vm: &mut Vm, _c: NativeCall) -> Ret {
                Ok(vec![Value::Num(cvar_number(vm, $cvar, $default))])
            }
        )*
    };
}

// The recorded CVar defaults.
voice_cvar_numbers! {
    voice_output_volume: "VoiceOutputVolume", 50.0;
    voice_input_volume: "VoiceInputVolume", 50.0;
    voice_vad_sensitivity: "VoiceVADSensitivity", 43.0;
    voice_master_volume_scale: "VoiceChatMasterVolumeScale", 1.0;
    voice_communication_mode: "VoiceCommunicationMode", 0.0;
}

macro_rules! voice_cvar_setters {
    ($($name:ident: $cvar:literal;)*) => {
        $(
            fn $name(vm: &mut Vm, c: NativeCall) -> Ret {
                set_cvar_text(vm, $cvar, c.arg(1));
                Ok(vec![])
            }
        )*
    };
}

voice_cvar_setters! {
    set_voice_output_volume: "VoiceOutputVolume";
    set_voice_input_volume: "VoiceInputVolume";
    set_voice_vad_sensitivity: "VoiceVADSensitivity";
    set_voice_master_volume_scale: "VoiceChatMasterVolumeScale";
    set_voice_communication_mode: "VoiceCommunicationMode";
    set_voice_deafened: "VoiceSelfDeafened";
    set_voice_muted: "VoiceSelfMuted";
    set_voice_input_device: "VoiceInputDevice";
    set_voice_output_device: "VoiceOutputDevice";
}

/// `C_VoiceChat.IsDeafened()`: the `VoiceSelfDeafened` CVar (recorded
/// "0").
fn voice_is_deafened(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(cvar_flag(vm, "VoiceSelfDeafened"))])
}

fn voice_toggle_deafened(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = !cvar_flag(vm, "VoiceSelfDeafened");
    set_cvar_text(vm, "VoiceSelfDeafened", Value::Bool(on));
    Ok(vec![])
}

fn voice_toggle_muted(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = !cvar_flag(vm, "VoiceSelfMuted");
    set_cvar_text(vm, "VoiceSelfMuted", Value::Bool(on));
    Ok(vec![])
}

/// `C_VoiceChat.CreateChannel(name)`: refused, not logged in
/// (`Enum.VoiceChatStatusCode.ClientNotLoggedIn`; unverified).
fn voice_not_logged_in(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(5.0)])
}

// ---------------------------------------------------------------- clubs

/// `C_Club.ValidateText(clubType, text, fieldType)`: `NoName` for an
/// empty text, else `Success` (unverified: the client's length and
/// character rules).
fn club_validate_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = text(vm, c.arg(2)).unwrap_or_default();
    Ok(vec![Value::Num(if t.trim().is_empty() {
        2.0
    } else {
        0.0
    })])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    let mut out: Vec<(&'static str, NativeFn)> = vec![
        ("Ambiguate", ambiguate),
        // ------------------------------------------------ guild
        ("BNGetDisplayName", bn_get_display_name),
        ("GuildControlGetRank", guild_control_get_rank),
        ("GuildControlSetRank", guild_control_set_rank),
        ("GetGuildFactionGroup", get_guild_faction_group),
        ("GetGuildCharterCost", get_guild_charter_cost),
        // ------------------------------------------------ Battle.net
        ("BNGetInfo", bn_get_info),
        (
            "C_AccountInfo.IsGUIDBattleNetAccountType",
            is_guid_bnet_account,
        ),
        (
            "C_AccountInfo.IsGUIDRelatedToLocalAccount",
            is_guid_local_account,
        ),
        // ------------------------------------------------ chat
        ("C_ChatInfo.IsPartyChannelType", is_party_channel_type),
        ("C_ChatInfo.GetChannelShortcut", get_channel_shortcut),
        // By the channel's local ID, as by its index (unverified: the full
        // probe's 1 answered with number 1 free, as the index).
        (
            "C_ChatInfo.GetChannelShortcutForChannelID",
            get_channel_shortcut,
        ),
        ("C_ChatInfo.GetNumActiveChannels", get_num_active_channels),
        (
            "C_ChatInfo.SwapChatChannelsByChannelIndex",
            swap_chat_channels,
        ),
        (
            "C_ChatInfo.ReplaceIconAndGroupExpressions",
            replace_icon_expressions,
        ),
        // ------------------------------------------------ voice
        ("C_VoiceChat.GetOutputVolume", voice_output_volume),
        ("C_VoiceChat.GetInputVolume", voice_input_volume),
        ("C_VoiceChat.GetVADSensitivity", voice_vad_sensitivity),
        (
            "C_VoiceChat.GetMasterVolumeScale",
            voice_master_volume_scale,
        ),
        ("C_VoiceChat.GetCommunicationMode", voice_communication_mode),
        ("C_VoiceChat.SetOutputVolume", set_voice_output_volume),
        ("C_VoiceChat.SetInputVolume", set_voice_input_volume),
        ("C_VoiceChat.SetVADSensitivity", set_voice_vad_sensitivity),
        (
            "C_VoiceChat.SetMasterVolumeScale",
            set_voice_master_volume_scale,
        ),
        (
            "C_VoiceChat.SetCommunicationMode",
            set_voice_communication_mode,
        ),
        ("C_VoiceChat.SetDeafened", set_voice_deafened),
        ("C_VoiceChat.SetMuted", set_voice_muted),
        ("C_VoiceChat.SetInputDevice", set_voice_input_device),
        ("C_VoiceChat.SetOutputDevice", set_voice_output_device),
        ("C_VoiceChat.IsDeafened", voice_is_deafened),
        ("C_VoiceChat.ToggleDeafened", voice_toggle_deafened),
        ("C_VoiceChat.ToggleMuted", voice_toggle_muted),
        ("C_VoiceChat.CreateChannel", voice_not_logged_in),
        // ------------------------------------------------ clubs
        ("C_Club.ValidateText", club_validate_text),
    ];
    // Recorded (`/stdtest engine answers`).
    let recorded: &[(&str, NativeFn)] = &[
        ("BNGetNumFriendInvites", zero),
        ("C_BattleNet.IsBattleNetFriendsListEnabled", yes),
        // SuuUI probe friendSystem and the answers battery: title friends and
        // their custom names on.
        ("C_BattleNet.AreTitleFriendsEnabled", yes),
        ("C_BattleNet.AreTitleFriendCustomNamesEnabled", yes),
        ("C_ChatInfo.CancelEmote", nothing),
        // Recorded nil even in a guild.
        ("C_GuildInfo.GetGuildTabardInfo", nil),
        ("GetGuildLogoInfo", nothing),
        ("GetGuildRenameRequired", no),
        ("GetNumGuildPerks", zero),
        ("C_RecentAllies.IsRecentAllyDataReady", yes),
        ("C_VoiceChat.GetChannelForChannelType", nothing),
        ("C_VoiceChat.IsChannelJoinPending", no),
        ("C_VoiceChat.IsVoiceChatConnected", yes),
    ];
    // Battle.net is connected (recorded `BNFeaturesEnabled`, `BNGetInfo`;
    // unverified: the connection itself); Real ID on, as `BNGetInfo`'s.
    let connected: &[&str] = &[
        "BNConnected",
        "BNFeaturesEnabledAndConnected",
        "C_BattleNet.AreRealIDFriendsEnabled",
        // Unverified: voice chat enabled (recorded `IsEnabled`), the player
        // free to use it and its settings.
        "C_VoiceChat.CanPlayerUseVoiceChat",
        "C_VoiceChat.CanAccessSettings",
        // Unverified: nothing keeps the player from chat.
        "C_SocialRestrictions.CanSendChat",
        // Unverified: an emote is performed.
        "C_ChatInfo.PerformEmote",
        // Unverified: a custom message is set.
        "C_BattleNet.SetCustomMessage",
        // Unverified: an ignore is accepted (the server's list answers
        // later; the engine keeps none).
        "C_FriendList.AddIgnore",
    ];
    // Lists with nothing in them: no friends, ignores, Battle.net
    // friends, clubs, members, messages, tickets, applicants, allies,
    // voice voices or processes, chat bubbles (unverified).
    let lists: &[&str] = &[
        "C_Club.GetClubStreamNotificationSettings",
        "C_Club.GetMessageRanges",
        "C_Club.GetMessagesBefore",
        "C_Club.GetMessagesInRange",
        "C_Club.GetAssignableRoles",
        "C_Club.GetInvitationCandidates",
        "C_Club.GetInvitationsForClub",
        "C_Club.GetTickets",
        "C_ClubFinder.ReturnClubApplicantList",
        "C_ClubFinder.ReturnPendingClubApplicantList",
        "C_ClubFinder.GetStatusOfPostingFromClubId",
        "C_ClubFinder.PlayerGetClubInvitationList",
        "C_ClubFinder.PlayerReturnPendingCommunitiesList",
        "C_ClubFinder.PlayerReturnPendingGuildsList",
        "C_ClubFinder.ReturnMatchingCommunityList",
        "C_ClubFinder.ReturnMatchingGuildList",
        "C_ChatInfo.GetClubStreamIDs",
        "C_BattleNet.SearchFriends",
        "C_RecentAllies.GetRecentAllies",
        "C_RecentAllies.SearchRecentAllies",
        "C_VoiceChat.GetRemoteTtsVoices",
        "C_VoiceChat.GetProcesses",
        "C_ChatBubbles.GetAllChatBubbles",
    ];
    // Counts of nothing, amounts of none (unverified; a guild bank, its
    // money and log, news, perks, rewards, challenges, events, recipes
    // the engine does not have).
    let zeros: &[&str] = &[
        "C_FriendList.GetNumFriends",
        "C_FriendList.GetNumOnlineFriends",
        "C_FriendList.GetNumIgnores",
        "C_BattleNet.GetFriendNumGameAccounts",
        "BNGetNumBlocked",
        "BNGetNumFOF",
        // Nothing selected (Blizzard's ignore list uses it as an index).
        "BNGetSelectedBlock",
        "BNGetSelectedFriend",
        "C_Club.CompareBattleNetDisplayName",
        "C_ClubFinder.GetTotalMatchingCommunityListSize",
        "C_ClubFinder.GetTotalMatchingGuildListSize",
        "C_ClubFinder.GetPlayerSettingsFocusFlagsSelectedCount",
        "C_AccountInfo.GetIDFromBattleNetAccountGUID",
        // `Enum.ChatChannelRuleset.None` (recorded for every
        // `GetChannelRuleset`).
        "C_ChatInfo.GetChannelRulesetForChannelID",
        // `Enum.ClubRestrictionReason.None`.
        "C_Club.IsRestricted",
        // `Enum.PlayerClubRequestStatus.None`.
        "C_ClubFinder.GetPlayerClubApplicationStatus",
        "GetGuildBankWithdrawGoldLimit",
        "GetGuildBankBonusDepositMoney",
        "GetNumGuildBankMoneyTransactions",
        "GetNumGuildBankTransactions",
        "GetNumGuildChallenges",
        "GetNumGuildEvents",
        "GetNumGuildNews",
        "GetNumGuildRewards",
        "GetGuildAchievementNumMembers",
        "GetGuildNewsSort",
        "GetGuildRosterLargestAchievementPoints",
        "GetGuildRosterSelection",
    ];
    // Unknown IDs, no one selected, nothing found: nil (unverified).
    let nils: &[&str] = &[
        "C_Club.GetMessageInfo",
        "C_Club.GetInvitationInfo",
        "C_Club.GetStreamViewMarker",
        "C_Club.GetCommunityNameResultText",
        "C_BattleNet.GetFriendAccountInfo",
        "C_BattleNet.GetFriendGameAccountInfo",
        "C_BattleNet.GetFriendInviteInfo",
        "C_BattleNet.GetAccountInfoByID",
        "C_BattleNet.GetAccountInfoByGUID",
        "C_BattleNet.GetGameAccountInfoByGUID",
        "C_BattleNet.GetGameAccountInfoByID",
        "C_ClubFinder.GetRecruitingClubInfoFromClubID",
        "C_ClubFinder.GetPostingIDFromClubFinderGUID",
        "C_FriendList.GetSelectedIgnore",
        "C_FriendList.GetSelectedFriend",
        "C_FriendList.GetIgnoreName",
        "C_RecentAllies.GetRecentAllyByFullName",
        "C_RecentAllies.GetRecentAllyByGUID",
        "C_VoiceChat.GetLocalPlayerMemberID",
        "C_VoiceChat.GetMemberID",
        "C_VoiceChat.GetMemberInfo",
        "C_VoiceChat.GetMemberName",
        "C_VoiceChat.GetJoinClubVoiceChannelError",
        "C_VoiceChat.GetLocalPlayerActiveChannelMemberInfo",
        // Not logged in: no devices, no status.
        "C_VoiceChat.GetAvailableInputDevices",
        "C_VoiceChat.GetAvailableOutputDevices",
        "C_VoiceChat.GetCurrentVoiceChatConnectionStatusCode",
        "C_VoiceChat.GetMemberVolume",
        "GetGuildBankItemLink",
        "GetGuildNewsMemberName",
        "BNGetFriendIndex",
        "BNTokenFindName",
    ];
    // Not so (unverified unless said).
    let falses: &[&str] = &[
        "C_FriendList.IsFriend",
        "C_FriendList.IsIgnored",
        "C_FriendList.IsIgnoredByGuid",
        "C_FriendList.IsOnIgnoredList",
        // Not on the list: nothing removed, no notes set.
        "C_FriendList.DelIgnore",
        "C_FriendList.RemoveFriend",
        "C_FriendList.SetFriendNotes",
        "BNIsFriend",
        "BNIsBlocked",
        "BNIsSelf",
        "C_BattleNet.SendWhisper",
        "C_ClubFinder.DoesPlayerBelongToClubFromClubGUID",
        "C_ClubFinder.IsPostingBanned",
        "C_ClubFinder.HasAlreadyAppliedToLinkedPosting",
        "C_ClubFinder.HasPostingBeenDelisted",
        "C_ClubFinder.RequestPostingInformationFromClubId",
        "C_ClubFinder.PostClub",
        // The club finder is off (recorded `IsCommunityFinderEnabled`,
        // `IsEnabled` false).
        "C_ClubFinder.ShouldShowClubFinder",
        "C_Club.IsAccountMuted",
        "C_Club.IsSubscribedToStream",
        "C_Club.RequestMoreMessagesBefore",
        "C_Club.CanResolvePlayerLocationFromClubMessageData",
        "C_Club.DoesAnyCommunityHaveUnreadMessages",
        "C_Club.DoesCommunityHaveMembersOfTheOppositeFaction",
        "C_Club.FocusStream",
        "C_Club.IsBeginningOfStream",
        // Classic has no guild events, news, reputation (recorded
        // `C_Reputation.GetGuildFactionData` nil).
        "C_GuildInfo.AreGuildEventsEnabled",
        "C_GuildInfo.IsEncounterGuildNewsEnabled",
        "C_GuildInfo.IsGuildReputationEnabled",
        "C_GuildInfo.IsDiscordStreamSeparate",
        "C_GuildInfo.RequestRenameStatus",
        "C_GuildInfo.SetPreferredPlaySettings",
        "C_ChatInfo.IsChatLineCensored",
        "C_ChatInfo.IsTimerunningPlayer",
        "C_ChatInfo.IsLoggingChat",
        "C_RecentAllies.IsRecentAllyByFullName",
        "C_RecentAllies.IsRecentAllyByGUID",
        "C_RecentAllies.IsRecentAllyPinned",
        "C_RecentAllies.CanSetRecentAllyNote",
        "C_SocialQueue.IsSystemEnabled",
        "C_SocialQueue.RequestToJoin",
        "C_SocialRestrictions.IsAgeVerificationRestrictedMinor",
        "C_SocialRestrictions.IsSilenced",
        "C_SocialRestrictions.IsSquelched",
        "C_VoiceChat.IsMemberLocalPlayer",
        "C_VoiceChat.IsMemberMuted",
        "C_VoiceChat.IsMemberMutedForAll",
        "C_VoiceChat.IsMemberSilenced",
        "C_VoiceChat.IsParentalDisabled",
        "C_VoiceChat.IsParentalMuted",
        "C_VoiceChat.IsPlayerUsingVoice",
        "C_VoiceChat.IsSilenced",
        "C_VoiceChat.IsTranscribing",
        "C_VoiceChat.ShouldDiscoverChannels",
        "C_VoiceChat.GetPTTButtonPressedState",
        "C_VoiceChat.RestoreActiveVoiceProvider",
        "IsDisplayChannelOwner",
        "IsDisplayChannelModerator",
        "IsInGuildGroup",
        "GetGuildRosterShowOffline",
    ];
    // Empty texts.
    let texts: &[&str] = &["C_VoiceChat.GetMemberGUID"];
    // The documentation's empty structures and pairs: no privileges, no
    // limits, a posting not found, play settings not loaded.
    let documented: &[&str] = &[
        "C_Club.GetClubPrivileges",
        "C_Club.GetClubLimits",
        "C_ClubFinder.GetRecruitingClubInfoFromFinderGUID",
        "C_GuildInfo.GetPreferredPlaySettings",
    ];
    // Two of nothing.
    let pairs: &[(&str, NativeFn)] = &[
        ("C_FriendList.GetNumWhoResults", zeros2),
        ("C_ChatInfo.IsLoggingCombat", no2),
        ("GuildControlGetAllowedShifts", no2),
    ];
    // Nothing returned: nothing found (functions that may return nothing,
    // undocumented ones for something unknown), or an action accepted that
    // changes nothing the engine keeps (unverified: the client's server
    // round trips and their events).
    let none: &[&str] = &[
        // Found nothing.
        "C_FriendList.GetFriendInfoByIndex",
        "C_FriendList.GetFriendInfo",
        "C_FriendList.GetWhoInfo",
        "C_VoiceChat.GetChannel",
        "C_VoiceChat.GetChannelForCommunityStream",
        "C_SocialQueue.GetGroupQueues",
        "C_SocialQueue.GetGroupForPlayer",
        "C_SocialQueue.GetGroupInfo",
        "C_SocialQueue.GetGroupMembers",
        "C_ChatInfo.GetChannelRosterInfo",
        "C_ClubFinder.GetClubTypeFromFinderGUID",
        "C_Club.GetInfoFromLastCommunityChatLine",
        "C_Club.GetLastTicketResponse",
        "C_GuildInfo.GetGuildNewsInfo",
        "C_GuildInfo.QueryGuildMembersForRecipe",
        "C_BattleNet.GetCustomTitleFriendName",
        "GetGuildBankItemInfo",
        "GetGuildBankTabInfo",
        "GetGuildRewardInfo",
        "GetGuildAchievementMemberInfo",
        "GetGuildAchievementMembers",
        "GetGuildRecipeInfoPostQuery",
        "GetGuildBankMoneyTransaction",
        "GetGuildBankTabPermissions",
        "GetGuildBankText",
        "GetGuildBankTransaction",
        "GetGuildChallengeInfo",
        "GetGuildEventInfo",
        "GetGuildPerkInfo",
        "GetGuildRecipeMember",
        "GetGuildTabardFiles",
        "GetGuildMemberRecipes",
        "BNGetBlockedInfo",
        "BNGetFOFInfo",
        // Guild actions.
        "C_GuildInfo.Invite",
        "C_GuildInfo.RequestGuildRename",
        "C_GuildInfo.RequestGuildRenameRefund",
        "C_GuildInfo.RequestRenameNameCheck",
        "C_GuildInfo.SetGuildRankOrder",
        "C_GuildInfo.QueryGuildMemberRecipes",
        "GuildInfo",
        "GuildControlSetRankFlag",
        "GuildControlSaveRank",
        "GuildControlAddRank",
        "GuildControlDelRank",
        "GuildControlShiftRankDown",
        "GuildControlShiftRankUp",
        "GuildNewsSetSticky",
        "GuildNewsSort",
        "GuildInvite",
        "GuildMasterAbsent",
        // Friends, ignores, who.
        "C_FriendList.AddFriend",
        "C_FriendList.AddOrRemoveFriend",
        "C_FriendList.AddOrDelIgnore",
        "C_FriendList.DelIgnoreByIndex",
        "C_FriendList.SetSelectedIgnore",
        "C_FriendList.RemoveFriendByIndex",
        "C_FriendList.SetSelectedFriend",
        "C_FriendList.SetFriendNotesByIndex",
        "C_FriendList.ShowFriends",
        "C_FriendList.SendWho",
        "C_FriendList.SortWho",
        "C_FriendList.SetWhoToUi",
        // Battle.net.
        "BNRequestInviteFriend",
        "BNSetFriendFavoriteFlag",
        "BNDeclineFriendInvite",
        "BNAcceptFriendInvite",
        "BNSendFriendInvite",
        "BNSendFriendInviteByID",
        "BNSetSelectedBlock",
        "BNSummonFriendByIndex",
        "BNRemoveFriend",
        "BNSetFriendNote",
        "BNSetSelectedFriend",
        "BNCheckBattleTagInviteToGuildMember",
        "BNCheckBattleTagInviteToUnit",
        "BNRequestFOFInfo",
        "C_BattleNet.SetBlocked",
        "C_BattleNet.SetAFK",
        "C_BattleNet.SetDND",
        "C_BattleNet.InviteFriend",
        "C_BattleNet.SendTitleFriendInviteByName",
        "C_BattleNet.SetAppearOffline",
        "C_BattleNet.SetCustomTitleFriendName",
        "C_BattleNet.BNCheckBattleTagInviteToRecentAlly",
        "C_BattleNet.BNCheckTitleFriendInviteToUnit",
        "C_BattleNet.InstallHighResTextures",
        "C_BattleNet.SendVerifiedBattleNetFriendInvite",
        "C_BattleNet.SetFriendTags",
        // Chat and channels (unverified: the client's chat lines, its
        // channel moderation; leaving and joining keep the engine's
        // numbering, which has no gaps, as it is).
        "C_ChatInfo.SendChatMessage",
        "C_ChatInfo.RequestCanLocalWhisperTarget",
        "C_ChatInfo.DropCautionaryChatMessage",
        "C_ChatInfo.SendCautionaryChatMessage",
        "C_ChatInfo.ResetDefaultZoneChannels",
        "C_ChatInfo.UncensorChatLine",
        "ChannelInvite",
        "ChannelModerator",
        "ChannelUnmoderator",
        "ChannelBan",
        "ChannelKick",
        "ChannelUnban",
        "SetChannelOwner",
        "SetChannelPassword",
        "DisplayChannelOwner",
        "LeaveChannelByName",
        "LeaveChannelByLocalID",
        "ListChannelByName",
        "ListChannels",
        "AddChatWindowChannel",
        "RemoveChatWindowChannel",
        "AddChatWindowMessages",
        "RemoveChatWindowMessages",
        // Clubs and the club finder.
        "C_Club.SetAvatarTexture",
        "C_Club.ClearClubPresenceSubscription",
        "C_Club.SetClubPresenceSubscription",
        "C_Club.FocusMembers",
        "C_Club.UnfocusMembers",
        "C_Club.AdvanceStreamViewMarker",
        "C_Club.DestroyClub",
        "C_Club.RedeemTicket",
        "C_Club.SendBattleTagFriendRequest",
        "C_Club.SetClubMemberNote",
        "C_Club.AcceptInvitation",
        "C_Club.AddClubStreamChatChannel",
        "C_Club.AssignMemberRole",
        "C_Club.ClearAutoAdvanceStreamViewMarker",
        "C_Club.CreateClub",
        "C_Club.CreateStream",
        "C_Club.CreateTicket",
        "C_Club.DeclineInvitation",
        "C_Club.DestroyMessage",
        "C_Club.DestroyStream",
        "C_Club.DestroyTicket",
        "C_Club.EditClub",
        "C_Club.EditMessage",
        "C_Club.EditStream",
        "C_Club.Flush",
        "C_Club.FocusCommunityStreams",
        "C_Club.KickMember",
        "C_Club.LeaveClub",
        "C_Club.RequestInvitationsForClub",
        "C_Club.RequestTicket",
        "C_Club.RequestTickets",
        "C_Club.RevokeInvitation",
        "C_Club.SendCharacterInvitation",
        "C_Club.SendInvitation",
        "C_Club.SendMessage",
        "C_Club.SendTitleFriendRequest",
        "C_Club.SetAutoAdvanceStreamViewMarker",
        "C_Club.SetClubStreamNotificationSettings",
        "C_Club.SetFavorite",
        "C_Club.SetSocialQueueingEnabled",
        "C_Club.UnfocusAllStreams",
        "C_Club.UnfocusStream",
        "C_Club.SetCommunityID",
        "C_ClubFinder.SetRecruitmentSettings",
        "C_ClubFinder.PlayerRequestPendingClubsList",
        "C_ClubFinder.SetPlayerApplicantSettings",
        "C_ClubFinder.RequestApplicantList",
        "C_ClubFinder.ResetClubPostingMapCache",
        "C_ClubFinder.SetAllRecruitmentSettings",
        "C_ClubFinder.SetPlayerApplicantLocaleFlags",
        "C_ClubFinder.ApplicantAcceptClubInvite",
        "C_ClubFinder.ApplicantDeclineClubInvite",
        "C_ClubFinder.CancelMembershipRequest",
        "C_ClubFinder.LookupClubPostingFromClubFinderGUID",
        "C_ClubFinder.RequestClubsList",
        "C_ClubFinder.RequestMembershipToClub",
        "C_ClubFinder.RequestNextCommunityPage",
        "C_ClubFinder.RequestNextGuildPage",
        "C_ClubFinder.RequestSubscribedClubPostingIDs",
        "C_ClubFinder.RespondToApplicant",
        "C_ClubFinder.SendChatWhisper",
        "C_ClubFinder.SetRecruitmentLocale",
        "C_ClubFinder.CheckAllPlayerApplicantSettings",
        "C_ClubFinder.ClearAllFinderCache",
        "C_ClubFinder.ClearClubApplicantsCache",
        "C_ClubFinder.ClearClubFinderPostingsCache",
        // Voice chat (not logged in, no channels).
        "C_VoiceChat.ActivateChannel",
        "C_VoiceChat.DeactivateChannel",
        "C_VoiceChat.LeaveChannel",
        "C_VoiceChat.RequestJoinChannelByChannelType",
        "C_VoiceChat.RequestJoinAndActivateCommunityStreamChannel",
        "C_VoiceChat.ActivateChannelTranscription",
        "C_VoiceChat.DeactivateChannelTranscription",
        "C_VoiceChat.MarkChannelsDiscovered",
        "C_VoiceChat.SetPortraitTexture",
        "C_VoiceChat.ToggleMemberMuted",
        "C_VoiceChat.SetMemberMuted",
        "C_VoiceChat.SetMemberVolume",
        "C_VoiceChat.StopSpeakingText",
        "C_VoiceChat.SpeakText",
        "C_VoiceChat.SpeakRemoteTextSample",
        "C_VoiceChat.BeginLocalCapture",
        "C_VoiceChat.EndLocalCapture",
        "C_VoiceChat.SetPushToTalkBinding",
        // Social queues, restrictions, recent allies, reports.
        "C_SocialQueue.SignalToastDisplayed",
        "C_SocialRestrictions.AcknowledgeAgeVerificationRestriction",
        "C_SocialRestrictions.AcknowledgeRegionalChatDisabled",
        "C_SocialRestrictions.SetChatDisabled",
        "C_RecentAllies.SetRecentAllyNote",
        "C_RecentAllies.TryRequestRecentAlliesData",
        "C_RecentAllies.SetRecentAllyPinned",
        "C_ReportSystem.SendReport",
        "C_ReportSystem.SetScreenshotPreviewTexture",
        "C_ReportSystem.TakeReportScreenshot",
        "C_ReportSystem.ReportServerLag",
        "C_ReportSystem.ReportStuckInCombat",
    ];
    out.extend(recorded.iter().copied());
    out.extend(pairs.iter().copied());
    let groups: [(&[&str], NativeFn); 8] = [
        (connected, yes),
        (lists, empty_list),
        (zeros, zero),
        (nils, nil),
        (falses, no),
        (texts, empty_string),
        (documented, documented_empty),
        (none, nothing),
    ];
    for (names, f) in groups {
        out.extend(names.iter().map(|&n| (n, f)));
    }
    out
}

#[cfg(test)]
mod tests {
    /// No function has two behaviours here.
    #[test]
    fn each_function_once() {
        let mut names: Vec<&str> = super::behaviours().iter().map(|(n, _)| *n).collect();
        let all = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all);
    }
}
