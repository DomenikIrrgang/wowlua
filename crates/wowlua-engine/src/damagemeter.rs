//! The damage meter (`C_DamageMeter`): the player's combat sessions, kept
//! from the fights the engine plays, and their updates.
//!
//! Recorded (the scenarios `combat`, `quest`, `death`, `auradeath` and
//! `talents`, every frame of their fights): each blow the player deals
//! tells `DAMAGE_METER_COMBAT_SESSION_UPDATED(type, sessionID)` for
//! `DamageDone` (0) and `Dps` (1) of the current session, then of the
//! overall one (sessionID 0), then `EnemyDamageTaken` (10) of both; each
//! blow the player takes `DamageTaken` (7) of both; a heal of the player's
//! `HealingDone` (2) and `Hps` (3), as the blow's; the player's death
//! `Deaths` (9). A blow that deals nothing (a parry, a dodge) tells
//! nothing. All of them wait for the frame's end (`batch`). Blizzard's
//! damage meter says the same: "Two events are always dispatched: one for
//! the overall session (with no sessionID) and one for the current session
//! with its sessionID" (`DamageMeterSessionWindow.lua`).
//!
//! A session begins with the player's combat or the first blow before it
//! (`DAMAGE_METER_CURRENT_SESSION_UPDATED`): `talents`' fox putting the
//! player in combat without a blow, `quest`'s first swing before the
//! combat. A fight that begins soon after the last one's end goes on in
//! its session (`quest`: 2.756 s after, the same session; 3.329 s and
//! more, a new one; unverified: the bound, 3 s here). The sessions' IDs
//! count up from one the client's history decides (2, 3, 4 and 11 in the
//! recordings: a test sets it, `setDamageMeterSession`).
//!
//! Unverified (never recorded): the sessions' contents. As documented
//! (`DamageMeterDocumentation.lua`): the sources by amount, highest first;
//! their spells; the session's duration. The player's and their group's
//! blows count as theirs (`DamageDone`), the enemies they hit as
//! `EnemyDamageTaken`'s sources, the friendly units hit as `DamageTaken`'s.
//! Secret in combat as documented: the amounts, names and GUIDs, the
//! fields documented `NeverSecret` plain, the lists readable (Details!'s
//! Forever build reads `issecretvalue(session.combatSources[1].name)` to
//! tell the server's combat).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{PLAYER, UnitId};
use crate::{engine, engine_mut};

/// `Enum.DamageMeterType`.
const DAMAGE_DONE: u8 = 0;
const DPS: u8 = 1;
const HEALING_DONE: u8 = 2;
const HPS: u8 = 3;
const DAMAGE_TAKEN: u8 = 7;
const DEATHS: u8 = 9;
const ENEMY_DAMAGE_TAKEN: u8 = 10;
const TYPES: u8 = 11;

/// A fight that begins within this many seconds of the last one's end goes
/// on in its session (unverified bound: see the module).
const RESUME_WITHIN: f64 = 3.0;

/// The melee swing's spell (Auto Attack).
pub const MELEE: f64 = 6603.0;

/// What a source did with one spell.
#[derive(Clone)]
struct Spell {
    id: f64,
    total: f64,
    /// The unit at the other end (the target, or the attacker of a blow
    /// taken).
    other: UnitId,
}

/// One unit's numbers in a session, for one type.
#[derive(Clone)]
struct Source {
    unit: UnitId,
    total: f64,
    spells: Vec<Spell>,
    /// Seconds into the session it died (`Deaths`).
    died: f64,
}

#[derive(Clone)]
struct Session {
    id: f64,
    start: f64,
    end: Option<f64>,
    types: Vec<Vec<Source>>,
}

impl Session {
    fn new(id: f64, start: f64) -> Session {
        Session {
            id,
            start,
            end: None,
            types: vec![Vec::new(); TYPES as usize],
        }
    }

