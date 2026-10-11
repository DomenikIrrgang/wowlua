# Corpus test: ElvUI 15.26 (2026-10-11)

First real-world test of milestone M0: the packaged ElvUI release (ElvUI,
ElvUI_Libraries, ElvUI_Options; 730 Lua files, about 219,000 lines) was
adopted with `wowlua init` and built with `wowlua build`.

## Method

An equivalence check compared, for every flavor, what the client loads from
the original per-flavor TOCs with what it loads from the generated TOC(s):
the file list in order, and all metadata directives (title, notes, icon,
group, categories, dependencies, saved variables, `X-` fields). Every output
file was compared byte for byte with the original.

## Result

After the fixes below: identical file lists and metadata for all 5 flavors
of all 3 addons, and 984 output files byte-identical, including
`Bindings.xml` / `Bindings_Mainline.xml`. A full build of ElvUI takes under
0.4 s (debug build).

## Findings

| # | Finding | Resolution |
|---|---------|------------|
| 1 | UTF-8 BOM at the start of a Lua file rejected; sources decoded lossily | Raw bytes kept and passed through; the syntax check skips a BOM like WoW |
| 2 | TOC version `v15.26` not accepted | `init` normalizes to `15.26.0` |
| 3 | Adjacent inline directives `[A x][B y]` parsed as a file variable | Parser fixed |
| 4 | Per-flavor dependencies (`LibHealComm-4.0` Classic only) lost | `init` writes `{ name, flavors }` entries |
| 5 | Per-flavor metadata (titles, icon, group, compartment) not expressible | `addon.flavor-overrides` (decision T50) |
| 6 | `Bindings*.xml` dropped (loaded by the client, not the TOC) | Passed through from the package root (T52) |
| 7 | Plain and suffixed TOCs combined instead of using the one each client reads | `init` picks the client's TOC per flavor (T53) |
| 8 | Files no TOC loads were left behind silently | `init` reports them |
| 9 | The git checkout fetches Ace3, LibStub, … from CurseForge SVN via `.pkgmeta` | SVN sources in the package manager (T51, milestone M6) |
| 10 | CI used the deprecated `actions/checkout@v4` | Bumped to v5 |
