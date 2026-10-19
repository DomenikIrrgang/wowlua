//! The client's checks of documented functions' arguments, as
//! `/stdtest argcheck` and `/stdtest engine fullprobe` recorded them on WoW
//! Forever 1.60.1 (fixtures `tests/client/forever-camelot-argcheck.lua`,
//! `forever-camelot-fullprobe.lua`; the full probe, newer and calling each
//! function directly, wins where they differ): a missing argument, a
//! secret where the function refuses one, or a value its C type does not
//! take raises "bad argument #N to 'name' (<message>Usage: …)", the usage
//! made from the documentation as the client makes it (`fail` tells whose
//! error it is and how it reads), and the undocumented functions' own
//! checks (`data/camelot-usages.txt`).
//!
//! The check depends on the argument's documented type, as the client's
//! C bindings differ: integers check their range, the restricted unit
//! tokens want a string, enumerations their range, some IDs take anything.
//! Types the probes did not reach follow their family (by their alias) and
//! are unverified.

use serde_json::Value as Json;
use wowlua_vm::{LuaError, NativeCall, Value, Vm};

use crate::api::{Api, Function};

/// How an argument's C type takes a value.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Family {
    /// An integer in `min..=max` (`luaIndex`: one more, counting from 1).
    Int { min: f64, max: f64, index: bool },
    /// Any number (or a string that converts).
    Number,
    /// A C float (widget methods' `number`): refused outside ±FLT_MAX, a
    /// non-number included.
    Float,
    /// A number, refused as "must be a finite number" otherwise.
    Finite,
    /// A table with something in it.
    Filled,
    /// A name plate's unit token.
    NamePlateUnit,
    /// A locale's name (`enUS`).
    Locale,
    /// A font object (a widget).
    FontObject,
    /// Any value, nil too, but not none (`luaL_checkany`).
    LuaAny,
    /// A string or a number (`luaL_checkstring`).
    LuaString,
    /// A garrison follower's ID.
    FollowerId,
    /// A number in `min..=max`, refused with the usage alone otherwise.
    Bounded { min: f64, max: f64 },
    /// A spell book item: its slot and its bank, two of Lua's arguments
    /// read as one (`spellBookItem`).
    BookItem,
    /// A structure read from several of Lua's arguments (its members):
    /// any of them refused, the first is, with the usage alone.
    Packed,
    /// One of the client's script objects (a curve, a heal prediction
    /// calculator), refused with the usage alone.
    ScriptObject,
    /// An item's location (`ItemLocationMixin`): a bag and slot or a gear
    /// slot; an azerite item's, never (Forever has none).
    ItemLocation { azerite: bool },
    /// A string (or a number).
    Str,
    /// A string, refused with "Expected a string." (restricted unit tokens).
    StrictStr,
    /// A table.
    Table { mixin: bool },
    /// A widget (a table holding its object).
    Object,
    /// A font object, or the name of one (`"GameFontNormal"`; names
    /// unverified, `'x'` refused as recorded).
    Font,
    /// A function.
    Func,
    /// An enumeration's value: a number in its `min..=max` (a non-number
    /// reads as 0), refused with the usage alone.
    Enum { min: f64, max: f64 },
    /// Anything: booleans, some IDs, Lua values.
    Any,
}

/// Enumerations the client takes any value of: bit flags whose
/// combinations pass (`C_Container.GetBagSlotFlag(0, 513)`,
/// `C_ReportSystem.GetMinorCategoryString(1048577)`; `/stdtest engine
/// fullprobe`).
const UNCHECKED_ENUMS: &[&str] = &["BagSlotFlags", "ReportMinorCategory"];

/// Enumerations whose client takes fewer values than the documentation's
/// range: `C_Intl.GetCurrencyName(code, 3)` and `4` are refused, `2`
/// answers (`/stdtest engine fullprobe`; why is not known).
fn enum_range(api: &Api, ty: &str) -> Option<(f64, f64)> {
    if UNCHECKED_ENUMS.contains(&ty) {
        return None;
    }
    if ty == "CurrencyNameStyle" {
        return Some((0.0, 2.0));
    }
    let meta = format!("{ty}Meta");
    api.enum_meta
        .iter()
        .find(|(name, _)| *name == meta)
        .map(|(_, [_, min, max])| (*min, *max))
}

const U32: (f64, f64) = (0.0, 4_294_967_295.0);
const I32: (f64, f64) = (-2_147_483_648.0, 2_147_483_647.0);

