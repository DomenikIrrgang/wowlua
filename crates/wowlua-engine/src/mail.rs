//! A mailbox, as the client recorded a Horde paladin's in Orgrimmar
//! (`/stdtest engine scenario mail`, a ping near 0.45 s):
//! `defineMailbox(unit, { letters, newMail, senders })` names its letters;
//! `player:interact(mailbox)` opens it a round trip later (`MAIL_SHOW`,
//! `PLAYER_INTERACTION_MANAGER_FRAME_SHOW(17)`); Blizzard's mail frame asks
//! for the inbox (`CheckInbox`), the server's `MAIL_INBOX_UPDATE` a round
//! trip later, another when the invoices' players' names are known.
//!
//! The inbox (`GetInboxNumItems`, `GetInboxHeaderInfo`'s fifteen values,
//! `GetInboxInvoiceInfo`'s twelve for an auction's invoice, seven else,
//! `GetInboxItem`, `GetInboxItemLink`, `HasInboxItem`); a letter read
//! (`GetInboxText`): the server's `MAIL_INBOX_UPDATE`; an item taken
//! (`TakeInboxItem`, `AutoLootMailItem`): the server closes the letter
//! (`CLOSE_INBOX_ITEM`), `MAIL_INBOX_UPDATE`, `MAIL_SUCCESS`, the item's
//! push and "You receive item", the bags' packet after; an auction's
//! letter emptied is gone (`MAIL_INBOX_UPDATE`, `MAIL_SUCCESS`).
//!
//! Sending: a bag item used while the send frame shows is attached at once
//! (locked, `MAIL_SEND_INFO_UPDATE`), right-clicking the attachment
//! (`ClickSendMailItemButton(i, true)`) takes it back; `SetSendMailCOD` and
//! `SetSendMailMoney` tell their events; `SendMail`: the server's answer
//! unlocks the items, resets the money, `MAIL_SEND_SUCCESS`, `MAIL_SUCCESS`,
//! "Mail sent.", the items gone and the postage paid in its next packets.
//! `CloseMail`: `SECURE_TRANSFER_CANCEL` and the interaction's hides; the
//! server's `UPDATE_PENDING_MAIL` a round trip later.
//!
//! Unverified: `GetInboxText`'s values (not recorded), deleting and
//! returning letters, money and COD letters, more than one attachment, a
//! letter one can answer, the postage beyond one item (30 copper).

use std::collections::BTreeMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::cursor::{self, Source};
use crate::units::{self, UnitId};
use crate::{character, engine, engine_mut, widgets};

/// `Enum.PlayerInteractionType.MailInfo`.
const MAILBOX: f64 = 17.0;
/// The invoices' players' names, after the inbox (recorded).
const NAMES_AFTER: f64 = 0.536;
/// A taken item's bag packet after the take (recorded).
pub(crate) const BAG_AFTER: f64 = 0.134;
/// An emptied auction letter gone after the take (recorded).
pub(crate) const GONE_AFTER: f64 = 0.268;
/// After a letter's sending: the spells' update, then the money and bags'
/// (recorded).
const SENT_SPELLS_AFTER: f64 = 0.041;
const SENT_MONEY_AFTER: f64 = 0.286;
/// A letter's attachments.
const ATTACHMENTS: usize = 12;
/// The postage for a letter (recorded: one item, none).
const POSTAGE: f64 = 30.0;

#[derive(Default)]
pub struct Mail {
    /// The mailboxes: unit → the test's table.
    pub boxes: BTreeMap<usize, TableRef>,
    /// The inbox's letters (the test's tables, changed as they are read
    /// and emptied).
    pub letters: Vec<TableRef>,
    pub open: Option<UnitId>,
    /// The invoices' players' names known.
    pub names: bool,
    /// The send frame showing (`SetSendMailShowing`).
    pub sending: bool,
    /// The attachments: the bag slots they came from.
    pub attached: Vec<Option<(i64, usize)>>,
    pub money: f64,
    pub cod: f64,
    pub new_mail: bool,
    /// Senders of mail on its way (an auction's: "Horde Auction House").
    pub senders: Vec<String>,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn later(vm: &mut Vm, after: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, after, native);
}

fn mail(vm: &Vm) -> &Mail {
    &engine(vm).world.mail
}

