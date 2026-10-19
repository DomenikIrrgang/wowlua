-- /stdtest engine scenario inspect, played with the engine's controls:
-- the level 21 paladin targets a hunter (another of the user's characters),
-- inspects it from the target's menu, then views its talents
-- (scenarios/inspect.lua). Each step at its recorded time (a click:
-- a round trip before the server's answer, ~0.5 s).
local e = __engine
local player = e.player
-- The paladin's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 10300, 19740, 1022, 1038, 19979, 25290, 5599, 19834, 10292, 19876, 25898, 19977, 10293, 7294, 19898, 19978, 19888, 20729, 19837, 10298, 465, 10278, 25782, 19850, 10290, 6940, 10301, 10299, 25916, 19896, 19899, 1044, 25890, 19838, 25895, 25291, 1032, 19854, 25918, 19836, 19742, 19900, 19746, 25894, 19853, 19895, 19897, 19852, 10291, 19835, 19891, 20217, 643 })
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
local hunter = e.spawnUnit({
    name = "Player", lastName = "Lastname", guid = "Player-0000-00000000", player = true, level = 17,
    reaction = "friendly", class = "HUNTER", classID = 3, sex = 2, powerType = 0, race = "Tauren",
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde", pvp = false,
})
hunter:setDistance(3)
-- What the server knows of it: its specialization, item level, gear (the
-- links the client had cached at once, the others' infos after, in the
-- recorded order), its talent config.
hunter:setInspect({
    spec = 1485, itemLevel = 8,
    items = {
        [4] = { link = "|cnIQ1:|Hitem:5107::::::::21:1486:::::::::|h[Deckhand's Shirt]|h|r" },
        [5] = { link = "|cnIQ2:|Hitem:5957::::::::21:1486:::::::::|h[Handstitched Leather Vest]|h|r" },
        [6] = { link = "|cnIQ2:|Hitem:4246::::::::21:1486:::::::::|h[Fine Leather Belt]|h|r" },
        [7] = { link = "|cnIQ2:|Hitem:3287::::::::21:1486:::::::::|h[Tribal Pants]|h|r" },
        [8] = { link = "|cnIQ3:|Hitem:252425::::::::21:1486:::::::::|h[Dark Leather Boots]|h|r" },
        [9] = { link = "|cnIQ2:|Hitem:7281::::::::21:1486:::::::::|h[Light Leather Bracers]|h|r" },
        [10] = { link = "|cnIQ2:|Hitem:15310::::::::21:1486:::1:12733::::::|h[Feral Gloves of Power]|h|r", cached = true },
        [15] = { link = "|cnIQ2:|Hitem:14365::::::::21:1486:::::::::|h[Mystic's Cape]|h|r", cached = true },
        [16] = { link = "|cnIQ2:|Hitem:3184::::::::21:1486::1:1:12733:1:28:5249:::::|h[Hook Dagger of Power]|h|r" },
        [18] = { link = "|cnIQ3:|Hitem:272999::::::::21:1486::1::1:28:5244:::::|h[Barbaric Crossbow]|h|r", cached = true },
    },
    received = { 3287, 252425, 3184, 7281, 4246, 5107, 5957 },
    config = { ID = -1, name = "Hunter", treeIDs = { 1091 }, type = 4, usesSharedActionBars = false },
    -- Its talents: the recorded hunter's (its class config's ID).
    traitCharacter = 4226427,
})
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
e.setLatency(0.54)
at(1.754) player:target(hunter)
at(4.324) press(TargetFrame, "RightButton")
at(5.192) press(menuEntry("Inspect"))
at(6.0)
check("the inspect frame shown", InspectFrame and InspectFrame:IsShown())
check("its gloves shown", GetInventoryItemLink("target", 10) ~= nil)
-- Its talents from the menu.
at(9.742) press(TargetFrame, "RightButton")
at(10.581) press(menuEntry("View Talents"))
at(11.5)
check("its talents shown", PlayerSpellsFrame and PlayerSpellsFrame:IsShown())
at(12.0)
