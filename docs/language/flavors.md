# Build info and profiles

`wowlua:meta` tells your code about the addon and the build it is part of:
the profile, the version, your own settings, and the game flavor. The
compiler knows these values, so it folds them into the output and removes
code that a build doesn't need.

## Know which build you are in

`addon` holds the addon's details from `wowlua.json`. `build` describes the
current build.

```wowlua
import addon, build from "wowlua:meta"
import event from "wowlua:events"
import print as chat from "wowlua:log"

@event("PLAYER_LOGIN")
local function onLogin()
    chat(`{addon.title} {build.version} loaded ({build.profile} build)`)
end
```

The values are folded in at compile time, so the game gets a plain string.
From the dev build:

```lua
	chat("MyAddon 0.1.0-dev+1100829 loaded (dev build)")
```

- `addon` has `name` (the folder and TOC name), `package`, `title`, `notes`,
  `version`, `authors`, `license`, `homepage`, `interface`, `flavors`,
  `savedVariables`, `savedVariablesPerCharacter`, `dependencies`, and `meta`
  for your own `addon.meta` entries.
- `build` has `profile` (`"dev"`, `"release"`, `"test"` or one of yours),
  `debug`, `version`, `commit`, `dirty`, `time`, `timestamp`, `compiler`,
  `id`, `bundled` and `minify`.
- Everything is read-only (E2010), and a field that doesn't exist, such as
  `addon.titel`, is a compile error (E2003).
- Code that uses a value at run time, for example
  `ipairs(addon.authors)`, reads it from a small table in the build that
  holds only the values used that way:

```lua
for _, name in ipairs(__wl.meta.addon.authors) do
	print(name)
end
```

