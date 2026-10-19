-- /stdtest engine scenario lfg, played with the engine's controls: the
-- level 21 paladin, at the Crossroads, opens the group finder (I) and lists
-- itself four times from Blizzard's listing panel, each time delisting from
-- its own entry among the browse tab's results: a dungeon (a playstyle
-- chosen), a zone of Quests & Zones, a Custom listing (its description
-- typed), a dungeon again (a playstyle and a voice chat chosen); another
-- player's invitation comes and is declined (scenarios/lfg.lua). Each step
-- at its recorded time (a click's at its mouse button's release).
-- Unrecorded: which dungeons and zone were listed, which playstyle and
-- voice chat were chosen, the description's text (none of it shows in
-- the recorded events); the ones here stand in.
local e = __engine
local player = e.player
-- The paladin's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded
-- in its other scenarios); Blessing of Protection (slots 42, 50) on the
-- player or a group member only (probed).
e.setHiddenGroupBuffs({ 10300, 19740, 1022, 1038, 19979, 25290, 5599, 19834, 10292, 19876, 25898, 19977, 10293, 7294, 19898, 19978, 19888, 20729, 19837, 10298, 465, 10278, 25782, 19850, 10290, 6940, 10301, 10299, 25916, 19896, 19899, 1044, 25890, 19838, 25895, 25291, 1032, 19854, 25918, 19836, 19742, 19900, 19746, 25894, 19853, 19895, 19897, 19852, 10291, 19835, 19891, 20217, 643 })
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
    if not frame then
        check("clicked: no frame at " .. now, false)
        return
    end
    local ok, err = pcall(e.click, frame, button)
    if not ok then check("clicked: " .. tostring(err), false) end
end
-- A shown frame the predicate picks, anywhere.
local function shown(pick)
    local f = EnumerateFrames()
    while f do
        if not f:IsForbidden() and f:IsVisible() and pick(f) then return f end
        f = EnumerateFrames(f)
    end
end
-- The open menu's entry by its text (a radio entry's font string is its
-- `fontString`).
local function menuEntry(text)
    local menu = Menu.GetManager():GetOpenMenu()
    local function find(f, depth)
        if depth > 6 then return nil end
        for _, child in ipairs({ f:GetChildren() }) do
            if child:IsObjectType("Button") and child:IsVisible() and child.fontString
                and child.fontString:GetText() == text then
                return child
            end
            local found = find(child, depth + 1)
            if found then return found end
        end
    end
    return menu and find(menu, 0)
end
local listing = LFGListingFrame
local function category(id)
    for _, b in ipairs(listing.CategoryView.CategoryButtons) do
        if b.categoryID == id and b:IsVisible() then return b end
    end
end
-- An activity's row in the listing panel, by its name.
local function activityRow(name)
    return shown(function(f)
        return f.NameButton and f.CheckButton and f.NameButton.Name:GetText() == name
    end)
end
-- The player's own entry's delist button among the browse tab's results.
local function delistButton()
    return shown(function(f)
        return f:IsObjectType("Button") and f.tooltipText ~= nil and f.tooltipText == LFG_LIST_UNLIST
    end)
end
-- The server's answers, each its own delay after its request (recorded):
-- the first now, the rest after each request the click makes in turn
-- (Blizzard's code removes the listing, searches, then shows the listing
-- tab, which asks for the saved instances).
local delays = {}
local function answers(first, ...)
    e.setLatency(first)
    delays = { ... }
end
local function nextDelay()
    if #delays > 0 then e.setLatency(table.remove(delays, 1)) end
end
hooksecurefunc(C_LFGList, "RemoveListing", nextDelay)
hooksecurefunc(C_LFGList, "Search", nextDelay)
hooksecurefunc(C_LFGList, "CreateListing", nextDelay)
-- The server's word that results of the last search changed (recorded).
local function updated(...)
    for i = 1, select("#", ...) do
        local id = select(i, ...)
        check("result " .. id .. " updated at " .. now, ScenarioLfg.update(id))
    end
end
-- How many other listings each search found: as many as number the
-- results the server's updates named (recorded: 2 to 34 after the first
-- search, 41 and 42 after the second, 69 after the fifth, 70 after the
-- sixth, 82 to 107 after the seventh, 129 to 142 after the eighth); how
-- they split between searches that named none is a fit.
local WAILING_CAVERNS, HILLSBRAD, CUSTOM, SHADOWFANG_KEEP = 796, 867, 1064, 800

-- Opened with its key: the listing tab asks for the saved instances
-- (answered 3.660).
answers(0.46)
at(3.2) e.pressKey("I")
at(3.8)
check("the group finder shown", LFGParentFrame:IsShown() and listing:IsShown())

