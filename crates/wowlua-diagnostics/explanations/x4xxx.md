# x4xxx · manifest

## E4001

`wowlua.json` is not valid JSON. The manifest is strict JSON: no comments, no
trailing commas, keys and strings in double quotes. Nothing else in the
package is checked until the manifest parses.

The message gives the position of the first problem. Remove the trailing comma
or comment, or quote the key. An editor that knows the manifest's `$schema`
shows these mistakes as you type.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12",
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    }
}
```

Related: E4002

## E4002

The manifest is valid JSON but does not match the schema: an unknown key
(often a typo, so the help suggests the closest key), a value of the wrong
type, a missing `package` section, or a value the schema does not allow, such
as a SavedVariable that is no Lua identifier, a TOC directive in `toc-extra`
that wowlua writes itself, or a `flavor-overrides` key for a flavor the
package does not target. Unknown keys are errors so that a misspelled setting
is never ignored without a word.

Fix the key or the value the message points at. If the key belongs to a newer
wowlua, check `package.wowlua` and your compiler's version (E4300).

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon": {
        "titel": "Playground"
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon": {
        "title": "Playground"
    },
    "flavors": {
        "camelot": {}
    }
}
```

Related: E4001, E4300

## E4003

`package.name` or `package.version` is not valid. A package name is 2 to 64
characters of lowercase letters, digits and `-` (`my-addon`); the version is a
full SemVer 2.0 version with three numbers (`1.0.0`, `2.1.0-beta.1`).

The package name is the ID used for dependencies, not what players see. The
AddOn's folder and TOC name come from `addon.name`, which defaults to the
PascalCase form of the package name (`my-addon` becomes `MyAddon`) and may use
capitals.

```json fails title=wowlua.json
{
    "package": {
        "name": "Playground",
        "version": "0.1",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    }
}
```

## E4004

A flavor name is not in wowlua's flavor table: in `flavors`, in a dependency's
or publish target's `flavors`, in `addon.flavor-overrides` or `addon.toc-data`,
or given with `--flavor`. Flavors are Blizzard's game types: `standard`,
`camelot`, `mists`, `cata`, `wrath`, `tbc` and `vanilla`, with aliases such as
`retail`, `forever`, `anniversary` and `classic-era`. The message lists the
known flavors.

`classic` is not a flavor: name the Classic game the addon is for (`vanilla`
for Classic Era, `mists` for Mists of Pandaria Classic, and so on).

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "classic": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "vanilla": {}
    }
}
```

## E4005

With `"compat": { "pkgmeta": true }`, the project's `.pkgmeta` stays the
source of its dependencies and packaging settings, and `wowlua.json` only adds
what is wowlua's own. A setting that both files make is this error, because
one of them would be ignored: a dependency's source keys (`git`, `svn`, `tag`,
`path`, `install-path`, …) for an external that `.pkgmeta` lists,
`build.libs-dir`, `build.exclude` (`.pkgmeta`'s `ignore`), `addon.name`
(`package-as`) or `publish.changelog` (`manual-changelog`).

Keep the setting in `.pkgmeta` and remove it from `wowlua.json`. A dependency
entry in `wowlua.json` may still add `files`, `flavors`, `optional` and the
other keys that are not about the source.

```yaml fails title=.pkgmeta
package-as: Playground

ignore:
  - notes
```
```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "build": {
        "exclude": ["notes/**"]
    },
    "compat": {
        "pkgmeta": true
    }
}
```

```yaml compiles title=.pkgmeta
package-as: Playground

ignore:
  - notes
```
```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "compat": {
        "pkgmeta": true
    }
}
```

Related: W4333

## E4006

A name that should refer to a profile, minification level or addon does not:
a profile's `inherits`, a profile's or publish target's `minify`, a target's
`profile`, a minification level's `extends`, an entry of
`publish.bundle-addons`, or `--profile` on the command line. The built-in
profiles are `dev`, `release` and `test`; the built-in minification levels
are `none`, `strip`, `compact`, `rename` and `max`. Your own come from
`profiles` and `minify-levels`.

Fix the name, or define the profile or level.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "profiles": {
        "release": { "minify": "tiny" }
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "profiles": {
        "release": { "minify": "compact" }
    }
}
```

Related: E4007, E4315, E6003

## E4007

Profiles that inherit from each other in a circle (`beta` inherits `staging`,
which inherits `beta`), or minification levels that extend each other in a
circle. The effective settings are built by walking the chain down to its
base, so a chain that never reaches a built-in profile or level has no
settings.

