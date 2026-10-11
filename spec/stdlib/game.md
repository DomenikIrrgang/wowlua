# 8.4 Standard Library: Game Data

Modules for querying game data that work the same across flavors. Each
module selects the best available client API **at compile time** in `split`
flavor mode, and with a runtime check in `shared` mode. Availability comes
from the flavor table. When a client lacks an API and the module has no
fallback, using the affected function for that flavor is error E8300. When a
fallback with reduced functionality is used, warning W8301 lists the missing
fields.

All modules respect secret values ([Types §2.11](../language/02-types.md#211-secret-values)):
values that the client may return as secrets are typed `T | Secret<T>` and are
passed through unchanged. The modules never compare, index or do arithmetic on
them internally. Where a function needs such an operation (e.g. matching a
spell ID), it skips values it cannot access and says so in its contract.

## unit

`wowlua:unit` provides unit information helpers.

```wowlua
import * as unit from "wowlua:unit"
import * as color from "wowlua:color"

local info = unit.info("target")                 -- UnitInfo?
if info then
    label:SetText(color.wrap(info.name, unit.classColor("target")))
end

local hp, maxHp = unit.health("target")           -- number | Secret<number> each
bar:SetMinMaxValues(0, maxHp)
bar:SetValue(hp)                                  -- widget setters accept secrets

for token in unit.group({ includePlayer = true }) do
    refreshRow(token)
end
```

| API | Signature | Notes |
|-----|-----------|-------|
| `info` | `(unit: UnitToken) -> UnitInfo?` | `nil` if the unit does not exist. Fields: `name`, `realm?`, `fullName`, `guid`, `class`, `classFile`, `level`, `isPlayer`, `isConnected`, `faction?`, `race?`, `role?`. Fields the client may return as secrets are typed `T \| Secret<T>`. |
| `exists` | `(unit: UnitToken) -> boolean` | |
| `fullName` | `(unit: UnitToken) -> string?` | `"Name-Realm"`. The local realm is filled in (normalized like `Ambiguate`'s inverse). `nil` if the name is not accessible. |
| `isSame` | `(a: UnitToken, b: UnitToken) -> boolean` | `UnitIsUnit` |
| `health` | `(unit: UnitToken) -> (number \| Secret<number>, number \| Secret<number>)` | Current and maximum |
| `healthPercent` | `(unit: UnitToken) -> (number \| Secret<number>)?` | Uses the client's native percent API where the flavor table declares one, which may return a secret. Otherwise computed from `health` only when both values are accessible, else `nil`. |
| `power` | `(unit: UnitToken, powerType?: number) -> (number \| Secret<number>, number \| Secret<number>)` | |
| `classColor` | `(unit: UnitToken) -> ColorMixin?` | From `wowlua:color` (honors `CUSTOM_CLASS_COLORS`) |
| `group` | `(opts?: { includePlayer: boolean?, includePets: boolean? }) -> (() -> UnitToken?)` | Iterates `raid1..raidN` in a raid, `party1..party4` (plus `player` when requested) in a party, and only `player` when solo |
| `groupSize` | `() -> number` | `GetNumGroupMembers()`, with 1 when solo |

## aura

`wowlua:aura` iterates and queries auras.

```wowlua
import * as aura from "wowlua:aura"

for a in aura.each("player", "HELPFUL") do
    print(a.name, a.applications)
end

local debuff = aura.find("target", 589, "HARMFUL|PLAYER")    -- AuraData?

local sub = aura.onChange("player", function(u: UnitToken, changes: AuraChanges?)
    if changes == nil or changes.isFullUpdate then rescan() else applyDelta(changes) end
end)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `each` | `(unit: UnitToken, filter: AuraFilter) -> (() -> AuraData?)` | Yields every aura, including ones with secret fields |
| `forEach` | `(unit: UnitToken, filter: AuraFilter, fn: (a: AuraData) -> boolean?)` | Allocation-free variant. Returning `true` stops iteration. |
| `find` | `(unit: UnitToken, spellId: number, filter: AuraFilter) -> AuraData?` | Auras whose `spellId` is not accessible (secret) are **skipped** |
| `findByName` | `(unit: UnitToken, name: string, filter: AuraFilter) -> AuraData?` | Same skipping rule |
| `byInstanceId` | `(unit: UnitToken, id: number) -> AuraData?` | Modern clients only. E8300 on flavors without aura instance IDs. |
| `onChange` | `(unit: UnitToken, fn: (unit: UnitToken, changes: AuraChanges?) -> ()) -> Subscription` | `UNIT_AURA` filtered to `unit`, using a unit-filtered event frame (see [Core: events](core.md#events)). `changes` is the client's update info where available, otherwise `nil` (meaning: full rescan needed). |

- **`AuraData`** is the type declared by the flavor's `wow-api` package for
  `C_UnitAuras` aura data. On clients without `C_UnitAuras`, the module builds
  `AuraData` tables from the legacy `UnitAura` returns. Fields the legacy API
  lacks are `nil`, and W8301 lists them.
- **`AuraFilter`** is a string of `|`-separated tokens (`HELPFUL`, `HARMFUL`,
  `PLAYER`, `RAID`, `CANCELABLE`, `NOT_CANCELABLE`, `INCLUDE_NAME_PLATE_ONLY`,
  plus flavor-specific tokens from the flavor table). Literal filters are
  validated at compile time (E8310).
- Iteration stops at the first missing index, matching the client APIs.

## spell

`wowlua:spell` provides spell information across `C_Spell` and the legacy
global functions.

```wowlua
import * as spell from "wowlua:spell"

local s = spell.info(133)            -- SpellInfo?
if s then icon:SetTexture(s.iconID) end
local cd = spell.cooldown(133)       -- SpellCooldownInfo?
```

| API | Signature | Notes |
|-----|-----------|-------|
| `info` | `(spell: number \| string) -> SpellInfo?` | `name`, `iconID`, `originalIconID`, `castTime`, `minRange`, `maxRange`, `spellID`. Uses `C_Spell.GetSpellInfo` where available, otherwise wraps `GetSpellInfo`'s multiple returns into the same record. |
| `name`, `icon` | `(spell) -> string?`, `(spell) -> number?` | |
| `link` | `(spell) -> string?` | |
| `description` | `(spell: number) -> string?` | May be empty until the spell data is loaded. Use `wowlua:item`'s `loadSpell` first. |
| `cooldown` | `(spell) -> SpellCooldownInfo?` | `startTime`, `duration`, `isEnabled`, `modRate`. Fields may be secret on restricted flavors. The legacy fallback wraps `GetSpellCooldown`. |
| `cooldownDuration` | `(spell) -> DurationObject?` | Only on clients with duration objects (E8300 elsewhere). Use for secret-safe display with `Cooldown:SetCooldownFromDurationObject`. |
| `charges` | `(spell) -> SpellChargeInfo?` | |
| `isKnown` | `(spellId: number, isPet?: boolean) -> boolean` | |

## item

`wowlua:item` loads and caches item **and spell** data asynchronously.
WoW loads item and spell data from the server lazily. The first query for an
uncached item returns `nil`, and the data arrives later.

```wowlua
import * as item from "wowlua:item"

item.load(19019, function(info: ItemInfo?)
    if info then print(info.link, info.itemLevel) end
end)

local cached = item.get(19019)                   -- ItemInfo? (nil and starts loading if uncached)

item.loadMany({ 19019, 17182 }, function(infos: { [number]: ItemInfo? }) … end)

item.loadSpell(133, function(ok: boolean) print(spell.description(133)) end)

-- inside a wowlua:jobs job
local info = item.await(ctx, 19019)              -- yields until loaded
```

| API | Signature | Notes |
|-----|-----------|-------|
| `get` | `(item: number \| string) -> ItemInfo?` | Returns immediately. On a cache miss, it starts loading and returns `nil`. |
| `load` | `(item: number \| string, fn: (info: ItemInfo?) -> (), opts?: LoadOptions) -> LoadHandle` | `fn` runs once the data is available. It receives `nil` if the item does not exist or loading timed out. If the item is already cached, `fn` runs synchronously before `load` returns. |
| `loadMany` | `(items: (number \| string)[], fn: (infos: { [number]: ItemInfo? }) -> (), opts?) -> LoadHandle` | One callback after all items have resolved |
| `loadSpell` | `(spellId: number, fn: (ok: boolean) -> (), opts?) -> LoadHandle` | Ensures spell data (name, description) is loaded |
| `await` | `(ctx: JobContext, item: number \| string) -> ItemInfo?` | Yields the job until loaded ([Infrastructure: jobs](infrastructure.md#jobs)) |
| `LoadOptions` | `{ timeout: number? }` | Default timeout: 10 seconds |
| `LoadHandle` | `Cancel()`-style handle: `cancel()`, `isDone(): boolean` | A cancelled load never calls its callback |
| `ItemInfo` | record | `id`, `name`, `link`, `quality`, `itemLevel`, `minLevel`, `type`, `subType`, `stackCount`, `equipLoc`, `icon`, `sellPrice`, `classID`, `subclassID`, `bindType`, `expansionID` |

Implementation: the item and spell mixins (`Item:CreateFromItemID(id):ContinueOnItemLoad`,
`Spell:CreateFromSpellID(id):ContinueOnSpellLoad`) where the flavor table
declares them. Otherwise `GET_ITEM_INFO_RECEIVED` plus the client's
item-data request function. Concurrent loads of the same item share one
request. Loaded `ItemInfo` records are cached per addon. The cache holds at most
`item.configure({ cacheSize = 500 })` entries, with least-recently-used eviction.
Callbacks run through the stdlib error policy ([Infrastructure: errors](infrastructure.md#errors)).

## tooltip

`wowlua:tooltip` provides typed post-hooks for tooltips. On clients with the
tooltip data system it uses `TooltipDataProcessor.AddTooltipPostCall`, and
elsewhere it hooks the `OnTooltipSet*` scripts of the standard tooltips.

```wowlua
import onTooltip from "wowlua:tooltip"
import * as tooltip from "wowlua:tooltip"

@onTooltip("Item")
local function addItemLevel(tip: GameTooltip, data: TooltipData)
    local info = item.get(data.id :: number)
    if info then tip:AddLine(`iLvl {info.itemLevel}`, 1, 1, 1) end
end

local h = tooltip.hook("Unit", function(tip: GameTooltip, data: TooltipData) … end)
h:disable()                          -- hooks cannot be removed; this stops calling the handler
tooltip.attach(MyAddonTooltip)       -- also apply hooks to a custom tooltip (fallback clients)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `@onTooltip(kind)` | function decorator | Registers at load time. Registering decorator: captures the final decorated function (L26). |
| `hook` | `(kind: TooltipKind, fn: (tip: GameTooltip, data: TooltipData) -> ()) -> TooltipHook` | |
| `TooltipHook` | `enable()`, `disable()`, `isEnabled()` | |
| `attach` | `(tip: GameTooltip)` | Fallback clients only: applies all hooks to an additional tooltip frame. A no-op on clients with the tooltip data system, which covers all tooltips. |
| `TooltipKind` | `"Item" \| "Spell" \| "Unit" \| …` | The full set comes from the flavor table. On fallback clients only `"Item"`, `"Spell"` and `"Unit"` are available, and other literal kinds are E8300 for those flavors. Unknown kinds are E8330. |
| `TooltipData` | record | Normalized: `kind`, `id?`, `guid?`, `unit?`, `link?`, `raw?` (the client's tooltip data where available). Values the client may return as secrets keep their `Secret` types. |

- Fallback clients hook `GameTooltip`, `ItemRefTooltip` and the shopping
  tooltips with `HookScript`, reading the item, spell or unit with the
  tooltip's getter methods.
- Handlers run through the stdlib error policy. A failing handler never breaks
  the tooltip or other handlers.
- These are post-hooks only. They cannot suppress or replace tooltip content.

## Testing

`wowlua:mock` support used by this chapter: `mock.setUnit`, `mock.setGroup`,
`mock.secret(v)` / `mock.setRestricted(bool)` (secret simulation for
`issecretvalue` / `canaccessvalue`), `mock.setAuras(unit, auras)`,
`mock.fireUnitAura(unit, changes?)`, `mock.setSpell(id, info)`,
`mock.setSpellCooldown(id, info)`, `mock.setItem(id, info, { cached?, delay? })`
(data arrives after `mock.advanceTime`), and `mock.showTooltip(kind, data)`,
which runs post-hooks against a stub `GameTooltip` whose added lines can be read
back with `tip:lines()`.
