//! The client's actions on state the engine already models, done through
//! the paths the engine's own recorded flows take (their events and
//! timing): the cursor's item put into a bag, equipped or sold
//! (`cursor.rs`, `inventory.rs`, `merchant.rs`); a letter's money taken,
//! a letter deleted, returned or copied (`mail.rs`); the quest log's
//! headers collapsed and expanded (`quests.rs`'s log); raid members moved
//! between subgroups (`party.rs`, `units.rs`); the inspected player
//! forgotten (`inspect.rs`).
//!
//! Most of these were not recorded: each says where its rule comes from,
//! and is unverified unless it names its recording (`mail2`'s money taken,
//! `repair`'s repair cursor, `questlog`'s headers). Protected functions (`SetPartyAssignment`,
//! warcraft.wiki.gg's `protected` tag) and the cursor's money (the
//! documentation's `HasRestrictions`) are not here.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::cursor::{self, Cursor, Source};
use crate::stats::{Answer, Entry};
use crate::units::{self, PLAYER, UnitId};
use crate::{character, engine, engine_mut, widgets};

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// A number argument (a secret one unwrapped).
fn num_arg(vm: &Vm, c: &NativeCall, n: usize) -> Option<f64> {
    num(vm.hidden(c.arg(n)))
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn later(vm: &mut Vm, after: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, after, native);
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

// ------------------------------------------------------------ the cursor

/// Whether the cursor holds an item.
fn holds_item(vm: &Vm) -> bool {
    matches!(engine(vm).world.character.cursor, Cursor::Item(..))
}

/// The bag an inventory slot holds: the bags' 31 to 35, the bank's 64 to
/// 69 (the inverse of `C_Container.ContainerIDToInventoryID`, recorded
/// in `queries.rs`).
fn bag_of_inventory_id(inv: f64) -> Option<i64> {
    let inv = inv as i64;
    match inv {
        31..=35 => Some(inv - 30),
        64..=69 => Some(inv - 58),
        _ => None,
    }
}

/// The cursor's item put into `bag`'s first free slot, through the
/// cursor's own move (`cursor::pickup_item_at`: a bag item let go at once
/// and moved a round trip later, a worn one the server's swap); back where
/// it was when it came from that bag; nothing when the bag is full or is
/// the closed bank's (unverified: the client's "That bag is full." not
/// told).
fn put_into_bag(vm: &mut Vm, bag: i64) {
    if !character::carried(bag) && !crate::bank::is_open(vm) {
        return;
    }
    let Some((_, from)) = cursor::held_item(vm) else {
        return;
    };
    if let Source::Bag(fb, _) = from
        && fb == bag
    {
        cursor::pickup_item_at(vm, from);
        return;
    }
    let free = engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .and_then(|b| (1..=b.slots).find(|s| !b.items.contains_key(s)));
    if let Some(slot) = free {
        cursor::pickup_item_at(vm, Source::Bag(bag, slot));
    }
}

/// `PutItemInBag(inventoryID)`: the cursor's item into the bag in that
/// slot (`bag + CONTAINER_BAG_OFFSET`, Blizzard's equipment manager);
/// whether the cursor held an item (warcraft.wiki.gg's `hadItem`;
/// Blizzard's bag buttons open the bag when it did not).
fn put_item_in_bag(vm: &mut Vm, c: NativeCall) -> Ret {
    let had = holds_item(vm);
    if had && let Some(bag) = num_arg(vm, &c, 1).and_then(bag_of_inventory_id) {
        put_into_bag(vm, bag);
    }
    Ok(vec![Value::Bool(had)])
}

/// `PutItemInBackpack()`: the cursor's item into the backpack; whether the
/// cursor held one (as `PutItemInBag`: Blizzard's backpack button reads it
/// so; warcraft.wiki.gg lists no return).
fn put_item_in_backpack(vm: &mut Vm, _c: NativeCall) -> Ret {
    let had = holds_item(vm);
    if had {
        put_into_bag(vm, 0);
    }
    Ok(vec![Value::Bool(had)])
}

/// `PickupBagFromSlot(inventoryID)`: the bag worn in that slot picked up
/// as `PickupInventoryItem` picks up worn gear, or put back when it is
/// the one held; nothing while holding something else or with no bag
/// item worn there (the engine's bags are not worn items unless the test
/// wears one).
fn pickup_bag_from_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(inv) = num_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let source = Source::Equipped(inv as i64);
    let back = matches!(cursor::held_item(vm), Some((_, from)) if from == source);
    let empty = matches!(engine(vm).world.character.cursor, Cursor::Empty);
    let worn = engine(vm)
        .world
        .character
        .equipment
        .contains_key(&(inv as i64));
    if back || (empty && worn) {
        cursor::pickup_item_at(vm, source);
    }
    Ok(vec![])
}

