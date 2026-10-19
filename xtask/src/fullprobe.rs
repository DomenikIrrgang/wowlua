//! `cargo xtask fullprobe-plan`: the plan of `/stdtest engine fullprobe`
//! (tests/ingame/src/fullprobe.wowlua), every C function of camelot's Lua
//! environment (`data/camelot-functions.txt`) called with many arguments,
//! into `tests/ingame/src/fullprobeplan.lua`.
//!
//! Each function is classified first (user decision 2026-10-19, "safe and
//! restored settings"): queries are called; settings with a read-back and a
//! restore (the curated [`setters`]) are set, read back and restored; the
//! rest is skipped with its reason (protected, restricted, an action that
//! spends, moves, casts, targets, joins, sends, accepts or changes the
//! character, or not named as a query).
//!
//! A documented query's calls ("full matrix"): without arguments; the
//! baseline (each argument's typical value); each argument through its
//! values (every enum value, the boundary numbers 0, 1, -1, 2^31-1, 1.5,
//! unit tokens, the character's own IDs as placeholders such as `{SPELL1}`
//! that StdTest resolves in game) with the others at the baseline, or every
//! combination when they are few; each argument of a type it does not take
//! (the usage errors: only values the argument check refuses, never a
//! numeric string for a number). At most [`CAP`] calls a function. An
//! undocumented query gets a few generic calls; a server request
//! (`Request*`) only its baseline and usage errors.

use anyhow::{Context, Result, anyhow};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::Path;

/// The plan StdTest loads.
pub const PLAN: &str = "tests/ingame/src/fullprobeplan.lua";

/// The most calls one function gets.
pub const CAP: usize = 60;

/// A product of the arguments' values up to this is called in full.
const FULL_PRODUCT: usize = 48;

/// The boundary numbers every number argument gets.
const BOUNDARY: &[&str] = &["1", "0", "-1", "2147483647", "1.5"];

/// Unit tokens (and the player's GUID, which some take).
const UNITS: &[&str] = &[
    "\"player\"",
    "\"target\"",
    "\"pet\"",
    "\"party1\"",
    "\"none\"",
    "UnitGUID(\"player\")",
];

/// The documentation, as the generator needs it.
pub struct Docs {
    /// Global and namespaced functions (`C_Spell.GetSpellInfo`).
    pub functions: BTreeMap<String, Json>,
    /// Enumerations: their values, ascending.
    pub enums: HashMap<String, Vec<i64>>,
    /// Structures: their fields.
    pub structures: HashMap<String, Vec<Json>>,
    /// Type aliases of the declarations (`type ClubId = string`).
    pub aliases: HashMap<String, String>,
    /// Callback types (functions).
    pub callbacks: HashSet<String>,
}

impl Docs {
    pub fn load(root: &Path) -> Result<Docs> {
        let path = root.join("crates/wowlua-api/data/camelot.api.json.gz");
        let gz = std::fs::read(&path).with_context(|| path.display().to_string())?;
        let mut text = Vec::new();
        flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut text)?;
        let data: Json = serde_json::from_slice(&text)?;
        let mut docs = Docs {
            functions: BTreeMap::new(),
            enums: HashMap::new(),
            structures: HashMap::new(),
            aliases: HashMap::new(),
            callbacks: HashSet::new(),
        };
        for system in data
            .get("systems")
            .and_then(Json::as_array)
            .ok_or_else(|| anyhow!("no systems in {}", path.display()))?
        {
            for t in system
                .get("Tables")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                let name = t.get("Name").and_then(Json::as_str).unwrap_or("");
                let fields = || {
                    t.get("Fields")
                        .and_then(Json::as_array)
                        .cloned()
                        .unwrap_or_default()
                };
                match t.get("Type").and_then(Json::as_str) {
                    Some("Enumeration") => {
                        let mut values: Vec<i64> = fields()
                            .iter()
                            .filter_map(|f| f.get("EnumValue").and_then(Json::as_i64))
                            .collect();
                        values.sort_unstable();
                        values.dedup();
                        docs.enums.insert(name.to_string(), values);
                    }
                    Some("Structure") => {
                        docs.structures.insert(name.to_string(), fields());
                    }
                    Some("CallbackType") => {
                        docs.callbacks.insert(name.to_string());
                    }
                    _ => {}
                }
            }
            if system.get("Type").and_then(Json::as_str) == Some("ScriptObject") {
                continue;
            }
            let namespace = system.get("Namespace").and_then(Json::as_str);
            for f in system
                .get("Functions")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                let Some(short) = f.get("Name").and_then(Json::as_str) else {
                    continue;
                };
                let name = match namespace {
                    Some(ns) => format!("{ns}.{short}"),
                    None => short.to_string(),
                };
                docs.functions.insert(name, f.clone());
            }
        }
        for file in ["shared.d.wowlua", "camelot.d.wowlua"] {
            let path = root.join("crates/wowlua-api/data").join(file);
            let text =
                std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
            for line in text.lines() {
                if let Some((name, ty)) = line
                    .strip_prefix("type ")
                    .and_then(|rest| rest.split_once(" = "))
                {
                    docs.aliases.insert(name.to_string(), ty.to_string());
                }
            }
        }
        Ok(docs)
    }
}

/// The C functions the client has (`data/camelot-functions.txt`).
fn client_functions(root: &Path) -> Result<Vec<String>> {
    let path = root.join("crates/wowlua-engine/data/camelot-functions.txt");
    let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    Ok(text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect())
}

