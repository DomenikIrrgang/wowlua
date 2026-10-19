//! What the client tells of units beyond `units.rs`: their state flags
//! (away, feigning, charmed, in a vehicle), their group and realm, the
//! player's own (`C_PlayerInfo`), units known by GUID, the pet's, the aura
//! queries `auras.rs` leaves (`C_UnitAuras.GetUnitAuras` and the queries by
//! instance), and the systems WoW Forever lacks (battle pets, empowered
//! casts, stagger, runes, Chromie time, mythic+, loss of control), which
//! answer their "nothing here" state.
//!
//! Answers come from the same state `units.rs`, `party.rs`, `auras.rs`,
//! `pets.rs` and `casting.rs` keep, so they agree with the unit functions.
//! Unless a function's comment names a recording, its answer is unverified:
//! taken from the API documentation, Blizzard's UI code or
//! warcraft.wiki.gg.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, PLAYER, Reaction, UnitId};
use crate::{auras, engine, engine_mut, widgets};

// ------------------------------------------------------------- helpers

fn num(vm: &Vm, v: Value) -> Option<f64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// The unit argument `n` names; a nil argument the player (the
/// documentation's nilable unit arguments).
fn unit_or_player(vm: &Vm, c: &NativeCall, n: usize) -> Option<UnitId> {
    if vm.hidden(c.arg(n)).is_nil() {
        return Some(PLAYER);
    }
    units::unit_arg(vm, c, n)
}

/// The unit with this GUID, if the world has it.
fn by_guid(vm: &Vm, v: Value) -> Option<UnitId> {
    let guid = text(vm, v)?;
    engine(vm)
        .units
        .list
        .iter()
        .position(|u| u.guid == guid)
        .map(UnitId)
}

/// The unit a `PlayerLocation` names: its `unit` token, else its `guid`
/// (`PlayerLocationMixin`'s fields).
fn located(vm: &mut Vm, v: Value) -> Option<UnitId> {
    let Value::Table(t) = vm.hidden(v) else {
        return None;
    };
    let k = vm.str("unit");
    let unit = vm.heap.table(t).get(k);
    if let Some(token) = text(vm, unit) {
        return units::resolve(&engine(vm).units, &token);
    }
    let k = vm.str("guid");
    let guid = vm.heap.table(t).get(k);
    by_guid(vm, guid)
}

/// Whether the unit is the player or in the player's group.
fn ours(vm: &Vm, id: UnitId) -> bool {
    id == PLAYER || engine(vm).units.group.contains(&id)
}

/// The player's realm relation to a unit's: the same unless it has a
/// realm of its own other than the player's.
fn same_realm(vm: &Vm, id: UnitId) -> bool {
    let list = &engine(vm).units.list;
    match &list[id.0].realm {
        None => true,
        Some(r) => list[PLAYER.0].realm.as_ref() == Some(r),
    }
}

/// A unit's realm's name as the client gives it beside a name: none (an
/// empty string) on the player's realm.
fn realm_text(vm: &Vm, id: UnitId) -> String {
    if same_realm(vm, id) {
        return String::new();
    }
    engine(vm).units.list[id.0]
        .realm
        .clone()
        .unwrap_or_default()
}

/// A documented structure's empty value (its fields' defaults).
fn structure_default(vm: &mut Vm, ty: &str) -> Value {
    let api = engine(vm).api;
    let doc = serde_json::json!({ "Type": ty, "Nilable": false });
    crate::default_value(vm, api, &doc, 0)
}

fn list_of(vm: &mut Vm, values: &[Value]) -> Value {
    let t = vm.new_table();
    for (i, v) in values.iter().enumerate() {
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), *v);
    }
    Value::Table(t)
}

/// `Enum.UnitSex` (male 0, female 1, none 2) from `UnitSex`'s numbers (1
/// unknown, 2 male, 3 female).
fn sex_enum(sex: f64) -> f64 {
    match sex as i64 {
        2 => 0.0,
        3 => 1.0,
        _ => 2.0,
    }
}

// ------------------------------------------------- the nothing answers

/// Answers of the systems WoW Forever lacks or the engine does not model
/// (the documentation's empty values: false, 0, nil, an empty list).
fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn yes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn one(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1.0)])
}

fn nil(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn empty_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

/// `UnitAlliedRaceInfo(unit)`: no allied races on WoW Forever.
fn two_falses(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false), Value::Bool(false)])
}

fn three_nils(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil, Value::Nil, Value::Nil])
}

fn three_zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 3])
}

/// Not eligible, no reason (`C_PlayerInfo.IsPlayerEligibleForNPE`,
/// `CanPlayerUseMountEquipment`: no new player experience or mount
/// equipment on WoW Forever).
fn no_with_reason(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false), vm.str("")])
}

// ------------------------------------------------------- unit states

/// `UnitSelectionType(unit [, useExtendedColors])`: a player 4 (5 with
/// extended colors), a creature by its reaction (hostile 0, neutral 2,
/// friendly 3), as `UnitSelectionColor` colors it; no unit 4 or 5
/// (warcraft.wiki.gg's values). Recorded (the user's manual probe,
/// 2026-10-19): the player 4, as "player" and targeted; hostile mobs 0
/// at the player's level, green and grey; a neutral grey mob 2: by the
/// reaction alone, not the level. Unverified: friendly creatures,
/// extended colors.
fn unit_selection_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let extended = vm.hidden(c.arg(2)).truthy();
    let player_type = if extended { 5.0 } else { 4.0 };
    let Some(id) = units::unit_arg(vm, &c, 1) else {
        return Ok(vec![Value::Num(player_type)]);
    };
    let u = &engine(vm).units.list[id.0];
    let t = if u.player {
        player_type
    } else {
        match u.reaction {
            Reaction::Hostile => 0.0,
            Reaction::Neutral => 2.0,
            Reaction::Friendly => 3.0,
        }
    };
    Ok(vec![Value::Num(t)])
}

/// `UnitHasPowerType(unit, powerType)`: its own power, or a druid's mana
/// besides its form's (unverified: the client's secondary powers).
fn unit_has_power_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = units::unit_arg(vm, &c, 1) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let pt = num(vm, c.arg(2)).unwrap_or(-1.0);
    let u = &engine(vm).units.list[id.0];
    let has = u.power_type == pt || (u.class == "DRUID" && pt == 0.0);
    Ok(vec![Value::Bool(has)])
}

/// `UnitPowerMissing(unit [, powerType])`: its maximum less its power, as
/// `UnitPower` and `UnitPowerMax` give them (they ignore the power type
/// too); no unit 0.
fn unit_power_missing(vm: &mut Vm, c: NativeCall) -> Ret {
    let missing = units::unit_arg(vm, &c, 1).map_or(0.0, |id| {
        let u = &engine(vm).units.list[id.0];
        (u.power_max - u.power).max(0.0)
    });
    Ok(vec![Value::Num(missing)])
}

