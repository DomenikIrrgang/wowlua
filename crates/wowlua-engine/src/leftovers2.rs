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
pub(crate) fn ask(vm: &mut Vm, name: &'static str, args: &[Value]) -> Vec<Value> {
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
    // An inspected player's talents: none.
    ("GetInspectTalent", "n"),
    // Guild news filters: none kept.
    ("GetGuildNewsFilters", "{"),
    // Quest POIs and sorting: none (the map's own `C_QuestLog` answers).
    ("GetQuestPOIs", ""),
    ("GetTaskPOIs", ""),
    ("GetQuestPOILeaderBoard", "n"),
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
    // Raid profiles (none: `GetNumRaidProfiles` 0), no death recap, no
    // housing of one's own, the old talent and pet spell buttons, the taxi
    // map's texture, the trainer and guild roster windows' requests, a
    // critter summoned (no companions), UIParent placed by the client.
    ("CreateNewRaidProfile", ""),
    ("DeleteRaidProfile", ""),
    ("RestoreRaidProfileFromCopy", ""),
    ("SaveRaidProfileCopy", ""),
    ("SetRaidProfileOption", ""),
    ("SetRaidProfileSavedPosition", ""),
    ("DeathRecap_GetEvents", "n"),
    ("DeathRecap_HasEvents", "f"),
    ("GetDeathRecapLink", "n"),
    ("C_Housing.IsInsideOwnHouse", "f"),
    ("PickupTalent", ""),
    ("RemoveTalent", ""),
    ("PickupPetAction", ""),
    ("PickupPetSpell", ""),
    ("SetSpellbookPetAction", ""),
    ("SetTaxiMap", ""),
    ("OpenTrainer", ""),
    ("SortGuildRoster", ""),
    ("SummonRandomCritter", ""),
    ("UpdateUIParentPosition", ""),
    // The spell queue window's offset: none.
    ("GetMaxSpellStartRecoveryOffset", "0"),
    // A chat link clicked: Blizzard's ItemRef.lua defines it when its UI
    // code is loaded (addon links then reach `EventRegistry`'s
    // `SetItemRef`); without that code nothing happens.
    ("SetItemRef", ""),
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
    // The client's debug switches and internal systems: nothing drawn,
    // nothing moved, nothing to buy, trade or mail securely here; the
    // direction and totem targeting the engine's world has no place for.
    "C_BarberShopInternal.SetQAMode",
    "C_CinematicList.GetUICinematicList",
    "C_CombatLogInternal.GetCurrentEventInfo",
    "C_ConfigurationWarnings.GetConfigurationWarningSeen",
    "C_ConfigurationWarnings.GetConfigurationWarningString",
    "C_ConfigurationWarnings.GetConfigurationWarnings",
    "C_ConfigurationWarnings.SetConfigurationWarningSeen",
    "C_HousingBasicMode.StartPlacingNewDecor",
    "C_HousingCatalog.DestroyEntry",
    "C_HousingCatalog.GetCatalogCategoryInfo",
    "C_HousingCatalog.GetCatalogEntryInfo",
    "C_HousingCatalog.GetCatalogEntryInfoByItem",
    "C_HousingCatalog.GetCatalogEntryInfoByRecordID",
    "C_HousingCatalog.GetCatalogSubcategoryInfo",
    "C_HousingCatalog.SearchCatalogCategories",
    "C_HousingCatalog.SearchCatalogSubcategories",
    "C_HousingDecor.GetMaxPlacementBudget",
    "C_HousingDecor.GetSpentPlacementBudget",
    "C_HousingLayout.GetRoomPlacementBudget",
    "C_HousingLayout.GetSpentPlacementBudget",
    "C_LiveEvent.OnLiveEventBannerClicked",
    "C_LiveEvent.OnLiveEventPopupClicked",
    "C_SecureTransfer.AcceptTrade",
    "C_SecureTransfer.Cancel",
    "C_SecureTransfer.CompleteHousingPurchase",
    "C_SecureTransfer.CompleteHousingVCPurchase",
    "C_SecureTransfer.GetHousingPurchaseCost",
    "C_SecureTransfer.GetHousingPurchaseQuantity",
    "C_SecureTransfer.GetHousingVCPurchaseProductID",
    "C_SecureTransfer.GetMailReceiveInfo",
    "C_SecureTransfer.GetMailSendInfo",
    "C_SecureTransfer.GetTradePartner",
    "C_SecureTransfer.SendMail",
    "C_SecureTransfer.ShouldShowTradeOfferWarning",
    "C_SecureTransfer.ShouldShowTradeRMTWarning",
    "C_SecureTransfer.TakeMailMoney",
    "C_SpellBook.CastSpellBookItem",
    "C_WorldSafeLocsUIInternal.GetWorldSafeLocs",
    "CreateSecureDelegate",
    "IsCollisionEnabled",
    "NotWhileDeadError",
    "SitStandOrDescendStart",
    "TargetDirectionEnemy",
    "TargetDirectionFinished",
    "TargetDirectionFriend",
    "TargetToggle",
    "TargetTotem",
    "ToggleAnimKitDisplay",
    "ToggleCollision",
    "ToggleCollisionDisplay",
    "ToggleDebugAIDisplay",
    "ToggleGravity",
    "TogglePlayerBounds",
    "TogglePortals",
    "ToggleSit",
    "ToggleTris",
    // Battle.net's optional features: off; no game data sent.
    "C_BattleNet.AreHighResTexturesInstalled",
    "C_BattleNet.GetFriendLevelFromInviteText",
    "C_BattleNet.SendGameData",
    // Voice chat: nothing to log into.
    "C_VoiceChat.Login",
    "C_VoiceChat.Logout",
    "C_VoiceChat.SetActiveVoiceProvider",
    // Reporting: nothing to report (the report frame's lists empty).
    "C_ReportSystem.CanReportPlayer",
    "C_ReportSystem.CanReportPlayerForLanguage",
    "C_ReportSystem.RequiresScreenshotForReportType",
    // Chat lines: none kept by their IDs; no regional or mentor channel.
    "C_ChatInfo.GetChatLineText",
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
        number: crate::channels::next_number(&engine(vm).world.character.channels),
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
    // Taken whole (the full probe: 1.5 answered as 1).
    let Some(id) = arg_num(vm, &c, 1).map(f64::trunc).filter(|&id| id > 0.0) else {
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

/// `C_ChatInfo.CanPlayerSpeakLanguage(languageID)`: whether the language is
/// among the player's (`GetLanguageByIndex`'s IDs, `GetNumLanguages`
/// counting them).
fn can_player_speak_language(vm: &mut Vm, c: NativeCall) -> Ret {
    // Taken whole (the full probe, 1.60.1: 1.5 as 1).
    let Some(id) = arg_num(vm, &c, 1).map(f64::trunc) else {
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
/// black for the background; 1 sky blue, 2 grey and 10 pale blue as bytes;
/// a color without alpha (the full probe: every type 0 to 11).
fn console_get_color_from_type(vm: &mut Vm, c: NativeCall) -> Ret {
    // A byte times 1/255 in single precision (the client's 196 is
    // 0.76862752437592).
    let platform = crate::engine(vm).platform;
    let byte = |b: f64| crate::byte_channel(platform, b as f32);
    let rgb = match arg_num(vm, &c, 1).map(|n| n as i64) {
        Some(1) => (byte(128.0), byte(196.0), 1.0),
        Some(2) => (byte(128.0), byte(128.0), byte(128.0)),
        Some(10) => (byte(200.0), byte(200.0), 1.0),
        Some(3) => (1.0, 0.0, 0.0),
        Some(4) => (1.0, 1.0, 0.0),
        Some(8) => (0.0, 0.0, 0.0),
        Some(11) => (0.0, 1.0, 0.0),
        _ => (1.0, 1.0, 1.0),
    };
    let color = color(vm, rgb);
    if let Value::Table(t) = color {
        set(vm, t, "a", Value::Nil);
    }
    Ok(vec![color])
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
    let ratio = if h > 0.0 { w / h } else { 16.0 / 9.0 };
    // A single-precision float (the full probe: 1.7777777910233).
    Ok(vec![Value::Num(f64::from(ratio as f32))])
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
    // Worked out in single-precision floats (the full probe: 0.12000000476837
    // for a point over, 0.079999998211861 for one under, 0.10000000149012).
    let chance = (0.1_f32 + (defense - skill) as f32 * 0.02_f32).clamp(0.0, 1.0);
    Ok(vec![Value::Num(f64::from(chance))])
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
    // Worked in single precision (the full probe: 0.049999952316284).
    let (low, high) = (low as f32, high as f32);
    Ok(vec![Value::Num(f64::from(1.0 - (low + high) / 2.0))])
}

/// `ResistancePercent(resistance, casterLevel)`: the share of damage the
/// resistance resists against a caster of the level, in percent,
/// `100 r / (r + 2.5 level)` as a single-precision float (recorded: 150
/// against level 60 is 50; the user's probe: 300 against 60 is
/// 66.66667175293, so not linear; unverified: a cap).
fn resistance_percent(vm: &mut Vm, c: NativeCall) -> Ret {
    let resistance = arg_num(vm, &c, 1).unwrap_or(0.0).max(0.0);
    let level = arg_num(vm, &c, 2).unwrap_or(60.0).max(1.0);
    let percent = 100.0 * resistance / (resistance + 2.5 * level);
    Ok(vec![Value::Num(f64::from(percent as f32))])
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

/// `C_FriendList.GetWhoRaceFilters()`: the player's faction's races' names
/// and IDs as `C_CreatureInfo.GetRaceInfo` names them (the full probe,
/// 1.60.1, a Horde character: 2, 5, 6, 8 and 96, the classic Horde races
/// and race 96, whose name the engine lacks: unverified; an Alliance
/// character's list unrecorded, here its four classic races).
fn get_who_race_filters(vm: &mut Vm, _c: NativeCall) -> Ret {
    let player = vm.str("player");
    let faction = ask(vm, "UnitFactionGroup", &[player]);
    let faction = match faction.first().copied() {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    let mut ids: Vec<f64> = Vec::new();
    for id in 1..=8 {
        let id = id as f64;
        let info = ask(vm, "C_CreatureInfo.GetFactionInfo", &[Value::Num(id)]);
        let tag = match info.first().copied() {
            Some(Value::Table(t)) => field(vm, t, "groupTag"),
            _ => Value::Nil,
        };
        let same = match tag {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)) == faction,
            _ => faction.is_empty(),
        };
        if same {
            ids.push(id);
        }
    }
    if faction == "Horde" {
        ids.push(96.0);
    }
    let mut filters = Vec::new();
    for id in ids {
        let info = ask(vm, "C_CreatureInfo.GetRaceInfo", &[Value::Num(id)]);
        let name = match info.first().copied() {
            Some(Value::Table(race)) => field(vm, race, "raceName"),
            _ => Value::Nil,
        };
        let name = match name {
            Value::Str(_) => name,
            _ if id == 96.0 => vm.str(""),
            _ => continue,
        };
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

// ------------------------------------------------ the full probe's constants

/// Functions of systems the character was not in (no pet battle, black
/// market, talking head, scenario, voice channel, adventure map…) or
/// whose answer is a rule of the client's: one answer for every argument
/// the full probe (1.60.1, `/stdtest engine fullprobe`) passed, as
/// `data/camelot-answers.txt` writes answers. Unverified: the systems in
/// use (a pet battle, an open black market).
const PROBED_CONSTANT: &[(&str, &str)] = &[
    // Pet battles, out of one: the slots' default pets.
    ("C_PetBattles.GetBreedQuality", "1: number 0"),
    ("C_PetBattles.GetXP", "2: number 0, number 50"),
    (
        "C_PetBattles.GetTurnTimeInfo",
        "3: number 0, number 0, number 0",
    ),
    ("C_PetBattles.GetPlayerTrapAbility", "0: "),
    ("C_PetBattles.GetPetType", "1: number 1"),
    ("C_PetBattles.GetNumAuras", "0: "),
    ("C_PetBattles.GetMaxHealth", "1: number 100"),
    ("C_PetBattles.GetLevel", "1: number 1"),
    ("C_PetBattles.GetHealth", "1: number 100"),
    ("C_PetBattles.GetDisplayID", "1: number 0"),
    ("C_PetBattles.GetActivePet", "1: number 1"),
    ("C_PetBattles.CanAcceptQueuedPVPMatch", "1: boolean true"),
    ("C_PetBattles.CanPetSwapIn", "1: boolean false"),
    ("C_PetBattles.GetAbilityStateModification", "1: number 0"),
    // No black market, adventure map, talking head, reincarnation,
    // diminishing returns, death recap events, debug objects.
    ("C_BlackMarket.GetNumItems", "0: "),
    ("C_BlackMarket.GetItemInfoByIndex", "0: "),
    ("C_BlackMarket.GetItemInfoByID", "0: "),
    ("C_BlackMarket.GetHotItem", "0: "),
    ("C_BlackMarket.IsViewOnly", "0: "),
    ("C_AdventureMap.GetMapID", "0: "),
    ("C_AdventureMap.GetQuestOfferInfo", "0: "),
    ("C_AdventureMap.GetQuestInfo", "0: "),
    ("C_AdventureMap.GetMapInsetInfo", "0: "),
    ("C_AdventureMap.GetZoneChoiceInfo", "0: "),
    ("C_AdventureMap.GetMapInsetDetailTileInfo", "0: "),
    ("C_AdventureMap.GetAdventureMapTextureKit", "1: nil nil"),
    ("C_AdventureJournal.GetReward", "0: "),
    ("C_TalkingHead.IsCurrentTalkingHeadIgnored", "0: "),
    ("C_TalkingHead.GetCurrentLineInfo", "0: "),
    ("C_TalkingHead.GetCurrentLineAnimationInfo", "0: "),
    ("C_TalkingHead.GetConversationsDeferred", "0: "),
    ("C_Reincarnation.IsReincarnating", "0: "),
    ("C_Reincarnation.GetReincarnatingCharacter", "0: "),
    ("C_SpellDiminish.ShouldTrackSpellDiminishCategory", "0: "),
    ("C_SpellDiminish.GetSpellDiminishCategoryInfo", "0: "),
    ("C_SpellDiminish.GetAllSpellDiminishCategories", "0: "),
    ("C_DeathRecap.GetRecapEvents", "0: "),
    ("C_DeathRecap.GetRecapLink", "1: string You died."),
    ("C_HouseExterior.GetSelectedFixtureDebugInfo", "0: "),
    ("C_HouseExterior.GetHoveredFixtureDebugInfo", "0: "),
    ("C_HouseExterior.GetFixtureDebugInfoForGUID", "0: "),
    // Voice chat: no channel, no members.
    ("C_VoiceChat.IsMemberSilenced", "1: nil nil"),
    ("C_VoiceChat.IsMemberMutedForAll", "1: nil nil"),
    ("C_VoiceChat.IsMemberMuted", "1: nil nil"),
    ("C_VoiceChat.GetMemberGUID", "1: nil nil"),
    (
        "C_VoiceChat.GetCurrentVoiceChatConnectionStatusCode",
        "1: number 0",
    ),
    ("C_VoiceChat.GetProcesses", "0: "),
    // The pet journal and toys: none summoned, no abilities listed.
    ("C_PetJournal.GetSummonedPetGUID", "0: "),
    (
        "C_PetJournal.GetSummonRandomFavoritePetGUID",
        "1: string BattlePet-0-FFFFFFFFFFFFFF",
    ),
    (
        "C_PetJournal.GetSummonBattlePetCooldown",
        "3: number 0, number 0, number 1",
    ),
    ("C_PetJournal.GetNumPetSources", "1: number 12"),
    ("C_PetJournal.GetPetAbilityListTable", "0: "),
    ("C_PetJournal.GetPetAbilityList", "0: "),
    ("C_PetJournal.GetNumCollectedInfo", "0: "),
    (
        "C_PetJournal.GetPetSummonInfo",
        "3: boolean false, number 0, nil nil",
    ),
    ("C_PetJournal.GetNonBattlePetLinkByIndex", "1: nil nil"),
    ("C_PetJournal.PetIsFavorite", "1: nil nil"),
    ("C_PetJournal.GetPetCooldownByGUID", "0: "),
    ("C_PetJournal.GetBattlePetLink", "0: "),
    ("C_ToyBox.IsToyUsable", "0: "),
    ("C_ToyBox.GetToyLink", "0: "),
    ("C_Heirloom.GetHeirloomMaxUpgradeLevel", "0: "),
    // Transmog: every source collectable, no variants, no drops.
    ("C_TransmogSets.GetVariantSets", "0: "),
    ("C_TransmogSets.GetSetNewSources", "1: {}"),
    ("C_TransmogSets.HasAnyValidSets", "1: boolean false"),
    (
        "C_TransmogSets.GetValidBaseSetsCountsForCharacter",
        "2: number 0, number 0",
    ),
    (
        "C_TransmogSets.GetFullBaseSetsCounts",
        "2: number 0, number 0",
    ),
    (
        "C_TransmogSets.GetFilteredBaseSetsCounts",
        "2: number 0, number 0",
    ),
    (
        "C_TransmogCollection.PlayerCanCollectSource",
        "2: boolean true, boolean false",
    ),
    (
        "C_TransmogCollection.AccountCanCollectSource",
        "2: boolean true, boolean false",
    ),
    ("C_TransmogCollection.GetAppearanceSourceDrops", "0: "),
    (
        "C_TransmogCollection.GetValidAppearanceSourcesForClass",
        "0: ",
    ),
    ("C_TransmogCollection.GetNumMaxCustomSets", "1: number 25"),
    (
        "C_TransmogCollection.GetInspectItemTransmogInfoList",
        "1: {}",
    ),
    (
        "C_TransmogOutfitInfo.GetOutfitSituationsEnabled",
        "1: boolean true",
    ),
    ("C_TransmogOutfitInfo.GetNextOutfitCost", "1: number 1000"),
    (
        "C_TransmogOutfitInfo.GetMaxNumberOfUsableOutfits",
        "1: number 50",
    ),
    // PvP outside a battleground: no flags, the rules' constants.
    (
        "C_PvP.GetBattlefieldFlagPosition",
        "3: number 0, number 0, number 0",
    ),
    ("C_PvP.IsMatchFactional", "1: boolean true"),
    ("C_PvP.GetUIDisplaySeason", "1: number 1"),
    ("C_PvP.GetPvpTalentsUnlockedLevel", "1: number 20"),
    ("C_PvP.GetActiveMatchBracket", "1: number -1"),
    ("C_PvP.CanDisplayHonorableKills", "1: boolean true"),
    ("C_PvP.CanDisplayDeaths", "1: boolean true"),
    ("C_PvP.AreTrainingGroundsEnabled", "1: boolean true"),
    // Game rules and services.
    (
        "C_CraftingOrders.CalculateCraftingOrderPostingFee",
        "1: number 100",
    ),
    ("C_WowTokenPublic.GetGuaranteedPrice", "1: number 0"),
    (
        "C_WowTokenPublic.GetCurrentMarketPrice",
        "2: nil nil, nil nil",
    ),
    (
        "C_WowTokenPublic.GetCommerceSystemStatus",
        "3: boolean false, number 300, boolean false",
    ),
    ("C_WowLabsDataManager.IsInPrematch", "1: boolean true"),
    ("C_WowLabsDataManager.GetWoWLabsAreaInfo", "1: {}"),
    ("C_Trophy.MonumentGetSelectedTrophyID", "1: number 0"),
    ("C_LFGList.HasActivityList", "1: boolean true"),
    ("C_LFGList.GetSearchResultEncounterInfo", "0: "),
    ("C_Scenario.GetBonusStepRewardQuestID", "1: number 0"),
    ("C_AddOns.GetAddOnLocalTable", "0: "),
    ("C_GameRules.IsGameModeEnabled", "1: boolean true"),
    ("C_GameRules.GetGameModePromoGlobalString", "1: nil nil"),
    ("C_GameRules.GetGameRuleAsFrameStrata", "1: nil nil"),
    ("C_GameRules.GetGameModeGlueScreenName", "1: nil nil"),
    ("C_GameRules.GetNumDisplayedGameModes", "1: number 5"),
    ("C_GameRules.GetCurrentGameModeRecordID", "1: number 15"),
    ("C_AccountStore.GetStoreFrontState", "1: number 1"),
    ("C_ClubFinder.GetPostingIDFromClubFinderGUID", "1: number 0"),
    ("C_ClubFinder.PlayerGetClubInvitationList", "0: "),
    ("C_AccountInfo.GetIDFromBattleNetAccountGUID", "1: nil nil"),
    ("C_Mail.GetCraftingOrderMailInfo", "1: nil nil"),
    (
        "C_ChatInfo.GetClubStreamIDs",
        "1: {1=number 0,2=number 0,3=number 0,4=number 0,5=number 0,6=number 0}",
    ),
    ("C_KeyBindings.GetBindingContextForAction", "1: number 0"),
    ("C_Intl.GetCurrencyFractionDigits", "1: number 2"),
    (
        "C_SharedCharacterServices.IsPurchaseIDPendingUpgrade",
        "0: ",
    ),
    (
        "C_SeasonInfo.GetCurrentDisplaySeasonExpansion",
        "1: number 0",
    ),
    ("C_SeasonInfo.GetCurrentDisplaySeasonID", "1: number 29"),
    (
        "C_RecruitAFriend.GetSummonFriendCooldown",
        "3: number 0, number 0, boolean true",
    ),
    // No club finder posting: the documentation's structure without its
    // GUIDs, the club's ID 0 (the full probe, 1.60.1: every GUID asked).
    (
        "C_ClubFinder.GetRecruitingClubInfoFromFinderGUID",
        "1: {cacheRequested=number 0,cached=number 0,clubId=number 0,comment=string ,emblemInfo=number 0,guildLeader=string ,isCrossFaction=boolean false,isGuild=boolean false,lastUpdatedTime=number 0,localeSet=boolean false,minILvl=number 0,name=string ,numActiveMembers=number 0,recruitingSpecIds={},recruitmentFlags=number 0,recruitmentLocale=number 0}",
    ),
    // Not spectating.
    ("C_SpectatingUI.GetSpectateTargetInfo", "0: "),
    // The encounter journal's slot filter: none (`Enum.ItemSlotFilterType`
    // NoFilter, 15).
    ("C_EncounterJournal.GetSlotFilter", "1: number 15"),
    // Retail's island expeditions' difficulties, game data the client
    // keeps.
    (
        "C_IslandsQueue.GetIslandDifficultyInfo",
        "1: {1={difficultyId=number 1726,previewRewardQuestId=number 51838},2={difficultyId=number 1736,previewRewardQuestId=number 53339},3={difficultyId=number 1737,previewRewardQuestId=number 53340},4={difficultyId=number 1762,previewRewardQuestId=number 53341}}",
    ),
    (
        "C_GameRules.GetCurrentGameModeDisplayInfo",
        "1: {characterCreateExtraHeight=number 0,logoHeight=number 136,logoShrunkenHeight=number 0,logoUsesDarkBackdrop=boolean false,logoVerticalOffset=number -4}",
    ),
    (
        "C_QuestOffer.GetQuestOfferMajorFactionReputationRewards",
        "0: ",
    ),
    ("C_PhotoSharing.GetCropRatio", "1: number 1"),
    ("C_PartyInfo.GetLootMethodStyle", "1: number 1"),
    ("C_IslandsQueue.GetIslandsMaxGroupSize", "1: number 3"),
    ("C_IslandsQueue.GetIslandsWeeklyQuestID", "1: number 53435"),
    ("C_HouseExterior.IsExteriorDecorHidden", "1: boolean true"),
    ("C_HouseEditor.GetHouseEditorAvailability", "1: number 50"),
    ("C_CombatLog.IsCombatLogRestricted", "1: boolean true"),
    ("C_CombatLog.GetMessageLimit", "1: number 300"),
    ("C_CombatLog.GetEntryRetentionTime", "1: number 300"),
    ("C_CombatText.GetActiveUnit", "0: "),
    ("C_Calendar.CanSendInvite", "1: boolean true"),
    ("C_Calendar.CanAddEvent", "1: boolean true"),
    (
        "C_BattleNet.CanToggleHighResTexturesWithoutClientReload",
        "1: boolean true",
    ),
    ("C_BattleNet.AreFriendTagsEnabled", "1: boolean true"),
    (
        "C_ArdenwealdGardening.IsGardenAccessible",
        "1: boolean true",
    ),
    ("C_AnimaDiversion.GetTextureKit", "1: nil nil"),
    ("C_AddOnProfiler.IsEnabled", "1: boolean true"),
    ("C_AddOnProfiler.GetTicksPerSecond", "1: number 24000000"),
    ("C_CovenantSanctumUI.GetRenownLevel", "1: number 1"),
    (
        "C_CovenantSanctumUI.GetAnimaInfo",
        "2: number 1813, number 0",
    ),
    ("C_CovenantSanctumUI.CanAccessReservoir", "1: boolean true"),
    (
        "C_CharacterServicesPublic.ShouldSeeControlPopup",
        "1: boolean true",
    ),
    ("C_EncounterWarnings.GetWarningsShown", "1: boolean true"),
    // Names: every one valid, the empty one too.
    ("C_ChatInfo.IsValidCombatFilterName", "1: boolean true"),
    ("C_ClubFinder.IsValidSearchString", "1: boolean true"),
    ("C_EditMode.IsValidLayoutName", "1: boolean true"),
    (
        "C_TransmogCollection.IsValidCustomSetName",
        "1: boolean true",
    ),
    (
        "C_TransmogOutfitInfo.IsValidTransmogOutfitName",
        "1: boolean true",
    ),
    // The group finders: usable (the reason text scrubbed).
    ("C_LFGInfo.CanPlayerUseLFR", "2: boolean true, string "),
    ("C_LFGInfo.CanPlayerUsePVP", "2: boolean true, string "),
    (
        "C_LFGInfo.CanPlayerUseScenarioFinder",
        "2: boolean true, string ",
    ),
    // The monument's trophies: none chosen, 57 of them.
    (
        "C_Trophy.MonumentGetTrophyInfoByIndex",
        "2: number 0, number 57",
    ),
    // Assisted combat not available.
    (
        "C_AssistedCombat.IsAvailable",
        "2: boolean false, string WRONG_WORLD_STATE_EXPRESSION",
    ),
    // Trait configs: no variation (every config probed).
    ("C_Traits.GetConfigVariationID", "1: number 0"),
    // Specializations: one each class (class 0 too), no PvP talent slot.
    (
        "C_SpecializationInfo.GetNumSpecializationsForClassID",
        "1: number 1",
    ),
    (
        "C_SpecializationInfo.GetPvpTalentSlotInfo",
        "1: {availableTalentIDs={},enabled=boolean false,level=number 0}",
    ),
];

/// A club finder posting's recruitment flags: bit `n` set for
/// `Enum.ClubFinderSettingFlags` value `n` (Blizzard's ClubFinder.lua
/// compares the focus index with the enumeration's values).
fn club_finder_flags(vm: &Vm, c: &NativeCall) -> u64 {
    arg_num(vm, c, 1).map_or(0, |n| n.trunc().max(0.0) as u64)
}

/// `C_ClubFinder.GetFocusIndexFromFlag(flags)`: the first focus set, from
/// Dungeons (1) to Social (5), else 0 (the full probe, 1.60.1: 2147483647
/// answered 1, 0 and 1 answered 0; which focus wins is unverified).
fn club_finder_focus_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let flags = club_finder_flags(vm, &c);
    let focus = (1..=5).find(|bit| flags & (1 << bit) != 0).unwrap_or(0);
    Ok(vec![Value::Num(f64::from(focus))])
}

/// `C_ClubFinder.IsListingEnabledFromFlags(flags)`: the EnableListing
/// flag (12) set (the full probe, 1.60.1: 2147483647 true, 0 and 1 false).
fn club_finder_listing_enabled(vm: &mut Vm, c: NativeCall) -> Ret {
    let flags = club_finder_flags(vm, &c);
    Ok(vec![Value::Bool(flags & (1 << 12) != 0)])
}

/// `C_Club.GetCommunityNameResultText(result)`: a failed name check's
/// text, nil for Success (the full probe, 1.60.1: 0 and "x" nil, 1 to 17
/// a text each, scrubbed there; here the result's name in
/// `Enum.ValidateNameResult`, unverified).
fn community_name_result_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let result = arg_num(vm, &c, 1).map_or(0.0, f64::trunc);
    let name = engine(vm)
        .api
        .enums
        .iter()
        .find(|(n, _)| *n == "ValidateNameResult")
        .and_then(|(_, fields)| fields.iter().find(|(_, v)| *v == result && result != 0.0))
        .map(|(f, _)| f.clone());
    Ok(vec![name.map_or(Value::Nil, |n| vm.str(&n))])
}

/// `C_Debug.GetMapDebugObjects(uiMapID)`, `GetAllPortLocsForMap(uiMapID)`
/// (undocumented, checked as the bindings check a documented 32-bit
/// integer): nothing, no debug objects there; no map refused with the
/// usage, a non-number for the range (the full probe, 1.60.1: `()`,
/// `("x")`, `("player")`; -1 to 1413 answer nothing).
fn debug_map_objects(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = crate::recorded::called_name(vm, &c).unwrap_or_default();
    let what = if name.ends_with("GetAllPortLocsForMap") {
        "local debugPortLocInfos = C_Debug.GetAllPortLocsForMap(uiMapID)"
    } else {
        "local mapDebugObjects = C_Debug.GetMapDebugObjects(uiMapID)"
    };
    let v = vm.hidden(c.arg(1));
    if v.is_nil() {
        return Err(vm.api_arg_error(1, &format!("Usage: {what}")));
    }
    if vm
        .tonumber(v)
        .is_none_or(|n| !(-2_147_483_648.0..=2_147_483_647.0).contains(&n))
    {
        return Err(vm.api_arg_error(
            1,
            &format!("outside of expected range -2147483648 to 2147483647 - Usage: {what}"),
        ));
    }
    Ok(vec![])
}

/// The creature types (`CreatureType.db2`) Forever lists: 1 to 12 and
/// 14 (the full probe, 1.60.1: `GetCreatureTypeIDs()`), by name (1 Beast
/// recorded; the others Classic's names, unverified).
const CREATURE_TYPES: [(f64, &str); 13] = [
    (1.0, "Beast"),
    (2.0, "Dragonkin"),
    (3.0, "Demon"),
    (4.0, "Elemental"),
    (5.0, "Giant"),
    (6.0, "Undead"),
    (7.0, "Humanoid"),
    (8.0, "Critter"),
    (9.0, "Mechanical"),
    (10.0, "Not specified"),
    (11.0, "Totem"),
    (12.0, "Non-combat Pet"),
    (14.0, "Wild Pet"),
];

/// The creature families Forever lists (the full probe, 1.60.1:
/// `GetCreatureFamilyIDs()`), in its order.
const CREATURE_FAMILIES: [f64; 27] = [
    1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 11.0, 12.0, 15.0, 16.0, 17.0, 19.0, 20.0, 21.0,
    23.0, 24.0, 25.0, 26.0, 27.0, 28.0, 302.0, 310.0, 312.0, 319.0,
];

/// A list of numbers, in order.
fn number_list(vm: &mut Vm, list: impl Iterator<Item = f64>) -> Value {
    let t = vm.new_table();
    for (i, n) in list.enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(i as f64 + 1.0), Value::Num(n));
    }
    Value::Table(t)
}

fn creature_type_ids(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![number_list(vm, CREATURE_TYPES.iter().map(|t| t.0))])
}

/// `C_CreatureInfo.GetCreatureTypeInfo(creatureTypeID)`: its ID and name,
/// nil for one not listed (the full probe, 1.60.1: 1 and 1.5 Beast; -1, 0
/// and 2147483647 nil).
fn creature_type_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = arg_num(vm, &c, 1).map_or(0.0, f64::trunc);
    let Some(&(id, name)) = CREATURE_TYPES.iter().find(|t| t.0 == id) else {
        return Ok(vec![Value::Nil]);
    };
    let t = vm.new_table();
    let (k, v) = (vm.str("id"), Value::Num(id));
    let _ = vm.heap.table_mut(t).set(k, v);
    let (k, v) = (vm.str("name"), vm.str(name));
    let _ = vm.heap.table_mut(t).set(k, v);
    Ok(vec![Value::Table(t)])
}

