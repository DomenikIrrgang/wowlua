# 17. WoW Client Emulation

**Status: planned (M9).** This chapter describes the environment `wowlua test`
runs addons in ([Tooling §16.1](tooling.md#161-test-runner-wowlua-test)): a Lua
VM and a WoW engine written in Rust, faithful enough that a test catches what
the game client would raise.

## 17.1 Goals

- **Fidelity over convenience.** A test passes only if the same code would run
  in the client of that flavor. Where the emulation cannot know the client's
  behaviour, it raises a clear error instead of guessing.
- **The bugs that matter.** Taint, combat lockdown, secret values, flavor
  differences and WoW's own Lua differences are where addons break in game; the
  emulation models them, and reports them better than the game does.
- **Verified against the client.** Every behaviour is backed by the Lua 5.1
  reference, Blizzard's documentation and UI code, or an in-game probe
  (`/stdtest probe`), and the verification is automated (§17.8).
- **One binary.** The VM and the engine are part of `wowlua`, in Rust, with no
  C toolchain (decision T185).

## 17.2 The VM

A Lua 5.1 virtual machine in Rust, in its own crate (`wowlua-vm`).

- **Language.** Lua 5.1 as the client runs it: values, tables with metatables
  and every metamethod, closures and upvalues, varargs, coroutines,
  `setfenv`/`getfenv`, weak tables, `__gc` on userdata, error values and
  `pcall`/`xpcall` with tracebacks, `string.dump` absent.
- **Input.** The VM runs what the compiler produces: wowlua's output and plain
  Lua files, with the compiler's source maps, so every location the VM reports
  is a `.wowlua` (or `.lua`) line and column.
- **Standard library.** Exact Lua 5.1 behaviour: Lua patterns, `string.format`
  (including `%g` and `%q` output byte for byte), `tostring` of numbers, table
  sorting, number parsing.
- **Determinism.** Iteration order, random numbers (`math.random`,
  `fastrandom`, seeded per test), time and memory are deterministic, so a test
  run repeats exactly.
- **Instrumentation.** Coverage counters (lines, statements, branches,
  functions, conditions) and profiling are counted by the VM, not inserted into
  the code (§17.6, Tooling §16.1.6).
- **Limits.** Lua's limits apply as in the client (200 locals, 60 upvalues,
  C stack depth), so code that compiles but would not load in game fails here
  too.

Its design (§17.9): a register bytecode of its own, a tracing garbage
collector, and stackless coroutines.

## 17.3 WoW's Lua

The differences from Lua 5.1 elsewhere, as probed on WoW: Forever (decisions
S37, S38):

- **Libraries.** No `os`, `io`, `debug` or `package`; `date`, `time` and
  `difftime` as globals. The client's additions to `string`, `table` and `math`
  ([Types §2.8](../language/02-types.md#28-globals)), `bit`, and WoW's global
  aliases and helpers (`strsplit`, `strjoin`, `strtrim`, `tinsert`, `wipe`,
  `debugstack`, `debuglocals`, `debugprofilestop`, `geterrorhandler`, …).
- **Arithmetic.** `x / 0` and `x % 0` raise "Division by zero"; `math.sqrt`
  and `math.log` raise on a negative argument; `string.format("%d", x)` raises
  for an infinite `x`.
- **Limits.** The client's limits, at its thresholds, from crash-search
  probes (`/stdtest crashlimit`, separate from `/stdtest probe`): a C
  function's frame holds 8000 slots with its arguments (`unpack` and
  `string.byte` fail beyond, "too many results to unpack", "stack overflow
  (string slice too long)"); `strsplit` returns at most 4000 pieces
  ("strsplit(): Stack overflow"). The client runs its Lua with API
  assertions on and gives C functions 100 free slots: `pcall` returning
  its argument count plus 100 or more values (`xpcall` one fewer, its
  handler takes a slot; `pcallwithenv` three fewer, it pushes the function
  and its environment after them) pushes its status past them, the assertion
  `L->top < L->ci->top` fails and the game crashes. Such calls fail with
  an error naming the call (decision T194). The call depth is Lua
  5.1's: a thread's call-info array (a slot per Lua or C call) doubles
  from 8, so the first "stack overflow" comes at 16384 calls; after one,
  the array is cut back to 20000 slots and each collection may halve it
  again (the next overflow comes at 20000, then 19968, 19456 or 18432),
  and while handling one it may grow to 32768 ("error in error
  handling"). Nested C calls (`pcall`, metamethods, `loadstring`'s
  parser, which counts its syntax levels on from them) stop at 200 with
  "C stack overflow"; unlike stock Lua 5.1's, the client's coroutines
  continue their resumer's count, so nesting `coroutine.resume` stops
  there too instead of overflowing the C stack. `securecall`,
  `securecallfunction`, `securecallmethod` and the
  coroutine functions return up to the 8000 slots without crashing.
- **Per flavor.** Where flavors differ (a library function, an arithmetic
  rule), the VM follows the flavor under test. Behaviour not yet probed on a
  flavor follows the newest probed client and is listed in the emulation's
  coverage report (§17.8).

### 17.3.1 Layers

The client's in-game environment, sorted from the client itself
(`/stdtest inventory`, recorded in `crates/wowlua-vm/tests/client/`) against
Blizzard's UI code and API documentation for the build:

| Layer | What | Where wowlua has it |
|-------|------|---------------------|
| Lua runtime | Lua 5.1's libraries and the client's C extensions: the string, table, math and bit additions, the aliases (`strfind`, `tinsert`, …), `date`/`time`, `debugstack`, the secure API, and the FrameScript functions (`issecretvalue`, `securecallmethod`, `Mixin`, …) | the VM (`Vm::wow`) |
| Engine API | C functions: the documented API (`C_*` and globals) and older undocumented ones (`GetActionInfo`, …) | the engine (§17.6) |
| Blizzard's Lua | FrameXML and SharedXML: `print`, `tostringall`, `assertf`, the degree-based `sin`/`cos`, mixins, and Lua wrappers even inside `C_` namespaces (`C_Traits.GetEntryInfo`) | Blizzard's own code, loaded by the engine (§17.7) |
| GlueXML | the login and character screens' Lua | not in game; not emulated |

On WoW Forever 1.60.1 (70374): 5,037 documented C functions, 2,059
undocumented ones (151 of them the runtime's), 3,942 Lua functions, every one
defined in the published UI code for the build.

## 17.4 Taint and secure execution

The client's security model, in the VM:

- **Secure and insecure code.** Blizzard's code (the engine's Lua and, when
  loaded, FrameXML, §17.7) runs secure; addon code runs insecure, tainted with
  its addon's name.
- **Propagation.** A value written by insecure code carries that taint; secure
  code that reads a tainted value becomes tainted for the rest of its
  execution. Taint follows table fields, globals, upvalues and locals, and
  crosses coroutines as in the client.
- **The secure API.** `issecure`, `issecurevariable`, `securecall`,
  `securecallfunction`, `secureexecuterange`, `hooksecurefunc` (hooks run
  insecure without tainting the hooked call), `forceinsecure`, `scrub`, and the
  taint barriers the client has (error handlers, `__gc`).
- **Protected actions.** Protected functions and frame methods are refused
  from tainted execution: the call does nothing, the client's event fires
  with the tainting addon's name and the action, **and** the test fails.
  - Protected global functions, always: `ADDON_ACTION_FORBIDDEN` with the
    addon and the action as Lua's debug information names the call
    (`"MyAddon", "UseAction()"` from `UseAction(…)`, as `/stdtest taint`
    recorded; `UNKNOWN()` through `pcall`, `/stdtest engine protected`);
    the call returns its documented results' empty values (`false` from
    `ClearTarget`). The documentation does not mark them; the engine lists
    the known ones (targeting, casting, using actions and items, movement,
    macros), the harmless ones checked in game: TargetUnit, ClearTarget,
    FocusUnit, AssistUnit, SpellStopCasting and SpellStopTargeting, the
    movement stops and PetFollow are protected; ClearFocus,
    TargetLastTarget and StopAttack are not. So is the pet spellbook's
    `C_SpellBook.ToggleSpellBookItemAutoCast`, out of combat too (found
    in game, decision T600), and `SetSpellBookItemAutoCastEnabled` with
    it (unverified), and `C_Macro.RunMacroText` (decision T601,
    unverified).
  - Protected methods (the documentation's `IsProtectedFunction`: `Show`,
    `Hide`, `SetPoint`, `SetSize`, `SetParent`, …) and `SetAttribute` of a
    protected frame, in combat lockdown: `ADDON_ACTION_BLOCKED`, after the
    method's own argument checks (`/stdtest engine blocked`). A frame is
    protected through a secure template (`Secure…Template`; `IsProtected`
    then gives `true, true`) or `mock.protect`, and through a protected
    frame it holds, however deep (`true, false`; decision T254): such a
    parent is blocked in combat as well, and secure code may use its handle
    then (a secure snippet sizing Blizzard's spells window). A template's script
    handlers from Blizzard's XML are Blizzard's code whoever makes the frame
    (decision T243): an addon's `SecureHandlerClickTemplate` button runs its
    `_onclick` snippet on the user's click. An addon's template's handlers
    are the addon's code, but making them does not taint the code applying
    the template (decision T493): a secure group header runs its
    `initialConfigFunction` for buttons of an addon's template with an
    inline `OnLoad`. The `OnAttributeChanged` a `SetAttribute` runs starts
    as the client calls a script, untainted for a secure handler, also
    within such an `OnLoad` of an addon's `CreateFrame` (Forever 70707's
    smart navigation sets its secure values from tainted template code
    through its own attribute handler, which asserts it runs secure;
    unverified, decision T576).
    `C_RestrictedActions.CheckAllowProtectedFunctions(object)` follows the
    same rule for its object: false for tainted code in combat lockdown on a
    protected frame (UIParent among them), true on an unprotected one, so an
    addon's `GameTooltip:SetUnit` & co. show in a fight (decision T245).
- **Forbidden objects.** Tainted code may not use a widget that Blizzard's
  secure code made forbidden (`StoreFrame`), nor one that secure code put
  under access restrictions (`AddAccessRestrictions`; an addon's call does
  nothing) while they apply. Every method then raises Lua's error for a
  bad `self`: "calling 'GetWidth' on bad self (Attempt to access forbidden
  object from code tainted by an AddOn - Usage: local width =
  self:GetWidth([ignoreRect]))" when called as a method, "bad argument #1
  to '?' (…)" through `pcall(b.GetWidth, b)`. `IsForbidden`,
  `CanBeAccessedInContext`, `HasAccessConstraints` and the attributes stay
  open: `CanBeAccessedInContext` answers false. `IsForbidden` answers true
  for a frame made forbidden only: access restrictions leave it false.
  `EnumerateFrames` leaves such frames out, and `GetParent` of a frame
  whose parent is one raises that error for `GetParent` instead of
  handing the parent out. `GetChildren` and `GetPoint` hand them out.
  Secure code still uses them. Decisions T632 and T636:
  - Blizzard's aura containers restrict each button they make
    (`DenyTaintedAccessWhenAurasAreSecret`, the only restriction), right
    after the addon's `initializeFrame` has built it, or at
    `PLAYER_ENTERING_WORLD` for buttons made before login. The addon's
    build works: textures, frames, `SetSize` on the button.
  - The restriction applies while aura information is secret
    (`C_Secrets.ShouldAurasBeSecret()`: in combat, an encounter, a
    challenge mode or a PvP match). In game, out of combat, the button
    answered its `GetWidth`, a child's `GetParent` handed it out and
    `EnumerateFrames` listed it; in combat both calls raised the error
    above. Its `IsForbidden` answered false in and out of combat.
  - Unverified (probe "Aura buttons forbidden after their build"):
    `CanBeAccessedInContext` and `HasAccessConstraints` on such a button,
    `EnumerateFrames` in combat (left out here), an encounter, a challenge
    mode or a PvP match out of combat, the error's form through `pcall`
    for a frame made forbidden, and `GetParent` under a parent made
    forbidden.
- **Taint reports.** A refusal reports the taint chain: the protected call,
  the secure code that became tainted, the value it read, and the addon code
  (file and line) that wrote that value, through any number of hops through
  secure code (decision T196). This is what `taint.log` cannot show:

  ```text
  UseAction() blocked (ADDON_ACTION_BLOCKED)
    ActionButton_OnClick  FrameXML/ActionButton.lua:211
      read field 'action'
    written by ActionBarButton_Update
      FrameXML/ActionButton.lua:98
      while tainted by MyAddon, after reading
    global 'ACTIONBAR_PAGE'
      written by MyAddon  src/bars.wowlua:42
  ```

  Every tainted value knows the event behind its taint: the store that put
  it there, and what tainted the code that stored it; every tainted
  execution, the read that tainted it. Tests can also assert that a value or
  a handler stays secure, and get the same chain when it does not (decision
  T197).
- **Taint trace.** `WOWLUA_TAINT_TRACE=1` (or `all`) prints to stderr each
  new site where secure execution became tainted, and each site where secure
  code stored a tainted value, with the chain behind it and the Lua calls
  leading there. Each is printed once per site and cause. Any other value
  keeps only the sites whose source contains it
  (`WOWLUA_TAINT_TRACE=Blizzard_AuraContainer`). This finds where Blizzard's
  code picks up an addon's taint before anything is refused:

  ```text
  taint: secure code stored a value tainted by SuuUI
    written by Set
      Interface/AddOns/Blizzard_SharedXMLBase/Flags.lua:71
      while tainted by SuuUI, after reading
    upvalue 'channels'
      at Interface/AddOns/Blizzard_SharedXMLBase/Flags.lua:71
      at Interface/AddOns/Blizzard_SharedXML/MixinUtil.lua:398
      at ...d_AuraContainer/Blizzard_AuraContainerFlowLayout.lua:23
      at src/UI/Auras.lua:321
  ```

## 17.5 Secret values

Secret values (WoW Forever and the 12.x clients) are a value kind of the VM,
as the client has them (probed on WoW Forever 1.60.1, `/stdtest probe`):

- **What tainted code may do.** A secret keeps its type: `type`, passing,
  storing as a table value, `select`, `unpack`, `pcall` and `error` work,
  and so does whatever needs only the type: truth tests of a secret number,
  string or table (a secret `nil` is false), `not` of those, and `==` with
  a value of another type (false) or of a secret `nil` with `nil` (true).
  Secret strings and numbers concatenate into a secret string, and
  `tostring`, `string.format`, `strjoin` and `strconcat` of a secret give a
  secret string.
- **What it may not.** Arithmetic, comparisons (`==` of two values of one
  type too), `#`, indexing, calls, `..` of other types, and a secret
  boolean's truth test fail with the client's error, naming the operand as
  Lua does: "attempt to compare local 's' (a secret number value, while
  execution tainted by 'MyAddon')", or without a name "attempt to compare a
  secret number value (execution tainted by 'MyAddon')". A secret table key
  fails for reads and writes ("attempted to index a table that cannot be
  indexed with secret keys"). Library functions see the type `secret` ("table
  expected, got secret") and refuse to convert one ("attempt to perform
  string conversion on a secret string value (execution tainted by …)");
  the client's own functions with usage messages add "Secret values are only
  allowed during untainted execution for this argument".
- **The functions.** `secretwrap`, `issecretvalue`, `hasanysecretvalues`,
  `scrubsecretvalues`, `scrub`, `canaccesssecrets`, `canaccessvalue`,
  `canaccessallvalues`, `canaccesstable`, `issecrettable`,
  `dropsecretaccess`, `mapvalues`, `dumpobject`, `settablesecurity` and
  `securecopy` behave as in the client.
- **Secure code** may use secrets; what it computes from one is secret again
  (decision T195, from Blizzard's documentation: addon code cannot observe
  it). That includes calling a secret function: the function it hides runs
  and its results are secret.

The engine returns secrets where the client does: from the documentation's
`SecretReturns`, `SecretArguments` and `SecretWhen…` flags, under the
addon restrictions in effect (combat, encounters, challenge modes, PvP
matches, restricted maps and chat lockdown; `/stdtest restrictions`
records the client's by the restrictions active, in
`tests/client/forever-camelot-restrictions.lua`). The rules above do not
change with them: every conformance case gives the same result in combat on
a restricted map as out of it.

A widget's alpha set from a secret (`SetAlpha`, `SetAlphaFromBoolean`) is
its secret alpha aspect: `GetAlpha` reads back a secret, and keeps doing so
after a plain alpha is set, whose value still applies (decision T251;
found in game). What takes the aspect away again is unverified.

A widget shown or hidden from a secret (`SetShown(secret)`, which only
secure code may pass) follows the value the secret hides: a secret false
or nil hides it (decision T575; Blizzard's aura buttons hide themselves
with `SetShown(secretwrap(false))` when they hold no aura). `IsShown`
answers a plain boolean (unverified: whether the client answers a secret
there).

A status bar's value or range set from a secret is its bar value aspect:
`GetValue`, `GetInterpolatedValue` and `GetMinMaxValues` read back
secrets, `OnValueChanged` and `OnMinMaxChanged` get the value and range
as secrets, and `HasSecretAspect(Enum.SecretAspect.BarValue)` is true,
until a plain value (or range) is set again (unverified: the scripts'
values, and what the client keeps).

A scroll frame's ranges (`GetVerticalScrollRange`,
`GetHorizontalScrollRange`) are secrets while a region in its scroll child
(the child itself, its children, theirs, shown or not) shows a text set
from a secret (`SetText`, `SetFormattedText`); its sizes, its child's and
its scroll offsets stay plain, and once the text is plain again so are the
ranges (decision T607). Found in game: Blizzard's character stats list in
SuuUI's scroll frame made its vertical range secret in combat while the
heights and the offset stayed plain, and an empty scroll frame in combat
answered a plain range. Unverified: that the secret text is what does it
(the stats' rows show the player's health and power), whether a secret
alpha, a bar's secret value or a hidden region count too, and the
horizontal range, which follows the vertical one here.

