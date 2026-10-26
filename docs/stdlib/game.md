# Units, auras, spells and items

Read units, auras, spells and items with one typed call each, and keep
working when the client hides values in combat.

The game's API spreads this data over many functions, loads some of it
later and, on newer clients, keeps some of it secret during combat. These
modules give you one typed way to ask that is safe in combat. Each picks the
right client function for the flavors you build for; the
[WoW API reference](../reference/api.md) has the functions underneath.

## First: secret values

On clients with secret values (retail since patch 12.0, and WoW: Forever),
some answers are secret while the player is in combat.
[`UnitHealth`](../reference/api.md) still returns a value, but your addon
may not compare it, do arithmetic with it or use it as a table key. You can
still hand it to the game, to a status bar or a font string for example,
which draws it for you.

wowlua types such a value as `number | Secret<number>`, so the checker shows
you each place where a secret would break your code
([Types: secret values](../../spec/language/02-types.md#211-secret-values)).
`wowlua:secret` has small helpers for handling them:

```wowlua title=src/healthbar.wowlua
import * as secret from "wowlua:secret"

local bar = CreateFrame("StatusBar", nil, UIParent)
local highlight = bar:CreateTexture(nil, "OVERLAY")

export function update()
    local hp = UnitHealth("target")              -- number | Secret<number>
    bar:SetValue(secret.amount(hp))              -- widgets take secrets; nil becomes 0

    local shown = secret.known(hp, 0)            -- a plain number: 0 while secret
    local name = secret.known(UnitName("target"), "?")
    print(name, shown)

    if secret.flag(UnitIsUnit("target", "focus")) then   -- a secret yes/no reads as false
        highlight:Show()
    end

    if not secret.isSecret(hp) then
        print(hp + 1)                            -- fine: hp is a number here
    end
end
```

Everything else on this page works the same way: values that may be secret
pass through untouched, and a function that has to look inside a value
skips what it can't read instead of raising an error. Out of combat the same
values are ordinary numbers and strings, and the same code works.

| Function | What it does |
|----------|--------------|
| `secret.amount(v)` | For widgets: a secret passes, nil becomes 0 |
| `secret.known(v, fallback)` | The value, or the fallback while it is secret or nil |
| `secret.flag(v)` | For conditions: a secret reads as false, and old `1`/`nil` answers work |
| `secret.isSecret(v)`, `secret.readable(v)` | Test first, then use the value freely |
| `secret.isNumber(v)`, `secret.isString(v)` | Whether `v` is a number or string that isn't secret |
| `secret.asString(v)`, `asNumber(v)`, `asBoolean(v)` | Read a value typed `unknown`: the value, or nil |

On a client without secret values nothing is secret, and the helpers return
the value as it is.

## Units and your group

`wowlua:unit` answers the everyday questions about a unit in one call, and
walks your group without the `raid1` to `raid40` and `party1` to `party4`
cases.

```wowlua title=src/target.wowlua
import * as unit from "wowlua:unit"
import * as color from "wowlua:color"

local frame = CreateFrame("Frame", nil, UIParent)
local nameText = frame:CreateFontString(nil, "OVERLAY", "GameFontNormal")
local levelText = frame:CreateFontString(nil, "OVERLAY", "GameFontNormal")
local healthBar = CreateFrame("StatusBar", nil, frame)

export function updateTarget()
    local info = unit.info("target")
    if not info then
        frame:Hide()
        return
    end
    local classColor = unit.classColor("target")
    nameText:SetText(classColor and color.wrap(info.name, classColor) or info.name)
    levelText:SetText(tostring(info.level))

    local hp, maxHp = unit.health("target")
    healthBar:SetMinMaxValues(0, maxHp)
    healthBar:SetValue(hp)                       -- secrets are fine here
    frame:Show()
end
```

```wowlua title=src/group.wowlua
import * as unit from "wowlua:unit"

export function refreshAll(refresh: (token: UnitToken) -> ())
    for token in unit.group({ includePlayer = true }) do
        refresh(token)
    end
    print(`{unit.groupSize()} players`)
end
```

`unit.group` walks the raid in a raid, the party in a party, and only the
player when solo. `includePlayer` adds the player in a group, and
`includePets` puts each pet after its owner. `unit.info` returns nil when
the unit doesn't exist; its `name` has no realm, and `fullName` is
`Name-Realm`, or nil while the name is secret.

### Who is who: the roster

Combat log events give you GUIDs, but your frames want unit tokens and
names. `unit.roster` keeps the group by GUID and tells you who joins and
leaves. It starts the first time you use it, and keeps up with raid
reshuffles: a member whose token changes keeps their entry.

```wowlua title=src/meters.wowlua
import * as unit from "wowlua:unit"
import type RosterMember from "wowlua:unit"

local damage: { [string]: number } = {}

export function addDamage(sourceGuid: string, amount: number)
    local member = unit.roster.byGuid(sourceGuid)
    if member and member.fullName then
        damage[member.fullName] = (damage[member.fullName] ?? 0) + amount
    end
end

unit.roster.onChange(function(joined: RosterMember[], left: RosterMember[])
    for _, m in ipairs(joined) do
        print(m.name, "joined")
    end
    print(#left, "left")
end)
```

`byGuid` returns nil for a secret GUID. A burst of roster events rebuilds
the roster once, 0.2 seconds after the first, and a query in between
rebuilds first, so answers are always current.

| Function | Returns |
|----------|---------|
| `unit.info(unit)` | Name, realm, class, level, role, faction and more, or nil |
| `unit.health(unit)`, `unit.power(unit, type?)` | The current and maximum value |
| `unit.healthPercent(unit)` | 0 to 100, or nil when it can't be worked out |
| `unit.classColor(unit)` | The class color, from ClassColors when the player has it |
| `unit.group(opts?)`, `unit.groupSize()` | The group's unit tokens, and its size (1 when solo) |
| `unit.fullName(unit)`, `unit.exists(unit)`, `unit.isSame(a, b)` | Common checks |
| `unit.invite(name)`, `unit.inviteBnet(id)` | Invites a player, or a Battle.net friend by game account |
| `unit.roster.byGuid`, `byUnit`, `members`, `onChange` | The group by GUID |

## Buffs and debuffs

Out of combat, read auras with `each`, `forEach` or `find`. On clients with
secret auras, addons can't read auras in combat at all, in the open world
too, and these functions then find nothing. `aura.readable(unit)` tells you
whether reading works right now. `onChange` listens to `UNIT_AURA` for one
unit.

```wowlua title=src/buffs.wowlua
import * as aura from "wowlua:aura"

export function listBuffs()
    if aura.readable("player") then
        for a in aura.each("player", "HELPFUL") do
            print(a.name, a.applications)
        end
    end
end

export function hasPain(): boolean
    return aura.find("target", 589, "HARMFUL|PLAYER") ~= nil   -- your Shadow Word: Pain
end

aura.onChange("player", function(u: UnitToken, changes: UnitAuraUpdateInfo?)
    if changes == nil or changes.isFullUpdate then
        listBuffs()                              -- rescan everything
    else
        print(u, "auras changed")
    end
end)
```

Filters such as `"HARMFUL|PLAYER"` are checked when you build, so a word the
client doesn't know is a build error (E8310) instead of an empty list.
`findByName` and `byInstanceId` find one aura by its name or its instance
ID. An aura whose spell ID is secret is skipped by `find`.

### Showing auras in combat

To show auras during a fight, let the game do the matching. An aura
container is filled by the game itself; your addon only sets up the groups
and how the buttons look. Without an `initialize` function, buttons look
like the default buff icons.

```wowlua title=src/debuffs.wowlua
import * as aura from "wowlua:aura"

local targetFrame = CreateFrame("Frame", nil, UIParent)

local box = aura.container(targetFrame, {
    unit = "target",
    groups = {
        { key = "mine", filter = "HARMFUL|PLAYER", max = 8 },
        { key = "curses", filter = "HARMFUL", dispelTypes = { "Curse" } },
        { key = "tracked", filter = "HELPFUL", spells = { 1459, 21562 } },
    },
    size = 26,
    growth = { "LEFT", "DOWN" },
})

box:setGroupMax("mine", 12)
```

> [!TIP]
> The game matches the groups' spell lists and dispel types, in combat too.
> Change them later with `setGroupSpells`, `setGroupFilter` and
> `setGroupSize`. A new size applies to existing buttons when combat ends.

A client without aura containers can't build one: `aura.container` is a
build error (E8300) on such a flavor, and `aura.hasContainers()` checks at
run time. The game can't free a container, so `release()` parks it until
the next `setUnit`.

| Function | What it does |
|----------|--------------|
| `aura.readable(unit)` | Whether auras can be read right now |
| `aura.each`, `aura.forEach` | Every aura that matches a filter |
| `aura.find`, `findByName`, `byInstanceId` | One aura |
| `aura.onChange(unit, fn)` | `UNIT_AURA` for one unit |
| `aura.container(parent, opts)` | Aura icons the game fills, in combat too |

## Spells and cooldowns

```wowlua title=src/cooldowns.wowlua
import * as spell from "wowlua:spell"

local button = CreateFrame("Button", nil, UIParent)
local icon = button:CreateTexture(nil, "ARTWORK")
local label = button:CreateFontString(nil, "OVERLAY", "GameFontNormal")
local cooldown = CreateFrame("Cooldown", nil, button, "CooldownFrameTemplate")

export function show()
    local s = spell.info(133)                    -- Fireball
    if s then
        icon:SetTexture(s.iconID)
        label:SetText(s.name)
    end

    icon:SetDesaturated(spell.onCooldown(133))   -- readable in combat too

    local d = spell.cooldownDuration(133)
    if d then
        cooldown:SetCooldownFromDurationObject(d)   -- draws even while secret
    end
end
```

`spell.info` takes an ID or a name. `cooldown` and `charges` return the
full details, whose times may be secret in combat. `onCooldown` is a yes or
no you can always read, and `cooldownDuration` hands the game a duration
object it draws for you (nil on a client without them).

| Function | Returns |
|----------|---------|
| `spell.info`, `name`, `icon`, `link` | Basic data |
| `spell.description(id)` | The tooltip text, once the spell's data is loaded (see below) |
| `spell.cooldown`, `spell.charges` | Details; their times may be secret in combat |
| `spell.onCooldown`, `spell.cooldownDuration` | Safe to use in combat |
| `spell.isKnown(id, isPet?)` | Whether it is in the player's or the pet's spellbook |

## Items that load later

The game loads item data from the server when it is first asked for, so the
first [`C_Item.GetItemInfo`](../reference/api.md) for an item often returns
nothing. `wowlua:item` asks the server, waits, caches the answer and calls
you back once.

```wowlua title=src/items.wowlua
import * as item from "wowlua:item"
import type ItemData from "wowlua:item"

item.load(19019, function(data: ItemData?)
    if data then
        print(data.link, data.itemLevel)
    end
end)

item.loadMany({ 19019, 17182, 18832 }, function(all: { [number]: ItemData? })
    for id, data in pairs(all) do
        print(id, data and data.name)
    end
end)

local cached = item.get(19019)                   -- right away, or nil (and loading starts)
print(cached?.name)
```

- Several loads of the same item share one request. A callback for an item
  that is already cached runs before `load` returns.
- An item that doesn't exist, or doesn't arrive within 10 seconds, calls back
  with nil. `{ timeout = 30 }` as the third argument changes the wait.
- `load` returns a handle: `cancel()` makes sure the callback never runs.
- The cache keeps 500 items; `item.configure({ cacheSize = 1000 })` changes
  that.
- `item.loadSpell(id, fn)` loads a spell's data, so that
  `spell.description` has text.
- Inside a [background job](timers.md), `item.await(ctx, id)` waits for the
  item without a callback.

## Add lines to tooltips

Put `@onTooltip` on a function and it runs after the game fills a tooltip
of that kind. It uses the game's tooltip data system, so it covers every
tooltip, not only `GameTooltip`.

```wowlua title=src/ilvl.wowlua
import onTooltip from "wowlua:tooltip"
import type TooltipData from "wowlua:tooltip"
import * as item from "wowlua:item"

@onTooltip("Item")
local function addItemLevel(tip: GameTooltip, data: TooltipData)
    local info = data.id and item.get(data.id)
    if info and info.itemLevel then
        tip:AddLine(`item level {info.itemLevel}`, 1, 1, 1)
    end
end
```

The kind must be one of the game's tooltip data types (`"Item"`, `"Spell"`,
`"Unit"`, `"Mount"`, …), so a typo is a build error (E8330). `data` has the
`id`, `unit` and `link` of what the tooltip shows, nil while they are
secret, and `raw`, the game's own tooltip data. A failing handler never
breaks the tooltip.

The classic flavors have no tooltip data system. There only `"Item"`,
`"Spell"` and `"Unit"` work, on the game's standard tooltips (and those you
pass to `tooltip.attach`), without `raw`; the build warns about it (W8301).
Another kind is a build error for those flavors (E8300), unless the hook is
kept to the flavors that have it, with `@if(flavor == …)` or inside
`if flavor == … then`.

`tooltip.hook(kind, fn)` does the same without a decorator and returns a
handle. Hooks can't be removed, but `disable()` stops calling your function
and `enable()` starts again. These are post-hooks only: they add to a
tooltip but can't hide or replace what the game put there.
