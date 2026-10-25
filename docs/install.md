# Install

wowlua is one binary with no runtime to install. The install scripts download
a release from [GitHub Releases](https://github.com/DomenikIrrgang/wowlua/releases),
check its SHA-256 checksum and put the binary in `~/.wowlua/bin`. Releases are
built for macOS, Linux and Windows, each on x86_64 and arm64.

## macOS and Linux

Run the install script in a terminal:

```sh
curl -fsSL https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.sh | sh
```

It installs `~/.wowlua/bin/wowlua`. When that folder is not on your `PATH`
yet, the script says so. Add it to your shell profile (`~/.zshrc`,
`~/.bashrc`, …) and open a new terminal:

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

It puts `wowlua.exe` in `$HOME\.wowlua\bin`, and says so when that folder is
not on your `PATH` yet. To add it for your user, run this once and open a new
terminal:

```powershell
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$HOME\.wowlua\bin", "User")
```

## From source

Building from source needs Rust 1.96 or newer:

```sh
git clone https://github.com/DomenikIrrgang/wowlua
cd wowlua
cargo install --path crates/wowlua-cli
```

Cargo puts the binary in `~/.cargo/bin`. A build from source has everything
on `main`, including work that is not in a release yet.

## Check the install

```sh
wowlua --version
```

```text
wowlua 0.11.0
```

`wowlua help` lists every command, and `wowlua help <command>` shows its
options. The [CLI reference](../spec/toolchain/cli.md) describes them all.

## Update or pin a version

Run the installer again to update to the latest release. Two environment
variables change what it installs and where:

| Variable | Default | Effect |
|----------|---------|--------|
| `WOWLUA_VERSION` | `latest` | The release tag to install, for example `v0.11.0` |
| `WOWLUA_INSTALL_DIR` | `~/.wowlua/bin` (`$HOME\.wowlua\bin` on Windows) | Where the binary goes |

```sh
curl -fsSL https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.sh | WOWLUA_VERSION=v0.11.0 sh
```

```powershell
$env:WOWLUA_VERSION = "v0.11.0"; irm https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.ps1 | iex
```

A project names the compiler versions it works with in `package.wowlua` of its
`wowlua.json` (`wowlua new` writes `"^0.11"`). An older compiler stops with
E4300 and names both versions
([Manifest §9.16](../spec/toolchain/manifest.md#916-schema-versions-and-migration)).

## In CI

The `setup-wowlua` action installs a release on GitHub Actions runners, with
the same install scripts. Its `version` input takes a release tag (default
`latest`):

```yaml
- uses: DomenikIrrgang/wowlua/.github/actions/setup-wowlua@main
```

`wowlua new --ci github` writes a complete workflow that uses it
([CLI §12.10](../spec/toolchain/cli.md#1210-continuous-integration)).

## Remove wowlua

Delete the binary from its install folder. `~/.wowlua` (or `$WOWLUA_HOME`)
also holds the global `config.json`, the dependency cache (`cache/`) and the
archived source maps of published builds (`maps/`). Delete the folder to
remove those too.

API tokens saved with `wowlua login` live in the OS credential store. Remove
them first with `wowlua logout <site>`.

`wowlua uninstall` removes an addon's builds from your WoW folders. It does not
remove wowlua itself.

## Next steps

- [Getting started](getting-started.md): `wowlua new MyAddon`, then
  `wowlua dev`.
- [Adopting an addon](adopting-an-addon.md): `wowlua init` in your addon's
  folder.
