-- /stdtest engine scenario items, played with the engine's controls: the
-- level 11 shaman (the second account's) right-clicks a two-handed hammer
-- (refused: the shield would need a bag slot, the bags are full), swaps
-- its shirt and back, eats a Skywall Souffle, picks up a shirt and puts it
-- back, deletes its Windstones, takes off its boots into the freed slot
-- and puts them back on (scenarios/items.lua). Each step at its recorded
-- time (a click: when the client acted), the ping as recorded.
local e = __engine
local player = e.player
-- The shaman's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 8185, 10613, 10478, 10534, 10441, 15036, 10609, 8836, 15111, 8249, 1239242, 10491, 17354, 8076, 8162, 15107, 10537, 16176, 8160, 10535, 15112, 8184, 8163, 10538, 10614, 16191, 17355, 5677, 10526, 8835, 17360, 10494, 8515, 8182, 10497, 15109, 10496, 8512, 16190, 10493, 8181, 10626, 8230, 10479, 1239243, 25362, 10495, 8227, 15108, 15110, 408521, 1325257, 10627, 10477, 17359, 16387, 8075, 1325256, 10521, 25360, 10442, 25361, 10476, 5678, 10612, 8161, 25359, 8250 })
-- The food: its use spell, the food's aura spell before it, the
-- cooldown's category (recorded).
e.defineItem(263509, {
    name = "Skywall Souffle", quality = 1, icon = 651570, stackCount = 20, classID = 0, subclassID = 5,
    equipLoc = "INVTYPE_NON_EQUIP_IGNORE", sellPrice = 100,
    link = "|cnIQ1:|Hitem:263509::::::::11:1489:::::::::|h[Skywall Souffle]|h|r",
    spell = 1248377, castSpells = { 433 }, cooldownCategory = 11,
    -- Eating, then Well Fed 10 s later: 10 more health (recorded; the
    -- session's aura instances).
    aura = { spellId = 433, name = "Food", icon = 134062, duration = 18, auraInstanceID = 47 },
    wellFed = {
        maxHealth = 10, cooldowns = { 1243969, 1248406 },
        aura = { spellId = 1248406, name = "Well Fed", icon = 133943, duration = 900, auraInstanceID = 51 },
    },
})
-- Tough Jerky on the bars (slots 12, 184): food too.
e.defineItem(117, { name = "Tough Jerky", quality = 1, icon = 133972, stackCount = 20, classID = 0, subclassID = 5 })
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
-- The bags opened as the user does (their key: opened from addon code,
-- their buttons would be tainted), a bag slot's button right-clicked.
e.pressKey("SHIFT-B")
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
local function rightClick(bag, slot)
    local b = bagButton(bag, slot)
    check(("bag %d slot %d shown"):format(bag, slot), b ~= nil)
    if b then press(b, "RightButton") end
end
local function leftClick(bag, slot)
    local b = bagButton(bag, slot)
    check(("bag %d slot %d shown"):format(bag, slot), b ~= nil)
    if b then press(b, "LeftButton") end
end
-- The hammer: refused.
at(18.489) rightClick(1, 4)
at(19.5)
check("the hammer stays in its bag", C_Container.GetContainerItemID(1, 4) == 1514)
-- The shirts swapped, and back.
at(21.172) rightClick(1, 5)
at(22.1)
check("the airy shirt worn", GetInventoryItemID("player", 4) == 273815)
at(22.746) rightClick(1, 5)
-- Eaten.
at(28.193) rightClick(0, 7)
at(30)
check("one souffle fewer", C_Container.GetContainerItemInfo(0, 7).stackCount == 2)
-- A shirt picked up and put back.
at(38.320) leftClick(1, 5)
at(38.949) leftClick(1, 5)
-- The Windstones deleted.
at(39.989) leftClick(1, 3)
at(40.280) e.clickWorld()
at(40.5)
check("asked to delete", StaticPopup1:IsShown() and StaticPopup1.which == "DELETE_ITEM")
at(41.226) press(StaticPopup1Button1)
at(42)
check("deleted", C_Container.GetContainerItemID(1, 3) == nil)
-- The boots off into the freed slot, then on again.
at(42.077) e.pressKey("C")
press(CharacterFeetSlot)
at(43.120) leftClick(1, 3)
at(43.9)
check("the boots in the bag", C_Container.GetContainerItemID(1, 3) == 1367 and GetInventoryItemID("player", 8) == nil)
at(44.083) rightClick(1, 3)
at(48)
check("the boots worn again", GetInventoryItemID("player", 8) == 1367)
