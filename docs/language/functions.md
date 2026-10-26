# Functions

wowlua functions are Lua functions with types for their parameters and
results, default values, overloads and generics. This page covers each, and
doc comments whose examples run as tests.

## Type parameters and results

A type goes after each parameter, and the result type after the closing
parenthesis. `(A, B)` declares several results, `()` none.

```wowlua title=src/money.wowlua
export function splitMoney(copper: number): (number, number, number)
    local gold = math.floor(copper / 10000)
    local silver = math.floor(copper % 10000 / 100)
    return gold, silver, copper % 100
end

export function announce(text: string): ()
    SendChatMessage(text, "GUILD")
end

local g, s, c = splitMoney(GetMoney())   -- three numbers
print(`You have {g}g {s}s {c}c`)
```

Leave the result type out and the function returns whatever its `return`
statements give. Write it when you want the body checked against it: a
`return "x"` in a function declared `: number` is E2001. Reading more values
than a function can give (`local a, b, c, d = splitMoney(1)`) is the warning
W2062, because `d` is always `nil`.

## Give a parameter a default

`= value` after a parameter's type fills it in when the caller passes nothing
or `nil`. Defaults run at call time, left to right, so a default may use the
parameters before it.

```wowlua title=src/layout.wowlua
local function placeIcon(icon: Texture, size: number = 32, gap: number = size / 8)
    icon:SetSize(size, size)
    icon:SetPoint("LEFT", gap, 0)
end

local tex = UIParent:CreateTexture()
placeIcon(tex)          -- 32 and 4
placeIcon(tex, 48)      -- 48 and 6
```

Each default becomes an `if` at the start of the function:

```lua
local function placeIcon(icon, size, gap)
	if size == nil then
		size = 32
	end
	if gap == nil then
		gap = size / 8
	end
	icon:SetSize(size, size)
	icon:SetPoint("LEFT", gap, 0)
end
```

Inside the function, `size` is a `number`. Callers see it as `number?` and may
leave it out. A default that reads its own parameter or a later one is E1004,
because that parameter isn't set yet.

## Make a parameter optional

A `?` after the type makes a parameter optional: the caller may leave it out,
and inside the function it may be `nil`.

```wowlua title=src/whisper.wowlua
local function whisper(target: string, text: string, prefix: string?)
    local line = prefix and `[{prefix}] {text}` or text
    SendChatMessage(line, "WHISPER", nil, target)
end

whisper("Thrall", "inv pls")
whisper("Thrall", "inv pls", "LFG")
```

Leaving out a parameter that isn't optional is caught at the call, in every
mode:

```text
error[E2007]: missing argument `text` of type `string`
  ┌─ src/whisper.wowlua:8:1
  │
8 │ whisper("Thrall")
  │ ^^^^^^^^^^^^^^^^^ here

error: aborting due to 1 previous error
```

A function with no annotations at all keeps Lua's rules: every parameter is
optional. In basic mode, passing more arguments than such a function takes is
the warning W2055. For an annotated function it's E2006.

## Take any number of arguments

`...: T` gives every extra argument the type `T`. `select("#", ...)`,
`select(i, ...)` and `{ ... }` are typed, so the values keep their type in a
loop.

```wowlua title=src/log.wowlua
local DEBUG = true

local function debugPrint(label: string, ...: any)
    if not DEBUG then return end
    local parts = { ... }
    for i = 1, select("#", ...) do
        parts[i] = tostring(parts[i])
    end
    print(`[{label}]`, table.concat(parts, " "))
end

local function total(...: number): number
    local sum = 0
    for _, n in ipairs({ ... }) do sum += n end
    return sum
end

debugPrint("loot", total(1, 2, 3), nil, true)
```

`total(1, "2")` is E2001. The table `{ ..., n = select("#", ...) }` with
`...: string` has the type `{ n: number, [number]: string? }`.

## Pass functions around

A function type reads `(parameters) -> results`. When you pass a function
literal where a function type is expected, its parameters take their types
from there, so most callbacks need no annotations.

```wowlua title=src/roster.wowlua
type UnitFilter = (unit: string) -> boolean

local function eachGroupMember(filter: UnitFilter, fn: (unit: string, index: number) -> ())
    local prefix = IsInRaid() and "raid" or "party"
    for i = 1, GetNumGroupMembers() do
        local unit = `{prefix}{i}`
        if filter(unit) then fn(unit, i) end
    end
end

eachGroupMember(UnitIsDeadOrGhost, function(unit, i)
    print(i, UnitName(unit), "needs a rez")   -- unit: string, i: number
end)
```

Parameter names in a function type are optional: `(string) -> boolean` is the
same type. The result after `->` extends as far as it can, so
`() -> number | nil` returns `number | nil`. Write `(() -> number)?` for a
function that may be missing.

## One function, several call shapes

When a function takes different arguments that give different results, list
each form with `overload function`, then write one implementation that handles
them all. Callers see only the overloads.

