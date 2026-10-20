# 11. Publishing

`wowlua package` builds a distributable zip. `wowlua publish` builds, packages
and uploads it to every configured **publish target**: CurseForge, Wago
Addons, WoWInterface, GitHub Releases, and (for libraries) the wowlua registry.

The upload protocols below were verified on 2026-10-11 against the sites'
documentation, their live endpoints, and the BigWigsMods packager, the de-facto
standard WoW release tool. Endpoints are data in the compiler and can be
overridden per target (`api-url`) if a site changes them.

## 11.1 Configuration

```json
{
  "publish": {
    "changelog": "CHANGELOG.md",
    "changelog-format": "markdown",
    "release-type": "auto",
    "archive-maps": true,
    "targets": {
      "curseforge": { "type": "curseforge", "project-id": 123456 },
      "cf-nolib":   { "type": "curseforge", "project-id": 123457, "nolib": true },
      "wago":       { "type": "wago", "project-id": "a1b2c3d4" },
      "wowi":       { "type": "wowinterface", "project-id": 5678 },
      "github":     { "type": "github", "repository": "owner/my-addon", "minify": "max" }
    }
  }
}
```

Targets are **freely named**. The `type` field selects the site, so a project
can have several targets for one site (another project, a `nolib` variant).
All target fields, with types and defaults, are listed in
[Manifest §9.11](manifest.md#911-publish).

### 11.1.1 Target builds

- Each target builds with its profile plus its own overrides (`minify`,
  `bundle`, `flavors`, `nolib`). Addon sites (CurseForge, Wago, WoWInterface)
  build unminified unless the target sets `minify`, which W6010 warns about
  ([Optimization §15.4.4](optimization.md#1544-platform-policies)). Targets
  with identical effective settings share one build and one zip.
- Publish builds are written to `dist/targets/<target>/<Addon>/` (a shared
  build lives under the first target's name, in manifest order), so targets
  with different settings never overwrite each other or the profile builds in
  `dist/<profile>/`.
- `wowlua publish` implies `--locked`: a publish never changes `wowlua.lock`.

### 11.1.2 Release type

With `"auto"` (BigWigs packager convention):

- the build is not on a VCS tag, or the working tree is dirty: `alpha`;
- the tag contains `alpha`: `alpha`; the tag contains `beta`: `beta`;
- otherwise: `release`.

### 11.1.3 Tokens

Tokens are never stored in `wowlua.json`. They come, in order of precedence,
from the target's environment variable, from the OS credential store (filled
by `wowlua login <type>`), or, where no credential store exists, from
`config.json` ([Package manager §10.11](package-manager.md#1011-security)).
A missing token is error E4201. Before reading what `wowlua login` saved,
`wowlua publish` says so (`Reading the curseforge token …`): the OS
credential store may wait for the user to allow it (a macOS Keychain
dialog). Default variables:

| Target | Default env vars, in order |
|--------|-----------------|
| CurseForge | `CF_API_TOKEN`, `CF_API_KEY` |
| Wago | `WAGO_API_TOKEN` |
| WoWInterface | `WOWI_API_TOKEN` |
| GitHub | `GITHUB_TOKEN`, `GH_API_TOKEN`, `GITHUB_API_TOKEN`, `GITHUB_OAUTH` |
| wowlua registry | `WOWLUA_REGISTRY_TOKEN` |

Besides `GITHUB_TOKEN`, the names are the BigWigs packager's, so existing CI
secrets keep working. A converted packager project reads exactly the
packager's variables (not `GITHUB_TOKEN`), and also its `.env` file in the
repository root, whose values win over the environment like the packager's
(decision T158).

## 11.2 Packaging

`wowlua package` produces zips in `dist/packages/`, named
`<Name>-<build.version>[-<target>][-<flavor>][-nolib].zip`:

- `-<target>` is added when publish targets produce different zips,
- `-<flavor>` when a target uses `split-flavors`,
- `-nolib` for nolib packages (§11.2.2).

```
MyAddon-1.4.2.zip
└── MyAddon/              # the complete AddOn folder of the build
    ├── MyAddon.toc
    ├── MyAddon.standard.lua
    ├── …
    ├── LICENSES.txt
    └── LICENSE           # copied from the package root if present
```

- The zip's top-level folder is the AddOn folder name. With
  `publish.bundle-addons` in a workspace root, the root's targets get one zip
  with several AddOn folders (decision T160): all addon members, or the listed
  ones. The first listed member (with `true`: the first default member) is the
  main one: the zip is `<Main>-<its build.version>…zip`, and it gives the
  changelog and the release name; CurseForge relations are every bundled
  member's. The root's targets are then not shared with the members, whose own
  targets keep their own zips. `package` and `publish` at the root load the
  bundled members even when they are not default members; with `-p` or inside
  a member, there is no bundle.
- Source maps, tests, dev files and `dist/.cache` are never included.
- Zip entries use `/`, are sorted, and have timestamps normalized (to
  `SOURCE_DATE_EPOCH` or the commit time), so packages are reproducible.
- Display names on the sites use `build.version`
  ([Manifest §9.9.2](manifest.md#992-build-version)).

A converted packager project (`compat.packager-keywords`) packages like the
BigWigs packager instead (decision T157): one zip
`{package-name}-{project-version}{nolib}{classic}.zip` (or `compat.archive-name`)
holding every AddOn folder of the repository, one per released game for
`compat.per-game` projects, and with `compat.nolib-package` a nolib zip that
only the GitHub target receives.

### 11.2.1 Changelog

| `changelog` | Content |
|-------------|---------|
| A path (default `CHANGELOG.md` if it exists) | If the file has version headings (`## 1.4.2`, `## [1.4.2]`, `# v1.4.2`, …), only the section for `package.version` is used. Otherwise the whole file. |
| `"git"` | Generated from the commit subjects since the previous tag: a heading `<Name> <build.version>` followed by one list item per commit (`- <subject> (<short hash>)`), merge commits omitted |
| `null` | No changelog |

Per site, the changelog is sent as:

| Site | Format |
|------|--------|
| CurseForge | `changelogType` = `markdown`, `text` or `html`, matching `changelog-format` |
| Wago | Markdown. `text` is sent as-is; `html` is converted to Markdown. |
| WoWInterface | BBCode by default, converted from Markdown (headings → `[size]`/`[b]`, lists → `[list][*]`, links → `[url]`, code → `[code]`, emphasis → `[b]`/`[i]`), or Markdown with the target's `changelog-output: "markdown"` for projects where the site enabled it |
| GitHub | Release body as Markdown (`text` and `html` are passed through) |

CurseForge requires a changelog for every file: publishing to CurseForge with
no changelog is error E4210.

### 11.2.2 Nolib packages

A target with `"nolib": true` packages the addon **without its `lua-library`
dependencies**, for users who install those libraries as standalone addons (a
packager convention). The library files and their TOC lines are omitted, and
the libraries' standalone addon names (the dependency field `addon-name`,
default: the install folder name) are added to `## OptionalDeps`. wowlua
libraries and standard-library modules are always embedded, because they are
private copies. On GitHub, nolib zips get `"nolib": true` entries in
`release.json`.

## 11.3 Game version mapping

Every site identifies supported game versions in its own way. The compiler
maps each flavor's interface numbers (e.g. `120100` → `12.1.0`) to the site's
identifiers. When the exact version is unknown to the site, it falls back to the
highest version below it, or the highest version of that flavor, with warning
W4328 naming the site, flavor and chosen version. A flavor the site does not
know at all is omitted from that site's upload, with warning W4220. This
matches the BigWigs packager's behavior. The mapping tables (flavor ↔
site-specific names and IDs) ship with the compiler and are refreshed from the
sites' version endpoints at publish time.

## 11.4 CurseForge

| | |
|---|---|
| Upload | `POST https://wow.curseforge.com/api/projects/{projectId}/upload-file` |
| Auth | Header `X-Api-Token: <token>` |
| Body | `multipart/form-data` with fields `metadata` (JSON string) and `file` (zip) |
| Success | HTTP 200, `{"id": <fileId>}` |
| Game versions | `GET https://wow.curseforge.com/api/game/wow/versions` (authenticated). Entries are `{id, gameVersionTypeID, name, slug}`. |

Metadata sent:

```json
{
  "displayName": "1.4.2",
  "gameVersions": [12345, 12399],
  "releaseType": "release",
  "changelog": "…",
  "changelogType": "markdown",
  "relations": { "projects": [
    { "slug": "ace3", "type": "embeddedLibrary" },
    { "slug": "details", "type": "optionalDependency" }
  ] }
}
```

- `gameVersions` contains the version IDs of **all** included flavors (one file
  for all flavors). The flavor is matched by `gameVersionTypeID` (for example
  517 = Retail). The type-ID table is part of the flavor table.
- `relations` are derived automatically: embedded `dependencies` with a
  `curseforge-slug` become `embeddedLibrary` (in nolib packages:
  `requiredDependency`); `addon-dependencies.required` / `.optional` entries
  with a `curseforge-slug` become `requiredDependency` / `optionalDependency`.
  Extra relations can be listed in the target's `relations` field as
  `{ "slug": "…", "type": "tool" | "incompatible" | "embeddedLibrary" | "requiredDependency" | "optionalDependency" }`.
- `releaseType` is the release type (`alpha`, `beta`, `release`).
- CurseForge requires a changelog for every file, so publishing to CurseForge
  without one is error E4210.

## 11.5 Wago Addons

| | |
|---|---|
| Upload | `POST https://addons.wago.io/api/projects/{projectId}/version` |
| Auth | Header `Authorization: Bearer <token>`, `Accept: application/json` |
| Body | `multipart/form-data` with fields `metadata` (JSON string) and `file` (zip) |
| Success | HTTP 200 or 201 |
| Game versions | `GET https://addons.wago.io/api/data/game` (public). Contains `patches` per flavor (`retail`, `classic`, `bc`, `wotlk`, `cata`, `mop`, …). |

Metadata sent:

```json
{
  "label": "1.4.2",
  "stability": "stable",
  "changelog": "…",
  "supported_retail_patches": ["12.1.0"],
  "supported_classic_patches": ["1.15.9"]
}
```

- `stability` is `stable`, `beta` or `alpha`, mapped from the release type
  (`release` → `stable`).
- One `supported_<flavor>_patches` array per included flavor. Wago's
  documentation shows singular `supported_<flavor>_patch` strings, but the
  plural array form is the one the BigWigs packager uses and the live API
  publishes. wowlua sends the plural form.

## 11.6 WoWInterface

| | |
|---|---|
| Upload | `POST https://api.wowinterface.com/addons/update` |
| Auth | Header `x-api-token: <token>` |
| Body | `multipart/form-data`: `id`, `version`, `compatible` (comma-separated version IDs), `changelog`, `archive` (`Yes`/`No`), `updatefile` (zip) |
| Success | HTTP 202. Errors: 401 (not author), 403 (token or addon disabled), 404 (no addon), 405 (missing fields), 415 (file type), 500 (failed) |
| Game versions | `GET https://api.wowinterface.com/addons/compatible.json` (public), entries `{id, name, game, interface, default}` |

- The changelog is converted from Markdown to BBCode by default
  (`changelog-output` on the target: `"bbcode"` or `"markdown"`). WoWInterface
  accepts Markdown only for projects where the site has enabled it.
- `archive-previous` (default `true`) controls whether the previous file is
  archived.
- WoWInterface's version list does not cover every flavor. Flavors without a
  matching entry are omitted from `compatible`, with warning W4220.
- WoWInterface has no release-type field. Following packager convention, only
  builds on a VCS tag are uploaded: `alpha` builds are skipped with a note
  (set `upload-untagged: true` on the target to upload them anyway), and `beta`
  builds are uploaded with the release type visible in `version`.

## 11.7 GitHub Releases

| | |
|---|---|
| Create release | `POST https://api.github.com/repos/{owner}/{repo}/releases` (`tag_name`, `target_commitish`, `name`, `body` = changelog, `draft` from the target, `prerelease` from the target, by default true unless the release type is `release`) |
| Update release | `PATCH …/releases/{id}` when a release for the tag already exists |
| Upload asset | `POST https://uploads.github.com/repos/{owner}/{repo}/releases/{id}/assets?name={file}`. Raw body, `Content-Type: application/zip` (`application/json` for `release.json`). GitHub rejects duplicate names (HTTP 422), so an existing asset with the same name is deleted first (`DELETE …/releases/assets/{asset_id}`). |
| Auth | Header `Authorization: Bearer <token>` |

With `release-json: true` (default), a `release.json` asset is uploaded as well.
It is read by addon managers such as WowUp:

```json
{
  "releases": [
    {
      "name": "MyAddon",
      "version": "1.4.2",
      "filename": "MyAddon-1.4.2.zip",
      "nolib": false,
      "metadata": [
        { "flavor": "mainline", "interface": 120100 },
        { "flavor": "classic", "interface": 11509 }
      ]
    }
  ]
}
```

- One `metadata` entry per interface number of every included flavor. The flavor
  names are the packager's (`mainline`, `classic`, `bcc`, `wrath`, `cata`,
  `mists`, …), taken from the flavor table.
- When a release is re-run, existing `release.json` entries are merged by
  `filename`.
- With `archive-maps: true`, each zip's source maps are uploaded as
  `<zip name>-maps.zip` (for example `MyAddon-1.4.2-maps.zip`), so
  `wowlua trace` can decode reports from that version later. Converted
  packager projects upload them only with an explicit `archive-maps: true`.
- The release is named after the tag, and the token is sent as
  `Authorization: Bearer` (`token` for converted packager projects).
- GitHub requires a tag. Publishing an untagged build to GitHub creates no
  release unless `--tag` is given (warning W4329 otherwise).

## 11.8 wowlua registry

For `library`, `lua-library` and `types` packages. Uploads the package archive
described in [Package manager §10.10](package-manager.md#1010-registry). Addons
cannot be published to the registry (E4230).

## 11.9 The publish command

```
wowlua publish [--target <name>…] [--dry-run] [--tag <tag>] [--release-type <t>] [--allow-dirty] [--no-test] [--deny-warnings] [-p <package>]
```

1. Verify a clean working tree (E4336), unless `--allow-dirty` (which makes
   the release type `alpha` under `"auto"`). Resolution runs with `--locked`.
   Every target's token must be present (E4201) before anything is uploaded.
2. Build every distinct target configuration and validate the output.
3. Run `wowlua test`, unless `--no-test`.
4. Package the zips.
5. Upload to each target in parallel, with retries using exponential backoff
   for 5xx and network errors. A target that fails does not stop the others.
6. Archive the source maps under `$WOWLUA_HOME/maps/<build id>/` (and upload
   them to GitHub with `archive-maps`).
7. Print a summary with the resulting file IDs and URLs. The exit code is
   non-zero if any target failed.

**`--tag <tag>`.** If the tag does not exist, wowlua creates an annotated tag at
`HEAD` locally (it never pushes git refs). The tag must equal `package.version`
with an optional leading `v` (E4319). The build is then treated as a tagged
build, and the GitHub release is created with `tag_name = <tag>` and
`target_commitish` = the `HEAD` commit, so GitHub creates the remote tag
together with the release. If the tag exists but points at another commit, the
publish stops (E4330).

`--dry-run` performs steps 1–4, prints the exact metadata each target would
receive (tokens redacted), and validates tokens with a read-only request where
the site offers one. It does not create a `--tag` tag.

**Converted packager projects** publish like the BigWigs packager (decision
T158) instead of steps 1, 5 and 7's parallel uploads:

- no clean-tree check (the packager does not look at the working tree);
- per package (one per game for `compat.per-game`), the targets upload in the
  packager's order: CurseForge, WoWInterface, Wago, GitHub. The CurseForge
  upload is tried up to ten times; if it still fails, the remaining uploads
  are aborted;
- a target without a token is skipped with a note;
- the changelog is the packager's: the `manual-changelog` file (markup type
  from `publish.changelog-format`, default `text`) or the `CHANGELOG.md` it
  generates, read from the built package. WoWInterface gets the manual
  Markdown changelog converted with `pandoc` when it is installed (the
  packager's HTML-to-BBCode rules), else as it is, or its generated BBCode
  changelog. `.pkgmeta` `wowi-convert-changelog: no` becomes the target's
  `changelog-output: "markdown"`, `wowi-create-changelog: no` its
  `changelog: null`, `wowi-archive-previous: no` its `archive-previous: false`;
- the GitHub release gets the package, its nolib package, and `release.json`
  entries named after the project title with the tag as version;
- on GitHub Actions, a branch push whose commit is tagged publishes nothing
  (the tag's run does), like the packager.