Let one of them inherit from a built-in profile (`dev` or `release`), or
extend a built-in level.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "profiles": {
        "beta": { "inherits": "staging" },
        "staging": { "inherits": "beta" }
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "profiles": {
        "beta": { "inherits": "staging" },
        "staging": { "inherits": "release" }
    }
}
```

Related: E4006, E4315

## E4012

A profile's `config` sets a key that the top-level `config` does not define,
or gives it a value of another type. The top-level `config` lists every
configuration key with its default, and its values decide the type of
`config` in `wowlua:meta`; profiles may only change those values.

Add the key with its default to the top-level `config`, or give the profile's
value the same type (below, a number instead of a string).

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "config": {
        "maxHistory": 50
    },
    "profiles": {
        "dev": { "config": { "maxHistory": "500" } }
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "config": {
        "maxHistory": 50
    },
    "profiles": {
        "dev": { "config": { "maxHistory": 500 } }
    }
}
```

## E4101

The versions that the package's dependencies ask for cannot all be met. Every
dependency is embedded in one AddOn, so each addon uses exactly one version of
each package, across all its flavors and features. When two declarations want
different versions of the same package (two git dependencies on different
tags of a shared library, for example), resolution fails with this error. Its
notes show the resolver's reasoning and which source each version stands for.

Make the declarations agree on one version, tag, branch or revision. The
error comes from dependency resolution, which needs the dependencies'
sources, so it has no example here.

Related: E4102, E4321

## E4102

A dependency cannot be found: a `path` that does not exist, a git repository
without the given tag, branch, commit or `subdir`, an archive without the
`strip-prefix` folder, a source whose `wowlua.json` names another package, or
a registry dependency (a bare version requirement such as `"^1.0"`). The
wowlua registry is not available yet, so registry dependencies are always this
error for now.

Check the source in the dependency entry. For a library you cannot get from a
registry, use a `git`, `svn`, `path` or `archive` source. The error comes from
dependency resolution, so it has no example here.

Related: E4101, E4105

## E4103

The files of a fetched dependency do not match the checksum in `wowlua.lock`.
wowlua checks every git, SVN and archive dependency against its locked
checksum on every fetch, so a library that changed after it was locked (a tag
moved, an archive replaced, a damaged cache) does not end up in your AddOn
unnoticed.

If the source changed on purpose, run `wowlua update` for that dependency and
review the new lock entry. Otherwise run `wowlua cache clean --all` and fetch
again. The error needs a lockfile and a fetched dependency, so it has no
example here.

## E4104

`--locked` (or `--frozen`) was given, but `wowlua.lock` would have to change,
for example because a dependency was added or changed in `wowlua.json`.
`--locked` is meant for CI, where a build must use exactly what was committed.
`wowlua publish` always resolves this way.

Run the command once without `--locked` and commit the updated `wowlua.lock`.
The error comes from the lockfile on disk, so it has no example here.

Related: E4105

## E4105

The command runs with `--offline` (or `--frozen`) and needs a dependency that
is not in the cache. Offline, wowlua may still resolve and update the
lockfile, but it never downloads anything.

Run `wowlua fetch` once with network access, then work offline. The error
needs the network and the cache, so it has no example here.

Related: E4104, E4102

## E4110

wowlua cannot tell which files of a `lua-library` dependency to load, or a
file it was told to load does not exist. For a plain Lua library it uses, in
this order: the dependency's `files`, the library's own `lua-library.files`,
the file list of a single `.toc` in the library's root, or a single
`lib.xml`, `embeds.xml` or `<name>.xml` there. Without any of these, or when
a listed file is missing, the library cannot be put into the TOC.

List the entry files in the dependency's `files`, in load order. Only the
entry files appear in the TOC, and their paths resolve case-insensitively,
like the client's. The error comes from a dependency's files, so it has no
example here.

Related: W4306

## E4120

Two addons of one workspace use the same SavedVariables name. The client keeps
SavedVariables in global variables, so both addons would read and overwrite
the same saved data.

