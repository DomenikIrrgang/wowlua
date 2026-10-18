//! Units casting spells (spec Tooling §16.1.4): `unit:castSpell(id, opts)`
//! and `unit:interruptCast()` drive `UnitCastingInfo`, `UnitChannelInfo`
//! and the `UNIT_SPELLCAST_*` events on the virtual clock.
//!
//! Verified against the client (`/stdtest engine record cast`, Forever
//! 70425): the player's `SENT(unit, target name | nil, castGUID, spellID)`;
//! `START`; then `SUCCEEDED` and `STOP`, or `INTERRUPTED(unit, castGUID,
//! spellID, nil, castBarID)` and `STOP`; each with the cast's
//! `CastBar-<16 hex>` id last; `CURRENT_SPELL_CAST_CHANGED(false)` after the
//! player's start and stop; an instant cast's `SUCCEEDED(…, nil)` between
//! two of those, without `START` / `STOP`; a player's cast while casting `FAILED_QUIET` with
//! `UI_ERROR_MESSAGE(280, "Another action is in progress")`, the running
//! cast kept; channels' `CHANNEL_START(unit, nil, spellID, castBarID)` and
//! `CHANNEL_STOP(unit, nil, spellID, nil, castBarID)`; the cast GUID's
//! shape; other units' cast GUIDs and spell IDs secret, the player's
//! `SENT` target in combat. Unverified: the default cast time (1.5 s), an interrupted
//! channel's `INTERRUPTED`, other units' casts replacing a running one.
//! The player's interrupted cast is answered 0.4 s later (recorded for an
//! interrupt by moving; unverified for other interrupts).

use std::collections::HashMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut};

#[derive(Clone, Debug)]
pub struct Cast {
    pub spell_id: f64,
    pub name: Value,
    pub icon: Value,
    /// Start and end, in the clock's seconds.
    pub start: f64,
    pub end: f64,
    pub channel: bool,
    pub not_interruptible: bool,
    pub guid: String,
    pub bar: String,
}