fn mail_mut(vm: &mut Vm) -> &mut Mail {
    &mut engine_mut(vm).world.mail
}

/// `defineMailbox(unit, { letters = { { packageIcon, stationeryIcon,
/// sender, subject, money, cod, daysLeft, wasRead, wasReturned,
/// textCreated, canReply, isGM, text, items = { { id, count } },
/// invoice = { type, itemName, playerName, bid, buyout, deposit,
/// consignment, moneyDelay, etaHour, etaMin, count, commerceAuction },
/// auction } }, newMail, senders })`: a mailbox and the inbox it holds
/// (`auction`: the letter goes once emptied).
pub fn control_define(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(unit) = units::handle_of(vm, c.arg(1)) else {
        return Err(vm.error("defineMailbox: a mailbox's unit expected"));
    };
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("defineMailbox: a table of the inbox expected"));
    };
    widgets::keep(vm, Value::Table(t));
    let mut letters = Vec::new();
    if let Value::Table(l) = field(vm, t, "letters") {
        let n = vm.heap.table(l).border();
        for i in 1..=n {
            if let Value::Table(letter) = vm.heap.table(l).get(Value::Num(i as f64)) {
                letters.push(letter);
            }
        }
    }
    let new_mail = field(vm, t, "newMail").truthy();
    let m = mail_mut(vm);
    m.boxes.insert(unit.0, t);
    m.letters = letters;
    m.new_mail = new_mail;
    m.attached = vec![None; ATTACHMENTS];
    Ok(vec![])
}

/// Mail on its way from `sender` (`UPDATE_PENDING_MAIL` is the caller's).
pub fn mail_coming(vm: &mut Vm, sender: &str) {
    let m = mail_mut(vm);
    m.new_mail = true;
    if !m.senders.iter().any(|s| s == sender) {
        m.senders.insert(0, sender.to_string());
    }
}

pub fn has(vm: &Vm, unit: UnitId) -> bool {
    mail(vm).boxes.contains_key(&unit.0)
}

/// The player at a mailbox: the server opens it a round trip later.
pub fn interact(vm: &mut Vm, unit: UnitId) {
    let delay = crate::clock::server_delay(vm);
    later(
        vm,
        delay,
        "mailShown",
        shown,
        vec![Value::Num(unit.0 as f64)],
    );
}

fn shown(vm: &mut Vm, c: NativeCall) -> Ret {
    let unit = UnitId(num(c.upvals[0]).unwrap_or(0.0) as usize);
    mail_mut(vm).open = Some(unit);
    engine_mut(vm).units.npc = Some(unit);
    fire(vm, "MAIL_SHOW", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(MAILBOX)],
    );
    Ok(vec![])
}

/// `CheckInbox()`: the inbox a round trip later, the invoices' names after.
fn check_inbox(vm: &mut Vm, _c: NativeCall) -> Ret {
    if mail(vm).open.is_none() {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    later(vm, delay, "mailInbox", inbox_updated, vec![]);
    if !mail(vm).names {
        later(vm, delay + NAMES_AFTER, "mailNames", names_known, vec![]);
    }
    Ok(vec![])
}

fn inbox_updated(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "MAIL_INBOX_UPDATE", &[]);
    Ok(vec![])
}

fn names_known(vm: &mut Vm, _c: NativeCall) -> Ret {
    mail_mut(vm).names = true;
    fire(vm, "MAIL_INBOX_UPDATE", &[]);
    Ok(vec![])
}

fn letter(vm: &mut Vm, c: &NativeCall) -> Option<(usize, TableRef)> {
    let Value::Num(i) = vm.hidden(c.arg(1)) else {
        return None;
    };
    let i = (i as usize).checked_sub(1)?;
    mail(vm).letters.get(i).copied().map(|t| (i, t))
}

/// A letter's attachments still in it: (index, item ID, count).
fn items_of(vm: &mut Vm, t: TableRef) -> Vec<(usize, i64, f64)> {
    let Value::Table(l) = field(vm, t, "items") else {
        return vec![];
    };
    let n = vm.heap.table(l).border();
    let mut out = Vec::new();
    for i in 1..=n {
        let Value::Table(item) = vm.heap.table(l).get(Value::Num(i as f64)) else {
            continue;
        };
        if field(vm, item, "taken").truthy() {
            continue;
        }
        let id = num(field(vm, item, "id")).unwrap_or(0.0) as i64;
        let count = num(field(vm, item, "count")).unwrap_or(1.0);
        out.push((i, id, count));
    }
    out
}

