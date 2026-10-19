//! The player's death, as the client recorded the warrior's (`/stdtest
//! engine scenario death`, a ping near 0.3 s): `player:die({ durability
//! })`, the server's death: "Your equipped items suffer a 10% durability
//! loss.", auto attack off, the mirror timers stopped, the rage gone, the
//! target cleared, "You died." (a death link), `UNIT_DIED(guid)`,
//! `PLAYER_DEAD()`, out of combat, each worn item's durability a tenth
//! of its maximum lower (`UPDATE_INVENTORY_DURABILITY`), every action unusable. Blizzard's UI
//! shows its death popup; its Release (`RepopMe()`): a round trip later the
//! ghost (`PLAYER_ALIVE()`, the flags, the faction, the ghost's spell's
//! cooldown, its aura Ghost, the corpse tracked). `e.nearCorpse(near)`:
//! `CORPSE_IN_RANGE()` / `CORPSE_OUT_OF_RANGE()` (Blizzard's popup
//! offers the corpse); `RetrieveCorpse()`: a round trip later alive
//! (`PLAYER_UNGHOST()`, the ghost's aura removed: `UNIT_AURA`).
//!
//! Unverified: where the ghost appears, the health on
//! resurrection (half), the release timer counting down, spirit healers,
//! a soulstone, dying in an instance.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, PLAYER};
use crate::{engine, engine_mut, widgets};

/// The ghost's spell (its cooldown told at release, its aura on the
/// ghost: recorded).
const GHOST_SPELL: f64 = 8326.0;
const GHOST_ICON: f64 = 132331.0;
/// The death's durability loss, in percent (recorded message).
const DURABILITY_LOSS: f64 = 10.0;
/// The release's second packet after its first (recorded).
const RELEASED_AFTER: f64 = 0.031;
/// The time to release, in seconds (`GetReleaseTimeRemaining()` at the
/// death, recorded).
const RELEASE_TIME: f64 = 360.0;

