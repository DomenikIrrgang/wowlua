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

### 2.2.4 Table types

| Syntax | Meaning |
|--------|---------|
| `T[]` or `{T}` | Array with elements `T` (1-based, sequence) |
| `{ [K]: V }` | Map from `K` to `V` |
| `{ x: number, y: number }` | Record. `,` or `;` separate fields. |
| `{ name: string, label?: string }` | `label` is optional: `string?`, may be absent |
| `{ ["end"]: number, ["my-key"]: string }` | Record fields whose keys are reserved words or not identifiers |
| `{ x: number, [string]: any }` | Record with an indexer for other keys |
| `{ [number]: string, n: number }` | Table used both as an array and a record (e.g. the result of `{ ..., n = select("#", ...) }`) |

Reading an element of an array (`xs[i]`) gives `T` in every mode, although
reading past the end gives `nil` at run time (decision T95). Reads through a
map or an indexer give `V?`.

Records are **structural**: any table with at least the required fields of
compatible types is assignable. Extra fields are allowed, except that an
object literal assigned directly to a record type with an unknown field is
an error in strict mode (E2003), to catch typos.

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
`local s: Stack<number> = Stack.new()`.

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

### 2.2.8 Type predicates

A function whose return type is `param is T` narrows its argument in the
caller:

- where the call is **truthy**, the argument is narrowed to `T` (intersected
  with its current type);
- where the call is **falsy** (`else` branch, `if not p(x)`, after
  `if p(x) then return end`), the argument is narrowed by **excluding** the
  members of its union type that are assignable to `T`. If the argument's type
  is not a union, it is unchanged.

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
| `Self` | Inside class and interface bodies: the type of the receiver. Used for fluent APIs and in declarations (`function SetScript(...): Self`). |
| `ClassOf<T>` | The class value (constructor table) of instance type `T`. Calling `new` through `ClassOf<T>` is an error if `T` is abstract (E3002). |
| `EventName` | Union of all **WoW event** names (`declare event`). |
| `MessageName` | Union of all **addon message** names (`declare message`). |
| `EventArgs<E>` | Type pack of the payload of event or message `E`. Used by `wowlua:events`. |
| `Thread<A..., Y..., R...>` | A typed coroutine: started with arguments `A`, yields `Y`, returns `R` (§2.4.8). |
| `Secret<T>` | A Midnight-era *secret value* of underlying type `T` (§2.11). |
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
- Interfaces and classes are named types and may refer to themselves and to each
  other freely, including across modules through `import type`, which never
  creates a load-order dependency.

## 2.4 Inference

Inference is local and bidirectional. There is no whole-program inference.

| Construct | Inferred type |
|-----------|---------------|
| `local x = 5` | declared type `number` (literal widened) |
| `const X = 5` | `5` (literal kept) |
| `local t = { a = 1, b = "x" }` | `{ a: number, b: string }` |
| `local t = { 1, 2, 3 }` | `number[]` |
| `local t = {}` | open table, see below |
| function without return annotation | union of all returned types (`()` if none) |
| unannotated parameter | `any` (basic/strict), or the contextual type if available |

### 2.4.1 Unannotated locals

An unannotated local's **declared type** is the union of the types of all
values assigned to it within its scope. At each use, its **narrowed type** is
the type of the most recent assignment on that control-flow path. This keeps
valid Lua patterns valid, and a local may hold different types over time,
while still catching misuse such as calling `:upper()` on a value that is a
number at that point.

A local declared without a value and assigned later (a forward declaration,
`local f` … `f = function() … end`) reads as `any` inside closures, which may
run after any of its assignments.

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
without an expected type. A field added as `nil` (`t.mode = nil`) is optional
and `any`.

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
  function type,
- table literals are checked against the expected record or array type, and
  nested literals get their field types; a literal whose elements all have
  the expected element type gets that type (`local xs: string[] = { "a" }` is a
  `string[]`, not a `"a"[]`, which matters because arrays are invariant in
  strict mode),
- a table literal for a union of table types is checked against the first
  member it fits, so literal fields can discriminate
  (`local c: Earned | Spent = { kind = "spent", amount = 1 }`); for a union
  with one table member (`string | string[]`), against that member,
- empty table literals take the expected type (`local xs: string[] = {}`),
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
declarations with type packs (§2.2.10). They are not special-cased.

