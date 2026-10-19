//! The second round's leftovers: functions no recording, model or earlier
//! fill-in answered. The action bars' page numbers; the channels a player
//! joins by hand; the client's own settings and network (IP and protocol
//! types, downloads, the console); combat sums the paper doll and skills
//! frame show (effective attack power, glancing blows, resistances); the
//! chat types' names and colors; quests' trivial range and daily reset;
//! and the "nothing here" answers of systems Forever lacks or the engine
//! does not model (Battle.net, clubs, voice chat, reports, raid profiles,
//! GM tickets, achievements comparison, arena).
//!
//! A documented function of those systems answers its documentation's
//! empty values (`crate::defaults`); an undocumented one answers by its
//! kind (`UNDOCUMENTED`).
//!
//! Unverified unless a comment says otherwise: Forever's own answers to
//! these were not recorded.

use std::rc::Rc;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

// ------------------------------------------------------------- helpers

/// What the engine answers for the client function `name` called with
/// `args` (its behaviour; none: nothing), secret answers unwrapped (as
/// `extras::ask`).
fn ask(vm: &mut Vm, name: &'static str, args: &[Value]) -> Vec<Value> {
    let Some(f) = engine(vm).behaviour(name) else {
        return vec![];
    };
    let full = vm.str(name);
    let call = NativeCall {
        args: args.iter().copied().collect(),
        taints: args.iter().map(|_| Default::default()).collect(),
        origins: args.iter().map(|_| Default::default()).collect(),
        upvals: Rc::from([full]),
        upval_taints: Rc::from([Default::default()]),
    };
    match f(vm, call) {
        Ok(values) => values.into_iter().map(|v| vm.hidden(v)).collect(),
        Err(_) => vec![],
    }
}

/// The engine's first answer to `name` as a number.
fn ask_num(vm: &mut Vm, name: &'static str, args: &[Value]) -> Option<f64> {
    match ask(vm, name, args).first() {
        Some(Value::Num(n)) => Some(*n),
        _ => None,
    }
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// An argument as a number (a secret one unwrapped).
fn arg_num(vm: &Vm, c: &NativeCall, n: usize) -> Option<f64> {
    num(vm.hidden(c.arg(n)))
}

/// An argument as text (a secret one unwrapped).
fn arg_text(vm: &Vm, c: &NativeCall, n: usize) -> Option<String> {
    match vm.hidden(c.arg(n)) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        Value::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => Some(format!("{}", n as i64)),
        Value::Num(n) => Some(format!("{n}")),
        _ => None,
    }
}

fn field(vm: &mut Vm, t: TableRef, k: &str) -> Value {
    let key = vm.str(k);
    vm.heap.table(t).get(key)
}

fn set(vm: &mut Vm, t: TableRef, k: &str, v: Value) {
    let key = vm.str(k);
    let _ = vm.heap.table_mut(t).set(key, v);
}

/// A list of values as a Lua array.
fn list(vm: &mut Vm, values: &[Value]) -> Value {
    let t = vm.new_table();
    for (i, v) in values.iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num((i + 1) as f64), *v);
    }
    Value::Table(t)
}

/// This module's settings (the action bar toggles, the console's font
/// height): a table the engine keeps for the VM's life, among its compiled
/// helpers' values.
fn state(vm: &mut Vm) -> TableRef {
    if let Some(Value::Table(t)) = engine(vm).lua_helpers.get("leftovers2.state").copied() {
        return t;
    }
    let t = vm.new_table();
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm)
        .lua_helpers
        .insert("leftovers2.state", Value::Table(t));
    t
}

fn state_get(vm: &mut Vm, k: &str) -> Value {
    let t = state(vm);
    field(vm, t, k)
}

fn state_set(vm: &mut Vm, k: &str, v: Value) {
    let t = state(vm);
    set(vm, t, k, v);
}

/// A color as Blizzard's code takes it (`CreateColor`'s when its UI code
/// is loaded, else the plain fields), as `extras::color`.
fn color(vm: &mut Vm, (r, g, b): (f64, f64, f64)) -> Value {
    let create = vm.get_global("CreateColor");
    if matches!(create, Value::Func(_))
        && let Ok(v) = vm.call(
            create,
            &[Value::Num(r), Value::Num(g), Value::Num(b), Value::Num(1.0)],
        )
        && let Some(&c) = v.first()
    {
        return c;
    }
    let t = vm.new_table();
    for (k, v) in [("r", r), ("g", g), ("b", b), ("a", 1.0)] {
        set(vm, t, k, Value::Num(v));
    }
    Value::Table(t)
}

/// The player's level (60 when the engine does not say).
fn player_level(vm: &mut Vm) -> f64 {
    let player = vm.str("player");
    ask_num(vm, "UnitLevel", &[player]).unwrap_or(60.0)
}

// ---------------------------------------------------- "nothing here"

