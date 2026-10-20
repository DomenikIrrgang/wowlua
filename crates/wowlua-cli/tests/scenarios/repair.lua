-- /stdtest engine scenario repair, played with the engine's controls: the
-- level 21 paladin in the Crossroads, its character frame open, talks to
-- Jahan Hawkwing (a leather merchant who repairs), opens his goods, takes
-- the repair cursor and mends the boots on the character frame, leaves
-- with Escape; talks to him again, mends the rest with Repair All and
-- leaves (scenarios/repair.lua). Each step at its recorded time (a
-- click's release), each answer's round trip as recorded.
local e = __engine
local player = e.player
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
local jahan = e.spawnUnit({
    name = "Jahan Hawkwing", guid = "Creature-0-4621-1-21-3483-F2E0F46302", npcID = 3483, level = 21,
    reaction = "friendly", class = "WARRIOR", classID = 1, sex = 2, powerType = 1, pvp = true,
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde",
})
-- His goods as the client listed them (their links name them): ID, name,
-- price, icon.
local goods = {
    { 846, "Tanned Leather Jerkin", 1452, 132760 }, { 1843, "Tanned Leather Belt", 725, 132512 },
    { 845, "Tanned Leather Pants", 1447, 134706 }, { 843, "Tanned Leather Boots", 1077, 132592 },
    { 1844, "Tanned Leather Bracers", 728, 132607 }, { 844, "Tanned Leather Gloves", 720, 132939 },
    { 847, "Chainmail Armor", 1749, 132624 }, { 1845, "Chainmail Belt", 877, 132492 },
    { 848, "Chainmail Pants", 1755, 134583 }, { 849, "Chainmail Boots", 1326, 132535 },
    { 1846, "Chainmail Bracers", 880, 132606 }, { 850, "Chainmail Gloves", 883, 132938 },
}
local items = {}
for i, g in ipairs(goods) do
    e.defineItem(g[1], { name = g[2], quality = 1, icon = g[4], stackCount = 1 })
    items[i] = { id = g[1], price = g[3] }
end
-- The repairs' costs by worn slot: the boots' 26 (recorded: the money
-- told), the rest's 228 together (recorded; how they split is not); Repair
-- All's money told after five of the six items' durability (the server's
-- packets).
e.defineGossip(jahan, {
    text = "The Barrens, with its hot sun and hostile denizens, is the perfect place to test your strength and fortitude.",
    options = { {
        name = "Let me browse your goods.", icon = 132060, gossipOptionID = 95584, opens = "merchant",
        merchant = {
            name = "Jahan Hawkwing", canRepair = true, cached = true, items = items, repairMoneyAfter = 5,
            repairCosts = { [8] = 26, [3] = 43, [5] = 68, [6] = 25, [9] = 25, [10] = 18, [16] = 49 },
        },
    } },
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
-- A shown button by its text.
local function button(root, text, depth)
    depth = depth or 0
    if depth > 8 or not root or root:IsForbidden() then return nil end
    if root:IsObjectType("Button") and root:IsVisible() then
        if root.GetText and root:GetText() == text then return root end
        for _, r in ipairs({ root:GetRegions() }) do
            if r:IsObjectType("FontString") and r:GetText() == text then return root end
        end
    end
    for _, c in ipairs({ root:GetChildren() }) do
        local found = button(c, text, depth + 1)
        if found then return found end
    end
end
-- The world's own, not modelled: the server's events as recorded.
local function world(...) e.fireEvent(...) end
-- The character frame open before the recording began (its key, as the
-- user opened it).
e.pressKey("C")
at(18.544) world("MOUNT_JOURNAL_USABILITY_CHANGED")
at(18.550) world("AREA_POIS_UPDATED")
world("FOG_OF_WAR_UPDATED")
-- Jahan targeted and talked to: the target's echo 0.306 s later, his
-- gossip 0.491 s.
jahan:setDistance(3)
e.setLatency(0.306)
at(19.299) player:target(jahan)
e.setLatency(0.491)
player:interact(jahan)
-- Away from the campfire.
at(19.605) player:removeAura(39)
at(20.2)
check("the gossip shown", GossipFrame:IsShown())
-- His goods (0.402 s).
local option = button(GossipFrame, "Let me browse your goods.")
check("the option shown", option ~= nil)
e.setLatency(0.402)
at(20.959) if option then press(option) end
at(21.5)
check("the merchant's window", MerchantFrame:IsShown() and GetRepairAllCost() == 254)
-- The repair cursor taken; the boots mended (0.353 s).
at(23.112) press(MerchantRepairItemButton)
check("in repair mode", InRepairMode())
e.setLatency(0.353)
at(30.163) press(CharacterFeetSlot)
at(30.6)
check("the boots mended", select(1, GetInventoryItemDurability(8)) == 40 and GetMoney() == 19090)
at(31.902) e.pressKey("ESCAPE")
at(32.0)
check("the merchant's window closed", not MerchantFrame:IsShown() and not InRepairMode())
-- Jahan again: his gossip 0.478 s later, his goods 0.393 s.
e.setLatency(0.478)
at(35.630) player:interact(jahan)
at(36.3)
option = button(GossipFrame, "Let me browse your goods.")
check("the option shown again", option ~= nil)
e.setLatency(0.393)
at(37.052) if option then press(option) end
at(37.6)
check("the merchant's window again", MerchantFrame:IsShown() and GetRepairAllCost() == 228)
-- Everything else mended (0.697 s).
e.setLatency(0.697)
at(38.741) press(MerchantRepairAllButton)
at(39.5)
check("all mended", select(1, GetInventoryItemDurability(16)) == 70 and GetMoney() == 18862)
at(39.937) e.pressKey("ESCAPE")
at(40.1)
check("the merchant's window closed again", not MerchantFrame:IsShown())
at(44.9)
