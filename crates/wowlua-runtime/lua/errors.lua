-- wowlua runtime: error capture (spec §7.10). Errors raised by this
-- addon's code are recorded; the previous handler always runs as well.
local errors = {
list = {},
byMessage = {}
}
local FOLDER = ("addons/" .. addonName .. "/"):lower()
local function normal(s)
	return tostring(s or ""):gsub("\\", "/"):lower()
end
-- The runtime's own errors start with the addon's name; otherwise the
-- message's location decides (`path:line: text`), and without a path in
-- it, the stack.
local PREFIX = (addonName .. ": "):lower()
local function mine(message, stack)
	local m = normal(message)
	if m:sub(1, #PREFIX) == PREFIX then
		return true
	end
	local location = m:match("^(.-):%d+:")
	if location and location:find("/", 1, true) then
		return location:find(FOLDER, 1, true) ~= nil
	end
	return normal(stack):find(FOLDER, 1, true) ~= nil
end
-- `Interface/AddOns/<Name>/src/a.lua:12` → `src/a.lua:12`.
local function shorten(s)
	s = tostring(s or "")
	local i, j = normal(s):find(FOLDER, 1, true)
	if i and j then
		return s:sub(1, i - 1):gsub("[Ii]nterface[/\\]$", "") .. s:sub(j + 1)
	end
	return s
end
function errors.record(message, stack)
	message = tostring(message)
	if not mine(message, stack) then
		return
	end
	local short = shorten(message)
	local e = errors.byMessage[short]
	if e then
		e.count = e.count + 1
		e.time = date and date("%H:%M:%S") or e.time
		return
	end
	e = {
	message = short,
	stack = shorten(stack),
	count = 1,
	time = date and date("%H:%M:%S") or ""
	}
	errors.byMessage[short] = e
	table.insert(errors.list, 1, e)
	-- The client hides Lua errors unless "Display Lua errors" is on.
	local chat = DEFAULT_CHAT_FRAME
	if chat then
		chat:AddMessage("|cff33ff99" .. addonName .. "|r |cffff9a80error captured:|r " .. short
		.. " |cffa39b8b(/wl errors " .. addonName .. ")|r")
	end
	-- Dev builds: the console's toast (spec §7.10).
	local reg = WOWLUA_DEV
	local console = reg and reg.console
	if console and console.errorCaptured then
		console.errorCaptured(addonName, e)
	end
	if #errors.list > 50 then
		local old = table.remove(errors.list)
		if old then
			errors.byMessage[old.message] = nil
		end
	end
end
function errors.clear()
	errors.list = {}
	errors.byMessage = {}
end
wl.errors = errors
-- BugGrabber keeps the error handler to itself: listen to it instead.
local grabber = _G.BugGrabber
if type(grabber) == "table" and type(grabber.RegisterCallback) == "function" then
	grabber.RegisterCallback(errors, "BugGrabber_BugGrabbed", function(_, e)
		errors.record(e.message, e.stack)
	end)
elseif type(seterrorhandler) == "function" and type(geterrorhandler) == "function" then
	local previous = geterrorhandler()
	seterrorhandler(function(message, ...)
		errors.record(message, debugstack and debugstack(2) or "")
		if previous then
			return previous(message, ...)
		end
	end)
end
