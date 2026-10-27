# Settings, minimap button and keybindings

`wowlua:settings` builds an options page under Options › AddOns,
`wowlua:launcher` a minimap button, an Addon Compartment entry and a data
broker launcher, and `wowlua:bindings` entries in the Key Bindings menu. Each
is a declaration in your code, bound to your saved data, with no library to
embed and no `Bindings.xml` to write.

## An options page in a few lines

The page edits a table you bound with [`saved.bind`](saved.md). The examples
on this page share this one:

```json title=wowlua.json
{
  "$schema": "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/wowlua-0.1.json",
  "package": {
    "name": "my-addon",
    "version": "0.1.0",
    "wowlua": "^0.13"
  },
  "addon": {
    "title": "MyAddon",
    "saved-variables": ["MyAddonDB"]
  }
}
```

```wowlua title=src/db.wowlua
import * as saved from "wowlua:saved"

export interface Options
    greet: boolean
    scale: number
    anchor: "TOPLEFT" | "CENTER"
    barColor: { r: number, g: number, b: number, a?: number }
    debug: boolean
    minimap: { hide?: boolean, minimapPos?: number }
end

export local db = saved.bind("MyAddonDB", {
    version = 1,
    defaults = {
        greet = true,
        scale = 1,
        anchor = "CENTER",
        barColor = { r = 0.2, g = 0.6, b = 1 },
        debug = false,
        minimap = {},
    } :: Options,
})
```

Pass it to `settings.register` as `store`, and list the controls. Each control
names the field it edits.

```wowlua title=src/options.wowlua
import * as settings from "wowlua:settings"
import db from "./db"

settings.register({
    title = "MyAddon",
    store = db,
    controls = {
        settings.header("General"),
        settings.checkbox("greet", { label = "Greet at login" }),
        settings.slider("scale", { label = "Window scale", min = 0.5, max = 2, step = 0.1 }),
    },
})
```

This gives a page called MyAddon with a General header, a checkbox and a
slider.

- The control keys are checked against the type of `store`. An unknown key is
  error E8113, and a control on a field of the wrong type, such as a checkbox
  on a `number`, is E8070.
- The defaults you gave `saved.bind` are the page's defaults. Nothing is
  repeated.
