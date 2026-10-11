-- wowlua runtime: profiler (spec §7.3). Statistics per function or label:
-- calls, total, self (total minus instrumented callees), max, avg (ms).
local prof = { stats = {}, timers = {} }
wl.prof = prof
local clock = debugprofilestop or function()
	return os.clock() * 1000
end
prof.clock = clock
-- Time spent in instrumented callees, per active call (innermost last).
local child = {}
local function record(name, location, elapsed, own)
	local s = prof.stats[name]
	if not s then
		s = { calls = 0, total = 0, self = 0, max = 0, avg = 0, errors = 0, location = location }
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
	return ...
end
-- Wraps `fn`: every call is timed under `name`; results pass through.
function prof.wrap(fn, name, location)
	if type(fn) ~= "function" then
		return fn
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
