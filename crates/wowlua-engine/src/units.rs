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
    /// The player's side's reaction toward it (`UnitReaction(player,
    /// unit)`), when not its own: a faction NPC's by the player's standing
    /// (recorded: an Orgrimmar auctioneer neutral to the player, the player
    /// friendly to it).
    pub player_reaction: Option<Reaction>,
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
    /// Dead with the player on its threat list, the list's update not told
    /// yet (told as the player leaves combat, or by `setThreat(nil)`).
    pub listed_dead: bool,
    /// What its corpse holds (`kill({ loot })`): items and counts.
    pub loot: Vec<(i64, f64)>,
    /// A creature's type and family: their names and IDs
    /// (`UnitCreatureType`, `UnitCreatureFamily`; none for players).
    pub creature_type: Option<(String, f64)>,
    pub creature_family: Option<(String, f64)>,
    /// The group role chosen (`TANK`, `HEALER`, `DAMAGER`;
    /// `UnitGroupRolesAssigned`, the roster's combat role).
    pub role: Option<String>,
    /// Where a group member is when not where the player is (the roster's
    /// zone; out of sight).
    pub zone: Option<String>,
    /// A raid member's subgroup when not the one its place gives.
    pub subgroup: Option<f64>,
    /// A player character logged in (an offline group member not).
    pub online: bool,
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
    /// A creature turned on the player this fight (its flags told then).
    pub engaged: bool,
    /// The next target change's echo (`UNIT_TARGET("player")`): after
    /// these seconds, or none (`echo` of `target` or of a kill).
    pub echo: Option<Option<f64>>,
    pub list: Vec<Unit>,
    pub target: Option<UnitId>,
    pub focus: Option<UnitId>,
    pub mouseover: Option<UnitId>,
    /// The soft targets: the enemy, the friend, the interact unit.
    pub soft: [Option<UnitId>; 3],
    /// The NPC the player talks to (`npc`: a vendor).
    pub npc: Option<UnitId>,
    pub nameplates: Vec<Option<UnitId>>,
    /// Each nameplate slot's frame, made the first time the slot is used.
    pub nameplate_frames: Vec<widgets::WidgetId>,
    /// The group without the player, in order; a raid when `raid`.
    pub group: Vec<UnitId>,
    pub raid: bool,
    pub bosses: Vec<Option<UnitId>>,
    /// The player's pet (`pets.rs`).
    pub pet: Option<UnitId>,
    /// Each restriction type's state: 0 inactive, 1 activating, 2 active.
    pub restrictions: [u8; 6],
    pub lockdown: bool,
    /// Auras on the units, in application order, and the last instance ID.
    pub auras: Vec<crate::auras::Aura>,
    /// The player eating: the food's aura instance and the item.
    pub eating: Option<(f64, i64)>,
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

/// A class's name by its file (`WARRIOR`: `Warrior`).
pub(crate) fn class_name_of(file: &str) -> Option<&'static str> {
    CLASS_INFO.iter().find(|c| c.1 == file).map(|c| c.2)
}

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
    // Truncated (the full probe, 1.60.1: 1.5 named the warriors).
    let id = match vm.hidden(c.arg(1)) {
        Value::Num(n) => n.trunc(),
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
        player_reaction: None,
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
        listed_dead: false,
        loot: Vec::new(),
        creature_type: None,
        creature_family: None,
        role: None,
        zone: None,
        subgroup: None,
        online: true,
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
        ("pet", u.pet),
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
            // raid1 is the player, raid2.. the others (recorded).
            if n == 1 {
                Some(PLAYER)
            } else {
                u.group.get(n.wrapping_sub(2)).copied()
            }
        } else {
            None
        }
    };
    // The party: in a raid, the others of the player's subgroup
    // (recorded: a member there party1, one moved away none).
    let party = |n: usize| party_members(u).get(n.wrapping_sub(1)).copied();
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

/// The members `partyN` names: the group's others, in a raid those in the
/// player's subgroup.
fn party_members(u: &Units) -> Vec<UnitId> {
    if !u.raid {
        return u.group.clone();
    }
    let mine = subgroup(u, PLAYER);
    u.group
        .iter()
        .copied()
        .filter(|g| subgroup(u, *g) == mine)
        .collect()
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
    // The pet's (unverified: its place in the order).
    if u.pet == Some(id) {
        out.push("pet".to_string());
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
        if *g == id && u.raid {
            out.push(format!("raid{}", i + 2));
        }
    }
    // In a raid too, the player's subgroup's (unverified: after raidN).
    if let Some(i) = party_members(u).iter().position(|g| *g == id) {
        out.push(format!("party{}", i + 1));
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
pub(crate) fn ours(u: &Units, id: UnitId) -> bool {
    id == PLAYER || u.group.contains(&id)
}

/// Whether a documented function's results are secret for this call, from
/// its flags and the world (recorded on WoW Forever; see the module).
pub fn secret_results(vm: &Vm, doc: &serde_json::Value, c: &NativeCall) -> bool {
    secret_results_flagged(vm, secret_flags(doc), c)
}

/// The documentation's flags [`secret_results`] reads.
const SECRET_FLAGS: [&str; 13] = [
    "SecretReturns",
    "SecretWhenUnitAuraRestricted",
    "SecretWhenAurasRestricted",
    "SecretWhenCooldownsRestricted",
    "SecretWhenTotemSlotSecret",
    "SecretWhenInCombat",
    "SecretOnRestrictedMaps",
    "SecretWhenUnitHealthMaxRestricted",
    "SecretWhenUnitSpellCastRestricted",
    "SecretWhenUnitPowerMaxRestricted",
    "SecretWhenUnitIdentityRestricted",
    "SecretWhenUnitNameIdentityRestricted",
    "SecretWhenUnitPowerRestricted",
];

/// Which of [`SECRET_FLAGS`] a function's documentation sets, as bits.
pub fn secret_flags(doc: &serde_json::Value) -> u32 {
    SECRET_FLAGS
        .iter()
        .enumerate()
        .filter(|(_, k)| doc.get(**k).and_then(serde_json::Value::as_bool) == Some(true))
        .fold(0, |bits, (i, _)| bits | 1 << i)
}

/// [`secret_results`] by the documentation's flags (`secret_flags`).
pub fn secret_results_flagged(vm: &Vm, flags: u32, c: &NativeCall) -> bool {
    // Every answer of secrecy below needs a flag.
    if flags == 0 {
        return false;
    }
    let flag = |k: &str| {
        SECRET_FLAGS
            .iter()
            .position(|f| *f == k)
            .is_some_and(|i| flags & 1 << i != 0)
    };
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
    // (recorded, scenario flight), a party member's too (scenario group,
    // targeted); the player's not.
    if (flag("SecretWhenUnitHealthMaxRestricted") || flag("SecretWhenUnitPowerMaxRestricted"))
        && id != PLAYER
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
    // A unit's power is secret, the player's too, in combat or out, any
    // power (every recording: 188 of the player's answers, rage, mana;
    // a wolf's, other players', a party member's).
    if flag("SecretWhenUnitPowerRestricted") {
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
        role: u.role.clone(),
        zone: u.zone.clone(),
        ..*u
    }
}

fn unit_exists(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(unit_arg(vm, &c, 1).is_some())])
}

/// `UnitGUID(unit)`: the GUID; a token naming no one one nil (recorded:
/// a target gone, in every recording that counts trailing nils).
fn unit_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    if is_none_token(vm, c.arg(1)) {
        return Ok(vec![vm.str("Uniq-WOWGUID_SCRIPT_TARGET_NONE")]);
    }
    if unit_arg(vm, &c, 1).is_none() {
        return Ok(vec![Value::Nil]);
    }
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

