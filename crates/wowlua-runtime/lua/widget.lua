-- wowlua runtime: widget (spec §3.2). Widget classes are mixins applied to
-- frames created by CreateFrame; construction is threaded through __ctor.
local SKIP = {
new = true, __index = true, __name = true, __super = true, __init = true,
__ctor = true, __initFields = true, __mixin = true
}
-- The `@script` handlers of a widget class and its superclasses: a handler
-- set with SetScript replaces one for the same script from a superclass;
-- hooks add up.
local function scriptList(cls)
	local chain, c = {}, cls
	while c do
		chain[#chain + 1] = c
		c = rawget(c, "__super")
	end
	local list = {}
	for i = #chain, 1, -1 do
		for _, entry in ipairs(rawget(chain[i], "__scripts") or {}) do
			local replaced = false
			if not entry[3] then
				for j, other in ipairs(list) do
					if other[1] == entry[1] and not other[3] then
						list[j] = entry
						replaced = true
						break
					end
				end
			end
			if not replaced then
				list[#list + 1] = entry
			end
		end
	end
	rawset(cls, "__scriptList", list)
	return list
end
-- Installs the `@script` handlers on a frame (after its constructor, or
-- before the class's OnLoad for frames created from XML).
local function installScripts(frame, cls)
	-- Dev builds: through the monitor (spec §7.9).
	local mon = wl.mon
	for _, entry in ipairs(rawget(cls, "__scriptList") or scriptList(cls)) do
		local handler = frame[entry[2]]
		local where = tostring(rawget(cls, "__name")) .. ":" .. entry[2]
		if entry[3] then
			if mon then
				mon.hookScript(frame, entry[1], handler, where)
			else
				frame:HookScript(entry[1], handler)
			end
		elseif mon then
			mon.setScript(frame, entry[1], handler, where)
		else
			frame:SetScript(entry[1], handler)
		end
	end
end
-- Registers an `@script` method: script name, method name, hook.
function wl.script(cls, script, method, hook)
	local list = rawget(cls, "__scripts")
	if not list then
		list = {}
		rawset(cls, "__scripts", list)
	end
	list[#list + 1] = {script, method, hook or false}
end
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
-- The class slice (required by this one) comes first.
local class, finalize = assert(wl.class), assert(wl.finalize)
function wl.widgetClass(name, super)
	local cls = class(name, super)
	cls.new = function(...)
		local self = cls.__ctor(cls, ...)
		installScripts(self, cls)
		return self
	end
	return cls
end
function wl.finalizeWidget(cls)
	finalize(cls)
	flatten(cls)
	scriptList(cls)
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
		installScripts(self, cls)
		if onLoad then
			return onLoad(self, ...)
		end
	end
	return m
end