Give each addon its own name, for example one that starts with the addon's
name.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon": {
        "saved-variables": ["PlaygroundDB"]
    },
    "flavors": {
        "camelot": {}
    },
    "workspace": {
        "members": ["options"],
        "default-members": ["."]
    }
}
```
```json fails title=options/wowlua.json
{
    "package": {
        "name": "playground-options",
        "version": "0.1.0"
    },
    "addon": {
        "saved-variables": ["PlaygroundDB"]
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon": {
        "saved-variables": ["PlaygroundDB"]
    },
    "flavors": {
        "camelot": {}
    },
    "workspace": {
        "members": ["options"],
        "default-members": ["."]
    }
}
```
```json compiles title=options/wowlua.json
{
    "package": {
        "name": "playground-options",
        "version": "0.1.0"
    },
    "addon": {
        "saved-variables": ["PlaygroundOptionsDB"]
    },
    "flavors": {
        "camelot": {}
    }
}
```

Related: E4325

## E4201

`wowlua publish` found no API token for a publish target. Tokens never go into
`wowlua.json`. They come from the target's environment variable
(`CF_API_TOKEN` for CurseForge, `WAGO_API_TOKEN` for Wago, `WOWI_API_TOKEN`
for WoWInterface, or the target's `token-env`), from the OS credential store
that `wowlua login <type>` fills, or from `config.json`. Every target's token
is checked before anything is uploaded.

Run `wowlua login <type>`, or set the environment variable (in CI, from a
secret). Without a terminal, wowlua does not read the OS credential store, so
a background job needs the variable or a token saved with
`WOWLUA_NO_KEYRING=1 wowlua login <type>`. The error comes from publishing,
so it has no example here.

Related: E4202, W4326

## E4202

A publish site rejected the upload. The message includes the site's response,
which says why: a wrong or expired token, a project ID that is not yours, a
disabled project, or a missing field. A target that fails does not stop the
uploads to the other targets, but the command exits with an error.

Check the target's `project-id` and the token, and read the site's response.
The error comes from publishing, so it has no example here.

Related: E4201, E4210

## E4210

A CurseForge upload has no changelog. CurseForge requires one for every file.
The changelog comes from `publish.changelog` (or the target's `changelog`):
`CHANGELOG.md` when that file exists, a file you name, or `"git"` to build it
from the commit history.

Add a `CHANGELOG.md`, or set `"changelog": "git"`. The error comes from
publishing, so it has no example here.

Related: E4202

## E4230

An addon cannot be published to the wowlua registry. The registry is for
packages that other packages depend on: `library`, `lua-library` and `types`
packages. Players get addons from the addon sites, so publish an addon with a
`curseforge`, `wago`, `wowinterface` or `github` target instead.

The rule belongs to publishing, so it has no example here.

## E4300

The package asks for another compiler version than the one running: the
running wowlua does not satisfy `package.wowlua`. This is checked before
anything else in the manifest, so a manifest written for a newer wowlua
reports this one error instead of an unknown key (E4002) for every new
setting.

Install a wowlua version that matches the requirement, or, if the package
works with your version, change the requirement.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^9.0"
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    }
}
```

Related: E4002

## E4301

`build.toc-layout` is `"single"`, but a client of a targeted flavor lacks a
TOC feature that one TOC for all flavors needs: comma-separated
`## Interface` values, inline load conditions on file lines, inline conditions
on `##` lines (for flavor-specific `addon-dependencies` entries),
`[AllowLoadTextLocale]`, or a game-type name the TOC uses. Such a client would
misread the TOC: one that knows none of the game types in a condition loads
the file anyway, so it could run every flavor's code at once. The message
names the client and the feature.