fn attachment(vm: &mut Vm, t: TableRef, a: usize) -> Option<(i64, f64)> {
    items_of(vm, t)
        .into_iter()
        .find(|(i, _, _)| *i == a)
        .map(|(_, id, count)| (id, count))
}

fn get_inbox_num_items(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = mail(vm).letters.len() as f64;
    Ok(vec![Value::Num(n), Value::Num(n)])
}

fn has_new_mail(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(mail(vm).new_mail)])
}

/// `GetLatestThreeSenders()`: the mailbox data's `senders`, else the
/// unread letters' senders.
fn get_latest_three_senders(vm: &mut Vm, _c: NativeCall) -> Ret {
    let pending = mail(vm).senders.clone();
    if !pending.is_empty() {
        return Ok(pending.iter().take(3).map(|s| vm.str(s)).collect());
    }
    let boxes: Vec<TableRef> = mail(vm).boxes.values().copied().collect();
    for t in boxes {
        if let Value::Table(s) = field(vm, t, "senders") {
            let n = vm.heap.table(s).border().min(3);
            return Ok((1..=n)
                .map(|i| vm.heap.table(s).get(Value::Num(i as f64)))
                .collect());
        }
    }
    Ok(vec![])
}

/// `GetInboxHeaderInfo(i)`: the first item's icon (none once emptied),
/// the stationery, sender, subject, money, COD, days left, the items'
/// count, read, returned, text created, can reply, from a GM, the first
/// item's count and link (recorded).
fn get_inbox_header_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter(vm, &c) else {
        return Ok(vec![]);
    };
    let items = items_of(vm, t);
    let first = items.first().copied();
    let icon = match first {
        Some((_, id, _)) => character::item_field(vm, id, "icon"),
        None => Value::Nil,
    };
    let link = match first {
        Some((_, id, _)) => match character::link_in_context(vm, id, "", &[]) {
            Some(l) => vm.str(&l),
            None => Value::Nil,
        },
        None => Value::Nil,
    };
    let get = |vm: &mut Vm, k: &str| field(vm, t, k);
    let flag = |vm: &mut Vm, k: &str| Value::Bool(field(vm, t, k).truthy());
    let count = if items.is_empty() {
        Value::Nil
    } else {
        Value::Num(items.len() as f64)
    };
    Ok(vec![
        icon,
        get(vm, "stationeryIcon"),
        get(vm, "sender"),
        get(vm, "subject"),
        Value::Num(num(get(vm, "money")).unwrap_or(0.0)),
        Value::Num(num(get(vm, "cod")).unwrap_or(0.0)),
        get(vm, "daysLeft"),
        count,
        flag(vm, "wasRead"),
        flag(vm, "wasReturned"),
        flag(vm, "textCreated"),
        flag(vm, "canReply"),
        flag(vm, "isGM"),
        first.map_or(Value::Nil, |(_, _, n)| Value::Num(n)),
        link,
    ])
}

/// `GetInboxInvoiceInfo(i)`: an auction's invoice (its player's name empty
/// till the names are known), else the seven of none (recorded).
fn get_inbox_invoice_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter(vm, &c) else {
        return Ok(vec![]);
    };
    let Value::Table(inv) = field(vm, t, "invoice") else {
        let zero = Value::Num(0.0);
        return Ok(vec![
            Value::Nil,
            Value::Nil,
            Value::Nil,
            zero,
            zero,
            zero,
            zero,
        ]);
    };
    let mut out: Vec<Value> = (1..=12)
        .map(|i| vm.heap.table(inv).get(Value::Num(f64::from(i))))
        .collect();
    if !mail(vm).names {
        out[2] = vm.str("");
    }
    Ok(out)
}

/// `GetInboxItem(i, a)`: name, ID, icon, count, quality, usable, crafted
/// (recorded).
fn get_inbox_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter(vm, &c) else {
        return Ok(vec![]);
    };
    let a = num(vm.hidden(c.arg(2))).unwrap_or(1.0) as usize;
    let Some((id, count)) = attachment(vm, t, a) else {
        return Ok(vec![]);
    };
    let name = character::item_field(vm, id, "name");
    let icon = character::item_field(vm, id, "icon");
    let quality = character::item_field(vm, id, "quality");
    Ok(vec![
        name,
        Value::Num(id as f64),
        icon,
        Value::Num(count),
        quality,
        Value::Bool(true),
        Value::Bool(false),
    ])
}

