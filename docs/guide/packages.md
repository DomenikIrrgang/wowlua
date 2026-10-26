# Libraries and dependencies

Use the Lua libraries you already rely on, share code between your own
addons, and ship an addon suite from one repository. wowlua fetches, locks and
embeds every dependency, so there is no `Libs/` folder to copy around by hand.

## Add a Lua library

Most WoW libraries are plain Lua. Point `wowlua add` at where the library
lives and declare it a `lua-library`. Git, SVN, a local folder and a release
archive all work:

```console
$ wowlua add ace3 --git https://github.com/WoWUIDev/Ace3 --tag Release-r1390 --kind lua-library \
    --files LibStub/LibStub.lua,CallbackHandler-1.0/CallbackHandler-1.0.xml,AceAddon-3.0/AceAddon-3.0.xml,AceEvent-3.0/AceEvent-3.0.xml
     Cloning https://github.com/WoWUIDev/Ace3
      Adding ace3 to dependencies
     Locking ace3 0.0.0+17db0bc6b72e
$ wowlua add libstub --svn https://repos.wowace.com/wow/libstub/tags/1.0 --kind lua-library
    Updating https://repos.wowace.com/wow/libstub/tags/1.0
    Fetching https://repos.wowace.com/wow/libstub/tags/1.0@90
      Adding libstub to dependencies
     Locking libstub 0.0.0+90
```

A git source takes one of `--tag`, `--branch` or `--rev`, and `--subdir` for a
library in a folder of the repository. An SVN source takes `--rev`. A release
archive takes `--archive <url>` with its `--sha256`, and `--strip-prefix` for
the folder inside it. `--path ../SomeLib` uses a folder on your disk.

Each `add` writes an entry into `wowlua.json`, keeping your formatting, and
pins the exact commit or revision in `wowlua.lock`. When the library can't be
fetched, `wowlua.json` stays as it was:

```json
"dependencies": {
  "ace3": {
    "git": "https://github.com/WoWUIDev/Ace3",
    "tag": "Release-r1390",
    "kind": "lua-library",
    "files": [
      "LibStub/LibStub.lua",
      "CallbackHandler-1.0/CallbackHandler-1.0.xml",
      "AceAddon-3.0/AceAddon-3.0.xml",
      "AceEvent-3.0/AceEvent-3.0.xml"
    ]
  }
}
```

Then use the library as you always have. Its entry files load before your own
files, the libraries in the order you declared them:

```lua title=src/core.lua
local MyAddon = LibStub("AceAddon-3.0"):NewAddon("MyAddon", "AceEvent-3.0")

function MyAddon:OnEnable()
    self:RegisterEvent("PLAYER_MONEY")
end

function MyAddon:PLAYER_MONEY()
    print("Money:", GetMoney())
end
```

> [!TIP]
> Much of what addons embed libraries for (events, saved variables, slash
> commands, timers, options, a minimap button, addon messages) is in the
> [standard library](../stdlib/overview.md), typed and compiled into your
> addon. Mixing both is fine.

## Only what the game loads ships

A library repository has tests, docs and tooling your players never need.
wowlua copies the entry files, everything they reach through XML `<Script>`
and `<Include>`, and the library's license file, into `libs/<name>/` of your
AddOn, and nothing else. The Ace3 repository holds many more libraries; the
dev build of the addon above has only the four it loads:

```text
dist/dev/MyAddon/
├── MyAddon.toc            generated, lists the libraries before your files
├── libs/
│   └── ace3/
│       ├── AceAddon-3.0/AceAddon-3.0.lua, AceAddon-3.0.xml
│       ├── AceEvent-3.0/AceEvent-3.0.lua, AceEvent-3.0.xml
│       ├── CallbackHandler-1.0/CallbackHandler-1.0.lua, CallbackHandler-1.0.xml
│       ├── LibStub/LibStub.lua
│       └── LICENSE.txt
├── src/
│   ├── core.lua
│   └── main.lua
├── wowlua/                the parts of wowlua's runtime the addon uses
└── LICENSES.txt
```

You often don't need `files`. When the library has its own `wowlua.json`, a
single `.toc` file, or a single `lib.xml`, `embeds.xml` or `<name>.xml` in its
root, wowlua finds the entry files itself; the `libstub` entry above works
without them. Set `"libs-dir": "Libs"` under `build` if you prefer the
classic folder name, and `install-path` on a dependency to rename its folder.
A dependency with `"bundle": false` keeps its own files even in bundled
release builds.

## Libraries from the wowlua registry

