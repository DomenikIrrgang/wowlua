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
- Queued arguments are only stored, so secrets reach the call unchanged.

## secure

`wowlua:secure` uses the client's secure state drivers: the client evaluates a
macro conditional (`[combat] show; hide`) and acts on it, also in combat, where
addon code may not show or hide protected frames.

```wowlua
import * as secure from "wowlua:secure"

secure.visibility(bar, "[petbattle][vehicleui] hide; [combat] show; hide")
secure.state(bar, "page", "[bonusbar:5] 11; [mod:shift] 2; 1", function(page: string)
    label:SetText(page)
end)
if secure.evaluate("[group:raid] yes") then … end
```

| API | Signature | Notes |
|-----|-----------|-------|
| `visibility` | `(frame: Frame, conditional: string?)` | `RegisterStateDriver(frame, "visibility", …)`; the values are `show` and `hide`. nil removes the driver; the frame stays as it is. |
| `state` | `(frame: Frame, name: string, conditional: string, fn: (value: string) -> ()) -> Subscription` | A driver for the state `name`; `fn` gets each new value, the first right away. Cancelling removes the driver. |
| `evaluate` | `(conditional: string) -> string?` | What the conditional gives now (`SecureCmdOptionParse`); nil when no clause matches |

Semantics (decision S35):

- **Combat.** Registering or removing a driver is blocked in combat, so it
  waits for the end of combat through `wowlua:combat`'s `defer`, in call order.
  The drivers themselves keep working in combat.
- **Values.** `state` watches the frame's `state-<name>` attribute through
  `OnAttributeChanged` (a hook, so it works on protected frames); `fn` runs as
  ordinary addon code, which in combat may not touch protected frames.
- **Compile-time check.** A literal conditional (or one folded from `const`s) is
  parsed: brackets must pair, and each condition must be one the client knows,
  with or without `no` (`combat`, `mod`, `group`, `form`, `bonusbar`, `exists`,
  `help`, `harm`, `vehicleui`, `petbattle`, …), else E8440. `@unit` and
  `target=unit` are accepted. A `visibility` value other than `show` or `hide`
  is E8441. Arguments (`mod:shift`, `form:1/2`) are not checked.

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
- **Arguments.** The hook gets the hooked call's arguments unchanged; on
  clients with secret values they may be secret.
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

channel:onMessage(function(msg: string, sender: string, distribution: string)
    …
end)

channel:send("hello", comm.Distribution.Party)
channel:send(largePayload, "WHISPER", "Name-Realm", { priority = comm.Priority.Bulk })
```

| API | Meaning |
|-----|---------|
| `open(prefix: string): Channel` | Registers the prefix through `C_ChatInfo.RegisterAddonMessagePrefix`. Prefixes over the API length limit are error E8050 (checked at compile time for literals). |
| `Channel:send(msg, distribution, target?, opts?): SendStatus` | Sends a string of any length. Messages over the per-message size limit are split into chunks and reassembled by receivers. Returns `SendStatus.Sent`, `Queued` (throttled or in lockdown) or `Dropped` (with `opts.dropInLockdown`, out of a group for `Group`, or for a secret `msg`, `distribution` or `target`, which the client refuses). |
| `Channel:onMessage(fn)` | Receives complete, reassembled messages |
| `Channel:sendObject(value, …)` / `onObject(fn)` | Serializes with `wowlua:serialize` (and optionally compresses) |
| `opts.priority` | A `Priority`: `Alert`, `Normal`, `Bulk` |
| `opts.callback` | Progress callback `(sentBytes, totalBytes)` |
| `opts.receiveOwn` | Deliver messages sent by the player themselves. Default `false`. |
| `Distribution` | `Party`, `Raid`, `Instance`, `Guild`, `Officer`, `Whisper`, `Channel`, plus `Group`, which picks the right one for the current group |

`comm.Distribution`, `comm.Priority`, `comm.Format` and `comm.SendStatus` are
exported string enums (`Distribution.Party` is `"PARTY"`). Parameters also
take the strings (`"PARTY"`, `"bulk"`): their types are `DistributionValue`,
`PriorityValue` and `FormatValue`, the enum or its strings. `receiveOwn` is
an option of `open` (`comm.open(prefix, { receiveOwn = true })`). Objects
travel as messages whose payload starts with the bytes `\255O`, so
`onMessage` and `onObject` handlers of one channel each get their own. A namespace
import (`import * as comm`) exposes a module's exported types as `comm.<Type>`
in type positions.

### Throttling

- If **ChatThrottleLib** (the shared throttle used by AceComm and many other
  addons) is loaded at send time, every chunk goes through
  `ChatThrottleLib:SendAddonMessage` with the matching priority
  (`"ALERT"`, `"NORMAL"`, `"BULK"`). This shares bandwidth with every other addon
  that uses it.
- Otherwise a built-in throttle with **conservative** limits is used, shared by
  all channels of the addon: 10 messages per prefix, regained at one a second,
  and ChatThrottleLib's byte budget (bursts of 4000 bytes, 800 a second). The
  limits are the same on every current flavor, so they are not in the flavor
  table yet.
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
the state; a secret answer counts as locked.

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
- A message with a secret prefix or text is ignored. A secret sender is passed
  to handlers as it is (not normalized, not filtered); it cannot key
  reassembly, so only single-chunk messages arrive from one.

**AceComm compatibility** (decision S16). `comm.open(prefix, { format =
"acecomm" })` switches a channel to AceComm-3.0's chunk markers and reassembly
rules instead of the wowlua format, so an addon migrating from Ace3 still
interoperates with older versions of itself in a group. The default is
`"wowlua"`. Both formats use the same throttling and lockdown handling.

## version

`wowlua:version` tells an addon which versions of itself and of other addons
are around. It shows the player nothing: a newer version only reaches the
addon through `onNewer`, and what the player sees is the addon's choice
(decision S33).

```wowlua
import * as version from "wowlua:version"