/// Undocumented functions of systems the engine does not model and their
/// answers by kind: `f` false, `t` true, `0` zero, `{` an empty table, `n`
/// nil, empty nothing (unverified: named as their kind).
const UNDOCUMENTED: &[(&str, &str)] = &[
    // A shared quest's acceptance, a loot roll's confirmation (a bind on
    // pickup's popup; `RollOnLoot` does nothing either), a tabard's
    // window, the help frame's web ticket, a GM survey.
    ("ConfirmAcceptQuest", ""),
    ("ConfirmLootRoll", ""),
    ("CloseTabardCreation", ""),
    ("GetTabardInfo", ""),
    ("GetWebTicket", ""),
    ("GetGMTicket", ""),
    ("GMSurveyAnswer", "n"),
    ("GetGMStatus", "f"),
    // No PvP talents, no totem bar spells kept, no talents taken back.
    ("PickupPvpTalent", ""),
    ("SetMultiCastSpell", ""),
    // The LFG dungeon teleport: not in an LFG dungeon.
    ("IsAllowedToUserTeleport", "f"),
    // No achievements compared, no statistics updated.
    ("GetComparisonCategoryNumAchievements", "0"),
    ("GetComparisonStatistic", "n"),
    ("GetLatestUpdatedStats", ""),
    ("GetLatestUpdatedComparisonStats", ""),
    // An inspected player's arena data and talents: none.
    ("GetInspectArenaData", "00000"),
    ("GetInspectTalent", "n"),
    // Guild news filters: none kept.
    ("GetGuildNewsFilters", "{"),
    // Quest POIs and sorting: none (the map's own `C_QuestLog` answers).
    ("GetQuestPOIs", ""),
    ("GetTaskPOIs", ""),
    ("GetQuestPOILeaderBoard", "n"),
    ("GetQuestSortIndex", "n"),
    // Raid profiles: none kept (`unitinfo`'s getters answer none), not
    // loaded.
    ("GetNumRaidProfiles", "0"),
    ("RaidProfileExists", "f"),
    ("RaidProfileHasUnsavedChanges", "f"),
    ("HasLoadedCUFProfiles", "f"),
    // The client's internals Blizzard's secure code calls: nothing to do.
    ("RegisterStaticConstants", ""),
    ("setsecurehookforbidden", ""),
    ("SetupFullscreenScale", ""),
    // Mouselook and movement switches: nothing moves in the engine.
    ("SetMouselookOverrideBinding", ""),
    ("SetMoveEnabled", ""),
    ("SetTurnEnabled", ""),
    // The talent tiers of the old talent frame: none (Forever's talents
    // are trait trees).
    ("GetMaxTalentTier", "0"),
    // Code runs in the global environment.
    ("IsInGlobalEnvironment", "t"),
];

/// A system the engine does not model: its documented empty values, or an
/// undocumented function's by its kind (`UNDOCUMENTED`).
fn nothing(vm: &mut Vm, c: NativeCall) -> Ret {
    let api = engine(vm).api;
    let name = match c.upvals.first().copied() {
        Some(Value::Num(i)) => {
            return Ok(match api.functions.get(i as usize) {
                Some(f) => crate::defaults(vm, api, &f.doc),
                None => vec![],
            });
        }
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => return Ok(vec![]),
    };
    if let Some(f) = api.functions.iter().find(|f| f.name == name) {
        return Ok(crate::defaults(vm, api, &f.doc));
    }
    let spec = UNDOCUMENTED
        .iter()
        .find(|(n, _)| *n == name)
        .map_or("", |(_, s)| *s);
    Ok(spec
        .chars()
        .map(|ch| match ch {
            'f' => Value::Bool(false),
            't' => Value::Bool(true),
            '0' => Value::Num(0.0),
            '{' => Value::Table(vm.new_table()),
            _ => Value::Nil,
        })
        .collect())
}

/// Documented functions of systems Forever lacks or the engine does not
/// model, answering their documentation's empty values (unverified).
const DOCUMENTED_EMPTY: &[&str] = &[
    // The club finder: no clubs. (`C_Club.GetAvatarIdList` answers as
    // recorded, `data/camelot-answers.txt`.)
    "C_ClubFinder.GetFocusIndexFromFlag",
    "C_ClubFinder.IsListingEnabledFromFlags",
    // Battle.net's optional features: off; no game data sent.
    "C_BattleNet.AreHighResTexturesInstalled",
    "C_BattleNet.AreFriendTagsEnabled",
    "C_BattleNet.CanToggleHighResTexturesWithoutClientReload",
    "C_BattleNet.GetFriendLevelFromInviteText",
    "C_BattleNet.SendGameData",
    // Voice chat: nothing to log into.
    "C_VoiceChat.Login",
    "C_VoiceChat.Logout",
    "C_VoiceChat.SetActiveVoiceProvider",
    // Reporting: nothing to report (the report frame's lists empty).
    "C_ReportSystem.CanReportPlayer",
    "C_ReportSystem.CanReportPlayerForLanguage",
    "C_ReportSystem.GetMajorCategoriesForReportType",
    "C_ReportSystem.GetMajorCategoryString",
    "C_ReportSystem.GetMinorCategoriesForReportTypeAndMajorCategory",
    "C_ReportSystem.GetMinorCategoryString",
    "C_ReportSystem.RequiresScreenshotForReportType",
    // Chat lines: none kept by their IDs; no regional or mentor channel.
    "C_ChatInfo.GetChatLineText",
    "C_ChatInfo.GetChatLineSenderGUID",
    "C_ChatInfo.GetChatLineSenderName",
    "C_ChatInfo.IsChannelRegionalForChannelID",
    "C_ChatInfo.GetMentorChannelID",
    // Loadout import strings: not read (an empty error).
    "C_ClassTalents.ImportLoadout",
    // No flyouts, no Torghast, no graveyard chosen, no sturdiness.
    "C_SpellBook.FindFlyoutSlotBySpellID",
    "IsJailersTowerLayerTimeLocked",
    "GetCemeteryPreference",
    "GetSturdiness",
    // The pet's model scene: Blizzard's paper doll falls back to its
    // default scene on 0.
    "C_PetInfo.GetPetUIModelSceneID",
    // A unit's cursor: none drawn.
    "SetUnitCursorTexture",
    // The world map is Blizzard's Lua; the profession's spells unknown.
    "C_Map.OpenWorldMap",
    "C_TradeSkillUI.GetProfessionSpells",
    // The script throttles: none.
    "GetScriptBucketThrottleLimits",
    // Input the engine does not simulate; the click-binding modifiers
    // none held as `MakeModifiers` would say without keys down.
    "SimulateMouseClick",
    "SimulateMouseDown",
    "SimulateMouseUp",
    "SimulateMouseWheel",
    "MakeModifiers",
    "GetStringFromModifiers",
    // The target priority highlight and a Battle.net outage: nothing to
    // show.
    "TargetPriorityHighlightStart",
    "TargetPriorityHighlightEnd",
    "OutageDetected",
    // The upgrade expansion: none (Forever's expansion level is recorded
    // 0).
    "GetUpgradeExpansionLevel",
    // An item's armor type preferred: Forever has no such rule.
    "IsItemPreferredArmorType",
];