    fn duration(&self, now: f64) -> f64 {
        (self.end.unwrap_or(now) - self.start).max(0.0)
    }

    fn add(&mut self, ty: u8, unit: UnitId, amount: f64, spell: f64, other: UnitId, died: f64) {
        let list = &mut self.types[ty as usize];
        let i = match list.iter().position(|s| s.unit == unit) {
            Some(i) => i,
            None => {
                list.push(Source {
                    unit,
                    total: 0.0,
                    spells: Vec::new(),
                    died: 0.0,
                });
                list.len() - 1
            }
        };
        let s = &mut list[i];
        s.total += amount;
        s.died = died;
        match s.spells.iter_mut().find(|x| x.id == spell) {
            Some(x) => x.total += amount,
            None => s.spells.push(Spell {
                id: spell,
                total: amount,
                other,
            }),
        }
    }
}

pub struct DamageMeter {
    /// The ID the next session gets.
    next_id: f64,
    sessions: Vec<Session>,
    /// Every session's numbers together (sessionID 0).
    overall: Option<Session>,
    /// The player's fight is on (its session open).
    open: bool,
}

impl Default for DamageMeter {
    fn default() -> Self {
        DamageMeter {
            next_id: 1.0,
            sessions: Vec::new(),
            overall: None,
            open: false,
        }
    }
}

fn fire(vm: &mut Vm, ty: u8, id: f64) {
    crate::widgets::fire_event(
        vm,
        "DAMAGE_METER_COMBAT_SESSION_UPDATED",
        &[Value::Num(f64::from(ty)), Value::Num(id)],
    );
}

/// The current session's ID. Out of the player's fight, a blow (`blow`)
/// counts in the session that ended a moment ago (`talents`: the fox's
/// last hit as the combat ends), else begins one (`quest`: the first
/// swing before the combat); the fight's beginning resumes that session
/// or begins one.
fn current(vm: &mut Vm, blow: bool) -> f64 {
    let now = crate::clock::now(vm);
    let m = &mut engine_mut(vm).damage_meter;
    if !m.open {
        let recent = m
            .sessions
            .last()
            .and_then(|s| s.end)
            .is_some_and(|end| now - end < RESUME_WITHIN);
        if recent && blow {
            return m.sessions.last().map_or(0.0, |s| s.id);
        }
        m.open = true;
        if recent {
            if let Some(s) = m.sessions.last_mut() {
                s.end = None;
            }
        } else {
            let id = m.next_id;
            m.next_id += 1.0;
            m.sessions.push(Session::new(id, now));
            if m.overall.is_none() {
                m.overall = Some(Session::new(0.0, now));
            }
            crate::widgets::fire_event(vm, "DAMAGE_METER_CURRENT_SESSION_UPDATED", &[]);
        }
        if let Some(o) = engine_mut(vm).damage_meter.overall.as_mut() {
            o.end = None;
        }
    }
    let m = &engine(vm).damage_meter;
    m.sessions.last().map_or(0.0, |s| s.id)
}

/// Records `amount` of `ty` for `unit` in the current and overall sessions.
fn add(vm: &mut Vm, ty: u8, unit: UnitId, amount: f64, spell: f64, other: UnitId) {
    let now = crate::clock::now(vm);
    let m = &mut engine_mut(vm).damage_meter;
    if let Some(s) = m.sessions.last_mut() {
        let died = now - s.start;
        s.add(ty, unit, amount, spell, other, died);
    }
    if let Some(o) = m.overall.as_mut() {
        let died = now - o.start;
        o.add(ty, unit, amount, spell, other, died);
    }
}

fn ours(vm: &Vm, id: UnitId) -> bool {
    crate::units::ours(&engine(vm).units, id)
}

/// The player's fight began (`PLAYER_REGEN_DISABLED`).
pub fn combat_began(vm: &mut Vm) {
    current(vm, false);
}