fn get_inbox_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter(vm, &c) else {
        return Ok(vec![]);
    };
    let a = num(vm.hidden(c.arg(2))).unwrap_or(1.0) as usize;
    let Some((id, _)) = attachment(vm, t, a) else {
        return Ok(vec![]);
    };
    Ok(match character::link_in_context(vm, id, "", &[]) {
        Some(l) => vec![vm.str(&l)],
        None => vec![],
    })
}

fn has_inbox_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter(vm, &c) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let a = num(vm.hidden(c.arg(2))).unwrap_or(1.0) as usize;
    Ok(vec![Value::Bool(attachment(vm, t, a).is_some())])
}

/// `GetInboxText(i)`: the letter read (the server's `MAIL_INBOX_UPDATE` a
/// round trip later): its text (empty without one: Blizzard's letter sets
/// it, a string), stationery, takeable, an invoice (unverified: not
/// recorded).
fn get_inbox_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter(vm, &c) else {
        return Ok(vec![]);
    };
    if !field(vm, t, "wasRead").truthy() {
        set(vm, t, "wasRead", Value::Bool(true));
        let delay = crate::clock::server_delay(vm);
        later(vm, delay, "mailRead", inbox_updated, vec![]);
    }
    let invoice = matches!(field(vm, t, "invoice"), Value::Table(_));
    let text = match field(vm, t, "text") {
        Value::Str(s) => Value::Str(s),
        _ => vm.str(""),
    };
    Ok(vec![
        text,
        Value::Nil,
        Value::Nil,
        Value::Bool(false),
        Value::Bool(invoice),
    ])
}

/// `InboxItemCanDelete(i)`: a letter without items or money (unverified).
fn inbox_item_can_delete(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter(vm, &c) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let money = num(field(vm, t, "money")).unwrap_or(0.0);
    Ok(vec![Value::Bool(
        items_of(vm, t).is_empty() && money == 0.0,
    )])
}

/// `TakeInboxItem(i, a)`: the server's answer a round trip later.
fn take_inbox_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((i, t)) = letter(vm, &c) else {
        return Ok(vec![]);
    };
    let a = num(vm.hidden(c.arg(2))).unwrap_or(1.0) as usize;
    if attachment(vm, t, a).is_none() {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    later(
        vm,
        delay,
        "mailTaken",
        taken,
        vec![Value::Num(i as f64), Value::Num(a as f64)],
    );
    Ok(vec![])
}

/// `AutoLootMailItem(i)`: the letter's first item taken.
fn auto_loot_mail_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((i, t)) = letter(vm, &c) else {
        return Ok(vec![]);
    };
    let Some((a, _, _)) = items_of(vm, t).first().copied() else {
        return Ok(vec![]);
    };
    let delay = crate::clock::server_delay(vm);
    later(
        vm,
        delay,
        "mailTaken",
        taken,
        vec![Value::Num(i as f64), Value::Num(a as f64)],
    );
    Ok(vec![])
}

