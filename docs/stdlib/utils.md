# Tables, text and formatting

The helpers most addons end up writing in a `Utils.lua`: deep copies and
sorting, containers for combat data, string functions that understand
accented and non-Latin names, localized durations, gold and numbers, and
one way to handle errors.

None of the table, container and UTF-8 helpers raise an error on a secret
value: they pass it on or skip it, as each section says, so they keep
working in combat.

## Copy, merge and sort tables

```wowlua title=src/profile.wowlua
import * as tbl from "wowlua:table"

interface Member
    name: string
    role: string
end

local profile = { window = { width = 300 }, characters = { Thrall = true, Jaina = true } }
local preset = { width = 300, height = 200 }

local copy = tbl.copy(profile)                       -- deep copy
tbl.defaults(copy.window, preset)                    -- fills in what is missing
tbl.merge(copy, { window = { height = 250 } })       -- nested tables merge

if not tbl.equal(copy.window, preset) then
    print("the window was changed")
end

for _, name in ipairs(tbl.keys(profile.characters)) do   -- sorted
    print(name)
end

export function names(members: Member[]): string[]
    return tbl.map(members, function(m: Member) return m.name end)
end
```

`copy` keeps shared tables shared and copies cycles as cycles. Keys stay as
they are, so a frame used as a key is still that frame, and metatables are
left out. When the copy goes into saved variables, pass `{ shared = false }`:
then no two places in the copy share a table (and a cycle is an error).

`merge` copies the second table into the first: nested tables merge, other
values replace. `defaults` only fills in what the first table lacks, nested
tables too.

### Sorting that never errors

`tbl.sort` is stable, so items that compare equal keep their order, and it
never raises "invalid order function for sorting", even with an order that
contradicts itself. `tbl.by` builds an order from a key: numbers and strings
ascending, or descending with `true` as the second argument. Entries whose
key is nil or secret go last.

```wowlua title=src/raid-list.wowlua
import * as tbl from "wowlua:table"

interface Member
    name: string
    role: string
end

interface Entry
    at: number
end

-- by role, alphabetical inside each role
export function sortMembers(members: Member[])
    tbl.sort(members, tbl.by(function(m: Member) return m.name end))
    tbl.sort(members, tbl.by(function(m: Member) return m.role end))
end

-- in combat, auras whose expiration is secret go last
export function sortAuras(auras: AuraData[])
    tbl.sort(auras, tbl.by(function(a: AuraData) return a.expirationTime end))
end

-- keep a list sorted while adding to it
local queue: Entry[] = {}
export function schedule(entry: Entry): number
    return tbl.insertSorted(queue, entry, tbl.by(function(e: Entry) return e.at end))
end
```

`insertSorted` puts the value after the ones equal to it and returns its
position. `searchSorted` finds a value in a sorted list by binary search.
Without an order, `sort` sorts numbers and strings ascending.

### Reading data you don't fully trust

Saved variables from an old version, and tables the game hands you, may
not have the shape you expect. These read them without a chain of `if`
checks:

```wowlua title=src/compat.wowlua
import * as tbl from "wowlua:table"

local frame = CreateFrame("Frame")
local saved: { [string]: unknown } = {}

local target = tbl.field(frame, "ScrollBox", "ScrollTarget")   -- nil at the first missing step
table.insert(tbl.listAt(saved, "errors"), "message")           -- creates the list if needed

export function each(maybeList: unknown)
    for _, entry in ipairs(tbl.list(maybeList)) do             -- empty if it isn't a list
        print(entry)
    end
end

print(target)
```

`recordAt` is `listAt` for a record with string keys. Tables the addon may
not read, such as secret ones, count as missing or empty.

| Function | What it does |
|----------|--------------|
| `copy`, `shallowCopy`, `merge`, `defaults` | Copying and combining |
| `equal`, `count`, `isEmpty` | Comparing and counting (all entries, not only the list part) |
| `keys`, `values` | A table's keys (sorted) or values, as a list |
| `filter`, `map`, `find`, `contains` | List helpers |
| `sort`, `by`, `order`, `insertSorted`, `searchSorted` | Stable ordering that is safe with secrets |
| `field`, `list`, `listAt`, `recordAt` | Safe reads of loosely shaped data |
| `weakKeys`, `weakValues` | Tables that don't keep their keys or values alive |

`weakKeys` is useful for noting something about the game's frames without
writing fields onto them:
`local seen: { [Frame]: boolean } = tbl.weakKeys()`.

## Containers for combat data

`wowlua:collections` has three classes that addons otherwise copy from one
another. They reuse their slots, so pushing in a busy combat log handler
creates no garbage, and they store secret values without looking at them.

```wowlua title=src/dps.wowlua
import RingBuffer, Heap, LruCache from "wowlua:collections"
import by from "wowlua:table"

local hits: RingBuffer<number> = RingBuffer.new(30)        -- the last 30 hits
local expiring: Heap<AuraData> = Heap.new(by(function(a: AuraData) return a.expirationTime end))
local names: LruCache<string, string> = LruCache.new(200)

export function onHit(amount: number)
    hits:push(amount)
    print("newest", hits:at(-1))
end

export function onAura(aura: AuraData)
    expiring:push(aura)
    print("soonest", expiring:peek()?.name)
end

export function remember(guid: string, name: string)
    names:set(guid, name)
    print(names:get(guid))
end
```

- `RingBuffer` keeps the last N values, oldest first. `push` returns the value
  a full buffer pushed out. `at(1)` is the oldest and `at(-1)` the newest;
  `shift`, `pop`, `each` and `toList` work from the oldest.
- `Heap` is a priority queue: `pop` and `peek` give the smallest by your
  order. Secret values come out after all others.
