//! A level gained, as the client recorded the warrior's second
//! (`/stdtest engine scenario quest`): `defineLevel(level, data)` tells
//! what the level brings; experience past the level's end (`UnitXPMax`)
//! levels the player up. With the experience's packet: the talents' and
//! the group buffs' updates, `PLAYER_LEVEL_UP(level, healthDelta,
//! powerDelta, numNewTalents, numNewPvpTalentSlots, strengthDelta,
//! agilityDelta, staminaDelta, intellectDelta, spiritDelta)`, the
//! specialization's and the spells' (the level still the old one); a frame
//! later the group buffs' again; 0.1 s later the rated PvP stats and the
//! achievements; and with the server's next packet (0.33 s later, as
//! recorded) the level itself: `PLAYER_LEVEL_CHANGED(old, new, true)`,
//! `UNIT_LEVEL`, the stats' updates, `PLAYER_XP_UPDATE` (the experience
//! left over, the new level's end), the skill lines' changes, the health
//! (its maximum grown by the level's `health`).
//!
//! Unverified: more than one level at once, a level with talents or
//! spells to learn, a pet's; the skill lines' changes' count (the
//! level's `skillLines`, 21 as recorded); the second packet's delay
//! beyond this recording.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::stats::{Answer, Entry};
use crate::units;
use crate::{engine, engine_mut, widgets};

/// The level's own packet after the experience's (recorded).
const LEVEL_AFTER: f64 = 0.329;
/// The rated PvP stats and the achievements after the experience's
/// packet (recorded).
const ACHIEVEMENTS_AFTER: f64 = 0.102;
/// The spell whose cooldown the level up tells (`SPELL_UPDATE_COOLDOWN`,
/// recorded for the warrior).
const COOLDOWN_SPELL: f64 = 341545.0;

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn num(vm: &mut Vm, t: TableRef, key: &str) -> f64 {
    match field(vm, t, key) {
        Value::Num(n) => n,
        _ => 0.0,
    }
}

/// `defineLevel(level, { xpMax, health, power, talents, pvpTalentSlots,
/// strength, agility, stamina, intellect, spirit, skillLines })`.
pub fn control_define(vm: &mut Vm, c: NativeCall) -> Ret {
    let level = vm.check_int(&c, 1)?;
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("defineLevel: a table of the level's data expected"));
    };
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm).world.levels.insert(level, t);
    Ok(vec![])
}

fn stat(vm: &Vm, key: &str) -> Option<f64> {
    match engine(vm).world.character.stats.get(key) {
        Some(Entry::One(a)) => match a.first() {
            Some(Answer::Num(n)) => Some(*n),
            _ => None,
        },
        _ => None,
    }
}

fn set_stat(vm: &mut Vm, key: &'static str, v: f64) {
    engine_mut(vm)
        .world
        .character
        .stats
        .insert(key, Entry::One(vec![Answer::Num(v)]));
}

/// Experience gained: whether it reaches a defined level, which then
/// takes it (`level_up`).
pub fn reaches(vm: &Vm, xp: f64) -> bool {
    let (Some(max), level) = (
        stat(vm, "xpMax"),
        engine(vm).units.list[units::PLAYER.0].level,
    ) else {
        return false;
    };
    xp >= max && engine(vm).world.levels.contains_key(&(level as i64 + 1))
}

/// The player levels up with `xp` in all (the level's end passed).
pub fn level_up(vm: &mut Vm, xp: f64) {
    let level = engine(vm).units.list[units::PLAYER.0].level + 1.0;
    let Some(t) = engine(vm).world.levels.get(&(level as i64)).copied() else {
        return;
    };
    fire(vm, "PLAYER_TALENT_UPDATE", &[]);
    group_buffs(vm);
    fire(vm, "PLAYER_PVP_TALENT_UPDATE", &[]);
    let group = stat(vm, "activeSpecGroup").unwrap_or(1.0);
    fire(
        vm,
        "ACTIVE_TALENT_GROUP_CHANGED",
        &[Value::Num(group), Value::Num(group)],
    );
    let mut args = vec![Value::Num(level)];
    for key in [
        "health",
        "power",
        "talents",
        "pvpTalentSlots",
        "strength",
        "agility",
        "stamina",
        "intellect",
        "spirit",
    ] {
        args.push(Value::Num(num(vm, t, key)));
    }
    fire(vm, "PLAYER_LEVEL_UP", &args);
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
    units::fire_unit(vm, "PLAYER_SPECIALIZATION_CHANGED", units::PLAYER, &[]);
    fire(
        vm,
        "SPELL_UPDATE_COOLDOWN",
        &[
            Value::Num(COOLDOWN_SPELL),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
        ],
    );
    group_buffs(vm);
    fire(vm, "SPELLS_CHANGED", &[]);
    fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
    let next = vm.native("levelBuffs", buffs_again as NativeFn);
    crate::clock::after_engine(vm, 1.0 / 60.0, next);
    let later = vm.native("levelAchievements", achievements as NativeFn);
    crate::clock::after_engine(vm, ACHIEVEMENTS_AFTER, later);
    let packet = vm.native_with(
        "levelGained",
        gained as NativeFn,
        vec![Value::Num(level), Value::Num(xp)],
    );
    crate::clock::after_engine(vm, LEVEL_AFTER, packet);
}

