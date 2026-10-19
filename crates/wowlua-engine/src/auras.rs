//! Auras on the world's units (spec Tooling §16.1.4): `unit:applyAura(data)`
//! and `unit:removeAura(id)` change them and fire `UNIT_AURA` for each of
//! the unit's tokens; `C_UnitAuras` reads them as `AuraData` tables.
//!
//! Recorded on WoW Forever 1.60.1 (`/stdtest restrictions`): in combat,
//! tainted code reading an aura fails with "`<Function>`(): Auras cannot be
//! accessed when secret while tainted by '`<addon>`'" and a "Lua Taint:"
//! line; on a restricted map outside combat the data is plain. Recorded
//! with `/stdtest engine record aura` and `combat`: `UNIT_AURA`'s update
//! information (`isFullUpdate` false, one-element `addedAuras` or
//! `removedAuraInstanceIDs`, empty keys absent; secret fields in combat)
//! and the added aura's fields. Unverified: the filters beyond HELPFUL,
//! HARMFUL, PLAYER and RAID; secure code reading auras in combat (plain
//! data).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{LuaError, NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, protection, widgets};

/// One aura on a unit.
#[derive(Clone, Debug)]
pub struct Aura {
    pub unit: UnitId,
    pub instance: f64,
    pub spell_id: f64,
    pub name: String,
    pub icon: f64,
    pub applications: f64,
    pub duration: f64,
    pub expiration: f64,
    pub harmful: bool,
    pub source: Option<UnitId>,
    pub dispel_name: Option<String>,
    pub stealable: bool,
    pub boss: bool,
    pub raid: bool,
    pub points: Vec<f64>,
    /// Whether the player could apply it (`canApplyAura`; by default an
    /// aura of the player's own).
    pub can_apply: Option<bool>,
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

/// An aura as the client's `AuraData` table.
fn aura_data(vm: &mut Vm, a: &Aura) -> Value {
    let t = vm.new_table();
    let source_token = a
        .source
        .and_then(|s| units::tokens_of(&engine(vm).units, s).into_iter().next());
    let from_player = a.source == Some(units::PLAYER);
    for (k, v) in [
        ("applications", Value::Num(a.applications)),
        ("auraInstanceID", Value::Num(a.instance)),
        ("canActivePlayerDispel", Value::Bool(false)),
        (
            "canApplyAura",
            Value::Bool(a.can_apply.unwrap_or(from_player)),
        ),
        ("duration", Value::Num(a.duration)),
        ("expirationTime", Value::Num(a.expiration)),
        ("hideOnPartyFrames", Value::Bool(false)),
        ("icon", Value::Num(a.icon)),
        ("isBossAura", Value::Bool(a.boss)),
        ("isDPSRoleAura", Value::Bool(false)),
        ("isFromPlayerOrPlayerPet", Value::Bool(from_player)),
        ("isHarmful", Value::Bool(a.harmful)),
        ("isHealerRoleAura", Value::Bool(false)),
        ("isHelpful", Value::Bool(!a.harmful)),
        ("isNameplateOnly", Value::Bool(false)),
        ("isRaid", Value::Bool(a.raid)),
        ("isStealable", Value::Bool(a.stealable)),
        ("isTankRoleAura", Value::Bool(false)),
        ("nameplateShowAll", Value::Bool(false)),
        ("nameplateShowPersonal", Value::Bool(false)),
        ("spellId", Value::Num(a.spell_id)),
        ("timeMod", Value::Num(1.0)),
    ] {
        set(vm, t, k, v);
    }
    let name = vm.str(&a.name);
    set(vm, t, "name", name);
    let points = vm.new_table();
    for (i, p) in a.points.iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(points)
            .set(Value::Num(i as f64 + 1.0), Value::Num(*p));
    }
    set(vm, t, "points", Value::Table(points));
    if let Some(token) = source_token {
        let s = vm.str(&token);
        set(vm, t, "sourceUnit", s);
    }
    if let Some(d) = &a.dispel_name {
        let s = vm.str(d);
        set(vm, t, "dispelName", s);
    }
    Value::Table(t)
}

