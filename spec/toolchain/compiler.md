# 13. Compiler Architecture

The compiler, package manager and tooling ship as **one Rust binary**,
`wowlua`. This chapter specifies its internal architecture. It is normative
for module boundaries and pipeline order, and advisory for library choices.

## 13.1 Requirements

| Requirement | Target |
|-------------|--------|
| Language | Rust stable, edition 2024. MSRV: latest stable minus 2 releases. |
| Platforms | macOS (arm64, x86_64), Windows (x86_64, arm64), Linux (x86_64, arm64, musl static) |
| Distribution | Single self-contained binary. No runtime dependencies besides an optional system `git`. |
| Performance | Full check + build of 100k lines of wowlua in under 1 s on a 2024 laptop. Incremental rebuild of one changed file in under 100 ms. |
| Determinism | Byte-identical output for identical inputs, compiler version and options. No hash-map iteration order leaks into output. |
| Robustness | Never panics on any input. Randomized robustness tests (random bytes, token soup, mutated sources) through parser, lowering, emitter and formatter are part of CI. Deeply nested input is an error (E5001), never a stack overflow. |

## 13.2 Workspace layout

```
wowlua/
├── Cargo.toml                      # workspace
├── crates/
│   ├── wowlua-cli/                 # binary "wowlua": argument parsing, terminal UI
│   ├── wowlua-driver/              # orchestration: build graph, profiles, flavors, caching, watch
│   ├── wowlua-span/                # FileId, Span, SourceDb, line index
│   ├── wowlua-diagnostics/         # Diagnostic model, codes, rendering (human + JSON)
│   ├── wowlua-syntax/              # lexer, parser, AST, visitors, Lua 5.1 grammar tests
│   ├── wowlua-sema/                # name resolution, module graph, types, checker, const-eval
│   ├── wowlua-lower/               # wowlua AST + semantic info → Lua IR
│   ├── wowlua-luair/               # Lua 5.1 IR, optimization passes, emitter, minifier, source maps
│   ├── wowlua-runtime/             # Lua sources of the __wl runtime, feature-sliced (include_str!)
│   ├── wowlua-stdlib/              # wowlua:* modules (written in wowlua) + their .d.wowlua
│   ├── wowlua-manifest/            # wowlua.json model, JSON schema, validation, profile merging
│   ├── wowlua-flavors/             # flavor table data: interfaces, suffixes, client features, site IDs, deferred-API table
│   ├── wowlua-toc/                 # TOC generation and parsing
│   ├── wowlua-xml/                 # WoW UI XML: well-formedness, Script/Include references, path rewriting
│   ├── wowlua-pkg/                 # resolver, sources (path/git/archive/registry), cache, lockfile
│   ├── wowlua-publish/             # packaging (zip), uploaders (CurseForge, Wago, WoWI, GitHub, registry)
│   ├── wowlua-types-gen/           # wow-api generator (Blizzard docs + LuaLS converter) → .d.wowlua
│   ├── wowlua-test/                # test runner on an embedded Lua 5.1 VM + WoW mock environment
│   ├── wowlua-fmt/                 # formatter
│   └── wowlua-lsp/                 # language server
├── runtime/                        # (symlink or path) Lua runtime sources used by wowlua-runtime
├── stdlib/                         # wowlua standard library sources
├── tests/                          # end-to-end tests, corpus tests
└── xtask/                          # dev automation (schema generation, corpus fetch, release)
```

Dependency direction (an acyclic graph; arrows point from user to dependency):

```
cli       → driver, publish, test, lsp, fmt, types-gen
lsp       → driver, sema, syntax, fmt
test      → driver                       (builds with the test profile, then runs mlua)
publish   → driver, manifest, flavors
driver    → pkg, manifest, toc, xml, lower, sema, luair, runtime, stdlib, flavors, diagnostics
pkg       → manifest, toc, flavors, diagnostics
manifest  → flavors, diagnostics, span
toc       → flavors, span
xml       → span, diagnostics
lower     → sema, luair, syntax
sema      → syntax, flavors, diagnostics, span
luair     → span, diagnostics
syntax    → span, diagnostics
fmt       → syntax
types-gen → syntax, flavors               (+ mlua, to execute Blizzard's documentation tables)
runtime, stdlib, flavors                  (data crates, no dependencies)
```

