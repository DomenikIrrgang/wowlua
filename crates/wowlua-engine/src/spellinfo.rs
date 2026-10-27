//! The spells' and specializations' remaining answers: `C_Spell`,
//! `C_SpellBook`, `C_ActionBar`, `C_ClassTalents`, `C_Traits`,
//! `C_SpecializationInfo` and the spell, spec, talent, totem, macro and
//! companion globals no other module answers. They answer from the
//! engine's world where it holds the answer: the recorded spellbook
//! (`stats`), the actions on the bars (`actions`), the spells' data and
//! cooldowns (`gamedata`, `casting`), the macros (`macros`), the traits
//! model (`traits`). Everything else is the "nothing here" state of a WoW
//! Forever character (one specialization per class, no PvP talents, hero
//! talents, loadouts, totems, companions, charges or loss of control).
//!
//! Unless a comment says recorded or probed, an answer is unverified: it
//! follows Blizzard's documentation (its return shapes) and, where noted,
//! warcraft.wiki.gg.

use std::rc::Rc;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::actions::Action;
use crate::stats::{Answer, Entry};
use crate::units::{self, UnitId};
use crate::{engine, widgets};

// ---------------------------------------------------------------- helpers

/// What the engine answers for the client function `name` called with
/// `args` (its behaviour; none: nothing), secrets read as the client's C
/// code reads them (as `conditionals` asks).
fn ask(vm: &mut Vm, name: &'static str, args: &[Value]) -> Vec<Value> {
    let Some(f) = engine(vm).behaviour(name) else {
        return vec![];
    };
    let full = vm.str(name);
    let call = NativeCall {
        args: args.iter().copied().collect(),
        taints: args.iter().map(|_| Default::default()).collect(),
        origins: args.iter().map(|_| Default::default()).collect(),
        upvals: Rc::from([full]),
        upval_taints: Rc::from([Default::default()]),
    };
    match f(vm, call) {
        Ok(values) => values.into_iter().map(|v| vm.hidden(v)).collect(),
        Err(_) => vec![],
    }
}

/// The first answer's truth.
fn yes(vm: &mut Vm, name: &'static str, args: &[Value]) -> bool {
    ask(vm, name, args).first().is_some_and(|v| v.truthy())
}

fn num(v: Value) -> Option<f64> {
    match v {
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

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn table(vm: &mut Vm, fields: &[(&str, Value)]) -> Value {
    let t = vm.new_table();
    for (k, v) in fields {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, *v);
    }
    Value::Table(t)
}

fn list(vm: &mut Vm, values: &[f64]) -> Value {
    let t = vm.new_table();
    for (i, v) in values.iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(i as f64 + 1.0), Value::Num(*v));
    }
    Value::Table(t)
}

/// A new `DurationObject` from `start` lasting `duration` (the clock's
/// seconds), not secret: the properties `curves` keeps a span in.
fn duration(vm: &mut Vm, start: f64, length: f64, rate: f64) -> Value {
    let d = widgets::create(vm, "DurationObject", None, None);
    if let Some(id) = widgets::widget_of(vm, d) {
        let props = engine(vm).world.widgets[id.0].props;
        for (k, v) in [
            ("start", Value::Num(start)),
            ("end", Value::Num(start + length.max(0.0))),
            ("modRate", Value::Num(rate)),
            ("secret", Value::Bool(false)),
        ] {
            let k = vm.str(k);
            let _ = vm.heap.table_mut(props).set(k, v);
        }
    }
    d
}

/// A zero duration (nothing running).
fn zero_duration(vm: &mut Vm) -> Value {
    duration(vm, 0.0, 0.0, 1.0)
}

/// A value the engine keeps by name (`widgets::keep_as`), if any.
fn named(vm: &mut Vm, name: &str) -> Value {
    let registry = vm.registry;
    let key = vm.heap.intern(b"wowlua.engine.named");
    let Value::Table(t) = vm.heap.table(registry).get_str(key) else {
        return Value::Nil;
    };
    let k = vm.str(name);
    vm.heap.table(t).get(k)
}

// ------------------------------------------------------------ the spellbook

/// A recorded spellbook item's fields.
type Fields = Vec<(String, Answer)>;

/// The player's recorded spellbook items, by slot in order.
fn book(vm: &Vm) -> Vec<(i64, Fields)> {
    let Some(Entry::ByIndex(items)) = engine(vm).world.character.stats.get("spellBookItem") else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|(slot, answer)| match answer.first() {
            Some(Answer::Table(fields)) => Some((
                *slot,
                fields
                    .iter()
                    .filter_map(|(k, v)| match k {
                        Answer::Str(k) => Some((k.clone(), v.clone())),
                        _ => None,
                    })
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

fn get_num(fields: &Fields, key: &str) -> Option<f64> {
    fields.iter().find_map(|(k, v)| match v {
        Answer::Num(n) if k == key => Some(*n),
        _ => None,
    })
}

fn get_str<'a>(fields: &'a Fields, key: &str) -> Option<&'a str> {
    fields.iter().find_map(|(k, v)| match v {
        Answer::Str(s) if k == key => Some(s.as_str()),
        _ => None,
    })
}

fn get_bool(fields: &Fields, key: &str) -> Option<bool> {
    fields.iter().find_map(|(k, v)| match v {
        Answer::Bool(b) if k == key => Some(*b),
        _ => None,
    })
}

/// The spellbook item `(slot, bank)` asks about: the player's bank (0)
/// only (the pet's items are not recorded).
///
/// The slot is truncated (the full probe, 1.60.1: 1.5 asked about slot 1).
fn book_item(vm: &Vm, c: &NativeCall) -> Option<Fields> {
    let slot = num(vm.hidden(c.arg(1)))?.trunc();
    if matches!(vm.hidden(c.arg(2)), Value::Num(b) if b == 1.0) {
        return None;
    }
    book(vm)
        .into_iter()
        .find(|(s, _)| *s as f64 == slot)
        .map(|(_, f)| f)
}

/// The spell of the spellbook item asked about.
fn book_spell(vm: &Vm, c: &NativeCall) -> Option<f64> {
    book_item(vm, c).and_then(|f| get_num(&f, "spellID"))
}

/// The spell a name means: the spellbook's (`"Name"`: its last rank, the
/// highest, as the client casts it; `"Name(Rank 2)"`: that rank), else a
/// defined spell's.
fn spell_named(vm: &mut Vm, text: &str) -> Option<f64> {
    let text = text.trim();
    let (name, rank) = match text.find('(') {
        Some(i) if text.ends_with(')') => {
            (text[..i].trim(), Some(text[i + 1..text.len() - 1].trim()))
        }
        _ => (text, None),
    };
    let mut found = None;
    for (_, f) in book(vm) {
        if get_str(&f, "name") != Some(name) {
            continue;
        }
        if let Some(r) = rank
            && !r.is_empty()
            && get_str(&f, "subName") != Some(r)
        {
            continue;
        }
        if let Some(id) = get_num(&f, "spellID") {
            found = Some(id);
        }
    }
    if found.is_some() {
        return found;
    }
    let mut defined: Vec<(i64, TableRef)> = engine(vm)
        .world
        .data
        .spells
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    defined.sort_by_key(|(k, _)| *k);
    for (id, t) in defined {
        if let Value::Str(s) = field(vm, t, "name")
            && vm.str_bytes(s) == name.as_bytes()
        {
            return Some(id as f64);
        }
    }
    None
}

/// The spell a `SpellIdentifier` names: an ID, or a name.
pub(crate) fn spell_of(vm: &mut Vm, v: Value) -> Option<f64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n),
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            match name.trim().parse::<f64>() {
                Ok(n) => Some(n),
                Err(_) => spell_named(vm, &name),
            }
        }
        _ => None,
    }
}

/// Whether the client has the spell: in the spellbook, defined or probed,
/// or as `C_Spell.DoesSpellExist` answers.
fn exists(vm: &mut Vm, id: f64) -> bool {
    crate::stats::spellbook_slot_of(vm, id).is_some()
        || crate::gamedata::spell_data(vm, id as i64).is_some()
        || yes(vm, "C_Spell.DoesSpellExist", &[Value::Num(id)])
}

/// Whether the player knows the spell: in the spellbook, or known as
/// `C_SpellBook.IsSpellKnown` and `IsPlayerSpell` answer.
fn known(vm: &mut Vm, id: f64) -> bool {
    crate::stats::spellbook_slot_of(vm, id).is_some()
        || crate::trainer::learned(vm, id)
        || yes(vm, "C_SpellBook.IsSpellKnown", &[Value::Num(id)])
        || yes(vm, "IsPlayerSpell", &[Value::Num(id)])
}

/// `IsPlayerSpell(spellID)`: true for a spell learned from a trainer in
/// the test (as the client knows it once taught), else as recorded.
fn is_player_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(id) = num(vm.hidden(c.arg(1)))
        && crate::trainer::learned(vm, id)
    {
        return Ok(vec![Value::Bool(true)]);
    }
    crate::recorded::recorded(vm, c)
}

