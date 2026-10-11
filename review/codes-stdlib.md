# Diagnostic codes: stdlib fork (existing stdlib chapters + tooling)

To be merged into `spec/appendix/diagnostics.md` §B.3 (and B.2 for warnings).

## New codes

| Code | Severity | Summary | Where used |
|------|----------|---------|------------|
| E8100 | error | `@unitEvent` with more than two units (`RegisterUnitEvent` limit) | stdlib/core.md §events |
| E8101 | error | Invalid slash command name (must start with `/`, no whitespace) | stdlib/core.md §slash |
| E8102 | error | `saved.bind` name is not a string literal | stdlib/core.md §saved |
| E8103 | error | SavedVariable bound twice with `saved.bind` | stdlib/core.md §saved |
| E8104 | error | `saved.bind` defaults type contains `Secret<…>` | stdlib/core.md §saved |
| E8105 | error | Invalid locale file name or unknown locale code | stdlib/ui.md §locale |
| E8106 | error | Base locale file missing | stdlib/ui.md §locale |
| E8107 | error | Settings dropdown option value not in the field's type | stdlib/ui.md §settings |
| E8108 | error | `wowlua:test` / `wowlua:mock` imported outside a test file | stdlib/README.md, toolchain/tooling.md §16.1.1 |
| E8109 | error | Standard-library API not available on this flavor | stdlib/README.md §8.1 |
| E8112 | error | Slash router: invalid `ArgSpec`, or `run` parameters don't match `args`/`flags` | stdlib/core.md §slash |
| E8113 | error | Settings control key not found in the store's type | stdlib/ui.md §settings |
| E8114 | error | `settings.register` called more than once | stdlib/ui.md §settings |
| E8117 | error | `profiles = true` on a per-character SavedVariable | stdlib/core.md §saved |
| E8118 | error | `wowlua:log` level function used as a value (cannot be stripped) | stdlib/advanced.md §log |
| E8120 | error | `settings.keybinding` names a binding not declared with `wowlua:bindings` | stdlib/ui.md §settings |
| E8122 | error | Name declared both as `event` and as `message` | stdlib/core.md §events |
| W8101 | warning | Event not available in every selected flavor (shared flavor mode); registered with a runtime validity check | stdlib/core.md §events |
| W8102 | warning | Hook target belongs to a load-on-demand addon but `opts.addon` is missing | stdlib/advanced.md §hooks |

## Changed meanings of existing codes

| Code | Old summary | New summary |
|------|-------------|-------------|
| E8001 | `send` used with a WoW event | unchanged (now based on `declare event` vs `declare message`) |
| E8003 | Unknown event name | Unknown event or message name |
| E8010 | Duplicate slash command (also used for "must start with `/`") | Duplicate slash command only (invalid names → E8101) |
| E8021 | Profiles enabled without the per-character profile SavedVariable | unchanged; the expected name is now defined as `<name>PerChar` |
| E8080 | `launcher.register` called more than once | unchanged; compile-time for top-level calls, runtime error otherwise |