// ------------------------------------------------------ the action bars

/// `NUM_ACTIONBAR_PAGES` (Blizzard's ActionButtonUtil.lua): the bonus bars'
/// pages follow it.
const NUM_ACTIONBAR_PAGES: f64 = 6.0;
/// The vehicle (and possess) bar's page: 31 (recorded, with the
/// temporary shapeshift bar's 32, the override bar's 33 and the extra
/// bar's 34 after it).
const VEHICLE_BAR_INDEX: f64 = 31.0;
/// The temporary shapeshift bar's page (recorded).
const TEMP_SHAPESHIFT_BAR_INDEX: f64 = 32.0;
/// The override bar's page (recorded).
const OVERRIDE_BAR_INDEX: f64 = 33.0;

/// `C_ActionBar.GetVehicleBarIndex()`.
fn get_vehicle_bar_index(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(VEHICLE_BAR_INDEX)])
}

/// `C_ActionBar.GetTempShapeshiftBarIndex()`.
fn get_temp_shapeshift_bar_index(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(TEMP_SHAPESHIFT_BAR_INDEX)])
}

/// `C_ActionBar.GetOverrideBarIndex()`.
fn get_override_bar_index(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(OVERRIDE_BAR_INDEX)])
}

/// `C_ActionBar.GetBonusBarIndex()`: the page of the bonus bar the player
/// is on, `NUM_ACTIONBAR_PAGES` plus `GetBonusBarOffset()` (a druid's cat
/// form's 7), 0 without one (Blizzard's IconIntroAnimation.lua checks the
/// answer against 0; unverified).
fn get_bonus_bar_index(vm: &mut Vm, _c: NativeCall) -> Ret {
    let offset = ask_num(vm, "C_ActionBar.GetBonusBarOffset", &[]).unwrap_or(0.0);
    let index = if offset > 0.0 {
        NUM_ACTIONBAR_PAGES + offset
    } else {
        0.0
    };
    Ok(vec![Value::Num(index)])
}

/// `C_ActionBar.GetBonusBarIndexForSlot(slot)`: the bonus bar page holding
/// the slot (slots 73-132 are pages 7-11, compared with
/// `GetBonusBarIndex()` in ActionButtonUtil.lua), nil for the others
/// (unverified).
fn get_bonus_bar_index_for_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(slot) = arg_num(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    let page = ((slot - 1.0) / 12.0).floor() + 1.0;
    if page > NUM_ACTIONBAR_PAGES && page <= NUM_ACTIONBAR_PAGES + 5.0 {
        return Ok(vec![Value::Num(page)]);
    }
    Ok(vec![Value::Nil])
}

/// `GetActionBarToggles()`'s recorded answer (camelot-answers.txt): the
/// bottom left, bottom right, right and right 2 bars shown, MultiBar 5-7
/// hidden.
const RECORDED_TOGGLES: [bool; 7] = [true, true, true, true, false, false, false];

/// `GetActionBarToggles()`: what `SetActionBarToggles` kept, else the
/// recorded answer.
fn get_action_bar_toggles(vm: &mut Vm, _c: NativeCall) -> Ret {
    let mut out = Vec::with_capacity(7);
    for (i, recorded) in RECORDED_TOGGLES.iter().enumerate() {
        let v = state_get(vm, &format!("toggle{}", i + 1));
        out.push(match v {
            Value::Nil => Value::Bool(*recorded),
            v => Value::Bool(v.truthy()),
        });
    }
    Ok(out)
}

/// `SetActionBarToggles(bottomLeft, bottomRight, right, right2, bar5,
/// bar6, bar7)` (Blizzard's settings, ActionBars.lua): kept for
/// `GetActionBarToggles` (unverified: the client may also keep them in
/// its CVars).
fn set_action_bar_toggles(vm: &mut Vm, c: NativeCall) -> Ret {
    for i in 1..=7 {
        let v = vm.hidden(c.arg(i));
        state_set(vm, &format!("toggle{i}"), Value::Bool(v.truthy()));
    }
    Ok(vec![])
}

// ------------------------------------------------------------ channels

