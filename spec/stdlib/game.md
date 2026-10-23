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

## secret

`wowlua:secret` holds the small helpers secret values need (decision S26).
Each tests for secrecy first, so it never compares, tests or does arithmetic
on a secret. On clients without secret values nothing is secret.

```wowlua
import * as secret from "wowlua:secret"

local hp = UnitHealth("target")                       -- number | Secret<number>
bar:SetValue(secret.amount(hp))                       -- secrets pass, nil becomes 0
local shown = secret.known(hp, 0)                     -- number: 0 when secret
local name = secret.known(UnitName("target"), nil)    -- string?: nil when secret
if secret.flag(UnitIsUnit("target", "focus")) then … end   -- a secret boolean reads as false
if secret.flag(UnitInRaid("player")) then … end       -- legacy 1/nil answers too
if not secret.isSecret(hp) then print(hp + 1) end     -- narrows like issecretvalue
if secret.readable(hp) and hp > 0 then … end          -- present and not secret
```

| API | Signature | Notes |
|-----|-----------|-------|
| `isSecret` | `(v: unknown) -> v is Secret<unknown>` | `issecretvalue`, or `false` where the client has none |
| `known` | `<T, F>(v: T \| Secret<T> \| nil, fallback: F) -> T \| F` | `fallback` when `v` is secret or nil. The fallback may be of another type: on a `string \| Secret<string>`, `known(v, nil)` is a `string?` and `known(v, "?")` a `string`. |
| `flag` | `(v: unknown) -> boolean` | For conditions, by truthiness (so legacy `1`/`nil` answers work): a secret reads as `false` |
| `readable` | `(v: unknown) -> v is Plain` | Whether `v` is present and may be looked into: not nil, not a secret, and a table only when addons may index it (`canaccesstable` where the client has it), so code that walks tables it is handed checks it first. Narrows to `Plain` (`string \| number \| boolean \| table \| function`), so `number \| Secret<number> \| nil` becomes `number`. Not generic yet: generic predicates do not infer their type parameter. |
| `amount` | `(v: number \| Secret<number> \| nil) -> number \| Secret<number>` | For widgets that take secrets: nil becomes 0 |
| `asString`, `asNumber`, `asBoolean` | `(v: unknown) -> string?`, `-> number?`, `-> boolean?` | `v` when it has that type and is not secret, else nil: for values typed `unknown` (what `errors.try` gives, a loosely declared API's result, a callback's argument), where `known` needs the type already. Legacy `1`/`nil` answers are no booleans: read them with `flag` (decision S42). |

