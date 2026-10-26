# Getting started

Create an addon, run it in the game while you edit it, test it without the
game, and build a release. It takes a few minutes. You need wowlua
[installed](install.md) and, for the in-game part, a WoW client on the same
machine.

## The short version

1. **Create an addon** A manifest, a first module, a first test and a locale
   file. `wowlua new MyAddon --flavors camelot`
2. **Run it in the game** Builds, copies the AddOn into your WoW folder and
   builds again each time you save. `wowlua dev`
3. **Test it without the game** Runs your tests in wowlua's emulated WoW
   client. `wowlua test`
4. **Build a release** One bundled file without the dev tools.
   `wowlua build --release`

## Create an addon

```console
$ wowlua new MyAddon --flavors camelot
     Created addon package in MyAddon
$ cd MyAddon
```

`--flavors` names the game flavors to build for, comma-separated. `camelot`
is WoW: Forever, the flavor wowlua's emulated client targets; without the
option you get `standard`, the retail client. `--kind` picks the template
(`addon`, `library`, `lua-library` or `types`), and `--ci github` adds a
GitHub Actions workflow that builds every push and pull request and publishes
when you push a `v*` tag. When `git` is installed and the folder is not inside
a repository yet, `new` also runs `git init`.

```text
MyAddon/
├── wowlua.json            the manifest: title, flavors, dependencies, publish targets
├── src/
│   └── main.wowlua        your code
├── tests/
│   └── main.test.wowlua   tests, never part of a build
├── locales/
│   └── enUS.json          translations for wowlua:locale
├── CHANGELOG.md
├── README.md
└── .gitignore             ignores dist/
```

There is no TOC file to edit: wowlua generates one for every flavor from
`wowlua.json`, which starts small:

```json
{
  "$schema": "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/wowlua-0.1.json",
  "package": {
    "name": "my-addon",
    "version": "0.1.0",
    "authors": [
      "Your Name"
    ],
    "wowlua": "^0.12"
  },
  "addon": {
    "title": "MyAddon"
  },
  "flavors": {
    "camelot": {}
  }
}
```

`authors` comes from your git `user.name`. The first build adds
`wowlua.lock`, which pins the interface numbers and dependency versions;
commit it. Builds go to `dist/`, which git ignores. The
[manifest reference](../reference/manifest.md) lists every key.

## Run it in the game

```console
$ wowlua dev
       Built MyAddon 0.1.0-dev (dev: camelot) in 0.09s
   Installed /Applications/World of Warcraft/_classic_beta_/Interface/AddOns/MyAddon
       Ready type /reload in game to load the changes
```

`wowlua dev` builds your addon with the `dev` profile and copies it into the
client folders of your WoW installation that match the flavors you build for.
For camelot that is `_classic_beta_`; for `standard` it is `_retail_`,
`_ptr_` and the other retail folders. Then it watches your files. Each time
you save, it builds again, compiling only what changed, and copies the result
over.

Log in, or type `/reload` if the game is already running. WoW only loads Lua
again on `/reload`, so reload after each change.

- It never overwrites an AddOn folder it did not install, such as the same
  addon from an addon manager. That is error E4331, unless you pass `--force`.
- A build that fails leaves the installed AddOn alone, so the last working
  version stays in the game.
- `--link` links the AddOn folder to the build folder instead of copying it.
- `wowlua install` builds once and installs, and `wowlua uninstall` removes
  what wowlua installed.
