# 3. Classes, Interfaces and Enums

## 3.1 Classes

### 3.1.1 Syntax

```wowlua
[decorators]
[export] [abstract] class Name[<TypeParams>] [extends Base[<TypeArgs>]] [implements I1, I2, …]
    member*
end
```

Classes may only be declared at the top level of a file (E0115). Members:

```wowlua
class Animal
    -- fields
    name: string
    protected sound: string = "..."
    readonly id: number
    nickname?: string                       -- optional field (string?)
    static count: number = 0

    -- constructor
    constructor(name: string, id: number)
        self.name = name
        self.id = id
        Animal.count += 1
    end

    -- methods (self is implicit)
    function speak(): string
        return `{self.name} says {self.sound}`
    end

    static function create(name: string): Animal
        return Animal.new(name, Animal.count + 1)
    end

    -- accessors (§3.1.10)
    get label(): string
        return `{self.name} #{self.id}`
    end

    -- metamethods
    function __tostring(): string
        return `Animal({self.name})`
    end
end
```

| Modifier | Applies to | Meaning |
|----------|------------|---------|
| `public` | fields, methods, accessors | Default. Accessible everywhere. |
| `protected` | fields, methods, accessors | Accessible in the class and its subclasses. |
| `private` | fields, methods, accessors | Accessible only inside the class body. |
| `static` | fields, methods | Belongs to the class table, not to instances. |
| `readonly` | fields | Assignable only in its initializer or its own class's constructor (not a method, not a subclass's constructor). |
| `abstract` | methods (abstract classes only) | No body. Subclasses must implement it. |
| `override` | methods, accessors | Required when overriding a superclass member (implementing an abstract method included), and an error when nothing is overridden (E3005). |
| `overload` | method and constructor signatures | Declares an overload signature without a body (§3.1.11). |

Access modifiers and `readonly` are **compile-time only**. They produce no
runtime checks.

**Visibility details.**
- As in TypeScript, `private` members are accessible on **any** instance of
  the same class from inside the class body, not only on `self`.
- `protected` members are accessible from inside the declaring class and its
  subclasses, on instances of the accessing class or its subclasses.
- A field and a method (or accessor) with the same name in one class, or across
  a class and its superclass, is error E3042.

### 3.1.2 Instances and construction

`Class.new(args)` creates an instance. `Class(args)` is equivalent (the class
table has a `__call` metamethod). The checker treats both as constructor
calls. `.new` avoids the metamethod and is what the compiler emits for
internal construction.

Construction order:

1. a new table is created with the class as metatable,
2. **field initializers** run in declaration order (for a derived class: after
   the `super(...)` call, see below),
3. the constructor body runs.

A class without a constructor gets an implicit one. For a base class it takes
no parameters. For a derived class it forwards all arguments to the
superclass constructor and has the superclass constructor's parameter types.

**Varargs.** A constructor may be variadic (`constructor(...: string)`). Field
initializers run inside the generated `__init` but are not part of the
constructor's body, so `...` cannot be used in field initializers or static
initializers (E3041).

**Definite assignment.** In strict mode, every non-optional instance field
without an initializer must be assigned on every path through the constructor
before the constructor returns or calls a method of a wowlua class on `self`
(E2052). Calls of native methods (`self:SetSize(…)` in a widget class) do not
end the check, because they cannot read the fields. A field assigned in every
branch of an `if` that has an `else` is assigned after it, and so is one
assigned in a `do` block; a call or `return` that ends the check in any branch
ends it there.

**Parameter properties** (decision L35). A parameter of a constructor with
a body may start with `public`, `protected`, `private` and/or `readonly`.
It declares an instance field of the same name, type and modifiers, and the
constructor assigns the argument to it first: in a base class before the
field initializers, in a derived class right after `super(…)` (before the
derived class's field initializers). A parameter with a default gives a
field that is always set (not optional). Its own declaration as a field
too is E3046, as is any field declared twice. Overload signatures cannot
have them.

```wowlua
class Log
    lines: string[] = {}
    constructor(private sink: Sink = printer, public readonly name: string = "log") end
    function add(text: string) self.sink:say(`{self.name}: {text}`) end
end
```

### 3.1.3 Methods and `self`

Methods declared with `function name()` take an implicit first parameter
`self` typed as the class instance (or `Self` in generic contexts). They are
called with `:`: `dog:speak()`. Calling a method with `.` and no explicit
receiver is an error (E2008). Static methods have no `self` and are called with
`.`: `Animal.create("x")`.

Inside methods, `self.field` accesses fields. A field or method that is not
declared is an error on class instances (E2003) in both modes. Classes are
**sealed**: all members are declared in the class body. Defining a method
outside the body (`function Animal:extra() … end` at top level) is E2003, with
a quick fix that moves it into the class (decision L29).

### 3.1.4 Inheritance

```wowlua
export interface Pet
    name: string
    function play(withToy: string?): boolean
