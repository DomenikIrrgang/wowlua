-- /stdtest engine scenario questlog, played with the engine's controls:
-- the level 21 paladin in the Crossroads opens the quest log (the world
-- map's) with L; collapses and expands Stonetalon Mountains, collapses it,
-- The Barrens, Thunder Bluff and Wailing Caverns; opens the map's filter
-- menu and clicks the map; expands Paladin and opens A Moon-Kissed Blade's
-- details, goes back; expands Blackfathom Deeps and opens Trouble in the
-- Deeps' details (scenarios/questlog.lua). Each step at its recorded time
-- (a click's release).
local e = __engine
local player = e.player
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
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
-- The world's own, not modelled: the server's events as recorded.
local function world(...) e.fireEvent(...) end
-- The quest log's shown button for a header (by its log index) or a quest
-- (by its ID).
local function logButton(header, quest)
    local found
    local function walk(f, depth)
        if found or depth > 12 or not f or f:IsForbidden() then return end
        if f:IsVisible() and f.GetButtonType and f:IsObjectType("Button") then
            local kind = f:GetButtonType()
            if header and kind == QuestLogButtonTypes.Header and f.questLogIndex == header then found = f return end
            if quest and kind == QuestLogButtonTypes.Quest and f.questID == quest then found = f return end
        end
        for _, c in ipairs({ f:GetChildren() }) do walk(c, depth + 1) end
    end
    walk(QuestScrollFrame, 0)
    return found
end
local function header(index)
    local b = logButton(index)
    check("the header " .. index .. " shown", b ~= nil)
    if b then press(b) end
end
local function quest(id)
    local b = logButton(nil, id)
    check("the quest " .. id .. " shown", b ~= nil)
    if b then press(b) end
end
-- The log as Ashenvale's map shows it (the client's answers while a
-- quest's details showed it): Blackfathom Deeps first, the quests on the
-- map (each with its place there); A Moon-Kissed Blade's and Trouble in
-- the Deeps' map.
local ashenvale = { 6562, 6301, 1068, 1087, 6393, 6282, 1095, 95036 }
e.defineQuestMap(1440, { first = "Blackfathom Deeps", onMap = ashenvale, localPOI = ashenvale, quests = { 95036, 6562 } })
-- Opened: Steelsnap's data asked for came in the same frame (the client
-- had them), the log's update with them (the server's, as in `profession`
-- and `talents`; after a quest accepted, `quest`, the data came after the
-- update).
at(2.095) e.pressKey("L")
world("QUEST_LOG_UPDATE")
at(2.3)
check("the quest log open", QuestMapFrame:IsVisible())
-- The map's quest lines (the server's answers to its asks: the map's
-- shown and its map set, 0.371 s).
at(2.466) world("QUESTLINE_UPDATE", false)
world("QUESTLINE_UPDATE", false)
-- Stonetalon Mountains collapsed, expanded, collapsed; The Barrens,
-- Thunder Bluff and Wailing Caverns collapsed.
at(5.514) header(1)
at(5.6)
check("Stonetalon collapsed", C_QuestLog.GetInfo(1).isCollapsed)
at(6.036) header(1)
at(13.639) header(1)
at(14.112) header(11)
at(14.480) header(14)
at(15.081) header(16)
-- Paladin expanded, its quest's details shown, back.
at(28.587) header(32)
at(28.981) quest(95036)
at(29.2)
check("the details shown", QuestMapFrame.DetailsFrame:IsShown() and WorldMapFrame:GetMapID() == 1440)
at(29.405) world("QUESTLINE_UPDATE", false)
at(30.082) press(QuestMapFrame.DetailsFrame.BackFrame.BackButton)
-- Blackfathom Deeps expanded, its quest's details shown.
at(30.533) header(20)
at(30.917) quest(6562)
at(31.2)
check("the second details shown", QuestMapFrame.DetailsFrame:IsShown() and C_QuestLog.GetSelectedQuest() == 6562)
at(31.298) world("QUESTLINE_UPDATE", false)
at(46)
