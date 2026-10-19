//! The action bars: the action in each slot (a world profile's `actions`,
//! recorded by `/stdtest snapshot`; `mock.setAction`), read through
//! `C_ActionBar` and `GetActionInfo` as the client answers (recorded,
//! `/stdtest engine answers`): slot 0 and below answer nothing; an empty
//! slot no texture, an empty count, a cooldown not enabled, not usable; a
//! filled slot its action, its texture and count, a cooldown enabled and
//! idle, usable as recorded (else usable; unverified). No cooldowns,
//! charges or loss of control run yet.

use std::collections::BTreeMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, engine_mut, widgets};

/// An action in a slot.
#[derive(Clone, Debug, Default)]
pub struct Action {
    /// `spell`, `item`, `macro`, `companion`, `equipmentset`, …
    pub kind: String,
    pub id: Value,
    pub sub_type: Value,
    pub texture: Value,
    /// The count shown (`GetActionDisplayCount`), "" for none.
    pub count: String,
    /// `GetActionText` (a macro's name).
    pub text: Value,
    /// `IsUsableAction`: usable, lacking power.
    pub usable: (bool, bool),
}

/// The actions, by slot.
pub type Actions = BTreeMap<i64, Action>;

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// The slot asked about: `None` for slot 0 and below (no answer).
fn slot(c: &NativeCall) -> Option<i64> {
    num(c.arg(1)).filter(|n| *n >= 1.0).map(|n| n as i64)
}

/// The global cooldown's category (`Enum`-less: the spells' start
/// recovery category, recorded).
pub const GLOBAL_COOLDOWN: f64 = 133.0;
/// The global cooldown's length (recorded: 1.5 s).
pub const GLOBAL_COOLDOWN_LENGTH: f64 = 1.5;

/// Auto Attack's spell.
const AUTO_ATTACK: f64 = 6603.0;

/// The global cooldown starts (a cast begun).
pub fn start_global_cooldown(vm: &mut Vm) {
    let now = crate::clock::now(vm);
    engine_mut(vm).world.global_cooldown = Some((now, GLOBAL_COOLDOWN_LENGTH));
}

/// `setSpellTraits({ offGlobalCooldown = { id, … }, tracking = { … },
/// profession = { … }, partyOrSelf = { … }, interruptible = { … },
/// weapon = { … } })`: what the client knows of the character's spells and
/// the engine does not (recorded: Judgement off the global cooldown, Find
/// Minerals and Sense Undead tracking, the crafts professions, the
/// Sharpening Wheel's craft interruptible, Holy Strike a weapon's; probed:
/// Blessing of Protection on the player or a member in reach only).
pub fn control_set_spell_traits(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setSpellTraits: a table of trait lists expected"));
    };
    let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
    for (k, v) in entries {
        let (Value::Str(k), Value::Table(list)) = (k, v) else {
            continue;
        };
        let trait_name = String::from_utf8_lossy(vm.str_bytes(k)).into_owned();
        let n = vm.heap.table(list).border();
        for i in 1..=n {
            if let Value::Num(id) = vm.heap.table(list).get(Value::Num(i as f64)) {
                engine_mut(vm)
                    .world
                    .spell_traits
                    .entry(trait_name.clone())
                    .or_default()
                    .insert(id as i64);
            }
        }
    }
    Ok(vec![])
}

/// Whether a spell has a trait (`setSpellTraits`).
pub fn has_trait(vm: &Vm, spell: Value, trait_name: &str) -> bool {
    let Value::Num(id) = spell else { return false };
    engine(vm)
        .world
        .spell_traits
        .get(trait_name)
        .is_some_and(|s| s.contains(&(id as i64)))
}

fn action(vm: &Vm, slot: i64) -> Option<Action> {
    engine(vm).world.character.actions.get(&slot).cloned()
}

fn table(vm: &mut Vm, fields: &[(&str, Value)]) -> Value {
    let t = vm.new_table();
    for (k, v) in fields {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, *v);
    }
    Value::Table(t)
}

