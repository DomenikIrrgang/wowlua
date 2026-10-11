# Appendix C. Decision Log

Design decisions made while drafting the specification, with their rationale.
Each entry records who decided: **(user)** means the project owner chose it
explicitly, and **(proposal)** means the spec authors proposed it and it is
open for review.

## Language

| # | Decision | Rationale |
|---|----------|-----------|
| L1 | wowlua is a **strict superset** of Lua 5.1, with no exceptions **(user)** | Existing addons must compile unchanged. This is why casts use `::` and why there are no word operators such as `as` or `is` in expressions. |
| L2 | **Luau/TypeScript-style inline type annotations** **(user)** | Familiar, concise, and parseable without ambiguity in invalid-Lua positions. |
| L3 | **Gradual typing**, default mode `basic` **(user)** | Existing Lua compiles unchanged. Annotated code gets checked. `--!strict` per file. |
| L4 | **Classes with `end` blocks** and a `constructor` keyword. Instances via `Class.new()` or `Class()` **(user)** | Looks like Lua and is explicit. |
| L5 | **import/export modules** **(user)** | No globals, deterministic load order, and room for a typed WoW standard library. |
| L6 | Build info and debug macros come from **compiler-known modules** (`wowlua:meta`, `wowlua:debug`), with **no `$` sigil** **(user)** | One mechanism (modules). Folding works because import bindings are immutable and compiler-known. |
| L7 | **Core decorators are always in scope**. Framework decorators need imports **(user)** | `@` cannot clash with Lua. Core decorators are language features. |
| L8 | **Decorators:** built-in ones are compile-time. User decorators are runtime functions `(target, ctx) -> replacement?`. Field decorators receive `(nil, ctx)` and their return value is ignored (metadata and registration only) **(user, amended in review R1)** | Powerful enough without the complexity of user compile-time macros. Fields have no value at decoration time. |
| L9 | **Reflection opt-in per class** (`@reflect`). Class identity is always available **(user)** | Keeps output small. Identity is cheap and widely needed. |
| L10 | Syntax sugar: compound assignment, `continue`, `?.`, `??`, backtick interpolation, default parameters, record destructuring **(user)** | Common requests from Lua developers. All lower to Lua 5.1. |
| L11 | Lua 5.3-style bitwise operators, `//`, digit separators, binary literals, `const` **(user)** | All lower to WoW's `bit` library or to literals. |
| L12 | **Enums with braces**: `enum E { A, B }` **(user)** | Mirrors the table it compiles to. |
| L13 | Interfaces, generics, abstract classes, type guards **(user)** | |
| L14 | `import a from "x"` (no braces) and `import _ from "x"` for side effects **(proposal)** | `import {a} from "x"` and `import "x"` are valid Lua calls (superset rule L1). |
| L15 | No explicit type arguments at call sites **(proposal)** | `f<T>(x)` is a valid Lua comparison. Inference and expected types cover the need. |
| L16 | Exported bindings are immutable **(proposal)**. Imports are local copies, except bindings that take part in a run-time-only import cycle, which are read lazily from the module table (see L20) | Cheap imports in the common case. |
| L17 | Widget classes lower to Blizzard-style mixins **(proposal)** | Frames already have a metatable, and the mixin pattern is idiomatic WoW. |
| L18 | Extended escapes (`\x`, `\u{}`) only in backtick strings **(proposal)** | `"\x41"` means `"x41"` in Lua 5.1 (superset rule L1). |
| L19 | **`Secret<T>`** models Midnight secret values. Forbidden operations are flagged, and `issecretvalue` / `canaccessvalue` narrow **(user)** | Catches a whole class of Midnight runtime errors at compile time. |

