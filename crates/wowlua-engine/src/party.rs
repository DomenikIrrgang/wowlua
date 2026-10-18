//! The player's party, as the client recorded a Horde paladin inviting
//! another character of the user's (`/stdtest engine scenario group`, a
//! ping near 0.45 s): `InviteUnit(name)`: "You have invited … to join your
//! group." a round trip later, the party formed a moment after with the
//! player alone and leading (its flags, factions, loot method and
//! threshold, `GROUP_JOINED`, the difficulty, the roster, the leader, the
//! loot, the roles; the raid targets after). `unit:joinGroup()`: the
//! invited player joins ("… joins the party.", `GROUP_FORMED`, the roster
//! twice, the social queue).
//!
//! `FollowUnit(unit)`: `AUTOFOLLOW_BEGIN(name)`. `InitiateTrade(unit)`:
//! "You have requested to trade with …." at once; `unit:openTrade()` (the
//! other side accepted): `TRADE_SHOW`, the money, the interaction, the
//! target's slots told; `CloseTrade()`: `TRADE_CLOSED`, the interaction's
//! hides, the server's "Trade canceled." after.
//!
//! Unverified: a party of more than two, leaving, a raid, roles, the loot
//! method's choice (group loot, uncommon: recorded), trading items or
//! money, the action bar's usability with a party (the paladin's
//! Blessing of Protection unusable alone in a party: recorded, not
//! modelled).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, PLAYER, UnitId};
use crate::{engine, engine_mut, widgets};

/// The party formed after the invitation's message (recorded).
const FORMED_AFTER: f64 = 0.047;
/// The raid targets told after the party formed (recorded).
const TARGETS_AFTER: f64 = 0.49;
/// A joined member's second roster, then the social queue (recorded).
const ROSTER_AFTER: f64 = 0.108;
const QUEUE_AFTER: f64 = 0.175;
/// The trade's target's slots told after it opened (recorded).
const TRADE_SLOTS_AFTER: f64 = 0.26;
/// `Enum.PlayerInteractionType.TradePartner`.
const TRADE: f64 = 1.0;
/// The trade's slots.
const TRADE_SLOTS: usize = 7;

#[derive(Default)]
pub struct Party {
    /// A party, maybe of the player alone (an invitation out).
    pub formed: bool,
    pub leader: bool,
    /// Trading with a unit.
    pub trading: Option<UnitId>,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn upval(c: &NativeCall) -> usize {
    match c.upvals.first() {
        Some(Value::Num(n)) => *n as usize,
        _ => 0,
    }
}

fn party(vm: &Vm) -> &Party {
    &engine(vm).world.party
}

fn party_mut(vm: &mut Vm) -> &mut Party {
    &mut engine_mut(vm).world.party
}

fn later(vm: &mut Vm, after: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, after, native);
}

/// A unit's full name ("Name Lastname" when it has one).
fn full_name(vm: &Vm, id: UnitId) -> String {
    let u = &engine(vm).units.list[id.0];
    match &u.last_name {
        Some(last) => format!("{} {last}", u.name),
        None => u.name.clone(),
    }
}

fn global(vm: &mut Vm, name: &str, default: &str) -> String {
    match vm.get_global(name) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => default.to_string(),
    }
}

fn system(vm: &mut Vm, text: &str) {
    crate::chat::game_message(vm, "CHAT_MSG_SYSTEM", text, None);
}

/// The unit a name or unit token names.
fn unit_named(vm: &mut Vm, v: Value) -> Option<UnitId> {
    let Value::Str(s) = vm.hidden(v) else {
        return None;
    };
    let text = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    if let Some(id) = units::resolve(&engine(vm).units, &text) {
        return Some(id);
    }
    let name = text.split('-').next().unwrap_or(&text).to_string();
    let list = &engine(vm).units.list;
    (1..list.len())
        .find(|&i| list[i].name == name || full_name(vm, UnitId(i)) == name)
        .map(UnitId)
}

/// The party's ID as `GROUP_JOINED` tells it (a session's; the harness
/// compares it as such).
fn party_guid(vm: &mut Vm) -> Value {
    vm.str("Party-0000-1-000000000001")
}

/// `C_PartyInfo.InviteUnit(name)`: the server's message a round trip
/// later, the party formed after.
fn invite_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = unit_named(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let delay = crate::clock::server_delay(vm);
    later(
        vm,
        delay,
        "partyInvited",
        invited,
        vec![Value::Num(id.0 as f64)],
    );
    Ok(vec![])
}

