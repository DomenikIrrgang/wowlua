# Slash commands

`wowlua:slash` registers slash commands with a decorator. Its router splits
what the player typed into subcommands, typed arguments and flags, and prints
help and usage lines.

## Add a slash command

Put `@slash` on a function, with the commands it answers to. The handler gets
the text typed after the command.

```wowlua title=src/commands.wowlua
import slash from "wowlua:slash"

local window = CreateFrame("Frame", nil, UIParent)

@slash("/mya", "/myaddon")
local function command(msg: string)
    if msg == "" then
        window:SetShown(not window:IsShown())
    end
end
```

This replaces the `SLASH_MYADDON1` globals and the `SlashCmdList` entry you
would write by hand. wowlua generates them under a key built from your addon's
name (`WOWLUA_MYADDON_1`), so they can't collide with another addon's. The
compiler reports a command that doesn't start with `/` or contains a space
(E8101), a command registered twice in your addon (E8010), and a command that
shadows one of Blizzard's (warning W8011).

The handler also gets the chat edit box as a second argument, as
`SlashCmdList` passes it.

## Subcommands with a router

Most addons end up splitting `msg` into subcommands. `router` does it for you.
Define the commands once and pass every message to `dispatch`:

```wowlua title=src/commands.wowlua
import slash, router from "wowlua:slash"

local options = {
    scale = 1,
    sortBy = "name" :: "name" | "quality" | "type",
    notes = {} :: { [string]: string },
    warnAt = 5,
}

local window = CreateFrame("Frame", nil, UIParent)

local commands = router({
    show = {
        aliases = { "open" },
        help = "Show the window",
        run = function() window:Show() end,
    },
    scale = {
        args = { "number" },
        help = "Set the window scale",
        run = function(value: number) options.scale = value end,
    },
    sort = {
        args = { { "name", "quality", "type" } },
        help = "Choose how items are sorted",
        run = function(by: "name" | "quality" | "type") options.sortBy = by end,
    },
    note = {
        args = { "string", "rest" },
        help = "Add a note to a bag",
        run = function(bag: string, text: string) options.notes[bag] = text end,
    },
    limit = {
        args = { "number?" },
        help = "Warn below this many free slots",
        run = function(slots: number?) options.warnAt = slots or 5 end,
    },
    reset = {
        flags = { "all" },
        help = "Reset the window position (--all: every setting)",
        run = function(flags: { all: boolean })
            window:ClearAllPoints()
            window:SetPoint("CENTER")
            if flags.all then
                options.scale = 1
                options.warnAt = 5
            end
        end,
    },
})

@slash("/mya", "/myaddon")
local function command(msg: string)
    commands:dispatch(msg)
end
```

`/mya scale 1.2` calls `scale`'s `run` with the number `1.2`, and
`/mya SORT Quality` calls `sort`'s with `"quality"`. Command names match in any
case. Argument values keep the case the player typed, except choices, which
come back spelled as in your list.

`dispatch` returns `true` when it ran a command, printed the help or printed a
usage line, and `false` for an unknown command.

## Typed arguments

Each entry of `args` converts one word of the input before `run` sees it.
Declare `run`'s parameters with the types below. The compiler checks that each
entry is a valid spec, that `run` takes one parameter per argument, plus one
for the flags, and that each typed parameter takes what its argument gives: a
`string` parameter for a `"number"` argument is an error (E8112).

| Spec | The player types | `run` gets |
|------|------------------|------------|
| `"string"` | `Gorak` | the word as typed |
| `"number"` | `1.5` | a number, converted with `tonumber` |
| `"boolean"` | `true`/`false`, `on`/`off`, `yes`/`no`, `1`/`0`, in any case | a boolean |
| `{ "a", "b" }` | one of the choices, in any case | that choice, as you spelled it |
| `"rest"` | anything | the rest of the line, unparsed (last argument only) |
| `"string?"`, `"number?"`, `"boolean?"` | the value, or nothing | the value or nil (trailing arguments only) |

Quotes group words into one argument: in `/mya note "Hearthstone Bag" keep
for later`, the bag is `Hearthstone Bag` and the note `keep for later`. Single
quotes work too, and `\"` puts a quote inside a quoted word.

## Flags

A flag is a `--name` word anywhere after the command. List the flags a command
takes in `flags`, and `run` gets a table of them, each `true` or `false`, after
the positional arguments. In the router above, `/mya reset --all` calls
`reset`'s `run` with `{ all = true }`.

A flag the command doesn't declare is a usage error. Inside a `"rest"`
argument's text, `--name` is plain text.

## Help and usage

`/mya` alone, `/mya help` and an unknown command print one line per command,
with its arguments and help text, after the slash command. A wrong argument
prints the usage of just that command. The lines go to the chat through [`wowlua:log`](chat.md), after
your addon's name. For the router above, `/mya help` then `/mya scale big`
print:

```text
MyAddon: /mya limit [number] - Warn below this many free slots
MyAddon: /mya note <string> <text...> - Add a note to a bag
MyAddon: /mya reset [--all] - Reset the window position (--all: every setting)
MyAddon: /mya scale <number> - Set the window scale
MyAddon: /mya show - Show the window
MyAddon: /mya sort <name|quality|type> - Choose how items are sorted
MyAddon: Usage: /mya scale <number>
```

Commands are listed by name. Pass `{ order = "added" }` as the router's second
argument to list them in the order they were added: the definitions first, by
name, since a table has no order, then each `add`. `commands:help()` returns
the same lines as one string, for a help panel of your own (outside a slash
command's handler, after your addon's first slash command).

## Aliases

`aliases` gives a command more names. `/mya open` runs `show` in the router
above. Aliases match in any case, like command names, and aren't listed in the
help.

## Commands that come and go

A part of your addon can add commands when it starts and remove them again.
Pass an owner to `add`, and `removeAll(owner)` removes everything it added.
`commands()` lists every command with its name, usage, help, aliases and
owner, in help order, for example to fill a command palette.

```wowlua title=src/palette.wowlua
import router from "wowlua:slash"
import print from "wowlua:log"

local commands = router({})
local errorsModule = {}

commands:add("errors", {
    args = { "rest" },
    help = "List recent errors",
    run = function(filter: string) print("errors matching", filter) end,
}, errorsModule)

for _, c in ipairs(commands:commands()) do
    print(c.usage, c.help)
end

commands:remove("errors")           -- one command, with its aliases
commands:removeAll(errorsModule)    -- everything this owner added
```

A literal command passed to `add` is checked like the definitions (E8112).
Adding a name or alias that is already taken, in any case, is an error.

> [!TIP]
> In tests, `mock.slash("/mya scale 1.2")` types a command as the player would.
> See [Testing your addon](../guide/testing.md).

The full rules are in the [specification](../../spec/stdlib/core.md#slash).
