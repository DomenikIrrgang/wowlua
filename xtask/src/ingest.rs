//! `cargo xtask ingest <SavedVariables/StdTest.lua>`: a client's StdTest
//! recordings into the repository's fixtures and data, scrubbed of the
//! player (name, last name and GUID; any player's GUID; the social systems'
//! texts), so a new client is checked by running the tests and reading the
//! diff:
//!
//! - `/stdtest engine` (and order, protected, blocked):
//!   `crates/wowlua-engine/tests/client/forever-camelot-engine.lua`;
//! - `/stdtest engine input`: `…/forever-camelot-input.lua`;
//! - `/stdtest engine answers`: `…/forever-camelot-answers.lua` (its checks
//!   `…/forever-camelot-checks.lua`);
//! - `/stdtest engine maps`: `…/forever-camelot-maps.lua` (and the data);
//! - `/stdtest engine cursor`: `…/forever-camelot-cursor.lua`;
//! - `/stdtest engine collections`: `…/forever-camelot-collections.lua`
//!   (and the data);
//! - the default character (a fresh level 1 Human Warrior's `/stdtest
//!   snapshot`): `cargo xtask profile <its StdTest.lua>
//!   crates/wowlua-engine/data/camelot-default-character.json`;
//! - `/stdtest snapshot`: the character's world as a test profile,
//!   `…/forever-camelot-character.json`;
//! - the CVar flags: `crates/wowlua-engine/data/camelot-cvar-flags.txt`;
//! - the font heights: `crates/wowlua-engine/data/camelot-font-heights.txt`;
//! - the static functions' answers: `crates/wowlua-engine/data/camelot-answers.txt`;
//! - the login: `crates/wowlua-engine/data/camelot-login.txt` (and reload).
//!
//! Each part is written only when the file has it.

use anyhow::{Context, Result, anyhow};
use std::fmt::Write;
use std::path::Path;

/// The player as the file names them.
struct Identity {
    name: String,
    last_name: Option<String>,
    guid: String,
}

/// Functions whose answers are the player's social life: their strings are
/// blanked (the structure stays).
const SOCIAL: &[&str] = &[
    "C_Club.",
    "C_ClubFinder.",
    "C_GuildInfo.",
    "GetGuild",
    "BNGet",
    "C_BattleNet.",
    "C_FriendList.",
    "C_DamageMeter.",
    "C_RecentAllies.",
    "C_SocialQueue.",
    "GetFriendInfo",
    "GetWhoInfo",
];

impl Identity {
    fn scrub(&self, text: &str) -> String {
        // The session's addresses (`function: 0x…`).
        let address = regex::Regex::new(r"(function|table|userdata|thread): 0x[0-9a-f]+")
            .expect("a valid pattern");
        let text = address.replace_all(text, "$1");
        let mut out = text.replace(&self.guid, "Player-0000-00000000");
        // Any other player's GUID.
        let guid = regex::Regex::new(r"Player-\d+-[0-9A-F]{8}").expect("a valid pattern");
        out = guid.replace_all(&out, "Player-0000-00000000").into_owned();
        out = out.replace(&self.name, "Player");
        if let Some(last) = &self.last_name {
            out = out.replace(last, "Lastname");
        }
        out
    }
}

/// A social function's answer with its strings blanked.
fn blank_strings(text: &str) -> String {
    let s = regex::Regex::new(r"string [^,}]*").expect("a valid pattern");
    s.replace_all(text, "string <scrubbed>").into_owned()
}

/// A Lua string literal (ASCII; others as `\ddd`).
fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for b in text.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            32..=126 => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03}");
            }
        }
    }
    out.push('"');
    out
}

/// `StdTestState`'s tables flattened as the fixtures are (`a.b.c` keys,
/// `tostring` values), by a Lua function given the state.
fn flatten(lua: &mlua::Lua, state: &mlua::Table, script: &str) -> Result<Vec<(String, String)>> {
    let f: mlua::Function = lua.load(script).eval().map_err(|e| anyhow!("{e}"))?;
    let t: mlua::Table = f.call(state.clone()).map_err(|e| anyhow!("{e}"))?;
    let mut out = Vec::new();
    for pair in t.pairs::<String, String>() {
        out.push(pair.map_err(|e| anyhow!("{e}"))?);
    }
    out.sort();
    Ok(out)
}

