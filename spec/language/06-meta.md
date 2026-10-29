# 6. Build Metadata and Conditional Compilation

Addon metadata and build information come from the compiler-known module
**`wowlua:meta`**. Its exports behave like ordinary immutable imports, with one
addition: because the compiler knows their values, constant accesses are folded
at compile time and code guarded by them is eliminated.

## 6.1 `wowlua:meta`

```wowlua
import addon, build, flavor, config, features, asset from "wowlua:meta"
```

| Export | Type | Content |
|--------|------|---------|
| `addon` | `AddonMeta` | Addon metadata from the manifest |
| `build` | `BuildInfo` | Information about the current build |
| `flavor` | `Flavor` | The flavor this code is compiled for (see §6.3) |
| `config` | generated record type | User-defined values from the manifest's `config`, merged with the active profile's overrides |
| `features` | generated record type | `true`/`false` for every feature declared in the manifest |
| `asset` | `(path: string) -> string` | Compile-time asset path helper (§6.2.1) |

```wowlua
declare module "wowlua:meta"
    interface AddonMeta
        readonly name: string             -- folder / TOC name, e.g. "MyAddon"
        readonly package: string          -- package name, e.g. "my-addon"
        readonly title: string
        readonly notes: string?
        readonly version: string          -- manifest version, e.g. "1.4.2"
        readonly authors: string[]
        readonly license: string?
        readonly homepage: string?
        readonly interface: number[]      -- interface versions of the current flavor
        readonly flavors: Flavor[]        -- every flavor the addon is built for
        readonly savedVariables: string[]
        readonly savedVariablesPerCharacter: string[]
        readonly dependencies: { required: string[], optional: string[] }
        readonly meta: { [string]: string | number | boolean }   -- custom "addon.meta" entries
    end

    interface BuildInfo
        readonly profile: string          -- "dev", "release", "test", or a custom profile
        readonly debug: boolean
        readonly version: string          -- full build version (§6.1.1)
        readonly commit: string?          -- VCS commit hash, if available
        readonly dirty: boolean           -- uncommitted changes at build time
        readonly time: string             -- ISO-8601 UTC build time
        readonly timestamp: number        -- Unix seconds
        readonly compiler: string         -- wowlua version
        readonly id: string               -- unique build id (hash of inputs)
        readonly bundled: boolean
        readonly minify: string           -- active minification level name
        readonly target: string?          -- publish target this build is for, if any
    end

    const addon: AddonMeta
    const build: BuildInfo
    const flavor: Flavor
    const config: Config         -- generated from the manifest
    const features: Features     -- generated from the manifest
    @intrinsic("asset") function asset(path: string): string
end
```

All fields are `readonly`. Assigning to them is error E2010. Nested tables
(`authors`, `meta`, …) are readonly too. Folding relies on these values never
changing.

`config` and `features` get precise generated types. With a top-level
`"config": { "logChannel": "SYSTEM", "maxHistory": 50 }` in the manifest,
`config` has the type `{ logChannel: string, maxHistory: number }`. Profiles
override individual values (decision T27). Objects merge deeply, and arrays
are replaced. A profile override whose type is incompatible with the default is
error E4012, so code type-checks the same in every profile.

### 6.1.1 Build version

`build.version` is formed as follows (decision T32):

