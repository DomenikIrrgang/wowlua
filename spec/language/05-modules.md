# 5. Modules

Every `.wowlua` file is a **module**. Modules replace the classic WoW
pattern of sharing state through globals or the addon table
(`local _, ns = ...`). That pattern keeps working, and the two can be mixed.

## 5.1 Exports

`export` in front of a declaration makes it visible to importers:

```wowlua
export const VERSION = 3
export type Handler = (msg: string) -> ()
export local registry: { [string]: Handler } = {}
export function register(name: string, h: Handler) registry[name] = h end
export class Cache … end
export abstract class Base … end
export enum Mode { Simple, Advanced }
export interface Disposable
    function dispose()
end
```

Rules:

- `export function name()` declares a **module-local** function and exports it.
  It does not create a global, unlike a bare `function name()`. Only a plain name
  may follow `export function` (not `t.f` or `t:m`, E0118).
- **Hoisting** (decision L24). All top-level exported functions, classes and
  enums are pre-declared as locals at the top of the module, so they can refer
  to each other regardless of source order:
  ```wowlua
  export function isEven(n: number): boolean return n == 0 or isOdd(n - 1) end
  export function isOdd(n: number): boolean return n ~= 0 and isEven(n - 1) end
  ```
  ```lua
  local isEven, isOdd                       -- hoisted, on line 1 with the prologue
  isEven = function(n) return n == 0 or isOdd(n - 1) end; __wl_exports.isEven = isEven
  isOdd = function(n) return n ~= 0 and isEven(n - 1) end; __wl_exports.isOdd = isOdd
  ```
  Calling a hoisted function at load time before its definition has executed
  is error E1037 (the value would still be `nil`).
- `export local` requires an initializer (E0119).
- Exported bindings are **immutable**. Reassigning an exported `local` after its
  declaration is an error (E1010). Mutate a table's contents instead, or export
  accessor functions. Assigning to an imported binding is also E1010.
