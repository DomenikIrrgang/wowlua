-- wowlua runtime: tracing (spec §7.4). Entry with arguments, exit with
-- results and errors, indented by call depth, at the `trace` log level.
local trace = {depth = 0}
local function show(v)
	if issecretvalue and issecretvalue(v) then
		return "<secret>"
	end
	local t = type(v)
	if t == "string" then
		return string.format("%q", #v > 40 and v:sub(1, 40) .. "..." or v)
	elseif t == "table" then
		if canaccesstable and not (canaccesstable)(v) then
			return "table(?)"
		end
		local n = 0
		for _ in pairs(v) do
			n = n + 1
			if n > 99 then
				break
			end
		end
		return "table(" .. (n > 99 and "99+" or n) .. ")"
	end
	return tostring(v)
end
local function list(...)
	local parts = {}
	for i = 1, select("#", ...) do
		parts[i] = show((select(i, ...)))
	end
	return table.concat(parts, ", ")
end
local function out(line)
	local emit = wl.logEmit
	if emit then
		emit("trace", nil, line)
	else
		print(tostring(addonName) .. ": " .. tostring(line))
	end
end
local function finish(name, indent, ok, ...)
	trace.depth = trace.depth - 1
	if not ok then
		out(indent .. "x " .. name .. ": " .. tostring((...)))
		error((...), 0)
	end
	out(indent .. "< " .. name .. (select("#", ...) > 0 and (" = " .. list(...)) or ""))
	return...
end
-- Wraps `fn`: each call logs entry, exit and errors under `name`.
function trace.wrap(fn, name, location, opts)
	if type(fn) ~= "function" then
		return fn
	end
	local __wl_t1 = opts if __wl_t1 ~= nil then __wl_t1 = __wl_t1.args end local args = __wl_t1 ~= false
	return function(...)
		local indent = string.rep("  ", trace.depth)
		out(indent .. "> " .. name .. (args and ("(" .. list(...) .. ")") or ""))
		trace.depth = trace.depth + 1
		return finish(name, indent, pcall(fn, ...))
	end
end
wl.trace = trace
