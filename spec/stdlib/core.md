# 8.1 Standard Library: Core

## events

`wowlua:events` handles WoW events and addon-internal **messages** (custom
events, like AceEvent messages).

### Events and messages

| Kind | Declared with | Delivered by |
|------|---------------|--------------|
| **WoW event** | `declare event NAME(params)`, provided by the `wow-api-<flavor>` types package | The game. The module registers it on a hidden frame with `RegisterEvent` / `RegisterUnitEvent`. |
| **Message** | `declare message NAME(params)`, in the project or a library | `send(...)` only. It is dispatched in Lua and never registered with the game. |

`EventName` is the union of all declared WoW events and `MessageName` the union
of all declared messages ([Types §2.2.9](../language/02-types.md#229-special-named-types)).
`EventArgs<E>` is the **type pack** of the payload of `E`, for events and
messages alike. A name declared both as an event and as a message is error
E2058.

```wowlua
import event, unitEvent, on, once, off, send from "wowlua:events"

@event("PLAYER_ENTERING_WORLD")
local function onEnterWorld(isInitialLogin: boolean, isReloadingUi: boolean)
    …
end

@event("BAG_UPDATE", "BAG_UPDATE_DELAYED")          -- several names: the handler gets the name first
local function onBags(e: "BAG_UPDATE" | "BAG_UPDATE_DELAYED", ...any) … end

@unitEvent("UNIT_HEALTH", "player", "target")      -- at most two units
local function onHealth(unit: UnitToken) … end

local sub = on("PLAYER_REGEN_DISABLED", function() hideWindow() end)
once("PLAYER_LOGIN", function() … end)
off(sub)                                            -- or sub:cancel()

-- messages
declare message MYADDON_SETTINGS_CHANGED(key: string, value: any)

@event("MYADDON_SETTINGS_CHANGED")                  -- dispatched in Lua, not registered with the game
local function onSettingsChanged(key: string, value: any) … end

send("MYADDON_SETTINGS_CHANGED", "scale", 1.2)      -- type-checked against the declaration
```

### API

```wowlua
declare module "wowlua:events"
    interface Subscription
        active: boolean
        function cancel()
    end

    function on<E extends EventName | MessageName>(event: E, handler: (EventArgs<E>...) -> (), owner: any?): Subscription
    function once<E extends EventName | MessageName>(event: E, handler: (EventArgs<E>...) -> (), owner: any?): Subscription
    function onUnit<E extends EventName>(event: E, units: UnitToken[], handler: (EventArgs<E>...) -> (), owner: any?): Subscription
    function onBucket(events: (EventName | MessageName) | (EventName | MessageName)[], interval: number, handler: (counts: BucketCounts) -> (), owner: any?): Subscription
    function off(sub: Subscription)
    function offAll(owner: any)
    function send<M extends MessageName>(name: M, ...: EventArgs<M>...)   -- EventArgs<M> is a type pack
end
```

| API | Meaning |
|-----|---------|
| `@event(...names)` | One name: the handler receives `EventArgs<E>`. Several names: the handler receives the name first (typed as the union of the names), then `...any`. Names may mix WoW events and messages. |
| `@unitEvent(name, ...units)` | WoW events only. Uses `RegisterUnitEvent` with **one or two** units (the API's limit). More than two units is error E8100. |
| `on`, `once` | Dynamic registration. `owner` allows `offAll(owner)`. |
| `onUnit` | Dynamic version of `@unitEvent`. More than two units is a runtime error. |
| `@bucket(interval, ...names)`, `onBucket` | Coalesces bursts (like AceBucket): the first of the events starts an `interval`-second wait, and the handler then gets one `BucketCounts`: `args` counts by first argument (the units of unit events, the bags of `BAG_UPDATE`), `events` by event. |
| `off(sub)`, `offAll(owner)` | Unregister. `offAll(nil)` is a runtime error: it would cancel every subscription without an owner, the standard library's own included. |
| `send(name, ...)` | Dispatches a **message** synchronously to this addon's handlers. Passing a WoW event is error E8001. |

### Semantics

- **Registration with the game.** A WoW event is registered on the module's
  shared hidden frame while at least one plain handler exists, and unregistered
  when none remain. Messages are never registered with the game.
- **Unit events.** WoW keeps **one unit filter per frame and event**, and a
  plain `RegisterEvent` for the same event would clear the filter. So every
  `@unitEvent` / `onUnit` registration gets its **own hidden frame**, taken from
  an internal pool. Two registrations with the same event and the same unit set
  share a frame. Unit filters never interfere with each other or with plain
  `@event` handlers.
- **Order and isolation.** Handlers run in registration order. Each handler runs
  isolated, so an error is reported through `geterrorhandler()` and the
  remaining handlers still run (see the error policy in
  [`wowlua:errors`](infrastructure.md#errors)).
- **Changes during dispatch.** Registering or unregistering inside a handler
  takes effect after the current dispatch.
- **Static methods.** `@event` on a static class method registers at class
  definition. Instance methods cannot be decorated with `@event` (E8002). Use
  `on(event, function(...) self:handle(...) end, self)` in the constructor and
  `offAll(self)` when the object is discarded.
- **Buckets.** A burst longer than the interval gives one call per interval.
  A first argument that is nil or secret is counted only under `events`
  (secrets cannot be table keys). Cancelling the subscription drops what is
  pending (decision S34).
- **Name checking.** Names are checked against `EventName | MessageName`. An
  unknown name is error E8003 (useful when an event is removed in a patch); the
  names of `@bucket` and of literal `onBucket` lists are checked too. In
  `split` flavor mode, events missing from a flavor are reported for that flavor.
- **Shared flavor mode.** In `shared` mode
  ([Build metadata §6.3](../language/06-meta.md#63-flavors)), an event that does
  not exist in every selected flavor produces warning W8101. Its registration is
  guarded at runtime: `C_EventUtils.IsEventValid(name)` where available,
  otherwise a protected `RegisterEvent` call. In flavors without the event, the
  handler is silently never called.

## lifecycle

`wowlua:lifecycle` wraps the addon startup sequence.

```wowlua
import onLoad, onLogin, onAddonLoaded from "wowlua:lifecycle"

@onLoad
local function init()          -- SavedVariables are available here
    migrateSettings()
end

@onLogin
local function start() … end  -- UI and character data are available

onAddonLoaded("Blizzard_PlayerSpells", function()
    -- runs when that addon loads, or right away if it already has
end)
```

| API | Meaning |
|-----|---------|
| `@onLoad` | `ADDON_LOADED` for this addon (by name, from `wowlua:meta`) |
| `@onLogin` | `PLAYER_LOGIN`. For load-on-demand addons loaded after login, it runs right after `@onLoad`. |
| `@onEnterWorld` | Every `PLAYER_ENTERING_WORLD`, with `isInitialLogin` and `isReloadingUi` |
| `@onLogout` | `PLAYER_LOGOUT` |
| `onAddonLoaded(name: string, fn: () -> ())` | Runs `fn` once when AddOn `name` has loaded. If `C_AddOns.IsAddOnLoaded(name)` is already true, it runs immediately. |
| `isLoaded(): boolean`, `isLoggedIn(): boolean` | State queries |

### Phase order

For each lifecycle event, the order is fixed:

| Event | Order |
|-------|-------|
| `ADDON_LOADED` (this addon) | 1. Standard-library work: `wowlua:saved` loads, migrates and attaches defaults for every bound SavedVariable. 2. `@onLoad` hooks, in load order (module order, then declaration order). 3. Deferred standard-library setup that reads saved data (`wowlua:settings`, `wowlua:launcher`). |
| `PLAYER_LOGIN` | `@onLogin` hooks in load order, then deferred standard-library work that needs the UI (minimap and compartment entries, LDB) |
| `PLAYER_LOGOUT` | 1. `@onLogout` hooks in load order. 2. Standard-library work last: `wowlua:saved` strips defaults. |

A hook that raises an error is reported through `geterrorhandler()`. The
remaining hooks of the phase still run.

## slash

`wowlua:slash` registers slash commands, with an optional typed subcommand
router.

```wowlua
import slash, router from "wowlua:slash"

-- simple
@slash("/mya", "/myaddon")
local function onCommand(msg: string) … end
```

```wowlua
-- alternative: a router ("/kt show", "/kt scale 1.5", "/kt reset --all")
import slash, router from "wowlua:slash"

local commands = router({
    show  = { run = function() window:Show() end, help = "Show the window" },
    scale = { args = { "number" }, run = function(v: number) setScale(v) end, help = "Set UI scale" },
    reset = { flags = { "all" }, run = function(flags: { all: boolean }) reset(flags.all) end,
              help = "Reset settings" },
})

@slash("/kt", "/killtracker")
local function onRouted(msg: string) commands:dispatch(msg) end
```

### `@slash`

- `@slash(...)` generates the `SLASH_<KEY>1..n` globals and the
  `SlashCmdList["<KEY>"]` entry, as WoW's slash API requires. These globals are
  listed in the normative global-writes list
  ([Code generation §14.1](../toolchain/codegen.md#141-target)).
- `<KEY>` is `WOWLUA_<ADDON>_<N>`: the addon name uppercased with every
  character outside `[A-Z0-9]` replaced by `_`, and `N` numbering the addon's
  slash registrations. Keys are therefore unique across all addons, not just
  within one.
- Command names must start with `/` and contain no whitespace (E8101).
  Registering the same command twice in one addon is error E8010. A collision
  with a Blizzard command is warning W8011.
- The handler receives the message text (`msg: string`) and the edit box
  (`editBox: EditBox`), as `SlashCmdList` provides them.

### `router`

```wowlua
declare module "wowlua:slash"
    type ArgSpec = "string" | "number" | "boolean" | "rest"
                 | "string?" | "number?" | "boolean?"
                 | string[]                      -- choices (literal union)

    interface Command
        args?: ArgSpec[]
        flags?: string[]
        aliases?: string[]
        help?: string
        run: (...any) -> ()                      -- checked against args/flags (intrinsic)
    end

    interface Router
        function dispatch(msg: string): boolean
        function help(): string
    end

    function router(commands: { [string]: Command }): Router
end
```

| Rule | Detail |
|------|--------|
| Tokenizing | Whitespace-separated. `"two words"` and `'two words'` are single tokens. `\"` escapes a quote inside a quoted token. |
| Command matching | The first token, **case-insensitive**, against command names and `aliases`. Argument values keep their case. |
| Positional arguments | Converted per `ArgSpec`: `"number"` via `tonumber`; `"boolean"` accepts `true/false/on/off/yes/no/1/0` (case-insensitive); a choice list must match one entry case-insensitively and yields that entry's spelling; `"rest"` (last only) yields the remaining text unparsed. `?` marks an optional trailing argument. |
| Flags | `--name` tokens anywhere after the command set `flags.name = true`, except inside a `"rest"` argument's text (which starts at the first token after the arguments before it, as typed). Unknown flags are a usage error. |
| Calling `run` | Positional values in order, then the flags table **last** if `flags` is declared. The compiler checks `run`'s parameter types against `args` and `flags` (intrinsic check, E8112 for a mismatch or an invalid `ArgSpec`). |
| Errors | A conversion or arity error prints `Usage: /<cmd> <command> <args>` for that command and returns `true`. |
| Help | `help` and an empty message print one line per command, sorted by name: `/<cmd> <command> <args> - <help>`. Output goes through `print` from `wowlua:log`. An unknown command prints the help and `dispatch` returns `false`. |

## saved

`wowlua:saved` provides typed SavedVariables with defaults, profiles and
migrations, in the spirit of AceDB.

```wowlua
import * as saved from "wowlua:saved"
import onLoad from "wowlua:lifecycle"

interface Settings
    scale: number
    position?: { x: number, y: number }
    hidden: { [string]: boolean }
end

local db = saved.bind("MyAddonDB", {
    version = 3,
    defaults = { scale = 1, hidden = {} } :: Settings,
    migrate = {
        [2] = function(old: any) old.scale = old.uiScale; old.uiScale = nil end,
        [3] = function(old: any) old.hidden = old.hidden or {} end,
    },
    profiles = true,                       -- needs MyAddonDBPerChar, see below
})

@onLoad
local function init()
    db.scale = math.max(db.scale, 0.5)     -- ok: data is loaded
    db.hidden.minimap = true               -- nested defaults persist (lazy deep defaults)
    saved.handle(db).profiles:use("Shared")
end
```

### API

```wowlua
declare module "wowlua:saved"
    interface BindOptions<T>
        version: number
        defaults: T
        migrate?: { [number]: (data: any) -> () }
        profiles?: boolean
        stripDefaults?: boolean            -- default true
    end

    interface Profiles
        function use(name: string)
        function current(): string
        function list(): string[]
        function copy(from: string)
        function delete(name: string)
        function onChange(fn: (name: string) -> ()): Subscription   -- Subscription from wowlua:events
    end

    interface SavedHandle
        profiles?: Profiles                -- nil unless bound with profiles = true
        function reset()                   -- current profile (or the whole data) back to defaults
        function isLoaded(): boolean
    end

    function bind<T>(name: string, opts: BindOptions<T>): T
    function handle<T>(db: T): SavedHandle
end
```

- `bind`'s `name` must be a **string literal** naming a SavedVariable declared
  in the manifest (`addon.saved-variables` or `addon.saved-variables-per-character`).
  A non-literal name is error E8102. An undeclared name is error E8020. Binding
  the same SavedVariable twice is error E8103.
- `bind` returns the data table itself, typed `T`. Housekeeping lives on a
  separate handle (`saved.handle(db)`), so user keys such as `reset` or
  `profiles` never clash with library methods. There are no helper methods on
  `db` itself (decision S13).
- The defaults' type must not contain `Secret<…>` (E8104): secret values cannot
  be compared or saved.

### Loading and early access

- `bind` can be called at module top level. Until the addon's SavedVariables are
  loaded, the returned table is **locked**: any read or write raises
  `MyAddon: MyAddonDB accessed before it was loaded; use @onLoad`, in every build.
  Writes before load would otherwise be lost silently.
- At `ADDON_LOADED`, before user `@onLoad` hooks, the module loads the stored
  data into the returned table (its identity never changes), points the global
  (`_G["MyAddonDB"]`) at it so WoW saves it, runs migrations and attaches
  defaults.
- With `addon.load-saved-variables-first` in the manifest, SavedVariables exist
  before the addon's files run. `bind` then loads immediately and the data is
  usable at top level.

### Defaults

- **Lazy deep defaults.** Defaults are attached through a metatable. Reading a
  missing key returns the default. When the default is a table, the first read
  creates a new table in the stored data (with its own defaults metatable for
  the nested defaults) and returns it. Writes into nested tables therefore always
  land in the saved data, never in the shared defaults.
- **Stripping.** At `PLAYER_LOGOUT`, after `@onLogout` hooks, values equal to their
  defaults are removed **recursively**: scalars are compared with `==`, nested
  tables are stripped key by key, and tables that end up empty and have a table
  default are removed. Values for which `issecretvalue(v)` is true are dropped
  (they cannot be saved) and reported once in debug builds.
- `stripDefaults = false` keeps every value.

### Migrations

- Migrations run at load, in order from the stored `__version + 1` up to
  `version`. Each receives the **whole stored SavedVariable table** (including all
  profiles), once per SavedVariable, not once per profile.
- **Fresh install** (no stored data): no migrations run, and `__version` is set to
  `version`.
- **Downgrade** (stored `__version` greater than `version`): no migrations run,
  the data is kept unchanged (including `__version`), stripping is disabled for
  this session, and a warning is reported once through `wowlua:errors`
  (decision S14). Nothing is lost if the user upgrades again.
- `__version` is a reserved top-level key of the SavedVariable table.

### Profiles

- With `profiles = true`, the SavedVariable's shape is
  `{ __version, profiles = { [name] = data } }`. The returned table forwards to the
  active profile's data (it is a proxy, so profile switches keep its identity).
- The active profile per character is stored in a per-character SavedVariable
  named `<name>PerChar` (e.g. `MyAddonDBPerChar`). It must be listed in
  `addon.saved-variables-per-character` (E8021 names it). The default profile
  is `"Default"`.
- Profiles on a per-character SavedVariable are error E8117.
- `profiles:onChange` fires after a switch. `wowlua:settings` and
  `wowlua:launcher` subscribe to it and refresh.

## timer

`wowlua:timer` wraps `C_Timer` and adds frame-based helpers. Timers are the
**native** `C_Timer` handles, typed by the `wow-api` packages.

```wowlua
import after, every, throttle, debounce, nextFrame from "wowlua:timer"

local t = after(2, function() print("2s later") end)
local tick = every(1, function(ticker: TickerHandle) update() end, 10)   -- 10 iterations
t:Cancel()

class Window
    @throttle(0.2)
    function onResize() self:layout() end

    @nextFrame
    function refresh() … end              -- many calls in one frame: one refresh

    function layout() … end
end

@debounce(0.5)
local function saveSearch(text: string) … end
```

| API | Meaning |
|-----|---------|
| `after(sec, fn): TimerHandle` | One-shot timer (`C_Timer.NewTimer`). The handle has `Cancel()` and `IsCancelled()`. |
| `every(sec, fn, iterations?): TickerHandle` | Repeating timer (`C_Timer.NewTicker`). `fn` receives the ticker. |
| `nextFrame(fn)` / `@nextFrame` | Run on the next frame. As a decorator, repeated calls before the next frame are coalesced, and the last call's arguments win. |
| `@throttle(sec)` | The first call runs immediately. Later calls within `sec` collapse into one trailing call with the latest arguments. |
| `@debounce(sec)` | Runs `sec` seconds after the last call, with the latest arguments. |

Decorated methods keep **per-instance state** in a weak-keyed table indexed by
`self`, so each object has its own throttle. Functions decorated with
`@throttle`, `@debounce` or `@nextFrame` must return nothing (E8030), because
calls may be deferred.
