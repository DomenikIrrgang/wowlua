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
2. **Mechanism.** Isolation uses `xpcall` with a closure that captures the
   arguments (Lua 5.1's `xpcall` passes none) and a handler of the policy's
   own, in every flavor. `securecallfunction` is not used: it hands errors to
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
| `report` | `(err: any, context: { [string]: any }?)` | Reports through the policy. Non-string errors are converted with `tostring`. A secret error, or a secret `tostring` result, is reported as `<secret>`. |
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
  next `fire`. Arguments are passed on unchanged, secrets included.
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
  status `"failed"`, and calls `onError`. Other jobs are unaffected. A secret
  message is reported as `job <name>: <secret>`; `onError` gets it unchanged.
  Payloads (`waitFor`) and results pass through unchanged, secrets included.
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

`wowlua:table` holds the table helpers addons keep rewriting. Values the
addon may not read (secrets, and tables whose security keeps addons out,
`secret.readable`) are never walked, compared or used as keys, since that
would error: they are copied as they are, never equal anything, are never
found, and sort after everything else in their order. A table argument the
addon may not read reads as empty.

```wowlua
import * as tbl from "wowlua:table"

local copy = tbl.copy(profile)                     -- deep; cycles kept
tbl.defaults(db.window, { width = 300, height = 200 })
if not tbl.equal(a, b) then … end
for _, key in ipairs(tbl.keys(map)) do … end       -- sorted when all numbers or all strings
local names = tbl.map(units, function(u: Unit) return u.name end)

-- Stable, and safe in combat: auras whose expiration is secret go last.
tbl.sort(auras, tbl.by(function(a: AuraData) return a.expirationTime end))
tbl.sort(members, tbl.by(function(m: Member) return m.role end))   -- keeps the name order within a role
local at = tbl.insertSorted(queue, entry, tbl.by(function(e: Entry) return e.at end))
```

| API | Signature | Notes |
|-----|-----------|-------|
| `copy` | `<T>(t: T) -> T` | Deep; shared and cyclic references stay shared; metatables dropped |
| `shallowCopy` | `<T extends table>(t: T) -> T` | |
| `merge` | `<T extends table>(target: T, source: table) -> T` | Copies `source` in: nested tables merge, other values replace |
| `defaults` | `<T extends table>(target: T, defaults: table) -> T` | Fills in what `target` lacks, nested too |
| `equal` | `(a: any, b: any) -> boolean` | Deep; secrets never equal |
| `keys`, `values` | `<K, V>(t: { [K]: V }) -> K[]`, `-> V[]` | `keys` is sorted when it can be |
| `count`, `isEmpty` | `(t: table) -> number`, `-> boolean` | All entries, not just the array part |
| `filter`, `map`, `find`, `contains` | list helpers | `find` returns the element and its index; `contains` never finds a secret |
| `sort` | `<T>(list: T[], less?: Less<T>) -> T[]` | In place, returns `list`. **Stable** (merge sort; insertion sort up to 12 elements, without garbage). An inconsistent `less` gives some order, never "invalid order function for sorting". Secret elements never reach `less`; they go last, in their order. Default: numbers and strings ascending, values of different types by type name. |
| `by` | `<T>(key: (v: T) -> unknown, descending?: boolean) -> Less<T>` | An order by a key: numbers and strings, ascending unless `descending`; elements whose key is nil or secret go last, in their order |
| `order` | `<T>(less?: Less<T>) -> Less<T>` | `less` behind the guard `sort` uses (secrets after everything, never handed to `less`), for ordered structures of the addon's own |
| `insertSorted` | `<T>(list: T[], value: T, less?: Less<T>) -> number` | Inserts behind the elements equal to `value` (binary search); returns the position. A secret goes last. |
| `searchSorted` | `<T>(list: T[], value: T, less?: Less<T>) -> number?` | The first element equal to `value` (neither before nor after it) by binary search; a secret is never found |
| `Less<T>` | `(a: T, b: T) -> boolean` | Whether `a` goes before `b` |

## collections

`wowlua:collections` holds the containers addons keep writing for combat
data. They store secret values as they are and never compare, index or use
them as keys: a `Heap` orders secrets after every other value, an
`LruCache` ignores secret keys.

```wowlua
import RingBuffer, Heap, LruCache from "wowlua:collections"
import by from "wowlua:table"

local damage = RingBuffer.new(30)                 -- the last 30 hits
damage:push(amount)                               -- a secret amount is kept as it is

local expiring: Heap<AuraData> = Heap.new(by(function(a: AuraData) return a.expirationTime end))
expiring:push(aura)
local next = expiring:peek()                      -- secret expirations come last

local items: LruCache<number, ItemInfo> = LruCache.new(200)
items:set(id, info)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `RingBuffer<T>` | `RingBuffer.new(capacity: number)` | The last `capacity` values, oldest first; slots are reused, so pushing makes no garbage |
| | `push(v: T) -> T?` | Appends; returns the value it pushed out of a full buffer |
| | `shift() -> T?`, `pop() -> T?` | Removes the oldest, the newest |
| | `at(i: number) -> T?` | 1 is the oldest, -1 the newest |
| | `size()`, `isFull()`, `clear()`, `each() -> (() -> T?)`, `toList() -> T[]`, `capacity` | `each` and `toList` go oldest first |
| `Heap<T>` | `Heap.new(less?: Less<T>)` | A binary heap: `pop` gives the smallest by `less` (default: numbers and strings ascending; `table.by` for a key). Secrets never reach `less` and come out after all others. |
| | `push(v: T)`, `pop() -> T?`, `peek() -> T?` | `O(log n)` |
| | `remove(v: T) -> boolean` | Removes that value (the same value, not an equal one); a secret is never found |
| | `size()`, `clear()` | |
| `LruCache<K, V>` | `LruCache.new(capacity: number)` | At most `capacity` entries; a new key in a full cache drops the least recently used one (`get` and `set` are uses, `has` is not). Entries are reused, so a full cache makes no garbage. |
| | `get(key: K) -> V?`, `set(key: K, value: V?)`, `has(key: K) -> boolean`, `delete(key: K) -> boolean` | `set` with nil deletes. A secret key finds nothing and is not stored (a secret cannot be a table key); values may be secret. |
| | `size()`, `clear()`, `capacity` | |

## utf8

`wowlua:utf8` counts characters, not bytes: WoW's Lua 5.1 has no `utf8`
library, and `string.sub` cuts names with umlauts, Cyrillic or Hangul in the
middle of a character. A byte that starts no valid sequence counts as one
character, so every string works. A secret string cannot be read:
`truncate` returns it whole (a FontString can still cut it on screen, with
its width and `SetWordWrap(false)`), and the other functions read it as
empty.

```wowlua
import * as utf8 from "wowlua:utf8"

nameText:SetText(utf8.truncate(UnitName(unit), 10))   -- in combat too: a secret name passes whole
local initial = utf8.sub(name, 1, 1)                     -- "Ä", not half of it
```

| API | Signature | Notes |
|-----|-----------|-------|
| `len` | `(s: string) -> number` | Characters; 0 for a secret |
| `sub` | `(s: string, i: number, j?: number) -> string` | Characters `i` to `j`, positions as `string.sub` counts them (negative from the end); "" for a secret |
| `truncate` | `(s: string \| Secret<string>, max: number, ellipsis?: string) -> string \| Secret<string>` | At most `max` characters, `ellipsis` (default none) included when it cuts; a secret returned whole |
| `chars` | `(s: string) -> (() -> string?)` | An iterator over the characters; none for a secret |

## format

`wowlua:format` formats durations, clocks, dates, money and numbers in the
client's own localized formats (decision S32): its `SecondsToTime`,
`SecondsToClock`, `FormatShortDate`, `GetMoneyString`, `BreakUpLargeNumbers`
and `AbbreviateNumbers`, and its `GOLD_AMOUNT`-style strings, which every
supported client has.

| API | Signature | Notes |
|-----|-----------|-------|
| `duration` | `(seconds: number, opts?: { noSeconds?, long?, units? }) -> string` | "1 Hr 5 Min" (`long`: unit words; `units` at most, default 2). Secret options count as unset. |
| `short` | `(seconds: number) -> string` | The largest unit, like aura and cooldown times |
| `clock` | `(seconds: number, hours?: boolean) -> string` | "3:07", "1:03:07" |
| `date` | `(at?: number) -> string` | The locale's short date (default today) |
| `money` | `(copper: number, opts?: { text?, separate? }) -> string` | Coin icons, or with `text` the client's words. Secret options count as unset. |
| `number` | `(n: number \| Secret<number>) -> string \| Secret<string>` | Thousands separators |
| `abbreviate` | `(n: number \| Secret<number>) -> string \| Secret<string>` | "8.5K"; a secret gives a secret string |
| `percent` | `(fraction: number, decimals?: number) -> string` | "46%" |

## Testing

`wowlua:mock` support used by this chapter: `mock.errors()` (the list of
messages that reached `geterrorhandler`, in order), a `securecallfunction`
stub with real error forwarding, and `mock.advanceFrames(n, msPerFrame?)`,
which drives `OnUpdate` with a controllable `debugprofilestop` so job budgets
behave deterministically.
