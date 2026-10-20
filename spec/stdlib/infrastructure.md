# 8.6 Standard Library: Infrastructure

Cross-cutting modules that other standard-library modules build on.

## errors

`wowlua:errors` defines the **uniform error policy** of the standard library,
and exposes it for user code. The policy is normative: every stdlib module
follows it.

### Policy

1. **Isolation.** Every function the standard library calls on behalf of user
   code runs isolated. That covers event and message handlers, lifecycle hooks,
   timers, hooks, tooltip handlers, comm handlers, callback-registry handlers,
   job bodies, and menu, dialog, list and settings callbacks. An error in one
   never prevents other handlers from running, and never breaks the stdlib's
   own state.
2. **Mechanism.** Isolation uses `xpcall` with the policy's own handler, in
   every flavor. The client's `xpcall` passes the arguments on (unlike Lua
   5.1's; a recorded client answer, and Blizzard's `ExecuteFrameScript` in
   `FunctionUtil.lua` relies on it in every flavor), so a call that succeeds
   allocates nothing: no closure, no table of arguments, the context of the
   innermost call kept for the one handler. Where `xpcall` passes none, a
   closure carries them. `securecallfunction` is not used: it hands errors to
   `geterrorhandler()` directly, which would bypass the prefix, the rate limit
   and the listeners below. Job bodies run in coroutines, so their errors are
   taken from `coroutine.resume`'s results with the coroutine's stack
   (`debugstack(co)`).
3. **Reporting.** Errors go to `geterrorhandler()`, so the default error frame
   and error-collecting addons (BugSack/BugGrabber) capture them. Messages are
   prefixed `"<AddonName>: "`. In debug builds, the module ID and the stdlib
   component (`events: PLAYER_LOGIN handler`) are appended.
4. **Rate limiting.** An identical message from the same source is reported at
   most once per `errors.configure({ rateLimit = 10 })` seconds. Suppressed
   repeats are counted and reported with the next report ("… (12 more)").
5. **Lifecycle hooks.** If an `@onLoad`, `@onLogin`, `@onEnterWorld` or
   `@onLogout` hook fails, the error is reported, the **remaining hooks of that
   phase still run**, and the phase completes.
6. **Saved-variable migrations.** If a `wowlua:saved` migration fails, the error
   is reported and the SavedVariable is left exactly as loaded. The binding
   enters a *failed* state: every access raises
   `"<Name> failed to migrate to version N; data preserved"`, and default
   stripping at logout is skipped, so the stored data is never overwritten by a
   half-migrated table.
7. **No loops.** Errors raised inside `errors.onError` listeners are passed
   straight to `geterrorhandler()` and are not re-dispatched.

### API

```wowlua
import * as errors from "wowlua:errors"

local safeRefresh = errors.wrap(refresh)          -- isolated version of a function
local ok, result = errors.call(compute, 1, 2)    -- (boolean, ...results)
errors.report("unexpected state", { module = "sync" })
errors.onError(function(msg: string) statusIcon:SetVertexColor(1, 0, 0) end)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `call` | `function call<A..., R...>(fn: (A...) -> R..., ...: A...): (boolean, R...)` | Isolated call. On error, it reports and returns `false`. |
| `wrap` | `function wrap<A...>(fn: (A...) -> ()): (A...) -> ()` | Returns an isolated function. The return values of `fn` are discarded. |
| `report` | `(err: any, context: { [string]: any }?)` | Reports through the policy. Non-string errors are converted with `tostring`. |
| `onError` | `(fn: (msg: string) -> ()) -> Subscription` | Addon-level listener, e.g. to show a warning in the UI |
| `configure` | `(opts: { rateLimit: number? })` | |
| `count` | `() -> number` | Errors reported by this addon since load |

## callbacks

`wowlua:callbacks` provides typed publish/subscribe on objects, in the spirit
of CallbackHandler, plus a way to offer a public callback API to other addons.

```wowlua
import * as callbacks from "wowlua:callbacks"

interface TrackerEvents
    KillRecorded: (count: number) -> ()
    Reset: () -> ()
end

export class Tracker
    readonly events: callbacks.Registry<TrackerEvents> = callbacks.registry()
    private kills: number = 0

    function record()
        self.kills += 1
        self.events:fire("KillRecorded", self.kills)
    end
