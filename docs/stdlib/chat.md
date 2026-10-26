# Chat, logging and alerts

How an addon talks to the player: chat lines with the addon's name, log
messages that are removed from release builds, links the player can click,
the chat line the player types in, and raid warnings and sounds for things
that can't wait.

## Print to chat with your addon's name

`print` from `wowlua:log` writes to the chat window with your addon's
colored name in front, so the player knows where the line comes from.

```wowlua title=src/main.wowlua
import print from "wowlua:log"

print("Hello!")                              -- MyAddon: Hello!
print("scale", UIParent:GetScale())          -- several values, joined with spaces
print.color("ff7262", "Import failed.")      -- in a color, as "rrggbb"
print.to(SELECTED_CHAT_FRAME, "Loot:", 3)    -- to the chat window whose tab is selected
print.to(ChatFrame3, "Loot:", 3)             -- to another chat window, or a message frame of your own
```

To use a prefix of your own instead of the name, set it once with
`configure`. It applies to every line `print` and the log write to chat:

```wowlua title=src/prefix.wowlua
import * as Log from "wowlua:log"

Log.configure({ prefix = "|cff9fd3c7[MA]|r " })
```

## Log messages that vanish from releases

Use `log.trace` and `log.debug` as much as you like while you work. Calls
below the build's log level are removed when the addon is compiled, with
their arguments, so the arguments are never evaluated and a release build
doesn't pay for them.

```wowlua title=src/cache.wowlua
import log from "wowlua:log"

local state = { hits = 0, misses = 0 }

export function lookup(id: number, hit: boolean)
    log.trace(`state: {state.hits} hits, {state.misses} misses`)
    log.debug("cache", id, "hit", hit)
    if not hit then
        log.warn(`missing spell {id}`)
    end
end

export function failed(err: string)
    log.error("failed:", err)
end

log.info("ready")
```

A release build keeps only the `log.warn` and `log.error` lines here. The
`trace`, `debug` and `info` calls are gone, and the template string of the
`trace` call is never built.

Each build profile has a level. The `dev` profile keeps `debug` and above,
`release` keeps `warn` and above, and `test` keeps everything. Set
`log-level` in a profile of [`wowlua.json`](../reference/manifest.md) to
change it, for example for a beta build that keeps `info`:

```json
"profiles": {
  "beta": { "inherits": "release", "log-level": "info" }
}
```

The levels are `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"` and
`"off"`. The functions are also exported on their own, so
`import * as Log from "wowlua:log"` with `Log.info(…)` works too.

### Where log messages go

By default, warnings and errors are printed to chat. Every message that is
logged also goes into a history of the last 500 entries, with the time, the
level and, in debug builds, the file and line it came from. In a dev build,
`/wl log MyAddon` shows it (see [debugging in game](../guide/debugging.md)).
The manifest key `stdlib.log.history-size` changes the size of the history.

```wowlua title=src/debugToggle.wowlua
import * as Log from "wowlua:log"

local logFrame = CreateFrame("ScrollingMessageFrame", nil, UIParent)

-- a "debug output" checkbox for players who help you find a bug
export function setDebugOutput(on: boolean)
    Log.configure({ chatLevel = on and "debug" or "warn" })
end

-- quieter at run time (never louder than the build allows)
Log.setLevel("error")

-- send messages somewhere else as well
Log.sink(function(level: string, message: string)
    logFrame:AddMessage(`[{level}] {message}`)
end)

-- the newest 20 entries, oldest first
for _, entry in ipairs(Log.history(20)) do
    print(entry.time, entry.level, entry.message)
end
```

`chatLevel` is the lowest level printed to chat (`"off"` prints none).
`setLevel` changes the level at run time, as `/wl log MyAddon error` does in
game. It can make the log quieter, but a call the build removed stays
removed. Secret values are logged as `<secret>`.

Call the level functions directly. Using one as a value, such as handing
`trace` to another function as a callback, is a build error, because the
compiler couldn't remove the call. Wrap it in a function instead:

```wowlua fails=E8118
import trace from "wowlua:log"

local report = trace
report("never removed")
```

```text
error[E8118]: a log level function can only be called directly, so that the compiler can remove it; wrap it in a function
  ┌─ src/main.wowlua:3:16
  │
3 │ local report = trace
  │                ^^^^^ here

error: aborting due to 1 previous error
```

