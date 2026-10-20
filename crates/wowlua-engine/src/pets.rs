//! The player's pet. No recording has one yet: what this module does is
//! modelled on Blizzard's UI code (the pet frame, its happiness, the pet
//! action bar, the pet's experience bar) and the API documentation, all of
//! it unverified.
//!
//! `mock.setPet(unit, { hunter, happiness, loyalty, xp, xpMax, actions })`:
//! the unit is the player's pet (`pet`: `UNIT_PET("player")`,
//! `PET_BAR_UPDATE`, `UNIT_HAPPINESS("pet")`); called again for the same
//! pet it changes what it gives (its happiness: `UNIT_HAPPINESS`; its
//! experience: `UNIT_PET_EXPERIENCE`; its actions: `PET_BAR_UPDATE`).
//! `mock.setPet(nil)`: dismissed (`UNIT_PET("player")`, `PET_BAR_UPDATE`).
//!
//! The pet's bar is a hunter's (classic's order: attack, follow, stay,
//! four spells, aggressive, defensive, passive; follow and defensive
//! active) unless `actions` replaces slots. `CastPetAction`, `PetAttack`,
//! `PetStopAttack`, `PetFollow`, `PetWait` and the modes switch what is
//! active (`PET_BAR_UPDATE`; attacking: `PET_ATTACK_START`, stopping
//! `PET_ATTACK_STOP`); `TogglePetAutocast` a spell's autocast.
//!
//! Unverified: the events' order and their units, the default bar, the
//! damage percentage by happiness (classic's 125, 100, 75), cooldowns
//! (none), a pet's spells cast, its death's and resurrection's events.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// An action's texture (kept as data: no VM value outlives its call).
#[derive(Clone)]
pub enum Texture {
    None,
    Id(f64),
    Name(String),
}

