# A tour of wowlua

wowlua is Lua 5.1, the Lua that WoW runs, plus the parts addon authors keep
writing by hand. This page shows each part in a few lines, so you know what
exists and where to read more.

## Your Lua already works

Every Lua 5.1 file is valid wowlua, so an existing addon builds as it is. In a
file that uses no wowlua syntax, the problems the checker finds are warnings,
not errors, so nothing you have today fails the build.

```lua
local addonName, ns = ...
local frame = CreateFrame("Frame")
frame:RegisterEvent("PLAYER_LOGIN")
frame:SetScript("OnEvent", function(self, event)
    print(addonName .. " loaded")
end)
```

Rename a file from `.lua` to `.wowlua` when you want more from it. You can do
that one file at a time: `.lua` and `.wowlua` files sit side by side in the
same addon. [Bring your addon](../guide/adopting.md) walks through moving an
existing addon.

## Less to type for everyday code

The things you write a hundred times have short forms: compound assignment,
`nil` fallbacks, `nil`-safe access and strings with values in them.

```wowlua title=src/gold.wowlua
local session = { earned = 0 }

local function onMoney(delta: number)
    session.earned += delta
    local guild = GetGuildInfo("player") ?? "no guild"
    print(`+{delta} copper ({guild}), {session.earned} this session`)
end

onMoney(500)
```

- `+=`, `..=` and the other compound operators update a value in place.
- `a ?? b` uses `b` only when `a` is `nil`. A `false` stays `false`.
- A backtick string puts any expression in `{…}`.
- There is also `?.` for `nil`-safe access, `continue` in loops, `const`,
  destructuring and bitwise operators. See [Syntax and sugar](syntax.md).

## Types where you want them

Add a type to a local, a parameter or a return value, and wowlua checks every
use of it. Leave types out and it infers what it can. The whole
[WoW API](../reference/api.md) has types, so calls into the game are checked
too.

```wowlua title=src/bags.wowlua
local function freeSlots(bag: number): number
    local free = C_Container.GetContainerNumFreeSlots(bag)
    return free
end

local total: number = 0
for bag = 0, 4 do
    total += freeSlots(bag)
end
print(`{total} free slots`)
```

Add `print(freeSlots("backpack"))` as line 11, and `wowlua check`, `build` or
`dev` tells you in the terminal, before the game loads the file:

```text
error[E2001]: `"backpack"` is not assignable to `number`
   ┌─ src/bags.wowlua:11:17
   │
11 │ print(freeSlots("backpack"))
   │                 ^^^^^^^^^^ here

error: aborting due to 1 previous error
```

More in [Types](types.md) and [Functions](functions.md).

## Classes instead of metatable boilerplate

A class has fields with defaults, a constructor, methods that get `self`
without declaring it, and inheritance. It compiles to the table and metatable
Lua you would write yourself.

```wowlua title=src/cooldown.wowlua
class Cooldown
    spell: string
    duration: number
    private started: number = 0

    constructor(spell: string, duration: number)
        self.spell = spell
        self.duration = duration
    end

    function start()
        self.started = GetTime()
    end

    function remaining(): number
        return math.max(0, self.started + self.duration - GetTime())
    end
end

local cd = Cooldown.new("Heroic Leap", 45)
cd:start()
```

[Classes](classes.md) covers inheritance, accessors and generic classes.

## Interfaces and enums

An interface names the shape of a table, so saved data and options are
checked. An enum names a set of values, and each use is replaced by its
value.

```wowlua title=src/loot.wowlua
enum Quality { Poor = 0, Common, Uncommon, Rare, Epic, Legendary }

interface LootEntry
    itemID: number
    quality: Quality
    looter: string
    note?: string
end

local function worthAnnouncing(entry: LootEntry): boolean
    return entry.quality >= Quality.Epic
end
```

In the Lua that comes out, `Quality.Epic` is the number `4`. See
[Interfaces and enums](interfaces.md).

## Frames as classes

A class can extend a frame type such as `Button` or `StatusBar`. Its
`super(...)` call creates the frame with `CreateFrame`, and your methods sit
next to the frame's own.

```wowlua title=src/ui/counter-button.wowlua
export class CounterButton extends Button
    count: number = 0

    constructor(parent: Frame)
        super(nil, parent, "UIPanelButtonTemplate")
        self:SetSize(120, 24)
        self:SetScript("OnClick", function() self:increment() end)
    end

    function increment()
        self.count += 1
        self:SetText(`Clicked {self.count}x`)
    end
end
```

More in [Frames as classes](widgets.md).

## Decorators for the wiring

A decorator above a function connects it to the game. The standard library
has decorators for events, slash commands, timers, combat and hooks, and you
can write your own.

```wowlua title=src/target.wowlua
import event from "wowlua:events"
import throttle from "wowlua:timer"
import slash from "wowlua:slash"

@event("PLAYER_TARGET_CHANGED")
@throttle(1)
local function targetChanged()
    print("target:", UnitName("target") or "none")
end

@slash("/gold")
local function showGold()
    print(GetCoinTextureString(GetMoney()))
end
```

`@throttle(1)` runs the first call at once, and turns the calls of the
following second into one call at its end. See [Decorators](decorators.md)
and the [standard library](../stdlib/overview.md).

## Modules instead of the addon table

Each `.wowlua` file is a module. `export` what other files may use, and
`import` it where you need it. wowlua works out the order of the files in the
TOC, so you no longer keep it by hand.

```wowlua title=src/format.wowlua
export function money(copper: number): string
    return GetCoinTextureString(copper)
end
```

```wowlua title=src/main.wowlua
import money from "./format"

print(`You have {money(GetMoney())}`)
```

The `local addonName, ns = ...` pattern keeps working, and the two mix. See
[Modules](modules.md).

## Build info

`wowlua:meta` tells your code about the addon and the build it is part of.
Its values are folded in when the file is compiled, so a branch for dev builds
is left out of release builds completely.

```wowlua title=src/banner.wowlua
import addon, build from "wowlua:meta"

if build.debug then
    print(`{addon.title} {addon.version} (dev build)`)
end
```

The dev build of this file contains:

```lua
do
	print("MyAddon 0.1.0 (dev build)")
end
```

The release build contains nothing for it. One package can build for several
flavors of the game, such as `standard` (Retail) and `camelot` (WoW: Forever),
and `flavor` from `wowlua:meta` tells the code which one it is built for. See
[Build info and profiles](flavors.md), and
[Debugging helpers](debugging.md) for profiling and assertions that are left
out of release builds.

> [!TIP]
> The playground on [wowlua.dev](https://wowlua.dev) checks and builds
> wowlua in the browser, so you can try the snippets on this page without
> installing anything.