const FLATTEN: &str = r#"return function(s)
    local out = {}
    local function dump(v, prefix)
        if type(v) ~= "table" then out[prefix] = tostring(v) return end
        for k, inner in pairs(v) do dump(inner, prefix == "" and tostring(k) or (prefix .. "." .. tostring(k))) end
    end
    if s.engineProbe then dump(s.engineProbe, "probe") end
    if s.engineOrder then dump(s.engineOrder, "order") end
    if s.engineProtected then dump(s.engineProtected, "protected") end
    if s.engineBlocked then dump(s.engineBlocked, "blocked") end
    return out
end"#;

fn write_table(path: &Path, header: &str, entries: &[(String, String)]) -> Result<()> {
    let mut out = String::new();
    for line in header.lines() {
        let _ = writeln!(out, "-- {line}");
    }
    out.push_str("return {\n");
    for (k, v) in entries {
        let _ = writeln!(out, "    [{}] = {},", quote(k), quote(v));
    }
    out.push_str("}\n");
    std::fs::write(path, out).with_context(|| path.display().to_string())?;
    println!("wrote {} ({} entries)", path.display(), entries.len());
    Ok(())
}

/// Functions that answer from the client's own data or the account's
/// settings (the same whatever the character): their recorded answers are
/// the engine's.
const STATIC: &[&str] = &[
    // Game data.
    "GetChatTypeIndex",
    "C_Map.",
    "C_GameRules.",
    "GetBuildOption",
    "C_ClassColor.GetClassColor",
    "C_CreatureInfo.GetClassInfo",
    "LocalizedClassList",
    "GetSpecializationInfoForClassID",
    "C_SpecializationInfo.GetNumSpecializationsForClassID",
    "C_TradeSkillUI.GetProfessionSkillLineID",
    "C_Spell.GetSchoolString",
    // Spell data (the spells Blizzard's UI names; a test's own spells are
    // defined by the test).
    "C_Spell.GetSpellInfo",
    "C_Spell.GetSpellName",
    "C_Spell.GetSpellTexture",
    "C_Spell.GetSpellDescription",
    "GetInventorySlotInfo",
    "C_PaperDollInfo.GetInventorySlotInfo",
    "C_ColorOverrides.GetColorOverrideInfo",
    "C_UIWidgetManager.GetWidgetSetInfo",
    "C_UIWidgetManager.GetObjectiveTrackerWidgetSetID",
    "C_MountJournal.GetDynamicFlightModeSpellID",
    "C_MountJournal.GetNumMounts",
    "C_PetBattles.GetAllEffectNames",
    "C_PetJournal.GetNumPetSources",
    "C_QuestLog.GetMaxNumQuests",
    "C_GamepadUI.GetFirstGamepadActionStorageSlotIndex",
    "C_GamepadUI.GetFirstGamepadPetActionStorageSlotIndex",
    "C_Ping.GetDefaultPingOptions",
    "GetText",
    "GetMaxPlayerLevel",
    "GetMaxLevelForPlayerExpansion",
    "GetClassicExpansionLevel",
    "GetSendMailPrice",
    "C_StableInfo.GetNextStableSlotCost",
    "C_Bank.FetchMaxNumBankTabs",
    // Key bindings' data.
    "C_KeyBindings.GetSearchTagsForAction",
    "C_KeyBindings.GetBindingIndex",
    "GetBindingText",
    "IsBindingForGamePad",
    "C_KeyBindings.GetCustomBindingType",
    // The client's systems, as this build has them.
    "C_AccountServices.",
    "C_AdventureJournal.CanBeShown",
    "BNFeaturesEnabled",
    "C_BattleNet.IsBattleNetFriendsListSupported",
    "C_BattleNet.IsFriendTagSupportedForCurrentGameType",
    "C_CatalogShop.IsShop2Enabled",
    "C_ChatInfo.AreOutgoingAddonChatMessagesRestricted",
    "C_Club.IsEnabled",
    "C_Club.ShouldAllowClubType",
    "C_ClubFinder.IsCommunityFinderEnabled",
    "C_ClubFinder.GetClubFinderDisableReason",
    "C_Discord.IsVoiceEnabled",
    "C_EventScheduler.CanShowEvents",
    "C_ExternalEventURL.",
    "C_Housing.IsHousingServiceEnabled",
    "C_MythicPlus.IsMythicPlusActive",
    "C_PartyInfo.IsRaidListSupported",
    "C_PhotoSharing.IsEnabled",
    "C_Ping.IsPingSystemEnabled",
    "C_RecentAllies.IsSystemEnabled",
    "C_RecentAllies.IsSystemSupported",
    "C_RecentAllies.IsInteractionCategoryFilterSupportedForCurrentGameType",
    "C_RecruitAFriend.IsSystemSupported",
    "C_RecruitAFriend.GetRAFSystemInfo",
    "C_SocialQueue.IsSystemSupported",
    "C_SocialRestrictions.IsFriendsDisabled",
    "C_SocialUI.IsSystemEnabled",
    "C_StorePublic.IsEnabled",
    "C_TradeSkillUI.IsGuildTradeSkillsEnabled",
    "C_VoiceChat.IsEnabled",
    "C_VoiceChat.GetActiveVoiceProviderID",
    "C_WowTokenPublic.GetCommerceSystemStatus",
    "CanShowAchievementUI",
    "GetLFGInfoServer",
    "IsTestBuild",
    "IsAccountSecured",
    "GetRestrictedAccountData",
    "NoPlayTime",
    "PartialPlayTime",
    // The machine's (the tests' screen is the recording's).
    "GetMonitorCount",
    "GetMonitorName",
    "IsGraphicsSettingValueSupported",
    "IsGraphicsCVarValueSupported",
    "MultiSampleAntiAliasingSupported",
    "GetAvailableLocales",
    "Sound_GameSystem_GetNumOutputDrivers",
    "Sound_GameSystem_GetOutputDriverNameByIndex",
    "Sound_GameSystem_GetNumInputDrivers",
    "Sound_GameSystem_GetInputDriverNameByIndex",
    "C_UI.ShouldUIParentAvoidNotch",
    "C_VideoOptions.",
    "GetCameraFOVDefaults",
    "GetGraphicsAPIs",
    "GetMaxRenderScale",
    "GetMinRenderScale",
    "IsCpuBound",
    "PlatformIsHandheld",
    "SupportsClipCursor",
    "CanAutoSetGamePadCursorControl",
    // The account's and the character's settings.
    "C_CombatAudioAlert.GetThrottle",
    "C_TTSSettings.",
    "GetChatWindowInfo",
    "GetChatWindowSavedDimensions",
    "GetChatWindowSavedPosition",
    "GetChatWindowChannels",
    "GetChatWindowMessages",
    "GetActionBarToggles",
    "IsThreatWarningEnabled",
    "GetTutorialsEnabled",
    "IsTutorialFlagged",
    "C_Spell.IsSpellPassive",
    "C_Spell.IsAutoAttackSpell",
    "C_Spell.IsRangedAutoAttackSpell",
    "C_ClickBindings.CanSpellBeClickBound",
    "C_KeyBindings.GetBindingContextForAction",
    "C_LevelLink.IsSpellLocked",
    "C_Minimap.IsFilteredOut",
    // The machine (graphics, display).
    "GetCurrentGraphicsAPI",
    "GetGraphicsAPIs",
    "IsGraphicsCVarValueSupported",
    "GetGraphicsCVarValueForQualityLevel",
    "GetMinRenderScale",
    "GetMaxRenderScale",
    "AntiAliasingSupported",
    "C_VideoOptions.",
    "C_UI.DoesAnyDisplayHaveNotch",
    // The account's settings and state.
    "C_CombatAudioAlert.",
    "C_SocialRestrictions.",
    "C_VoiceChat.IsSpeakForMeAllowed",
    "C_VoiceChat.GetPushToTalkBinding",
    "C_ClickBindings.GetProfileInfo",
    "C_ClickBindings.GetTutorialShown",
    "C_ClickBindings.GetBindingType",
    "C_SplashScreen.CanViewSplashScreen",
    "C_TransmogOutfitInfo.IsTransmogEnabled",
    "GetAllowRecentAlliesSeeLocation",
    "C_EditMode.GetEditModeDefaultLayout",
    "C_EditMode.GetAccountSettings",
    "GetAutoDeclineGuildInvites",
    "GetCurrentBindingSet",
];

