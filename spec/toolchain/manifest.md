# 9. Manifest (`wowlua.json`)

Every package has a `wowlua.json` at its root. It is **strict JSON**
(RFC 8259, no comments, no trailing commas). The compiler publishes a JSON
Schema, and the manifest should reference it through `$schema` so editors
offer completion and validation:

```json
{ "$schema": "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/wowlua-0.1.json" }
```

`wowlua add`, `remove` and `update` edit the file in place, preserving key order
and indentation. Unknown keys are error E4002, so typos are caught. Keys are
`kebab-case`. Schema versions and migration are covered in §9.16.

## 9.1 Complete example

```json
{
  "$schema": "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/wowlua-0.1.json",
  "package": {
    "name": "my-addon",
    "version": "1.4.2",
    "kind": "addon",
    "description": "Tracks things.",
    "authors": ["Jane Doe <jane@example.com>"],
    "license": "MIT",
    "repository": "https://github.com/jane/my-addon",
    "wowlua": "^0.1"
  },
  "addon": {
    "name": "MyAddon",
    "title": "|cff33ff99My|r Addon",
    "title-localized": { "deDE": "Mein Addon" },
    "notes": "Tracks things.",
    "category": "Combat",
    "icon-texture": "Interface\\Icons\\INV_Misc_QuestionMark",
    "saved-variables": ["MyAddonDB"],
    "saved-variables-per-character": ["MyAddonCharDB"],
    "toc-extra": { "X-Discord": "https://discord.gg/example" },
    "meta": { "discord": "https://discord.gg/example" }
  },
  "flavors": {
    "standard": {},
    "mists": {},
    "vanilla": { "interface": [11509] }
  },
  "dependencies": {
    "libstub": { "git": "https://github.com/example/LibStub", "tag": "1.0.3", "kind": "lua-library", "curseforge-slug": "libstub" },
    "wowlua-ui": { "version": "^0.4", "optional": true }
  },
  "addon-dependencies": {
    "optional": [{ "name": "Details", "curseforge-slug": "details" }]
  },
  "features": {
    "default": ["minimap"],
    "minimap": [],
    "experimental-ui": ["dep:wowlua-ui"]
  },
  "build": {
    "src": "src",
    "assets": ["media/**"],
    "exclude": ["src/**/*.scratch.wowlua"]
  },
  "check": {
    "mode": "basic"
  },
  "config": {
    "logChannel": "SYSTEM",
    "maxHistory": 50
  },
  "profiles": {
    "dev": { "config": { "maxHistory": 500 } }
  },
  "publish": {
    "changelog": "CHANGELOG.md",
    "targets": {
      "curseforge": { "type": "curseforge", "project-id": 123456 },
      "wago": { "type": "wago", "project-id": "a1b2c3d4" },
      "wowinterface": { "type": "wowinterface", "project-id": 5678 },
      "github": { "type": "github", "repository": "jane/my-addon" }
    }
  }
}
```

Top-level keys: `$schema`, `package`, `addon`, `lua-library`, `types`,
`flavors`, `dependencies`, `dev-dependencies`, `addon-dependencies`,
`features`, `build`, `check`, `config`, `profiles`, `minify-levels`, `publish`,
`meta`, `fmt`, `compat`, `workspace`, `stdlib`.

## 9.2 `package`

