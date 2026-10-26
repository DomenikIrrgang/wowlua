# x8xxx · stdlib

## E8001

`send` from `wowlua:events` dispatches a message to your own addon's handlers.
WoW events such as `PLAYER_LOGIN` come from the game client and can't be sent
by an addon, so passing one to `send` is an error.

Declare your own message with `declare message` and send that instead;
handlers subscribe to it with `on` like to any event.

```wowlua fails
import send from "wowlua:events"
send("PLAYER_LOGIN")
```

```wowlua compiles
import on, send from "wowlua:events"

declare message MYADDON_READY()

on("MYADDON_READY", function() print("ready") end)
send("MYADDON_READY")
```

Related: E8003

## E8002

`@event`, `@bucket` and `@unitEvent` register a function when the module
(or the class) is defined. An instance method has no object at that time, so
these decorators can't go on one; on a static method they are fine.

Subscribe in the constructor instead, with `on(name, handler, self)` (or
`onBucket` / `onUnit`), and call `offAll(self)` when the object is discarded.

```wowlua fails
import event from "wowlua:events"

class Tracker
    @event("PLAYER_LOGIN")
    function onLogin()
        print("logged in")
    end
end
```

```wowlua compiles
import on from "wowlua:events"

class Tracker
    constructor()
        on("PLAYER_LOGIN", function() self:onLogin() end, self)
    end

    function onLogin()
        print("logged in")
    end
end

local tracker = Tracker.new()
print(tracker)
```

Related: E8003

## E8003

An event or message name given to `wowlua:events` (`on`, `once`, `@event`,
`@bucket`, `onUnit`, `watchUnit`, `offAll`, `send`) is checked against the game's
events and the messages your addon declares. This name is neither, usually a
typo or an event that was removed in a patch. `onUnit`, `@unitEvent` and
`watchUnit` also take only game events, not messages; `send` takes only
declared messages.

Fix the spelling, or declare your own message with `declare message NAME(…)`.
In `split` flavor mode an event missing from one flavor is reported for that
flavor.

```wowlua fails
import on from "wowlua:events"
on("PLAYER_LOGN", function() print("hi") end)
```

```wowlua compiles
import on from "wowlua:events"
on("PLAYER_LOGIN", function() print("hi") end)
```

Related: E8001, W8101

## E8010

The same slash command is registered twice in the addon, with `@slash` or the
slash module. Commands are compared without case, so `/mytool` and `/MyTool`
are the same command; the game would run only one of the handlers.

Register each command once. To give one handler several commands, list them
all in one `@slash(...)`.

```wowlua fails
import slash from "wowlua:slash"

@slash("/mytool")
local function open(msg: string) print(msg) end

@slash("/MyTool")
local function toggle(msg: string) print(msg) end
```

```wowlua compiles
import slash from "wowlua:slash"

@slash("/mytool", "/mt")
local function open(msg: string) print(msg) end
```

Related: E8101, W8011

## E8020

`saved.bind` names a SavedVariable that the manifest doesn't declare. The game
only saves variables that the TOC lists, and wowlua writes the TOC from
`addon.saved-variables` and `addon.saved-variables-per-character`, so this data
would never be saved.

Add the name to one of the two lists in `wowlua.json`, or fix its spelling.

```wowlua fails
import * as saved from "wowlua:saved"
local db = saved.bind("MyAddonDB", { version = 1, defaults = { scale = 1 } })
print(db)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["MyAddonDB"] }
}
```

```wowlua compiles title=src/main.wowlua
import * as saved from "wowlua:saved"
local db = saved.bind("MyAddonDB", { version = 1, defaults = { scale = 1 } })
print(db)
```

Related: E8102, E8103

## E8021

`saved.bind` with `profiles = true` keeps the active profile of each character
in a second, per-character SavedVariable named after the first with `PerChar`
appended (`MyAddonDB` uses `MyAddonDBPerChar`). That variable isn't declared,
so the profile choice would be lost at logout.

Add `<name>PerChar` to `addon.saved-variables-per-character` in `wowlua.json`.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["MyAddonDB"] }
}
```

```wowlua fails title=src/main.wowlua
import * as saved from "wowlua:saved"
local db = saved.bind("MyAddonDB", { version = 1, defaults = { scale = 1 }, profiles = true })
print(db)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": {
        "saved-variables": ["MyAddonDB"],
        "saved-variables-per-character": ["MyAddonDBPerChar"]
    }
}
```

```wowlua compiles title=src/main.wowlua
import * as saved from "wowlua:saved"
local db = saved.bind("MyAddonDB", { version = 1, defaults = { scale = 1 }, profiles = true })
print(db)
```

Related: E8020, E8117

## E8030

`@throttle`, `@debounce` and `@nextFrame` from `wowlua:timer`, and
`@outOfCombat` from `wowlua:combat`, may run the function later than it was
called, or merge several calls into one. The caller never gets a result back,
so the decorated function must not return values.

Remove the `return` values. If the caller needs a result, pass it on another
way, such as a field the function sets or a callback it calls.

```wowlua fails
import throttle from "wowlua:timer"

@throttle(0.5)
local function refresh(): boolean
    print("refresh")
    return true
end
refresh()
```

```wowlua compiles
import throttle from "wowlua:timer"

@throttle(0.5)
local function refresh()
    print("refresh")
