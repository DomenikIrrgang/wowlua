-- /stdtest engine scenario quest, played with the engine's controls: the
-- recording's level 1 warrior takes "Wolves Across the Border" from Eagan
-- Peltskinner, hunts wolves for eight Tough Wolf Meat (levelling to 2 on
-- the way), and turns it in for the Soft Fur-lined Shoes
-- (scenarios/quest.lua). Each step at its recorded time, the ping as
-- recorded (~0.3 s). The fights' steps were read off the recording (its
-- swings, spells, threat, kills, loot); the rage values are guesses
-- consistent with the usability the client told (it keeps rage secret in
-- combat). Other players' kills nearby: their deaths only.
local e = __engine
local player = e.player
e.setLatency(0.3)
-- The warrior's names as the recording has them, scrubbed.
player:setName("Player", "Lastname")
-- The warrior's Defensive State: its cooldown told when the warrior dodges,
-- parries or blocks (recorded).
e.defineSpell(5302, { name = "Defensive State", procOnAvoid = true })
-- The worn gear's durability (the recording's answers before any wear).
player:setDurability(7, 25, 25)
player:setDurability(8, 16, 16)
player:setDurability(16, 20, 20)
player:setDurability(17, 20, 20)
local eagan = e.spawnUnit({
    name = "Eagan Peltskinner", guid = "Creature-0-6782-0-7-196-835BD76D8F", npcID = 196, level = 3,
    reaction = "friendly", class = "WARRIOR", classID = 1, sex = 2, powerType = 1,
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Alliance",
})
eagan:setDistance(9.9)
-- The items: the quest's (bound, no value: recorded), the rewards (the
-- shoes bound), the wolves' loot.
for id, item in pairs({
    [750] = { name = "Tough Wolf Meat", quality = 1, icon = 133972, stackCount = 20, classID = 12, subclassID = 0, bindType = 4, sellPrice = 0 },
    [80] = { name = "Soft Fur-lined Shoes", quality = 1, icon = 132543, stackCount = 1, classID = 4, subclassID = 1, bindType = 1 },
    [6070] = { name = "Wolfskin Bracers", quality = 1, icon = 132604, stackCount = 1, classID = 4, subclassID = 2 },
    [7074] = { name = "Chipped Claw", quality = 0, icon = 133723, stackCount = 5, classID = 15, subclassID = 0 },
    [7073] = { name = "Broken Fang", quality = 0, icon = 133725, stackCount = 5, classID = 15, subclassID = 0 },
    [4865] = { name = "Ruined Pelt", quality = 0, icon = 134371, stackCount = 5, classID = 15, subclassID = 0 },
    [2672] = { name = "Stringy Wolf Meat", quality = 1, icon = 133970, stackCount = 20, classID = 7, subclassID = 8 },
    [2652] = { name = "Flimsy Chain Cloak", quality = 0, icon = 133754, stackCount = 1, classID = 4, subclassID = 1 },
    [1368] = { name = "Ragged Leather Gloves", quality = 0, icon = 132952, stackCount = 1, classID = 4, subclassID = 2 },
    [1372] = { name = "Ragged Cloak", quality = 0, icon = 133762, stackCount = 1, classID = 4, subclassID = 1 },
    [2654] = { name = "Flimsy Chain Pants", quality = 0, icon = 134583, stackCount = 1, classID = 4, subclassID = 3 },
}) do
    e.defineItem(id, item)