### 2.4.5 String methods

Values of type `string`, including string literals in parentheses, index the
`string` library: `s:upper()` is typed as `string.upper(s)`, and
`("%d"):format(n)` as `string.format("%d", n)`. WoW's additional string
functions (`strsplit`, `strtrim`, …) are available as methods where WoW adds them
to the `string` table.

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
  the same module are part of its type.

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

- truthiness: `if x then`, `x and x.y`, `if not x then return end`
- nil comparison: `x ~= nil`, `x == nil`
- `type(x) == "string"` (and the other Lua type names)
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
`local p = assert(t.profiles)` is non-nil.

**Fields.** A field path of a local or of a declared global (`a.b`, `a.b.c`,
`G.x`) is narrowed like a local, and assigning it narrows it by the value's
`nil`-ness (to the declared type without `nil`, or to `nil`). Its narrowing
is invalidated by an assignment to the root, to the path or to a path above
it, and by a call that may modify the table: one that receives the root, or a
path below it, as an argument or as its receiver (`a:m()`). Calls that do not
receive the table keep the narrowing. A method call on a narrowed optional
function field (`if t.f then t:f() end`) uses the narrowed type.

**Locals.** Narrowing of a local is invalidated by assignments to it. Assigning
a declared local narrows it to the members of its declared type that the value
fits (a literal stays a literal where the declared type is a literal union; a
record or interface stays as declared), except a declared `any`. A closure sees
the narrowing of the locals that are never assigned after their declaration (it
cannot change), except open tables, which are sealed for closures (§2.4.2);
other locals are their declared type inside closures. A local
narrowed to `nil` that a closure assigns is its declared type again after any
call, which may run that closure.

## 2.6 Assignability

`S` is assignable to `T` when:

- either is `any`, or `T` is `unknown`, or `S` is `never`;
- they are the same primitive, or `S` is a literal of primitive `T`;
- `S` is `nil` and the mode is `basic` (nil is assignable to everything in basic mode);
- `T` is a union containing a member `S` is assignable to, or `S` is a union whose
  members are all assignable to `T`;
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
- `S` is `table` and `T` is `{ [any]: any }`, or `S` is `function` and `T` is
  `(...any) -> ...any` (the types every table and every function have);
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

When no overload matches, the error (E2012) lists every overload.

## 2.8 Globals

The global environment (`_G`) is modelled as a set of **declared globals**:

- the WoW API (from the `wow-api-<flavor>` declarations of the current flavor,
  embedded in the compiler until the package manager provides them as types
  packages; see [Package manager](../toolchain/package-manager.md#109-types-packages)
  and decision T83). A function is declared for a flavor only when that
  flavor's client has it (decision T85),
- the globals of Blizzard's UI code (FrameXML functions, tables, mixins and
  named frames) and its global strings (decision T84),
- the Lua 5.1 standard library as available in WoW, and `LibStub`
  (decision T86),
- `declare global` declarations in the project and its dependencies,
- SavedVariables listed in the manifest (typed `any` unless declared),
- names listed in the manifest under `check.globals`.

Reading an undeclared global is warning W1001 in basic mode and error E1001
in strict mode. **Assigning** an undeclared global, including through
`function foo()` in a module, is warning W1002 in basic mode and error E1002 in
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
| Sources | API return values and event payload fields that Blizzard's API documentation marks as possibly secret are typed `T \| Secret<T>` by the generated `wow-api` types, for the flavors where restrictions apply. |
| Allowed operations | Assignment, passing as an argument where the parameter accepts `Secret<T>`, `..` (the result is `string \| Secret<string>`), `string.format`, `tostring`, `type()`, truthiness of non-boolean secrets |
| Errors | Arithmetic, comparison, `#`, indexing, calling, use as a table key, or a boolean test on `Secret<boolean>`: error E2040 in strict mode and warning W2040 in basic mode and in pure Lua files |
| Narrowing | `issecretvalue(v)` is declared as the predicate `v is Secret<unknown>`, and `canaccessvalue(v)` as its complement. The false branch of `issecretvalue` and the true branch of `canaccessvalue` remove all `Secret<…>` members (§2.2.8). |
| Widget setters | Declared to accept `T \| Secret<T>` where Blizzard documents secret support, so values can be displayed without being read |
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
