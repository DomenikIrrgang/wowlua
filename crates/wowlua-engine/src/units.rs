//! Units, tokens, combat and addon restrictions (spec emulator §17.6,
//! Tooling §16.1.4, decision T201): the player and the units a test spawns,
//! the tokens that name them (`target`, `nameplate1`, `party2`,
//! `targettarget`), the unit functions that read them, and the events a
//! change fires.
//!
//! What results are secret follows each function's documentation and the
//! restrictions active, as `/stdtest restrictions` recorded them on WoW
//! Forever 1.60.1 (`tests/client/forever-camelot-restrictions.lua`).
//!
//! Unverified until probed: the player's defaults; GUID formats; the order
//! of the events a change fires (a unit event fires once per token the unit
//! has, in token order); `ADDON_RESTRICTION_STATE_CHANGED` with `Activating`
//! before the regen event, the restriction then active; which restrictions
//! besides a restricted map make identities secret.

use wowlua_vm::heap::Userdata;
use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, TableRef, Value, Vm};

use crate::widgets;
use crate::{engine, engine_mut};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnitId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    Hostile,
    Neutral,
    Friendly,
}

pub struct Unit {
    pub guid: String,
    pub name: String,
    pub realm: Option<String>,
    /// WoW Forever's characters' last name: `UnitName`'s second value there
    /// (`/stdtest engine`: the first name, then the last), not the realm.
    pub last_name: Option<String>,
    pub level: f64,
    /// The class's file name (`WARRIOR`), its name and ID.
    pub class: String,
    pub class_name: String,
    pub class_id: f64,
    pub race: String,
    /// A creature's faction group (`UnitFactionGroup`), when it has one.
    pub faction_group: Option<String>,
    /// 1 unknown, 2 male, 3 female (`UnitSex`).
    pub sex: f64,
    pub reaction: Reaction,
    /// A player character (not an NPC).
    pub player: bool,
    pub health: f64,
    pub health_max: f64,
    pub power: f64,
    pub power_max: f64,
    pub power_type: f64,
    pub dead: bool,
    /// Flagged for PvP (`UnitIsPVP`).
    pub pvp: bool,
    /// The player released as a ghost (`death.rs`).
    pub ghost: bool,
    pub in_combat: bool,
    /// Whom the unit targets (`unittarget`).
    pub target: Option<UnitId>,
    pub alive_in_world: bool,
    /// How far from the player, in yards (`None`: unknown).
    pub distance: Option<f64>,
    /// `UnitClassification` (`normal`, `elite`, `rare`, …).
    pub classification: String,
    /// The player's threat on this creature (`setThreat`).
    pub threat: Option<f64>,
    /// What its corpse holds (`kill({ loot })`): items and counts.
    pub loot: Vec<(i64, f64)>,
    /// A creature's type and family: their names and IDs
    /// (`UnitCreatureType`, `UnitCreatureFamily`; none for players).
    pub creature_type: Option<(String, f64)>,
    pub creature_family: Option<(String, f64)>,
}

/// The addon restriction types (`Enum.AddOnRestrictionType`).
pub const COMBAT: usize = 0;
pub const ENCOUNTER: usize = 1;
pub const CHALLENGE_MODE: usize = 2;
pub const PVP_MATCH: usize = 3;
pub const MAP: usize = 4;
pub const CHAT: usize = 5;

#[derive(Default)]
pub struct Units {
    /// The player's auto attack on (`PLAYER_ENTER_COMBAT`).
    pub auto_attack: bool,
    /// The player was on a creature's threat list this fight (its end
    /// tells `UNIT_THREAT_SITUATION_UPDATE`).
    pub threat_situation: bool,
    pub list: Vec<Unit>,
    pub target: Option<UnitId>,
    pub focus: Option<UnitId>,
    pub mouseover: Option<UnitId>,
    /// The soft targets: the enemy, the friend, the interact unit.
    pub soft: [Option<UnitId>; 3],
    /// The NPC the player talks to (`npc`: a vendor).
    pub npc: Option<UnitId>,
    pub nameplates: Vec<Option<UnitId>>,
    /// The group without the player, in order; a raid when `raid`.
    pub group: Vec<UnitId>,
    pub raid: bool,
    pub bosses: Vec<Option<UnitId>>,
    /// Each restriction type's state: 0 inactive, 1 activating, 2 active.
    pub restrictions: [u8; 6],
    pub lockdown: bool,
    /// Auras on the units, in application order, and the last instance ID.
    pub auras: Vec<crate::auras::Aura>,
    pub next_aura: f64,
    /// The test's unit handles' metatable.
    handle_meta: Option<TableRef>,
    next_guid: u64,
    /// Spells units are casting.
    pub casting: crate::casting::Casting,
}

pub const PLAYER: UnitId = UnitId(0);

/// A class file's ID (`UnitClass`'s third result).
fn class_id(class: &str) -> f64 {
    const CLASSES: &[&str] = &[
        "WARRIOR",
        "PALADIN",
        "HUNTER",
        "ROGUE",
        "PRIEST",
        "DEATHKNIGHT",
        "SHAMAN",
        "MAGE",
        "WARLOCK",
        "MONK",
        "DRUID",
        "DEMONHUNTER",
        "EVOKER",
    ];
    CLASSES
        .iter()
        .position(|c| *c == class)
        .map_or(0.0, |i| i as f64 + 1.0)
}

/// Each class's ID, file and name (enUS; the client's for Paladin,
/// `UnitClass`; the others unverified).
const CLASS_INFO: &[(f64, &str, &str)] = &[
    (1.0, "WARRIOR", "Warrior"),
    (2.0, "PALADIN", "Paladin"),
    (3.0, "HUNTER", "Hunter"),
    (4.0, "ROGUE", "Rogue"),
    (5.0, "PRIEST", "Priest"),
    (6.0, "DEATHKNIGHT", "Death Knight"),
    (7.0, "SHAMAN", "Shaman"),
    (8.0, "MAGE", "Mage"),
    (9.0, "WARLOCK", "Warlock"),
    (10.0, "MONK", "Monk"),
    (11.0, "DRUID", "Druid"),
    (12.0, "DEMONHUNTER", "Demon Hunter"),
    (13.0, "EVOKER", "Evoker"),
];

/// `GetClassInfo(classID)`: the class's name, file and ID.
fn get_class_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = match vm.hidden(c.arg(1)) {
        Value::Num(n) => n,
        _ => return Ok(vec![]),
    };
    Ok(CLASS_INFO
        .iter()
        .find(|(i, _, _)| *i == id)
        .map(|(i, file, name)| vec![vm.str(name), vm.str(file), Value::Num(*i)])
        .unwrap_or_default())
}

/// The classes camelot has (WoW Forever: `LocalizedClassList` lists nine;
/// no class color for the others).
const CAMELOT_CLASSES: &[f64] = &[1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 8.0, 9.0, 11.0];

/// `C_CreatureInfo.GetClassInfo(classID)`: `{ className, classFile,
/// classID }` for camelot's classes; nil for another (recorded for 0;
/// unverified for the rest, probes queued).
fn creature_class_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = match vm.hidden(c.arg(1)) {
        Value::Num(n) => n,
        _ => return Ok(vec![Value::Nil]),
    };
    let Some((i, file, name)) = CLASS_INFO
        .iter()
        .find(|(i, _, _)| *i == id && CAMELOT_CLASSES.contains(i))
    else {
        return Ok(vec![Value::Nil]);
    };
    let t = vm.new_table();
    for (k, v) in [
        ("className", vm.str(name)),
        ("classFile", vm.str(file)),
        ("classID", Value::Num(*i)),
    ] {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `C_SpecializationInfo.GetAllClassIDs()`: camelot's classes (unverified,
/// probe queued).
fn all_class_ids(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = vm.new_table();
    for (i, id) in CAMELOT_CLASSES.iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(i as f64 + 1.0), Value::Num(*id));
    }
    Ok(vec![Value::Table(t)])
}

/// The player the world starts with.
pub fn install(vm: &mut Vm) {
    let player = Unit {
        guid: "Player-4621-0A000001".into(),
        name: "Player".into(),
        realm: None,
        last_name: None,
        level: 60.0,
        class: "WARRIOR".into(),
        class_name: "Warrior".into(),
        class_id: 1.0,
        race: "Human".into(),
        faction_group: None,
        sex: 2.0,
        reaction: Reaction::Friendly,
        player: true,
        health: 100.0,
        health_max: 100.0,
        power: 0.0,
        power_max: 100.0,
        power_type: 1.0,
        dead: false,
        pvp: false,
        ghost: false,
        in_combat: false,
        target: None,
        alive_in_world: true,
        distance: Some(0.0),
        classification: "normal".into(),
        threat: None,
        loot: Vec::new(),
        creature_type: None,
        creature_family: None,
    };
    engine_mut(vm).units.list.push(player);
}

// ------------------------------------------------------------- tokens

/// The unit a token names, if any (`target`, `nameplate3`, `party1target`).
pub fn resolve(u: &Units, token: &str) -> Option<UnitId> {
    let t = token.to_ascii_lowercase();
    let (mut unit, mut rest) = base_token(u, &t)?;
    while !rest.is_empty() {
        rest = rest.strip_prefix("target")?;
        unit = u.list[unit?.0].target;
    }
    unit.filter(|id| u.list[id.0].alive_in_world)
}

/// A base token's unit (possibly none: `target` without one) and what is
/// left of the token after it.
fn base_token<'a>(u: &Units, t: &'a str) -> Option<(Option<UnitId>, &'a str)> {
    let numbered = |prefix: &str,
                    list: &dyn Fn(usize) -> Option<UnitId>|
     -> Option<(Option<UnitId>, &'a str)> {
        let rest = t.strip_prefix(prefix)?;
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let n: usize = rest[..digits].parse().ok()?;
        Some((list(n), &rest[digits..]))
    };
    for (name, unit) in [
        ("player", Some(PLAYER)),
        ("target", u.target),
        ("focus", u.focus),
        ("mouseover", u.mouseover),
        ("softenemy", u.soft[0]),
        ("softfriend", u.soft[1]),
        ("softinteract", u.soft[2]),
        ("npc", u.npc),
        ("questnpc", u.npc),
        ("none", None),
    ] {
        if let Some(rest) = t.strip_prefix(name) {
            // `targettarget` is target's target, not a token of its own.
            return Some((unit, rest));
        }
    }
    let group = |n: usize| {
        if u.raid {
            // raid1 is the player, raid2.. the others (unverified).
            if n == 1 {
                Some(PLAYER)
            } else {
                u.group.get(n.wrapping_sub(2)).copied()
            }
        } else {
            None
        }
    };
    let party = |n: usize| {
        (!u.raid)
            .then(|| u.group.get(n.wrapping_sub(1)).copied())
            .flatten()
    };
    numbered("nameplate", &|n| {
        u.nameplates.get(n.wrapping_sub(1)).copied().flatten()
    })
    .or_else(|| numbered("party", &party))
    .or_else(|| numbered("raid", &group))
    .or_else(|| {
        numbered("boss", &|n| {
            u.bosses.get(n.wrapping_sub(1)).copied().flatten()
        })
    })
}

