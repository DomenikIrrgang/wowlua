# 2. Types

wowlua has a **gradual, mostly structural** static type system in the spirit of
TypeScript and Luau. Types exist only at compile time and are erased from
the output, except where a build profile asks for runtime checks or
reflection.

## 2.1 Checking modes

| Mode | Selected by | Behavior |
|------|-------------|----------|
| `nocheck` | `--!nocheck`, manifest | No type diagnostics. Syntax and name resolution only. |
| `basic` | default | Annotated and inferred types are checked. Unannotated parameters are `any`. `nil` is assignable to every type. Undefined globals are warnings. |
| `strict` | `--!strict`, manifest | Everything in `basic`, plus: **nil safety** (`T` excludes `nil`, use `T?`), implicit global reads/writes are errors, parameters of non-callback functions need annotations (E2050), adding unknown fields to sealed tables is an error (E2051), and non-optional class fields must be definitely assigned in the constructor (E2052). |

A **callback** (exempt from E2050) is a function expression passed directly
as a call argument or written where a type is expected, even when that type
is only `function` (as for `SetScript` handlers in the generated
declarations).

The project default is set in the manifest (`check.mode`, plus `check.strict`
globs). A file directive overrides both for that file.

**Superset rule.** A *pure Lua* file (a `.wowlua` file without wowlua syntax,
or a checked `.lua` file) is never failed by the checker unless the file
itself is opted into strict mode, either by its own `--!strict` directive or by
matching a `check.strict` glob. A project-wide `check.mode = "strict"` does not
apply to pure Lua files. In a pure Lua file that is not opted in:

- **every** diagnostic in the ranges x1xxx (names, globals, modules) and x2xxx
  (types, flavors, secret values) is reported as a warning: E1xxx as W1xxx and
  E2xxx as W2xxx with the same number (for example E2030 becomes W2030 and E2040
  becomes W2040);
- syntax errors (x01xx) and Lua limits (E5001) remain errors, because the file
  would not load in WoW either.

`.lua` files are not type-checked unless `check.lua` is enabled in the
manifest, and then only with warnings.

## 2.2 Type syntax

```
Type        ::= Union
Union       ::= "|"? Intersection ( "|" Intersection )*
Intersection::= Postfix ( "&" Postfix )*
Postfix     ::= Primary ( "?" | "[" "]" )*
Primary     ::= "nil" | "true" | "false" | "function" | String | Number
              | TypeRef | "typeof" "(" Expr ")"
              | TableType | FunctionType | "(" Type ")"
TypeRef     ::= Name ( "." Name )* TypeArgs?
TypeArgs    ::= "<" TypeArg ( "," TypeArg )* ">"
TypeArg     ::= Type | TypePack
```