#[derive(Default)]
pub struct Casting {
    pub casts: HashMap<UnitId, Cast>,
    pub next: u64,
    /// The player's spell for the next swing (Heroic Strike), taken by the
    /// server: its ID and cast GUID.
    pub next_swing: Option<(f64, String)>,
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

/// The player's `CURRENT_SPELL_CAST_CHANGED(false)`.
fn cast_changed(vm: &mut Vm, unit: UnitId) {
    if unit == units::PLAYER {
        crate::widgets::fire_event(vm, "CURRENT_SPELL_CAST_CHANGED", &[Value::Bool(false)]);
    }
}

/// The player's `UNIT_SPELLCAST_SENT(unit, target name, castGUID, spellID)`:
/// the target's full name (with a last name), or nil.
fn sent(vm: &mut Vm, unit: UnitId, guid: Value, spell_id: f64) {
    if unit != units::PLAYER {
        return;
    }
    // A spell without a target (its data's `targeted = false`) names none
    // (recorded: an instant self spell's nil).
    let untargeted = matches!(spell_field(vm, spell_id, "targeted"), Value::Bool(false));
    let target = {
        let u = &engine(vm).units;
        u.target.map(|t| {
            let t = &u.list[t.0];
            match &t.last_name {
                Some(last) => format!("{} {last}", t.name),
                None => t.name.clone(),
            }
        })
    };
    let mut target = match target {
        Some(t) if !untargeted => vm.str(&t),
        _ => Value::Nil,
    };
    // Secret where units' identities are (`ConditionalSecret`): a
    // paladin's Judgement in Ragefire Chasm (its cast GUIDs' map 389)
    // secret, out of combat too (`/stdtest engine record combat`); a
    // warrior's Heroic Strike in the open world plain, in combat (scenarios
    // combat and quest). Unverified: which instance restrictions count.
    let restricted = units::active(
        &engine(vm).units,
        &[
            units::MAP,
            units::ENCOUNTER,
            units::CHALLENGE_MODE,
            units::PVP_MATCH,
        ],
    );
    if !target.is_nil() && restricted {
        target = vm.secret_wrap(target);
    }
    units::fire_unit(
        vm,
        "UNIT_SPELLCAST_SENT",
        unit,
        &[target, guid, Value::Num(spell_id)],
    );
}

/// A cast event for each of the unit's tokens: another unit's cast GUID
/// and spell ID secret (`SecretWhenUnitSpellCastRestricted`; recorded for
/// nameplates out of combat), its cast bar id never.
fn fire_cast(vm: &mut Vm, event: &str, unit: UnitId, args: &[Value]) {
    let mut args = args.to_vec();
    if unit != units::PLAYER {
        for v in args.iter_mut().take(2) {
            if !v.is_nil() {
                *v = vm.secret_wrap(*v);
            }
        }
    }
    units::fire_unit(vm, event, unit, &args);
}

fn events(channel: bool) -> (&'static str, &'static str) {
    if channel {
        (
            "UNIT_SPELLCAST_CHANNEL_START",
            "UNIT_SPELLCAST_CHANNEL_STOP",
        )
    } else {
        ("UNIT_SPELLCAST_START", "UNIT_SPELLCAST_STOP")
    }
}

/// `unit:castSpell(spellID, { castTime?, channel?, notInterruptible? })`.
pub fn control_cast(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let spell_id = vm.check_number(&c, 2)?;
    let opts = match c.arg(3) {
        Value::Table(t) => Some(t),
        _ => None,
    };
    let opt = |vm: &mut Vm, k: &str| opts.map(|t| field(vm, t, k)).unwrap_or(Value::Nil);
    let data = engine(vm)
        .world
        .data
        .spells
        .get(&(spell_id as i64))
        .copied();
    let (name, icon, data_time) = match data {
        Some(t) => (
            field(vm, t, "name"),
            field(vm, t, "iconID"),
            match field(vm, t, "castTime") {
                // `castTime` in spell data is in milliseconds.
                Value::Num(ms) => Some(ms / 1000.0),
                _ => None,
            },
        ),
        None => (
            vm.str(&format!("Spell {spell_id}")),
            Value::Num(136243.0),
            None,
        ),
    };
    let seconds = match opt(vm, "castTime") {
        Value::Num(s) => s,
        _ => data_time.unwrap_or(1.5),
    };
    let channel = opt(vm, "channel").truthy();
    let not_interruptible = opt(vm, "notInterruptible").truthy();
    let now = crate::clock::now(vm);
    let n = {
        let c = &mut engine_mut(vm).units.casting;
        c.next += 1;
        c.next
    };
    let guid = format!("Cast-3-4621-0-1-{}-{:010X}", spell_id as i64, n);
    let bar = format!("CastBar-{:016X}", 0x5EED_0000_0000_0000u64 + n);
    let g = vm.str(&guid);
    let id = Value::Num(spell_id);
    if engine(vm).units.casting.casts.contains_key(&me) {
        if me == units::PLAYER {
            // The player's running cast stays; the new one fails.
            sent(vm, me, g, spell_id);
            fire_cast(vm, "UNIT_SPELLCAST_FAILED_QUIET", me, &[g, id, Value::Nil]);
            cast_changed(vm, me);
            let message = vm.str("Another action is in progress");
            crate::widgets::fire_event(vm, "UI_ERROR_MESSAGE", &[Value::Num(280.0), message]);
            return Ok(vec![]);
        }
        // Another unit's new cast replaces the one running (unverified).
        stop(vm, me, true);
    }
    sent(vm, me, g, spell_id);
    if seconds <= 0.0 && !channel {
        cast_changed(vm, me);
        fire_cast(vm, "UNIT_SPELLCAST_SUCCEEDED", me, &[g, id, Value::Nil]);
        cast_changed(vm, me);
        return Ok(vec![]);
    }
    let cast = Cast {
        spell_id,
        name,
        icon,
        start: now,
        end: now + seconds,
        channel,
        not_interruptible,
        guid: guid.clone(),
        bar: bar.clone(),
    };
    engine_mut(vm).units.casting.casts.insert(me, cast);
    let (start_event, _) = events(channel);
    let b = vm.str(&bar);
    // A channel has no cast GUID.
    let shown = if channel { Value::Nil } else { g };
    fire_cast(vm, start_event, me, &[shown, id, b]);
    cast_changed(vm, me);
    let native = vm.native_with(
        "castDone",
        cast_done as NativeFn,
        vec![Value::Num(me.0 as f64), g],
    );
    crate::clock::after_engine(vm, seconds, native);
    Ok(vec![])
}

/// The global cooldown's category (`SPELL_UPDATE_COOLDOWN`'s fourth value
/// for a spell on it: recorded 133).
const GCD_CATEGORY: f64 = 133.0;

/// A spell's data field (`defineSpell`'s, the profile's).
fn spell_field(vm: &mut Vm, id: f64, key: &str) -> Value {
    match engine(vm).world.data.spells.get(&(id as i64)).copied() {
        Some(t) => field(vm, t, key),
        None => Value::Nil,
    }
}

/// The cooldown events after a spell's use: the action bars', the bags',
/// `SPELL_UPDATE_COOLDOWN(id, nil, 0, category, nil)` (the global
/// cooldown's category, or 0 off it), the stance bar's.
fn cooldown_events(vm: &mut Vm, id: f64, category: f64, bags: bool) {
    let fire = |vm: &mut Vm, e: &str, args: &[Value]| crate::widgets::fire_event(vm, e, args);
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    if bags {
        fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
    }
    fire(
        vm,
        "SPELL_UPDATE_COOLDOWN",
        &[
            Value::Num(id),
            Value::Nil,
            Value::Num(0.0),
            Value::Num(category),
            Value::Nil,
        ],
    );
    fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
}

/// `player:useSpell(spellID)`: the player pressing a spell, as recorded at a
/// ping near 0.3 s (`/stdtest engine scenario combat`). Without the power
/// its data's `powerCost` needs: `UNIT_SPELLCAST_FAILED` with a client
/// cast GUID, `CURRENT_SPELL_CAST_CHANGED`, `ACTIONBAR_UPDATE_STATE`,
/// `UI_ERROR_MESSAGE(359, "Not enough rage")`. A spell on the next swing
/// (data `nextSwing`): `CURRENT_SPELL_CAST_CHANGED`,
/// `ACTIONBAR_UPDATE_STATE`; a round trip later `SENT`, again both; the
/// next swing lands it (`units::swing`). Else the global cooldown starts
/// (`CURRENT_SPELL_CAST_CHANGED`, the cooldown events, category 133 unless
/// data `offGCD`) and a round trip later the cast (`castSpell`), then the
/// cooldown events again.
pub fn control_use_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    if me != units::PLAYER {
        return Err(vm.error("useSpell: the player's"));
    }
    let id = vm.check_number(&c, 2)?;
    let fire = |vm: &mut Vm, e: &str, args: &[Value]| crate::widgets::fire_event(vm, e, args);
    if let Value::Num(cost) = spell_field(vm, id, "powerCost")
        && engine(vm).units.list[units::PLAYER.0].power < cost
    {
        let n = {
            let c = &mut engine_mut(vm).units.casting;
            c.next += 1;
            c.next
        };
        // The client's own cast GUID (`Cast-2-0-0-0-…`).
        let g = vm.str(&format!("Cast-2-0-0-0-{}-{:010X}", id as i64, n));
        units::fire_unit(
            vm,
            "UNIT_SPELLCAST_FAILED",
            me,
            &[g, Value::Num(id), Value::Nil],
        );
        cast_changed(vm, me);
        fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
        let token = engine(vm).units.list[units::PLAYER.0].power_type;
        let (code, text) = match token as i64 {
            1 => (359.0, "Not enough rage"),
            3 => (360.0, "Not enough energy"),
            _ => (358.0, "Not enough mana"),
        };
        let text = vm.str(text);
        fire(vm, "UI_ERROR_MESSAGE", &[Value::Num(code), text]);
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    if spell_field(vm, id, "nextSwing").truthy() {
        cast_changed(vm, me);
        fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
        let later = vm.native_with(
            "nextSwingSent",
            next_swing_sent as NativeFn,
            vec![Value::Num(id)],
        );
        crate::clock::after_engine(vm, delay, later);
        return Ok(vec![]);
    }
    let category = if spell_field(vm, id, "offGCD").truthy() {
        0.0
    } else {
        GCD_CATEGORY
    };
    cast_changed(vm, me);
    cooldown_events(vm, id, category, false);
    let later = vm.native_with(
        "spellUsed",
        spell_used as NativeFn,
        vec![Value::Num(id), Value::Num(category)],
    );
    crate::clock::after_engine(vm, delay, later);
    Ok(vec![])
}

/// The server's answer to an instant spell: the cast, the cooldown events.
fn spell_used(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(id), Value::Num(category)) = (c.upvals[0], c.upvals[1]) else {
        return Ok(vec![]);
    };
    instant(vm, id);
    cooldown_events(vm, id, category, true);
    Ok(vec![])
}

