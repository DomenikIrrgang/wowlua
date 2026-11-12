# In-game probes to run

The probes that would close the emulator's open gaps
([coverage report](../../docs/emulator-coverage.md)), for WoW Forever (the
camelot flavor). Install a current StdTest build first (`wowlua install
--client _forever_` in `tests/ingame`), then `/reload` in game so it
loads.

Everything here is read-only: no item is used, nothing is bought, moved or
sent, the character does not move, and no output stores a player's name.
After a run that StdTest records, `/reload` writes the results to the saved
variables; `cargo xtask ingest` then takes them into the fixtures.

## Batched (2026-11-09)

`/stdtest engine owed` runs the owed steps below that are pure code, by
itself, in one go (`src/owedsteps.lua` holds them, each the item's `/run`
line with the same calls and IDs). Every step's printed values land in
`StdTestState.engineOwed.results` under a key naming the section and step
(`M12.3.2`: "Owed from the M12 screenshots", item 3, step 2), as `print`
shows them (a secret as `secret`, an error as `error: <text>`, a `/dump`'s
tables flattened); `order` has every line in the order printed, `times`
when each step ran. One step per frame, as lines typed one after another;
a step's "about N seconds later" waits that long. What the steps make is
hidden after. These items are covered: do not run them by hand again.

**Ran on 2026-11-09** (WoW Forever 1.60.1.70862, a level 36 paladin with
AtlasLoot and SuuUI): the answers, compared with the emulator's run of the
same steps, are in "Answered by the batched run (2026-11-09)" below; the
items they answered left the owed sections, and what stays open is marked
in its section. Run it again only for new steps.

- **Owed from the M12 screenshots**: 1 (both steps, and step 1 again after
  step 2; step 2 loads `AtlasLootClassic_Crafting` if it is installed, a
  load-on-demand addon, else the client answers for a missing one), 2, 3,
  4, 7, 8 (steps 1 and 2: the widths and heights), 11, 14, 16 (steps 1 and
  2: the size, and again a frame later), 17, 18 step 2 (a title that is not
  a holiday's as its length only; again 2 s after `C_Calendar.OpenCalendar()`),
  19 steps 7 and 9, 20d steps 1 and 2 (and step 3's line without the
  character frame open), 20e step 3 (and `select("#", BNGetInfo())`, the
  count only).
- **Owed from M13**: 1 (the widths, and again a frame later), 2 (with the
  character's class and level: it means something on a paladin with spell
  power), 3 (the dumps).
- **Owed from M14**: 1 (the three lines in the order printed; the
  settings panel then closed with `HideUIPanel(SettingsPanel)`).
- **Owed from the contributions in 0.13.5**: 1, 2, 4, 6 (timed: 3 s, 0.5 s,
  2 s and 3 s apart; a white bar shows in the middle for about 8 s).
- **Still to run**: the `GetBindingText(key, 1)` bullet (each key with `1`,
  `true` and no second argument; the keys and modifiers alone too).
- **Decision T605's open questions** (new; keys `T605.<case>`; SuuUI's
  `anchors3` case G as in "Found with SuuUI" item 1, each case alone, a
  second apart; the item's left and top, top from UIParent's: `112,-140`
  placed, `nil` not):
  - `G`, `I`, `M`: the recorded cases again. wowlua: G `place | now nil |
    next nil | 1s nil`; I and M `place | size 55x30 | now 112,-140 | next
    112,-140 | 1s 112,-140`.
  - `Gq`, the pass order: G with nothing asking for a rectangle before the
    frame's end (no "now"). wowlua: `place | next nil | 1s nil`.
  - `Gh`, a panel built hidden: G built with the panel hidden, placed (and
    asked "now") hidden, shown a frame later and asked at once ("shown").
    wowlua: `place | now nil | show | shown nil | next nil | 1s nil`.
  - `Gh2`: as `Gh` without "now" and "shown". wowlua: `place | size 55x30
    | show | next 112,-140 | 1s 112,-140`.
  - `T`: the body's `OnSizeChanged` asks for its own size and resizes
    nothing. wowlua: `body asks 1 | place | body asks 30 | size 55x30 | now
    112,-140 | next 112,-140 | 1s 112,-140`.
  - `W`, a frame laid out for itself: G with `body:GetSize()` asked right
    after the body's height is set, before the item is asked for. wowlua:
    `place | body 30 | size 55x30 | now 112,-140 | next 112,-140 | 1s
    112,-140`.
- **Other sections**: Widget state (T289) 1 to 7 and 11 to 14; Targeting
  and history (T291) 3; Leftovers (T295) 4 (a system line "probe line"
  shows); Model frames' cameras 3; Textures the screenshots draw 1 and 4;
  Unanchored textures 1 (its Lua half); The Camelot update 5 and 6 (its
  print); Unanchored font strings (T602) 1; Names over units 1; How far
  the world is drawn 1; Found with SuuUI 2 (SuuUI's font, if installed), 7
  and 9.

The steps:

1. On the desktop, in `tests/ingame`: `wowlua install --client _forever_`
   (done 2026-11-09; again only after StdTest changes).
2. In game, out of combat, the settings closed, standing anywhere: `/reload`
3. `/stdtest engine owed`
4. Do not type or click until the chat line `engine owed: N answers
   recorded (M with an error); type /reload so the file is written` (about
   25 seconds). A white bar, a short text, a tooltip and the Settings
   window show in the middle of the screen for a moment and go, and a
   system line "probe line" comes in the chat.
5. `/reload`
6. On the desktop: keep a copy of
   `WTF/Account/<account>/SavedVariables/StdTest.lua`; its
   `StdTestState.engineOwed` holds the answers (not ingested: read it).

**A mailbox walked away from** (recorded 2026-11-09 19:17, 70862; decision
T609). Recorded: the interaction open at the start (MailFrame shown),
`PLAYER_STARTED_MOVING` at 1.684 s, and at 2.240 s, 3.8 yards from where
the player stood (2.5 at the position before, 2.052 s, still open):
`SECURE_TRANSFER_CANCEL` twice, then
`PLAYER_INTERACTION_MANAGER_FRAME_HIDE(17)` twice,
`IsInteractingWithNpcOfType(17)` true at the first hide and false at the
second; no `MAIL_CLOSED`, no hide of type 0, nothing else of the mail. The
recorder has one frame for all events (`RegisterAllEvents`, one handler),
so the game tells each twice. wowlua (before): open at any distance; now
it ends past 5 yards (gossip's range: the mailbox's own place is not in
the recording) with these events. Still open: the range. The steps, for a
second recording with the mailbox's place (step 1 below):

SuuUI found that the game ends a mailbox interaction when you walk away;
wowlua kept it open at any distance. The scenario recorder records every event with its time
(`MAIL_CLOSED`, `PLAYER_INTERACTION_MANAGER_FRAME_HIDE` with its type,
`SECURE_TRANSFER_CANCEL`, …); for `mailwalk` it also records the player's
position and whether the mailbox interaction is on, at the start, at those
events and every quarter second the player moved half a yard. Nothing is
read from the mail.

1. Walk into the mailbox until you stand against it (W pressed into it),
   then `/run print(UnitPosition("player"))` and note the two numbers.
2. Step back a yard, out of combat, and right-click the mailbox so the
   mail window opens.
3. `/stdtest engine scenario mailwalk`
4. Walk straight away from the mailbox (W), not closing the mail window,
   until it closes by itself, then two more seconds. If it is still open
   60 yards away, stop there.
5. `/stdtest engine scenario stop`
6. `/reload`

Still by hand (screenshots, other players, combat, walking, a window
opened by hand): Owed from the M12 screenshots 5, 6, 8 step 3, 9, 10, 12,
13, 15 (a blocked action: `/reload` after), 16 step 3, 18 step 1, 19
steps 1 to 6 and 8, 20a, 20b, 20c, 20d step 3 (the character frame open),
20e steps 1 and 2 (at login), 20; Owed from M13 1's screenshots; Owed from
the contributions in 0.13.5 3, 5, 7; and the other sections' steps not
named above.

## Still to run (2026-10-21, postponed)

The beta's last day left these open, most needed first. Each gives data the
engine cannot get otherwise.

1. **`/stdtest engine screen` with "Use UI Scale" on and the slider all the
   way left**, then `/reload`. Why: the scale rule with "Use UI Scale" on
   is recorded at 1.15 only (UIParent's scale is the slider's); its lower
   bound is not. That run was blocked last time: note the blocked
   function's name in the message ("StdTest has been blocked from an
   action..."): it is a taint finding (StdTest's code tainting Blizzard's
   settings code), for the engine's taint emulation too.
2. **A fresh Human Warrior's `/stdtest snapshot`**, twice a few seconds
   apart, then `/reload`. Why: the engine's default world is that
   character; the new snapshot also records its mounts, toys and wardrobe
   (decision T235).
3. **The replay scenarios** with a fresh character (step by step in the
   session notes): a quest turned in for an item reward
   (`/stdtest engine scenario turnin`), a second level-up (`levelup`),
   Heroic Strike with a target change (`heroicstrike`), smelting bronze
   (`smelting`, needs Mining); each `/stdtest engine scenario stop` then
   `/reload`. Why: see "Replay leftovers" below.
4. **The SuuUI Escape check** (by hand): see "Regressions" below.
5. **`/stdtest engine journals`** on a character with a mount or a toy.
   Why: the events after a collected mount's or toy's favourite changes are
   unrecorded (the recording character had none; decision T235).
6. **How the client breaks lines**: done 2026-10-24 (the beta's last
   night): `tests/client/forever-camelot-wrap.lua` in wowlua-engine, with a
   `/run` of line counts at widths 6 to 70; the engine follows it (decision
   T282). Left open: why the client keeps some texts on one line below two
   glyphs' width, and one line count without word wrap.
7. **An early landing** (the scenario recorder as for the taxi): at a
   flight master, pick a route with a stop on the way; type
   `/stdtest engine scenario earlylanding`, take the flight, and before the
   first stop `/run TaxiRequestEarlyLanding()`; once landed,
   `/stdtest engine scenario stop` and `/reload`. Why: the engine lands the
   flight a server round trip after the request (decision T289, a guess:
   it keeps no stops on the way); the recording gives when it lands and
   which events come.
8. **Atlases on a 1x screen**: the answers battery's atlas checks at a
   1920x1080 screen with Windows' DPI scale at 100%. Why: both recordings
   were on high-DPI screens and gave the 2x textures' files and
   coordinates (decision T238); a 1x screen may take the 1x textures (the
   sizes are the same either way).
9. **Talents the engine doesn't know yet**: partly done 2026-10-24
   (`tests/client/forever-camelot-talents2.lua` in wowlua-engine: the spec
   groups' configs, the active config, another class's view loadout). Left
   open: a rank planned and taken back (`PurchaseRank` refused, the
   character had no unspent point) and `/stdtest engine talentwatch` (a
   level-up to 5).
10. **Addon messages restricted?** In a raid out of an instance and out of
   combat: `/dump C_ChatInfo.AreOutgoingAddonChatMessagesRestricted(),
   C_ChatInfo.InChatMessagingLockdown()`, then
   `/dump C_ChatInfo.SendAddonMessage("SuuUI", "x", "RAID")` and whether a
   raid member's SuuUI hears it. Why: the full probe recorded `true` for
   the first (alone, out of combat), so the engine answers `true` always;
   SuuUI's raid planner no longer reads it (its specs were never shared).
11. **Moving a raid member**: as leader, `SetRaidSubgroup(i, g)`; then
   `GetRaidRosterInfo(i)` at once and after `GROUP_ROSTER_UPDATE` (with
   `GetTime()` at each). Why: SuuUI's probe saw the update 0.4-0.7 s
   later; the engine moves them and fires it at once.
12. **The game's settings while the UI runs** (decision T482: the
    events' order, whether a window size that changes UIParent's scale
    tells both, the CVars). In windowed mode, out of combat:
    1. `/run W={} F=CreateFrame("Frame") for _,e in ipairs({"UI_SCALE_CHANGED","DISPLAY_SIZE_CHANGED","CVAR_UPDATE"}) do F:RegisterEvent(e) end F:SetScript("OnEvent",function(_,e,a,b) W[#W+1]=e..(a and (":"..tostring(a)..":"..tostring(b)) or "") end)`
    2. Esc → Options → Graphics: tick "Use UI Scale", Apply.
    3. `/run print(table.concat(W," | ")) print(UIParent:GetScale(), GetCVar("useUiScale"), GetCVar("uiScale")) W={}`
    4. Move the UI Scale slider to its middle, Apply.
    5. `/run print(table.concat(W," | ")) print(UIParent:GetScale(), GetCVar("uiScale")) W={}`
    6. Pick another Window Size, Apply.
    7. `/run print(table.concat(W," | ")) local s=C_VideoOptions.GetCurrentGameWindowSize(0,false) print(s.x, s.y, GetCVar("gxWindowedResolution"), UIParent:GetScale(), GetScreenWidth(), GetScreenHeight()) W={}`
    8. `/run C_VideoOptions.SetGameWindowSize(1000, 700)`, then step 7 again (a size the monitor does not list).
    9. Untick "Use UI Scale", set the Window Size back, Apply.

(The `GetBindingText(key, 1)` bullet was answered by the batched run,
2026-11-09: "Answered by the batched run".)

## The answers battery's new calls

About a minute, standing anywhere, out of a group and out of combat.

1. Log in (or `/reload` if the game was running) so the new StdTest loads.
2. Type `/stdtest engine answers` and wait for the chat line `engine answers:
   N calls recorded; type /reload so the file is written`. It can pause for a
   few seconds (map lookups, atlases, glyph measurements).
3. Type `/reload`, then run `cargo xtask ingest`.

If the game freezes or crashes instead, the newest file in the client's
`Errors/` folder shows where it stopped (its "Lua Stack").

What it answers:

- Coins: `GetCoinIcon` 0/99/9999, `C_CurrencyInfo.GetCoinIcon` 100/10000.
- Units: `UnitStat("player", 6)`, `GetSpellBonusDamage` 2 to 8,
  `GetMirrorTimerInfo` 2/3/4, `C_PaperDollInfo.GetTemporaryEnchantmentInfo`
  16/17/18/20, `GetNumCompanions("MOUNT")`/`("CRITTER")`, `UnitGUID(nil)`;
  what `UnitName`/`UnitPVPName` give for 17 unit tokens ("none", "target",
  "softenemy"...), as nil, empty, length and whether it equals the player's
  own (no names are stored).
- Game rules: `C_GameRules.IsGameRuleActive` 10/213/214, and every
  `Enum.GameRule` value in one check.
- Commentator and timeline: `C_Commentator.GetTeamColor` 2/3;
  `C_EncounterTimeline.GetCurrentTime()` between two `GetTime()`s.
- Artifact: `C_ArtifactUI.GetCostForPointAtRank` ranks 2 to 10 (both
  argument orders), `GetTotalPowerCost` for a few spans.
- Maps: `C_Map.GetWorldPosFromMapPos` at (0,0) and (1,1) for every recorded
  map; `C_Map.GetMapPosFromWorldPos` for 6 world points (Orgrimmar,
  Crossroads, Mulgore, Stormwind, Ironforge, Undercity) without and with 8
  override maps; `GetMapChildrenInfo` of 1414/1415/947 as counts and IDs;
  which map IDs from 1 to 2500 exist.
- Layout: `C_Texture.GetAtlasInfo` (size, file, texture coordinates, tiling)
  for every atlas the engine knows; string width, height and unbounded width
  of 8 texts and every printable ASCII glyph in GameFontNormal,
  GameFontHighlightSmall, SystemFont_Shadow_Large_Outline and
  NumberFontNormal.
- Script objects: `CreateFrame(..., "BackdropTemplate")`;
  `CreateAbbreviateConfig` with divisors 1/1 (raw and global abbreviation)
  and 60 (as significand and as fraction divisor); writing a field to a
  `C_Timer.NewTimer` and a `NewTicker` handle (both cancelled at once).

## The collections (decision T235)

About a minute, on any character, out of combat; the journals may stay
closed. A character with a mount or a toy also records the events after its
favourite changes; without, the probe records whether the client takes an
uncollected one's (a new character has its starting gear's appearances).

1. Type `/stdtest engine journals` and wait for the chat line `engine
   journals: ... lines; type /reload so the file is written` (about 7
   seconds). It records the order of the mount journal's and the toy box's
   lists (as their filters are, and with every filter on, put back after),
   and every event the client sends when one mount's, one toy's and one
   appearance's favourite changes. Each favourite is changed and put back,
   a second apart; the filters are put back as they were.
2. Type `/stdtest snapshot`: the world profile now records the mount
   journal (every mount with its info, and which uncollected ones the
   character can ride), the toy box (its collected toys; its checks are
   opened for it, put back after) and the wardrobe's collected sources. A toy
   whose item the client has not loaded is asked for: run the snapshot a
   second time a few seconds later so its name comes along.
3. Type `/reload`, and keep a copy of `WTF/Account/<account>/SavedVariables/StdTest.lua`.
4. Then `/stdtest engine journals2` (about 8 seconds, any character): each
   filter's CVar on its own, the toy box's list a second after its filters
   change, every toy, and an appearance's favourite in three more categories,
   on and off. Everything is put back. `/reload` after.

## The method probe

About 10 to 15 minutes, after the answers battery, out of combat, standing
anywhere. `/stdtest engine methodprobe` calls every widget method the client
has (about 13,000 calls on 53 widget kinds), each on a fresh widget under a
hidden frame of its own: nothing shows, no Blizzard frame is touched, no
setting changes. It reloads the UI by itself every 500 calls.

1. Type `/stdtest engine methodprobe`. The chat says `methodprobe: N calls
   (...)`; after each reload it prints `methodprobe: n/N calls answered`. Do
   not type while it runs; in combat it waits. `/stdtest engine methodprobe
   status` shows where it is, `/stdtest engine methodprobe stop` pauses it
   (the same command goes on).
2. It ends with `methodprobe: done, ...; reloading so the file is written`
   and, after that reload, `methodprobe: finished`.
3. Run `cargo xtask ingest` (it writes `forever-camelot-methodprobe.lua`).

If the game freezes or crashes, start it again and log in with the same
character. The probe notices the crash at login, sets that stretch of 500
calls aside, goes on, and comes back to it in smaller steps until the one
call that crashes is found and skipped. The riskiest calls (boundary numbers
like -1 and 2^31-1) come last. More than three crashes: keep the newest file
in `Errors/` and the output of `/stdtest engine methodprobe status`.

## Before and after the next full probe

- `/stdtest engine snapshot` right before `/stdtest engine fullprobe` and
  again right after it finishes, so the replay starts from the exact state
  (bind location, mail, macros, channels, the quest log).
- `/run for i=1,GetNumBindings() do local c=GetBinding(i) print(i,c) end`
  (binding command names only): the binding list.
- `/dump C_Spell.GetSpellPowerCost(20271)`, `/dump C_Spell.GetSpellInfo(20271)`
  and `/dump C_Spell.IsSpellHarmful(20271)` (Judgement): spell cost, range,
  cast time and harmfulness, the data `IsHarmfulAction` needs.
- `/run print(date("%H:%M"), date("!%H:%M"))`: the machine's time zone, for
  `C_DateAndTime.GetCalendarTimeFromEpoch`.

## Situations (one /dump each, whenever it happens)

A screenshot of the output or the values typed back is enough.

1. Under water (until the breath bar shows): `/dump GetMirrorTimerInfo(1)`,
   `/dump GetMirrorTimerInfo(2)` and `/dump GetMirrorTimerProgress("BREATH")`:
   a running breath timer.
2. In a party: `/dump UnitGroupRolesAssigned(), UnitGroupRolesAssigned("player"), UnitGroupRolesAssigned("party1")`
   and `/dump UnitGetAvailableRoles("party1")`: whether the no-argument form
   follows the group role.
3. Inside a dungeon or battleground entrance (only when going there anyway):
   `/dump GetInstanceInfo()` and `/dump GetRealZoneText()`.
4. With a soft target showing:
   `/run local n=UnitName("softinteract") print(n and #n, n==UnitName("target"))`
   (only the length is printed).
5. With a temporary weapon enchant already on (do not use one just for
   this): `/dump C_PaperDollInfo.GetTemporaryEnchantmentInfo(16)` and
   `/dump GetWeaponEnchantInfo()`.
6. In combat with a target that has a buff or debuff:
   `/run local a=C_UnitAuras.GetAuraDataByIndex("target",1) print(a and issecrettable(a), a and issecretvalue(a.spellId), a and issecretvalue(a.duration), a and issecretvalue(a.name))`
   and the same for `C_UnitAuras.GetPlayerAuraBySpellID(<one of your buffs' spell IDs>)`:
   whether the aura table or only its fields are secret.

7. In a party when a member logs out or back in (decision T249): a frame
   registered for `UNIT_CONNECTION` and `GROUP_ROSTER_UPDATE` printing each
   with its arguments: whether `UNIT_CONNECTION("party1", false)` comes, for
   `target` too while they're targeted, and before or after the roster
   update.

8. Anywhere, once: an addon's timer on the encounter timeline. That it
   goes on it is settled (2026-10-22: SuuUI's pull countdown shows on its
   timeline in game; the engine keeps script events, decision T255); what
   it answers is the engine's guess from the documentation:
   `/dump C_EncounterTimeline.AddScriptEvent({ spellID = 100, iconFileID = 132337, duration = 30, overrideName = "Probe" })`,
   then `/dump C_EncounterTimeline.GetEventList()` and, for the ID,
   `/dump C_EncounterTimeline.GetEventInfo(<id>)`,
   `/dump C_EncounterTimeline.GetEventState(<id>)`,
   `/dump C_EncounterTimeline.GetEventTimeRemaining(<id>)`: the ID (the
   engine counts from 41), the info's fields (its `spellName` the
   `overrideName`), the state while it runs (0) and after it ran out.

9. In combat, from an addon (a `/run` counts as one): `/run C_Spell.PickupSpell(6603)`
   (Auto Attack) and `/run PickupAction(1)`: whether either is blocked as
   `C_SpellBook.PickupSpellBookItem` is (decision T253), then
   `/run ClearCursor()`.

