//! The game's click bindings (decision T625): the character's profile of
//! what a click on a unit frame does (`C_ClickBindings`), the modifier
//! keys held as the profile writes them (`MakeModifiers`) and their names
//! (`GetStringFromModifiers`). Blizzard's `SecureUnitButton_OnClick` reads
//! them before a unit frame's `type`: a `target` or `menu` click the
//! profile has no interaction for does nothing.
//!
//! Probed in WoW Forever on the Mac (2026-11-10, `tests/ingame/PROBES.md`,
//! "Click bindings"): the profile's two interactions, Target on the left
//! button and the menu on the right one, no modifiers; Shift held is 3 (a
//! modifier's two bits, its left and right key, both for one key held);
//! `GetBindingType("LeftButton", 3)` none and
//! `GetEffectiveInteractionButton("LeftButton", 3)` nil with the default
//! profile; the names of single bits. Unverified: an interaction moved
//! (its effective button), an event on a change, the names of several
//! bits together, what `ExecuteBinding` does with the unit.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, engine_mut};

/// `Enum.ClickBindingType`'s.
const SPELL: f64 = 1.0;
const MACRO: f64 = 2.0;
const INTERACTION: f64 = 3.0;
/// `Enum.ClickBindingInteraction`'s.
const TARGET: f64 = 1.0;
const OPEN_CONTEXT_MENU: f64 = 2.0;

/// Each modifier's bits (its left key's and its right key's), in the
/// order a binding writes them (`ALT-CTRL-SHIFT-META`, decision T624).
const BITS: [(u32, &str); 4] = [
    (0b0011_0000, "ALT"),
    (0b0000_1100, "CTRL"),
    (0b0000_0011, "SHIFT"),
    (0b1100_0000, "META"),
];

/// One of the profile's bindings (`ClickBindingInfo`).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// `Enum.ClickBindingType`.
    pub kind: f64,
    /// A spell's ID, a macro's index, an interaction
    /// (`Enum.ClickBindingInteraction`), a pet action's spell.
    pub action_id: f64,
    pub button: String,
    pub modifiers: u32,
}

/// The character's click bindings: none set, the default profile.
#[derive(Clone, Debug, Default)]
pub struct ClickBindings {
    pub profile: Option<Vec<Entry>>,
}

/// The default profile, as Forever answered a fresh character's: Target on
/// the left button, the menu on the right one, no modifiers.
fn default_profile() -> Vec<Entry> {
    vec![
        Entry {
            kind: INTERACTION,
            action_id: TARGET,
            button: "LeftButton".into(),
            modifiers: 0,
        },
        Entry {
            kind: INTERACTION,
            action_id: OPEN_CONTEXT_MENU,
            button: "RightButton".into(),
            modifiers: 0,
        },
    ]
}

/// The character's profile now.
pub fn profile(vm: &Vm) -> Vec<Entry> {
    engine(vm)
        .world
        .character
        .click_bindings
        .profile
        .clone()
        .unwrap_or_else(default_profile)
}

/// The modifier keys held, as `MakeModifiers` writes them: each held key
/// both its bits (Forever: one Shift held, 3).
pub fn held_mask(vm: &Vm) -> u32 {
    let [shift, ctrl, alt, meta] = engine(vm).world.input.modifiers;
    [
        (shift, 0b11),
        (ctrl, 0b1100),
        (alt, 0b11_0000),
        (meta, 0b1100_0000),
    ]
    .iter()
    .filter(|(held, _)| *held)
    .map(|(_, bits)| bits)
    .sum()
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// A modifier mask argument (the client's checks let only 0 to 2^32-1
/// through; a fraction counts as its whole part, as the full probe's 1.5
/// answered as 1).
fn mask(vm: &Vm, v: Value) -> u32 {
    match vm.hidden(v) {
        Value::Num(n) if n >= 0.0 => n as u32,
        Value::Str(s) => std::str::from_utf8(vm.str_bytes(s))
            .ok()
            .and_then(|t| t.trim().parse::<f64>().ok())
            .map_or(0, |n| n.max(0.0) as u32),
        _ => 0,
    }
}

/// The profile's binding for this button and these modifiers, exactly.
fn binding(vm: &Vm, button: Value, modifiers: Value) -> Option<Entry> {
    let button = text(vm, button)?;
    let mods = mask(vm, modifiers);
    profile(vm)
        .into_iter()
        .find(|e| e.button == button && e.modifiers == mods)
}

/// `MakeModifiers()`: the modifier keys held (`mock.setModifiers`), each
/// key's two bits (probed: Shift held 3; Ctrl 12, Alt 48, Meta 192 by the
/// same rule).
fn make_modifiers(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(held_mask(vm) as f64)])
}

/// `GetStringFromModifiers(modifiers)`: the keys a mask holds (probed: 0
/// "", 1 and 2 "SHIFT", 4 and 8 "CTRL", 16 and 32 "ALT", 64 and 128
/// "META", 256 and 512 ""). Several keys joined with `-` in a binding's
/// order, `ALT-CTRL-SHIFT-META` (unverified: Blizzard's click binding
/// window shows the string as it comes).
fn get_string_from_modifiers(vm: &mut Vm, c: NativeCall) -> Ret {
    let m = mask(vm, c.arg(1));
    let keys: Vec<&str> = BITS
        .iter()
        .filter(|(bits, _)| m & bits != 0)
        .map(|(_, name)| *name)
        .collect();
    Ok(vec![vm.str(&keys.join("-"))])
}

