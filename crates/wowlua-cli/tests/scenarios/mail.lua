-- /stdtest engine scenario mail, played with the engine's controls: the
-- level 21 paladin at the Crossroads' mailbox opens it, reads an expired
-- auction's letter and takes its Copper Ore, attaches the ore to a letter
-- and takes it back, attaches a Shiny Bauble instead and sends it, then
-- walks off (scenarios/mail.lua). Each step at its recorded time (a click:
-- a round trip before the server's answer, ~0.49 s here).
local e = __engine
local player = e.player
-- The paladin's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 10300, 19740, 1022, 1038, 19979, 25290, 5599, 19834, 10292, 19876, 25898, 19977, 10293, 7294, 19898, 19978, 19888, 20729, 19837, 10298, 465, 10278, 25782, 19850, 10290, 6940, 10301, 10299, 25916, 19896, 19899, 1044, 25890, 19838, 25895, 25291, 1032, 19854, 25918, 19836, 19742, 19900, 19746, 25894, 19853, 19895, 19897, 19852, 10291, 19835, 19891, 20217, 643 })
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
e.setLatency(0.488)
-- The letters' items (the recording's answers).
e.defineItem(8067, { name = "Crafted Light Shot", quality = 1, icon = 132384, stackCount = 200, classID = 6, subclassID = 3 })
e.defineItem(858, { name = "Lesser Healing Potion", quality = 1, icon = 134830, stackCount = 5, classID = 0, subclassID = 1 })
local mailbox = e.spawnUnit({ name = "Mailbox", guid = "GameObject-0-6782-1-4-143981-0000000001", reaction = "friendly", level = 1 })
e.defineMailbox(mailbox, {
    newMail = true,
    senders = { "Horde Auction House" },
    letters = {
        {
            stationeryIcon = 134939, sender = "Horde Auction House", subject = "Auction won: Crafted Light Shot",
            daysLeft = 29.220371246338, textCreated = true, items = { { id = 8067, count = 1 } },
            invoice = { "buyer", "Crafted Light Shot", "Otherplayer", 1, 1, 0, 0, 0, 0, 0, 1, false },
        },
        {
            stationeryIcon = 134939, sender = "Horde Auction House", subject = "Auction expired: Copper Ore",
            daysLeft = 28.830543518066, textCreated = true, items = { { id = 2770, count = 1 } }, auction = true,
        },
        {
            stationeryIcon = 134939, sender = "Horde Auction House", subject = "Auction expired: Lesser Healing Potion (3)",
            daysLeft = 2.8916897773743, wasRead = true, textCreated = true, items = { { id = 858, count = 3 } }, auction = true,
        },
    },
})
-- Another of the user's characters, targeted later, with Devrak (the
-- flight master) its target.
local devrak = e.spawnUnit({ name = "Devrak", guid = "Creature-0-6782-1-4-3615-0000000001", npcID = 3615, level = 55, reaction = "friendly" })
local other = e.spawnUnit({
    name = "Player", lastName = "Lastname", guid = "Player-0000-00000000", player = true, level = 14,
    reaction = "friendly", class = "WARRIOR", classID = 1, sex = 2, powerType = 1, race = "Tauren",
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde", pvp = false,
})
other:target(devrak)
-- Its buffs as the client answered them (GetAuraDataByIndex, in order).
for _, aura in ipairs({
    { auraInstanceID = 3, spellId = 2580, name = "Find Minerals", icon = 136025, canApplyAura = true, source = other },
    { auraInstanceID = 43, spellId = 1283391, name = "Campfire Nearby", icon = 7808147 },
    { auraInstanceID = 41, spellId = 1460, name = "Arcane Intellect", icon = 135932, duration = 3600, expirationTime = 68117.19, dispelName = "Magic" },
    { auraInstanceID = 39, spellId = 5232, name = "Mark of the Wild", icon = 136078, duration = 3600, expirationTime = 67340.319, dispelName = "Magic" },
    { auraInstanceID = 37, spellId = 1299038, name = "Plainsrunning", icon = 236717, applications = 25, points = { 25 }, source = other },
    { auraInstanceID = 33, spellId = 8091, name = "Armor", icon = 132341, duration = 1800, expirationTime = 64902.275, dispelName = "Magic", source = other },
}) do
    other:applyAura(aura)
