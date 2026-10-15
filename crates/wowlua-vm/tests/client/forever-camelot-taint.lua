-- WoW Forever 1.60.1 (70374): /stdtest taint experiments (tests/ingame/src/taintlab.wowlua).
-- Each tainted one value on the path a mouse click on action button 1 takes
-- to the protected UseAction; then the click.
return {
    { name = "baseline: nothing tainted (the click must work)", secureBefore = nil, taintBefore = nil, clicked = true, blocked = {  } },
    { name = "a Lua global on the path, reassigned to itself (reading a tainted global)", secureBefore = false, taintBefore = "StdTest", clicked = true, blocked = { "ADDON_ACTION_FORBIDDEN: StdTest, UseAction()" } },
    { name = "a C function's global on the path, reassigned to itself", secureBefore = false, taintBefore = "StdTest", clicked = true, blocked = { "ADDON_ACTION_FORBIDDEN: StdTest, UseAction()" } },
    { name = "the button's own field TriggerSecureClick, reassigned (a tainted table field)", secureBefore = false, taintBefore = "StdTest", clicked = true, blocked = { "ADDON_ACTION_FORBIDDEN: StdTest, UseAction()" } },
    { name = "Settings.GetValue, reassigned (a field of a Blizzard table)", secureBefore = false, taintBefore = "StdTest", clicked = true, blocked = { "ADDON_ACTION_FORBIDDEN: StdTest, UseAction()" } },
    { name = "hooksecurefunc on the Lua global (hooks must not taint)", secureBefore = true, taintBefore = nil, clicked = true, blocked = {  } },
    { name = "hooksecurefunc on the button's field", secureBefore = true, taintBefore = nil, clicked = true, blocked = {  } },
    { name = "setfenv on the Lua global's function, same environment (does it taint the function?)", secureBefore = true, taintBefore = nil, clicked = true, blocked = { "ADDON_ACTION_FORBIDDEN: StdTest, UseAction()" } },
}