- `LruCache` holds at most N entries and drops the one used longest ago.
  A secret key finds nothing and is not stored.

## Names with umlauts, Cyrillic and Hangul

WoW's Lua counts bytes, so `string.sub(name, 1, 1)` cuts "Ärger" in the
middle of the "Ä". `wowlua:utf8` counts characters instead.

```wowlua title=src/nameplate.wowlua
import * as utf8 from "wowlua:utf8"

local nameText = UIParent:CreateFontString(nil, "OVERLAY", "GameFontNormal")

export function update(unit: UnitToken)
    nameText:SetText(utf8.truncate(UnitName(unit) ?? "", 10, "…"))

    local name = "Ärgerlich"
    local initial = utf8.sub(name, 1, 1)         -- "Ä", not half of it
    print(initial, utf8.len(name))               -- Ä 9

    for ch in utf8.chars(name) do
        print(ch)
    end
end
```

`truncate` cuts to at most that many characters, the ellipsis included. A
secret name in combat can't be read, so `truncate` returns it whole and the
line above still works mid-fight; let the font string cut it on screen with
its width and `SetWordWrap(false)`. The other functions read a secret as an
empty string. A byte that starts no valid character counts as one
character, so any string works.

## Durations, gold and big numbers

`wowlua:format` uses the client's own localized formats, so a German player
sees German units without your addon shipping translations for them.

```wowlua title=src/display.wowlua
import * as format from "wowlua:format"

print(format.duration(3900))                     -- "1 Hr 5 Min"
print(format.short(95))                          -- one unit, as aura timers show it
print(format.clock(187))                         -- "3:07"
print(format.ago(7200))                          -- "2 h ago"
print(format.money(123456))                      -- gold, silver and copper with coin icons
print(format.money(123456, { short = true }))    -- "12g 34s 56c"
print(format.number(1234567))                    -- "1,234,567"
print(format.abbreviate(8500))                   -- "8.5K"
print(format.percent(0.456))                     -- "46%"
print(format.date())                             -- today, in the locale's short date format
```

- `duration` shows two units by default. `{ units = 3 }` shows more,
  `long = true` writes the unit words out, and `noSeconds = true` leaves the
  seconds out.
- `clock(seconds, true)` always shows the hours.
- `ago` takes the seconds since. With `{ long = true }` it writes "1 hour
  ago" and goes on to months and years. Its words are English, since the
  client has no format of its own for this.
- `money` leaves out units that are zero. `text = true` writes the client's
  words instead of coin icons.
- `percent(fraction, decimals?)` rounds to whole percents unless you ask for
  decimals.
- `number` and `abbreviate` accept a secret number and return a secret
  string, which a font string can still show.

## When something goes wrong

Every function the standard library calls for you runs isolated: event
handlers, lifecycle hooks, timers, hooks, tooltip, comm and callback
handlers, menu, dialog, list and settings callbacks. One that fails is
reported, and the others still run. The error goes to the game's error
handler, so the default error frame and addons such as BugSack see it, with
your addon's name in front. An identical error from the same place is
reported at most once every 10 seconds, with a count of the repeats, instead
of flooding the screen.

`wowlua:errors` gives your own code the same treatment:

```wowlua title=src/safe.wowlua
import * as errors from "wowlua:errors"

local statusIcon = UIParent:CreateTexture(nil, "OVERLAY")

local function refresh()
    statusIcon:Show()
end

local function computeTotals(rows: number[]): number
    local total = 0
    for _, n in ipairs(rows) do
        total += n
    end
    return total
end

local safeRefresh = errors.wrap(refresh)             -- never breaks the caller
safeRefresh()

local ok, total = errors.call(computeTotals, { 1, 2, 3 })   -- false if it raised (and was reported)
print(ok, total)

errors.report("unexpected state", { module = "sync" })

errors.onError(function(msg: string, info: errors.ErrorInfo)
    statusIcon:SetVertexColor(1, 0, 0)               -- show the player that something went wrong
    print(msg, info.component)
end)
```

The listener gets the message and an `ErrorInfo`: the standard library
component that called the failing function, the `owner` it was registered
with, and the stack. In debug builds the message also names the module and
the component. `errors.count()` returns how many errors your addon has
reported since it loaded, and `errors.configure({ rateLimit = 30 })` changes
the 10 seconds.

### Calling game functions a client may not have

Some API functions only exist once a Blizzard addon has loaded, or raise an
error on input they don't expect. `tryValue` calls such a function, here
[`C_TradeSkillUI.GetRecipeInfo`](../reference/api.md), if it exists, and
returns its first result with its own type, or nil. It reports nothing:

```wowlua title=src/professions.wowlua
import * as errors from "wowlua:errors"

export function showRecipe(recipeID: number)
    local recipe = errors.tryValue(C_TradeSkillUI.GetRecipeInfo, recipeID)   -- TradeSkillRecipeInfo?
    if recipe then
        print(recipe.name)
    end
end
```

`errors.try(fn, ...)` does the same for a value of unknown type and returns
all its results, typed `unknown`; read them with `secret.asString` and the
like ([units, auras, spells and items](game.md#first-secret-values)).
`errors.tryMethod(obj, "Name", ...)` calls a method that may be missing.
Use `call` for your own code, which should be reported when it fails, and
the `try` functions for the game's.

| Function | What it does |
|----------|--------------|
| `errors.wrap(fn)`, `errors.call(fn, …)` | Isolate your own code |
| `errors.report(err, context?)` | Report through the same channel |
| `errors.onError(fn)`, `errors.count()` | Notice errors in your interface |
| `errors.try`, `tryValue`, `tryMethod` | Silently call functions a client may lack |
| `errors.configure({ rateLimit })` | How often a repeated error is shown |
