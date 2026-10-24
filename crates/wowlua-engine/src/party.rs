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
//! money. The action bar's usability with a party: the paladin's Blessing
//! of Protection unusable alone in a party with a soft friend outside it
//! (recorded; `actions.rs`).

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

/// The unit a name or unit token names: another's name first, exactly as
/// written (a recording's scrubbed names are the player's own, "Player":
/// typed, they name the other), then a token.
fn unit_named(vm: &mut Vm, v: Value) -> Option<UnitId> {
    let Value::Str(s) = vm.hidden(v) else {
        return None;
    };
    let text = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let name = text.split('-').next().unwrap_or(&text).to_string();
    let list = &engine(vm).units.list;
    let named = (1..list.len())
        .find(|&i| list[i].name == name || full_name(vm, UnitId(i)) == name)
        .map(UnitId);
    named.or_else(|| units::resolve(&engine(vm).units, &text))
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
    // The actions on a group (recorded: Blessing of Protection's re-told
    // here, the bars' update after the difficulty's).
    crate::actions::usability_changed(vm);
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
    // Its faction by its group token alone (recorded: party1's, not the
    // soft friend's it also was; unverified: other tokens).
    let tokens: Vec<String> = units::tokens_of(&engine(vm).units, me)
        .into_iter()
        .filter(|t| t.starts_with("party") || t.starts_with("raid"))
        .collect();
    for t in tokens {
        let t = vm.str(&t);
        fire(vm, "UNIT_FACTION", &[t]);
    }
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

/// `C_PartyInfo.LeaveParty()`: the player leaves the group a round trip
/// later ([`leave`]); alone, nothing.
fn leave_party(vm: &mut Vm, _c: NativeCall) -> Ret {
    if grouped(vm) {
        let delay = crate::clock::server_delay(vm);
        later(vm, delay, "partyLeft", left, vec![]);
    }
    Ok(vec![])
}

fn left(vm: &mut Vm, _c: NativeCall) -> Ret {
    leave(vm, "ERR_LEFT_GROUP_YOU", "You leave the group.");
    Ok(vec![])
}

/// The player's group is gone (left, or its last member removed): the
/// server's message (`message`, a global string, else `default`),
/// `GROUP_LEFT`, the roster and the leader (unverified: the client's
/// events and their order).
pub fn leave(vm: &mut Vm, message: &str, default: &str) {
    if !grouped(vm) {
        return;
    }
    {
        let u = &mut engine_mut(vm).units;
        for id in std::mem::take(&mut u.group) {
            u.list[id.0].subgroup = None;
        }
        u.raid = false;
    }
    {
        let p = party_mut(vm);
        p.formed = false;
        p.leader = false;
    }
    let text = global(vm, message, default);
    system(vm, &text);
    let guid = party_guid(vm);
    fire(vm, "GROUP_LEFT", &[Value::Num(1.0), guid]);
    fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    fire(vm, "PARTY_LEADER_CHANGED", &[]);
}

/// `mock.group:leave()`: the player leaves the group now.
pub fn control_leave_group(vm: &mut Vm, _c: NativeCall) -> Ret {
    leave(vm, "ERR_LEFT_GROUP_YOU", "You leave the group.");
    Ok(vec![])
}

/// The player leading a group of its own making (`mock.addToGroup`).
pub fn lead(vm: &mut Vm) {
    let p = party_mut(vm);
    p.formed = true;
    p.leader = true;
}

/// `GetRaidRosterInfo(index)`: name, rank, subgroup, level, class, file
/// name, zone, online, dead, role, master looter, combat role; in a party
/// too (recorded): the player first, then the members in joining order;
/// the full name ("Name Lastname" on Forever); the leader's rank 2, the
/// others' 0; each member's zone (out of sight: its own); an offline
/// member's level 0 and zone "Offline"; dead a ghost too; the chosen role
/// last. An index past the members, or 0: a placeholder (recorded: `nil,
/// 0, 1, 1, nil × 6, false, "NONE"`). Unverified: solo; assistants; a
/// cross-realm member's realm.
fn get_raid_roster_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let placeholder = |vm: &mut Vm| {
        let mut out = vec![
            Value::Nil,
            Value::Num(0.0),
            Value::Num(1.0),
            Value::Num(1.0),
        ];
        out.extend([Value::Nil; 6]);
        out.push(Value::Bool(false));
        out.push(vm.str("NONE"));
        out
    };
    let index = match c.arg(1) {
        Value::Num(n) if n >= 1.0 && n.fract() == 0.0 => n as usize,
        _ => return Ok(placeholder(vm)),
    };
    let unit = {
        let u = &engine(vm).units;
        if !grouped(vm) {
            None
        } else if index == 1 {
            Some(PLAYER)
        } else {
            u.group.get(index - 2).copied()
        }
    };
    let Some(unit) = unit else {
        return Ok(placeholder(vm));
    };
    let rank = if unit == PLAYER && party(vm).leader {
        2.0
    } else {
        0.0
    };
    let subgroup = {
        let u = &engine(vm).units;
        if u.raid {
            units::subgroup(u, unit)
        } else {
            1.0
        }
    };
    let here = engine(vm).world.character.position.zone.clone();
    let offline = global(vm, "PLAYER_OFFLINE", "Offline");
    let name = full_name(vm, unit);
    let (level, class_name, class, dead, zone, role, online) = {
        let u = &engine(vm).units.list[unit.0];
        (
            if u.online { u.level } else { 0.0 },
            if u.player {
                u.class_name.clone()
            } else {
                u.name.clone()
            },
            u.class.clone(),
            u.dead || u.ghost,
            if u.online {
                u.zone.clone().unwrap_or(here)
            } else {
                offline
            },
            u.role.clone().unwrap_or_else(|| "NONE".to_string()),
            u.online,
        )
    };
    Ok(vec![
        vm.str(&name),
        Value::Num(rank),
        Value::Num(subgroup),
        Value::Num(level),
        vm.str(&class_name),
        vm.str(&class),
        vm.str(&zone),
        Value::Bool(online),
        Value::Bool(dead),
        Value::Nil,
        Value::Bool(false),
        vm.str(&role),
    ])
}

