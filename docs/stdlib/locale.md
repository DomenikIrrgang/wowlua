# Localization

`wowlua:locale` reads your translations from one JSON file per language and
gives your code a typed `L`. A misspelled key is a build error, and players
load only their own language.

## Write your strings once

`wowlua new` creates a `locales/` folder with an empty `enUS.json`. The base
locale's file, English by default, defines which keys exist. Every other
language translates as many of them as it has.

```text
locales/
├── enUS.json      the base locale: defines the keys
├── deDE.json
└── frFR.json
```

```json
{
  "TITLE": "Kill Tracker",
  "WELCOME": "Welcome back, {name}!",
  "KILLS": "{count} kills in {zone}"
}
```

That is `enUS.json`. `deDE.json` translates two of its keys:

```json
{
  "WELCOME": "Willkommen zurück, {name}!",
  "KILLS": "{count} Siege in {zone}"
}
```

- Each file is a JSON object of strings (E8115 otherwise).
- Keys are upper case letters, digits and underscores (E8060).
- Values may contain the game's escape sequences, such as colors
  (`|cffff8800…|r`) and textures (`|T…|t`), and newlines.
- A file's name is a locale code and `.json` (E8105): `enUS`, `enGB`, `deDE`,
  `esES`, `esMX`, `frFR`, `itIT`, `koKR`, `ptBR`, `ruRU`, `zhCN` or `zhTW`.

German players see `TITLE` in English, since `deDE.json` doesn't translate it.

## Use them in code

Import `L` and read the keys as fields. A key without placeholders is a
string. A key with placeholders is a function that takes their values by name.

```wowlua fragment
import L from "wowlua:locale"
import print from "wowlua:log"

local title = UIParent:CreateFontString(nil, "OVERLAY", "GameFontNormal")
title:SetText(L.TITLE)                                    -- a string
print(L.WELCOME({ name = UnitName("player") }))           -- "Welcome back, Thrall!"
print(L.KILLS({ count = 12, zone = GetZoneText() }))
```

The type of `L` is generated from the base locale, so the editor completes the
keys, and a misspelled one stops the build:

```text
error[E2003]: `WowluaLocale` has no field or method `TITEL`
  ┌─ src/main.wowlua:5:17
  │
5 │ title:SetText(L.TITEL)                                    -- a string
  │                 ^^^^^ here

error: aborting due to 1 previous error
```

Calling a plain string, or passing something other than a table to a key with
placeholders, is a type error too.

## Placeholders

A placeholder is a name in braces: `{name}`. A translation may move its
placeholders and repeat them, since languages order words differently, but it
must use the same names as the base string (E8063). Write `{{` and `}}` for
literal braces.

Values are turned into text with `tostring`. Format numbers before you pass
them, for example with `format.number` from [`wowlua:format`](utils.md), or
with `("%.1f"):format(x)`.

```wowlua fragment
import L from "wowlua:locale"
import * as format from "wowlua:format"
import print from "wowlua:log"

export function report(kills: number)
    print(L.KILLS({ count = format.number(kills), zone = GetZoneText() }))
end
```

When a value may be a secret, the result is typed `string | Secret<string>`,
since text built from a secret is secret too.

## Missing and stray translations

Translations rarely keep up with every release, and that is fine.

- A key a language doesn't translate falls back to the base string. The build
  warns once per language, with the keys it lacks, so you know what to send
  your translators:

  ```text
  warning[W8062]: `deDE` lacks 1 of 3 translations (`TITLE`); the base strings are used
   = in locales/deDE.json
  ```

- A key that only a translation has, usually one you renamed in the base
  locale, is an error (E8061), so old strings don't linger.
- A placeholder name that differs from the base string's is an error (E8063).
- A missing base-locale file is an error (E8106).

> [!NOTE]
> British English clients report `enGB`. They load your `enUS` strings, then
> `enGB.json` if you have one, so it only needs the keys that differ, such as
> "Colour" for "Color".

## What the player loads

Each language compiles to a small Lua file of its own, listed before all your
other files, base locale first, so `L` is complete when your code runs. With
the default settings the game loads the base locale and the player's own
language only. When every client you build for supports it, the TOC file
tells the client which file belongs to which language. Otherwise every file is
listed, and each one checks `GetLocale()` first and stops when it isn't the
player's language.

The defaults suit most addons. These keys of the `build` section of
[`wowlua.json`](../reference/manifest.md) change them:

| Key | Default | Meaning |
|-----|---------|---------|
| `locales-dir` | `"locales"` | The folder of the JSON files |
| `locale-base` | `"enUS"` | The locale that defines the keys and provides the fallbacks |
| `locales` | `"client"` | `"client"` loads the player's language only; `"all"` puts every language in one file |

```json
"build": {
  "locales-dir": "Locales",
  "locale-base": "deDE",
  "locales": "all"
}
```

> [!TIP]
> Bringing an addon that has a `locales/` folder of Lua files? wowlua reads only
> the `*.json` files in it, and only when your code imports `wowlua:locale`, so
> your Lua files keep working while you move the strings over.

## At a glance

| Write | Get |
|-------|-----|
| `import L from "wowlua:locale"` | the typed string table |
| `L.KEY` | a string, for a key without placeholders |
| `L.KEY({ name = … })` | a string, for a key with placeholders |
| `locales/<code>.json` | one language: `enUS`, `enGB`, `deDE`, `esES`, `esMX`, `frFR`, `itIT`, `koKR`, `ptBR`, `ruRU`, `zhCN`, `zhTW` |

The full rules are in the [specification](../../spec/stdlib/ui.md#locale).