fn has_action(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => vec![Value::Bool(action(vm, s).is_some())],
        None => vec![],
    })
}

/// `C_ActionBar.IsItemAction(slot)`: an item there (recorded).
fn is_item_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let item = slot(&c)
        .and_then(|s| action(vm, s))
        .is_some_and(|a| a.kind == "item");
    Ok(vec![Value::Bool(item)])
}

/// The gamepad bars' first slot (the items recording: slot 184 mirrors
/// 12).
const GAMEPAD_SLOTS: i64 = 181;

/// `C_ActionBar.FindSpellActionButtons(spellID [, setToSearch])`: the slots
/// holding the spell, in order, or nothing; `Enum.ActionBarSet` All (3, the
/// default), Mkb (1: not the gamepad bars, recorded: slot 200 left out),
/// Gamepad (2, unverified). Unverified: overridden spells.
fn find_spell_action_buttons(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(spell) = num(c.arg(1)) else {
        return Ok(vec![]);
    };
    let set = num(c.arg(2)).unwrap_or(3.0);
    let slots: Vec<i64> = engine(vm)
        .world
        .character
        .actions
        .iter()
        .filter(|(_, a)| a.kind == "spell" && num(a.id) == Some(spell))
        .map(|(s, _)| *s)
        .filter(|s| match set as i64 {
            1 => *s < GAMEPAD_SLOTS,
            2 => *s >= GAMEPAD_SLOTS,
            3 => true,
            _ => false,
        })
        .collect();
    if slots.is_empty() {
        return Ok(vec![]);
    }
    let list = vm.new_table();
    for (i, s) in slots.into_iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(list)
            .set(Value::Num(i as f64 + 1.0), Value::Num(s as f64));
    }
    Ok(vec![Value::Table(list)])
}

/// `GetActionInfo(slot)`: the action's type, ID and sub type; an item's
/// type and ID alone (recorded: slots 181 and 184).
fn get_action_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c).and_then(|s| action(vm, s)) {
        Some(a) if a.kind == "item" => vec![vm.str(&a.kind), a.id],
        Some(a) => vec![vm.str(&a.kind), a.id, a.sub_type],
        None => vec![],
    })
}

fn get_action_texture(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => vec![action(vm, s).map_or(Value::Nil, |a| a.texture)],
        None => vec![],
    })
}

fn get_action_text(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => vec![action(vm, s).map_or(Value::Nil, |a| a.text)],
        None => vec![],
    })
}

fn get_action_display_count(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => {
            let count = action(vm, s).map(|a| a.count).unwrap_or_default();
            vec![vm.str(&count)]
        }
        None => vec![],
    })
}

fn is_usable_action(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => {
            let (usable, no_power) = action(vm, s).map_or((false, false), |a| usability(vm, &a));
            vec![Value::Bool(usable), Value::Bool(no_power)]
        }
        None => vec![],
    })
}

