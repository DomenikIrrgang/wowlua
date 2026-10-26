-- wowlua runtime: frame time, slow calls, call paths, garbage and the
-- recordings (spec §7.3.1–7.3.3, §7.7.4, §7.7.7, §7.7.9). The profiler and
-- the monitor report every measured call here; the dev window reads it.
local __wl_t1 = wl.perfDefaults if __wl_t1 == nil then __wl_t1 = {callPaths = false, garbage = false, frameBudget = 1, slowCall = 2} end local defaults = __wl_t1
-- Frames kept in the ring: 60 seconds at 60 fps.

-- Seconds of per-second figures kept: two hours.

-- Slow calls kept, automatic recordings kept, names kept per frame.



-- A burst: the same event handled this often within one frame.

local function now()
	return GetTime and GetTime() or 0
end
local function memory()
	return collectgarbage("count")
end

local perf = {
callPaths = defaults.callPaths,
garbage = defaults.garbage,
frameBudget = defaults.frameBudget,
slowCall = defaults.slowCall,
defaults = defaults,
-- Measured calls running (profiled functions and monitored handlers).
depth = 0,
-- The root of the work running, its kind, and the call path's node.
root = nil,
rootKind = nil,
node = nil,
-- The number of the frame being measured (the first is 1).
frame = 1,
-- Totals since load: frames closed, measured time (ms), frames over the
-- budget, garbage sampled per frame (KB), garbage of profiled functions
-- themselves (KB, with garbage per function).
frames = 0,
time = 0,
over = 0,
garbageAll = 0,
garbageFns = 0,
lastFrame = 0,
lastOver = -1,
-- Roots, by label: a call path's first node, with the root's own
-- statistics and each function's time under it (`fns`).
roots = {},
-- Every call path node below a root.
nodes = {},
-- The frame ring, by slot.
ring = {id = {}, time = {}, garbage = {}, at = {}, names = {}, times = {}},
-- Per-second figures, by second number (`first` to `last`).
seconds = {first = 1, last = 0, at = {}, frames = {}, time = {}, max = {}, over = {}, garbage = {}},
-- The slow calls, oldest first.
slow = {},
-- Each function's time per 4 seconds over the last minute.
trend = {},
trendBase = {},
trendSlot = 0,
-- The recordings, `live` first, and those that run besides `live`.
recordings = {},
open = {},
counts = {},
-- Recordings marked or unmarked for export since the last reload.
exportPending = false,
-- The addon's WowluaDevDB_<Addon> once it is loaded.
db = nil
}
wl.perf = perf

-- The time and names of the frame being measured.
local cur, curN, curRest = 0, 0, 0
local curNames, curTimes = {}, {}
local secFrames, secTime, secMax, secOver, secGarbage, secStart = 0, 0, 0, 0, 0, now()
local lastMemory = memory()

-- The open recordings' maxima: `kind` is `fn`, `path`, `root`, `ev`, `sc`,
-- `tm` or `burst`.
local function maxOpen(kind, key, value)
	local open = perf.open
	for i = 1, #open do
		local m = open[i].max[kind]
		do local __wl_t2 = m[key] if __wl_t2 == nil then __wl_t2 = 0 end if value > (__wl_t2) then
				m[key] = value
			end end
	end
end
perf.maxOpen = maxOpen

local function rootNode(label, kind)
	local r = perf.roots[label]
	if not r then
		r = {name = label, kind = kind, kids = {}, fns = {}, calls = 0, total = 0, self = 0, max = 0}
		perf.roots[label] = r
	end
	return r
end

-- Adds `elapsed` to the frame being measured, under `label`.
local function addToFrame(label, elapsed)
	cur = cur + elapsed
	for i = 1, curN do
		if curNames[i] == label then
			curTimes[i] = curTimes[i] + elapsed
			return
		end
	end
	if curN < 8 then
		curN = curN + 1
		curNames[curN] = label
		curTimes[curN] = elapsed
	else
		curRest = curRest + elapsed
	end
end

