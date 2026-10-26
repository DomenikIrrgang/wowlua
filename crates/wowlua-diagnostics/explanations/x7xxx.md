# x7xxx · debugging

## E7001

`nameof` from `wowlua:debug` folds to the last name of its argument at
compile time (`nameof(Tracker.record)` is `"record"`), so the argument must be
a name, a field access or an index with a string key. A call, a literal or an
index with a computed key has no name to give. Pass the name itself.

```wowlua fails
import nameof from "wowlua:debug"

local function onLogin() end
print(nameof(onLogin()))
```

```wowlua compiles
import nameof from "wowlua:debug"

local function onLogin() end
print(nameof(onLogin))
```

## W7001

A release build (a profile with `debug = false`) still contains `todo()` from
`wowlua:debug`. `todo` is kept in every build and raises "not implemented"
when it runs, so players would hit the error. `wowlua build --release` and
`wowlua check --release` report it; the example turns `debug` off for the dev
profile so that `wowlua check` sees it.

Write the missing code before releasing.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "debug": false } }
}
```
```wowlua fails title=src/main.wowlua
import todo from "wowlua:debug"

local function exportData(): string
    return todo("export")
end
print(exportData())
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "debug": false } }
}
```
```wowlua compiles title=src/main.wowlua
local function exportData(): string
    return "data"
end
print(exportData())
```

## W7010

Code asks `wowlua:reflect` for full metadata (`reflect.fields`,
`reflect.methods`, `reflect.functionInfo`, …), but the profile sets
`reflection = "none"`, so no metadata is emitted and the call finds none at
run time. The default, `"decorated"`, keeps it for classes, enums and
functions marked `@reflect`; `"all"` keeps it for all of them. Class identity
(`reflect.isInstance`, `reflect.className`, the enum functions) is always
available and never warns.

Set the profile's `reflection` to `"decorated"` or `"all"`, or remove the
reflection call.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "reflection": "none" } }
}
```
```wowlua fails title=src/main.wowlua
import * as reflect from "wowlua:reflect"

@reflect
class Settings
    scale: number = 1
end

print(#reflect.fields(Settings))
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "reflection": "decorated" } }
}
```
```wowlua compiles title=src/main.wowlua
import * as reflect from "wowlua:reflect"

@reflect
class Settings
    scale: number = 1
end

print(#reflect.fields(Settings))
```

## P7101

A performance hint from the dev window, not the compiler. A handler ran very
often in the recording, in bursts: the same event arrived many times within a
short time and the handler did its full work for each one. Events such as
`UNIT_AURA`, `BAG_UPDATE` or `COMBAT_LOG_EVENT_UNFILTERED` can fire dozens of
times in one frame, while the addon usually only needs the last state.

Collapse a burst into one call: decorate the function that does the work with
`@throttle(0.1)` or `@debounce(0.1)` from `wowlua:timer`, or mark it
`@nextFrame` so that many calls in one frame run it once. Keep the handler
itself cheap, and filter on the event's arguments (the unit, the bag) before
doing anything else.

Related: P7301

## P7102

A performance hint from the dev window, not the compiler. An `OnUpdate`
script ran in nearly every frame while its frame showed nothing: the frame
has no size or is fully transparent, as a frame made only to run code usually
is. `OnUpdate` runs once per rendered frame, 60 to 240 times a second, for as
long as the frame is shown, so a script that only waits or polls costs time
in every frame.

When the work is needed every few tenths of a second, use `every` or
`repeating` from `wowlua:timer` instead, which run on `C_Timer` and cost
nothing in between. When the script only matters while some UI is open, set
it in that UI's `OnShow` and clear it with `SetScript("OnUpdate", nil)` in
`OnHide`, or hide the frame that carries it.

Related: P7101

## P7201

A performance hint from the dev window, not the compiler. A single function
created a large share of the garbage the addon's functions made in the
recording. Garbage is every table, closure and new string that is created
and dropped again; the game collects it in steps, and the collection costs
frame time, for every addon.

Reuse tables instead of creating them per call (clear them with `wipe`),
move closures that capture nothing out of loops and handlers, cache results
that don't change, and build strings once instead of in every frame. The hint
needs garbage per function, which the dev window's switch or the profile's
`profile-garbage` turns on.

Related: P7301

## P7301

A performance hint from the dev window, not the compiler. A single call took
longer than the frame budget (`frame-budget`, 1 ms by default), so that frame
was late because of one piece of work. The hint names the function and the
slowest call; the slow view of the dev window has its stack.

Spread the work across frames: run it as a job of `wowlua:jobs` and call
`ctx:check()` in its loop, which yields when the frame's share of time is
used up. Work that does not need to happen at once, such as rebuilding an
index, can also wait for the end of combat. If the budget is too strict for
the addon, raise `frame-budget` in the profile or in the window.

Related: P7101, P7201