Libraries published to the [wowlua registry](https://registry.wowlua.dev)
need no URL. Find one with `wowlua search`, by its name, description and
keywords:

```console
$ wowlua search tooltip
lib-tooltip  1.2.3  Tooltips for frames
       Found 1 package, 1–1 shown
```

Then add it by name, with a version requirement after `@`.

```console
$ wowlua add lib-tooltip@^1.2
    Updating https://registry.wowlua.dev index
 Downloading lib-tooltip 1.2.3
      Adding lib-tooltip@^1.2 to dependencies
     Locking lib-tooltip 1.2.3
```

The entry is just the requirement:

```json
"dependencies": {
  "lib-tooltip": "^1.2"
}
```

Without `@`, `wowlua add lib-tooltip` writes `^` and the newest version.
Requirements work like Cargo's: `^1.2` takes any `1.x` from `1.2.0` on,
`~1.2.3` any `1.2.x` from `1.2.3`, `=1.2.3` exactly that version, and
`>=1.0, <2.0` a range. wowlua picks the newest version that every
requirement allows, fetches the library's own registry dependencies too,
and checks every download against the checksum the registry published.

To use another registry, give its URL, or a name you define under
`registries` in `~/.wowlua/config.json`:

```json
"dependencies": {
  "lib-tooltip": { "version": "^1.2", "registry": "https://registry.example.com" }
}
```

Sometimes an author **yanks** a version, usually because of a serious bug.
If your lockfile has it, it keeps working and wowlua warns with W4322 until
you run `wowlua update lib-tooltip`; a new resolution never picks a yanked
version.

## The lockfile and staying up to date

`wowlua.lock` records the exact commit, SVN revision, archive hash or
registry version of every dependency, so every machine and your CI build the
same AddOn. Commit it.
Fetched code lives in a shared cache in `~/.wowlua/cache`, so a second addon
that uses the same library downloads nothing.

```console
$ wowlua outdated            # newer versions, tags, commits or revisions, and what to change
$ wowlua update              # move registry versions and branch and trunk pins forward
$ wowlua update ace3         # just one
$ wowlua tree                # the dependency graph
$ wowlua remove libstub
$ wowlua fetch               # download everything into the cache
$ wowlua cache clean         # drop cache entries unused for 30 days
```

`outdated` compares tags as version numbers, so it doesn't see newer tags
that aren't versions, such as Ace3's `Release-r1403`. A dependency on a tag
or an explicit revision stays there until you change it in `wowlua.json`;
`update` moves registry dependencies to the newest version their
requirement allows, and branches, default branches and SVN sources without
a revision to their newest commit. `wowlua update lib-tooltip --precise
1.2.0` pins one registry dependency to a version. Builds read the
registry's index from the cache when it has the locked versions; `fetch`,
`update`, `add` and `outdated` ask the registry again. In CI, pass
`--locked` so that a build fails instead of quietly changing the lockfile,
or `--frozen` to stay offline too.

## Depending on another addon

Sometimes you don't embed code but need another addon installed, such as a
core addon your plugin extends. That goes into the TOC as a dependency, not
into your folder:

```console
$ wowlua add --addon MyAddonCore --optional
      Adding MyAddonCore to addon-dependencies.optional
```

The generated TOC then has `## OptionalDeps: MyAddonCore`; without
`--optional`, the addon is required.

## Share code with a wowlua library

Have helpers you copy between addons? Make them a wowlua library. A library is
typed, and each addon that uses it gets its own private copy with only the
modules it imports, so two addons on different versions never step on each
other.

```console
$ wowlua new shared-utils --kind library
     Created library package in shared-utils
$ cd MyAddon
$ wowlua add shared-utils --path ../shared-utils
      Adding shared-utils to dependencies
     Locking shared-utils 0.1.0
```

In the library, `shared-utils/src/math.wowlua`:

```wowlua
export function clamp(v: number, lo: number, hi: number): number
    return math.max(lo, math.min(hi, v))
end
```

In the addon, a module of the library is imported as `<dependency>/<module>`:

```wowlua fragment
import clamp from "shared-utils/math"

export function scaleOf(value: number): number
    return clamp(value, 0.5, 2)
end
```

A path dependency is handy while you work on both. When the library has its
own repository, switch the entry to `git` with a tag, or
[publish it to the registry](publishing.md#publish-a-library-to-the-registry)
and depend on it by version. `wowlua check` in the
library's folder checks it on its own, for every flavor it supports, and
`wowlua test` runs its tests. The other templates are `--kind lua-library`
for a plain Lua library that others embed, and `--kind types` for type
declarations only; `wowlua check` checks them as an addon that uses them
would.

## Addon suites in one workspace

A core addon with load-on-demand modules, or a family of addons that share
libraries, can live in one repository as a workspace. It has one
`wowlua.lock` and one `dist/` at the root, and its libraries resolve
together. The root's `wowlua.json`:

```json
{
  "workspace": {
    "members": ["addons/*", "libs/*"],
    "default-members": ["addons/core"]
  }
}
```

```console
$ wowlua build                 # the default members, run at the root
$ wowlua dev -p my-suite-raid  # one member, by its package name
```

Inside a member's folder, commands work on that member and `-p` isn't
needed. A dependency named like a library member resolves to it without a
`path` entry.

- An addon can list a sibling under `addon-dependencies` and gets the
  sibling's global API typed, without any of its code.
- With `publish.bundle-addons`, the suite ships as one zip with several
  AddOn folders.
- Addons you ship unchanged next to yours, such as a voice pack, go under
  `workspace.external-addons`: fetched, locked and built into `dist/` with
  their own TOC.

The details are in
[Package manager §10.8](../../spec/toolchain/package-manager.md#108-workspaces).

## Where to go next

- [Building and publishing](publishing.md): embedded libraries are in every
  zip.
- [Modules](../language/modules.md): `import` and `export` across files and
  libraries.
- [Bring your addon](adopting.md): `.pkgmeta` externals become dependencies
  by themselves.
- [wowlua.json](../reference/manifest.md): every dependency field.