/// Off the global cooldown: Auto Attack, a trait's, a spell on the next
/// swing or the data's `offGCD` (as the action bars have it).
fn off_gcd(vm: &mut Vm, id: f64) -> bool {
    id == 6603.0
        || crate::actions::has_trait(vm, Value::Num(id), "offGlobalCooldown")
        || crate::casting::spell_field(vm, id, "nextSwing").truthy()
        || crate::casting::spell_field(vm, id, "offGCD").truthy()
}

/// A spell's cooldown running now: start, length, rate. Its own (begun by
/// a use in the engine, its data's `cooldown` seconds; running since
/// before the snapshot; a defined spell's `cooldown` table), else the
/// global cooldown for a known spell on it (unless `ignore_gcd`), as
/// `C_ActionBar.GetActionCooldown` has them.
fn spell_cooldown(vm: &mut Vm, id: f64, ignore_gcd: bool) -> Option<(f64, f64, f64)> {
    let now = crate::clock::now(vm);
    let data = crate::casting::spell_field(vm, id, "cooldown");
    if let Some(at) = engine(vm).units.casting.ready_at.get(&(id as i64)).copied()
        && at > now
        && let Value::Num(cd) = data
    {
        return Some((at - cd, cd, 1.0));
    }
    if let Some((start, length)) = crate::stats::recorded_cooldown(vm, id)
        && start + length > now
    {
        return Some((start, length, 1.0));
    }
    if let Value::Table(cd) = data {
        let start = num(field(vm, cd, "startTime")).unwrap_or(0.0);
        let length = num(field(vm, cd, "duration")).unwrap_or(0.0);
        let rate = num(field(vm, cd, "modRate")).unwrap_or(1.0);
        if length > 0.0 && start + length > now {
            return Some((start, length, rate));
        }
    }
    if !ignore_gcd
        && let Some((start, length)) = engine(vm).world.global_cooldown
        && start + length > now
        && !off_gcd(vm, id)
        && known(vm, id)
    {
        return Some((start, length, 1.0));
    }
    None
}

/// A spell's range (its data's `minRange`, `maxRange`; probed spells'
/// recorded info).
fn range(vm: &mut Vm, id: f64) -> (f64, f64) {
    let Some(t) = crate::gamedata::spell_data(vm, id as i64) else {
        return (0.0, 0.0);
    };
    (
        num(field(vm, t, "minRange")).unwrap_or(0.0),
        num(field(vm, t, "maxRange")).unwrap_or(0.0),
    )
}

/// The unit argument `n` (none: the target).
fn unit_at(vm: &Vm, c: &NativeCall, n: usize) -> Option<UnitId> {
    if vm.hidden(c.arg(n)).is_nil() {
        engine(vm).units.target
    } else {
        units::unit_arg(vm, c, n)
    }
}

/// Whether the unit is in the spell's range: nil for a spell without one,
/// no unit, or a unit at no known distance (the client's nil for "can't
/// tell"; unverified).
fn in_range(vm: &mut Vm, id: f64, unit: Option<UnitId>) -> Value {
    let (min, max) = range(vm, id);
    let Some(unit) = unit else {
        return Value::Nil;
    };
    if max <= 0.0 {
        return Value::Nil;
    }
    let d = if unit == units::PLAYER {
        Some(0.0)
    } else {
        engine(vm).units.list[unit.0].distance
    };
    match d {
        Some(d) => Value::Bool(d >= min && d <= max),
        None => Value::Nil,
    }
}

/// A spell's usability now, as an action holding it would be (the bars'
/// rules: dead, on a flight, a weapon's, power, combat); a spell the
/// player does not know is not usable.
fn usable(vm: &mut Vm, id: f64) -> (bool, bool) {
    if !known(vm, id) {
        return (false, false);
    }
    let a = Action {
        kind: "spell".into(),
        id: Value::Num(id),
        usable: (true, false),
        ..Default::default()
    };
    crate::actions::usability(vm, &a)
}

/// A loss of control nothing caused (`SpellLossOfControlInfo`).
fn no_loss_of_control(vm: &mut Vm) -> Value {
    table(
        vm,
        &[
            ("startTime", Value::Num(0.0)),
            ("duration", Value::Num(0.0)),
            ("modRate", Value::Num(1.0)),
            ("isActive", Value::Bool(false)),
            ("shouldReplaceNormalCooldown", Value::Bool(false)),
        ],
    )
}

// ------------------------------------------------- answers of no state

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn nil(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn zero_zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0)])
}

fn empty_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn empty_string(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

// ------------------------------------------------------------ C_Spell

/// `C_Spell.GetSpellCooldownDuration(spell [, ignoreGCD])`: its cooldown
/// running (`spell_cooldown`) as a duration object, a zero one when none,
/// nothing for a spell the client lacks.
fn get_spell_cooldown_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = spell_of(vm, c.arg(1)).filter(|id| exists(vm, *id)) else {
        return Ok(vec![]);
    };
    let ignore = vm.hidden(c.arg(2)).truthy();
    let d = match spell_cooldown(vm, id, ignore) {
        Some((start, length, rate)) => duration(vm, start, length, rate),
        None => zero_duration(vm),
    };
    Ok(vec![d])
}

/// `C_Spell.GetSpellLossOfControlCooldownDuration(spell)`: zero (no loss
/// of control on the classic spells), nothing for a spell the client
/// lacks.
fn spell_zero_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    match spell_of(vm, c.arg(1)) {
        Some(id) if exists(vm, id) => Ok(vec![zero_duration(vm)]),
        _ => Ok(vec![]),
    }
}

/// `C_Spell.GetSpellChargeDuration(spell)`: a zero duration for a spell
/// with charges (`C_Spell.GetSpellCharges` answers some), nothing for
/// others (the full probe, 1.60.1: Attack and Cannibalize nothing).
fn spell_charge_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = spell_of(vm, c.arg(1)).filter(|id| exists(vm, *id)) else {
        return Ok(vec![]);
    };
    let charges = ask(vm, "C_Spell.GetSpellCharges", &[Value::Num(id)]);
    Ok(match charges.first() {
        Some(Value::Table(_)) => vec![zero_duration(vm)],
        _ => vec![],
    })
}

/// `C_Spell.GetVisibilityInfo(spell, context)`: no custom visibility, not
/// always shown, not for the spec (the full probe, 1.60.1: every spell and
/// context alike).
fn get_visibility_info(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false); 3])
}

/// `C_Spell.GetSchoolString(schoolMask)`: the school's name, the global
/// strings' (`STRING_SCHOOL_*`) for a school and the combinations they
/// name, "Unknown" for none (the full probe, 1.60.1: 0 "Unknown", 1
/// "Physical"; recorded "Fire" for 4, "Frost" for 16; unverified: the
/// combinations, and "Unknown" for one no string names). The game data's
/// behaviour (`gamedata.rs`), before the two recorded answers.
pub(crate) fn get_school_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(mask) = vm.hidden(c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let name = match mask as i64 {
        0 => "Unknown",
        1 => "Physical",
        2 => "Holy",
        3 => "Holystrike",
        4 => "Fire",
        5 => "Flamestrike",
        6 => "Holyfire",
        8 => "Nature",
        9 => "Stormstrike",
        10 => "Holystorm",
        12 => "Firestorm",
        16 => "Frost",
        17 => "Froststrike",
        18 => "Holyfrost",
        20 => "Frostfire",
        24 => "Froststorm",
        28 => "Elemental",
        32 => "Shadow",
        33 => "Shadowstrike",
        34 => "Twilight",
        36 => "Shadowflame",
        40 => "Plague",
        48 => "Shadowfrost",
        64 => "Arcane",
        65 => "Spellstrike",
        66 => "Divine",
        68 => "Spellfire",
        72 => "Spellstorm",
        80 => "Spellfrost",
        96 => "Spellshadow",
        124 => "Chromatic",
        126 => "Magic",
        127 => "Chaos",
        // A mask no string names: "Unknown" as for none (the full probe:
        // `GetSchoolString(-1)`, every bit set).
        _ => "Unknown",
    };
    Ok(vec![vm.str(name)])
}

/// `C_Spell.IsSpellUsable(spell)`: usable, short of power (`usable`).
fn is_spell_usable(vm: &mut Vm, c: NativeCall) -> Ret {
    let (ok, power) = match spell_of(vm, c.arg(1)) {
        Some(id) => usable(vm, id),
        None => (false, false),
    };
    Ok(vec![Value::Bool(ok), Value::Bool(power)])
}

