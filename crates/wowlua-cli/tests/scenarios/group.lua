-- /stdtest engine scenario group, played with the engine's controls: the
-- level 21 paladin invites another of the user's characters (a druid
-- standing by), who joins; targets it on its party frame, follows it from
-- the target's menu, asks to trade from the menu, and closes the trade
-- window (scenarios/group.lua). Each step at its recorded time (a typed
-- command or click: a round trip before the server's answer, ~0.45 s).
local e = __engine
local player = e.player
-- The paladin's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 10300, 19740, 1022, 1038, 19979, 25290, 5599, 19834, 10292, 19876, 25898, 19977, 10293, 7294, 19898, 19978, 19888, 20729, 19837, 10298, 465, 10278, 25782, 19850, 10290, 6940, 10301, 10299, 25916, 19896, 19899, 1044, 25890, 19838, 25895, 25291, 1032, 19854, 25918, 19836, 19742, 19900, 19746, 25894, 19853, 19895, 19897, 19852, 10291, 19835, 19891, 20217, 643 })
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
e.setLatency(0.453)
local druid = e.spawnUnit({
    name = "Player", lastName = "Lastname", guid = "Player-0000-00000000", player = true, level = 21,
    reaction = "friendly", class = "DRUID", classID = 11, sex = 3, powerType = 0, race = "Skyborne",
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde", pvp = false,
})
druid:setDistance(3)
player:softTarget(druid, "friend")
local now = 0
-- Time passes in frames, as in the client (60 a second: Blizzard's
-- animations and OnUpdate code run), the rest of a step as one jump.
local function at(t)
    if t < now then error(("at(%s): before %s"):format(t, now), 2) end
    local frames = math.floor((t - now) * 60)
    if frames > 0 then
        e.advanceFrames(frames, 1000 / 60)
    end
    e.advanceTime(math.max(0, t - now - frames / 60))
    now = t
end
local function press(frame, button)
    local ok, err = pcall(e.click, frame, button)
    if not ok then check("clicked: " .. tostring(err), false) end
end
-- The open menu's entry by its text.
local function menuEntry(text)
    local menu = Menu.GetManager():GetOpenMenu()
    local function find(f, depth)
        if depth > 6 then return nil end
        for _, r in ipairs({ f:GetRegions() }) do
            if r:IsObjectType("FontString") and r:GetText() == text then return f end
        end
        for _, child in ipairs({ f:GetChildren() }) do
            local found = find(child, depth + 1)
            if found then return found end
        end
    end
    return menu and find(menu, 0)
end
-- A shown button by its text, anywhere.
local function shownButton(text)
    local f = EnumerateFrames()
    while f do
        if not f:IsForbidden() and f:IsObjectType("Button") and f:IsVisible() then
            if f.GetText and f:GetText() == text then return f end
            for _, r in ipairs({ f:GetRegions() }) do
                if r:IsObjectType("FontString") and r:GetText() == text then return f end
            end
        end
        f = EnumerateFrames(f)
    end
end
-- The shown frame for a unit.
local function unitButton(unit)
    local f = EnumerateFrames()
    while f do
        if not f:IsForbidden() and f:IsObjectType("Button") and f:IsVisible() and f:GetAttribute("unit") == unit then
            return f
        end
        f = EnumerateFrames(f)
    end
end
-- The invitation typed.
at(3.3)
e.pressKey("ENTER")
e.type(ChatEdit_GetActiveWindow() or ChatFrame1EditBox, "/invite Player")
at(3.479) e.pressKey("ENTER")
at(4.5)
check("in a party", IsInGroup() and UnitIsGroupLeader("player"))
at(7.606) druid:joinGroup()
at(8.0)
check("two in the party", GetNumGroupMembers() == 2)
-- Tooltips' data updates as the mouse passed over the party frame, not
-- modelled: as recorded.
at(8.735) for id = 817, 821 do e.fireEvent("TOOLTIP_DATA_UPDATE", id) end
-- The campfire nearby, on the druid, later on the paladin following it
-- (the session's aura instances, recorded).
local function campfire(unit, instance)
    unit:applyAura({ spellId = 1283391, name = "Campfire Nearby", icon = 7808147, auraInstanceID = instance })
end
at(8.832) campfire(druid, 21)
at(9.499) e.newWmoChunk()
-- Targeted on its party frame.
at(10.525)
press(unitButton("party1"))
at(11.0)
check("the druid targeted", UnitIsUnit("target", "party1"))
-- Followed from the target's menu.
at(18.536) press(TargetFrame, "RightButton")
at(19.419) press(menuEntry("Follow"))
at(19.480) campfire(player, 51)
-- Asked to trade from the menu; the druid accepted; the window closed.
at(20.077) press(TargetFrame, "RightButton")
at(22.593) press(menuEntry("Trade"))
at(23.331) druid:openTrade()
at(23.5)
check("the trade window open", TradeFrame and TradeFrame:IsShown())
at(23.736) e.pressKey("ESCAPE")
-- The druid within the paladin's Devotion Aura (recorded).
at(23.900) druid:applyAura({
    spellId = 643, name = "Devotion Aura", icon = 135893, source = player, auraInstanceID = 23,
})
at(24.2)
check("the trade window closed", not TradeFrame:IsShown())
at(25.4)
