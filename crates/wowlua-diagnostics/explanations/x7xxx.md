# x7xxx · debugging

## E7001

`nameof` from `wowlua:debug` folds to the last name of its argument at
compile time (`nameof(Tracker.record)` is `"record"`), so the argument must be
a name, a field access or an index with a string key. A call, a literal or an
index with a computed key has no name to give. Pass the name itself.

```wowlua fails
import nameof from "wowlua:debug"

local function onLogin() end
print(nameof(onLogin()))
```

```wowlua compiles
import nameof from "wowlua:debug"

local function onLogin() end
print(nameof(onLogin))
```

## W7001

A release build (a profile with `debug = false`) still contains `todo()` from
`wowlua:debug`. `todo` is kept in every build and raises "not implemented"
when it runs, so players would hit the error. `wowlua build --release` and
`wowlua check --release` report it; the example turns `debug` off for the dev
profile so that `wowlua check` sees it.

Write the missing code before releasing.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "debug": false } }
}
```
```wowlua fails title=src/main.wowlua
import todo from "wowlua:debug"

local function exportData(): string
    return todo("export")
end
print(exportData())
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "debug": false } }
}
```
```wowlua compiles title=src/main.wowlua
local function exportData(): string
    return "data"
end
print(exportData())
```

## W7010

Code asks `wowlua:reflect` for full metadata (`reflect.fields`,
`reflect.methods`, `reflect.functionInfo`, …), but the profile sets
`reflection = "none"`, so no metadata is emitted and the call finds none at
run time. The default, `"decorated"`, keeps it for classes, enums and
functions marked `@reflect`; `"all"` keeps it for all of them. Class identity
(`reflect.isInstance`, `reflect.className`, the enum functions) is always
available and never warns.

Set the profile's `reflection` to `"decorated"` or `"all"`, or remove the
reflection call.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "reflection": "none" } }
}
```
```wowlua fails title=src/main.wowlua
import * as reflect from "wowlua:reflect"

@reflect
class Settings
    scale: number = 1
end

print(#reflect.fields(Settings))
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "profiles": { "dev": { "reflection": "decorated" } }
}
```
```wowlua compiles title=src/main.wowlua
import * as reflect from "wowlua:reflect"

@reflect
class Settings
    scale: number = 1
end

print(#reflect.fields(Settings))
```
