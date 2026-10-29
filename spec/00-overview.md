# 0. Overview

## 0.1 What wowlua is

wowlua is a toolchain for writing World of Warcraft AddOns. It has four parts:

| Part | Responsibility |
|------|----------------|
| **Language** | `.wowlua`, a strict superset of Lua 5.1 with types, classes, enums, interfaces, decorators, modules, reflection and syntax sugar. |
| **Compiler** | `wowlua`, a single Rust binary. It parses, type-checks and lowers wowlua to plain Lua 5.1, then emits a complete, installable AddOn. |
| **Framework** | The `wowlua:*` standard library: WoW-specific functionality (events, saved variables, slash commands, localization, settings, comms, …) as typed, tree-shakeable modules. |
| **Package manager** | Built into the same binary. It reads the `wowlua.json` manifest, resolves and locks dependencies, embeds libraries, packages releases and uploads them to CurseForge, Wago, WoWInterface and GitHub. |

## 0.2 Goals

1. **Zero-cost adoption.** Every existing Lua addon file is valid wowlua. A
   project can rename `.lua` to `.wowlua` file by file and adopt features
   gradually.
2. **Catch bugs before `/reload`.** Gradual static typing catches typos,
   wrong arguments, accidental globals and nil errors at compile time.
3. **Always a working addon.** The compiler's output is always a complete,
   loadable AddOn folder. The TOC, libraries, runtime, assets and per-flavor
   variants are generated and validated, and nothing has to be assembled by hand.
4. **Fast in game.** Release builds are bundled, folded, tree-shaken and minified
   to the level configured by the project, to keep load times and memory low.
   Debug-only code costs nothing in release builds.
5. **First-class debugging.** Profiling, tracing, assertions, runtime type checks
   and error-trace decoding come built in and are switched on by the build profile.
6. **One tool.** Create, build, test, format, run in game, package and publish
   with a single `wowlua` binary.

## 0.3 Non-goals