/// `UnitPowerDisplayMod(powerType)`: 1, soul shards 10 (Blizzard's
/// `ShardBar.lua`: `UnitPower` gives them tenfold).
fn unit_power_display_mod(vm: &mut Vm, c: NativeCall) -> Ret {
    let pt = num(vm, c.arg(1)).unwrap_or(0.0);
    Ok(vec![Value::Num(if pt == 7.0 { 10.0 } else { 1.0 })])
}

/// `UnitIsPVPSanctuary([unit])`: in a sanctuary zone (`C_PvP.GetZonePVPInfo`'s
/// type `sanctuary`), the unit where the player is.
fn unit_is_pvp_sanctuary(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = unit_or_player(vm, &c, 1) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let e = engine(vm);
    let position = &e.world.character.position;
    let sanctuary = position
        .pvp
        .as_ref()
        .is_some_and(|(kind, _, _)| kind == "sanctuary");
    let here = e.units.list[id.0]
        .zone
        .as_ref()
        .is_none_or(|z| *z == position.zone);
    Ok(vec![Value::Bool(sanctuary && here)])
}

/// `UnitHasRelicSlot(unit)`: the classes whose ranged slot holds a relic
/// (recorded: a paladin's true; druids and shamans unverified).
fn unit_has_relic_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    let relic = units::unit_arg(vm, &c, 1).is_some_and(|id| {
        let u = &engine(vm).units.list[id.0];
        u.player && matches!(u.class.as_str(), "PALADIN" | "DRUID" | "SHAMAN")
    });
    Ok(vec![Value::Bool(relic)])
}

/// `UnitUsesAmmo(unit)`: a hunter (recorded: a paladin's false; the other
/// classes unverified, a warrior's or rogue's bow not looked at).
fn unit_uses_ammo(vm: &mut Vm, c: NativeCall) -> Ret {
    let ammo = units::unit_arg(vm, &c, 1).is_some_and(|id| {
        let u = &engine(vm).units.list[id.0];
        u.player && u.class == "HUNTER"
    });
    Ok(vec![Value::Bool(ammo)])
}

/// `UnitTrialBankedLevels(unit)`: none, no experience banked (recorded:
/// 0, 0, 0).
fn unit_trial_banked_levels(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 3])
}

/// The level difference within which a lower level's content is still
/// worth it: classic's grey level (at most 5: 0; to 39: the level less 5
/// and a tenth; above: the level less 1 and a fifth), the range up to it
/// (unverified: classic's experience rules applied to quests).
fn trivial_range(level: f64) -> f64 {
    let grey = if level <= 5.0 {
        0.0
    } else if level <= 39.0 {
        level - 5.0 - (level / 10.0).floor()
    } else {
        level - 1.0 - (level / 5.0).floor()
    };
    (level - grey - 1.0).max(0.0)
}

/// `UnitQuestTrivialLevelRange(unit)`, `…Scaling`: the unit's
/// [`trivial_range`] (Blizzard's `DifficultyUtil`: green while the level
/// difference is within it).
fn unit_quest_trivial_level_range(vm: &mut Vm, c: NativeCall) -> Ret {
    let level = units::unit_arg(vm, &c, 1).map_or(engine(vm).units.list[PLAYER.0].level, |id| {
        engine(vm).units.list[id.0].level
    });
    Ok(vec![Value::Num(trivial_range(level))])
}

/// `C_PlayerInfo.GetContentDifficultyCreatureForPlayer(unit)`: the
/// `RelativeContentDifficulty` of its level against the player's, as
/// Blizzard's `DifficultyUtil.GetRelativeDifficultyColor` colors it
/// (5 above impossible, 3 above difficult, down to 4 below fair, within
/// the trivial range easy, below trivial). Recorded (the user's manual
/// probe, 2026-10-19): a mob at the player's level 2 (fair), a green one
/// below it 1 (easy), grey ones far below 0 (trivial), neutral or
/// hostile alike: by the level alone. The difficult and impossible bands
/// unverified.
fn content_difficulty_creature(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = units::unit_arg(vm, &c, 1) else {
        return Ok(vec![Value::Num(2.0)]);
    };
    let list = &engine(vm).units.list;
    let mine = list[PLAYER.0].level;
    let diff = list[id.0].level - mine;
    let d = if diff >= 5.0 {
        4.0
    } else if diff >= 3.0 {
        3.0
    } else if diff >= -4.0 {
        2.0
    } else if -diff <= trivial_range(mine) {
        1.0
    } else {
        0.0
    };
    Ok(vec![Value::Num(d)])
}

/// `UnitIsPlayerControlledOrGroupMember(unit)`: a player, the player's
/// pet, a group member.
fn unit_is_player_controlled_or_group_member(vm: &mut Vm, c: NativeCall) -> Ret {
    let controlled = units::unit_arg(vm, &c, 1).is_some_and(|id| {
        let u = &engine(vm).units;
        u.list[id.0].player || u.pet == Some(id) || ours(vm, id)
    });
    Ok(vec![Value::Bool(controlled)])
}

/// `UnitIsOwnerOrControllerOfUnit(controlling, controlled)`: the player
/// and its pet.
fn unit_is_owner_or_controller(vm: &mut Vm, c: NativeCall) -> Ret {
    let a = units::unit_arg(vm, &c, 1);
    let b = units::unit_arg(vm, &c, 2);
    let pet = engine(vm).units.pet;
    Ok(vec![Value::Bool(
        a == Some(PLAYER) && b.is_some() && b == pet,
    )])
}

/// `UnitOwnerGUID(unit)`: the pet's owner, the player; none for others.
fn unit_owner_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = units::unit_arg(vm, &c, 1);
    let u = &engine(vm).units;
    if id.is_some() && id == u.pet {
        let guid = u.list[PLAYER.0].guid.clone();
        return Ok(vec![vm.str(&guid)]);
    }
    Ok(vec![Value::Nil])
}

/// `UnitCreatureID(unit)`: a creature's or pet's NPC ID, its GUID's sixth
/// part (`Creature-0-4621-0-1-<npc>-<spawn>`); none for players.
fn unit_creature_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = units::unit_arg(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    let guid = &engine(vm).units.list[id.0].guid;
    let mut parts = guid.split('-');
    let kind = parts.next().unwrap_or("");
    let npc = parts.nth(4).and_then(|p| p.parse::<f64>().ok());
    Ok(vec![match (kind, npc) {
        ("Creature" | "Vehicle" | "Pet", Some(n)) => Value::Num(n),
        _ => Value::Nil,
    }])
}

