# Types

Types catch a misspelled field, a wrong argument or a `nil` before you
`/reload`. This page covers annotations, inference, the checking modes,
narrowing, table and function types, generics, the typed WoW API and
declaration files.

## Add a type

A type goes after a colon: after a local's name, after a parameter's name, and
after the parameter list for what the function returns.

```wowlua title=src/repair.wowlua
local function repairCost(useGuild: boolean): number
    local cost: number, canRepair: boolean = GetRepairAllCost()
    if not canRepair then
        return 0
    end
    if useGuild and CanGuildBankRepair() then
        RepairAllItems(true)
    end
    return cost
end

print(repairCost(false))
```

The compiler checks the types and then removes them. The Lua it writes is the
Lua you would have written without them:

```lua
local function repairCost(useGuild)
	local cost, canRepair = GetRepairAllCost()
	if not canRepair then
		return 0
	end
	if useGuild and CanGuildBankRepair() then
		RepairAllItems(true)
	end
	return cost
end

print(repairCost(false))
```

Classes and enums are the exception: they are values, so they produce code.
Type aliases, interfaces and `::` assertions produce none
([Types §2.10](../../spec/language/02-types.md#210-type-erasure)).

## Let wowlua work it out

Most locals need no annotation. A local takes the type of the value you assign
to it, and a function without a return type returns whatever its `return`
statements give.

```wowlua title=src/session.wowlua
local kills = 0                          -- number
local zone = GetZoneText()               -- string
local stats = { gold = 0, deaths = 0 }   -- { gold: number, deaths: number }
local names = { "Thrall", "Jaina" }      -- string[]

local function perHour(amount: number, seconds: number)
    return amount / seconds * 3600       -- returns number
end

stats.gold += perHour(1200, 600)
print(kills, zone, names[1], stats.gold)
```

The inferred types are checked like written ones. Use the result of `perHour`
where a string is expected and the checker knows it's a number:

```text
error[E2001]: `number` is not assignable to `string`
   ┌─ src/session.wowlua:11:7
   │
11 │ local rate: string = perHour(stats.gold, 600)
   │       ^^^^ here

error: aborting due to 1 previous error
```

A parameter without a type is `any`, unless the function is a callback whose
parameters the call already fixes (see
[the game's API](#the-games-api-is-typed)). Hover a name in
[VS Code](../guide/vscode.md) to see the type wowlua inferred for it.

## Start loose, tighten later

Each file is checked in one of three modes:

| Mode | What it checks |
|------|----------------|
| `basic` (the default) | Annotated and inferred types. Parameters without a type are `any`, `nil` fits every type, and an undeclared global is a warning. |
| `strict` | Everything `basic` checks, plus `nil` safety (`string` never holds `nil`, write `string?`), undeclared globals are errors, parameters of functions that are not callbacks need types (E2050), and a sealed table can't gain new fields (E2051). |
| `nocheck` | No type checking. Syntax and names only. |

A file that uses no wowlua syntax, such as a plain Lua file renamed to
`.wowlua`, never fails the check. Its name and type errors are reported as
warnings with the same number: E2001 becomes W2001, E1001 becomes W1001. Only
syntax errors stay errors, because the game could not load the file either.
`.lua` files are not type-checked at all unless `check.lua` is set in the
manifest, and then only with warnings.

Set the mode for the whole addon in `wowlua.json`, and list files that are
checked in strict mode whatever the mode is:

```json
"check": {
    "mode": "basic",
    "strict": ["src/core/**"]
}
```

A directive before the first line of code sets the mode of one file:
`--!strict`, `--!basic` or `--!nocheck`.

```wowlua title=src/core/db.wowlua
--!strict
local function load(name: string): string
    return `{name}DB`
end

print(load("MyAddon"))
```

The [manifest reference](../reference/manifest.md) has the other `check` keys,
and [Types §2.1](../../spec/language/02-types.md#21-checking-modes) the exact
rules.

## Handle values that may be nil

`T?` means "a `T` or `nil`". In strict mode you must deal with the `nil` before
you use the value. An `if`, an early `return` or `assert(x)` is enough: after
it, the checker knows the value is there.

```wowlua title=src/guild.wowlua
--!strict
local function guildLabel(): string
    local guild, rank = GetGuildInfo("player")
    if not guild then
        return "no guild"
    end
    -- guild is a string from here on
    return rank and `<{guild}> {rank}` or guild:upper()
end

print(guildLabel())
```

Leave out the `if` and the error points at the spot:

```text
error[E2005]: `string?` may be `nil`
  ┌─ src/guild.wowlua:4:49
  │
4 │     return rank and `<{guild}> {rank}` or guild:upper()
  │                                                 ^^^^^ here

error: aborting due to 1 previous error
```

In basic mode `nil` fits every type, so this is not reported there. Leaving out
an argument the function needs is reported in both modes (see
[Functions](functions.md#make-a-parameter-optional)).

## Accept more than one kind of value

`A | B` is a value of either type. The checker follows your tests and narrows
the type: after `type(spell) == "number"`, `spell` is a `number`, and after the
`return`, a `string`.

```wowlua title=src/spell.wowlua
--!strict
local function spellName(spell: number | string): string
    if type(spell) == "number" then
        local info = C_Spell.GetSpellInfo(spell)
        return info and info.name or tostring(spell)
    end
    return spell                       -- a string here
end

print(spellName(774))
```

Tables with a field that holds a literal, a tag, narrow the same way. Compare
the tag and the fields of the matching type are there:

```wowlua title=src/ledger.wowlua
--!strict
type Earned = { kind: "earned", amount: number, source: string }
type Spent = { kind: "spent", amount: number, vendor: string }

local function describe(entry: Earned | Spent): string
    if entry.kind == "earned" then
        return `+{entry.amount} from {entry.source}`
    end
    return `-{entry.amount} at {entry.vendor}`
end

print(describe({ kind = "spent", amount = 120, vendor = "Arnok" }))
```

Reading `entry.vendor` before the comparison is E2003 in strict mode, because
an `Earned` has no `vendor`. The other tests that narrow are truthiness,
comparisons with `nil`, `assert`, functions that check a type
([Functions](functions.md#functions-that-check-a-type)) and tests whether a
global exists ([Types §2.5](../../spec/language/02-types.md#25-narrowing)).

## Describe tables

Most addon data lives in tables. These forms cover lists, lookups and records:

```wowlua title=src/raid.wowlua
local members: string[] = {}                       -- a list
local roleOf: { [string]: string } = {}            -- a lookup by name
local pull: { boss: string, started: number, note?: string } = {
    boss = "Ragnaros",
    started = GetTime(),
}
local frozen: { readonly id: number } = { id = 42 } -- can't be assigned

print(#members, roleOf["Thrall"], pull.boss, frozen.id)
```

- `T[]` is a list. Reading `members[i]` gives a `string`.
- `{ [K]: V }` is a map. Reading it gives `V?`, because the key may be missing.
- `{ name: T }` is a record. `note?: string` marks a field that may be absent.
- A `readonly` field can't be assigned (E2010).
- Records are structural: any table with the fields the type needs fits,
  wherever it was made. Extra fields are allowed.
- In strict mode, a field the record type doesn't have is E2003, both when
  you read it and when you write it in a table literal assigned to that type,
  so `{ bos = "Ragnaros" }` is caught.

An unannotated local that holds a table literal stays open while the function
that made it fills it in: assigning `stats.total = 0` adds a field to its type.
Reading a field it doesn't have yet gives `any`. In strict mode, reading a
field the file never stores in it, such as a misspelled `stats.deths`, is
E2003.

## Give a type a name

`type` gives a name to a type you use in several places. `typeof(expr)` is the
type of a value, so a defaults table you write once also gives you its type.

```wowlua title=src/settings.wowlua
local DEFAULTS = { scale = 1, locked = false, anchor = "CENTER" }
type Settings = typeof(DEFAULTS)     -- { scale: number, locked: boolean, anchor: string }

type ItemID = number
type OnUpdate = (frame: Frame, elapsed: number) -> ()

local function apply(frame: Frame, s: Settings)
    frame:SetScale(s.scale)
end

local function get<K extends keyof Settings>(s: Settings, key: K): Settings[K]
    return s[key]
end

local id: ItemID = 19019
local scale = get(DEFAULTS, "scale")  -- number
apply(UIParent, DEFAULTS)
print(id, scale)
```

An alias is only another name: `ItemID` and `number` are interchangeable.
`keyof Settings` is the union of the field names, `"scale" | "locked" |
"anchor"`, and `Settings[K]` is the type of field `K`. Together they type a
getter like `get` above: `get(DEFAULTS, "scael")` is E2014, and the result of
`get(DEFAULTS, "scale")` is a `number`.

Add `export` to use an alias in other files (`export type Settings = …`, see
[Modules](modules.md)). An alias may refer to itself inside a table or
function type, as in `type Tree = { value: number, children: Tree[] }`.

## Allow only certain values

A string or number literal is a type of its own, with exactly that one value.
A union of literals works like an enum of WoW's string constants. The game's
anchor points, event names and units are typed this way.

```wowlua title=src/anchor.wowlua
type Corner = "TOPLEFT" | "TOPRIGHT" | "BOTTOMLEFT" | "BOTTOMRIGHT"

local function dock(frame: Frame, corner: Corner)
    frame:ClearAllPoints()
    frame:SetPoint(corner, UIParent, corner, 0, 0)
end

local panel = CreateFrame("Frame", nil, UIParent)
dock(panel, "TOPLEFT")
```

A typo in the corner is caught at the call:

```text
error[E2001]: `"TOPLFET"` is not assignable to `"TOPLEFT" | "TOPRIGHT" | "BOTTOMLEFT" | "BOTTOMRIGHT"`
   ┌─ src/anchor.wowlua:10:13
   │
10 │ dock(panel, "TOPLFET")
   │             ^^^^^^^^^ here

error: aborting due to 1 previous error
```

When the values are your own, an [enum](interfaces.md) gives them names.

## Write code that works for any type

A type parameter in angle brackets lets one function keep the types of what you
pass it. wowlua infers it at each call. There is no syntax to write it out,
because `f<number>(x)` is already a valid comparison in Lua.

```wowlua title=src/util.wowlua
local members: string[] = { "Thrall", "Jaina" }
local roleOf: { [string]: string } = { Thrall = "TANK" }

local function first<T>(list: T[], pred: (value: T) -> boolean): T?
    for _, v in ipairs(list) do
        if pred(v) then return v end
    end
    return nil
end

local tank = first(members, function(name) return roleOf[name] == "TANK" end)  -- string?
print(tank)
```

`<T extends { size: number }>` limits what may be passed: an argument that
doesn't fit is E2014. Generic type aliases, interfaces and classes take
parameters the same way. [Functions](functions.md#generic-functions) and
[Classes](classes.md) have more.

## The game's API is typed

Every function, widget method, event and constant of the game is typed, for
each flavor you build for. Browse them in the
[WoW API reference](../reference/api.md). A callback you pass to the API gets
its parameter types from the API, so even code without a single annotation is
checked:

```wowlua title=src/fade.wowlua
local frame = CreateFrame("Frame", nil, UIParent)   -- Frame
frame:SetScript("OnUpdate", function(self, elapsed)
    -- self: Frame, elapsed: number, without writing either
    self:SetAlpha(math.max(0, self:GetAlpha() - elapsed))
end)

local bar = CreateFrame("StatusBar")                -- StatusBar
bar:SetMinMaxValues(0, UnitHealthMax("player"))
```

`CreateFrame` gives the type of the frame you ask for, so `bar` has the
`StatusBar` methods.

Testing whether a global exists narrows it. Inside
`if C_EditMode then … end`, calls into `C_EditMode` are accepted, even in a
flavor whose API doesn't have it, so the feature checks you already write keep
working. [Build info and profiles](flavors.md) covers code that differs by
flavor.

## Secret values in combat

Since Midnight, many API results are secret values to addon code in combat,
in encounters and in Mythic+. You can store a secret, pass it on, concatenate
it, and hand it to the widget setters that accept secrets. Arithmetic,
comparisons, `#`, indexing and calling it raise an error at once. wowlua types
such a result as `T | Secret<T>` and reports a forbidden use when you write it,
not when it breaks in a raid.

```wowlua title=src/healthbar.wowlua
--!strict
local bar = CreateFrame("StatusBar", nil, UIParent)
local warning = bar:CreateTexture()

local function update(unit: string)
    local hp = UnitHealth(unit)                 -- number | Secret<number>
    bar:SetValue(hp)                            -- fine: SetValue accepts secrets

    if canaccessvalue(hp) and hp < 1000 then    -- fine: a plain number here
        warning:Show()
    end
end

update("target")
```

`canaccessvalue(v)` and `issecretvalue(v)` narrow: where the value can be
read, the `Secret` part is gone. Compare `hp` without the check and it's
E2040 in strict mode:

```text
error[E2040]: `number | Secret<number>` may be a secret value; narrow it with `canaccessvalue` or `issecretvalue` first
   ┌─ src/healthbar.wowlua:12:8
   │
12 │     if hp < 1000 then warning:Show() end
   │        ^^ here
```

In basic mode and in plain Lua files it's the warning W2040. The rules are in
[Types §2.11](../../spec/language/02-types.md#211-secret-values).

## Tell wowlua what you know

`value :: Type` tells the checker the type of a value it can't see, such as a
frame a template created under a global name. It costs nothing at run time:
the compiler removes it.

```wowlua title=src/template.wowlua
local addonName = ...
type SavedSettings = { scale: number }

local button = _G["MyAddonCloseButton"] :: Button
button:SetText("Close")

local db = _G[addonName .. "DB"] :: SavedSettings
print(db.scale)
```

An assertion must make sense: one of the two types must fit the other, or
both are tables. `label :: Frame` with a string `label` is E2015. When you
really mean it, go through `any`: `label :: any :: Frame`.

`::` binds tighter than every binary operator, so write `(x :: number) + 1`
([Syntax and sugar](syntax.md)).

## Type your addon table, globals and Lua libraries

A `.d.wowlua` file holds only declarations and produces no code. Use one to
type your addon table, the globals other addons create, or a plain Lua library
you embed. Its declarations are visible in every file of the addon.

```wowlua title=src/types.d.wowlua
type Settings = { scale: number, locked: boolean }

declare interface AddonNamespace
    db: Settings
    version: string
end

declare global MyAddonDB: Settings

declare function RangeKit_GetRange(unit: UnitToken): (number?, number?)
```

`AddonNamespace` is the type of the addon table WoW passes each file as the
second value of `...`:

```wowlua title=src/core.wowlua
local addonName, ns = ...
ns.db.scale = 2          -- checked against Settings
ns.loaded = true         -- never declared: any

local minRange, maxRange = RangeKit_GetRange("target")
print(addonName, MyAddonDB.locked, minRange, maxRange)
```

All `declare interface AddonNamespace` blocks merge into one, wherever they
are. Fields you never declared stay `any`, so existing code keeps working.
`declare namespace`, `declare class`, `declare event` and `declare module`
are in [Types §2.9](../../spec/language/02-types.md#29-declaration-files).

## Classic Lua objects are understood

You don't have to switch to classes to get types. The `setmetatable` pattern
for objects is followed: the methods of the `__index` table are found on the
object, and calls to them are checked.

```wowlua title=src/account.wowlua
local Account = {}
Account.__index = Account

function Account:deposit(amount: number)
    self.balance += amount
end

function Account.new(balance: number)
    return setmetatable({ balance = balance }, Account)
end

local acc = Account.new(100)
acc:deposit(5)
```

`acc:deposit("5 gold")` is E2001, and `setmetatable(o, { __index = Account })`
works the same. In `function Account:deposit`, `self` has the type of
`Account`. The object has the methods the file defines on `Account`, also
those after `Account.new`, so in strict mode a misspelled `acc:depositt(5)` is
E2003. A [class](classes.md) checks more: missing fields and visibility, and a
misspelled method name in every mode.
