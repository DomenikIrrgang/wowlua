//! Group content the engine's world has little or none of: instance
//! difficulties, saved instances, loot rolls, the dungeon finder and the
//! group finder's listings, battlegrounds and arenas, PvP seasons,
//! challenge modes and mythic keystones, scenarios, the encounter
//! journal, the calendar's events, delves and weekly rewards.
//!
//! What the world models is answered from it: the instance the player is
//! in (`encounters.rs`), the party and its members (`party.rs`,
//! `units.rs`), the loot window (`loot.rs`). Difficulties, the loot
//! threshold and the dungeon finder's roles are kept here (in the VM's
//! registry, `wowlua.engine.content`). Everything else answers the state
//! of a character with none of it: no queue, no listing, no roll, no
//! keystone, no rating, no event; a list empty, a count 0, an `Is…`
//! false, a `Get…Info` of an unknown ID nothing, a structure its
//! documented empty values (unverified unless said otherwise: WoW Forever
//! has no dungeon finder, mythic keystones, rated PvP or delves; its
//! answers to these were not recorded). Actions with no visible effect
//! here are accepted and do nothing.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, PLAYER};
use crate::{engine, widgets};

// ------------------------------------------------------ the shared answers

/// Nothing at all (an action; a `Get…Info` of something that is not there).
fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// A single nil.
fn nil(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `C_PartyInfo.DoCountdown(seconds)`: the player's countdown starts,
/// `START_PLAYER_COUNTDOWN(guid, seconds, seconds, true, name)` told
/// within the call (SuuUI probe countdown, ten samples: the time left the
/// whole, chat informed, the name without a realm; told before the call
/// returned per SuuUI's timeline notes). Unverified: what it returns, a
/// cancel (0 seconds), the rules on who may start one.
fn do_countdown(vm: &mut Vm, c: NativeCall) -> Ret {
    let seconds = match vm.hidden(c.arg(1)) {
        Value::Num(n) if n > 0.0 => n,
        _ => return Ok(vec![Value::Bool(false)]),
    };
    let p = &engine(vm).units.list[PLAYER.0];
    let (guid, name) = (p.guid.clone(), p.name.clone());
    let args = [
        vm.str(&guid),
        Value::Num(seconds),
        Value::Num(seconds),
        Value::Bool(true),
        vm.str(&name),
    ];
    widgets::fire_event(vm, "START_PLAYER_COUNTDOWN", &args);
    Ok(vec![Value::Bool(true)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// Whether a legacy function's first argument, an ID or index, is below 1
/// or not a number: the client answers nothing (the full probe: -1, 0 and
/// the wrong types for the dungeon finder's and PvP's indexes).
fn below_one(vm: &Vm, c: &NativeCall) -> bool {
    !matches!(vm.hidden(c.arg(1)), Value::Num(n) if n >= 1.0)
}

/// `GetLFGDungeonNumEncounters(dungeonID)`: one encounter, none done;
/// nothing for no ID (the full probe: 1, 0 for IDs 1 and 2; another
/// dungeon's count unverified).
fn lfg_dungeon_num_encounters(vm: &mut Vm, c: NativeCall) -> Ret {
    if below_one(vm, &c) {
        return Ok(vec![]);
    }
    Ok(vec![Value::Num(1.0), Value::Num(0.0)])
}

/// `IsLFGDungeonJoinable(dungeonID)`: joinable, for the player, not
/// locked (the full probe: true, true, false for IDs 1 and 2; nothing for
/// no ID; unverified for a dungeon the level locks).
fn is_lfg_dungeon_joinable(vm: &mut Vm, c: NativeCall) -> Ret {
    if below_one(vm, &c) {
        return Ok(vec![]);
    }
    Ok(vec![
        Value::Bool(true),
        Value::Bool(true),
        Value::Bool(false),
    ])
}

/// `GetBattlefieldTeamInfo(index)`: no team: an empty name for the first
/// two (0 and 1, and -1), none past them, nothing rated (the full probe).
fn battlefield_team_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = match vm.hidden(c.arg(1)) {
        Value::Num(n) if n <= 1.0 => vm.str(""),
        Value::Num(_) => Value::Nil,
        _ => return Ok(vec![]),
    };
    let mut out = vec![name];
    out.extend([Value::Num(0.0); 4]);
    Ok(out)
}

fn zeros4(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 4])
}

fn empty_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn empty_string(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

/// `GetBattlefieldStatus(index)`: no queue in that slot (`"none"`), no map,
/// no team, not registered or suspended, solo queue false (the
/// declarations' types, Ketho's annotations; unverified).
fn status_none(vm: &mut Vm, _c: NativeCall) -> Ret {
    let mut out = vec![
        vm.str("none"),
        Value::Nil,
        Value::Num(0.0),
        Value::Bool(false),
        Value::Bool(false),
    ];
    out.extend([Value::Nil; 6]);
    out.push(Value::Bool(false));
    Ok(out)
}

/// A documented function's documented empty values (a structure with its
/// fields' empty values, `false`, 0, nil where nilable): what Blizzard's
/// code was given so far, now the addons' answer too.
fn documented(vm: &mut Vm, c: NativeCall) -> Ret {
    let api = engine(vm).api;
    let f = match c.upvals.first().copied() {
        Some(Value::Num(i)) => api.functions.get(i as usize),
        Some(Value::Str(s)) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            api.functions.iter().find(|f| f.name == name)
        }
        _ => None,
    };
    match f {
        Some(f) => Ok(crate::defaults(vm, api, &f.doc)),
        None => Ok(vec![]),
    }
}

// ------------------------------------------------------------- the state

/// The registry table this module keeps its state in.
fn state(vm: &mut Vm) -> TableRef {
    let registry = vm.registry;
    let key = vm.heap.intern(b"wowlua.engine.content");
    if let Value::Table(t) = vm.heap.table(registry).get_str(key) {
        return t;
    }
    let t = vm.new_table();
    vm.heap.table_mut(registry).set_str(key, Value::Table(t));
    t
}

fn get_state(vm: &mut Vm, key: &str) -> Value {
    let t = state(vm);
    let k = vm.heap.intern(key.as_bytes());
    vm.heap.table(t).get_str(k)
}

fn set_state(vm: &mut Vm, key: &str, v: Value) {
    let t = state(vm);
    let k = vm.heap.intern(key.as_bytes());
    vm.heap.table_mut(t).set_str(k, v);
}

fn state_num(vm: &mut Vm, key: &str, default: f64) -> f64 {
    match get_state(vm, key) {
        Value::Num(n) => n,
        _ => default,
    }
}

fn state_bool(vm: &mut Vm, key: &str) -> bool {
    matches!(get_state(vm, key), Value::Bool(true))
}

fn truthy(v: Value) -> bool {
    !matches!(v, Value::Nil | Value::Bool(false))
}

// ------------------------------------------------------- the difficulties

/// A difficulty: its ID, name, instance kind, its heroic flag, the fewest
/// and most players.
struct Difficulty {
    id: f64,
    name: &'static str,
    kind: &'static str,
    heroic: bool,
    min: f64,
    max: f64,
}

/// The difficulties the client has (`/stdtest engine answers`: 1, 9 and
/// 148 answered; 2, 3, 4, 173 to 176 and 9999 nothing, so the retail and
/// later classic IDs warcraft.wiki.gg lists are not this client's). Its
/// raids are flagged heroic, as recorded; none challenge mode, shown as
/// heroic or mythic, toggling or LFR. Unverified: IDs not asked (5 to 8,
/// 14 and up but 148), which are taken to be missing as well.
const DIFFICULTIES: &[Difficulty] = &[
    Difficulty {
        id: 1.0,
        name: "Normal",
        kind: "party",
        heroic: false,
        min: 5.0,
        max: 5.0,
    },
    Difficulty {
        id: 9.0,
        name: "40 Player",
        kind: "raid",
        heroic: true,
        min: 40.0,
        max: 40.0,
    },
    Difficulty {
        id: 148.0,
        name: "20 Player",
        kind: "raid",
        heroic: true,
        min: 20.0,
        max: 20.0,
    },
];

fn difficulty(id: f64) -> Option<&'static Difficulty> {
    DIFFICULTIES.iter().find(|d| d.id == id)
}