local function logSlow(name, location, elapsed)
	local open = perf.open
	local rec = open[#open]
	do
		local __wl_t4 = now()



		local __wl_t3 = perf.root if __wl_t3 == nil then __wl_t3 = "other" end perf.slow[#perf.slow + 1] = {at = __wl_t4, ms = elapsed, name = name, location = location, root = __wl_t3,
		recording = rec and rec.name or "live",
		stack = debugstack and debugstack(3) or ""
		} end
	if #perf.slow > 200 then
		table.remove(perf.slow, 1)
	end
end

-- A profiled call starts: with call paths, its node below the running one.
function perf.enter(name)
	local __wl_t7 = perf.node if __wl_t7 == nil then local __wl_t5 = perf.root if __wl_t5 == nil then __wl_t5 = "other" end local __wl_t6 = perf.rootKind if __wl_t6 == nil then __wl_t6 = "other" end __wl_t7 = rootNode(__wl_t5, __wl_t6) end local parent = __wl_t7
	local node = parent.kids[name]
	if not node then
		node = {name = name, parent = parent, kids = {}, calls = 0, total = 0, self = 0, max = 0}
		parent.kids[name] = node
		perf.nodes[#perf.nodes + 1] = node
	end
	perf.node = node
	return node
end

-- A profiled call of `name` ended after `elapsed` ms (`own` without
-- profiled callees); `stat` is its statistics entry. A `suspended` call
-- (it ended in a later frame: a coroutine yielded inside it) counts its
-- time, but is no slow call and not over the budget.
function perf.done(name, location, elapsed, own, node, stat, suspended)
	local depth = perf.depth - 1
	perf.depth = depth
	local __wl_t8 = perf.root if __wl_t8 == nil then __wl_t8 = "other" end local label = __wl_t8
	local __wl_t10 = perf.roots[label] if __wl_t10 == nil then local __wl_t9 = perf.rootKind if __wl_t9 == nil then __wl_t9 = "other" end __wl_t10 = rootNode(label, __wl_t9) end local r = __wl_t10
	do local __wl_t11 = r.fns[name] if __wl_t11 == nil then __wl_t11 = 0 end r.fns[name] = (__wl_t11) + own end
	if node then
		node.calls = node.calls + 1
		node.total = node.total + elapsed
		node.self = node.self + own
		if elapsed > node.max then
			node.max = elapsed
		end
		maxOpen("path", node, elapsed)
		perf.node = node.parent
	end
	if elapsed > perf.frameBudget and not suspended then
		do local __wl_t12 = stat.over if __wl_t12 == nil then __wl_t12 = 0 end stat.over = (__wl_t12) + 1 end
		if elapsed > perf.slowCall then
			logSlow(name, location, elapsed)
		end
	end
	if depth <= 0 then
		perf.depth = 0
		perf.node = nil
		addToFrame(name, elapsed)
		if not perf.root then
			r.calls = r.calls + 1
			r.total = r.total + elapsed
			r.self = r.self + own
			if elapsed > r.max then
				r.max = elapsed
			end
			maxOpen("root", label, elapsed)
		end
	end
end

-- A monitored handler starts: at the top, it is the root of the work.
function perf.begin(label, kind)
	if perf.depth <= 0 then
		perf.depth = 1
		perf.root, perf.rootKind = label, kind
		perf.node = perf.callPaths and rootNode(label, kind) or nil
		return true
	end
	perf.depth = perf.depth + 1
	return false
end

-- A monitored handler ended; `stat` is its monitor entry.
function perf.finish(top, label, kind, location, elapsed, stat)
	perf.depth = perf.depth - 1
	if elapsed > perf.frameBudget then
		do local __wl_t13 = stat.over if __wl_t13 == nil then __wl_t13 = 0 end stat.over = (__wl_t13) + 1 end
		if elapsed > perf.slowCall then
			logSlow(label, location, elapsed)
		end
	end
	if top then
		local r = rootNode(label, kind)
		r.calls = r.calls + 1
		r.total = r.total + elapsed
		if elapsed > r.max then
			r.max = elapsed
		end
		maxOpen("root", label, elapsed)
		addToFrame(label, elapsed)
		perf.depth = 0
		perf.root, perf.rootKind, perf.node = nil, nil, nil
	end
end

-- Statistics snapshots: what a recording counts from its start.
local function copyStats(t, fields)
	local out = {}
	for k, s in pairs(t) do
		local c = {}
		for _, f in ipairs(fields) do
			c[f] = s[f]
		end
		out[k] = c
	end
	return out
end
local FN_FIELDS = {"calls", "total", "self", "errors", "garbage", "selfGarbage", "over"}
local MON_FIELDS = {"count", "total", "over", "bursts", "frames"}
local NODE_FIELDS = {"calls", "total", "self"}
local function snapshot()
	local prof, mon = wl.prof, wl.mon
	local roots = {}
	for label, r in pairs(perf.roots) do
		local fns = {}
		for k, v in pairs(r.fns) do
			fns[k] = v
		end
		roots[label] = {calls = r.calls, total = r.total, self = r.self, fns = fns}
	end
	local nodes = {}
	for _, n in ipairs(perf.nodes) do
		nodes[n] = {calls = n.calls, total = n.total, self = n.self}
	end
	return {
	functions = prof and copyStats(prof.stats, FN_FIELDS) or {},
	events = mon and copyStats(mon.events, MON_FIELDS) or {},
	scripts = mon and copyStats(mon.scripts, MON_FIELDS) or {},
	timers = mon and copyStats(mon.timers, MON_FIELDS) or {},
	commands = mon and copyStats(mon.commands, MON_FIELDS) or {},
	roots = roots,
	nodes = nodes,
	frames = perf.frames,
	time = perf.time,
	over = perf.over,
	garbageAll = perf.garbageAll,
	garbageFns = perf.garbageFns
	}
end
perf.snapshot = snapshot

-- `a - b`, field by field; entries that did not run are left out.
local function minus(a, b, fields, countField)
	local out = {}
	for k, s in pairs(a) do
		local base = b and b[k]
		local d = {}
		for _, f in ipairs(fields) do
			local v = s[f]
			if type(v) == "number" then
				d[f] = v - (base and base[f] or 0)
			end
		end
		do local __wl_t14 = d[countField] if __wl_t14 == nil then __wl_t14 = 0 end if (__wl_t14) > 0 then
				out[k] = d
			end end
	end
	return out
end
-- `a + b` into `a`, for a recording's figures from before a reset.
local function plus(a, b, fields)
	for k, s in pairs(b or {}) do
		local d = a[k]
		if not d then
			d = {}
			a[k] = d
		end
		for _, f in ipairs(fields) do
			if type(s[f]) == "number" then
				do local __wl_t15 = d[f] if __wl_t15 == nil then __wl_t15 = 0 end d[f] = (__wl_t15) + s[f] end
			end
		end
	end
end

-- The figures of the recordings since `base` (nil: since load).
local function delta(snap, base)

	local __wl_t16 = base if __wl_t16 ~= nil then __wl_t16 = __wl_t16.functions end local __wl_t18 = minus(snap.functions, __wl_t16, FN_FIELDS, "calls")
	local __wl_t17 = base if __wl_t17 ~= nil then __wl_t17 = __wl_t17.events end local __wl_t20 = minus(snap.events, __wl_t17, MON_FIELDS, "count")
	local __wl_t19 = base if __wl_t19 ~= nil then __wl_t19 = __wl_t19.scripts end local __wl_t22 = minus(snap.scripts, __wl_t19, MON_FIELDS, "count")
	local __wl_t21 = base if __wl_t21 ~= nil then __wl_t21 = __wl_t21.timers end local __wl_t24 = minus(snap.timers, __wl_t21, MON_FIELDS, "count")
	local __wl_t23 = base if __wl_t23 ~= nil then __wl_t23 = __wl_t23.commands end local __wl_t26 = minus(snap.commands, __wl_t23, MON_FIELDS, "count")
	local __wl_t25 = base if __wl_t25 ~= nil then __wl_t25 = __wl_t25.roots end local __wl_t28 = minus(snap.roots, __wl_t25, NODE_FIELDS, "calls")
	local __wl_t27 = base if __wl_t27 ~= nil then __wl_t27 = __wl_t27.nodes end local __wl_t31 = minus(snap.nodes, __wl_t27, NODE_FIELDS, "calls")
	local __wl_t29 = base if __wl_t29 ~= nil then __wl_t29 = __wl_t29.frames end local __wl_t30 = __wl_t29 if __wl_t30 == nil then __wl_t30 = 0 end
	local __wl_t32 = base if __wl_t32 ~= nil then __wl_t32 = __wl_t32.time end local __wl_t33 = __wl_t32 if __wl_t33 == nil then __wl_t33 = 0 end
	local __wl_t34 = base if __wl_t34 ~= nil then __wl_t34 = __wl_t34.over end local __wl_t35 = __wl_t34 if __wl_t35 == nil then __wl_t35 = 0 end
	local __wl_t36 = base if __wl_t36 ~= nil then __wl_t36 = __wl_t36.garbageAll end local __wl_t37 = __wl_t36 if __wl_t37 == nil then __wl_t37 = 0 end
	local __wl_t38 = base if __wl_t38 ~= nil then __wl_t38 = __wl_t38.garbageFns end local __wl_t39 = __wl_t38 if __wl_t39 == nil then __wl_t39 = 0 end local d = {functions = __wl_t18, events = __wl_t20, scripts = __wl_t22, timers = __wl_t24, commands = __wl_t26, roots = __wl_t28, nodes = __wl_t31, frames = snap.frames - (__wl_t30), time = snap.time - (__wl_t33), over = snap.over - (__wl_t35), garbageAll = snap.garbageAll - (__wl_t37), garbageFns = snap.garbageFns - (__wl_t39)
	}
	for label, r in pairs(d.roots) do
		r.fns = {}
		for k, v in pairs(snap.roots[label].fns) do
			local __wl_t40 = base if __wl_t40 ~= nil then __wl_t40 = __wl_t40.roots[label] if __wl_t40 ~= nil then __wl_t40 = __wl_t40.fns[k] end end local __wl_t41 = __wl_t40 if __wl_t41 == nil then __wl_t41 = 0 end local was = __wl_t41
			if v > was then
				r.fns[k] = v - was
			end
		end
	end
	return d
end
local function add(into, more)
	if not more then
		return
	end
	plus(into.functions, more.functions, FN_FIELDS)
	plus(into.events, more.events, MON_FIELDS)
	plus(into.scripts, more.scripts, MON_FIELDS)
	plus(into.timers, more.timers, MON_FIELDS)
	plus(into.commands, more.commands, MON_FIELDS)
	plus(into.roots, more.roots, NODE_FIELDS)
	plus(into.nodes, more.nodes, NODE_FIELDS)
	for label, r in pairs(more.roots) do
		local fns = into.roots[label].fns or {}
		into.roots[label].fns = fns
		for k, v in pairs(r.fns or {}) do
			do local __wl_t42 = fns[k] if __wl_t42 == nil then __wl_t42 = 0 end fns[k] = (__wl_t42) + v end
		end
	end
	for _, f in ipairs({"frames", "time", "over", "garbageAll", "garbageFns"}) do
		into[f] = into[f] + more[f]
	end
end

-- A call path as names, root first.
local function pathOf(node)
	local out = {}
	while node do
		table.insert(out, 1, node.name)
		node = node.parent
	end
	return out
end

-- Whether a frame shows nothing: no size, children or regions, or fully
-- transparent (P7102). Secret measures count as showing something.
local function showsNothing(f)
	if type(f) ~= "table" then
		return nil
	end
	local function number(method)
		local fn = f[method]
		if type(fn) ~= "function" then
			return nil
		end
		local ok, v = pcall(fn, f)
		if not ok or type(v) ~= "number" or (issecretvalue and issecretvalue(v)) then
			return nil
		end
		return v
	end
	local alpha = number("GetEffectiveAlpha")
	if alpha == 0 then
		return "fully transparent"
	end
	local w, h = number("GetWidth"), number("GetHeight")
	if (w == 0 or h == 0) and number("GetNumChildren") == 0 and number("GetNumRegions") == 0 then
		return "no size"
	end
	return nil
end

-- The data of a recording in the export format (§7.7.9), from its figures.
local function render(rec, d, maxima, ended)
	local prof, mon = wl.prof, wl.mon
	local __wl_t43 = wl.devInfo if __wl_t43 == nil then __wl_t43 = {} end local info = __wl_t43
	local data = {
	format = 1,
	addon = addonName,
	version = info.version,
	profile = info.profile,
	build = info.build,
	compiler = info.compiler,
	name = rec.name,
	kind = rec.kind,
	detail = rec.detail,
	started = rec.started,
	duration = math.max(ended - rec.at, 0),
	frameBudget = perf.frameBudget,
	slowCall = perf.slowCall,
	callPaths = rec.callPaths or perf.callPaths,
	garbage = rec.garbage or perf.garbage,
	frames = d.frames,
	time = d.time,
	over = d.over,
	garbageAll = d.garbageAll,
	garbageFns = d.garbageFns,
	functions = {},
	paths = {},
	roots = {},
	events = {},
	scripts = {},
	timers = {},
	commands = {},
	seconds = {frames = {}, time = {}, max = {}, over = {}, garbage = {}},
	memory = {},
	slow = {}
	}
	for name, s in pairs(d.functions) do
		local __wl_t44 = prof if __wl_t44 ~= nil then __wl_t44 = __wl_t44.stats[name] end local live = __wl_t44
		do local __wl_t46 = maxima if __wl_t46 then local __wl_t45 = maxima.fn[name] if __wl_t45 == nil then __wl_t45 = 0 end __wl_t46 = (__wl_t45) end local __wl_t49 = __wl_t46 if not __wl_t49 then local __wl_t47 = live if __wl_t47 ~= nil then __wl_t47 = __wl_t47.max end local __wl_t48 = __wl_t47 if __wl_t48 == nil then __wl_t48 = 0 end __wl_t49 = (__wl_t48) end s.max = __wl_t49 end
		do local __wl_t50 = live if __wl_t50 ~= nil then __wl_t50 = __wl_t50.location end s.location = __wl_t50 end
		data.functions[name] = s
	end
	for node, s in pairs(d.nodes) do
		s.path = pathOf(node)
		do local __wl_t52 = maxima if __wl_t52 then local __wl_t51 = maxima.path[node] if __wl_t51 == nil then __wl_t51 = 0 end __wl_t52 = (__wl_t51) end s.max = __wl_t52 or node.max end
		data.paths[#data.paths + 1] = s
	end
	for label, s in pairs(d.roots) do
		local top, topSelf = nil, 0
		for name, t in pairs(s.fns) do
			if t > topSelf then
				top, topSelf = name, t
			end
		end
		local live = perf.roots[label]
		do
			local __wl_t53 = live if __wl_t53 ~= nil then __wl_t53 = __wl_t53.kind end local __wl_t54 = __wl_t53 if __wl_t54 == nil then __wl_t54 = "other" end


			local __wl_t56 = maxima if __wl_t56 then local __wl_t55 = maxima.root[label] if __wl_t55 == nil then __wl_t55 = 0 end __wl_t56 = (__wl_t55) end local __wl_t59 = __wl_t56 if not __wl_t59 then local __wl_t57 = live if __wl_t57 ~= nil then __wl_t57 = __wl_t57.max end local __wl_t58 = __wl_t57 if __wl_t58 == nil then __wl_t58 = 0 end __wl_t59 = (__wl_t58) end data.roots[label] = {kind = __wl_t54, calls = s.calls, total = s.total, max = __wl_t59,
			top = top,
			topSelf = topSelf
			} end
	end
	for _, kind in ipairs({"events", "scripts", "timers", "commands"}) do
		local short = ({events = "ev", scripts = "sc", timers = "tm", commands = "cm"})[kind]
		for key, s in pairs(d[kind]) do
			local live = mon and mon[kind][key]
			do local __wl_t61 = maxima if __wl_t61 then local __wl_t60 = maxima[short][key] if __wl_t60 == nil then __wl_t60 = 0 end __wl_t61 = (__wl_t60) end local __wl_t64 = __wl_t61 if not __wl_t64 then local __wl_t62 = live if __wl_t62 ~= nil then __wl_t62 = __wl_t62.max end local __wl_t63 = __wl_t62 if __wl_t63 == nil then __wl_t63 = 0 end __wl_t64 = (__wl_t63) end s.max = __wl_t64 end
			do local __wl_t65 = live if __wl_t65 ~= nil then __wl_t65 = __wl_t65.detail end s.detail = __wl_t65 end
			do local __wl_t66 = live if __wl_t66 ~= nil then __wl_t66 = __wl_t66.script end s.script = __wl_t66 end
			if kind == "events" then
				do local __wl_t68 = maxima if __wl_t68 then local __wl_t67 = maxima.burst[key] if __wl_t67 == nil then __wl_t67 = 0 end __wl_t68 = (__wl_t67) end local __wl_t71 = __wl_t68 if not __wl_t71 then local __wl_t69 = live if __wl_t69 ~= nil then __wl_t69 = __wl_t69.burstMax end local __wl_t70 = __wl_t69 if __wl_t70 == nil then __wl_t70 = 0 end __wl_t71 = (__wl_t70) end s.burstMax = __wl_t71 end
			end
			if s.script == "OnUpdate" and mon then
				s.showsNothing = showsNothing(mon.objects[key])
			end
			data[kind][key] = s
		end
	end
	local sec = perf.seconds
	do local __wl_t72 = rec.second if __wl_t72 == nil then __wl_t72 = 1 end for i = math.max(__wl_t72, sec.first), sec.last do
			local n = #data.seconds.frames + 1
			data.seconds.frames[n] = sec.frames[i]
			data.seconds.time[n] = sec.time[i]
			data.seconds.max[n] = sec.max[i]
			data.seconds.over[n] = sec.over[i]
			data.seconds.garbage[n] = sec.garbage[i]
		end end
	local __wl_t73 = mon if __wl_t73 ~= nil then __wl_t73 = __wl_t73.memory end local __wl_t74 = __wl_t73 if __wl_t74 == nil then __wl_t74 = {} end local __wl_t75 = mon if __wl_t75 ~= nil then __wl_t75 = __wl_t75.memoryAt end local __wl_t76 = __wl_t75 if __wl_t76 == nil then __wl_t76 = {} end local samples, times = __wl_t74, __wl_t76
	for i, kb in ipairs(samples) do
		local at = times[i]
		if at and at >= rec.at and at <= ended then
			data.memory[#data.memory + 1] = {at = at - rec.at, kb = kb}
		end
	end
	for i = #perf.slow, 1, -1 do
		local s = perf.slow[i]
		if s.at >= rec.at and s.at <= ended then
			data.slow[#data.slow + 1] = {
			at = s.at - rec.at,
			ms = s.ms,
			name = s.name,
			location = s.location,
			root = s.root,
			recording = s.recording,
			stack = s.stack
			}
		end
	end
	return data
end

-- The data of a recording: finished ones keep theirs; `live` and open ones
-- are taken now.
function perf.data(rec)
	if rec.data then
		return rec.data
	end
	local t = now()
	if rec.kind == "live" then
		local d = delta(snapshot(), rec.base)
		return render(rec, d, nil, t)
	end
	local d = delta(snapshot(), rec.base)
	add(d, rec.carry)
	return render(rec, d, rec.max, t)
end

-- The frames of a recording still in the ring: first and last frame number.
function perf.frameRange(rec)
	local __wl_t77 = rec.lastFrame if __wl_t77 == nil then __wl_t77 = (perf.frame - 1) end local last = __wl_t77
	local __wl_t78 = rec.frame if __wl_t78 == nil then __wl_t78 = 1 end local first = math.max(__wl_t78, perf.frame - 3600)
	return first, last
end
-- The figures of frame `id` while it is in the ring: time, garbage, the
-- time it closed, and the names and times of what ran in it.
function perf.frameAt(id)
	local ring = perf.ring
	local slot = id % 3600 + 1
	if ring.id[slot] ~= id then
		return nil, nil, nil, nil, nil
	end
	return ring.time[slot], ring.garbage[slot], ring.at[slot], ring.names[slot], ring.times[slot]
end

local function counted(kind)
	do local __wl_t79 = perf.counts[kind] if __wl_t79 == nil then __wl_t79 = 0 end perf.counts[kind] = (__wl_t79) + 1 end
	return perf.counts[kind]
end

-- Starts a recording (`kind`: combat, pull, manual).
function perf.start(kind, name, detail)


	local __wl_t80 = name if __wl_t80 == nil then __wl_t80 = (kind == "manual" and "recording " .. tostring(counted(kind)) or tostring(kind) .. " " .. tostring(counted(kind))) end local rec = {kind = kind, name = __wl_t80,
	detail = detail,
	at = now(),
	started = time and time() or 0,
	frame = perf.frame,
	second = perf.seconds.last + 1,
	base = snapshot(),
	max = {fn = {}, path = {}, root = {}, ev = {}, sc = {}, tm = {}, cm = {}, burst = {}},
	callPaths = perf.callPaths,
	garbage = perf.garbage,
	running = true
	}
	perf.recordings[#perf.recordings + 1] = rec
	perf.open[#perf.open + 1] = rec
	return rec
end

-- The open recording of `kind`, newest first.
function perf.running(kind)
	for i = #perf.open, 1, -1 do
		if perf.open[i].kind == kind then
			return perf.open[i]
		end
	end
	return nil
end

local function writeExports()
	local db = perf.db
	if not db then
		return
	end
	local out = {}
	for _, rec in ipairs(perf.recordings) do
		if rec.export then
			local data = perf.data(rec)
			data.hints = perf.hints(data)
			out[#out + 1] = data
		end
	end
	db.exports = out
end
perf.writeExports = writeExports

-- Ends a recording; `name` renames it. The oldest automatic recordings
-- beyond the last 20 are dropped.
function perf.stop(rec, name)
	if not rec.running then
		return
	end
	if name and name ~= "" then
		rec.name = name
	end
	rec.lastFrame = perf.frame - 1
	rec.data = perf.data(rec)
	rec.running = false
	rec.base, rec.carry = nil, nil
	for i, r in ipairs(perf.open) do
		if r == rec then
			table.remove(perf.open, i)
			break
		end
	end
	if rec.kind == "combat" or rec.kind == "pull" then
		local auto = 0
		for i = #perf.recordings, 1, -1 do
			local r = perf.recordings[i]
			if (r.kind == "combat" or r.kind == "pull") and not r.running and not r.export then
				auto = auto + 1
				if auto > 20 then
					table.remove(perf.recordings, i)
				end
			end
		end
	end
	if rec.export then
		writeExports()
	end
end

-- Marks or unmarks a recording for export (§7.7.9).
function perf.setExport(rec, on)
	rec.export = on
	perf.exportPending = true
	writeExports()
end

-- Clears statistics (`what`: "prof", "mon", or nil for everything):
-- `live` starts again, the open recordings keep what they counted so far.
function perf.reset(what)
	local snap = snapshot()
	for _, rec in ipairs(perf.open) do
		local so = delta(snap, rec.base)
		if rec.carry then
			add(so, rec.carry)
		end
		rec.carry = so
	end
	if wl.prof and what ~= "mon" then
		wl.prof.stats = {}
		perf.nodes = {}
		for _, r in pairs(perf.roots) do
			r.fns, r.kids = {}, {}
		end
		perf.trend, perf.trendBase = {}, {}
		perf.garbageFns = 0
	end
	if wl.mon and what ~= "prof" then
		wl.mon.events, wl.mon.scripts, wl.mon.timers, wl.mon.commands = {}, {}, {}, {}
		wl.mon.objects = setmetatable({}, {__mode = "v"})
	end
	if not what then
		perf.roots, perf.nodes = {}, {}
		perf.frames, perf.time, perf.over, perf.garbageAll, perf.garbageFns = 0, 0, 0, 0, 0
		perf.slow = {}
	end
	local base = snapshot()
	for _, rec in ipairs(perf.open) do
		rec.base = base
	end
	local live = perf.recordings[1]
	live.at, live.started, live.frame, live.second = now(), time and time() or 0, perf.frame, perf.seconds.last + 1
	live.base = base
end

-- Switches and thresholds (§7.7.5): the window's, saved per addon, win
-- over the profile's until reset.
function perf.set(key, value)
	(perf)[key] = value
	local db = perf.db
	if db then
		db.settings = db.settings or {}
		db.settings[key] = value
	end
end
function perf.resetSettings()
	for _, k in ipairs({"callPaths", "garbage", "frameBudget", "slowCall"}) do
		(perf)[k] = (defaults)[k]
	end
	if perf.db then
		perf.db.settings = nil
	end
end

-- The `live` recording.
perf.recordings[1] = {
kind = "live",
name = "live",
at = now(),
started = time and time() or 0,
frame = 1,
second = 1,
running = true
}

-- Closes a second: its figures, and every 4 seconds the functions' trends.
local function closeSecond(t)
	local sec = perf.seconds
	local i = sec.last + 1
	sec.last = i
	sec.at[i], sec.frames[i], sec.time[i], sec.max[i], sec.over[i], sec.garbage[i] =
	secStart, secFrames, secTime, secMax, secOver, secGarbage
	local old = i - 7200
	if old >= sec.first then
		sec.at[old], sec.frames[old], sec.time[old], sec.max[old], sec.over[old], sec.garbage[old] =
		nil, nil, nil, nil, nil, nil
		sec.first = old + 1
	end
	secFrames, secTime, secMax, secOver, secGarbage, secStart = 0, 0, 0, 0, 0, t
	if i % 4 == 0 and wl.prof then
		local slot = perf.trendSlot % 15 + 1
		perf.trendSlot = slot
		for name, s in pairs(wl.prof.stats) do
			local tr = perf.trend[name]
			if not tr then
				tr = {}
				perf.trend[name] = tr
			end
			do local __wl_t81 = perf.trendBase[name] if __wl_t81 == nil then __wl_t81 = 0 end tr[slot] = s.total - (__wl_t81) end
			perf.trendBase[name] = s.total
		end
	end
end

-- The frame mark: closes the frame being measured.
local function mark()
	local t = now()
	local count = memory()
	local garbage = count - lastMemory
	if garbage < 0 then
		garbage = 0
	end
	lastMemory = count
	local id = perf.frame
	local ring = perf.ring
	local slot = id % 3600 + 1
	ring.id[slot], ring.time[slot], ring.garbage[slot], ring.at[slot] = id, cur, garbage, t
	local names, times = ring.names[slot], ring.times[slot]
	if not names then
		names, times = {}, {}
		ring.names[slot], ring.times[slot] = names, times
	end
	for i = 1, curN do
		names[i], times[i] = curNames[i], curTimes[i]
	end
	for i = curN + 1, #names do
		names[i], times[i] = nil, nil
	end
	if curRest > 0 then
		names[curN + 1], times[curN + 1] = "(other)", curRest
	end
	perf.frames = perf.frames + 1
	perf.time = perf.time + cur
	perf.garbageAll = perf.garbageAll + garbage
	perf.lastFrame = cur
	local over = cur > perf.frameBudget
	if over then
		perf.over = perf.over + 1
		perf.lastOver = t
	end
	secFrames = secFrames + 1
	secTime = secTime + cur
	secGarbage = secGarbage + garbage
	if cur > secMax then
		secMax = cur
	end
	if over then
		secOver = secOver + 1
	end
	if t - secStart >= 1 then
		closeSecond(t)
	end
	perf.frame = id + 1
	cur, curN, curRest = 0, 0, 0
	-- Nothing of the addon runs between frames: what an error left open
	-- is closed.
	perf.depth, perf.root, perf.rootKind, perf.node = 0, nil, nil, nil
	if wl.prof then
		wl.prof.unwind()
	end
end
perf.mark = mark

-- Combats, pulls, the saved data, and the frame mark.
if type(CreateFrame) == "function" then
	local f = CreateFrame("Frame")
	f:SetScript("OnUpdate", mark)
	for _, e in ipairs({"ADDON_LOADED", "PLAYER_REGEN_DISABLED", "PLAYER_REGEN_ENABLED", "ENCOUNTER_START", "ENCOUNTER_END", "PLAYER_LOGOUT"}) do
		pcall(f.RegisterEvent, f, e)
	end
	local pulls = {}
	f:SetScript("OnEvent", function(_, event, ...)
		if event == "ADDON_LOADED" and ... == addonName then
			local name = "WowluaDevDB_" .. addonName:gsub("[^%w_]", "_")
			local db = _G[name]
			if type(db) ~= "table" then
				db = {}
				_G[name] = db
			end
			perf.db = db
			for k, v in pairs(db.settings or {}) do
				(perf)[k] = v
			end
			-- Recordings exported before the reload stay marked.
			for _, data in ipairs(db.exports or {}) do
				if type(data) == "table" then
					perf.recordings[#perf.recordings + 1] = {
					kind = data.kind, name = data.name, detail = data.detail, at = 0,
					started = data.started, data = data, export = true, saved = true
					}
				end
			end
		elseif event == "PLAYER_REGEN_DISABLED" then
			if not perf.running("combat") then
				perf.start("combat")
			end
		elseif event == "PLAYER_REGEN_ENABLED" then
			local rec = perf.running("combat")
			if rec then
				perf.stop(rec)
			end
		elseif event == "ENCOUNTER_START" then
			local _, encounter = ...
			local key = tostring(encounter or "?")
			do local __wl_t82 = pulls[key] if __wl_t82 == nil then __wl_t82 = 0 end pulls[key] = (__wl_t82) + 1 end
			perf.start("pull", "pull " .. tostring(pulls[key]), encounter)
		elseif event == "ENCOUNTER_END" then
			local rec = perf.running("pull")
			if rec then
				perf.stop(rec)
			end
		elseif event == "PLAYER_LOGOUT" then
			writeExports()
		end
	end)
end

-- Performance hints (§7.7.7): the rules over a recording's data. Each hint
-- has a code, its subject, the evidence, a fix, and the number that grows
-- when the finding gets worse.
local function n(v)
	local s = tostring(math.floor(v + 0.5))
	local out = s:reverse():gsub("(%d%d%d)", "%1,"):reverse()
	return (out:gsub("^,", ""))
end
local function ms(v)
	return string.format(v >= 10 and "%.1f ms" or "%.2f ms", v)
end
local function kb(v)
	return v >= 1024 and string.format("%.1f MB", v / 1024) or string.format("%.0f KB", v)
end
local function span(data)
	local __wl_t83 = data.duration if __wl_t83 == nil then __wl_t83 = 0 end local d = __wl_t83
	return d >= 60 and string.format("%s (%.1f min)", data.name, d / 60) or string.format("%s (%d s)", data.name, d)
end
function perf.hints(data)
	local out = {}
	local __wl_t84 = data.frames if __wl_t84 == nil then __wl_t84 = 0 end local frames = math.max(__wl_t84, 1)
	-- P7101: bursts of one event.
	do local __wl_t85 = data.events if __wl_t85 == nil then __wl_t85 = {} end for event, s in pairs(__wl_t85) do
			do local __wl_t86 = s.bursts if __wl_t86 == nil then __wl_t86 = 0 end if (__wl_t86) >= 10 then
					do


						local __wl_t88 = n(s.count) local __wl_t89 = span(data) local __wl_t90 = n(s.bursts) local __wl_t87 = s.burstMax if __wl_t87 == nil then __wl_t87 = 0 end local __wl_t92 = tostring(event) .. " was handled " .. tostring(__wl_t88) .. " times in " .. tostring(__wl_t89) .. ", in bursts of 5 or more in " .. tostring(__wl_t90) .. " frames (up to " .. tostring(n(__wl_t87)) .. " in one frame), " .. tostring(ms(s.total)) .. " in all"

						local __wl_t91 = data.duration if __wl_t91 == nil then __wl_t91 = 1 end out[#out + 1] = {code = "P7101", subject = event, evidence = __wl_t92, fix = "Collapse each burst into one call: give the function that does the work @throttle(0.1) or @nextFrame from wowlua:timer, and return early for units or bags the addon does not show", metric = s.bursts / math.max(__wl_t91, 1)
						} end
				end end
		end end
	-- P7102: an OnUpdate in nearly every frame while its frame shows nothing.
	do local __wl_t93 = data.scripts if __wl_t93 == nil then __wl_t93 = {} end for key, s in pairs(__wl_t93) do
			local __wl_t94 = s.frames if __wl_t94 == nil then __wl_t94 = 0 end local share = (__wl_t94) / frames
			do local __wl_t96 = s.script == "OnUpdate" and s.showsNothing and share >= 0.9 if __wl_t96 then local __wl_t95 = data.duration if __wl_t95 == nil then __wl_t95 = 0 end __wl_t96 = (__wl_t95) >= 10 end if __wl_t96 then
					do


						local __wl_t98 = string.format("%s ran in %d%% of the %s frames of %s while its frame showed nothing (%s), %s in all",
						key, math.floor(share * 100), n(data.frames), span(data), s.showsNothing, ms(s.total))

						local __wl_t97 = data.duration if __wl_t97 == nil then __wl_t97 = 1 end out[#out + 1] = {code = "P7102", subject = key, evidence = __wl_t98, fix = "Run the work on a timer, with every(0.1, fn) from wowlua:timer, or set the script in OnShow and clear it with SetScript(\"OnUpdate\", nil) in OnHide", metric = s.total / math.max(__wl_t97, 1)
						} end
				end end
		end end
	-- P7201: one function's share of the garbage.
	local __wl_t99 = data.garbageFns if __wl_t99 == nil then __wl_t99 = 0 end local all = __wl_t99
	do local __wl_t100 = data.functions if __wl_t100 == nil then __wl_t100 = {} end for name, s in pairs(__wl_t100) do
			local __wl_t101 = s.selfGarbage if __wl_t101 == nil then __wl_t101 = 0 end local own = __wl_t101
			if all > 0 and own >= 100 and own / all >= 0.25 then
				do


					local __wl_t103 = string.format("%s created %s, %d%% of the garbage the addon's functions made in %s, %s per call",
					name, kb(own), math.floor(own / all * 100), span(data), kb(own / math.max(s.calls, 1)))

					local __wl_t102 = data.duration if __wl_t102 == nil then __wl_t102 = 1 end out[#out + 1] = {code = "P7201", subject = name, evidence = __wl_t103, fix = "Reuse tables instead of creating them per call (clear them with wipe), move closures that capture nothing out of loops and handlers, and cache strings and results that do not change", metric = own / math.max(__wl_t102, 1)
					} end
			end
		end end
	-- P7301: single calls over the frame budget.
	local function over(name, s, what)
		do local __wl_t104 = s.over if __wl_t104 == nil then __wl_t104 = 0 end if (__wl_t104) > 0 then
				do



					local __wl_t105 = s.max if __wl_t105 == nil then __wl_t105 = 0 end local __wl_t107 = ms(__wl_t105) local __wl_t106 = data.frameBudget if __wl_t106 == nil then __wl_t106 = 1 end local __wl_t110 = ms(__wl_t106) local __wl_t111 = n(s.over) local __wl_t108 = s.calls if __wl_t108 == nil then __wl_t108 = s.count end local __wl_t109 = __wl_t108 if __wl_t109 == nil then __wl_t109 = 0 end local __wl_t113 = string.format("%s took %s in its slowest call, over the %s frame budget; %s of %s %s were over it in %s", name, __wl_t107, __wl_t110, __wl_t111, n(__wl_t109), what, span(data))

					local __wl_t112 = s.max if __wl_t112 == nil then __wl_t112 = 0 end out[#out + 1] = {code = "P7301", subject = name, evidence = __wl_t113, fix = "Spread the work across frames: run it as a job with jobs.start from wowlua:jobs and call ctx:check() in its loop, or wait for the end of combat", metric = __wl_t112
					} end
			end end
	end
	do local __wl_t114 = data.functions if __wl_t114 == nil then __wl_t114 = {} end for name, s in pairs(__wl_t114) do
			over(name, s, "calls")
		end end
	for _, kind in ipairs({"events", "scripts", "timers", "commands"}) do
		do local __wl_t115 = data[kind] if __wl_t115 == nil then __wl_t115 = {} end for key, s in pairs(__wl_t115) do
				over(key, s, "runs")
			end end
	end
	for _, h in ipairs(out) do
		h.recording = data.name
	end
	table.sort(out, function(a, b)
		if a.code ~= b.code then
			return a.code < b.code
		end
		return a.metric > b.metric
	end)
	return out
end
