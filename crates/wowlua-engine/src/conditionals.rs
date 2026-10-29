//! Macro conditionals: `SecureCmdOptionParse(options)`, which Blizzard's
//! state drivers (`SecureStateDriver.lua`), slash commands
//! (`SlashCommands.lua`: `/cast`, `/use`, `/target` with options) and
//! addons' bar paging evaluate; and the action bar page they test
//! (`C_ActionBar.GetActionBarPage`, `C_ActionBar.SetActionBarPage`).
//!
//! The syntax is the client's documented one (no probe yet; Wowpedia's
//! macro conditionals are a hint, Blizzard's callers the evidence for the
//! call shape): clauses separated by `;`, each `[cond,cond][cond] value`
//! or a bare value. A clause matches when it has no brackets or when all
//! conditions of one of its bracket groups hold; the first match gives its
//! value (trimmed) and the group's target (`@unit` or `target=unit`), or
//! the target nil. No match gives nothing. Conditions take a `no` prefix
//! (negation) and `:a/b` arguments (any of them). Unit conditions test the
//! group's target, else `target`. Each condition asks what the engine
//! answers for the client function that reports it (its behaviour by name,
//! so recorded answers count), never the addons' globals. The conditions
//! are those the compiler checks state drivers against (`wowlua-sema`'s
//! `MACRO_CONDITIONS`), their names in lower case; an unknown one, a known
//! one capitalized among them, holds. Recorded (`/stdtest engine answers`):
//! `[combat] a` out of combat answers nothing, `[Combat] a; b` and
//! `[frobnicate] a; b` "a", `[group:party] a; b` alone "b", targets by
//! `@unit` and `target=unit`; the rest unverified.

use std::rc::Rc;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, engine_mut};

/// The action bar page (`C_ActionBar.GetActionBarPage`).
pub struct Bars {
    pub page: f64,
}

impl Default for Bars {
    /// Page 1: the client's answer at load (`camelot-load-calls.txt`).
    fn default() -> Self {
        Bars { page: 1.0 }
    }
}

/// What the engine answers for the client function `name` called with
/// `args` (its behaviour; none: nothing). Secret answers are read as the
/// client's C code reads them, unwrapped.
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

/// The first answer as a number.
pub(crate) fn number(vm: &mut Vm, name: &'static str, args: &[Value]) -> Option<f64> {
    match ask(vm, name, args).first() {
        Some(Value::Num(n)) => Some(*n),
        _ => None,
    }
}

/// The first answer as text.
fn text(vm: &mut Vm, name: &'static str, args: &[Value]) -> Option<String> {
    match ask(vm, name, args).first() {
        Some(Value::Str(s)) => Some(String::from_utf8_lossy(vm.str_bytes(*s)).into_owned()),
        _ => None,
    }
}

fn trim(s: &[u8]) -> &[u8] {
    s.trim_ascii()
}

/// The options' first matching clause: its value and its target.
pub fn parse(vm: &mut Vm, options: &[u8]) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    for clause in options.split(|&b| b == b';') {
        let mut rest = trim(clause);
        let mut groups = Vec::new();
        while rest.first() == Some(&b'[') {
            // An unclosed bracket: the rest is its group (unverified).
            let end = rest.iter().position(|&b| b == b']').unwrap_or(rest.len());
            groups.push(&rest[1..end]);
            rest = trim(rest.get(end + 1..).unwrap_or_default());
        }
        let value = trim(rest).to_vec();
        if groups.is_empty() {
            return Some((value, None));
        }
        for group in groups {
            if let Some(target) = group_holds(vm, group) {
                return Some((value, target));
            }
        }
    }
    None
}

/// Whether all of a bracket group's conditions hold; if so, its target.
/// The target applies to the whole group, wherever it stands in it
/// (unverified).
fn group_holds(vm: &mut Vm, group: &[u8]) -> Option<Option<Vec<u8>>> {
    let conditions: Vec<&[u8]> = group
        .split(|&b| b == b',')
        .map(trim)
        .filter(|c| !c.is_empty())
        .collect();
    let mut target = None;
    for c in &conditions {
        if let Some(unit) = c.strip_prefix(b"@") {
            target = Some(trim(unit).to_vec());
        } else if c.len() > 7 && c[..7].eq_ignore_ascii_case(b"target=") {
            target = Some(trim(&c[7..]).to_vec());
        }
    }
    let unit = target.clone().unwrap_or_else(|| b"target".to_vec());
    for c in conditions {
        if c.starts_with(b"@") || (c.len() > 7 && c[..7].eq_ignore_ascii_case(b"target=")) {
            continue;
        }
        if !condition_holds(vm, c, &unit) {
            return None;
        }
    }
    Some(target)
}

