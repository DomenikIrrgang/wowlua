# wowlua Specification

Status: **Draft 0.1** · Last updated 2026-10-11

wowlua is a language, compiler, framework and package manager for World of
Warcraft AddOns. This directory is the normative specification. The compiler
(written in Rust) is implemented against it.

## Documents

| # | Document | Contents |
|---|----------|----------|
| 0 | [Overview](00-overview.md) | Goals, principles, the superset guarantee, project anatomy, terminology |
| **Language** | | |
| 1 | [Syntax](language/01-syntax.md) | Lexical structure, statement and expression extensions, syntax sugar |
| 2 | [Types](language/02-types.md) | Type syntax, gradual checking, inference, narrowing, declaration files |
| 3 | [Classes, interfaces, enums](language/03-classes.md) | Classes, inheritance, widget classes, interfaces, enums |
| 4 | [Decorators](language/04-decorators.md) | Decorator syntax and semantics, built-in decorators, user decorators |
| 5 | [Modules](language/05-modules.md) | `import` / `export`, resolution, load order, interop with plain Lua |
| 6 | [Build metadata & conditional compilation](language/06-meta.md) | `wowlua:meta`, flavors, profiles, compile-time folding |
| 7 | [Debugging & reflection](language/07-debugging.md) | `wowlua:debug`, profiling, tracing, runtime checks, reflection |
| **Standard library** | | |
| 8 | [Standard library](stdlib/README.md) | WoW-specific framework modules (`wowlua:*`) |
| **Toolchain** | | |
| 9 | [Manifest](toolchain/manifest.md) | `wowlua.json` reference |
| 10 | [Package manager](toolchain/package-manager.md) | Packages, sources, resolution, lockfile, embedding |
| 11 | [Publishing](toolchain/publishing.md) | Packaging and upload to CurseForge, Wago, WoWInterface, GitHub |
| 12 | [CLI](toolchain/cli.md) | `wowlua` command reference |
| 13 | [Compiler architecture](toolchain/compiler.md) | Rust implementation: crates, pipeline, data structures |
| 14 | [Code generation & output](toolchain/codegen.md) | Lowering, runtime library, output layout, TOC generation |
| 15 | [Optimization](toolchain/optimization.md) | Bundling, minification levels, folding, tree-shaking |
| 16 | [Developer tooling](toolchain/tooling.md) | Test runner, language server, formatter, trace decoder |
| 17 | [WoW client emulation](toolchain/emulator.md) | The test VM and engine: WoW's Lua, taint, secret values, the generated API, FrameXML (planned, M9) |
| 18 | [Debugger, profiler and UI rendering](toolchain/debugger.md) | `wowlua debug` (DAP), `wowlua mcp`, the profiler, the scene, assets, the renderer, screenshots (planned, M12) |
| **Appendices** | | |
| A | [Grammar](appendix/grammar.md) | Complete grammar (EBNF) |
| B | [Diagnostics](appendix/diagnostics.md) | Error and warning codes |
| C | [Decision log](appendix/decisions.md) | Design decisions and their rationale |
| — | [Roadmap](roadmap.md) | Implementation milestones |

## Conventions

- **MUST**, **MUST NOT**, **SHOULD**, **MAY** are used as in RFC 2119.
- `wowlua` code blocks are wowlua source. `lua` code blocks show the Lua 5.1
  the compiler emits. Emitted code in this spec is shown *pretty-printed* for
  readability. Real output follows the layout rules in
  [Code generation](toolchain/codegen.md).
- Names starting with `__wl` in emitted code are compiler-generated. The compiler
  makes them hygienic (see [Code generation](toolchain/codegen.md#143-hygiene)).
- "Lua" without a version always means **Lua 5.1 as embedded in World of
  Warcraft**.
