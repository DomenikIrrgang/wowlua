-- wowlua runtime: enum (spec §3.4.3). Debug builds: reading an unknown
-- member or assigning to an enum table raises an error.
function wl.enum(t, name)
	return setmetatable(t, {
	__index = function(_, k)
		error("enum " .. tostring(name) .. " has no member '" .. tostring(k) .. "'", 2)
	end,
	__newindex = function()
		error("enum " .. tostring(name) .. " cannot be changed", 2)
	end
	})
end