10. On a rogue (or a druid in cat form), in a fight with a target: a frame
    registered for `UNIT_POWER_UPDATE`, `UNIT_POWER_FREQUENT` and
    `PLAYER_TARGET_CHANGED` printing each with its arguments, then build
    two combo points and
    `/dump GetComboPoints("player", "target"), UnitPower("player", Enum.PowerType.ComboPoints), UnitPowerMax("player", Enum.PowerType.ComboPoints)`;
    then target another creature and dump again. Why: the engine has no
    combo points (SuuUI's finding V31): their events, and whether they stay
    on the target or with the player.
11. In a duel with a friend (when one happens anyway): a frame registered
    for `UNIT_FACTION` and `PLAYER_TARGET_CHANGED` printing each with its
    arguments, the friend targeted;
    `/dump UnitCanAttack("player", "target"), UnitReaction("target", "player"), UnitIsEnemy("player", "target")`
    before the duel starts, during it, and after. Why: a unit can't turn
    hostile in the engine (SuuUI's finding V30): the events and answers as
    it does.

## Replay leftovers

- Before turning in a quest with an item reward, two lines:
  `/run Q=function() print(GetTime(), C_Container.GetContainerNumFreeSlots(0)) end`
  then `/run local f=CreateFrame("Frame") f:RegisterEvent("QUEST_TURNED_IN") f:SetScript("OnEvent",function() Q() C_Timer.After(0.2,Q) end)`:
  when the reward takes its bag slot.
- Around a second level-up: `/stdtest engine scenario`, to confirm the
  specialization event is batched.
- Queue Heroic Strike on a mob, then change target before the swing; note
  any error text.
- While smelting bronze with copper, tin and bronze stacks all in the bags
  (with and without a skill-up), and once crafting with the last reagent
  stack: `/stdtest engine scenario`. Answers the order of the item counts
  and when a used-up stack is removed (the profession replay's leftovers).

## The guild

One `/dump` each, any time (names in the output can be cropped from a
screenshot).

1. `/dump C_GuildInfo.GuildControlGetRankFlags(2), C_GuildInfo.GuildControlGetRankFlags(3)`
   and the same for 4 and 5.
2. `/dump GuildControlGetAllowedShifts(3), GuildControlGetAllowedShifts(4), GuildControlGetAllowedShifts(5)`
3. `/run for i=1,GetNumGuildMembers() do local _,r,ri,l,c=GetGuildRosterInfo(i) print(i,ri,l,c) end`
   (rank index, level and class only), once with "show offline" on and once
   off.
4. `/run for i=3,10 do print(GetGuildTradeSkillInfo(i)) end`
5. `/dump C_Club.GetClubInfo(C_Club.GetGuildClubId())` and
   `/dump C_Club.GetClubPrivileges(C_Club.GetGuildClubId())`.

## Widget state (decision T289)

The engine's guesses for methods that had no behaviour. Anywhere, out of
combat; each step is one line to paste, then a screenshot of the chat.

Steps 1 to 7 and 11 to 14 were answered by the batched run (2026-11-09,
70862; "Answered by the batched run"). Step 4 stays as the set-up of 8 to
10.

**The selection on focus** (`HighlightText` in `OnEditFocusGained`):

4. `/run E=CreateFrame("EditBox",nil,UIParent) E:SetSize(200,20) E:SetPoint("CENTER") E:SetFontObject(ChatFontNormal) E:SetAutoFocus(false) E:SetText("hello world") E:HighlightText(0,5) E:Insert("HELLO") print(E:GetText(),E:GetCursorPosition())`
8. `/run E:SetScript("OnEditFocusGained",function(s) s:HighlightText() end) E:SetText("Sheep")`
9. By hand: click into the box (in the middle of the screen), type `x`.
10. `/run print(E:GetText()) E:ClearFocus() E:Hide()`

Screenshot after step 10. The engine says `x`: the text selected on focus
stays selected after the click, and typing replaces it. If the click put
the cursor somewhere instead, it says where the text went.

**At a vendor** (the merchant's window open):

15. `/run local f=CreateFrame("Frame") f:RegisterEvent("MERCHANT_UPDATE") f:SetScript("OnEvent",function(_,e) print(e,GetMerchantFilter(),GetMerchantNumItems()) end) print(LE_LOOT_FILTER_ALL,LE_LOOT_FILTER_CLASS,GetMerchantFilter(),GetMerchantNumItems())`
16. `/run SetMerchantFilter(LE_LOOT_FILTER_ALL)`
17. `/run ResetSetMerchantFilter()`

Screenshot after step 17. The engine keeps the filter and tells
`MERCHANT_UPDATE` once per change, every item still listed; the client may
list fewer items with the class filter.

**In a guild**:

18. `/run print(GetGuildRosterSelection()) SetGuildRosterSelection(2) print(GetGuildRosterSelection()) SetGuildRosterSelection(999) print(GetGuildRosterSelection())`

Screenshot. The engine says 0, 2, 0 (a row past the roster selects
none).

## Targeting and history (decision T291)

**Tab-targeting**, near a few creatures (out of combat):

1. `/run local f=CreateFrame("Frame") f:RegisterEvent("PLAYER_TARGET_CHANGED") f:SetScript("OnEvent",function() print(UnitName("target"),UnitReaction("target","player")) end)`
2. Press Tab five times, then Shift-Tab twice.

Screenshot. The engine cycles the hostile and neutral creatures within
40 yards nearest first, Shift-Tab back; the names and reactions show
whether neutral ones count and how far it looks.

**An edit box's history and arrows**:

3. `/run H=CreateFrame("EditBox",nil,UIParent) H:SetSize(200,20) H:SetPoint("CENTER") H:SetFontObject(ChatFontNormal) H:SetAutoFocus(false) H:AddHistoryLine("one") H:AddHistoryLine("two") print(H:GetHistoryLines())`
   (answered 2026-11-09 by the batched run: 0, as the engine; still the
   set-up of 4 to 7)
4. `/run H:SetScript("OnArrowPressed",function(s,k) print("arrow",k,s:GetText()) end) H:SetScript("OnTextChanged",function(s,u) print("changed",u,s:GetText()) end) H:SetFocus()`
5. By hand: press Up twice, then Down twice.
6. `/run H:SetHistoryLines(5) H:AddHistoryLine("three") H:SetFocus()`
7. By hand: press Up, then Escape.

Screenshot after step 7. The engine keeps no line without
`SetHistoryLines` (step 5 changes nothing), runs `OnArrowPressed` before
the text changes, and step 7 brings back "three".

## Leftovers (decision T295)

**Money on the cursor** (bags open, some money):

1. `/run local f=CreateFrame("Frame") f:RegisterEvent("CURSOR_CHANGED") f:SetScript("OnEvent",function(_,e,...) print(e,...) end) PickupPlayerMoney(1) print(GetCursorMoney(),GetCursorInfo())`
2. `/run ClearCursor() print(GetCursorMoney())`

Screenshot after step 2. The engine says `CURSOR_CHANGED false 2 0 0`,
then 1 (and no cursor info), then `CURSOR_CHANGED true 0 2 0` and 0.

**Assignments** (as a raid leader, a member targeted):

3. `/run local f=CreateFrame("Frame") f:RegisterEvent("GROUP_ROSTER_UPDATE") f:SetScript("OnEvent",function(_,e) print(e) end) SetPartyAssignment("MAINTANK","target") C_Timer.After(1,function() print(GetPartyAssignment("MAINTANK","target")) end)`

Screenshot. The engine tells `GROUP_ROSTER_UPDATE` at once and true.

(Step 4, a system line and `Sign`, was answered by the batched run,
2026-11-09: as the engine.)

## Regressions and other flavors

1. With SuuUI's current bags, open the spellbook, enter combat (a training
   dummy is enough) and press Escape: the spells window should close, with no
   "Interface action failed" in chat. Confirms the fix the regression test
   `suuui_bag_calls_taint_escape_in_combat` stands for.
2. On a classic flavor's client (vanilla, tbc, wrath or mists), run the
   function inventory probe as on camelot, so the engine knows which of the
   old global names (`GetItemCount`, `InviteUnit`, `UseContainerItem`, ...)
   that client really defines.

## Model frames' cameras

The wardrobe's head shots (`Model_ApplyUICamera`) sit off center in
wowlua's screenshots: how the client's model frame camera sees a model is
not known yet. Answered (2026-10-23, decision T334) with a DressUpModel of
the player under UIParent and UI camera 542 at 200 by 250 and 400 by 250,
with and without `SetPosition(0, 0, 0)`; the wardrobe's head cells match
a Skyborne female's screenshot once the camera follows the race (T335).
Still open: the body's transmog skin (`useTransmogSkin`, drawn dark in
game, the character's clothes in wowlua).

1. Open the appearances collection on the head slot and take a screenshot of
   the first page (the cells' framing to compare with).
2. With the collections open, two lines:
   `/run M=WardrobeCollectionFrame.ItemsCollectionFrame.Models[1] print(M:GetCameraPosition()) print(M:GetCameraTarget()) print(M:GetPosition())`
   then `/run print(M:GetFacing(), M:GetPitch(), M:GetRoll()) print(M:IsUsingModelCenterToTransform()) print(M:TransformCameraSpaceToModelSpace(CreateVector3D(4,0,0)):GetXYZ())`
3. `/run local m=CreateFrame("PlayerModel") m:SetSize(100,100) m:SetUnit("player") print(m:GetCameraDistance(), m:GetCameraPosition()) print(m:GetCameraTarget())`
   Answers the model frame's own camera and field of view. Recorded
   2026-11-09 (70862, the batched run, a paladin's model in a frame never
   shown): `-1.4979703116779e+30 2.8272478580475 -0.07107862085104
   0.68821173906326`, then `-0.29772615432739 -0.074905693531036
   0.74097633361816` (wowlua `0 0 0 0`, `0 0 0`). Open: the distance is no
   number (the camera not set up before the frame draws); owed again with
   the frame shown: `/run local m=CreateFrame("PlayerModel",nil,UIParent) m:SetSize(100,100) m:SetPoint("CENTER") m:SetUnit("player") C_Timer.After(0.5,function() print(m:GetCameraDistance(), m:GetCameraPosition()) print(m:GetCameraTarget()) m:Hide() end)`

## Textures the screenshots draw

(Items 1 and 4 were answered by the batched run, 2026-11-09: "Answered by
the batched run".)

2. An action button with a spell: `/run local b=ActionButton1 print(b.icon:GetDrawLayer()) print(b.SlotArt:GetDrawLayer()) print(b.icon:GetNumMaskTextures())`
   and a screenshot of it hovered: wowlua draws a masked texture over the
   unmasked ones of its layer and sublevel (the icon over the slot art).
   Answered (2026-10-23): both `BACKGROUND` 0, one mask, as in wowlua;
   still open: the hovered screenshot (which of the two is drawn on top).
3. `/run local t=UIParent:CreateTexture() print(t:IsSnappingToPixelGrid(), t:GetTexelSnappingBias())`:
   a fresh texture's pixel snapping (wowlua: true, 0). Answered
   (2026-10-23): true, 0.30000001192093; wowlua now gives the same.
5. **An addon's file that is there** (item 4 used SuuUI's
   `Interface\AddOns\SuuUI\media\logo`, which SuuUI does not have: all nil).
   SuuUI's `Media\Textures\Hatch.tga` is there:
   1. `/run T=UIParent:CreateTexture() print(T:SetTexture("Interface\\AddOns\\SuuUI\\Media\\Textures\\Hatch"), T:GetTexture(), T:GetTextureFileID(), T:GetTextureFilePath())`
   2. `/run print(GetFileIDFromPath("Interface\\AddOns\\SuuUI\\Media\\Fonts\\Figtree-Medium.ttf"), GetFileIDFromPath("Interface\\AddOns\\SuuUI\\Media\\Textures\\Hatch"))`

   wowlua: the loose file's negative ID for both getters and the path for
   `GetTextureFilePath` (decision T356, unverified).

## Minimap

wowlua draws the minimap's terrain at the long-known zoom spans and the
player's arrow 32 by 32 UI units; neither is recorded on camelot.

1. Outdoors in Northshire (or any place with a landmark near), take a
   screenshot of the minimap at each zoom: `/run Minimap:SetZoom(0)` to
   `/run Minimap:SetZoom(5)`, with `/run print(UnitPosition("player"))`
   and `/run print(C_Map.GetPlayerMapPosition(C_Map.GetBestMapForUnit("player"), "player"):GetXY())`
   for each. The landmark's distance in the shots gives each zoom's span.
2. The same spot with `/console rotateMinimap 1`, facing east: the terrain
   turned and the arrow up.
3. In a party, a screenshot of the minimap with a member in sight, and
   one of a ping (click the minimap) at once, with
   `/run C_Timer.NewTicker(0.5,function() print("ping",GetTime(),Minimap:GetPingPosition()) end,16)`
   first: the blip's size, the ping's look and how long it keeps its
   place (`RegisterEvent("MINIMAP_PING")` is refused as unknown,
   2026-10-23: a callback event alone).
4. Which callback events `RegisterEvent` refuses, and whether an addon's
   callback hears one:
   `/run P=function(e) local f=CreateFrame("Frame") print(e,(pcall(f.RegisterEvent,f,e)),C_EventUtils.IsEventValid(e)) end`
   then `P` for each of the documentation's twelve `CallbackEvent`s, and
   `/run CreateFrame("Frame"):RegisterEventCallback("MINIMAP_PING",function(_,...) print("cb",...) end)`
   before a ping (the payload's order).

## Unanchored textures and slider thumbs

1. `/run local f=CreateFrame("Frame",nil,UIParent) f:SetSize(200,100) f:SetPoint("CENTER") local t=f:CreateTexture() t:SetSize(40,20) print(t:GetNumPoints(), t:GetSize())`
   then the same with a texture from XML that has `<Size>` and no
   `<Anchors>` (any addon's): wowlua fills the parent with an XML one
   (2 points, the parent's size), leaves a Lua one unplaced. The Lua half
   answered 2026-11-09 (the batched run): `0 40 20`, as wowlua; the XML
   half owed (an XML font string answered one point, decision T602).
2. Open Settings, Mouse, hover the Mouse Look Speed slider with
   `/fstack` on, and note the thumb's left relative to the slider at the
   value shown, at the minimum and at the maximum (wowlua: it travels the
   slider's width less the thumb's, by the value). Answered for a
   horizontal slider made in Lua (2026-10-23): it does, by one point,
   `LEFT` to the slider's `LEFT`; still open: a vertical slider's point.


## Owed from the M12 screenshots (next beta, about 2026-11-07)

Answered by the batched run (2026-11-09, 70862; "Answered by the batched
run" below): 1, 2, 3, 4, 7, 11, 14 and 17 (taken out here), 8 steps 1-2,
16 steps 1-2, 18 step 2, 19 steps 7 and 9, 20d steps 1-2 (and step 3
without the character frame), 20e step 3.

The camelot beta closed on 2026-10-24; these close what the screenshot work
left as proposals. Each item stands alone: after a `/reload` (or an error)
start the item again from its first step. In the chat box `|` is escaped,
so the lines write it as `\124`. Take the screenshots with the default UI
alone unless an item names an addon.

5. **Font strings over textures in one layer** (decision T340; wowlua: the
   text is drawn over the red, though made first):
   1. `/run F=CreateFrame("Frame",nil,UIParent) F:SetSize(240,60) F:SetPoint("CENTER") local s=F:CreateFontString(nil,"ARTWORK","GameFontNormalHuge") s:SetPoint("CENTER") s:SetText("TEXT ON TOP?")`
   2. `/run local t=F:CreateTexture(nil,"ARTWORK") t:SetAllPoints() t:SetColorTexture(1,0,0)`
   3. A screenshot.
6. **A frame on its parent's level** (decision T341; wowlua: the child's
   green `BACKGROUND` is drawn under the parent's blue `ARTWORK`, the two
   sorted together by layer):
   1. `/run local p=CreateFrame("Frame",nil,UIParent) p:SetSize(200,100) p:SetPoint("CENTER") local a=p:CreateTexture(nil,"ARTWORK") a:SetPoint("LEFT") a:SetSize(120,100) a:SetColorTexture(0,0,1) P=p`
   2. `/run local c=CreateFrame("Frame",nil,P) c:SetUsingParentLevel(true) c:SetAllPoints() local g=c:CreateTexture(nil,"BACKGROUND") g:SetPoint("RIGHT") g:SetSize(120,100) g:SetColorTexture(0,1,0)`
   3. A screenshot: is the middle strip blue (wowlua) or green?
8. **Inline textures in text** (decision T347; wowlua: the icon is as tall
   as the text for height 0, the atlas 20 by 20, each its width in the
   line). Steps 1 and 2 recorded 2026-11-09 (70862): `15.000000953674
   11.666667938232`, then `53.289131164551 19.122459411621` (wowlua `16
   12`, `56 12`): open, the line taller with the atlas; step 3 owed, and
   with it `/run print(UIParent:GetEffectiveScale(), GetPhysicalScreenSize())`
   (the screen's scale the measures were taken at):
   1. `/run S=UIParent:CreateFontString(nil,"OVERLAY","GameFontNormal") S:SetPoint("CENTER") S:SetText("ab") print(S:GetStringWidth(),S:GetStringHeight())`
   2. `/run S:SetText("a\124TInterface\\Icons\\INV_Misc_QuestionMark:0\124tb\124A:128-RedButton-Exit:20:20\124ac") print(S:GetStringWidth(),S:GetStringHeight())`
   3. A screenshot of the text in the middle of the screen.
9. **The dressing room** (decision T348; wowlua: `OnDressModel` once per
   changed slot, at once; the outfit list shows an item it does not know
   as not collectable, red):
   1. `/run DressUpItemLink("item:2575") DressUpFrame.ModelScene:HookScript("OnDressModel",function(_,...) print("dress",GetTime(),...) end)`
   2. `/run DressUpItemLink("item:6125")`
   3. A screenshot of the dressing room with the outfit list open.
10. **Gradients** (decision T349; wowlua: red at the left to blue at the
    right, and turned with the texture):
    1. `/run local t=UIParent:CreateTexture() t:SetSize(200,50) t:SetPoint("CENTER") t:SetColorTexture(1,1,1) t:SetGradient("HORIZONTAL",CreateColor(1,0,0,1),CreateColor(0,0,1,1)) T=t`
    2. A screenshot.
    3. `/run T:SetRotation(math.pi/4)` and a screenshot.
12. **Text in a scaled frame** (decision T351; wowlua: 0.8 the size, the
    width the same in the frame's units):
    1. `/run F=CreateFrame("Frame",nil,UIParent) F:SetScale(0.8) F:SetSize(200,40) F:SetPoint("CENTER") S=F:CreateFontString(nil,"OVERLAY","GameFontNormal") S:SetPoint("CENTER") S:SetText("Scaled 0.8 text")`
    2. `/run print(S:GetStringWidth(),S:GetEffectiveScale())`
    3. A screenshot.
13. **A creature display's portrait** (decision T353; wowlua: the face,
    framed as a unit's portrait):
    1. `/run local t=UIParent:CreateTexture() t:SetSize(128,128) t:SetPoint("CENTER") SetPortraitTextureFromCreatureDisplayID(t,7970)`
    2. A screenshot (7970 is Taragaman the Hungerer, AtlasLoot's).
15. **Restricted callback events** (decision T336; wowlua: refused from
    addon code, `ADDON_ACTION_FORBIDDEN` with `Frame:RegisterEventCallback()`).
    It shows a blocked-action message: `/reload` after it.
    1. `/run local f=CreateFrame("Frame") print(f:RegisterEventCallback("COMBAT_LOG_ENTRIES_CLEARED",print))`
16. **Tooltip spacing** (decision T342; wowlua: 10 padding, 2 between
    lines, 40 between a double line's sides). Steps 1 and 2 recorded
    2026-11-09 (70862): `250.88012695312 79.262634277344`, a frame later
    the same, 3 lines (wowlua `320 78`): open with Owed from M13 item 1;
    step 3 owed:
    1. `/run GameTooltip:SetOwner(UIParent,"ANCHOR_NONE") GameTooltip:SetPoint("CENTER") GameTooltip:AddLine("Title") GameTooltip:AddDoubleLine("Left","Right")`
    2. `/run GameTooltip:AddLine("A long line to see where and how a tooltip wraps its text",1,1,1,true) GameTooltip:Show() print(GameTooltip:GetSize())`
    3. A screenshot of the tooltip.
18. **The calendar's holidays** (open: wowlua shows no holiday events).
    Step 2 recorded 2026-11-09 (70862, "Answered by the batched run"):
    November 2026's holidays, the same before and 2 s after
    `C_Calendar.OpenCalendar()`; wowlua none. Open: the holidays' schedule
    (the server's calendar, over the client's `Holidays` table); step 1
    owed:
    1. Open the calendar on this month and take a screenshot.
    2. `/run local m=C_Calendar.GetMonthInfo(0) for d=1,m.numDays do for j=1,C_Calendar.GetNumDayEvents(0,d) do local e=C_Calendar.GetDayEvent(0,d,j) print(d,e.title,e.calendarType,e.sequenceType,e.eventID) end end`
19. **The skill bars' fill.** In game (screenshot of 2026-10-23, build
    70532, the same Blizzard code as wowlua's) the skills list's bars fill
    the whole track at any value, the left `p` of the atlas stretched over
    it (`SetTexCoord(0, p, 1, 0)`), while the detail pane's bar fills by
    value (87/150 to 57%). wowlua fills the list's by value too. Blizzard's
    `SetFillPercent` sets the fill to `p * FillArea:GetWidth()`, so in game
    `FillArea` must read 0 when the rows are set, and a 0-wide fill with one
    point is drawn at its atlas size (240, clipped by the bar's mask). Why
    it reads 0 is open: rows made and initialized in the same frame are the
    likely cause, but Blizzard's micro menu and text-to-speech options read
    sizes of regions made moments before at login and need them at once
    (unless they fail silently in game; step 1 shows it). Steps 7 and 9
    recorded 2026-11-09 (70862): `0 15` (as wowlua), and `100.00003051758
    true true`: a chain made in one call has its size at once and its
    rectangles are valid once asked for (wowlua answered `100 false false`;
    now as the game, decision T605). Steps 1 to 6 and 8 owed:
    1. `/console scriptErrors 1`, then `/reload`; note any error shown at
       login (`FrameUtil.lua:247`, `TextToSpeechFrame.lua:597`?), then
       `/console scriptErrors 0`.
    2. `/reload`, and do not open the character window before step 4.
    3. `/run hooksecurefunc(SkillsBarMixin,"SetFillPercent",function(b,p) local a=b.FillArea print(format("%.2f",p),a:GetWidth(),tostring(a:IsRectValid()),b:GetWidth(),b:GetParent():GetParent():GetWidth()) end)`
    4. Open the character window's Skills tab; note the lines printed.
    5. `/run for _,f in ipairs(SkillsFrame.ScrollBox:GetFrames()) do local s=f.Content and f.Content.SkillsBar if s then print(s.FillArea:GetWidth(),s.Fill:GetWidth(),s.Fill:GetTexCoord()) end end`
    6. `/run local b=SkillsFrame.SkillDetailFrame.RankBar print(b.FillArea:GetWidth(),b.Fill:GetWidth())`
    7. `/run T=UIParent:CreateTexture(nil,"OVERLAY") T:SetAtlas("common-stat-bar-blue",true) T:SetPoint("LEFT",UIParent,"CENTER",-100,0) T:SetWidth(0) print(T:GetSize())`
    8. A screenshot: is a 240-wide blue bar drawn in the middle (wowlua:
       nothing)?
    9. `/run local p=CreateFrame("Frame",nil,UIParent) p:SetPoint("CENTER") p:SetSize(100,50) local c=CreateFrame("Frame",nil,p) c:SetAllPoints() local g=CreateFrame("Frame",nil,c) g:SetAllPoints() print(g:GetWidth(),g:IsRectValid(),c:IsRectValid())`
       (wowlua: `100 true true`): whether a chain made in one call has its
       size at once.
20a. **Chat the player sends** (decision T360; wowlua: `CHAT_MSG_SAY`
    back, `CHAT_MSG_WHISPER_INFORM` then `CHAT_MSG_WHISPER` for a whisper to
    yourself, `ERR_CHAT_PLAYER_NOT_FOUND_S` for a name nobody has, `ERR_NOT_IN_GROUP`
    for /p alone):
    1. `/run F=CreateFrame("Frame") for _,e in ipairs({"CHAT_MSG_SAY","CHAT_MSG_WHISPER","CHAT_MSG_WHISPER_INFORM","CHAT_MSG_SYSTEM","CHAT_MSG_PARTY"}) do F:RegisterEvent(e) end F:SetScript("OnEvent",function(_,e,...) print(e,...) end)`
    2. `/s probe one`, then `/p probe two` (not in a group).
    3. `/w <your first and last name> probe three`, then `/w Nobody Anyone probe four`.
    4. A screenshot of the chat with the printed lines.
20b. **Typed commands' answers** (decision T362; wowlua's lines in the
    decision). With the frame of 20a also registering `CHAT_MSG_TEXT_EMOTE`,
    `TIME_PLAYED_MSG`, `WHO_LIST_UPDATE`, `PLAYER_FLAGS_CHANGED`, and
    `IGNORELIST_UPDATE`, type each and note what prints:
    1. `/roll`, `/roll 1 6`, `/played`, `/afk`, `/afk`, `/dnd`, `/dnd busy`
    2. `/ignore Nobody Anyone`, `/invite Nobody Anyone`, `/ginvite Nobody Anyone`,
       `/gpromote Nobody Anyone`, `/gquit` (as guild master), `/who Nobody`
    3. `/dance`, `/friend Nobody Anyone`, `/1 hello`, `/join Trade`
20c. **Inviting players** (decision T363), with a second character of yours
    online: `/invite` it and note the lines and how long until it shows in
    the party after accepting; `/promote` and `/kick` it; `/who` with three
    or fewer results and with more (the chat lines); `/ginvite` (the lines
    after it accepts).
20d. **Mouse scripts turn the mouse on** (decision T365; wowlua: `false
    true true true true true`; the in-game suite's "Mouse script enables"
    asks the same). Steps 1 and 2 recorded 2026-11-09 (70862): `false true
    true`, `true false true`, as wowlua; step 3 without the character frame
    opened `true true true` (wowlua `true false false`, now as the game).
    Step 3 with the frame open owed:
    1. `/run local f=CreateFrame("Frame") local a=f:IsMouseEnabled() f:SetScript("OnMouseUp",function() end) local b=f:IsMouseEnabled() f:SetScript("OnMouseUp",nil) print(a,b,f:IsMouseEnabled())`
    2. `/run local g=CreateFrame("Frame") g:SetScript("OnEnter",function() end) local h=CreateFrame("Frame") h:SetScript("OnMouseWheel",function() end) print(g:IsMouseEnabled(),h:IsMouseEnabled(),h:IsMouseWheelEnabled())`
    3. Open the character frame (C) and type `/run local t=CharacterFrameModeTab2 print(t:IsMouseEnabled(),t:IsMouseClickEnabled(),t:IsMouseMotionEnabled())`
20e. **`BNGetInfo()`** (two recordings disagree: the full probe of
    2026-10-20 has nothing, the answers of 2026-10-21 eight values; wowlua
    answers the eight). Step 3 recorded 2026-11-09 (70862): `0 true`, as
    wowlua; and `select("#", BNGetInfo())` mid-session: 0 (a third
    recording with nothing; wowlua 8, kept until steps 1 and 2 answer).
    Steps 1 and 2 owed:
    1. Right after logging in (before opening anything): `/run print(select("#",BNGetInfo()),BNGetInfo())`
    2. `/reload`, then the same line again.
    3. `/run print(select("#",BNGetInfo("player")),BNConnected())`
20. **Screenshots to compare with wowlua's** (in `~/Desktop/wowlua-*`):
    1. Settings, Accessibility: the "Colors" category and the "Use UI
       Scale" check box (Graphics).
    2. The Forever Dungeon Journal: a boss with its portrait (the ring under
       it).
    3. EllesmereUI: the action bars' key labels and the chat window's text.
    4. AtlasLoot: Ragefire Chasm, Taragaman, its loot list.


## Owed from M13 (next beta, about 2026-11-07)

Answered by the batched run (2026-11-09, 70862; "Answered by the batched
run" below): 3 (taken out here); 1's widths and 2 recorded, still open.

What the M13 work (the client's tables in tests) guessed. Each item stands
alone: after a `/reload` (or an error) start it again from its first step.
Take a screenshot after each step that names one.

1. **A tooltip's wrap width** (decision T393; wowlua wraps at the widest
   other line, at least 300). The widths recorded 2026-11-09 (70862):
   250.87994384766 and 229.17562866211, the same a frame later (wowlua 298
   and 320): the client wraps narrower, its rule open; the screenshots
   owed. Take a screenshot after each of:
   1. `/run GameTooltip:SetOwner(UIParent,"ANCHOR_CURSOR") GameTooltip:SetSpellByID(26573) GameTooltip:Show() print(GameTooltip:GetWidth())`
   2. `/run GameTooltip:SetOwner(UIParent,"ANCHOR_CURSOR") GameTooltip:AddLine("Short") GameTooltip:AddLine(string.rep("word ",60),1,1,1,true) GameTooltip:Show() print(GameTooltip:GetWidth())`
2. **A spell's description with spell power** (decision T391; wowlua adds
   the coefficient times `GetSpellBonusDamage`): on a paladin with gear
   that gives spell power. Recorded 2026-11-09 (70862, a level 36
   paladin): `17 25`; Consecration "… doing 16 Holy damage over 8 sec … an
   additional 38 damage over 8 sec."; Holy Light "Heals a friendly target
   for 43 to 52." Not compared yet: the emulator's default world is a
   level 1 warrior (32, and "39 to 47" with the installation); owed: the
   same in a test with that paladin.
   1. `/run print(GetSpellBonusDamage(2), GetSpellBonusHealing())`
   2. `/run print(C_Spell.GetSpellDescription(26573))`
   3. `/run print(C_Spell.GetSpellDescription(635))`
## Owed from M14 (next beta, about 2026-11-07)

All answered by the batched run (2026-11-09, 70862; "Answered by the
batched run" below).

## Owed from the contributions in 0.13.5 (next beta, about 2026-11-07)

Answered by the batched run (2026-11-09, 70862; "Answered by the batched
run" below): 1, 2 and 4 (taken out here); 6 recorded, still open.

What pull requests #81 to #91 guessed. Their decisions (T485 to T490)
stay proposals, open for review, until these answer. Each item stands
alone: after a `/reload` (or an error) start it again from its first step.
After each step, note what it prints; take a screenshot where a step says
so. "wowlua:" is what the engine answers now.

3. **`UNIT_IN_RANGE_UPDATE`** (decision T485): in a party with another
   player, who can walk away.
   1. `/run local f=CreateFrame("Frame") f:RegisterEvent("UNIT_IN_RANGE_UPDATE") f:SetScript("OnEvent",function(_,e,u,r) print(e,u,issecretvalue(r)) end) WOWLUA_RANGE=f`
   2. The other player walks more than 40 yards away.
   3. The other player comes back next to you.

   wowlua: one line at each crossing of 38 yards, for `party1` only, with
   `true` (the payload secret). Note every token it names, and whether
   the payload is secret.
5. **Shields and `UNIT_HEALTH`** (decision T486): on a character that can
   get an absorb shield (a priest's Power Word: Shield).
   1. `/run local f=CreateFrame("Frame") for _,e in ipairs({"UNIT_ABSORB_AMOUNT_CHANGED","UNIT_HEAL_ABSORB_AMOUNT_CHANGED","UNIT_HEALTH","UNIT_HEAL_PREDICTION"}) do f:RegisterUnitEvent(e,"player") end f:SetScript("OnEvent",function(_,e) print(GetTime(),e) end) WOWLUA_ABS=f`
   2. Get the shield, then take one hit the shield takes whole.
   3. Take a hit bigger than what is left of the shield.

   wowlua: the shield's `UNIT_ABSORB_AMOUNT_CHANGED` before `UNIT_HEALTH`,
   and no `UNIT_HEALTH` for the hit the shield takes whole.
6. **A status bar on a timer** (decision T487):
   1. `/run local b=CreateFrame("StatusBar",nil,UIParent) b:SetSize(200,20) b:SetPoint("CENTER") b:SetStatusBarTexture("Interface\\Buttons\\WHITE8X8") print(b:GetMinMaxValues()) b:SetScript("OnValueChanged",function() WOWLUA_VC=(WOWLUA_VC or 0)+1 end) b:SetScript("OnMinMaxChanged",function() WOWLUA_MC=(WOWLUA_MC or 0)+1 end) local d=C_DurationUtil.CreateDuration() d:SetTimeFromStart(GetTime(),10) b:SetTimerDuration(d) WOWLUA_BAR=b print(b:GetMinMaxValues()) print(b:GetValue())`
   2. About 3 seconds later, with a screenshot: `/run print(WOWLUA_BAR:GetMinMaxValues()) print(WOWLUA_BAR:GetValue(), WOWLUA_VC, WOWLUA_MC)`
   3. `/run WOWLUA_BAR:SetValue(3) print(WOWLUA_BAR:GetValue())`
   4. About 2 seconds later: `/run print(WOWLUA_BAR:GetValue())`
   5. `/run local b=CreateFrame("StatusBar",nil,UIParent) b:SetMinMaxValues(0,200) local d=C_DurationUtil.CreateDuration() d:SetTimeFromStart(GetTime(),10) b:SetTimerDuration(d,Enum.StatusBarInterpolation.Immediate,Enum.StatusBarTimerDirection.RemainingTime) WOWLUA_BAR2=b print(b:GetMinMaxValues()) print(b:GetValue())`
   6. About 3 seconds later: `/run print(WOWLUA_BAR2:GetValue()) WOWLUA_BAR2:SetMinMaxValues(0,0) print(WOWLUA_BAR2:GetValue())`

   wowlua: step 1 `0 0` before the timer, `0 10` and `0` after it; step 2 `0 10`, about
   `3`, `nil nil` (no scripts run); steps 3 and 4 `3` (a value set stops
   the timer); step 5 `0 10` and `10`; step 6 about `7`, then `0`.

   Recorded 2026-11-09 (70862, the batched run): step 1 `0 0`, `0 0`,
   `0`; step 2 `0 0`, `0 nil nil`; steps 3 and 4 `0`; step 5 `0 0`, `0`;
   step 6 `0`, `0`: the bar followed no timer, its range 0 to 0 (also
   over the 0 to 200 set before). Open (decision T487): whether the
   duration read 0. Owed, one line each:
   1. `/run local d=C_DurationUtil.CreateDuration() d:SetTimeFromStart(GetTime(),10) print(d:GetStartTime(), d:GetEndTime(), d:GetTotalDuration(), d:GetRemainingDuration(), d:IsZero())`
   2. `/run local d=C_DurationUtil.CreateDuration() d:SetTimeFromStart(GetTime(),10) local b=CreateFrame("StatusBar",nil,UIParent) b:SetMinMaxValues(0,200) b:SetValue(50) b:SetTimerDuration(d) print(b:GetMinMaxValues()) print(b:GetValue())`
7. **A frame anchored to a bar's fill, with a secret value** (decision
   T490): in a dungeon, where the player's health is secret in combat.
   1. Out of combat: `/run local b=CreateFrame("StatusBar",nil,UIParent) b:SetSize(200,20) b:SetPoint("CENTER") b:SetStatusBarTexture("Interface\\Buttons\\WHITE8X8") local n=CreateFrame("Frame",nil,b) n:SetPoint("TOPLEFT",b:GetStatusBarTexture(),"TOPRIGHT") n:SetPoint("BOTTOMLEFT",b:GetStatusBarTexture(),"BOTTOMRIGHT") n:SetWidth(10) local t=n:CreateTexture() t:SetAllPoints() t:SetColorTexture(1,0,0) WOWLUA_HB=b WOWLUA_HN=n print(b:GetStatusBarTexture():GetNumPoints())`
   2. In combat, with a screenshot: `/run WOWLUA_HB:SetMinMaxValues(0,UnitHealthMax("player")) WOWLUA_HB:SetValue(UnitHealth("player")) print(issecretvalue(UnitHealth("player")), issecretvalue(WOWLUA_HN:GetLeft()))`
   3. Still in combat: `/run print(WOWLUA_HN:GetLeft())`

   wowlua: step 1 `0`; step 2 `true false`, the red square at the fill's
   right edge; step 3 a plain number. If step 2 prints `false` first, the
   health isn't secret there: try again in a boss fight.

## Owed from the world profiles (next beta, about 2026-11-07)

The snapshot moved from StdTest into the dev runtime (decision T498):
`/wl snapshot` takes it in any dev build and `wowlua profile snapshot`
writes the profile. Its calls are StdTest's, probed before; these check the
move. Install StdTest (`wowlua install --client _forever_` in
`tests/ingame`) and log in with a character that has gear, bags, action
bars and a target.

1. **`/wl snapshot`**:
   1. `/reload`
   2. `/wl snapshot StdTest`
   3. Wait for the line `StdTest: snapshot taken; /reload, then wowlua profile snapshot <name>`
      and note whether it says that parts failed.
   4. `/reload`
   5. On the desktop, in `tests/ingame`: `wowlua profile snapshot probe-world --client _forever_`

   Send `tests/ingame/tests/profiles/probe-world.json` and the command's
   output. The file must not name your character or realm, and its
   `trained` must list your spellbook's spells (decision T500): Attack
   (6603) among them.
2. **`/stdtest snapshot` through the runtime's**:
   1. `/stdtest snapshot`
   2. Wait for `snapshot: recorded`, then `/reload`
   3. On the desktop: `cargo xtask profile "<WoW>/_forever_/WTF/Account/<account>/SavedVariables/StdTest.lua" stdtest-world.json`

   Send `stdtest-world.json`.

## The other clients' answers (decisions T510, T511)

Recorded on 2026-10-29 on Retail 12.1.5 (70575), MoP Classic 5.5.4
(70631), TBC Anniversary 2.5.6 (70577) and Classic Era 1.15.9 (70576),
with a character alone, no target and out of combat (a Death Knight on
Retail and MoP Classic). Run each step again on each of those clients when
they change, and note what it prints. "wowlua:" is what the engine answers
now; "Recorded:" what the clients printed.

1. **Functions the documentation lacks**:
   1. `/dump type(GetQuestLogTitle), type(GetGlyphSocketInfo), type(GetReadyCheckStatus)`

   wowlua: Retail `nil nil function`; MoP Classic `function function
   function`; TBC and Classic Era `function nil function`.
   Recorded: the same.
2. **Nothing there**:
   1. `/dump IsTargetLoose(), C_ActionBar.GetBonusBarOffset(), UnitIsTapDenied("target"), GetRaidTargetIndex("target")`
   2. `/dump select("#", GetReadyCheckStatus("player")), C_TTSSettings.GetSpeechRate(), C_TTSSettings.GetSpeechVolume()`

   wowlua: `false 0 false nil` and `0 0 100` on every client.
   Recorded: the same.
3. **Expansions** (after the login, as the server's is 0 before):
   1. `/dump GetExpansionLevel(), GetServerExpansionLevel(), GetAccountExpansionLevel(), GetMaximumExpansionLevel(), GetMinimumExpansionLevel()`
   2. `/dump GetClassicExpansionLevel and GetClassicExpansionLevel()`

   wowlua: Retail `11 11 11 11 10`; MoP Classic `4 4 0 4 3`; TBC `1 1 0 1
   5`; Classic Era `0 0 0 0 5`.
   Recorded: the same, and `GetClassicExpansionLevel()` 11, 4, 1 and 0.
4. **Class colours**:
   1. `/run for _, c in ipairs({"WARRIOR", "SHAMAN", "MONK", "DEATHKNIGHT", "EVOKER"}) do local x = C_ClassColor.GetClassColor(c) print(c, x and x.r, x and x.g, x and x.b) end`

   wowlua: Retail's colours where the client has the class (Classic Era's
   Shaman the Paladin's pink), nothing for a class it lacks.
   Recorded: the same values, to the last digit.
5. **Refused functions**:
   1. `/run for _, n in ipairs({"GetTalentGroupRole", "GetNumTalentTabs", "GetNumTalentGroups", "GetUnspentTalentPoints", "GetNumSpecGroups", "GetTalentTierInfo"}) do print(n, pcall(_G[n], 1)) end`

   wowlua: on MoP Classic the first four `false Script_<name>: API
   unsupported in this version of World of Warcraft.`; on TBC and Classic
   Era `GetTalentGroupRole`, `GetNumSpecGroups` and `GetTalentTierInfo`.
   Recorded: the same. The others answer: on Retail the first four do not
   exist, `GetNumSpecGroups(1)` is 0 and `GetTalentTierInfo(1)` nothing (as
   on MoP Classic, and in wowlua); on TBC and Classic Era `GetNumTalentTabs`
   3, `GetNumTalentGroups` 1 and `GetUnspentTalentPoints` the character's
   (0 at level 1, 1 at level 11), which a test mocks.
6. **Runes and weapon enchants** (the classic clients):
   1. `/run for i = 1, 7 do print(i, GetRuneType(i)) end`
   2. `/run print(select("#", GetWeaponEnchantInfo()))`

   wowlua: `1 1 3 3 2 2` and nothing for rune 7; 8 values on MoP Classic,
   12 on TBC and Classic Era.
   Recorded: the same. On Retail `GetRuneType` does not exist, and
   `GetWeaponEnchantInfo` gives 12 values: Blizzard's deprecation fallback
   (`Blizzard_Deprecated`, `Deprecated_12_1_0.lua`) defines it, not the
   client, so wowlua's `standard` has none without Blizzard's code.
7. **Hardcore** (the classic clients):
   1. `/dump C_GameRules.IsHardcoreActive()`

   wowlua: `false` (a test sets a Classic Era hardcore realm with
   `mock.setHardcore(true)`).
   Recorded: `false` on the classic clients; on Retail the function does
   not exist.

## The untyped functions (M11's full API types)

About 55 functions (2026-11-01) are still `(...unknown) -> ...unknown`:
no source types them (warcraft.wiki.gg has no pages for them, Ketho's
annotations no types, Blizzard's Lua calls only a few). These steps record
what the getters among them answer, so the type generator can type them.
Only getters: the functions that act (`DoCraft`, `StartAuction`,
`GMSubmitSuggestion`, the combat log's filters, …) are not called. Run each
step on the clients named, with a character alone, no target and out of
combat, and note what it prints. Every step starts with the helper `P`,
which calls a function safely and prints each answer with its type
(`number=0.05`); `false` after the name is the error the call raised.

1. **Arena teams and the auction house** (Classic Era, TBC Anniversary,
   MoP Classic):
   1. `/run P=function(n,...) local function f(ok,...) local t={} for i=1,select("#",...) do local v=select(i,...) t[i]=type(v).."="..tostring(v) end print(n,ok,table.concat(t," ")) end f(pcall(_G[n],...)) end`
   2. `/run P("GetAuctionHouseDepositRate") P("GetArenaTeamRosterShowOffline") P("GetPreviousArenaSeasonUsesTeams") P("IsInArenaTeam") P("IsRatedBattleground")`

   wowlua: untyped. Recorded: (to do)
2. **Spells by ID** (Classic Era, TBC Anniversary, MoP Classic; 6603 is
   Auto Attack):
   1. `/run P=function(n,...) local function f(ok,...) local t={} for i=1,select("#",...) do local v=select(i,...) t[i]=type(v).."="..tostring(v) end print(n,ok,table.concat(t," ")) end f(pcall(_G[n],...)) end`
   2. `/run P("GetSpellSubtext",6603) P("GetSpellRank",6603) P("IsHelpfulSpell",6603) P("IsHarmfulSpell",6603) P("SpellHasRange",6603)`
   3. `/run P("IsAutoRepeatSpell",6603) P("IsConsumableSpell",6603)`

   wowlua: untyped. Recorded: (to do)
3. **Spells by their place in the spellbook** (the same clients):
   1. `/run P=function(n,...) local function f(ok,...) local t={} for i=1,select("#",...) do local v=select(i,...) t[i]=type(v).."="..tostring(v) end print(n,ok,table.concat(t," ")) end f(pcall(_G[n],...)) end`
   2. `/run P("GetSpellSubtext",1,"spell") P("IsHelpfulSpell",1,"spell") P("IsHarmfulSpell",1,"spell") P("SpellHasRange",1,"spell") P("IsAutoRepeatSpell",1,"spell")`

   wowlua: untyped. Recorded: (to do)
4. **Quests** (the same clients; with a quest in the log and tracked, and
   once with none):
   1. `/run P=function(n,...) local function f(ok,...) local t={} for i=1,select("#",...) do local v=select(i,...) t[i]=type(v).."="..tostring(v) end print(n,ok,table.concat(t," ")) end f(pcall(_G[n],...)) end`
   2. `/run P("GetQuestWatchInfo",1) P("IsQuestHardWatched",1) P("QuestPOIGetQuestIDByIndex",1) P("IsUnitOnQuestByQuestID",1,"player")`

   wowlua: untyped. Recorded: (to do)
5. **Professions and talents** (the same clients; once with a profession
   window open):
   1. `/run P=function(n,...) local function f(ok,...) local t={} for i=1,select("#",...) do local v=select(i,...) t[i]=type(v).."="..tostring(v) end print(n,ok,table.concat(t," ")) end f(pcall(_G[n],...)) end`
   2. `/run P("GetTradeSkillItemLevelFilter") P("GetTradeSkillItemNameFilter") P("GetPreviewTalentPointsSpent") P("GetPreviewTalentPointsSpent",1) P("GetArenaTeamGdfInfo")`

   wowlua: untyped. Recorded: (to do)
6. **The combat log's entries** (MoP Classic and Retail):
   1. `/run P=function(n,...) local function f(ok,...) local t={} for i=1,select("#",...) do local v=select(i,...) t[i]=type(v).."="..tostring(v) end print(n,ok,table.concat(t," ")) end f(pcall(_G[n],...)) end`
   2. `/run P("CombatLogGetNumEntries") P("CombatLogGetNumEntries",true) P("CombatLogGetCurrentEntry")`

   wowlua: untyped. Recorded: (to do)

## Answered by the batched run (2026-11-09)

`/stdtest engine owed` on WoW Forever 1.60.1.70862 (a level 36 paladin,
AtlasLoot and SuuUI installed), compared with the emulator's run of the
same steps (`wowlua test` with Blizzard's UI, camelot, the default world: a
level 1 warrior, no addons; with `--game-data` where a game file's ID
counts). "wowlua" is what 0.13.9 answered, "now" what it answers since.
Float noise (`50.000061035156` for 50) is left out of the comparison.

Owed from the M12 screenshots:

- 1 (T354): AtlasLoot's seven load-on-demand modules each `false
  [DEMAND_LOADED]` (`loadable` false), the loaded ones too (`…_Data`,
  `…_DungeonsAndRaids` `true true`); `LoadAddOn("AtlasLootClassic_Crafting")`
  `true [DEMAND_LOADED]`, the module then `false [DEMAND_LOADED] true
  true`. wowlua: `loadable` true and the reason empty once loaded; now as
  the game.
- 2: `nil nil` (no `SetDesaturation`), as wowlua.
- 3 (T337): `50 50 TOPLEFT`. wowlua `50 50 RIGHT`; now `TOPLEFT`
  (`GetPoint` in `Enum.FramePoint`'s order).
- 4 (T338): `GameFontRed`, `false Button:SetHighlightFontObject(): Usage:
  ("fontname" or fontObject)`. wowlua's error was the documented usage;
  now the game's words.
- 7 (T343): `1 6 7`, `11 16 17`, as wowlua.
- 8 steps 1-2 (T347): `15.000000953674 11.666667938232`,
  `53.289131164551 19.122459411621`; wowlua `16 12`, `56 12`. Open (item
  8 above).
- 11 (T350): `true 374226` for `.blp`, `.tga`, `.png`, `.jpg` and none.
  wowlua (with the installation): `.jpg` nil; now 374226.
- 14 (T346): `nil nil nil`, as wowlua.
- 16 steps 1-2 (T342): `250.88012695312 79.262634277344`, the same a
  frame later, 3 lines; wowlua `320 78`, 3 lines. Open (item 16 above).
- 17 (T345): ranks 5 to 14 `5250 6750 8400 10200 12150 14250 16600 19100
  21750 24750`; ranks 2 to 6 one reward each, `Rank 2 Rewards` to `Rank
  6 Rewards`. wowlua 16500, 18900 and 21450 for 11 to 13, no rewards; now
  as the game.
- 18 step 2: `month 11 2026 30`; Hallow's End to the 2nd, Call to Arms
  (Darkspear Islands, Arathi Basin, Warsong Gulch, a week each in turn),
  Darkmoon Faire the 9th to the 16th, Stranglethorn Fishing Extravaganza
  each Sunday (no sequence type), the same after `OpenCalendar`. wowlua
  none. Open (item 18 above).
- 19 step 7: `0 15`, as wowlua; step 9: `100.00003051758 true true`;
  wowlua `100 false false`; now `100 true true` (`IsRectValid`, T605).
- 20d steps 1-2 (T365): `false true true`, `true false true`, as wowlua;
  step 3 without the character frame: `true true true`; wowlua `true false
  false`; now as the game.
- 20e step 3: `0 true`, as wowlua; `select("#", BNGetInfo())` mid-session
  0; wowlua 8. Open (item 20e above).

Owed from M13:

- 1 (T393): 250.87994384766 and 229.17562866211, the same a frame
  later; wowlua 298 and 320. Open (M13 item 1).
- 2 (T391): `PALADIN 36`, `17 25`; Consecration "… an additional 38
  damage …", Holy Light "Heals a friendly target for 43 to 52." Not
  compared (M13 item 2).
- 3: `GetCurrencyInfo(1792)` Honor Points' table (quantity 0, max 25000),
  as wowlua; `(1901)` nothing, as wowlua; `GetFactionDataByID(69)` and
  `(930)` nil (wowlua's warrior has Darnassus: the character's);
  `GetMountInfoByID(6)` `Brown Horse, 458, 132261, false, false, 0,
  false, true, 1, false, false, 6, false`; wowlua nothing; now these
  values (T608).

Owed from M14:

- 1 (T400): `before 221427.597`, `event nil nil 221427.597`, `after
  221427.597 true`; a frame later shown, then closed. As wowlua.

Owed from the contributions in 0.13.5:

- 1 (#81): slots 1 to 8 `7 false  0 0 nil 0 0`, 0 and 9 `0`, as wowlua.
- 2 (#82): `[1]={}`, as wowlua; `[1]={anchorOffsets={bottom=0, left=0,
  right=0, top=0}, backdropInfo={}}`; wowlua filled `backdropInfo`'s
  `insets`; now as the game (T608).
- 4 (T486): step 1 `1.0499999523163 0 0 0 0 0`; step 2 `7000 7000 0
  false`, `0 false`, `33000 true`, `44999.995231628 33000 60000`; step 3
  `12000 8000 4000 false`, `5000 false`, `28000 true`. wowlua's overflow
  1 (`40000 33000 60000`); now 1.05, as the game.
- 6 (T487): the bar 0 to 0 and 0 throughout; wowlua followed the timer.
  Open (0.13.5 item 6).

Still to run:

- `GetBindingText(key, 1)` with modifiers: `SHIFT-1 s-1 s-1 SHIFT-1`,
  `CTRL-ALT-2 c-a-2 c-a-2 CTRL-ALT-2`, `META-E m-E m-E CMD-E`,
  `SHIFT-MOUSEWHEELUP s-Mouse Wheel Up s-Mouse Wheel Up SHIFT-Mouse Wheel
  Up`, `SHIFT-NUMPAD1 s-Num Pad 1 s-Num Pad 1 SHIFT-Num Pad 1`, the keys
  alone as themselves (`1`, `2`, `E`, `Mouse Wheel Up`, `Num Pad 1`), as
  wowlua; the modifiers alone `SHIFT s SHIFT`, `CTRL c CTRL`, `ALT a
  ALT`, `META m CMD`; wowlua kept them whole (`SHIFT SHIFT SHIFT`); now as
  the game (T608).

Decision T605's open questions (SuuUI's case G):

- `G` `place | now nil | next nil | 1s nil`; `I` and `M` `place | size
  55x30 | now 112,-140 | next 112,-140 | 1s 112,-140`: as recorded
  before, and as wowlua.
- `Gq` `place | next nil | 1s nil`, as wowlua.
- `Gh` `place | now nil | show | shown nil | next nil | 1s nil`, as
  wowlua.
- `Gh2` `place | show | next nil | 1s nil`; wowlua placed it (`place |
  size 55x30 | show | next 112,-140 | 1s 112,-140`); now lost, as the
  game (the layout pass lays out the frames anchored since first).
- `T` `body asks 1.00002 | place | body asks 30 | size 55x30 | now
  112,-140 | next 112,-140 | 1s 112,-140`, as wowlua.
- `W` `place | body 30 | size 55x30 | now 112,-140 | next 112,-140 | 1s
  112,-140`, as wowlua.

Other sections:

- Widget state (T289) 1-3: `1 7`, `1`, `1 7`; 4: `HELLO world 5`; 11:
  `Fonts\FRIZQT__.TTF 13.999999046326 OUTLINE`, `1 0 0 1`; 12: `15 15`,
  `0 0`: as wowlua. 5: `after Insert`, `changed true`; 6: `3`, then
  `changed false`; 7: `abcdZ`, then `changed true` (one call). wowlua
  `changed false` for the `Insert`s and `aZef`; now as the game. 13:
  the texture, `true true`; wowlua `false` for `IsShown`; now shown. 14:
  `nil`; wowlua 0; now nil.
- Targeting and history (T291) 3: `0`; Leftovers (T295) 4: "probe line"
  and `-1 0 1`: as wowlua.
- Model frames' cameras 3: recorded, open (that section).
- Textures the screenshots draw 1: `nil 137012`, as wowlua with the
  installation. 4 (SuuUI has no `media\logo`): `true nil nil nil`; wowlua
  gave the path for `GetTextureFilePath`, now nil; `nil`; `8063723
  8063723 8063723` (wowlua's path nil, now the atlas's file ID); `nil nil
  nil`; `nil 130871`, as wowlua with the installation.
- Unanchored textures 1 (Lua): `0 40 20`, as wowlua.
- The Camelot update 5: alpha 1; wowlua 0; now 1. 6: `true nil true`;
  wowlua `true nil false`; now as the game.
- Unanchored font strings (T602) 1: `1 1`; wowlua `2 2`. Open (that
  section).
- Names over units 1: `UnitNameNPC 0`, `UnitNameInteractiveNPC 1`,
  `UnitNameHostleNPC 1`, `UnitNameFriendlySpecialNPCName 1`,
  `UnitNameNonCombatCreatureName 0`, `nameplateShowAll 1`,
  `nameplateShowEnemies 1`, `nameplateShowFriendlyNpcs 0`,
  `nameplateMaxDistance 45.000000`: the client's defaults. wowlua's
  default world has the three at 0 (the recorded fresh character's
  settings), the rest the same; not a rule.
- How far the world is drawn 1: `farclip 7000`, `terrainLodDist 500`,
  `horizonStart 1900`, `lodObjectCullSize 18`, `lodObjectMinSize 0`,
  `wmoLodDist 400`, as wowlua.
- Found with SuuUI 2, 7 and 9: in their items.

## Answered on the beta's last day (2026-10-24)

- PvP ranks (major faction 2800): `GetMajorFactionProgressionInfo` gives
  maxLevel 14, renownLevel 0, renownLevelThreshold 750, the weekly fields
  0 and weekNumber -1 on a rank 0 character; ranks 1 to 4 total 750, 1650,
  2700, 3900; rank 1's reward is the Faction Tabard (decision T345).
- A frame anchored to UIParent has its width and left at once and on the
  next frame alike (200, 100).
- A frame anchored inside a frame that is sized but not placed reports
  width 0 (the parent 200): `child 0 200` (decision T346).
- A texture with `SetAtlas(atlas, true)`, a `LEFT` point and `SetWidth(0)`
  reports width 0 and its atlas's height.
- The macro window shows no "Create Macros" title in game either.
- `|cAARRGGBB`'s alpha could not be checked from the chat box (it escapes
  `|`); decision T342 stays a proposal.
- Still owed: the skill bars' fill (item 19 of "Owed from the M12
  screenshots").

## Moving and facing (decisions T516, T519)

Unverified numbers and answers from M13's movement and ranges. A paladin
(or any class with an instant harmful spell that needs facing, and a heal).

1. **Speeds**: stand still and enter
   `/run print(GetUnitSpeed("player"))`, then press the walk key (`/` on
   the numpad by default) and enter it again while walking forward. Answers
   the run and walk speeds (wowlua: 7 and 2.5).
2. **A spell at a target behind you**: enter
   `/run local f=CreateFrame("Frame") f:RegisterEvent("UI_ERROR_MESSAGE") f:SetScript("OnEvent",function(_,_,c,t) print(c,t) end)`,
   target a hostile mob within 10 yards, turn your back to it and press
   Judgement (or the class's instant attack spell). Answers the code and
   text (wowlua: 57, "Target needs to be in front of you.").
3. **A heal with a far hostile target**: with the frame from step 2, target
   a hostile mob more than 45 yards away and press Flash of Light (or the
   class's heal). Note whether it heals you or prints "Out of range."
   (wowlua: it heals you).
4. **The bars' range at login**: `/reload` with a hostile mob 10 yards away
   targeted, then enter
   `/run local f=CreateFrame("Frame") f:RegisterEvent("ACTION_RANGE_CHECK_UPDATE") f:SetScript("OnEvent",function(_,_,s,i,c) print(s,i,c) end)`
   and walk away from it until a button's hotkey turns red. Answers the
   slots told and their values (wowlua: each slot's `false true` once).

## A solid world (decisions T524, T532)

1. **Line of sight**: enter
   `/run local f=CreateFrame("Frame") f:RegisterEvent("UI_ERROR_MESSAGE") f:SetScript("OnEvent",function(_,_,c,t) print(c,t) end)`,
   target a mob with a wall or hill between you and press an attack spell
   in range. Answers the code and text (wowlua: 57, "Target not in line of
   sight").
2. **A step's height**: walk onto a crate or a ledge and note the highest
   one you step onto without jumping (wowlua: 0.6 yards, to the knee).
3. **The camera and the trees**: zoom the camera out, then turn it so a
   tree's trunk stands between it and your character. Note whether the
   camera moves in front of the trunk, as it does for a wall (wowlua: it
   does not).

## Names over units (decision T534)

Any character, outdoors near a town's NPCs, with nameplates for friendly
NPCs off.

(Step 1, the settings, was answered by the batched run, 2026-11-09:
"Answered by the batched run".)

2. **A guard**: stand by a guard (a friendly NPC with nothing to sell)
   and note whether its name shows, then enter
   `/run SetCVar("UnitNameFriendlySpecialNPCName", 0)` and note whether
   it still shows (wowlua: it goes).
3. **How far**: walk away from a vendor until its name goes, then enter
   `/run print(UnitPosition("player"))`; walk back to it and enter the
   same. Answers how far names show (wowlua: as far as nameplates, 45
   yards).
4. **Plates out of combat**: enter `/run SetCVar("nameplateShowAll", 0)`
   and look at a hostile mob out of combat: note whether its plate shows
   (wowlua: not until it is in combat). Enter
   `/run SetCVar("nameplateShowAll", 1)` after.
5. **The look**: take a screenshot (Print Screen) of a vendor's name and
   title close up, for their size, colors and height over its head
   (wowlua: 15 and 13 pixels at 1080, green, 2.4 yards up).

## How far the world is drawn (decision T525)

(Step 1, the settings, was answered by the batched run, 2026-11-09: as
wowlua.)

2. **Small things far away**: stand at Northshire's start facing the
   abbey, then walk back from it. Note at about which distance the
   lanterns and crates disappear (wowlua: where they look smaller than 18
   pixels).

## Frame rate and render scale (decisions T538, T539)

1. **The render scale's limits**: enter
   `/run SetCVar("RenderScale","0.1") print(GetCVar("RenderScale"))`,
   then `/run SetCVar("RenderScale","4") print(GetCVar("RenderScale"))`,
   then `/run SetCVar("RenderScale","1")`.
   Answers whether the client keeps the setting to 0.33 to 2 (wowlua does).
2. **How GetFramerate averages**: enter
   `/run SetCVar("maxFPS","30") C_Timer.After(2,function() print(GetFramerate()) end)`,
   then `/run SetCVar("maxFPS","120") C_Timer.After(0.1,function() print(GetFramerate()) end)`.
   Answers whether the frame rate follows at once (one frame's time) or
   over a second (wowlua: the last second's frames).
3. **The background's frame rate**: enter
   `/run C_Timer.After(5,function() print(GetFramerate()) end)`, then click
   another window within the 5 seconds and wait. Answers the frame rate in
   the background (wowlua: `maxFPSBk`, 30).

## The camera (decisions T520, T521)

The camera's numbers wowlua could not read from the settings. Any
character, outdoors.

1. **The furthest zoom**: scroll all the way out, then enter
   `/run print(GetCameraZoom(), GetCVar("cameraDistanceMaxZoomFactor"))`.
   Answers the furthest zoom (wowlua: 15 times the factor).
2. **Zooming at once**: enter
   `/run CameraZoomOut(5) print(GetCameraZoom())`. Answers whether the
   distance changes at once (wowlua) or as the camera glides.
3. **A view never saved**: enter `/run ResetView(3) SetView(3)`. Note where
   the camera goes (wowlua: it stays).
4. **A nameplate's height**: target a mob 20 yards ahead on flat ground,
   then enter
   `/run local p=C_NamePlate.GetNamePlateForUnit("target") print(p:GetCenter()) print(WorldFrame:GetCenter())`.
   Answers how far above the screen's middle its plate sits (wowlua: the
   pivot 2 yards over the player's feet, the plate 2.5 over the mob's).

## The dev tools (0.12.1)

The dev window's performance tab, hints, HUD and error toast are tested in
the emulator (`crates/wowlua-cli/tests/devtools.rs`); these check what the
emulator cannot: the fonts, the look and the real frame times. StdTest's dev
build has them all (`wowlua install` in `tests/ingame`).

1. **The window in the design system's fonts**: `/wl`, then every tab.
   Titles in Martian Mono, everything else in IBM Plex Mono; no empty boxes
   (a glyph the fonts lack). A screenshot of the performance tab's
   functions view and of the flame view (switch call paths on first).
2. **A combat and a boss pull**: fight something, then a boss in a
   dungeon; the recordings list gets `combat N` and `pull N` with the boss's
   name. Compare the pull with `live` (right-click): the change column is
   per minute.
3. **The HUD**: `/wl hud`, move it, `/reload`: it comes back where it was.
   It hides while the window is open and during a cinematic, not in a pet
   battle.
4. **The toast**: an error from StdTest's own code (any probe that fails;
   `/run error("x")` is not the addon's and shows none) shows the toast at
   the top for 8 seconds, longer with the cursor on it; in combat it waits
   for the end of combat. Clicking it opens the errors tab.
5. **Export**: mark the pull for export, `/reload`, and check that
   `WTF/Account/<account>/SavedVariables/StdTest.lua` holds
   `WowluaDevDB_StdTest` with `exports`; `wowlua prof report` reads it.

## The Camelot update (decision T553)

On WoW Forever: from its launch on 5 November 2026 (no login before).

The declarations and Blizzard's UI code now come from the installed client
(1.60.1.70707 or newer), but the engine's recordings are of 70425: its
`Enum` corrections, CVars, number globals, colors, spells, talents and
trainers. These steps record them again on the updated client.

1. **Update the client**: start the Battle.net launcher, let it update WoW
   Forever, and log in a character once (the update is then on disk).
2. **Install StdTest**: in `tests/ingame`, `wowlua install --client _forever_`,
   then `/reload` in game.
3. **The engine**: `/stdtest engine`, wait for it to finish, then `/reload`.
4. **Take it in**: `cargo xtask ingest`, then `cargo xtask types fetch` and
   `cargo xtask types gen`.

   wowlua: `engine_probe`'s `NEWER_BUILD` lists what 70658 and 70707
   changed (70707: `Enum.VasTransactionPurchaseResult.ProxyNoMixedAlliance`,
   and the documentation's `Constants.LevelConstsExposed_Camelot`, its
   `MIN_RES_SICKNESS_LEVEL` 11; the engine keeps the recorded
   `LevelConstsExposed`, 10, 10, 10, which 70707's micro menu reads,
   decision T576). The trees' and transmog sources' tests (`wowlua-render`,
   `CHANGED`) list what 70707's tables changed: the mage tree, two
   sources' appearances. And what 70707's server changed, in the client's
   hotfix cache since the game reached its login screen on 2 November:
   the paladin's, druid's and hunter's trees (`CHANGED`, "70707's
   hotfixes"; the paladin's new Shieldsworn), the paladin's blessings'
   and Holy Shield's costs (`wowlua-casc` install, `NEWER_COSTS`), five
   spells' base cooldowns and an achievement's trait currency
   (`wowlua-render` answers, `NEWER`), the 24 nameplate atlases back as
   70658 had them. The same lists hold what 70755, 70862 and 70888
   changed (70862: four transmog sources' appearances, Thorns' damage in
   `NEWER_DESCRIPTIONS`, decision T604; 70888: Coarse Grinding Stone's
   `maxTrivialLevel` in `wowlua-render` tradeskill's `NEWER`, the
   inventory's `ShouldShowBattlePetIcon` in `NEWER_BUILD`, the lfg
   scenario's `PARTY_INVITE_REQUEST` with its new `isCrossShard` in
   `engine_probe.rs` `NEWER_PAYLOADS`, decision T627).
   Recorded: (to do)
5. **One color** (decision T554): `/run for _,c in ipairs(C_UIColor.GetColors()) do if c.baseTag=="COMPACT_UNIT_FRAME_FRIENDLY_HEALTH_COLOR" then print(c.color:GetRGBA()) end end`

   wowlua: `0.16862745583057 0.57647061347961 0.019607843831182 0`, the
   installed table's alpha (0); 70425 answered 1. Recorded 2026-11-09
   (70862, the batched run): `0.16862745583057 0.57647061347961
   0.019607843831182 1`; wowlua now answers 1 (decision T554).
6. **A cleared normal texture** (decision T576): `/console scriptErrors 1`,
   then `/run local b=CreateFrame("Button") b:SetNormalTexture("Interface\\Buttons\\WHITE8X8") local t=b:GetNormalTexture() b:ClearNormalTexture() print(b:GetNormalTexture()==t, t:GetTexture(), t:IsShown())`

   wowlua: `true nil false` (inferred: 70707's item buttons mask their
   normal texture after their `OnLoad`, in which the professions' recraft
   slot clears it). Then open a profession's window and note whether
   `ItemButtonTemplate.lua:17: attempt to index a nil value` shows.
   Recorded 2026-11-09 (70862, the batched run, the print only): `true nil
   true`; wowlua now answers the same (decision T576). The profession
   window's error still owed.

## The Classic clients' data (decision T554)

Each Classic client answers for itself: run every step on MoP Classic,
TBC Anniversary and Classic Era, with a character alone, no target and out
of combat, and note what each prints. "wowlua:" is what the engine answers
now.

1. **Locales**:
   1. `/run for i,l in ipairs(GetAvailableLocaleInfo()) do print(i,l.localeId,l.localeName) end`

   wowlua: nothing (no answer yet on these clients). Recorded: (to do)
2. **Loot rolls and product choices**:
   1. `/dump C_LootHistory.GetNumItems(), #C_ProductChoice.GetChoices()`

   wowlua: `0 0` (unverified). Recorded: (to do)
3. **Max level**:
   1. `/dump GetMaxPlayerLevel(), GetMaxLevelForPlayerExpansion(), GetMaxLevelForLatestExpansion()`

   wowlua: MoP Classic `90 90 90`, TBC Anniversary `70 70 70`, Classic Era
   `60 60 60` (the flavor table's). Recorded: (to do)
4. **Skill lines**:
   1. `/dump GetNumSkillLines()`
   2. `/dump GetSkillLineInfo(1)`
   3. `/dump GetSkillLineInfo(2)`

   wowlua: nothing (skills are not modelled). Recorded: (to do)
5. **Functions the dump lacks**:
   1. `/dump type(C_UnitAurasPrivate), type(C_UnitAurasPrivate and C_UnitAurasPrivate.SetShowDispelTypeCallback), type(C_StoreSecure)`

   wowlua: `nil nil nil` (Ketho's dumps of older builds lack them).
   Recorded: (to do)
6. **Constants the client defines**:
   1. `/dump Constants.InventoryConstants.NumGenericBankSlots, LE_GAME_ERR_SPELL_FAILED_TOTEMS, LE_GAME_ERR_SPELL_FAILED_EQUIPPED_ITEM`
   2. `/run local n=0 for k in pairs(_G) do if type(k)=="string" and k:find("^LE_GAME_ERR_") then n=n+1 end end print(n)`

   wowlua: `nil nil nil` and `0`. Recorded: (to do)

## The world's motion (decision T558)

On WoW Forever: from its launch on 5 November 2026 (no login before).

Look, do not run anything: on WoW Forever, stand at Northshire Abbey on a
clear day and watch the trees and bushes about it for ten seconds.

1. **Trees in the wind**: do their leaves or branches sway?

   wowlua: they stand still (their models have no animation; a sway in
   the game would be the shaders' wind). Seen: (to do)

## The Graphics settings (decision T561)

What each Graphics setting's levels set, recorded on each client. It
changes your graphics settings for about a minute and puts every one back
at the end. The Classic clients can be recorded now; WoW Forever from its
launch on 5 November 2026 (no login before).

1. **Classic Era**:
   1. in `tests/ingame`: `wowlua install --client _classic_era_`
   2. in game: `/reload`, then `/stdtest engine graphics`, wait for
      "engine graphics: … lines", then `/reload`
   3. `cargo xtask ingest --client _classic_era_`
2. **TBC Anniversary**:
   1. in `tests/ingame`: `wowlua install --client _anniversary_`
   2. in game: `/reload`, then `/stdtest engine graphics`, wait for
      "engine graphics: … lines", then `/reload`
   3. `cargo xtask ingest --client _anniversary_`
3. **MoP Classic**:
   1. in `tests/ingame`: `wowlua install --client _classic_`
   2. in game: `/reload`, then `/stdtest engine graphics`, wait for
      "engine graphics: … lines", then `/reload`
   3. `cargo xtask ingest --client _classic_`
4. **WoW Forever** (from 5 November):
   1. in `tests/ingame`: `wowlua install --client _forever_`
   2. in game: `/reload`, then `/stdtest engine graphics`, wait for
      "engine graphics: … lines", then `/reload`
   3. `cargo xtask ingest`

   wowlua: `crates/wowlua-engine/data/<flavor>-graphics.txt`; camelot's has
   only the shadow levels seen in Config.wtf files (unverified), the
   Classic clients' none. Recorded: (to do)

## Spells in the world (decision T582)

On Classic Era now (WoW Forever from its launch on 5 November 2026). Look,
do not run anything: with a mage, stand ten yards from a training dummy or
a weak creature, target it and cast Fireball; then have a priest friend
cast Renew on you, or cast it on a party member with a priest.

1. **Fireball's cast**: while the bar fills, do both hands glow?

   wowlua: both (the precast kit's models at points 21 and 22). Seen: (to do)
2. **Fireball's flight**: does it leave from the right hand, the left
   hand, or between them?

   wowlua: from point 38 where the missile names none (Classic Era's
   names -1). Seen: (to do)
3. **Fireball's burst**: as it lands, does anything also show on you (the
   mage), or only on the target?

   wowlua: only on the target. Seen: (to do)
4. **Renew on someone else**: with Renew cast by a priest on another
   player, does the priest glow too while it is up?

   wowlua: only the player with Renew glows. Seen: (to do)
5. **How long a burst shows**: about how many seconds is Fireball's burst
   on the target visible?

   wowlua: its model's first animation, at most 3 seconds. Seen: (to do)

## WoW Forever's water (decision T583)

On WoW Forever: from its launch on 5 November 2026 (no login before).
Look, do not run anything: stand on the bank of the river south of
Northshire Abbey on a clear day at noon, then take a screenshot looking down
at the water and one looking along it.

1. **Its color**: is the deep middle a darker teal than the shallows by the
   bank, or the other way round?

   wowlua: the darker of its type's two colors deep. Seen: (to do)
2. **Its bed**: does the riverbed show through the shallows?

   wowlua: yes, less as it deepens (to 5 yards). Seen: (to do)
3. **Its foam**: is there foam along the banks, and on the open water?

   wowlua: a line of foam where it is under a yard deep, a little on the
   waves' crests. Seen: (to do)
4. **Its waves**: about how big are its ripples (a hand, a foot, a yard
   across), and which way do they drift?

   wowlua: about a foot, drifting slowly each its own way. Seen: (to do)

## The world's shading (decisions T584, T585)

On WoW Forever: from its launch on 5 November 2026 (no login before).
Stand at the foot of Northshire Abbey's steps at noon, the camera behind you
looking at the steps, and take a screenshot after each step.

1. **SSAO off**: `/run C_CVar.SetCVar("SSAO", "0")`
2. **SSAO high**: `/run C_CVar.SetCVar("SSAO", "3")`

   Seen: do the steps' edges, the corners where the steps meet the wall,
   and the ground at the foot of the lantern post darken? wowlua: yes, a
   soft shadow about a yard wide. Seen: (to do)
3. **SSAO back**: `/run C_CVar.SetCVar("SSAO", "0")`
4. **GI off**: `/run C_CVar.SetCVar("giQuality", "0")`
5. **GI on**: `/run C_CVar.SetCVar("giQuality", "1")`

   Seen: does anything change between the two (the undersides of the
   steps and roofs lighter, a warm tint from the ground)? wowlua: the
   whole scene a little brighter, undersides warmer. Seen: (to do)

## The frame-order steps in one run

The next three sections (T587, T588, T586), with steps for a texture's
masks (T596) and a draw layer's case (T597), run as one probe in StdTest; nothing needs typing. Ten seconds after login, once per client
build, it records what the calls answer by itself (nothing shows). Then a
panel at the top of the screen offers the look-and-click steps: squares
appear in the middle, and each step has a button per answer, "None of the
above" among them, with what wowlua draws in grey below. Later asks again
at the next login; a finished run is not offered again on that build
(`/stdtest engine layers` starts it at any time). Everything lands in
`StdTestState.engineLayers`, written at logout or `/reload`. The steps
below stay for running by hand.

## Frames of the same level (decision T587)

On Classic Era now, and again on WoW Forever from its launch on 5 November
2026 (the same steps on each). Two squares at the same strata and level,
overlapping: which is drawn on top, and whether `SetFrameLevel`, `Show`
or `SetParent` changes it. wowlua guessed the one placed later (made, or
its level changed, later) is on top: SuuUI's character window had its
chosen tab's marker over the tab's label in wowlua, the label over it in
the game. WoW Forever (2026-11-07): the one made later, whatever level or
showing changed since; wowlua now does the same, and the character
window's tabs need another look.

1. **Red, made first**: `/run A=CreateFrame("Frame",nil,UIParent) A:SetSize(200,200) A:SetPoint("CENTER") A:SetFrameLevel(50) local t=A:CreateTexture() t:SetAllPoints() t:SetColorTexture(1,0,0)`
2. **Blue, made second**: `/run B=CreateFrame("Frame",nil,UIParent) B:SetSize(200,200) B:SetPoint("CENTER",60,60) B:SetFrameLevel(50) local t=B:CreateTexture() t:SetAllPoints() t:SetColorTexture(0,0,1)`

   Seen: where they overlap, red or blue? wowlua: blue. WoW Forever (2026-11-07): blue.
3. **Red's level set again, the same**: `/run A:SetFrameLevel(50)`

   Seen: red or blue on top now? wowlua: blue. WoW Forever (2026-11-07): blue.
4. **Red's level changed and back**: `/run A:SetFrameLevel(49) A:SetFrameLevel(50)`

   Seen: red or blue? WoW Forever (2026-11-07): blue. wowlua now blue (it said red).
5. **Blue hidden and shown**: `/run B:Hide() B:Show()`

   Seen: red or blue? WoW Forever (2026-11-07): blue. wowlua now blue (it said red).
6. **Red hidden and shown**: `/run A:Hide() A:Show()`

   Seen: red or blue? WoW Forever (2026-11-07): blue. wowlua now blue (it said red).
7. **Gone**: `/run A:Hide() B:Hide()`

Owed (decision T620, 2026-11-10): SuuUI's character window sets its side
tabs' level after making the marker at that level, and the game draws the
tab's label over the marker, though the marker was made later. wowlua now
takes a level set to a new one as placing the frame anew, counted at the
next frame drawn (step 4 set it back in the same `/run`). Steps 1 and 2
again, then each line as its own `/run`:

8. **Red away**: `/run A:SetFrameLevel(49)`
9. **Red back, a frame later**: `/run A:SetFrameLevel(50)`

   Seen: red or blue? wowlua: red.
10. **Gone**: `/run A:Hide() B:Hide()`
11. **SuuUI's tabs, red made first**: `/run T=CreateFrame("Frame",nil,UIParent) T:SetSize(200,200) T:SetPoint("CENTER") T:SetFrameLevel(40) C=CreateFrame("Frame",nil,T) C:SetAllPoints() local t=C:CreateTexture() t:SetAllPoints() t:SetColorTexture(1,0,0)`
12. **Blue made at 43, then red set to 43**: `/run local h=CreateFrame("Frame",nil,T) h:SetAllPoints() h:SetFrameLevel(42) local x=CreateFrame("Frame",nil,h) x:SetSize(200,200) x:SetPoint("CENTER",60,60) local t=x:CreateTexture() t:SetAllPoints() t:SetColorTexture(0,0,1) C:SetFrameLevel(43) print(C:GetFrameLevel(), x:GetFrameLevel())`

    Seen: `43 43`; red or blue where they overlap? wowlua: red.
13. **Gone**: `/run T:Hide()`

## The mouse wheel over frames (decision T588)

On Classic Era now, and again on WoW Forever from its launch on 5 November
2026 (the same steps on each). Which frame the wheel reaches: one taking
only the wheel over a window taking the mouse, then a frame taking the
mouse above it.

1. **The window**: `/run W=CreateFrame("Frame",nil,UIParent) W:SetSize(300,300) W:SetPoint("CENTER") W:EnableMouse(true) local t=W:CreateTexture() t:SetAllPoints() t:SetColorTexture(0,0,0,0.5)`
2. **Wheel only, in it**: `/run S=CreateFrame("Frame",nil,W) S:SetAllPoints() S:SetScript("OnMouseWheel",function(_,d) print("S wheel",d) end)`
3. Put the mouse over the dark square and turn the wheel one notch.

   Seen: does the chat print "S wheel"? wowlua: yes. WoW Forever
   (2026-11-07): yes, the camera did not zoom.
4. **A frame taking the mouse above it**: `/run B=CreateFrame("Frame",nil,UIParent) B:SetSize(100,100) B:SetPoint("CENTER") B:SetFrameStrata("HIGH") B:EnableMouse(true) local t=B:CreateTexture() t:SetAllPoints() t:SetColorTexture(1,0,0,0.5)`
5. Put the mouse over the red square and turn the wheel one notch.

   Seen: does the chat print "S wheel", or does the camera zoom? WoW
   Forever (2026-11-07): "S wheel", the camera did not zoom: a frame
   taking only the mouse does not keep the wheel from one below taking
   it. wowlua now does the same (it zoomed the camera before).
6. **Gone**: `/run W:Hide() B:Hide()`

## Top-level frames (decision T586)

Recorded on TBC Anniversary (2026-11-04); again on WoW Forever from its
launch, the same steps. Whether a top-level frame raises its level as it
shows, as it is clicked, and what `Raise()` does; then whether it is drawn
above another frame's window of a higher level (steps 11 to 17). Each
line in the chat, one after another.

1. `/run X=CreateFrame("Frame",nil,UIParent) X:SetSize(10,10) X:SetPoint("CENTER") X:SetFrameLevel(500)`
2. `/run T=CreateFrame("Frame",nil,UIParent) T:SetSize(10,10) T:SetPoint("CENTER") T:SetToplevel(true) T:Hide() T:Show() print("shown", T:GetFrameLevel())`

   TBC Anniversary: "shown 1". WoW Forever (2026-11-07): "shown 1".
3. `/run T:Raise() print("raised", T:GetFrameLevel())`

   TBC Anniversary: "raised 1". WoW Forever (2026-11-07): "raised 1".
   wowlua now answers 1 (it raised the level above X's, to 501).
4. `/run X:SetToplevel(true) T:Hide() T:Show() print("shown2", T:GetFrameLevel())`

   TBC Anniversary: "shown2 1". WoW Forever (2026-11-07): "shown2 1".
5. **A click**: `/run T:SetSize(200,200) T:EnableMouse(true) local t=T:CreateTexture() t:SetAllPoints() t:SetColorTexture(0,1,0)`, then click the green square, then `/run print("clicked", T:GetFrameLevel())`

   TBC Anniversary: (to do). WoW Forever (2026-11-07): "clicked 1" (1
   before), as wowlua.
6. `/run A=CreateFrame("Frame",nil,UIParent) A:SetSize(200,200) A:SetPoint("CENTER") A:SetFrameLevel(50) local t=A:CreateTexture() t:SetAllPoints() t:SetColorTexture(1,0,0)`
7. `/run B=CreateFrame("Frame",nil,UIParent) B:SetSize(200,200) B:SetPoint("CENTER",60,60) B:SetFrameLevel(50) local u=B:CreateTexture() u:SetAllPoints() u:SetColorTexture(0,0,1)`
8. `/run A:Raise() print("A", A:GetFrameLevel())`

   Seen: the level printed, and red or blue on top where they overlap.
   TBC Anniversary: "A 50", blue on top. WoW Forever (2026-11-07): the
   same.
9. **Gone**: `/run X:Hide() T:Hide() A:Hide() B:Hide()`
10. **SuuUI's bags** (WoW Forever with SuuUI): open the bags, then `/run print(SuuUIBags:GetFrameLevel(), SuuUIBag2Slot1 and SuuUIBag2Slot1:GetFrameLevel(), SuuUIDamageMeter:GetFrameLevel())`

    WoW Forever (2026-11-07): not run, without SuuUI.
11. `/run R=CreateFrame("Frame",nil,UIParent) R:SetAllPoints() M=CreateFrame("Frame",nil,R) M:SetAllPoints()`
12. `/run W=CreateFrame("Frame",nil,M) W:SetSize(200,200) W:SetPoint("CENTER") local t=W:CreateTexture() t:SetAllPoints() t:SetColorTexture(0,0,1)`
13. `/run B=CreateFrame("Frame",nil,R) B:SetSize(200,200) B:SetPoint("CENTER",60,60) local t=B:CreateTexture() t:SetAllPoints() t:SetColorTexture(1,0,0) B:Hide() B:Show() print(W:GetFrameLevel(),B:GetFrameLevel())`

    Seen: red or blue on top. wowlua: "3 2", blue. TBC Anniversary:
    (to do). WoW Forever (2026-11-07): "3 2", blue.
14. `/run B:SetToplevel(true) B:Hide() B:Show()`

    Seen: red or blue on top. wowlua: red. TBC Anniversary (the same
    window made top-level before it showed): red, answering 2. WoW
    Forever (2026-11-07): red, "3 2".
15. `/run B:EnableMouse(true) W:EnableMouse(true) W:SetToplevel(true) W:Hide() W:Show()`, then click the red square where they do not overlap

    Seen: red or blue on top after the show, then after the click.
    TBC Anniversary: (to do). WoW Forever (2026-11-07): blue, then still
    blue, "3 2": a top-level frame clicked is not raised. wowlua now does
    the same (it raised it: red).
16. `/run R:Hide()`

## Found with SuuUI on WoW Forever (2026-11-06)

Where SuuUI's tests passed in wowlua and the game did otherwise (found in
game, from SuuUI's error log and screenshots, 70755). Each is a gap in the
engine to close, with the steps that would record the game's answer.

1. **A frame laid out while a size script resizes what its anchor hangs
   on is never placed** (answered 2026-11-09, WoW Forever 1.60.1.70862;
   decision T605, wowlua follows it). First seen in SuuUI's bags (the
   window's Bags/Bank switch and toggles, anchored through a row frame,
   shown but `GetLeft` nil and not drawn on the first open after a
   reload) and in keybind mode's Done button. Taken first for "a frame
   anchored before its parent had a place is never placed": the probes
   disproved that. `/suu probe anchors` (plain frames) and `anchors2`
   (buttons): a box anchored to a row or parent before they had points, a
   parent with a width but no height whose height comes the same frame or
   a frame later, a sized parent whose point comes a frame later, a button
   on a placed parent, a body anchored `TOPLEFT` and `TOPRIGHT` 1x1 whose
   height is set right after a button anchors to it: all placed (some only
   once the parent had its point or height). `panelanchors` and
   `panelanchors2` (SuuUI's titled panel and layout stack, as keybind
   mode): the button stayed unplaced with the body's `OnSizeChanged`
   fitting the panel's height; with a fixed body or panel height it was
   placed, built hidden and shown a frame later too.

   `anchors3` and `anchors4`, the replica (all in one slash command, all
   shown): `P` a panel on UIParent; `body` a child of `P` at
   `TOPLEFT` (12, -40), `SetSize(1, 1)`, `P:SetWidth(320)`, `TOPRIGHT`
   (-12, -40); `fit()` sets `P`'s height to 40 + the body's height + 12;
   `body:SetScript("OnSizeChanged", fit)`; `P:SetHeight(53)`; `item` a
   Button child of `body`, `SetSize(55, 30)`, an `OnSizeChanged` hook
   logging; `P:SetPoint("TOPLEFT", x, y)`, `P:Show()`. `place()`:
   `item:ClearAllPoints()`, `item:SetPoint("TOPLEFT", body, "TOPLEFT")`,
   `body:SetHeight(30)`, run in a driver's `OnUpdate` a frame later; then
   the item's `GetLeft`/`GetTop` at once ("now"), `C_Timer.After(0)`
   ("next") and `After(1)` ("1s"). Seen:
   - G (as above): nil, nil, nil; the item's `OnSizeChanged` never ran.
   - H (a Frame for the item): nil, nil, nil.
   - I (no body script): the item's `OnSizeChanged` 55x30 ran inside the
     `OnUpdate`, between `place()` and the "now" line; placed, stays.
   - J (`place()` in the build itself): now nil; the item's
     `OnSizeChanged` 55x30 later; placed the next frame and after 1 s.
   - K (G, the item's `GetLeft` asked before it was anchored): as G.
   - L (G without the item's hook): as G.
   - M (no body script; `fit()` called in `place()` after the height):
     as I.
   - N (an empty body script): as I.
   - O (G with `body:SetHeight(30)` before the item is anchored): as G.
   - P (the body script defers `fit` with `C_Timer.After(0)`): as I.

   The rule: a rectangle is laid out when asked for and in the layout
   pass at the frame's end, `OnSizeChanged` running then. When a frame's
   `OnSizeChanged`, run while a frame anchored to it is laid out, resizes
   the frame it hangs on, the frame being laid out, if it had no place
   since it was anchored, is left without one until it is anchored again
   (SuuUI's fix anchors it anew later), or only until the next pass when
   that was the resizing frame's first rectangle (J). wowlua's
   `crates/wowlua-engine/tests/frames.rs` replays G to P. Left open: the
   order the end-of-frame pass takes frames in (a resize nobody asked for
   before the pass), whether a frame laid out for itself (the body's own
   `GetSize` in a script while the item waits) loses the item too (wowlua:
   no, only the frame whose layout was asked for), whether a hidden frame
   is laid out (`panelanchors2` built hidden placed the button), the
   nesting bound.
2. **Text measures taller in the game.** SuuUI's own fonts (Figtree
   caption 11, label 12, body 13) wrap and stand taller in the game than
   wowlua measures them: two lines of a toy's description ran into the
   card's bottom row, and the character window's skill details (the game's
   side pane rows given SuuUI's font) ran past the window. wowlua: both
   fitted. Steps (the numbers to compare with the engine's):
   - `/run local f=UIParent:CreateFontString(nil,"OVERLAY") f:SetFont("Interface\\AddOns\\SuuUI\\Media\\Fonts\\Figtree-Medium.ttf",11,"") f:SetWidth(160) f:SetText("Throw a handful into the campfire, transforming it into a violet flame.") print(f:GetStringHeight(), f:GetNumLines(), f:GetUnboundedStringWidth())`

     Seen (2026-11-09, 70862, the batched run): `SetFont true`, then
     `32.5 3 368.33334350586`. Not compared yet: wowlua's run of the
     batch has no SuuUI (its font missing: `33 3 380` with the fallback);
     owed: the same line in SuuUI's tests.

     Compared (2026-11-10, decision T619): the numbers are a pixel of 5/6
     (Figtree at 13 pixels), and the font file's glyphs by T392's rule give
     the game's to the pixel: 442 wide, three lines of 13. wowlua measured
     an addon's font as Friz Quadrata; it now measures it with the addon's
     file (`crates/wowlua-engine/tests/frames.rs` replays the line).
3. **A text's height can be a secret.** In a post-hook of a character
   window side pane's `LayoutRows` (Skills, Reputation or PvP rank), a
   wrapped row's `Label:GetStringHeight()` was a secret number ("attempt to
   perform numeric conversion on a secret number value (execution tainted
   by 'SuuUI')"). wowlua: a plain number. Which pane and text: unknown; SuuUI
   now reads it through `secret.asNumber`.
4. **`SetPassThroughButtons` on an addon's own button, in combat.** SuuUI's
   world map made a map pin (its own `Button`) while the map opened in a
   fight and set `SetPassThroughButtons("RightButton")`: "Blocked in combat:
   UNKNOWN()". The minimap's and the guild window's code had found the same
   before. To check in wowlua: whether the engine blocks it in combat
   (SuuUI's tests never did it in combat, so nothing showed).
5. **Using a bag item from an addon-made item button.** SuuUI's bags make
   their slots with `CreateFrame("ItemButton", name, parent,
   "ContainerFrameItemButtonTemplate")` from SuuUI's code. A right-click on
   one runs the template's `ContainerFrameItemButton_OnClick`, which calls
   the button's mixin methods (`self:GetBagID()`): written as SuuUI's while
   it made the button, they taint the click, and `C_Container.
   UseContainerItem` was refused ("Blocked in combat: UseContainerItem()",
   and out of combat "Forbidden: UseContainerItem()"). In SuuUI's log since
   0.6.4. To check in wowlua: whether a click on an addon-made container
   item button taints the use there too (SuuUI's bag tests click items
   without an error).
6. **Tagged mobs.** wowlua answers `UnitIsTapDenied` with a fixed `false`
   (a blank, `crates/wowlua-engine/src/blanks.rs`), and no mock tags a mob
   for someone else, so SuuUI's grey for a mob another player tagged was
   tested with a stand-in. In the game the tag changes with `UNIT_FACTION`
   (ElvUI's oUF redraws on it), a tagger's death too. Steps (another
   player's mob targeted, then them dying or leaving):
   - `/run print("tap", UnitIsTapDenied("target"), UnitPlayerControlled("target"))`
   - `/run local f=CreateFrame("Frame") f:RegisterUnitEvent("UNIT_FACTION","target") f:SetScript("OnEvent",function(_,e,u) print(e,u,UnitIsTapDenied("target")) end)`

     Seen: does `UNIT_FACTION` come when the tag goes? (to do)
7. **The pet journal.** wowlua's client has no pets: `C_PetJournal` is
   fixed to "none" (`crates/wowlua-engine/src/collections.rs`: no
   `GetPetInfoByIndex`, no `mock.setPets`), so SuuUI's Pets tab was tested
   with a stand-in journal. WoW Forever has companion pets with fanfare
   gifts. To record (a character with a few pets, one a gift if possible):
   - `/run local n,o=C_PetJournal.GetNumPets() print(n,o,C_PetJournal.GetNumPetsNeedingFanfare())`
   - `/run print(C_PetJournal.GetPetInfoByIndex(1))`
   - `/run local _,s=C_PetJournal.GetPetInfoByIndex(1) print(C_PetJournal.GetPetModelSceneInfoBySpeciesID(s), select(12, C_PetJournal.GetPetInfoBySpeciesID(s)))`

     Seen (2026-11-09, 70862, the batched run): `4 4 0`; the first pet
     `BattlePet-0-… 5161 true nil 1 false false Mini Diablo 7831523 5
     272328   false true false true true` (its GUID cut here); `1272 146956
     0.25`. wowlua's journal has no pets (`0 0 0`, nothing): open until
     the pet journal is modelled (`mock.setPets`).
8. **Pet favourites and the locked journal.** Recorded in game
   (2026-11-07, 70755): `C_PetJournal.SetFavorite(petID, v)` takes 1 or 0
   (also "1"); `true`, `false` or a method call raise
   `Usage: SetFavorite(petID, bool)`, as does a nil petID. While the
   account is logged in on two clients, `C_PetJournal.IsJournalUnlocked()`
   is false and `SetFavorite` takes the call without an error and changes
   nothing (`PetIsFavorite` stays false). Mount and toy favourites are not
   locked then: `C_MountJournal.GetIsFavorite(i)` gave `false true` and
   `C_ToyBox.SetIsFavorite` changed the toy. For wowlua's pet journal when it
   comes (item 7): numbers only, and a lock a test can set. The steps (a pet
   summoned):
   - `/run local g=C_PetJournal.GetSummonedPetGUID() print(C_PetJournal.IsJournalUnlocked(), pcall(C_PetJournal.SetFavorite, g, 1))`
   - `/run local g=C_PetJournal.GetSummonedPetGUID() print(pcall(C_PetJournal.SetFavorite, g, true))`
   - `/run print(C_PetJournal.PetIsFavorite(C_PetJournal.GetSummonedPetGUID()))`

     Seen (two clients): `false true`, then `false Usage: SetFavorite(petID, bool)`, then `false`.
9. **A bag's family.** wowlua's `C_Container.GetContainerNumFreeSlots(bag)`
   answers family 0 for every bag (`crates/wowlua-engine/src/character.rs`),
   so SuuUI's sort with a profession bag (a leatherworking bag in the
   reagent slot, found in game: the sort put other reagents in it and the
   game refused) was tested with a faked family. Steps (a profession bag in
   a bag slot or the reagent slot):
   - `/run for b=1,5 do print(b, C_Container.GetContainerNumFreeSlots(b)) end`
   - `/run for b=1,5 do local s=C_Container.ContainerIDToInventoryID(b) local id=GetInventoryItemID("player",s) print(b, id, id and C_Item.GetItemFamily(id)) end`

     Seen (2026-11-09, 70862, the batched run, no profession bag):
     bags 1 to 5 `0 0` free slots and family each (all full); their items
     918, 4245, 221498, 4245 and 276892, family 0 each. As wowlua answers
     a bag of family 0; a profession bag still owed.
10. **Secret group finder results.** Forever handed a search result's
    `activityIDs` over as a secret (SuuUI's ForeverLFG on
    `LFG_LIST_SEARCH_RESULT_UPDATED`: `bad argument #1 to 'ipairs' (table
    expected, got secret)`, 2026-11-06); wowlua's results have no secrets.
    Steps (after a search in the group finder):
    - `/run local _,ids=C_LFGList.GetSearchResults() local i=C_LFGList.GetSearchResultInfo(ids[1]) print(issecretvalue(i.activityIDs), issecretvalue(i.numMembers), issecretvalue(i.comment), issecretvalue(i.leaderName))`

      Seen: which fields are secret, and when (in combat, an update)? (to do)
11. **Combat blocks wowlua let through.** SuuUI's Collections window holds
    secure buttons, so it is protected. Opened in a fight through a secure
    toggle, its OnShow moved a frame its secure buttons hang on
    (`ClearAllPoints`, `SetPoint`): the game blocked both, wowlua's tests
    did not. Escape closed it through `UISpecialFrames` (tainted by addon
    entries): the game blocked its `Hide()`, wowlua's tests did not. Fixed in
    SuuUI (moved only out of combat; Escape bound through a secure handler).
    To check: whether wowlua restricts a frame that protected frames are
    anchored to, and whether `CloseWindows` runs tainted when an addon put
    its window in `UISpecialFrames`. Steps (out of combat, then in a fight):
    - `/run W=CreateFrame("Frame","ProbeWin",UIParent) W:SetSize(100,100) W:SetPoint("CENTER") A=CreateFrame("Frame",nil,W) A:SetSize(10,10) A:SetPoint("TOP") B=CreateFrame("Button",nil,W,"SecureActionButtonTemplate") B:SetSize(10,10) B:SetPoint("TOP",A,"BOTTOM") tinsert(UISpecialFrames,"ProbeWin")`
    - in a fight: `/run print(pcall(A.ClearAllPoints, A))`, then press Escape

      Seen: blocked messages for `ClearAllPoints` and `ProbeWin:Hide()`? (to do)
12. **An item put on an empty slot of its own bag.** In wowlua,
    `C_Container.PickupContainerItem(0, 1)` then `PickupContainerItem(0, 3)`
    (slot 3 empty) empties the cursor but leaves the item in slot 1, also
    frames later; in the game it moves to slot 3. SuuUI's drag-and-drop test
    (2026-11-07) checks the slots' calls instead. Steps (an item in backpack
    slot 1, slot 3 empty):
    - `/run C_Container.PickupContainerItem(0,1) C_Container.PickupContainerItem(0,3)`
    - `/run print(C_Container.GetContainerItemID(0,1), C_Container.GetContainerItemID(0,3))`

      Seen: nil and the item's ID (moved); wowlua: the item's ID and nil. (to confirm)
13. **A hunter pet's happiness.** wowlua's pets follow classic's UI code,
    unverified: `C_PetInfo.GetPetHappiness()` gives the mood (1 unhappy, 2
    content, 3 happy), the damage in percent (75, 100, 125) and a loyalty
    rate of 0, for any pet; `C_PetInfo.GetPetLoyalty()` a loyalty name.
    SuuUI's pet frame shows the mood (2026-11-07) and keeps the last one if
    it reads secret. To record: the values, a non-hunter's pet, and secrets
    in a fight. Steps (a hunter, the pet out, out of combat):
    - `/run print(C_PetInfo.GetPetHappiness())`
    - `/run print(C_PetInfo.GetPetLoyalty())`
    - in a fight: `/run local h,d,r=C_PetInfo.GetPetHappiness() print(issecretvalue(h), issecretvalue(d), issecretvalue(r))`
    - as a warlock with a demon out: `/run print(C_PetInfo.GetPetHappiness())`

      Seen: (to do)
14. **Which spells' auras are never secret.** The game's aura container
    applies spell-ID filters (`includeSpellIDs`, `excludeSpellIDs`) to a
    debuff on a unit you can assist only when
    `C_Secrets.GetSpellAuraSecrecy(spellId)` is `Enum.SecrecyLevel.NeverSecret`
    (`Blizzard_AuraContainerUtil.lua`, `CanApplyIdentityCandidateFilters`:
    "allows noisy debuffs (Exhaustion/Sated) to be filtered out on friendly
    units"). wowlua answers ContextuallySecret for every spell, so it never
    filters a friendly unit's debuffs, and a test can't make one never secret.
    SuuUI's aura filters (2026-11-08) hide Forbearance, Weakened Soul,
    Boosted Rest… on party and raid frames only if they are. Steps (anywhere):
    - `/suu probe aurasecrecy` (SuuUI's dev build: prints each filter spell's level, saves `SuuUIProbe.auraSecrecy`)
    - or `/run for _,id in ipairs({25771,6788,15007,11196,8326,26013,1229451}) do print(id, C_Secrets.GetSpellAuraSecrecy(id)) end` (compare with `Enum.SecrecyLevel`)

      Seen (1.60.1, 2026-11-08): all 11 of SuuUI's list contextually secret
      (Forbearance, Weakened Soul, Resurrection Sickness, Recently Bandaged
      11196/1305047, Ghost, Deserter, Recently Dropped Flag ×3, Boosted
      Rest), none never secret. Also seen: `C_Spell.GetVisibilityInfo(id,
      RaidInCombat)` gives `false false false` for each (and for Corrosive
      Poison 3396): no raid-frame rules; `GetDebuffDataByIndex("player", i,
      "HARMFUL|RAID_IN_COMBAT")` returned nothing with Forbearance and
      Corrosive Poison on (out of combat); `"HARMFUL|DISPELLABLE"` returned
      Corrosive Poison. wowlua to match: never-secret only where the game
      says, these visibility answers, and the RAID_IN_COMBAT and DISPELLABLE
      filter words.
15. **SuuUI's character window tab label over its marker.** In the game the
    chosen side tab's label ("Stats", on Blizzard's `PaperDollSidebarTab1`)
    draws over SuuUI's marker fill, made after the tabs at the same level
    (6); wowlua draws the marker on top, as T587 (made later, on top;
    probed on WoW Forever 2026-11-07 for plain frames) says. Something else
    decides in the game: the strata, a parent's level, the label's draw
    layer, or the buttons' level set after the marker was made. Steps (the
    character window open on Stats; hover the tab, then the marker's edge):
    - `/run local t=PaperDollSidebarTab1 local p=t:GetParent() print(t:GetFrameStrata(), t:GetFrameLevel(), p and p:GetName(), p and p:GetFrameLevel(), t:GetFontString() and t:GetFontString():GetDrawLayer())`
    - hover the marker: `/run local f=GetMouseFoci()[1] local p=f and f:GetParent() print(f and f:GetName(), f and f:GetFrameStrata(), f and f:GetFrameLevel(), p and p:GetName(), p and p:GetFrameLevel())`

      Seen: (to do)
16. **A macro's `/click` on the pet bar's button** (decision T601).
    SuuUI's tests: a secure button with `type2 = "macro"` and `macrotext2
    = "/click PetActionButton4 RightButton"`, right-clicked, did nothing in
    wowlua (its macros ran no line). In the game, with Bite on pet bar
    slot 4, out of combat, then in combat against a training dummy:
    `/click PetActionButton4 RightButton`

    wowlua (now): the slot's autocast switched, from chat and from a
    secure button's macro, in combat too. Seen: WoW Forever 1.60.1
    (2026-11-08): `/click PetActionButton4 RightButton` toggled the slot's
    autocast, in and out of combat.

## The pet's spellbook (decision T599)

wowlua's pet spellbook is modelled on Blizzard's spellbook code
(`SpellBookPetCategoryMixin`, `SpellBookItemMixin`) and the API
documentation; seen so far only the toggle forbidden and the pet bar's
right-click (decision T600, steps 5 and 8). With a hunter, the pet out,
out of combat, the spellbook closed, the cursor empty:

1. **How many spells, and the token.**
   `/run print(C_SpellBook.HasPetSpells())`

   wowlua: the pet's spells' count and `PET` (a warlock's demon: `DEMON`);
   nothing with a pet without spells. Seen: (to do)
2. **The first pet spell's info.**
   `/run local i=C_SpellBook.GetSpellBookItemInfo(1, Enum.SpellBookSpellBank.Pet) for k,v in pairs(i or {}) do print(k, v) end`

   wowlua: `itemType` 3 (`PetAction`), `spellID` and `actionID` the spell's
   ID, `name`, `subName` (the rank, or empty), `iconID`, `isPassive`,
   `isOffSpec` false, no `skillLineIndex`. Seen: is `actionID` the spell's ID,
   or a pet action ID with flags? (to do)
3. **The first pet spell's name and texture.**
   `/run print(C_SpellBook.GetSpellBookItemName(1, 1), C_SpellBook.GetSpellBookItemTexture(1, 1), C_SpellBook.GetSpellBookItemType(1, 1))`

   wowlua: name, rank, icon, then `3`, the spell's ID twice. Seen: (to do)
4. **Autocast.** For each slot up to the count from step 1:
   `/run for i=1,(C_SpellBook.HasPetSpells() or 0) do print(i, C_SpellBook.GetSpellBookItemName(i, 1), C_SpellBook.GetSpellBookItemAutoCast(i, 1)) end`

   wowlua: `autoCastAllowed`, `autoCastEnabled` (false, false for a spell
   that can't autocast). Seen: (to do)
5. **Toggling autocast from /run.** Pick a slot that showed `true` as its
   allowed value in step 4 (here slot 1):
   `/run local f=CreateFrame("Frame") f:RegisterEvent("PET_BAR_UPDATE") f:SetScript("OnEvent", function(_, e) print(e) f:UnregisterAllEvents() end) C_SpellBook.ToggleSpellBookItemAutoCast(1, 1) print(C_SpellBook.GetSpellBookItemAutoCast(1, 1))`

   wowlua: forbidden (decision T600): `ADDON_ACTION_FORBIDDEN` with
   `ToggleSpellBookItemAutoCast()`, autocast unchanged, no `PET_BAR_UPDATE`.
   Seen: WoW Forever 1.60.1 (2026-11-08), out of combat, from SuuUI's
   spellbook: "Forbidden: ToggleSpellBookItemAutoCast()" (SuuUI's report of
   `ADDON_ACTION_FORBIDDEN`). Still to do from this /run line: is anything
   printed after the refusal (the autocast values), and does the game show
   its "interface action failed because of an AddOn" message?
6. **Toggling autocast in combat.** Attack a training dummy, then run the
   line of step 5 again.

   wowlua: forbidden as out of combat (`ADDON_ACTION_FORBIDDEN`, not
   `ADDON_ACTION_BLOCKED`). Seen: (to do; forbidden out of combat already,
   step 5)
7. **Setting autocast from /run.** The same slot:
   `/run local f=CreateFrame("Frame") f:RegisterEvent("ADDON_ACTION_FORBIDDEN") f:SetScript("OnEvent", function(_, e, a, b) print(e, a, b) f:UnregisterAllEvents() end) C_SpellBook.SetSpellBookItemAutoCastEnabled(1, 1, false) print(C_SpellBook.GetSpellBookItemAutoCast(1, 1))`

   wowlua: forbidden like the toggle (`ADDON_ACTION_FORBIDDEN`,
   `SetSpellBookItemAutoCastEnabled()`; the documentation flags both
   alike), autocast unchanged. Seen: forbidden too, or switched? (to do)
8. **Toggling autocast with the pet bar's button.** Bite (or another spell
   that may autocast) on pet bar slot 4, out of combat, then in combat
   against a training dummy:
   `/click PetActionButton4 RightButton`

   wowlua: the slot's autocast switched (`TogglePetAutocast` from the
   button's secure click), in combat too, nothing refused. Seen: WoW
   Forever 1.60.1 (2026-11-08): switched, in and out of combat (the way
   SuuUI switches it).
9. **Picking up a pet spell.**
   `/run C_SpellBook.PickupSpellBookItem(1, Enum.SpellBookSpellBank.Pet) print(GetCursorInfo())`
   then `/run ClearCursor()`

   wowlua: `petaction`, the spell's ID, 1. Seen: (to do)
10. **The pet's icon.**
   `/run print(GetPetIcon(), PetHasSpellbook())`

   wowlua: the icon `mock.setPet` gives (else the first spell's), `true`.
   Seen: a file ID or a path, and is it the family's icon? (to do)

## A texture's masks (decision T596)

On WoW Forever from its launch, each line in the chat. How many masks a
texture takes and what a fourth does: wowlua takes three and raises
an error on the fourth. Then whether two shaped masks intersect or
join.

1. `/run F=CreateFrame("Frame",nil,UIParent) F:SetSize(256,64) F:SetPoint("CENTER") T=F:CreateTexture() T:SetAllPoints() T:SetColorTexture(1,0,0) M={} for i=1,4 do M[i]=F:CreateMaskTexture() M[i]:SetAllPoints() M[i]:SetTexture("Interface\\CHATFRAME\\CHATFRAMEBACKGROUND") end`
2. `/run for i=1,3 do T:AddMaskTexture(M[i]) end print(T:GetNumMaskTextures(), pcall(T.AddMaskTexture, T, M[4])) print(T:GetNumMaskTextures())`

   Seen: the counts, and the error if any (else whether one shows in the
   error frame or the log). WoW Forever (2026-11-07): "3 false
   Texture:AddMaskTexture(): Texture already has the maximum number of
   mask textures (3)", with a "Lua Taint" line naming the addon, then
   "3". wowlua now words it so (it said "…at most 3 mask textures").
3. `/run for i=1,3 do T:RemoveMaskTexture(M[i]) end M[1]:SetTexture("Interface\\CHARACTERFRAME\\TempPortraitAlphaMask") M[2]:SetTexture("Interface\\CHARACTERFRAME\\TempPortraitAlphaMask") M[2]:ClearAllPoints() M[2]:SetPoint("LEFT",F,"CENTER") M[2]:SetSize(256,64) T:AddMaskTexture(M[1]) T:AddMaskTexture(M[2])`

   Seen: red only where the two ellipses overlap? WoW Forever
   (2026-11-07): no, red in both ovals (their union). wowlua now draws
   the union (it drew the overlap).
4. `/run F:Hide()`

## Draw layer case (decision T597)

On WoW Forever from its launch, each line in the chat. Whether a region's
layer named in lowercase is answered in capitals, and draws on that layer.

1. `/run F=CreateFrame("Frame",nil,UIParent) F:SetSize(200,40) F:SetPoint("CENTER") A=F:CreateTexture(nil,"artwork",nil,-6) A:SetAllPoints() A:SetColorTexture(1,.73,.1) B=F:CreateTexture(nil,"background",nil,-5) B:SetAllPoints() B:SetColorTexture(0,0,0) print(A:GetDrawLayer()) print(B:GetDrawLayer())`

   Seen: the names and sublevels, and whether the bar is amber (the
   `artwork` texture over the black `background` one). wowlua:
   "ARTWORK -6", "BACKGROUND -5", amber. Seen on WoW Forever 1.60.1
   (70862), 2026-11-07: "ARTWORK -6", "BACKGROUND -5", amber, as wowlua.
2. `/run F:Hide()`

## Unanchored font strings (decision T602)

On WoW Forever from its launch, each line in the chat. Whether a font
string the XML does not anchor has points of its own, and what its first
`SetPoint` leaves. wowlua: such a font string fills its parent with two
points until its first `SetPoint`, which replaces them.

1. `/run local r=MapQuestInfoRewardsFrame print(r.QuestSessionBonusReward:GetNumPoints(), r.PlayerTitleText:GetNumPoints())`

   Seen: two counts (these two are anchored only for a quest-session
   bonus or a title reward). wowlua: "2 2". Recorded 2026-11-09 (70862,
   the batched run): `1 1`. Open: which point the client gives them; owed:
   `/run local r=MapQuestInfoRewardsFrame print(r.PlayerTitleText:GetPoint(1)) print(r.QuestSessionBonusReward:GetPoint(1)) print(r.ItemReceiveText:GetNumPoints(), r.ItemReceiveText:GetPoint(1))`
   right after a `/reload` (before the map is opened).
2. Open the world map (M), and in its quest log click a quest whose
   rewards are XP and money, so its details show.
3. `/run local r=MapQuestInfoRewardsFrame local t=r.ItemReceiveText print(t:GetNumPoints(), t:GetHeight(), t:GetStringHeight(), r:GetHeight())`

   Seen: the points, the height and the text's height of "You will
   receive:", and the rewards frame's height. wowlua: "1", the two
   heights equal (about 10), the frame about 51.
4. Close the map (M).

## A secret scroll range (decision T607)

Found with SuuUI's character window on WoW Forever 1.60.1 (70862),
2026-11-09. SuuUI puts Blizzard's character stats list
(`CharacterStatsPaneScrollBox.ScrollBox`) in its own scroll frame's
scroll child, the child given a fixed height.

- Seen out of combat: `issecretvalue` false for the scroll frame's
  `GetVerticalScrollRange()`, its `GetHeight()`, the scroll child's
  `GetHeight()`, its `GetVerticalScroll()` and the list's `GetHeight()`.
- Seen in combat: `GetVerticalScrollRange()` secret, the other four
  plain; SuuUI's `math.max(0, scroll:GetVerticalScrollRange())` raised
  "attempt to perform numeric conversion on a secret number value
  (execution tainted by 'SuuUI')".
- Seen in combat: a new scroll frame (50 by 50, a 50 by 200 child, nothing
  else in it) answered a plain range.

wowlua answered a plain number always; it now answers the ranges secret
while a region in the scroll child shows a text set from a secret. That
the text is the cause is inferred (the stats' rows show the player's
health and power), not seen. To check it, each line in the chat:

1. `/run S=CreateFrame("ScrollFrame",nil,UIParent) S:SetSize(50,50) S:SetPoint("CENTER") C=CreateFrame("Frame",nil,S) C:SetSize(50,200) S:SetScrollChild(C) T=C:CreateFontString(nil,"ARTWORK","GameFontNormal") T:SetPoint("TOPLEFT")`
2. Enter combat (attack a training dummy), and stay in it for the next
   three lines.
3. `/run T:SetText(UnitHealth("player")) print(issecretvalue(UnitHealth("player")), issecretvalue(S:GetVerticalScrollRange()), issecretvalue(S:GetHorizontalScrollRange()))`

   Seen: whether the health is secret, then whether each range is.
   wowlua: the last two as the first ("true true true" when the health is
   secret).
4. `/run T:SetText("1") print(issecretvalue(S:GetVerticalScrollRange()))`

   Seen: whether a plain text makes the range plain again. wowlua:
   "false".
5. `/run T:SetText("") T:Hide() T:SetText(UnitHealth("player")) print(issecretvalue(S:GetVerticalScrollRange()))`

   Seen: whether a hidden font string's secret text counts. wowlua:
   "true".
6. `/run T:SetText("") B=CreateFrame("StatusBar",nil,C) B:SetMinMaxValues(0,UnitHealthMax("player")) B:SetValue(UnitHealth("player")) print(issecretvalue(B:GetValue()), issecretvalue(S:GetVerticalScrollRange()))`

   Seen: whether a bar's secret value counts. wowlua: "true false" (only
   texts count).
7. Leave combat, then `/run S:Hide()`

## A disabled button (decision T616)

Found with SuuUI's Beast Training on WoW Forever 1.60.1, 2026-11-10: the
game's Train button, disabled by Blizzard's trainer frame, did nothing
when SuuUI's secure button clicked it (`type = "click"`, its
`clickbutton` the Train button), where wowlua ran its `OnClick` and
bought. warcraft.wiki.gg's `Button:Click` says the same: nothing for a
disabled button or one already being clicked, `button` and `isDown`
passed on, `RegisterForClicks` ignored, no `OnMouseDown`/`OnMouseUp`.
wowlua now follows that. Not seen, left as wowlua had it: what the
mouse's own clicks and motion do on a disabled button (wowlua: no click
script, but `OnMouseDown`, `OnMouseUp`, `OnEnter` and `OnLeave` run).
Blizzard's 317 buttons with `motionScriptsWhileDisabled="true"` suggest a
disabled button gets no `OnEnter`/`OnLeave` without it. Each line in the
chat, out of combat:

1. `/run B=CreateFrame("Button",nil,UIParent,"UIPanelButtonTemplate") B:SetSize(120,30) B:SetPoint("CENTER") B:SetText("Probe") for _,s in ipairs({"PreClick","OnClick","PostClick","OnMouseDown","OnMouseUp","OnEnter","OnLeave"}) do B:SetScript(s,function(_,a,b) print(s,a,b) end) end`
2. `/run B:Disable() B:Click() print("clicked")`

   wowlua: only `clicked`. Seen (SuuUI's report, not this line): nothing
   ran. (to do)
3. Move the mouse onto the button, click it with the left button, then
   move the mouse off it.

   Seen: which of `OnEnter`, `OnMouseDown`, `OnMouseUp`, `PreClick`,
   `OnClick`, `PostClick`, `OnLeave` print, in order. wowlua: `OnEnter`,
   `OnMouseDown`, `OnMouseUp`, `OnLeave` (no click script). (to do)
4. `/run B:SetMotionScriptsWhileDisabled(true)`, then move the mouse onto
   the button and off it again.

   Seen: whether `OnEnter` and `OnLeave` print now (and did not in step
   3). wowlua: both, as in step 3. (to do)
5. `/run B:Enable() B:SetScript("OnClick",function(self,a,b) print("OnClick",a,b) self:Click() end) B:Click("RightButton", true)`

   wowlua: `PreClick RightButton true`, `OnClick RightButton true`,
   `PostClick RightButton true`, once each (the inner `Click()` does
   nothing). (to do)
6. `/run B:Hide()`

## The trainer's choice (decision T617)

Blizzard's trainer frame takes the game's choice up in
`TRAINER_DESCRIPTION_UPDATE` (`ClassTrainer_SetSelection(…
GetTrainerSelectionIndex())`), and its handler selects again, so the
event can only come for a change. SuuUI's Beast Training relies on it
(its Train is a secure click on the game's Train, which trains the
frame's choice). Recorded so far: the trainer scenario has no event when
the warrior clicked the first service, and 11 of the 14 class trainers
probed read the first available service as the choice with the window
open (the other three, with none available, read 2). wowlua chooses the
first available service as the list comes, with no event, and sends
`TRAINER_DESCRIPTION_UPDATE` at once for each change by
`SelectTrainerService`; an index past the list chooses none. Inferred,
not seen: the event at once (warcraft.wiki.gg says `SelectTrainerService`
notifies the server; the answer may come a round trip later), and the
choice when none is available. At any class trainer, its window just
opened with Blizzard's frame, nothing clicked in it yet, each line in
the chat:

1. `/run print("choice", GetTrainerSelectionIndex(), GetNumTrainerServices()) for i=1,GetNumTrainerServices() do print(i, (GetTrainerServiceInfo(i)), select(2, GetTrainerServiceInfo(i))) end`

   Seen: the choice before anything was clicked, and each service's
   state. wowlua: the first `available` service's index (nil with none).
   (to do)
2. `/run E=CreateFrame("Frame") E:RegisterEvent("TRAINER_DESCRIPTION_UPDATE") E:SetScript("OnEvent",function(_,e) print(e, GetTime(), GetTrainerSelectionIndex()) end)`
3. `/run local n=GetTrainerSelectionIndex()==2 and 3 or 2 print("before", GetTime(), GetTrainerSelectionIndex()) SelectTrainerService(n) print("after", n, GetTime(), GetTrainerSelectionIndex())`

   Seen: whether `TRAINER_DESCRIPTION_UPDATE` prints, and where: between
   `before` and `after` (at once) or later (with the server's answer; wait
   two seconds). wowlua: between them, at the same time, with the new
   index. (to do)
4. `/run local n=GetTrainerSelectionIndex() print("again", n) SelectTrainerService(n) print("after again", GetTrainerSelectionIndex())`

   wowlua: no event (the same choice). (to do)
5. `/run print("past", GetTime()) SelectTrainerService(GetNumTrainerServices()+1) print("after past", GetTime(), GetTrainerSelectionIndex())`

   wowlua: the event with nil, then `after past nil`. (to do)
6. Click the first row of Blizzard's list, wait two seconds, then click
   the second row.

   Seen: an event for each click, or only for the one that changed the
   choice? wowlua: only for a change. (to do)
7. `/run E:UnregisterAllEvents()`, then close the window.

## Beast Training (decision T618)

Reported by the user (2026-11-10): in WoW Forever Beast Training is a
hunter's spell, not an NPC. Casting it opens the game's trainer window
(`ClassTrainerFrame`) in its pet mode: `C_Trainer.GetTrainerType()` is
`Enum.TrainerType.Pet`, with training points and the trainer's functions
and events. Seen in game earlier (SuuUI's code says so):
`BuyTrainerService` from an addon's code is forbidden there ("Forbidden"),
so SuuUI trains with a secure click on the game's Train button. Seen (a
hunter's screenshot, 2026-11-10, reported by SuuUI, not a probe): the
list shows the pet's known abilities as available with no cost, not as
used.

wowlua models the rest on Blizzard's trainer code, the API documentation
and TBC's pet frame, none of it recorded: the cast of 5149 (inferred: the
client names 5149 and 5300 Beast Training) opens the window with
`TRAINER_SHOW` then `TRAINER_UPDATE`, without the interaction manager's
events; "used" for a service the pet's spellbook holds (the API's
meaning of the state); the pet's dismissal closes it with
`TRAINER_CLOSED` only. With a hunter, the pet out, out of combat, each
line in the chat:

1. Before casting: `/run print("points", C_PetInfo.GetPetTrainingPoints()) print("level", UnitLevel("pet"), "type", C_Trainer.GetTrainerType())`

   wowlua: the pet's points (all, spent), its level, type `0`. (to do)
2. `/run local n=C_Spell.GetSpellInfo("Beast Training") print("spell", n and n.spellID, n and n.name) B=CreateFrame("Frame") for _,e in ipairs({"TRAINER_SHOW","TRAINER_UPDATE","TRAINER_CLOSED","PLAYER_INTERACTION_MANAGER_FRAME_SHOW","PLAYER_INTERACTION_MANAGER_FRAME_HIDE","UNIT_SPELLCAST_SUCCEEDED"}) do B:RegisterEvent(e) end B:SetScript("OnEvent",function(_,e,a,b,c) print(GetTime(), e, a, b, c) end)`

   Seen: Beast Training's spell ID (wowlua: 5149). (to do)
3. Cast Beast Training from the spellbook, wait two seconds.

   Seen: the events in order with their times. wowlua:
   `UNIT_SPELLCAST_SUCCEEDED player … 5149`, then `TRAINER_SHOW`,
   `TRAINER_UPDATE`, no interaction manager's event. (to do)
4. Nothing clicked in the window yet: `/run print("type", C_Trainer.GetTrainerType(), "n", GetNumTrainerServices(), "choice", GetTrainerSelectionIndex()) for _,f in ipairs({"available","unavailable","used"}) do print(f, GetTrainerServiceTypeFilter(f)) end`

   wowlua: type `3`, the list's size, the first available service's
   index; the filters as at a class trainer (`used` off). (to do)
5. `/run for i=1,GetNumTrainerServices() do print(i, GetTrainerServiceInfo(i)) end`

   Seen: name, state, icon, level, rank, category of each. (to do)
6. `/run for i=1,GetNumTrainerServices() do local t=C_TooltipInfo.GetTrainerService(i) local id=t and t.id print(i, (GetTrainerServiceInfo(i)), "cost", GetTrainerServiceCost(i), "level", GetTrainerServiceLevelReq(i), "spell", id, "known", id and C_SpellBook.IsSpellKnown(id, Enum.SpellBookSpellBank.Pet)) end`

   Seen: each one's cost (training points?), level, spell, and whether
   the pet knows it. The open question: a known one's state (step 5) and
   cost here. wowlua: known ones `used`, their cost the service's. (to do)
7. `/run local u=GetTrainerServiceTypeFilter("used") SetTrainerServiceTypeFilter("used", true) for i=1,GetNumTrainerServices() do print(i, GetTrainerServiceInfo(i)) end SetTrainerServiceTypeFilter("used", u)`

   Seen: whether more services show with `used` on, and their state.
   (to do)
8. `/run print("pet book", C_SpellBook.HasPetSpells()) for i=1,(C_SpellBook.HasPetSpells() or 0) do print(i, C_SpellBook.GetSpellBookItemName(i, Enum.SpellBookSpellBank.Pet)) end`

   Seen: the pet's spellbook, to compare with steps 5 and 6. (to do)
9. Only if the pet has an ability to learn anyway (this spends training
   points): `/run P=CreateFrame("Frame") for _,e in ipairs({"UNIT_PET_TRAINING_POINTS","TRAINER_UPDATE","TRAINER_DESCRIPTION_UPDATE","SPELLS_CHANGED","PET_BAR_UPDATE","LEARNED_SPELL_IN_SKILL_LINE","CHAT_MSG_SYSTEM"}) do P:RegisterEvent(e) end P:SetScript("OnEvent",function(_,e,a) print(GetTime(), e, a) end)`,
   then choose that ability in Blizzard's list, click Train, wait two
   seconds, and `/run print("points", C_PetInfo.GetPetTrainingPoints()) P:UnregisterAllEvents()`

   Seen: the events in order with their times, and the points after.
   wowlua: a round trip after Train, `CHAT_MSG_SYSTEM` ("Your pet has
   learned a new ability: …"), `UNIT_PET_TRAINING_POINTS pet`,
   `SPELLS_CHANGED`, `TRAINER_UPDATE`; the points' spent part up by the
   cost. (to do)
10. Dismiss the pet (Dismiss Pet) with the window open, wait two seconds.

    Seen: whether the window closes, and which events print (step 2's
    frame). wowlua: `TRAINER_CLOSED` only; Blizzard's window stays
    shown. (to do)
11. Close the window, then `/run print("type", C_Trainer.GetTrainerType()) B:UnregisterAllEvents()`

    wowlua: `0`. (to do)
12. Without a pet (dismissed), cast Beast Training.

    Seen: the error shown, and whether the window opens. wowlua: nothing
    opens (no error modelled). (to do)

## Found making SuuUI's store screenshots (2026-11-10)

SuuUI's store pictures (its draft PR #125, `tests/marketing/`) are made in
wowlua with the 3D world. Where the emulator drew them otherwise than the
game, and what wowlua does now.

1. **Gaps inside words** (decision T619). SuuUI's Marcellus SC and Figtree
   drew "SILVE RMANE", "CHARACTE R", "M ana", "Maevria' s"; in the game
   (the user's screenshots of SuuUI's windows) the words are whole.
   wowlua measured an addon's own font as Friz Quadrata and drew the real
   font's glyphs at those advances. It now measures it with the addon's
   file; the T392 rule matches the game's numbers of item 2 of "Found
   with SuuUI on WoW Forever" to the pixel. Owed, to confirm the rule on
   Marcellus SC: `/run local f=UIParent:CreateFontString(nil,"OVERLAY") f:SetFont("Interface\\AddOns\\SuuUI\\Media\\Fonts\\MarcellusSC-Regular.ttf",14,"") f:SetText("Silvermane Character") print(f:GetUnboundedStringWidth(), UIParent:GetEffectiveScale(), select(2, GetPhysicalScreenSize()))`

   Seen: (to do) wowlua: the width at the screen and scale printed.
2. **The chosen side tab's label under its marker** (decision T620): see
   "Frames of the same level", steps 8 to 13.
3. **Labels broken before their colon** (decision T621). Once Figtree
   measured as itself, the character window's stat labels ("Health:",
   one point, no width) broke into "Health" and ":" in wowlua: a line's
   advances passed the width its own text gave it. In the game they are
   one line. wowlua no longer wraps a font string at a width that is its
   own text's. Owed: `/run local f=UIParent:CreateFontString(nil,"OVERLAY") f:SetFont("Interface\\AddOns\\SuuUI\\Media\\Fonts\\Figtree-Regular.ttf",13,"") f:SetPoint("CENTER") f:SetText("Movement Speed:") f:SetWidth(f:GetStringWidth()) print(f:GetNumLines(), f:GetStringWidth())`

   Seen: whether a width set to the string's own width wraps it. wowlua:
   two lines when the last glyph's advance passes its ink and one (T282's
   rule), one otherwise.
4. **Stormwind drawn upside down** (decision T622). Placed in the Trade
   District by map position (`mock.moveTo({ mapID = 1453, x = 0.62, y =
   0.70 })`), the player stood on the terrain under the city's streets (a
   building's floors over it), and the camera saw the city's underside
   above it. wowlua now stands a map position on the highest floor there.
   Nothing to probe: the game always knows the height.
5. **Two texts over each other on a raid frame** (seen, not looked into):
   in the raid scene (`02-raid-healer`) two members' frames (the third
   column's sixth, the fourth column's seventh) drew a second text over
   their names. To compare with the game's raid frames in the same state.

## Animations after a hitch (decision T623)

Found with SuuUI's windows on 2026-11-10: their fade-in (an alpha 0 to 1
of 0.12 s) did not move while a test waited with `mock.advanceTime`, so
a screenshot drew the window at alpha 0. wowlua now plays a group on the
clock: a frame after a long one (a hitch) moves it by the whole time, and
a repeating group that passes several lengths in one frame runs `OnLoop`
once and goes on where the clock has it. Neither is seen in game yet.
Each line in the chat, out of combat (step 3 freezes the game for three
seconds, step 4 for 1.3 seconds):

1. `/run F=CreateFrame("Frame",nil,UIParent) G=F:CreateAnimationGroup() A=G:CreateAnimation("Alpha") A:SetFromAlpha(1) A:SetToAlpha(0.5) A:SetDuration(30) G:SetScript("OnLoop",function(_,s) print("loop",s,GetTime()-P) end) G:SetScript("OnFinished",function() print("finished",GetTime()-P) end)`
2. `/run G:Play() P=GetTime() print("playing",G:IsPlaying())`

   wowlua: `playing true`.
3. Within ten seconds of step 2: `/run local t=debugprofilestop() while debugprofilestop()-t<3000 do end print("before",G:GetElapsed(),GetTime()-P) C_Timer.After(0.1,function() print("after",G:GetElapsed(),GetTime()-P) end)`

   Seen: on each line, the group's time and the time since its `Play`.
   wowlua: on each line the two equal (within a frame), the `after`
   line's about three seconds past the `before` line's (the hitch
   counted whole). (to do)
4. `/run G:Stop() A:SetDuration(0.5) G:SetLooping("REPEAT") G:Play() P=GetTime() local t=debugprofilestop() while debugprofilestop()-t<1300 do end C_Timer.After(0.1,function() print("after",G:GetElapsed(),GetTime()-P) G:Stop() end)`

   Seen: the `loop` lines before the `after` line (how many, each with
   its time) and the `after` line. wowlua: one `loop FORWARD` line
   about 1.3 s after `Play`, then `after` with the group's time the time
   since `Play` less whole repeats of 0.5 s (about 0.3 to 0.4 then).
   (to do)
5. `/run G:Stop() F:Hide()`

## A mouse script after the clicks were turned off (decision T365)

Found with SuuUI on wowlua's main (2026-11-10): SuuUI's weapon enchant
icon turns its mouse motion on and its clicks off
(`SetMouseMotionEnabled(true)`, `SetMouseClickEnabled(false)`), and only
then sets `OnEnter` and `OnLeave` for its tooltip. Since T365 took WoW
Forever's `CharacterFrameModeTab2` (`true true true`), wowlua turns the
clicks and the motion on with any mouse script, so the icon takes clicks
again; 0.13.9 turned on only `IsMouseEnabled`'s own flag.
warcraft.wiki.gg says a mouse script implies `EnableMouse(true)`, motion
scripts too since 9.0.1 ("full mouse input"), but the side tab has click
and motion scripts both, and no recording sets a script after an
explicit `SetMouseClickEnabled(false)`. Each line in the chat:

1. `/run local f=CreateFrame("Frame") f:SetMouseMotionEnabled(true) f:SetMouseClickEnabled(false) print(f:IsMouseEnabled(),f:IsMouseClickEnabled(),f:IsMouseMotionEnabled()) f:SetScript("OnEnter",function() end) print(f:IsMouseEnabled(),f:IsMouseClickEnabled(),f:IsMouseMotionEnabled())`

   Seen: (to do). wowlua: `false false true`, then `true true true`;
   SuuUI's test expected the clicks to stay off.
2. `/run local g=CreateFrame("Frame") g:SetScript("OnEnter",function() end) print(g:IsMouseEnabled(),g:IsMouseClickEnabled(),g:IsMouseMotionEnabled())`

   Seen: (to do). wowlua: `true true true` (a motion script alone turns
   the clicks on too).
3. `/run local h=CreateFrame("Frame") h:SetMouseClickEnabled(true) h:SetMouseMotionEnabled(false) h:SetScript("OnMouseUp",function() end) print(h:IsMouseEnabled(),h:IsMouseClickEnabled(),h:IsMouseMotionEnabled())`

   Seen: (to do). wowlua: `true true true`.
4. `/run local k=CreateFrame("Frame") k:EnableMouse(false) k:SetScript("OnEnter",function() end) print(k:IsMouseEnabled(),k:IsMouseClickEnabled(),k:IsMouseMotionEnabled())`

   Seen: (to do). wowlua: `true true true` (an explicit
   `EnableMouse(false)` before does not keep it off).
5. `/run local m=CreateFrame("Frame") m:SetMouseMotionEnabled(true) m:SetMouseClickEnabled(false) m:HookScript("OnEnter",function() end) print(m:IsMouseEnabled(),m:IsMouseClickEnabled(),m:IsMouseMotionEnabled())`

   Seen: (to do). wowlua: `true true true`.
6. `/run local n=CreateFrame("Frame") n:SetMouseClickEnabled(true) print(n:IsMouseEnabled(),n:IsMouseClickEnabled(),n:IsMouseMotionEnabled())`

   Seen: (to do). wowlua: `false true false` (`IsMouseEnabled` is its
   own flag; the wiki says it is true only with both on).

## Cmd as a modifier (decision T624, 2026-11-10)

Found with SuuUI's click-target menu (its draft PR #132): a Cmd click
with something bound renames the click in its secure `OnClick` wrap, and
wowlua could not hold Cmd (`IsMetaKeyDown` always false). Answered in
WoW Forever on the user's Mac, each line
`IsMetaKeyDown(), IsModifierKeyDown(), SecureCmdOptionParse("[mod:meta] meta; none"), SecureCmdOptionParse("[mod] mod; none")`
(the last `nil` that one's second return):

- Cmd held: `true true meta mod nil`.
- Nothing held: `false false none none nil`.
- Shift held: `false true none mod nil`.

wowlua before: `false false none none nil` with Cmd held (no way to hold
it); now as the game, with `mock.setModifiers({ meta = true })`.

Also seen in Blizzard's code, nothing to change:

- The secure templates' modifier prefix (`SecureButton_GetModifierPrefix`)
  knows only shift, ctrl and alt: a Cmd click is a plain click to their
  attributes. That is Blizzard's Lua, which wowlua runs as it is.
- `SecureCmdOptionParse` and `IsModifiedClick` are in the restricted
  environment (`Blizzard_RestrictedAddOnEnvironment`'s list), so a
  secure snippet can ask for Cmd with `[mod:meta]`; `IsMetaKeyDown` is
  not.

### Cmd+Left as a right click (decision T624)

Seen in game the same day: with Options > Mac > "Use Command + Left
Click as Right Click" on (`MacUseCommandLeftClickAsRightClick`), a
Cmd+Left click on a SuuUI box (its `OnMouseDown` reading the button and
`IsMetaKeyDown`) came as a plain right click: `RightButton`, Cmd not
seen as held. The CVar's default is 1 (recorded on the Mac:
`data/camelot-cvars.txt`). wowlua now gives `mock.click` with Cmd held
and the left button on the Mac client a `RightButton` click with Cmd let
go while it runs (held again after), unless the CVar is 0. Owed, to
settle what the click sees of Cmd and the CVar off. Each line in the
chat, one at a time:

1. Make a green square in the middle that prints each mouse script:

   ```
   /run F=CreateFrame("Button",nil,UIParent) F:SetSize(200,200) F:SetPoint("CENTER") F:RegisterForClicks("AnyUp") local t=F:CreateTexture() t:SetAllPoints() t:SetColorTexture(0,0.5,0,0.5) for _,s in ipairs({"OnMouseDown","OnMouseUp","OnClick"}) do F:SetScript(s,function(_,b) print(s,b,IsMetaKeyDown(),IsModifierKeyDown(),SecureCmdOptionParse("[mod:meta] meta; none")) end) end
   ```

2. Print the setting and its default:

   ```
   /run print(C_CVar.GetCVar("MacUseCommandLeftClickAsRightClick"),C_CVar.GetCVarDefault("MacUseCommandLeftClickAsRightClick"))
   ```

   wowlua: `1 1`.
3. Hold Cmd, left-click the green square, let go of Cmd.

   Seen: (to do) the three lines. wowlua: `OnMouseDown RightButton false
   false none`, the same for `OnMouseUp` and `OnClick`.
4. Turn the setting off:

   ```
   /run C_CVar.SetCVar("MacUseCommandLeftClickAsRightClick","0")
   ```

5. Hold Cmd, left-click the green square, let go of Cmd.

   Seen: (to do). wowlua: `OnMouseDown LeftButton true true meta`, the
   same for `OnMouseUp` and `OnClick`.
6. Turn the setting back on and remove the square:

   ```
   /run C_CVar.SetCVar("MacUseCommandLeftClickAsRightClick","1")
   ```

   ```
   /run F:Hide()
   ```

## Click bindings (decision T625, 2026-11-10)

Found with SuuUI's mouseover bindings: in game a unit frame
(`SecureUnitButtonTemplate`) with `*type1 = target` did nothing on
Shift-Left or Alt-Left, and a friend-only spell on Ctrl-Left
(`ctrl-helpbutton1` set, no `ctrl-type1`) did nothing either. Blizzard's
`SecureUnitButton_OnClick` reads the click's type first, sees `target`,
finds no click binding for the click and returns. wowlua let both
through: its `MakeModifiers()` answered 0 whatever was held, and its
`GetBindingType` ignored the modifiers.

Answered in WoW Forever on the user's Mac (2026-11-10):

- `C_ClickBindings.GetProfileInfo()`: exactly two entries,
  `{type=3, actionID=1, button="LeftButton", modifiers=0}` and
  `{type=3, actionID=2, button="RightButton", modifiers=0}`: the
  interactions Target and OpenContextMenu.
- `GetStringFromModifiers(n)`: 1 `SHIFT`, 2 `SHIFT`, 4 `CTRL`, 8 `CTRL`,
  16 `ALT`, 32 `ALT`, 64 `META`, 128 `META`; 256 and 512 an empty string.
  Each modifier has two bits, its left and its right key.
- One Shift held: `MakeModifiers()` 3 (both bits),
  `C_ClickBindings.GetBindingType("LeftButton", 3)` 0 (None),
  `C_ClickBindings.GetEffectiveInteractionButton("LeftButton", 3)` nil.

wowlua now keeps the profile per character (`SetProfileByInfo`,
`ResetCurrentProfile`), answers `MakeModifiers()` from the keys held
(Shift 3, Ctrl 12, Alt 48, Cmd 192) and `GetBindingType` for exactly the
button and modifiers, so both clicks do nothing as in the game.

### Owed: a moved interaction and an event on a change

wowlua gives a moved interaction's effective button as its default
button (Target `LeftButton`, the menu `RightButton`, as
`SecureUnitButton_OnClick`'s comment says), fires no event when the
profile changes, and joins several modifiers' names with `-` in the
`ALT-CTRL-SHIFT-META` order. None is seen in game yet. Each line in the
chat, one at a time, out of combat. Step 7 puts your own click bindings
back as they were.

1. Keep your click bindings to put them back later:

   ```
   /run CBSAVE=C_ClickBindings.GetProfileInfo() print(#CBSAVE)
   ```

2. Make a frame that prints every event while `CBW` is set:

   ```
   /run CBF=CreateFrame("Frame") CBF:RegisterAllEvents() CBF:SetScript("OnEvent",function(_,e,...) if CBW then print("event",e,...) end end)
   ```

3. Move Target to Shift-Left (the menu stays on Right) and watch the
   events for a second:

   ```
   /run CBW=true C_ClickBindings.SetProfileByInfo({{type=3,actionID=1,button="LeftButton",modifiers=3},{type=3,actionID=2,button="RightButton",modifiers=0}}) C_Timer.After(1,function() CBW=nil print("done") end)
   ```

   Seen: (to do) every `event` line before `done`, and any error.
   wowlua: only `done`.
4. Ask for the left button with Shift and without:

   ```
   /run print(C_ClickBindings.GetBindingType("LeftButton",3),C_ClickBindings.GetEffectiveInteractionButton("LeftButton",3),C_ClickBindings.GetBindingType("LeftButton",0),C_ClickBindings.GetEffectiveInteractionButton("LeftButton",0))
   ```

   Seen: (to do). wowlua: `3 LeftButton 0 nil`.
5. Swap them, Target on Right and the menu on Left:

   ```
   /run C_ClickBindings.SetProfileByInfo({{type=3,actionID=1,button="RightButton",modifiers=0},{type=3,actionID=2,button="LeftButton",modifiers=0}})
   ```

6. Ask for both buttons:

   ```
   /run print(C_ClickBindings.GetEffectiveInteractionButton("RightButton",0),C_ClickBindings.GetEffectiveInteractionButton("LeftButton",0))
   ```

   Seen: (to do). wowlua: `LeftButton RightButton`.
7. Put your click bindings back and stop the event frame:

   ```
   /run C_ClickBindings.SetProfileByInfo(CBSAVE) CBF:UnregisterAllEvents() print(#C_ClickBindings.GetProfileInfo())
   ```

   Seen: the same number as in step 1.
8. Several modifiers' names together:

   ```
   /run print(GetStringFromModifiers(3),GetStringFromModifiers(51),GetStringFromModifiers(255))
   ```

   Seen: (to do). wowlua: `SHIFT ALT-SHIFT ALT-CTRL-SHIFT-META`.

## Mounts, pets and toys dragged (decision T626, 2026-11-10)

Found with SuuUI's collections window (released in 0.6.37), which lets a
mount, a pet or a toy of yours be dragged onto an action bar: wowlua's
`C_MountJournal.Pickup`, `C_PetJournal.PickupPet` and
`C_ToyBox.PickupToyBoxItem` did nothing, so the drag could not be
tested. Answered in WoW Forever on the user's Mac:

- `C_MountJournal.Pickup(1); print(GetCursorInfo())`: `mount 3111 36`,
  the displayed mount's ID and a second number that is not the display
  index (1). warcraft.wiki.gg calls it the mount's index in the journal;
  wowlua answers its place in `GetMountIDs()` (unverified: the recorded
  journal has mount 3111 24th).
- `C_PetJournal.PickupPet(guid)`, then `GetCursorInfo()`:
  `battlepet BattlePet-0-00000009B56D`, the type and the GUID.
- `C_ToyBox.PickupToyBoxItem(280740)`, then `GetCursorInfo()`:
  `item 280740 [Shen'dorei Windwell]`, a plain item with its ID and link.
- Placed on action bars by dragging, `GetActionInfo(slot)`: a mount
  `summonmount 3130 nil` (its mount ID), a pet
  `summonpet BattlePet-0-00000009B56B nil` (its GUID), a toy
  `item 280740 nil` (a plain item action).

wowlua before: nothing on the cursor for any of them. Now as the game.
Still open: what the mount's second number is, an uncollected mount
picked up (`Pickup` of a not collected one's index), a toy the player
lacks, `Pickup(0)` (the random favourite), and the `CURSOR_CHANGED`
types.

## Protected frames anchored through frames to regions (decision T628, 2026-11-11)

Found in game (WoW Forever 1.60.1.70888) with SuuUI's world map: its
title's `secure.overlay` (a `SecureActionButtonTemplate` button on
UIParent) was anchored with `SetAllPoints` to a plain button whose two
points were on a font string, and the game refused it:
`Action[SetPoint] failed because[Cannot anchor protected frames to
regions]: attempted from: Button:SetAllPoints.` So the check reaches at
least one frame past the relative, and `SetAllPoints` reports the action
as `SetPoint`. wowlua let it through (it only refused a region as the
direct relative, decision T252).

wowlua now refuses a protected frame's `SetPoint` or `SetAllPoints` when
the relative (or, without one, the parent) is a region or a frame whose
anchors lead to one, through any number of frames, an anchor without a
relative going to the frame's parent. Only the frame's own protection
counts (a secure template, not a secure child), and only at the moment it
anchors. Unverified: two frames deep, the parent's place, a frame
protected through its children, a chain made after the anchor.

Each line in the chat, one at a time, out of combat. Each step makes a
fresh secure button. If a step's line shows the game's error window
instead of a printed line, note that (`pcall` didn't catch it).

1. Make a holder frame with a font string and a texture, and the printing
   helper:

   ```
   /run AH=CreateFrame("Frame",nil,UIParent) AH:SetSize(100,100) AH:SetPoint("CENTER") AT=AH:CreateFontString(nil,"ARTWORK","GameFontNormal") AT:SetPoint("TOP") AT:SetText("x") AX=AH:CreateTexture() AX:SetSize(10,10) AX:SetPoint("BOTTOM") function AP(n,...) local ok,e=pcall(...) print(n,ok and "ok" or tostring(e):match("Action.*") or tostring(e)) end print("ready")
   ```

2. Make `A1` on the font string, `A2` on `A1` (two steps away) and `A3`,
   a child of `A1` placed without a relative:

   ```
   /run A1=CreateFrame("Button",nil,AH) A1:SetPoint("TOPLEFT",AT,"TOPLEFT") A1:SetPoint("BOTTOMRIGHT",AT,"BOTTOMRIGHT") A2=CreateFrame("Frame",nil,AH) A2:SetAllPoints(A1) A3=CreateFrame("Frame",nil,A1) A3:SetSize(5,5) A3:SetPoint("CENTER") print("ready")
   ```

3. A font string as the relative, `SetPoint`:

   ```
   /run local b=CreateFrame("Button",nil,UIParent,"SecureActionButtonTemplate") AP("fs point",b.SetPoint,b,"CENTER",AT)
   ```

   Seen: (to do). wowlua: `fs point Action[SetPoint] failed because[Cannot anchor protected frames to regions]: attempted from: Button:SetPoint.`
4. A font string as the relative, `SetAllPoints`:

   ```
   /run local b=CreateFrame("Button",nil,UIParent,"SecureActionButtonTemplate") AP("fs all",b.SetAllPoints,b,AT)
   ```

   Seen: (to do). wowlua: `fs all Action[SetPoint] failed because[Cannot anchor protected frames to regions]: attempted from: Button:SetAllPoints.`
5. A texture as the relative, `SetPoint`:

   ```
   /run local b=CreateFrame("Button",nil,UIParent,"SecureActionButtonTemplate") AP("tex point",b.SetPoint,b,"CENTER",AX)
   ```

   Seen: (to do). wowlua: `tex point Action[SetPoint] failed because[…]: attempted from: Button:SetPoint.`
6. A texture as the relative, `SetAllPoints`:

   ```
   /run local b=CreateFrame("Button",nil,UIParent,"SecureActionButtonTemplate") AP("tex all",b.SetAllPoints,b,AX)
   ```

   Seen: (to do). wowlua: `tex all Action[SetPoint] failed because[…]: attempted from: Button:SetAllPoints.`
7. A frame on the font string, `SetPoint`:

   ```
   /run local b=CreateFrame("Button",nil,UIParent,"SecureActionButtonTemplate") AP("A1 point",b.SetPoint,b,"CENTER",A1)
   ```

   Seen: (to do). wowlua: `A1 point Action[SetPoint] failed because[…]: attempted from: Button:SetPoint.`
8. A frame on the font string, `SetAllPoints` (the case seen with SuuUI):

   ```
   /run local b=CreateFrame("Button",nil,UIParent,"SecureActionButtonTemplate") AP("A1 all",b.SetAllPoints,b,A1)
   ```

   Seen: (to do). wowlua: `A1 all Action[SetPoint] failed because[…]: attempted from: Button:SetAllPoints.`
9. A frame two steps away from the font string:

   ```
   /run local b=CreateFrame("Button",nil,UIParent,"SecureActionButtonTemplate") AP("A2 point",b.SetPoint,b,"CENTER",A2)
   ```

   Seen: (to do). wowlua: `A2 point Action[SetPoint] failed because[…]: attempted from: Button:SetPoint.`
10. A frame placed on its parent, which is on the font string:

    ```
    /run local b=CreateFrame("Button",nil,UIParent,"SecureActionButtonTemplate") AP("A3 point",b.SetPoint,b,"CENTER",A3)
    ```

    Seen: (to do). wowlua: `A3 point Action[SetPoint] failed because[…]: attempted from: Button:SetPoint.`
11. A secure child of `A1`, placed without a relative (on its parent),
    and whether `A1` is now protected:

    ```
    /run local b=CreateFrame("Button",nil,A1,"SecureActionButtonTemplate") AP("in A1",b.SetPoint,b,"CENTER") print(A1:IsProtected())
    ```

    Seen: (to do). wowlua: `in A1 Action[SetPoint] failed because[…]: attempted from: Button:SetPoint.`, then `true false`.
12. `A1`, now protected through its secure child, anchored to the font
    string again:

    ```
    /run AP("A1 again",A1.SetPoint,A1,"TOPLEFT",AT,"TOPLEFT")
    ```

    Seen: (to do). wowlua: `A1 again ok`.
13. A secure button anchored to a plain frame first, the frame then
    anchored to the font string, and where the button is:

    ```
    /run local f=CreateFrame("Frame",nil,AH) f:SetSize(10,10) f:SetPoint("CENTER") local b=CreateFrame("Button",nil,UIParent,"SecureActionButtonTemplate") b:SetSize(10,10) b:SetPoint("CENTER",f) AP("later",f.SetPoint,f,"TOP",AT,"BOTTOM") print(b:GetCenter())
    ```

    Seen: (to do). wowlua: `later ok`, then the button's center (under
    the font string).
14. Hide what the steps made:

    ```
    /run AH:Hide() print("done")
    ```

## Group loot rolls (decision T629, 2026-11-11)

Found with SuuUI's loot roll tray (its draft `loot-roll-tray`): wowlua had
no group loot at all, so the tray could only be tested with stubs. wowlua
now starts rolls from tests (`mock.startLootRoll`) and plays them as
Blizzard's GroupLootFrame and SuuUI's comments ("found in game") describe,
but no roll is recorded yet. These need a group with group loot and an
item rolled for (a dungeon's Uncommon or better drop); one Bind on Pickup
item for steps 3 to 5. Each line in the chat, one at a time.

1. Make a frame that prints the roll events and the loot lines (the raw
   text, its links shown as `||H…`, and the line's sender), and keeps the
   newest roll's ID in `LRID`:

   ```
   /run LRF=CreateFrame("Frame") for _,e in ipairs({"START_LOOT_ROLL","CONFIRM_LOOT_ROLL","CANCEL_LOOT_ROLL","CANCEL_ALL_LOOT_ROLLS","LOOT_ROLLS_COMPLETE","MAIN_SPEC_NEED_ROLL","LOOT_HISTORY_UPDATE_DROP","LOOT_HISTORY_UPDATE_ENCOUNTER","LOOT_HISTORY_GO_TO_ENCOUNTER","CHAT_MSG_LOOT"}) do LRF:RegisterEvent(e) end LRF:SetScript("OnEvent",function(_,e,...) if e=="START_LOOT_ROLL" then LRID=... end if e=="CHAT_MSG_LOOT" then local t,s=... print(format("%.3f",GetTime()),e,(t:gsub("|","||")),"sender:",s) else print(format("%.3f",GetTime()),e,...) end end)
   ```

   Seen: (to do) for each roll that starts, `START_LOOT_ROLL` with the
   roll's ID, its time and the loot's handle, and any loot line with it
   (`LOOT_ROLL_STARTED`'s "[Loot]: [item]"?). wowlua: `START_LOOT_ROLL(1,
   60000, 1)`, `LOOT_HISTORY_UPDATE_ENCOUNTER(0)`, no line.
2. While the roll is open, before choosing, ask for it:

   ```
   /run print(LRID,GetLootRollTimeLeft(LRID),C_Loot.GetLootRollDuration(LRID),#GetActiveLootRollIDs(),select("#",GetLootRollItemInfo(LRID)),GetLootRollItemInfo(LRID))
   ```

   Seen: (to do). wowlua: the ID, the milliseconds left, `60000`, `1`,
   `13`, then the texture, name, count, quality, Bind on Pickup, `true
   true false 0 0 0 0 false` for an item you may Need and Greed.
3. On the Bind on Pickup item, click Need in the game's roll frame, but
   do not answer the popup yet.

   Seen: (to do) whether `CONFIRM_LOOT_ROLL` prints, its third value
   (`confirmReason`), and whether `CANCEL_LOOT_ROLL` comes before the
   popup's answer. wowlua: `CONFIRM_LOOT_ROLL 1 1` and an empty reason,
   nothing else.
4. Click OK on the popup.

   Seen: (to do) the events and lines, and how long after the click each
   comes. wowlua: `CANCEL_LOOT_ROLL` at once, a round trip later the line
   "[Loot]: You have selected Need for: [item]" with no sender, and
   `LOOT_HISTORY_UPDATE_DROP`.
5. Right after, ask for the roll again:

   ```
   /run print(LRID,GetLootRollTimeLeft(LRID),#GetActiveLootRollIDs(),select("#",GetLootRollItemInfo(LRID)),(select(2,GetLootRollItemInfo(LRID))),GetLootRollItemLink(LRID))
   ```

   Seen: (to do). wowlua: the time still counting, `0`, `13`, the item's
   name and link.
6. Let the group finish choosing, and watch the end.

   Seen: (to do) every line and event of the result in order: each
   roller's "Need Roll - 91 for [item] by …" (yours by your name or
   otherwise?), the "won" line, "You receive loot" if you won,
   `LOOT_HISTORY_UPDATE_DROP`, `LOOT_ROLLS_COMPLETE` (with which handle),
   `MAIN_SPEC_NEED_ROLL`. wowlua: the roll lines in choice order, the won
   line, the drop's update, `LOOT_ROLLS_COMPLETE` with the handle, the
   item pushed (`You receive loot`) after it if yours; no
   `MAIN_SPEC_NEED_ROLL`.
7. On another roll, choose nothing and let its time run out.

   Seen: (to do) `CANCEL_LOOT_ROLL` at the time out or not, a "passed"
   line for you or the others who did not choose, then the result.
   wowlua: `CANCEL_LOOT_ROLL`, no line for who did not choose, the result.
8. After the rolls, read the loot history's encounters:

   ```
   /run for _,e in ipairs(C_LootHistory.GetAllEncounterInfos()) do print(e.encounterID,e.encounterName,e.startTime,e.duration,C_LootHistory.GetLootHistoryTime(),GetTime()) end
   ```

   Seen: (to do), outside a boss fight especially (which encounter a
   trash drop is under). wowlua: the encounter under way's ID or `0`, the
   newest first, times in `GetTime`'s seconds.
9. Read the newest encounter's drops:

   ```
   /run local e=C_LootHistory.GetAllEncounterInfos()[1] for _,d in ipairs(C_LootHistory.GetSortedDropsForEncounter(e.encounterID)) do print(d.lootListKey,d.playerRollState,d.allPassed,d.winner and d.winner.playerName,d.startTime,d.duration) for _,r in ipairs(d.rollInfos) do print(" ",r.playerName,r.playerClass,r.isSelf,r.state,r.roll,r.isWinner) end end
   ```

   Seen: (to do) the keys (from 1?), the states, the rollers' order.
   wowlua: keys from 1 in the drops' order; the rollers first by number,
   then Need, Greed, Pass, none, then in the order chosen.
10. Stop the frame:

    ```
    /run LRF:UnregisterAllEvents()
    ```

## A second click on the same item link (2026-11-11)

Found with SuuUI's chat tests: a second `SetItemRef` with the same item
link leaves `ItemRefTooltip` shown in wowlua. The game's
`ItemRefTooltipMixin:SetHyperlink` means to hide it ("it's the same
hyperlink as current data, close instead"), but its check can't match as
the code reads: the tooltip keeps the arguments packed by `SafePack`, so
`getterArgs` is `{ "item:6948", n = 1 }`, and `tCompare` against the new
`{ "item:6948" }` fails on the `n`. wowlua runs that code unchanged, so it
answers as the code does. Whether the game closes it all the same is open;
wowlua is not changed until these steps answer. Each line in the chat,
one at a time.

1. Hide the tooltip:

   ```
   /run ItemRefTooltip:Hide()
   ```

2. Open the Hearthstone's link the way a chat click does, and ask whether
   it shows:

   ```
   /run SetItemRef("item:6948","[Hearthstone]","LeftButton",ChatFrame1) print(ItemRefTooltip:IsShown())
   ```

   Seen: (to do). wowlua: `true`.
3. Print what the tooltip kept, and the game's compare of it:

   ```
   /run local a=ItemRefTooltip:GetPrimaryTooltipInfo().getterArgs for k,v in pairs(a) do print(k,v) end print(tCompare(a,{"item:6948"}))
   ```

   Seen: (to do). wowlua: `1 item:6948`, `n 1`, `false`.
4. Open the same link again:

   ```
   /run SetItemRef("item:6948","[Hearthstone]","LeftButton",ChatFrame1) print(ItemRefTooltip:IsShown())
   ```

   Seen: (to do) `false` if the game closes it. wowlua: `true`.
5. Hide the tooltip and print the Hearthstone's link into the chat (if it
   prints `nil`, run the same line again a second later):

   ```
   /run ItemRefTooltip:Hide() print(select(2,C_Item.GetItemInfo(6948)))
   ```

6. Click the printed link in the chat with the left mouse button, once.

   Seen: (to do) the item's tooltip opens. wowlua: it opens.
7. Click the same link again, with the left mouse button.

   Seen: (to do) whether the tooltip closes or stays. wowlua: it stays.
8. Hide the tooltip:

   ```
   /run ItemRefTooltip:Hide()
   ```

## SetRaidTarget in combat (decision T631, 2026-11-11)

Found in game: in combat, a raid target icon picked from the unit menu
that SuuUI's restyle had tainted was refused ("Interface action failed
because of an AddOn", `ADDON_ACTION_FORBIDDEN` with "SetRaidTarget()");
with the restyle off the same pick marked the target in combat. wowlua now
forbids an addon's `SetRaidTarget` in combat on camelot and keeps the
markers. These steps check it from `/run` (an addon's code, tainted) and
what is still guessed: whether the marker is there at once or a round trip
later, and whether `GetRaidTargetIndex` answers a secret out of combat.
They need a hostile creature a few levels below you to fight. Turn
SuuUI's menu restyle off for step 7. Each line in the chat, one at a time.

1. Make a frame that prints the raid target and forbidden events:

   ```
   /run RTP=CreateFrame("Frame") RTP:RegisterEvent("RAID_TARGET_UPDATE") RTP:RegisterEvent("ADDON_ACTION_FORBIDDEN") RTP:SetScript("OnEvent",function(_,e,...) print(format("%.3f",GetTime()),e,...) end)
   ```

2. Target the creature, without attacking it. Out of combat, put the skull
   on it and read its marker at once:

   ```
   /run SetRaidTarget("target",8) print(format("%.3f",GetTime()),"asked",GetRaidTargetIndex("target")~=nil)
   ```

   Seen: (to do) the skull over the creature, and the order of the lines:
   `RAID_TARGET_UPDATE` before `asked true` if the client marks at once,
   after `asked false` if the server answers later. wowlua: `asked false`,
   then `RAID_TARGET_UPDATE` about 0.4 s later.
3. Read the marker a moment later:

   ```
   /run local i=GetRaidTargetIndex("target") print(issecretvalue(i),canaccessvalue(i) and i or "hidden")
   ```

   Seen: (to do). wowlua: `true hidden` (secret on camelot by the
   documentation's `SecretReturns`).
4. Take it off:

   ```
   /run SetRaidTarget("target",0)
   ```

   Seen: (to do) the skull goes, one `RAID_TARGET_UPDATE`. wowlua: the
   same.
5. Attack the creature so you are in combat, then try the skull again:

   ```
   /run print("combat",InCombatLockdown()) SetRaidTarget("target",8)
   ```

   Seen: (to do) "Interface action failed because of an AddOn" (or a
   popup that an AddOn was blocked), `ADDON_ACTION_FORBIDDEN` with the
   addon's name and `SetRaidTarget()`, no skull and no
   `RAID_TARGET_UPDATE`. wowlua: `combat true`, `ADDON_ACTION_FORBIDDEN`
   with `SetRaidTarget()`, no skull.
6. Still in combat, read the marker:

   ```
   /run print(GetRaidTargetIndex("target")~=nil)
   ```

   Seen: (to do). wowlua: `false`.
7. Still in combat, right-click the creature's target frame and pick
   Raid Target Icon, then Skull.

   Seen: (to do) the skull over the creature, `RAID_TARGET_UPDATE`, no
   forbidden event. wowlua: the same.
8. After the fight, stop the printing frame:

   ```
   /run RTP:UnregisterAllEvents()
   ```

Open: the other flavors (Retail, Mists, TBC, Classic Era) document
`SetRaidTarget` as restricted too, and wowlua leaves it unrestricted there;
steps 1, 5 and 6 on each of those clients would settle it.

## Aura buttons forbidden after their build (decision T632, 2026-11-11)

Found in game: in the open world, with SuuUI loaded, a walk of every frame
stopped at a frame SuuUI's `initializeFrame` had put on an engine aura
button: its `IsForbidden()` answered false, but its `GetParent()` raised
"calling 'GetParent' on bad self (Attempt to access forbidden object from
code tainted by an AddOn - Usage: local parent = self:GetParent())".
Blizzard's aura container code restricts each button after the addon's
`initializeFrame` (`DenyTaintedAccessWhenAurasAreSecret`), and wowlua now
applies that restriction at all times. These steps check what is still
guessed:

- what the button answers inside `initializeFrame` and after;
- whether the restriction lifts out of combat, where
  `C_Secrets.ShouldAurasBeSecret()` was false;
- which other calls hand the button out.

Steps 1 to 8 are out of combat; steps 9 and 10 need a hostile creature a
few levels below you to fight. Each line in the chat, one at a time.

1. Make an aura container of your own:

   ```
   /run AP={} AC=CreateFrame("AuraContainer",nil,UIParent,"CustomAuraContainerTemplate") AC:SetSize(1,1) AC:SetPoint("CENTER")
   ```

2. Give it a group of your buffs; its `initializeFrame` keeps the button,
   its answers there, and a frame put on it (the last of the batch of
   buttons the container makes at once):

   ```
   /run AC:AddAuraGroup("b","HELPFUL",{maxFrameCount=1,initializeFrame=function(b) AP.b=b AP.f=b:IsForbidden() AP.k=CreateFrame("Frame",nil,b) AP.p=AP.k:GetParent()==b AP.s=pcall(b.SetSize,b,20,20) end}) AC:SetUnit("player")
   ```

3. Print what `initializeFrame` saw:

   ```
   /run print("inside",AP.b~=nil,AP.f,AP.p,AP.s)
   ```

   Seen: (to do). wowlua: `inside true false true true` (the button not
   forbidden yet; the frame's parent is the button; `SetSize` works).
4. Out of combat, ask the button itself:

   ```
   /run local b=AP.b print("after",InCombatLockdown(),C_Secrets.ShouldAurasBeSecret(),pcall(b.IsForbidden,b)) print(pcall(b.GetWidth,b))
   ```

   Seen: (to do). wowlua: `after false false true true`, then `false
   calling 'GetWidth' on bad self (Attempt to access forbidden object from
   code tainted by an AddOn - Usage: local width =
   self:GetWidth([ignoreRect]))`. If the game answers `true 20` on the
   second line, the restriction lifts while auras are not secret.
5. Ask the frame on the button for its parent:

   ```
   /run local k=AP.k print("kid",k:IsForbidden(),pcall(k.GetParent,k))
   ```

   Seen: (to do). wowlua: `kid false false calling 'GetParent' on bad self
   (Attempt to access forbidden object from code tainted by an AddOn -
   Usage: local parent = self:GetParent())`.
6. Does the container's `GetChildren` hand the button out:

   ```
   /run for i,f in ipairs({AC:GetChildren()}) do print("child",i,f==AP.b,pcall(f.IsForbidden,f)) end
   ```

   Seen: (to do). wowlua: ten lines, `child 1 false true true` to
   `child 10 true true true`: the batch of ten buttons the container makes
   at once, all handed out and forbidden, the last one the button step 2
   kept.
7. Does `EnumerateFrames` hand it out:

   ```
   /run local f,n=EnumerateFrames(),0 while f do if f==AP.b then n=n+1 end f=EnumerateFrames(f) end print("listed",n)
   ```

   Seen: (to do). wowlua: `listed 0`.
8. Anchor the frame to the button, then ask for its anchor:

   ```
   /run local k=AP.k print("anchor",pcall(k.SetPoint,k,"CENTER",AP.b)) print(pcall(k.GetPoint,k,1))
   ```

   Seen: (to do). wowlua: `anchor true`, then `true CENTER table: …
   CENTER 0 0` (`GetPoint` hands the button out).
9. Attack the creature so you are in combat, then repeat step 4's line and
   step 5's line.

   Seen: (to do). wowlua: as out of combat, but `after true true …` on
   step 4's first line.
10. After the fight, put the container away:

    ```
    /run AC:SetUnit("none") AC:Hide()
    ```

## Lines under a pixel (decision T633, 2026-11-12)

Found in game: SuuUI 0.8.0's nameplate debuff icons lost most of their
borders (a player's screenshot: fragments of some edges left). The borders
are textures that don't snap to the pixel grid, 0.71 of a pixel thick.
wowlua draws such a texture as the GPU fills a triangle: a pixel is filled
when its centre lies inside the texture, so a line under a pixel shows
where it crosses a row's centre and vanishes where it lies between two.
A texture that snaps has its edges rounded to whole pixels (a guess).
These steps check both on screen: a red line 600 pixels wide, its top 300
pixels below the screen's top, nudged by fractions of a pixel. Each line
in the chat, one at a time; take a screenshot after each step that moves
the line, and zoom in if it is hard to see.

1. Make a frame over the whole screen and print one pixel in UIParent's
   units:

   ```
   /run SPU=768/select(2,GetPhysicalScreenSize())/UIParent:GetEffectiveScale() SPF=CreateFrame("Frame",nil,UIParent) SPF:SetAllPoints() SPF:SetFrameStrata("TOOLTIP") print("pixel",SPU)
   ```

   Seen: (to do). wowlua: about `pixel 1` with "Use UI Scale" off (UIParent
   then lays out in screen pixels), more at a smaller UI scale.
2. Make the line, half a pixel thick, not snapping:

   ```
   /run SPT=SPF:CreateTexture(nil,"OVERLAY") SPT:SetSnapToPixelGrid(false) SPT:SetTexelSnappingBias(0) SPT:SetColorTexture(1,0,0) SPT:SetSize(600*SPU,0.5*SPU) print(SPT:IsSnappingToPixelGrid())
   ```

   Seen: (to do). wowlua: `false`.
3. Its top 300.25 pixels down (it covers the centre of row 300):

   ```
   /run SPT:ClearAllPoints() SPT:SetPoint("TOPLEFT",SPF,"TOPLEFT",200*SPU,-300.25*SPU)
   ```

   Seen: (to do). wowlua: a red line one pixel thick, full red.
4. Its top 300.6 pixels down (between the centres of rows 300 and 301):

   ```
   /run SPT:ClearAllPoints() SPT:SetPoint("TOPLEFT",SPF,"TOPLEFT",200*SPU,-300.6*SPU)
   ```

   Seen: (to do). wowlua: no line at all. A faint line here means the game
   blends by coverage instead.
5. Its top 301.1 pixels down (it covers the centre of row 301):

   ```
   /run SPT:ClearAllPoints() SPT:SetPoint("TOPLEFT",SPF,"TOPLEFT",200*SPU,-301.1*SPU)
   ```

   Seen: (to do). wowlua: the line again, one pixel lower than in step 3.
6. A whole pixel thick, its top 300.6 pixels down:

   ```
   /run SPT:SetHeight(SPU) SPT:ClearAllPoints() SPT:SetPoint("TOPLEFT",SPF,"TOPLEFT",200*SPU,-300.6*SPU)
   ```

   Seen: (to do). wowlua: a line one pixel thick (row 301).
7. Half a pixel thick again, now snapping, its top 300.25 pixels down:

   ```
   /run SPT:SetHeight(0.5*SPU) SPT:SetSnapToPixelGrid(true) SPT:ClearAllPoints() SPT:SetPoint("TOPLEFT",SPF,"TOPLEFT",200*SPU,-300.25*SPU)
   ```

   Seen: (to do). wowlua: a line one pixel thick (its edges rounded to 300
   and 301).
8. Snapping, its top 300.6 pixels down:

   ```
   /run SPT:ClearAllPoints() SPT:SetPoint("TOPLEFT",SPF,"TOPLEFT",200*SPU,-300.6*SPU)
   ```

   Seen: (to do). wowlua: no line (both edges rounded to 301).
9. Put the line away:

   ```
   /run SPF:Hide()
   ```

## The nameplates' scale (decisions T633, T634, 2026-11-12)

SuuUI lays its nameplate lines out for the scale the game draws plates at,
and can't lay a debuff icon's lines again once the game has made the icon
forbidden, so a plate drawn at another scale later thins them under a
pixel. wowlua put the plates on WorldFrame at a scale of 1; the game's
scale was unknown (SuuUI found it isn't WorldFrame's and guessed UIParent's),
and so is whether it changes with distance or the target. These steps read
it. Stand where enemy nameplates show, some near and some farther than 20
yards, without moving. Each line in the chat, one at a time.

Step 1 was answered on 2026-11-12 (WoW Forever 1.60.1, two hostile
plates): the plates draw at UIParent's scale, and wowlua now does too
(decision T634). SuuUI sets `nameplateMinScale`, `nameplateMaxScale` and
`nameplateSelectedScale` to 1, so steps 2 to 4 stay open: whether the
scale changes with distance or the target at the settings' defaults
(wowlua models neither). Run them with SuuUI's nameplates off, or with
those settings back at their defaults.

1. Every plate's scale and effective scale, then UIParent's and
   WorldFrame's:

   ```
   /run for _,p in ipairs(C_NamePlate.GetNamePlates()) do print(p:GetName(),p:GetScale(),p:GetEffectiveScale()) end print(UIParent:GetEffectiveScale(),WorldFrame:GetEffectiveScale())
   ```

   Seen: WoW Forever (2026-11-12): `NamePlate10 0.63999998569489
   0.63999998569489`, `NamePlate39 0.63999998569489 0.63999998569489`,
   then `0.63999998569489 1` (UIParent's scale and effective scale the
   same as the plates'). wowlua said `NamePlate1 1 1`; it now gives each
   plate UIParent's effective scale over WorldFrame's
   (`NamePlate1 0.71111112833023 0.71111112833023` on a 1920 by 1080
   screen at the default UI scale), then that and `1`.
2. The nameplate scale settings:

   ```
   /run for _,c in ipairs({"nameplateGlobalScale","nameplateMinScale","nameplateMaxScale","nameplateSelectedScale","nameplateLargerScale"}) do print(c,GetCVar(c)) end
   ```

   Seen: (to do). wowlua: the client's defaults as recorded.
3. Target a near enemy, then a far one, and after each read the target's
   plate:

   ```
   /run local t=C_NamePlate.GetNamePlateForUnit("target") print("target",t and t:GetName(),t and t:GetEffectiveScale())
   ```

   Seen: (to do, at the settings' defaults). wowlua: `target NamePlate1`
   and UIParent's effective scale (the plate's name varies), the same near
   and far.
4. Print each plate's effective scale when it comes and a second later:

   ```
   /run NPP=CreateFrame("Frame") NPP:RegisterEvent("NAME_PLATE_UNIT_ADDED") NPP:SetScript("OnEvent",function(_,_,u) local p=C_NamePlate.GetNamePlateForUnit(u) local f=function() print(u,p:GetEffectiveScale()) end f() C_Timer.After(1,f) end)
   ```

   Then turn the camera away from the enemies and back, so their plates
   go and come again.

   Seen: (to do, at the settings' defaults). wowlua: `nameplate1` and
   UIParent's effective scale twice for each plate, a second apart. Different numbers in a plate's two lines mean the game changes a
   plate's scale after `NAME_PLATE_UNIT_ADDED`.
5. Stop the printing frame:

   ```
   /run NPP:UnregisterAllEvents()
   ```

## UnitIsTapDenied of a mob tagged by another (decision T635, 2026-11-12)

wowlua answered `UnitIsTapDenied` false for every unit, so an addon's look
for a mob someone else tagged (SuuUI greys its plate and target frame)
could not be tested. It now keeps who tagged an NPC first: the first
player to hit it, or a test's `tappedByOther` / `unit:tap()`. A mob tagged
by someone outside your group answers true, and `UNIT_FACTION` fires for
the mob when that answer changes (the recorded `combat` scenario has none
for your own tag). These steps check what is guessed: the event and when
it comes, the tag on the corpse, and the tag lost when the
mob resets. They need an area where other players fight mobs a few levels
below you, outside your group. Each line in the chat, one at a time.

1. Make a frame that prints the target's faction, flags and combat events
   with its tap state:

   ```
   /run TDP=CreateFrame("Frame") for _,e in ipairs({"UNIT_FACTION","UNIT_FLAGS","UNIT_COMBAT"}) do TDP:RegisterUnitEvent(e,"target") end TDP:SetScript("OnEvent",function(_,e,u) print(format("%.3f",GetTime()),e,u,UnitIsTapDenied("target")) end)
   ```

2. Target a hostile mob no one is fighting (its health bar is not grey),
   without attacking it, and read it:

   ```
   /run print(format("%.3f",GetTime()),"tap",UnitIsTapDenied("target"),UnitPlayerControlled("target"),UnitAffectingCombat("target"),UnitIsDead("target"),UnitIsPlayer("target"))
   ```

   Seen: (to do). wowlua: `tap false false false false false`.
3. Attack it and kill it. Watch the lines from step 1 as the first blow
   lands.

   Seen: (to do) whether a `UNIT_FACTION` or `UNIT_FLAGS` line comes with
   your first blow. wowlua: no `UNIT_FACTION` (none in the recorded
   `combat` scenario).
4. Target a mob another player outside your group is fighting (its health
   bar grey), and run the line from step 2 again.

   Seen: (to do). wowlua: `tap true false true false false` (another
   player's `hit` in a test). Best: target a mob another player is about
   to attack before they do, and watch the lines from step 1 as their
   first blow lands. Seen: (to do) whether `UNIT_FACTION` comes with it,
   before or after its `UNIT_COMBAT`. wowlua: `UNIT_FACTION target true`
   before the blow's `UNIT_COMBAT`.
5. Keep it targeted. When it dies, run the line from step 2 again.

   Seen: (to do) whether the corpse still answers `true` (its tag kept).
   wowlua: `tap true` with `UnitIsDead` true (the tag kept).
6. If you see a mob someone else tagged reset (the player dies or runs
   away and the mob walks back, its health bar no longer grey), target it
   before it does, and watch the lines from step 1 as it resets; then run
   the line from step 2.

   Seen: (to do) whether `UNIT_FACTION` (or `UNIT_FLAGS`) comes when the
   tag goes, and the answer after. wowlua: the tag stays until a test's
   `unit:untap()`, which fires `UNIT_FACTION`.
7. Stop the printing frame:

   ```
   /run TDP:UnregisterAllEvents()
   ```

Open: a missed first blow tagging, and a tagger joining your group while
the mob lives (wowlua: the mob is no longer tap-denied, without an event).