A status bar given a duration (`SetTimerDuration(duration, interpolation,
direction)`) follows it: its range becomes 0 to the duration's length in
seconds, and its value the seconds passed
(`Enum.StatusBarTimerDirection.ElapsedTime`, the default) or left
(`RemainingTime`), read at each `GetValue` and moved each frame for
drawing; secret while the duration is. A value or a range set stops it
(Blizzard's encounter timeline sets no range for its timer bars, and
releases a timer with `SetMinMaxValues(0, 0)`; decision T487;
unverified: the range and value the client reports, and whether it runs
`OnValueChanged` or `OnMinMaxChanged`, which the engine doesn't).

A status bar's texture without points of its own is laid out over the
part of the bar its value fills, from the right (or the top, vertical)
when the bar fills in reverse, secret value or not, so a frame anchored to
its edge follows the value (decision T490). `GetNumPoints` still gives 0
(unverified: the points the client reports for it).

While the bar's value or range is secret, that texture's anchoring is
secret (`IsAnchoringSecret()` true), and so is the anchoring of every
region whose rectangle comes from it, through any number of anchors (a
point without a relative is to the parent; a frame anchored to the bar
itself stays plain). A region whose anchoring is secret answers its
rectangle's getters, those the documentation marks
`SecretWhenAnchoringSecret`, with secrets: `GetWidth`, `GetHeight`,
`GetSize`, `GetRect`, `GetScaledRect`, `GetLeft`, `GetRight`, `GetTop`,
`GetBottom`, `GetCenter`, `GetPoint` and `GetPointByName`, every value of
the answer, the height of a horizontal bar's fill too. A plain value
again makes them plain (decision T638). Found in game: SuuUI's incoming
heals bar, anchored by two points to the health fill of another unit
(`UnitHealth`, a secret), answered `GetHeight()` with a secret number, and
comparing it raised "attempt to compare local 'a' (a secret number value,
while execution tainted by 'SuuUI')". A size or offsets set from secrets
(`SetSize(secretwrap(…))`, `SetPoint(secretwrap(…))`, which only secure
code may pass: Blizzard's aura containers do) leave the rectangle plain:
the buttons of such a container answered a plain `GetWidth` in game.
Unverified (probe "Secret sizes and anchors"): the fill's own rectangle
and its height, a frame anchored by one point, a single point's
position, `GetPoint`'s answer, a size or offsets set from secrets, the
size `OnSizeChanged` is given (plain here), and the documentation's other
such functions (`IsMouseOver`, `Intersects`, a font string's measures),
which answer plainly here.

A protected frame anchored to a region (a texture, a font string, a line,
a mask texture) is refused with the client's error, out of combat too:
"Action[SetPoint] failed because[Cannot anchor protected frames to
regions]: attempted from: Button:SetPoint." (decision T252; found in game).
So is one anchored to a frame whose anchors lead to a region, through any
number of frames (an anchor without a relative is to the frame's parent):
"Action[SetPoint] … attempted from: Button:SetAllPoints.", the action
named `SetPoint` for `SetAllPoints` too (decision T628; found in game one
frame deep). The check is made when the frame anchors, for its own
protection (a secure template), not a secure child's; unverified: deeper
chains, the parent's place, a frame protected through its children.

Some global functions are blocked for tainted code in combat lockdown
only, with `ADDON_ACTION_BLOCKED`: `C_SpellBook.PickupSpellBookItem`
(decision T253; found in game) and `C_GuildInfo.GetMOTD`, which
Blizzard's deprecated guild script also names `GetGuildRosterMOTD`
(decision T265; found in game). Blizzard's own code, the secure drag
handlers among it, may call them.

Others are forbidden to tainted code in combat lockdown only, with
`ADDON_ACTION_FORBIDDEN` and nothing done: `SetCVar` of a secure CVar
(recorded for `SoftTargetEnemy` from `/run`), and on camelot
`SetRaidTarget` (decision T631; found in game: the unit menu's raid
target icons, tainted by an addon's restyle, were refused in combat with
"SetRaidTarget()" through Blizzard's `SetRaidTargetIcon`; untainted they
marked in combat). Out of combat an addon may call them, and so may
Blizzard's untainted code in combat. On the other flavors `SetRaidTarget`
is not restricted (every flavor's documentation marks it
`HasRestrictions` without saying what; unverified).

Raid target markers are kept (decision T631): `SetRaidTarget(unit,
index)` puts marker 1 to 8 on a unit that exists, taking it off the unit
that had it and the unit's old marker off, and 0 takes the unit's marker
off; the server's answer comes a round trip later, when the marker
changes and `RAID_TARGET_UPDATE` fires (no payload; none for no change).
`GetRaidTargetIndex(unit)` answers the unit's marker, else nil (secret on
camelot, by the documentation's `SecretReturns`), so Blizzard's target
frame, nameplates and unit menu show it. Unverified: the round trip (the
client may set a solo player's marker at once), whether the index is
secret out of combat, a group's rules for who may mark, and a dead unit
keeping its marker.

NPCs are tagged (decision T635): the first player-controlled unit to hit
an NPC no one has tagged (a test's `unit:hit`, `unit:swing` or `kill`'s
`by`) tags it, a player unit for itself and the player's pet for the
player; an NPC's blows tag nothing, and later blows change nothing.
`UnitIsTapDenied(unit)` is true for an NPC tagged by someone outside the
player's group (another player unit, or a player the world has no unit
for: `tappedByOther`, `unit:tap()`), and false for one the player or a
group member tagged, one no one tagged, a player-controlled unit (a
player, the player's pet) and a unit that does not exist (recorded false
for every token with nothing tagged). Its tap denied, or no longer,
fires `UNIT_FACTION` for each of the unit's tokens, on which Blizzard's
target frame greys a tap-denied unit (`CheckFaction`); the player's own
tag fires none (recorded: the `combat` scenario, a mob the player pulled
and killed, tells no `UNIT_FACTION`). Unverified (probe "UnitIsTapDenied
of a mob tagged by another"): the event for another's tag and its order
among the hit's, a missed blow tagging, the tag kept on the corpse, a
tagger joining or leaving the group (the answer follows the group,
untold), and the tag lost when the mob leaves combat (not modelled).

The encounter timeline keeps addons' timers (`C_EncounterTimeline.
AddScriptEvent`, decision T255; found in game though the timeline is off
on Forever): listed, read, paused, resumed, finished, cancelled and ended
when their time runs out, with `ENCOUNTER_TIMELINE_EVENT_ADDED`,
`…_STATE_CHANGED` and `…_REMOVED`. The functions that require a timeline
event answer nothing for an ID not on it, as recorded. The IDs, the info's
fields and the events' order follow the documentation, unverified.

`IsMouseOver(top, bottom, left, right)` measures the mouse against the
region's rectangle, each edge moved by its offset (up or right for a
positive one), whatever covers it (decision T257; documented). A region
without points has no rectangle; a hidden one's rectangle still counts
(seen in game). The mouse is off every frame at first; the test runner's
hover and click put it on the frame.

The chat windows' settings are kept (decision T263): what
`SetChatWindowName`, `…Size`, `…Color`, `…Alpha`, `…Shown`, `…Locked`,
`…Docked`, `…Uninteractable`, `…SavedPosition` and `…SavedDimensions`
set is what `GetChatWindowInfo`, `GetChatWindowSavedPosition` and
`GetChatWindowSavedDimensions` answer after, over the recorded answers.
No events follow (unverified).

`ClearPoint(point)` removes that one point and keeps the others (decision
T258; documented): Blizzard's menus flip a point this way to open above
their owner near the screen's bottom.

An item put on a part stack of the same item in the bags stacks onto it
(decision T256; recorded by SuuUI's bag sort probe): the target slot
locked at once, the cursor let go, and a round trip later the source's
bag update, the target's lock gone, its bag update and
`BAG_UPDATE_DELAYED`. A split part stacks the same way, and what doesn't
fit stays in the source slot; a part stack put on a full stack of its own
swaps places with it (all seen in game; their events unverified). A split
part onto a full stack: nothing (unverified).

An accepted quest is watched only while `autoQuestWatch` is on (decision
T267; seen in game: off, `GetQuestWatchType` answers nothing for it, and
Blizzard's UI still focuses it); a watch added or removed by hand stays
so through the log's updates.

## 17.6 The engine

The WoW API the VM's code calls, per flavor.

- **Generated.** Every function, namespace, event, enum and widget method is
  generated from the flavor's API documentation (the same source as the
  `wow-api` declarations, decision T83): its signature, its existence in the
  flavor, whether it is protected or restricted, and which of its arguments and
  results are secret. A function the flavor lacks is absent, as in the client.
  The documentation comes as data (decision T199): `wowlua-types-gen` writes
  each flavor's as JSON, every key kept, to
  `crates/wowlua-api/data/<flavor>.api.json.gz`. Functions the client has
  but does not document (`UseAction`, about 2,000 on WoW Forever, from
  `/stdtest inventory`) exist too, with their behaviour or "not mocked";
  on standard, mists, tbc and vanilla, whose clients StdTest does not record yet,
  the client's C functions are those of Ketho's dump of it (the one the
  flavor's declarations take, `data/<flavor>-functions.txt`), so a
  function only some clients have exists on theirs alone (decision T510).
  What every client answers with nothing there (no vehicle bar, pet
  battle, ready check or raid target; the flavor's expansion) answers on
  every flavor, and `C_CombatLog.GetCurrentEventInfo`, which only the
  classic clients give addons, answers the payload of the combat log
  event last fired. As recorded on those clients (decision T511), each
  class's colour is Retail's (Classic Era's Shaman the Paladin's pink,
  a class the client lacks none), and the classic clients refuse
  `GetTalentGroupRole` with "Script_<name>: API unsupported in this version
  of World of Warcraft." (Mists also the talent tabs' `GetNumTalentTabs`,
  `GetNumTalentGroups` and `GetUnspentTalentPoints`, TBC and Classic Era
  the tiers' `GetNumSpecGroups` and `GetTalentTierInfo`), and their
  `GetRuneType` answers two runes of each type (1, 1, 3, 3, 2, 2) on every
  class, and no value for another rune. The expansion
  getters answer the client's (the server's only after login, as the
  client's does), and the classic clients' `GetWeaponEnchantInfo` answers
  no enchant on each hand (Mists: two hands, 8 values; TBC and Classic Era
  also the ranged slot, 12).
  The `Enum` and `Constants` the client changes and the number globals it
  defines itself (`LE_*`) are camelot's recording; on standard, mists, tbc
  and vanilla, what the documentation lacks comes from Ketho's dump of the
  client (its `LuaEnum.lua`, at the commit the declarations take: older
  than an installed client, so it only adds), and the tables the flavor's
  own `EnvironmentCleanup.lua` removes are secure, as camelot's recorded
  ones (`data/<flavor>-enums.txt`, `data/<flavor>-globals.txt`; decision
  T554). A `Constants` table the recording has that a newer build's
  documentation names otherwise, while that build's code still reads the
  recorded name, keeps its recorded values until the battery is recorded
  again (camelot 70707's `LevelConstsExposed`, documented as
  `LevelConstsExposed_Camelot`; decision T576). `C_UIColor.GetColors` answers the installed client's `GlobalColor`
  table, by name, each byte times 1/255 in floats as the client's
  (`data/<flavor>-colors.txt`, which `cargo xtask types gen` writes).
- **Natives.** The API functions are natives of the VM, as the client's are C
  functions (decision T198): `[C]` in stack traces, the client's argument
  errors, no taint of their own.
- **Behaviour.** Functions have hand-written behaviour where tests need it:
  the widget system (frames, regions, textures, font strings, scripts, events
  and unit events, points and sizes, show and hide, parenting, templates,
  mixins, protected frames), the event loop and `OnUpdate` frames (each by
  its `SetOnUpdateMode`, decision T594), timers on a
  virtual clock, combat lockdown, units, auras, spells and items (from test
  data), skill lines (the one selected is the one `SetSelectedSkill`
  chose, decision T357), saved variables across reloads, slash commands, keybindings, addon
  messages, settings, tooltips, menus and dialogs. The M8 test mock is the
  starting point.
- **Powers.** A unit has every power its class has in the flavor table
  ([Manifest §9.4.1](manifest.md#941-flavor-table); decision T512), its
  mana full at its class's base mana for its level (100 of 100 without),
  and shows one (`UnitPowerType`, its token the table's). `UnitPower(unit,
  type)`, `UnitPowerMax`, `UnitPowerMissing` and `UnitPowerPercent` answer
  that power, the shown one without a type (or with one not a number); a
  type the unit lacks a plain 0 (recorded: a paladin's
  `UnitPower("player", 1..29)`). A power it has but does not show is as
  secret as the shown one, combo points never (`GetPowerTypeSecrecy(4)`
  answers never, recorded; the answers unverified); `UnitHasPowerType` and
  `C_Secrets.ShouldUnitPowerBeSecret` follow the powers it has. Changing a
  power tells `UNIT_POWER_UPDATE` (a regeneration tick
  `UNIT_POWER_FREQUENT`) and a maximum `UNIT_MAXPOWER`, each with the
  power's token; another power shown, `UNIT_DISPLAYPOWER`. A spell's cost
  and an action's usability read the shown power, an item's mana the mana
  (none for a character without it). Unverified: the answers for powers
  not shown, `UnitPowerType(unit, 1)` of a unit with several powers (none,
  as recorded for one with one).
- **Races and factions.** A unit's faction (`UnitFactionGroup`) follows
  its race's in the flavor table ([Manifest §9.4.1](manifest.md#941-flavor-table);
  decision T513), whatever its data says; a race of either faction (the
  Pandaren, the Skyborne) has the one given (`faction`, `factionGroup`),
  none when none is given; a creature its own. `UnitRace` of a race of
  either answers its faction's race ID and name (camelot's Skyborne: 95
  High Order Skyborne, 96 Windshaper Skyborne; with none, 96). A player
  given no race is a Human, an Orc when given the Horde. The player knows
  its race's racial spells for its faction and class, from the table,
  without learning them (`IsPlayerSpell`, `C_SpellBook.IsSpellKnown`);
  unverified: the recorded spellbook does not list them.
- **The pet's spellbook.** The player's pet (`mock.setPet`) has a
  spellbook, the spellbook's bank `Enum.SpellBookSpellBank.Pet` (1): the
  spells `mock.setPet` gives (`spells`), else those on its bar (none on
  the default hunter bar). `C_SpellBook.HasPetSpells` answers their count
  and the pet's kind, `"PET"` for a hunter's, `"DEMON"` for another's
  (nothing for a pet without spells; without a pet, the recorded answer).
  The spellbook functions asked by slot and bank answer bank 1 from them,
  each spell a pet action: `GetSpellBookItemInfo`'s `itemType` is
  `PetAction` (3), its `spellID` and `actionID` the spell's ID, with its
  `name`, `subName`, `iconID` and `isPassive`, `isOffSpec` false; so do
  `GetSpellBookItemName`, `GetSpellBookItemTexture`,
  `GetSpellBookItemType`, `GetSpellBookItemLink`, `IsSpellBookItemPassive`,
  `GetSpellBookItemAutoCast` and `IsSpellInSpellBook(spellID, 1)`.
  `ToggleSpellBookItemAutoCast(slot, 1)` and
  `SetSpellBookItemAutoCastEnabled` switch a spell's autocast where it
  may autocast, in the book and on the pet's bar together, as
  `TogglePetAutocast` does from the bar, each with `PET_BAR_UPDATE`. The
  two spellbook functions are protected: only Blizzard's code may call
  them (its spellbook's right-click), an addon's call is forbidden, in
  combat or out (decision T600); addons switch autocast with a secure
  right-click on the game's pet bar button (`/click PetActionButton4
  RightButton`), which calls `TogglePetAutocast`, in combat too.
  `PickupSpellBookItem(slot, 1)` puts the
  pet's spell on the cursor (`GetCursorInfo`: `"petaction"`, its ID, its
  slot; placed on an action bar, a spell of subtype `"pet"`).
  `GetPetIcon` answers the pet's `icon`, else its first spell's, nil
  without a pet; `PetHasSpellbook` whether there is a pet. A change of the
  pet's spells is told by `SPELLS_CHANGED`, which Blizzard's spellbook
  listens to (decision T599). Modelled on Blizzard's spellbook code and
  the documentation, all unverified: the token for other pets, the action
  ID (the client may add flags to it), whether the setter is protected
  as the toggle is, the cursor's values and the icon's kind (`tests/ingame/PROBES.md`, "The
  pet's spellbook").
- **Trainers.** A trainer (`mock.defineTrainer`) opened through its
  gossip lists its services; as the list comes its first available
  service is chosen (`GetTrainerSelectionIndex`), with no event.
  `SelectTrainerService(i)` chooses the listed service `i`, any other
  index none, and a change of the choice sends
  `TRAINER_DESCRIPTION_UPDATE()` at once, in which Blizzard's trainer
  frame takes it up; the same choice again sends nothing (decision T617;
  unverified: at once rather than with the server's answer, the choice
  with none available).
- **Beast Training.** In WoW Forever a hunter's Beast Training is a
  spell, not an NPC (reported by the user, 2026-11-10): the player's cast
  of it (5149; inferred, of the client's two spells of that name) with a
  pet out opens the trainer window in its pet mode, as an NPC opens the
  class trainer but without gossip or the interaction manager's events:
  `TRAINER_SHOW()`, then `TRAINER_UPDATE()` with the list
  `mock.defineBeastTraining({ services })` gives (decision T618). While
  it is open `C_Trainer.GetTrainerType()` answers `Enum.TrainerType.Pet`:
  a service's cost is in the pet's training points
  (`C_PetInfo.GetPetTrainingPoints`: all and spent, `mock.setPet`'s
  `trainingPoints` and `trainingPointsUsed`, a change telling
  `UNIT_PET_TRAINING_POINTS("pet")`) and its level the pet's; a service
  the pet's spellbook holds is "used". `BuyTrainerService` from an
  addon's code is forbidden there (`ADDON_ACTION_FORBIDDEN`, seen in
  game: addons train with a secure click on the game's Train button); a
  class trainer's stays allowed. Blizzard's Train, for a service the
  points pay for, teaches it to the pet a round trip later: the chat's
  "Your pet has learned a new ability", the spell in the pet's spellbook
  (in place of one of the same name), its cost spent, `SPELLS_CHANGED`,
  `TRAINER_UPDATE`; `C_SpellBook.IsSpellKnown(id, 1)` is true for it.
  The window hidden (`CloseTrainer`) or the pet dismissed:
  `TRAINER_CLOSED()`. Unverified beyond the spell opening the window, the
  pet mode and the forbidden call (`tests/ingame/PROBES.md`, "Beast
  Training").
- **Click bindings.** The character has the game's click bindings
  profile (`C_ClickBindings`; decision T625): by default two
  interactions (`Enum.ClickBindingType.Interaction`, 3), Target
  (`Enum.ClickBindingInteraction.Target`, 1) on `LeftButton` and the
  unit's menu (`OpenContextMenu`, 2) on `RightButton`, no modifiers, as
  WoW Forever answers. `GetProfileInfo()` answers copies of its entries
  (`{ type, actionID, button, modifiers }`); `SetProfileByInfo(infoVec)`
  replaces it by the list's entries in order, `ResetCurrentProfile()`
  restores the default; neither tells an event (unverified: the
  documentation names none). `GetBindingType(button, modifiers)` is the
  type of the entry with exactly that button and those modifiers, else
  `None` (0); `GetEffectiveInteractionButton(button, modifiers)` is, for
  an interaction there, its default button (Target `LeftButton`, the menu
  `RightButton`: what Blizzard's `SecureUnitButton_OnClick` reads the
  frame's `type` with, as its comment says; unverified for a moved
  interaction), else nil. `ExecuteBinding(unit, button, modifiers)` presses
  a spell entry's spell as the player does (as `UseAction`; unverified: on
  the clicked unit) and runs a macro entry's macro by index; a pet action
  does nothing yet. `MakeModifiers()` is the modifier keys held, each key
  both its bits, its left and its right key's (one Shift held answered 3
  in game): Shift 3, Ctrl 12, Alt 48, Meta 192. `GetStringFromModifiers(n)`
  names each bit as the game does (1 and 2 `SHIFT`, 4 and 8 `CTRL`, 16 and
  32 `ALT`, 64 and 128 `META`, other bits nothing), several joined with
  `-` in a binding's order, `ALT-CTRL-SHIFT-META` (unverified). So
  Blizzard's unit frames act as in the game: a click whose `type` is
  `target`, `menu` or `togglemenu` does nothing where the profile has no
  binding for the click (Shift-Left on a frame with `*type1` `target` by
  default), and a spell or macro binding goes to `ExecuteBinding` before
  the frame's attributes.
- **Mounts, pets and toys on the cursor.** The collections' pickups put
  them on the cursor as the game does (decision T626; probed in WoW
  Forever, `tests/ingame/PROBES.md`, "Mounts, pets and toys dragged"):
  `C_MountJournal.Pickup(displayIndex)` the journal's displayed mount
  there (as `GetDisplayedMountID` sees it), `GetCursorInfo` answering
  `"mount"`, its mount ID and its index in the journal (its place in
  `GetMountIDs()`); `C_PetJournal.PickupPet(petGUID)` the battle pet,
  `"battlepet"` and its GUID (wowlua has no pet journal yet: any GUID is
  taken); `C_ToyBox.PickupToyBoxItem(itemID)` a toy the player has as a
  plain item, `"item"`, its ID and its link. Placed on an action slot
  (`PlaceAction`, a drag onto an action button, a swap through
  `PickupAction`) a mount is a `summonmount` action of its mount ID, a
  pet a `summonpet` action of its GUID (`GetActionInfo`: the type, the
  ID, no subtype), a toy an `item` action; `GetActionTexture` answers
  the mount's icon (`GetMountInfoByID`'s), the pet's
  (`GetPetInfoByPetID`'s, none without a pet journal) and the item's.
  Picked up from the slot they are on the cursor again; `ClearCursor` and
  any other pickup let them go. Using such a slot does what
  `SummonByID`, `SummonPetByGUID` and `UseToy` do: nothing yet.
  Unverified: the cursor's second number for a mount (the probe's mount
  3111 answered 36, no list wowlua knows has it there), an uncollected
  mount or a toy the player lacks (not picked up here), the random
  favourite (`Pickup(0)`, nothing here), `CURSOR_CHANGED`'s types (16
  and 17, `Enum.UICursorType`; a toy's 1, an item's), the grid shown for
  them, `CursorHasItem` for a toy (true here).
- **Group loot rolls.** The server's rolls for the group (decision T629;
  Blizzard's GroupLootFrame and the `CONFIRM_LOOT_ROLL` popup run on them):
  `mock.startLootRoll` starts one, `START_LOOT_ROLL(rollID, rollTime,
  lootHandle)` with the time in milliseconds. Till it ends
  `GetLootRollItemInfo(rollID)` answers the item's texture, name, count
  and quality, Bind on Pickup, `canNeed`, `canGreed`, `canDisenchant`
  (false: WoW Forever has no Disenchant roll), the reasons (a number of
  `LOOT_ROLL_INELIGIBLE_REASON1`..7, 0 when allowed), the Disenchant
  reason and skill (0) and `canTransmog` (false); `GetLootRollItemLink`
  the link, `GetLootRollTimeLeft` the milliseconds left on the clock,
  `C_Loot.GetLootRollDuration` its time, `GetActiveLootRollIDs` the rolls
  the player has not chosen on, `C_TooltipInfo.GetLootRollItem` the item's
  tooltip; a roll not under way answers as the full probe recorded
  (nothing, 0, `{}`, nil). `RollOnLoot(rollID, rollType)` (0 Pass, 1 Need,
  2 Greed; another type, a choice the item does not allow, a roll chosen
  on or ended: nothing) chooses; a Bind on Pickup item's Need or Greed
  first asks, from inside the call: `CONFIRM_LOOT_ROLL(rollID, rollType,
  confirmReason)`, whose popup's OK is `ConfirmLootRoll(rollID,
  rollType)`, the choice. A choice made, the client is done with the roll
  (`CANCEL_LOOT_ROLL(rollID)`, from inside the call, as SuuUI found in
  game) and the server tells it a round trip later (`CHAT_MSG_LOOT`,
  `LOOT_ROLL_NEED_SELF`, `_GREED_SELF`, `_PASSED_SELF`); the other
  members' choices (`mock.lootRollChoice`) are told at once
  (`LOOT_ROLL_NEED`, `_GREED`, `_PASSED`). Everyone chosen, or the time
  out (the player not chosen: `CANCEL_LOOT_ROLL`), the roll ends: Need
  over Greed, each of the best choice's rollers rolls 1 to 100 (the test's
  number, else the engine's own generator; `LOOT_ROLL_ROLLED_NEED` or
  `_GREED` by name, the player's too) and the highest wins, the first to
  choose on a tie (`LOOT_ROLL_WON`, the player's `LOOT_ROLL_YOU_WON` and
  the item into the bags with "You receive loot"); nobody but passes:
  `LOOT_ROLL_ALL_PASSED`. The loot's rolls all ended:
  `LOOT_ROLLS_COMPLETE(lootHandle)`. The lines are the client's global
  strings, their loot history link naming the encounter. The loot history
  (`C_LootHistory`) keeps each roll as a drop of its encounter (the
  test's, the one under way, else 0), numbered from 1: who chose what
  (`Enum.EncounterLootDropRollState`), the numbers, the winner or
  everyone passed, the rollers first by number, then by choice and order;
  `LOOT_HISTORY_UPDATE_ENCOUNTER` when a drop comes,
  `LOOT_HISTORY_UPDATE_DROP` at each choice and the end. Unverified (no
  roll recorded yet; the probes in `tests/ingame/PROBES.md`, "Group loot
  rolls"): the IDs and handles (from 1 here), `confirmReason` (empty
  here), the lines' sender (none), the result's lines' and events' order,
  the player's own roll line (by name), the time out's lines (none for who
  did not choose), what a choice the item does not allow does, the
  Disenchant values, `GetLootRollItemInfo` after the player chose (the
  item's still), the loot history's encounter outside one and its times
  (`GetTime`'s seconds), `LOOT_ROLL_STARTED`'s line and
  `MAIN_SPEC_NEED_ROLL` (neither sent here).
- **Layout.** A region's rectangle comes from its points as the client
  lays it out: each edge from the points that set it (a `LEFT` and a
  `RIGHT` point give the width, a `TOP` and a `BOTTOM` the height),
  otherwise its set size placed by the axis's first point. Of two points
  that set one edge, the one earlier in `Enum.FramePoint`'s order
  (`TOPLEFT`, `TOP`, `TOPRIGHT`, `LEFT`, `CENTER`, `RIGHT`, `BOTTOMLEFT`,
  `BOTTOM`, `BOTTOMRIGHT`) places it, whatever order they were set in,
  and `GetPoint(i)` answers them in that order (decision T337). Points that
  put the right edge left of the left one, or the top below the bottom,
  give no size: 0, at the `LEFT` and `BOTTOM` points (decision T266; seen
  in game). `GetSize`,
  `GetWidth` and `GetHeight` give the laid-out size of an anchored region
  (unverified beyond fresh widgets; `/stdtest engine` records
  `layoutSizes`). A rectangle is laid out when it is asked for (`GetLeft`,
  `GetTop`, `GetRect`, `GetSize` and the other getters of an anchored
  region) and in the layout pass before the next frame's `OnUpdate`
  (the client's at the end of the frame), after the frames it hangs on;
  the pass takes the frames anchored since the last pass first, each
  laying out what it hangs on for itself, as a query would, hidden or
  shown;
  an anchored frame with an `OnSizeChanged` script or hook gets it there
  and then, with its laid-out size, when that size differs from the one
  it was last told, its first rectangle too: an item anchored and asked
  for in an `OnUpdate` runs its `OnSizeChanged` inside that `OnUpdate`,
  before the getter answers; a frame without points gets none. When a
  frame's `OnSizeChanged`, run while a frame anchored to it was being
  laid out, resizes the frame it hangs on, and the frame being laid out
  had no place since it was last anchored, that frame gets none: `GetLeft`
  and `GetRect` answer nothing and its own `OnSizeChanged` never runs,
  until it is anchored again (`SetPoint`, `SetAllPoints`); when that was
  the resizing frame's first rectangle, only until the next layout pass.
  A resize the handler leaves for later (a timer) loses nothing, nor does
  a fit run before the frame is asked for, nor a frame laid out for
  itself (its own `GetSize` in a script, or asked before the frame
  anchored to it). Handlers nest ten deep at most; deeper, a size waits.
  `IsRectValid()` answers whether an anchored region was laid out since
  it was last anchored and has a place (UIParent's always) (decision
  T605; SuuUI's `anchors` probes and `/stdtest engine owed` on WoW
  Forever 70862, recorded in `tests/ingame/PROBES.md`; unverified: the
  bound).
- **Screen and text.** The client draws on a screen (decision T237, a
  test setting; a world profile's `screen` and the game's settings while
  it runs, decision T482: the window's size, "Use UI Scale" and its slider,
  told by `DISPLAY_SIZE_CHANGED` and `UI_SCALE_CHANGED`): UIParent's size
  and scale and the screen functions follow it, every region's effective scale includes UIParent's (unless it ignores
  its parents' scale), WorldFrame covers the screen unscaled, 768 units
  high at any resolution and UI scale, its width following the screen's
  aspect ratio (decision T492), and text is measured from the client's recorded
  glyphs at the pixel size the client renders at that scale, so string
  widths, wrapping and truncation are the client's. Lines break as the
  client breaks them (decision T282, recorded by the wrap probe): at the
  end of a space run or after a hyphen, a line as long as its advances fit
  the width; a word wider than the line is cut between characters (at
  least one a line), and without `SetNonSpaceWrap(true)` the rest of the
  text then fills the lines character by character and the string is
  truncated. A font string without a width of its own (none set, not
  two points across) is as wide as its widest line and is not wrapped at
  that width, though a line's advances may pass it (decision T621).
  `GetNumLines` counts every line the text breaks into, also
  with word wrap off or beyond `SetMaxLines`; the height is the lines
  shown; held to fewer lines, the last shown line ends in an ellipsis (one
  glyph three periods wide) and `GetStringWidth` is the shown text's;
  `IsTruncated` says whether lines are left out or a word was cut. The
  client's advances are finer than the recorded whole pixels, so a narrow
  line now and then holds a glyph fewer than in the client. The recordings
  cover Friz Quadrata, Arial Narrow, Skurri and Morpheus at 8 to 45 pixels,
  printable ASCII; with the user's installation at hand (tests with
  `--game-data`, sessions with a screenshot host), a glyph they lack (another
  of the client's fonts, another character, another size) is measured with
  the client's font file (decision T392): its advance rounded up, its width
  alone (a line's last glyph) its ink's right edge rounded up and one more,
  a space's its advance; checked against the recordings, 98.6 % of the
  advances and 95 % of the widths are the same. Without it, such a glyph
  is the recorded Friz Quadrata's (scaled to the size), a character outside
  ASCII a `?`'s. A font file of an addon's folder
  (`Interface\AddOns\<Name>\…`: the addon under test's, a `test.addons`
  one's) is measured with that file by the same rule, with or without the
  installation, before the recordings (decision T619; SuuUI's probe on WoW
  Forever, Figtree Medium at 13 pixels: the game's width, height and lines
  to the pixel), and drawn where it measures, so its words draw without
  gaps; a character the file lacks is measured as above. Unverified:
  kerning beyond the recorded pairs (none; an addon's font none), an
  outlined font's glyphs (as the plain one's).
- **Atlases.** Every atlas of the client, from its own tables (decision
  T238): sizes, files, texture coordinates and tiling as the client gives
  them; an atlas it lacks is none.
- **Textures' files.** A texture shows one image: a color, a file or an
  atlas, each set replacing the others (decisions T352, T356). It tells its
  file by ID, as the client does: `GetTexture()` and `GetTextureFileID()`
  give a file ID set, a path's file as `GetFileIDFromPath` gives it (a game
  file's own ID where the installation is at hand, an addon's file its
  loose file's, nil for a file there is not), an atlas's file, else nil;
  `GetTextureFilePath()` the path set (nil for an addon's file that is not
  there), an atlas's file ID. A path named `.tga`, `.png` or `.jpg` that
  the storage lacks is its `.blp` (decision T350). A loose file's ID set
  (`b:SetTexture(a:GetTexture())`) shows that file. `SetTexture` fails
  (false) for `""` and file ID 0. Without the installation a game file's ID is not known, and
  a path to one answers a stand-in: a negative number, the same for the
  same path.
- **Widget state.** A method with no result changes what the widget's
  getters answer as the client's does (decision T289): `Raise()` and
  `Lower()` put a frame above or below every other shown frame of its
  strata (`GetFrameLevel`), and a top-level frame (`SetToplevel(true)`)
  shown or clicked is drawn, with what is in it, above the rest of its
  strata, its level kept (decision T586); an edit box keeps a selection
  (`HighlightText`, `ClearHighlightText`; an end before the start selects
  to the text's end) that `Insert(text)` and the
  user's typing (`mock.type`) replace, else they go in at the cursor, and
  `SetText` puts the cursor at the end, as does the user's click or typing
  into a box without the focus (before its `OnEditFocusGained`);
  `Insert`'s `OnTextChanged` comes in the frame's update after, as
  `SetText`'s, but telling user input (`true`);
  `AdjustPointsOffset(x, y)` moves every anchor, `ClearPointsOffset()`
  zeroes them; the clears (a button's `ClearNormalTexture`
  and the other states', `FontString:ClearText`, `ClearParentKey`,
  `ResetTexCoord`, `StopAnimating`, `ClearAttributes`, `ClearScripts`,
  `RemoveAnimations`, `ClearModel`, after which `GetModelFileID` is nil)
  undo their setters, but a button's cleared state
  texture stays its own, emptied and shown (`GetNormalTexture` answers
  it: Forever 70707's item buttons mask it after their `OnLoad`, in which
  the professions' recraft slot clears it; decision T576, recorded on
  70862 with the rest of T289's steps, 2026-11-09); `LockHighlight` is
  `SetHighlightLocked(true)`. The methods of what the engine draws or keeps
  nothing of (model cameras and lights, the browser, the shop's checkout,
  housing's pins, the minimap's blips, movies, vector graphics) change
  nothing, and their getters answer the documented empty values instead of
  "not mocked" (decision T295). An edit box keeps
  the lines `AddHistoryLine` adds (as many as `SetHistoryLines` allows,
  `ClearHistory` forgets them); the arrow keys run `OnArrowPressed`, left
  and right move the cursor, up and down step through the history (with
  `SetAltArrowKeyMode(true)` only while Alt is held; decision T291).
- **Clicks.** `Button:Click([button [, isDown]])` runs the button's
  `PreClick`, `OnClick` and `PostClick` with `button` (`"LeftButton"`)
  and `isDown` (false), whatever it registered for, without `OnMouseDown`
  or `OnMouseUp`. On a disabled button it does nothing, so a secure
  button's `click` action on it does nothing either, and neither does a
  button's `Click()` while its own runs (decision T616). The user's click
  (`mock.click`) and a `CLICK` key binding run no click script on a
  disabled button; unverified: the mouse's `OnMouseDown`, `OnMouseUp`,
  `OnEnter` and `OnLeave` on it, which still run.
- **Animations.** An animation group plays on the clock, as the game's
  follow real time (decision T623): a playing group's time is the
  clock's since its `Play`, less the time it was paused (`Pause` stops
  it where it is, `Play` resumes it from there), so time
  `mock.advanceTime` passes plays it on as one long frame would, and the
  time before its `Play` is not its own. Groups move at each frame, after
  its `OnUpdate`s: one whose time passed its length runs `OnFinished`
  then, or, repeating, `OnLoop` once and goes on where the clock has it
  in its next repeat. Between frames, `GetElapsed` and `GetProgress` (the
  group's and its animations') and the scene (§18.9) read it at the
  clock's time now, held at its end until the frame that finishes it.
  `Restart`, `Stop` and `Finish` start it again, stop it and finish it
  at once. Unverified (`tests/ingame/PROBES.md`, "Animations after a
  hitch"): that a frame after a hitch jumps a group ahead by the whole
  time, and that a repeat spanning several lengths runs `OnLoop` once.
- **Targeting.** Tab-targeting cycles the units of its kind within 40
  yards, nearest first (`TargetNearestEnemy([reverse])` and its kin);
  `TargetLastTarget`, `AssistUnit` and `FocusUnit` follow the units'
  targets; `CancelUnitBuff` and `CancelSpellByName` remove the player's
  own buff (decision T291). They are protected: an addon's call is
  refused first.
- **Leftovers.** `StartAttack` and `AttackTarget` turn auto attack on
  against a target that can be attacked, `StopAttack` off;
  `SetPartyAssignment` and `ClearPartyAssignment` keep the raid's main
  tanks and assists for `GetPartyAssignment`; `SendSystemMessage` prints a
  system line; `PickupPlayerMoney`, `DropCursorMoney` and `GetCursorMoney`
  hold money on the cursor (`ClearCursor` lets it go); `TargetLastEnemy`
  and `TargetLastFriend` retarget; `Sign` and `secretunwrap` answer as the
  client's. The client's debug switches and the systems the engine does not
  model answer their documented empty values (decision T295). Every
  function the camelot client has now has behaviour.
- **Not mocked.** A generated function without behaviour raises
  "`<name>` is not mocked" when called, unless the project provides a mock
  (Tooling §16.1.4), so gaps are visible instead of silently returning nil.
- **Engine-side Lua.** Lua the client defines in its engine (not FrameXML) is
  provided as secure code.
- **What addons use of Blizzard's UI code.** Until Blizzard's UI code loads
  (without `test.blizzard-ui`, never), the engine has the parts of it that
  addons and wowlua's standard library use as the client's:
  `SecondsToTime`, `SecondsToTimeAbbrev`, `SecondsToClock`,
  `FormatShortDate`, `GetMoneyString` and `FormatLargeNumber`, as
  Blizzard's `TimeUtil.lua` and `FormattingUtil.lua` write them (from the
  global strings they read, the client's enUS text where those are not
  loaded); `SendChatMessage`, `DoEmote` and `CancelEmote`, which call the
  `C_ChatInfo` functions as Blizzard's deprecation file does; the coins'
  constants (`COPPER_PER_SILVER`, `SILVER_PER_GOLD`, `COPPER_PER_GOLD`)
  and global strings (`GOLD_AMOUNT`, `SILVER_AMOUNT`, `COPPER_AMOUNT`);
  and the container constants Blizzard's `Constants.lua` makes from
  `Constants.InventoryConstants` (`NUM_BAG_SLOTS`, `NUM_REAGENTBAG_SLOTS`,
  `NUM_TOTAL_EQUIPPED_BAG_SLOTS`, …) where the flavor's `Constants` has
  their values (camelot's are recorded). Blizzard's code defines its own
  in their place as it loads.

## 17.7 Blizzard's UI code

With `test.blizzard-ui` ([Manifest §9.18](manifest.md#918-test)), the test
runner loads the flavor's FrameXML (Lua and XML from wow-ui-source, at the
client version the API declarations come from) as secure code before the
addon. Tests then run against the real Edit Mode, Settings, action bars, unit
frames and the rest, so taint the addon spreads into Blizzard's code, and
Blizzard code that fails because of what an addon did, show up as in game.
Off by default: mock-only tests start faster (decision T186).

Where the flavor's client is installed, the code and the global strings are
the installed build's own (decision T553), read once per build into the user
cache (`$WOWLUA_HOME/cache/blizzard-ui/<flavor>-installed-<version>/`). The
client names its interface files by hashes only, so they are found as the
client finds them: from Blizzard's addons' names (wow-ui-source's, at the
commit the declarations name), each addon's TOC (the client's suffixes
first), the files it lists with `[Game]` and `[Family]` as the flavor's, its
key bindings (`Bindings_<suffix>.xml`, `Bindings.xml`), the files its XML
includes, and the addons it depends on or loads (`LoadAddOn("…")`). The global
strings are the client's `GlobalStrings` table's that are set in the game,
the escapes the table writes them with (`\n`, `\32`, `\\`) read as in Lua
(decision T576).
`WOWLUA_UI_FROM=upstream` leaves the installation out.

Elsewhere (CI), the code comes from wow-ui-source at the flavor's client
version and the global strings from the same dump the declarations use,
downloaded on first use into a user cache keyed by version (decision T204; about 10 seconds and
45 MB per flavor, once: `$WOWLUA_HOME/cache/blizzard-ui/`, at the commit the
flavor's declarations name; `WOWLUA_UI_SOURCE` names a local copy instead), for
each flavor that has test files to run (`build.flavor-files` may give them to
other flavors). The dump can be older than the UI code: the strings an
installed client of the declarations' build has and the dump lacks are kept
with the engine (`camelot-newer-strings.txt`) and set after the dump, where
the global is still nil (decision T590). The engine's loader (`wowlua_engine::ui::load`)
takes the flavor's `Interface` folder and its global strings:

- **Startup set** (as `/stdtest engine` recorded: the 201 Blizzard addons
  WoW Forever had loaded when StdTest loaded): every Blizzard addon allowed
  in the game (`AllowLoad` game or both) and for the flavor's game types
  (its id and family, for `AllowLoadGameType` and `ExcludeLoadGameType`),
  read from the client's suffixed TOC first (`_Mainline.toc`); a
  `LoadOnDemand` one only as a startup addon's dependency; `OnlyBetaAndPTR`
  ones on beta and PTR clients (WoW Forever's beta ran from `_classic_beta_`).
- **Load order** (inferred, unverified): `LoadFirst` ones first, each after
  its `Dep`, `Dependencies`, `RequiredDep` and `OptionalDep` addons,
  otherwise by name; a startup addon with `LoadWith` right after the addons
  it names (Classic's Blizzard_UIParentPanelManager after Blizzard_UIParent,
  before the `LoadFirst` addons that use its `UIPanelWindows`; decision
  T554).
  File lines follow their inline directives; `[Family]`, `[Game]` and
  `[TextLocale]` name the flavor's folders (`Mainline`, `Camelot`, `enUS`).
  `ADDON_LOADED` fires after each addon.
- **Global strings** come first, from the client's `GlobalStrings` dump.
- **Lua files** run as secure code with the addon's name and table, also
  when an addon's code loads a load-on-demand one (`C_AddOns.LoadAddOn`)
  while the addon loads: Blizzard's files and scripts compile as Blizzard's
  code whoever runs when they load, and keep their own file names. A file's
  result is its module: `import(".Dir.Name")` gives the result of
  `Dir/Name.lua` next to the importing file, `import("AddOn.Dir.Name")` that
  of `Dir/Name.lua` in the addon `AddOn`'s folder (decision T576).
- **XML** builds widgets as the client does: virtual elements are templates
  (for `inherits` and `CreateFrame`), intrinsic ones frame types; an
  element's templates apply first, then its attributes, mixins, `parentKey`
  and `parentArray`, key values, sizes, anchors (an element's own
  `<Anchors>` take the place of its `setAllPoints`, decision T323; a
  texture with none fills its parent, sized or not, but a slider's thumb,
  which the slider anchors by its `LEFT` to its own `LEFT` (`TOP` to `TOP`
  on a vertical slider), offset along its length by its value, so
  `GetNumPoints` gives 1 and `GetPoint` that anchor, decision T326; a
  font string with none fills its parent too, but a `ButtonText`, until
  its first `SetPoint`, which replaces the fill instead of adding to it:
  by one point it is its text's size, decision T602; a
  texture's `rotation` is in degrees, turning it about its center), layers,
  child frames and scripts (`function`, `method` or a body;
  `inherit="prepend"` / `"append"`; a texture's `<Color>` is its plain
  color without a file and its tint with one, `clipChildren` clips,
  decision T339);
  `OnLoad` runs once the widget and its children are built. Fonts and font
  families become font objects.
- **Secure APIs for addons** (Blizzard_AuraContainer's custom containers):
  a type or template declared under `<ScopedModifier
  allowUntaintedCreation="true">` is built untainted for any caller of
  `CreateFrame`: its mixins, its scripts (compiled as Blizzard's) and its
  `OnLoad`, also when an addon creates it while the addon loads (an
  intrinsic type's scripts, like a template's, are the code of the addon
  that declared it, never the caller's). A `<Mixin secureDelegates="true">` puts secure delegates on the
  frame: they call the mixin's function untainted, with the arguments as
  untainted values, and give the caller's taint back after. A tainted
  caller passing a secret gets no elevation from a mixin's delegates
  (Blizzard's ScriptErrorsFrame says so), but does from
  `CreateSecureDelegate`'s. Partitions are not modelled: an object is its
  own `GetForbiddenObjectTable` (secure code only), and the delegate stays
  on the frame where a later private mixin has the same function.
  Unverified: errors in a delegate reach its caller.
- **Macros** (decision T601): `C_Macro.RunMacroText(text, button)` (a
  secure button's `macrotext`) and `RunMacro(index or name, button)` (its
  `macro`, a saved macro's body) run the text line by line through the
  function Blizzard's UI gives `C_Macro.SetMacroExecuteLineCallback` at
  `PLAYER_ENTERING_WORLD` (its private chat edit box's `SendText`), as the
  client does: each line as typed into chat, so the client's secure
  commands run as Blizzard's code has them. `/click <name> [mouse button]
  [down]` clicks the named button with that button (`LeftButton` without),
  in combat too; a frame that is no button, or a name nothing has, gets
  nothing (the access constraints and forbidden aspects the handler checks
  are none on the engine's widgets). Empty lines and `#` lines
  (`#showtooltip`) run nothing (unverified); `/stopmacro` (`StopMacro`)
  stops the rest. `[button]` sees the click's button. Without Blizzard's UI
  no callback is set and a macro runs nothing. `C_Macro.RunMacroText` is
  protected, as the global `RunMacroText` is (the documentation's
  `HasRestrictions`; unverified).
- **Gaps.** Without game data or a recording, Blizzard's code gets empty
  values from functions that have no behaviour: secure code calling such a
  function gets its documented results' empty values (nil where nilable,
  false, 0, "", a table) and the engine lists the function; addon code still
  gets "not mocked". Secure code may register events and set scripts the
  engine's lists lack. On camelot (70425) the UI loads, logs in and runs
  frames without a Lua error, as the client's does (its probe logged none):
  enums, constants and namespaces the documentation lacks come from the
  client's recording (`/stdtest engine`), secure addons run in the client's
  secure environment, and Blizzard's errors are caught where ScriptErrors
  reports them (`C_Log.LogErrorMessage`). The CVars start at the client's defaults (the 248 the UI code reads that
  have one, recorded by `/stdtest engine`). The other flavors' clients are
  not recorded yet: their UI code meets enums, colors and constants the
  documentation lacks and raises errors while it loads, so for now
  `test.blizzard-ui` is for camelot, and wowlua's example addons run
  their tests on camelot alone (their manifests' `flavor-files` give
  `tests/**` to camelot) until standard's and vanilla's are recorded.
- **The client's cases.** With camelot's UI code loaded (and the addon's
  saved variables loaded as the addon's), all 819 cases `/stdtest probe`
  recorded on WoW Forever 1.60.1 give the client's results, the 22 the VM
  alone cannot pass included (Blizzard's `string.rtgsub`, `getglobal`,
  the string metatable `RestrictedExecution.lua` sets up).

## 17.8 Verification

- **Lua 5.1 conformance.** Against PUC Lua 5.1 (embedded only in wowlua's own
  tests):
  - `cargo xtask corpus vm` compiles every file of the conformance corpus to
    the same bytecode and line info as `luac`, and runs its main chunk with
    the same output and error.
  - `cargo xtask corpus lua51` runs the official Lua 5.1 test suite. It
    excludes only what the client does not have, each with its reason: the
    `io`, `os`, `debug` and `package` libraries, `string.dump`, the implicit
    `arg` of vararg functions, `string.gfind`, and a 32-bit `size_t`
    overflow. Shims serve the suite's plumbing only (timing, temporary files
    in memory, loading its helper files).
  - Differential tests compare snippets' output and errors.
- **Sources of truth.** In-game probes on the current client first, then
  Blizzard's documentation and UI code for the build (the generated API
  documentation's `SecretArguments`, `ProtectedFunction`, `HasRestrictions`,
  `SecureHooksAllowed` and `SecretWhen…` flags, `FrameScriptDocumentation`).
  elune (an open reimplementation of an older client) is only a hint: a rule
  that rests on it alone is listed as unverified in the coverage report.
