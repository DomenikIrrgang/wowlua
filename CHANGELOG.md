# Changelog

All notable changes to wowlua. Each minor version closes a milestone of the [roadmap](spec/roadmap.md).

## Unreleased

### Added

- A field named by a string key may be optional: `{ ["not"]?: boolean }`
- A function over the 60-upvalue limit reads its excess imports through the module registry, as the spec describes, instead of failing with E5001

### Fixed

- `export const x: T = { … }` and an annotated plain `const` have the annotation's type, not the literal's
- An exported table has the fields assigned to it after its declaration (`function chat.Hide() … end`), sealed, for importers
- A callback taking a wider parameter type no longer widens a type argument (`table.sort(rows, function(a: { ms: number }, …) … end)`)
- Namespaces, enum tables, class tables and instances are tables for `{ [any]: any }`, so `pairs(Enum.X)` and `pairs(C_X or {})` check
- A list literal fits a record with a numeric indexer and optional named fields (`{ [number]: any, talent?: boolean }`)
- A table's own method replaces the one `setmetatable`'s `__index` gives it
- Reading or writing a field of a function, number or boolean is E2003 with any key, not only a `.` read
- No false W1003 for a forward-declared local that a module's `function name()` defines
- E2001 names the missing or mismatched field of a record (and hints at `x?: T`), and the first missing return value; E2015 names the union member that does not fit; a function passed to `pcall` with too few arguments is E2007 naming the argument

## [0.9.1] - 2026-10-20

M9 so far, released before the milestone closes: wowlua's emulation of the WoW
client, and the test framework that runs on it. The language server, the VS
Code extension and the milestones after M9 are planned; see the
[roadmap](spec/roadmap.md).

### Added

- `wowlua-vm`, a Lua 5.1 VM in Rust: the corpus compiles to `luac`'s bytecode and lines, and the Lua 5.1 test suite passes (S1)
- WoW's Lua environment in the VM: taint and secure execution with taint reports, secret values, the client's limits and call depths (S2)
- `wowlua-engine`: the WoW API from Blizzard's documentation with its argument checks, frames, events, timers, units, auras and protected actions; Blizzard's UI code loads as secure code, and on WoW: Forever it logs in and runs without an error, as the client's does (S3)
- `wowlua test`: a package's test files run per flavor in the emulated client, with the addon loaded from its TOC as the client loads it; `test.blizzard-ui` loads Blizzard's UI first, `test.addons` other addons' folders, each as its own addon; `--watch` ([docs](docs/getting-started.md#test-it), S4)
- `wowlua:test`: tests, hooks, `@skip`, `@only`, `@flavors`, `@cases`, `describe`, typed matchers, widget matchers, snapshots (`-u` updates them), custom matchers and compile-time tests with `@expectError` (S4)
- `wowlua:mock`: units, combat, time and events of a simulated world, spies, stubs and fakes, waiting, printed output and errors, `mock.module`, user input (click, hover, drag, typing, key bindings), casting, vendors, `mock.reloadUI`, and taint assertions (`toBeBlocked`, `toBeSecure`) (S4)
- World profiles: a test file starts with the character, money, gear, bags, place, auras and units of `tests/profiles/<name>.json`; `/stdtest snapshot` records one in the client (S4)
- `import internal` reaches a module's private declarations from tests (S4)
- Coverage and reports: `wowlua test --coverage` (lines, statements, branches, functions, conditions), `test.coverage` minimums, and the `junit`, `tap`, `json`, `github`, `lcov`, `cobertura`, `coverage-json` and `html` reporters (S5)
- The three examples have tests
- The standard library's tests run in the VM, on the engine
- `-j, --jobs` (`WOWLUA_JOBS`, `build.jobs`) and `build.jobs-memory`: the worker threads of a build and the memory they may take
- Lua compile results are kept between runs in the cache (`build.compile-cache`, `build.compile-cache-size`, `WOWLUA_NO_COMPILE_CACHE`)
- The emulated client answers WoW: Forever's whole native API and its widget methods; answers come from client recordings where there are any, from models of the game's state (bags, mail, quests, auras, the group finder…) and otherwise from Blizzard's documentation (S4)
- Twenty-four recorded client scenarios replay in the engine (bank, vendor, mail, auction, taxi, talents, professions, quests, combat, death, the group finder, repair, bags, the quest log…), checked line by line against the client (S4)
- SuuUI, Ellesmere UI and ElvUI run in `wowlua test` with Blizzard's UI as in the client, ElvUI with exactly the client's errors (S4)
- `/stdtest engine answers`, `/stdtest engine fullprobe` and `/stdtest engine methodprobe` (every widget method on fresh hidden widgets) ask the client the engine's open questions; `cargo xtask ingest` turns the answers into the engine's data (S4)