Use `"toc-layout": "auto"` (the default), which falls back to one TOC per
flavor when a client lacks a feature, or stop targeting that flavor. Below, a
Retail beta client cannot read the condition that the `camelot`-only
dependency needs.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "standard": {},
        "camelot": {}
    },
    "addon-dependencies": {
        "optional": [{ "name": "Details", "flavors": ["camelot"] }]
    },
    "build": {
        "toc-layout": "single"
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "standard": {},
        "camelot": {}
    },
    "addon-dependencies": {
        "optional": [{ "name": "Details", "flavors": ["camelot"] }]
    },
    "build": {
        "toc-layout": "auto"
    }
}
```

Related: E4302, W4304

## E4302

With one TOC per flavor, the file names cannot tell the targeted flavors
apart. The client looks for `<Name>_<GameSuffix>.toc`, then
`<Name>_<FamilySuffix>.toc` (`_Mainline` or `_Classic`), then `<Name>.toc`.
wowlua gives each flavor a suffix its clients recognize; when two flavors of
the same family would both need the family suffix, their clients would load
the same file.

Use the single TOC layout, or drop one of the flavors. With the flavor table
that ships with this wowlua, the error cannot happen yet: at most one flavor
per family lacks a game suffix all its clients know. It has no example here.

Related: E4301

## E4303

A line of a generated TOC is longer than 1024 characters, and the client reads
only the first 1024 characters of a line. wowlua splits the directives that
the client merges from repeated lines (`Dependencies`, `OptionalDeps`,
`LoadWith`, `LoadManagers`, `SavedVariables`,
`SavedVariablesPerCharacter`). Any other directive, such as a long `Notes`,
`Title` or `toc-extra` value, or a file line, cannot be split, so it is this
error.

Shorten the value, or spread long data over several `X-` directives. An
example would need a line of more than 1024 characters, so it is left out.

## E4307

An `asset()` path from `wowlua:meta` does not name an asset of the build.
`asset("media/icon.tga")` becomes the file's in-game path, so a file missing
from the output would be a missing texture in game.

The compiler already reports a path that is not in `build.assets` as E1038
while it checks the code, so this second check of the finished output does
not fire in practice. Add the file to `build.assets`, with its case exactly as
in the call.

Related: E1038

## E4308

Two files of the output have paths that differ only in case, such as
`src/Core.lua` and `src/core.lua`. On Windows and macOS these name the same
file, so one would overwrite the other when the AddOn is installed. This
happens, for example, when a `.wowlua` file and a plain `.lua` file of almost
the same name compile to the same output name.

Rename or remove one of the files.

```wowlua fails title=src/Core.wowlua
print("new core")
```
```lua fails title=src/core.lua
print("old core")
```

```wowlua compiles title=src/Core.wowlua
print("new core")
```
```lua compiles title=src/legacy.lua
print("old core")
```

Related: E4311

## E4311

A file would be written to a path that the build itself uses: an asset in one
of the generated folders (`src/`, `wowlua/`, `libs/`), an asset named like a
generated file (`Bindings.xml`, `LICENSES.txt`, a `.toc` in the AddOn root),
an asset that is also a compiled file, or a file at the path of a bundle. One
of them would overwrite the other.

Move or rename the asset, or exclude it from `build.assets`. For a bundle,
rename the file or set `bundle: false`. `LICENSES.txt` is where wowlua collects
the licenses of embedded libraries, so keep your own license under another
name such as `LICENSE.txt`.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "build": {
        "assets": ["LICENSES.txt"]
    }
}
```
```text fails title=LICENSES.txt
MIT License
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "build": {
        "assets": ["LICENSE.txt"]
    }
}
```
```text compiles title=LICENSE.txt
MIT License
```

Related: E4308

## E4312

`addon.load-on-demand` is set to `false`, but `addon-dependencies.load-with`
lists addons. `LoadWith` makes the client load the addon together with those
addons, which only works for an addon that loads on demand, so the two
settings contradict each other.

Remove `"load-on-demand": false`: with `load-with`, the addon loads on demand
by itself. If it should load at login instead, remove `load-with`.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon": {
        "load-on-demand": false
    },
    "addon-dependencies": {
        "load-with": ["Blizzard_Collections"]
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon-dependencies": {
        "load-with": ["Blizzard_Collections"]
    },
    "flavors": {
        "camelot": {}
    }
}
```

## E4314

An entry of `features` names something that does not exist or cannot be
switched on: a feature that is not defined (or `default`, which only lists the
features that are on by default), `dep:<name>` for a dependency the manifest
does not declare, or `dep:<name>` for a dependency that is not marked
`"optional": true`. A required dependency is always included, so no feature
can switch it on. `--features` on the command line reports it too when it
names a feature the package does not define.

Define the missing feature, fix the name, or mark the dependency optional.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "features": {
        "default": ["minimap"],
        "minimap": ["icons"]
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "features": {
        "default": ["minimap"],
        "minimap": ["icons"],
        "icons": []
    }
}
```

## E4315

A profile of your own does not say which profile it builds on. Only `dev`,
`release` and `test` have defaults; every other profile starts from one of
them through `inherits` and changes only what it sets.

Add `"inherits": "dev"` or `"inherits": "release"` (or the name of another
profile of yours).

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "profiles": {
        "beta": { "log-level": "info" }
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "profiles": {
        "beta": { "inherits": "release", "log-level": "info" }
    }
}
```

Related: E4006, E4007

## E4316

A workspace member sets its own `build.out`. All addons of a workspace are
built into one output folder, the workspace root's `build.out` (by default
`dist`), so that `dist/<profile>/` holds every AddOn side by side.

Remove `build.out` from the member, and set it in the workspace root if you
want another folder.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "workspace": {
        "members": ["options"],
        "default-members": ["."]
    }
}
```
```json fails title=options/wowlua.json
{
    "package": {
        "name": "playground-options",
        "version": "0.1.0"
    },
    "flavors": {
        "camelot": {}
    },
    "build": {
        "out": "build"
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "build": {
        "out": "build"
    },
    "workspace": {
        "members": ["options"],
        "default-members": ["."]
    }
}
```
```json compiles title=options/wowlua.json
{
    "package": {
        "name": "playground-options",
        "version": "0.1.0"
    },
    "flavors": {
        "camelot": {}
    }
}
```

## E4317

