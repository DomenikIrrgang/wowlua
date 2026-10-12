# 12. CLI

All functionality is exposed through one binary, `wowlua`.

```
wowlua <command> [options]
```

## 12.1 Global options

| Option | Meaning |
|--------|---------|
| `-p, --package <name>` | Select a workspace member |
| `--manifest <path>` | Use a specific `wowlua.json` |
| `--locked` | Fail if `wowlua.lock` would change (E4104) |
| `--offline` | Use only the cache; never access the network (E4105 if something is missing) |
| `--frozen` | `--locked` plus `--offline` ([Package manager §10.6](package-manager.md#106-lockfile)) |
| `--features <a,b>` / `--no-default-features` | Select features ([Manifest §9.6](manifest.md#96-features)) |
| `--message-format human\|short\|json\|github` | Diagnostic output format. `json` emits one JSON object per line, for editors and tools. `github` emits GitHub Actions annotations. |
| `--color auto\|always\|never` | Terminal colors |
| `-q, --quiet` / `-v, --verbose` | Output verbosity. Tokens are always redacted. |
| `--deny-warnings` | Treat warnings as errors |

## 12.2 Project commands

| Command | Description |
|---------|-------------|
| `wowlua new <path> [--kind addon\|library\|lua-library\|types] [--name <Name>] [--flavors <f,…>] [--ci github]` | Create a new package from a template (§12.9) |
| `wowlua init [--from-toc <file>] [--from-pkgmeta]` | Create `wowlua.json` in an existing directory (§12.6) |
| `wowlua check [--flavor <f>…] [--watch]` | Parse and type-check without emitting |
| `wowlua build [--profile <p> \| --release] [--flavor <f>…] [--watch] [--out <dir>] [--stats] [--reproducible]` | Build the AddOn into `dist/<profile>/<Name>/` |
| `wowlua dev [--flavor <f>…] [--client <folder>…] [--link] [--wow <install>] [--force]` | Watch, build with the `dev` profile, and sync the AddOn into WoW (§12.5) |
| `wowlua install [--profile <p> \| --release] [--flavor <f>…] [--client <folder>…] [--wow <install>] [--force]` | Build once (default profile `dev`, decision T45) and install into WoW (§12.5) |
| `wowlua uninstall [--client <folder>…] [--wow <install>]` | Remove installed builds of this addon (§12.5) |
| `wowlua test [filter] [--flavor <f>] [--watch] [--coverage] [--reporter <r>]` | Run tests ([Tooling §16.1](tooling.md#161-test-runner-wowlua-test)) |
| `wowlua fmt [--check] [paths…]` | Format sources |
| `wowlua lint [--fix]` | Run lints only. `--fix` applies machine-applicable fixes. |
| `wowlua fix [--manifest]` | Apply all machine-applicable fixes from diagnostics. `--manifest` migrates `wowlua.json` to the current schema ([Manifest §9.16](manifest.md#916-schema-versions-and-migration)). |
| `wowlua trace [file] [--clipboard] [--build <id>]` | Decode error stacks ([Tooling §16.4](tooling.md#164-trace-decoder-wowlua-trace)) |
| `wowlua clean` | Remove `dist/` (archived source maps in `$WOWLUA_HOME/maps/` are kept) |
| `wowlua explain <code>` | Explain a diagnostic code, with examples (works offline) |
| `wowlua lsp` | Start the language server on stdio |
| `wowlua toc [--flavor <f>]` | Print the TOC(s) that would be generated |

## 12.3 Dependency commands

| Command | Description |
|---------|-------------|
| `wowlua add <name>[@<req>] [--git <url> [--tag\|--branch\|--rev <x>] [--subdir <d>]] [--svn <url> [--rev <n>]] [--path <p>] [--archive <url> --sha256 <h> [--strip-prefix <p>]] [--kind <k>] [--files <a,b>] [--install-path <p>] [--dev] [--optional] [--flavors <f,…>] [--features <a,b>]` | Add (or replace) a dependency and update the lockfile. If the dependency cannot be resolved, `wowlua.json` is left unchanged |
| `wowlua add --addon <Name> [--optional]` | Add an `addon-dependencies` entry |
| `wowlua remove <name> [--dev]` | Remove a dependency |
| `wowlua update [<name>…] [--precise <pin>]` | Move locked pins within their declarations: branches and default branches to their newest commit, SVN sources without `rev` to their newest revision. Tags and explicit revisions are pinned by the manifest. `--precise` pins one dependency to a given git commit or SVN revision |
| `wowlua fetch` | Download all dependencies into the cache |
| `wowlua tree [--flavor <f>] [--invert <name>]` | Show the dependency graph |
| `wowlua outdated` | List dependencies with newer tags (by SemVer), newer commits on their branch, or newer SVN revisions, and what to change |
| `wowlua cache clean [--all]` | Remove cache entries not used by any fetch or build for 30 days; `--all` empties the cache (decision T109) |
| `wowlua types sync [--flavor <f>…]` | Refresh the flavor table, the pinned interface numbers and the locked `wow-api-<flavor>` types packages after a game patch ([Package manager §10.9](package-manager.md#109-types-packages)) |
| `wowlua types convert <path> [--out <dir>]` | Convert LuaLS `---@` annotations into `.d.wowlua` declarations |

`add`, `remove` and `update` rewrite `wowlua.json` **in place**: key order,
indentation and line endings are preserved, and only the affected entries change.

## 12.4 Publishing commands

| Command | Description |
|---------|-------------|
| `wowlua package [--target <t>] [--flavor <f>…] [--registry]` | Build and package zips into `dist/packages/`. `--registry` creates the registry archive of a library package instead ([Package manager §10.10.3](package-manager.md#10103-package-archive)). |
| `wowlua publish [--target <t>…] [--dry-run] [--tag <tag>] [--release-type <t>] [--allow-dirty] [--no-test]` | Package and upload ([Publishing §11.9](publishing.md#119-the-publish-command)) |
| `wowlua login <type>` | Store an API token for a site type in the OS credential store ([Package manager §10.11](package-manager.md#1011-security)) |

## 12.5 WoW installation integration

`wowlua dev`, `install` and `uninstall` need the WoW install directory. It is
determined from, in order: the `--wow` option, the `WOWLUA_WOW_PATH`
environment variable, the global config, and auto-detection of default install
locations (Battle.net default paths on Windows and macOS).

**Clients.** Each flavor maps to client folders inside the installation (e.g.
`standard` → `_retail_`, `_ptr_`, `_xptr_`), from the flavor table
([Manifest §9.4](manifest.md#94-flavors)). By default the AddOn is installed
into **every existing client folder of every built flavor**:
`<install>/<client folder>/Interface/AddOns/<Name>/`. `--flavor` restricts the
flavors, and `--client` restricts the client folders. Every client receives the
same AddOn folder: in split flavor mode it contains all flavors' files, and the
TOC lets each client load its own. A built flavor without any existing client
folder is reported with a note.

| Mode | Behavior |
|------|----------|
| copy (default) | Replace the AddOn folder after each successful build ([Compiler §13.12](compiler.md#1312-platform-considerations)). Files the build did not produce are removed, so the folder always matches the build exactly. |
| `--link` | Create a symlink (macOS, Linux) or directory junction (Windows) to the build folder once. Later builds update it in place. |

**Safety.** `dev` and `install` only replace an existing AddOn folder whose TOC
carries an `X-Wowlua-Build` field (i.e. was installed by wowlua). Any other
existing folder, such as the same addon installed by an addon manager, is
left untouched and reported as error E4331, unless `--force` is given.
`wowlua uninstall` removes folders (or links) of this addon that carry
`X-Wowlua-Build`, in the selected clients.

A failed build never touches the installed AddOn, so the last working version
stays in place. WoW only reloads Lua on `/reload`. `wowlua dev` prints a
reminder after each sync, and in dev builds the dev console's `/wl reload` is
available.

## 12.6 Adopting an existing addon

`wowlua init` in a folder containing an existing addon:

1. Reads the `.toc` file(s) the client finds in the folder: `<Folder>.toc` and
   `<Folder><suffix>.toc`, compared case-insensitively, so the addon name is
   the folder name (`LittleWigs_Classic.toc` in `LittleWigs_Classic/` is the
   plain TOC, not a `_Classic` variant). Suffixes are the current ones and the
   legacy `-Classic`, `-BCC`, `_BCC`, `-WOTLKC`, `_WOTLKC`, `_TBC`, `_Vanilla`
   and `_Mainline` forms. It takes the metadata (`Title`, `Notes`, `Author`,
   `Version`, `IconTexture`, `Category`, …), interface versions (giving the
   flavors), SavedVariables, dependencies and `X-` fields into `wowlua.json`.
   Addon-wide `## AllowLoadGameType:` and `## ExcludeLoadGameType:` lines
   narrow the flavors. Boolean directives count as set when their value is `1`
   or `true`, as in the client (decision T62).
2. **Moves** every Lua and XML file the TOCs load (directly or through XML
   includes) into `src/`, keeping relative paths (`git mv` inside a Git
   repository), so the project gets the standard layout (decision T42). If the
   addon already has a `src` folder of its own, the sources go to
   `wowlua-src/` and `build.src` is set accordingly, so nothing is nested in
   the old folder (decision T60). Paths keep their on-disk spelling, even if a
   TOC spells them differently. Files that AddOns next to this one load
   through `..\<this>\…` lines move too
   ([Manifest §9.7.2](manifest.md#972-files-shared-between-addons)). All
   references are updated: relative `<Script>`/`<Include>` paths stay valid
   because files move together, and absolute in-game paths to moved files
   (`Interface\AddOns\<Name>\<path>`) in Lua and XML files are rewritten to the
   new output location (`…\<Name>\src\<path>`, also when the sources are in
   `wowlua-src/`). Other files (textures, sounds,
   fonts) stay where they are and are added to `build.assets`, so their in-game
   paths do not change.
3. Files loaded by only some flavors' TOCs, or by lines with
   `[AllowLoadGameType …]`, are recorded in `build.flavor-files`. Inline
   directives may stand anywhere in a line, may be written with a colon
   (`[AllowLoadGameType: tbc]`), and several may follow each other. Lines
   loading files of another AddOn (`..\Other\file.lua`) go to
   `build.external-files`. File lines with variables (`[Game]`, `[Family]`,
   `[TextLocale]`) go to `build.variable-files`, and all their expansions are
   moved ([Manifest §9.7.1](manifest.md#971-file-variables)).
4. Records the existing file order in `build.load-order`. Each flavor's order
   comes from the TOC its client reads (decision T53). The orders are merged
   topologically, so files keep their relative order in every flavor. When
   the flavors conflict (A before B in one flavor, B before A in another), the
   merged order follows the first flavor, and each flavor whose TOC order
   differs keeps its own in `build.flavor-load-order` (decision T126). The
   compiler keeps these orders
   wherever load-order analysis finds no reason to change it
   ([Modules §5.4](../language/05-modules.md#54-load-order)), so the build
   loads files as before.
5. Takes per-flavor metadata from the flavors' TOCs: the value most flavors use
   becomes the base and the others become `addon.flavor-overrides`. Structural
   fields cannot vary per flavor (§9.3.1). `Version`, the load flags and the
   publish IDs take the value used by the most flavors among the TOCs that set
   them. `SavedVariables` and
   `SavedVariablesPerCharacter` take the union of all TOCs, so no flavor loses
   saved data (decision T57). TOCs with more than 200 `X-` directives get
   `addon.toc-data` files ([Manifest §9.3.2](manifest.md#932-toc-data-files)).
   Addons that load no code at all (data or archive addons) are valid. Lua and
   XML files that no TOC loads are listed in a warning and left in place.
6. With `--from-pkgmeta` (or automatically if `.pkgmeta` / `pkgmeta.yaml` exists):
   - `externals` become `dependencies`: git, SVN and zip-archive externals are
     converted (decision T51); hg externals are listed with warning W4333 and
     must be replaced by a git mirror or archive. The common parent folder of the
     externals' paths (e.g. `Libs`) becomes `build.libs-dir`, and each
     remaining path becomes the dependency's `install-path`, so the addon's
     existing XML (`embeds.xml`) keeps working. URLs are interpreted like the
     packager does (decision T116): old CurseForge URLs, SVN `trunk` and
     `tags/<tag>` paths (and `tag:` on a trunk URL), sub-paths of CurseForge
     and GitHub git URLs (`subdir`), `.zip` archives (their `sha256` is
     computed by downloading them once), and git otherwise, with
     `"submodules": true` like the packager. Each dependency is named after
     its folder (`Libs/AceAddon-3.0` → `aceaddon-3-0`), takes the TOC lines
     into its folder (and the files the addon's own XML loads from it) as its
     `files`, and gets a `flavors` restriction when only some flavors load
     it. The TOC lines keep their place in `build.load-order` under their
     output path (decision T106). An external that nothing loads is not
     converted (warning); `tag: latest` follows the default branch instead
     (warning).
   - `ignore` becomes `build.exclude`, `plain-copy` becomes `build.assets`,
     `package-as` becomes `addon.name`, `manual-changelog` becomes
     `publish.changelog`.
   - `move-folders` entries that move whole addon folders become workspace
     members, adopted the same way; the externals and `ignore` entries inside
     such a folder become the member's. A repository whose addons all come from
     `move-folders` (WeakAuras, DBM, ElvUI) becomes a workspace root without a
     package. Entries that make an external (or a folder of one) an AddOn of
     its own become `workspace.external-addons` (decision T127); entries that
     move a library's inner folder up become the external's `subdir`. Other `move-folders` uses are
     reported with warning W4332.
   - TOCs are read as the packager packages them: `#@do-not-package@`,
     `#@debug@` and `#@alpha@` blocks are dropped and `#@non-debug@` /
     `#@non-alpha@` blocks uncommented.
   - Network operations (clones, fetches, SVN, downloads) are retried up to
     three times, like the packager does.
   - `enable-nolib-creation: yes` adds a GitHub target with `"nolib": true`.
   - The relation lists (`required-dependencies`, `optional-dependencies`,
     `embedded-libraries`, `tools-used`) become CurseForge relations.
   - Passthrough files that use packager keywords turn on
     `compat.packager-keywords`. A TOC `Version` of `@project-version@` takes
     the version of the latest git tag.
7. Reads `## X-Curse-Project-ID`, `## X-Wago-ID` and `## X-WoWI-ID` into publish
   targets.
8. Writes `wowlua.lock` pinning the interface numbers the TOCs list, so the
   first build generates the same `## Interface` lines; `wowlua types sync`
   moves them on (decision T115).
9. Leaves the contents of all `.lua` files untouched except for the path
   rewrites of step 2. They build as-is, and can be renamed to `.wowlua` one at a
   time.

As an alternative to converting, a project may keep its `.pkgmeta` as the source
of truth for dependencies and publishing with `"compat": { "pkgmeta": true }`.
The manifest then only adds wowlua-specific settings, and conflicting keys are
error E4005. Externals become dependencies as above (named after their folders);
a manifest entry of the same name may add `files`, `flavors`, `optional`,
`sha256` (zip externals) and the other non-source keys, while source keys
(`git`, `svn`, `tag`, `install-path`, …), `build.libs-dir`, `build.exclude`,
`addon.name` and `publish.changelog` that `.pkgmeta` also sets are E4005
(decision T117).

**Packager keywords.** With `"compat": { "packager-keywords": true }`, BigWigs
packager keyword substitution is applied to passthrough `.lua` and `.xml`
files, as a text preprocessing step before parsing, per build configuration and
flavor ([Compiler §13.3.1](compiler.md#1331-stages), stage 5). TOC files are
generated, so TOC keywords do not apply.

| Keyword | wowlua value |
|---------|--------------|
| `@project-version@` | `build.version` |
| `@project-hash@`, `@project-abbreviated-hash@`, `@project-revision@` | VCS commit hash, short hash, commit count |
| `@project-author@`, `@project-date-iso@`, `@project-date-integer@`, `@project-timestamp@` | From the `HEAD` commit |
| `@file-*@` variants | From the last commit touching the file |
| `@build-date@`, `@build-date-iso@`, `@build-date-integer@`, `@build-timestamp@` | Build time (`SOURCE_DATE_EPOCH` when reproducible) |

| Block | Active when |
|-------|-------------|
| `@debug@ … @end-debug@` | The profile's `debug` is true |
| `@alpha@ … @end-alpha@` | `build.version` is an alpha or dev version (untagged build) |
| `@do-not-package@ … @end-do-not-package@` | Removed (lines deleted, like the packager) when the profile's `debug` is false, kept otherwise (decision T120) |
| `@retail@`, `@version-retail@` | Flavor `standard` |
| `@version-classic@`, `@version-vanilla@` | Flavor `vanilla` |
| `@version-bcc@` | Flavor `tbc` |
| `@version-wrath@`, `@version-titan@` | Flavor `wrath` |
| `@version-cata@` | Flavor `cata` |
| `@version-mists@` | Flavor `mists` |
| `@version-forever@` | Flavor `camelot` |

The `non-` prefixed forms invert a block. Inactive blocks are commented out in
the packager's style, so line numbers stay correct; `@do-not-package@` blocks
are deleted like the packager deletes them. Embedded `lua-library` files get the processing the packager gives its
externals: repository keywords from the library's locked commit or revision,
`@debug@` and `@do-not-package@` blocks as above, and XML
`<!--@no-lib-strip@-->` blocks commented out (decision T120). Keywords are applied before the files are checked. Flavor blocks
need `build.flavor-mode: "split"` when several flavors are built, and then
produce per-flavor variants of the file (decision T118). `@localization(...)@` is not supported (W4030). Use
`wowlua:locale` instead.

## 12.7 Configuration

Global configuration lives in `$WOWLUA_HOME/config.json` (default `~/.wowlua/config.json`):

```json
{
  "wow": { "path": "/Applications/World of Warcraft" },
  "registries": { "default": "https://registry.wowlua.dev" },
  "dev": { "link": false }
}
```

API tokens live in the OS credential store (`wowlua login`). Only when no
credential store is available does `wowlua login` write a `tokens` object into
this file ([Package manager §10.11](package-manager.md#1011-security)).

Environment variables override config values: `WOWLUA_HOME`,
`WOWLUA_WOW_PATH`, the token variables in [Publishing §11.1.3](publishing.md#1113-tokens),
and `NO_COLOR`.

## 12.8 Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | User errors (compile errors, failed tests, failed uploads) |
| 2 | Invalid command-line usage |
| 101 | Internal compiler error. The message asks for a bug report with the build id. |

## 12.9 Templates (`wowlua new`)

| Kind | Files |
|------|-------|
| `addon` | `wowlua.json`, `src/main.wowlua`, `tests/main.test.wowlua`, `locales/enUS.json`, `CHANGELOG.md`, `README.md`, `.gitignore` (`dist/`; `wowlua.lock` is committed) |
| `library` | `wowlua.json` (`kind: "library"`), `src/init.wowlua`, `tests/init.test.wowlua`, `README.md`, `.gitignore` (`dist/`, `wowlua.lock`) |
| `lua-library` | `wowlua.json` (`kind: "lua-library"` with `lua-library.files`), `<Name>.lua`, `README.md`, `.gitignore` |
| `types` | `wowlua.json` (`kind: "types"`), `types/index.d.wowlua`, `README.md`, `.gitignore` |

With `--ci github`, the template also contains the workflow of §12.10. A Git
repository is initialized when `git` is available and the target is not
already inside one.

## 12.10 Continuous integration

A typical GitHub Actions setup checks every push and publishes on tags:

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
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }          # tags and history for build.version and changelogs
      - uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main   # installs the release binary
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

- `fetch-depth: 0` is needed for tag-based `build.version` and `"git"`
  changelogs.
- Caching `~/.wowlua/cache` keyed by the lockfile hash avoids refetching
  dependencies and types packages.
- `setup-wowlua` (a composite action in the wowlua repository) downloads the
  prebuilt binary from the project's GitHub Releases (decision T41). Other ways
  to install: the install scripts attached to every release
  (`install.sh` for macOS and Linux, `install.ps1` for Windows), and later
  Homebrew (macOS), winget and Scoop (Windows).
