# 8.2 Standard Library: UI

## locale

`wowlua:locale` provides typed localization, checked at compile time. The game
loads the base locale plus the player's language, and nothing else.

### Locale files

Translations are **JSON files**, one per locale, in `locales/` (configurable
with `build.locales-dir`):

```
locales/
├── enUS.json      # base locale: defines the keys
├── deDE.json
└── frFR.json
```

```json
{
  "TITLE": "Kill Tracker",
  "WELCOME": "Welcome back, {name}!",
  "KILLS": "{count} kills in {zone}"
}
```

- The folder is read when the package imports `wowlua:locale`, and only its
  `*.json` files, so an adopted addon's own `locales/` of Lua files is left
  alone.
- Each file is a JSON object of strings (E8115 otherwise).
- The **base locale** (default `enUS`, configurable with `build.locale-base`)
  defines the set of keys. A missing base-locale file is error E8106. Keys are
  `[A-Z0-9_]+` (E8060). A key starting with a digit is reachable only as
  `L["1ST"]` and is not part of `L`'s type.
- Other locales may omit keys, and the base string is used. Unknown keys are
  error E8061, and missing keys are warning W8062 with a per-locale summary.
- File names must be a valid locale code plus `.json` (E8105). Valid codes:
  `enUS`, `enGB`, `deDE`, `esES`, `esMX`, `frFR`, `itIT`, `koKR`, `ptBR`, `ruRU`,
  `zhCN`, `zhTW`.
- **`enGB`.** British English clients report `GetLocale() == "enGB"`. They load
  `enUS` (as base or as a locale file), and then `enGB.json` if it exists, which
  overrides individual keys.
- Values may contain WoW escape sequences (`|cff…|r`, `|T…|t`) and newlines.

### Usage

```wowlua
import L from "wowlua:locale"

title:SetText(L.TITLE)                                   -- string
print(L.WELCOME({ name = UnitName("player") }))          -- typed parameters
print(L.KILLS({ count = kills, zone = GetZoneText() }))
```

- `L` has a generated type. A key without placeholders is a `string`. A key with
  placeholders is a function `(params: { name: any, … }) -> string`. If any
  argument's static type may be secret, the result is
  `string | Secret<string>` (intrinsic check), because concatenating a secret
  yields a secret.
- Placeholders are `{name}`. `{{` and `}}` produce literal braces. Every locale
  must use **exactly the same placeholder names** for a key as the base locale
  (E8063), but may use them in any order and repeat them.
- Unknown keys (`L.TITEL`) are compile errors (E2003) and get completion in the
  LSP.
- Parameters are converted with `tostring`. Formatting numbers is the caller's
  job (`FormatLargeNumber(n)`, `("%.1f"):format(x)`).

### Lowering and loading