/// A data field of a defined item or spell.
fn data_field(vm: &mut Vm, item: bool, id: Value, key: &str) -> Value {
    let Value::Num(n) = id else { return Value::Nil };
    let data = &engine(vm).world.data;
    let t = if item {
        data.items.get(&(n as i64))
    } else {
        data.spells.get(&(n as i64))
    };
    let Some(t) = t.copied() else {
        return Value::Nil;
    };
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

/// An action's usability now: usable, lacking power. As recorded (the
/// profile's), but food and drink not in combat, and a spell with a power
/// cost (its data's `powerCost`) only with that much of the player's
/// power (`/stdtest engine scenario combat`: Tough Jerky, Heroic Strike).
fn usability(vm: &mut Vm, a: &Action) -> (bool, bool) {
    // Dead or a ghost: nothing (recorded, every action at the death).
    let p = &engine(vm).units.list[crate::units::PLAYER.0];
    if p.dead || p.ghost {
        return (false, false);
    }
    // On a flight: no spell but the professions' (taking off), the
    // tracking ones too once on the taxi; no macro, even one without a
    // spell (recorded: a whisper's).
    let stage = engine(vm).world.taxi_stage;
    if stage > 0 && a.kind == "macro" {
        return (false, false);
    }
    if stage > 0 && a.kind == "spell" && !has_trait(vm, a.id, "profession") {
        let tracking = has_trait(vm, a.id, "tracking");
        if stage == 1 || !tracking {
            return (false, false);
        }
    }
    // A weapon's strike (trait `weapon`): unusable with no main hand, or
    // while the inventory loads (recorded: Holy Strike told usable once it
    // loaded after a loading screen; unverified: unarmed).
    if a.kind == "spell" && has_trait(vm, a.id, "weapon") {
        let w = &engine(vm).world;
        if w.inventory_loading || !w.character.equipment.contains_key(&16) {
            return (false, false);
        }
    }
    let in_combat = engine(vm).units.list[crate::units::PLAYER.0].in_combat;
    // A spell on the player or a group member in reach only (trait
    // `partyOrSelf`; probed: Blessing of Protection usable targeting the
    // player, not with no target, an NPC, or a member in another zone;
    // recorded: usable on a member nearby). With no target, the soft
    // friend decides (probed: unusable with a friendly NPC soft targeted;
    // recorded, `profession`, on the road); with neither, usable (probed
    // in the open world, away from anyone). A player soft friend while not
    // grouped is (recorded, `group`: usable with the druid soft targeted,
    // unusable once the party formed without it; unverified: a player
    // targeted).
    if a.kind == "spell" && has_trait(vm, a.id, "partyOrSelf") {
        let grouped = crate::party::grouped(vm);
        let e = engine(vm);
        let u = &e.units;
        let ok = match u.target.or(u.soft[1]) {
            None => true,
            Some(crate::units::PLAYER) => true,
            Some(t) if u.target.is_none() && !grouped && u.list[t.0].player => true,
            Some(t) => {
                let m = &u.list[t.0];
                let here = &e.world.character.position.zone;
                u.group.contains(&t) && m.online && m.zone.as_ref().is_none_or(|z| z == here)
            }
        };
        return (ok, false);
    }
    if a.kind == "item" {
        let class = data_field(vm, true, a.id, "classID");
        let sub = data_field(vm, true, a.id, "subclassID");
        let food = matches!((class, sub), (Value::Num(c), Value::Num(s)) if c == 0.0 && s == 5.0);
        if food {
            return (a.usable.0 && !in_combat, false);
        }
    }
    // A spell only out of combat (data `outOfCombat`; recorded: Redemption
    // unusable once in combat, not short of mana).
    if a.kind == "spell" && in_combat && data_field(vm, false, a.id, "outOfCombat").truthy() {
        return (false, false);
    }
    if a.kind == "spell"
        && let Value::Num(cost) = data_field(vm, false, a.id, "powerCost")
    {
        let power = engine(vm).units.list[crate::units::PLAYER.0].power;
        let enough = power >= cost;
        return (enough, !enough);
    }
    a.usable
}

/// After the player's power or combat changed: the actions whose
/// usability changed, `ACTION_USABLE_CHANGED({ { slot, usable, noMana },
/// … })` in slot order, then `ACTIONBAR_UPDATE_USABLE()` (recorded).
pub fn update_usability(vm: &mut Vm) {
    if usability_changed(vm) {
        widgets::fire_event(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    }
}

/// The actions whose usability changed told (`ACTION_USABLE_CHANGED`);
/// whether any did (the bars' `ACTIONBAR_UPDATE_USABLE` is the caller's:
/// after a spell's swing it comes after the cooldowns, recorded).
pub fn usability_changed(vm: &mut Vm) -> bool {
    // Batching frames: checked at the frame's end (`batch`).
    if crate::batch::owe_usability(vm, true) {
        return false;
    }
    usability_changed_of(vm, &|_, _| true)
}

/// After the player's target changed: the actions on a target's kind
/// alone (`partyOrSelf`), others' changes left for their own moment
/// (recorded: a warrior's rage-bound spell told after a kill's loot).
pub fn update_target_usability(vm: &mut Vm) {
    if crate::batch::owe_usability(vm, false) {
        return;
    }
    target_usability_changed(vm);
}

/// The target-bound actions' usability now (`update_target_usability`).
pub(crate) fn target_usability_changed(vm: &mut Vm) {
    let target_bound =
        |vm: &Vm, a: &Action| a.kind == "spell" && has_trait(vm, a.id, "partyOrSelf");
    if usability_changed_of(vm, &target_bound) {
        widgets::fire_event(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    }
}

/// The bars reloaded (a loading screen's `ACTIONBAR_SLOT_CHANGED(0)`): each
/// action's usability as it is now, untold (recorded, `hearth`: Holy
/// Strike told usable once the inventory loaded, never told unusable;
/// Blessing of Protection told unusable after the load, the soft friend
/// dropped at the loading screen never told).
pub fn bars_reloaded(vm: &mut Vm) {
    let actions: Vec<(i64, Action)> = engine(vm)
        .world
        .character
        .actions
        .iter()
        .map(|(k, a)| (*k, a.clone()))
        .collect();
    for (slot, a) in actions {
        let now = usability(vm, &a);
        engine_mut(vm).world.character.usable_seen.insert(slot, now);
    }
}

fn usability_changed_of(vm: &mut Vm, which: &dyn Fn(&Vm, &Action) -> bool) -> bool {
    let actions: Vec<(i64, Action)> = engine(vm)
        .world
        .character
        .actions
        .iter()
        .filter(|(_, a)| which(vm, a))
        .map(|(k, a)| (*k, a.clone()))
        .collect();
    let mut changed = Vec::new();
    for (slot, a) in actions {
        let now = usability(vm, &a);
        let before = engine(vm)
            .world
            .character
            .usable_seen
            .get(&slot)
            .copied()
            .unwrap_or(a.usable);
        if now != before {
            changed.push((slot, now));
        }
        engine_mut(vm).world.character.usable_seen.insert(slot, now);
    }
    if changed.is_empty() {
        return false;
    }
    let list = vm.new_table();
    for (i, (slot, (usable, no_power))) in changed.into_iter().enumerate() {
        let entry = table(
            vm,
            &[
                ("slot", Value::Num(slot as f64)),
                ("usable", Value::Bool(usable)),
                ("noMana", Value::Bool(no_power)),
            ],
        );
        let _ = vm
            .heap
            .table_mut(list)
            .set(Value::Num(i as f64 + 1.0), entry);
    }
    widgets::fire_event(vm, "ACTION_USABLE_CHANGED", &[Value::Table(list)]);
    true
}

/// `IsCurrentAction`, `IsAutoRepeatAction`, `IsEquippedAction`,
/// `UsesActionText`: false for a slot (none running; unverified for
/// equipped items' actions).
fn not_for_slot(_vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(_) => vec![Value::Bool(false)],
        None => vec![],
    })
}

/// `IsAttackAction(slot)`: whether the slot holds Auto Attack (spell
/// 6603; unverified for a filled slot: the recording's were empty).
fn is_attack_action(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match slot(&c) {
        Some(s) => vec![Value::Bool(action(vm, s).is_some_and(|a| {
            a.kind == "spell" && matches!(a.id, Value::Num(n) if n == 6603.0)
        }))],
        None => vec![],
    })
}