/// The tokens that name a unit by themselves (not compounds), in the order
/// unit events fire for them (unverified).
pub fn tokens_of(u: &Units, id: UnitId) -> Vec<String> {
    // Recorded orders: nameplate3 before target, player before
    // targettarget (`/stdtest engine record combat`); the rest unverified.
    let mut out = Vec::new();
    if id == PLAYER {
        out.push("player".to_string());
    }
    for (i, n) in u.nameplates.iter().enumerate() {
        if *n == Some(id) {
            out.push(format!("nameplate{}", i + 1));
        }
    }
    if u.target == Some(id) {
        out.push("target".into());
    }
    if u.target.is_some_and(|t| u.list[t.0].target == Some(id)) {
        out.push("targettarget".into());
    }
    // The NPC talked to: after target, before softfriend (recorded: a
    // flight master's `UNIT_TARGET`).
    if u.npc == Some(id) {
        out.push("npc".into());
    }
    // After target (`/stdtest engine record combat`: UNIT_AURA for
    // softenemy after target); the rest's order unverified.
    for (i, name) in ["softenemy", "softfriend", "softinteract"]
        .iter()
        .enumerate()
    {
        if u.soft[i] == Some(id) {
            out.push((*name).into());
        }
    }
    if u.focus == Some(id) {
        out.push("focus".into());
    }
    if u.mouseover == Some(id) {
        out.push("mouseover".into());
    }
    for (i, g) in u.group.iter().enumerate() {
        if *g == id {
            out.push(if u.raid {
                format!("raid{}", i + 2)
            } else {
                format!("party{}", i + 1)
            });
        }
    }
    if u.raid && id == PLAYER {
        out.push("raid1".into());
    }
    for (i, b) in u.bosses.iter().enumerate() {
        if *b == Some(id) {
            out.push(format!("boss{}", i + 1));
        }
    }
    out
}

/// Fires a unit event once for each token the unit has.
pub(crate) fn fire_unit(vm: &mut Vm, event: &str, id: UnitId, extra: &[Value]) {
    for token in tokens_of(&engine(vm).units, id) {
        let t = vm.str(&token);
        let mut args = vec![t];
        args.extend_from_slice(extra);
        widgets::fire_event(vm, event, &args);
    }
}

fn fire(vm: &mut Vm, event: &str, args: &[Value]) {
    widgets::fire_event(vm, event, args);
}

// ------------------------------------------------------------ secrecy

/// Whether any of the restriction types is active.
pub fn active(u: &Units, types: &[usize]) -> bool {
    types.iter().any(|&t| u.restrictions[t] == 2)
}

/// Whether a unit is under the player's control or in the group.
fn ours(u: &Units, id: UnitId) -> bool {
    id == PLAYER || u.group.contains(&id)
}

/// Whether a documented function's results are secret for this call, from
/// its flags and the world (recorded on WoW Forever; see the module).
pub fn secret_results(vm: &Vm, doc: &serde_json::Value, c: &NativeCall) -> bool {
    let flag = |k: &str| doc.get(k).and_then(serde_json::Value::as_bool) == Some(true);
    let u = &engine(vm).units;
    if flag("SecretReturns") {
        return true;
    }
    // The unit the call is about: its first unit token argument.
    let unit = match c.arg(1) {
        Value::Str(s) => resolve(u, &String::from_utf8_lossy(vm.str_bytes(s))),
        _ => None,
    };
    let restricted = active(u, &[COMBAT, ENCOUNTER, CHALLENGE_MODE, PVP_MATCH]);
    if (flag("SecretWhenUnitAuraRestricted")
        || flag("SecretWhenAurasRestricted")
        || flag("SecretWhenCooldownsRestricted")
        || flag("SecretWhenTotemSlotSecret"))
        && restricted
    {
        return true;
    }
    if flag("SecretWhenInCombat") && active(u, &[COMBAT]) {
        return true;
    }
    if flag("SecretOnRestrictedMaps") && active(u, &[MAP]) {
        return true;
    }
    let Some(id) = unit else {
        // A unit that does not exist: its maximum health and casting are
        // secret (`/stdtest restrictions`, no target).
        return flag("SecretWhenUnitHealthMaxRestricted")
            || flag("SecretWhenUnitSpellCastRestricted");
    };
    // Another's maximum health and power: secret, another player's too
    // (recorded, scenario flight); the player's and the group's not.
    if (flag("SecretWhenUnitHealthMaxRestricted") || flag("SecretWhenUnitPowerMaxRestricted"))
        && !ours(u, id)
    {
        return true;
    }
    if flag("SecretWhenUnitSpellCastRestricted") && id != PLAYER {
        return true;
    }
    if (flag("SecretWhenUnitIdentityRestricted") || flag("SecretWhenUnitNameIdentityRestricted"))
        && active(u, &[MAP, ENCOUNTER, CHALLENGE_MODE, PVP_MATCH])
        && !ours(u, id)
    {
        return true;
    }
    // Another unit's power is secret even out of combat (the scenarios: a
    // wolf's, another player's out of combat; none targeted, 0); the
    // player's in combat, and the player's rage always (every scenario,
    // out of combat too). Unverified: the group's; the player's other
    // powers out of combat.
    if flag("SecretWhenUnitPowerRestricted")
        && (!ours(u, id)
            || active(u, &[COMBAT, MAP, ENCOUNTER, CHALLENGE_MODE, PVP_MATCH])
            || (id == PLAYER && u.list[id.0].power_type == 1.0))
    {
        return true;
    }
    false
}

// --------------------------------------------------- the unit functions

pub(crate) fn unit_arg(vm: &Vm, c: &NativeCall, n: usize) -> Option<UnitId> {
    match vm.hidden(c.arg(n)) {
        Value::Str(s) => resolve(&engine(vm).units, &String::from_utf8_lossy(vm.str_bytes(s))),
        _ => None,
    }
}

fn with_unit(vm: &mut Vm, c: &NativeCall, f: impl FnOnce(&mut Vm, &Unit) -> Vec<Value>) -> Ret {
    match unit_arg(vm, c, 1) {
        Some(id) => {
            // The unit's fields are copied out before `f` makes strings.
            let unit = clone_unit(&engine(vm).units.list[id.0]);
            Ok(f(vm, &unit))
        }
        None => Ok(vec![]),
    }
}

fn clone_unit(u: &Unit) -> Unit {
    Unit {
        guid: u.guid.clone(),
        name: u.name.clone(),
        realm: u.realm.clone(),
        last_name: u.last_name.clone(),
        class: u.class.clone(),
        class_name: u.class_name.clone(),
        race: u.race.clone(),
        faction_group: u.faction_group.clone(),
        classification: u.classification.clone(),
        loot: u.loot.clone(),
        creature_type: u.creature_type.clone(),
        creature_family: u.creature_family.clone(),
        ..*u
    }
}

fn unit_exists(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(unit_arg(vm, &c, 1).is_some())])
}

fn unit_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| vec![vm.str(&u.guid)])
}

/// `UnitTokenFromGUID(guid)`: a token naming the unit (the first in
/// `tokens_of`'s order; unverified which the client prefers), or nil.
fn unit_token_from_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(g) = c.arg(1) else {
        return Ok(vec![Value::Nil]);
    };
    let guid = String::from_utf8_lossy(vm.str_bytes(g)).into_owned();
    let u = &engine(vm).units;
    let token = (0..u.list.len())
        .map(UnitId)
        .filter(|id| u.list[id.0].guid == guid)
        .find_map(|id| tokens_of(u, id).into_iter().next());
    Ok(vec![token.map_or(Value::Nil, |t| vm.str(&t))])
}

fn unit_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let home = engine(vm).units.list[PLAYER.0].realm.clone();
    with_unit(vm, &c, |vm, u| {
        // On WoW Forever the second value is the last name; elsewhere a
        // player from another realm's realm (the player's own realm none).
        let other_realm = u.realm.as_ref().filter(|r| Some(*r) != home.as_ref());
        let second = u
            .last_name
            .as_ref()
            .or(other_realm)
            .map_or(Value::Nil, |r| vm.str(r));
        vec![vm.str(&u.name), second]
    })
}

/// For a unit that does not exist, the numbers are 0 and the flags false
/// (`/stdtest restrictions`: no target; identity calls give nothing).
fn with_unit_or(
    vm: &mut Vm,
    c: &NativeCall,
    none: Value,
    f: impl FnOnce(&mut Vm, &Unit) -> Vec<Value>,
) -> Ret {
    if unit_arg(vm, c, 1).is_none() {
        return Ok(vec![none]);
    }
    with_unit(vm, c, f)
}

fn unit_level(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Num(0.0), |_, u| vec![Value::Num(u.level)])
}

/// `UnitClass`: an NPC's class name is its own name (`/stdtest
/// restrictions`: "Plainstrider", "WARRIOR", 1).
fn unit_class(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| {
        vec![
            vm.str(if u.player { &u.class_name } else { &u.name }),
            vm.str(&u.class),
            Value::Num(u.class_id),
        ]
    })
}

/// `UnitRace`: the race's name, file name and ID (recorded: "Undead",
/// "Scourge", 5); a creature has none (`/stdtest engine scenario combat`).
fn unit_race(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| {
        if u.race.is_empty() {
            return vec![];
        }
        let (file, id) = race_file_and_id(&u.race);
        let name = race_name(&u.race);
        vec![
            vm.str(&name),
            vm.str(&file),
            id.map_or(Value::Nil, Value::Num),
        ]
    })
}

/// A race's file name and ID (the client's races; only the Undead's
/// recorded, the others unverified).
fn race_file_and_id(race: &str) -> (String, Option<f64>) {
    const RACES: &[(&str, &str, f64)] = &[
        ("Human", "Human", 1.0),
        ("Orc", "Orc", 2.0),
        ("Dwarf", "Dwarf", 3.0),
        ("Night Elf", "NightElf", 4.0),
        ("Undead", "Scourge", 5.0),
        ("Tauren", "Tauren", 6.0),
        ("Gnome", "Gnome", 7.0),
        ("Troll", "Troll", 8.0),
        ("Goblin", "Goblin", 9.0),
        ("Blood Elf", "BloodElf", 10.0),
        ("Draenei", "Draenei", 11.0),
        ("Worgen", "Worgen", 22.0),
    ];
    match RACES
        .iter()
        .find(|(name, file, _)| *name == race || *file == race)
    {
        Some((_, file, id)) => (file.to_string(), Some(*id)),
        None => (race.replace(' ', ""), None),
    }
}

