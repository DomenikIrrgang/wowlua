# Debugging in game

Dev builds carry debugging tools without any change to your code: a profiler,
a monitor, error capture, a log and live watches, shown in the dev window that
`/wl` opens. Release builds contain none of it. When an error comes back from
the game, `wowlua trace` maps it to your source lines.

## What a dev build includes

`wowlua dev` and `wowlua build` use the `dev` profile, which turns every tool
on. The `release` profile turns them off.

| Tool | Dev build |
|------|-----------|
| Profiler | Every named function and method: calls, total, self, max and average time |
| Monitor | The addon's events, scripts and timers (count, total and max time), and its memory every 5 seconds outside combat, for the last 10 minutes |
| Error capture | Errors raised in the addon's folder, with stack and repeat count; BugSack and the default error frame still see every error |
| Log | The `wowlua:log` ring buffer, from `debug` level up |
| Runtime type checks | The parameters of exported functions, public methods of exported classes and event, slash and hook handlers, against their declared types |

Plain `.lua` files are profiled and monitored too; embedded libraries are
not. The profile settings, their values per profile and what each costs are
in [Debugging](../spec/language/07-debugging.md).

## The dev window

Type `/wl` to open or close it. The window is movable and resizable, has an
addon selector for every loaded wowlua dev build, and these tabs:

| Tab | Shows |
|-----|-------|
| overview | Version, profile, build id, `wowlua:meta` values and memory over time |
| profiler | The profiler table per function |
| monitor | Events, scripts and timers since load |
| log | The log ring buffer, filterable by level |
| errors | Captured errors with count, location and stack |
| watch | Watched values, live |
| inspector | A table as an expandable tree |

The window refreshes only while it is shown. Escape closes it. Its position,
size, tab and selected addon are saved per addon in the SavedVariables
`WowluaDevDB_<Addon>`, which dev builds add to the TOC.

## Chat commands

| Command | Effect |
|---------|--------|
| `/wl` | Toggle the dev window |
| `/wl list` | List loaded wowlua addons with version, profile and build id |
| `/wl prof <addon> [top N] [sort calls\|total\|self\|max] [reset]` | Print the profiler table |
| `/wl mon <addon> [reset]` | Print the event, script and timer monitor |
| `/wl errors <addon> [clear]` | Print captured errors |
| `/wl log <addon> [level]` | Print the log, or set the runtime log level |
| `/wl meta <addon>` | Print `wowlua:meta` information |
| `/wl inspect <addon> <module>` | Print a module's exports |
| `/wl watch <addon> <expression>` | Add a watch |
| `/wl reload` | Reload the UI |

The runtime log level can be raised (`/wl log MyAddon error`) but never lowered
below the level the build was compiled with: log calls below it are removed at
compile time.

## Watches

A watch is a live value with a label. Add one from the watch tab's input, from
chat with `/wl watch`, or from code:

```wowlua
import * as debug from "wowlua:debug"

local kills = 0
debug.watch("kills", function() return kills end)
```

Typed expressions run with `ns` (the addon table), `modules` (each module's
exports) and the globals in scope. File-local variables are only reachable
from `debug.watch` closures. `debug.watch` is removed from builds without the
dev console, together with its arguments.

## Map errors back to your source

Dev builds keep line numbers, so an error in
`Interface/AddOns/MyAddon/src/main.lua:7` is line 7 of `src/main.wowlua`.
Release builds are bundled into one file. `wowlua trace` maps both back, using
the source maps of the matching build. Paste the error from BugSack or the
error frame into a file, or use `--clipboard`:

```sh
wowlua trace error.txt
```

```text
MyAddon/MyAddon.lua:768: attempt to call upvalue 'chat' (a nil value)
  → src/main.wowlua:7 in onLogin
[string "@Interface/AddOns/MyAddon/MyAddon.lua"]:768: in function <MyAddon/MyAddon.lua:767>
  → src/main.wowlua:7 in onLogin
  → src/main.wowlua:5
```

It reads standard input when no file is given. The build id in the TOC
(`X-Wowlua-Build`) picks the right maps; `--build <id>` names one. Maps of
published builds are archived in `~/.wowlua/maps/`, so reports from older
versions can still be decoded
([Tooling §16.4](../spec/toolchain/tooling.md#164-trace-decoder-wowlua-trace)).

## Planned

Recordings per combat and boss pull, frame-time views, performance hints, a
compact HUD and `wowlua prof report` are specified and planned for
[M11](../spec/roadmap.md#m11-performance-optimization). A debugger on the
emulated client is planned for [M12](../spec/roadmap.md#m12-debugger).