fn creature_family_ids(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![number_list(vm, CREATURE_FAMILIES.iter().copied())])
}

/// `C_CreatureInfo.GetCreatureFamilyInfo(creatureFamilyID)`: the wolf's
/// (the full probe, 1.60.1: 1 and 1.5); nil for the others (their names
/// and icons unrecorded: unverified).
fn creature_family_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if arg_num(vm, &c, 1).map(f64::trunc) != Some(1.0) {
        return Ok(vec![Value::Nil]);
    }
    Ok(crate::recorded::parse_answer(
        vm,
        "1: {iconFile=number 132203,id=number 1,name=string Wolf}",
    ))
}

/// `C_VoiceChat.GetActiveChannelID()`, `C_QuestSession.
/// GetSessionBeginDetails()`: nothing while Blizzard's code loads (the
/// load calls, 1.60.1 70425), nil once logged in (the full probe, 70450;
/// when it changes is unverified: here at `PLAYER_LOGIN`).
fn nil_after_login(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(if engine(vm).world.logged_in {
        vec![Value::Nil]
    } else {
        vec![]
    })
}

/// `C_QuestSession.GetProposedMaxLevelForSession()`: the player's level,
/// out of a group (the full probe, 1.60.1: 21 for a level 21 character
/// alone; a group's lowest level unverified).
fn proposed_session_level(vm: &mut Vm, _c: NativeCall) -> Ret {
    let player = vm.str("player");
    let level = ask(vm, "UnitLevel", &[player]);
    Ok(vec![level.first().copied().unwrap_or(Value::Num(0.0))])
}

