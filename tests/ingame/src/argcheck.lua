-- `/stdtest argcheck`: how the client checks documented functions'
-- arguments. Each call passes one kind of bad first argument to a harmless
-- documented query function (picked per argument type from the camelot
-- documentation) through pcall and records the error or the results'
-- types; wowlua's engine runs the same file and must give the same
-- (crates/wowlua-engine). Plain Lua, so the engine can load it as it is.
-- Calling C functions with bad arguments in bulk is untested ground: a
-- crash report's Lua stack shows the call in progress (`current`).
local _, ns = ...

-- A function, its first argument's documented type, whether it may be nil.
local calls = {
    { "C_AchievementInfo.GetRewardItemID", "number", false },
    { "C_AchievementInfo.GetSupercedingAchievements", "number", false },
    { "C_ActionBar.GetActionAutocast", "luaIndex", false },
    { "C_ActionBar.GetActionChargeDuration", "luaIndex", false },
    { "C_ActionBar.FindAssistedCombatActionButtons", "ActionBarSet", false },
    { "C_ActionBar.HasAssistedCombatActionButtons", "ActionBarSet", false },
    { "C_ClickBindings.CanSpellBeClickBound", "SpellIdentifier", false },
    { "C_MountJournal.GetMountFromSpell", "SpellIdentifier", false },
    { "C_Commentator.FindSpectatedUnit", "UnitToken", false },
    { "C_Commentator.GetTeamColorByUnit", "UnitToken", false },
    { "C_ArtifactUI.GetItemLevelIncreaseProvidedByRelic", "ItemInfo", false },
    { "C_ArtifactUI.GetPowersAffectedByRelicItemLink", "ItemInfo", false },
    { "C_AutoComplete.GetAutoCompletePresenceID", "cstring", false },
    { "C_CVar.GetCVar", "cstring", false },
    { "C_AccountInfo.GetIDFromBattleNetAccountGUID", "WOWGUID", false },
    { "C_AccountInfo.IsGUIDBattleNetAccountType", "WOWGUID", false },
    { "C_ArtifactUI.IsArtifactItem", "ItemLocation", false },
    { "C_AuctionHouse.GetAvailablePostCount", "ItemLocation", false },
    { "C_ChatInfo.IsValidChatLine", "number", true },
    { "C_ClassTalents.GetConfigIDsBySpecID", "number", true },
    { "C_ChatInfo.GetClubStreamIDs", "ClubId", false },
    { "C_Club.AreMembersReady", "ClubId", false },
    { "C_UnitAuras.GetAuraDataByAuraInstanceID", "UnitTokenRestrictedForAddOns", false },
    { "C_UnitAuras.GetAuraDataBySlot", "UnitTokenRestrictedForAddOns", false },
    { "C_AuctionHouse.GetExtraBrowseInfo", "ItemKey", false },
    { "C_AuctionHouse.GetItemKeyRequiredLevel", "ItemKey", false },
    { "C_ArtifactUI.GetNumRelicSlots", "bool", false },
    { "C_ChatBubbles.GetAllChatBubbles", "bool", false },
    { "C_EncounterTimeline.GetEventInfo", "EncounterTimelineEventID", false },
    { "C_EncounterTimeline.GetEventState", "EncounterTimelineEventID", false },
    { "C_VoiceChat.GetChannel", "VoiceChatID", false },
    { "C_VoiceChat.GetMemberGUID", "VoiceChatID", false },
    { "C_Club.GetLastTicketResponse", "string", false },
    { "C_ConsoleScriptCollection.GetCollectionDataByTag", "string", false },
    { "C_AzeriteEmpoweredItem.GetAllTierInfo", "AzeriteEmpoweredItemLocation", false },
    { "C_AzeriteEmpoweredItem.HasAnyUnselectedPowers", "AzeriteEmpoweredItemLocation", false },
    { "C_GuildInfo.GetGuildTabardInfo", "UnitToken", true },
    { "IsFalling", "UnitToken", true },
    { "C_AzeriteItem.GetAzeriteItemXPInfo", "AzeriteItemLocation", false },
    { "C_AzeriteItem.GetPowerLevel", "AzeriteItemLocation", false },
    { "C_Garrison.GetFollowerAutoCombatStats", "GarrisonFollower", false },
    { "C_Garrison.GetFollowerMissionCompleteInfo", "GarrisonFollower", false },
    { "C_PartyInfo.GetMinLevel", "luaIndex", true },
    { "C_PartyInfo.IsCrossFactionParty", "luaIndex", true },
    { "C_Texture.GetAtlasElementID", "textureAtlas", false },
    { "C_Texture.GetAtlasExists", "textureAtlas", false },
    { "C_UIFileAsset.GetFileID", "FileAsset", false },
    { "C_UIFileAsset.IsKnownFile", "FileAsset", false },
    { "C_GameRules.GetGameRuleAsFrameStrata", "GameRule", false },
    { "C_GameRules.IsGameRuleActive", "GameRule", false },
    { "C_CurrencyInfo.GetCoinIcon", "WOWMONEY", false },
    { "C_CurrencyInfo.GetCoinText", "WOWMONEY", false },
    { "C_TooltipInfo.GetPvpBrawl", "bool", true },
    { "C_SpecializationInfo.GetActiveSpecGroup", "bool", true },
    { "C_ClientScene.IsSceneTypeActive", "ClientSceneType", true },
    { "C_SpellDiminish.GetAllSpellDiminishCategories", "SpellDiminishRuleset", true },
    { "C_EventUtils.IsCallbackEvent", "stringView", false },
    { "C_EventUtils.IsEventValid", "stringView", false },
    { "C_Commentator.FindTeamNameInDirectory", "table", false },
    { "C_TradeSkillUI.GetCraftingTargetItems", "table", false },
    { "C_DateAndTime.GetCalendarTimeFromEpoch", "BigUInteger", false },
    { "C_TradeSkillUI.GetItemSlotModificationsForOrder", "BigUInteger", false },
    { "C_HousingCatalog.GetAllVariantInfosForEntry", "HousingCatalogEntryID", false },
    { "C_HousingCatalog.GetCatalogEntryInfo", "HousingCatalogEntryID", false },
    { "C_VoiceChat.GetMemberVolume", "PlayerLocation", false },
    { "C_VoiceChat.IsMemberMuted", "PlayerLocation", false },
    { "C_TooltipComparison.GetItemComparisonInfo", "TooltipComparisonItem", false },
    { "C_TooltipComparison.GetItemComparisonDelta", "TooltipComparisonItem", false },
    { "C_NamePlateManager.IsNamePlateUnitBehindCamera", "UnitTokenNamePlate", false },
    { "C_NamePlate.GetNamePlateForUnit", "UnitTokenNamePlate", false },
    { "C_TooltipInfo.GetUnit", "UnitTokenPvPRestrictedForAddOns", false },
    { "GetComboPoints", "UnitTokenPvPRestrictedForAddOns", false },
    { "C_WeeklyRewards.GetActivityEncounterInfo", "WeeklyRewardChestThresholdType", false },
    { "C_WeeklyRewards.GetSortedProgressForActivity", "WeeklyRewardChestThresholdType", false },
    { "C_TooltipInfo.GetWeeklyReward", "WeeklyRewardItemDBID", false },
    { "C_WeeklyRewards.GetItemHyperlink", "WeeklyRewardItemDBID", false },
    { "C_Calendar.GetEventIndexInfo", "CalendarEventID", false },
    { "C_CraftingOrders.GetCustomerOptions", "CraftingOrderCustomerSearchParams", false },
    { "C_TradeSkillUI.GetDependentReagents", "CraftingReagent", false },
    { "C_Item.DoesItemExist", "EmptiableItemLocation", false },
    { "GetForbiddenObjectTable", "FrameScriptObject", false },
    { "C_GamePad.GetConfig", "GamePadConfigID", false },
    { "C_HousingCatalog.GetCatalogEntryVariantInfo", "HousingCatalogEntryVariantID", false },
    { "C_TransmogOutfitInfo.GetTransmogOutfitSlotFromInventorySlot", "InventorySlots", false },
    { "C_LegendaryCrafting.GetRuneforgePowers", "ItemLocation", true },
    { "C_PaperDollInfo.GetTemporaryEnchantmentInfo", "LuaInventorySlot", false },
    { "GetFontInfo", "SimpleFont", false },
    { "GetFrameCPUUsage", "SimpleFrame", false },
    { "C_SpecializationInfo.GetTalentInfo", "TalentInfoQuery", false },
    { "C_Transmog.GetSlotVisualInfo", "TransmogLocation", false },
    { "C_TransmogOutfitInfo.GetOutfitSituation", "TransmogSituationOption", false },
    { "C_Housing.GetOthersOwnedHouses", "WOWGUID", true },
    { "C_WeeklyRewards.GetActivities", "WeeklyRewardChestThresholdType", true },
    { "GetDefaultAbbreviationBreakpoints", "WowLocale", true },
    { "GetMouseButtonName", "mouseButton", false },
    { "IsMouseButtonDown", "mouseButton", true },
    { "C_Club.GetInvitationCandidates", "string", true },
}

