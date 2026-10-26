# Building and publishing

Turn your project into the release build players download, and upload it to
CurseForge, Wago, WoWInterface and GitHub with one command, from your machine
or from CI on every tag.

## Dev builds and release builds

While you work, `wowlua dev` makes dev builds: one `.lua` file per source with
your line numbers kept, plus the profiler and the `/wl` dev window. A release
build is what players get: bundled into one file, comments and whitespace
stripped, and none of the debugging tools.

```console
$ wowlua build
    Finished MyAddon 0.1.0-dev (dev: camelot) in 0.09s → dist/dev/MyAddon
$ wowlua build --release
    Finished MyAddon 0.1.0-alpha (release: camelot) in 0.09s → dist/release/MyAddon
$ wowlua build --release --stats
    Finished MyAddon 0.1.0-alpha (release: camelot) in 0.10s → dist/release/MyAddon
       Stats 3 files (27 KiB), 1 Lua file (27 KiB), compiled by Lua 5.1 in 0.5 ms
```

Every build is a complete AddOn folder: a generated TOC, your code, embedded
libraries, assets, and only the parts of wowlua's runtime your code uses.
`--stats` adds the file count, the size and how long Lua 5.1 takes to compile
the Lua files, which is the parse cost the client pays at load.
`wowlua install --release` builds the release profile once and installs it
into your game, to try what players will get.

## Tuning a profile

The built-in profiles are `dev`, `release` and `test`. Change them under
`profiles` in `wowlua.json`, or add your own, which names the profile it
builds on with `inherits`. A common need is to keep developer-only files in
dev builds and tests, but out of what players download:

```json
"profiles": {
  "release": {
    "minify": "compact",
    "exclude": ["src/Diagnostics/**"]
  },
  "beta": { "inherits": "release", "log-level": "info" }
}
```

Code that stays in the build and imports an excluded file guards the import,
so the release build never asks for it. Say `src/Diagnostics/panel.wowlua`
holds a panel for you:

```wowlua title=src/Diagnostics/panel.wowlua
export function show()
    print("diagnostics")
end
```

Then `src/main.wowlua` imports it only in builds with `build.debug`:

```wowlua title=src/main.wowlua
import build from "wowlua:meta"
@if(build.debug)
import show from "/Diagnostics/panel"
import event from "wowlua:events"

@event("PLAYER_LOGIN")
local function onLogin()
    if build.debug then
        show()
    end
end
```

