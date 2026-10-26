# Classes

A class bundles a table's fields with the functions that work on them, such
as a cooldown timer or a gold ledger. wowlua checks every field and method
you use, and compiles the class to a plain Lua table with a metatable.

## Your first class

A class lists its fields with their types, a constructor that fills them, and
its methods. Methods get `self` without declaring it.

```wowlua title=src/cooldown.wowlua
export class CooldownTimer
    spellId: number
    label: string
    startedAt: number = 0
    duration: number = 0

    constructor(spellId: number, label: string)
        self.spellId = spellId
        self.label = label
    end

    function start(duration: number)
        self.startedAt = GetTime()
        self.duration = duration
    end

    function remaining(): number
        return math.max(0, self.startedAt + self.duration - GetTime())
    end
end
```

`CooldownTimer.new(...)` creates an instance, and so does calling the class
itself. Methods are called with `:`, as in Lua:

```wowlua title=src/main.wowlua
import CooldownTimer from "./cooldown"

local lust = CooldownTimer.new(2825, "Bloodlust")
local rebirth = CooldownTimer(20484, "Rebirth")    -- the same as .new

lust:start(600)
rebirth:start(600)
print(`{lust.label}: {lust:remaining()} seconds left`)
```

- A field with a value (`startedAt: number = 0`) is set on each new instance,
  before the constructor body runs.
- Classes are sealed. A method or field the class doesn't declare, such as
  `lust:strat(600)`, is error E2003. So is a method defined outside the class
  body (`function CooldownTimer:extra()`).
- Classes are declared at the top level of a file. `export` makes a class
  importable from other files ([Modules](modules.md)).
- Calls into the game, like `GetTime()`, are checked against the typed
  [WoW API](../reference/api.md) in the same way.

## Fields from constructor parameters

Many constructors only copy their arguments into fields. A constructor
parameter that starts with `public`, `protected`, `private` or `readonly`
declares a field of the same name and type, and the constructor assigns it
for you. A default value works as for any parameter.

```wowlua title=src/announcer.wowlua
type Channel = "RAID" | "PARTY" | "INSTANCE_CHAT"

export class Announcer
    sent: number = 0

    constructor(private channel: Channel = "RAID", public readonly prefix: string = "[Raid]") end

    function say(text: string)
        SendChatMessage(`{self.prefix} {text}`, self.channel)
        self.sent += 1
    end
end

local raid = Announcer.new()
local party = Announcer.new("PARTY", "[Group]")
raid:say("Pull in 10")
party:say("Summons are up")
```

The constructor sets these fields first, then the other field values:

```lua
function Announcer:__init(channel, prefix)
	if channel == nil then
		channel = "RAID"
	end
	if prefix == nil then
		prefix = "[Raid]"
	end
	self.channel = channel
	self.prefix = prefix
	self.sent = 0
end
```

A field with a default is always set, so `raid.prefix` is a `string`, not a
`string?`.

## Private and read-only members

Modifiers say who may use a member:

| Modifier | Who can use it |
|----------|----------------|
| `public` | Everyone. This is the default. |
| `protected` | The class and its subclasses. |
| `private` | The class body only, on any instance of the class. |
| `readonly` | Everyone can read it. Only the field's initializer and the class's own constructor assign it. |

A ledger that only changes through its methods:

```wowlua title=src/ledger.wowlua
export class Ledger
    readonly character: string    -- set once, in the constructor
    private balance: number       -- only Ledger's own code
    protected earned: number = 0  -- Ledger and its subclasses

    constructor(character: string, balance: number)
        self.character = character
        self.balance = balance
    end

    function update(balance: number)
        local delta = balance - self.balance
        if delta > 0 then
            self.earned += delta
        end
        self.balance = balance
    end
end

local ledger = Ledger.new("Thrall", GetMoney())
ledger:update(GetMoney())
```

Reading the balance from outside the class is an error:

```wowlua fragment
print(ledger.balance)
```

```text
error[E2011]: `balance` is private
   ┌─ src/ledger.wowlua:22:14
   │
22 │ print(ledger.balance)
   │              ^^^^^^^ here
```

Assigning `ledger.character` outside the constructor is error E2010.

