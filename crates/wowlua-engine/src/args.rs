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

/// Integer arguments whose C type the documentation does not tell (both are
/// `number`): unsigned where the probe found them so.
const UNSIGNED: &[(&str, usize)] = &[("C_ChatInfo.IsValidChatLine", 1)];

/// The client's usage line: `Usage: local a, (b)* = C_X.Fn(x [, y])`.
pub fn usage(f: &Function) -> String {
    let list = |key: &str| {
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
    let args = f
        .doc
        .get("Arguments")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    // A method's arguments count `self` as the first (`/stdtest engine`).
    let method = first == 2;
    for (i, arg) in args.iter().enumerate() {
        let at = i + first;
        let n = if method { at } else { i + 1 };
        let ty = arg.get("Type").and_then(Json::as_str).unwrap_or("");
        let mut family = family(api, ty, arg, method);
        if let Family::Int { index, .. } = family
            && UNSIGNED.contains(&(f.name, n))
        {
            family = Family::Int {
                min: U32.0,
                max: U32.1,
                index,
            };
        }
        let v = c.arg(at);
        let present = c.has(at) && !vm.hidden(v).is_nil();
        if !present {
            let optional = arg.get("Nilable").and_then(Json::as_bool).unwrap_or(false)
                || arg.get("Default").is_some();
            if optional {
                continue;
            }
            return Err(fail(vm, f, n, None, method));
        }
        // Secrets: refused first, but by plain structures and widgets only
        // through their type check.
        let type_first = matches!(family, Family::Table { mixin: false } | Family::Object);
        if let Value::Secret(_) = v
            && !type_first
            && let Some(why) = refuses_secret(vm, f, arg)
        {
            return Err(fail(vm, f, n, Some(Refusal::Secret(why)), method));
        }
        let h = if type_first { v } else { vm.hidden(v) };
        if let Some(r) = refuses_type(vm, family, h) {
            return Err(fail(vm, f, n, Some(r), method));
        }
    }
    Ok(())
}

enum Refusal {
    Type,
    Range(f64, f64),
    FloatRange,
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
            let Some(mut n) = vm.tonumber(v) else {
                return Some(Refusal::Range(min, max));
            };
            if index {
                n -= 1.0;
            }
            (!(min..=max).contains(&n)).then_some(Refusal::Range(min, max))
        }
        Family::Number => vm.tonumber(v).is_none().then_some(Refusal::Type),
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

fn fail(vm: &mut Vm, f: &Function, n: usize, why: Option<Refusal>, method: bool) -> LuaError {
    let usage = usage(f);
    if let Some(Refusal::FloatRange) = why {
        return vm.api_arg_error(
            n,
            &format!("outside of expected range -3.402823e+38 to 3.402823e+38 - {usage}"),
        );
    }
    // A range error is Lua's own (`luaL_argerror`: the caller's position and
    // the function's name); the others are the bindings', with neither. A
    // method's is the bindings'.
    if let Some(Refusal::Range(min, max)) = why
        && method
    {
        return vm.api_arg_error(
            n,
            &format!("outside of expected range {min} to {max} - {usage}"),
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
        Some(Refusal::This) => format!(
            "Attempt to find 'this' in non-table object (used '.' instead of ':' ?) - {usage}"
        ),
        Some(Refusal::Secret(why)) => {
            format!("{usage}. Secret values are {why} for this argument.")
        }
    };
    vm.api_arg_error(n, &msg)
}