/// An instant cast: `SENT`, `CURRENT_SPELL_CAST_CHANGED`, `SUCCEEDED`,
/// `CURRENT_SPELL_CAST_CHANGED`.
fn instant(vm: &mut Vm, id: f64) {
    let me = units::PLAYER;
    let g = new_guid(vm, id);
    let g = vm.str(&g);
    sent(vm, me, g, id);
    cast_changed(vm, me);
    fire_cast(
        vm,
        "UNIT_SPELLCAST_SUCCEEDED",
        me,
        &[g, Value::Num(id), Value::Nil],
    );
    cast_changed(vm, me);
}

/// A new cast GUID of a kind (3 a spell, 4 an item's, 15 a triggered one,
/// recorded) for the player's spell `id`.
pub fn cast_guid(vm: &mut Vm, kind: u8, id: f64) -> String {
    let n = {
        let c = &mut engine_mut(vm).units.casting;
        c.next += 1;
        c.next
    };
    format!("Cast-{kind}-4621-0-1-{}-{:010X}", id as i64, n)
}

/// A new cast GUID for the player's spell `id`.
fn new_guid(vm: &mut Vm, id: f64) -> String {
    let n = {
        let c = &mut engine_mut(vm).units.casting;
        c.next += 1;
        c.next
    };
    format!("Cast-3-4621-0-1-{}-{:010X}", id as i64, n)
}