A leading `|` is allowed for multi-line unions. The complete grammar,
including function types and type packs, is in
[Appendix A §A.5](../appendix/grammar.md#a5-types).

### 2.2.1 Primitive and special types

| Type | Values |
|------|--------|
| `nil` | `nil` |
| `boolean` | `true`, `false` |
| `number` | Lua numbers (doubles) |
| `string` | Lua strings |
| `table` | Any table. Indexing yields `any`. |
| `function` | Any function. Calling yields `any`. (`function` is a reserved word, accepted here as a type.) |
| `thread` | Any coroutine. See `Thread<…>` (§2.2.9) for typed coroutines. |
| `userdata` | Raw userdata |
| `any` | Opts out of checking. Assignable to and from everything. |
| `unknown` | Top type. Anything is assignable to it, and it must be narrowed before use. |
| `never` | Bottom type. Has no values, e.g. the return type of `error`. |

### 2.2.2 Literal types

`"LEFT"`, `42`, `true` and `false` are types containing exactly that value. String
literal unions model WoW's string enums:

```wowlua
type Anchor =
    | "TOPLEFT" | "TOP" | "TOPRIGHT" | "LEFT" | "CENTER" | "RIGHT"
    | "BOTTOMLEFT" | "BOTTOM" | "BOTTOMRIGHT"
```

### 2.2.3 Optional, union, intersection

- `T?` means `T | nil`.
- `A | B` is a value of either type.
- `A & B` is a value satisfying both types, used mainly for combining table
  shapes and for function overloads in declarations.
  Reading a field of an intersection gives the type of the first part that
  has the field; a part that only reads it as `any` (an array asked for a
  named field) doesn't count. A key read (`c[i]`) gives the first part's
  with numbered values or an indexer: in `number[] & { hex: string }`,
  `c.hex` is a `string` and `c[1]` a `number` (decision L60).

### 2.2.4 Table types

| Syntax | Meaning |
|--------|---------|
| `T[]` or `{T}` | Array with elements `T` (1-based, sequence) |
| `{ [K]: V }` | Map from `K` to `V` |
| `{ x: number, y: number }` | Record. `,` or `;` separate fields. |
| `{ name: string, label?: string }` | `label` is optional: `string?`, may be absent |
| `{ ["end"]: number, ["my-key"]?: string }` | Record fields whose keys are reserved words or not identifiers (`?:` makes one optional) |
| `{ readonly id: number, readonly ["end"]: number }` | Read-only fields: assigning one is E2010 |
| `{ x: number, [string]: any }` | Record with an indexer for other keys |
| `{ [number]: string, n: number }` | Table used both as an array and a record (e.g. the result of `{ ..., n = select("#", ...) }`) |

Reading an element of an array (`xs[i]`) gives `T` in every mode, although
reading past the end gives `nil` at run time (decision T95). Reads through a
map or an indexer give `V?`. In strict mode an array has no named fields but
`n` (`xs.label` is E2003: a list with named fields is written
`{ [number]: string, label?: string }`), and a record with fields read with a
number literal it doesn't have (`art[1]` of `{ atlas: string }`) is E2003;
another key on a table without an indexer reads `any`, which W2063 reports
(decision L49). A key whose type is one or more string literals (`k: keyof
Colors`, `"a" | "b"`) reads those fields, as `T[K]` does (decision L53).
A method call `r:Name(...)` with a name the record doesn't declare calls the
indexer's value, as `r.Name` reads it: with `[string]: unknown` it is E2004
(an `unknown` must be narrowed before it is called), with `[string]: F` it
checks against `F` (decision L77).

Records are **structural**: any table with at least the required fields of
compatible types is assignable. Extra fields are allowed, except that an
object literal assigned directly to a record type with an unknown field is
an error in strict mode (E2003), to catch typos. So is a literal's named
entry other than `n` where an array is expected (`{ class = "X", "Y" }` for
a `string[]`): an array has numbered values only, and the entry would be
dropped (decision L71).

### 2.2.5 Function types

```wowlua
type Handler = (event: string, ...any) -> ()
type Predicate<T> = (value: T) -> boolean
type Multi = (string) -> (boolean, string?)     -- parameter names are optional
type Rest = (string) -> (number, ...string)     -- fixed prefix plus variadic results
type Guard = (value: unknown) -> value is Frame -- type predicate
```

- Return position: a single type, `()` for no values, `(A, B)` for multiple
  values, `(A, ...B)` for a fixed prefix plus variadic values, or a type pack
  `T...` (§2.2.10).
- Parameters: `name: T`, `T`, `name: T?` (optional), `...T` or `...: T`
  (variadic), or `...: T...` (pack).
- Methods are functions whose first parameter is `self`. In class and
  interface bodies `self` is implicit (see [Classes](03-classes.md)).
- A function type's return type extends as far as the type grammar allows:
  `() -> number | nil` returns `number | nil`. Write `(() -> number) | nil` for an
  optional function.

### 2.2.6 Generics

Functions, type aliases, interfaces and classes may have type parameters,
optionally constrained with `extends`:

```wowlua
type Pair<A, B> = { first: A, second: B }

function map<T, U>(xs: T[], f: (T) -> U): U[]
    local out: U[] = {}
    for i, x in ipairs(xs) do out[i] = f(x) end
    return out
end

function largest<T extends { size: number }>(xs: T[]): T? … end
```

Type arguments are **always inferred at call sites**. An inferred type
argument that does not satisfy its constraint is error E2014. There is no explicit
call syntax like `f<number>(x)`, because in Lua `f < number > (x)` is a valid
comparison expression. Expected types also drive inference:
`local s: Stack<number> = Stack.new()`. The expected type binds a type
parameter the arguments left open, or replaces an argument's binding that it
fits (`local xs: (number | string)[] = list(1)` binds `T` to
`number | string`); a binding that does not fit stays, so `local b: boolean =
id(5)` reports `number?`, not a union with `boolean` (decision L36). An
expected `any` or `unknown` (`print(id(5))`) binds nothing (decision L48). A
parameter `ClassOf<T>` binds `T` to the class whose table is passed (any type
arguments of a generic class `any`), or to the enum whose table is passed:
`enumFrom<E>(e: ClassOf<E>, raw: any): E?` called with `Direction` gives
`Direction?`. Inside the generic body a `ClassOf<T>` value is a table, and
where nothing binds `T` it is `any`. A parameter `T | Secret<T>` (or
`T | Secret<T> | nil`) binds `T` from both parts of an argument: passing
`string | Secret<string>` binds `T` to `string`. A literal argument binds a
constrained type parameter to the literal (`E extends EventName` called with
`"UNIT_HEALTH"` binds `"UNIT_HEALTH"`) and an unconstrained one to its
widened type (`T` binds `string`); a value typed elsewhere (a name, a field,
a call's result) binds as its type, a union of literals included, as a
`local` keeps it (`assert(tokens[k])` with `tokens: { [string]: Token }` is
a `Token`); decision L44. Inside the body, a value of a
constrained type parameter fits wherever its constraint does (an `E extends
EventName` is passed on as an `EventName`), and its fields and methods are
the constraint's (`b:GetWidth()` with `b: B` and `B extends Frame`; decision
T279). An unconstrained type parameter's members are `any`.

**Variance** (decision L34). For generic classes, interfaces and aliases:

| Mode | Rule |
|------|------|
| `basic` | Type arguments compare **covariantly**: `List<Dog>` is assignable to `List<Animal>`. Pragmatic and unsound, as in TypeScript. |
| `strict` | A type parameter is **covariant** if it appears only in read positions (readonly fields, return types), **contravariant** if only in parameter positions, and **invariant** otherwise (mutable fields, or both). `List<Dog>` is then not assignable to `List<Animal>` when `List` has a mutable `items: T[]`. |

Arrays and maps (`T[]`, `{ [K]: V }`) follow the same rule: covariant in basic
mode, invariant in strict mode.

### 2.2.7 `typeof`

`typeof(expr)` in type position is the static type of `expr`:

```wowlua
local defaults = { scale = 1, locked = false }
type Settings = typeof(defaults)    -- { scale: number, locked: boolean }
```

The expression is typed where the alias stands, so it may only refer to
names declared before it. A `typeof` alias has its type once its file has been
checked up to it: in the file's later annotations and in the modules checked
after it (importers, and a library's consumers, spec §5.4.4). A class
declared after it in the file has its field and method types worked out
again where the file reaches the class, so they read the alias's type
(decision L57); so does a class in another module whose field or method
annotations name an alias of an imported module (`api: UnitApi`, a record of
`typeof(UnitHealth)` and the like, checked after that module), decision L76.
Other uses before that point, such as a hoisted function's signature or
another alias that refers to it, read it as `any`.

Written in a class field's own annotation (`map: typeof(WorldMapFrame)? =
nil`), `typeof(expr)` is typed where the file reaches the class, and the
field reads that type in the class's methods and outside (decision L70).

Written in a function's own annotations (a `local function`, a module's
function, a function value), `typeof(expr)` is typed where the function
stands, as in a `local`'s annotation: `local function hex(k: keyof
typeof(colors)): string return colors[k].hex end` takes the colours' field
names and reads their type (decision L62).

### 2.2.8 Type predicates

A function whose return type is `param is T` narrows its argument in the
caller:

- where the call is **truthy**, the argument is narrowed to `T` (intersected
  with its current type); a member that passes for `T` only because it is
  open (a game frame takes any field, so `Frame` passes for `Frame &
  Disableable`) is narrowed to `T` too, whose members are declared (decision
  L79); so is an object of an unrelated type when `T` is a class: it may still
  be one (`reflect.isInstance(module, WorldMapModule)` on a module's record
  type, decision L82);
- where the call is **falsy** (`else` branch, `if not p(x)`, after
  `if p(x) then return end`), the argument is narrowed by **excluding** the
  members of its union type that are assignable to `T`. A member that is a
  supertype of `T` (`Frame` for `f is Button`) or `any` may be either, so it
  stays in both.
- a branch that **no member reaches** (every member excluded, as in a second
  `if not issecretvalue(c) and c then … end` after the first returned, or the
  `else` of a predicate every member fits) leaves the argument `never`: the
  branch is **unreachable**, and nothing in it is reported. (Whether the code
  after the `if` runs still depends only on `return`, `break` and calls that
  never return, so it is not W1005.) The same holds for `canaccessvalue`
  (§2.11).

A generic predicate's type parameters are inferred from its arguments, the
narrowed one's type as it is at the call: `plain<V>(v: V | Secret<V> | nil):
v is V` called with a `number | Secret<number>` narrows it to `number`, and
`reflect.isInstance<T>(v: any, cls: ClassOf<T>): v is T` called with `Dog`
narrows `v` to `Dog`.

```wowlua
local function isButton(f: Frame): f is Button
    return f:GetObjectType() == "Button"
end

if isButton(region) then region:Click() end   -- region: Button here

local v: string | Secret<string> = UnitName("target")
if issecretvalue(v) then return end
print(v:upper())                              -- v: string (Secret excluded)
```

### 2.2.9 Special named types

| Type | Meaning |
|------|---------|
| `Self` | Inside class and interface bodies: the type of the receiver. Used for fluent APIs and in declarations (`function SetScript(...): Self`). In a declared class (`declare class`), `Self` is bound where a member is read, to the receiver's type: `toggle:SetScript("OnClick", function(self) … end)` on an instance of `class Toggle extends Button` gives `self: Toggle` (decision L46); on a value typed as a game type with more (`Button & { owner: Frame? }`), the whole value: `self.owner` is typed (decision L80). |
| `ClassOf<T>` | The class value (constructor table) of instance type `T`, or the table of enum `T`. Calling `new` through `ClassOf<T>` is an error if `T` is abstract (E3002). For a type parameter `T` it binds `T` from the table passed (§2.2.6). |
| `EventName` | Union of all **WoW event** names (`declare event`). |
| `MessageName` | Union of all **addon message** names (`declare message`). |
| `EventArgs<E>` | Type pack of the payload of event or message `E`. Used by `wowlua:events`, and usable in any function: in a call where `E` is a declared name, `(name: E, EventArgs<E>...) -> ()` takes the payload's parameters and `...: EventArgs<E>...` its values; elsewhere (`E` still a parameter, or a union of names) it is any values (decision L44). |
| `Thread<A..., Y..., R...>` | A typed coroutine: started with arguments `A`, yields `Y`, returns `R` (§2.4.8). |
| `Secret<T>` | A Midnight-era *secret value* of underlying type `T` (§2.11). |
| `AspectSecret<T, S>` | A widget result that is secret only after a setter got a secret: `T`, or `S` (`T \| Secret<T>`) when the package sets `check.secret-widget-results` (§2.11, decision T474). The generated declarations use it. |
| `AddonNamespace` | The type of the addon table received through `...` (§2.4.7). |

### 2.2.10 Type packs

A **type pack** is a sequence of types: the parameter list or the results of
a function. Generic **pack parameters** (decision L27, Luau style) let
signatures pass whole sequences through:

```wowlua
-- declared in the Lua standard library declarations
declare function pcall<A..., R...>(f: (A...) -> R..., ...: A...): (boolean, R...)
declare function select<T...>(index: number, ...: T...): ...any
declare function select<T...>(index: "#", ...: T...): number

-- user code
local function retry<A..., R...>(times: number, f: (A...) -> R..., ...: A...): R...
    for i = 1, times - 1 do
        local ok = pcall(f, ...)
        if ok then break end
    end
    return f(...)
end
```

| Syntax | Meaning |
|--------|---------|
| `<T...>` | Declares a pack parameter |
| `...: T...` | Variadic parameter bound to a pack |
| `(A...) -> R...` | Function type with pack parameters and results |
| `(number, string)` as a type argument | A pack literal, e.g. `Thread<(number), (), (boolean)>` |
| `EventArgs<"UNIT_HEALTH">` | A pack-valued special type |

Packs are inferred from arguments like other type parameters. A pack can only
appear in parameter-list and result positions, as a type argument for a pack
parameter, or as the expansion `...: T...` (E2056 otherwise).

### 2.2.11 `keyof` and indexed access

`keyof T` is the union of the names of a table type's fields (and its
indexer's key type); `T[K]` is the type of `T`'s field `K` (decision L45):

```wowlua
type Defaults = { line: number, figure: boolean }
type Key = keyof Defaults          -- "line" | "figure"
type Line = Defaults["line"]       -- number

function get<D extends { [string]: unknown }, K extends keyof D>(values: D, key: K): D[K]
    return values[key] :: D[K]
end
local n = get({ line = 20, figure = true }, "line")   -- number
get({ line = 20 }, "lnie")         -- E2014: not a key of the table
```

| Form | Gives |
|------|-------|
| `keyof` a record | its field names as string literals, and its indexer's key type |
| `keyof` a map `{ [K]: V }` / an array | `K` / `number` |
| `T["name"]` | the field's type (with `nil` for an optional field); a missing field: `nil`, or the indexer's value or `nil` |
| `T["a" \| "b"]` | the union of both fields' types |
| `T[string]` | every field's type, the indexer's, and `nil` |
| `T[K]` of a map or an array | the element type or `nil` |
| `keyof (A & B)` / `(A & B)["name"]` | the names of both / the field of the part that has it |

While `T` or `K` is a type parameter, `keyof T` and `T[K]` stay as they are
and are worked out where the parameter is bound. Inside the generic body, a
`keyof T` value is a `string`, and a `T[K]` value is not checked (as `any`):
it is checked where the call binds `T` and `K`. With `K extends keyof T`, the
literal argument binds `K` (decision L44), so the result is that field's
type. A generic method of a generic class reads the class's type arguments
in its own constraints, so `Get<K extends keyof D>(key: K): D[K]` of
`Module<D>` takes the keys of the `D` an instance or a subclass gives
(`class Durability extends Module<typeof(DEFAULTS)>`); a base written with
`typeof`, directly or through a `typeof` alias, is worked out where the
file reaches the class. A table literal passed for a parameter whose type
parameters are constrained to literal unions (`keyof T`, `EventName`) is
checked against the constraints, so its
strings stay literals: `watchAll<K extends keyof D>(keys: K[], …)` called
with `{ "line", "figure" }` binds `K` to those keys, and a name that is not
one is E2014. (Only a list of the parameter itself, `K[]`: a table literal
for `D[K]` takes its type from the bound `K` as usual.) A table literal
argument holding a function with an unannotated parameter
(`segmented({ options = { … }, onChange = function(v) … end })`) binds the
type parameters from its other fields first, on trial, and is then checked
against its parameter with them bound, so `v` gets the option values' type
(decision L48).

## 2.3 Type aliases and recursive types

```wowlua
type ItemId = number
type Callback<T> = (value: T) -> ()
export type Point = { x: number, y: number }
```

Aliases are transparent. `ItemId` and `number` are interchangeable.

**Recursion.**

- An alias may refer to itself only inside a table type or function type
  (`type Tree = { value: number, children: Tree[] }`). An unguarded recursive
  alias (`type A = A | nil`, `type B = C; type C = B`) is error E2053.
- An alias whose body is a table type and that refers to itself, directly
  or through other aliases, is a named structural type: its references have
  its fields (as an interface's members, with the alias's type arguments),
  error messages name it, and values match it exactly as they match its
  table type (decision L58). In a body that is not a table
  type, a reference to the alias itself reads as `any`.
- Interfaces and classes are named types and may refer to themselves and to each
  other freely, including across modules through `import type`, which never
  creates a load-order dependency.

## 2.4 Inference

Inference is local and bidirectional. There is no whole-program inference.

| Construct | Inferred type |
|-----------|---------------|
| `local x = 5` | declared type `number` (literal widened); a later `x = "a"` is E2001 (§2.4.1) |
| `local x`, `local x = nil` | no declared type of its own: the union of the values assigned to it (§2.4.1) |
| `local p = s :: Point`, `local k = opts.kind` | `Point`, the field's type: a value typed elsewhere (a name, a field or index read, a call's result, a `::` assertion) keeps its type, unions of literals included (decision L64) |
| `local y = ok and t` (`ok: boolean`, `t: T`) | `false \| T` (`false` and `true` beside types other than `nil` and booleans are not widened) |
| `const X = 5` | `5` (literal kept) |
| `local t = { a = 1, b = "x" }` | `{ a: number, b: string }`; a field or element written `v :: T` keeps `T` (`{ corner = "TOP" :: FramePoint }` is `{ corner: FramePoint }`, decision L84) |
| `local t = { 1, 2, 3 }` | `number[]` |
| `local t = {}` | open table, see below |
| function without return annotation | union of all returned types (`()` if none); a class's method without one too: its results become its body's when the check reaches it, the class's methods checked first (callees first), the file again where one was read earlier; a generic method's stay `...any`, decision L86 |
| unannotated parameter | `any` (basic/strict), or the contextual type if available |

### 2.4.1 Unannotated locals

An unannotated local declared with a value takes that value's type as its
**declared type**, as if it were annotated with it: the type the table above
gives (`local x = 5` is `number`; `local k = opts.kind` is the field's type,
a union of literals included). A single literal widens wherever it comes
from (`local n = LIMIT` with `const LIMIT = 40` is `number`), a literal
boolean included (`local done = false` is `boolean`), and an enum's member is
its enum (`local m = Mode.Fast` is `Mode`). Each name of `local a, b = f()`
takes the type of its own value. A later assignment is checked against the
declared type as against an annotation: a value that does not fit is E2001 in
basic and strict mode alike, and W2001 in a pure Lua file (§2.1); `nil` fits
it in basic mode only, so in strict mode a local that is cleared later is
annotated (`local t: Timer? = …`). A local that should hold values of several
types says so (`local x: number | string = 5`), decision L100.

```wowlua
local test = 3
test = "hello world"   -- E2001: `"hello world"` is not assignable to `number`
```

A value of type `any` gives no declared type: the local is `any`. A local
without a value, with `nil` or with a value of type `nil` (a name past the
values of `local a, b = 1` too) has nothing to take a type from: its declared
type is the union of the types of all values assigned to it within its scope.
So is a local declared under `@if` (§6.4.3): a build that removes it may
define the name otherwise (`export function name()`).

At each use, an unannotated local's **narrowed type** is the type of the most
recent assignment on that control-flow path (within its declared type), so a
local without a value may hold different types over time while misuse is still
caught, such as calling `:upper()` on a value that is a number at that point.
A test narrows it as it narrows an annotated local (§2.5): after
`local x = f()` with `f(): T?`, `if x then` gives `T`.

A table assigned to an unannotated local after its declaration fills in what
an empty table leaves out: where the value fits the local's type (its declared
type, or for a local without a value its type so far) once filled, each field
holding `{}` (with no fields and no indexer) takes that type's field type,
also in the records of an array (`cards = { { lines = {} } }` for a
`{ lines: string[] }[]`). In
`local p = byKey[key]; if not p then p = { seen = {} } end` with
`byKey: { [string]: { seen: { [number]: boolean } } }`, `p.seen` is the map,
not an empty table read as `any`. The value's other fields stay as they are
(decision L59).

A literal assigned to an unannotated local keeps its literal type where the
local's type has that literal: `local w = cache[k]; if w == nil then w = false
end` with `cache: { [string]: T | false }` leaves `w` a `T | false`, so
`cache[k] = w` fits. Other assigned literals widen as in a `local` (`w = true`
there is E2001: `w` is declared `T | false | nil`, and `true` is a `boolean`),
decision L61.

A table literal's declared type is the table as declared: `local t = {}` is
`{}`, so any table fits it later, and `local t = { a = 1 }` takes tables with
a numeric `a`. While the table is open (§2.4.2) the fields it gains extend its
narrowed type, not its declared type.

Inside a closure, an unannotated local that a closure assigns, or that is
captured and assigned later, may hold any value it is given before the closure
runs: it reads as the union of its narrowed types where functions are made in
its scope and the values assigned to it after one is made or from inside a
function (decision L47), within its declared type when it was declared with a
value (`local f = self.frame` … `if not f then f = make() end` … `later(function()
f:Show() end)` reads as the frame, decision L102). An annotated local reads as its
declared type (§2.5). A local declared without a value or with
`nil` adds no `nil` for a function made before it is first assigned, so a
forward declaration (`local f` … `f = function() … end`) reads as the
function. The union depends on what is read in closures, so a file with such
reads is checked again with the unions the pass before found until they hold
(at most four passes; a local whose union still changes reads as `any`).

An annotated local without a value (`local x: T`) is `nil` until assigned:
its narrowed type follows the assignments, so where some path did not assign
it, it reads as `T?` and strict mode reports the use. Closures see `T`.

### 2.4.2 Open and sealed tables

A table literal assigned to an unannotated local is **open** while it stays in
the function that created it: assigning new fields extends its type. It
becomes **sealed** when it escapes (passed as an argument, returned, stored in
another table, or captured by a closure). Adding unknown fields to a sealed
table is an error in strict mode (E2051) and allowed in basic mode.

A field added by an assignment takes the assigned value's widened type
(`t.enabled = true` adds `enabled: boolean`), as a literal's fields do
without an expected type. A table literal held in a field of an open table
is open too, and a field added through the path (`API.UI.B = f` after
`API.UI = { A = g }`) is part of the outer table's type; sealing a table
seals the tables in its fields (decision T275). A field added as `nil` (`t.mode = nil`) is optional
and `any`.

Reading a field an open table does not have gives `any` in basic mode
(decision T87). In strict mode, reading a field the file never stores in that
table (no `t.name = …`, `function t.name()` or `function t:name()` anywhere)
is E2003, as for a record: it is a misspelling (`stats.deths`). A field stored
somewhere in the file may be set by the time it is read, and stays `any`
until then. This holds for a local declared with a table literal and for an
object `setmetatable` made (§2.4.6).

Writes with non-constant keys (`t[k] = v`) to an open table add an indexer: the
table's type becomes its known fields plus `[K]: V`, where `K` and `V` are the
unions of all such keys and values.

```wowlua
local M = {}
M.version = 1               -- M: { version: number }
function M.greet(n: string) end
return M                    -- sealed: { version: number, greet: (n: string) -> () }
```

### 2.4.3 Contextual typing

When an expression appears where a type is expected, that type flows into the
expression:

- function expressions get parameter and return types from the expected
  function type; parameters past the expected ones take its rest's type, so
  each capture of `s:gsub("(%w+)=(%w+)", function(k, v) … end)` is a
  `string` (`(...string) -> any`, decision L78),
- table literals are checked against the expected record or array type, and
  nested literals get their field types; a literal whose elements all have
  the expected element type gets that type (`local xs: string[] = { "a" }` is a
  `string[]`, not a `"a"[]`, which matters because arrays are invariant in
  strict mode),
- a table literal for a union of table types is checked against the first
  member it fits, so literal fields can discriminate
  (`local c: Earned | Spent = { kind = "spent", amount = 1 }`); for a union
  with one table member (`string | string[]`), against that member,
- empty table literals take the expected type (`local xs: string[] = {}`);
  without one, an empty table after `or` or `??` stands in for the value
  before it and takes its type when it fits (a record whose fields are all
  optional, a list, a map): `local o = opts or {}` with `opts: Options?` is an
  `Options`, `list or {}` with `list: number[]?` a `number[]` (decision L52),
- generic calls use the expected return type to infer type arguments, and
  arguments for parameters that do not mention a type parameter are typed by
  those parameters.

Overloads with literal parameters let WoW APIs provide precise callback types:

```wowlua
-- in a declaration file
declare class Frame extends Region
    function SetScript(script: "OnUpdate", handler: ((self: Self, elapsed: number) -> ())?)
    function SetScript(script: "OnEvent", handler: ((self: Self, event: EventName, ...any) -> ())?)
end
```

### 2.4.4 Iteration

A generic `for` calls its iterator function with the state and control
values, as in Lua. The loop variables take the iterator's result types, with
`nil` removed from the first one (the loop ends when it is `nil`):

| Iterator expression | Loop variables |
|---------------------|----------------|
| `pairs(t)` with `t: { [K]: V }` | `K`, `V` |
| `pairs(t)` with a record `t` | union of the field-name literals, union of field types |
| `ipairs(t)` with `t: T[]` | `number`, `T` |
| `next, t` | as `pairs(t)` |
| any `f, s, c` where `f: (S, C) -> (C?, X...)` | `C`, then the types of `X...` |

`pairs`, `ipairs`, `next`, `select`, `unpack`, `pcall`, `xpcall`,
`securecallfunction` and `coroutine.*` are declared in the Lua standard library
declarations with type packs (§2.2.10). One is special-cased: `select(n, …)`
with a whole number literal `n` gives the types of the values from the n-th
on (`select(2, f())` with `f(): (number, string)` is a `string`, decision
L54); with another `n`, the values' type (`select(i, ...)` with `...: string`
gives `(string, ...string)`: the loop `for i = 1, select("#", ...)` reads each value as its type, decision L82).

### 2.4.5 String methods

Values of type `string`, including string literals in parentheses, index the
`string` library: `s:upper()` is typed as `string.upper(s)`, and
`("%d"):format(n)` as `string.format("%d", n)`. WoW's additional string
functions (`strsplit`, `strtrim`, …) are available as methods where WoW adds them
to the `string` table. In strict mode a name the `string` library doesn't
have (`s.message`, `s:nope()`) is E2003: Lua reads it as `nil` (decision
L71).

### 2.4.6 `setmetatable`

The classic Lua object pattern is understood (decision L31):

```lua
local Account = {}
Account.__index = Account
function Account.new(balance) return setmetatable({ balance = balance }, Account) end
function Account:deposit(v) self.balance = self.balance + v end
```

`setmetatable(t, mt)` returns the type of `t` extended by the **`__index`
fallback** of `mt`:

- if `mt` has an `__index` field whose type is a table type `I`, the result
  is `typeof(t)` with every field of `I` that `t` does not define readable as a
  fallback (methods included);
- chains (`setmetatable(Derived, { __index = Base })`) are followed;
- `__index` functions and other metamethods give `any` for unknown fields,
  and the checker does not try to evaluate them;
- `mt` is usually an open module-level table, so methods assigned to it later in
  the same module are part of its type. When `__index` names a table (`mt.__index
  = mt`, `mt.__index = Base`, `{ __index = Base }`), the result has every field
  the file stores in that table (methods defined after the call, fields its
  methods set on `self`), as `any` until they are defined, and no others: in
  strict mode a misspelled method or field is E2003.

In `function Account:deposit(v)`, `self` has the type of `Account`. Calls on
the result of `Account.new` therefore see `deposit`.

### 2.4.7 The addon namespace

At the top level of every file, `...` has the type `(string, AddonNamespace)`
(decision L32), matching WoW's `addonName, addonTable` arguments.

`AddonNamespace` is an ambient interface. Projects extend it with
`declare interface`. All declarations of the same ambient interface **merge**
(declaration merging). A field declared twice with incompatible types is error
E2054.

```wowlua
-- src/types.d.wowlua
declare interface AddonNamespace
    db: Settings
    Util: UtilModule
end

-- any file
local addonName, ns = ...
ns.db.scale = 2            -- checked against Settings
ns.somethingElse = true    -- undeclared field: any
```

The compiler's own declaration of `AddonNamespace` contains an indexer
`[string]: any`, so undeclared fields stay `any` in every mode and existing
code is unaffected. The key `__wowlua` is reserved for the runtime (E2003 on
assignment).

### 2.4.8 Coroutines

WoW's Lua includes the full `coroutine` library. It is commonly used to spread
work over several frames.

```wowlua
local co = coroutine.create(function(items: Item[]): number
    for i, item in ipairs(items) do
        process(item)
        if i % 50 == 0 then coroutine.yield(i) end
    end
    return #items
end)                                   -- Thread<(Item[]), (...any), (number)>

local ok, n = coroutine.resume(co, items)
```

- `coroutine.create<A..., R...>(f: (A...) -> R...): Thread<A..., (...any), R...>`
  types the start arguments and the final results.
- `coroutine.yield` is typed `(...any) -> ...any`, because the checker cannot
  connect a `yield` inside a function to the thread that runs it. A yield type
  can be fixed by annotating the variable: `local co: Thread<(Item[]), (number), (number)> = …`.
- `coroutine.wrap` returns `(A...) -> ...any` with the same limitation.
- **Lua 5.1 limitation:** a coroutine cannot yield across `pcall`, `xpcall`,
  metamethods, or C functions (for example inside a `table.sort` comparator or a
  `string.gsub` callback). Doing so raises "attempt to yield across
  metamethod/C-call boundary" at runtime. The checker reports W2057 when
  `coroutine.yield` appears lexically inside a function expression passed
  directly to one of these functions.

## 2.5 Narrowing

The checker narrows union types along control flow using:

- truthiness: `if x then`, `x and x.y`, `if not x then return end`; where
  an `unknown` is falsy it is `false | nil`, so `x and true or false` is a
  `boolean` (decision L63)
- nil comparison: `x ~= nil`, `x == nil`
- `type(x) == "string"` (and the other Lua type names); an `unknown` (or a
  member that is) narrowed to `"function"` is a function that takes and gives
  `unknown` (`(...unknown) -> ...unknown`), and to `"table"` a table of
  `unknown` keys and values (`{ [unknown]: unknown }`): what is inside stays
  `unknown` until it too is narrowed (decision L75). `any` narrows to the
  plain `function` and `table` types
- literal equality: `if kind == "spell" then` on discriminated unions; a
  comparison of a field, `if x.kind == "spell" then`, narrows `x` to the
  members whose `kind` can be that literal (and the false branch drops the
  members whose `kind` is exactly that literal). Comparing a local that holds
  the field (`local kind = x.kind`) does not narrow `x`.
- type predicates (§2.2.8), in both the true and the false branch, including
  `reflect.isInstance(x, Class)`, `issecretvalue` and `canaccessvalue`
- `assert(x)`, `assert(type(x) == "table")`, and `error(...)` / functions
  returning `never` (code after them is unreachable)
- early `return`, `break` and `continue`
- **existence checks of globals**: `if C_EditMode then` narrows the global
  `C_EditMode` to non-nil in the branch, also for globals the current flavor does
  not declare (see [Build metadata §6.5](06-meta.md#65-runtime-flavor-checks-shared-mode))

`assert(v)` also returns `v` without `nil` and `false`, so
`local p = assert(t.profiles)` is non-nil. Like Lua's, it returns every
argument it was given (`assert<T, A...>(value: T, ...: A...): (T, A...)`):
`return assert(x, "msg")` returns two values, which a function declared to
return one reports (E2001, too many values); write `return (assert(x, "msg"))`
(decision L38).

**Fields.** A field path of a local or of a declared global (`a.b`, `a.b.c`,
`G.x`) is narrowed like a local (a field of an intersection's part too:
`b.owner` of a `Button & { owner: Frame? }`, decision L81), and assigning it narrows it by the value's
`nil`-ness (to the declared type without `nil`, or to `nil`). Its narrowing
is invalidated by an assignment to the root, to the path or to a path above
it, and by a call that may modify the table holding it: one that receives the
root as an argument or as its receiver (`a:m()`), or a path above it (`a.b`
for `a.b.c`). A call that receives the path itself or a sibling keeps it: it
gets the field's table, not the one holding the field, so after `self.icon =
tex`, `self.icon:SetSize(1, 1)` leaves `self.icon` set (decision L41). Calls
that do not receive the table keep the narrowing. A method call on a narrowed optional
function field (`if t.f then t:f() end`) uses the narrowed type.

**Locals.** Narrowing of a local is invalidated by assignments to it. Assigning
a declared local narrows it to the members of its declared type that the value
fits (a literal stays a literal where the declared type is a literal union; a
record or interface stays as declared), except a declared `any`. A closure sees
the narrowing of the locals that are never assigned after their declaration (it
cannot change), except open tables, which are sealed for closures (§2.4.2);
other locals are their declared type inside closures (an unannotated one, the
union of §2.4.1). A local
narrowed to `nil` that a closure assigns is its declared type again after any
call, which may run that closure.

**Loops.** A local a loop body assigns may hold the body's values when the
loop starts again. A declared local keeps its narrowing at the loop start
(as the members of its declared type it fits: `0` in a `number?` is `number`)
when every path back to the start, the end of the body and `continue`, leaves
it within that narrowing. An unannotated local is its type before the loop
at the loop start when the paths back bring it no other type; when they do,
the loop is checked again with the union of that type and theirs, which then
holds when the paths back bring nothing more. The `break` paths of the locals
kept so join the flow after the loop: `local found = nil` that a search loop
sets to an index is a `number?` after it. Otherwise (a declared local that
leaves its narrowing, an unannotated one whose union still grows, or one a
closure assigns) the loop body sees the declared type (`any` for an
unannotated local without a value), decision L65.

**Fields through a narrowed local.** Assigning a field of an annotated local
(`item.x = 1`) checks against the members of its declared type that its
narrowed value fits: inside `if item then`, an `Item | false` local is an
`Item`.

## 2.6 Assignability

`S` is assignable to `T` when:

- either is `any`, or `T` is `unknown`, or `S` is `never`;
- they are the same primitive, or `S` is a literal of primitive `T`;
- `S` is `nil` and the mode is `basic` (nil is assignable to everything in basic mode);
- `T` is a union containing a member `S` is assignable to, or `S` is a union whose
  members are all assignable to `T`;
- `T` is an intersection and `S` is assignable to each of its parts, or `S` is
  an intersection one of whose parts is assignable to `T` (not an
  intersection): `A & B & C` fits `A & B`, `C & B` and `B` (decision L74);
- both are records and `S` has every required field of `T` with an assignable
  type (fields are compared covariantly; readonly-ness is ignored);
- both are arrays or maps with assignable element types (variance as in §2.2.6);
- both are functions: parameters compare bivariantly in basic mode and
  contravariantly in strict mode. Returns compare covariantly. `S` may take
  fewer parameters than `T`. Type packs compare element by element.
- both are instantiations of the same generic class, interface or alias, and the
  type arguments satisfy the variance rules (§2.2.6);
- `S` is an instance of class `C` and `T` is `C` or a superclass of `C`
  (classes are **nominal**; see [Classes](03-classes.md#318-typing));
- `S` is an instance of a class and `T` is a **record or interface** that `S`
  structurally satisfies through its **public** fields, accessors and methods;
- `S` is a record and `T` is an interface that `S` structurally satisfies
  (interfaces are **structural**);
- `S` is an enum member type and `T` is that enum or its underlying primitive;
- `S` is `table` and `T` is a map whose values are `any` (`{ [any]: any }`,
  `{ [string]: any }`, `{ [number]: any }`), or `S` is `function` and `T` is
  `(...any) -> ...any` (the types every table and every function have). A map
  with typed values (`{ [string]: number }`) needs a cast;
- `S` is an array of `E` and `T` is a record whose named fields are all
  optional and whose indexer takes numbers as keys and `E` as values
  (`string[]` fits `{ [number]: any, talent?: boolean }`); a record without an
  indexer takes an array only beside an array type in an intersection
  (`string[]` fits `any[] & { jewel?: boolean }`, not `{ name?: string }`);
- `S` is a record with a numeric indexer (`{ [number]: V, n: number }`) and `T`
  is an array of `V`'s supertype: the table used both as an array and a
  record (§2.2.4).

## 2.7 Calls and arity

| Situation | basic | strict |
|-----------|-------|--------|
| Too many arguments to a non-variadic **annotated** function | error E2006 | error E2006 |
| Too many arguments to an unannotated function | warning W2055 | error E2006 |
| Missing argument for a **non-optional parameter** (type without `?` or `nil`, no default) | error E2007 | error E2007 |
| Calling a value whose type is not callable | error E2004 | error E2004 |
| Calling a method with `.` instead of `:` (missing `self`) | error E2008 | error E2008 |

In basic mode `nil` is assignable to every type, so passing an explicit `nil`
is accepted, but **omitting** a non-optional argument is still E2007. A
function is **unannotated** when none of its parameters and not its return
type are annotated; its parameters are then all optional.

**Overload resolution** tries overloads in declaration order and picks the
first whose parameters accept the arguments. Function-expression arguments
are resolved in two passes:

1. Overloads are filtered using the non-function arguments only. A function
   expression matches any function type with compatible arity.
2. The function expressions are contextually typed against the first remaining
   overload and checked.

A type parameter that only a function expression's results can bind (`R` in
`call<R>(f: () -> R): R`, `R...` in `pcall`) is bound from what the
expression returns when it has no return annotation:
`pcall(function() return "x" end)` gives `(boolean, string)` (decision L55).
`pcall` and `xpcall` give `unknown` after the boolean where the function never
returns or returns nothing, since then the values are only what it raised,
which may be anything (decision L75). `pcall(f, ...)` of an overloaded `f`
(`CreateFrame`) gives the results of the overload the arguments match:
`pcall(CreateFrame, "ModelScene")` gives `(boolean, ModelScene)` (decision L82).

These are the results where the function returns. Where it raises, `pcall`
gives `false` and the error, which is `unknown` (`xpcall`: what its handler
returns, `unknown` where that is not known). So the call gives `boolean`,
then the first result or the error (`unknown` when the error is), then the
other results or `nil` (`R?`). A `local ok, a, b = pcall(…)` (or the
assignment `ok, a, b = pcall(…)` to locals) keeps the two outcomes apart, as
a discriminated union does: where a condition narrows `ok` to `true`
(`if ok then`, the code after `if not ok then return end`), `a` and `b` are
the results; where it narrows `ok` to `false`, `a` is the error and `b` is
`nil`. Assigning any of the locals ends this (decision L101).

```wowlua
local ok, frame = pcall(CreateFrame, "ModelScene")
if ok then
    frame:Show()                    -- frame: ModelScene
else
    print(tostring(frame))          -- frame: unknown, the error
end
```

A last argument that gives several values (a call such as `unpack(t)`, or
`...`) gives its values to the parameters from its position on, as in Lua:
`tex:SetTexCoord(unpack(coords))` matches the four-number overload when
`coords` is a `number[]`. Values it gives in any number fill as many of the
remaining parameters as there are (decision L40).

When no overload matches, the error (E2012) lists every overload.

## 2.8 Globals

The global environment (`_G`) is modelled as a set of **declared globals**:

- the WoW API (from the `wow-api-<flavor>` declarations of the current flavor,
  embedded in the compiler until the package manager provides them as types
  packages; see [Package manager](../toolchain/package-manager.md#109-types-packages)
  and decision T83). A function is declared for a flavor only when that
  flavor's client has it (decision T85). A class the annotations give a data
  class and a mixin as parents (`ItemLocation : ItemLocationData,
  ItemLocationMixin`) extends the mixin, what the API's functions take, and
  has the data's fields (decision T284),
- the globals of Blizzard's UI code (FrameXML functions, tables, mixins and
  named frames) and its global strings (decision T84). A named frame of the
  XML is typed by its mixins and its widget type, the most specific first
  (`ChatFrame1: PrimaryChatFrameMixin & FloatingChatFrameMixin & … & Frame`):
  the mixins of its `mixin` and `secureMixin` attributes, then those of the
  templates it inherits, in order, and of its intrinsic type
  (`<ScrollingMessageFrame>`). A member is looked up in the parts in order, so
  a mixin's method comes before the widget's, as in the client. These mixins
  are declared as classes with the functions their Lua defines (`function
  M:F(a, b)` a method, `function M.F(a)` a function field); the parameters
  are untyped (`any`) and the results any values, except a message frame's
  font and message methods (`FontableFrameMixin`, `ScrollingMessageFrameMixin`
  and its secure part: `SetFont`, `AddMessage`, `GetScrollOffset`, …), typed
  as the client's `MessageFrame` has them, a line and its colour possibly
  secret. A frame type the client makes from Blizzard's Lua, not as a widget of
  its own, is its intrinsic's mixin on a frame: `CreateFrame("ScrollingMessageFrame")`
  gives a `ScrollingMessageFrameMixin & Frame` (decision T285).
  `CreateFrame(frameType, …)` gives the class of each frame type the flavor
  has, also those only some flavors document (`CreateFrame("Browser")` is a
  `Browser`; decision T292). A mixin made with
  `CreateFromMixins(A, B)` extends `A` and has `B`'s functions,
- the Lua 5.1 standard library as available in WoW, and `LibStub`
  (decision T86). WoW has no `os`, `io`, `debug` or `package` library; it
  keeps `date`, `time` and `difftime` as globals (`date("*t")` and
  `date("!*t")` give a `DateTable` of the time's parts, any other format a
  `string`, decision L72). Its additions to `string`
  (`startswith`, `endswith`, `contains`, `ltrim`, `rtrim`, `concat`, …),
  `table` (`contains`, `indexof`, `keys`, `values`, `count`, `isempty`,
  `removevalue`, `freeze`, …) and `math` (`clamp`, `lerp`, `round`, `wrap`,
  `saturate`, …) are declared for every flavor (decision S37). Reading `os`
  reports W1001/E1001 with the replacements (`time()`, `date(…)`,
  `difftime(…)`, `debugprofilestop()`),
- `declare global` declarations in the project and its dependencies,
- SavedVariables listed in the manifest (typed `any` unless declared),
- names listed in the manifest under `check.globals`.

**WoW's arithmetic** differs from Lua 5.1 elsewhere (probed on WoW: Forever,
decision S38): `x / 0` and `x % 0` raise "Division by zero" at run time
instead of giving inf or nan, `math.sqrt` and `math.log` raise on a negative
argument, and `string.format("%d", x)` raises for an infinite `x`. Infinity is
`math.huge`; NaN is `math.huge - math.huge` (or `math.fmod(x, 0)`). Division
or modulo by a literal zero (or a `const` of it) is warning W2060 in every
mode; a divisor that is zero only at run time cannot be seen and needs a
guard (`if max > 0 then … end`).

**`a and nil or b`** (or `false` in the middle) is always `b`: Lua's
`and`/`or` choice cannot give a falsy middle value, the `or` takes over. It
is warning W2061 in every mode (decision S39); write `not a and b or nil`
when `b` is never falsy, or an `if`.

**A value read past what a function gives** is always `nil`. For a
function of the package whose every `return` gives a fixed number of
values (none ends in `...` or in a call that gives an open number), the
most it gives is known, whatever its declaration says (`...any` included);
a name past that in a `local` or an assignment whose last value is its
call, or `select(n, …)` with a literal `n` past it, is warning W2062 in
every mode (decisions S40, S41). The functions counted: a `local
function`, a module's `function f()`, one imported from another module
(`import f from "./m"`, `import internal`), and the methods of a
file-level table (`function M:f()`, `function M.f()`), called as `M:f()`,
`M.f()` or `self:f()` in `M`'s methods, wherever in the file they are
defined. A function or method assigned again loses its count; the API's
declared functions are not counted.
A name starting with `_` takes no value it wants (as for W1003) and is
skipped: the warning names the first other one.
A helper meant to pass every value on returns them through `...`
(`local function okThen(ok, ...) if ok then return ... end end`).
An `and` or `or` gives one value, so a name past an `and` or `or` of a call
that ends a `local` or an assignment (`local x, y = pos and
pos:GetXY()`) is W2062 too, whatever the call (decision L73): write
`if pos then x, y = pos:GetXY() end`.

Reading an undeclared global is warning W1001 in basic mode and error E1001
in strict mode. **Assigning** an undeclared global, including through
`function foo()` in a script, is warning W1002 in basic mode and error E1002 in
strict mode. In pure Lua files both are always warnings (§2.1). Accidental
globals are a common source of addon bugs and taint. Explicit
`_G.name = value` or `_G["name"]` is always allowed.

A global that any file of the package assigns can be read everywhere (typed
`any`); the assignment itself is still W1002/E1002 unless declared. A file
that sets the environment of its own code gets no global diagnostics, because
its names resolve in an environment the checker cannot see: a `setfenv` with a
stack level or with a function the file defines. Setting the environment of
other code (`setfenv(chunk, env)` for a chunk from `loadstring`) keeps them. Existence tests (`if GetPetHappiness
then … end`) never report the tested name, and the guarded code may use it
(feature detection, [Build metadata §6.5](06-meta.md#65-runtime-flavor-checks-shared-mode),
decision T89).

## 2.9 Declaration files

`.d.wowlua` files contain only declarations. They produce no code. They are
how the WoW API, plain Lua libraries and other addons are typed.

```wowlua
-- types/wow.d.wowlua (excerpt)
declare function GetTime(): number
declare function UnitName(unit: UnitToken): (string?, string?)
declare function hooksecurefunc(name: string, hook: (...any) -> ())
declare function hooksecurefunc<T>(table: T, name: string, hook: (...any) -> ())

declare global SlashCmdList: { [string]: (msg: string, editBox: EditBox) -> () }
declare global UIParent: Frame

declare namespace C_Timer
    function After(seconds: number, callback: () -> ())
    function NewTicker(seconds: number, callback: (ticker: TickerHandle) -> (), iterations: number?): TickerHandle
end

@widget("Frame")
declare class Frame extends Region
    function RegisterEvent(event: EventName)
    function Show()
    function SetScript(script: "OnUpdate", handler: ((self: Self, elapsed: number) -> ())?)
end

declare event PLAYER_LOGIN()
declare event PLAYER_ENTERING_WORLD(isInitialLogin: boolean, isReloadingUi: boolean)
declare event UNIT_HEALTH(unitTarget: UnitToken)

declare interface AddonNamespace       -- merges with every other declaration
    db: Settings
end

declare module "/legacy/util"           -- types for a plain-Lua module, see Modules
    function round(n: number, decimals: number?): number
end
```

```wowlua
-- src/settings.wowlua: an addon-internal message, declared next to its sender
declare message MYADDON_SETTINGS_CHANGED(key: string, value: any)
```

| Form | Declares |
|------|----------|
| `declare function f(...)` | A global function. Repeated declarations form an **overload set**. |
| `declare global name: T` | A global variable. |
| `declare namespace N … end` | A global table of functions and fields, called with `.`. Namespaces may nest. |
| `declare class C … end` | A native or external class (e.g. widget types). `function` members are methods (`:`), and `static function` members are called with `.`. |
| `declare interface I … end` | An ambient interface. All declarations with the same name merge (§2.4.7). |
| `declare event NAME(params)` | A **WoW event** and its payload. It feeds `EventName` and `EventArgs<E>`. Only events declared this way are registered with the game (`frame:RegisterEvent`). |
| `declare message NAME(params)` | An **addon-internal message** (decision S2). It feeds `MessageName` and `EventArgs<E>`. Messages are dispatched inside the addon and never registered with the game. |
| `declare module "spec" … end` | The exports of a module implemented in plain Lua. |

Event and message names share one namespace. Declaring the same name as both
is error E2058. Parameters in `declare event` and `declare message` cannot have
default values (E2059).

`declare` statements may also appear at the top level of `.wowlua` files, e.g.
to declare a message next to the code that sends it. Declarations are global to
the project. Flavor-specific declarations use `@if` (see
[Build metadata](06-meta.md#643-conditional-declarations)).

## 2.10 Type erasure

All type syntax is removed during lowering: annotations, `type`, `interface`,
`declare`, generic parameters, `::` assertions and type-only imports.
Interfaces and type aliases produce no code. Classes and enums produce code
(they are values). The exceptions are:

- **runtime checks**, inserted for checkable types when enabled by the profile
  ([Debugging](07-debugging.md#75-runtime-type-checks)), and
- **reflection metadata**, which keeps type names as strings when enabled
  ([Debugging](07-debugging.md#78-reflection)).

## 2.11 Secret values

Since patch 12.0 (Midnight), many WoW APIs return **secret values** to addon
(tainted) code during combat, boss encounters, Mythic+ and other restricted
situations. A secret can be stored, passed around, concatenated, formatted,
passed to `type()`, and handed to widget setters that accept secrets
(`FontString:SetText`, `StatusBar:SetValue`, …). Arithmetic, comparison, `#`,
indexing, calling, use as a table key, and boolean tests on secret booleans
**raise an error immediately**.

wowlua models this with the type `Secret<T>`:

| Rule | Detail |
|------|--------|
| Sources | API return values and event payload fields that Blizzard's API documentation marks as possibly secret are typed `T \| Secret<T>` by the generated `wow-api` types, for the flavors where restrictions apply. A documented structure such a function returns is a readable table whose fields may be secret, as the client answers: the function returns the structure's companion `<Name>Secret` (`C_Spell.GetSpellCooldown` returns `SpellCooldownInfoSecret?`), whose fields are `T \| Secret<T>` except those documented `NeverSecret`, and whose lists of structures hold companions. Functions that never answer secrets keep the plain structure (`C_Spell.GetItemCooldown` returns `SpellCooldownInfo?`). |
| Allowed operations | Assignment, passing as an argument where the parameter accepts `Secret<T>`, `..` (the result is `string \| Secret<string>`), `string.format`, `tostring`, `type()`, truthiness of non-boolean secrets |
| Errors | Arithmetic, comparison, `#`, indexing, calling, use as a table key, or a boolean test on `Secret<boolean>`: error E2040 in strict mode and warning W2040 in basic mode and in pure Lua files |
| Narrowing | `issecretvalue(v)` is declared as the predicate `v is Secret<unknown>`, and the global `canaccessvalue(v)` narrows as its complement (declared returning `boolean`; a local of that name does not narrow). The false branch of `issecretvalue` and the true branch of `canaccessvalue` remove all `Secret<…>` members (§2.2.8); the true branch of `issecretvalue` and the false branch of `canaccessvalue` keep only them. |
| Widget setters | Declared to accept `T \| Secret<T>` where Blizzard documents secret support, so values can be displayed without being read |
| Widget getters | Where Blizzard documents their results as possibly secret (`SecretReturns`, or `SecretReturnsForAspect`: a widget's aspect a setter was given a secret for reads back secret, `FontString:GetText()` after a secret `SetText`), declared to return `AspectSecret<T, T \| Secret<T>>`: `T`, or `T \| Secret<T>` with the manifest's `check.secret-widget-results` (§9.8). (`StatusBar:GetValue()` after a secret `SetValue` among them). The checker can't follow which widget got a secret; the emulated client returns one where the game does (decision T474) |
| Assignability | `Secret<T>` is not assignable to `T`. `T` is assignable to `T \| Secret<T>`. |
| Erasure | Purely static. No runtime cost. |

```wowlua
local hp = UnitHealth("target")             -- number | Secret<number>
bar:SetValue(hp)                            -- ok: SetValue accepts secrets
if hp < 1000 then … end                     -- E2040: comparison on a possibly secret value
if canaccessvalue(hp) and hp < 1000 then    -- ok: narrowed to number
    …
end
```

Generated code (runtime checks, profiling, tracing, `dump`) never performs
forbidden operations on values whose static type may be secret. Trace and dump
output print `<secret>` for them.
