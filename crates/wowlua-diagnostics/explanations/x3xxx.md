# x3xxx · classes

## E3001

A derived class's constructor calls `super(...)` wrongly. It must call it
exactly once, as a statement at the top level of the constructor body, before
anything uses `self`, the arguments of `super(...)` included: until then the
instance isn't built (for a widget class, the frame doesn't exist yet). The
code is also reported when a class without a superclass calls `super(...)`,
and when a derived class's constructor doesn't call it at all.

```wowlua fails
class Animal
    name: string
    constructor(name: string)
        self.name = name
    end
end
class Dog extends Animal
    constructor(name: string)
        self.name = name
        super(name)
    end
end
print(Dog.new("Rex"))
```

```wowlua compiles
class Animal
    name: string
    constructor(name: string)
        self.name = name
    end
end
class Dog extends Animal
    tricks: number = 0
    constructor(name: string)
        super(name)
        self.tricks = 1
    end
end
print(Dog.new("Rex"))
```

Related: E3040

## E3002

An `abstract` class is instantiated, directly or through a `ClassOf<T>` whose
`T` is abstract. An abstract class only exists to be extended: its abstract
methods have no body. Create an instance of a concrete subclass instead.

```wowlua fails
abstract class Shape
    abstract function area(): number
end
print(Shape.new())
```

```wowlua compiles
abstract class Shape
    abstract function area(): number
end
class Square extends Shape
    side: number = 2
    override function area(): number
        return self.side * self.side
    end
end
print(Square.new():area())
```

Related: E3003

## E3003

A concrete class doesn't implement an abstract method it inherits. Implement
it with `override function`, or make the class `abstract` too. The code is
also reported for an `abstract` method in a class that isn't marked
`abstract`. In debug builds an unimplemented abstract method is a stub that
raises an error when called.

```wowlua fails
abstract class Shape
    abstract function area(): number
end
class Square extends Shape
    side: number = 2
end
print(Square.new())
```

```wowlua compiles
abstract class Shape
    abstract function area(): number
end
class Square extends Shape
    side: number = 2
    override function area(): number
        return self.side * self.side
    end
end
print(Square.new())
```

Related: E3002, E3005

## E3004

A class member uses a name the class runtime keeps for itself: `new`,
`__index`, `__newindex`, `__name`, `__super`, `__init`, `__ctor`,
`__initFields`, `__class`, `__meta`, `__getters` or `__setters`. Defining one
would break construction or member lookup. Rename the member; to set up a new
instance, write a `constructor`.

```wowlua fails
class Item
    function new()
    end
end
print(Item)
```

```wowlua compiles
class Item
    constructor()
    end
end
print(Item.new())
```

Related: E3007

## E3005

A problem with `override`: a method or accessor replaces one a superclass
declares (or implements an abstract one) without `override`, or it is marked
`override` but no superclass declares it. The keyword makes replacing a
method a visible choice and catches a misspelt name. Add or remove `override`,
or fix the name.

```wowlua fails
class Animal
    function speak(): string
        return "..."
    end
end
class Dog extends Animal
    function speak(): string
        return "woof"
    end
end
print(Dog.new():speak())
```

```wowlua compiles
class Animal
    function speak(): string
        return "..."
    end
end
class Dog extends Animal
    override function speak(): string
        return "woof"
    end
end
print(Dog.new():speak())
```

Related: E3003

## E3006

A class says `implements I` but doesn't satisfy the interface `I`: a member
is missing, or its type doesn't fit. Add the member or fix its type.

Only public members count, as where an instance is used as the interface: a
`private` or `protected` member doesn't satisfy it. Members the class
inherits do, and so do those of the interfaces the interface extends.

```wowlua fails
interface Pet
    name: string
    function play(): boolean
end
class Dog implements Pet
    name: string = "Rex"
end
print(Dog)
```

```wowlua compiles
interface Pet
    name: string
    function play(): boolean
end
class Dog implements Pet
    name: string = "Rex"
    function play(): boolean
        return true
    end
end
print(Dog)
```

Related: E2001

## E3007

A widget class (a class that extends a frame type such as `Frame` or `Button`)
defines a metamethod such as `__tostring` or `__eq`. The frame's metatable
belongs to WoW, so a widget class can't have metamethods. Use an ordinary
method instead.

```wowlua fails
class Tile extends Frame
    function __tostring(): string
        return "tile"
    end
end
print(Tile)
```

```wowlua compiles
class Tile extends Frame
    function describe(): string
        return "tile"
    end
end
print(Tile)
```

Related: E3043, E3004

## E3008