/// Functions of systems the engine does not model yet, answered as the
/// recording's world had them: nothing going on (no pet battle, scenario,
/// vehicle, summons, queue, totem, pet…). The engine's models take over
/// as they come.
const BASELINE: &[&str] = &[
    "C_ActionBar.HasAssistedCombatActionButtons",
    "C_ActionBar.HasBonusActionBar",
    "C_ActionBar.HasExtraActionBar",
    "C_ActionBar.HasOverrideActionBar",
    "C_ActionBar.HasTempShapeshiftActionBar",
    "C_ActionBar.HasVehicleActionBar",
    "C_ArtifactUI.IsAtForge",
    "C_AssistedCombat.",
    "C_AzeriteItem.",
    "C_Bank.CanPurchaseBankTab",
    "C_Bank.FetchBankLockedReason",
    "C_Bank.FetchNumPurchasedBankTabs",
    "C_Bank.FetchPurchasedBankTabData",
    "C_CatalogShop.HasNewProducts",
    "C_ChatInfo.GetChannelShortcutForChannelID",
    "C_ClientScene.IsSceneTypeActive",
    "C_Club.GetInvitationsForSelf",
    "C_ClubFinder.GetPlayerApplicantLocaleFlags",
    "C_ClubFinder.GetPlayerApplicantSettings",
    "C_Container.GetBackpackAutosortDisabled",
    "C_Container.GetBackpackSellJunkDisabled",
    "C_Container.IsContainerFiltered",
    "C_ContentTracking.",
    "C_CurrencyInfo.DoesCurrentFilterRequireAccountCurrencyData",
    "C_DeathInfo.GetSelfResurrectOptions",
    "C_DelvesUI.",
    "C_EventScheduler.HasSavedReminders",
    "C_FogOfWar.GetFogOfWarForMap",
    "C_FrameManager.GetFrameVisibilityState",
    "C_GamePad.GetDeviceMappedState",
    "C_Heirloom.",
    "C_HeirloomInfo.",
    "C_Housing.GetTrackedHouseGuid",
    "C_Housing.IsInsideHouseOrPlot",
    "C_LFGList.",
    "C_LFGInfo.CanPlayerUseLFD",
    "C_LFGInfo.CanPlayerUsePremadeGroup",
    "C_LobbyMatchmakerInfo.IsInQueue",
    "C_MountJournal.GetDisplayedMountInfo",
    "C_MountJournal.GetNumDisplayedMounts",
    "C_MountJournal.HasDisplayableMount",
    "C_MountJournal.IsDragonridingUnlocked",
    "C_MountJournal.MountEquipmentAvailable",
    "C_NeighborhoodInitiative.GetTrackedInitiativeTasks",
    "C_PartyInfo.AllowedToDoPartyConversion",
    "C_PerksActivities.GetTrackedPerksActivities",
    "C_PetBattles.",
    "C_PetJournal.GetPetInfoByIndex",
    "C_PetJournal.GetSummonBattlePetCooldown",
    "C_PlayerChoice.",
    "C_PlayerInfo.GetAlternateFormInfo",
    "C_PlayerInfo.IsPlayerInRPE",
    "C_PlayerInfo.IsPlayerNPERestricted",
    "C_PlayerInfo.ShouldDisplaySurname",
    "C_PvP.",
    "C_QuestLog.CanAbandonQuest",
    "C_QuestLog.GetBountiesForMapID",
    "C_QuestLog.GetBountySetInfoForMapID",
    "C_QuestLog.GetNumWorldQuestWatches",
    "C_QuestLog.GetQuestWatchType",
    "C_QuestLog.GetQuestsOnMap",
    "C_QuestLog.GetZoneStoryInfo",
    "C_QuestLog.HasActiveThreats",
    "C_QuestLog.IsPushableQuest",
    "C_QuestLog.IsQuestDisabledForSession",
    "C_QuestSession.",
    "C_RecruitAFriend.GetRAFInfo",
    "C_Scenario.GetStepInfo",
    "C_Scenario.IsInScenario",
    "C_Scenario.ShouldShowCriteria",
    "C_ScenarioInfo.",
    "C_SocialQueue.GetAllGroups",
    "C_SpectatingUI.",
    "C_Spell.GetMawPowerRarityInfoBySpellID",
    "C_StableInfo.GetStablePetInfo",
    "C_StableInfo.GetNumStableSlots",
    "C_SummonInfo.",
    "C_TaskQuest.GetQuestsOnMap",
    "C_ToyBox.GetNumFilteredToys",
    "C_ToyBox.HasFavorites",
    "C_ToyBoxInfo.HasAnyToy",
    "C_TradeSkillUI.AnyRecipeCategoriesFiltered",
    "C_TradeSkillUI.AreAnyInventorySlotsFiltered",
    "C_TradeSkillUI.GetAllFilterableInventorySlotsCount",
    "C_TradeSkillUI.GetBaseProfessionInfo",
    "C_TradeSkillUI.GetChildProfessionInfo",
    "C_TradeSkillUI.GetChildProfessionInfos",
    "C_TradeSkillUI.GetOnlyShowFirstCraftRecipes",
    "C_TradeSkillUI.GetOnlyShowMakeableRecipes",
    "C_TradeSkillUI.GetOnlyShowSkillUpRecipes",
    "C_TradeSkillUI.GetRecipeItemNameFilter",
    "C_TradeSkillUI.IsNPCCrafting",
    "C_TradeSkillUI.IsTradeSkillGuild",
    "C_Traits.GetConfigIDByTreeID",
    "C_TransmogCollection.GetLatestAppearance",
    "C_TransmogCollection.IsSearchInProgress",
    "C_TransmogCollection.IsUsingDefaultFilters",
    "C_VoiceChat.GetActiveChannelType",
    "C_VoiceChat.IsMuted",
    "C_VoiceChat.IsSpeakForMeActive",
    "CanExitVehicle",
    "CanHearthAndResurrectFromArea",
    "GetArenaOpponentSpec",
    "GetBackgroundLoadingStatus",
    "GetCorpseRecoveryDelay",
    "GetCurrentArenaSeason",
    "GetLFGProposal",
    "GetLFGReadyCheckUpdate",
    "GetLFGRoleUpdate",
    "GetLootSpecialization",
    "GetNumArenaOpponentSpecs",
    "GetNumSavedWorldBosses",
    "GetPartyLFGID",
    "GetPetActionCooldown",
    "GetPetActionInfo",
    "GetPetExperience",
    "GetPetIcon",
    "GetPossessInfo",
    "GetReadyCheckStatus",
    "GetReleaseTimeRemaining",
    "GetSpecializationRoleEnum",
    "GetTotemInfo",
    "GetWorldElapsedTimers",
    "HasArtifactEquipped",
    "HasPetUI",
    "IsActiveBattlefieldArena",
    "IsEveryoneAssistant",
    "IsInActiveWorldPVP",
    "IsInInstance",
    "IsOnGroundFloorInJailersTower",
    "IsPVPTimerRunning",
    "IsPartyLFG",
    "IsTargetLoose",
    "PetHasActionBar",
    "ResurrectGetOfferer",
    "ResurrectHasSickness",
    "ResurrectHasTimer",
    "ShouldDisplayRaidRolesInSocialFrame",
    "C_LossOfControl.GetActiveLossOfControlDataByUnit",
    "C_NamePlateManager.IsNamePlateUnitBehindCamera",
    "C_PvP.IsPVPMap",
    "C_SpecializationInfo.GetPvpTalentSlotInfo",
    "C_SpecializationInfo.IsSpecSelectionEnabled",
    "InCinematic",
    "GetRaidTargetIndex",
    "UnitClassification",
    "UnitDetailedThreatSituation",
    "UnitIsMinion",
    "UnitIsTapDenied",
    "UnitNameplateShowsWidgetsOnly",
    "UnitPlayerControlled",
    "UnitSelectionColor",
    "UnitShouldDisplayName",
    "UnitWidgetSet",
];