fn invited(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = UnitId(upval(&c));
    let format = global(
        vm,
        "ERR_INVITE_PLAYER_S",
        "You have invited %s to join your group.",
    );
    let name = full_name(vm, id);
    system(vm, &format.replacen("%s", &name, 1));
    if !party(vm).formed {
        later(vm, FORMED_AFTER, "partyFormed", formed, c.upvals.to_vec());
    }
    Ok(vec![])
}

/// The party of the player alone, leading.
fn formed(vm: &mut Vm, c: NativeCall) -> Ret {
    let invitee = UnitId(upval(&c));
    {
        let p = party_mut(vm);
        p.formed = true;
        p.leader = true;
    }
    units::fire_unit(vm, "PLAYER_FLAGS_CHANGED", PLAYER, &[]);
    fire(vm, "PARTY_LFG_RESTRICTED", &[]);
    units::fire_unit(vm, "UNIT_FACTION", PLAYER, &[]);
    units::fire_unit(vm, "UNIT_FACTION", invitee, &[]);
    fire(vm, "RAID_TARGET_UPDATE", &[]);
    let method = global(vm, "ERR_SET_LOOT_GROUP", "Looting set to Group Loot.");
    system(vm, &method);
    let threshold = global(vm, "ERR_SET_LOOT_THRESHOLD_S", "Loot threshold set to %s.");
    let uncommon = global(vm, "ITEM_QUALITY2_DESC", "Uncommon");
    system(vm, &threshold.replacen("%s", &uncommon, 1));
    let guid = party_guid(vm);
    fire(vm, "GROUP_JOINED", &[Value::Num(1.0), guid]);
    fire(vm, "CHANNEL_UI_UPDATE", &[]);
    fire(vm, "PLAYER_DIFFICULTY_CHANGED", &[]);
    fire(vm, "CHANNEL_UI_UPDATE", &[]);
    for _ in 0..3 {
        fire(vm, "PLAYER_DIFFICULTY_CHANGED", &[]);
    }
    fire(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    fire(vm, "PARTY_LEADER_CHANGED", &[]);
    fire(vm, "PARTY_LOOT_METHOD_CHANGED", &[]);
    fire(vm, "PLAYER_ROLES_ASSIGNED", &[]);
    later(vm, TARGETS_AFTER, "partyTargets", raid_targets, vec![]);
    Ok(vec![])
}

fn raid_targets(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "RAID_TARGET_UPDATE", &[]);
    fire(vm, "RAID_TARGET_UPDATE", &[]);
    Ok(vec![])
}

/// `unit:joinGroup()`: the invited player joins the party.
pub fn control_join_group(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    {
        let u = &mut engine_mut(vm).units;
        if !u.group.contains(&me) {
            u.group.push(me);
        }
    }
    party_mut(vm).formed = true;
    units::fire_unit(vm, "UNIT_FACTION", me, &[]);
    let format = global(vm, "ERR_JOINED_GROUP_S", "%s joins the party.");
    let name = full_name(vm, me);
    system(vm, &format.replacen("%s", &name, 1));
    let guid = party_guid(vm);
    fire(vm, "GROUP_FORMED", &[Value::Num(1.0), guid]);
    fire(vm, "CHANNEL_UI_UPDATE", &[]);
    roster(vm);
    later(vm, ROSTER_AFTER, "partyRoster", roster_again, vec![]);
    later(vm, QUEUE_AFTER, "partyQueue", social_queue, vec![]);
    Ok(vec![])
}

fn roster(vm: &mut Vm) {
    fire(vm, "PLAYER_DIFFICULTY_CHANGED", &[]);
    fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    fire(vm, "PLAYER_ROLES_ASSIGNED", &[]);
}

fn roster_again(vm: &mut Vm, _c: NativeCall) -> Ret {
    roster(vm);
    Ok(vec![])
}

fn social_queue(vm: &mut Vm, _c: NativeCall) -> Ret {
    let guid = party_guid(vm);
    fire(vm, "SOCIAL_QUEUE_UPDATE", &[guid, Value::Num(0.0)]);
    Ok(vec![])
}

/// Whether the player is grouped (a party of one while an invitation is
/// out, recorded).
pub fn grouped(vm: &Vm) -> bool {
    party(vm).formed || !engine(vm).units.group.is_empty()
}

fn unit_is_group_leader(vm: &mut Vm, c: NativeCall) -> Ret {
    let leader = units::unit_arg(vm, &c, 1) == Some(PLAYER) && party(vm).leader;
    Ok(vec![Value::Bool(leader)])
}

