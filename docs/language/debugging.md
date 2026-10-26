# Debugging helpers

`wowlua:debug` has assertions, value dumps, timers and names the compiler
fills in, and `wowlua:reflect` answers questions about classes and enums at
run time. This page also covers profiling, tracing and the runtime checks
of arguments. Most of it is removed from release builds, together with the
arguments of the calls, so it costs players nothing.

## What a dev build gives you for free

Without any helper in your code, the dev build that `wowlua dev` makes
already has a profiler around every named function, a monitor for events,
scripts, timers and slash commands, recordings of the time it takes per
frame, error capture, and the `/wl` dev window. It keeps one Lua
file per source with the same line numbers, so an error on line 14 of
`src/core/tracker.lua` is on line 14 of `src/core/tracker.wowlua`.

|  | dev | test | release |
|--|-----|------|---------|
| `assert`, `dump` | kept | kept | removed |
| Lowest log level compiled in | `debug` | `trace` | `warn` |
| Functions profiled | all | none | none |
| Runtime argument checks | exports and handlers | all annotated functions | none |
| Dev window, monitor, error capture | yes | no | no |

Each row is a profile setting (`assertions`, `debug`, `log-level`,
`instrument`, `runtime-checks`, `dev-console`, `monitor`, `capture-errors`)
that you can change per profile in `wowlua.json`, see
[Build info and profiles](flavors.md#profiles-dev-release-test-and-your-own).
The dev window, the VS Code debugger and `wowlua trace` are covered in the
[debugging guide](../guide/debugging.md).

## Check assumptions and look at values

`assert` raises an error with your message and the place in the source.
`dump` prints the source text of each expression next to its value, so you
don't have to label your `print` calls.

```wowlua title=src/ledger.wowlua
import assert, dump from "wowlua:debug"

export interface Entry
    amount: number
    item: string
end

local entries: Entry[] = {}

local function summarize(list: Entry[]): { gold: number, items: number }
    local gold = 0
    for _, e in ipairs(list) do
        gold += e.amount
    end
    return { gold = gold, items = #list }
end

export function addEntry(entry: Entry?)
    assert(entry, "addEntry needs an entry")
    -- from here on, entry is an Entry
    dump(entry.amount, #entries, summarize(entries))
    entries[#entries + 1] = entry
end
```

Two calls with entries, then one with `nil`, print this in `wowlua test`, and
the same in a dev build:

```text
src/ledger.wowlua:21 (addEntry) entry.amount = 4085
src/ledger.wowlua:21 (addEntry) #entries = 0
src/ledger.wowlua:21 (addEntry) summarize(entries) = { items = 0, gold = 0 }
src/ledger.wowlua:21 (addEntry) entry.amount = 125
src/ledger.wowlua:21 (addEntry) #entries = 1
src/ledger.wowlua:21 (addEntry) summarize(entries) = { items = 1, gold = 4085 }
MyAddon: src/ledger.wowlua:19 (addEntry): assertion failed: addEntry needs an entry
```

In a release build both lines are gone, and `summarize(entries)` never runs:

```lua
addEntry = function(entry)
	-- from here on, entry is an Entry
	entries[#entries + 1] = entry
end
```

- That is more than `if build.debug then` does: the arguments of a removed
  call are never evaluated.
- `assert` narrows types as Lua's `assert` does. Lua's own is still there as
  `_G.assert`, and is never removed.
- Because the calls can be removed, `assert` and `dump` must stand on their
  own as statements. `local ok = assert(x)` is error E3045.
- `dump` prints tables one level deep, and secret values as `<secret>`.

## Names and places, filled in by the compiler

```wowlua title=src/tracker.wowlua
import location, nameof, typeName, todo, unreachable from "wowlua:debug"
import event from "wowlua:events"

enum Mode { Simple, Advanced }

export class Tracker
    total: number = 0

    function record(amount: number)
        self.total += amount
    end
end

local frame = CreateFrame("Frame")

@event("PLAYER_LOGIN")
local function onLogin()
    print(location())
    print(nameof(Tracker.record))
    print(typeName(frame))
end

export function exportCsv(): string
    return todo("CSV export")
end

export function describe(mode: Mode): string
    if mode == Mode.Simple then
        return "simple"
    elseif mode == Mode.Advanced then
        return "advanced"
    else
        unreachable()
    end
end
```

`location`, `nameof` and `typeName` become strings at compile time, in every
build. `nameof` names what you pass it, so it changes along when you rename
the method:

```lua
	print("src/tracker.wowlua:18 (onLogin)")
	print("record")
	print("Frame")
```

`todo` raises "not implemented" and `unreachable` raises "unreachable code
reached" when they run. Both stay in every build. A `todo` in a release build
is also a warning when you build:

```text
warning[W7001]: `todo()` in a release build
   ┌─ src/tracker.wowlua:24:12
   │
24 │     return todo("CSV export")
   │            ^^^^^^^^^^^^^^^^^^ raises "not implemented" when it runs
```

## Time things and watch values live

```wowlua title=src/scan.wowlua
import * as debug from "wowlua:debug"

local queue: string[] = {}

local function scanBags(first: number, last: number): number[]
    local slots: number[] = {}
    for bag = first, last do
        slots[#slots + 1] = C_Container.GetContainerNumSlots(bag)
    end
    return slots
end

local function rebuildIndex(slots: number[])
    queue[#queue + 1] = tostring(#slots)
end

export function rescan()
    local slots = debug.measure("scanBags", scanBags, 0, 4)

    debug.time("rebuild")
    rebuildIndex(slots)
    debug.timeEnd("rebuild")

    debug.watch("queued", function() return #queue end)
end
```

- `debug.measure(label, fn, ...)` calls `fn(...)`, records the time under
  `label`, and returns what `fn` returns.
- `debug.time(label)` and `debug.timeEnd(label)` time the statements between
  them.
- `debug.watch(label, fn)` shows `fn()` in the watch list of the dev window,
  refreshed while the window is open. The function can read locals.

In a release build `measure` becomes a plain call, and the other three are
removed:

```lua
rescan = function()
	local slots = scanBags(0, 4)

	rebuildIndex(slots)
end
```

## Profile and trace functions

Dev builds profile every named function already. `@profile` marks a function
for builds that only profile marked ones (`"instrument": "decorated"`), or
asks for more. `@trace` logs every call. `@noprofile` keeps a function out of
the profiler.

```wowlua title=src/importer.wowlua
interface Row
    item: string
    amount: number
end

export class Importer
    count: number = 0

    @profile({ memory = true })    -- also measure the memory it allocates
    function record(amount: number)
        self.count += amount
    end

    @trace                         -- log arguments, results and errors
    function parse(line: string): Row?
        local item, amount = line:match("^(%S+) (%d+)$")
        if not item then
            return nil
        end
        return { item = item, amount = tonumber(amount) :: number }
    end

    @noprofile                     -- uses error(msg, 2), so no wrapper
    function check(v: unknown)
        if type(v) ~= "number" then
            error("expected a number", 2)
        end
    end
end
```

`@trace` writes at the `trace` log level, so it only works in builds that
compile that level in: `test` by default, not `dev` or `release`.

For each profiled function the profiler records the number of calls, the
total time, the self time (without profiled functions it called), the
slowest call and the average. Read them in the game with `/wl prof MyAddon`,
in the dev window's performance tab, or in code:

```wowlua title=src/report.wowlua
import * as debug from "wowlua:debug"

export function report()
    for name, s in pairs(debug.stats()) do
        print(name, s.calls, s.total, s.max)
    end
end
```

In a release build, `debug.stats()` is an empty table.

The dev window adds to these numbers. It adds up the time your addon takes
in each frame, and any single call longer than `slow-call` (2 ms by default)
is kept as a slow call with its stack. With call paths on, it also knows
which event, script, timer or slash command started each call, and with
garbage per function, how much garbage each function made itself. Both cost
time on every call, so they are off until you switch them on in the window
or the profile:

```json
"profiles": {
  "dev": { "profile-call-paths": true, "profile-garbage": true, "slow-call": 4 }
}
```

From these numbers the window's hints tab finds the usual problems and names
the fix, with `P` codes that `wowlua explain` explains: P7101 for an event
that comes in bursts, P7102 for an `OnUpdate` on a frame nobody sees, P7201
for a function that makes much of the garbage and P7301 for a call longer
than the frame budget.

> [!NOTE]
> A profiled or traced function runs inside a wrapper, which adds a stack
> level. Code that uses `error(msg, 2)` or `debugstack()` to point at its
> caller should be `@noprofile`. A traced function must not yield inside a
> coroutine.

## Whole functions just for you

`@debugOnly` removes a function, and every call to it, from builds without
`debug`. The function can't return a value (E3032), and you call it as a
statement.

```wowlua
local totals: { [string]: number } = {}

local function add(item: string, amount: number)
    totals[item] = (totals[item] or 0) + amount
end

@debugOnly
local function checkTotals()
    for item, amount in pairs(totals) do
        if amount < 0 then
            print("negative total for", item)
        end
    end
end

add("Linen Cloth", 20)
checkTotals()
```

The release build:

```lua
local totals = {}

local function add(item, amount)
	totals[item] = (totals[item] or 0) + amount
end

add("Linen Cloth", 20)
```

## Log at the right level

Calls of `wowlua:log` below the build's log level are removed at compile
time, with their arguments, so an expensive debug message never runs for
players.

```wowlua
import log from "wowlua:log"

local state = { queued = 3, sent = 12 }

local function serialize(t: { [string]: number }): string
    local parts: string[] = {}
    for k, v in pairs(t) do
        parts[#parts + 1] = `{k}={v}`
    end
    return table.concat(parts, ",")
end

log.trace(`state: {serialize(state)}`)   -- removed outside test builds
log.warn("cache miss for", 42)           -- kept
```

```lua
__wl_m1.emitAt("warn", nil, "cache miss for", 42)
```

In the game, `/wl log MyAddon error` raises the level for the session. It
can't go below the level the build was compiled with. More on output in
[Chat, logging and alerts](../stdlib/chat.md).

## Catch bad values at the door

Types can't see what comes from SavedVariables, other addons, plain Lua or
the game. So dev builds check the arguments of exported functions, public
methods of exported classes and event, slash and hook handlers when they are
called, and test builds those of every annotated function. A wrong value
fails with the parameter, the function and both types:

```text
MyAddon: src/tracker.wowlua:9: bad argument #1 'amount' to 'Tracker:record' (number expected, got string)
```

```wowlua
@checked
export function setScale(scale: number)
    UIParent:SetScale(scale)
end

@unchecked
export function fill(partial: { amount: number? })
    partial.amount = partial.amount or 0
end
```

- `@checked` checks a function in every build, release included.
  `@unchecked` never checks it, for example a builder that receives
  half-built objects.
- For one value you know is incomplete, cast it through `any`:
  `local f: Panel = CreateFrame("Frame") :: any` is never checked.
- Release builds check nothing else, so they run at full speed.

What each type is checked for is listed in
[Debugging §7.5](../../spec/language/07-debugging.md#75-runtime-type-checks).

## Ask a value what it is

`wowlua:reflect` answers questions about classes and enums at run time.
These functions work in every build:

```wowlua title=src/pets.wowlua
import * as reflect from "wowlua:reflect"

export enum Direction { Up, Down, Left, Right }

export class Animal
    name: string

    constructor(name: string)
        self.name = name
    end
end

export class Dog extends Animal
    function fetch()
        print(`{self.name} fetches`)
    end
end

export function inspect(v: unknown, raw: unknown)
    if reflect.isInstance(v, Dog) then
        v:fetch()                              -- v is a Dog here
    end
    print(reflect.className(v))                -- "Dog"
    print(reflect.isSubclass(Dog, Animal))     -- true
    print(reflect.enumName(Direction, 2))      -- "Down"
    print(reflect.enumFrom(Direction, raw))    -- a Direction, or nil
end
```

`classOf`, `superclass` and `enumEntries` are there too. Mark a class
`@reflect` to also list its fields, methods, properties and decorators, for
example to build an options panel or a serializer from the class:

```wowlua title=src/settings.wowlua
import * as reflect from "wowlua:reflect"

@reflect
export class Settings
    scale: number = 1
    private locked: boolean = false

    function reset()
        self.scale = 1
    end
end

export function listFields()
    for _, f in ipairs(reflect.fields(Settings) ?? {}) do
        print(f.name, f.type, f.visibility)
    end
end
```

```text
scale number public
locked boolean private
```

`reflect.fields` returns `nil` for a class without `@reflect`, unless the
profile sets `"reflection": "all"`, as `test` does. The other functions are
listed in [Debugging §7.8](../../spec/language/07-debugging.md#78-reflection).

## Where to go next

- [Debugging](../guide/debugging.md): the `/wl` dev window, breakpoints in
  VS Code, and `wowlua trace`.
- [Testing your addon](../guide/testing.md): catch a bug once, and keep it
  caught.
- [Standard library](../stdlib/overview.md): the modules for the everyday
  work.
