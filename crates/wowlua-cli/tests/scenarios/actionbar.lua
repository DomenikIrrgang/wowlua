-- /stdtest engine scenario actionbar, played with the engine's controls:
-- the paladin's spellbook opened, its Protection tab, Blessing of Freedom
-- dragged onto the empty action slot 5, that button dragged onto slot 4
-- (Hammer of Justice onto the cursor) and it put on slot 5, then the same
-- shift-dragged back (scenarios/actionbar.lua). Each step at its recorded
-- time (a click: when the client acted).
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
at(0.316) e.pressKey("P")
local book = PlayerSpellsFrame and PlayerSpellsFrame.SpellBookFrame
check("the spellbook shown", book and book:IsVisible())
-- The Protection tab (recorded: its tabID 3).
at(4.989) press(book:GetTabButton(3))
local freedom
book:ForEachDisplayedSpell(function(f)
    if f.spellBookItemInfo and f.spellBookItemInfo.spellID == 1044 then freedom = f end
end)
check("Blessing of Freedom shown", freedom ~= nil)
-- The durability's update the server sent (recorded).
at(5.165) e.fireEvent("UPDATE_INVENTORY_DURABILITY")
-- Dragged onto slot 5.
at(5.924) script(freedom.Button, "OnDragStart", "LeftButton")
at(6.888) script(freedom.Button, "OnDragStop") script(ActionButton5, "OnReceiveDrag")
check("freedom on slot 5", select(2, GetActionInfo(5)) == 1044)
-- Shift held (the bars locked), slot 5's button onto slot 4: Hammer of
-- Justice on the cursor, then put on slot 5.
at(10.410) e.setModifiers({ shift = true })
at(11.459) script(ActionButton5, "OnDragStart", "LeftButton")
at(11.756) script(ActionButton5, "OnDragStop") script(ActionButton4, "OnReceiveDrag")
at(11.973) e.setModifiers({})
at(12.651) script(ActionButton5, "OnReceiveDrag")
check("swapped", select(2, GetActionInfo(4)) == 1044 and select(2, GetActionInfo(5)) == 20288)
-- Shift held, the same back.
at(13.912) e.setModifiers({ shift = true })
at(14.164) script(ActionButton5, "OnDragStart", "LeftButton")
at(14.311) script(ActionButton5, "OnDragStop") script(ActionButton4, "OnReceiveDrag")
at(14.330) e.setModifiers({})
at(14.966) script(ActionButton5, "OnReceiveDrag")
check("back", select(2, GetActionInfo(4)) == 20288 and select(2, GetActionInfo(5)) == 1044)
at(21)
