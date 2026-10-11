# 8.3 Standard Library: Advanced

## combat

`wowlua:combat` helps with combat lockdown. While the player is in combat,
protected functions (showing secure frames, changing secure attributes, moving
protected frames, …) cannot be called by addons.

```wowlua
import outOfCombat, defer, inCombat, onCombatChange from "wowlua:combat"

class ActionBar
    frame: Frame
    offset: number = 0

    @outOfCombat
    function relayout()
        self.frame:SetPoint("BOTTOM", UIParent, "BOTTOM", 0, self.offset)
    end
end

defer(function() secureButton:SetAttribute("spell", spellName) end)
if inCombat() then … end
onCombatChange(function(entering: boolean) … end)
```

| API | Meaning |
|-----|---------|
| `@outOfCombat` | If `InCombatLockdown()` is false, the call runs immediately. Otherwise it is queued and runs after `PLAYER_REGEN_ENABLED`. Repeated queued calls to the same function (per receiver for methods) are coalesced, and the last arguments win. The function must return nothing (E8030). |
| `defer(fn)` | The same for an anonymous function (not coalesced) |
| `inCombat(): boolean` | `InCombatLockdown()` |
| `onCombatChange(fn): Subscription` | Called with `true` on `PLAYER_REGEN_DISABLED` and `false` on `PLAYER_REGEN_ENABLED` |

Queue semantics:

- The queue is flushed on `PLAYER_REGEN_ENABLED`, in FIFO order.
- A coalesced call keeps the queue position of its **first** queuing. Only its
  arguments are replaced.
- If combat starts again during a flush (`InCombatLockdown()` becomes true), the
  remaining entries stay queued for the next flush.
- A queued function that errors is reported through `geterrorhandler()` without
  blocking the rest of the queue.

## hooks

`wowlua:hooks` provides post-hooks through `hooksecurefunc` and `HookScript`.
These do not taint the hooked Blizzard function itself, but the hook body still
runs as addon (insecure) code and must follow the usual taint rules.

```wowlua
import hook, hookScript from "wowlua:hooks"

@hook("ToggleGameMenu")
local function afterGameMenu() … end

@hook(GameTooltip, "SetUnit")
local function afterSetUnit(tooltip: GameTooltip, unit: UnitToken) … end

@hookScript(PlayerFrame, "OnShow")
local function onPlayerFrameShow(frame: Frame) … end

-- a target that only exists once a load-on-demand addon has loaded
@hook("PlayerSpellsFrame", "Show", { addon = "Blizzard_PlayerSpells" })
local function afterSpellbookShow(frame: Frame) … end
```

| API | Lowering |
|-----|----------|
| `@hook(name)` | `hooksecurefunc(name, fn)` |
| `@hook(target, name, opts?)` | `hooksecurefunc(target, name, fn)` |
| `@hookScript(frame, script, opts?)` | `frame:HookScript(script, fn)` |
| `hook(...)`, `hookScript(...)` | The same as plain functions |

- **Targets.** `target` (and `frame` for `@hookScript`) is either a table or a
  **string naming a global**. A string is resolved from `_G` when the hook is
  installed.
- **Deferred hooks.** With `opts.addon = "<AddOn>"`, the hook is installed when
  that addon has loaded (through `onAddonLoaded` from `wowlua:lifecycle`), or
  immediately if it already has. Use a string target for objects that don't
  exist before then. Hooking a target that the type declarations attribute to a
  load-on-demand addon without `opts.addon` is warning W8102.
- **Signatures.** The hook's parameters are checked against the hooked
  function's declared signature, with the receiver first for method hooks.
  Hooking a name that is not declared is warning W8040.
- **No removal, no pre-hooks.** Hooks are post-hooks and cannot be removed (a WoW
  API limitation). The module intentionally provides **no** pre-hooks or
  replacement hooks, because replacing Blizzard functions taints secure code.
  Code that needs them must write the Lua by hand.

## comm

`wowlua:comm` provides addon-to-addon messaging over WoW's addon message
channels, like AceComm with ChatThrottleLib.

```wowlua
import * as comm from "wowlua:comm"

local channel = comm.open("MyAddon")          -- prefix, registered automatically

channel:onMessage(function(msg: string, sender: string, distribution: comm.Distribution)
    …
end)

channel:send("hello", "PARTY")
channel:send(largePayload, "WHISPER", "Name-Realm", { priority = "bulk" })
```

