-- /stdtest engine scenario taxi, played with the engine's controls: the
-- level 21 paladin talks to Doras (Orgrimmar's flight master), asks for a
-- ride, picks the Crossroads on the taxi map and flies there, out of the
-- city, over Durotar and the river into the Barrens (scenarios/taxi.lua).
-- Each step at its recorded time (a click: a round trip before the
-- server's answer), the places on the way as the client told them.
local e = __engine
local player = e.player
-- The paladin's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 10300, 19740, 1022, 1038, 19979, 25290, 5599, 19834, 10292, 19876, 25898, 19977, 10293, 7294, 19898, 19978, 19888, 20729, 19837, 10298, 465, 10278, 25782, 19850, 10290, 6940, 10301, 10299, 25916, 19896, 19899, 1044, 25890, 19838, 25895, 25291, 1032, 19854, 25918, 19836, 19742, 19900, 19746, 25894, 19853, 19895, 19897, 19852, 10291, 19835, 19891, 20217, 643 })
-- Judgement off the global cooldown; Find Minerals and Sense Undead
-- tracking (usable on the taxi); Blacksmithing and Smelting professions
-- (usable all the flight): as recorded.
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
local doras = e.spawnUnit({
    name = "Doras", guid = "Creature-0-6782-1-4-3310-54D6D229C9", npcID = 3310, level = 55, classification = "elite",
    reaction = "friendly", class = "WARRIOR", classID = 1, sex = 2, powerType = 1,
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde", pvp = true,
})
e.defineGossip(doras, {
    text = "You haven't lived until you've looked down on the world from the back of a wind rider.",
    options = { { name = "I need a ride.", icon = 132057, gossipOptionID = 98541, opens = "taxi" } },
})
-- Kalimdor's flight paths as the paladin knew them in Orgrimmar (state: 0
-- here, 1 reachable, 2 not); the Crossroads' flight as long as it took.
local nodes = {
    { name = "Thunder Bluff, Mulgore", nodeID = 22, x = 0.44947779178619, y = 0.56151235103607, state = 1, cost = 50, routes = 1, hops = { 2, 1 } },
    { name = "Orgrimmar, Durotar", nodeID = 23, x = 0.62800776958466, y = 0.44340223073959, state = 0, cost = 0, routes = 0 },
    { name = "Crossroads, The Barrens", nodeID = 25, x = 0.55735743045807, y = 0.53047657012939, state = 1, cost = 110, routes = 1, hops = { 2, 3 }, flightTime = 110.176 },
    { name = "Sun Rock Retreat, Stonetalon Mountains", nodeID = 29, x = 0.4079572558403, y = 0.47261422872543, state = 1, cost = 320, routes = 2, hops = { 2, 3, 4 } },
    { name = "Freewind Post, Thousand Needles", nodeID = 30, x = 0.549889087677, y = 0.73449921607971, state = 2, cost = 0, routes = 0 },
    { name = "Shadowprey Village, Desolace", nodeID = 38, x = 0.31660270690918, y = 0.58494818210602, state = 2, cost = 0, routes = 0 },
    { name = "Gadgetzan, Tanaris", nodeID = 40, x = 0.60601317882538, y = 0.80192649364471, state = 2, cost = 730, routes = 1, hops = { 2, 7 } },
    { name = "Camp Mojache, Feralas", nodeID = 42, x = 0.44250988960266, y = 0.69391369819641, state = 2, cost = 0, routes = 0 },
    { name = "Valormok, Azshara", nodeID = 44, x = 0.63107562065125, y = 0.36189317703247, state = 2, cost = 830, routes = 1, hops = { 2, 9 } },
    { name = "Bloodvenom Post, Felwood", nodeID = 48, x = 0.46455299854279, y = 0.30409198999405, state = 2, cost = 830, routes = 1, hops = { 2, 10 } },
    { name = "Everlook, Winterspring", nodeID = 53, x = 0.64014458656311, y = 0.23241329193115, state = 2, cost = 1020, routes = 1, hops = { 2, 11 } },
    { name = "Brackenwall Village, Dustwallow Marsh", nodeID = 55, x = 0.56746834516525, y = 0.6416347026825, state = 2, cost = 630, routes = 1, hops = { 2, 12 } },
    { name = "Zoram'gar Outpost, Ashenvale", nodeID = 58, x = 0.40973830223083, y = 0.3736766576767, state = 2, cost = 0, routes = 0 },
    { name = "Splintertree Post, Ashenvale", nodeID = 61, x = 0.55441868305206, y = 0.41773253679276, state = 2, cost = 530, routes = 1, hops = { 2, 14 } },
    { name = "Moonglade", nodeID = 69, x = 0.53793668746948, y = 0.20540714263916, state = 2, cost = 0, routes = 0 },
    { name = "Cenarion Hold, Silithus", nodeID = 72, x = 0.41632127761841, y = 0.79216885566711, state = 2, cost = 0, routes = 0 },
    { name = "Camp Taurajo, The Barrens", nodeID = 77, x = 0.52804720401764, y = 0.61013436317444, state = 1, cost = 220, routes = 2, hops = { 2, 3, 17 } },
    { name = "Marshal's Refuge, Un'Goro Crater", nodeID = 79, x = 0.49764585494995, y = 0.76350945234299, state = 2, cost = 0, routes = 0 },
    { name = "Ratchet, The Barrens", nodeID = 80, x = 0.6057111620903, y = 0.54907929897308, state = 1, cost = 220, routes = 2, hops = { 2, 3, 19 } },
    { name = "Summit of Eternity, Mount Hyjal", nodeID = 559, x = 0.58655297756195, y = 0.29526746273041, state = 2, cost = 0, routes = 0 },
    { name = "zzOLDPowderfuse Port, Riverglades", nodeID = 3208, x = 0.6057111620903, y = 0.54907929897308, state = 2, cost = 0, routes = 0 },
    { name = "Tainted Foothills, Mount Hyjal", nodeID = 3242, x = 0.5672287940979, y = 0.33181923627853, state = 2, cost = 0, routes = 0 },
}
local mapNodes = {
    { name = "Thunder Bluff, Mulgore", nodeID = 22, x = 0.44947779178619, y = 0.56151235103607, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Orgrimmar, Durotar", nodeID = 23, x = 0.62800776958466, y = 0.44340223073959, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Crossroads, The Barrens", nodeID = 25, x = 0.55735743045807, y = 0.53047657012939, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Auberdine, Darkshore", nodeID = 26, x = 0.42778635025024, y = 0.25179213285446, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Rut'theran Village, Teldrassil", nodeID = 27, x = 0.41614419221878, y = 0.15720665454865, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Astranaar, Ashenvale", nodeID = 28, x = 0.46258175373077, y = 0.39616519212723, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Sun Rock Retreat, Stonetalon Mountains", nodeID = 29, x = 0.4079572558403, y = 0.47261422872543, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Freewind Post, Thousand Needles", nodeID = 30, x = 0.549889087677, y = 0.73449921607971, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Thalanaar, Feralas", nodeID = 31, x = 0.48257559537888, y = 0.69687259197235, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Theramore, Dustwallow Marsh", nodeID = 32, x = 0.63626050949097, y = 0.66948932409286, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Stonetalon Peak, Stonetalon Mountains", nodeID = 33, x = 0.39064586162567, y = 0.40217214822769, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Nijel's Point, Desolace", nodeID = 37, x = 0.39622759819031, y = 0.50660479068756, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Shadowprey Village, Desolace", nodeID = 38, x = 0.31660270690918, y = 0.58494818210602, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Gadgetzan, Tanaris", nodeID = 39, x = 0.60413265228271, y = 0.80911958217621, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Gadgetzan, Tanaris", nodeID = 40, x = 0.60601317882538, y = 0.80192649364471, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Feathermoon, Feralas", nodeID = 41, x = 0.31353121995926, y = 0.69202136993408, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Camp Mojache, Feralas", nodeID = 42, x = 0.44250988960266, y = 0.69391369819641, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Valormok, Azshara", nodeID = 44, x = 0.63107562065125, y = 0.36189317703247, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Bloodvenom Post, Felwood", nodeID = 48, x = 0.46455299854279, y = 0.30409198999405, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Moonglade", nodeID = 49, x = 0.55288457870483, y = 0.20589774847031, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Everlook, Winterspring", nodeID = 52, x = 0.64553982019424, y = 0.23298108577728, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Everlook, Winterspring", nodeID = 53, x = 0.64014458656311, y = 0.23241329193115, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Brackenwall Village, Dustwallow Marsh", nodeID = 55, x = 0.56746834516525, y = 0.6416347026825, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Zoram'gar Outpost, Ashenvale", nodeID = 58, x = 0.40973830223083, y = 0.3736766576767, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Splintertree Post, Ashenvale", nodeID = 61, x = 0.55441868305206, y = 0.41773253679276, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Nighthaven, Moonglade", nodeID = 62, x = 0.54944407939911, y = 0.19212782382965, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Nighthaven, Moonglade", nodeID = 63, x = 0.54946994781494, y = 0.19236975908279, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Talrendis Point, Azshara", nodeID = 64, x = 0.6101331114769, y = 0.40049344301224, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Talonbranch Glade, Felwood", nodeID = 65, x = 0.53079825639725, y = 0.25735914707184, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Moonglade", nodeID = 69, x = 0.53793668746948, y = 0.20540714263916, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Cenarion Hold, Silithus", nodeID = 72, x = 0.41632127761841, y = 0.79216885566711, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Cenarion Hold, Silithus", nodeID = 73, x = 0.41897988319397, y = 0.79013270139694, atlasName = "TaxiNode_Alliance", faction = 2, isUndiscovered = false },
    { name = "Camp Taurajo, The Barrens", nodeID = 77, x = 0.52804720401764, y = 0.61013436317444, atlasName = "TaxiNode_Horde", faction = 1, isUndiscovered = false },
    { name = "Marshal's Refuge, Un'Goro Crater", nodeID = 79, x = 0.49764585494995, y = 0.76350945234299, atlasName = "TaxiNode_Neutral", faction = 0, isUndiscovered = false },
    { name = "Ratchet, The Barrens", nodeID = 80, x = 0.6057111620903, y = 0.54907929897308, atlasName = "TaxiNode_Neutral", faction = 0, isUndiscovered = false },
    { name = "Summit of Eternity, Mount Hyjal", nodeID = 559, x = 0.58655297756195, y = 0.29526746273041, atlasName = "TaxiNode_Neutral", faction = 0, isUndiscovered = false },
    { name = "zzOLDPowderfuse Port, Riverglades", nodeID = 3208, x = 0.6057111620903, y = 0.54907929897308, atlasName = "TaxiNode_Neutral", faction = 0, isUndiscovered = false },
    { name = "Tainted Foothills, Mount Hyjal", nodeID = 3242, x = 0.5672287940979, y = 0.33181923627853, atlasName = "TaxiNode_Neutral", faction = 0, isUndiscovered = false },
}
e.defineTaxi(doras, { mapID = 1464, nodes = nodes, mapNodes = mapNodes })
-- Someone else's: the flight master turns to the players around.
local someone = e.spawnUnit({ name = "Someone", guid = "Creature-0-6782-1-4-3310-0000000001", npcID = 3310, level = 1, reaction = "friendly" })
-- The worn gear's durability (the recording's answers).
for slot, d in pairs({ [3] = { 48, 50 }, [5] = { 93, 95 }, [6] = { 35, 35 }, [8] = { 37, 40 }, [9] = { 30, 30 }, [10] = { 35, 35 }, [16] = { 69, 70 } }) do
    player:setDurability(slot, d[1], d[2])
end
-- The world's own, not modelled: the server's events as recorded.
local function world(...) e.fireEvent(...) end
-- The player's target cleared (`PLAYER_TARGET_CHANGED`, the server's
-- echo a round trip later); the server's own change of it before
-- (`UNIT_TARGET("player")` alone), as recorded on the flight.
local function cleared() player:target(nil) end
local function servers() world("UNIT_TARGET", "player") end
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
-- The places on the way (the recording's zone texts and maps).
local function place(zone, subZone, mapID, more)
    local p = { zone = zone, subZone = subZone, mapID = mapID }
    for k, v in pairs(more or {}) do p[k] = v end
    e.moveTo(p)
end
-- Talking to Doras: the round trip then 0.445 s.
e.setLatency(0.445)
at(1.801) player:target(doras)
doras:setDistance(3)
player:interact(doras)
player:softTarget(doras, "friend")
at(2.467) doras:target(someone)
at(2.891) doras:target(player)
at(3.627) e.spawnUnit({ name = "Someone's kill", guid = "Creature-0-6782-1-4-6466-2E87AA3F78", npcID = 6466, level = 1, reaction = "hostile" }):kill()
at(4.061) doras:target(someone) world("UPDATE_EXHAUSTION")
at(4.473) doras:target(player)
at(5.340) doras:target(someone)
at(5.789) doras:target(player)
at(6.088) doras:target(someone)
at(6.5)
check("the gossip shown", GossipFrame:IsShown())
local option = button(GossipFrame, "I need a ride.")
check("the gossip's option shown", option ~= nil)
-- The option chosen: the round trip then 0.487 s.
e.setLatency(0.487)
at(6.855) if option then press(option) end
at(7.342) doras:target(player)
at(7.821) doras:target(someone)
at(8.523) doras:target(player)
at(8.8)
check("the taxi map shown", TaxiFrame and TaxiFrame:IsShown())
check("the Crossroads on the map", TaxiButton3 and TaxiButton3:IsShown())
-- The Crossroads clicked: the server's answer 0.48 s later.
e.setLatency(0.48)
at(8.912) if TaxiButton3 then press(TaxiButton3) end
at(9.6)
check("flying: the map closed", not TaxiFrame:IsShown())
check("flying: on the taxi", UnitOnTaxi("player"))
-- The server's cooldown for a spell (416221, not the player's) and its
-- updates of the worn items (no change), their causes unknown.
at(11.495)
world("ACTIONBAR_UPDATE_COOLDOWN")
world("BAG_UPDATE_COOLDOWN")
world("SPELL_UPDATE_COOLDOWN", 416221, nil, nil, nil, nil)
world("UPDATE_SHAPESHIFT_COOLDOWN")
at(13.062) player:wear() player:wear()
at(13.861) for _ = 1, 7 do player:wear() end
-- Out of the city, Doras out of sight.
at(14.162) place("Orgrimmar", "Valley of Spirits", 1454, { wmo = true })
at(14.262) cleared()
at(15.828) place("Orgrimmar", "Orgrimmar", 1454, { wmo = true })
at(18.495) e.newWmoChunk()
at(23.161) e.newWmoChunk()
at(26.295) world("MINIMAP_UPDATE_ZOOM")
at(26.495) e.newWmoChunk()
at(28.162) place("Durotar", "Southfury River", 1411)
at(28.495) place("Orgrimmar", "Orgrimmar", 1454, { pvp = { "friendly", false, "Horde" } })
at(29.761) world("REQUEST_CEMETERY_LIST_RESPONSE", false)
at(30.162) place("The Barrens", "Southfury River", 1414)
at(30.195) world("WEATHER_CHANGED")
-- Out of the city: its trade channels left, no longer resting.
at(30.395)
world("CHANNEL_UI_UPDATE")
world("CHANNEL_LEFT", 2, "Trade - English")
world("CHANNEL_UI_UPDATE")
world("CHANNEL_UI_UPDATE")
world("CHANNEL_LEFT", 42, "Trade (Services) - English")
world("CHANNEL_UI_UPDATE")
world("CHANNEL_LEFT", 46, "Trade (Local) - Orgrimmar")
world("CHANNEL_UI_UPDATE")
at(30.628) world("PLAYER_FLAGS_CHANGED", "player") world("PLAYER_UPDATE_RESTING")
at(30.995) world("PLAYER_CHOICE_UPDATE")
-- Another server's shard.
at(33.228)
world("SHARD_TRANSFER")
world("GARRISON_HIDE_LANDING_PAGE")
world("SCENARIO_UPDATE", nil)
world("MINIMAP_UPDATE_ZOOM")
at(33.562)
world("PLAYER_FLAGS_CHANGED", "player")
world("REQUEST_CEMETERY_LIST_RESPONSE", false)
world("PLAYER_FLAGS_CHANGED", "player")
world("GARRISON_UPDATE")
world("GARRISON_HIDE_LANDING_PAGE")
world("GARRISON_LANDINGPAGE_SHIPMENTS")
world("WEATHER_CHANGED")
at(38.295) world("TREASURE_PICKER_CACHE_FLUSH")
at(40.495) place("The Barrens", "", 1413)
at(44.828) place("The Barrens", "Boulder Lode Mine", 1413)
at(50.537) place("The Barrens", "", 1413)
at(54.858) servers()
at(55.292) cleared()
at(56.125) servers()
at(56.458) cleared()
at(60.558) servers() cleared()
at(67.792) servers()
at(68.325) cleared()
at(69.458) servers()
at(70.539) cleared()
at(78.703) servers()
at(78.770) cleared()
at(79.570) cleared()
at(80.737) servers()
at(81.570) cleared()
at(82.570) cleared()
at(82.703) servers()
at(84.736) place("The Barrens", "Grol'dom Farm", 1413)
at(91.403) e.areaChanged()
at(92.070) e.newWmoChunk()
at(96.970) cleared()
at(99.070) place("The Barrens", "", 1413)
at(99.403) place("The Barrens", "Gold Road", 1413)
at(102.403) place("The Barrens", "", 1413)
at(103.737) place("The Barrens", "Gold Road", 1413)
at(109.070) place("The Barrens", "", 1413)
at(114.404) place("The Barrens", "The Crossroads", 1413)
at(119.0)
check("still flying", UnitOnTaxi("player") and not HasFullControl())
at(120.0)
check("landed", not UnitOnTaxi("player") and HasFullControl())
check("landed: the fare paid", GetMoney() == 31712)
-- At the Crossroads, standing.
at(127.234) player:wear()
at(216.2)