end

class Dog extends Animal implements Pet
    private static lastId: number = 100
    tricks: string[] = {}

    static function nextId(): number
        Dog.lastId += 1
        return Dog.lastId
    end

    constructor(name: string)
        super(name, Dog.nextId())       -- must come before any use of self
        self.sound = "woof"
    end

    override function speak(): string
        return super:speak() .. "!"
    end

    function play(withToy: string?): boolean
        return withToy ~= nil
    end
end
```

- Single inheritance. A class extends at most one class (E3008 for non-class
  values).
- In a derived class's constructor, `super(args)` calls the superclass
  constructor. It must be called exactly once, as a top-level statement of the
  constructor body, before any access to `self` (E3001). The derived class's
  field initializers run immediately after it.
- `super:method(args)` / `super.method(self, args)` calls the superclass
  implementation. Lowered to a direct call on the superclass table:
  `Animal.speak(self)`. In static methods, `super.member` refers to the
  superclass table (`super.count` → `Animal.count`).
- Static members are inherited: `Dog.count` reads `Animal.count` until `Dog`
  assigns its own.
- Metamethods are inherited. The runtime copies them into subclasses at class
  finalization, because Lua looks up metamethods with a raw access.

### 3.1.5 Abstract classes

```wowlua
abstract class Shape
    abstract function area(): number
    function describe(): string return `area {self:area()}` end