| API | Meaning |
|-----|---------|
| `open(prefix: string): Channel` | Registers the prefix through `C_ChatInfo.RegisterAddonMessagePrefix`. Prefixes over the API length limit are error E8050 (checked at compile time for literals). |
| `Channel:send(msg, distribution, target?, opts?): SendStatus` | Sends a string of any length. Messages over the per-message size limit are split into chunks and reassembled by receivers. Returns `"sent"`, `"queued"` (throttled or in lockdown) or `"dropped"` (only with `opts.dropInLockdown`). |
| `Channel:onMessage(fn)` | Receives complete, reassembled messages |
| `Channel:sendObject(value, …)` / `onObject(fn)` | Serializes with `wowlua:serialize` (and optionally compresses) |
| `opts.priority` | `"alert"`, `"normal"`, `"bulk"` |
| `opts.callback` | Progress callback `(sentBytes, totalBytes)` |
| `opts.receiveOwn` | Deliver messages sent by the player themselves. Default `false`. |
| `Distribution` | `"PARTY"`, `"RAID"`, `"INSTANCE_CHAT"`, `"GUILD"`, `"OFFICER"`, `"WHISPER"`, `"CHANNEL"`, plus `"GROUP"`, which picks the right one for the current group |

`comm.Distribution` and `comm.SendStatus` are exported types. A namespace
import (`import * as comm`) exposes a module's exported types as `comm.<Type>`
in type positions.

### Throttling

- If **ChatThrottleLib** (the shared throttle used by AceComm and many other
  addons) is loaded at send time, every chunk goes through
  `ChatThrottleLib:SendAddonMessage` with the matching priority
  (`"ALERT"`, `"NORMAL"`, `"BULK"`). This shares bandwidth with every other addon
  that uses it.
- Otherwise a built-in throttle with **conservative** limits from the flavor
  table is used, shared by all channels of the addon.
- Where the send API reports a result (`Enum.SendAddonMessageResult`), a
  throttled result requeues the chunk with backoff. Other failures are reported
  through `wowlua:errors`.
- Messages are sent with `C_ChatInfo.SendAddonMessage`, or `SendAddonMessageLogged`
  when `opts.logged` is set.

### Messaging lockdown

Since patch 12.0, addon chat is locked down in some situations on restricted
flavors. The module treats chat as locked whenever
`C_ChatInfo.InChatMessagingLockdown()` returns true. While locked, outgoing
messages stay queued (or are dropped with `opts.dropInLockdown`). While the
queue is non-empty and chat is locked, the module re-checks the lockdown state
once per second and drains the queue when it ends. `comm.inLockdown()` exposes
the state.

### Wire format

Wire format **version 1**, so that addons of different versions interoperate in
one group:

- Every chunk starts with one control byte: `\001` complete message, `\002`
  first chunk, `\003` middle chunk, `\004` last chunk. Other control bytes are
  reserved for future versions, and messages that use them are ignored.
- Chunks of one message are sent in order, and reassembly is keyed by sender and
  prefix.
- Addon messages cannot contain `\0`, so payload bytes `\0` are escaped
  transparently (`toAddonChannel` encoding from `wowlua:serialize`) and unescaped
  on receipt.
- The sender passed to handlers is normalized with `Ambiguate(sender, "none")`.
  With `receiveOwn = false`, messages whose sender is the player are filtered out.

**AceComm compatibility** (decision S16). `comm.open(prefix, { format =
"acecomm" })` switches a channel to AceComm-3.0's chunk markers and reassembly
rules instead of the wowlua format, so an addon migrating from Ace3 still
interoperates with older versions of itself in a group. The default is
`"wowlua"`. Both formats use the same throttling and lockdown handling.

## serialize

`wowlua:serialize` serializes, compresses and encodes data, for import/export
strings and comms, in the spirit of LibSerialize and LibDeflate.

```wowlua
import * as serialize from "wowlua:serialize"

local s = serialize.exportString(profile, { version = 1 })   -- all steps plus a header and checksum
local ok, value = serialize.importString(s)

-- the individual steps
local text = serialize.toPrintable(serialize.compress(serialize.encode(profile)))
local packed = serialize.fromPrintable(text)
local raw = packed and serialize.decompress(packed)
if raw then
    local decoded, back = serialize.decode(raw)
end
```

