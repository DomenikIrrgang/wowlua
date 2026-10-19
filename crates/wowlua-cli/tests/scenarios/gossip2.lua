-- /stdtest engine scenario gossip2, played with the engine's controls: the
-- level 21 paladin, in the Crossroads, talks to Jahan Hawkwing (a leather
-- merchant) for the first time, browses his goods and closes the window,
-- then talks to him again and closes the gossip with Escape
-- (scenarios/gossip2.lua). Each step at its recorded time; the gossip's
-- text with the interaction both times (recorded).
local e = __engine
local player = e.player
-- The option's answer 0.283 s after the click (recorded).
e.setLatency(0.283)
local jahan = e.spawnUnit({
    name = "Jahan Hawkwing", guid = "Creature-0-4621-1-21-3483-F2E0F46302", npcID = 3483, level = 21,
    reaction = "friendly", class = "WARRIOR", classID = 1, sex = 2, powerType = 1, pvp = true,
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde",
})
-- His goods as the client listed them: their prices and icons by place;
-- their IDs as its filter updates named them (which is where unrecorded),
-- their names not loaded (the client gave none).
local goods = {
    { 844, 1452, 132760 }, { 1844, 725, 132512 }, { 847, 1447, 134706 }, { 848, 1077, 132592 },
    { 1846, 728, 132607 }, { 1843, 720, 132939 }, { 850, 1749, 132624 }, { 846, 877, 132492 },
    { 1845, 1755, 134583 }, { 845, 1326, 132535 }, { 849, 880, 132606 }, { 843, 883, 132938 },
}
local items = {}
for i, g in ipairs(goods) do
    e.defineItem(g[1], { icon = g[3], stackCount = 1 })
    items[i] = { id = g[1], price = g[2] }
end
e.defineGossip(jahan, {
    text = "The Barrens, with its hot sun and hostile denizens, is the perfect place to test your strength and fortitude.",
    options = { {
        name = "Let me browse your goods.", icon = 132060, gossipOptionID = 95584, opens = "merchant",
        merchant = { name = "Jahan Hawkwing", canRepair = true, repairCost = 42, items = items },
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
jahan:setDistance(3)
at(2.430) player:target(jahan)
-- The interaction's answer: the gossip with its text (2.972).
at(2.689) player:interact(jahan)
at(3.1)
check("the gossip shown", GossipFrame:IsShown())
local option = button(GossipFrame, "Let me browse your goods.")
check("the option shown", option ~= nil)
at(6.234) if option then press(option) end
at(6.6)
check("the merchant's window", MerchantFrame:IsShown())
at(9.286) HideUIPanel(MerchantFrame)
at(9.4)
check("the merchant's window closed", not MerchantFrame:IsShown())
at(21.045) player:interact(jahan)
at(21.5)
check("the gossip shown again", GossipFrame:IsShown())
at(26.097) HideUIPanel(GossipFrame)
at(26.5)
check("the gossip closed", not GossipFrame:IsShown())
