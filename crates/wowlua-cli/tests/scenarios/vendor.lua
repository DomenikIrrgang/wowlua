-- /stdtest engine scenario vendor, played with the engine's controls: the
-- level 2 warrior at Godric Rothgar (Northshire's armorer) sells a Small
-- Shield, two Broken Fangs and a Chipped Claw, buys a Small Shield, buys
-- the claw back, cannot afford a second shield, sells three Stringy Wolf
-- Meat and repairs everything (scenarios/vendor.lua). Each step at its
-- recorded time (a click: a moment before the server's answer), the ping
-- as recorded (~0.3 s). The goods' prices other than the shield's (34)
-- are unrecorded: 0.
local e = __engine
local player = e.player
e.setLatency(0.3)
-- The warrior's names as the recording has them, scrubbed.
player:setName("Player", "Lastname")
local godric = e.spawnUnit({
    name = "Godric Rothgar", guid = "Creature-0-6782-0-7-1213-835BD76D8F", npcID = 1213, level = 5,
    reaction = "friendly", class = "WARRIOR", classID = 1, sex = 2, powerType = 1,
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Alliance",
})
-- The worn gear's durability (the recording's answers): pants, boots,
-- sword and shield damaged.
player:setDurability(7, 22, 25)
player:setDurability(8, 15, 16)
player:setDurability(16, 17, 20)
player:setDurability(17, 18, 20)
-- The goods, and what the warrior sells (their prices as the money told).
for id, item in pairs({
    [2379] = { name = "Tarnished Chain Vest", quality = 1 },
    [2381] = { name = "Tarnished Chain Leggings", quality = 1 },
    [2380] = { name = "Tarnished Chain Belt", quality = 1 },
    [2383] = { name = "Tarnished Chain Boots", quality = 1 },
    [2384] = { name = "Tarnished Chain Bracers", quality = 1 },
    [2385] = { name = "Tarnished Chain Gloves", quality = 1 },
    [17184] = { name = "Small Shield", quality = 1, icon = 134955, sellPrice = 6, stackCount = 1 },
    [2129] = { name = "Large Round Shield", quality = 1 },
    [7073] = { name = "Broken Fang", quality = 0, icon = 133725, sellPrice = 6, stackCount = 5 },
    [7074] = { name = "Chipped Claw", quality = 0, icon = 133723, sellPrice = 4, stackCount = 5 },
    [2672] = { name = "Stringy Wolf Meat", quality = 1, icon = 133970, sellPrice = 4, stackCount = 20 },
}) do
    e.defineItem(id, item)
end
-- A creature another player killed nearby: its death.
local function bystander(guid)
    e.spawnUnit({ name = "Young Wolf", guid = guid, npcID = 299, level = 1, reaction = "neutral" }):kill()
end
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
local function chatHas(frame, text)
    for i = 1, frame:GetNumMessages() do
        if (frame:GetMessageInfo(i)) == text then return true end
    end
    return false
end
at(1.491) player:target(godric)
at(1.800) e.openMerchant({
    name = "Godric Rothgar", canRepair = true, repairCost = 7,
    items = {
        { id = 2379 }, { id = 2381 }, { id = 2380 }, { id = 2383 },
        { id = 2384 }, { id = 2385 }, { id = 17184, price = 34 }, { id = 2129 },
    },
})
at(1.9)
check("the merchant's window", MerchantFrame:IsShown() and MerchantItem7Name:GetText() == "Small Shield")
-- Out of combat: rage fades, health comes back (values unrecorded).
at(1.968) player:setPower(4)
player:heal(1)
at(2.235) bystander("Creature-0-6782-0-7-299-02D204F60B")
at(3.901) player:setPower(3)
player:heal(1)
at(5.975) player:setPower(2)
-- Sold: a Small Shield, the fangs.
at(6.032) e.sell(0, 5)
at(6.904) player:wear()
at(7.965) player:setPower(1)
at(8.834) e.sell(0, 6)
at(10.051) player:setPower(0)
at(10.725) bystander("Creature-0-6782-0-7-299-B913903ABE")
-- Bought: a Small Shield.
at(11.691) press(MerchantItem7ItemButton, "RightButton")
at(12.748) bystander("Creature-0-6782-0-7-299-7852634FCE")
at(14.249) e.sell(0, 7)
-- The claw bought back.
at(15.262) press(MerchantBuyBackItemItemButton, "RightButton")
at(16.680) player:wear()
at(17.413) bystander("Creature-0-6782-0-7-299-44D9865248")
-- Another shield: not enough money.
at(17.980) player:wear()
at(18.520) player:wear()
at(23.102) press(MerchantItem7ItemButton, "RightButton")
at(26.334) e.sell(0, 9)
-- Everything repaired.
at(27.583) player:wear()
at(29.517) press(MerchantRepairAllButton)
at(30.534) e.closeMerchant()
at(31)
-- What Blizzard's UI shows at the end.
check("the merchant's window closed", not MerchantFrame:IsShown())
check("chat: the shield bought", chatHas(ChatFrame1, "You receive item: |cnIQ1:|Hitem:17184::::::::2:1491::14:::::::|h[Small Shield]|h|r"))
check("the money: 7 copper", GetMoney() == 7)
check("repaired: the sword's durability 20 of 20", select(1, GetInventoryItemDurability(16)) == 20)