fn family(api: &Api, ty: &str, arg: &Json, method: bool) -> Family {
    let int = |(min, max): (f64, f64), index| Family::Int { min, max, index };
    match ty {
        "luaIndex" => return int(U32, true),
        "__spellBookItem" => return Family::BookItem,
        "__packed" => return Family::Packed,
        "number" if method => return Family::Float,
        "number" => return int(I32, false),
        "EncounterTimelineEventID" => return int(U32, false),
        // A bag index is a plain integer: `GetBagSlotFlag(-4, 1)` and
        // `(24, 1)` answer, `("x", 1)` is refused for the range (the full
        // probe; not an enumeration's check).
        "BagIndex" => return int(I32, false),
        // An aura instance's ID (`C_UnitAuras`' `auraInstance`): -1 refused,
        // 2147483647 taken (the full probe).
        "__u32" => return int(U32, false),
        // Money and big integers are unsigned: -1 and "x" refused with the
        // usage alone, 2147483647 taken (`C_CurrencyInfo.GetCoinIcon`,
        // `C_DateAndTime.GetCalendarTimeFromEpoch`, the full probe).
        "WOWMONEY" | "BigUInteger" if !method => {
            return Family::Bounded {
                min: 0.0,
                max: f64::MAX,
            };
        }
        "WOWMONEY" | "BigUInteger" => return Family::Number,
        // Combat log flags and game rules: any number, a non-number refused
        // with the usage alone (`C_CombatLog.DoesObjectMatchFilter(0, "x")`,
        // `(-1, 0)` and `(0, 2147483649)` answer; `C_GameRules.
        // IsGameRuleActive(true)` refused; the full probe).
        "CombatLogObject" | "GameRule" if !method => return Family::Number,
        // An inventory slot: -1, 2147483647 and "x" refused, 0 to 1.5 taken
        // (`C_PaperDollInfo.GetTemporaryEnchantmentInfo`, the full probe;
        // the upper bound is unverified).
        "LuaInventorySlot" if !method => {
            return Family::Bounded {
                min: 0.0,
                max: 255.0,
            };
        }
        // An artifact tier, read as an enumeration's value: "x" taken as 0,
        // -1 and 2147483647 refused (`C_ArtifactUI.GetCostForPointAtRank`,
        // the full probe; the upper bound is unverified).
        "ArtifactTiers" if !method => {
            return Family::Enum {
                min: 0.0,
                max: 255.0,
            };
        }
        "CalendarEventID" | "ClubId" | "WeeklyRewardItemDBID" | "InventorySlots" => {
            return Family::Any;
        }
        // A function's font: a font object, not any table (`GetFontInfo({})`
        // refused with the usage alone, the full probe).
        "SimpleFont" if !method => return Family::FontObject,
        "SimpleFont" => return Family::Font,
        // A garrison follower's ID: "" and "x" refused with the usage alone
        // (`C_Garrison.GetFollowerAutoCombatSpells`, the full probe; Forever
        // has no followers: what it takes, here "0x…" as the IDs were in
        // retail, is unverified).
        "GarrisonFollower" if !method => return Family::FollowerId,
        // A name plate's unit: a string, but neither a group member's
        // ("party1": "Raid<n>/Party<n> unit tokens are not allowed for this
        // call.") nor a compound token (a GUID: "Compound unit tokens
        // (example: boss1targetpet) are not allowed for this call."), the
        // full probe; which other strings count as compound is unverified
        // (here: any but letters and a number).
        "UnitTokenNamePlate" if !method => return Family::NamePlateUnit,
        "UnitTokenNamePlate"
        | "UnitTokenRestrictedForAddOns"
        | "UnitTokenPvPRestrictedForAddOns" => {
            return Family::StrictStr;
        }
        "bool" | "LuaValueReference" | "LuaValueVariant" => return Family::Any,
        "SimpleFrame" | "FrameScriptObject" => return Family::Object,
        "table" => return Family::Table { mixin: false },
        "luaFunction" | "function" | "LuaFunctionReference" => return Family::Func,
        "ScriptTypeName" => return Family::Str,
        // The client's own structures the probe met: an empty table refused
        // for a comparison item and a transmog location (`C_TooltipComparison.
        // GetItemComparisonInfo({})`, `C_Transmog.GetSlotVisualInfo({})`, the
        // full probe; a table with some of their fields unverified).
        "TooltipComparisonItem" | "TransmogLocation" if !method => return Family::Filled,
        "TooltipComparisonItem" | "PlayerLocation" => return Family::Table { mixin: true },
        // An item's location: an empty one refused (`ItemLocation:
        // CreateEmpty()` by all 54 functions the full probe gave it, even
        // `C_Item.DoesItemExist`); an azerite item's, every location
        // (`C_AzeriteItem.GetPowerLevel(ItemLocation:CreateFromBagAndSlot(0,
        // 1))`, a gear slot's too: Forever has no azerite items; unverified
        // for one that holds one).
        "ItemLocation" | "EmptiableItemLocation" if !method => {
            return Family::ItemLocation { azerite: false };
        }
        "AzeriteItemLocation" | "AzeriteEmpoweredItemLocation" if !method => {
            return Family::ItemLocation { azerite: true };
        }
        // A heal prediction calculator (`UnitGetDetailedHealPrediction(unit,
        // healer, 1)`, `"x"`, `true`, `{}` refused as #3, the full probe).
        "UnitHealPredictionCalculator" if !method => return Family::ScriptObject,
        "cstring" | "string" | "stringView" => return Family::Str,
        _ => {}
    }
    // An enumeration's value outside its range is refused: `-1` and one
    // past the last (`UnitPowerDisplayMod(30)`, `C_Intl.ParseNumber(s, 4)`)
    // for all 77 enumerations the full probe passed them to (but the bit
    // flags below and `CurrencyNameStyle`'s narrower range); a non-number
    // reads as 0 (`ParseNumber(s, "x")`, `IsSceneTypeActive({})` answer).
    // The newest probe wins over argcheck's "enums take anything", which
    // passed only values in range. Methods' enums are not known.
    if api.enum_names.contains(ty) {
        return match enum_range(api, ty) {
            Some((min, max)) if !method => Family::Enum { min, max },
            _ => Family::Any,
        };
    }
    if api.callbacks.contains(ty) {
        return Family::Func;
    }
    // A script object the client gives Lua as a userdata (`LuaCurveObjectBase`:
    // the class `CurveObjectBase`): `UnitHealthPercent(unit, true, 1)`,
    // `"x"`, `{}` refused as #3 with the usage alone, as for
    // `C_UnitAuras.GetAuraDispelTypeColor`'s color curve (the full probe;
    // whether a curve of another class is refused is unverified).
    let class = ty.strip_prefix("Lua").unwrap_or(ty);
    if !method && api.classes.get(class).is_some_and(|c| c.userdata) {
        return Family::ScriptObject;
    }
    // Structures, and the client's mixin-backed ones (`ItemLocation`).
    if api.structures.contains(ty) || arg.get("Mixin").is_some() {
        return Family::Table {
            mixin: arg.get("Mixin").is_some(),
        };
    }
    // Unverified: an alias's family (string literals are strings); a type
    // neither documented nor an alias takes anything until probed.
    match api.aliases.get(ty).map(String::as_str) {
        Some("string") => Family::Str,
        // A method's number alias refuses a non-number by type
        // (`SetAlpha('x')`: the usage alone; `/stdtest engine`).
        Some("number") if method => Family::Number,
        Some("number") => int(I32, false),
        Some(a) if a.contains("string") && a.contains("number") => Family::Str,
        Some(a) if a.starts_with('"') => Family::Str,
        _ => Family::Any,
    }
}

/// A structure's field the documentation spells out on its own (`hslH`,
/// `positionX`): the structure's name and the field's letter.
fn spelled_field(name: &str) -> Option<(&str, char)> {
    let last = name.chars().last()?;
    let prefix = &name[..name.len() - last.len_utf8()];
    let mut chars = prefix.chars();
    (last.is_ascii_uppercase()
        && chars.next()?.is_ascii_lowercase()
        && chars.all(|c| c.is_ascii_alphabetic()))
    .then_some((prefix, last))
}

/// Runs of at least two spelled-out fields of one structure (`hslH, hslS,
/// hslL`): their start, length and the structure's name.
fn field_runs(items: &[Json]) -> Vec<(usize, usize, String)> {
    let name = |v: &Json| {
        v.get("Name")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string()
    };
    let mut runs = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let first = name(&items[i]);
        if let Some((prefix, _)) = spelled_field(&first) {
            let mut j = i + 1;
            while j < items.len()
                && spelled_field(&name(&items[j])).is_some_and(|(p, _)| p == prefix)
            {
                j += 1;
            }
            if j - i >= 2 {
                runs.push((i, j - i, prefix.to_string()));
                i = j;
                continue;
            }
        }
        i += 1;
    }
    runs
}

/// A function's arguments as its client takes them: the documentation's,
/// but some read several of Lua's arguments as one, in the usage and the
/// checks (`/stdtest engine fullprobe`):
/// - a spell book item's slot and bank (`spellBookItemSlotIndex,
///   spellBookItemSpellBank`) are `spellBookItem` (all 34 functions taking
///   them, `C_SpellBook.GetSpellBookItemName(0, 2)` refused as argument 1
///   with "Usage: … GetSpellBookItemName(spellBookItem)"; an argument after
///   it keeps its own number, `IsSpellBookItemInRange(1, 0, {})` refusing
///   #3);
/// - an aura's unit and instance ID (`auraInstanceUnit, auraInstanceID`)
///   are `auraInstance` (all 8 functions: `C_UnitAuras.GetAuraDuration(
///   "player", -1)`, `("player", "x")`, `({}, 1)` refused as #1 with the
///   usage alone; `(UnitGUID("player"), 1)` taken);
/// - a structure's fields the documentation spells out (`hslH, hslS,
///   hslL`) are the structure (`hsl`): `C_ColorUtil.ConvertHSLToHSV(1, "x",
///   1)` and `(1, 1, nil)` refused as #1 with "Usage: local h, s, v =
///   C_ColorUtil.ConvertHSLToHSV(hsl)", -1 and 2147483647 taken (all four
///   converters; methods' structures are not known).
fn client_arguments(f: &Function) -> Vec<Json> {
    let mut args = f
        .doc
        .get("Arguments")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    let name = |v: &Json| v.get("Name").and_then(Json::as_str).map(str::to_string);
    if let Some(i) = args
        .iter()
        .position(|a| name(a).as_deref() == Some("spellBookItemSlotIndex"))
        && args.get(i + 1).and_then(name).as_deref() == Some("spellBookItemSpellBank")
    {
        args.splice(
            i..i + 2,
            [serde_json::json!({
                "Name": "spellBookItem", "Type": "__spellBookItem", "Nilable": false
            })],
        );
    }
    if let Some(i) = args
        .iter()
        .position(|a| name(a).as_deref() == Some("auraInstanceUnit"))
        && args.get(i + 1).and_then(name).as_deref() == Some("auraInstanceID")
    {
        let unit = args[i].clone();
        let mut id = args[i + 1].clone();
        if let Json::Object(o) = &mut id {
            o.insert("Type".into(), Json::from("__u32"));
        }
        args.splice(
            i..i + 2,
            [serde_json::json!({
                "Name": "auraInstance", "Type": "__packed", "Nilable": false,
                "Members": [unit, id]
            })],
        );
    }
    // Functions only: methods' structures are not known.
    let runs = if f.name.contains(':') {
        Vec::new()
    } else {
        field_runs(&args)
    };
    for (start, len, prefix) in runs.into_iter().rev() {
        let members: Vec<Json> = args[start..start + len].to_vec();
        args.splice(
            start..start + len,
            [serde_json::json!({
                "Name": prefix, "Type": "__packed", "Nilable": false, "Members": members
            })],
        );
    }
    args
}

