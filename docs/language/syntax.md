# Syntax and sugar

wowlua adds a few short forms to Lua 5.1 for code that addons write all the
time. This page shows each one next to the Lua it compiles to, so you know
what runs in the game.

The Lua on this page is real compiler output from a release build, unbundled
and not minified. The [syntax chapter](../../spec/language/01-syntax.md) of the
specification has every rule.

## Put values into strings

A string in backticks can hold any expression in `{…}`. You no longer need a
`..` chain with `tostring` around each value.

```wowlua title=src/announce.wowlua
local function announce(name: string, count: number, item: string)
    local msg = `{name} needs {count} more {item}{count == 1 and "" or "s"}`
    SendChatMessage(msg, "PARTY")
end

announce("Thrall", 2, "Linen Bandage")
```

```lua
local function announce(name, count, item)
	local msg = tostring(name) .. " needs " .. tostring(count) .. " more " .. tostring(item) .. tostring(count == 1 and "" or "s")
	SendChatMessage(msg, "PARTY")
end
```

- A backtick string can span lines. The line breaks are part of the string.
- `\{` and `\}` give literal braces, and `` \` `` a backtick.
- Backtick strings also accept `\xFF`, `\u{2764}` and `\z`. Quoted strings
  stay exactly as in Lua 5.1, where these escapes mean something else.
- Constant values are folded into the text when the file is compiled:
  literals, `const` values, enum members and values from `wowlua:meta`. With
  `addon` imported from `wowlua:meta`, `` `{addon.title} loaded` `` becomes the
  plain string `"MyAddon loaded"`.
- There are no format specifiers. Use `string.format` for `%.1f` and the like.
- A backtick string is an expression, not a literal, so a call needs
  parentheses around it: ``print(`hi`)``, not ``print`hi` ``.

## Update a value in place

`x += 1` means `x = x + (1)`. Every arithmetic, concatenation and bitwise
operator has a compound form: `+=` `-=` `*=` `/=` `%=` `^=` `..=` `//=` `&=`
`|=` `<<=` `>>=`.

```wowlua title=src/tracker.wowlua
local stats = { kills = 0 }
local label = "Thrall"
local counts: { [number]: number } = {}

stats.kills += 1
label ..= " (offline)"
counts[C_Container.GetContainerItemID(0, 1)] -= 1
```

```lua
stats.kills = stats.kills + 1
label = label .. " (offline)"
do
	local __wl_k1 = C_Container.GetContainerItemID(0, 1)
	counts[__wl_k1] = counts[__wl_k1] - 1
end
```

A call inside the target runs once, not twice. A compound assignment has one
target, so `a, b += 1` is an error (E0111).

## Fall back when something is nil

`a ?? b` is `a`, unless `a` is `nil`; then it is `b`. `a or b` would also
replace `false`. `??` keeps it, which matters for options a player can turn
off.

```wowlua title=src/options.wowlua
local db: { showMinimap: boolean? } = {}
local cache: { [string]: number[] } = {}

local showMinimap = db.showMinimap ?? true   -- false stays false
local title = GetGuildInfo("player") ?? "no guild"

local function sample(unit: string)
    cache[unit] ??= {}                       -- create once, then reuse
    table.insert(cache[unit], UnitHealth(unit))
end

sample("player")
print(showMinimap, title)
```

```lua
local __wl_t1 = db.showMinimap
if __wl_t1 == nil then
	__wl_t1 = true
end
local showMinimap = __wl_t1 -- false stays false
```

- `b` is only evaluated when `a` is `nil`.
- `x ??= v` assigns only when `x` is `nil`: `if x == nil then x = v end`.
- Mixing `??` with `and` or `or` needs parentheses (E0110), so the order is
  never a guess.

## Reach into tables that might not be there

Optional chaining stops at the first `nil` and gives `nil`, where Lua would
raise "attempt to index a nil value".

```wowlua title=src/profile.wowlua
interface Profile
    icon: number?
end

local db: { profiles: { [string]: Profile }? } = {}

local function iconOf(name: string): number?
    return db.profiles?.[name]?.icon
end

print(iconOf("Default"))
```

```lua
local function iconOf(name)
	do
		local __wl_t1 = db.profiles
		if __wl_t1 ~= nil then
			__wl_t1 = __wl_t1[name]
			if __wl_t1 ~= nil then
				__wl_t1 = __wl_t1.icon
			end
		end
		return __wl_t1
	end
end
```

There is a form for each kind of access:

```wowlua fragment
local current = handle.profiles?:current()   -- method call
opts.onDone?.(result)                        -- call only if set
```

- `a?.b` reads a field, `a?.[k]` indexes, `a?:m()` calls a method, and
  `f?.()` calls a function that may be missing.
- Once a link is `nil`, the rest of the chain is skipped, including the
  arguments of its calls.
- Only `nil` stops a chain. `false?.x` still errors, as it does in Lua.
- A chain that ends in a call gives exactly one value, like `(f())`.
- You can't assign through a chain: `a?.b = 1` is an error (E0113).

## Skip to the next loop iteration

`continue` jumps to the next iteration of the innermost loop. Like `break`, it
must be the last statement of its block (E0114). Write `do continue end` to
use it in the middle of one.

```wowlua title=src/bags.wowlua
local function count(info: ContainerItemInfo)
    print(info.stackCount)
end

local function scan(bag: number)
    for slot = 1, C_Container.GetContainerNumSlots(bag) do
        local info = C_Container.GetContainerItemInfo(bag, slot)
        if not info then continue end
        if info.isLocked then break end
        count(info)
    end
end

scan(0)
```

