-- wowlua runtime: reflection metadata (spec §7.8). A weak-keyed registry,
-- so class tables contain only their members.
local weak = {__mode = "k"}
local registry = {
classes = setmetatable({}, weak),
byName = {},
functions = setmetatable({}, weak),
enums = setmetatable({}, weak)
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