The manifest has an `addon` section, but the package is not an addon
(`package.kind` is `library`, `lua-library` or `types`). Only an addon gets a
TOC and an AddOn folder; a library is built into the addons that depend on
it, and they bring their own `addon` settings.

Remove the `addon` section, or set `kind` to `addon` if the package is meant
to be one.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "kind": "library",
        "wowlua": "^0.12"
    },
    "addon": {
        "title": "Playground"
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "kind": "library",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    }
}
```

## E4318

A targeted flavor has no interface number, so its TOC would have no
`## Interface` value. The numbers come from the flavor's `interface` in the
manifest, or else from the flavor table pinned in `wowlua.lock`.

Remove the empty `interface` list to use the pinned numbers, list the numbers
yourself, or update the pinned table with `wowlua types sync`.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": { "interface": [] }
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": { "interface": [16001] }
    }
}
```

## E4319

A release build is on a git tag that does not match `package.version`, or
`wowlua publish --tag` names such a tag. A tag, with an optional leading `v`,
must equal the version (`1.4.2` or `v1.4.2` for version `1.4.2`), because a
tagged build is released as exactly that version.

Update `package.version` or the tag. In a workspace only the root package's
version must match; a member whose version no tag names is built as an
untagged (alpha) build. Converted packager projects take their version from
the tag and are exempt. The error needs git, so it has no example here.

Related: E4330

## E4320

`wowlua.lock` was written by a newer wowlua and has a lockfile version this
compiler does not understand. Older lockfiles are read and rewritten in the
current format, but a newer one cannot be read safely.

Update wowlua to the version your project uses (see `package.wowlua`). The
error comes from the lockfile on disk, so it has no example here.

Related: E4300

## E4321

Two dependencies in one addon's graph bring a package of the same name from
different sources, for example one from git and one from a path, or the same
repository at two URLs. A package is identified by its name and its source,
and an addon can only embed one copy of each package.

Make every package that depends on it use the same source. The notes say which
dependency uses which source. The error comes from dependency resolution, so
it has no example here.

Related: E4101

## E4323

A fetched dependency contains an entry that wowlua refuses to unpack: an
absolute path, a path with `..`, or a symlink or hardlink that points outside
the package. Such entries could write files outside the cache, so the
dependency is rejected.

Use another version or source of the library, and tell its author. The error
comes from fetching, so it has no example here.

## E4324

The workspace's addons depend on each other in a circle through
`addon-dependencies.required` (A requires B, and B requires A). The client
loads a required dependency first, so none of them could load.

Make one of the dependencies optional (`addon-dependencies.optional`).

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon-dependencies": {
        "required": ["PlaygroundOptions"]
    },
    "flavors": {
        "camelot": {}
    },
    "workspace": {
        "members": ["options"],
        "default-members": ["."]
    }
}
```
```json fails title=options/wowlua.json
{
    "package": {
        "name": "playground-options",
        "version": "0.1.0"
    },
    "addon-dependencies": {
        "required": ["Playground"]
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon-dependencies": {
        "optional": ["PlaygroundOptions"]
    },
    "flavors": {
        "camelot": {}
    },
    "workspace": {
        "members": ["options"],
        "default-members": ["."]
    }
}
```
```json compiles title=options/wowlua.json
{
    "package": {
        "name": "playground-options",
        "version": "0.1.0"
    },
    "addon-dependencies": {
        "required": ["Playground"]
    },
    "flavors": {
        "camelot": {}
    }
}
```

## E4325

Two members of a workspace are the same AddOn: their `addon.name` is equal
(ignoring case), or one's default name, the PascalCase form of its package
name, equals the other's. The AddOn name is the folder in `Interface\AddOns`,
so both would be installed into the same folder.

