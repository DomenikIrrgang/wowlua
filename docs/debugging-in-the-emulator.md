# Debugging in the emulated client

`wowlua test` runs your addon in wowlua's emulation of the WoW client. The
same emulation can be stopped, stepped through and looked at: wowlua's
debugger, profiler and screenshots work on it, with no game running.

## Screenshots in tests

```wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("the options window")
local function options()
    mock.slash("/myaddon")
    expect(_G.MyAddonOptions).toMatchScreenshot("open")
    expect(mock.screen).toMatchScreenshot("whole screen")
end
```

The first run writes `tests/__screenshots__/options.test.wowlua/the-options-window-open.png`;
commit it. Later runs compare against it: a test fails when more than 0.1% of the
pixels differ noticeably (`{ threshold = 0.1, maxDiffRatio = 0.001 }` to
change that), and writes `….actual.png` and `….diff.png` (the differing
pixels in red) beside the stored image. `wowlua test -u` takes the new image.

The images are drawn as the client draws them: strata, levels, layers, blend
modes, texture coordinates, text. Textures and fonts come from your addon's
folder, and from your own WoW installation when wowlua finds one (`--wow`,
`WOWLUA_WOW_PATH`, the config, the default places), for the client of the
test's flavor (`camelot` reads `_classic_beta_`, `standard` `_retail_`, …).
Nothing of it is copied anywhere. Without an installation, Blizzard's
textures are drawn as magenta checkers and text in a stand-in font, so the
images of a project should be made on machines that agree on that.

`mock.screenshot("name")` writes an image of the screen (or of a frame:
`mock.screenshot("name", frame)`) to `dist/screenshots/` and returns its
path, and `wowlua screenshot` draws what a test file leaves on screen:

```sh
wowlua screenshot tests/options.test.wowlua --test "options window" --frame MyAddonOptions --scale 2 -o options.png
```

Drawing needs a GPU, or a software one (WARP on Windows, lavapipe on Linux).

## The debugger

`wowlua debug` speaks the Debug Adapter Protocol on stdio (`--port 4711` for
TCP). Editors with a DAP client (Neovim's nvim-dap, Zed, Emacs' dap-mode)
can use it now; the wowlua VS Code extension will. A launch configuration:

```json
{ "type": "wowlua", "request": "launch", "test": "tests/options.test.wowlua", "filter": "options window" }
```

or `"addon": true` to run the addon itself, logged in, its game frames
running in real time (`"clock": "manual"` keeps time still until asked),
and `"flavor": "camelot"`, `"stopOnEntry": true`, `"debugBlizzardUI": true`
(step into Blizzard's code too).

What it stops at:

- line breakpoints in `.wowlua` and `.lua` files, with conditions, hit
  counts and log messages (`count is {count}`);
- function breakpoints: your functions (`MyAddon.db:Reset`), API calls
  (`C_Timer.After`, `CreateFrame`), events (`event:PLAYER_LOGIN`,
  `event:*`), scripts (`script:OnClick`, `script:MyButton:OnClick`) and
  timers (`timer`);
- exceptions: errors that reach the error handler, every error, the moment
  your code becomes tainted, and a blocked protected action.

Stepping goes into what the engine runs because of a line too: step into
`frame:Show()` and you are in its `OnShow`. Stepping out of a handler stops
in the next handler of the same event. The call stack shows why your code
runs: `event BAG_UPDATE` → `MyFrame:OnEvent` → your function. Variables show
their taint (`string · tainted by MyAddon`), secrets what they hide, and
frames their type, size, points and children. Watches and the console
evaluate Lua in the stopped function: `count = 0` sets its local.

## For AI agents: `wowlua mcp`

`wowlua mcp` is an MCP server that lets an agent drive and debug your addon
in the emulated client. Run it in your package's folder; with Claude Code:

```sh
claude mcp add wowlua -- wowlua mcp
```

The agent can start the addon or a test file (`start_session`), look at the
UI (`screenshot`, `frame_tree`, `find_frames`), act (`click` a frame or a
point of the screenshot, `type_text`, `press_key`, `slash`, `fire_event`,
`advance` time), debug (`set_breakpoints`, `continue`, `step`, `stack`,
`variables`, `eval`) and measure (`profile_start`, `profile_stop`).

## Profiling

`profile_start` and `profile_stop` (MCP), or the DAP request
`wowlua/profile`, count per function, event, script, `OnUpdate` and game
frame: calls, Lua instructions run (the same on every machine, so you can
compare before and after a change), memory allocated, and time on your
machine (the emulator's speed, not the game's). The report lists the
costliest functions and writes a speedscope file (open it on
[speedscope.app](https://www.speedscope.app)) with the call tree.