/// Events of the player's session, not the client's startup: left out of
/// the login data.
const SESSION: &[&str] = &[
    "CONSOLE_MESSAGE",
    "SPELL_DATA_LOAD_RESULT",
    "QUEST_DATA_LOAD_RESULT",
    "ITEM_DATA_LOAD_RESULT",
    "CHAT_MSG_",
    "NAME_PLATE_",
    "UNIT_SPELLCAST_",
    "GUILD",
    "BN_",
    "CLUB_",
    "RAF_",
    "DISCORD",
    "RECENT_ALLIES",
    "TOKEN_",
    "TABARD_",
    "PLAYER_GUILD_UPDATE",
    "CRITERIA_UPDATE",
    "ACTION_RANGE_CHECK_UPDATE",
    "SPELL_ACTIVATION_OVERLAY",
    "UPDATE_MOUSEOVER_UNIT",
    "CURSOR_CHANGED",
    "WORLD_CURSOR_TOOLTIP_UPDATE",
    "GLOBAL_MOUSE",
    "GLOBAL_REGION_MOUSE",
    "MODIFIER_STATE_CHANGED",
    "PLAYER_SOFT_",
    "CURRENT_SPELL_CAST_CHANGED",
    // The recorder's own marks (when timers and OnUpdate run).
    "TIMER_TICK",
    "ON_UPDATE",
];