end

local sub = tracker.events:on("KillRecorded", function(count: number) label:SetText(count) end, owner)
tracker.events:offAll(owner)
```

| API | Declared signature | Notes |
|-----|--------------------|-------|
| `registry` | `function registry<E>(opts: RegistryOptions?): Registry<E>` | `E` is inferred from the expected type (the field annotation above) |
| `Registry<E>:on` | `(name, handler, owner?) -> Subscription` | `name` must be a literal key of `E` (E8420). `handler` is checked against `E[name]`. |
| `Registry<E>:once` | same | |
| `Registry<E>:off` / `offAll(owner)` | | |
| `Registry<E>:fire` | `(name, ...args)` | Arguments are checked against `E[name]`'s parameters |
| `Registry<E>:has` | `(name) -> boolean` | Whether any handler is subscribed |
| `RegistryOptions` | `{ compat: "CallbackHandler"? }` | Also provides CallbackHandler-style methods (`RegisterCallback(owner, name, methodOrFn)`, `UnregisterCallback`, `UnregisterAllCallbacks`) for plain Lua consumers |

Typing: the language has no indexed-access types, so `on`, `once` and `fire`
are `@intrinsic`-checked. The compiler resolves `E[name]` for literal names and
checks handlers and arguments with the normal assignability rules (E2001 for
mismatches).

Semantics:

- Handlers run in subscription order through the error policy. `fire` iterates a
  snapshot, so subscribing or unsubscribing during `fire` takes effect for the
  next `fire`.
- Owners are held in a weak-keyed table, so a registry never keeps an owner
  alive by itself. Lua 5.1 has no ephemerons, though: a handler closure that
  references its owner keeps the owner alive. Call `offAll(owner)` explicitly
  when an owner is discarded.
- **Public API for other addons:** a registry becomes reachable from other
  addons only by exporting it through the sanctioned global mechanism, e.g.
  `@global("MyAddonAPI")` on a table that contains it (with `compat =
  "CallbackHandler"` for plain-Lua consumers). This works on every client
  (decision S12). `C_AddOns.GetAddOnLocalTable` is not used for this.

## jobs

`wowlua:jobs` provides background jobs. They spread heavy work across frames
using coroutines and a per-frame time budget, which avoids frame drops and
the client's "script ran too long" watchdog.

```wowlua
import * as jobs from "wowlua:jobs"