- [Installation](install.md#where-wowlua-finds-world-of-warcraft) explains
  how wowlua finds your WoW folder.

## Your first edit

Replace `src/main.wowlua` with a module that greets you in the chat when you
log in:

```wowlua
import addon, build from "wowlua:meta"
import event from "wowlua:events"
import print as chat from "wowlua:log"

@event("PLAYER_LOGIN")
local function onLogin()
    chat(`{addon.title} {addon.version} loaded ({build.profile} build)`)
end
```

Save, then `/reload`. The chat shows `MyAddon 0.1.0 loaded (dev build)`,
after your addon's name in color.

`wowlua:meta` holds the addon's title, version and build information, filled
in at compile time. `@event` registers the function for the event, so you
need no frame and no `OnEvent` handler. `print` from `wowlua:log` writes to
the chat frame with your addon's name in front.

> [!TIP]
> Dev builds keep every line where it was: an error in the game on line 7 of
> `src/main.lua` is line 7 of `src/main.wowlua`. Type `/wl` in the game for
> the dev window with the profiler, the errors and the log
> ([Debugging](debugging.md)).

## Mistakes show up before /reload

Make a typo, such as `addon.titel`. `wowlua dev` reports it in the terminal
before the game ever loads the file, and so do `wowlua build` and
`wowlua check`. With the [VS Code extension](vscode.md), you see it
underlined as you type.

```text
error[E2003]: `wowlua:meta` has no field `addon.titel`
  ┌─ src/main.wowlua:7:12
  │
7 │     chat(`{addon.titel} {addon.version} loaded ({build.profile} build)`)
  │            ^^^^^^^^^^^ here

error: aborting due to 1 previous error
```

`wowlua check` type-checks without writing a build, and `wowlua explain E2003`
explains a code. [Errors and warnings](../reference/diagnostics.md) lists
them all.

> [!NOTE]
> Plain `.lua` files keep working: every Lua 5.1 file is valid wowlua. In a
> file that uses no wowlua syntax, name and type problems are only warnings,
> so existing code never fails the build. Rename files to `.wowlua` one at a
> time when you want types.

## Test it without the game

`wowlua test` runs your tests in wowlua's emulation of the WoW client. It
loads the whole addon as the game would, fires `ADDON_LOADED`,
`PLAYER_LOGIN` and `PLAYER_ENTERING_WORLD`, then runs the tests. The
template's test still expects the template's greeting, so it fails now:

```console
$ wowlua test
 FAIL tests/main.test.wowlua (camelot, 0.07s)
   ✗ greets at login
       expected { "|cff33ff99MyAddon|r: MyAddon 0.1.0 loaded (test build)" } to contain "MyAddon loaded."
       at tests/main.test.wowlua:8
     printed:
       |cff33ff99MyAddon|r: MyAddon 0.1.0 loaded (test build)
error: tests failed: 0 passed, 1 failed, 0 skipped (1 file) in 0.18s
```

Change `tests/main.test.wowlua` to look for the new message:

```wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("greets at login")
local function greets()
    local shown = table.concat(mock.printed(), "\n")
    expect(shown).toContain("MyAddon 0.1.0 loaded")
end
```

```console
$ wowlua test
 PASS tests/main.test.wowlua (camelot, 0.07s)
   ✓ greets at login
      Tested 1 passed, 0 failed, 0 skipped (1 file) in 0.17s
```

`mock.printed()` returns what the chat frame showed. From here a test can
spawn units, click buttons, type into edit boxes, move time forward and
compare screenshots. [Testing your addon](testing.md) shows how.

## Build a release

```console
$ wowlua build --release
    Finished MyAddon 0.1.0-alpha (release: camelot) in 0.09s → dist/release/MyAddon
```

The release build bundles your addon into one file (`MyAddon.lua`), strips
comments and whitespace, and leaves out the dev tools. Its version stays
`0.1.0-alpha` until you build on a git tag that matches `package.version`
(`v0.1.0`). When you are ready to ship, `wowlua publish` builds, runs your
tests and uploads to CurseForge, Wago, WoWInterface and GitHub Releases
([Building and publishing](publishing.md)).

## Where to go next

- [VS Code](vscode.md): errors as you type, and a debugger that shows the
  game's screen.
- [A tour of wowlua](../language/tour.md): types, classes, decorators and
  modules on one page.
- [The standard library](../stdlib/overview.md): events, saved variables,
  slash commands, settings and more.
- [Bring your addon](adopting.md): move an existing addon over without
  rewriting it.
