# Standard library

The standard library is a set of `wowlua:*` modules for what nearly every WoW
addon needs: events, saved variables, slash commands, timers, an options page,
a minimap button and more. This page shows how to use them and which module
covers what.

## Import what you use

Nothing from the standard library is in scope until you import it. The modules
have names that start with `wowlua:`, and you import from them as from any
other module.

```wowlua fragment
import event from "wowlua:events"
import * as saved from "wowlua:saved"
import after, throttle from "wowlua:timer"
```

The modules are written in wowlua and ship inside the compiler. Only the
modules your addon uses are compiled into it, and of those only the code it
uses. Their version is the compiler's version.

## A small addon, start to finish

Six imports cover what most addons write by hand or embed from libraries:
saved settings, a login hook, an event handler that doesn't run too often, and
a slash command.

```wowlua title=src/main.wowlua
import onLogin from "wowlua:lifecycle"
import event from "wowlua:events"
import slash from "wowlua:slash"
import * as saved from "wowlua:saved"
import throttle from "wowlua:timer"
import print from "wowlua:log"

interface Settings
    announce: boolean
    logins: number
end

local db = saved.bind("MyAddonDB", {
    version = 1,
    defaults = { announce = true, logins = 0 } :: Settings,
})

@onLogin
local function greet()
    db.logins += 1
    if db.announce then
        print(`login number {db.logins}`)
    end
end

@event("PLAYER_TARGET_CHANGED")
@throttle(1)
local function targetChanged()
    print("target:", UnitName("target") or "none")
end

@slash("/mya", "/myaddon")
local function command(msg: string)
    if msg == "announce" then
        db.announce = not db.announce
        print("announce", db.announce and "on" or "off")
    end
end
```

Declare the SavedVariable in the manifest, as you would in a TOC file. wowlua
generates the TOC from it.

```json title=wowlua.json
{
  "$schema": "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/wowlua-0.1.json",
  "package": {
    "name": "my-addon",
    "version": "0.1.0",
    "wowlua": "^0.12"
  },
  "addon": {
    "title": "MyAddon",
    "saved-variables": ["MyAddonDB"]
  }
}
```

There is no event frame with an `OnEvent` script, no `SLASH_MYADDON1` global
and no `ADDON_LOADED` handler that checks the addon's name. The decorators
generate that registration code when the addon is compiled.

## What you get over embedding libraries

- **Your own copy.** Each addon gets a private copy of the modules it uses,
  compiled into it. Two addons built with different wowlua versions never
  share, or fight over, a library.
- **Types.** Event payloads, slash command arguments, settings keys and saved
  data are typed and checked. A misspelled event name is a compile error, not a
  handler that never runs.
- **Fits each client.** The modules adapt to the game flavor at compile time,
  and use only what that client has. Where a module needs an API a flavor
  lacks, it has a fallback (warning W8301 when the fallback does less) or
  reports error E8300.
- **Safe around startup.** Modules that need saved data or the UI wait for the
  right moment by themselves, so you can set them up at the top of a file.
- **One failing handler doesn't stop the others.** Every callback runs
  isolated. Its error goes to the game's error handler with your addon's name
  in front, so error display addons still catch it.

```text
error[E8003]: unknown event or message `PLAYER_TARGET_CHANGD`
   ┌─ src/main.wowlua:26:8
   │
26 │ @event("PLAYER_TARGET_CHANGD")
   │        ^^^^^^^^^^^^^^^^^^^^^^ not declared

error: aborting due to 1 previous error
```

A typo in an event name is caught before the game loads the file. The
[WoW API reference](../reference/api.md) lists every event and function.

## What you no longer write by hand

| The chore | The module |
|-----------|------------|
| Registering events on a frame, coalescing bursts of them | `wowlua:events` |
| Waiting for `ADDON_LOADED` and `PLAYER_LOGIN` | `wowlua:lifecycle` |
| Defaults, profiles and migrations of SavedVariables | `wowlua:saved` |
| Parsing slash command text; printing with your addon's name | `wowlua:slash`, `wowlua:log` |
| Keeping track of timers; throttling and debouncing | `wowlua:timer` |
| Building an options panel | `wowlua:settings` |
| A draggable minimap button | `wowlua:launcher` |
| Translation tables | `wowlua:locale` |
| Splitting addon messages into chunks; serializing and compressing data | `wowlua:comm`, `wowlua:serialize` |
| Callbacks other addons can register for | `wowlua:callbacks` |
| Post-hooks on Blizzard's functions and scripts | `wowlua:hooks` |

> [!TIP]
> You don't have to switch everything at once. The libraries your addon
> embeds keep working as [dependencies](../guide/packages.md) next to the
> standard library.

## Find the module for the job

