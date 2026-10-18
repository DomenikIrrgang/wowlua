-- /stdtest engine scenario bank, played with the engine's controls: the
-- level 21 paladin talks to Atepa (Thunder Bluff's banker), takes Copper
-- Ore from the bank's first tab into the backpack, right-clicks it back
-- into the bank, buys a third bank tab and walks off (scenarios/bank.lua).
-- Each step at its recorded time (a click: a round trip before the
-- server's answer, ~0.26 s, 0.5 s for the moves).
local e = __engine
local player = e.player
-- The paladin's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 10300, 19740, 1022, 1038, 19979, 25290, 5599, 19834, 10292, 19876, 25898, 19977, 10293, 7294, 19898, 19978, 19888, 20729, 19837, 10298, 465, 10278, 25782, 19850, 10290, 6940, 10301, 10299, 25916, 19896, 19899, 1044, 25890, 19838, 25895, 25291, 1032, 19854, 25918, 19836, 19742, 19900, 19746, 25894, 19853, 19895, 19897, 19852, 10291, 19835, 19891, 20217, 643 })
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 } })
-- The bank's items (the recording's answers).
e.defineItem(1210, { name = "Shadowgem", quality = 2, icon = 134074, stackCount = 20, classID = 7, subclassID = 4, link = "|cnIQ2:|Hitem:1210::::::::21:1486:::::::::|h[Shadowgem]|h|r" })
e.defineItem(249391, { name = "Pyrite", quality = 2, icon = 961627, stackCount = 20, classID = 7, subclassID = 4, link = "|cnIQ2:|Hitem:249391::::::::21:1486:::::::::|h[Pyrite]|h|r" })
e.defineItem(3470, { name = "Rough Grinding Stone", quality = 1, icon = 135243, stackCount = 20, classID = 7, subclassID = 7, link = "|cnIQ1:|Hitem:3470::::::::21:1486:::::::::|h[Rough Grinding Stone]|h|r" })
e.defineItem(2840, { name = "Copper Bar", quality = 1, icon = 133216, stackCount = 20, classID = 7, subclassID = 7, link = "|cnIQ1:|Hitem:2840::::::::21:1486:::::::::|h[Copper Bar]|h|r" })
local atepa = e.spawnUnit({
    name = "Atepa", guid = "Creature-0-4615-1-6-8357-2B5E93E430", npcID = 8357, level = 45,
    reaction = "neutral", class = "WARRIOR", classID = 1, sex = 2, powerType = 1,
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde", pvp = true,
})
-- The character's two tabs: the first's items as the client answered them
-- (the second's not seen: empty, unverified); the next tab's price.
e.defineBank(atepa, {
    nextTabCost = 10000,
    tabs = {
        { name = "Tab 1", icon = 134400, slots = 48, items = {
            [1] = { id = 1210, count = 1 }, [2] = { id = 249391, count = 9 }, [3] = { id = 2771, count = 5 },
            [4] = { id = 3470, count = 20 }, [5] = { id = 2840, count = 20 }, [6] = { id = 2840, count = 20 },
            [8] = { id = 2770, count = 12 }, [9] = { id = 2835, count = 20 }, [10] = { id = 2835, count = 2 },
        } },
        { name = "Tab 2", icon = 134400, slots = 48 },
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
-- A bag slot's button (the bags' or the bank's).
local function bagButton(bag, slot)
    if bag >= 6 and BankPanel and BankPanel.itemButtonPool then
        for b in BankPanel.itemButtonPool:EnumerateActive() do
            if b:GetBankTabID() == bag and b:GetContainerSlotID() == slot then return b end
        end
    end
    for _, name in ipairs({ "ContainerFrameCombinedBags", "ContainerFrame1", "ContainerFrame2", "ContainerFrame3", "ContainerFrame4", "ContainerFrame5", "ContainerFrame6" }) do
        local f = _G[name]
        if f and f:IsShown() and f.EnumerateValidItems then
            for _, b in f:EnumerateValidItems() do
                if b:GetBagID() == bag and b:GetID() == slot then return b end
            end
        end
    end
end
-- The world's own, not modelled: the server's events as recorded.
local function world(...) e.fireEvent(...) end
-- The worn items' updates from the server (no change), the rested
-- experience, a weapon enchant's end now and then.
local function wear(n) for _ = 1, n or 1 do player:wear() end end
at(9.047) wear()
at(10.325) wear() world("UPDATE_EXHAUSTION")
at(18.072) wear()
at(18.913) wear()
at(19.216) wear()
at(20.486) wear()
at(22.057) wear()
at(22.890) wear()
at(23.721) wear()
at(25.723) wear()
at(36.607) world("ENCHANT_SPELL_COMPLETED", false, nil)
at(41.025) world("ENCHANT_SPELL_COMPLETED", false, nil)
at(44.503) world("ENCHANT_SPELL_COMPLETED", false, nil)
at(47.319) world("UPDATE_EXHAUSTION")
at(50.034) wear()
-- At the banker: the round trip 0.257 s.
e.setLatency(0.257)
at(53.082)
player:target(atepa)
atepa:setDistance(3)
player:interact(atepa)
at(53.8)
check("the bank open", BankFrame:IsShown())
at(55.184) world("ENCHANT_SPELL_COMPLETED", false, nil)
-- The ore taken from the first tab, put in the backpack's sixth slot.
at(56.205) press(bagButton(6, 8))
e.setLatency(0.516)
at(57.289) press(bagButton(0, 6))
at(58.2)
check("the ore in the backpack", C_Container.GetContainerItemID(0, 6) == 2770)
-- Right-clicked back into the bank.
e.setLatency(0.485)
at(58.518) press(bagButton(0, 6), "RightButton")
at(59.3)
check("the ore back in the bank", C_Container.GetContainerItemID(6, 7) == 2770 and C_Container.GetContainerItemID(0, 6) == nil)
-- A third tab bought.
e.setLatency(0.257)
at(60.6) press(BankFrame.BankPanel.PurchaseButton)
at(61.24) press(StaticPopup1Button1)
at(61.8)
check("the tab bought", C_Bank.FetchNumPurchasedBankTabs(0) == 3 and GetMoney() == 21682)
at(63.064) world("ENCHANT_SPELL_COMPLETED", false, nil)
at(68.670) HideUIPanel(BankFrame)
at(68.934) player:target(nil)
at(69.0)
check("the bank closed", not BankFrame:IsShown())
at(70.269) world("ENCHANT_SPELL_COMPLETED", false, nil)
at(70.5)
