# wowlua documentation

How to write, test, debug and ship World of Warcraft AddOns with wowlua, in
reading order. The [specification](../spec/README.md) is the complete,
normative reference behind these pages.

## Guide

- [Installation](guide/install.md) — install the wowlua binary on macOS, Linux or Windows, update it and pin a version
- [Getting started](guide/getting-started.md) — create an addon, run it in the game, test it and build a release
- [VS Code](guide/vscode.md) — errors as you type, completion, formatting and the debugger in the editor
- [Testing your addon](guide/testing.md) — `wowlua test`: tests in the emulated client, mocks, world profiles and screenshots
- [Debugging](guide/debugging.md) — the dev window in game, `wowlua trace`, and the debugger in the emulated client
- [AI agents](guide/agents.md) — `wowlua mcp`: let an agent run, see and debug your addon
- [Libraries and dependencies](guide/packages.md) — depend on libraries and other packages, and share your own
- [Building and publishing](guide/publishing.md) — release builds, and uploads to CurseForge, Wago, WoWInterface and GitHub
- [Bring your addon](guide/adopting.md) — move an existing addon to wowlua, including one built with the BigWigs packager
- [Command cheat sheet](guide/commands.md) — every `wowlua` command at a glance

## Language

- [A tour of wowlua](language/tour.md) — the language on one page, for Lua programmers
- [Syntax and sugar](language/syntax.md) — what wowlua adds to Lua's syntax, and the Lua it compiles to
- [Types](language/types.md) — annotations, inference, narrowing, generics and strict mode
- [Functions](language/functions.md) — typed parameters and returns, optional and variadic parameters
- [Classes](language/classes.md) — classes, inheritance, methods and fields
- [Interfaces and enums](language/interfaces.md) — structural interfaces and enums
- [Frames as classes](language/widgets.md) — widget classes: frames and their scripts as classes
- [Decorators](language/decorators.md) — the built-in decorators, and writing your own
- [Modules](language/modules.md) — `import` and `export`, load order and plain Lua files
- [Build info and profiles](language/flavors.md) — `wowlua:meta`, flavors, build profiles and conditional compilation
- [Debugging helpers](language/debugging.md) — `wowlua:debug`, `wowlua:reflect`, profiling, tracing and runtime checks

## Standard library

- [Overview](stdlib/overview.md) — the `wowlua:*` modules and what each is for
- [Events and lifecycle](stdlib/events.md) — `wowlua:events` and `wowlua:lifecycle`
- [Saved variables](stdlib/saved.md) — `wowlua:saved`: typed saved variables with defaults, profiles and migrations
- [Slash commands](stdlib/slash.md) — `wowlua:slash`: commands, subcommands and arguments
- [Timers and background jobs](stdlib/timers.md) — `wowlua:timer` and `wowlua:jobs`
- [Settings, minimap button and keybindings](stdlib/settings.md) — `wowlua:settings`, `wowlua:launcher` and `wowlua:bindings`
- [Localization](stdlib/locale.md) — `wowlua:locale`: typed translations
- [Frames, lists and menus](stdlib/ui.md) — `wowlua:frames`, `wowlua:pool`, `wowlua:list`, `wowlua:menu`, `wowlua:dialog`, `wowlua:color` and `wowlua:media`
- [Chat, logging and alerts](stdlib/chat.md) — `wowlua:log`, `wowlua:chat`, `wowlua:links` and `wowlua:alert`
- [Talking to other addons](stdlib/comm.md) — `wowlua:comm`, `wowlua:serialize`, `wowlua:callbacks` and `wowlua:version`
- [Units, auras, spells and items](stdlib/game.md) — `wowlua:unit`, `wowlua:aura`, `wowlua:spell`, `wowlua:item`, `wowlua:tooltip` and `wowlua:secret`
- [Combat, secure code and hooks](stdlib/combat.md) — `wowlua:combat`, `wowlua:secure` and `wowlua:hooks`
- [Tables, text and formatting](stdlib/utils.md) — `wowlua:table`, `wowlua:collections`, `wowlua:utf8`, `wowlua:format` and `wowlua:errors`

## Reference

- [WoW API](reference/api.md) — every function, event and widget method of the game, and how far the emulated client implements it
- [wowlua.json](reference/manifest.md) — the manifest, field by field
- [Errors and warnings](reference/diagnostics.md) — what the codes mean, and every error and warning by area
- [Emulator coverage](emulator-coverage.md) — how much of the WoW client `wowlua test` emulates, verified against the real client (generated)
- [Embedding wowlua](reference/embedding.md) — `@wowlua/web`: the compiler, language server and emulated client as WebAssembly
