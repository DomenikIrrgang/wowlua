# Installation

wowlua is a single binary: the compiler, the type checker, the test runner,
the formatter, the language server and the package manager, with no runtime
to install. This page installs it, tells it where your game is, and shows how
to update it or pin a version.

## Install wowlua

Releases are built for macOS, Linux and Windows, each on x86_64 and arm64. The
install scripts download a release from
[GitHub Releases](https://github.com/DomenikIrrgang/wowlua/releases), check
its SHA-256 checksum and put the binary in `~/.wowlua/bin`. Pick the section
for your system below, or build wowlua from source.

## macOS and Linux

Run the install script in a terminal:

```console
$ curl -fsSL https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.sh | sh
```

It installs `~/.wowlua/bin/wowlua`. If that folder is not on your `PATH` yet,
the script says so. Add it in your shell profile (`~/.zshrc`, `~/.bashrc`, …)
and open a new terminal:

```sh
export PATH="$HOME/.wowlua/bin:$PATH"
```

The Linux builds are for glibc (`x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`).

## Windows

Run the installer in PowerShell:

```powershell
irm https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.ps1 | iex
```

It puts `wowlua.exe` in `$HOME\.wowlua\bin` and says so when that folder is
not on your `PATH`. To add it for your user, run this once and open a new
terminal:

```powershell
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$HOME\.wowlua\bin", "User")
```

## From source

A build from source has everything on `main`, including changes that are not
released yet. It needs Rust 1.96 or newer:

```console
$ git clone https://github.com/DomenikIrrgang/wowlua
$ cd wowlua
$ cargo install --path crates/wowlua-cli
```

Cargo puts the binary in `~/.cargo/bin`.

## Check that it works

```console
$ wowlua --version
wowlua 0.13.5
```

`wowlua help` lists every command, and `wowlua help <command>` (or
`wowlua <command> --help`) shows the options of one. The
[command cheat sheet](commands.md) has the ones you use every day on one page.

## Add the VS Code extension

The wowlua extension for VS Code shows errors as you type, explains the WoW
API on hover, completes and renames names, lists your tests in the Testing
view and debugs your addon in the emulated client, with the game's screen
beside your code. It runs the `wowlua` binary you just installed, so the
binary must be on your `PATH`, or its location set in the `wowlua.path`
setting. [VS Code](vscode.md) explains the setup.

## Where wowlua finds World of Warcraft

`wowlua dev` and `wowlua install` copy your addon into the game. Tests,
screenshots and debug sessions draw with the game's textures and fonts when
an installation is at hand. wowlua looks for the installation in this order:

1. `--wow <path>` on the command line
2. the `WOWLUA_WOW_PATH` environment variable
3. `wow.path` in `~/.wowlua/config.json`
4. the Battle.net default folder on macOS and Windows; on Linux, the
   `drive_c/Program Files (x86)/World of Warcraft` folder of a Wine prefix:
   Lutris's `~/Games/battlenet` or `~/Games/battle-net`, then `~/.wine`

If your game is somewhere else, set it once in the global config, and every
command finds it:

```json
{
  "wow": { "path": "D:/Games/World of Warcraft" }
}
```

The config is `$WOWLUA_HOME/config.json`, and `WOWLUA_HOME` defaults to
`~/.wowlua`. Its other keys are in
[CLI §12.7](../../spec/toolchain/cli.md#127-configuration).

## Update or pin a version

Run the installer again to update to the latest release. Two environment
variables change what it installs and where:

| Variable | Default | Effect |
|----------|---------|--------|
| `WOWLUA_VERSION` | `latest` | The release tag to install, for example `v0.13.5` |
| `WOWLUA_INSTALL_DIR` | `~/.wowlua/bin` (`$HOME\.wowlua\bin` on Windows) | Where the binary goes |

```console
$ curl -fsSL https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.sh | WOWLUA_VERSION=v0.13.5 sh
```

```powershell
$env:WOWLUA_VERSION = "v0.13.5"; irm https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.ps1 | iex
```

A project names the wowlua versions it works with in `package.wowlua` of its
`wowlua.json`. `wowlua new` writes the current minor version:

```json
"package": {
  "name": "my-addon",
  "version": "0.1.0",
  "wowlua": "^0.13"
}
```

An older wowlua stops with E4300 and names both versions, so everyone who
builds the project uses a compiler that understands it
([Manifest §9.16](../../spec/toolchain/manifest.md#916-schema-versions-and-migration)).

## In CI

On GitHub Actions, the `setup-wowlua` action installs a release with the same
install scripts. Its `version` input takes a release tag and defaults to
`latest`. The workflow that `wowlua new --ci github` writes uses it like this:

```yaml
steps:
  - uses: actions/checkout@v5
    with: { fetch-depth: 0 }
  - uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main
  - run: wowlua build --release --locked --message-format github
```

[Building and publishing](publishing.md#publish-from-ci-on-every-tag) shows
the whole workflow.

## Remove wowlua

Delete the binary from its install folder. `~/.wowlua` (or `$WOWLUA_HOME`)
also holds the global `config.json`, the dependency cache (`cache/`) and the
archived source maps of builds you published (`maps/`). Delete the folder to
remove those too.

API tokens saved with `wowlua login` live in your system's credential store.
Remove them first with `wowlua logout <site>`.

> [!NOTE]
> `wowlua uninstall` is about your addons, not about wowlua: it removes an
> addon's builds from your WoW folders.

## Next steps

- [Getting started](getting-started.md): `wowlua new MyAddon`, then
  `wowlua dev`.
- [Bring your addon](adopting.md): `wowlua init` in your addon's folder.
