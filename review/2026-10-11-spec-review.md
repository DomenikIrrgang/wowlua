# Spec Review — 2026-10-11

Consolidated findings from three parallel reviews of `spec/` (language, toolchain,
stdlib) plus gap analysis. Working document: items are ticked off as the spec is
updated. **D-xx** = needs an owner decision, **F-xx** = mechanical fix.

Status legend: `[ ]` open · `[x]` done · `[~]` decided, not yet applied

## Owner decisions

**Status: all D-xx decided in review round R1** and recorded in `spec/appendix/decisions.md` (L8/L16/T8 amended; L20–L34, T23–T39, S1–S10 added). The owner chose the recommendation everywhere except: **D-01** (automatic load order with usage analysis, see L20), **D-05** (`::` takes full types), **D-24** (`strip` keeps `simplify`, T8 amended), **D-26** (user code under `src/`, generated code under `wowlua/`), **D-12** (all four features in v1). D-50/D-51: every proposed module approved.

Fixes are being applied by four parallel passes (language, toolchain, stdlib, new stdlib modules). New diagnostic codes are collected in `review/codes-*.md` and merged into Appendix B afterwards.

### Language

| ID | Topic | Problem | Recommendation |
|----|-------|---------|----------------|
| D-01 | Load order of files without imports | Renamed `.lua`→`.wowlua` files that share state via globals/`ns` get reordered by path; breaks "existing addons compile unchanged" | Files without import edges keep the order of `build.load-order` (written by `wowlua init` from the old TOC); imports only add constraints; path order only as last resort |
| D-02 | Decorator name resolution | `import * as reflect` shadows core `@reflect`; re-exported `profile` triggers W3021 | Core decorator names always resolve to the core decorator; a user function with that name must be aliased |
| D-03 | `?.` calls and multiple returns | Lowerings truncate to one value; and/or form double-evaluates | Optional calls always yield exactly one value; and/or form only for side-effect-free operands |
| D-04 | Forward references between module-level exports/classes | `export function a() b() end; export function b()` → `b` is a nil global | Pre-declare all top-level exported functions, classes and enums (`local a, b` hoisted) |
| D-05 | Extent of type after `::` | `x :: number \| y` ambiguous | `::` takes a postfix type only; unions/intersections need parentheses |
| D-06 | Decorator ordering | Core/framework before user decorators means `@global @memoize` exports the unmemoized fn | Registration decorators (`@global`, `@event`, `@slash`, `@hook`, `export`) capture the final value; transforming core decorators (`@profile`, `@trace`, `@if`, `@debugOnly`, `@inline`) apply innermost |
| D-07 | Field decorators vs L8 | L8 says `(target, ctx) -> replacement?`; fields defined as `(ctx) -> ()` | Field decorators receive `(nil, ctx)` and return is ignored; amend L8 wording |
| D-08 | Variadic generics / tuple types | `pairs`, `select`, `pcall`, `EventArgs<E>`, router args, pool ctor args untypeable | Add type packs (`...T` generic parameters, Luau-style) + intrinsic-checked stdlib APIs |
| D-09 | Lowerings relying on unsound types (basic mode) | Interpolation without `tostring`, `?.`→`and`, `??`→`or` trust types that may be wrong at runtime | Optimized forms only in strict mode; basic mode always uses safe forms |
| D-10 | Methods defined outside class bodies | `function Tracker:record()` at top level on a sealed class | Not allowed (E2003 with fix-it) |
| D-11 | Where declarations may appear | class/enum/interface/type inside functions undefined | `type`/`interface` anywhere (block-scoped); `class`/`enum` only at module top level |
| D-12 | v1 scope: getters/setters, implementation overloads, coroutine typing | Undefined | Out of v1 (documented as future work); coroutines typed via `thread` + `any` |
| D-13 | `setmetatable` and addon namespace typing | Classic OOP pattern and `local _, ns = ...` untyped | `setmetatable` returns first argument's type merged with literal `__index`; `...` at file top is `(string, AddonNamespace)`, `AddonNamespace` extendable via `declare` |
| D-14 | Module-level `return` | Undefined in modules with exports | Error in modules, allowed in scripts |
| D-15 | Generic variance | Undefined for generic classes/interfaces | Covariant in basic, invariant for mutable positions in strict |

### Toolchain

