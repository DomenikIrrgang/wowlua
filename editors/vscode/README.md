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

or `"addon": true` to run the addon itself, `"flavor": "camelot"`,
`"clock": "manual"`, `"stopOnEntry": true`, `"debugBlizzardUI": true`.
See [Debugging in the emulator](../../docs/debugging-in-the-emulator.md).

## Trying it from the repository

```sh
ln -s "$(pwd)/editors/vscode" ~/.vscode/extensions/wowlua
```

then reload VS Code. To package it: `npx @vscode/vsce package` in this
folder.