`build.debug` is a constant, so the release build drops the import and the
call. Build a custom profile with `wowlua build --profile beta`. More on `@if`
and build information is in [Build info and profiles](../language/flavors.md),
and every profile key in
[Manifest §9.9](../../spec/toolchain/manifest.md#99-profiles).

## How small should it get?

WoW reads and compiles every file of a TOC separately, so the cost per file
adds up for an addon with hundreds of small files. Release builds bundle them
into one. On top of that, you choose a minification level with `minify`:

| Level | What it does |
|-------|--------------|
| `none` | The code as compiled. Dev and test builds use it. |
| `strip` | Removes comments and extra whitespace and applies safe simplifications; names stay. The release default. |
| `compact` | Also the tightest layout within each line, and shorter literals |
| `rename` | Also shortens local names and hoists repeated strings |
| `max` | Also renames private and protected methods, and the fields of tables private to a module, where the name occurs nowhere else (not in a string, an API, XML or a library). Instance fields keep their names, since saved variables use them |

Every level keeps at least one line break per statement, so an error from a
player still points at one statement, and
[`wowlua trace`](#when-players-report-errors) maps it back. Blizzard's
add-on policy asks for code that isn't hidden or obfuscated, so CurseForge,
Wago and WoWInterface targets build unminified unless the target sets
`minify` itself, and then `wowlua publish` warns (W6010). Levels are
explained in
[Optimization §15.4](../../spec/toolchain/optimization.md#154-minification-levels).

## Versions come from git tags

You set `package.version` in `wowlua.json` and tag the commit you release.
wowlua writes the TOC's `## Version` and `build.version` from both:

- A release build on the tag `v1.4.2` (or `1.4.2`) of version `1.4.2`:
  `1.4.2`. A tag that doesn't match the version stops the build (E4319).
- A release build anywhere else, or with uncommitted changes:
  `1.4.2-alpha+3f9c2e1`.
- Dev and test builds: `1.4.2-dev+3f9c2e1`.

Without git, the `+<hash>` part is left out. The release type on the addon
sites follows the same rule: a build that is not on a tag, or has uncommitted
changes, is an `alpha`; a tag that contains `alpha` or `beta` is that; any
other tag is a `release`. `publish.release-type`, or `--release-type` on the
command line, sets it instead.

## Set up your publish targets

Add one target per place you upload to. The names are yours, and `type`
picks the site, so you can have two CurseForge targets, for example a normal
one and a `nolib` one without the embedded Lua libraries:

```json
"publish": {
  "changelog": "CHANGELOG.md",
  "targets": {
    "curseforge": { "type": "curseforge", "project-id": 123456 },
    "cf-nolib": { "type": "curseforge", "project-id": 123457, "nolib": true },
    "wago": { "type": "wago", "project-id": "a1b2c3d4" },
    "wowi": { "type": "wowinterface", "project-id": 5678 },
    "github": { "type": "github", "repository": "you/my-addon", "minify": "rename" }
  }
}
```

From a changelog with version headings (`## 1.4.2`), each upload gets the
section of its version; `"changelog": "git"` lists the commits since the
previous tag instead. Tokens never go into `wowlua.json`. Save each one once
in your system's credential store:

```console
$ wowlua login curseforge
$ wowlua login wago
$ wowlua login wowinterface
$ wowlua login github
```

`login` asks for the token in a hidden prompt, or reads it from standard
input or `--token`. CurseForge and GitHub tokens are checked with a
read-only request before they are saved. In CI, set environment variables
instead: `CF_API_TOKEN`, `WAGO_API_TOKEN`, `WOWI_API_TOKEN` and
`GITHUB_TOKEN`. All but the last are the names the BigWigs packager uses, so
the secrets you have keep working.

## Publish

Look before you upload. A dry run builds, tests and packages everything, then
prints what each site would receive, without uploading anything:

```console
$ wowlua publish --dry-run          # build, test, package, show what each site gets
$ git tag v1.4.2 && wowlua publish  # publish a tagged commit
$ wowlua publish --tag v1.4.2       # or let wowlua create the tag at HEAD
$ wowlua publish --target curseforge
$ wowlua package                    # just the zips, in dist/packages/
```

`wowlua publish` goes through these steps:

1. **Checks first.** The working tree must be clean (`--allow-dirty` makes an
   alpha anyway), the lockfile must not change, and every target needs its
   token, all before anything is uploaded.
2. **Builds and tests.** Each target's build is made and validated, then
   `wowlua test` runs. A failing test stops the release; `--no-test` skips
   the tests.
3. **Packages.** The zips go to `dist/packages/`, one per distinct target
   build.
4. **Uploads.** The zips go to every target in parallel, with retries. A site
   that fails doesn't stop the others.
5. **Keeps the source maps.** They are archived in `~/.wowlua/maps/`, and
   attached to the GitHub release, so you can decode error reports from this
   version months later.
6. **Sums up.** The summary lists the file IDs and links, and the exit code
   is not zero if a target failed.

`--tag` creates the tag locally and never pushes it: the GitHub release
creates the tag on GitHub. Without a tag, the GitHub target creates no
release and warns (W4329). The command and every site's protocol are in
[Publishing](../../spec/toolchain/publishing.md).

## Publish from CI on every tag

Start a project with `wowlua new MyAddon --ci github` and you get a GitHub
Actions workflow, `.github/workflows/ci.yml`, that checks, tests and builds
every push to `main` and every pull request, and publishes when you push a
`v*` tag:

```yaml
name: ci
on:
  push:
    branches: [main]
    tags: ["v*"]
  pull_request:
jobs:
  build:
    runs-on: ubuntu-latest
    permissions:
      contents: write # GitHub releases
    steps:
      - uses: actions/checkout@v5
        with: { fetch-depth: 0 } # tags and history for build.version and changelogs
      - uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main
      - uses: actions/cache@v4
        with:
          path: ~/.wowlua/cache
          key: wowlua-${{ hashFiles('wowlua.lock') }}
      - run: wowlua check --locked --message-format github
      - run: wowlua test --locked --message-format github
      - run: wowlua build --release --locked
      - if: startsWith(github.ref, 'refs/tags/')
        run: wowlua publish
        env:
          CF_API_TOKEN: ${{ secrets.CF_API_TOKEN }}
          WAGO_API_TOKEN: ${{ secrets.WAGO_API_TOKEN }}
          WOWI_API_TOKEN: ${{ secrets.WOWI_API_TOKEN }}
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

`fetch-depth: 0` fetches the tags and history the version and the changelog
need. `--message-format github` turns errors and failed tests into
annotations on the pull request. The tests run in wowlua's emulated client, so
the runner needs no game and no display. Releasing is then
`git tag v1.4.2 && git push origin v1.4.2`. A library's workflow (`--kind
library`, `lua-library` or `types`) only checks and tests: a library is not
published on its own.

## When players report errors

A bundled build is one big file, so an error says `MyAddon.lua:1022`. Paste
the report from the game into `wowlua trace`, and it points at your source,
even in a build that renamed your locals:

```console
$ wowlua trace --clipboard
MyAddon/MyAddon.lua:1022: attempt to index local 'a' (a nil value)
  → src/ui/window.wowlua:6 in Window:refresh   ('a' is 'frame')
[string "@Interface/AddOns/MyAddon/MyAddon.lua"]:1022: in function <MyAddon/MyAddon.lua:1020>
  → src/ui/window.wowlua:6 in Window:refresh
  → src/ui/window.wowlua:4 in Window:refresh
[string "@Interface/AddOns/MyAddon/MyAddon.lua"]:1033: in function <MyAddon/MyAddon.lua:1032>
  → src/main.wowlua:6 in onLogin
  → src/main.wowlua:4
```

The build id in the TOC (`X-Wowlua-Build`) tells `trace` which source maps to
use, so a report from an old version is decoded with that version's maps.
[Debugging](debugging.md#map-errors-back-to-your-source) has more on it.

## Where to go next

- [Bring your addon](adopting.md): switch a packager workflow to
  `wowlua publish`.
- [Testing your addon](testing.md): the tests every publish runs.
- [wowlua.json](../reference/manifest.md): every target and profile key.
- [WoW API](../reference/api.md): the game functions your addon calls, typed.