| L20 | **Automatic load order** from imports plus usage analysis of globals and `ns` fields. Edges are classified as load-time or run-time. The order is a topological sort over load-time edges. Cycles of run-time edges only are allowed (lazy bindings). Cycles containing a load-time edge are errors. Unknown callees count as load-time. `build.load-order` overrides **(user, R1)** | Migrated addons keep working, mutual references are possible, and genuine load-time cycles are caught. |
| L21 | Ambiguous ordering (dynamic global names, several writers): warning, and the affected files keep their source order **(user, R1)** | |
| L22 | Core decorator names **always** resolve to the core decorator **(user, R1)** | Predictable. A user function with the same name must be imported under an alias. |
| L23 | Optional calls (`f?.()`, `a?:m()`) yield **exactly one value** **(user, R1)** | Simple, efficient lowering. |
| L24 | Top-level exported functions, classes and enums are **hoisted** (pre-declared at module top) **(user, R1)** | Mutual references between declarations work. |
| L25 | `::` takes a **full type** including `\|` and `&`. A bitwise operator after a cast needs parentheses **(user, R1)** | Reads naturally for union casts. |
| L26 | Registering decorators (`@global`, `@event`, `@slash`, `@hook`, `export`) capture the **final** decorated value. Transforming core decorators (`@profile`, `@trace`, `@if`, `@debugOnly`, `@inline`) apply innermost **(user, R1)** | `@event @memoize` registers the memoized function. |
| L27 | **Type packs** (Luau-style `T...` generic pack parameters) plus intrinsic-checked stdlib APIs **(user, R1)** | `pairs`, `select`, `pcall`, event payloads and constructor forwarding become typeable. |
| L28 | Type-based fast lowerings (no `tostring` in interpolation, `?.` as `and`, `??` as `or`) only in **strict** mode **(user, R1)** | Basic-mode types may be wrong at runtime, so basic mode always uses the safe forms. |
| L29 | No methods outside class bodies (E2003 with a quick fix) **(user, R1)** | Classes are sealed. |
| L30 | `type`/`interface` may be declared anywhere (block-scoped). `class`/`enum` only at module top level **(user, R1)** | |
| L31 | v1 includes **getters/setters, overloads in classes, typed coroutines, `setmetatable` typing** **(user, R1)** | Only classes with properties pay for an `__index` function. |
| L32 | `local addonName, ns = ...` is typed `(string, AddonNamespace)`. `AddonNamespace` is an interface projects extend with `declare interface` (declaration merging) **(user, R1)** | Typed classic namespace sharing. |
| L33 | Top-level `return` is an error in modules and allowed in scripts **(user, R1)** | |
| L34 | Generic variance: covariant in basic mode, invariant for mutable positions in strict mode **(user, R1)** | Pragmatic by default, sound in strict. |

## Toolchain

