# x1xxx · names

## E1001

In strict mode, the code reads a global that nothing declares: no WoW API,
Lua library, `declare global`, SavedVariable from the manifest,
`check.globals` entry, or global that a file of the package assigns. Most
often it is a typo or a missing `local`; in game the read gives `nil` and
fails later with "attempt to call/index a nil value". Fix the name, make it
a local or an import, or declare the global (`declare global name: T`).

Strict mode comes from `--!strict` or the manifest (`check.mode`,
`check.strict`). In basic mode, the default, the same read is the warning
W1001. An existence test (`if SomeApi then … end`) never reports the tested
name.

```wowlua fails
--!strict
local playerName = UnitName("player")
print(playerNmae)
```

```wowlua compiles
--!strict
local playerName = UnitName("player")
print(playerName)
```

Related: W1001, E1002

## W1001

The code reads a global that nothing declares (see E1001 for what counts as
declared). It is a warning in basic mode, the default, and in plain Lua
files; strict mode makes it the error E1001. Usually it is a typo or a
missing `local`, and the value is `nil` in game. Fix the name, or declare a
global that comes from somewhere the checker can't see: `declare global`, or
the manifest's `check.globals` for another addon's globals.

```wowlua fails
local playerName = UnitName("player")
print(playerNmae)
```

```wowlua compiles
local playerName = UnitName("player")
print(playerName)
```

Related: E1001, W1002

## E1002

In strict mode, the code assigns a global that isn't declared, including
through `function name()` in a script. Accidental globals are a classic addon
bug: they leak into every addon's environment, can collide with other addons'
names and can taint Blizzard code. Make it `local` if only this file needs
it, export it from a module, or declare a real global (`declare global
name: T`). Writing `_G.name = value` is always allowed.

In basic mode the same assignment is the warning W1002.

```wowlua fails
--!strict
counter = 0
```

```wowlua compiles
--!strict
declare global MyAddonCounter: number
MyAddonCounter = 0
```

Related: W1002, E1001

## W1002

The code assigns a global that isn't declared (see E1002). It is a warning in
basic mode, the default, and in plain Lua files; strict mode makes it the
error E1002. If the global is meant to exist (a SavedVariable, an API for
other addons), declare it or write `_G.name = value`; otherwise make it
`local`.

```wowlua fails
counter = 0
```

```wowlua compiles
local counter = 0
print(counter)
```

Related: E1002, W1001

## E1003

A name used in a wowlua construct, such as a decorator argument, is not a
local or upvalue in scope there. Declare the local before the construct, or
fix the name.

The current compiler does not report this code: it is registered for this
rule, but no check produces it yet, so there are no examples.

Related: E1001

## W1003

A local, parameter, loop variable or import is never read. It is often a
leftover of a refactoring, or a sign that the wrong name is used somewhere.
Remove it, or start its name with `_` (`_event`) to keep it on purpose, as
for a callback parameter you must accept but don't need. Like every lint, its
level can be changed in the manifest's `check.lints` (`"W1003": "allow"`).

```wowlua fails
local function greet(name: string, title: string)
    print("Hello " .. name)
end
greet("Ana", "Dr.")
```

```wowlua compiles
local function greet(name: string, _title: string)
    print("Hello " .. name)
end
greet("Ana", "Dr.")
```

## E1004

A parameter's default value reads that parameter or a later one. Every
parameter is in scope in a default, as in the Lua it compiles to, so the
name means the parameter itself, which isn't set yet: it is `nil` there,
even when an outer local has the same name. A default may read earlier
parameters only. Reorder the parameters, or rename the parameter to read an
outer local of that name.

```wowlua fails
local function resize(width: number = height, height: number = 100)
    print(width, height)
end
resize()
```

```wowlua compiles
local function resize(height: number = 100, width: number = height)
    print(width, height)
end
resize()
```

Related: W1004

## W1004