/// The server gave the item: the letter closed (an auction's, emptied, to
/// go), the inbox's update, the item pushed into the bags; their packet
/// after; the emptied auction letter gone.
fn taken(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = num(c.upvals[0]).unwrap_or(0.0) as usize;
    let a = num(c.upvals[1]).unwrap_or(1.0) as usize;
    let Some(t) = mail(vm).letters.get(i).copied() else {
        return Ok(vec![]);
    };
    let Some((id, count)) = attachment(vm, t, a) else {
        return Ok(vec![]);
    };
    if let Value::Table(l) = field(vm, t, "items")
        && let Value::Table(item) = vm.heap.table(l).get(Value::Num(a as f64))
    {
        set(vm, item, "taken", Value::Bool(true));
    }
    let emptied = items_of(vm, t).is_empty();
    let goes = emptied && field(vm, t, "auction").truthy();
    if goes {
        fire(vm, "CLOSE_INBOX_ITEM", &[Value::Num((i + 1) as f64)]);
    }
    fire(vm, "MAIL_INBOX_UPDATE", &[]);
    fire(vm, "MAIL_SUCCESS", &[Value::Nil]);
    let Some((bag, slot)) = character::push_to_bags(vm, id, count) else {
        return Ok(vec![]);
    };
    let icon = character::item_field(vm, id, "icon");
    fire(
        vm,
        "ITEM_PUSH",
        &[Value::Num(character::bag_inventory_slot(bag)), icon],
    );
    crate::merchant::loot_message(vm, id, count, "");
    fire(vm, "BAG_NEW_ITEMS_UPDATED", &[]);
    later(
        vm,
        BAG_AFTER,
        "mailBag",
        bag_packet,
        vec![
            Value::Num(bag as f64),
            slot.map_or(Value::Nil, |s| Value::Num(s as f64)),
            Value::Num(id as f64),
        ],
    );
    if goes {
        later(
            vm,
            GONE_AFTER,
            "mailGone",
            letter_gone,
            vec![Value::Num(i as f64)],
        );
    }
    Ok(vec![])
}

/// The bags' packet: the spells' update, the bag's, the count's.
pub(crate) fn bag_packet(vm: &mut Vm, c: NativeCall) -> Ret {
    let bag = num(c.upvals[0]).unwrap_or(0.0) as i64;
    if let Some(slot) = num(c.upvals[1]) {
        character::arrived(vm, bag, slot as usize);
    }
    character::equipped_spells_changed(vm);
    character::bag_updated(vm, bag);
    units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", units::PLAYER, &[]);
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    fire(vm, "ITEM_COUNT_CHANGED", &[c.upvals[2]]);
    Ok(vec![])
}

fn letter_gone(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = num(c.upvals[0]).unwrap_or(0.0) as usize;
    if i < mail(vm).letters.len() {
        mail_mut(vm).letters.remove(i);
    }
    fire(vm, "MAIL_INBOX_UPDATE", &[]);
    fire(vm, "MAIL_SUCCESS", &[Value::Nil]);
    Ok(vec![])
}

fn set_send_mail_showing(vm: &mut Vm, c: NativeCall) -> Ret {
    mail_mut(vm).sending = c.arg(1).truthy();
    Ok(vec![])
}

/// Whether a used bag item goes to the letter being written.
pub fn writing(vm: &Vm) -> bool {
    let m = mail(vm);
    m.open.is_some() && m.sending
}

/// A bag item used while writing: attached to the first free slot, locked
/// (`MAIL_SEND_INFO_UPDATE`).
pub fn attach(vm: &mut Vm, bag: i64, slot: usize) {
    if mail(vm).attached.contains(&Some((bag, slot))) {
        return;
    }
    let Some(free) = mail(vm).attached.iter().position(Option::is_none) else {
        return;
    };
    mail_mut(vm).attached[free] = Some((bag, slot));
    cursor::lock(vm, &Source::Bag(bag, slot), true);
    fire(vm, "MAIL_SEND_INFO_UPDATE", &[]);
}

/// `ClickSendMailItemButton(i, clear)`: the attachment taken back (to its
/// bag slot, unlocked; picking it up unverified).
fn click_send_mail_item_button(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(i) = num(vm.hidden(c.arg(1))) else {
        return Ok(vec![]);
    };
    let Some(Some((bag, slot))) = mail(vm).attached.get((i as usize).wrapping_sub(1)).copied()
    else {
        return Ok(vec![]);
    };
    mail_mut(vm).attached[i as usize - 1] = None;
    cursor::lock(vm, &Source::Bag(bag, slot), false);
    fire(vm, "MAIL_SEND_INFO_UPDATE", &[]);
    Ok(vec![])
}

fn attached_item(vm: &mut Vm, c: &NativeCall) -> Option<(i64, f64)> {
    let i = num(vm.hidden(c.arg(1)))? as usize;
    let (bag, slot) = mail(vm).attached.get(i.checked_sub(1)?).copied()??;
    let held = engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .and_then(|b| b.items.get(&slot))
        .cloned()?;
    Some((held.id, held.count))
}

