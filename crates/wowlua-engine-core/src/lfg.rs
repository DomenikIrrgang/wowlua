//! The group finder's state (wowlua-engine's `lfg` answers from it).

use std::collections::BTreeMap;

use crate::widgets::WidgetId;

/// The player's own listing.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub activity_ids: Vec<f64>,
    /// When it was made (`GetTime`).
    pub created: f64,
    /// Its description: the listing panel's comment box's text (the
    /// client's, `StoreSecureReference("LFGListingComment", editBox)`).
    pub comment: String,
    pub auto_accept: bool,
    pub private_group: bool,
    pub new_player_friendly: bool,
    pub cross_faction: bool,
    pub general_playstyle: f64,
    pub voice_mode: f64,
    pub required_item_level: f64,
}

/// A player of another group's or player's listing.
#[derive(Clone, Debug, Default)]
pub struct Member {
    pub name: String,
    pub class_name: String,
    pub class_filename: String,
    pub level: f64,
    pub area_name: String,
    /// `TANK`, `HEALER`, `DAMAGER` or `NONE`.
    pub assigned_role: String,
    pub tank: bool,
    pub healer: bool,
    pub dps: bool,
}

/// Another group's or player's listing, as the server has it; its
/// `searchResultID` is given by each search that finds it.
#[derive(Clone, Debug, Default)]
pub struct SearchResult {
    pub id: f64,
    /// The server's listing it is (`add_listing`'s handle; 0 the
    /// player's own).
    pub handle: u32,
    /// The player's own listing (`hasSelf`).
    pub has_self: bool,
    pub activity_ids: Vec<f64>,
    pub name: String,
    pub comment: String,
    pub voice_chat: String,
    pub voice_mode: f64,
    /// Seconds listed (unverified whether it grows between searches).
    pub age: f64,
    pub auto_accept: bool,
    pub delisted: bool,
    pub general_playstyle: f64,
    pub new_player_friendly: bool,
    /// The leader first.
    pub members: Vec<Member>,
}

/// What a request's answer, a round trip later, does.
#[derive(Clone, Debug)]
pub enum Answer {
    Search { category: f64, activities: Vec<f64> },
    Created(Listing),
    Updated(Listing),
    Removed,
}

#[derive(Default)]
pub struct Lfg {
    pub listing: Option<Listing>,
    /// The server's other listings (none unless a test adds them).
    pub listings: Vec<SearchResult>,
    /// The last search's answer: its results, numbered.
    pub results: Vec<SearchResult>,
    /// The last result's number (each search's after the last's).
    pub next_result: f64,
    /// The last listing's handle (`add_listing`'s).
    pub next_handle: u32,
    /// The next search fails (`fail_next_search`), with its reason.
    pub fail_search: Option<Option<String>>,
    /// Requests awaiting their answer, by number.
    pub pending: BTreeMap<u32, Answer>,
    pub next_request: u32,
    /// `SaveLanguageSearchFilter`'s locales (none saved: recorded `{}`).
    pub languages: Vec<String>,
    /// `StoreSecureReference`'s edit boxes, by name.
    pub secure_refs: BTreeMap<String, WidgetId>,
    /// `C_LFGListRoles.SetRoles`' tank, healer and damage.
    pub roles: [bool; 3],
}