impl Texture {
    fn of(vm: &Vm, v: Value) -> Texture {
        match v {
            Value::Num(n) => Texture::Id(n),
            Value::Str(s) => Texture::Name(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
            _ => Texture::None,
        }
    }

    fn value(&self, vm: &mut Vm) -> Value {
        match self {
            Texture::None => Value::Nil,
            Texture::Id(n) => Value::Num(*n),
            Texture::Name(s) => vm.str(s),
        }
    }
}

/// A slot of the pet's bar.
#[derive(Clone)]
pub struct Action {
    /// A token's global string's name (`PET_ACTION_ATTACK`), else a spell's
    /// name.
    pub name: String,
    /// A token's texture's global's name, else a file ID or path.
    pub texture: Texture,
    pub token: bool,
    pub spell: Option<f64>,
    pub active: bool,
    pub autocast_allowed: bool,
    pub autocast: bool,
}

#[derive(Default)]
pub struct Pets {
    pub hunter: bool,
    /// 1 unhappy, 2 content, 3 happy.
    pub happiness: f64,
    pub loyalty: String,
    pub xp: (f64, f64),
    pub actions: Vec<Option<Action>>,
    pub attacking: bool,
    /// Its stats (`pet:setStats`); the default character's stand-ins
    /// until a test gives them (unverified).
    pub stats: crate::stats::Stats,
}

const SLOTS: usize = 10;

fn token(name: &str, texture: &str, active: bool) -> Action {
    Action {
        name: name.into(),
        texture: Texture::Name(texture.into()),
        token: true,
        spell: None,
        active,
        autocast_allowed: false,
        autocast: false,
    }
}

/// The hunter's bar (unverified: classic's layout).
fn default_bar() -> Vec<Option<Action>> {
    let mut bar: Vec<Option<Action>> = vec![None; SLOTS];
    for (slot, name, texture, active) in [
        (0, "PET_ACTION_ATTACK", "PET_ATTACK_TEXTURE", false),
        (1, "PET_ACTION_FOLLOW", "PET_FOLLOW_TEXTURE", true),
        (2, "PET_ACTION_WAIT", "PET_WAIT_TEXTURE", false),
        (7, "PET_MODE_AGGRESSIVE", "PET_AGGRESSIVE_TEXTURE", false),
        (8, "PET_MODE_DEFENSIVE", "PET_DEFENSIVE_TEXTURE", true),
        (9, "PET_MODE_PASSIVE", "PET_PASSIVE_TEXTURE", false),
    ] {
        bar[slot] = Some(token(name, texture, active));
    }
    bar
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn get(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn pet(vm: &Vm) -> Option<UnitId> {
    engine(vm).units.pet
}

/// `mock.setPet(unit?, opts?)`.
pub fn control_set_pet(vm: &mut Vm, c: NativeCall) -> Ret {
    let unit = match c.arg(1) {
        Value::Nil => None,
        v => match units::handle_of(vm, v) {
            Some(id) => Some(id),
            None => return Err(vm.error("setPet: a unit (or nil) expected")),
        },
    };
    let Some(id) = unit else {
        if engine_mut(vm).units.pet.take().is_some() {
            engine_mut(vm).world.pets = Pets::default();
            let player = vm.str("player");
            fire(vm, "UNIT_PET", &[player]);
            fire(vm, "PET_BAR_UPDATE", &[]);
        }
        return Ok(vec![]);
    };
    let opts = match c.arg(2) {
        Value::Table(t) => Some(t),
        _ => None,
    };
    let new = pet(vm) != Some(id);
    if new {
        let bar = default_bar();
        engine_mut(vm).world.pets = Pets {
            hunter: true,
            happiness: 3.0,
            loyalty: String::new(),
            xp: (0.0, 0.0),
            actions: bar,
            attacking: false,
            stats: crate::stats::defaults(),
        };
    }
    let (mut happy_changed, mut xp_changed, mut bar_changed) = (false, false, false);
    if let Some(t) = opts {
        if let Value::Bool(b) = get(vm, t, "hunter") {
            engine_mut(vm).world.pets.hunter = b;
        }
        if let Value::Num(n) = get(vm, t, "happiness") {
            happy_changed = engine(vm).world.pets.happiness != n;
            engine_mut(vm).world.pets.happiness = n;
        }
        if let Value::Str(s) = get(vm, t, "loyalty") {
            let s = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            engine_mut(vm).world.pets.loyalty = s;
        }
        let (xp, max) = engine(vm).world.pets.xp;
        let xp = match get(vm, t, "xp") {
            Value::Num(n) => n,
            _ => xp,
        };
        let max = match get(vm, t, "xpMax") {
            Value::Num(n) => n,
            _ => max,
        };
        xp_changed = (xp, max) != engine(vm).world.pets.xp;
        engine_mut(vm).world.pets.xp = (xp, max);
        if let Value::Table(list) = get(vm, t, "actions") {
            bar_changed = true;
            for slot in 1..=SLOTS {
                let Value::Table(a) = vm.heap.table(list).get(Value::Num(slot as f64)) else {
                    continue;
                };
                let action = action_of(vm, a);
                engine_mut(vm).world.pets.actions[slot - 1] = Some(action);
            }
        }
    }
    let player = vm.str("player");
    let pet_token = vm.str("pet");
    if new {
        engine_mut(vm).units.pet = Some(id);
        fire(vm, "UNIT_PET", &[player]);
        fire(vm, "PET_BAR_UPDATE", &[]);
        fire(vm, "UNIT_HAPPINESS", &[pet_token]);
        return Ok(vec![]);
    }
    if happy_changed {
        fire(vm, "UNIT_HAPPINESS", &[pet_token]);
    }
    if xp_changed {
        fire(vm, "UNIT_PET_EXPERIENCE", &[pet_token]);
    }
    if bar_changed {
        fire(vm, "PET_BAR_UPDATE", &[]);
    }
    Ok(vec![])
}

/// An `actions` entry: `{ spell, name, icon, autocast?, autocastAllowed? }`
/// or `{ token = "PET_ACTION_ATTACK", texture = "PET_ATTACK_TEXTURE",
/// active? }`.
fn action_of(vm: &mut Vm, t: TableRef) -> Action {
    let token = get(vm, t, "token");
    let active = get(vm, t, "active").truthy();
    if let Value::Str(_) = token {
        let name = match token {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        };
        return Action {
            name,
            texture: {
                let v = get(vm, t, "texture");
                Texture::of(vm, v)
            },
            token: true,
            spell: None,
            active,
            autocast_allowed: false,
            autocast: false,
        };
    }
    let name = match get(vm, t, "name") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    let autocast = get(vm, t, "autocast").truthy();
    Action {
        name,
        texture: {
            let v = get(vm, t, "icon");
            Texture::of(vm, v)
        },
        token: false,
        spell: match get(vm, t, "spell") {
            Value::Num(n) => Some(n),
            _ => None,
        },
        active,
        autocast_allowed: autocast || get(vm, t, "autocastAllowed").truthy(),
        autocast,
    }
}

fn slot_arg(vm: &mut Vm, c: &NativeCall) -> Option<usize> {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) if (1.0..=SLOTS as f64).contains(&n) => Some(n as usize - 1),
        _ => None,
    }
}

fn action(vm: &mut Vm, c: &NativeCall) -> Option<Action> {
    pet(vm)?;
    let i = slot_arg(vm, c)?;
    engine(vm).world.pets.actions.get(i).cloned().flatten()
}

/// `HasPetUI()`: a pet, and whether a hunter's (recorded without one:
/// `false, false`).
fn has_pet_ui(vm: &mut Vm, _c: NativeCall) -> Ret {
    let has = pet(vm).is_some();
    Ok(vec![
        Value::Bool(has),
        Value::Bool(has && engine(vm).world.pets.hunter),
    ])
}

/// `PetHasActionBar()`: with a pet (recorded without one: false).
fn pet_has_action_bar(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(pet(vm).is_some())])
}

