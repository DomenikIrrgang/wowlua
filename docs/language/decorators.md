# Decorators

A decorator is a line starting with `@` above a function, class or field. It
says what should happen to the declaration: register it for an event, limit
how often it runs, profile it, or leave it out of release builds. This page
covers the built-in decorators, the ones from the standard library, and
writing your own.

## Your first decorator

Here is the usual event frame in plain Lua:

```lua
local f = CreateFrame("Frame")
f:RegisterEvent("PLAYER_ENTERING_WORLD")
f:SetScript("OnEvent", function(self, event, isLogin, isReload)
    if isLogin then
        print("Welcome back")
    end
end)
```

With `@event` from `wowlua:events`, the function is the handler and its
parameters are the event's payload:

```wowlua title=src/main.wowlua
import event from "wowlua:events"

@event("PLAYER_ENTERING_WORLD")
local function onEnter(isLogin: boolean)
    if isLogin then
        print("Welcome back")
    end
end
```

- A decorator goes on its own line above the declaration, or on the same line
  before it. Its arguments start on the same line as its name. Besides
  `@name(...)`, the Lua call forms `@name "text"` and `@name { key = value }`
  work too.
- On an exported declaration the decorators come before `export`:
  `@profile export function refresh() … end`.
- The event name is checked. The payload types come from the game's API (see
  the [WoW API reference](../reference/api.md)), and a handler whose
  parameters don't fit them is an error. A misspelled event is reported before
  the game loads anything, instead of being a handler that never runs:

```text
error[E8003]: unknown event or message `PLAYER_ENTERNG_WORLD`
  ┌─ src/main.wowlua:3:8
  │
3 │ @event("PLAYER_ENTERNG_WORLD")
  │        ^^^^^^^^^^^^^^^^^^^^^^ not declared
```

## Decorators from the standard library

The `wowlua:*` modules export decorators for the things addons do all the
time. Import them like any other name:

```wowlua title=src/commands.wowlua
import onLogin from "wowlua:lifecycle"
import event from "wowlua:events"
import slash from "wowlua:slash"
import throttle from "wowlua:timer"
import outOfCombat from "wowlua:combat"

@onLogin
local function greet()
    print("Type /mya for options")
end

@event("PLAYER_TARGET_CHANGED")
@throttle(1)                     -- at most once a second
local function targetChanged()
    print("target:", UnitName("target") or "none")
end

@outOfCombat                     -- in combat, waits until combat ends
local function applyLayout()
    UIParent:SetScale(1)
end

@slash("/mya", "/myaddon")
local function command(msg: string)
    if msg == "reset" then
        applyLayout()
    end
end
```

They come in two kinds:

- **Registering** decorators hand the function to the game or to wowlua's
  runtime: `@event`, `@unitEvent`, `@bucket`, `@onLoad`, `@onLogin`,
  `@onEnterWorld`, `@onLogout`, `@slash`, `@hook`, `@hookScript` and
  `@binding`.
- **Wrapping** decorators replace the function with one that changes when or
  how often it runs: `@throttle`, `@debounce`, `@nextFrame` and
  `@outOfCombat`.

Each is shown in use on its standard library page:

- [Events and lifecycle](../stdlib/events.md): `@event`, `@unitEvent`,
  `@bucket`, `@onLoad`, `@onLogin`, `@onEnterWorld`, `@onLogout`
- [Slash commands](../stdlib/slash.md): `@slash`
- [Timers and background jobs](../stdlib/timers.md): `@throttle`, `@debounce`,
  `@nextFrame`
- [Combat, secure code, hooks](../stdlib/combat.md): `@outOfCombat`, `@hook`,
  `@hookScript`
- [Settings, minimap, keybindings](../stdlib/settings.md): `@binding`

## Built-in decorators

Some decorators are part of the language and need no import. Most of them
change what the compiler emits, so in a build where they are switched off
nothing of them is left.

