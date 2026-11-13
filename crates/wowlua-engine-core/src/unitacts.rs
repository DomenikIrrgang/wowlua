//! What units do on their own (M13, decision T518): a unit's actions,
//! each when (`at` seconds after it appeared, `every` so many seconds,
//! `within` so many yards of the player) and what: `attack` (a melee swing
//! at the player every `speed` seconds while within reach, as
//! `unit:swing` swings), `cast` (`spell`, as `unit:castSpell` casts it) or
//! `face` (it turns to the player). They run each frame, after the units
//! walk (T517).

use wowlua_vm::{TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut};

/// A swing reaches this far (yards).
pub const MELEE: f64 = 5.0;

/// When an action happens.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    /// Once, this many seconds after the unit appeared.
    At(f64),
    /// Every so many seconds, the first after that long.
    Every(f64),
    /// Each time the player comes within so many yards.
    Within(f64),
}

/// What it does.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    /// From then on a swing at the player every `speed` seconds while
    /// within reach, `damage` each.
    Attack {
        damage: f64,
        speed: f64,
    },
    Cast {
        spell: f64,
        cast_time: Option<f64>,
    },
    Face,
}

/// An action and how far along it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Act {
    pub when: When,
    pub what: What,
    /// The next time it happens (seconds on the clock); none: not yet
    /// armed.
    next: Option<f64>,
    /// Done (`at`), or the player within (`within`).
    done: bool,
    /// An attack started: its next swing.
    swing: Option<f64>,
}

fn number(vm: &mut Vm, t: TableRef, k: &str) -> Option<f64> {
    let key = vm.str(k);
    match vm.heap.table(t).get(key) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// A unit's `actions` (a world profile's, `spawnUnit`'s, `update`'s): a
/// list of `{ at | every | within, do = "attack" | "cast" | "face",
/// damage?, speed?, spell?, castTime? }`.
pub fn acts_of(vm: &mut Vm, v: Value, attack: Value) -> Result<Vec<Act>, String> {
    let Value::Table(list) = v else {
        return Ok(Vec::new());
    };
    // (the unit's `attack`: its swings' damage and speed where an action
    // says none, decision T530)
    let (damage, speed) = match attack {
        Value::Table(a) => (number(vm, a, "damage"), number(vm, a, "speed")),
        _ => (None, None),
    };
    let mut acts = Vec::new();
    for i in 1.. {
        let Value::Table(t) = vm.heap.table(list).get(Value::Num(i as f64)) else {
            break;
        };
        let when = match (
            number(vm, t, "at"),
            number(vm, t, "every"),
            number(vm, t, "within"),
        ) {
            (Some(at), None, None) => When::At(at),
            (None, Some(n), None) if n > 0.0 => When::Every(n),
            (None, None, Some(d)) => When::Within(d),
            _ => return Err("actions: each one is `at`, `every` (above 0) or `within`".into()),
        };
        let key = vm.str("do");
        let what = match vm.heap.table(t).get(key) {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        };
        let what = match what.as_str() {
            "attack" => What::Attack {
                damage: number(vm, t, "damage").or(damage).unwrap_or(10.0),
                speed: number(vm, t, "speed").or(speed).unwrap_or(2.0).max(0.1),
            },
            "cast" => What::Cast {
                spell: number(vm, t, "spell").ok_or("actions: a cast needs its `spell`")?,
                cast_time: number(vm, t, "castTime"),
            },
            "face" => What::Face,
            other => {
                return Err(format!(
                    "actions: `do = \"{other}\"` is none of attack, cast, face"
                ));
            }
        };
        acts.push(Act {
            when,
            what,
            next: None,
            done: false,
            swing: None,
        });
    }
    Ok(acts)
}

/// The units' actions take a frame.
pub fn tick(vm: &mut Vm) {
    let now = vm.clock.elapsed_ms / 1000.0;
    let acting: Vec<usize> = engine(vm)
        .units
        .list
        .iter()
        .enumerate()
        .filter(|(i, u)| !u.acts.is_empty() && *i != units::PLAYER.0 && !u.dead)
        .map(|(i, _)| i)
        .collect();
    for i in acting {
        let id = UnitId(i);
        let born = engine(vm).units.list[i].born;
        let distance = engine(vm).units.list[i].distance;
        for k in 0..engine(vm).units.list[i].acts.len() {
            let Some(mut act) = engine(vm).units.list[i].acts.get(k).cloned() else {
                break;
            };
            let due = match act.when {
                When::At(at) => !act.done && now - born >= at,
                When::Every(n) => {
                    let next = *act.next.get_or_insert(born + n);
                    now >= next
                }
                When::Within(d) => {
                    let inside = distance.is_some_and(|x| x <= d);
                    let entered = inside && !act.done;
                    act.done = inside;
                    entered
                }
            };
            if due {
                match act.when {
                    When::At(_) => act.done = true,
                    When::Every(n) => act.next = Some(act.next.unwrap_or(now) + n),
                    When::Within(_) => {}
                }
                match act.what {
                    What::Attack { .. } => {
                        act.swing.get_or_insert(now);
                    }
                    What::Cast { spell, cast_time } => cast(vm, id, spell, cast_time),
                    What::Face => face(vm, id),
                }
            }
            // (an attack started: a swing each `speed` seconds in reach)
            if let (What::Attack { damage, speed }, Some(next)) = (&act.what, act.swing)
                && now >= next
            {
                act.swing = Some(next + speed);
                if engine(vm).units.list[i]
                    .distance
                    .is_some_and(|d| d <= MELEE)
                {
                    swing(vm, id, *damage);
                }
            }
            if let Some(slot) = engine_mut(vm).units.list[i].acts.get_mut(k) {
                *slot = act;
            }
        }
    }
}

/// `unit:castSpell(spell, { castTime })`, as a test casts it.
fn cast(vm: &mut Vm, id: UnitId, spell: f64, cast_time: Option<f64>) {
    let me = units::handle(vm, id);
    let opts = vm.new_table();
    if let Some(t) = cast_time {
        let k = vm.str("castTime");
        let _ = vm.heap.table_mut(opts).set(k, Value::Num(t));
    }
    let _ = crate::widgets::call_method(
        vm,
        me,
        "castSpell",
        &[Value::Num(spell), Value::Table(opts)],
    );
}

/// `unit:swing(player, damage)`, as a test swings it.
fn swing(vm: &mut Vm, id: UnitId, damage: f64) {
    let me = units::handle(vm, id);
    let player = units::handle(vm, units::PLAYER);
    let _ = crate::widgets::call_method(vm, me, "swing", &[player, Value::Num(damage)]);
}

/// The unit turns to the player.
fn face(vm: &mut Vm, id: UnitId) {
    let at = crate::character::place_of(vm, id);
    let player = crate::character::place_of(vm, units::PLAYER);
    if let (Some(at), Some(player)) = (at, player) {
        engine_mut(vm).units.list[id.0].facing = Some(crate::character::facing_toward(at, player));
    }
}