/// `C_Spell.IsSpellInRange(spell [, unit])`: the target by default.
fn is_spell_in_range(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = spell_of(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let unit = unit_at(vm, &c, 2);
    Ok(vec![in_range(vm, id, unit)])
}

/// `C_Spell.SpellHasRange(spell)`: a maximum range above 0.
fn spell_has_range(vm: &mut Vm, c: NativeCall) -> Ret {
    let has = match spell_of(vm, c.arg(1)) {
        Some(id) => range(vm, id).1 > 0.0,
        None => false,
    };
    Ok(vec![Value::Bool(has)])
}

/// `GetSpellBaseCooldown(spell)`: its cooldown and global cooldown in ms,
/// as the trainers' probes recorded them (`0, 1500` for most); else from
/// its data (`cooldown` seconds; off the global cooldown 0, else 1500);
/// nothing for a spell the client lacks.
fn get_spell_base_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = spell_of(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    if let Some(answer) = crate::recorded::probed(&format!("GetSpellBaseCooldown({})", id as i64)) {
        return Ok(crate::recorded::parse_answer(vm, answer));
    }
    if !exists(vm, id) {
        return Ok(vec![]);
    }
    let cd = match crate::casting::spell_field(vm, id, "cooldown") {
        Value::Num(s) => s * 1000.0,
        _ => 0.0,
    };
    let gcd = if off_gcd(vm, id) { 0.0 } else { 1500.0 };
    Ok(vec![Value::Num(cd), Value::Num(gcd)])
}

/// `C_Spell.GetSpellDisplayCount(spell, …)`: no count (no spell consumes
/// a counted reagent here).
fn get_spell_display_count(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

/// `C_Spell.IsAutoRepeatSpell(spell)`: Auto Shot (75) and the wand's Shoot
/// (5019), the classic spells that repeat.
fn is_auto_repeat_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = spell_of(vm, c.arg(1));
    Ok(vec![Value::Bool(
        id.is_some_and(|i| i == 75.0 || i == 5019.0),
    )])
}

/// `C_Spell.GetSpellAutoCast(spell)`: not allowed, not on (no pet spell
/// auto-casts by spell here), nothing for a spell the client lacks.
fn get_spell_auto_cast(vm: &mut Vm, c: NativeCall) -> Ret {
    match spell_of(vm, c.arg(1)) {
        Some(id) if exists(vm, id) => Ok(vec![Value::Bool(false), Value::Bool(false)]),
        _ => Ok(vec![]),
    }
}

/// `C_Spell.GetSpellLossOfControlCooldownInfo(spell)`: none active.
fn get_spell_loss_of_control_info(vm: &mut Vm, c: NativeCall) -> Ret {
    match spell_of(vm, c.arg(1)) {
        Some(id) if exists(vm, id) => Ok(vec![no_loss_of_control(vm)]),
        _ => Ok(vec![]),
    }
}

/// `C_Spell.GetItemCooldown(itemID)`: nothing for an item without a use
/// cooldown running (the full probe, 1.60.1: the Hearthstone, 6948, and
/// items the character lacks nothing); else its cooldown as
/// `C_Container.GetItemCooldown` has it, as a `SpellCooldownInfo`
/// (unverified).
fn get_item_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    let r = ask(vm, "C_Container.GetItemCooldown", &[c.arg(1)]);
    let start = r.first().copied().and_then(num).unwrap_or(0.0);
    let length = r.get(1).copied().and_then(num).unwrap_or(0.0);
    if length <= 0.0 {
        return Ok(vec![]);
    }
    let now = crate::clock::now(vm);
    let t = table(
        vm,
        &[
            ("startTime", Value::Num(start)),
            ("duration", Value::Num(length)),
            ("isEnabled", Value::Bool(true)),
            (
                "isActive",
                Value::Bool(length > 0.0 && start + length > now),
            ),
            ("modRate", Value::Num(1.0)),
        ],
    );
    Ok(vec![t])
}

/// `C_Spell.GetSpellDescriptionForItemLocation(spell, location)`: the
/// spell's description.
fn get_spell_description_for_item_location(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(ask(vm, "C_Spell.GetSpellDescription", &[c.arg(1)]))
}

/// `C_Spell.GetAuraStatChanges(spell)`: no health, no power changes.
fn get_aura_stat_changes(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Table(vm.new_table())])
}

/// `GetSpellQueueWindow()`, `C_Spell.GetSpellQueueWindow()`: the
/// `SpellQueueWindow` CVar in ms (the client's default 400, recorded in
/// its CVars).
fn get_spell_queue_window(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ms = crate::framescript::cvar_get(vm, "SpellQueueWindow")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(400.0);
    Ok(vec![Value::Num(ms)])
}

// --------------------------------------------------------- C_SpellBook

/// `C_SpellBook.IsSpellKnownOrInSpellBook(spellID [, bank,
/// includeOverrides])`: the player's (`known`); the pet's as
/// `C_SpellBook.IsSpellKnown` answers for its bank.
fn is_spell_known_or_in_spellbook(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = num(vm.hidden(c.arg(1))) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let pet = matches!(vm.hidden(c.arg(2)), Value::Num(b) if b == 1.0);
    let k = if pet {
        yes(
            vm,
            "C_SpellBook.IsSpellKnown",
            &[Value::Num(id), Value::Num(1.0)],
        )
    } else {
        known(vm, id)
    };
    Ok(vec![Value::Bool(k)])
}

/// `C_SpellBook.ContainsAnyDisenchantSpell()`: Disenchant (13262) in the
/// spellbook.
fn contains_any_disenchant_spell(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        crate::stats::spellbook_slot_of(vm, 13262.0).is_some(),
    )])
}

/// `C_SpellBook.GetSpellBookItemDescription(slot, bank)`: its spell's.
fn get_spellbook_item_description(vm: &mut Vm, c: NativeCall) -> Ret {
    match book_spell(vm, &c) {
        Some(id) => Ok(ask(vm, "C_Spell.GetSpellDescription", &[Value::Num(id)])),
        None => Ok(vec![]),
    }
}

/// `C_SpellBook.GetSpellBookItemCharges(slot, bank)`: its spell's.
fn get_spellbook_item_charges(vm: &mut Vm, c: NativeCall) -> Ret {
    match book_spell(vm, &c) {
        Some(id) => Ok(ask(vm, "C_Spell.GetSpellCharges", &[Value::Num(id)])),
        None => Ok(vec![]),
    }
}

/// `C_SpellBook.GetSpellBookItemPowerCost(slot, bank)`: its spell's.
fn get_spellbook_item_power_cost(vm: &mut Vm, c: NativeCall) -> Ret {
    match book_spell(vm, &c) {
        Some(id) => Ok(ask(vm, "C_Spell.GetSpellPowerCost", &[Value::Num(id)])),
        None => Ok(vec![]),
    }
}

/// `C_SpellBook.GetSpellBookItemLossOfControlCooldownInfo(slot, bank)`:
/// none active.
fn get_spellbook_item_loss_of_control_info(vm: &mut Vm, c: NativeCall) -> Ret {
    match book_spell(vm, &c) {
        Some(_) => Ok(vec![no_loss_of_control(vm)]),
        None => Ok(vec![]),
    }
}

/// `C_SpellBook.GetSpellBookItemCastCount(slot, bank)`: 0.
fn get_spellbook_item_cast_count(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `C_SpellBook.IsAutoAttackSpellBookItem(slot, bank)`: its spell as
/// `C_Spell.IsAutoAttackSpell` answers (recorded: Attack, 6603).
fn is_auto_attack_spellbook_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = match book_spell(vm, &c) {
        Some(id) => yes(vm, "C_Spell.IsAutoAttackSpell", &[Value::Num(id)]),
        None => false,
    };
    Ok(vec![Value::Bool(v)])
}

/// `C_SpellBook.IsRangedAutoAttackSpellBookItem(slot, bank)`: its spell
/// as `C_Spell.IsRangedAutoAttackSpell` answers.
fn is_ranged_auto_attack_spellbook_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = match book_spell(vm, &c) {
        Some(id) => yes(vm, "C_Spell.IsRangedAutoAttackSpell", &[Value::Num(id)]),
        None => false,
    };
    Ok(vec![Value::Bool(v)])
}

/// `C_SpellBook.IsSpellBookItemInRange(slot, bank [, unit])`.
fn is_spellbook_item_in_range(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = book_spell(vm, &c) else {
        return Ok(vec![Value::Nil]);
    };
    let unit = unit_at(vm, &c, 3);
    Ok(vec![in_range(vm, id, unit)])
}

/// `C_SpellBook.IsSpellBookItemUsable(slot, bank)`.
fn is_spellbook_item_usable(vm: &mut Vm, c: NativeCall) -> Ret {
    let (ok, power) = match book_spell(vm, &c) {
        Some(id) => usable(vm, id),
        None => (false, false),
    };
    Ok(vec![Value::Bool(ok), Value::Bool(power)])
}

