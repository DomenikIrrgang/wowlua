# Interfaces and enums

An interface describes the shape of a plain table, such as saved settings or
a roster entry, so that every field you read or write is checked. An enum
names a fixed set of values. Interfaces produce no code, and enum members
usually compile to plain numbers or strings.

## Describe the shape of a table

An interface lists fields and their types. A `?` after the name makes a field
optional: it may be `nil`, and wowlua makes you check it before you use it as
a value.

```wowlua title=src/roster.wowlua
export interface RaidMember
    name: string
    class: string
    role: "TANK" | "HEALER" | "DAMAGER"
    note?: string                  -- may be nil
    readonly joinedAt: number
end

export function describe(m: RaidMember): string
    local text = `{m.name} ({m.role})`
    if m.note then
        text ..= `: {m.note}`      -- a string here, not nil
    end
    return text
end
```

- `role` takes only the three strings listed. A misspelled `"HEAL"` is an
  error, and so is a field the interface doesn't declare.
- A `readonly` field is set when the table is created. Assigning it later is
  error E2010.

## Any table with the right fields fits

A table never has to say that it is a `RaidMember`. If it has the fields with
the right types, it fits: a table literal, a table from SavedVariables, or an
instance of a class. Here the values come from `GetRaidRosterInfo`, whose
return types are part of the typed [WoW API](../reference/api.md):

```wowlua title=src/sync.wowlua
import RaidMember, describe from "./roster"

local function toRole(raw: string?): "TANK" | "HEALER" | "DAMAGER"
    if raw == "TANK" or raw == "HEALER" then
        return raw
    end
    return "DAMAGER"
end

local roster: RaidMember[] = {}

for i = 1, GetNumGroupMembers() do
    local name, _, _, _, _, class, _, _, _, _, _, role = GetRaidRosterInfo(i)
    roster[#roster + 1] = {
        name = name,
        class = class,
        role = toRole(role),
        joinedAt = time(),
    }
end

for _, member in ipairs(roster) do
    print(describe(member))
end
```

`GetRaidRosterInfo` returns the role as a `string`, which could be any text,
so `toRole` turns it into one of the three values. Comparing `raw` with a
string narrows it to that string, so `return raw` fits the return type.

A table may carry more fields than the interface lists.

## Interfaces with methods

An interface can list methods as well, written like a method of a class
without a body. Then different objects can be handled the same way. Anything
that reports a number can be shown here:

```wowlua title=src/display.wowlua
export interface Readout
    title: string
    function current(): number
end

class GoldReadout implements Readout
    title: string = "Gold"

    function current(): number
        return math.floor(GetMoney() / 10000)
    end
end

class BagReadout implements Readout
    title: string = "Free slots"

    function current(): number
        local free = 0
        for bag = 0, 4 do
            free += C_Container.GetContainerNumFreeSlots(bag)
        end
        return free
    end
end

local function render(items: Readout[]): string
    local parts: string[] = {}
    for _, r in ipairs(items) do
        parts[#parts + 1] = `{r.title}: {r:current()}`
    end
    return table.concat(parts, "  ")
end

print(render({ GoldReadout.new(), BagReadout.new() }))
```

`implements` is optional: both classes fit `Readout` without it, because a
class instance fits an interface through its public fields, accessors and
methods.

