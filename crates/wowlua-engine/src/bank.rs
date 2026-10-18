//! A banker, as the client recorded a Horde paladin's in Orgrimmar
//! (`/stdtest engine scenario bank`, a ping near 0.26 s):
//! `defineBank(unit, { tabs, nextTabCost, tabSlots })` names the
//! character's bank tabs (bags 6 on, `setBag` their contents);
//! `player:interact(banker)` opens it a round trip later
//! (`BANKFRAME_OPENED`, `PLAYER_INTERACTION_MANAGER_FRAME_SHOW(67)`).
//!
//! `C_Bank`'s answers by bank type (0 the character's, 2 the account's,
//! none here): usable and viewable at the bank, the tabs bought
//! (`FetchPurchasedBankTabData`: empty away from it, recorded), the next
//! tab's price, one to buy at the bank. `C_Bank.PurchaseBankTab(0)`: the
//! server's `BANK_TAB_SETTINGS_UPDATED(0)`, `PLAYER_MONEY` (the price
//! paid), `BANK_TABS_CHANGED(0)`. `C_Bank.CloseBankFrame()`:
//! `BANKFRAME_CLOSED` and the interaction's hides.
//!
//! Unverified: the account's bank, money, a new tab's size (its data's
//! `tabSlots`, else 98), the tabs' settings.

use std::collections::BTreeMap;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// `Enum.PlayerInteractionType.CharacterBanker`.
const BANKER: f64 = 67.0;
/// `Enum.BankType.Character`.
const CHARACTER: f64 = 0.0;
/// The character's first bank tab's bag (`Enum.BagIndex.CharacterBankTab_1`).
const FIRST_TAB: i64 = 6;

#[derive(Default)]
pub struct Bank {
    /// The bankers: unit → the test's table.
    pub bankers: BTreeMap<usize, TableRef>,
    /// The character's tabs (`{ name, icon, depositFlags }`), the bags 6 on.
    pub tabs: Vec<TableRef>,
    pub open: Option<UnitId>,
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

fn bank(vm: &Vm) -> &Bank {
    &engine(vm).world.bank
}

fn bank_mut(vm: &mut Vm) -> &mut Bank {
    &mut engine_mut(vm).world.bank
}

/// `defineBank(unit, { tabs = { { name, icon, depositFlags, slots, items }
/// }, nextTabCost, tabSlots })`: a banker and the character's bank tabs
/// (their bags 6 on, `slots` and `items` as `setBag`'s).
pub fn control_define(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(unit) = units::handle_of(vm, c.arg(1)) else {
        return Err(vm.error("defineBank: a banker expected"));
    };
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("defineBank: a table of the bank expected"));
    };
    widgets::keep(vm, Value::Table(t));
    let mut tabs = Vec::new();
    if let Value::Table(l) = field(vm, t, "tabs") {
        let n = vm.heap.table(l).border();
        for i in 1..=n {
            if let Value::Table(tab) = vm.heap.table(l).get(Value::Num(i as f64)) {
                let slots = num(field(vm, tab, "slots")).unwrap_or(98.0) as usize;
                let items = match field(vm, tab, "items") {
                    Value::Table(items) => Some(items),
                    _ => None,
                };
                crate::character::place_bag(vm, FIRST_TAB + i as i64 - 1, slots, items);
                tabs.push(tab);
            }
        }
    }
    let b = bank_mut(vm);
    b.bankers.insert(unit.0, t);
    b.tabs = tabs;
    Ok(vec![])
}

/// Whether the bank is open (a bag item used goes into it).
pub fn is_open(vm: &Vm) -> bool {
    bank(vm).open.is_some()
}

/// The first free slot of the character's bank tabs.
pub fn free_slot(vm: &Vm) -> Option<(i64, usize)> {
    let ch = &engine(vm).world.character;
    (0..bank(vm).tabs.len() as i64).find_map(|i| {
        let bag = FIRST_TAB + i;
        let b = ch.bags.get(&bag)?;
        (1..=b.slots)
            .find(|s| !b.items.contains_key(s))
            .map(|s| (bag, s))
    })
}

pub fn has(vm: &Vm, unit: UnitId) -> bool {
    bank(vm).bankers.contains_key(&unit.0)
}

