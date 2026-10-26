# x01xx · parsing

## E0101

The parser met a token it can't use at that point: a missing `then`, `do`,
`end` or closing parenthesis, a stray operator, or `...` outside a vararg
function. The message names what was expected and the token found instead
("near '<eof>'" means the file ended too early). WoW would refuse to load the
file with the same Lua syntax error, so it is an error in every file.

The same code is reported for wowlua syntax (a type annotation, `+=`, `?.`,
…) in a `.lua` file, which stays plain Lua 5.1: rename the file to `.wowlua`,
or write the code in Lua.

```wowlua fails
local hp = UnitHealth("player")
if hp > 0
    print(hp)
end
```

```wowlua compiles
local hp = UnitHealth("player")
if hp > 0 then
    print(hp)
end
```

Related: E0102, E0104

## E0102

A string runs to the end of its line, or a long string or long comment
(`[[ … ]]`, `--[==[ … ]==]`) or a backtick string runs to the end of the
file, without its closing delimiter. Quoted strings (`"…"`, `'…'`) cannot span
lines, as in Lua 5.1: close the string on the same line, or use a long string
or a backtick string for text with line breaks.

```wowlua fails
local greeting = "Welcome back
print(greeting)
```

```wowlua compiles
local greeting = "Welcome back"
print(greeting)
```

Related: E0101, E0116

## E0103

A number literal is malformed, such as `1e`, `3..2` or `0xG`, or uses a `_`
digit separator in the wrong place. wowlua allows `_` between two digits of
the same run (`1_000_000`, `0xFF_FF`, `3.141_592`, `0b1010_1010`), but not at
the start or end, twice in a row, or next to `.`, `e`, `x`, `b` or a sign:
`1__0`, `_1`, `1_`, `1_.5`, `0x_FF` and `1e_5` are all E0103.

```wowlua fails
local maxGold = 1__000_000
print(maxGold)
```

```wowlua compiles
local maxGold = 1_000_000
print(maxGold)
```

## E0104

A line starts with `(` right after an expression that could be called, so it
reads as a call that spans two lines. Lua 5.1 rejects this as "ambiguous
syntax (function call x new statement)", and wowlua does too. If you meant a
call, put the `(` on the same line as the function. If you meant a new
statement, start it differently (assign the parenthesized value to a local).

```wowlua fails
local show = print
show
("ready")
```

```wowlua compiles
local show = print
show("ready")
```

Related: E0101

## E0105

A backtick string contains an escape it doesn't know. Backtick strings take
Lua's escapes (`\n`, `\t`, `\\`, `\"`, `\ddd`, …) plus `\{`, `\}`, `` \` ``,
`\xXX` (two hex digits), `\u{XXXX}` and `\z`; anything else, such as `\q` or
the `\A` of a WoW path, is an error. Quoted strings keep Lua 5.1's rule, where
an unknown escape yields the character itself, so the same text may work in
`"…"` and fail in a backtick string. Write `\\` for a backslash.

```wowlua fails
local name = "MyAddon"
print(`{name} lives in Interface\AddOns`)
```

```wowlua compiles
local name = "MyAddon"
print(`{name} lives in Interface\\AddOns`)
```

Related: E0116

## E0106

A `break` or `continue` is not inside a loop of the same function. Both only
leave or restart the innermost `while`, `repeat` or `for` loop; a loop around
the function that contains them doesn't count. Use `return` to leave a
function early.

```wowlua fails
local function onEvent(event: string)
    if event ~= "PLAYER_LOGIN" then
        break
    end
    print("logged in")
end
onEvent("PLAYER_LOGIN")
```

```wowlua compiles
local function onEvent(event: string)
    if event ~= "PLAYER_LOGIN" then
        return
    end
    print("logged in")
end
onEvent("PLAYER_LOGIN")
```

Related: E0114

## E0107

`continue` is used in a `repeat … until cond` loop whose condition reads a
local declared in the loop body. Lua 5.1 has no `goto`, so wowlua wraps the
body in an inner loop to implement `continue`, and the body's locals would no
longer be visible to `until`. Declare the local before the loop and assign it
in the body, or use another kind of loop.

```wowlua fails
local i = 0
repeat
    i += 1
    local finished = i >= 10
    if i % 2 == 0 then continue end
    print(i)
until finished
```

```wowlua compiles
local i = 0
local finished = false
repeat
    i += 1
    finished = i >= 10
    if i % 2 == 0 then continue end
    print(i)
until finished
```

Related: E0114

## E0108

`import` or `export` is inside a block (a function, `if`, loop or `do`).
Both only work at the top level of a file: imports are resolved when the
file loads, and exports become the module's table. Move the `import` to the
top of the file, and export a top-level declaration.

```wowlua fails
local function start()
    import after from "wowlua:timer"
    after(1, function() print("started") end)
end
start()
```

```wowlua compiles
import after from "wowlua:timer"

local function start()
    after(1, function() print("started") end)
end
start()
```