/// `JoinPermanentChannel(name, password, frameID, hasVoice)` (and
/// `JoinTemporaryChannel`, `JoinChannelByName`): the channel joined as a
/// custom one, numbered after those joined; answers its zone channel ID
/// (0 for a custom channel) and its name. A channel already joined answers
/// its own (unverified: the client's notice events are not told, none was
/// recorded).
fn join_channel(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = arg_text(vm, &c, 1).filter(|n| !n.trim().is_empty()) else {
        return Ok(vec![Value::Nil]);
    };
    let name = name.trim().to_string();
    let joined = &engine(vm).world.character.channels.joined;
    if let Some(ch) = joined
        .iter()
        .find(|ch| ch.name.eq_ignore_ascii_case(&name))
        .cloned()
    {
        let n = vm.str(&ch.name);
        return Ok(vec![Value::Num(ch.zone_channel_id), n]);
    }
    let channel = crate::channels::Channel {
        name: name.clone(),
        full_name: name.clone(),
        category: "CHANNEL_CATEGORY_CUSTOM".to_string(),
        ..Default::default()
    };
    engine_mut(vm).world.character.channels.joined.push(channel);
    let n = vm.str(&name);
    Ok(vec![Value::Num(0.0), n])
}

/// `EnumerateServerChannels()`: the server's channels' names, as the
/// recording's realm listed them where the character stood (recorded,
/// "ChromieTime" six times over; unverified: how the list changes with
/// the zone and realm).
fn enumerate_server_channels(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok([
        "General",
        "LocalDefense",
        "LookingForGroup",
        "BigfootWorldChannel",
        "MeetingStone",
        "ShadowlandsBetaDiscussion",
        "ShadowlandsPTRDiscussion",
        "ChromieTime",
        "ChromieTime",
        "ChromieTime",
        "ChromieTime",
        "ChromieTime",
        "ChromieTime",
    ]
    .iter()
    .map(|n| vm.str(n))
    .collect())
}

/// `GetNumChannelMembers(id)`: no member lists are kept (unverified).
fn get_num_channel_members(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

// ------------------------------------------------------- chat types

/// The `ChatTypeInfo` entry of `chatType` (Blizzard's chat frames fill the
/// table).
fn chat_type_info(vm: &mut Vm, chat_type: &str) -> Option<TableRef> {
    let Value::Table(info) = vm.get_global("ChatTypeInfo") else {
        return None;
    };
    match field(vm, info, chat_type) {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

/// `C_ChatInfo.GetColorForChatType(chatType)`: the chat type's color as
/// `ChatTypeInfo` holds it, nil for an unknown type (unverified: the
/// client's own table of colors).
fn get_color_for_chat_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(ty) = arg_text(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    let Some(info) = chat_type_info(vm, &ty) else {
        return Ok(vec![Value::Nil]);
    };
    let r = num(field(vm, info, "r"));
    let g = num(field(vm, info, "g"));
    let b = num(field(vm, info, "b"));
    let (Some(r), Some(g), Some(b)) = (r, g, b) else {
        return Ok(vec![Value::Nil]);
    };
    Ok(vec![color(vm, (r, g, b))])
}

/// `C_ChatInfo.GetChatTypeName(typeID)`: the chat type whose recorded
/// `GetChatTypeIndex` is the ID (recorded: 1 is "SYSTEM", as its index);
/// nil for 0, which several types share, and IDs none has (unverified).
fn get_chat_type_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = arg_num(vm, &c, 1).filter(|&id| id > 0.0) else {
        return Ok(vec![Value::Nil]);
    };
    let answer = format!("1: number {}", wowlua_vm::number::fmt_num(id));
    let found = crate::recorded::answers().iter().find_map(|(call, a)| {
        (*a == answer)
            .then(|| {
                call.strip_prefix("GetChatTypeIndex(\"")?
                    .strip_suffix("\")")
            })
            .flatten()
    });
    Ok(vec![match found {
        Some(name) => vm.str(name),
        None => Value::Nil,
    }])
}

/// `C_ChatInfo.GetNumReservedChatWindows()`: 3 (recorded; Blizzard's
/// FloatingChatFrame.lua keeps the windows up to it: General, Combat Log
/// and a third, the voice transcription's in the mainline UI).
fn get_num_reserved_chat_windows(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(3.0)])
}

/// `C_ChatInfo.IsValidCombatFilterName(name)`: any name with a letter
/// (unverified: the client's name rules).
fn is_valid_combat_filter_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let ok = arg_text(vm, &c, 1).is_some_and(|n| n.chars().any(char::is_alphabetic));
    Ok(vec![Value::Bool(ok)])
}

/// `C_ClubFinder.IsValidSearchString(name)`: any text but an empty one
/// (unverified: the client's profanity filter).
fn is_valid_search_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let ok = arg_text(vm, &c, 1).is_some_and(|n| !n.trim().is_empty());
    Ok(vec![Value::Bool(ok)])
}

/// `C_ChatInfo.CanPlayerSpeakLanguage(languageID)`: whether the language is
/// among the player's (`GetLanguageByIndex`'s IDs, `GetNumLanguages`
/// counting them).
fn can_player_speak_language(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = arg_num(vm, &c, 1) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let count = ask_num(vm, "GetNumLanguages", &[]).unwrap_or(0.0) as i64;
    for i in 1..=count {
        let answer = ask(vm, "GetLanguageByIndex", &[Value::Num(i as f64)]);
        if answer.get(1).copied().and_then(num) == Some(id) {
            return Ok(vec![Value::Bool(true)]);
        }
    }
    Ok(vec![Value::Bool(false)])
}

/// `DeclineName(name, gender, declensionSet)`: the name in each of the
/// five Russian cases; other locales do not decline, so the name itself
/// five times (unverified).
fn decline_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = match vm.hidden(c.arg(1)) {
        v @ Value::Str(_) => v,
        _ => vm.str(""),
    };
    Ok(vec![name; 5])
}

// -------------------------------------------------------- the client