/// `UnitSexBase(unit)`: its sex as `Enum.UnitSex` (unverified: the
/// documentation's type; `UnitSex` gives 2 male, 3 female).
fn unit_sex_base(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = units::unit_arg(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    let sex = engine(vm).units.list[id.0].sex;
    Ok(vec![Value::Num(sex_enum(sex))])
}

/// `UnitHasMouseoverHighlight(unit)`: the unit under the mouse.
fn unit_has_mouseover_highlight(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = units::unit_arg(vm, &c, 1);
    Ok(vec![Value::Bool(
        id.is_some() && id == engine(vm).units.mouseover,
    )])
}

/// `UnitIsInteractable([unit])`: the soft interact target or the NPC
/// talked to.
fn unit_is_interactable(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = units::unit_arg(vm, &c, 1);
    let u = &engine(vm).units;
    Ok(vec![Value::Bool(
        id.is_some() && (id == u.soft[2] || id == u.npc),
    )])
}

/// `UnitInPartyShard(unit)`: the player and its group share a shard.
fn unit_in_party_shard(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = units::unit_arg(vm, &c, 1);
    Ok(vec![Value::Bool(id.is_some_and(|id| ours(vm, id)))])
}

/// `UnitRealmRelationship(unit)`: a player on the player's realm
/// `LE_REALM_RELATION_SAME` (1), else connected (3); none for creatures
/// (warcraft.wiki.gg).
fn unit_realm_relationship(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = units::unit_arg(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    if !engine(vm).units.list[id.0].player {
        return Ok(vec![Value::Nil]);
    }
    let r = if same_realm(vm, id) { 1.0 } else { 3.0 };
    Ok(vec![Value::Num(r)])
}

/// `UnitIsSameServer(unit)`: a unit on the player's realm.
fn unit_is_same_server(vm: &mut Vm, c: NativeCall) -> Ret {
    let same = units::unit_arg(vm, &c, 1).is_some_and(|id| same_realm(vm, id));
    Ok(vec![Value::Bool(same)])
}

/// `UnitIsInMyGuild(unit)`: the player itself in a guild (the engine
/// keeps no other guild members).
fn unit_is_in_my_guild(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::unit_arg(vm, &c, 1) == Some(PLAYER);
    Ok(vec![Value::Bool(me && engine(vm).world.chat.guild)])
}

/// `IsPlayerInGuildFromGUID(guid)`: the player's own GUID while in a
/// guild.
fn is_player_in_guild_from_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = by_guid(vm, c.arg(1)) == Some(PLAYER);
    Ok(vec![Value::Bool(me && engine(vm).world.chat.guild)])
}

/// `IsPlayerNeutral()`: a player of no faction (a creature's `Neutral`
/// faction group only; the player's race always has one here).
fn is_player_neutral(vm: &mut Vm, _c: NativeCall) -> Ret {
    let neutral = engine(vm).units.list[PLAYER.0].faction_group.as_deref() == Some("Neutral");
    Ok(vec![Value::Bool(neutral)])
}

/// `IsStealthed()`: the player under a stealth aura (by its name: Stealth,
/// Prowl, Shadowmeld; unverified, the client's own flag).
fn is_stealthed(vm: &mut Vm, _c: NativeCall) -> Ret {
    let stealthed = engine(vm).units.auras.iter().any(|a| {
        a.unit == PLAYER
            && !a.harmful
            && matches!(a.name.as_str(), "Stealth" | "Prowl" | "Shadowmeld")
    });
    Ok(vec![Value::Bool(stealthed)])
}

/// `IsPlayerMoving()`: the player moving (`player:setMoving`; recorded
/// false standing).
fn is_player_moving(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).units.casting.moving)])
}

/// `GetPlayerFacing()`: north (0) in the open world, none in an instance
/// (warcraft.wiki.gg: restricted there; the engine keeps no facing).
fn get_player_facing(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine(vm).world.encounters.instance.is_some() {
        return Ok(vec![Value::Nil]);
    }
    Ok(vec![Value::Num(0.0)])
}

/// `GetRaidDifficultyID()`: the one `SetRaidDifficultyID` chose
/// (`content`), else none, 0 (recorded at load; the dungeon's is 1).
fn get_raid_difficulty_id(vm: &mut Vm, _c: NativeCall) -> Ret {
    let id = crate::content::raid_difficulty(vm).unwrap_or(0.0);
    Ok(vec![Value::Num(id)])
}

/// `GetComboPoints(unit, target)`: none (recorded: 0 for the player and
/// target; the engine keeps no combo points).
fn get_combo_points(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `GetShapeshiftFormCooldown(index)`: no cooldown: start 0, duration 0,
/// enabled (warcraft.wiki.gg's shape; Blizzard's `StanceBar.lua` passes it
/// to `CooldownFrame_Set`).
fn get_shapeshift_form_cooldown(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0), Value::Num(1.0)])
}

