//! The third round's leftovers: functions with state the engine models
//! that had no behaviour (decision T295).
//!
//! - `StartAttack()` and `AttackTarget()`: the player's auto attack on a
//!   target that can be attacked (`PLAYER_ENTER_COMBAT`).
//! - `SetPartyAssignment(assignment, unit)` and
//!   `ClearPartyAssignment(assignment, unit)`: the raid's main tanks and
//!   assists, as `GetPartyAssignment` answers (`GROUP_ROSTER_UPDATE`).
//! - `SendSystemMessage(text)`: a system line (`CHAT_MSG_SYSTEM`), as the
//!   server's.
//! - `Sign(x)`: -1, 0 or 1.
//! - Money on the cursor: `PickupPlayerMoney(copper)` holds that much of
//!   the player's money, `DropCursorMoney()` (and `ClearCursor()`) lets it
//!   go, `GetCursorMoney()` answers it (`CURSOR_CHANGED` with the cursor
//!   type 2, `Enum.UICursorType.Money`).
//! - `TargetLastEnemy()` and `TargetLastFriend()`: the last hostile or
//!   friendly target before the current one.
//! - `secretunwrap(...)`: the values secrets hide (the documentation's
//!   `SecretArguments`: tainted code may not pass one).
//!
//! Unverified, none recorded: the events of an assignment, of money on
//! the cursor and of a system line sent from Lua; which units
//! `TargetLastEnemy` remembers (the last one targeted here).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, Reaction, UnitId};
use crate::{engine, engine_mut, widgets};

/// `Enum.UICursorType.Money`.
const MONEY_CURSOR: f64 = 2.0;

fn attack(vm: &mut Vm, _c: NativeCall) -> Ret {
    let target = engine(vm).units.target;
    let can = target.is_some_and(|t| {
        let u = &engine(vm).units.list[t.0];
        u.reaction != Reaction::Friendly && !u.dead
    });
    if can {
        units::start_auto_attack(vm);
    }
    Ok(vec![])
}

fn set_party_assignment(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(a), Some(u)) = (
        crate::party::assignment(vm, c.arg(1)),
        units::unit_arg(vm, &c, 2),
    ) else {
        return Ok(vec![]);
    };
    let list = &mut engine_mut(vm).units.assignments;
    if !list.iter().any(|(x, w)| *x == a && *w == u) {
        list.push((a, u));
        widgets::fire_event(vm, "GROUP_ROSTER_UPDATE", &[]);
    }
    Ok(vec![])
}

fn clear_party_assignment(vm: &mut Vm, c: NativeCall) -> Ret {
    let a = crate::party::assignment(vm, c.arg(1));
    let u: Option<UnitId> = units::unit_arg(vm, &c, 2);
    let list = &mut engine_mut(vm).units.assignments;
    let before = list.len();
    list.retain(|(x, w)| !(a.as_ref().is_none_or(|a| a == x) && u.is_none_or(|u| u == *w)));
    if list.len() != before {
        widgets::fire_event(vm, "GROUP_ROSTER_UPDATE", &[]);
    }
    Ok(vec![])
}

fn send_system_message(vm: &mut Vm, c: NativeCall) -> Ret {
    crate::chat::system_message_control(vm, c)
}

fn sign(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = vm.check_number(&c, 1)?;
    let s = if n > 0.0 {
        1.0
    } else if n < 0.0 {
        -1.0
    } else {
        0.0
    };
    Ok(vec![Value::Num(s)])
}

fn cursor_changed(vm: &mut Vm, cleared: bool, new: f64, old: f64) {
    widgets::fire_event(
        vm,
        "CURSOR_CHANGED",
        &[
            Value::Bool(cleared),
            Value::Num(new),
            Value::Num(old),
            Value::Num(0.0),
        ],
    );
}

fn pickup_player_money(vm: &mut Vm, c: NativeCall) -> Ret {
    let copper = vm.check_number(&c, 1)?.floor();
    let character = &engine(vm).world.character;
    let empty = matches!(character.cursor, crate::cursor::Cursor::Empty);
    if copper > 0.0 && copper <= character.money && empty && character.cursor_money == 0.0 {
        engine_mut(vm).world.character.cursor_money = copper;
        cursor_changed(vm, false, MONEY_CURSOR, 0.0);
    }
    Ok(vec![])
}

/// The money on the cursor let go, if any (`DropCursorMoney`,
/// `ClearCursor`): whether there was.
pub fn drop_money(vm: &mut Vm) -> bool {
    let held = std::mem::take(&mut engine_mut(vm).world.character.cursor_money);
    if held > 0.0 {
        cursor_changed(vm, true, 0.0, MONEY_CURSOR);
    }
    held > 0.0
}

fn drop_cursor_money(vm: &mut Vm, _c: NativeCall) -> Ret {
    drop_money(vm);
    Ok(vec![])
}

fn get_cursor_money(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(engine(vm).world.character.cursor_money)])
}

fn retarget(vm: &mut Vm, last: Option<UnitId>) {
    if let Some(l) = last
        && engine(vm).units.list[l.0].alive_in_world
    {
        units::set_player_target(vm, Some(l));
    }
}

fn target_last_enemy(vm: &mut Vm, _c: NativeCall) -> Ret {
    let last = engine(vm).units.last_enemy;
    retarget(vm, last);
    Ok(vec![])
}

fn target_last_friend(vm: &mut Vm, _c: NativeCall) -> Ret {
    let last = engine(vm).units.last_friend;
    retarget(vm, last);
    Ok(vec![])
}

fn secretunwrap(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(c.args.iter().map(|&v| vm.hidden(v)).collect())
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("StartAttack", attack),
        ("AttackTarget", attack),
        ("SetPartyAssignment", set_party_assignment),
        ("ClearPartyAssignment", clear_party_assignment),
        ("SendSystemMessage", send_system_message),
        ("Sign", sign),
        ("PickupPlayerMoney", pickup_player_money),
        ("DropCursorMoney", drop_cursor_money),
        ("GetCursorMoney", get_cursor_money),
        ("TargetLastEnemy", target_last_enemy),
        ("TargetLastFriend", target_last_friend),
        ("secretunwrap", secretunwrap),
    ]
}