Related: E1011, E0115

## E0109

A decorator (`@name` or `@name(…)`) stands before a statement that can't be
decorated, such as a call or an assignment. Decorators apply to declarations:
functions, `local`, `const`, classes, class members, enums and the like. Put
the decorator right before the declaration it is meant for.

```wowlua fails
@deprecated
print("old API")
```

```wowlua compiles
@deprecated
local function oldApi()
    print("old API")
end
```

## E0110

`??` is mixed with `and` or `or` in one expression without parentheses, as
in `a ?? b or c`. They have the same precedence, and the reading isn't
obvious (`??` keeps `false`, `or` doesn't), so wowlua asks for parentheses, as
JavaScript does. Add them around the part that should go first.

```wowlua fails
local saved: boolean? = nil
local fallback = true
print(saved ?? fallback or false)
```

```wowlua compiles
local saved: boolean? = nil
local fallback = true
print((saved ?? fallback) or false)
```

## E0111

A compound assignment (`+=`, `..=`, `??=`, …) has several targets, as in
`a, b += 1`. Compound assignment takes exactly one target; write one
statement per variable.

```wowlua fails
local kills, deaths = 0, 0
kills, deaths += 1
print(kills, deaths)
```

```wowlua compiles
local kills, deaths = 0, 0
kills += 1
deaths += 1
print(kills, deaths)
```

## E0112

A `const` has no `= value`, or declares several names (`const A, B = 1, 2`).
A constant can't be assigned later, so it needs its value where it is
declared, and each `const` declares one name. Use one `const` per value, or
`local` for a value that is set later.

```wowlua fails
const MAX_ITEMS
print(MAX_ITEMS)
```

```wowlua compiles
const MAX_ITEMS = 40
print(MAX_ITEMS)
```

Related: E2010

## E0113

An optional chain (`?.`, `?.[`, `?:`) is the target of an assignment or a
compound assignment, as in `t.inner?.n = 1`. If the chain stopped at `nil`
there would be nothing to assign to. Test the value first and assign
through a plain access.

```wowlua fails
local settings: { frame: { scale: number }? } = {}
settings.frame?.scale = 1
```

```wowlua compiles
local settings: { frame: { scale: number }? } = {}
if settings.frame then
    settings.frame.scale = 1
end
```

## E0114

`continue` is followed by more statements in its block. Like `break` and
`return`, it must be the last statement of its block, because the code after
it could never run. Write `do continue end` to continue from the middle of a
block, or put it inside an `if`.

```wowlua fails
for i = 1, 3 do
    continue
    print(i)
end
```

```wowlua compiles
for i = 1, 3 do
    if i == 2 then continue end
    print(i)
end
```

Related: E0106, E0107

## E0115

A `class`, `abstract class`, `enum`, `declare` or `overload function` stands
inside a block (a function, `if`, loop or `do`). They may only appear at the
top level of a file: each class and enum is exactly one runtime table, and
declarations and overloads describe the file's top-level names. Move the
declaration to the top level. (`type` and `interface` may appear in any
block.)

```wowlua fails
local function setup()
    enum Mode { Simple, Advanced }
    return Mode.Simple
end
print(setup())
```

```wowlua compiles
enum Mode { Simple, Advanced }

local function setup()
    return Mode.Simple
end
print(setup())
```

Related: E0108

## E0116

A backtick string contains a `}` that doesn't close an embedded `{…}`
expression. Write `\}` for a literal closing brace (and `\{` for an opening
one).

```wowlua fails
local count = 3
print(`count} items: {count}`)
```

```wowlua compiles
local count = 3
print(`\{count\} items: {count}`)
```

Related: E0105, E0102

## E0117

`import _` is combined with other names, as in `import _, clamp from "./math"`.
`import _` only makes sure a file loads first and binds nothing, so it must be
the only name in its import. A named import already loads its module first,
so drop the `_`.

```wowlua fails
import _, clamp from "./math"
print(clamp(5, 0, 1))
```

```wowlua compiles title=src/main.wowlua
import clamp from "./math"
print(clamp(5, 0, 1))
```

```wowlua compiles title=src/math.wowlua
export function clamp(v: number, lo: number, hi: number): number
    return math.max(lo, math.min(hi, v))
end
```

## E0118

`export function` is followed by a dotted or method name (`M.f`, `M:m`).
An export names a module-level local, so it takes a plain name. Export the
function by its own name, or export the table and add functions to it.

```wowlua fails
export local M = {}
export function M.greet()
    print("hello")
end
```

```wowlua compiles
export local M = {}
function M.greet()
    print("hello")
end
```

Related: E1016

## E0119

An `export local` has no initializer. Exported bindings can't be assigned
after their declaration (E1010), so the value must be given where it is
exported. For state that changes, export a table and change its contents.

```wowlua fails
export local count: number
```

```wowlua compiles
export local count: number = 0
```

Related: E1010