/// The server took a spell for the next swing: `SENT`,
/// `CURRENT_SPELL_CAST_CHANGED`, `ACTIONBAR_UPDATE_STATE`; queued.
fn next_swing_sent(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(id) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let g = new_guid(vm, id);
    let gv = vm.str(&g);
    sent(vm, units::PLAYER, gv, id);
    cast_changed(vm, units::PLAYER);
    crate::widgets::fire_event(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    engine_mut(vm).units.casting.next_swing = Some((id, g));
    Ok(vec![])
}

/// The spell queued for the next swing, taken: its `SUCCEEDED` (and
/// `CURRENT_SPELL_CAST_CHANGED`); its ID.
pub fn land_next_swing(vm: &mut Vm) -> Option<f64> {
    let (id, g) = engine_mut(vm).units.casting.next_swing.take()?;
    let g = vm.str(&g);
    fire_cast(
        vm,
        "UNIT_SPELLCAST_SUCCEEDED",
        units::PLAYER,
        &[g, Value::Num(id), Value::Nil],
    );
    cast_changed(vm, units::PLAYER);
    Some(id)
}

/// After a spell on the next swing landed: the cooldown events (off the
/// global cooldown).
pub fn next_swing_cooldowns(vm: &mut Vm, id: f64) {
    cooldown_events(vm, id, 0.0, true);
}

/// Ends `unit`'s cast: interrupted, or done.
fn stop(vm: &mut Vm, unit: UnitId, interrupted: bool) {
    let Some(cast) = engine_mut(vm).units.casting.casts.remove(&unit) else {
        return;
    };
    let g = if cast.channel {
        Value::Nil
    } else {
        vm.str(&cast.guid)
    };
    let id = Value::Num(cast.spell_id);
    let b = vm.str(&cast.bar);
    if interrupted {
        fire_cast(
            vm,
            "UNIT_SPELLCAST_INTERRUPTED",
            unit,
            &[g, id, Value::Nil, b],
        );
    } else if !cast.channel {
        fire_cast(vm, "UNIT_SPELLCAST_SUCCEEDED", unit, &[g, id, b]);
    }
    let (_, stop_event) = events(cast.channel);
    if cast.channel {
        fire_cast(vm, stop_event, unit, &[g, id, Value::Nil, b]);
    } else {
        fire_cast(vm, stop_event, unit, &[g, id, b]);
    }
    cast_changed(vm, unit);
}

/// The cast's end, if it is still the unit's cast.
fn cast_done(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(Value::Num(u)), Some(Value::Str(g))) = (c.upvals.first(), c.upvals.get(1)) else {
        return Ok(vec![]);
    };
    let unit = UnitId(*u as usize);
    let guid = String::from_utf8_lossy(vm.str_bytes(*g)).into_owned();
    if engine(vm)
        .units
        .casting
        .casts
        .get(&unit)
        .is_some_and(|c| c.guid == guid)
    {
        stop(vm, unit, false);
    }
    Ok(vec![])
}