/// `C_SpellBook.SpellBookItemHasRange(slot, bank)`.
fn spellbook_item_has_range(vm: &mut Vm, c: NativeCall) -> Ret {
    let has = match book_spell(vm, &c) {
        Some(id) => range(vm, id).1 > 0.0,
        None => false,
    };
    Ok(vec![Value::Bool(has)])
}

/// `C_SpellBook.IsSpellBookItemOffSpec(slot, bank)`: its recorded
/// `isOffSpec`.
fn is_spellbook_item_off_spec(vm: &mut Vm, c: NativeCall) -> Ret {
    let off = book_item(vm, &c).and_then(|f| get_bool(&f, "isOffSpec"));
    Ok(vec![Value::Bool(off.unwrap_or(false))])
}

/// The recorded spellbook skill lines: index, item offset, item count,
/// skill line ID.
fn skill_lines(vm: &Vm) -> Vec<(i64, f64, f64, Option<f64>)> {
    let Some(Entry::ByIndex(lines)) = engine(vm).world.character.stats.get("spellBookSkillLine")
    else {
        return Vec::new();
    };
    lines
        .iter()
        .filter_map(|(i, answer)| match answer.first() {
            Some(Answer::Table(fields)) => {
                let fields: Fields = fields
                    .iter()
                    .filter_map(|(k, v)| match k {
                        Answer::Str(k) => Some((k.clone(), v.clone())),
                        _ => None,
                    })
                    .collect();
                Some((
                    *i,
                    get_num(&fields, "itemIndexOffset").unwrap_or(0.0),
                    get_num(&fields, "numSpellBookItems").unwrap_or(0.0),
                    get_num(&fields, "skillLineID"),
                ))
            }
            _ => None,
        })
        .collect()
}

/// `C_SpellBook.GetSpellBookItemSkillLineIndex(slot, bank)`: its recorded
/// `skillLineIndex`, nil for an item without one (the full probe, 1.60.1:
/// the General line's Attack nil, as its recorded item lacks the field).
fn get_spellbook_item_skill_line_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let line = book_item(vm, &c).and_then(|f| get_num(&f, "skillLineIndex"));
    Ok(vec![line.map_or(Value::Nil, Value::Num)])
}

/// `C_SpellBook.GetSkillLineIndexByID(skillLineID)`: the recorded skill
/// line with that `skillLineID`, else nil.
fn get_skill_line_index_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = num(vm.hidden(c.arg(1))) else {
        return Ok(vec![Value::Nil]);
    };
    let line = skill_lines(vm)
        .into_iter()
        .find(|(.., skill)| *skill == Some(id))
        .map(|(i, ..)| Value::Num(i as f64));
    Ok(vec![line.unwrap_or(Value::Nil)])
}

/// `C_SpellBook.GetSpellBookItemCooldownDuration(slot, bank [,
/// ignoreGCD])`: its spell's cooldown as a duration object.
fn get_spellbook_item_cooldown_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = book_spell(vm, &c) else {
        return Ok(vec![]);
    };
    let ignore = vm.hidden(c.arg(3)).truthy();
    let d = match spell_cooldown(vm, id, ignore) {
        Some((start, length, rate)) => duration(vm, start, length, rate),
        None => zero_duration(vm),
    };
    Ok(vec![d])
}

/// `C_SpellBook.GetSpellBookItemLossOfControlCooldownDuration(slot,
/// bank)`: zero for an item.
fn spellbook_item_zero_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    match book_item(vm, &c) {
        Some(_) => Ok(vec![zero_duration(vm)]),
        None => Ok(vec![]),
    }
}

/// `C_SpellBook.GetSpellBookItemChargeDuration(slot, bank)`: its spell's
/// (`C_Spell.GetSpellChargeDuration`; the full probe, 1.60.1: Attack's
/// nothing).
fn spellbook_item_charge_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    match book_spell(vm, &c) {
        Some(id) => Ok(ask(vm, "C_Spell.GetSpellChargeDuration", &[Value::Num(id)])),
        None => Ok(vec![]),
    }
}

/// `C_SpellBook.GetCurrentLevelSpells(level)`: the spells of the
/// spellbook's General line (its items without a skill line) learned at
/// that level, in the book's order (the full probe, 1.60.1: level 0
/// Attack, the racials and the passives, level 1 Block, Dodge, Parry and
/// Sacred Duty; the class lines' Holy Light and Seal of Righteousness at
/// level 1 not; unverified: the rule beyond this character).
fn get_current_level_spells(vm: &mut Vm, c: NativeCall) -> Ret {
    let level = num(vm.hidden(c.arg(1)));
    let levels = match engine(vm).world.character.stats.get("spellBookItemLevel") {
        Some(Entry::ByIndex(m)) => m.clone(),
        _ => Default::default(),
    };
    let spells: Vec<f64> = book(vm)
        .into_iter()
        .filter(|(_, f)| get_num(f, "skillLineIndex").is_none())
        .filter(|(s, _)| {
            matches!(levels.get(s).and_then(|a| a.first()), Some(Answer::Num(l)) if Some(*l) == level)
        })
        .filter_map(|(_, f)| get_num(&f, "spellID"))
        .collect();
    Ok(vec![list(vm, &spells)])
}

/// `GetFlyoutID(slot)`: the flyout a spellbook slot holds (its item's
/// `actionID` when its type is a flyout, 4), else nil.
fn get_flyout_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let slot = num(vm.hidden(c.arg(1)));
    let id = book(vm)
        .into_iter()
        .find(|(s, _)| Some(*s as f64) == slot)
        .filter(|(_, f)| get_num(f, "itemType") == Some(4.0))
        .and_then(|(_, f)| get_num(&f, "actionID"));
    Ok(vec![id.map_or(Value::Nil, Value::Num)])
}

// --------------------------------------------------------- C_ActionBar

/// The slot asked about (`None` for 0 and below, as the bars answer
/// nothing for them) and the action in it.
fn action_at(vm: &Vm, c: &NativeCall) -> Option<Option<Action>> {
    let slot =
        num(vm.hidden(c.arg(1))).filter(|n| (1.0..=crate::actions::LAST_SLOT).contains(n))? as i64;
    Some(engine(vm).world.character.actions.get(&slot).cloned())
}

/// The spell an action holds.
fn action_spell(a: &Option<Action>) -> Option<f64> {
    match a {
        Some(a) if a.kind == "spell" => num(a.id),
        _ => None,
    }
}

/// `C_ActionBar.GetActionCooldownDuration(slot [, ignoreGCD])`: the
/// slot's cooldown as `C_ActionBar.GetActionCooldown` has it, as a
/// duration object (the global cooldown left out when ignored).
fn get_action_cooldown_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    if action_at(vm, &c).is_none() {
        return Ok(vec![]);
    }
    let ignore = vm.hidden(c.arg(2)).truthy();
    let info = ask(vm, "C_ActionBar.GetActionCooldown", &[c.arg(1)]);
    let Some(Value::Table(t)) = info.first().copied() else {
        return Ok(vec![zero_duration(vm)]);
    };
    let on_gcd = field(vm, t, "isOnGCD").truthy();
    let start = num(field(vm, t, "startTime")).unwrap_or(0.0);
    let length = num(field(vm, t, "duration")).unwrap_or(0.0);
    let rate = num(field(vm, t, "modRate")).unwrap_or(1.0);
    let d = if ignore && on_gcd {
        zero_duration(vm)
    } else {
        duration(vm, start, length, rate)
    };
    Ok(vec![d])
}

/// `C_ActionBar.GetActionLossOfControlCooldownDuration(slot)`: zero (no
/// loss of control runs).
fn action_zero_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    match action_at(vm, &c) {
        Some(_) => Ok(vec![zero_duration(vm)]),
        None => Ok(vec![]),
    }
}

/// `C_ActionBar.GetActionChargeDuration(slot)`: nil, no action having
/// charges (the full probe, 1.60.1: a spell's slot; as `GetActionCharges`'
/// zero charges answer).
fn action_charge_duration(vm: &mut Vm, c: NativeCall) -> Ret {
    match action_at(vm, &c) {
        Some(_) => Ok(vec![Value::Nil]),
        None => Ok(vec![]),
    }
}

/// `C_ActionBar.GetSpell(slot)`: the spell in the slot, else 0.
fn get_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match action_at(vm, &c) {
        Some(a) => vec![Value::Num(action_spell(&a).unwrap_or(0.0))],
        None => vec![],
    })
}

/// `C_ActionBar.GetActionAutocast(slot)`: not allowed, not on (no pet
/// actions on the bars, as `IsAutoCastPetAction` answers).
fn get_action_autocast(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match action_at(vm, &c) {
        Some(_) => vec![Value::Bool(false), Value::Bool(false)],
        None => vec![],
    })
}

/// `C_ActionBar.GetActionUseCount(slot)`: 0.
fn get_action_use_count(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match action_at(vm, &c) {
        Some(_) => vec![Value::Num(0.0)],
        None => vec![],
    })
}