/// `C_PartyInfo.GetMinItemLevel(avgItemLevelCategory)`: out of a group,
/// the player's average item level, whole, and name (the full probe,
/// 1.60.1: categories 0 to 3 and "x" 11 for `GetAverageItemLevel()`'s
/// 11.25). Unverified: the weighted categories, 4 and 5 (the client
/// answered 68 and 73 by a formula not known; here the same 11), a
/// group's lowest.
fn min_item_level(vm: &mut Vm, _c: NativeCall) -> Ret {
    let player = vm.str("player");
    let average = ask(vm, "GetAverageItemLevel", &[]);
    let level = average
        .first()
        .copied()
        .and_then(num)
        .unwrap_or(0.0)
        .floor();
    let name = ask(vm, "UnitName", &[player]);
    let name = match name.first().copied() {
        Some(v @ Value::Str(_)) => v,
        _ => vm.str(""),
    };
    Ok(vec![Value::Num(level), name])
}

/// `C_UIFileAsset.GetFileID(asset)`: a file ID is its own (the full probe,
/// 1.60.1: 134400); a path's ID nil, the engine knowing no file's
/// (unverified for the client's files: `INV_Misc_QuestionMark`'s is
/// 134400).
fn file_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match vm.hidden(c.arg(1)) {
        Value::Num(n) => Value::Num(n),
        _ => Value::Nil,
    }])
}

