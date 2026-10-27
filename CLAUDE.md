# wowlua

The compiler, checker, test runner and emulated client for World of Warcraft
addons. The spec (`spec/`) is the source of truth: a change that differs from
it updates it too (the section, and a row in `spec/appendix/decisions.md`),
with tests for the new rule.

## Documentation

`docs/` is the documentation for addon authors, and wowlua.dev renders it as
it is on GitHub: `docs/guide/install.md` is `/docs/guide/install`, and the
sidebar follows the `##` sections and links of `docs/README.md`. Write it so it
reads well on GitHub too, for addon authors, in plain language; the spec stays
the normative text behind it. Links between docs are relative `.md` links;
links to anything else in the repository go to GitHub on the site.

Blocks the site draws as the design's components, and GitHub as plain Markdown:

- ` ```console ` with `$ ` command lines: a terminal; the other lines are output.
- ` ```text ` that starts with `error[` or `warning[`: compiler output.
- ` ```wowlua title=src/main.wowlua `: the file name in the code block's bar.
  `fragment` marks a block that is not a whole file, `fails=E2001` one that
  must report that code.
- An ordered list whose items are `**Title**`, a sentence and an inline
  `` `command` ``: numbered steps.
- `> [!NOTE]`, `> [!TIP]`, `> [!WARNING]`: callouts.

`node scripts/check-doc-examples.mjs` compiles every wowlua example in `docs/`
(`WOWLUA=target/release/wowlua` for a local build). Fix a failing example in
the text, unless the compiler is wrong.

## Versions

wowlua follows [semantic versioning](https://semver.org): `MAJOR.MINOR.PATCH`.
Each minor version closes a milestone of the [roadmap](spec/roadmap.md);
releases within a milestone bump PATCH. Pre-releases add `-alpha.N` or
`-beta.N`.

## Changelog

`CHANGELOG.md` has the same form as SuuUI's, so a section can be posted as it
is on the website and on Discord:

- A section per version, newest first, headed `## <version> (<YYYY-MM-DD>)`
  with the exact version. Between releases the changes collect under
  `## Unreleased`.
- Entries grouped under `### New`, `### Changed` and `### Fixed` (leave out
  empty groups), one `- ` line per change an addon author notices. A big
  feature can lead with its name in bold (`- **Test runner**: …`).
- Written for addon authors, in plain language: what they can do now or what
  works differently, not how wowlua does it inside. Name commands, warnings
  and functions as authors write them (`wowlua test`, W1004,
  `mock.defineItem`). Leave out internal refactors, probes and spec-only
  edits.

## Releasing

A tag `v<version>` on `main` releases that version: `.github/workflows/release.yml`
builds the binaries, makes the GitHub release with the version's changelog
section as its notes (no section: no release), and posts that section to
Discord (`.github/scripts/discord-release.mjs`).

Before setting a tag:

1. Find the previous release tag (`git describe --tags --abbrev=0`) and read
   every change since it (`git log <tag>..HEAD`, plus the diffs where a
   subject is unclear).
2. Choose the next version by the rules above. Set it in `Cargo.toml`
   (`workspace.package.version`; `cargo metadata` updates `Cargo.lock`), in
   `web/package.json` (the playground's package, attached to the release),
   in `editors/vscode/package.json` (the VS Code extension's version),
   in `README.md`'s roadmap line and in `docs/guide/install.md`'s and
   `docs/reference/embedding.md`'s examples.
3. Write the changelog section as above: rename `## Unreleased` and check it
   against the log.
4. Check it: `TAG=v<version> node .github/scripts/discord-release.mjs --dry-run`
   prints the Discord post. Run the whole test suite
   (`cargo test --workspace --no-fail-fast`), `cargo fmt --all -- --check`
   and `cargo clippy --workspace --all-targets -- -D warnings`, then
   `cargo xtask check` (`docs/reference/diagnostics.json` quotes the
   version: `cargo xtask diagnostics` updates it).
5. Commit (`Release <version>`), create the tag
   (`git tag -a v<version> -m "wowlua <version>"`), and push both
   (`git push origin main && git push origin v<version>`). Then watch the
   release run until the release and its binaries are up.