end
refresh()
```

## E8050

`comm.open` registers an addon message prefix with the game, which takes 1 to
16 characters. A literal prefix that is empty or longer than that is reported
at compile time; the client would refuse to register it.

Use a shorter prefix, such as your addon's name or an abbreviation of it.

```wowlua fails
import * as comm from "wowlua:comm"
local channel = comm.open("MyVeryLongAddonPrefix")
print(channel)
```

```wowlua compiles
import * as comm from "wowlua:comm"
local channel = comm.open("MyAddon")
print(channel)
```

## E8060

A key in a locale file is not a valid locale key. Keys are made of upper-case
letters, digits and `_` only (`[A-Z0-9_]+`), so that each one is a field of
`L`. Rename the key in every locale file and in the code that reads it.

The locale files are read only when the package imports `wowlua:locale`. A key
that starts with a digit is allowed, but it is reachable only as `L["1ST"]`.

```json fails title=locales/enUS.json
{ "welcome-text": "Welcome back!" }
```
```wowlua fails title=src/main.wowlua
import L from "wowlua:locale"
print(L)
```

```json compiles title=locales/enUS.json
{ "WELCOME_TEXT": "Welcome back!" }
```
```wowlua compiles title=src/main.wowlua
import L from "wowlua:locale"
print(L.WELCOME_TEXT)
```

Related: E8061, E8115

## E8061

A translation file has a key that the base locale (`enUS.json` by default,
`build.locale-base` names another) does not have. The base locale defines the
keys, so a key that exists only in a translation is a typo or a string that was
renamed or removed. Fix the key, or add it to the base locale too.

```json fails title=locales/enUS.json
{ "TITLE": "Kill Tracker" }
```
```json fails title=locales/deDE.json
{ "TITLE": "Kill-Tracker", "SUBTITLE": "Statistik" }
```
```wowlua fails title=src/main.wowlua
import L from "wowlua:locale"
print(L.TITLE)
```

```json compiles title=locales/enUS.json
{ "TITLE": "Kill Tracker", "SUBTITLE": "Statistics" }
```
```json compiles title=locales/deDE.json
{ "TITLE": "Kill-Tracker", "SUBTITLE": "Statistik" }
```
```wowlua compiles title=src/main.wowlua
import L from "wowlua:locale"
print(L.TITLE, L.SUBTITLE)
```

Related: W8062, E8060

## E8063

A translation uses other placeholders than the base locale's string for the
same key. A key with placeholders (`{name}`) becomes a function whose
parameters are the base locale's placeholder names, so a translation must use
exactly the same names. It may put them in any order and repeat them. `{{` and
`}}` are literal braces, not placeholders.

```json fails title=locales/enUS.json
{ "WELCOME": "Welcome back, {name}!" }
```
```json fails title=locales/deDE.json
{ "WELCOME": "Willkommen zurück, {spieler}!" }
```
```wowlua fails title=src/main.wowlua
import L from "wowlua:locale"
print(L.WELCOME({ name = "Thrall" }))
```

```json compiles title=locales/enUS.json
{ "WELCOME": "Welcome back, {name}!" }
```
```json compiles title=locales/deDE.json
{ "WELCOME": "Willkommen zurück, {name}!" }
```
```wowlua compiles title=src/main.wowlua
import L from "wowlua:locale"
print(L.WELCOME({ name = "Thrall" }))
```

Related: E8061

## E8070

A `wowlua:settings` control is bound to a store field of a type it can't edit.
A `checkbox` needs a `boolean` field, a `slider` a `number` field and a `color`
a `{ r: number, g: number, b: number, a: number? }` field. The compiler checks
each control in the `controls` list against the type of `store` (what
`saved.bind` returns). Use the control that fits the field, or change the
field's default value to the type the control needs.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["PlaygroundDB"] }
}
```
```wowlua fails title=src/main.wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"

local db = saved.bind("PlaygroundDB", { version = 1, defaults = { scale = 1, locked = false } })

settings.register({
    title = "Playground",
    store = db,
    controls = { settings.checkbox("scale", { label = "Scale" }) },
})
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["PlaygroundDB"] }
}
```
```wowlua compiles title=src/main.wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"

local db = saved.bind("PlaygroundDB", { version = 1, defaults = { scale = 1, locked = false } })

settings.register({
    title = "Playground",
    store = db,
    controls = {
        settings.slider("scale", { label = "Scale", min = 0.5, max = 2, step = 0.1 }),
        settings.checkbox("locked", { label = "Lock frame" }),
    },
})
```

Related: E8113, E8107

## E8080

`launcher.register` is called more than once. An addon has one launcher: one
minimap button, one Addon Compartment entry and one LDB object. The compiler
counts the calls that are statements at the top level of your files, and
reports a second one. Any other second call, such as one in a function that
runs twice or in a branch, raises an error at run time instead.

Register once, and change the launcher later with `launcher.setIcon`,
`setLabel`, `showMinimap` and `refresh`.

```wowlua fails
import * as launcher from "wowlua:launcher"

launcher.register({ icon = "Interface\\Icons\\INV_Misc_QuestionMark", label = "Playground", onClick = function() end })
launcher.register({ icon = "Interface\\Icons\\INV_Misc_Book_09", label = "Playground", onClick = function() end })
```

```wowlua compiles
import * as launcher from "wowlua:launcher"

launcher.register({ icon = "Interface\\Icons\\INV_Misc_QuestionMark", label = "Playground", onClick = function() end })
launcher.setIcon("Interface\\Icons\\INV_Misc_Book_09")
```

Related: E8114

## E8101

A slash command name must start with `/`, have at least one character after it
and contain no spaces or other whitespace (`"/mytool"`). The game can't register
anything else as a command.

Add the leading `/`, or remove the spaces.

```wowlua fails
import slash from "wowlua:slash"

@slash("mytool")
local function open(msg: string) print(msg) end
```