| # | Decision | Rationale |
|---|----------|-----------|
| T1 | Spec as Markdown chapters in the repository **(user)** | Versioned with the compiler. |
| T2 | Compiler in **Rust**, one binary for compiler, package manager and tooling **(user)** | |
| T3 | Support **all WoW flavors**. Split per-flavor compilation by default **(user)** | Per-flavor type checking and dead-code removal. |
| T4 | Manifest is **strict JSON** (`wowlua.json`) with a `$schema` key **(user)** | Tool-friendly. Editors get completion through the JSON schema. |
| T5 | Dependency sources: git, local path, archive URL, and a wowlua registry (later phase) **(user)** | |
| T6 | Publish targets: CurseForge, Wago, WoWInterface, GitHub Releases **(user)** | The major WoW addon sites. |
| T7 | Minification levels are **user-defined in the project**, selectable per profile and per publish target **(user)** | Different platforms allow different amounts of minification. |
| T8 | Default with no configuration: release is **bundled + `strip`** (comments, whitespace and safe syntactic simplifications), with no renaming. Dev is unminified. **(user, amended in review R1)** | Fast loading, readable code. |
| T9 | Dev builds: one file per module with **line numbers preserved**. Release: bundled. **(user)** | In-game errors in dev builds point at the right source line with no tooling. |
| T10 | Libraries and the stdlib are embedded as a **private, tree-shaken copy per addon** **(user)** | No cross-addon version conflicts. |
| T11 | Additional tooling: test runner, language server, formatter, trace decoder **(user)** | |
| T12 | WoW API types: generated from **Blizzard's API documentation**, enriched with additional information from Ketho's vscode-wow-api (MIT) **(user)** | Most accurate, per-flavor, typed event payloads. |
| T13 | Warning W6010 when a publish target uses a renaming minification level **(user)** | Blizzard's AddOn policy forbids hidden or obfuscated code. |
| T14 | **One zip for all flavors**, tagged with every flavor's game version per site. Per-target `split-flavors` available. **(user)** | Packager convention, supported by CurseForge and Wago. |
| T15 | Ecosystem compatibility: import existing TOC and `.pkgmeta`, optionally keep `.pkgmeta`, support BigWigs packager keywords in passthrough files **(user)** | Easy migration of existing addons. |
| T16 | Resolver: PubGrub, one version per package per addon **(proposal)** | Everything is embedded into one AddOn. |
| T17 | `wowlua publish` requires a clean tree and runs tests by default **(proposal)** | Safe releases. Both can be overridden with flags. |
| T18 | Build output is validated (TOC references, Lua 5.1 reparse, Lua limits, XML) before writing **(proposal)** | Guarantees a working AddOn. |
| T19 | **TOC layout `auto`**: a single TOC with comma-separated interfaces and inline `[AllowLoadGameType …]` directives when every targeted client supports it, otherwise per-flavor TOC files **(user)** | Blizzard's direction, fewer files. The fallback keeps frozen clients working. |
| T20 | Flavor IDs are **Blizzard game types** (`standard`, `camelot`, `mists`, `cata`, `wrath`, `tbc`, `vanilla`), with friendly aliases in the manifest **(user)** | They name rulesets rather than clients, and match TOC directives. |
| T21 | WoW API types are **pre-generated** in CI and published as `wow-api-<flavor>` packages **(user)** | Convenience. Blizzard's documentation has no explicit license, and the project owner accepted publishing derived type information. |
| T22 | `wowlua:locale`: JSON files per locale, typed named placeholders, and **only the base locale plus the client's locale are loaded** (via `[AllowLoadTextLocale]` where every client supports it, otherwise a runtime guard) **(user, wording clarified in R1)** | Tool-friendly translations, compile-time checked, less memory. |
| T23 | Every minification level keeps **at least one line break per statement**, so source maps stay line-based and `wowlua trace` always works **(user, R1)** | WoW error messages have no column numbers. |
| T24 | Bundles **isolate modules**: each runs protected, errors go to `geterrorhandler()`, and later modules still load **(user, R1)** | Same behavior as separate files. |
| T25 | Publish builds go to `dist/targets/<target>/`. Zips get a target suffix when targets differ **(user, R1)** | No collisions between targets. |
| T26 | Module graph and load order are computed **per flavor**. Single-TOC order is a topological sort over all flavors **(user, R1)** | `@if` on imports works. |
| T27 | Top-level `config` defines defaults that profiles override (deep merge for objects, replacement for arrays) **(user, R1)** | |
| T28 | Output layout: user code under `src/`, generated code under `wowlua/`, libraries under `libs/` **(user, R1)** | No collisions with user folders. |
| T29 | `wowlua:meta` in libraries: `addon`, `build`, `flavor` from the consuming addon; `features`, `config` are the library's own. Libraries support all flavors unless declared otherwise **(user, R1)** | |
| T30 | Resolution over the **union** of features, flavors and dev-dependencies, with one version per package across the workspace where possible. Lock entries record their users **(user, R1)** | `--features` never invalidates the lockfile. |
| T31 | The lockfile pins the **flavor-table version and resolved interface numbers** **(user, R1)** | Deterministic TOCs across machines. |
| T32 | `build.version`: tagged release builds use `X.Y.Z` (the tag must match), untagged builds `X.Y.Z-alpha+<hash>`, and the dev profile `X.Y.Z-dev+<hash>` **(user, R1)** | |
| T33 | Publish targets have **free names plus a `type` field**. Per-target `nolib` zips are supported **(user, R1)** | Several targets per site. |
| T34 | XML references to `.wowlua` are rewritten. Modules referenced from XML stay separate files in bundled builds **(user, R1)** | |
| T35 | `asset(path)` in `wowlua:meta` folds to the full game path and checks that the file exists **(user, R1)** | |
| T36 | License header comments are never stripped. Every build gets a generated `LICENSES.txt` **(user, R1)** | Library licenses stay intact. |
| T37 | No telemetry. English messages only. `explain` works offline **(user, R1)** | |
| T38 | `dev`/`install` refuse to overwrite AddOn folders not built by wowlua unless `--force` is given. `wowlua uninstall` exists **(user, R1)** | |
| T39 | Dependencies never run code at fetch or build time. Path traversal and escaping symlinks are rejected. Tokens are stored in the OS keychain where available **(user, R1)** | |
| T40 | The wowlua runtime and standard library embedded in addons are **MIT-licensed** **(user, R1)** | Permissive. Addon authors keep any license for their own code. |
| T41 | Distribution through **GitHub Releases binaries** (plus install script and a `setup-wowlua` GitHub Action), **Homebrew, winget and Scoop** **(user, R1)** | |
| T42 | `wowlua init` **moves** Lua and XML files into `src/` and rewrites absolute in-game paths to them. Assets stay in place **(user, M0)** | Standard layout for adopted addons, without breaking references. |
| T43 | `build.flavor-files` restricts source files to flavors (glob → flavors) **(user, M0)** | Plain Lua files cannot use `@if`. `init` writes it from flavor-specific TOCs. |
| T44 | In single TOCs, a flavor that is the only targeted one of its family is written as the family name (`mainline`/`classic`) in load directives **(user, M0)** | Recognized by every client with inline directives. |
| T45 | `wowlua install` defaults to the `dev` profile **(user, M0)** | Same as `wowlua dev`, without watching. |
| T46 | Manifest schema URL: the GitHub raw URL of `schema/wowlua-<version>.json` in the repository **(user, M0)** | Works without a website. |
| T47 | M0 accepts `.wowlua` files that are pure Lua 5.1. M0 includes a release pipeline to GitHub Releases. M0 creates only the crates it needs **(user, M0)** | |
| T48 | Releases are built by a hand-written GitHub Actions workflow (six targets, checksums, install scripts) instead of cargo-dist. `setup-wowlua` is a composite action inside the wowlua repository **(proposal, M0)** | Transparent and easy to extend with Homebrew, winget and Scoop. No separate `wowlua` GitHub organization is needed. |
| T49 | A file matched by several `build.flavor-files` globs belongs to the union of their flavors **(proposal, M0)** | Predictable, order-independent. |
| T50 | `addon.flavor-overrides` expresses per-flavor metadata (title, notes, author, category, group, icon, `toc-extra`). `null` removes a field for a flavor **(user, ElvUI test)** | Real addons (ElvUI) vary titles, icons and compartment entries per flavor. One TOC can do this with inline conditions on `##` lines. |
| T51 | The package manager supports **SVN** sources, locked to a revision, using the system `svn` client **(user, ElvUI test)** | Ace3, LibStub and other core libraries are published on CurseForge's SVN repositories. `.pkgmeta` projects convert without manual work. |
| T52 | `Bindings.xml` / `Bindings_<Suffix>.xml` in the package root are passed through to the AddOn root **(user, ElvUI test)** | The client loads them automatically; dropping them loses keybindings. |
| T53 | `wowlua init` determines, per flavor, the one TOC that flavor's client reads (game suffix, then family suffix, then the plain name) and takes files, dependencies and metadata only from it **(proposal, ElvUI test)** | Matches the client. Combining TOCs would load files a client never loads. |
| T54 | `addon.toc-data` holds per-flavor files of verbatim `## Key: Value` lines. `init` writes them when a TOC has more than 200 `X-` directives **(proposal, corpus test)** | QuestieDB stores its database in about 100,000 TOC directives per flavor. In the manifest they made a 163 MB file; as data files the manifest is 16 KB. |
| T55 | `build.variable-files` keeps TOC lines with `[Game]`, `[Family]` and `[TextLocale]`. They are emitted verbatim when every targeted client supports file variables and expanded otherwise. Every expansion must exist (E5002) **(proposal, corpus test)** | BigWigs and DBM use them. Expansion keeps older clients working. |
| T56 | Flavor table: Cataclysm Classic 4.4.2 and the closed Wrath Classic 3.4.5 client support every TOC feature, including file variables, and recognize their own and all older game types **(proposal, corpus test)** | BigWigs and DBM ship single TOCs with these features for both clients. The earlier data was too conservative and forced per-flavor TOCs. |
| T57 | `init` takes the union of `SavedVariables` across the TOCs of all flavors and the value used by the most flavors for `Version`, load flags and publish IDs **(proposal, corpus test)** | Structural fields cannot vary per flavor (T50). The union loses no saved data; a single stub TOC, such as Questie's `Interface: 00000` file, is outvoted by the real ones. |

