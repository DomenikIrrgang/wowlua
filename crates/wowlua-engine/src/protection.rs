//! Protected actions (spec emulator §17.4, decision T197): what tainted code
//! may not do. A protected function called from tainted execution does
//! nothing and fires `ADDON_ACTION_FORBIDDEN`; a protected method of a
//! protected frame does the same in combat lockdown with
//! `ADDON_ACTION_BLOCKED`. Both payloads are the tainting addon's name and
//! the action as Lua's debug information names the call (`UseAction()`
//! from `UseAction(…)`, `UNKNOWN()` through `pcall`; `/stdtest taint` and
//! `engine`); the call returns its documented results' empty values
//! (`false` from `ClearTarget`). Each refusal is kept with its taint
//! report, for the test it fails.
//!
//! Protected methods are the documentation's (`IsProtectedFunction`); the
//! documentation does not mark global functions, so [`PROTECTED`] lists the
//! ones known to be protected. `/stdtest taint` recorded a global's payload
//! (`"StdTest", "UseAction()"`); unverified until `/stdtest engine
//! protected` and `blocked` run: the list, a method's payload, that a
//! refusal returns nothing without an error and comes before the argument
//! checks, and that the event reaches handlers before the call returns.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::widgets::{self, WidgetId};
use crate::{engine, engine_mut};

/// Global functions only Blizzard's code may call (not in the
/// documentation's flags). `/stdtest engine protected` saw TargetUnit,
/// ClearTarget, FocusUnit, AssistUnit, SpellStopCasting,
/// SpellStopTargeting, MoveForwardStop, TurnLeftStop, StrafeLeftStop and
/// PetFollow forbidden, ClearFocus, TargetLastTarget and StopAttack not;
/// CastSpellByName with an unknown spell fired nothing (kept, unverified),
/// and the rest are unverified. Added by their probed neighbours (the
/// movement, pet and targeting commands refused) or the documentation's
/// `HasRestrictions` (TargetToggle, SitStandOrDescendStart, ToggleSit),
/// all used by Blizzard's secure slash commands, secure templates or key
/// bindings only: the other movement and pitch bindings, the pet's modes,
/// moving and dismissing, the nearest-target variants, toys, totems, quest
/// items, `SpellTargetItem`, `ReleaseAction`, `StopMacro` (unverified; not
/// every secure slash command is protected: TargetLastTarget is not).
/// SuuUI's probes saw AcceptBattlefieldPort and Logout (popups),
/// GuildControlSetRank, `C_GuildInfo.SetMOTD` and `SetInfoText`
/// (guildControl) and `C_GuildInfo.SetNote` (guildNotes, from a macro)
/// forbidden; GuildControlSaveRank not.
pub const PROTECTED: &[&str] = &[
    "AcceptBattlefieldPort",
    "AscendStop",
    "AssistUnit",
    "AttackTarget",
    "C_Container.UseContainerItem",
    "C_GuildInfo.SetInfoText",
    "C_GuildInfo.SetMOTD",
    "C_GuildInfo.SetNote",
    "CameraOrSelectOrMoveStart",
    "CameraOrSelectOrMoveStop",
    "CancelShapeshiftForm",
    "CastPetAction",
    "CastShapeshiftForm",
    "CastSpell",
    "CastSpellByID",
    "CastSpellByName",
    "ClearTarget",
    "DescendStop",
    "DestroyTotem",
    "FocusUnit",
    "GuildControlSetRank",
    "JumpOrAscendStart",
    "Logout",
    "MoveAndSteerStart",
    "MoveAndSteerStop",
    "MoveBackwardStart",
    "MoveBackwardStop",
    "MoveForwardStart",
    "MoveForwardStop",
    "PetAggressiveMode",
    "PetAttack",
    "PetDefensiveAssistMode",
    "PetDefensiveMode",
    "PetDismiss",
    "PetFollow",
    "PetMoveTo",
    "PetPassiveMode",
    "PetWait",
    "PitchDownStart",
    "PitchDownStop",
    "PitchUpStart",
    "PitchUpStop",
    "ReleaseAction",
    "RunMacro",
    "RunMacroText",
    "SitStandOrDescendStart",
    "SpellStopCasting",
    "SpellStopTargeting",
    "SpellTargetItem",
    "SpellTargetUnit",
    "StartAttack",
    "StartAutoRun",
    "StopAutoRun",
    "StopMacro",
    "StrafeLeftStart",
    "StrafeLeftStop",
    "StrafeRightStart",
    "StrafeRightStop",
    "TargetLastEnemy",
    "TargetLastFriend",
    "TargetNearest",
    "TargetNearestEnemy",
    "TargetNearestEnemyPlayer",
    "TargetNearestFriend",
    "TargetNearestFriendPlayer",
    "TargetNearestPartyMember",
    "TargetNearestRaidMember",
    "TargetToggle",
    "TargetUnit",
    "ToggleAutoRun",
    "ToggleRun",
    "ToggleSit",
    "TurnLeftStart",
    "TurnLeftStop",
    "TurnOrActionStart",
    "TurnOrActionStop",
    "TurnRightStart",
    "TurnRightStop",
    "UseAction",
    "UseContainerItem",
    "UseInventoryItem",
    "UseItemByName",
    "UseQuestLogSpecialItem",
    "UseToy",
    "UseToyByName",
];

