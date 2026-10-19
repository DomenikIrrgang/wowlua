-- /stdtest engine scenario talents, played with the engine's controls: the
-- level 11 shaman opens its talents (N), buys a rank of node 104753 and
-- applies it; talks to the town's trainer and walks away; is attacked by a
-- fox that gives up; walks on through the island (the quest log's updates);
-- targets another of the user's characters, a druid, follows it and views
-- its talents (scenarios/talents.lua). Each step at its recorded time.
-- Earth Shock's range checks (SPELL_RANGE_CHECK_UPDATE: the cooldown
-- viewer's) are the recorded events: the engine checks no range.
local e = __engine
-- The damage meter's sessions numbered on from the client's before the
-- recording (its first: 4).
e.setDamageMeterSession(4)
local player = e.player
-- The shaman's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 8185, 10613, 10478, 10534, 10441, 15036, 10609, 8836, 15111, 8249, 1239242, 10491, 17354, 8076, 8162, 15107, 10537, 16176, 8160, 10535, 15112, 8184, 8163, 10538, 10614, 16191, 17355, 5677, 10526, 8835, 17360, 10494, 8515, 8182, 10497, 15109, 10496, 8512, 16190, 10493, 8181, 10626, 8230, 10479, 1239243, 25362, 10495, 8227, 15108, 15110, 408521, 1325257, 10627, 10477, 17359, 16387, 8075, 1325256, 10521, 25360, 10442, 25361, 10476, 5678, 10612, 8161, 25359, 8250 })
-- Before: the node not bought yet (the probe's ranks are the end's: 1).
e.setTraitRanks(5999541, { [104753] = 0 })
-- The recorded character's trait data (its class config's ID).
e.setTraitCharacter(5999541)
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
    if not frame then
        check("clicked: no frame", false)
        return
    end
    local ok, err = pcall(e.click, frame, button)
    if not ok then check("clicked: " .. tostring(err), false) end
end
e.setLatency(0.465)
at(5.969) e.pressKey("N")
-- Walking into the town (6.333 on): its walls' chunk, resting.
at(7.078) e.newWmoChunk()
at(7.847) e.setResting(true)
check("the talents shown", PlayerSpellsFrame and PlayerSpellsFrame:IsShown()
    and PlayerSpellsFrame.TalentsFrame:IsVisible())
local talents = PlayerSpellsFrame.TalentsFrame
local node = talents:GetTalentButtonByNodeID(104753)
check("the node's button shown", node and node:IsVisible())
at(8.508) press(node)
at(10.078) press(talents.ApplyButton)
at(11.0)
check("the rank applied", C_Traits.GetNodeInfo(5999541, 104753).ranksPurchased == 1
    and not C_Traits.ConfigHasStagedChanges(5999541))
