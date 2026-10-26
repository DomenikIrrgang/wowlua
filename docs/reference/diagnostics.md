# Errors and warnings

Every error and warning wowlua reports has a stable code. This page explains
how the codes are built, how errors and warnings differ, and what each range
of codes covers, with every code and its summary below.

## Reading a code

A code is a letter and four digits, such as `E2001`:

- The letter is the kind: `E` for an error, `W` for a warning, `P` for a
  performance hint.
- The first digit is the area, such as `2` for types. For lexing and parsing
  the area is `01`, so those codes start `E01`.
- The rest numbers the problem within its area.

Some problems come in both kinds, one per check mode: `E1001` is an undefined
global read in strict mode, `W1001` the same read in basic mode. The code is
the same in the terminal, in VS Code and in `--message-format json`, so you
can search for it.

```text
error[E2001]: `"three"` is not assignable to `number`
  ┌─ src/main.wowlua:1:7
  │
1 │ local count: number = "three"
  │       ^^^^^ here
```

## Errors, warnings and hints

An error fails the command, which exits with status 1. A warning is printed
and the command goes on.
With `--deny-warnings`, any warning fails the command too, which suits CI.

```text
warning[W1003]: unused local `unused`
  ┌─ src/main.wowlua:2:7
  │
2 │ local unused = 1
  │       ^^^^^^ never read
  │
  = help: remove the unused local
```

`P` codes are different. The compiler never reports them: the dev window
finds them in the game while your addon runs, from what it measures, and they
never affect a build. See [Debugging](../guide/debugging.md).

## Warnings in plain Lua files

Plain Lua is checked more gently. In a file that uses no wowlua syntax, a
`.lua` file or a `.wowlua` file written in plain Lua, every name, type, flavor
and secret-value error (the `E1xxx` and `E2xxx` codes) is reported as the
warning with the same number: `E2001` becomes `W2001`. Those warning forms
are not listed separately below; they mean what their `E` code means.

```text
warning[W2001]: `"wide"` is not assignable to `number`
  ┌─ src/plain.lua:2:15
  │
2 │ frame:SetSize("wide", 10)
  │               ^^^^^^ here
```

`.lua` files are only type-checked when the manifest sets `"check": { "lua":
true }`. A file whose first line is `--!strict` gets the errors instead of the
warnings.

## Changing a warning's level

`check.lints` in `wowlua.json` changes the level of any warning, the plain Lua
forms included: `"allow"` hides it, `"deny"` makes it an error, `"warn"` keeps
it a warning.

```json
"check": {
  "lints": { "W1003": "allow", "W1004": "deny", "W2001": "allow" }
}
```

