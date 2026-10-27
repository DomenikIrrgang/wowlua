# Frames, lists and menus

The modules for building an addon window: frames the player can drag and
resize, pools and scrolling lists, context menus, dialogs, widget themes,
colors and shared media. They work on the game's own widgets and fit well
with [frames as classes](../language/widgets.md).

## Let players move and resize a window

`frames.movable` makes a frame draggable and keeps its position, size and
scale in a table you give it, usually your saved variables. The examples on
this page save into `MyAddonDB`, declared in the manifest:

```json title=wowlua.json
{
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

```wowlua title=src/window.wowlua
import * as frames from "wowlua:frames"
import * as saved from "wowlua:saved"
import type FramePosition from "wowlua:frames"

interface Data
    window?: FramePosition
    bar?: FramePosition
    locked: boolean
    barTexture?: string
end

export local db = saved.bind("MyAddonDB", {
    version = 1,
    defaults = { locked = false } :: Data,
})

export local window = CreateFrame("Frame", nil, UIParent, "BackdropTemplate")
window:SetSize(300, 200)
window:SetPoint("CENTER")

const handle = frames.movable(window, {
    store = db, key = "window",
    clamp = true,                            -- keep it on screen
    modifier = "SHIFT",                      -- drag only while Shift is held
    resize = { min = { 200, 120 }, max = { 900, 700 } },
    scaleWheel = "CTRL",                     -- Ctrl + mouse wheel scales it
})

export function setLocked(locked: boolean)
    db.locked = locked
    handle:lock(locked)                      -- no dragging, resizing or scaling
end

export function resetWindow()
    handle:reset()                           -- the anchor, size and scale it had
end
```

`store[key]` must be typed `FramePosition?`; anything else is a build error
(E8340).

- Call `movable` at the top of a file. The saved position is applied once
  the saved variables have loaded, and again when the player switches
  profiles.
- The position is anchored to the third of the screen the frame's center is
  in. A window parked near the right edge stays near it when the resolution
  or UI scale changes.
- `resize` adds a grip in the bottom-right corner, drawn with the chat
  window's resize textures. Pass `resize.grip` to use a handle of your own.
- `scaleWheel` changes the scale by 0.05 per notch, between 0.5 and 2 unless
  you set `scaleRange`.
- A protected frame can't be dragged in combat, and `restore` and `reset`
  wait until combat ends.
- `onChange` is called after every save, and with `nil` after a reset.

### Join Edit Mode

Players arrange the whole interface in Edit Mode. With the `editMode` option
your frame takes part: it gets the same highlight and name label as
Blizzard's frames, and keeps a position for each Edit Mode layout.

```wowlua title=src/bar.wowlua
import * as frames from "wowlua:frames"
import db from "./window"

local bar = CreateFrame("Frame", nil, UIParent)
bar:SetSize(240, 40)
bar:SetPoint("BOTTOM", 0, 120)

frames.movable(bar, { store = db, key = "bar", editMode = { label = "MyAddon Bar" } })
```

A drag in Edit Mode is saved when it ends, not when the player presses Edit
Mode's Save button. Outside Edit Mode the frame is locked, so the player
arranges everything in one place. Add `lock = false` to `editMode` to keep it
movable there as well. On a client without Edit Mode (Cata Classic) the option
is ignored, and the compiler warns with W8301 when you build for it.

## Reuse frames with pools

The game never frees a frame. A list that creates new rows every time it
refreshes keeps growing. A pool hands out objects, takes them back and hands
the same ones out again. `FramePool.new` takes a widget class and the
arguments for its constructor, and checks them against the constructor's
parameters.

```wowlua title=src/rows.wowlua
import FramePool from "wowlua:pool"
import window from "./window"

export class RowButton extends Button
    constructor(parent: Frame)
        super(nil, parent)
        self:SetSize(260, 20)
        self:SetNormalFontObject("GameFontHighlight")
    end

    function onRelease()                     -- called when the pool takes it back
        self:SetText("")
    end
end

const rows = FramePool.new(RowButton, window)

export function showNames(names: string[])
    rows:releaseAll()
    for i, name in ipairs(names) do
        local row = rows:acquire()           -- a RowButton, new or reused
        row:SetPoint("TOP", 0, -20 * i)
        row:SetText(name)
        row:Show()
    end