/// `UnitVehicleSkin([unit])`: no vehicle, none (Blizzard's
/// `ActionBarController.lua` tests it for nil or empty).
fn unit_vehicle_skin(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

// --------------------------------------------------------------- group

/// `UnitInAnyGroup([unit [, partyIndex]])`: grouped, the player or a
/// member.
fn unit_in_any_group(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_or_player(vm, &c, 1);
    let grouped = crate::party::grouped(vm);
    Ok(vec![Value::Bool(
        grouped && id.is_some_and(|id| ours(vm, id)),
    )])
}

/// `UnitInSubgroup([unit [, partyIndex]])`: grouped, the player or a
/// member of the player's subgroup (a `partyN` token's).
fn unit_in_subgroup(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_or_player(vm, &c, 1);
    let grouped = crate::party::grouped(vm);
    let inside = id.is_some_and(|id| {
        id == PLAYER
            || units::tokens_of(&engine(vm).units, id)
                .iter()
                .any(|t| t.starts_with("party"))
    });
    Ok(vec![Value::Bool(grouped && inside)])
}

/// `UnitLeadsAnyGroup(unit)`: the player leading its group (the engine
/// knows no other leader).
fn unit_leads_any_group(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::unit_arg(vm, &c, 1) == Some(PLAYER);
    let leads = me && crate::party::grouped(vm) && engine(vm).world.party.leader;
    Ok(vec![Value::Bool(leads)])
}

/// `Enum.LFGRole` of a role's name; none -1 (recorded for the player
/// alone: -1).
fn role_enum(role: Option<&str>) -> f64 {
    match role {
        Some("TANK") => 0.0,
        Some("HEALER") => 1.0,
        Some("DAMAGER") => 2.0,
        _ => -1.0,
    }
}

/// `UnitGroupRolesAssignedEnum([unit])`: `UnitGroupRolesAssigned`'s role
/// as `Enum.LFGRole`.
fn unit_group_roles_assigned_enum(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = unit_or_player(vm, &c, 1);
    let grouped = crate::party::grouped(vm);
    let role = id
        .filter(|&id| grouped && ours(vm, id))
        .and_then(|id| engine(vm).units.list[id.0].role.clone());
    Ok(vec![Value::Num(role_enum(role.as_deref()))])
}

/// A group member's role chosen (`GROUP_ROSTER_UPDATE`, as `unit:setRole`
/// tells it); false outside a group.
fn set_role(vm: &mut Vm, id: Option<UnitId>, role: Option<String>) -> Ret {
    let Some(id) = id.filter(|&id| crate::party::grouped(vm) && ours(vm, id)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    engine_mut(vm).units.list[id.0].role = role.filter(|r| r.as_str() != "NONE");
    widgets::fire_event(vm, "GROUP_ROSTER_UPDATE", &[]);
    Ok(vec![Value::Bool(true)])
}

/// `UnitSetRole(unit, roleStr)`.
fn unit_set_role(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = units::unit_arg(vm, &c, 1);
    let role = text(vm, c.arg(2));
    set_role(vm, id, role)
}

/// `UnitSetRoleEnum(unit, role)`: `Enum.LFGRole`.
fn unit_set_role_enum(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = units::unit_arg(vm, &c, 1);
    let role = match num(vm, c.arg(2)).map(|n| n as i64) {
        Some(0) => Some("TANK".to_string()),
        Some(1) => Some("HEALER".to_string()),
        Some(2) => Some("DAMAGER".to_string()),
        _ => None,
    };
    set_role(vm, id, role)
}

// ----------------------------------------------------------- by GUID

/// `GetPlayerInfoByGUID(guid)`: a player's class, race, sex, name, realm
/// (empty on the player's) and level; nothing for creatures or GUIDs the
/// world lacks (warcraft.wiki.gg; unverified: the client's cache of
/// players seen).
fn get_player_info_by_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = by_guid(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let realm = realm_text(vm, id);
    let u = &engine(vm).units.list[id.0];
    if !u.player {
        return Ok(vec![]);
    }
    let (class_name, class, race, sex, name, level) = (
        u.class_name.clone(),
        u.class.clone(),
        u.race.clone(),
        u.sex,
        u.name.clone(),
        u.level,
    );
    let (race_file, _) = units::race_file_and_id(&race);
    let race_name = units::race_name(&race);
    Ok(vec![
        vm.str(&class_name),
        vm.str(&class),
        vm.str(&race_name),
        vm.str(&race_file),
        Value::Num(sex),
        vm.str(&name),
        vm.str(&realm),
        Value::Num(level),
    ])
}

/// `UnitClassFromGUID(guid)`: a player's class's name, file and ID;
/// nothing for creatures (unverified).
fn unit_class_from_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = by_guid(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let u = &engine(vm).units.list[id.0];
    if !u.player {
        return Ok(vec![]);
    }
    let (name, class, class_id) = (u.class_name.clone(), u.class.clone(), u.class_id);
    Ok(vec![vm.str(&name), vm.str(&class), Value::Num(class_id)])
}

/// `UnitNameFromGUID(guid)`: the unit's name and realm (empty on the
/// player's); nothing for a GUID the world lacks.
fn unit_name_from_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = by_guid(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let realm = realm_text(vm, id);
    let name = engine(vm).units.list[id.0].name.clone();
    Ok(vec![vm.str(&name), vm.str(&realm)])
}

/// `UnitPercentHealthFromGUID(guid)`: its health's fraction of its
/// maximum (0 to 1, as `UnitHealthPercent` computes it); nil unknown.
fn unit_percent_health_from_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = by_guid(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let u = &engine(vm).units.list[id.0];
    let pct = if u.health_max > 0.0 {
        u.health / u.health_max
    } else {
        0.0
    };
    Ok(vec![Value::Num(pct)])
}

/// `C_PlayerInfo.GUIDIsPlayer(guid)`: a player's GUID (`Player-…`).
fn guid_is_player(vm: &mut Vm, c: NativeCall) -> Ret {
    let player = text(vm, c.arg(1)).is_some_and(|g| g.starts_with("Player-"));
    Ok(vec![Value::Bool(player)])
}

// -------------------------------------------------------- C_PlayerInfo

/// `C_PlayerInfo.GetClass(playerLocation)`: the class's name, file, ID.
fn player_info_get_class(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = located(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil, Value::Nil, Value::Nil]);
    };
    let u = &engine(vm).units.list[id.0];
    let (name, class, class_id) = (u.class_name.clone(), u.class.clone(), u.class_id);
    Ok(vec![vm.str(&name), vm.str(&class), Value::Num(class_id)])
}

/// `C_PlayerInfo.GetName(playerLocation)`: the name.
fn player_info_get_name(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = located(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let name = engine(vm).units.list[id.0].name.clone();
    Ok(vec![vm.str(&name)])
}

/// `C_PlayerInfo.GetRace(playerLocation)`: the race's ID.
fn player_info_get_race(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = located(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let race = engine(vm).units.list[id.0].race.clone();
    Ok(vec![
        units::race_file_and_id(&race)
            .1
            .map_or(Value::Nil, Value::Num),
    ])
}

/// `C_PlayerInfo.GetSex(playerLocation)`: `Enum.UnitSex`.
fn player_info_get_sex(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = located(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let sex = engine(vm).units.list[id.0].sex;
    Ok(vec![Value::Num(sex_enum(sex))])
}

/// `C_PlayerInfo.IsConnected([playerLocation])`: logged in; nil for a
/// location naming no one.
fn player_info_is_connected(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = if vm.hidden(c.arg(1)).is_nil() {
        Some(PLAYER)
    } else {
        located(vm, c.arg(1))
    };
    Ok(vec![id.map_or(Value::Nil, |id| {
        Value::Bool(engine(vm).units.list[id.0].online)
    })])
}

/// `C_PlayerInfo.UnitIsSameServer(playerLocation)`.
fn player_info_unit_is_same_server(vm: &mut Vm, c: NativeCall) -> Ret {
    let same = located(vm, c.arg(1)).is_some_and(|id| same_realm(vm, id));
    Ok(vec![Value::Bool(same)])
}

/// `C_PlayerInfo.GetDisplayID()`, `GetNativeDisplayID()`: the recorded
/// character's model, 54041 (recorded both; unverified for other races).
fn get_display_id(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(54041.0)])
}

/// `C_PlayerInfo.GetPlayerCharacterData()`: the player's race's name and
/// file and its sex (`Enum.UnitSex`), no alternate form, no atlas known.
fn get_player_character_data(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (race, sex) = {
        let u = &engine(vm).units.list[PLAYER.0];
        (u.race.clone(), u.sex)
    };
    let (file, _) = units::race_file_and_id(&race);
    let name = units::race_name(&race);
    let t = vm.new_table();
    for (k, v) in [
        ("name", vm.str(&name)),
        ("fileName", vm.str(&file)),
        ("createScreenIconAtlas", vm.str("")),
        ("sex", Value::Num(sex_enum(sex))),
    ] {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `C_PlayerInfo.GetGlidingInfo()`: not gliding, cannot, speed 0 (no
/// skyriding on WoW Forever).
fn get_gliding_info(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Bool(false),
        Value::Bool(false),
        Value::Num(0.0),
    ])
}

/// `C_PlayerInfo.GetPlayerMythicPlusRatingSummary(unit)`: an empty summary
/// (no mythic+ on WoW Forever).
fn get_mythic_plus_rating_summary(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![structure_default(vm, "MythicPlusRatingSummary")])
}

// ----------------------------------------------------------------- pet

/// `IsPetActive()`: the player has a pet.
fn is_pet_active(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).units.pet.is_some())])
}

