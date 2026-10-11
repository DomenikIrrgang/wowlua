-- wowlua runtime: runtime type checks (spec §7.5).
local check = {}
wl.check = check
local function got(value)
	if issecretvalue and issecretvalue(value) then
		return "secret"
	end
	return type(value)
end
-- A parameter failed its check.
function check.fail(index, name, fname, location, expected, value)
	error(
		addonName .. ": " .. location .. ": bad argument #" .. index .. " '" .. name .. "' to '" .. fname .. "' ("
			.. expected .. " expected, got " .. got(value) .. ")",
		0
	)
end
-- A `::` assertion failed.
function check.castFail(expected, value, location)
	error(addonName .. ": " .. location .. ": bad value for '::' (" .. expected .. " expected, got " .. got(value) .. ")", 0)
end