Give each addon its own `addon.name`.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "workspace": {
        "members": ["options"],
        "default-members": ["."]
    }
}
```
```json fails title=options/wowlua.json
{
    "package": {
        "name": "playground-options",
        "version": "0.1.0"
    },
    "addon": {
        "name": "Playground"
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "workspace": {
        "members": ["options"],
        "default-members": ["."]
    }
}
```
```json compiles title=options/wowlua.json
{
    "package": {
        "name": "playground-options",
        "version": "0.1.0"
    },
    "addon": {
        "name": "PlaygroundOptions"
    },
    "flavors": {
        "camelot": {}
    }
}
```

Related: E4120

## E4327

A wowlua library dependency does not support a flavor your package targets.
A library supports every flavor unless its manifest lists `flavors`; then it
supports only those.

If you need the library only on some flavors, restrict the dependency with
`flavors`, and use it only in code for those flavors (`@if(flavor == …)`).
Otherwise ask the library's author to support the flavor, or stop targeting
it. The error comes from a dependency's manifest, so it has no example
here.

## E4330

`wowlua publish --tag <tag>` names a tag that already exists but points at
another commit than the one being published. wowlua creates a missing tag at
the current commit, but it never moves an existing one, so the release would
not match its tag.

Use a new tag (and version), or check out the tagged commit. The error needs
git, so it has no example here.

Related: E4319

## E4331

`wowlua dev` or `wowlua install` would replace an AddOn folder that wowlua did
not install: its TOC has no `X-Wowlua-Build` line. That is usually the same
addon installed by an addon manager, and wowlua leaves it alone rather than
delete files it did not make.

Remove the folder (or uninstall the addon in the addon manager), or pass
`--force` to replace it. The error comes from installing into the game, so it
has no example here.

Related: W4309

## E4334

The package root has a hand-written `Bindings.xml` (or a flavor variant such as
`Bindings_Mainline.xml`), and the code declares keybindings with `@binding`
from `wowlua:bindings`, which generates a `Bindings.xml`. The client reads only
one keybinding file per AddOn, so one set of bindings would be lost.

Declare every binding with `@binding` and delete the hand-written file, or keep
the file and drop `wowlua:bindings`.

```xml fails title=Bindings.xml
<Bindings>
    <Binding name="PLAYGROUND_HIDE">PlaygroundHide()</Binding>
</Bindings>
```
```wowlua fails title=src/main.wowlua
import binding from "wowlua:bindings"

@binding("PLAYGROUND_SHOW", { label = "Show" })
local function show()
    print("show")
end
```

```wowlua compiles
import binding from "wowlua:bindings"

@binding("PLAYGROUND_SHOW", { label = "Show" })
local function show()
    print("show")
end

@binding("PLAYGROUND_HIDE", { label = "Hide" })
local function hide()
    print("hide")
end
```

## E4335

A Lua or XML file of a converted packager project (`compat.packager-keywords`)
uses a game version block such as `--@version-mists@` in a build for several
flavors. The BigWigs packager refuses these blocks in a build for several game
versions, and wowlua does the same.

Check the game at run time with `WOW_PROJECT_ID`, or put the code in a file
that only some flavors load (`build.flavor-files`). A project that releases
one game at a time can set `compat.per-game`, which applies the blocks per
flavor.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {},
        "mists": {}
    },
    "compat": {
        "packager-keywords": true
    }
}
```
```lua fails title=src/main.lua
--@version-mists@
print("Mists only")
--@end-version-mists@
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {},
        "mists": {}
    },
    "compat": {
        "packager-keywords": true
    }
}
```
```lua compiles title=src/main.lua
if WOW_PROJECT_ID == WOW_PROJECT_MISTS_CLASSIC then
    print("Mists only")
end
```

Related: W4031, W4030

## E4336

`wowlua publish` found uncommitted changes in the working tree. A release
should be built from a commit, so that its version and source maps match what
is in the repository.

Commit or stash the changes, or pass `--allow-dirty`, which publishes an alpha
build under `"release-type": "auto"`. The error comes from publishing, so it
has no example here.

## E4400

A world profile (`tests/profiles/<name>.json`) is not valid JSON, or is not a
JSON object. World profiles describe the character and world that tests and
playground runs start in, and like the manifest they are strict JSON: no
comments, no trailing commas.

Fix the JSON at the position the message gives.

```json fails title=tests/profiles/rich.json
{ "money": 123456, }
```

```json compiles title=tests/profiles/rich.json
{ "money": 123456 }
```

Related: E4401, E4402

## E4401

A world profile does not match its schema: an unknown part (the emulated world
would ignore it), a part of the wrong type, or a key that should be an ID but
is not (parts such as `items` and `spells` are keyed by IDs). Fields of a part
beyond the schema's are allowed and passed on as they are.

Fix the part's name or value. An editor completes the parts when the profile
names `schema/world-profile-0.1.json` in `$schema`.

```json fails title=tests/profiles/rich.json
{ "monies": 123456 }
```

```json compiles title=tests/profiles/rich.json
{ "money": 123456 }
```

Related: E4400, E4402

## E4402

`test.profile` in the manifest, or a test file's `config({ profile = … })`,
names a world profile the package does not have. Profiles are the files in
`tests/profiles/`, named without `.json`.

Add `tests/profiles/<name>.json`, or fix the name; the help lists the profiles
that exist.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "test": {
        "profile": "rich"
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "test": {
        "profile": "rich"
    }
}
```
```json compiles title=tests/profiles/rich.json
{ "money": 123456 }
```

Related: E4400, E4401

## W4001

A key of `addon.toc-extra` is neither a TOC directive wowlua knows nor a
custom field starting with `X-`. The client ignores directives it does not
know, and custom metadata fields, which addons read with
`C_AddOns.GetAddOnMetadata`, are meant to start with `X-`.

Rename the key to `X-<Name>` (`X-Website`). Directives that wowlua writes
itself (`Title`, `Interface`, `SavedVariables`, …) are E4002 in `toc-extra`;
set them through their manifest keys.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon": {
        "toc-extra": { "Website": "https://example.com" }
    },
    "flavors": {
        "camelot": {}
    }
}
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "addon": {
        "toc-extra": { "X-Website": "https://example.com" }
    },
    "flavors": {
        "camelot": {}
    }
}
```

Related: E4002

## W4030

A Lua or XML file of a converted packager project (`compat.packager-keywords`)
uses the BigWigs packager's `@localization(…)@` keyword, which the packager
replaces with translations from CurseForge's localization tool. wowlua does
not support it and leaves the line unchanged, so it stays a comment and the
translations are missing.

Move the translations to `wowlua:locale`, whose locale files live in the
package, or write the strings into the file.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "compat": {
        "packager-keywords": true
    }
}
```
```lua fails title=src/main.lua
local L = {}
--@localization(locale="enUS", format="lua_additive_table")@
print(L)
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "compat": {
        "packager-keywords": true
    }
}
```
```lua compiles title=src/main.lua
local L = {}
L["Hello"] = "Hello"
print(L)
```

Related: W4031, E4335

## W4031

A Lua or XML file of a converted packager project (`compat.packager-keywords`)
has a `--@retail@` (or `--@non-retail@`) block, and the build covers several
flavors. Like the BigWigs packager in a multi-version build, wowlua leaves the
block as written, so the code in a `--@retail@` block runs on every client,
not only on Retail.

Check the game at run time with `WOW_PROJECT_ID`. A project that releases one
game at a time can set `compat.per-game`, which applies the blocks per flavor.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {},
        "mists": {}
    },
    "compat": {
        "packager-keywords": true
    }
}
```
```lua fails title=src/main.lua
--@retail@
print("Retail only")
--@end-retail@
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {},
        "mists": {}
    },
    "compat": {
        "packager-keywords": true
    }
}
```
```lua compiles title=src/main.lua
if WOW_PROJECT_ID == WOW_PROJECT_MAINLINE then
    print("Retail only")
end
```

