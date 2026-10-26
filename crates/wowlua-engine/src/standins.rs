//! What Blizzard's UI code defines that addons and wowlua's own library
//! use as if the client had it (the client always loads that code), the
//! engine's own until the UI code loads (`test.blizzard-ui`), which then
//! defines its own over them:
//!
//! - `SecondsToTime`, `SecondsToTimeAbbrev`, `SecondsToClock` and
//!   `FormatShortDate` (Blizzard_SharedXML's `TimeUtil.lua`),
//!   `GetMoneyString` and `FormatLargeNumber` (`FormattingUtil.lua`), as
//!   those files write them, from the global strings they read (the
//!   client's enUS text where the strings are not loaded).
//! - `SendChatMessage`, `DoEmote` and `CancelEmote`
//!   (Blizzard_DeprecatedChatInfo): the `C_ChatInfo` functions they call,
//!   as they are when called.
//! - The coin constants (`COPPER_PER_SILVER`, `SILVER_PER_GOLD`,
//!   `COPPER_PER_GOLD`) and the global strings `wowlua:format` reads
//!   (`GOLD_AMOUNT`, `SILVER_AMOUNT`, `COPPER_AMOUNT`).
//! - The container constants Blizzard_FrameXMLBase's `Constants.lua` makes
//!   from `Constants.InventoryConstants` (`NUM_BAG_SLOTS`,
//!   `NUM_REAGENTBAG_SLOTS`, `NUM_TOTAL_EQUIPPED_BAG_SLOTS`, …), where the
//!   flavor's `Constants` table has the values (camelot's, recorded).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{LuaError, NativeCall, NativeFn, Value, Vm};

use crate::engine;

/// A global string, else the client's enUS text.
fn global_text(vm: &mut Vm, name: &str, fallback: &str) -> String {
    match vm.get_global(name) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => fallback.to_string(),
    }
}

/// `string.format(fmt, ...)` as the client's Lua runs it.
fn format(vm: &mut Vm, fmt: &str, args: &[Value]) -> Result<String, LuaError> {
    let main = engine(vm).main_globals;
    let k = vm.str("string");
    let f = match vm.heap.table(main).get(k) {
        Value::Table(t) => {
            let k = vm.str("format");
            vm.heap.table(t).get(k)
        }
        _ => Value::Nil,
    };
    let mut call = vec![vm.str(fmt)];
    call.extend_from_slice(args);
    let r = vm.call(f, &call)?;
    Ok(match r.first() {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(*s)).into_owned(),
        _ => String::new(),
    })
}

fn num(n: f64) -> Value {
    Value::Num(n)
}

/// Lua's `a % b`.
fn modulo(a: f64, b: f64) -> f64 {
    a - (a / b).floor() * b
}

const SECONDS_PER_MIN: f64 = 60.0;
const SECONDS_PER_HOUR: f64 = 60.0 * SECONDS_PER_MIN;
const SECONDS_PER_DAY: f64 = 24.0 * SECONDS_PER_HOUR;

