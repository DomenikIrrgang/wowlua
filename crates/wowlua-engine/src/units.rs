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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// (`/stdtest engine`: "Suther", "Stargazer"), not the realm.
    pub last_name: Option<String>,
    pub level: f64,
    /// The class's file name (`WARRIOR`), its name and ID.
    pub class: String,
    pub class_name: String,
    pub class_id: f64,
    pub race: String,
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
    pub in_combat: bool,
    /// Whom the unit targets (`unittarget`).
    pub target: Option<UnitId>,
    pub alive_in_world: bool,
    /// How far from the player, in yards (`None`: unknown).
    pub distance: Option<f64>,
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
    pub list: Vec<Unit>,
    pub target: Option<UnitId>,
    pub focus: Option<UnitId>,
    pub mouseover: Option<UnitId>,
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
        sex: 2.0,
        reaction: Reaction::Friendly,
        player: true,
        health: 100.0,
        health_max: 100.0,
        power: 0.0,
        power_max: 100.0,
        power_type: 1.0,
        dead: false,
        in_combat: false,
        target: None,
        alive_in_world: true,
        distance: Some(0.0),
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
    let player_controlled = u.list[id.0].player;
    if (flag("SecretWhenUnitHealthMaxRestricted") || flag("SecretWhenUnitPowerMaxRestricted"))
        && !player_controlled
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
    if flag("SecretWhenUnitPowerRestricted")
        && active(u, &[COMBAT, MAP, ENCOUNTER, CHALLENGE_MODE, PVP_MATCH])
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
        ..*u
    }
}

fn unit_exists(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(unit_arg(vm, &c, 1).is_some())])
}

fn unit_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| vec![vm.str(&u.guid)])
}

fn unit_name(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| {
        // On WoW Forever the second value is the last name; elsewhere a
        // player from another realm's realm.
        let second = u
            .last_name
            .as_ref()
            .or(u.realm.as_ref())
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

fn unit_race(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| vec![vm.str(&u.race), vm.str(&u.race)])
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
    with_unit(vm, &c, |_, u| vec![Value::Bool(u.dead)])
}

fn unit_is_player(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |_, u| vec![Value::Bool(u.player)])
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

/// The reaction of `b` toward `a` (1 hated .. 8 exalted; hostile 2,
/// neutral 4, friendly 5).
fn unit_reaction(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(_), Some(b)) = (unit_arg(vm, &c, 1), unit_arg(vm, &c, 2)) else {
        return Ok(vec![]);
    };
    let r = match engine(vm).units.list[b.0].reaction {
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
        ("UnitName", unit_name),
        ("UnitLevel", unit_level),
        ("UnitClass", unit_class),
        ("UnitRace", unit_race),
        ("UnitSex", unit_sex),
        ("UnitHealth", unit_health),
        ("UnitHealthMax", unit_health_max),
        ("UnitPower", unit_power),
        ("UnitPowerMax", unit_power_max),
        ("UnitPowerType", unit_power_type),
        ("UnitIsDead", unit_is_dead),
        ("UnitIsDeadOrGhost", unit_is_dead),
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
        ("focus", control_focus),
        ("mouseover", control_mouseover),
        ("damage", control_damage),
        ("heal", control_heal),
        ("kill", control_kill),
        ("setPower", control_set_power),
        ("enterCombat", control_enter_combat),
        ("leaveCombat", control_leave_combat),
        ("showNameplate", control_show_nameplate),
        ("hideNameplate", control_hide_nameplate),
        ("despawn", control_despawn),
        ("token", control_tokens),
        ("setName", control_set_name),
        ("applyAura", crate::auras::control_apply),
        ("removeAura", crate::auras::control_remove),
        ("setDistance", crate::character::control_set_distance),
        ("setMoney", crate::character::control_set_money),
        ("equip", crate::character::control_equip),
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
        sex: num(vm, "sex", 1.0),
        reaction,
        player,
        health: num(vm, "health", health_max),
        health_max,
        power: num(vm, "power", 0.0),
        power_max: num(vm, "maxPower", 100.0),
        power_type: num(vm, "powerType", 0.0),
        dead: get(vm, "dead").truthy(),
        in_combat: false,
        target: None,
        alive_in_world: true,
        distance: match get(vm, "distance") {
            Value::Num(d) => Some(d),
            _ => None,
        },
    }
}

