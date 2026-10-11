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
| `MovableOptions` | `store`, `key`, `clamp?`, `button?` (default `"LeftButton"`), `modifier?` (`"SHIFT" \| "CTRL" \| "ALT"`), `resize?` (`{ min: {number, number}?, max: {number, number}?, grip: Frame? }`), `scaleWheel?`, `scaleRange?` (default `{0.5, 2}`), `onChange?` | |
| `MovableHandle` | `lock(locked: boolean)`, `isLocked()`, `reset()`, `restore()`, `save()` | |
| `FramePosition` | `{ point: AnchorPoint, relativePoint: AnchorPoint, x: number, y: number, width: number?, height: number?, scale: number? }` | Positions are always stored relative to `UIParent`, never to named frames that may not exist on the next load |

Semantics:

- `store[key]` must have type `FramePosition?` (E8340). The store is a
  `wowlua:saved` binding. Positions are restored once the store is loaded
  (decision S3), so calling `movable` at module top level is fine.
- The module calls `SetMovable`, `RegisterForDrag`,
  `StartMoving`/`StopMovingOrSizing`, and `SetUserPlaced(false)`, so the game's
  own layout cache does not fight with the saved position.
- **Combat safety:** for protected frames, dragging is disabled while
  `InCombatLockdown()` is true, and `restore`/`reset` are deferred through
  `wowlua:combat` until combat ends.
- A built-in resize grip (bottom-right) is created when `resize` is given
  without `grip`.

## bindings

`wowlua:bindings` declares keybindings in code. The compiler generates the
`Bindings.xml` file, which the game loads automatically from the AddOn
root, and the required `BINDING_HEADER_*` / `BINDING_NAME_*` globals.

```wowlua
import binding, header from "wowlua:bindings"
import L from "wowlua:locale"

header("MYADDON", L.BINDING_HEADER)           -- category shown in the Key Bindings UI

@binding("MYADDON_TOGGLE", { label = L.TOGGLE_WINDOW, header = "MYADDON" })
local function toggleWindow()
    window:SetShown(not window:IsShown())
end

@binding("MYADDON_PUSH_TO_TALK", { label = "Push to talk", runOnUp = true })
local function pushToTalk(keystate: "down" | "up") … end
```

| API | Signature | Notes |
|-----|-----------|-------|
| `header` | `(id: string, label: string)` | Statement at module top level. `id` must be a literal. |
| `@binding(name, opts)` | function decorator | `name` is a literal matching `[A-Z][A-Z0-9_]*` (E8352), unique within the addon (E8351) |
| `opts` | `{ label: string, header: string?, runOnUp: boolean?, default: string? }` | All fields except `label` must be compile-time constants. `label` may be any string expression, e.g. a locale entry. |
| Handler | `() -> ()` or `(keystate: "down" \| "up") -> ()` (with `runOnUp`) | |

**Compile-time collection.** `@binding` is allowed only on top-level function
declarations, and `header` only as a top-level statement, both with constant
arguments as described (E8350). The compiler collects all of them across the
addon, per flavor, and generates:

```xml
<!-- Bindings.xml (generated) -->
<Bindings>
  <Binding name="MYADDON_TOGGLE" header="MYADDON">MyAddon_Bindings["MYADDON_TOGGLE"](keystate)</Binding>
  <Binding name="MYADDON_PUSH_TO_TALK" runOnUp="true">MyAddon_Bindings["MYADDON_PUSH_TO_TALK"](keystate)</Binding>
</Bindings>
```

and, at load time, assigns `BINDING_HEADER_MYADDON` and `BINDING_NAME_<name>`
from the labels. The XML body runs in the global environment, so it calls
through **one global dispatch table per addon** (`<Addon>_Bindings`), whose
entries forward to the private handlers. This table and the `BINDING_*`
globals are part of the normative list of global writes
([Code generation §14.1.1](../toolchain/codegen.md#1411-global-writes)).

- `default` is emitted as the binding's default-key attribute only on clients
  whose flavor table declares support for it. Elsewhere it is ignored, with
  W8354. The module never calls `SetBinding` on the player's behalf.
- In single-TOC builds, `Bindings.xml` is shared by all flavors. It contains the
  union of all flavors' bindings, and on flavors where a binding's handler was
  removed by `@if`, the dispatch table has a no-op entry for it.
- `settings.keybinding(name)` ([UI: settings](ui.md#settings)) checks that `name`
  is declared with `@binding` (E8353).
- Bindings also fire in combat. Handlers that call protected functions are
  subject to the usual combat restrictions (use `@outOfCombat` where
  appropriate).

## menu

`wowlua:menu` builds context and dropdown menus. It uses Blizzard's
`MenuUtil` on clients where the flavor table declares it, and a built-in,
taint-free menu frame elsewhere. It never uses `UIDropDownMenu`, which spreads
taint.

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
| `attach` | `(frame: Frame, gen: (m: MenuBuilder) -> (), opts?: { button: string? })` | Opens on right-click (default) |
| `MenuBuilder` | `title(text)`, `button(text, onClick)`, `checkbox(text, isChecked, toggle)`, `radio(text, isSelected, select)`, `divider()`, `spacer()`, `submenu(text, gen)`, `disabled(text)` | All return the builder, so calls can be chained |

The generator runs every time the menu opens, so it always reflects current
state. On fallback clients, menu buttons are pooled (`wowlua:pool`) and the
menu closes on Escape, on an outside click, or when the owner hides.

## list

`wowlua:list` is a virtualized scrolling list on top of `wowlua:pool`. It
uses Blizzard's ScrollBox/DataProvider where the flavor table declares it, and
a built-in faux-scroll implementation elsewhere.

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

Only visible rows exist. Rows are recycled through a `FramePool` of the given
widget class, and `reset` runs on release.

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
- Very large imports: the edit box accepts unlimited text (`SetMaxLetters(0)`),
  and import parsing of large strings runs as a `wowlua:jobs` job.

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
| `hex` | `(hex: string) -> ColorMixin` | `"rrggbb"` or `"aarrggbb"`, with an optional `#`. Literal arguments are validated (E8390) and folded to `CreateColor(r, g, b, a)` with constant components. |
| `rgb` | `(r: number, g: number, b: number, a: number?) -> ColorMixin` | Components 0–1 |
| `toHex` | `(c: ColorMixin, withAlpha?: boolean) -> string` | |
| `class` | `(classFile: string) -> ColorMixin?` | Prefers `CUSTOM_CLASS_COLORS` (ClassColors addon) when present, then the client's class colors |
| `quality` | `(quality: number) -> ColorMixin?` | Item quality colors |
| `wrap` | `(text: string \| Secret<string>, c: ColorMixin) -> string \| Secret<string>` | Uses concatenation only, so secret strings are allowed |
| `colorize` | `(hex: string, text: string) -> string` | |
| `strip` | `(text: string) -> string` | Removes `\|c…`/`\|r` escape sequences |
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
| `fetch` | `(kind: MediaType, name: string?, opts?: { noDefault: boolean? }) -> (string \| number)?` | Unknown names return the kind's default (e.g. the standard UI font) unless `noDefault` |
| `list` | `(kind: MediaType) -> string[]` | Sorted |
| `onRegistered` | `(fn: (kind: MediaType, name: string) -> ()) -> Subscription` | Fires for registrations by any addon when LibSharedMedia is present |
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