```wowlua title=src/items.wowlua
type ItemInfo = { id: number, name: string }
local byID: { [number]: ItemInfo } = {}
local byName: { [string]: ItemInfo } = {}

overload function findItem(id: number): ItemInfo?
overload function findItem(name: string): ItemInfo?
local function findItem(key: number | string): ItemInfo?
    if type(key) == "number" then
        return byID[key]
    end
    return byName[key]
end

findItem(19019)
findItem("Thunderfury")
```

A call that fits no overload lists them all:

```text
error[E2012]: no overload matches the arguments (true); overloads:
  (id: number) -> { id: number, name: string }?
  (name: string) -> { id: number, name: string }?
   ┌─ src/items.wowlua:16:1
   │
16 │ findItem(true)
   │ ^^^^^^^^^^^^^^ here

error: aborting due to 1 previous error
```

- The overloads come right before the implementation, which has the same name
  (E3047 otherwise) and must accept every one of them (E3044). It may be a
  `local function`, an `export function`, or a global `function` in a file
  without imports and exports, whose calls in that file see the overloads.
- `export overload function` before an `export function` exports them, and
  importers see the overloads too.
- Overloads are top-level only. Methods of a [class](classes.md) have them as
  well.
- They exist only for the checker. One function is emitted:

```lua
local function findItem(key)
	if type(key) == "number" then
		return byID[key]
	end
	return byName[key]
end
```

## Generic functions

Type parameters in angle brackets keep the types of what you pass. They are
always inferred at the call, and `extends` limits what may be passed.

```wowlua title=src/util.wowlua
type Member = { name: string, class: string }
local members: Member[] = {}

local function map<T, U>(list: T[], fn: (value: T) -> U): U[]
    local out: U[] = {}
    for i, v in ipairs(list) do out[i] = fn(v) end
    return out
end

local names = map(members, function(m) return m.name end)   -- string[]

local function widest<F extends Region>(regions: F[]): F?
    local best: F? = nil
    for _, r in ipairs(regions) do
        if not best or r:GetWidth() > best:GetWidth() then best = r end
    end
    return best
end

local buttons: Button[] = {}
local button = widest(buttons)                               -- Button?
print(#names, button)
```

Inside `widest`, `r:GetWidth()` is known because every `F` is a `Region`. Pass
a list of buttons and you get a `Button?` back. Pass a list of numbers and the
call is E2014: `number` doesn't satisfy the constraint `Region`.

A function that passes on whatever another function takes and gives uses type
packs, written `A...`.
[Types §2.2.10](../../spec/language/02-types.md#2210-type-packs) has the
details.

## Functions that check a type

A result type `param is T` makes a function a check the caller can rely on:
where the call is true, the argument is a `T`. Where it's false, the members
of the argument's type that are a `T` are excluded.

```wowlua title=src/frames.wowlua
local function isButton(region: Region): region is Button
    return region:GetObjectType() == "Button"
end

local function press(button: Button)
    button:Click()
end

local function clickAll(regions: Region[])
    for _, region in ipairs(regions) do
        if isButton(region) then
            press(region)            -- region: Button here
        end
    end
end

clickAll({})
```

Outside the `if`, `press(region)` is E2001: a `Region` is not a `Button`.
`issecretvalue` and `canaccessvalue` are such checks too
([Types](types.md#secret-values-in-combat)).

## Methods and self

`function t:m()` works as in Lua, and `self` has the type of `t`. For objects
with state, a [class](classes.md) gives you the same with less to write.

```wowlua title=src/tracker.wowlua
local Tracker = { kills = 0 }

function Tracker:add(count: number = 1)
    self.kills += count              -- self: { kills: number }
end

Tracker:add()
Tracker:add(5)
```

## Document a function, with examples that run

A `---` comment above a function is its documentation: VS Code shows it when
you hover or complete the name. A ` ```wowlua ` block in it is an example, and
`wowlua test` runs every example as a test, with `expect` and everything the
module exports at hand.

```wowlua title=src/format.wowlua
import splitMoney from "./money"

--- Formats copper as gold, silver and copper.
---
--- ```wowlua
--- expect(formatMoney(123456)).toBe("12g 34s 56c")
--- expect(formatMoney(50)).toBe("50c")
--- ```
export function formatMoney(copper: number): string
    local g, s, c = splitMoney(copper)
    if g > 0 then return `{g}g {s}s {c}c` end
    if s > 0 then return `{s}s {c}c` end
    return `{c}c`
end
```

```console
$ wowlua test
 PASS src/format.wowlua (camelot, 0.07s)
   ✓ formatMoney example
      Tested 1 passed, 0 failed, 0 skipped (1 file) in 0.15s
```

- The test is named after the function. A second example in the same comment
  is `formatMoney example 2`.
- A failing example, or a type error in one, points at its line in the
  comment, in the terminal and in the editor.
- Mark a block ` ```wowlua norun ` to show it without running it. Blocks in
  other languages never run.
- Examples run in test builds only. They are not part of your addon.
- More about tests in [Testing your addon](../guide/testing.md).
