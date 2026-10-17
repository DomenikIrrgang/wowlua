# Adopting an addon

`wowlua init` turns an existing addon into a wowlua project without changing
its code. For an addon built with the BigWigs packager, `init` also converts
`.pkgmeta`, and the build matches the packager's: the same loaded code, the
same shipped files, line endings and TOC versions. This is checked on 36
addons, including BigWigs, DBM, WeakAuras, ElvUI and Details.

## Run it

Run `init` in the addon's folder, best with a clean Git working tree. Inside a
Git repository, files are moved with `git mv`, so their history follows them.

```sh
cd MyAddon
wowlua init --from-pkgmeta     # TOCs and .pkgmeta → wowlua.json
wowlua build --release         # compare with the packager's zip
wowlua publish --dry-run       # see what would be uploaded
```

`.pkgmeta` (or `pkgmeta.yaml`) is converted whenever it exists.
`--from-pkgmeta` makes it required, and `--from-pkgmeta <file>` names another
packager project file. An addon without `.pkgmeta` is adopted from its TOC
files alone with `wowlua init`; `--from-toc <file>` picks the TOC.

## An example

Tally is a small addon with a retail and a Classic Era TOC, three Ace3
libraries as packager externals, and a release workflow that runs the
packager:

```text
Tally/
├── .github/workflows/release.yml   runs BigWigsMods/packager@v2
├── .pkgmeta
├── Tally.toc                       ## Interface: 120100
├── Tally_Vanilla.toc               ## Interface: 11509
├── embeds.xml
├── Locale.lua
├── Core.lua
└── Media/icon.tga
```

```yaml
package-as: Tally

externals:
  Libs/LibStub: https://repos.wowace.com/wow/libstub/tags/1.0
  Libs/CallbackHandler-1.0: https://repos.wowace.com/wow/callbackhandler/trunk/CallbackHandler-1.0
  Libs/AceAddon-3.0: https://repos.wowace.com/wow/ace3/trunk/AceAddon-3.0
  Libs/AceEvent-3.0: https://repos.wowace.com/wow/ace3/trunk/AceEvent-3.0

ignore:
  - README.md

manual-changelog: CHANGELOG.md
```

`wowlua init --from-pkgmeta` adopts it:

```text
     Adopted Tally for standard, vanilla (3 files moved into src/, 0 in-game paths updated)
   Converted 4 externals from .pkgmeta
        Note TOC files are generated from wowlua.json now; the old Tally.toc, Tally_Vanilla.toc can be deleted
        Note .github/workflows/release.yml publishes with the BigWigs packager, which keeps working; to publish with wowlua, replace its packager steps with one
      - uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main
      - run: wowlua publish
        env: # the packager step's env (CF_API_TOKEN, WOWI_API_TOKEN, WAGO_API_TOKEN, GITHUB_OAUTH)
(runs with `-d`, which upload nothing, become `wowlua package`)
```

The Lua and XML files moved into `src/`. The textures stayed where they were.
The new `wowlua.json` holds what the TOCs and `.pkgmeta` said; in part:

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
  },
  "compat": {
    "packager-keywords": true,
    "line-endings": "crlf",
    "toc-version": "@project-version@"
  }
}
```

The full file also has the other three dependencies, CurseForge and Wago
publish targets from the TOCs' `X-Curse-Project-ID` and `X-Wago-ID`, the
changelog, and a `release` profile without bundling, minification or
optimizations, like the packager's zip.

The first build fetches the externals into the global cache and locks them in
`wowlua.lock`:

```text
    Updating https://repos.wowace.com/wow/libstub/tags/1.0
    Fetching https://repos.wowace.com/wow/libstub/tags/1.0@90
    Updating https://repos.wowace.com/wow/callbackhandler/trunk/CallbackHandler-1.0
    Fetching https://repos.wowace.com/wow/callbackhandler/trunk/CallbackHandler-1.0@26
    Updating https://repos.wowace.com/wow/ace3/trunk/AceAddon-3.0
    Fetching https://repos.wowace.com/wow/ace3/trunk/AceAddon-3.0@1284
    Updating https://repos.wowace.com/wow/ace3/trunk/AceEvent-3.0
    Fetching https://repos.wowace.com/wow/ace3/trunk/AceEvent-3.0@1202
warning[W4306]: embedded libraries without a recognizable license file or header: callbackhandler-1-0, aceaddon-3-0, aceevent-3-0
 = help: check their licenses before publishing; `lua-library.license-files` can name the file

warning: 1 warning emitted

    Finished Tally 1.0.0-alpha+bccb7af (release: standard, vanilla) in 36.16s → dist/release/Tally
```

`dist/release/Tally/` has the generated TOC, `src/`, the four libraries under
`Libs/`, `Media/`, `CHANGELOG.md` and `LICENSES.txt`. With the `v1.0.0` tag, the
TOC's `@project-version@` became `## Version: v1.0.0`, as the packager writes
it. Add `dist/` to `.gitignore`: `init` does not.

