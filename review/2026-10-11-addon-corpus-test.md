# Corpus test: ten large addons (2026-10-11)

Second real-world test of milestone M0, after
[ElvUI](2026-10-11-elvui-corpus-test.md). Ten packaged releases with 44
addon folders between them were adopted with `wowlua init` and built with
`wowlua build`: WeakAuras, DBM, BigWigs, Plater, Bartender4, Questie (with
QuestieDB), OmniCC, Cell, Grid2 and AtlasLoot.

## Method

The same per-flavor equivalence check as for ElvUI: for every flavor, the
files the client loads (in order) and all metadata directives from the
original TOCs were compared with those from the generated TOC(s), and every
output file was compared byte for byte with the original. Defaults the client
applies anyway (for example a missing `DefaultState`) were normalized, and
`## X ##` comment lines were ignored.

## Result

43 of 44 addons are equivalent for every flavor. The remaining difference
(Questie, below) is intended. Adopting and building the whole corpus takes
about 28 s with a release build of wowlua.

## Findings

| # | Addon | Finding | Resolution |
|---|-------|---------|------------|
| 1 | QuestieDB | `init` hung: about 100,000 `X-` directives per flavor, deduplicated with `Vec::contains` (quadratic) | Index sets |
| 2 | QuestieDB | The TOC is a database. Copied into the manifest it made a 163 MB `wowlua.json` | `addon.toc-data` files (decision T54); the manifest is now 16 KB and the data is reproduced exactly |
| 3 | BigWigs, DBM | File lines with variables (`Locales\[TextLocale].lua`, `[Game]`) were treated as missing files | `build.variable-files` (decision T55); verbatim when every client supports variables, expanded otherwise; every expansion is checked |
| 4 | BigWigs, DBM | They ship single TOCs to Cataclysm Classic, which the flavor table said could not read them | Cata 4.4.2 and the closed Wrath Classic 3.4.5 client get full TOC features (decision T56) |
| 5 | Bartender4 | Inline directive before the value: `## OptionalDeps: [AllowLoadGameType vanilla] Ace3` | Parser accepts leading directives |
| 6 | Plater | `IconTexture-blank` rejected as a generated localized directive (E4002) | Only `Title-`, `Notes-` and `Category-` have localized forms |
| 7 | DBM-VPVEM, WeakAurasArchive | Addons that load no code rejected ("no sources") | Code-less addons are valid |
| 8 | several | Per-flavor file orders merged naively, which reordered files in some flavors | Topological merge; conflicts are warned about |
| 9 | Questie | Structural fields (version, flags, saved variables) taken from the first TOC, which for Questie is an `Interface: 00000` stub | Majority by flavor for version, flags and publish IDs; union for saved variables (decision T57) |

## Remaining difference

Questie's `Questie.toc` is a stub that retail reads: it has no files, just
an error message. Structural fields cannot vary per flavor (decision T50),
so the generated retail TOC carries the same `LoadSavedVariablesFirst`,
`X-Curse-Project-ID`, `X-Wago-ID` and the union of saved variables as the
Classic TOCs. The client loads the same (empty) file list for retail, so the
difference has no effect in game.

## Not covered

As with ElvUI, the check is static: it compares what the client would load,
not runtime behavior. In-game smoke tests of the built addons remain manual.