/// A `pub const NAME: &[&str] = &[ … ];` list of the engine's sources.
fn engine_list(root: &Path, file: &str, name: &str) -> Result<Vec<String>> {
    let path = root.join("crates/wowlua-engine/src").join(file);
    let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let start = text
        .find(&format!("pub const {name}: &[&str] = &["))
        .ok_or_else(|| anyhow!("no {name} in {}", path.display()))?;
    let rest = &text[start..];
    let end = rest.find("];").unwrap_or(rest.len());
    let quoted = regex::Regex::new(r#""([^"]+)""#).expect("a valid pattern");
    Ok(quoted
        .captures_iter(&rest[..end])
        .map(|c| c[1].to_string())
        .collect())
}

/// A name's words: `GetSpellInfo` → Get, Spell, Info; `EJ_GetTier` → EJ,
/// Get, Tier; `GUIDIsPlayer` → GUID, Is, Player.
pub fn words(name: &str) -> Vec<String> {
    let re = regex::Regex::new(r"[A-Z]+(?:[a-z0-9]+)?|[a-z0-9]+").expect("a valid pattern");
    let mut out = Vec::new();
    for m in re.find_iter(name) {
        let w = m.as_str();
        // `GUIDIs`: an upper-case run before a capitalised word.
        let upper = w.bytes().take_while(u8::is_ascii_uppercase).count();
        if upper > 1 && upper < w.len() {
            out.push(w[..upper - 1].to_string());
            out.push(w[upper - 1..].to_string());
        } else {
            out.push(w.to_string());
        }
    }
    out
}

/// Words a query's name starts with.
const QUERY_FIRST: &[&str] = &[
    "Get",
    "Is",
    "Has",
    "Can",
    "Should",
    "Are",
    "Does",
    "Was",
    "Were",
    "Find",
    "In",
    "Calculate",
    "Format",
    "Abbreviate",
    "Ambiguate",
    "Compare",
    "Escape",
    "Strip",
    "Round",
    "Decode",
    "Encode",
    "Serialize",
    "Deserialize",
    "Parse",
    "Evaluate",
    "Supports",
    "Showing",
    "Cursor",
    "Any",
    "Have",
    "Needs",
    "Closest",
    "Classic",
    "Anti",
    "Unit",
    "To",
    "Break",
];

/// Words that make a name a query after a noun (`PetIsHurt`,
/// `EventCanEdit`, `QuestGetAutoAccept`).
const QUERY_LATER: &[&str] = &[
    "Get",
    "Is",
    "Has",
    "Can",
    "Should",
    "Are",
    "Does",
    "Exists",
    "Enabled",
    "Supported",
    "Meets",
    "Contains",
    "Ignores",
    "Uses",
    "Connected",
];

/// Words that make a name an action wherever they stand before its query
/// word (or anywhere in a `Unit…`/`In…` name): what the probe never does.
const ACTION_WORDS: &[&str] = &[
    "Set",
    "Switch",
    "Clear",
    "Reset",
    "Remove",
    "Delete",
    "Del",
    "Destroy",
    "Accept",
    "Decline",
    "Confirm",
    "Purchase",
    "Buy",
    "Sell",
    "Use",
    "Cast",
    "Invite",
    "Uninvite",
    "Kick",
    "Leave",
    "Join",
    "Logout",
    "Quit",
    "Reload",
    "Send",
    "Report",
    "Abandon",
    "Learn",
    "Unlearn",
    "Equip",
    "Pickup",
    "Drop",
    "Place",
    "Swap",
    "Split",
    "Release",
    "Summon",
    "Dismiss",
    "Start",
    "Stop",
    "Toggle",
    "Add",
    "Cancel",
    "Close",
    "Open",
    "Show",
    "Hide",
    "Select",
    "Save",
    "Commit",
    "Apply",
    "Assign",
    "Take",
    "Claim",
    "Complete",
    "Promote",
    "Demote",
    "Respond",
    "Rename",
    "Upgrade",
    "Activate",
    "Deactivate",
    "Enable",
    "Disable",
    "Create",
    "Move",
    "Target",
    "Focus",
    "Attack",
    "Follow",
    "Ban",
    "Unban",
    "Mute",
    "Ignore",
    "Squelch",
    "Vote",
    "Loot",
    "Roll",
    "Bid",
    "Post",
    "Deposit",
    "Withdraw",
    "Repair",
    "Trade",
    "Craft",
    "Train",
    "Queue",
    "Enter",
    "Exit",
    "Teleport",
    "Mount",
    "Dismount",
    "Request",
    "Play",
    "Run",
    "Exec",
    "Execute",
    "Load",
    "Unload",
    "Register",
    "Unregister",
    "Update",
    "Refresh",
    "Search",
    "Query",
    "Sort",
    "Expand",
    "Collapse",
    "Submit",
    "Acknowledge",
    "Mark",
    "Notify",
    "Unlock",
    "Lock",
];

/// Words a server request (`Request…`) must not have: it would act.
const REQUEST_DENY: &[&str] = &[
    "Invite",
    "Join",
    "Leave",
    "Accept",
    "Decline",
    "Membership",
    "Purchase",
    "Trade",
    "Duel",
    "Release",
    "Resurrect",
    "Summon",
    "Kick",
    "Vote",
    "Report",
    "Ticket",
    "Loot",
    "Roll",
    "Rename",
    "Delete",
    "Start",
    "Stop",
    "Cancel",
    "Use",
    "Landing",
    "Reward",
    "Respec",
    "Port",
    "Teleport",
    "Ready",
    "Check",
    "Vehicle",
    "Exit",
    "Apply",
    "New",
    "Creation",
    "Checkout",
    "Refund",
    "Reroll",
    "Reset",
    "Bottom",
    "Session",
];

/// Functions named as queries that act (with the reason).
const ACTING_QUERIES: &[(&str, &str)] = &[
    (
        "GetQuestReward",
        "acts: completes the quest and takes its reward",
    ),
    ("GetInboxText", "acts: marks the mail read"),
    ("UnitSetRole", "acts: sets the group role"),
    ("UnitSetRoleEnum", "acts: sets the group role"),
    ("UnitSwitchToVehicleSeat", "acts: moves the character"),
    (
        "C_AddOns.EnableAddOn",
        "takes effect on reload: a crash would leave it changed",
    ),
    (
        "C_AddOns.DisableAddOn",
        "takes effect on reload: a crash would leave StdTest disabled",
    ),
];

/// Pure helpers that are not named as queries.
const PURE: &[&str] = &[
    "AbbreviateLargeNumbers",
    "AbbreviateNumbers",
    "BreakUpLargeNumbers",
    "Ambiguate",
    "C_ColorUtil.ConvertHSLToHSV",
    "C_ColorUtil.ConvertHSVToHSL",
    "C_ColorUtil.ConvertHSVToRGB",
    "C_ColorUtil.ConvertRGBToHSV",
    "ContainerIDToInventoryID",
    "TaxiNodeCost",
    "TaxiNodeName",
    "TaxiNodePosition",
    "TaxiNodeGetType",
    "GMEuropaBugsEnabled",
    "GMEuropaComplaintsEnabled",
    "GMEuropaSuggestionsEnabled",
    "GMEuropaTicketsEnabled",
    "GMItemRestorationButtonEnabled",
    "GMQuickTicketSystemEnabled",
    "GMQuickTicketSystemThrottled",
    "BNConnected",
    "BNFeaturesEnabled",
    "BNFeaturesEnabledAndConnected",
];

/// What the probe does with a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A query: the full matrix.
    Query,
    /// A server request: its baseline and usage errors only.
    Request,
    /// Set, read back and restored (a curated setter).
    Setter,
    Skipped(String),
}

/// The function's class.
pub fn classify(
    name: &str,
    doc: Option<&Json>,
    protected: &HashSet<String>,
    setters: &HashSet<String>,
) -> Kind {
    let short = name.rsplit('.').next().unwrap_or(name);
    if short.starts_with(|c: char| c.is_ascii_lowercase())
        || ["string.", "table.", "math.", "bit.", "coroutine."]
            .iter()
            .any(|p| name.starts_with(p))
    {
        return Kind::Skipped(
            "a Lua or WoW language function (the conformance suite probes these)".into(),
        );
    }
    if let Some((_, why)) = ACTING_QUERIES.iter().find(|(n, _)| *n == name) {
        return Kind::Skipped((*why).to_string());
    }
    let flag = |key: &str| doc.and_then(|d| d.get(key)).and_then(Json::as_bool) == Some(true);
    if protected.contains(name) || flag("IsProtectedFunction") {
        return Kind::Skipped("protected".into());
    }
    if flag("HasRestrictions") {
        return Kind::Skipped("restricted (HasRestrictions)".into());
    }
    if doc
        .and_then(|d| d.get("Environment"))
        .and_then(Json::as_str)
        == Some("SecureOnly")
    {
        return Kind::Skipped("Blizzard's secure code only".into());
    }
    if setters.contains(name) {
        return Kind::Setter;
    }
    if PURE.contains(&name) {
        return Kind::Query;
    }
    let w = words(short);
    let first = w.first().map(String::as_str).unwrap_or("");
    let acts = |list: &[String]| list.iter().any(|x| ACTION_WORDS.contains(&x.as_str()));
    if first == "Request" {
        if w.iter().any(|x| REQUEST_DENY.contains(&x.as_str())) {
            return Kind::Skipped("acts: a request that joins, buys or answers".into());
        }
        return Kind::Request;
    }
    if QUERY_FIRST.contains(&first) {
        // `Unit…`, `In…`, `To…`, `Cursor…` name a subject, not a question:
        // any action word makes them act (`UnitSetRole`).
        let subject = ["Unit", "In", "To", "Cursor", "Any", "Closest"].contains(&first);
        if !subject || !acts(&w[1..]) {
            return Kind::Query;
        }
    } else if let Some(at) = w
        .iter()
        .position(|x| QUERY_LATER.contains(&x.as_str()))
        .filter(|&at| at > 0 && at <= 3)
        && !acts(&w[..at])
    {
        return Kind::Query;
    }
    if first == "Set" {
        return Kind::Skipped("a setter without a curated read-back and restore".into());
    }
    if doc.is_none() {
        return Kind::Skipped("undocumented and not named as a query".into());
    }
    Kind::Skipped("not named as a query (an action or a state change)".into())
}