/// `PetCanBeAbandoned()`: a hunter's pet (Blizzard's unit menu offers
/// abandoning it; others are dismissed).
fn pet_can_be_abandoned(vm: &mut Vm, _c: NativeCall) -> Ret {
    let e = engine(vm);
    Ok(vec![Value::Bool(
        e.units.pet.is_some() && e.world.pets.hunter,
    )])
}

/// A pet's stat (`pet:setStats`, the pet's own as `stats.rs` keeps them):
/// its first number; 0 without a pet or the stat.
fn pet_stat(vm: &Vm, key: &str) -> f64 {
    use crate::stats::{Answer, Entry};
    if engine(vm).units.pet.is_none() {
        return 0.0;
    }
    let first = |answers: &Vec<Answer>| match answers.first() {
        Some(Answer::Num(n)) => Some(*n),
        _ => None,
    };
    let n = match engine(vm).world.pets.stats.get(key) {
        Some(Entry::One(v)) => first(v),
        Some(Entry::ByIndex(m)) => m.values().find_map(first),
        None => None,
    };
    n.unwrap_or(0.0)
}

/// `GetPetCritChance()`: the pet's melee and spell crit.
fn get_pet_crit_chance(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Num(pet_stat(vm, "crit")),
        Value::Num(pet_stat(vm, "spellCrit")),
    ])
}

macro_rules! pet_stats {
    ($($name:ident: $key:literal;)*) => {
        $(
            fn $name(vm: &mut Vm, _c: NativeCall) -> Ret {
                Ok(vec![Value::Num(pet_stat(vm, $key))])
            }
        )*
    };
}

pet_stats! {
    get_pet_dodge_chance: "dodge";
    get_pet_hit_chance_modifier: "hit";
    get_pet_spell_hit_chance_modifier: "spellHit";
    get_pet_melee_haste: "meleeHaste";
    get_pet_spell_bonus_damage: "spellDamage";
}

// --------------------------------------------------------------- auras

/// The aura with this instance ID on the unit argument `n` names.
fn aura_at(vm: &Vm, c: &NativeCall, n: usize) -> Option<auras::Aura> {
    let unit = units::unit_arg(vm, c, n)?;
    let id = num(vm, c.arg(n + 1))?;
    engine(vm)
        .units
        .auras
        .iter()
        .find(|a| a.unit == unit && a.instance == id)
        .cloned()
}

/// `C_UnitAuras.GetUnitAuras(unit, filter [, maxCount])`: the unit's
/// auras passing the filter as `AuraData`, in application order (recorded:
/// the player's helpful auras listed, harmful none; unverified: the sort
/// rules).
fn get_unit_auras(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "GetUnitAuras")?;
    let filter = text(vm, c.arg(2)).unwrap_or_default();
    let max = num(vm, c.arg(3)).map(|n| n.max(0.0) as usize);
    let list = match units::unit_arg(vm, &c, 1) {
        Some(unit) => auras::listed(vm, unit, &filter),
        None => Vec::new(),
    };
    let values: Vec<Value> = list
        .iter()
        .take(max.unwrap_or(usize::MAX))
        .map(|a| auras::aura_data(vm, a))
        .collect();
    Ok(vec![list_of(vm, &values)])
}

/// `C_UnitAuras.GetUnitAuraInstanceIDs(unit, filter [, maxCount])`: the
/// instance IDs of the unit's auras that pass the filter.
fn get_unit_aura_instance_ids(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "GetUnitAuraInstanceIDs")?;
    let filter = text(vm, c.arg(2)).unwrap_or_default();
    let max = num(vm, c.arg(3)).map(|n| n.max(0.0) as usize);
    let ids: Vec<Value> = match units::unit_arg(vm, &c, 1) {
        Some(unit) => auras::listed(vm, unit, &filter)
            .iter()
            .take(max.unwrap_or(usize::MAX))
            .map(|a| Value::Num(a.instance))
            .collect(),
        None => Vec::new(),
    };
    Ok(vec![list_of(vm, &ids)])
}

/// `C_UnitAuras.GetUnitAuraBySpellID(unit, spellID)`: the unit's first
/// aura of the spell.
fn get_unit_aura_by_spell_id(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "GetUnitAuraBySpellID")?;
    let (Some(unit), Some(spell)) = (units::unit_arg(vm, &c, 1), num(vm, c.arg(2))) else {
        return Ok(vec![Value::Nil]);
    };
    let found = engine(vm)
        .units
        .auras
        .iter()
        .find(|a| a.unit == unit && a.spell_id == spell)
        .cloned();
    Ok(vec![found.map_or(Value::Nil, |a| auras::aura_data(vm, &a))])
}

/// `C_UnitAuras.IsAuraFilteredOutByInstanceID(unit, auraInstanceID,
/// filter)`: the aura fails the filter (an aura the unit lacks too).
fn is_aura_filtered_out(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "IsAuraFilteredOutByInstanceID")?;
    let filter = text(vm, c.arg(3)).unwrap_or_default();
    let out = aura_at(vm, &c, 1).is_none_or(|a| !auras::matches(&a, &filter));
    Ok(vec![Value::Bool(out)])
}

/// `C_UnitAuras.GetAuraCasterGUID(unit, auraInstanceID)`: its caster's
/// GUID.
fn get_aura_caster_guid(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "GetAuraCasterGUID")?;
    let guid = aura_at(vm, &c, 1)
        .and_then(|a| a.source)
        .map(|s| engine(vm).units.list[s.0].guid.clone());
    Ok(vec![guid.map_or(Value::Nil, |g| vm.str(&g))])
}

/// `C_UnitAuras.DoesAuraHaveExpirationTime(unit, auraInstanceID)`.
fn does_aura_have_expiration_time(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "DoesAuraHaveExpirationTime")?;
    let has = aura_at(vm, &c, 1).is_some_and(|a| a.expiration > 0.0);
    Ok(vec![Value::Bool(has)])
}

