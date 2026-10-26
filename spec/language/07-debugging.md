# 7. Debugging and Reflection

wowlua builds debugging tools into the language. Everything in this chapter is
controlled by the active **profile**
([Manifest](../toolchain/manifest.md#99-profiles)) and costs nothing when switched off.

| Profile setting | Values | `dev` | `release` | `test` |
|-----------------|--------|-------|-----------|--------|
| `debug` | bool | `true` | `false` | `true` |
| `assertions` | bool | `true` | `false` | `true` |
| `log-level` | `"trace"` `"debug"` `"info"` `"warn"` `"error"` `"off"` | `"debug"` | `"warn"` | `"trace"` |
| `instrument` | `"none"` `"decorated"` `"exports"` `"all"` | `"all"` | `"none"` | `"none"` |
| `runtime-checks` | `"none"` `"exports"` `"all"` | `"exports"` | `"none"` | `"all"` |
| `reflection` | `"none"` `"decorated"` `"all"` | `"decorated"` | `"decorated"` | `"all"` |
| `dev-console` | bool | `true` | `false` | `false` |
| `monitor` | bool | `true` | `false` | `false` |
| `capture-errors` | bool | `true` | `false` | `false` |
| `source-maps` | bool | `true` | `true` | `true` |
| `profile-call-paths` | bool | `false` | `false` | `false` |
| `profile-garbage` | bool | `false` | `false` | `false` |
| `frame-budget` | number (ms) | `1` | `1` | `1` |
| `slow-call` | number (ms) | `2` | `2` | `2` |

A dev build therefore has every tool of this chapter built in without any
change to the code (decision T96): every function is profiled, events,
scripts and timers are monitored, errors are captured, and the dev window
(§7.7) shows all of it. Instrumentation and the monitor rewrite code at
compile time; in profiles that use them, plain `.lua` files are compiled too
(line numbers preserved) instead of copied byte for byte, except embedded
libraries under `build.libs-dir` (decision T97). Release builds contain none
of it.

Two measurements cost something on every call and are therefore off by
default: call paths (`profile-call-paths`, §7.3.2) and garbage per function
(`profile-garbage`, §7.3.3). The profile settings are the starting values; the
dev window switches both at run time (§7.7.5). `frame-budget` and `slow-call`
are the starting thresholds of the frame timeline and the slow-call log
(§7.3.1); the window changes them too and saves the changes per addon
(decision T174).

## 7.1 `wowlua:debug`

A compiler-known module. Most exports are **erased with their arguments** when
the corresponding setting is off. That is stronger than an `if build.debug`
guard, because argument expressions are never evaluated.

```wowlua
import assert, dump, todo, unreachable, location, nameof, typeName from "wowlua:debug"
import * as debug from "wowlua:debug"
```

| Export | Kind | Effect | When disabled |
|--------|------|--------|---------------|
| `assert(cond, msg?)` | statement | Raises an error with message and source location if `cond` is falsy. Narrows like Lua's `assert`. A secret `cond` cannot be tested and passes. | Removed (`assertions = false`) |
| `dump(expr, ...)` | statement | Prints each expression's **source text** and value: `dump(x + y)` prints `src/a.wowlua:12 x + y = 42`. Tables are printed one level deep, secret values as `<secret>`; a table addons may not index (`canaccesstable`) prints as itself. | Removed (`debug = false`) |
| `todo(msg?)` | expression, `never` | Raises "not implemented". Warning W7001 in release builds. | Kept |
| `unreachable(msg?)` | expression, `never` | Raises "unreachable code reached". | Kept, without the message text in minified builds |
| `location()` | expression | Folds to a string literal `"src/a.wowlua:12 (onLogin)"`. | Kept (constant) |
| `nameof(expr)` | expression | Folds to the final identifier of `expr` as a string: `nameof(Tracker.record)` gives `"record"`. Survives renaming refactors. | Kept (constant) |
| `typeName(expr)` | expression | Folds to the static type as a string, e.g. `"Frame?"`. | Kept (constant) |
| `measure(label, fn, ...)` | expression | Calls `fn(...)`, records its time under `label`, and returns its results. | Replaced by a direct call `fn(...)` (`debug = false`) |
| `time(label)` / `timeEnd(label)` | statement | Manual timing spans. | Removed (`debug = false`) |
| `stats()` | expression | Returns the profiler statistics table (see §7.3). | Returns `{}` (`instrument = "none"` and `debug = false`) |
| `resetStats()` | statement | Clears profiler statistics. | Removed (`debug = false`) |
| `watch(label, fn)` | statement | Shows `fn()` live in the dev window's watch list (§7.7.2), refreshed while the window is visible. The closure may read locals. | Removed (`dev-console = false`) |
| `profile`, `trace`, `debugOnly` | decorators | Re-exported core decorators, for explicitness. | See §7.3, §7.4 |

`assert` from `wowlua:debug` deliberately shadows Lua's global `assert` when
imported under that name. Lua's `assert` remains available as `_G.assert` and
is never removed.

Exports marked *statement* can only be used as expression statements (a call
on its own line). Using one as a value (passing `dump` around, or
`local ok = assert(x)`) is error E3045, because the call may be removed.
Importing the re-exported decorators `profile`, `trace` and `debugOnly`
produces no warning: they are the same core decorators
([Decorators §4.2](04-decorators.md#42-name-resolution)).

## 7.2 Logging

Logging lives in the standard library (`wowlua:log`, see
[Standard library](../stdlib/README.md)). The language-level guarantee is:
**log calls below the profile's `log-level` are removed at compile time, with
their arguments.**

```wowlua
import log from "wowlua:log"

log.trace(`state: {serialize(state)}`)   -- release (log-level "warn"): removed, serialize() never runs
log.warn("cache miss for", id)          -- kept
```

At runtime, the level can be raised further (`/wl log MyAddon error`) but never
lowered below the compiled level.

## 7.3 Profiling

`@profile` and the `instrument` setting wrap functions to collect timing data.

| `instrument` | Instrumented functions |
|--------------|------------------------|
| `"none"` | None, not even `@profile`-decorated ones |
| `"decorated"` | Functions, methods and classes marked `@profile` |
| `"exports"` | `@profile` plus every exported function and every public method of exported classes |
| `"all"` | Every named function and method, except `@noprofile` ones |

Collected per function:

| Metric | Meaning |
|--------|---------|
| `calls` | Number of calls |
| `total` | Inclusive time in milliseconds (`debugprofilestop()` resolution) |
| `self` | Exclusive time: total minus time spent in instrumented callees |
| `max` | Slowest single call |
| `avg` | `total / calls` |
| `errors` | Calls that raised an error |
| `memory` | KiB allocated (only with `@profile({ memory = true })`, using `collectgarbage("count")` deltas; expensive) |

The `errors` metric is collected only when error tracking is enabled, with
`@profile({ errors = true })` or the profile setting `profile-errors = true`
(default `false`).

**Lowering.** The function is wrapped after its definition (on its `end` line):

```wowlua
class Tracker
    @profile
    function record(amount: number) … end
end
```
```lua
function Tracker.record(self, amount) … end; Tracker.record = __wl.prof.wrap(Tracker.record, "Tracker:record", "src/tracker.wowlua:14")
```

`__wl.prof.wrap` records the start time, calls the function, and records the
result through a vararg helper, so every return value is passed through
unchanged. Recursive calls of a `local function` go through the wrapper because
the local is reassigned (as for decorators).

A **named function** is a `function` statement (`function M.f()`,
`function M:f()`, `function f()`), a `local function`, a method or
constructor, and a function literal assigned to one name or field
(`local f = function … end`, `M.f = function … end`). Statistics are keyed by
the function's name as written: `Tracker:record` for methods, `Tracker.new`
for constructors, `M.f`, `f`. Wrappers are added after decorators run on the
function's own value, so a decorator wraps the profiled function. The
standard library's own modules are never instrumented.

**Limits of wrappers** (profiling and tracing):

- **Tail calls.** A wrapped function's `return f(...)` is no longer a tail call
  from the caller's perspective. Deep tail recursion through wrapped functions
  uses stack space.
- **Stack levels.** The wrapper is one more stack frame: `error(msg, 2)`,
  `getfenv(2)` and `debugstack()` inside a wrapped function see the wrapper
  where they would see the caller. Mark such functions `@noprofile`, or use a
  profile with `instrument = "decorated"`.
- **Error tracking** needs `xpcall`. Lua 5.1's `xpcall` cannot pass arguments,
  so each call allocates a closure, which is why tracking is opt-in. The error is
  re-raised with its original message. The original stack trace is captured with
  `debugstack()` in the handler and passed to the error handler, but the
  re-raised error's own stack starts at the wrapper.
- **Coroutines.** Lua 5.1 cannot yield across `xpcall`. A function that yields
  (directly or through callees) must not have error tracking enabled. Without
  error tracking, wrapped functions may yield normally, but the time spent
  suspended is counted as call time.

### 7.3.1 Frame time and slow calls

In dev builds (`dev-console`) with `instrument` other than `"none"` or with
`monitor = true`, the runtime also measures time **per rendered frame**: the time of every
top-level instrumented call and monitored handler (one that no other measured
call is running under) is added to the current frame, and the frame is closed
by the runtime's `OnUpdate` mark. The last frames of the live recording are
kept in a ring buffer (at least 60 seconds at 60 fps). The frame timeline
(§7.7.4) and the HUD (§7.7.8) read it.

A single call that takes longer than the slow-call threshold (`slow-call`,
default 2 ms) is a **slow call**. It is recorded with its time, function and
location, the root that started the work (§7.3.2) and a stack from
`debugstack()`. Only calls over the threshold pay for the stack. The last 200
slow calls per addon are kept (decision T178). A frame whose total exceeds
`frame-budget` (default 1 ms) is **over budget**, and so is a single call
longer than it. A call that ends in a later frame than it started (a
coroutine yielded inside it) counts its time but is neither a slow call nor
over budget, because the time includes the frames it was suspended.

### 7.3.2 Call paths

With call paths switched on (`profile-call-paths`, or the window's switch),
the profiler also keys its statistics by **path**: the chain of instrumented
functions from a **root** to the function. A root is the monitored handler
that started the work (an event, a script, a timer or a slash command, §7.9)
when no other measured call was running; work that started elsewhere has the
root `other`. Paths feed the call tree,
the flame graph and the by-event view (§7.7.4). Each path costs one statistics
entry, and every call pays a path lookup, which is why it is opt-in.

### 7.3.3 Garbage

The runtime samples `collectgarbage("count")` once per frame and reports the
addon-wide **garbage rate** (KB created per second, collections removed). With
garbage per function switched on (`profile-garbage`, or the window's switch),
every instrumented call records its allocation, as `@profile({ memory = true })`
does for one function (§7.3): all of it, and its **own** part without the
instrumented callees' (as `self` is for time). `collectgarbage("count")` measures the whole
Lua state, so the figures are exact only for work that runs synchronously
inside the call; a collection step during a call lowers the count and is
recorded as zero.

**Reading the data.** In game, through the dev console (`/wl prof MyAddon`), or
in code through `debug.stats()`:

```wowlua
for name, s in pairs(debug.stats()) do
    print(name, s.calls, s.total, s.max)
end
```

## 7.4 Tracing

`@trace` logs function entry with arguments, exit with return values, and errors,
at `trace` level. Output is indented by call depth. Large tables are summarized.
`@trace({ args = false })` logs calls without arguments. Tracing is removed
entirely when the profile's `log-level` is above `trace`.

The lines go through `wowlua:log` when the addon uses it (into its ring
buffer and sinks), otherwise to `print` with the addon's name. The wrapper
calls the function with `pcall` to see errors, so a traced function must not
yield (Lua 5.1 cannot yield across `pcall`); the error is re-raised unchanged.

## 7.5 Runtime type checks

Static types cannot see values coming from the WoW API, SavedVariables, other
addons or plain Lua. In profiles with `runtime-checks` enabled, the compiler
inserts checks at function entry for **checkable** parameter types:

| Checkable type | Check |
|----------------|-------|
| `nil`, `boolean`, `number`, `string`, `table`, `function`, `thread`, `userdata` | `type(v) == "…"` |
| `T?` | `v == nil or check(T)` |
| Literal unions (`"LEFT" \| "RIGHT"`, enums) | Set membership: a comparison per literal, or for more than 8 a lookup in a set the file makes once (`EventName`, decision T286) |
| Class instance types | `reflect.isInstance(v, C)` |
| The game's object types (frames, `DurationObject`, `ColorMixin`) | `type(v)` is `"table"` or `"userdata"` (the engine's objects can be either, decision T277) |
| Arrays `T[]` | `type(v) == "table"` (elements are not checked, to keep cost bounded) |
| Records, interfaces | `type(v) == "table"` plus presence and primitive type of each required field (one level) |
| `any`, `unknown`, generics, function signatures | Not checked |

| `runtime-checks` | Functions checked |
|------------------|-------------------|
| `"none"` | None, except `@checked` |
| `"exports"` | Exported functions, public methods of exported classes, `@global` functions, event/slash/hook handlers, and `@checked` |
| `"all"` | Every annotated function, plus `::` assertions with checkable types, except `@unchecked` |

Details:

- Checks run **after default parameter values are applied**. A parameter with a
  default is checked against `T`, not `T?`.
- A variadic parameter `...: T` is checked value by value (`select("#", ...)`
  values).
- A parameter hidden by a later one of the same name (`_: number, _: any`)
  is not checked: in Lua the name reads the later one's value, and the body
  cannot read the hidden one (decision T262). Calls are still checked
  statically.
- For possibly secret types (`T | Secret<T>`), only `type()`-based checks are
  emitted, because set-membership and field checks would index the value
  ([Types §2.11](02-types.md#211-secret-values)).

A failed check raises:

```
MyAddon: src/tracker.wowlua:14: bad argument #1 'amount' to 'Tracker:record' (number expected, got string)
```

Arguments are numbered from the first declared parameter (a method's `self`
is not counted), values of `...` after the declared ones. A failed `::`
assertion raises `MyAddon: src/a.wowlua:9: bad value for '::' (number expected,
got nil)`. A class instance check needs the class in scope in that file;
where only its type is imported, the check is `type(v) == "table"`. The
standard library's own functions are never checked.

Opting out where a value is knowingly incomplete (a frame whose mixin
methods are added after it is created, for example):

- **Unchecked cast.** A local's annotation is never checked, and `:: any`
  is not checkable, so `local f: Panel = CreateFrame("Frame") :: any` is a
  cast without a runtime check, also under `"all"`. Inline, `(e :: any)`
  passes anywhere.
- **`@unchecked`** on a function exempts its parameters, e.g. a builder that
  takes half-built objects.

Both keep the rest of the profile's checks, unlike lowering
`runtime-checks` for the whole build.

## 7.6 Error locations

- **Dev builds** emit one Lua file per source file with **line numbers
  preserved**. An in-game error such as
  `Interface/AddOns/MyAddon/src/core/tracker.lua:14` points at line 14 of
  `src/core/tracker.wowlua`, with no extra tooling. In split flavor builds the
  path may contain a flavor directory, which `wowlua trace` strips.
- **Bundled or minified builds** emit source maps (written outside the AddOn
  folder and never packaged; see
  [Code generation §14.6](../toolchain/codegen.md#146-source-maps)). `wowlua trace` maps an error or stack pasted from BugSack or the
  default error frame back to source locations
  ([Developer tooling](../toolchain/tooling.md#164-trace-decoder-wowlua-trace)).
- The runtime's error helper (`__wl.error`) prefixes messages with the addon
  name, so reports always identify the addon.

## 7.7 Dev console

With `dev-console = true`, the build includes a small in-game console shared by
all wowlua dev builds that are loaded. Its slash command is `/wl`. It creates the
globals `WOWLUA_DEV`, the slash registration and the dev window
`WowluaDevWindow`, only in dev builds (see the normative list in
[Code generation §14.1.1](../toolchain/codegen.md#1411-global-writes)).

Every loaded dev build registers itself in the `WOWLUA_DEV` registry. The
console of the **newest** runtime version among them serves `/wl` and the
window; older addons still answer through the registry's versioned interface
(decision T98).

| Command | Effect |
|---------|--------|
| `/wl` | Toggle the dev window |
| `/wl list` | List loaded wowlua addons with version, profile and build id |
| `/wl errors <addon> [clear]` | Captured errors (§7.10) |
| `/wl mon <addon> [reset]` | Event, script and timer monitor report (§7.9) |
| `/wl watch <addon> <expression>` | Add a watch (§7.7.2) |
| `/wl prof <addon> [top N] [sort calls\|total\|self\|max] [reset]` | Profiler report |
| `/wl log <addon> [level]` | Show or set the runtime log level. Also opens the log ring buffer. |
| `/wl meta <addon>` | Print `wowlua:meta` information |
| `/wl inspect <addon> <module>` | Print a module's exports |
| `/wl record <addon> [name]` / `/wl record <addon> stop` | Start or stop a manual recording (§7.7.4) |
| `/wl bench <addon> <expression> [n]` | Benchmark an expression (§7.7.6) |
| `/wl hints <addon>` | Print the current performance hints (§7.7.7) |
| `/wl hud` | Toggle the compact HUD (§7.7.8) |
| `/wl reload` | `ReloadUI()` |

Chat output of `/wl prof` and `/wl mon` is an aligned table (name, calls,
total, self, max, avg).

### 7.7.1 Dev window

`/wl` opens a movable, resizable window (880 × 640 by default) with an addon
selector and these tabs (decision T170):

| Tab | Contents |
|-----|----------|
| overview | Version, profile, build id, `wowlua:meta` values, memory use over time |
| performance | Recordings and eight views of the profiler data (§7.7.4) |
| monitor | Events, scripts and timers since load (§7.9) |
| log | The `wowlua:log` ring buffer, filterable by level |
| errors | Captured errors with count, location and stack (§7.10) |
| watch | Watched values (§7.7.2) |
| inspector | A table tree view (§7.7.3) |
| hints | Performance hints with a count on the tab (§7.7.7) |

The footer shows the addon's memory, its garbage rate and the last frame's
time, the HUD switch (§7.7.8) and, while recordings are marked for export,
the reminder that `/reload` saves them.

The window refreshes only while it is visible (default every 0.25 s for
watches and the inspector, every second for the other tabs). It follows the
wowlua design system (dark, flat textures, 1px lines); dev builds ship its
fonts (Martian Mono for titles, IBM Plex Mono for everything else, SIL Open
Font License, listed in `LICENSES.txt`) in `wowlua/fonts/` (decision T171,
which replaces T102). The window uses only glyphs the font files
contain; the client draws a missing glyph as an empty box. Its position, size,
tab, addon, thresholds and switches are saved in the SavedVariables
`WowluaDevDB_<Addon>`, which dev builds add to the TOC (decision T103).
Escape closes it.

### 7.7.2 Watches

A watch is a live value with a label. Watches come from:

- code: `debug.watch(label, fn)` (§7.1);
- the window: an expression typed into the Watch tab's input;
- chat: `/wl watch <addon> <expression>`;
- other values: right-clicking a value in the Watch or Inspector tab offers
  *Watch* (adds the value's path as a new watch) and *Inspect*.

Typed expressions are Lua expressions compiled with `loadstring` and run in
an environment where `ns` is the addon table, `modules` maps module IDs to
their export tables, and every other name is a global. File-local variables
are not reachable this way; `debug.watch` closures can read them. An
expression that raises shows its error as the value. Secret values show as
`<secret>`.

### 7.7.3 Inspector

The Inspector tab shows a table as an expandable tree (keys sorted, cycles
marked, metatables shown as a `<metatable>` node), refreshed live. A watch
value, `/wl inspect <addon> <module>` or *Inspect* on a value opens it.

### 7.7.4 Performance

The performance tab lists **recordings** on the left and shows the selected
one in one of eight **views** on the right.

A recording is a time slice of the profiler (§7.3), monitor (§7.9), frame
time and garbage data (decision T172):

| Recording | Covers |
|-----------|--------|
| `live` | Everything since the last reload. Always first, never dropped. |
| `combat` | One combat, from `PLAYER_REGEN_DISABLED` to `PLAYER_REGEN_ENABLED`. Made automatically. |
| `pull N` | One boss pull, from `ENCOUNTER_START` to `ENCOUNTER_END`, numbered per encounter in the session. Made automatically. |
| manual | From `record` to `stop` (or `/wl record`), named when stopped. |

The last 20 automatic recordings per addon are kept; an older one is dropped
when a new one ends. Manual recordings and recordings marked for export are
never dropped automatically (and an automatic one marked for export does not
count toward the 20). Recordings live in memory and end with the
session, except those marked for export (§7.7.9).

| View | Shows |
|------|-------|
| functions | The profiler table: calls, total, self, max, the garbage column when garbage per function is on, and a 60-second trend per function; filter and reset |
| tree | The call tree, rooted at the event, script, timer or slash command that started the work (needs call paths) |
| flame | The same paths as a flame graph by total time, zoomable (needs call paths) |
| by event | Time per root, with the function that took most of it |
| timeline | Time per rendered frame against the frame budget, the garbage rate, and the recording's combats and pulls; selecting a frame lists what ran in it |
| slow | The slow calls (§7.3.1), newest first, with root, recording and stack |
| garbage | The garbage rate and memory over time, then garbage per function (needs garbage per function) |
| bench | Benchmarks (§7.7.6) |

**Comparing.** A second recording can be selected for comparison. Every view
then shows the difference of the selected recording against the compared
one. Totals are compared **per minute of recording**, so recordings of
different lengths compare fairly (decision T173). A difference within ±1% is
shown as unchanged.

### 7.7.5 Switches and thresholds

The window switches call paths and garbage per function on and off at run
time, for the addon shown, starting from the profile's `profile-call-paths`
and `profile-garbage`. A view that needs a switch that is off says so and
offers it. The frame budget and the slow-call threshold start from the
profile's `frame-budget` and `slow-call`. Changes made in the window are saved
per addon in `WowluaDevDB_<Addon>` and win over the profile until reset
(decision T174).

### 7.7.6 Bench

The bench view (or `/wl bench <addon> <expression> [n]`) runs an expression
n times (default 10000) and reports min, median, max and total. Expressions
follow the rules of watch expressions (§7.7.2): they are compiled once with
`loadstring`, run with `ns`, `modules` and globals in scope, and time each call
with `debugprofilestop()`. A run longer than 50 ms is split into chunks of at
most 50 ms per frame, so the game never freezes; the statistics cover all
chunks. Bench does not run in combat. The last 20 runs per addon are kept for
the session (decision T178).

### 7.7.7 Hints

While the window is open, the runtime checks the live recording every few
seconds against a set of rules and lists its findings in the hints tab, with
the count of new hints on the tab. Each hint has a `P` code (`hint[P7301]`),
names its subject, states the evidence with numbers and the recording it came
from, and suggests a fix that uses the standard library or plain Lua.
`wowlua explain P7301` explains it (decision T176). The codes are listed in
[Diagnostics §B.4](../appendix/diagnostics.md#b4-performance-hints). Hints
can be dismissed; a dismissed hint comes back when its finding gets worse:
when the number it is measured by has grown by half. A hint is new until the
hints tab has shown it. The rules run only while the window is shown (every
5 seconds), and `/wl hints <addon>` runs them once. The rules:

| Code | Finds | Measured by |
|------|-------|-------------|
| P7101 | An event handled 5 or more times within one frame, in 10 or more frames of the recording | Such frames per second |
| P7102 | An `OnUpdate` script that ran in 90% or more of the recording's frames, over 10 seconds or more, while its frame shows nothing: no size and no children or regions, or an effective alpha of 0 (a script only runs while its frame is shown, so "hidden" means hidden from the player) | Its time per second |
| P7201 | With garbage per function: a function whose own garbage is a quarter or more of the garbage the addon's functions made, and 100 KB or more | Its own garbage per second |
| P7301 | A function, event, script, timer or slash command with a call over the frame budget | Its slowest call |

Exported recordings keep the hints of their data (§7.7.9).

### 7.7.8 HUD

The HUD is a small movable overlay with one row per loaded wowlua dev build:
the addon, the time it used in the last frame, its average, its garbage rate
and its error count. A row is highlighted for a second when a frame goes over
the frame budget; clicking a row opens the window on that addon. `/wl hud`
or the window's switch shows and hides it; the choice and its position are
saved. It hides itself while the window is open and during cinematics
(decision T177). It reads the frame-time ring buffers through the
`WOWLUA_DEV` registry and samples nothing itself.

### 7.7.9 Export

Recordings can be marked for export in the window. Marked recordings are
written to the addon's `WowluaDevDB_<Addon>` SavedVariables, which WoW saves
on `/reload` and on logout, so the window reminds of a pending `/reload`.
On the desktop, `wowlua prof report` reads them and writes a report
([Developer tooling §16.5](../toolchain/tooling.md#165-profiling-reports-wowlua-prof-report),
decision T179). An exported recording keeps its statistics, call paths,
frame times (bucketed) and slow calls; stacks are kept as text.

`WowluaDevDB_<Addon>.exports` is an array with a table per exported
recording (the same while it is shown in the window):

| Field | Contents |
|-------|----------|
| `format` | `1` |
| `addon`, `version`, `profile`, `build`, `compiler` | The build that recorded it; `build` maps locations back (§7.6) |
| `name`, `kind`, `detail` | Its name, `"live"`, `"combat"`, `"pull"` or `"manual"`, and a pull's encounter name |
| `started`, `duration` | Its start as `time()` (Unix seconds) and its length in seconds |
| `frameBudget`, `slowCall`, `callPaths`, `garbage` | The thresholds (ms) and switches |
| `frames`, `time`, `over`, `garbageAll`, `garbageFns` | Frames rendered, measured time (ms), frames over budget, garbage sampled per frame and the functions' own garbage (KB) |
| `functions` | By function: `calls`, `total`, `self`, `max`, `errors`, `over` (calls over budget), `garbage`, `selfGarbage` (KB, with garbage per function), `location` |
| `paths` | With call paths: an array of `{ path = { root, name, … }, calls, total, self, max }` |
| `roots` | By root: `kind` (`event`, `script`, `timer`, `slash`, `other`), `calls`, `total`, `max`, `top` (the function with the most own time under it) and `topSelf` |
| `events`, `scripts`, `timers`, `commands` | The monitor's entries: `count`, `total`, `max`, `over`, `detail` (where it was set), `frames` (frames it ran in); events also `bursts` and `burstMax`, scripts `script` and for `OnUpdate` `showsNothing` |
| `seconds` | Per second of the recording, arrays `frames`, `time`, `max` (slowest frame), `over` and `garbage` |
| `memory` | An array of `{ at, kb }` (seconds from the start) |
| `slow` | The slow calls, newest first: `{ at, ms, name, location, root, recording, stack }` |
| `hints` | The hints of the recording: `{ code, subject, evidence, fix, recording, metric }` |

Times are milliseconds unless named otherwise.

## 7.8 Reflection

Reflection is provided by the `wowlua:reflect` module.

### 7.8.1 Always available

Class identity costs a field or two per class and is always emitted:

```wowlua
import * as reflect from "wowlua:reflect"

reflect.isInstance(v, Dog)       -- type predicate: v is Dog (narrows v)
reflect.classOf(dog)             -- ClassOf<Dog>? (works for widget classes via __class)
reflect.className(dog)           -- "Dog"
reflect.superclass(Dog)          -- ClassOf<Animal>?
reflect.isSubclass(Dog, Animal)  -- true
reflect.enumName(Direction, 2)   -- "Down"
reflect.enumEntries(Direction)   -- { {name = "Up", value = 1}, … } in declaration order
reflect.enumFrom(Direction, raw) -- Direction? (checked conversion)
```

A secret (or a table addons may not index) has no class and is no enum
member: these return nil, false or an empty list, as the full-metadata
functions (§7.8.2) do for it.

### 7.8.2 Full metadata

Full metadata is emitted for classes, enums and functions marked `@reflect`, or
for all of them when the profile sets `reflection = "all"`:

```wowlua
@reflect
class Settings
    @range(0.5, 2) scale: number = 1     -- `range`: a user field decorator in scope
    private locked: boolean = false
    function reset() … end
end

for _, f in ipairs(reflect.fields(Settings)) do
    print(f.name, f.type, f.visibility, f.optional, f.static, f.readonly)
end
-- scale    number   public  false false false
-- locked   boolean  private false false false
```

| Function | Returns |
|----------|---------|
| `reflect.fields(C)` | `FieldInfo[]`: name, type (string), visibility, optional, static, readonly, hasDefault, decorators |
| `reflect.methods(C)` | `MethodInfo[]`: name, params (`{name, type, optional}[]`), returns (string), visibility, static, abstract, overloads (signature strings), decorators |
| `reflect.properties(C)` | `PropertyInfo[]`: name, type, visibility, static, hasGetter, hasSetter, decorators |
| `reflect.decorators(C, member?)` | `DecoratorInfo[]`: name and arguments (only compile-time constant arguments are recorded; others appear as `"<expr>"`) |
| `reflect.interfaces(C)` | Names of interfaces in `implements` |
| `reflect.functionInfo(fn)` | `FunctionInfo?`: name, source location, params, returns. Only for `@reflect` functions. |
| `reflect.getClass(name)` | Look up a reflected class by name in this addon |
| `reflect.classes()` | All reflected classes in this addon |

Calling a full-metadata function on a class without metadata returns `nil`.
With `reflection = "none"`, it is warning W7010 at compile time.

Metadata is stored in a weak-keyed registry in the runtime, not on the class
table, so class tables contain only their members. Type strings use wowlua type
syntax (`"string[]"`, `"Frame?"`).

## 7.9 Monitor

With `monitor = true`, the compiler routes the addon's own handler
installations through the runtime's monitor (decision T99):
`frame:SetScript(name, fn)`, `frame:HookScript(name, fn)`, `C_Timer.After`,
`C_Timer.NewTimer` and `C_Timer.NewTicker` calls in the addon's sources become
calls of monitor helpers that install a timing wrapper around `fn`, and so do
assignments of a slash command's handler (`SlashCmdList.KEY = fn`,
`SlashCmdList[key] = fn`). The standard library's modules are routed as
well, since their frames dispatch the addon's `@event` handlers, timers and
jobs; wowlua library dependencies are not. Nothing global is hooked, so no
other addon or Blizzard frame is affected.

| Monitored | Recorded per entry |
|-----------|--------------------|
| Events (`OnEvent` handlers) | Per event name: count, total and max handler time |
| Scripts | Per frame (name, or widget class and creation location) and script: count, total, max |
| Timers | Per creation location: callbacks run, total, max |
| Slash commands | Per command (`/myaddon`): runs, total, max |
| Memory | The addon's memory (`GetAddOnMemoryUsage`) sampled every 5 s, last 10 minutes |

Handlers set in XML files, and calls through a local alias of `C_Timer`,
are not monitored. Memory is not sampled in combat
(`UpdateAddOnMemoryUsage` is expensive). The data is shown in the Monitor tab
and by `/wl mon`.

## 7.10 Error capture

With `capture-errors = true`, the runtime chains itself into the global
error handler (`seterrorhandler`): errors raised in the addon's folder are
recorded (message, stack from `debugstack()`, time, count of repeats), and the
previous handler is always called afterwards, so BugSack and the default error
frame still see every error (decision T100). The location at the start of
the message decides whose error it is; a message without one is the addon's
when its stack passes through the addon's folder. When BugGrabber is loaded
(it keeps the handler to itself), the runtime listens to its
`BugGrabber_BugGrabbed` callback instead. Paths are shown relative to the
AddOn folder. The last 50 distinct errors are kept. Recorded errors are shown
in the Errors tab and by `/wl errors`.

**Error toast.** In dev builds with `dev-console = true`, a newly captured
error also shows a small notice at the top of the screen while the dev window
is closed: the addon, the count of new errors, the first line of the message
with its location, and an action that opens the errors tab with that error
expanded. Errors within two seconds collapse into one notice. It hides after
eight seconds (the timer pauses while the cursor is over it), when clicked,
or when the window opens. In combat it waits until combat ends. It makes no
sound. BugSack and the default error frame still receive every error
(decision T180).