`sema` does not depend on `stdlib`: the driver loads the standard library's
`.d.wowlua` declarations and sources from `wowlua-stdlib` and passes them to
`sema` as ordinary inputs, together with the compiler-known semantics of
framework decorators (a table keyed by `@intrinsic` id). Only the modules an
addon imports (transitively) are loaded; each compiles to
`wowlua/std/<name>.lua` with the module ID `wowlua:<name>`. The environment
variable `WOWLUA_STDLIB_DIR` replaces the shipped modules with a directory of
`<name>.wowlua` / `<name>.d.wowlua` files, for developing the standard library
(decision T79).

## 13.3 Pipeline

```
manifest ──► resolve deps ──► discover (project + libraries + stdlib) ──► parse (parallel)
                                     │                                        │
                                     └── XML parse ──── locale JSON ──────────┤
                                                                              ▼
 ┌──────────────── for each build configuration (profile, or publish target) ─────────────────┐
 │  packager-keyword preprocessing of passthrough files (compat only, per flavor)              │
 │  ┌──────────────────────────── for each flavor (split mode) ───────────────────────────┐   │
 │  │  bind meta constants (profile, flavor, features, config)                            │   │
 │  │  static-condition pre-pass → static pruning (@if, static if-conditions)             │   │
 │  │  name resolution & scopes (per file)                                                │   │
 │  │  module graph: imports + usage analysis → load-time/run-time edges → load order     │   │
 │  │  declaration collection (+ implicit declarations, locale L type)                    │   │
 │  │  type checking (per function, parallel per module)                                  │   │
 │  │  lowering → Lua IR (+ locale files, Bindings.xml model)                              │   │
 │  │  IR passes (fold, DCE, instrument, inline, cache-globals)                            │   │
 │  │  bundling (optional) → hygiene → minification (level) → emission + source maps      │   │
 │  └──────────────────────────────────────────────────────────────────────────────────────┘   │
 │  assemble: merge flavors (shared vs per-flavor files, union TOC order), runtime slices,   │
 │            libraries, XML rewriting, Bindings.xml, LICENSES.txt, assets, TOC files         │
 │  output validation ──► atomic write of dist/<profile>/<Addon>/ or dist/targets/<target>/<Addon>/
 └─────────────────────────────────────────────────────────────────────────────────────────────┘
```