- Targeting Lua versions other than WoW's Lua 5.1.
- Running outside WoW (except the test runner's mocked environment).
- Sound static typing. The type system is gradual and pragmatic, like
  TypeScript or Luau. `any` exists on purpose.
- Obfuscation. Minification exists to reduce load time. How far it goes is the
  project's decision per publish target (see [Optimization](toolchain/optimization.md)).

## 0.4 Design principles

- **Lua first.** New syntax looks like Lua: `end`-terminated blocks, `local`,
  `function`. Everything lowers to idiomatic, readable Lua unless a minification
  level says otherwise.
- **Explicit over magic.** Build information, debug helpers and framework
  features come from imported modules (`wowlua:meta`, `wowlua:debug`,
  `wowlua:events`, …). No identifiers are injected into user code. The one
  exception is a small set of core decorators (`@profile`, `@deprecated`, …),
  which are always available because the `@` namespace cannot clash with Lua.
- **No globals unless asked.** Modules live in the addon's private namespace
  (`select(2, ...)`). Globals are only created through `@global`, explicit `_G`
  access, plain Lua code that already does so, and the few documented stdlib
  registrations that WoW's APIs require (for example `@slash`). The normative
  list is in [Code generation §14.1](toolchain/codegen.md#141-target).
  (`declare global` only *describes* an existing global to the checker.)
- **Pay for what you use.** Runtime support code and standard-library modules are
  included only when referenced.
- **Deterministic.** The same inputs, compiler version and profile produce
  byte-identical output.

## 0.5 The superset guarantee

> **Every chunk accepted by the Lua 5.1 parser is accepted by the wowlua parser,
> and compiles to Lua with identical observable behavior.**

The design rules that make this hold (detailed in [Syntax](language/01-syntax.md)):

1. **New tokens only where Lua has none.** New tokens use characters or
   sequences that are lexical or syntax errors in Lua 5.1, such as `@`, `?`,
   `` ` ``, `::`, `->`, `+=`, `//`, `&` and `|`.
2. **Contextual keywords only in invalid-Lua positions.** New keywords (`class`,
   `import`, `export`, `type`, `const`, `enum`, `interface`, `declare`,
   `continue`, …) are *contextual*. They act as keywords only where Lua would
   report a syntax error, typically when followed by a name (`class Foo`,
   `import x`). Otherwise they remain ordinary identifiers: `class("Foo")`,
   `type(x)` and `local import = 1` keep their Lua meaning.
3. **No reinterpretation of existing syntax.** Quoted strings, numbers,
   operators and escapes keep their exact Lua 5.1 meaning. For example, `"\x41"`
   stays `"x41"` as in Lua 5.1. Extended escapes exist only in the new
   backtick strings.
4. **No breakage from the checker.** In a file that uses no wowlua syntax,
   every checker diagnostic (names, globals, modules, types, flavors, secret
   values) is a warning, never an error, unless that file itself is opted into
   strict mode by its own `--!strict` directive or a `check.strict` glob. A
   project-wide strict mode does not apply to such files
   ([Types §2.1](language/02-types.md#21-checking-modes)).
5. **No breakage from load order.** Files keep working when they share state
   through globals or the addon table: the load order is computed from imports
   *and* from usage analysis of globals and `ns` fields
   ([Modules §5.4](language/05-modules.md#54-load-order)).
6. **Hygienic generated code.** Compiler-generated identifiers never collide
   with user identifiers, and generated calls to standard globals cannot be
   captured by user bindings ([Syntax §1.4.7](language/01-syntax.md#147-generated-references-to-globals)).

## 0.6 Project anatomy

```
my-addon/
├── wowlua.json            # manifest (addon metadata, deps, profiles, publish targets)
├── wowlua.lock            # resolved dependency versions (generated, committed)
├── src/
│   ├── main.wowlua        # entry module
│   ├── core/
│   │   ├── events.wowlua
│   │   └── state.wowlua
│   ├── ui/
│   │   ├── frames.xml     # WoW UI XML, passed through
│   │   └── window.wowlua
│   ├── legacy.lua         # plain Lua, passed through (still syntax-checked)
│   └── types.d.wowlua     # ambient type declarations
├── locales/               # localization tables (see stdlib: wowlua:locale)
├── media/                 # textures, sounds, fonts (copied as assets)
├── tests/
│   └── state.test.wowlua
└── dist/                  # build output (generated)
    ├── dev/MyAddon/       # dev profile: one .lua per source, line numbers preserved
    └── release/MyAddon/   # release profile: bundled + minified per settings
```

Compiling produces a ready-to-install AddOn folder such as
`dist/release/MyAddon/`. It contains the generated `.toc` file(s), compiled Lua,
embedded libraries, the wowlua runtime (only what is used) and the assets.

## 0.7 A complete example

```wowlua
-- src/main.wowlua
import addon, build from "wowlua:meta"
import event from "wowlua:events"
import slash from "wowlua:slash"
import log from "wowlua:log"
import * as saved from "wowlua:saved"

interface Settings
    scale: number
    showMinimap: boolean
end

const DEFAULTS: Settings = { scale = 1, showMinimap = true }

local db = saved.bind("MyAddonDB", { version = 1, defaults = DEFAULTS })   -- db is Settings

export class Tracker
    private kills: number = 0

    @profile
    function record(amount: number = 1)
        self.kills += amount
        log.debug(`kills now {self.kills}`)
    end
end

local tracker = Tracker.new()

@event("PLAYER_LOGIN")
local function onLogin()
    log.info(`{addon.title} {addon.version} ({build.profile})`)
end

@slash("/mya")
local function command(msg: string)
    if msg == "reset" then
        db.scale = DEFAULTS.scale
    end
end
```

## 0.8 Terminology

| Term | Meaning |
|------|---------|
| **AddOn** | A folder WoW loads from `Interface/AddOns/`, described by a `.toc` file. |
| **Package** | A unit managed by the package manager, described by a `wowlua.json`. Kinds: `addon`, `library`, `lua-library`, `types`. |
| **Module** | A single `.wowlua` source file with its own scope, imports and exports. |
| **Flavor** | A WoW ruleset, identified by Blizzard's game type (`camelot` = WoW Forever, `standard` = Retail, `vanilla`, `tbc`). Each has its own API surface and interface version. |
| **Profile** | A named set of build settings (`dev`, `release`, `test`, or custom). |
| **Publish target** | A configured upload destination (CurseForge, Wago, WoWInterface, GitHub Releases, wowlua registry), each with its own packaging and minification settings. |
| **Lowering** | Translating wowlua constructs into equivalent Lua 5.1. |
| **Runtime** | The small Lua support library (`__wl`) emitted into an addon when generated code needs it. |
| **Compiler-known module** | A `wowlua:*` module whose exports the compiler understands and may fold, strip or expand at compile time. |
| **Pure Lua file** | A `.wowlua` file that uses no wowlua syntax, or a `.lua` file. |