The generated `wow-api` declarations type the arguments Blizzard documents
with `SecretArguments = "AllowedWhenTainted"` (widget setters such as
`StatusBar:SetValue`, `FontString:SetText`) as `T | Secret<T>`, except those
marked `NeverSecret`.

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
| `info` | `(unit: UnitToken) -> UnitInfo?` | `nil` if the unit does not exist. Fields: `name` (without the realm; on WoW: Forever it includes the surname), `realm?` (other realms only), `fullName?` (nil when the name is secret), `guid?`, `class?`, `classFile?`, `level`, `isPlayer`, `isConnected`, `faction?`, `race?`, `role?`. Fields the client may return as secrets, the yes/no ones included, are typed `T \| Secret<T>`. |
| `exists` | `(unit: UnitToken) -> boolean` | |
| `fullName` | `(unit: UnitToken) -> string?` | `"Name-Realm"`, from `GetUnitName(unit, true)`; the own realm is filled in (`GetNormalizedRealmName`). `nil` if the name is secret. |
| `isSame` | `(a: UnitToken, b: UnitToken) -> boolean \| Secret<boolean>` | `UnitIsUnit`; may be secret in combat (read it with `secret.flag`) |
| `health` | `(unit: UnitToken) -> (number \| Secret<number>, number \| Secret<number>)` | Current and maximum |
| `healthPercent` | `(unit: UnitToken) -> (number \| Secret<number>)?` | 0 to 100. The client's `UnitHealthPercent` (with `CurveConstants.ScaleTo100`) where it has one, which may return a secret. Otherwise computed from `health` only when both values are accessible, else `nil`. |
| `power` | `(unit: UnitToken, powerType?: number) -> (number \| Secret<number>, number \| Secret<number>)` | |
| `classColor` | `(unit: UnitToken) -> ColorMixin?` | From `wowlua:color` (honors `CUSTOM_CLASS_COLORS`); `nil` when the class is secret |
| `group` | `(opts?: { includePlayer: boolean?, includePets: boolean? }) -> (() -> UnitToken?)` | Iterates `raid1..raidN` in a raid (the player's own raid token only with `includePlayer`), `party1..party4` (plus `player` when requested) in a party, and only `player` when solo. Pets follow their owner with `includePets`. A secret answer to "in a raid/group" or "is it me" counts as no, as do secret options. |
| `groupSize` | `() -> number` | `GetNumGroupMembers()`, with 1 when solo |

The client's newer APIs (`UnitHealthPercent`, `GetUnitName`) are detected at
run time in every flavor mode; the result is the same as a compile-time
choice.

### Roster

`unit.roster` keeps the group's members (the player included) by GUID and
reports who joins and leaves.

```wowlua
const m = unit.roster.byGuid(sourceGuid)            -- RosterMember?
if m then print(m.fullName, m.unit) end
unit.roster.onChange(function(joined: RosterMember[], left: RosterMember[])
    for _, m in ipairs(joined) do print(`{m.name} joined`) end
end)
```

| API | Signature | Notes |
|-----|-----------|-------|
| `roster.byGuid` | `(guid: string \| Secret<string>) -> RosterMember?` | `nil` for a secret GUID (combat log GUIDs can be) |
| `roster.byUnit` | `(unit: UnitToken) -> RosterMember?` | The member behind a group token |
| `roster.members` | `() -> RosterMember[]` | The player first, then by group index |
| `roster.onChange` | `(fn: (joined: RosterMember[], left: RosterMember[]) -> ()) -> Subscription` | After each change |
| `RosterMember` | `{ guid: string, unit: UnitToken, name: string \| Secret<string>, fullName?: string, classFile?: string \| Secret<string> }` | `fullName` is nil while the name is secret |

Semantics (decision S34):

- **Lazy.** The roster starts on first use: it registers `GROUP_ROSTER_UPDATE`,
  `PLAYER_ENTERING_WORLD` and `UNIT_NAME_UPDATE`, and the group at that moment
  counts as already there (no joins are reported for it).
- **Coalesced.** A burst of roster events rebuilds once, 0.2 seconds after the
  first. A query while a rebuild is pending rebuilds first, so queries are
  always current.
- **Moves are not changes.** A member whose unit token changes (party to raid,
  a raid reshuffle) keeps their entry with the new token.
- **Secrets.** A member whose GUID reads as secret keeps the last known entry
  for that unit token; one never seen is left out until it can be read.

## aura

`wowlua:aura` reads auras and wraps the engine's aura containers. On
clients with secret auras (retail 12.x, WoW: Forever) every addon read of
auras errors in combat ("cannot be accessed when secret while tainted"), in
the open world too. The reads here are guarded and find nothing then; the
engine's **aura containers** are the way to show auras in combat: the
engine fills them, the addon only styles their buttons (decision S26).

```wowlua
import * as aura from "wowlua:aura"

if aura.readable("player") then
    for a in aura.each("player", "HELPFUL") do
        print(a.name, a.applications)
    end
end

local debuff = aura.find("target", 589, "HARMFUL|PLAYER")    -- AuraData?

local sub = aura.onChange("player", function(u: UnitToken, changes: UnitAuraUpdateInfo?)
    if changes == nil or changes.isFullUpdate then rescan() else applyDelta(changes) end
end)

-- Shown in combat too: the engine matches filters and spell lists itself.
local box = aura.container(frame, {
    unit = "target",
    groups = {
        { key = "mine", filter = "HARMFUL|PLAYER", max = 8 },
        { key = "curses", filter = "HARMFUL", dispelTypes = { "Curse" } },
        { key = "tracked", filter = "HELPFUL", spells = { 1459, 21562 } },
    },
    size = 26,
    growth = { "LEFT", "DOWN" },
})

-- Groups that cost nothing until they have room (the engine makes a batch
-- of buttons for every group it has).
if aura.hasContainers() then
    local raid = aura.container(frame, { unit = "raid1", lazy = true, groups = {
        { key = "buffs", filter = "HELPFUL", max = 0, exclude = { 871 }, order = 2 },
        { key = "debuffs", filter = "HARMFUL", max = 3, size = 18, spacing = 1, order = 1 },
    } })
    raid:setGroupMax("buffs", 6)                -- added to the engine now
    raid:setGroupSize("debuffs", 22)            -- the buttons resize out of combat
end
```

| API | Signature | Notes |
|-----|-----------|-------|
| `readable` | `(unit: UnitToken) -> boolean` | Whether the unit's auras can be read right now (false in combat on clients with secret auras) |
| `each` | `(unit: UnitToken, filter: AuraFilter) -> (() -> AuraData?)` | Every aura; nothing when they cannot be read. Auras the client hands back as a secret table, or one addons may not index (`canaccesstable`), end the iteration. |
| `forEach` | `(unit: UnitToken, filter: AuraFilter, fn: (a: AuraData) -> boolean?)` | Allocation-free variant. Returning `true` stops iteration (a secret `true` does not). |
| `find` | `(unit: UnitToken, spellId: number, filter: AuraFilter) -> AuraData?` | Auras whose `spellId` is secret are **skipped** |
| `findByName` | `(unit: UnitToken, name: string, filter: AuraFilter) -> AuraData?` | Same skipping rule |
| `byInstanceId` | `(unit: UnitToken, id: number) -> AuraData?` | `nil` when the client refuses or hands back what `each` stops at |
| `onChange` | `(unit: UnitToken, fn: (unit: UnitToken, changes: UnitAuraUpdateInfo?) -> ()) -> Subscription` | `UNIT_AURA` filtered to `unit` (a unit-filtered event frame, [Core: events](core.md#events)). `changes` is the client's update info; `nil` or `isFullUpdate` means rescan. |
| `hasContainers` | `() -> boolean` | Whether this client has aura containers (`Blizzard_AuraContainer`'s `CustomAuraContainerTemplate`, through `C_XMLUtil.GetTemplateInfo`), so that a caller can check before `container` raises |
| `container` | `(parent: Frame, opts: ContainerOptions) -> AuraContainer` | The engine's container (`CreateFrame("AuraContainer", …, "CustomAuraContainerTemplate")`, loading `Blizzard_AuraContainer`). E8300 on flavors without aura containers. |
| `ContainerOptions` | `{ unit, groups: AuraGroup[], size?, spacing?, anchor?, growth?, lineWidth?, lazy? }` | `size` 30 and `spacing` 2 by default; `anchor` the corner icons start from (`"TOPLEFT"`), `growth` `{ "RIGHT", "DOWN" }` by default; `lineWidth` before icons wrap. With `lazy`, a group whose `max` is 0 goes to the engine only when `setGroupMax` raises it; until then the setters only record their values. Sizes, counts and `lazy` are compared and handed to buttons, which refuse secrets from addon code: a secret one reads as its default. |
| `AuraGroup` | `{ key, filter, max?, spells?, exclude?, dispelTypes?, size?, spacing?, order?, gap?, initialize? }` | `max` 16 by default; `spells`, `exclude` and `dispelTypes` become the engine's candidate filters (`includeSpellIDs`, `excludeSpellIDs`, `includeDispelTypes`; matched in combat too; spell IDs only where the client permits matching; secret IDs in `spells` and `exclude` are skipped). `size` and `spacing` (between the group's icons) default to the container's; `order` places the group among the groups, lower first (default: its position in `groups`, which a lazy group added later keeps); `gap` is extra space before the group. `initialize(button: AuraButton)` builds each button once, after the button is sized to the group's size. |
| `AuraContainer` | `frame`, `setUnit(unit)`, `refresh()`, `setGroupMax(key, n)`, `setGroupSpells(key, ids?)`, `setGroupFilter(key, filter)`, `setGroupSize(key, size)`, `setFlow(anchor, growth?)`, `setLineWidth(width?)`, `release()` | Containers rescan when their token points at someone new (`PLAYER_TARGET_CHANGED`, `PLAYER_FOCUS_CHANGED`, boss and group changes). `setGroupSpells` keeps the group's `exclude` and `dispelTypes`. `setGroupSize` changes the engine's layout at once and resizes the group's existing buttons when combat ends (the buttons refuse addon code while auras are secret); new buttons take the new size. Unknown keys, and secret sizes and counts, are ignored. Engine containers cannot be freed: `release` parks one (no unit, hidden) until the next `setUnit`. |
| `AuraButton` | `Button & AuraButtonApi` | The engine's button: `SetIcon`, `SetApplicationCount`, `SetDurationText`, `SetDurationBar`, `SetDurationCooldown`, `AddDispelTypeTexture`, `SetSpellName`, `SetCasterName`, `SetCancelAuraButtons`, `SetHideTooltipInCombat`, `SetTooltipAnchorPoint`. Register regions only in `initialize`; the engine then drives them. |

- **`AuraData`** is the type the flavor's `wow-api` declarations give
  `C_UnitAuras` aura data. Every flavor has `C_UnitAuras`, so no legacy
  `UnitAura` fallback is needed. On clients with secret auras its fields may
  be secret; the module never compares them.
- **`AuraFilter`** is a string of `|`-separated words. Each flavor's words
  come from its client's `AuraUtil.AuraFilters`, extracted into the
  `AuraFilterWord` type of its `wow-api` declarations (decision S27; `!`
  negation where the flavor has it). Literal filters are checked for every
  targeted flavor (E8310); container groups' filters too.
- **The default button** (no `initialize`) looks like the client's buffs
  (decisions S17, S28): the icon, the stack count at the bottom right, the
  engine's duration text below the icon, a debuff-type border on harmful
  auras, and right click cancels the player's own buffs.
- Engine calls are guarded: an error inside one would abort the engine's
  whole button batch. The engine calls used are `Blizzard_AuraContainer`'s
  (`CustomAuraContainerSharedMixin`): `AddAuraGroup`,
  `SetAuraGroupMaxFrameCount`, `SetAuraGroupCandidateFilters`,
  `SetAuraGroupFilterString`, `SetAuraGroupLayout` (`elementWidth`,
  `elementHeight`, `elementSpacing`, `lineSpacing`, `layoutIndex`,
  `groupSpacing`), `GetAuraGroupFrameCount`, `GetAuraGroupFrame`, `SetUnit`
  and `UpdateAllAuras`. Layout methods renamed between client builds
  (`SetFlowLayout…`/`SetAuraLayout…`) are tried in turn.
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
| `info` | `(spell: number \| string) -> SpellInfo?` | `name`, `iconID`, `originalIconID`, `castTime`, `minRange`, `maxRange`, `spellID`, from `C_Spell.GetSpellInfo` (every flavor's client has it) |
| `name`, `icon` | `(spell) -> string?`, `(spell) -> number?` | |
| `link` | `(spell) -> string?` | |
| `description` | `(spell: number) -> string?` | May be empty until the spell data is loaded. Use `wowlua:item`'s `loadSpell` first. |
| `cooldown` | `(spell) -> CooldownInfo?` | `startTime`, `duration`, `modRate` (each `number \| Secret<number>`: secret in combat on restricted clients), `isEnabled`, `isActive` (readable there), `isOnGCD?`. `nil` when the client hands back a secret table or one addons may not index. Falls back to the global `GetSpellCooldown` where `C_Spell.GetSpellCooldown` is missing. |
| `onCooldown` | `(spell) -> boolean` | Whether it is cooling down: `isActive`, readable in combat too |
| `cooldownDuration` | `(spell, ignoreGCD?: boolean) -> LuaDurationObject?` | For secret-safe display with `Cooldown:SetCooldownFromDurationObject`; `nil` where the client has no duration objects |
| `charges` | `(spell) -> SpellChargeInfoSecret?` | `currentCharges`, `cooldownStartTime`, `cooldownDuration`, `chargeModRate` (each `number \| Secret<number>`: secret in combat on restricted clients), `maxCharges`, `isActive` (readable there). `nil` when the client hands back a table addons may not index. |
| `isKnown` | `(spellId: number, isPet?: boolean) -> boolean` | `C_SpellBook.IsSpellKnown`, in the player's or the pet's spellbook |

## item

`wowlua:item` loads and caches item **and spell** data asynchronously.
WoW loads item and spell data from the server lazily. The first query for an
uncached item returns `nil`, and the data arrives later.

```wowlua
import * as item from "wowlua:item"

item.load(19019, function(data: ItemData?)
    if data then print(data.link, data.itemLevel) end
end)

local cached = item.get(19019)                   -- ItemData? (nil and starts loading if uncached)

item.loadMany({ 19019, 17182 }, function(all: { [number]: ItemData? }) … end)

item.loadSpell(133, function(ok: boolean) print(spell.description(133)) end)

-- inside a wowlua:jobs job
local info = item.await(ctx, 19019)              -- yields until loaded
```

| API | Signature | Notes |
|-----|-----------|-------|
| `get` | `(item: number \| string) -> ItemData?` | Returns immediately. On a cache miss, it starts loading and returns `nil`. A string is an item link, or a name the client has cached. |
| `load` | `(item: number \| string, fn: (data: ItemData?) -> (), opts?: LoadOptions) -> LoadHandle` | `fn` runs once the data is available. It receives `nil` if the item does not exist or loading timed out. If the item is already cached, `fn` runs synchronously before `load` returns. |
| `loadMany` | `(items: (number \| string)[], fn: (all: { [number]: ItemData? }) -> (), opts?) -> LoadHandle` | One callback after all items have resolved |
| `configure` | `(opts: { cacheSize: number? })` | The cache's size (default 500) |
| `loadSpell` | `(spellId: number, fn: (ok: boolean) -> (), opts?) -> LoadHandle` | Ensures spell data (name, description) is loaded |
| `await` | `(ctx: JobContext, item: number \| string, opts?: LoadOptions) -> ItemData?` | Yields the job until loaded ([Infrastructure: jobs](infrastructure.md#jobs)) |
| `LoadOptions` | `{ timeout: number? }` | Default timeout: 10 seconds |
| `LoadHandle` | `Cancel()`-style handle: `cancel()`, `isDone(): boolean` | A cancelled load never calls its callback |
| `ItemData` | record | `id`, `name`, `link`, `quality`, `itemLevel`, `minLevel`, `type`, `subType`, `stackCount`, `equipLoc`, `icon`, `sellPrice`, `classID`, `subclassID`, `bindType`, `expansionID`. (The API's own `ItemInfo` is the item reference, `number \| string`; the record has its own name so the two do not shadow each other.) |

Implementation: `C_Item.RequestLoadItemDataByID` and `C_Spell.RequestLoadSpellData`,
answered by `ITEM_DATA_LOAD_RESULT` (or `GET_ITEM_INFO_RECEIVED`) and
`SPELL_DATA_LOAD_RESULT`; every flavor's client has them, so no item or
spell mixins are needed. Concurrent loads of the same item (and `get`s
while it loads) share one request. Loaded `ItemData` records are cached per
addon. The cache holds at most `item.configure({ cacheSize = 500 })`
entries, with least-recently-used eviction. Callbacks run through the stdlib
error policy ([Infrastructure: errors](infrastructure.md#errors)).

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
| `TooltipKind` | `string`: an `Enum.TooltipDataType` name (`"Item"`, `"Spell"`, `"Unit"`, `"Mount"`, …) | Checked per targeted flavor from its API declarations (decision S22): a name in no flavor's `Enum.TooltipDataType` is E8330; a kind a targeted flavor lacks is E8300 for it. A flavor has the tooltip data system when its declarations include `C_TooltipInfo`; fallback flavors have only `"Item"`, `"Spell"` and `"Unit"`. A hook under `@if` is checked only for the flavors that keep it. At run time, `hook` errors for a kind the client lacks. |
| `TooltipData` | record | Normalized: `kind`, `id?`, `guid?` (`string \| Secret<string>`), `unit?`, `link?`, `raw?` (the client's tooltip data where available). `id`, `unit` and `link` are nil when the client's are secret; `guid` passes through. Client data addons may not index (`canaccesstable`) gives only `kind`. |

- Fallback clients hook `GameTooltip`, `ItemRefTooltip` and the shopping
  tooltips with `HookScript`, reading the item, spell or unit with the
  tooltip's getter methods.
- Handlers run through the stdlib error policy. A failing handler never breaks
  the tooltip or other handlers.
- With the data system, forbidden tooltips (`tip:IsForbidden()`) are skipped.
  `TooltipHook` is a class; hooks run in registration order.
- These are post-hooks only. They cannot suppress or replace tooltip content.
- Hooks run synchronously inside the tooltip's setter (`SetSpellByID`,
  `SetUnit`, …) on both paths, so a flag a handler sets is known right after
  the setter returns.

## Testing

`wowlua:mock` support used by this chapter: `mock.setUnit`, `mock.setGroup`,
`mock.secret(v)` / `mock.setRestricted(bool)` (secret simulation for
`issecretvalue` / `canaccessvalue`), `mock.setAuras(unit, auras)`,
`mock.fireUnitAura(unit, changes?)`, `mock.setSpell(id, info)`,
`mock.setSpellCooldown(id, info)`, `mock.setItem(id, info, { cached?, delay? })`
(data arrives after `mock.advanceTime`), and `mock.showTooltip(kind, data)`,
which runs post-hooks against a stub `GameTooltip` whose added lines can be read
back with `tip:lines()`.
