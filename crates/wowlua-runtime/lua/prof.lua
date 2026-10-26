-- wowlua runtime: profiler (spec §7.3). Statistics per function or label:
-- calls, total, self (total minus instrumented callees), max, avg (ms).
local prof = {
stats = {},
timers = {},
errors = wl.profErrors
}
local clock = debugprofilestop
prof.clock = clock
-- Time spent in instrumented callees, per active call (innermost last;
-- finished calls are cleared to nil).
local child = {}
local function record(name, location, elapsed, own)
	local s = prof.stats[name]
	if not s then
		s = {calls = 0, total = 0, self = 0, max = 0, avg = 0, errors = 0, location = location}
		prof.stats[name] = s
	end
	s.calls = s.calls + 1
	s.total = s.total + elapsed
	s.self = s.self + own
	if elapsed > s.max then
		s.max = elapsed
	end
	s.avg = s.total / s.calls
end
prof.record = record
local function finish(name, location, start, depth, ...)
	local elapsed = clock() - start
	local inner = child[depth] or 0
	for i = #child, depth, -1 do
		child[i] = nil
	end
	if depth > 1 then
		child[depth - 1] = (child[depth - 1] or 0) + elapsed
	end
	record(name, location, elapsed, elapsed - inner)
	return...
end
local function memoryKiB()
	return collectgarbage("count")
end
-- Errors are counted with `xpcall` (an allocation per call), only when
-- asked for (`profile-errors`, `@profile({ errors = true })`).
local function failed(name, location, depth, start, message)
	for i = #child, depth, -1 do
		child[i] = nil
	end
	record(name, location, clock() - start, 0)
	local s = assert(prof.stats[name])
	s.errors = s.errors + 1
	error(message, 0)
end
local function finishTracked(name, location, start, depth, ok, ...)
	if not ok then
		return failed(name, location, depth, start, (...))
	end
	return finish(name, location, start, depth, ...)
end
-- Wraps `fn`: every call is timed under `name`; results pass through.
function prof.wrap(fn, name, location, opts)
	if type(fn) ~= "function" then
		return fn
	end
	local __wl_t2 = prof.errors if not __wl_t2 then local __wl_t1 = opts if __wl_t1 ~= nil then __wl_t1 = __wl_t1.errors end __wl_t2 = __wl_t1 end local errors = __wl_t2
	local __wl_t3 = opts if __wl_t3 ~= nil then __wl_t3 = __wl_t3.memory end local memory = __wl_t3
	if memory then
		return function(...)
			local before = memoryKiB()
			local depth = #child + 1
			child[depth] = 0
			local function done(...)
				-- (finish made the entry)
				local s = prof.stats[name]
				if s then
					s.memory = (s.memory or 0) + math.max(memoryKiB() - before, 0)
				end
				return...
			end
			return done(finish(name, location, clock(), depth, fn(...)))
		end
	end
	if errors then
		return function(...)
			local depth = #child + 1
			child[depth] = 0
			local n = select("#", ...)
			local args = {...}
			return finishTracked(name, location, clock(), depth, xpcall(function()
				return fn(unpack(args, 1, n))
			end, function(message)
				return message
			end))
		end
	end
	return function(...)
		local depth = #child + 1
		child[depth] = 0
		return finish(name, location, clock(), depth, fn(...))
	end
end
-- Times one call of `fn(...)` under `label`.
function prof.measure(label, fn, ...)
	local depth = #child + 1
	child[depth] = 0
	return finish(label, nil, clock(), depth, fn(...))
end
function prof.time(label)
	prof.timers[label] = clock()
end
function prof.timeEnd(label)
	local start = prof.timers[label]
	if start then
		prof.timers[label] = nil
		local elapsed = clock() - start
		record(label, nil, elapsed, elapsed)
	end
end
function prof.reset()
	prof.stats = {}
end
wl.prof = prof