/// `GetDifficultyInfo(difficultyID)`: the difficulty's name, instance
/// kind, flags, toggle (none), LFR, sizes and whether any is selectable
/// by the user (false: camelot's UI asks `HasAnyUserSelectableDifficulties`
/// with it); an unknown difficulty nothing (recorded, above).
fn get_difficulty_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_number(&c, 1)?;
    let Some(d) = difficulty(id) else {
        return Ok(vec![]);
    };
    Ok(vec![
        vm.str(d.name),
        vm.str(d.kind),
        Value::Bool(d.heroic),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        Value::Nil,
        Value::Bool(false),
        Value::Num(d.min),
        Value::Num(d.max),
        Value::Bool(false),
    ])
}

/// `GetDungeonDifficultyID()`: the dungeon difficulty chosen, normal (1)
/// until changed (recorded at load: 1).
fn get_dungeon_difficulty_id(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(state_num(vm, "dungeonDifficulty", 1.0))])
}

/// `SetDungeonDifficultyID(difficultyID)`: a dungeon difficulty chosen
/// outside an instance, `PLAYER_DIFFICULTY_CHANGED` when it changed
/// (unverified: the server's message, refusals in a group not led).
fn set_dungeon_difficulty_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_number(&c, 1)?;
    let valid = difficulty(id).is_some_and(|d| d.kind == "party");
    let inside = engine(vm).world.encounters.instance.is_some();
    if valid && !inside && state_num(vm, "dungeonDifficulty", 1.0) != id {
        set_state(vm, "dungeonDifficulty", Value::Num(id));
        widgets::fire_event(vm, "PLAYER_DIFFICULTY_CHANGED", &[]);
    }
    Ok(vec![])
}

/// The raid difficulty `SetRaidDifficultyID` chose, if any (for
/// `GetRaidDifficultyID`).
pub fn raid_difficulty(vm: &mut Vm) -> Option<f64> {
    match get_state(vm, "raidDifficulty") {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// `SetRaidDifficultyID(difficultyID [, force])`: a raid difficulty
/// chosen outside an instance, `PLAYER_DIFFICULTY_CHANGED` when it
/// changed (unverified, as the dungeon's).
fn set_raid_difficulty_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_number(&c, 1)?;
    let valid = difficulty(id).is_some_and(|d| d.kind == "raid");
    let inside = engine(vm).world.encounters.instance.is_some();
    if valid && !inside && raid_difficulty(vm) != Some(id) {
        set_state(vm, "raidDifficulty", Value::Num(id));
        widgets::fire_event(vm, "PLAYER_DIFFICULTY_CHANGED", &[]);
    }
    Ok(vec![])
}

/// Chooses the legacy raid difficulty (for `SetLegacyRaidDifficultyID`).
pub fn set_legacy_raid_difficulty(vm: &mut Vm, id: f64) {
    set_state(vm, "legacyRaidDifficulty", Value::Num(id));
}

/// `GetLegacyRaidDifficultyID()`: the legacy raid difficulty chosen, none
/// (0) until changed (recorded at load: 0).
fn get_legacy_raid_difficulty_id(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(state_num(vm, "legacyRaidDifficulty", 0.0))])
}

/// `C_RaidLocks.GetRedirectedDifficultyID(mapID, difficultyID)`: the
/// difficulty itself, nothing redirected (unverified).
fn get_redirected_difficulty_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_number(&c, 2)?;
    Ok(vec![Value::Num(id)])
}

/// `C_EncounterJournal.GetBaseDifficultyID(difficultyID)`: the difficulty
/// itself (unverified).
fn get_base_difficulty_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_number(&c, 1)?;
    Ok(vec![Value::Num(id)])
}