## What init does

- Reads every TOC the client finds in the folder (`<Folder>.toc` and the
  flavor suffixes, the legacy ones included) and takes the metadata, the
  interface versions (which give the flavors), SavedVariables, dependencies
  and `X-` fields into `wowlua.json`.
- Moves every Lua and XML file the TOCs load into `src/` and rewrites
  absolute in-game paths to them (`Interface\AddOns\<Name>\…`). Textures,
  sounds and fonts stay where they are and become `build.assets`, so their
  paths do not change. If the addon already has a `src` folder, the sources go
  to `wowlua-src/`.
- Records the existing file order in `build.load-order`, per flavor where the
  TOCs differ, so the build loads files as before.
- Records files that only some flavors load in `build.flavor-files`, and
  metadata that differs between flavors as flavor overrides.
- Reads `## X-Curse-Project-ID`, `## X-Wago-ID` and `## X-WoWI-ID` into
  publish targets.
- Pins the interface numbers in `wowlua.lock`, so the first build writes the
  same `## Interface` lines.
- Leaves the contents of your `.lua` files alone, apart from those path
  rewrites.
- Leaves the old TOC files in place. TOCs are generated from now on; delete
  the old ones when you no longer need them.

Lua and XML files that no TOC loads are listed in a warning and left in
place. Every step, with its edge cases, is in
[CLI §12.6](../spec/toolchain/cli.md#126-adopting-an-existing-addon).

## How .pkgmeta maps to wowlua.json

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
git mirror or an archive instead. A repository whose addons all come from
`move-folders`, such as WeakAuras, DBM or ElvUI, becomes a workspace.

## Building like the packager

The `compat` settings `init` writes keep the build identical to the
packager's ([Manifest §9.14](../spec/toolchain/manifest.md#914-compat)):

- `packager-keywords` applies the packager's keyword substitution to
  passthrough Lua and XML files: `@project-version@`, `@debug@` blocks,
  `@retail@` and the other flavor blocks. `init` turns it on when your files
  use them.
- `line-endings` and `toc-version` keep the packager's line endings and TOC
  `Version`.
- The `release` profile builds unbundled and unminified. Remove those
  overrides from `profiles.release` when you want bundled, minified release
  builds ([Optimization](../spec/toolchain/optimization.md)).

You can keep `.pkgmeta` as the source of truth for dependencies and
publishing instead of converting it, with `"compat": { "pkgmeta": true }`.
The manifest then only adds wowlua's own settings; a key both files set is
E4005.

## Your CI keeps working

`init` leaves `.github/workflows` untouched, so workflows that run the
packager keep publishing. For each one, it prints how to switch: the packager
steps become `setup-wowlua` and one `wowlua publish` with the same `env`, and
runs with `-d` become `wowlua package`.

A converted project publishes the way the packager does. There is no clean
working tree check, the targets upload in the packager's order (CurseForge,
WoWInterface, Wago, GitHub), a target without a token is skipped with a note,
and tokens come from the packager's environment variables (`CF_API_KEY` or
`CF_API_TOKEN`, `WAGO_API_TOKEN`, `WOWI_API_TOKEN`, `GITHUB_OAUTH`, …; not
`GITHUB_TOKEN`) and from its `.env` file, so existing CI secrets keep working.
Without tokens, a dry run of Tally looks like this:

```sh
wowlua publish --dry-run
```

```text
    Packaged Tally-v1.0.0.zip (release) → dist/packages/Tally-v1.0.0.zip
     Skipped curseforge (Tally-v1.0.0.zip): no token (`CF_API_TOKEN`)
     Skipped wago (Tally-v1.0.0.zip): no token (`WAGO_API_TOKEN`)
warning[W4306]: embedded libraries without a recognizable license file or header: callbackhandler-1-0, aceaddon-3-0, aceevent-3-0
 = help: check their licenses before publishing; `lua-library.license-files` can name the file

warning: 1 warning emitted

    Finished publishing in 0.20s
```

See [Publishing §11.9](../spec/toolchain/publishing.md#119-the-publish-command)
and [§11.1.3](../spec/toolchain/publishing.md#1113-tokens) for the details.

## Then, file by file

The addon builds as it is. Try it in the game with `wowlua dev` (see
[Getting started](getting-started.md#run-it-in-the-game)) and with the
[dev window](debugging-in-game.md).

When you want types for a file, rename it to `.wowlua`. In a file that uses no
wowlua syntax, name and type errors are only warnings, so a renamed file
builds as before; they become errors once the file uses wowlua syntax. To
type-check the remaining `.lua` files too, with warnings only, set
`"check": { "lua": true }`.

LuaLS `---@` annotations convert into a `.d.wowlua` declaration file, from
one file or a folder:

```sh
wowlua types convert src     # writes src.d.wowlua
```
