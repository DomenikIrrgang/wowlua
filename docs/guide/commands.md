# Command cheat sheet

Everything wowlua does is one binary, `wowlua`. This page lists its commands
by what you are doing. `wowlua <command> --help` shows every option of a
command.

## Start a project

```console
$ wowlua new MyAddon                       # a new addon: wowlua.json, src/, tests/, locales/
$ wowlua new MyAddon --flavors camelot     # for these flavors (default: standard)
$ wowlua new MyAddon --ci github           # plus a workflow that checks, tests, builds and publishes tags
$ wowlua new shared-utils --kind library   # a wowlua library; also lua-library and types
$ wowlua init                              # adopt the addon in this folder, with its .pkgmeta if it has one
$ wowlua init --from-pkgmeta               # the same, but require a .pkgmeta
```

More: [Getting started](getting-started.md), [Bring your addon](adopting.md).

## Work on it in the game

```console
$ wowlua dev                                   # build, install into WoW, build again on every save
$ wowlua dev --link                            # link the AddOn folder instead of copying it
$ wowlua dev --wow "/Games/World of Warcraft"  # when wowlua can't find your installation
$ wowlua dev --client _retail_                 # only these client folders
$ wowlua install --release                     # build once and install, here the release build
$ wowlua uninstall                             # remove what wowlua installed
$ wowlua toc                                   # print the TOC files a build would write
```

`dev` and `install` replace only AddOn folders wowlua installed; `--force`
replaces others too. In the game, `/reload` after each change and `/wl` for
the dev window. More: [Debugging](debugging.md).

## Check, test and tidy

```console
$ wowlua check                             # type-check everything, tests included, without building
$ wowlua check --watch                     # again on every change
$ wowlua test                              # run the tests in the emulated client
$ wowlua test "options window"             # only the files and tests whose path or name contains it
$ wowlua test --watch                      # again on every change
$ wowlua data pull                         # read your WoW installation's game data into wowlua's cache, once
$ wowlua test --game-data                  # tests see the client's items and spells where they define none
$ wowlua test -u                           # take the new snapshots and screenshots
$ wowlua test --coverage --reporter html   # which lines and branches the tests ran
$ wowlua fmt                               # format the package's sources
$ wowlua fmt --check                       # only report what would change
$ wowlua lint                              # only the lints: unused, shadowing, unreachable
$ wowlua lint --fix                        # and apply their safe fixes
$ wowlua fix                               # apply every machine-applicable fix
$ wowlua fix --migrate                     # move wowlua.json to the current schema
$ wowlua explain E2003                     # what an error code means, with examples, offline
```

More: [Testing your addon](testing.md),
[Errors and warnings](../reference/diagnostics.md).

## Debug and investigate

```console
$ wowlua trace --clipboard                                  # map an error copied from the game to your source
$ wowlua trace error.txt                                    # or from a file, or from standard input
$ wowlua screenshot tests/window.test.wowlua -o window.png  # the UI as a test file leaves it
$ wowlua test --profile-out prof                            # a speedscope profile per test file
```

More: [Debugging](debugging.md).

## For your editor and AI agents

You rarely type these yourself: the [VS Code extension](vscode.md) and your
agent start them. Other editors can use them too.

```console
$ wowlua lsp                 # language server on stdio: errors as you type, hover, completion, rename
$ wowlua debug               # debug adapter on stdio: your addon or a test in the emulated client
$ wowlua debug --port 4711   # the same on a TCP port
$ wowlua mcp                 # MCP server on stdio: an agent runs, sees and debugs your addon
```

More: [AI agents](agents.md).

## Libraries and dependencies

```console
$ wowlua add libstub --svn https://repos.wowace.com/wow/libstub/tags/1.0 --kind lua-library
$ wowlua add shared-utils --path ../shared-utils   # your own wowlua library; --dev for tests only
$ wowlua add --addon MyAddonCore --optional        # another addon, as a TOC dependency
$ wowlua remove libstub
$ wowlua outdated                  # what has newer tags, commits or revisions
$ wowlua update                    # move branch and trunk pins to their newest commit
$ wowlua fetch                     # download every dependency into the cache
$ wowlua tree                      # the dependency graph
$ wowlua tree --invert libstub     # what depends on it
$ wowlua types convert src         # LuaLS annotations → src.d.wowlua
$ wowlua types sync                # after a game patch: pin wowlua.lock to this wowlua's flavor table
$ wowlua cache clean               # drop what nothing used for 30 days; --all for everything
```

More: [Libraries and dependencies](packages.md).

## Build and ship

```console
$ wowlua build                          # dev build into dist/dev/
$ wowlua build --release --stats        # bundled release build, with its size and Lua compile time
$ wowlua build --profile beta           # a profile of your own
$ wowlua build --watch                  # again on every save
$ wowlua build --release --reproducible # build.time from SOURCE_DATE_EPOCH or the commit, not the clock
$ wowlua compile src/util.wowlua        # just the Lua for some files, no AddOn folder
$ wowlua login curseforge               # save a token once; also wago, wowinterface, github
$ wowlua logout curseforge              # remove it
$ wowlua publish --dry-run              # build, test, package and show what each site would get
$ wowlua publish                        # upload to every target
$ wowlua publish --tag v1.4.2           # tag HEAD and publish it as that tag
$ wowlua package                        # only the zips, into dist/packages/
$ wowlua package --target curseforge    # only these targets (one that doesn't exist is E4006)
$ wowlua clean                          # remove dist/
```

More: [Building and publishing](publishing.md).

## Options that work everywhere

- `-p my-addon` picks one member of a
  [workspace](packages.md#addon-suites-in-one-workspace), by its package
  name.
- `--locked` fails instead of changing `wowlua.lock`, `--offline` never uses
  the network, and `--frozen` is both. They are meant for CI.
- `--message-format github` turns errors into pull request annotations;
  `json` is one JSON object per line, for tools, and `short` one line per
  error.
- `--deny-warnings` treats warnings as errors.
- `-q` prints less, `-v` more: each `git` and `svn` command the package
  manager runs and each request publishing makes. Tokens are never printed.
- `--features a,b` turns on more of the package's
  [features](../reference/manifest.md#features), and
  `--no-default-features` turns its `default` ones off. With `wowlua add`,
  they choose the new dependency's features instead.
- `--manifest <path>` uses another `wowlua.json`, `--color never` turns colors
  off, and `-j <n>` sets the number of worker threads.

> [!TIP]
> Tell wowlua once where your game is installed and every command finds it:
> set `WOWLUA_WOW_PATH`, or `wow.path` in `~/.wowlua/config.json`
> ([Installation](install.md#where-wowlua-finds-world-of-warcraft)).