/// `GetSendMailItem(i)`: name, ID, icon, count, quality (recorded).
fn get_send_mail_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((id, count)) = attached_item(vm, &c) else {
        return Ok(vec![]);
    };
    let name = character::item_field(vm, id, "name");
    let icon = character::item_field(vm, id, "icon");
    let quality = character::item_field(vm, id, "quality");
    Ok(vec![
        name,
        Value::Num(id as f64),
        icon,
        Value::Num(count),
        quality,
    ])
}

fn get_send_mail_item_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((id, _)) = attached_item(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(match character::link_in_context(vm, id, "", &[]) {
        Some(l) => vec![vm.str(&l)],
        None => vec![],
    })
}

fn has_send_mail_item(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(attached_item(vm, &c).is_some())])
}

/// `GetSendMailPrice()`: 30 copper a letter, an item more each after the
/// first (unverified beyond one).
fn get_send_mail_price(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = mail(vm).attached.iter().flatten().count().max(1);
    Ok(vec![Value::Num(POSTAGE * n as f64)])
}

fn set_send_mail_money(vm: &mut Vm, c: NativeCall) -> Ret {
    mail_mut(vm).money = num(vm.hidden(c.arg(1))).unwrap_or(0.0);
    fire(vm, "SEND_MAIL_MONEY_CHANGED", &[]);
    Ok(vec![Value::Bool(true)])
}

fn set_send_mail_cod(vm: &mut Vm, c: NativeCall) -> Ret {
    mail_mut(vm).cod = num(vm.hidden(c.arg(1))).unwrap_or(0.0);
    fire(vm, "SEND_MAIL_COD_CHANGED", &[]);
    Ok(vec![])
}

fn get_send_mail_money(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(mail(vm).money)])
}

fn get_send_mail_cod(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(mail(vm).cod)])
}

/// `SendMail(to, subject, body)`: the server's answer a round trip later.
fn send_mail(vm: &mut Vm, _c: NativeCall) -> Ret {
    if mail(vm).open.is_none() {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    later(vm, delay, "mailSent", sent, vec![]);
    Ok(vec![])
}

/// Sent: the attachments unlocked, the money and COD reset, "Mail sent.";
/// the spells' update, then the postage paid and the items gone.
fn sent(vm: &mut Vm, _c: NativeCall) -> Ret {
    let attached: Vec<(i64, usize)> = mail(vm).attached.iter().flatten().copied().collect();
    let postage = POSTAGE * attached.len().max(1) as f64 + mail(vm).money;
    for (bag, slot) in &attached {
        cursor::lock(vm, &Source::Bag(*bag, *slot), false);
    }
    {
        let m = mail_mut(vm);
        m.attached = vec![None; ATTACHMENTS];
        m.money = 0.0;
        m.cod = 0.0;
    }
    fire(vm, "SEND_MAIL_MONEY_CHANGED", &[]);
    fire(vm, "SEND_MAIL_COD_CHANGED", &[]);
    fire(vm, "MAIL_SEND_SUCCESS", &[]);
    fire(vm, "MAIL_SUCCESS", &[Value::Nil]);
    let code = match vm.get_global("LE_GAME_ERR_MAIL_SENT") {
        Value::Num(n) => n,
        _ => 0.0,
    };
    let text = match vm.get_global("ERR_MAIL_SENT") {
        Value::Str(s) => Value::Str(s),
        _ => vm.str("Mail sent."),
    };
    fire(vm, "UI_INFO_MESSAGE", &[Value::Num(code), text]);
    let list = vm.new_table();
    for (i, (bag, slot)) in attached.iter().enumerate() {
        let pair = vm.new_table();
        let _ = vm
            .heap
            .table_mut(pair)
            .set(Value::Num(1.0), Value::Num(*bag as f64));
        let _ = vm
            .heap
            .table_mut(pair)
            .set(Value::Num(2.0), Value::Num(*slot as f64));
        let _ = vm
            .heap
            .table_mut(list)
            .set(Value::Num(i as f64 + 1.0), Value::Table(pair));
    }
    widgets::keep(vm, Value::Table(list));
    later(vm, SENT_SPELLS_AFTER, "mailSentSpells", sent_spells, vec![]);
    later(
        vm,
        SENT_MONEY_AFTER,
        "mailSentItems",
        sent_items,
        vec![Value::Table(list), Value::Num(postage)],
    );
    Ok(vec![])
}

fn sent_spells(vm: &mut Vm, _c: NativeCall) -> Ret {
    character::equipped_spells_changed(vm);
    Ok(vec![])
}

fn sent_items(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(list) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let postage = num(c.upvals[1]).unwrap_or(POSTAGE);
    {
        let money = &mut engine_mut(vm).world.character.money;
        *money = (*money - postage).max(0.0);
    }
    fire(vm, "PLAYER_MONEY", &[]);
    let n = vm.heap.table(list).border();
    let mut bags = Vec::new();
    for i in 1..=n {
        let Value::Table(pair) = vm.heap.table(list).get(Value::Num(i as f64)) else {
            continue;
        };
        let bag = num(vm.heap.table(pair).get(Value::Num(1.0))).unwrap_or(0.0) as i64;
        let slot = num(vm.heap.table(pair).get(Value::Num(2.0))).unwrap_or(0.0) as usize;
        if let Some(b) = engine_mut(vm).world.character.bags.get_mut(&bag) {
            b.items.remove(&slot);
        }
        if !bags.contains(&bag) {
            bags.push(bag);
        }
    }
    for bag in bags {
        character::bag_updated(vm, bag);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    Ok(vec![])
}

/// `CloseMail()`: `SECURE_TRANSFER_CANCEL`, the interaction's hides; the
/// server's `UPDATE_PENDING_MAIL` a round trip later.
fn close_mail(vm: &mut Vm, _c: NativeCall) -> Ret {
    if mail_mut(vm).open.take().is_none() {
        return Ok(vec![]);
    }
    let attached: Vec<(i64, usize)> = mail(vm).attached.iter().flatten().copied().collect();
    for (bag, slot) in attached {
        cursor::lock(vm, &Source::Bag(bag, slot), false);
    }
    {
        let m = mail_mut(vm);
        m.attached = vec![None; ATTACHMENTS];
        m.sending = false;
    }
    engine_mut(vm).units.npc = None;
    fire(vm, "SECURE_TRANSFER_CANCEL", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(MAILBOX)],
    );
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(0.0)],
    );
    let delay = crate::clock::server_delay(vm);
    later(vm, delay, "mailPending", pending, vec![]);
    Ok(vec![])
}