The full list of fields is in
[Meta §6.1](../../spec/language/06-meta.md#61-wowluameta).

## Code that only exists in dev builds

Put code you only want while developing in `if build.debug then`. In a build
where `build.debug` is false, the block is removed, and it isn't type-checked
either.

```wowlua
import addon, build from "wowlua:meta"

if build.debug then
    print("Loaded", addon.name, addon.version)
end
```

The dev build keeps the block:

```lua
do
	print("Loaded", "MyAddon", "0.1.0")
end
```

The release build has nothing in its place.

This works for `if`/`elseif` chains whose conditions only read `wowlua:meta`
values, `const` constants and literals, combined with `not`, `and`, `or`,
`==`, `~=`, arithmetic and number comparisons. A branch whose condition is
false is removed, a branch whose condition is true is kept on its own. For
whole functions there is
[`@debugOnly`](debugging.md#whole-functions-just-for-you), and `log.debug(…)`
calls are removed with their arguments when the build's log level is above
`debug`.

## Leave out a declaration or an import

`@if(condition)` removes a declaration when its condition is false: a
function, a class or one of its members, an enum, a `local` or `const`, a
type, an interface, or an import.

```wowlua title=src/Diagnostics/panel.wowlua
export function show(value: unknown)
    print("state:", value)
end
```

```wowlua title=src/bar.wowlua
import build from "wowlua:meta"

@if(build.debug)
import * as diagnostics from "/Diagnostics/panel"

export class Bar
    value: number = 0

    @if(build.debug)
    function dumpState()
        diagnostics.show(self.value)
    end

    function refresh()
        self.value += 1
    end
end
```

The condition of `@if` must be known at compile time (E1040). On an import it
may only use `wowlua:meta` values and literals, not constants. Code that is
kept and still uses something `@if` removed is an error that shows both
places. Here `refresh` calls `diagnostics.show`, in a release build:

```text
error[E1020]: `diagnostics` was removed by `@if` for this build
   ┌─ src/bar.wowlua:16:9
   │  
 3 │ ╭ @if(build.debug)
 4 │ │ import * as diagnostics from "/Diagnostics/panel"
   │ ╰─────────────────────────────────────────────────' removed here
   ·  
16 │           diagnostics.show(self.value)
   │           ^^^^^^^^^^^ used here
```

## Profiles: dev, release, test and your own

A profile decides what goes into a build. The usual commands pick one for
you:

- `wowlua dev` builds `dev`: one Lua file per source with its line numbers
  kept, every function profiled, the `/wl` dev window, error capture and
  `debug` logging.
- `wowlua build --release` and `wowlua publish` build `release`: bundled
  into one file, comments and whitespace stripped, `warn` logging and none of
  the dev tools.
- `wowlua test` builds `test`: assertions, every runtime check and `trace`
  logging.

`wowlua build` and `wowlua check` use `dev` unless you pass `--release` or
`--profile <name>`.

Change a built-in profile, or add your own on top of one, under `profiles` in
`wowlua.json`. A profile of your own names the one it builds on with
`inherits` (E4315 without it), and lists only what differs. Here the release
leaves a developer folder out and minifies a step further, and `beta` is a
release that keeps `info` logging:

```json
"profiles": {
  "release": { "minify": "compact", "exclude": ["src/Diagnostics/**"] },
  "beta": { "inherits": "release", "log-level": "info" }
}
```

```console
$ wowlua build --profile beta
    Finished MyAddon 0.1.0-alpha+1100829 (beta: standard) in 0.11s → dist/beta/MyAddon
```

In this build `build.profile` is `"beta"`, and `build.debug` is false, taken
from `release`. Arrays such as `exclude` are inherited as a whole, so `beta`
leaves `src/Diagnostics/` out too. Dev builds and tests keep it. Guard every
import of an excluded file with `@if(build.debug)`, as above.

The keys of a profile are listed in the
[manifest reference](../reference/manifest.md); the dev tools they switch on
are on [Debugging helpers](debugging.md).

## Your own settings per profile

Define values in a top-level `config` and override them in a profile:

```json
"config": {
  "logChannel": "SYSTEM",
  "maxHistory": 50
},
"profiles": {
  "dev": { "config": { "maxHistory": 500 } }
}
```

In code, `config` has the type of what you wrote, here
`{ logChannel: string, maxHistory: number }`, and each value is folded in:

```wowlua title=src/history.wowlua
import config from "wowlua:meta"

-- 500 in dev builds, 50 in all others
export local history: string[] = {}

export function remember(line: string)
    history[#history + 1] = line
    if #history > config.maxHistory then
        table.remove(history, 1)
    end
end
```

A profile can't change the type of a value, so the code checks the same in
every build:

```text
error[E4012]: profile `dev`: config key `maxHistory` is a string but the top-level `config` defines a number
   ┌─ wowlua.json:33:9
   │
33 │         "maxHistory": "lots"
   │         ^^^^^^^^^^^^ invalid config key
```

## Optional features

Features are named switches in `wowlua.json`. Each one lists other features
it turns on, or `dep:<name>` to turn on an optional dependency. The features
in `default`, and those they include, are on. Feature names are kebab-case in
the manifest and camelCase in code. The manifest of this page, with the
settings and profiles from above:

```json title=wowlua.json
{
  "package": {
    "name": "my-addon",
    "version": "0.1.0",
    "wowlua": "^0.13"
  },
  "addon": {
    "title": "MyAddon"
  },
  "features": {
    "default": ["minimap"],
    "minimap": [],
    "experimental-ui": []
  },
  "config": {
    "logChannel": "SYSTEM",
    "maxHistory": 50
  },
  "profiles": {
    "dev": { "config": { "maxHistory": 500 } },
    "release": { "minify": "compact", "exclude": ["src/Diagnostics/**"] },
    "beta": { "inherits": "release", "log-level": "info" }
  }
}
```

```wowlua title=src/main.wowlua
import features from "wowlua:meta"

@if(features.experimentalUi)
import Bar from "/bar"

if features.minimap then
    print("minimap button on")
end

if features.experimentalUi then
    Bar.new():refresh()
end
```

`features.minimap` is `true` and `features.experimentalUi` is `false`, so the
build keeps the `print` and leaves out the import and the second `if`. A
library's features are switched on by the packages that depend on it, with
`features` in their dependency entry (see
[Libraries and dependencies](../guide/packages.md)).

## Versions that tell builds apart

`addon.version` is always `package.version` from `wowlua.json`.
`build.version`, which also goes into the TOC's `## Version:`, says what kind
of build it is:

- `0.1.0`: a release-type build of a commit tagged `v0.1.0` (or `0.1.0`).
  If the tag doesn't match `package.version`, the build stops with E4319.
- `0.1.0-alpha+1100829`: a release-type build of any other commit, or with
  uncommitted changes.
- `0.1.0-dev+1100829`: a dev or test build.

Without git the `+hash` part is left out. A bug report that names
`0.1.0-alpha+1100829` therefore points at one commit.

```console
$ wowlua build --release
    Finished MyAddon 0.1.0-alpha+1100829 (release: standard) in 0.10s → dist/release/MyAddon
$ git tag v0.1.0
$ wowlua build --release
    Finished MyAddon 0.1.0 (release: standard) in 0.10s → dist/release/MyAddon
```

## Paths to your textures and sounds

`asset(path)` turns a path in your package into the full in-game path, and
checks that the file is one of the addon's assets (`build.assets` in
`wowlua.json`). The path is relative to the package root and uses `/`.

```wowlua fragment
import asset from "wowlua:meta"

local icon = UIParent:CreateTexture()
icon:SetTexture(asset("media/icon.tga"))
```

```lua
icon:SetTexture("Interface\\AddOns\\MyAddon\\media\\icon.tga")
```

A file that isn't an asset, a typo in its name or a difference in case is
error E1038 at compile time, instead of a green square in the game. The path
is built from the addon's folder name, so it follows a rename.

## The game you build for

A flavor is one of WoW's rulesets. The flavors your addon supports are the
keys of `flavors` in `wowlua.json`, and `wowlua new --flavors` fills it in.
Without `flavors`, an addon builds for `standard` only.

| Flavor | Also called | Game | Support |
|--------|-------------|------|---------|
| `camelot` | `forever` | WoW Forever | Supported |
| `standard` | `retail`, `mainline` | Retail | Coming soon |
| `vanilla` | `classic-era` | Classic Era | Coming soon |
| `tbc` | `anniversary` | TBC Anniversary | Coming soon |
| `mists` | | MoP Classic | Coming soon |

A supported flavor runs in every part of wowlua: `wowlua test`, the emulated
client, the debugger and Blizzard's UI. A flavor coming soon is checked and
built like the others (types, TOCs, packages), but wowlua can't run it yet.
Wrath of the Lich King Classic and Cataclysm Classic are not flavors of
wowlua.

Each flavor brings its own typed [WoW API](../reference/api.md), so your code
is checked against the functions that client has. The TOC's interface number
comes from the same place and is pinned in `wowlua.lock`.

`flavor` is the flavor being built, and `addon.interface` its interface
numbers. By default the compiler runs once per flavor, so `flavor` is a
constant and a branch for another flavor is removed. A removed branch isn't
checked either, so it may call an API the other flavor lacks:

```wowlua
import addon, flavor from "wowlua:meta"

local frame = CreateFrame("Frame", "MyAddonFrame", UIParent)

if flavor == "standard" then
    EditModeManagerFrame:RegisterSystemFrame(frame)
else
    frame:SetMovable(true)
end

print(flavor, addon.interface[1])
```

With `"flavors": { "standard": {}, "vanilla": {} }`, the build has one
copy of the file per flavor, and a single TOC loads the right one:

```lua
-- wowlua/flavors/standard/src/main.lua
do
	EditModeManagerFrame:RegisterSystemFrame(frame)
end

print("standard", 120100)

-- wowlua/flavors/vanilla/src/main.lua
do
	frame:SetMovable(true)
end

print("vanilla", 11509)
```

```text
## Interface: 120100, 11509
wowlua\flavors\standard\src\main.lua [AllowLoadGameType mainline]
wowlua\flavors\vanilla\src\main.lua [AllowLoadGameType classic]
```

With `"build": { "flavor-mode": "shared" }` the addon is compiled once for
all its flavors. Then `flavor` is read at run time, the code is checked
against the API all of them share, and a call into an API that only some
flavors have must be guarded with a flavor check or an existence check such
as `if C_Garrison then` (E2030 otherwise). See
[Meta §6.3](../../spec/language/06-meta.md#63-flavors) and
[§6.5](../../spec/language/06-meta.md#65-runtime-flavor-checks-shared-mode).

## Where to go next

- [Debugging helpers](debugging.md): what dev builds measure, and `assert`
  and `dump`.
- [Building and publishing](../guide/publishing.md): release builds, tags and
  publish targets.
- [wowlua.json](../reference/manifest.md): every key of the manifest.