/// `C_ClickBindings.GetProfileInfo()`: the profile's bindings, copies.
fn get_profile_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    let list = profile(vm);
    let out = vm.new_table();
    for (i, e) in list.into_iter().enumerate() {
        let t = vm.new_table();
        let button = vm.str(&e.button);
        for (k, v) in [
            ("type", Value::Num(e.kind)),
            ("actionID", Value::Num(e.action_id)),
            ("button", button),
            ("modifiers", Value::Num(e.modifiers as f64)),
        ] {
            let key = vm.str(k);
            let _ = vm.heap.table_mut(t).set(key, v);
        }
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num((i + 1) as f64), Value::Table(t));
    }
    Ok(vec![Value::Table(out)])
}

/// `C_ClickBindings.SetProfileByInfo(infoVec)`: the profile replaced by
/// the list's `{ type, actionID, button, modifiers }` entries, in order
/// (`modifiers` 0 when left out; an entry without a type or a button is
/// left out, unverified). No event (unverified: the documentation names
/// none for it).
fn set_profile_by_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(list) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    let mut entries = Vec::new();
    for i in 1.. {
        let Value::Table(t) = vm.heap.table(list).get(Value::Num(i as f64)) else {
            break;
        };
        let mut get = |k: &str| {
            let key = vm.str(k);
            vm.heap.table(t).get(key)
        };
        let (kind, action, button, mods) = (
            get("type"),
            get("actionID"),
            get("button"),
            get("modifiers"),
        );
        let (Value::Num(kind), Some(button)) = (vm.hidden(kind), text(vm, button)) else {
            continue;
        };
        let action_id = match vm.hidden(action) {
            Value::Num(n) => n,
            _ => 0.0,
        };
        entries.push(Entry {
            kind,
            action_id,
            button,
            modifiers: mask(vm, mods),
        });
    }
    engine_mut(vm).world.character.click_bindings.profile = Some(entries);
    Ok(vec![])
}

/// `C_ClickBindings.ResetCurrentProfile()`: the default profile again.
fn reset_current_profile(vm: &mut Vm, _c: NativeCall) -> Ret {
    engine_mut(vm).world.character.click_bindings.profile = None;
    Ok(vec![])
}

/// `C_ClickBindings.GetBindingType(button, modifiers)`: the type of the
/// profile's binding for exactly this button and these modifiers, else
/// none (0). Recorded: the left button plain an interaction (3), held
/// Shift (3) none.
fn get_binding_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let kind = binding(vm, c.arg(1), c.arg(2)).map_or(0.0, |e| e.kind);
    Ok(vec![Value::Num(kind)])
}

/// `C_ClickBindings.GetEffectiveInteractionButton(button, modifiers)`: for
/// an interaction bound there, the interaction's default button (Target the
/// left button, the menu the right one), whose `type` attribute Blizzard's
/// `SecureUnitButton_OnClick` then reads, as its comment says
/// (unverified: an interaction moved off its default); nil without one
/// (probed: Shift and the left button with the default profile, a name
/// that is no button).
fn get_effective_interaction_button(vm: &mut Vm, c: NativeCall) -> Ret {
    let button = match binding(vm, c.arg(1), c.arg(2)) {
        Some(e) if e.kind == INTERACTION && e.action_id == TARGET => "LeftButton",
        Some(e) if e.kind == INTERACTION && e.action_id == OPEN_CONTEXT_MENU => "RightButton",
        _ => return Ok(vec![Value::Nil]),
    };
    Ok(vec![vm.str(button)])
}

/// `C_ClickBindings.ExecuteBinding(unit, button, modifiers)` (Blizzard's
/// unit frames' click on a spell or macro binding): a spell is pressed as
/// the player presses a spell (`casting::use_spell`, as `UseAction`
/// does; unverified: on the clicked unit), a macro (by index) runs. A pet
/// action does nothing yet (unverified).
fn execute_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(e) = binding(vm, c.arg(2), c.arg(3)) else {
        return Ok(vec![]);
    };
    if e.kind == SPELL {
        return crate::casting::use_spell(vm, e.action_id);
    }
    if e.kind == MACRO {
        crate::macros::run_at(vm, e.action_id);
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("MakeModifiers", make_modifiers),
        ("GetStringFromModifiers", get_string_from_modifiers),
        ("C_ClickBindings.GetProfileInfo", get_profile_info),
        ("C_ClickBindings.SetProfileByInfo", set_profile_by_info),
        ("C_ClickBindings.ResetCurrentProfile", reset_current_profile),
        ("C_ClickBindings.GetBindingType", get_binding_type),
        (
            "C_ClickBindings.GetEffectiveInteractionButton",
            get_effective_interaction_button,
        ),
        ("C_ClickBindings.ExecuteBinding", execute_binding),
    ]
}
