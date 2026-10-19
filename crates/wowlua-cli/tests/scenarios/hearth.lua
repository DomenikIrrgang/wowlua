-- /stdtest engine scenario hearth, played with the engine's controls: the
-- level 21 paladin (Horde, at the Crossroads) right-clicks its Hearthstone
-- in the backpack, casts it for ten seconds and goes through the loading
-- screen (scenarios/hearth.lua). Each step at its recorded time, the ping
-- as recorded.
local e = __engine
local player = e.player
-- The paladin's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 10300, 19740, 1022, 1038, 19979, 25290, 5599, 19834, 10292, 19876, 25898, 19977, 10293, 7294, 19898, 19978, 19888, 20729, 19837, 10298, 465, 10278, 25782, 19850, 10290, 6940, 10301, 10299, 25916, 19896, 19899, 1044, 25890, 19838, 25895, 25291, 1032, 19854, 25918, 19836, 19742, 19900, 19746, 25894, 19853, 19895, 19897, 19852, 10291, 19835, 19891, 20217, 643 })
-- Judgement: off the global cooldown (recorded).
e.setSpellTraits({ offGlobalCooldown = { 20271 }, partyOrSelf = { 1022 } })
-- The Hearthstone: its spell, cast time, cooldown category, the spells
-- sharing its cooldown, the place it takes the player to (recorded).
e.defineItem(6948, {
    name = "Hearthstone", quality = 1, icon = 134414, stackCount = 1, classID = 15, subclassID = 0,
    equipLoc = "INVTYPE_NON_EQUIP_IGNORE", sellPrice = 0,
    link = "|cnIQ1:|Hitem:6948::::::::21:1486::75:::::::|h[Hearthstone]|h|r",
    spell = 8690, castTime = 10,
    aura = { spellId = 2479, name = "Honorless Target", icon = 135994, duration = 30 }, cooldownCategory = 89, sharedCooldowns = { 2479, 441924 },
    teleport = {
        map = 1, zone = "Thunder Bluff", subZone = "", mapID = 1456, instance = "Kalimdor", instanceID = 1,
        pvp = { "friendly", false, "Horde" },
    },
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
-- The bags opened as the user does (their key), a bag slot's button.
local function bagButton(bag, slot)
    for _, name in ipairs({ "ContainerFrameCombinedBags", "ContainerFrame1", "ContainerFrame2", "ContainerFrame3", "ContainerFrame4", "ContainerFrame5", "ContainerFrame6" }) do
        local f = _G[name]
        if f and f:IsShown() and f.EnumerateValidItems then
            for _, b in f:EnumerateValidItems() do
                if b:GetBagID() == bag and b:GetID() == slot then return b end
            end
        end
    end
end
e.pressKey("B")
at(7.122)
-- The cast's round trip as it went (its start 0.335 s after the click;
-- GetNetStats said 0.245).
e.setLatency(0.335)
local stone = bagButton(0, 1)
check("the Hearthstone shown", stone ~= nil)
if stone then press(stone, "RightButton") end
-- The client's frame the cast's start came in.
at(7.457)
at(8)
check("casting: the cast bar", PlayerCastingBarFrame:IsShown())
at(25.3)
check("in the world again: the cast bar gone", not PlayerCastingBarFrame:IsShown())