end
```

An abstract class cannot be instantiated (E3002), whether directly or through a
value of type `ClassOf<T>` where `T` is abstract. Every concrete subclass must
implement all abstract methods (E3003). In debug builds, abstract methods are
emitted as stubs that raise an error if ever called.

### 3.1.6 Generic classes

```wowlua
class Stack<T>
    private items: T[] = {}
    function push(item: T) self.items[#self.items + 1] = item end
    function pop(): T? local n = #self.items; local v = self.items[n]; self.items[n] = nil; return v end
end

local s: Stack<string> = Stack.new()
```

Type arguments are inferred from constructor arguments or the expected type.
They are erased at runtime. Variance follows [Types §2.2.6](02-types.md#226-generics).

### 3.1.7 Reserved member names

`new`, `__index`, `__newindex`, `__name`, `__super`, `__init`, `__ctor`,
`__initFields`, `__class`, `__meta`, `__getters` and `__setters` are reserved
(E3004). The metamethods allowed as methods are `__tostring`, `__eq`, `__lt`,
`__le`, `__add`, `__sub`, `__mul`, `__div`, `__mod`, `__pow`, `__unm`,
`__concat` and `__call`.

### 3.1.8 Typing

- Classes are **nominal**. A value has type `Dog` only if it was created by
  `Dog` or a subclass. A record with the same fields is not a `Dog`.
- Class instances may be assigned to interfaces and record types they
  structurally satisfy through their **public** members
  ([Types §2.6](02-types.md#26-assignability)).
- The class value itself has type `ClassOf<Dog>`. It exposes static members,
  `new` and the call signature.
- `implements I` is checked at the class declaration (E3006 for missing or
  incompatible members). It also adds `I` as a declared supertype for
  documentation and reflection.

### 3.1.9 Definition order and lowering

A class declaration executes in this order (decisions L24, L26):

1. **Hoisting.** Every top-level exported function, class and enum is
   pre-declared as a local at the top of the module, so declarations can refer to
   each other regardless of source order.
2. The class table is created (`__wl.class`).
3. Methods, accessors and the constructor are defined, in member order.
4. Member decorators run, in member order ([Decorators §4.5.2](04-decorators.md#452-evaluation-order)).
5. `__wl.finalize` copies inherited metamethods and installs debug stubs.
6. **Static field initializers** run once, in declaration order. They can call
   static methods and see the finalized class.
7. Class decorators run.
8. Registering decorators (`@global`) and the export assignment receive the
   final class value.

```wowlua
export class Dog extends Animal
    tricks: string[] = {}
    static created: number = 0
    constructor(name: string)
        super(name, 1)
        self.sound = "woof"
    end
    override function speak(): string
        return super:speak() .. "!"
    end
end
```
```lua
-- module top: local Dog  (hoisted)
Dog = __wl.class("Dog", Animal)
function Dog.__init(self, name)
    Animal.__init(self, name, 1); self.tricks = {}
    self.sound = "woof"
end
function Dog.speak(self)
    return Animal.speak(self) .. "!"
end
__wl.finalize(Dog); Dog.created = 0; __wl_exports.Dog = Dog
```

`__wl.class(name, super)` creates the class table (with `__index` pointing to
itself, a metatable chaining to `super` and providing `__call`), and defines
`new`. Accessors are registered with `__wl.accessor(cls, kind, name, fn)`
(`kind` is `get`, `set`, `sget` or `sset`), and abstract methods of debug builds
with `__wl.abstract(cls, name)`:

```lua
cls.new = function(...)
    local self = setmetatable({}, cls)
    cls.__init(self, ...)
    return self
end
```

Field initializers of a base class without a constructor are emitted into a
generated `__init`. A class with no fields and no constructor inherits
`__init` (the root `__init` is a no-op).

### 3.1.10 Accessors (getters and setters)

```wowlua
class Temperature
    private celsius: number = 0

    get fahrenheit(): number
        return self.celsius * 9 / 5 + 32
    end

    set fahrenheit(value: number)
        self.celsius = (value - 32) * 5 / 9
    end
end

local t = Temperature.new()
t.fahrenheit = 212
print(t.fahrenheit)        -- 212
```

- `get name(): T … end` and `set name(value: T) … end` declare a property. A
  getter without a setter is read-only, and assignment is error E2010.
- Accessors may be `static`, `private`/`protected`, and `override` a superclass
  accessor.
- **Lowering.** Only classes that declare accessors, and their subclasses, get
  function-based `__index` and `__newindex` metamethods that dispatch property
  names to the getter and setter tables, and fall back to the class table for
  methods. Property values are never stored under the property name, so
  `__newindex` fires for every property assignment. All other classes keep the
  plain table `__index`, so field access stays fast.
- Widget classes cannot declare accessors (E3043), because the frame's
  metatable belongs to WoW.

### 3.1.11 Overloads

A method or constructor may declare several **overload signatures**, followed
by one implementation:

```wowlua
class Inventory
    overload function find(id: number): Item?
    overload function find(name: string): Item?
    function find(key: number | string): Item?
        if type(key) == "number" then return self.byId[key] end
        return self.byName[key]
    end
end
```

- Callers see only the overload signatures, which are resolved as in
  [Types §2.7](02-types.md#27-calls-and-arity).
- The implementation signature must accept every overload (E3044) and is not
  callable directly.
- Overloads are compile-time only. One function is emitted.

## 3.2 Widget classes

A class that extends a **native widget type**, i.e. a declared class marked
`@widget("…")` such as `Frame`, `Button` or `StatusBar`, is a *widget class*.
WoW widgets are created by `CreateFrame` and already have a metatable, so
widget classes are lowered as **mixins**, following Blizzard's own
`Mixin` pattern.

```wowlua
export class CounterButton extends Button
    count: number = 0

    constructor(parent: Frame)
        super(nil, parent, "UIPanelButtonTemplate")   -- CreateFrame("Button", nil, parent, template)
        self:SetSize(120, 24)
        self:SetScript("OnClick", function() self:increment() end)
        self:refresh()
    end

    function increment()
        self.count += 1
        self:refresh()
    end

    function refresh()
        self:SetText(`Clicked {self.count}x`)
    end
end

local b = CounterButton.new(UIParent)
b:SetPoint("CENTER")
```

Rules:

- For the first class in the chain that extends a native widget, `super(name?,
  parent?, template?, id?)` creates the frame with
  `CreateFrame(<widgetType>, name, parent, template, id)`. Before that call,
  `self` does not exist (E3001). In that class, `super:m(args)` calls the
  native widget method (`getmetatable(self).__index.m(self, args)`).
- Native widget types are the classes declared with `@widget("…")`. Until the
  generated `wow-api` packages provide them (milestone M4), the compiler
  declares every frame type `CreateFrame` creates (decision T77).
- Construction applies the methods of the **most-derived** class and its
  wowlua superclasses, flattened into one mixin, with a raw copy, so
  overrides work and virtual calls in constructors dispatch to the
  most-derived implementation. The mixin is flattened from the class chain
  as it is at construction: a method replaced on a widget class or one of
  its superclasses after the class is defined (a plugin patching it) reaches
  the frames constructed afterwards, as instances of a plain class see it;
  frames constructed before keep the methods they were given (decision
  T268).
- `C.adopt(frame, …)` builds the class on a frame that already exists (the
  buttons a secure group header makes, a frame from a template) instead of
  creating one: the root class's `super(…)` takes `frame`, the constructor
  arguments follow it, and the `@script` handlers are installed as by `new`.
  `frame` must be of the native type the class chain ends in (`Button` for a
  class extending `Button`). A root class without a constructor of its own
  takes `frame` alone (decision L42).
- Widget classes may extend other widget classes (`class FancyButton extends
  CounterButton`). Construction is threaded through a hidden `__ctor(cls, …)`
  so only the root creates the frame.
- Metamethods and accessors are not allowed on widget classes (E3007, E3043),
  because the frame's metatable belongs to WoW.
- The instance has a `__class` field pointing to its class, used by
  `reflect.classOf` and `reflect.isInstance`.
- `@global("MyButtonMixin")` exposes the flattened mixin as a global so XML
  templates can use it with `mixin="MyButtonMixin"`. A method named `OnLoad`
  is then called by the XML `OnLoad` script as usual. Field initializers for
  XML-created frames run in a generated `__initFields(self)`, which the
  generated mixin calls before the user's `OnLoad`.

### 3.2.1 Script handlers

The core decorator `@script(name, opts?)` makes an instance method of a widget
class the handler of one of the frame's scripts:

```wowlua
export class InputBox extends EditBox
    onSubmit: (string) -> ()

    constructor(parent: Frame, onSubmit: (string) -> ())
        super(nil, parent, "InputBoxTemplate")
        self.onSubmit = onSubmit
    end

    @script("OnEnterPressed")
    function submit()
        self.onSubmit(self:GetText())
        self:ClearFocus()
    end

    @script("OnShow", { hook = true })    -- keeps the template's OnShow
    function shown() … end
end
```

- The handler is set with `SetScript`, replacing any other handler of that
  script; with `{ hook = true }` it is added with `HookScript`.
- Handlers are installed **after** the most-derived constructor has finished,
  so the constructor cannot trigger them early (a handler the constructor sets
  for the same script is replaced). For frames created from XML, they are
  installed by the generated mixin's `OnLoad`, after the field initializers and
  before the class's own `OnLoad`.
- `self` is the frame; the script's arguments are the method's parameters
  (`function clicked(button: string, down: boolean)`).
- Registrations are inherited: a subclass that overrides a decorated method
  keeps the registration, and its override is called. A `SetScript` handler of
  a subclass for the same script replaces the superclass's; hooks add up.
- `@script` applies only to instance methods of widget classes (E3021). The
  checker (milestone M4) also checks the script name against the widget type.

## 3.3 Interfaces

```wowlua
export interface Pet
    name: string
    owner?: string
    function play(withToy: string?): boolean
end

interface Comparable<T>
    function compare(other: T): number
end

interface Saveable extends Pet, Comparable<Saveable>
    function serialize(): string
end
```

- Interfaces are compile-time only and produce no code.
- `function m(...)` members are method signatures with implicit `self`. Fields
  use the record field syntax and may be `readonly`.
- Interfaces may extend multiple interfaces.
- Any value with a compatible shape satisfies an interface: class instances,
  records, plain Lua tables. `implements` is an explicit check, not a
  requirement.
- Interfaces may be declared in any block and are scoped to it.
- Two interfaces with the same name in the same scope are error E1031. Only
  ambient `declare interface` declarations merge
  ([Types §2.4.7](02-types.md#247-the-addon-namespace)).

## 3.4 Enums

### 3.4.1 Syntax

```wowlua
enum Direction { Up, Down, Left, Right }        -- 1, 2, 3, 4

enum Quality { Poor = 0, Common, Uncommon, Rare, Epic, Legendary }   -- 0..5

enum Anchor {
    TopLeft = "TOPLEFT",
    Center  = "CENTER",
}
```

- Members are separated by `,` or `;`, and a trailing separator is allowed.
- Members without a value are numbered: they continue from the previous numeric
  value plus one, starting at `1` (Lua convention).
- Explicit values must be number or string literals or constant expressions.
- An enum is either fully numeric or fully string-valued (E3010).
- Duplicate values are an error (E3011).
- Enums may only be declared at the top level of a file (E0115).

### 3.4.2 Typing

- `Direction` as a type is the union of its members. `Direction.Up` has the
  member type `Direction.Up`.
- Enum members are assignable to their enum and to the underlying primitive
  (`number` / `string`). The reverse needs a cast (`raw :: Direction`) or a
  checked conversion (`reflect.enumFrom(Direction, raw)`), because enums are
  nominal.
- `switch`-like narrowing on enum comparisons works as for literal unions.
- Assigning to an enum member (`Direction.Up = 5`) is error E2010.

### 3.4.3 Lowering

Member accesses with a constant key are **inlined**: `Direction.Up` becomes `1`.
The enum table is emitted only if the enum is used as a value (iterated, passed
around, used with `reflect`, or exported to plain Lua):

```lua
local Direction = { Up = 1, Down = 2, Left = 3, Right = 4 }
```

An exported enum always gets its table until tree-shaking removes unused
ones (milestone M7, decision T81). The emitted table contains exactly the
members, in every profile. Debug builds
attach a metatable to that same table (no proxy) that raises an error when an
unknown member is read or a new key is assigned. `pairs` therefore behaves
identically in debug and release builds, and plain Lua code sees the same
table.

**Iteration order.** `pairs` over the enum table is unordered, as for any Lua
table. `reflect.enumEntries(E)` returns the members in declaration order, and
`reflect.enumName(E, value)` does reverse lookup, so the enum table never
contains anything but its members.
