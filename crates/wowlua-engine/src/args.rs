//! The client's checks of documented functions' arguments, as
//! `/stdtest argcheck` recorded them on WoW Forever 1.60.1 (fixture
//! `tests/client/forever-camelot-argcheck.lua`): a missing argument, a
//! secret where the function refuses one, or a value its C type does not
//! take raises "bad argument #N to 'name' (<message>Usage: …)", the usage
//! made from the documentation. (The client cuts some secret-argument
//! messages at 199 bytes, but not all: which is not known, so none are.)
//!
//! The check depends on the argument's documented type, as the client's
//! C bindings differ: integers check their range, the restricted unit
//! tokens want a string, enums and some IDs take anything. Types the probe
//! did not reach follow their family (by their alias) and are unverified.

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
    /// A number, a non-number refused as "not a numerical value".
    Numerical,
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
    /// Anything: enums, booleans, some IDs, Lua values.
    Any,
}

const U32: (f64, f64) = (0.0, 4_294_967_295.0);
const I32: (f64, f64) = (-2_147_483_648.0, 2_147_483_647.0);

fn family(api: &Api, ty: &str, arg: &Json, method: bool) -> Family {
    let int = |(min, max): (f64, f64), index| Family::Int { min, max, index };
    match ty {
        "luaIndex" => return int(U32, true),
        "__numerical" => return Family::Numerical,
        "number" if method => return Family::Float,
        "number" => return int(I32, false),
        "EncounterTimelineEventID" => return int(U32, false),
        "WOWMONEY" | "BigUInteger" => return Family::Number,
        "CalendarEventID" | "ClubId" | "WeeklyRewardItemDBID" | "InventorySlots" => {
            return Family::Any;
        }
        "SimpleFont" => return Family::Font,
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
        // The client's own structures the probe met.
        "TooltipComparisonItem" | "PlayerLocation" => return Family::Table { mixin: true },
        "cstring" | "string" | "stringView" => return Family::Str,
        _ => {}
    }
    if api.enum_names.contains(ty) {
        return Family::Any;
    }
    if api.callbacks.contains(ty) {
        return Family::Func;
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

/// Functions whose client takes other arguments than the documentation
/// says (`/stdtest restrictions`): their arguments as the client checks
/// them.
fn overridden_arguments(name: &str) -> Option<Json> {
    match name {
        // One spell book item, not a slot and a bank.
        "C_Secrets.ShouldSpellBookItemCooldownBeSecret" => Some(serde_json::json!([
            { "Name": "spellBookItem", "Type": "__numerical", "Nilable": false }
        ])),
        _ => None,
    }
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
        // cooldowns, which the client runs without an error).
        (true, "SetSwipeTexture" | "SetEdgeTexture") => {
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
        if key == "Arguments"
            && let Some(Json::Array(a)) = overridden_arguments(f.name)
        {
            return a;
        }
        f.doc
            .get(key)
            .and_then(Json::as_array)
            .cloned()
            .unwrap_or_default()
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
}

/// The arguments as the client checks them from argument `first` on: the
/// documentation's, or the client's own, relaxed; the same each call, so
/// worked out once.
fn checked_arguments<'f>(api: &Api, f: &'f Function, first: usize) -> &'f [Checked] {
    let method = first == 2;
    f.checked_arguments[first - 1].get_or_init(|| {
        let mut args = match overridden_arguments(f.name) {
            Some(Json::Array(a)) => a,
            _ => f
                .doc
                .get("Arguments")
                .and_then(Json::as_array)
                .cloned()
                .unwrap_or_default(),
        };
        relax(f, &mut args);
        args.into_iter()
            .enumerate()
            .map(|(i, arg)| {
                let n = if method { i + first } else { i + 1 };
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
                // A repeated (stride) argument may come no times: Blizzard's
                // `ChannelFrameMixin = CreateFromMixins()` works in the client.
                let optional = arg.get("Nilable").and_then(Json::as_bool).unwrap_or(false)
                    || arg.get("Default").is_some()
                    || arg.get("StrideIndex").is_some();
                Checked {
                    arg,
                    family,
                    optional,
                }
            })
            .collect()
    })
}

/// Checks a call's arguments as the client does.
pub fn check(vm: &mut Vm, api: &Api, f: &Function, c: &NativeCall) -> Result<(), LuaError> {
    check_from(vm, api, f, c, 1)
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
    // A spell book item's slot and bank read as one value: without the
    // bank the first is refused (SuuUI probe spellbook: these three asked
    // with a spell ID alone, 46 items each).
    const BOOK_ITEM: &[&str] = &[
        "C_SpellBook.GetSpellBookItemLevelLearned",
        "C_SpellBook.GetSpellBookItemPowerCost",
        "C_SpellBook.IsSpellBookItemLowRank",
    ];
    let given = |i: usize| c.has(i) && !vm.hidden(c.arg(i)).is_nil();
    if first == 1 && BOOK_ITEM.contains(&f.name) && given(1) && !given(2) {
        return Err(fail(vm, f, 1, Some(Refusal::NotNumerical)));
    }
    // A method's arguments count `self` as the first (`/stdtest engine`).
    let method = first == 2;
    for (i, checked) in checked_arguments(api, f, first).iter().enumerate() {
        let at = i + first;
        let n = if method { at } else { i + 1 };
        let (arg, family) = (&checked.arg, checked.family);
        let v = c.arg(at);
        let present = c.has(at) && !vm.hidden(v).is_nil();
        if !present {
            if checked.optional {
                continue;
            }
            return Err(fail(vm, f, n, None));
        }
        // Secrets: refused first, but by plain structures and widgets only
        // through their type check.
        let type_first = matches!(family, Family::Table { mixin: false } | Family::Object);
        if let Value::Secret(_) = v
            && !type_first
            && let Some(why) = refuses_secret(vm, f, arg)
        {
            return Err(fail(vm, f, n, Some(Refusal::Secret(why))));
        }
        let h = if type_first { v } else { vm.hidden(v) };
        if let Some(r) = refuses_type(vm, family, h) {
            return Err(fail(vm, f, n, Some(r)));
        }
    }
    Ok(())
}

enum Refusal {
    Type,
    Range(f64, f64),
    FloatRange,
    NotNumerical,
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
        Family::Any => None,
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
        Family::Numerical => vm.tonumber(v).is_none().then_some(Refusal::NotNumerical),
        Family::Float => match vm.tonumber(v) {
            Some(n) if n.abs() <= f64::from(f32::MAX) => None,
            _ => Some(Refusal::FloatRange),
        },
        Family::Func => (!matches!(v, Value::Func(_))).then_some(Refusal::Type),
        Family::Str => (!is_str).then_some(Refusal::Type),
        Family::StrictStr => (!is_str).then_some(Refusal::String),
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

fn fail(vm: &mut Vm, f: &Function, n: usize, why: Option<Refusal>) -> LuaError {
    let usage = usage(f);
    // A range error is Lua's own (`luaL_argerror`: the caller's position and
    // the function's name, a method's self not counted; through `pcall`,
    // neither): `s:SetValue(math.huge)` and `GetActionAutocast("x")` called
    // directly name themselves (`/stdtest engine answers`, argcheck). The
    // others are the bindings', with neither.
    if let Some(Refusal::FloatRange) = why {
        return vm.arg_error(
            n,
            &format!("outside of expected range -3.402823e+38 to 3.402823e+38 - {usage}"),
        );
    }
    if let Some(Refusal::Range(min, max)) = why {
        return vm.arg_error(
            n,
            &format!("outside of expected range {min} to {max} - {usage}"),
        );
    }
    let msg = match why {
        None | Some(Refusal::Type) => usage,
        Some(Refusal::Range(..) | Refusal::FloatRange) => unreachable!("raised above"),
        Some(Refusal::String) => format!("Expected a string. - {usage}"),
        Some(Refusal::NotNumerical) => format!("not a numerical value - {usage}"),
        Some(Refusal::This) => format!(
            "Attempt to find 'this' in non-table object (used '.' instead of ':' ?) - {usage}"
        ),
        Some(Refusal::Secret(why)) => {
            format!("{usage}. Secret values are {why} for this argument.")
        }
    };
    vm.api_arg_error(n, &msg)
}
