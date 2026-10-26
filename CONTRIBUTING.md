# Contributing to wowlua

Thanks for helping. Bug reports, fixes, documentation and new features are all
welcome. Questions are best asked on the
[Discord server](https://discord.gg/Dn795czPYk). Everyone taking part follows
the [code of conduct](CODE_OF_CONDUCT.md).

## Reporting a bug

Open an [issue](https://github.com/DomenikIrrgang/wowlua/issues/new) with:

- the wowlua version (`wowlua --version`) and your operating system;
- the command you ran and its full output;
- the smallest code or `wowlua.json` that shows the problem.

For an error you saw in game, map it back to your source first:
`wowlua trace --clipboard` reads an error copied from the game's error frame,
and `wowlua trace error.txt` one saved to a file.

## Before a larger change

Open an issue first, so the behaviour can be agreed on before you write the
code. The [specification](spec/README.md) is the source of truth: a change that
differs from it updates it too, the section and a row in
[the decision log](spec/appendix/decisions.md).

## Building and testing

wowlua is written in Rust, version 1.96 or newer.

```sh
cargo test --workspace
cargo xtask check    # generated files are up to date (CI runs it)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Every change comes with tests for what it changes. The README's
[Developing](README.md#developing) section lists the other tasks: regenerating
the diagnostic codes, the manifest schema and the WoW API declarations, the
conformance corpus and the in-game tests.

## Documentation

The guides in [`docs/`](docs/README.md) are the documentation on
[wowlua.dev](https://wowlua.dev/docs); the site renders them from this
repository. Write for addon authors, in plain language, and check the
examples compile: `node scripts/check-doc-examples.mjs`. [CLAUDE.md](CLAUDE.md)
lists the Markdown the site draws as terminals, compiler output and steps.

Diagnostic codes are explained in
[`crates/wowlua-diagnostics/explanations/`](crates/wowlua-diagnostics/explanations/README.md);
`wowlua explain` and the website show those explanations.

## Changelog

Add a line for every change an addon author notices to `## Unreleased` in
[CHANGELOG.md](CHANGELOG.md), under `### New`, `### Changed` or `### Fixed`,
in plain language: what they can do now, not how wowlua does it inside.

## Licence

By contributing you agree that your contribution is licensed under the
[MIT licence](LICENSE).
