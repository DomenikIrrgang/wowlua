# x2xxx · types

## E2001

A value's type doesn't fit where it is used: an argument, an assignment to an
annotated local or field, or a return value. The same code covers an operator
used on a type it doesn't take, such as comparing a number with a string or
doing arithmetic on a boolean. Change the value, or widen the annotation if
the value is right.

In a plain Lua file (no wowlua syntax, no `--!strict`) the checker reports the
warning with the same number, W2001, so existing code keeps building.

```wowlua fails
local count: number = "3"
print(count)
```

```wowlua compiles
local count: number = 3
print(count)
```

Related: W2001, E2005

## E2002

A type annotation names a type that doesn't exist: a typo, a type declared in
another module that isn't imported, or a type only some flavors have. Check
the spelling, or bring the type in with `import type Name from "./module"`.

```wowlua fails
local f: Fram = CreateFrame("Frame")
print(f)
```

```wowlua compiles
local f: Frame = CreateFrame("Frame")
print(f)
```

Related: E2013

## E2003

The value has no field or method of that name. Classes are sealed, so every
member of a class instance must be declared in the class body, in every mode;
a misspelt name is the usual cause. Strict mode also reports a named field
read from an array, an unknown field in a table literal written for a record
type, and a name the `string` library doesn't have (`s:nope()`), all of which
Lua would read as `nil`.

The code has two more meanings. A method defined outside its class body
(`function Animal:speak() … end` after the class) is E2003: move it into the
class. And the key `__wowlua` is reserved for the wowlua runtime, so assigning
it on a frame or an instance is E2003 too.

```wowlua fails
class Animal
    name: string = "Rex"
end
local a = Animal.new()
print(a.nmae)
```

```wowlua fails
class Animal
    name: string = "Rex"
end
function Animal:speak()
    print(self.name)
end
```

```wowlua compiles
class Animal
    name: string = "Rex"
    function speak()
        print(self.name)
    end
end
local a = Animal.new()
print(a.name)
a:speak()
```

Related: W2003, E2051, E2011

## E2004

Something is called that isn't a function: a number, a string, a table
without `__call`, or an `unknown` that hasn't been narrowed. In game this is
the "attempt to call a … value" error. Usually a name holds something other
than you think, or a field's name is wrong; narrow an `unknown` with
`type(x) == "function"` before calling it.

In a plain Lua file it is the warning W2004.

```wowlua fails
local count: number = 3
count()
```

```wowlua compiles
local count: number = 3
print(count)
```

Related: W2004, E2008

## E2005

Strict mode only: the value may be `nil`, and it is used where `nil` doesn't
fit, such as calling a method on it, indexing it or passing it for a
parameter that doesn't take `nil`. In game that is the "attempt to index a nil
value" error. Check the value first (`if s then … end`, `assert(s)`, or an
early `return`), give a default with `??` or `or`, or make the target
optional (`T?`).

Basic mode lets `nil` into every type, so this code only appears in files
under `--!strict` or `check.mode = "strict"`.

```wowlua fails
--!strict
local function first(xs: string[]): string?
    return xs[1]
end
local s = first({ "a" })
print(s:upper())
```

```wowlua compiles
--!strict
local function first(xs: string[]): string?
    return xs[1]
end
local s = first({ "a" })
if s then
    print(s:upper())
end
```

Related: E2001

## E2006

A call passes more arguments than the function takes. Lua would drop the
extra ones silently, which usually hides a mistake: a wrong function, or
arguments in the wrong order. Remove the extra arguments, or add the
parameters (`...: T` for any number of them).

For a function with no annotations at all, basic mode reports the warning
W2055 instead; strict mode reports E2006 for every function.

```wowlua fails
local function add(a: number, b: number): number
    return a + b
end
print(add(1, 2, 3))
```

```wowlua compiles
local function add(a: number, b: number): number
    return a + b
end
print(add(1, 2))
```

Related: W2055, E2007, E2012

## E2007

A call leaves out an argument for a parameter that isn't optional: its type
has no `?` or `nil` and it has no default. Pass the value, or make the
parameter optional (`b: number?`, or a default `b: number = 0`) if leaving it
out is meant. In basic mode passing an explicit `nil` is accepted, but leaving
the argument out is still E2007.

```wowlua fails
local function add(a: number, b: number): number
    return a + b
end
print(add(1))
```

```wowlua compiles
local function add(a: number, b: number?): number
    return a + (b or 0)
end
print(add(1))
```

Related: E2006

## E2008

A method is called with `.` instead of `:`, so it gets no `self`, and in game
its first line that reads `self` raises an error. Call it with `:`
(`c:bump()`), or pass the receiver yourself (`c.bump(c)`). Static methods take
no `self` and are called with `.`.

```wowlua fails
class Counter
    n: number = 0
    function bump()
        self.n += 1
    end
end
local c = Counter.new()
c.bump()
```