#[derive(Default)]
pub struct Death {
    /// When the player died (the clock's seconds), while dead.
    pub died_at: Option<f64>,
    /// The ghost's hidden aura's instance (removed on resurrection).
    pub ghost_aura: f64,
    /// Deaths so far (the death link's number).
    pub deaths: u32,
    /// A resurrection offered to the player: by whom, the health it gives.
    pub offer: Option<(units::UnitId, f64)>,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn mirror_timers_stop(vm: &mut Vm) {
    for t in ["EXHAUSTION", "BREATH", "FEIGNDEATH"] {
        let t = vm.str(t);
        fire(vm, "MIRROR_TIMER_STOP", &[t]);
    }
}

/// `player:die({ durability = { [slot] = current }, ghostAura })`: the
/// server's death of the player.
pub fn control_die(vm: &mut Vm, c: NativeCall) -> Ret {
    if units::this(vm, &c)? != PLAYER {
        return Err(vm.error("die: the player's"));
    }
    // Each worn item loses a tenth of its maximum, rounded down (recorded:
    // 24, 16, 19, 20 of 25, 16, 20, 20 left 22, 15, 17, 18); the test may
    // give what is left instead.
    let mut durability: Vec<(i64, f64)> = engine(vm)
        .world
        .character
        .equipment
        .iter()
        .filter_map(|(slot, h)| {
            h.durability
                .map(|(d, m)| (*slot, (d - (m * DURABILITY_LOSS / 100.0).floor()).max(0.0)))
        })
        .collect();
    let mut ghost_aura = 0.0;
    if let Value::Table(opts) = c.arg(2) {
        let k = vm.str("durability");
        if let Value::Table(d) = vm.heap.table(opts).get(k) {
            let entries: Vec<(Value, Value)> = vm.heap.table(d).entries().collect();
            durability.clear();
            for (slot, cur) in entries {
                if let (Value::Num(s), Value::Num(v)) = (slot, cur) {
                    durability.push((s as i64, v));
                }
            }
            durability.sort_by_key(|(s, _)| *s);
        }
        let k = vm.str("ghostAura");
        if let Value::Num(n) = vm.heap.table(opts).get(k) {
            ghost_aura = n;
        }
    }
    let format = match vm.get_global("DURABILITYDAMAGE_DEATH") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => "Your equipped items suffer a %d%% durability loss.".into(),
    };
    let text = format
        .replacen("%d", &format!("{DURABILITY_LOSS}"), 1)
        .replace("%%", "%");
    crate::chat::game_message(vm, "CHAT_MSG_COMBAT_MISC_INFO", &text, None);
    units::stop_auto_attack(vm);
    mirror_timers_stop(vm);
    units::set_player_power(vm, 0.0);
    if engine(vm).units.target.is_some() {
        units::clear_target(vm);
    }
    let now = crate::clock::now(vm);
    let n = {
        let d = &mut engine_mut(vm).world.death;
        d.deaths += 1;
        d.died_at = Some(now);
        d.ghost_aura = ghost_aura;
        d.deaths
    };
    crate::chat::game_message(
        vm,
        "CHAT_MSG_SYSTEM",
        &format!("|cff71d5ff|Hdeath:{n}|h[You died.]|h"),
        None,
    );
    {
        let p = &mut engine_mut(vm).units.list[PLAYER.0];
        p.dead = true;
        p.health = 0.0;
    }
    let g = units::event_guid(vm, PLAYER);
    fire(vm, "UNIT_DIED", &[g]);
    fire(vm, "PLAYER_DEAD", &[]);
    units::player_out_of_combat(vm);
    fire(vm, "PET_BAR_UPDATE_USABLE", &[]);
    units::set_player_power(vm, 0.0);
    for (i, (slot, current)) in durability.into_iter().enumerate() {
        if let Some(h) = engine_mut(vm).world.character.equipment.get_mut(&slot) {
            let max = h.durability.map_or(current, |(_, m)| m);
            h.durability = Some((current, max));
        }
        fire(vm, "UPDATE_INVENTORY_DURABILITY", &[]);
        if i == 0 {
            units::fire_unit(vm, "UNIT_FLAGS", PLAYER, &[]);
        }
    }
    crate::actions::update_usability(vm);
    units::fire_unit(vm, "UNIT_HEALTH", PLAYER, &[]);
    fire(vm, "UPDATE_SHAPESHIFT_USABLE", &[]);
    Ok(vec![])
}

fn dead(vm: &Vm) -> bool {
    engine(vm).units.list[PLAYER.0].dead
}

fn ghost(vm: &Vm) -> bool {
    engine(vm).units.list[PLAYER.0].ghost
}

