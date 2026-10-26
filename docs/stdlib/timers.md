# Timers and background jobs

`wowlua:timer` delays and repeats work and keeps busy handlers in check with
throttle, debounce and next-frame batching. `wowlua:jobs` spreads heavy work
over many frames, so the game keeps a steady frame rate.

## Do something later

`after(sec, fn)` runs a function once, `sec` seconds from now. It returns the
game's own timer handle (`C_Timer.NewTimer`), with `Cancel()` and
`IsCancelled()`.

```wowlua title=src/welcome.wowlua
import after from "wowlua:timer"
import onLogin from "wowlua:lifecycle"
import on from "wowlua:events"
import print from "wowlua:log"

@onLogin
local function welcome()
    local hint = after(10, function()
        print("type /mya for options")
    end)
    -- no hint if the player opens a bag first
    on("BAG_OPEN", function()
        hint:Cancel()
    end)
end
```

## Repeat on an interval

`every(sec, fn, iterations?)` repeats until you cancel it, or the given number
of times (`C_Timer.NewTicker`). The function gets the ticker, so it can cancel
itself.

```wowlua title=src/countdown.wowlua
import every from "wowlua:timer"
import print from "wowlua:log"

-- a five second countdown: five ticks, one per second
export function countdown()
    local remaining = 5
    every(1, function(ticker: TickerHandle)
        remaining -= 1
        if remaining > 0 then
            print(remaining)
        else
            print("go")
            ticker:Cancel()
        end
    end, 5)
end
```

For a timer that you switch on and off often, `repeating(sec, fn)` is simpler.
It starts stopped. `start()` and `stop()` switch it, `setRunning(running)` does
either, and `isRunning()` tells which. A start schedules the first tick one
interval later, and starting a running timer does nothing.

```wowlua title=src/range.wowlua
import repeating from "wowlua:timer"
import event from "wowlua:events"
import print from "wowlua:log"

local poll = repeating(0.2, function()
    print("target in range:", CheckInteractDistance("target", 4))
end)

@event("PLAYER_TARGET_CHANGED")
local function targetChanged()
    poll:setRunning(UnitExists("target"))   -- poll only while there is a target
end
```

## Don't run too often

Some events fire many times a second. A function with `@throttle(sec)` runs
right away on the first call. Further calls within `sec` collapse into one
trailing call, with the latest arguments. The work runs at most once per
interval, and the last state is never missed.

```wowlua title=src/health.wowlua
import event from "wowlua:events"
import throttle from "wowlua:timer"
import print from "wowlua:log"

@event("UNIT_HEALTH")
@throttle(0.2)
local function healthChanged(unit: UnitToken)
    print(unit, UnitHealth(unit))
end
```

On a method, each object gets its own throttle, so two windows resized at the
same time don't hold each other up. The state is kept in a table with weak
keys, and goes away with the object.

```wowlua title=src/window.wowlua
import throttle from "wowlua:timer"

export class Panel
    frame: Frame = CreateFrame("Frame", nil, UIParent)

    @throttle(0.1)
    function onResize()
        self:layout()
    end

    function layout()
        self.frame:SetHeight(self.frame:GetWidth() / 2)
    end
end
```

## Wait until the player stops

`@debounce(sec)` runs the function `sec` seconds after the last call, with that
call's arguments. A search box is the usual case: filter when the player stops
typing, not on every key.

```wowlua title=src/search.wowlua
import debounce from "wowlua:timer"
import print from "wowlua:log"

@debounce(0.3)
local function search(text: string)
    print("searching for", text)
end

local box = CreateFrame("EditBox", nil, UIParent, "InputBoxTemplate")
box:SetScript("OnTextChanged", function(self: EditBox)
    search(self:GetText())
end)
```

## Batch work into the next frame

`@nextFrame` moves the call to the next frame and merges the calls made until
then into one, with the last call's arguments. Call `redraw()` from three
event handlers in the same frame and the window redraws once.