/// Events the test's world makes (its items, auras and action bars), not
/// replayed as recorded.
const WORLD: &[&str] = &[
    "PLAYER_EQUIPED_SPELLS_CHANGED",
    "BAG_UPDATE",
    "UNIT_INVENTORY_CHANGED",
    "UNIT_AURA",
    "ACTION_USABLE_CHANGED",
    "PLAYERBANKSLOTS_CHANGED",
];

/// The login's events as the engine replays them: one per line, the
/// recorded time, the event and its arguments; `$ADDON`, `$BINDINGS`,
/// `$CVAR` (the world's value of the CVar named before it), `$INVENTORY`
/// (the inventory's load, made from the world's items) in place of what
/// the test's world gives.
fn login_data(me: &Identity, version: &str, lines: &[String]) -> String {
    let mut out = format!(
        "# WoW Forever {version}'s login (/stdtest engine startup), from the addon's\n\
         # ADDON_LOADED to three seconds after PLAYER_ENTERING_WORLD: time, event and\n\
         # arguments (as recorded; $-names the test's world fills in). The player's\n\
         # session (chat, guild, Battle.net, others' casts, data loads) is left out\n\
         # (cargo xtask ingest).\n"
    );
    let mut inventory = false;
    for line in lines {
        let Some((time, rest)) = line.split_once(' ') else {
            continue;
        };
        let (event, args) = rest.split_once('(').unwrap_or((rest, ")"));
        let args = args.strip_suffix(')').unwrap_or(args);
        if SESSION.iter().any(|p| event.starts_with(p)) {
            continue;
        }
        // Units seen around the player (`nameplate5`): the world, which
        // differs each time.
        if args
            .strip_prefix("string nameplate")
            .is_some_and(|n| n.starts_with(|c: char| c.is_ascii_digit()))
        {
            continue;
        }
        // Only the addon's own ADDON_LOADED: Blizzard's load-on-demand
        // addons fire theirs as Blizzard's code loads them.
        if event == "ADDON_LOADED" {
            if args.starts_with("string StdTest;") {
                let _ = writeln!(out, "{time}\tADDON_LOADED\t$ADDON; $BINDINGS");
            }
            continue;
        }
        if WORLD.contains(&event) {
            if !inventory && event == "PLAYER_EQUIPED_SPELLS_CHANGED" {
                inventory = true;
                let _ = writeln!(out, "{time}\t$INVENTORY\t");
            }
            continue;
        }
        if event == "CVAR_UPDATE" {
            let name = args.split("; ").next().unwrap_or("");
            let _ = writeln!(out, "{time}\tCVAR_UPDATE\t{name}; $CVAR");
            continue;
        }
        // Edit Mode's layouts: the test's world's (the account's own
        // layouts, named by the player, are not kept).
        if event == "EDIT_MODE_LAYOUTS_UPDATED" {
            let reconcile = args.rsplit("; ").next().unwrap_or("boolean false");
            let _ = writeln!(
                out,
                "{time}	EDIT_MODE_LAYOUTS_UPDATED	$EDIT_MODE_LAYOUTS; {reconcile}"
            );
            continue;
        }
        // Other tables' contents are not replayed (the startup's other
        // events with one are the session's: group buffs, covenant
        // callings).
        if args.contains('{') {
            continue;
        }
        let _ = writeln!(out, "{time}\t{event}\t{}", me.scrub(args));
    }
    out
}