## Links players can click

`wowlua:links` puts a clickable link into a chat line, and calls your
handler when the player clicks it. Only your addon receives clicks on your
links.

```wowlua title=src/import.wowlua
import * as links from "wowlua:links"
import print from "wowlua:log"

print(`Import failed. {links.link("Show details", "import-log")}`)

links.onClick(function(data: string, button: string)
    if data == "import-log" and button == "LeftButton" then
        print("1 entry could not be read.")
    end
end)
```

The link shows `[Show details]`, colored like a spell link. A color from
`wowlua:color` as the third argument changes that. The data string may not
contain `|`. The game doesn't send these links to other players: they only
work in the player's own chat windows.

## The chat line and chat windows

`wowlua:chat` opens the line the player types in, through the game's own
chat code, the way its own buttons do.

```wowlua title=src/roster.wowlua
import * as chat from "wowlua:chat"

export function whisperTo(name: string)
    chat.whisper(name)                       -- "/w Name " ready to type
end

export function shareLink(link: string)
    chat.insertLink(link)                    -- as a shift-click does
end

chat.open("/roll 100")                       -- the chat line with text in it

-- hide lines: return true to drop one
local filter = chat.filter("CHAT_MSG_CHANNEL", function(text: unknown)
    return type(text) == "string" and text:find("WTS") ~= nil
end)

export function showTradeAgain()
    filter:cancel()
end
```

`insertLink` opens a chat line when none is open. `whisperBnet` opens a
whisper to a Battle.net friend by the account name the game gives you. A
filter that raises an error is reported, and the line is shown.

Chat addons that restyle or mirror the chat windows can find them:

| Function | Returns |
|----------|---------|
| `chat.windows()` | Every chat window: the numbered ones, then the temporary ones |
| `chat.docked()` | The windows docked with General, in the dock's order |
| `chat.window("ChatFrame3")` | The chat window with that global name, or nil |
| `chat.editBox(w)` | The window's edit box |
| `chat.isTemporary(w)` | Whether it is a temporary window, such as a whisper window |
| `chat.conversation(w)` | A temporary window's chat type and target (`"WHISPER"`, the player) |

## Get the player's attention

For moments that can't wait, `wowlua:alert` shows large text in the middle
of the screen, plays a sound and flashes the taskbar icon of a game in the
background. `warn` does all three at once.

```wowlua title=src/interrupts.wowlua
import * as alert from "wowlua:alert"

alert.warn("Interrupt!", { sound = SOUNDKIT.RAID_WARNING, flash = true })

alert.sound(SOUNDKIT.READY_CHECK)            -- a sound kit
alert.sound("MyAddon Ping", "SFX")           -- a sound registered with wowlua:media
alert.raidWarning("Pull in 5")               -- text only
alert.flash()                                -- taskbar only
```

- The same `warn` text again within a second does nothing, so an event that
  fires five times in a burst gives one warning. `throttle` sets the window
  in seconds.
- Sounds play on the `"Master"` channel unless you pass another:
  `"SFX"`, `"Music"`, `"Ambience"` or `"Dialog"`. `sound` returns whether the
  sound plays.
- A number below 1000000 is a sound kit, other numbers and paths are sound
  files, and names are looked up in
  [`wowlua:media`](ui.md#fonts-textures-and-sounds).
- Raid warnings are orange unless you pass a `color`.

## At a glance

| Module | Functions |
|--------|-----------|
| `wowlua:log` | `print`, `print.color`, `print.to`; `trace`, `debug`, `info`, `warn`, `error`; `configure`, `setLevel`, `sink`, `history` |
| `wowlua:links` | `link`, `onClick` |
| `wowlua:chat` | `whisper`, `whisperBnet`, `open`, `insertLink`, `filter`; `windows`, `docked`, `window`, `editBox`, `isTemporary`, `conversation` |
| `wowlua:alert` | `warn`, `raidWarning`, `sound`, `flash` |

> [!TIP]
> In a test, `mock.printed()` returns every chat line your addon printed, so
> you can check messages with `expect(…).toContain(…)`. See
> [testing your addon](../guide/testing.md).