/// The player's fight ended (`PLAYER_REGEN_ENABLED`).
pub fn combat_ended(vm: &mut Vm) {
    let now = crate::clock::now(vm);
    let m = &mut engine_mut(vm).damage_meter;
    if !m.open {
        return;
    }
    m.open = false;
    if let Some(s) = m.sessions.last_mut() {
        s.end = Some(now);
    }
    if let Some(o) = m.overall.as_mut() {
        o.end = Some(now);
    }
}

/// A blow of `amount` from `by` (none: the world's) on `victim` with
/// `spell`.
pub fn damage(vm: &mut Vm, by: Option<UnitId>, victim: UnitId, amount: f64, spell: f64) {
    if amount <= 0.0 {
        return;
    }
    let dealt = by.is_some_and(|b| ours(vm, b) && !ours(vm, victim));
    let taken = ours(vm, victim);
    if !dealt && !taken {
        return;
    }
    let id = current(vm, true);
    let other = by.unwrap_or(victim);
    if let (true, Some(b)) = (dealt, by) {
        add(vm, DAMAGE_DONE, b, amount, spell, victim);
        add(vm, DPS, b, amount, spell, victim);
        add(vm, ENEMY_DAMAGE_TAKEN, victim, amount, spell, b);
        for ty in [DAMAGE_DONE, DPS] {
            fire(vm, ty, id);
        }
        for ty in [DAMAGE_DONE, DPS] {
            fire(vm, ty, 0.0);
        }
        fire(vm, ENEMY_DAMAGE_TAKEN, id);
        fire(vm, ENEMY_DAMAGE_TAKEN, 0.0);
    }
    if taken {
        add(vm, DAMAGE_TAKEN, victim, amount, spell, other);
        fire(vm, DAMAGE_TAKEN, id);
        fire(vm, DAMAGE_TAKEN, 0.0);
    }
}

/// A heal of `amount` from `by` on `target` with `spell`.
pub fn heal(vm: &mut Vm, by: UnitId, target: UnitId, amount: f64, spell: f64) {
    if amount <= 0.0 || !ours(vm, by) {
        return;
    }
    let id = current(vm, true);
    add(vm, HEALING_DONE, by, amount, spell, target);
    add(vm, HPS, by, amount, spell, target);
    for ty in [HEALING_DONE, HPS] {
        fire(vm, ty, id);
    }
    for ty in [HEALING_DONE, HPS] {
        fire(vm, ty, 0.0);
    }
}

/// The player (or a group member) died.
pub fn died(vm: &mut Vm, unit: UnitId) {
    if !ours(vm, unit) {
        return;
    }
    let id = current(vm, true);
    add(vm, DEATHS, unit, 1.0, 0.0, unit);
    fire(vm, DEATHS, id);
    fire(vm, DEATHS, 0.0);
}

// ------------------------------------------------------------ the API

fn set(vm: &mut Vm, t: TableRef, k: &str, v: Value) {
    let k = vm.str(k);
    let _ = vm.heap.table_mut(t).set(k, v);
}

/// The session a type and kind name: `Overall` 0, `Current` 1 (the last
/// one begun), `Expired` 2 (none kept).
fn session_of_type(vm: &Vm, session_type: f64) -> Option<Session> {
    let m = &engine(vm).damage_meter;
    match session_type as i64 {
        0 => m.overall.clone(),
        1 => m.sessions.last().cloned(),
        _ => None,
    }
}

fn session_of_id(vm: &Vm, id: f64) -> Option<Session> {
    let m = &engine(vm).damage_meter;
    if id == 0.0 {
        return m.overall.clone();
    }
    m.sessions.iter().find(|s| s.id == id).cloned()
}

/// Whether the sessions' numbers are secret now (in combat, as
/// documented: `SecretWhenInCombat`).
fn secret_now(vm: &Vm) -> bool {
    crate::units::active(&engine(vm).units, &[crate::units::COMBAT])
}