A local hides another one: it has the name of a local of an enclosing scope,
or a `local function` is named like a local declared before it in the same
block (from there on, the earlier one is out of reach, even inside the new
function). A parameter named `self` in a class's method or constructor also
gets this warning, because it hides the instance. Hidden names are a common
source of "it reads the wrong value" bugs; rename one of them. Names starting
with `_` are exempt.

```wowlua fails
local count = 1
do
    local count = 2
    print(count)
end
print(count)
```

```wowlua compiles
local count = 1
do
    local bonus = 2
    print(bonus)
end
print(count)
```

Related: E1004

## W1005

Code follows a statement that execution never gets past: a `do return end`
(or `break`, `continue`), an `if` whose every branch returns or breaks, or a
call that never returns, such as `error(…)`. The code after it can never run.
Remove it, or move it before the statement that leaves.

```wowlua fails
local function sign(x: number): number
    if x >= 0 then return 1 else return -1 end
    print("never printed")
end
print(sign(2))
```

```wowlua compiles
local function sign(x: number): number
    print("checking", x)
    if x >= 0 then return 1 else return -1 end
end
print(sign(2))
```

## E1010

The code assigns to an exported `local` after its declaration, or to an
imported name. Importers copy an export once while the file loads, so a
later assignment would never reach them; exported and imported bindings are
therefore read-only. Change a table's contents instead (`state.count += 1`),
or export functions that read and change the value.

```wowlua fails
export local count = 0
count = count + 1
```

```wowlua compiles
export local state = { count = 0 }
state.count = state.count + 1
```

Related: E2010, E0119

## E1011

A top-level `import` comes after another statement. Imports must come first
in a module, before any other statement (comments and `--!` directives may
precede them). Move every import to the top of the file.

```wowlua fails
local delay = 1
import after from "wowlua:timer"
after(delay, function() print("later") end)
```

```wowlua compiles
import after from "wowlua:timer"
local delay = 1
after(delay, function() print("later") end)
```

Related: E0108

## E1012

An import names something the module doesn't export: a typo, a name that
was renamed or removed, or a function the module defines without `export`.
Check the module's `export` declarations and fix the name, or export it
there.

```wowlua fails
import sleepFor from "wowlua:timer"
sleepFor(1)
```

```wowlua compiles
import after from "wowlua:timer"
after(1, function() print("later") end)
```

Related: E1015

## E1013

An import's path differs in case from the file on disk (`"./Math"` for
`math.wowlua`). Module paths are case-sensitive everywhere: WoW on Windows
would tolerate the mismatch, other systems would not find the file. Write
the path exactly as the file is named.

```wowlua fails title=src/main.wowlua
import clamp from "./Math"
print(clamp(5, 0, 1))
```

```wowlua fails title=src/math.wowlua
export function clamp(v: number, lo: number, hi: number): number
    return math.max(lo, math.min(hi, v))
end
```

```wowlua compiles title=src/main.wowlua
import clamp from "./math"
print(clamp(5, 0, 1))
```

```wowlua compiles title=src/math.wowlua
export function clamp(v: number, lo: number, hi: number): number
    return math.max(lo, math.min(hi, v))
end
```

Related: E1015, E1038

## E1014

Files depend on each other in a cycle, and at least one dependency is needed
while a file loads. WoW loads files one after another, so one of them would
run before the other has defined what it uses. Modules may import each other
as long as the uses only happen later (inside functions called from events,
timers or handlers); a top-level call is what makes a use load-time. The
message shows the cycle and the calls that lead to each load-time use. Move
the use into a function that runs later, or move the shared code into a
third module.

```wowlua fails title=src/main.wowlua
import greet from "./other"
export function hello(): string
    return "hi"
end
print(greet())
```

```wowlua fails title=src/other.wowlua
import hello from "./main"
export function greet(): string
    return hello() .. "!"
end
print(greet())
```

