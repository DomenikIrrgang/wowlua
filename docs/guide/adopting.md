# Bring your addon

You don't have to rewrite anything. `wowlua init` turns the addon you ship
today into a wowlua project, leaves its code as it is, and builds the same
AddOn your players have now. Then you add types one file at a time, when you
want them.

## Three commands

Run them in your addon's folder, best with a clean git working tree. Inside a
git repository, files are moved with `git mv`, so their history follows them.

```console
$ wowlua init --from-pkgmeta
     Adopted Tally for standard, vanilla (3 files moved into src/, 0 in-game paths updated)
   Converted 4 externals from .pkgmeta
        Note TOC files are generated from wowlua.json now; the old Tally.toc, Tally_Vanilla.toc can be deleted
        Note .github/workflows/release.yml publishes with the BigWigs packager, which keeps working; to publish with wowlua, replace its packager steps with one
      - uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main
      - run: wowlua publish
        env: # the packager step's env (CF_API_TOKEN, WOWI_API_TOKEN, WAGO_API_TOKEN, GITHUB_OAUTH)
(runs with `-d`, which upload nothing, become `wowlua package`)
$ wowlua build --release      # compare dist/release/Tally with the packager's zip
$ wowlua publish --dry-run    # see what each site would get
```

Tally is a small example addon: a retail and a Classic Era TOC, four Ace3
libraries as packager externals in `.pkgmeta`, and a release workflow that
runs the BigWigs packager.

`.pkgmeta` (or `pkgmeta.yaml`) is converted whenever it exists.
`--from-pkgmeta` makes it required, and `--from-pkgmeta <file>` names another
packager project file. An addon without one is adopted from its TOC files
alone with plain `wowlua init`; `--from-toc <file>` picks the TOC when there
are several.

## What changes in your folder

Before:

```text
Tally/
├── .github/workflows/release.yml   runs BigWigsMods/packager@v2
├── .pkgmeta
├── CHANGELOG.md
├── README.md
├── Tally.toc                       ## Interface: 120100
├── Tally_Vanilla.toc               ## Interface: 11509
├── embeds.xml
├── Locale.lua
├── Core.lua
└── Media/icon.tga
```

After `wowlua init --from-pkgmeta`:

```text
Tally/
├── .github/workflows/release.yml   unchanged
├── .pkgmeta                        unchanged
├── CHANGELOG.md
├── README.md
├── Tally.toc                       no longer used, delete when you like
├── Tally_Vanilla.toc               no longer used, delete when you like
├── wowlua.json                     what the TOCs and .pkgmeta said
├── wowlua.lock                     the pinned interface numbers
├── src/
│   ├── embeds.xml
│   ├── Locale.lua                  contents untouched
│   └── Core.lua
└── Media/icon.tga                  assets stay where they are
```

- The title, notes, SavedVariables, dependencies, `X-` fields and interface
  versions move from the TOCs into `wowlua.json`. The interface versions give
  the flavors, here `standard` and `vanilla`. TOCs are generated from now on.
- Every Lua and XML file the TOCs load moves into `src/`, and absolute
  in-game paths to them (`Interface\AddOns\Tally\…`) are rewritten. If the
  addon already has a `src` folder, the sources go to `wowlua-src/`.
- The load order is kept exactly, in `build.load-order`, per flavor where the
  TOCs differ. Files only some flavors load go into `build.flavor-files`.
- `.pkgmeta` externals become dependencies, `ignore` becomes `build.exclude`,
  and `X-Curse-Project-ID`, `X-Wago-ID` and `X-WoWI-ID` become publish
  targets.
- Textures, sounds and fonts stay where they are and become `build.assets`,
  so their paths don't change.
- `wowlua.lock` pins the interface numbers, so the first build writes the same
  `## Interface` lines.

Lua and XML files that no TOC loads are listed in a warning and left in
place. Part of Tally's new `wowlua.json`:

```json
{
  "addon": {
    "title": "Tally",
    "notes": "Counts your kills.",
    "saved-variables": ["TallyDB"]
  },
  "flavors": {
    "standard": {},
    "vanilla": {}
  },
  "build": {
    "assets": ["Media/**/*.{tga,blp,png,jpg,jpeg,ogg,mp3,wav,ttf,otf}"],
    "load-order": ["src/embeds.xml", "src/Locale.lua", "src/Core.lua"],
    "libs-dir": "Libs",
    "exclude": ["README.md/**", "README.md"]
  },
  "dependencies": {
    "aceaddon-3-0": {
      "svn": "https://repos.wowace.com/wow/ace3/trunk/AceAddon-3.0",
      "kind": "lua-library",
      "curseforge-slug": "ace3",
      "install-path": "AceAddon-3.0",
      "files": ["AceAddon-3.0.xml"]
    }
  }
}
```