/// A race's name as the client shows it (a snapshot may give its file
/// name: "Scourge" is "Undead"; one the list lacks, as given).
fn race_name(race: &str) -> String {
    const NAMES: &[(&str, &str)] = &[
        ("Scourge", "Undead"),
        ("NightElf", "Night Elf"),
        ("BloodElf", "Blood Elf"),
    ];
    NAMES
        .iter()
        .find(|(file, _)| *file == race)
        .map_or_else(|| race.to_string(), |(_, name)| name.to_string())
}

// No such unit (a target cleared): level 0, "normal", not a player, not
// dead (recorded, scenario combat).

fn unit_effective_level(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Num(0.0), |_, u| vec![Value::Num(u.level)])
}

fn unit_classification(vm: &mut Vm, c: NativeCall) -> Ret {
    let normal = vm.str("normal");
    with_unit_or(vm, &c, normal, |vm, u| vec![vm.str(&u.classification)])
}

fn unit_creature_type(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| match &u.creature_type {
        Some((name, id)) => vec![vm.str(name), Value::Num(*id)],
        None => vec![],
    })
}

fn unit_creature_family(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| match &u.creature_family {
        Some((name, id)) => vec![vm.str(name), Value::Num(*id)],
        None => vec![],
    })
}

/// A boss, quest boss, other player's pet, trivial or wild battle pet: none
/// the world has yet (recorded false for a wolf).
fn unit_not_special(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, _| vec![Value::Bool(false)])
}

/// `UnitIsConnected`: every unit there is.
fn unit_is_connected(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, _| vec![Value::Bool(true)])
}

/// `UnitShouldDisplayName`: others' names, not the player's (recorded).
fn unit_should_display_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_arg(vm, &c, 1);
    Ok(vec![Value::Bool(id.is_some_and(|id| id != PLAYER))])
}

/// `UnitSelectionColor(unit [, useExtendedColors])`: a creature's by its
/// reaction (recorded: a neutral wolf's yellow); the player's as recorded.
fn unit_selection_color(vm: &mut Vm, c: NativeCall) -> Ret {
    // No such unit: blue (recorded, a target cleared).
    let Some(id) = unit_arg(vm, &c, 1) else {
        return Ok([0.0, 0.0, 1.0, 1.0].map(Value::Num).to_vec());
    };
    if engine(vm).units.list[id.0].player {
        // A player: blue (recorded, another player targeted); with
        // extended colors as recorded for the player.
        if c.arg(2).truthy() {
            return Ok(
                crate::recorded::lookup_text(vm, "UnitSelectionColor(\"player\", true)")
                    .unwrap_or_else(|| [0.0, 0.0, 1.0, 1.0].map(Value::Num).to_vec()),
            );
        }
        return Ok([0.0, 0.0, 1.0, 1.0].map(Value::Num).to_vec());
    }
    let rgb = match engine(vm).units.list[id.0].reaction {
        Reaction::Hostile => [1.0, 0.0, 0.0],
        Reaction::Neutral => [1.0, 1.0, 0.0],
        Reaction::Friendly => [0.0, 1.0, 0.0],
    };
    Ok(vec![
        Value::Num(rgb[0]),
        Value::Num(rgb[1]),
        Value::Num(rgb[2]),
        Value::Num(1.0),
    ])
}

/// `UnitPlayerControlled`: a player (recorded; pets unverified).
fn unit_player_controlled(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| {
        vec![Value::Bool(u.player)]
    })
}

/// `UnitIsPVP`: flagged for PvP (another player's, recorded).
fn unit_is_pvp(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| vec![Value::Bool(u.pvp)])
}

fn unit_sex(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |_, u| vec![Value::Num(u.sex)])
}

fn unit_health(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Num(0.0), |_, u| vec![Value::Num(u.health)])
}

fn unit_health_max(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Num(0.0), |_, u| {
        vec![Value::Num(u.health_max)]
    })
}

fn unit_power(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Num(0.0), |_, u| vec![Value::Num(u.power)])
}

fn unit_power_max(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Num(0.0), |_, u| {
        vec![Value::Num(u.power_max)]
    })
}

/// A power type's token (`UnitPowerType`'s second result: `/stdtest
/// engine` recorded `0, "MANA"`; the others by `Enum.PowerType`).
fn power_token(power_type: f64) -> Option<&'static str> {
    const TOKENS: &[&str] = &[
        "MANA",
        "RAGE",
        "FOCUS",
        "ENERGY",
        "COMBO_POINTS",
        "RUNES",
        "RUNIC_POWER",
        "SOUL_SHARDS",
        "LUNAR_POWER",
        "HOLY_POWER",
        "ALTERNATE",
        "MAELSTROM",
        "CHI",
        "INSANITY",
    ];
    usize::try_from(power_type as i64)
        .ok()
        .and_then(|i| TOKENS.get(i).copied())
}

fn unit_power_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = with_unit(vm, &c, |_, u| vec![Value::Num(u.power_type)])?;
    let Some(Value::Num(n)) = t.first().copied() else {
        return Ok(t);
    };
    Ok(match power_token(n) {
        Some(token) => vec![Value::Num(n), vm.str(token)],
        None => t,
    })
}

fn unit_is_dead(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| vec![Value::Bool(u.dead)])
}

fn unit_is_player(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| {
        vec![Value::Bool(u.player)]
    })
}

fn unit_affecting_combat(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| {
        vec![Value::Bool(u.in_combat)]
    })
}

fn unit_is_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let a = unit_arg(vm, &c, 1);
    let b = unit_arg(vm, &c, 2);
    Ok(vec![Value::Bool(a.is_some() && a == b)])
}

/// The reaction between `a` and `b`, the same both ways (1 hated .. 8
/// exalted; hostile 2, neutral 4, friendly 5: a neutral wolf's is 4 toward
/// the player and the player's toward it, recorded).
fn unit_reaction(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(reaction) = reaction_of(vm, &c) else {
        return Ok(vec![]);
    };
    let r = match reaction {
        Reaction::Hostile => 2.0,
        Reaction::Neutral => 4.0,
        Reaction::Friendly => 5.0,
    };
    Ok(vec![Value::Num(r)])
}

fn reaction_of(vm: &Vm, c: &NativeCall) -> Option<Reaction> {
    let (a, b) = (unit_arg(vm, c, 1)?, unit_arg(vm, c, 2)?);
    let u = &engine(vm).units;
    // Between the player's side and a unit: that unit's reaction.
    Some(if ours(u, a) {
        u.list[b.0].reaction
    } else {
        u.list[a.0].reaction
    })
}

fn unit_is_enemy(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        reaction_of(vm, &c) == Some(Reaction::Hostile),
    )])
}

fn unit_is_friend(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        reaction_of(vm, &c) == Some(Reaction::Friendly),
    )])
}

fn unit_can_attack(vm: &mut Vm, c: NativeCall) -> Ret {
    let r = reaction_of(vm, &c);
    Ok(vec![Value::Bool(matches!(
        r,
        Some(Reaction::Hostile | Reaction::Neutral)
    ))])
}

fn unit_can_assist(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        reaction_of(vm, &c) == Some(Reaction::Friendly),
    )])
}

fn unit_in_party(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_arg(vm, &c, 1);
    let u = &engine(vm).units;
    let in_party = id.is_some_and(|id| !u.group.is_empty() && ours(u, id));
    Ok(vec![Value::Bool(in_party)])
}

fn unit_in_raid(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_arg(vm, &c, 1);
    let u = &engine(vm).units;
    // The raid index (1-based), or nil.
    Ok(vec![match id {
        Some(PLAYER) if u.raid => Value::Num(1.0),
        Some(id) if u.raid => u
            .group
            .iter()
            .position(|&g| g == id)
            .map_or(Value::Nil, |i| Value::Num(i as f64 + 2.0)),
        _ => Value::Nil,
    }])
}

fn is_in_group(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(!engine(vm).units.group.is_empty())])
}

fn is_in_raid(vm: &mut Vm, _c: NativeCall) -> Ret {
    let u = &engine(vm).units;
    Ok(vec![Value::Bool(u.raid && !u.group.is_empty())])
}

fn get_num_group_members(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = engine(vm).units.group.len();
    Ok(vec![Value::Num(if n == 0 { 0.0 } else { n as f64 + 1.0 })])
}

/// `GetNumSubgroupMembers()`: the others in the player's party (raid
/// subgroups unverified).
fn get_num_subgroup_members(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).units.group.len() as f64)])
}

fn in_combat_lockdown(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).units.lockdown)])
}

fn restriction_arg(vm: &mut Vm, c: &NativeCall) -> Result<usize, wowlua_vm::LuaError> {
    let t = vm.check_int(c, 1)?;
    Ok(usize::try_from(t)
        .ok()
        .filter(|&t| t < 6)
        .unwrap_or(usize::MAX))
}

fn get_restriction_state(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = restriction_arg(vm, &c)?;
    let state = engine(vm).units.restrictions.get(t).copied().unwrap_or(0);
    Ok(vec![Value::Num(f64::from(state))])
}

fn is_restriction_active(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = restriction_arg(vm, &c)?;
    let state = engine(vm).units.restrictions.get(t).copied().unwrap_or(0);
    Ok(vec![Value::Bool(state == 2)])
}

/// Protected functions are refused for tainted code in combat lockdown.
fn check_allow_protected(vm: &mut Vm, _c: NativeCall) -> Ret {
    let allowed = !engine(vm).units.lockdown || vm.taint().stack.is_secure();
    Ok(vec![Value::Bool(allowed)])
}

/// `C_Secrets.Should…BeSecret(unit)`: the same predicates the unit
/// functions' results follow.
fn should_be_secret(flag: &'static str) -> impl Fn(&mut Vm, NativeCall) -> Ret {
    move |vm: &mut Vm, c: NativeCall| {
        // A unit that does not exist: its maximum health and its casting
        // are secret (`/stdtest restrictions`, no target).
        if matches!(
            flag,
            "SecretWhenUnitHealthMaxRestricted" | "SecretWhenUnitSpellCastRestricted"
        ) && unit_arg(vm, &c, 1).is_none()
        {
            return Ok(vec![Value::Bool(true)]);
        }
        let doc = serde_json::json!({ flag: true });
        Ok(vec![Value::Bool(secret_results(vm, &doc, &c))])
    }
}

/// `C_Secrets.Get…Secrecy(…)`: contextually secret (2) in every recorded
/// state.
fn contextually_secret(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(2.0)])
}