end
-- The hunter targeted at the start (the snapshot's): its buffs as the
-- client answered them (GetAuraDataByIndex, in order), one more cast on
-- it at 5.099.
local hunter = e.unitOf("target")
for _, aura in ipairs({
    { auraInstanceID = 35, spellId = 20217, name = "Blessing of Kings", icon = 135995, duration = 3600, expirationTime = 66598.731, dispelName = "Magic", canApplyAura = true },
    { auraInstanceID = 31, spellId = 19834, name = "Blessing of Might", icon = 135906, duration = 3600, expirationTime = 67448.992, dispelName = "Magic", canApplyAura = true },
    { auraInstanceID = 41, spellId = 1244, name = "Power Word: Fortitude", icon = 135987, duration = 3600, expirationTime = 67546.456, dispelName = "Magic", points = { 10 } },
    { auraInstanceID = 37, spellId = 6756, name = "Mark of the Wild", icon = 136078, duration = 3600, expirationTime = 66391.672, dispelName = "Magic" },
    { auraInstanceID = 3, spellId = 1494, name = "Track Beasts", icon = 132328, source = hunter },
    { auraInstanceID = 1, spellId = 13165, name = "Aspect of the Hawk", icon = 136076, source = hunter },
    { auraInstanceID = 51, spellId = 1283391, name = "Campfire Nearby", icon = 7808147 },
}) do
    aura.quiet = true
    hunter:applyAura(aura)
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
-- A bag slot's button.
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
-- The world's own, not modelled: the server's events as recorded.
local function world(...) e.fireEvent(...) end
-- A spell (1229739) the client tries now and then, failing quietly.
local function tried(guid)
    world("UNIT_SPELLCAST_FAILED_QUIET", "player", "Cast-3-6783-1-5-1229739-" .. guid, 1229739, nil)
    world("CURRENT_SPELL_CAST_CHANGED", true)
    world("ACTIONBAR_UPDATE_STATE")
end
-- The targeted hunter's mana and pet.
local function mana(n)
    for _ = 1, n or 1 do world("UNIT_POWER_UPDATE", "target", "MANA") end
    world("UNIT_HAPPINESS", "target")
end
at(1.799) tried("CE1480F436")
at(5.099) hunter:applyAura({ auraInstanceID = 93, spellId = 1460, name = "Arcane Intellect", icon = 135932, duration = 3600, expirationTime = 68220.063, dispelName = "Magic" })
at(5.199) world("UNIT_MAXPOWER", "target", "MANA")
at(5.632) mana()
at(6.766) tried("7F7327E72E")
at(7.521) mana()
at(9.588) mana()
at(10.383) world("PLAYER_FLAGS_CHANGED", "softfriend")
at(11.675) mana()
at(13.648) mana()
at(15.376)
mailbox:setDistance(2)
player:interact(mailbox)
at(15.716) mana(2)
at(16.2)
check("the mailbox open", MailFrame:IsShown())
at(16.518) player:wear() player:wear() player:wear()
at(17.2)
check("three letters", InboxFrame:IsShown() and GetInboxNumItems() == 3)
-- The expired ore's letter opened, its ore taken.
at(17.938) press(_G["MailItem2Button"])
at(18.2)
check("the letter open", OpenMailFrame:IsShown())
at(18.289) press(OpenMailFrame.OpenMailAttachments[1])
at(19.5)
check("the ore taken, the letter gone", GetInboxNumItems() == 2)
at(19.961) player:target(other)
-- Soft targeted too (its auras' news for `softfriend` at 24.890), then
-- others (who, unrecorded: one stands in).
at(19.986) player:softTarget(other, "friend")
at(21.872) tried("2A53D79807")
at(24.890) other:updateAura(37)
at(25.805) world("TOOLTIP_DATA_UPDATE", 295)
-- Writing a letter: the ore attached and taken back, the bauble attached.
at(26.0) press(MailFrameTab2)
at(26.577) press(bagButton(5, 2), "RightButton")
at(26.796) tried("93907496AC")
at(27)
check("the ore attached", HasSendMailItem(1))
at(27.864) player:softTarget(e.spawnUnit({ name = "Crossroads Grunt", reaction = "friendly" }), "friend")
at(29.370) press(SendMailAttachment1, "RightButton")
at(29.864) other:updateAura(37)
at(30.280) world("TOOLTIP_DATA_UPDATE", 323)
at(31.568) world("TOOLTIP_DATA_UPDATE", 331)
at(31.838) tried("51DB0F8E36")
at(32.258) world("TOOLTIP_DATA_UPDATE", 338)
at(33.491) world("TOOLTIP_DATA_UPDATE", 345)
at(33.935) other:target(nil)
at(34.363) other:removeAura(43)
at(34.654) press(bagButton(0, 4), "RightButton")
at(34.852) other:updateAura(37)
at(36.856) tried("F9BFA15C06")
at(37.0) SendMailNameEditBox:SetText("Otherplayer")
-- Sent: the server's answer 0.545 s later.
e.setLatency(0.545)
at(37.577) press(SendMailMailButton)
at(38.6)
check("sent: the bauble gone", not HasSendMailItem(1) and C_Container.GetContainerItemID(0, 4) == nil)
check("sent: the postage paid", GetMoney() == 31682)
e.setLatency(0.491)
at(38.925) HideUIPanel(MailFrame)
at(39.2)
check("the mailbox closed", not MailFrame:IsShown())
at(39.817) other:updateAura(37)
at(41.550) world("PLAYER_FLAGS_CHANGED", "softfriend")
at(41.883) tried("C3E6ACC4D0")
at(43.3)