-- The first arguments tried, by label.
local variants = {
    { "no arguments", function() end },
    { "nil", function() return nil end },
    { "true", function() return true end },
    { "'x'", function() return "x" end },
    { "'1'", function() return "1" end },
    { "1.5", function() return 1.5 end },
    { "-1", function() return -1 end },
    { "0", function() return 0 end },
    { "2^31", function() return 2 ^ 31 end },
    { "2^32", function() return 2 ^ 32 end },
    { "{}", function() return {} end },
    { "print", function() return print end },
    { "secret 1", function() return secretwrap(1) end },
    { "secret 'x'", function() return secretwrap("x") end },
}

local isSecret = issecretvalue or function()
    return false
end

-- A call's outcome: its error, or the types of its results.
local function describe(ok, ...)
    if not ok then
        local e = ...
        return "error: " .. (isSecret(e) and "<secret>" or tostring(e))
    end
    local types = {}
    for i = 1, select("#", ...) do
        local v = select(i, ...)
        types[#types + 1] = (isSecret(v) and "secret " or "") .. type(v)
    end
    return select("#", ...) .. ": " .. table.concat(types, ", ")
end

-- The function a dotted name names, if the client has it.
local function resolve(name)
    local value = _G
    for part in name:gmatch("[^.]+") do
        if type(value) ~= "table" then
            return nil
        end
        value = value[part]
    end
    return value
end

local function run()
    local results = {}
    for _, call in ipairs(calls) do
        local name = call[1]
        local f = resolve(name)
        local row = {}
        if type(f) ~= "function" then
            row["absent"] = "absent"
        else
            for _, variant in ipairs(variants) do
                local current = name .. "(" .. variant[1] .. ")"
                row[variant[1]] = describe(pcall(f, variant[2]()))
                current = nil
            end
        end
        results[name] = row
    end
    return results
end

ns.__wowlua.define("argcheck", { run = run, calls = calls })
