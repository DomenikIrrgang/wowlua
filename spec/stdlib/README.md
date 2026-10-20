# 8. Standard Library

The standard library is a set of **compiler-known modules** under the
`wowlua:` scheme. It covers functionality that appears in nearly every WoW
addon. Today that functionality is usually written by hand or embedded from
libraries such as Ace3, LibDBIcon or LibSerialize.

## 8.1 Principles

- **Import to use.** Nothing is in scope automatically. `import event from
  "wowlua:events"`.
- **Private copy, tree-shaken.** Standard-library modules are written in
  wowlua, shipped inside the compiler, and compiled into each addon like a
  `library` dependency. Only the modules (and module-internal code) an addon uses
  are included. Each addon has its own copy, so there are no cross-addon version
  conflicts. Modules communicate through the addon's private runtime, never
  through globals, except where the WoW API requires a global (§8.4).
- **Typed.** Every API is fully typed. Decorators are checked against event
  payloads, slash handler signatures, and so on. APIs whose checks go beyond the
  type system (settings keys, router arguments, callback registries) are
  intrinsic-checked by the compiler. The modules are checked in strict mode
  whatever the project's settings; what they share with the runtime
  (`ns.__wowlua`) is typed by the compiler's `WowluaRuntime` interface.
  Their objects are classes where they hold state (pools, callback registries,
  comm channels, slash routers, the options page, the built-in minimap button
  as a widget class), so an addon importing them includes the runtime's class
  slice.
- **Compile-time where possible.** Framework decorators generate registration
  code at compile time, and debug features disappear in release builds.
- **Flavor-aware.** Modules adapt to the flavor at compile time. A module that
  needs an API missing from a flavor offers a fallback (warning W8301 when the
  fallback is reduced), or reports error E8300 when used on that flavor.