fn target_arg(vm: &Vm, c: &NativeCall) -> Option<UnitId> {
    handle_of(vm, c.arg(2))
}

fn control_target(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let new = target_arg(vm, &c);
    if me == PLAYER {
        engine_mut(vm).units.target = new;
        engine_mut(vm).units.list[PLAYER.0].target = new;
        fire(vm, "PLAYER_TARGET_CHANGED", &[]);
    } else {
        engine_mut(vm).units.list[me.0].target = new;
    }
    fire_unit(vm, "UNIT_TARGET", me, &[]);
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

fn control_kill(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let h = engine(vm).units.list[me.0].health;
    change_health(vm, me, -h);
    Ok(vec![])
}

fn control_set_power(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    let n = vm.check_number(&c, 2)?;
    engine_mut(vm).units.list[me.0].power = n;
    let token = vm.str("MANA");
    fire_unit(vm, "UNIT_POWER_UPDATE", me, &[token]);
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
    engine_mut(vm).units.list[me.0].in_combat = true;
    if me == PLAYER {
        // As recorded (`/stdtest engine record combat`): REGEN_DISABLED,
        // the restriction activating, PLAYER_IN_COMBAT_CHANGED, the
        // player's flags; the lockdown after.
        fire(vm, "PLAYER_REGEN_DISABLED", &[]);
        set_restriction(vm, COMBAT, true);
        fire(vm, "PLAYER_IN_COMBAT_CHANGED", &[Value::Bool(true)]);
        fire_unit(vm, "UNIT_FLAGS", PLAYER, &[]);
        engine_mut(vm).units.lockdown = true;
    }
    Ok(vec![])
}

fn control_leave_combat(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = this(vm, &c)?;
    engine_mut(vm).units.list[me.0].in_combat = false;
    if me == PLAYER {
        // The reverse: PLAYER_IN_COMBAT_CHANGED, the restriction ending,
        // REGEN_ENABLED, the player's flags.
        engine_mut(vm).units.lockdown = false;
        fire(vm, "PLAYER_IN_COMBAT_CHANGED", &[Value::Bool(false)]);
        set_restriction(vm, COMBAT, false);
        fire(vm, "PLAYER_REGEN_ENABLED", &[]);
        fire_unit(vm, "UNIT_FLAGS", PLAYER, &[]);
    }
    Ok(vec![])
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

fn unit_faction_group(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| match faction(&u.race) {
        Some(f) => vec![vm.str(f), vm.str(f)],
        None => vec![],
    })
}

fn unit_pvp_name(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |vm, u| vec![vm.str(&u.name)])
}

fn unit_is_ghost(vm: &mut Vm, c: NativeCall) -> Ret {
    with_unit(vm, &c, |_, _| vec![Value::Bool(false)])
}

/// Not casting or channeling: no values.
fn not_casting(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

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
        ("UnitFullName", unit_full_name),
        ("ClearFocus", clear_focus),
        ("TargetLastTarget", nothing),
        ("StopAttack", nothing),
        ("UnitFactionGroup", unit_faction_group),
        ("UnitPVPName", unit_pvp_name),
        ("UnitIsGhost", unit_is_ghost),
        ("UnitCastingInfo", not_casting),
        ("UnitChannelInfo", not_casting),
        ("UnitGetAvailableRoles", available_roles),
        ("UnitHealthPercent", unit_health_percent),
        ("UnitHealthMissing", unit_health_missing),
        ("UnitPowerPercent", unit_power_percent),
    ]
}
