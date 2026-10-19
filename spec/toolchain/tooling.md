# 16. Developer Tooling

## 16.1 Test runner (`wowlua test`)

Tests run outside the game, in wowlua's **emulation of the WoW client**
([WoW client emulation](emulator.md)): a Lua 5.1 VM written in Rust with WoW's
Lua differences, taint and secret values, and an engine with the flavor's WoW
API, optionally with Blizzard's own UI code. Like the game it has no `io`,
`os`, `require` or file access. The framework ships with wowlua; tests need no
third-party library (decision T184).

### 16.1.1 Writing tests

| Files | Role |
|-------|------|
| `tests/**/*.test.wowlua`, `src/**/*.test.wowlua` | Test files |
| `tests/mocks/**/*.wowlua` | Project mocks. Loaded before the addon, never treated as tests. |
| other `tests/**/*.wowlua` | Test helpers, importable from test files |

Test-only files are compiled with the `test` profile and never included in
addon builds. Importing `wowlua:test` or `wowlua:mock` from a non-test file is
error E8108. In test files, `/x` names `build.src`'s `x` (the addon's
modules), and `./x` a file next to the test file (helpers under `tests/`).

```wowlua
-- tests/tracker.test.wowlua
import test, beforeEach, expect from "wowlua:test"
import * as mock from "wowlua:mock"
import Tracker from "/tracker"

local tracker: Tracker

@beforeEach
local function setup()
    mock.resetSpies()
    tracker = Tracker.new()
end

@test("records kills")
local function recordsKills()
    tracker:record(2)
    expect(tracker:total()).toBe(2)
end

@test("flushes on logout")
local function flushOnLogout()
    tracker:record()                       -- saved data is loaded: lifecycle already ran
    mock.fireEvent("PLAYER_LOGOUT")
    expect(MyAddonDB.kills).toBe(1)
end

@test("ticks every second")
local function ticks()
    local spy = mock.spy()
    tracker:startTicker(spy)
    mock.advanceTime(3.5)
    expect(spy).toHaveBeenCalledTimes(3)
end
```

### 16.1.2 What a test run loads

For each test file and flavor, the runner:

1. creates a fresh VM and installs `wowlua:mock` for that flavor,
2. loads the TOC's files up to the runtime (locale files, the runtime), then
   the project mocks (`tests/mocks/`, with what they import), then the
   manifest's `test.addons` (below),
