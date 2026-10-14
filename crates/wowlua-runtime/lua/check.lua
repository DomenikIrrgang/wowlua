-- wowlua runtime: runtime type checks (spec §7.5).
local function got(value)
	if issecretvalue and issecretvalue(value) then
		return "secret"
	end
	return type(value)
end
wl.check = {
-- A parameter failed its check.
fail = function(index, name, fname, location, expected, value)
	error(
	addonName .. ": " .. location .. ": bad argument #" .. index .. " '" .. name .. "' to '" .. fname .. "' ("
	.. expected .. " expected, got " .. got(value) .. ")",
	0
	)
end,
-- A `::` assertion failed.
castFail = function(expected, value, location)
	error(tostring(addonName) .. ": " .. tostring(location) .. ": bad value for '::' (" .. tostring(expected) .. " expected, got " .. tostring(got(value)) .. ")", 0)
end
}
