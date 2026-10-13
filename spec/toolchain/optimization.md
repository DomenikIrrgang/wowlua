# 15. Optimization

Optimizations reduce in-game **load time** (fewer files, fewer bytes to read
and parse), **memory** and **runtime cost**. Every optimization preserves
observable behavior. The minifier is verified by differential execution in the
compiler's test suite ([Compiler §13.11](compiler.md#1311-testing-the-compiler)).

## 15.1 Always-on optimizations

These run in every profile, because they make output smaller without making it
harder to read:

| Pass | Effect |
|------|--------|
| Constant folding | `wowlua:meta` paths, `const`s, enum members and constant arithmetic/concatenation become literals |
| Static branch elimination | `if` branches with static conditions, `@if` declarations, `@debugOnly` functions and calls |
| Debug stripping | Disabled `wowlua:debug` calls, log calls below `log-level`, and trace wrappers are removed with their arguments |
| Tree-shaking | Runtime slices, unused standard-library modules, unused **wowlua library** modules, and unused enum tables are omitted. Plain Lua code reading exports through `__wowlua.require(...)` cannot be traced: modules listed in `build.keep` are always kept, and a string-literal `__wowlua.require("id")` in a `.lua` file keeps that module automatically. |
| Type erasure | All type syntax removed |

## 15.2 Profile-controlled optimizations

| Setting | Values | Effect |
|---------|--------|--------|
| `optimize.inline` | bool | Inline local functions whose body is a single `return <expression>` (§15.2) |
| `optimize.cache-globals` | `"off"` `"safe"` | Hoist frequently used read-only globals and Lua standard-library functions into file-level locals (`local pairs, type = pairs, type`). Faster access in hot code. `"safe"` only hoists names the module never assigns and that are on the compiler's stable-global list (Lua stdlib and immutable WoW API namespaces). Never hoists names that addons commonly hook. |
| `optimize.dead-code` | bool | Remove unused local functions, unused module-private declarations, and **side-effect-free** modules nothing uses (§15.2.1) |
| `bundle` | bool | Bundle all Lua into as few files as possible (§15.3) |
| `minify` | level name | Apply a minification level (§15.4) |

Defaults:

| Setting | `dev` | `test` | `release` |
|---------|-------|--------|-----------|
| `optimize.inline` | `false` | `false` | `true` |
| `optimize.cache-globals` | `"off"` | `"off"` | `"safe"` |
| `optimize.dead-code` | `false` | `false` | `true` |
| `bundle` | `false` | `false` | `true` |
| `minify` | `"none"` | `"none"` | `"strip"` |

In the manifest, `optimize` is a nested object:
`"optimize": { "inline": true, "cache-globals": "safe", "dead-code": true }`.

The optimizations apply to compiled code and to the project's plain Lua
files (like minification, §15.4.1), before minification.

**`cache-globals: "safe"`** hoists, the most used first, names read at least
twice and never assigned in the file: Lua 5.1's base functions (`pairs`,
`type`, `tostring`, `select`, …, but not `print`), the library tables and
their functions (`string.format` becomes a local of its own), WoW's Lua
aliases (`strsplit`, `tinsert`, `wipe`, `format`, …) and `C_*` namespace
tables, never their functions and never other WoW API functions (decision
T162). A file that uses `setfenv`, `getfenv`, `loadstring` or `_G` is left
alone. A function that takes the hoisted locals as upvalues stays within the
budgets of §15.2.2.

**`inline`** replaces calls of a local function whose body is a single
`return <expression>` (no function expressions, no `...`) with that
expression when the function is not recursive, never assigned and only ever
called, the call's arguments are literals or locals, and the body's other
names mean the same at the call. Calls used as statements stay calls. A
function whose calls were all inlined goes. `@inline` is a hint for such
functions; on anything else (a longer body, a recursive or escaping function,
a method) it is ignored with W3031 (decision T166).

**`dead-code`** removes `local` declarations and local functions nothing uses
whose values have no side effects (literals, tables of them, functions,
locals), repeatedly, and dead modules (§15.2.1).

### 15.2.1 Dead modules

A module is removed only if **both** hold:

- it is **side-effect-free**: its top level contains only declarations
  (`local function`, `const`, `class`, `enum`, `interface`, `type`, `export` of
  these, and `local`s initialized with literals, table constructors or function
  expressions), with no calls, no global or addon-table writes, and no
  decorators that register anything (`@event`, `@slash`, `@hook`, `@global`,
  user decorators); and
- no kept module imports it, no XML file loads it, it is not listed in
  `build.keep`, and no plain Lua file requires it by name
  (`__wowlua.require("id")`).

Plain `.lua` files and scripts are never removed. They are entry points. Each
removed module is reported with warning W6002.

### 15.2.2 Lua-limit budgets

Passes that add locals or upvalues stay within budgets, so optimization never
creates a Lua 5.1 limit error that the unoptimized code did not have:

| Pass | Budget |
|------|--------|
| `cache-globals` | Stops hoisting when the file's main chunk would exceed 180 active locals, or when any function would exceed 50 upvalues. The most frequently used names are hoisted first. |
| `dedupe-strings` | Same limits, applied to its shared string locals |
| `inline` | Does not inline into a function if that would exceed 180 locals or 50 upvalues in it |

Bundling adds no locals to modules, because each module keeps its own
function (§15.3.1).

## 15.3 Bundling

Bundling combines the runtime, libraries, standard-library modules and all
modules of the addon into **one Lua file**. When XML files must load
between Lua files, it produces the minimum number of segments that preserve the
load order.

### 15.3.1 Structure

```lua
local __wl_name, __wl_ns = ...
local function __wl_load(id, f)
xpcall(function() f(__wl_name, __wl_ns) end, function(e)
local message = __wl_name .. " (" .. id .. ", build 9f3a1c2e): " .. tostring(e)
local handler = geterrorhandler and geterrorhandler()
if handler then handler(message) else print(message) end
end)
end
-- each file as a function, in load order, named by its path
__wl_load("wowlua/runtime", function(...) <runtime> end)
__wl_load("libs/libstub/LibStub", function(...) <LibStub.lua> end)
__wl_load("src/core/cache", function(...) <module> end)
__wl_load("src/main", function(...) <module> end)
```

- Each module or plain Lua file (the runtime too) becomes a function, called
  with `(addonName, ns)` so that `...` behaves exactly as in a separate file.
  Locals stay private to their file, and Lua's per-function limits (200
  locals, 60 upvalues, registers, constants) apply per file, as they did
  before bundling. Each wrapper adds one nested syntax level and two call
  levels.
- **Error isolation.** In WoW, a load-time error in one file stops only that
  file. Bundles keep that behavior: the bundle's own `__wl_load` runs each
  wrapper protected, through a closure (Lua 5.1's `xpcall` takes no extra
  arguments). The handler forwards the error, prefixed with the addon name,
  the file and the build id, to `geterrorhandler()`, so BugSack and the
  default error frame show it as usual, and loading continues with the next
  file. Modules that imported something from the failed module see `nil`
  exports, exactly as with separate files. Every segment defines its own
  loader, so bundles need no runtime (decision T165).
- **What is bundled.** The TOC's Lua files: the runtime, the standard-library
  modules, wowlua libraries, lua-library entries and the project's files.
  Files that must stay separate split the bundle into segments at their place
  in the load order: XML files, locale files, files of other AddOns
  (`build.external-files`), TOC lines with variables, library files kept
  apart (below), and files that do not compile on their own. When the
  flavors load different files or orders, each flavor gets bundles of its
  own (`<Name>.<flavor>.lua`).
- **Libraries kept apart.** `"bundle": false` on the dependency, and every
  lua-library that calls `debugstack` or `debuglocals` (often to find its own
  folder for media paths), stay separate files; the build notes why (decision
  T163).
- **Source maps.** A bundle's map (`<Name>.lua.map`) composes the maps of its
  files (identity for files that were not compiled), with the original names
  of renamed locals, so `wowlua trace` decodes bundle locations.
- A top-level `return` in a plain Lua file returns from its wrapper function,
  which matches file semantics.
- **Remaining differences from separate files** (documented, and checked by
  warning W6001 for the project's bundled files that call `debugstack` or
  `debuglocals`): `debugstack()` and error messages show the bundle file name
  and line, and the stack is two levels deeper. `wowlua trace`
  maps locations back through source maps. Code that inspects its own file name
  through `debugstack`, or that uses numeric stack levels with `getfenv`,
  `setfenv` or `error`, sees the bundle frames.