Each locale compiles to a **self-contained** Lua file (no runtime needed) that
fills the addon's locale table, stored in the addon table under the reserved
`__wowlua` key. The runtime, which loads later, reuses that table instead of
replacing it (see [Code generation §14.9.2](../toolchain/codegen.md#1492-file-order)).
Placeholder strings become small functions using concatenation. Every
non-base locale file starts with a cheap runtime guard:

```lua
-- wowlua/locales/deDE.lua
if GetLocale() ~= "deDE" then return end
local __wl_ns = select(2, ...); local __wl = __wl_ns.__wowlua or {}; __wl_ns.__wowlua = __wl
local L = __wl.locale or {}; __wl.locale = L
L.WELCOME = function(p) return "Willkommen zurück, " .. tostring(p.name) .. "!" end
```

**Load order.** Locale files are listed **before every other Lua file**, base
locale first, so `L` is complete before any module runs. They are never
bundled, which is why no bundle segments are needed for them.

**Client-only loading.** With `build.locales = "client"` (default):

- If **every** targeted client supports `[AllowLoadTextLocale]` (a capability
  in the flavor table, checked like game-type names), the TOC lists the base
  locale unconditionally and every other locale with
  `[AllowLoadTextLocale <code>]` (`enUS` is tagged `enUS enGB` when it is not the
  base). The client then reads only its own language. This works in both TOC
  layouts.
- Otherwise every locale file is listed without a directive, and the runtime
  guard skips the files that don't match.

With `build.locales = "all"`, all locales are compiled into one file
(`wowlua/locales/locales.lua`): the base strings, then each locale's under a
`GetLocale()` test.

## pool

`wowlua:pool` provides typed object pools that reuse frames and tables
instead of creating garbage.

```wowlua
import FramePool, TablePool from "wowlua:pool"

local rows = FramePool.new(RowButton, listFrame)        -- widget class + constructor args
local row = rows:acquire()                               -- RowButton
row:SetPoint("TOP", 0, -20 * i)
rows:releaseAll()

local buffers = TablePool.new()
local t = buffers:acquire()
buffers:release(t)                                       -- wiped and reused
```

```wowlua
declare module "wowlua:pool"
    interface Pool<T>
        function acquire(): (T, boolean)        -- instance, true if newly created
        function release(obj: T)
        function releaseAll()
        function active(): () -> T?              -- iterator over acquired objects
        function count(): (number, number)       -- active, inactive
    end

    namespace FramePool
        function new<T, A...>(cls: ClassOf<T>, ...: A...): Pool<T>       -- A: pack of the constructor's parameters
        function fromTemplate(frameType: FrameType, parent: Frame?, template: string?): Pool<Frame>
    end

    namespace TablePool
        function new(opts: { max: number? }?): Pool<table>
    end
end
```

- `FramePool.new` forwards the constructor arguments, type-checked against the
  class's constructor through a type pack.
- `release` hides the frame, clears its points and calls `obj:onRelease()` if the
  class defines it. For **protected** frames (`frame:IsProtected()`) during combat
  lockdown, hiding and clearing points is blocked, so the release is deferred
  through `wowlua:combat` and the object stays active until combat ends.
- `TablePool:release` wipes the table. `opts.max` caps the inactive tables kept.
- In debug builds, releasing an object twice or releasing a foreign object raises
  an error.

## settings

`wowlua:settings` provides declarative options pages built on Blizzard's
**Settings API** (`Settings.RegisterVerticalLayoutCategory` and friends).
That API is available on every current flavor, so no fallback UI is needed.
Values are bound to typed SavedVariables from `wowlua:saved`.

```wowlua
import * as settings from "wowlua:settings"
import * as saved from "wowlua:saved"
import L from "wowlua:locale"

settings.register({
    title = L.TITLE,
    store = db,                                -- from saved.bind(...)
    controls = {
        settings.header(L.GENERAL),
        settings.checkbox("showMinimap", { label = L.SHOW_MINIMAP, tooltip = L.SHOW_MINIMAP_TT }),
        settings.slider("scale", { label = L.SCALE, min = 0.5, max = 2, step = 0.05,
                                   format = function(v: number) return ("%.0f%%"):format(v * 100) end }),
        settings.dropdown("anchor", { label = L.ANCHOR, options = {
            { value = "TOPLEFT", label = L.TOP_LEFT },
            { value = "CENTER",  label = L.CENTER },
        } }),
        settings.keybinding("MYADDON_TOGGLE", { label = L.TOGGLE }),
        settings.button(L.RESET, function() saved.handle(db):reset() end),
    },
    subcategories = {
        { title = L.ADVANCED, controls = { … } },
    },
    onChange = function(key: string, value: any) refresh() end,
})

settings.open()                                -- Settings.OpenToCategory for this addon
```

### Controls

| Control | Signature |
|---------|-----------|
| `header` | `header(text: string)` |
| `text` | `text(content: string)`: a description line |
| `checkbox` | `checkbox(key, { label: string, tooltip: string? })`. The field must be `boolean`. |
| `slider` | `slider(key, { label, tooltip?, min: number, max: number, step: number, format: ((v: number) -> string)? })`. The field must be `number`. |
| `dropdown` | `dropdown(key, { label, tooltip?, options: { value: V, label: string }[] })`. Options are an **ordered array**, shown in that order. Each `value` must belong to the field's type (E8107). |
| `color` | `color(key, { label, tooltip?, alpha: boolean? })`. The field must be `{ r: number, g: number, b: number, a: number? }`. |
| `keybinding` | `keybinding(name, { label })`. `name` must be a binding declared with [`wowlua:bindings`](interface.md#bindings) (E8353). |
| `button` | `button(text: string, onClick: () -> ())` |
| `custom` | `custom(factory: (parent: Frame) -> Frame, { height: number? })`: a canvas area for arbitrary UI |

### Rules

- **Key checking.** `register` and the control constructors are
  intrinsic-checked: the compiler takes the type of `store` and checks every
  control key in the `controls` literal against it. An unknown key is error
  E8113. A field of the wrong type for the control is error E8070.
- **Binding mechanism.** Each control is registered with
  `Settings.RegisterProxySetting`. Its getter and setter read and write the
  store, so defaults apply and profile switches are reflected. The exact
  signature differs between clients, so the module contains a per-flavor adapter
  selected at compile time.
- **Timing.** `register` may be called at top level. The category is created at
  `ADDON_LOADED`, after the store is loaded ([phase order](core.md#phase-order)).
- **One registration.** `register` may be called once per addon (E8114). Use
  `subcategories` for more pages. It returns a `SettingsCategory` handle
  (`open()`, `refresh()`, `id`).
- **Profile changes.** When the store's profile changes, every control is
  re-read automatically (`refresh()`).
- **Combat.** Opening the Settings panel during combat lockdown is not reliable,
  so `settings.open()` in combat is deferred through `wowlua:combat` and a
  notice is printed.
- **Slash access.** The module creates no slash commands. Call
  `settings.open()` from a `wowlua:slash` router command. There is no automatic
  `/<addon> config` command (decision S15).

## launcher

`wowlua:launcher` gives the addon a minimap button, an Addon Compartment
entry and an LDB launcher, all from one declaration.

```wowlua
import * as launcher from "wowlua:launcher"
import * as settings from "wowlua:settings"
import addon from "wowlua:meta"
import L from "wowlua:locale"

launcher.register({
    icon = "Interface\\Icons\\INV_Misc_QuestionMark",
    label = addon.title,
    onClick = function(button: string)
        if button == "RightButton" then settings.open() else window:Toggle() end
    end,
    tooltip = function(tip: GameTooltip)
        tip:AddLine(addon.title)
        tip:AddLine(L.CLICK_TO_TOGGLE, 1, 1, 1)
    end,
    minimap = { store = db, key = "minimap" },     -- { hide: boolean, minimapPos: number }
    compartment = true,
    ldb = true,
})
```

| Surface | Availability | Implementation |
|---------|--------------|----------------|
| Addon Compartment | Clients with the Addon Compartment (Mainline family: `standard`, `camelot`), detected at runtime | Registered at `PLAYER_LOGIN` with `AddonCompartmentFrame:RegisterAddon({...})`: no global function and no TOC directive. `onClick` receives the mouse button name taken from the compartment's click data, and `tooltip` is called with `GameTooltip` anchored to the entry. The icon and label can change at runtime. |
| Minimap button | All flavors | At `PLAYER_LOGIN`: if LibDBIcon (`LibStub("LibDBIcon-1.0", true)`) is loaded, the launcher registers with it, so minimap button managers see it. Otherwise it creates a **built-in** draggable button named `<Addon>MinimapButton`, with shape-aware positioning. **Never both.** |
| LDB data object | All flavors | Registered with LibDataBroker if it is present at `PLAYER_LOGIN`, so display addons (Titan Panel, ChocolateBar, …) can show it. No hard dependency. |

- **Identity.** LDB and LibDBIcon use `addon.name` as the object name, never the
  title, which may contain color codes. The title is only shown as the label.
- **Position store.** `minimap.store[minimap.key]` uses LibDBIcon's
  table shape (`hide`, `minimapPos`), so switching between the built-in button and
  LibDBIcon keeps the position. Thanks to lazy deep defaults
  ([saved](core.md#saved)), writes persist.
- **Timing.** `register` may be called at top level. Surfaces are created at
  `PLAYER_LOGIN`, after the store is loaded.
- **Globals.** The built-in button's frame name `<Addon>MinimapButton` is a
  global (needed by minimap button managers). It is listed in the normative
  global-writes list ([Code generation §14.1](../toolchain/codegen.md#141-target)).
- **One registration.** `register` may be called once per addon (E8080). For
  top-level calls this is checked at compile time; otherwise a second call raises
  a runtime error.
- `launcher.setIcon`, `setLabel`, `showMinimap(bool)` and `refresh()` update
  the surfaces at runtime.
- **Look.** The built-in minimap button looks like the client's own minimap
  buttons by default. `theme` in `register`, or `launcher.setTheme(theme)`,
  restyles it with a `WidgetTheme`
  ([Interface: widget themes](interface.md#widget-themes)). A button drawn by
  LibDBIcon, the Addon Compartment entry and LDB displays are not themed.