```wowlua compiles
import slash from "wowlua:slash"

@slash("/mytool")
local function open(msg: string) print(msg) end
```

Related: E8010, W8011

## E8102

`saved.bind` takes the SavedVariable's name as a string literal, so the
compiler can check it against the manifest (E8020) and make sure it is bound
only once (E8103). A variable or any other expression can't be checked.

Write the name directly in the call.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["MyAddonDB"] }
}
```

```wowlua fails title=src/main.wowlua
import * as saved from "wowlua:saved"
local name = "MyAddonDB"
local db = saved.bind(name, { version = 1, defaults = {} })
print(db)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["MyAddonDB"] }
}
```

```wowlua compiles title=src/main.wowlua
import * as saved from "wowlua:saved"
local db = saved.bind("MyAddonDB", { version = 1, defaults = {} })
print(db)
```

Related: E8020, E8103

## E8103

The same SavedVariable is bound with `saved.bind` twice in the addon. Each
binding would load, fill with defaults and strip the same data on its own, so
the two tables would fight over it.

Bind it once, in one module, and export the returned table to the modules that
need it.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["MyAddonDB"] }
}
```

```wowlua fails title=src/main.wowlua
import * as saved from "wowlua:saved"
local db = saved.bind("MyAddonDB", { version = 1, defaults = {} })
local again = saved.bind("MyAddonDB", { version = 1, defaults = {} })
print(db, again)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["MyAddonDB"] }
}
```

```wowlua compiles title=src/db.wowlua
import * as saved from "wowlua:saved"
export const db = saved.bind("MyAddonDB", { version = 1, defaults = {} })
```

```wowlua compiles title=src/main.wowlua
import db from "/db"
print(db)
```

Related: E8020, E8102

## E8104

The type of the defaults given to `saved.bind` contains a `Secret<…>` type,
somewhere inside it (the message names the field). Secret values can't be
compared or saved: the module drops them at logout, so such a field would never
keep its value.

Store a plain value type instead, and keep secret values in memory only.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["MyAddonDB"] }
}
```

```wowlua fails title=src/main.wowlua
import * as saved from "wowlua:saved"

interface Settings
    lastTarget: Secret<string>?
end

local db = saved.bind("MyAddonDB", { version = 1, defaults = {} :: Settings })
print(db)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["MyAddonDB"] }
}
```

```wowlua compiles title=src/main.wowlua
import * as saved from "wowlua:saved"

interface Settings
    lastTarget: string?
end

local db = saved.bind("MyAddonDB", { version = 1, defaults = {} :: Settings })
print(db)
```

## E8105

A file in the locales folder (`locales/` by default, `build.locales-dir` names
another) is not named by a locale code. Only `*.json` files are read, and each
one must be called after the client locale it translates: `enUS`, `enGB`,
`deDE`, `esES`, `esMX`, `frFR`, `itIT`, `koKR`, `ptBR`, `ruRU`, `zhCN` or
`zhTW`, plus `.json`. Rename the file.

```json fails title=locales/enUS.json
{ "TITLE": "Kill Tracker" }
```
```json fails title=locales/german.json
{ "TITLE": "Kill-Tracker" }
```
```wowlua fails title=src/main.wowlua
import L from "wowlua:locale"
print(L.TITLE)
```

```json compiles title=locales/enUS.json
{ "TITLE": "Kill Tracker" }
```
```json compiles title=locales/deDE.json
{ "TITLE": "Kill-Tracker" }
```
```wowlua compiles title=src/main.wowlua
import L from "wowlua:locale"
print(L.TITLE)
```

Related: E8106

## E8106

The package imports `wowlua:locale`, but the base locale's file is missing. The
base locale (`enUS` by default) defines the keys, and its strings are used
wherever a translation lacks a key. Add `locales/enUS.json`, or set
`build.locale-base` to the locale your strings are written in.

```json fails title=locales/deDE.json
{ "TITLE": "Kill-Tracker" }
```
```wowlua fails title=src/main.wowlua
import L from "wowlua:locale"
print(L)
```

```json compiles title=locales/enUS.json
{ "TITLE": "Kill Tracker" }
```
```json compiles title=locales/deDE.json
{ "TITLE": "Kill-Tracker" }
```
```wowlua compiles title=src/main.wowlua
import L from "wowlua:locale"
print(L.TITLE)
```

Related: E8105

## E8107

A `settings.dropdown` option has a `value` that the bound store field can't
hold. Picking that option would write a value of the wrong type into the
SavedVariable. Fix the option's value, or widen the field's type so that it
includes the value.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["PlaygroundDB"] }
}
```
```wowlua fails title=src/main.wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"

interface Options
    anchor: "TOPLEFT" | "CENTER"
end

local db = saved.bind("PlaygroundDB", { version = 1, defaults = { anchor = "CENTER" } :: Options })

settings.register({
    title = "Playground",
    store = db,
    controls = {
        settings.dropdown("anchor", { label = "Anchor", options = {
            { value = "TOPLEFT", label = "Top left" },
            { value = "BOTTOM", label = "Bottom" },
        } }),
    },
})
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["PlaygroundDB"] }
}
```
```wowlua compiles title=src/main.wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"

interface Options
    anchor: "TOPLEFT" | "CENTER" | "BOTTOM"
end

local db = saved.bind("PlaygroundDB", { version = 1, defaults = { anchor = "CENTER" } :: Options })

settings.register({
    title = "Playground",
    store = db,
    controls = {
        settings.dropdown("anchor", { label = "Anchor", options = {
            { value = "TOPLEFT", label = "Top left" },
            { value = "BOTTOM", label = "Bottom" },
        } }),
    },
})
```

