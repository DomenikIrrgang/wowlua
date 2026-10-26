# Saved variables

`wowlua:saved` keeps settings and data between sessions in typed
SavedVariables, with defaults, profiles and migrations. This page shows how to
bind one, when it can be read, and how to change its shape in an update.

## Save your settings

Declare the SavedVariable in the manifest, as you would list it in a TOC file.
wowlua writes the TOC from it.

```json title=wowlua.json
{
  "$schema": "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/wowlua-0.1.json",
  "package": {
    "name": "my-addon",
    "version": "0.1.0",
    "wowlua": "^0.12"
  },
  "addon": {
    "title": "MyAddon",
    "saved-variables": ["MyAddonDB"]
  }
}
```

Describe its shape with an interface and bind it with `saved.bind`, giving a
version and the defaults. `bind` returns the data table itself, typed as the
interface.

```wowlua title=src/db.wowlua
import * as saved from "wowlua:saved"

export interface Settings
    scale: number
    showFreeSlots: boolean
    position?: { x: number, y: number }
    hidden: { [string]: boolean }
end

export local db = saved.bind("MyAddonDB", {
    version = 1,
    defaults = { scale = 1, showFreeSlots = true, hidden = {} } :: Settings,
})
```

Other files import `db` and use it. A misspelled field such as
`db.showFreeSlot`, or a string assigned to `db.scale`, is a compile error. The
name passed to `bind` must be a string literal (E8102) naming a SavedVariable
the manifest declares (E8020), and each SavedVariable can be bound once
(E8103).

## Read and write it like any table

There are no getters or setters. Reading a field gives the stored value, or
its default when nothing is stored. Assigning a field stores the value.

```wowlua title=src/window.wowlua
import onLoad from "wowlua:lifecycle"
import db from "./db"

local frame = CreateFrame("Frame", nil, UIParent)

@onLoad
local function restore()
    frame:SetScale(db.scale)                  -- 1 on a fresh install
    if db.position then
        frame:SetPoint("BOTTOMLEFT", db.position.x, db.position.y)
    end
end

export function savePosition()
    db.position = { x = frame:GetLeft(), y = frame:GetBottom() }
end

export function hideCategory(name: string)
    db.hidden[name] = true                    -- stored in the saved data
end
```

Defaults are attached lazily, nested tables included. The first read of
`db.hidden` creates a new table in the saved data, with the nested defaults
attached to it, so writes into it never change the shared defaults.

## Only what changed is written to disk

At logout, values equal to their defaults are removed before WoW writes the
file, nested tables included, key by key. The file stays small. When an update
changes a default, players who never changed that setting get the new one.

After a player sets the scale to 1.2 and hides one category, WoW writes:

```lua
MyAddonDB = {
	["scale"] = 1.2,
	["hidden"] = {
		["Junk"] = true,
	},
	["__version"] = 1,
}
```

`showFreeSlots` is missing because it still has its default. To keep every
value, bind with `stripDefaults = false`. `__version` is reserved: the module
stores the data's version under it.

## When the data is ready

WoW loads SavedVariables after the addon's files have run. You can call `bind`
at the top of a file, but the table it returns stays locked until the data has
arrived. Any read or write before that raises an error, in every build, since a
write before load would be lost:

```text
MyAddon: MyAddonDB accessed before it was loaded; use @onLoad
```

