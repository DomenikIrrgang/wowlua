-- wowlua runtime: widget (spec §3.2). Widget classes are mixins applied to
-- frames created by CreateFrame; construction is threaded through __ctor.
local SKIP = {
	new = true, __index = true, __name = true, __super = true, __init = true,
	__ctor = true, __initFields = true, __mixin = true,
}
-- The methods of a widget class and its superclasses, flattened.
local function flatten(cls)
	local list, c = {}, cls
	while c do
		list[#list + 1] = c
		c = rawget(c, "__super")
	end
	local m = {}
	for i = #list, 1, -1 do
		for k, v in pairs(list[i]) do
			if type(v) == "function" and not SKIP[k] then
				m[k] = v
			end
		end
	end
	rawset(cls, "__mixin", m)
	return m
end
function wl.widgetClass(name, super)
	local cls = wl.class(name, super)
	cls.new = function(...)
		return cls.__ctor(cls, ...)
	end
	return cls
end
function wl.finalizeWidget(cls)
	wl.finalize(cls)
	flatten(cls)
	return cls
end
-- Applies the most-derived class's methods to a new frame.
function wl.mixin(frame, cls)
	for k, v in pairs(rawget(cls, "__mixin") or flatten(cls)) do
		frame[k] = v
	end
	frame.__class = cls
	return frame
end
-- The mixin exposed for XML templates (`@global` on a widget class): its
-- OnLoad runs the field initializers, root class first, then the class's
-- own OnLoad.
function wl.xmlMixin(cls)
	local m = {}
	for k, v in pairs(rawget(cls, "__mixin") or flatten(cls)) do
		m[k] = v
	end
	local onLoad = m.OnLoad
	m.OnLoad = function(self, ...)
		local list, c = {}, cls
		while c do
			list[#list + 1] = c
			c = rawget(c, "__super")
		end
		for i = #list, 1, -1 do
			local init = rawget(list[i], "__initFields")
			if init then
				init(self)
			end
		end
		self.__class = cls
		if onLoad then
			return onLoad(self, ...)
		end
	end
	return m
end
