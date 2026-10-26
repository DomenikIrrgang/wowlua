-- wowlua runtime: profiler (spec §7.3). Statistics per function or label:
-- calls, total, self (total minus instrumented callees), max, avg (ms).
-- In dev builds every call is also reported to `wl.perf` (frame time,
-- slow calls, call paths, garbage per function).
local prof = {
stats = {},
timers = {},
errors = wl.profErrors
}
local clock = debugprofilestop
prof.clock = clock
local perf = wl.perf
-- Time spent in instrumented callees, per active call (innermost last;
-- finished calls are cleared to nil), and with garbage per function the
-- garbage they made (KB).
local child = {}
local childGarbage = {}
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
	if perf then
		perf.maxOpen("fn", name, elapsed)
	end
	return s
end
prof.record = record
local function memoryKiB()
	return collectgarbage("count")
end
-- The garbage a call made: all of it, and without profiled callees'.
local function garbage(s, depth, before)
	local made = memoryKiB() - before
	if made < 0 then
		-- A collection step ran during the call.
		made = 0
	end
	local __wl_t1 = childGarbage[depth] if __wl_t1 == nil then __wl_t1 = 0 end local inner = __wl_t1
	childGarbage[depth] = nil
	local own = math.max(made - inner, 0)
	do local __wl_t2 = s.garbage if __wl_t2 == nil then __wl_t2 = 0 end s.garbage = (__wl_t2) + made end
	do local __wl_t3 = s.selfGarbage if __wl_t3 == nil then __wl_t3 = 0 end s.selfGarbage = (__wl_t3) + own end
	if depth > 1 then
		do local __wl_t4 = childGarbage[depth - 1] if __wl_t4 == nil then __wl_t4 = 0 end childGarbage[depth - 1] = (__wl_t4) + made end
	end
	perf.garbageFns = perf.garbageFns + own
end
-- `frame0`: the frame the call started in (a call that ends in a later
-- frame was suspended: a coroutine yielded inside it).
local function finish(name, location, start, depth, frame0, node, before, ...)
	local elapsed = clock() - start
	local __wl_t5 = child[depth] if __wl_t5 == nil then __wl_t5 = 0 end local inner = __wl_t5
	for i = #child, depth, -1 do
		child[i] = nil
	end
	if depth > 1 then
		do local __wl_t6 = child[depth - 1] if __wl_t6 == nil then __wl_t6 = 0 end child[depth - 1] = (__wl_t6) + elapsed end
	end
	local s = record(name, location, elapsed, elapsed - inner)
	if before then
		garbage(s, depth, before)
	end
	if perf then
		perf.done(name, location, elapsed, elapsed - inner, node, s, frame0 ~= perf.frame)
	end
	return...
end
-- Errors are counted with `xpcall` (an allocation per call), only when
-- asked for (`profile-errors`, `@profile({ errors = true })`).
local function failed(name, location, depth, start, frame0, node, message)
	for i = #child, depth, -1 do
		child[i] = nil
	end
	local elapsed = clock() - start
	local s = record(name, location, elapsed, 0)
	s.errors = s.errors + 1
	if perf then
		perf.done(name, location, elapsed, 0, node, s, frame0 ~= perf.frame)
	end
	error(message, 0)
end
local function finishTracked(name, location, start, depth, frame0, node, before, ok, ...)
	if not ok then
		return failed(name, location, depth, start, frame0, node, (...))
	end
	return finish(name, location, start, depth, frame0, node, before, ...)
end
-- A call starts: its slot for callees' time, and what perf counts.
local function enter(name)
	local depth = #child + 1
	child[depth] = 0
	if not perf then
		return depth, nil, nil, nil
	end
	perf.depth = perf.depth + 1
	local node = perf.callPaths and perf.enter(name) or nil
	if perf.garbage then
		childGarbage[depth] = 0
		return depth, perf.frame, node, memoryKiB()
	end
	return depth, perf.frame, node, nil
end
-- Wraps `fn`: every call is timed under `name`; results pass through.
function prof.wrap(fn, name, location, opts)
	if type(fn) ~= "function" then
		return fn
	end
	local __wl_t8 = prof.errors if not __wl_t8 then local __wl_t7 = opts if __wl_t7 ~= nil then __wl_t7 = __wl_t7.errors end __wl_t8 = __wl_t7 end local errors = __wl_t8
	local __wl_t9 = opts if __wl_t9 ~= nil then __wl_t9 = __wl_t9.memory end local memory = __wl_t9
	if memory then
		return function(...)
			local before = memoryKiB()
			local depth, frame0, node, gc = enter(name)
			local function done(...)
				-- (finish made the entry)
				local s = prof.stats[name]
				if s then
					do local __wl_t10 = s.memory if __wl_t10 == nil then __wl_t10 = 0 end s.memory = (__wl_t10) + math.max(memoryKiB() - before, 0) end
				end
				return...
			end
			return done(finish(name, location, clock(), depth, frame0, node, gc, fn(...)))
		end
	end
	if errors then
		return function(...)
			local depth, frame0, node, gc = enter(name)
			local n = select("#", ...)
			local args = {...}
			return finishTracked(name, location, clock(), depth, frame0, node, gc, xpcall(function()
				return fn(unpack(args, 1, n))
			end, function(message)
				return message
			end))
		end
	end
	return function(...)
		local depth, frame0, node, gc = enter(name)
		return finish(name, location, clock(), depth, frame0, node, gc, fn(...))
	end
end
-- Times one call of `fn(...)` under `label`.
function prof.measure(label, fn, ...)
	local depth, frame0, node, gc = enter(label)
	return finish(label, nil, clock(), depth, frame0, node, gc, fn(...))
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
	if perf then
		perf.reset("prof")
	else
		prof.stats = {}
	end
end
-- Between frames nothing runs: the slots an error left open are cleared.
function prof.unwind()
	for i = #child, 1, -1 do
		child[i] = nil
	end
	for k in pairs(childGarbage) do
		childGarbage[k] = nil
	end
end
wl.prof = prof
