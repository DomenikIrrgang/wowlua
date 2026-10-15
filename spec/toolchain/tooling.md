# 16. Developer Tooling

## 16.1 Test runner (`wowlua test`)

Tests run outside the game, in wowlua's **emulation of the WoW client**
([WoW client emulation](emulator.md)): a Lua 5.1 VM written in Rust with WoW's
Lua differences, taint and secret values, and an engine with the flavor's WoW
API, optionally with Blizzard's own UI code. Like the game it has no `io`,
`os`, `require` or file access. The framework ships with wowlua; tests need no
third-party library (decision T184).

### 16.1.1 Writing tests

| Files | Role |
|-------|------|
| `tests/**/*.test.wowlua`, `src/**/*.test.wowlua` | Test files |
| `tests/mocks/**/*.wowlua` | Project mocks. Loaded before the addon, never treated as tests. |
| other `tests/**/*.wowlua` | Test helpers, importable from test files |

Test-only files are compiled with the `test` profile and never included in
addon builds. Importing `wowlua:test` or `wowlua:mock` from a non-test file is
error E8108.

```wowlua
-- tests/tracker.test.wowlua
import test, beforeEach, expect from "wowlua:test"
import * as mock from "wowlua:mock"
import Tracker from "/tracker"

local tracker: Tracker

@beforeEach
local function setup()
    mock.resetSpies()
    tracker = Tracker.new()
end

@test("records kills")
local function recordsKills()
    tracker:record(2)
    expect(tracker:total()).toBe(2)
end

@test("flushes on logout")
local function flushOnLogout()
    tracker:record()                       -- saved data is loaded: lifecycle already ran
    mock.fireEvent("PLAYER_LOGOUT")
    expect(MyAddonDB.kills).toBe(1)
end

@test("ticks every second")
local function ticks()
    local spy = mock.spy()
    tracker:startTicker(spy)
    mock.advanceTime(3.5)
    expect(spy).toHaveBeenCalledTimes(3)
end
```

### 16.1.2 What a test run loads

For each test file and flavor, the runner:

1. creates a fresh VM and installs `wowlua:mock` for that flavor,
2. loads the project mocks (`tests/mocks/`),
3. loads the **whole addon** in its computed load order, compiled with the
   `test` profile (exactly as the generated TOC would load it),
4. loads the test file (its top-level code runs),
5. unless the file opted out, fires the lifecycle: `ADDON_LOADED` for the addon,
   `PLAYER_LOGIN`, and `PLAYER_ENTERING_WORLD` (`isInitialLogin = true`),
6. runs `@beforeAll`, then each test with its `@beforeEach` / `@afterEach`
   hooks, then `@afterAll`.

A test file opts out of step 5 with `config({ lifecycle = "manual" })` from
`wowlua:test` at its top level, and then drives the lifecycle itself with
`mock.login()`. Tests within one file share the VM. Each file starts fresh.

### 16.1.3 `wowlua:test`

| Export | Purpose |
|--------|---------|
| `@test(name?)` | Marks a test function (top level) |
| `@beforeEach`, `@afterEach`, `@beforeAll`, `@afterAll` | Hooks, per file |
| `@skip(reason?)`, `@only` | Test selection |
| `@flavors("standard", …)` | Run only for these flavors |
| `config({ lifecycle: "auto" \| "manual" })` | Per-file runner settings |
| `describe(name, fn)` | Grouping for runtime-generated tests |
| `expect(v)` | Matchers: `toBe`, `toEqual` (deep), `toBeNil`, `toBeTruthy`, `toBeFalsy`, `toBeInstanceOf`, `toContain`, `toHaveLength`, `toBeCloseTo`, `toThrow`, `toHaveBeenCalled`, `toHaveBeenCalledWith`, `toHaveBeenCalledTimes`. Negation with `expect(v).never.toBe(x)`. |

### 16.1.4 `wowlua:mock`

`wowlua:mock` is a **standard-library module**, versioned with the compiler. It is
a Lua implementation of the parts of the WoW API that addons and the standard
library touch. It follows the flavor's API: functions that don't exist in the
flavor under test are absent, as in the game.