### Changed

- `wowlua fmt` formats all of wowlua's syntax (S6)
- Builds parse, lower, check, lint and walk the load order's call graph on worker threads
- Builds take less memory: smaller syntax trees and IR, each source's text kept once, source maps kept encoded
- `wowlua dev` rebuilds once per batch of changes, keeps compiled files between builds, does not parse unchanged plain Lua files again and writes only the files that changed
- Runs end with a count of errors and warnings; a source line longer than 160 characters is shown as a window around its labels
- Load order: no W1033 for files whose order `build.load-order` or an XML file fixes
- Load order: a load-on-demand addon may read its dependencies' tables while it loads without W1034
- Load order: branches that run only when an argument is not a function no longer count for callers that pass one (fewer W1035)

### Fixed

- Long operator chains build without a stack overflow or quadratic time, also on Windows
- `check.lints` accepts the warning forms of name and type errors (`"W2001": "allow"`)
- `wowlua dev` ignores the output folder `build.out` names, not only `dist/`
- Field assignments of an annotated variable are checked against its declared type

## [0.9.0] - 2026-10-15

M8, the standard library: `wowlua:` modules written in wowlua, typed, and
tested in game on WoW: Forever. `examples/core-demo` uses no third-party
libraries for its events, saved variables, settings, minimap button,
keybindings and comms.

### Added

- Core: `events`, `lifecycle`, `timer`, `combat`, `hooks`, `slash`, `saved`, `callbacks`, `errors`, `jobs`, `table`, `format`
- Interface: `locale` (JSON translations, a typed `L`), `pool`, `settings`, `launcher`, `frames` (with Edit Mode), `bindings`, `menu`, `list`, `dialog`, `color`, `media`, `alert`, `links` and widget themes
- Game data, secret-value aware: `secret`, `unit`, `aura`, `spell`, `item`, `tooltip`; and `secure`, `serialize`, `comm`, `version`
- `wowlua compile`: files to Lua without an AddOn folder
- WoW's Lua environment as probed in game, with W2060 for a literal zero divisor

### Changed

- Builds are about 5x faster with 2–3x less memory on large multi-flavor addons
- The runtime and the mock client are written in wowlua

### Fixed

- Variable-file folders load only in their own flavor
- Load order: shared files are not loaded by the shipping addon, and API functions do not call their arguments
- Declared table types bind exactly in generic calls

## [0.8.2] - 2026-10-13

### Fixed