/// Spells' auras and cooldowns, and unit stats: secret in combat
/// (`/stdtest restrictions`).
fn secret_in_combat(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(active(&engine(vm).units, &[COMBAT]))])
}

macro_rules! should {
    ($name:ident, $flag:literal) => {
        fn $name(vm: &mut Vm, c: NativeCall) -> Ret {
            should_be_secret($flag)(vm, c)
        }
    };
}
should!(should_identity, "SecretWhenUnitIdentityRestricted");
should!(should_power, "SecretWhenUnitPowerRestricted");
should!(should_power_max, "SecretWhenUnitPowerMaxRestricted");
should!(should_health_max, "SecretWhenUnitHealthMaxRestricted");
should!(should_casting, "SecretWhenUnitSpellCastRestricted");
should!(should_auras, "SecretWhenUnitAuraRestricted");
should!(should_cooldowns, "SecretWhenCooldownsRestricted");

fn has_secret_restrictions(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

/// The documented functions this module gives behaviour.
pub fn behaviours() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        ("UnitExists", unit_exists),
        ("UnitGUID", unit_guid),
        ("UnitTokenFromGUID", unit_token_from_guid),
        ("UnitName", unit_name),
        ("UnitLevel", unit_level),
        ("UnitClass", unit_class),
        ("UnitRace", unit_race),
        ("UnitEffectiveLevel", unit_effective_level),
        ("UnitClassification", unit_classification),
        ("UnitCreatureType", unit_creature_type),
        ("UnitCreatureFamily", unit_creature_family),
        ("UnitIsQuestBoss", unit_not_special),
        ("UnitIsBossMob", unit_not_special),
        ("UnitIsOtherPlayersPet", unit_not_special),
        ("UnitIsTrivial", unit_not_special),
        ("UnitIsWildBattlePet", unit_not_special),
        ("UnitIsConnected", unit_is_connected),
        ("UnitShouldDisplayName", unit_should_display_name),
        ("UnitSelectionColor", unit_selection_color),
        (
            "UnitDetailedThreatSituation",
            unit_detailed_threat_situation,
        ),
        ("UnitThreatSituation", unit_threat_situation),
        ("UnitSex", unit_sex),
        ("UnitPlayerControlled", unit_player_controlled),
        ("UnitIsPVP", unit_is_pvp),
        ("UnitHealth", unit_health),
        ("UnitHealthMax", unit_health_max),
        ("UnitPower", unit_power),
        ("UnitPowerMax", unit_power_max),
        ("UnitPowerType", unit_power_type),
        ("UnitIsDead", unit_is_dead),
        ("UnitIsDeadOrGhost", unit_is_dead_or_ghost),
        ("UnitIsPlayer", unit_is_player),
        ("UnitAffectingCombat", unit_affecting_combat),
        ("UnitIsUnit", unit_is_unit),
        ("UnitReaction", unit_reaction),
        ("UnitIsEnemy", unit_is_enemy),
        ("UnitIsFriend", unit_is_friend),
        ("UnitCanAttack", unit_can_attack),
        ("UnitCanAssist", unit_can_assist),
        ("UnitInParty", unit_in_party),
        ("UnitInRaid", unit_in_raid),
        ("IsInGroup", is_in_group),
        ("IsInRaid", is_in_raid),
        ("GetNumGroupMembers", get_num_group_members),
        ("GetNumSubgroupMembers", get_num_subgroup_members),
        ("InCombatLockdown", in_combat_lockdown),
        (
            "C_RestrictedActions.GetAddOnRestrictionState",
            get_restriction_state,
        ),
        (
            "C_RestrictedActions.IsAddOnRestrictionActive",
            is_restriction_active,
        ),
        (
            "C_RestrictedActions.CheckAllowProtectedFunctions",
            check_allow_protected,
        ),
        ("C_Secrets.HasSecretRestrictions", has_secret_restrictions),
        ("C_Secrets.ShouldUnitIdentityBeSecret", should_identity),
        ("C_Secrets.ShouldUnitPowerBeSecret", should_power),
        ("C_Secrets.ShouldUnitPowerMaxBeSecret", should_power_max),
        ("C_Secrets.ShouldUnitHealthMaxBeSecret", should_health_max),
        ("C_Secrets.ShouldUnitSpellCastingBeSecret", should_casting),
        ("C_Secrets.ShouldAurasBeSecret", should_auras),
        ("C_Secrets.ShouldCooldownsBeSecret", should_cooldowns),
        ("C_Secrets.GetPowerTypeSecrecy", contextually_secret),
        ("C_Secrets.GetSpellAuraSecrecy", contextually_secret),
        ("C_Secrets.GetSpellCastSecrecy", contextually_secret),
        ("C_Secrets.GetSpellCooldownSecrecy", contextually_secret),
        ("C_Secrets.ShouldSpellAuraBeSecret", secret_in_combat),
        ("C_Secrets.ShouldSpellCooldownBeSecret", secret_in_combat),
        ("C_Secrets.ShouldUnitStatsBeSecret", secret_in_combat),
    ]
}

// ----------------------------------------------- the controls (mock)

fn handle_meta(vm: &mut Vm) -> TableRef {
    if let Some(t) = engine(vm).units.handle_meta {
        return t;
    }
    let methods = vm.new_table();
    for (name, f) in [
        ("target", control_target as wowlua_vm::NativeFn),
        ("softTarget", control_soft_target),
        ("focus", control_focus),
        ("mouseover", control_mouseover),
        ("damage", control_damage),
        ("heal", control_heal),
        ("kill", control_kill),
        ("setPower", control_set_power),
        ("enterCombat", control_enter_combat),
        ("startAttack", control_start_attack),
        ("stopAttack", control_stop_attack),
        ("swing", control_swing),
        ("loot", crate::loot::control_loot),
        ("interact", crate::quests::control_interact),
        ("die", crate::death::control_die),
        ("setThreat", control_set_threat),
        ("raiseSkill", control_raise_skill),
        ("leaveCombat", control_leave_combat),
        ("showNameplate", control_show_nameplate),
        ("hideNameplate", control_hide_nameplate),
        ("despawn", control_despawn),
        ("token", control_tokens),
        ("setName", control_set_name),
        ("applyAura", crate::auras::control_apply),
        ("removeAura", crate::auras::control_remove),
        ("castSpell", crate::casting::control_cast),
        ("useSpell", crate::casting::control_use_spell),
        ("interruptCast", crate::casting::control_interrupt),
        ("setDistance", crate::character::control_set_distance),
        ("setMoney", crate::character::control_set_money),
        ("setStats", crate::stats::control_set_stats),
        ("equip", crate::character::control_equip),
        ("setDurability", crate::character::control_set_durability),
        ("wear", crate::character::control_wear),
    ] {
        let native = vm.native(name, f);
        let key = vm.heap.intern(name.as_bytes());
        vm.heap.table_mut(methods).set_str(key, native);
    }
    let meta = vm.new_table();
    let index = vm.heap.intern(b"__index");
    vm.heap
        .table_mut(meta)
        .set_str(index, Value::Table(methods));
    widgets::keep(vm, Value::Table(meta));
    engine_mut(vm).units.handle_meta = Some(meta);
    meta
}

/// A test's handle on a unit: a table with the control methods.
pub fn handle(vm: &mut Vm, id: UnitId) -> Value {
    let meta = handle_meta(vm);
    let t = vm.new_table();
    let ud = vm.heap.alloc_userdata(Userdata {
        serial: 0,
        meta: None,
        env: None,
        data: Box::new(id),
    });
    let _ = vm
        .heap
        .table_mut(t)
        .set(Value::Num(0.0), Value::Userdata(ud));
    vm.heap.table_mut(t).meta = Some(meta);
    Value::Table(t)
}

pub fn handle_of(vm: &Vm, v: Value) -> Option<UnitId> {
    let Value::Table(t) = v else { return None };
    let Value::Userdata(u) = vm.heap.table(t).get(Value::Num(0.0)) else {
        return None;
    };
    vm.heap.userdata(u).data.downcast_ref::<UnitId>().copied()
}

pub fn this(vm: &mut Vm, c: &NativeCall) -> Result<UnitId, wowlua_vm::LuaError> {
    handle_of(vm, c.arg(1))
        .ok_or_else(|| vm.error("a unit method called without its unit (use ':')"))
}

/// `mock.spawnUnit({ name, level, class, race, sex, reaction, player,
/// health, maxHealth, power, maxPower, powerType, guid, npcID, distance
/// })`.
pub fn spawn(vm: &mut Vm, c: NativeCall) -> Ret {
    let data = match c.arg(1) {
        Value::Table(t) => Some(t),
        Value::Nil => None,
        _ => return Err(vm.error("mock.spawnUnit: a table of the unit's data expected")),
    };
    let unit = unit_from(vm, data, None);
    let units = &mut engine_mut(vm).units;
    units.list.push(unit);
    let id = UnitId(units.list.len() - 1);
    Ok(vec![handle(vm, id)])
}

/// `mock.setPlayer({ name, realm, lastName, level, class, race, sex,
/// health, maxHealth, power, maxPower, powerType, guid })`: the player's
/// data (a world profile's character), firing no events.
pub fn set_player(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("mock.setPlayer: a table of the player's data expected"));
    };
    let player = std::mem::take(&mut engine_mut(vm).units.list[PLAYER.0].guid);
    let mut unit = unit_from(vm, Some(t), Some(player));
    let old = &engine(vm).units.list[PLAYER.0];
    unit.target = old.target;
    unit.in_combat = old.in_combat;
    unit.distance = Some(0.0);
    engine_mut(vm).units.list[PLAYER.0] = unit;
    Ok(vec![])
}

/// A unit from a test's data; `player_guid`: the player's (it keeps its
/// GUID unless the data has one).
/// A friendly NPC named `name`, spawned.
pub fn spawn_named(vm: &mut Vm, name: &str) -> UnitId {
    let data = vm.new_table();
    for (k, v) in [("name", name), ("reaction", "friendly")] {
        let key = vm.str(k);
        let v = vm.str(v);
        let _ = vm.heap.table_mut(data).set(key, v);
    }
    let unit = unit_from(vm, Some(data), None);
    let units = &mut engine_mut(vm).units;
    units.list.push(unit);
    UnitId(units.list.len() - 1)
}

