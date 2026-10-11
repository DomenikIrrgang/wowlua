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

A dev build therefore has every tool of this chapter built in without any
change to the code (decision T96): every function is profiled, events,
scripts and timers are monitored, errors are captured, and the dev window
(§7.7) shows all of it. Instrumentation and the monitor rewrite code at
compile time; in profiles that use them, plain `.lua` files are compiled too
(line numbers preserved) instead of copied byte for byte, except embedded
libraries under `build.libs-dir` (decision T97). Release builds contain none
of it.

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
| `assert(cond, msg?)` | statement | Raises an error with message and source location if `cond` is falsy. Narrows like Lua's `assert`. | Removed (`assertions = false`) |
| `dump(expr, ...)` | statement | Prints each expression's **source text** and value: `dump(x + y)` prints `src/a.wowlua:12 x + y = 42`. Tables are printed one level deep. | Removed (`debug = false`) |
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
| Literal unions (`"LEFT" \| "RIGHT"`, enums) | Set membership |
| Class instance types | `reflect.isInstance(v, C)` |
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
| `/wl reload` | `ReloadUI()` |

Chat output of `/wl prof` and `/wl mon` is an aligned table (name, calls,
total, self, max, avg).

### 7.7.1 Dev window

`/wl` opens a movable, resizable window with an addon selector and these tabs:

| Tab | Contents |
|-----|----------|
| Overview | Version, profile, build id, `wowlua:meta` values, memory use over time |
| Profiler | The profiler statistics (§7.3), sortable by any column, with reset |
| Monitor | Events, scripts and timers (§7.9) |
| Log | The `wowlua:log` ring buffer, filterable by level |
| Errors | Captured errors with count, location and stack (§7.10) |
| Watch | Watched values (§7.7.2) |
| Inspector | A table tree view (§7.7.3) |

The window refreshes only while it is visible (default every 0.25 s for
watches and the inspector, every second for the other tabs).

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

## 7.8 Reflection

Reflection is provided by the `wowlua:reflect` module.

### 7.8.1 Always available

Class identity costs a field or two per class and is always emitted:

```wowlua
import * as reflect from "wowlua:reflect"

reflect.isInstance(v, Dog)       -- type predicate: v is Dog
reflect.classOf(dog)             -- ClassOf<Dog>? (works for widget classes via __class)
reflect.className(dog)           -- "Dog"
reflect.superclass(Dog)          -- ClassOf<Animal>?
reflect.isSubclass(Dog, Animal)  -- true
reflect.enumName(Direction, 2)   -- "Down"
reflect.enumEntries(Direction)   -- { {name = "Up", value = 1}, … } in declaration order
reflect.enumFrom(Direction, raw) -- Direction? (checked conversion)
```

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
calls of monitor helpers that install a timing wrapper around `fn`. Nothing
global is hooked, so no other addon or Blizzard frame is affected.

| Monitored | Recorded per entry |
|-----------|--------------------|
| Events (`OnEvent` handlers) | Per event name: count, total and max handler time |
| Scripts | Per frame (name, or widget class and creation location) and script: count, total, max |
| Timers | Per creation location: callbacks run, total, max |
| Memory | The addon's memory (`GetAddOnMemoryUsage`) sampled every 5 s, last 10 minutes |

Handlers set in XML files are not monitored. The data is shown in the
Monitor tab and by `/wl mon`.

## 7.10 Error capture

With `capture-errors = true`, the runtime chains itself into the global
error handler (`seterrorhandler`): errors whose stack contains the addon's
folder are recorded (message, first location, stack from `debugstack()`, time,
count of repeats), and the previous handler is always called afterwards, so
BugSack and the default error frame still see every error (decision T100).
Recorded errors are shown in the Errors tab and by `/wl errors`.