/// `AutoEquipCursorItem()`: the cursor's bag item to the gear slot its
/// kind goes to, as a drag onto the gear (`inventory::cursor_move`: let go,
/// the server's swap); a worn one put back; one that is not worn stays
/// held (unverified: the client's error not told).
fn auto_equip_cursor_item(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((held, from)) = cursor::held_item(vm) else {
        return Ok(vec![]);
    };
    match from {
        Source::Equipped(_) => cursor::pickup_item_at(vm, from),
        Source::Bag(..) => {
            if let Some(slot) = crate::inventory::slot_for(vm, held.id) {
                crate::inventory::cursor_move(vm, from, Source::Equipped(slot));
            }
        }
    }
    Ok(vec![])
}

/// The cursor's bag item sold to the open vendor: let go (unlocked, the
/// grid hidden), then the sale as a right-click's (`merchant::sell`:
/// locked again, sold a round trip later), the order `DeleteCursorItem`
/// lets go in; nothing away from a vendor or for a worn item.
fn sell_cursor(vm: &mut Vm) {
    if !engine(vm).world.merchant.open {
        return;
    }
    let Some((_, Source::Bag(bag, slot))) = cursor::held_item(vm) else {
        return;
    };
    let source = Source::Bag(bag, slot);
    cursor::lock(vm, &source, false);
    cursor::release_keeping_lock(vm);
    crate::merchant::sell(vm, bag, slot);
}

/// `SellCursorItem()`: the cursor's item sold (Blizzard's refund popup
/// calls it).
fn sell_cursor_item(vm: &mut Vm, _c: NativeCall) -> Ret {
    sell_cursor(vm);
    Ok(vec![])
}

/// `PickupMerchantItem(index)`: with an item on the cursor, the item
/// dropped on the vendor's window is sold (Blizzard's merchant frame calls
/// `PickupMerchantItem(0)` on a click with one); the vendor's goods taken
/// onto the cursor are not modelled (the cursor holds no goods), so
/// otherwise nothing.
fn pickup_merchant_item(vm: &mut Vm, _c: NativeCall) -> Ret {
    if holds_item(vm) {
        sell_cursor(vm);
    }
    Ok(vec![])
}

const REPAIR_CURSOR: &str = "actions2.repairCursor";

fn set_repair_cursor(vm: &mut Vm, on: bool) {
    engine_mut(vm)
        .lua_helpers
        .insert(REPAIR_CURSOR, Value::Bool(on));
}

/// `ShowRepairCursor()`: the repair cursor on (Blizzard's repair button
/// toggles it by `InRepairMode`; no event: recorded, `repair`).
fn show_repair_cursor(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_repair_cursor(vm, true);
    Ok(vec![])
}

/// `HideRepairCursor()`: the repair cursor off.
fn hide_repair_cursor(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_repair_cursor(vm, false);
    Ok(vec![])
}

/// The repair cursor gone (the vendor's window closed).
pub fn end_repair_mode(vm: &mut Vm) {
    set_repair_cursor(vm, false);
}

/// Whether the repair cursor is on: `ShowRepairCursor` and
/// `HideRepairCursor` leave it so, while a vendor that repairs is open.
pub fn repairing(vm: &Vm) -> bool {
    let on = matches!(
        engine(vm).lua_helpers.get(REPAIR_CURSOR),
        Some(Value::Bool(true))
    );
    let m = &engine(vm).world.merchant;
    on && m.open && m.can_repair
}

/// `InRepairMode()` (`repairing`; recorded only through its effects: the
/// repair cursor's click on a worn item mends it, `repair`).
pub fn in_repair_mode(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(repairing(vm))])
}

// ------------------------------------------------------------------ mail

/// The letter an index argument names: its place and table.
fn letter_at(vm: &Vm, c: &NativeCall) -> Option<(usize, TableRef)> {
    let i = num_arg(vm, c, 1)?;
    if i < 1.0 {
        return None;
    }
    let i = i as usize - 1;
    engine(vm)
        .world
        .mail
        .letters
        .get(i)
        .copied()
        .map(|t| (i, t))
}