end
e.defineQuest(33, {
    title = "Wolves Across the Border",
    text = "I hate those nasty timber wolves!  But I sure like eating wolf steaks...  Bring me tough wolf meat and I will exchange it for something you'll find useful.\n\nTough wolf meat is gathered from hunting the timber wolves and young wolves wandering the Northshire countryside.",
    objective = "Bring 8 pieces of Tough Wolf Meat to Eagan Peltskinner outside Northshire Abbey.",
    progress = "Hey $N.  I'm getting hungry...did you get that tough wolf meat?",
    completion = "You've been busy!  I can't wait to cook up that wolf meat...\n\nI have some things here you might want - take your pick!",
    giver = eagan,
    xp = 170, money = 0,
    choices = { 80, 6070 },
    objectives = { { item = 750, required = 8, text = "Tough Wolf Meat", type = "item", objectiveType = 1 } },
    header = "Northshire Valley", headerSortKey = 1073741833,
    level = 2, difficultyLevel = 2, questClassification = 7,
    reputation = { faction = 72, name = "Stormwind", amount = 100, standing = 4100 },
})
-- Level 2: as the client told it (PLAYER_LEVEL_UP's deltas, the new end).
e.defineLevel(2, {
    xpMax = 900, health = 19, power = 0, talents = 0, pvpTalentSlots = 0,
    strength = 1, agility = 1, stamina = 1, intellect = 0, spirit = 0,
})
-- Heroic Strike: on the next swing, its rage cost unverified (15).
e.defineSpell(78, { name = "Heroic Strike", powerCost = 15, nextSwing = true })
-- The rage (secret in the recording) on the side of Heroic Strike's cost
-- the client's usability told after each moment: 20 above, 5 below; its
-- updates where the client told them (decay out of combat: frequent).
-- The wolves the warrior fought (each one's health: the damage it took).
local u_53fa49cc84 = e.spawnUnit({ name = "Young Wolf", guid = "Creature-0-6782-0-7-299-53FA49CC84", npcID = 299, level = 1, reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 42, maxHealth = 42 })
local u_57ac1525ca = e.spawnUnit({ name = "Young Wolf", guid = "Creature-0-6782-0-7-299-57AC1525CA", npcID = 299, level = 1, reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 49, maxHealth = 49 })
local u_57461dbc45 = e.spawnUnit({ name = "Young Wolf", guid = "Creature-0-6782-0-7-299-57461DBC45", npcID = 299, level = 1, reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 57, maxHealth = 57 })
local u_848ad88022 = e.spawnUnit({ name = "Timber Wolf", guid = "Creature-0-6782-0-7-69-848AD88022", npcID = 69, level = 2, reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 55, maxHealth = 55 })
local u_ba7c88cbe5 = e.spawnUnit({ name = "Young Wolf", guid = "Creature-0-6782-0-7-299-BA7C88CBE5", npcID = 299, level = 1, reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 41, maxHealth = 41 })
local u_fed254bade = e.spawnUnit({ name = "Young Wolf", guid = "Creature-0-6782-0-7-299-FED254BADE", npcID = 299, level = 1, reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 44, maxHealth = 44 })
local u_989f54f5af = e.spawnUnit({ name = "Young Wolf", guid = "Creature-0-6782-0-7-299-989F54F5AF", npcID = 299, level = 1, reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 1, maxHealth = 1 })
local u_3139e0d9e3 = e.spawnUnit({ name = "Young Wolf", guid = "Creature-0-6782-0-7-299-3139E0D9E3", npcID = 299, level = 1, reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 56, maxHealth = 56 })
local u_fba621fce1 = e.spawnUnit({ name = "Young Wolf", guid = "Creature-0-6782-0-7-299-FBA621FCE1", npcID = 299, level = 1, reaction = "neutral", class = "WARRIOR", classID = 1, sex = 1, powerType = 1, creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Wolf", creatureFamilyID = 1, health = 45, maxHealth = 45 })
-- A creature another player killed nearby: its death.
local function bystander(guid, npcID, name)
    e.spawnUnit({ name = name, guid = guid, npcID = npcID, level = 1, reaction = "hostile" }):kill()
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
local function press(frame)
    local ok, err = pcall(e.click, frame)
    if not ok then check("clicked: " .. tostring(err), false) end
end
local function chatHas(frame, text)
    for i = 1, frame:GetNumMessages() do
        if (frame:GetMessageInfo(i)) == text then return true end
    end
    return false
end
-- Eagan, too far at first; then near: the quest offered.
at(2.829) player:target(eagan)
player:interact(eagan)
at(3.583) eagan:setDistance(3)
player:interact(eagan)
at(4.2)
check("the quest's details shown", QuestFrame:IsShown() and QuestFrameDetailPanel:IsShown())
check("the quest's title", QuestInfoTitleHeader:GetText() == "Wolves Across the Border")
check("the quest's objective", QuestInfoObjectivesText:GetText() == "Bring 8 pieces of Tough Wolf Meat to Eagan Peltskinner outside Northshire Abbey.")
at(4.824) press(QuestFrameAcceptButton)
check("accepted: the quest frame closed", not QuestFrame:IsShown())
at(5.6)
check("chat: the quest accepted", chatHas(ChatFrame1, "Quest accepted: Wolves Across the Border"))
check("the log: the quest", C_QuestLog.GetLogIndexForQuestID(33) == 2)
-- The hunt.
at(7.892) player:target(u_53fa49cc84)
at(8.147) player:startAttack()
at(10.695) player:swing(u_53fa49cc84, 5, {  })
at(10.695) u_53fa49cc84:setThreat(4)
at(10.781) u_53fa49cc84:swing(player, 0, { kind = "DODGE" })
at(10.781) u_53fa49cc84:target(player)
at(10.781) player:setPower(5)
at(12.094) u_53fa49cc84:setThreat(5)
at(12.694) player:swing(u_53fa49cc84, 5, { power = 5 })
at(12.794) u_53fa49cc84:swing(player, 2, { victimPower = 5 })
at(13.694) u_53fa49cc84:setThreat(9)
at(14.560) player:swing(u_53fa49cc84, 5, { flags = "GLANCING", power = 20 })
at(14.794) u_53fa49cc84:swing(player, 1, { victimPower = 20 })
at(14.894) bystander("Creature-0-6782-0-7-257-2ACAED63D2", 257, "Kobold Worker")
at(14.894) u_53fa49cc84:setThreat(13)
at(16.488) player:swing(u_53fa49cc84, 5, { power = 20 })
at(16.695) u_53fa49cc84:swing(player, 0, { kind = "DODGE" })
at(16.988) player:useSpell(78)
at(17.312) u_53fa49cc84:setThreat(17)
at(18.394) player:swing(u_53fa49cc84, 17, { power = 5 })
at(18.556) u_53fa49cc84:setThreat(44)
at(18.732) u_53fa49cc84:swing(player, 1, { victimPower = 5 })
at(19.590) player:useSpell(78) -- fails: not enough rage
at(20.239) player:swing(u_53fa49cc84, 5, { power = 20, kill = { xp = 50, loot = { { id = 750 }, { id = 7074 }, { id = 2652 } } } })
at(20.655) player:stopAttack()
at(21.050) player:leaveCombat()
at(21.177) player:target(u_53fa49cc84)
at(21.227) player:loot(u_53fa49cc84, { auto = true })
at(22.562) player:heal(1)
at(22.562) player:setPower(20)
at(22.593) CloseLoot() -- walked off
at(24.259) player:target(u_57ac1525ca)
at(24.429) player:setPower(5, { frequent = true })
at(24.671) player:startAttack()
at(24.671) player:swing(u_57ac1525ca, 11, { flags = "CRITICAL", power = 20 })
at(24.671) u_57ac1525ca:target(player)
at(24.671) u_57ac1525ca:swing(player, 1, { victimPower = 20 })
at(24.671) u_57ac1525ca:setThreat(9)
at(25.477) player:useSpell(78)
at(25.898) u_57ac1525ca:setThreat(10)
at(26.422) bystander("Creature-0-6782-0-7-6-529B52607A", 6, "Kobold Vermin")
at(26.487) player:swing(u_57ac1525ca, 17, { power = 20 })
at(26.612) player:wear(16) -- the sword: 19 of 20
at(26.612) u_57ac1525ca:swing(player, 1, { victimPower = 20 })
at(27.107) u_57ac1525ca:setThreat(37)
at(28.518) player:swing(u_57ac1525ca, 6, { power = 20 })
at(28.588) u_57ac1525ca:swing(player, 2, { victimPower = 20 })
at(28.693) player:setPower(20)
at(29.474) u_57ac1525ca:setThreat(42)
at(29.693) player:useSpell(78)
at(30.386) player:swing(u_57ac1525ca, 15, { power = 5, kill = { xp = 50, loot = { { id = 750 }, { id = 7074 } } } })
at(30.716) player:stopAttack()
at(30.716) player:leaveCombat()
at(30.968) player:loot(u_57ac1525ca, { auto = true })
at(31.015) player:target(u_57ac1525ca)
at(32.053) CloseLoot() -- walked off
at(32.714) bystander("Creature-0-6782-0-7-6-F55ABBB5D6", 6, "Kobold Vermin")
at(32.714) player:heal(1)
at(32.714) player:setPower(5)
at(33.919) player:target(u_57461dbc45)
at(34.291) player:startAttack()
at(34.809) player:setPower(5)
at(35.766) player:swing(u_57461dbc45, 6, { power = 5 })
at(35.766) u_57461dbc45:target(player)
at(35.766) u_57461dbc45:swing(player, 1, { victimPower = 5 })
at(35.766) u_57461dbc45:setThreat(5)
at(35.972) player:wear()
at(36.728) player:wear()
at(36.909) u_57461dbc45:setThreat(6)
at(37.657) player:swing(u_57461dbc45, 5, { power = 20 })
at(37.772) u_57461dbc45:swing(player, 1, { victimPower = 20 })
at(38.319) player:useSpell(78)
at(38.477) u_57461dbc45:setThreat(10)
at(39.586) player:swing(u_57461dbc45, 16, { power = 5 })
at(39.748) u_57461dbc45:swing(player, 1, { victimPower = 5 })
at(40.782) player:wear(7) -- the pants: 24 of 25
at(40.887) u_57461dbc45:setThreat(36)
at(41.480) player:swing(u_57461dbc45, 6, { power = 5 })
at(41.791) u_57461dbc45:swing(player, 0, { kind = "DODGE" })
at(42.061) u_57461dbc45:setThreat(41)
at(42.498) player:useSpell(78) -- fails: not enough rage
at(43.286) player:swing(u_57461dbc45, 7, {  })
at(43.286) player:setPower(20, { frequent = true })
at(43.396) player:setPower(20)
at(43.752) bystander("Creature-0-6782-0-7-6-8D68555A0C", 6, "Kobold Vermin")
at(43.752) u_57461dbc45:swing(player, 2, { victimPower = 20 })
at(44.009) player:useSpell(78)
at(44.110) u_57461dbc45:setThreat(47)
at(45.297) player:swing(u_57461dbc45, 17, { power = 5, kill = { xp = 50, loot = { { id = 750 }, { id = 4865 }, { id = 7074 }, { id = 1368 } } } })
at(45.365) player:leaveCombat()
at(45.587) player:stopAttack()
at(45.681) player:target(u_57461dbc45)
at(45.731) player:loot(u_57461dbc45, { auto = true })
at(46.868) player:heal(1)
at(46.868) player:setPower(5)
at(47.264) CloseLoot() -- walked off
at(47.780) player:target(u_848ad88022)
at(48.121) player:startAttack()
at(48.121) player:swing(u_848ad88022, 5, { flags = "GLANCING", power = 5 })
at(48.121) u_848ad88022:target(player)
at(48.121) u_848ad88022:swing(player, 2, { victimPower = 5 })
at(48.121) u_848ad88022:setThreat(4)
at(48.966) u_848ad88022:setThreat(5)
at(50.017) player:swing(u_848ad88022, 5, { power = 5 })
at(50.107) u_848ad88022:swing(player, 2, { victimPower = 20 })
at(50.539) u_848ad88022:setThreat(9)
at(51.937) player:swing(u_848ad88022, 5, { flags = "GLANCING", power = 20 })
at(52.115) u_848ad88022:swing(player, 2, { victimPower = 20 })
at(52.639) u_848ad88022:setThreat(13)
at(53.706) player:useSpell(78)
at(53.812) player:wear()
at(53.812) player:swing(u_848ad88022, 17, { power = 5 })
at(54.205) u_848ad88022:swing(player, 2, { victimPower = 5 })
at(55.431) u_848ad88022:setThreat(40)
at(55.782) player:swing(u_848ad88022, 6, { power = 5 })
at(55.872) player:useSpell(78) -- fails: not enough rage
at(56.079) u_848ad88022:swing(player, 3, { victimPower = 20 })
at(56.206) u_848ad88022:setThreat(45)
at(56.434) player:useSpell(78)
at(57.376) player:wear()
at(57.638) player:swing(u_848ad88022, 17, { power = 5, kill = { xp = 52, loot = { { id = 750 }, { id = 2672 }, { id = 4865 } } } })
at(57.985) player:stopAttack()
at(58.293) player:target(u_848ad88022)
at(58.352) player:loot(u_848ad88022, { auto = true })
at(58.652) player:leaveCombat()
at(58.768) bystander("Creature-0-6782-0-7-69-A50EDA3B96", 69, "Timber Wolf")
at(58.910) CloseLoot() -- walked off
at(59.086) player:heal(1)
at(59.086) player:setPower(5)
at(60.687) player:target(u_ba7c88cbe5)
at(61.025) player:startAttack()
at(61.071) player:heal(1)
at(61.981) player:swing(u_ba7c88cbe5, 6, { power = 5 })
at(61.981) u_ba7c88cbe5:setThreat(5)
at(61.982) u_ba7c88cbe5:swing(player, 0, { kind = "BLOCK" })
at(62.084) u_ba7c88cbe5:target(player)
at(63.125) u_ba7c88cbe5:setThreat(6)
at(63.879) player:swing(u_ba7c88cbe5, 7, { power = 5 })
at(64.012) u_ba7c88cbe5:swing(player, 0, { kind = "DODGE" })
at(64.282) u_ba7c88cbe5:setThreat(12)
at(64.995) player:useSpell(78) -- fails: not enough rage
at(65.181) player:useSpell(78) -- fails: not enough rage
at(65.378) player:useSpell(78) -- fails: not enough rage
at(65.548) player:useSpell(78) -- fails: not enough rage
at(65.720) player:useSpell(78) -- fails: not enough rage
at(65.826) player:swing(u_ba7c88cbe5, 6, { flags = "GLANCING", power = 20 })
at(66.052) u_ba7c88cbe5:swing(player, 2, { victimPower = 20 })
at(66.305) player:useSpell(78)
at(66.789) u_ba7c88cbe5:setThreat(17)
at(66.874) bystander("Creature-0-6782-0-7-6-2F53F83518", 6, "Kobold Vermin")
at(67.726) player:swing(u_ba7c88cbe5, 17, { power = 5 })
at(67.966) u_ba7c88cbe5:swing(player, 1, { victimPower = 5 })
at(67.966) u_ba7c88cbe5:setThreat(44)
at(68.064) player:setPower(5)
at(68.548) player:useSpell(78) -- fails: not enough rage
at(68.695) player:useSpell(78) -- fails: not enough rage
at(68.870) player:useSpell(78) -- fails: not enough rage
at(69.065) player:useSpell(78) -- fails: not enough rage
at(69.576) player:swing(u_ba7c88cbe5, 5, { power = 5, kill = { xp = 50, loot = { { id = 750 }, { id = 2672 }, { id = 7074 }, { id = 1372 } } } })
at(69.942) player:stopAttack()
at(69.942) bystander("Creature-0-6782-0-7-257-8A19E1C4FE", 257, "Kobold Worker")
at(70.006) player:heal(1)
at(70.043) player:target(u_ba7c88cbe5)
at(70.072) player:loot(u_ba7c88cbe5, { auto = true })
at(70.366) player:leaveCombat()
at(71.420) CloseLoot() -- walked off
at(71.558) player:setPower(5)
at(72.960) player:target(u_fed254bade)
at(73.218) player:startAttack()
at(73.657) player:setPower(5)
at(74.301) player:swing(u_fed254bade, 8, { power = 20 })
at(74.301) u_fed254bade:target(player)
at(74.301) u_fed254bade:swing(player, 1, { victimPower = 20 })
at(74.301) u_fed254bade:setThreat(6)
at(75.706) u_fed254bade:setThreat(7)
at(76.228) player:swing(u_fed254bade, 7, { power = 20 })
at(76.310) u_fed254bade:swing(player, 1, { victimPower = 20 })
at(76.885) u_fed254bade:setThreat(13)
at(77.569) player:useSpell(78)
at(77.927) player:wear()
at(78.042) bystander("Creature-0-6782-0-7-6-58ABEEECC7", 6, "Kobold Vermin")
at(78.042) player:swing(u_fed254bade, 17, { power = 5 })
at(78.328) u_fed254bade:swing(player, 1, { victimPower = 5 })
at(78.957) player:useSpell(78) -- fails: not enough rage
at(79.120) player:useSpell(78) -- fails: not enough rage
at(79.258) u_fed254bade:setThreat(40)
at(79.284) player:useSpell(78) -- fails: not enough rage
at(79.467) player:useSpell(78) -- fails: not enough rage
at(79.995) player:wear()
at(79.995) player:swing(u_fed254bade, 5, { flags = "GLANCING", power = 5 })
at(80.251) u_fed254bade:swing(player, 2, { victimPower = 20 })
at(80.511) u_fed254bade:setThreat(44)
at(81.998) player:useSpell(78) -- fails: not enough rage
at(81.998) player:swing(u_fed254bade, 7, { power = 20, kill = { xp = 40, loot = { { id = 750 }, { id = 7073 }, { id = 2654 } } } })
at(82.074) player:target(u_989f54f5af)
at(82.292) player:stopAttack()
at(82.416) player:startAttack()
at(82.416) player:useSpell(78)
at(82.515) player:loot(u_fed254bade, { auto = true })
at(82.530) player:target(u_fed254bade)
at(82.809) player:stopAttack()
at(82.924) player:leaveCombat()
at(83.708) player:heal(1)
at(83.708) player:setPower(20)
at(84.394) bystander("Creature-0-6782-0-7-257-212FF47877", 257, "Kobold Worker")
at(85.764) player:target(u_3139e0d9e3)
at(85.820) player:setPower(20)
at(86.072) player:startAttack()
at(86.599) player:swing(u_3139e0d9e3, 14, { flags = "CRITICAL", power = 20 })
at(86.599) u_3139e0d9e3:target(player)
at(86.599) u_3139e0d9e3:swing(player, 1, { victimPower = 20 })
at(86.599) u_3139e0d9e3:setThreat(11)
at(87.866) u_3139e0d9e3:setThreat(12)
at(88.499) player:swing(u_3139e0d9e3, 6, { power = 20 })
at(88.632) u_3139e0d9e3:swing(player, 1, { victimPower = 20 })
at(89.059) u_3139e0d9e3:setThreat(17)
at(90.208) player:useSpell(78)
at(90.425) player:swing(u_3139e0d9e3, 36, { flags = "CRITICAL", power = 20, kill = { xp = 40, loot = { { id = 750 }, { id = 2672 }, { id = 4865 }, { id = 7074 } } } })
at(90.682) player:stopAttack()
at(91.040) player:target(u_3139e0d9e3)
at(91.132) player:loot(u_3139e0d9e3, { auto = true })
at(91.496) player:leaveCombat()
at(91.844) player:heal(1)
at(91.844) player:setPower(20)
at(92.735) CloseLoot() -- walked off
at(93.655) player:target(u_fba621fce1)
at(93.932) player:startAttack()
at(93.932) player:setPower(20)
at(95.447) player:swing(u_fba621fce1, 6, { power = 20 })
at(95.447) u_fba621fce1:target(player)
at(95.447) u_fba621fce1:swing(player, 1, { victimPower = 20 })
at(95.447) u_fba621fce1:setThreat(5)
at(95.629) player:setPower(20)
at(95.629) player:setPower(20)
at(96.733) u_fba621fce1:setThreat(6)
at(97.011) player:useSpell(78)
at(97.354) player:swing(u_fba621fce1, 16, { power = 5 })
at(97.483) u_fba621fce1:swing(player, 2, { victimPower = 5 })
at(97.583) u_fba621fce1:setThreat(32)
at(99.305) player:swing(u_fba621fce1, 7, { power = 20 })
at(99.523) u_fba621fce1:swing(player, 1, { victimPower = 20 })
at(100.377) u_fba621fce1:setThreat(38)
at(100.874) player:useSpell(78)
at(101.134) player:swing(u_fba621fce1, 16, { kill = { xp = 40, loot = { { id = 750 }, { id = 4865 } } } })
at(101.134) player:setPower(5)
at(101.238) player:setPower(5)
at(101.558) player:stopAttack()
at(101.558) bystander("Creature-0-6782-0-7-257-BA7C88CBE5", 257, "Kobold Worker")
at(101.558) player:leaveCombat()
at(101.791) player:loot(u_fba621fce1, { auto = true })
at(101.821) player:target(u_fba621fce1)
at(101.967) player:heal(1)
at(101.967) player:setPower(5)
at(102.729) CloseLoot() -- walked off
at(103.161) player:wear()
at(104.047) player:setPower(5)
at(106.018) player:setPower(5)
at(106.5)
check("the objective done: 8/8", C_QuestLog.IsComplete(33))
check("level 2: the player frame", PlayerLevelText:GetText() == "2")
-- Back to Eagan: too far twice, then the quest's progress.
at(111.872) player:target(eagan)
eagan:setDistance(9.9)
player:interact(eagan)
at(115.953) player:interact(eagan)
at(118.398) eagan:setDistance(3)
player:interact(eagan)
at(119.0)
check("the quest's progress shown", QuestFrame:IsShown() and QuestFrameProgressPanel:IsShown())
at(119.609) press(QuestFrameCompleteButton)
at(120.5)
check("the rewards shown", QuestFrame:IsShown() and QuestFrameRewardPanel:IsShown())
-- The shoes chosen, the quest completed.
at(121.020) press(QuestInfoRewardsFrame.RewardButtons[1])
at(121.882) press(QuestFrameCompleteQuestButton)
at(123.939) bystander("Creature-0-6782-0-7-257-C602D3661B", 257, "Kobold Worker")
at(124.5)
-- What Blizzard's UI shows at the end.
check("turned in: the quest frame closed", not QuestFrame:IsShown())
check("chat: the quest completed", chatHas(ChatFrame1, "Wolves Across the Border completed."))
check("chat: the reward", chatHas(ChatFrame1, "You receive item: |cnIQ1:|Hitem:80::::::::2:1491::11:::::::|h[Soft Fur-lined Shoes]|h|r"))
check("chat: the reputation", chatHas(ChatFrame1, "Reputation with Stormwind increased by 100."))
check("the log: empty", C_QuestLog.GetNumQuestLogEntries() == 0)
local xpBar
local function find(f, depth)
    if depth > 5 or not f or f:IsForbidden() or xpBar then return end
    if f:IsObjectType("StatusBar") and f:IsVisible() then xpBar = f return end
    for _, c in ipairs({ f:GetChildren() }) do find(c, depth + 1) end
end
find(MainStatusTrackingBarContainer, 0)
local _, xpMax = xpBar:GetMinMaxValues()
check("the experience bar: 293 of 900", xpBar:GetValue() == 293 and xpMax == 900)