| Key | Type | Required | Meaning |
|-----|------|----------|---------|
| `name` | string | yes | Package ID: lowercase `[a-z0-9-]`, 2–64 characters |
| `version` | string | yes | SemVer 2.0 |
| `kind` | `"addon"` `"library"` `"lua-library"` `"types"` | no (default `"addon"`) | See [Package manager §10.1](package-manager.md#101-packages) |
| `description`, `license`, `repository`, `homepage` | string | no | `license` is an SPDX expression |
| `authors` | string[] | no | `"Name <email>"` or `"Name"` |
| `keywords` | string[] | no | Registry search |
| `wowlua` | semver req | no | Required compiler version. Checked **before** schema validation; unmet is error E4300 (§9.16). |
| `main` | path | libraries only | Main module (default `src/init.wowlua`) |

The `addon` section is only valid for `kind: "addon"` (E4317). Library-kind
packages may use these sections instead:

| Section | Kind | Keys |
|---------|------|------|
| `lua-library` | `lua-library` | `files`: ordered list of entry files (`.lua`, `.xml`, or a `.toc` whose file list is used), relative to the package root. Required unless the package root contains exactly one `.toc`. `license-files`: extra files to copy as licenses (default: `LICENSE*`, `COPYING*`). |
| `types` | `types` | `declarations`: globs of `.d.wowlua` files (default `["**/*.d.wowlua"]`). |

## 9.3 `addon`

Only for `kind: "addon"`. Every key maps to a TOC directive and/or `wowlua:meta`.

| Key | Type | Default | TOC directive | Notes |
|-----|------|---------|---------------|-------|
| `name` | string | PascalCase of `package.name` | file and folder name | Must match `[A-Za-z0-9_-]+`. |
| `title` | string | `name` | `Title` | May contain color codes. |
| `title-localized` | `{locale: string}` | `{}` | `Title-<locale>` | Emitted after `Title`, as the client requires |
| `notes`, `notes-localized` | string / map | none | `Notes`, `Notes-<locale>` | |
| `author` | string | `package.authors`, names only (`<email>` parts removed), joined by `", "` | `Author` | |
| `category`, `category-localized` | string / map | none | `Category`, `Category-<locale>` | 11.1.0+ clients |
| `group` | string | none | `Group` | Name of the parent addon (suites) |
| `icon-texture` / `icon-atlas` | string | none | `IconTexture` / `IconAtlas` | |
| `load-on-demand` | bool | `false` | `LoadOnDemand` | `false` together with a non-empty `addon-dependencies.load-with` is error E4312 (load-with implies load on demand). |
| `default-state` | `"enabled"` `"disabled"` | `"enabled"` | `DefaultState` | |
| `saved-variables` | string[] | `[]` | `SavedVariables` | Also declared as globals for the checker |
| `saved-variables-per-character` | string[] | `[]` | `SavedVariablesPerCharacter` | |
| `load-saved-variables-first` | bool | `false` | `LoadSavedVariablesFirst` | |
| `allow-table-access` | bool | `false` | `AllowAddOnTableAccess` | Lets other addons read this addon's table through `C_AddOns.GetAddOnLocalTable` (and so `__wowlua.meta` and module exports) |
| `only-beta-and-ptr` | bool | `false` | `OnlyBetaAndPTR` | |
| `expose-meta` | string | none | (none) | Also store `wowlua:meta` runtime tables in this global (an explicit `@global`-style write, see [Code generation §14.1.1](codegen.md#1411-global-writes)) |
| `toc-extra` | `{string: string}` | `{}` | verbatim | Extra directives. Keys must start with `X-` unless they are known directives (W4001). |
| `meta` | `{string: string\|number\|bool}` | `{}` | (none) | Custom values in `addon.meta` of `wowlua:meta` |
| `flavor-overrides` | `{flavor: Override}` | `{}` | per-flavor directives | Metadata that differs per flavor (§9.3.1) |

The TOC `## Version:` is always `build.version` (§9.9.2) and is not
configurable. The Addon Compartment entry is registered at runtime by
`wowlua:launcher` and needs no manifest key and no TOC directive.

### 9.3.1 Per-flavor metadata

`addon.flavor-overrides` maps a flavor to the metadata fields that differ for
it (decision T50). A field that is absent keeps the base value. `null` removes
the field for that flavor.

```json
"addon": {
  "title": "|cff1784d1ElvUI|r",
  "icon-texture": "Interface\\AddOns\\ElvUI\\Media\\LogoAddon",
  "group": "ElvUI",
  "flavor-overrides": {
    "mists": { "title": "|cff1784d1ElvUI|r Mists" },
    "wrath": { "title": "|cff1784d1ElvUI|r Wrath", "icon-texture": null, "group": null },
    "standard": { "toc-extra": { "AddonCompartmentFunc": "ElvUI_AddonCompartmentFunc" } }
  }
}
```

- Overridable fields: `title`, `title-localized`, `notes`, `notes-localized`,
  `author`, `category`, `category-localized`, `group`, `icon-texture`,
  `icon-atlas` and `toc-extra`. Localized maps and `toc-extra` override per
  key. Structural fields (saved variables, load behavior) cannot vary per
  flavor, because the game data they describe must not depend on the client.
- Keys must be flavors listed in `flavors` (E4002 otherwise).
- In the TOC, flavors that share a value share one directive. A value used by
  only some flavors gets an inline `[AllowLoadGameType …]` condition in the
  single layout. That needs inline directives on `##` lines (12.0.7+ clients),
  so `toc-layout: "auto"` falls back to per-flavor TOCs when a targeted client
  lacks them ([Code generation §14.9](codegen.md#149-toc-generation)).
- `wowlua init` writes overrides automatically when the adopted addon's TOCs
  differ per flavor: the value most flavors use becomes the base.

## 9.4 `flavors`

The flavors the package supports: an object whose keys are flavor IDs or
aliases (§9.4.1), normalized to IDs.

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `interface` | number[] | the interface numbers pinned in `wowlua.lock` for that flavor | Interface numbers to emit |
| `client-folders` | string[] | from the flavor table | Install folders `wowlua dev` / `install` use |
| `types` | semver req | latest compatible | Version requirement for the flavor's `wow-api-<flavor>` package |

When `flavors` is absent, an `addon` targets `standard` only, and `library`,
`lua-library` and `types` packages support **all** flavors.

### 9.4.1 Flavor table

Flavor IDs are **Blizzard's game types**, as used by TOC load directives. They
name the ruleset, not the client. The table is data shipped with the compiler
(crate `wowlua-flavors`). `wowlua types sync` updates it, and the version in use
is pinned in `wowlua.lock` ([Package manager §10.6](package-manager.md#106-lockfile)),
so every machine builds identical TOCs. The values below are as of 2026-10-11.

| Flavor ID | Aliases | Family | Current client(s) | Interface | `WOW_PROJECT_ID` | Client folder(s) | Game suffix (family suffix) |
|-----------|---------|--------|-------------------|-----------|------------------|------------------|-----------------------------|
| `standard` | `retail`, `mainline` | Mainline | Retail (Midnight) 12.1.0 | 120100 | 1 | `_retail_`, `_ptr_`, `_xptr_`, `_beta_` | `_Standard`, 12.1.5+ only (`_Mainline`) |
| `camelot` | `forever` | Mainline | WoW: Forever (launch 2026-11-04) | 16001 (beta) | 18 | `_classic_beta_` (beta). Live folder pending. | `_Camelot` / `_Forever` (`_Mainline`) |
| `mists` | | Classic | MoP Classic 5.5.4 | 50504 | 19 | `_classic_`, `_classic_ptr_` | `_Mists` (`_Classic`) |
| `cata` | | Classic | Cataclysm Classic 4.4.2 (frozen) | 40402 | 14 | — | `_Cata` (`_Classic`) |
| `wrath` | `titan` | Classic | Titan Reforged 3.80.2 (China) | 38002 | 11 | pending | `_Wrath` (`_Classic`) |
| `tbc` | `anniversary` | Classic | TBC Anniversary 2.5.6 | 20506 | 5 | `_anniversary_`, `_classic_era_ptr_` | `_TBC` (`_Classic`) |
| `vanilla` | `classic-era` | Classic | Classic Era 1.15.9 | 11509 | 2 | `_classic_era_` | `_Vanilla` (`_Classic`) |

**Aliases.** Aliases that name a ruleset (`retail`, `mainline`,
`classic-era`) are permanent. Aliases that name a client (`forever`, `titan`,
`anniversary`) resolve to the ruleset that client runs **in the pinned flavor
table**. When `wowlua types sync` moves such an alias to another ruleset (for
example when Anniversary realms progress), it reports warning W4313 and the
manifest keeps working with the new meaning only after the lockfile update is
accepted.

The table also records, for each client: supported TOC features
(comma-separated interfaces, inline load directives on file and directive
lines, `[AllowLoadTextLocale]`, file variables, recognized game-type names and
suffixes), the deferred/immediate API markers used by load-order analysis
([Compiler §13.6.1](compiler.md#1361-load-order-analysis)), and the
site-specific identifiers used when publishing
([Publishing §11.3](publishing.md#113-game-version-mapping)). A flavor without
a types package or pinned interface is error E4318.

## 9.5 Dependencies

`dependencies` and `dev-dependencies` are maps from name to a requirement
string or a source object. See
[Package manager §10.3](package-manager.md#103-sources) for sources and their
fields. Additional fields on every dependency:

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `curseforge-slug` | string | none | Slug for CurseForge relations |
| `features` | string[] | `[]` | Features of the dependency to enable |
| `default-features` | bool | `true` | Enable the dependency's `default` features |
| `install-path` | path | the dependency key | Folder under `build.libs-dir` the library is placed in (e.g. `AceAddon-3.0` for migrated projects whose XML expects that path) |
| `addon-name` | string | the install folder name | Standalone addon name of a `lua-library`, used for `OptionalDeps` in nolib packages ([Publishing §11.2.2](publishing.md#1122-nolib-packages)) |
| `config` | object | `{}` | Overrides of a wowlua library's `config` defaults ([Package manager §10.5.1](package-manager.md#1051-wowlua-libraries)) |

The standard library (including `wowlua:mock` for tests) ships with the
compiler and is never declared as a dependency.

`addon-dependencies`:

| Key | TOC directive | Entries |
|-----|---------------|---------|
| `required` | `Dependencies` | AddOn name strings, or `{ name, curseforge-slug?, flavors? }` |
| `optional` | `OptionalDeps` | same |
| `load-with` | `LoadWith` | AddOn names. Makes the addon load on demand (see E4312). |
| `load-managers` | `LoadManagers` | AddOn names |

Entries with `flavors` are emitted only for those flavors: in a single TOC with
an inline `[AllowLoadGameType …]` directive, otherwise only in those flavors'
TOCs.

## 9.6 Features

```json
"features": {
  "default": ["minimap"],
  "minimap": [],
  "comms": ["dep:libdeflate"],
  "full": ["minimap", "comms"]
}
```

- Each feature maps to a list of entries: another feature's name, or
  `dep:<name>`, which enables the **optional** dependency `<name>`.
- `default` lists the features enabled unless `--no-default-features` is given.
  `--features a,b` enables more.
- An entry naming an unknown feature, an unknown dependency, or a non-optional
  dependency is error E4314.
- Features appear in `wowlua:meta` as `features.<name>: boolean`, folded at compile
  time, and can be used in `@if` and static conditions.
- Feature names are `kebab-case` in the manifest and `camelCase` in code
  (`experimental-ui` → `features.experimentalUi`).
- Dependency resolution always covers the union of all features
  ([Package manager §10.4](package-manager.md#104-versions-and-resolution)), so
  enabling features never changes the lockfile.

## 9.7 `build`

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `src` | path | `"src"` | Source root |
| `out` | path | `"dist"` | Output root (workspace members use the workspace's, §9.15) |
| `assets` | glob[] | `[]` | Files copied verbatim, at their path relative to the package root. Must not collide with reserved output names ([Code generation §14.8](codegen.md#148-output-layout)). |
| `exclude` | glob[] | `[]` | Excluded from sources and assets |
| `load-order` | (path\|glob)[] | `[]` | Explicit order, written by `wowlua init`: files without load-time dependencies between them keep this order, and it breaks ties in the computed order ([Modules §5.4](../language/05-modules.md#54-load-order)) |
| `keep` | string[] | `[]` | Module IDs never removed by tree-shaking (for plain-Lua consumers, [Optimization §15.1](optimization.md#151-always-on-optimizations)) |
| `libs-dir` | path | `"libs"` | Output folder for dependencies. Migrated projects may use e.g. `"Libs"`. |
| `flavor-files` | `{glob: flavor[]}` | `{}` | Restricts matching source files (Lua, `.wowlua`, XML) to the listed flavors. Files not matched belong to every flavor. A file matched by several globs belongs to the union of their flavors. A file whose flavors are all untargeted is left out of the build. Mainly for plain Lua files, which cannot use `@if`; written by `wowlua init` from flavor-specific TOCs (decision T43). |
| `flavor-mode` | `"split"` `"shared"` | `"split"` | [Build metadata §6.3](../language/06-meta.md#63-flavors) |
| `toc-layout` | `"auto"` `"single"` `"per-flavor"` | `"auto"` | [Code generation §14.9](codegen.md#149-toc-generation) |
| `locales` | `"client"` `"all"` | `"client"` | Load only the player's language, or all ([Standard library: locale](../stdlib/ui.md#locale)) |
| `locales-dir` | path | `"locales"` | Directory of locale JSON files |
| `locale-base` | locale code | `"enUS"` | Locale that defines the keys and provides fallbacks |

## 9.8 `check`

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `mode` | `"nocheck"` `"basic"` `"strict"` | `"basic"` | Project check mode |
| `strict` | glob[] | `[]` | Files checked in strict mode regardless of `mode` |
| `lua` | bool | `false` | Also type-check `.lua` files (warnings only) |
| `globals` | string[] | `[]` | Extra allowed globals (typed `any`) |
| `lints` | `{code: "allow"\|"warn"\|"deny"}` | `{}` | Severity overrides, e.g. `{ "W1003": "allow" }` |

## 9.9 `profiles`

Built-in profiles: `dev`, `release`, `test`. Settings given here are merged
over the built-in defaults. A custom profile must name the profile it builds
on with `inherits` (E4315 if missing).

```json
"profiles": {
  "release": { "minify": "compact" },
  "beta": { "inherits": "release", "log-level": "info", "config": { "maxHistory": 100 } }
}
```

| Key | Type | Defaults and chapter |
|-----|------|----------------------|
| `inherits` | profile name | Required for custom profiles. Cycles are E4007. |
| `debug`, `assertions` | bool | [Debugging](../language/07-debugging.md) |
| `log-level` | `"trace"` … `"off"` | [Debugging](../language/07-debugging.md) |
| `instrument` | `"none"` `"decorated"` `"exports"` `"all"` | [Debugging](../language/07-debugging.md) |
| `runtime-checks` | `"none"` `"exports"` `"all"` | [Debugging](../language/07-debugging.md) |
| `profile-errors` | bool, default `false` | Count errors in profiled functions ([Debugging §7.3](../language/07-debugging.md#73-profiling)) |
| `reflection` | `"none"` `"decorated"` `"all"` | [Debugging](../language/07-debugging.md) |
| `dev-console`, `source-maps` | bool | [Debugging](../language/07-debugging.md) |
| `bundle` | bool | `false` (dev, test), `true` (release). [Optimization](optimization.md) |
| `minify` | level name | `"none"` (dev, test), `"strip"` (release) |
| `minify-libraries` | bool | `false`. Apply the minification level to lua-library dependencies too. |
| `optimize` | object | `{ "inline": bool, "cache-globals": "off"\|"safe", "dead-code": bool }`, defaults in [Optimization §15.2](optimization.md#152-profile-controlled-optimizations) |
| `line-mode` | `"preserve"` `"statement"` `"pretty"` | `"preserve"` for unbundled, `"statement"` for bundled output ([Code generation §14.5](codegen.md#145-line-preservation)) |
| `config` | object | Overrides of the top-level `config` (§9.9.1) |

**Merging.** The effective profile is built from the built-in defaults, then
each profile in the `inherits` chain (base first), then the profile itself,
then publish-target overrides. Objects are merged deeply, arrays and scalars
replace.

### 9.9.1 `config`

The top-level `config` object defines every user configuration key with its
default value. Profiles override values in their own `config`. The result is
exposed as `config` in `wowlua:meta`, with a generated type taken from the
top-level object. A profile `config` key missing from the top-level object, or
with an incompatible type, is error E4012.

### 9.9.2 Build version

`build.version` (in `wowlua:meta` and the TOC's `## Version:`) is derived from
`package.version`:

| Build | `build.version` |
|-------|-----------------|
| `release` profile (or a publish target) on a VCS tag | `X.Y.Z`. The tag, with an optional leading `v`, must equal `package.version`, otherwise error E4319. |
| `release` profile (or a publish target) not on a tag, or with uncommitted changes | `X.Y.Z-alpha+<short hash>` |
| `dev` and `test` profiles (and custom profiles inheriting `dev`) | `X.Y.Z-dev+<short hash>` |

Without version control, `<short hash>` is omitted (`X.Y.Z-dev`).

## 9.10 `minify-levels`

User-defined minification levels: `{ name: { extends?: level, passes: pass[] } }`.
See [Optimization §15.4](optimization.md#154-minification-levels). Unknown pass
names are E6003.

## 9.11 `publish`

Semantics in [Publishing](publishing.md).

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `changelog` | path \| `"git"` \| `null` | `"CHANGELOG.md"` if that file exists, else `null` | Changelog source ([Publishing §11.2.1](publishing.md#1121-changelog)) |
| `changelog-format` | `"markdown"` `"text"` `"html"` | `"markdown"` | Format of the changelog source |
| `release-type` | `"auto"` `"release"` `"beta"` `"alpha"` | `"auto"` | |
| `archive-maps` | bool | `true` | Archive source maps of published builds |
| `bundle-addons` | bool \| string[] | `false` | Workspaces: one zip with all (or the listed) member addons |
| `targets` | `{name: Target}` | `{}` | Publish targets, freely named |

Target objects:

| Field | Applies to | Type | Default |
|-------|------------|------|---------|
| `type` | all | `"curseforge"` `"wago"` `"wowinterface"` `"github"` `"registry"` | required |
| `enabled` | all | bool | `true` |
| `profile` | all | profile name | `"release"` |
| `minify`, `bundle` | all | level name / bool | from the profile |
| `flavors` | all | flavor[] | all manifest flavors |
| `split-flavors` | all | bool | `false` |
| `nolib` | all | bool | `false`. Package without embedded `lua-library` dependencies ([Publishing §11.2.2](publishing.md#1122-nolib-packages)). |
| `release-type` | all | as above | `publish.release-type` |
| `changelog`, `changelog-format` | all | as above | `publish.*` |
| `changelog-output` | `wowinterface` | `"bbcode"` `"markdown"` | `"bbcode"`. Format sent to WoWInterface; Markdown only for projects where the site enabled it. |
| `token-env` | all | string | per type ([Publishing §11.1.3](publishing.md#1113-tokens)) |
| `api-url` | all | URL | built in |
| `project-id` | curseforge, wowinterface | number | required |
| `project-id` | wago | string (8 characters) | required |
| `relations` | curseforge | `{ slug, type }[]` | derived ([Publishing §11.4](publishing.md#114-curseforge)) |
| `archive-previous` | wowinterface | bool | `true` |
| `upload-untagged` | wowinterface | bool | `false` |
| `repository` | github | `"owner/repo"` | required |
| `release-json` | github | bool | `true` |
| `draft` | github | bool | `false` |
| `prerelease` | github | `"auto"` \| bool | `"auto"` (true unless the release type is `release`) |
| `registry` | registry | URL | default registry |

## 9.12 `meta`

| Key | Default | Meaning |
|-----|---------|---------|
| `runtime` | `"used"` | `"used"` emits only the meta fields used as runtime values. `"full"` emits all of them. |

## 9.13 `fmt`

| Key | Type | Default |
|-----|------|---------|
| `indent` | `"tab"` \| 2 \| 4 | `"tab"` |
| `line-width` | number | `100` |
| `quote-style` | `"double"` `"single"` `"preserve"` | `"double"` |
| `lua` | bool | `false` (also format `.lua` files) |

See [Tooling §16.3](tooling.md#163-formatter-wowlua-fmt).

## 9.14 `compat`

| Key | Default | Meaning |
|-----|---------|---------|
| `pkgmeta` | `false` | Read dependencies and publishing settings from `.pkgmeta` ([CLI §12.6](cli.md#126-adopting-an-existing-addon)) |
| `packager-keywords` | `false` | Apply BigWigs packager keyword substitution to passthrough files |

## 9.15 `workspace`

Only in a workspace root manifest, which may omit `package`:

| Key | Meaning |
|-----|---------|
| `members` | Globs of member package directories |
| `default-members` | Members used when no `-p` is given |
| `exclude` | Globs excluded from `members` |
| `build.out` | The single output root for all members (default `"dist"`). A member setting its own `build.out` is error E4316. |
| `profiles`, `minify-levels`, `check`, `fmt`, `config`, `publish` | Shared defaults. A member's values are merged over them (objects deeply, arrays and scalars replace). Publish targets merge by target name; a member can turn a shared target off with `"enabled": false`. |

Within a workspace, a dependency whose name matches a member library resolves
to that member automatically, without a `path` entry.

## 9.16 Schema versions and migration

- The `$schema` URL names the manifest schema version
  (`wowlua-<major>.<minor>.json`). Schema files live in the repository's
  `schema/` directory and are referenced through their GitHub raw URL
  (decision T46). Editors use it. The compiler does not need
  it.
- `package.wowlua` (the required compiler version) is read **first**, before
  any other validation. If the running compiler does not satisfy it, the build
  stops with E4300, which names both versions, instead of reporting unknown keys
  of a newer schema as E4002.
- When the schema changes incompatibly, the compiler accepts the previous major
  schema with warnings for one release cycle, and `wowlua fix --manifest`
  rewrites the manifest in place (keeping formatting) to the current schema.
- `wowlua.lock` has its own `version` field. The compiler reads lockfiles of
  older versions and rewrites them in the current format on the next resolve.
  A lockfile from a newer compiler is error E4320.

## 9.17 `stdlib`

Settings for standard-library modules. Each key is only read when the module is
used.

| Key | Type | Default | Meaning |
|-----|------|---------|---------|
| `log.history-size` | integer | `500` | Size of the `wowlua:log` ring buffer ([Standard library: log](../stdlib/advanced.md#log)) |

```json
"stdlib": { "log": { "history-size": 1000 } }
```