Only `W` codes go there; errors can't be turned down (E4002). One warning is
off until you turn it on: W2063, a value that is silently `any`, which reports
with `"W2063": "warn"`. The [manifest reference](manifest.md#checklints) has
the rest of `check`.

## wowlua explain

`wowlua explain` prints what a code means, without a network connection:
its summary, how to fix it, an example that reports it and one that
compiles, and related codes.

```console
$ wowlua explain E2001
E2001 (error): Type mismatch: S is not assignable to T

A value's type doesn't fit where it is used: an argument, an assignment to an
annotated local or field, or a return value. The same code covers an operator
used on a type it doesn't take, such as comparing a number with a string or
doing arithmetic on a boolean. Change the value, or widen the annotation if
the value is right.

In a plain Lua file (no wowlua syntax, no `--!strict`) the checker reports the
warning with the same number, W2001, so existing code keeps building.

Reports it:
    local count: number = "3"
    print(count)

Compiles:
    local count: number = 3
    print(count)

Related: W2001, E2005

https://wowlua.dev/docs/reference/diagnostics/E2001
```

Lowercase works too (`wowlua explain w1003`). For the warning form of a plain
Lua file, ask for its `E` code. The [command cheat sheet](../guide/commands.md)
lists the other commands.

## x01xx: Lexing and parsing

The file could not be read as code: an unexpected token, a string or long
bracket that never ends, a malformed number. This range also covers wowlua's
own syntax used in the wrong place, such as `continue` outside a loop,
`import` inside a function, a decorator with nothing to decorate, or `??`
mixed with `and` and `or` without parentheses. See
[Syntax and sugar](../language/syntax.md).

## x1xxx: Names, globals, modules, conditional compilation

What a name refers to. Undefined globals and implicit global assignments
(errors in strict mode, warnings in basic mode), unused and shadowed locals,
unreachable code, imports of missing modules or exports, and import cycles
that matter at load time. Load order belongs here too: a name read while the
addon loads that is only defined later, or a `build.load-order` the code
contradicts. So does conditional compilation: a reference to a declaration
`@if` removed, or an `@if` condition that is not static. See
[Modules](../language/modules.md) and
[Build info and profiles](../language/flavors.md).

## x2xxx: Types

The type checker. Type mismatches, unknown types, fields and methods, wrong
argument counts, values that may be `nil` in strict mode, assignments to
constants and read-only fields, access to private members, and missing
returns. APIs that not every flavor has (in shared flavor mode) and
operations on secret values are here as well. Some warnings catch Lua
mistakes in any mode, such as a division by a literal zero or
`a and nil or b`. See [Types](../language/types.md).

## x3xxx: Classes, enums, decorators

Classes and their members: a missing or misplaced `super(...)`, instances of
abstract classes, unimplemented abstract methods, `override` problems,
interfaces a class doesn't satisfy, and widget classes that use what frames
can't have. Enums that mix numbers and strings or repeat a value, unknown
decorators and decorators on the wrong target, and uses of `@deprecated`
declarations. See [Classes](../language/classes.md) and
[Decorators](../language/decorators.md).

## x4xxx: Manifest, packages, publishing

Everything around the code. `x40xx` is `wowlua.json` itself (invalid JSON,
unknown keys, wrong types) and settings read from `.pkgmeta`. `x41xx` is
dependencies and the lockfile, `x42xx` uploads to the publish sites, and
`x43xx` the TOC, flavors, the build's output files, workspaces, version tags
and installing into the game. `x44xx` is world profiles. See
[wowlua.json](manifest.md),
[Libraries and dependencies](../guide/packages.md) and
[Building and publishing](../guide/publishing.md).

## x5xxx, x6xxx, x7xxx: Code generation, bundling, debugging

`x5xxx` is the Lua wowlua writes: a Lua 5.1 limit exceeded (too many locals,
upvalues or constants in one function), and output that fails validation,
such as a TOC or XML file that names a file the build lacks.

`x6xxx` is bundling and minification: an unknown minification pass, a module
that dead-code removal dropped, code that reads Lua file names that bundling
changes, and minified code sent to CurseForge, Wago or WoWInterface, which
Blizzard's AddOn policy forbids.

`x7xxx` is debugging and reflection: `todo()` left in a release build,
reflection asked for in a profile that turns it off, and `nameof` with an
argument it can't name. The `P7xxx` performance hints share this range:
`P71xx` events and scripts, `P72xx` garbage and memory, `P73xx` frame time.
See [Debugging helpers](../language/debugging.md).

## x8xxx: Standard library

Mistakes in using the `wowlua:*` modules that the compiler can see: unknown
event names, duplicate slash commands, `saved.bind` on a SavedVariable the
manifest doesn't declare, locale files that don't match the base locale,
settings controls bound to the wrong field, invalid keybinding names, and
test helpers imported outside tests. `x80xx` to `x82xx` are the first
modules (events, slash commands, saved variables, hooks, locales, settings
and tests),
`x83xx` to `x85xx` the game data, interface and infrastructure modules
(flavor fallbacks, auras, tooltips, frames, keybindings, colors, media,
callbacks, jobs, state drivers). See the
[standard library overview](../stdlib/overview.md).