/// `C_ActionBar.IsOnBarOrSpecialBar(spell [, set])`,
/// `C_ActionBar.HasSpellActionButtons(spell [, set])`: a slot of the set
/// holds it (`C_ActionBar.FindSpellActionButtons`).
///
/// A string naming no spell searches as `FindSpellActionButtons` does (the
/// full probe, 1.60.1: "x" found the macro holding spell 0).
fn has_spell_action_buttons(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = match vm.hidden(c.arg(1)) {
        Value::Str(_) => spell_of(vm, c.arg(1)).unwrap_or(0.0),
        _ => match spell_of(vm, c.arg(1)) {
            Some(id) => id,
            None => return Ok(vec![Value::Bool(false)]),
        },
    };
    let set = match vm.hidden(c.arg(2)) {
        Value::Nil => Value::Num(3.0),
        v => v,
    };
    let found = match ask(
        vm,
        "C_ActionBar.FindSpellActionButtons",
        &[Value::Num(id), set],
    )
    .first()
    {
        Some(Value::Table(t)) => vm.heap.table(*t).border() > 0,
        _ => false,
    };
    Ok(vec![Value::Bool(found)])
}

/// The slots holding a flyout, in order.
fn flyout_slots(vm: &Vm, flyout: f64) -> Vec<f64> {
    engine(vm)
        .world
        .character
        .actions
        .iter()
        .filter(|(_, a)| a.kind == "flyout" && num(a.id) == Some(flyout))
        .map(|(s, _)| *s as f64)
        .collect()
}

/// `C_ActionBar.FindFlyoutActionButtons(flyoutID [, set])`: the slots
/// holding it, or nothing (as `FindSpellActionButtons` answers).
fn find_flyout_action_buttons(vm: &mut Vm, c: NativeCall) -> Ret {
    let slots = num(vm.hidden(c.arg(1)))
        .map(|f| flyout_slots(vm, f))
        .unwrap_or_default();
    if slots.is_empty() {
        return Ok(vec![]);
    }
    Ok(vec![list(vm, &slots)])
}

/// `C_ActionBar.HasFlyoutActionButtons(flyoutID [, set])`.
fn has_flyout_action_buttons(vm: &mut Vm, c: NativeCall) -> Ret {
    let any = num(vm.hidden(c.arg(1))).is_some_and(|f| !flyout_slots(vm, f).is_empty());
    Ok(vec![Value::Bool(any)])
}

/// `C_ActionBar.IsMacroActionWithShowTooltip(slot)`: a macro whose body
/// has `#showtooltip`.
fn is_macro_action_with_show_tooltip(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(a) = action_at(vm, &c) else {
        return Ok(vec![]);
    };
    let show = match a {
        Some(a) if a.kind == "macro" => {
            macro_body(vm, a.id).is_some_and(|b| b.to_ascii_lowercase().contains("#showtooltip"))
        }
        _ => false,
    };
    Ok(vec![Value::Bool(show)])
}

/// `C_ActionBar.PutActionInSlot(slot)`: what the cursor holds placed
/// there (`PlaceAction`).
fn put_action_in_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    ask(vm, "PlaceAction", &[c.arg(1)]);
    Ok(vec![])
}

/// `C_ActionBar.HasRangeRequirements(slot)`: a spell with a range.
fn has_range_requirements(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(a) = action_at(vm, &c) else {
        return Ok(vec![]);
    };
    let has = match action_spell(&a) {
        Some(id) => range(vm, id).1 > 0.0,
        None => false,
    };
    Ok(vec![Value::Bool(has)])
}

/// `C_ActionBar.IsActionInRange(slot [, unit])`: a spell's range to the
/// unit (the target by default); nil for others.
fn is_action_in_range(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(a) = action_at(vm, &c) else {
        return Ok(vec![]);
    };
    let Some(id) = action_spell(&a) else {
        return Ok(vec![Value::Nil]);
    };
    let unit = unit_at(vm, &c, 2);
    Ok(vec![in_range(vm, id, unit)])
}

/// An item action's data field.
fn action_item_field(vm: &mut Vm, a: &Option<Action>, key: &str) -> Value {
    match a {
        Some(a) if a.kind == "item" => {
            match num(a.id).and_then(|id| crate::gamedata::item_data(vm, id as i64)) {
                Some(t) => field(vm, t, key),
                None => Value::Nil,
            }
        }
        _ => Value::Nil,
    }
}

/// `C_ActionBar.IsConsumableAction(slot)`: an item of the consumables
/// (class 0).
fn is_consumable_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(a) = action_at(vm, &c) else {
        return Ok(vec![]);
    };
    let class = action_item_field(vm, &a, "classID");
    Ok(vec![Value::Bool(num(class) == Some(0.0))])
}

/// `C_ActionBar.IsStackableAction(slot)`: an item that stacks.
fn is_stackable_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(a) = action_at(vm, &c) else {
        return Ok(vec![]);
    };
    let stack = action_item_field(vm, &a, "stackCount");
    Ok(vec![Value::Bool(num(stack).is_some_and(|n| n > 1.0))])
}

// -------------------------------------------------------------- macros

/// A macro's body by index (the account's 1 to 120, the character's from
/// 121) or name.
fn macro_body(vm: &Vm, v: Value) -> Option<String> {
    let m = &engine(vm).world.character.macros;
    let base = crate::macros::CHARACTER_BASE;
    match vm.hidden(v) {
        Value::Num(n) if n >= 1.0 => {
            let i = n as usize;
            if i <= base {
                m.account.get(i - 1).map(|x| x.body.clone())
            } else {
                m.character.get(i - base - 1).map(|x| x.body.clone())
            }
        }
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            m.account
                .iter()
                .chain(m.character.iter())
                .find(|x| x.name == name)
                .map(|x| x.body.clone())
        }
        _ => None,
    }
}

/// What a macro shows: its `#showtooltip`'s options, else its first
/// `/cast` or `/use` line's, through the conditions (`conditionals`);
/// a leading `!` dropped.
fn macro_value(vm: &mut Vm, body: &str) -> Option<String> {
    let mut options = None;
    for line in body.lines().map(str::trim) {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("#showtooltip") {
            let rest = line[line.len() - rest.len()..].trim();
            if !rest.is_empty() {
                options = Some(rest.to_string());
                break;
            }
            continue;
        }
        for cmd in ["/cast ", "/use "] {
            if lower.starts_with(cmd) && options.is_none() {
                options = Some(line[cmd.len()..].trim().to_string());
            }
        }
    }
    let options = options?;
    let (value, _) = crate::conditionals::parse(vm, options.as_bytes())?;
    let value = String::from_utf8_lossy(&value).into_owned();
    let value = value.trim().trim_start_matches('!').trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// `GetMacroSpell(macro)`: the spell the macro casts now (its value named
/// a spell of the spellbook, or a defined one), else nothing
/// (warcraft.wiki.gg: the spell ID the macro is set to cast).
/// The spell a macro (by index or name) casts now, as `GetMacroSpell`
/// answers.
pub(crate) fn macro_spell(vm: &mut Vm, id: Value) -> Option<f64> {
    let body = macro_body(vm, id)?;
    let value = macro_value(vm, &body)?;
    spell_named(vm, &value)
}

fn get_macro_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(body) = macro_body(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let Some(value) = macro_value(vm, &body) else {
        return Ok(vec![]);
    };
    Ok(match spell_named(vm, &value) {
        Some(id) => vec![Value::Num(id)],
        None => vec![],
    })
}

/// `GetMacroItem(macro)`: the item the macro uses now (its value names a
/// cached item): its name and link, else nothing.
fn get_macro_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(body) = macro_body(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let Some(value) = macro_value(vm, &body) else {
        return Ok(vec![]);
    };
    if spell_named(vm, &value).is_some() {
        return Ok(vec![]);
    }
    let mut items: Vec<(i64, TableRef)> = engine(vm)
        .world
        .data
        .items
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();
    items.sort_by_key(|(k, _)| *k);
    for (_, t) in items {
        let name = field(vm, t, "name");
        if text(vm, name).as_deref() == Some(value.as_str()) && field(vm, t, "cached").truthy() {
            let link = field(vm, t, "link");
            return Ok(vec![name, link]);
        }
    }
    Ok(vec![])
}

// --------------------------------------------------------- totems

/// `GetTotemInfo(slot)`: an empty totem for every slot `GetNumTotemSlots`
/// counts, as recorded for the first four (slots 5 to 8 unrecorded:
/// Blizzard's cooldown viewer reads them all, and the API documentation's
/// results are not nilable); slot 0 (none) nothing (the full probe).
fn get_totem_info(vm: &mut Vm, c: NativeCall) -> Ret {
    match num(vm.hidden(c.arg(1))) {
        Some(s) if (1.0..=crate::globalsinfo::TOTEM_SLOTS).contains(&s) => {
            let name = vm.str("");
            Ok(vec![
                Value::Bool(false),
                name,
                Value::Num(0.0),
                Value::Num(0.0),
                Value::Nil,
                Value::Num(0.0),
                Value::Num(0.0),
            ])
        }
        _ => Ok(vec![]),
    }
}

/// `GetTotemDuration(slot)`: zero (no totem down: `GetTotemInfo` recorded
/// every slot empty).
fn get_totem_duration(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![zero_duration(vm)])
}