/// `CanShowResetInstances()`: false, no instance run to reset (recorded
/// alone outside an instance; the engine keeps no instances run, so the
/// answer after one, alone or leading, is unverified).
fn can_show_reset_instances(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

// ------------------------------------------------------------ the party

/// The group's members with the player, when grouped.
fn members(vm: &Vm) -> Vec<units::UnitId> {
    if !crate::party::grouped(vm) {
        return Vec::new();
    }
    let mut out = vec![PLAYER];
    out.extend(engine(vm).units.group.iter().copied());
    out
}

/// `C_PartyInfo.IsGUIDInGroup(guid)`: the player or a member, when
/// grouped.
fn is_guid_in_group(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(s) = vm.hidden(c.arg(1)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let guid = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let list = &engine(vm).units.list;
    let found = members(vm).iter().any(|id| list[id.0].guid == guid);
    Ok(vec![Value::Bool(found)])
}

/// `C_PartyInfo.CanInvite()`: alone, or leading the group (unverified:
/// a raid's assistants).
fn can_invite(vm: &mut Vm, _c: NativeCall) -> Ret {
    let allowed = !crate::party::grouped(vm) || engine(vm).world.party.leader;
    Ok(vec![Value::Bool(allowed)])
}

/// `C_PartyInfo.GetMinLevel()`: the lowest level in the group, the
/// player's alone (unverified).
fn get_min_level(vm: &mut Vm, _c: NativeCall) -> Ret {
    let list = &engine(vm).units.list;
    let level = members(vm)
        .iter()
        .map(|id| list[id.0].level)
        .fold(list[PLAYER.0].level, f64::min);
    Ok(vec![Value::Num(level)])
}

/// Classic's loot methods: free for all, round robin, master looter,
/// group, need before greed (`Enum.LootMethod` 0 to 4; personal loot, 5,
/// left out: unverified).
const LOOT_METHODS: [f64; 5] = [0.0, 1.0, 2.0, 3.0, 4.0];

/// `C_PartyInfo.GetAvailableLootMethods()`.
fn get_available_loot_methods(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = vm.new_table();
    for (i, m) in LOOT_METHODS.iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(i as f64 + 1.0), Value::Num(*m));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_PartyInfo.IsLootMethodAvailable(method)`.
fn is_loot_method_available(vm: &mut Vm, c: NativeCall) -> Ret {
    let m = vm.check_number(&c, 1)?;
    Ok(vec![Value::Bool(LOOT_METHODS.contains(&m))])
}

// ------------------------------------------------------------- the loot

/// `GetLootThreshold()`: uncommon (2) until changed (recorded: the party
/// was formed with group loot at uncommon; unverified alone).
fn get_loot_threshold(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(state_num(vm, "lootThreshold", 2.0))])
}

/// `SetLootThreshold(threshold)`: the leader's party's threshold,
/// `PARTY_LOOT_METHOD_CHANGED` (unverified: the event, the server's
/// message, the allowed range uncommon to epic).
fn set_loot_threshold(vm: &mut Vm, c: NativeCall) -> Ret {
    let threshold = vm.check_number(&c, 1)?;
    let leading = crate::party::grouped(vm) && engine(vm).world.party.leader;
    if leading && (2.0..=4.0).contains(&threshold) {
        set_state(vm, "lootThreshold", Value::Num(threshold));
        widgets::fire_event(vm, "PARTY_LOOT_METHOD_CHANGED", &[]);
    }
    Ok(vec![])
}

/// `GetLootInfo()`: the open loot window's slots as tables (`texture`,
/// `item`, `quantity`, `quality`, `locked`, `isQuestItem`, `roll`:
/// warcraft.wiki.gg's fields; the values `GetLootSlotInfo` gives),
/// a slot taken an empty entry (unverified).
fn get_loot_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    let out = vm.new_table();
    let items = {
        let l = &engine(vm).world.loot;
        if l.from.is_some() {
            l.items.clone()
        } else {
            Vec::new()
        }
    };
    for (i, item) in items.iter().enumerate() {
        let entry = vm.new_table();
        if !item.taken {
            let icon = crate::character::item_field(vm, item.id, "icon");
            let name = crate::character::item_field(vm, item.id, "name");
            let quality = match crate::character::item_field(vm, item.id, "quality") {
                Value::Nil => Value::Num(1.0),
                q => q,
            };
            let quest = crate::quests::is_quest_item(vm, item.id);
            for (k, v) in [
                ("texture", icon),
                ("item", name),
                ("quantity", Value::Num(item.count)),
                ("quality", quality),
                ("locked", Value::Bool(false)),
                ("isQuestItem", Value::Bool(quest)),
                ("roll", Value::Bool(false)),
            ] {
                let k = vm.str(k);
                let _ = vm.heap.table_mut(entry).set(k, v);
            }
        }
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num(i as f64 + 1.0), Value::Table(entry));
    }
    Ok(vec![Value::Table(out)])
}

// ------------------------------------------------- the dungeon finder

/// `GetLFGRoles()`: leader, tank, healer, damage as `SetLFGRoles` chose
/// them, at first as the `lfgSelectedRoles` CVar keeps them, bits 1, 2, 4,
/// 8 (the full probe: false, true, true, false with it "6").
fn get_lfg_roles(vm: &mut Vm, _c: NativeCall) -> Ret {
    let keys = ["lfgLeader", "lfgTank", "lfgHealer", "lfgDamage"];
    if get_state(vm, keys[0]).is_nil() {
        let bits = crate::framescript::cvar_get(vm, "lfgSelectedRoles")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        return Ok((0..4).map(|i| Value::Bool(bits & 1 << i != 0)).collect());
    }
    Ok(keys
        .iter()
        .map(|k| Value::Bool(state_bool(vm, k)))
        .collect())
}

/// `SetLFGRoles(leader, tank, healer, damage)`: kept (unverified: the
/// class's allowed roles, `LFG_ROLE_UPDATE`).
fn set_lfg_roles(vm: &mut Vm, c: NativeCall) -> Ret {
    for (i, k) in ["lfgLeader", "lfgTank", "lfgHealer", "lfgDamage"]
        .iter()
        .enumerate()
    {
        let v = Value::Bool(truthy(c.arg(i + 1)));
        set_state(vm, k, v);
    }
    Ok(vec![])
}

/// `GetPVPRoles()`: tank, healer, damage: none (unverified).
fn get_pvp_roles(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false); 3])
}

/// `GetLFGRoleShortageRewards(dungeonID, shortageIndex)`: not eligible,
/// for no role, no items, money, experience or a fourth count (the full
/// probe: four falses, four zeros).
fn get_lfg_role_shortage_rewards(_vm: &mut Vm, _c: NativeCall) -> Ret {
    let mut out = vec![Value::Bool(false); 4];
    out.extend([Value::Num(0.0); 4]);
    Ok(out)
}

/// `GetLFGDungeonRewards(dungeonID)`: not done today, no money,
/// experience or rewards (warcraft.wiki.gg's results).
fn get_lfg_dungeon_rewards(vm: &mut Vm, c: NativeCall) -> Ret {
    if below_one(vm, &c) {
        return Ok(vec![]);
    }
    // Seven values (the full probe: a sixth count).
    let mut out = vec![Value::Bool(false)];
    out.extend([Value::Num(0.0); 6]);
    Ok(out)
}

/// `GetLFGBootProposal()`: no vote under way, false alone (the full
/// probe; the newest answer, over warcraft.wiki.gg's eight results).
fn get_lfg_boot_proposal(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `GetLFGQueuedList(category [, table])`: the table given (else a new
/// one), no dungeon queued in it (unverified).
fn get_lfg_queued_list(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = match c.arg(2) {
        Value::Table(t) => t,
        _ => vm.new_table(),
    };
    Ok(vec![Value::Table(t)])
}

/// `C_LFGList.GetNumApplications()`: none, none active.
fn get_num_applications(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 2])
}

/// `C_LFGList.GetApplicationInfo(searchResultID)`: the ID, not applied
/// (`"none"`), no pending status, no time, no role (warcraft.wiki.gg's
/// results; unverified).
fn get_application_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![
        c.arg(1),
        vm.str("none"),
        Value::Nil,
        Value::Num(0.0),
        vm.str("NONE"),
    ])
}

// ------------------------------------------------------------- the PvP

/// `GetPVPTimer()`: 301000, the timer not running (warcraft.wiki.gg: 301000
/// or -1 when not flagged; ElvUI tests both).
fn get_pvp_timer(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(301000.0)])
}

/// `GetPersonalRatedInfo(bracketIndex)`: no rating, games or tier; no
/// ranking (warcraft.wiki.gg's 15 results).
fn get_personal_rated_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if below_one(vm, &c) {
        return Ok(vec![]);
    }
    let mut out = vec![Value::Num(0.0); 8];
    out.push(Value::Bool(false));
    out.push(Value::Num(0.0));
    out.push(Value::Nil);
    out.extend([Value::Num(0.0); 4]);
    Ok(out)
}

/// `GetWorldPVPQueueStatus(index)`: no queue: status `"none"`, no map,
/// queue ID, expire time and average wait 0, the queued time, not
/// suspended (recorded for 1 and 2: seven values, the sixth 195499355,
/// read here as the client's clock in milliseconds as Blizzard's
/// `QueueStatusFrame.lua` names it `queuedTime`; unverified).
fn get_world_pvp_queue_status(vm: &mut Vm, c: NativeCall) -> Ret {
    if below_one(vm, &c) {
        return Ok(vec![]);
    }
    let queued = (crate::clock::now(vm) * 1000.0).floor();
    Ok(vec![
        vm.str("none"),
        Value::Nil,
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Num(queued),
        Value::Bool(false),
    ])
}

// ----------------------------------------------- challenge mode colours

/// A colour object (Blizzard's `CreateColor`), white; nil before
/// Blizzard's code defined it.
fn white(vm: &mut Vm) -> Ret {
    let create = vm.get_global("CreateColor");
    if !matches!(create, Value::Func(_)) {
        return Ok(vec![Value::Nil]);
    }
    let one = Value::Num(1.0);
    let color = vm.call(create, &[one, one, one, one])?;
    Ok(vec![color.first().copied().unwrap_or(Value::Nil)])
}

/// `C_ChallengeMode.Get…RarityColor(score or level)`: white, no score
/// or keystone being of any rarity (the documentation gives no results;
/// retail returns a colour object; unverified).
fn rarity_color(vm: &mut Vm, _c: NativeCall) -> Ret {
    white(vm)
}

// --------------------------------------------------------- the calendar

/// The weekday of a date, Sunday 1 (Sakamoto's method).
fn weekday(year: i64, month: i64, day: i64) -> i64 {
    const T: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    (y + y / 4 - y / 100 + y / 400 + T[(month - 1) as usize] + day).rem_euclid(7) + 1
}

/// A `CalendarTime` of a date at midnight.
fn calendar_time(vm: &mut Vm, year: i64, month: i64, day: i64) -> Value {
    let t = vm.new_table();
    for (k, v) in [
        ("year", year),
        ("month", month),
        ("monthDay", day),
        ("weekday", weekday(year, month, day)),
        ("hour", 0),
        ("minute", 0),
    ] {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, Value::Num(v as f64));
    }
    Value::Table(t)
}

/// `C_Calendar.GetMinDate()`: the calendar's first day, 23 November 2004
/// (unverified: the game's release; the client's answer not recorded).
fn get_min_date(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![calendar_time(vm, 2004, 11, 23)])
}

/// `C_Calendar.GetMaxCreateDate()`: a year from the clock's day
/// (unverified).
fn get_max_create_date(vm: &mut Vm, _c: NativeCall) -> Ret {
    let date = vm.get_global("date");
    let fmt = vm.str("*t");
    let Some(Value::Table(tm)) = vm.call(date, &[fmt])?.first().copied() else {
        return Ok(vec![calendar_time(vm, 2001, 1, 1)]);
    };
    let mut get = |k: &str| {
        let k = vm.str(k);
        match vm.heap.table(tm).get(k) {
            Value::Num(n) => n as i64,
            _ => 1,
        }
    };
    let (year, month, day) = (get("year"), get("month"), get("day"));
    // 29 February a year on is the 28th.
    let day = if month == 2 && day == 29 { 28 } else { day };
    Ok(vec![calendar_time(vm, year + 1, month, day)])
}

/// `C_LFGInfo.CanPlayerUseGroupFinder()`: usable, no reason (Forever has
/// the group finder: `CanPlayerUseLFD()` and `CanPlayerUsePremadeGroup()`
/// recorded `true, ""`; this one unverified).
fn can_player_use_group_finder(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true), vm.str("")])
}

/// `C_LFGInfo.IsLFDEnabled()`, `C_LFGList.IsPremadeGroupFinderEnabled()`:
/// on, as the recorded `CanPlayerUseLFD` and `CanPlayerUsePremadeGroup`
/// (unverified).
fn finder_enabled(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    let own: Vec<(&'static str, NativeFn)> = vec![
        // The group finder.
        (
            "C_LFGInfo.CanPlayerUseGroupFinder",
            can_player_use_group_finder,
        ),
        ("C_LFGInfo.IsLFDEnabled", finder_enabled),
        ("C_LFGList.IsPremadeGroupFinderEnabled", finder_enabled),
        // Difficulties and instances.
        ("GetDifficultyInfo", get_difficulty_info),
        ("GetDungeonDifficultyID", get_dungeon_difficulty_id),
        ("SetDungeonDifficultyID", set_dungeon_difficulty_id),
        ("SetRaidDifficultyID", set_raid_difficulty_id),
        ("GetLegacyRaidDifficultyID", get_legacy_raid_difficulty_id),
        (
            "C_RaidLocks.GetRedirectedDifficultyID",
            get_redirected_difficulty_id,
        ),
        (
            "C_EncounterJournal.GetBaseDifficultyID",
            get_base_difficulty_id,
        ),
        ("CanShowResetInstances", can_show_reset_instances),
        // No instance saved (`GetNumSavedInstances` 0): resetting does
        // nothing, an index names none.
        ("ResetInstances", nothing),
        ("GetSavedInstanceInfo", nothing),
        ("GetSavedInstanceEncounterInfo", nothing),
        ("GetSavedInstanceChatLink", nil),
        ("GetInstanceLockTimeRemaining", documented),
        ("GetInstanceLockTimeRemainingEncounter", documented),
        // The party.
        ("C_PartyInfo.IsGUIDInGroup", is_guid_in_group),
        ("C_PartyInfo.CanInvite", can_invite),
        ("C_PartyInfo.GetMinLevel", get_min_level),
        (
            "C_PartyInfo.GetAvailableLootMethods",
            get_available_loot_methods,
        ),
        (
            "C_PartyInfo.IsLootMethodAvailable",
            is_loot_method_available,
        ),
        ("C_PartyInfo.GetLootMethodStyle", documented),
        ("C_PartyInfo.IsCrossFactionParty", no),
        ("C_PartyInfo.IsPartyWalkIn", no),
        ("C_PartyInfo.DoCountdown", do_countdown),
        // Loot: no roll under way (`GetActiveLootRollIDs` none), so a roll
        // ID names none.
        ("GetLootThreshold", get_loot_threshold),
        ("SetLootThreshold", set_loot_threshold),
        ("GetLootInfo", get_loot_info),
        ("GetLootRollItemInfo", nothing),
        ("GetLootRollItemLink", nil),
        ("GetLootRollTimeLeft", zero),
        ("RollOnLoot", nothing),
        ("ConfirmLootSlot", nothing),
        ("GetMasterLootCandidate", nil),
        // The dungeon finder: no queue, proposal, reward or lock.
        ("GetLFGRoles", get_lfg_roles),
        ("SetLFGRoles", set_lfg_roles),
        ("GetPVPRoles", get_pvp_roles),
        ("GetLFGRoleShortageRewards", get_lfg_role_shortage_rewards),
        ("GetLFGDungeonRewards", get_lfg_dungeon_rewards),
        ("GetLFGBootProposal", get_lfg_boot_proposal),
        ("GetLFGQueuedList", get_lfg_queued_list),
        ("HasLFGRestrictions", no),
        ("IsLFGDungeonJoinable", is_lfg_dungeon_joinable),
        ("IsInLFGDungeon", no),
        ("IsLFGComplete", no),
        ("IsInScenarioGroup", no),
        ("GetNumRandomDungeons", zero),
        ("GetLFGDungeonNumEncounters", lfg_dungeon_num_encounters),
        ("GetLFGDungeonInfo", nothing),
        ("GetLFGRandomDungeonInfo", nothing),
        ("GetLFGCompletionReward", nothing),
        ("GetLFGCompletionRewardItem", nothing),
        ("GetLFGDungeonRewardInfo", nothing),
        ("GetLFGDungeonShortageRewardInfo", nothing),
        ("GetLFGDungeonRewardCapInfo", nothing),
        ("GetLFGDungeonRewardCapBarInfo", nothing),
        ("GetLFGDungeonEncounterInfo", nothing),
        ("GetLFGProposalMember", nothing),
        ("GetLFGProposalEncounter", nothing),
        ("GetLFGRoleUpdateBattlegroundInfo", nothing),
        ("GetLFGReadyCheckUpdateBattlegroundInfo", nothing),
        ("GetLFGRoleUpdateSlot", nothing),
        ("GetLFGRoleUpdateMember", nothing),
        ("GetLFGSuspendedPlayers", nothing),
        ("GetLFGInviteRoleAvailability", nothing),
        ("GetLFGInviteRoleRestrictions", nothing),
        ("GetLFGQueueStats", nothing),
        ("GetLFGCategoryForID", nil),
        ("GetLFGCompletionRewardItemLink", nil),
        ("GetLFGDungeonRewardLink", nil),
        ("GetLFGDungeonShortageRewardLink", nil),
        ("GetLFGDeserterExpiration", nil),
        ("GetLFGRandomCooldownExpiration", nil),
        ("GetRandomDungeonBestChoice", nil),
        ("LFGTeleport", nothing),
        ("SetLFGDungeon", nothing),
        ("SetLFGBootVote", nothing),
        ("SetLFGDungeonEnabled", nothing),
        ("SetLFGHeaderCollapsed", nothing),
        ("SetLFGComment", nothing),
        // The group finder's listings (undocumented ones): no listing,
        // application, applicant or search result.
        ("C_LFGList.GetApplicationInfo", get_application_info),
        ("C_LFGList.GetNumApplications", get_num_applications),
        ("C_LFGList.GetNumApplicants", zero),
        ("C_LFGList.GetNumInvitedApplicantMembers", zero),
        ("C_LFGList.GetNumPendingApplicantMembers", zero),
        ("C_LFGList.GetApplicants", empty_list),
        ("C_LFGList.HasActivityList", no),
        ("C_LFGList.IsCurrentlyApplying", no),
        ("C_LFGList.GetActivityInfoExpensive", no),
        ("C_LFGList.GetRoleCheckInfo", no),
        ("C_LFGList.GetActivityIDForQuestID", nil),
        ("C_LFGList.GetApplicantMemberStats", nil),
        ("C_LFGList.GetSearchResultEncounterInfo", nil),
        ("C_LFGList.GetApplicantMemberInfo", nothing),
        ("C_LFGList.GetSearchResultFriends", nothing),
        ("C_LFGList.AcceptInvite", nothing),
        ("C_LFGList.ApplyToGroup", nothing),
        ("C_LFGList.CancelApplication", nothing),
        ("C_LFGList.DeclineApplicant", nothing),
        ("C_LFGList.DeclineInvite", nothing),
        ("C_LFGList.InviteApplicant", nothing),
        ("C_LFGList.RefreshApplicants", nothing),
        ("C_LFGList.RemoveApplicant", nothing),
        ("C_LFGList.RequestAvailableActivities", nothing),
        ("C_LFGList.SetApplicantMemberRole", nothing),
        // Battlegrounds and arenas: not in one, no queue
        // (`GetMaxBattlefieldID` 0).
        ("GetBattlefieldStatus", status_none),
        ("GetWorldPVPQueueStatus", get_world_pvp_queue_status),
        ("GetPersonalRatedInfo", get_personal_rated_info),
        ("GetPVPTimer", get_pvp_timer),
        ("GetPVPDesired", no),
        ("GetBattlefieldArenaFaction", zero),
        ("GetBattlefieldWinner", nil),
        ("GetNumBattlefieldScores", zero),
        ("GetNumBattlefieldVehicles", zero),
        ("GetNumArenaOpponents", zero),
        ("GetBattlefieldPortExpiration", zero),
        ("GetBattlefieldInstanceExpiration", zero),
        ("GetBattlefieldInstanceRunTime", zero),
        ("GetBattlefieldEstimatedWaitTime", zero),
        ("GetBattlefieldTimeWaited", zero),
        ("GetBattlefieldTeamInfo", battlefield_team_info),
        ("GetRatedBattleGroundInfo", nothing),
        ("AcceptBattlefieldPort", nothing),
        ("LeaveBattlefield", nothing),
        // Scenarios (undocumented ones): none under way.
        ("C_Scenario.GetBonusStepRewardQuestID", nil),
        ("C_Scenario.GetBonusSteps", empty_list),
        ("C_Scenario.GetSupersededObjectives", empty_list),
        ("C_Scenario.GetProvingGroundsInfo", zeros4),
        ("C_Scenario.TreatScenarioAsDungeon", no),
        // Challenge modes' colours.
        ("C_ChallengeMode.GetDungeonScoreRarityColor", rarity_color),
        ("C_ChallengeMode.GetKeystoneLevelRarityColor", rarity_color),
        (
            "C_ChallengeMode.GetSpecificDungeonOverallScoreRarityColor",
            rarity_color,
        ),
        (
            "C_ChallengeMode.GetSpecificDungeonScoreRarityColor",
            rarity_color,
        ),
        // The calendar's range.
        ("C_Calendar.GetMinDate", get_min_date),
        ("C_Calendar.GetMaxCreateDate", get_max_create_date),
    ];
    own.into_iter()
        .chain(NOTHING_HERE.iter().copied())
        .collect()
}

/// Documented functions of systems the world has none of, answering
/// their documented "nothing here" (a `MayReturnNothing` function or an
/// action: nothing; a flag false; a count, ID or score 0; a nilable
/// result nil; a list empty; a text empty; a structure or several
/// results their documented empty values). The same answers Blizzard's
/// code was given so far.
const NOTHING_HERE: &[(&str, NativeFn)] = &[
    // Flags.
    ("C_ChallengeMode.IsChallengeModeActive", no),
    ("C_PartyInfo.IsDelveInProgress", no),
    ("C_WeeklyRewards.HasAvailableRewards", no),
    ("C_Calendar.EventCanEdit", no),
    ("C_LFGInfo.IsInLFGFollowerDungeon", no),
    ("C_PvP.IsSoloShuffle", no),
    ("C_WeeklyRewards.CanClaimRewards", no),
    ("C_PvP.IsRatedArena", no),
    ("C_PvP.IsRatedBattleground", no),
    ("C_PvP.IsRatedSoloShuffle", no),
    ("GetPVPGearStatRules", no),
    ("C_LFGList.IsCensoredActiveEntryUnresolved", no),
    ("C_PvP.IsRatedMap", no),
    ("C_Calendar.CanSendInvite", no),
    ("C_Calendar.OpenEvent", no),
    ("C_DelvesUI.HasActiveLair", no),
    ("C_LFGList.IsPlayerValidForEndgameFieldEdits", no),
    ("C_LFGList.ListingUsesEndgameEditRestrictions", no),
    ("C_PartyInfo.ChallengeModeRestrictionsActive", no),
    ("C_PartyInfo.SetEveryoneIsAssistant", no),
    ("C_PvP.CanToggleWarMode", no),
    ("C_PvP.IsMatchFactional", no),
    ("C_PvP.IsRatedSoloRBG", no),
    ("C_Calendar.AreNamesReady", no),
    ("C_DelvesUI.HasActiveDelve", no),
    ("C_DelvesUI.HasActiveLFGLair", no),
    ("C_EncounterJournal.InstanceHasLoot", no),
    ("C_EncounterJournal.IsEncounterComplete", no),
    ("C_InstanceEncounter.IsEncounterLimitingResurrections", no),
    ("C_InstanceEncounter.IsEncounterSuppressingRelease", no),
    ("C_LFGInfo.IsLFGFollowerDungeon", no),
    ("C_PartyInfo.IsPartyInJailersTower", no),
    ("C_PartyInfo.SetLootMethod", no),
    ("C_PartyPose.HasExtraAction", no),
    ("C_PvP.IsBrawlSoloRBG", no),
    ("C_PvP.IsBrawlSoloShuffle", no),
    ("C_PvP.IsInRatedMatchWithDeserterPenalty", no),
    ("C_PvP.IsMatchConsideredArena", no),
    ("C_PvP.IsWarModeActive", no),
    ("C_RaidLocks.IsRaidLockExtendFeatureSupported", no),
    ("C_SummonInfo.IsSummonSkippingStartExperience", no),
    ("C_WeeklyRewards.HasInteraction", no),
    ("C_WeeklyRewards.ShouldShowFinalRetirementMessage", no),
    ("C_Calendar.CanAddEvent", no),
    ("C_Calendar.ContextMenuEventCanComplain", no),
    ("C_Calendar.ContextMenuEventCanEdit", no),
    ("C_Calendar.ContextMenuEventCanRemove", no),
    ("C_Calendar.ContextMenuEventClipboard", no),
    ("C_Calendar.EventHasPendingInvite", no),
    ("C_Calendar.EventHaveSettingsChanged", no),
    ("C_Calendar.IsActionPending", no),
    ("C_ChallengeMode.CanUseKeystoneInCurrentMap", no),
    ("C_ChallengeMode.HasSlottedKeystone", no),
    ("C_ChallengeMode.IsChallengeModeResettable", no),
    ("C_ChallengeMode.RemoveKeystone", no),
    ("C_ChallengeMode.StartChallengeMode", no),
    ("C_DeathRecap.HasRecapEvents", no),
    ("C_DelvesUI.IsInLair", no),
    ("C_DelvesUI.IsTraitTreeForCompanion", no),
    ("C_EncounterJournal.InstanceHasDifficultyID", no),
    ("C_InstanceEncounter.ShouldShowTimelineForEncounter", no),
    ("C_LFGInfo.AreCrossFactionGroupQueuesAllowed", no),
    ("C_LFGInfo.DoesActivePartyMeetPremadeLaunchCount", no),
    ("C_LFGInfo.DoesCrossFactionQueueRequireFullPremade", no),
    ("C_LFGInfo.HideNameFromUI", no),
    ("C_LFGInfo.IsInMatchmadeRaidWithoutRoleRequirements", no),
    ("C_LFGInfo.IsLFREnabled", no),
    ("C_LFGList.CanActiveEntryUseAutoAccept", no),
    ("C_LFGList.CanCreateScenarioGroup", no),
    ("C_LFGList.CreateScenarioListing", no),
    ("C_LFGList.DoesCensoredTextMatch", no),
    ("C_LFGList.DoesEntryTitleMatchPrebuiltTitle", no),
    ("C_LFGList.ValidateRequiredDungeonScore", no),
    ("C_LFGList.ValidateRequiredPvpRatingForActivity", no),
    ("C_Loot.IsLegacyLootModeEnabled", no),
    ("C_PartyInfo.CanStartInstanceAbandonVote", no),
    ("C_PartyInfo.IsChallengeModeKeystoneOwner", no),
    ("C_PvP.ArePvpTalentsUnlocked", no),
    ("C_PvP.AreTrainingGroundsEnabled", no),
    ("C_PvP.CanDisplayDeaths", no),
    ("C_PvP.CanDisplayHonorableKills", no),
    ("C_PvP.CanSurrenderArena", no),
    ("C_PvP.CanToggleWarModeInArea", no),
    ("C_PvP.DoesMatchOutcomeAffectRating", no),
    ("C_PvP.IsSoloRBG", no),
    ("C_PvP.IsTrainingGroundsArena", no),
    ("C_PvP.IsTrainingGroundsBG", no),
    ("C_RaidLocks.IsEncounterComplete", no),
    ("C_WeeklyRewards.AreRewardsForCurrentRewardPeriod", no),
    ("C_WeeklyRewards.ShouldShowRetirementMessage", no),
    ("C_Calendar.IsEventOpen", no),
    ("C_DelvesUI.IsEligibleForActiveDelveRewards", no),
    ("C_PartyInfo.IsChallengeModeActive", no),
    ("C_PartyInfo.IsDelveComplete", no),
    ("C_PvP.HasArenaSkirmishWinToday", no),
    ("C_PvP.HasMatchStarted", no),
    ("C_PvP.HasRandomTrainingGroundArenaWinToday", no),
    ("C_PvP.HasRandomTrainingGroundBGWinToday", no),
    ("C_PvP.IsActiveMatchRegistered", no),
    ("C_PvP.IsArena", no),
    ("C_PvP.IsBattleground", no),
    ("C_PvP.IsSubZonePVPPOI", no),
    ("C_PvP.IsWarModeFeatureEnabled", no),
    ("C_RaidLocks.IsRaidLockExtendFeatureEnabled", no),
    ("C_WeeklyRewards.HasGeneratedRewards", no),
    ("C_WeeklyRewards.IsWeeklyChestRetired", no),
    // Nothing: an unknown ID's information, or an action without effect
    // here (a protected one's protection is the protection layer's).
    ("C_ChallengeMode.GetMapUIInfo", nothing),
    ("C_ChallengeMode.GetAffixInfo", nothing),
    ("C_PartyInfo.UninviteUnit", nothing),
    ("C_EncounterJournal.GetSectionInfo", nothing),
    ("C_MythicPlus.GetOwnedKeystoneLevel", nothing),
    ("C_PartyInfo.LeaveParty", nothing),
    ("C_ScenarioInfo.GetCriteriaInfo", nothing),
    ("C_ChallengeMode.GetDeathCount", nothing),
    ("C_Calendar.EventGetInvite", nothing),
    ("C_MythicPlus.GetOwnedKeystoneChallengeMapID", nothing),
    ("C_PvP.RequestCrowdControlSpell", nothing),
    ("C_ChallengeMode.SlotKeystone", nothing),
    ("C_Calendar.OpenCalendar", nothing),
    ("C_LFGList.GetApplicantInfo", nothing),
    ("C_LFGList.SaveAdvancedFilter", nothing),
    ("C_WeeklyRewards.GetItemHyperlink", nothing),
    ("C_ChallengeMode.GetSlottedKeystoneInfo", nothing),
    ("C_WeeklyRewards.GetExampleRewardItemHyperlinks", nothing),
    ("C_Calendar.GetEventIndex", nothing),
    ("C_EncounterJournal.GetLootInfoByIndex", nothing),
    ("C_PartyInfo.GetInviteReferralInfo", nothing),
    ("C_PartyInfo.PromoteToLeader", nothing),
    ("C_PvP.JoinBattlefield", nothing),
    ("C_ScenarioInfo.GetDisplayInfo", nothing),
    ("C_ScenarioInfo.GetUnitCriteriaProgressValues", nothing),
    ("C_Calendar.EventSetClubId", nothing),
    ("C_Calendar.EventSetTitle", nothing),
    ("C_Calendar.EventSetType", nothing),
    ("C_Calendar.GetEventInfo", nothing),
    ("C_Calendar.SetNextClubId", nothing),
    ("C_EncounterJournal.InitalizeSelectedTier", nothing),
    ("C_MythicPlus.RequestMapInfo", nothing),
    ("C_PartyInfo.ConfirmConvertToRaid", nothing),
    ("C_PartyInfo.ConfirmInviteUnit", nothing),
    ("C_PartyInfo.DoReadyCheck", nothing),
    ("C_ScenarioInfo.GetCriteriaInfoByStep", nothing),
    ("C_SummonInfo.CancelSummon", nothing),
    ("C_SummonInfo.ConfirmSummon", nothing),
    ("C_WeeklyRewards.GetActivityEncounterInfo", nothing),
    ("C_Calendar.CloseEvent", nothing),
    ("C_Calendar.EventInvite", nothing),
    ("C_Calendar.EventSetDescription", nothing),
    ("C_Calendar.EventSetInviteStatus", nothing),
    ("C_Calendar.SetAbsMonth", nothing),
    ("C_Calendar.SetMonth", nothing),
    ("C_ChallengeMode.Reset", nothing),
    ("C_DelvesUI.GetPlayerCompanionPDEID", nothing),
    ("C_EncounterJournal.SetSlotFilter", nothing),
    ("C_LFGList.ClearSearchTextFields", nothing),
    ("C_LFGList.GetApplicantPvpRatingInfoForListing", nothing),
    (
        "C_LFGList.GetOwnedKeystoneActivityAndGroupAndLevel",
        nothing,
    ),
    ("C_LFGList.SetSearchToActivity", nothing),
    ("C_MythicPlus.GetCurrentAffixes", nothing),
    ("C_MythicPlus.GetSeasonBestAffixScoreInfoForMap", nothing),
    ("C_PartyInfo.ConfirmInviteTravelPass", nothing),
    ("C_PartyInfo.ConfirmLeaveParty", nothing),
    ("C_PartyInfo.ConfirmRequestInviteFromUnit", nothing),
    ("C_PartyInfo.DemoteAssistant", nothing),
    ("C_PartyInfo.GetActiveCategories", nothing),
    ("C_PartyInfo.GetInstanceAbandonVoteCooldownTime", nothing),
    ("C_PartyInfo.PromoteToAssistant", nothing),
    ("C_PartyInfo.RequestInviteFromUnit", nothing),
    ("C_PartyInfo.SetRestrictPings", nothing),
    ("C_PartyInfo.StartInstanceAbandonVote", nothing),
    ("C_PvP.GetArenaRewards", nothing),
    ("C_PvP.GetBattlefieldVehicleInfo", nothing),
    ("C_PvP.GetBrawlRewards", nothing),
    ("C_PvP.JoinBrawl", nothing),
    ("C_PvP.SetPVP", nothing),
    ("C_Calendar.AddEvent", nothing),
    ("C_Calendar.ContextMenuEventPaste", nothing),
    ("C_Calendar.ContextMenuEventRemove", nothing),
    ("C_Calendar.ContextMenuEventSignUp", nothing),
    ("C_Calendar.ContextMenuGetEventIndex", nothing),
    ("C_Calendar.ContextMenuInviteAvailable", nothing),
    ("C_Calendar.ContextMenuInviteDecline", nothing),
    ("C_Calendar.ContextMenuInviteRemove", nothing),
    ("C_Calendar.ContextMenuInviteTentative", nothing),
    ("C_Calendar.ContextMenuSelectEvent", nothing),
    ("C_Calendar.CreateCommunitySignUpEvent", nothing),
    ("C_Calendar.CreateGuildAnnouncementEvent", nothing),
    ("C_Calendar.CreateGuildSignUpEvent", nothing),
    ("C_Calendar.CreatePlayerEvent", nothing),
    ("C_Calendar.EventAvailable", nothing),
    ("C_Calendar.EventClearAutoApprove", nothing),
    ("C_Calendar.EventClearLocked", nothing),
    ("C_Calendar.EventDecline", nothing),
    ("C_Calendar.EventGetInviteResponseTime", nothing),
    ("C_Calendar.EventSelectInvite", nothing),
    ("C_Calendar.EventSetAutoApprove", nothing),
    ("C_Calendar.EventSetDate", nothing),
    ("C_Calendar.EventSetLocked", nothing),
    ("C_Calendar.EventSetTextureID", nothing),
    ("C_Calendar.EventSetTime", nothing),
    ("C_Calendar.EventSignUp", nothing),
    ("C_Calendar.EventSortInvites", nothing),
    ("C_Calendar.EventTentative", nothing),
    ("C_Calendar.GetGuildEventInfo", nothing),
    ("C_Calendar.GetGuildEventSelectionInfo", nothing),
    ("C_Calendar.GetHolidayInfo", nothing),
    ("C_Calendar.GetRaidInfo", nothing),
    ("C_Calendar.MassInviteCommunity", nothing),
    ("C_Calendar.MassInviteGuild", nothing),
    ("C_Calendar.RemoveEvent", nothing),
    ("C_Calendar.UpdateEvent", nothing),
    ("C_ChallengeMode.ClearKeystone", nothing),
    ("C_ChallengeMode.CloseKeystoneFrame", nothing),
    ("C_ChallengeMode.GetMapScoreInfo", nothing),
    ("C_ChallengeMode.GetPowerLevelDamageHealthMod", nothing),
    ("C_ChallengeMode.RequestLeaders", nothing),
    ("C_DelvesUI.RequestPartyEligibilityForDelveTiers", nothing),
    ("C_DelvesUI.SaveSeenCuriosBySlotType", nothing),
    ("C_DelvesUI.SelectDelveEntranceTier", nothing),
    ("C_EncounterJournal.OnClose", nothing),
    ("C_EncounterJournal.OnOpen", nothing),
    ("C_EncounterJournal.ResetSlotFilter", nothing),
    ("C_EncounterJournal.SetTab", nothing),
    ("C_EncounterJournal.StartArathiRPE", nothing),
    (
        "C_EventToastManager.GetLevelUpDisplayToastsFromLevel",
        nothing,
    ),
    ("C_EventToastManager.GetNextToastToDisplay", nothing),
    ("C_EventToastManager.RemoveCurrentToast", nothing),
    ("C_LFGList.ClearApplicationTextFields", nothing),
    ("C_LFGList.ConfirmCensoredActiveEntry", nothing),
    ("C_LFGList.GetApplicantBestDungeonScore", nothing),
    ("C_LFGList.GetApplicantDungeonScoreForListing", nothing),
    ("C_LFGList.GetKeystoneForActivity", nothing),
    ("C_LFGList.ReportGroupAsAdvertisement", nothing),
    ("C_LFGList.RevealCensoredActiveEntry", nothing),
    ("C_LFGList.RevealCensoredSearchResult", nothing),
    ("C_LFGList.SetEntryTitle", nothing),
    ("C_LFGList.SetSearchToQuestID", nothing),
    ("C_LFGList.SetSearchToScenarioID", nothing),
    ("C_LootFrame.TryAutoLoot", nothing),
    ("C_LootJournal.GetItemSetItems", nothing),
    ("C_LootJournal.GetItemSets", nothing),
    (
        "C_MythicPlus.GetSeasonBestMythicRatingFromThisExpansion",
        nothing,
    ),
    ("C_MythicPlus.GetWeeklyBestForMap", nothing),
    ("C_MythicPlus.RequestCurrentAffixes", nothing),
    ("C_PartyInfo.ConfirmReadyCheck", nothing),
    ("C_PartyInfo.DelveTeleportOut", nothing),
    ("C_PartyInfo.GetInviteConfirmationInvalidQueues", nothing),
    ("C_PartyInfo.SetInstanceAbandonVoteResponse", nothing),
    ("C_PartyPose.ExtraAction", nothing),
    ("C_PartyPose.GetPartyPoseInfoByID", nothing),
    ("C_PartyPose.GetPartyPoseInfoByMapID", nothing),
    ("C_PvP.GetPVPSeasonRewardAchievementID", nothing),
    ("C_PvP.GetSkirmishInfo", nothing),
    ("C_PvP.JoinRandomTrainingGroundArena", nothing),
    ("C_PvP.JoinRandomTrainingGroundBG", nothing),
    ("C_PvP.JoinRatedBGBlitz", nothing),
    ("C_PvP.JoinTrainingGround", nothing),
    ("C_PvP.TogglePVP", nothing),
    ("C_PvP.ToggleWarMode", nothing),
    ("C_ScenarioInfo.GetScenarioIconInfo", nothing),
    ("C_ScenarioInfo.GetScenarioInfo", nothing),
    ("C_WeeklyRewards.ClaimReward", nothing),
    ("C_WeeklyRewards.CloseInteraction", nothing),
    ("C_WeeklyRewards.GetDifficultyIDForActivityTier", nothing),
    ("C_WeeklyRewards.OnUIInteract", nothing),
    ("C_Calendar.ContextMenuEventCopy", nothing),
    ("C_Calendar.EventClearModerator", nothing),
    ("C_Calendar.EventRemoveInvite", nothing),
    ("C_Calendar.EventRemoveInviteByGuid", nothing),
    ("C_Calendar.EventSetModerator", nothing),
    ("C_ChallengeMode.GetGuildLeaders", nothing),
    ("C_EncounterJournal.SetPreviewMythicPlusLevel", nothing),
    ("C_EncounterJournal.SetPreviewPvpTier", nothing),
    ("C_LFGInfo.ConfirmLfgExpandSearch", nothing),
    ("C_LFGInfo.GetDungeonInfo", nothing),
    ("C_MythicPlus.GetLastWeeklyBestInformation", nothing),
    ("C_MythicPlus.GetOwnedKeystoneMapID", nothing),
    ("C_MythicPlus.RequestRewards", nothing),
    ("C_PvP.GetGlobalPvpScalingInfoForSpecID", nothing),
    ("C_PvP.SetWarModeDesired", nothing),
    ("C_ScenarioInfo.GetScenarioStepInfo", nothing),
    // Structures and several results: their documented empty values.
    ("C_ChallengeMode.GetChallengeCompletionInfo", documented),
    ("C_ChallengeMode.GetActiveKeystoneInfo", documented),
    ("C_LFGList.GetAdvancedFilter", documented),
    ("C_PartyInfo.GetRestrictPings", documented),
    (
        "C_DelvesUI.GetCurioRarityByTraitCondAccountElementID",
        documented,
    ),
    (
        "C_DelvesUI.GetWorldTierDifficultyForActivePlayer",
        documented,
    ),
    ("C_Calendar.EventGetInviteSortCriterion", documented),
    ("C_Calendar.GetDefaultGuildFilter", documented),
    ("C_EncounterJournal.GetSlotFilter", documented),
    ("C_MythicPlus.GetSeasonBestForMap", documented),
    ("C_PartyInfo.GetMinItemLevel", documented),
    ("C_PvP.CanPlayerUseRatedPVPUI", documented),
    ("C_PvP.CanPlayerUseTrainingGroundsUI", documented),
    ("C_PvP.GetRandomBGInfo", documented),
    ("C_PvP.GetRandomBGRewards", documented),
    ("C_PvP.GetRatedBGRewards", documented),
    ("C_PvP.GetRewardItemLevelsByTierEnum", documented),
    ("C_WeeklyRewards.GetNextActivitiesIncrease", documented),
    ("C_WeeklyRewards.GetNumCompletedDungeonRuns", documented),
    ("C_DelvesUI.GetActiveDelveTier", documented),
    ("C_DelvesUI.GetTieredEntranceType", documented),
    ("C_DelvesUI.IsDelveEntranceTierEnabled", documented),
    ("C_EncounterJournal.GetLootInfo", documented),
    ("C_LFGInfo.CanPlayerUseLFR", documented),
    ("C_LFGInfo.CanPlayerUseScenarioFinder", documented),
    ("C_LFGInfo.GetRoleCheckDifficultyDetails", documented),
    ("C_LFGList.GetGroupLeaverCountsByRole", documented),
    ("C_MythicPlus.GetWeeklyChestRewardLevel", documented),
    ("C_PartyInfo.GetInstanceAbandonShutdownTime", documented),
    ("C_PartyInfo.GetInstanceAbandonVoteRequirements", documented),
    ("C_PvP.GetArenaSkirmishRewards", documented),
    ("C_PvP.GetBattlefieldFlagPosition", documented),
    ("C_PvP.GetRandomEpicBGInfo", documented),
    ("C_PvP.GetRandomEpicBGRewards", documented),
    ("C_PvP.GetRatedSoloRBGRewards", documented),
    ("C_PvP.GetRatedSoloShuffleRewards", documented),
    ("C_PvP.GetSeasonBestInfo", documented),
    ("C_PvP.IsBattlegroundEnlistmentBonusActive", documented),
    ("C_WeeklyRewards.GetConquestWeeklyProgress", documented),
    ("GetPVPLifetimeStats", documented),
    ("GetPVPSessionStats", documented),
    ("GetPVPYesterdayStats", documented),
    ("C_LFGInfo.CanPlayerUsePVP", documented),
    ("C_MythicPlus.GetCurrentSeasonValues", documented),
    ("C_MythicPlus.GetRewardLevelForDifficultyLevel", documented),
    ("C_PvP.GetArenaCrowdControlDuration", documented),
    ("C_PvP.GetRandomTrainingGroundArenaRewards", documented),
    ("C_PvP.GetRandomTrainingGroundBGRewards", documented),
    ("C_PvP.GetWeeklyChestInfo", documented),
    ("C_WeeklyRewards.GetNextMythicPlusIncrease", documented),
    // Lists: empty.
    ("C_WeeklyRewards.GetActivities", empty_list),
    ("C_ChallengeMode.GetMapTable", empty_list),
    ("C_MythicPlus.GetRunHistory", empty_list),
    ("C_Calendar.GetClubCalendarEvents", empty_list),
    ("C_LootHistory.GetAllEncounterInfos", empty_list),
    ("C_Calendar.EventGetStatusOptions", empty_list),
    ("C_DeathRecap.GetRecapEvents", empty_list),
    ("C_DelvesUI.GetUnseenCuriosBySlotType", empty_list),
    ("C_EncounterJournal.GetEncountersOnMap", empty_list),
    ("C_Calendar.EventGetTextures", empty_list),
    ("C_Calendar.EventGetTypesDisplayOrdered", empty_list),
    ("C_DelvesUI.GetDelveEntranceTiers", empty_list),
    ("C_LFGInfo.GetAllEntriesForCategory", empty_list),
    ("C_LFGInfo.GetLFDLockStates", empty_list),
    ("C_PvP.GetMatchPVPStatColumns", empty_list),
    ("C_PvP.GetPostMatchCurrencyRewards", empty_list),
    ("C_PvP.GetPostMatchItemRewards", empty_list),
    ("C_PvP.GetTrainingGrounds", empty_list),
    ("C_WeeklyRewards.GetSortedProgressForActivity", empty_list),
    ("C_Calendar.EventGetTypes", empty_list),
    ("C_DelvesUI.GetDelvesAffixSpellsForSeason", empty_list),
    ("C_LFGInfo.GetLevelUpInstances", empty_list),
    ("C_PvP.GetLevelUpBattlegrounds", empty_list),
    // Counts, IDs, scores: 0.
    ("C_ChallengeMode.GetOverallDungeonScore", zero),
    ("C_DelvesUI.GetFactionForCompanion", zero),
    ("C_PvP.GetWarModeRewardBonus", zero),
    ("C_Calendar.GetNumInvites", zero),
    ("C_DelvesUI.GetDelvesFactionForSeason", zero),
    ("C_DelvesUI.GetTieredEntrancePDEID", zero),
    ("C_PvP.GetCustomVictoryStatID", zero),
    ("C_DelvesUI.GetCreatureDisplayInfoForCompanion", zero),
    ("C_DelvesUI.GetRoleSubtreeForCompanion", zero),
    ("C_DelvesUI.GetDelveEntranceMapID", zero),
    ("C_MythicPlus.GetCurrentSeason", zero),
    ("C_PvP.GetPvpTalentsUnlockedLevel", zero),
    ("C_PvP.GetUIDisplaySeason", zero),
    ("C_SeasonInfo.GetCurrentDisplaySeasonID", zero),
    ("C_SummonInfo.GetSummonReason", zero),
    ("C_Calendar.GetNumGuildEvents", zero),
    ("C_DeathRecap.GetRecapMaxHealth", zero),
    ("C_DelvesUI.GetCurrentDelvesSeasonNumber", zero),
    ("C_DelvesUI.GetDelveEntranceBackgroundWidgetSetID", zero),
    ("C_LootHistory.GetLootHistoryTime", zero),
    ("C_PartyInfo.GetNumInstanceAbandonGroupVoteResponses", zero),
    ("C_PvP.GetActiveMatchWinner", zero),
    ("C_PvP.GetRatedSoloRBGMinItemLevel", zero),
    ("C_PvP.GetRatedSoloShuffleMinItemLevel", zero),
    ("C_PvP.GetWarModeRewardBonusDefault", zero),
    ("C_PvP.StartSoloRBGWarGameByName", zero),
    ("C_PvP.StartSpectatorSoloRBGWarGame", zero),
    ("C_SeasonInfo.GetTimeUntilCurrentPVPSeasonEnd", zero),
    ("C_ChallengeMode.GetStartTime", zero),
    ("C_DelvesUI.GetCompanionInfoForActivePlayer", zero),
    ("C_DelvesUI.GetModelSceneForCompanion", zero),
    ("C_PvP.GetActiveMatchBracket", zero),
    ("C_PvP.GetActiveMatchDuration", zero),
    ("C_PvP.GetBrawlSoloRBGMinItemLevel", zero),
    // Nilable results: nil.
    ("C_ChallengeMode.GetActiveChallengeMapID", nil),
    ("C_PvP.GetPvpTierInfo", nil),
    ("C_PvP.GetTeamInfo", nil),
    ("C_PvP.GetHonorRewardInfo", nil),
    ("C_PvP.GetAvailableBrawlInfo", nil),
    ("C_SeasonInfo.GetCurrentDisplaySeasonExpansion", nil),
    ("C_PvP.GetActiveBrawlInfo", nil),
    ("C_PvP.GetSpecialEventBrawlInfo", nil),
    ("C_PvP.GetNextHonorLevelForReward", nil),
    ("C_PvP.GetScoreInfo", nil),
    ("C_PvP.GetScoreInfoByPlayerGuid", nil),
    ("C_Calendar.EventGetCalendarType", nil),
    ("C_Calendar.EventGetClubId", nil),
    ("C_DelvesUI.GetDelvesMinRequiredLevel", nil),
    ("C_EncounterJournal.GetInstanceForGameMap", nil),
    ("C_EncounterJournal.GetSectionIconFlags", nil),
    ("C_Loot.GetLootRollDuration", nil),
    ("C_LootHistory.GetSortedDropsForEncounter", nil),
    ("C_LootHistory.GetSortedInfoForDrop", nil),
    ("C_Calendar.ContextMenuEventGetCalendarType", nil),
    ("C_Calendar.EventGetSelectedInvite", nil),
    ("C_Calendar.GetEventIndexInfo", nil),
    ("C_Calendar.GetFirstPendingInvite", nil),
    ("C_Calendar.GetNextClubId", nil),
    ("C_DelvesUI.GetDelveEntranceDescriptionString", nil),
    ("C_DelvesUI.GetDelveEntranceHeaderString", nil),
    ("C_DelvesUI.GetDelveEntranceTitleString", nil),
    ("C_DelvesUI.GetTieredEntranceOptionalAffixTraitTreeID", nil),
    ("C_LootHistory.GetInfoForEncounter", nil),
    ("C_MythicPlus.GetCurrentUIDisplaySeason", nil),
    ("C_MythicPlus.GetRewardLevelFromKeystoneLevel", nil),
    ("C_PartyInfo.GetInstanceAbandonVoteResponse", nil),
    ("C_PvP.GetAssignedSpecForBattlefieldQueue", nil),
    ("C_PvP.GetBattlegroundInfo", nil),
    ("C_PvP.GetPVPActiveMatchPersonalRatedInfo", nil),
    ("C_PvP.GetPVPActiveRatedMatchDeserterPenalty", nil),
    ("C_PvP.GetPersonalRatedBGBlitzSpecStats", nil),
    ("C_PvP.GetPersonalRatedSoloShuffleSpecStats", nil),
    ("C_PvP.GetPvpTierID", nil),
    ("C_ScenarioInfo.GetTieredEntranceActiveSpells", nil),
    ("C_SummonInfo.GetSummonConfirmSummoner", nil),
    ("C_MythicPlus.GetEndOfRunGearSequenceLevel", nil),
    ("C_PvP.GetMatchPVPStatColumn", nil),
    ("C_ScenarioInfo.GetJailersTowerTypeString", nil),
    // Texts: empty.
    ("C_DelvesUI.GetCurioLink", empty_string),
    ("C_DeathRecap.GetRecapLink", empty_string),
    ("C_DelvesUI.GetFlavorNodeNameForCompanion", empty_string),
    ("C_DelvesUI.GetLockedTextForCompanion", empty_string),
    ("C_LFGInfo.GetActiveLFGDungeonName", empty_string),
    ("C_LFGList.GetPlaystyleString", empty_string),
    ("C_SummonInfo.GetSummonConfirmAreaName", empty_string),
    ("C_EncounterJournal.GetEncounterJournalLink", empty_string),
];