/// `GetDownloadedPercentage()`: 0 (recorded on a fully installed client:
/// nothing being downloaded).
fn get_downloaded_percentage(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `GetNetIpTypes()`: the home and world connections over IPv4 (1 in
/// Blizzard's PerformanceBar.lua table `{ "IPv4", "IPv6" }`; unverified).
fn get_net_ip_types(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1.0), Value::Num(1.0)])
}

/// `GetProtocolTypes()`: the home and world connections over TCP (1 in
/// Blizzard's PerformanceBar.lua table `{ "TCP", "UDP" }`; unverified).
fn get_protocol_types(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1.0), Value::Num(1.0)])
}

/// `GetNumExpansions()`: 1, classic alone (recorded, though the client
/// defines `LE_EXPANSION_MIDNIGHT` 11).
fn get_num_expansions(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1.0)])
}

/// `RequestTimePlayed()`: the server's answer (`TIME_PLAYED_MSG`) was
/// never recorded, so nothing is told (unverified).
fn request_time_played(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `RandomRoll(min, max)`: the server's roll message was never recorded,
/// so nothing is told (unverified).
fn random_roll(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `ConsoleExec(command, addToHistory)`: the engine has no console; the
/// command not run (false; unverified).
fn console_exec(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `ConsoleEcho(command, addToHistory, prefix)`: the text shown on the
/// console (true; unverified).
fn console_echo(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `ConsoleGetColorFromType(colorType)`: the console's colors, white for
/// text, red for errors, yellow for warnings, green for the default green,
/// black for the background (unverified: the client's palette).
fn console_get_color_from_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let rgb = match arg_num(vm, &c, 1).map(|n| n as i64) {
        Some(3) => (1.0, 0.0, 0.0),
        Some(4) => (1.0, 1.0, 0.0),
        Some(8) => (0.0, 0.0, 0.0),
        Some(11) => (0.0, 1.0, 0.0),
        _ => (1.0, 1.0, 1.0),
    };
    Ok(vec![color(vm, rgb)])
}

/// The console's font height until set (unverified).
const CONSOLE_FONT_HEIGHT: f64 = 14.0;

/// `ConsoleGetFontHeight()`: what `ConsoleSetFontHeight` kept, else the
/// default.
fn console_get_font_height(vm: &mut Vm, _c: NativeCall) -> Ret {
    let v = state_get(vm, "consoleFontHeight");
    Ok(vec![Value::Num(num(v).unwrap_or(CONSOLE_FONT_HEIGHT))])
}

/// `ConsoleSetFontHeight(height)`: kept for `ConsoleGetFontHeight`.
fn console_set_font_height(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(h) = arg_num(vm, &c, 1) {
        state_set(vm, "consoleFontHeight", Value::Num(h));
    }
    Ok(vec![])
}

/// `UIReload()`: as `C_UI.Reload` (the session ends; unverified that the
/// two differ in nothing).
fn ui_reload(vm: &mut Vm, c: NativeCall) -> Ret {
    crate::login::reload_ui(vm, c)
}

/// `RunScript(text)`: the text compiled as `loadstring` compiles it (its
/// taint too) and run; a compile error raised (unverified: the client's
/// error text).
fn run_script(vm: &mut Vm, c: NativeCall) -> Ret {
    let text = match vm.hidden(c.arg(1)) {
        v @ (Value::Str(_) | Value::Num(_)) => v,
        _ => return Err(vm.error("Usage: RunScript(text)")),
    };
    let loadstring = vm.get_global("loadstring");
    let compiled = vm.call(loadstring, &[text])?;
    match compiled.first().copied() {
        Some(f @ Value::Func(_)) => {
            vm.call(f, &[])?;
            Ok(vec![])
        }
        _ => {
            let msg = match compiled.get(1).copied() {
                Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                _ => "RunScript: the script did not compile".to_string(),
            };
            Err(vm.error(&msg))
        }
    }
}

/// `C_AddOnProfiler.MeasureCall(func, ...)`: the function called with the
/// arguments; its results and a measure of nothing spent (the engine keeps
/// no profiler; unverified).
fn measure_call(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = c.arg(1);
    let args: Vec<Value> = (2..=c.len()).map(|i| c.arg(i)).collect();
    let returns = vm.call(f, &args)?;
    let results = vm.new_table();
    for k in [
        "elapsedMilliseconds",
        "elapsedTicks",
        "allocatedBytes",
        "deallocatedBytes",
    ] {
        set(vm, results, k, Value::Num(0.0));
    }
    let events = vm.new_table();
    set(vm, results, "events", Value::Table(events));
    let mut out = vec![Value::Table(results)];
    out.extend(returns);
    Ok(out)
}

/// `GetCurrentGraphicsSetting()`: the normal settings, not the raid ones
/// (0; `SetCurrentGraphicsSetting` changes nothing in the engine;
/// unverified).
fn get_current_graphics_setting(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `GetMonitorAspectRatio()`: the physical screen's width over its height
/// (`GetPhysicalScreenSize`; unverified).
fn get_monitor_aspect_ratio(vm: &mut Vm, _c: NativeCall) -> Ret {
    let size = ask(vm, "GetPhysicalScreenSize", &[]);
    let w = size.first().copied().and_then(num).unwrap_or(1920.0);
    let h = size.get(1).copied().and_then(num).unwrap_or(1080.0);
    Ok(vec![Value::Num(if h > 0.0 { w / h } else { 16.0 / 9.0 })])
}

/// `GetCurrentScaledResolution()`: the UI's width and height
/// (`GetScreenWidth`, `GetScreenHeight`; unverified).
fn get_current_scaled_resolution(vm: &mut Vm, _c: NativeCall) -> Ret {
    let w = ask_num(vm, "GetScreenWidth", &[]).unwrap_or(1920.0);
    let h = ask_num(vm, "GetScreenHeight", &[]).unwrap_or(1080.0);
    Ok(vec![Value::Num(w), Value::Num(h)])
}

/// `GetBattlefieldMapIconScale()`: the battlefield map's icons at their
/// size (unverified).
fn get_battlefield_map_icon_scale(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1.0)])
}

/// `IsBNLogin()`: logged in through Battle.net, as `BNFeaturesEnabled`
/// answers (recorded true; unverified that the two agree).
fn is_bn_login(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = ask(vm, "BNFeaturesEnabled", &[])
        .first()
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(on)])
}

