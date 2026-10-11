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

```wowlua
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

Design phase. The specification lives in [`spec/`](spec/README.md). The
implementation follows the [roadmap](spec/roadmap.md).

## License

[MIT](LICENSE)