end
```

Releasing hides the frame, clears its points and calls its `onRelease`
method if it has one. A protected frame released in combat stays acquired
until combat ends.

- `acquire` also returns `true` when the object was newly made.
- `active()` iterates the acquired objects, and `count()` returns how many are
  acquired and how many wait for reuse.
- `FramePool.fromTemplate("Button", parent, "UIPanelButtonTemplate")` pools
  plain frames made from an XML template.
- `TablePool.new()` pools scratch tables and wipes each one on `release`.
  `TablePool.new({ max = 50 })` keeps at most 50 released tables.
- In debug builds (the `dev` and `test` profiles), releasing an object twice,
  or one from another pool, raises an error.

## Scrolling lists

`list.create` builds a list on the game's ScrollBox that only has rows for
the entries on screen, and reuses them as the player scrolls. You give it the
row's widget class and a function that shows one entry in a row. The list
fills its parent and has a scroll bar on the right.

```wowlua title=src/loot.wowlua
import * as list from "wowlua:list"
import RowButton from "./rows"
import window from "./window"

interface Drop
    name: string
    count: number
end

local drops: Drop[] = {
    { name = "Linen Cloth", count = 12 },
    { name = "Copper Ore", count = 4 },
}

const view = list.create(window, {
    rowHeight = 22,
    row = RowButton,
    init = function(row: RowButton, drop: Drop)
        row:SetText(`{drop.name} x{drop.count}`)
    end,
})

view:setData(drops)
view:onSelect(function(drop: Drop)
    print("selected", drop.name)
end)
view:scrollTo(2)
```

The entry type comes from `init`, so `setData` and `onSelect` are checked
against it. Each row is an instance of the class, constructed with the
list's frame as its only argument. A left click selects an entry;
`view:selected()` returns the last one selected. After changing entries in
place, call `view:refresh()` to show them again. `reset` in the options runs
when a row is released, and `spacing` puts space between rows.

## Right-click menus

`menu.attach` opens a context menu when the player right-clicks a frame. The
function that builds the menu runs every time it opens, so checkboxes and
radio buttons always show the current state.

```wowlua title=src/notes.wowlua
import * as menu from "wowlua:menu"
import type MenuBuilder from "wowlua:menu"

interface Note
    title: string
    pinned: boolean
    folder: string
end

const folders = { "Inbox", "Raids", "Archive" }

export function addMenu(row: Frame, note: Note)
    row:EnableMouse(true)
    menu.attach(row, function(m: MenuBuilder)
        m:title(note.title)
        m:button("Print", function() print(note.title) end)
        m:checkbox("Pinned",
            function() return note.pinned end,
            function() note.pinned = not note.pinned end)
        m:divider()
        m:submenu("Move to", function(sub: MenuBuilder)
            for _, folder in ipairs(folders) do
                sub:radio(folder,
                    function() return note.folder == folder end,
                    function() note.folder = folder end)
            end
        end)
    end)
end
```

The builder also has `spacer()` and `disabled(text)`, and every call returns
the builder, so calls can be chained. `attach` takes a third argument,
`{ button = "LeftButton" }`, for another mouse button, and
`menu.open(owner, build)` opens a menu at the cursor straight away.

The menus are the game's own, built with `MenuUtil`. They close on Escape
or a click elsewhere, and they don't spread taint as `UIDropDownMenu` does.

## Confirm, prompt, import and export

Four dialogs cover the common cases: a yes-or-no question, a line of text, and
a text box to copy a string out of or paste one into.

```wowlua title=src/dialogs.wowlua
import * as dialog from "wowlua:dialog"
import * as saved from "wowlua:saved"
import * as serialize from "wowlua:serialize"
import db from "./window"

export function askReset()
    dialog.confirm({
        text = "Reset all settings?",
        onAccept = function() saved.handle(db):reset() end,
    })
end

export function askName(apply: (name: string) -> ())
    dialog.prompt({
        text = "Name of the new layout",
        validate = function(v: string) return #v > 0 and #v <= 32 end,
        onAccept = apply,
    })
end

export function showExport()
    dialog.export({ title = "Export settings", value = serialize.exportString(db, { version = 1 }) })
end

export function showImport(apply: (value: unknown) -> ())
    dialog.import({
        title = "Import settings",
        onImport = function(text: string)
            local ok, value = serialize.importString(text)
            if not ok then
                return "That isn't a valid settings string."   -- shown; the dialog stays open
            end
            apply(value)
            return nil
        end,
    })
