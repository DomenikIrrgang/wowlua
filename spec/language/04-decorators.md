# 4. Decorators

Decorators attach behavior or metadata to declarations. There are three
kinds:

| Kind | Where defined | Evaluated |
|------|---------------|-----------|
| **Core decorators** | The language; always in scope | At compile time. They transform or remove code. |
| **Framework decorators** | Exports of `wowlua:*` modules; must be imported | At compile time. They generate wrapping or registration code. |
| **User decorators** | Any function value | At load time. Plain function application. |

## 4.1 Syntax

```wowlua
@name
@name(args)
@name "string"          -- Lua call sugar
@name { key = value }   -- Lua call sugar
@a.b.c(args)            -- dotted path
```

- A decorator is `@` followed by a dotted name and optional call arguments. The
  arguments use the same forms as a Lua call (`(…)`, a string literal, a
  table constructor), and they **must start on the same line** as the name.
- Any number of decorators may precede a declaration, on separate lines or the
  same line. On exported declarations, decorators come before `export`
  (`@profile export function f() … end`).
- `@` is not a Lua token, so decorators never conflict with Lua code.

Decorators may be applied to:

| Target | Allowed decorators |
|--------|--------------------|
| Function declarations (`function f`, `local function f`, `function t.f`, `function t:m`) | all |
| Classes | all |
| Class methods, constructors, accessors and fields | all applicable (fields: no wrapping decorators) |
| Enums | core only: `@deprecated`, `@if`, `@global`, `@reflect` |
| Interfaces, type aliases, `declare` statements | core only: `@deprecated`, `@if` (plus `@widget`, `@intrinsic`, `@onDemand` and, on parameters, `@deferred` where documented) |
| `const` / `local` declarations | `@if`, `@global`, `@deprecated` |
| `import` statements | `@if` |

A decorator that is not applicable to its target is error E3021. User
decorators on enums are not allowed because enum members are inlined at
compile time, so a replacement value could never take effect. Interfaces and
aliases have no runtime value.

## 4.2 Name resolution

A decorator name is resolved as follows (decision L22):

1. If the path is a single name that names a **core decorator**, the core
   decorator is used, **always**, regardless of lexical bindings.
2. If the first segment is `_G`, the rest of the path is looked up in the global
   environment at load time (`@_G.MyLib.memoize`).
3. Otherwise, if the first segment is a binding in lexical scope (local, import,
   parameter), the decorator refers to that binding.
4. Otherwise it is an error (E3020). Decorators never resolve to globals
   implicitly.

A user function whose name equals a core decorator can only be used as a
decorator through an alias (`import profile as myProfile from "./perf"`).
Warning W3021 is reported when a lexical binding with a core decorator's name
is a function that is **not** a re-export of that core decorator, because the
binding is ignored in decorator position. It is reported at each decorator
that names it (`@profile`); a binding no decorator names gets none. Re-exports such as
`import profile from "wowlua:debug"` and namespace imports such as
`import * as reflect from "wowlua:reflect"` produce no warning.

## 4.3 Core decorators

Always available without import.

