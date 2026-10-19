# Getting started

Create an addon, run it in the game while you edit it, and test it outside the
game. This takes a few minutes. You need wowlua [installed](install.md) and,
for the in-game part, a WoW client on the same machine.

## Create an addon

```sh
wowlua new MyAddon
cd MyAddon
```

```text
     Created addon package in MyAddon
```

`--kind` picks the template (`addon`, `library`, `lua-library` or `types`),
`--flavors` the game flavors to build for (default `standard`, the retail
client), and `--ci github` adds a GitHub Actions workflow that builds pushes
to `main` and pull requests, and publishes `v*` tags. When `git` is available and the folder is not inside
a repository yet, `new` also runs `git init`.

### What `wowlua new` creates

```text
MyAddon/
├── wowlua.json            manifest: addon metadata, flavors, dependencies, publish targets
├── src/
│   └── main.wowlua        the addon's code
├── tests/
│   └── main.test.wowlua   tests, never part of a build
├── locales/
│   └── enUS.json          translations for wowlua:locale
├── CHANGELOG.md
├── README.md
└── .gitignore             ignores dist/
```

The first build adds `wowlua.lock`, which pins the interface numbers and
dependency versions. Commit it. Builds go to `dist/`, which is generated:
`dist/dev/MyAddon/` holds one `.lua` file per source with line numbers
preserved, and `dist/release/MyAddon/` the bundled release build.

The manifest starts small:

```json
{
  "$schema": "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/wowlua-0.1.json",
  "package": {
    "name": "my-addon",
    "version": "0.1.0",
    "authors": [
      "Your Name"
    ],
    "wowlua": "^0.9"
  },
  "addon": {
    "title": "MyAddon"
  }
}
```

`authors` comes from your git `user.name`. There is no TOC file to edit:
wowlua generates one for every flavor from `wowlua.json`. The
[manifest reference](../spec/toolchain/manifest.md) lists every key.

## Run it in the game

```sh
wowlua dev
```

```text
       Built MyAddon 0.1.0-dev (dev: standard) in 0.06s
   Installed /Applications/World of Warcraft/_retail_/Interface/AddOns/MyAddon
       Ready type /reload in game to load the changes
```

`wowlua dev` builds with the `dev` profile and copies the AddOn into every
client folder of your WoW installation that matches a built flavor
(`_retail_`, `_ptr_`, `_classic_era_`, …). Then it watches your files. Each
time you save, it builds again, compiling only what changed, and syncs the
result.