/// A `PROBED_CONSTANT` function: its one answer.
fn probed_constant(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = crate::recorded::called_name(vm, &c).unwrap_or_default();
    let answer = PROBED_CONSTANT
        .iter()
        .find(|(n, _)| *n == name)
        .map_or("0: ", |(_, a)| *a);
    Ok(crate::recorded::parse_answer(vm, answer))
}

/// `C_ReportSystem.GetMajorCategoryString(category)`: the global string's
/// name (the full probe, 1.60.1: 0 to 5; a string reads as 0; unverified:
/// the categories past 5).
fn report_major_category_string(vm: &mut Vm, c: NativeCall) -> Ret {
    const NAMES: [&str; 6] = [
        "INAPPROPRIATE_COMMUNICATION",
        "GAMEPLAY_SABOTAGE",
        "CHEATING",
        "INAPPROPRIATE_NAME",
        "INAPPROPRIATE_DECOR",
        "INAPPROPRIATE_BLUEPRINT",
    ];
    let i = arg_num(vm, &c, 1).unwrap_or(0.0) as usize;
    let name = NAMES
        .get(i)
        .map_or(String::new(), |n| format!("REPORTING_MAJOR_CATEGORY_{n}"));
    Ok(vec![vm.str(&name)])
}

/// `C_ReportSystem.GetMinorCategoryString(category)`: the global string's
/// name of its lowest flag (the full probe, 1.60.1: every flag of
/// `Enum.ReportMinorCategory`; 1048577 named as 1).
fn report_minor_category_string(vm: &mut Vm, c: NativeCall) -> Ret {
    const NAMES: [&str; 21] = [
        "MINOR_CATEGORY_TEXT_CHAT",
        "MINOR_CATEGORY_BOOSTING",
        "MINOR_CATEGORY_SPAM",
        "MINOR_CATEGORY_AFK",
        "MINOR_CATEGORY_FEEDING",
        "MINOR_CATEGORY_BLOCKING_PROG",
        "MINOR_CATEGORY_HACKING",
        "MINOR_CATEGORY_BOTTING",
        "MINOR_CATEGORY_ADVERTISEMENT",
        "MINOR_CATEGORY_BTAG",
        "MINOR_CATEGORY_GROUP_NAME",
        "MINOR_CATEGORY_CHARACTER_NAME",
        "MINOR_CATEGORY_GUILD_NAME",
        "MINOR_CATEGORY_DESCRIPTION",
        "MINOR_CATEGORY_NAME",
        "MAJOR_CATEGORY_CHINA_HARMFUL_MINORS",
        "MINOR_CATEGORY_DISRUPTION",
        "MINOR_CATEGORY_TVEC",
        "MINOR_CATEGORY_CSEA",
        "MINOR_CATEGORY_NEIGHBORHOOD_NAME",
        "MINOR_CATEGORY_HOUSING_BLUEPRINT",
    ];
    let flags = arg_num(vm, &c, 1).unwrap_or(0.0) as u64;
    let name = (flags != 0)
        .then(|| NAMES.get(flags.trailing_zeros() as usize))
        .flatten()
        .map_or(String::new(), |n| format!("REPORTING_{n}"));
    Ok(vec![vm.str(&name)])
}

/// `C_ReportSystem.GetMajorCategoriesForReportType(reportType)`: the
/// client's list for each type (the full probe, 1.60.1: types 0 to 21; a
/// string reads as 0).
fn report_major_categories(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = arg_num(vm, &c, 1).unwrap_or(0.0) as usize;
    let cats: &[f64] = match t {
        0 | 2..=6 => &[0.0, 3.0],
        1 | 14 => &[3.0, 2.0, 1.0],
        7 => &[3.0, 0.0, 1.0],
        8 | 11 | 12 => &[3.0, 0.0],
        9 | 10 | 17 | 19 | 20 => &[3.0],
        13 => &[1.0],
        15 => &[3.0, 0.0, 2.0, 1.0],
        16 => &[0.0],
        18 => &[4.0, 3.0],
        21 => &[5.0],
        _ => &[],
    };
    let values: Vec<Value> = cats.iter().map(|&n| Value::Num(n)).collect();
    Ok(vec![list(vm, &values)])
}

/// `C_ReportSystem.GetMinorCategoriesForReportTypeAndMajorCategory(type,
/// major)`: the client's lists where the full probe (1.60.1) asked them
/// (major 0 for each type, and type 0's majors); none elsewhere
/// (unverified).
fn report_minor_categories(vm: &mut Vm, c: NativeCall) -> Ret {
    const CHAT: &[f64] = &[1.0, 4.0, 256.0, 131072.0, 262144.0];
    const PROFILE: &[f64] = &[256.0, 8192.0, 131072.0, 262144.0];
    let t = arg_num(vm, &c, 1).unwrap_or(0.0) as usize;
    let major = arg_num(vm, &c, 2).unwrap_or(0.0) as usize;
    let cats: &[f64] = match (t, major) {
        (0 | 6 | 7 | 8 | 12 | 15, 0) => CHAT,
        (2..=5 | 11, 0) => PROFILE,
        (16, 0) => &[1.0],
        (21, 0) => &[1048576.0],
        (0, 3) => &[2048.0, 512.0],
        _ => &[],
    };
    let values: Vec<Value> = cats.iter().map(|&n| Value::Num(n)).collect();
    Ok(vec![list(vm, &values)])
}

