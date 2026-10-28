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
| Monitor | The addon's events, scripts, timers and slash commands (count, total and max time), and its memory every 5 seconds outside combat, for the last 10 minutes |
| Recordings | The time the addon takes in each frame, its slow calls with their stack, and the garbage it makes; one recording per combat and per boss pull |
| Error capture | Errors raised in the addon's folder, with stack and repeat count; BugSack and the default error frame still see every error |
| Log | The `wowlua:log` ring buffer, from `debug` level up |
| Runtime type checks | The parameters of exported functions, of public methods of exported classes, and of event, slash and hook handlers, against their declared types |

Plain `.lua` files are profiled and monitored too; embedded libraries are
not. Type `/wl` in the game to open or close the dev window. It is movable and
resizable, has a selector for every loaded wowlua dev build, and these tabs:

| Tab | Shows |
|-----|-------|
| overview | Version, profile, build id, your `wowlua:meta` values and memory over time |
| performance | The recordings, and eight views of them (below) |
| monitor | Events, scripts, timers and slash commands since load |
| log | The log ring buffer, filterable by level |
| errors | Captured errors with count, location and stack |
| watch | Watched values, live |
| inspector | A table as an expandable tree |
| hints | Performance hints, with the number of new ones on the tab |

The footer shows the addon's memory, the garbage it makes per second (and
next to it all addons' together) and the time the last frame took. The window refreshes only while it is shown, and Escape closes
it. Its position, size, tab and selected addon are saved per addon, in the
SavedVariables `WowluaDevDB_<Addon>` that dev builds add to the TOC. The
profile settings and what each one costs are in
[Debugging](../../spec/language/07-debugging.md) of the specification.

### Recordings

A recording is a slice of time. `live` covers everything since the last
reload; a recording is made for every combat and every boss pull on its own,
and you start and stop your own with the window's **Record** button or
`/wl record`. The window keeps the last 20 combats and pulls.

Pick a recording on the left, and a view on the right:

| View | Shows |
|------|-------|
| functions | Calls, total, self, max and average time per function, the garbage it made, and its last minute |
| tree | The call tree, from the event, script, timer or slash command that started the work |
| flame | The same as a flame graph; click a bar to zoom in, right-click to zoom out |
| by event | The time each event, script, timer or command took, and the function that took most of it |
| timeline | The time of every frame against the frame budget, the garbage, and the combats and pulls; click a frame to see what ran in it |
| slow | The slow calls, newest first, with their stack |
| garbage | The addon's garbage per second against all addons', memory over time, then the garbage per function |
| bench | Runs of an expression, timed |

Right-click a recording to compare the selected one with it. Every view then
shows the change per minute of recording, so a longer pull is not slower
just because it is longer.

The call tree, the flame graph and the garbage per function cost time on
every call, so they are off until you switch them on in the bar above the
views, or in the profile with `profile-call-paths` and `profile-garbage`.
The same bar sets the frame budget (`frame-budget`, 1 ms) and the time from
which a call counts as slow (`slow-call`, 2 ms). What you set there is saved
for the addon and wins over the profile; **Profile's** goes back to the
profile's values.

```json
"profiles": {
  "dev": { "profile-call-paths": true, "frame-budget": 2 }
}
```

### Hints

While the window is open, it checks the live recording every few seconds and
lists what it finds in the hints tab: an event that comes in bursts (P7101),
an `OnUpdate` that runs every frame on a frame nobody sees (P7102), a
function that makes much of the garbage (P7201) and a call longer than the
frame budget (P7301). Each hint says what it found, with the numbers, and how
to fix it; `wowlua explain P7301` explains it at length. Dismiss a hint by
clicking it; it comes back if it gets worse.

### The HUD and the error toast

`/wl hud`, or **HUD** in the window's footer, shows a small overlay with a row
per loaded dev build: the time it took in the last frame, its average, the
garbage rate and its errors. A row turns red for a second when a frame goes
over the budget, and clicking it opens the window. The HUD hides while the
window is open and during cinematics, and it comes back where you left it.

When the window is closed, a new error shows a notice at the top of the
screen for eight seconds (longer while the cursor is on it); click it to see
the error in the errors tab. In combat it waits until the combat ends. BugSack
and the default error frame still get every error.

### Exporting a recording

The dev window is small, and a recording ends with the session. To look at a
boss pull at full size, or send it to someone, export it: the recording
leaves the game through the addon's SavedVariables, and
`wowlua prof report` turns it into one HTML page on your computer.

1. **Run a dev build in the game** Build, install and log in as usual.
   `wowlua dev`
2. **Switch on call paths** Open the dev window, pick your addon in its
   selector and, in the performance tab, switch on **Call paths** for the
   call tree and the flame graph (and **Garbage per function** for the garbage
   per function). Do it before the pull: a recording only has what was on
   while it ran. `/wl`
3. **Pull the boss** The window starts a recording when the encounter starts
   and ends it when the encounter ends, win or wipe. It is listed as `pull 1`
   with the boss's name. `pull 1`
4. **Mark it and reload** Right-click the recording, choose **Mark for
   export**, then reload. WoW writes SavedVariables only on a reload or when
   you log out, and the window's footer reminds you until you do. `/reload`
5. **Open the report** In your addon's folder on your computer: it finds
   your WoW installation as `wowlua dev` does, reads the exported recordings
   and opens the page in your browser. `wowlua prof report`

The page is `dist/reports/<Name>-<date>.html` (`--out` names another file,
`--no-open` only writes it). It shows every exported recording, or the ones
you name with `--recording "pull 1"`: its hints, the functions table, the
time by event, the call tree and flame graph, the frame timeline against the
frame budget, the slow calls with their stacks and the garbage. Locations in
the stacks are mapped back to your source lines as `wowlua trace` does, with
the build each recording was made with. The page is self-contained and
nothing is uploaded.

The command looks in every client folder (`_retail_`, `_classic_`, …) and
every account under `WTF/Account/`. When more than one has exported
recordings of the addon, it lists them and asks which one; in a script, pick
one with `--client` and `--account`:

```console
$ wowlua prof report --client _retail_ --account MYACCOUNT
```

A recording stays marked for export after the reload, so the next reload
saves it again. Right-click it and choose **Unmark for export** when you no
longer need it.

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
| `/wl record <addon> [name]`, `/wl record <addon> stop` | Start or stop a recording |
| `/wl bench <addon> <expression> [n]` | Run an expression n times (10,000 by default) and print the fastest, median and slowest run |
| `/wl hints <addon>` | Print the performance hints |
| `/wl snapshot <addon>` | Record your character's world for a [world profile](testing.md#world-profiles) |
| `/wl hud` | Show or hide the HUD |
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
