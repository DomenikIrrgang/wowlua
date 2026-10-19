-- /stdtest engine scenario equip, played with the engine's controls: the
-- paladin's character window open, the gloves (Grip of Fear) dragged into
-- the first bag's free slot, worn again by right-click, a bag item hovered
-- for its comparison tooltip (scenarios/equip.lua). Each step at its
-- recorded time (a click: when the client acted), the server's answers
-- as late as recorded.
local e = __engine
local now = 0
-- Time passes in frames, as in the client (60 a second), the rest of a
-- step as one jump.
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
local function script(frame, name, ...)
    local ok, err = pcall(e.script, frame, name, ...)
    if not ok then check(name .. ": " .. tostring(err), false) end
end
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
-- The paladin's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 10300, 19740, 1022, 1038, 19979, 25290, 5599, 19834, 10292, 19876, 25898, 19977, 10293, 7294, 19898, 19978, 19888, 20729, 19837, 10298, 465, 10278, 25782, 19850, 10290, 6940, 10301, 10299, 25916, 19896, 19899, 1044, 25890, 19838, 25895, 25291, 1032, 19854, 25918, 19836, 19742, 19900, 19746, 25894, 19853, 19895, 19897, 19852, 10291, 19835, 19891, 20217, 643 })
-- The gloves (the snapshot's data) and what wearing them gives: 30 more
-- health and 90 more mana (`UnitHealthMax`, `UnitPowerMax` recorded),
-- three stats (`UNIT_STATS` three times), strength (the attack power's
-- update).
e.defineItem(271208, {
    name = "Grip of Fear", quality = 3, icon = 132938, itemLevel = 22, minLevel = 17,
    classID = 4, subclassID = 3, type = "Armor", subType = "Mail", equipLoc = "INVTYPE_HAND",
    sellPrice = 686, stackCount = 1,
    -- It binds (recorded bound in the bag once taken off; which binding
    -- not recorded: on equip taken).
    bindType = 2,
    link = "|cnIQ3:|Hitem:271208::::::::21:1486:::::::::|h[Grip of Fear]|h|r",
    worn = { maxHealth = 30, maxMana = 90, stats = 3, attackPower = true },
})
-- The character window open before the recording began.
e.pressKey("C")
-- Full mana (the snapshot keeps no current power; the gloves' removal
-- cut it, recorded).
e.player:setPower(755)
-- A friendly NPC nearby soft-targeted, as friend and to interact with
-- (recorded: its maximum health told as the player's changed; who it is
-- not recorded).
local npc = e.spawnUnit({ name = "Nearby NPC", reaction = "friendly", level = 20 })
at(8.508) e.player:softTarget(npc, "friend") e.player:softTarget(npc, "interact")
-- The bags opened (their items' data loaded, recorded).
at(9.791) e.pressKey("SHIFT-B")
local slot = bagButton(1, 4)
check("bag 1 slot 4 shown", slot ~= nil)
-- The gloves dragged into bag 1's slot 4: the server's move 0.436 s after
-- the drop (recorded; two round trips, as the engine counts it).
at(12.043) script(CharacterHandsSlot, "OnDragStart", "LeftButton")
e.setLatency(0.436 / 2)
at(12.875) script(CharacterHandsSlot, "OnDragStop") script(slot, "OnReceiveDrag")
at(13.6)
check("the gloves in the bag", C_Container.GetContainerItemID(1, 4) == 271208 and GetInventoryItemID("player", 10) == nil)
-- Worn again by right-click: the server's answer 0.359 s later.
e.setLatency(0.359 / 2)
at(13.832) press(slot, "RightButton")
at(14.5)
check("the gloves worn", GetInventoryItemID("player", 10) == 271208)
-- The 90 mana the removal cut regenerated in the server's six updates
-- (recorded; their values secret, 15 each assumed: full after the sixth,
-- none after it).
local mana = 665
local function regen(t)
    mana = mana + 15
    at(t) e.player:setPower(mana)
end
regen(15.089)
-- An equippable bag item hovered: its tooltip (the comparison with the
-- worn weapon not modelled: C_TooltipComparison; which item and when not
-- recorded, the Mining Pick taken; no events recorded).
at(16) e.hover(bagButton(0, 4))
check("the pick's tooltip", GameTooltip:IsShown())
regen(17.123)
regen(19.124)
regen(21.124)
regen(22.211)
at(22.226) e.hover(nil)
regen(23.153)
-- An enchanting spell's end the server told, its cause unknown (recorded).
at(26.234) e.fireEvent("ENCHANT_SPELL_COMPLETED", false, nil)
at(27)