### Events and startup

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:events` | Handle WoW events, unit events and your own messages | [Events and lifecycle](events.md) |
| `wowlua:lifecycle` | Run code when your addon loads, the player logs in or out, or another addon loads | [Events and lifecycle](events.md) |

### Data and commands

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:saved` | Keep typed SavedVariables with defaults, profiles and migrations | [Saved variables](saved.md) |
| `wowlua:slash` | Register slash commands with subcommands and typed arguments | [Slash commands](slash.md) |
| `wowlua:timer` | Delay, repeat, throttle, debounce, defer to the next frame | [Timers and background jobs](timers.md) |
| `wowlua:jobs` | Spread heavy work over many frames | [Timers and background jobs](timers.md) |

### Options and access

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:settings` | Build an options page under Options › AddOns, bound to your saved data | [Settings, minimap, keybindings](settings.md) |
| `wowlua:launcher` | Add a minimap button, an Addon Compartment entry and a data broker launcher | [Settings, minimap, keybindings](settings.md) |
| `wowlua:bindings` | Declare keybindings; the build writes `Bindings.xml` | [Settings, minimap, keybindings](settings.md) |
| `wowlua:locale` | Translate from JSON files through a typed `L` | [Localization](locale.md) |

### Building UI

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:frames` | Make frames movable and resizable, with a saved position | [Frames, lists and menus](ui.md) |
| `wowlua:pool`, `wowlua:list` | Reuse frames and tables; build scrolling lists on pools | [Frames, lists and menus](ui.md) |
| `wowlua:menu`, `wowlua:dialog` | Open context menus; confirm and import/export dialogs | [Frames, lists and menus](ui.md) |
| `wowlua:media` | Use shared fonts, textures and sounds (SharedMedia) | [Frames, lists and menus](ui.md) |

### Talking to the player

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:log` | Log by level, and print to the chat with your addon's name | [Chat, logging and alerts](chat.md) |
| `wowlua:chat` | Whisper, put a link in the chat line, or open it with text | [Chat, logging and alerts](chat.md) |
| `wowlua:links` | Put clickable links in the player's chat | [Chat, logging and alerts](chat.md) |
| `wowlua:alert` | Play sounds, show raid warnings, flash the taskbar | [Chat, logging and alerts](chat.md) |

### Other addons and players

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:comm`, `wowlua:serialize` | Send addon messages with chunking and throttling; turn tables into strings | [Talking to other addons](comm.md) |
| `wowlua:callbacks` | Offer callbacks and a public API to other addons | [Talking to other addons](comm.md) |
| `wowlua:version` | See installed versions, and newer ones in your guild and group | [Talking to other addons](comm.md) |

### Game data

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:unit`, `wowlua:aura` | Read units, the group roster and auras on every flavor | [Units, auras, spells, items](game.md) |
| `wowlua:spell`, `wowlua:item` | Read spell information; item information that waits for the server | [Units, auras, spells, items](game.md) |
| `wowlua:tooltip` | Add lines to tooltips with typed post-hooks | [Units, auras, spells, items](game.md) |
| `wowlua:secret` | Work with secret values | [Units, auras, spells, items](game.md) |

### Combat, secure code and errors

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:combat` | Wait for the end of combat before touching protected frames | [Combat, secure code, hooks](combat.md) |
| `wowlua:secure` | Show frames and switch states by macro conditionals | [Combat, secure code, hooks](combat.md) |
| `wowlua:hooks` | Post-hook Blizzard's functions and scripts, also of addons that load later | [Combat, secure code, hooks](combat.md) |
| `wowlua:errors` | Set how errors from your handlers are caught and reported | [Combat, secure code, hooks](combat.md) |

### Utilities

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:table`, `wowlua:collections` | Copy, merge and compare tables; ring buffer, heap, LRU cache | [Tables, text and formatting](utils.md) |
| `wowlua:utf8` | Count, cut and truncate UTF-8 text safely | [Tables, text and formatting](utils.md) |
| `wowlua:format` | Show durations, dates, money and numbers as the client does | [Tables, text and formatting](utils.md) |
| `wowlua:color` | Use class colors and color escapes | [Tables, text and formatting](utils.md) |

### Build, debug and test

| Module | Use it to | Page |
|--------|-----------|------|
| `wowlua:meta` | Read your addon's title, version and build profile | [Build info and profiles](../language/flavors.md) |
| `wowlua:debug`, `wowlua:reflect` | Assert, dump and profile; read class identity | [Debugging helpers](../language/debugging.md) |
| `wowlua:test`, `wowlua:mock` | Write tests against the emulated game world (test files only) | [Testing your addon](../guide/testing.md) |

The complete module list, with links to the normative chapters, is in the
[specification](../../spec/stdlib/README.md#82-modules).
