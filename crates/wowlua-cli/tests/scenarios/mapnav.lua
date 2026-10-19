-- /stdtest engine scenario mapnav, played with the engine's controls: the
-- world map opened (M), zoomed out to Kalimdor with right-clicks, the
-- Barrens clicked into, a map pin placed with a ctrl-click, clicked to be
-- super-tracked, ctrl-clicked away, the map closed (scenarios/mapnav.lua).
-- Each step at its recorded time (a click: when the client acted).
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
local map = WorldMapFrame
-- The mouse over the map's canvas at a normalized point of the map shown.
local function pointAt(nx, ny)
    local child = map.ScrollContainer.Child
    local scale = child:GetEffectiveScale()
    local left, top = child:GetLeft(), child:GetTop()
    local w, h = child:GetWidth(), child:GetHeight()
    e.setCursorPosition((left + nx * w) * scale, (top - ny * h) * scale)
    e.hover(map.ScrollContainer)
end
local function script(frame, name, ...)
    local ok, err = pcall(e.script, frame, name, ...)
    if not ok then check(name .. ": " .. tostring(err), false) end
end
-- The client's quest log update as the map was shown or hidden (recorded
-- each time; its cause not modelled: Blizzard's quest map sets the quest
-- POIs' map on every update, the same map at both; a change to another
-- map's view of the log is the engine's update).
local function questLog() e.fireEvent("QUEST_LOG_UPDATE") end
-- An enchanting spell's end the server told, its cause unknown (recorded).
local function enchant() e.fireEvent("ENCHANT_SPELL_COMPLETED", false, nil) end
-- The log as Kalimdor's map showed it (the client's answers): Blackfathom
-- Deeps and Orgrimmar first and expanded, Paladin expanded in its place,
-- the quests on the map (each with its place there). Why these two moved
-- up and the Barrens' headers (on Kalimdor too) did not is not known.
local barrens = { 1063, 6481, 6301, 1068, 6393, 86576, 6282, 1095, 894, 3369, 1131, 1486, 962, 959 }
local kalimdor = { 6562, 97275, 1087, 95036 }
for _, id in ipairs(barrens) do kalimdor[#kalimdor + 1] = id end
e.defineQuestMap(1414, {
    first = { "Blackfathom Deeps", "Orgrimmar" },
    expanded = { "Paladin" },
    onMap = kalimdor,
    localPOI = kalimdor,
})
-- Back on the Barrens the log kept Kalimdor's order (its headers' own
-- state, the Barrens' quests on the map) until the map closed (recorded;
-- in `questlog` a quest's details going back from Ashenvale showed the
-- snapshot's order at once).
local function barrensAfterKalimdor()
    e.defineQuestMap(1413, {
        top = { "Blackfathom Deeps", "Orgrimmar" },
        onMap = barrens,
        localPOI = barrens,
    })
end
at(1.866) enchant()
at(2.404) e.pressKey("M") questLog()
check("the world map shown", map:IsShown() and map:GetMapID() == 1413)
-- A right-click the mouse moved during: no click, nothing changed
-- (recorded: no map change, no update).
pointAt(0.5, 0.5)
at(2.650) script(map.ScrollContainer, "OnMouseDown", "RightButton")
pointAt(0.6, 0.6)
at(2.756) script(map.ScrollContainer, "OnMouseUp", "RightButton")
-- The server's quest lines for the map shown (recorded).
at(2.761) e.fireEvent("QUESTLINE_UPDATE", false)
-- Out to Kalimdor.
at(3.281) pointAt(0.5, 0.5) press(map.ScrollContainer, "RightButton")
check("Kalimdor shown", map:GetMapID() == 1414)
barrensAfterKalimdor()
-- Into the Barrens (where on Kalimdor is not recorded: a point of its
-- rectangle in no other zone's taken).
at(6.099) pointAt(0.65, 0.6) press(map.ScrollContainer)
check("the Barrens shown", map:GetMapID() == 1413)
-- A pin placed with a ctrl-click (recorded: at 0.5308, 0.4559).
at(8.644) e.setModifiers({ ctrl = true })
at(8.959) pointAt(0.53084659576416, 0.45588240027428) press(map.ScrollContainer)
at(9.171) e.setModifiers({})
check("a pin", C_Map.HasUserWaypoint())
at(10.026) enchant()
-- The pin clicked: super-tracked.
local function pin()
    for p in map:EnumeratePinsByTemplate("WaypointLocationPinTemplate") do return p end
end
check("its pin shown", pin() ~= nil)
at(14.577) if pin() then press(pin()) end
check("super-tracked", C_SuperTrack.IsSuperTrackingUserWaypoint())
-- Ctrl-clicked away.
at(17.947) e.setModifiers({ ctrl = true })
at(18.230) if pin() then press(pin()) end
at(18.421) e.setModifiers({})
check("no pin", not C_Map.HasUserWaypoint() and not C_SuperTrack.IsSuperTrackingUserWaypoint())
-- Closed.
at(20.167) e.defineQuestMap(1413, nil) e.pressKey("M") questLog()
check("the map closed", not map:IsShown())
at(21.361) enchant()
at(22)