/// `GetMaxNumCUFProfiles()`: the raid profiles one may keep (5 as on
/// Blizzard's other clients; unverified).
fn get_max_num_cuf_profiles(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(5.0)])
}

// ---------------------------------------------------------- the player

/// `PlayerCanTeleport()`: whether a summons may be accepted; nothing in
/// the engine stops it (Blizzard's summon popups test combat apart;
/// unverified).
fn player_can_teleport(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `GetAreaText()`: the zone the player is in, as the Who list's zone
/// filter uses it (`GetZoneText`; unverified).
fn get_area_text(vm: &mut Vm, _c: NativeCall) -> Ret {
    match ask(vm, "GetZoneText", &[]).first().copied() {
        Some(v @ Value::Str(_)) => Ok(vec![v]),
        _ => Ok(vec![vm.str("")]),
    }
}

/// `GetQuestResetTime()`: the seconds until the daily quests reset
/// (`C_DateAndTime.GetSecondsUntilDailyReset`; unverified).
fn get_quest_reset_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    let s = ask_num(vm, "C_DateAndTime.GetSecondsUntilDailyReset", &[]).unwrap_or(0.0);
    Ok(vec![Value::Num(s)])
}

/// `GetNumUnspentTalents()`: the unspent talent points
/// `C_ClassTalents.HasUnspentTalentPoints` counts (its second answer;
/// unverified).
fn get_num_unspent_talents(vm: &mut Vm, _c: NativeCall) -> Ret {
    let answer = ask(vm, "C_ClassTalents.HasUnspentTalentPoints", &[]);
    let n = answer.get(1).copied().and_then(num).unwrap_or(0.0);
    Ok(vec![Value::Num(n)])
}

/// `GetQuestLogSpecialItemCooldown(questLogIndex)`: no cooldown running
/// (start 0, duration 0, enabled; unverified).
fn get_quest_log_special_item_cooldown(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0), Value::Num(1.0)])
}

/// `GetTabardCreationCost()`: ten gold, the classic clients' guild tabard
/// price (unverified).
fn get_tabard_creation_cost(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(100_000.0)])
}

/// `IsAchievementEligible(achievementID)`: nothing in the engine keeps an
/// achievement from being earned (unverified).
fn is_achievement_eligible(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// `C_QuestLog.GetTrivialRange()`: how many levels below the player a
/// creature or quest turns grey (Blizzard's camelot name plates colour a
/// level difference below minus it trivial). The classic clients' grey
/// levels: up to 5 none, to 39 `level - floor(level / 10) - 5`, to 59
/// `level - floor(level / 5) - 1`, at 60 `level - 9`, so the range is the
/// difference minus one (unverified).
fn get_trivial_range(vm: &mut Vm, _c: NativeCall) -> Ret {
    let level = player_level(vm).floor();
    let range = if level <= 5.0 {
        level
    } else if level < 40.0 {
        (level / 10.0).floor() + 4.0
    } else if level < 60.0 {
        (level / 5.0).floor()
    } else {
        8.0
    };
    Ok(vec![Value::Num(range)])
}

/// The weapon skill a glancing blow is worked out from: the player's,
/// capped at their level's maximum (the classic clients' rule).
fn capped_skill(vm: &mut Vm, c: &NativeCall) -> f64 {
    let cap = player_level(vm) * 5.0;
    arg_num(vm, c, 1).map_or(cap, |s| s.min(cap))
}

/// `GetGlancingBlowChance(playerWeaponSkill, targetDefenseSkill)`: 10%
/// plus 2% for each point of the target's defense over the player's
/// capped weapon skill, as a fraction (Blizzard's camelot skills frame
/// shows it times 100; the classic clients' 40% against a raid boss;
/// unverified).
fn get_glancing_blow_chance(vm: &mut Vm, c: NativeCall) -> Ret {
    let skill = capped_skill(vm, &c);
    let defense = arg_num(vm, &c, 2).unwrap_or(skill);
    let chance = (0.1 + (defense - skill) * 0.02).clamp(0.0, 1.0);
    Ok(vec![Value::Num(chance)])
}

/// `GetGlancingBlowPenalty(playerWeaponSkill, targetDefenseSkill)`: the
/// damage a glancing blow loses, one minus the average of its low end
/// (1.3 less 0.05 a point of defense over the weapon skill, between 0.01
/// and 0.91) and high end (1.2 less 0.03 a point, between 0.2 and 0.99):
/// the classic clients' 35% at 300 skill against a raid boss, 5% at 308
/// (unverified).
fn get_glancing_blow_penalty(vm: &mut Vm, c: NativeCall) -> Ret {
    let skill = arg_num(vm, &c, 1).unwrap_or_else(|| player_level(vm) * 5.0);
    let defense = arg_num(vm, &c, 2).unwrap_or(skill);
    let diff = defense - skill;
    let low = (1.3 - 0.05 * diff).clamp(0.01, 0.91);
    let high = (1.2 - 0.03 * diff).clamp(0.2, 0.99);
    Ok(vec![Value::Num(1.0 - (low + high) / 2.0)])
}

/// `ResistancePercent(resistance, casterLevel)`: the share of damage the
/// resistance resists against a caster of the level, in percent: 100 at
/// five points a caster level, at most 75 (recorded: 150 against level 60
/// is 50; unverified: whether the share is linear, as here, or
/// `r / (r + 2.5 * level)`, which agrees at that point, and the cap).
fn resistance_percent(vm: &mut Vm, c: NativeCall) -> Ret {
    let resistance = arg_num(vm, &c, 1).unwrap_or(0.0).max(0.0);
    let level = arg_num(vm, &c, 2).unwrap_or(60.0).max(1.0);
    let percent = (100.0 * resistance / (level * 5.0)).min(75.0);
    Ok(vec![Value::Num(percent)])
}

/// `PlayerEffectiveAttackPower()`: the main hand's, off hand's and
/// ranged attack power with their buffs, then the base melee and ranged
/// (`UnitAttackPower` and `UnitRangedAttackPower`'s base plus buffs;
/// unverified that the hands differ in nothing).
fn player_effective_attack_power(vm: &mut Vm, _c: NativeCall) -> Ret {
    let player = vm.str("player");
    let melee = ask(vm, "UnitAttackPower", &[player]);
    let ranged = ask(vm, "UnitRangedAttackPower", &[player]);
    let total = |v: &[Value]| -> (f64, f64) {
        let n = |i: usize| v.get(i).copied().and_then(num).unwrap_or(0.0);
        (n(0), n(0) + n(1) + n(2))
    };
    let (base, melee_total) = total(&melee);
    let (ranged_base, ranged_total) = total(&ranged);
    Ok(vec![
        Value::Num(melee_total),
        Value::Num(melee_total),
        Value::Num(ranged_total),
        Value::Num(base),
        Value::Num(ranged_base),
    ])
}

/// `FindSpellBookSlotBySpellID(spellID)`: the spellbook slot
/// `C_SpellBook.FindSpellBookSlotForSpell` finds (unverified).
fn find_spell_book_slot_by_spell_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let spell = vm.hidden(c.arg(1));
    let answer = ask(vm, "C_SpellBook.FindSpellBookSlotForSpell", &[spell]);
    Ok(vec![answer.first().copied().unwrap_or(Value::Nil)])
}