/// A chat line's ID: kept lines from 1 (the full probe, 1.60.1: line 1
/// valid with a sender, 0, 1.5 and 2147483647 not; unverified: how many
/// the client keeps, taken as 1000).
fn chat_line(vm: &Vm, c: &NativeCall) -> bool {
    arg_num(vm, c, 1).is_some_and(|n| n.fract() == 0.0 && (1.0..=1000.0).contains(&n))
}

/// `C_ChatInfo.IsValidChatLine(lineID)`.
fn is_valid_chat_line(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(chat_line(vm, &c))])
}

/// `C_ChatInfo.GetChatLineSenderGUID(lineID)`: nil for no line (the
/// sender the engine does not keep: empty).
fn chat_line_sender_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![if chat_line(vm, &c) {
        vm.str("")
    } else {
        Value::Nil
    }])
}

/// `C_LFGInfo.AreCrossFactionGroupQueuesAllowed(category)`,
/// `DoesCrossFactionQueueRequireFullPremade(category)`: a category from 1
/// (the full probe, 1.60.1: 1 and 1.5 true, -1, 0 and 2147483647 false;
/// unverified: the last category, taken as 20).
fn cross_faction_category(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    Ok(vec![Value::Bool((1.0..=20.0).contains(&n))])
}

/// `C_Club.GetClubLimits(clubType)`: 50 streams, none for type 3 (the full
/// probe, 1.60.1: 0 to 2 and a string 50, 3 none).
fn club_limits(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    let streams = if t == 3.0 { 0.0 } else { 50.0 };
    let r = vm.new_table();
    set(vm, r, "maximumNumberOfStreams", Value::Num(streams));
    Ok(vec![Value::Table(r)])
}

/// `C_Minimap.GetDefaultTrackingValue(filter)`: on for the filters the
/// minimap tracks by default (the full probe, 1.60.1: 0, 8, 32, 65536 and
/// 2097152 of `Enum.MinimapTrackingFilter`, a string as 0; the other
/// flags probed off).
fn default_tracking_value(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = arg_num(vm, &c, 1).unwrap_or(0.0);
    Ok(vec![Value::Bool(matches!(
        f as i64,
        0 | 8 | 32 | 65536 | 2097152
    ))])
}

/// `C_AccountStore.GetCategoryInfo(id)`, `GetCurrencyInfo(id)`: the ID
/// (truncated) with an empty name, a category of type 2, a currency of
/// none (the full probe, 1.60.1: every ID probed, -1 to 2147483647;
/// unverified: the store's own categories).
fn account_store_category(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    let t = vm.new_table();
    set(vm, t, "id", Value::Num(id));
    let name = vm.str("");
    set(vm, t, "name", name);
    set(vm, t, "type", Value::Num(2.0));
    Ok(vec![Value::Table(t)])
}

fn account_store_currency(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    let t = vm.new_table();
    set(vm, t, "id", Value::Num(id));
    let name = vm.str("");
    set(vm, t, "name", name);
    set(vm, t, "amount", Value::Num(0.0));
    Ok(vec![Value::Table(t)])
}

/// `C_Heirloom.GetHeirloomLink(itemID)`: an item link at the player's
/// level and specialization, the ID empty for 0 (the full probe, 1.60.1:
/// `|cnIQ1:|Hitem:1::::::::21:1486:::::::::|h[]|h|r` for 1, a level 21
/// paladin; unverified: an heirloom's own name and quality).
fn heirloom_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = arg_num(vm, &c, 1).unwrap_or(0.0).trunc() as i64;
    let id = if id == 0 {
        String::new()
    } else {
        id.to_string()
    };
    let level = player_level(vm) as i64;
    let spec_index = ask(vm, "C_SpecializationInfo.GetSpecialization", &[]);
    let spec = match spec_index.first().copied() {
        Some(i @ Value::Num(_)) => ask(vm, "C_SpecializationInfo.GetSpecializationInfo", &[i])
            .first()
            .copied()
            .and_then(num)
            .unwrap_or(0.0),
        _ => 0.0,
    } as i64;
    Ok(vec![vm.str(&format!(
        "|cnIQ1:|Hitem:{id}::::::::{level}:{spec}:::::::::|h[]|h|r"
    ))])
}

/// `C_ZoneAbility.GetZoneAbilityIcon(spellID)`: the spell's icon, 0 for
/// none (the full probe, 1.60.1: spell 1's 136235, 0, -1 and 2147483647
/// 0).
fn zone_ability_icon(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    let icon = if id > 0.0 {
        ask(vm, "C_Spell.GetSpellTexture", &[Value::Num(id)])
            .first()
            .copied()
            .and_then(num)
    } else {
        None
    };
    Ok(vec![Value::Num(icon.unwrap_or(0.0))])
}

/// `C_ContentTracking.GetTrackablesOnMap(type, uiMapID)`: as recorded;
/// else nothing to track, the result 0 on a map the client has and 2
/// (`Enum.ContentTrackingResult`'s failure) on one it lacks (the full
/// probe, 1.60.1: maps 1413 and 1414 0, maps -1 to 1 and 2147483647 2,
/// as the recorded 946's).
fn trackables_on_map(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(values) = crate::recorded::lookup(vm, "C_ContentTracking.GetTrackablesOnMap", &c) {
        return Ok(values);
    }
    let map = arg_num(vm, &c, 2).unwrap_or(0.0).trunc();
    let known = crate::recorded::lookup_text(vm, &format!("C_Map.GetMapInfo({})", map as i64))
        .is_some_and(|v| v.first().is_some_and(|v| !v.is_nil()));
    let empty = vm.new_table();
    Ok(vec![
        Value::Num(if known { 0.0 } else { 2.0 }),
        Value::Table(empty),
    ])
}

/// `C_EncounterWarnings.GetColorForSeverity(severity)`: magenta for every
/// severity (the full probe, 1.60.1: 0 to 2 and a string; unverified: an
/// edit mode setting that colors them).
fn warning_color(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![color(vm, (1.0, 0.0, 1.0))])
}

/// `C_EventToastManager.GetLevelUpDisplayToastsFromLevel(level)`: the
/// level-up toast, "Level <level>" (truncated) under "You've Reached" (the
/// full probe, 1.60.1: every level probed, -1 to 2147483647; unverified:
/// the toasts of a level's new spells).
fn level_up_toasts(vm: &mut Vm, c: NativeCall) -> Ret {
    let level = arg_num(vm, &c, 1).unwrap_or(0.0).trunc() as i64;
    let toast = vm.new_table();
    for (k, v) in [
        ("displayType", 2.0),
        ("eventToastID", 481.0),
        ("eventType", 0.0),
        ("flags", 0.0),
        ("sortOrder", 0.0),
    ] {
        set(vm, toast, k, Value::Num(v));
    }
    for (k, v) in [
        ("instructionText", String::new()),
        ("link", String::new()),
        ("subtitle", "You've Reached".to_string()),
        ("title", format!("Level {level}")),
    ] {
        let v = vm.str(&v);
        set(vm, toast, k, v);
    }
    Ok(vec![list(vm, &[Value::Table(toast)])])
}

/// `Enum.TransmogOutfitSlot`'s slots as the transmog atlases name them
/// (the full probe, 1.60.1: both shoulders "shoulders"; the ranged
/// gear slot's atlas spelled "gearslot").
const OUTFIT_SLOTS: [&str; 15] = [
    "head",
    "shoulders",
    "shoulders",
    "back",
    "chest",
    "tabard",
    "shirt",
    "wrist",
    "hands",
    "waist",
    "legs",
    "feet",
    "mainHand",
    "offHand",
    "ranged",
];

fn outfit_slot(vm: &Vm, c: &NativeCall) -> Option<usize> {
    let n = arg_num(vm, c, 1).unwrap_or(0.0).trunc();
    (n >= 0.0)
        .then_some(n as usize)
        .filter(|i| *i < OUTFIT_SLOTS.len())
}

/// `C_TransmogOutfitInfo.GetUnassignedAtlasForSlot(slot)`: the gear slot's
/// empty art (the full probe, 1.60.1: every slot, a string as 0;
/// unverified: past the ranged slot, nil).
fn unassigned_atlas(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match outfit_slot(vm, &c) {
        Some(14) => vm.str("transmog-gearslot-unassigned-ranged"),
        Some(i) => vm.str(&format!("transmog-gearSlot-unassigned-{}", OUTFIT_SLOTS[i])),
        None => Value::Nil,
    }])
}

/// `C_TransmogOutfitInfo.GetUnassignedDisplayAtlasForSlot(slot)`: the
/// appearance's empty art (as `GetUnassignedAtlasForSlot`).
fn unassigned_display_atlas(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match outfit_slot(vm, &c) {
        Some(i) => vm.str(&format!(
            "transmog-appearance-unassigned-{}",
            OUTFIT_SLOTS[i]
        )),
        None => Value::Nil,
    }])
}

/// `C_TransmogOutfitInfo.CanPlayerTransmogSlot(slot)`: every slot but the
/// ranged one (the full probe, 1.60.1, a paladin: 0 to 13 and a string
/// true, 14 false; unverified: a class with a ranged slot).
fn can_transmog_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        outfit_slot(vm, &c).is_some_and(|i| i < 14),
    )])
}

/// `C_TransmogOutfitInfo.SlotHasSecondary(slot)`: the shoulders (the full
/// probe, 1.60.1: slots 1 and 2).
fn slot_has_secondary(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(matches!(
        outfit_slot(vm, &c),
        Some(1 | 2)
    ))])
}