```wowlua title=src/ledger.wowlua
import flavor from "wowlua:meta"

local entries: string[] = {}

@profile                       -- measure calls and time
local function rebuild()
    table.sort(entries)
end

@trace                         -- log arguments and results at trace level
local function parse(msg: string): string?
    return msg:match("^(%S+)")
end

@debugOnly                     -- gone from release builds, with every call to it
local function checkEntries()
    for _, entry in ipairs(entries) do
        assert(type(entry) == "string")
    end
end

@global("MyAddon_OnClick")     -- also a global, for XML and key bindings
local function onClick(button: Button)
    print(button:GetName())
end

@if(flavor == "standard")      -- only built for the retail client
local function addEditModeOptions()
    print("Edit Mode options added")
end

@deprecated("use rebuild()")
local function refresh()
    rebuild()
end

table.insert(entries, parse("hello world") ?? "")
checkEntries()
rebuild()
```

- `@profile` and `@trace` are on [Debugging helpers](debugging.md). A
  profile's `instrument` setting decides whether `@profile` functions are
  measured; the dev profile measures every function anyway. `@trace` is
  removed when the profile's log level is above `trace`.
- `@debugOnly` works on functions that return nothing, called as statements.
  In a release build the function and those calls are removed.
- `@if` takes a condition the compiler can decide, mostly values from
  `wowlua:meta`. See [Build info and profiles](flavors.md).
- `@global` is the way to make a global. The global gets the decorated value,
  and other modules of the addon, and addons that depend on it, see it with
  its type.
- `@deprecated` gives a warning (W3030) everywhere the function is still
  used, and in debug builds a message in the chat the first time it is
  called.
- Also built in: `@inline` for small local functions whose body is a single
  `return` (inlined in optimized builds), `@reflect` for full reflection data,
  `@noprofile` to keep a function out of the dev build's profiling,
  `@checked` and `@unchecked` to force or skip runtime parameter checks,
  `@nocheck` to skip type checking a body, and `@script` for
  [frame classes](widgets.md).

