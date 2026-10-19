-- /stdtest engine scenario macro, played with the engine's controls: the
-- game menu's Macros, the character's tab, a new macro "Test" (an icon
-- chosen, "/say hi" typed), dragged onto the empty action slot 6, dragged
-- off again and let go, deleted (scenarios/macro.lua). Each step at its
-- recorded time (a click: when the client acted).
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
-- The game menu open before the recording began.
e.pressKey("ESCAPE")
check("the game menu shown", GameMenuFrame:IsShown())
local macros
for _, b in ipairs(GameMenuFrame.buttons or {}) do
    if b:GetText() == MACROS then macros = b end
end
check("its Macros button", macros ~= nil)
at(6.668) press(macros)
check("the macro window shown", MacroFrame and MacroFrame:IsShown())
-- The character's macros: the account macro shown first is saved as the
-- tab changes (its text set as it was shown).
at(7.305) press(MacroFrameTab2)
-- A new one: an icon chosen, named, made.
at(8.073) press(MacroNewButton)
check("the icons shown", MacroPopupFrame:IsShown())
local icon
for b in MacroPopupFrame.IconSelector:EnumerateButtons() do
    if b.GetSelectionIndex and b:GetSelectionIndex() == 48 then icon = b end
end
-- The 48th icon clicked (recorded): the engine lists only the spellbook's
-- icons (the client's macro icons not recorded), the first kept then.
if icon then at(12.174) press(icon) end
local name = MacroPopupFrame.BorderBox.IconSelectorEditBox
at(13.005) press(name)
e.type(name, "Test")
at(13.264) press(MacroPopupFrame.BorderBox.OkayButton)
check("made", GetMacroInfo(121) == "Test")
-- Its text typed.
at(13.748) press(MacroFrameText)
e.type(MacroFrameText, "/say hi")
-- Dragged onto slot 6.
local first
for b in MacroFrame.MacroSelector:EnumerateButtons() do
    if b:GetSelectionIndex() == 1 then first = b end
end
check("its button", first ~= nil)
at(19.280) script(first, "OnDragStart", "LeftButton")
at(19.906) script(first, "OnDragStop") script(ActionButton6, "OnReceiveDrag")
check("on slot 6", select(1, GetActionInfo(6)) == "macro")
-- Shift held (the bars locked), dragged off, let go with a right-click.
at(24.232) e.setModifiers({ shift = true })
at(25.213) script(ActionButton6, "OnDragStart", "LeftButton")
at(25.378) script(ActionButton6, "OnDragStop")
at(25.464) e.setModifiers({})
at(25.571) ClearCursor()
check("slot 6 empty", not HasAction(6))
-- Deleted.
at(26.965) press(MacroDeleteButton)
check("asked", StaticPopup1:IsShown())
at(27.910) press(StaticPopup1Button1)
check("deleted", GetMacroInfo(121) == nil)
at(32)