```wowlua compiles
class Counter
    n: number = 0
    function bump()
        self.n += 1
    end
end
local c = Counter.new()
c:bump()
```

Related: E2004

## E2009

A function that declares results can reach its end without a `return`, so on
that path it gives `nil`. Add the missing `return`, often at the end of the
function, or include `nil` in the result type (`: number?`) if giving nothing
is meant.

```wowlua fails
local function sign(n: number): number
    if n > 0 then
        return 1
    elseif n < 0 then
        return -1
    end
end
print(sign(2))
```

```wowlua compiles
local function sign(n: number): number
    if n > 0 then
        return 1
    elseif n < 0 then
        return -1
    end
    return 0
end
print(sign(2))
```

Related: E2001

## E2010

The target can't be assigned: a `const`, a `readonly` field (outside its
initializer and its own class's constructor), an accessor with a getter but no
setter, an enum member, or a field of `wowlua:meta` (`build`, `flavor`,
`config` and the others, which the compiler folds into the code). Use a
`local` if the value has to change, add a setter, or move the assignment into
the constructor.

```wowlua fails
const MAX = 5
MAX = 6
```

```wowlua compiles
local max = 5
max = 6
print(max)
```

Related: E2011

## E2011

A `private` or `protected` member is used where it isn't visible. A `private`
member is visible only inside its class body (on any instance of the class);
a `protected` one also in subclasses. Use a public method instead, or make
the member public if outside code should reach it. The check is compile-time
only: nothing stops it at run time.

```wowlua fails
class Account
    private balance: number = 0
end
local a = Account.new()
print(a.balance)
```

```wowlua compiles
class Account
    private balance: number = 0
    function getBalance(): number
        return self.balance
    end
end
local a = Account.new()
print(a:getBalance())
```

Related: E2003

## E2012

The function has overloads, and none of them takes these arguments. The
message lists every overload; compare your arguments with them. Many WoW API
functions are declared with overloads, one per way Blizzard documents to call
them.

```wowlua fails
overload function size(s: string): number
overload function size(t: table): number
local function size(v: string | table): number
    return #v
end
print(size(5))
```

```wowlua compiles
overload function size(s: string): number
overload function size(t: table): number
local function size(v: string | table): number
    return #v
end
print(size("abc"))
```

Related: E2001, E2006, E2007

## E2013

A generic type is given the wrong number of type arguments: `Pair<A, B>`
written as `Pair<number>`. Give one argument per type parameter. Writing a
generic type without any arguments is allowed (they are `any`), which the
optional lint W2063 can report.

```wowlua fails
type Pair<A, B> = { first: A, second: B }
local p: Pair<number> = { first = 1, second = 2 }
print(p)
```

```wowlua compiles
type Pair<A, B> = { first: A, second: B }
local p: Pair<number, number> = { first = 1, second = 2 }
print(p)
```

Related: E2002, E2014

## E2014

A type parameter declared with `extends` was given a type that doesn't
satisfy its constraint. Type arguments are inferred from the call, so this
usually means an argument of the wrong type: a list of strings where items
with a `size` are needed, or a key that isn't one of the table's fields for
`K extends keyof T`.

```wowlua fails
local function largest<T extends { size: number }>(xs: T[]): T
    return xs[1]
end
print(largest({ "a", "b" }))
```

```wowlua compiles
local function largest<T extends { size: number }>(xs: T[]): T
    return xs[1]
end
print(largest({ { size = 1 } }))
```

Related: E2013, E2001

## E2015

A type assertion (`expr :: T`) between types that can't be the same value:
neither is assignable to the other, and they aren't both tables. An assertion
can narrow or widen a type, not turn a number into a string. Fix the type, or,
if you really know better, go through `any`: `expr :: any :: T`.

```wowlua fails
local n = 5
local s = n :: string
print(s)
```

```wowlua compiles
local n: number | string = "five"
local s = n :: string
print(s)
```

Related: E2001

## E2030

Shared flavor mode only (`"build": { "flavor-mode": "shared" }`): one build
runs on several flavors, and this API doesn't exist on all of them. On a
flavor without it, the name is `nil` in game, and using it raises an error.
Guard the use with a flavor check (`if flavor == "standard" then`, a
`WOW_PROJECT_ID` comparison) or an existence check (`if C_Garrison then`).

In a plain Lua file it is the warning W2030.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "standard": {}, "vanilla": {} },
    "build": { "flavor-mode": "shared" }
}
```

```wowlua fails title=src/main.wowlua
--!strict
C_Garrison.GetGarrisonInfo(1)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "standard": {}, "vanilla": {} },
    "build": { "flavor-mode": "shared" }
}
```

```wowlua compiles title=src/main.wowlua
--!strict
if C_Garrison then
    C_Garrison.GetGarrisonInfo(1)