The complete list is in
[Decorators §4.3](../../spec/language/04-decorators.md#43-core-decorators).

## Stacking decorators

A declaration can have any number of decorators. The ones nearest the
function apply first, and registering decorators get the finished function:

```wowlua title=src/bags.wowlua
import event from "wowlua:events"
import debounce from "wowlua:timer"

@event("BAG_UPDATE_DELAYED")   -- 3. registers the debounced function
@debounce(0.2)                 -- 2. wraps the profiled function
@profile                       -- 1. measures the function as written
local function rescanBags()
    print(C_Container.GetContainerNumFreeSlots(0))
end
```

Where you write a decorator matters less than it looks:

1. `@if` is decided first. When its condition is false, the declaration and
   all its other decorators are gone.
2. Built-ins that transform the function (`@profile`, `@trace`, `@inline`,
   `@checked`, `@unchecked`, `@nocheck`, `@debugOnly`) apply directly to the
   function as written, wherever they stand.
3. Your own decorators and the wrapping ones apply bottom to top. Their
   arguments are evaluated top to bottom first.
4. Registering decorators, `@global` and `export` receive the final value.

## Writing your own

Any function that takes a function and a `DecoratorContext` and returns a
function can be a decorator. It runs once, when the file loads. This one, in
the spirit of the widget demo's, announces every call in the chat:

```wowlua title=src/announce.wowlua
import addon from "wowlua:meta"

local function announced(fn: () -> (), ctx: DecoratorContext): () -> ()
    return function()
        print(`{addon.title}: {ctx.name}`)
        fn()
    end
end

@announced
local function toggleOptions()
    print("options")
end

toggleOptions()
```

The context says what the decorator is attached to:

| Field | Value |
|-------|-------|
| `ctx.name` | The declaration's name |
| `ctx.kind` | `"function"`, `"method"`, `"constructor"`, `"getter"`, `"setter"`, `"class"` or `"field"` |
| `ctx.file`, `ctx.line` | Where it is, such as `"src/announce.wowlua"` |
| `ctx.class` | For class members, the class table |
| `ctx.static` | For methods, accessors and fields, whether they are static |

Return `nil` to keep the original function.

Decorators work on classes and class members too. A class decorator runs
after the class is complete, with its static fields set, and may return a
replacement class. A field has no value yet when its decorator runs, so a
field decorator gets `nil` and its result is ignored. Use it to register the
field or record metadata about it.

### Decorators that take arguments

A decorator with arguments is a function that returns a decorator.
`@cached(64)` calls `cached(64)` and applies what it returns.
`FunctionDecorator<F>` is the type of a decorator for functions of type `F`:

```wowlua title=src/items.wowlua
export function memoize<A, R>(fn: (A) -> R, _ctx: DecoratorContext): (A) -> R
    local cache = {}
    return function(a: A): R
        local v = cache[a]
        if v == nil then
            v = fn(a)
            cache[a] = v
        end
        return v
    end
end

-- Keeps at most `size` results, then starts over.
local function cached(size: number): FunctionDecorator<(number) -> string>
    return function(fn, _ctx)
        local cache: { [number]: string } = {}
        local count = 0
        return function(id: number): string
            local v = cache[id]
            if v == nil then
                if count >= size then
                    cache, count = {}, 0
                end
                v = fn(id)
                cache[id] = v
                count += 1
            end
            return v
        end
    end
end

@memoize
local function spellName(id: number): string
    return C_Spell.GetSpellName(id) ?? "?"
end

@cached(64)
local function itemName(id: number): string
    return C_Item.GetItemNameByID(id) ?? "?"
end

print(spellName(6673), itemName(6948))
```

A decorated `local function` calls itself through the decorated value, so
`@memoize` caches the recursive calls of a function too.

## What it compiles to

Your own decorators become plain function calls right after the function, on
the line of its `end`, so line numbers in errors still match your source. In
`src/lookup.wowlua`, with `memoize` and `logCalls` defined above it:

```wowlua fragment
@memoize
@logCalls("cache")
local function lookup(id: number): string
    local name = C_Item.GetItemNameByID(id)
    return name or "?"
end
```

The dev build:

```lua
local lookup lookup = function(id)
	local name = C_Item.GetItemNameByID(id)
	return name or "?"
end lookup = __wl.prof.wrap(lookup, "lookup", "src/lookup.wowlua:11") lookup = __wl.decorate(lookup, {kind = "function", name = "lookup", static = false, file = "src/lookup.wowlua", line = 14}, memoize, logCalls("cache"))
```

- `local function` becomes `local lookup; lookup = function`, which is why
  recursive calls see the decorated value.
- `__wl.prof.wrap` is the dev build's profiler, which wraps every function.
  A release build has none of it.
- `__wl.decorate` gets one context table and the decorators, evaluated top to
  bottom, and applies them bottom to top. A decorator that returns `nil`
  leaves the value as it was.

Registering decorators come last. The stack from
[Stacking decorators](#stacking-decorators), in the dev build:

```lua
local rescanBags rescanBags = function()
	print(C_Container.GetContainerNumFreeSlots(0))
end rescanBags = __wl.prof.wrap(rescanBags, "rescanBags", "src/bags.wowlua:7") rescanBags = __wl.decorate(rescanBags, {kind = "function", name = "rescanBags", static = false, file = "src/bags.wowlua", line = 9}, debounce(0.2)) __wl.register(rescanBags, {kind = "function", name = "rescanBags", static = false, file = "src/bags.wowlua", line = 9}, event("BAG_UPDATE_DELAYED"))
```

> [!NOTE]
> The name of a built-in decorator always means the built-in. A function of
> yours called `profile` can still be a decorator under another name:
> `import profile as myProfile from "./perf"`, then `@myProfile`. Writing
> `@profile` while such a function is in scope gives warning W3021. Decorators
> never pick up globals by accident: a name that is not a built-in, a local or
> an import is error E3020. To use a decorator another addon exposes as a
> global, write the path from `_G`: `@_G.MyLib.memoize`.

## Where to go next

- [Modules](modules.md): `import` and `export`, and how files find each other
- [Events and lifecycle](../stdlib/events.md): every event decorator in use
- [Debugging helpers](debugging.md): `@profile`, `@trace` and `@debugOnly`