/// The player at a banker: the bank opens a round trip later.
pub fn interact(vm: &mut Vm, unit: UnitId) {
    let delay = crate::clock::server_delay(vm);
    let native = vm.native_with("bankOpened", opened, vec![Value::Num(unit.0 as f64)]);
    crate::clock::after_engine(vm, delay, native);
}

fn opened(vm: &mut Vm, c: NativeCall) -> Ret {
    let unit = UnitId(num(c.upvals[0]).unwrap_or(0.0) as usize);
    bank_mut(vm).open = Some(unit);
    engine_mut(vm).units.npc = Some(unit);
    fire(vm, "BANKFRAME_OPENED", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(BANKER)],
    );
    Ok(vec![])
}

fn open_data(vm: &Vm) -> Option<TableRef> {
    let b = bank(vm);
    b.open.and_then(|u| b.bankers.get(&u.0).copied())
}

/// The bank type asked about: the character's (the account's has nothing
/// here).
fn character(vm: &mut Vm, c: &NativeCall) -> bool {
    num(vm.hidden(c.arg(1))) == Some(CHARACTER)
}

/// `C_Bank.CanUseBank(type)`, `CanViewBank`: the character's at the bank.
fn can_use_bank(vm: &mut Vm, c: NativeCall) -> Ret {
    let ok = character(vm, &c) && bank(vm).open.is_some();
    Ok(vec![Value::Bool(ok)])
}

fn are_any_bank_types_viewable(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(bank(vm).open.is_some())])
}

fn fetch_num_purchased_bank_tabs(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = if character(vm, &c) {
        bank(vm).tabs.len()
    } else {
        0
    };
    Ok(vec![Value::Num(n as f64)])
}

/// `C_Bank.FetchPurchasedBankTabData(type)`: the tabs at the bank (`ID`
/// their bag), none away from it (recorded).
fn fetch_purchased_bank_tab_data(vm: &mut Vm, c: NativeCall) -> Ret {
    let out = vm.new_table();
    if !character(vm, &c) || bank(vm).open.is_none() {
        return Ok(vec![Value::Table(out)]);
    }
    let cleanup = global(
        vm,
        "BANK_CONFIRM_CLEANUP_PROMPT",
        "Are you sure you want to automatically sort your items?|nThis will affect all tabs.",
    );
    let header = global(vm, "BANK_TAB_NAME_PROMPT", "Enter Bank Tab Name:");
    let tabs = bank(vm).tabs.clone();
    for (i, tab) in tabs.into_iter().enumerate() {
        let e = vm.new_table();
        set(vm, e, "ID", Value::Num((FIRST_TAB + i as i64) as f64));
        set(vm, e, "bankType", Value::Num(CHARACTER));
        let flags = num(field(vm, tab, "depositFlags")).unwrap_or(0.0);
        set(vm, e, "depositFlags", Value::Num(flags));
        let icon = num(field(vm, tab, "icon")).unwrap_or(134400.0);
        set(vm, e, "icon", Value::Num(icon));
        let name = match field(vm, tab, "name") {
            Value::Str(s) => Value::Str(s),
            _ => vm.str(&format!("Tab {}", i + 1)),
        };
        set(vm, e, "name", name);
        let v = vm.str(&cleanup);
        set(vm, e, "tabCleanupConfirmation", v);
        let v = vm.str(&header);
        set(vm, e, "tabNameEditBoxHeader", v);
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num((i + 1) as f64), Value::Table(e));
    }
    Ok(vec![Value::Table(out)])
}

fn global(vm: &mut Vm, name: &str, default: &str) -> String {
    match vm.get_global(name) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => default.to_string(),
    }
}

fn next_cost(vm: &mut Vm) -> f64 {
    let fallback = match open_data(vm) {
        Some(t) => num(field(vm, t, "nextTabCost")),
        None => None,
    };
    let banker = bank(vm).bankers.values().next().copied();
    fallback
        .or_else(|| banker.and_then(|t| num(field(vm, t, "nextTabCost"))))
        .unwrap_or(10000.0)
}