- Load order: fields of global and addon tables (`ns.Tools.Run`) order files too ([#1](https://github.com/DomenikIrrgang/wowlua/issues/1))

## [0.8.1] - 2026-10-13

M7's exit criterion is met: SuuUI, a WoW: Forever addon adopted with
`wowlua init`, was published from CI to CurseForge, Wago and GitHub with one
tag push.

### Changed

- CurseForge, Wago and WoWInterface targets build unminified unless the target sets `minify`
- W6010 warns when an addon-site target publishes minified code, no longer for GitHub targets

## [0.8.0] - 2026-10-13

M7, optimization and publishing: bundled, minified and optimized release
builds, and `wowlua publish` to four addon sites.

### Added

- `wowlua package`: target builds and reproducible zips
- `wowlua publish` to CurseForge, Wago, WoWInterface and GitHub
- `wowlua login` / `logout` with the OS credential store
- Bundling with segments, bundle source maps, and `wowlua trace` through maps
- Minification passes and levels, with W6010 for renaming levels on publish targets
- Inline, cache-globals and dead-code optimizations
- `build --stats`, shared bundles and script-only XML flattening
- `publish.bundle-addons`; member targets stay with their project

### Changed

- The CI template publishes on tags; `init` shows how to switch packager workflows
- Optimizations never break Lua limits

### Fixed

- `publish` tags without a git identity
- Packager token variables are read from `.env` only

## [0.7.0] - 2026-10-12

M6, the package manager. Addons built with the BigWigs packager migrate with
`wowlua init --from-pkgmeta` and build the same, checked on thirty-six addons.

### Added

- Git, SVN, path and archive sources, a global cache and `wowlua.lock`
- `add`, `remove`, `update`, `fetch`, `tree`, `outdated`, `cache clean`
- Workspaces for addon suites, with generated declaration files
- `.pkgmeta` import and packager compatibility mode
- Per-flavor load orders, SavedVariables and publish IDs; external addons

### Fixed

- Shipped files compare case-insensitively, like the game's file systems
- SVN trunk libraries on a tag take the tag's revision, like the packager
- Packager parity for TOC versions, line endings and shipped files

## [0.6.0] - 2026-10-11

M5, debugging: `wowlua:debug`, `wowlua:reflect` and `wowlua:log`; profiling,
tracing and runtime checks; the in-game dev window (`/wl`); `wowlua trace`.

### Added

- `wowlua:debug`, `wowlua:reflect` and `wowlua:log`
- Profiling, tracing and runtime type checks
- Dev builds with every tool built in: every function profiled, events, scripts and timers monitored, errors captured
- The dev window (`/wl`) with profiler, monitor, log, errors, watch and inspector tabs, and `/wl` chat commands
- `wowlua trace` for line-preserved builds

## [0.5.0] - 2026-10-11

M4, the type checker: basic and strict mode, generics, narrowing, flavor
guards, `wowlua lint` and `wowlua fix`.

### Added

- Basic mode: inference, narrowing, assignability, arity, globals, classes, interfaces, generics, overloads
- Strict mode and `Secret<T>`
- The WoW API of every flavor, generated from Blizzard's documentation
- Declaration files and `wowlua types convert` for LuaLS annotations
- `shared` flavor mode with flavor guards (E2030)
- `wowlua lint` and `wowlua fix`

## [0.4.0] - 2026-10-11

M3: classes, widget classes, enums and decorators, and the standard-library
infrastructure.

### Added

- Classes (abstract and generic), widget classes, interfaces (parsed) and enums
- Core decorators (`@global`, `@deprecated`, `@if`, `@inline`) and user decorators
- Standard-library infrastructure: `wowlua:*` modules compiled into addons and tree-shaken
- `examples/widget-demo`

## [0.3.0] - 2026-10-11

M2: lowering of syntax sugar and modules.

### Added

- `+=`, `continue`, `?.`, `??`, string interpolation, bitwise operators, `//`, `const`, default parameters and destructuring
- `import` / `export` modules
- Load-order analysis of globals and addon-table fields, per flavor
- `wowlua:meta`, static conditions, `@if`, flavor split builds and XML reference rewriting

## [0.2.0] - 2026-10-11

M1: the parser and the emitter.

### Added

- A parser for Lua 5.1 and all of wowlua's syntax, with error recovery
- The Lua emitter, with line modes and source maps
- `.wowlua` files compile with their type annotations erased
- `wowlua check` (syntax only) and a first `wowlua fmt` for Lua 5.1

## [0.1.0] - 2026-10-11

M0, the foundation: an existing Lua addon builds and installs from a
`wowlua.json`, with generated TOCs for several flavors.

### Added

- `new`, `init` (from a TOC), `build` for plain Lua projects (TOC generation, assets, `LICENSES.txt`, output validation), `install`, `uninstall`, `dev` (copy and link modes, safe installs), `clean`, `toc` and `explain`
- `.wowlua` files that are pure Lua 5.1
- `wowlua.lock` with the pinned flavor table
- Release binaries for macOS, Linux and Windows on GitHub Releases

[0.9.1]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.9.1
[0.9.0]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.9.0
[0.8.2]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.8.2
[0.8.1]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.8.1
[0.8.0]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.8.0
[0.7.0]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.7.0
[0.6.0]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.6.0
[0.5.0]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.5.0
[0.4.0]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.4.0
[0.3.0]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.3.0
[0.2.0]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.2.0
[0.1.0]: https://github.com/DomenikIrrgang/wowlua/releases/tag/v0.1.0
