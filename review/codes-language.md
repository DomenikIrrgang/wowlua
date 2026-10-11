# Diagnostic codes from the language fixes (review R1)

To be merged into `spec/appendix/diagnostics.md`.

## New codes

| Code | Severity | Summary | Where used |
|------|----------|---------|------------|
| E0112 | error | `const` without an initializer, or declaring several names | 01-syntax §1.3.2 |
| E0113 | error | Optional chain used as an assignment or compound-assignment target | 01-syntax §1.3.4, §1.4.1; grammar A.4 |
| E0114 | error | `continue` is not the last statement of its block | 01-syntax §1.3.6; grammar A.1 |
| E0115 | error | `class`, `enum` or `declare` not at the top level of a file | 01-syntax §1.3.8; 03-classes §3.1.1, §3.4.1 |
| E0116 | error | Unbalanced `}` in an interpolated string | 01-syntax §1.2.4; grammar A.6 |
| E0117 | error | `import _` combined with other names | 05-modules §5.2 |
| E0118 | error | `export function` with a dotted or method name | 05-modules §5.1 |
| E0119 | error | `export local` without an initializer | 05-modules §5.1 |
| E1031 | error | Duplicate (non-ambient) interface in the same scope | 03-classes §3.3 |
| E1032 | error | Top-level `return` in a module | 05-modules §5.1 |
| W1033 | warning | Load order cannot be determined (dynamic names, several writers, external names); files keep their source order | 05-modules §5.4.5 |
| W1034 | warning | Name read at load time but only defined at run time | 05-modules §5.4.3 |
| E1035 | error | `build.load-order` contradicts a computed load-time dependency | 05-modules §5.4.5 |
| E1036 | error | Flavors need incompatible load orders and `toc-layout = "single"` is forced | 05-modules §5.4.6 |
| E1037 | error | Hoisted declaration used at load time before its definition has executed | 05-modules §5.1 |
| E1038 | error | `asset()` path is not a package asset (or case mismatch) | 06-meta §6.2.1 |
| E1039 | error | `@if(flavor …)` in a `.wowlua` file in shared flavor mode | 06-meta §6.4.3 |
| E1040 | error | `@if` condition is not a static condition | 06-meta §6.4.3 |
| E2050 | error | Missing parameter type annotation (strict mode) | 02-types §2.1 |
| E2051 | error | Unknown field added to a sealed table (strict mode) | 02-types §2.1, §2.4.2 |
| E2052 | error | Non-optional class field not definitely assigned in the constructor (strict mode) | 02-types §2.1; 03-classes §3.1.2 |
| E2053 | error | Recursive type alias not guarded by a table or function type | 02-types §2.3 |
| E2054 | error | Conflicting field types in merged `declare interface` declarations | 02-types §2.4.7 |
| W2055 | warning | Too many arguments to an unannotated function (basic mode). Replaces the second meaning of W2006. | 02-types §2.7 |
| E2056 | error | Type pack used in an invalid position | 02-types §2.2.10 |
| W2057 | warning | `coroutine.yield` inside a function passed to `pcall`/`xpcall`/a C callback (cannot yield across that boundary in Lua 5.1) | 02-types §2.4.8 |
| E2058 | error | Name declared both as `declare event` and `declare message` | 02-types §2.9 |
| E2059 | error | Default value in a `declare event` / `declare message` parameter | 02-types §2.9 |
| E3040 | error | Local or parameter named `super` inside a class body | 01-syntax §1.2.5 |
| E3041 | error | `...` used in a field or static initializer | 03-classes §3.1.2 |
| E3042 | error | Field and method/accessor with the same name (in a class or across its superclass) | 03-classes §3.1.1 |
| E3043 | error | Accessor declared on a widget class | 03-classes §3.1.10, §3.2 |
| E3044 | error | Overload implementation does not accept every overload signature | 03-classes §3.1.11 |
| E3045 | error | Statement-only `wowlua:debug` helper used as a value | 07-debugging §7.1 |

## Changed meanings

| Code | Change |
|------|--------|
| W2006 | Now **only** the downgraded form of E2006 (W2001–W2015 downgrade range). The "too many arguments to an unannotated function" warning moved to W2055. |
| W1xxx / W2xxx (all) | In pure Lua files, **every** E1xxx/E2xxx diagnostic is downgraded to the W code with the same number, including W2030 and W2040 (02-types §2.1). Appendix B should state this generally instead of only W2001–W2015. |
| E1010 | Also covers assignment to an **imported** binding (05-modules §5.1). |
| E1014 | Now specifically an import/usage cycle that contains a **load-time** edge; run-time-only cycles are allowed (05-modules §5.4.4). |
| E2003 | Also used for methods defined outside the class body (03-classes §3.1.3) and for assigning the reserved `__wowlua` namespace key (02-types §2.4.7). |
| E2010 | Also covers assignment to read-only accessors, enum members and `wowlua:meta` fields. |
| E3021 | Also covers user decorators on enums/interfaces/aliases (04-decorators §4.1). |
| W3021 | Now: a lexical binding named like a core decorator is a function that is not a re-export of that decorator (it is ignored in decorator position). Re-exports and namespace imports do not warn. |
| E0103 | Covers all digit-separator misuse (01-syntax §1.2.2). |
| E0108 vs E1011 | E0108 = `import`/`export` not at the top level; E1011 = top-level `import` after another (non-import) statement. Keep both, with these distinct meanings. |
