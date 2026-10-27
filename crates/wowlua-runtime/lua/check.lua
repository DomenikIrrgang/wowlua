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
-- A test's `mock.partial` fake: it passes a record's check without the
-- fields the test left out.
partial = function(value)
	local mt = getmetatable(value)
	return type(mt) == "table" and rawget(mt, "__wowluaPartial") == true
end,
-- A `::` assertion failed.
castFail = function(expected, value, location)
	error(addonName .. ": " .. location .. ": bad value for '::' (" .. expected .. " expected, got " .. got(value) .. ")", 0)
end
}