```lua
local function scan(bag)
	for slot = 1, C_Container.GetContainerNumSlots(bag) do
		local __wl_brk1 = false
		repeat
			local info = C_Container.GetContainerItemInfo(bag, slot)
			if not info then
				break
			end
			if info.isLocked then
				__wl_brk1 = true
				break
			end
			count(info)
		until true
		if __wl_brk1 then
			break
		end
	end
end
```

Lua 5.1 has no `goto`, so the loop body is wrapped in a `repeat … until true`
that runs once, and `continue` becomes `break`. The flag only appears when a
loop uses both `continue` and `break`. `continue` is a keyword only at the
start of a statement that can't be anything else, so old code that uses it as
a variable name still works.

## Name a value that never changes

`const` declares a local that can't be reassigned (E2010). When its value is
known at compile time, wowlua writes the value wherever you use the name, and
no local is emitted at all. That keeps you away from Lua's limit of 200
locals per function.

```wowlua title=src/layout.wowlua
const ICON = 32
const PADDING = 4
const ROW = ICON + PADDING * 2
const ROW_WIDTH = ROW * 6

local row = CreateFrame("Frame")
row:SetSize(ROW_WIDTH, ROW)
```

```lua
local row = CreateFrame("Frame")
row:SetSize(240, 40)
```

A value counts as known when it is a literal, another folded constant, a
string, number or boolean from `wowlua:meta`, or arithmetic, `..`, equality
and logic on those. Anything else becomes a normal `local`. A table bound with
`const` can still change inside; only the name is fixed.

## Pull fields out of a table

`local { a, b } = t` declares one local per field and reads the field of the
same name. `= value` gives a default for a field that is `nil`.

```wowlua title=src/tooltip.wowlua
local function addSpell(tooltip: GameTooltip, spellID: number)
    local info = C_Spell.GetSpellInfo(spellID)
    if not info then return end
    local { name, iconID, castTime = 0 } = info
    tooltip:AddLine(`|T{iconID}:0|t {name} ({castTime / 1000}s)`)
end

addSpell(GameTooltip, 6673)
```

```lua
local name, iconID, castTime = info.name, info.iconID, info.castTime
if castTime == nil then
	castTime = 0
end
```

The table is evaluated once. A local name is read directly, as here. Any other
expression, such as a call, is stored in a temporary first. Only `local`
destructures, and there is no array form: for several return values, Lua's
`local a, b = f()` already does the job.

## Readable numbers and bit flags

Underscores separate digits, and `0b` writes a binary number. Bitwise
operators and integer division compile to WoW's `bit` library and
`math.floor`.

```wowlua title=src/flags.wowlua
const GOLD_CAP = 99_999_999_999
const FLAG_DIRTY = 0b0001
const FLAG_SHOWN = 0b0010

local flags = 0
local items = {}

flags |= FLAG_DIRTY
local shown = flags & FLAG_SHOWN ~= 0
local rows = #items // 6
print(shown, rows, GOLD_CAP)
```

```lua
flags = bit.bor(flags, 1)
local shown = bit.band(flags, 2) ~= 0
local rows = math.floor(#items / 6)
```

- `&` `|` `~` `<<` `>>` work on 32-bit integers, as WoW's `bit` library does,
  and need numbers. `~x` alone is `bit.bnot(x)`; `~=` is still "not equal".
- `>>` is a logical shift (`bit.rshift`).
- `a | b | c` becomes a single `bit.bor(a, b, c)`.
- `//` is floor division: `7 // 2` is `3`.
- An underscore must sit between two digits: `1_000` is fine, `1_` and
  `0x_FF` are errors (E0103).

## Types on locals and loop variables

Any local, loop variable or parameter can have a type after a colon. Types are
checked, then erased: `local count: number = 0` compiles to
`local count = 0`.

```wowlua title=src/slots.wowlua
local free: number[] = {}
for bag: number = 0, 4 do
    free[#free + 1] = C_Container.GetContainerNumFreeSlots(bag)
end
for bag: number, slots: number in ipairs(free) do
    print(`bag {bag - 1}: {slots} free`)
end
```

`value :: Type` tells the checker what a value is when you know better than it
does. Both are covered in [Types](types.md).

## Settings for one file

A comment that starts with `--!` at the top of a file, before any code,
changes how wowlua treats that file. It is still a Lua comment, so the file
runs anywhere.

```lua
--!nocheck
-- vendored code we do not want to touch
local Parser = {}
```

- `--!strict` checks the file in strict mode: `nil` must be spelled out as
  `T?`, reading or writing an undeclared global is an error, and parameters of
  functions that aren't callbacks need types.
- `--!basic` is the default mode.
- `--!nocheck` skips type checking. The file is still parsed and compiled.
- `--!nolint` turns off lint warnings for the file.

The project-wide default is `check.mode` in `wowlua.json`, and a directive
overrides it for its file. [Types](types.md) explains the modes.

## What stays out

WoW runs Lua 5.1, so some features of newer Lua versions are not part of
wowlua:

- `goto` and labels. `goto` stays an ordinary name, as in Lua 5.1.
- The `<const>` and `<close>` attributes of Lua 5.4. Use `const`.
- A separate integer type. All numbers are doubles, as in the game.
- `\x`, `\u{…}` and `\z` in quoted strings. Use a backtick string.