/// `SecondsToTime(seconds, noSeconds, notAbbreviated, maxCount, roundUp)`:
/// up to `maxCount` units (2 by default) from days down, each by its
/// global string (`DAYS_ABBR`, or `D_DAYS` with `notAbbreviated`), joined
/// by `TIME_UNIT_DELIMITER`; a single unit from 1.5 of it; the seconds
/// left out with `noSeconds`; the last unit rounded up with `roundUp`.
fn seconds_to_time(vm: &mut Vm, c: NativeCall) -> Ret {
    let seconds = vm.check_number(&c, 1)?;
    let no_seconds = vm.hidden(c.arg(2)).truthy();
    let long = vm.hidden(c.arg(3)).truthy();
    let max_count = vm.opt_number(&c, 4, 2.0)?;
    let round_up = vm.hidden(c.arg(5)).truthy();
    let mut seconds = if round_up {
        seconds.ceil()
    } else {
        seconds.floor()
    };
    let threshold = if max_count > 1.0 { 1.0 } else { 1.5 };
    let delimiter = global_text(vm, "TIME_UNIT_DELIMITER", " ");
    let mut time = String::new();
    let mut count = 0.0;
    // Each unit's seconds, its words' and its abbreviation's global
    // strings (with their enUS text).
    type Unit = (
        f64,
        (&'static str, &'static str),
        (&'static str, &'static str),
    );
    let units: [Unit; 3] = [
        (
            SECONDS_PER_DAY,
            ("D_DAYS", "%d |4Day:Days;"),
            ("DAYS_ABBR", "%d |4Day:Days;"),
        ),
        (
            SECONDS_PER_HOUR,
            ("D_HOURS", "%d |4Hour:Hours;"),
            ("HOURS_ABBR", "%d |4Hr:Hr;"),
        ),
        (
            SECONDS_PER_MIN,
            ("D_MINUTES", "%d |4Minute:Minutes;"),
            ("MINUTES_ABBR", "%d |4Min:Min;"),
        ),
    ];
    for (size, words, abbr) in units {
        if count < max_count && seconds >= size * threshold {
            count += 1.0;
            if !time.is_empty() {
                time.push_str(&delimiter);
            }
            let n = if count == max_count && round_up {
                (seconds / size).ceil()
            } else {
                (seconds / size).floor()
            };
            let (name, fallback) = if long { words } else { abbr };
            let f = global_text(vm, name, fallback);
            time.push_str(&format(vm, &f, &[num(n)])?);
            seconds = modulo(seconds, size);
        }
    }
    if count < max_count && seconds > 0.0 && !no_seconds {
        if !time.is_empty() {
            time.push_str(&delimiter);
        }
        let (name, fallback) = if long {
            ("D_SECONDS", "%d |4Second:Seconds;")
        } else {
            ("SECONDS_ABBR", "%d |4Sec:Sec;")
        };
        let f = global_text(vm, name, fallback);
        time.push_str(&format(vm, &f, &[num(seconds)])?);
    }
    Ok(vec![vm.str(&time)])
}

/// `SecondsToTimeAbbrev(seconds, thresholdOverride)`: the largest unit
/// from 1.5 of it (or the override) as its one-letter format string
/// (`DAY_ONELETTER_ABBR`, "%d d") and the count, rounded up; else the
/// seconds' format and the seconds as given.
fn seconds_to_time_abbrev(vm: &mut Vm, c: NativeCall) -> Ret {
    let seconds = vm.check_number(&c, 1)?;
    let threshold = vm.opt_number(&c, 2, 1.5)?;
    let units = [
        (SECONDS_PER_DAY, "DAY_ONELETTER_ABBR", "%d d"),
        (SECONDS_PER_HOUR, "HOUR_ONELETTER_ABBR", "%d h"),
        (SECONDS_PER_MIN, "MINUTE_ONELETTER_ABBR", "%d m"),
    ];
    for (size, name, fallback) in units {
        if seconds >= size * threshold {
            let f = global_text(vm, name, fallback);
            return Ok(vec![vm.str(&f), num((seconds / size).ceil())]);
        }
    }
    let f = global_text(vm, "SECOND_ONELETTER_ABBR", "%d s");
    Ok(vec![vm.str(&f), num(seconds)])
}

/// `SecondsToClock(seconds, displayZeroHours)`: `HOURS_MINUTES_SECONDS`
/// ("01:03:07") when there are hours (those beyond whole days) or
/// `displayZeroHours`, else `MINUTES_SECONDS` ("03:07"); below zero as
/// zero.
fn seconds_to_clock(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut t = vm.check_number(&c, 1)?.max(0.0);
    let zero_hours = vm.hidden(c.arg(2)).truthy();
    let days = (t / SECONDS_PER_DAY).floor();
    t -= days * SECONDS_PER_DAY;
    let hours = (t / SECONDS_PER_HOUR).floor();
    t -= hours * SECONDS_PER_HOUR;
    let minutes = (t / SECONDS_PER_MIN).floor();
    t -= minutes * SECONDS_PER_MIN;
    let seconds = t.floor();
    let text = if hours > 0.0 || zero_hours {
        let f = global_text(vm, "HOURS_MINUTES_SECONDS", "%.2d:%.2d:%.2d");
        format(vm, &f, &[num(hours), num(minutes), num(seconds)])?
    } else {
        let f = global_text(vm, "MINUTES_SECONDS", "%.2d:%.2d");
        format(vm, &f, &[num(minutes), num(seconds)])?
    };
    Ok(vec![vm.str(&text)])
}

