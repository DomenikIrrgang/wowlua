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
| `MovableOptions` | `store`, `key`, `clamp?`, `button?` (default `"LeftButton"`), `modifier?` (`"SHIFT" \| "CTRL" \| "ALT"`), `resize?` (`{ min: number[]?, max: number[]?, grip: Frame? }`, sizes as `{ width, height }`), `scaleWheel?` (a modifier), `scaleRange?` (default `{ 0.5, 2 }`), `onChange?` (`(pos: FramePosition?) -> ()`, after a save, or `nil` after a reset), `editMode?` (`{ label: string, lock: boolean? }`, see [Edit Mode](#edit-mode)) | |
| `MovableHandle` | `lock(locked: boolean)`, `isLocked()`, `reset()`, `restore()`, `save()` | A locked frame cannot be moved, resized or scaled with the mouse; the built-in grip is hidden (decision S24). |
| `FramePosition` | `{ point: AnchorPoint, relativePoint: AnchorPoint, x: number, y: number, width: number?, height: number?, scale: number?, layouts: { [string]: FramePosition }? }` | Positions are always stored relative to `UIParent`, never to named frames that may not exist on the next load |

Semantics:

- `store[key]` must have type `FramePosition?` (E8340): optional, and its
  value type assignable to and from `FramePosition` (the same fields under
  another name pass; `string?` or `{ x: number }?` do not). The store is a
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
- **Secrets:** a position, size or scale that reads as secret (a frame
  anchored to a restricted one) is neither saved nor applied: `save` (and a
  drag or wheel notch) does nothing while it does, and `reset` restores only
  the anchors, size and scale that were readable when `movable` was called.
  A stored position that cannot be read (secret, or with secret fields) is
  not applied. `lock` ignores a secret.

### The game's frames

The game's frames are read by name or by key, often parts the declarations
do not know or some clients lack; these read them without trusting a type
(decision S42):

```wowlua
local cf = frames.find("CharacterFrame")                   -- Frame?: nil before its addon loads
local close = frames.asButton(tbl.field(cf, "CloseButton")) -- Button?
local list = frames.asFrame(tbl.field(cf, "PaneHost", "List"))
if list then frames.keepAway(list, { fade = true }) end     -- out of sight, still shown
```

| API | Signature | Notes |
|-----|-----------|-------|
| `find` | `(name: string) -> Frame?` | The global frame of that name, nil when there is none |
| `textures`, `fontStrings` | `(frame: Frame) -> Texture[]`, `-> FontString[]` | The textures (its art) or font strings among the frame's own regions; none for a frame that is forbidden (decision S45); any number of them (gathered inside the protected call: `pcall` handing back a hundred values or more crashes the game, decision S50) |
| `children` | `(frame: Frame) -> Frame[]` | The frame's children the addon may read |
| `keyed` | `(frame: unknown) -> { [string]: Region }` | The widgets the frame holds in its own fields, by key (`frame.ScrollBox`): what its template or code gave it, named or not; forbidden ones left out, none for a value that is no table the addon may read (decision S49) |
| `stateTextures` | `(button: Button) -> Texture[]` | The button's state textures it has: normal, pushed, highlight, disabled, and a check button's checked ones |
| `rows` | `(list: unknown) -> Frame[]` | The frames one of the game's lists shows: an object pool's active ones (`EnumerateActive`) or a scroll box's (`ForEachFrame`), in its order; none for anything else |
| `mapPins` | `(map: unknown, template: string) -> Frame[]` | The pins one of the game's maps (Blizzard's map canvas: the world map, the flight map) shows of a pin template, through `EnumeratePinsByTemplate`; none for anything else (decision S46) |
| `asRegion`, `asFrame`, `asButton`, `asCheckButton`, `asEditBox`, `asScrollFrame`, `asSlider`, `asStatusBar`, `asCooldown`, `asFontString`, `asTexture`, `asBrowser` | `(v: unknown) -> Region?`, … | `v` when its `IsObjectType` says it is one (called protected; a table the addon may not read is none), else nil |
| `keepAway` | `(frame: Frame, opts?: { fade?: boolean, when?: () -> boolean }) -> AwayHandle` | Keeps the frame out of sight while it stays shown (its scripts and children keep working): moved off screen, and with `fade` see-through; again a frame after the game places it (with `fade`, also after it sets its alpha) and when it shows. Only while `when` returns true (default: always). A protected frame moves only out of combat (`wowlua:combat`'s `defer`). The handle's `apply()` puts it away now, `release()` stops (hooks can't be removed: they then do nothing). |

### Edit Mode

With `editMode = { label = … }` the frame joins the client's Edit Mode, which
every active client has but Cata Classic's (decision S36):

```wowlua
frames.movable(bar, { store = db, key = "bar", editMode = { label = "MyAddon Bar" } })
```

- **In Edit Mode** the frame shows the client's selection overlay
  (`EditModeSystemSelectionTemplate`): highlighted like Blizzard's frames, with
  `label` as its name, and selected on click (which deselects the others).
  It is dragged through the overlay; the resize grip sits above the overlay,
  and `scaleWheel` works on it.
- **Per layout.** A save in Edit Mode stores the position under the active
  layout's name in `layouts`, and as the top-level fields. A layout without
  its own position uses the top-level fields (the last position saved).
  Switching layouts, or reverting Edit Mode's changes, moves the frame to that
  layout's position.
- **Saved at once.** Positions are saved when the drag ends, like
  LibEditMode's, not with Edit Mode's Save button; leaving Edit Mode without
  saving does not undo them.
- **Locked outside Edit Mode**, so the player arranges frames in one place;
  `lock = false` keeps the frame movable outside Edit Mode too (then
  `MovableHandle:lock` applies there).
- On a client without Edit Mode the option is ignored and the frame is an
  ordinary movable frame. A client has Edit Mode when its
  `EditModeManagerFrame` has the manager's methods (`IsEditModeActive`,
  `GetActiveLayoutInfo`, `ClearSelectedSystem`, `SelectSystem`,
  `UpdateLayoutInfo`) and `EventRegistry` exists; Cata Classic's client
  has only a stub of the manager (`IsEditModeActive`). The compiler warns
  (W8301) where a targeted flavor's declarations lack
  `EditModeManagerFrame.GetActiveLayoutInfo` and the call passes `editMode`
  (decision S59).

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
| `MenuBuilder` | `title(text)`, `button(text, onClick)`, `checkbox(text, isChecked, toggle)`, `radio(text, isSelected, select)`, `divider()`, `spacer()`, `submenu(text, gen)`, `disabled(text)` | All return the builder, so calls can be chained. `text` is `string \| Secret<string>`: MenuUtil measures it, so a secret shows as `UNKNOWN`. A secret `isChecked`/`isSelected` answer reads as unchecked. |

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
policy. Entries reach `init` and `onSelect` as they are, secret ones too (the
list never compares or sorts them); a `data` table that cannot be read
(secret, `canaccesstable`) shows empty, and `scrollTo` ignores a secret
index.

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
| `confirm` | `(opts: { text: string \| Secret<string>, accept: string?, cancel: string?, onAccept: () -> (), onCancel: (() -> ())? })` | A secret `accept`/`cancel` label shows the default (buttons refuse secrets) |
| `prompt` | `(opts: { text: string \| Secret<string>, default: (string \| Secret<string>)?, validate: ((v: string) -> boolean \| Secret<boolean>)?, onAccept: (v: string) -> (), onCancel: (() -> ())? })` | A secret `default` is left out (edit boxes refuse secrets); a secret `validate` answer counts as no |
| `export` | `(opts: { title: string \| Secret<string>, value: string \| Secret<string> })` | Read-only multi-line box, text pre-selected for Ctrl+C. A secret `value` shows an empty box. |
| `import` | `(opts: { title: string \| Secret<string>, onImport: (text: string) -> string? })` | A returned string is shown as an error and the dialog stays open |
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
| `toHex` | `(c: ColorMixin, withAlpha?: boolean) -> string` | White (`ffffff`) for a color with secret components (colors the client derives from secrets, such as curve results) |
| `class` | `(classFile: string) -> ColorMixin?` | Prefers `CUSTOM_CLASS_COLORS` (ClassColors addon) when present, then the client's class colors. A new color each call. |
| `quality` | `(quality: number) -> ColorMixin?` | Item quality colors |
| `wrap` | `(text: string \| Secret<string>, c: ColorMixin \| Secret<ColorMixin>) -> string \| Secret<string>` | Uses concatenation only, so secret strings are allowed. A secret color, or one with secret components, is left to the client's `C_ColorUtil.WrapTextInColor`; without it the text comes back uncolored. |
| `colorize` | `(hex: string, text: string) -> string` | |
| `strip` | `(text: string) -> string` | Removes `\|cAARRGGBB`, `\|cnNAME:` and `\|r` escape sequences |
| `lerp` | `(a: ColorMixin, b: ColorMixin, t: number) -> ColorMixin` | The nearer of `a` and `b` when either has secret components |

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
| `register` | `(kind: MediaType, name: string, path: string \| number)` | Paths from `asset()` are checked at compile time ([Build metadata](../language/06-meta.md)). A secret name cannot be a key and is not registered. |
| `fetch` | `(kind: MediaType, name: string?, opts?: { noDefault: boolean? }) -> (string \| number)?` | Unknown names (and secret ones) return the kind's default unless `noDefault`: the client's `STANDARD_TEXT_FONT`, its status bar, tooltip border and tooltip background; sounds have none. With LibSharedMedia, its defaults. |
| `list` | `(kind: MediaType) -> string[]` | Sorted |
| `onRegistered` | `(fn: (kind: MediaType, name: string) -> ()) -> Subscription` | Fires for this addon's registrations, and with LibSharedMedia for every addon's |
| `MediaType` | `"font" \| "statusbar" \| "border" \| "background" \| "sound"` | Unknown literal kinds are E8400 |

LibSharedMedia may be embedded in an addon that loads after this one.
Registrations are therefore queued, and the module looks up the library through
`LibStub(…, true)` at load, on `PLAYER_LOGIN`, and after each later
`ADDON_LOADED` until it is found. Queued registrations are then flushed to it.

## alert

`wowlua:alert` gets the player's attention: sounds, raid-warning text and the
taskbar flash, or all of them at once with `warn`.

```wowlua
import * as alert from "wowlua:alert"

alert.sound(SOUNDKIT.RAID_WARNING)                 -- a sound kit
alert.sound("MyAddon Ping")                        -- a wowlua:media sound
alert.warn("Interrupt!", { sound = SOUNDKIT.RAID_WARNING, flash = true })
```

| API | Signature | Notes |
|-----|-----------|-------|
| `sound` | `(ref: SoundRef, channel?: SoundChannel) -> boolean` | A number below 1000000 is a sound kit (`PlaySound`); other numbers and paths go to `PlaySoundFile`, and names are looked up in `wowlua:media` first. The channel defaults to `"Master"`. Returns whether it plays. The client refuses secrets: a secret `ref` does not play, a secret channel is `"Master"`. |
| `flash` | `()` | Flashes the client's taskbar icon (`FlashClientIcon`) |
| `raidWarning` | `(text: string \| Secret<string>, color?: Color)` | Text in the middle of the screen through `RaidWarningFrame`, orange by default |
| `warn` | `(text: string \| Secret<string>, opts?: { sound?, channel?, flash?, color?, throttle? })` | A raid warning with an optional sound and flash. The same text again within `throttle` seconds (default 1) does nothing. A secret text cannot be a key, so it is never throttled; a secret `throttle` is 1, a secret `flash` is no. |
| `SoundRef`, `SoundChannel` | `number \| string`, `"Master" \| "SFX" \| "Music" \| "Ambience" \| "Dialog"` | |

## links

`wowlua:links` puts clickable links into the player's own chat, using the
client's `addon` link type. A click reaches the handlers of the addon that made
the link, and no other addon.

```wowlua
import * as links from "wowlua:links"

print(`Import failed: {links.link("Show details", "import-log")}`)
links.onClick(function(data: string, button: string)
    if data == "import-log" then showLog() end
end)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `link` | `<S extends string \| Secret<string>>(text: S, data: string \| Secret<string>, color?: Color) -> S` | `\|Haddon:<Addon>:<data>\|h[text]\|h`, colored like spell links by default. `data` may not contain `\|` (runtime error). A secret text gives a secret link; secret data cannot be checked, so it gives `[text]` without a link. |
| `onClick` | `(fn: (data: string, button: string) -> ()) -> Subscription` | The link's `data` and the mouse button |

Clicks arrive through the client's `EventRegistry` callback `SetItemRef`,
which every supported client fires for `addon` links. The client does not
send addon links to other players, so they are for the player's own chat
frames only.

## chat

`wowlua:chat` opens the chat line the player types in, through the game's own
chat code: `ChatFrameUtil`, or the older `ChatFrame_…` and `ChatEdit_…`
globals on clients that have those. Where a client has neither, nothing
happens (decision S44).

```wowlua
import * as chat from "wowlua:chat"

menu:CreateButton("Whisper", function() chat.whisper(member.name) end)
linkButton:SetScript("OnClick", function() chat.insertLink(recipeLink) end)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `whisper` | `(name: string, frame?: Frame)` | Opens the chat line to whisper `name`, as the game's own "Whisper" does (`SendTell`); the frame goes to the game only when given |
| `whisperBnet` | `(accountName: string, frame?: Frame)` | Opens the chat line to whisper a Battle.net friend by their account name as the game gives it (protected text only the game draws), as the friends list does (`SendBNetTell`) (decision S46) |
| `open` | `(text: string, frame?: Frame)` | Opens the chat line with `text` in it (`OpenChat`) |
| `insertLink` | `(link: string)` | Puts `link` into the line being typed, as a shift-click does (`InsertLink`); with none open, opens one with it |
| `filter` | `(event: string, fn: (text: unknown, ...unknown) -> boolean?) -> Subscription` | Hides the chat lines of `event` for which `fn` returns true, through the game's message filters (`AddMessageEventFilter`); `fn` gets the line's values, its text first. Isolated: an error is reported and the line shows. Cancelling removes the filter (`RemoveMessageEventFilter`) (decision S46) |

It also finds the game's chat windows, typed as `chat.Window`
(`ScrollingMessageFrameMixin & Frame`: a message frame, its window ID
`GetID()`), for a chat addon that restyles or mirrors them (decision S48):

| API | Signature | Notes |
|-----|-----------|-------|
| `windows` | `() -> Window[]` | The windows that exist: the numbered ones (`ChatFrame1` up to `NUM_CHAT_WINDOWS`, 10 where the client doesn't say), then the temporary ones of `CHAT_FRAMES`, in the game's order |
| `docked` | `() -> Window[]` | The windows docked with General, in the dock's order (`GENERAL_CHAT_DOCK`); `ChatFrame1` alone where there is no dock |
| `window` | `(name: string) -> Window?` | The chat window of a global name (`"ChatFrame3"`) |
| `asWindow` | `(v: unknown) -> Window?` | `v` when it is a frame with a message frame's `AddMessage` and `GetMessageInfo` |
| `editBox` | `(w: Window) -> EditBox?` | The window's edit box (`ChatFrame1EditBox`) |
| `isTemporary` | `(w: Window) -> boolean` | Whether it is a temporary window (a whisper window, which closes) |
| `conversation` | `(w: Window) -> (string?, string?)` | What a temporary window is for: its chat type (`"WHISPER"`, …) and target (the player); nils for any other window and for values that aren't readable |

## Testing

`wowlua:mock` support used by this chapter: `mock.drag(frame, dx, dy)`,
`mock.setScreen(width, height)`, `mock.setInCombat`, `mock.pressBinding(name,
keystate?)` (the test runner reads the generated `Bindings.xml`),
`mock.lastMenu()` (returns the built menu as a tree) and `mock.clickMenu(path)`,
`mock.scroll(view, offset)`, `mock.dialog()`, `mock.dialogAccept(value?)` and
`mock.dialogCancel()`, stubs for `CreateColor` and the class color tables, and
`mock.provideLibrary("LibSharedMedia-3.0", impl?)` to simulate a present or
absent library.