/// `GetTotemTimeLeft(slot)`: no totem, 0 left; slot 0 (none) nil (the full
/// probe).
fn totem_time_left(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match num(vm.hidden(c.arg(1))) {
        Some(s) if s >= 1.0 => Value::Num(0.0),
        _ => Value::Nil,
    }])
}

/// `GetTotemCannotDismiss(slot)`: false; slot 0 nil (the full probe).
fn totem_cannot_dismiss(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match num(vm.hidden(c.arg(1))) {
        Some(s) if s >= 1.0 => Value::Bool(false),
        _ => Value::Nil,
    }])
}

// ------------------------------------------------------ specializations

/// Each class's one specialization in WoW Forever (recorded:
/// `GetSpecializationInfoForClassID`, `C_SpecializationInfo.
/// GetSpecializationInfo`): spec ID, class ID, class file, name (the
/// class's), icon; every one a damage dealer.
const SPECS: &[(f64, f64, &str, &str, f64)] = &[
    (1491.0, 1.0, "WARRIOR", "Warrior", 626008.0),
    (1486.0, 2.0, "PALADIN", "Paladin", 626003.0),
    (1485.0, 3.0, "HUNTER", "Hunter", 626000.0),
    (1488.0, 4.0, "ROGUE", "Rogue", 626005.0),
    (1487.0, 5.0, "PRIEST", "Priest", 626004.0),
    (1489.0, 7.0, "SHAMAN", "Shaman", 626006.0),
    (1482.0, 8.0, "MAGE", "Mage", 626001.0),
    (1490.0, 9.0, "WARLOCK", "Warlock", 626007.0),
    (1484.0, 11.0, "DRUID", "Druid", 625999.0),
];

/// The recorded role of every specialization.
const ROLE: &str = "DAMAGER";

fn spec(vm: &Vm, c: &NativeCall) -> Option<&'static (f64, f64, &'static str, &'static str, f64)> {
    let id = num(vm.hidden(c.arg(1)))?;
    SPECS.iter().find(|s| s.0 == id)
}

/// `GetSpecializationInfoByID(specID [, sex])`: id, name, description,
/// icon, role, class file, class name (warcraft.wiki.gg's order; the
/// values as `GetSpecializationInfoForClassID` recorded them), nothing for
/// another ID.
fn get_specialization_info_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(&(id, _, file, name, icon)) = spec(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(vec![
        Value::Num(id),
        vm.str(name),
        vm.str(""),
        Value::Num(icon),
        vm.str(ROLE),
        vm.str(file),
        vm.str(name),
    ])
}

/// `GetSpecializationInfoForClassID(classID, specIndex [, sex])`: the class's
/// one specialization at index 1, as recorded; class 0 the pets' (the full
/// probe: 1493 "Default", "I'm a pet!"); numbers taken whole, the sex
/// ignored, any other index or class nothing (the full probe).
fn get_specialization_info_for_class_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let whole = |v: Value| num(v).map(f64::trunc);
    let (Some(class), Some(index)) = (whole(vm.hidden(c.arg(1))), whole(vm.hidden(c.arg(2))))
    else {
        return Ok(vec![]);
    };
    if index != 1.0 {
        return Ok(vec![]);
    }
    let (id, name, about, icon) = if class == 0.0 {
        (1493.0, "Default", "I'm a pet!", 6642732.0)
    } else {
        match SPECS.iter().find(|s| s.1 == class) {
            Some(&(id, _, _, name, icon)) => (id, name, "", icon),
            None => return Ok(vec![]),
        }
    };
    Ok(vec![
        Value::Num(id),
        vm.str(name),
        vm.str(about),
        Value::Num(icon),
        vm.str(ROLE),
        Value::Bool(false),
        Value::Bool(false),
        Value::Nil,
        Value::Nil,
    ])
}

/// `GetSpecializationInfoForSpecID(specID [, sex])`: as
/// `GetSpecializationInfoForClassID` recorded it (id, name, "", icon,
/// role, not recommended, not for boosts, no mastery spells).
fn get_specialization_info_for_spec_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(&(id, _, _, name, icon)) = spec(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(vec![
        Value::Num(id),
        vm.str(name),
        vm.str(""),
        Value::Num(icon),
        vm.str(ROLE),
        Value::Bool(false),
        Value::Bool(false),
        Value::Nil,
        Value::Nil,
    ])
}

/// `GetSpecializationNameForSpecID(specID [, sex])`.
fn get_specialization_name_for_spec_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match spec(vm, &c) {
        Some(s) => vm.str(s.3),
        None => Value::Nil,
    }])
}

/// `GetSpecializationRoleByID(specID)`.
fn get_specialization_role_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    // Another ID: nothing (the full probe).
    Ok(match spec(vm, &c) {
        Some(_) => vec![vm.str(ROLE)],
        None => vec![],
    })
}

/// `GetPrimarySpecialization()`: the player's class's one specialization
/// (the full probe: the paladin's 1486).
fn get_primary_specialization(vm: &mut Vm, _c: NativeCall) -> Ret {
    let class = engine(vm).units.list[crate::units::PLAYER.0].class.clone();
    Ok(match SPECS.iter().find(|s| s.2 == class) {
        Some(s) => vec![Value::Num(s.0)],
        None => vec![Value::Nil],
    })
}

/// `GetSpecializationRoleEnumByID(specID)`: `Enum.LFGRole.Damage`, 2
/// (recorded: `GetSpecializationRoleEnum(1)`).
fn get_specialization_role_enum_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match spec(vm, &c) {
        Some(_) => vec![Value::Num(2.0)],
        None => vec![],
    })
}

/// `GetSpecializationRole(specIndex [, isInspect, isPet])`: the role of
/// the player's specialization at that index
/// (`C_SpecializationInfo.GetSpecializationInfo`'s fifth), nil for a pet.
fn get_specialization_role(vm: &mut Vm, c: NativeCall) -> Ret {
    if vm.hidden(c.arg(3)).truthy() {
        return Ok(vec![Value::Nil]);
    }
    let info = ask(
        vm,
        "C_SpecializationInfo.GetSpecializationInfo",
        &[c.arg(1)],
    );
    let role = info.get(4).copied().unwrap_or(Value::Nil);
    Ok(vec![if matches!(role, Value::Str(_)) {
        role
    } else {
        Value::Nil
    }])
}

/// `GetSpecializationSystem()`: `Enum.SpecializationSystem.
/// ChrSpecialization`, 1 (the recorded specializations are such IDs).
fn get_specialization_system(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1.0)])
}

// ------------------------------------------------------ talents, traits

/// `C_ClassTalents.LoadConfig(configID, autoApply)`: the active config
/// needs no change (`Enum.LoadConfigResult.NoChangesNecessary`, 1);
/// another is an error (0): the engine keeps no saved loadouts.
fn load_config(vm: &mut Vm, c: NativeCall) -> Ret {
    let active = ask(vm, "C_ClassTalents.GetActiveConfigID", &[]);
    let same = matches!(
        (active.first().copied(), vm.hidden(c.arg(1))),
        (Some(Value::Num(a)), Value::Num(b)) if a == b
    );
    let result = if same { 1.0 } else { 0.0 };
    Ok(vec![
        Value::Num(result),
        Value::Nil,
        Value::Table(vm.new_table()),
    ])
}