```wowlua compiles title=src/main.wowlua
import greet from "./other"
export function hello(): string
    return "hi"
end
print(greet())
```

```wowlua compiles title=src/other.wowlua
import hello from "./main"
export function greet(): string
    return hello() .. "!"
end
```

Related: W1034, E1037

## E1015

The module an import names can't be found: no file at that path (tried with
`.wowlua`, `.d.wowlua`, `.lua` and `/init.wowlua`), an unknown
standard-library module (`wowlua:…`), a package that isn't in the
manifest's `dependencies`, or a relative path that leaves the source root.
Check the path (`./` and `../` are relative to the importing file, `/` to
`src/`) and the dependency list.

```wowlua fails
import clamp from "./math"
print(clamp(5, 0, 1))
```

```wowlua compiles title=src/main.wowlua
import clamp from "./math"
print(clamp(5, 0, 1))
```

```wowlua compiles title=src/math.wowlua
export function clamp(v: number, lo: number, hi: number): number
    return math.max(lo, math.min(hi, v))
end
```

Related: E1013, E1012

## E1016

An export has the name of another top-level declaration of the module (a
`local`, function, `const`, class, enum or import), or a name is exported
twice. An export is itself a module-level local, so the two would shadow
each other. Rename one of them, or put `export` on the original declaration.
Declarations that `@if` removes from the build don't count, so alternatives
for different flavors may share a name.

```wowlua fails
local items = {}
export function items()
    return {}
end
```

```wowlua compiles
local itemList = {}
export function items()
    return itemList
end
```

Related: E0118

## E1020

Code that is kept in this build uses a declaration that `@if` (or static
elimination) removed from it, for example a flavor-only function called
without a check. In game the name would be `nil`. Guard the use with the
same static condition (`if flavor == "standard" then … end`, which is
removed with it), or remove the `@if`.

```wowlua fails
import flavor from "wowlua:meta"

@if(flavor == "standard")
local function setupEditMode()
    print("edit mode")
end

setupEditMode()
```

```wowlua compiles
import flavor from "wowlua:meta"

@if(flavor == "standard")
local function setupEditMode()
    print("edit mode")
end

if flavor == "standard" then
    setupEditMode()
end
```

Related: E1040, E1039

## E1031

Two interfaces with the same name are declared in the same scope. Only
ambient `declare interface` declarations merge (as for the addon namespace);
give the other interface its own name, or merge the two into one.

The current compiler does not report this code: the later interface replaces
the earlier one without a diagnostic, so there are no examples.

## E1032

A module (a file with an `import` or `export`) has a `return` at its top
level. Exports after it would never be assigned, and WoW ignores a file's
return value anyway. Remove the `return`; to share values, export them. A
script (a file without imports and exports) may still `return`, as in Lua.

```wowlua fails
export const VERSION = 1
return VERSION
```

```wowlua compiles
export const VERSION = 1
```

## W1033

wowlua can't tell in which order some files must load, so they keep their
source order (their place in `build.load-order`, then their path). The
warning names the globals or fields that cause it: several files define the
same name while loading, a name is written through a dynamic key
(`_G["x" .. i]`, `ns[key]`), or names come from outside the package. Give the
name a single defining file, or fix the order with an import (`import _
from "./file"`) or `build.load-order`.

```lua fails title=src/a.lua
MyAddonDB = {}
```

```lua fails title=src/b.lua
MyAddonDB = { version = 2 }
```

```lua fails title=src/c.lua
print(MyAddonDB.version)
```

```lua compiles title=src/a.lua
MyAddonDB = {}
```

```lua compiles title=src/b.lua
MyAddonDB.version = 2
```

```lua compiles title=src/c.lua
print(MyAddonDB.version)
```

Related: W1034, W1035

## W1034

A file reads a global (or an addon-namespace field) while it loads, but the
only definitions of that name run later, inside functions such as event
handlers. When the file loads in game the value is still `nil`. Define the
name at the top level of its file, or move the read into code that runs
after the definition (the same event, or a later one).