/// `C_Bank.FetchNextPurchasableBankTabData(type)`: the next tab's price
/// and its prompt's texts (recorded).
fn fetch_next_purchasable_bank_tab_data(vm: &mut Vm, c: NativeCall) -> Ret {
    let account = !character(vm, &c);
    // The account's first tab: 100 copper (recorded).
    let cost = if account { 100.0 } else { next_cost(vm) };
    let money = engine(vm).world.character.money;
    let t = vm.new_table();
    set(vm, t, "canAfford", Value::Bool(money >= cost));
    let body = if account {
        "The Account Bank offers storage that is shared with all members in your Account.|n|nDo you wish to purchase this tab?"
    } else {
        "The Bank offers additional storage for your items.|n|nDo you wish to purchase this tab?"
    };
    let v = vm.str(body);
    set(vm, t, "purchasePromptBody", v);
    let confirm = if account {
        "Do you want to purchase an Account Bank tab for:"
    } else {
        "Do you want to purchase a Bank tab for:"
    };
    let v = vm.str(confirm);
    set(vm, t, "purchasePromptConfirmation", v);
    let v = vm.str(if account { "Account Bank" } else { "Bank" });
    set(vm, t, "purchasePromptTitle", v);
    set(vm, t, "tabCost", Value::Num(cost));
    Ok(vec![Value::Table(t)])
}

/// `C_Bank.CanPurchaseBankTab(type)`: the character's at the bank.
fn can_purchase_bank_tab(vm: &mut Vm, c: NativeCall) -> Ret {
    let ok = character(vm, &c) && bank(vm).open.is_some();
    Ok(vec![Value::Bool(ok)])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

/// `C_Bank.PurchaseBankTab(type)`: the server's answer a round trip later.
fn purchase_bank_tab(vm: &mut Vm, c: NativeCall) -> Ret {
    if !character(vm, &c) || bank(vm).open.is_none() {
        return Ok(vec![]);
    }
    let delay = crate::clock::server_delay(vm);
    let native = vm.native("bankTabBought", tab_bought);
    crate::clock::after_engine(vm, delay, native);
    Ok(vec![])
}

/// A tab bought: its settings, the price paid, the tabs changed; its bag.
fn tab_bought(vm: &mut Vm, _c: NativeCall) -> Ret {
    let cost = next_cost(vm);
    let slots = match open_data(vm) {
        Some(t) => num(field(vm, t, "tabSlots")).unwrap_or(98.0),
        None => 98.0,
    };
    let tab = vm.new_table();
    widgets::keep(vm, Value::Table(tab));
    let bag = FIRST_TAB + bank(vm).tabs.len() as i64;
    bank_mut(vm).tabs.push(tab);
    crate::character::place_bag(vm, bag, slots as usize, None);
    fire(vm, "BANK_TAB_SETTINGS_UPDATED", &[Value::Num(CHARACTER)]);
    {
        let money = &mut engine_mut(vm).world.character.money;
        *money = (*money - cost).max(0.0);
    }
    fire(vm, "PLAYER_MONEY", &[]);
    fire(vm, "BANK_TABS_CHANGED", &[Value::Num(CHARACTER)]);
    Ok(vec![])
}

/// `C_Bank.CloseBankFrame()`: `BANKFRAME_CLOSED`, the interaction's hides.
fn close_bank_frame(vm: &mut Vm, _c: NativeCall) -> Ret {
    if bank_mut(vm).open.take().is_none() {
        return Ok(vec![]);
    }
    engine_mut(vm).units.npc = None;
    fire(vm, "BANKFRAME_CLOSED", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(BANKER)],
    );
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(0.0)],
    );
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_Bank.CanUseBank", can_use_bank),
        ("C_Bank.CanViewBank", can_use_bank),
        (
            "C_Bank.AreAnyBankTypesViewable",
            are_any_bank_types_viewable,
        ),
        (
            "C_Bank.FetchNumPurchasedBankTabs",
            fetch_num_purchased_bank_tabs,
        ),
        (
            "C_Bank.FetchPurchasedBankTabData",
            fetch_purchased_bank_tab_data,
        ),
        (
            "C_Bank.FetchNextPurchasableBankTabData",
            fetch_next_purchasable_bank_tab_data,
        ),
        ("C_Bank.CanPurchaseBankTab", can_purchase_bank_tab),
        ("C_Bank.HasMaxBankTabs", no),
        ("C_Bank.FetchBankLockedReason", nothing),
        ("C_Bank.CanDepositMoney", no),
        ("C_Bank.CanWithdrawMoney", no),
        ("C_Bank.PurchaseBankTab", purchase_bank_tab),
        ("C_Bank.CloseBankFrame", close_bank_frame),
    ]
}
