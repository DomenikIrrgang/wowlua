# Appendix B. Diagnostics

Every diagnostic has a stable code. `E` codes are errors, which fail the build.
`W` codes are warnings, which fail it only with `--deny-warnings`. The level of
any warning can be changed in the manifest (`check.lints`). `wowlua explain
<code>` prints an explanation with examples.

| Range | Area |
|-------|------|
| `x01xx` | Lexing and parsing |
| `x1xxx` | Names, globals, modules, conditional compilation |
| `x2xxx` | Types |
| `x3xxx` | Classes, enums, decorators |
| `x4xxx` | Manifest, packages, publishing |
| `x5xxx` | Code generation and Lua limits |
| `x6xxx` | Bundling and minification |
| `x7xxx` | Debugging and reflection |
| `x8xxx` | Standard library (`x80xx`–`x82xx` existing modules, `x83xx`–`x85xx` game data, interface and infrastructure modules) |

## B.1 Errors

| Code | Message (summary) |
|------|-------------------|
| E0101 | Unexpected token |
| E0102 | Unterminated string or long bracket |
| E0103 | Malformed number, including any misuse of `_` digit separators |
| E0104 | Ambiguous syntax (function call vs. new statement), as in Lua 5.1 |
| E0105 | Invalid escape in a backtick string |
| E0106 | `continue` or `break` outside a loop |
| E0107 | `continue` in a `repeat` loop whose `until` condition uses a body local |
| E0108 | `import` / `export` not at the top level of a module |
| E0109 | Decorator not followed by a decoratable declaration |
| E0110 | `??` mixed with `and`/`or` without parentheses |
| E0111 | Compound assignment with multiple targets |
| E0112 | `const` without an initializer, or declaring several names |
| E0113 | Optional chain used as an assignment or compound-assignment target |
| E0114 | `continue` is not the last statement of its block |
| E0115 | `class`, `enum` or `declare` not at the top level of a file |
| E0116 | Unbalanced `}` in an interpolated string |
| E0117 | `import _` combined with other names |
| E0118 | `export function` with a dotted or method name |
| E0119 | `export local` without an initializer |
| E1001 | Undefined global read (strict mode) |
| E1002 | Implicit global assignment (strict mode) |
| E1003 | Undefined local or upvalue in a `wowlua` construct (e.g. a decorator argument) |
| E1010 | Assignment to an exported or imported binding |
| E1011 | Top-level `import` after another (non-import) statement |
| E1012 | Module has no export with this name |
| E1013 | Module specifier case does not match the file on disk |
| E1014 | Import or usage cycle containing a load-time edge (run-time-only cycles are allowed) |
| E1015 | Module not found |
| E1020 | Reference to a declaration removed by `@if` or static elimination |
| E1031 | Duplicate (non-ambient) interface in the same scope |
| E1032 | Top-level `return` in a module |
| E1035 | `build.load-order` contradicts a computed load-time dependency |
| E1036 | Flavors need incompatible load orders and `toc-layout = "single"` is forced |
| E1037 | Hoisted declaration used at load time before its definition has executed |
| E1038 | `asset()` path is not a package asset (or case mismatch) |
| E1039 | `@if(flavor …)` in a `.wowlua` file in shared flavor mode |
| E1040 | `@if` condition is not a static condition |
| E2001 | Type mismatch: `S` is not assignable to `T` |
| E2002 | Unknown type name |
| E2003 | Unknown field or method (also: method defined outside its class body, assignment to the reserved `__wowlua` key) |
| E2004 | Value is not callable |
| E2005 | Value may be `nil` (strict mode) |
| E2006 | Too many arguments |
| E2007 | Missing argument |
| E2008 | Method called without `self` (use `:`) |
| E2009 | Not all code paths return a value |
| E2010 | Assignment to a `const`, `readonly` field, read-only accessor, enum member or `wowlua:meta` field |
| E2011 | Access to a `private` / `protected` member |
| E2012 | No overload matches the call |
| E2013 | Wrong number of type arguments |
| E2014 | Type argument does not satisfy its constraint |
| E2015 | Type assertion between incomparable types |
| E2030 | API not available in every flavor (shared flavor mode), missing a flavor guard |
| E2040 | Forbidden operation on a possibly secret value (strict mode) |
| E2050 | Missing parameter type annotation (strict mode) |
| E2051 | Unknown field added to a sealed table (strict mode) |
| E2052 | Non-optional class field not definitely assigned in the constructor (strict mode) |
| E2053 | Recursive type alias not guarded by a table or function type |
| E2054 | Conflicting field types in merged `declare interface` declarations |
| E2056 | Type pack used in an invalid position |
| E2058 | Name declared both as `declare event` and `declare message` |
| E2059 | Default value in a `declare event` / `declare message` parameter |
| E3001 | `super(...)` missing, repeated, or after use of `self` |
| E3002 | Cannot instantiate an abstract class |
| E3003 | Abstract method not implemented |
| E3004 | Reserved member name |
| E3005 | `override` problem: missing `override`, or nothing to override |
| E3006 | Class does not satisfy an implemented interface |
| E3007 | Metamethod on a widget class |
| E3008 | Class extends a non-class value |
| E3010 | Enum mixes numeric and string values |
| E3011 | Duplicate enum value |
| E3020 | Unknown decorator |
| E3021 | Decorator not applicable to this target (including user decorators on enums, interfaces and aliases) |
| E3022 | User decorator signature does not accept the target |
| E3032 | Invalid `@debugOnly` function (returns values, or a call is used as an expression) |
| E3033 | `@intrinsic` used outside compiler-known modules |
| E3040 | Local or parameter named `super` inside a class body |
| E3041 | `...` used in a field or static initializer |
| E3042 | Field and method/accessor with the same name (in a class or across its superclass) |
| E3043 | Accessor declared on a widget class |
| E3044 | Overload implementation does not accept every overload signature |
| E3045 | Statement-only `wowlua:debug` helper used as a value |
| E4001 | Manifest is not valid JSON |
| E4002 | Manifest does not match the schema (unknown key, wrong type) |
| E4003 | Invalid package name or version |
| E4004 | Unknown flavor |
| E4005 | Conflicting setting between `.pkgmeta` and `wowlua.json` in compatibility mode |
| E4006 | Unknown profile, minification level or publish target referenced |
| E4007 | Cyclic profile or minification-level inheritance (`inherits` / `extends`) |
| E4012 | Profile `config` key missing from the top-level `config`, or with an incompatible type |
| E4101 | Version conflict in dependency resolution |
| E4102 | Dependency not found (registry, git ref, path) |
| E4103 | Checksum mismatch |
| E4104 | Lockfile out of date (`--locked`) |
| E4105 | Network access required (`--frozen` / `--offline`) |
| E4110 | Cannot determine the load order of a `lua-library`. Specify `files`. |
| E4120 | SavedVariables name used by several addons in the workspace |
| E4201 | Missing API token for a publish target |
| E4202 | Upload rejected by the site (the site's response is included) |
| E4210 | CurseForge upload without a changelog |
| E4230 | Addons cannot be published to the wowlua registry |
| E4300 | Running compiler does not satisfy `package.wowlua` (checked before schema validation) |
| E4301 | `toc-layout: "single"` forced, but a targeted client lacks a needed TOC feature |
| E4302 | Per-flavor TOC names cannot distinguish the targeted flavors |
| E4303 | TOC line longer than 1024 characters that cannot be split (non-appendable directive or file line) |
| E4307 | `asset()` path does not name an asset of the build |
| E4308 | Output paths differ only in case |
| E4311 | Output path collides with a reserved top-level name or another output file |
| E4312 | `addon.load-on-demand: false` together with `addon-dependencies.load-with` |
| E4314 | Feature entry names an unknown feature, unknown dependency, or non-optional dependency |
| E4315 | Custom profile without `inherits` |
| E4316 | Workspace member sets its own `build.out` |
| E4317 | `addon` section in a non-addon package |
| E4318 | Targeted flavor has no types package or pinned interface |
| E4319 | VCS tag does not match `package.version` (tagged release build / `--tag`) |
| E4320 | Lockfile written by a newer compiler (unsupported lockfile version) |
| E4321 | Same package name from two different sources in one addon's graph |
| E4323 | Unsafe archive/checkout content (absolute path, `..`, escaping symlink or hardlink) |
| E4324 | Cycle of `required` addon dependencies within a workspace |
| E4325 | Two workspace members with the same `addon.name` |
| E4327 | Library does not support a targeted flavor (and the dependency is not restricted with `flavors`) |
| E4330 | `--tag` names an existing tag that points at another commit |
| E4331 | `dev`/`install` refuses to overwrite an AddOn folder not installed by wowlua (no `X-Wowlua-Build`); use `--force` |
| E5001 | Lua 5.1 limit exceeded (locals, upvalues, registers, constants, syntax levels, jump distance), including generated locals |
| E5002 | Output validation failed (missing or unreachable file, unresolvable XML reference, invalid TOC or XML) |
| E6003 | Unknown minification pass name |

## B.2 Warnings

**Pure Lua files.** In a file that uses no wowlua syntax and does not contain
`--!strict`, every name, type, flavor and secret diagnostic (`E1xxx`, `E2xxx`)
is reported as the warning with the same number (`E2001` → `W2001`, `E2030` →
`W2030`, …). Those downgraded forms are not listed separately below.

| Code | Message (summary) |
|------|-------------------|
| W1001 | Undefined global read (basic mode) |
| W1002 | Implicit global assignment (basic mode) |
| W1003 | Unused local, parameter or import |
| W1004 | Local shadows another local |
| W1005 | Unreachable code |
| W1033 | Load order cannot be determined (dynamic names, several writers, external names); files keep their source order |
| W1034 | Name read at load time but only defined at run time |
| W1035 | Usage-analysis dependency contradicts `build.load-order`; the listed order is kept (contradicting imports are E1035) |
| W2040 | Forbidden operation on a possibly secret value (basic mode) |
| W2055 | Too many arguments to an unannotated function (basic mode). Replaces the second meaning of W2006. |
| W2057 | `coroutine.yield` inside a function passed to `pcall`/`xpcall`/a C callback (cannot yield across that boundary in Lua 5.1) |
| W3021 | Lexical binding named like a core decorator is not a re-export of it (ignored in decorator position) |
| W3030 | Use of a `@deprecated` declaration |
| W3031 | `@inline` ignored |
| W4001 | Unknown TOC directive in `addon.toc-extra` |
| W4030 | Unsupported packager keyword |
| W4220 | Flavor unknown to a publish site and omitted from that upload |
| W4304 | Flavors' load orders conflict; falling back to per-flavor TOC layout |
| W4306 | Embedded library without a recognizable license file or header |
| W4309 | `--link` could not create a symlink/junction; copying instead |
| W4310 | XML reference differs in case from the referenced file (the client resolves it case-insensitively) |
| W4313 | `types sync` moved a client alias (`anniversary`, `titan`, `forever`) to another ruleset |
| W4322 | Locked version has been yanked |
| W4326 | No OS credential store; token stored in `config.json` (mode 0600) |
| W4328 | Site does not know the exact game version; fell back to a lower/highest version |
| W4329 | GitHub target skipped: build is not on a tag and no `--tag` given |
| W4332 | `.pkgmeta` `move-folders` use that cannot be converted |
| E4334 | Both a hand-written `Bindings.xml` and `wowlua:bindings` declarations |
| W4333 | `.pkgmeta` hg external cannot be converted |
| W6001 | Code inspects its own file name (`debugstack`, `debuglocals`), which changes when bundled |
| W6002 | Side-effect-free, unused module removed by dead-code elimination |
| W6010 | Publish target uses a renaming minification level (Blizzard AddOn policy: code must not be hidden or obfuscated) |
| W7001 | `todo()` in a release build |
| W7010 | Reflection metadata requested but `reflection = "none"` |
| W8011 | Slash command collides with a Blizzard command |
| W8040 | Hook target not declared |
| W8062 | Locale is missing translations (summary per locale) |
| W8101 | Event not available in every selected flavor (shared flavor mode); registered with a runtime validity check |
| W8102 | Hook target belongs to a load-on-demand addon but `opts.addon` is missing |
| W8301 | Standard-library function uses a reduced fallback on a targeted flavor (lists missing fields or features) |
| W8354 | Binding `default` key ignored on a flavor without default-key support |
| W8430 | Job yield call (`check`/`yield`/`sleep`/`waitFor`) inside `pcall`/`xpcall`/`securecall*`, a metamethod or a C-function callback |

## B.3 Standard library errors

| Code | Message (summary) |
|------|-------------------|
| E7001 | `nameof` argument is not a name, field access or string-keyed index |
| E8001 | `send` used with a WoW event (only messages can be sent) |
| E8002 | `@event` on an instance method |
| E8003 | Unknown event or message name |
| E8010 | Duplicate slash command |
| E8020 | `saved.bind` on a SavedVariable not declared in the manifest |
| E8021 | Profiles enabled without the per-character profile SavedVariable (`<name>PerChar`) |
| E8030 | Deferred-call decorator on a function that returns values |
| E8050 | Addon message prefix too long |
| E8060 | Invalid locale key |
| E8061 | Locale file defines a key unknown to the base locale |
| E8063 | Locale string uses different placeholders than the base locale |
| E8070 | Settings control bound to a field of the wrong type |
| E8080 | `launcher.register` called more than once (compile time for top-level calls, otherwise at runtime) |
| E8100 | `@unitEvent` with more than two units (`RegisterUnitEvent` limit) |
| E8101 | Invalid slash command name (must start with `/`, no whitespace) |
| E8102 | `saved.bind` name is not a string literal |
| E8103 | SavedVariable bound twice with `saved.bind` |
| E8104 | `saved.bind` defaults type contains `Secret<…>` |
| E8105 | Invalid locale file name or unknown locale code |
| E8106 | Base locale file missing |
| E8107 | Settings dropdown option value not in the field's type |
| E8108 | `wowlua:test` / `wowlua:mock` imported outside a test file |
| E8112 | Slash router: invalid `ArgSpec`, or `run` parameters don't match `args`/`flags` |
| E8113 | Settings control key not found in the store's type |
| E8114 | `settings.register` called more than once |
| E8117 | `profiles = true` on a per-character SavedVariable |
| E8118 | `wowlua:log` level function used as a value (cannot be stripped) |
| E8300 | Standard-library function unavailable on a targeted flavor (no fallback) |
| E8310 | Invalid token in a literal aura filter |
| E8330 | Unknown tooltip kind |
| E8340 | Movable frame store field is not of type `FramePosition?` |
| E8350 | `@binding` / `header` not at module top level, or with non-constant arguments |
| E8351 | Duplicate keybinding name in the addon |
| E8352 | Invalid keybinding name (must match `[A-Z][A-Z0-9_]*`) |
| E8353 | `settings.keybinding` references a binding not declared with `@binding` |
| E8390 | Invalid color hex literal |
| E8400 | Unknown media kind |
| E8420 | Unknown callback event name for the registry's event type |
