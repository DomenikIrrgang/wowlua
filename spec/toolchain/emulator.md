# 17. WoW Client Emulation

**Status: planned (M9).** This chapter describes the environment `wowlua test`
runs addons in ([Tooling §16.1](tooling.md#161-test-runner-wowlua-test)): a Lua
VM and a WoW engine written in Rust, faithful enough that a test catches what
the game client would raise.

## 17.1 Goals

- **Fidelity over convenience.** A test passes only if the same code would run
  in the client of that flavor. Where the emulation cannot know the client's
  behaviour, it raises a clear error instead of guessing.
- **The bugs that matter.** Taint, combat lockdown, secret values, flavor
  differences and WoW's own Lua differences are where addons break in game; the
  emulation models them, and reports them better than the game does.
- **Verified against the client.** Every behaviour is backed by the Lua 5.1
  reference, Blizzard's documentation and UI code, or an in-game probe
  (`/stdtest probe`), and the verification is automated (§17.8).
- **One binary.** The VM and the engine are part of `wowlua`, in Rust, with no
  C toolchain (decision T185).

## 17.2 The VM

A Lua 5.1 virtual machine in Rust, in its own crate (`wowlua-vm`).

- **Language.** Lua 5.1 as the client runs it: values, tables with metatables
  and every metamethod, closures and upvalues, varargs, coroutines,
  `setfenv`/`getfenv`, weak tables, `__gc` on userdata, error values and
  `pcall`/`xpcall` with tracebacks, `string.dump` absent.
- **Input.** The VM runs what the compiler produces: wowlua's output and plain
  Lua files, with the compiler's source maps, so every location the VM reports
  is a `.wowlua` (or `.lua`) line and column.
- **Standard library.** Exact Lua 5.1 behaviour: Lua patterns, `string.format`
  (including `%g` and `%q` output byte for byte), `tostring` of numbers, table
  sorting, number parsing.
- **Determinism.** Iteration order, random numbers (`math.random`,
  `fastrandom`, seeded per test), time and memory are deterministic, so a test
  run repeats exactly.
- **Instrumentation.** Coverage counters (lines, statements, branches,
  functions, conditions) and profiling are counted by the VM, not inserted into
  the code (§17.6, Tooling §16.1.6).
- **Limits.** Lua's limits apply as in the client (200 locals, 60 upvalues,
  C stack depth), so code that compiles but would not load in game fails here
  too.

Its design (§17.9): a register bytecode of its own, a tracing garbage
collector, and stackless coroutines.

## 17.3 WoW's Lua

The differences from Lua 5.1 elsewhere, as probed on WoW: Forever (decisions
S37, S38):

- **Libraries.** No `os`, `io`, `debug` or `package`; `date`, `time` and
  `difftime` as globals. The client's additions to `string`, `table` and `math`
  ([Types §2.8](../language/02-types.md#28-globals)), `bit`, and WoW's global
  aliases and helpers (`strsplit`, `strjoin`, `strtrim`, `tinsert`, `wipe`,
  `debugstack`, `debuglocals`, `debugprofilestop`, `geterrorhandler`, …).
- **Arithmetic.** `x / 0` and `x % 0` raise "Division by zero"; `math.sqrt`
  and `math.log` raise on a negative argument; `string.format("%d", x)` raises
  for an infinite `x`.
- **Limits.** The client's limits, at its thresholds: `strsplit`'s and
  `unpack`'s errors for too many results, and the calls that crash it (a
  few thousand results trip its Lua assertion, `L->top < L->ci->top`), which
  fail with an error naming the call (decision T194). The thresholds come
  from crash-search probes (`/stdtest crashlimit`), separate from
  `/stdtest probe`.
- **Per flavor.** Where flavors differ (a library function, an arithmetic
  rule), the VM follows the flavor under test. Behaviour not yet probed on a
  flavor follows the newest probed client and is listed in the emulation's
  coverage report (§17.8).

### 17.3.1 Layers

The client's in-game environment, sorted from the client itself
(`/stdtest inventory`, recorded in `crates/wowlua-vm/tests/client/`) against
Blizzard's UI code and API documentation for the build:

| Layer | What | Where wowlua has it |
|-------|------|---------------------|
| Lua runtime | Lua 5.1's libraries and the client's C extensions: the string, table, math and bit additions, the aliases (`strfind`, `tinsert`, …), `date`/`time`, `debugstack`, the secure API, and the FrameScript functions (`issecretvalue`, `securecallmethod`, `Mixin`, …) | the VM (`Vm::wow`) |
| Engine API | C functions: the documented API (`C_*` and globals) and older undocumented ones (`GetActionInfo`, …) | the engine (§17.6) |
| Blizzard's Lua | FrameXML and SharedXML: `print`, `tostringall`, `assertf`, the degree-based `sin`/`cos`, mixins, and Lua wrappers even inside `C_` namespaces (`C_Traits.GetEntryInfo`) | Blizzard's own code, loaded by the engine (§17.7) |
| GlueXML | the login and character screens' Lua | not in game; not emulated |

On WoW Forever 1.60.1 (70374): 5,037 documented C functions, 2,059
undocumented ones (151 of them the runtime's), 3,942 Lua functions, every one
defined in the published UI code for the build.

## 17.4 Taint and secure execution

The client's security model, in the VM:

- **Secure and insecure code.** Blizzard's code (the engine's Lua and, when
  loaded, FrameXML, §17.7) runs secure; addon code runs insecure, tainted with
  its addon's name.
- **Propagation.** A value written by insecure code carries that taint; secure
  code that reads a tainted value becomes tainted for the rest of its
  execution. Taint follows table fields, globals, upvalues and locals, and
  crosses coroutines as in the client.
- **The secure API.** `issecure`, `issecurevariable`, `securecall`,
  `securecallfunction`, `secureexecuterange`, `hooksecurefunc` (hooks run
  insecure without tainting the hooked call), `forceinsecure`, `scrub`, and the
  taint barriers the client has (error handlers, `__gc`).
- **Protected actions.** Protected functions and frame methods (from
  Blizzard's documentation and UI code, §17.5) are refused from tainted
  execution: some always, some only in combat lockdown. A refusal raises
  `ADDON_ACTION_BLOCKED` or `ADDON_ACTION_FORBIDDEN` as in the client **and**
  fails the test.
- **Taint reports.** A refusal reports the taint chain: the protected call,
  the secure code that became tainted, the value it read, and the addon code
  (file and line) that wrote that value. This is what `taint.log` cannot show.

## 17.5 Secret values

Secret values (the 12.x clients) are a native value kind of the VM: what may be
done with them (pass, store, hand to the APIs that take them) works; what may
not (compare, do arithmetic, index, concatenate, test) raises the client's
errors. `issecretvalue`, `canaccessvalue` and the related API behave as in the
client, and the engine returns secrets where the client does (from the
documentation's `SecretReturns` and `SecretArguments` flags).

## 17.6 The engine

The WoW API the VM's code calls, per flavor.

- **Generated.** Every function, namespace, event, enum and widget method is
  generated from the flavor's API documentation (the same source as the
  `wow-api` declarations, decision T83): its signature, its existence in the
  flavor, whether it is protected or restricted, and which of its arguments and
  results are secret. A function the flavor lacks is absent, as in the client.
- **Behaviour.** Functions have hand-written behaviour where tests need it:
  the widget system (frames, regions, textures, font strings, scripts, events
  and unit events, points and sizes, show and hide, parenting, templates,
  mixins, protected frames), the event loop and `OnUpdate` frames, timers on a
  virtual clock, combat lockdown, units, auras, spells and items (from test
  data), saved variables across reloads, slash commands, keybindings, addon
  messages, settings, tooltips, menus and dialogs. The M8 test mock is the
  starting point.
- **Not mocked.** A generated function without behaviour raises
  "`<name>` is not mocked" when called, unless the project provides a mock
  (Tooling §16.1.4), so gaps are visible instead of silently returning nil.
- **Engine-side Lua.** Lua the client defines in its engine (not FrameXML) is
  provided as secure code.

## 17.7 Blizzard's UI code

With `test.blizzard-ui` ([Manifest §9.18](manifest.md#918-test)), the test
runner loads the flavor's FrameXML (Lua and XML from wow-ui-source, at the
client version the API declarations come from) as secure code before the
addon. Tests then run against the real Edit Mode, Settings, action bars, unit
frames and the rest, so taint the addon spreads into Blizzard's code, and
Blizzard code that fails because of what an addon did, show up as in game.
Off by default: mock-only tests start faster (decision T186).

## 17.8 Verification

- **Lua 5.1 conformance.** Against PUC Lua 5.1 (embedded only in wowlua's own
  tests):
  - `cargo xtask corpus vm` compiles every file of the conformance corpus to
    the same bytecode and line info as `luac`, and runs its main chunk with
    the same output and error.
  - `cargo xtask corpus lua51` runs the official Lua 5.1 test suite. It
    excludes only what the client does not have, each with its reason: the
    `io`, `os`, `debug` and `package` libraries, `string.dump`, the implicit
    `arg` of vararg functions, `string.gfind`, and a 32-bit `size_t`
    overflow. Shims serve the suite's plumbing only (timing, temporary files
    in memory, loading its helper files).
  - Differential tests compare snippets' output and errors.
- **Sources of truth.** In-game probes on the current client first, then
  Blizzard's documentation and UI code for the build (the generated API
  documentation's `SecretArguments`, `ProtectedFunction`, `HasRestrictions`,
  `SecureHooksAllowed` and `SecretWhen…` flags, `FrameScriptDocumentation`).
  elune (an open reimplementation of an older client) is only a hint: a rule
  that rests on it alone is listed as unverified in the coverage report.
- **Taint.** StdTest's taint experiments (`/stdtest taint`) taint one value on
  a path the client's secure code takes to a protected function and record
  whether the action is blocked; the VM must agree. On WoW Forever 1.60.1
  (`tests/client/forever-camelot-taint.lua`): secure code reading a tainted
  global (Lua or C function) or table field is tainted, and the protected
  call raises `ADDON_ACTION_FORBIDDEN` naming the addon; `hooksecurefunc`
  leaves the hooked variable secure and the click allowed; `setfenv` taints
  the function itself (its variable stays secure, the click is forbidden). elune's taint cases
  (vendored in `crates/wowlua-vm/tests/elune/`) also pass on the VM, but count
  only where the experiments confirm them.
- **The client's behaviour.** Every WoW-specific rule (§17.3 to §17.5) has a
  probe in StdTest whose recorded in-game result is a test fixture; a rule
  changes only with a new probe.
- **In-game regressions.** Bugs found in game become emulation tests that must
  fail the same way: `os.date` in `wowlua:format`, `0 / 0` in
  `wowlua:serialize`, SuuUI's reparented status-bar containers breaking Edit
  Mode, and taint met in combat.
- **Coverage of the emulation.** A report lists which API functions have
  behaviour, which are generated only, and which rules were probed on which
  flavor.

## 17.9 VM design

- **Execution** (decision T190). The VM compiles the compiler's output to a
  register bytecode of its own, close to Lua 5.1's design. Each instruction
  carries its source span (from the source maps), and coverage, taint and
  profiling attach per instruction.
- **Memory** (decision T191). A tracing garbage collector over an arena, in
  safe Rust, stopping the world at the interpreter's allocation points:
  - Cycles are collected and weak tables clear.
  - `__gc` runs as in the client: newest first and unprotected, so an error
    in a finalizer reaches the code that caused the collection.
  - `collectgarbage` reports memory, and `step` counts work, as Lua 5.1's
    incremental collector would.
- **Taint** (decision T193). Exact, as the client tracks it: every stack
  slot, table field, upvalue and object carries the taint (the addon) of the
  code that wrote it, and each thread its execution's taint (§17.4).
- **Coroutines** (decision T192). Stackless: each coroutine is its own VM
  stack, and a yield unwinds to its `resume` without the native stack. As in
  the client's Lua 5.1, a yield cannot cross `pcall`, a metamethod or another
  native call ("attempt to yield across metamethod/C-call boundary"); each
  thread has its own global table (`setfenv(0, t)`).