| Decorator | Targets | Effect |
|-----------|---------|--------|
| `@deprecated(msg?)` | any declaration | Warning W3030 at every use site. In debug builds, a one-time runtime warning on first use. |
| `@global(name?)` | functions, classes, `local`/`const`, enums | Also assigns the **final** (fully decorated) value to global `name`, which defaults to the declaration's name. The sanctioned way to create globals for XML scripts, key bindings and other addons. The global is declared for the type checker with the declaration's type: reading it in another module of the addon, or in an addon that lists this one in `addon-dependencies`, gives the type of its value (decision L83; a module that reads it is checked after the one declaring it, unless they import each other in a cycle, where it reads as `any`). A later reassignment of a `local` does not update the global. |
| `@if(condition)` | any declaration, `import` | Conditional compilation. The declaration is removed when the compile-time condition is false. See [Build metadata](06-meta.md#643-conditional-declarations). |
| `@script(name, opts?)` | instance methods of widget classes | The method handles the frame script `name` (`SetScript`; `{ hook = true }`: `HookScript`) on every frame of the class ([Classes §3.2.1](03-classes.md#321-script-handlers)). |
| `@inline` | functions, methods | Hint to inline at call sites in optimized builds (`optimize.inline`). Local functions and private methods whose body is a single `return` expression are inlined, and those whose body is statements (without an early `return`) where a call is a statement of its own: a private method where its class calls it (`obj:m(…)` on any instance, `C.m(…)` for a static one), as nothing outside the class can call or replace it. On any other function or method (a public or protected one can be overridden), or on a recursive or escaping function, it is ignored with W3031 ([Optimization §15.2](../toolchain/optimization.md#152-profile-controlled-optimizations)). |
| `@reflect(opts?)` | classes, enums, functions | Emit full reflection metadata. See [Debugging](07-debugging.md#78-reflection). |
| `@profile(opts?)` | functions, methods, constructors, accessors, classes (all methods) | Measure call count and time. Removed unless the profile enables instrumentation. See [Debugging](07-debugging.md#73-profiling). |
| `@noprofile` | functions, methods, classes | Exclude from `instrument = "all"`. |
| `@trace(opts?)` | functions, methods, constructors, classes | Log entry, arguments, return values and errors at `trace` level. Removed when `trace` logging is compiled out. |
| `@debugOnly` | functions, methods | In non-debug builds, removes the declaration and every call statement to it. The function must return nothing, and calls must be statements (E3032). A method becomes a no-op in non-debug builds, because calls through `obj:m()` cannot be found without types (decision T81). |
| `@checked` / `@unchecked` | functions, methods, constructors | Force or suppress runtime parameter type checks regardless of profile. |
| `@nocheck` | functions, methods, classes | Skip static type checking of the body. |
| `@widget(type)` | `declare class` only | Marks a native widget type for widget classes ([Classes](03-classes.md#32-widget-classes)). Used by type packages. |
| `@intrinsic(id)` | declarations in compiler-known modules only | Binds a declaration to compiler behavior. Not for user code (E3033). |
| `@onDemand(addon)` | `declare global` and `declare function` only | The global exists only once Blizzard's load-on-demand addon `addon` has loaded. Hooks of it without `opts.addon` are W8102. Generated into the `wow-api` declarations. |
| `@deferred` | callback parameters in `declare` signatures and interface method signatures | Marks a callback that the API (or every implementation of the interface) invokes later, not during the call (e.g. `C_Timer.After`, an event bus's `on`). Used by load-order analysis ([Modules §5.4.3](05-modules.md#543-load-time-and-run-time-edges)). |

## 4.4 Framework decorators

Framework decorators are exports of standard-library modules and require an
import, for example:

```wowlua
import event from "wowlua:events"
import slash from "wowlua:slash"
import outOfCombat from "wowlua:combat"

@outOfCombat
local function applyPendingLayout() … end

@event("PLAYER_REGEN_ENABLED")
local function onCombatEnd()
    applyPendingLayout()
end

@slash("/mya", "/myaddon")
local function onCommand(msg: string) … end
```

They are declared with `@intrinsic` in the module's declaration file, so the
compiler can check their arguments (for example, the event name exists and the
handler's parameters match `EventArgs<E>`) and generate code. There are two
groups:

| Group | Examples | Behavior |
|-------|----------|----------|
| **Wrapping** | `@throttle`, `@debounce`, `@nextFrame`, `@outOfCombat` | Replace the function with a wrapper, like a user decorator, in source position |
| **Registering** | `@event`, `@unitEvent`, `@slash`, `@hook`, `@hookScript`, `@onLoad`, `@onLogin`, `@test` | Register the **final** function value with the game or the runtime |

At run time, a framework decorator is a function exported by its module, with
the calling convention of user decorators. Wrapping decorators are applied
like user decorators; registering decorators are called as
`decorator(final, ctx)` after every transformation, in source order, and their
results are ignored (`__wl.register`, decision T79).

The full list is in the [Standard library](../stdlib/README.md#83-decorator-index).

## 4.5 User decorators

A user decorator is any function with a decorator signature. It runs **once,
at load time**, when the decorated declaration is evaluated.

```wowlua
type DecoratorContext = {
    kind: "function" | "method" | "constructor" | "getter" | "setter" | "class" | "field",
    name: string,
    static: boolean,              -- methods, accessors and fields
    class: table?,                -- class members: the class table
    file: string,                 -- source path, e.g. "src/core/cache.wowlua"
    line: number,
}

type FunctionDecorator<F> = (fn: F, ctx: DecoratorContext) -> F?
type ClassDecorator<C>    = (cls: C, ctx: DecoratorContext) -> C?
type FieldDecorator       = (target: nil, ctx: DecoratorContext) -> any
```

- **Function, method, constructor and accessor decorators** receive the function
  and return a replacement, or `nil` to keep the original.
- **Class decorators** run after the class is finalized and its static fields are
  initialized, and may return a replacement class table.
- **Field decorators** receive `(nil, ctx)`, because a field has no value at
  decoration time. Their return value is ignored (decision L8). They are for
  registration and metadata, e.g. registering serializable fields.
- **Decorator factories** are functions that return a decorator. `@cached(64)`
  calls `cached(64)` and applies the result.

```wowlua
local function memoize<A, R>(fn: (A) -> R, ctx: DecoratorContext): (A) -> R
    local cache = setmetatable({}, { __mode = "k" })
    return function(a: A): R
        local v = cache[a]
        if v == nil then v = fn(a); cache[a] = v end
        return v
    end
end

local function cached(size: number): FunctionDecorator<(number) -> ItemInfo>
    return function(fn, ctx) … end            -- LRU cache with `size` entries
end

@memoize
local function heavyLookup(id: number): ItemInfo … end

@cached(64)
local function itemInfo(id: number): ItemInfo … end
```

### 4.5.1 Typing

The checker verifies that a user decorator's parameter types accept the target
and the context (E3022). When the decorator returns a function, the declared
name gets the **return type of the decorator**, which allows decorators that
change signatures.

- A decorator's expression is checked where the declaration is, as load time
  evaluates it (§4.5.2): a factory's arguments against its parameters, and
  a name in them that is neither in scope there nor a declared global is
  E1003, in every mode. Function bodies in the arguments run later and are
  checked as such.
- The target is the function as written; a method's (and a constructor's or
  accessor's) function takes `self` first, a class decorator receives the
  class table and a field decorator `nil`. Type parameters of a generic
  decorator are inferred from the target and the context. The second
  parameter must take a `DecoratorContext`; parameters after it must be
  optional.
- Decorators apply bottom to top, each to the result of the one below it.
  A result of `nil`, `any` or anything but a function keeps the value's
  type; a `F?` result gives `F`. Functions and methods get the result's
  type (a method's without its `self`); callers checked after the
  declaration see it, and so do a function's own recursive calls, which go
  through the decorated value (§4.5.3). A method's body is checked as
  declared.
  Constructors, accessors, classes and fields keep their declared types.
- Wrapping framework decorators (§4.4) are checked and typed like user
  decorators; registering ones are checked by their own rules (the event
  names, the handler's parameters) and change no type.

### 4.5.2 Evaluation order

Decorators of one declaration are applied in this order (decision L26):

1. `@if` is evaluated first. If its condition is false, the declaration and
   all its other decorators disappear.
2. **Transforming core decorators** (`@profile`, `@trace`, `@inline`,
   `@checked`, `@unchecked`, `@nocheck`, `@debugOnly`) apply **innermost**,
   directly to the function as written, regardless of their position.
3. **User decorators and wrapping framework decorators** are applied
   **bottom to top**, the one nearest the declaration first. Their expressions,
   including arguments, are evaluated **top to bottom** in source order before
   any of them is applied.
4. **Registering decorators** (`@global`, registering framework decorators) and
   the **export assignment** receive the **final** value, after every
   transformation.

```wowlua
@event("BAG_UPDATE")      -- 4: registers the memoized, profiled function
@memoize                  -- 3: wraps the profiled function
@profile                  -- 2: wraps the raw function first
local function onBags() … end
```

Within a class, member decorators run in member order, before `__wl.finalize`
and before the class decorators (see
[Classes §3.1.9](03-classes.md#319-definition-order-and-lowering)).

### 4.5.3 Lowering

```wowlua
@memoize
@logCalls("cache")
local function lookup(id: number) … end
```
```lua
local lookup; lookup = function(id) … end
do local __wl_d1, __wl_d2 = memoize, logCalls("cache"); local __wl_c = { kind = "function", name = "lookup", static = false, file = "src/a.wowlua", line = 3 }
   lookup = __wl_d2(lookup, __wl_c) or lookup; lookup = __wl_d1(lookup, __wl_c) or lookup end
```

- A `local function` becomes `local f; f = function … end`, so recursive calls
  inside the body go through the decorated value (this matters for `@memoize`).
- **One context table** is created per decorated declaration and passed to all
  of its user decorators.
- The emitter places the application code on the declaration's `end` line to
  keep line numbers stable.