> [!NOTE]
> `public`, `protected`, `private` and `readonly` are checked when you build
> and produce no code. Plain Lua that gets hold of a `Ledger` can still read
> and write all of its fields.

## Accessors

A `get` accessor runs a function when a name is read, and a `set` accessor
when it is assigned. Callers write `session.net`, not `session:net()`.

```wowlua title=src/session.wowlua
export class Session
    private earned: number = 0
    private spent: number = 0
    private scale: number = 1

    get net(): number
        return self.earned - self.spent
    end

    get windowScale(): number
        return self.scale
    end

    set windowScale(value: number)
        self.scale = math.min(math.max(value, 0.5), 2)
    end
end

local session = Session.new()
session.windowScale = 5       -- stored as 2
print(session.net, session.windowScale)
```

A getter without a setter is read-only: `session.net = 0` is error E2010.
Accessors can be `static`, `private` or `protected`, and can `override` an
accessor of the superclass.

Only a class with accessors, and its subclasses, get the `__index` and
`__newindex` functions that route these names. Every other class keeps a
plain table lookup, so its fields and methods stay as fast as in hand-written
Lua.

## Static members

A `static` field or function belongs to the class table, not to each
instance. It suits counters, caches and factory functions. Static functions
have no `self` and are called with `.`.

```wowlua
export class SpellTimer
    static bySpell: { [number]: SpellTimer } = {}
    spellId: number

    constructor(spellId: number)
        self.spellId = spellId
    end

    -- One timer per spell, created on first use.
    static function forSpell(spellId: number): SpellTimer
        local timer = SpellTimer.bySpell[spellId]
        if not timer then
            timer = SpellTimer.new(spellId)
            SpellTimer.bySpell[spellId] = timer
        end
        return timer
    end
end

local lust = SpellTimer.forSpell(2825)
print(lust.spellId)
```

Static field values are set once, after the class's methods are defined, so
they can call static functions.

## Inheritance

A class can extend one other class. The constructor calls `super(...)` before
it uses `self`, a method that replaces one of the parent's is marked
`override`, and `super:method(...)` calls the parent's version.

```wowlua title=src/charges.wowlua
import CooldownTimer from "./cooldown"

export class ChargeTimer extends CooldownTimer
    charges: number = 0
    maxCharges: number

    constructor(spellId: number, label: string, maxCharges: number)
        super(spellId, label)
        self.maxCharges = maxCharges
        self.charges = maxCharges
    end

    override function start(duration: number)
        self.charges -= 1
        super:start(duration)
    end
end
```

`super` calls compile to direct calls on the parent class table, with no
lookup at run time:

```lua
ChargeTimer = __wl.class("ChargeTimer", CooldownTimer)

function ChargeTimer:__init(spellId, label, maxCharges)
	CooldownTimer.__init(self, spellId, label)
	self.charges = 0

	self.maxCharges = maxCharges
	self.charges = maxCharges
end

function ChargeTimer:start(duration)
	self.charges = self.charges - 1
	CooldownTimer.start(self, duration)
end
__wl.finalize(ChargeTimer)
```

- The subclass's field values are set right after `super(...)`, then the rest
  of the constructor runs.
- A `ChargeTimer` can be used wherever a `CooldownTimer` is expected. Static
  members and metamethods are inherited too.
- A subclass without a constructor takes the parent constructor's parameters
  and passes them on.

Leaving out `override` is an error, and so is using `self` before
`super(...)` (E3001):

```text
error[E3005]: `start` overrides a member of a superclass; mark it `override`
   ┌─ src/charges.wowlua:13:14
   │
13 │     function start(duration: number)
   │              ^^^^^ missing `override`
   │
   ┌─ src/cooldown.wowlua:12:14
   │
12 │     function start(duration: number)
   │              ----- overridden member
```

## Abstract classes

An abstract class is a base that is never created itself. Its `abstract`
methods have no body, and every subclass must implement them. Here each
tracker computes its own value, and all of them share `describe`:

```wowlua
abstract class Tracker
    abstract function label(): string
    abstract function value(): number

    function describe(): string
        return `{self:label()}: {self:value()}`
    end
end

class BagSpace extends Tracker
    override function label(): string
        return "Free slots"
    end

    override function value(): number
        local free = 0
        for bag = 0, 4 do
            free += C_Container.GetContainerNumFreeSlots(bag)
        end
        return free
    end
end

print(BagSpace.new():describe())
```

`Tracker.new()` is error E3002, and a subclass that leaves out an abstract
method is error E3003. In debug builds, an abstract method that is called
anyway raises an error that names it, instead of a call to `nil`.

## Generic classes

A generic class takes type parameters, so one ring buffer can hold the last
loot drops or the last combat events, each with its own element type. This
one is from the [typed demo addon](../../examples/typed-demo):

```wowlua title=src/ring.wowlua
-- A fixed-size buffer that keeps the newest values.
export class Ring<T>
    private values: T[] = {}
    private first: number = 1
    private count: number = 0
    readonly capacity: number

    constructor(capacity: number)
        self.capacity = capacity
    end

    function push(value: T)
        local index = (self.first + self.count - 1) % self.capacity + 1
        self.values[index] = value
        if self.count < self.capacity then
            self.count += 1
        else
            self.first = self.first % self.capacity + 1
        end
    end
end

local drops: Ring<string> = Ring.new(20)
drops:push("Hearthstone")
```

Type arguments come from the constructor's arguments or, as here, from the
declared type. Pushing a value of another type is an error:

```wowlua fragment
drops:push(42)
```

```text
error[E2001]: `42` is not assignable to `string`
   ┌─ src/ring.wowlua:25:12
   │
25 │ drops:push(42)
   │            ^^ here
```

Type parameters exist only while checking. A generic class compiles to the
same Lua as any other class.

## Overloaded methods

When a method can be called in several ways, list each form with `overload`,
then write one implementation that accepts all of them. Callers see only the
overload signatures.

```wowlua
class BagIndex
    private byId: { [number]: number } = {}
    private byName: { [string]: number } = {}

    overload function count(itemId: number): number
    overload function count(name: string): number
    function count(key: number | string): number
        if type(key) == "number" then
            return self.byId[key] or 0
        end
        return self.byName[key] or 0
    end
end

local index = BagIndex.new()
print(index:count(6948), index:count("Hearthstone"))
```

A call that matches no overload is error E2012, and an implementation that
doesn't accept every overload is error E3044. Overloads produce no code:
one function is emitted. Constructors can be overloaded the same way.

## Metamethods

A method named after a metamethod becomes that metamethod of the instances.
The allowed names are `__tostring`, `__eq`, `__lt`, `__le`, `__add`, `__sub`,
`__mul`, `__div`, `__mod`, `__pow`, `__unm`, `__concat` and `__call`.

```wowlua
class Money
    copper: number

    constructor(copper: number)
        self.copper = copper
    end

    function __add(other: Money): Money
        return Money.new(self.copper + other.copper)
    end

    function __tostring(): string
        return GetCoinText(self.copper)
    end
end

local total = Money.new(150) + Money.new(250)
print(tostring(total))
```

Subclasses inherit them. Names the runtime uses itself, such as `new`,
`__index` and `__init`, are reserved (E3004).

## Using classes from plain Lua

An instance is an ordinary table, and the class is a table whose methods take
`self` first. Plain Lua uses them the usual way: `timer:remaining()`,
`timer.label`. A `.lua` file of the same addon reaches a module's exports
through the addon table:

```lua title=src/report.lua
local wowlua = select(2, ...).__wowlua
local CooldownTimer = wowlua.require("cooldown").CooldownTimer

local hearth = CooldownTimer.new(8690, "Hearthstone")
hearth:start(900)
print(hearth.label, hearth:remaining())
```

To hand a class to other addons, expose it as a global with `@global`
([Decorators](decorators.md)). [Modules](modules.md) has the details of mixing
Lua and wowlua files.

> [!TIP]
> Building a frame? A class can extend `Frame`, `Button` or another widget
> type, and `new` creates the frame with `CreateFrame`. See
> [Frames as classes](widgets.md).

The full rules are in [Classes, interfaces and
enums](../../spec/language/03-classes.md).
