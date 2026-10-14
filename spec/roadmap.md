# Roadmap

Milestones are ordered so that every milestone produces a usable tool. Each
lists its exit criteria and, once reached, its status.

## M0: Foundation

**Status: done (2026-10-11).**

- Cargo workspace, CI (fmt, clippy, tests on macOS, Windows, Linux), release
  pipeline publishing binaries to GitHub Releases on tags (Homebrew, winget and
  Scoop follow later).
- `wowlua-span`, `wowlua-diagnostics`, `wowlua-flavors` (flavor table),
  `wowlua-manifest` (JSON schema generation, profile merging, `package.wowlua`
  check), `wowlua-toc` (layouts, suffix algorithm, line-length rules),
  `wowlua-xml` (reference checks).
- CLI skeleton: `new`, `init` (from TOC), `build` for **plain Lua projects**
  (passthrough, TOC generation, assets, `LICENSES.txt`, output validation),
  `install`, `uninstall`, `dev` (copy and link modes, safe installs), `clean`,
  `toc`, `explain`.
- `.wowlua` files are accepted when they are pure Lua 5.1 and passed through as
  `.lua`; files using wowlua syntax report that they need the M1 compiler.
  Bundling and minification are not available yet (release builds are
  unbundled, with a note).
- Output validation of Lua syntax uses the embedded Lua 5.1 VM (`mlua`,
  compiling without running) until the compiler's own parser exists in M1.
- Lockfile with the pinned flavor table.

**Exit:** an existing Lua addon can be built and installed from a `wowlua.json`
with generated TOCs for several flavors.

## M1: Parser and emitter

**Status: done (2026-10-11).**

- Lexer and parser for full Lua 5.1 plus all wowlua syntax, with error recovery.
- Lua IR and emitter with line modes, hygiene and source maps.
- Superset conformance suite: Lua 5.1 tests, addon corpus, bytecode round-trip.
- `wowlua check` (syntax only), `wowlua fmt` (initial: comment-preserving
  printer for Lua 5.1 constructs).

**Exit:** the corpus parses with zero errors, and round-trip bytecode is
identical for every corpus file.

Notes on the delivered state:

- `.wowlua` files are compiled with type-only syntax erased (decision T64);
  other wowlua syntax reports the milestone that implements it.
- The corpus (28 pinned addon releases and the Lua 5.1 test suite, 7,179
  files, 4.8 million lines) parses with the same accept/reject result as Lua
  5.1, and every file round-trips to identical bytecode in all line modes; in
  `preserve` mode even the line info is identical. The formatter formats every
  corpus file idempotently without changing its meaning.
- Fuzzing is done with deterministic randomized tests on stable Rust
  (decision T67). `wowlua check --watch` follows with incremental checking
  (M4).

## M2: Lowering of syntax sugar and modules

**Status: done (2026-10-11).**

- `+=`, `continue`, `?.`, `??`, interpolation, bitwise, `//`, literals, `const`,
  defaults, destructuring, preceding-statement linearization.
- Modules: `import`/`export`, resolution, declaration hoisting, runtime `core`
  slice.
- Load-order analysis: usage analysis of globals and addon-table fields,
  load-time/run-time edges, lazy bindings for run-time cycles, per-flavor
  module graphs and the union order for single TOCs.
- `wowlua:meta` (including `asset()`), static conditions, `@if`, flavor split
  builds, XML reference rewriting.

**Exit:** multi-module example addons build for all flavors and run in the
embedded Lua VM with identical behavior to hand-written Lua. A migrated
plain-Lua addon keeps its working load order.

Notes on the delivered state:

- Lowering is checked by differential tests that run compiled `.wowlua` and
  hand-written Lua in the embedded VM; multi-module and split-flavor packages
  are built and run per flavor in the CLI tests.
- `cargo xtask corpus addons` adopts every corpus addon with `wowlua init`
  and builds it: all 108 keep their TOC order. The usage analysis
  over-approximates without types; where it contradicts the listed order the
  listed order wins with W1035 (decision T70).
- Classes, enums and decorators other than `@if` report M3.

## M3: Classes, enums, decorators and stdlib infrastructure

**Status: done (2026-10-11).**