/// Arguments the client's usage names as one though it checks them one by
/// one, each by its own number (`/stdtest engine fullprobe`): the event
/// list's `filter` (`C_EncounterTimeline.GetSortedEventList(1, "x")`
/// refused as #2), the specialization's `query` (`#5`, `#6` and `#7`
/// refused for their ranges) and the hyperlinks' `options`. The usage
/// shows them without brackets, though they may be left out
/// (`GetSortedEventList()` answers).
const NAMED_TOGETHER: &[(&str, &str, usize, &str)] = &[
    (
        "C_EncounterTimeline.GetSortedEventList",
        "maxEventCount",
        4,
        "filter",
    ),
    (
        "C_SpecializationInfo.GetSpecializationInfo",
        "specializationIndex",
        7,
        "query",
    ),
    (
        "C_StringUtil.StripHyperlinks",
        "maintainColor",
        5,
        "options",
    ),
];

/// Returns the client's usage names otherwise than the documentation: a
/// structure's spelled-out fields by their letters (`UnitPosition`: "local
/// x, y, z, mapID", documented `positionX, positionY, positionZ`; 13
/// functions in the full probe), but where the client keeps the documented
/// names (the map highlights' `texturePercentageX`) or the structure's
/// (the artifact appearances' `swatchColor`); an addon's
/// dependencies as `unpackedPrimitiveType`. Methods' are not known.
fn client_returns(f: &Function) -> Vec<Json> {
    let mut returns = f
        .doc
        .get("Returns")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    let set_name = |v: &mut Json, name: &str| {
        if let Json::Object(o) = v {
            o.insert("Name".into(), Json::from(name));
        }
    };
    match f.name {
        "C_AddOns.GetAddOnDependencies" | "C_AddOns.GetAddOnOptionalDependencies" => {
            for r in &mut returns {
                set_name(r, "unpackedPrimitiveType");
            }
        }
        "C_Map.GetMapHighlightInfoAtPosition" | "C_Map.GetMapHighlightPulseInfo" => {}
        // One return, the structure's (`swatchColor`).
        "C_ArtifactUI.GetAppearanceInfo" | "C_ArtifactUI.GetAppearanceInfoByID" => {
            for (start, len, prefix) in field_runs(&returns).into_iter().rev() {
                let mut one = returns[start].clone();
                set_name(&mut one, &prefix);
                returns.splice(start..start + len, [one]);
            }
        }
        _ if f.name.contains(':') => {}
        _ => {
            for (start, len, _) in field_runs(&returns) {
                for r in &mut returns[start..start + len] {
                    let letter = r
                        .get("Name")
                        .and_then(Json::as_str)
                        .and_then(|n| n.chars().last())
                        .map(|c| c.to_ascii_lowercase().to_string())
                        .unwrap_or_default();
                    set_name(r, &letter);
                }
            }
        }
    }
    returns
}

/// Arguments the client takes more loosely than its documentation says,
/// as Blizzard's code passes them at every login without an error:
/// `UnitGUID(nil)` (arena pet frames), `SetAtlas(nil)` (the minimap
/// outside a house), `SetTexture(file, true, true)` (backdrops), and
/// below (unverified beyond Blizzard's code needing it).
fn relax(f: &Function, args: &mut [Json]) {
    let short = f.name.rsplit(':').next().unwrap_or(f.name);
    let method = f.name.contains(':');
    let mut set = |i: usize, key: &str, value: Json| {
        if let Some(Json::Object(a)) = args.get_mut(i) {
            a.insert(key.to_string(), value);
        }
    };
    match (method, short) {
        (false, "UnitGUID") => set(0, "Nilable", Json::Bool(true)),
        (true, "SetAtlas") => set(0, "Nilable", Json::Bool(true)),
        (true, "SetTexture") => {
            set(1, "Type", Json::from("LuaValueVariant"));
            set(2, "Type", Json::from("LuaValueVariant"));
        }
        // `SetFont(GetFont())` with no font yet (the developer console).
        (true, "SetFont") => set(0, "Nilable", Json::Bool(true)),
        // A texture alone (gamepad action bars; an edge's: Ellesmere UI's
        // cooldowns, which the client runs without an error; a bling's:
        // ElvUI's action bar cooldowns at every login, which the user saw
        // error-free in the client, unverified by a probe).
        (true, "SetSwipeTexture" | "SetEdgeTexture" | "SetBlingTexture") => {
            for i in 1..5 {
                set(i, "Nilable", Json::Bool(true));
            }
        }
        // A channel's name for the sound's sub type (`PlaySound(kit,
        // "Master")`: Ellesmere UI's unlock sound, error-free in the
        // client). Unverified: a name the client does not know.
        (false, "C_Sound.PlaySound") => set(1, "Type", Json::from("LuaValueVariant")),
        // Any value, nil too (UIParentPanelManager stores true and numbers).
        (true, "SetAttributeNoHandler") => {
            set(1, "Nilable", Json::Bool(true));
            set(1, "Type", Json::from("LuaValueVariant"));
        }
        _ => {}
    }
}

/// Integer arguments whose C type the documentation does not tell (both are
/// `number`): unsigned where the probe found them so.
const UNSIGNED: &[(&str, usize)] = &[("C_ChatInfo.IsValidChatLine", 1)];

