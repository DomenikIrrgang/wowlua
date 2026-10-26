-- wowlua runtime: monitor (spec §7.9). The compiler routes this addon's
-- SetScript, HookScript and C_Timer calls here; nothing global is hooked.
-- In dev builds a handler that runs first is the root of the work it
-- starts (`wl.perf`, spec §7.3.2).
local mon = {
events = {},
scripts = {},
timers = {},
commands = {},
memory = {},
memoryAt = {},
-- The frame of each script entry, while it exists.
objects = setmetatable({}, {__mode = "v"})
}
local clock = debugprofilestop
local perf = wl.perf

--- A frame, animation or other script object (what compiled code passes).



--- Frames have names; animations do not.



local function entry(bucket, key, detail)
	local s = bucket[key]
	if not s then
		s = {count = 0, total = 0, max = 0, detail = detail}
		bucket[key] = s
	end
	return s
end
local function record(s, short, key, elapsed)
	s.count = s.count + 1
	s.total = s.total + elapsed
	if elapsed > s.max then
		s.max = elapsed
	end
	if perf then
		perf.maxOpen(short, key, elapsed)
	end
end
-- In dev builds: how often an event comes within one frame (bursts), and
-- in how many frames a script runs.
local function perFrame(s, key, event)
	local frame = perf.frame
	if s.lastFrame ~= frame then
		s.lastFrame = frame
		s.frames = (s.frames or 0) + 1
		s.inFrame = 0
	end
	if event then
		local n = (s.inFrame or 0) + 1
		s.inFrame = n
		if n == 5 then
			s.bursts = (s.bursts or 0) + 1
		end
		if n > (s.burstMax or 0) then
			s.burstMax = n
		end
		perf.maxOpen("burst", key, n)
	end
end
local function timed(s, short, key, start, top, ...)
	local elapsed = clock() - start
	record(s, short, key, elapsed)
	if perf then
		perf.finish(top, key, ({ev = "event", sc = "script", tm = "timer", cm = "slash"})[short], s.detail, elapsed, s)
	end
	return...
end
-- A frame's label: its name, its wowlua class, or where it was set up.
local function label(frame, location)
	if type(frame) == "table" and frame.GetName then
		local name = frame:GetName()
		if type(name) == "string" and not (issecretvalue and issecretvalue(name)) then
			return name
		end
	end
	local classOf = wl.classOf
	local cls = classOf and classOf(frame)
	if cls then
		return cls.__name
	end
	return "frame at " .. location
end
local function wrap(frame, script, fn, location)
	if type(fn) ~= "function" then
		return fn
	end
	if script == "OnEvent" then
		return function(self, event, ...)
			local key = tostring(event)
			local s = entry(mon.events, key, location)
			local top = false
			if perf then
				perFrame(s, key, true)
				top = perf.begin(key, "event")
			end
			return timed(s, "ev", key, clock(), top, fn(self, event, ...))
		end
	end
	local key = label(frame, location) .. " · " .. script
	local update = script == "OnUpdate"
	return function(...)
		local s = entry(mon.scripts, key, location)
		s.script = script
		local top = false
		if perf then
			if update then
				mon.objects[key] = frame
			end
			perFrame(s, key, false)
			top = perf.begin(key, "script")
		end
		return timed(s, "sc", key, clock(), top, fn(...))
	end
end
function mon.setScript(frame, script, fn, location)
	return frame:SetScript(script, wrap(frame, script, fn, location))
end
function mon.hookScript(frame, script, fn, location)
	return frame:HookScript(script, wrap(frame, script, fn, location))
end
local function timer(kind, fn, location)
	if type(fn) ~= "function" then
		return fn
	end
	local key = kind .. " at " .. location
	return function(...)
		local s = entry(mon.timers, key, location)
		local top = perf and perf.begin(key, "timer") or false
		return timed(s, "tm", key, clock(), top, fn(...))
	end
end
-- A slash command's handler (`SlashCmdList[key] = fn`): its label is the
-- command (`/myaddon`), found when it first runs.
function mon.slash(fn, location)
	if type(fn) ~= "function" then
		return fn
	end
	local key = nil
	local wrapper
	wrapper = function(...)
		if not key then
			for k, v in pairs(SlashCmdList) do
				if v == wrapper then
					local command = _G["SLASH_" .. tostring(k) .. "1"]
					key = type(command) == "string" and command or tostring(k)
					break
				end
			end
		end
		local command = key or ("command at " .. location)
		local s = entry(mon.commands, command, location)
		local top = perf and perf.begin(command, "slash") or false
		return timed(s, "cm", command, clock(), top, fn(...))
	end
	return wrapper
end
function mon.after(seconds, fn, location)
	return C_Timer.After(seconds, timer("C_Timer.After", fn, location))
end
function mon.newTimer(seconds, fn, location)
	return C_Timer.NewTimer(seconds, timer("C_Timer.NewTimer", fn, location))
end
function mon.newTicker(seconds, fn, iterations, location)
	return C_Timer.NewTicker(seconds, timer("C_Timer.NewTicker", fn, location), iterations)
end
function mon.reset()
	if perf then
		perf.reset("mon")
	else
		mon.events, mon.scripts, mon.timers = {}, {}, {}
	end
end
wl.mon = mon
-- Memory every 5 seconds outside combat, the last 10 minutes.
if C_Timer and C_Timer.NewTicker and UpdateAddOnMemoryUsage and GetAddOnMemoryUsage then
	C_Timer.NewTicker(5, function()
		if InCombatLockdown and InCombatLockdown() then
			return
		end
		UpdateAddOnMemoryUsage()
		mon.memory[#mon.memory + 1] = GetAddOnMemoryUsage(addonName)
		mon.memoryAt[#mon.memoryAt + 1] = GetTime and GetTime() or 0
		if #mon.memory > 120 then
			table.remove(mon.memory, 1)
			table.remove(mon.memoryAt, 1)
		end
	end)
end
