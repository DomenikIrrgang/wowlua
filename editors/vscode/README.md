# wowlua for VS Code

Check, format and outline wowlua addons as you type. The extension starts
`wowlua lsp`: errors and warnings show in the Problems view a moment after
you stop typing (and at once when you save), for the edits you haven't saved
too; *Format Document* formats as `wowlua fmt` does with your package's
`fmt` settings; the outline and breadcrumbs list a file's functions, classes
with their methods, locals, enums and types. `wowlua.languageServer` turns
it off; *wowlua: Restart Language Server* restarts it (its log is in the
**wowlua Language Server** output).

Debug wowlua addons and their tests in wowlua's emulated World of Warcraft
client: breakpoints in `.wowlua` and `.lua` files, stepping, the call stack
with the events and scripts that ran your code, variables with their taint,
watches. The extension starts `wowlua debug` (it needs wowlua 0.10 or newer
on your `PATH`, or its path in the `wowlua.path` setting).

A launch configuration (`.vscode/launch.json`):

```json
{
  "type": "wowlua",
  "request": "launch",
  "name": "wowlua: debug a test",
  "test": "tests/options.test.wowlua",
  "filter": "options window"
}
```

A session runs one flavor: `"flavor": "camelot"` chooses it; without one,
the extension takes the package's only flavor or asks which of its flavors
to debug, and the Debug Console names the flavor that runs.

Also `"addon": true` to run the addon itself,
`"clock": "manual"`, `"stopOnEntry": true`, `"debugBlizzardUI": true`.
See [Debugging in the emulator](../../docs/debugging-in-the-emulator.md).

## Debugging the addon itself

`"addon": true` runs the addon in the emulated client, logged in, without a
test: set breakpoints, then use it. In the Debug Console a line starting
with `/` is a slash command, as typed in chat (`/wdemo`); any other line is
Lua, run in the game (or in the stopped frame while paused). Clicks in the
UI panel (below) click in the game.

```json
{ "type": "wowlua", "request": "launch", "name": "wowlua: run the addon", "addon": true, "flavor": "camelot", "profile": "camelot" }
```

`"profile"` starts the session in a world profile (`tests/profiles/<name>.json`:
the character with its name, gear and bags, the world around it) instead of
the default character; the widget demo has `camelot`, a recorded WoW Forever
character, "Player Lastname", with eight other players online to `/invite`
(`/invite Gorak Stonefist`), enough for a party and, after
`C_PartyInfo.ConvertToRaid()`, a raid.

## The game's screen

While a session runs, the **wowlua: UI** panel beside the editor shows the
game's screen as the client draws it (with your installation's textures
and fonts). It redraws at every stop and step, and once a second while the
game runs (`wowlua.ui.refreshInterval`, `0` for stops only). Click in it to
click in the game (right-click for the right button): the frame under the
pointer gets the click, so its breakpoints stop. Zoom with Ctrl or Cmd and
the wheel (or a pinch), or the buttons; pan with the scrollbars, the wheel,
or by dragging with the middle button or Space held. *Zoom to frame…*
lists the frames on screen (type to filter, arrows and Enter to pick) and
zooms to the one you pick; zoomed in, the screen is drawn sharper. *wowlua: Show UI* opens it;
`wowlua.ui.autoOpen` opens it as a session starts.

The **wowlua Frames** view in the debug sidebar lists the frames on
screen with their type, size and text; select one to outline it in the
panel, or its zoom button to zoom to it.

## Trying it from the repository

```sh
ln -s "$(pwd)/editors/vscode" ~/.vscode/extensions/wowlua
```

then reload VS Code. To package it: `npx @vscode/vsce package` in this
folder.

Its tests drive a VS Code of their own on `examples/widget-demo` (they
need a WoW installation and `cargo build --release -p wowlua-cli`):
`editors/vscode/test/run.sh` (`WOWLUA_SCENARIO=lsp` for the language
client alone). `node --test editors/vscode/test/connection.test.js` checks
the client's connection to `wowlua lsp` without VS Code.