fn maybe_secret(vm: &mut Vm, v: Value, secret: bool) -> Value {
    if secret && !v.is_nil() {
        vm.secret_wrap(v)
    } else {
        v
    }
}

/// A creature's ID, from its GUID (`Creature-0-…-<id>-<spawn>`).
fn creature_id(guid: &str) -> Option<f64> {
    let parts: Vec<&str> = guid.split('-').collect();
    match parts.first() {
        Some(&"Creature") | Some(&"Vehicle") | Some(&"Pet") if parts.len() >= 7 => {
            parts[5].parse().ok()
        }
        _ => None,
    }
}

/// A unit's facts a session tells: name, GUID, class file name,
/// classification, player, faction.
fn unit_facts(vm: &Vm, id: UnitId) -> (String, String, String, String, bool, Option<String>) {
    let u = &engine(vm).units.list[id.0];
    let classification = if u.classification.is_empty() {
        "normal".to_string()
    } else {
        u.classification.clone()
    };
    (
        u.name.clone(),
        u.guid.clone(),
        u.class.clone(),
        classification,
        u.player,
        u.faction_group.clone(),
    )
}

fn session_table(vm: &mut Vm, s: Option<Session>, ty: f64) -> Value {
    let secret = secret_now(vm);
    let now = crate::clock::now(vm);
    let t = vm.new_table();
    let sources = vm.new_table();
    set(vm, t, "combatSources", Value::Table(sources));
    let (mut total, mut max, mut duration) = (0.0, 0.0, None);
    if let Some(s) = &s {
        let d = s.duration(now);
        duration = Some(d.floor());
        let mut list = s.types.get(ty as usize).cloned().unwrap_or_default();
        list.sort_by(|a, b| b.total.total_cmp(&a.total));
        for (i, src) in list.iter().enumerate() {
            total += src.total;
            max = f64::max(max, src.total);
            let e = vm.new_table();
            let (name, guid, class, classification, player, faction) = unit_facts(vm, src.unit);
            let ally = ours(vm, src.unit);
            let v = vm.str(&guid);
            let v = maybe_secret(vm, v, secret);
            set(vm, e, "sourceGUID", v);
            if !player && let Some(c) = creature_id(&guid) {
                set(vm, e, "sourceCreatureID", Value::Num(c));
            }
            let v = vm.str(&name);
            let v = maybe_secret(vm, v, secret);
            set(vm, e, "name", v);
            let v = vm.str(&class);
            set(vm, e, "classFilename", v);
            set(vm, e, "specIconID", Value::Num(0.0));
            let v = maybe_secret(vm, Value::Num(src.total), secret);
            set(vm, e, "totalAmount", v);
            let per = if d > 0.0 { src.total / d } else { src.total };
            let v = maybe_secret(vm, Value::Num(per), secret);
            set(vm, e, "amountPerSecond", v);
            set(vm, e, "isLocalPlayer", Value::Bool(src.unit == PLAYER));
            set(vm, e, "deathRecapID", Value::Num(0.0));
            let died = if ty as u8 == DEATHS { src.died } else { 0.0 };
            let v = maybe_secret(vm, Value::Num(died), secret);
            set(vm, e, "deathTimeSeconds", v);
            let v = vm.str(&classification);
            set(vm, e, "classification", v);
            // `Enum.DamageMeterSourceDisplayType`: Ally 1, Enemy 2.
            let display = if ally { 1.0 } else { 2.0 };
            let v = maybe_secret(vm, Value::Num(display), secret);
            set(vm, e, "sourceDisplayType", v);
            if let Some(f) = faction {
                let v = vm.str(&f);
                let v = maybe_secret(vm, v, secret);
                set(vm, e, "factionGroup", v);
            }
            let _ = vm
                .heap
                .table_mut(sources)
                .set(Value::Num(i as f64 + 1.0), Value::Table(e));
        }
    }
    let v = maybe_secret(vm, Value::Num(max), secret);
    set(vm, t, "maxAmount", v);
    let v = maybe_secret(vm, Value::Num(total), secret);
    set(vm, t, "totalAmount", v);
    if let Some(d) = duration {
        let v = maybe_secret(vm, Value::Num(d), secret);
        set(vm, t, "durationSeconds", v);
    }
    Value::Table(t)
}