fn unit_from(vm: &mut Vm, data: Option<TableRef>, player_guid: Option<String>) -> Unit {
    let get = |vm: &mut Vm, k: &str| -> Value {
        match data {
            Some(t) => {
                let key = vm.heap.intern(k.as_bytes());
                vm.heap.table(t).get_str(key)
            }
            None => Value::Nil,
        }
    };
    let text = |vm: &mut Vm, k: &str, default: &str| -> String {
        match get(vm, k) {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => default.to_string(),
        }
    };
    let num = |vm: &mut Vm, k: &str, default: f64| match get(vm, k) {
        Value::Num(n) => n,
        _ => default,
    };
    let is_player = player_guid.is_some();
    let player = is_player || get(vm, "player").truthy();
    let reaction = match text(
        vm,
        "reaction",
        if is_player { "friendly" } else { "hostile" },
    )
    .as_str()
    {
        "friendly" => Reaction::Friendly,
        "neutral" => Reaction::Neutral,
        _ => Reaction::Hostile,
    };
    let n = {
        let u = &mut engine_mut(vm).units;
        u.next_guid += 1;
        u.next_guid
    };
    let npc = num(vm, "npcID", 1000.0 + n as f64);
    let default_guid = if let Some(g) = player_guid {
        g
    } else if player {
        format!("Player-4621-{n:08X}")
    } else {
        format!("Creature-0-4621-0-1-{npc}-{n:010X}")
    };
    let class = text(vm, "class", if player { "WARRIOR" } else { "" }).to_uppercase();
    let class_name = {
        let mut c = class.to_lowercase();
        if let Some(f) = c.get_mut(0..1) {
            f.make_ascii_uppercase();
        }
        c
    };
    let health = num(vm, "health", 100.0);
    let health_max = num(vm, "maxHealth", health);
    Unit {
        guid: text(vm, "guid", &default_guid),
        name: text(vm, "name", "Unit"),
        realm: match get(vm, "realm") {
            Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
            _ => None,
        },
        last_name: match get(vm, "lastName") {
            Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
            _ => None,
        },
        level: num(vm, "level", 1.0),
        class_id: num(vm, "classID", class_id(&class)),
        class,
        class_name,
        race: text(vm, "race", if player { "Human" } else { "" }),
        faction_group: match get(vm, "factionGroup") {
            Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
            _ => None,
        },
        sex: num(vm, "sex", 1.0),
        reaction,
        player,
        health: num(vm, "health", health_max),
        health_max,
        power: num(vm, "power", 0.0),
        power_max: num(vm, "maxPower", 100.0),
        power_type: num(vm, "powerType", 0.0),
        dead: get(vm, "dead").truthy(),
        pvp: get(vm, "pvp").truthy(),
        ghost: false,
        in_combat: false,
        target: None,
        alive_in_world: true,
        distance: match get(vm, "distance") {
            Value::Num(d) => Some(d),
            _ => None,
        },
        classification: text(vm, "classification", "normal"),
        threat: None,
        loot: Vec::new(),
        creature_type: match get(vm, "creatureType") {
            Value::Str(s) => Some((
                String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                num(vm, "creatureTypeID", 0.0),
            )),
            _ => None,
        },
        creature_family: match get(vm, "creatureFamily") {
            Value::Str(s) => Some((
                String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                num(vm, "creatureFamilyID", 0.0),
            )),
            _ => None,
        },
    }
}

fn target_arg(vm: &Vm, c: &NativeCall) -> Option<UnitId> {
    handle_of(vm, c.arg(2))
}

// The server's `UNIT_TARGET("player")` comes a round trip after the
// player's `PLAYER_TARGET_CHANGED` (`/stdtest engine record target`:
// 0.4–0.6 s; scenario combat 0.3–0.5 s at a ping near 0.3 s).

/// `unit:target(other)`: the player's `PLAYER_TARGET_CHANGED()` at once and
/// `UNIT_TARGET("player")` when the server's echo arrives; another unit's
/// `UNIT_TARGET` at once (unverified).
fn control_target(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let new = target_arg(vm, &c);
    if me == PLAYER {
        engine_mut(vm).units.target = new;
        engine_mut(vm).units.list[PLAYER.0].target = new;
        fire(vm, "PLAYER_TARGET_CHANGED", &[]);
        let echo = vm.native("targetEcho", target_echo);
        let delay = crate::clock::server_delay(vm);
        crate::clock::after_engine(vm, delay, echo);
    } else {
        engine_mut(vm).units.list[me.0].target = new;
        // A creature turning on someone, as a wolf on the player
        // (`/stdtest engine scenario combat`): the new target's flags
        // (also by its new token, `targettarget`), the creature's target,
        // its flags in combat; a friendly one turning to face someone (a
        // flight master, `taxi`): its target only. Unverified: for others
        // than the player.
        let friendly = engine(vm).units.list[me.0].reaction == Reaction::Friendly;
        if let Some(t) = new
            && !friendly
        {
            fire_unit(vm, "UNIT_FLAGS", t, &[]);
        }
        fire_unit(vm, "UNIT_TARGET", me, &[]);
        if engine(vm).units.list[me.0].in_combat {
            fire_unit(vm, "UNIT_FLAGS", me, &[]);
        }
    }
    Ok(vec![])
}

/// `unit:setThreat(value)`: the player's threat on a creature, as the
/// server tells it (`/stdtest engine scenario combat`: 5, 6, 10, … as the
/// fight went): `UNIT_THREAT_LIST_UPDATE(unit)`, the first time also
/// `UNIT_THREAT_SITUATION_UPDATE(player)`.
fn control_set_threat(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let value = vm.check_number(&c, 2)?;
    let first = engine(vm).units.list[me.0].threat.is_none();
    engine_mut(vm).units.list[me.0].threat = Some(value);
    fire_unit(vm, "UNIT_THREAT_LIST_UPDATE", me, &[]);
    if first {
        engine_mut(vm).units.threat_situation = true;
        fire_unit(vm, "UNIT_THREAT_SITUATION_UPDATE", PLAYER, &[]);
    }
    Ok(vec![])
}

/// The player's threat on `mob`: whether tanking, the status, the scaled
/// and raw percentages, the value; none without threat or alive (recorded:
/// `true, 3, 100, 255, 5` alone on a wolf's list, before it turned on the
/// player). Unverified: others on the list (here the player is alone).
fn threat_of(vm: &Vm, unit: UnitId, mob: UnitId) -> Option<(bool, f64, f64, f64, f64)> {
    let u = &engine(vm).units;
    let m = &u.list[mob.0];
    if unit != PLAYER || m.dead {
        return None;
    }
    let value = m.threat?;
    Some((true, 3.0, 100.0, 255.0, value))
}

/// `UnitDetailedThreatSituation(unit, mob)`.
fn unit_detailed_threat_situation(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(unit), Some(mob)) = (unit_arg(vm, &c, 1), unit_arg(vm, &c, 2)) else {
        return Ok(vec![]);
    };
    Ok(match threat_of(vm, unit, mob) {
        Some((tanking, status, scaled, raw, value)) => vec![
            Value::Bool(tanking),
            Value::Num(status),
            Value::Num(scaled),
            Value::Num(raw),
            Value::Num(value),
        ],
        None => vec![],
    })
}

/// `UnitThreatSituation(unit [, mob])`: the status on `mob`, else the
/// highest on any creature; one nil without threat or unit (recorded:
/// `party1`, the player alone, the player and an absent boss).
fn unit_threat_situation(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(unit) = unit_arg(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    let status = match unit_arg(vm, &c, 2) {
        Some(mob) => threat_of(vm, unit, mob).map(|t| t.1),
        None => (0..engine(vm).units.list.len())
            .filter_map(|i| threat_of(vm, unit, UnitId(i)).map(|t| t.1))
            .reduce(f64::max),
    };
    Ok(vec![status.map_or(Value::Nil, Value::Num)])
}

/// `unit:softTarget(other [, kind])` (the player): the soft target of
/// `kind` ("enemy", "friend", "interact"; by default "enemy" for a hostile
/// or neutral unit, else "friend"); nil clears it. `PLAYER_SOFT_ENEMY_CHANGED()` and
/// `PLAYER_SOFT_FRIEND_CHANGED()` have no arguments,
/// `PLAYER_SOFT_INTERACT_CHANGED(oldGUID, newGUID)` the GUIDs
/// (`/stdtest engine record soft`, `softinteract`). Unverified: the soft
/// targeting CVars are not consulted.
fn control_soft_target(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    if me != PLAYER {
        return Err(vm.error("softTarget: the player's"));
    }
    let new = target_arg(vm, &c);
    let kind = match c.arg(3) {
        Value::Str(k) => String::from_utf8_lossy(vm.str_bytes(k)).into_owned(),
        _ => match new {
            Some(id) if engine(vm).units.list[id.0].reaction != Reaction::Friendly => {
                "enemy".into()
            }
            Some(_) => "friend".into(),
            None => return Err(vm.error("softTarget(nil, kind): which soft target to clear")),
        },
    };
    let (i, event) = match kind.as_str() {
        "enemy" => (0, "PLAYER_SOFT_ENEMY_CHANGED"),
        "friend" => (1, "PLAYER_SOFT_FRIEND_CHANGED"),
        "interact" => (2, "PLAYER_SOFT_INTERACT_CHANGED"),
        _ => return Err(vm.error("softTarget: kind is \"enemy\", \"friend\" or \"interact\"")),
    };
    let old = engine(vm).units.soft[i];
    if old == new {
        return Ok(vec![]);
    }
    engine_mut(vm).units.soft[i] = new;
    if i == 2 {
        let guid = |vm: &mut Vm, id: Option<UnitId>| match id {
            Some(id) => {
                let g = engine(vm).units.list[id.0].guid.clone();
                vm.str(&g)
            }
            None => Value::Nil,
        };
        let (o, n) = (guid(vm, old), guid(vm, new));
        fire(vm, event, &[o, n]);
    } else {
        fire(vm, event, &[]);
    }
    Ok(vec![])
}

fn target_echo(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire_unit(vm, "UNIT_TARGET", PLAYER, &[]);
    Ok(vec![])
}

fn control_focus(vm: &mut Vm, c: NativeCall) -> Ret {
    let new = target_arg(vm, &c);
    engine_mut(vm).units.focus = new;
    fire(vm, "PLAYER_FOCUS_CHANGED", &[]);
    Ok(vec![])
}

fn control_mouseover(vm: &mut Vm, c: NativeCall) -> Ret {
    let new = target_arg(vm, &c);
    engine_mut(vm).units.mouseover = new;
    fire(vm, "UPDATE_MOUSEOVER_UNIT", &[]);
    Ok(vec![])
}

fn change_health(vm: &mut Vm, id: UnitId, delta: f64) {
    {
        let u = &mut engine_mut(vm).units.list[id.0];
        u.health = (u.health + delta).clamp(0.0, u.health_max);
        u.dead = u.health <= 0.0;
    }
    fire_unit(vm, "UNIT_HEALTH", id, &[]);
}

/// `unit:setName(name [, lastName])`: a WoW Forever character's first and
/// last name.
fn control_set_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let name = vm.check_str(&c, 2)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let last = vm
        .opt_str(&c, 3)?
        .map(|s| String::from_utf8_lossy(vm.str_bytes(s)).into_owned());
    let u = &mut engine_mut(vm).units.list[me.0];
    u.name = name;
    u.last_name = last;
    Ok(vec![])
}