Related: E4335, W4030

## W4220

A publish site does not know one of the package's flavors at all, so that
flavor is left out of the game versions of that site's upload. WoWInterface's
version list, for example, does not cover every flavor. The file still
uploads, but the site does not list it for that flavor.

Restrict the target's `flavors` to the ones the site knows to silence the
warning. It comes from publishing, so it has no example here.

Related: W4328

## W4304

The flavors need their files in different orders: in one flavor `a` must load
before `b`, in another `b` before `a` (below, because an import depends on the
flavor). A single TOC lists the files once, in one order, so with
`"toc-layout": "auto"` wowlua falls back to one TOC per flavor, each in its
own order. The AddOn works; the warning says why its TOC layout changed.

To keep one TOC, remove the conflicting load-time dependencies. To accept one
TOC per flavor without the warning, set `"toc-layout": "per-flavor"`.

```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {},
        "mists": {}
    }
}
```
```wowlua fails title=src/a.wowlua
import flavor from "wowlua:meta"
@if(flavor == "camelot")
import b from "./b"
export local a = "a"
print(a, flavor)
```
```wowlua fails title=src/b.wowlua
import flavor from "wowlua:meta"
@if(flavor == "mists")
import a from "./a"
export local b = "b"
print(b, flavor)
```

```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {},
        "mists": {}
    },
    "build": {
        "toc-layout": "per-flavor"
    }
}
```
```wowlua compiles title=src/a.wowlua
import flavor from "wowlua:meta"
@if(flavor == "camelot")
import b from "./b"
export local a = "a"
print(a, flavor)
```
```wowlua compiles title=src/b.wowlua
import flavor from "wowlua:meta"
@if(flavor == "mists")
import a from "./a"
export local b = "b"
print(b, flavor)
```

Related: E4301

## W4306

An embedded library has no license wowlua can find. Every embedded library's
license goes into the AddOn's `LICENSES.txt`. wowlua looks for a `LICENSE` or
`COPYING` file in the library's folder and its package's `license`; for a
plain Lua library also a license header in its entry file or the files
`lua-library.license-files` names. A wowlua library in a workspace (or in the
folder of the package that uses it) without a license of its own has its root
package's.