/// The client's usage line: `Usage: local a, (b)* = C_X.Fn(x [, y])`.
pub fn usage(f: &Function) -> String {
    let list = |key: &str| {
        if key == "Returns" {
            return client_returns(f);
        }
        let mut args = client_arguments(f);
        for &(function, first, count, name) in NAMED_TOGETHER {
            if function == f.name
                && let Some(i) = args
                    .iter()
                    .position(|a| a.get("Name").and_then(Json::as_str) == Some(first))
                && i + count <= args.len()
            {
                args.splice(
                    i..i + count,
                    [serde_json::json!({ "Name": name, "Nilable": false })],
                );
            }
        }
        args
    };
    let name_of = |v: &Json| {
        v.get("Name")
            .and_then(Json::as_str)
            .unwrap_or("?")
            .to_string()
    };
    let stride = |v: &Json| v.get("StrideIndex").is_some();
    // Runs of stride values in parentheses with a star.
    let join_runs = |items: &[Json]| {
        let mut out: Vec<String> = Vec::new();
        let mut i = 0;
        while i < items.len() {
            if stride(&items[i]) {
                let start = i;
                while i < items.len() && stride(&items[i]) {
                    i += 1;
                }
                let names: Vec<String> = items[start..i].iter().map(name_of).collect();
                out.push(format!("({})*", names.join(", ")));
            } else {
                out.push(name_of(&items[i]));
                i += 1;
            }
        }
        out
    };
    let returns = list("Returns");
    let mut text = String::from("Usage: ");
    if !returns.is_empty() {
        text.push_str(&format!("local {} = ", join_runs(&returns).join(", ")));
    }
    // A method's usage names it on `self` (`self:SetAlpha(alpha)`).
    if f.name.contains(':') {
        text.push_str(&format!("self:{}", f.short));
    } else {
        text.push_str(f.name);
    }
    text.push('(');
    // Runs of nilable arguments share brackets: `[a, b], c` and `a [, b]`.
    let args = list("Arguments");
    // Nilable, or with a default.
    let nilable = |v: &Json| {
        v.get("Nilable").and_then(Json::as_bool).unwrap_or(false) || v.get("Default").is_some()
    };
    let mut i = 0;
    while i < args.len() {
        if nilable(&args[i]) {
            let start = i;
            while i < args.len() && nilable(&args[i]) {
                i += 1;
            }
            let names = join_runs(&args[start..i]).join(", ");
            if start == 0 {
                text.push_str(&format!("[{names}]"));
            } else {
                text.push_str(&format!(" [, {names}]"));
            }
        } else {
            if i > 0 {
                text.push_str(", ");
            }
            text.push_str(&join_runs(&args[i..=i]).join(", "));
            i += 1;
        }
    }
    text.push(')');
    text
}

/// An argument as the client checks it: its documentation, how its type
/// takes a value, and whether it may be left out.
pub struct Checked {
    arg: Json,
    family: Family,
    optional: bool,
    /// A packed structure's members (`Family::Packed`).
    members: Vec<Checked>,
}

impl Checked {
    /// How many of Lua's arguments it reads.
    fn width(&self) -> usize {
        match self.family {
            Family::BookItem => 2,
            Family::Packed => self.members.len(),
            _ => 1,
        }
    }
}

/// Documented `number` arguments the client takes otherwise than as a
/// signed 32-bit integer (`data/camelot-number-args.txt`): by function and
/// argument number, `u32`, `float`, `finite` or `plain`.
fn number_kind(function: &str, n: usize) -> Option<&'static str> {
    static KINDS: std::sync::OnceLock<
        std::collections::HashMap<(&'static str, usize), &'static str>,
    > = std::sync::OnceLock::new();
    KINDS
        .get_or_init(|| {
            include_str!("../data/camelot-number-args.txt")
                .lines()
                .filter(|l| !l.starts_with('#') && !l.is_empty())
                .filter_map(|l| {
                    let mut parts = l.split('\t');
                    let f = parts.next()?;
                    let n = parts.next()?.parse().ok()?;
                    Some(((f, n), parts.next()?))
                })
                .collect()
        })
        .get(&(function, n))
        .copied()
}

/// An argument's check: its family by its type, a packed structure's
/// members' too.
fn checked(api: &Api, f: &Function, arg: Json, n: usize, method: bool) -> Checked {
    let ty = arg.get("Type").and_then(Json::as_str).unwrap_or("");
    let mut family = family(api, ty, &arg, method);
    if let Family::Int { index, .. } = family
        && UNSIGNED.contains(&(f.name, n))
    {
        family = Family::Int {
            min: U32.0,
            max: U32.1,
            index,
        };
    }
    // A locale's name: "Invalid locale name 'x'." for "x", "player", -1, 0,
    // 1 and 2 (`C_StringUtil.GetDefaultAbbreviationBreakpoints`, the full
    // probe; which names it knows is unverified: the client's locales).
    if f.name == "C_StringUtil.GetDefaultAbbreviationBreakpoints" && n == 1 {
        family = Family::Locale;
    }
    // The encodings' values are Lua's own checks: `C_EncodingUtil.
    // SerializeJSON()` "value expected" (`SerializeJSON(nil)` answers),
    // `DeserializeJSON({})` "string expected, got table" (the full probe).
    match (f.name, n) {
        ("C_EncodingUtil.SerializeJSON" | "C_EncodingUtil.SerializeCBOR", 1) => {
            family = Family::LuaAny;
        }
        ("C_EncodingUtil.DeserializeJSON" | "C_EncodingUtil.DeserializeCBOR", 1) => {
            family = Family::LuaString;
        }
        _ => {}
    }
    // The client's own C types of documented numbers (the full probe).
    if ty == "number" && !method {
        match number_kind(f.name, n) {
            Some("u32") => {
                family = Family::Int {
                    min: U32.0,
                    max: U32.1,
                    index: false,
                }
            }
            Some("float") => family = Family::Float,
            Some("finite") => family = Family::Finite,
            Some("plain") => family = Family::Number,
            _ => {}
        }
    }
    // A repeated (stride) argument may come no times: Blizzard's
    // `ChannelFrameMixin = CreateFromMixins()` works in the client.
    let optional = arg.get("Nilable").and_then(Json::as_bool).unwrap_or(false)
        || arg.get("Default").is_some()
        || arg.get("StrideIndex").is_some();
    // A packed structure's numbers: any number (`ConvertHSLToHSV(1, 1,
    // 2147483647)` and `(1.5, 1, 1)` answer).
    let members = arg
        .get("Members")
        .and_then(Json::as_array)
        .map(|m| {
            m.iter()
                .map(|a| {
                    let mut c = checked(api, f, a.clone(), 0, method);
                    if a.get("Type").and_then(Json::as_str) == Some("number") {
                        c.family = Family::Number;
                    }
                    c
                })
                .collect()
        })
        .unwrap_or_default();
    Checked {
        arg,
        family,
        optional,
        members,
    }
}

/// The arguments as the client checks them from argument `first` on: the
/// documentation's, or the client's own, relaxed; the same each call, so
/// worked out once.
fn checked_arguments<'f>(api: &Api, f: &'f Function, first: usize) -> &'f [Checked] {
    let method = first == 2;
    f.checked_arguments[first - 1].get_or_init(|| {
        let mut args = client_arguments(f);
        relax(f, &mut args);
        // Each by its first Lua argument's number.
        let mut n = first;
        args.into_iter()
            .map(|arg| {
                let c = checked(api, f, arg, n, method);
                n += c.width();
                c
            })
            .collect()
    })
}

/// Checks a call's arguments as the client does.
pub fn check(vm: &mut Vm, api: &Api, f: &Function, c: &NativeCall) -> Result<(), LuaError> {
    check_from(vm, api, f, c, 1)?;
    own_check(vm, f, c)
}