/// `FormatShortDate(day, month, year)`: `SHORTDATE` ("10/26/2026"), or
/// `SHORTDATENOYEAR` without a year; the `_EU` forms where `LOCALE_enGB`
/// is set.
fn format_short_date(vm: &mut Vm, c: NativeCall) -> Ret {
    let gb = vm.get_global("LOCALE_enGB").truthy();
    let year = c.arg(3);
    let (name, fallback) = match (vm.hidden(year).is_nil(), gb) {
        (false, true) => ("SHORTDATE_EU", "%1$d/%2$d/%3$02d"),
        (false, false) => ("SHORTDATE", "%2$d/%1$02d/%3$02d"),
        (true, true) => ("SHORTDATENOYEAR_EU", "%1$d/%2$d"),
        (true, false) => ("SHORTDATENOYEAR", "%2$d/%1$02d"),
    };
    let f = global_text(vm, name, fallback);
    let mut args = vec![c.arg(1), c.arg(2)];
    if !vm.hidden(year).is_nil() {
        args.push(year);
    }
    let text = format(vm, &f, &args)?;
    Ok(vec![vm.str(&text)])
}

/// A number's digits grouped by `LARGE_NUMBER_SEPERATOR` (`FormatLargeNumber`).
fn large_number(vm: &mut Vm, amount: Value) -> Result<String, LuaError> {
    let s = match vm.tostring(amount)? {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    };
    let sep = global_text(vm, "LARGE_NUMBER_SEPERATOR", ",");
    let bytes = s.as_bytes();
    let len = bytes.len();
    // Blizzard's loop: each group of three from the right, behind a
    // separator, then what is before the first.
    let mut out = String::new();
    let mut i = 4;
    while i <= len {
        // (`amount:sub(-(i - 1), -(i - 3))`)
        let group = &s[len + 1 - i..len + 4 - i];
        out = format!("{sep}{group}{out}");
        i += 3;
    }
    let head = if len % 3 == 0 { 3 } else { len % 3 };
    Ok(format!("{}{out}", &s[..head.min(len)]))
}

/// `FormatLargeNumber(amount)`: "1,234,567".
fn format_large_number(vm: &mut Vm, c: NativeCall) -> Ret {
    let text = large_number(vm, c.arg(1))?;
    Ok(vec![vm.str(&text)])
}

