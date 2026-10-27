# x6xxx · bundling

## E6003

A level in `minify-levels` lists a pass that doesn't exist. The passes are
`strip-comments`, `strip-whitespace`, `simplify`, `compact`,
`shorten-literals`, `rename-locals`, `rename-private` and `dedupe-strings`;
the message suggests the closest name. Correct the spelling, or build the
level on a preset with `extends` (`strip`, `compact`, `rename`, `max`).

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.13" },
    "flavors": { "camelot": {} },
    "minify-levels": { "readable": { "passes": ["strip-comment"] } }
}
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.13" },
    "flavors": { "camelot": {} },
    "minify-levels": { "readable": { "passes": ["strip-comments"] } }
}
```

Related: W6010

## W6001

A bundled file calls `debugstack` or `debuglocals` and has a string holding
`.lua`, so it most likely reads its own Lua file name from the stack. Bundling
(on in release builds) puts the files into one file, so the stack shows the
bundle's name and line instead, and code that matches on its own file name
stops finding it. Reading the addon's folder (`AddOns/<name>/`) is fine: the
bundle keeps it. Keeping or showing a stack is fine too, because `wowlua
trace` maps it back.

Match on the addon's folder instead of the file name, or turn bundling off
for the profile with `"bundle": false`. Libraries that call either function
are never bundled, so this warning is only about your own files. The examples
turn bundling on for the dev profile, so that `wowlua check` sees it.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.13" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "bundle": true } }
}
```
```lua fails title=src/main.lua
local file = debugstack(1, 1, 0):match("([^/\\]+)%.lua")
print(file)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.13" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "bundle": true } }
}
```
```lua compiles title=src/main.lua
local folder = debugstack(1, 1, 0):match("AddOns[/\\]([^/\\]+)")
print(folder)
```

## W6002

With `optimize.dead-code` (on in release builds), a module was left out of
the build: its top level only declares things (functions, constants, classes,
types, locals set to literals or tables), it runs no code and registers
nothing, and no other module imports it. Removing it changes nothing in game;
the warning tells you it is unused.

Delete the module if you no longer need it. If plain Lua code reaches it by
name (`__wowlua.require("id")`), list it in `build.keep`. The examples turn
dead-code removal on for the dev profile, so that `wowlua check` sees it.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.13" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "optimize": { "dead-code": true } } }
}
```
```wowlua fails title=src/colors.wowlua
export const RED = "ff0000"
```
```wowlua fails title=src/main.wowlua
print("hello")
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.13" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "optimize": { "dead-code": true } } }
}
```
```wowlua compiles title=src/colors.wowlua
export const RED = "ff0000"
```
```wowlua compiles title=src/main.wowlua
import RED from "/colors"
print(RED)
```

Related: W1003

## W6010

A publish target for CurseForge, Wago or WoWInterface sets a `minify` level
with at least one minification pass. Blizzard's UI Add-On Development Policy
says an addon's code "must in no way be hidden or obfuscated, and must be
freely accessible to and viewable by the general public", and these sites
take readable code only. Their targets therefore build unminified unless the
target sets `minify` itself.

`wowlua publish` (also with `--dry-run`) reports it, before anything is
uploaded; with `--deny-warnings` it stops the publish. `wowlua check` and
`wowlua build` don't look at publish targets. Remove the target's `minify`,
or publish minified builds to a GitHub target, which may minify fully:

```json
"publish": {
    "targets": {
        "cf": { "type": "curseforge", "project-id": 123456 },
        "gh": { "type": "github", "repository": "jane/my-addon", "minify": "max" }
    }
}
```

Related: E6003