| ID | Topic | Problem | Recommendation |
|----|-------|---------|----------------|
| D-20 | Compact minification vs trace | WoW errors have no column → one-line output undecodable | Every level keeps a newline at least at each statement boundary; source maps are line-based |
| D-21 | Bundling and load errors | One module error aborts all later modules (separate files don't) | `__wl.load` isolates each module (xpcall → `geterrorhandler()`) and continues |
| D-22 | Output dirs per publish target | Targets with different settings overwrite each other | `dist/<profile>/` for builds; `dist/targets/<target>/` + target-suffixed zips for publish builds |
| D-23 | `@if` on imports / pipeline order | Module graph is shared but `@if` can remove imports per flavor | Module graph per flavor; single-TOC order = topological sort of the union |
| D-24 | `strip` preset vs T8 | `strip` includes `simplify` (beyond "comments/whitespace") | Remove `simplify` from `strip` |
| D-25 | `config` keys across profiles | E4012 makes the manifest's own example invalid | Top-level `config` with defaults; profiles override |
| D-26 | Generated output dirs collide with source dirs | `src/libs` vs generated `libs/` etc. | Generated files under `_wl/` (runtime, std, locales, flavor dirs), libraries under `libs/`; collisions are errors |
| D-27 | `wowlua:meta` inside libraries | Whose addon/features/config? | `addon`/`build`/`flavor` = consuming addon's build; `features`/`config` = the library's own (enabled by consumer); libraries default to all flavors |
| D-28 | Lockfile & workspaces | Flat lockfile can't express per-addon versions/features | Resolve union of all features/flavors/dev-deps; one version per package per workspace where possible; entries record `used-by` |
| D-29 | Flavor table determinism | Synced table differs per machine → different TOCs | Lockfile records flavor-table version + resolved interface numbers |
| D-30 | `build.version` | Never defined | `X.Y.Z` for tagged release builds (tag must equal version); `X.Y.Z-alpha+hash` untagged; `X.Y.Z-dev+hash` dev profile |
| D-31 | Publish target keys / nolib | Keys are site names → no two CurseForge targets | Arbitrary target names with a `type` field; `nolib` zips supported per target |
| D-32 | XML referencing compiled code | `<Script file="x.lua">` vs `.wowlua`, bundling | Paths rewritten; modules referenced from XML stay unbundled |
| D-33 | Asset paths | Hard-coded `Interface\AddOns\Name\media\…` | Compile-time `asset("media/x.tga")` in `wowlua:meta` with existence check |
| D-34 | License files of embedded code | Stripping comments removes license headers | Preserve `--[[!` / license header comments; emit `LICENSES.txt` with each library's license |
| D-35 | Telemetry / message language / offline | Unstated | No telemetry; English only; `explain` offline |
| D-36 | Uninstall & safety of `wowlua dev` | Could overwrite non-wowlua folders | Refuse folders without `X-Wowlua-Build` unless `--force`; `wowlua uninstall` |

### Standard library

| ID | Topic | Problem | Recommendation |
|----|-------|---------|----------------|
| D-40 | Saved defaults for nested tables | `__index` defaults make nested writes go into the shared defaults | AceDB-style lazy deep defaults: nested defaults materialized into the real table on first access; strip compares recursively |
| D-41 | WoW events vs addon messages | Both `declare event` → can't implement `RegisterEvent` vs internal dispatch | `declare message NAME(...)` with separate `MessageName`; `send` only accepts messages |
| D-42 | Access to saved data before load | Undefined; examples violate it | Error in all builds (with clear message); stdlib-internal phase work runs first on load, last on logout; settings/launcher defer store access |
| D-43 | Addon Compartment mechanism | Compiler-generated TOC directives vs manifest vs `@global` | Runtime `AddonCompartmentFrame:RegisterAddon` (no global, no TOC directive) |
| D-44 | Minimap button vs LibDBIcon | Two buttons; detection timing | Use LibDBIcon if present at PLAYER_LOGIN, else built-in button; named frame `<Addon>MinimapButton` |
| D-45 | `wowlua:mock` | Stdlib module or separate `wow-mock` package? | Stdlib module (compiler-versioned); project mocks in `tests/mocks/` |
| D-46 | Comm throttling across addons | Private copies each assume full bandwidth | Use ChatThrottleLib when loaded, else built-in; versioned chunk wire format |
| D-47 | Test VM scope | Whole addon or import graph? | Whole addon in TOC order; lifecycle (`ADDON_LOADED`, `PLAYER_LOGIN`) fired before tests unless opted out |
| D-48 | Timer handle API | Wrapper vs native `Cancel()` | Native `C_Timer` handles typed as-is (`Cancel`, `IsCancelled`) |
| D-49 | Hooks on load-on-demand targets | Registration at module load fails for LoD Blizzard addons | `@hook(target, name, { addon = "Blizzard_X" })` defers until loaded |
| D-50 | Core stdlib gaps | Keybindings, tooltip hooks, movable frames, waiting for addons | (owner chooses) |
| D-51 | Nice-to-have stdlib | Unit/aura/spell helpers, colors, LibSharedMedia, item cache, callbacks, jobs, import/export dialog, menus | (owner chooses) |

## Mechanical fixes (applied after decisions)

Grouped; each review item is referenced by reviewer (L = language, T = toolchain, S = stdlib) and number.

- [ ] Syntax/lowering: interpolation parentheses (L-M1), compound assignment `(e)` (L-M2), `??=` semantics (L-M2), destructuring scoping (L-C5), static-if `do…end` + one-sided partial folding (L-M7), `continue` as laststat (L-M6), `continue` lookahead list (L-m2), hygiene for referenced globals (L-M3), `?:` token in optional fields (L-M4), splitting `>=`/`>>=` in type context (L-M5), constant folding rules (L-m14)
- [ ] Grammar: rettype packs, `function` type, decorated `export`, no optional-chain assignment targets, `import type` ambiguity, digit separator rule, enum separators, typealias double export, `[String]` keys (L-M20, L-m4–m9)
- [ ] Types: false-branch narrowing (L-M11), `string` method indexing, class→record assignability in 2.6, E2007 wording, recursive types wording (L-gaps 7, 9)
- [ ] Pure-Lua downgrade covers all x1xxx/x2xxx incl. E2030/E2040, W2030; consistent wording between 0.5 and 2.1; `WOW_PROJECT_ID`/existence checks as flavor guards (L-C2)
- [ ] Shared mode: `@if`/`flavor` static only in split mode (L-M12)
- [ ] `wowlua:meta` fields readonly (L-M17); reproducibility wording (L-m12, T-m3)
- [ ] Examples: Dog/Pet, applyPendingLayout order, duplicate Handler, memoize(64), Tracker:record outside class, saved.bind overview, slash duplicate, timer examples in classes, launcher imports, serialize decode, `import * as Log` (L-M19, S-M2/M3/M4, S-minor)
- [ ] Diagnostics: W2006 duplicate, E4003 reuse, E0108/E1011 overlap, ~30 missing codes (all reviews)
- [ ] Profiling/trace wrapper limits (xpcall closure, coroutines, tail calls) (L-M14)
- [ ] Upvalue limit fallback for imports (L-M15); cache-globals/dedupe budget; remove "splits functions" (T-M6)
- [ ] `.d.wowlua` + `.lua` pairing implies load edge (L-M21)
- [ ] Toolchain: 06-meta per-flavor TOC text vs T19 (T-M1), source-map locations (T-M4), line-mode vs passes (T-M5), locale load order (T-M7, S-C2), auto-layout checks + forced single error (T-M8), appendable directives only (T-M9), suffix algorithm (T-M10), packager keyword mapping + preprocessing stage (T-M11), pipeline missing stages (T-M13), crate graph (T-C6), output validation via XML reachability (T-C7), dead-code only side-effect-free modules (T-C3), XML lua-library bundling (T-M19), manifest key completeness (T list), CLI flags, publishing details (T-m8), registry PUT spec (T-m11), roadmap gaps
- [ ] Stdlib: unitEvent per-frame registration + 2-unit limit (S-C3), router spec (S-M5), comm details b–g (S-M9), settings details (S-M15), saved details (S-M18), locale safety (S-M12), global-writes normative list (S-M11), mock coverage (S gap 7), error-handling policy (S gap 5)
- [ ] Language gaps: `...` in classes, static init order, enum iteration, contextual names in class bodies, const scoping, visibility details, overload+contextual typing, directive placement, goto/labels note (L-gaps)

## Completion (R1)

- All mechanical fixes applied by four parallel passes (language, toolchain, stdlib, new stdlib chapters `game.md`, `interface.md`, `infrastructure.md`).
- Open questions raised by the passes decided by the owner: S11–S16, T40–T41.
- New codes merged into Appendix B (`review/codes-*.md` are the raw inputs).
- Final cross-chapter consistency pass: 29 leftovers found and fixed (duplicate codes E8122/E8109/E8120 retired; locale/runtime table reuse; global-writes list; examples vs. APIs; manifest keys `profile-errors`, `stdlib`, `changelog-output`; build version without VCS; output paths; mock helpers).
- Verified in game (Retail, 2026-10-11): WoW's Lua has **no** `LUA_COMPAT_VARARG` (`global arg: nil`, `compat: nil nil`). `optimization.md` §15.3 updated.