| Build | `build.version` |
|-------|-----------------|
| Release-type build of a commit tagged `vX.Y.Z` or `X.Y.Z` | `X.Y.Z`. The tag must equal `package.version`, otherwise the build fails (error). |
| Release-type build of an untagged commit, or with uncommitted changes | `X.Y.Z-alpha+<short hash>` |
| `dev` and `test` profiles (and custom profiles inheriting `dev`) | `X.Y.Z-dev+<short hash>` |
| No VCS available | `<short hash>` is omitted: `X.Y.Z-alpha` for release-type builds, `X.Y.Z-dev` otherwise ([Manifest §9.9.2](../toolchain/manifest.md#992-build-version)) |

`addon.version` is always the manifest version `X.Y.Z`.

### 6.1.2 Reproducibility

`build.time`, `build.timestamp`, `build.commit` and `build.dirty` make output
depend on the environment.

- With `--reproducible`, `build.time` and `build.timestamp` come from
  `SOURCE_DATE_EPOCH` if it is set, and otherwise from the **commit time** of the
  current VCS commit (or `0` without a VCS). They never come from the clock.
- `build.commit` always comes from the VCS, so it is reproducible for a given
  checkout.
- Without `--reproducible`, the clock is used. That is the only intended
  exception to the determinism principle, and it only affects code that reads
  these fields.

## 6.2 Folding and runtime values

Every **constant access path** into `wowlua:meta` is replaced with a literal:

```wowlua
import addon, build from "wowlua:meta"

if build.debug then
    print("Loaded", addon.name, addon.version)
end
```
```lua
-- dev profile
do
    print("Loaded", "MyAddon", "1.4.2")
end
-- release profile: the whole statement is removed
```

When a `wowlua:meta` export is used **as a value** (passed to a function,
iterated, stored, indexed with a non-constant key), the compiler emits a
reference to a runtime table instead. That table is generated into the
runtime file:

```wowlua
Settings.showAbout(addon)        -- passes the runtime table
```
```lua
Settings.showAbout(__wl.meta.addon)
```

Runtime tables contain only the fields that are actually used as values
(tree-shaken), unless `"meta": { "runtime": "full" }` is set in the manifest.
Plain Lua files and other addons can read them through
`select(2, ...).__wowlua.meta` or, if the manifest sets `addon.expose-meta`, a
named global.

### 6.2.1 Asset paths

`asset(path)` folds, at compile time, to the full in-game path of an asset
file (decision T35):

```wowlua
import asset from "wowlua:meta"

icon:SetTexture(asset("media/icon.tga"))
```
```lua
icon:SetTexture("Interface\\AddOns\\MyAddon\\media\\icon.tga")
```

- `path` must be a string literal (or constant), relative to the package root,
  using `/`.
- The file must be part of the package's assets (`build.assets`, without
  `build.exclude`), otherwise error E1038 at compile time. Case must match
  exactly. Output validation checks again that the asset is present in the
  build (E4307): the profile's `exclude` and `build.flavor-files` can leave a
  package asset out of a build.
- The result uses the addon's folder name and its location in the output
  layout, so it survives renaming the addon. Inside a library, `path` is relative
  to the library's root and resolves to the library's location in the consuming
  addon.

### 6.2.2 `wowlua:meta` in libraries

Inside a wowlua `library` package (decision T29):

| Export | Refers to |
|--------|-----------|
| `addon`, `build`, `flavor` | The **consuming addon** and its current build |
| `features` | The **library's own** features, as enabled by the consumer's dependency declaration |
| `config` | The **library's own** `config`, with values the consumer sets in its dependency declaration |
| `asset` | The library's own assets |

Libraries support all flavors unless their manifest restricts `flavors`.

## 6.3 Flavors

A **flavor** is a WoW ruleset, identified by Blizzard's *game type* as used in
TOC load directives: `camelot` (WoW Forever), `standard` (Retail), `vanilla`
(Classic Era) and `tbc` (TBC Anniversary). The manifest
lists the flavors a package supports (see
[Manifest §9.4](../toolchain/manifest.md#94-flavors), which also has the flavor
table). `Flavor` is the string literal union of the flavor IDs known to the
compiler, which ships an updatable flavor table.

A package can build in one of two **flavor modes**, set by `build.flavor-mode`:

| Mode | Meaning |
|------|---------|
| `"split"` (default) | The compiler runs once per flavor. `flavor` is a compile-time constant, flavor-specific branches are removed, and each flavor is **type-checked against its own API**. The module graph and load order are computed per flavor ([Modules §5.4.6](05-modules.md#546-several-flavors)). The output contains each flavor's compiled Lua, and the TOC layout (one TOC with load directives, or per-flavor TOCs) decides which files each client loads ([Code generation §14.9](../toolchain/codegen.md#149-toc-generation)). |
| `"shared"` | One compilation for all flavors. `flavor` is a runtime value derived from `WOW_PROJECT_ID`, compared against the literal project IDs from the flavor table (Retail does not define the Classic `WOW_PROJECT_*` constants). Code is type-checked against the *intersection* of the selected flavors' APIs, and flavor-specific APIs must be guarded (see §6.5). One TOC lists several interface versions. |

## 6.4 Conditional compilation

### 6.4.1 Static conditions

An expression is a **static condition** if it consists only of:

- constant access paths into **scalar** `wowlua:meta` values (`build.debug`,
  `features.minimap`, `config.maxHistory`, …), and `flavor` **in split mode
  only**,
- folded `const` constants (including imported ones) and literals,
- the operators `not`, `and`, `or`, `==`, `~=`, arithmetic, `..`, and the
  ordering comparisons `<`, `<=`, `>`, `>=` on **numbers** (not on strings,
  whose order depends on the runtime locale),

and reads at least one `wowlua:meta` value or `const` constant (decision T71).
Literal-only conditions such as `if true then` are plain Lua and are emitted
as written, so plain Lua compiles to identical bytecode. In `@if` a
literal-only condition is allowed.

### 6.4.2 `if` elimination

When conditions of an `if` / `elseif` chain are static, the compiler evaluates
them from left to right:

- a branch whose static condition is **false** is removed;
- a branch whose static condition is **true** becomes the final branch: it
  replaces the `else` part, and all later branches are removed;
- if the first remaining branch was an `elseif`, it becomes the `if`;
- if only an unconditional branch remains, its block is emitted as
  `do … end`, so that its locals keep their scope.

Removed branches:

- are **removed** from the output,
- are **not type-checked** or name-resolved (they are still parsed). This is
  what allows flavor-specific API calls:

```wowlua
import flavor from "wowlua:meta"

if flavor == "standard" then
    EditModeManagerFrame:RegisterSystemFrame(frame)   -- only exists on Retail
else
    frame:SetMovable(true)
end
```

Partially static `and`/`or` expressions are simplified **only when the static
operand is on the left**: `false and x` becomes `false`, `true and x` becomes
`x`, `true or x` becomes `true`, `false or x` becomes `x`. A static right
operand (`x and false`) is not folded, because `x` must still be evaluated.

Imports used only inside eliminated branches do not produce unused-import warnings.

### 6.4.3 Conditional declarations

`@if(condition)` removes a declaration (function, class, class member,
enum, `local`, `const`, `import`, type alias, interface, or `declare`) when the
condition is false. The condition must be static (E1040).

```wowlua
@if(flavor == "standard")
import * as EditMode from "./editmode"

class Bar
    @if(build.debug)
    function dumpState() … end
end
```

Referencing a removed declaration from code that is kept is error E1020,
reported with both locations. Removing an `import` changes that flavor's module
graph ([Modules §5.4.6](05-modules.md#546-several-flavors)). An `@if` on an
`import` may only use `wowlua:meta` paths and literals, not `const` constants
(E1040), because the per-flavor import graph is built before constants are
evaluated ([Compiler §13.3](../toolchain/compiler.md#133-pipeline)).

In declaration files, `@if` is how type packages describe flavor-specific
APIs:

```wowlua
@if(flavor == "standard")
declare namespace C_EditMode … end
```

**Shared mode.** In `shared` flavor mode, `flavor` is not static, so
`@if(flavor == …)` in a `.wowlua` file is error E1039. In declaration files it
keeps its meaning: the guarded declaration exists only in those flavors. That
is how the API *intersection* is computed: a declaration guarded for a subset of
the selected flavors is usable only inside a runtime guard for that subset
(§6.5).

### 6.4.4 Debug-only code

Besides `if build.debug then … end`, debug-only code can be written with:

- `@debugOnly` on functions, which removes them and every call
  ([Decorators](04-decorators.md#43-core-decorators)),
- `wowlua:debug` helpers such as `assert` and `dump`, and `wowlua:log` calls
  such as `log.debug`, which are removed when disabled and do **not evaluate
  their arguments** ([Debugging](07-debugging.md)).

## 6.5 Runtime flavor checks (shared mode)

In `shared` flavor mode, `flavor` is a runtime value and the checker requires
flavor-specific APIs to be guarded. A use of an API that does not exist in
every selected flavor is error E2030 (warning W2030 in pure Lua files) unless it
is inside a **flavor guard**. Where an API exists comes from each flavor's
declarations: its globals, the members of its namespaces (`C_Garrison.GetGarrisonInfo`)
and the members of the classes its globals are typed with, through the classes
they extend (`ChatFrameUtil.SendTell`, `EditModeManagerFrame.GetActiveLayoutInfo`).
A global whose declared type says nothing of its members (`unknown`, `table`)
may have any. These forms are recognized as guards:

| Guard | Narrows to |
|-------|-----------|
| `flavor == "standard"`, `flavor ~= "vanilla"` | The matching flavors |
| `WOW_PROJECT_ID == WOW_PROJECT_MAINLINE`, `WOW_PROJECT_ID == 2`, `WOW_PROJECT_ID ~= …` | The flavors with that project ID (constant names and literal numbers, from the flavor table) |
| Existence checks: `if C_Garrison then`, `C_Garrison and C_Garrison.GetGarrisonInfo(…)`, `if C_Garrison ~= nil then`, `if not C_Garrison then return end` | The API is usable where the check holds. A namespace that exists narrows to the flavors that have it, so its members that some of those flavors lack still need a guard |

Guards combine through `and`, `or` and `not`, hold in the branch they guard
and after an `if` whose other branches all end (`return`, `error`), and are
joined where paths meet. Availability comes from each flavor's own
declarations; the code is typed against the first selected flavor's API.

```wowlua
if flavor == "standard" then
    C_Garrison.GetGarrisonInfo(1)    -- ok: narrowed to "standard"
end
if C_Garrison then
    C_Garrison.GetGarrisonInfo(1)    -- ok: existence check
end
C_Garrison.GetGarrisonInfo(1)        -- E2030: C_Garrison is not available in "vanilla"
```

The existence check itself is never reported, in any build mode: an
undeclared global or a member the type lacks reads as `nil` in an existence
test and as `any` in the code it guards (feature detection, decision T89).

Existing multi-flavor code, which typically guards with `WOW_PROJECT_ID` or
existence checks, therefore compiles without changes.

## 6.6 Profiles

The active profile (`dev`, `release`, `test`, or a custom one) determines
`build.profile`, `build.debug`, the `config` overrides, and every debug and
optimization setting. Profiles are defined in the manifest (see
[Manifest](../toolchain/manifest.md#99-profiles)). Publish targets select a profile
and may override its minification level
([Publishing](../toolchain/publishing.md)).