/// `GetMoneyString(money, separateThousands, checkGoldThreshold,
/// showZeroAsGold, color)`: gold, silver and copper with their coin icons
/// (`GOLD_AMOUNT_TEXTURE`; the symbols `GOLD_AMOUNT_SYMBOL` in colorblind
/// mode), the units at zero left out (copper stays when all are), the
/// gold's thousands separated with `separateThousands`, the copper left
/// out above `Constants.MoneyFormattingConstants`' gold threshold with
/// `checkGoldThreshold`, zero as gold with `showZeroAsGold`, wrapped in
/// `color`'s code when given.
fn get_money_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let money = vm.check_number(&c, 1)?;
    let separate = vm.hidden(c.arg(2)).truthy();
    let check_threshold = vm.hidden(c.arg(3)).truthy();
    let zero_as_gold = vm.hidden(c.arg(4)).truthy();
    let color = c.arg(5);
    let gold = (money / 10000.0).floor();
    let silver = ((money - gold * 10000.0) / 100.0).floor();
    let mut copper = money % 100.0;
    let colorblind = crate::framescript::cvar_get(vm, "colorblindMode").as_deref() == Some("1")
        || matches!(vm.get_global("ENABLE_COLORBLIND_MODE"), Value::Str(s) if vm.str_bytes(s) == b"1");
    let (gold_text, silver_text, copper_text) = if colorblind {
        let gold_n = if separate {
            large_number(vm, num(gold))?
        } else {
            format(vm, "%s", &[num(gold)])?
        };
        let g = global_text(vm, "GOLD_AMOUNT_SYMBOL", "g");
        let s = global_text(vm, "SILVER_AMOUNT_SYMBOL", "s");
        let cp = global_text(vm, "COPPER_AMOUNT_SYMBOL", "c");
        let silver_n = format(vm, "%s", &[num(silver)])?;
        let copper_n = format(vm, "%s", &[num(copper)])?;
        (
            format!("{gold_n}{g}"),
            format!("{silver_n}{s}"),
            format!("{copper_n}{cp}"),
        )
    } else {
        let gold_text = if separate {
            let f = global_text(
                vm,
                "GOLD_AMOUNT_TEXTURE_STRING",
                "%s|TInterface\\MoneyFrame\\UI-GoldIcon:%d:%d:2:0|t",
            );
            let grouped = large_number(vm, num(gold))?;
            let grouped = vm.str(&grouped);
            format(vm, &f, &[grouped, num(0.0), num(0.0)])?
        } else {
            let f = global_text(
                vm,
                "GOLD_AMOUNT_TEXTURE",
                "%d|TInterface\\MoneyFrame\\UI-GoldIcon:%d:%d:2:0|t",
            );
            format(vm, &f, &[num(gold), num(0.0), num(0.0)])?
        };
        let f = global_text(
            vm,
            "SILVER_AMOUNT_TEXTURE",
            "%d|TInterface\\MoneyFrame\\UI-SilverIcon:%d:%d:2:0|t",
        );
        let silver_text = format(vm, &f, &[num(silver), num(0.0), num(0.0)])?;
        let f = global_text(
            vm,
            "COPPER_AMOUNT_TEXTURE",
            "%d|TInterface\\MoneyFrame\\UI-CopperIcon:%d:%d:2:0|t",
        );
        let copper_text = format(vm, &f, &[num(copper), num(0.0), num(0.0)])?;
        (gold_text, silver_text, copper_text)
    };
    let text = if zero_as_gold && money == 0.0 {
        gold_text
    } else {
        if check_threshold
            && let Some(limit) = gold_threshold(vm)
            && gold > limit
        {
            copper = 0.0;
        }
        let mut parts = Vec::new();
        if gold > 0.0 {
            parts.push(gold_text);
        }
        if silver > 0.0 {
            parts.push(silver_text);
        }
        if copper > 0.0 || parts.is_empty() {
            parts.push(copper_text);
        }
        parts.join(" ")
    };
    let v = vm.str(&text);
    if let Value::Table(t) = color {
        let k = vm.str("WrapTextInColorCode");
        let wrap = vm.heap.table(t).get(k);
        if !wrap.is_nil() {
            return Ok(vm.call(wrap, &[color, v])?.into_iter().take(1).collect());
        }
    }
    Ok(vec![v])
}

/// `Constants.MoneyFormattingConstants.GOLD_REWARD_THRESHOLD_TO_HIDE_COPPER`.
fn gold_threshold(vm: &mut Vm) -> Option<f64> {
    let Value::Table(constants) = vm.get_global("Constants") else {
        return None;
    };
    let k = vm.str("MoneyFormattingConstants");
    let Value::Table(t) = vm.heap.table(constants).get(k) else {
        return None;
    };
    let k = vm.str("GOLD_REWARD_THRESHOLD_TO_HIDE_COPPER");
    match vm.heap.table(t).get(k) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// `C_ChatInfo.<name>` as it is now (a test's stub or spy included).
fn chat_info(vm: &mut Vm, name: &str) -> Value {
    let Value::Table(t) = vm.get_global("C_ChatInfo") else {
        return Value::Nil;
    };
    let k = vm.str(name);
    vm.heap.table(t).get(k)
}

fn call_chat_info(vm: &mut Vm, name: &str, args: &[Value]) -> Result<Vec<Value>, LuaError> {
    let f = chat_info(vm, name);
    if f.is_nil() {
        return Err(vm.error(&format!("C_ChatInfo.{name} is not defined")));
    }
    vm.call(f, args)
}

/// `SendChatMessage(message, chatType, languageID, target)`:
/// `C_ChatInfo.SendChatMessage` with them, nothing returned.
fn send_chat_message(vm: &mut Vm, c: NativeCall) -> Ret {
    let args = [c.arg(1), c.arg(2), c.arg(3), c.arg(4)];
    call_chat_info(vm, "SendChatMessage", &args)?;
    Ok(vec![])
}

/// `DoEmote(emoteName, targetName, suppressMoveError)`:
/// `C_ChatInfo.PerformEmote`'s answer; without a name
/// `C_ChatInfo.CancelEmote()` and false.
fn do_emote(vm: &mut Vm, c: NativeCall) -> Ret {
    if vm.hidden(c.arg(1)).is_nil() {
        call_chat_info(vm, "CancelEmote", &[])?;
        return Ok(vec![Value::Bool(false)]);
    }
    let args = [c.arg(1), c.arg(2), c.arg(3)];
    Ok(call_chat_info(vm, "PerformEmote", &args)?
        .into_iter()
        .take(1)
        .collect())
}

/// `CancelEmote()`: `C_ChatInfo.CancelEmote()`.
fn cancel_emote(vm: &mut Vm, _c: NativeCall) -> Ret {
    call_chat_info(vm, "CancelEmote", &[])?;
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("SecondsToTime", seconds_to_time),
        ("SecondsToTimeAbbrev", seconds_to_time_abbrev),
        ("SecondsToClock", seconds_to_clock),
        ("FormatShortDate", format_short_date),
        ("FormatLargeNumber", format_large_number),
        ("GetMoneyString", get_money_string),
        ("SendChatMessage", send_chat_message),
        ("DoEmote", do_emote),
        ("CancelEmote", cancel_emote),
    ]
}