- When the player switches
  [profiles](saved.md#let-players-keep-several-profiles), every control reads
  its value again.
- `register` can be called at the top of a file. The page is created once your
  SavedVariables have loaded, after the `@onLoad` hooks.

## More kinds of controls

Besides headers, checkboxes and sliders there are text lines, dropdowns, color
swatches, keybinding rows, buttons and custom areas. `subcategories` adds pages
below the main one, and `onChange` is called with the key and the new value
after every change the player makes.

```wowlua title=src/options.wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"
import print from "wowlua:log"
import db from "./db"

settings.register({
    title = "MyAddon",
    store = db,
    controls = {
        settings.text("Changes apply right away."),
        settings.slider("scale", {
            label = "Window scale",
            min = 0.5, max = 2, step = 0.05,
            format = function(v: number) return ("%.0f%%"):format(v * 100) end,
        }),
        settings.dropdown("anchor", { label = "Anchor", options = {
            { value = "TOPLEFT", label = "Top left" },
            { value = "CENTER", label = "Center" },
        } }),
        settings.color("barColor", { label = "Bar color", alpha = true }),
        settings.keybinding("MYADDON_TOGGLE", { label = "Toggle window" }),
        settings.button("Reset", function() saved.handle(db):reset() end),
    },
    subcategories = {
        { title = "Advanced", controls = {
            settings.checkbox("debug", { label = "Debug output", tooltip = "Prints more to the chat" }),
        } },
    },
    onChange = function(key: string, value: any)
        print(key, "is now", tostring(value))
    end,
})
```

| Control | Edits |
|---------|-------|
| `header(text)` | nothing: a section header |
| `text(content)` | nothing: a line of text |
| `checkbox(key, { label, tooltip? })` | a `boolean` field |
| `slider(key, { label, tooltip?, min, max, step, format? })` | a `number` field; `format` turns the value into the label next to it |
| `dropdown(key, { label, tooltip?, options })` | a field whose type holds every option's `value` (E8107); the options are shown in their order |
| `color(key, { label, tooltip?, alpha? })` | a `{ r, g, b, a? }` field |
| `keybinding(name, { label })` | a binding declared with [`@binding`](#keybindings) or in your own `Bindings.xml` (E8353) |
| `button(text, onClick)` | nothing: runs `onClick` |
| `custom(factory, { height? })` | nothing: `factory(parent)` builds your own UI once, in an area `height` high (100 by default) |

`register` can be called once per addon (E8114). More pages go in
`subcategories`. It returns the page's handle, with `open()`, `refresh()`,
which reads every control again, and `id`, the category's id once the page
exists.

## Opening the page

The module adds no slash command, so you decide how players reach the page. A
command of a [slash router](slash.md) and a right-click on the minimap button
are the usual ways. `settings.open()` opens it.

```wowlua title=src/commands.wowlua
import slash, router from "wowlua:slash"
import * as settings from "wowlua:settings"

local commands = router({
    options = { help = "Open the options", run = function() settings.open() end },
})

@slash("/mya")
local function command(msg: string)
    commands:dispatch(msg)
end
```

> [!NOTE]
> The game's Settings panel doesn't open reliably in combat. `settings.open()`
> during combat prints "The options open once combat ends." and opens the page
> when combat ends.

## A minimap button

`launcher.register` gives your addon a minimap button, an Addon Compartment
entry and a data broker launcher, from one icon, label, click handler and
tooltip.

```wowlua title=src/launcher.wowlua
import * as launcher from "wowlua:launcher"
import * as settings from "wowlua:settings"
import addon from "wowlua:meta"
import db from "./db"

local window = CreateFrame("Frame", nil, UIParent)

launcher.register({
    icon = "Interface\\Icons\\INV_Misc_QuestionMark",
    label = addon.title,
    onClick = function(button: string)
        if button == "RightButton" then
            settings.open()
        else
            window:SetShown(not window:IsShown())
        end
    end,
    tooltip = function(tip: GameTooltip)
        tip:AddLine(addon.title)
        tip:AddLine("Left-click: window. Right-click: options.", 1, 1, 1)
    end,
    minimap = { store = db, key = "minimap" },
    compartment = true,
    ldb = true,
})
```

- The minimap button needs `minimap`: the table and key where its position is
  stored. When LibDBIcon is loaded, the button registers with it, so minimap
  button managers can collect it. Otherwise wowlua draws its own draggable
  button, named `MyAddonMinimapButton`. Never both.
- The position is stored in LibDBIcon's shape (`hide`, `minimapPos`), so it
  survives a switch between the two.
- The Addon Compartment entry, the list behind the addons button by the
  minimap, appears on clients that have one unless you pass
  `compartment = false`. When you also build for a flavor without it, the
  build warns (W8301); `compartment = flavor == "camelot"` turns it on only
  where it exists.
- With `ldb = true`, the launcher is published to LibDataBroker when another
  addon has loaded it, so broker displays such as Titan Panel show it. wowlua
  depends on none of these libraries.
- Everything is created at login. `register` can be called once per addon
  (E8080).
- `launcher.setIcon`, `setLabel`, `showMinimap(show)` and `refresh()` change
  it later. `refresh()` reads the stored position again, for example after you
  change it in code. A profile switch of a `minimap.store` from
  `saved.bind` refreshes the button by itself.
- The built-in button looks like the game's own minimap buttons. A `theme` in
  `register`, or `launcher.setTheme(theme)`, restyles it with a widget theme
  (see [Frames, lists and menus](ui.md)).

## Live text in broker displays

A data source is text that broker displays show and keep up to date, such as
your gold or a counter. It is independent of the launcher.

```wowlua title=src/gold.wowlua
import * as launcher from "wowlua:launcher"
import * as format from "wowlua:format"
import event from "wowlua:events"

local gold = launcher.dataSource("MyAddon Gold", {
    text = format.money(GetMoney()),
    icon = "Interface\\Icons\\INV_Misc_Coin_01",
})

@event("PLAYER_MONEY")
local function moneyChanged()
    gold:setText(format.money(GetMoney()))
end
```

LibDataBroker often comes embedded in another addon that loads later. The data
source waits for it and is published once it is there. `setText`, `setIcon`
and `setLabel` change the data source, and its `object` field is the
LibDataBroker object, or nil until then. Data source names are shared by all
addons: a name that is already taken is reported as an error.

## Keybindings

Put `@binding` on a function and it appears in the game's Key Bindings menu.
`header` adds a heading to group your bindings under. wowlua writes the
`Bindings.xml` file and the `BINDING_*` globals the game needs.

```wowlua title=src/bindings.wowlua
import binding, header from "wowlua:bindings"
import type KeyState from "wowlua:bindings"

local window = CreateFrame("Frame", nil, UIParent)

header("MYADDON", "MyAddon")

@binding("MYADDON_TOGGLE", { label = "Toggle window", header = "MYADDON" })
local function toggle()
    window:SetShown(not window:IsShown())
end

@binding("MYADDON_PEEK", { label = "Show while held", header = "MYADDON", runOnUp = true })
local function peek(state: KeyState)
    window:SetShown(state == "down")
end
```

The build generates this `Bindings.xml`:

```xml
<Bindings>
  <Binding name="MYADDON_TOGGLE" header="MYADDON">MyAddon_Bindings["MYADDON_TOGGLE"](keystate)</Binding>
  <Binding name="MYADDON_PEEK" runOnUp="true">MyAddon_Bindings["MYADDON_PEEK"](keystate)</Binding>
</Bindings>
```

`MyAddon_Bindings` is the one global table that forwards the bindings to your
functions. The header is on the first binding of its group, so the menu shows
it once.

- A binding name is upper case letters, digits and underscores, starting with
  a letter (E8352), and unique in the addon (E8351).
- `@binding` goes on top-level functions and `header` is a top-level
  statement. Their arguments must be constants, except `label`, which can be
  any string, a [locale entry](locale.md) such as `L.TOGGLE_WINDOW` included
  (E8350).
- With `runOnUp = true` the function runs when the key goes down and when it
  goes up, and gets `"down"` or `"up"`.
- A package that has its own `Bindings.xml` can't also use `@binding` (E4334).
- Bindings fire in combat too. If the function touches protected frames, add
  [`@outOfCombat`](combat.md).

## At a glance

| Module | Call | Does |
|--------|------|------|
| `wowlua:settings` | `register(page)` | Builds the options page |
| | `open()` | Opens it, after combat if need be |
| | `header`, `text`, `checkbox`, `slider`, `dropdown`, `color`, `keybinding`, `button`, `custom` | The controls |
| `wowlua:launcher` | `register(options)` | Minimap button, Addon Compartment entry, data broker launcher |
| | `setIcon`, `setLabel`, `showMinimap`, `refresh` | Change them later |
| | `setTheme(theme)` | Restyles the built-in button |
| | `dataSource(name, options)` | Live text for broker displays |
| `wowlua:bindings` | `header(id, label)` | A heading in the Key Bindings menu |
| | `@binding(name, options)` | A bindable action |

> [!TIP]
> In tests, `mock.pressBinding("MYADDON_TOGGLE")` presses a binding as the
> game would, through the generated `Bindings.xml`. See
> [Testing your addon](../guide/testing.md).

The full rules are in the specification:
[settings](../../spec/stdlib/ui.md#settings),
[launcher](../../spec/stdlib/ui.md#launcher) and
[bindings](../../spec/stdlib/interface.md#bindings).