/// `C_TransmogOutfitInfo.GetMaxNumberOfTotalOutfitsForSource(source)`: 1,
/// 2 and 48 for sources 0 to 2 (the full probe, 1.60.1; a string as 0;
/// unverified: past 2, 0).
fn max_outfits_for_source(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    let max = match n as i64 {
        0 => 1.0,
        1 => 2.0,
        2 => 48.0,
        _ => 0.0,
    };
    Ok(vec![Value::Num(max)])
}

/// `C_TransmogOutfitInfo.GetIllusionDefaultIMAIDForCollectionType(type)`:
/// the weapon types' default illusion appearances, nothing for armor (the
/// full probe, 1.60.1: types 0 to 29, a string as 0).
fn illusion_default_imaid(vm: &mut Vm, c: NativeCall) -> Ret {
    const IDS: [f64; 17] = [
        28149.0, 18.0, 15.0, 17.0, 590.0, 6499.0, 746.0, 16155.0, 4385.0, 9212.0, 25771.0, 16.0,
        28166.0, 829.0, 833.0, 9213.0, 114255.0,
    ];
    let n = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    let id = (n >= 12.0)
        .then(|| IDS.get(n as usize - 12).copied())
        .flatten();
    Ok(id.map(|i| vec![Value::Num(i)]).unwrap_or_default())
}

/// `C_Club.ShouldAllowClubType(clubType)`: every type but 3 (the full
/// probe, 1.60.1: 0 to 2 and a string true, 3 false).
fn should_allow_club_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    Ok(vec![Value::Bool(t != 3.0)])
}

/// `C_Club.GetAvatarIdList(clubType)`: as recorded (type 1's avatars),
/// none for types 2 and 3, nil for another (the full probe, 1.60.1: 0 and
/// a string nil).
fn club_avatar_ids(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(values) = crate::recorded::lookup(vm, "C_Club.GetAvatarIdList", &c) {
        return Ok(values);
    }
    let t = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    Ok(vec![if t == 2.0 || t == 3.0 {
        Value::Table(vm.new_table())
    } else {
        Value::Nil
    }])
}

/// The inventory slot each `Enum.TransmogOutfitSlot` dresses.
const OUTFIT_INVENTORY: [f64; 15] = [
    1.0, 3.0, 3.0, 15.0, 5.0, 19.0, 4.0, 9.0, 10.0, 6.0, 7.0, 8.0, 16.0, 17.0, 18.0,
];

/// `C_TransmogOutfitInfo.GetViewedOutfitSlotInfo(slot, type, option)`: the
/// slot as nothing transmogrified shows it, with the warning that no item
/// is equipped there; an illusion (type 1) not transmogrifiable, error 12
/// (the full probe, 1.60.1: the character's empty slots warned, the
/// others not, every option alike; unverified: an item that cannot be
/// transmogrified).
fn viewed_outfit_slot_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(slot) = outfit_slot(vm, &c) else {
        return Ok(vec![]);
    };
    let illusion = arg_num(vm, &c, 2).unwrap_or(0.0).trunc() == 1.0;
    let player = vm.str("player");
    let equipped = !ask(
        vm,
        "GetInventoryItemID",
        &[player, Value::Num(OUTFIT_INVENTORY[slot])],
    )
    .first()
    .is_none_or(|v| v.is_nil());
    let t = vm.new_table();
    for (k, v) in [
        ("canTransmogrify", Value::Bool(!illusion)),
        ("displayType", Value::Num(0.0)),
        ("error", Value::Num(if illusion { 12.0 } else { 0.0 })),
        ("hasPending", Value::Bool(false)),
        ("isPendingCollected", Value::Bool(false)),
        ("isTransmogrified", Value::Bool(false)),
        ("sheatheCategory", Value::Num(0.0)),
        ("transmogID", Value::Num(0.0)),
        ("warning", Value::Num(if equipped { 0.0 } else { 7.0 })),
    ] {
        set(vm, t, k, v);
    }
    let error_text = vm.str("");
    set(vm, t, "errorText", error_text);
    let warning = vm.str(if equipped {
        ""
    } else {
        "There is no equipped item in this slot that can be transmogrified."
    });
    set(vm, t, "warningText", warning);
    Ok(vec![Value::Table(t)])
}

/// `C_TransmogOutfitInfo.GetEquippedSlotOptionFromTransmogSlot(slot)`:
/// the option the item equipped there wears, `Enum.TransmogOutfitSlotOption`
/// (the full probe, 1.60.1, a paladin in mail: mail 14 in the armor slots,
/// 0 for the cloak, the shirt and an empty slot, 1 for a one-handed main
/// hand; unverified: cloth 12, leather 13, plate 15, two-handed 2, a
/// shield 5 and a held item 4, after `GetOptionsForSlot`'s types).
fn equipped_slot_option(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(slot) = outfit_slot(vm, &c) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    let player = vm.str("player");
    let id = ask(
        vm,
        "GetInventoryItemID",
        &[player, Value::Num(OUTFIT_INVENTORY[slot])],
    )
    .first()
    .copied()
    .and_then(num);
    let Some(data) = id.and_then(|id| crate::gamedata::item_data(vm, id as i64)) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    let class = num(field(vm, data, "classID")).unwrap_or(-1.0) as i64;
    let sub = num(field(vm, data, "subclassID")).unwrap_or(-1.0) as i64;
    let equip = match field(vm, data, "equipLoc") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    let option = match (class, sub) {
        _ if matches!(slot, 3 | 5 | 6) => 0,
        (4, 1..=4) => 11 + sub,
        (4, 6) => 5,
        _ if equip == "INVTYPE_HOLDABLE" => 4,
        (2, _) if equip == "INVTYPE_2HWEAPON" => 2,
        (2, _) => 1,
        _ => 0,
    };
    Ok(vec![Value::Num(option as f64)])
}

/// `C_PvP.GetBrawlRewards(brawlType)`: no rewards for a brawl type from 1,
/// nothing for 0 (the full probe, 1.60.1: types 1 to 5; 0 and a string
/// nothing).
fn brawl_rewards(vm: &mut Vm, c: NativeCall) -> Ret {
    if arg_num(vm, &c, 1).unwrap_or(0.0) < 1.0 {
        return Ok(vec![]);
    }
    Ok(vec![
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Nil,
        Value::Nil,
        Value::Nil,
        Value::Bool(false),
    ])
}

/// `C_PvP.GetArenaCrowdControlDuration(unit)`: a zero duration outside an
/// arena (the full probe, 1.60.1: every unit token and the player's GUID).
fn arena_crowd_control_duration(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(ask(vm, "C_DurationUtil.CreateDuration", &[]))
}

/// `C_EncounterWarnings.GetSoundKitForSeverity(severity)`: the sound kits
/// of `Enum.EncounterEventSeverity` 0 to 2 (the full probe, 1.60.1; a
/// string as 0; unverified: past 2).
fn warning_sound_kit(vm: &mut Vm, c: NativeCall) -> Ret {
    const KITS: [f64; 3] = [297258.0, 297255.0, 297253.0];
    let i = arg_num(vm, &c, 1).unwrap_or(0.0);
    let kit = KITS.get(i as usize).filter(|_| i >= 0.0).copied();
    Ok(vec![Value::Num(kit.unwrap_or(0.0))])
}

/// `C_Minimap.GetPOITextureCoords(index)`: the cell of the POI sheet, 18
/// pixels square on a 256 by 512 texture, a pixel's border inside (the
/// full probe, 1.60.1: index 0 and none 1/256 to 18/256 by 1/512 to
/// 18/512, index 1 the next cell; unverified: the rows, 14 cells each).
fn poi_texture_coords(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    // Past the sheet (28 rows): zeros (the full probe, 1.60.1: -1 and
    // 2147483647).
    if !(0.0..392.0).contains(&i) {
        return Ok(vec![Value::Num(0.0); 4]);
    }
    let i = i as i64;
    let (col, row) = ((i % 14) as f64, (i / 14) as f64);
    Ok(vec![
        Value::Num((col * 18.0 + 1.0) / 256.0),
        Value::Num((col * 18.0 + 18.0) / 256.0),
        Value::Num((row * 18.0 + 1.0) / 512.0),
        Value::Num((row * 18.0 + 18.0) / 512.0),
    ])
}

/// `C_Macro.GetSelectedMacroIcon(index)`: the macro's icon, the question
/// mark (134400) for none (the full probe, 1.60.1: 1.5 the account's
/// first macro's 841219, 0 and 2147483647 the question mark).
fn selected_macro_icon(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = arg_num(vm, &c, 1).unwrap_or(0.0).trunc();
    let info = ask(vm, "GetMacroInfo", &[Value::Num(i)]);
    let icon = info.get(1).copied().and_then(num).unwrap_or(134400.0);
    Ok(vec![Value::Num(icon)])
}

/// `C_EncounterJournal.GetEncounterJournalLink(type, id, name,
/// difficulty)`: the journal link (the full probe, 1.60.1: numbers
/// truncated, a string as 0, difficulty 2147483647 written -1, as the
/// client keeps it in 16 bits; unverified: the color for other types).
fn encounter_journal_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = |i: usize| arg_num(vm, &c, i).unwrap_or(0.0).trunc() as i64;
    let (kind, id, difficulty) = (n(1), n(2), n(4) as i16);
    let name = arg_text(vm, &c, 3).unwrap_or_default();
    Ok(vec![vm.str(&format!(
        "|cff66bbff|Hjournal:{kind}:{id}:{difficulty}|h[{name}]|h|r"
    ))])
}

// ------------------------------------------------------- dates and texts