/// `RepopMe()`: the spirit released, a round trip later.
fn repop_me(vm: &mut Vm, _c: NativeCall) -> Ret {
    if !dead(vm) {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    let later = vm.native("deathReleased", released as NativeFn);
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn released(vm: &mut Vm, _c: NativeCall) -> Ret {
    if !dead(vm) {
        return Ok(vec![]);
    }
    mirror_timers_stop(vm);
    {
        let p = &mut engine_mut(vm).units.list[PLAYER.0];
        p.dead = false;
        p.ghost = true;
        p.health = p.health_max;
    }
    engine_mut(vm).world.death.died_at = None;
    fire(vm, "PLAYER_ALIVE", &[]);
    for e in ["UNIT_FLAGS", "PLAYER_FLAGS_CHANGED", "UNIT_FACTION"] {
        units::fire_unit(vm, e, PLAYER, &[]);
    }
    // The ghost's aura: Ghost, harmful, without end, the player's own but
    // not the player's to apply (its instance the death's `ghostAura`,
    // recorded).
    let data = vm.new_table();
    let name = vm.str("Ghost");
    let me = vm.str("player");
    let instance = engine(vm).world.death.ghost_aura;
    let mut fields = vec![
        ("spellId", Value::Num(GHOST_SPELL)),
        ("name", name),
        ("icon", Value::Num(GHOST_ICON)),
        ("harmful", Value::Bool(true)),
        ("sourceUnit", me),
        ("canApplyAura", Value::Bool(false)),
    ];
    if instance > 0.0 {
        fields.push(("auraInstanceID", Value::Num(instance)));
    }
    for (k, v) in fields {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(data).set(k, v);
    }
    let instance = crate::auras::apply(vm, PLAYER, data);
    engine_mut(vm).world.death.ghost_aura = instance;
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
    fire(
        vm,
        "SPELL_UPDATE_COOLDOWN",
        &[
            Value::Num(GHOST_SPELL),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
        ],
    );
    fire(vm, "SUPER_TRACKING_CHANGED", &[]);
    units::fire_unit(vm, "UNIT_HEALTH", PLAYER, &[]);
    fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
    let later = vm.native("deathGhost", ghost_flags as NativeFn);
    crate::clock::after_engine(vm, RELEASED_AFTER, later);
    Ok(vec![])
}

fn ghost_flags(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "PET_BAR_UPDATE_USABLE", &[]);
    units::fire_unit(vm, "PLAYER_FLAGS_CHANGED", PLAYER, &[]);
    Ok(vec![])
}

/// `e.nearCorpse(near)`: the ghost reaches its corpse (`CORPSE_IN_RANGE`)
/// or leaves it (`CORPSE_OUT_OF_RANGE`).
pub fn control_near_corpse(vm: &mut Vm, c: NativeCall) -> Ret {
    if !ghost(vm) {
        return Err(vm.error("nearCorpse: the player is no ghost"));
    }
    if c.arg(1).truthy() {
        fire(vm, "CORPSE_IN_RANGE", &[]);
    } else {
        fire(vm, "CORPSE_OUT_OF_RANGE", &[]);
    }
    Ok(vec![])
}

/// `RetrieveCorpse()`: alive at the corpse, a round trip later.
fn retrieve_corpse(vm: &mut Vm, _c: NativeCall) -> Ret {
    if !ghost(vm) {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    let later = vm.native("deathRevived", revived as NativeFn);
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn revived(vm: &mut Vm, _c: NativeCall) -> Ret {
    if !ghost(vm) {
        return Ok(vec![]);
    }
    fire(vm, "PET_BAR_UPDATE_USABLE", &[]);
    units::fire_unit(vm, "PLAYER_FLAGS_CHANGED", PLAYER, &[]);
    {
        let p = &mut engine_mut(vm).units.list[PLAYER.0];
        p.ghost = false;
        p.health = (p.health_max / 2.0).floor();
    }
    fire(vm, "PLAYER_UNGHOST", &[]);
    fire(vm, "CORPSE_OUT_OF_RANGE", &[]);
    units::fire_unit(vm, "UNIT_FACTION", PLAYER, &[]);
    let changed = crate::actions::usability_changed(vm);
    // The ghost's aura gone.
    let aura = engine(vm).world.death.ghost_aura;
    crate::auras::remove(vm, PLAYER, aura);
    if changed {
        fire(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    }
    fire(vm, "SUPER_TRACKING_CHANGED", &[]);
    units::fire_unit(vm, "UNIT_HEALTH", PLAYER, &[]);
    fire(vm, "UPDATE_SHAPESHIFT_USABLE", &[]);
    Ok(vec![])
}

/// `caster:resurrect(target, { health })`: another's resurrection spell
/// landing (unverified throughout: no recording). On the dead player: an
/// offer, `RESURRECT_REQUEST(name)` (Blizzard's popup; `AcceptResurrect`
/// takes it, a round trip later alive with `health`, half the maximum by
/// default, as `RetrieveCorpse`'s: `PLAYER_ALIVE`, or `PLAYER_UNGHOST` for
/// a ghost). On another dead unit: alive at once with `health`
/// (`UNIT_HEALTH`, `UNIT_FLAGS`).
pub fn control_resurrect(vm: &mut Vm, c: NativeCall) -> Ret {
    let caster = units::this(vm, &c)?;
    let Some(target) = units::handle_of(vm, c.arg(2)) else {
        return Err(vm.error("resurrect: a unit to resurrect expected"));
    };
    let health = match c.arg(3) {
        Value::Table(t) => {
            let k = vm.str("health");
            match vm.heap.table(t).get(k) {
                Value::Num(n) => Some(n),
                _ => None,
            }
        }
        _ => None,
    };
    let (is_dead, max) = {
        let u = &engine(vm).units.list[target.0];
        (u.dead || u.ghost, u.health_max)
    };
    if !is_dead {
        return Err(vm.error("resurrect: the unit is alive"));
    }
    let health = health
        .unwrap_or((max / 2.0).floor())
        .clamp(1.0, max.max(1.0));
    if target == PLAYER {
        engine_mut(vm).world.death.offer = Some((caster, health));
        let name = engine(vm).units.list[caster.0].name.clone();
        let name = vm.str(&name);
        fire(vm, "RESURRECT_REQUEST", &[name]);
        return Ok(vec![]);
    }
    {
        let u = &mut engine_mut(vm).units.list[target.0];
        u.dead = false;
        u.ghost = false;
        u.health = health;
    }
    units::fire_unit(vm, "UNIT_HEALTH", target, &[]);
    units::fire_unit(vm, "UNIT_FLAGS", target, &[]);
    Ok(vec![])
}

/// `ResurrectGetOfferer()`: the offer's caster's name, else nil (recorded
/// without an offer).
fn resurrect_get_offerer(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((caster, _)) = engine(vm).world.death.offer else {
        return Ok(vec![Value::Nil]);
    };
    let name = engine(vm).units.list[caster.0].name.clone();
    Ok(vec![vm.str(&name)])
}

/// `AcceptResurrect()`: alive a round trip later.
fn accept_resurrect(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((_, health)) = engine(vm).world.death.offer else {
        return Ok(vec![]);
    };
    let delay = crate::clock::server_delay(vm);
    let later = vm.native_with(
        "deathResurrected",
        resurrected as NativeFn,
        vec![Value::Num(health)],
    );
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

fn resurrected(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(health) = c.upvals[0] else {
        return Ok(vec![]);
    };
    if engine_mut(vm).world.death.offer.take().is_none() || !(dead(vm) || ghost(vm)) {
        return Ok(vec![]);
    }
    let was_ghost = ghost(vm);
    {
        let p = &mut engine_mut(vm).units.list[PLAYER.0];
        p.dead = false;
        p.ghost = false;
        p.health = health;
    }
    engine_mut(vm).world.death.died_at = None;
    let event = if was_ghost {
        "PLAYER_UNGHOST"
    } else {
        "PLAYER_ALIVE"
    };
    fire(vm, event, &[]);
    units::fire_unit(vm, "UNIT_FLAGS", PLAYER, &[]);
    if crate::actions::usability_changed(vm) {
        fire(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    }
    units::fire_unit(vm, "UNIT_HEALTH", PLAYER, &[]);
    fire(vm, "UPDATE_SHAPESHIFT_USABLE", &[]);
    Ok(vec![])
}

/// `DeclineResurrect()`, `TimeoutResurrect()`: the offer gone.
fn decline_resurrect(vm: &mut Vm, _c: NativeCall) -> Ret {
    engine_mut(vm).world.death.offer = None;
    Ok(vec![])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `GetReleaseTimeRemaining()`: 360 s from the death (unverified: the
/// count down), 0 alive.
fn get_release_time_remaining(vm: &mut Vm, _c: NativeCall) -> Ret {
    let left = match engine(vm).world.death.died_at {
        Some(at) => (RELEASE_TIME - (crate::clock::now(vm) - at))
            .max(0.0)
            .floor(),
        None => 0.0,
    };
    Ok(vec![Value::Num(left)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `HasNoReleaseAura()`: none (recorded `false, 0, false`).
fn has_no_release_aura(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Bool(false),
        Value::Num(0.0),
        Value::Bool(false),
    ])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("RepopMe", repop_me),
        ("RetrieveCorpse", retrieve_corpse),
        ("GetReleaseTimeRemaining", get_release_time_remaining),
        ("GetCorpseRecoveryDelay", zero),
        ("HasNoReleaseAura", has_no_release_aura),
        ("ResurrectGetOfferer", resurrect_get_offerer),
        ("AcceptResurrect", accept_resurrect),
        ("DeclineResurrect", decline_resurrect),
        ("TimeoutResurrect", decline_resurrect),
        // No resurrection sickness, no timer (recorded without an offer;
        // with one unverified).
        ("ResurrectHasSickness", no),
        ("ResurrectHasTimer", no),
    ]
}