/// One condition (`nocombat`, `mod:shift/ctrl`), testing `unit`.
fn condition_holds(vm: &mut Vm, c: &[u8], unit: &[u8]) -> bool {
    let text = String::from_utf8_lossy(c).into_owned();
    // The name as written: "Combat" is no condition (recorded).
    let (name, args) = match text.split_once(':') {
        Some((n, a)) => (n.trim().to_string(), Some(a.to_string())),
        None => (text.trim().to_string(), None),
    };
    let args: Vec<String> = args
        .map(|a| a.split('/').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    if let Some(holds) = test(vm, &name, &args, unit) {
        return holds;
    }
    // `no` negates a known condition (`noexists`, `nomod:shift`).
    if let Some(positive) = name.strip_prefix("no")
        && let Some(holds) = test(vm, positive, &args, unit)
    {
        return !holds;
    }
    // Unknown: it holds (recorded: `[frobnicate] a; b` is "a").
    true
}

/// The arguments' numbers.
fn numbers(args: &[String]) -> Vec<f64> {
    args.iter().filter_map(|a| a.parse().ok()).collect()
}

/// A condition by name, or None when unknown.
fn test(vm: &mut Vm, name: &str, args: &[String], unit: &[u8]) -> Option<bool> {
    let player = vm.str("player");
    let u = Value::Str(vm.heap.intern(unit));
    // A numbered state against the arguments (none: any but 0).
    let numbered = |n: Option<f64>, args: &[String]| {
        let n = n.unwrap_or(0.0);
        if args.is_empty() {
            n != 0.0
        } else {
            numbers(args).contains(&n)
        }
    };
    Some(match name {
        "combat" => yes(vm, "UnitAffectingCombat", &[player]),
        "mod" | "modifier" => {
            if args.is_empty() {
                yes(vm, "IsModifierKeyDown", &[])
            } else {
                args.iter().any(|a| modifier_held(vm, a))
            }
        }
        "harm" => yes(vm, "UnitCanAttack", &[player, u]),
        "help" => yes(vm, "UnitCanAssist", &[player, u]),
        "exists" => yes(vm, "UnitExists", &[u]),
        "dead" => yes(vm, "UnitIsDeadOrGhost", &[u]),
        "party" => yes(vm, "UnitPlayerOrPetInParty", &[u]),
        "raid" => yes(vm, "UnitPlayerOrPetInRaid", &[u]),
        "unithasvehicleui" => yes(vm, "UnitHasVehicleUI", &[u]),
        "group" => {
            if args.is_empty() {
                yes(vm, "IsInGroup", &[])
            } else {
                args.iter().any(|a| match a.to_ascii_lowercase().as_str() {
                    "party" => yes(vm, "IsInGroup", &[]),
                    "raid" => yes(vm, "IsInRaid", &[]),
                    _ => false,
                })
            }
        }
        "pet" => {
            let pet = vm.str("pet");
            if !yes(vm, "UnitExists", &[pet]) {
                false
            } else if args.is_empty() {
                true
            } else {
                let name = text(vm, "UnitName", &[pet]);
                let family = text(vm, "UnitCreatureFamily", &[pet]);
                args.iter().any(|a| {
                    [&name, &family]
                        .iter()
                        .any(|v| v.as_deref().is_some_and(|v| v.eq_ignore_ascii_case(a)))
                })
            }
        }
        "actionbar" | "bar" => {
            let page = number(vm, "C_ActionBar.GetActionBarPage", &[]);
            numbered(page, args)
        }
        "bonusbar" => {
            let offset = number(vm, "C_ActionBar.GetBonusBarOffset", &[]);
            numbered(offset, args)
        }
        "stance" | "form" => {
            let form = number(vm, "GetShapeshiftForm", &[]);
            numbered(form, args)
        }
        "spec" => {
            let spec = number(vm, "C_SpecializationInfo.GetSpecialization", &[]);
            numbered(spec, args)
        }
        "overridebar" => yes(vm, "C_ActionBar.HasOverrideActionBar", &[]),
        "possessbar" => yes(vm, "C_ActionBar.IsPossessBarVisible", &[]),
        "vehicleui" => yes(vm, "C_ActionBar.HasVehicleActionBar", &[]),
        "extrabar" => yes(vm, "C_ActionBar.HasExtraActionBar", &[]),
        "shapeshift" => yes(vm, "C_ActionBar.HasTempShapeshiftActionBar", &[]),
        "petbattle" => yes(vm, "C_PetBattles.IsInBattle", &[]),
        "canexitvehicle" => yes(vm, "CanExitVehicle", &[]),
        "stealth" => yes(vm, "IsStealthed", &[]),
        "mounted" => yes(vm, "IsMounted", &[]),
        "flying" => yes(vm, "IsFlying", &[]),
        "flyable" => yes(vm, "IsFlyableArea", &[]),
        "advflyable" => yes(vm, "IsAdvancedFlyableArea", &[]),
        "swimming" => yes(vm, "IsSwimming", &[]),
        "indoors" => yes(vm, "IsIndoors", &[]),
        // Outdoors: not indoors where the engine has no answer of its own
        // (unverified).
        "outdoors" => match ask(vm, "IsOutdoors", &[]).first() {
            Some(v) => v.truthy(),
            None => !yes(vm, "IsIndoors", &[]),
        },
        "resting" => yes(vm, "IsResting", &[]),
        "channeling" => match text(vm, "UnitChannelInfo", &[player]) {
            Some(spell) => args.is_empty() || args.iter().any(|a| a.eq_ignore_ascii_case(&spell)),
            None => false,
        },
        "cursor" => !ask(vm, "GetCursorInfo", &[])
            .first()
            .is_none_or(|v| v.is_nil()),
        "button" | "btn" => {
            // The click's button; none (a key binding, a slash command):
            // button 1 (unverified).
            let clicked = text(vm, "GetMouseButtonClicked", &[]);
            let n = match clicked.as_deref() {
                None | Some("") | Some("LeftButton") => "1",
                Some("RightButton") => "2",
                Some("MiddleButton") => "3",
                Some("Button4") => "4",
                Some("Button5") => "5",
                Some(other) => other,
            }
            .to_string();
            args.is_empty()
                || args
                    .iter()
                    .any(|a| *a == n || clicked.as_deref().is_some_and(|c| a == c))
        }
        "equipped" | "worn" => args.iter().any(|a| {
            let kind = vm.str(a);
            yes(vm, "C_Item.IsEquippedItemType", &[kind])
        }),
        "known" => args.iter().any(|a| match a.parse::<f64>() {
            Ok(id) => yes(vm, "C_SpellBook.IsSpellKnown", &[Value::Num(id)]),
            Err(_) => crate::stats::spellbook_has_named(vm, a),
        }),
        // Talents the engine does not model here: never chosen (unverified).
        "talent" | "pvptalent" => false,
        _ => return None,
    })
}

/// `mod:<key>`: `shift`, `ctrl`, `alt` (either side, or `l`/`r` for one),
/// else a modified click's action (`SELFCAST`: `IsModifiedClick`).
fn modifier_held(vm: &mut Vm, key: &str) -> bool {
    let f = match key.to_ascii_lowercase().as_str() {
        "shift" => "IsShiftKeyDown",
        "ctrl" => "IsControlKeyDown",
        "alt" => "IsAltKeyDown",
        "lshift" => "IsLeftShiftKeyDown",
        "rshift" => "IsRightShiftKeyDown",
        "lctrl" => "IsLeftControlKeyDown",
        "rctrl" => "IsRightControlKeyDown",
        "lalt" => "IsLeftAltKeyDown",
        "ralt" => "IsRightAltKeyDown",
        _ => {
            let action = vm.str(&key.to_ascii_uppercase());
            return yes(vm, "IsModifiedClick", &[action]);
        }
    };
    yes(vm, f, &[])
}

/// `SecureCmdOptionParse(options)`: the first matching clause's value and
/// target (nil without one), or nothing when no clause matches. Recorded:
/// an empty string answers "" and nil (`SlashCommands.lua`'s
/// `/stopcasting` with no options must act), a number is read as its
/// text, nil errors with the usage; anything else errors as nil does
/// (unverified).
fn secure_cmd_option_parse(vm: &mut Vm, c: NativeCall) -> Ret {
    let options = match vm.hidden(c.arg(1)) {
        Value::Str(s) => vm.str_bytes(s).to_vec(),
        Value::Num(n) => wowlua_vm::number::fmt_num(n).into_bytes(),
        _ => return Err(vm.error("Usage: SecureCmdOptionParse(\"options\")")),
    };
    Ok(match parse(vm, &options) {
        Some((value, target)) => {
            let value = Value::Str(vm.heap.intern(&value));
            let target = match target {
                Some(t) => Value::Str(vm.heap.intern(&t)),
                None => Value::Nil,
            };
            vec![value, target]
        }
        None => vec![],
    })
}

/// `C_ActionBar.GetActionBarPage()`.
fn get_action_bar_page(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).bars.page)])
}

/// `C_ActionBar.SetActionBarPage(page)`: a new page fires
/// `ACTIONBAR_PAGE_CHANGED` (Blizzard's `ActionBarController` and the
/// state driver listen for it). Unverified: no arguments, pages beyond
/// `NUM_ACTIONBAR_PAGES` (6) and the same page again do nothing.
fn set_action_bar_page(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(page) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    let page = page.floor();
    if !(1.0..=6.0).contains(&page) || engine(vm).bars.page == page {
        return Ok(vec![]);
    }
    engine_mut(vm).bars.page = page;
    crate::widgets::fire_event(vm, "ACTIONBAR_PAGE_CHANGED", &[]);
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("SecureCmdOptionParse", secure_cmd_option_parse),
        ("C_ActionBar.GetActionBarPage", get_action_bar_page),
        ("C_ActionBar.SetActionBarPage", set_action_bar_page),
    ]
}