/// A curated setting: its read-back (a call), its restore (a Lua function
/// given the read-back's values), the setting calls.
pub struct Setter {
    pub get: String,
    pub restore: String,
    pub sets: Vec<String>,
}

/// CVars set and restored: the UI's own, one secure (refused), one the
/// client lacks, a number.
const CVARS: &[&str] = &[
    "autoLootDefault",
    "autoQuestWatch",
    "chatBubbles",
    "chatBubblesParty",
    "lockActionBars",
    "showTargetOfTarget",
    "countdownForCooldowns",
    "profanityFilter",
    "nameplateShowEnemies",
    "cameraDistanceMaxZoomFactor",
    "ActionButtonUseKeyDown",
    "stdtestNoSuchCVar",
];

/// The curated settings (each read back and restored).
pub fn setters(docs: &Docs) -> Vec<Setter> {
    let mut out = Vec::new();
    for cvar in CVARS {
        let set = |v: &str| format!("C_CVar.SetCVar(\"{cvar}\"{v})");
        out.push(Setter {
            get: format!("C_CVar.GetCVar(\"{cvar}\")"),
            restore: format!("function(o) C_CVar.SetCVar(\"{cvar}\", o) end"),
            sets: vec![
                set(", \"0\""),
                set(", \"1\""),
                set(", \"2\""),
                set(", \"\""),
                set(", \"abc\""),
                set(", \"1.5\""),
                set(", 1"),
                set(", true"),
                set(""),
                set(", {}"),
            ],
        });
    }
    out.push(Setter {
        get: "C_CVar.GetCVar(\"closedInfoFrames\")".into(),
        restore: "function(o) C_CVar.SetCVar(\"closedInfoFrames\", o) end".into(),
        sets: [
            "1, true",
            "2, true",
            "1, false",
            "0, true",
            "64, true",
            "1000, true",
            "-1, true",
            "1",
        ]
        .iter()
        .map(|a| format!("C_CVar.SetCVarBitfield(\"closedInfoFrames\", {a})"))
        .collect(),
    });
    // A boolean setting: getter and setter without arguments.
    for (get, set) in [
        (
            "C_Container.GetSortBagsRightToLeft",
            "C_Container.SetSortBagsRightToLeft",
        ),
        (
            "C_Container.GetInsertItemsLeftToRight",
            "C_Container.SetInsertItemsLeftToRight",
        ),
        (
            "C_Container.GetBackpackAutosortDisabled",
            "C_Container.SetBackpackAutosortDisabled",
        ),
        (
            "C_Container.GetBackpackSellJunkDisabled",
            "C_Container.SetBackpackSellJunkDisabled",
        ),
        (
            "C_Container.GetBankAutosortDisabled",
            "C_Container.SetBankAutosortDisabled",
        ),
        ("C_ToyBox.GetCollectedShown", "C_ToyBox.SetCollectedShown"),
        (
            "C_ToyBox.GetUncollectedShown",
            "C_ToyBox.SetUncollectedShown",
        ),
        ("C_ToyBox.GetUnusableShown", "C_ToyBox.SetUnusableShown"),
        (
            "C_Heirloom.GetCollectedHeirloomFilter",
            "C_Heirloom.SetCollectedHeirloomFilter",
        ),
        (
            "C_Heirloom.GetUncollectedHeirloomFilter",
            "C_Heirloom.SetUncollectedHeirloomFilter",
        ),
        (
            "C_TransmogCollection.GetCollectedShown",
            "C_TransmogCollection.SetCollectedShown",
        ),
        (
            "C_TransmogCollection.GetUncollectedShown",
            "C_TransmogCollection.SetUncollectedShown",
        ),
        (
            "C_TransmogCollection.GetAllFactionsShown",
            "C_TransmogCollection.SetAllFactionsShown",
        ),
        (
            "C_TransmogCollection.GetAllRacesShown",
            "C_TransmogCollection.SetAllRacesShown",
        ),
        ("GetOptOutOfLoot", "SetOptOutOfLoot"),
        ("GetAutoDeclineGuildInvites", "SetAutoDeclineGuildInvites"),
        ("GetAllowLowLevelRaid", "SetAllowLowLevelRaid"),
        ("GetGuildRosterShowOffline", "SetGuildRosterShowOffline"),
        // What other players see, put back at once (user decision
        // 2026-10-20: probed as restorable settings).
        ("ShowingHelm", "ShowHelm"),
        ("ShowingCloak", "ShowCloak"),
    ] {
        out.push(Setter {
            get: format!("{get}()"),
            restore: format!("function(o) {set}(o) end"),
            sets: ["true", "false", "", "0", "\"x\""]
                .iter()
                .map(|v| format!("{set}({v})"))
                .collect(),
        });
    }
    // A boolean setting by index.
    for (get, set) in [
        (
            "C_MountJournal.GetCollectedFilterSetting",
            "C_MountJournal.SetCollectedFilterSetting",
        ),
        (
            "C_MountJournal.IsSourceChecked",
            "C_MountJournal.SetSourceFilter",
        ),
        (
            "C_MountJournal.IsTypeChecked",
            "C_MountJournal.SetTypeFilter",
        ),
        (
            "C_PetJournal.IsFilterChecked",
            "C_PetJournal.SetFilterChecked",
        ),
        (
            "C_PetJournal.IsPetSourceChecked",
            "C_PetJournal.SetPetSourceChecked",
        ),
        (
            "C_PetJournal.IsPetTypeChecked",
            "C_PetJournal.SetPetTypeFilter",
        ),
        (
            "C_ToyBox.IsSourceTypeFilterChecked",
            "C_ToyBox.SetSourceTypeFilter",
        ),
        (
            "C_ToyBox.IsExpansionTypeFilterChecked",
            "C_ToyBox.SetExpansionTypeFilter",
        ),
        (
            "C_Heirloom.GetHeirloomSourceFilter",
            "C_Heirloom.SetHeirloomSourceFilter",
        ),
        (
            "C_TransmogCollection.IsSourceTypeFilterChecked",
            "C_TransmogCollection.SetSourceTypeFilter",
        ),
    ] {
        for i in ["1", "2", "0", "99"] {
            out.push(Setter {
                get: format!("{get}({i})"),
                restore: format!("function(o) {set}({i}, o) end"),
                sets: ["true", "false"]
                    .iter()
                    .map(|v| format!("{set}({i}, {v})"))
                    .collect(),
            });
        }
    }
    // The bags' flags (the documentation's `BagSlotFlags`).
    let flags = docs.enums.get("BagSlotFlags").cloned().unwrap_or_default();
    for bag in ["1", "0"] {
        for flag in flags.iter().take(8) {
            out.push(Setter {
                get: format!("C_Container.GetBagSlotFlag({bag}, {flag})"),
                restore: format!("function(o) C_Container.SetBagSlotFlag({bag}, {flag}, o) end"),
                sets: ["true", "false"]
                    .iter()
                    .map(|v| format!("C_Container.SetBagSlotFlag({bag}, {flag}, {v})"))
                    .collect(),
            });
        }
    }
    let one = |get: &str, restore: &str, sets: &[&str]| Setter {
        get: get.to_string(),
        restore: restore.to_string(),
        sets: sets.iter().map(|s| (*s).to_string()).collect(),
    };
    out.push(one(
        "C_PetJournal.GetPetSortParameter()",
        "function(o) C_PetJournal.SetPetSortParameter(o) end",
        &[
            "C_PetJournal.SetPetSortParameter(1)",
            "C_PetJournal.SetPetSortParameter(2)",
            "C_PetJournal.SetPetSortParameter(3)",
            "C_PetJournal.SetPetSortParameter(4)",
            "C_PetJournal.SetPetSortParameter(0)",
            "C_PetJournal.SetPetSortParameter(99)",
            "C_PetJournal.SetPetSortParameter(\"x\")",
        ],
    ));
    out.push(one(
        "C_PetJournal.GetSearchFilter()",
        "function(o) C_PetJournal.SetSearchFilter(o or \"\") end",
        &[
            "C_PetJournal.SetSearchFilter(\"a\")",
            "C_PetJournal.SetSearchFilter(\"\")",
            "C_PetJournal.SetSearchFilter(\"zzzz\")",
            "C_PetJournal.SetSearchFilter({})",
        ],
    ));
    out.push(one(
        "C_TransmogCollection.GetClassFilter()",
        "function(o) C_TransmogCollection.SetClassFilter(o) end",
        &[
            "C_TransmogCollection.SetClassFilter({CLASSID})",
            "C_TransmogCollection.SetClassFilter(1)",
            "C_TransmogCollection.SetClassFilter(0)",
            "C_TransmogCollection.SetClassFilter(99)",
            "C_TransmogCollection.SetClassFilter(\"x\")",
        ],
    ));
    out.push(one(
        "C_Heirloom.GetClassAndSpecFilters()",
        "function(c, s) C_Heirloom.SetClassAndSpecFilters(c, s) end",
        &[
            "C_Heirloom.SetClassAndSpecFilters({CLASSID}, 0)",
            "C_Heirloom.SetClassAndSpecFilters(1, 0)",
            "C_Heirloom.SetClassAndSpecFilters(0, 0)",
            "C_Heirloom.SetClassAndSpecFilters(99, 99)",
            "C_Heirloom.SetClassAndSpecFilters(\"x\", 0)",
        ],
    ));
    out.push(one(
        "C_QuestLog.GetSelectedQuest()",
        "function(o) C_QuestLog.SetSelectedQuest(o or 0) end",
        &[
            "C_QuestLog.SetSelectedQuest({QUEST1})",
            "C_QuestLog.SetSelectedQuest({QUEST2})",
            "C_QuestLog.SetSelectedQuest(0)",
            "C_QuestLog.SetSelectedQuest(-1)",
            "C_QuestLog.SetSelectedQuest(\"x\")",
        ],
    ));
    out.push(one(
        "C_SuperTrack.GetSuperTrackedQuestID()",
        "function(o) C_SuperTrack.SetSuperTrackedQuestID(o or 0) end",
        &[
            "C_SuperTrack.SetSuperTrackedQuestID({QUEST1})",
            "C_SuperTrack.SetSuperTrackedQuestID(0)",
            "C_SuperTrack.SetSuperTrackedQuestID(-1)",
            "C_SuperTrack.SetSuperTrackedQuestID(\"x\")",
        ],
    ));
    out.push(one(
        "C_QuestLog.GetQuestWatchType({QUEST1})",
        "function(o) if o == nil then C_QuestLog.RemoveQuestWatch({QUEST1}) else C_QuestLog.AddQuestWatch({QUEST1}, o) end end",
        &[
            "C_QuestLog.AddQuestWatch({QUEST1})",
            "C_QuestLog.RemoveQuestWatch({QUEST1})",
            "C_QuestLog.AddQuestWatch({QUEST1}, 0)",
            "C_QuestLog.AddQuestWatch({QUEST1}, 1)",
            "C_QuestLog.AddQuestWatch(0)",
            "C_QuestLog.AddQuestWatch(\"x\")",
        ],
    ));
    out.push(one(
        "C_Reputation.GetWatchedFactionData()",
        "function(o) C_Reputation.SetWatchedFactionByID(o and o.factionID or 0) end",
        &[
            "C_Reputation.SetWatchedFactionByID({FACTION1})",
            "C_Reputation.SetWatchedFactionByID(0)",
            "C_Reputation.SetWatchedFactionByID(-1)",
            "C_Reputation.SetWatchedFactionByID(\"x\")",
            "C_Reputation.SetWatchedFactionByIndex({FACTIONINDEX1})",
            "C_Reputation.SetWatchedFactionByIndex(0)",
        ],
    ));
    out.push(one(
        "C_Reputation.GetSelectedFaction()",
        "function(o) C_Reputation.SetSelectedFaction(o or 0) end",
        &[
            "C_Reputation.SetSelectedFaction(1)",
            "C_Reputation.SetSelectedFaction(2)",
            "C_Reputation.SetSelectedFaction(0)",
            "C_Reputation.SetSelectedFaction(\"x\")",
        ],
    ));
    out.push(one(
        "C_CurrencyInfo.GetCurrencyInfo({CURRENCY1})",
        "function(o) if o then C_CurrencyInfo.SetCurrencyBackpackByID({CURRENCY1}, o.isShowInBackpack) end end",
        &[
            "C_CurrencyInfo.SetCurrencyBackpackByID({CURRENCY1}, true)",
            "C_CurrencyInfo.SetCurrencyBackpackByID({CURRENCY1}, false)",
            "C_CurrencyInfo.SetCurrencyBackpackByID(0, true)",
            "C_CurrencyInfo.SetCurrencyBackpackByID(\"x\", true)",
        ],
    ));
    out.push(one(
        "C_Calendar.GetMonthInfo()",
        "function(o) if o then C_Calendar.SetAbsMonth(o.month, o.year) end end",
        &[
            "C_Calendar.SetMonth(1)",
            "C_Calendar.SetMonth(-1)",
            "C_Calendar.SetMonth(0)",
            "C_Calendar.SetMonth(\"x\")",
            "C_Calendar.SetAbsMonth(1, 2025)",
            "C_Calendar.SetAbsMonth(13, 2025)",
            "C_Calendar.SetAbsMonth(0, 0)",
            "C_Calendar.SetAbsMonth(\"x\", 2025)",
        ],
    ));
    out.push(one(
        "C_FriendList.GetSelectedFriend()",
        "function(o) C_FriendList.SetSelectedFriend(o or 0) end",
        &[
            "C_FriendList.SetSelectedFriend(1)",
            "C_FriendList.SetSelectedFriend(0)",
            "C_FriendList.SetSelectedFriend(-1)",
            "C_FriendList.SetSelectedFriend(\"x\")",
        ],
    ));
    out.push(one(
        "C_FriendList.GetSelectedIgnore()",
        "function(o) C_FriendList.SetSelectedIgnore(o or 0) end",
        &[
            "C_FriendList.SetSelectedIgnore(1)",
            "C_FriendList.SetSelectedIgnore(0)",
            "C_FriendList.SetSelectedIgnore(\"x\")",
        ],
    ));
    out.push(one(
        "GetActionBarToggles()",
        "function(...) SetActionBarToggles(...) end",
        &[
            "SetActionBarToggles(true, true, true, true, true)",
            "SetActionBarToggles(false, false, false, false, false)",
            "SetActionBarToggles(true)",
            "SetActionBarToggles()",
        ],
    ));
    // The title other players see, put back at once (user decision
    // 2026-10-20). `{TITLE1}`: a title the character knows.
    out.push(one(
        "GetCurrentTitle()",
        "function(o) SetCurrentTitle(o) end",
        &[
            "SetCurrentTitle({TITLE1})",
            "SetCurrentTitle(0)",
            "SetCurrentTitle(-1)",
            "SetCurrentTitle(99999)",
            "SetCurrentTitle(\"x\")",
        ],
    ));
    out.push(one(
        "C_Map.GetUserWaypoint()",
        "function(o) if o then C_Map.SetUserWaypoint(o) else C_Map.ClearUserWaypoint() end end",
        &[
            "C_Map.SetUserWaypoint(UiMapPoint.CreateFromCoordinates({MAP1}, 0.5, 0.5))",
            "C_Map.SetUserWaypoint(UiMapPoint.CreateFromCoordinates({MAP1}, 2, 2))",
            "C_Map.SetUserWaypoint(UiMapPoint.CreateFromCoordinates(0, 0.5, 0.5))",
            "C_Map.SetUserWaypoint(\"x\")",
            "C_Map.ClearUserWaypoint()",
        ],
    ));
    out
}