/// A source's spells (`combatSpells`), by amount.
fn source_table(vm: &mut Vm, s: Option<Session>, ty: f64, guid: Value, creature: Value) -> Value {
    let secret = secret_now(vm);
    let now = crate::clock::now(vm);
    let t = vm.new_table();
    let spells = vm.new_table();
    set(vm, t, "combatSpells", Value::Table(spells));
    let guid = match guid {
        Value::Str(g) => Some(String::from_utf8_lossy(vm.str_bytes(g)).into_owned()),
        _ => None,
    };
    let creature = match creature {
        Value::Num(n) => Some(n),
        _ => None,
    };
    let (mut total, mut max) = (0.0, 0.0);
    if let Some(s) = &s {
        let d = s.duration(now);
        let list = s.types.get(ty as usize).cloned().unwrap_or_default();
        let found = list.into_iter().find(|src| {
            let g = &engine(vm).units.list[src.unit.0].guid;
            guid.as_deref() == Some(g.as_str())
                || (guid.is_none() && creature.is_some() && creature == creature_id(g))
        });
        if let Some(src) = found {
            let mut list = src.spells.clone();
            list.sort_by(|a, b| b.total.total_cmp(&a.total));
            for (i, sp) in list.iter().enumerate() {
                total += sp.total;
                max = f64::max(max, sp.total);
                let e = vm.new_table();
                let (other, _, other_class, classification, other_player, _) =
                    unit_facts(vm, sp.other);
                set(vm, e, "spellID", Value::Num(sp.id));
                let v = maybe_secret(vm, Value::Num(sp.total), secret);
                set(vm, e, "totalAmount", v);
                let per = if d > 0.0 { sp.total / d } else { sp.total };
                let v = maybe_secret(vm, Value::Num(per), secret);
                set(vm, e, "amountPerSecond", v);
                let v = vm.str(if other_player { "" } else { &other });
                let v = maybe_secret(vm, v, secret);
                set(vm, e, "creatureName", v);
                set(vm, e, "overkillAmount", Value::Num(0.0));
                set(vm, e, "isAvoidable", Value::Bool(false));
                set(vm, e, "isDeadly", Value::Bool(false));
                let details = vm.new_table();
                let v = vm.str(&other);
                let v = maybe_secret(vm, v, secret);
                set(vm, details, "unitName", v);
                let v = vm.str(&other_class);
                set(vm, details, "unitClassFilename", v);
                let v = vm.str(&classification);
                set(vm, details, "classification", v);
                set(vm, details, "isPet", Value::Bool(false));
                set(vm, details, "isMob", Value::Bool(!other_player));
                let v = maybe_secret(vm, Value::Num(sp.total), secret);
                set(vm, details, "amount", v);
                set(vm, details, "specIconID", Value::Num(0.0));
                set(vm, e, "combatSpellDetails", Value::Table(details));
                let _ = vm
                    .heap
                    .table_mut(spells)
                    .set(Value::Num(i as f64 + 1.0), Value::Table(e));
            }
        }
    }
    let v = maybe_secret(vm, Value::Num(max), secret);
    set(vm, t, "maxAmount", v);
    let v = maybe_secret(vm, Value::Num(total), secret);
    set(vm, t, "totalAmount", v);
    Value::Table(t)
}

fn num(c: &NativeCall, i: usize) -> f64 {
    match c.arg(i) {
        Value::Num(n) => n,
        _ => 0.0,
    }
}