/// `C_UnitAuras.GetAuraBaseDuration(unit, auraInstanceID [, spellID])`:
/// its duration.
fn get_aura_base_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "GetAuraBaseDuration")?;
    Ok(vec![
        aura_at(vm, &c, 1).map_or(Value::Nil, |a| Value::Num(a.duration)),
    ])
}

/// `C_UnitAuras.GetAuraApplicationDisplayCount(unit, auraInstanceID
/// [, min [, max]])`: its stacks as text, empty below the minimum (2),
/// `*` above the maximum (the documentation).
fn get_aura_application_display_count(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "GetAuraApplicationDisplayCount")?;
    let min = num(vm, c.arg(3)).unwrap_or(2.0);
    let max = num(vm, c.arg(4));
    let count = aura_at(vm, &c, 1).map_or(0.0, |a| a.applications);
    let shown = if count < min {
        String::new()
    } else if max.is_some_and(|m| count > m) {
        "*".to_string()
    } else {
        format!("{}", count as i64)
    };
    Ok(vec![vm.str(&shown)])
}

/// `C_UnitAuras.GetAuraDuration(unit, auraInstanceID)`: a duration object
/// from its start to its expiration (none without the aura).
fn get_aura_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "GetAuraDuration")?;
    let Some(a) = aura_at(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let ns = vm.get_global("C_DurationUtil");
    let Value::Table(ns) = ns else {
        return Ok(vec![]);
    };
    let k = vm.str("CreateDuration");
    let create = vm.heap.table(ns).get(k);
    let Some(d) = vm.call(create, &[])?.first().copied() else {
        return Ok(vec![]);
    };
    if a.expiration > 0.0 {
        let k = vm.str("SetTimeFromStart");
        let set = vm.index(d, k, None)?;
        let start = a.expiration - a.duration;
        vm.call(set, &[d, Value::Num(start), Value::Num(a.duration)])?;
    }
    Ok(vec![d])
}

