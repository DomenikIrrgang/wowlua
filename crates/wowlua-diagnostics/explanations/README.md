# Diagnostic explanations

What each diagnostic code means and how to fix it. `wowlua explain <code>`
prints a code's explanation, and wowlua.dev shows it on the code's page (from
`docs/reference/diagnostics.json`, written by `cargo xtask diagnostics`). The
codes and their one-line summaries come from
[Appendix B](../../../spec/appendix/diagnostics.md); this folder only explains
them.

One file per area, headed `# <range> · <short name>`. Each code is a `## CODE`
section:

````markdown
## E2001

What the code means, when it is reported and how to fix it, in plain words
for addon authors. Markdown; one to three short paragraphs.

```wowlua fails
local count: number = "3"
```

```wowlua compiles
local count: number = 3
```

Related: E2002, E2005
````

- A `fails` block must report the code, a `compiles` block must report no
  error and not the code. `cargo test -p wowlua-web --test explanations` checks
  every example in the playground's in-memory package (camelot).
- A block without `title=` is `src/main.wowlua` (`tests/main.test.wowlua` when
  it imports `wowlua:test` or `wowlua:mock`). Blocks with `title=path` next to
  each other, of the same verdict, are the files of one example:
  `title=wowlua.json` replaces the manifest, `title=src/other.wowlua` adds a
  file.
- Leave the examples out for a code that `wowlua check` can't report: one found
  by `build`, `publish`, `test` or in game (the `P` hints).
- `Related:` lists codes worth reading next: the warning or error twin, codes
  that are often confused with this one.