fn control_damage(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let n = vm.check_number(&c, 2)?;
    change_health(vm, me, -n);
    Ok(vec![])
}

fn control_heal(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let n = vm.check_number(&c, 2)?;
    change_health(vm, me, n);
    Ok(vec![])
}

/// `unit:kill({ by?, lootable?, xp? })`: the unit dies. As the client
/// recorded it, in phases (`kill_blow`, `kill_xp`, `died`): the player's
/// or the group's kill `PARTY_KILL(killer, victim)` and the killing blow's
/// `UNIT_COMBAT(unit, "WOUND", "", amount, 1)`; the player's experience
/// (`xp`: `CHAT_MSG_COMBAT_XP_GAIN`, `PLAYER_XP_UPDATE`); then the death.
fn control_kill(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let opts = Kill::of(vm, c.arg(2))?;
    let amount = engine(vm).units.list[me.0].health;
    let blow = opts.by.map(|by| (by, amount));
    {
        let u = &mut engine_mut(vm).units.list[me.0];
        u.health = 0.0;
        u.dead = true;
    }
    kill_blow(vm, me, blow);
    kill_xp(vm, me, &opts, true);
    kill_xp(vm, me, &opts, false);
    engine_mut(vm).units.list[me.0].loot = opts.loot.clone();
    died(vm, me, opts.lootable);
    Ok(vec![])
}

/// A kill's options: by whom, lootable, the player's experience.
pub(crate) struct Kill {
    pub by: Option<UnitId>,
    pub lootable: bool,
    pub xp: Option<f64>,
    /// The corpse's items: `{ { id, count? }, … }`.
    pub loot: Vec<(i64, f64)>,
}

impl Kill {
    fn of(vm: &mut Vm, v: Value) -> Result<Kill, wowlua_vm::LuaError> {
        let Value::Table(t) = v else {
            return Ok(Kill {
                by: None,
                lootable: false,
                xp: None,
                loot: Vec::new(),
            });
        };
        let field = |vm: &mut Vm, k: &str| {
            let key = vm.str(k);
            vm.heap.table(t).get(key)
        };
        let by_value = field(vm, "by");
        let by = if by_value.is_nil() {
            None
        } else {
            match handle_of(vm, by_value) {
                Some(u) => Some(u),
                None => return Err(vm.error("kill: `by` is a unit")),
            }
        };
        let lootable = field(vm, "lootable").truthy();
        let xp = match field(vm, "xp") {
            Value::Num(n) => Some(n),
            _ => None,
        };
        let mut loot = Vec::new();
        if let Value::Table(list) = field(vm, "loot") {
            let n = vm.heap.table(list).border();
            for i in 1..=n {
                if let Value::Table(item) = vm.heap.table(list).get(Value::Num(i as f64)) {
                    let (k_id, k_count) = (vm.str("id"), vm.str("count"));
                    if let Value::Num(id) = vm.heap.table(item).get(k_id) {
                        let count = match vm.heap.table(item).get(k_count) {
                            Value::Num(c) => c,
                            _ => 1.0,
                        };
                        loot.push((id as i64, count));
                    }
                }
            }
        }
        let lootable = lootable || !loot.is_empty();
        Ok(Kill {
            by,
            lootable,
            xp,
            loot,
        })
    }
}

/// Whether units' identities are restricted (an instance, an encounter, a
/// match): their GUIDs secret, documented `SecretWhenUnitIdentityRestricted`.
fn identity_restricted(vm: &Vm) -> bool {
    active(
        &engine(vm).units,
        &[MAP, ENCOUNTER, CHALLENGE_MODE, PVP_MATCH],
    )
}

/// A unit's GUID as events give it: secret where identities are
/// restricted (Ragefire Chasm's `PARTY_KILL` and `UNIT_DIED`, `/stdtest
/// engine record combat`), plain in the open world (scenario combat).
pub(crate) fn event_guid(vm: &mut Vm, id: UnitId) -> Value {
    let g = engine(vm).units.list[id.0].guid.clone();
    let v = vm.str(&g);
    if identity_restricted(vm) && !ours(&engine(vm).units, id) {
        vm.secret_wrap(v)
    } else {
        v
    }
}

/// The killing blow: `PARTY_KILL(killer, victim)` for the player's or the
/// group's, then its `UNIT_COMBAT`.
pub(crate) fn kill_blow(vm: &mut Vm, me: UnitId, blow: Option<(UnitId, f64)>) {
    let Some((killer, amount)) = blow else { return };
    if ours(&engine(vm).units, killer) {
        let k = event_guid(vm, killer);
        let v = event_guid(vm, me);
        fire(vm, "PARTY_KILL", &[k, v]);
    }
    let wound = vm.str("WOUND");
    let empty = vm.str("");
    fire_unit(
        vm,
        "UNIT_COMBAT",
        me,
        &[wound, empty, Value::Num(amount), Value::Num(1.0)],
    );
}

/// The player's experience for a kill: its message (`first`), else its
/// update.
pub(crate) fn kill_xp(vm: &mut Vm, me: UnitId, opts: &Kill, first: bool) {
    let Some(xp) = opts.xp else { return };
    if first {
        let name = engine(vm).units.list[me.0].name.clone();
        let format = match vm.get_global("COMBATLOG_XPGAIN_FIRSTPERSON") {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => "%s dies, you gain %d experience.".into(),
        };
        let text = format
            .replacen("%s", &name, 1)
            .replacen("%d", &format!("{xp}"), 1);
        crate::chat::game_message(vm, "CHAT_MSG_COMBAT_XP_GAIN", &text, None);
    } else {
        crate::stats::gain_xp(vm, xp);
    }
}

/// The player's target cleared by the world (its death, its corpse
/// emptied): `PLAYER_TARGET_CHANGED()`, and the server's
/// `UNIT_TARGET("player")` a round trip later.
pub(crate) fn clear_target(vm: &mut Vm) {
    engine_mut(vm).units.target = None;
    engine_mut(vm).units.list[PLAYER.0].target = None;
    fire(vm, "PLAYER_TARGET_CHANGED", &[]);
    let echo = vm.native("targetEcho", target_echo);
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, echo);
}

/// The death's events. In the open world (`/stdtest engine scenario
/// combat`): `UNIT_DIED(guid)`; for the player's target
/// `PLAYER_TARGET_DIED()` and the target cleared (`PLAYER_TARGET_CHANGED()`
/// and the server's `UNIT_TARGET("player")` a round trip later); then
/// `UNIT_LOOT(guid, true)` when lootable. Where identities are restricted
/// (Ragefire Chasm, `/stdtest engine record combat`) `UNIT_LOOT` came first,
/// in the same frame; and `UNIT_HEALTH` last. The unit is dead before any.
/// Unverified: what decides the order.
pub(crate) fn died(vm: &mut Vm, me: UnitId, lootable: bool) {
    // A corpse is out of combat and targets no one (recorded: its
    // `targettarget` gone when retargeted).
    {
        let u = &mut engine_mut(vm).units.list[me.0];
        u.in_combat = false;
        u.target = None;
        u.threat = None;
    }
    let instance = identity_restricted(vm);
    let loot = |vm: &mut Vm| {
        if lootable {
            let g = engine(vm).units.list[me.0].guid.clone();
            let g = vm.str(&g);
            fire(vm, "UNIT_LOOT", &[g, Value::Bool(true)]);
        }
    };
    if instance {
        loot(vm);
    }
    let g = event_guid(vm, me);
    fire(vm, "UNIT_DIED", &[g]);
    if engine(vm).units.target == Some(me) {
        fire(vm, "PLAYER_TARGET_DIED", &[]);
        clear_target(vm);
    }
    if !instance {
        loot(vm);
    }
    fire_unit(vm, "UNIT_HEALTH", me, &[]);
}

fn control_set_power(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let n = vm.check_number(&c, 2)?;
    set_power(vm, me, n);
    Ok(vec![])
}

/// The unit's power now `n`: `UNIT_POWER_UPDATE(unit, token)`; the player's
/// actions' usability follows.
fn set_power(vm: &mut Vm, id: UnitId, n: f64) {
    let kind = {
        let u = &mut engine_mut(vm).units.list[id.0];
        u.power = n.clamp(0.0, u.power_max.max(0.0));
        u.power_type
    };
    let token = vm.str(power_token(kind).unwrap_or("MANA"));
    fire_unit(vm, "UNIT_POWER_UPDATE", id, &[token]);
    if id == PLAYER {
        crate::actions::update_usability(vm);
    }
}

/// `player:startAttack()`: auto attack on (`PLAYER_ENTER_COMBAT`).
fn control_start_attack(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    if me == PLAYER && !engine(vm).units.auto_attack {
        engine_mut(vm).units.auto_attack = true;
        fire(vm, "PLAYER_ENTER_COMBAT", &[]);
    }
    Ok(vec![])
}

/// `player:stopAttack()`: auto attack off (`PLAYER_LEAVE_COMBAT`).
fn control_stop_attack(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    if me == PLAYER {
        stop_auto_attack(vm);
    }
    Ok(vec![])
}

/// The player's auto attack off (`PLAYER_LEAVE_COMBAT`), if on.
pub(crate) fn stop_auto_attack(vm: &mut Vm) {
    if engine(vm).units.auto_attack {
        engine_mut(vm).units.auto_attack = false;
        fire(vm, "PLAYER_LEAVE_COMBAT", &[]);
    }
}

/// The player's power now `n`, told (`UNIT_POWER_UPDATE`), the usability
/// left to the caller.
pub(crate) fn set_player_power(vm: &mut Vm, n: f64) {
    set_power_quietly(vm, PLAYER, n);
}

