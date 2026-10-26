# Talking to other addons

Send data between players, share profiles as strings, notice newer versions
of your addon, and give other addons a typed API to subscribe to.

Four modules cover this. `wowlua:comm` sends addon messages of any length,
`wowlua:serialize` turns values into strings and back, `wowlua:version`
notices when someone in your guild or group has a newer version, and
`wowlua:callbacks` lets other code subscribe to what yours does. They are
compiled into your addon like the rest of the standard library, so there is
no library to embed.

## Send a message to your group

Open a channel with your prefix, listen with `onMessage` and send with
`send`. `open` registers the prefix with the game. A message can be any
length: it is split into chunks, and the receiver gets it back in one piece.

```wowlua title=src/sync.wowlua
import * as comm from "wowlua:comm"
import print from "wowlua:log"

export const channel = comm.open("MyRaidTools")

channel:onMessage(function(msg: string, sender: string, distribution: string)
    if msg == "ping" then
        channel:send("pong", comm.Distribution.Whisper, sender)
    elseif msg == "pong" then
        print(`{sender} has MyRaidTools ({distribution})`)
    end
end)

export function ping()
    channel:send("ping", comm.Distribution.Group)
end
```

A prefix has 1 to 16 characters; a longer literal is a build error (E8050).

`Distribution.Group` picks the channel for the player's current group:
instance chat, raid or party. Out of a group the message is dropped. The
other distributions are the game's own: `Party`, `Raid`, `Instance`,
`Guild`, `Officer`, `Whisper` and `Channel`. Their strings work too
(`"GUILD"`, `"INSTANCE_CHAT"`).

The sender is a plain name for players of your realm and `Name-Realm` for
others. Your own messages are filtered out, unless you open the channel with
`comm.open(prefix, { receiveOwn = true })`.

### Big messages without a disconnect

Every message goes through a throttle, so a large send doesn't flood the
server. When another addon has loaded ChatThrottleLib, the throttle most
addons share, chunks go through it. Otherwise wowlua uses a conservative
throttle of its own. Give bulk data a low priority, and follow its progress
with a callback:

```wowlua title=src/share.wowlua
import * as comm from "wowlua:comm"
import print from "wowlua:log"
import channel from "./sync"

local progress = CreateFrame("StatusBar", nil, UIParent)

export function share(data: string, target: string)
    local status = channel:send(data, "WHISPER", target, {
        priority = comm.Priority.Bulk,
        callback = function(sent: number, total: number)
            progress:SetValue(sent / total)
        end,
    })
    if status == comm.SendStatus.Queued then
        print("sending in the background…")
    end
end
```

`send` returns what happened to the message: `Sent`, `Queued` (throttled, or
addon chat is locked) or `Dropped`. The priorities are `Alert`, `Normal` (the
default) and `Bulk`.

The game locks addon chat in some situations, for example during
encounters. Queued messages wait and go out when the lockdown ends. Pass
`dropInLockdown = true` for messages that are only useful right now, and call
`comm.inLockdown()` when your interface should say so.

### Send tables directly

`sendObject` serializes a value before sending it, and `onObject` hands the
receiver the rebuilt value, so you don't write a string protocol by hand.
Objects and plain messages on one channel don't mix: each handler only gets
its own kind.

```wowlua title=src/loot.wowlua
import * as comm from "wowlua:comm"
import print from "wowlua:log"
import channel from "./sync"

interface LootRoll
    item: number
    roll: number
end

channel:onObject(function(value: any, sender: string)
    local roll = value :: LootRoll
    print(`{sender} rolled {roll.roll} on item {roll.item}`)
end)

export function announce(item: number, roll: number)
    const entry: LootRoll = { item = item, roll = roll }
    channel:sendObject(entry, comm.Distribution.Raid)
end
```

The value arrives typed `any`, because another player's addon sent it.
Check it before you trust its shape.

| Function | What it does |
|----------|--------------|
| `comm.open(prefix, opts?)` | A channel for one prefix |
| `channel:send(msg, distribution, target?, opts?)` | Sends a string of any length |
| `channel:onMessage(fn)` | Receives complete messages |
| `channel:sendObject(value, …)`, `channel:onObject(fn)` | Sends and receives serialized values |
| `comm.inLockdown()` | Whether addon chat is locked right now |

An addon moving from AceComm can open its channel with
`{ format = "acecomm" }` to keep talking to older versions of itself that
still use AceComm's format.

## Import and export strings

For profile sharing and other strings players copy and paste,
`exportString` encodes a value, compresses it, turns it into characters that
are safe to paste, and adds a header with a version and a checksum.
`importString` reverses it. It never runs code, so a string from a stranger
is safe to import.

```wowlua title=src/profiles.wowlua
import * as serialize from "wowlua:serialize"
import print from "wowlua:log"

export function exportProfile(profile: table): string
    return serialize.exportString(profile, { version = 1, prefix = "MYUI" })   -- "!MYUI:1!…"
end

export function importProfile(text: string): unknown
    local ok, value = serialize.importString(text)
    if not ok then
        print("That doesn't look like a profile string:", value)
        return nil
    end
    return value
end
```

