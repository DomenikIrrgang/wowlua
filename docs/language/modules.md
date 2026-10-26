# Modules

Each `.wowlua` file is a module: it exports names, and other files import
them. This page covers `export` and `import`, how import paths resolve, the
load order wowlua computes for the TOC, and how modules mix with plain Lua
files and the addon table.

## Share a function between files

Write `export` before a declaration to make it visible to other files:

```wowlua title=src/core/math.wowlua
export function clamp(v: number, lo: number, hi: number): number
    return math.max(lo, math.min(hi, v))
end

export function lerp(a: number, b: number, t: number): number
    return a + (b - a) * clamp(t, 0, 1)
end
```

Then import it by name where you need it:

```wowlua title=src/main.wowlua
import clamp from "/core/math"

print(clamp(5, 0, 1))
```

The imported `clamp` is a local of `src/main.wowlua`. It is copied out of the
module once, when the file loads, so a call costs the same as a call to any
local function. This is the Lua the two files compile to:

```lua
-- src/core/math.lua
local __wl = select(2, ...).__wowlua
local __wl_exports = __wl.module("core/math")
local clamp
clamp = function(v, lo, hi)
	return math.max(lo, math.min(hi, v))
end
__wl_exports.clamp = clamp

-- src/main.lua
local __wl = select(2, ...).__wowlua
local clamp = __wl.require("core/math").clamp

print(clamp(5, 0, 1))
```

Each module's export table is kept in the addon's private runtime, which
hangs off the addon table that WoW passes to every file. Nothing goes into
`_G`, so a module of another addon with the same path never clashes with
yours.

## What you can export

You can export functions, classes, abstract classes, enums, constants,
locals, type aliases and interfaces. Types and interfaces are only for the
checker and produce no code.

```wowlua title=src/registry.wowlua
export const VERSION = 3
export type Handler = (msg: string) -> ()
export local registry: { [string]: Handler } = {}

export function register(name: string, h: Handler)
    registry[name] = h
end

export class Cache
    items: { [string]: number } = {}
end

export enum Mode { Simple, Advanced }

export interface Disposable
    function dispose()
end
```

- Exported names are fixed. Assigning to an exported local after its
  declaration, or to an imported name, is error E1010. Change the contents
  of an exported table instead, or export a function that does the change.
- `export local` needs a value (E0119).
- Only declarations can be exported. There is no `export { a, b }` list,
  because `export {a}` is already a Lua call.
- All top-level functions, classes and enums of a module are declared at the
  top of the file, so two functions can call each other whatever their order
  in the source. Calling one at load time before its definition has run is
  error E1037.
- In a module, a top-level `function name()` is local to the module, exported
  or not, so you can't create a global by mistake. Write `_G.name = name`, or
  use [`@global`](decorators.md), when you want one. A file with no `import`
  and no `export` is a script instead and keeps Lua's meaning: its top-level
  functions are globals.
- A top-level `return` is an error in a module (E1032).
- Constants are copied into the files that import them. They stay in the
  export table too, for plain Lua code that reads it.

## Ways to import

```wowlua title=src/ui/window.wowlua
import clamp, lerp from "/core/math"          -- named
import Cache as ItemCache from "/registry"    -- renamed
import * as log from "wowlua:log"             -- the whole module
import type Handler from "../registry"        -- types only, no code

local onShow: Handler = function(msg: string)
    log.info(msg, clamp(lerp(0, 10, 0.5), 0, 8))
end

local cache = ItemCache.new()
cache.items.opened = 1
onShow("window opened")
```

- Imports come first in a file, before any other statement (E1011). Only
  directive comments such as `--!strict` may come before them.
- `import * as M` binds the module's export table. Its exported types are
  available as `M.TypeName` in annotations.
- `import type` brings in types only. It never affects the load order. A
  class imported this way can be used as a type but not extended (E3008).
- `import _ from "./frames.xml"` binds no names. It only makes sure that an
  XML file, a plain Lua file or a module run for its side effects loads
  before this file. `_` must be the only name in the statement (E0117).
- Importing a name the module doesn't export is error E1012.

> [!NOTE]
> wowlua has no braces in imports because `import { a } from "x"` is valid
> Lua: a call of `import` with a table, then a call of `from` with a string.
> `import a from "x"` is a syntax error in Lua, so it can't change the meaning
> of an existing file.

## Where imports look

| You write | It finds |
|-----------|----------|
| `"./row"`, `"../theme"` | A file relative to the importing file |
| `"/core/cache"` | `src/core/cache.wowlua`, from anywhere in the package (the source root is `build.src`, default `src/`) |
| `"wowlua:events"` | A module of the [standard library](../stdlib/overview.md) |
| `"libfoo"` | The main module of a dependency, `src/init.wowlua` unless its `package.main` says otherwise |
| `"libfoo/ui/list"` | Module `ui/list` inside the dependency `libfoo` |