/// Whether an aura passes a filter (`"HELPFUL|PLAYER"`; HELPFUL when none
/// says HARMFUL).
fn matches(a: &Aura, filter: &str) -> bool {
    let words: Vec<String> = filter
        .split(['|', ' '])
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_uppercase)
        .collect();
    let harmful = words.iter().any(|w| w == "HARMFUL");
    if a.harmful != harmful {
        return false;
    }
    words.iter().all(|w| match w.as_str() {
        "PLAYER" => a.source == Some(units::PLAYER),
        "RAID" => a.raid,
        _ => true,
    })
}

/// `UNIT_AURA` for each of the unit's tokens, with the update information:
/// its fields secret while auras are restricted (`/stdtest engine record
/// aura` and `combat`: in combat, every token's).
fn fire_update(vm: &mut Vm, id: UnitId, added: Option<&Aura>, removed: &[f64]) {
    let restricted = units::active(
        &engine(vm).units,
        &[
            units::COMBAT,
            units::ENCOUNTER,
            units::CHALLENGE_MODE,
            units::PVP_MATCH,
        ],
    );
    let field = |vm: &mut Vm, v: Value| if restricted { vm.secret_wrap(v) } else { v };
    for token in units::tokens_of(&engine(vm).units, id) {
        let info = vm.new_table();
        let full = field(vm, Value::Bool(false));
        set(vm, info, "isFullUpdate", full);
        if let Some(a) = added {
            let list = vm.new_table();
            let data = aura_data(vm, a);
            let _ = vm.heap.table_mut(list).set(Value::Num(1.0), data);
            let list = field(vm, Value::Table(list));
            set(vm, info, "addedAuras", list);
        }
        if !removed.is_empty() {
            let list = vm.new_table();
            for (i, r) in removed.iter().enumerate() {
                let _ = vm
                    .heap
                    .table_mut(list)
                    .set(Value::Num(i as f64 + 1.0), Value::Num(*r));
            }
            let list = field(vm, Value::Table(list));
            set(vm, info, "removedAuraInstanceIDs", list);
        }
        let t = vm.str(&token);
        widgets::fire_event(vm, "UNIT_AURA", &[t, Value::Table(info)]);
    }
}

/// `unit:applyAura({ spellId, name, icon, duration, applications, harmful,
/// source, dispelName, stealable, boss, raid, points, auraInstanceID,
/// sourceUnit, expirationTime })`: the aura's instance ID (`auraInstanceID`,
/// `sourceUnit` and `expirationTime` as a snapshot has them).
pub fn control_apply(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let Value::Table(data) = c.arg(2) else {
        return Err(vm.error("applyAura: a table of the aura's data expected"));
    };
    let instance = apply(vm, me, data);
    Ok(vec![Value::Num(instance)])
}