On failure, the second value is the reason. The prefix defaults to `WL`.
`importString` reads a string with any prefix and doesn't return the
version, so check the start of the string yourself if you need to tell your
strings from another addon's.

Each step is available on its own as well:

```wowlua title=src/codec.wowlua
import * as serialize from "wowlua:serialize"

local data = { name = "Raid", spells = { 133, 116 } }

local packed = serialize.compress(serialize.encode(data))
local printable = serialize.toPrintable(packed)      -- chat-safe characters

local raw = serialize.fromPrintable(printable)       -- nil if it was changed
local text = raw and serialize.decompress(raw)       -- nil if it is corrupt
if text then
    local ok, back = serialize.decode(text)
    print(ok, back)
end
```

The encoding handles nil, booleans, numbers, strings and nested tables, and
a string that appears several times is stored once. Functions can't be
encoded, and a table that contains itself is rejected. Secret values (see
[units, auras, spells and items](game.md#first-secret-values)) are written
as nil, because no addon may read them.

| Function | What it does |
|----------|--------------|
| `exportString`, `importString` | Versioned, checksummed strings for players to copy |
| `encode`, `decode` | A compact binary form of a value |
| `compress`, `decompress` | DEFLATE, in plain Lua |
| `toPrintable`, `fromPrintable` | Text safe to paste in chat |
| `toAddonChannel`, `fromAddonChannel` | Text safe for addon messages |

## Tell players when an update is out

With `wowlua:version` imported, your addon compares versions with guild
members and group members in the background. It never shows the player
anything by itself: you decide what happens when a newer version turns up.

```wowlua title=src/update.wowlua
import * as version from "wowlua:version"
import print from "wowlua:log"

version.onNewer(function(newest: string, sender: string)
    print(`version {newest} is out ({sender} has it), you have {version.current()}`)
end)

-- other addons that use wowlua:version are listed too
if version.of("MyRaidTools") then
    print("MyRaidTools is installed")
end
```

Versions compare as semantic versions: `1.10.0` is newer than `1.9.2`, and
`2.0.0-beta.1` comes before `2.0.0`. Your addon tells its guild 10 seconds
after login and each group 3 seconds after joining it. A player who
announces an older version gets yours in a whisper, once per session.

| Function | Returns |
|----------|---------|
| `version.current()` | This build's version |
| `version.onNewer(fn)` | Calls `fn` once for each newer version seen |
| `version.newest()` | The newest version seen so far |
| `version.peers()` | Other players' versions of your addon, by player |
| `version.of(name)`, `version.installed()` | Installed addons that use this module, and their versions |
| `version.compare(a, b)` | -1, 0 or 1 |

## Let other addons react to yours

A registry is a typed set of events an object fires. Describe the events in
an interface, and every `on` and `fire` is checked against it: a misspelled
name or a wrong argument is a build error.

```wowlua title=src/tracker.wowlua
import * as callbacks from "wowlua:callbacks"

export interface TrackerEvents
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

    function reset()
        self.kills = 0
        self.events:fire("Reset")
    end
end
```

```wowlua title=src/counter.wowlua
import Tracker from "./tracker"

local tracker = Tracker.new()
local panel = CreateFrame("Frame", nil, UIParent)
local counter = panel:CreateFontString(nil, "OVERLAY", "GameFontNormal")

tracker.events:on("KillRecorded", function(count: number)
    counter:SetText(tostring(count))
end, panel)

-- when the panel goes away, drop everything it subscribed
panel:SetScript("OnHide", function()
    tracker.events:offAll(panel)
end)
```

The third argument of `on` is an owner, so you can remove all of its
handlers in one call. Handlers run in the order they subscribed, and one
that raises an error is reported without stopping the rest.

| Method | What it does |
|--------|--------------|
| `callbacks.registry(opts?)` | A typed registry |
| `registry:on(name, fn, owner?)`, `registry:once(…)` | Subscribes, optionally with an owner |
| `registry:off(sub)`, `registry:off(name, fn)` | Removes one subscription, or every one of a handler |
| `registry:offAll(owner, name?)` | Removes an owner's subscriptions |
| `registry:fire(name, …)` | Calls every handler |
| `registry:has(name)` | Whether anything listens |

### A public API for other addons

To offer a registry to other addons, put it in a table marked `@global`,
the way wowlua lets you create a global on purpose. With
`compat = "CallbackHandler"` it also has the `RegisterCallback`,
`UnregisterCallback` and `UnregisterAllCallbacks` methods that plain Lua
addons know from CallbackHandler.

```wowlua title=src/api.wowlua
import * as callbacks from "wowlua:callbacks"
import type TrackerEvents from "./tracker"

@global("MyRaidToolsAPI")
local api = {
    events = callbacks.registry({ compat = "CallbackHandler" }) :: callbacks.Registry<TrackerEvents>,
}
```

Another addon, in plain Lua:

```lua
MyRaidToolsAPI.events.RegisterCallback(self, "KillRecorded", function(event, count)
    print("kills:", count)
end)
```