end
```

Related: W2030

## E2040

Strict mode: the value may be a *secret value*, and this operation raises an
error on one. Since Midnight, many APIs give addon code secrets in combat,
encounters and Mythic+; arithmetic, comparison, `#`, indexing, calling, use as
a table key and testing a secret boolean all raise at once. Pass the value
straight to a widget that accepts secrets (`StatusBar:SetValue`,
`FontString:SetText`), or narrow it first with `canaccessvalue(v)` or
`not issecretvalue(v)`.

In basic mode and in plain Lua files it is the warning W2040.

```wowlua fails
--!strict
local hp = UnitHealth("target")
if hp < 1000 then
    print("low")
end
```

```wowlua compiles
--!strict
local hp = UnitHealth("target")
if canaccessvalue(hp) and hp < 1000 then
    print("low")
end
```

Related: W2040

## E2050

Strict mode: a parameter has no type annotation. Annotate it. Callbacks are
exempt: a function written directly as a call argument, or where a type is
expected, takes its parameter types from there.

```wowlua fails
--!strict
local function double(n)
    return n * 2
end
print(double(2))
```

```wowlua compiles
--!strict
local function double(n: number): number
    return n * 2
end
print(double(2))
```

Related: W2063

## E2051

Strict mode: a field is added to a sealed table, one whose type is fixed. A
table literal stays open (fields can be added) while it is a local in the
function that made it; it is sealed once it escapes (passed, returned, stored
or captured) and when its type comes from an annotation. Declare the field in
the type, write it in the literal, or give the type an indexer
(`[string]: V`). Basic mode allows it.

```wowlua fails
--!strict
local function make(): { x: number }
    return { x = 1 }
end
local p = make()
p.y = 2
```

```wowlua compiles
--!strict
local function make(): { x: number, y?: number }
    return { x = 1 }
end
local p = make()
p.y = 2
```

Related: E2003

## E2052

Strict mode: a class field that isn't optional and has no initializer isn't
assigned on every path through the constructor. It must be assigned before
the constructor returns, and before it calls one of the class's own methods
on `self`, which could read it (the message names that call). Assign it, give
it an initializer (`level: number = 1`), or make it optional (`level?:
number`).

```wowlua fails
--!strict
class Player
    name: string
    level: number
    constructor(name: string)
        self.name = name
    end
end
print(Player.new("a"))
```

```wowlua compiles
--!strict
class Player
    name: string
    level: number
    constructor(name: string)
        self.name = name
        self.level = 1
    end
end
print(Player.new("a"))
```

Related: E2005

## E2053

A type alias refers to itself outside a table or function type, so it never
becomes a type: `type A = A | nil`, or two aliases that name each other
(`type B = C`, `type C = B`). A recursive alias is fine inside a table or
function type: `type Tree = { value: number, children: Tree[] }`.

```wowlua fails
type Node = Node | nil
local n: Node = nil
print(n)
```

```wowlua compiles
type Node = { value: number, next: Node? }
local n: Node = { value = 1 }
print(n.value)
```

## E2054

Two `declare interface` declarations of the same name (such as
`AddonNamespace`) merge into one interface, and they declare the same field
with conflicting types. Give the field one type in every declaration, or
rename one of them.

```wowlua fails
declare interface AddonNamespace
    db: { scale: number }
end
declare interface AddonNamespace
    db: string
end
local _, ns = ...
print(ns.db)
```

```wowlua compiles
declare interface AddonNamespace
    db: { scale: number }
end
declare interface AddonNamespace
    version: string
end
local _, ns = ...
print(ns.db.scale, ns.version)
```

## E2056

A type pack (`T...`, a generic pack parameter) is used where only a single
type can stand. A pack belongs in a function type's parameters or results, as
a type argument for a pack parameter, or as the expansion `...: T...`.

```wowlua fails
local function first<T...>(...: T...): T
    return (...)
end
print(first(1, 2))
```

```wowlua compiles
local function all<T...>(...: T...): T...
    return ...
end
print(all(1, 2))
```

## E2058

A name is declared both as `declare event` (a WoW event) and as
`declare message` (an addon message). Events and messages share one set of
names, so one of them has to be renamed.

```wowlua fails
declare event MYADDON_READY()
declare message MYADDON_READY()
print("ready")
```

```wowlua compiles
declare event MYADDON_READY()
declare message MYADDON_SETTINGS_CHANGED(key: string)
print("ready")
```

## E2059

A parameter of `declare event` or `declare message` has a default value.
The parameters describe the payload the game or the sender gives, so a
default has no meaning there; remove it.

```wowlua fails
declare message MYADDON_CHANGED(key: string = "scale")
print("changed")
```

```wowlua compiles
declare message MYADDON_CHANGED(key: string)
print("changed")
```

## E2060