```lua fails title=src/a.lua
local frame = CreateFrame("Frame")
frame:RegisterEvent("PLAYER_LOGIN")
frame:SetScript("OnEvent", function()
    MyAddonDB = {}
end)
```

```lua fails title=src/b.lua
print(MyAddonDB)
```

```lua compiles title=src/a.lua
local frame = CreateFrame("Frame")
frame:RegisterEvent("PLAYER_LOGIN")
frame:SetScript("OnEvent", function()
    MyAddonDB = {}
end)
```

```lua compiles title=src/b.lua
local frame = CreateFrame("Frame")
frame:RegisterEvent("PLAYER_LOGIN")
frame:SetScript("OnEvent", function()
    print(MyAddonDB)
end)
```

Related: W1033, E1014

## E1035

`build.load-order` lists a file before a file it imports. An import always
loads the imported module first, so the listed order can't be kept. Reorder
the list, or remove the files from it and let wowlua compute the order.
(A contradiction with a dependency found only by usage analysis is the
warning W1035.)

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "build": { "load-order": ["src/main.wowlua", "src/util.wowlua"] }
}
```

```wowlua fails title=src/main.wowlua
import greet from "./util"
print(greet())
```

```wowlua fails title=src/util.wowlua
export function greet(): string
    return "hi"
end
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "build": { "load-order": ["src/util.wowlua", "src/main.wowlua"] }
}
```

```wowlua compiles title=src/main.wowlua
import greet from "./util"
print(greet())
```

```wowlua compiles title=src/util.wowlua
export function greet(): string
    return "hi"
end
```

Related: W1035, E1014

## W1035

`build.load-order` lists a file before one whose definitions it uses while
loading, as found by usage analysis (globals, addon-namespace fields). The
listed order wins, because the analysis can over-approximate without types
and `wowlua init` writes the order of a TOC that worked. If the warning is
right, reorder the list; if the use really happens later, the warning can be
silenced in `check.lints`.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "build": { "load-order": ["src/main.lua", "src/util.lua"] }
}
```

```lua fails title=src/main.lua
print(MyAddonGreeting)
```

```lua fails title=src/util.lua
MyAddonGreeting = "hi"
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "build": { "load-order": ["src/util.lua", "src/main.lua"] }
}
```

```lua compiles title=src/main.lua
print(MyAddonGreeting)
```

```lua compiles title=src/util.lua
MyAddonGreeting = "hi"
```

Related: E1035, W1033

## E1036

