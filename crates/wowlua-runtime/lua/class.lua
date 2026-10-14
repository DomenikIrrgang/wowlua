-- wowlua runtime: class (spec §3.1). Class tables, finalization (inherited
-- metamethods, property dispatch), abstract stubs and class identity.
local METAMETHODS = {
"__tostring", "__eq", "__lt", "__le", "__add", "__sub", "__mul", "__div",
"__mod", "__pow", "__unm", "__concat", "__call"
}
local ACCESSORS = {get = "__getters", set = "__setters", sget = "__sgetters", sset = "__ssetters"}
local function noop() end
local function isClass(v)
	return type(v) == "table" and rawget(v, "__name") ~= nil and rawget(v, "new") ~= nil
end
-- Creates the table of class `name` (spec §3.1.9).
function wl.class(name, super)
	if super ~= nil and not isClass(super) then
		error(addonName .. ": class " .. name .. " extends a value that is not a class", 2)
	end
	local cls = {__name = name, __super = super}
	cls.__index = cls
	cls.new = function(...)
		local self = setmetatable({}, cls)
		cls.__init(self, ...)
		return self
	end
	if super == nil then
		cls.__init = noop
	end
	setmetatable(cls, {
	__index = super,
	__call = function(c, ...)
		return c.new(...)
	end
	})
	return cls
end
-- Registers an accessor: kind is "get", "set", "sget" or "sset" (static).
function wl.accessor(cls, kind, name, fn)
	local key = ACCESSORS[kind]
	local t = rawget(cls, key)
	if not t then
		t = {}
		rawset(cls, key, t)
	end
	t[name] = fn
end
-- The accessors of `key` along the superclass chain, the class's own last.
local function merged(cls, key)
	local list, c = {}, cls
	while c do
		list[#list + 1] = rawget(c, key)
		c = rawget(c, "__super")
	end
	if #list == 0 then
		return nil
	end
	local out = {}
	for i = #list, 1, -1 do
		for k, v in pairs(list[i]) do
			out[k] = v
		end
	end
	return out
end
local function readOnly(cls, k)
	error("property '" .. tostring(k) .. "' of " .. cls.__name .. " is read-only", 3)
end
-- Completes a class after its members are defined.
function wl.finalize(cls)
	local super = rawget(cls, "__super")
	if super then
		for _, k in ipairs(METAMETHODS) do
			if rawget(cls, k) == nil and super[k] ~= nil then
				rawset(cls, k, super[k])
			end
		end
	end
	local getters, setters = merged(cls, "__getters"), merged(cls, "__setters")
	if getters or setters then
		getters, setters = getters or {}, setters or {}
		cls.__index = function(self, k)
			local g = getters[k]
			if g then
				return g(self)
			end
			return cls[k]
		end
		cls.__newindex = function(self, k, v)
			local s = setters[k]
			if s then
				s(self, v)
			elseif getters[k] then
				readOnly(cls, k)
			else
				rawset(self, k, v)
			end
		end
	end
	local sgetters, ssetters = merged(cls, "__sgetters"), merged(cls, "__ssetters")
	if sgetters or ssetters then
		sgetters, ssetters = sgetters or {}, ssetters or {}
		local mt = getmetatable(cls)
		mt.__index = function(_, k)
			local g = sgetters[k]
			if g then
				return g()
			end
			if super then
				return super[k]
			end
		end
		mt.__newindex = function(c, k, v)
			local s = ssetters[k]
			if s then
				s(v)
			elseif sgetters[k] then
				readOnly(cls, k)
			else
				rawset(c, k, v)
			end
		end
	end
	return cls
end
-- Debug builds: an abstract method raises an error when called.
function wl.abstract(cls, name)
	rawset(cls, name, function()
		error("abstract method " .. cls.__name .. ":" .. name .. " is not implemented", 2)
	end)
end
-- The class of a value (widget instances carry `__class`).
function wl.classOf(v)
	if type(v) ~= "table" then
		return nil
	end
	local c = rawget(v, "__class")
	if c then
		return c
	end
	local mt = getmetatable(v)
	if isClass(mt) then
		return mt
	end
	return nil
end
function wl.isInstance(v, cls)
	local c = wl.classOf(v)
	while c do
		if c == cls then
			return true
		end
		c = rawget(c, "__super")
	end
	return false
end
