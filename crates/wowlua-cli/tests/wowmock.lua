-- A small model of the WoW client for the standard library's tests: frames
-- and events (with unit filters), scripts and hooks, error handlers,
-- C_Timer on a controlled clock, combat state, addon loading and slash
-- commands. Test scripts drive it through `Mock`.

Mock = { now = 0, frames = {}, timers = {}, errors = {}, out = {}, loaded = {},
         combat = false, loggedIn = false }

-- The client's events (anything else is unknown, like an addon's messages).
Mock.events = {}
for _, e in ipairs({ "ADDON_LOADED", "PLAYER_LOGIN", "PLAYER_LOGOUT", "PLAYER_ENTERING_WORLD",
    "PLAYER_REGEN_DISABLED", "PLAYER_REGEN_ENABLED", "PLAYER_STARTED_MOVING", "BAG_UPDATE",
    "BAG_UPDATE_DELAYED", "UNIT_HEALTH", "UNIT_POWER_UPDATE", "PLAYER_TARGET_CHANGED" }) do
    Mock.events[e] = true
end

function print(...)
    local parts = {}
    for i = 1, select("#", ...) do parts[i] = tostring((select(i, ...))) end
    Mock.out[#Mock.out + 1] = table.concat(parts, " ")
end

-- Error handling ------------------------------------------------------------

local handler = function(msg)
    Mock.errors[#Mock.errors + 1] = tostring(msg)
    print("ERROR: " .. tostring(msg))
end
function geterrorhandler() return handler end
function seterrorhandler(fn) handler = fn end

function securecallfunction(fn, ...)
    local n, args = select("#", ...), { ... }
    local results = { xpcall(function() return fn(unpack(args, 1, n)) end, function(e) geterrorhandler()(e) return e end) }
    if results[1] then return unpack(results, 2) end
end

function debugstack() return "" end
function GetTime() return Mock.now end
function InCombatLockdown() return Mock.combat end
function IsLoggedIn() return Mock.loggedIn end
function issecretvalue() return false end

DEFAULT_CHAT_FRAME = { AddMessage = function(_, msg) print(msg) end }

-- Frames --------------------------------------------------------------------

local Frame = {}
Frame.__index = function(self, key)
    local method = Frame[key]
    if method then return method end
    -- Layout and drawing methods do nothing.
    return function() end
end

function Frame:RegisterEvent(event)
    if not Mock.events[event] then error("Attempt to register unknown event \"" .. event .. "\"", 2) end
    self.events[event] = true
    self.units[event] = nil
end
function Frame:RegisterUnitEvent(event, u1, u2)
    if not Mock.events[event] then error("Attempt to register unknown event \"" .. event .. "\"", 2) end
    self.events[event] = true
    self.units[event] = { [u1] = true }
    if u2 then self.units[event][u2] = true end
end
function Frame:UnregisterEvent(event) self.events[event] = nil; self.units[event] = nil end
function Frame:UnregisterAllEvents() self.events = {}; self.units = {} end
function Frame:IsEventRegistered(event) return self.events[event] == true end
function Frame:SetScript(name, fn) self.scripts[name] = fn; self.hooks[name] = nil end
function Frame:GetScript(name) return self.scripts[name] end
function Frame:HookScript(name, fn)
    self.hooks[name] = self.hooks[name] or {}
    table.insert(self.hooks[name], fn)
end
function Frame:Show() self.shown = true; Mock.script(self, "OnShow") end
function Frame:Hide() self.shown = false; Mock.script(self, "OnHide") end
function Frame:IsShown() return self.shown end
function Frame:GetName() return self.name end

function CreateFrame(kind, name, parent, template)
    local f = setmetatable({ kind = kind, name = name, events = {}, units = {}, scripts = {}, hooks = {}, shown = true }, Frame)
    Mock.frames[#Mock.frames + 1] = f
    if name then _G[name] = f end
    return f
end

function Mock.script(frame, name, ...)
    local fn = frame.scripts[name]
    if fn then fn(frame, ...) end
    for _, h in ipairs(frame.hooks[name] or {}) do h(frame, ...) end
end

-- Fires a WoW event at every frame registered for it, in creation order.
function Mock.fire(event, ...)
    if event == "PLAYER_REGEN_DISABLED" then Mock.combat = true end
    if event == "PLAYER_REGEN_ENABLED" then Mock.combat = false end
    if event == "PLAYER_LOGIN" then Mock.loggedIn = true end
    local unit = ...
    for _, f in ipairs({ unpack(Mock.frames) }) do
        local units = f.units[event]
        if f.events[event] and (not units or units[unit]) then
            Mock.script(f, "OnEvent", event, ...)
        end
    end
end

-- Loads an addon (fires ADDON_LOADED for it).
function Mock.loadAddon(name)
    Mock.loaded[name] = true
    Mock.fire("ADDON_LOADED", name, false)
end

C_AddOns = { IsAddOnLoaded = function(name) return Mock.loaded[name] == true end }
C_EventUtils = { IsEventValid = function(event) return Mock.events[event] == true end }

-- Hooks ---------------------------------------------------------------------

function hooksecurefunc(target, name, fn)
    if type(target) == "string" then target, name, fn = _G, target, name end
    local original = target[name]
    assert(type(original) == "function", "hooksecurefunc: " .. tostring(name) .. " is not a function")
    target[name] = function(...)
        local r = { original(...) }
        fn(...)
        return unpack(r)
    end
end

-- Timers --------------------------------------------------------------------

local Timer = {}
Timer.__index = Timer
function Timer:Cancel() self.cancelled = true end
function Timer:IsCancelled() return self.cancelled == true end

local function schedule(sec, fn, iterations)
    local t = setmetatable({ due = Mock.now + sec, every = sec, fn = fn, left = iterations, cancelled = false }, Timer)
    Mock.timers[#Mock.timers + 1] = t
    return t
end

C_Timer = {
    After = function(sec, fn) schedule(sec, fn, 1) end,
    NewTimer = function(sec, fn) return schedule(sec, fn, 1) end,
    NewTicker = function(sec, fn, iterations) return schedule(sec, fn, iterations or math.huge) end,
}

-- Advances the clock, running due timers in order (a frame is 0 seconds).
function Mock.advance(sec)
    local target = Mock.now + (sec or 0)
    while true do
        local next
        for _, t in ipairs(Mock.timers) do
            if not t.cancelled and t.left > 0 and t.due <= target and (not next or t.due < next.due) then next = t end
        end
        if not next then break end
        Mock.now = math.max(Mock.now, next.due)
        next.left = next.left - 1
        next.due = next.due + next.every
        next.fn(next)
    end
    Mock.now = target
end

-- Slash commands ------------------------------------------------------------

SlashCmdList = {}

-- Runs "/cmd args" like the chat box.
function Mock.slash(line)
    local cmd, msg = line:match("^(%S+)%s*(.-)$")
    for key, fn in pairs(SlashCmdList) do
        local i = 1
        while _G["SLASH_" .. key .. i] do
            if _G["SLASH_" .. key .. i]:lower() == cmd:lower() then return fn(msg, {}) end
            i = i + 1
        end
    end
    print("unknown command " .. cmd)
end

-- A few of Blizzard's globals for hooks.
function ToggleGameMenu() print("menu") end
GameTooltip = CreateFrame("GameTooltip", "GameTooltip")
function GameTooltip:SetUnit(unit) print("SetUnit " .. unit) end
CreateFrame("Frame", "PlayerFrame")
