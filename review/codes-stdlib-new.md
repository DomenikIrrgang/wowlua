# New diagnostic codes: stdlib game, interface and infrastructure modules

Range E8300–E8599 / W8300–W8599. To be merged into `spec/appendix/diagnostics.md` §B.3.

| Code | Severity | Summary | Where used |
|------|----------|---------|------------|
| E8300 | error | Standard-library function unavailable on a targeted flavor (no fallback) | game.md (intro, aura.byInstanceId, spell.cooldownDuration, tooltip kinds), interface.md (intro) |
| W8301 | warning | Standard-library function uses a reduced fallback on a targeted flavor (lists missing fields or features) | game.md (intro, aura), interface.md (intro) |
| E8310 | error | Invalid token in a literal aura filter | game.md#aura |
| E8330 | error | Unknown tooltip kind | game.md#tooltip |
| E8340 | error | Movable frame store field is not of type `FramePosition?` | interface.md#frames |
| E8350 | error | `@binding` / `header` not at module top level, or with non-constant arguments | interface.md#bindings |
| E8351 | error | Duplicate keybinding name in the addon | interface.md#bindings |
| E8352 | error | Invalid keybinding name (must match `[A-Z][A-Z0-9_]*`) | interface.md#bindings |
| E8353 | error | `settings.keybinding` references a binding not declared with `@binding` | interface.md#bindings |
| W8354 | warning | Binding `default` key ignored on a flavor without default-key support | interface.md#bindings |
| E8390 | error | Invalid color hex literal | interface.md#color |
| E8400 | error | Unknown media kind | interface.md#media |
| E8420 | error | Unknown callback event name for the registry's event type | infrastructure.md#callbacks |
| W8430 | warning | Job yield call (`check`/`yield`/`sleep`/`waitFor`) inside `pcall`/`xpcall`/`securecall*`, a metamethod or a C-function callback | infrastructure.md#jobs |
