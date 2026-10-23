# In-game probes to run

The probes that would close the emulator's open gaps
([coverage report](../../docs/emulator-coverage.md)), for WoW Forever (the
camelot flavor). Install a current StdTest build first (`wowlua install
--client _classic_beta_` in `tests/ingame`), then `/reload` in game so it
loads.

Everything here is read-only: no item is used, nothing is bought, moved or
sent, the character does not move, and no output stores a player's name.
After a run that StdTest records, `/reload` writes the results to the saved
variables; `cargo xtask ingest` then takes them into the fixtures.

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
6. **How the client breaks lines**: `/stdtest engine wrap` (a few
   seconds, read-only), then `/reload`. Why: text is measured with the
   client's glyphs (decision T237), but how it wraps is the engine's guess:
   by words at spaces, a word wider than the string on a line of its own
   unless `SetNonSpaceWrap(true)` breaks it between characters. It records
   `GetNumLines`, `GetStringHeight`, `GetStringWidth` and `IsTruncated`
   for long words, hyphens, several spaces, colour codes and sentences at
   four widths, with and without `SetNonSpaceWrap`, free and held to two
   lines; kerning pairs at FRIZQT 10/14/20 and ARIALN 10/12/16; the
   outlined digits (StdTestState.engineWrap; read by hand, no ingest yet).
7. **An early landing** (the scenario recorder as for the taxi): at a
   flight master, pick a route with a stop on the way; type
   `/stdtest engine scenario earlylanding`, take the flight, and before the
   first stop `/run TaxiRequestEarlyLanding()`; once landed,
   `/stdtest engine scenario stop` and `/reload`. Why: the engine has no
   early landing; SuuUI's Land at is tested with it stubbed.
8. **Atlases on a 1x screen**: the answers battery's atlas checks at a
   1920x1080 screen with Windows' DPI scale at 100%. Why: both recordings
   were on high-DPI screens and gave the 2x textures' files and
   coordinates (decision T238); a 1x screen may take the 1x textures (the
   sizes are the same either way).
9. **Talents the engine doesn't know yet**: out of combat, with an unspent
   talent point if you have one, `/stdtest engine talents2` (two seconds),
   then `/reload`. It records both spec groups' configs
   (`GetCombatConfigIDForSpecGroup`), a rank planned and taken back
   (`C_Traits.PurchaseRank`, `RefundRank`, then `RollbackConfig`: nothing
   is applied), another class's talents in the view config
   (`InitializeViewLoadout`, `ViewLoadout`), and the events between
   (StdTestState.engineTalents2). On a character about to reach level 5:
   `/stdtest engine talentwatch` first, level up, then `/reload`: every
   `PLAYER_TALENT_UPDATE` and `PLAYER_LEVEL_UP` with the time and level
   (StdTestState.engineTalentWatch). Why: SuuUI's Talents tests use a
   stand-in for these.
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

- **`GetBindingText(key, 1)` with modifiers** (the action bars' hotkeys):
  `SHIFT-1`, `CTRL-ALT-2`, `META-E`, `SHIFT-MOUSEWHEELUP`, `SHIFT-NUMPAD1`.
  Why: the engine shortens the modifiers with the client's
  `*_KEY_TEXT_ABBR` strings and joins them with `-` (`s-1`), and keeps the
  key's own name whole; neither the joining nor the key names are
  recorded (M12's default-UI screenshot showed the hotkeys cut to `SHI…`).

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

- Before turning in a quest with an item reward:
  `/run local f=CreateFrame("Frame") f:RegisterEvent("QUEST_TURNED_IN") f:SetScript("OnEvent",function() print(GetTime(), C_Container.GetContainerNumFreeSlots(0)) C_Timer.After(0.2,function() print(GetTime(), C_Container.GetContainerNumFreeSlots(0)) end) end)`:
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
2. With the collections open:
   `/run local m=WardrobeCollectionFrame.ItemsCollectionFrame.Models[1] print(m:GetCameraPosition()) print(m:GetCameraTarget()) print(m:GetPosition()) print(m:GetFacing(), m:GetPitch(), m:GetRoll()) print(m:IsUsingModelCenterToTransform()) local v=m:TransformCameraSpaceToModelSpace(CreateVector3D(4,0,0)) print(v:GetXYZ())`
3. `/run local m=CreateFrame("PlayerModel") m:SetSize(100,100) m:SetUnit("player") print(m:GetCameraDistance(), m:GetCameraPosition()) print(m:GetCameraTarget())`
   Answers the model frame's own camera and field of view.

## Textures the screenshots draw

1. `/run local t=MainStatusTrackingBarContainer.bars and MainStatusTrackingBarContainer.bars[1] or nil print(t and t.StatusBar:GetStatusBarTexture():GetAtlas(), t and t.StatusBar:GetStatusBarTexture():GetTexture())`
   with the experience bar shown: whether `SetStatusBarTexture(atlasName)`
   makes the texture's atlas (wowlua: it does; `GetTexture` answers nil).
2. An action button with a spell: `/run local b=ActionButton1 print(b.icon:GetDrawLayer()) print(b.SlotArt:GetDrawLayer()) print(b.icon:GetNumMaskTextures())`
   and a screenshot of it hovered: wowlua draws a masked texture over the
   unmasked ones of its layer and sublevel (the icon over the slot art).
   Answered (2026-10-23): both `BACKGROUND` 0, one mask, as in wowlua;
   still open: the hovered screenshot (which of the two is drawn on top).
3. `/run local t=UIParent:CreateTexture() print(t:IsSnappingToPixelGrid(), t:GetTexelSnappingBias())`:
   a fresh texture's pixel snapping (wowlua: true, 0). Answered
   (2026-10-23): true, 0.30000001192093; wowlua now gives the same.

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
   (2 points, the parent's size), leaves a Lua one unplaced.
2. Open Settings, Mouse, hover the Mouse Look Speed slider with
   `/fstack` on, and note the thumb's left relative to the slider at the
   value shown, at the minimum and at the maximum (wowlua: it travels the
   slider's width less the thumb's, by the value). Answered for a
   horizontal slider made in Lua (2026-10-23): it does, by one point,
   `LEFT` to the slider's `LEFT`; still open: a vertical slider's point.


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
- Still owed: the skill bars' fill (Blizzard's `ColoredProgressBar` sets
  its fill by `FillArea:GetWidth()`; in game a 49/105 bar looks full).