/// The container constants of Blizzard's `Constants.lua`: the global and
/// its `Constants.InventoryConstants` field.
const CONTAINER_CONSTANTS: &[(&str, &str)] = &[
    ("NUM_BAG_SLOTS", "NumBagSlots"),
    ("NUM_REAGENTBAG_SLOTS", "NumReagentBagSlots"),
    ("NUM_CHARACTER_BANK_SLOTS", "NumCharacterBankSlots"),
    ("NUM_BANKGENERIC_SLOTS", "NumGenericBankSlots"),
    ("NUM_BANKBAGSLOTS", "NumBankBagSlots"),
];

fn set_if_nil(vm: &mut Vm, name: &str, v: Value) {
    if vm.get_global(name).is_nil() {
        vm.set_global(name, v);
    }
}

/// The constants and global strings above, where nothing defines them yet.
pub fn install(vm: &mut Vm) {
    set_if_nil(vm, "COPPER_PER_SILVER", num(100.0));
    set_if_nil(vm, "SILVER_PER_GOLD", num(100.0));
    set_if_nil(vm, "COPPER_PER_GOLD", num(10000.0));
    for (name, text) in [
        ("GOLD_AMOUNT", "%d Gold"),
        ("SILVER_AMOUNT", "%d Silver"),
        ("COPPER_AMOUNT", "%d Copper"),
    ] {
        let v = vm.str(text);
        set_if_nil(vm, name, v);
    }
    let inventory = match vm.get_global("Constants") {
        Value::Table(t) => {
            let k = vm.str("InventoryConstants");
            match vm.heap.table(t).get(k) {
                Value::Table(i) => Some(i),
                _ => None,
            }
        }
        _ => None,
    };
    let Some(inventory) = inventory else {
        return;
    };
    let field = |vm: &mut Vm, name: &str| {
        let k = vm.str(name);
        match vm.heap.table(inventory).get(k) {
            Value::Num(n) => Some(n),
            _ => None,
        }
    };
    let mut bags = None;
    let mut reagent = None;
    for (global, name) in CONTAINER_CONSTANTS {
        if let Some(n) = field(vm, name) {
            set_if_nil(vm, global, num(n));
            match *name {
                "NumBagSlots" => bags = Some(n),
                "NumReagentBagSlots" => reagent = Some(n),
                _ => {}
            }
        }
    }
    // `NUM_TOTAL_EQUIPPED_BAG_SLOTS = NUM_BAG_SLOTS + NUM_REAGENTBAG_SLOTS`
    // (the flavors with a reagent bag).
    if let (Some(b), Some(r)) = (bags, reagent) {
        set_if_nil(vm, "NUM_TOTAL_EQUIPPED_BAG_SLOTS", num(b + r));
    }
}