Related: E8070, E8113

## E8108

A file of the addon imports `wowlua:test` or `wowlua:mock`. These modules exist
only in `wowlua test`'s emulated client, not in the game, and test files are
never part of an addon build. Move the code into a test file: a
`*.test.wowlua` file, or a file under `tests/`.

```wowlua fails title=src/main.wowlua
import expect from "wowlua:test"

expect(1 + 1).toBe(2)
```

```wowlua compiles title=tests/math.test.wowlua
import test, expect from "wowlua:test"

@test("adds")
local function adds()
    expect(1 + 1).toBe(2)
end
```

Related: E8119

## E8112

The commands of a slash `router` (and a literal command given to
`Router:add`) are checked at compile time. Each entry of `args` must be
`"string"`, `"number"` or `"boolean"` (with `?` when optional), a list of
choice strings, or `"rest"` as the last one. `run` takes one parameter per
argument, plus one for the flags table when `flags` is declared, and each
typed parameter takes what its argument converts to: a `number` for
`"number"`, a `string` for `"rest"`, the choices' union for a list of choices,
a table of booleans for the flags.

Make `run`'s parameters match `args` and `flags`, or fix the invalid argument
spec.

```wowlua fails
import router from "wowlua:slash"

local commands = router({
    scale = { args = { "number" }, run = function(value: number, extra: string) print(value, extra) end },
})
print(commands)
```

```wowlua compiles
import router from "wowlua:slash"

local commands = router({
    scale = { args = { "number" }, run = function(value: number) print(value) end },
})
print(commands)
```

## E8113

A `wowlua:settings` control names a key that the store has no field for. The
compiler checks each `checkbox`, `slider`, `dropdown` and `color` in the
`controls` list against the type of `store`, which is the type of the defaults
given to `saved.bind`. Usually the key is misspelled. Fix the key, or add the
field to the defaults.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["PlaygroundDB"] }
}
```
```wowlua fails title=src/main.wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"

local db = saved.bind("PlaygroundDB", { version = 1, defaults = { showMinimap = true } })

settings.register({
    title = "Playground",
    store = db,
    controls = { settings.checkbox("showMiniMap", { label = "Minimap button" }) },
})
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["PlaygroundDB"] }
}
```
```wowlua compiles title=src/main.wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"

local db = saved.bind("PlaygroundDB", { version = 1, defaults = { showMinimap = true } })

settings.register({
    title = "Playground",
    store = db,
    controls = { settings.checkbox("showMinimap", { label = "Minimap button" }) },
})
```

Related: E8070, E8107

## E8114

`settings.register` is called more than once. An addon has one options
category in the game's Settings panel. Put the other pages in
`subcategories` of the one registration.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["PlaygroundDB"] }
}
```
```wowlua fails title=src/main.wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"

local db = saved.bind("PlaygroundDB", { version = 1, defaults = { locked = false, debug = false } })

settings.register({ title = "Playground", store = db, controls = {
    settings.checkbox("locked", { label = "Lock frame" }),
} })
settings.register({ title = "Playground: Advanced", store = db, controls = {
    settings.checkbox("debug", { label = "Debug output" }),
} })
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables": ["PlaygroundDB"] }
}
```
```wowlua compiles title=src/main.wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"

local db = saved.bind("PlaygroundDB", { version = 1, defaults = { locked = false, debug = false } })

settings.register({ title = "Playground", store = db, controls = {
    settings.checkbox("locked", { label = "Lock frame" }),
}, subcategories = {
    { title = "Advanced", controls = { settings.checkbox("debug", { label = "Debug output" }) } },
} })
```

Related: E8080

## E8115

A locale file is not a JSON object whose values are all strings. The file may
not be valid JSON, its top level may not be an object, or a value may be a
number, a table or `null`. Every translation is a string, so write numbers in
quotes, and fix the JSON syntax if the file doesn't parse.

```json fails title=locales/enUS.json
{ "TITLE": "Kill Tracker", "MAX_KILLS": 100 }
```
```wowlua fails title=src/main.wowlua
import L from "wowlua:locale"
print(L.TITLE)
```

```json compiles title=locales/enUS.json
{ "TITLE": "Kill Tracker", "MAX_KILLS": "100" }
```
```wowlua compiles title=src/main.wowlua
import L from "wowlua:locale"
print(L.TITLE, L.MAX_KILLS)
```

Related: E8060

## E8117

`profiles = true` was given for a per-character SavedVariable. Profiles let
characters share setups, so they need an account-wide SavedVariable; the
active profile of each character goes into `<name>PerChar`.

Bind an account-wide SavedVariable (`addon.saved-variables`) with profiles, and
add its `<name>PerChar` to `addon.saved-variables-per-character`.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": { "saved-variables-per-character": ["MyAddonCharDB"] }
}
```

```wowlua fails title=src/main.wowlua
import * as saved from "wowlua:saved"
local db = saved.bind("MyAddonCharDB", { version = 1, defaults = {}, profiles = true })
print(db)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "addon": {
        "saved-variables": ["MyAddonDB"],
        "saved-variables-per-character": ["MyAddonDBPerChar"]
    }
}
```

```wowlua compiles title=src/main.wowlua
import * as saved from "wowlua:saved"
local db = saved.bind("MyAddonDB", { version = 1, defaults = {}, profiles = true })
print(db)
```

Related: E8021

## E8118

A `wowlua:log` level function (`trace`, `debug`, `info`, `warn`, `error`) is
used as a value instead of being called, whether imported by name or read
from the `log` object or a namespace import (`local f = log.warn`,
`Log.debug`). The compiler removes log calls below
the profile's `log-level`, with their arguments, so they cost nothing in a
release build. It can do that only for direct calls, never for a function that
was stored in a variable or passed along. Wrap the call in your own function.

```wowlua fails
import warn from "wowlua:log"

