# wowlua for VS Code

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

## The game's screen

While a session runs, the **wowlua: UI** panel beside the editor shows the
game's screen as the client draws it (with your installation's textures
and fonts). It redraws at every stop and step, and once a second while the
game runs (`wowlua.ui.refreshInterval`, `0` for stops only). Click in it to
click in the game (right-click for the right button): the frame under the
pointer gets the click, so its breakpoints stop. *wowlua: Show UI* opens it;
`wowlua.ui.autoOpen` opens it as a session starts.

The **wowlua Frames** view in the debug sidebar lists the frames on
screen with their type, size and text; select one to outline it in the
panel.

## Trying it from the repository

```sh
ln -s "$(pwd)/editors/vscode" ~/.vscode/extensions/wowlua
```

then reload VS Code. To package it: `npx @vscode/vsce package` in this
folder.

Its tests drive a VS Code of their own on `examples/widget-demo` (they
need a WoW installation and `cargo build --release -p wowlua-cli`):
`editors/vscode/test/run.sh`.