// The server answers the player's interrupted cast a round trip later
// (`/stdtest engine record cast`, an interrupt by moving: 0.35–0.54 s).

/// `unit:interruptCast()`; the player's is answered by the server later:
/// three `INTERRUPTED(unit, castGUID, spellID, nil, nil)`, each with
/// `CURRENT_SPELL_CAST_CHANGED(false)`, then `UI_ERROR_MESSAGE(57,
/// "Interrupted")`.
pub fn control_interrupt(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let cast = engine(vm).units.casting.casts.get(&me).cloned();
    stop(vm, me, true);
    if me == units::PLAYER
        && let Some(cast) = cast
        && !cast.channel
    {
        let g = vm.str(&cast.guid);
        let echo = vm.native_with(
            "interruptEcho",
            interrupt_echo,
            vec![g, Value::Num(cast.spell_id)],
        );
        let delay = crate::clock::server_delay(vm);
        crate::clock::after_engine(vm, delay, echo);
    }
    Ok(vec![])
}

fn interrupt_echo(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(&g), Some(&id)) = (c.upvals.first(), c.upvals.get(1)) else {
        return Ok(vec![]);
    };
    for _ in 0..3 {
        fire_cast(
            vm,
            "UNIT_SPELLCAST_INTERRUPTED",
            units::PLAYER,
            &[g, id, Value::Nil, Value::Nil],
        );
        cast_changed(vm, units::PLAYER);
    }
    let message = vm.str("Interrupted");
    crate::widgets::fire_event(vm, "UI_ERROR_MESSAGE", &[Value::Num(57.0), message]);
    Ok(vec![])
}

fn info(vm: &mut Vm, c: &NativeCall, channel: bool) -> Ret {
    let Some(unit) = units::unit_arg(vm, c, 1) else {
        return Ok(vec![]);
    };
    let Some(cast) = engine(vm).units.casting.casts.get(&unit).cloned() else {
        return Ok(vec![]);
    };
    if cast.channel != channel {
        return Ok(vec![]);
    }
    let mut out = vec![
        cast.name,
        cast.name,
        cast.icon,
        Value::Num((cast.start * 1000.0).floor()),
        Value::Num((cast.end * 1000.0).floor()),
        Value::Bool(false),
    ];
    if !channel {
        out.push(vm.str(&cast.guid));
    }
    out.push(Value::Bool(cast.not_interruptible));
    out.push(Value::Num(cast.spell_id));
    Ok(out)
}

fn unit_casting_info(vm: &mut Vm, c: NativeCall) -> Ret {
    info(vm, &c, false)
}

fn unit_channel_info(vm: &mut Vm, c: NativeCall) -> Ret {
    info(vm, &c, true)
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("UnitCastingInfo", unit_casting_info),
        ("UnitChannelInfo", unit_channel_info),
    ]
}