/// `IsAssistedCombatAction`: false, even for slot 0 (recorded).
fn not_assisted(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `C_ActionBar.GetActionCooldown(slot)`: a spell's slot during the global
/// cooldown (category 133, its start, length, what is left), none else;
/// behind a loading screen no slot enabled (recorded).
fn get_action_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(s) = slot(&c) else {
        return Ok(vec![]);
    };
    let a = action(vm, s);
    let enabled = a.is_some() && !engine(vm).world.loading_screen;
    // An empty slot's cooldown is plain in combat too (recorded).
    if a.is_none() {
        engine_mut(vm).plain_results = true;
    }
    let now = crate::clock::now(vm);
    // Off the global cooldown: Auto Attack, a spell on the next swing (the
    // combat recording's Heroic Strike), a trait's or the data's.
    let off = |vm: &mut Vm, id: Value| {
        num(id) == Some(AUTO_ATTACK)
            || has_trait(vm, id, "offGlobalCooldown")
            || num(id).is_some_and(|id| {
                crate::casting::spell_field(vm, id, "nextSwing").truthy()
                    || crate::casting::spell_field(vm, id, "offGCD").truthy()
            })
    };
    let gcd = match (&a, engine(vm).world.global_cooldown) {
        (Some(a), Some((start, length))) if a.kind == "spell" && start + length > now => {
            (!off(vm, a.id)).then_some((start, length))
        }
        _ => None,
    };
    // An item of a food's category just used (`inventory.rs`).
    if let Some(a) = &a
        && a.kind == "item"
        && let Some((category, start, length)) = engine(vm).units.casting.category_cooldown
        && start + length > now
        && num(data_field(vm, true, a.id, "cooldownCategory")) == Some(category)
    {
        let t = table(
            vm,
            &[
                ("startTime", Value::Num(start)),
                ("duration", Value::Num(length)),
                ("isEnabled", Value::Bool(true)),
                ("modRate", Value::Num(1.0)),
                ("isActive", Value::Bool(true)),
            ],
        );
        return Ok(vec![t]);
    }
    // Its own cooldown, running since before the snapshot (recorded: a
    // warrior's three-minute spell on its slot until it ended), longer
    // than the global one: told without the global cooldown's fields.
    let own = match &a {
        Some(a) if a.kind == "spell" => num(a.id)
            .and_then(|id| crate::stats::recorded_cooldown(vm, id))
            .filter(|(start, length)| start + length > now),
        _ => None,
    };
    let gcd_end = gcd.map(|(s, l)| s + l);
    if let Some((start, length)) = own
        && gcd_end.is_none_or(|end| start + length > end)
    {
        let t = table(
            vm,
            &[
                ("startTime", Value::Num(start)),
                ("duration", Value::Num(length)),
                ("isEnabled", Value::Bool(true)),
                ("modRate", Value::Num(1.0)),
                ("isActive", Value::Bool(true)),
                (
                    "timeUntilEndOfStartRecovery",
                    Value::Num(((start + length - now) * 1000.0).round() / 1000.0),
                ),
            ],
        );
        return Ok(vec![t]);
    }
    if let Some((start, length)) = gcd {
        let t = table(
            vm,
            &[
                ("startTime", Value::Num(start)),
                ("duration", Value::Num(length)),
                ("isEnabled", Value::Bool(true)),
                ("modRate", Value::Num(1.0)),
                ("isActive", Value::Bool(true)),
                ("isOnGCD", Value::Bool(true)),
                ("activeCategory", Value::Num(GLOBAL_COOLDOWN)),
                (
                    "timeUntilEndOfStartRecovery",
                    Value::Num(((start + length - now) * 1000.0).round() / 1000.0),
                ),
            ],
        );
        return Ok(vec![t]);
    }
    let t = table(
        vm,
        &[
            ("startTime", Value::Num(0.0)),
            ("duration", Value::Num(0.0)),
            ("isEnabled", Value::Bool(enabled)),
            ("modRate", Value::Num(1.0)),
            ("isActive", Value::Bool(false)),
        ],
    );
    Ok(vec![t])
}

