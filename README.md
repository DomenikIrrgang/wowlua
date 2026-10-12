# wowlua

**wowlua** is a language, compiler, framework and package manager for building
World of Warcraft AddOns.

- **Language.** `.wowlua` is a strict superset of Lua 5.1, the dialect WoW runs.
  Every Lua file is a valid wowlua file. It adds gradual types, classes,
  interfaces, enums, decorators, import/export modules, reflection,
  compile-time build information, and syntax sugar (`+=`, `continue`, `?.`,
  `??`, string interpolation, …). Everything compiles to plain Lua 5.1.
- **Compiler.** Written in Rust, shipped as a single binary. It type-checks your
  code and always emits a complete, installable AddOn: TOC files for every game
  flavor, embedded libraries, assets and only the runtime pieces you use.
  Release builds are bundled and minified to the level you choose, to keep
  in-game load times low.
- **Framework.** A typed standard library for the things almost every addon
  needs: events, saved variables, slash commands, timers, localization,
  settings, minimap button, addon comms, serialization, logging, and more.
- **Package manager.** A `wowlua.json` manifest describes the addon. Dependencies
  are resolved, locked and embedded. Finished addons are packaged and uploaded
  to CurseForge, Wago, WoWInterface and GitHub Releases with one command.

```lua
-- src/main.wowlua
import addon, build from "wowlua:meta"
import event from "wowlua:events"
import log from "wowlua:log"

export class Counter
    private count: number = 0

    @profile
    function increment(by: number = 1): number
        self.count += by
        return self.count
    end
end

local counter = Counter.new()

@event("PLAYER_LOGIN")
local function onLogin()
    log.info(`{addon.title} {addon.version} loaded ({build.profile} build)`)
    if build.debug then
        counter:increment()
    end
end
```

```sh
wowlua new MyAddon && cd MyAddon
wowlua dev               # watch, build and sync into your WoW AddOns folder
wowlua build --release   # bundled, optimized build
wowlua publish           # package and upload to every configured addon site
```

## Status

The specification lives in [`spec/`](spec/README.md). Implementation follows
the [roadmap](spec/roadmap.md).

**Milestones M0–M6 are done**: the toolchain builds and installs plain Lua
addons and compiles `.wowlua` files with type annotations (erased), syntax
sugar (`+=`, `continue`, `?.`, `??`, string interpolation, bitwise operators,
`//`, `const`, default parameters, destructuring), modules
(`import`/`export`), classes (abstract, generic, accessors), widget classes
that extend native frame types, enums, core and user decorators,
`wowlua:meta` with static conditions and `@if`, and per-flavor builds. The
load order is computed from imports and usage analysis of Lua, `.wowlua` and
XML files. The type checker (basic and strict mode, generics, narrowing,
flavor guards for shared multi-flavor builds) checks against the WoW API of
every flavor, generated from Blizzard's documentation. Dev builds have
debugging tools built in: a profiler for every function, an event, script
and timer monitor, error capture, runtime type checks and an in-game dev
window (`/wl`) with watches and a table inspector; `wowlua:debug`,
`wowlua:log` and `wowlua:reflect` are the first standard-library modules.
The package manager fetches git, SVN, path and archive dependencies into a
global cache, locks them in `wowlua.lock`, embeds Lua libraries (only the
files the game loads) and compiles wowlua libraries into the addon;
workspaces group addon suites, and BigWigs packager projects migrate with
`wowlua init --from-pkgmeta`: their builds match the packager's (same
loaded code, same shipped files, line endings and TOC versions), checked
on 36 addons including BigWigs, DBM, WeakAuras, ElvUI and Details.
The other standard-library modules arrive with later milestones; using a
feature of a later milestone reports that milestone. [`examples/widget-demo`](examples/widget-demo) and
[`examples/typed-demo`](examples/typed-demo) are sample addons.

What works today:

- `wowlua new <path>`: create a package (`--kind`, `--flavors`, `--ci github`).
- `wowlua init [--from-pkgmeta]`: adopt an existing addon. Its TOC(s) become
  `wowlua.json`, Lua/XML files move into `src/` and in-game paths to them are
  updated; `.pkgmeta` externals become dependencies, and the build matches
  the BigWigs packager's (`compat` settings, read from `.pkgmeta` and the
  project's CI workflows).
- `wowlua add` / `remove` / `update` / `fetch` / `tree` / `outdated` /
  `cache clean`: manage dependencies (`git`, `svn`, `path`, `archive`;
  the registry comes later). `-p <package>` selects a workspace member.
- `wowlua build [--release] [--flavor …]`: a complete AddOn in
  `dist/<profile>/<Name>/` with generated TOC(s) for every flavor, Lua 5.1
  syntax checks, XML reference checks and output validation. `.wowlua` files
  are compiled with their line numbers preserved (in-game errors point at the
  source line); source maps on request.
- `wowlua check`: type-check without writing a build (`.lua` files too with
  `check.lua`; `--!strict` or `check.mode` for strict mode).
- `wowlua lint [--fix]` and `wowlua fix`: unused bindings, shadowing and
  unreachable code, with machine-applicable fixes.
- `wowlua types convert <path>`: turn LuaLS `---@` annotations into
  `.d.wowlua` declarations.
- `wowlua trace [file] [--clipboard]`: map error messages and stacks from
  the game (error frame, BugSack) back to source lines and functions.
- In game, dev builds: `/wl` opens the dev window (profiler, monitor, log,
  errors, watches, inspector); `/wl prof|mon|errors|log|watch <addon>` print
  to chat.
- `wowlua fmt [--check]`: a StyLua-like, comment-preserving formatter
  (plain Lua syntax for now).
- `wowlua dev [--link]` / `install` / `uninstall`: sync builds into every WoW
  client folder (`_retail_`, `_classic_era_`, …), never overwriting folders
  that wowlua did not install.
- `wowlua toc`, `wowlua clean`, `wowlua explain <code>`.

### Installing

From a release (macOS, Linux):

```sh
curl -fsSL https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.ps1 | iex
```

From source (Rust 1.96+):

```sh
cargo install --path crates/wowlua-cli
```

### Developing

```sh
cargo test --workspace
cargo xtask codes    # regenerate diagnostic codes from spec/appendix/diagnostics.md
cargo xtask schema   # regenerate schema/wowlua-<version>.json
cargo xtask check    # verify generated files (CI)
cargo xtask corpus fetch && cargo run --release -p xtask -- corpus run
                     # conformance corpus: Lua 5.1 parity, bytecode round trip, formatter
cargo xtask corpus addons
                     # adopt, build and type-check the corpus addons (WOWLUA_CORPUS_FIX=1: also apply lint fixes)
cargo xtask types fetch && cargo xtask types gen
                     # regenerate the embedded wow-api declarations
```

## License

[MIT](LICENSE)