- **Lifecycle-safe.** Modules that need saved data or the UI defer that work
  to the right lifecycle phase ([phase order](core.md#phase-order)), so they can
  be configured at module top level.
- **Versioned with the compiler.** The standard library version equals the
  compiler version. Breaking changes follow the compiler's SemVer.

## 8.2 Modules

| Module | Purpose | Chapter |
|--------|---------|---------|
| `wowlua:meta` | Addon metadata and build information | [Build metadata](../language/06-meta.md) |
| `wowlua:debug` | Assertions, dumps, profiling helpers | [Debugging](../language/07-debugging.md) |
| `wowlua:reflect` | Class identity and metadata | [Debugging](../language/07-debugging.md#78-reflection) |
| `wowlua:events` | WoW events and internal messages | [Core](core.md#events) |
| `wowlua:lifecycle` | Addon load, login and logout hooks, waiting for other addons | [Core](core.md#lifecycle) |
| `wowlua:slash` | Slash commands with argument parsing and subcommands | [Core](core.md#slash) |
| `wowlua:saved` | Typed SavedVariables: defaults, profiles, migrations | [Core](core.md#saved) |
| `wowlua:timer` | Timers, throttle, debounce | [Core](core.md#timer) |
| `wowlua:locale` | Typed localization | [UI](ui.md#locale) |
| `wowlua:pool` | Frame and table pools | [UI](ui.md#pool) |
| `wowlua:settings` | Declarative options pages | [UI](ui.md#settings) |
| `wowlua:launcher` | Minimap button, Addon Compartment, LDB launcher | [UI](ui.md#launcher) |
| `wowlua:combat` | Combat lockdown handling | [Advanced](advanced.md#combat) |
| `wowlua:secure` | Secure state drivers: visibility and states by macro conditionals | [Advanced](advanced.md#secure) |
| `wowlua:hooks` | Secure post-hooks, including deferred hooks | [Advanced](advanced.md#hooks) |
| `wowlua:comm` | Addon-to-addon messaging with chunking and throttling | [Advanced](advanced.md#comm) |
| `wowlua:serialize` | Serialization, compression, print-safe encoding | [Advanced](advanced.md#serialize) |
| `wowlua:log` | Leveled logging and chat output | [Advanced](advanced.md#log) |
| `wowlua:version` | Installed addon versions; newer versions seen in the guild and group | [Advanced](advanced.md#version) |
| `wowlua:secret` | Helpers for secret values | [Game data](game.md#secret) |
| `wowlua:unit` | Unit information across flavors | [Game data](game.md#unit) |
| `wowlua:aura` | Aura queries across flavors, secret-value aware | [Game data](game.md#aura) |
| `wowlua:spell` | Spell information across flavors | [Game data](game.md#spell) |
| `wowlua:item` | Item information with async loading and caching | [Game data](game.md#item) |
| `wowlua:tooltip` | Typed tooltip post-hooks | [Game data](game.md#tooltip) |
| `wowlua:frames` | Movable and resizable frames with saved position | [Interface](interface.md#frames) |
| `wowlua:bindings` | Keybindings (the compiler generates `Bindings.xml`) | [Interface](interface.md#bindings) |
| `wowlua:menu` | Context menus | [Interface](interface.md#menu) |
| `wowlua:list` | Scrolling lists on top of pools | [Interface](interface.md#list) |
| `wowlua:dialog` | Import/export and confirm dialogs | [Interface](interface.md#dialog) |
| `wowlua:color` | Colors, class colors, color escapes | [Interface](interface.md#color) |
| `wowlua:media` | SharedMedia interop for fonts, textures and sounds | [Interface](interface.md#media) |
| `wowlua:alert` | Sounds, raid warnings, taskbar flash | [Interface](interface.md#alert) |
| `wowlua:links` | Clickable links in the player's chat | [Interface](interface.md#links) |
| `wowlua:errors` | Error-handling policy and reporting | [Infrastructure](infrastructure.md#errors) |
| `wowlua:callbacks` | Callback registries and public APIs for other addons | [Infrastructure](infrastructure.md#callbacks) |
| `wowlua:jobs` | Background jobs spread across frames | [Infrastructure](infrastructure.md#jobs) |
| `wowlua:table` | Copies, merging, defaults, equality, list helpers, a stable sort, sorted lists | [Infrastructure](infrastructure.md#table) |
| `wowlua:collections` | Ring buffer, heap (priority queue), LRU cache | [Infrastructure](infrastructure.md#collections) |
| `wowlua:utf8` | Character counts, substrings and truncation of UTF-8 strings | [Infrastructure](infrastructure.md#utf8) |
| `wowlua:format` | Durations, dates, money and numbers in the client's formats | [Infrastructure](infrastructure.md#format) |
| `wowlua:test`, `wowlua:mock` | Testing (test files only, E8108 elsewhere) | [Tooling](../toolchain/tooling.md#161-test-runner-wowlua-test) |

## 8.3 Decorator index

| Decorator | Module | Target | Effect |
|-----------|--------|--------|--------|
| `@event(name, ...)` | `wowlua:events` | function, static method | Handles one or more WoW events or messages. Only WoW events are registered with the game. |
| `@unitEvent(name, ...units)` | `wowlua:events` | function, static method | `RegisterUnitEvent` for any number of units, on frames of its own (one per pair) |
| `@bucket(interval, name, ...)` | `wowlua:events` | function, static method | Coalesces bursts of events; the handler gets their counts |
| `@onLoad` | `wowlua:lifecycle` | function | Runs once this addon's SavedVariables are loaded (`ADDON_LOADED` for this addon) |
| `@onLogin` | `wowlua:lifecycle` | function | Runs on `PLAYER_LOGIN`, or right after `@onLoad` if already logged in (load-on-demand addons) |
| `@onEnterWorld` | `wowlua:lifecycle` | function | Runs on every `PLAYER_ENTERING_WORLD` |
| `@onLogout` | `wowlua:lifecycle` | function | Runs on `PLAYER_LOGOUT`, before `wowlua:saved` strips defaults |
| `@slash(cmd, ...)` | `wowlua:slash` | function | Registers a slash command |
| `@throttle(sec)` | `wowlua:timer` | function, method | At most one call per interval (leading edge, trailing call kept) |
| `@debounce(sec)` | `wowlua:timer` | function, method | Calls only after `sec` seconds of quiet |
| `@nextFrame` | `wowlua:timer` | function, method | Defers the call to the next frame and coalesces repeated calls |
| `@outOfCombat` | `wowlua:combat` | function, method | Runs immediately out of combat, otherwise queues until combat ends |
| `@hook(name)`, `@hook(target, name, opts?)` | `wowlua:hooks` | function | `hooksecurefunc` post-hook. `opts.addon` defers until that addon loads. |
| `@hookScript(frame, script, opts?)` | `wowlua:hooks` | function | `frame:HookScript(script, fn)` |
| `@test`, `@beforeEach`, … | `wowlua:test` | function | See [Tooling](../toolchain/tooling.md#161-test-runner-wowlua-test) |

Decorators of the game-data, interface and infrastructure modules are listed in
their chapters ([Game data](game.md), [Interface](interface.md),
[Infrastructure](infrastructure.md)).

Core decorators (`@profile`, `@trace`, `@debugOnly`, `@global`, `@if`, …) are
always in scope and always refer to the core decorator
([Decorators §4.3](../language/04-decorators.md#43-core-decorators)).

## 8.4 Globals created by the standard library

Standard-library modules create globals only where the WoW API requires one.
The normative list of every global the compiler or its output may create is in
[Code generation §14.1](../toolchain/codegen.md#141-target). The standard
library's entries are:

| Module | Globals | Why |
|--------|---------|-----|
| `wowlua:slash` | `SLASH_WOWLUA_<ADDON>_<N>1..n`, entries in `SlashCmdList` | WoW's slash command API |
| `wowlua:launcher` | Frame name `<Addon>MinimapButton` (built-in button only) | Minimap button managers find buttons by frame name |
| `wowlua:dialog` | Frame names `<Addon>Dialog<N>`, registered in `UISpecialFrames` | Escape closes dialogs (the standard WoW mechanism) |
| `wowlua:bindings` | `BINDING_HEADER_*`, `BINDING_NAME_*`, the dispatch table `<Addon>_Bindings` | Required by `Bindings.xml` |
| `wowlua:saved` | The manifest's SavedVariables (`_G[name]` is pointed at the bound table) | WoW saves SavedVariables from globals |
| `wowlua:version` | The shared table `WowluaVersions` (addon name → version) | One registry every addon using the module can read |

The chapters of the game-data, interface and infrastructure modules list any
further globals they need, and each is mirrored in the normative list.

## 8.5 Error handling

All standard-library callbacks (event handlers, lifecycle hooks, timers,
deferred calls, comm handlers, settings callbacks) follow one error policy,
defined by [`wowlua:errors`](infrastructure.md#errors):

- each callback runs isolated (`xpcall` with the policy's handler), so one
  failing handler never stops the others;
- errors are passed to `geterrorhandler()` (and so to BugSack and similar
  addons), with the addon name as prefix;
- a failing lifecycle hook does not stop the remaining hooks of its phase.

## 8.6 Teardown

WoW cannot unload an addon. Subscriptions, timers, hooks and registrations live
until the UI is reloaded, and none of them survive a reload. There is no
"disable" phase: an addon that wants to pause work must check its own state.
Hooks cannot be removed at all (a WoW limitation).