/// `FollowUnit(unit)`: `AUTOFOLLOW_BEGIN(name)` (recorded at the click).
fn follow_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = unit_named(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let name = full_name(vm, id);
    let n = vm.str(&name);
    fire(vm, "AUTOFOLLOW_BEGIN", &[n]);
    Ok(vec![])
}

/// `InitiateTrade(unit)`: the request told at once (recorded).
fn initiate_trade(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = unit_named(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let format = global(
        vm,
        "ERR_INITIATE_TRADE_S",
        "You have requested to trade with %s.",
    );
    let name = full_name(vm, id);
    system(vm, &format.replacen("%s", &name, 1));
    Ok(vec![])
}

/// `unit:openTrade()`: the other side accepted: the trade window.
pub fn control_open_trade(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    party_mut(vm).trading = Some(me);
    engine_mut(vm).units.npc = Some(me);
    fire(vm, "PLAYER_TRADE_MONEY", &[]);
    fire(vm, "TRADE_SHOW", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(TRADE)],
    );
    for _ in 0..3 {
        fire(vm, "PLAYER_TRADE_MONEY", &[]);
    }
    later(vm, TRADE_SLOTS_AFTER, "tradeSlots", trade_slots, vec![]);
    Ok(vec![])
}

/// The target's slots told (recorded: four times over, each after a
/// secure transfer's cancel).
fn trade_slots(vm: &mut Vm, _c: NativeCall) -> Ret {
    if party(vm).trading.is_none() {
        return Ok(vec![]);
    }
    fire(vm, "SECURE_TRANSFER_CANCEL", &[]);
    for _ in 0..4 {
        fire(vm, "SECURE_TRANSFER_CANCEL", &[]);
        for slot in 1..=TRADE_SLOTS {
            fire(vm, "TRADE_TARGET_ITEM_CHANGED", &[Value::Num(slot as f64)]);
        }
    }
    Ok(vec![])
}

/// `CloseTrade()`: closed; the server's cancel a round trip later.
fn close_trade(vm: &mut Vm, _c: NativeCall) -> Ret {
    if party_mut(vm).trading.take().is_none() {
        return Ok(vec![]);
    }
    engine_mut(vm).units.npc = None;
    fire(vm, "PLAYER_TRADE_MONEY", &[]);
    fire(vm, "TRADE_CLOSED", &[]);
    fire(vm, "SECURE_TRANSFER_CANCEL", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(TRADE)],
    );
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(0.0)],
    );
    let delay = crate::clock::server_delay(vm);
    later(vm, delay, "tradeCanceled", trade_canceled, vec![]);
    Ok(vec![])
}

fn trade_canceled(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "PLAYER_MONEY", &[]);
    fire(vm, "TRADE_REQUEST_CANCEL", &[]);
    let code = match vm.get_global("LE_GAME_ERR_TRADE_CANCELLED") {
        Value::Num(n) => n,
        _ => 0.0,
    };
    let text = global(vm, "ERR_TRADE_CANCELLED", "Trade canceled.");
    let t = vm.str(&text);
    fire(vm, "UI_INFO_MESSAGE", &[Value::Num(code), t]);
    Ok(vec![])
}

/// `GetTradeTargetItemInfo(i)`: an empty slot's (recorded: nil, nil, 0,
/// 0, nil, nil, 0).
fn empty_trade_slot(_vm: &mut Vm, _c: NativeCall) -> Ret {
    let zero = Value::Num(0.0);
    Ok(vec![
        Value::Nil,
        Value::Nil,
        zero,
        zero,
        Value::Nil,
        Value::Nil,
        zero,
    ])
}

/// `GetTradePlayerItemInfo(i)`: an empty slot's, its item ID last (the
/// eighth: Blizzard's trade frame reads it; unverified).
fn empty_player_trade_slot(_vm: &mut Vm, _c: NativeCall) -> Ret {
    let (zero, no) = (Value::Num(0.0), Value::Bool(false));
    Ok(vec![
        Value::Nil,
        Value::Nil,
        zero,
        zero,
        Value::Nil,
        no,
        no,
        zero,
    ])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_PartyInfo.InviteUnit", invite_unit),
        ("InviteUnit", invite_unit),
        ("UnitIsGroupLeader", unit_is_group_leader),
        ("FollowUnit", follow_unit),
        ("InitiateTrade", initiate_trade),
        ("CloseTrade", close_trade),
        ("GetTradeTargetItemInfo", empty_trade_slot),
        ("GetTradePlayerItemInfo", empty_player_trade_slot),
        ("GetTradeTargetItemLink", nothing),
        ("GetTradePlayerItemLink", nothing),
        ("GetPlayerTradeMoney", zero),
        ("GetTargetTradeMoney", zero),
    ]
}