- Classes (including abstract and generic), widget classes, interfaces (parsed;
  checked in M4), enums.
- Core decorators (`@global`, `@deprecated`, `@if`, `@inline`) and user
  decorators.
- Runtime slices `class`, `widget`, `decorate`.
- Standard-library infrastructure: compiling `wowlua:*` modules from
  `wowlua-stdlib` into addons, tree-shaking them, and the `@intrinsic`
  decorator table, so later milestones can ship modules.

**Exit:** a UI-heavy sample addon written with widget classes works in game.

Notes on the delivered state:

- The sample addon is `examples/widget-demo` (decision T80); CI builds it and
  runs it against a mock frame API. It was checked in game on the WoW: Forever
  beta client (1.60.1).
- Class, enum and decorator checks that need only declarations are reported
  now (decision T75); type-based ones come with M4.
- Native widget types are declared by the compiler until M4 (decision T77).
- No public standard-library modules yet: the infrastructure is checked with
  test modules through `WOWLUA_STDLIB_DIR` (decision T79).

## M4: Type checker

**Status: done (2026-10-12).**

- Basic mode: inference, narrowing, assignability, arity, globals, classes,
  interfaces, generics, overloads.
- Declaration files. `wow-api-<flavor>` packages generated per flavor from
  Blizzard's API documentation and merged with Ketho's annotations;
  `wowlua types sync`, `wowlua types convert`.
- Strict mode, `Secret<T>`.
- `shared` flavor mode with flavor guards (E2030).
- `wowlua lint`, `wowlua fix`.

**Exit:** the corpus checks in basic mode with zero errors. Sample addons
check in strict mode with zero errors.

Notes on the delivered state:

- The corpus (108 addons, plain Lua checked with `check.lua`) checks with
  zero errors; applying all lint fixes to it keeps every addon building.
  `examples/widget-demo` and `examples/typed-demo` check in strict mode
  without diagnostics, and CI runs both.
- The `wow-api` declarations are embedded in the compiler (crate
  `wowlua-api`) and regenerated with `cargo xtask types fetch` and
  `cargo xtask types gen` (decisions T83–T85). The `wow-api-<flavor>` types
  packages and `wowlua types sync` arrive with the package manager.
- Leniency rules that keep valid Lua free of errors are decision T87;
  feature detection (existence tests) works in every build mode (T89).

## M5: Debugging

**Status: done (2026-10-12).**

- `wowlua:debug`, `wowlua:reflect`, `wowlua:log` (on the M3 stdlib
  infrastructure).
- Profiling, tracing, runtime checks, dev console.
- `wowlua trace`.

**Exit:** a profiling report appears in game, and runtime checks catch bad
values from the WoW API in a dev build.

Notes on the delivered state:

- Checked in game on the WoW: Forever client with `examples/typed-demo` and
  `examples/widget-demo`: the dev window's profiler, monitor (events,
  scripts, timers), log and errors tabs, and `/tdemo check`, whose runtime
  check catches a `nil` from `UnitName`.
- Dev builds have every tool built in without code changes (decision T96):
  every function is profiled, plain `.lua` files included (T97), handlers
  are monitored, errors are captured, and `/wl` opens the dev window, drawn
  in the SuuUI design with its fonts shipped (T101–T103). The corpus builds
  this way with zero failures.
- `wowlua trace` maps locations of line-preserved builds; decoding bundled
  builds through source maps arrives with bundling (M7).

## M6: Package manager

**Status: done (2026-10-12).**

- Path, git and archive sources, lua-library embedding (reachable files and
  licenses only), lockfile, PubGrub resolution over the union, cache,
  workspaces with generated declaration files.
- `add`, `remove`, `update`, `fetch`, `tree`, `outdated`, `cache clean`,
  `fix --manifest`.
- Dependency safety (archive extraction rules, submodule policy).
- `.pkgmeta` import and compatibility mode, packager keywords.

**Exit:** an Ace3-based addon migrates with `wowlua init --from-pkgmeta` and
builds identically.

Notes on the delivered state:

- Sources: git (bare clones, exported per commit), SVN (system `svn`,
  pinned to a revision), path and archive (`https`, `file://`); the registry
  waits for its server (T104), and the wow-api types stay embedded.