/// `player:raiseSkill(name, rank)`: a skill up, as recorded (scenarios
/// combat and quest: Defense, Swords): "Your skill in … has increased to
/// …" (`CHAT_MSG_SKILL`), the player's `UNIT_ATTACK`, `UNIT_DEFENSE`,
/// `UNIT_RANGED_ATTACK_POWER`, `SKILL_LINES_CHANGED()`,
/// `MOUNT_JOURNAL_USABILITY_CHANGED()`, `PLAYER_DAMAGE_DONE_MODS` (twice
/// for Defense, once for a weapon). Unverified: other skills; the skill
/// lines' answers.
fn control_raise_skill(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    if me != PLAYER {
        return Err(vm.error("raiseSkill: the player's"));
    }
    let name = vm.check_str(&c, 2)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let rank = vm.check_number(&c, 3)?;
    let format = match vm.get_global("ERR_SKILL_UP_SI") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "Your skill in %s has increased to %d.".into(),
    };
    let text = format
        .replacen("%s", &name, 1)
        .replacen("%d", &format!("{rank}"), 1);
    crate::chat::game_message(vm, "CHAT_MSG_SKILL", &text, None);
    for e in ["UNIT_ATTACK", "UNIT_DEFENSE", "UNIT_RANGED_ATTACK_POWER"] {
        fire_unit(vm, e, PLAYER, &[]);
    }
    fire(vm, "SKILL_LINES_CHANGED", &[]);
    fire(vm, "MOUNT_JOURNAL_USABILITY_CHANGED", &[]);
    let times = if name == "Defense" { 2 } else { 1 };
    for _ in 0..times {
        fire_unit(vm, "PLAYER_DAMAGE_DONE_MODS", PLAYER, &[]);
    }
    Ok(vec![])
}

/// The combat text's delay after a hit lands (`/stdtest engine scenario
/// combat`: `UNIT_COMBAT` 0.33 to 0.53 s after the health; unverified:
/// what it waits for).
const COMBAT_TEXT_DELAY: f64 = 0.4;

/// `unit:swing(victim, amount, { kind?, power?, victimPower?, speed? })`: a
/// melee swing landing, as the client recorded the player's and a wolf's
/// (`/stdtest engine scenario combat`): `PLAYER_SWING(speed, 0)` for the
/// player's, combat for both if not yet, the attacker's then the victim's
/// power (`power`, `victimPower`: rage), the victim's health (`amount`;
/// none for a `PARRY`, `DODGE` or `MISS`), and the combat text
/// `UNIT_COMBAT(victim, kind, "", amount, 1)` a moment later.
fn control_swing(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let Some(victim) = handle_of(vm, c.arg(2)) else {
        return Err(vm.error("swing: a unit to hit expected"));
    };
    let amount = vm.check_number(&c, 3)?;
    let opts = match c.arg(4) {
        Value::Table(t) => Some(t),
        _ => None,
    };
    let field = |vm: &mut Vm, k: &str| match opts {
        Some(t) => {
            let key = vm.str(k);
            vm.heap.table(t).get(key)
        }
        None => Value::Nil,
    };
    let kind = match field(vm, "kind") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "WOUND".into(),
    };
    if me == PLAYER && engine(vm).units.casting.next_swing.is_some() {
        let spent = field(vm, "spent");
        let power = field(vm, "power");
        let kill = field(vm, "kill");
        return swing_with_spell(vm, victim, amount, &kind, spent, power, kill);
    }
    if me == PLAYER {
        let speed = match field(vm, "speed") {
            Value::Num(n) => n,
            _ => 1.9,
        };
        // In milliseconds times the float 0.001, as the client gives it
        // (1.9 is 1.9000000953674).
        let speed = (speed * 1000.0).round() as f32 * 0.001f32;
        fire(
            vm,
            "PLAYER_SWING",
            &[Value::Num(f64::from(speed)), Value::Num(0.0)],
        );
    }
    let kill = match field(vm, "kill") {
        k @ Value::Table(_) => Some(Kill::of(vm, k)?),
        _ => None,
    };
    if let Some(k) = kill {
        let power = field(vm, "power");
        return swing_kill(vm, me, victim, k, power);
    }
    for id in [me, victim] {
        if !engine(vm).units.list[id.0].in_combat {
            enter_combat(vm, id, false);
        }
    }
    if let Value::Num(p) = field(vm, "power") {
        set_power(vm, me, p);
    }
    if let Value::Num(p) = field(vm, "victimPower") {
        set_power(vm, victim, p);
    }
    let lands = kind == "WOUND" || kind == "BLOCK" || kind == "ABSORB";
    if lands && amount > 0.0 {
        change_health(vm, victim, -amount);
    }
    if victim == PLAYER && matches!(kind.as_str(), "DODGE" | "PARRY" | "BLOCK") {
        avoided(vm);
    }
    let upvals = vec![
        Value::Num(victim.0 as f64),
        vm.str(&kind),
        Value::Num(if lands { amount } else { 0.0 }),
    ];
    let text = vm.native_with("swingText", swing_text, upvals);
    crate::clock::after_engine(vm, COMBAT_TEXT_DELAY, text);
    Ok(vec![])
}

/// The player dodged, parried or blocked: a spell that procs on it (the
/// warrior's Defensive State, 5302: `procOnAvoid`) tells its cooldown
/// (`ACTIONBAR_UPDATE_COOLDOWN`, `BAG_UPDATE_COOLDOWN`,
/// `SPELL_UPDATE_COOLDOWN(spell)`, `UPDATE_SHAPESHIFT_COOLDOWN`; recorded
/// at each of the quest scenario's).
fn avoided(vm: &mut Vm) {
    let spells: Vec<(i64, wowlua_vm::TableRef)> = engine(vm)
        .world
        .data
        .spells
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    for (id, t) in spells {
        let k = vm.str("procOnAvoid");
        if !vm.heap.table(t).get(k).truthy() {
            continue;
        }
        fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
        fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
        fire(
            vm,
            "SPELL_UPDATE_COOLDOWN",
            &[
                Value::Num(id as f64),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ],
        );
        fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
    }
}

/// An auto attack's killing swing, as the client recorded the player's
/// (`/stdtest engine scenario quest`): after `PLAYER_SWING`, the kill
/// (`PARTY_KILL`; no combat text, the target gone before it), the rage
/// gained, the death (the target cleared), the experience's message, the
/// usability; the experience itself a round trip later.
fn swing_kill(vm: &mut Vm, me: UnitId, victim: UnitId, k: Kill, power: Value) -> Ret {
    {
        let u = &mut engine_mut(vm).units.list[victim.0];
        u.health = 0.0;
        u.dead = true;
    }
    if ours(&engine(vm).units, me) {
        let killer = event_guid(vm, me);
        let v = event_guid(vm, victim);
        fire(vm, "PARTY_KILL", &[killer, v]);
    }
    if let Value::Num(p) = power {
        set_power_quietly(vm, me, p);
    }
    engine_mut(vm).units.list[victim.0].loot = k.loot.clone();
    died(vm, victim, k.lootable);
    kill_xp(vm, victim, &k, true);
    if crate::actions::usability_changed(vm) {
        fire(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    }
    if let Some(xp) = k.xp {
        let later = vm.native_with("killXp", kill_xp_later, vec![Value::Num(xp)]);
        let delay = crate::clock::server_delay(vm);
        crate::clock::after_engine(vm, delay, later);
    }
    Ok(vec![])
}

fn kill_xp_later(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Value::Num(xp) = c.upvals[0] {
        crate::stats::gain_xp(vm, xp);
    }
    Ok(vec![])
}

/// The player's swing landing the spell queued for it (Heroic Strike), as
/// recorded (`/stdtest engine scenario combat`): the rage spent (`spent`),
/// `SUCCEEDED`, a `UNIT_COMBAT` of 0, `PLAYER_SWING`, (a kill's
/// `PARTY_KILL`,) the hit's `UNIT_COMBAT` at once, (its experience's
/// message,) the rage gained (`power`), (the experience, the death,) the
/// usability, the cooldowns and the victim's health.
fn swing_with_spell(
    vm: &mut Vm,
    victim: UnitId,
    amount: f64,
    kind: &str,
    spent: Value,
    power: Value,
    kill: Value,
) -> Ret {
    let kill = match kill {
        Value::Table(_) => Some(Kill::of(vm, kill)?),
        _ => None,
    };
    if let Value::Num(p) = spent {
        set_power_quietly(vm, PLAYER, p);
    }
    let Some(spell) = crate::casting::land_next_swing(vm) else {
        return Ok(vec![]);
    };
    let wound = vm.str("WOUND");
    let empty = vm.str("");
    fire_unit(
        vm,
        "UNIT_COMBAT",
        victim,
        &[wound, empty, Value::Num(0.0), Value::Num(1.0)],
    );
    fire(
        vm,
        "PLAYER_SWING",
        &[Value::Num(f64::from(1900.0f32 * 0.001f32)), Value::Num(0.0)],
    );
    let lands = kind == "WOUND" || kind == "BLOCK" || kind == "ABSORB";
    let dealt = if lands { amount } else { 0.0 };
    {
        let u = &mut engine_mut(vm).units.list[victim.0];
        u.health = (u.health - dealt).max(0.0);
        if kill.is_some() {
            u.health = 0.0;
            u.dead = true;
        }
    }
    if kill.is_some() {
        kill_blow(vm, victim, Some((PLAYER, dealt)));
    } else {
        let k = vm.str(kind);
        fire_unit(
            vm,
            "UNIT_COMBAT",
            victim,
            &[k, empty, Value::Num(dealt), Value::Num(1.0)],
        );
    }
    if let Some(k) = &kill {
        kill_xp(vm, victim, k, true);
    }
    if let Value::Num(p) = power {
        set_power_quietly(vm, PLAYER, p);
    }
    if let Some(k) = &kill {
        kill_xp(vm, victim, k, false);
        engine_mut(vm).units.list[victim.0].loot = k.loot.clone();
        died(vm, victim, k.lootable);
    }
    let changed = crate::actions::usability_changed(vm);
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    if changed {
        fire(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    }
    fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
    fire(
        vm,
        "SPELL_UPDATE_COOLDOWN",
        &[
            Value::Num(spell),
            Value::Nil,
            Value::Num(0.0),
            Value::Num(0.0),
            Value::Nil,
        ],
    );
    if kill.is_none() {
        fire_unit(vm, "UNIT_HEALTH", victim, &[]);
    }
    fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
    Ok(vec![])
}

/// The unit's power now `n`, told (`UNIT_POWER_UPDATE`) without the
/// actions' usability (told later by the caller).
fn set_power_quietly(vm: &mut Vm, id: UnitId, n: f64) {
    let kind = {
        let u = &mut engine_mut(vm).units.list[id.0];
        u.power = n.clamp(0.0, u.power_max.max(0.0));
        u.power_type
    };
    let token = vm.str(power_token(kind).unwrap_or("MANA"));
    fire_unit(vm, "UNIT_POWER_UPDATE", id, &[token]);
}

/// A swing's combat text: `UNIT_COMBAT(unit, kind, "", amount, 1)`.
fn swing_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(id), kind, Value::Num(amount)) = (c.upvals[0], c.upvals[1], c.upvals[2]) else {
        return Ok(vec![]);
    };
    let empty = vm.str("");
    fire_unit(
        vm,
        "UNIT_COMBAT",
        UnitId(id as usize),
        &[kind, empty, Value::Num(amount), Value::Num(1.0)],
    );
    Ok(vec![])
}

