-- wowlua runtime: decorate (spec §4.5). Decorators were evaluated top to
-- bottom; they are applied bottom to top, each receiving the context table.


function wl.decorate(value, ctx, ...)
	for i = select("#", ...), 1, -1 do
		local result = (select(i, ...))(value, ctx)
		if result ~= nil then
			value = result
		end
	end
	return value
end
-- Field decorators receive no value; their results are ignored.
function wl.decorateField(ctx, ...)
	for i = select("#", ...), 1, -1 do
		(select(i, ...))(nil, ctx)
	end
end
-- Debug builds: `@deprecated` functions warn once, on their first call.
function wl.deprecated(fn, name, message)
	local warned = false
	return function(...)
		if not warned then
			warned = true
			print((tostring(addonName) .. ": " .. tostring(name) .. " is deprecated") .. (message and ": " .. tostring(message) or ""))
		end
		return fn(...)
	end
end
-- Registering framework decorators receive the final value, in source
-- order; their results are ignored.
function wl.register(value, ctx, ...)
	for i = 1, select("#", ...) do
		(select(i, ...))(value, ctx)
	end
	return value
end