-- The first listing: Dungeons, one dungeon, the first playstyle; listed
-- 0.481 s after the click, its search's results 0.780 s after that.
at(4.298) press(category(2))
-- Blessing of Protection told unusable (why unrecorded: no target or soft
-- target change was told).
at(5.943) player:softTarget(e.spawnUnit({ name = "Crossroads Grunt", reaction = "friendly" }), "friend")
at(8.294) press(activityRow("Wailing Caverns") and activityRow("Wailing Caverns").CheckButton)
at(8.759) press(listing.ActivityView.PlayStyleDropdown)
at(9.168) press(menuEntry(GetGeneralPlaystyleString(Enum.LFGEntryGeneralPlaystyle.Learning)))
ScenarioLfg.listings(WAILING_CAVERNS, 33)
answers(0.481, 0.780)
at(9.911) press(listing.PostButton)
at(10.6)
check("listed", C_LFGList.HasActiveEntryInfo())
at(11.442) updated(2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 21, 22, 23, 24, 26, 27, 28, 29, 30, 31, 32, 34)
at(13.547) updated(21)
at(14.547) updated(29, 29)
at(15.153) updated(8, 8, 8, 31, 31, 34, 34, 34)
-- Delisted from its entry: the listing gone 0.417 s later, the saved
-- instances told first; the search's results 0.609 s later.
ScenarioLfg.listings(WAILING_CAVERNS, 20)
answers(0.417, 0.609, 0.416)
at(16.900) press(delistButton())
at(17.4)
check("delisted", not C_LFGList.HasActiveEntryInfo())

-- The second listing: Back, Quests & Zones, a zone.
at(18.425) press(listing.BackButton)
at(19.042) press(category(116))
at(21.567) updated(42, 41)
at(22.256) press(activityRow("Hillsbrad Foothills") and activityRow("Hillsbrad Foothills").CheckButton)
ScenarioLfg.listings(HILLSBRAD, 3)
answers(0.519, 0.518)
at(23.016) press(listing.PostButton)
at(23.6)
check("listed again", C_LFGList.HasActiveEntryInfo())
-- Delisted: the listing gone and the saved instances told 0.474 s later,
-- the search's results in the same frame after them.
ScenarioLfg.listings(HILLSBRAD, 3)
answers(0.474, 0.475, 0.474)
at(29.089) press(delistButton())

-- The third: Back, Dungeons, Back, Custom: its description typed (the
-- box has the focus), posted; listed and its description checked 0.491 s
-- later.
at(30.355) press(listing.BackButton)
at(31.398) press(category(2))
at(32.022) press(listing.BackButton)
at(32.505) press(category(120))
at(33.4) e.type(listing.ActivityView.Comment.EditBox, "Questing in the Barrens")
ScenarioLfg.listings(CUSTOM, 7)
answers(0.491, 0.533)
at(34.439) press(listing.PostButton)
at(35.0)
check("listed with its description", C_LFGList.HasActiveEntryInfo()
    and C_LFGList.GetActiveEntryInfo().comment == "Questing in the Barrens")
at(35.986) updated(69)
-- Delisted: gone 0.597 s later, the saved instances told then too, the
-- search's results 8 ms later.
ScenarioLfg.listings(CUSTOM, 12)
answers(0.597, 0.605, 0.597)
at(37.318) press(delistButton())

-- The fourth: Back, Dungeons, a dungeon's name clicked, a playstyle, a
-- voice chat; listed 0.540 s after the click, its results 0.536 s later.
at(38.334) press(listing.BackButton)
at(38.950) press(category(2))
at(39.668) press(activityRow("Shadowfang Keep") and activityRow("Shadowfang Keep").NameButton)
at(40.325) press(listing.ActivityView.PlayStyleDropdown)
at(40.784) press(menuEntry(GetGeneralPlaystyleString(Enum.LFGEntryGeneralPlaystyle.Learning)))
at(42.860) press(listing.ActivityView.VoiceChatDropdown)
at(43.494) updated(70)
at(44.986) press(menuEntry(VOICE_CHAT_MODE_NONE))
ScenarioLfg.listings(SHADOWFANG_KEEP, 30)
answers(0.540, 0.536)
at(45.237) press(listing.PostButton)
at(45.9)
check("listed a fourth time, without a description", C_LFGList.HasActiveEntryInfo()
    and C_LFGList.GetActiveEntryInfo().comment == "")
at(46.551) updated(100, 82, 92, 93, 101, 106)
at(52.439) updated(103, 95, 93)
at(57.726) updated(93, 93, 93)
-- Another player's invitation (the server's), declined.
at(58.885)
e.fireEvent("PARTY_INVITE_REQUEST", "Otherplayer", true, true, true, false, false, "Player-0000-00000000", false)
e.systemMessage("|Hplayer:Otherplayer|h[Otherplayer]|h has invited you to join a group.")
at(59.438) updated(107, 87)
at(64.736) press(shown(function(f)
    return f:IsObjectType("Button") and f.GetText and f:GetText() == DECLINE
end))
-- Delisted: gone and the saved instances told 0.354 s later, the
-- search's results 0.580 s after the click.
ScenarioLfg.listings(SHADOWFANG_KEEP, 30)
answers(0.354, 0.580, 0.354)
at(67.221) press(delistButton())
at(68.046) updated(138, 142, 129, 137)
at(69.003)
check("delisted at the end", not C_LFGList.HasActiveEntryInfo())
