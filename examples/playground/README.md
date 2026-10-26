# Playground examples

The programs in the example picker of the playground on
[wowlua.dev](https://wowlua.dev/playground). The site reads this folder from
`main`, so a change here shows up there without a site deploy.

- One `.wowlua` file per example, in the picker's order by the number in
  front of its name (`1-syntax.wowlua`, `2-types.wowlua`, …).
- Line 1 is the picker's label: `-- Syntax and sugar`.
- Line 2, when it is a `-- ` comment too, is the hint under the problems tab:
  what to change to see a diagnostic. `` `code` `` and `**bold**` work there.
  Line numbers in it count from the line after it, which is line 1 in the
  playground's editor (the two header lines are not shown there).
- Each example is the whole `src/main.wowlua` of a package for camelot, so it
  must pass `wowlua check` and print something when it runs: the playground's
  output tab shows what it printed.
