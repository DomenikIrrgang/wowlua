-- wowlua runtime: monitor (spec §7.9). The compiler routes this addon's
-- SetScript, HookScript and C_Timer calls here; nothing global is hooked.
local mon = {
events = {},
scripts = {},
timers = {},
memory = {}
}
local clock = debugprofilestop or function()
	return os.clock() * 1000
end
--- A frame, animation or other script object (what compiled code passes).



--- Frames have names; animations do not.



local function record(bucket, key, detail, elapsed)
	local s = bucket[key]
	if not s then
		s = {count = 0, total = 0, max = 0, detail = detail}
		bucket[key] = s
	end
	s.count = s.count + 1
	s.total = s.total + elapsed
	if elapsed > s.max then
		s.max = elapsed
	end
end
local function timed(bucket, key, detail, start, ...)
	record(bucket, key, detail, clock() - start)
	return...
end
-- A frame's label: its name, its wowlua class, or where it was set up.
local function label(frame, location)
	if type(frame) == "table" and frame.GetName then
		local name = frame:GetName()
		if name then
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
			return timed(mon.events, tostring(event), location, clock(), fn(self, event, ...))
		end
	end
	local key = label(frame, location) .. " · " .. script
	return function(...)
		return timed(mon.scripts, key, location, clock(), fn(...))
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
		return timed(mon.timers, key, location, clock(), fn(...))
	end
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
	mon.events, mon.scripts, mon.timers = {}, {}, {}
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
		if #mon.memory > 120 then
			table.remove(mon.memory, 1)
		end
	end)
end