/// `C_TradeSkillUI.GetTradeSkillLineForRecipe(recipeID)`: the recipe's
/// profession ID, name and parent profession ID, from
/// `C_TradeSkillUI.GetProfessionInfoByRecipeID` (recorded per recipe;
/// unverified that the two agree).
fn get_trade_skill_line_for_recipe(vm: &mut Vm, c: NativeCall) -> Ret {
    let recipe = vm.hidden(c.arg(1));
    let answer = ask(vm, "C_TradeSkillUI.GetProfessionInfoByRecipeID", &[recipe]);
    let Some(Value::Table(info)) = answer.first().copied() else {
        return Ok(vec![]);
    };
    let id = field(vm, info, "professionID");
    if num(id).is_none_or(|n| n == 0.0) {
        return Ok(vec![]);
    }
    let name = field(vm, info, "professionName");
    let parent = field(vm, info, "parentProfessionID");
    Ok(vec![id, name, parent])
}

/// `C_FriendList.GetWhoRaceFilters()`: the playable races' names and IDs
/// as `C_CreatureInfo.GetRaceInfo` names them, IDs 1 to 8 (the classic
/// races; unverified: the client's filter list and order).
fn get_who_race_filters(vm: &mut Vm, _c: NativeCall) -> Ret {
    let mut filters = Vec::new();
    for id in 1..=8 {
        let id = id as f64;
        let info = ask(vm, "C_CreatureInfo.GetRaceInfo", &[Value::Num(id)]);
        let Some(Value::Table(race)) = info.first().copied() else {
            continue;
        };
        let name = field(vm, race, "raceName");
        if !matches!(name, Value::Str(_)) {
            continue;
        }
        let f = vm.new_table();
        set(vm, f, "name", name);
        set(vm, f, "ID", Value::Num(id));
        filters.push(Value::Table(f));
    }
    Ok(vec![list(vm, &filters)])
}

/// `GetMouseButtonClicked()`: nil outside a click (recorded; unverified:
/// the button during a click's handlers, which the engine does not keep
/// for it).
fn get_mouse_button_clicked(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

/// `C_Club.GetClubCapacity()`: 1000 (recorded).
fn get_club_capacity(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1000.0)])
}

/// `C_XMLUtil.GetTemplates()`: none listed (the engine's templates are
/// named by `C_XMLUtil.GetTemplateInfo`; unverified).
fn get_templates(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![list(vm, &[])])
}