local report = warn
report("low on bag space")
```

```wowlua compiles
import warn from "wowlua:log"

local function report(message: string)
    warn(message)
end
report("low on bag space")
```

Related: E3045

## E8119

A file of the addon uses `import internal`. `import internal` gives a file a
module's private locals and types, so that tests can test helpers without
exporting them. Only test files may use it, and addon builds stay unchanged.
In addon code, export what other files need and import it normally.

```wowlua fails title=src/tracker.wowlua
local function parse(text: string): number?
    return tonumber(text)
end
export function record(text: string)
    print(parse(text))
end
```
```wowlua fails title=src/main.wowlua
import internal parse from "/tracker"
print(parse("3"))
```

```wowlua compiles title=src/tracker.wowlua
local function parse(text: string): number?
    return tonumber(text)
end
export function record(text: string)
    print(parse(text))
end
```
```wowlua compiles title=tests/tracker.test.wowlua
import test, expect from "wowlua:test"
import internal parse from "/tracker"

@test("parses numbers")
local function parses()
    expect(parse("3")).toBe(3)
end
```

Related: E8108

## E8120

A case in `@cases` doesn't fit the parameters of the test function. Each case
is a table of the arguments for one run (or a single value for a function
with one parameter). A case is reported when it has more values than the
function takes, when it lacks a value for a parameter that isn't optional, or
when a value's type doesn't fit its parameter.

```wowlua fails
import test, cases, expect from "wowlua:test"

@test("doubles")
@cases({ { 1, 2 }, { 2, "4" } })
local function doubles(n: number, expected: number)
    expect(n * 2).toBe(expected)
end
```

```wowlua compiles
import test, cases, expect from "wowlua:test"

@test("doubles")
@cases({ { 1, 2 }, { 2, 4 } })
local function doubles(n: number, expected: number)
    expect(n * 2).toBe(expected)
end
```

Related: E2001

## E8121

A matcher is used on a kind of value it doesn't take. `toHaveLength` and
`toContain` need a string or a table, `toBeCloseTo` a number, and `toThrow` and
the spy matchers a function. A custom matcher made with `matcher` takes the
type of its first parameter. Such an assertion could never pass, so check that
`expect` gets the value you meant to test.

```wowlua fails
import test, expect from "wowlua:test"

@test("counts")
local function counts()
    local count = 3
    expect(count).toHaveLength(3)
end
```

```wowlua compiles
import test, expect from "wowlua:test"

@test("counts")
local function counts()
    local items = { "a", "b", "c" }
    expect(items).toHaveLength(3)
end
```

Related: E2001

## E8122

A compile-time test marked `@expectError` doesn't fail to compile the way it
says. The checker collects the error codes reported inside the function, plus
any warning codes the decorator names, and compares them with the decorator's
list. When the two match, those diagnostics are dropped and the test passes.
When they don't, E8122 shows what was expected and what was reported. Fix the
function so that it gives exactly the listed diagnostics, or fix the list.

```wowlua fails
import expectError from "wowlua:test"

@expectError("E2001")
local function wrongArgument()
    local n: number = 3
    print(n)
end
```

```wowlua compiles
import expectError from "wowlua:test"

@expectError("E2001")
local function wrongArgument()
    local n: number = "3"
    print(n)
end
```

Related: E8120

## E8123

`mock.module` replaces an export that the module doesn't have. The replacement
table is checked against the module's exports: a name the module doesn't
export is E8123, and a replacement of the wrong type is a type error. Usually
the name is misspelled, or the export was renamed.

```wowlua fails title=src/clock.wowlua
export function now(): number
    return GetTime()
end
```
```wowlua fails title=tests/clock.test.wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"
import * as clock from "/clock"

mock.module(clock, { today = function(): number return 0 end })

@test("reads the clock")
local function reads()
    expect(clock.now()).toBe(0)
end
```

```wowlua compiles title=src/clock.wowlua
export function now(): number
    return GetTime()
end
```
```wowlua compiles title=tests/clock.test.wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"
import * as clock from "/clock"

mock.module(clock, { now = function(): number return 0 end })

@test("reads the clock")
local function reads()
    expect(clock.now()).toBe(0)
end
```

Related: E8119

## E8300

A standard-library function, or a kind it takes, doesn't exist on one of
the flavors you build for, and the module has no fallback there. The
compiler reports it for `aura.container` on flavors without aura containers,
and for tooltip kinds (`tooltip.hook`, `@onTooltip`) a flavor lacks: clients
without the tooltip data system only hook `"Item"`, `"Spell"` and `"Unit"`
tooltips. In game the call would raise an error on that flavor.

Keep the code to the flavors that have it: in an `if flavor == … then`
branch (either flavor mode), or with `@if(flavor == …)` on the declaration
(in split flavor mode). A call is checked only for the flavors that can run
it. Or use something every flavor has, such as `aura.each` instead of a
container.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {}, "vanilla": {} }
}
```
```wowlua fails title=src/main.wowlua
import onTooltip from "wowlua:tooltip"

@onTooltip("Mount")
local function showMountId(tip, data)
    print(data.id)
end
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {}, "vanilla": {} }
}
```
```wowlua compiles title=src/main.wowlua
import onTooltip from "wowlua:tooltip"
import flavor from "wowlua:meta"

@if(flavor == "camelot")
@onTooltip("Mount")
local function showMountId(tip, data)
    print(data.id)
end
```