## Standard library

| # | Decision | Rationale |
|---|----------|-----------|
| S1 | `wowlua:saved` uses **lazy deep defaults** (AceDB-style). Stripping at logout is recursive **(user, R1)** | Nested writes persist, and files stay small. |
| S2 | Addon-internal messages are declared with **`declare message`**. `send` accepts only messages, and only WoW events are registered with the game **(user, R1)** | |
| S3 | Accessing saved data before load is **always an error**. Stdlib phase work runs first on load and last on logout **(user, R1)** | Writes would otherwise be silently lost. |
| S4 | Addon Compartment through `AddonCompartmentFrame:RegisterAddon` at runtime. Minimap button through LibDBIcon if it is loaded at login, otherwise a built-in `<Addon>MinimapButton`, never both **(user, R1)** | No globals, no TOC directives, works with button managers. |
| S5 | `wowlua:mock` is a **stdlib module**. Project mocks go in `tests/mocks/` **(user, R1)** | Always compatible with the compiler. |
| S6 | `wowlua:comm` uses **ChatThrottleLib** when loaded, otherwise a built-in conservative throttle. Versioned wire format **(user, R1)** | Shared bandwidth across addons. |
| S7 | Tests load the **whole addon** in load order and fire `ADDON_LOADED` and `PLAYER_LOGIN` before tests, unless opted out **(user, R1)** | Matches the game. |
| S8 | `@hook(target, name, { addon = "…" })` defers until a load-on-demand addon loads. `onAddonLoaded(name, fn)` helper **(user, R1)** | |
| S9 | Timers return **native C_Timer handles** **(user, R1)** | No extra allocation. |
| S10 | New stdlib modules: keybindings, tooltip hooks, movable frames, a uniform error policy, unit/aura/spell helpers, colors, SharedMedia interop, item/spell data cache, callback registry, background jobs, dialogs, menus and lists **(user, R1)** | |
| S11 | Dialogs get named frames `<Addon>Dialog<N>` registered in `UISpecialFrames`, so Escape closes them **(user, R1)** | Standard WoW mechanism, works in combat. |
| S12 | Public callback APIs are exposed through `@global` **(user, R1)** | Works on every client and for plain-Lua consumers. |
| S13 | Saved-data helpers only through `saved.handle(db)`, never on the data table **(user, R1)** | User keys never clash with helpers. |
| S14 | On downgrade, newer saved data is kept unchanged, stripping is paused, and one warning is logged **(user, R1)** | Nothing is lost if the user upgrades again. |
| S15 | No automatic `/<addon> config` command. Settings are opened through an explicit slash router entry **(user, R1)** | No unexpected globals. |
| S16 | `wowlua:comm` supports an optional AceComm-compatible wire format per channel **(user, R1)** | Addons migrating from Ace3 stay compatible with their older versions. |
