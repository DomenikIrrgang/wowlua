//! Units casting spells (spec Tooling §16.1.4): `unit:castSpell(id, opts)`
//! and `unit:interruptCast()` drive `UnitCastingInfo`, `UnitChannelInfo`
//! and the `UNIT_SPELLCAST_*` events on the virtual clock.
//!
//! Unverified (from Blizzard's documentation and cast bar code): the
//! events' order (`START`; then `SUCCEEDED` and `STOP`, or `INTERRUPTED`
//! and `STOP`; channels `CHANNEL_START` / `CHANNEL_STOP`), the cast GUID's
//! format, and the default cast time (1.5 s).

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
}

#[derive(Default)]
pub struct Casting {
    pub casts: HashMap<UnitId, Cast>,
    pub next: u64,
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
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
    let cast = Cast {
        spell_id,
        name,
        icon,
        start: now,
        end: now + seconds,
        channel,
        not_interruptible,
        guid: guid.clone(),
    };
    // A new cast replaces the one running.
    if engine(vm).units.casting.casts.contains_key(&me) {
        stop(vm, me, true);
    }
    engine_mut(vm).units.casting.casts.insert(me, cast);
    let (start_event, _) = events(channel);
    let g = vm.str(&guid);
    units::fire_unit(vm, start_event, me, &[g, Value::Num(spell_id)]);
    let native = vm.native_with(
        "castDone",
        cast_done as NativeFn,
        vec![Value::Num(me.0 as f64), g],
    );
    crate::clock::after_engine(vm, seconds, native);
    Ok(vec![])
}

/// Ends `unit`'s cast: interrupted, or done.
fn stop(vm: &mut Vm, unit: UnitId, interrupted: bool) {
    let Some(cast) = engine_mut(vm).units.casting.casts.remove(&unit) else {
        return;
    };
    let g = vm.str(&cast.guid);
    let id = Value::Num(cast.spell_id);
    if interrupted {
        units::fire_unit(vm, "UNIT_SPELLCAST_INTERRUPTED", unit, &[g, id]);
    } else if !cast.channel {
        units::fire_unit(vm, "UNIT_SPELLCAST_SUCCEEDED", unit, &[g, id]);
    }
    let (_, stop_event) = events(cast.channel);
    units::fire_unit(vm, stop_event, unit, &[g, id]);
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

/// `unit:interruptCast()`.
pub fn control_interrupt(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    stop(vm, me, true);
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