Related: W8301

## E8310

A literal aura filter passed to `wowlua:aura` (`each`, `forEach`, `find`,
`findByName`, or a container group's `filter`) has a word that isn't an aura
filter, an empty word, or a word or `!` negation one of your flavors doesn't
have. The words are `|`-separated and come from each client's
`AuraUtil.AuraFilters` (`HELPFUL`, `HARMFUL`, `PLAYER`, `RAID`, `CANCELABLE`,
…). Fix the spelling, or drop the word.

```wowlua fails
import * as aura from "wowlua:aura"

for a in aura.each("player", "HELPFUL|MINE") do
    print(a.name)
end
```

```wowlua compiles
import * as aura from "wowlua:aura"

for a in aura.each("player", "HELPFUL|PLAYER") do
    print(a.name)
end
```

Related: E8300

## E8330

The tooltip kind given to `tooltip.hook` or `@onTooltip` is not a name of
`Enum.TooltipDataType` in any of your flavors. Kinds are written like the
enum's names: `"Item"`, `"Spell"`, `"Unit"`, `"Mount"`, `"Currency"`, …
Correct the name; a kind that exists, but not on every flavor you build for,
is E8300 instead.

```wowlua fails
import * as tooltip from "wowlua:tooltip"

tooltip.hook("Items", function(tip, data)
    print(data.id)
end)
```

```wowlua compiles
import * as tooltip from "wowlua:tooltip"

tooltip.hook("Item", function(tip, data)
    print(data.id)
end)
```

Related: E8300

## E8340

`frames.movable` saves the frame's position in `store[key]`, so that field
must have the type `FramePosition?` (from `wowlua:frames`): optional, because
it starts empty until the player first moves the frame. A field of another
type, a required field, or a key the store doesn't have is reported. Declare
the field as `FramePosition?`, usually in the defaults type of your
`saved.bind` store.

```wowlua fails
import * as frames from "wowlua:frames"

local db: { window: number? } = {}
local window = CreateFrame("Frame", nil, UIParent)
frames.movable(window, { store = db, key = "window" })
```

```wowlua compiles
import * as frames from "wowlua:frames"
import type FramePosition from "wowlua:frames"

local db: { window: FramePosition? } = {}
local window = CreateFrame("Frame", nil, UIParent)
frames.movable(window, { store = db, key = "window" })
```

## E8350

`wowlua:bindings` declarations are collected at compile time to generate
`Bindings.xml`, so they must be fixed when the addon is built. `@binding` is
allowed only on a top-level function declaration, with the binding's name as
a constant string and the `header`, `default` and `runOnUp` options as
constants in a table literal (`label` may be any string, such as a locale
entry). `header(id, label)` must be a statement at the module's top level,
with a constant `id`. A constant is a literal or a top-level `const` of the
file.

```wowlua fails
import binding from "wowlua:bindings"

local name = "MYADDON_TOGGLE"

@binding(name, { label = "Toggle window" })
local function toggleWindow()
    print("toggle")
end
```

```wowlua compiles
import binding from "wowlua:bindings"

const NAME = "MYADDON_TOGGLE"

@binding(NAME, { label = "Toggle window" })
local function toggleWindow()
    print("toggle")
end
```

Related: E8351, E8352

## E8351

Two `@binding` declarations in the addon use the same name. The game knows
a binding only by its name, so the second one would replace the first in the
Key Bindings UI. Give each binding its own name; the message points at both.

```wowlua fails
import binding from "wowlua:bindings"

@binding("MYADDON_TOGGLE", { label = "Toggle window" })
local function toggleWindow()
    print("toggle")
end

@binding("MYADDON_TOGGLE", { label = "Toggle map" })
local function toggleMap()
    print("map")
end
```

```wowlua compiles
import binding from "wowlua:bindings"

@binding("MYADDON_TOGGLE", { label = "Toggle window" })
local function toggleWindow()
    print("toggle")
end

@binding("MYADDON_TOGGLE_MAP", { label = "Toggle map" })
local function toggleMap()
    print("map")
end
```

Related: E8352

## E8352

A keybinding name must match `[A-Z][A-Z0-9_]*`: capital letters, digits
and underscores, starting with a letter. The game builds global names from it
(`BINDING_NAME_<name>`). Prefix it with your addon's name to keep it unique,
for example `MYADDON_TOGGLE`.

```wowlua fails
import binding from "wowlua:bindings"

@binding("myaddon-toggle", { label = "Toggle window" })
local function toggleWindow()
    print("toggle")
end
```

```wowlua compiles
import binding from "wowlua:bindings"

@binding("MYADDON_TOGGLE", { label = "Toggle window" })
local function toggleWindow()
    print("toggle")
end
```

Related: E8351

## E8353

`settings.keybinding(name, …)` adds a key binding control to your options
page, but no binding of that name is declared in the addon: neither with
`@binding` from `wowlua:bindings` nor in the package's own `Bindings.xml`.
The control would have nothing to bind. Declare the binding, or fix the name.

```wowlua fails
import * as settings from "wowlua:settings"

local control = settings.keybinding("MYADDON_TOGGLE", { label = "Toggle window" })
print(control)
```

```wowlua compiles
import * as settings from "wowlua:settings"
import binding from "wowlua:bindings"

@binding("MYADDON_TOGGLE", { label = "Toggle window" })
local function toggleWindow()
    print("toggle")
end

local control = settings.keybinding("MYADDON_TOGGLE", { label = "Toggle window" })
print(control)
```

Related: E8352

## E8390

A literal passed to `color.hex` is not a color. Write six hex digits
`rrggbb`, or eight `aarrggbb` with the alpha first, optionally after `#`.
Literals are checked and folded into `CreateColor(r, g, b, a)` at compile
time, so a wrong one is caught before the addon runs.

```wowlua fails
import * as color from "wowlua:color"

local orange = color.hex("ff880")
print(orange:GetRGB())
```

```wowlua compiles
import * as color from "wowlua:color"

local orange = color.hex("ff8800")
print(orange:GetRGB())
```

## E8400

The media kind passed to `media.register`, `media.fetch` or `media.list` is
not one wowlua:media knows. The kinds are those of LibSharedMedia:
`"font"`, `"statusbar"`, `"border"`, `"background"` and `"sound"`. A bar
texture is a `"statusbar"`.

```wowlua fails
import * as media from "wowlua:media"

media.register("texture", "MyAddon Flat", "Interface\\AddOns\\MyAddon\\flat.tga")
```

```wowlua compiles
import * as media from "wowlua:media"

media.register("statusbar", "MyAddon Flat", "Interface\\AddOns\\MyAddon\\flat.tga")
```

## E8420

The event name given to a `wowlua:callbacks` registry (`on`, `once`, `off`,
`offAll`) must be a string literal naming a key of the registry's event type.
That is how the handler is checked against the event's signature. Fix the
spelling, add the event to the interface, or pass the name as a literal
instead of a variable.

```wowlua fails
import * as callbacks from "wowlua:callbacks"

interface TrackerEvents
    KillRecorded: (count: number) -> ()
end

local events: callbacks.Registry<TrackerEvents> = callbacks.registry()
events:on("KillsRecorded", function(count: number)
    print(count)
end)
```

```wowlua compiles
import * as callbacks from "wowlua:callbacks"

interface TrackerEvents
    KillRecorded: (count: number) -> ()
end

local events: callbacks.Registry<TrackerEvents> = callbacks.registry()
events:on("KillRecorded", function(count: number)
    print(count)
end)
```

## E8440

A literal macro conditional handed to `wowlua:secure` (`visibility`,
`state`, `evaluate`) doesn't parse: a `[` without `]`, a bracket in the wrong
place, `@` without a unit, or a condition the client doesn't know (`combat`,
`mod`, `group`, `form`, `bonusbar`, `exists`, `help`, `harm`, `vehicleui`,
`petbattle`, …, each also with `no`). `@unit` and `target=unit` are
accepted; arguments such as `mod:shift` are not checked. Fix the conditional
so that the driver does what you wrote.