/// The functions the curated settings call.
fn setter_functions(list: &[Setter]) -> HashSet<String> {
    let name = regex::Regex::new(r"^([A-Za-z_][A-Za-z0-9_.]*)\(").expect("a valid pattern");
    let mut out = HashSet::new();
    for s in list {
        for set in &s.sets {
            if let Some(c) = name.captures(set) {
                out.insert(c[1].to_string());
            }
        }
    }
    out
}

/// An argument's values (Lua sources; placeholders `{NAME}` StdTest
/// resolves), the first its baseline, and a value of a type it does not
/// take (none for a boolean or `any`: they take every value).
#[derive(Debug, Default)]
pub struct Pool {
    pub values: Vec<String>,
    pub wrong: Option<String>,
}

fn push(values: &mut Vec<String>, v: impl Into<String>) {
    let v = v.into();
    if !values.contains(&v) {
        values.push(v);
    }
}

/// The base of a type, through the aliases: `number`, `string`, `boolean`,
/// `unit`, `guid`, an enum's or structure's name, a mixin, a widget, `any`.
fn base(docs: &Docs, ty: &str, depth: usize) -> String {
    let builtin = match ty {
        "number" | "luaIndex" | "BigUInteger" | "BigInteger" | "WOWMONEY" | "fileID"
        | "normalizedValue" | "SingleColorValue" | "uiFontHeight" | "Seconds" | "time_t"
        | "FrameTime" | "DurationSeconds" | "size" | "uiUnit" | "LuaInventorySlot" => {
            Some("number")
        }
        "bool" | "boolean" => Some("boolean"),
        "cstring" | "string" | "stringView" | "kstring" => Some("string"),
        "UnitToken" | "UnitId" => Some("unit"),
        "WOWGUID" => Some("guid"),
        "table" | "LuaValueVariant" => Some("table"),
        "any" | "LuaValueReference" => Some("any"),
        "ItemLocation" | "ItemLocationMixin" => Some("itemloc"),
        "PlayerLocation" | "PlayerLocationMixin" => Some("playerloc"),
        "SpellIdentifier" => Some("spellid"),
        "ItemInfo" | "IDOrLink" => Some("iteminfo"),
        "colorRGB" | "colorRGBA" | "ColorMixin" => Some("color"),
        "vector2" | "Vector2DMixin" => Some("vector2"),
        "function" | "luaFunction" => Some("function"),
        _ => None,
    };
    if let Some(b) = builtin {
        return b.to_string();
    }
    if docs.enums.contains_key(ty) || docs.structures.contains_key(ty) {
        return ty.to_string();
    }
    if docs.callbacks.contains(ty) {
        return "function".into();
    }
    if depth < 6
        && let Some(alias) = docs.aliases.get(ty)
    {
        let alias = alias.trim();
        if alias.starts_with('(') {
            return "function".into();
        }
        // A union of string literals (`DrawLayer`): kept as it is.
        if alias.contains('"') {
            return format!("literals:{ty}");
        }
        let first = alias.split('|').next().unwrap_or(alias).trim();
        return base(docs, first, depth + 1);
    }
    // Widgets (`SimpleFrame`, `ScriptRegion`, `GameTooltip`…).
    if ty.starts_with("Simple")
        || ty.ends_with("Frame")
        || ty.starts_with("Script")
        || [
            "Region",
            "Tooltip",
            "Texture",
            "FontString",
            "Frame",
            "Object",
        ]
        .contains(&ty)
    {
        return "widget".into();
    }
    "any".into()
}