/// A Unix time's UTC date and time: year, month (1..12), day, weekday
/// (0 Sunday), hour, minute, second.
fn utc_parts(t: i64) -> (i64, usize, i64, usize, i64, i64, i64) {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let weekday = (days + 4).rem_euclid(7) as usize;
    (
        y,
        m as usize,
        d,
        weekday,
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
    )
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// A date in an `Enum.DateTimeStyle` (1 short, 2 medium, 3 long, 4 full),
/// en-US as ICU writes it; none for 0.
fn date_text(t: i64, style: i64) -> Option<String> {
    let (y, m, d, wd, ..) = utc_parts(t);
    let month = MONTHS[m - 1];
    Some(match style {
        1 => format!("{m}/{d}/{:02}", y.rem_euclid(100)),
        2 => format!("{} {d}, {y}", &month[..3]),
        3 => format!("{month} {d}, {y}"),
        4 => format!("{}, {month} {d}, {y}", WEEKDAYS[wd]),
        _ => return None,
    })
}

/// A time in an `Enum.DateTimeStyle`, en-US as ICU writes it (a narrow
/// no-break space before AM and PM), in UTC; none for 0.
fn time_text(t: i64, style: i64) -> Option<String> {
    let (.., h, min, s) = utc_parts(t);
    let (h12, half) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    Some(match style {
        1 => format!("{h12}:{min:02}\u{202f}{half}"),
        2 => format!("{h12}:{min:02}:{s:02}\u{202f}{half}"),
        3 => format!("{h12}:{min:02}:{s:02}\u{202f}{half} GMT+0"),
        4 => format!("{h12}:{min:02}:{s:02}\u{202f}{half} GMT+00:00"),
        _ => return None,
    })
}

fn style_arg(vm: &Vm, c: &NativeCall, n: usize) -> i64 {
    arg_num(vm, c, n).unwrap_or(0.0) as i64
}

fn text_or_nothing(vm: &mut Vm, text: Option<String>) -> Ret {
    Ok(text.map(|t| vec![vm.str(&t)]).unwrap_or_default())
}

// `C_Intl.FormatDate`, `FormatTime`, `FormatDateTime`: en-US, the time
// zone given read as UTC (the full probe, 1.60.1: time 1 in each style
// with zone "x": "1/1/70", "Jan 1, 1970", "January 1, 1970", "Thursday,
// January 1, 1970", "12:00 AM", "12:00:01 AM", "… GMT+0", "… GMT+00:00";
// style 0 or a string nothing). Unverified: a real time zone, both styles
// at once (joined by ", " as ICU joins them).

fn intl_format_date(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = arg_num(vm, &c, 1).unwrap_or(0.0).floor() as i64;
    let text = date_text(t, style_arg(vm, &c, 2));
    text_or_nothing(vm, text)
}

fn intl_format_time(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = arg_num(vm, &c, 1).unwrap_or(0.0).floor() as i64;
    let text = time_text(t, style_arg(vm, &c, 2));
    text_or_nothing(vm, text)
}

fn intl_format_date_time(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = arg_num(vm, &c, 1).unwrap_or(0.0).floor() as i64;
    let date = date_text(t, style_arg(vm, &c, 2));
    let time = time_text(t, style_arg(vm, &c, 3));
    let text = match (date, time) {
        (Some(d), Some(t)) => Some(format!("{d}, {t}")),
        (d, t) => d.or(t),
    };
    text_or_nothing(vm, text)
}

/// `C_Intl.GetCurrencyName(code, style)`: the code itself for a code ICU
/// does not know, nothing for none (the full probe, 1.60.1: "x" in every
/// style; unverified: a real currency's name).
fn intl_get_currency_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let code = arg_text(vm, &c, 1).unwrap_or_default();
    if code.is_empty() {
        return Ok(vec![]);
    }
    Ok(vec![vm.str(&code)])
}

/// `C_Intl.GetDisplayName(displayLocale)`: the client's locale named in
/// that locale: English (United States), "en (US)" in the root locale ""
/// (the full probe, 1.60.1; unverified: other display locales).
fn intl_get_display_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let shown = arg_text(vm, &c, 1).unwrap_or_default();
    let name = if shown.is_empty() {
        "en (US)"
    } else {
        "English (United States)"
    };
    Ok(vec![vm.str(name)])
}

/// `C_Ping.GetTextureKitForType(type)`: `Enum.PingSubjectType`'s kits (the
/// full probe, 1.60.1: 0 to 8, a string as 0; nil past them).
fn ping_texture_kit(vm: &mut Vm, c: NativeCall) -> Ret {
    const KITS: [&str; 9] = [
        "Attack",
        "Warning",
        "Assist",
        "OnMyWay",
        "Threat",
        "NonThreat",
        "Available",
        "Unavailable",
        "Unavailable",
    ];
    let t = arg_num(vm, &c, 1).unwrap_or(0.0);
    Ok(vec![match KITS.get(t as usize).filter(|_| t >= 0.0) {
        Some(k) => vm.str(k),
        None => Value::Nil,
    }])
}

/// `C_ScenarioInfo.GetJailersTowerTypeString(type)`: Torghast's wings by
/// `Enum.JailersTowerType` (the full probe, 1.60.1: 0 to 14, a string as
/// 0; unverified: past 14).
fn jailers_tower_type_string(vm: &mut Vm, c: NativeCall) -> Ret {
    const WINGS: [&str; 15] = [
        "Twisting Corridors",
        "Skoldus Hall",
        "Fracture Chambers",
        "The Soulforges",
        "Coldheart Interstitia",
        "Mort'regar",
        "Upper Reaches",
        "Arkoban Hall",
        "Torment Chamber: Jaina",
        "Torment Chamber: Thrall",
        "Torment Chamber: Anduin",
        "Adamant Vaults",
        "Fervid Catacombs",
        "",
        "The Jailer's Gauntlet",
    ];
    let t = arg_num(vm, &c, 1).unwrap_or(0.0);
    Ok(vec![match WINGS.get(t as usize).filter(|_| t >= 0.0) {
        Some(w) => vm.str(w),
        None => Value::Nil,
    }])
}

// --------------------------------------------------------- text by ICU

/// `C_Intl.FindBreaks(text, breakType)`: the 1-based byte offsets of the
/// breaks, the text's start and end among them: characters (0), words (1:
/// a run of letters and digits, each other character alone), lines (2:
/// after spaces), sentences (3) (the full probe, 1.60.1: "Hello" and "x"
/// in each, a color code's words at 1, 2, 14, 15 and 16, "" {1};
/// unverified: sentences, which split the color code at 2 and 15, and
/// text past ASCII).
fn intl_find_breaks(vm: &mut Vm, c: NativeCall) -> Ret {
    let text = arg_text(vm, &c, 1).unwrap_or_default();
    let kind = style_arg(vm, &c, 2);
    let end = text.len() + 1;
    let mut breaks: Vec<usize> = vec![1];
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    match kind {
        0 => breaks.extend(chars.iter().skip(1).map(|(i, _)| i + 1)),
        1 => {
            let word = |ch: char| ch.is_alphanumeric();
            for w in chars.windows(2) {
                let ((_, a), (i, b)) = (w[0], w[1]);
                if !(word(a) && word(b)) {
                    breaks.push(i + 1);
                }
            }
        }
        2 => {
            for w in chars.windows(2) {
                let ((_, a), (i, b)) = (w[0], w[1]);
                if a.is_whitespace() && !b.is_whitespace() {
                    breaks.push(i + 1);
                }
            }
        }
        // Sentences: after a bar (the full probe, 1.60.1: a color code's
        // at 2 and 15), and after a sentence's end and its spaces
        // (unverified).
        3 => {
            for w in chars.windows(2) {
                let ((_, a), (i, b)) = (w[0], w[1]);
                let ended = text[..i].trim_end().ends_with(['.', '!', '?']);
                if a == '|' || (a.is_whitespace() && !b.is_whitespace() && ended) {
                    breaks.push(i + 1);
                }
            }
        }
        _ => {}
    }
    if !text.is_empty() {
        breaks.push(end);
    }
    let values: Vec<Value> = breaks.into_iter().map(|b| Value::Num(b as f64)).collect();
    Ok(vec![list(vm, &values)])
}

/// `C_Intl.GetSortKey(text, strength)`: ICU's sort key of the text, its
/// levels by the strength (0 primary, 1 secondary, 2 tertiary, 3
/// quaternary, 4 identical), each after a 1 byte (the full probe, 1.60.1:
/// "", "x", "Hello" and "|cffff0000Red|r" at every strength; "x" as a
/// strength reads 0). Primary weights: a letter one byte from 0x2B in
/// steps of 2, either case; the digit 0 0x12; the bar 0x0C 0x8D (the
/// probed characters). Unverified: the digits 1 to 9 (0x12 on in steps of
/// 2), every other character (here 0x0C and a byte after the bar's by
/// its code), accents. Secondary and tertiary weights are common but a
/// capital's tertiary 0xDC, common runs compressed as ICU does (a run
/// before a higher weight counted down from 0xC5, one at the end up from
/// 0x05); the quaternary level is empty; the identical level is the
/// text's BOCSU.
fn intl_get_sort_key(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(text) = arg_text(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let strength = style_arg(vm, &c, 2);
    let mut key: Vec<u8> = Vec::new();
    let mut upper: Vec<bool> = Vec::new();
    for ch in text.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' => key.push(0x2B + 2 * (ch.to_ascii_lowercase() as u8 - b'a')),
            '0'..='9' => key.push(0x12 + 2 * (ch as u8 - b'0')),
            '|' => key.extend([0x0C, 0x8D]),
            other => key.extend([0x0C, 0x11u32.wrapping_add(u32::from(other)) as u8]),
        }
        upper.push(ch.is_uppercase());
    }
    let n = upper.len();
    if strength >= 1 {
        key.push(0x01);
        compress_end(&mut key, n);
    }
    if strength >= 2 {
        key.push(0x01);
        let mut run = 0;
        for &up in &upper {
            if up {
                // A run before a higher weight, counted down from the top.
                let mut left = run;
                while left > 0 {
                    let k = left.min(0x3F);
                    key.push((0xC5 - (k - 1)) as u8);
                    left -= k;
                }
                run = 0;
                key.push(0xDC);
            } else {
                run += 1;
            }
        }
        compress_end(&mut key, run);
    }
    if strength >= 3 {
        key.push(0x01);
    }
    if strength >= 4 {
        key.push(0x01);
        bocsu(&mut key, &text);
    }
    Ok(vec![vm.bytes(&key)])
}

/// A run of common weights at a level's end: counted up from 0x05.
fn compress_end(key: &mut Vec<u8>, mut run: usize) {
    while run > 0 {
        let k = run.min(0x21);
        key.push((0x05 + (k - 1)) as u8);
        run -= k;
    }
}