/// `GetCombatSessionFromType(sessionType, type)`. Its secrecy its own
/// (see the module): told as built.
fn combat_session_from_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = session_of_type(vm, num(&c, 1));
    let t = session_table(vm, s, num(&c, 2));
    engine_mut(vm).plain_results = true;
    Ok(vec![t])
}

/// `GetCombatSessionFromID(sessionID, type)`.
fn combat_session_from_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = session_of_id(vm, num(&c, 1));
    let t = session_table(vm, s, num(&c, 2));
    engine_mut(vm).plain_results = true;
    Ok(vec![t])
}

/// `GetCombatSessionSourceFromType(sessionType, type, guid?, creatureID?)`.
fn source_from_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = session_of_type(vm, num(&c, 1));
    let t = source_table(vm, s, num(&c, 2), c.arg(3), c.arg(4));
    engine_mut(vm).plain_results = true;
    Ok(vec![t])
}

/// `GetCombatSessionSourceFromID(sessionID, type, guid?, creatureID?)`.
fn source_from_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = session_of_id(vm, num(&c, 1));
    let t = source_table(vm, s, num(&c, 2), c.arg(3), c.arg(4));
    engine_mut(vm).plain_results = true;
    Ok(vec![t])
}

/// `GetAvailableCombatSessions()`: the sessions kept, oldest first, each
/// `{ sessionID, name, durationSeconds }` (unverified: the name empty, as
/// Blizzard's meter allows, naming it by number then).
fn available_sessions(vm: &mut Vm, _c: NativeCall) -> Ret {
    let now = crate::clock::now(vm);
    let list: Vec<(f64, f64)> = engine(vm)
        .damage_meter
        .sessions
        .iter()
        .map(|s| (s.id, s.duration(now).floor()))
        .collect();
    let t = vm.new_table();
    for (i, (id, d)) in list.into_iter().enumerate() {
        let e = vm.new_table();
        set(vm, e, "sessionID", Value::Num(id));
        let name = vm.str("");
        set(vm, e, "name", name);
        set(vm, e, "durationSeconds", Value::Num(d));
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(i as f64 + 1.0), Value::Table(e));
    }
    Ok(vec![Value::Table(t)])
}

/// `GetSessionDurationSeconds(sessionType)`: none before a session.
fn session_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    let now = crate::clock::now(vm);
    Ok(vec![match session_of_type(vm, num(&c, 1)) {
        Some(s) => Value::Num(s.duration(now).floor()),
        None => Value::Nil,
    }])
}

/// `ResetAllCombatSessions()`: every session gone, `DAMAGE_METER_RESET`
/// (documented; unverified: an open fight's session begun again at its
/// next blow).
fn reset(vm: &mut Vm, _c: NativeCall) -> Ret {
    let m = &mut engine_mut(vm).damage_meter;
    m.sessions.clear();
    m.overall = None;
    m.open = false;
    crate::widgets::fire_event(vm, "DAMAGE_METER_RESET", &[]);
    Ok(vec![])
}

/// `setDamageMeterSession(id)`: the ID the next session gets (the
/// client's count of its sessions: a recording's).
pub fn control_set_session(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = vm.check_number(&c, 1)?;
    engine_mut(vm).damage_meter.next_id = id;
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        (
            "C_DamageMeter.GetCombatSessionFromType",
            combat_session_from_type,
        ),
        (
            "C_DamageMeter.GetCombatSessionFromID",
            combat_session_from_id,
        ),
        (
            "C_DamageMeter.GetCombatSessionSourceFromType",
            source_from_type,
        ),
        ("C_DamageMeter.GetCombatSessionSourceFromID", source_from_id),
        (
            "C_DamageMeter.GetAvailableCombatSessions",
            available_sessions,
        ),
        ("C_DamageMeter.GetSessionDurationSeconds", session_duration),
        ("C_DamageMeter.ResetAllCombatSessions", reset),
    ]
}