fn get_action_charges(vm: &mut Vm, c: NativeCall) -> Ret {
    if slot(&c).is_none() {
        return Ok(vec![]);
    }
    let t = table(
        vm,
        &[
            ("currentCharges", Value::Num(0.0)),
            ("maxCharges", Value::Num(0.0)),
            ("cooldownStartTime", Value::Num(0.0)),
            ("cooldownDuration", Value::Num(0.0)),
            ("chargeModRate", Value::Num(1.0)),
            ("isActive", Value::Bool(false)),
        ],
    );
    Ok(vec![t])
}

fn get_loss_of_control_cooldown(vm: &mut Vm, c: NativeCall) -> Ret {
    if slot(&c).is_none() {
        return Ok(vec![]);
    }
    let t = table(
        vm,
        &[
            ("startTime", Value::Num(0.0)),
            ("duration", Value::Num(0.0)),
            ("modRate", Value::Num(1.0)),
            ("isActive", Value::Bool(false)),
            ("shouldReplaceNormalCooldown", Value::Bool(false)),
        ],
    );
    Ok(vec![t])
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        Value::Num(n) => Some(format!("{n}")),
        _ => None,
    }
}

/// `mock.setAction(slot, action)`: the action in a slot (`nil`: none),
/// `{ type, id, subType, texture, count, text, usable }` (`usable`:
/// `{ usable, noPower }` or a boolean); `ACTIONBAR_SLOT_CHANGED`.
pub fn control_set_action(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = vm.check_int(&c, 1)?;
    let action = match c.arg(2) {
        Value::Nil => None,
        Value::Table(t) => {
            let kind = field(vm, t, "type");
            let Some(kind) = text(vm, kind) else {
                return Err(vm.error("setAction: the action's type expected"));
            };
            let usable = match field(vm, t, "usable") {
                Value::Bool(b) => (b, false),
                Value::Table(u) => (
                    matches!(vm.heap.table(u).get(Value::Num(1.0)), Value::Bool(true)),
                    matches!(vm.heap.table(u).get(Value::Num(2.0)), Value::Bool(true)),
                ),
                _ => (true, false),
            };
            let count = field(vm, t, "count");
            let id = field(vm, t, "id");
            let sub_type = field(vm, t, "subType");
            let texture = field(vm, t, "texture");
            let label = field(vm, t, "text");
            for v in [id, sub_type, texture, label] {
                widgets::keep(vm, v);
            }
            Some(Action {
                kind,
                id,
                sub_type,
                texture,
                count: text(vm, count).unwrap_or_default(),
                text: label,
                usable,
            })
        }
        _ => return Err(vm.error("setAction: a table of the action, or nil, expected")),
    };
    let actions = &mut engine_mut(vm).world.character.actions;
    match action {
        Some(a) => actions.insert(n, a),
        None => actions.remove(&n),
    };
    widgets::fire_event(vm, "ACTIONBAR_SLOT_CHANGED", &[Value::Num(n as f64)]);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_ActionBar.HasAction", has_action),
        ("C_ActionBar.IsItemAction", is_item_action),
        (
            "C_ActionBar.FindSpellActionButtons",
            find_spell_action_buttons,
        ),
        ("HasAction", has_action),
        ("GetActionInfo", get_action_info),
        ("C_ActionBar.GetActionTexture", get_action_texture),
        ("GetActionTexture", get_action_texture),
        ("GetActionText", get_action_text),
        ("C_ActionBar.GetActionText", get_action_text),
        (
            "C_ActionBar.GetActionDisplayCount",
            get_action_display_count,
        ),
        ("C_ActionBar.IsUsableAction", is_usable_action),
        ("IsUsableAction", is_usable_action),
        ("C_ActionBar.IsCurrentAction", not_for_slot),
        ("C_ActionBar.IsAutoRepeatAction", not_for_slot),
        ("C_ActionBar.IsEquippedAction", not_for_slot),
        ("C_ActionBar.UsesActionText", not_for_slot),
        ("C_ActionBar.IsAttackAction", is_attack_action),
        ("IsAttackAction", is_attack_action),
        // No pet actions or outfits on the bars yet (recorded false).
        ("C_ActionBar.IsAutoCastPetAction", not_for_slot),
        ("C_ActionBar.IsEnabledAutoCastPetAction", not_for_slot),
        ("C_ActionBar.IsEquippedGearOutfitAction", not_for_slot),
        ("C_ActionBar.IsAssistedCombatAction", not_assisted),
        ("C_ActionBar.GetActionCooldown", get_action_cooldown),
        ("C_ActionBar.GetActionCharges", get_action_charges),
        (
            "C_ActionBar.GetActionLossOfControlCooldownInfo",
            get_loss_of_control_cooldown,
        ),
    ]
}
