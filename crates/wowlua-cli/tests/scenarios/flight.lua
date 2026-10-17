-- /stdtest engine scenario flight, played with the engine's controls: the
-- level 17 druid passes another player, talks to Tal (Thunder Bluff's
-- flight master), asks for a ride, looks at the taxi map and closes it
-- without flying (scenarios/flight.lua). Each step at its recorded time (a
-- click: a moment before the server's answer), the ping as recorded
-- (~0.3 s). The map's nodes as the client answered them.
local e = __engine
local player = e.player
e.setLatency(0.3)
local other = e.spawnUnit({
    name = "Otherplayer", lastName = "Othername", guid = "Player-0000-00000000", player = true,
    level = 17, reaction = "friendly", class = "HUNTER", classID = 3, sex = 2, powerType = 0,
    race = "Orc", creatureType = "Humanoid", creatureTypeID = 7, pvp = true,
})
local tal = e.spawnUnit({
    name = "Tal", guid = "Creature-0-4615-1-2-2995-E0D69E7BFE", npcID = 2995, level = 55, classification = "elite",
    reaction = "friendly", class = "WARRIOR", classID = 1, sex = 2, powerType = 1,
    creatureType = "Humanoid", creatureTypeID = 7, factionGroup = "Horde",
})
e.defineGossip(tal, {
    text = "When mounted on the back of a wind rider, one sees that its reputation for strength and speed is well earned.",
    options = { { name = "I need a ride.", icon = 132057, gossipOptionID = 98025, opens = "taxi" } },
})
-- Kalimdor's flight paths as the druid knew them (state: 0 here, 1
-- reachable, 2 not).
local nodes = {
    { name = "Thunder Bluff, Mulgore", nodeID = 22, x = 0.44947779178619, y = 0.56151235103607, state = 0, cost = 0, routes = 0 },
    { name = "Orgrimmar, Durotar", nodeID = 23, x = 0.62800776958466, y = 0.44340223073959, state = 1, cost = 50, routes = 1 },
    { name = "Crossroads, The Barrens", nodeID = 25, x = 0.55735743045807, y = 0.53047657012939, state = 1, cost = 110, routes = 1 },
    { name = "Sun Rock Retreat, Stonetalon Mountains", nodeID = 29, x = 0.4079572558403, y = 0.47261422872543, state = 2, cost = 210, routes = 1 },
    { name = "Freewind Post, Thousand Needles", nodeID = 30, x = 0.549889087677, y = 0.73449921607971, state = 2, cost = 430, routes = 1 },
    { name = "Shadowprey Village, Desolace", nodeID = 38, x = 0.31660270690918, y = 0.58494818210602, state = 2, cost = 530, routes = 1 },
    { name = "Gadgetzan, Tanaris", nodeID = 40, x = 0.60601317882538, y = 0.80192649364471, state = 2, cost = 730, routes = 1 },
    { name = "Camp Mojache, Feralas", nodeID = 42, x = 0.44250988960266, y = 0.69391369819641, state = 2, cost = 730, routes = 1 },
    { name = "Valormok, Azshara", nodeID = 44, x = 0.63107562065125, y = 0.36189317703247, state = 2, cost = 830, routes = 1 },
    { name = "Bloodvenom Post, Felwood", nodeID = 48, x = 0.46455299854279, y = 0.30409198999405, state = 2, cost = 0, routes = 0 },
    { name = "Everlook, Winterspring", nodeID = 53, x = 0.64014458656311, y = 0.23241329193115, state = 2, cost = 0, routes = 0 },
    { name = "Brackenwall Village, Dustwallow Marsh", nodeID = 55, x = 0.56746834516525, y = 0.6416347026825, state = 2, cost = 630, routes = 1 },
    { name = "Zoram'gar Outpost, Ashenvale", nodeID = 58, x = 0.40973830223083, y = 0.3736766576767, state = 2, cost = 0, routes = 0 },
    { name = "Splintertree Post, Ashenvale", nodeID = 61, x = 0.55441868305206, y = 0.41773253679276, state = 2, cost = 0, routes = 0 },
    { name = "Moonglade", nodeID = 69, x = 0.53793668746948, y = 0.20540714263916, state = 2, cost = 0, routes = 0 },
    { name = "Cenarion Hold, Silithus", nodeID = 72, x = 0.41632127761841, y = 0.79216885566711, state = 2, cost = 0, routes = 0 },
    { name = "Camp Taurajo, The Barrens", nodeID = 77, x = 0.52804720401764, y = 0.61013436317444, state = 2, cost = 110, routes = 1 },
    { name = "Marshal's Refuge, Un'Goro Crater", nodeID = 79, x = 0.49764585494995, y = 0.76350945234299, state = 2, cost = 0, routes = 0 },
    { name = "Ratchet, The Barrens", nodeID = 80, x = 0.6057111620903, y = 0.54907929897308, state = 1, cost = 220, routes = 2 },
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
e.defineTaxi(tal, { mapID = 1464, nodes = nodes, mapNodes = mapNodes })
local now = 0
-- Time passes in frames, as in the client (60 a second: Blizzard's
-- animations and OnUpdate code run), the rest of a step as one jump.
local function at(t)
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
at(1.459) player:target(other)
at(1.854) player:target(tal)
tal:setDistance(3)
player:interact(tal)
at(3.0)
check("the gossip shown", GossipFrame:IsShown())
local option = button(GossipFrame, "I need a ride.")
check("the gossip's option shown", option ~= nil)
at(5.676) if option then press(option) end
at(6.5)
check("the taxi map shown", TaxiFrame and TaxiFrame:IsShown())
at(13.557) if TaxiFrame then HideUIPanel(TaxiFrame) end
at(14)
check("the taxi map closed", TaxiFrame and not TaxiFrame:IsShown())
at(16.5)