at(12.0)
-- The town's shaman trainer: targeted, talked to (its gossip: training,
-- a lost item, a quest; its text 0.548 s after the interaction).
local aarnor = e.spawnUnit({
    name = "Aarnor Galestrike", guid = "Creature-0-4615-2991-3-254082-4973C18C42", npcID = 254082, level = 10,
    reaction = "friendly", class = "PALADIN", classID = 2, sex = 2, powerType = 0, pvp = false,
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde",
})
aarnor:setDistance(3)
e.defineGossip(aarnor, {
    text = "The spirits of the wind may have left us, but the power of the elements is not entirely out of reach for those with the patience to seek them out.\r\n\r\nIf you are initiated in the ways of the shaman, I can help you grasp them.",
    textAfter = 0.548,
    options = {
        { name = "I'd like training!", icon = 132058, gossipOptionID = 136811 },
        { name = "I've misplaced my Earth Sapta. May I have another?", icon = 132053, gossipOptionID = 139262 },
    },
    availableQuests = { { questID = 97243, title = "Call of Fire" } },
})
-- Two other shamans passing (their names and GUIDs scrubbed), each
-- targeting an NPC (the first's GUID unrecorded).
local illaya = e.spawnUnit({
    name = "Illaya Amberwind", guid = "Creature-0-4615-2991-3-0-0000000000", level = 10,
    reaction = "friendly", creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde",
})
local function shaman(sex)
    local s = e.spawnUnit({
        name = "Otherplayer", lastName = "Othername", guid = "Player-0000-00000000", player = true, level = 6,
        reaction = "friendly", class = "SHAMAN", classID = 7, sex = sex, powerType = 0, race = "Windshaper Skyborne",
        creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde", pvp = false,
    })
    s:setDistance(5)
    return s
end
local shaman1, shaman2 = shaman(2), shaman(3)
shaman1:target(illaya)
shaman2:target(aarnor)
-- The server's echo of the first target 0.836 s later (recorded: after
-- the interaction's), the interaction's a round trip after the click.
e.setLatency(0.836)
at(12.737) player:target(aarnor)
e.setLatency(0.469)
at(12.819) player:interact(aarnor)
e.setLatency(0.656)
at(13.679) player:target(shaman1)
at(14.0)
check("the gossip shown", GossipFrame:IsShown())
e.setLatency(0.538)
at(15.418) player:target(shaman2)
-- Walked away: the gossip closes.
at(15.792) aarnor:setDistance(40)
at(16.0)
check("the gossip closed", not GossipFrame:IsShown())
-- The world's own, not modelled: the server's events as recorded (the
-- quest log's and quest lines' updates while walking, the devices').
local function world(...) e.fireEvent(...) end
-- Someone else's kill nearby.
local kill1 = e.spawnUnit({
    name = "Creature", guid = "Creature-0-4615-2991-3-251245-A55DDEC273", npcID = 251245, level = 10,
    reaction = "hostile",
})
at(17.887) kill1:kill()
at(23.367) world("QUEST_LOG_UPDATE")
at(23.788) world("QUESTLINE_UPDATE", false)
at(28.241) world("QUEST_LOG_UPDATE")
-- The target cleared (its echo 0.524 s later).
e.setLatency(0.524)
at(30.872) player:target(nil)
at(37.169) world("QUEST_LOG_UPDATE")
-- A fox attacks the walking shaman; targeted; its hits taken until it
-- gives up. (Each hit at the health's update, its combat text recorded
-- later: the order kept.)
player:setName("Player", "Lastname")
local vuldren = e.spawnUnit({
    name = "Vuldren", guid = "Creature-0-4615-2991-3-250868-4DDC3583E7", npcID = 250868, level = 6,
    reaction = "hostile", class = "WARRIOR", classID = 1, sex = 1, powerType = 1,
    creatureType = "Beast", creatureTypeID = 1, creatureFamily = "Fox", creatureFamilyID = 319,
})
vuldren:setDistance(3)
at(39.002) vuldren:target(player)
vuldren:enterCombat()
player:enterCombat()
vuldren:setThreat(0)
e.setLatency(0.699)
-- Earth Shock's range checks (the cooldown viewer's: its cvar on in the
-- client, not in the profile; the engine checks no range): as recorded.
local function earthShockInRange(inRange, checksRange)
    world("SPELL_RANGE_CHECK_UPDATE", 8044, inRange, checksRange)
end
at(39.516) player:target(vuldren)
earthShockInRange(true, true)
vuldren:swing(player, 6)
-- Its soft targeting as the client switched it (PLAYER_SOFT_ENEMY_CHANGED).
at(39.530) player:softTarget(vuldren, "enemy")
at(40.146) player:softTarget(nil, "enemy")
at(40.426) player:softTarget(vuldren, "enemy")
at(41.435) vuldren:swing(player, 5)
at(43.463) vuldren:swing(player, 5)
at(45.558) vuldren:swing(player, 6)
at(47.505) vuldren:swing(player, 6)
at(49.525) vuldren:swing(player, 6)
at(51.540) vuldren:swing(player, 5)
-- It gives up: the player out of combat, its target dropped (the
-- engine's), its last hit landing as it goes (the health's update, the
-- combat text 1.33 s later: recorded). The health comes back every 2 s
-- after (the amounts guessed: the client keeps them secret).
at(53.550) player:leaveCombat()
vuldren:swing(player, 6, { combat = false, textAfter = 1.33 })
at(53.713) player:softTarget(nil, "enemy")
-- Out of the fox's reach, in the highlands (the quest watch's update
-- after the map's, as recorded).
at(53.918) e.moveTo({ subZone = "Shen'dar Highlands" })
at(53.96) world("QUEST_WATCH_LIST_CHANGED", nil, nil)
at(54.467) player:heal(3)
at(54.567) vuldren:setDistance(40)
earthShockInRange(false, true)
at(56.090) player:softTarget(vuldren, "enemy")
at(56.428) player:heal(3)
at(58.457) player:heal(3)
e.setLatency(0.834)
at(60.057) player:target(nil)
earthShockInRange(false, false)
at(60.491) player:heal(3)
at(62.491) player:heal(3)
at(64.492) player:heal(3)
at(66.525) player:heal(3)
at(67.759) world("SOUND_DEVICE_UPDATE")
at(68.085) world("VOICE_CHAT_INPUT_DEVICES_UPDATED")
world("VOICE_CHAT_OUTPUT_DEVICES_UPDATED")
-- Walking on: the areas passed (their walls' chunks, two subzones), a
-- kill nearby, the quest log's updates.
at(101.924) e.areaChanged()
at(113.848) world("QUEST_LOG_UPDATE")
at(114.305) world("QUESTLINE_UPDATE", false)
at(115.493) e.newWmoChunk()
-- (After the map's updates, as recorded.)
at(115.53) world("QUEST_LOG_UPDATE")
at(116.879) world("QUEST_LOG_UPDATE")
at(121.309) world("QUEST_LOG_UPDATE")
at(128.280) world("QUEST_LOG_UPDATE")
at(163.307) e.areaChanged()
at(167.345) e.newWmoChunk()
local kill2 = e.spawnUnit({
    name = "Creature", guid = "Creature-0-4615-2991-3-251661-7682E00834", npcID = 251661, level = 10,
    reaction = "hostile",
})
at(168.079) kill2:kill()
at(172.366) world("QUEST_LOG_UPDATE")
at(172.847) world("QUESTLINE_UPDATE", false)
at(205.011) e.moveTo({ subZone = "Gustberry Lowlands" })
at(210.717) world("QUEST_LOG_UPDATE")
at(211.118) world("QUESTLINE_UPDATE", false)
at(259.981) e.areaChanged()
at(260.398) world("QUEST_LOG_UPDATE")
at(260.794) world("QUESTLINE_UPDATE", false)
at(271.536) e.newWmoChunk()
at(280.358) world("QUEST_LOG_UPDATE")
at(286.600) world("QUEST_LOG_UPDATE")
at(304.884) e.moveTo({ subZone = "Valanaar" })
at(311.817) world("QUEST_LOG_UPDATE")
at(312.187) world("QUESTLINE_UPDATE", false)
at(325.220) world("QUEST_LOG_UPDATE")
-- A citizen targeted and right-clicked from too far.
local citizen = e.spawnUnit({
    name = "Zephras Citizen", guid = "Creature-0-4615-2991-3-254100-4AD179E0ED", npcID = 254100, level = 10,
    reaction = "friendly", class = "PALADIN", classID = 2, sex = 2, powerType = 0, pvp = false,
    creatureType = "Humanoid", creatureTypeID = 7,
})
citizen:setDistance(20)
e.setLatency(0.738)
at(327.985) player:target(citizen)
player:interact(citizen)
at(336.404) world("QUEST_LOG_UPDATE")
-- Another of the user's characters, a druid in bear form: targeted,
-- followed and its talents viewed from the target's menu.
local druid = e.spawnUnit({
    name = "Player", lastName = "Lastname", guid = "Player-0000-00000000", player = true, level = 14,
    reaction = "friendly", class = "DRUID", classID = 11, sex = 3, powerType = 1, race = "Windshaper Skyborne",
    creatureType = "Beast", creatureTypeID = 1, factionGroup = "Horde", pvp = false,
})
druid:setDistance(3)
druid:setInspect({
    spec = 1484, itemLevel = 6,
    items = {
        [4] = { link = "|cnIQ1:|Hitem:271668::::::::11:1489::75:::::::|h[Thendal Survivalist's Shirt]|h|r", cached = true },
        [11] = { link = "|cnIQ1:|Hitem:264908::::::::11:1489::11:::::::|h[Ancient Heirloom]|h|r", cached = true },
        [15] = { link = "|cnIQ2:|Hitem:257943::::::::11:1489::11:::::::|h[Hunter's Simple Cloak]|h|r", cached = true },
    },
    config = { ID = -1, name = "Druid", treeIDs = { 1089 }, type = 4, usesSharedActionBars = false },
})
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
e.setLatency(0.783)
at(344.444) player:target(druid)
at(344.458) player:softTarget(druid, "friend")
at(345.854) press(TargetFrame, "RightButton")
at(347.127) press(menuEntry("Follow"))
e.setLatency(0.79)
at(348.392) press(TargetFrame, "RightButton")
at(349.598) press(menuEntry("View Talents"))
at(350.5)
check("its talents shown", PlayerSpellsFrame:IsShown())
-- The following ended (its cause not recorded: twice, as recorded).
at(352.503) world("AUTOFOLLOW_END")
world("AUTOFOLLOW_END")
-- The druid's own target changed (to whom not recorded).
at(353.772) druid:target(nil)
at(354.0)