- It finds the installation from `--wow <path>`, the `WOWLUA_WOW_PATH`
  environment variable, `wow.path` in `~/.wowlua/config.json`, or the
  Battle.net default folder: on macOS and Windows, and on Linux in a Wine
  prefix (Lutris's `~/Games/battlenet`, or `~/.wine`). Elsewhere, set one
  of the first three.
- It never overwrites an AddOn folder it did not install, such as the same
  addon from an addon manager. That is error E4331, unless you pass `--force`.
- A failed build leaves the installed AddOn alone, so the last working version
  stays in the game.
- `--link` links the AddOn folder to the build folder instead of copying it.
- `wowlua install` builds once and installs, and `wowlua uninstall` removes
  what wowlua installed.

Log in, or type `/reload` if the game is already running. WoW only loads Lua
again on `/reload`, so reload after each change.

The details are in [CLI §12.5](../spec/toolchain/cli.md#125-wow-installation-integration).

## Your first edit

Replace `src/main.wowlua` with a module that prints a line in the chat when the
player logs in:

```wowlua
import addon, build from "wowlua:meta"
import event from "wowlua:events"
import print as chat from "wowlua:log"

@event("PLAYER_LOGIN")
local function onLogin()
    chat(`{addon.title} {addon.version} loaded ({build.profile} build)`)
end
```

Save, then `/reload` in the game. The chat shows `MyAddon 0.1.0 loaded (dev
build)` after the addon's colored name.

`wowlua:meta` holds the addon's metadata and build information, folded into
the output at compile time. `@event` registers the function for the event.
`print` from `wowlua:log` writes to the chat frame with the addon's name in
front. The dev build keeps each line where it was, so an in-game error in
`src/main.lua` line 7 is line 7 of `src/main.wowlua`:

```lua
local __wl = select(2, ...).__wowlua
local __wl_m1 = __wl.require("wowlua:events") local event = __wl_m1.event
local __wl_m2 = __wl.require("wowlua:log") local chat = __wl_m2.print

local function
onLogin()
	chat("MyAddon 0.1.0 loaded (dev build)")
end onLogin = __wl.prof.wrap(onLogin, "onLogin", "src/main.wowlua:6") __wl.register(onLogin, {kind = "function", name = "onLogin", static = false, file = "src/main.wowlua", line = 8}, event("PLAYER_LOGIN"))
```

The `__wl.prof.wrap` call is the dev build's profiler. Release builds have
none of it.

## Mistakes before `/reload`

Make a typo, such as `addon.titel`. `wowlua dev` reports it in the terminal
before the game ever loads the file, and so do `wowlua build` and
`wowlua check`:

```text
error[E2003]: `wowlua:meta` has no field `addon.titel`
  ┌─ src/main.wowlua:7:12
  │
7 │     chat(`{addon.titel} {addon.version} loaded ({build.profile} build)`)
  │            ^^^^^^^^^^^ here

error: aborting due to 1 previous error
```

`wowlua check` type-checks without writing a build. `wowlua explain E2003`
explains a code. All codes are listed in the
[diagnostics appendix](../spec/appendix/diagnostics.md).

Plain `.lua` files keep working: every Lua 5.1 file is valid wowlua. In a file
that uses no wowlua syntax, name and type errors are only warnings, so an
existing file never fails the build. Rename files to `.wowlua` one at a time
when you want types ([Types §2.1](../spec/language/02-types.md#21-checking-modes)).

## Test it

`wowlua test` runs the test files in wowlua's emulation of the WoW client: a
Lua 5.1 VM with WoW's Lua, taint and secret values, and the flavor's WoW API.
It loads the whole addon as the client would, fires `ADDON_LOADED`,
`PLAYER_LOGIN` and `PLAYER_ENTERING_WORLD`, then runs the tests. No game
client is needed.

The template's test still expects the template's message, so it fails now:

```sh
wowlua test
```

```text
 FAIL tests/main.test.wowlua (standard, 0.04s)
   ✗ greets at login
       expected { "|cff33ff99MyAddon|r: MyAddon 0.1.0 loaded (test build)" } to contain "MyAddon loaded."
       at tests/main.test.wowlua:8
     printed:
       |cff33ff99MyAddon|r: MyAddon 0.1.0 loaded (test build)
error: tests failed: 0 passed, 1 failed, 0 skipped (1 file) in 0.12s
```

Update `tests/main.test.wowlua`:

```wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("greets at login")
local function greets()
    local shown = table.concat(mock.printed(), "\n")
    expect(shown).toContain("MyAddon 0.1.0 loaded")
end
```

```text
 PASS tests/main.test.wowlua (standard, 0.04s)
   ✓ greets at login
      Tested 1 passed, 0 failed, 0 skipped (1 file) in 0.12s
```

Each test file runs once per flavor of the manifest, in a fresh VM.
`wowlua:test` has the test decorators, hooks and typed matchers. `wowlua:mock`
drives the simulated game world: units, combat, time, events, user input,
spies and stubs. `wowlua test --watch` runs again on every change, and
`--coverage` measures which lines ran. See
[Tooling §16.1](../spec/toolchain/tooling.md#161-test-runner-wowlua-test) for
everything a test can do.

`wowlua test` is newer than release 0.9.0. Until a release includes it, build
wowlua [from source](install.md#from-source) to use it.

## Build a release

```sh
wowlua build --release
```

```text
    Finished MyAddon 0.1.0-alpha (release: standard) in 0.06s → dist/release/MyAddon
```

The release profile bundles the addon into one file (`MyAddon.lua`), strips
comments and whitespace, and leaves out the debugging tools. Its version is
`0.1.0-alpha` until you build on a git tag that matches `package.version`
(`v0.1.0`). Source maps go next to the build, outside the AddOn folder, for
[`wowlua trace`](debugging-in-game.md#map-errors-back-to-your-source).

To publish, add targets under `publish.targets` in `wowlua.json`, save a token
with `wowlua login <site>`, and run `wowlua publish`. It builds, runs the
tests, packages the zip and uploads it to CurseForge, Wago, WoWInterface and
GitHub Releases. `wowlua publish --dry-run` shows what each site would get
without uploading anything ([Publishing](../spec/toolchain/publishing.md)).

## Where to go next

- [Debugging in game](debugging-in-game.md): the dev window and `/wl`.
- [Adopting an addon](adopting-an-addon.md): bring an existing addon to
  wowlua.
- The language: [syntax](../spec/language/01-syntax.md),
  [types](../spec/language/02-types.md),
  [classes, interfaces and enums](../spec/language/03-classes.md),
  [decorators](../spec/language/04-decorators.md) and
  [modules](../spec/language/05-modules.md).
- The [standard library](../spec/stdlib/README.md): events, saved variables,
  slash commands, timers, settings, a minimap button and more.
- The sample addons in [`examples/`](../examples), each with tests.
