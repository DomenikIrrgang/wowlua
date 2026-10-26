# Debugging

wowlua gives you three ways to find out what your addon does. In the game,
dev builds carry a profiler, an event monitor, error capture and live watches,
with no change to your code. In the emulated client, a debugger stops on
breakpoints, events and taint. And when a player sends you an error from a
release build, `wowlua trace` maps it back to your source.

## In the game: the dev window

`wowlua dev` and `wowlua build` use the `dev` profile, which turns every tool
on. The `release` profile turns them all off, so what players download
contains none of it.

| Tool | In a dev build |
|------|----------------|
| Profiler | Every named function and method: calls, total, self, max and average time |
| Monitor | The addon's events, scripts and timers (count, total and max time), and its memory every 5 seconds outside combat, for the last 10 minutes |
| Error capture | Errors raised in the addon's folder, with stack and repeat count; BugSack and the default error frame still see every error |
| Log | The `wowlua:log` ring buffer, from `debug` level up |
| Runtime type checks | The parameters of exported functions, of public methods of exported classes, and of event, slash and hook handlers, against their declared types |

Plain `.lua` files are profiled and monitored too; embedded libraries are
not. Type `/wl` in the game to open or close the dev window. It is movable and
resizable, has a selector for every loaded wowlua dev build, and these tabs:

| Tab | Shows |
|-----|-------|
| overview | Version, profile, build id, your `wowlua:meta` values and memory over time |
| profiler | The profiler table, per function |
| monitor | Events, scripts and timers since load |
| log | The log ring buffer, filterable by level |
| errors | Captured errors with count, location and stack |
| watch | Watched values, live |
| inspector | A table as an expandable tree |

The window refreshes only while it is shown, and Escape closes it. Its
position, size, tab and selected addon are saved per addon, in the
SavedVariables `WowluaDevDB_<Addon>` that dev builds add to the TOC. The
profile settings and what each one costs are in
[Debugging](../../spec/language/07-debugging.md) of the specification.

### From chat

| Command | Effect |
|---------|--------|
| `/wl` | Toggle the dev window |
| `/wl list` | List the loaded wowlua addons with version, profile and build id |
| `/wl prof <addon> [top N] [sort calls\|total\|self\|max] [reset]` | Print the profiler table |
| `/wl mon <addon> [reset]` | Print the event, script and timer monitor |
| `/wl errors <addon> [clear]` | Print the captured errors |
| `/wl log <addon> [level]` | Print the log, or set the log level at runtime |
| `/wl meta <addon>` | Print the `wowlua:meta` information |
| `/wl inspect <addon> <module>` | Print a module's exports |
| `/wl watch <addon> <expression>` | Add a watch |
| `/wl reload` | Reload the UI |

The log level can be raised at runtime (`/wl log MyAddon error`), but never
lowered below the level the build was compiled with: log calls below it are
removed at compile time.

### Watches from code

A watch is a live value with a label. Add one in the watch tab's input, from
chat with `/wl watch`, or from code. Expressions typed in the game run with
`ns` (the addon table), `modules` (each module's exports) and the globals.
File-local variables are out of their reach, so watch those from code:

```wowlua
import * as debug from "wowlua:debug"

local kills = 0
debug.watch("kills", function() return kills end)
```

`debug.watch` is removed from builds without the dev console, together with
its arguments.

## Map errors back to your source

Dev builds keep your line numbers, so an error on line 7 of
`Interface/AddOns/MyAddon/src/main.lua` is line 7 of `src/main.wowlua`.
Release builds are bundled into one file, which makes a player's error report
hard to read. `wowlua trace` maps both back to your source, with the source
maps of the build the error came from. Paste the error from BugSack or the
error frame into a file, pipe it in, or use `--clipboard`:

```console
$ wowlua trace error.txt
MyAddon/MyAddon.lua:1194: attempt to call upvalue 'chat' (a nil value)
  → src/main.wowlua:7 in onLogin
[string "@Interface/AddOns/MyAddon/MyAddon.lua"]:1194: in function <MyAddon/MyAddon.lua:1193>
  → src/main.wowlua:7 in onLogin
  → src/main.wowlua:5
```

The build id in the TOC (`X-Wowlua-Build`) picks the right maps, and
`--build <id>` names one. The maps of builds you published are archived in
`~/.wowlua/maps/`, so a report from an older version still decodes
([Tooling §16.4](../../spec/toolchain/tooling.md#164-trace-decoder-wowlua-trace)).

## In the emulator: a real debugger

`wowlua debug` runs a test file, or your addon itself, in the same emulated
client as `wowlua test`, and lets your editor stop it, step through it and
look inside. No game needs to run. With the [VS Code extension](vscode.md),
press `F5`: the extension starts `wowlua debug` and draws the game's screen
next to your code.

- **Line breakpoints** in `.wowlua` and `.lua` files, with conditions, hit
  counts and log messages (`count is {count}`).
- **Breakpoints on what the game does**: one of your functions
  (`MyAddon.db:Reset`), an API call (`C_Timer.After`, `CreateFrame`), an event
  (`event:PLAYER_LOGIN`, or `event:*`), a script (`script:OnClick`,
  `script:MyButton:OnClick`) or any timer (`timer`).
- **Stops on trouble**: errors that reach the error handler, every error, the
  moment your code becomes tainted, and a protected action the game blocks.
- **Stepping that follows the game**: step into `frame:Show()` and you are in
  its `OnShow`; step out of an event handler and you stop in the next handler
  of the same event.
- **A call stack that says why**: `event BAG_UPDATE` → `MyFrame:OnEvent` →
  your function. Variables show their taint (`string · tainted by MyAddon`),
  what a secret value hides, and a frame's type, size, points and children.
- **Watches and the console** evaluate Lua in the stopped function:
  `count = 0` sets its local.

A session runs `"test": "tests/options.test.wowlua"` (with `"filter"` for
some of its tests) or `"addon": true`: the addon, logged in, its frames
running in real time until you stop it. `"clock": "manual"` keeps time still
until asked, `"stopOnEntry": true` stops before the first line, and
`"debugBlizzardUI": true` steps into Blizzard's code too.
[VS Code](vscode.md#debug-in-the-emulated-client) lists every launch
attribute.

Other editors work too. `wowlua debug` speaks the Debug Adapter Protocol on
stdio, or on a TCP port with `--port 4711`, so Neovim's nvim-dap, Zed and
Emacs' dap-mode connect to it directly. The same debugger is open to AI
agents through [`wowlua mcp`](agents.md).

### Profiling in the emulator

The emulator's profiler counts per function, event, script, `OnUpdate` and
game frame: calls, Lua instructions run, memory allocated, and time. The
instruction counts are the same on every machine and every run, so they are
the number to compare before and after a change. The time is the emulator's
speed on your machine, not the game's. A report lists the costliest functions
and writes a [speedscope](https://www.speedscope.app) file with the call
tree.

```console
$ wowlua test --profile-out prof    # one speedscope file per test file and flavor
```

`prof/` then holds files such as
`tests_tracker.test.wowlua.camelot.speedscope.json`; open one on
speedscope.app. An agent profiles with the `profile_start` and
`profile_stop` tools of [`wowlua mcp`](agents.md), and an editor with the
debug adapter's `wowlua/profile` request
([Debugger §18.8](../../spec/toolchain/debugger.md#188-profiler)).

Screenshots of the emulated client, in tests and from the terminal, are on
[Testing your addon](testing.md#snapshots-and-screenshots).