- The exit check (`packager-compat` workflow, T119): thirty-six addons built
  with the BigWigs packager, at pinned commits, packaged by the packager the
  way their CI does (multi-version, `-S`, or game by game with `-g`/`-m`)
  and built by wowlua after `init --from-pkgmeta`, load identical code in
  every flavor and client locale and ship the same files (the same bytes
  outside `src/`, with the packager's line endings and TOC versions): Bartender4, BigWigs (12 AddOns),
  LittleWigs (12), WeakAuras (5), DBM (8, with its voice pack as an external
  addon), Details (8), ElvUI (3), Inventorian (2), Plater, Masque, Mapster,
  Quartz, BugSack, SimulationCraft, AdvancedInterfaceOptions, GatherMate2
  (4), ShadowedUnitFrames (2), AtlasLootClassic (8), RCLootCouncil2,
  omnibar, Capping (2), BigWigs_Voice, BigWigs_Classic, SexyMap,
  MythicDungeonTools (2), TellMeWhen (2), AllTheThings, Dominos (6), OmniCC
  (2), Cell, TotalRP3 (2), BlizzMove (2), QuickQuest,
  BigWigs_WrathOfTheLichKing, Tukui and KuiNameplates (4). Rarity and hekili
  are left out: the packager itself cannot fetch their LibCompress SVN
  external. The corpus still adopts and builds all its addons.
- wowlua `library` packages compile into the addon (tree-shaken modules,
  their own check settings and meta, T110–T112); workspaces share one lock
  and `dist/`, with generated declaration files of `@global` APIs (T113,
  T114).

## M7: Optimization and publishing

- Bundling (with module isolation and XML-aware segments), minification passes
  and levels, global caching, inlining, dead-code removal, Lua-limit budgets.
- `package`, `publish` for CurseForge, Wago, WoWInterface and GitHub
  (`release.json`, archived source maps, `split-flavors`, `nolib`,
  `bundle-addons`), `login` with OS credential stores, changelog handling.
- `wowlua new --ci github` template.

**Exit:** a release is published to all four sites from CI with one command.

- Done 2026-10-13 with SuuUI, a WoW Forever addon adopted with `wowlua init`:
  one tag push (`v0.2.0-beta.2`) made CI publish to CurseForge and Wago (the
  readable build, as betas) and to GitHub Releases (the fully optimized
  build, with source maps and `release.json`). WoWInterface takes no WoW
  Forever addons yet; its upload is covered by the publish tests against a
  mock site.

- Optimization was checked in game on the WoW: Forever client (2026-10-13):
  Bartender4 adopted with `wowlua init` and built bundled (`strip`, release
  optimizations) loads from 2 bundles and its template XML, with no Lua
  errors; so does the build at `max` with `minify-libraries` (Ace3 and the
  other libraries renamed, compacted and optimized too: 774 KiB of Lua
  instead of 1.29 MB, compiled in a third of the time). The corpus checks every minification pass on 7179 files and builds
  its 108 addons bundled at every level.

## M8: Standard library

- Core set first: `events`, `lifecycle`, `slash`, `saved`, `timer`, `combat`,
  `hooks`, `log`, error policy.
- Then `locale` (with the compiler's locale pipeline: JSON compilation, the
  `L` type, client-locale TOC directives), `pool`, `settings`, `launcher`,
  `comm`, `serialize`, keybindings (with `Bindings.xml` generation), tooltip
  hooks, movable frames.
- Then unit/aura/spell helpers, colors, SharedMedia interop, item/spell data
  cache, callback registry, background jobs, dialogs, menus and lists.
- Widget themes for dialogs, the fallback menu and the built-in minimap button
  (decisions S17, S18).

**Exit:** a sample addon uses no third-party libraries for its events, saved
variables, settings, minimap button, keybindings and comms.

**Status: in progress.** The stages follow the bullets above: S1 the core set,
S2 `locale` to movable frames, S3 the helpers, registries, jobs, dialogs,
menus and lists, then the widget themes.

- S1 done 2026-10-14: `errors`, `events`, `lifecycle`, `timer`, `combat`,
  `hooks`, `slash`, `saved` and `callbacks` (with `log`, `debug` and `reflect`
  from M5), written in wowlua and type-checked, so their exports are typed
  for importers. Their compile-time checks are E8001–E8003, E8010, E8020,
  E8021, E8030, E8100–E8104, E8112, E8117, W8011, W8040, W8101 and W8102.
  Tests run every addon in dev, release and fully minified builds against a
  mock client; the in-game
  suite (`tests/ingame`, `/stdtest`) passed on the WoW: Forever client,
  including combat lockdown and a `/reload` with real SavedVariables, and
  runs against the mock in CI. `examples/core-demo` uses every core module.
- Completed 2026-10-14: handlers are typed by their event or message
  (`on`, `once`, `onUnit`, `send`, `@event`, `@unitEvent`), hook targets are
  checked (W8040, and W8102 for globals of Blizzard's load-on-demand addons,
  marked `@onDemand` in the `wow-api` declarations), events missing from a
  shared-mode flavor warn (W8101; the declarations now list each flavor's
  own events, from client dumps), and `Secret<…>` in `saved.bind` defaults
  is E8104.
- S2 in progress: `locale` done 2026-10-14 (JSON locales checked against the
  base locale and compiled into guarded Lua files that load first, with
  `[AllowLoadTextLocale]` tags; `L` typed by the base locale); `pool` done
  2026-10-14 (frame and table pools; `FramePool.new` typed by the pooled
  class's constructor; protected frames released after combat).

## M9: Test runner, language server, formatter

- `wowlua test` with `wowlua:test` and `wowlua:mock`, plus coverage.
- `wowlua lsp` and the VS Code extension.
- Persistent incremental cache (phase 2), shared with the LSP.
- `wowlua fmt` complete for all wowlua constructs, with an idempotence suite.

**Exit:** the full edit, test, debug loop works without leaving the editor.

## M10: Registry

- Registry server (separate repository), `publish --target registry`,
  `package --registry`, registry sources in the resolver, owners and yanking.

**Exit:** libraries can be published and consumed by version requirement.

## M11: Performance optimization

WoW loads Lua source only, and its Lua 5.1 compiler barely optimizes, so the
compiler shapes the source the client compiles. Each optimization applies
only where the type checker proves its preconditions (plain tables without
metatables, numbers, no writes in between), so plain Lua and `any` values stay
as they are, and secure-code paths keep their call structure (taint).

- `wowlua bench`: benchmarks run on the embedded Lua 5.1 (WoW runs plain Lua
  5.1, not LuaJIT), comparing the optimized and the unoptimized build; every
  optimization is measured before it is kept.
- Loop-invariant field reads hoisted out of loops (`self.frame.texture` read
  once).
- Repeated field reads in one stretch of code read once into a local.
- `ipairs` loops over plain arrays become numeric loops.
- `table.insert(t, v)` on plain tables becomes `t[#t + 1] = v`.
- Upvalues read often in hot functions copied into locals when not written
  meanwhile.
- Closures that capture nothing from a loop created once, outside it.
- Inlining of statement bodies and private methods (decision T166 left them
  out of M7).
- Constant propagation of locals assigned once with a constant.

In-game performance tools, to measure what the optimizations change
([Debugging §7.7](language/07-debugging.md#77-dev-console), decision T181):

- The dev window in the wowlua design system with its fonts (T170, T171).
- Recordings: live, per combat, per boss pull and manual; comparison per
  minute (T172, T173).
- Frame time, slow calls, call paths, event attribution and garbage per
  function, with the profile settings `profile-call-paths`,
  `profile-garbage`, `frame-budget` and `slow-call` (T174).
- The performance views (functions, tree, flame, by event, timeline, slow,
  garbage, bench) and `/wl record`, `/wl bench`, `/wl hints`, `/wl hud`.
- Performance hints with `P` codes and their `wowlua explain` texts; the
  code generator learns the `P` prefix (T176).
- The compact HUD (T177) and the error toast (T180).
- Export through SavedVariables and `wowlua prof report` (T175, T179).

**Exit:** the benchmark suite shows a measured speed-up of the optimized
release build over the unoptimized one, the corpus and differential tests show
unchanged behavior, and an optimized addon is checked in game; a recording of
a boss pull, taken in game with the dev window, is exported and opened with
`wowlua prof report`.