/// The string literals of a union alias (`DrawLayer`), at most 12.
fn literals(docs: &Docs, ty: &str) -> Vec<String> {
    let lit = regex::Regex::new(r#""([^"]*)""#).expect("a valid pattern");
    docs.aliases
        .get(ty)
        .map(|a| {
            lit.captures_iter(a)
                .take(12)
                .map(|c| format!("{:?}", &c[1]))
                .collect()
        })
        .unwrap_or_default()
}

/// The character's own IDs a number argument takes, by its name.
fn semantic_numbers(function: &str, arg: &str, others: &[String]) -> Vec<&'static str> {
    let a = arg.to_ascii_lowercase();
    let f = function.to_ascii_lowercase();
    let has_bag = others.iter().any(|o| {
        let o = o.to_ascii_lowercase();
        o.contains("bag") || o.contains("container")
    });
    match a.as_str() {
        "questlogindex" | "questindex" | "logindex" => vec!["{QUESTINDEX1}"],
        "questid" | "quest" => vec!["{QUEST1}", "{QUEST2}"],
        "spellid" | "spell" | "spellidentifier" | "basespellid" | "overridespellid" => {
            vec!["{SPELL1}", "{SPELL2}"]
        }
        "spellbookitemslotindex" | "spellbookslot" => vec!["{SPELLBOOKSLOT1}"],
        "itemid" | "item" | "relicitemid" | "itemidorlink" => vec!["{ITEM1}", "{ITEM2}"],
        "uimapid" | "mapid" => vec!["{MAP1}", "{MAP2}"],
        "currencyid" | "currencytype" => vec!["{CURRENCY1}"],
        "factionid" => vec!["{FACTION1}"],
        "factionindex" | "factionsortindex" => vec!["{FACTIONINDEX1}"],
        "achievementid" => vec!["{ACHIEVEMENT1}"],
        "categoryid" if f.contains("achievement") => vec!["{ACHCATEGORY1}"],
        "classid" => vec!["{CLASSID}"],
        "raceid" => vec!["{RACEID}"],
        "level" => vec!["{LEVEL}"],
        "mountid" => vec!["{MOUNT1}"],
        "actionid" | "action" | "actionslot" => vec!["{ACTIONSLOT1}"],
        "bagindex" | "containerindex" | "bagid" | "bag" | "containerid" => {
            vec!["{BAG}", "0", "4", "-1", "-2", "5"]
        }
        "slot" | "slotindex" | "containerslot" if has_bag => vec!["{BAGSLOT}"],
        "slot" | "slotid" | "invslot" | "inventoryslot" if f.contains("inventory") => {
            vec!["{EQUIPSLOT}", "16", "19"]
        }
        "slot" | "slotid" if f.contains("action") => vec!["{ACTIONSLOT1}"],
        "type" if f.contains("currency") => vec!["{CURRENCY1}"],
        _ => vec![],
    }
}