/// `C_PartyInfo.ConvertToParty()`: the leader's raid of five or fewer a
/// party again (`GROUP_ROSTER_UPDATE`; unverified: its other events).
fn convert_to_party(vm: &mut Vm, _c: NativeCall) -> Ret {
    let small = engine(vm).units.group.len() < 5;
    if party(vm).leader && engine(vm).units.raid && small {
        engine_mut(vm).units.raid = false;
        fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    }
    Ok(vec![])
}

/// `C_PartyInfo.GetLootMethod()`: group loot (recorded in a party and a
/// raid, as the party was formed with; unverified: solo, other methods).
fn get_loot_method(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(3.0), Value::Nil, Value::Nil])
}

/// `C_PartyInfo.IsPartyFull()`: a party of five (recorded: not with two;
/// unverified: full, a raid).
fn is_party_full(vm: &mut Vm, _c: NativeCall) -> Ret {
    let u = &engine(vm).units;
    Ok(vec![Value::Bool(!u.raid && u.group.len() >= 4)])
}

/// An assignment's name upper-cased, if it is one (`MAINTANK`,
/// `MAINASSIST`).
pub(crate) fn assignment(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => {
            let a = String::from_utf8_lossy(vm.str_bytes(s)).to_ascii_uppercase();
            (a == "MAINTANK" || a == "MAINASSIST").then_some(a)
        }
        _ => None,
    }
}

/// `GetPartyAssignment(assignment, unit [, exactMatch])`: whether the unit
/// has the assignment (`SetPartyAssignment`); any other assignment the
/// recorded error.
fn get_party_assignment(vm: &mut Vm, c: NativeCall) -> Ret {
    // Asked with nothing, its usage (the full probe: the text up to its
    // first comma blanked, `...` for it).
    if c.arg(1).is_nil() {
        return Err(vm.error("Usage: GetPartyAssignment(...,\"raidmember\"] [,exactMatch])"));
    }
    let Some(a) = assignment(vm, c.arg(1)) else {
        return Err(vm.error("GetPartyAssignment(): Invalid party assignment"));
    };
    let unit = crate::units::unit_arg(vm, &c, 2);
    let has = unit.is_some_and(|u| {
        engine(vm)
            .units
            .assignments
            .iter()
            .any(|(x, w)| *x == a && *w == u)
    });
    Ok(vec![Value::Bool(has)])
}

/// `C_PartyInfo.ConvertToRaid()`: the leader's party a raid
/// (`GROUP_ROSTER_UPDATE`; unverified: its other events and messages).
fn convert_to_raid(vm: &mut Vm, _c: NativeCall) -> Ret {
    let leading = party(vm).leader && !engine(vm).units.group.is_empty();
    if leading && !engine(vm).units.raid {
        engine_mut(vm).units.raid = true;
        fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    }
    Ok(vec![])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_PartyInfo.InviteUnit", invite_unit),
        ("InviteUnit", invite_unit),
        ("UnitIsGroupLeader", unit_is_group_leader),
        ("GetRaidRosterInfo", get_raid_roster_info),
        ("C_PartyInfo.ConvertToRaid", convert_to_raid),
        ("C_PartyInfo.ConvertToParty", convert_to_party),
        ("C_PartyInfo.LeaveParty", leave_party),
        ("C_PartyInfo.GetLootMethod", get_loot_method),
        ("C_PartyInfo.IsPartyFull", is_party_full),
        ("C_PartyInfo.CanFormCrossFactionParties", no),
        ("GetPartyAssignment", get_party_assignment),
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