Check the library's license before you publish: you may not be allowed to
pass on code without one. Add the license file to the library, or ask its
author to. The warning comes from a dependency's files, so it has no example
here.

Related: E4110

## W4309

`wowlua dev --link` could not create a symlink (macOS, Linux) or directory
junction (Windows) from the game's AddOns folder to the build, so it copies
the build instead. The AddOn works the same; later builds are copied as well
instead of updated in place.

Check that the AddOns folder is writable and on a file system that supports
links. The warning comes from installing into the game, so it has no example
here.

Related: E4331

## W4310

An XML file loads a file by a path that differs in case from the file's name
(`Core.lua` for `core.lua`). The game resolves the path regardless of case, so
the AddOn loads, but the mismatch is easy to miss and breaks tools that care
about case.

Write the path exactly as the file is named.

```xml fails title=src/main.xml
<Ui xmlns="http://www.blizzard.com/wow/ui/">
    <Script file="Core.lua"/>
</Ui>
```
```lua fails title=src/core.lua
print("core")
```

```xml compiles title=src/main.xml
<Ui xmlns="http://www.blizzard.com/wow/ui/">
    <Script file="core.lua"/>
</Ui>
```
```lua compiles title=src/core.lua
print("core")
```

Related: E4308

## W4313

`wowlua types sync` moved a client alias (`anniversary`, `titan` or `forever`)
to another ruleset. These aliases name a client and stand for the ruleset that
client runs in the pinned flavor table; when Anniversary realms move on to
the next expansion, for example, `anniversary` names another flavor. The
manifest gets the new meaning only once you accept the lockfile update.

Check that your addon supports the new ruleset, or write the flavor ID
(`tbc`, `wrath`, `camelot`) instead of the alias. The warning comes from
`wowlua types sync`, so it has no example here.

Related: E4004

## W4322

A dependency's locked version has been yanked from the registry by its
author, usually because of a serious bug. The locked version keeps working,
but a new resolution never chooses it.

Run `wowlua update` for the dependency to move to a version that is not
yanked. The warning needs the registry, so it has no example here.

## W4326

`wowlua login` found no OS credential store (macOS Keychain, Windows
Credential Manager, Secret Service on Linux), so it saved the token in
`config.json` in the wowlua home folder, readable only by you (mode 0600).

The token works the same. To keep it in a credential store instead, install or
unlock one (on Linux, a Secret Service such as GNOME Keyring) and run
`wowlua login` again. The warning comes from `wowlua login`, so it has no
example here.

Related: E4201

## W4328

A publish site does not know the exact game version of a flavor's interface
number (often right after a game patch), so wowlua used the highest version
below it, or the highest version of that flavor the site knows. The message
names the site, the flavor and the version chosen.

Usually nothing needs doing; check the file's game versions on the site if
the choice matters. The warning comes from publishing, so it has no example
here.

Related: W4220

## W4329

A `github` publish target was skipped because the build is not on a git tag
and no `--tag` was given. A GitHub release belongs to a tag, so an untagged
build creates none; the other targets still upload.

Tag the commit, or pass `--tag <version>`, which creates the tag locally and
the release on GitHub with it. The warning comes from publishing, so it has
no example here.

Related: E4319, E4330

## W4332

`wowlua init` found a `.pkgmeta` `move-folders` entry it cannot convert. It
converts entries that move whole addon folders (they become workspace
members), make an external an AddOn of its own
(`workspace.external-addons`), or move a library's inner folder up (the
dependency's `subdir`). Other uses are reported and left out of the new
manifest.

Recreate the entry's effect by hand, for example with a workspace member,
`build.assets` or a dependency's `install-path`. The warning comes from
`wowlua init`, so it has no example here.

Related: W4333

## W4333

A `.pkgmeta` external is a Mercurial (hg) repository, which wowlua cannot
fetch, so it is not converted into a dependency and the library is missing
from the AddOn. `wowlua init` reports it, and with `compat.pkgmeta` every
check and build does.

Replace the external with a git mirror or a release archive of the library,
or keep the library's files in the repository.

```yaml fails title=.pkgmeta
package-as: Playground

externals:
  libs/LibOld: https://hg.wowace.com/wow/libold/mainline
```
```json fails title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "compat": {
        "pkgmeta": true
    }
}
```

```yaml compiles title=.pkgmeta
package-as: Playground
```
```json compiles title=wowlua.json
{
    "package": {
        "name": "playground",
        "version": "0.1.0",
        "wowlua": "^0.12"
    },
    "flavors": {
        "camelot": {}
    },
    "compat": {
        "pkgmeta": true
    }
}
```

Related: E4005, W4332