> [!NOTE]
> The [specification](../../spec/language/03-classes.md#318-typing) has
> `implements` check the class where it is declared (E3006). wowlua 0.12.0
> doesn't do that check yet. A class that doesn't fit is reported where it is
> used as the interface, as in `render` above.

## Extend and combine interfaces

An interface can extend one or more other interfaces, and it can take type
parameters like a generic class.

```wowlua title=src/loot.wowlua
interface Named
    name: string
end

interface Ordered<T>
    function compare(other: T): number
end

export interface LootEntry extends Named, Ordered<LootEntry>
    itemLevel: number
    winner?: string
end

export function sortLoot(entries: LootEntry[])
    table.sort(entries, function(a, b)
        return a:compare(b) < 0
    end)
end
```

A `LootEntry` has `name`, `compare`, `itemLevel` and `winner`.

## Tables of several kinds

When a value can be one of several shapes, give each shape a field with a
fixed string and combine them in a union. Checking that field tells wowlua
which shape you have. The [typed demo addon](../../examples/typed-demo)
records money coming in and going out this way:

```wowlua title=src/money.wowlua
export interface Earned
    kind: "earned"
    time: number
    amount: number
    source: string
end

export interface Spent
    kind: "spent"
    time: number
    amount: number
end

export type Change = Earned | Spent

export function isEarned(change: Change): change is Earned
    return change.kind == "earned"
end

local history: Change[] = {
    { kind = "earned", time = time(), amount = 5000, source = "Loot" },
    { kind = "spent", time = time(), amount = 1200 },
}

for _, change in ipairs(history) do
    if isEarned(change) then
        print(`+{change.amount} from {change.source}`)   -- only Earned has source
    else
        print(`-{change.amount}`)
    end
end
```

The return type `change is Earned` makes `isEarned` a type predicate: where
the call is true, `change` is an `Earned`, and in the `else` branch a `Spent`.
Comparing `change.kind == "spent"` directly narrows in the same way. Reading
`change.source` on a `Spent` is error E2003. [Types](types.md) covers
narrowing in full.

## Type the addon table

Every file of an addon receives the same private table as the second value of
`...`. Describe what you store in it with `declare interface AddonNamespace`
in a declaration file. Declarations of `AddonNamespace` in several files are
merged.

```wowlua title=src/types.d.wowlua
declare interface Settings
    scale: number
    locked: boolean
end

declare interface AddonNamespace
    db: Settings
end
```

```lua title=src/core.lua
local addonName, ns = ...
ns.db.scale = 2          -- checked against Settings
ns.lastSync = time()     -- not declared: allowed, typed any
```

Fields you haven't declared stay allowed, so an existing addon keeps building
while you add types one field at a time. In `.wowlua` files the declared
fields are checked. `.lua` files are type-checked only when the manifest sets
`"check": { "lua": true }`, and then their problems are warnings.

## Enums

An enum names a fixed set of values, such as item qualities, display modes or
anchor points. Members without a value are numbered from 1, each one more than
the member before it. Give a member a value to start elsewhere. String enums
suit values that go into saved data or into the game's API.

```wowlua title=src/enums.wowlua
export enum BarMode { Hidden, Combat, Always }            -- 1, 2, 3

export enum Quality { Poor = 0, Common, Uncommon, Rare, Epic, Legendary }   -- 0 to 5

export enum Anchor {
    TopLeft = "TOPLEFT",
    Center = "CENTER",
}

export function shouldShow(mode: BarMode): boolean
    if mode == BarMode.Always then
        return true
    end
    return mode == BarMode.Combat and InCombatLockdown()
end

local frame = CreateFrame("Frame", nil, UIParent)
frame:SetPoint(Anchor.Center)     -- an Anchor is a string, so SetPoint takes it
```

A `BarMode` member fits wherever a `number` is expected, but a plain number is
not a `BarMode`:

```wowlua fragment
shouldShow(7)
```

```text
error[E2001]: `7` is not assignable to `BarMode`
   ┌─ src/enums.wowlua:19:12
   │
19 │ shouldShow(7)
   │            ^ here
```

- An enum is all numbers or all strings (E3010), and no two members have the
  same value (E3011).
- Enums are declared at the top level of a file (E0115).
- Members are separated by `,` or `;`, and a trailing separator is allowed.
- Assigning to a member, as in `BarMode.Hidden = 5`, is error E2010.

## What enums compile to

A member read with a constant name is replaced by its value. When the enum is
only read that way, no table is created. The palette of the
[widget demo addon](../../examples/widget-demo) works like this:

```wowlua
enum Palette {
    Accent = "33ff99",
    Muted = "9d9d9d",
}

local title = UIParent:CreateFontString(nil, "OVERLAY", "GameFontNormal")
title:SetText(`|cff{Palette.Accent}wowlua|r`)
```

```lua
local title = UIParent:CreateFontString(nil, "OVERLAY", "GameFontNormal")
title:SetText("|cff33ff99wowlua|r")
```

An enum used as a value, such as one that is exported, passed to a function
or iterated, gets a table that holds exactly its members. The `BarMode` above
compiles to:

```lua
BarMode = {Hidden = 1, Combat = 2, Always = 3}
__wl.reflectEnum(BarMode, {"Hidden", "Combat", "Always"})
```

`__wl.reflectEnum` records the declaration order for `wowlua:reflect`,
outside the table. In debug builds the table also gets a metatable that
raises an error when code reads a member that doesn't exist or assigns a new
key, instead of quietly giving `nil`. `pairs` sees the same members in every
build.

## Convert saved values to enums

A value read from SavedVariables or a chat message is a plain number or
string. `wowlua:reflect` converts it with a check, and gives you member names
for display:

```wowlua title=src/options.wowlua
import * as reflect from "wowlua:reflect"
import BarMode from "./enums"

local db = { barMode = 2 :: unknown }

local mode = reflect.enumFrom(BarMode, db.barMode) ?? BarMode.Combat
print(`bars: {reflect.enumName(BarMode, mode)}`)        -- bars: Combat

for _, entry in ipairs(reflect.enumEntries(BarMode)) do
    print(entry.name, entry.value)                      -- in declaration order
end
```

`enumFrom` returns `nil` for a value that isn't a member, so an old or
hand-edited saved value falls back to your default. `enumName` returns the
member's name, and `enumEntries` the members as `{ name, value }` records in
declaration order. [Debugging helpers](debugging.md) covers the rest of
`wowlua:reflect`.

The full rules are in [Classes, interfaces and
enums](../../spec/language/03-classes.md#33-interfaces) and
[Types](../../spec/language/02-types.md).