- The export table receives the **final** value of a decorated declaration, after
  all decorators ([Decorators §4.5.2](04-decorators.md#452-evaluation-order)).
- Only declarations can be exported. There is no `export { a, b }` list form,
  because `export {a}` is a valid Lua call.
- `export type` and `export interface` are type-only exports and produce no code.
- A module with no `export` and no `import` is a **script**: it behaves exactly
  like a Lua file, and its top-level `function f()` declarations are globals.
- A top-level `return` statement is an error in a module (E1032), because exports
  after it would never be assigned. It is allowed in scripts, as in Lua (the
  value is ignored by WoW).

## 5.2 Imports

```wowlua
import clamp, lerp from "./math"            -- named imports
import Cache as ItemCache from "/core/cache" -- renamed
import * as Log from "wowlua:log"           -- namespace import: Log.info(…)
import log from "wowlua:log"                -- named import: log.info(…)
import type Handler from "./types"          -- type-only (erased)
import _ from "./frames.xml"                -- load-order only (no bindings)
```

| Form | Meaning |
|------|---------|
| `import a, b as c from "spec"` | Binds exported `a` and `b` (as `c`) as immutable locals. |
| `import * as M from "spec"` | Binds the whole module's export table. Exported types are reachable as `M.TypeName` in type positions. |
| `import type T, U from "spec"` | Imports types only, with no runtime effect. Never creates a load-order dependency. |
| `import _ from "spec"` | Ensures `spec` loads before this file. Used for XML, plain Lua files and side-effect modules. `_` must be the only name in the statement (E0117). |

**`import type` disambiguation.** `import type` followed by a name other than
`from` starts a type-only import. `import type from "x"` imports a value export
named `type`.

Imports must appear at the top level of a module, before any other statement
except directive comments and other imports (E1011). Importing a name the
module does not export is error E1012. A module that cannot be found is error
E1015.

### 5.2.1 Why this syntax

`import { a } from "x"` is **valid Lua**: a call `import{a}` followed by a call
`from "x"`. wowlua therefore uses `import a from "x"`, where `import` followed
by a name is a syntax error in Lua. `import "x"` is also a valid Lua call, which
is why side-effect imports use `import _ from "x"`, following Go's convention.

## 5.3 Module specifiers

| Specifier | Resolves to |
|-----------|-------------|
| `"./x"`, `"../x"` | Relative to the importing file |
| `"/x/y"` | Relative to the package's source root (`build.src`, default `src/`) |
| `"pkg"` | The main module of dependency `pkg` (`package.main`, default `src/init.wowlua`) |
| `"pkg/x/y"` | Module `x/y` inside dependency `pkg`, relative to its `build.src` |
| `"wowlua:name"` | Standard-library module, e.g. `wowlua:events` |

Extension resolution, in order:

1. an exact match if an extension is given (`.xml`, `.lua`);
2. `x.wowlua`;
3. `x.d.wowlua` (types). If a sibling `x.lua` exists, the pair forms one module:
   types come from the declaration file, and a **value import also creates a
   load-order dependency on `x.lua`**;
4. `x.lua` (side-effect import only, unless typed through `declare module`);
5. `x/init.wowlua`.

Specifiers are case-sensitive on every platform, and a mismatch in case is
error E1013, because WoW on Windows would tolerate it but other platforms
would not.

**Module IDs** are canonical paths without extension relative to the source
root (`core/cache`), prefixed by the package name for dependencies
(`libfoo/core/cache`).

Inside a wowlua library, `"/x"` is relative to the library's own source root,
and relative specifiers may not leave it (E1015).

## 5.4 Load order

WoW loads the files listed in the TOC in order, synchronously. The compiler
computes the order **automatically** and **per flavor**, after `@if` and
static conditions have been applied (decisions L20, L21, T26).

### 5.4.1 Overall order

1. the wowlua runtime and metadata (if used),
2. dependencies, in dependency order (plain Lua libraries in their declared
   file order; wowlua libraries and standard-library modules by the same
   analysis as below),
3. all files of the package, in the order computed by §5.4.2–§5.4.5.

Every file under `src/` belongs to the addon unless excluded, except test files
(`*.test.wowlua`), which are only compiled by `wowlua test`. Files that nothing
imports are included too. Compiled locale files load before all of this
([Code generation §14.9.2](../toolchain/codegen.md#1492-file-order)).

### 5.4.2 Dependency edges

An edge `A → B` means "A must be loaded before B". Edges come from:

| Source | Edge |
|--------|------|
| Value import in `B` of module `A` | `A → B` |
| `import _ from "A"` in `B` | `A → B` (always load-time) |
| **Usage analysis**: `B` reads a global `G`, an addon-namespace field `ns.F`, or a field below one of them (`G.f`, `ns.F.f`) that `A` defines | `A → B` |
| XML: `B.xml` references a global defined by `A` (`mixin="X"`, script handler attributes, inline `<OnLoad>` code) or a template defined in `A.xml` (`inherits="T"`) | `A → B` (load-time: frames are created while the XML loads) |
| `build.load-order` | consecutive listed files, in the listed order |

**Usage analysis** covers `.wowlua`, `.lua` and the Lua inside XML. A
*definition* is an assignment to a global (`G = …`, `function G()`,
`_G.G = …`, `_G["G"] = …` with a constant key, `@global`) or to a field of the
addon namespace (`ns.F = …`, where `ns` is bound from the file's `...`, through
any local alias) with a constant key. A *use* is a read of the same name.

**Fields of tables** (decision T182). A path below a global or `ns` key
(`Lib.Register`, `ns.Tools.Run`, `ns.UI.Icon`) is a name of its own:

- It is *defined* by `function Lib.Register()`, `function Lib:Register()`,
  `Lib.Register = …` (also `Lib["Register"] = …`), a named field of a table
  constructor assigned to the parent (`Lib = { Register = … }`), and the same
  forms through a local alias (`local API = Lib`, `local UI = ns.UI`).
  `Lib.n += 1` reads `Lib.n` and does not define it.
- It is *used* by every read of the path, including calls and method calls
  (`Lib.Register(…)`, `Lib:Register(…)`). Reading `Lib.Register` also reads
  `Lib`. Guards work per path: `if Lib.Maybe then Lib.Maybe() end` does not
  order files.
- **Table identity:** a global or `ns` key assigned another one
  (`ns.API = SuuUI`, `Alias = Lib`) holds the same table, so their fields are
  the same names: `API.RegisterModule = …` (with `local API = ns.API`) defines
  `SuuUI.RegisterModule`. A local initialized with a table constructor
  (`local Roles = {}`) is a table too: after `ns.Roles = Roles`,
  `function Roles.Current()` defines `ns.Roles.Current`, wherever in the file
  it stands.
- Only fields of tables that the package defines take part; reads of the
  game's tables (`C_Timer.After`) order nothing.
- Fields get the same diagnostics as globals: several load-time definers are
  W1033, a field read at load time but only defined at run time is W1034.
- **Untraceable writes:** a write into a table the analysis cannot follow
  (a parameter, a call result, a local not initialized with a table
  constructor: `get().f = …`) defines nothing.
  If a field of a package table is read at load time, no file defines it
  traceably, and another file writes a field of that name through such a table
  at load time, that is W1033. Otherwise the read stays silent.

### 5.4.3 Load-time and run-time edges

Each edge is classified by **when** the use happens:

- **Load-time:** the use executes while the file loads. That covers top-level
  statements, and any function called from load-time code. Calls are followed
  transitively through local functions, module functions, methods and
  constructors, **across modules**, using per-function summaries of what each
  function reads and writes.
- **Run-time:** the use happens only inside functions that are invoked later.
  These are:
  - functions passed to callback parameters marked `@deferred` in declaration
    files (`SetScript`, `HookScript`, `C_Timer.After`, `C_Timer.NewTicker`,
    `hooksecurefunc`, `RegisterCallback`-style APIs; the `wow-api` types mark
    them);
  - functions registered through registering decorators (`@event`, `@slash`,
    `@onLoad`, `@onLogin`, `@hook`, …);
  - functions only stored (in tables, fields, upvalues) and never called at load
    time.
- A function passed to a **package function** counts as called only if that
  function calls the parameter, or passes it on to a callee that does
  (per-function parameter summaries). Passed to an **unknown** callee, or to a
  parameter not marked `@deferred`, it counts as **called at load time**.
  `pcall`, `xpcall`, `securecallfunction` and `table.sort` comparators run
  immediately anyway. Wrongly assuming load-time at worst reports a cycle that
  isn't real. Wrongly assuming run-time would produce an addon that breaks in
  game. The deferred APIs are a curated list in the flavor table (until the
  `wow-api` declarations mark them); `X.Name(…)` matches a deferred method
  name like `X:Name(…)`.
- **Guarded uses** do not order files (decision T70): a name tested for
  existence (`if X then`, `X and …`, `X or …`, `not X`, `X ~= nil`,
  `type(X) == "table"`), and the uses that test guards (the `then` branch, the
  right operand of `and`). The left operand of `??` and the object of an
  optional link (`X?.f`, `X?.[k]`, `X?:m()`, `X?.()`) are existence tests too,
  and an optional link guards the rest of its chain. If `X` is not defined yet, the guarded code is
  skipped, so the file still loads.
- **XML**: creating an element reads its `mixin`, `secureMixin`, `parent` and
  `relativeTo` globals and its handlers' `function` attributes, runs its
  `<OnLoad>` code and instantiates the templates it `inherits`. That happens
  while the XML file loads, except inside templates (`virtual` or
  `intrinsic`), whose needs apply where they are instantiated. Named elements
  outside templates define globals; templates define template names. Inline
  `<Script>` code runs while the file loads; other handlers run later.

Definitions are classified the same way. If a name is **read at load time** but
only **defined at run time**, that is warning W1034. A **load-on-demand** addon
(`addon.load-on-demand` with required `addon-dependencies`) loads after its
required addons have loaded and run, so a field of a table one of them owns (a
global the addon never defines and that is not one of its SavedVariables, such
as `DBM.Options` in an addon that only writes `DBM.Options` at run time) may be
set by its owner: such a read is not W1034 (decision T206). Its own
SavedVariables are set after its files load, so reading them still is.

### 5.4.4 Ordering and cycles

The order is a **topological sort over the load-time edges**. Run-time edges
do not constrain the order, because every file has loaded before any deferred
function runs.

Cycles are found over **all** edges:

| Cycle contains | Result |
|----------------|--------|
| only run-time edges | **Allowed.** Modules may import each other. Within the cycle, files keep their source order (§5.4.5). An imported binding whose module loads **after** the importer is lowered to a **lazy access** through the module table (§5.5). |
| a load-time edge | **Error E1014.** The message shows the complete cycle and, for each load-time edge, the chain of calls from top-level code down to the use, with a suggested fix (move the use into a function that runs later, or split the module). |

Type-only imports never create edges.

### 5.4.5 Ties, ambiguity and overrides

- **Source order** breaks ties: the position in `build.load-order` for listed
  files (which `wowlua init` writes from an existing TOC), then the source path
  compared byte-wise for all other files. The result is deterministic.
- **Ambiguity** (decision L21). When the analysis cannot decide an order, it
  reports warning W1033 naming the affected globals or fields. The affected
  files keep their source order relative to each other. Examples: dynamic names
  (`_G["x" .. i]`, `ns[key]`), several files defining the same name at load
  time, or names that other addons provide. An import or a `build.load-order`
  entry resolves it.
- **`build.load-order`** adds load-time edges between consecutive listed files.
  If a listed order contradicts an **import**, that is error E1035. If it
  contradicts an edge found by **usage analysis**, the listed order wins with
  warning W1035 (decision T70): without types the analysis over-approximates,
  and `wowlua init` writes the order of a TOC that works. Edges between an XML
  file and the files it loads are ignored; they load interleaved in document
  order.
- **Fixed order** (decision T205). Files whose relative order the layout fixes
  get no W1033: two files listed in `build.load-order` (directly, or loaded by
  an XML file that is), two files one XML file loads (in document order, like
  a default locale and the locale files that override it), and an XML file
  and a file it loads.

### 5.4.6 Several flavors

In `split` flavor mode, the graph and order are computed separately for each
flavor. Modules removed by `@if` for a flavor are not loaded in that flavor.
For a single TOC, the file order is a topological sort over the **union** of
all flavors' load-time edges. If the union contains a cycle (two flavors need
incompatible orders), `toc-layout = "auto"` falls back to per-flavor TOC files,
and `toc-layout = "single"` reports error E1036.

## 5.5 Runtime representation

Each module that exports values gets an **export table** registered in the
addon's private runtime (never a global). Importers copy exports into locals
once, at load time. After that, using an import costs the same as a local access.

```wowlua
-- src/core/math.wowlua
export function clamp(v: number, lo: number, hi: number): number
    return math.max(lo, math.min(hi, v))
end

-- src/main.wowlua
import clamp from "/core/math"
print(clamp(5, 0, 1))
```
```lua
-- src/core/math.lua
local __wl = select(2, ...).__wowlua; local __wl_exports = __wl.module("core/math"); local clamp
clamp = function(v, lo, hi)
    return math.max(lo, math.min(hi, v))
end; __wl_exports.clamp = clamp

-- src/main.lua
local __wl = select(2, ...).__wowlua; local clamp = __wl.require("core/math").clamp
print(clamp(5, 0, 1))
```

- `__wl.module(id)` creates and registers the export table.
- `__wl.require(id)` returns it. In debug builds it raises a clear error if the
  module has not loaded yet. That cannot happen with compiler-generated TOCs, but
  can with hand-edited ones.
- Constant exports (`export const X = 3`) are inlined into importers and also
  stored in the export table for plain-Lua consumers.
- **Lazy bindings.** In a run-time-only cycle, a binding imported from a module
  that loads later is not copied. The importer keeps the module table
  (`local __wl_m2 = __wl.require("ui/window")`; the table already exists,
  because `__wl.module` registers placeholders for every module ahead of
  time), and each use reads `__wl_m2.Window`. This costs one table lookup per
  use, and only for those bindings.
- **Upvalue limit.** Imports are locals of the module chunk, so functions capture
  them as upvalues (at most 60 per function in Lua 5.1). If a function would
  exceed the limit, the compiler reads the excess imports through their module
  tables instead, which works like a lazy binding. If the limit is still
  exceeded, that is error E5001 naming the function.
- In bundled output ([Optimization](../toolchain/optimization.md#153-bundling)),
  each module body becomes a function inside the single output file and the
  same registry is used, so module semantics are identical in both layouts.

## 5.6 Interop with plain Lua

**wowlua consuming plain Lua.** A `.lua` file in the package keeps its normal
WoW semantics: it receives `addonName, ns` through `...` and may write to `ns`
or globals. Usage analysis (§5.4.2) orders it correctly relative to the code
that uses what it defines. To use what it provides with types, either:

- declare the globals or `ns` fields it creates in a `.d.wowlua` file
  (`declare global`, or `declare interface AddonNamespace`; see
  [Types §2.4.7](02-types.md#247-the-addon-namespace)), or
- have it register a module, and describe that module in a `.d.wowlua` file
  with the same base name:

```lua
-- src/legacy/util.lua
local _, ns = ...
ns.__wowlua.define("legacy/util", {
    round = function(n, d) local m = 10 ^ (d or 0); return math.floor(n * m + 0.5) / m end,
})
```
```wowlua
-- src/legacy/util.d.wowlua
declare module "/legacy/util"
    function round(n: number, decimals: number?): number
end

-- src/main.wowlua
import round from "/legacy/util"    -- also loads legacy/util.lua first (§5.3)
```

**Plain Lua consuming wowlua.** Plain Lua can read any wowlua module's exports
through `select(2, ...).__wowlua.require("core/math")`. To share with other
addons, use `@global` on the exported declarations.

**The addon table.** wowlua code can still use the WoW addon table:
`local addonName, ns = ...` works unchanged at module top level, typed as
described in [Types §2.4.7](02-types.md#247-the-addon-namespace). The key
`__wowlua` in that table is reserved for the runtime.

## 5.7 Cross-addon boundaries

Modules are private to the AddOn they are compiled into. Two AddOns never share
module instances, even in the same workspace. Addons communicate through
`@global` APIs, which another addon in the same workspace can type-check against
via the generated declaration file (see
[Package manager](../toolchain/package-manager.md#108-workspaces)).