3. loads the rest of the **whole addon** in its computed load order,
   compiled with the `test` profile (exactly as the generated TOC would load
   it, XML included, as the addon's code),
4. loads the test file after the helpers it imports (its top-level code
   runs),
5. unless the file opted out, fires the lifecycle: `ADDON_LOADED` for each
   of the `test.addons` loaded and then for the addon, `PLAYER_LOGIN`, and
   `PLAYER_ENTERING_WORLD` (`isInitialLogin = true`),
6. runs `@beforeAll`, then each test with its `@beforeEach` / `@afterEach`
   hooks, then `@afterAll`.

A test file opts out of step 5 with `config({ lifecycle = "manual" })` from
`wowlua:test` at its top level, and then drives the lifecycle itself with
`mock.login()`. Tests within one file share the VM. Each file starts fresh.
An error while loading (a step above) fails the file, Blizzard's code's
included (with `test.blizzard-ui`: it runs without an error in the client,
so the engine must too). An error that reaches
the error handler during a test (an event handler's, a timer's) fails that
test, unless the test read it with `mock.errors()`. An error the client
drops without a report (a `secureexecuterange` callback's: no error handler
hears it, the next entries still run) fails the test or the file too, though
`mock.errors()` does not list it: the code failed. The addon's
`Bindings.xml` is read before its files load, as the client does.

`test.addons` (manifest §9.18) names other addons' folders, a test's
neighbours as the client would load them: each folder is an addon named
after it, read from the TOC the client picks for the flavor
(`<Name>_Mainline.toc` and the flavor's other suffixes, else `<Name>.toc`),
its files loaded as its own code with its own addon table (`local name, ns =
...`), its `Bindings.xml` read. They load in the listed order, each after
those of its `## Dependencies` / `## RequiredDeps` the list has (and after
the Blizzard addons it depends on, with `test.blizzard-ui`). An addon whose
`AllowLoadGameType` / `ExcludeLoadGameType` / `AllowLoad` leaves out the
flavor's game is skipped. A `## LoadOnDemand: 1` addon is known but not
loaded: `C_AddOns.LoadAddOn(name)` loads it (its dependencies, its files,
then its `ADDON_LOADED`; its files' errors reach the error handler).
`C_AddOns.GetNumAddOns`, `GetAddOnInfo`, `GetAddOnMetadata`,
`IsAddOnLoaded` and `IsAddOnLoadOnDemand` see each of them, in load order,
and the addon itself last, as its generated TOC describes it (its `Title`,
`Version`, `Notes`, `X-…` fields), with or without `test.addons`.
Their SavedVariables (`## SavedVariables`, `## SavedVariablesPerCharacter`)
survive `mock.reloadUI()` as the addon's do. An error while one loads fails
the file, naming the addon. Coverage counts only the addon's own files.

### 16.1.3 `wowlua:test`

| Export | Purpose |
|--------|---------|
| `@test(name?, opts?)` | Marks a test function (top level); the name defaults to the function's; `opts.timeout` (virtual seconds) for its waits |
| `@beforeEach`, `@afterEach`, `@beforeAll`, `@afterAll` | Hooks, per file |
| `@skip(reason?)`, `@only` | Test selection |
| `@flavors("standard", …)` | Run only for these flavors |
| `config({ lifecycle: "auto" \| "manual", timeout?, profile? })` | Per-file runner settings (`profile`: the world profile, read before anything loads) |
| `describe(name, fn)` | Grouping for runtime-generated tests (`test(name)(fn)` inside `fn`) |
| `expect(v)` | Matchers: `toBe`, `toEqual` (deep), `toBeNil`, `toBeTruthy`, `toBeFalsy`, `toBeInstanceOf`, `toContain`, `toHaveLength`, `toBeCloseTo`, `toThrow`, `toHaveBeenCalled`, `toHaveBeenCalledWith`, `toHaveBeenCalledTimes`, `toMatchSnapshot`; for widgets `toBeShown`, `toBeVisible`, `toHaveText`, `toHaveSize`, `toBeAnchoredTo`. Negation with `expect(v).never.toBe(x)`. Type-checked (below). |
| `@cases(list)` | Runs a test once per case, the case's values as its arguments (below) |
| `@expectError(code, …)` | A compile-time test: the function must not compile, with these diagnostics (below) |
| `matcher(name, fn)` | Defines a typed matcher (below) |
| `@bench(name?)` | A benchmark, run by `wowlua bench` (M11), not by `wowlua test` |

**Typed matchers** (decision T214). `expect` is generic: `expect(x)` has the
type `Expect<T>` for the type `T` of `x`, and its matchers take `T`:
`expect(count).toBe("3")` is a type error, as is a matcher on a kind of
value it does not take (E8121: `toHaveLength` and `toContain` need a string
or a table, `toBeCloseTo` a number, `toThrow` and the spy matchers a
function). A spy keeps the type of the function it spies on, so
`toHaveBeenCalledWith(...)` checks its arguments against that function's
parameters. Matchers are called with a dot: `expect` returns a table of
them.

**Parametrized tests** (decision T215). `@cases` gives a test its cases;
each runs and is reported on its own, named after the test and the case's
values (`doubles [2, 4]`), and each is checked against the test function's
parameters (E8120 when a case does not fit):

```wowlua
@test("doubles")
@cases({ { 1, 2 }, { 2, 4 }, { -3, -6 } })
local function doubles(n: number, expected: number)
    expect(double(n)).toBe(expected)
end
```

**Waiting** (decision T216). A test may wait for the world, written as
straight-line code: `mock.waitFor(event, predicate?)` (the event's arguments
are returned), `mock.waitFrames(n)`, `mock.waitSeconds(s)` and
`mock.waitUntil(fn)`. A test runs as a coroutine; waiting advances the virtual
clock and the frames (60 a second, each running `OnUpdate` and the due
timers), so a test stays deterministic and runs instantly. A
wait longer than `timeout` virtual seconds (default 10, `config({ timeout })`
or `@test(name, { timeout })`) fails the test, naming what it waited for.

```wowlua
@test("loads the item first")
local function itemLoads()
    tooltip:showItem(19019)
    local itemID = mock.waitFor("ITEM_DATA_LOAD_RESULT")
    expect(itemID).toBe(19019)
    expect(tooltip.frame).toHaveText("Thunderfury, Blessed Blade of the Windseeker")
end
```

**Widget matchers** (decision T217): `toBeShown()` (`IsShown`),
`toBeVisible()` (shown with every parent), `toHaveText(text)` (a font string,
an edit box, a button's text, or a frame with one font string), `toHaveSize(w,
h)`, `toBeAnchoredTo(other, point?, relativePoint?)`, and
`expect(GameTooltip).toHaveLines({ … })` for a tooltip's left and right texts,
line by line.

**Snapshots** (decision T218). `expect(v).toMatchSnapshot(name?)` compares a
value (serialized deterministically: keys sorted, numbers with at most two
decimals, frames as their widget tree with shown state, size, points and
text, then their regions and children) with the snapshot stored in
`__snapshots__/<test file>.snap` next to the test, keyed by the test's name
and `name` (else numbered); a missing snapshot is written, a different one
fails the test with a line diff, and `wowlua test -u` rewrites them (and,
when every test of the file ran, drops those no test matched). Snapshots
are reviewed in version control: each is a `# <key>` line and its text,
each line after `| `.

**Compile-time tests** (decision T219). `@expectError` marks a function that
must not compile: the test passes when the checker reports exactly these
diagnostics in it (by code, optionally `{ code, message }`), and the function
is never run. Libraries pin down what their types reject, the stdlib its own
diagnostics:

```wowlua
@expectError("E2001")
local function wrongArgument()
    tracker:record("two")    -- record takes a number
end
```

**Custom matchers** (decision T220). `matcher(name, fn)` adds a matcher to
`expect`, typed by `fn`'s signature: `fn(actual: T, ...): (boolean, string)`
returns whether it passed and the message; a library exports its matchers and
a test file that imports it gets them, checked like the built-in ones.

**Benchmarks and examples.** `@bench` functions in test files are run by
`wowlua bench` (M11, decision T221); code examples in `---` doc comments run
as tests with the language server's documentation work (M9 S8, decision
T222).

### 16.1.3.1 Module internals and mocked imports

A test file may import what a module does not export (decision T223):
`import internal parse, Cache from "/tracker"` binds the module's
file-level locals and types by name, as if exported, typed as the module has
them; only test files may (E8119 elsewhere). The module registers the
locals test files read at its end (their values then), in test builds only:
an addon build is unchanged.

`mock.module(namespace, replacement)` replaces a module's exports for the
rest of the test file (decision T224): `namespace` is the module's namespace
import (`import * as clock from "/clock"`), `replacement` the exports to
replace, checked against the module's export types (E8123 for one it does
not have, a type error for a mistyped one); the others stay. In a test
build the addon's files read their imports at each use, so every importer
sees the replacement (a value an importer stored before keeps the old
one).

### 16.1.4 `wowlua:mock`

`wowlua:mock` drives the emulated client ([WoW client emulation](emulator.md)):
every API function of the flavor exists, generated from Blizzard's
documentation with the client's argument checks, and reads a **simulated
game world** that tests set up and change. A change to the world fires the
events the client fires for it, in the client's order (decision T201).
Functions the flavor lacks are absent, as in the game; a function without
behaviour raises `"<name> is not mocked"` unless a test fakes it. A widget
method without behaviour follows its name: `SetX(...)` and `EnableX(...)`
store their arguments, `GetX()` returns what `SetX` stored, `IsX()` whether
its first value was true (`IsXEnabled` reads `EnableX`), and another method
that returns nothing does nothing; only a method with results and nothing
stored is not mocked.

```wowlua
@test("shows the target's name")
local function targetName()
    local hogger = mock.spawnUnit({ name = "Hogger", level = 11, reaction = "hostile", health = 666 })
    mock.player:target(hogger)        -- PLAYER_TARGET_CHANGED
    expect(MyFrame.title:GetText()).toBe("Hogger")
    hogger:damage(100)                -- UNIT_HEALTH ("target")
    mock.player:enterCombat()         -- PLAYER_REGEN_DISABLED; combat restrictions apply
    expect(issecretvalue(UnitHealth("target"))).toBe(true)
end
```

**The world.**

| Part | Controls |
|------|----------|
| Units | `mock.player`, `mock.spawnUnit(data)` returning a unit: name, GUID (generated unless given), level, class, race, sex, reaction (`"hostile"`, `"neutral"`, `"friendly"`), player or NPC, health and power, dead or alive, distance from the player (yards). Unit methods act and fire events: `unit:damage(n)`, `unit:heal(n)`, `unit:kill()`, `unit:setPower(n, { frequent? })`, `unit:applyAura(data)` (`quiet = true`: already there, no `UNIT_AURA`), `unit:removeAura(id)`, `unit:castSpell(id, opts?)` (`UnitCastingInfo` / `UnitChannelInfo`, `UNIT_SPELLCAST_START` then `SUCCEEDED` and `STOP`, or the channel's; `opts.castTime` in seconds, else the spell data's, else 1.5; the order unverified), `unit:interruptCast()`, `mock.player:setMoving(moving)` (`PLAYER_STARTED_MOVING` / `PLAYER_STOPPED_MOVING`; a spell with a cast time refused meanwhile), `unit:setDistance(yards)` (`CheckInteractDistance`, `UnitInRange`, `UnitDistanceSquared`), `unit:despawn()`, `caster:resurrect(target, { health? })` (the dead player is offered it: `RESURRECT_REQUEST`, Blizzard's popup, `AcceptResurrect` a round trip later alive with `PLAYER_ALIVE`; another dead unit lives at once; unverified). `mock.setPlayer(data)` sets the player's data without events; `mock.unitOf(token)` is the unit a token names (a world profile's target, say), nil when none. `mock.setPet(unit?, { hunter, happiness, loyalty, xp, xpMax, actions })`: the player's pet (`pet`; `UNIT_PET`, `PET_BAR_UPDATE`, `UNIT_HAPPINESS`; Blizzard's pet frame and pet bar, `HasPetUI`, `GetPetActionInfo`, `CastPetAction`, `C_PetInfo.GetPetHappiness`, `GetPetExperience`; `pet:setStats` its stats; `nil` dismisses it; unverified). |
| The character | `mock.player:setMoney(copper)` (`PLAYER_MONEY`), `mock.player:equip(slot, item)` (`PLAYER_EQUIPMENT_CHANGED`, `UNIT_INVENTORY_CHANGED`; `GetInventoryItem…`), `mock.setBag(bag, { slots, items })` (`BAG_UPDATE`, `BAG_UPDATE_DELAYED`; `C_Container`, `C_Item.GetItemCount`), `mock.setItemCooldown(itemID, duration, remaining?)` (`BAG_UPDATE_COOLDOWN`; `C_Container.GetContainerItemCooldown`, `C_Container.GetItemCooldown`), the server's bag packets: `mock.setBagOrder({ bag, … })` (the order a session's server tells the bags' updates in), `mock.setItemDataDelay(seconds)` (a split stack's data after the answer), `mock.setSortResult({ [bag] = { [slot] = { id, count, shownFirst, after } } })` (where `C_Container.SortBags` puts the items), `mock.moveTo({ mapID, zone, subZone, x, y, world })` (`ZONE_CHANGED_NEW_AREA`, `ZONE_CHANGED`; `GetZoneText` and the other zone texts, `C_Map.GetBestMapForUnit`, `C_Map.GetPlayerMapPosition`, `UnitPosition`). An item is its ID or `{ id, count, link, bound, locked, cooldown = { remaining, duration } }`; links are the test's, the item data's, or made like the client's. The events and their order are the client's (`/stdtest engine record`); camelot's links have its form (`|cnIQ1:|Hitem:…`). |
| Vendors | `mock.openMerchant({ name, items = { { id, price, stack, available, maxStack, usable, itemAfter } }, canRepair, repairCost, damaged, cached, repairMoneyAfter, repairCosts })` (`MERCHANT_SHOW`, the interaction frame, then `MERCHANT_UPDATE` unless `cached`; the vendor is the `npc` unit; `itemAfter`: a bought item's data that many seconds after the answer; `repairMoneyAfter`: a repair's money after that many items' durability; `repairCosts`: each worn slot's mending price, the repair cursor's `PickupInventoryItem` mending one), `mock.closeMerchant()`, `mock.sell(bag, slot)` (the user's right-click); buying, selling, buying back and repairing through the client's API (`BuyMerchantItem`, `BuybackItem`, `RepairAllItems`, `C_MerchantFrame`), answered by the server a round trip later with the recorded events |
| Unit tokens | Follow from relationships, as in the client: `mock.player:target(unit)`, `:focus(unit)`, `:mouseover(unit)`, `mock.player:softTarget(unit, kind?)` (`softenemy`, `softfriend`, `softinteract`), `unit:showNameplate()` (`nameplateN`), `mock.group:add(unit, raid? | { raid?, subgroup? })` (`partyN`; with `raid`, `raidN`; the player leading), bosses (`bossN`); compound tokens (`targettarget`) resolve through them. |
| Combat and restrictions | `mock.player:enterCombat()`, `:leaveCombat()`; `mock.enterInstance(kind, { name, instanceID, difficultyID, difficultyName, maxPlayers }?)` (a restricted map; with a kind `IsInInstance`, `GetInstanceInfo`), `mock.leaveInstance()`, encounters (`mock.startEncounter({ id, name, difficultyID, groupSize, bosses })`: `ENCOUNTER_START`, the bosses on `bossN`, `INSTANCE_ENCOUNTER_ENGAGE_UNIT`; `mock.endEncounter(success)`: `BOSS_KILL`, `ENCOUNTER_END`; unverified), challenge modes, PvP matches, chat lockdown. The client's addon restrictions follow ([WoW client emulation §17.5](emulator.md#175-secret-values)): `ADDON_RESTRICTION_STATE_CHANGED`, secret API results, protected functions refused. |
| Time | `GetTime` and `C_Timer` on a virtual clock: `mock.advanceTime(sec)`; `OnUpdate` per simulated frame: `mock.nextFrame()`, `mock.advanceFrames(n, ms?)`; the client's batching of a frame's events to its end: `mock.setFrameBatching(on)`; the damage meter's next session's ID: `mock.setDamageMeterSession(id)`; waiting in a test: `mock.waitFor`, `mock.waitFrames`, `mock.waitSeconds`, `mock.waitUntil` (§16.1.3) |
| Lifecycle | `mock.login()` (`ADDON_LOADED`, `PLAYER_LOGIN`, `PLAYER_ENTERING_WORLD`), `mock.loadAddon(name)`, `mock.reloadUI()` (`/reload`: the pass ends; the test runs again from the top in a fresh VM that kept only the TOC's SavedVariables, as the client writes them, logged in as a reload; `mock.reloads()` counts the passes; the file's other tests stay in the first VM; code that calls `ReloadUI()` itself ends the session there, `PLAYER_LOGOUT` and the rest, nothing of it running after, and the pass at the test's next wait) |
| Communication | `mock.receiveAddonMessage(prefix, message, chatType, sender)` (another player's), `mock.sentAddonMessages()`, `mock.joinGuild()`, `mock.systemMessage(text)` (the server's `CHAT_MSG_SYSTEM`); the player hears their own group and guild messages and whispers to themselves on the next frame, for registered prefixes; WoW Forever's characters are addressed by first and last name (`unit:setName(name, lastName)`), as `/stdtest engine comm` recorded |
| Addons | `mock.defineAddon(name, { metadata, loadOnDemand, load })`: an addon the client knows (`C_AddOns.GetAddOnMetadata`, `GetAddOnInfo`, `LoadAddOn`) |
| Input | `mock.slash(line)`, `mock.defineBinding(name, action, { header, runOnUp })` (from `Bindings.xml`), `mock.pressBinding(name)`, `mock.pressKey(key)` (the key's binding, as `SetBinding` bound it; `ENTER`, `ESCAPE`, `TAB` and `SPACE` go to the focused edit box first), `mock.click(frame, button?)` (a visible frame: `OnMouseDown`, `OnMouseUp`, then an enabled button's `PreClick`, `OnClick`, `PostClick`; the order unverified), `mock.hover(frame?)` (`OnLeave`, `OnEnter`; `GetMouseFoci`, `IsMouseOver`), `mock.typeText(editBox, text)` (keyboard focus, then `OnChar` and `OnTextChanged(true)` per character; `HasFocus`, `GetCurrentKeyBoardFocus`), `mock.drag(frame, dx, dy)` (the frame, or an overlay's parent, moves if it starts moving), `mock.sizeBy(grip, dw, dh)` (a resize grip's frame), `mock.wheel(frame, delta)`, `mock.script(frame, name, ...)` (any other user action), `mock.setModifiers(...)`, `mock.setCursorPosition(x, y)` (where the mouse is: `GetCursorPosition`, screen units) |
| UI | tooltips (`mock.showTooltip(kind, data, tip?)` with Blizzard's UI code: an item `{ id }`, `{ link }`, `{ bag, slot }` or equipped `{ slot }`, a spell `{ id }`, a unit `{ unit }`; the engine's `C_TooltipInfo` gives the data from the tests' game data, Blizzard's tooltip code shows it and runs the addons' hooks; returns the lines), menus and dialogs, Settings, the screen size, locale, CVars, other addons and libraries (`mock.provideLibrary`); `mock.frames()` (every frame, in creation order); stand-ins for Blizzard's UI code: `mock.defineTemplate(name, fn)` (`CreateFrame` with the template calls `fn(frame)`; an undefined template only warns) and `mock.defineFrameType(name, base, fn)` (an intrinsic frame type such as `EventFrame`) |
| Output | `mock.printed()` (what the chat frame shows: `print`'s lines, and with Blizzard's UI everything `DEFAULT_CHAT_FRAME:AddMessage` gets), `mock.errors()` |

**World profiles** (decision T213). A test file starts in the situation a
profile describes: `config({ profile = "stormwind" })` (a literal, read
before anything loads) or `test.profile` of the manifest names
`tests/profiles/<name>.json`, applied through the controls above before the
addon loads:

```json
{
  "character": { "name": "Thrall", "realm": "Realm", "class": "SHAMAN", "race": "Orc", "level": 60 },
  "money": 123456,
  "items": { "19019": { "name": "Thunderfury, Blessed Blade of the Windseeker", "quality": 5 } },
  "spells": { "403": { "name": "Lightning Bolt", "known": true } },
  "equipment": { "MainHandSlot": 19019 },
  "bags": { "0": { "slots": 16, "items": { "1": { "id": 159, "count": 5 } } } },
  "position": { "mapID": 1453, "zone": "Stormwind City", "subZone": "Trade District", "x": 0.5, "y": 0.6 },
  "auras": [ { "spellId": 546, "name": "Water Walking", "duration": 600 } ],
  "units": [ { "name": "Hogger", "level": 11, "reaction": "hostile", "distance": 12, "target": true } ],
  "cvars": { "nameplateShowEnemies": "1" },
  "bindings": { "SHIFT-1": "ACTIONBUTTON1" },
  "savedVariables": { "MyAddonDB": { "logins": 41 } }
}
```

`/stdtest snapshot` records the character's world in the client (StdTest,
then `/reload`), and `cargo xtask profile <SavedVariables/StdTest.lua>
<out.json>` writes it as a profile with the character's name and realm, a
player target's name and identifying CVars scrubbed. `character` is
`mock.setPlayer`'s data; `items` and `spells` are
`defineItem` and `defineSpell` data (items cached unless `cached` is
`false`); equipment slots are IDs or the client's slot names
(`GetInventorySlotInfo`); a unit's `target`, `focus`, `group` (`"party"`,
`"raid"`) and `nameplate` relate it to the player, its `auras` are
applied; object keys that are whole numbers become number keys. Every part
stays drivable during the test.

**Raw events.** `mock.fireEvent(name, ...)` and `mock.fireUnitEvent(name,
unit, ...)` fire any event the flavor documents, the payload checked against
its documentation (count, types, and secrets where the documentation says);
an event the flavor does not document is an error (decision T202).

**Game data.** Spells, items and other game data come from the tests
(decision T203): `mock.defineSpell(id, data)` (`name`, `iconID`,
`description`, `known`, `petKnown`, `cooldown`, `charges`, ranges, `cached`),
`mock.defineItem(id, data, { cached?, delay?, missing? })` (`GetItemInfo`'s
fields by name; `RequestLoadItemDataByID` fires `ITEM_DATA_LOAD_RESULT`
after `delay`, 0.2 s by default), in the test or in `tests/mocks/`. The data
stays the test's table, so its values may be secrets. An ID the test did not define behaves as data
the client does not have. Optionally, tests can run with the real client's
data: `wowlua data pull --wow <path to the WoW installation>` reads the
flavor's game data from the user's own client into a local cache (nothing is
distributed with wowlua), and `wowlua test --game-data` loads it. Off by
default, so tests in CI run with test data only.

**Secret values.** `mock.secret(v)` makes a secret; the restrictions above
make the API return secrets where the client does.

Mocking (decision T187):

- **Spies, stubs and fakes** for any function: `mock.spy(fn?)`,
  `mock.stub(table, key, fn)`, `mock.fake(name, impl)` for an engine function,
  restored after each test.
- **Scenario controls** set the game's state: combat, units, group, auras,
  spells, items, time, screen, locale, CVars, other addons and libraries.
- **Taint in tests.** A protected action refused because of taint fails the
  test with its taint chain ([WoW client emulation §17.4](emulator.md#174-taint-and-secure-execution)),
  unless the test expected it: `expect(fn).toBeBlocked()` asserts that
  calling `fn` is refused, and reading `mock.refusals()` marks the refusals
  so far as expected. `expect("Name").toBeSecure()` asserts that a global
  is untainted, `expect(t).toBeSecure(key)` a table's field
  (`issecurevariable`).

### 16.1.5 Running

```
wowlua test [filter] [--flavor <f>] [-u] [--watch] [--coverage] [--reporter <r>,…] [--output <dir>] [--game-data]
```

- Each test file runs once **per manifest flavor** by default. `--flavor`
  restricts the run, and `@flavors` restricts individual tests.
- Each file and flavor runs in a fresh VM. Files run in parallel.
- `filter` keeps the test files and tests whose path or name contains it.
- The test build of each flavor goes to `<build.out>/.test/<flavor>/<Name>`
  (the AddOn folder as the client would load it, with the test side next to
  it); error locations in reports are the sources' (the `test` profile keeps
  lines).
- Test reporters: `pretty` (the terminal default), `junit` (XML,
  `junit.xml`), `tap` (`results.tap`), `json` (`results.json`), `github`
  (annotations, on standard output). Several may be given (or
  `test.reporters` names them); files go to `<build.out>/test/` unless
  `--output` says otherwise.
- Exit code 1 if any test fails, or if coverage is below a threshold
  (§16.1.6).

### 16.1.6 Coverage

`--coverage` measures, mapped to the `.wowlua` and `.lua` sources
(decision T188):

| Kind | Counts |
|------|--------|
| Lines and statements | Each statement executed |
| Branches | Each arm of `if`/`elseif`/`else` (the implicit "no else" too), loops entered or skipped, both outcomes of `and`, `or`, `??` and `?.` |
| Functions | Each function, method and closure called |
| Conditions | Each condition of a compound test (`a and b`, `a or b`) both true and false |

The VM counts them; the compiled code is not changed. It counts the
instructions that run and the outcomes of conditional jumps, mapped to the
sources (the `test` profile keeps lines): a line is covered when code on it
ran, statements count as lines, a branch is one outcome of a conditional
jump, a condition one on a line with several (a compound test), and the
compiler's runtime checks and generated functions are left out. Code that
`@if` removes for the flavor under test counts as not applicable, and
coverage of all flavors is merged unless `--coverage-per-flavor` is given
(reports then in `<flavor>/`). `-- wowlua: coverage ignore` (the next line
with code, or the line it ends) and `-- wowlua: coverage ignore-file`
exclude code, and `test.coverage.exclude` excludes files.

Reports, any combination: a terminal summary (always, with each file's lines
that never ran), `lcov` (`lcov.info`, with branch and function data),
`cobertura` (`cobertura.xml`), `coverage-json` (`coverage.json`), and `html`
(`coverage.html`): a self-contained report in the wowlua design system (its
fonts and tokens, dark and light), with every file's source, hit counts,
branch markers and totals per file and folder.

Thresholds in `test.coverage` ([Manifest §9.18](manifest.md#918-test)) set
minimum percentages per kind; a run below one fails.

## 16.2 Language server (`wowlua lsp`)

A Language Server Protocol implementation over stdio, shipped in the same
binary. It shares the parser, binder and checker with the compiler.

| Feature | Details |
|---------|---------|
| Diagnostics | All compiler diagnostics, live, per flavor and profile (both switchable from the editor) |
| Hover | Types, documentation comments, class members, `wowlua:meta` folded values |
| Completion | Locals, imports with auto-import, members, string literal unions (events, anchors, frame types), decorator names, manifest keys in `wowlua.json` |
| Navigation | Go to definition (including into `.d.wowlua` and dependencies), find references, document and workspace symbols |
| Refactoring | Rename (scope-aware), organize imports, quick fixes from diagnostic `fixes` |
| Signature help | Including overloads |
| Inlay hints | Inferred types of locals and parameters, parameter names |
| Semantic tokens | Distinguishes contextual keywords from identifiers |
| Formatting | Via the formatter (§16.3) |
| Code lens | Profiler and test shortcuts (run test, run file) |

**Configuration discovery.** For each open file, the server uses the nearest
`wowlua.json` found by walking up from the file. If that manifest belongs to a
workspace, it uses the workspace root manifest. The selected flavor and profile
are stored in the editor's settings. The server never accesses the network
unless the editor setting `wowlua.lsp.fetch` allows it. Missing dependencies
produce a diagnostic suggesting `wowlua fetch`.

A VS Code extension (TextMate grammar plus LSP client) is part of the project.
Other editors use the LSP directly.

## 16.3 Formatter (`wowlua fmt`)

An opinionated formatter for `.wowlua` and `.d.wowlua`. It can optionally format
`.lua` with `fmt.lua = true`.

- Idempotent, and preserves comments (it is built on the lossless trivia kept by
  the parser).
- Configuration is deliberately small, in the manifest:
  `"fmt": { "indent": "tab" | 2 | 4, "line-width": 100, "quote-style": "double" | "single" | "preserve" }`.
  Defaults: `indent` `"tab"`, `line-width` 100, `quote-style` `"double"`
  ([Manifest §9.13](manifest.md#913-fmt)).
- `wowlua fmt --check` exits 1 if any file would change (for CI).
- Style (decision T65), following StyLua: lines are reflowed at `line-width`;
  call arguments are always parenthesized (`f "x"` becomes `f("x")`); tables
  print as `{ a = 1, b = 2 }` when they fit, otherwise one field per line with
  a trailing comma; a single table or function argument, or a function as the
  last argument, hugs the parentheses; long binary expressions break after
  the operator with a hanging indent; blocks are never kept on one line,
  except an empty function body (`function() end`); at most one blank line is
  kept; `;` is written only before a statement that starts with `(`.
  Trailing comments stay on their line, comments on their own line stay on
  their own line. wowlua's syntax (M9 S6): decorators each on their own
  line; annotations as `name: T`, `name?: T` and `name: T = default`;
  `function f<T extends X>(a: T, ...: U): R`; function types as
  `(a: A, ...B) -> R` and a results list of one `...T` without
  parentheses; table types like tables; long unions and intersections
  break before `|` / `&`; class and interface members one per line with
  their blank lines kept; enums like tables; `import`, `export`, `const`,
  `continue`, compound assignment, destructuring, `?.`, `?:`, `??` and
  `::` with single spaces; `declare` statements like the declarations they
  declare (members one per line); interpolated strings keep their text.
- Formatting never changes semantics. Every formatted file is verified to
  produce the same AST (types, decorators and declarations included); an
  idempotence suite formats every `.wowlua` file of the repository twice.

## 16.4 Trace decoder (`wowlua trace`)

Maps error messages and stack traces from bundled or minified builds back to
source locations, using the source maps of the matching build
([Code generation §14.6](codegen.md#146-source-maps)).

```
$ wowlua trace < bugsack.txt
MyAddon/MyAddon.lua:4821: attempt to index local 'a' (a nil value)
  → src/ui/window.wowlua:87 in Window:refresh   ('a' is 'self.frame')
[string "@Interface/AddOns/MyAddon/MyAddon.lua"]:5310: in function <MyAddon/MyAddon.lua:5302>
  → src/main.wowlua:22 in onLogin
```

- WoW error messages and stacks contain **line numbers only**, never columns.
  Every minification level keeps at least one line break per statement
  ([Optimization §15.4](optimization.md#154-minification-levels)), so a line
  always identifies one source statement.
- Input: pasted text, a file, or `--clipboard`. It recognizes WoW error-frame
  and BugSack formats.
- Build matching: the build id (`--build`, the `X-Wowlua-Build` TOC field,
  also included in the load errors of bundles) selects the right source maps:
  a build in `dist/` with that id, else the archive. Without one, the newest
  build in `dist/` is used. Maps of published builds
  are archived as described in [Code generation §14.6](codegen.md#146-source-maps),
  and can be uploaded as release assets (`publish.archive-maps`). That way,
  reports from users of old versions can still be decoded.
- Renamed locals are reported with their original names, looked up by line and
  minified name.
- Line-preserved builds need no map. `wowlua trace` passes their locations
  through, rewriting the output path (including any flavor directory) to the
  `.wowlua` source file, and names the enclosing function found in the source
  (`in Window:refresh`).
- Locations of other addons are left as they are.

## 16.5 Profiling reports (`wowlua prof report`)

`wowlua prof report` turns recordings exported from the dev window
([Debugging §7.7.9](../language/07-debugging.md#779-export)) into a single,
self-contained HTML page and opens it in the browser (`--no-open` only writes
it). The default output is `dist/reports/<Name>-<date>.html`; `--out` sets
another path.

- **Finding the data.** The command reads `WowluaDevDB_<Addon>.lua` of the
  current package from the SavedVariables folders of the WoW installation,
  found as for `wowlua dev` ([CLI §12.5](cli.md#125-wow-installation-integration)):
  `--wow`, `WOWLUA_WOW_PATH`, the global config, then auto-detection. Every
  client folder and every account under `WTF/Account/` is searched. When more
  than one has exported recordings, the command lists them and asks, or takes
  `--client` and `--account` (decision T179).
- **What it shows.** Every exported recording (`--recording` selects some),
  with the functions table, the call tree and flame graph when call paths
  were on, the frame timeline with the frame budget, the slow calls with
  their stacks, the garbage view and the hints found in that recording, in
  the style of the wowlua website. Locations are mapped back to source lines
  as `wowlua trace` does (§16.4), using the build id stored with each
  recording.
- **Nothing is uploaded.** The report is a local file; the command does not
  use the network.