A build configuration is a profile (for `build`, `dev`, `install`, `test`) or a
publish target, which selects a profile plus overrides
([Publishing §11.1](publishing.md#111-configuration)). Configurations with
identical effective settings share one build.

### 13.3.1 Stages

| # | Stage | Crate | Input → Output | Notes |
|---|-------|-------|----------------|-------|
| 1 | Load manifest | manifest | `wowlua.json` → `Manifest` | `package.wowlua` is checked **before** schema validation (E4300), then JSON-schema validation, profile inheritance and merging, workspace expansion |
| 2 | Resolve | pkg | `Manifest` + lockfile → `ResolvedGraph` | Fetches sources into the cache. `--locked` / `--frozen` / `--offline` honored. Pins the flavor table version ([Package manager §10.6](package-manager.md#106-lockfile)). |
| 3 | Discover | driver | file system → `SourceDb` | Project sources (`build.src`, `build.exclude`, `.gitignore`), dependency sources, used standard-library modules, implicit types packages, locale JSON files, XML files, assets |
| 4 | Parse | syntax, xml | bytes → `Ast` / `XmlDoc` + diagnostics | Parallel per file (rayon). Error-recovering. XML `<Script>`/`<Include>` references are collected. Parsing is shared by all configurations, except for files changed by stage 5. |
| 5 | Preprocess | driver | passthrough text → text | Only with `compat.packager-keywords`: BigWigs keyword substitution on passthrough `.lua`/`.xml` files, per configuration and flavor ([CLI §12.6](cli.md#126-adopting-an-existing-addon)). Affected files are re-parsed per flavor. |
| 6 | Locales | lower, sema | locale JSON → Lua IR + `L` type | Key and placeholder checks, generation of the `L` record type used by stage 11 |
| 7 | Meta binding | sema | configuration, flavor → `ConstEnv` | Values of `wowlua:meta` exports (for libraries: the consuming addon's `addon`/`build`/`flavor`, the library's own `features`/`config`) |
| 8 | Static pre-pass | sema | ASTs + `ConstEnv` → constant values | Evaluates `const` initializers that are static, in syntactic import order. Static conditions may only use `wowlua:meta` paths, literals and such constants; `@if` conditions on `import` statements may only use `wowlua:meta` paths and literals, so that the per-flavor import graph can be built without type information. |
| 9 | Static pruning | sema | AST + constants → pruned view | Marks dead branches and removed declarations, including imports removed by `@if`. Later stages skip them. |
| 10 | Resolve names | sema | AST → `Scopes`, `Bindings` | Per file: locals, upvalues, globals, `ns` fields, imports, decorators. Needs no other module. |
| 11 | Module graph | sema | imports + bindings → `ModuleGraph` | Per flavor. Import edges plus usage-analysis edges for globals and addon-table fields, classified load-time / run-time (§13.6.1). Cycle check, lazy bindings, load order. |
| 12 | Collect declarations | sema | → `TypeEnv` | Classes, interfaces, enums, aliases, `declare`s in load order, plus **implicit declarations**: `wow-api-<flavor>`, WoW's Lua environment, SavedVariables globals, `check.globals`, `AddonNamespace`, the `L` type, and the generated declaration files of workspace addons listed in `addon-dependencies`. Workspace declaration files are produced from this stage's output, so mutually dependent addons do not need each other's full build. |
| 13 | Type check | sema | → `TypeTable` + diagnostics | Bidirectional checking per function body. Dependency and standard-library modules are checked with **their own package's** check settings. |
| 14 | Lower | lower | AST + sema → `lua::Chunk` | Desugaring, classes, decorators, modules (copied vs lazy bindings), debug features, keybinding declarations → `Bindings.xml` model |
| 15 | IR passes | luair | `Chunk` → `Chunk` | See [Optimization](optimization.md) |
| 16 | Bundle | luair | many chunks → one or a few chunks | Release layout. XML-referenced modules stay separate. |
| 17 | Hygiene | luair | chunk → chunk | Renames generated identifiers that collide. Runs after bundling, over each final output chunk ([Code generation §14.3](codegen.md#143-hygiene)). |
| 18 | Minify | luair | `Chunk` → `Chunk` | Level-dependent, within the Lua-limit budgets |
| 19 | Emit | luair | `Chunk` → text + source map | According to `line-mode` |
| 20 | Assemble | driver, toc, xml | → output tree | Merges flavors (byte-identical files stored once), union TOC order, runtime slices, meta tables, libraries (entry files, XML-reachable files and licenses only), XML path rewriting, `Bindings.xml`, `LICENSES.txt`, assets, TOC files |
| 21 | Validate | driver | output tree → ok / errors | See §13.8 |
| 22 | Write | driver | → `dist/` | Write to a temp directory, then replace the target directory (§13.12) |

Stages 7–19 run once per flavor in `split` mode, and once in `shared` mode.
Stages 1–4 and 6 are shared by all configurations and flavors.

## 13.4 Core data structures

- **`SourceDb`** stores file contents, `FileId`, path, line index and hash.
  Spans are `(FileId, u32 start, u32 end)` byte offsets.
- **Interning.** Identifiers and string literals use a global interner
  (`Symbol(u32)`). Types are hash-consed in a `TypeInterner` (`TypeId(u32)`).
- **AST.** Typed enums per node kind, allocated in per-file arenas. Every node
  has a `NodeId` and a `Span`. Comments are kept as trivia attached to the
  following token. The formatter needs them, and they are discarded at
  emission unless kept by the profile.
- **Side tables.** Semantic results are stored in maps keyed by `NodeId`
  (`IndexVec`/`FxHashMap`), not in the AST. The AST is immutable after parsing.
- **Lua IR.** A separate, small AST that can only represent Lua 5.1. Every
  statement and expression carries an `Origin` (source span, or "generated with
  anchor span"), which drives line preservation and source maps.
- **Diagnostics.** `Diagnostic { code, severity, message, primary: Span,
  labels: Vec<(Span, String)>, notes, help, fixes: Vec<Fix> }`. Fixes are
  machine-applicable edits, used by the LSP and `wowlua fix`.

## 13.5 Front end

### 13.5.1 Lexer

A hand-written, byte-oriented lexer. It implements Lua 5.1's lexical
grammar exactly, including long brackets, numeral reading and escape rules,
plus the wowlua tokens. Contextual keywords are lexed as `Name` and recognized
by the parser. Backtick strings produce a token stream of fragments and
embedded-expression boundaries.

### 13.5.2 Parser

Hand-written recursive descent, with Pratt parsing for expressions (the
precedence table in [Syntax §1.4.6](../language/01-syntax.md#146-operator-precedence)).

- **Contextual keywords** are recognized with one or two tokens of
  lookahead, following the rules in
  [Syntax §1.2.5](../language/01-syntax.md#125-reserved-and-contextual-keywords).
- **Type contexts** have their own sub-parser. `>>`, `>=` and `>>=` are split
  into their component tokens there.
- **Error recovery.** On an error the parser synchronizes at statement
  boundaries (`local`, `function`, `end`, `if`, `for`, `while`, `repeat`,
  `return`, `class`, …). It produces `Error` nodes and continues, so that one
  run reports multiple independent errors and the LSP gets a usable tree.
- **Lua 5.1 quirks.** It reports the "ambiguous syntax (function call x new
  statement)" error exactly where Lua 5.1 does, so that pure Lua files have the
  same accept/reject behavior.

### 13.5.3 Superset conformance

The parser has a **Lua 5.1 compatibility suite**:

1. the official Lua 5.1 test suite files must parse,
2. a corpus of popular open-source addons (pinned commits, fetched by `xtask
   corpus`) must parse, check in basic mode with **zero errors**, and
3. round-trip: for every corpus file, `emit(lower(parse(file)))` compiled by a
   reference Lua 5.1 compiler with debug info stripped must produce **the same
   bytecode** as the original file. In `preserve` line mode the line info must
   be identical as well, so in-game error lines match the source.

`cargo xtask corpus fetch` downloads the pinned corpus (`xtask/corpus.txt`,
checked by SHA-256) and `cargo xtask corpus run` runs these checks plus the
formatter's (meaning preserved, idempotent). A scheduled CI job runs the full
corpus; regular CI runs `tests/corpus-sample/` (decision T66).

## 13.6 Semantic analysis

- **Scopes.** Lua's block scoping is modelled exactly, including the rule that a
  `local` is visible only after its declaration statement, and `repeat …
  until` condition scoping. The binder records each local's captures and
  assignments, which narrowing invalidation and const checks use.
- **Module graph.** Built per flavor after static pruning and name resolution,
  and before type checking (§13.6.1).
- **Type checker.** Bidirectional:
  - `check(expr, expected)` and `infer(expr)`, with flow-sensitive narrowing
    computed on a per-function control-flow graph,
  - generic inference by unification of argument types against parameter types,
    with widening of literals at inference boundaries,
  - overload resolution by trying candidates in order.
  The checker never mutates the AST. Unknown or erroneous types become `Error`,
  which is assignable to everything, so one error does not cascade.
- **Const evaluation.** A small interpreter over constant expressions,
  covering numbers, strings, booleans, `nil`, arithmetic with Lua 5.1 semantics
  (double precision, `%` as `a - floor(a/b)*b`) and concatenation with Lua's
  number formatting (`%.14g`). It is used for `const`, enum values, static
  conditions and folding.

### 13.6.1 Load-order analysis

This implements the load-order rules of
[Modules §5.4](../language/05-modules.md#54-load-order):

1. **Edges.** An edge `A → B` (A must be considered before B) comes from:
   - an `import` in B of module A (the import's bindings mark which uses count),
   - usage analysis: B reads a global or an addon-table field (`ns.Foo`,
     including through locals aliasing `ns`) that A assigns at its top level.
     This also covers plain `.lua` files and XML (globals and templates an
     element needs when it is created, named frames as definitions).
2. **Classification.** Each use is **load-time** if it can execute while B's
   file loads: in a top-level statement, or in a function that is reachable
   from top-level code through the call graph, followed **across modules** using
   per-function summaries of what each function may call and read. A function value passed to
   a parameter marked *deferred* in the declarations (e.g. `SetScript`
   handlers, `HookScript`, `C_Timer.After`, `hooksecurefunc`, `@event` /
   `@slash` / lifecycle handlers) does not make its body load-time. A function
   passed to a parameter marked *immediate* (e.g. `pcall`, `xpcall`,
   `securecallfunction`, `table.sort` comparators) does. A function passed to
   a package function is load-time only if the callee's summary says it calls
   (or forwards) that parameter; passed to an **unknown** callee it is treated
   as load-time. The deferred/immediate markers come from the flavor table and
   the declarations. Guarded uses (existence tests and the code they guard)
   do not create edges (decision T70).
3. **Order.** A deterministic topological sort over load-time edges. Among
   ready files, the order of `build.load-order` (into which `wowlua init` writes
   the old TOC order), then byte-wise path order, breaks ties.
4. **Cycles.** Strongly connected components (Tarjan's algorithm). A cycle of
   load-time edges alone is error E1014; the diagnostic shows every edge of the
   cycle and the call chain that makes it load-time. Over all edges, a run-time
   import inside a component becomes a **lazy binding**, lowered as a field
   read on the imported module's export table (`__wl_m3.Foo`) instead of a
   local copy; outside cycles, run-time imports still order the files
   (decision T72). Usage edges that contradict `build.load-order` are dropped
   with W1035 before cycles are searched.
5. **Ambiguity.** Dynamic names (`_G["x" .. i]`), several writers of the same
   global or field, or `setfenv` make an edge undecidable. The files involved
   keep their relative source order and warning W1033 names the cause.

### 13.6.2 Lints

Lints run in `wowlua check`, `wowlua build` and `wowlua lint` (which reports
only lints, and errors that stop the analysis) on `.wowlua` files, and on
`.lua` files when `check.lua` is set. `--!nolint` turns them off for a file;
`check.lints` changes their level (decisions T90–T93).

- **W1003, unused binding.** A local, `const`, local function, destructured
  field, import, parameter or loop variable that is never read. Assigning a
  local is not a read. Names starting with `_` and `self` are exempt, as are
  exported and decorated declarations. A use in a type position (an imported
  type) or in a conditionally eliminated branch counts. Parameters and loop
  variables are positional: only those after the last used one of the same
  list are reported. Top-level `function f()` declarations of a module are
  hoisted locals and are reported like local functions.
- **W1004, shadowing.** A local, local function, loop variable or parameter
  (also of a nested function) that reuses the name of a visible local of an
  enclosing scope. Redeclaring a local in the same block (`local x = f(x)`)
  is not reported.
- **W1005, unreachable code.** The first statement after `return`, `break`,
  `continue`, or a call whose type returns `never` (`error`), or after an `if`
  whose branches all end that way; once per region. Reported by the type
  checker, so not in `--!nocheck` files.

**Fixes** (`wowlua fix`, `wowlua lint --fix`, the LSP). A statement whose
names are all unused is removed when its values have no side effects
(literals, names, functions, and tables of those); its line goes with it when
nothing else is on it. Other unused locals, parameters and loop variables are
renamed `_name`, unless the file already uses `_name`. An unused import name
is removed; an import without used names is removed when it has no load-order
effect (`import type`, `wowlua:` modules), and otherwise becomes
`import _ from "…"`, which keeps the load order. Destructured fields, W1004 and
W1005 get no fix. Fixes are applied in rounds until none is left, because one
fix can make another one possible.

## 13.7 Back end

Covered in detail by [Code generation](codegen.md) and
[Optimization](optimization.md). Architecture rules:

- Lowering produces Lua IR only. It never produces text.
- Every IR pass is a function `fn(&mut Chunk, &PassCtx)`. The pass list is chosen
  by the profile and minification level. Passes are individually testable.
- The emitter is the only component that produces Lua text. It handles
  precedence-based parenthesization, statement separators (`;` where Lua would
  otherwise see an ambiguous call), line preservation and source maps.

## 13.8 Output validation

Before anything is written, the assembled output must pass these checks. A
failure is an internal compiler error (exit code 101) if caused by codegen, or a
user error otherwise:

1. **References.** Every file referenced by a TOC exists. Every output Lua or XML
   file is **reachable**: listed in a TOC, or referenced through a `<Script>` /
   `<Include>` chain starting at a TOC-listed XML file. `Bindings.xml` (loaded by
   the client automatically), `LICENSES.txt` and assets are exempt. Violations
   are E5002.
2. **Lua syntax.** Every emitted Lua file parses as Lua 5.1 with the compiler's
   own Lua 5.1 parser. In the compiler's own CI it is also loaded by the
   embedded Lua 5.1 VM.
3. **Lua limits.** No function exceeds Lua 5.1's limits: 200 active locals,
   60 upvalues, about 250 registers ("function or expression too complex"),
   200 nested syntax levels (C-call depth, including the extra levels added by
   bundle wrappers), 262 143 constants, and jump offsets of ±131 071
   instructions. Optimization passes stay within budgets for these limits
   ([Optimization §15.2](optimization.md#152-profile-controlled-optimizations)).
   A user function that alone exceeds a limit is error E5001, naming the
   function and the limit.
4. **XML.** Well-formed, and `<Script file="…">` / `<Include file="…">`
   references resolve after rewriting ([Code generation §14.8.4](codegen.md#1484-xml)).
5. **TOC.** Interface numbers are present for every flavor, file paths use the
   exact case and `\` separators, and no line exceeds 1024 characters (E4303).
6. **SavedVariables** names are valid Lua identifiers and unique across the
   workspace's addons (E4120).
7. **Assets.** Every path folded by `asset()` exists among the output's assets
   (E4307).
8. **Paths.** No two output paths are equal or differ only in case (E4311 for
   collisions with reserved names, E4308 for case collisions).
9. **Licenses.** Every embedded library contributed license text to
   `LICENSES.txt`; a library without a recognizable license file or header is
   warning W4306.

## 13.9 Caching and incremental builds

- **Phase 1 (MVP).** Full rebuilds. Parsing is cached by file content hash in
  memory during `--watch` and `wowlua dev`.
- **Phase 2.** Persistent per-module cache in `dist/.cache/` keyed by the
  module's content hash, the compiler version, the effective configuration
  hash (profile, target overrides, feature set), the flavor, the flavor-table
  version, the locked types-package versions, the lockfile hash, and the
  export-interface hashes of its imports. A module is re-checked only if its
  own key changes. Dependents are re-checked only if its **export interface
  hash** (the types of its exports) changes. A different compiler version
  invalidates the whole cache.
- **LSP.** Uses the same queries in a demand-driven database (`salsa` is the
  intended framework), sharing code with the batch compiler.

## 13.10 Suggested crates

| Purpose | Crate |
|---------|-------|
| CLI | `clap` (derive) |
| JSON manifest, preserving formatting on edit | `serde`, `serde_json`, `jsonc-parser`-style CST editing for `wowlua add` (`json-patch` fallback) |
| JSON schema | `schemars` (generated from the manifest types) |
| Diagnostics rendering | `codespan-reporting` |
| Parallelism | `rayon` |
| Hashing | `rustc-hash` (in-memory), `blake3` (content hashes), `sha2` (lockfile checksums) |
| Deterministic maps | `indexmap` |
| Semver and resolution | `semver`, `pubgrub` |
| Git | system `git` via `std::process` (MVP), `gix` later |
| HTTP | `ureq` with rustls |
| Archives | `zip`, `flate2`, `tar` |
| File watching | `notify` |
| Globs | `globset`, `ignore` |
| Embedded Lua 5.1 (tests, validation) | `mlua` with features `lua51`, `vendored` |
| LSP | `tower-lsp` (or `lsp-server`) |
| Snapshot tests | `insta` |
| Deep recursion | `stacker` (grows the stack per nesting level) |
| Logging | `tracing` |

## 13.11 Testing the compiler

| Layer | Method |
|-------|--------|
| Lexer and parser | Unit tests, accept/reject parity with the embedded Lua 5.1 on hand-picked and randomized inputs, randomized robustness tests (no panics, recovery terminates) |
| Superset | Lua 5.1 test suite, addon corpus, bytecode round-trip (§13.5.3) |
| Type checker | `.wowlua` test files with inline expectations (`--~ E2001` on the offending line) |
| Lowering | Snapshot tests of emitted Lua per feature, plus **differential execution**: run the wowlua source's expected behavior as embedded Lua and compare printed output |
| Minifier | Every corpus file minified at every level must behave identically in the VM (the same printed trace for test scripts) |
| End to end | Example projects built for every flavor. The output is validated and loaded in the mock WoW environment. |
| Package manager | Local git and HTTP fixtures, resolver property tests |

## 13.12 Platform considerations

- **Paths.** Internally, paths are UTF-8 with `/`. TOC file lists use `\`, zip
  entries use `/`. Output paths that collide case-insensitively are errors on
  every platform (E4308), because WoW on Windows would load only one of them.
- **Long paths.** On Windows, file operations use extended-length (`\\?\`)
  paths, so deep `dist/` trees work without the 260-character limit.
- **Atomic replacement.** On macOS and Linux, the output directory is replaced
  by renaming the finished temp directory over it. Windows cannot rename over a
  non-empty directory, so the old directory is first renamed aside, the new one
  renamed into place, and the old one deleted. If the process dies in between,
  the next build cleans up the leftover `*.wowlua-old` directory.
- **Links.** `wowlua dev --link` uses symlinks on macOS and Linux and directory
  junctions on Windows (no administrator rights needed). If neither can be
  created, it falls back to copying with warning W4309.

## 13.13 Privacy and messages

- **No telemetry.** The compiler never sends usage data. Network access happens
  only for dependency fetching, registry access, `wowlua types sync`,
  publishing, and the site-version lookups publishing needs.
- **Messages are English only.** Diagnostic codes are stable and searchable.
- **Offline documentation.** `wowlua explain <code>` texts are embedded in the
  binary and work offline.