pub fn run(root: &Path, saved: &Path) -> Result<()> {
    let src = std::fs::read(saved).with_context(|| saved.display().to_string())?;
    let lua = mlua::Lua::new();
    lua.load(&src[..])
        .exec()
        .map_err(|e| anyhow!("{}: {e}", saved.display()))?;
    let state: mlua::Table = lua
        .globals()
        .get("StdTestState")
        .map_err(|_| anyhow!("no StdTestState in {}", saved.display()))?;
    let client = root.join("crates/wowlua-engine/tests/client");
    let battery = flatten(&lua, &state, FLATTEN)?;
    let get = |key: &str| {
        battery
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    // The player, from the battery's unit calls.
    let strings = |v: &str| -> Vec<String> {
        let re = regex::Regex::new(r"string ([^,]+)").expect("a valid pattern");
        re.captures_iter(v)
            .map(|c| c[1].trim().to_string())
            .collect()
    };
    let names = get("probe.units.UnitName(player)")
        .map(|v| strings(&v))
        .ok_or_else(|| anyhow!("no /stdtest engine results to name the player by"))?;
    let guid = get("probe.units.UnitGUID(player)")
        .and_then(|v| strings(&v).into_iter().next())
        .ok_or_else(|| anyhow!("no player GUID in the /stdtest engine results"))?;
    let me = Identity {
        name: names.first().cloned().unwrap_or_default(),
        last_name: names.get(1).cloned(),
        guid,
    };
    let version = get("probe.client").unwrap_or_else(|| "an unknown client".into());

    let scrubbed: Vec<(String, String)> = battery
        .iter()
        .map(|(k, v)| (me.scrub(k), me.scrub(v)))
        .collect();
    write_table(
        &client.join("forever-camelot-engine.lua"),
        &format!(
            "WoW Forever {version}: /stdtest engine, order, protected and blocked\n\
             (tests/ingame/src/engineprobe.wowlua), flattened; the player's name, realm\n\
             and GUID replaced (cargo xtask ingest)."
        ),
        &scrubbed,
    )?;

    // The CVar flags, as the engine's data.
    let mut flags = String::from(&format!(
        "# The CVars C_CVar.GetCVarInfo flags on WoW Forever {version}\n\
         # (/stdtest engine, probe.cvarFlags): secure (addons may not set them),\n\
         # readOnly, locked (from the user); one per line, the flag then the name.\n"
    ));
    let mut any_flags = false;
    for kind in ["secure", "readOnly", "locked"] {
        if let Some(list) = get(&format!("probe.cvarFlags.{kind}")) {
            any_flags = true;
            for name in list.split_whitespace() {
                let _ = writeln!(flags, "{kind}\t{name}");
            }
        }
    }
    if any_flags {
        let path = root.join("crates/wowlua-engine/data/camelot-cvar-flags.txt");
        std::fs::write(&path, flags)?;
        println!("wrote {}", path.display());
    }

    // The heights fonts keep after SetFont (the same for every file).
    if let Some(list) = get("probe.fontHeights.Fonts\\FRIZQT__.TTF") {
        let mut out = format!(
            "# The heights a font keeps after SetFont on WoW Forever {version}, the same\n\
             # for every font file (/stdtest engine, probe.fontHeights; GetFont's\n\
             # height, GetFontInfo floors it); one per line: the height set, then kept.\n"
        );
        for entry in list.split_whitespace() {
            if let Some((set, rest)) = entry.split_once('=') {
                let kept = rest.split('/').next().unwrap_or(rest);
                let _ = writeln!(out, "{set}\t{kept}");
            }
        }
        let path = root.join("crates/wowlua-engine/data/camelot-font-heights.txt");
        std::fs::write(&path, out)?;
        println!("wrote {}", path.display());
    }

    // The client's login, as engine data (`/stdtest engine startup`).
    for (key, file) in [
        ("engineStartupLogin", "camelot-login.txt"),
        ("engineStartup", "camelot-reload.txt"),
    ] {
        let Ok(list) = state.get::<mlua::Table>(key) else {
            continue;
        };
        let lines: Vec<String> = list
            .sequence_values::<String>()
            .collect::<Result<_, _>>()
            .map_err(|e| anyhow!("{e}"))?;
        // A recording without full payloads (an older StdTest) is skipped.
        if lines
            .iter()
            .any(|l| l.contains("ACTION_USABLE_CHANGED({1,"))
        {
            println!("skipped {key}: recorded without full payloads (record it again)");
            continue;
        }
        let text = login_data(&me, &version, &lines);
        let path = root.join("crates/wowlua-engine/data").join(file);
        std::fs::write(&path, text)?;
        println!("wrote {}", path.display());
    }

    // The input recording, in order.
    if let Ok(input) = state.get::<mlua::Table>("engineInput") {
        let mut out = format!(
            "-- WoW Forever {version}: /stdtest engine input\n\
             -- (tests/ingame/src/engineprobe.wowlua): hover the button, click it,\n\
             -- Shift-click it, click the box, type ab, press Enter.\nreturn {{\n"
        );
        for line in input.sequence_values::<String>() {
            let line = line.map_err(|e| anyhow!("{e}"))?;
            let _ = writeln!(out, "    {},", quote(&me.scrub(&line)));
        }
        out.push_str("}\n");
        let path = client.join("forever-camelot-input.lua");
        std::fs::write(&path, out)?;
        println!("wrote {}", path.display());
    }

    // The character's world, as a test profile (`/stdtest snapshot`).
    if state.get::<mlua::Table>("snapshot").is_ok() {
        let tmp = tempfile::NamedTempFile::new()?;
        crate::profile::run(saved, tmp.path())?;
        let text = std::fs::read_to_string(tmp.path())?;
        let path = client.join("forever-camelot-character.json");
        std::fs::write(&path, me.scrub(&text))?;
        println!("wrote {}", path.display());
    }

    // The answers to the calls the engine could not answer.
    if let Ok(answers) = state.get::<mlua::Table>("engineAnswers") {
        let mut entries = Vec::new();
        for pair in answers.pairs::<String, String>() {
            let (call, answer) = pair.map_err(|e| anyhow!("{e}"))?;
            let answer = if SOCIAL.iter().any(|p| call.starts_with(p)) {
                blank_strings(&answer)
            } else {
                answer
            };
            entries.push((me.scrub(&call), me.scrub(&answer)));
        }
        entries.sort();
        // The static functions' answers, as engine data.
        let mut data = format!(
            "# WoW Forever {version}'s answers to calls of functions that answer from the\n\
             # client's own data (/stdtest engine answers), as the engine gives them:\n\
             # the call, a tab, the answer (cargo xtask ingest).\n"
        );
        for (call, answer) in &entries {
            let name = call.split('(').next().unwrap_or("");
            let listed = |list: &[&str]| {
                list.iter()
                    .any(|p| name == *p || (p.ends_with('.') && name.starts_with(p)))
            };
            if (listed(STATIC) || listed(BASELINE)) && !answer.starts_with("error") {
                let _ = writeln!(data, "{call}\t{answer}");
            }
        }
        // The cursor (`/stdtest engine cursor`): each step's state and the
        // events between.
        if let Ok(log) = state.get::<mlua::Table>("engineCursor") {
            let mut list = Vec::new();
            for (i, line) in log.sequence_values::<String>().enumerate() {
                let line = line.map_err(|e| anyhow!("{e}"))?;
                list.push((format!("{:03}", i + 1), me.scrub(&line)));
            }
            write_table(
                &client.join("forever-camelot-cursor.lua"),
                &format!(
                    "WoW Forever {version}: /stdtest engine cursor, the cursor's steps and\n\
                     events in order (cargo xtask ingest)."
                ),
                &list,
            )?;
        }
        // The keyboard (`/stdtest engine keys`): which frames each key
        // reached, and the binding's clicks, in order.
        if let Ok(log) = state.get::<mlua::Table>("engineKeys") {
            let mut list = Vec::new();
            for (i, line) in log.sequence_values::<String>().enumerate() {
                let line = line.map_err(|e| anyhow!("{e}"))?;
                list.push((format!("{:03}", i + 1), me.scrub(&line)));
            }
            write_table(
                &client.join("forever-camelot-keys.lua"),
                &format!(
                    "WoW Forever {version}: /stdtest engine keys, the keyboard frames each\n\
                     key reached and the binding's clicks, in order (cargo xtask ingest)."
                ),
                &list,
            )?;
        }
        // The checks of rules the call form cannot ask (`/stdtest engine
        // answers`' CHECKS), for the engine's tests.
        if let Ok(checks) = state.get::<mlua::Table>("engineChecks") {
            let mut list = Vec::new();
            for pair in checks.pairs::<String, String>() {
                let (label, answer) = pair.map_err(|e| anyhow!("{e}"))?;
                list.push((label, me.scrub(&answer)));
            }
            list.sort();
            write_table(
                &client.join("forever-camelot-checks.lua"),
                &format!(
                    "WoW Forever {version}: /stdtest engine answers' checks, the client's\n\
                     answers to rules the call form cannot ask (cargo xtask ingest)."
                ),
                &list,
            )?;
        }
        // The map data (`/stdtest engine maps`, all static) and the
        // account's collections (`/stdtest engine collections`, the default
        // world's).
        for (key, fixture, about) in [
            (
                "engineMapAnswers",
                "forever-camelot-maps.lua",
                "/stdtest engine maps, the client's map data (every map from\n\
                 the world map and the player's map down) as Blizzard's UI asks it",
            ),
            (
                "engineCollectionAnswers",
                "forever-camelot-collections.lua",
                "/stdtest engine collections, the account's collections\n\
                 (transmog appearances by category; mounts, pets, toys, heirlooms)",
            ),
        ] {
            if let Ok(maps) = state.get::<mlua::Table>(key) {
                let mut map_entries = Vec::new();
                for pair in maps.pairs::<String, String>() {
                    let (call, answer) = pair.map_err(|e| anyhow!("{e}"))?;
                    map_entries.push((call, answer));
                }
                map_entries.sort();
                for (call, answer) in &map_entries {
                    if !answer.starts_with("error") && !entries.iter().any(|(c, _)| c == call) {
                        let _ = writeln!(data, "{call}\t{answer}");
                    }
                }
                write_table(
                    &client.join(fixture),
                    &format!("WoW Forever {version}: {about} (cargo xtask ingest)."),
                    &map_entries,
                )?;
            }
        }
        let path = root.join("crates/wowlua-engine/data/camelot-answers.txt");
        std::fs::write(&path, data)?;
        println!("wrote {}", path.display());
        write_table(
            &client.join("forever-camelot-answers.lua"),
            &format!(
                "WoW Forever {version}: /stdtest engine answers, the client's answers to the\n\
                 calls Blizzard's UI made in the engine's sweep that the engine could not\n\
                 answer (tests/ingame/src/answers.wowlua); the player scrubbed, the social\n\
                 systems' texts blanked (cargo xtask ingest)."
            ),
            &entries,
        )?;
    }
    Ok(())
}