end
```

- `confirm` takes `accept` and `cancel` for the button labels, and `onCancel`.
  `prompt` takes a `default` text.
- The prompt's Accept button and Enter only work while `validate` returns
  true.
- A string returned from `onImport` is shown as an error, and the dialog stays
  open.
- The export box is read-only and its text is selected, ready for Ctrl+C.
- One dialog shows at a time. Opening another cancels the current one, and
  `dialog.close()` closes it.
- Escape closes them, in combat too. Each dialog frame has a global name
  (`MyAddonDialog1`, …) in `UISpecialFrames`, the game's list of frames
  Escape closes.
- They use frames of their own, not `StaticPopupDialogs`, so they can't taint
  Blizzard's popups.

## Match your addon's look

The dialogs and the built-in minimap button of
[`wowlua:launcher`](settings.md) look like the game's own UI and carry no
wowlua branding. If your addon has a style of its own, describe it once as a
`WidgetTheme`. Every field is optional, and `setTheme(nil)` returns to the
default look.

```wowlua title=src/theme.wowlua
import type WidgetTheme from "wowlua:frames"
import * as dialog from "wowlua:dialog"
import * as launcher from "wowlua:launcher"

export const THEME: WidgetTheme = {
    font = { title = "Fonts\\FRIZQT__.TTF", size = 12 },
    colors = {
        background = { 0.08, 0.07, 0.06, 0.94 },
        border = { 0.43, 0.40, 0.33 },
        accent = { 1, 0.71, 0.28 },
    },
    borderSize = 1,
    button = "flat",                         -- a fill, a border and a label
}

dialog.setTheme(THEME)
launcher.setTheme(THEME)
```

| Field | What it sets |
|-------|--------------|
| `font.title`, `font.body` | A font file or a font object name |
| `font.size` | The font size |
| `colors.background`, `border`, `title`, `text`, `muted`, `accent`, `onAccent`, `error` | Colors as `{ r, g, b, a? }`, 0 to 1 |
| `borderSize`, `padding` | The border width and the inner spacing |
| `button` | `"template"` (the game's button) or `"flat"` |

A theme applies to frames created afterwards; frames already made take it
the next time they are shown. `wowlua:frames` also exports the builders the
dialogs use, so your own frames can match:

```wowlua title=src/panel.wowlua
import * as frames from "wowlua:frames"
import THEME from "./theme"

local panel = CreateFrame("Frame", nil, UIParent, "BackdropTemplate")
frames.applyBackdrop(panel, THEME)

local label = panel:CreateFontString(nil, "OVERLAY")
frames.styleText(label, "title", THEME)      -- "title", "body", "muted" or "error"

local ok = frames.button(panel, "OK", THEME)
ok:SetPoint("BOTTOM", 0, 10)
```

Menus are not themed: they are always the game's.

## Colors

`wowlua:color` has class and quality colors, hex colors checked at compile
time, and the `|cff…|r` escapes for colored text.

```wowlua title=src/colors.wowlua
import * as color from "wowlua:color"
import type Color from "wowlua:color"

local label = UIParent:CreateFontString(nil, "OVERLAY", "GameFontNormal")

local orange = color.hex("ff8800")           -- a typo such as "ff88g0" is a build error
label:SetTextColor(orange:GetRGB())
print(color.wrap("Warning", orange))         -- "|cffff8800Warning|r"

local mage = color.class("MAGE")
if mage then
    print(color.wrap(UnitName("player"), mage))
end

export function healthColor(pct: number): Color
    return color.lerp(color.hex("ff0000"), color.hex("00ff00"), pct)
end
```

- `hex` takes `"rrggbb"` or `"aarrggbb"`, with or without `#`. A literal is
  checked (E8390) and compiled to a `CreateColor` call.
- `rgb(r, g, b, a?)` makes a color from components between 0 and 1, and
  `toHex(c)` goes the other way.
- `class` uses the colors of the ClassColors addon when the player has it.
  `quality` returns an item quality's color.
- `wrap` works with secret text too. `colorize("ff8800", text)` wraps with a
  hex string.
- `strip` removes color escapes from a string, for measuring or comparing
  text.

## Fonts, textures and sounds

`wowlua:media` registers fonts, bar textures, borders, backgrounds and sounds
by name and finds them by name. When any addon has loaded LibSharedMedia,
your media goes there too, so players can pick from every addon's media in
one list. Without it, a registry of your own addon does the same job.