Leave out the extension. wowlua tries, in order, `x.wowlua`, the declaration
file `x.d.wowlua`, `x.lua` and `x/init.wowlua`. A path that names its
extension, such as `"./frames.xml"`, must match exactly. A module that isn't
found is error E1015.

Paths are case-sensitive on every system. A path whose case doesn't match the
file is error E1013: WoW on Windows would load it anyway, but the same addon
would fail for players on macOS.

Dependencies are covered in
[Libraries and dependencies](../guide/packages.md).

## Load order, worked out for you

WoW runs an addon's files in the order its TOC lists them. wowlua writes the
TOC and computes that order. It follows your imports, and it also reads which
globals and addon-table fields each file defines and uses, in `.wowlua` files,
plain `.lua` files and the Lua inside XML files.

- Only what runs while a file loads counts. Code that runs later, in an event
  handler, a script, a timer or a slash command, doesn't constrain the order,
  because every file has loaded by then.
- Two modules may import each other, as long as neither needs the other while
  it loads.
- When files need each other while they load, the order can't work. That is
  error E1014, which names each use and where it happens.
- When the order can't be decided, for example because two files define the
  same global, you get warning W1033. An `import`, or a `build.load-order`
  entry in `wowlua.json`, settles it.
- Every file under `src/` is part of the addon, imported or not. A
  release build leaves out a module that nothing imports and that only
  declares things, with warning W6002. Test files (`*.test.wowlua`) only run
  in `wowlua test` and are never shipped.

Here `theme` reads `Window` and `window` reads `width`, both at the top
level:

```text
error[E1014]: load-time dependency cycle between ui/theme.wowlua, ui/window.wowlua
  ┌─ src/ui/theme.wowlua:4:22
  │
4 │ export local width = Window.defaultWidth() + PADDING
  │                      ^^^^^^ `ui/theme.wowlua` uses the import `Window` from `ui/window.wowlua` while it loads
  │
  ┌─ src/ui/window.wowlua:9:27
  │
9 │ export local frameWidth = width * 2
  │                           ----- `ui/window.wowlua` uses the import `width` from `ui/theme.wowlua` while it loads
  │
  = note: ui/theme.wowlua: in top-level code
  = note: ui/window.wowlua: in top-level code
  = help: move the use into a function that runs later (an event handler, for example), or split a module

error: aborting due to 1 previous error
```

When you [bring an existing addon](../guide/adopting.md) with a TOC that
works, `wowlua init` writes its file order into `build.load-order`, and the
build keeps it. The rules are in
[Modules §5.4](../../spec/language/05-modules.md#54-load-order).

## Living alongside plain Lua

You don't have to convert an addon in one go. Plain `.lua` files keep working
as they are: they still get `addonName, ns` from `...`, and the load order
accounts for what they define. `.wowlua` files can use the addon table too,
with the usual `local addonName, ns = ...` at the top.

### Give an old Lua file types

Have the Lua file register a module, and describe that module in a
declaration file with the same name next to it:

```lua title=src/legacy/util.lua
local _, ns = ...
ns.__wowlua.define("legacy/util", {
    round = function(n, d)
        local m = 10 ^ (d or 0)
        return math.floor(n * m + 0.5) / m
    end,
})
```

```wowlua title=src/legacy/util.d.wowlua
declare module "/legacy/util"
    function round(n: number, decimals: number?): number
end
```

Importing a name from it gives you the declared types, and makes the Lua file
load first:

```wowlua title=src/main.wowlua
import round from "/legacy/util"

print(round(3.14159, 2))
```

If the Lua file writes globals or fields of `ns` instead, describe those in a
`.d.wowlua` file with `declare global` or `declare interface AddonNamespace`.
See [Types](types.md).

### Use a module from plain Lua

A plain Lua file of the addon reads a module's exports with
`select(2, ...).__wowlua.require("core/math")`. The key `__wowlua` of the
addon table belongs to wowlua's runtime; don't use it for anything else.

## Sharing with other addons

Modules are private to the addon they are built into. Two addons never share
a module, even in the same workspace. To offer an API to other addons, put
`@global` on what they should see:

```wowlua title=src/api.wowlua
local scores: { [string]: number } = {}

@global("MyAddonAPI")
export const api = {
    getScore = function(name: string): number
        return scores[name] or 0
    end,
}
```

```lua
local api = {getScore = function(name)
	return scores[name] or 0
end}
_G.MyAddonAPI = api
__wl_exports.api = api
```

Another addon in the same workspace that lists yours in
`addon-dependencies` is checked against your sources, so it sees
`MyAddonAPI` with its types. For callbacks between addons, see
[Talking to other addons](../stdlib/comm.md).

## Where to go next

- [Build info and profiles](flavors.md): imports and code that only exist in
  some builds.
- [Libraries and dependencies](../guide/packages.md): import from other
  packages.
- [Bring your addon](../guide/adopting.md): move an existing addon over, file
  by file.