| Area | Mocked |
|------|--------|
| Frames | `CreateFrame` with scripts, events (`RegisterEvent`, `RegisterUnitEvent` with unit filters), show/hide, points, sizes, text, parent/child hierarchy, `Mixin`, `CreateFromMixins`, protected frames, common templates as no-ops |
| Events | `mock.fireEvent(name, ...)`, `mock.fireUnitEvent(name, unit, ...)`. Lifecycle via `mock.login()` (`ADDON_LOADED`, `PLAYER_LOGIN`, `PLAYER_ENTERING_WORLD`). `IsLoggedIn`. |
| Addons | `C_AddOns.IsAddOnLoaded`, `mock.loadAddon(name)` fires `ADDON_LOADED` for another addon (deferred hooks, `onAddonLoaded`) |
| Time | `GetTime`, `GetTimePreciseSec`, `debugprofilestop`, native-style `C_Timer` handles driven by a virtual clock (`mock.advanceTime(sec)`), `OnUpdate` dispatch per simulated frame (`mock.nextFrame()`, `mock.advanceFrames(n, msPerFrame?)` with a controllable `debugprofilestop` for `wowlua:jobs`) |
| Combat | `InCombatLockdown` with `mock.setInCombat(bool)`, including the regen events and blocked protected-frame calls |
| Units, auras, spells, items | `UnitName`, `UnitClass`, `UnitHealth`, aura and spell queries from `mock.setUnit(token, data)`, `mock.setAuras(token, list)`, `mock.setSpell(id, data)`, `mock.setSpellCooldown(id, info)`, `mock.fireUnitAura(unit, changes?)`, `mock.setGroup(members)`. `mock.setItem(id, data, { cached = bool, delay = n })` simulates asynchronous item data (`GET_ITEM_INFO_RECEIVED`). |
| Secret values | `mock.secret(v)` creates a value for which `issecretvalue` / `canaccessvalue` behave like secrets, and forbidden operations on it raise errors. `mock.setRestricted(bool)` makes secret-capable APIs return secrets. |
| Saved variables | Read from and written to an in-memory store. `mock.reloadUI()` simulates persistence across a reload. |
| Slash commands | `mock.slash("/mya reset")` runs the registered handler |
| Keybindings | Generated bindings are registered. `mock.pressBinding(name)` runs one. |
| Chat / addon comms | `C_ChatInfo.RegisterAddonMessagePrefix`, `SendAddonMessage` loopback through `CHAT_MSG_ADDON` with size limits, `mock.setMessagingLockdown(bool)`, captured `print` and chat frame output |
| Settings and launcher | `Settings.*` registration (inspectable categories and settings), `AddonCompartmentFrame`, `Minimap`, `GameTooltip`, `TooltipDataProcessor` |
| Tooltips | `mock.showTooltip(kind, data)` runs tooltip post-hooks on a stub tooltip whose added lines are readable with `tip:lines()` |
| Frames and layout | `mock.drag(frame, dx, dy)`, `mock.setScreen(w, h)` for movable frames; `mock.scroll(view, offset)` for lists |
| Menus and dialogs | `mock.lastMenu()`, `mock.clickMenu(path)`, `mock.dialog()`, `mock.dialogAccept(value?)`, `mock.dialogCancel()` |
| Colors | `CreateColor` and the class color tables |
| Libraries | `LibStub` present or absent. `mock.provideLibrary(name, impl)` simulates LibDBIcon, LibDataBroker, LibSharedMedia or ChatThrottleLib being loaded by another addon. |
| Locale | `GetLocale` with `mock.setLocale(code)` (set before loading, via the file's `config`) |
| Errors | `geterrorhandler` / `seterrorhandler`, `securecallfunction`. `mock.errors()` returns the errors reported so far. |
| Lua environment | WoW's Lua additions: `strsplit`, `strsplittable`, `strjoin`, `strtrim`, `strconcat`, the `str*` aliases, `format`, `wipe`, `tinsert`, `tremove`, `tContains`, `CopyTable`, `bit`, `date`, `time`, `debugstack` |
| Spies | `mock.spy(fn?)`, `mock.stub(table, key, fn)`, `mock.resetSpies()` |

- Unmocked API functions raise `"<name> is not mocked"`, which makes gaps visible.
- Projects add or override mocks in `tests/mocks/`.
- **Reloads.** `mock.reloadUI()` discards all addon state (modules, frames,
  subscriptions, timers, hooks) and keeps only the SavedVariables store. It then
  re-runs the load sequence and lifecycle, mirroring a real `/reload`.

Mocking (decision T187):

- **Spies, stubs and fakes** for any function: `mock.spy(fn?)`,
  `mock.stub(table, key, fn)`, `mock.fake(name, impl)` for an engine function,
  restored after each test.
- **Scenario controls** set the game's state: combat, units, group, auras,
  spells, items, time, screen, locale, CVars, other addons and libraries.
- **Taint in tests.** A protected action refused because of taint fails the
  test with its taint chain ([WoW client emulation §17.4](emulator.md#174-taint-and-secure-execution));
  `expect(fn).toBeBlocked()` asserts that it is refused, and
  `expect(value).toBeSecure()` that a variable is untainted.

### 16.1.5 Running

```
wowlua test [filter] [--flavor <f>] [--watch] [--coverage] [--reporter <r>,…] [--output <dir>]
```

- Each test file runs once **per manifest flavor** by default. `--flavor`
  restricts the run, and `@flavors` restricts individual tests.
- Each file and flavor runs in a fresh VM. Files run in parallel.
- Test reporters: `pretty` (the terminal default), `junit` (XML), `tap`,
  `json`, `github` (annotations). Several may be given; files go to
  `dist/test/` unless `--output` says otherwise.
- Exit code 1 if any test fails, or if coverage is below a threshold
  (§16.1.6).

### 16.1.6 Coverage

`--coverage` measures, mapped to the `.wowlua` and `.lua` sources
(decision T188):

| Kind | Counts |
|------|--------|
| Lines and statements | Each statement executed |
| Branches | Each arm of `if`/`elseif`/`else` (the implicit "no else" too), loops entered or skipped, both outcomes of `and`, `or`, `??` and `?.` |
| Functions | Each function, method and closure called |
| Conditions | Each condition of a compound test (`a and b`, `a or b`) both true and false |

The VM counts them; the compiled code is not changed. Code that `@if` removes
for the flavor under test counts as not applicable, and coverage of all flavors
is merged unless `--coverage-per-flavor` is given. `-- wowlua: coverage ignore`
(next statement) and `-- wowlua: coverage ignore-file` exclude code, and
`test.coverage.exclude` excludes files.

Reports, any combination: a terminal summary, `lcov` (with branch and function
data), `cobertura` (XML), `json`, and `html`: a self-contained report in the
wowlua design system (its fonts and tokens, dark and light), with every file's
source, hit counts, branch markers and totals per file and folder.

Thresholds in `test.coverage` ([Manifest §9.18](manifest.md#918-test)) set
minimum percentages per kind; a run below one fails.

## 16.2 Language server (`wowlua lsp`)

A Language Server Protocol implementation over stdio, shipped in the same
binary. It shares the parser, binder and checker with the compiler.

| Feature | Details |
|---------|---------|
| Diagnostics | All compiler diagnostics, live, per flavor and profile (both switchable from the editor) |
| Hover | Types, documentation comments, class members, `wowlua:meta` folded values |
| Completion | Locals, imports with auto-import, members, string literal unions (events, anchors, frame types), decorator names, manifest keys in `wowlua.json` |
| Navigation | Go to definition (including into `.d.wowlua` and dependencies), find references, document and workspace symbols |
| Refactoring | Rename (scope-aware), organize imports, quick fixes from diagnostic `fixes` |
| Signature help | Including overloads |
| Inlay hints | Inferred types of locals and parameters, parameter names |
| Semantic tokens | Distinguishes contextual keywords from identifiers |
| Formatting | Via the formatter (§16.3) |
| Code lens | Profiler and test shortcuts (run test, run file) |

**Configuration discovery.** For each open file, the server uses the nearest
`wowlua.json` found by walking up from the file. If that manifest belongs to a
workspace, it uses the workspace root manifest. The selected flavor and profile
are stored in the editor's settings. The server never accesses the network
unless the editor setting `wowlua.lsp.fetch` allows it. Missing dependencies
produce a diagnostic suggesting `wowlua fetch`.

A VS Code extension (TextMate grammar plus LSP client) is part of the project.
Other editors use the LSP directly.

## 16.3 Formatter (`wowlua fmt`)

An opinionated formatter for `.wowlua` and `.d.wowlua`. It can optionally format
`.lua` with `fmt.lua = true`.

- Idempotent, and preserves comments (it is built on the lossless trivia kept by
  the parser).
- Configuration is deliberately small, in the manifest:
  `"fmt": { "indent": "tab" | 2 | 4, "line-width": 100, "quote-style": "double" | "single" | "preserve" }`.
  Defaults: `indent` `"tab"`, `line-width` 100, `quote-style` `"double"`
  ([Manifest §9.13](manifest.md#913-fmt)).
- `wowlua fmt --check` exits 1 if any file would change (for CI).
- Style (decision T65), following StyLua: lines are reflowed at `line-width`;
  call arguments are always parenthesized (`f "x"` becomes `f("x")`); tables
  print as `{ a = 1, b = 2 }` when they fit, otherwise one field per line with
  a trailing comma; a single table or function argument, or a function as the
  last argument, hugs the parentheses; long binary expressions break after
  the operator with a hanging indent; blocks are never kept on one line,
  except an empty function body (`function() end`); at most one blank line is
  kept; `;` is written only before a statement that starts with `(`.
  Trailing comments stay on their line, comments on their own line stay on
  their own line.
- Formatting never changes semantics. Every formatted file is verified to
  produce the same AST.

## 16.4 Trace decoder (`wowlua trace`)

Maps error messages and stack traces from bundled or minified builds back to
source locations, using the source maps of the matching build
([Code generation §14.6](codegen.md#146-source-maps)).

```
$ wowlua trace < bugsack.txt
MyAddon/MyAddon.lua:4821: attempt to index local 'a' (a nil value)
  → src/ui/window.wowlua:87 in Window:refresh   ('a' is 'self.frame')
[string "@Interface/AddOns/MyAddon/MyAddon.lua"]:5310: in function <MyAddon/MyAddon.lua:5302>
  → src/main.wowlua:22 in onLogin
```

- WoW error messages and stacks contain **line numbers only**, never columns.
  Every minification level keeps at least one line break per statement
  ([Optimization §15.4](optimization.md#154-minification-levels)), so a line
  always identifies one source statement.
- Input: pasted text, a file, or `--clipboard`. It recognizes WoW error-frame
  and BugSack formats.
- Build matching: the build id (`--build`, the `X-Wowlua-Build` TOC field,
  also included in the load errors of bundles) selects the right source maps:
  a build in `dist/` with that id, else the archive. Without one, the newest
  build in `dist/` is used. Maps of published builds
  are archived as described in [Code generation §14.6](codegen.md#146-source-maps),
  and can be uploaded as release assets (`publish.archive-maps`). That way,
  reports from users of old versions can still be decoded.
- Renamed locals are reported with their original names, looked up by line and
  minified name.
- Line-preserved builds need no map. `wowlua trace` passes their locations
  through, rewriting the output path (including any flavor directory) to the
  `.wowlua` source file, and names the enclosing function found in the source
  (`in Window:refresh`).
- Locations of other addons are left as they are.

## 16.5 Profiling reports (`wowlua prof report`)

`wowlua prof report` turns recordings exported from the dev window
([Debugging §7.7.9](../language/07-debugging.md#779-export)) into a single,
self-contained HTML page and opens it in the browser (`--no-open` only writes
it). The default output is `dist/reports/<Name>-<date>.html`; `--out` sets
another path.

- **Finding the data.** The command reads `WowluaDevDB_<Addon>.lua` of the
  current package from the SavedVariables folders of the WoW installation,
  found as for `wowlua dev` ([CLI §12.5](cli.md#125-wow-installation-integration)):
  `--wow`, `WOWLUA_WOW_PATH`, the global config, then auto-detection. Every
  client folder and every account under `WTF/Account/` is searched. When more
  than one has exported recordings, the command lists them and asks, or takes
  `--client` and `--account` (decision T179).
- **What it shows.** Every exported recording (`--recording` selects some),
  with the functions table, the call tree and flame graph when call paths
  were on, the frame timeline with the frame budget, the slow calls with
  their stacks, the garbage view and the hints found in that recording, in
  the style of the wowlua website. Locations are mapped back to source lines
  as `wowlua trace` does (§16.4), using the build id stored with each
  recording.
- **Nothing is uploaded.** The report is a local file; the command does not
  use the network.