/// Functions tainted code may call only while the user acts (a click or a
/// key): otherwise refused with `ADDON_ACTION_BLOCKED`, doing nothing.
/// SuuUI probe popups (tries: each called from a timer out of combat):
/// these four blocked (`DeleteCursorItem` with an empty cursor too);
/// AcceptGroup, DeclineGroup, AcceptResurrect, DeclineResurrect,
/// `C_SummonInfo.ConfirmSummon`, `CancelSummon`, AcceptDuel, CancelDuel,
/// ConfirmReadyCheck, the area spirit heal's, RetrieveCorpse, RepopMe,
/// CancelTrade and RejectProposal ran. Unverified: refused during a
/// user's action too, or for Blizzard's code.
pub const HARDWARE: &[&str] = &[
    "AcceptProposal",
    "AcceptTrade",
    "CancelLogout",
    "DeleteCursorItem",
];

/// Refuses a [`HARDWARE`] function called by tainted code while the user
/// does nothing; true if refused.
pub fn refuses_without_user(vm: &mut Vm, name: &str) -> bool {
    if !HARDWARE.contains(&name) || !tainted(vm) || engine(vm).world.input.user_actions > 0 {
        return false;
    }
    let action = action_name(vm);
    refuse(vm, "ADDON_ACTION_BLOCKED", action);
    true
}

/// Refuses a function blocked for tainted code in combat lockdown
/// (`ADDON_ACTION_BLOCKED`); true if refused. Found in game:
/// `C_SpellBook.PickupSpellBookItem` from SuuUI's spellbook in combat,
/// "Blocked in combat: PickupSpellBookItem()" (decision T253).
pub fn refuses_in_combat(vm: &mut Vm) -> bool {
    if !(engine(vm).units.lockdown && tainted(vm)) {
        return false;
    }
    let action = action_name(vm);
    refuse(vm, "ADDON_ACTION_BLOCKED", action);
    true
}

/// A [`HARDWARE`] function the documentation lacks: refused without the
/// user's action; otherwise its behaviour.
pub fn hardware_global(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Str(name)) = c.upvals.first().copied() else {
        return Err(vm.error("a function without its name"));
    };
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    if refuses_without_user(vm, &name) {
        return Ok(vec![]);
    }
    match engine(vm).behaviour(&name) {
        Some(b) => b(vm, c),
        None => Ok(vec![]),
    }
}

/// A refused protected action.
pub struct Refusal {
    pub event: &'static str,
    pub addon: String,
    pub action: String,
    /// The taint chain from the refused call back to the addon's code.
    pub report: String,
}

/// Whether the running code is tainted.
pub fn tainted(vm: &Vm) -> bool {
    !vm.taint().stack.is_secure()
}

/// Whether the function calling the engine is Blizzard's (secure) code,
/// tainted execution or not: its gaps get empty values, not errors.
pub fn blizzard_caller(vm: &Vm) -> bool {
    // Called by a C function (`securecallfunction(DropCursorMoney)`): by
    // whoever runs, untainted execution being Blizzard's.
    vm.caller_is_secure(1) || (vm.caller_source(1).is_none() && !tainted(vm))
}