- **No implicit `arg` table.** WoW's Lua is built **without** Lua 5.1's
  `LUA_COMPAT_VARARG` (verified 2026-10-11 on the Retail client: a vararg
  function that never uses `...` has no local `arg`, and there is no global
  `arg`). Wrapping a file in `function(...) … end` therefore cannot shadow
  anything. Wrappers still mention `...` in their body, at no cost, so the
  output stays correct on any Lua 5.1 build, including the test VM.
- **XML.** A TOC-listed XML file (the project's own, like `embeds.xml`, or a
  library's entry) that, with every XML file it includes, contains nothing but
  `<Script file>` and `<Include file>` elements is flattened into the files it
  loads, in document order, unless another XML file loads any of them
  (decision T167). Any other XML file stays, splits the bundle into segments
  at its position, and the files it loads stay separate
  ([Code generation §14.8.4](codegen.md#1484-xml)). Segments that every flavor
  has alike are one shared bundle.
- Libraries that must stay separate files can also be excluded with
  `"bundle": false` on the dependency, e.g. libraries other addons reference by
  path.

### 15.3.2 Why bundling helps

WoW reads and compiles every file listed in a TOC separately, so per-file
overhead adds up for addons with hundreds of small files (typical with embedded
libraries). One file also enables whole-addon minification (shared renaming,
dead-module removal). The spec does not promise a specific speed-up. `wowlua
build --stats` reports the output's file count and size, its Lua files and
their size, and how long the embedded Lua 5.1 takes to compile them (the parse
cost the client pays at load, without its per-file overhead), so the effect is
measurable for each project.

## 15.4 Minification levels

Minification levels are **named and defined in the manifest**. Each publish
target (and each profile) selects one. That lets a project ship different levels
to different sites according to each site's rules. The compiler provides
built-in passes and preset levels, and projects may redefine presets or add their
own.

### 15.4.1 Passes

| Pass | Effect | Readability impact |
|------|--------|--------------------|
| `strip-comments` | Remove comments, except license headers ([Code generation §14.8.6](codegen.md#1486-licenses)) | Low |
| `strip-whitespace` | Remove indentation, trailing spaces and blank lines. In `line-mode = "preserve"`, blank lines are kept so that line numbers stay correct. | Low |
| `compact` | Minimal whitespace inside each line: only the separators the Lua grammar requires | Medium |
| `shorten-literals` | Shortest equivalent numeric literals (`1000000` → `1e6`) and shortest string quoting | Low |
| `simplify` | Safe syntactic simplifications (below) | Low |
| `rename-locals` | Rename local variables, parameters and upvalues to the shortest available names, most frequent first | High. Original names are recoverable only through source maps. |
| `rename-private` | Also rename private and protected class methods and the fields of module-private tables that provably do not escape (below) | High |
| `dedupe-strings` | Hoist repeated long string literals into shared locals, within the budgets of §15.2.2 | Medium |

**What is minified.** The level applies to compiled code, to the project's
plain `.lua` files (not those in `build.libs-dir`, files with packager flavor
blocks, or `compat.plain-copy` files) and to the runtime. Lua-library
dependencies (and `build.libs-dir`) are minified only with
`minify-libraries`. A library file that is not plain Lua 5.1 ships as it is.

**Preserve mode keeps every line.** Unbundled output (`line-mode =
"preserve"`) keeps the line of every instruction, so in-game errors point at
the source line without a source map. Lua gives an instruction the line of
the last token it read, so there `simplify` rewrites only statements that sit
on one line, and `shorten-literals` leaves literals that span lines alone.

**`rename-locals`** never renames globals, table fields or the implicit
`self` of methods, and leaves locals named `arg` alone: a Lua 5.1 built with
`LUA_COMPAT_VARARG` (unlike WoW's, §15.3.1) declares `arg` in every vararg
function, so keeping the name keeps the meaning on any build. No local is
renamed to `arg`.

**`dedupe-strings`** hoists a string only when that saves bytes: it occurs at
least twice and its occurrences are longer than the local's declaration plus
one name per use.

**`rename-private`** renames:

- the fields of a `local t = { … }` (named and positional fields only) that is
  never assigned and only used as `t.field`, `t.field = v` or
  `function t.field()`: never passed, returned, indexed with `[…]`, iterated,
  given a metatable or called with `:`;
- private and protected **methods** that are not abstract, not decorated, not
  metamethods (`__…`) and not members of widget classes. A method name is
  renamed only if every occurrence of it in the package (as a name or as a
  string's content), in its declaration files, the WoW API declarations, its
  XML files and its embedded libraries is a definition of such a method or an
  access the type checker resolved to one. The new name occurs in none of
  them, and is the same in every class, so overrides stay overrides.

Instance **fields** are never renamed: their names are the keys of saved
variables and serialized data, which must stay readable across versions
(decision T161).

**Every level keeps at least one line break per statement.** WoW error
messages contain a line number but no column, so output joined onto a single
line could not be mapped back to the source. Bundled output therefore uses
`line-mode = "statement"` at minimum
([Code generation §14.5](codegen.md#145-line-preservation)).

**`simplify` rules.** The pass may remove redundant parentheses and `;`, merge
consecutive `local` declarations, and rewrite `not (a == b)` to `a ~= b`, with
these restrictions:

- never remove parentheses around a call or `...` whose result count they
  truncate (`(f())`, `(...)`), or parentheses that change precedence or
  associativity;
- never remove a `;` that separates a statement from a following one starting
  with `(` (Lua's ambiguous-call rule);
- never merge `local a = x` and `local b = y` if `y` mentions `a`, or if the
  second declaration shadows a name of the first (`local x = 1; local x = x + 1`);
- never rewrite comparisons whose operands may have metamethods
  (`not (a == b)` → `a ~= b` is always safe in Lua 5.1, but `not (a < b)` →
  `a >= b` is not, and is never done).

Globals, table field names reachable from outside the addon, `@global` names,
exported names, event names, and names used by XML or `_G` lookups are **never
renamed**.

### 15.4.2 Preset levels

| Level | Passes |
|-------|--------|
| `none` | (none) |
| `strip` | `strip-comments`, `strip-whitespace`, `simplify` |
| `compact` | `strip`, `compact`, `shorten-literals` |
| `rename` | `compact`, `rename-locals`, `dedupe-strings` |
| `max` | `rename`, `rename-private` |

**Defaults:** dev and test use `none`. Release uses `strip`: bundled, with
comments and whitespace removed and safe simplifications applied, identifiers
untouched.

### 15.4.3 Defining levels

```json
{
  "minify-levels": {
    "readable": { "passes": ["strip-comments", "simplify"] },
    "small":    { "extends": "compact", "passes": ["dedupe-strings"] },
    "strip":    { "passes": ["strip-comments", "strip-whitespace"] }
  },
  "profiles": {
    "release": { "minify": "small" }
  },
  "publish": {
    "targets": {
      "cf": { "type": "curseforge", "project-id": 123456, "minify": "readable" },
      "gh": { "type": "github", "repository": "jane/my-addon", "minify": "small" }
    }
  }
}
```

- `extends` starts from another level's passes.
- Redefining a preset name (like `strip` above) replaces the preset for this
  project.
- Pass order is fixed by the compiler, not by list order, so results are
  deterministic.
- An unknown pass name is error E6003. An `extends` cycle is error E4007.

### 15.4.4 Platform policies

Blizzard's UI Add-On Development Policy states that *"the programming code of
an add-on must in no way be hidden or obfuscated, and must be freely accessible
to and viewable by the general public."* CurseForge and Wago require uploads to
comply with Blizzard's terms. wowlua leaves the choice of level per target to
the project. When a publish target uses a level containing `rename-locals` or
`rename-private`, `wowlua publish` prints warning W6010 citing the policy, and
`--deny-warnings` turns that warning into a failure before anything is
uploaded.

### 15.4.5 Verification

Besides the compiler's tests, the conformance corpus
([Compiler §13.5.3](compiler.md#1353-superset-conformance)) checks every file at
every pass: with every pass but `dedupe-strings` and `rename-private`, the
output compiles to the same bytecode once debug information is stripped, and
in preserve mode every instruction keeps its line; with all passes, it still
compiles. The semantic passes are checked by running test programs before and
after. The corpus addons are also adopted and built bundled, with the release
optimizations, at every preset level; every Lua file of each build must
compile (`cargo xtask corpus addons`).