/// An argument's pool.
pub fn pool(docs: &Docs, function: &str, arg: &Json, others: &[String]) -> Pool {
    let name = arg.get("Name").and_then(Json::as_str).unwrap_or("");
    let ty = arg.get("Type").and_then(Json::as_str).unwrap_or("any");
    let nilable =
        arg.get("Nilable").and_then(Json::as_bool) == Some(true) || arg.get("Default").is_some();
    let lname = name.to_ascii_lowercase();
    let lfunction = function.to_ascii_lowercase();
    let mut p = Pool::default();
    let b = base(docs, ty, 0);
    match b.as_str() {
        "number" => {
            for v in semantic_numbers(function, name, others) {
                push(&mut p.values, v);
            }
            for v in BOUNDARY {
                push(&mut p.values, *v);
            }
            p.wrong = Some("\"x\"".into());
        }
        "boolean" => {
            push(&mut p.values, "true");
            push(&mut p.values, "false");
        }
        "unit" => {
            for u in UNITS {
                push(&mut p.values, *u);
            }
            p.wrong = Some("{}".into());
        }
        "guid" => {
            for v in [
                "UnitGUID(\"player\")",
                "\"\"",
                "\"x\"",
                "\"Creature-0-0-0-0-0-0000000000\"",
            ] {
                push(&mut p.values, v);
            }
            p.wrong = Some("{}".into());
        }
        "string" => {
            let unitish =
                ["unit", "unittoken", "target", "targettoken", "unitid"].contains(&lname.as_str());
            if ty == "AuraFilters" || (lname == "filter" && lfunction.contains("aura")) {
                for v in [
                    "\"HELPFUL\"",
                    "\"HARMFUL\"",
                    "\"HELPFUL|PLAYER\"",
                    "\"HARMFUL|RAID\"",
                ] {
                    push(&mut p.values, v);
                }
            } else if lfunction.contains("cvar")
                && ["name", "cvar", "variable"].contains(&lname.as_str())
            {
                for v in [
                    "\"autoLootDefault\"",
                    "\"nameplateShowEnemies\"",
                    "\"ActionButtonUseKeyDown\"",
                    "\"closedInfoFrames\"",
                    "\"stdtestNoSuchCVar\"",
                    "\"\"",
                ] {
                    push(&mut p.values, v);
                }
            } else if unitish {
                for u in UNITS {
                    push(&mut p.values, *u);
                }
            } else if lfunction.contains("addon") && (lname.contains("name") || lname == "addon") {
                for v in [
                    "\"StdTest\"",
                    "\"Blizzard_AuctionHouseUI\"",
                    "\"NoSuchAddOn\"",
                    "\"\"",
                ] {
                    push(&mut p.values, v);
                }
            } else if lname.contains("link") || lname == "hyperlink" {
                for v in ["{ITEMLINK1}", "\"item:6948\"", "\"x\""] {
                    push(&mut p.values, v);
                }
            } else if lname.contains("event") {
                for v in ["\"PLAYER_LOGIN\"", "\"STDTEST_NOT_AN_EVENT\""] {
                    push(&mut p.values, v);
                }
            } else if [
                "name",
                "playername",
                "charactername",
                "fullname",
                "unitname",
                "targetname",
                "character",
            ]
            .contains(&lname.as_str())
            {
                for v in ["UnitName(\"player\")", "\"Nobody\""] {
                    push(&mut p.values, v);
                }
            } else if ["text", "str", "string", "input", "message", "msg"].contains(&lname.as_str())
            {
                for v in ["\"Hello\"", "\"|cffff0000Red|r\""] {
                    push(&mut p.values, v);
                }
            }
            if ty == "textureAtlas" {
                push(&mut p.values, "\"common-icon-checkmark\"");
            }
            if ty == "FileAsset" || ty.starts_with("TextureAsset") {
                push(
                    &mut p.values,
                    "\"Interface\\\\Icons\\\\INV_Misc_QuestionMark\"",
                );
                push(&mut p.values, "134400");
            }
            if ty == "uiAddon" {
                for v in ["\"StdTest\"", "\"NoSuchAddOn\"", "1"] {
                    push(&mut p.values, v);
                }
            }
            push(&mut p.values, "\"x\"");
            push(&mut p.values, "\"\"");
            p.wrong = Some("{}".into());
        }
        "itemloc" => {
            for v in ["{ITEMLOC}", "{EQUIPLOC}", "ItemLocation:CreateEmpty()"] {
                push(&mut p.values, v);
            }
            p.wrong = Some("\"x\"".into());
        }
        "playerloc" => {
            for v in [
                "PlayerLocation:CreateFromUnit(\"player\")",
                "PlayerLocation:CreateFromGUID(UnitGUID(\"player\"))",
            ] {
                push(&mut p.values, v);
            }
            p.wrong = Some("\"x\"".into());
        }
        "spellid" => {
            for v in ["{SPELL1}", "{SPELL2}", "{SPELLNAME1}", "0", "-1", "\"x\""] {
                push(&mut p.values, v);
            }
            p.wrong = Some("{}".into());
        }
        "iteminfo" => {
            for v in [
                "{ITEM1}",
                "{ITEMLINK1}",
                "\"item:6948\"",
                "0",
                "-1",
                "\"x\"",
            ] {
                push(&mut p.values, v);
            }
            p.wrong = Some("{}".into());
        }
        "color" => {
            push(&mut p.values, "CreateColor(1, 0.5, 0, 1)");
            p.wrong = Some("\"x\"".into());
        }
        "vector2" => {
            push(&mut p.values, "CreateVector2D(0.5, 0.5)");
            p.wrong = Some("\"x\"".into());
        }
        "function" => {
            push(&mut p.values, "function() end");
            p.wrong = Some("\"x\"".into());
        }
        "widget" => {
            push(&mut p.values, "UIParent");
            p.wrong = Some("\"x\"".into());
        }
        "table" => {
            push(&mut p.values, "{}");
            p.wrong = Some("\"x\"".into());
        }
        "any" => {
            for v in ["1", "\"x\"", "true", "{}"] {
                push(&mut p.values, v);
            }
        }
        other if other.starts_with("literals:") => {
            for v in literals(docs, &other["literals:".len()..]) {
                push(&mut p.values, v);
            }
            push(&mut p.values, "\"x\"");
            p.wrong = Some("{}".into());
        }
        other if docs.enums.contains_key(other) => {
            for v in semantic_numbers(function, name, others) {
                push(&mut p.values, v);
            }
            let values = &docs.enums[other];
            for v in values {
                push(&mut p.values, v.to_string());
            }
            // Past either end.
            if let (Some(lo), Some(hi)) = (values.first(), values.last()) {
                push(&mut p.values, (hi + 1).to_string());
                push(&mut p.values, (lo - 1).to_string());
            }
            p.wrong = Some("\"x\"".into());
        }
        other if docs.structures.contains_key(other) => {
            match other {
                "ItemKey" => push(&mut p.values, "C_AuctionHouse.MakeItemKey({ITEM1})"),
                "CalendarTime" => {
                    push(&mut p.values, "C_DateAndTime.GetCurrentCalendarTime()");
                }
                _ => push(&mut p.values, structure(docs, other, 0)),
            }
            push(&mut p.values, "{}");
            p.wrong = Some("\"x\"".into());
        }
        _ => {
            push(&mut p.values, "1");
        }
    }
    if nilable {
        push(&mut p.values, "nil");
    }
    p
}

/// A structure's value: its fields that may not be nil, each at its
/// baseline.
fn structure(docs: &Docs, name: &str, depth: usize) -> String {
    if depth > 2 {
        return "{}".into();
    }
    let mut parts = Vec::new();
    for field in docs.structures.get(name).into_iter().flatten() {
        if field.get("Nilable").and_then(Json::as_bool) == Some(true) {
            continue;
        }
        let Some(fname) = field.get("Name").and_then(Json::as_str) else {
            continue;
        };
        let ty = field.get("Type").and_then(Json::as_str).unwrap_or("any");
        let value = if docs.structures.contains_key(ty) {
            structure(docs, ty, depth + 1)
        } else {
            pool(docs, name, field, &[])
                .values
                .first()
                .cloned()
                .unwrap_or_else(|| "nil".into())
        };
        parts.push(format!("{fname} = {value}"));
    }
    format!("{{ {} }}", parts.join(", "))
}