/// `C_UnitAuras.GetAuraDispelTypeColor(unit, auraInstanceID, curve)`: the
/// curve at the aura's dispel type (none 0, magic 1, curse 2, disease 3,
/// poison 4; unverified: the client's numbering).
fn get_aura_dispel_type_color(vm: &mut Vm, c: NativeCall) -> Ret {
    auras::refuse(vm, "GetAuraDispelTypeColor")?;
    let Some(a) = aura_at(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let kind = match a.dispel_name.as_deref() {
        Some("Magic") => 1.0,
        Some("Curse") => 2.0,
        Some("Disease") => 3.0,
        Some("Poison") => 4.0,
        _ => 0.0,
    };
    Ok(vec![crate::curves::apply(vm, c.arg(3), kind)?])
}

/// `C_UnitAuras.CancelAuraByInstanceID(unit, auraInstanceID)`: the
/// player's helpful aura removed (`UNIT_AURA`); others stay.
fn cancel_aura_by_instance_id(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(a) = aura_at(vm, &c, 1)
        && a.unit == PLAYER
        && !a.harmful
    {
        auras::remove(vm, PLAYER, a.instance);
    }
    Ok(vec![])
}

/// `C_UnitAuras.SetHiddenGroupBuffs(spellIDs)`: accepted, the class's
/// hidden group buffs as they were (the cooldown viewer sets its layout's
/// at load; replacing the class's told a `HIDDEN_GROUP_BUFFS_CHANGED` the
/// recordings lack; unverified what the client keeps).
fn set_hidden_group_buffs(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `C_UnitAuras.AddAuraSound(trigger, sound)` (SuuUI probe sound, one
/// session): a sound with a `soundFileID` or a `soundFileName` (an
/// `outputChannel` or not) is registered under the session's next ID,
/// counting from 1 (removed ones not reused); one with neither (a
/// `soundKitID`, unknown fields) answers nil. The sound's fields are
/// checked in the documentation's order: no `unitToken`, or a `spellID`
/// that is not a number, raise the recorded "bad argument #3" errors.
/// Unverified: other fields' errors, a missing `spellID`.
fn add_aura_sound(vm: &mut Vm, c: NativeCall) -> Ret {
    const USAGE: &str = "Usage: local auraSoundID = C_UnitAuras.AddAuraSound(trigger, sound)";
    let Value::Table(sound) = vm.hidden(c.arg(2)) else {
        return Ok(vec![Value::Nil]);
    };
    let field = |vm: &mut Vm, k: &str| {
        let key = vm.str(k);
        let v = vm.heap.table(sound).get(key);
        vm.hidden(v)
    };
    if field(vm, "unitToken").is_nil() {
        let msg = format!("bad argument #3 to '?' (Current Field: [unitToken], {USAGE})");
        return Err(widgets::c_error(vm, &msg, false));
    }
    let spell = field(vm, "spellID");
    if !spell.is_nil() && !matches!(spell, Value::Num(_)) {
        let msg = format!(
            "bad argument #3 to '?' (Current Field: [spellID], outside of expected range -2147483648 to 2147483647 - {USAGE})"
        );
        return Err(widgets::c_error(vm, &msg, false));
    }
    if field(vm, "soundFileID").is_nil() && field(vm, "soundFileName").is_nil() {
        return Ok(vec![Value::Nil]);
    }
    let registry = vm.registry;
    let key = vm.heap.intern(b"wowlua.engine.auraSounds");
    let last = match vm.heap.table(registry).get_str(key) {
        Value::Num(n) => n,
        _ => 0.0,
    };
    vm.heap
        .table_mut(registry)
        .set_str(key, Value::Num(last + 1.0));
    Ok(vec![Value::Num(last + 1.0)])
}

// ------------------------------------------------------- raid profiles

/// `GetRaidProfileFlattenedOptions(profile)`: no options (no raid profile
/// is kept: `GetNumRaidProfiles` answers none).
fn raid_profile_options(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

/// `C_NamePlate.GetNamePlateSize()`: 237.5 by 85 (recorded with the
/// character's settings; unverified: how the nameplate CVars change it,
/// `SetNamePlateSize` is not kept).
fn get_name_plate_size(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(237.5), Value::Num(85.0)])
}

/// `C_PlayerInfo.IsDisplayRaceNative()`, `CanUseItem(itemID)` and the
/// character bank: true (no alternate forms; usability not modelled; the
/// engine has a bank, `bank.rs`).
fn native(vm: &mut Vm, c: NativeCall) -> Ret {
    yes(vm, c)
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        // Unit states.
        ("GetPlayerInfoByGUID", get_player_info_by_guid),
        ("UnitClassFromGUID", unit_class_from_guid),
        ("UnitNameFromGUID", unit_name_from_guid),
        ("UnitPercentHealthFromGUID", unit_percent_health_from_guid),
        ("UnitSelectionType", unit_selection_type),
        ("UnitHasPowerType", unit_has_power_type),
        ("UnitPowerMissing", unit_power_missing),
        ("UnitPowerDisplayMod", unit_power_display_mod),
        ("UnitIsPVPSanctuary", unit_is_pvp_sanctuary),
        ("UnitHasRelicSlot", unit_has_relic_slot),
        ("UnitUsesAmmo", unit_uses_ammo),
        ("UnitTrialBankedLevels", unit_trial_banked_levels),
        ("UnitQuestTrivialLevelRange", unit_quest_trivial_level_range),
        (
            "UnitQuestTrivialLevelRangeScaling",
            unit_quest_trivial_level_range,
        ),
        (
            "C_PlayerInfo.GetContentDifficultyCreatureForPlayer",
            content_difficulty_creature,
        ),
        (
            "UnitIsPlayerControlledOrGroupMember",
            unit_is_player_controlled_or_group_member,
        ),
        ("UnitIsOwnerOrControllerOfUnit", unit_is_owner_or_controller),
        ("UnitOwnerGUID", unit_owner_guid),
        ("UnitCreatureID", unit_creature_id),
        ("UnitSexBase", unit_sex_base),
        ("UnitHasMouseoverHighlight", unit_has_mouseover_highlight),
        ("UnitIsInteractable", unit_is_interactable),
        ("UnitInPartyShard", unit_in_party_shard),
        ("UnitRealmRelationship", unit_realm_relationship),
        ("UnitIsSameServer", unit_is_same_server),
        ("UnitIsInMyGuild", unit_is_in_my_guild),
        ("IsPlayerInGuildFromGUID", is_player_in_guild_from_guid),
        ("IsPlayerNeutral", is_player_neutral),
        ("IsStealthed", is_stealthed),
        ("IsPlayerMoving", is_player_moving),
        ("GetPlayerFacing", get_player_facing),
        ("GetRaidDifficultyID", get_raid_difficulty_id),
        ("GetComboPoints", get_combo_points),
        ("GetShapeshiftFormCooldown", get_shapeshift_form_cooldown),
        ("UnitVehicleSkin", unit_vehicle_skin),
        // States the engine does not model: none of them.
        ("UnitIsAFK", no),
        ("UnitIsDND", no),
        ("UnitIsFeignDeath", no),
        ("UnitIsCharmed", no),
        ("UnitIsMercenary", no),
        ("UnitIsCorpse", no),
        ("UnitIsUnconscious", no),
        ("UnitIsControlling", no),
        ("UnitTreatAsPlayerForDisplay", no),
        ("UnitIsNPCAsPlayer", no),
        ("UnitShouldDisplaySpellTargetName", no),
        ("UnitHasLFGDeserter", no),
        ("UnitHasLFGRandomCooldown", no),
        ("UnitHasEffectivelyTankAura", no),
        ("UnitIsLieutenant", no),
        ("UnitIsRaidOfficer", no),
        ("UnitAlliedRaceInfo", two_falses),
        ("UnitInBattleground", nil),
        ("UnitPvpClassification", nil),
        ("UnitThreatPercentageOfLead", nil),
        ("UnitThreatLeadSituation", nil),
        ("UnitSpellTargetName", nothing),
        ("UnitSpellTargetClass", nothing),
        ("UnitGetDetailedHealPrediction", nothing),
        ("UnitStagger", zero),
        ("UnitTrialXP", zero),
        ("UnitChromieTimeID", zero),
        ("UnitPartialPower", zero),
        ("UnitWeaponAttackPower", three_zeros),
        ("GetUnitHealthModifier", one),
        ("GetUnitMaxHealthModifier", one),
        ("GetUnitPowerModifier", one),
        // Vehicles: none on WoW Forever.
        ("UnitTargetsVehicleInRaidUI", no),
        ("UnitControllingVehicle", no),
        ("UnitUsingVehicle", no),
        ("UnitInVehicleControlSeat", no),
        ("UnitInVehicleHidesPetFrame", no),
        ("UnitVehicleSeatCount", zero),
        ("UnitVehicleSeatInfo", nothing),
        ("UnitSwitchToVehicleSeat", nothing),
        // Battle pets: none.
        ("UnitIsBattlePetCompanion", no),
        ("UnitIsBattlePet", no),
        ("UnitIsOtherPlayersBattlePet", no),
        ("UnitCanPetBattle", no),
        ("UnitBattlePetLevel", nil),
        ("UnitBattlePetType", nil),
        ("UnitBattlePetSpeciesID", nil),
        // Empowered casts, runes, charged points: none.
        ("UnitEmpoweredChannelDuration", nothing),
        ("UnitEmpoweredStagePercentages", nothing),
        ("UnitEmpoweredStageDurations", nothing),
        ("GetUnitEmpowerHoldAtMaxTime", zero),
        ("GetUnitEmpowerMinHoldTime", zero),
        ("GetUnitEmpowerStageDuration", zero),
        ("GetRuneCooldown", nothing),
        ("GetRuneCount", nothing),
        ("GetUnitChargedPowerPoints", nothing),
        // Alternate power bars: none (recorded: the player's timer
        // nothing).
        ("UnitPowerBarID", zero),
        ("UnitNumPowerBarTimers", zero),
        ("UnitPowerBarTimerInfo", nothing),
        ("GetUnitPowerBarInfoByID", nothing),
        ("GetUnitPowerBarStrings", three_nils),
        ("GetUnitPowerBarStringsByID", three_nils),
        ("GetUnitPowerBarTextureInfo", nothing),
        ("GetUnitPowerBarTextureInfoByID", nothing),
        // Group.
        ("UnitInAnyGroup", unit_in_any_group),
        ("UnitInSubgroup", unit_in_subgroup),
        ("UnitLeadsAnyGroup", unit_leads_any_group),
        ("UnitGroupRolesAssignedEnum", unit_group_roles_assigned_enum),
        ("UnitSetRole", unit_set_role),
        ("UnitSetRoleEnum", unit_set_role_enum),
        // `Enum.SummonStatus.None`.
        ("C_IncomingSummon.IncomingSummonStatus", zero),
        // C_PlayerInfo.
        ("C_PlayerInfo.GUIDIsPlayer", guid_is_player),
        ("C_PlayerInfo.GetClass", player_info_get_class),
        ("C_PlayerInfo.GetName", player_info_get_name),
        ("C_PlayerInfo.GetRace", player_info_get_race),
        ("C_PlayerInfo.GetSex", player_info_get_sex),
        ("C_PlayerInfo.IsConnected", player_info_is_connected),
        (
            "C_PlayerInfo.UnitIsSameServer",
            player_info_unit_is_same_server,
        ),
        ("C_PlayerInfo.GetDisplayID", get_display_id),
        ("C_PlayerInfo.GetNativeDisplayID", get_display_id),
        (
            "C_PlayerInfo.GetPlayerCharacterData",
            get_player_character_data,
        ),
        ("C_PlayerInfo.GetGlidingInfo", get_gliding_info),
        (
            "C_PlayerInfo.GetPlayerMythicPlusRatingSummary",
            get_mythic_plus_rating_summary,
        ),
        // Recorded: slots 8 and 10 true (the others unverified).
        ("C_PlayerInfo.HasVisibleInvSlot", yes),
        ("C_PlayerInfo.IsDisplayRaceNative", native),
        ("C_PlayerInfo.CanUseItem", native),
        ("C_PlayerInfo.IsCharacterBankEnabled", native),
        ("C_PlayerInfo.IsExpansionLandingPageUnlockedForPlayer", no),
        ("C_PlayerInfo.IsPlayerInChromieTime", no),
        ("C_PlayerInfo.CanPlayerEnterChromieTime", no),
        ("C_PlayerInfo.CanPlayerUseMountEquipment", no_with_reason),
        ("C_PlayerInfo.IsPlayerEligibleForNPE", no_with_reason),
        ("C_PlayerInfo.IsPlayerEligibleForNPEv2", no_with_reason),
        ("C_PlayerInfo.IsTravelersLogAvailable", no),
        ("C_PlayerInfo.IsTradingPostAvailable", no),
        ("C_PlayerInfo.IsReturningCharacter", no),
        ("C_PlayerInfo.IsTutorialsTabAvailable", no),
        ("C_PlayerInfo.IsMirrorImage", no),
        ("C_PlayerInfo.CanPlayerUseAreaLoot", no),
        ("C_PlayerInfo.HasAccountInventoryLock", no),
        ("C_PlayerInfo.IsAccountBankEnabled", no),
        ("C_PlayerInfo.IsPlayerInTimerunningHeroicWorldTier", no),
        ("C_PlayerInfo.IsSelfFoundActive", no),
        ("C_PlayerInfo.GetInstancesUnlockedAtLevel", empty_list),
        ("C_PlayerInfo.GetPetStableCreatureDisplayInfoID", zero),
        // The pet.
        ("IsPetActive", is_pet_active),
        ("PetCanBeAbandoned", pet_can_be_abandoned),
        ("GetPetTimeRemaining", nothing),
        ("GetPetCritChance", get_pet_crit_chance),
        ("GetPetDodgeChance", get_pet_dodge_chance),
        ("GetPetHitChanceModifier", get_pet_hit_chance_modifier),
        (
            "GetPetSpellHitChanceModifier",
            get_pet_spell_hit_chance_modifier,
        ),
        ("GetPetMeleeHaste", get_pet_melee_haste),
        ("GetPetSpellBonusDamage", get_pet_spell_bonus_damage),
        // Auras.
        ("C_UnitAuras.GetUnitAuras", get_unit_auras),
        (
            "C_UnitAuras.GetUnitAuraInstanceIDs",
            get_unit_aura_instance_ids,
        ),
        (
            "C_UnitAuras.GetUnitAuraBySpellID",
            get_unit_aura_by_spell_id,
        ),
        (
            "C_UnitAuras.IsAuraFilteredOutByInstanceID",
            is_aura_filtered_out,
        ),
        ("C_UnitAuras.GetAuraCasterGUID", get_aura_caster_guid),
        (
            "C_UnitAuras.DoesAuraHaveExpirationTime",
            does_aura_have_expiration_time,
        ),
        ("C_UnitAuras.GetAuraBaseDuration", get_aura_base_duration),
        (
            "C_UnitAuras.GetAuraApplicationDisplayCount",
            get_aura_application_display_count,
        ),
        ("C_UnitAuras.GetAuraDuration", get_aura_duration),
        (
            "C_UnitAuras.GetAuraDispelTypeColor",
            get_aura_dispel_type_color,
        ),
        (
            "C_UnitAuras.CancelAuraByInstanceID",
            cancel_aura_by_instance_id,
        ),
        ("C_UnitAuras.SetHiddenGroupBuffs", set_hidden_group_buffs),
        ("C_UnitAuras.GetRefreshCarryOverDuration", nil),
        ("C_UnitAuras.GetRefreshExtendedDuration", nil),
        ("C_UnitAuras.GetCooldownAuraBySpellID", nil),
        ("C_UnitAuras.AuraIsBigDefensive", no),
        ("C_UnitAuras.AuraIsPrivate", no),
        ("C_UnitAuras.WantsAlteredForm", no),
        // Private auras, aura sounds, blocked auras, the data provider:
        // accepted, nothing to show (no anchor made: nil).
        ("C_UnitAuras.AddPrivateAuraAnchor", nil),
        ("C_UnitAuras.AddAuraSound", add_aura_sound),
        ("C_UnitAuras.RemovePrivateAuraAnchor", nothing),
        ("C_UnitAuras.RemoveAuraSound", nothing),
        ("C_UnitAuras.RemovePrivateAuraAppliedSound", nothing),
        ("C_UnitAuras.SetPrivateWarningTextAnchor", nothing),
        ("C_UnitAuras.AddBlockedAura", nothing),
        ("C_UnitAuras.ClearBlockedAuras", nothing),
        ("C_UnitAuras.ResetAuraDataProvider", nothing),
        ("C_UnitAuras.SwitchAuraDataProvider", nothing),
        ("C_UnitAuras.SetGroupBuffVisualAlerts", nothing),
        // Loss of control: none.
        ("C_LossOfControl.GetActiveLossOfControlData", nil),
        ("C_LossOfControl.GetActiveLossOfControlDataCount", zero),
        (
            "C_LossOfControl.GetActiveLossOfControlDataCountByUnit",
            zero,
        ),
        ("C_LossOfControl.GetActiveLossOfControlDuration", nil),
        // Nameplates.
        ("C_NamePlate.SetNamePlateSize", nothing),
        ("C_NamePlate.GetNamePlateSize", get_name_plate_size),
        // Death: no release position known; no self resurrection.
        ("C_DeathInfo.GetDeathReleasePosition", nil),
        ("C_DeathInfo.UseSelfResurrectOption", nothing),
        // Petitions, trade currency, raid profiles: none.
        ("GetPetitionInfo", nothing),
        ("GetPetitionNameInfo", nothing),
        ("GetPlayerTradeCurrency", nothing),
        ("GetRaidProfileName", nil),
        ("GetRaidProfileOption", nil),
        ("GetRaidProfileSavedPosition", nothing),
        ("GetRaidProfileFlattenedOptions", raid_profile_options),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn trivial_ranges() {
        assert_eq!(super::trivial_range(60.0), 12.0);
        assert_eq!(super::trivial_range(20.0), 6.0);
        assert_eq!(super::trivial_range(3.0), 2.0);
    }

    #[test]
    fn sexes() {
        assert_eq!(super::sex_enum(2.0), 0.0);
        assert_eq!(super::sex_enum(3.0), 1.0);
        assert_eq!(super::sex_enum(1.0), 2.0);
    }
}