In `local a, b: T, U`, the annotation belongs to `b` only, so `U` is read as
a third local named after a type. Annotate each name on its own:
`local a: T, b: U`.

```wowlua fails
local x, y: number, number = 1, 2
print(x, y)
```

```wowlua compiles
local x: number, y: number = 1, 2
print(x, y)
```

Related: E2001

## W2040

The basic-mode warning for E2040: the value may be a secret value, and this
operation raises an error on one in game (arithmetic, comparison, `#`,
indexing, calling, use as a table key, testing a secret boolean). It is a
warning in basic mode and in plain Lua files, an error under `--!strict`.
Narrow the value with `canaccessvalue(v)` or `not issecretvalue(v)` first.

```wowlua fails
local hp = UnitHealth("target")
if hp < 1000 then
    print("low")
end
```

```wowlua compiles
local hp = UnitHealth("target")
if canaccessvalue(hp) and hp < 1000 then
    print("low")
end
```

Related: E2040

## W2055

Basic mode: a call passes more arguments than a function without any
annotations takes. Lua drops the extra ones. It is a warning because plain
Lua code often does this on purpose; in strict mode it is E2006. Remove the
extra arguments or add the parameters.

```wowlua fails
local function add(a, b)
    return a + b
end
print(add(1, 2, 3))
```

```wowlua compiles
local function add(a, b)
    return a + b
end
print(add(1, 2))
```

Related: E2006

## W2057

`coroutine.yield` inside a function passed straight to `pcall`, `xpcall` or a
C function that calls it back (a `table.sort` comparator, a `string.gsub`
callback). In WoW's Lua 5.1 a coroutine can't yield across those, and the
yield raises "attempt to yield across metamethod/C-call boundary". Move the
yield out of the callback, for example by collecting the work and yielding in
the loop around it.

```wowlua fails
local co = coroutine.create(function()
    pcall(function()
        coroutine.yield(1)
    end)
end)
coroutine.resume(co)
```

```wowlua compiles
local co = coroutine.create(function()
    pcall(function()
        print("work")
    end)
    coroutine.yield(1)
end)
coroutine.resume(co)
```

Related: W8430

## W2060

Division or modulo by a literal zero (or a `const` of it). Elsewhere Lua gives
inf or nan, but WoW raises "Division by zero". Use `math.huge` for infinity,
and guard a divisor that can be zero at run time (`if max > 0 then`); the
checker can't see those.

```wowlua fails
local function half(n: number): number
    return n / 0
end
print(half(1))
```

```wowlua compiles
local function half(n: number): number
    return n / 2
end
print(half(1))
```

## W2061

`a and nil or b` (or `false` in the middle) is always `b`: when `a` is truthy,
`a and nil` is `nil`, so the `or` takes over and gives `b` anyway. The idiom
can't give a falsy value. Write `not a and b or nil` when `b` is never falsy,
or an `if`.

```wowlua fails
local function gapFor(subtitle: string?): number?
    return subtitle and nil or 8
end
print(gapFor("x"))
```

```wowlua compiles
local function gapFor(subtitle: string?): number?
    return not subtitle and 8 or nil
end
print(gapFor("x"))
```

## W2062

A name reads past the values a function gives, so it is always `nil`. The
checker counts the values of the package's own functions whose every
`return` gives a fixed number of them (local functions, a module's functions,
imported ones, methods of a file-level table). It also reports a name past an
`and` or `or` that ends a `local` or an assignment, since those give one
value: in `local x, y = pos and pos:GetXY()`, `y` is always `nil`. Fix the
names, or write `if pos then x, y = pos:GetXY() end`. Names starting with
`_` are skipped.

```wowlua fails
local function pos()
    return 1, 2
end
local x, y, z = pos()
print(x, y, z)
```

```wowlua compiles
local function pos()
    return 1, 2
end
local x, y = pos()
print(x, y)
```

## W2063

A value is `any` without anyone writing `any`: a parameter nothing gives a
type, a `local` whose value is `any`, a generic type written without its type
arguments, or a member a frame or widget class doesn't declare. `any` turns
checking off, so mistakes there pass unseen. Annotate the value, or write
`: any` where you mean it.

The lint is off unless the manifest turns it on
(`"check": { "lints": { "W2063": "warn" } }`). Names starting with `_` are
exempt.

```json fails title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "check": { "lints": { "W2063": "warn" } }
}
```

```wowlua fails title=src/main.wowlua
local function show(value)
    print(value)
end
show(1)
```

```json compiles title=wowlua.json
{
    "package": { "name": "playground", "version": "0.1.0", "wowlua": "^0.12" },
    "flavors": { "camelot": {} },
    "check": { "lints": { "W2063": "warn" } }
}
```

```wowlua compiles title=src/main.wowlua
local function show(value: number)
    print(value)
end
show(1)
```

Related: E2050