/// `GetPetActionInfo(slot)`: name, texture, isToken, isActive,
/// autoCastAllowed, autoCastEnabled, spellID, checksRange, inRange.
fn get_pet_action_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(a) = action(vm, &c) else {
        return Ok(vec![]);
    };
    let name = vm.str(&a.name);
    let texture = a.texture.value(vm);
    Ok(vec![
        name,
        texture,
        Value::Bool(a.token),
        Value::Bool(a.active),
        Value::Bool(a.autocast_allowed),
        Value::Bool(a.autocast),
        a.spell.map_or(Value::Nil, Value::Num),
        Value::Bool(false),
        Value::Bool(false),
    ])
}

/// `GetPetActionCooldown(slot)`: none, `0, 0, 0` (recorded without a pet,
/// every slot; with one unverified).
fn get_pet_action_cooldown(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0), Value::Num(0.0)])
}

/// `GetPetActionSlotUsable(slot)`: an action there, the pet alive; with
/// no pet, any slot (the full probe: -1, 0, 1 and 2 true without a pet;
/// unverified why).
fn get_pet_action_slot_usable(vm: &mut Vm, c: NativeCall) -> Ret {
    if pet(vm).is_none() {
        return Ok(vec![Value::Bool(true)]);
    }
    let usable = action(vm, &c).is_some() && pet_alive(vm);
    Ok(vec![Value::Bool(usable)])
}

fn pet_uses_pet_frame(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(pet(vm).is_some())])
}

fn pet_alive(vm: &Vm) -> bool {
    pet(vm).is_some_and(|id| !engine(vm).units.list[id.0].dead)
}

/// `GetPetActionsUsable()`: the pet alive.
fn get_pet_actions_usable(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(pet_alive(vm))])
}

/// `IsPetAttackAction(slot)`.
fn is_pet_attack_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let attack = action(vm, &c).is_some_and(|a| a.token && a.name == "PET_ACTION_ATTACK");
    Ok(vec![Value::Bool(attack)])
}

fn is_pet_attack_active(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).world.pets.attacking)])
}

/// `C_PetInfo.GetPetHappiness()`: the happiness, the damage it deals in
/// percent, the loyalty's rate.
fn get_pet_happiness(vm: &mut Vm, _c: NativeCall) -> Ret {
    if pet(vm).is_none() {
        return Ok(vec![]);
    }
    let h = engine(vm).world.pets.happiness;
    let damage = match h as i64 {
        1 => 75.0,
        2 => 100.0,
        _ => 125.0,
    };
    Ok(vec![Value::Num(h), Value::Num(damage), Value::Num(0.0)])
}

/// `C_PetInfo.GetPetLoyalty()`: the loyalty's name.
fn get_pet_loyalty(vm: &mut Vm, _c: NativeCall) -> Ret {
    // No pet: nothing (the full probe, 1.60.1, a paladin).
    if engine(vm).units.pet.is_none() {
        return Ok(vec![]);
    }
    let l = engine(vm).world.pets.loyalty.clone();
    Ok(vec![vm.str(&l)])
}

/// `GetPetExperience()`: the experience and the level's end.
fn get_pet_experience(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (xp, max) = engine(vm).world.pets.xp;
    Ok(vec![Value::Num(xp), Value::Num(max)])
}