/// Documented functions' own checks after the bindings': the C code's
/// error, without a position, and for an addon's call the taint line the
/// client adds ("Lua Taint: *** ForceTaint_Strong ***", as recorded for
/// `SetMinMaxValues`). From the full probe (`/stdtest engine fullprobe`):
/// `UnitStat(unit, 0)` and `(unit, 2147483647)` refused, 1 taken (the
/// stats run to 5, unverified); `GetCombatRating(2147483647)` refused,
/// 0 and 1 taken; `GetMirrorTimerInfo(0)` refused, 1 taken (three timers,
/// unverified), `GetMirrorTimerProgress("x")` refused; `GetSpellBonusDamage
/// (0)` refused, 1 taken (seven schools, unverified); a secure addon's
/// CPU or memory usage refused (`GetAddOnCPUUsage("Blizzard_AuctionHouseUI")`).
fn own_check(vm: &mut Vm, f: &Function, c: &NativeCall) -> Result<(), LuaError> {
    let num = |vm: &Vm, i: usize| vm.tonumber(vm.hidden(c.arg(i))).unwrap_or(0.0).trunc();
    let text = |vm: &Vm, i: usize| match vm.hidden(c.arg(i)) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    let refused: Option<String> = match f.name {
        "UnitStat" if !(1.0..=5.0).contains(&num(vm, 2)) => {
            Some("UnitStat(): Invalid stat index in UnitStat".into())
        }
        "GetCombatRating" | "GetCombatRatingBonus" | "GetCombatRatingBonusForCombatRatingValue"
            if num(vm, 1) > 32.0 =>
        {
            Some(format!("{}(): ratingIndex is in the range 1 .. 32", f.name))
        }
        "GetMirrorTimerInfo" if !(1.0..=3.0).contains(&num(vm, 1)) => {
            Some("GetMirrorTimerInfo(): ERROR: Invalid timer (Ex: \"BREATH\"".into())
        }
        "GetMirrorTimerProgress"
            if !matches!(text(vm, 1).as_str(), "BREATH" | "EXHAUSTION" | "FEIGNDEATH") =>
        {
            Some("GetMirrorTimerProgress(): ERROR: Invalid timer (Ex: \"BREATH\"".into())
        }
        "GetSpellBonusDamage" if !(1.0..=7.0).contains(&num(vm, 1)) => {
            Some("GetSpellBonusDamage(): Usage: GetSpellBonusDamage(school)".into())
        }
        "GetAddOnCPUUsage" if text(vm, 1).starts_with("Blizzard_") => {
            Some("GetAddOnCPUUsage(): Cannot get CPU usage for secure AddOns".into())
        }
        "GetAddOnMemoryUsage" if text(vm, 1).starts_with("Blizzard_") => {
            Some("GetAddOnMemoryUsage(): Cannot get memory usage for secure AddOns".into())
        }
        _ => None,
    };
    let Some(mut msg) = refused else {
        return Ok(());
    };
    if !crate::protection::blizzard_caller(vm) {
        msg.push_str("\nLua Taint: *** ForceTaint_Strong ***");
    }
    let v = vm.str(&msg);
    Err(vm.raise(v))
}

/// [`check`] with the documented arguments from argument `first` on (2 for
/// a method: `self` comes first). Errors number the documented arguments
/// (unverified for methods).
pub fn check_from(
    vm: &mut Vm,
    api: &Api,
    f: &Function,
    c: &NativeCall,
    first: usize,
) -> Result<(), LuaError> {
    // Errors number Lua's arguments: a method's count `self` as the first
    // (`/stdtest engine`), and a spell book item takes two.
    let mut at = first;
    for checked in checked_arguments(api, f, first) {
        let n = at;
        at += checked.width();
        let (arg, family) = (&checked.arg, checked.family);
        if family == Family::LuaAny {
            if !c.has(n) {
                return Err(fail(
                    vm,
                    api,
                    f,
                    n,
                    Some(Refusal::Lua("value expected".into())),
                ));
            }
            continue;
        }
        if family == Family::LuaString {
            let v = vm.hidden(c.arg(n));
            if !matches!(v, Value::Str(_) | Value::Num(_)) {
                let got = if c.has(n) { v.type_name() } else { "no value" };
                let text = format!("string expected, got {got}");
                return Err(fail(vm, api, f, n, Some(Refusal::Lua(text))));
            }
            continue;
        }
        if family == Family::Packed {
            for (k, member) in checked.members.iter().enumerate() {
                let v = vm.hidden(c.arg(n + k));
                let refused = if v.is_nil() {
                    !member.optional
                } else {
                    refuses_type(vm, member.family, v).is_some()
                };
                if refused {
                    return Err(fail(vm, api, f, n, None));
                }
            }
            continue;
        }
        let v = c.arg(n);
        let present = c.has(n) && !vm.hidden(v).is_nil();
        if !present {
            if checked.optional {
                continue;
            }
            return Err(fail(vm, api, f, n, None));
        }
        // Secrets: refused first, but by plain structures and widgets only
        // through their type check.
        let type_first = matches!(family, Family::Table { mixin: false } | Family::Object);
        if let Value::Secret(_) = v
            && !type_first
            && let Some(why) = refuses_secret(vm, f, arg)
        {
            return Err(fail(vm, api, f, n, Some(Refusal::Secret(why))));
        }
        let h = if type_first { v } else { vm.hidden(v) };
        if let Some(r) = refuses_type(vm, family, h) {
            return Err(fail(vm, api, f, n, Some(r)));
        }
        if let (Family::Table { mixin: false }, Value::Table(_)) = (family, h)
            && let Some(field) = missing_field(vm, api, arg, h)
        {
            // Refused as the argument after the last given: the field's
            // value pushed on top of Lua's arguments
            // (`C_Club.GetMessageInfo("", "", {})` refuses #4 with "Current
            // Field: [epoch]", `C_DateAndTime.CompareCalendarTime({}, {})` #3
            // with `monthDay`, the first field the documentation requires;
            // all 55 of the full probe's).
            return Err(fail(vm, api, f, c.len() + 1, Some(Refusal::Field(field))));
        }
        // A spell book item's bank: a number (nil refused as well), one of
        // `Enum.SpellBookSpellBank` (the full probe's `(1, "x")`, `(1, nil)`
        // and `(1, 2)`; the slot is checked first, `(-1, -1)` refused for
        // its range; SuuUI's spellbook probe: a slot alone refused). How a
        // fraction prints is unverified.
        if family == Family::BookItem {
            let bank = vm.hidden(c.arg(n + 1));
            let refusal = match vm.tonumber(bank) {
                None => Some(Refusal::NotNumerical),
                Some(b) if !(0.0..=1.0).contains(&b.trunc()) => Some(Refusal::SpellBank(b.trunc())),
                Some(_) => None,
            };
            if let Some(r) = refusal {
                return Err(fail(vm, api, f, n, Some(r)));
            }
        }
    }
    Ok(())
}

/// A structure's first field the documentation requires (neither
/// nilable nor defaulted) that a table lacks.
fn missing_field(vm: &mut Vm, api: &Api, arg: &Json, table: Value) -> Option<String> {
    let Value::Table(t) = table else {
        return None;
    };
    let ty = arg.get("Type").and_then(Json::as_str)?;
    for field in api.structure_fields.get(ty)? {
        let required = !field
            .get("Nilable")
            .and_then(Json::as_bool)
            .unwrap_or(false)
            && field.get("Default").is_none();
        let Some(name) = field.get("Name").and_then(Json::as_str) else {
            continue;
        };
        let k = vm.str(name);
        if required && vm.heap.table(t).get(k).is_nil() {
            return Some(name.to_string());
        }
    }
    None
}

