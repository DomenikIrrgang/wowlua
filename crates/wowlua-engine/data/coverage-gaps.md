What the engine still answers differently from the client, by cause, and what
would close each. The counts are the full probe's misses
(`crates/wowlua-cli/tests/fullprobe-misses.txt`) and the replays' leftovers
above; this section is written by hand (`crates/wowlua-engine/data/coverage-gaps.md`).

### Session variance

Answers that differ between the client's own sessions, so no fixed answer
matches every recording.

- **Aura instance IDs.** Each session numbers its auras anew: the
  `C_UnitAuras` answers by instance ID, slot or index (88 full-probe misses)
  and the aura tooltips (`C_TooltipInfo.GetUnitAura`, `GetUnitBuff`, by
  instance ID: 30). Closed by comparing an aura by its spell and position
  instead of its ID, or by recording the probe's own aura list with each
  answer.
- **Clocks.** `GetQuestResetTime`, the calendar's month and day events, and
  `C_DateAndTime.GetCalendarTimeFromEpoch` (4), which follows the machine's
  time zone. Closed by recording the session's clock and time zone with the
  probe (`date("%H:%M")` against `date("!%H:%M")`, [probes](../tests/ingame/PROBES.md)).
- **The client's own measurements.** `C_AddOnProfiler` (49) times the
  client's frames; `GetNumModifiedClickActions` answered 380, 500 and 820 in
  three sessions. Nothing to model: compared by shape at most.

### Game data the engine lacks

- **Spells.** Power cost, range, cast time, base cooldown
  (`GetSpellBaseCooldown`) and descriptions, and the tooltips' spell lines.
  Closed by a spell data probe over the class spells
  (`C_Spell.GetSpellPowerCost`, `C_Spell.GetSpellInfo`) into
  `data/camelot-spells.txt`.
- **Item sets.** `C_LootJournal.GetItemSets` and `C_Item.GetItemSetInfo` (31).
  Closed by recording the loot journal's sets.
- **Transmog category totals.** `C_TransmogCollection.GetFilteredCategoryTotal`
  and `GetFilteredCategoryCollectedCount` (17 of the namespace's 24). Closed by
  a probe of each category with the default filters.
- **Key bindings.** `GetNumBindings` (8; the client has 290) and the binding
  list. Closed by the binding list probe.
- **Tooltips.** The item, recipe, heirloom and toy tooltips' lines
  (`C_TooltipInfo`, 90 with the aura tooltips): the engine builds tooltips
  from its item data, which lacks most lines. Closed by recording
  `C_TooltipInfo` for the recorded items and recipes.
- **Other systems.** `GetLFGDungeonInfo`, the model scenes
  (`C_ModelInfo.GetModelScene*`), the calendar's holidays
  (`C_Calendar.GetHolidayInfo`, `GetDayEvent`), PvP talents by level
  (`GetNumUnspentPvpTalents`), the allied races' racial abilities, and
  `C_ActionBar.IsHarmfulAction` (4), which needs each spell's harmfulness
  (`C_Spell.IsSpellHarmful`). Each closes with a recording of its data.

### State not recorded

The full probe ran from about 01:00 to 02:44; the replay starts from the last
snapshot before it (00:43) and what the probe's own answers say changed.

- Battle.net friends (`C_BattleNet`, 21): the account's, scrubbed and not in
  the snapshot.
- Item GUIDs (`C_Item.GetItemGUID`): the server's.
- Quest 1492's log entry (its reward money and experience), the bind
  location, the mail senders (`GetLatestThreeSenders`), the chat windows'
  saved places, the stats' latest update (`GetLatestUpdatedStats`).
- The account's Edit Mode layouts (the hearth replay's one line).

Closed by a snapshot right before and right after the next full probe
([probes](../tests/ingame/PROBES.md)).

### Scenario leftovers

- **quest** (53 lines): one-off orders of the server's messages within a
  frame, which the recording shows once and the engine cannot predict.
- **profession** (2 lines): the order of the bronze item counts after a
  smelt. Closed by the crafting scenario in the probes.
- **death** (1 line): `ADDON_LOADED(Blizzard_PlayerSpells)`, loaded on
  demand at a moment the engine does not reproduce.
- **hearth** (1 line): the account's Edit Mode layouts, not in the snapshot.
- **trade**: recorded, without a replay script yet.

### Other flavors

Only camelot's Blizzard UI code loads without an error in the engine (every
replay asserts it). Measured on 2026-10-20 with each flavor's UI code from
`cargo xtask types fetch` (no test counts these yet): standard 648 errors,
mists 475, wrath 481, tbc 430, vanilla 426, cata 18. The first of them come
from tables and globals the engine has for camelot (from its documentation and
recordings) and lacks for the others (`Enum.GameMode`, `Enum.VasServiceType`,
`PLAYER_FACTION_COLOR_HORDE`).
Closed per flavor: a test that loads its UI code with a ratchet on the
errors, then its missing tables and globals, then probes on that client.

### Scrubbing

`cargo xtask ingest` scrubs what identifies a player: the recordings hold
`Player` or `<scrubbed>` where the client gave a name, and
`Player-0000-00000000` where it gave a player's GUID. The engine answers with
its default character's name and GUID, so an answer that holds them differs
unless the comparison blanks both sides, which it does only for the social
systems' strings. Closed by blanking the engine's side as ingest blanks the
client's, or by recording only what the answers battery's name check records:
whether a name is empty, its length and whether it equals the player's own.

### Widget methods

The method probe compares only the 765 fresh-widget getters recorded so far.
Closed by the method probe run (`/stdtest engine methodprobe`, about 13,000
calls on 53 widget kinds).