/// ICU's BOCSU of a text (its identical level): each code point as its
/// difference from the previous one's block middle; the single and
/// double byte ranges (ASCII's single, the probe's; longer differences
/// unverified).
fn bocsu(key: &mut Vec<u8>, text: &str) {
    let mut prev: i64 = 0;
    for ch in text.chars() {
        let c = i64::from(u32::from(ch));
        if !(0x4E00..0xA000).contains(&prev) {
            prev = (prev & !0x7F) + 80;
        } else {
            prev = 0x9FFF - 10667;
        }
        let diff = c - prev;
        if (-80..=80).contains(&diff) {
            key.push((0x81 + diff) as u8);
        } else if (81..=10667).contains(&diff) {
            key.push((0xD2 + diff / 253) as u8);
            key.push((3 + diff % 253) as u8);
        } else if (-10668..=-81).contains(&diff) {
            let (q, m) = (diff.div_euclid(253), diff.rem_euclid(253));
            key.push((0x31 + q) as u8);
            key.push((3 + m) as u8);
        } else {
            for b in c.to_be_bytes().iter().skip(5) {
                key.push(*b);
            }
        }
        prev = c;
    }
}

/// `C_Intl.FindStringMatches(text, pattern, strength)`: the 1-based byte
/// offsets where the pattern starts; nothing for an empty text or pattern
/// (the full probe, 1.60.1: "x" in "x" {1}, in "Hello" none; unverified:
/// the strengths, case ignored below 2 as ICU's primary and secondary).
fn intl_find_string_matches(vm: &mut Vm, c: NativeCall) -> Ret {
    let text = arg_text(vm, &c, 1).unwrap_or_default();
    let pattern = arg_text(vm, &c, 2).unwrap_or_default();
    if text.is_empty() || pattern.is_empty() {
        return Ok(vec![]);
    }
    let (text, pattern) = if style_arg(vm, &c, 3) < 2 {
        (text.to_lowercase(), pattern.to_lowercase())
    } else {
        (text, pattern)
    };
    let values: Vec<Value> = text
        .match_indices(&pattern)
        .map(|(i, _)| Value::Num(i as f64 + 1.0))
        .collect();
    Ok(vec![list(vm, &values)])
}

/// `C_Intl.GetCharacterProperties(text)`: the first character's Unicode
/// properties (the full probe, 1.60.1: "H" Lu, "x" Ll, both Latin, "|" Sm
/// common, each in the ASCII block; nothing for ""; unverified: past
/// ASCII, named by the character's own kind).
fn intl_character_properties(vm: &mut Vm, c: NativeCall) -> Ret {
    let text = arg_text(vm, &c, 1).unwrap_or_default();
    let Some(ch) = text.chars().next() else {
        return Ok(vec![]);
    };
    let category = match ch {
        'A'..='Z' => "Lu",
        'a'..='z' => "Ll",
        '0'..='9' => "Nd",
        ' ' => "Zs",
        '$' => "Sc",
        '(' | '[' | '{' => "Ps",
        ')' | ']' | '}' => "Pe",
        '+' | '<' | '=' | '>' | '|' | '~' => "Sm",
        '-' => "Pd",
        '^' | '`' => "Sk",
        '_' => "Pc",
        c if c.is_control() => "Cc",
        c if c.is_ascii_punctuation() => "Po",
        c if c.is_uppercase() => "Lu",
        c if c.is_lowercase() => "Ll",
        c if c.is_numeric() => "Nd",
        c if c.is_whitespace() => "Zs",
        _ => "Lo",
    };
    let latin = ch.is_ascii_alphabetic();
    let t = vm.new_table();
    let block = vm.str(if ch.is_ascii() { "ASCII" } else { "Unknown" });
    set(vm, t, "blockCode", block);
    set(vm, t, "codePoint", Value::Num(f64::from(u32::from(ch))));
    let category = vm.str(category);
    set(vm, t, "generalCategory", category);
    set(vm, t, "isAlphabetic", Value::Bool(ch.is_alphabetic()));
    set(vm, t, "isDigit", Value::Bool(ch.is_ascii_digit()));
    set(vm, t, "isWhitespace", Value::Bool(ch.is_whitespace()));
    let script = vm.str(if latin { "Latn" } else { "Zyyy" });
    set(vm, t, "scriptCode", script);
    Ok(vec![Value::Table(t)])
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
            "C_ChatInfo.CanPlayerSpeakLanguage",
            can_player_speak_language,
        ),
        ("DeclineName", decline_name),
        // The client.
        ("GetDownloadedPercentage", get_downloaded_percentage),
        ("GetNetIpTypes", get_net_ip_types),
        ("GetProtocolTypes", get_protocol_types),
        ("GetNumExpansions", get_num_expansions),
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
    list.extend(
        PROBED_CONSTANT
            .iter()
            .map(|&(n, _)| (n, probed_constant as NativeFn)),
    );
    list.extend([
        (
            "C_ClubFinder.GetFocusIndexFromFlag",
            club_finder_focus_index as NativeFn,
        ),
        (
            "C_ClubFinder.IsListingEnabledFromFlags",
            club_finder_listing_enabled,
        ),
        (
            "C_Club.GetCommunityNameResultText",
            community_name_result_text,
        ),
        ("C_CreatureInfo.GetCreatureTypeIDs", creature_type_ids),
        ("C_CreatureInfo.GetCreatureTypeInfo", creature_type_info),
        ("C_CreatureInfo.GetCreatureFamilyIDs", creature_family_ids),
        ("C_CreatureInfo.GetCreatureFamilyInfo", creature_family_info),
        ("C_UIFileAsset.GetFileID", file_id),
        ("C_PartyInfo.GetMinItemLevel", min_item_level),
        (
            "C_QuestSession.GetProposedMaxLevelForSession",
            proposed_session_level,
        ),
        ("C_VoiceChat.GetActiveChannelID", nil_after_login),
        ("C_QuestSession.GetSessionBeginDetails", nil_after_login),
        ("C_Debug.GetMapDebugObjects", debug_map_objects),
        ("C_Debug.GetAllPortLocsForMap", debug_map_objects),
        (
            "C_ReportSystem.GetMajorCategoryString",
            report_major_category_string,
        ),
        (
            "C_ReportSystem.GetMinorCategoryString",
            report_minor_category_string,
        ),
        (
            "C_ReportSystem.GetMajorCategoriesForReportType",
            report_major_categories,
        ),
        (
            "C_ReportSystem.GetMinorCategoriesForReportTypeAndMajorCategory",
            report_minor_categories,
        ),
        ("C_Intl.FormatDate", intl_format_date),
        ("C_Intl.FormatTime", intl_format_time),
        ("C_Intl.FormatDateTime", intl_format_date_time),
        ("C_Intl.GetCurrencyName", intl_get_currency_name),
        ("C_Intl.GetDisplayName", intl_get_display_name),
        ("C_Intl.FindBreaks", intl_find_breaks),
        ("C_Intl.GetSortKey", intl_get_sort_key),
        ("C_Intl.FindStringMatches", intl_find_string_matches),
        ("C_Intl.GetCharacterProperties", intl_character_properties),
        ("C_Ping.GetTextureKitForType", ping_texture_kit),
        (
            "C_EncounterWarnings.GetSoundKitForSeverity",
            warning_sound_kit,
        ),
        ("C_Minimap.GetPOITextureCoords", poi_texture_coords),
        ("C_AccountStore.GetCategoryInfo", account_store_category),
        ("C_AccountStore.GetCurrencyInfo", account_store_currency),
        ("C_Heirloom.GetHeirloomLink", heirloom_link),
        ("C_ZoneAbility.GetZoneAbilityIcon", zone_ability_icon),
        ("C_PvP.GetBrawlRewards", brawl_rewards),
        ("C_Club.ShouldAllowClubType", should_allow_club_type),
        ("C_Club.GetAvatarIdList", club_avatar_ids),
        (
            "C_TransmogOutfitInfo.GetUnassignedAtlasForSlot",
            unassigned_atlas,
        ),
        (
            "C_TransmogOutfitInfo.GetUnassignedDisplayAtlasForSlot",
            unassigned_display_atlas,
        ),
        (
            "C_TransmogOutfitInfo.CanPlayerTransmogSlot",
            can_transmog_slot,
        ),
        ("C_TransmogOutfitInfo.SlotHasSecondary", slot_has_secondary),
        (
            "C_TransmogOutfitInfo.GetEquippedSlotOptionFromTransmogSlot",
            equipped_slot_option,
        ),
        (
            "C_TransmogOutfitInfo.GetViewedOutfitSlotInfo",
            viewed_outfit_slot_info,
        ),
        (
            "C_TransmogOutfitInfo.GetMaxNumberOfTotalOutfitsForSource",
            max_outfits_for_source,
        ),
        (
            "C_TransmogOutfitInfo.GetIllusionDefaultIMAIDForCollectionType",
            illusion_default_imaid,
        ),
        (
            "C_EventToastManager.GetLevelUpDisplayToastsFromLevel",
            level_up_toasts,
        ),
        ("C_EncounterWarnings.GetColorForSeverity", warning_color),
        ("C_ContentTracking.GetTrackablesOnMap", trackables_on_map),
        (
            "C_PvP.GetArenaCrowdControlDuration",
            arena_crowd_control_duration,
        ),
        ("C_ChatInfo.IsValidChatLine", is_valid_chat_line),
        ("C_ChatInfo.GetChatLineSenderGUID", chat_line_sender_guid),
        (
            "C_LFGInfo.AreCrossFactionGroupQueuesAllowed",
            cross_faction_category,
        ),
        (
            "C_LFGInfo.DoesCrossFactionQueueRequireFullPremade",
            cross_faction_category,
        ),
        ("C_Club.GetClubLimits", club_limits),
        ("C_Minimap.GetDefaultTrackingValue", default_tracking_value),
        ("C_Macro.GetSelectedMacroIcon", selected_macro_icon),
        (
            "C_EncounterJournal.GetEncounterJournalLink",
            encounter_journal_link,
        ),
        (
            "C_ScenarioInfo.GetJailersTowerTypeString",
            jailers_tower_type_string,
        ),
    ]);
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