```wowlua fails
import * as secure from "wowlua:secure"

local bar = CreateFrame("Frame", nil, UIParent, "SecureHandlerStateTemplate")
secure.visibility(bar, "[incombat] show; hide")
```

```wowlua compiles
import * as secure from "wowlua:secure"

local bar = CreateFrame("Frame", nil, UIParent, "SecureHandlerStateTemplate")
secure.visibility(bar, "[combat] show; hide")
```

Related: E8441

## E8441

A visibility driver (`secure.visibility`) only has two values, `show` and
`hide`; every clause of the conditional must end in one of them. Use
`secure.state` for a driver with values of your own.

```wowlua fails
import * as secure from "wowlua:secure"

local bar = CreateFrame("Frame", nil, UIParent, "SecureHandlerStateTemplate")
secure.visibility(bar, "[combat] visible; hidden")
```

```wowlua compiles
import * as secure from "wowlua:secure"

local bar = CreateFrame("Frame", nil, UIParent, "SecureHandlerStateTemplate")
secure.visibility(bar, "[combat] show; hide")
```

Related: E8440

## W8011

The slash command is one of the game's own commands, such as `/reload`. Your
handler and the game's would compete for it, and players may lose the command
they know.

Pick a name of your own, ideally prefixed with your addon's name.

```wowlua fails
import slash from "wowlua:slash"

@slash("/reload")
local function reload(msg: string) print(msg) end
```

```wowlua compiles
import slash from "wowlua:slash"

@slash("/myreload")
local function reload(msg: string) print(msg) end
```

Related: E8101, E8010

## W8040

The function or member given to `hook`, `@hook` or `hookScript` from
`wowlua:hooks` isn't declared: a global function, a frame, or a method of the
target's type that the game's declarations don't know. Where it doesn't exist,
the hook fails with an error when it is installed. This is usually a typo, or
an API that was removed or that only some clients have.

Fix the name. If the target really exists only sometimes, test for it around
the hook (`if _G.Name then` for a global, `if obj.Method then` for a method);
that test silences the warning.

```wowlua fails
import hook from "wowlua:hooks"
hook("MyMissingFunction", function() print("called") end)
```

```wowlua compiles
import hook from "wowlua:hooks"
if _G.MyMissingFunction then
    hook("MyMissingFunction", function() print("called") end)
end
```

Related: W8102

## W8062

A translation file lacks some keys of the base locale. The addon still works:
players with that client language see the base locale's string for each
missing key. The warning is one summary per locale, with the count and the
first few missing keys. Add the translations, or lower the lint's level in
`check.lints` while a translation is in progress.

```json fails title=locales/enUS.json
{ "TITLE": "Kill Tracker", "SUBTITLE": "Statistics" }
```
```json fails title=locales/deDE.json
{ "TITLE": "Kill-Tracker" }
```
```wowlua fails title=src/main.wowlua
import L from "wowlua:locale"
print(L.TITLE, L.SUBTITLE)
```