/// An aura on `me` (`applyAura`'s table): its instance ID.
pub fn apply(vm: &mut Vm, me: UnitId, data: TableRef) -> f64 {
    let num = |vm: &mut Vm, k: &str, d: f64| match field(vm, data, k) {
        Value::Num(n) => n,
        _ => d,
    };
    let flag = |vm: &mut Vm, k: &str| field(vm, data, k).truthy();
    let text = |vm: &mut Vm, k: &str| match field(vm, data, k) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    };
    let spell_id = num(vm, "spellId", 0.0);
    let duration = num(vm, "duration", 0.0);
    let source = match field(vm, data, "source") {
        Value::Nil => match field(vm, data, "sourceUnit") {
            Value::Str(s) => {
                let token = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
                units::resolve(&engine(vm).units, &token)
            }
            _ => None,
        },
        Value::Str(s) => {
            let token = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            units::resolve(&engine(vm).units, &token)
        }
        v => units::handle_of(vm, v),
    };
    let mut points = Vec::new();
    if let Value::Table(p) = field(vm, data, "points") {
        for i in 1.. {
            match vm.heap.table(p).get(Value::Num(f64::from(i))) {
                Value::Num(n) => points.push(n),
                _ => break,
            }
        }
    }
    let now = crate::clock::now(vm);
    let instance = match field(vm, data, "auraInstanceID") {
        Value::Num(n) => {
            let u = &mut engine_mut(vm).units;
            u.next_aura = u.next_aura.max(n);
            n
        }
        _ => {
            let u = &mut engine_mut(vm).units;
            u.next_aura += 1.0;
            u.next_aura
        }
    };
    let expiration = match field(vm, data, "expirationTime") {
        Value::Num(n) => n,
        _ if duration > 0.0 => now + duration,
        _ => 0.0,
    };
    let aura = Aura {
        unit: me,
        instance,
        spell_id,
        name: text(vm, "name").unwrap_or_else(|| format!("Spell {spell_id}")),
        icon: num(vm, "icon", 136243.0),
        applications: num(vm, "applications", 0.0),
        duration,
        expiration,
        harmful: flag(vm, "harmful"),
        source,
        dispel_name: text(vm, "dispelName"),
        stealable: flag(vm, "stealable"),
        boss: flag(vm, "boss"),
        raid: flag(vm, "raid"),
        points,
        can_apply: match field(vm, data, "canApplyAura") {
            Value::Bool(b) => Some(b),
            _ => None,
        },
    };
    engine_mut(vm).units.auras.push(aura.clone());
    fire_update(vm, me, Some(&aura), &[]);
    // Gone when it runs out, the server's word a moment later (recorded:
    // a food's aura 0.086 s after its expiration).
    if expiration > now {
        let gone = vm.native_with(
            "auraExpired",
            expired,
            vec![Value::Num(me.0 as f64), Value::Num(instance)],
        );
        crate::clock::after_engine(vm, expiration - now + EXPIRY_LAG, gone);
    }
    instance
}

/// An aura's removal after its expiration (recorded once).
const EXPIRY_LAG: f64 = 0.086;

/// An aura run out: removed, unless already gone or renewed since.
fn expired(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(Value::Num(unit)), Some(Value::Num(id))) = (c.upvals.first(), c.upvals.get(1)) else {
        return Ok(vec![]);
    };
    let unit = UnitId(*unit as usize);
    let id = *id;
    let now = crate::clock::now(vm);
    let due =
        engine(vm).units.auras.iter().any(|a| {
            a.unit == unit && a.instance == id && a.expiration > 0.0 && a.expiration <= now
        });
    if due {
        let food = crate::inventory::eating_ends(vm, id);
        remove(vm, unit, id);
        if food {
            crate::inventory::eating_ended(vm);
        }
    }
    Ok(vec![])
}

/// Auras a place gives, gone as the player leaves it (recorded: Campfire
/// Nearby, at a Hearthstone's loading screen).
const PLACE_AURAS: &[f64] = &[1283391.0];

/// The player left the place: its auras gone.
pub fn left_place(vm: &mut Vm) {
    let gone: Vec<f64> = engine(vm)
        .units
        .auras
        .iter()
        .filter(|a| a.unit == units::PLAYER && PLACE_AURAS.contains(&a.spell_id))
        .map(|a| a.instance)
        .collect();
    for id in gone {
        remove(vm, units::PLAYER, id);
    }
}

/// An aura gone from `unit`.
pub fn remove(vm: &mut Vm, unit: UnitId, id: f64) {
    let auras = &mut engine_mut(vm).units.auras;
    let before = auras.len();
    auras.retain(|a| !(a.unit == unit && a.instance == id));
    if auras.len() != before {
        fire_update(vm, unit, None, &[id]);
    }
}

/// A unit's death takes its auras: one update removing them all, after
/// the death, the target's clearing and the loot (recorded: a stunned
/// mob's; the same update added something, secret, not modelled).
pub fn died(vm: &mut Vm, unit: UnitId) {
    let gone: Vec<f64> = engine(vm)
        .units
        .auras
        .iter()
        .filter(|a| a.unit == unit)
        .map(|a| a.instance)
        .collect();
    if gone.is_empty() {
        return;
    }
    engine_mut(vm).units.auras.retain(|a| a.unit != unit);
    fire_update(vm, unit, None, &gone);
}