A class extends something that isn't a class: an interface, a type alias,
an enum, or a value such as a table. `extends` needs a class value to inherit
from at run time. To take an interface's shape, write `implements` instead
(`class Dog implements Pet`). A class that `import type` brings is only a
type, with nothing to extend at run time: import it as a value
(`import Animal from "./animal"`).

```wowlua fails
interface Pet
    name: string
end
class Dog extends Pet
    name: string = "Rex"
end
print(Dog)
```

```wowlua compiles
interface Pet
    name: string
end
class Dog implements Pet
    name: string = "Rex"
end
print(Dog)
```

```wowlua fails title=src/main.wowlua
import type Animal from "./animal"
class Dog extends Animal
end
print(Dog)
```

```wowlua fails title=src/animal.wowlua
export class Animal
end
```

```wowlua compiles title=src/main.wowlua
import Animal from "./animal"
class Dog extends Animal
end
print(Dog)
```

```wowlua compiles title=src/animal.wowlua
export class Animal
end
```

## E3010

An enum's values are mixed or not constant. An enum is either all numbers or
all strings; members without a value are numbered after the previous one, so
in a string enum every member needs its own value. Values must be number or
string constants.

```wowlua fails
enum Size { Small = 1, Large = "large" }
print(Size.Small)
```

```wowlua compiles
enum Size { Small = "small", Large = "large" }
print(Size.Small)
```

Related: E3011

## E3011

Two members of an enum have the same value, or the same name. Members without
a value count on from the previous one, so a later explicit value can clash
with one counted earlier. Give each member its own value.

```wowlua fails
enum Quality { Poor = 0, Common, Junk = 1 }
print(Quality.Poor)
```

```wowlua compiles
enum Quality { Poor = 0, Common, Junk = 2 }
print(Quality.Poor)
```

Related: E3010

## E3020

A decorator name is neither a core decorator nor a local or imported binding.
Decorators never resolve to globals by themselves: import the decorator, or
define it as a local function before its use, or write `@_G.Name` to look it
up in the global environment at load time. Framework decorators such as
`@event` need their import (`import event from "wowlua:events"`).

```wowlua fails
@memoize
local function lookup(id: number): number
    return id * 2
end
print(lookup(1))
```

```wowlua compiles
local function memoize<A, R>(fn: (A) -> R, ctx: DecoratorContext): (A) -> R
    print(ctx.name)
    return fn
end

@memoize
local function lookup(id: number): number
    return id * 2
end
print(lookup(1))
```

Related: E3021

## E3021

A decorator is used on a declaration it can't apply to. User decorators work
on functions, classes and class members; on enums, interfaces, type aliases,
`declare` statements and imports they can't take effect (enum members are
inlined, types have no value at run time). Each core decorator has its own
targets: `@script` only on instance methods of widget classes, `@inline` and
`@debugOnly` on functions and methods, and so on. The code also covers a
`@script` without a script name or with options other than `{ hook = true }`
or `{ hook = false }`.

```wowlua fails
@script("OnClick")
local function clicked()
end
clicked()
```

```wowlua compiles
class Clicker extends Button
    @script("OnClick")
    function clicked()
        print("clicked")
    end
end
print(Clicker)
```

Related: E3020, E0109

## E3022

A user decorator's parameters don't accept the declaration it decorates: a
function decorator is `(fn: F, ctx: DecoratorContext) -> F?`, and its `fn`
must take the decorated function's type. Fix the decorator's signature, or
decorate a fitting declaration.

The checker doesn't report this code yet.

Related: E3021

## E3032

A `@debugOnly` function is used in a way that can't be removed. Release
builds remove the function and every call statement to it, so it can't return
values and its calls can't be used as values (`local x = report(1)`). Make it
return nothing, and call it on its own line. A call used as a value is
reported when checking or building without `debug` (`wowlua check --release`).

```wowlua fails
@debugOnly
local function check(n: number): boolean
    return n > 0
end
check(1)
```

```wowlua compiles
@debugOnly
local function report(n: number)
    print(n)
end
report(1)
```

Related: E3045

## E3033

`@intrinsic` binds a declaration to behaviour built into the compiler, and is
only for the compiler-known modules of the standard library. Remove it from
your declarations. The same code with the message "unknown intrinsic" means
the standard library asks for a newer compiler: update wowlua.

```wowlua fails
@intrinsic("events.event")
declare function event(name: string): any
```

```wowlua compiles
declare function event(name: string): any
```

## E3040

A local or parameter named `super` inside a class body. There `super` means
the superclass (`super(...)`, `super:speak()`), so a local of that name would
hide it. Rename the local.

```wowlua fails
class Animal
    function speak()
        local super = 1
        print(super)
    end
end
print(Animal)
```