```json compiles title=locales/enUS.json
{ "TITLE": "Kill Tracker", "SUBTITLE": "Statistics" }
```
```json compiles title=locales/deDE.json
{ "TITLE": "Kill-Tracker", "SUBTITLE": "Statistik" }
```
```wowlua compiles title=src/main.wowlua
import L from "wowlua:locale"
print(L.TITLE, L.SUBTITLE)
```

Related: E8061

## W8101

In `shared` flavor mode one build runs on every selected flavor. The event
registered here doesn't exist in some of them (the message names which). The
registration is guarded at run time, so nothing breaks, but there the handler
is never called.

If that is what you want, guard the registration with a flavor test so the
intent is clear; otherwise use an event every flavor has, or build the flavors
separately (`split` mode).

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "standard": {}, "vanilla": {} },
    "build": { "flavor-mode": "shared" }
}
```

```wowlua fails title=src/main.wowlua
import on from "wowlua:events"
on("ACTIVE_COMBAT_CONFIG_CHANGED", function() print("changed") end)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "standard": {}, "vanilla": {} },
    "build": { "flavor-mode": "shared" }
}
```

```wowlua compiles title=src/main.wowlua
import on from "wowlua:events"
if WOW_PROJECT_ID == WOW_PROJECT_MAINLINE then
    on("ACTIVE_COMBAT_CONFIG_CHANGED", function() print("changed") end)
end
```

Related: E8003

## W8102

The hook's target belongs to one of Blizzard's load-on-demand addons (such as
`Blizzard_AchievementUI`) and exists only once that addon has loaded. Hooked
without `opts.addon`, the hook runs too early and fails, or finds nothing.

Pass `{ addon = "<AddOn>" }` so the hook waits for the addon to load, and name
the target as a string so it is looked up then. A global function is hooked as
`hook(_G, "Name", fn, { addon = … })`. An `if` test around the hook doesn't
help here: before the addon loads the test fails and the hook is never
installed.

```wowlua fails
import hook from "wowlua:hooks"
hook("AchievementFrame_ToggleAchievementFrame", function() print("toggled") end)
```

```wowlua compiles
import hook from "wowlua:hooks"
hook(_G, "AchievementFrame_ToggleAchievementFrame", function() print("toggled") end, { addon = "Blizzard_AchievementUI" })
```

Related: W8040

## W8301

A standard-library function falls back to a reduced version on one of the
flavors you build for, because that client lacks an API it uses. It still
works there, with less, and the warning lists what is missing:

- `tooltip.hook` and `@onTooltip` for `"Item"`, `"Spell"` and `"Unit"` on
  clients without the tooltip data system (the classic flavors) hook only
  `GameTooltip`, `ItemRefTooltip` and the shopping tooltips (and those given
  to `tooltip.attach`); the data has no `raw`, no `guid` for items and spells
  and no `link` for spells and units.
- `launcher.register` on clients without the Addon Compartment adds no
  compartment entry; the minimap button and the LDB launcher work.
- `frames.movable` with `editMode` on clients without Edit Mode (Cata
  Classic) ignores the option: the frame is an ordinary movable frame.

Keep the call to the flavors that have the full version (an `if flavor == …`
branch or `@if`), turn the feature off where it has an option
(`compartment = false`), or accept the warning. A function a flavor can't
offer at all is E8300.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {}, "vanilla": {} }
}
```
```wowlua fails title=src/main.wowlua
import * as launcher from "wowlua:launcher"

launcher.register({ icon = "Interface\\Icons\\INV_Misc_Book_09", label = "Playground", onClick = function() end })
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {}, "vanilla": {} }
}
```
```wowlua compiles title=src/main.wowlua
import * as launcher from "wowlua:launcher"
import flavor from "wowlua:meta"

launcher.register({
    icon = "Interface\\Icons\\INV_Misc_Book_09",
    label = "Playground",
    onClick = function() end,
    compartment = flavor == "camelot",
})
```

Related: E8300

## W8354

The `default` option of `@binding` sets the key a binding gets by default,
but the targeted clients have no support for default keys in `Bindings.xml`
(no client in the flavor table declares it yet), so the key is ignored. The
binding itself works; players bind a key in the Key Bindings UI. Remove
`default`, or keep it for clients that support it later. The module never
calls `SetBinding` for the player.

```wowlua fails
import binding from "wowlua:bindings"

@binding("MYADDON_TOGGLE", { label = "Toggle window", default = "CTRL-T" })
local function toggleWindow()
    print("toggle")
end
```

```wowlua compiles
import binding from "wowlua:bindings"

@binding("MYADDON_TOGGLE", { label = "Toggle window" })
local function toggleWindow()
    print("toggle")
end
```

## W8430

Inside a function passed to `jobs.start`, the job's context
(`ctx:check`, `ctx:yield`, `ctx:sleep`, `ctx:waitFor`) is called inside a
function literal handed to `pcall`, `xpcall`, `securecall`,
`securecallfunction`, `table.sort` or `gsub`, or inside a metamethod
(`setmetatable(t, { __index = function … end })`, `mt.__lt = function … end`).
Jobs are coroutines, and Lua 5.1 cannot yield across these calls: at run time it raises "attempt to yield
across metamethod/C-call boundary". Yield before or after the call instead,
or split the work so the callback doesn't need to yield.

```wowlua fails
import * as jobs from "wowlua:jobs"

local items = { 3, 1, 2 }
jobs.start(function(ctx)
    table.sort(items, function(a, b)
        ctx:check()
        return a < b
    end)
end)
```

```wowlua compiles
import * as jobs from "wowlua:jobs"

local items = { 3, 1, 2 }
jobs.start(function(ctx)
    ctx:check()
    table.sort(items, function(a, b)
        return a < b
    end)
end)
```