/// The pet attacks (`PET_ATTACK_START`), its attack token active.
fn attack(vm: &mut Vm) {
    if pet(vm).is_none() || engine(vm).world.pets.attacking {
        return;
    }
    engine_mut(vm).world.pets.attacking = true;
    set_active(vm, &["PET_ACTION_ATTACK"], &["PET_ACTION_ATTACK"]);
    fire(vm, "PET_ATTACK_START", &[]);
    fire(vm, "PET_BAR_UPDATE", &[]);
}

fn stop_attack(vm: &mut Vm) {
    if !engine(vm).world.pets.attacking {
        return;
    }
    engine_mut(vm).world.pets.attacking = false;
    set_active(vm, &["PET_ACTION_ATTACK"], &[]);
    fire(vm, "PET_ATTACK_STOP", &[]);
    fire(vm, "PET_BAR_UPDATE", &[]);
}

/// Among the tokens `group`, those in `on` active, the others not.
fn set_active(vm: &mut Vm, group: &[&str], on: &[&str]) {
    for a in engine_mut(vm).world.pets.actions.iter_mut().flatten() {
        if a.token && group.contains(&a.name.as_str()) {
            a.active = on.contains(&a.name.as_str());
        }
    }
}

const COMMANDS: &[&str] = &["PET_ACTION_FOLLOW", "PET_ACTION_WAIT"];
const MODES: &[&str] = &[
    "PET_MODE_AGGRESSIVE",
    "PET_MODE_DEFENSIVE",
    "PET_MODE_PASSIVE",
    "PET_MODE_ASSIST",
    "PET_MODE_DEFENSIVEASSIST",
];

fn command(vm: &mut Vm, name: &str) {
    if name == "PET_ACTION_ATTACK" {
        attack(vm);
        return;
    }
    if COMMANDS.contains(&name) {
        stop_attack(vm);
        set_active(vm, COMMANDS, &[name]);
    } else if MODES.contains(&name) {
        set_active(vm, MODES, &[name]);
    }
    fire(vm, "PET_BAR_UPDATE", &[]);
}

/// `CastPetAction(slot)`: a token's command, a spell's nothing yet.
fn cast_pet_action(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(a) = action(vm, &c)
        && a.token
    {
        command(vm, &a.name);
    }
    Ok(vec![])
}

fn pet_attack(vm: &mut Vm, _c: NativeCall) -> Ret {
    attack(vm);
    Ok(vec![])
}

fn pet_stop_attack(vm: &mut Vm, _c: NativeCall) -> Ret {
    stop_attack(vm);
    Ok(vec![])
}

fn pet_follow(vm: &mut Vm, _c: NativeCall) -> Ret {
    command(vm, "PET_ACTION_FOLLOW");
    Ok(vec![])
}

fn pet_wait(vm: &mut Vm, _c: NativeCall) -> Ret {
    command(vm, "PET_ACTION_WAIT");
    Ok(vec![])
}

/// `TogglePetAutocast(slot)`: a spell's autocast on or off.
fn toggle_pet_autocast(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = slot_arg(vm, &c) else {
        return Ok(vec![]);
    };
    if pet(vm).is_none() {
        return Ok(vec![]);
    }
    let changed = match engine_mut(vm).world.pets.actions.get_mut(i) {
        Some(Some(a)) if a.autocast_allowed => {
            a.autocast = !a.autocast;
            true
        }
        _ => false,
    };
    if changed {
        fire(vm, "PET_BAR_UPDATE", &[]);
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("HasPetUI", has_pet_ui),
        ("PetHasActionBar", pet_has_action_bar),
        // With a pet (the full probe: false without one).
        ("PetUsesPetFrame", pet_uses_pet_frame),
        ("GetPetActionInfo", get_pet_action_info),
        ("GetPetActionCooldown", get_pet_action_cooldown),
        ("GetPetActionSlotUsable", get_pet_action_slot_usable),
        ("GetPetActionsUsable", get_pet_actions_usable),
        ("IsPetAttackAction", is_pet_attack_action),
        ("IsPetAttackActive", is_pet_attack_active),
        ("C_PetInfo.GetPetHappiness", get_pet_happiness),
        ("C_PetInfo.GetPetLoyalty", get_pet_loyalty),
        ("GetPetExperience", get_pet_experience),
        ("CastPetAction", cast_pet_action),
        ("PetAttack", pet_attack),
        ("PetStopAttack", pet_stop_attack),
        ("PetFollow", pet_follow),
        ("PetWait", pet_wait),
        ("TogglePetAutocast", toggle_pet_autocast),
    ]
}