```wowlua compiles
class Animal
    function speak()
        local base = 1
        print(base)
    end
end
print(Animal)
```

Related: E3001

## E3041

`...` in a field or static initializer. Field initializers run in the
generated `__init`, not in the constructor's body, so the constructor's
`...` isn't there. Assign the field in the constructor instead.

```wowlua fails
class Bag
    items: any[] = { ... }
    constructor(...: any)
    end
end
print(Bag)
```

```wowlua compiles
class Bag
    items: any[]
    constructor(...: any)
        self.items = { ... }
    end
end
print(Bag)
```

## E3042

A field and a method (or accessor) with the same name, in one class or across
a class and its superclass. An instance has one value per name, so the field
would hide the method. Rename one of them.

```wowlua fails
class Unit
    function label(): string
        return "x"
    end
end
class Player extends Unit
    label: string = "y"
end
print(Player)
```

```wowlua compiles
class Unit
    function label(): string
        return "x"
    end
end
class Player extends Unit
    title: string = "y"
end
print(Player)
```

Related: E3046

## E3043

A widget class declares an accessor (`get name()` or `set name()`). Accessors
need the class's own `__index` and `__newindex`, but a frame's metatable
belongs to WoW. Use methods (`getSize()`, `setSize(v)`) instead.

```wowlua fails
class Tile extends Frame
    get size(): number
        return 1
    end
end
print(Tile)
```

```wowlua compiles
class Tile extends Frame
    function getSize(): number
        return 1
    end
end
print(Tile)
```

Related: E3007

## E3044

The implementation after `overload function` signatures doesn't accept one of
them: an argument that overload takes doesn't fit the implementation's
parameters, or the results don't fit either way. Callers only see the
overloads, so the implementation must handle all of them. Widen the
implementation's parameters, often to a union.

```wowlua fails
overload function size(s: string): number
overload function size(t: table): number
local function size(v: string): number
    return #v
end
print(size("a"))
```

```wowlua compiles
overload function size(s: string): number
overload function size(t: table): number
local function size(v: string | table): number
    return #v
end
print(size("a"))
```

Related: E3047, E2012

## E3045

A statement-only helper of `wowlua:debug` (`assert`, `dump`, `time`,
`timeEnd`, `resetStats`, `watch`) is used as a value. Builds that turn the
helper off remove the call with its arguments, so there would be no value
left. Call it on its own line; for a value, use Lua's `assert` (`_G.assert`).

```wowlua fails
import assert from "wowlua:debug"
local ok = assert(true, "x")
print(ok)
```

```wowlua compiles
import assert from "wowlua:debug"
local ready = true
assert(ready, "not ready")
print(ready)
```

Related: E3032

## E3046

A field is declared twice in one class, also when a constructor's parameter
property (`constructor(public x: number)`) declares a field the class body
declares too. Keep one declaration.

```wowlua fails
class Point
    x: number
    constructor(public x: number)
    end
end
print(Point)
```

```wowlua compiles
class Point
    constructor(public x: number)
    end
end
print(Point.new(1).x)
```

Related: E3042

## E3047

An `overload function` signature isn't followed right away by its
implementation: a function of the same name (`function`, `local function` or
`export function`) must come directly after the signatures. Move the
implementation up, or add it.

```wowlua fails
overload function size(s: string): number
local count = 0
local function size(v: string): number
    return #v
end
print(size("a"), count)
```

```wowlua compiles
local count = 0
overload function size(s: string): number
local function size(v: string): number
    return #v
end
print(size("a"), count)
```

Related: E3044

## W3021

A local or imported function has a core decorator's name (`profile`, `trace`,
…) but isn't a re-export of that decorator. In decorator position a core
decorator's name always means the core decorator, so `@profile` ignores your
function. Import it under another name (`import profile as myProfile from
"./perf"`) and write `@myProfile`. Re-exports such as `import profile from
"wowlua:debug"` are fine.

The checker doesn't report this code yet.

Related: E3020

## W3030

The code uses a declaration marked `@deprecated`. The warning carries the
decorator's message, which usually names the replacement. In debug builds the
first use also prints a one-time warning in game. Switch to the replacement.

```wowlua fails
@deprecated("use newName")
local function oldName()
end
oldName()
```

```wowlua compiles
local function newName()
end
newName()
```

## W3031

`@inline` is ignored: the function can't be inlined. Only a `local function`
whose body is a single `return` expression is inlined, and only when it isn't
recursive and is used only by calling it; the warning says which rule failed.
A method with `@inline` is reported too. Simplify the function, or remove
the hint.

The warning comes from `wowlua build` with a profile that inlines
(`optimize.inline`, on in `release`), not from `wowlua check`.
