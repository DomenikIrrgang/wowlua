-- wowlua runtime: dev console (spec §7.7). This addon's dev data, the
-- shared `WOWLUA_DEV` registry, and — from the newest runtime loaded — the
-- `/wl` command and the dev window.
local dev = { name = addonName, info = wl.devInfo or {}, watches = {}, schema = 1 }
wl.dev = dev
-- A watch from code (`debug.watch`): `fn()` shown live under `label`.
function dev.watch(label, fn)
	dev.watches[#dev.watches + 1] = { label = label, fn = fn, source = "code" }
end
-- The data the console reads, through one versioned interface.
dev.runtime = wl
local reg = WOWLUA_DEV
if type(reg) ~= "table" then
	reg = { addons = {}, order = {} }
	WOWLUA_DEV = reg
end
if not reg.addons[addonName] then
	reg.order[#reg.order + 1] = addonName
end
reg.addons[addonName] = dev

local VERSION = (wl.devInfo and wl.devInfo.runtime) or 0

-- This runtime's console replaces older ones.
local function install()
local console = {}
reg.version = VERSION
reg.console = console

local function say(text)
	local frame = DEFAULT_CHAT_FRAME
	local line = "|cff33ff99wowlua|r " .. text
	if frame then
		frame:AddMessage(line)
	else
		print(line)
	end
end
console.say = say

-- The addon a command names: exact, then by prefix, case-insensitive; the
-- only loaded one when none is named.
function console.find(name)
	if not name or name == "" then
		if #reg.order == 1 then
			return reg.addons[reg.order[1]]
		end
		return nil
	end
	local lower = name:lower()
	for _, n in ipairs(reg.order) do
		if n:lower() == lower then
			return reg.addons[n]
		end
	end
	for _, n in ipairs(reg.order) do
		if n:lower():sub(1, #lower) == lower then
			return reg.addons[n]
		end
	end
	return nil
end

local function pad(s, n, right)
	s = tostring(s)
	if #s >= n then
		return s
	end
	local fill = string.rep(" ", n - #s)
	return right and (fill .. s) or (s .. fill)
end
local function ms(v)
	return string.format(v >= 10 and "%.1f" or "%.2f", v)
end

-- Rows of a statistics table, sorted by `key` (descending).
function console.sorted(stats, key)
	local rows = {}
	for name, s in pairs(stats or {}) do
		rows[#rows + 1] = { name = name, s = s }
	end
	table.sort(rows, function(a, b)
		return (a.s[key] or 0) > (b.s[key] or 0)
	end)
	return rows
end

local function statTable(stats, sortKey, top, countHead)
	local rows = console.sorted(stats, sortKey)
	say(pad("name", 34) .. pad(countHead, 8, true) .. pad("total", 9, true) .. pad("self", 9, true) .. pad("max", 9, true) .. pad("avg", 9, true))
	for i = 1, math.min(top, #rows) do
		local r = rows[i]
		local s = r.s
		say(pad(r.name, 34) .. pad(s.calls or s.count or 0, 8, true) .. pad(ms(s.total or 0), 9, true)
			.. pad(ms(s.self or s.total or 0), 9, true) .. pad(ms(s.max or 0), 9, true)
			.. pad(ms((s.total or 0) / math.max(s.calls or s.count or 1, 1)), 9, true))
	end
	if #rows == 0 then
		say("(no data yet)")
	end
end

local function words(msg)
	local out = {}
	for w in (msg or ""):gmatch("%S+") do
		out[#out + 1] = w
	end
	return out
end

local commands = {}
console.commands = commands

function commands.list()
	for _, n in ipairs(reg.order) do
		local i = reg.addons[n].info
		say(n .. " " .. (i.version or "?") .. " · " .. (i.profile or "?") .. " · build " .. (i.build or "?"))
	end
end

function commands.prof(args)
	local d = console.find(args[1])
	if not d then
		return say("which addon? /wl prof <addon> [top N] [sort calls|total|self|max] [reset]")
	end
	local prof = d.runtime.prof
	if not prof then
		return say(d.name .. ": the profiler is off in this build")
	end
	local top, sortKey = 20, "total"
	local i = 2
	while args[i] do
		local w = args[i]:lower()
		if w == "reset" then
			prof.reset()
			return say(d.name .. ": profiler statistics reset")
		elseif w == "top" then
			top = tonumber(args[i + 1]) or top
			i = i + 1
		elseif w == "sort" then
			sortKey = (args[i + 1] or "total"):lower()
			i = i + 1
		end
		i = i + 1
	end
	say(d.name .. " profiler (ms), by " .. sortKey .. ":")
	statTable(prof.stats, sortKey, top, "calls")
end

function commands.mon(args)
	local d = console.find(args[1])
	if not d then
		return say("which addon? /wl mon <addon> [reset]")
	end
	local mon = d.runtime.mon
	if not mon then
		return say(d.name .. ": the monitor is off in this build")
	end
	if (args[2] or ""):lower() == "reset" then
		mon.reset()
		return say(d.name .. ": monitor reset")
	end
	for _, kind in ipairs({ "events", "scripts", "timers" }) do
		say(d.name .. " " .. kind .. ":")
		statTable(mon[kind], "total", 10, kind == "events" and "count" or "runs")
	end
	local mem = mon.memory[#mon.memory]
	if mem then
		say(string.format("memory %.2f MB", mem / 1024))
	end
end

function commands.errors(args)
	local d = console.find(args[1])
	if not d then
		return say("which addon? /wl errors <addon> [clear]")
	end
	local errors = d.runtime.errors
	if not errors then
		return say(d.name .. ": error capture is off in this build")
	end
	if (args[2] or ""):lower() == "clear" then
		errors.clear()
		return say(d.name .. ": errors cleared")
	end
	if #errors.list == 0 then
		return say(d.name .. ": no errors")
	end
	for _, e in ipairs(errors.list) do
		say("|cffff9a80" .. e.message .. "|r ×" .. e.count .. " · " .. e.time)
	end
end

function commands.log(args)
	local d = console.find(args[1])
	if not d then
		return say("which addon? /wl log <addon> [level]")
	end
	local logs = d.runtime.logs
	if not logs then
		return say(d.name .. ": this addon does not use wowlua:log")
	end
	if args[2] then
		logs.setLevel(args[2]:lower())
		return say(d.name .. ": log level " .. args[2]:lower())
	end
	for _, e in ipairs(logs.history(20)) do
		say(string.format("%.1f", e.time) .. " [" .. e.level .. "] " .. e.message)
	end
end

function commands.meta(args)
	local d = console.find(args[1])
	if not d then
		return say("which addon? /wl meta <addon>")
	end
	for k, v in pairs(d.info) do
		say(d.name .. " " .. k .. " = " .. tostring(v))
	end
	local meta = d.runtime.meta
	for kind, t in pairs(meta or {}) do
		if type(t) == "table" then
			for k, v in pairs(t) do
				if type(v) ~= "table" then
					say(d.name .. " " .. kind .. "." .. k .. " = " .. tostring(v))
				end
			end
		end
	end
end

function commands.inspect(args)
	local d = console.find(args[1])
	if not d or not args[2] then
		return say("/wl inspect <addon> <module>")
	end
	local exports = d.runtime.require(args[2], true)
	local names = {}
	for k in pairs(exports or {}) do
		names[#names + 1] = tostring(k)
	end
	table.sort(names)
	say(d.name .. " " .. args[2] .. ": " .. (#names > 0 and table.concat(names, ", ") or "(no exports)"))
end

-- A typed watch: a Lua expression over `ns`, `modules` and globals.
function console.compile(d, expression)
	local chunk, problem = loadstring("return " .. expression, "=watch")
	if not chunk then
		return nil, problem
	end
	local env = setmetatable({ ns = d.runtime.ns, modules = d.runtime.modules }, { __index = _G })
	setfenv(chunk, env)
	return chunk
end

function console.show(v)
	if issecretvalue and issecretvalue(v) then
		return "<secret>"
	end
	if type(v) == "string" then
		return string.format("%q", v)
	end
	return tostring(v)
end

function commands.watch(args)
	local d = console.find(args[1])
	local expression = table.concat(args, " ", 2)
	if not d or expression == "" then
		return say("/wl watch <addon> <expression>")
	end
	local fn, problem = console.compile(d, expression)
	if not fn then
		return say("|cffff9a80" .. tostring(problem) .. "|r")
	end
	d.watches[#d.watches + 1] = { label = expression, fn = fn, source = "typed" }
	local ok, value = pcall(fn)
	say(d.name .. " watch " .. expression .. " = " .. (ok and console.show(value) or ("|cffff9a80" .. tostring(value) .. "|r")))
end

function commands.reload()
	ReloadUI()
end

function console.command(msg)
	local args = words(msg)
	local name = table.remove(args, 1)
	if not name then
		if console.toggle then
			return console.toggle()
		end
		return commands.list()
	end
	local f = commands[name:lower()]
	if f then
		return f(args)
	end
	say("/wl [list | prof | mon | errors | log | meta | inspect | watch | reload]")
end

if type(SlashCmdList) == "table" then
	SLASH_WOWLUA1 = "/wl"
	SlashCmdList.WOWLUA = function(msg)
		reg.console.command(msg)
	end
end
-- `/wl`: the dev window, built when first opened.
if type(CreateFrame) == "function" then
	function console.toggle()
		if not console.window then
			console.window = buildDevWindow(console, reg)
		end
		local w = console.window
		if w:IsShown() then
			w:Hide()
		else
			w:Show()
			console.refresh()
		end
	end
end
dev.console = console
end
if (reg.version or -1) < VERSION then
	install()
end