/// `unit:updateAura(instanceID, { cooldowns? })`: the aura changed in
/// place (a periodic effect's tick: `UNIT_AURA` with
/// `updatedAuraInstanceIDs`; recorded: Consecration's every 0.5 s,
/// auradeath), then the spells it triggered tell their cooldowns, then
/// `UPDATE_SHAPESHIFT_COOLDOWN`.
pub fn control_update(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let id = vm.check_number(&c, 2)?;
    let on = engine(vm)
        .units
        .auras
        .iter()
        .any(|a| a.unit == me && a.instance == id);
    if on {
        fire_updated(vm, me, id);
    }
    let cooldowns = match c.arg(3) {
        Value::Table(t) => field(vm, t, "cooldowns"),
        _ => Value::Nil,
    };
    if units::triggered_cooldowns(vm, cooldowns) {
        widgets::fire_event(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
    }
    Ok(vec![])
}

/// `UNIT_AURA` telling one aura updated, for each of the unit's tokens.
pub(crate) fn fire_updated(vm: &mut Vm, id: UnitId, instance: f64) {
    let restricted = units::active(
        &engine(vm).units,
        &[
            units::COMBAT,
            units::ENCOUNTER,
            units::CHALLENGE_MODE,
            units::PVP_MATCH,
        ],
    );
    let field = |vm: &mut Vm, v: Value| if restricted { vm.secret_wrap(v) } else { v };
    for token in units::tokens_of(&engine(vm).units, id) {
        let info = vm.new_table();
        let full = field(vm, Value::Bool(false));
        set(vm, info, "isFullUpdate", full);
        let list = vm.new_table();
        let _ = vm
            .heap
            .table_mut(list)
            .set(Value::Num(1.0), Value::Num(instance));
        let list = field(vm, Value::Table(list));
        set(vm, info, "updatedAuraInstanceIDs", list);
        let t = vm.str(&token);
        widgets::fire_event(vm, "UNIT_AURA", &[t, Value::Table(info)]);
    }
}

/// `unit:removeAura(instanceID)`.
pub fn control_remove(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    let id = vm.check_number(&c, 2)?;
    remove(vm, me, id);
    Ok(vec![])
}

/// The recorded refusal: in combat, tainted code may not read auras.
fn refuse(vm: &mut Vm, function: &str) -> Result<(), LuaError> {
    let restricted = units::active(&engine(vm).units, &[units::COMBAT]);
    if !(restricted && protection::tainted(vm)) {
        return Ok(());
    }
    let addon = vm.taint_name(vm.taint().stack).unwrap_or_default();
    let msg = vm.str(&format!(
        "{function}(): Auras cannot be accessed when secret while tainted by '{addon}'\nLua Taint: {addon}"
    ));
    Err(vm.raise(msg))
}

fn unit_and_filter(vm: &mut Vm, c: &NativeCall, filter_at: usize) -> (Option<UnitId>, String) {
    let unit = match vm.hidden(c.arg(1)) {
        Value::Str(s) => {
            let token = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            units::resolve(&engine(vm).units, &token)
        }
        _ => None,
    };
    let filter = match c.arg(filter_at) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    (unit, filter)
}

/// The unit's auras passing `filter`, in application order.
fn listed(vm: &Vm, unit: UnitId, filter: &str) -> Vec<Aura> {
    engine(vm)
        .units
        .auras
        .iter()
        .filter(|a| a.unit == unit && matches(a, filter))
        .cloned()
        .collect()
}

fn by_index(vm: &mut Vm, c: NativeCall, function: &str, extra: &str) -> Ret {
    refuse(vm, function)?;
    let index = vm.check_int(&c, 2)?;
    let (unit, filter) = unit_and_filter(vm, &c, 3);
    let Some(unit) = unit else { return Ok(vec![]) };
    let filter = format!("{filter}|{extra}");
    let list = listed(vm, unit, &filter);
    match usize::try_from(index - 1).ok().and_then(|i| list.get(i)) {
        Some(a) => Ok(vec![aura_data(vm, a)]),
        None => Ok(vec![]),
    }
}

fn get_aura_data_by_index(vm: &mut Vm, c: NativeCall) -> Ret {
    by_index(vm, c, "GetAuraDataByIndex", "")
}

fn get_buff_data_by_index(vm: &mut Vm, c: NativeCall) -> Ret {
    by_index(vm, c, "GetBuffDataByIndex", "HELPFUL")
}

fn get_debuff_data_by_index(vm: &mut Vm, c: NativeCall) -> Ret {
    by_index(vm, c, "GetDebuffDataByIndex", "HARMFUL")
}

fn get_aura_data_by_instance(vm: &mut Vm, c: NativeCall) -> Ret {
    refuse(vm, "GetAuraDataByAuraInstanceID")?;
    let id = vm.check_number(&c, 2)?;
    let (unit, _) = unit_and_filter(vm, &c, 3);
    let Some(unit) = unit else { return Ok(vec![]) };
    let found = engine(vm)
        .units
        .auras
        .iter()
        .find(|a| a.unit == unit && a.instance == id)
        .cloned();
    Ok(found.map(|a| vec![aura_data(vm, &a)]).unwrap_or_default())
}

fn get_aura_data_by_spell_name(vm: &mut Vm, c: NativeCall) -> Ret {
    refuse(vm, "GetAuraDataBySpellName")?;
    let name = vm.check_str(&c, 2)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let (unit, filter) = unit_and_filter(vm, &c, 3);
    let Some(unit) = unit else { return Ok(vec![]) };
    let found = listed(vm, unit, &filter)
        .into_iter()
        .find(|a| a.name == name);
    Ok(found.map(|a| vec![aura_data(vm, &a)]).unwrap_or_default())
}

fn get_player_aura_by_spell_id(vm: &mut Vm, c: NativeCall) -> Ret {
    refuse(vm, "GetPlayerAuraBySpellID")?;
    let id = vm.check_number(&c, 1)?;
    let found = engine(vm)
        .units
        .auras
        .iter()
        .find(|a| a.unit == units::PLAYER && a.spell_id == id)
        .cloned();
    Ok(found.map(|a| vec![aura_data(vm, &a)]).unwrap_or_default())
}

/// `GetAuraSlots(unit, filter)`: no continuation, then the slots (the
/// auras' positions, from 1; unverified numbering).
fn get_aura_slots(vm: &mut Vm, c: NativeCall) -> Ret {
    refuse(vm, "GetAuraSlots")?;
    let (unit, filter) = unit_and_filter(vm, &c, 2);
    let Some(unit) = unit else {
        return Ok(vec![Value::Nil]);
    };
    let all: Vec<f64> = engine(vm)
        .units
        .auras
        .iter()
        .filter(|a| a.unit == unit)
        .map(|a| a.instance)
        .collect();
    let mut out = vec![Value::Nil];
    for a in listed(vm, unit, &filter) {
        if let Some(i) = all.iter().position(|&x| x == a.instance) {
            out.push(Value::Num(i as f64 + 1.0));
        }
    }
    Ok(out)
}

fn get_aura_data_by_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    refuse(vm, "GetAuraDataBySlot")?;
    let slot = vm.check_int(&c, 2)?;
    let (unit, _) = unit_and_filter(vm, &c, 3);
    let Some(unit) = unit else { return Ok(vec![]) };
    let found = engine(vm)
        .units
        .auras
        .iter()
        .filter(|a| a.unit == unit)
        .nth(usize::try_from(slot - 1).unwrap_or(usize::MAX))
        .cloned();
    Ok(found.map(|a| vec![aura_data(vm, &a)]).unwrap_or_default())
}

/// The aura functions with behaviour.
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_UnitAuras.GetAuraDataByIndex", get_aura_data_by_index),
        ("C_UnitAuras.GetBuffDataByIndex", get_buff_data_by_index),
        ("C_UnitAuras.GetDebuffDataByIndex", get_debuff_data_by_index),
        (
            "C_UnitAuras.GetAuraDataByAuraInstanceID",
            get_aura_data_by_instance,
        ),
        (
            "C_UnitAuras.GetAuraDataBySpellName",
            get_aura_data_by_spell_name,
        ),
        (
            "C_UnitAuras.GetPlayerAuraBySpellID",
            get_player_aura_by_spell_id,
        ),
        ("C_UnitAuras.GetAuraSlots", get_aura_slots),
        ("C_UnitAuras.GetAuraDataBySlot", get_aura_data_by_slot),
    ]
}