```wowlua
declare module "wowlua:serialize"
    function encode(value: any): string
    function decode(s: string): (boolean, any)
    function compress(s: string, level: number?): string
    function decompress(s: string): string?
    function toPrintable(s: string): string
    function fromPrintable(s: string): string?
    function toAddonChannel(s: string): string
    function fromAddonChannel(s: string): string
    function exportString(value: any, opts: { version: number, prefix: string? }): string
    function importString(s: string): (boolean, any)
end
```

| API | Meaning |
|-----|---------|
| `encode` / `decode` | Compact binary format: nil, booleans, numbers (integer and float paths), strings (with deduplication), tables (array and hash parts, nested, cycles rejected). Functions, userdata and threads are errors. `encode` raises a clear error on secret values (checked with `issecretvalue` where available). |
| `compress` / `decompress` | DEFLATE (RFC 1951) in pure Lua. `decompress` returns `nil` on corrupt input. |
| `toPrintable` / `fromPrintable` | Encoding into a chat-safe alphabet. `fromPrintable` returns `nil` on invalid input. |
| `toAddonChannel` / `fromAddonChannel` | Encoding safe for addon message channels (escapes `\0`) |
| `exportString` / `importString` | Versioned, checksummed export strings with a configurable prefix (e.g. `"!MYADDON:1!"`) |

`decode` and `importString` never execute code (no `loadstring`), so importing
strings from other players is safe. Depth and size limits guard against
malicious input.

## log

`wowlua:log` provides leveled logging with compile-time stripping
([Debugging §7.2](../language/07-debugging.md#72-logging)).

```wowlua
import log, print as chat from "wowlua:log"

log.info("ready")
log.debug("cache", id, "hit", hit)
log.warn(`missing spell {spellId}`)
log.error("failed:", err)

chat("Hello!")                                -- "|cff33ff99MyAddon|r: Hello!"
chat.color("ff0000", "Error!")
```

The level functions are exported both at the top level and on the `log`
object, so `import log from "wowlua:log"` (`log.info(…)`) and
`import * as Log from "wowlua:log"` (`Log.info(…)`) both work.

```wowlua
declare module "wowlua:log"
    type Level = "trace" | "debug" | "info" | "warn" | "error"
    type Printer = ((...any) -> ()) & {
        color: (hex: string, ...any) -> (),
        to: (frame: ScrollingMessageFrame, ...any) -> (),
    }

    function trace(...: any)
    function debug(...: any)
    function info(...: any)
    function warn(...: any)
    function error(...: any)
    function setLevel(level: Level)
    function sink(fn: (level: Level, message: string) -> ())
    function history(n: number?): { time: number, level: Level, message: string }[]
    const log: { trace: (...any) -> (), debug: (...any) -> (), info: (...any) -> (),
                 warn: (...any) -> (), error: (...any) -> () }
    const print: Printer
end
```

| API | Meaning |
|-----|---------|
| `trace/debug/info/warn/error(...)` | Log at the given level. Calls below the profile's `log-level` are removed at compile time, together with their arguments. Using a level function as a value (not calling it directly) is error E8118, because the compiler could not strip it. Wrap it in a function instead. |
| `setLevel(level)` | Runtime level, which cannot go below the compiled level |
| `sink(fn)` | Add an output. The default sink writes `warn` and above to the default chat frame and everything to the ring buffer. |
| `history(n?)` | Entries from the ring buffer. The default size is 500, configurable with the manifest key `stdlib.log.history-size`. Shown by `/wl log` in dev builds. |
| `print(...)` | Prints to the default chat frame with the addon's colored name prefix. `print` is a callable value with fields (an intersection type). |
| `print.color(hex, ...)`, `print.to(frame, ...)` | Colored output, or output to a specific chat frame |

Color and link formatting helpers live in [`wowlua:color`](interface.md#color)
and [`wowlua:item`](game.md#item).

Log entries include a timestamp (`GetTime()`), the level and, in debug builds,
the source location (`file:line`, folded at compile time). Secret values are
logged as `<secret>`.
