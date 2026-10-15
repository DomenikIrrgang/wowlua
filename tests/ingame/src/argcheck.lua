-- `/stdtest argcheck`: how the client checks documented functions'
-- arguments. Each call passes one kind of bad first argument to a harmless
-- documented query function (picked per argument type from the camelot
-- documentation) through pcall and records the error or the results'
-- types; wowlua's engine runs the same file and must give the same
-- (crates/wowlua-engine). Plain Lua, so the engine can load it as it is.
-- Calling C functions with bad arguments in bulk is untested ground: a
-- crash report's Lua stack shows the call in progress (`current`).
local _, ns = ...

-- A function, its first argument's documented type, whether it may be
-- nil, the Lua type that type is, and the variants tried: only values of
-- another type, which the argument check refuses before the game's code
-- runs. A value the type takes (or converts) reaches the game's code, which
-- may assert on it: C_Garrison.GetFollowerMissionCompleteInfo("1") crashed
-- the client (GarrisonFollower.cpp, 2026-10-16), so it is left out.
local calls = {
    { "C_AchievementInfo.GetRewardItemID", "number", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_AchievementInfo.GetSupercedingAchievements", "number", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ActionBar.GetActionAutocast", "luaIndex", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ActionBar.GetActionChargeDuration", "luaIndex", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ActionBar.FindAssistedCombatActionButtons", "ActionBarSet", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ActionBar.HasAssistedCombatActionButtons", "ActionBarSet", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ClickBindings.CanSpellBeClickBound", "SpellIdentifier", false, "number+string", { "no arguments", "nil", "true", "{}", "print" } },
    { "C_MountJournal.GetMountFromSpell", "SpellIdentifier", false, "number+string", { "no arguments", "nil", "true", "{}", "print" } },
    { "C_Commentator.FindSpectatedUnit", "UnitToken", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_Commentator.GetTeamColorByUnit", "UnitToken", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ArtifactUI.GetItemLevelIncreaseProvidedByRelic", "ItemInfo", false, "number+string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ArtifactUI.GetPowersAffectedByRelicItemLink", "ItemInfo", false, "number+string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_AutoComplete.GetAutoCompletePresenceID", "cstring", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_CVar.GetCVar", "cstring", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_AccountInfo.GetIDFromBattleNetAccountGUID", "WOWGUID", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_AccountInfo.IsGUIDBattleNetAccountType", "WOWGUID", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ArtifactUI.IsArtifactItem", "ItemLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_AuctionHouse.GetAvailablePostCount", "ItemLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_ChatInfo.IsValidChatLine", "number", true, "number", { "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ClassTalents.GetConfigIDsBySpecID", "number", true, "number", { "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ChatInfo.GetClubStreamIDs", "ClubId", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_Club.AreMembersReady", "ClubId", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_UnitAuras.GetAuraDataByAuraInstanceID", "UnitTokenRestrictedForAddOns", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_UnitAuras.GetAuraDataBySlot", "UnitTokenRestrictedForAddOns", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_AuctionHouse.GetExtraBrowseInfo", "ItemKey", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_AuctionHouse.GetItemKeyRequiredLevel", "ItemKey", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_EncounterTimeline.GetEventInfo", "EncounterTimelineEventID", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_EncounterTimeline.GetEventState", "EncounterTimelineEventID", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_Club.GetLastTicketResponse", "string", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ConsoleScriptCollection.GetCollectionDataByTag", "string", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_AzeriteEmpoweredItem.GetAllTierInfo", "AzeriteEmpoweredItemLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_AzeriteEmpoweredItem.HasAnyUnselectedPowers", "AzeriteEmpoweredItemLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_GuildInfo.GetGuildTabardInfo", "UnitToken", true, "string", { "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "IsFalling", "UnitToken", true, "string", { "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_AzeriteItem.GetAzeriteItemXPInfo", "AzeriteItemLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_AzeriteItem.GetPowerLevel", "AzeriteItemLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_Garrison.GetFollowerAutoCombatStats", "GarrisonFollower", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_PartyInfo.GetMinLevel", "luaIndex", true, "number", { "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_PartyInfo.IsCrossFactionParty", "luaIndex", true, "number", { "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_Texture.GetAtlasElementID", "textureAtlas", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_Texture.GetAtlasExists", "textureAtlas", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_UIFileAsset.GetFileID", "FileAsset", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_UIFileAsset.IsKnownFile", "FileAsset", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_CurrencyInfo.GetCoinIcon", "WOWMONEY", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_CurrencyInfo.GetCoinText", "WOWMONEY", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_ClientScene.IsSceneTypeActive", "ClientSceneType", true, "number", { "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_SpellDiminish.GetAllSpellDiminishCategories", "SpellDiminishRuleset", true, "number", { "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_EventUtils.IsCallbackEvent", "stringView", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_EventUtils.IsEventValid", "stringView", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_Commentator.FindTeamNameInDirectory", "table", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_TradeSkillUI.GetCraftingTargetItems", "table", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_DateAndTime.GetCalendarTimeFromEpoch", "BigUInteger", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_TradeSkillUI.GetItemSlotModificationsForOrder", "BigUInteger", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_HousingCatalog.GetAllVariantInfosForEntry", "HousingCatalogEntryID", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_HousingCatalog.GetCatalogEntryInfo", "HousingCatalogEntryID", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_VoiceChat.GetMemberVolume", "PlayerLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_VoiceChat.IsMemberMuted", "PlayerLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_TooltipComparison.GetItemComparisonInfo", "TooltipComparisonItem", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_TooltipComparison.GetItemComparisonDelta", "TooltipComparisonItem", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_NamePlateManager.IsNamePlateUnitBehindCamera", "UnitTokenNamePlate", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_NamePlate.GetNamePlateForUnit", "UnitTokenNamePlate", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_TooltipInfo.GetUnit", "UnitTokenPvPRestrictedForAddOns", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "GetComboPoints", "UnitTokenPvPRestrictedForAddOns", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_TooltipInfo.GetWeeklyReward", "WeeklyRewardItemDBID", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_WeeklyRewards.GetItemHyperlink", "WeeklyRewardItemDBID", false, "string", { "no arguments", "nil", "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_Calendar.GetEventIndexInfo", "CalendarEventID", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_CraftingOrders.GetCustomerOptions", "CraftingOrderCustomerSearchParams", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_TradeSkillUI.GetDependentReagents", "CraftingReagent", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_Item.DoesItemExist", "EmptiableItemLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "GetForbiddenObjectTable", "FrameScriptObject", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_GamePad.GetConfig", "GamePadConfigID", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_HousingCatalog.GetCatalogEntryVariantInfo", "HousingCatalogEntryVariantID", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_TransmogOutfitInfo.GetTransmogOutfitSlotFromInventorySlot", "InventorySlots", false, "number", { "no arguments", "nil", "true", "'x'", "{}", "print", "secret 1", "secret 'x'" } },
    { "C_LegendaryCrafting.GetRuneforgePowers", "ItemLocation", true, "table", { "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "GetFontInfo", "SimpleFont", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "GetFrameCPUUsage", "SimpleFrame", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_SpecializationInfo.GetTalentInfo", "TalentInfoQuery", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_Transmog.GetSlotVisualInfo", "TransmogLocation", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_TransmogOutfitInfo.GetOutfitSituation", "TransmogSituationOption", false, "table", { "no arguments", "nil", "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_Housing.GetOthersOwnedHouses", "WOWGUID", true, "string", { "true", "{}", "print", "secret 1", "secret 'x'" } },
    { "GetDefaultAbbreviationBreakpoints", "WowLocale", true, "table", { "true", "'x'", "'1'", "print", "secret 1", "secret 'x'" } },
    { "C_Club.GetInvitationCandidates", "string", true, "string", { "true", "{}", "print", "secret 1", "secret 'x'" } },
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
            local allowed = {}
            for _, label in ipairs(call[5]) do
                allowed[label] = true
            end
            for _, variant in ipairs(variants) do
                if allowed[variant[1]] then
                    local current = name .. "(" .. variant[1] .. ")"
                    row[variant[1]] = describe(pcall(f, variant[2]()))
                    current = nil
                end
            end
        end
        results[name] = row
    end
    -- Called from Lua, not through pcall: the error names the function and
    -- the caller's line as the client writes them.
    local direct = {}
    local function directly(label, f)
        local ok, e = pcall(f)
        direct[label] = ok and "no error" or (isSecret(e) and "<secret>" or tostring(e))
    end
    directly("C_CVar.GetCVar()", function() return C_CVar.GetCVar() end)
    directly("C_CVar.GetCVar({})", function() local r = C_CVar.GetCVar({}) return r end)
    directly("C_ActionBar.GetActionAutocast('x')", function() local r = C_ActionBar.GetActionAutocast("x") return r end)
    directly("C_UnitAuras.GetAuraDataBySlot(true)", function() local r = C_UnitAuras.GetAuraDataBySlot(true) return r end)
    directly("C_CVar.GetCVar(secret)", function() local r = C_CVar.GetCVar(secretwrap("x")) return r end)
    directly("local f = C_CVar.GetCVar; f()", function() local f = C_CVar.GetCVar local r = f() return r end)
    results["(direct calls)"] = direct
    return results
end

ns.__wowlua.define("argcheck", { run = run, calls = calls })
