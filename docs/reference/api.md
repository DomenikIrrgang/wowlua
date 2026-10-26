# WoW API

The game's API as wowlua knows it: every function, event and widget method of
the client, with what it does, its parameters and results, and how far
wowlua's emulated client implements it for your tests and debug sessions.

The reference covers the camelot flavor, the client of WoW Forever, today.
`standard` (retail) and `vanilla` (Classic Era) are coming.

## How much of the API wowlua implements

`wowlua test` and `wowlua debug` run your addon in an emulated camelot client.
The functions answer there, but the answers don't all come from the same
place, so each function and widget method has a status that says how far your
tests can rely on it:

| Status | What it means for your tests |
|--------|------------------------------|
| implemented | wowlua models the system behind it: the units in the world, auras, bags, money, quests, the clock and so on. Tests set that state, with `mock.spawnUnit` or `mock.setBag` for example, and the function and its events follow, as in the game. |
| recorded | The answer was recorded from the real client and is played back for the same call. Exact for the calls that were recorded; a call that never was gets no answer. |
| guessed | The answer is worked out from Blizzard's documentation, its UI code and the function's name, and not checked against the client. Good enough for code that only needs a sensible value; check it in the game if your addon depends on the result. |
| not implemented | Only the stand-in generated from the documentation. Your addon's call fails with "`<name>` is not mocked", so you notice at once. |

For an event, not implemented means the emulated client never fires it by
itself. Fire it from a test with `mock.fireEvent`, or `mock.fireUnitEvent` for
a unit event. The event must be one the flavor documents, and its payload is
checked against the documentation. See
[Testing your addon](../guide/testing.md).

How the emulator's answers compare with the real client, and what is still
missing, is in [Emulator coverage](../emulator-coverage.md).

## Namespaces

The functions are grouped as the game groups them: the global functions, and
namespaces such as `C_Container`, `C_Spell` or `C_Timer`. A namespace lists
the functions Blizzard's documentation describes and also those the client
has that the documentation leaves out.

## Where the descriptions come from

- **Signatures, structures and events** come from Blizzard's API
  documentation for the client, typed for wowlua.
- **Descriptions, parameter notes and links** come from
  [warcraft.wiki.gg](https://warcraft.wiki.gg), by the wiki's contributors,
  under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). The
  wiki's pages also give the signatures of functions Blizzard's
  documentation lacks.
- **Descriptions by wowlua**: where neither says what a function does, wowlua
  wrote a description from the function's name, its signature and how the
  emulator answers it. These are marked "Description by wowlua".

The declarations also use information from Ketho's
[vscode-wow-api](https://github.com/Ketho/vscode-wow-api), under the MIT
license.

> [!TIP]
> The same descriptions show when you hover or complete a function in
> VS Code. See [VS Code](../guide/vscode.md).