The package's flavors need their files in incompatible orders (different
`build.flavor-load-order` lists, or `@if` imports that point different
ways), and `build.toc-layout` is `"single"`, which forces one TOC for all
flavors. With `"auto"`, the default, wowlua writes per-flavor TOC files in
this case. Use `"auto"`, or make the orders agree.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {}, "standard": {} },
    "build": {
        "toc-layout": "single",
        "flavor-load-order": {
            "camelot": ["src/a.lua", "src/b.lua"],
            "standard": ["src/b.lua", "src/a.lua"]
        }
    }
}
```

```lua fails title=src/a.lua
print("a")
```

```lua fails title=src/b.lua
print("b")
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {}, "standard": {} },
    "build": {
        "toc-layout": "auto",
        "flavor-load-order": {
            "camelot": ["src/a.lua", "src/b.lua"],
            "standard": ["src/b.lua", "src/a.lua"]
        }
    }
}
```

```lua compiles title=src/a.lua
print("a")
```

```lua compiles title=src/b.lua
print("b")
```

Related: E1035

## W1036

Two modules import each other (a cycle that only runs later, which is
allowed), and the one checked first reads an export of the other that has
no type annotation. The modules of a cycle are checked in source order, so
that export's type isn't known yet and reads as `any` there: mistakes in its
use go unnoticed. Annotate the export in its module (`export local items:
string[] = {}`). The same applies to a `typeof` type alias of the later
module, and to file-level locals read through `import internal` in tests.

```wowlua fails title=src/main.wowlua
import items from "./other"
export function show()
    print(#items)
end
```

```wowlua fails title=src/other.wowlua
import show from "./main"
export local items = {}
export function add(name: string)
    table.insert(items, name)
    show()
end
```

```wowlua compiles title=src/main.wowlua
import items from "./other"
export function show()
    print(#items)
end
```

```wowlua compiles title=src/other.wowlua
import show from "./main"
export local items: string[] = {}
export function add(name: string)
    table.insert(items, name)
    show()
end
```

Related: E1014

## E1037

A top-level function, class or enum of a module is used while the file
loads, before the line that defines it has run. They are hoisted, so other
functions can refer to them in any order, but the value is still `nil` until
its definition executes. Move the definition above the load-time use, or
move the use into code that runs later.

```wowlua fails
export const VERSION = 1
print(describe())
function describe(): string
    return "v" .. VERSION
end
```

```wowlua compiles
export const VERSION = 1
function describe(): string
    return "v" .. VERSION
end
print(describe())
```

Related: E1014, W1034

## E1038

`asset(path)` names a file that isn't one of the package's assets, or
differs from it in case, or the path isn't a constant string. The path is
relative to the package root, uses `/`, and must match a file that
`build.assets` includes, letter case and all. Add the file to
`build.assets`, or fix the path.

```wowlua fails
import asset from "wowlua:meta"
local icon = asset("media/icon.tga")
print(icon)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "build": { "assets": ["media/**"] }
}
```

```text compiles title=media/icon.tga
(the image file)
```

```wowlua compiles title=src/main.wowlua
import asset from "wowlua:meta"
local icon = asset("media/icon.tga")
print(icon)
```

Related: E1013

## E1039

`@if(flavor …)` is used in a `.wowlua` file in shared flavor mode
(`build.flavor-mode = "shared"`). There, one build runs on every flavor, so
`flavor` is a value known only in game and can't remove declarations at
compile time. Test the flavor at run time instead (`if flavor == "standard"
then … end`), or switch to split mode, where each flavor gets its own build.
In declaration files `@if(flavor …)` keeps its meaning.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {}, "standard": {} },
    "build": { "flavor-mode": "shared" }
}
```

```wowlua fails title=src/main.wowlua
import flavor from "wowlua:meta"

@if(flavor == "standard")
local function setupEditMode()
    print("edit mode")
end

setupEditMode()
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {}, "standard": {} },
    "build": { "flavor-mode": "shared" }
}
```

```wowlua compiles title=src/main.wowlua
import flavor from "wowlua:meta"

local function setupEditMode()
    print("edit mode")
end

if flavor == "standard" then
    setupEditMode()
end
```

Related: E1040, E1020

## E1040

An `@if` condition isn't static: it reads a runtime value such as a `local`,
or `@if` is given more than one condition. Conditions may use `wowlua:meta`
paths (`flavor`, `build.debug`, `features.x`, `config.x`), constants and
literals. On an `import`, only `wowlua:meta` paths and literals are allowed,
not `const`s, because each flavor's import graph is built before constants
are evaluated. For a runtime decision, use an ordinary `if`.

```wowlua fails
local verbose = true

@if(verbose)
local function dumpState()
    print("state")
end
```

```wowlua compiles
import build from "wowlua:meta"

@if(build.debug)
local function dumpState()
    print("state")
end

if build.debug then
    dumpState()
end
```

Related: E1039, E1020

## E1041

A module imports a value from a module of another addon of the workspace
(one listed in `addon-dependencies`). Modules are private to the addon they
are compiled into, so another addon's module isn't part of this one. Use
`import type` for its types, and its `@global` API for its values.

It needs a workspace with two addons, so it has no example here; `wowlua
check` reports it in the workspace.

Related: E1015