/// A documented query's calls (each an argument list's source).
pub fn matrix(docs: &Docs, function: &str, doc: &Json, request: bool) -> Vec<String> {
    let args: Vec<&Json> = doc
        .get("Arguments")
        .and_then(Json::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let names: Vec<String> = args
        .iter()
        .map(|a| {
            a.get("Name")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let pools: Vec<Pool> = args
        .iter()
        .map(|a| pool(docs, function, a, &names))
        .collect();
    let mut out: Vec<String> = Vec::new();
    let add = |list: Vec<String>, out: &mut Vec<String>| {
        let call = list.join(", ");
        if !out.contains(&call) {
            out.push(call);
        }
    };
    add(vec![], &mut out);
    if pools.is_empty() {
        return out;
    }
    let baseline: Vec<String> = pools
        .iter()
        .map(|p| p.values.first().cloned().unwrap_or_else(|| "nil".into()))
        .collect();
    add(baseline.clone(), &mut out);
    // Usage errors: a value of a type the argument does not take; nil for
    // one after the first that may not be nil.
    for (i, p) in pools.iter().enumerate() {
        if let Some(w) = &p.wrong {
            let mut call = baseline.clone();
            call[i] = w.clone();
            add(call, &mut out);
        }
        let nilable = args[i].get("Nilable").and_then(Json::as_bool) == Some(true)
            || args[i].get("Default").is_some();
        if i > 0 && !nilable {
            let mut call = baseline.clone();
            call[i] = "nil".into();
            add(call, &mut out);
        }
    }
    // The optional arguments left out.
    let required = args
        .iter()
        .rposition(|a| {
            a.get("Nilable").and_then(Json::as_bool) != Some(true) && a.get("Default").is_none()
        })
        .map_or(0, |i| i + 1);
    if required < baseline.len() {
        add(baseline[..required].to_vec(), &mut out);
    }
    if request {
        return out;
    }
    let product: usize = pools
        .iter()
        .map(|p| p.values.len().max(1))
        .try_fold(1usize, |acc, n| acc.checked_mul(n))
        .unwrap_or(usize::MAX);
    if pools.len() > 1 && product <= FULL_PRODUCT {
        let mut index = vec![0usize; pools.len()];
        loop {
            add(
                index
                    .iter()
                    .zip(&pools)
                    .map(|(&i, p)| p.values[i].clone())
                    .collect(),
                &mut out,
            );
            let mut k = pools.len();
            loop {
                if k == 0 {
                    break;
                }
                k -= 1;
                index[k] += 1;
                if index[k] < pools[k].values.len() {
                    break;
                }
                index[k] = 0;
                if k == 0 {
                    k = usize::MAX;
                    break;
                }
            }
            if k == usize::MAX {
                break;
            }
        }
    } else {
        // One argument at a time around the baseline, the arguments' values
        // taken in turn (so a cap leaves each argument some).
        let longest = pools.iter().map(|p| p.values.len()).max().unwrap_or(0);
        for v in 1..longest {
            for (i, p) in pools.iter().enumerate() {
                if let Some(value) = p.values.get(v) {
                    let mut call = baseline.clone();
                    call[i] = value.clone();
                    add(call, &mut out);
                }
            }
        }
    }
    out.truncate(CAP);
    out
}

/// An undocumented query's calls: none, a few numbers and strings, and
/// the character's own IDs its name suggests.
pub fn generic(function: &str) -> Vec<String> {
    let mut out: Vec<String> = ["", "1", "0", "-1", "2", "\"player\"", "\"x\"", "1, 1"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let short = function.rsplit('.').next().unwrap_or(function);
    let hints: &[(&str, &[&str])] = &[
        ("Spell", &["{SPELL1}", "{SPELLBOOKSLOT1}, \"spell\""]),
        ("Item", &["{ITEM1}", "{ITEMLINK1}"]),
        ("Quest", &["{QUESTINDEX1}", "{QUEST1}"]),
        ("Container", &["{BAG}, {BAGSLOT}", "{BAG}"]),
        ("Bag", &["{BAG}, {BAGSLOT}", "{BAG}"]),
        ("Inventory", &["\"player\", {EQUIPSLOT}"]),
        ("Faction", &["{FACTIONINDEX1}", "{FACTION1}"]),
        ("Currency", &["{CURRENCY1}"]),
        ("Achievement", &["{ACHIEVEMENT1}", "{ACHCATEGORY1}"]),
        ("Map", &["{MAP1}"]),
        ("Action", &["{ACTIONSLOT1}"]),
        ("Unit", &["\"target\"", "\"pet\""]),
    ];
    for (word, calls) in hints {
        if short.contains(word) {
            for c in *calls {
                let c = (*c).to_string();
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// The plan.
pub struct Plan {
    /// Each probed function and its calls' argument lists.
    pub calls: Vec<(String, Vec<String>)>,
    pub setters: Vec<Setter>,
    /// Each skipped function and why.
    pub skipped: Vec<(String, String)>,
    /// The probed functions the documentation lacks (no argument checks).
    pub undocumented: BTreeSet<String>,
}

/// Functions with their calls' argument lists.
pub type Calls = Vec<(String, Vec<String>)>;

impl Plan {
    pub fn build(root: &Path) -> Result<Plan> {
        let docs = Docs::load(root)?;
        let functions = client_functions(root)?;
        let mut protected: HashSet<String> = HashSet::new();
        protected.extend(engine_list(root, "protection.rs", "PROTECTED")?);
        protected.extend(engine_list(root, "protection.rs", "HARDWARE")?);
        Ok(Plan::from_parts(&docs, &functions, &protected))
    }

    pub fn from_parts(docs: &Docs, functions: &[String], protected: &HashSet<String>) -> Plan {
        let setters = setters(docs);
        let setter_names = setter_functions(&setters);
        let mut calls = Vec::new();
        let mut skipped = Vec::new();
        let mut undocumented = BTreeSet::new();
        let unique: BTreeSet<&String> = functions.iter().collect();
        for name in unique {
            let doc = docs.functions.get(name.as_str());
            if doc.is_none() {
                undocumented.insert(name.clone());
            }
            match classify(name, doc, protected, &setter_names) {
                Kind::Query => {
                    let list = match doc {
                        Some(d) => matrix(docs, name, d, false),
                        None => generic(name),
                    };
                    calls.push((name.clone(), list));
                }
                Kind::Request => {
                    let list = match doc {
                        Some(d) => matrix(docs, name, d, true),
                        None => vec![String::new()],
                    };
                    calls.push((name.clone(), list));
                }
                Kind::Setter => {}
                Kind::Skipped(why) => skipped.push((name.clone(), why)),
            }
        }
        Plan {
            calls,
            setters,
            skipped,
            undocumented,
        }
    }

    /// The calls in three passes: each function's first two (no
    /// arguments, the baseline) for documented functions; the other calls
    /// of documented functions without a boundary number; and the rest
    /// (boundary numbers, out of range values of undocumented functions,
    /// every call of an undocumented function past its first).
    pub fn passes(&self) -> (Calls, Calls, Calls) {
        let boundary = regex::Regex::new(r"(^|[^0-9.\w])(-1|2147483647|1\.5)($|[^0-9.])")
            .expect("a valid pattern");
        let (mut safe, mut checked, mut risky) = (Vec::new(), Vec::new(), Vec::new());
        for (name, list) in &self.calls {
            let documented = !self.undocumented.contains(name);
            let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
            for (i, args) in list.iter().enumerate() {
                if documented && i < 2 || !documented && i == 0 {
                    a.push(args.clone());
                } else if documented && !boundary.is_match(args) {
                    b.push(args.clone());
                } else {
                    c.push(args.clone());
                }
            }
            for (pass, part) in [(&mut safe, a), (&mut checked, b), (&mut risky, c)] {
                if !part.is_empty() {
                    pass.push((name.clone(), part));
                }
            }
        }
        (safe, checked, risky)
    }

    pub fn call_count(&self) -> usize {
        self.calls.iter().map(|(_, l)| l.len()).sum::<usize>()
            + self.setters.iter().map(|s| s.sets.len()).sum::<usize>()
    }

    /// The skipped functions counted by reason, most first.
    pub fn skipped_by_reason(&self) -> Vec<(String, usize)> {
        let mut by: BTreeMap<String, usize> = BTreeMap::new();
        for (_, why) in &self.skipped {
            *by.entry(why.clone()).or_default() += 1;
        }
        let mut list: Vec<(String, usize)> = by.into_iter().collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        list
    }

    /// The plan as StdTest's module `/fullprobeplan`.
    pub fn lua(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "-- @generated by `cargo xtask fullprobe-plan` (xtask/src/fullprobe.rs). Do not edit.\n\
             -- The plan of `/stdtest engine fullprobe` (fullprobe.wowlua): {} functions called\n\
             -- {} times in all ({} setting calls of {} settings, each set, read back and\n\
             -- restored); {} functions skipped:",
            self.calls.len(),
            self.call_count(),
            self.setters.iter().map(|s| s.sets.len()).sum::<usize>(),
            self.setters.len(),
            self.skipped.len()
        );
        for (why, n) in self.skipped_by_reason() {
            let _ = writeln!(out, "--   {n:5} {why}");
        }
        out.push_str(
            "-- calls: { function, argument list… } (Lua sources; `{NAME}` a value of the\n\
             -- character StdTest resolves when the run starts). setters: the read-back,\n\
             -- the restore (a function given the read-back's values), the settings.\n\
             local _, ns = ...\n\nlocal calls = {\n",
        );
        // Safest first, so a short run gets the most: every function's
        // call without arguments and its baseline, then the calls the
        // argument checks answer; the calls most likely to crash the client
        // (boundary numbers, undocumented functions, which have no argument
        // checks) come last, after the settings (`risky`).
        let (safe, checked, risky) = self.passes();
        for pass in [&safe, &checked] {
            for (name, list) in pass {
                let _ = write!(out, "    {{ {}", lua_string(name));
                for args in list {
                    let _ = write!(out, ", {}", lua_string(args));
                }
                out.push_str(" },\n");
            }
        }
        out.push_str("}\n\nlocal risky = {\n");
        for (name, list) in &risky {
            let _ = write!(out, "    {{ {}", lua_string(name));
            for args in list {
                let _ = write!(out, ", {}", lua_string(args));
            }
            out.push_str(" },\n");
        }
        out.push_str("}\n\nlocal setters = {\n");
        for s in &self.setters {
            let _ = write!(
                out,
                "    {{ get = {}, restore = {}, sets = {{",
                lua_string(&s.get),
                lua_string(&s.restore)
            );
            for (i, set) in s.sets.iter().enumerate() {
                let _ = write!(
                    out,
                    "{}{}",
                    if i == 0 { " " } else { ", " },
                    lua_string(set)
                );
            }
            out.push_str(" } },\n");
        }
        out.push_str("}\n\nlocal skipped = {\n");
        for (name, why) in &self.skipped {
            let _ = writeln!(out, "    {{ {}, {} }},", lua_string(name), lua_string(why));
        }
        out.push_str(
            "}\n\nns.__wowlua.define(\"fullprobeplan\", { calls = calls, risky = risky, setters = setters, skipped = skipped })\n",
        );
        out
    }
}

/// A Lua string literal: in long brackets of level 2 or more (as the
/// answers battery writes its calls), quoted when it has a line break.
fn lua_string(text: &str) -> String {
    if text.contains('\n') {
        return format!("{text:?}");
    }
    let mut level = 2;
    loop {
        let eq = "=".repeat(level);
        if !text.contains(&format!("]{eq}]")) && !text.ends_with(&format!("]{eq}")) {
            return format!("[{eq}[{text}]{eq}]");
        }
        level += 1;
    }
}

/// `cargo xtask fullprobe-plan [--stats]`.
pub fn generate(root: &Path) -> Result<String> {
    Ok(Plan::build(root)?.lua())
}

pub fn stats(root: &Path) -> Result<()> {
    let plan = Plan::build(root)?;
    println!(
        "{} functions probed, {} calls ({} settings); {} skipped:",
        plan.calls.len(),
        plan.call_count(),
        plan.setters.len(),
        plan.skipped.len()
    );
    for (why, n) in plan.skipped_by_reason() {
        println!("  {n:5} {why}");
    }
    let mut most: Vec<&(String, Vec<String>)> = plan.calls.iter().collect();
    most.sort_by_key(|(_, l)| std::cmp::Reverse(l.len()));
    println!("most calls:");
    for (name, list) in most.iter().take(10) {
        println!("  {:3} {name}", list.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
    }

    fn docs() -> Docs {
        Docs::load(root()).unwrap()
    }

    #[test]
    fn names_split_into_words() {
        assert_eq!(words("GetSpellInfo"), ["Get", "Spell", "Info"]);
        assert_eq!(words("EJ_GetTier"), ["EJ", "Get", "Tier"]);
        assert_eq!(words("GUIDIsPlayer"), ["GUID", "Is", "Player"]);
        assert_eq!(words("BNGetInfo"), ["BN", "Get", "Info"]);
    }

    #[test]
    fn functions_are_classified_safe_first() {
        let d = docs();
        let protected: HashSet<String> = ["CastSpellByName".to_string()].into_iter().collect();
        let setters: HashSet<String> = ["C_CVar.SetCVar".to_string()].into_iter().collect();
        let kind = |name: &str| classify(name, d.functions.get(name), &protected, &setters);
        assert_eq!(kind("C_Spell.GetSpellInfo"), Kind::Query);
        assert_eq!(kind("UnitName"), Kind::Query);
        assert_eq!(kind("C_PetJournal.PetIsHurt"), Kind::Query);
        assert_eq!(kind("InCombatLockdown"), Kind::Query);
        assert_eq!(kind("C_CVar.SetCVar"), Kind::Setter);
        assert_eq!(kind("RequestRaidInfo"), Kind::Request);
        for acts in [
            "CastSpellByName",
            "UnitSetRole",
            "GetQuestReward",
            "AcceptQuest",
            "C_PartyInfo.RequestInviteFromUnit",
            "C_ClassTalents.RequestNewConfig",
            "SetBinding",
            "SortBags",
            "strsplit",
            "string.format",
            "ReloadUI",
        ] {
            assert!(
                matches!(kind(acts), Kind::Skipped(_)),
                "{acts}: {:?}",
                kind(acts)
            );
        }
    }

    #[test]
    fn a_query_gets_the_full_matrix() {
        let d = docs();
        let f = |name: &str| matrix(&d, name, &d.functions[name], false);
        // None, the baseline, the usage errors, each value.
        let spell = f("C_Spell.GetSpellInfo");
        assert_eq!(&spell[..3], ["", "{SPELL1}", "{}"]);
        assert!(spell.contains(&"0".to_string()) && spell.contains(&"-1".to_string()));
        // Few combinations: all of them.
        let unit = f("UnitIsUnit");
        assert!(
            unit.contains(&"\"party1\", \"pet\"".to_string()),
            "{unit:?}"
        );
        assert!(unit.contains(&"{}, \"player\"".to_string()));
        // Every enum value.
        let bank = f("C_SpellBook.GetSpellBookItemInfo");
        for v in [
            "{SPELLBOOKSLOT1}, 0",
            "{SPELLBOOKSLOT1}, 1",
            "{SPELLBOOKSLOT1}, 2",
            "2147483647, 0",
        ] {
            assert!(bank.contains(&v.to_string()), "{v}: {bank:?}");
        }
        // A server request: its baseline and usage errors only.
        let name = "C_Item.RequestLoadItemDataByID";
        let request = matrix(&d, name, &d.functions[name], true);
        assert!(request.len() <= 4, "{request:?}");
    }

    /// The plan never passes a numeric string (the client converts it and
    /// its game code may assert: `C_Garrison.GetFollowerMissionCompleteInfo("1")`
    /// crashed it), calls only functions the client has and no protected
    /// one, and caps every function's calls.
    #[test]
    fn the_plan_is_safe() {
        let plan = Plan::build(root()).unwrap();
        let functions: HashSet<String> = client_functions(root()).unwrap().into_iter().collect();
        let protected: HashSet<String> = engine_list(root(), "protection.rs", "PROTECTED")
            .unwrap()
            .into_iter()
            .collect();
        let numeric = regex::Regex::new(r#"(^|[(,{=] ?)"-?[0-9.]+""#).unwrap();
        for (name, list) in &plan.calls {
            assert!(functions.contains(name), "{name}");
            assert!(!protected.contains(name), "{name}");
            assert!(list.len() <= CAP, "{name}");
            for args in list {
                assert!(!numeric.is_match(args), "{name}({args})");
            }
        }
        for s in &plan.setters {
            for set in &s.sets {
                assert!(!numeric.is_match(set) || set.contains("SetCVar"), "{set}");
            }
        }
        assert!(plan.call_count() > 20000, "{}", plan.call_count());
        // Every function is either called or listed with its reason.
        let called = plan.calls.len() + setter_functions(&plan.setters).len();
        assert!(called + plan.skipped.len() + 10 >= functions.len());
    }
}