```wowlua title=src/redraw.wowlua
import nextFrame from "wowlua:timer"
import on from "wowlua:events"
import print from "wowlua:log"

@nextFrame
local function redraw()
    print("free slots:", C_Container.GetContainerNumFreeSlots(0))
end

on("BAG_UPDATE", function() redraw() end)
on("PLAYER_MONEY", function() redraw() end)
on("ITEM_LOCK_CHANGED", function() redraw() end)
```

Each decorated function merges its own calls, and all of them run on the next
frame in the order they were first called. Several kinds of deferred work need
nothing more than one decorated function each. `nextFrame(fn)`, called as a
function, runs `fn` on the next frame without merging.

> [!NOTE]
> A function with `@throttle`, `@debounce` or `@nextFrame` may run later than
> it was called, so it can't return a value (E8030). Arguments are passed on
> as they are, secret values included.

## Spread heavy work over many frames

Indexing every item of a large collection, or scanning a long list of auctions,
freezes the screen if done in one go, and the client may stop a script that
runs too long. `wowlua:jobs` runs such work a slice at a time, in a coroutine,
within a few milliseconds per frame.

```wowlua title=src/index.wowlua
import * as jobs from "wowlua:jobs"
import type JobContext from "wowlua:jobs"
import print from "wowlua:log"

local names: { [number]: string } = {}

export local job = jobs.start(function(ctx: JobContext)
    local last = 250000
    for id = 1, last do
        local name = C_Item.GetItemNameByID(id)
        if name then
            names[id] = name
        end
        ctx:progress(id / last)
        ctx:check()         -- yields once this frame's time is used up
    end
    return last
end, {
    name = "item index",
    priority = "low",
    pauseInCombat = true,
    onProgress = function(p: number) print(("indexing: %d%%"):format(p * 100)) end,
    onDone = function(count: number) print("indexed", count, "items") end,
})
```

- The job starts on the next frame and resumes frame by frame until its
  function returns. Its results go to `onDone` and to `job:result()`.
- `ctx:check()` is cheap, so call it often: it yields only when the frame's
  budget is spent. `ctx:yield()` always waits for the next frame,
  `ctx:sleep(seconds)` for that long, and `ctx:waitFor(event)` until the event
  or message fires, returning its payload.
- `priority` is `"high"`, `"normal"` (the default) or `"low"`. When several
  jobs share the budget, they take turns weighted 4, 2 and 1.
- With `pauseInCombat`, the job pauses when combat starts and resumes when it
  ends.
- `job:cancel()` stops the job. `job:status()` returns `"pending"`,
  `"running"`, `"paused"`, `"done"`, `"failed"` or `"cancelled"`, and
  `job:progress()` the last value passed to `ctx:progress`.
- A job that raises an error is reported, ends as `"failed"` and calls
  `onError` with the message. Other jobs keep running.
- `jobs.configure({ budget = 2 })` sets the milliseconds per frame that all of
  your addon's jobs share (4 by default). Each addon has its own budget.

> [!WARNING]
> Lua 5.1 can't yield across `pcall`, `xpcall`, a `table.sort` comparator, a
> `gsub` callback or a metamethod. Calling `check`, `yield`, `sleep` or
> `waitFor` there fails at run time with "attempt to yield across
> metamethod/C-call boundary". The compiler warns about the cases it can see
> (W8430). Yield between those calls instead.

## At a glance

| Call | Use it to |
|------|-----------|
| `after(sec, fn)` | Run once later; returns a handle with `Cancel()` |
| `every(sec, fn, n?)` | Repeat forever, or `n` times |
| `repeating(sec, fn)` | A timer to `start()`, `stop()` or `setRunning(on)` |
| `@throttle(sec)` | Run now, then at most once per interval |
| `@debounce(sec)` | Run once the calls have stopped |
| `@nextFrame`, `nextFrame(fn)` | Defer to the next frame, merging repeated calls |
| `jobs.start(fn, opts)` | Run heavy work a slice per frame |
| `jobs.configure({ budget })` | Set the per-frame budget of all your jobs |

The full rules are in the specification:
[timer](../../spec/stdlib/core.md#timer) and
[jobs](../../spec/stdlib/infrastructure.md#jobs).