enum Refusal {
    /// Lua's own message alone (`luaL_checkany`).
    Lua(String),
    Field(String),
    Text(String),
    Message(&'static str),
    Type,
    Range(f64, f64),
    FloatRange,
    NotNumerical,
    Finite,
    SpellBank(f64),
    String,
    This,
    Secret(&'static str),
}

/// Why a secret may not be passed here, if it may not.
fn refuses_secret(vm: &Vm, f: &Function, arg: &Json) -> Option<&'static str> {
    if arg.get("NeverSecret").and_then(Json::as_bool) == Some(true) {
        return Some("never allowed");
    }
    match f.doc.get("SecretArguments").and_then(Json::as_str) {
        Some("NotAllowed") => Some("not allowed"),
        Some("AllowedWhenTainted") => None,
        _ if vm.can_access_secrets() => None,
        _ => Some("only allowed during untainted execution"),
    }
}

fn refuses_type(vm: &mut Vm, family: Family, v: Value) -> Option<Refusal> {
    let is_str = matches!(v, Value::Str(_) | Value::Num(_));
    match family {
        Family::Any | Family::Packed | Family::LuaAny | Family::LuaString => None,
        Family::ScriptObject => crate::widgets::widget_of(vm, v)
            .is_none()
            .then_some(Refusal::Type),
        Family::ItemLocation { azerite } => {
            let Value::Table(t) = v else {
                return Some(Refusal::Type);
            };
            let mut field = |key: &str| {
                let k = vm.str(key);
                !vm.heap.table(t).get(k).is_nil()
            };
            let located = field("equipmentSlotIndex") || (field("bagID") && field("slotIndex"));
            (azerite || !located).then_some(Refusal::Type)
        }
        Family::Filled => match v {
            Value::Table(t) => vm
                .heap
                .table(t)
                .entries()
                .next()
                .is_none()
                .then_some(Refusal::Type),
            _ => Some(Refusal::Type),
        },
        Family::Bounded { min, max } => match vm.tonumber(v) {
            Some(n) if (min..=max).contains(&n) => None,
            _ => Some(Refusal::Type),
        },
        Family::Finite => match vm.tonumber(v) {
            Some(n) if n.is_finite() => None,
            _ => Some(Refusal::Finite),
        },
        Family::Enum { min, max } => {
            let n = vm.tonumber(v).unwrap_or(0.0).trunc();
            (!(min..=max).contains(&n)).then_some(Refusal::Type)
        }
        Family::Int { min, max, index } => {
            let Some(n) = vm.tonumber(v) else {
                return Some(Refusal::Range(min, max));
            };
            // A Lua index's range is the raw value's, 0 included (the
            // client logs no error for Blizzard's gamepad bars registering
            // action 0, `/stdtest engine` `blizzardErrors`).
            let _ = index;
            (!(min..=max).contains(&n)).then_some(Refusal::Range(min, max))
        }
        Family::Number => vm.tonumber(v).is_none().then_some(Refusal::Type),
        // The slot: a number in an unsigned integer's range.
        Family::BookItem => match vm.tonumber(v) {
            None => Some(Refusal::NotNumerical),
            Some(n) if !(U32.0..=U32.1).contains(&n) => Some(Refusal::Range(U32.0, U32.1)),
            Some(_) => None,
        },
        Family::Float => match vm.tonumber(v) {
            Some(n) if n.abs() <= f64::from(f32::MAX) => None,
            _ => Some(Refusal::FloatRange),
        },
        Family::Func => (!matches!(v, Value::Func(_))).then_some(Refusal::Type),
        Family::Str => (!is_str).then_some(Refusal::Type),
        Family::StrictStr => (!is_str).then_some(Refusal::String),
        Family::FontObject => {
            let font = match v {
                Value::Table(_) => crate::widgets::widget_of(vm, v).is_some(),
                Value::Str(s) => {
                    let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
                    let g = vm.get_global(&name);
                    crate::widgets::widget_of(vm, g).is_some()
                }
                _ => false,
            };
            (!font).then_some(Refusal::Type)
        }
        Family::FollowerId => match v {
            Value::Str(s) if vm.str_bytes(s).starts_with(b"0x") => None,
            _ => Some(Refusal::Type),
        },
        Family::Locale => {
            const LOCALES: &[&[u8]] = &[
                b"enUS", b"enGB", b"deDE", b"esES", b"esMX", b"frFR", b"itIT", b"koKR", b"ptBR",
                b"ruRU", b"zhCN", b"zhTW",
            ];
            let name = match v {
                Value::Str(s) => vm.str_bytes(s).to_vec(),
                Value::Num(_) => match vm.tostring(v) {
                    Ok(Value::Str(s)) => vm.str_bytes(s).to_vec(),
                    _ => return Some(Refusal::Type),
                },
                _ => return Some(Refusal::Type),
            };
            (!LOCALES.contains(&name.as_slice())).then(|| {
                Refusal::Text(format!(
                    "Invalid locale name '{}'.",
                    String::from_utf8_lossy(&name)
                ))
            })
        }
        Family::NamePlateUnit => {
            if !is_str {
                return Some(Refusal::String);
            }
            let token = match v {
                Value::Str(s) => vm.str_bytes(s).to_ascii_lowercase(),
                _ => return None,
            };
            let letters = token.iter().take_while(|c| c.is_ascii_alphabetic()).count();
            let digits = token[letters..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .count();
            let base = &token[..letters];
            if digits > 0 && matches!(base, b"party" | b"raid") {
                Some(Refusal::Message(
                    "Raid<n>/Party<n> unit tokens are not allowed for this call.",
                ))
            } else if letters + digits != token.len() {
                Some(Refusal::Message(
                    "Compound unit tokens (example: boss1targetpet) are not allowed for this call.",
                ))
            } else {
                None
            }
        }
        Family::Table { .. } => (!matches!(v, Value::Table(_))).then_some(Refusal::Type),
        Family::Font => {
            let font = match v {
                Value::Table(_) => true,
                Value::Str(s) => {
                    let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
                    matches!(vm.get_global(&name), Value::Table(_))
                }
                _ => false,
            };
            (!font).then_some(Refusal::Type)
        }
        Family::Object => {
            let object = match v {
                Value::Table(t) => {
                    matches!(vm.heap.table(t).get(Value::Num(0.0)), Value::Userdata(_))
                }
                _ => false,
            };
            (!object).then_some(Refusal::This)
        }
    }
}

/// Functions whose argument errors are the bindings' own ("bad argument #N
/// to '?'", no position) though the rule in [`names_itself`] says Lua's,
/// and the other way round (`/stdtest engine fullprobe`: each function
/// raised every error of the run the one way).
const UNNAMED: &[&str] = &[
    "C_AccountStore.GetCurrencyInfo",
    "C_AccountStore.GetItemInfo",
    "C_ActionBar.GetActionCharges",
    "C_ActionBar.GetActionCooldown",
    "C_ActionBar.GetActionLossOfControlCooldownInfo",
    "C_AddOns.GetAddOnDependencies",
    "C_AddOns.GetAddOnOptionalDependencies",
    "C_ArtifactUI.GetPowerInfo",
    "C_ArtifactUI.GetPowersAffectedByRelic",
    "C_AzeriteEssence.GetMilestoneInfo",
    "C_BattleNet.GetFriendInviteInfo",
    "C_Calendar.GetHolidayInfo",
    "C_CampaignInfo.GetCampaignInfo",
    "C_CatalogShop.FindBestCurrencyProductForNeededAmount",
    "C_CatalogShop.GetSpellVisualInfoForMount",
    "C_ClassColor.GetClassColor",
    "C_ConsoleScriptCollection.GetCollectionDataByTag",
    "C_Container.GetContainerItemPurchaseCurrency",
    "C_Container.GetContainerItemQuestInfo",
    "C_ContentTracking.GetEncounterTrackingInfo",
    "C_ContributionCollector.GetBuffs",
    "C_ContributionCollector.GetManagedContributionsForCreatureID",
    "C_CraftingOrders.GetOrderClaimInfo",
    "C_CraftingOrders.RequestCrafterOrders",
    "C_CraftingOrders.RequestCustomerOrders",
    "C_CreatureInfo.GetCreatureFamilyInfo",
    "C_CurrencyInfo.GetCurrencyInfo",
    "C_CurrencyInfo.GetCurrencyInfoFromLink",
    "C_CurrencyInfo.GetCurrencyListInfo",
    "C_DyeColor.GetDyeColorInfo",
    "C_EncounterTimeline.GetTrackInfo",
    "C_EncounterWarnings.GetEditModeWarningInfo",
    "C_Item.GetAppliedItemTransmogInfo",
    "C_Item.GetBaseItemTransmogInfo",
    "C_Item.GetCurrentItemTransmogInfo",
    "C_Item.GetItemStatDelta",
    "C_Item.GetItemStats",
    "C_LossOfControl.GetActiveLossOfControlData",
    "C_LossOfControl.GetActiveLossOfControlDataByUnit",
    "C_Map.GetUserWaypointFromHyperlink",
    "C_Minimap.GetTrackingFilter",
    "C_Minimap.GetTrackingInfo",
    "C_ModelInfo.GetModelSceneActorDisplayInfoByID",
    "C_ModelInfo.GetModelSceneActorInfoByID",
    "C_MountJournal.GetDisplayedMountInfo",
    "C_MountJournal.GetDisplayedMountInfoExtra",
    "C_MountJournal.GetMountInfoByID",
    "C_MountJournal.GetMountInfoExtraByID",
    "C_PetJournal.GetPetInfoTableByPetID",
    "C_PetJournal.GetPetInfoTableBySpeciesID",
    "C_PvP.GetBattlegroundInfo",
    "C_QuestInfoSystem.GetQuestRewardSpellInfo",
    "C_QuestLog.GetQuestDetailsTheme",
    "C_QuestLog.GetQuestRewardCurrencyInfo",
    "C_QuestLog.GetQuestTagInfo",
    "C_QuestOffer.GetQuestRewardCurrencyInfo",
    "C_Spell.GetItemCooldown",
    "C_Spell.GetSpellCharges",
    "C_Spell.GetSpellCooldown",
    "C_Spell.GetSpellLossOfControlCooldownInfo",
    "C_SpellBook.GetSpellBookItemCharges",
    "C_SpellBook.GetSpellBookItemCooldown",
    "C_SpellBook.GetSpellBookItemLossOfControlCooldownInfo",
    "C_TTSSettings.ShouldOverrideMessage",
    "C_Texture.GetTitleIconTexture",
    "C_Texture.IsTitleIconTextureReady",
    "C_TradeSkillUI.GetProfessionInfoByRecipeID",
    "C_TradeSkillUI.GetProfessionInfoBySkillLineID",
    "C_TradeSkillUI.GetRecipeOutputItemData",
    "C_TradeSkillUI.IsEnchantTargetValid",
    "C_TradeSkillUI.IsRecraftReagentValid",
    "C_Transmog.GetSlotVisualInfo",
    "C_TransmogCollection.GetCustomSetHyperlinkFromItemTransmogInfoList",
    "C_TransmogOutfitInfo.GetViewedOutfitSlotInfo",
    "C_UnitAuras.DoesAuraHaveExpirationTime",
    "C_UnitAuras.GetAuraBaseDuration",
    "C_UnitAuras.GetAuraCasterGUID",
    "C_UnitAuras.GetAuraDispelTypeColor",
    "C_UnitAuras.GetAuraSlots",
    "C_UnitAuras.GetRefreshCarryOverDuration",
    "C_UnitAuras.GetRefreshExtendedDuration",
    "C_VignetteInfo.FindBestUniqueVignette",
    "C_VignetteInfo.GetVignetteInfo",
    "GetDifficultyInfo",
    "GetMouseButtonName",
    "IsMouseButtonDown",
    "UnitChannelInfo",
    "UnitGetDetailedHealPrediction",
    "UnitHealthPercent",
    "UnitPowerPercent",
];
const NAMED: &[&str] = &[
    "C_EncodingUtil.SerializeCBOR",
    "C_EncodingUtil.SerializeJSON",
];

/// Whether a function's argument errors are Lua's own (`luaL_argerror`:
/// the caller's position and the name it was called by, as a direct call
/// shows them; through `pcall`, neither) or the bindings' ("bad argument
/// #N to '?'", never a position). The full probe called each function
/// directly (`return F(…)`): of 1716 documented functions that refused an
/// argument, those handing Lua a string or table they build (a `string`
/// or `stringView`, not a `cstring`; a table; a duration object; a
/// structure holding one of these) or taking an item by ID, name or link
/// (`ItemInfo`) raised the bindings' error, the others Lua's, but for
/// the 92 listed above. Unverified for functions the probe did not refuse.
fn names_itself(api: &Api, f: &Function) -> bool {
    if UNNAMED.contains(&f.name) {
        return false;
    }
    if NAMED.contains(&f.name) {
        return true;
    }
    fn built(api: &Api, ty: &str, depth: usize) -> bool {
        const BUILT: &[&str] = &[
            "string",
            "stringView",
            "table",
            "LuaDurationObject",
            "TooltipData",
            "AuraData",
        ];
        if BUILT.contains(&ty) {
            return true;
        }
        depth < 4
            && api.structure_fields.get(ty).is_some_and(|fields| {
                fields.iter().any(|field| {
                    ["Type", "InnerType"].iter().any(|k| {
                        field
                            .get(*k)
                            .and_then(Json::as_str)
                            .is_some_and(|t| built(api, t, depth + 1))
                    })
                })
            })
    }
    let types = |key: &str| -> Vec<String> {
        f.doc
            .get(key)
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.get("Type").and_then(Json::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    !(types("Returns").iter().any(|t| built(api, t, 0))
        || types("Arguments").iter().any(|t| t == "ItemInfo"))
}

fn fail(vm: &mut Vm, api: &Api, f: &Function, n: usize, why: Option<Refusal>) -> LuaError {
    let usage = usage(f);
    let msg = match &why {
        None | Some(Refusal::Type) => usage,
        Some(Refusal::FloatRange) => {
            format!("outside of expected range -3.402823e+38 to 3.402823e+38 - {usage}")
        }
        Some(Refusal::Range(min, max)) => {
            format!("outside of expected range {min} to {max} - {usage}")
        }
        Some(Refusal::String) => format!("Expected a string. - {usage}"),
        Some(Refusal::NotNumerical) => format!("not a numerical value - {usage}"),
        Some(Refusal::Finite) => format!("must be a finite number - {usage}"),
        Some(Refusal::Field(field)) => format!("Current Field: [{field}], {usage}"),
        Some(Refusal::Message(text)) => format!("{text} - {usage}"),
        Some(Refusal::Text(text)) => format!("{text} - {usage}"),
        Some(Refusal::Lua(text)) => text.clone(),
        Some(Refusal::SpellBank(bank)) => format!(
            "Invalid spellBank value {bank}. Must be an Enum.SpellBookSpellBank value. - {usage}"
        ),
        Some(Refusal::This) => format!(
            "Attempt to find 'this' in non-table object (used '.' instead of ':' ?) - {usage}"
        ),
        Some(Refusal::Secret(why)) => {
            format!("{usage}. Secret values are {why} for this argument.")
        }
    };
    // A method's range error is Lua's own (`s:SetValue(math.huge)` called
    // directly names itself, `/stdtest engine answers`), its others the
    // bindings' (unverified beyond the recordings).
    let named = if f.name.contains(':') {
        matches!(why, Some(Refusal::Range(..) | Refusal::FloatRange))
    } else {
        names_itself(api, f)
    };
    if named {
        return vm.arg_error(n, &msg);
    }
    // The bindings' own error is cut at 199 bytes, its closing parenthesis
    // too: all 65 of the full probe's longer ones were, Lua's own never
    // (208 longer, whole). Through `pcall` Lua's own reads "to '?'" as
    // well, which is why argcheck saw some cut and some whole.
    let mut text = format!("bad argument #{n} to '?' ({msg})").into_bytes();
    if text.len() > 199 {
        text.truncate(199);
    }
    let v = vm.bytes(&text);
    vm.raise(v)
}

/// An undocumented function's own check of one argument
/// (`data/camelot-usages.txt`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Usage {
    /// A number or a numeric string (`lua_isnumber`).
    Number,
    /// A number above 0.
    Index,
    /// A number of 0 or above.
    NonNegative,
    /// A number but 0 (`GetShapeshiftFormInfo(0)` refused, `(-1)` taken:
    /// an unsigned index counted from 1).
    NonZero,
    /// A string or a number (`lua_isstring`).
    String,
    /// A string.
    StrictString,
    Table,
    /// "CRITTER" or "MOUNT" (unverified: the probe passed neither).
    Companion,
    /// Every value refused (all the probe passed).
    Never,
    /// Any value but nil.
    Required,
    /// A number or none (`luaL_optnumber`): Lua's own message.
    OptNumber,
    /// A quest window item's type ("required", "choice", "reward").
    QuestItem,
    /// A taxi node's slot: refused while no flight map is open.
    TaxiSlot,
    /// A GUID (`BattlePet-…`), or 0, which reads as none
    /// (`C_PetJournal.PetIsHurt(0)` answers, `1` and `"x"` are refused;
    /// a GUID of the right kind unverified: the probe passed none).
    Guid,
    Any,
}

/// An undocumented function's checks: by argument, whether it may be nil
/// or left out, and the message.
struct UsageChecks {
    args: Vec<(Usage, bool)>,
    message: &'static str,
}

/// The undocumented functions' checks by name, in order, camelot's (the
/// full probe's).
fn usage_checks() -> &'static std::collections::HashMap<&'static str, Vec<UsageChecks>> {
    static CHECKS: std::sync::OnceLock<std::collections::HashMap<&'static str, Vec<UsageChecks>>> =
        std::sync::OnceLock::new();
    CHECKS.get_or_init(|| {
        let mut out = std::collections::HashMap::new();
        for line in include_str!("../data/camelot-usages.txt").lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(3, '\t');
            let (Some(name), Some(checks), Some(message)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let args = checks
                .split(',')
                .map(|c| {
                    let optional = c.ends_with('?');
                    let usage = match c.trim_end_matches('?') {
                        "number" => Usage::Number,
                        "index" => Usage::Index,
                        "nonneg" => Usage::NonNegative,
                        "nonzero" => Usage::NonZero,
                        "string" => Usage::String,
                        "strictstr" => Usage::StrictString,
                        "table" => Usage::Table,
                        "companion" => Usage::Companion,
                        "never" => Usage::Never,
                        "required" => Usage::Required,
                        "optnumber" => Usage::OptNumber,
                        "questitem" => Usage::QuestItem,
                        "taxislot" => Usage::TaxiSlot,
                        "guid" => Usage::Guid,
                        _ => Usage::Any,
                    };
                    (usage, optional)
                })
                .collect();
            out.entry(name)
                .or_insert_with(Vec::new)
                .push(UsageChecks { args, message });
        }
        out
    })
}

/// Gives the client's undocumented functions that check their arguments
/// (`data/camelot-usages.txt`) those checks: each keeps its behaviour,
/// asked after them.
pub(crate) fn install_usages(vm: &mut Vm) {
    for &name in usage_checks().keys() {
        let (table, short) = match name.split_once('.') {
            Some((ns, short)) => match vm.get_global(ns) {
                Value::Table(t) => (t, short),
                _ => continue,
            },
            None => (vm.globals, name),
        };
        let k = vm.heap.intern(short.as_bytes());
        let Value::Func(fr) = vm.heap.table(table).get_str(k) else {
            continue;
        };
        // Only the engine's own natives, called by this name.
        let wowlua_vm::function::Function::Native(n) = vm.heap.func(fr) else {
            continue;
        };
        let named = match n.upvals.first() {
            Some(&Value::Str(s)) => vm.heap.str_bytes(s) == name.as_bytes(),
            _ => false,
        };
        let inner = n.f;
        if !named || crate::engine(vm).usage_checked.contains_key(name) {
            continue;
        }
        crate::engine_mut(vm).usage_checked.insert(name, inner);
        if let wowlua_vm::function::Function::Native(n) = vm.heap.func_mut(fr) {
            n.f = usage_checked;
        }
    }
}

/// An undocumented function with checks: a refused argument raises the
/// client's usage (Lua's `luaL_error`, the caller's position first: `return
/// TaxiNodeGetType("x")` in the full probe), else its behaviour answers.
fn usage_checked(vm: &mut Vm, c: NativeCall) -> Result<Vec<Value>, LuaError> {
    let name = match c.upvals.first() {
        Some(&Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    let Some(all) = usage_checks().get(name.as_str()) else {
        return Err(vm.error("an undocumented function without its checks"));
    };
    for checks in all {
        usage_check(vm, checks, &c)?;
    }
    let inner = crate::engine(vm).usage_checked.get(name.as_str()).copied();
    match inner {
        Some(f) => f(vm, c),
        None => Err(vm.error(&format!("{name} is not mocked"))),
    }
}

/// One of an undocumented function's checks.
fn usage_check(vm: &mut Vm, checks: &UsageChecks, c: &NativeCall) -> Result<(), LuaError> {
    for (i, &(usage, optional)) in checks.args.iter().enumerate() {
        let v = vm.hidden(c.arg(i + 1));
        if usage == Usage::OptNumber {
            if !v.is_nil() && vm.tonumber(v).is_none() {
                return Err(vm.type_arg_error(i + 1, "number", Some(v)));
            }
            continue;
        }
        let ok = match (usage, v) {
            (Usage::Any, _) => true,
            (Usage::Never, _) => false,
            (Usage::TaxiSlot, _) => crate::engine(vm).world.taxis.open.is_some(),
            (_, Value::Nil) => optional,
            (Usage::Required | Usage::OptNumber, _) => true,
            (Usage::Guid, Value::Num(n)) => n == 0.0,
            (Usage::Guid, Value::Str(s)) => {
                let b = vm.str_bytes(s);
                let prefix = b.iter().take_while(|c| c.is_ascii_alphabetic()).count();
                b == b"0" || (prefix > 0 && b.get(prefix) == Some(&b'-'))
            }
            (Usage::Guid, _) => false,
            (Usage::Number, v) => vm.tonumber(v).is_some(),
            (Usage::Index, v) => vm.tonumber(v).is_some_and(|n| n > 0.0),
            (Usage::NonNegative, v) => vm.tonumber(v).is_some_and(|n| n >= 0.0),
            (Usage::NonZero, v) => vm.tonumber(v).is_some_and(|n| n != 0.0),
            (Usage::String, v) => matches!(v, Value::Str(_) | Value::Num(_)),
            (Usage::StrictString, v) => matches!(v, Value::Str(_)),
            (Usage::Table, v) => matches!(v, Value::Table(_)),
            (Usage::Companion, Value::Str(s)) => matches!(vm.str_bytes(s), b"CRITTER" | b"MOUNT"),
            (Usage::Companion, _) => false,
            (Usage::QuestItem, Value::Str(s)) => {
                matches!(vm.str_bytes(s), b"required" | b"choice" | b"reward")
            }
            (Usage::QuestItem, _) => false,
        };
        if !ok {
            return Err(vm.error(checks.message));
        }
    }
    Ok(())
}