/// `UnitName(unit)`: the name and the last name (or realm); a token naming
/// no one two nils (recorded: absent group tokens).
fn unit_name(vm: &mut Vm, c: NativeCall) -> Ret {
    // "none" has a name (the full probe, blanked: a string and nil;
    // unverified which, here `UNKNOWNOBJECT`'s).
    if is_none_token(vm, c.arg(1)) {
        let unknown = match vm.get_global("UNKNOWNOBJECT") {
            v @ Value::Str(_) => v,
            _ => vm.str("Unknown"),
        };
        return Ok(vec![unknown, Value::Nil]);
    }
    if unit_arg(vm, &c, 1).is_none() {
        return Ok(vec![Value::Nil, Value::Nil]);
    }
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

/// `UnitClassBase`: the class's file name and ID (recorded: "PALADIN", 2).
fn unit_class_base(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| {
        vec![vm.str(&u.class), Value::Num(u.class_id)]
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

/// A race's file name and ID (the client's races; the Undead's and the
/// Skyborne's recorded, the others unverified).
pub(crate) fn race_file_and_id(race: &str) -> (String, Option<f64>) {
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
        // A druid's (recorded, scenario group).
        ("Windshaper Skyborne", "Skyborne", 96.0),
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
pub(crate) fn race_name(race: &str) -> String {
    const NAMES: &[(&str, &str)] = &[
        ("Scourge", "Undead"),
        ("NightElf", "Night Elf"),
        ("BloodElf", "Blood Elf"),
        ("Skyborne", "Windshaper Skyborne"),
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

/// `UnitCreatureType(unit)`: a creature's own; a player character a
/// humanoid (the full probe: the Undead player "Humanoid", 7).
fn unit_creature_type(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| match &u.creature_type {
        Some((name, id)) => vec![vm.str(name), Value::Num(*id)],
        None if u.player => vec![vm.str("Humanoid"), Value::Num(7.0)],
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

/// `UnitHasPowerType(unit, powerType)`: the unit's power is of that type
/// (unverified: a unit's other powers, a druid's mana in a form).
fn unit_has_power_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let wanted = match vm.hidden(c.arg(2)) {
        Value::Num(n) => n,
        _ => return Ok(vec![Value::Bool(false)]),
    };
    with_unit_or(vm, &c, Value::Bool(false), |_, u| {
        vec![Value::Bool(u.power_type == wanted)]
    })
}

/// `UnitIsConnected`: every unit there is.
/// `UnitIsConnected(unit)`: logged in (recorded: an offline party member
/// not).
fn unit_is_connected(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| {
        vec![Value::Bool(u.online)]
    })
}

/// `UnitShouldDisplayName`: others' names, not the player's (recorded).
fn unit_should_display_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_arg(vm, &c, 1);
    Ok(vec![Value::Bool(id.is_some_and(|id| id != PLAYER))])
}

/// `UnitSelectionColor(unit [, useExtendedColors])`: a creature's by its
/// reaction (recorded: a neutral wolf's yellow); the player's as recorded.
fn unit_selection_color(vm: &mut Vm, c: NativeCall) -> Ret {
    // No such unit: blue (recorded, a target cleared), the extended
    // colors' blue as the player's (the full probe).
    let Some(id) = unit_arg(vm, &c, 1) else {
        if c.arg(2).truthy() {
            return Ok(
                crate::recorded::lookup_text(vm, "UnitSelectionColor(\"player\", true)")
                    .unwrap_or_else(|| [0.0, 0.0, 1.0, 1.0].map(Value::Num).to_vec()),
            );
        }
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

/// `UnitPlayerControlled`: a player (recorded) or the player's pet
/// (unverified).
fn unit_player_controlled(vm: &mut Vm, c: NativeCall) -> Ret {
    let pet = unit_arg(vm, &c, 1).is_some_and(|id| engine(vm).units.pet == Some(id));
    with_unit_or(vm, &c, Value::Bool(false), |_, u| {
        vec![Value::Bool(u.player || pet)]
    })
}

/// `UnitIsPVP`: flagged for PvP (another player's, recorded).
fn unit_is_pvp(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| vec![Value::Bool(u.pvp)])
}

/// `UnitSex(unit)`: no such unit nil (the full probe).
fn unit_sex(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Nil, |_, u| vec![Value::Num(u.sex)])
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
    if other_power_type(vm, &c) {
        return Ok(vec![Value::Num(0.0)]);
    }
    with_unit_or(vm, &c, Value::Num(0.0), |_, u| vec![Value::Num(u.power)])
}

fn unit_power_max(vm: &mut Vm, c: NativeCall) -> Ret {
    if other_power_type(vm, &c) {
        return Ok(vec![Value::Num(0.0)]);
    }
    with_unit_or(vm, &c, Value::Num(0.0), |_, u| {
        vec![Value::Num(u.power_max)]
    })
}

/// Whether a power call (`UnitPower(unit [, powerType])`) asks for a power
/// type the unit does not have: it has none of it, 0, and nothing to hide
/// (the full probe: the paladin's `UnitPower("player", 1..29)` a plain 0,
/// its mana and a power type not a number secret; unverified for a unit
/// with a second power, a rogue's combo points).
pub(crate) fn other_power_type(vm: &mut Vm, c: &NativeCall) -> bool {
    let Value::Num(asked) = vm.hidden(c.arg(2)) else {
        return false;
    };
    let other =
        unit_arg(vm, c, 1).is_some_and(|id| engine(vm).units.list[id.0].power_type != asked);
    if other {
        engine_mut(vm).plain_results = true;
    }
    other
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

/// `UnitPowerType(unit [, index])`: the power type, its token and its
/// alternate colors (none: nils); an index but the first (0) nothing (the
/// full probe: the player's 0, "MANA", nil, nil, nil; with index 1, -1,
/// 1.5 nothing; every recording shows the three nils).
fn unit_power_type(vm: &mut Vm, c: NativeCall) -> Ret {
    if !matches!(vm.hidden(c.arg(2)), Value::Nil | Value::Num(0.0)) {
        return Ok(vec![]);
    }
    let t = with_unit(vm, &c, |_, u| vec![Value::Num(u.power_type)])?;
    let Some(Value::Num(n)) = t.first().copied() else {
        return Ok(t);
    };
    Ok(match power_token(n) {
        Some(token) => vec![
            Value::Num(n),
            vm.str(token),
            Value::Nil,
            Value::Nil,
            Value::Nil,
        ],
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
    // "none" is the script's no-target, with a GUID of its own: itself
    // (the full probe: `UnitIsUnit("none", "none")` true, with any other
    // false).
    let none = is_none_token(vm, c.arg(1)) && is_none_token(vm, c.arg(2));
    Ok(vec![Value::Bool((a.is_some() && a == b) || none)])
}

/// Whether a value is the unit token "none" (the scripts' no-target: no
/// unit, yet a GUID and a name, the full probe).
pub(crate) fn is_none_token(vm: &Vm, v: Value) -> bool {
    match vm.hidden(v) {
        Value::Str(s) => vm.str_bytes(s).eq_ignore_ascii_case(b"none"),
        _ => false,
    }
}

/// The reaction between `a` and `b`, the same both ways (1 hated .. 8
/// exalted; hostile 2, neutral 4, friendly 5: a neutral wolf's is 4 toward
/// the player and the player's toward it, recorded).
fn unit_reaction(vm: &mut Vm, c: NativeCall) -> Ret {
    // Either unit missing: nil (the full probe, every pair with "target",
    // "pet", "party1" or "none" absent, or a GUID; the newest answer, over
    // nothing).
    let Some(reaction) = reaction_of(vm, &c) else {
        return Ok(vec![Value::Nil]);
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
    // Between the player's side and a unit: the player's toward it, else
    // that unit's reaction.
    Some(if ours(u, a) {
        let b = &u.list[b.0];
        b.player_reaction.unwrap_or(b.reaction)
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
    Ok(vec![Value::Bool(crate::party::grouped(vm))])
}

fn is_in_raid(vm: &mut Vm, _c: NativeCall) -> Ret {
    let u = &engine(vm).units;
    Ok(vec![Value::Bool(u.raid && !u.group.is_empty())])
}

fn get_num_group_members(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = engine(vm).units.group.len();
    let grouped = crate::party::grouped(vm);
    Ok(vec![Value::Num(if grouped { n as f64 + 1.0 } else { 0.0 })])
}

/// `GetNumSubgroupMembers()`: the others in the player's party; in a raid,
/// in the player's subgroup (recorded: none, the other in subgroup 4).
fn get_num_subgroup_members(vm: &mut Vm, _c: NativeCall) -> Ret {
    let u = &engine(vm).units;
    let n = if u.raid {
        let mine = subgroup(u, PLAYER);
        u.group
            .iter()
            .filter(|&&id| subgroup(u, id) == mine)
            .count()
    } else {
        u.group.len()
    };
    Ok(vec![Value::Num(n as f64)])
}

/// `UnitPlayerOrPetInParty(unit)`: a group member, a raid's too (recorded;
/// pets unverified).
fn unit_player_or_pet_in_party(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_arg(vm, &c, 1);
    let grouped = crate::party::grouped(vm);
    let u = &engine(vm).units;
    Ok(vec![Value::Bool(
        grouped && id.is_some_and(|id| ours(u, id)),
    )])
}

/// `UnitPlayerOrPetInRaid(unit)`: a raid's member, none in a party
/// (recorded; pets unverified).
fn unit_player_or_pet_in_raid(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_arg(vm, &c, 1);
    let grouped = crate::party::grouped(vm);
    let u = &engine(vm).units;
    Ok(vec![Value::Bool(
        grouped && u.raid && id.is_some_and(|id| ours(u, id)),
    )])
}

/// A group member's raid subgroup: its own (moved: the recording's druid,
/// moved to subgroup 4 by hand), else five to a subgroup in raid order,
/// the player first (unverified beyond a party made a raid).
pub fn subgroup(u: &Units, id: UnitId) -> f64 {
    if let Some(n) = u.list[id.0].subgroup {
        return n;
    }
    let index = if id == PLAYER {
        0
    } else {
        u.group.iter().position(|&g| g == id).map_or(0, |i| i + 1)
    };
    (index / 5 + 1) as f64
}

/// `UnitGroupRolesAssigned(unit)`: a group member's chosen role, else
/// `NONE` (recorded: the player's and a member's; absent tokens `NONE`).
fn unit_group_roles_assigned(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_arg(vm, &c, 1);
    let role = {
        let u = &engine(vm).units;
        id.filter(|&id| crate::party::grouped(vm) && ours(u, id))
            .and_then(|id| u.list[id.0].role.clone())
            .unwrap_or_else(|| "NONE".to_string())
    };
    Ok(vec![vm.str(&role)])
}

/// `UnitIsVisible(unit)`: a unit in the player's zone (recorded: a group
/// member in another zone not; unverified: by distance).
fn unit_is_visible(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_arg(vm, &c, 1);
    let e = engine(vm);
    let here = &e.world.character.position.zone;
    let visible = id.is_some_and(|id| {
        let u = &e.units.list[id.0];
        u.online && u.zone.as_ref().is_none_or(|z| z == here)
    });
    Ok(vec![Value::Bool(visible)])
}

/// `unit:setOnline(online)`: a group member logged in or out
/// (`GROUP_ROSTER_UPDATE`; unverified: the client's other events).
fn control_set_online(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let online = c.arg(2).truthy();
    engine_mut(vm).units.list[me.0].online = online;
    if crate::party::grouped(vm) {
        fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    }
    Ok(vec![])
}

/// `unit:setRole(role?)`: the group role it chose (`TANK`, `HEALER`,
/// `DAMAGER`; nil none).
fn control_set_role(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let role = match c.arg(2) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    };
    engine_mut(vm).units.list[me.0].role = role;
    if crate::party::grouped(vm) {
        fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    }
    Ok(vec![])
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

/// `C_Secrets.GetPowerTypeSecrecy(powerType)`: contextually secret (2),
/// combo points (4) never (0) (the full probe, 1.60.1: every type from 0
/// to 29).
fn power_type_secrecy(vm: &mut Vm, c: NativeCall) -> Ret {
    let never = matches!(vm.hidden(c.arg(1)), Value::Num(n) if n.trunc() == 4.0);
    Ok(vec![Value::Num(if never { 0.0 } else { 2.0 })])
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

/// `C_Secrets.ShouldUnitPowerBeSecret(unit [, powerType])`: as the unit's
/// power, but a power type the unit does not have is never secret (the
/// full probe, 1.60.1: the player's mana, the default and `"x"` alike,
/// secret, every other type of the player's not; unverified: a unit with
/// several power types).
fn should_power(vm: &mut Vm, c: NativeCall) -> Ret {
    if let (Some(id), Value::Num(wanted)) = (unit_arg(vm, &c, 1), vm.hidden(c.arg(2)))
        && engine(vm).units.list[id.0].power_type != wanted.trunc()
    {
        return Ok(vec![Value::Bool(false)]);
    }
    should_be_secret("SecretWhenUnitPowerRestricted")(vm, c)
}
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
        // As `UnitName` (recorded alike for the player: its first and last
        // name; unverified for a renamed unit, a vehicle's).
        ("UnitNameUnmodified", unit_name),
        ("UnitLevel", unit_level),
        ("UnitClass", unit_class),
        ("UnitClassBase", unit_class_base),
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
        // No battle pet companions; nobody feigns death; no cast has a
        // target the engine knows (the documentation: false then).
        ("UnitIsBattlePetCompanion", unit_not_special),
        ("UnitIsFeignDeath", unit_not_special),
        ("UnitShouldDisplaySpellTargetName", unit_not_special),
        ("UnitHasPowerType", unit_has_power_type),
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
        ("UnitCanCooperate", unit_can_cooperate),
        ("UnitIsHumanPlayer", unit_is_human_player),
        ("UnitAffectingCombat", unit_affecting_combat),
        ("UnitIsUnit", unit_is_unit),
        ("UnitReaction", unit_reaction),
        ("UnitIsEnemy", unit_is_enemy),
        ("UnitIsFriend", unit_is_friend),
        ("UnitCanAttack", unit_can_attack),
        ("UnitCanAssist", unit_can_assist),
        ("UnitInParty", unit_in_party),
        ("UnitInRaid", unit_in_raid),
        ("C_NamePlate.GetNamePlateForUnit", get_name_plate_for_unit),
        ("C_NamePlate.GetNamePlates", get_name_plates),
        ("IsInGroup", is_in_group),
        ("TargetUnit", target_unit),
        (
            "C_ClickBindings.GetEffectiveInteractionButton",
            effective_interaction_button,
        ),
        ("C_ClickBindings.GetBindingType", click_binding_type),
        ("ClearTarget", clear_target_call),
        ("IsInRaid", is_in_raid),
        ("GetNumGroupMembers", get_num_group_members),
        ("GetNumSubgroupMembers", get_num_subgroup_members),
        ("UnitGroupRolesAssigned", unit_group_roles_assigned),
        ("UnitIsVisible", unit_is_visible),
        ("UnitPlayerOrPetInParty", unit_player_or_pet_in_party),
        ("UnitPlayerOrPetInRaid", unit_player_or_pet_in_raid),
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
        ("C_Secrets.GetPowerTypeSecrecy", power_type_secrecy),
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
        ("joinGroup", crate::party::control_join_group),
        ("openTrade", crate::party::control_open_trade),
        ("setInspect", crate::inspect::control_set_inspect),
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
        ("gather", crate::loot::control_gather),
        ("openTradeSkill", crate::tradeskill::control_open),
        ("craftSkillUps", crate::tradeskill::control_skill_ups),
        ("craftTimings", crate::tradeskill::control_timings),
        ("setSpellFocus", crate::tradeskill::control_spell_focus),
        ("interact", crate::quests::control_interact),
        ("die", crate::death::control_die),
        ("resurrect", crate::death::control_resurrect),
        ("setThreat", control_set_threat),
        ("raiseSkill", control_raise_skill),
        ("leaveCombat", control_leave_combat),
        ("showNameplate", control_show_nameplate),
        ("hit", control_hit),
        ("updateAura", crate::auras::control_update),
        ("setRole", control_set_role),
        ("setOnline", control_set_online),
        ("hideNameplate", control_hide_nameplate),
        ("despawn", control_despawn),
        ("token", control_tokens),
        ("setName", control_set_name),
        ("applyAura", crate::auras::control_apply),
        ("removeAura", crate::auras::control_remove),
        ("castSpell", crate::casting::control_cast),
        ("useSpell", crate::casting::control_use_spell),
        ("castFailed", crate::casting::control_cast_failed),
        ("interruptCast", crate::casting::control_interrupt),
        (
            "spellFailedQuiet",
            crate::casting::control_spell_failed_quiet,
        ),
        ("setMoving", crate::casting::control_set_moving),
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

/// `mock.unitOf(token)`: the unit a token names (a world profile's target,
/// say), nil when none.
pub fn control_unit_of(vm: &mut Vm, c: NativeCall) -> Ret {
    let token = vm.check_bytes(&c, 1)?;
    let token = String::from_utf8_lossy(&token).into_owned();
    Ok(vec![match resolve(&engine(vm).units, &token) {
        Some(id) => handle(vm, id),
        None => Value::Nil,
    }])
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
    let player_reaction = match get(vm, "playerReaction") {
        Value::Str(r) => Some(match vm.str_bytes(r) {
            b"friendly" => Reaction::Friendly,
            b"neutral" => Reaction::Neutral,
            _ => Reaction::Hostile,
        }),
        _ => None,
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
        // Its faction, else a world profile's character's (`faction`: a
        // race of either, the Skyborne, recorded Horde).
        faction_group: match (get(vm, "factionGroup"), get(vm, "faction")) {
            (Value::Str(s), _) | (_, Value::Str(s)) => {
                Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned())
            }
            _ => None,
        },
        sex: num(vm, "sex", 1.0),
        reaction,
        player_reaction,
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
        listed_dead: false,
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
        role: match get(vm, "role") {
            Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
            _ => None,
        },
        zone: match get(vm, "zone") {
            Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
            _ => None,
        },
        subgroup: None,
        online: !matches!(get(vm, "online"), Value::Bool(false)),
    }
}

fn target_arg(vm: &Vm, c: &NativeCall) -> Option<UnitId> {
    handle_of(vm, c.arg(2))
}

/// The next target change's echo (`echo`: seconds, as recorded, or `false`
/// for none: a target changed again before the server told it).
fn set_echo(vm: &mut Vm, echo: Value) {
    match echo {
        Value::Num(n) => engine_mut(vm).units.echo = Some(Some(n)),
        Value::Bool(false) => engine_mut(vm).units.echo = Some(None),
        _ => {}
    }
}

// The server's `UNIT_TARGET("player")` comes a round trip after the
// player's `PLAYER_TARGET_CHANGED` (`/stdtest engine record target`:
// 0.4–0.6 s; scenario combat 0.3–0.5 s at a ping near 0.3 s).

/// `unit:target(other, { echo? })`: the player's `PLAYER_TARGET_CHANGED()`
/// at once and `UNIT_TARGET("player")` when the server's echo arrives (a
/// round trip later, or `echo` seconds as recorded; `echo = false`: none);
/// another unit's `UNIT_TARGET` at once (unverified).
fn control_target(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let new = target_arg(vm, &c);
    if me == PLAYER {
        if let Value::Table(t) = c.arg(3) {
            let k = vm.str("echo");
            let echo = vm.heap.table(t).get(k);
            set_echo(vm, echo);
        }
        set_player_target(vm, new);
    } else {
        unit_target(vm, me, new);
    }
    Ok(vec![])
}

/// Another unit's target now `new`, told at once.
fn unit_target(vm: &mut Vm, me: UnitId, new: Option<UnitId>) {
    engine_mut(vm).units.list[me.0].target = new;
    // A creature turning on someone, as a wolf on the player
    // (`/stdtest engine scenario combat`): the new target's flags (also by
    // its new token, `targettarget`), the creature's target, its flags in
    // combat; a friendly one turning to face someone (a flight master,
    // `taxi`): its target only.
    let friendly = engine(vm).units.list[me.0].reaction == Reaction::Friendly;
    // On the player: its flags only the first time this fight, and only
    // once in combat (else told as it enters combat); the creature in
    // combat (recorded: `combat`'s wolf turning after the player's first
    // swing, flags; `death`'s second creature, none; `death`'s and
    // `auradeath`'s first, before the player's combat, none of their own).
    let on_player = new == Some(PLAYER) && !friendly;
    // On another: its flags only when the creature is the player's target
    // (the flags come with the targettarget token: recorded, `talents`, a
    // fox turning on the player untargeted told none, its flags told once
    // targeted, see `set_player_target`). Unverified: for others than the
    // player.
    let targeted = engine(vm).units.target == Some(me);
    if on_player {
        engine_mut(vm).units.list[me.0].in_combat = true;
        let first = !engine(vm).units.engaged;
        engine_mut(vm).units.engaged = true;
        if first && engine(vm).units.list[PLAYER.0].in_combat {
            fire_unit(vm, "UNIT_FLAGS", PLAYER, &[]);
        }
    } else if let Some(t) = new
        && !friendly
        && targeted
    {
        fire_unit(vm, "UNIT_FLAGS", t, &[]);
    }
    // On the player in a fight: its own update told after the player's in
    // the packet (its power), at the frame's end (`batch`).
    if on_player && engine(vm).units.list[PLAYER.0].in_combat && crate::batch::owe_turn(vm, me.0) {
        return;
    }
    told_turn(vm, me);
}

/// A creature's target told (`UNIT_TARGET`), its flags if in combat.
pub(crate) fn told_turn(vm: &mut Vm, me: UnitId) {
    fire_unit(vm, "UNIT_TARGET", me, &[]);
    if engine(vm).units.list[me.0].in_combat {
        fire_unit(vm, "UNIT_FLAGS", me, &[]);
    }
}

/// `unit:setThreat(value)`: the player's threat on a creature, as the
/// server tells it (`/stdtest engine scenario combat`: 5, 6, 10, … as the
/// fight went): `UNIT_THREAT_LIST_UPDATE(unit)`, the first time also
/// `UNIT_THREAT_SITUATION_UPDATE(player)`. `nil`: the player off its list
/// (its list's update; off the last list, the player's situation too:
/// recorded, a dead wolf's list left before the player left combat,
/// `combat`).
fn control_set_threat(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    if c.arg(2).is_nil() {
        engine_mut(vm).units.list[me.0].threat = None;
        engine_mut(vm).units.list[me.0].listed_dead = false;
        fire_unit(vm, "UNIT_THREAT_LIST_UPDATE", me, &[]);
        let any = engine(vm).units.list.iter().any(|u| u.threat.is_some());
        if !any && engine(vm).units.threat_situation {
            engine_mut(vm).units.threat_situation = false;
            fire_unit(vm, "UNIT_THREAT_SITUATION_UPDATE", PLAYER, &[]);
        }
        return Ok(vec![]);
    }
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
/// `party1`, the player alone, the player and an absent boss; in `death`,
/// the player in combat with no target, twice: a mob named that is not
/// there, nil).
fn unit_threat_situation(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(unit) = unit_arg(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    let status = match unit_arg(vm, &c, 2) {
        Some(mob) => threat_of(vm, unit, mob).map(|t| t.1),
        None if !matches!(c.arg(2), Value::Nil) => None,
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
    // A soft friend is where a spell on a friend lands without a target
    // (recorded, `profession`: Blessing of Protection unusable once a
    // friendly NPC was soft targeted, 0.2 s later; unverified: the delay).
    if i == 1 {
        crate::actions::update_target_usability(vm);
    }
    Ok(vec![])
}

/// The player's target now `new`: `PLAYER_TARGET_CHANGED`, the server's
/// echo (`UNIT_TARGET("player")`) a round trip later.
fn set_player_target(vm: &mut Vm, new: Option<UnitId>) {
    engine_mut(vm).units.target = new;
    engine_mut(vm).units.list[PLAYER.0].target = new;
    fire(vm, "PLAYER_TARGET_CHANGED", &[]);
    // A hostile target on someone: that one's flags, by its tokens with
    // targettarget (recorded, `talents`: the fox on the player targeted,
    // UNIT_FLAGS for player and targettarget; as when the target turns on
    // the player, `combat`). Unverified: for others than the player.
    let turned = new.and_then(|n| {
        let u = &engine(vm).units.list[n.0];
        (u.reaction != Reaction::Friendly)
            .then_some(u.target)
            .flatten()
    });
    if let Some(t) = turned {
        fire_unit(vm, "UNIT_FLAGS", t, &[]);
    }
    // The actions on a target's kind (recorded: Blessing of Protection's
    // after targeting an NPC, then a party member).
    crate::actions::update_target_usability(vm);
    let delay = match engine_mut(vm).units.echo.take() {
        Some(Some(d)) => d,
        Some(None) => return,
        None => crate::clock::server_delay(vm),
    };
    let echo = vm.native("targetEcho", target_echo);
    crate::clock::after_engine(vm, delay, echo);
}

/// `C_ClickBindings.GetEffectiveInteractionButton(button, modifiers)`: the
/// default interaction bindings' (a unit frame's left click targets, its
/// right click opens the menu): the button itself (unverified: not
/// recorded; `GetBindingType("LeftButton", 0)` recorded an interaction).
fn effective_interaction_button(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(r) =
        crate::recorded::lookup(vm, "C_ClickBindings.GetEffectiveInteractionButton", &c)
    {
        return Ok(r);
    }
    Ok(vec![c.arg(1)])
}

/// `C_ClickBindings.GetBindingType(button, modifiers)`: the recorded
/// answer (the left button unmodified: an interaction), else the right
/// button unmodified an interaction too (its default: the unit's menu;
/// unverified), else none.
fn click_binding_type(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(r) = crate::recorded::lookup(vm, "C_ClickBindings.GetBindingType", &c) {
        return Ok(r);
    }
    let button = match vm.hidden(c.arg(1)) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    let plain = matches!(vm.hidden(c.arg(2)), Value::Num(0.0) | Value::Nil);
    let interaction = plain && (button == "LeftButton" || button == "RightButton");
    Ok(vec![Value::Num(if interaction { 3.0 } else { 0.0 })])
}

/// `TargetUnit(unit)` (protected: a unit frame's click), `ClearTarget()`.
fn target_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let new = unit_arg(vm, &c, 1);
    set_player_target(vm, new);
    Ok(vec![])
}

fn clear_target_call(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine(vm).units.target.is_some() {
        clear_target(vm);
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
    let amount = opts.amount.unwrap_or(engine(vm).units.list[me.0].health);
    let blow = opts.by.map(|by| (by, amount));
    if let Some(by) = opts.by {
        crate::damagemeter::damage(vm, Some(by), me, amount, 0.0);
    }
    {
        let u = &mut engine_mut(vm).units.list[me.0];
        u.health = 0.0;
        u.dead = true;
    }
    kill_blow_of(vm, me, blow, opts.school);
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
    /// The killing blow's damage (else the health left) and school.
    pub amount: Option<f64>,
    pub school: f64,
    /// The experience's update this long after the kill (`xpAfter`, as
    /// recorded; else as the kill's kind has it).
    pub xp_after: Option<f64>,
    /// The death told this long after the killing blow (`diesAfter`, as
    /// recorded; else with it).
    pub dies_after: Option<f64>,
}

impl Kill {
    fn of(vm: &mut Vm, v: Value) -> Result<Kill, wowlua_vm::LuaError> {
        let Value::Table(t) = v else {
            return Ok(Kill {
                by: None,
                lootable: false,
                xp: None,
                loot: Vec::new(),
                amount: None,
                school: 1.0,
                xp_after: None,
                dies_after: None,
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
        let amount = match field(vm, "amount") {
            Value::Num(n) => Some(n),
            _ => None,
        };
        let school = match field(vm, "school") {
            Value::Num(n) => n,
            _ => 1.0,
        };
        let echo = field(vm, "echo");
        set_echo(vm, echo);
        let xp_after = match field(vm, "xpAfter") {
            Value::Num(n) => Some(n),
            _ => None,
        };
        let dies_after = match field(vm, "diesAfter") {
            Value::Num(n) if n > 0.0 => Some(n),
            _ => None,
        };
        Ok(Kill {
            xp_after,
            dies_after,
            by,
            lootable,
            xp,
            loot,
            amount,
            school,
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
    kill_blow_of(vm, me, blow, 1.0);
}

/// The killing blow's kill and combat text, of a school (`1` physical,
/// `2` holy: a Judgement's, recorded).
fn kill_blow_of(vm: &mut Vm, me: UnitId, blow: Option<(UnitId, f64)>, school: f64) {
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
        &[wound, empty, Value::Num(amount), Value::Num(school)],
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
    set_player_target(vm, None);
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
    // A corpse is out of combat and targets no one, but the player it
    // fought until the player leaves combat (recorded: its
    // `targettarget` told when the player retargets it to loot, its
    // target dropped as the player leaves combat: `quest`, `combat`,
    // `auradeath`); the player's held presses on it told to `targettarget`
    // too (recorded).
    {
        let u = &mut engine_mut(vm).units.list[me.0];
        u.in_combat = false;
        u.listed_dead |= u.threat.is_some();
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
        crate::casting::target_died(vm);
        fire(vm, "PLAYER_TARGET_DIED", &[]);
        clear_target(vm);
    }
    if engine(vm).units.list[me.0].target != Some(PLAYER) {
        engine_mut(vm).units.list[me.0].target = None;
    }
    if !instance {
        loot(vm);
    }
    crate::auras::died(vm, me);
    fire_unit(vm, "UNIT_HEALTH", me, &[]);
}

/// `unit:setPower(n, { frequent? })`: its power now `n`
/// (`UNIT_POWER_UPDATE`); `frequent`: a regeneration tick's, told by
/// `UNIT_POWER_FREQUENT` alone (recorded: mana's every ~0.1 s).
fn control_set_power(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let n = vm.check_number(&c, 2)?;
    let frequent = match c.arg(3) {
        Value::Table(t) => {
            let k = vm.str("frequent");
            vm.heap.table(t).get(k).truthy()
        }
        _ => false,
    };
    if !frequent {
        set_power(vm, me, n);
        return Ok(vec![]);
    }
    let kind = {
        let u = &mut engine_mut(vm).units.list[me.0];
        u.power = n.clamp(0.0, u.power_max.max(0.0));
        u.power_type
    };
    let token = vm.str(power_token(kind).unwrap_or("MANA"));
    fire_unit(vm, "UNIT_POWER_FREQUENT", me, &[token]);
    if me == PLAYER {
        crate::actions::update_usability(vm);
    }
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
    // Several skills up in one packet (`raiseSkill(name, rank, name2,
    // rank2)`): their messages, then the stats' events once with every
    // skill's mods (unverified, once in `death`: Defense and Swords at
    // 5.036, the damage's mods three times).
    let mut mods = 0;
    let mut i = 2;
    loop {
        let name = vm.check_str(&c, i)?;
        let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
        let rank = vm.check_number(&c, i + 1)?;
        mods += if name == "Defense" { 2 } else { 1 };
        skill_message(vm, &name, rank);
        i += 2;
        if matches!(c.arg(i), Value::Nil) {
            break;
        }
    }
    skill_stats(vm, mods);
    Ok(vec![])
}

/// A skill up to `rank` told: "Your skill in … has increased", the stats'
/// events, the damage's mods `mods` times (a weapon's once, Defense twice,
/// a profession's none).
pub fn skill_raised(vm: &mut Vm, name: &str, rank: f64, mods: usize) {
    skill_message(vm, name, rank);
    skill_stats(vm, mods);
}

/// "Your skill in … has increased to …" (`CHAT_MSG_SKILL`).
fn skill_message(vm: &mut Vm, name: &str, rank: f64) {
    let format = match vm.get_global("ERR_SKILL_UP_SI") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "Your skill in %s has increased to %d.".into(),
    };
    let text = format
        .replacen("%s", name, 1)
        .replacen("%d", &format!("{rank}"), 1);
    crate::chat::game_message(vm, "CHAT_MSG_SKILL", &text, None);
}

/// A skill's stats' events, the damage's mods `mods` times.
fn skill_stats(vm: &mut Vm, mods: usize) {
    for e in ["UNIT_ATTACK", "UNIT_DEFENSE", "UNIT_RANGED_ATTACK_POWER"] {
        fire_unit(vm, e, PLAYER, &[]);
    }
    fire(vm, "SKILL_LINES_CHANGED", &[]);
    fire(vm, "MOUNT_JOURNAL_USABILITY_CHANGED", &[]);
    for _ in 0..mods {
        fire_unit(vm, "PLAYER_DAMAGE_DONE_MODS", PLAYER, &[]);
    }
}

/// The combat text's delay after a hit lands (`/stdtest engine scenario
/// combat`: `UNIT_COMBAT` 0.33 to 0.53 s after the health; unverified:
/// what it waits for).
const COMBAT_TEXT_DELAY: f64 = 0.4;

/// `unit:swing(victim, amount, { kind?, power?, victimPower?, speed?,
/// textAfter?, combat?, indicator? })`: a melee swing landing, as the client recorded the
/// player's and a wolf's (`/stdtest engine scenario combat`):
/// `PLAYER_SWING(speed, 0)` for the player's, combat for both if not yet,
/// the attacker's then the victim's power (`power`, `victimPower`: rage),
/// the victim's health (`amount`; none for a `PARRY`, `DODGE` or `MISS`),
/// and the combat text `UNIT_COMBAT(victim, kind, "", amount, 1)` a moment
/// later: `textAfter` seconds (the recordings show 0.12 to 0.5; 0.4 when
/// not given).
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
    let lands = kind == "WOUND" || kind == "BLOCK" || kind == "ABSORB";
    if lands {
        crate::damagemeter::damage(vm, Some(me), victim, amount, crate::damagemeter::MELEE);
    }
    if let Some(k) = kill {
        let power = field(vm, "power");
        return swing_kill(vm, me, victim, k, power);
    }
    // `combat = false`: the swing puts no one in combat (recorded: the
    // quest's first, combat a moment later with the wolf's answer).
    if !matches!(field(vm, "combat"), Value::Bool(false)) {
        for id in [me, victim] {
            if !engine(vm).units.list[id.0].in_combat {
                enter_combat(vm, id, false);
            }
        }
    }
    if let Value::Num(p) = field(vm, "power") {
        set_power(vm, me, p);
    }
    if let Value::Num(p) = field(vm, "victimPower") {
        set_power(vm, victim, p);
    }
    if lands && amount > 0.0 {
        change_health(vm, victim, -amount);
    }
    if victim == PLAYER && matches!(kind.as_str(), "DODGE" | "PARRY" | "BLOCK") {
        avoided(vm);
    }
    // Its text's indicator (`indicator`: "CRITICAL", "GLANCING"; recorded,
    // the quest's).
    let indicator = match field(vm, "indicator") {
        Value::Str(s) => Value::Str(s),
        _ => vm.str(""),
    };
    let upvals = vec![
        Value::Num(victim.0 as f64),
        vm.str(&kind),
        Value::Num(if lands { amount } else { 0.0 }),
        indicator,
    ];
    let text = vm.native_with("swingText", swing_text, upvals);
    // The combat text's delay: the swing's `textAfter` (a recording's,
    // per hit: 0.12 to 0.5 s), else the usual.
    let after = match field(vm, "textAfter") {
        Value::Num(n) => n,
        _ => COMBAT_TEXT_DELAY,
    };
    // The client's own delay: the text comes after the frame's server
    // packets and batched events (recorded: a swing's text after the
    // frame's mana update, auradeath 13.382), so half a millisecond on.
    let late = if engine(vm).batch.on { 0.0005 } else { 0.0 };
    crate::clock::after_engine(vm, after + late, text);
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
        let delay = k.xp_after.unwrap_or_else(|| crate::clock::server_delay(vm));
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
    crate::damagemeter::damage(vm, Some(PLAYER), victim, dealt, spell);
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
    if let Some(k) = kill.as_ref().filter(|k| k.dies_after.is_none()) {
        kill_xp(vm, victim, k, true);
    }
    if let Value::Num(p) = power {
        set_power_quietly(vm, PLAYER, p);
    }
    if let Some(k) = &kill {
        // The experience with the blow, or its recorded delay after.
        match (k.xp, k.xp_after) {
            (Some(xp), Some(after)) if after > 0.0 => {
                let later = vm.native_with("killXp", kill_xp_later, vec![Value::Num(xp)]);
                crate::clock::after_engine(vm, after, later);
            }
            _ => kill_xp(vm, victim, k, false),
        }
        engine_mut(vm).units.list[victim.0].loot = k.loot.clone();
        match k.dies_after {
            Some(after) => {
                let later = vm.native_with(
                    "killDied",
                    died_later,
                    vec![
                        Value::Num(victim.0 as f64),
                        Value::Bool(k.lootable),
                        k.xp.map_or(Value::Nil, Value::Num),
                    ],
                );
                crate::clock::after_engine(vm, after, later);
            }
            None => died(vm, victim, k.lootable),
        }
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
    if kill.as_ref().is_none_or(|k| k.dies_after.is_some()) {
        fire_unit(vm, "UNIT_HEALTH", victim, &[]);
    }
    fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
    Ok(vec![])
}

/// A kill's death told after its blow (`diesAfter`, recorded once in
/// `quest`: a Heroic Strike's kill at 101.134, the death 0.104 s later
/// with the experience's message and the corpse's loot after it, the
/// blow's usability, cooldowns and the victim's health told before).
fn died_later(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(victim), Value::Bool(lootable)) = (c.upvals[0], c.upvals[1]) else {
        return Ok(vec![]);
    };
    let victim = UnitId(victim as usize);
    died(vm, victim, false);
    if let Value::Num(xp) = c.upvals[2] {
        let k = Kill {
            by: None,
            lootable,
            xp: Some(xp),
            loot: Vec::new(),
            amount: None,
            school: 1.0,
            xp_after: None,
            dies_after: None,
        };
        kill_xp(vm, victim, &k, true);
    }
    if lootable {
        let g = engine(vm).units.list[victim.0].guid.clone();
        let g = vm.str(&g);
        fire(vm, "UNIT_LOOT", &[g, Value::Bool(true)]);
    }
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
/// `unit:hit(victim, amount, { school?, indicator?, kind?, aura?,
/// cooldowns? })`: a spell's or a periodic effect's hit, as one server
/// packet (recorded: a Consecration's ticks, a seal's holy damage,
/// auradeath): combat for both if not yet; the combat text at once, with
/// the school (`2` holy; `1` physical by default) and indicator
/// (`"CRITICAL"`); the victim's aura `aura` updated; the triggered spells'
/// cooldowns (`cooldowns = { { spell, a?, b? } }`: `SPELL_UPDATE_COOLDOWN`'s
/// third and fourth values); the victim's health; then
/// `UPDATE_SHAPESHIFT_COOLDOWN`.
fn control_hit(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let Some(victim) = handle_of(vm, c.arg(2)) else {
        return Err(vm.error("hit: a unit to hit expected"));
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
    let text = |vm: &mut Vm, k: &str, d: &str| match field(vm, k) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => d.to_string(),
    };
    let kind = text(vm, "kind", "WOUND");
    let indicator = text(vm, "indicator", "");
    let school = match field(vm, "school") {
        Value::Num(n) => n,
        _ => 1.0,
    };
    let aura = field(vm, "aura");
    let cooldowns = field(vm, "cooldowns");
    let spell = match field(vm, "spell") {
        Value::Num(n) => n,
        _ => 0.0,
    };
    for id in [me, victim] {
        if !engine(vm).units.list[id.0].in_combat {
            enter_combat(vm, id, false);
        }
    }
    let lands = kind == "WOUND" || kind == "BLOCK" || kind == "ABSORB";
    if lands && amount > 0.0 {
        let u = &mut engine_mut(vm).units.list[victim.0];
        u.health = (u.health - amount).clamp(0.0, u.health_max);
        u.dead = u.health <= 0.0;
    }
    // A heal: the health up, told as a wound's (recorded: a drain's heal on
    // the player, auradeath).
    let heals = kind == "HEAL" && amount > 0.0;
    if heals {
        let u = &mut engine_mut(vm).units.list[victim.0];
        u.health = (u.health + amount).min(u.health_max);
        crate::damagemeter::heal(vm, me, victim, amount, spell);
    } else if lands {
        crate::damagemeter::damage(vm, Some(me), victim, amount, spell);
    }
    let kind = vm.str(&kind);
    let indicator = vm.str(&indicator);
    fire_unit(
        vm,
        "UNIT_COMBAT",
        victim,
        &[kind, indicator, Value::Num(amount), Value::Num(school)],
    );
    if let Value::Num(id) = aura {
        crate::auras::fire_updated(vm, victim, id);
    }
    let any = triggered_cooldowns(vm, cooldowns);
    if (lands && amount > 0.0) || heals {
        fire_unit(vm, "UNIT_HEALTH", victim, &[]);
    }
    if any {
        fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
    }
    Ok(vec![])
}

/// Triggered spells' cooldowns (`{ { spell, a?, b? } }`):
/// `ACTIONBAR_UPDATE_COOLDOWN`, `BAG_UPDATE_COOLDOWN`, then each
/// `SPELL_UPDATE_COOLDOWN(spell, nil, a, b, nil)` (recorded); whether any.
pub(crate) fn triggered_cooldowns(vm: &mut Vm, list: Value) -> bool {
    let Value::Table(list) = list else {
        return false;
    };
    let n = vm.heap.table(list).border();
    if n == 0 {
        return false;
    }
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
    for i in 1..=n {
        let Value::Table(e) = vm.heap.table(list).get(Value::Num(i as f64)) else {
            continue;
        };
        let t = vm.heap.table(e);
        let (spell, a, b) = (
            t.get(Value::Num(1.0)),
            t.get(Value::Num(2.0)),
            t.get(Value::Num(3.0)),
        );
        fire(
            vm,
            "SPELL_UPDATE_COOLDOWN",
            &[spell, Value::Nil, a, b, Value::Nil],
        );
    }
    true
}

fn swing_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(id), kind, Value::Num(amount)) = (c.upvals[0], c.upvals[1], c.upvals[2]) else {
        return Ok(vec![]);
    };
    let indicator = match c.upvals.get(3) {
        Some(v @ Value::Str(_)) => *v,
        _ => vm.str(""),
    };
    fire_unit(
        vm,
        "UNIT_COMBAT",
        UnitId(id as usize),
        &[kind, indicator, Value::Num(amount), Value::Num(1.0)],
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
        crate::damagemeter::combat_began(vm);
    }
}

fn control_leave_combat(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    engine_mut(vm).units.list[me.0].in_combat = false;
    // The living creatures whose threat lists the player was on (one
    // giving up, alive: their lists told by the server after the rest of
    // the moment, the player's health too; recorded: talents, a fox's).
    let living: Vec<UnitId> = engine(vm)
        .units
        .list
        .iter()
        .enumerate()
        .filter(|(_, u)| u.threat.is_some() && !u.dead)
        .map(|(i, _)| UnitId(i))
        .collect();
    let situation = me == PLAYER && engine(vm).units.threat_situation;
    if situation {
        engine_mut(vm).units.threat_situation = false;
        for u in engine_mut(vm).units.list.iter_mut() {
            u.threat = None;
        }
    }
    if me == PLAYER {
        // A creature giving up drops the player as its target with the
        // combat's end, told after the player's flags (already without the
        // targettarget token), before the actions' usability (recorded
        // once, `talents`: the fox's).
        let dropping: Vec<UnitId> = living
            .iter()
            .copied()
            .filter(|u| engine(vm).units.list[u.0].target == Some(PLAYER))
            .collect();
        for &u in &dropping {
            engine_mut(vm).units.list[u.0].target = None;
        }
        player_out_of_combat(vm);
        for u in dropping {
            unit_target(vm, u, None);
        }
        // The corpses still on the player drop it (recorded: after the
        // player's flags, `quest`, `combat`; unverified: after the living
        // ones', never recorded together).
        let corpses: Vec<UnitId> = engine(vm)
            .units
            .list
            .iter()
            .enumerate()
            .filter(|(_, u)| u.dead && u.target == Some(PLAYER))
            .map(|(i, _)| UnitId(i))
            .collect();
        for id in corpses {
            engine_mut(vm).units.list[id.0].target = None;
            fire_unit(vm, "UNIT_TARGET", id, &[]);
            fire_unit(vm, "UNIT_FLAGS", id, &[]);
        }
        crate::actions::update_usability(vm);
    }
    // Off the dead creatures' lists with the combat's end, after the
    // corpses dropped the player (so no `targettarget`): each corpse's
    // list not told yet, then the player's situation (recorded: `quest`'s
    // 8 fights' ends, the corpse's list by its tokens, the situation the
    // player's alone; `auradeath`'s; `combat`'s list told before, by the
    // server, the situation with it).
    if me == PLAYER {
        let listed: Vec<UnitId> = engine(vm)
            .units
            .list
            .iter()
            .enumerate()
            .filter(|(_, u)| u.listed_dead)
            .map(|(i, _)| UnitId(i))
            .collect();
        for id in listed {
            engine_mut(vm).units.list[id.0].listed_dead = false;
            if situation {
                fire_unit(vm, "UNIT_THREAT_LIST_UPDATE", id, &[]);
            }
        }
    }
    if situation && living.is_empty() {
        fire_unit(vm, "UNIT_THREAT_SITUATION_UPDATE", PLAYER, &[]);
    }
    if situation && !living.is_empty() {
        let ids = living.iter().map(|id| Value::Num(id.0 as f64)).collect();
        let told = vm.native_with("threatDropped", threat_dropped, ids);
        crate::clock::after_engine(vm, 0.0, told);
    }
    Ok(vec![])
}

/// Off living creatures' threat lists: each list's update, then the
/// player's situation.
fn threat_dropped(vm: &mut Vm, c: NativeCall) -> Ret {
    for v in c.upvals.iter().copied() {
        if let Value::Num(n) = v {
            fire_unit(vm, "UNIT_THREAT_LIST_UPDATE", UnitId(n as usize), &[]);
        }
    }
    fire_unit(vm, "UNIT_THREAT_SITUATION_UPDATE", PLAYER, &[]);
    Ok(vec![])
}

/// The player out of combat: `PLAYER_IN_COMBAT_CHANGED(false)`, the
/// restriction ending, `PLAYER_REGEN_ENABLED()`, the player's flags.
pub(crate) fn player_out_of_combat(vm: &mut Vm) {
    engine_mut(vm).units.list[PLAYER.0].in_combat = false;
    engine_mut(vm).units.engaged = false;
    engine_mut(vm).units.lockdown = false;
    fire(vm, "PLAYER_IN_COMBAT_CHANGED", &[Value::Bool(false)]);
    set_restriction(vm, COMBAT, false);
    fire(vm, "PLAYER_REGEN_ENABLED", &[]);
    fire_unit(vm, "UNIT_FLAGS", PLAYER, &[]);
    crate::damagemeter::combat_ended(vm);
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
    // A slot's first use makes its frame: `NAME_PLATE_CREATED(frame)` in
    // the same frame as, and before, its `NAME_PLATE_UNIT_ADDED` (recorded,
    // the auction scenario). Its name and parent are unverified.
    if engine(vm).units.nameplate_frames.len() <= slot {
        let world = engine(vm)
            .world
            .widgets
            .iter()
            .position(|w| w.name.as_deref() == Some("WorldFrame"))
            .map(widgets::WidgetId);
        let frame = widgets::create(
            vm,
            "NamePlate",
            Some(format!("NamePlate{}", slot + 1)),
            world,
        );
        let id = widgets::widget_of(vm, frame).expect("a widget");
        engine_mut(vm).world.widgets[id.0].frame = true;
        engine_mut(vm).units.nameplate_frames.push(id);
        fire(vm, "NAME_PLATE_CREATED", &[frame]);
    }
    let token = vm.str(&format!("nameplate{}", slot + 1));
    fire(vm, "NAME_PLATE_UNIT_ADDED", &[token]);
    Ok(vec![Value::Num(slot as f64 + 1.0)])
}

/// `C_NamePlate.GetNamePlateForUnit(unit)`: the frame of the nameplate the
/// unit is on, or nil.
///
/// No nameplate: nil (the full probe, 1.60.1: every token, none shown).
fn get_name_plate_for_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = unit_arg(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    let u = &engine(vm).units;
    let frame = u
        .nameplates
        .iter()
        .position(|n| *n == Some(id))
        .and_then(|i| u.nameplate_frames.get(i));
    Ok(match frame {
        Some(f) => vec![Value::Table(engine(vm).world.widgets[f.0].table)],
        None => vec![Value::Nil],
    })
}

/// `C_NamePlate.GetNamePlates()`: the frames of the nameplates shown, in
/// slot order.
fn get_name_plates(vm: &mut Vm, _c: NativeCall) -> Ret {
    let frames: Vec<Value> = {
        let e = engine(vm);
        let u = &e.units;
        u.nameplates
            .iter()
            .zip(&u.nameplate_frames)
            .filter(|(n, _)| n.is_some())
            .map(|(_, f)| Value::Table(e.world.widgets[f.0].table))
            .collect()
    };
    let list = vm.new_table();
    for (i, f) in frames.into_iter().enumerate() {
        let _ = vm.heap.table_mut(list).set(Value::Num(i as f64 + 1.0), f);
    }
    Ok(vec![Value::Table(list)])
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

/// Every nameplate removed (`NAME_PLATE_UNIT_REMOVED` each: leaving the
/// world).
pub(crate) fn remove_nameplates(vm: &mut Vm) {
    let shown: Vec<usize> = engine(vm)
        .units
        .nameplates
        .iter()
        .enumerate()
        .filter(|(_, n)| n.is_some())
        .map(|(i, _)| i)
        .collect();
    for i in shown {
        let token = vm.str(&format!("nameplate{}", i + 1));
        fire(vm, "NAME_PLATE_UNIT_REMOVED", &[token]);
        engine_mut(vm).units.nameplates[i] = None;
    }
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

/// `mock.addToGroup(unit, raid? | { raid, subgroup })`: the unit joins the
/// player's group (a raid with `raid`; in a raid, in `subgroup`). A group
/// the player makes this way the player leads (unverified).
pub fn add_to_group(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = handle_of(vm, c.arg(1)) else {
        return Err(vm.error("mock.addToGroup: a unit expected"));
    };
    let (raid, subgroup) = match c.arg(2) {
        Value::Table(t) => {
            let raid = vm.str("raid");
            let raid = vm.heap.table(t).get(raid).truthy();
            let key = vm.str("subgroup");
            let subgroup = match vm.heap.table(t).get(key) {
                Value::Num(n) => Some(n),
                _ => None,
            };
            (raid, subgroup)
        }
        v => (v.truthy(), None),
    };
    let forming = !crate::party::grouped(vm);
    {
        let u = &mut engine_mut(vm).units;
        if !u.group.contains(&id) {
            u.group.push(id);
        }
        u.raid = u.raid || raid;
        if subgroup.is_some() {
            u.list[id.0].subgroup = subgroup;
        }
    }
    if forming {
        crate::party::lead(vm);
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

/// `UnitHealthMissing(unit)`: a unit that does not exist (or a GUID)
/// misses none, a secret like any unit's (the full probe).
fn unit_health_missing(vm: &mut Vm, c: NativeCall) -> Ret {
    if unit_arg(vm, &c, 1).is_none() {
        let zero = vm.secret_wrap(Value::Num(0.0));
        return Ok(vec![zero]);
    }
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

/// A unit's faction (its own, else its race's).
fn faction_of(u: &Unit) -> Option<String> {
    u.faction_group
        .clone()
        .or_else(|| faction(&race_file_and_id(&u.race).0).map(str::to_string))
}

/// `UnitCanCooperate(a, b)`: two players of one faction (unverified: not
/// recorded; Blizzard's unit menu shows Trade and Follow by it); not a
/// unit with itself (the full probe: the player's with "player" false).
fn unit_can_cooperate(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(a), Some(b)) = (unit_arg(vm, &c, 1), unit_arg(vm, &c, 2)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let u = &engine(vm).units.list;
    let ok = a != b
        && u[a.0].player
        && u[b.0].player
        && faction_of(&u[a.0]).is_some()
        && faction_of(&u[a.0]) == faction_of(&u[b.0]);
    Ok(vec![Value::Bool(ok)])
}

/// `UnitIsHumanPlayer(unit)`: a player's character (unverified: as
/// `UnitIsPlayer`).
fn unit_is_human_player(vm: &mut Vm, c: NativeCall) -> Ret {
    unit_is_player(vm, c)
}

/// `UnitPVPName(unit)`: the full name, the last name with it on WoW
/// Forever ("Player Lastname", the full probe); a unit that does not exist
/// an empty one (the full probe: a GUID's "", the tokens' blanked).
fn unit_pvp_name(vm: &mut Vm, c: NativeCall) -> Ret {
    if unit_arg(vm, &c, 1).is_none() {
        return Ok(vec![vm.str("")]);
    }
    with_unit(vm, &c, |vm, u| {
        let name = match &u.last_name {
            Some(last) => format!("{} {last}", u.name),
            None => u.name.clone(),
        };
        vec![vm.str(&name)]
    })
}

/// `UnitIsGhost(unit)`: no such unit false (the full probe).
fn unit_is_ghost(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| {
        vec![Value::Bool(u.ghost)]
    })
}

fn unit_is_dead_or_ghost(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit_or(vm, &c, Value::Bool(false), |_, u| {
        vec![Value::Bool(u.dead || u.ghost)]
    })
}

/// Not casting or channeling: no values.
/// The roles a unit can take (tank, healer, damage) by its class (the
/// full probe: the paladin all three; the other classes unverified, as
/// their talents allow); a unit that does not exist nothing (the full
/// probe).
fn available_roles(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |_, u| {
        let (tank, healer) = match u.class.as_str() {
            "PALADIN" | "DRUID" | "MONK" => (true, true),
            "WARRIOR" | "DEATHKNIGHT" | "DEMONHUNTER" => (true, false),
            "PRIEST" | "SHAMAN" | "EVOKER" => (false, true),
            _ => (false, false),
        };
        vec![Value::Bool(tank), Value::Bool(healer), Value::Bool(true)]
    })
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