- **Taint.** StdTest's taint experiments (`/stdtest taint`) taint one value on
  a path the client's secure code takes to a protected function and record
  whether the action is blocked; the VM must agree. On WoW Forever 1.60.1
  (`tests/client/forever-camelot-taint.lua`): secure code reading a tainted
  global (Lua or C function) or table field is tainted, and the protected
  call raises `ADDON_ACTION_FORBIDDEN` naming the addon; `hooksecurefunc`
  leaves the hooked variable secure and the click allowed; `setfenv` taints
  the function itself (its variable stays secure, the click is forbidden).
  With camelot's UI code loaded, the engine reproduces all eight: the
  action button's click reaches `UseAction` through Blizzard's own
  `SecureTemplates.lua` and is forbidden exactly where the client forbade it
  (`wowlua-engine`'s `tests/taint.rs`). elune's taint cases
  (vendored in `crates/wowlua-vm/tests/elune/`) also pass on the VM, but count
  only where the experiments confirm them.
- **The client's behaviour.** Every WoW-specific rule (§17.3 to §17.5) has a
  probe in StdTest whose recorded in-game result is a test fixture; a rule
  changes only with a new probe.
- **In-game regressions.** Bugs found in game become emulation tests that must
  fail the same way: `os.date` in `wowlua:format`, `0 / 0` in
  `wowlua:serialize`, SuuUI's reparented status-bar containers breaking Edit
  Mode, and taint met in combat.
- **Coverage of the emulation.** A report lists which API functions have
  behaviour, which are generated only, and which rules were probed on which
  flavor: [`docs/emulator-coverage.md`](../../docs/emulator-coverage.md),
  written by `cargo xtask coverage-report` from the engine's own tables, the
  fixtures and the replay tests' ratchets; `cargo xtask check` fails when it
  is out of date.

## 17.9 VM design

- **Execution** (decision T190). The VM compiles the compiler's output to a
  register bytecode of its own, close to Lua 5.1's design. Each instruction
  carries its source span (from the source maps), and coverage, taint and
  profiling attach per instruction.
- **Memory** (decision T191). A tracing garbage collector over an arena, in
  safe Rust, stopping the world at the interpreter's allocation points:
  - Cycles are collected and weak tables clear.
  - `__gc` runs as in the client: newest first and unprotected, so an error
    in a finalizer reaches the code that caused the collection.
  - `collectgarbage` reports memory, and `step` counts work, as Lua 5.1's
    incremental collector would.
  - `collectgarbage("count")` is the whole Lua state's memory (Blizzard's UI
    and every addon), and `GetAddOnMemoryUsage(addon)` (by name or index)
    an addon's as `UpdateAddOnMemoryUsage()` last measured it, 0 before
    (decision T610): the live objects its code made. Each object belongs to
    the taint it took when made (`luaR_taintalloc`): an addon's while its
    code runs, the game's functions it calls included, so the frames,
    tables and strings `CreateFrame` makes for it are its; secure code's
    objects are no addon's. A test's own code is the addon's (it runs with
    the addon's taint), so a test counts toward it too. Sizes are the
    collector's estimates (a table 64 bytes and its slots, a string 32 and
    its length), the same on every run.
- **Taint** (decision T193). Exact, as the client tracks it: every stack
  slot, table field, upvalue and object carries the taint (the addon) of the
  code that wrote it, and each thread its execution's taint (§17.4).
- **Coroutines** (decision T192). Stackless: each coroutine is its own VM
  stack, and a yield unwinds to its `resume` without the native stack. As in
  the client's Lua 5.1, a yield cannot cross `pcall`, a metamethod or another
  native call ("attempt to yield across metamethod/C-call boundary"); each
  thread has its own global table (`setfenv(0, t)`).
