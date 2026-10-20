# 14. Code Generation and Output

This chapter defines how lowered code is emitted, which runtime support exists,
and what the output AddOn looks like. The guarantee behind it:

> **The output of every successful build is a complete, working AddOn.** It can
> be copied into `Interface/AddOns/` as-is. It contains every TOC, compiled file,
> library, runtime piece and asset it references, and nothing it doesn't.

## 14.1 Target

Emitted code is Lua 5.1 as embedded in WoW:

- no `goto`, integer division operator, bitwise operators or 5.2+ escapes;
- bit operations use WoW's `bit` library;
- `unpack`, not `table.unpack`; `select("#", ...)` for vararg counts;
- no reliance on what WoW removed: `io`, `os`, the `debug` library, `require`,
  `module`, `package`, `dofile`, `loadfile`. `date`, `time`, `difftime`,
  `debugstack` and `debugprofilestop` are WoW globals;
- no use of Lua helpers that exist only on some clients (e.g. `math.clamp`,
  `string.startswith` and `table.contains` exist only in 12.1.5+ clients) unless
  the flavor's types declare them;
- never arithmetic, comparison, `#` or indexing on values typed as possibly
  secret ([Types §2.11](../language/02-types.md#211-secret-values)) in generated
  code, for example in runtime checks or profiling;
- no global writes other than those listed in §14.1.1.

### 14.1.1 Global writes

This is the **complete, normative list** of globals a wowlua build may create.
Other chapters refer to it.

| Global(s) | Created by | Builds |
|-----------|------------|--------|
| The name given to `@global(name)` | User code ([Decorators §4.3](../language/04-decorators.md#43-core-decorators)) | All |
| `SLASH_<KEY>1…n`, `SlashCmdList["<KEY>"]`, where `<KEY>` is `WOWLUA_<ADDON>_<N>` | `@slash` / `wowlua:slash` (WoW's slash API requires them) | All |
| `BINDING_HEADER_<ADDON>`, `BINDING_NAME_<BINDING>`, and one dispatch table `<Addon>_Bindings` | Keybinding declarations (standard library), read by the generated `Bindings.xml` (§14.8.5) | All |
| `<Addon>MinimapButton` (a named frame) | `wowlua:launcher`, only when it falls back to its built-in minimap button because LibDBIcon is absent | All |
| `<Addon>Dialog<N>` (named frames, also added to `UISpecialFrames`) | `wowlua:dialog`, so Escape closes dialogs | All |
| The name in `addon.expose-meta` (the `wowlua:meta` runtime tables) | Manifest opt-in ([Manifest §9.3](manifest.md#93-addon)) | All |
| SavedVariables globals (`_G[name]` is pointed at the table bound by `saved.bind`, so WoW saves it) | `wowlua:saved`, for names declared in the manifest | All |
| `WowluaVersions` (one table shared by every addon; each writes only its own entry) | `wowlua:version`, so any addon can see the installed versions | All |
| `WOWLUA_DEV`, `SLASH_WOWLUA1`, `SlashCmdList["WOWLUA"]`, `WowluaDevWindow` (a named frame), the SavedVariables `WowluaDevDB_<Addon>` | Dev console ([Debugging §7.7](../language/07-debugging.md#77-dev-console)) | Dev builds with `dev-console` |
| Anything written by user code through `_G`, to a `declare global`d name, or by plain Lua | User code | All |

`<Addon>` is `addon.name` with characters outside `[A-Za-z0-9_]` replaced by
`_`. The Addon Compartment uses `AddonCompartmentFrame:RegisterAddon` at runtime
and creates no global. Neither the runtime nor any other standard-library
module writes globals.

## 14.2 File prologue

A compiled module that needs the runtime starts with a prologue **on line 1**,
followed on the same line by the module's own first line:

```lua
local __wl = select(2, ...).__wowlua; local __wl_exports = __wl.module("core/cache"); <first source line>
```

- Files that use no runtime features and have no exports get no prologue. A pure
  Lua file therefore compiles to itself, apart from formatting.
- The prologue declares `__wl`, `__wl_exports` (modules with exports only), the
  locals of hoisted declarations, and the hygienic global caches of §14.3 that
  the file needs. Imports add one local per imported binding on the import's
  own line, or one local per imported module for lazy bindings
  ([Modules §5.5](../language/05-modules.md#55-runtime-representation)). If a
  chunk exceeds the 200-local limit because of generated locals, the build
  reports E5001 and names the generated locals involved.

## 14.3 Hygiene

Generated identifiers use the prefix `__wl` (`__wl`, `__wl_exports`, `__wl_t1`,
…). Before emitting a chunk, the compiler collects every identifier that appears
in it, including globals, and renames any generated identifier that would
collide by appending a numeric suffix. User identifiers are never renamed,
except by the `rename-locals` and `rename-private` minification passes.

**Referenced globals.** Lowerings call standard globals (`tostring`, `type`,
`select`, `setmetatable`, `rawget`, `error`, `math.floor`, `bit.*`, …). When a
user binding with the same name is in scope at a generated use site, the
compiler caches the global in a hygienic local in the prologue
(`local __wl_tostring = tostring`) and uses that instead. Otherwise it uses the
global directly.

Hygiene runs on each **final** output chunk, after bundling, so generated names
of different modules and of the bundle frame cannot collide.

## 14.4 Preceding statements

Some expressions cannot be lowered to a single Lua expression without changing
semantics: optional chains whose result can be `false`, `??` on possibly-false
values, compound assignment targets containing calls, and destructuring. These
lower to **preceding statements**, which are temporaries and `if`s inserted
before the statement that contains the expression.

To preserve Lua's left-to-right evaluation order, every operand that is
evaluated before the complex expression in the same statement, and that has
side effects (contains a call or a vararg), is hoisted into a temporary as well.

| Context of the complex expression | Strategy |
|-----------------------------------|----------|
| Ordinary statement | Insert the statements before it, wrapped in `do … end` together with the statement if temporaries would leak |
| `local` declaration | Evaluate into temporaries first, then declare the locals from the temporaries, so the right-hand side never sees the new locals |
| `if` / `elseif` condition | `elseif` becomes `else` + nested `if`, with the statements in between |
| `while` condition | `while true do <stmts>; if not (cond) then break end; <body> end` |
| `repeat … until` condition | Statements appended at the end of the body, before `until` |
| Numeric/generic `for` header | Insert before the loop (headers are evaluated once) |
| `return` | Insert before the `return` |
| Function argument default | Inside the default's `if v == nil then … end` |
| Short-circuit operand (right side of `and`/`or`/`??`) | Lowered to an `if` on a temporary, so the right side's statements only run when needed |

## 14.5 Line preservation

The profile key `line-mode` selects how emitted statements map to lines:

| `line-mode` | Behavior | Default for |
|-------------|----------|-------------|
| `"preserve"` | Every source statement stays on its original line number | Unbundled output |
| `"statement"` | Every statement on its own line, no blank lines | Bundled output |
| `"pretty"` | Readable formatting with indentation and blank lines | — |

**No minification level ever produces fewer lines than `"statement"`**: every
statement boundary keeps a line break, so source maps stay line-based and
`wowlua trace` can always resolve an in-game error, which carries a line but no
column ([Optimization §15.4](optimization.md#154-minification-levels)). In
`"preserve"` mode, whitespace passes remove indentation and trailing spaces but
never line breaks.

In `"preserve"` mode the emitter works as follows:

- Each IR statement's origin line is its target line. Before emitting a
  statement, the emitter adds newlines until the output reaches the target line.
- Generated code that belongs to a source construct (field initializers,
  decorator application, `continue` flags, class finalization, profiling
  wrappers, export assignments, hoisted declarations) is placed **on the same
  line** as its anchor: the line of the construct's start, or of its `end`.
  Statements are separated with `; `.
- If the output is already past the target line (only possible when a
  multi-line construct lowered to more lines than the source had, which no
  construct in this spec does), the emitter continues on the current line and
  records a source-map entry for the drift.
- Multi-line backtick strings are emitted as single-line strings with `\n`
  escapes, followed by padding newlines.

As a result, the line numbers in in-game error messages from dev builds match
the `.wowlua` source. In split flavor mode, error paths of per-flavor files
contain the flavor directory (`wowlua/flavors/standard/src/…`), which
`wowlua trace` strips.

## 14.6 Source maps

When `source-maps` is enabled, every emitted Lua file gets a Source Map v3 file
mapping output lines to `.wowlua` positions, including original identifier names
when locals are renamed (keyed by output line and name, since WoW errors have
no columns).

| Build | Location |
|-------|----------|
| Profile builds | `dist/<profile>/.maps/<Addon>/<flavor>/<output path>.map` for per-flavor files, `dist/<profile>/.maps/<Addon>/<output path>.map` for files shared by all flavors (decision T68) and bundles (`<Name>.lua.map`) |
| Publish builds | `dist/targets/<target>/.maps/<Addon>/<flavor>/<output path>.map` |
| Published builds (archive) | `$WOWLUA_HOME/maps/<build id>/`, plus the `archive-maps` upload ([Publishing §11.7](publishing.md#117-github-releases)) |

Maps are always **outside** the AddOn folder, so they are never installed or
packaged. `wowlua trace` looks them up by build id: first in `dist/`, then in
the archive, then in downloaded `-maps.zip` release assets.

## 14.7 The runtime (`__wl`)

The runtime is a small Lua library emitted as `wowlua/runtime.lua`, or the
first file of a bundle. It is **sliced by feature**: only the parts the
compiled code uses are included. It stores itself in the addon table as
`ns.__wowlua` and never in globals. If locale files already created that table
(§14.9.2), the runtime extends it instead of replacing it.

| Slice | Contents | Needed by |
|-------|----------|-----------|
| `core` | `module`, `require`, `define`, `error` helper, `meta` tables, lazy-binding support | modules, `wowlua:meta` values |
| `class` | `class`, `finalize`, `isInstance`, `classOf`, abstract stubs, property dispatch | classes |
| `widget` | `widgetClass`, mixin flattening, `__ctor` threading | widget classes |
| `enum` | frozen enum tables (debug) | enums used as values in debug builds |
| `decorate` | context table helpers | user decorators |
| `locale` | locale table access (base locale, then client locale) | `wowlua:locale` |
| `check` | runtime type checks and error formatting | `runtime-checks` |
| `prof` | profiler wrappers, statistics | `@profile`, `instrument` |
| `trace` | trace wrappers and formatting | `@trace` |
| `reflect` | metadata registry | `@reflect`, `reflection = "all"` |
| `devconsole` | `/wl` command, cross-addon registry, dev window | `dev-console` |
| `monitor` | event, script and timer wrappers, memory samples | `monitor` |
| `errors` | error handler chaining and the error log | `capture-errors` |

The `class` slice provides `class`, `accessor`, `finalize`, `abstract`,
`classOf` and `isInstance`; `widget` provides `widgetClass`, `mixin`,
`finalizeWidget`, `xmlMixin` and `script`; `enum` provides `enum`; `decorate` provides
`decorate`, `decorateField`, `register` and `deprecated`.

The runtime is written in wowlua in `crates/wowlua-runtime/wowlua/`, one
file per slice, and compiled into the Lua the compiler embeds
(`crates/wowlua-runtime/lua/`, committed) by `cargo xtask runtime`; the
compiler cannot compile its own runtime while it is being built. The slices
are concatenated into one chunk, so each is compiled on its own with the
runtime package's `runtime` profile (no instrumentation, checks,
optimizations or minification; line numbers preserved), the slices after
`core` seeing its locals through declarations. `cargo xtask check` (CI) fails
when the committed Lua is out of date or a slice has a warning. Each slice
declares its dependencies on other slices. The driver computes the closure from
the features recorded during lowering. **Standard-library modules** are not part
of the runtime. They are wowlua modules compiled into the addon like any
dependency, and tree-shaken the same way.

Generated meta tables (`__wl.meta.addon`, `__wl.meta.build`, …) are part of the
`core` slice and contain only the fields used as runtime values (see
[Build metadata](../language/06-meta.md#62-folding-and-runtime-values)).

## 14.8 Output layout

The output tree has a fixed top level:

| Top-level entry | Content |
|-----------------|---------|
| `<Name>.toc` / `<Name>_<Suffix>.toc` | Generated TOC file(s) (§14.9) |
| `<Name>.lua`, `<Name>.<n>.lua`, `<Name>.<flavor>[.<n>].lua` | Bundles and bundle segments (bundled builds) |
| `Bindings.xml`, `Bindings_<Suffix>.xml` | Keybindings (§14.8.5): generated from `wowlua:bindings`, or the package's own files copied from the package root |
| `LICENSES.txt` | Licenses of embedded code (§14.8.6) |
| `wowlua/` | Everything generated: `runtime.lua`, `std/` (standard-library modules), `locales/`, `flavors/<flavor>/` (per-flavor variants) |
| `libs/<name>/` | Dependencies: lua-libraries in their original layout, wowlua libraries compiled |
| `src/` | The project's compiled sources and XML, mirroring `build.src` |
| anything else | Assets, at their path relative to the package root (`media/x.tga` → `media/x.tga`) |

These names are **reserved**. An asset whose output path collides with a
reserved entry, or with another output file, is error E4311.

### 14.8.1 Unbundled (default for `dev` and `test`)

```
dist/dev/MyAddon/
├── MyAddon.toc
├── LICENSES.txt
├── wowlua/
│   ├── runtime.lua
│   ├── std/events.lua           # used standard-library modules
│   └── locales/
│       ├── enUS.lua
│       └── deDE.lua
├── libs/
│   ├── libstub/LibStub.lua      # lua-library dependency, original layout
│   └── wowlua-ui/…              # wowlua library dependency, compiled
├── src/
│   ├── core/cache.lua           # one file per source module
│   ├── main.lua
│   └── ui/frames.xml
└── media/…                      # assets
```

### 14.8.2 Bundled (default for `release`)

```
dist/release/MyAddon/
├── MyAddon.toc
├── MyAddon.lua                  # runtime + libraries + std + all modules
├── LICENSES.txt
├── wowlua/locales/…             # with client-locale loading
├── src/ui/frames.xml            # XML cannot be bundled
├── src/ui/window.lua            # referenced by frames.xml <Script>: kept separate
└── media/…
```

Bundling rules are in [Optimization](optimization.md#153-bundling). When XML
files sit between Lua files in the load order, the Lua is split into the
minimum number of bundle **segments** (`MyAddon.lua`, `MyAddon.2.lua`, …) needed
to keep the order. Modules referenced from XML stay separate files (§14.8.4).

### 14.8.3 Split flavor mode

In `split` mode with more than one flavor, compiled code can differ per flavor.
Files that are **byte-identical across all flavors** stay at their normal path
and are listed for every flavor. Files that differ are stored per flavor under
`wowlua/flavors/<flavor>/<normal path>`. Bundles are named
`<Name>.<flavor>.lua` (segments `<Name>.<flavor>.<n>.lua`).

```
dist/release/MyAddon/
├── MyAddon.toc                  # single TOC (auto layout, all targets support it)
├── MyAddon.standard.lua
├── MyAddon.mists.lua
├── MyAddon.vanilla.lua
├── LICENSES.txt
├── wowlua/locales/…
├── src/ui/frames.xml            # identical in every flavor: stored once
└── media/…
```

### 14.8.4 XML

`.xml` files from `build.src` are copied to `src/`, mirroring their path, and
`.xml` files of lua-libraries keep their place under `libs/<name>/`.

- **Reference rewriting.** `<Script file="…">` and `<Include file="…">` paths
  are resolved relative to the XML file. References to `.wowlua` sources are
  rewritten to the compiled `.lua` path. Unresolvable references are E5002.
- **Inline directives.** A `file` attribute may carry inline load directives
  like a TOC file line (`file="Data_TBC.lua [AllowLoadGameType tbc]"`), which
  the client evaluates (decision T62). The path is resolved without them, and
  they are kept unchanged in the output.
- **Case.** References resolve case-insensitively, like the client. A
  reference whose case differs from the file is warning W4310 (decision T61).
- **XML-referenced modules stay separate.** A module loaded through `<Script>`
  is not bundled and not listed in the TOC: the XML file loads it, at the
  position the XML file has in the load order.
- **Per-flavor XML.** If an XML file references a module whose output differs
  per flavor, the XML file is emitted per flavor as well
  (`wowlua/flavors/<flavor>/src/…`), with references pointing at that flavor's
  files.
- XML files are otherwise passed through unchanged.

### 14.8.5 Keybindings

When the addon declares keybindings through the standard library, the compiler
generates `Bindings.xml` in the AddOn root, which the client loads
automatically. Each `<Binding>` element's body calls the dispatch table
`<Addon>_Bindings["<binding name>"]`, which the module fills at load time. The
binding header and display names become `BINDING_HEADER_*` /
`BINDING_NAME_*` globals (localized through `wowlua:locale` when the names are
locale keys). These globals are listed in §14.1.1.

**The package's own keybinding files** (decision T52). `Bindings.xml` and
flavor variants such as `Bindings_Mainline.xml` in the **package root** are
copied to the AddOn root unchanged, because the client loads them
automatically, not through the TOC. They must be well-formed XML (E5002).
`wowlua init` leaves them where they are. A package that declares keybindings
through `wowlua:bindings` and also has its own `Bindings.xml` gets error
E4334, because the client would read only one of them.

### 14.8.6 Licenses

- **Header comments are kept.** Comments starting with `--[[!`, and the leading
  comment block of a file when it contains "license", "copyright" or an SPDX
  identifier, are never removed by any minification pass.
- **`LICENSES.txt`.** Every build contains a generated `LICENSES.txt` with one
  section per embedded component: each library (its `LICENSE`/`COPYING` file,
  its manifest `license`, or its preserved header), the standard-library
  modules used, and the wowlua runtime. A wowlua library inside a workspace
  (a member, or a path dependency in the workspace root's folder) with none
  of these has the workspace root's license: its `LICENSE`/`COPYING` files,
  else its package's `license`. A package without a `workspace` key counts
  as its own workspace here, so a wowlua library path dependency in its
  folder has the package's license. A Lua library never inherits one, since a
  copy of someone else's code in the repository is not under its license. The runtime and standard library are
  released under the **MIT license** (decision T40). Its notice is included in
  `LICENSES.txt`, and addon authors may use any license for their own code.
- Library license files are copied as part of the library
  (`libs/<name>/LICENSE`).

### 14.8.7 Assets

Files matched by `build.assets` are copied verbatim to their path relative to
the package root. `asset("media/x.tga")` from `wowlua:meta` folds to the
in-game path (`Interface\AddOns\<Name>\media\x.tga`). The path must name an
asset of the package (E1038 at compile time) and is checked again during output
validation (E4307).

## 14.9 TOC generation

TOC files are generated entirely from the manifest and the computed load order.
Users never edit them.

### 14.9.1 Layouts

| `build.toc-layout` | Output |
|--------------------|--------|
| `"single"` | One `<Name>.toc`. `## Interface:` lists every flavor's interface numbers, comma-separated. Each file line that does not apply to all flavors ends with an inline `[AllowLoadGameType …]` directive. Locale files use `[AllowLoadTextLocale …]`. |
| `"per-flavor"` | One TOC per flavor, named by the suffix algorithm in §14.9.3, each listing only that flavor's files. No inline game-type directives. |
| `"auto"` (default) | `"single"` if **every** targeted client supports all TOC features the single layout needs (below). Otherwise `"per-flavor"`. |

A single TOC needs, from **every** targeted client:

- comma-separated `## Interface:` values,
- inline load directives on file lines,
- inline directives on `##` directive lines, if any flavor-specific
  `addon-dependencies` entry exists (12.0.7+ clients),
- `[AllowLoadTextLocale]`, if locale files are loaded per client locale,
- recognition of every game-type name the TOC uses.

The flavor table records each client's support. Forcing `"single"` when a
targeted client lacks a needed feature is error E4301, naming the client and
the feature. The compiler only uses `AllowLoadGameType` (never
`ExcludeLoadGameType`), because its behavior is defined for clients that do
not recognize a listed game type. In the per-flavor layout,
`[AllowLoadTextLocale]` is used if every client of that TOC supports it;
otherwise every locale file is listed without a directive and skipped by its
runtime `GetLocale()` guard ([Standard library: locale](../stdlib/ui.md#locale)).

Example of a single TOC (bundled, split flavors):

```
## Interface: 120100, 50504, 11509
## Title: |cff33ff99My|r Addon
## Version: 1.4.2
## SavedVariables: MyAddonDB
## X-Wowlua-Build: release 9f3a1c2e
wowlua\locales\enUS.lua
wowlua\locales\deDE.lua [AllowLoadTextLocale deDE]
MyAddon.standard.lua [AllowLoadGameType standard]
MyAddon.mists.lua [AllowLoadGameType mists]
MyAddon.vanilla.lua [AllowLoadGameType vanilla]
src\ui\frames.xml
```

**Directive names.** A flavor that is the only targeted flavor of its family is
written as the **family name** (`mainline` or `classic`), which every client
with inline directives recognizes. Game-type names (`standard`, `mists`, …) are
used only when several targeted flavors share a family (decision T44). This
mirrors rule 1 of the suffix algorithm (§14.9.3).

**Directive safety.** A client that recognizes none of the game types listed in
`[AllowLoadGameType …]` treats the condition as passed and loads the file. That
is why `auto` checks, using the flavor table, that every targeted client
recognizes every game-type name used. Without that check, an older client could
load every flavor's code at once.

### 14.9.2 File order

1. **Locale files first.** Compiled locale files are self-contained: they write
   into a table in the addon table and do not need the runtime. They are listed
   before every other Lua file, base locale first, so `wowlua:locale` is filled
   before any module reads it.
2. **Then the load order** computed by the compiler (runtime, libraries,
   standard-library modules, project files; see
   [Modules §5.4](../language/05-modules.md#54-load-order) and
   [Compiler §13.6.1](compiler.md#1361-load-order-analysis)).
3. **Single TOC with split flavors.** The file list is a topological sort over
   the **union** of all flavors' load-time graphs. Flavor-specific files carry
   their game-type directive. If the flavors' orders conflict (A before B in one
   flavor, B before A in another, for files shared by both), the compiler falls
   back to the per-flavor layout with warning W4304.

### 14.9.3 Per-flavor TOC names

The client looks for `<Name>_<GameSuffix>.toc`, then `<Name>_<FamilySuffix>.toc`
(`_Mainline` or `_Classic`), then `<Name>.toc`. Game suffixes are recognized
only by clients that know them (`_Standard`, for example, only by 12.1.5+
clients). For each targeted flavor, the compiler chooses:

1. the **family suffix** if this flavor is the only targeted flavor of its
   family,
2. otherwise the **game suffix**, if every targeted client of this flavor
   recognizes it,
3. otherwise the family suffix, if no other targeted flavor of the same family
   uses it (each client prefers a recognized game suffix over the family
   suffix, so the other flavors still find their own file),
4. otherwise error E4302: the flavors cannot be told apart by file name. Use
   the single layout or drop a flavor.

A plain `<Name>.toc` is emitted only when exactly one flavor is targeted.

Example: targeting `standard`, `camelot` and `vanilla` while Retail is on
12.1.0 gives `MyAddon_Mainline.toc` (standard, rule 3, since 12.1.0 does not
know `_Standard`), `MyAddon_Camelot.toc` (rule 2) and `MyAddon_Classic.toc`
(vanilla, rule 1).

### 14.9.4 Contents

Each TOC contains, in order:

1. `## Interface:` with the interface version(s) pinned in the lockfile,
2. `## Title:` followed by localized `## Title-<locale>:` lines (the fallback
   must come first), then notes, author, version (`build.version`), icon,
   category and group,
3. dependency directives from `addon-dependencies`, in manifest order (the
   order may influence which dependency the client loads first). Consecutive
   entries for the same flavors share one directive (decision T63).
   Flavor-specific entries
   get an inline directive in single layout or go only into that flavor's TOC,
4. `## SavedVariables:` / `## SavedVariablesPerCharacter:`,
   `## LoadSavedVariablesFirst:`, `## AllowAddOnTableAccess:`,
5. verbatim `addon.toc-extra` fields, then the lines of the flavor's
   `addon.toc-data` files ([Manifest §9.3.2](manifest.md#932-toc-data-files)),
6. `## X-Wowlua-Build: <profile> <build id>` and the publish-target ID fields
   (`## X-Curse-Project-ID`, `## X-Wago-ID`, `## X-WoWI-ID`) when configured,
7. the file list (§14.9.2), using `\` separators. `build.external-files`
   lines (`..\<AddOn>\src\…`) are listed verbatim at their load-order
   position; `build.shared-files` are shipped but not listed
   ([Manifest §9.7.2](manifest.md#972-files-shared-between-addons)).

**Line length.** The client reads only the first 1024 characters of each line.
Only the appendable directives `Dependencies`, `OptionalDeps`, `LoadWith`,
`LoadManagers`, `SavedVariables` and `SavedVariablesPerCharacter` are split
across repeated lines, which the client merges. Any other directive or file
line longer than 1024 characters is error E4303.

The directive set, the flavor suffixes, the supported features and the interface
numbers come from the compiler's flavor table
([Manifest §9.4.1](manifest.md#941-flavor-table)).

## 14.10 Lowering reference

| Construct | Chapter |
|-----------|---------|
| Syntax sugar (`+=`, `continue`, `?.`, `??`, interpolation, bitwise, `const`, defaults, destructuring) | [Syntax](../language/01-syntax.md) |
| Classes, widget classes, enums | [Classes](../language/03-classes.md) |
| Decorators | [Decorators](../language/04-decorators.md#453-lowering) |
| Modules | [Modules](../language/05-modules.md#55-runtime-representation) |
| Meta folding, conditional compilation | [Build metadata](../language/06-meta.md) |
| Profiling, tracing, checks, reflection | [Debugging](../language/07-debugging.md) |
