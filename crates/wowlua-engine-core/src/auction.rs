//! The auction house's state (wowlua-engine's `auction` answers from it).

use std::collections::{BTreeMap, VecDeque};

use wowlua_vm::{TableRef, Vm};

use crate::hooks;
use crate::units::UnitId;

pub fn has(vm: &Vm, unit: UnitId) -> bool {
    (hooks::get().auction_has)(vm, unit)
}

pub fn interact(vm: &mut Vm, unit: UnitId) {
    (hooks::get().auction_interact)(vm, unit)
}

/// What a query's answer does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Query {
    Bids,
    Browse,
    MoreBrowse,
    Owned,
    Cancel(f64),
    Commodity(i64),
    Item(i64),
    Price(i64, f64),
    Purchase(i64, f64),
    Post(i64, usize, i64),
}

/// An auction of the player's: its ID, item, buyout, the item's link (the
/// bag's, as posted), its end (`GetTime`).
#[derive(Clone, Debug)]
pub struct Owned {
    pub auction: f64,
    pub item: i64,
    pub buyout: f64,
    pub link: Option<String>,
    pub ends: f64,
}

#[derive(Default)]
pub struct AuctionHouse {
    pub houses: BTreeMap<usize, TableRef>,
    pub open: Option<UnitId>,
    /// The throttled system ready for a message.
    pub ready: bool,
    pub queue: VecDeque<Query>,
    /// The browse results shown (after a browse search).
    pub browsed: bool,
    /// The browse query's search string (lower case; none: everything).
    pub browse_filter: Option<String>,
    /// The browse results all told (`HasFullBrowseResults`).
    pub browse_full: bool,
    /// The player's auctions as the last owned auctions query told them
    /// (`GetNumOwnedAuctions`: none before one).
    pub owned_told: Option<Vec<Owned>>,
    /// Commodities searched: the results' answer (`GetTime`) and owners
    /// known; how many were bought.
    pub searched: BTreeMap<i64, (f64, bool)>,
    /// Commodities ever searched: the first search's answer (`GetTime`),
    /// whether their owners' names came.
    pub first_searched: BTreeMap<i64, (f64, bool)>,
    pub bought: BTreeMap<i64, f64>,
    /// Items searched (their results told).
    pub item_searched: BTreeMap<i64, bool>,
    /// The player's auctions.
    pub owned: Vec<Owned>,
    pub next_auction: f64,
    /// The item being posted: its buyout, its duration (seconds), its
    /// link.
    pub posting: (f64, f64, Option<String>),
    /// A purchase's quote's end (`GetTime`).
    pub quote_until: f64,
}