/// `CreateFrameWithOptions(options)`: `CreateFrame` with the options'
/// type, name, parent, templates (joined with commas) and ID, hidden when
/// asked (unverified: `forbidden`).
fn create_frame_with_options(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(options) = vm.hidden(c.arg(1)) else {
        return Err(vm.error("Usage: CreateFrameWithOptions(options)"));
    };
    let frame_type = field(vm, options, "frameType");
    let name = field(vm, options, "name");
    let parent = field(vm, options, "parent");
    let inherits = match field(vm, options, "inherits") {
        Value::Table(t) => {
            let mut names = Vec::new();
            let mut i = 1;
            while let Value::Str(s) = vm.heap.table(t).get(Value::Num(i as f64)) {
                names.push(String::from_utf8_lossy(vm.str_bytes(s)).into_owned());
                i += 1;
            }
            if names.is_empty() {
                Value::Nil
            } else {
                vm.str(&names.join(", "))
            }
        }
        v @ Value::Str(_) => v,
        _ => Value::Nil,
    };
    let id = field(vm, options, "id");
    let create = vm.get_global("CreateFrame");
    let made = vm.call(create, &[frame_type, name, parent, inherits, id])?;
    let frame = made.first().copied().unwrap_or(Value::Nil);
    if field(vm, options, "hidden").truthy()
        && let Value::Table(f) = frame
        && let Value::Func(_) = field(vm, f, "Hide")
    {
        let hide = field(vm, f, "Hide");
        vm.call(hide, &[frame])?;
    }
    Ok(vec![frame])
}

/// The module's behaviours.
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    let mut list: Vec<(&'static str, NativeFn)> = vec![
        // Action bars.
        ("C_ActionBar.GetVehicleBarIndex", get_vehicle_bar_index),
        (
            "C_ActionBar.GetTempShapeshiftBarIndex",
            get_temp_shapeshift_bar_index,
        ),
        ("C_ActionBar.GetOverrideBarIndex", get_override_bar_index),
        ("C_ActionBar.GetBonusBarIndex", get_bonus_bar_index),
        (
            "C_ActionBar.GetBonusBarIndexForSlot",
            get_bonus_bar_index_for_slot,
        ),
        ("GetActionBarToggles", get_action_bar_toggles),
        ("SetActionBarToggles", set_action_bar_toggles),
        // Channels.
        ("JoinPermanentChannel", join_channel),
        ("JoinTemporaryChannel", join_channel),
        ("JoinChannelByName", join_channel),
        ("EnumerateServerChannels", enumerate_server_channels),
        ("GetNumChannelMembers", get_num_channel_members),
        // Chat types and names.
        ("C_ChatInfo.GetColorForChatType", get_color_for_chat_type),
        ("C_ChatInfo.GetChatTypeName", get_chat_type_name),
        (
            "C_ChatInfo.GetNumReservedChatWindows",
            get_num_reserved_chat_windows,
        ),
        (
            "C_ChatInfo.IsValidCombatFilterName",
            is_valid_combat_filter_name,
        ),
        (
            "C_ChatInfo.CanPlayerSpeakLanguage",
            can_player_speak_language,
        ),
        ("C_ClubFinder.IsValidSearchString", is_valid_search_string),
        ("DeclineName", decline_name),
        // The client.
        ("GetDownloadedPercentage", get_downloaded_percentage),
        ("GetNetIpTypes", get_net_ip_types),
        ("GetProtocolTypes", get_protocol_types),
        ("GetNumExpansions", get_num_expansions),
        ("RequestTimePlayed", request_time_played),
        ("RandomRoll", random_roll),
        ("ConsoleExec", console_exec),
        ("ConsoleEcho", console_echo),
        ("ConsoleGetColorFromType", console_get_color_from_type),
        ("ConsoleGetFontHeight", console_get_font_height),
        ("ConsoleSetFontHeight", console_set_font_height),
        ("UIReload", ui_reload),
        ("RunScript", run_script),
        ("C_AddOnProfiler.MeasureCall", measure_call),
        ("GetCurrentGraphicsSetting", get_current_graphics_setting),
        ("GetMonitorAspectRatio", get_monitor_aspect_ratio),
        ("GetCurrentScaledResolution", get_current_scaled_resolution),
        ("GetBattlefieldMapIconScale", get_battlefield_map_icon_scale),
        ("IsBNLogin", is_bn_login),
        ("GetMaxNumCUFProfiles", get_max_num_cuf_profiles),
        ("C_XMLUtil.GetTemplates", get_templates),
        ("CreateFrameWithOptions", create_frame_with_options),
        // The player.
        ("PlayerCanTeleport", player_can_teleport),
        ("GetAreaText", get_area_text),
        ("GetQuestResetTime", get_quest_reset_time),
        ("GetNumUnspentTalents", get_num_unspent_talents),
        (
            "GetQuestLogSpecialItemCooldown",
            get_quest_log_special_item_cooldown,
        ),
        ("GetTabardCreationCost", get_tabard_creation_cost),
        ("IsAchievementEligible", is_achievement_eligible),
        ("C_QuestLog.GetTrivialRange", get_trivial_range),
        ("GetGlancingBlowChance", get_glancing_blow_chance),
        ("GetGlancingBlowPenalty", get_glancing_blow_penalty),
        ("ResistancePercent", resistance_percent),
        ("PlayerEffectiveAttackPower", player_effective_attack_power),
        (
            "FindSpellBookSlotBySpellID",
            find_spell_book_slot_by_spell_id,
        ),
        (
            "C_TradeSkillUI.GetTradeSkillLineForRecipe",
            get_trade_skill_line_for_recipe,
        ),
        ("C_FriendList.GetWhoRaceFilters", get_who_race_filters),
        ("C_Club.GetClubCapacity", get_club_capacity),
        ("GetMouseButtonClicked", get_mouse_button_clicked),
    ];
    list.extend(DOCUMENTED_EMPTY.iter().map(|&n| (n, nothing as NativeFn)));
    list.extend(UNDOCUMENTED.iter().map(|&(n, _)| (n, nothing as NativeFn)));
    list
}

#[cfg(test)]
mod tests {
    #[test]
    fn listed_once() {
        let names: Vec<&str> = super::behaviours().iter().map(|(n, _)| *n).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "a function listed twice");
    }
}