```wowlua title=src/media.wowlua
import * as media from "wowlua:media"
import type MediaType from "wowlua:media"
import db from "./window"

media.register("font", "MyAddon Bold", "Interface\\AddOns\\MyAddon\\media\\Bold.ttf")
media.register("statusbar", "MyAddon Flat", "Interface\\AddOns\\MyAddon\\media\\Flat.tga")

local bar = CreateFrame("StatusBar", nil, UIParent)
bar:SetStatusBarTexture(media.fetch("statusbar", db.barTexture) ?? "")

media.onRegistered(function(kind: MediaType, name: string)
    print("new", kind, name)
end)
```

- For files in your package, `asset("media/Bold.ttf")` from
  [`wowlua:meta`](../language/flavors.md) gives the in-game path and checks at
  build time that the file is one of the package's assets.
- `fetch` returns the kind's default for an unknown name: the game's standard
  font, status bar, tooltip border or tooltip background (sounds have none).
  Pass `{ noDefault = true }` to get `nil` instead.
- `list(kind)` returns the registered names, sorted, for a dropdown.
- `onRegistered` fires for your registrations, and with LibSharedMedia for
  every addon's.
- The kinds are `"font"`, `"statusbar"`, `"border"`, `"background"` and
  `"sound"`. Another kind is a build error (E8400).

LibSharedMedia often comes embedded in an addon that loads after yours. Your
registrations wait for it and are handed over when it turns up.

## Reaching into the game's frames

Addons that restyle Blizzard's windows read frames by name or by key, and
those parts may not exist yet, or may differ between clients. These helpers
check what they find instead of trusting a type.

```wowlua title=src/skin.wowlua
import * as frames from "wowlua:frames"
import * as tbl from "wowlua:table"

local cf = frames.find("CharacterFrame")                     -- Frame?: nil until it exists
local close = frames.asButton(tbl.field(cf, "CloseButton"))  -- Button?, if it really is one
if close then
    close:SetAlpha(0.5)
end

if cf then
    for _, tex in ipairs(frames.textures(cf)) do
        tex:SetAlpha(0)
    end
end

local zoom = frames.find("MinimapZoomIn")
if zoom then
    frames.keepAway(zoom, { fade = true })                   -- out of sight, still shown
end
```

| Function | Returns |
|----------|---------|
| `find(name)` | The global frame of that name, or nil |
| `children(frame)` | The frame's children the addon may read |
| `textures(frame)`, `fontStrings(frame)` | The frame's own textures or font strings |
| `keyed(frame)` | The widgets the frame keeps in its fields, by key (`ScrollBox`, …) |
| `stateTextures(button)` | A button's normal, pushed, highlight, disabled and checked textures |
| `rows(list)` | The frames one of the game's pools or scroll boxes shows |
| `mapPins(map, template)` | The pins of a template on one of the game's maps |
| `asFrame(v)`, `asButton(v)`, `asTexture(v)`, … | `v` if it is that kind of widget, else nil |
| `keepAway(frame, opts?)` | Keeps a frame off screen while it stays shown |

`keepAway` moves the frame off screen and, with `fade`, makes it
transparent, and does so again whenever the game moves or shows it. Its
scripts and children keep working. `when` limits it to times a function
returns true, and the handle's `release()` stops it.

## At a glance

| Module | Functions |
|--------|-----------|
| `wowlua:frames` | `movable` (drag, resize, scale, Edit Mode); `applyBackdrop`, `styleText`, `button` (themed parts); `find`, `children`, `textures`, `keyed`, `asButton`, … (the game's frames); `keepAway` |
| `wowlua:pool` | `FramePool.new`, `FramePool.fromTemplate`, `TablePool.new`; `acquire`, `release`, `releaseAll`, `active`, `count` |
| `wowlua:list` | `create`, then `setData`, `refresh`, `onSelect`, `selected`, `scrollTo` |
| `wowlua:menu` | `attach`, `open` |
| `wowlua:dialog` | `confirm`, `prompt`, `export`, `import`, `close`, `setTheme` |
| `wowlua:color` | `hex`, `rgb`, `toHex`, `class`, `quality`, `wrap`, `colorize`, `strip`, `lerp` |
| `wowlua:media` | `register`, `fetch`, `list`, `onRegistered` |

> [!TIP]
> All of this can be tested without the game: `mock.drag(window, 40, -20)`
> drags a frame, `mock.click(row, "RightButton")` clicks one, and
> `expect(window).toMatchScreenshot()` compares what the player would see.
> See [testing your addon](../guide/testing.md).
