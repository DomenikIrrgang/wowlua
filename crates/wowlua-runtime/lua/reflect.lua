-- wowlua runtime: reflection metadata (spec §7.8). A WEAK_KEYS-keyed registry,
-- so class tables contain only their members.
local WEAK_KEYS = {__mode = "k"}
local registry = {
classes = setmetatable({}, WEAK_KEYS),
byName = {},
functions = setmetatable({}, WEAK_KEYS),
enums = setmetatable({}, WEAK_KEYS)
}
wl.reflect = registry
function wl.reflectClass(cls, meta)
	registry.classes[cls] = meta
	registry.byName[meta.name] = cls
end
function wl.reflectFn(fn, meta)
	if type(fn) == "function" then
		registry.functions[fn] = meta
	end
end
function wl.reflectEnum(e, order)
	registry.enums[e] = order
end