/// The group buffs' changes (`character::group_buffs_changed`).
fn group_buffs(vm: &mut Vm) {
    crate::character::group_buffs_changed(vm);
}

fn buffs_again(vm: &mut Vm, _c: NativeCall) -> Ret {
    group_buffs(vm);
    Ok(vec![])
}

fn achievements(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "PVP_RATED_STATS_UPDATE", &[]);
    fire(vm, "RECEIVED_ACHIEVEMENT_LIST", &[]);
    Ok(vec![])
}

/// The level's packet: the level, its stats, the experience left over.
fn gained(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(level), Value::Num(xp)) = (c.upvals[0], c.upvals[1]) else {
        return Ok(vec![]);
    };
    let Some(t) = engine(vm).world.levels.get(&(level as i64)).copied() else {
        return Ok(vec![]);
    };
    let old = engine(vm).units.list[units::PLAYER.0].level;
    let old_max = stat(vm, "xpMax").unwrap_or(xp);
    let health = num(vm, t, "health");
    // The health full at the new maximum (the packet's `UNIT_HEALTH`; no
    // aura state's change told, `quest` 70.006, the player full before;
    // unverified from a wounded player).
    {
        let p = &mut engine_mut(vm).units.list[units::PLAYER.0];
        p.health_max += health;
        p.health = p.health_max;
    }
    let player = units::PLAYER;
    units::fire_unit(vm, "UNIT_MAXHEALTH", player, &[]);
    units::fire_unit(vm, "UNIT_MAXHEALTH", player, &[]);
    fire(vm, "TREASURE_PICKER_CACHE_FLUSH", &[]);
    fire(vm, "PVPQUEUE_ANYWHERE_UPDATE_AVAILABLE", &[]);
    group_buffs(vm);
    fire(vm, "PLAYER_PVP_TALENT_UPDATE", &[]);
    let empty = vm.str("");
    fire(vm, "DISPLAY_EVENT_TOAST_LINK", &[empty]);
    fire(vm, "DISPLAY_EVENT_TOASTS", &[]);
    engine_mut(vm).units.list[units::PLAYER.0].level = level;
    set_stat(vm, "xp", xp - old_max);
    let max = num(vm, t, "xpMax");
    set_stat(vm, "xpMax", max);
    fire(
        vm,
        "PLAYER_LEVEL_CHANGED",
        &[Value::Num(old), Value::Num(level), Value::Bool(true)],
    );
    for (e, times) in [
        ("UNIT_LEVEL", 1),
        ("UNIT_DAMAGE", 2),
        ("UNIT_ATTACK_POWER", 1),
        ("UNIT_RANGED_ATTACK_POWER", 1),
        ("UNIT_RANGEDDAMAGE", 2),
        ("UNIT_STATS", 3),
        ("UNIT_RESISTANCES", 2),
    ] {
        for _ in 0..times {
            units::fire_unit(vm, e, player, &[]);
        }
    }
    fire(vm, "PLAYER_AVG_ITEM_LEVEL_UPDATE", &[]);
    units::fire_unit(vm, "PLAYER_XP_UPDATE", player, &[]);
    units::fire_unit(vm, "PLAYER_XP_UPDATE", player, &[]);
    let lines = match field(vm, t, "skillLines") {
        Value::Num(n) => n as usize,
        _ => 21,
    };
    for _ in 0..lines.saturating_sub(1) {
        fire(vm, "SKILL_LINES_CHANGED", &[]);
    }
    units::fire_unit(vm, "UNIT_ATTACK", player, &[]);
    units::fire_unit(vm, "UNIT_DEFENSE", player, &[]);
    units::fire_unit(vm, "UNIT_RANGED_ATTACK_POWER", player, &[]);
    if lines > 0 {
        fire(vm, "SKILL_LINES_CHANGED", &[]);
    }
    fire(vm, "MOUNT_JOURNAL_USABILITY_CHANGED", &[]);
    for _ in 0..6 {
        units::fire_unit(vm, "PLAYER_DAMAGE_DONE_MODS", player, &[]);
    }
    fire(vm, "LFG_LIST_AVAILABILITY_UPDATE", &[]);
    fire(
        vm,
        "HEIRLOOMS_UPDATED",
        &[Value::Nil, Value::Nil, Value::Nil],
    );
    fire(vm, "QUEST_LOG_UPDATE", &[]);
    fire(vm, "SKILL_LINE_SPECS_RANKS_CHANGED", &[]);
    group_buffs(vm);
    fire(vm, "SPELLS_CHANGED", &[]);
    units::fire_unit(vm, "UNIT_HEALTH", player, &[]);
    units::fire_unit(vm, "UNIT_MAXHEALTH", player, &[]);
    Ok(vec![])
}
