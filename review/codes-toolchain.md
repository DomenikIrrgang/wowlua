# New and changed diagnostic codes — toolchain fork

To be merged into `spec/appendix/diagnostics.md`.

## New codes

| Code | Severity | Summary | Where used |
|------|----------|---------|------------|
| E4300 | error | Running compiler does not satisfy `package.wowlua` (checked before schema validation) | manifest.md §9.2, §9.16; compiler.md §13.3.1 |
| E4301 | error | `toc-layout: "single"` forced, but a targeted client lacks a needed TOC feature | codegen.md §14.9.1 |
| E4302 | error | Per-flavor TOC names cannot distinguish the targeted flavors | codegen.md §14.9.3 |
| E4303 | error | TOC line longer than 1024 characters that cannot be split (non-appendable directive or file line) | codegen.md §14.9.4; compiler.md §13.8 |
| W4304 | warning | Flavors' load orders conflict; falling back to per-flavor TOC layout | codegen.md §14.9.2 |
| W4306 | warning | Embedded library without a recognizable license file or header | compiler.md §13.8 |
| E4307 | error | `asset()` path does not name an asset of the build | codegen.md §14.8.7; compiler.md §13.8 |
| E4308 | error | Output paths differ only in case | compiler.md §13.8, §13.12 |
| W4309 | warning | `--link` could not create a symlink/junction; copying instead | compiler.md §13.12 |
| W4310 | warning | Load order undecidable (dynamic names, several writers, `setfenv`); files keep source order | compiler.md §13.6.1 |
| E4311 | error | Output path collides with a reserved top-level name or another output file | codegen.md §14.8; compiler.md §13.8 |
| E4312 | error | `addon.load-on-demand: false` together with `addon-dependencies.load-with` | manifest.md §9.3 |
| W4313 | warning | `types sync` moved a client alias (`anniversary`, `titan`, `forever`) to another ruleset | manifest.md §9.4.1 |
| E4314 | error | Feature entry names an unknown feature, unknown dependency, or non-optional dependency | manifest.md §9.6 |
| E4315 | error | Custom profile without `inherits` | manifest.md §9.9 |
| E4316 | error | Workspace member sets its own `build.out` | manifest.md §9.15 |
| E4317 | error | `addon` section in a non-addon package | manifest.md §9.2 |
| E4318 | error | Targeted flavor has no types package or pinned interface | manifest.md §9.4.1 |
| E4319 | error | VCS tag does not match `package.version` (tagged release build / `--tag`) | manifest.md §9.9.2; publishing.md §11.9 |
| E4320 | error | Lockfile written by a newer compiler (unsupported lockfile version) | manifest.md §9.16 |
| E4321 | error | Same package name from two different sources in one addon's graph | package-manager.md §10.4 |
| W4322 | warning | Locked version has been yanked | package-manager.md §10.4 |
| E4323 | error | Unsafe archive/checkout content (absolute path, `..`, escaping symlink or hardlink) | package-manager.md §10.11 |
| E4324 | error | Cycle of `required` addon dependencies within a workspace | package-manager.md §10.8 |
| E4325 | error | Two workspace members with the same `addon.name` | package-manager.md §10.8 |
| W4326 | warning | No OS credential store; token stored in `config.json` (mode 0600) | package-manager.md §10.11 |
| E4327 | error | Library does not support a targeted flavor (and the dependency is not restricted with `flavors`) | package-manager.md §10.5.1 |
| W4328 | warning | Site does not know the exact game version; fell back to a lower/highest version | publishing.md §11.3 |
| W4329 | warning | GitHub target skipped: build is not on a tag and no `--tag` given | publishing.md §11.7 |
| E4330 | error | `--tag` names an existing tag that points at another commit | publishing.md §11.9 |
| E4331 | error | `dev`/`install` refuses to overwrite an AddOn folder not installed by wowlua (no `X-Wowlua-Build`); use `--force` | cli.md §12.5 |
| W4332 | warning | `.pkgmeta` `move-folders` use that cannot be converted | cli.md §12.6 |
| W4333 | warning | `.pkgmeta` svn/hg external cannot be converted | cli.md §12.6 |
| W6002 | warning | Side-effect-free, unused module removed by dead-code elimination | optimization.md §15.2.1 |
| E6003 | error | Unknown minification pass name | optimization.md §15.4.3; manifest.md §9.10 |

## Changed meaning / usage of existing codes

| Code | Change |
|------|--------|
| E4003 | No longer used for an unmet `package.wowlua` (now E4300). Keeps "Invalid package name or version". |
| E4007 | Also covers `minify-levels` `extends` cycles (appendix text already says "profile or minification-level inheritance"). |
| E4012 | Now: profile `config` key missing from the top-level `config`, or with an incompatible type. |
| W4220 | Generalized to all sites: flavor unknown to a site, omitted from that site's upload. |
| E5001 | Message also names generated locals when the prologue/hoisting pushes a chunk over the limit. |
| E5002 | Also covers unresolvable XML `<Script>`/`<Include>` references after rewriting, and unreachable output files. |
| E1014 | Used by compiler.md §13.6.1 for **load-time** import cycles only (run-time-only cycles are allowed). Language fork owns the code text. |