version.onNewer(function(newest: string, sender: string)
    log.info(`{newest} is out ({sender} has it); you have {version.current()}`)
end)
if version.of("OtherAddon") then … end              -- installed, with this module
if version.compare(version.current(), "2.0.0") < 0 then … end
```

| API | Signature | Notes |
|-----|-----------|-------|
| `current` | `() -> string` | This addon's build version |
| `compare` | `(a: string, b: string) -> number` | -1, 0 or 1 in semantic version order: numeric parts, a pre-release before its release, build metadata ignored |
| `installed` | `() -> { [string]: string }` | Every installed addon that uses this module, and its version |
| `of` | `(name: string) -> string?` | One addon's version; nil when it is not installed or does not use this module |
| `onNewer` | `(fn: (newest: string, sender: string) -> ()) -> Subscription` | Once for each newer version seen; right away when one was seen already |
| `newest` | `() -> string` | The newest version of this addon seen (this one's, if none is newer) |
| `peers` | `() -> { [string]: string }` | Other players' versions of this addon, by player |

Semantics:

- **Registry.** Every addon that uses this module writes its build version to
  the shared global table `WowluaVersions[addonName]` at load, so the registry
  holds every such installed addon whatever its load order
  ([Code generation §14.1.1](../toolchain/codegen.md#1411-global-writes)).
- **Exchange.** Over `wowlua:comm` with the prefix `WowluaVersion`, one prefix
  for all addons; a message is `1:<Addon>:<version>`. The addon tells its guild
  10 seconds after login and each group 3 seconds after joining it. A player
  who announces an older version is whispered this one's, once per session.
  A secret sender (see [comm](#comm)) still reaches `onNewer`, but is neither
  listed in `peers()` nor answered.
- Messages for other addons, and other protocol versions, are ignored.

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
    function exportString(value: any, opts: { version: number, prefix?: string }): string
    function importString(s: string): (boolean, any)
end
```

| API | Meaning |
|-----|---------|
| `encode` / `decode` | Compact binary format: nil, booleans, numbers (integer and float paths), strings (with deduplication), tables (array and hash parts, nested, cycles rejected). Functions, userdata and threads are errors. A secret, or a table addons may not index (`canaccesstable`), encodes as nil (a table leaves its key out); `decode` returns false for a secret string. |
| `compress` / `decompress` | DEFLATE (RFC 1951) in pure Lua. `decompress` returns `nil` on corrupt input. |
| `toPrintable` / `fromPrintable` | Encoding into a chat-safe alphabet. `fromPrintable` returns `nil` on invalid input. |
| `toAddonChannel` / `fromAddonChannel` | Encoding safe for addon message channels (escapes `\0`) |
| `exportString` / `importString` | Versioned, checksummed export strings with a configurable prefix (e.g. `"!MYADDON:1!"`). `importString` returns false for a secret string. |

`decode` and `importString` never execute code (no `loadstring`), so importing
strings from other players is safe. Depth and size limits guard against
malicious input.

### Formats

Other versions of an addon read these, so they are fixed:

- **`encode`** starts with the format version byte `1`, then one value. Each
  value is a type byte: `0` nil, `1` false, `2` true, `3`/`4` a non-negative /
  negated integer below 2^52 as a varint (7 bits per byte, low first, high
  bit continues), `5` any other finite number as a varint length and its
  `%.17g` text, `6`/`7`/`8` +inf, -inf and NaN, `9` a string (varint length,
  bytes; strings longer than one byte are numbered in order of appearance),
  `10` a reference to such a string by number, `11` a table (varint count and
  the values of its array part `1..n`, then a varint count and key/value
  pairs). Decoding allows nesting up to 100 levels.
- **`compress`** writes raw DEFLATE: stored blocks at level 0 or for
  incompressible data, else dynamic Huffman blocks. `decompress` reads any raw
  DEFLATE stream and refuses outputs over 16 MiB.
- **`toPrintable`** encodes 3 bytes as 4 characters of
  `a-z A-Z 0-9 ( )` (values 0 to 63 in that order, low bits first); a last
  group of 1 or 2 bytes takes 2 or 3 characters. Surrounding whitespace is
  ignored when reading.
- **`toAddonChannel`** escapes with byte 1: `\0` becomes `\1\1`, `\1`
  becomes `\1\2`.
- **`exportString`** is `!<prefix>:<version>!` (prefix `WL` by default) and the
  printable, compressed bytes of an Adler-32 checksum (4 bytes, big-endian) of
  the encoded value, followed by the encoded value.

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
    enum Level { Trace = "trace", Debug = "debug", Info = "info", Warn = "warn", Error = "error", Off = "off" }
    type LevelValue = Level | "trace" | "debug" | "info" | "warn" | "error" | "off"
    type Printer = ((...any) -> ()) & {
        color: (hex: string, ...any) -> (),
        to: (frame: ScrollingMessageFrame, ...any) -> (),
    }

    function trace(...: any)
    function debug(...: any)
    function info(...: any)
    function warn(...: any)
    function error(...: any)
    function setLevel(level: LevelValue)
    function sink(fn: (level: string, message: string) -> ())
    function history(n: number?): { time: number, level: string, message: string, location?: string }[]
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