/// A letter's place in the inbox now, if it is still there.
fn index_of(vm: &Vm, t: TableRef) -> Option<usize> {
    engine(vm).world.mail.letters.iter().position(|l| *l == t)
}

fn letter_of(c: &NativeCall) -> Option<TableRef> {
    match c.upvals.first() {
        Some(Value::Table(t)) => Some(*t),
        _ => None,
    }
}

/// The letter's items not taken yet (as `mail.rs` counts them).
fn items_left(vm: &mut Vm, t: TableRef) -> usize {
    let Value::Table(l) = field(vm, t, "items") else {
        return 0;
    };
    let n = vm.heap.table(l).border();
    let mut left = 0;
    for i in 1..=n {
        if let Value::Table(item) = vm.heap.table(l).get(Value::Num(i as f64))
            && !field(vm, item, "taken").truthy()
        {
            left += 1;
        }
    }
    left
}

fn money_of(vm: &mut Vm, t: TableRef) -> f64 {
    num(field(vm, t, "money")).unwrap_or(0.0)
}

/// An auction's letter emptied: `mail.rs` removes it itself (its
/// `CLOSE_INBOX_ITEM`, then gone); Blizzard's open letter, hidden by that
/// close, deletes it again (`DeleteInboxItem` for an emptied letter with
/// its text read): nothing more to do.
fn going(vm: &mut Vm, t: TableRef) -> bool {
    field(vm, t, "auction").truthy() && items_left(vm, t) == 0 && money_of(vm, t) <= 0.0
}

/// The letter gone from the inbox: `MAIL_INBOX_UPDATE`, `MAIL_SUCCESS`
/// (as `mail.rs`'s emptied auction letter goes, recorded).
fn gone(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = letter_of(&c) else {
        return Ok(vec![]);
    };
    let Some(i) = index_of(vm, t) else {
        return Ok(vec![]);
    };
    engine_mut(vm).world.mail.letters.remove(i);
    fire(vm, "MAIL_INBOX_UPDATE", &[]);
    fire(vm, "MAIL_SUCCESS", &[Value::Nil]);
    Ok(vec![])
}

/// `TakeInboxMoney(index)`: the letter's money, the server's answer a
/// round trip later (as `TakeInboxItem`'s).
fn take_inbox_money(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter_at(vm, &c) else {
        return Ok(vec![]);
    };
    if money_of(vm, t) <= 0.0 {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    later(
        vm,
        delay,
        "mailMoneyTaken",
        money_taken,
        vec![Value::Table(t)],
    );
    Ok(vec![])
}

/// The server gave the money: as an item taken (`mail.rs`'s `taken`: an
/// auction's letter emptied closed, the inbox's update, `MAIL_SUCCESS`);
/// the emptied auction letter gone after, the money the player's with it
/// (`PLAYER_MONEY` after the going's `MAIL_SUCCESS`: recorded once,
/// `mail2`'s sold auction). A letter that stays: the money at once
/// (unverified).
fn money_taken(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = letter_of(&c) else {
        return Ok(vec![]);
    };
    let Some(i) = index_of(vm, t) else {
        return Ok(vec![]);
    };
    let money = money_of(vm, t);
    if money <= 0.0 {
        return Ok(vec![]);
    }
    set(vm, t, "money", Value::Num(0.0));
    let goes = going(vm, t);
    if goes {
        fire(vm, "CLOSE_INBOX_ITEM", &[Value::Num((i + 1) as f64)]);
    }
    fire(vm, "MAIL_INBOX_UPDATE", &[]);
    fire(vm, "MAIL_SUCCESS", &[Value::Nil]);
    if goes {
        later(
            vm,
            crate::mail::timing(vm).gone,
            "mailMoneyLetterGone",
            gone_with_money,
            vec![Value::Table(t), Value::Num(money)],
        );
    } else {
        engine_mut(vm).world.character.money += money;
        fire(vm, "PLAYER_MONEY", &[]);
    }
    Ok(vec![])
}

/// The emptied letter gone (`gone`), then its money the player's.
fn gone_with_money(vm: &mut Vm, c: NativeCall) -> Ret {
    let money = c.upvals.get(1).copied().and_then(num).unwrap_or(0.0);
    gone(vm, c)?;
    engine_mut(vm).world.character.money += money;
    fire(vm, "PLAYER_MONEY", &[]);
    Ok(vec![])
}

/// `DeleteInboxItem(index)`: the letter gone, the server's answer a round
/// trip later (Blizzard's letter asks first when it holds items or money;
/// an emptied auction letter is already going).
fn delete_inbox_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter_at(vm, &c) else {
        return Ok(vec![]);
    };
    if going(vm, t) {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    later(vm, delay, "mailDeleted", gone, vec![Value::Table(t)]);
    Ok(vec![])
}

/// `ReturnInboxItem(index)`: the letter back to its sender, gone from the
/// inbox a round trip later; not a returned letter, an auction's or a
/// GM's.
fn return_inbox_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter_at(vm, &c) else {
        return Ok(vec![]);
    };
    for key in ["wasReturned", "auction", "isGM"] {
        if field(vm, t, key).truthy() {
            return Ok(vec![]);
        }
    }
    let delay = crate::clock::server_delay(vm);
    later(vm, delay, "mailReturned", gone, vec![Value::Table(t)]);
    Ok(vec![])
}

