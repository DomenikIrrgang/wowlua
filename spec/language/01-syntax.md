# 1. Syntax

This chapter defines the lexical structure of wowlua and every statement- and
expression-level extension to Lua 5.1. Type syntax is in [Types](02-types.md),
classes, interfaces and enums are in [Classes](03-classes.md), decorators are in
[Decorators](04-decorators.md), and `import`/`export` is in [Modules](05-modules.md).
The complete grammar is in [Appendix A](../appendix/grammar.md).

## 1.1 Source files

| Extension | Meaning |
|-----------|---------|
| `.wowlua` | wowlua source module. |
| `.d.wowlua` | Declaration file: types and ambient declarations only, no emitted code. |
| `.lua` | Plain Lua. It is syntax-checked and passed through. It may be type-checked (warnings only). |
| `.xml` | WoW UI XML. It is checked for well-formedness and passed through. |

Source files are byte sequences. Like Lua, wowlua does not require an encoding,
but tools assume UTF-8 for diagnostics. A leading UTF-8 BOM and a leading `#!`
line are ignored by the parser and **stripped from the output**, because
WoW's loader does not skip a `#!` line.

### 1.1.1 Directive comments

A directive is a line comment starting with `--!` that appears **before the
first token** of the file. Only comments and blank lines (after an optional BOM
or `#!` line) may precede it, including a license block comment. A `--!` comment
after the first token is an ordinary comment. Directives are Lua comments, so
they don't affect the superset guarantee.

| Directive | Effect |
|-----------|--------|
| `--!strict` | Type-check this file in strict mode. |
| `--!basic` | Type-check this file in basic mode (the default). |
| `--!nocheck` | Do not type-check this file (it is still parsed and compiled). |
| `--!nolint` | Suppress lint warnings for this file. |

A Lua file written for Luau that starts with `--!strict` is checked in strict
mode, as its author intended.

## 1.2 Lexical structure

wowlua lexing is Lua 5.1 lexing plus the tokens below. Every new token is
built from characters or character sequences that cannot appear outside
strings and comments in valid Lua 5.1, so adding them cannot change the
meaning of a valid Lua program. Comments (`--`) are recognized first, as in Lua,
so `-->` is still a comment.

### 1.2.1 New tokens

| Token | Use |
|-------|-----|
| `@` | Decorator prefix |
| `?` | Optional types (`T?`) |
| `?.` `?.[` `?.(` | Optional chaining |
| `?:` | Optional method call (`a?:m()`), and optional fields (`label?: string`) |
| `??`, `??=` | Nil-coalescing operator and assignment |
| `` ` `` | Interpolated string delimiter |
| `::` | Type assertion (cast) |
| `->` | Function type arrow (type context only) |
| `+=` `-=` `*=` `/=` `%=` `^=` `..=` `//=` `&=` `\|=` `<<=` `>>=` | Compound assignment |
| `//` | Integer (floor) division |
| `&` `\|` `~` `<<` `>>` | Bitwise operators. `~` alone, not as part of `~=`. In type context `\|` and `&` mean union and intersection. |

Tokens are formed by maximal munch, so `nickname?: string` lexes as `nickname`,
`?:`, `string`. The grammar accepts the single `?:` token everywhere it expects
`?` followed by `:`.

**Splitting in type context.** In type context, the tokens `>>`, `>=` and `>>=`
are split at their first `>`, so nested generics followed by an initializer
work, as in Rust and Java:

```wowlua
local m: Map<string, List<number>> = {}
local t: List<number>= {}          -- lexed as `>=`, split into `>` `=`
```

### 1.2.2 Numeric literals

Lua 5.1 numerals (decimal, hexadecimal `0x`, exponents) are unchanged. wowlua
adds two forms. Both are malformed numbers in Lua 5.1 (Lua's lexer consumes
the `_` or `b` into the numeral and rejects it), so adding them is safe:

| Form | Example | Lowered to |
|------|---------|------------|
| Digit separators | `1_000_000`, `0xFF_FF`, `3.141_592`, `1e1_0` | `1000000`, `0xFFFF`, `3.141592`, `1e10` |
| Binary literals | `0b1010_1010`, `0B11` | `170`, `3` |

