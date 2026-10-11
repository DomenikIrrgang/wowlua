# Corpus test 2: eighteen more addons (2026-10-11)

Third real-world test of milestone M0, after
[ElvUI](2026-10-11-elvui-corpus-test.md) and the
[first corpus](2026-10-11-addon-corpus-test.md). Twenty release packages of
eighteen addons, with 66 addon folders between them, were adopted and built:
AllTheThings, Hekili, MythicDungeonTools, TellMeWhen, Masque,
Simulationcraft, ShadowedUnitFrames (Classic), RealUI (13 addons),
AdvancedInterfaceOptions (retail, Classic and Cata packages), oUF,
Total RP 3, LittleWigs (12 addons), ls_UI, BetterBags, Rarity, SpartanUI,
HandyNotes and DBM-Dungeons (18 addons). The mix covers large data addons,
standalone libraries, UI suites, Classic-only addons and load-on-demand
companions.

## Method

The same per-flavor equivalence check as before. All addons of a package
are now adopted side by side, because some load files from their siblings.
Three client behaviors the documentation does not cover were checked in
game with a probe addon on the WoW: Forever beta client (1.60.1).

## Result

63 of 66 addons are equivalent for every flavor. The three
AdvancedInterfaceOptions builds differ only in an intended path rewrite
inside a comment (decision T42). As a regression check, the first corpus and
ElvUI were downloaded again and rerun: 46 of 47 equivalent, with only the
known Questie stub-TOC difference. Adopting and building this corpus takes
about 10 s with a release build.

## In-game probe

| Question | Answer |
|----------|--------|
| Does `## LoadOnDemand: true` work like `1`? | Yes |
| Does `[AllowLoadGameType: mainline]` (with a colon) filter like the plain form? | Yes, a `: vanilla` line was not loaded |
| Do inline directives work in XML `<Script file>` and `<Include file>`? | Yes, with and without colon; a `vanilla` reference was not loaded |
| Is `camelot` recognized as a game type? | Yes |

## Findings

| # | Addon | Finding | Resolution |
|---|-------|---------|------------|
| 1 | LittleWigs_Classic | `LittleWigs_Classic.toc` taken as the `_Classic` TOC of an addon named `LittleWigs`, so the output had the wrong name and lost the standard flavor | The folder name decides: a TOC named like its folder is the plain TOC |
| 2 | LittleWigs_Classic | Addon-wide `## AllowLoadGameType: classic, standard` copied into `toc-extra` | It narrows the flavors |
| 3 | ls_UI_Options, ShadowedUF_Options | `## LoadOnDemand: true` not recognized | `1` and `true` both count (verified in game) |
| 4 | Total RP 3 | `[AllowLoadGameType: tbc]` with a colon, inside XML `<Include file>` attributes | Inline directives parsed anywhere, with or without colon, also in XML (decision T62) |
| 5 | TellMeWhen_Options, MythicDungeonTools_UI | TOC lines load files from the main addon's folder (`..\TellMeWhen\Options\…`) | `build.external-files` and `build.shared-files`; the owner moves and ships the files (decision T58) |
| 6 | !RealUI_Preloads | Addon names with `!` rejected | Any valid folder name (decision T59) |
| 7 | AllTheThings | The addon has its own `src/`; files were nested in `src/src/`, and unloaded leftovers were built | Sources go to `wowlua-src/` (decision T60) |
| 8 | Simulationcraft | TOC spells one folder `libs\` and `Libs\`; on macOS the recorded load order kept the TOC spelling and did not match | Paths always use the on-disk spelling |
| 9 | SpartanUI, Total RP 3 | XML references differing in case from the file (`ouf.lua` for `oUF.lua`) | Resolved like the client, warning W4310 (decision T61) |
| 10 | BetterBags, RealUI | Dependencies reordered: all unrestricted entries came first | Manifest order kept, consecutive entries with the same flavors share a line (decision T63) |

## Not covered

The check is static, as before. The built addons were not started in game,
apart from the probe.
