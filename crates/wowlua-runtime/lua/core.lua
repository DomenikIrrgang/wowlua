-- wowlua runtime: core (MIT License). Generated into every build that uses
-- modules or build metadata. Stored in the addon table, never in globals.
local addonName, ns = ...
local wl = ns.__wowlua or {}
ns.__wowlua = wl
local modules, loaded = {}, {}
--@placeholders
local function fail(message)
	error(addonName .. ": " .. message, 3)
end
-- Creates (or completes) the export table of module `id`.
function wl.module(id)
	local exports = modules[id]
	if not exports then
		exports = {}
		modules[id] = exports
	end
	loaded[id] = true
	return exports
end
-- The export table of module `id`. `lazy` imports may run before it loads.
function wl.require(id, lazy)
	local exports = modules[id]
	if not exports or (not lazy and not loaded[id] and wl.debug) then
		fail("module '" .. tostring(id) .. "' has not been loaded")
	end
	return exports
end
-- Registers a module from plain Lua: ns.__wowlua.define(id, exports).
function wl.define(id, exports)
	local target = wl.module(id)
	for k, v in pairs(exports) do
		target[k] = v
	end
	return target
end