A `_` must appear **between two digits** of the same digit run: in the integer
part, the fraction, the exponent, or the hexadecimal or binary digits. It may
not touch `.`, `e`, `E`, `x`, `X`, `b`, `B`, a sign, or another `_`. `1__0`,
`_1`, `1_`, `1_.5`, `0x_FF` and `1e_5` are errors (E0103).

### 1.2.3 Quoted strings

`'…'`, `"…"` and long strings `[[…]]` / `[==[…]==]` follow Lua 5.1 exactly,
including the 5.1 rule that an unknown escape such as `\x` or `\q` yields the
character itself. wowlua does **not** add `\x`, `\u{…}` or `\z` to quoted
strings, because that would change the meaning of valid Lua 5.1 code. Those
escapes are available in backtick strings only.

### 1.2.4 Interpolated strings

A backtick string may contain expressions in braces:

```wowlua
local msg = `Hello {name}, you have {#items} item{#items == 1 and "" or "s"}`
```

Rules:

- `{expr}` embeds any expression, including nested backtick strings.
- `\{` and `\}` produce literal braces. `` \` `` produces a backtick. A lone
  `}` outside an embedded expression is an error (E0116).
- Escapes: all Lua 5.1 escapes, plus `\xXX` (byte), `\u{XXXX}` (UTF-8 encoded
  code point) and `\z` (skip following whitespace).
- A backtick string may span lines. Newlines are part of the string.
- There are no format specifiers. Use `string.format` for formatting.
- A backtick string is an expression, not a string literal. It cannot be used
  as a call argument without parentheses (`f`…``), unlike `f "…"`.

**Lowering.** The parts are concatenated with `..`.

- Every embedded expression whose operator precedence is lower than `..`
  (comparisons, `and`, `or`, `??`, bitwise operators) is wrapped in parentheses.
- In **basic** mode, every embedded value is wrapped in `tostring(…)`. In
  **strict** mode, `tostring` is omitted when the static type is a non-optional
  `string` or `number` (decision L28: type-based fast forms only where types are
  trustworthy).
- Constant embedded values (literals, `const`s, scalar `wowlua:meta` values,
  enum members) are folded into the surrounding text (decision T81).
- Lua 5.1 limits nesting to 200 syntax levels, and `..` chains nest. A string
  with more than 50 parts is emitted as parenthesized groups of at most 50
  operands.
- A string without embedded expressions lowers to a plain quoted string.

```wowlua
local s = `{player} has {hp} HP ({hp > 50 and "ok" or "low"})`
```
```lua
-- basic mode
local s = tostring(player) .. " has " .. tostring(hp) .. " HP (" .. tostring((hp > 50 and "ok" or "low")) .. ")"
-- strict mode, player: string, hp: number
local s = player .. " has " .. hp .. " HP (" .. (hp > 50 and "ok" or "low") .. ")"
```

### 1.2.5 Reserved and contextual keywords

Lua 5.1's reserved words are unchanged. No new *reserved* words are added. The
following are **contextual keywords**: they act as keywords only in the
positions listed, and everywhere else they are ordinary identifiers.

| Word | Keyword position | Why this position is safe |
|------|------------------|---------------------------|
| `class` | Statement start, followed by a name: `class Foo` | `Name Name` is a syntax error in Lua |
| `abstract` | Statement start, followed by `class`, or a member modifier inside a class body | Same as above, or new syntax |
| `interface` | Statement start, followed by a name | `Name Name` |
| `enum` | Statement start, followed by a name | `Name Name` |
| `type` | Statement start, followed by a name: `type Point = …` | `Name Name`. `type(x)` stays a call. |
| `const` | Statement start, followed by a name | `Name Name` |
| `declare` | Statement start, followed by `function`, `global`, `class`, `interface`, `namespace`, `event`, `message`, `module` | `Name keyword` / `Name Name` |
| `import` | Statement start, followed by a name, `*` or `type` | `Name Name`, `Name *` is not a valid statement. `import("x")` and `import "x"` stay calls. |
| `export` | Statement start, followed by `local`, `function`, `const`, `class`, `abstract`, `interface`, `enum`, `type` | `Name keyword` / `Name Name` |
| `from`, `as` | Inside an `import` statement only | New syntax |
| `extends` | In class and interface headers, and in generic constraints (`<T extends X>`) | New syntax |
| `implements` | Inside a class header only | New syntax |
| `continue` | Statement start, see §1.3.6 | See §1.3.6 |
| `static`, `readonly` | In class bodies, interface bodies, `declare class` and `declare namespace` bodies | New syntax |
| `private`, `protected`, `public`, `override`, `overload`, `constructor`, `get`, `set` | Inside a class body only | New syntax |
| `super` | Inside class bodies (constructors, methods, accessors) | New syntax |
| `typeof`, `is` | Type context only | New syntax |

**Member names inside class and interface bodies.** A modifier word
(`static`, `readonly`, `private`, `get`, `constructor`, …) is a *member name*
when it is directly followed by `:`, `?:` or `(`, so a field may be called
`static: boolean` and a method `get(key: string)`. Otherwise it is a modifier.
Inside a class body, `super` is always the keyword. Declaring a local or
parameter named `super` there is an error (E3040).

A pure Lua file therefore never contains a wowlua keyword in keyword position.

## 1.3 Statements

### 1.3.1 Typed locals

```wowlua
local count: number = 0
local name: string?, realm: string? = UnitName("target")
local frames: Frame[] = {}
```

Annotations are erased. Lowering: `local count = 0`.

### 1.3.2 Constants

```wowlua
const MAX_ITEMS = 40
const PREFIX: string = "MyAddon"
const DEFAULTS = { scale = 1 }
```

`const` declares a local binding that **cannot be reassigned** (error E2010).
It requires an initializer and declares exactly one name (E0112). It is
block-scoped like `local` and may appear in any block. Inside a loop body, a
non-folded `const` is a new binding on every iteration, as with `local`.

If the initializer is a *constant expression*, every reference is replaced by
its folded value and no local is emitted. A constant expression is a literal,
another folded constant, a **scalar** `wowlua:meta` value (boolean, number or
string), or an arithmetic, concatenation, equality or logical operation on
constant expressions. Otherwise the constant lowers to a normal `local`, which
counts toward Lua's limits of 200 locals and 60 upvalues per function. A table
bound with `const` is still mutable. Only the binding is fixed.

Folding rules:

| Case | Rule |
|------|------|
| Numbers | Folded with Lua 5.1 double semantics and emitted so they read back exactly (`%.17g` when the shortest form does not round-trip) |
| NaN, infinity | Emitted as `(0/0)`, `(1/0)`, `(-1/0)` |
| Ordering comparisons (`<`, `<=`, `>`, `>=`) on strings | **Not folded**: Lua 5.1 compares strings with the C locale's `strcoll` at runtime |
| Bitwise operators | Folded only with WoW's `bit` library semantics: operands converted to 32-bit integers, results normalized to signed 32-bit, shift counts taken modulo 32 |
| `..` | Lua's number formatting (`%.14g`) for numeric operands |

```wowlua
const SIZE = 16
const HALF = SIZE / 2
frame:SetSize(SIZE, HALF)
```
```lua
frame:SetSize(16, 8)
```

### 1.3.3 Destructuring

```wowlua
local { x, y } = point
local { name, level = 1 } = getInfo()          -- default when the field is nil
local { width, height }: Size = frame:GetSizeInfo()
```

Record destructuring declares one local per listed field, reading the field
of the same name. `= default` substitutes a value when the field is `nil`.

Semantics:

- The source expression is evaluated **once**, **before** the new locals come
  into scope, as for any Lua `local` statement. `local { x } = x` therefore reads
  the outer `x`.
- Defaults are evaluated after all fields have been read, in field order, and
  only for fields that are `nil`. A default may refer to names destructured
  before it in the same statement.

```lua
local x, y = point.x, point.y                    -- plain name: read directly
local __wl_t = getInfo(); local name, level = __wl_t.name, __wl_t.level; if level == nil then level = 1 end
```

The source is read directly when it is a local or upvalue name. Otherwise it is
stored in a hygienic temporary that stays in scope until the end of the
enclosing block (it counts toward the 200-locals limit).

Only `local` declarations destructure. There is no array destructuring. Lua's
multiple assignment already covers multiple return values.

### 1.3.4 Compound assignment

```wowlua
total += amount
label ..= " (offline)"
flags |= FLAG_DIRTY
items[getIndex()] -= 1
x *= a + b
cache[key] ??= {}
```

`a op= e` means `a = a op (e)`. The right-hand side is always treated as a
parenthesized operand. Every subexpression of the target that contains a call
is evaluated exactly once:

| Target | Lowering |
|--------|----------|
| `x op= e` | `x = x op (e)` (parentheses omitted when not needed) |
| `t.k op= e` (where `t` is a local or upvalue name) | `t.k = t.k op (e)` |
| `a.b.k op= e` | `local __wl_o = a.b; __wl_o.k = __wl_o.k op (e)` |
| `t[f()] op= e` | `local __wl_k = f(); t[__wl_k] = t[__wl_k] op (e)` |
| `a ??= e` | `if a == nil then a = e end`. `e` is evaluated only when `a` is `nil`, and nothing is written otherwise. |

Temporaries are scoped in a `do … end` block when needed. Compound assignment
has a single target (`a, b += 1` is error E0111). An optional chain cannot be
an assignment target (`a?.b = 1`, `a?.b += 1`: error E0113).

### 1.3.5 Functions: types, generics, defaults

All Lua function forms (`function f`, `local function f`, `function t.a.b`,
`function t:m`, function expressions) accept:

- parameter type annotations: `(x: number, y: string?)`
- variadic annotation: `(...: number)`, or a generic pack `(...: T...)`
  (see [Types §2.2.10](02-types.md#2210-type-packs))
- return type annotation after `)`: `): boolean`, `): (number, string)`, `): ()`
- generic type parameters after the name or after `function` in expressions:
  `function map<T, U>(…)`, `function<T>(x: T): T … end`
- default parameter values: `(x: number = 1)`

```wowlua
local function clamp(v: number, lo: number = 0, hi: number = 1): number
    return math.max(lo, math.min(hi, v))