/// The copy of a letter's text: a Plain Letter (warcraft.wiki.gg).
const PLAIN_LETTER: i64 = 8383;

/// `TakeInboxTextItem(index)`: a copy of the letter's text, once; the
/// server's answer a round trip later.
fn take_inbox_text_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter_at(vm, &c) else {
        return Ok(vec![]);
    };
    if field(vm, t, "textCreated").truthy() {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    later(
        vm,
        delay,
        "mailTextTaken",
        text_taken,
        vec![Value::Table(t)],
    );
    Ok(vec![])
}

/// The copy made: the letter's `textCreated`, the inbox's update,
/// `MAIL_SUCCESS`; the letter pushed into the bags as an attachment taken
/// (`mail.rs`'s `taken` and its bags' packet) when the item's data are
/// known.
fn text_taken(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = letter_of(&c) else {
        return Ok(vec![]);
    };
    if index_of(vm, t).is_none() || field(vm, t, "textCreated").truthy() {
        return Ok(vec![]);
    }
    set(vm, t, "textCreated", Value::Bool(true));
    fire(vm, "MAIL_INBOX_UPDATE", &[]);
    fire(vm, "MAIL_SUCCESS", &[Value::Nil]);
    if crate::gamedata::item_data(vm, PLAIN_LETTER).is_none() {
        return Ok(vec![]);
    }
    let Some((bag, slot)) = character::push_to_bags(vm, PLAIN_LETTER, 1.0) else {
        return Ok(vec![]);
    };
    let icon = character::item_field(vm, PLAIN_LETTER, "icon");
    fire(
        vm,
        "ITEM_PUSH",
        &[Value::Num(character::bag_inventory_slot(bag)), icon],
    );
    crate::merchant::loot_message(vm, PLAIN_LETTER, 1.0, "");
    fire(vm, "BAG_NEW_ITEMS_UPDATED", &[]);
    later(
        vm,
        crate::mail::timing(vm).bag,
        "mailTextBag",
        crate::mail::bag_packet,
        vec![
            Value::Num(bag as f64),
            slot.map_or(Value::Nil, |s| Value::Num(s as f64)),
            Value::Num(PLAIN_LETTER as f64),
        ],
    );
    Ok(vec![])
}

/// `ClearSendMail()`: the letter being written emptied: its attachments
/// back in their bag slots unlocked (as `CloseMail` unlocks them), told
/// as an attachment taken back is (`MAIL_SEND_INFO_UPDATE`); its money and
/// COD reset, told as `SendMail`'s reset tells them (each only when it
/// changed; unverified).
fn clear_send_mail(vm: &mut Vm, _c: NativeCall) -> Ret {
    let attached: Vec<(i64, usize)> = engine(vm)
        .world
        .mail
        .attached
        .iter()
        .flatten()
        .copied()
        .collect();
    for (bag, slot) in &attached {
        cursor::lock(vm, &Source::Bag(*bag, *slot), false);
    }
    let (money, cod) = {
        let m = &mut engine_mut(vm).world.mail;
        m.attached.fill(None);
        let was = (m.money, m.cod);
        m.money = 0.0;
        m.cod = 0.0;
        was
    };
    if !attached.is_empty() {
        fire(vm, "MAIL_SEND_INFO_UPDATE", &[]);
    }
    if money != 0.0 {
        fire(vm, "SEND_MAIL_MONEY_CHANGED", &[]);
    }
    if cod != 0.0 {
        fire(vm, "SEND_MAIL_COD_CHANGED", &[]);
    }
    Ok(vec![])
}