/// Whether a global function is protected.
pub fn is_protected(name: &str, doc: Option<&serde_json::Value>) -> bool {
    PROTECTED.contains(&name)
        || doc
            .and_then(|d| d.get("IsProtectedFunction"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

/// The refused action's name: how the call site names the function, else
/// `UNKNOWN`, with `()`.
pub fn action_name(vm: &Vm) -> String {
    format!("{}()", vm.called_as().unwrap_or_else(|| "UNKNOWN".into()))
}

/// Refuses `action`: keeps the refusal and fires `event` with the
/// tainting addon and the action.
pub fn refuse(vm: &mut Vm, event: &'static str, action: String) {
    let addon = vm.taint_name(vm.taint().stack).unwrap_or_default();
    let report = vm.execution_report();
    engine_mut(vm).refusals.push(Refusal {
        event,
        addon: addon.clone(),
        action: action.clone(),
        report,
    });
    let a = vm.str(&addon);
    let f = vm.str(&action);
    widgets::fire_event(vm, event, &[a, f]);
}

/// A protected method of `id` refused: in combat lockdown, on a protected
/// widget, for tainted code. The action names the widget by its name, or
/// its type when it has none (unverified).
pub fn refuses_method(vm: &mut Vm, id: WidgetId, f: &crate::api::Function) -> bool {
    // `SetAttribute` too (`/stdtest engine blocked`).
    let protected_method = f
        .doc
        .get("IsProtectedFunction")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        || f.short == "SetAttribute";
    let e = engine(vm);
    if !(protected_method && e.units.lockdown && tainted(vm) && widgets::protected(e, id)) {
        return false;
    }
    let action = action_name(vm);
    refuse(vm, "ADDON_ACTION_BLOCKED", action);
    true
}

/// A protected global function the documentation lacks: refused for
/// tainted code; otherwise its behaviour, or not mocked.
pub fn protected_global(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(Value::Str(name)) = c.upvals.first().copied() else {
        return Err(vm.error("a protected function without its name"));
    };
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    // A spell the player does not have fails before the protection
    // (`/stdtest engine protected`: no event for an unknown spell).
    if name == "CastSpellByName" {
        let known = match c.arg(1) {
            Value::Str(s) => {
                let spell = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
                let spells: Vec<wowlua_vm::TableRef> =
                    engine(vm).world.data.spells.values().copied().collect();
                spells.into_iter().any(|t| {
                    let k = vm.str("name");
                    matches!(vm.heap.table(t).get(k), Value::Str(n) if vm.str_bytes(n) == spell.as_bytes())
                })
            }
            _ => false,
        };
        if !known {
            return Ok(vec![]);
        }
    }
    if tainted(vm) {
        let action = action_name(vm);
        refuse(vm, "ADDON_ACTION_FORBIDDEN", action);
        // What the recording saw these return when refused.
        if matches!(name.as_str(), "SpellStopCasting" | "SpellStopTargeting") {
            return Ok(vec![Value::Bool(false)]);
        }
        return Ok(vec![]);
    }
    match engine(vm).behaviour(&name) {
        Some(b) => b(vm, c),
        // Secure code's protected call without behaviour does nothing.
        None => Ok(vec![]),
    }
}

/// `refusals()`: the refused actions so far, as tables (`event`, `addon`,
/// `action`, `report`).
pub fn refusals_control(vm: &mut Vm, _c: NativeCall) -> Ret {
    let list: Vec<[String; 4]> = engine(vm)
        .refusals
        .iter()
        .map(|r| {
            [
                r.event.to_string(),
                r.addon.clone(),
                r.action.clone(),
                r.report.clone(),
            ]
        })
        .collect();
    let out = vm.new_table();
    for (i, fields) in list.into_iter().enumerate() {
        let t = vm.new_table();
        for (key, value) in ["event", "addon", "action", "report"].iter().zip(fields) {
            let k = vm.str(key);
            let v = vm.str(&value);
            let _ = vm.heap.table_mut(t).set(k, v);
        }
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num(i as f64 + 1.0), Value::Table(t));
    }
    Ok(vec![Value::Table(out)])
}

// ---------------------------------------------------------- forbidden aspects

/// `Enum.ForbiddenAspect` (camelot's `ForbiddenAspectConstantsDocumentation`).
const ASPECTS: &[(&str, u32)] = &[
    ("SetToDefaults", 1),
    ("ScriptBindings", 2),
    ("UntrustedScriptExecution", 4),
    ("UntrustedLayoutScriptExecution", 8),
    ("EventRegistrations", 16),
    ("AlwaysPropagateInput", 32),
    ("ScriptedInput", 64),
    ("QueryFocus", 128),
    ("ChangeAnimationTarget", 256),
    ("RemoveSecretAspects", 512),
    ("ChangeParent", 1024),
    ("SetTexture", 2048),
    ("QueryRotation", 4096),
    ("QueryAnimationProgress", 8192),
    ("AddAnimations", 16384),
];

/// "Restricts execution of layout scripts (eg. OnSizeChanged) for this
/// object" (the documentation).
pub const UNTRUSTED_LAYOUT_SCRIPTS: u32 = 8;

/// A `<ForbiddenAspect aspect="…"/>`'s flag (none for an unknown name).
pub fn aspect_flag(name: &str) -> u32 {
    ASPECTS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map_or(0, |(_, f)| *f)
}

/// Adds forbidden aspects to a widget.
pub fn add_aspects(vm: &mut Vm, id: WidgetId, flags: u32) {
    engine_mut(vm).world.widgets[id.0].forbidden_aspects |= flags;
}

/// `AddForbiddenAspects(aspects)`: from secure code only. Blizzard's
/// `ForbiddenAspectTemplates.xml`: "we don't allow ... tainted calls to
/// AddForbiddenAspects on frames created by tainted code" (ElvUI's aura
/// containers inherit `DisableUntrustedLayoutScriptsTemplate` for it).
/// Unverified: a tainted call does nothing and raises no error (the user's
/// client showed none for ElvUI's containers, whose `AddAuraGroup` calls
/// it tainted); a tainted call on a secure frame's untested.
pub fn add_forbidden_aspects(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    if tainted(vm) {
        return Ok(vec![]);
    }
    if let Value::Num(n) = c.arg(2) {
        add_aspects(vm, id, n as u32);
    }
    Ok(vec![])
}

/// Whether `v` is a widget whose untrusted layout scripts are forbidden
/// (its own aspect; unverified and not kept: the documented propagation
/// to its children and the frames anchored to it).
pub fn untrusted_layout_forbidden(vm: &Vm, v: Value) -> bool {
    widgets::widget_of(vm, v).is_some_and(|id| {
        engine(vm).world.widgets[id.0].forbidden_aspects & UNTRUSTED_LAYOUT_SCRIPTS != 0
    })
}