fn pending(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "UPDATE_PENDING_MAIL", &[]);
    Ok(vec![])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn yes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// `C_Mail.HasInboxMoney(i)`.
fn has_inbox_money(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, t)) = letter(vm, &c) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let money = num(field(vm, t, "money")).unwrap_or(0.0);
    Ok(vec![Value::Bool(money > 0.0)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("CheckInbox", check_inbox),
        ("GetInboxNumItems", get_inbox_num_items),
        ("HasNewMail", has_new_mail),
        ("GetLatestThreeSenders", get_latest_three_senders),
        ("GetInboxHeaderInfo", get_inbox_header_info),
        ("GetInboxInvoiceInfo", get_inbox_invoice_info),
        ("GetInboxItem", get_inbox_item),
        ("GetInboxItemLink", get_inbox_item_link),
        ("HasInboxItem", has_inbox_item),
        ("GetInboxText", get_inbox_text),
        ("InboxItemCanDelete", inbox_item_can_delete),
        ("TakeInboxItem", take_inbox_item),
        ("AutoLootMailItem", auto_loot_mail_item),
        ("SetSendMailShowing", set_send_mail_showing),
        ("ClickSendMailItemButton", click_send_mail_item_button),
        ("GetSendMailItem", get_send_mail_item),
        ("GetSendMailItemLink", get_send_mail_item_link),
        ("HasSendMailItem", has_send_mail_item),
        ("GetSendMailPrice", get_send_mail_price),
        ("SetSendMailMoney", set_send_mail_money),
        ("SetSendMailCOD", set_send_mail_cod),
        ("GetSendMailMoney", get_send_mail_money),
        ("GetSendMailCOD", get_send_mail_cod),
        ("SendMail", send_mail),
        ("CloseMail", close_mail),
        ("CanComplainInboxItem", no),
        ("C_Mail.HasInboxMoney", has_inbox_money),
        ("C_Mail.IsCommandPending", no),
        ("C_Mail.CanCheckInbox", yes),
        ("C_Mail.SetOpeningAll", nothing),
        ("C_Mail.GetCraftingOrderMailInfo", nothing),
    ]
}
