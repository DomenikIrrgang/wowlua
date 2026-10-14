# 8.5 Standard Library: Interface

User-interface building blocks. Like the game-data modules, each selects the
best client API per flavor at compile time ([Game Data](game.md) intro), with
E8300 for unavailable functions and W8301 for reduced fallbacks. Every handler
the modules call runs through the stdlib error policy
([Infrastructure: errors](infrastructure.md#errors)).

## frames

`wowlua:frames` makes frames movable and resizable, and saves their position,
size and scale.

```wowlua
import * as frames from "wowlua:frames"

local handle = frames.movable(window, {
    store = db, key = "window",               -- db.window: FramePosition?
    clamp = true,
    modifier = "SHIFT",                       -- drag only while Shift is held (optional)
    resize = { min = { 200, 120 }, max = { 900, 700 } },
    scaleWheel = "CTRL",                      -- Ctrl + mouse wheel changes scale (optional)
})

handle:lock(true)
handle:reset()                                -- back to the frame's original anchor and size
```

| API | Signature | Notes |
|-----|-----------|-------|
| `movable` | `(frame: Frame, opts: MovableOptions) -> MovableHandle` | |
| `MovableOptions` | `store`, `key`, `clamp?`, `button?` (default `"LeftButton"`), `modifier?` (`"SHIFT" \| "CTRL" \| "ALT"`), `resize?` (`{ min: number[]?, max: number[]?, grip: Frame? }`, sizes as `{ width, height }`), `scaleWheel?` (a modifier), `scaleRange?` (default `{ 0.5, 2 }`), `onChange?` (`(pos: FramePosition?) -> ()`, after a save, or `nil` after a reset) | |
| `MovableHandle` | `lock(locked: boolean)`, `isLocked()`, `reset()`, `restore()`, `save()` | A locked frame cannot be moved, resized or scaled with the mouse; the built-in grip is hidden (decision S24). |
| `FramePosition` | `{ point: AnchorPoint, relativePoint: AnchorPoint, x: number, y: number, width: number?, height: number?, scale: number? }` | Positions are always stored relative to `UIParent`, never to named frames that may not exist on the next load |

Semantics:

- `store[key]` must have type `FramePosition?` (E8340). The store is a
  `wowlua:saved` binding (a plain table works too). Positions are restored
  once the store is loaded (decision S3), so calling `movable` at module top
  level is fine, and again when the store's profile changes.
- **Anchoring** (decision S23): a saved position is anchored at the screen
  third the frame's center is in, horizontally and vertically (`TOPLEFT`,
  `TOP`, …, `CENTER`), with offsets in UIParent units from that edge. A
  frame near an edge stays there when the screen size or UI scale changes,
  and resizing grows away from that edge.
- Only a drag or resize the mouse actually started saves (not a press
  without the modifier, or on a locked frame).
- `scaleWheel`: each wheel notch changes the scale by 0.05 (decision S25),
  within `scaleRange`, keeping the frame's anchor where it is. The frame then
  takes the mouse wheel.
- The module calls `SetMovable`, `RegisterForDrag`,
  `StartMoving`/`StopMovingOrSizing`, and `SetUserPlaced(false)`, so the game's
  own layout cache does not fight with the saved position.
- **Combat safety:** for protected frames, dragging is disabled while
  `InCombatLockdown()` is true, and `restore`/`reset` are deferred through
  `wowlua:combat` until combat ends.
- A built-in resize grip (bottom-right) is created when `resize` is given
  without `grip`, with the client's chat-frame size grabber textures
  (decision S17).

## bindings

`wowlua:bindings` declares keybindings in code. The compiler generates the
`Bindings.xml` file, which the game loads automatically from the AddOn
root, and the required `BINDING_HEADER_*` / `BINDING_NAME_*` globals.

```wowlua
import binding, header from "wowlua:bindings"
import type KeyState from "wowlua:bindings"
import L from "wowlua:locale"

header("MYADDON", L.BINDING_HEADER)           -- heading shown in the Key Bindings UI

@binding("MYADDON_TOGGLE", { label = L.TOGGLE_WINDOW, header = "MYADDON" })
local function toggleWindow()
    window:SetShown(not window:IsShown())
end

@binding("MYADDON_PUSH_TO_TALK", { label = "Push to talk", runOnUp = true })
local function pushToTalk(keystate: KeyState) … end
```

| API | Signature | Notes |
|-----|-----------|-------|
| `header` | `(id: string, label: string)` | Statement at module top level. `id` must be a constant. |
| `@binding(name, opts)` | function decorator | `name` is a constant matching `[A-Z][A-Z0-9_]*` (E8352), unique within the addon (E8351) |
| `opts` | `{ label: string, header: string?, runOnUp: boolean?, default: string? }` | All fields except `label` must be constants. `label` may be any string expression, e.g. a locale entry. |
| Handler | `() -> ()` or `(keystate: KeyState) -> ()` (with `runOnUp`) | `KeyState` is `"down" \| "up"`. Runs through the stdlib error policy: a failing handler is reported, not raised into the client. |

A constant is a literal or a value folded from the file's top-level `const`
declarations (`const HEADER = "MYADDON"`).

**Compile-time collection.** `@binding` is allowed only on top-level function
declarations, and `header` only as a top-level statement, both with constant
arguments as described (E8350). The compiler collects all of them across the
addon, per flavor, and generates one `Bindings.xml` with every flavor's
bindings. Bindings without a header come first, then each header's bindings
together, in order of first use; the `header` attribute is on the first
binding of its group, so the Key Bindings UI shows each header once:

```xml
<!-- Bindings.xml (generated) -->
<Bindings>
  <Binding name="MYADDON_PUSH_TO_TALK" runOnUp="true">MyAddon_Bindings["MYADDON_PUSH_TO_TALK"](keystate)</Binding>
  <Binding name="MYADDON_TOGGLE" header="MYADDON">MyAddon_Bindings["MYADDON_TOGGLE"](keystate)</Binding>
</Bindings>
```

and, at load time, assigns `BINDING_HEADER_MYADDON` and `BINDING_NAME_<name>`
from the labels. The XML body runs in the global environment, so it calls
through **one global dispatch table per addon** (`<Addon>_Bindings`), whose
entries forward to the private handlers. This table and the `BINDING_*`
globals are part of the normative list of global writes
([Code generation §14.1.1](../toolchain/codegen.md#1411-global-writes)).

- The dispatch table's name is the AddOn's name with every byte other than
  `[A-Za-z0-9_]` replaced by `_` (and `_` in front of a leading digit),
  followed by `_Bindings`.
- `default` is emitted as the binding's default-key attribute only when a
  targeted client's flavor table entry declares the `binding-defaults`
  feature; for clients without it, W8354. No client declares it yet. The
  module never calls `SetBinding` on the player's behalf.
- `Bindings.xml` is shared by all flavors, in every TOC layout (the client
  reads one per AddOn). It contains the union of all flavors' bindings; on a
  flavor where `@if` removed a binding's handler, the dispatch table runs
  nothing for it. A binding that `@if` removes on every flavor is left out.
- A package root with its own `Bindings.xml` (or `Bindings_<Suffix>.xml`) and
  `@binding` declarations is E4334 ([Code generation
  §14.8.5](../toolchain/codegen.md#1485-keybindings)).
- `settings.keybinding(name)` ([UI: settings](ui.md#settings)) checks that `name`
  is declared with `@binding`, or in the package's own `Bindings.xml` (E8353).
- Bindings also fire in combat. Handlers that call protected functions are
  subject to the usual combat restrictions (use `@outOfCombat` where
  appropriate).

## menu

`wowlua:menu` builds context menus through Blizzard's `MenuUtil`, which
every supported client has (decision S30). It never uses `UIDropDownMenu`,
which spreads taint.

```wowlua
import * as menu from "wowlua:menu"

menu.attach(rowButton, function(m: MenuBuilder)
    m:title(item.name)
    m:button(L.OPEN, function() open(item) end)
    m:checkbox(L.PINNED, function() return item.pinned end, function() item.pinned = not item.pinned end)
    m:divider()
    m:submenu(L.MOVE_TO, function(sub: MenuBuilder)
        for _, folder in ipairs(folders) do
            sub:radio(folder.name, function() return item.folder == folder.id end,
                      function() item.folder = folder.id end)
        end
    end)
end)

menu.open(anchor, generator)                  -- open immediately at a frame
```

| API | Signature | Notes |
|-----|-----------|-------|
| `open` | `(owner: Frame, gen: (m: MenuBuilder) -> ())` | Opens at the cursor, owned by `owner` |
| `attach` | `(frame: Frame, gen: (m: MenuBuilder) -> (), opts?: { button: string? })` | Opens on a mouse-up of `button` (default the right button); the frame must receive mouse input |
| `MenuBuilder` | `title(text)`, `button(text, onClick)`, `checkbox(text, isChecked, toggle)`, `radio(text, isSelected, select)`, `divider()`, `spacer()`, `submenu(text, gen)`, `disabled(text)` | All return the builder, so calls can be chained |

The generator runs every time the menu opens, so it always reflects current
state. The menus are Blizzard's own (they close on Escape and outside
clicks) and are not themed. Entry callbacks and the generator run through
the stdlib error policy.

## list

`wowlua:list` is a virtualized scrolling list on Blizzard's
ScrollBox/DataProvider, which every supported client has (decision S30), and
`wowlua:pool`.

```wowlua
import * as list from "wowlua:list"

local view = list.create(container, {
    rowHeight = 22,
    row = RowButton,                                   -- widget class
    init = function(row: RowButton, entry: Entry, index: number) row:bind(entry) end,
})

view:setData(entries)                                  -- Entry[]
view:onSelect(function(entry: Entry, index: number) showDetails(entry) end)
view:scrollTo(42)
```

| API | Declared signature | Notes |
|-----|--------------------|-------|
| `create` | `function create<T, R extends Frame>(parent: Frame, opts: ListOptions<T, R>): ListView<T>` | Type arguments are inferred from `row` and `init` (L15) |
| `ListOptions<T, R>` | `{ rowHeight: number, row: ClassOf<R>, init: (row: R, entry: T, index: number) -> (), reset: ((row: R) -> ())?, spacing: number? }` | |
| `ListView<T>` | `setData(data: T[])`, `refresh()`, `scrollTo(index)`, `onSelect(fn)`, `selected(): T?`, `data(): T[]` | |

Only visible rows exist. ScrollBox recycles small holder frames; each shows a
row from a `FramePool` of the given widget class (real instances, constructed
with the list's frame as their only argument), and `reset` runs when a row is
released. The list fills `parent`, with a scroll bar on the right. A left
click on a row selects its entry. Callbacks run through the stdlib error
policy.

## dialog

`wowlua:dialog` provides confirm, prompt, import and export dialogs. It uses
its own frames, created lazily once per addon and pooled, and **not**
`StaticPopupDialogs`, which is a global table that taints shared Blizzard
dialogs.

```wowlua
import * as dialog from "wowlua:dialog"
import * as serialize from "wowlua:serialize"

dialog.confirm({ text = L.RESET_CONFIRM, onAccept = function() saved.handle(db):reset() end })

dialog.prompt({
    text = L.PROFILE_NAME,
    validate = function(v: string) return #v > 0 and #v <= 32 end,
    onAccept = function(v: string) saved.handle(db).profiles:copy(v) end,
})

dialog.export({ title = L.EXPORT, value = serialize.exportString(profile, { version = 1 }) })

dialog.import({
    title = L.IMPORT,
    onImport = function(text: string)
        local ok, value = serialize.importString(text)
        if not ok then return L.INVALID_STRING end     -- returning a string keeps the dialog open with this error
        applyProfile(value)
    end,
})
```

| API | Signature | Notes |
|-----|-----------|-------|
| `confirm` | `(opts: { text: string, accept: string?, cancel: string?, onAccept: () -> (), onCancel: (() -> ())? })` | |
| `prompt` | `(opts: { text: string, default: string?, validate: ((v: string) -> boolean)?, onAccept: (v: string) -> (), onCancel: (() -> ())? })` | |
| `export` | `(opts: { title: string, value: string })` | Read-only multi-line box, text pre-selected for Ctrl+C |
| `import` | `(opts: { title: string, onImport: (text: string) -> string? })` | A returned string is shown as an error and the dialog stays open |
| `close` | `()` | Closes the current dialog |

- One dialog is visible per addon at a time. Opening another replaces the
  current one, which calls its `onCancel`.
- **Escape to close:** each dialog frame is created with the global name
  `<Addon>Dialog<N>` and registered in `UISpecialFrames`, the standard WoW
  mechanism, so Escape closes it everywhere, including in combat (decision S11).
  These frame names are in the normative global list
  ([Code generation §14.1.1](../toolchain/codegen.md#1411-global-writes)).
- Very large imports: the edit box accepts unlimited text (`SetMaxLetters(0)`).
  Parsing is `onImport`'s; for very large strings it can start a
  `wowlua:jobs` job.
- **Look** (decision S29): like the client's popups by default, with the
  dialog backdrop, the client's font objects and panel buttons. `confirm` and
  `prompt` show at the top center like `StaticPopup1`; `export` and `import`
  are a larger dialog (about 500×300) at the center, with a title, a scrolling
  multi-line box, an error line and buttons at the bottom.
  `dialog.setTheme(theme)` restyles them ([Widget themes](#widget-themes)).
- `export` is read-only: typing restores the value. `prompt` enables Accept
  (and Enter) only while `validate` agrees.

## Widget themes

`wowlua:dialog` and the built-in minimap button of `wowlua:launcher`
([UI](ui.md#launcher)) draw frames that ship inside
the addon, in front of players who may never have heard of wowlua. By default
they look like the client's own UI and carry no wowlua branding (decision
S17): they are built from the client's templates and font objects
(`BackdropTemplate`, `UIPanelButtonTemplate`, `InputBoxTemplate`,
`UIPanelScrollFrameTemplate`; `GameFontNormal`, `GameFontHighlight`,
`GameFontNormalSmall`, `GameFontHighlightSmall`, `ChatFontNormal`), so they
blend into any addon and follow the player's UI scale and font replacements.
Where a template does not exist on a flavor, the flavor table names the
equivalent.

An addon restyles them with one `WidgetTheme`, a type exported by
`wowlua:frames` (decision S18), which also exports the builders the standard
library's widgets use: `applyBackdrop(frame, theme?)`, `styleText(text, role,
theme?)` (roles `"title"`, `"body"`, `"muted"`, `"error"`) and
`button(parent, label, theme?)`.

```wowlua
import type WidgetTheme from "wowlua:frames"
import * as dialog from "wowlua:dialog"

const THEME: WidgetTheme = {
    font = { title = "Interface\\AddOns\\MyAddon\\media\\Title.ttf", size = 12 },
    colors = { background = { 0.08, 0.07, 0.06, 0.94 }, border = { 0.43, 0.40, 0.33 }, accent = { 1, 0.71, 0.28 } },
    borderSize = 1,
    button = "flat",
}

dialog.setTheme(THEME)
```

| Slot | Type | Default |
|------|------|---------|
| `font.title`, `font.body` | `string?`: a font file path or a font object name | The client's font objects |
| `font.size` | `number?` | The font object's size |
| `colors.background`, `border`, `title`, `text`, `muted`, `accent`, `onAccent`, `error` | `{ number, number, number, number? }?` (0–1) | The client's template art and font colors |
| `borderSize` | `number?` | The template's border |
| `padding` | `number?` | The template's insets |
| `button` | `("template" \| "flat")?` | `"template"`: the client's button template; `"flat"` draws a fill, a border and a label |

- Every slot is optional; a theme with only `colors.accent` changes only the
  accent, and `setTheme(nil)` returns to the default look.
- A theme is per addon and applies to frames created afterwards; pooled frames
  take it the next time they are shown.
- Not themed: menus (`wowlua:menu` uses `MenuUtil` on every client; decision
  S30), minimap buttons drawn by LibDBIcon,
  the Addon Compartment and LDB displays. They belong to the client or to the
  player's other addons.

## color

`wowlua:color` provides color utilities.

```wowlua
import * as color from "wowlua:color"

local orange = color.hex("ff8800")                  -- ColorMixin, validated and folded at compile time
label:SetTextColor(orange:GetRGB())
print(color.wrap("Warning", orange))                -- "|cffff8800Warning|r"
print(color.wrap(UnitName("player"), color.class("MAGE")))
local c = color.lerp(color.hex("ff0000"), color.hex("00ff00"), 0.25)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `Color` | type | What `CreateColor` returns: the API's `colorRGBA` (a `ColorMixin` with `r`, `g`, `b`, `a`). The functions below return it where this table says `ColorMixin`. |
| `hex` | `(hex: string) -> ColorMixin` | `"rrggbb"` or `"aarrggbb"`, with an optional `#`. Literal arguments are validated (E8390) and folded to `CreateColor(r, g, b, a)` with constant components. |
| `rgb` | `(r: number, g: number, b: number, a: number?) -> ColorMixin` | Components 0–1 |
| `toHex` | `(c: ColorMixin, withAlpha?: boolean) -> string` | |
| `class` | `(classFile: string) -> ColorMixin?` | Prefers `CUSTOM_CLASS_COLORS` (ClassColors addon) when present, then the client's class colors. A new color each call. |
| `quality` | `(quality: number) -> ColorMixin?` | Item quality colors |
| `wrap` | `(text: string \| Secret<string>, c: ColorMixin) -> string \| Secret<string>` | Uses concatenation only, so secret strings are allowed |
| `colorize` | `(hex: string, text: string) -> string` | |
| `strip` | `(text: string) -> string` | Removes `\|cAARRGGBB`, `\|cnNAME:` and `\|r` escape sequences |
| `lerp` | `(a: ColorMixin, b: ColorMixin, t: number) -> ColorMixin` | |

## media

`wowlua:media` handles fonts, textures and sounds, with optional
LibSharedMedia interop. If LibSharedMedia-3.0 is loaded (by any addon), media
is registered with it and fetched through it, so users can pick shared media.
Otherwise a built-in per-addon registry is used. There is no hard dependency.

```wowlua
import * as media from "wowlua:media"
import asset from "wowlua:meta"

media.register("font", "MyAddon Bold", asset("media/fonts/Bold.ttf"))
media.register("statusbar", "MyAddon Flat", asset("media/textures/flat.tga"))

local path = media.fetch("statusbar", db.barTexture)    -- falls back to the default if unknown
for _, name in ipairs(media.list("font")) do … end
media.onRegistered(function(kind: MediaType, name: string) refreshDropdowns() end)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `register` | `(kind: MediaType, name: string, path: string \| number)` | Paths from `asset()` are checked at compile time ([Build metadata](../language/06-meta.md)) |
| `fetch` | `(kind: MediaType, name: string?, opts?: { noDefault: boolean? }) -> (string \| number)?` | Unknown names return the kind's default unless `noDefault`: the client's `STANDARD_TEXT_FONT`, its status bar, tooltip border and tooltip background; sounds have none. With LibSharedMedia, its defaults. |
| `list` | `(kind: MediaType) -> string[]` | Sorted |
| `onRegistered` | `(fn: (kind: MediaType, name: string) -> ()) -> Subscription` | Fires for this addon's registrations, and with LibSharedMedia for every addon's |
| `MediaType` | `"font" \| "statusbar" \| "border" \| "background" \| "sound"` | Unknown literal kinds are E8400 |

LibSharedMedia may be embedded in an addon that loads after this one.
Registrations are therefore queued, and the module looks up the library through
`LibStub(…, true)` at load, on `PLAYER_LOGIN`, and after each later
`ADDON_LOADED` until it is found. Queued registrations are then flushed to it.

## Testing

`wowlua:mock` support used by this chapter: `mock.drag(frame, dx, dy)`,
`mock.setScreen(width, height)`, `mock.setInCombat`, `mock.pressBinding(name,
keystate?)` (the test runner reads the generated `Bindings.xml`),
`mock.lastMenu()` (returns the built menu as a tree) and `mock.clickMenu(path)`,
`mock.scroll(view, offset)`, `mock.dialog()`, `mock.dialogAccept(value?)` and
`mock.dialogCancel()`, stubs for `CreateColor` and the class color tables, and
`mock.provideLibrary("LibSharedMedia-3.0", impl?)` to simulate a present or
absent library.
