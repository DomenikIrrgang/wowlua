-- The character's world as the full probe found it (/stdtest engine
-- fullprobe, forever-camelot-fullprobe.lua), set with the engine's controls
-- on top of the last snapshot recorded before it (scenarios/mapnav-after,
-- 00:43; the probe ran from about 01:00 to 02:44). Each change is what the
-- probe's own answers say. The probe ran over many sessions (a /reload
-- after each segment of calls): where they differ, the most answers'
-- session is the one set here.
local e = __engine
local player = e.player

-- The player: the snapshot's character, its last name (`UnitName("player")`:
-- "Player", "Lastname"), its GUID as the recording scrubbed it
-- (`UnitGUID("player")`), its mana (`UnitPowerMax("player")`: 875 in every
-- session; the snapshot's 755).
e.setPlayer({
    name = "Player",
    lastName = "Lastname",
    realm = "Realm",
    guid = "Player-0000-00000000",
    class = "PALADIN",
    race = "Scourge",
    sex = 2,
    level = 21,
    faction = "Horde",
    maxHealth = 856,
    maxPower = 875,
    powerType = 0,
})

-- Its one aura: Blessing of Kings, its own (`C_UnitAuras.GetAuraDataByIndex(
-- "player", 1)`, 25 answers: instance 3, cast by the player, Magic, a raid
-- buff, 3600.022 seconds, running out at 208096.694). Other sessions had it
-- cast again (other instances and times), one another paladin's too.
for i = 1, 40 do
    local aura = C_UnitAuras.GetAuraDataByIndex("player", i, "HELPFUL")
    if aura == nil then break end
    player:removeAura(aura.auraInstanceID)
end
player:applyAura({
    spellId = 20217,
    name = "Blessing of Kings",
    icon = 135995,
    auraInstanceID = 3,
    sourceUnit = "player",
    dispelName = "Magic",
    raid = true,
    duration = 3600.022,
    expirationTime = 208096.694,
    quiet = true,
})

-- The chat channels: LookingForGroup (the snapshot's 1) left, the others
-- keeping their numbers (`GetChannelList()`: General 2 to TradeLocal 6;
-- `GetNumGroupChannels()`: 1).
e.setChannels({
    joined = {
        { name = "General", fullName = "General - The Barrens", category = "CHANNEL_CATEGORY_WORLD", shortcut = "General", zoneChannelID = 1, channelType = 1, number = 2 },
        { name = "Trade", fullName = "Trade", category = "CHANNEL_CATEGORY_WORLD", disabled = true, shortcut = "Trade", zoneChannelID = 2, channelType = 1 },
        { name = "LocalDefense", fullName = "LocalDefense - The Barrens", category = "CHANNEL_CATEGORY_WORLD", shortcut = "LocalDefense", zoneChannelID = 22, channelType = 1 },
        { name = "Services", fullName = "Services", category = "CHANNEL_CATEGORY_WORLD", disabled = true, shortcut = "Services", zoneChannelID = 42, channelType = 1 },
        { name = "TradeLocal", fullName = "TradeLocal", category = "CHANNEL_CATEGORY_WORLD", disabled = true, shortcut = "TradeLocal", zoneChannelID = 46, channelType = 1 },
    },
    groupChannels = 1,
    regionalService = true,
})

-- Mail from the auction house waiting (`HasNewMail()`: true;
-- `GetLatestThreeSenders()`: "Horde Auction House").
e.setNewMail({ "Horde Auction House" })

-- A character macro besides the account's "Test" (`GetNumMacros()`: 1, 1).
-- Which one was not asked (the probe reads macros 1 and 2): the macro
-- scenario's "Test" stands in for it (unverified).
e.setMacros({
    account = { { name = "Test", icon = 841219, body = "/whisper Player Lastname hello\n" } },
    character = { { name = "Test", icon = 134400, body = "/say hi\n" } },
})

-- Two equipment sets of the ten items worn, none ignored, no spec
-- (`C_EquipmentSet.GetEquipmentSetIDs()`: 0, 1; `GetEquipmentSetInfo`:
-- "Test" with icon 133361 and "Test2" with 135821, all ten worn;
-- `GetItemIDs(0)` and `(1)` the gear's items).
local worn = {}
for slot = 1, 19 do
    worn[slot] = GetInventoryItemID("player", slot)
end
e.setEquipmentSets({
    { name = "Test", icon = 133361, items = worn },
    { name = "Test2", icon = 135821, items = worn },
})

-- No quest selected in the log (`C_QuestLog.GetSelectedQuest()`: 0; the
-- snapshot's 6562, the map's).
player:setStats({ selectedQuest = { 0 } })

-- The bags, rearranged since the snapshot and a Secure Crate looted. The
-- probe's answers: the backpack's first slot the crate (a quest item,
-- `C_Container.GetContainerItemInfo(0, 1)`), its slots 2 and 3 free
-- (`GetContainerFreeSlots(0)`), the Hearthstone the next item (the
-- probe's second item, {ITEM2}); bag 1 full (`GetContainerFreeSlots(1)`:
-- none), its first slot the Apprentice's Mining Pack (the snapshot's
-- backpack slot 12); the reagent bag full, its first slot the Copper Bar
-- (the snapshot's last). Where the snapshot's other items went is not
-- known: they fill the other slots in the snapshot's order (fitted,
-- unverified), the backpack's last two moved to bag 1.
local function held(bag, slot)
    local info = C_Container.GetContainerItemInfo(bag, slot)
    local quest = C_Container.GetContainerItemQuestInfo(bag, slot)
    return {
        id = info.itemID,
        count = info.stackCount,
        link = info.hyperlink,
        bound = info.isBound,
        questItem = quest.isQuestItem,
        questID = quest.questID,
        questActive = quest.isActive,
    }
end
local function contents(bag, slots)
    local out = {}
    for slot = 1, slots do
        if C_Container.GetContainerItemID(bag, slot) then
            out[#out + 1] = held(bag, slot)
        end
    end
    return out
end
local backpack, basket, reagents = contents(0, 20), contents(1, 6), contents(5, 6)
local function take(list, id)
    for i, item in ipairs(list) do
        if item.id == id then return table.remove(list, i) end
    end
end
local hearthstone = take(backpack, 6948)
local pack = take(backpack, 277115)
local copper = take(reagents, 2840)
local bag0 = { [1] = { id = 6462, count = 1, bound = true, questItem = true }, [4] = hearthstone }
for slot = 5, 20 do bag0[slot] = table.remove(backpack, 1) end
local bag1 = { pack }
for _, item in ipairs(basket) do bag1[#bag1 + 1] = item end
for _, item in ipairs(backpack) do bag1[#bag1 + 1] = item end
local bag5 = { copper }
for _, item in ipairs(reagents) do bag5[#bag5 + 1] = item end
e.setBag(0, { slots = 20, items = bag0 })
e.setBag(1, { slots = 6, items = bag1 })
e.setBag(5, { slots = 6, items = bag5 })