/// Sets a restriction's state, firing `ADDON_RESTRICTION_STATE_CHANGED`
/// with `Activating` before it becomes active (unverified order).
pub fn set_restriction(vm: &mut Vm, t: usize, on: bool) {
    let was = engine(vm).units.restrictions[t];
    if on && was != 2 {
        engine_mut(vm).units.restrictions[t] = 1;
        fire(
            vm,
            "ADDON_RESTRICTION_STATE_CHANGED",
            &[Value::Num(t as f64), Value::Num(1.0)],
        );
        engine_mut(vm).units.restrictions[t] = 2;
    } else if !on && was != 0 {
        engine_mut(vm).units.restrictions[t] = 0;
        fire(
            vm,
            "ADDON_RESTRICTION_STATE_CHANGED",
            &[Value::Num(t as f64), Value::Num(0.0)],
        );
    }
}

fn control_enter_combat(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    enter_combat(vm, me, true);
    Ok(vec![])
}

/// The unit in combat. The player's, as recorded (`/stdtest engine record
/// combat`): REGEN_DISABLED, the restriction activating,
/// PLAYER_IN_COMBAT_CHANGED, the player's flags; the lockdown after; the
/// actions' usability (`usability`: now, else after the power that came
/// with it).
fn enter_combat(vm: &mut Vm, me: UnitId, usability: bool) {
    engine_mut(vm).units.list[me.0].in_combat = true;
    if me == PLAYER {
        fire(vm, "PLAYER_REGEN_DISABLED", &[]);
        set_restriction(vm, COMBAT, true);
        fire(vm, "PLAYER_IN_COMBAT_CHANGED", &[Value::Bool(true)]);
        fire_unit(vm, "UNIT_FLAGS", PLAYER, &[]);
        engine_mut(vm).units.lockdown = true;
        if usability {
            crate::actions::update_usability(vm);
        }
    }
}

fn control_leave_combat(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    engine_mut(vm).units.list[me.0].in_combat = false;
    if me == PLAYER && engine(vm).units.threat_situation {
        // Off the threat lists first (recorded).
        engine_mut(vm).units.threat_situation = false;
        for u in engine_mut(vm).units.list.iter_mut() {
            u.threat = None;
        }
        fire_unit(vm, "UNIT_THREAT_SITUATION_UPDATE", PLAYER, &[]);
    }
    if me == PLAYER {
        player_out_of_combat(vm);
        crate::actions::update_usability(vm);
    }
    Ok(vec![])
}

/// The player out of combat: `PLAYER_IN_COMBAT_CHANGED(false)`, the
/// restriction ending, `PLAYER_REGEN_ENABLED()`, the player's flags.
pub(crate) fn player_out_of_combat(vm: &mut Vm) {
    engine_mut(vm).units.list[PLAYER.0].in_combat = false;
    engine_mut(vm).units.lockdown = false;
    fire(vm, "PLAYER_IN_COMBAT_CHANGED", &[Value::Bool(false)]);
    set_restriction(vm, COMBAT, false);
    fire(vm, "PLAYER_REGEN_ENABLED", &[]);
    fire_unit(vm, "UNIT_FLAGS", PLAYER, &[]);
}

fn control_show_nameplate(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let slot = {
        let u = &mut engine_mut(vm).units;
        if let Some(i) = u.nameplates.iter().position(|n| *n == Some(me)) {
            return Ok(vec![Value::Num(i as f64 + 1.0)]);
        }
        match u.nameplates.iter().position(Option::is_none) {
            Some(i) => {
                u.nameplates[i] = Some(me);
                i
            }
            None => {
                u.nameplates.push(Some(me));
                u.nameplates.len() - 1
            }
        }
    };
    let token = vm.str(&format!("nameplate{}", slot + 1));
    fire(vm, "NAME_PLATE_UNIT_ADDED", &[token]);
    Ok(vec![Value::Num(slot as f64 + 1.0)])
}

fn control_hide_nameplate(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let slot = engine(vm)
        .units
        .nameplates
        .iter()
        .position(|n| *n == Some(me));
    if let Some(i) = slot {
        let token = vm.str(&format!("nameplate{}", i + 1));
        fire(vm, "NAME_PLATE_UNIT_REMOVED", &[token]);
        engine_mut(vm).units.nameplates[i] = None;
    }
    Ok(vec![])
}

fn control_despawn(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    control_hide_nameplate(vm, c)?;
    let was_target = engine(vm).units.target == Some(me);
    {
        let u = &mut engine_mut(vm).units;
        u.list[me.0].alive_in_world = false;
        u.auras.retain(|a| a.unit != me);
        if u.focus == Some(me) {
            u.focus = None;
        }
        if u.mouseover == Some(me) {
            u.mouseover = None;
        }
    }
    if was_target {
        engine_mut(vm).units.target = None;
        fire(vm, "PLAYER_TARGET_CHANGED", &[]);
    }
    Ok(vec![])
}

fn control_tokens(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let tokens = tokens_of(&engine(vm).units, me);
    Ok(tokens.first().map_or(vec![Value::Nil], |t| vec![vm.str(t)]))
}

/// `mock.addToGroup(unit, raid?)`: the unit joins the player's group.
pub fn add_to_group(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = handle_of(vm, c.arg(1)) else {
        return Err(vm.error("mock.addToGroup: a unit expected"));
    };
    let raid = c.arg(2).truthy();
    {
        let u = &mut engine_mut(vm).units;
        if !u.group.contains(&id) {
            u.group.push(id);
        }
        u.raid = u.raid || raid;
    }
    fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    Ok(vec![])
}

/// `mock.enterInstance()` / `mock.leaveInstance()`: a restricted map.
pub fn enter_instance(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_restriction(vm, MAP, true);
    Ok(vec![])
}

pub fn leave_instance(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_restriction(vm, MAP, false);
    Ok(vec![])
}

/// `mock.setRestriction(type, active)` for the other restriction types.
pub fn set_restriction_control(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_int(&c, 1)?;
    let t = usize::try_from(t)
        .ok()
        .filter(|&t| t < 6)
        .ok_or_else(|| vm.error("mock.setRestriction: a restriction type (0-5) expected"))?;
    set_restriction(vm, t, c.arg(2).truthy());
    Ok(vec![])
}

// ------------------------------------------------- percents and curves

/// `UnitHealthPercent(unit [, usePredicted, curve])`: the health as a
/// fraction, or a curve's value for it (unverified: the fraction's scale
/// is 0 to 1 without a curve, as the documentation's "floating point
/// percentage value" and Blizzard's ScaleTo100 curve suggest).
fn unit_health_percent(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = unit_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let u = &engine(vm).units.list[id.0];
    let pct = if u.health_max > 0.0 {
        u.health / u.health_max
    } else {
        0.0
    };
    Ok(vec![crate::curves::apply(vm, c.arg(3), pct)?])
}

fn unit_health_missing(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |_, u| vec![Value::Num(u.health_max - u.health)])
}

fn unit_power_percent(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = unit_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let u = &engine(vm).units.list[id.0];
    let pct = if u.power_max > 0.0 {
        u.power / u.power_max
    } else {
        0.0
    };
    Ok(vec![crate::curves::apply(vm, c.arg(4), pct)?])
}

/// More of this module's behaviours.
/// The faction of a race (`UnitFactionGroup`); a race neither lists is
/// neutral (unverified for races the list lacks).
fn faction(race: &str) -> Option<&'static str> {
    const ALLIANCE: &[&str] = &[
        "Human",
        "Dwarf",
        "NightElf",
        "Gnome",
        "Draenei",
        "Worgen",
        "VoidElf",
        "LightforgedDraenei",
        "DarkIronDwarf",
        "KulTiran",
        "Mechagnome",
    ];
    const HORDE: &[&str] = &[
        "Orc",
        "Scourge",
        "Tauren",
        "Troll",
        "BloodElf",
        "Goblin",
        "Nightborne",
        "HighmountainTauren",
        "MagharOrc",
        "ZandalariTroll",
        "Vulpera",
    ];
    if ALLIANCE.contains(&race) {
        Some("Alliance")
    } else if HORDE.contains(&race) {
        Some("Horde")
    } else {
        None
    }
}

/// `UnitFactionGroup`: a race's faction; a creature its own (a city's
/// NPC: "Alliance", recorded) or none; no such unit two empty strings
/// (recorded, scenario combat).
fn unit_faction_group(vm: &mut Vm, c: NativeCall) -> Ret {
    if unit_arg(vm, &c, 1).is_none() {
        return Ok(vec![vm.str(""), vm.str("")]);
    }
    with_unit(vm, &c, |vm, u| {
        let f = u
            .faction_group
            .clone()
            .or_else(|| faction(&race_file_and_id(&u.race).0).map(str::to_string));
        match f {
            Some(f) => vec![vm.str(&f), vm.str(&f)],
            None => vec![],
        }
    })
}

fn unit_pvp_name(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| vec![vm.str(&u.name)])
}

fn unit_is_ghost(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |_, u| vec![Value::Bool(u.ghost)])
}

fn unit_is_dead_or_ghost(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| {
        vec![Value::Bool(u.dead || u.ghost)]
    })
}

/// Not casting or channeling: no values.
/// The roles a unit can take (tank, healer, damage): damage (unverified
/// per class).
fn available_roles(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(true),
    ])
}

/// `ClearFocus()`: no focus (`PLAYER_FOCUS_CHANGED` if there was one).
fn clear_focus(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine_mut(vm).units.focus.take().is_some() {
        fire(vm, "PLAYER_FOCUS_CHANGED", &[]);
    }
    Ok(vec![])
}

/// Nothing to do in the world (no auto attack, no target history; the
/// client returned nothing for addon code).
fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `UnitFullName(unit)`: the name and, on WoW Forever, the last name
/// (`/stdtest engine comm`), else the realm.
fn unit_full_name(vm: &mut Vm, c: NativeCall) -> Ret {
    unit_name(vm, c)
}

pub fn more_behaviours() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        ("GetClassInfo", get_class_info),
        ("C_CreatureInfo.GetClassInfo", creature_class_info),
        ("C_SpecializationInfo.GetAllClassIDs", all_class_ids),
        ("UnitFullName", unit_full_name),
        ("ClearFocus", clear_focus),
        ("TargetLastTarget", nothing),
        ("StopAttack", nothing),
        ("UnitFactionGroup", unit_faction_group),
        ("UnitPVPName", unit_pvp_name),
        ("UnitIsGhost", unit_is_ghost),
        ("UnitGetAvailableRoles", available_roles),
        ("UnitHealthPercent", unit_health_percent),
        ("UnitHealthMissing", unit_health_missing),
        ("UnitPowerPercent", unit_power_percent),
    ]
}