local job = jobs.start(function(ctx: JobContext)
    for i, entry in ipairs(entries) do
        index(entry)
        ctx:progress(i / #entries)
        ctx:check()                         -- yields if this frame's budget is used up
    end
    return #entries
end, {
    name = "rebuild index",
    priority = "low",
    pauseInCombat = true,
    onDone = function(count: number) print("indexed", count) end,
})

job:cancel()
```

| API | Declared signature | Notes |
|-----|--------------------|-------|
| `start` | `function start(fn: (ctx: JobContext) -> ...any, opts: JobOptions?): Job` | Starts on the next frame |
| `JobOptions` | `{ name: string?, priority: ("high" \| "normal" \| "low")?, pauseInCombat: boolean?, onDone: ((...any) -> ())?, onError: ((err: string) -> ())?, onProgress: ((p: number) -> ())? }` | |
| `Job` | `cancel()`, `status(): "pending" \| "running" \| "paused" \| "done" \| "failed" \| "cancelled"`, `progress(): number`, `result(): ...any` | The function's results go to `onDone` and `result()` untyped (typed result packs may follow). |
| `JobContext:check` | `()` | Yields only if the budget is exhausted. Cheap to call often. |
| `JobContext:yield` | `()` | Always yields until the next frame |
| `JobContext:sleep` | `(seconds: number)` | |
| `JobContext:waitFor` | `(event: EventName \| MessageName) -> ...any` | Resumes with the event's payload |
| `JobContext:progress` | `(p: number)` | 0–1 |
| `configure` | `(opts: { budget: number? })` | Milliseconds per frame for all jobs of this addon. Default 4. |

Semantics:

- **Scheduler:** one hidden frame's `OnUpdate`. Each frame, jobs resume in
  smooth weighted round-robin by priority (weights high 4, normal 2, low 1)
  until the addon's budget (measured with `debugprofilestop()`) is used up;
  a job runs at most once per frame, so `yield` waits for the next frame.
  The scheduler stops its `OnUpdate` when no job is left.
- **Budget scope:** every addon has its own copy of the module (T10), so
  budgets are per addon, not global.
- **Errors:** a failing job is reported through the error policy, ends with
  status `"failed"`, and calls `onError`. Other jobs are unaffected.
- **Combat:** with `pauseInCombat`, the job is paused on
  `PLAYER_REGEN_DISABLED` and resumed after `PLAYER_REGEN_ENABLED`.
- **Lua 5.1 limitation:** a coroutine cannot yield across `pcall`, `xpcall`,
  `securecall*`, metamethods or C functions (e.g. inside a `table.sort`
  comparator or a `gsub` callback). In a function passed to `jobs.start`,
  calling `check`, `yield`, `sleep` or `waitFor` on its context parameter
  inside a function literal handed to `pcall`, `xpcall`,
  `securecallfunction`, `table.sort` or `gsub` is warning W8430. At runtime it
  raises "attempt to yield across metamethod/C-call boundary".
- `wowlua:item`'s `await` and the import parsing of large strings in
  `wowlua:dialog` ([Interface: dialog](interface.md#dialog)) are built on this module.

## table

`wowlua:table` holds the table helpers addons keep rewriting. Secret values
are copied as they are and never equal anything (comparing them would error).

```wowlua
import * as tbl from "wowlua:table"

local copy = tbl.copy(profile)                     -- deep; cycles kept
tbl.defaults(db.window, { width = 300, height = 200 })
if not tbl.equal(a, b) then … end
for _, key in ipairs(tbl.keys(map)) do … end       -- sorted when all numbers or all strings
local names = tbl.map(units, function(u: Unit) return u.name end)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `copy` | `<T>(t: T) -> T` | Deep; shared and cyclic references stay shared; metatables dropped |
| `shallowCopy` | `<T extends table>(t: T) -> T` | |
| `merge` | `<T extends table>(target: T, source: table) -> T` | Copies `source` in: nested tables merge, other values replace |
| `defaults` | `<T extends table>(target: T, defaults: table) -> T` | Fills in what `target` lacks, nested too |
| `equal` | `(a: any, b: any) -> boolean` | Deep |
| `keys`, `values` | `<K, V>(t: { [K]: V }) -> K[]`, `-> V[]` | `keys` is sorted when it can be |
| `count`, `isEmpty` | `(t: table) -> number`, `-> boolean` | All entries, not just the array part |
| `filter`, `map`, `find`, `contains` | list helpers | `find` returns the element and its index |

## format

`wowlua:format` formats durations, clocks, dates, money and numbers in the
client's own localized formats (decision S32): its `SecondsToTime`,
`SecondsToClock`, `FormatShortDate`, `GetMoneyString`, `BreakUpLargeNumbers`
and `AbbreviateNumbers`, and its `GOLD_AMOUNT`-style strings, which every
supported client has.

| API | Signature | Notes |
|-----|-----------|-------|
| `duration` | `(seconds: number, opts?: { noSeconds?, long?, units? }) -> string` | "1 Hr 5 Min" (`long`: unit words; `units` at most, default 2) |
| `short` | `(seconds: number) -> string` | The largest unit, like aura and cooldown times |
| `clock` | `(seconds: number, hours?: boolean) -> string` | "3:07", "1:03:07" |
| `date` | `(at?: number) -> string` | The locale's short date (default today) |
| `money` | `(copper: number, opts?: { text?, separate? }) -> string` | Coin icons, or with `text` the client's words |
| `number` | `(n: number \| Secret<number>) -> string \| Secret<string>` | Thousands separators |
| `abbreviate` | `(n: number \| Secret<number>) -> string \| Secret<string>` | "8.5K"; a secret gives a secret string |
| `percent` | `(fraction: number, decimals?: number) -> string` | "46%" |

## Testing

`wowlua:mock` support used by this chapter: `mock.errors()` (the list of
messages that reached `geterrorhandler`, in order), a `securecallfunction`
stub with real error forwarding, and `mock.advanceFrames(n, msPerFrame?)`,
which drives `OnUpdate` with a controllable `debugprofilestop` so job budgets
behave deterministically.
