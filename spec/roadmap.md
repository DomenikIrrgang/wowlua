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
  runs it against a mock frame API.
- Class, enum and decorator checks that need only declarations are reported
  now (decision T75); type-based ones come with M4.
- Native widget types are declared by the compiler until M4 (decision T77).
- No public standard-library modules yet: the infrastructure is checked with
  test modules through `WOWLUA_STDLIB_DIR` (decision T79).

## M4: Type checker

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

## M5: Debugging

- `wowlua:debug`, `wowlua:reflect`, `wowlua:log` (on the M3 stdlib
  infrastructure).
- Profiling, tracing, runtime checks, dev console.
- `wowlua trace`.

**Exit:** a profiling report appears in game, and runtime checks catch bad
values from the WoW API in a dev build.

## M6: Package manager

- Path, git and archive sources, lua-library embedding (reachable files and
  licenses only), lockfile, PubGrub resolution over the union, cache,
  workspaces with generated declaration files.
- `add`, `remove`, `update`, `fetch`, `tree`, `outdated`, `cache clean`,
  `fix --manifest`.
- Dependency safety (archive extraction rules, submodule policy).
- `.pkgmeta` import and compatibility mode, packager keywords.

**Exit:** an Ace3-based addon migrates with `wowlua init --from-pkgmeta` and
builds identically.

## M7: Optimization and publishing

- Bundling (with module isolation and XML-aware segments), minification passes
  and levels, global caching, inlining, dead-code removal, Lua-limit budgets.
- `package`, `publish` for CurseForge, Wago, WoWInterface and GitHub
  (`release.json`, archived source maps, `split-flavors`, `nolib`,
  `bundle-addons`), `login` with OS credential stores, changelog handling.
- `wowlua new --ci github` template.

**Exit:** a release is published to all four sites from CI with one command.

## M8: Standard library

- Core set first: `events`, `lifecycle`, `slash`, `saved`, `timer`, `combat`,
  `hooks`, `log`, error policy.
- Then `locale` (with the compiler's locale pipeline: JSON compilation, the
  `L` type, client-locale TOC directives), `pool`, `settings`, `launcher`,
  `comm`, `serialize`, keybindings (with `Bindings.xml` generation), tooltip
  hooks, movable frames.
- Then unit/aura/spell helpers, colors, SharedMedia interop, item/spell data
  cache, callback registry, background jobs, dialogs, menus and lists.

**Exit:** a sample addon uses no third-party libraries for its events, saved
variables, settings, minimap button, keybindings and comms.

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