// ------------------------------------------------------------- quest log

/// The quest log's headers collapsed or expanded: the header at `index`
/// (0: every header, as Blizzard's classic quest log asks) and the quests
/// under it (recorded in `questlog`: the log reads `isCollapsed` alike on
/// a header and its quests), told by `QUEST_LOG_UPDATE` in the same frame
/// when anything changed (recorded: each of seven header clicks; Blizzard's
/// quest map ignores the updates while it calls these:
/// `ignoreQuestLogUpdate`). The quest map's own calls (`isAuto`, its
/// filters' reset on every map change: on-map headers expanded, the rest
/// collapsed) leave a header the player collapsed or expanded as it is
/// (recorded: none of them changed one in `questlog`, the client's log
/// unchanged by them through four map changes; the untouched headers'
/// part unverified: all of those were as the reset leaves them). The
/// engine's own log, when it changes, is told collapsed again
/// (`quests::sync_log`).
fn set_collapsed(vm: &mut Vm, c: &NativeCall, collapsed: bool) {
    let Some(index) = num_arg(vm, c, 1).map(|n| n as i64) else {
        return;
    };
    let auto = c.arg(2).truthy();
    let changed = {
        let e = engine_mut(vm);
        let manual = &mut e.world.quests.manual_headers;
        let Some(Entry::ByIndex(log)) = e.world.character.stats.get_mut("questLogInfo") else {
            return;
        };
        let mut changed = false;
        let mut chosen = index == 0;
        for (&i, answers) in log.iter_mut() {
            let Some(Answer::Table(fields)) = answers.first_mut() else {
                continue;
            };
            let header = fields.iter().any(|(k, v)| {
                matches!(k, Answer::Str(k) if k == "isHeader") && *v == Answer::Bool(true)
            });
            if header {
                chosen = index == 0 || i == index;
                let title = fields
                    .iter()
                    .find_map(|(k, v)| match (k, v) {
                        (Answer::Str(k), Answer::Str(t)) if k == "title" => Some(t.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                if chosen && auto && manual.contains(&title) {
                    chosen = false;
                } else if chosen && !auto {
                    manual.insert(title);
                }
            }
            if !chosen {
                continue;
            }
            for (k, v) in fields.iter_mut() {
                if matches!(k, Answer::Str(k) if k == "isCollapsed")
                    && *v != Answer::Bool(collapsed)
                {
                    *v = Answer::Bool(collapsed);
                    changed = true;
                }
            }
        }
        changed
    };
    if changed {
        fire(vm, "QUEST_LOG_UPDATE", &[]);
    }
}

/// `ExpandQuestHeader(index [, isAuto])`.
fn expand_quest_header(vm: &mut Vm, c: NativeCall) -> Ret {
    set_collapsed(vm, &c, false);
    Ok(vec![])
}

/// `CollapseQuestHeader(index [, isAuto])`.
fn collapse_quest_header(vm: &mut Vm, c: NativeCall) -> Ret {
    set_collapsed(vm, &c, true);
    Ok(vec![])
}

// ------------------------------------------------------------- the raid

/// The raid member at a raid index (`GetRaidRosterInfo`'s order: the
/// player first, then the members as they joined).
fn raid_member(vm: &Vm, index: f64) -> Option<UnitId> {
    if index < 1.0 || index.fract() != 0.0 {
        return None;
    }
    if index == 1.0 {
        return Some(PLAYER);
    }
    engine(vm).units.group.get(index as usize - 2).copied()
}

/// Whether the player may move the raid's members: leading a raid, out of
/// combat lockdown (warcraft.wiki.gg: `nocombat`; assistants are not
/// modelled).
fn arranging(vm: &Vm) -> bool {
    let e = engine(vm);
    e.units.raid && e.world.party.leader && !e.units.lockdown
}

fn members_in(vm: &Vm, subgroup: f64) -> usize {
    let u = &engine(vm).units;
    std::iter::once(PLAYER)
        .chain(u.group.iter().copied())
        .filter(|&id| units::subgroup(u, id) == subgroup)
        .count()
}

/// `SetRaidSubgroup(index, subgroup)`: the member into a subgroup (1 to 8)
/// that is not full (five), told as the raid's other changes are
/// (`GROUP_ROSTER_UPDATE`, `C_PartyInfo.ConvertToRaid`'s; warcraft.wiki.gg
/// for the rule, unverified).
fn set_raid_subgroup(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(index), Some(subgroup)) = (num_arg(vm, &c, 1), num_arg(vm, &c, 2)) else {
        return Ok(vec![]);
    };
    if !arranging(vm) || !(1.0..=8.0).contains(&subgroup) || subgroup.fract() != 0.0 {
        return Ok(vec![]);
    }
    let Some(id) = raid_member(vm, index) else {
        return Ok(vec![]);
    };
    if units::subgroup(&engine(vm).units, id) == subgroup || members_in(vm, subgroup) >= 5 {
        return Ok(vec![]);
    }
    engine_mut(vm).units.list[id.0].subgroup = Some(subgroup);
    fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    Ok(vec![])
}

/// `SwapRaidSubgroup(index1, index2)`: two members of different subgroups
/// trade places (`GROUP_ROSTER_UPDATE`; unverified).
fn swap_raid_subgroup(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(a), Some(b)) = (num_arg(vm, &c, 1), num_arg(vm, &c, 2)) else {
        return Ok(vec![]);
    };
    if !arranging(vm) {
        return Ok(vec![]);
    }
    let (Some(a), Some(b)) = (raid_member(vm, a), raid_member(vm, b)) else {
        return Ok(vec![]);
    };
    let (sa, sb) = {
        let u = &engine(vm).units;
        (units::subgroup(u, a), units::subgroup(u, b))
    };
    if sa == sb {
        return Ok(vec![]);
    }
    {
        let list = &mut engine_mut(vm).units.list;
        list[a.0].subgroup = Some(sb);
        list[b.0].subgroup = Some(sa);
    }
    fire(vm, "GROUP_ROSTER_UPDATE", &[]);
    Ok(vec![])
}

// ------------------------------------------------------------ inspecting

/// `ClearInspectPlayer()`: the inspected player forgotten (its talent
/// config, `C_Traits.GetConfigInfo(-1)`, no longer answered); what the
/// server sent of its gear stays known (unverified: Blizzard's inspect
/// and talent frames call it as they close).
fn clear_inspect_player(vm: &mut Vm, _c: NativeCall) -> Ret {
    engine_mut(vm).world.inspect.last = None;
    Ok(vec![])
}

// -------------------------------------------- accepted, nothing to change

// `Dismount()`: the engine's only mount is a flight's, which no one gets
// off. `PickupCompanion(type, index)`: no companions (Forever's pets and
// mounts are items and spells). `ShowBuybackSellCursor(index)`: the
// cursor's look alone. `ShowQuestComplete(questID)`,
// `ShowQuestOffer(index)`: no quest completes or is offered by itself.
// `ConfirmAcceptQuest()`: no shared quest waits (`QUEST_ACCEPT_CONFIRM`
// never fires). `ClearPartyAssignment(assignment, unit)`: no one is
// assigned (`SetPartyAssignment` is protected). `RespondMailLockSendItem(
// slot, keepItem)`: no attachment waits on it (`MAIL_LOCK_SEND_ITEMS`
// never fires). All unverified.

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("PutItemInBag", put_item_in_bag),
        ("PutItemInBackpack", put_item_in_backpack),
        ("PickupBagFromSlot", pickup_bag_from_slot),
        ("AutoEquipCursorItem", auto_equip_cursor_item),
        ("SellCursorItem", sell_cursor_item),
        ("PickupMerchantItem", pickup_merchant_item),
        ("ShowRepairCursor", show_repair_cursor),
        ("HideRepairCursor", hide_repair_cursor),
        ("TakeInboxMoney", take_inbox_money),
        ("DeleteInboxItem", delete_inbox_item),
        ("ReturnInboxItem", return_inbox_item),
        ("TakeInboxTextItem", take_inbox_text_item),
        ("ClearSendMail", clear_send_mail),
        ("ExpandQuestHeader", expand_quest_header),
        ("CollapseQuestHeader", collapse_quest_header),
        ("SetRaidSubgroup", set_raid_subgroup),
        ("SwapRaidSubgroup", swap_raid_subgroup),
        ("ClearInspectPlayer", clear_inspect_player),
        ("Dismount", nothing),
        ("PickupCompanion", nothing),
        ("ShowBuybackSellCursor", nothing),
        ("ShowQuestComplete", nothing),
        ("ShowQuestOffer", nothing),
        ("ConfirmAcceptQuest", nothing),
        ("ClearPartyAssignment", nothing),
        ("RespondMailLockSendItem", nothing),
    ]
}