/// `C_ClassTalents.SetStarterBuildActive(active)`: an error (no starter
/// builds, as `GetHasStarterBuild` recorded).
fn set_starter_build_active(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `C_Traits.PurchaseAllRanks(configID, nodeID [, ignoreCost])`,
/// `C_Traits.TryPurchaseAllRanks(configID, nodeID)`: ranks bought one by
/// one (`C_Traits.PurchaseRank`) while they can be; whether any was.
fn purchase_all_ranks(vm: &mut Vm, c: NativeCall) -> Ret {
    let (config, node) = (c.arg(1), c.arg(2));
    let mut any = false;
    for _ in 0..10 {
        if !yes(vm, "C_Traits.PurchaseRank", &[config, node]) {
            break;
        }
        any = true;
    }
    Ok(vec![Value::Bool(any)])
}

// ------------------------------------------------------ cooldown viewer

const LAYOUT_KEY: &str = "spellinfo.cooldownViewerLayout";

/// `C_CooldownViewer.GetLayoutData()`: what `SetLayoutData` saved, else
/// an empty string, the documented string's empty value Blizzard's code
/// read before (its serializer takes "" as no layouts saved). The client
/// answers the account's saved layout (recorded `"1|a2Fd…"`), which
/// Blizzard's code decodes with `C_EncodingUtil`; unverified that the
/// engine's decoding reads it as the client does, so it is not answered.
fn get_layout_data(vm: &mut Vm, _c: NativeCall) -> Ret {
    match named(vm, LAYOUT_KEY) {
        v @ Value::Str(_) => Ok(vec![v]),
        _ => Ok(vec![vm.str("")]),
    }
}

/// `C_CooldownViewer.SetLayoutData(data)`: saved for `GetLayoutData`.
fn set_layout_data(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(data) = text(vm, c.arg(1)) {
        let v = vm.str(&data);
        widgets::keep_as(vm, LAYOUT_KEY, v);
    }
    Ok(vec![])
}

/// `C_CooldownViewer.IsCooldownViewerAvailable()`: available, no reason
/// (the full probe, 1.60.1, a level 21 character; the newest answer wins
/// over the engine's earlier "not available"; whether a low level's is
/// refused is unverified). Blizzard's viewer stays hidden while its
/// `cooldownViewerEnabled` CVar is 0, as recorded.
fn is_cooldown_viewer_available(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true), vm.str("")])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("IsPlayerSpell", is_player_spell),
        // C_Spell (`GetSpellIDForSpellIdentifier`, `GetBaseSpell`,
        // `C_SpellBook.FindSpellOverrideByID` and `FindBaseSpellByID` are
        // the game data's, `gamedata.rs`).
        (
            "C_Spell.GetSpellCooldownDuration",
            get_spell_cooldown_duration,
        ),
        ("C_Spell.GetSpellChargeDuration", spell_charge_duration),
        (
            "C_Spell.GetSpellLossOfControlCooldownDuration",
            spell_zero_duration,
        ),
        ("C_Spell.IsSpellUsable", is_spell_usable),
        ("C_Spell.IsSpellInRange", is_spell_in_range),
        ("C_Spell.SpellHasRange", spell_has_range),
        ("C_Spell.EnableSpellRangeCheck", nothing),
        ("GetSpellBaseCooldown", get_spell_base_cooldown),
        ("C_Spell.GetSpellCastCount", zero),
        ("C_Spell.GetSpellDisplayCount", get_spell_display_count),
        ("C_Spell.IsAutoRepeatSpell", is_auto_repeat_spell),
        ("C_Spell.GetSpellAutoCast", get_spell_auto_cast),
        (
            "C_Spell.GetSpellLossOfControlCooldownInfo",
            get_spell_loss_of_control_info,
        ),
        ("C_Spell.GetItemCooldown", get_item_cooldown),
        (
            "C_Spell.GetSpellDescriptionForItemLocation",
            get_spell_description_for_item_location,
        ),
        ("C_Spell.GetAuraStatChanges", get_aura_stat_changes),
        ("C_Spell.GetSpellQueueWindow", get_spell_queue_window),
        ("GetSpellQueueWindow", get_spell_queue_window),
        // No spell here is flagged important, crowd control, a defensive
        // on others, a priority aura, a self buff, disabled, a talent's,
        // a consumable's, harmful or helpful by the client's data (not
        // known to the engine).
        ("C_Spell.IsSpellImportant", no),
        ("C_Spell.IsSpellCrowdControl", no),
        ("C_Spell.IsExternalDefensive", no),
        ("C_Spell.IsPriorityAura", no),
        ("C_Spell.IsSelfBuff", no),
        ("C_Spell.IsSpellDisabled", no),
        ("C_Spell.IsClassTalentSpell", no),
        ("C_Spell.IsPvPTalentSpell", no),
        ("C_Spell.IsConsumableSpell", no),
        ("C_Spell.IsSpellHarmful", no),
        ("C_Spell.IsSpellHelpful", no),
        ("C_Spell.IsActiveSpell", no),
        ("C_Spell.CancelAutoRepeatSpell", no),
        ("C_Spell.GetSpellMaxCumulativeAuraApplications", zero),
        ("C_Spell.GetItemModifiedAppearancesApplied", empty_list),
        ("C_Spell.GetSpellSkillLineAbilityRank", nothing),
        ("C_Spell.GetSpellTradeSkillLink", nothing),
        ("C_Spell.GetVisibilityInfo", get_visibility_info),
        ("C_Spell.GetDeadlyDebuffInfo", nothing),
        ("C_Spell.GetLastCategoryCooldownSource", nothing),
        ("C_Spell.GetMawPowerLinkBySpellID", nothing),
        // Actions with no visible effect in the engine.
        ("C_Spell.CancelSpellByID", nothing),
        ("C_Spell.CancelItemTempEnchantment", nothing),
        ("C_Spell.SetSpellAutoCastEnabled", nothing),
        ("C_Spell.ToggleSpellAutoCast", nothing),
        ("C_Spell.PlaceTargetingSpellAtCursor", nothing),
        // No spell waiting for its target.
        ("SpellIsTargeting", no),
        ("SpellCanTargetItem", no),
        ("SpellCanTargetItemID", no),
        ("SpellCanTargetQuest", no),
        ("SpellCanTargetUnit", no),
        ("SpellCanTargetGarrisonFollower", no),
        ("SpellCanTargetGarrisonFollowerAbility", no),
        ("SpellCanTargetGarrisonMission", no),
        ("SpellCancelQueuedSpell", nothing),
        // C_SpellBook
        (
            "C_SpellBook.IsSpellKnownOrInSpellBook",
            is_spell_known_or_in_spellbook,
        ),
        (
            "C_SpellBook.ContainsAnyDisenchantSpell",
            contains_any_disenchant_spell,
        ),
        (
            "C_SpellBook.GetSpellBookItemDescription",
            get_spellbook_item_description,
        ),
        (
            "C_SpellBook.GetSpellBookItemCharges",
            get_spellbook_item_charges,
        ),
        (
            "C_SpellBook.GetSpellBookItemPowerCost",
            get_spellbook_item_power_cost,
        ),
        (
            "C_SpellBook.GetSpellBookItemLossOfControlCooldownInfo",
            get_spellbook_item_loss_of_control_info,
        ),
        (
            "C_SpellBook.GetSpellBookItemCastCount",
            get_spellbook_item_cast_count,
        ),
        (
            "C_SpellBook.IsAutoAttackSpellBookItem",
            is_auto_attack_spellbook_item,
        ),
        (
            "C_SpellBook.IsRangedAutoAttackSpellBookItem",
            is_ranged_auto_attack_spellbook_item,
        ),
        ("C_SpellBook.IsClassTalentSpellBookItem", no),
        ("C_SpellBook.IsPvPTalentSpellBookItem", no),
        ("C_SpellBook.IsSpellBookItemHarmful", no),
        ("C_SpellBook.IsSpellBookItemHelpful", no),
        (
            "C_SpellBook.IsSpellBookItemInRange",
            is_spellbook_item_in_range,
        ),
        (
            "C_SpellBook.IsSpellBookItemUsable",
            is_spellbook_item_usable,
        ),
        (
            "C_SpellBook.SpellBookItemHasRange",
            spellbook_item_has_range,
        ),
        (
            "C_SpellBook.IsSpellBookItemOffSpec",
            is_spellbook_item_off_spec,
        ),
        (
            "C_SpellBook.GetSpellBookItemSkillLineIndex",
            get_spellbook_item_skill_line_index,
        ),
        (
            "C_SpellBook.GetSkillLineIndexByID",
            get_skill_line_index_by_id,
        ),
        (
            "C_SpellBook.GetSpellBookItemCooldownDuration",
            get_spellbook_item_cooldown_duration,
        ),
        (
            "C_SpellBook.GetSpellBookItemChargeDuration",
            spellbook_item_charge_duration,
        ),
        (
            "C_SpellBook.GetSpellBookItemLossOfControlCooldownDuration",
            spellbook_item_zero_duration,
        ),
        ("C_SpellBook.GetSpellBookItemTradeSkillLink", nothing),
        (
            "C_SpellBook.GetCurrentLevelSpells",
            get_current_level_spells,
        ),
        ("C_SpellBook.AbortSpellIntro", nothing),
        ("C_SpellBook.SetBarSlotFromIntro", nothing),
        ("C_SpellBook.SetSpellBookItemAutoCastEnabled", nothing),
        ("C_SpellBook.ToggleSpellBookItemAutoCast", nothing),
        ("GetFlyoutID", get_flyout_id),
        // C_ActionBar
        (
            "C_ActionBar.GetActionCooldownDuration",
            get_action_cooldown_duration,
        ),
        (
            "C_ActionBar.GetActionChargeDuration",
            action_charge_duration,
        ),
        (
            "C_ActionBar.GetActionLossOfControlCooldownDuration",
            action_zero_duration,
        ),
        ("C_ActionBar.GetSpell", get_spell),
        ("C_ActionBar.GetActionAutocast", get_action_autocast),
        ("C_ActionBar.GetActionUseCount", get_action_use_count),
        ("C_ActionBar.IsOnBarOrSpecialBar", has_spell_action_buttons),
        (
            "C_ActionBar.HasSpellActionButtons",
            has_spell_action_buttons,
        ),
        (
            "C_ActionBar.FindFlyoutActionButtons",
            find_flyout_action_buttons,
        ),
        (
            "C_ActionBar.HasFlyoutActionButtons",
            has_flyout_action_buttons,
        ),
        // No pet actions on the bars, no assisted combat.
        ("C_ActionBar.FindPetActionButtons", nothing),
        ("C_ActionBar.GetPetActionPetBarIndices", nothing),
        ("C_ActionBar.HasPetActionButtons", no),
        ("C_ActionBar.HasPetActionPetBarIndices", no),
        ("C_ActionBar.FindAssistedCombatActionButtons", nothing),
        (
            "C_ActionBar.IsMacroActionWithShowTooltip",
            is_macro_action_with_show_tooltip,
        ),
        ("C_ActionBar.PutActionInSlot", put_action_in_slot),
        ("C_ActionBar.HasRangeRequirements", has_range_requirements),
        ("C_ActionBar.IsActionInRange", is_action_in_range),
        ("C_ActionBar.IsConsumableAction", is_consumable_action),
        ("C_ActionBar.IsStackableAction", is_stackable_action),
        ("C_ActionBar.IsHarmfulAction", no),
        ("C_ActionBar.IsHelpfulAction", no),
        ("C_ActionBar.IsInterruptAction", no),
        // No override bar.
        ("C_ActionBar.ShouldOverrideBarShowHealthBar", no),
        ("C_ActionBar.ShouldOverrideBarShowManaBar", no),
        ("C_ActionBar.GetOverrideBarSkin", nil),
        ("C_ActionBar.GetProfessionQualityInfo", nil),
        ("C_ActionBar.GetProfessionQuality", nil),
        ("C_ActionBar.GetItemActionOnEquipSpellID", nil),
        ("C_ActionBar.RegisterActionUIButton", nothing),
        ("C_ActionBar.UnregisterActionUIButton", nothing),
        ("C_ActionBar.EnableActionRangeCheck", nothing),
        ("C_ActionBar.ToggleAutoCastPetAction", nothing),
        ("C_ActionBar.ForceUpdateAction", nothing),
        // Macros
        ("GetMacroSpell", get_macro_spell),
        ("GetMacroItem", get_macro_item),
        // Totems (none down), companions (none: Forever keeps mounts and
        // pets as items and spells), multi-cast bars (none in classic).
        ("GetTotemInfo", get_totem_info),
        ("GetTotemDuration", get_totem_duration),
        ("GetTotemTimeLeft", totem_time_left),
        ("GetTotemCannotDismiss", totem_cannot_dismiss),
        ("GetMultiCastTotemSpells", nothing),
        ("GetNumCompanions", zero),
        ("GetCompanionInfo", nothing),
        // Specializations
        ("GetSpecializationInfoByID", get_specialization_info_by_id),
        (
            "GetSpecializationInfoForSpecID",
            get_specialization_info_for_spec_id,
        ),
        (
            "GetSpecializationNameForSpecID",
            get_specialization_name_for_spec_id,
        ),
        ("GetSpecializationRoleByID", get_specialization_role_by_id),
        ("GetPrimarySpecialization", get_primary_specialization),
        (
            "GetSpecializationInfoForClassID",
            get_specialization_info_for_class_id,
        ),
        (
            "GetSpecializationRoleEnumByID",
            get_specialization_role_enum_by_id,
        ),
        ("GetSpecializationRole", get_specialization_role),
        ("GetSpecializationSystem", get_specialization_system),
        // One specialization, nothing to switch to.
        ("C_SpecializationInfo.SetSpecialization", no),
        ("C_SpecializationInfo.GetSpecIDs", empty_list),
        ("C_SpecializationInfo.MatchesCurrentSpecSet", no),
        ("C_SpecializationInfo.SetActiveSpecGroup", nothing),
        ("C_SpecializationInfo.SetPetSpecialization", nothing),
        ("GetSpecializationSpells", nothing),
        ("GetSpecsForSpell", nothing),
        ("IsSpellClassOrSpec", nil),
        ("GetSpellsForCharacterUpgradeTier", nothing),
        // Classic talents are trait trees here (`traits`): the old talent
        // functions find none.
        ("C_SpecializationInfo.GetTalentInfo", nil),
        ("GetTalentInfoByID", nothing),
        ("GetTalentInfoBySpecialization", nothing),
        ("GetTalentTierInfo", nothing),
        ("GetTalentLink", nil),
        ("LearnTalent", nothing),
        ("LearnTalents", nothing),
        // No PvP talents (recorded: `GetPvpTalentSlotInfo` every slot
        // empty, not enabled).
        ("C_SpecializationInfo.GetPvpTalentInfo", nil),
        ("C_SpecializationInfo.GetPvpTalentUnlockLevel", nil),
        ("C_SpecializationInfo.IsPvpTalentLocked", no),
        ("C_SpecializationInfo.SetPvpTalentLocked", nothing),
        ("C_SpecializationInfo.GetInspectSelectedPvpTalent", nil),
        // C_ClassTalents: no saved loadouts, starter builds or hero talents.
        ("C_ClassTalents.LoadConfig", load_config),
        (
            "C_ClassTalents.SetStarterBuildActive",
            set_starter_build_active,
        ),
        ("C_ClassTalents.UpdateLastSelectedSavedConfigID", nothing),
        ("C_ClassTalents.DeleteConfig", no),
        ("C_ClassTalents.RenameConfig", no),
        ("C_ClassTalents.RequestNewConfig", no),
        ("C_ClassTalents.SaveConfig", no),
        ("C_ClassTalents.ViewLoadout", no),
        ("C_ClassTalents.InitializeViewLoadout", nothing),
        ("C_ClassTalents.SetUsesSharedActionBars", nothing),
        ("C_ClassTalents.SwitchToLoadoutByIndex", nothing),
        ("C_ClassTalents.SwitchToLoadoutByName", nothing),
        ("C_ClassTalents.SwitchToSpecializationByIndex", nothing),
        ("C_ClassTalents.SwitchToSpecializationByName", nothing),
        // C_Traits: what the traits model does not do.
        ("C_Traits.PurchaseAllRanks", purchase_all_ranks),
        ("C_Traits.TryPurchaseAllRanks", purchase_all_ranks),
        ("C_Traits.GetSubTreeInfo", nothing),
        ("C_Traits.GetConfigIDBySystemID", nothing),
        ("C_Traits.GetConfigVariationID", nothing),
        ("C_Traits.GenerateImportString", empty_string),
        ("C_Traits.GenerateInspectImportString", empty_string),
        ("C_Traits.ResetTree", no),
        ("C_Traits.ResetTreeByCurrency", no),
        ("C_Traits.SetSelection", no),
        ("C_Traits.StageConfig", no),
        ("C_Traits.CascadeRepurchaseRanks", no),
        ("C_Traits.RefundAllRanks", no),
        ("C_Traits.RefundRank", no),
        ("C_Traits.TryPurchaseToNode", no),
        ("C_Traits.TryRefundToNode", no),
        ("C_Traits.ClearCascadeRepurchaseHistory", nothing),
        ("C_Traits.CloseTraitSystemInteraction", nothing),
        ("C_Traits.TalentTestUnlearnSpells", nothing),
        // C_PetInfo: what `pets` does not model.
        ("C_PetInfo.GetPetFoodTypes", empty_list),
        ("C_PetInfo.GetPetTrainingPoints", zero_zero),
        ("C_PetInfo.CanPetEatItem", no),
        ("C_PetInfo.IsPetActionPassive", no),
        ("C_PetInfo.GetPetTamersForMap", empty_list),
        ("C_PetInfo.GetPetTalentTree", nothing),
        ("C_PetInfo.GetSpellForPetAction", nil),
        ("C_PetInfo.PetRename", nothing),
        ("C_PetInfo.PetAbandon", nothing),
        ("C_PetInfo.PetAssistMode", nothing),
        // C_CooldownViewer
        ("C_CooldownViewer.GetLayoutData", get_layout_data),
        ("C_CooldownViewer.SetLayoutData", set_layout_data),
        ("C_CooldownViewer.GetValidAlertTypes", empty_list),
        (
            "C_CooldownViewer.IsCooldownViewerAvailable",
            is_cooldown_viewer_available,
        ),
        // The macro icon lists: none known to the engine.
        ("GetMacroIcons", nothing),
        ("GetMacroItemIcons", nothing),
    ]
}