The full file also has the other three dependencies, the CurseForge and Wago
targets, the changelog, and the `compat` settings and `release` profile of
the next section. Every step of `init`, with its edge cases, is in
[CLI §12.6](../../spec/toolchain/cli.md#126-adopting-an-existing-addon).

> [!NOTE]
> The old TOC files stay until you delete them, and `init` doesn't touch
> `.gitignore`: add `dist/` to it yourself.

## The same AddOn as the packager

An adopted packager project builds what the packager builds: the same loaded
code, the same shipped files, line endings and TOC versions, with
`@project-version@`, `@debug@` blocks and the flavor blocks replaced as the
packager replaces them. This is checked on 36 addons, including BigWigs, DBM,
WeakAuras, ElvUI and Details.

To get there, `init` writes `compat` settings
([Manifest §9.14](../../spec/toolchain/manifest.md#914-compat)):

```json
"compat": {
  "packager-keywords": true,
  "line-endings": "crlf",
  "toc-version": "@project-version@"
}
```

It also writes a `release` profile without bundling, minification or
optimizations, like the packager's zip. When you want smaller release builds
that load faster, remove those overrides from `profiles.release`
([Building and publishing](publishing.md#how-small-should-it-get)).

A repository whose addons all come from `.pkgmeta` `move-folders`, such as
WeakAuras, DBM or ElvUI, becomes a
[workspace](packages.md#addon-suites-in-one-workspace). You can also keep
`.pkgmeta` as the source of truth for dependencies and publishing, with
`"compat": { "pkgmeta": true }`; `wowlua.json` then only adds wowlua's own
settings, and a key both files set is E4005.

### How .pkgmeta maps to wowlua.json

| `.pkgmeta` | `wowlua.json` |
|------------|---------------|
| `externals` | `dependencies`, with `build.libs-dir` and an `install-path` each |
| `ignore` | `build.exclude` |
| `plain-copy` | `build.assets` |
| `package-as` | `addon.name` |
| `manual-changelog` | `publish.changelog` |
| `move-folders` | workspace members, or `workspace.external-addons` |
| `license-output` | `compat.license-output` |
| `changelog-title` | `compat.changelog-title` |
| `enable-nolib-creation` | `compat.nolib-package` |
| `wowi-convert-changelog`, `wowi-create-changelog`, `wowi-archive-previous` | the WoWInterface target's `changelog-output`, `changelog` and `archive-previous` |
| `required-dependencies`, `optional-dependencies`, `embedded-libraries`, `tools-used` | CurseForge relations |

git, SVN and zip externals convert. URLs are read the way the packager reads
them: old CurseForge URLs, SVN `trunk` and `tags/` paths, and sub-paths of
CurseForge and GitHub git URLs. hg externals are listed with W4333 and need a
git mirror or an archive instead.

## Your CI keeps working

`init` leaves `.github/workflows` alone, so a workflow that runs the packager
keeps publishing. For each one, it prints how to switch: the packager step
becomes two steps with the same secrets. Before:

```yaml
- uses: BigWigsMods/packager@v2
  env:
    CF_API_KEY: ${{ secrets.CF_API_KEY }}
    GITHUB_OAUTH: ${{ secrets.GITHUB_TOKEN }}
```

After:

```yaml
- uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main
- run: wowlua publish
  env:
    CF_API_KEY: ${{ secrets.CF_API_KEY }}
    GITHUB_OAUTH: ${{ secrets.GITHUB_TOKEN }}
```

A converted project publishes the way the packager does. There is no clean
working tree check, the targets upload in the packager's order (CurseForge,
WoWInterface, Wago, GitHub), and a target without a token is skipped with a
note. Tokens come from the packager's environment variables (`CF_API_KEY` or
`CF_API_TOKEN`, `WAGO_API_TOKEN`, `WOWI_API_TOKEN`, `GITHUB_OAUTH`, …; not
`GITHUB_TOKEN`) and from its `.env` file, so the secrets you have keep
working. Runs with `-d`, which upload nothing, become `wowlua package`.
[Publishing §11.9](../../spec/toolchain/publishing.md#119-the-publish-command)
has the details.

## Try it in the game

The addon builds as it is. Run `wowlua dev`, `/reload` in the game, and type
`/wl`: the [dev window](debugging.md#in-the-game-the-dev-window) profiles and
monitors your existing Lua with no change to your code
([Getting started](getting-started.md#run-it-in-the-game) explains
`wowlua dev`).

## Then add types, file by file

Every Lua 5.1 file is already valid wowlua. When you want types in a file,
rename it from `.lua` to `.wowlua`. In a file that uses no wowlua syntax,
name and type problems are only warnings, so a renamed file builds as
before. They become errors once the file uses wowlua syntax, so fix them at
your own pace. In `src/Core.lua`:

```lua
local function addKill(name, count)
    count = count or 1
    TallyDB[name] = (TallyDB[name] or 0) + count
end
```

Later, in `src/Core.wowlua`, with typed parameters, a default value and `??`:

```wowlua fragment
local function addKill(name: string, count: number = 1)
    TallyDB[name] = (TallyDB[name] ?? 0) + count
end
```

To get hints in the files you haven't renamed yet, set
`"check": { "lua": true }`: it type-checks the remaining `.lua` files too,
with warnings only.

### Already using LuaLS annotations?

Keep the work you put into `---@class` and `---@param`. `wowlua types
convert` turns them into a `.d.wowlua` declaration file, from one file or a
whole folder:

```console
$ wowlua types convert src     # writes src.d.wowlua
```

`--out <dir>` names the folder to write it to; the default is the current
one.

> [!TIP]
> Open the project in [VS Code](vscode.md) with the wowlua extension. It
> checks your old `.lua` files as you type and finds definitions in them, as
> it does in the new `.wowlua` ones.

## Where to go next

- [A tour of wowlua](../language/tour.md): what you can write once a file is
  `.wowlua`.
- [Testing your addon](testing.md) without starting the game.
- [Building and publishing](publishing.md) with bundled release builds.
- [WoW API](../reference/api.md): what the checker knows about the game's
  functions.