end
```
```lua
local function clamp(v, lo, hi)
    if lo == nil then lo = 0 end; if hi == nil then hi = 1 end
    return math.max(lo, math.min(hi, v))
end
```

Default rules:

- A default applies when the argument is `nil`, whether omitted or explicit.
- Defaults are evaluated at call time, left to right, and may reference earlier
  parameters.
- A parameter with a default has type `T` inside the body and `T?` for callers.

In `function t:m()`, `self` has the type of `t`.

### 1.3.6 `continue`

`continue` skips to the next iteration of the innermost enclosing loop
(`while`, `repeat`, numeric or generic `for`) in the same function. Like
`break`, it must be the **last statement of its block** (E0114). Write
`do continue end` to continue from the middle of a block.

**Disambiguation.** A statement beginning with the identifier `continue` is a
continue statement if the next token is **none** of `=`, `,`, `.`, `[`, `:`,
`(` (on the same line), a string literal, `{`, `?.`, `?.[`, `?.(`, `?:`, `::`
or a compound assignment operator. In every remaining case Lua 5.1 would report
a syntax error, because a bare name is not a statement. A `(` on the next line
also makes it a continue statement, because Lua 5.1 rejects that case as
ambiguous syntax.

**Lowering.** Lua 5.1 has no `goto`, so the loop body is wrapped in a
single-iteration `repeat … until true`, and `continue` becomes `break`:

```wowlua
for i, item in ipairs(items) do
    if item.hidden then continue end
    if item.last then break end
    show(item)
end
```
```lua
for i, item in ipairs(items) do
    local __wl_brk = false
    repeat
        if item.hidden then break end
        if item.last then __wl_brk = true; break end
        show(item)
    until true
    if __wl_brk then break end
end
```

The `__wl_brk` flag is only emitted when the body contains both `continue` and
`break` for the same loop. `continue` inside a `repeat … until cond` loop whose
condition references a local declared in the body is an error (E0107), because
wrapping the body would hide that local from the condition.

### 1.3.7 Typed loop variables

```wowlua
for i: number = 1, 10 do … end
for unit: string, info: UnitInfo in pairs(units) do … end
```

Without annotations, generic `for` variables are typed from the iterator's
signature (see [Types §2.4.4](02-types.md#244-iteration)).

### 1.3.8 Declarations

The following statements are covered in their own chapters:

| Statement | Chapter |
|-----------|---------|
| `type`, `declare`, `interface` | [Types](02-types.md), [Classes](03-classes.md) |
| `class`, `abstract class`, `enum` | [Classes](03-classes.md) |
| `@decorator` | [Decorators](04-decorators.md) |
| `import`, `export` | [Modules](05-modules.md) |

**Placement.** `type` and `interface` may appear in any block and are scoped to
it. `class`, `abstract class` and `enum` may appear only at the top level of a
file (E0115), because each declaration creates exactly one runtime table.
`import` and `export` are top-level only (E0108). `declare` is top-level only.

## 1.4 Expressions

### 1.4.1 Optional chaining

| Form | Meaning |
|------|---------|
| `a?.b` | `nil` if `a` is `nil`, else `a.b` |
| `a?.[k]` | `nil` if `a` is `nil`, else `a[k]` |
| `a?:m(args)` | `nil` if `a` is `nil`, else `a:m(args)` |
| `f?.(args)` | `nil` if `f` is `nil`, else `f(args)` |

Short-circuiting covers the rest of the chain. In `a?.b.c(d)`, if `a` is `nil`,
then `.c`, the call and the evaluation of `d` are all skipped and the result
is `nil`. Only `nil` short-circuits. `false?.x` indexes `false` and errors, as
in Lua.

An optional chain that ends in a call **always yields exactly one value**,
like a parenthesized call `(f(x))` (decision L23). Use an ordinary call when
all return values are needed. Optional chains cannot be assignment targets
(E0113).

**Lowering** uses the cheapest correct form:

1. **Strict mode only:** if the chain result's static type cannot be `false`
   (and is not `any`), and every operand before a `?.` is a local or upvalue
   name or otherwise free of side effects, the and/or form is used: `a?.b`
   becomes `(a ~= nil and a.b or nil)`, or `(a and a.b)` when `a` cannot be
   `false` either.
2. Otherwise each operand before a `?.` is stored in a temporary, and the
   containing statement is linearized: preceding operands are hoisted into
   temporaries, left to right, to keep Lua's evaluation order, and the chain
   becomes an `if`:
   ```lua
   local __wl_t = a; if __wl_t ~= nil then __wl_t = __wl_t.b end
   ```
   This "preceding statement" mechanism is described in
   [Code generation](../toolchain/codegen.md#144-preceding-statements).

### 1.4.2 Nil-coalescing

`a ?? b` evaluates to `a` unless `a` is `nil`, in which case it evaluates
`b`. Unlike `a or b`, a `false` value of `a` is kept. `b` is only evaluated when
needed.

- Lowering: in **strict** mode, when `a`'s static type cannot be `false`, it
  lowers to `a or b`. Otherwise it is linearized:
  `local __wl_t = a; if __wl_t == nil then __wl_t = b end`.
- `??` has the same precedence as `or`. Mixing `??` with `and`/`or` without
  parentheses is an error (E0110), as in JavaScript.
- `a ??= b` assigns only when `a` is `nil` (§1.3.4).

### 1.4.3 Type assertions

`expr :: T` asserts to the checker that `expr` has type `T`. It has no runtime
effect, except in profiles with `runtime-checks = "all"`, where checkable types
are verified (see [Debugging](07-debugging.md#75-runtime-type-checks)).

**Extent.** `::` applies to the operand on its left at unary level: it binds
tighter than every binary operator and looser than calls and indexing, so
`a.b :: Frame` asserts `a.b` and `-x :: number` asserts `x`. The type on the
right is a **full type**, including `|` and `&` (decision L25). It extends as
far as the type grammar allows. After a type name, `<` always opens type
arguments. A binary operator applied to the result of a cast needs
parentheses:

```wowlua
local v = raw :: string | number          -- union type
local m = (flags :: number) | MASK        -- bitwise or on the cast value
local ok = (x :: number) < limit          -- comparison on the cast value
```

The asserted type must be *comparable* with the expression's type: one must
be assignable to the other (E2015). To force an unrelated type, write
`expr :: any :: T`.

### 1.4.4 Bitwise and integer division

| Operator | Lowering | Notes |
|----------|----------|-------|
| `a & b` | `bit.band(a, b)` | |
| `a \| b` | `bit.bor(a, b)` | |
| `a ~ b` | `bit.bxor(a, b)` | |
| `~a` | `bit.bnot(a)` | |
| `a << n` | `bit.lshift(a, n)` | |
| `a >> n` | `bit.rshift(a, n)` | Logical shift, as in Lua 5.3 |
| `a // b` | `math.floor(a / b)` | |

WoW's `bit` library (LuaBitOp) operates on 32-bit integers and returns signed
results. That differs from Lua 5.3's 64-bit semantics, and the language
documents the WoW behavior. The checker requires `number` operands. Chains of
the same associative operator are flattened, because `bit.band`, `bit.bor` and
`bit.bxor` accept multiple arguments (`a | b | c` becomes `bit.bor(a, b, c)`).
Constant operands are folded with the rules in §1.3.2.

### 1.4.5 Function expressions

Function expressions accept the same annotations as declarations:

```wowlua
frame:SetScript("OnUpdate", function(self, elapsed: number)
    acc += elapsed
end)
local identity = function<T>(x: T): T return x end
```

When a function expression is passed where a function type is expected, its
parameters are *contextually typed*. In the example above `self` is typed as
`Frame` without an annotation (see [Types](02-types.md#243-contextual-typing)).

### 1.4.6 Operator precedence

From lowest to highest. All binary operators are left-associative except `..`
and `^`, which are right-associative.

| Level | Operators |
|-------|-----------|
| 1 | `or` `??` (not mixable without parentheses) |
| 2 | `and` |
| 3 | `<` `>` `<=` `>=` `~=` `==` |
| 4 | `\|` |
| 5 | `~` (binary) |
| 6 | `&` |
| 7 | `<<` `>>` |
| 8 | `..` (right) |
| 9 | `+` `-` |
| 10 | `*` `/` `//` `%` |
| 11 | unary `not` `#` `-` `~` |
| 12 | `^` (right) |
| 13 | `::` (postfix type assertion; the type extends as far as possible, §1.4.3) |
| 14 | calls, indexing, `.`, `:`, `?.`, `?:`, `?.[`, `?.(` |

Levels 3–12 follow Lua 5.3. Lua 5.1 code contains none of the new operators,
so its parse is unchanged.

### 1.4.7 Generated references to globals

Lowerings call standard globals: `tostring`, `type`, `select`, `setmetatable`,
`rawget`, `error`, `math.floor`, `bit.*`. When a user binding with one of those
names is in scope at the point of use (e.g. `local bit = …` or a parameter named
`type`), the generated code refers to a hygienic file-level alias instead
(`local __wl_bit_band = bit.band`), bound in the prologue before any user code.
Otherwise the global is referenced directly. User code can therefore never
capture or redirect compiler-generated calls.

## 1.5 Unsupported Lua features

| Feature | Status |
|---------|--------|
| `goto` / labels (Lua 5.2+) | Not supported. `goto` stays an identifier, as in 5.1. A Lua 5.2 label `::name::` is a syntax error (the `::` token), as it is in Lua 5.1. |
| `local x <const>` / `<close>` attributes (Lua 5.4) | Not supported. Use `const`. |
| Integer subtype (Lua 5.3+) | Not supported. All numbers are doubles, as in WoW. |
| `\x`, `\u{}`, `\z` in quoted strings | Not supported (§1.2.3). Use backtick strings. |