So use the table in
[`@onLoad`](events.md#run-code-at-the-right-moment-of-startup) or later. By then the module has loaded the stored data into the same table,
run the migrations and attached the defaults. If the addon needs its data while
its files load, set `"load-saved-variables-first": true` under `addon` in the
manifest, and the table is usable at the top level.

## Let players keep several profiles

Profiles let a player share one setup across characters, or keep several. Bind
with `profiles = true`, and declare a per-character SavedVariable named after
the first with `PerChar` appended. It remembers which profile each character
uses (E8021 when it is missing).

```json
"addon": {
  "title": "MyAddon",
  "saved-variables": ["MyAddonDB"],
  "saved-variables-per-character": ["MyAddonDBPerChar"]
}
```

```wowlua title=src/db.wowlua
import * as saved from "wowlua:saved"

export interface Settings
    scale: number
    showFreeSlots: boolean
    position?: { x: number, y: number }
    hidden: { [string]: boolean }
end

export local db = saved.bind("MyAddonDB", {
    version = 1,
    defaults = { scale = 1, showFreeSlots = true, hidden = {} } :: Settings,
    profiles = true,
})
```

`db` now always shows the active profile, and the same table keeps working
after a switch. Profile housekeeping lives on a separate handle,
`saved.handle(db)`, so your own fields can have any name, `reset` and
`profiles` included.

```wowlua title=src/profiles.wowlua
import * as saved from "wowlua:saved"
import print from "wowlua:log"
import db from "./db"

export function manageProfiles()
    local profiles = saved.handle(db).profiles
    if not profiles then
        return
    end
    profiles:use("Healer")                -- switch, created empty when new
    print("active:", profiles:current())  -- "Default" until the first switch
    for _, name in ipairs(profiles:list()) do
        print(" -", name)
    end
    profiles:copy("Default")              -- replace this profile's data with a copy
    profiles:delete("Old")                -- any profile but the active one

    profiles:onChange(function(name: string)
        print("now using", name, "at scale", db.scale)
    end)
end
```

`handle(db).profiles` is nil for a binding without profiles. `list()` returns
the names sorted. `copy` of a profile that doesn't exist and `delete` of the
active profile are errors. `use`, `list`, `copy`, `delete` and `reset` raise an
error before the data is loaded.

> [!TIP]
> An options page from [`wowlua:settings`](settings.md) whose `store` is `db`
> re-reads its controls when the profile changes, and a minimap button from
> `wowlua:launcher` whose `minimap.store` is `db` moves to the new profile's
> position.

## Change the shape of your data

When an update renames or restructures a field, raise `version` and add a
migration for the new version. `migrate[v]` upgrades data saved at version
`v - 1` and changes it in place.

```wowlua title=src/db.wowlua
import * as saved from "wowlua:saved"

export interface Settings
    scale: number
    showFreeSlots: boolean
    position?: { x: number, y: number }
    hidden: { [string]: boolean }
end

export local db = saved.bind("MyAddonDB", {
    version = 3,
    defaults = { scale = 1, showFreeSlots = true, hidden = {} } :: Settings,
    profiles = true,
    migrate = {
        -- 2: "uiScale" was renamed to "scale"
        [2] = function(old: any)
            for _, profile in pairs(old.profiles or {}) do
                profile.scale = profile.uiScale
                profile.uiScale = nil
            end
        end,
        -- 3: "hidden" became a table
        [3] = function(old: any)
            for _, profile in pairs(old.profiles or {}) do
                profile.hidden = profile.hidden or {}
            end
        end,
    },
})
```

- Migrations run at load, from the stored version plus one up to `version`, in
  order. Data saved at version 1 gets both of these.
- A fresh install, with nothing stored, runs none.
- Each migration gets the whole stored table once, every profile included,
  which is why the example loops over `old.profiles`.
- Migrations run on a copy. When one fails, the error is reported, the stored
  data stays as it was, nothing is stripped at logout, and using the table
  raises an error such as
  `MyAddon: MyAddonDB failed to migrate to version 3; data preserved`.
  Ship a fix, and the player's data is still there.
- Data saved by a newer version of your addon is left untouched: no migration
  runs, nothing is stripped, and a warning is reported once. A player who goes
  back to an older version loses nothing.

## Reset to defaults

Resetting is one call. With profiles it resets the active profile, without
profiles the whole data.

```wowlua fragment
saved.handle(db):reset()
```

`saved.handle(db):isLoaded()` tells whether the data has been loaded, for code
that may run before or after `ADDON_LOADED`.

## Data per character

Data that belongs to one character, such as that character's bag contents,
goes in a SavedVariable listed under `saved-variables-per-character`. Bind it
the same way. Profiles are for account-wide data, so `profiles = true` on a
per-character SavedVariable is an error (E8117).

```json title=wowlua.json
{
  "$schema": "https://raw.githubusercontent.com/DomenikIrrgang/wowlua/main/schema/wowlua-0.1.json",
  "package": {
    "name": "my-addon",
    "version": "0.1.0",
    "wowlua": "^0.12"
  },
  "addon": {
    "title": "MyAddon",
    "saved-variables": ["MyAddonDB"],
    "saved-variables-per-character": ["MyAddonDBPerChar", "MyAddonCharDB"]
  }
}
```

```wowlua title=src/inventory.wowlua
import * as saved from "wowlua:saved"

interface CharData
    gold: number
    items: { [number]: number }
end

export local charDb = saved.bind("MyAddonCharDB", {
    version = 1,
    defaults = { gold = 0, items = {} } :: CharData,
})
```

## At a glance

| Call | What it does |
|------|--------------|
| `saved.bind(name, options)` | Binds a SavedVariable: `version`, `defaults`, and optionally `migrate`, `profiles` and `stripDefaults`. Returns the data table. |
| `saved.handle(db)` | The housekeeping handle of a bound table |
| `handle:reset()` | The active profile, or the whole data, back to the defaults |
| `handle:isLoaded()` | Whether the data has been loaded |
| `handle.profiles` | `use`, `current`, `list`, `copy`, `delete` and `onChange`; nil without profiles |

The defaults' type may not contain secret values (E8104), since they can't be
compared or saved. The full rules are in the
[specification](../../spec/stdlib/core.md#saved).
