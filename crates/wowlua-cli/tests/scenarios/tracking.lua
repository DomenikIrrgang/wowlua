-- /stdtest engine scenario tracking, played with the engine's controls:
-- the world map opened and closed from the minimap's zone text, the
-- minimap's tracking menu, Find Minerals on and off, town tracking types
-- toggled, the menu closed, the minimap zoomed in and out
-- (scenarios/tracking.lua). Each step at its recorded time (a click: when
-- the client acted), the server's answers as late as recorded.
local e = __engine
-- The client's frame end: its cooldown and tracking events batched, each
-- once, in their names' order (`setFrameBatching`).
e.setFrameBatching(true)
-- The paladin's spells as the other paladin scenarios have them.
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
-- Find Minerals: its slot (29) told changed as it is sent, its aura as it
-- lands (recorded: instance 45, no duration).
e.defineSpell(2580, {
    name = "Find Minerals", slotChanges = { 0 },
    onCast = function()
        e.player:applyAura({ spellId = 2580, name = "Find Minerals", icon = 136025, auraInstanceID = 45, canApplyAura = true })
    end,
})
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
-- The open menu's entry by its text.
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
local function entry(text)
    local b = menuEntry(text)
    check("the menu's " .. text, b ~= nil)
    if b then press(b) end
end
-- The client's quest log update as the map was shown or hidden (recorded
-- each time; its cause not modelled, as in scenario mapnav).
local function questLog() e.fireEvent("QUEST_LOG_UPDATE") end
-- The world map from the minimap's zone text, and closed again; the
-- server's quest lines for its map (recorded).
at(8.592) press(MinimapCluster.ZoneTextButton) questLog()
check("the world map shown", WorldMapFrame and WorldMapFrame:IsShown())
at(8.961) e.fireEvent("QUESTLINE_UPDATE", false)
at(9.223) press(MinimapCluster.ZoneTextButton) questLog()
check("the world map closed", not WorldMapFrame:IsShown())
-- The tracking menu.
at(10.051) press(MinimapCluster.Tracking.Button)
check("the tracking menu open", Menu.GetManager():GetOpenMenu() ~= nil)
-- Find Minerals on: cast 0.416 s after the click, off: its aura gone
-- 0.382 s after (recorded).
e.setLatency(0.416)
at(23.558) entry("Find Minerals")
at(24.5)
check("finding minerals", C_UnitAuras.GetPlayerAuraBySpellID(2580) ~= nil)
e.setLatency(0.382)
at(25.131) entry("Find Minerals")
at(26)
check("no longer", C_UnitAuras.GetPlayerAuraBySpellID(2580) == nil)
-- Town tracking types toggled: four clicks, each told at once (recorded;
-- which entries is not: Stable Master, off before, on and off twice taken).
at(33.082) entry("Stable Master")
at(33.502) entry("Stable Master")
at(33.878) entry("Stable Master")
at(34.239) entry("Stable Master")
-- The menu closed by a click elsewhere.
at(35.008) Menu.GetManager():CloseMenus()
-- The minimap zoomed in and out.
at(36.5) e.hover(Minimap)
at(36.976) press(Minimap.ZoomIn)
at(37.423) press(Minimap.ZoomOut)
at(38)
-- The last frame's batched events.
e.setFrameBatching(false)
