//! What the client answers when there is nothing to tell, and the
//! client's and the realm's own settings, whatever the arguments: the full
//! probe (`/stdtest engine fullprobe`) called each of these with many
//! arguments (none, a baseline, numbers, wrong types) and got one answer
//! from all that did not fail. No petition, item text, LFG proposal,
//! archaeology, war game or saved instance is there in the engine's world
//! either, so these are its answers too; a system the engine comes to
//! model answers from its state instead (its own behaviour takes the
//! place of a fill-in).
//!
//! Unverified: the answers with something there (a petition offered, a
//! dungeon proposed), and for the settings another account's or realm's.
//! A function a recording answers per call (`data/camelot-answers.txt`)
//! gives this answer to the calls it lacks.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Vm};

/// The answers, as the probe shows them (`n: type value, …`).
const BLANKS: &[(&str, &str)] = &[
    // Nothing there: no achievement comparison, search or reward.
    ("GetAchievementComparisonInfo", "0: "),
    ("GetAchievementNumRewards", "1: boolean false"),
    ("GetAchievementReward", "1: nil nil"),
    ("GetAchievementNumCriteria", "1: number 0"),
    // The dungeon finder and saved instances: no proposal, vote,
    // cooldown, reward or lock.
    ("GetInstanceLockTimeRemainingEncounter", "0: "),
    ("GetLFGCategoryForID", "0: "),
    ("GetLFGCompletionRewardItemLink", "0: "),
    ("GetLFGDeserterExpiration", "0: "),
    ("GetLFGRandomCooldownExpiration", "0: "),
    ("GetLFGDungeonRewardLink", "0: "),
    ("GetLootRollItemLink", "0: "),
    ("GetRandomDungeonBestChoice", "0: "),
    (
        "GetLFGDungeonEncounterInfo",
        "4: string , nil nil, boolean false, boolean false",
    ),
    (
        "GetLFGProposalEncounter",
        "4: string , nil nil, boolean false, boolean false",
    ),
    (
        "GetLFGInviteRoleAvailability",
        "2: boolean true, boolean false",
    ),
    ("GetLFGInviteRoleRestrictions", "1: {}"),
    ("GetLFGReadyCheckUpdateBattlegroundInfo", "1: nil nil"),
    ("GetLFGRoleUpdateBattlegroundInfo", "1: nil nil"),
    (
        "GetLFGInfoServer",
        "16: boolean false, boolean false, boolean false, boolean false, boolean false, string , number 0, nil nil, boolean false, boolean false, boolean false, boolean false, number 0, number 0, number 0, number 0",
    ),
    ("GetLFDLockInfo", "2: string Unknown, number 0"),
    ("GetLFDLockPlayerCount", "1: number 1"),
    ("GetLFDRoleLockInfo", "1: {}"),
    ("RequestLFDPartyLockInfo", "1: boolean true"),
    ("RequestLFDPlayerLockInfo", "1: boolean true"),
    ("GetSavedInstanceChatLink", "1: string "),
    (
        "GetSavedInstanceInfo",
        "14: nil nil, number 0, nil nil, number 0, boolean false, boolean false, number 0, boolean false, number 0, string , number 0, number 0, boolean false, number 0",
    ),
    ("GetSavedWorldBossInfo", "3: nil nil, number 0, nil nil"),
    ("GetBestFlexRaidChoice", "0: "),
    ("GetBestRFChoice", "0: "),
    ("GetRFDungeonInfo", "0: "),
    ("GetRandomScenarioBestChoice", "0: "),
    ("GetRandomScenarioInfo", "0: "),
    ("GetPartyLFGBackfillInfo", "0: "),
    ("GetHomePartyInfo", "0: "),
    ("IsMasterLooter", "0: "),
    // The encounter journal: nothing selected or searched.
    ("EJ_GetCreatureInfo", "0: "),
    ("EJ_GetEncounterInfo", "0: "),
    ("EJ_GetEncounterInfoByIndex", "0: "),
    ("EJ_GetInstanceInfo", "0: "),
    ("EJ_GetMapEncounter", "0: "),
    ("EJ_GetSearchResult", "0: "),
    ("EJ_GetSectionPath", "0: "),
    ("EJ_IsSearchFinished", "1: boolean false"),
    // Archaeology, artifacts by race: none.
    ("ArchaeologyGetIconInfo", "0: "),
    ("GetActiveArtifactByRace", "0: "),
    ("GetArtifactInfoByRace", "0: "),
    ("GetNumArtifactsByRace", "0: "),
    ("GetSelectedArtifactInfo", "0: "),
    ("GetArchaeologyInfo", "1: string UNKNOWN"),
    (
        "GetArchaeologyRaceInfo",
        "6: string UNKNOWN, nil nil, number 0, number 0, number 0, number 0",
    ),
    (
        "GetArchaeologyRaceInfoByID",
        "6: string UNKNOWN, nil nil, number 0, number 0, number 0, number 0",
    ),
    ("CanSolveArtifact", "1: boolean true"),
    ("IsArtifactCompletionHistoryAvailable", "1: boolean true"),
    // A quest giver's dialog closed: no active or available quest.
    ("GetActiveLevel", "1: number 0"),
    ("GetActiveQuestID", "1: number 0"),
    ("GetActiveTitle", "2: nil nil, boolean false"),
    ("GetAvailableLevel", "1: number 0"),
    (
        "GetAvailableQuestInfo",
        "8: boolean false, number 0, boolean false, boolean false, number 0, boolean false, boolean false, number 0",
    ),
    ("GetAutoQuestPopUp", "0: "),
    ("GetQuestLogChoiceInfoLootType", "0: "),
    ("GetQuestLogItemDrop", "3: nil nil, nil nil, nil nil"),
    ("GetQuestLogQuestType", "1: number 0"),
    ("GetQuestLogRewardFactionInfo", "2: number 0, number 0"),
    (
        "GetQuestLogRewardSkillPoints",
        "3: nil nil, nil nil, nil nil",
    ),
    ("GetQuestLogRewardTitle", "1: nil nil"),
    ("GetQuestLogSpecialItemCooldown", "0: "),
    ("GetQuestPOILeaderBoard", "3: nil nil, nil nil, nil nil"),
    // A quest log index the snapshot lacks (`stats.rs` answers the
    // snapshot's).
    ("IsQuestSequenced", "1: boolean false"),
    ("GetCombatRatingBonus", "1: number 0"),
    // Another unit's ranged weapon: none, at speed 1.
    (
        "UnitRangedDamage",
        "6: number 0, number 0, number 0, number 0, number 0, number 1",
    ),
    ("GetTaskPOIs", "1: {}"),
    ("GetRewardPackArtifactPower", "0: "),
    ("GetRewardPackCurrencies", "0: "),
    ("GetRewardPackItems", "0: "),
    ("GetRewardPackMoney", "0: "),
    ("GetRewardPackTitle", "0: "),
    ("GetRewardPackTitleName", "0: "),
    // No item text open.
    ("ItemTextGetPage", "1: number 1"),
    ("ItemTextGetText", "1: string "),
    // No petition, trade, invite or war game.
    (
        "GetPetitionInfo",
        "7: nil nil, nil nil, nil nil, number 0, nil nil, nil nil, number 0",
    ),
    ("GetPetitionNameInfo", "1: nil nil"),
    ("GetPlayerTradeCurrency", "2: number 0, number 0"),
    ("GetTargetTradeCurrency", "2: number 0, number 0"),
    ("GetInviteConfirmationInfo", "0: "),
    ("GetNextPendingInviteConfirmation", "0: "),
    ("GetWarGameQueueStatus", "0: "),
    ("GetWarGameTypeInfo", "0: "),
    // No pet summoned for a time, no glyph, pvp talent or garrison.
    ("GetPetTimeRemaining", "1: nil nil"),
    ("PetCanBeDismissed", "1: boolean true"),
    ("GetCallPetSpellInfo", "0: "),
    ("GetCurrentGlyphNameForSpell", "0: "),
    ("GetPendingGlyphName", "0: "),
    ("GetJournalInfoForSpellConfirmation", "0: "),
    ("GetFollowerTypeIDFromSpell", "0: "),
    ("GetEquipmentNameFromSpell", "0: "),
    ("GetWorldMapActionButtonSpellInfo", "0: "),
    ("ItemCanTargetGarrisonFollowerAbility", "0: "),
    (
        "SpellCanTargetGarrisonFollower",
        "2: boolean false, number 2",
    ),
    (
        "SpellCanTargetGarrisonFollowerAbility",
        "2: boolean false, number 3",
    ),
    ("GetPvpTalentInfoByID", "0: "),
    ("GetPvpTalentInfoBySpecialization", "0: "),
    ("GetPvpTalentLink", "0: "),
    ("FlyoutHasSpell", "0: "),
    ("GetTotemDuration", "1: nil nil"),
    ("UnitIsBattlePet", "1: nil nil"),
    // No inspected player, raid profile, guild event or channel shown.
    ("GetInspectTalent", "0: "),
    (
        "GetInspectHonorData",
        "9: number 0, number 0, number 0, number 0, number 0, number 0, number 0, number 0, number 0",
    ),
    ("GetComparisonStatistic", "1: string "),
    ("GetRaidProfileFlattenedOptions", "0: "),
    ("GetRaidProfileName", "0: "),
    ("GetRaidProfileOption", "0: "),
    (
        "GetGuildEventInfo",
        "8: nil nil, nil nil, nil nil, nil nil, number 0, number 0, number 0, number 0",
    ),
    ("GetGuildNewsMemberName", "0: "),
    (
        "GetGuildRecipeInfoPostQuery",
        "3: number 0, number 0, number 0",
    ),
    ("GetGuildCategoryList", "1: {}"),
    ("GetNumChannelMembers", "1: nil nil"),
    ("IsDisplayChannelModerator", "1: nil nil"),
    ("IsDisplayChannelOwner", "1: nil nil"),
    ("GetGMStatus", "0: "),
    ("GetCurrentCinematicSummary", "1: nil nil"),
    ("IsJailersTowerLayerTimeLocked", "1: nil nil"),
    // No alternate power bar: its texture's shape with no file.
    (
        "GetUnitPowerBarTextureInfo",
        "5: nil nil, number 0.0039215688593686, number 0.0039215688593686, number 0.0039215688593686, number 0.0039215688593686",
    ),
    (
        "GetUnitPowerBarTextureInfoByID",
        "5: nil nil, number 0.0039215688593686, number 0.0039215688593686, number 0.0039215688593686, number 0.0039215688593686",
    ),
    ("GetMerchantItemCostItem", "2: nil nil, number 0"),
    // Battle.net: none of it answers (the full probe: no friend, nothing
    // selected). `BNGetInfo` is `social`'s: given an argument it answers
    // nothing; without one the two recordings disagree (the full probe of
    // 2026-10-20: nothing; the answers of 2026-10-21: the account's eight
    // values, kept; probe 20e).
    ("BNGetFriendIndex", "0: "),
    ("BNGetNumFOF", "0: "),
    ("BNGetSelectedFriend", "0: "),
    ("BNIsBlocked", "0: "),
    // The client's and the realm's settings and capabilities.
    ("AreClassRolesSoftSuggestions", "1: boolean true"),
    (
        "CanChangePlayerDifficulty",
        "2: boolean false, boolean true",
    ),
    ("CanShowSetRoleButton", "1: boolean true"),
    ("CanResetTutorials", "1: boolean true"),
    ("CanAutoSetGamePadCursorControl", "1: boolean false"),
    ("DoesCurrentLocaleSellExpansionLevels", "1: boolean true"),
    ("GMEuropaBugsEnabled", "1: boolean true"),
    ("GMEuropaComplaintsEnabled", "1: boolean true"),
    ("GMEuropaSuggestionsEnabled", "1: boolean true"),
    ("GMQuickTicketSystemEnabled", "1: boolean true"),
    ("HasDualWieldPenalty", "1: boolean true"),
    ("IsDemonHunterAvailable", "1: boolean true"),
    ("IsPublicTestClient", "1: boolean true"),
    ("IsBetaBuild", "1: boolean true"),
    ("IsRaidMarkerSystemEnabled", "1: boolean true"),
    ("IsOutlineModeSupported", "1: boolean true"),
    ("IsUsingMouse", "1: boolean false"),
    ("IsBindingForGamePad", "1: boolean false"),
    ("ShouldShowSpecialSplashScreen", "1: boolean true"),
    ("SplashFrameCanBeShown", "1: boolean true"),
    ("HasLoadedCUFProfiles", "1: boolean true"),
    ("GetGuildNewsSort", "1: boolean false"),
    (
        "GetGuildNewsFilters",
        "9: boolean true, boolean true, boolean true, boolean true, boolean true, boolean true, boolean true, boolean true, boolean true",
    ),
    ("GetExpansionTrialInfo", "2: boolean false, nil nil"),
    ("GetMaxRewardCurrencies", "1: number 4"),
    ("GetNumBattlegroundTypes", "1: number 3"),
    ("GetNumFlyouts", "1: number 19"),
    ("GetSoundEntryCount", "1: number 0"),
    (
        "GetVideoCaps",
        "7: boolean true, boolean true, boolean true, boolean true, boolean true, number 16, boolean true",
    ),
    (
        "GetAvailableLocales",
        "10: string enUS, string koKR, string frFR, string deDE, string zhCN, string esES, string zhTW, string esMX, string ruRU, string ptBR",
    ),
    (
        "GetScriptBucketThrottleLimits",
        "1: {luaScriptBucketThrottleMaxMsBurstNormal=number 20000,luaScriptBucketThrottleMaxMsBurstRestricted=number 1000,luaScriptBucketThrottleMaxMsPerSecondNormal=number 800,luaScriptBucketThrottleMaxMsPerSecondRestricted=number 500}",
    ),
    (
        "Sound_ChatSystem_GetInputDriverNameByIndex",
        "1: string None",
    ),
    (
        "Sound_ChatSystem_GetOutputDriverNameByIndex",
        "1: string None",
    ),
    (
        "Sound_GameSystem_GetInputDriverNameByIndex",
        "1: string None",
    ),
    ("GetChatTypeIndex", "1: number 0"),
    // Recorded per call; an achievement category or driver the recording
    // lacks.
    ("GetNumCompletedAchievements", "2: number 0, number 0"),
    (
        "GetCategoryNumAchievements",
        "3: number 0, number 0, number 0",
    ),
    (
        "Sound_GameSystem_GetOutputDriverNameByIndex",
        "1: string System Default",
    ),
    // No death to recap (the global `DeathRecap_GetEvents` is it, Blizzard's
    // deprecation file).
    ("C_DeathRecap.GetRecapEvents", "0: "),
    // Every client has these (Ketho's client dumps): no bonus, override or
    // vehicle bar, pet battle, soft target, minion, widgets-only nameplate,
    // tapped unit, raid target (`SetRaidTarget` sets none), arena
    // opponent or ready check; text to speech at its defaults. Recorded on
    // camelot, Retail 12.1.5, Mists 5.5.4, TBC 2.5.6 and Classic Era 1.15.9
    // (alone, no target, out of combat).
    ("C_ActionBar.GetBonusBarOffset", "1: number 0"),
    ("C_ActionBar.HasOverrideActionBar", "1: boolean false"),
    ("C_ActionBar.HasVehicleActionBar", "1: boolean false"),
    ("C_PetBattles.IsInBattle", "1: boolean false"),
    ("IsTargetLoose", "1: boolean false"),
    ("UnitIsMinion", "1: boolean false"),
    ("UnitNameplateShowsWidgetsOnly", "1: boolean false"),
    ("UnitIsTapDenied", "1: boolean false"),
    ("GetRaidTargetIndex", "1: nil nil"),
    ("GetNumArenaOpponentSpecs", "1: number 0"),
    ("GetArenaOpponentSpec", "0: "),
    ("GetReadyCheckStatus", "0: "),
    ("C_TTSSettings.GetSpeechRate", "1: number 0"),
    ("C_TTSSettings.GetSpeechVolume", "1: number 100"),
];

/// Every recorded answer: the function and its answer (the type
/// generator's evidence of undocumented functions, decision T294).
pub fn all() -> &'static [(&'static str, &'static str)] {
    BLANKS
}

/// The client's answer to a call of `name` when there is nothing to tell,
/// if the probe settled it.
pub fn answer(name: &str) -> Option<&'static str> {
    BLANKS.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
}

/// The behaviour of a function in [`BLANKS`]: its answer.
fn blank(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = crate::recorded::called_name(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(match answer(&name) {
        Some(a) => crate::recorded::parse_answer(vm, a),
        None => vec![],
    })
}

/// The functions' behaviours (fill-ins: the engine's own models and the
/// recordings answer first).
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    BLANKS
        .iter()
        .map(|(name, _)| (*name, blank as NativeFn))
        .collect()
}
