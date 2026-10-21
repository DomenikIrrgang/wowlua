//! What the client says about itself and the machine rather than the
//! world: sound, the cursor, the mouse and keys held, the screen, the
//! frame rate and clocks, keybinding helpers, the client's build and
//! expansion, its math helpers (C functions on camelot), the add-on list,
//! currencies and reputations from the player's recorded stats, colors and
//! encodings, and the systems Forever lacks (game modes, renown, game
//! pads, text to speech), which answer their "nothing here".
//!
//! Settings these functions change and read back (add-ons disabled, the
//! reputation sort, mouselook, binding contexts, …) live in a table in the
//! VM's registry, one per engine.
//!
//! Unverified unless a comment says recorded: most answers follow the
//! documentation's shapes and the systems' empty states; nothing here was
//! probed on Forever.

use std::io::{Read, Write};

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::stats::{Answer, Entry};
use crate::units;
use crate::widgets;
use crate::{engine, engine_mut};

// ------------------------------------------------------------- helpers

/// This module's settings table, in the registry.
fn state(vm: &mut Vm) -> TableRef {
    let registry = vm.registry;
    let key = vm.heap.intern(b"wowlua.engine.systeminfo");
    if let Value::Table(t) = vm.heap.table(registry).get_str(key) {
        return t;
    }
    let t = vm.new_table();
    vm.heap.table_mut(registry).set_str(key, Value::Table(t));
    t
}

fn get_state(vm: &mut Vm, key: &str) -> Value {
    let t = state(vm);
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set_state(vm: &mut Vm, key: &str, v: Value) {
    let t = state(vm);
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn num(vm: &Vm, v: Value) -> Option<f64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n),
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).trim().parse().ok(),
        _ => None,
    }
}

fn text(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        Value::Num(n) => Some(wowlua_vm::number::fmt_num(n)),
        _ => None,
    }
}

fn set_field(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

fn get_field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn yes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(true)])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn nil(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn empty_string(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

fn empty_table(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn four_zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 4])
}

/// A recorded stat's answer as a value (a fresh table each time).
fn answer_value(vm: &mut Vm, a: &Answer) -> Value {
    match a {
        Answer::Nil => Value::Nil,
        Answer::Bool(b) => Value::Bool(*b),
        Answer::Num(n) => Value::Num(*n),
        Answer::Str(s) => vm.str(s),
        Answer::Table(entries) => {
            let t = vm.new_table();
            for (k, v) in entries {
                let k = answer_value(vm, k);
                let v = answer_value(vm, v);
                let _ = vm.heap.table_mut(t).set(k, v);
            }
            Value::Table(t)
        }
    }
}

type Fields = Vec<(Answer, Answer)>;

fn field<'a>(fields: &'a [(Answer, Answer)], key: &str) -> Option<&'a Answer> {
    fields.iter().find_map(|(k, v)| match k {
        Answer::Str(s) if s.as_str() == key => Some(v),
        _ => None,
    })
}

fn field_num(fields: &[(Answer, Answer)], key: &str) -> Option<f64> {
    match field(fields, key) {
        Some(Answer::Num(n)) => Some(*n),
        _ => None,
    }
}

fn field_bool(fields: &[(Answer, Answer)], key: &str) -> bool {
    matches!(field(fields, key), Some(Answer::Bool(true)))
}

fn field_str(fields: &[(Answer, Answer)], key: &str) -> Option<String> {
    match field(fields, key) {
        Some(Answer::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

/// An indexed stat's structures (`faction`, `currencyListInfo`), by index.
fn indexed(vm: &Vm, key: &str) -> Vec<(i64, Fields)> {
    match engine(vm).world.character.stats.get(key) {
        Some(Entry::ByIndex(m)) => m
            .iter()
            .filter_map(|(i, v)| match v.first() {
                Some(Answer::Table(f)) => Some((*i, f.clone())),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn one_num(vm: &Vm, key: &str) -> Option<f64> {
    match engine(vm).world.character.stats.get(key) {
        Some(Entry::One(v)) => match v.first() {
            Some(Answer::Num(n)) => Some(*n),
            _ => None,
        },
        _ => None,
    }
}

// --------------------------------------------------------------- math

/// `Round(value)`: halves away from zero (Blizzard's `MathUtil.lua` in the
/// other flavors; a C function on camelot, unverified the same).
fn round_value(v: f64) -> f64 {
    if v < 0.0 {
        (v - 0.5).ceil()
    } else {
        (v + 0.5).floor()
    }
}

fn round(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = vm.check_number(&c, 1)?;
    Ok(vec![Value::Num(round_value(v))])
}

/// `Clamp(value, min, max)`: max above it, min below (MathUtil.lua).
fn clamp(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = vm.check_number(&c, 1)?;
    let min = vm.check_number(&c, 2)?;
    let max = vm.check_number(&c, 3)?;
    let out = if v > max {
        max
    } else if v < min {
        min
    } else {
        v
    };
    Ok(vec![Value::Num(out)])
}

/// `Lerp(start, end, amount)`: `(1 - amount) * start + amount * end`
/// (MathUtil.lua).
fn lerp(vm: &mut Vm, c: NativeCall) -> Ret {
    let a = vm.check_number(&c, 1)?;
    let b = vm.check_number(&c, 2)?;
    let t = vm.check_number(&c, 3)?;
    Ok(vec![Value::Num((1.0 - t) * a + t * b)])
}

/// `Saturate(value)`: clamped to 0..1 (MathUtil.lua).
fn saturate(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = vm.check_number(&c, 1)?;
    Ok(vec![Value::Num(v.clamp(0.0, 1.0))])
}

/// `RoundToSignificantDigits(value, digits)`: `Round(value * 10^digits) /
/// 10^digits` (MathUtil.lua).
fn round_to_significant_digits(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = vm.check_number(&c, 1)?;
    let digits = vm.check_number(&c, 2)?;
    let m = 10f64.powf(digits);
    Ok(vec![Value::Num(round_value(v * m) / m)])
}

// -------------------------------------------------------------- sound

/// A new sound handle (the counter `C_Sound.PlaySound` uses, facts.rs).
fn sound_handle(vm: &mut Vm) -> f64 {
    let w = &mut engine_mut(vm).world;
    w.sound_handles += 1.0;
    w.sound_handles
}

/// `PlaySound(soundKitID [, channel, forceNoDuplicates,
/// runFinishCallback])`, `PlaySoundFile(file [, channel])`: played, a
/// handle; nothing to play (no kit or file): not played (the engine plays
/// no sound; unverified: the handles' numbers, a bad kit's answer).
fn play_sound(vm: &mut Vm, c: NativeCall) -> Ret {
    if !matches!(vm.hidden(c.arg(1)), Value::Num(_) | Value::Str(_)) {
        return Ok(vec![Value::Bool(false)]);
    }
    let h = sound_handle(vm);
    Ok(vec![Value::Bool(true), Value::Num(h)])
}

/// `C_Sound.PlaySoundWithOptions(params)`: played, a handle (as
/// `PlaySound`).
fn play_sound_with_options(vm: &mut Vm, _c: NativeCall) -> Ret {
    let h = sound_handle(vm);
    Ok(vec![Value::Bool(true), Value::Num(h)])
}

/// `GetCombatRatingBonusForCombatRatingValue(rating, value)`: rating 1's
/// bonus the value's whole part, as a single-precision float; the others none
/// (the full probe: 1 for 1, -1 for -1, 2147483648 for 2147483647, rating
/// 0 always 0; the other ratings unverified).
fn combat_rating_bonus_for_value(vm: &mut Vm, c: NativeCall) -> Ret {
    let (rating, value) = (num(vm, c.arg(1)), num(vm, c.arg(2)));
    let bonus = match (rating.map(f64::trunc), value) {
        (Some(1.0), Some(v)) => f64::from(v.trunc() as f32),
        _ => 0.0,
    };
    Ok(vec![Value::Num(bonus)])
}

// -------------------------------------------------------- mouse, keys

/// `IsKeyDown(key)`: a modifier the engine holds (Shift, Ctrl, Alt, the
/// left ones or either); any other key or button up between the engine's
/// actions (unverified: the right keys, `excludeBindingState`).
fn is_key_down(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = text(vm, c.arg(1)).unwrap_or_default().to_uppercase();
    // No key named: nil (the full probe: `IsKeyDown("")`).
    if key.is_empty() && matches!(c.arg(1), Value::Str(_)) {
        return Ok(vec![Value::Nil]);
    }
    let i = match key.as_str() {
        "SHIFT" | "LSHIFT" => Some(0),
        "CTRL" | "LCTRL" => Some(1),
        "ALT" | "LALT" => Some(2),
        _ => None,
    };
    let down = i.is_some_and(|i| engine(vm).world.input.modifiers[i]);
    Ok(vec![Value::Bool(down)])
}

/// `IsMouselooking()`: since `MouselookStart`, until `MouselookStop`
/// (unverified: the client also starts it with a held right button).
fn is_mouselooking(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(get_state(vm, "mouselook").truthy())])
}

fn mouselook_start(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_state(vm, "mouselook", Value::Bool(true));
    Ok(vec![])
}

fn mouselook_stop(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_state(vm, "mouselook", Value::Nil);
    Ok(vec![])
}

// ------------------------------------------------------------- cursor

/// `C_Cursor.GetCursorItem()`: the held item's location (Blizzard's
/// `ItemLocation` when its code is loaded), nothing when the cursor holds
/// no item from a bag or gear slot.
fn get_cursor_item(vm: &mut Vm, _c: NativeCall) -> Ret {
    let Some((_, source)) = crate::cursor::held_item(vm) else {
        return Ok(vec![]);
    };
    let (ctor, args, fields) = match source {
        crate::cursor::Source::Bag(bag, slot) => (
            "CreateFromBagAndSlot",
            vec![Value::Num(bag as f64), Value::Num(slot as f64)],
            vec![("bagID", bag as f64), ("slotIndex", slot as f64)],
        ),
        crate::cursor::Source::Equipped(slot) => (
            "CreateFromEquipmentSlot",
            vec![Value::Num(slot as f64)],
            vec![("equipmentSlotIndex", slot as f64)],
        ),
    };
    let class = vm.get_global("ItemLocation");
    if let Value::Table(t) = class {
        let f = get_field(vm, t, ctor);
        if matches!(f, Value::Func(_)) {
            let mut call = vec![class];
            call.extend(args);
            let r = vm.call(f, &call)?;
            return Ok(r.into_iter().take(1).collect());
        }
    }
    let t = vm.new_table();
    for (k, v) in fields {
        set_field(vm, t, k, Value::Num(v));
    }
    Ok(vec![Value::Table(t)])
}

// ------------------------------------------------------------- screen

/// `GetScreenWidth()`: UIParent's width on the screen (decision T237;
/// recorded: 1920.0001220703 at 1080 high on the Mac, 1365.3333740234 on
/// Windows at four sizes).
fn get_screen_width(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        crate::engine(vm).screen.ui_parent_size().0,
    )])
}

/// `GetScreenDPIScale()`: the screen's (decision T237; recorded: the Mac's
/// 1, Windows' 1.5 to 3 by the window's size on a 4K screen), a single
/// float.
fn get_screen_dpi_scale(vm: &mut Vm, _c: NativeCall) -> Ret {
    let dpi = crate::engine(vm).screen.dpi_scale as f32;
    Ok(vec![Value::Num(f64::from(dpi))])
}

/// `C_UI.GetUIParent()`, `C_UI.GetWorldFrame()`: the frames.
fn get_ui_parent(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.get_global("UIParent")])
}

fn get_world_frame(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.get_global("WorldFrame")])
}

// ------------------------------------------------- frame rate, clocks

/// `GetFramerate()`: frames a second by the last frame's length, 60
/// before any frame (recorded: 166.4 on the probing machine; unverified:
/// the client's averaging).
fn get_framerate(vm: &mut Vm, _c: NativeCall) -> Ret {
    let tick = engine(vm).timers.tick_time;
    let fps = if tick > 0.0 { 1.0 / tick } else { 60.0 };
    Ok(vec![Value::Num(fps)])
}

/// `GetServerTime()`: the clock's epoch seconds (as `time()`).
fn get_server_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(vm.clock.now.floor())])
}

/// `C_DateAndTime.GetServerTimeLocal()`: the epoch seconds in the clock's
/// local zone (unverified).
fn get_server_time_local(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = vm.clock.now.floor() + vm.clock.utc_offset as f64;
    Ok(vec![Value::Num(t)])
}

/// `GetSessionTime()`: whole seconds since the session began (`GetTime`'s
/// start; unverified).
fn get_session_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(crate::clock::now(vm).floor())])
}

/// The daily reset at 16:00 UTC, the weekly one on Tuesdays then (the full
/// probe, 1.60.1: `GetWeeklyResetStartTime()` 1791907200, Tuesday 13
/// October 2026 16:00 UTC; `GetSecondsUntilDailyReset()` 79296 in the
/// session whose `GetCurrentCalendarTime()` was 10:57 on the Pacific
/// realm, 17:57 UTC, both the next 16:00).
const DAILY_RESET: i64 = 16 * 3600;
/// 1970-01-06 (a Tuesday) 16:00 UTC.
const WEEKLY_RESET: i64 = 5 * 86400 + 16 * 3600;

fn now_secs(vm: &Vm) -> i64 {
    vm.clock.now.floor() as i64
}

fn seconds_until_daily_reset(vm: &mut Vm, _c: NativeCall) -> Ret {
    let since = (now_secs(vm) - DAILY_RESET).rem_euclid(86400);
    Ok(vec![Value::Num((86400 - since) as f64)])
}

fn seconds_until_weekly_reset(vm: &mut Vm, _c: NativeCall) -> Ret {
    let since = (now_secs(vm) - WEEKLY_RESET).rem_euclid(7 * 86400);
    Ok(vec![Value::Num((7 * 86400 - since) as f64)])
}

/// `C_DateAndTime.GetWeeklyResetStartTime()`: when this week's began.
fn weekly_reset_start_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = now_secs(vm);
    let since = (t - WEEKLY_RESET).rem_euclid(7 * 86400);
    Ok(vec![Value::Num((t - since) as f64)])
}

/// Days since 1970-01-01 of a Gregorian date (Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The Gregorian date of a day since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A `CalendarTime` as minutes since 1970-01-01 (its zone's).
fn calendar_minutes(vm: &mut Vm, v: Value) -> Option<i64> {
    let Value::Table(t) = v else { return None };
    let mut get = |k: &str| {
        let v = get_field(vm, t, k);
        num(vm, v).map(|n| n as i64)
    };
    let (y, m, d) = (get("year")?, get("month")?, get("monthDay")?);
    let (h, min) = (get("hour").unwrap_or(0), get("minute").unwrap_or(0));
    Some(days_from_civil(y, m, d) * 1440 + h * 60 + min)
}

/// A `CalendarTime` (`year, month, monthDay, weekday` with Sunday 1,
/// `hour, minute`) of minutes since 1970-01-01.
fn calendar_time(vm: &mut Vm, minutes: i64) -> Value {
    let days = minutes.div_euclid(1440);
    let rest = minutes.rem_euclid(1440);
    let (y, m, d) = civil_from_days(days);
    let t = vm.new_table();
    for (k, v) in [
        ("year", y),
        ("month", m),
        ("monthDay", d),
        ("weekday", (days + 4).rem_euclid(7) + 1),
        ("hour", rest / 60),
        ("minute", rest % 60),
    ] {
        set_field(vm, t, k, Value::Num(v as f64));
    }
    Value::Table(t)
}

/// `C_DateAndTime.AdjustTimeByDays(date, days)`, `…ByMinutes`: the date
/// moved (unverified: a date without its time).
fn adjust_time_by_days(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(m) = calendar_minutes(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let days = num(vm, c.arg(2)).unwrap_or(0.0) as i64;
    Ok(vec![calendar_time(vm, m + days * 1440)])
}

fn adjust_time_by_minutes(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(m) = calendar_minutes(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let minutes = num(vm, c.arg(2)).unwrap_or(0.0) as i64;
    Ok(vec![calendar_time(vm, m + minutes)])
}

/// `C_DateAndTime.AdjustTimeByMonths(date, months)`: the same day of the
/// month that many on, the month's last if it has fewer (unverified).
fn adjust_time_by_months(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(m) = calendar_minutes(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let months = num(vm, c.arg(2)).unwrap_or(0.0) as i64;
    let (days, rest) = (m.div_euclid(1440), m.rem_euclid(1440));
    let (y, mo, d) = civil_from_days(days);
    let index = y * 12 + (mo - 1) + months;
    let (y, mo) = (index.div_euclid(12), index.rem_euclid(12) + 1);
    let d = d.min(days_in_month(y, mo));
    Ok(vec![calendar_time(
        vm,
        days_from_civil(y, mo, d) * 1440 + rest,
    )])
}

/// `C_DateAndTime.CompareCalendarTime(lhs, rhs)`: 1 when rhs is later, -1
/// when earlier, 0 the same minute (warcraft.wiki.gg; unverified).
fn compare_calendar_time(vm: &mut Vm, c: NativeCall) -> Ret {
    let a = calendar_minutes(vm, c.arg(1)).unwrap_or(0);
    let b = calendar_minutes(vm, c.arg(2)).unwrap_or(0);
    let r = match b.cmp(&a) {
        std::cmp::Ordering::Greater => 1.0,
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
    };
    Ok(vec![Value::Num(r)])
}

/// `C_DateAndTime.GetCalendarTimeFromEpoch(epoch)`: the date of an epoch
/// in microseconds (as a club message's `epoch`), moved by the realm's
/// offset the other way round, as if the epoch were the realm's time (the
/// full probe, 1.60.1, a Pacific realm: `GetServerTimeLocal()` 7 hours
/// before `GetServerTime()`, epoch 0 at 08:00 on 1 January 1970).
fn calendar_time_from_epoch(vm: &mut Vm, c: NativeCall) -> Ret {
    let epoch = num(vm, c.arg(1)).unwrap_or(0.0);
    let secs = (epoch / 1_000_000.0).floor() as i64 - vm.clock.utc_offset;
    Ok(vec![calendar_time(vm, secs.div_euclid(60))])
}

// ------------------------------------------------------ build, account

/// `GetClientDisplayExpansionLevel()`, `GetAccountExpansionLevel()`,
/// `GetExpansionForLevel(level)`: classic's, 0 (as `GetExpansionLevel`,
/// recorded 0 at load; unverified each).
fn expansion_zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

// ------------------------------------------------------------ add-ons

/// The add-on an argument names: a test-defined one's name (by name or
/// index), one of Blizzard's UI code loaded, or one whose `ADDON_LOADED`
/// came.
fn addon(vm: &mut Vm, v: Value) -> Option<String> {
    let addons = &engine(vm).world.bindings.addons;
    match vm.hidden(v) {
        Value::Num(n) => {
            let i = n as usize;
            (i >= 1 && i <= addons.len()).then(|| addons[i - 1].name.clone())
        }
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            let w = &engine(vm).world;
            if let Some(a) = w
                .bindings
                .addons
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(&name))
            {
                return Some(a.name.clone());
            }
            if let Some(l) = w.ui.loaded.iter().find(|l| l.eq_ignore_ascii_case(&name)) {
                return Some(l.clone());
            }
            w.loaded_addons
                .contains(&name.to_lowercase())
                .then_some(name)
        }
        _ => None,
    }
}

/// A test-defined add-on's TOC field, any case.
fn metadata(vm: &Vm, name: &str, key: &str) -> Option<String> {
    engine(vm)
        .world
        .bindings
        .addons
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))?
        .metadata
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.clone())
}

fn disabled_key(name: &str) -> String {
    format!("disabled:{}", name.to_lowercase())
}

fn is_disabled(vm: &mut Vm, name: &str) -> bool {
    get_state(vm, &disabled_key(name)).truthy()
}

/// `C_AddOns.DoesAddOnExist(name)`.
fn does_addon_exist(vm: &mut Vm, c: NativeCall) -> Ret {
    let exists = addon(vm, c.arg(1)).is_some();
    Ok(vec![Value::Bool(exists)])
}

/// `C_AddOns.GetAddOnEnableState(name [, character])`: all (2) for an
/// add-on that exists, none (0) disabled or unknown
/// (`Enum.AddOnEnableState`; unverified: per character).
fn get_addon_enable_state(vm: &mut Vm, c: NativeCall) -> Ret {
    let state = match addon(vm, c.arg(1)) {
        Some(name) if !is_disabled(vm, &name) => 2.0,
        _ => 0.0,
    };
    Ok(vec![Value::Num(state)])
}

/// `C_AddOns.EnableAddOn(name [, character])`, `DisableAddOn`: its state
/// for the next load (kept; the add-on loaded stays).
fn enable_addon(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(name) = addon_arg_name(vm, c.arg(1)) {
        set_state(vm, &disabled_key(&name), Value::Nil);
    }
    Ok(vec![])
}

/// An add-on argument's name: an index's add-on, else the name given.
fn addon_arg_name(vm: &mut Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Num(_) => addon(vm, v),
        _ => text(vm, v),
    }
}

fn disable_addon(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(name) = addon_arg_name(vm, c.arg(1)) {
        set_state(vm, &disabled_key(&name), Value::Bool(true));
    }
    Ok(vec![])
}

fn defined_names(vm: &Vm) -> Vec<String> {
    engine(vm)
        .world
        .bindings
        .addons
        .iter()
        .map(|a| a.name.clone())
        .collect()
}

/// `C_AddOns.EnableAllAddOns`, `DisableAllAddOns`: each add-on's state.
fn enable_all_addons(vm: &mut Vm, _c: NativeCall) -> Ret {
    for name in defined_names(vm) {
        set_state(vm, &disabled_key(&name), Value::Nil);
    }
    let disabled: Vec<Value> = {
        let t = state(vm);
        vm.heap
            .table(t)
            .entries()
            .map(|(k, _)| k)
            .filter(|k| matches!(k, Value::Str(s) if vm.str_bytes(*s).starts_with(b"disabled:")))
            .collect()
    };
    let t = state(vm);
    for k in disabled {
        let _ = vm.heap.table_mut(t).set(k, Value::Nil);
    }
    Ok(vec![])
}

fn disable_all_addons(vm: &mut Vm, _c: NativeCall) -> Ret {
    for name in defined_names(vm) {
        set_state(vm, &disabled_key(&name), Value::Bool(true));
    }
    Ok(vec![])
}

/// `C_AddOns.GetAddOnName(index)`: a defined add-on's name.
fn get_addon_name(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match addon(vm, c.arg(1)) {
        Some(n) => vm.str(&n),
        None => Value::Nil,
    }])
}

/// `C_AddOns.GetAddOnTitle(name)`: its TOC title, else its name.
fn get_addon_title(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = addon(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let title = metadata(vm, &name, "Title").unwrap_or(name);
    Ok(vec![vm.str(&title)])
}

/// `C_AddOns.GetAddOnNotes(name)`: its TOC notes, nil without (as
/// `GetAddOnInfo`).
fn get_addon_notes(vm: &mut Vm, c: NativeCall) -> Ret {
    let notes = addon(vm, c.arg(1)).and_then(|n| metadata(vm, &n, "Notes"));
    Ok(vec![match notes {
        Some(n) => vm.str(&n),
        None => Value::Nil,
    }])
}

/// The names a TOC field lists, comma separated.
fn listed(vm: &mut Vm, c: &NativeCall, keys: &[&str]) -> Vec<Value> {
    let Some(name) = addon(vm, c.arg(1)) else {
        return vec![];
    };
    let Some(field) = keys.iter().find_map(|k| metadata(vm, &name, k)) else {
        return vec![];
    };
    field
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| vm.str(s))
        .collect::<Vec<_>>()
}

/// `C_AddOns.GetAddOnDependencies(name)`: the required add-ons, one
/// result each (the client returns them so; unverified).
fn get_addon_dependencies(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(listed(
        vm,
        &c,
        &["Dependencies", "RequiredDeps", "Dependancies"],
    ))
}

fn get_addon_optional_dependencies(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(listed(vm, &c, &["OptionalDeps", "OptionalDependencies"]))
}

/// `C_AddOns.GetAddOnSecurity(name)`: Blizzard's secure (0), others
/// insecure (1), unknown ones not available (3) (`Enum.AddOnSecurityStatus`;
/// unverified).
fn get_addon_security(vm: &mut Vm, c: NativeCall) -> Ret {
    let status = match addon(vm, c.arg(1)) {
        Some(n) if n.starts_with("Blizzard_") => 0.0,
        Some(_) => 1.0,
        None => 3.0,
    };
    Ok(vec![Value::Num(status)])
}

/// `C_AddOns.GetAddOnInterfaceVersion(name)`: its TOC's first interface
/// number, else 0.
fn get_addon_interface_version(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = addon(vm, c.arg(1))
        .and_then(|n| metadata(vm, &n, "Interface"))
        .and_then(|i| {
            i.split(',')
                .next()
                .and_then(|s| s.trim().parse::<f64>().ok())
        })
        .unwrap_or(0.0);
    Ok(vec![Value::Num(v)])
}

/// `C_AddOns.IsAddOnLoadable(name)`: loadable with no reason; disabled
/// `"DISABLED"`, unknown `"MISSING"` (the reasons' global strings'
/// keys; unverified).
fn is_addon_loadable(vm: &mut Vm, c: NativeCall) -> Ret {
    match addon(vm, c.arg(1)) {
        Some(n) if is_disabled(vm, &n) => Ok(vec![Value::Bool(false), vm.str("DISABLED")]),
        Some(_) => Ok(vec![Value::Bool(true), Value::Nil]),
        None => Ok(vec![Value::Bool(false), vm.str("MISSING")]),
    }
}

/// `C_AddOns.IsAddOnDefaultEnabled(name)`: unless its TOC's `DefaultState`
/// is disabled.
fn is_addon_default_enabled(vm: &mut Vm, c: NativeCall) -> Ret {
    let enabled = addon(vm, c.arg(1)).is_some_and(|n| {
        !metadata(vm, &n, "DefaultState").is_some_and(|s| s.eq_ignore_ascii_case("disabled"))
    });
    Ok(vec![Value::Bool(enabled)])
}

// ----------------------------------------------------------- bindings

/// Binds `key` to `command` as `SetBinding` does: a key in lower case
/// binds nothing (false); a binding set tells `UPDATE_BINDINGS`
/// (unverified, as `SetBinding`'s).
fn bind(vm: &mut Vm, key: Option<String>, command: Option<String>) -> Ret {
    let Some(key) = key else {
        return Ok(vec![Value::Bool(false)]);
    };
    if key != key.to_uppercase() {
        return Ok(vec![Value::Bool(false)]);
    }
    let keys = &mut engine_mut(vm).world.bindings.keys;
    keys.retain(|(k, _)| *k != key);
    if let Some(command) = command {
        keys.push((key, command));
    }
    widgets::fire_event(vm, "UPDATE_BINDINGS", &[]);
    Ok(vec![Value::Bool(true)])
}

/// `SetBindingClick(key, button [, mouseButton])`: the key clicks the
/// button (`CLICK name:LeftButton` by default), as override bindings
/// write it.
fn set_binding_click(vm: &mut Vm, c: NativeCall) -> Ret {
    let key = text(vm, c.arg(1));
    let button = text(vm, c.arg(2));
    let mouse = text(vm, c.arg(3)).unwrap_or_else(|| "LeftButton".into());
    bind(vm, key, button.map(|b| format!("CLICK {b}:{mouse}")))
}

/// `SetBindingSpell(key, spell)`, `SetBindingItem`, `SetBindingMacro`:
/// the key casts, uses or runs it (`SPELL name`, `ITEM name`, `MACRO
/// name`).
fn set_binding_with(vm: &mut Vm, c: &NativeCall, kind: &str) -> Ret {
    let key = text(vm, c.arg(1));
    let what = text(vm, c.arg(2));
    bind(vm, key, what.map(|w| format!("{kind} {w}")))
}

fn set_binding_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    set_binding_with(vm, &c, "SPELL")
}

fn set_binding_item(vm: &mut Vm, c: NativeCall) -> Ret {
    set_binding_with(vm, &c, "ITEM")
}

fn set_binding_macro(vm: &mut Vm, c: NativeCall) -> Ret {
    set_binding_with(vm, &c, "MACRO")
}

/// `LoadBindings(set)`: the saved bindings back, `UPDATE_BINDINGS` (the
/// engine keeps one set; unverified).
fn load_bindings(vm: &mut Vm, _c: NativeCall) -> Ret {
    widgets::fire_event(vm, "UPDATE_BINDINGS", &[]);
    Ok(vec![])
}

/// `SaveBindings(set)`: kept (the engine keeps one set), `UPDATE_BINDINGS`
/// (unverified: warcraft.wiki.gg's UPDATE_BINDINGS names SaveBindings).
fn save_bindings(vm: &mut Vm, _c: NativeCall) -> Ret {
    widgets::fire_event(vm, "UPDATE_BINDINGS", &[]);
    Ok(vec![])
}

fn context_key(vm: &Vm, v: Value) -> String {
    format!("context:{}", num(vm, v).unwrap_or(0.0))
}

/// `C_KeyBindings.ActivateBindingContext(context)`, `Deactivate…`,
/// `IsBindingContextActive`: kept (none active at first).
fn activate_binding_context(vm: &mut Vm, c: NativeCall) -> Ret {
    let k = context_key(vm, c.arg(1));
    set_state(vm, &k, Value::Bool(true));
    Ok(vec![])
}

fn deactivate_binding_context(vm: &mut Vm, c: NativeCall) -> Ret {
    let k = context_key(vm, c.arg(1));
    set_state(vm, &k, Value::Nil);
    Ok(vec![])
}

///
/// Context 0 (the default) is active unless deactivated (the full probe,
/// 1.60.1: 0 and "x" true, 1 to 9 false).
fn is_binding_context_active(vm: &mut Vm, c: NativeCall) -> Ret {
    let k = context_key(vm, c.arg(1));
    let active = match get_state(vm, &k) {
        Value::Nil => num(vm, c.arg(1)).unwrap_or(0.0) == 0.0,
        v => v.truthy(),
    };
    Ok(vec![Value::Bool(active)])
}

/// `C_KeyBindings.GetTurnStrafeStyle()`: as set, else 2 (recorded with
/// the character's settings; `Enum.TurnStrafeStyle` documents 0 and 1
/// only, unverified what 2 is).
fn get_turn_strafe_style(vm: &mut Vm, _c: NativeCall) -> Ret {
    let v = get_state(vm, "turnStrafeStyle");
    Ok(vec![if v.is_nil() { Value::Num(2.0) } else { v }])
}

fn set_turn_strafe_style(vm: &mut Vm, c: NativeCall) -> Ret {
    // Only modern or legacy (documented).
    if let Some(s) = num(vm, c.arg(1))
        && (s == 0.0 || s == 1.0)
    {
        set_state(vm, "turnStrafeStyle", Value::Num(s));
    }
    Ok(vec![])
}

// --------------------------------------------------------------- stats

/// `GetHaste()`: the melee haste the player's stats hold (`GetMeleeHaste`),
/// else 0 (unverified).
fn get_haste(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(one_num(vm, "meleeHaste").unwrap_or(0.0))])
}

/// `GetMastery()`: none, as classic has no mastery rating (the recorded
/// `GetMasteryEffect` is 0; unverified).
fn get_mastery(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `GetExpertisePercent()`: none for each hand and ranged (classic-era
/// gear has no expertise; unverified).
fn get_expertise_percent(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 3])
}

/// `GetPowerRegenForPowerType(powerType)`: the player's power's
/// regeneration (`GetPowerRegen`, recorded per character) for its own
/// power type; for another a base of 0.001 each, rage decaying 12.5 less
/// out of combat; none for a value no power type has (the full probe, a
/// paladin: -1 0.001, 0.001; rage, 1 and 1.5, -12.499, 0.001; 2147483647
/// 0, 0; as C floats; the other types' and classes' unverified).
fn get_power_regen_for_power_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let asked = num(vm, c.arg(1));
    let own = engine(vm).units.list[units::PLAYER.0].power_type;
    if asked == Some(own)
        && let Some(Entry::One(v)) = engine(vm).world.character.stats.get("powerRegen")
    {
        let v = v.clone();
        let mut out: Vec<Value> = v.iter().take(2).map(|a| answer_value(vm, a)).collect();
        out.resize(2, Value::Num(0.0));
        return Ok(out);
    }
    let base = 0.001_f32;
    let regen = match asked.map(f64::trunc) {
        Some(1.0) => base - 12.5,
        // `Enum.PowerType`'s range, `HealthCost` to the last.
        Some(t) if (-2.0..=29.0).contains(&t) => base,
        _ => return Ok(vec![Value::Num(0.0), Value::Num(0.0)]),
    };
    Ok(vec![
        Value::Num(f64::from(regen)),
        Value::Num(f64::from(base)),
    ])
}

// --------------------------------------------------------- raid marks

/// `CanBeRaidTarget(unit)`: any unit that exists (unverified: hostile
/// players in instances).
fn can_be_raid_target(vm: &mut Vm, c: NativeCall) -> Ret {
    let exists = units::unit_arg(vm, &c, 1).is_some();
    Ok(vec![Value::Bool(exists)])
}

// ----------------------------------------------------------- currency

/// Honor Points as the client answered for a character without any
/// (recorded: `GetCurrencyInfo(1792)`).
fn honor_points() -> Fields {
    let s = |k: &str| Answer::Str(k.to_string());
    let n = Answer::Num;
    let b = Answer::Bool;
    vec![
        (s("canEarnPerWeek"), b(false)),
        (s("currencyID"), n(1792.0)),
        (s("currencyListDepth"), n(0.0)),
        (
            s("description"),
            s(
                "Honor is gained by killing members of the opposite faction in PvP combat. You can use honor points to purchase special items.",
            ),
        ),
        (s("discovered"), b(false)),
        (s("iconFileID"), n(2173920.0)),
        (s("isAccountTransferable"), b(false)),
        (s("isAccountWide"), b(false)),
        (s("isHeader"), b(false)),
        (s("isHeaderExpanded"), b(false)),
        (s("isShowInBackpack"), b(false)),
        (s("isTradeable"), b(false)),
        (s("isTypeUnused"), b(false)),
        (s("maxQuantity"), n(25000.0)),
        (s("maxWeeklyQuantity"), n(0.0)),
        (s("name"), s("Honor Points")),
        (s("quality"), n(1.0)),
        (s("quantity"), n(0.0)),
        (s("quantityEarnedThisWeek"), n(0.0)),
        (s("rechargingAmountPerCycle"), n(0.0)),
        (s("rechargingCycleDurationMS"), n(0.0)),
        (s("totalEarned"), n(0.0)),
        (s("trackedQuantity"), n(0.0)),
        (s("transferPercentage"), n(0.0)),
        (s("useTotalEarnedForMaxQty"), b(false)),
    ]
}

/// A currency's info: the player's currency list's entry for it, else
/// Honor Points' recorded answer; none else (recorded: 1602 answers
/// nothing).
fn currency(vm: &Vm, id: f64) -> Option<Fields> {
    if let Some((_, f)) = indexed(vm, "currencyListInfo")
        .into_iter()
        .find(|(_, f)| field_num(f, "currencyID") == Some(id))
    {
        return Some(f);
    }
    (id == 1792.0).then(honor_points)
}

fn currency_arg(vm: &Vm, c: &NativeCall) -> Option<Fields> {
    num(vm, c.arg(1)).and_then(|id| currency(vm, id))
}

/// `C_CurrencyInfo.GetCurrencyInfo(type)`.
fn get_currency_info(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match currency_arg(vm, &c) {
        Some(f) => vec![answer_value(vm, &Answer::Table(f))],
        None => vec![],
    })
}

/// The currency a link names (`|Hcurrency:ID…`).
fn link_currency_id(vm: &Vm, v: Value) -> Option<f64> {
    let link = text(vm, v)?;
    let rest = &link[link.find("currency:")? + "currency:".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// `C_CurrencyInfo.GetCurrencyIDFromLink(link)`: its ID, else 0.
fn get_currency_id_from_link(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        link_currency_id(vm, c.arg(1)).unwrap_or(0.0),
    )])
}

fn get_currency_info_from_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let info = link_currency_id(vm, c.arg(1)).and_then(|id| currency(vm, id));
    Ok(match info {
        Some(f) => vec![answer_value(vm, &Answer::Table(f))],
        None => vec![],
    })
}

/// A currency's link, colored by its quality (item links' form;
/// unverified).
fn currency_link(f: &[(Answer, Answer)], amount: f64) -> String {
    let color = match field_num(f, "quality").unwrap_or(1.0) as i64 {
        0 => "ff9d9d9d",
        2 => "ff1eff00",
        3 => "ff0070dd",
        4 => "ffa335ee",
        5 => "ffff8000",
        6 => "ffe6cc80",
        7 => "ff00ccff",
        _ => "ffffffff",
    };
    let id = field_num(f, "currencyID").unwrap_or(0.0);
    let name = field_str(f, "name").unwrap_or_default();
    format!(
        "|c{color}|Hcurrency:{}:{}|h[{name}]|h|r",
        id as i64, amount as i64
    )
}

/// `C_CurrencyInfo.GetCurrencyLink(type [, amount])`: nil for a currency
/// not known here (the full probe: IDs -1 to 2147483647).
fn get_currency_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let amount = num(vm, c.arg(2)).unwrap_or(0.0);
    let link = currency_arg(vm, &c).map(|f| currency_link(&f, amount));
    Ok(vec![match link {
        Some(l) => vm.str(&l),
        None => Value::Nil,
    }])
}

/// `C_CurrencyInfo.GetCurrencyListLink(index)`: the list entry's link.
fn get_currency_list_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = num(vm, c.arg(1)).unwrap_or(0.0) as i64;
    let link = indexed(vm, "currencyListInfo")
        .into_iter()
        .find(|(k, _)| *k == i)
        .filter(|(_, f)| !field_bool(f, "isHeader"))
        .map(|(_, f)| currency_link(&f, field_num(&f, "quantity").unwrap_or(0.0)));
    Ok(vec![match link {
        Some(l) => vm.str(&l),
        None => Value::Nil,
    }])
}

/// `C_CurrencyInfo.GetBasicCurrencyInfo(type [, quantity])`: its name,
/// description, icon, quality, and the amount (the one given, else held).
fn get_basic_currency_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(f) = currency_arg(vm, &c) else {
        return Ok(vec![]);
    };
    let amount = num(vm, c.arg(2))
        .or_else(|| field_num(&f, "quantity"))
        .unwrap_or(0.0);
    let t = vm.new_table();
    let name = vm.str(&field_str(&f, "name").unwrap_or_default());
    let description = vm.str(&field_str(&f, "description").unwrap_or_default());
    set_field(vm, t, "name", name);
    set_field(vm, t, "description", description);
    for (k, v) in [
        ("icon", field_num(&f, "iconFileID").unwrap_or(0.0)),
        ("quality", field_num(&f, "quality").unwrap_or(1.0)),
        ("displayAmount", amount),
        ("actualAmount", amount),
    ] {
        set_field(vm, t, k, Value::Num(v));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_CurrencyInfo.GetCurrencyDescription(type)`: nil for one not known
/// (the full probe).
fn get_currency_description(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(f) = currency_arg(vm, &c) else {
        return Ok(vec![Value::Nil]);
    };
    let d = field_str(&f, "description").unwrap_or_default();
    Ok(vec![vm.str(&d)])
}

/// `C_CurrencyInfo.PlayerHasMaxQuantity(id)`: held at a maximum it has.
fn player_has_max_quantity(vm: &mut Vm, c: NativeCall) -> Ret {
    let full = currency_arg(vm, &c).is_some_and(|f| {
        let max = field_num(&f, "maxQuantity").unwrap_or(0.0);
        max > 0.0 && field_num(&f, "quantity").unwrap_or(0.0) >= max
    });
    Ok(vec![Value::Bool(full)])
}

fn player_has_max_weekly_quantity(vm: &mut Vm, c: NativeCall) -> Ret {
    let full = currency_arg(vm, &c).is_some_and(|f| {
        let max = field_num(&f, "maxWeeklyQuantity").unwrap_or(0.0);
        max > 0.0 && field_num(&f, "quantityEarnedThisWeek").unwrap_or(0.0) >= max
    });
    Ok(vec![Value::Bool(full)])
}

fn is_account_wide_currency(vm: &mut Vm, c: NativeCall) -> Ret {
    let wide = currency_arg(vm, &c).is_some_and(|f| field_bool(&f, "isAccountWide"));
    Ok(vec![Value::Bool(wide)])
}

fn is_account_transferable_currency(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = currency_arg(vm, &c).is_some_and(|f| field_bool(&f, "isAccountTransferable"));
    Ok(vec![Value::Bool(t)])
}

/// `C_CurrencyInfo.CanTransferCurrency(id)`: no (no account transfers on
/// Forever); a currency not known: `InvalidCurrency` (5, the full probe),
/// a known one no reason (unverified).
fn can_transfer_currency(vm: &mut Vm, c: NativeCall) -> Ret {
    let reason = if currency_arg(vm, &c).is_some() {
        Value::Nil
    } else {
        Value::Num(5.0)
    };
    Ok(vec![Value::Bool(false), reason])
}

/// `C_CurrencyInfo.DoesWarModeBonusApply(id)`: no war mode, both nil (the
/// full probe, currencies not held; unverified for one held).
fn does_war_mode_bonus_apply(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil, Value::Nil])
}

/// The retail currencies' IDs the client still answers (Azerite 1553,
/// War Resources 1560, Dragon Isles Supplies 2003; unverified on Forever).
fn azerite_currency_id(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1553.0)])
}

fn war_resources_currency_id(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(1560.0)])
}

fn dragon_isles_supplies_currency_id(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(2003.0)])
}

/// `C_CurrencyInfo.GetPlayerCurrencyCategoryInfo(categoryID)`: the
/// category's name, the ID taken whole, no currencies (the full probe,
/// 1.60.1: 1 and 1.5 "Miscellaneous", -1, 0 and 2147483647 no name;
/// unverified: the other categories' names, here none, and their
/// currencies).
fn get_player_currency_category_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.new_table();
    let id = num(vm, c.arg(1)).map(f64::trunc);
    let name = vm.str(if id == Some(1.0) { "Miscellaneous" } else { "" });
    set_field(vm, t, "categoryName", name);
    for k in ["currencyTypes", "childCategories"] {
        let l = vm.new_table();
        set_field(vm, t, k, Value::Table(l));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_CurrencyInfo.GetCoinIcon(amount)`: the largest coin's icon (the
/// full probe: copper 133788, gold 133785, `INV_Misc_Coin_*`; silver
/// 133787 unverified, between them).
fn get_coin_icon(vm: &mut Vm, c: NativeCall) -> Ret {
    let amount = num(vm, c.arg(1)).unwrap_or(0.0);
    let icon = if amount >= 10000.0 {
        133785.0
    } else if amount >= 100.0 {
        133787.0
    } else {
        133788.0
    };
    Ok(vec![Value::Num(icon)])
}

/// `C_CurrencyInfo.GetCoinText(amount [, separator])`: "1 Gold, 2 Silver,
/// 3 Copper" by the `GOLD_AMOUNT` strings, parts that are 0 left out; 0
/// as nothing ("", the full probe).
fn get_coin_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let amount = num(vm, c.arg(1)).unwrap_or(0.0).max(0.0).floor() as i64;
    if amount == 0 {
        return Ok(vec![vm.str("")]);
    }
    let sep = text(vm, c.arg(2)).unwrap_or_else(|| ", ".into());
    let mut part = |global: &str, fallback: &str, n: i64| {
        let f = vm.get_global(global);
        let f = text(vm, f).unwrap_or_else(|| fallback.to_string());
        f.replacen("%d", &n.to_string(), 1)
    };
    let (gold, silver, copper) = (amount / 10000, amount / 100 % 100, amount % 100);
    let mut parts = Vec::new();
    if gold > 0 {
        parts.push(part("GOLD_AMOUNT", "%d Gold", gold));
    }
    if silver > 0 {
        parts.push(part("SILVER_AMOUNT", "%d Silver", silver));
    }
    if copper > 0 || parts.is_empty() {
        parts.push(part("COPPER_AMOUNT", "%d Copper", copper));
    }
    let joined = parts.join(sep.as_str());
    Ok(vec![vm.str(&joined)])
}

// -------------------------------------------------------- reputation

/// `C_Reputation.GetFactionDataByID(id)`: the player's reputation list's
/// entry for it, else nil (the client knows factions the list lacks:
/// unverified those).
fn get_faction_data_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = num(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let found = indexed(vm, "faction")
        .into_iter()
        .find(|(_, f)| field_num(f, "factionID") == Some(id));
    Ok(vec![match found {
        Some((_, f)) => answer_value(vm, &Answer::Table(f)),
        None => Value::Nil,
    }])
}

/// `C_Reputation.SetSelectedFaction(index)`: what
/// `GetSelectedFaction` answers. Unverified: 0 is taken, though the full
/// probe's client kept 12 after `SetSelectedFaction(0)`; refusing it would
/// leave the engine's own start (0, no selection) beyond reach of the
/// probe plan's restore.
fn set_selected_faction(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = num(vm, c.arg(1)).unwrap_or(0.0).trunc();
    engine_mut(vm)
        .world
        .character
        .stats
        .insert("selectedFaction", Entry::One(vec![Answer::Num(i)]));
    Ok(vec![])
}

/// Sets each faction's `isWatched` (the one at `index` watched, the
/// others not) and the watched faction's data, then `UPDATE_FACTION`
/// (unverified the event).
fn watch_faction(vm: &mut Vm, index: Option<i64>) {
    let stats = &mut engine_mut(vm).world.character.stats;
    let mut watched = Answer::Nil;
    if let Some(Entry::ByIndex(m)) = stats.get_mut("faction") {
        for (i, answers) in m.iter_mut() {
            if let Some(Answer::Table(fields)) = answers.first_mut() {
                let on = Some(*i) == index;
                for (k, v) in fields.iter_mut() {
                    if matches!(k, Answer::Str(s) if s.as_str() == "isWatched") {
                        *v = Answer::Bool(on);
                    }
                }
                if on {
                    watched = Answer::Table(fields.clone());
                }
            }
        }
    }
    stats.insert("watchedFaction", Entry::One(vec![watched]));
    widgets::fire_event(vm, "UPDATE_FACTION", &[]);
}

/// `C_Reputation.SetWatchedFactionByIndex(index)`: that faction watched
/// (0 or none there: none), once the server answers.
fn set_watched_faction_by_index(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = num(vm, c.arg(1)).map(|i| i as i64);
    let exists = indexed(vm, "faction").iter().any(|(k, _)| Some(*k) == i);
    request_watch(vm, i.filter(|_| exists));
    Ok(vec![])
}

/// `C_Reputation.SetWatchedFactionByID(factionID)`: as by its index; a
/// header's ID 0 watches none.
fn set_watched_faction_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(vm, c.arg(1)).filter(|&id| id > 0.0);
    let i = indexed(vm, "faction")
        .into_iter()
        .find(|(_, f)| id.is_some() && field_num(f, "factionID") == id)
        .map(|(i, _)| i);
    request_watch(vm, i);
    Ok(vec![])
}

/// A watched faction asked of the server: the watch changes when it
/// answers, the last asked winning (the full probe, 1.60.1:
/// `GetWatchedFactionData()` still nil right after
/// `SetWatchedFactionByID(530)` and `SetWatchedFactionByIndex(2)`, a
/// faction the character has; unverified: the answer's delay, here the
/// server's).
fn request_watch(vm: &mut Vm, index: Option<i64>) {
    let asked = index.map_or(Value::Bool(false), |i| Value::Num(i as f64));
    set_state(vm, "watchRequest", asked);
    let answer = vm.native("WatchFactionAnswer", |vm, _c| {
        match get_state(vm, "watchRequest") {
            Value::Num(i) => watch_faction(vm, Some(i as i64)),
            Value::Bool(false) => watch_faction(vm, None),
            _ => return Ok(vec![]),
        }
        set_state(vm, "watchRequest", Value::Nil);
        Ok(vec![])
    });
    let delay = crate::clock::server_delay(vm);
    crate::clock::after_engine(vm, delay, answer);
}

/// `C_Reputation.ToggleFactionAtWar(index)`: at war or not, where it can
/// be toggled; `UPDATE_FACTION` (unverified).
fn toggle_faction_at_war(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = num(vm, c.arg(1)).map(|i| i as i64);
    let mut changed = false;
    if let Some(i) = i
        && let Some(Entry::ByIndex(m)) = engine_mut(vm).world.character.stats.get_mut("faction")
        && let Some(answers) = m.get_mut(&i)
        && let Some(Answer::Table(fields)) = answers.first_mut()
        && field_bool(fields, "canToggleAtWar")
    {
        for (k, v) in fields.iter_mut() {
            if matches!(k, Answer::Str(s) if s.as_str() == "atWarWith") {
                *v = Answer::Bool(!matches!(v, Answer::Bool(true)));
                changed = true;
            }
        }
    }
    if changed {
        widgets::fire_event(vm, "UPDATE_FACTION", &[]);
    }
    Ok(vec![])
}

fn inactive_key(vm: &Vm, v: Value) -> String {
    format!("inactiveFaction:{}", num(vm, v).unwrap_or(0.0))
}

/// `C_Reputation.IsFactionActive(index)`: unless set inactive.
fn is_faction_active(vm: &mut Vm, c: NativeCall) -> Ret {
    let k = inactive_key(vm, c.arg(1));
    Ok(vec![Value::Bool(!get_state(vm, &k).truthy())])
}

fn set_faction_active(vm: &mut Vm, c: NativeCall) -> Ret {
    let k = inactive_key(vm, c.arg(1));
    let v = if c.arg(2).truthy() {
        Value::Nil
    } else {
        Value::Bool(true)
    };
    set_state(vm, &k, v);
    Ok(vec![])
}

/// `C_Reputation.AreLegacyReputationsShown()`: as set, shown at first
/// (unverified).
fn are_legacy_reputations_shown(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        !get_state(vm, "legacyReputationsHidden").truthy(),
    )])
}

fn set_legacy_reputations_shown(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = if c.arg(1).truthy() {
        Value::Nil
    } else {
        Value::Bool(true)
    };
    set_state(vm, "legacyReputationsHidden", v);
    Ok(vec![])
}

/// `C_Reputation.GetReputationSortType()`: as set, none (0) at first.
fn get_reputation_sort_type(vm: &mut Vm, _c: NativeCall) -> Ret {
    let v = get_state(vm, "reputationSortType");
    Ok(vec![if v.is_nil() { Value::Num(0.0) } else { v }])
}

fn set_reputation_sort_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = num(vm, c.arg(1)).map_or(Value::Nil, Value::Num);
    set_state(vm, "reputationSortType", v);
    Ok(vec![])
}

// ------------------------------------------------------------- CVars

fn cvar(vm: &Vm, name: &str) -> Option<String> {
    engine(vm)
        .world
        .cvars
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn put_cvar(vm: &mut Vm, name: &str, value: String) {
    let cvars = &mut engine_mut(vm).world.cvars;
    match cvars.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
        Some(slot) => slot.1 = value,
        None => cvars.push((name.to_string(), value)),
    }
}

/// `C_CVar.SetCVarBitfield(name, index, value)`: bit `index` set or
/// cleared in the six bits a character above `@` that `GetCVarBitfield`
/// reads, after the value's header; `CVAR_UPDATE` (unverified: the header
/// of a value that had none, the event).
fn set_cvar_bitfield(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = text(vm, c.arg(1)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let index = num(vm, c.arg(2)).unwrap_or(0.0) as i64;
    if index < 1 {
        return Ok(vec![Value::Bool(false)]);
    }
    let set = c.arg(3).truthy();
    let value = cvar(vm, &name).unwrap_or_default();
    let mut bytes = value.into_bytes();
    let header = bytes.iter().take_while(|b| **b < b'@').count();
    let i = (index - 1) as usize;
    let at = header + i / 6;
    while bytes.len() <= at {
        bytes.push(b'@');
    }
    let bits = bytes[at].wrapping_sub(b'@');
    let bit: u8 = 1 << (i % 6);
    let bits = if set { bits | bit } else { bits & !bit };
    bytes[at] = b'@' + bits;
    let value = String::from_utf8_lossy(&bytes).into_owned();
    put_cvar(vm, &name, value.clone());
    let n = vm.str(&name);
    let v = vm.str(&value);
    widgets::fire_event(vm, "CVAR_UPDATE", &[n, v]);
    Ok(vec![Value::Bool(true)])
}

/// `C_CVar.SetTempCVar(name [, value])`: the value for now, the old one
/// kept for `RemoveTempCVar` (unverified: not saved; no `CVAR_UPDATE`).
fn set_temp_cvar(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = text(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let key = format!("tempCVar:{}", name.to_lowercase());
    if get_state(vm, &key).is_nil() {
        let old = cvar(vm, &name).unwrap_or_default();
        let old = vm.str(&old);
        set_state(vm, &key, old);
    }
    let value = text(vm, c.arg(2)).unwrap_or_default();
    put_cvar(vm, &name, value);
    Ok(vec![])
}

fn remove_temp_cvar(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = text(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let key = format!("tempCVar:{}", name.to_lowercase());
    let old = get_state(vm, &key);
    if let Some(old) = text(vm, old) {
        put_cvar(vm, &name, old);
        set_state(vm, &key, Value::Nil);
    }
    Ok(vec![])
}

/// `C_GameRules.IsPersonalResourceDisplayEnabled()`: the
/// `nameplateShowSelf` CVar (documented: the game rule as well; Forever's
/// rule allows it, unverified).
fn is_personal_resource_display_enabled(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = cvar(vm, "nameplateShowSelf").is_some_and(|v| v != "0" && !v.is_empty());
    Ok(vec![Value::Bool(on)])
}

/// `C_GameRules.IsClassAllowedForGameMode(classID)`: classic's nine
/// classes (unverified).
fn is_class_allowed_for_game_mode(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(vm, c.arg(1)).unwrap_or(0.0) as i64;
    Ok(vec![Value::Bool(matches!(id, 1..=5 | 7..=9 | 11))])
}

// ------------------------------------------------------------ minimap

/// `C_Minimap.GetViewRadius()`: the outdoor radius at zoom 0, 233 1/3
/// yards as a C float (recorded 233.33332824707; unverified: zoom levels,
/// indoors).
fn get_view_radius(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(f64::from(700.0f32 / 3.0))])
}

/// `C_Minimap.GetUiMapID()`: none (SuuUI probes minimap and marks: nil
/// while `C_Map.GetBestMapForUnit` named the player's map; Blizzard's
/// Minimap.lua asks the player's map itself unless a hybrid minimap).
fn get_minimap_ui_map_id(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

/// The minimap's tracking types: name, type and filter (a spell's ID or
/// an `Enum.MinimapTrackingFilter`; `data/camelot-tracking.txt`, as the
/// recording's character had them: its tracking spells first, then the
/// townsfolk and others).
fn tracking_types() -> &'static [(&'static str, &'static str, f64)] {
    static TYPES: std::sync::OnceLock<Vec<(&'static str, &'static str, f64)>> =
        std::sync::OnceLock::new();
    TYPES.get_or_init(|| {
        include_str!("../data/camelot-tracking.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut parts = l.split('\t');
                let name = parts.next()?;
                let kind = parts.next()?;
                let id = parts.next()?.parse().ok()?;
                Some((name, kind, id))
            })
            .collect()
    })
}

/// `C_Minimap.GetNumTrackingTypes()`: the tracking types listed (recorded
/// 25 for a paladin with mining; unverified: other classes' and
/// professions' spells).
fn get_num_tracking_types(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(tracking_types().len() as f64)])
}

/// The tracking type at a 1-based index.
fn tracking_type(vm: &Vm, v: Value) -> Option<(&'static str, &'static str, f64)> {
    let i = num(vm, v)?;
    let types = tracking_types();
    (i >= 1.0 && i <= types.len() as f64).then(|| types[i as usize - 1])
}

/// The filters tracked: the bits of `minimapTrackedInfov4` (recorded
/// 10552047 for the paladin, the same before and after its townsfolk were
/// toggled: the CVar is not told changed as they are).
fn tracked_filters(vm: &mut Vm) -> u64 {
    if let Value::Num(n) = get_state(vm, "trackedFilters") {
        return n as u64;
    }
    crate::framescript::cvar_get(vm, "minimapTrackedInfov4")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// `C_Minimap.IsFilteredOut(filterType)`: a tracking filter's bit not
/// tracked; 0 and a number of more bits none (the full probe, 1.60.1:
/// every filter of `Enum.MinimapTrackingFilter` out exactly when its bit
/// was clear in `minimapTrackedInfov4`, 10552047; 16777217 not out).
fn is_filtered_out(vm: &mut Vm, c: NativeCall) -> Ret {
    let filter = num(vm, c.arg(1)).map_or(0, |n| n.max(0.0) as u64);
    let out = filter.is_power_of_two() && tracked_filters(vm) & filter == 0;
    Ok(vec![Value::Bool(out)])
}

/// `C_Minimap.IsTrackingAccountCompletedQuests()`: its filter's bit
/// tracked (the full probe, 1.60.1: true with 2097152 set).
fn tracking_account_completed_quests(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(tracked_filters(vm) & 2_097_152 != 0)])
}

/// Whether a tracking type is on: a spell's while its aura is on the
/// player, a filter's while its bit is set.
fn tracking_active(vm: &mut Vm, kind: &str, id: f64) -> bool {
    if kind == "spell" {
        engine(vm)
            .units
            .auras
            .iter()
            .any(|a| a.unit == crate::units::PLAYER && a.spell_id == id)
    } else {
        tracked_filters(vm) & (id as u64) != 0
    }
}

/// `C_Minimap.GetTrackingInfo(index)`: the type's name, kind, state and a
/// spell's ID and icon (the spellbook's); a spell's subtype 1 (the full
/// probe, 1.60.1: Sense Undead's); a filter's texture and subtype are not
/// recorded (none, 0: unverified); nil past the list.
fn get_tracking_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((name, kind, id)) = tracking_type(vm, c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let active = tracking_active(vm, kind, id);
    let texture = if kind == "spell" {
        match crate::stats::spellbook_item(vm, id) {
            Some(Value::Table(item)) => {
                let k = vm.str("iconID");
                vm.heap.table(item).get(k)
            }
            _ => Value::Num(0.0),
        }
    } else {
        Value::Num(0.0)
    };
    let t = vm.new_table();
    let mut fields = vec![
        ("name", vm.str(name)),
        ("texture", texture),
        ("active", Value::Bool(active)),
        ("type", vm.str(kind)),
        (
            "subType",
            Value::Num(if kind == "spell" { 1.0 } else { 0.0 }),
        ),
    ];
    if kind == "spell" {
        fields.push(("spellID", Value::Num(id)));
    }
    for (k, v) in fields {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(t).set(k, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `C_Minimap.GetTrackingFilter(index)`: `{ spellID }` for a spell,
/// `{ filterID }` else (documented; see the data).
fn get_tracking_filter(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.new_table();
    if let Some((_, kind, id)) = tracking_type(vm, c.arg(1)) {
        let k = vm.str(if kind == "spell" {
            "spellID"
        } else {
            "filterID"
        });
        let _ = vm.heap.table_mut(t).set(k, Value::Num(id));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_Minimap.SetTracking(index, on)`: nothing when it already is so. A
/// filter's bit set or cleared, `MINIMAP_UPDATE_TRACKING` at once
/// (recorded four times: a town's tracking types toggled from Blizzard's
/// menu). A spell turned on is used as the player's press (recorded: Find
/// Minerals, its cast a round trip later, its aura then); turned off, its
/// aura cancelled a round trip later: its action slots told changed, the
/// aura's removal, the bars' cooldowns (recorded once). A tracking spell's
/// aura gained or lost tells `MINIMAP_UPDATE_TRACKING`
/// (`tracking_aura_changed`).
fn set_tracking(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((_, kind, id)) = tracking_type(vm, c.arg(1)) else {
        return Ok(vec![]);
    };
    let on = c.arg(2).truthy();
    if tracking_active(vm, kind, id) == on {
        return Ok(vec![]);
    }
    if kind == "spell" {
        if on {
            return crate::casting::use_spell(vm, id);
        }
        let later = vm.native_with(
            "trackingCancelled",
            tracking_cancelled,
            vec![Value::Num(id)],
        );
        let delay = crate::clock::server_delay(vm);
        crate::clock::after_engine(vm, delay, later);
        return Ok(vec![]);
    }
    let bits = tracked_filters(vm) ^ (id as u64);
    set_state(vm, "trackedFilters", Value::Num(bits as f64));
    crate::widgets::fire_event(vm, "MINIMAP_UPDATE_TRACKING", &[]);
    Ok(vec![])
}

/// A tracking spell's aura cancelled by the server (see `set_tracking`).
fn tracking_cancelled(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(id) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let slots: Vec<i64> = engine(vm)
        .world
        .character
        .actions
        .iter()
        .filter(|(_, a)| a.kind == "spell" && matches!(a.id, Value::Num(n) if n == id))
        .map(|(s, _)| *s)
        .collect();
    for s in slots {
        crate::widgets::fire_event(vm, "ACTIONBAR_SLOT_CHANGED", &[Value::Num(s as f64)]);
    }
    let auras: Vec<f64> = engine(vm)
        .units
        .auras
        .iter()
        .filter(|a| a.unit == crate::units::PLAYER && a.spell_id == id)
        .map(|a| a.instance)
        .collect();
    for a in auras {
        crate::auras::remove(vm, crate::units::PLAYER, a);
    }
    crate::widgets::fire_event(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    Ok(vec![])
}

/// The player's aura of `spell` gained or lost: a tracking spell's tells
/// `MINIMAP_UPDATE_TRACKING` (recorded: Find Minerals on and off, the
/// event among the frame's batched ones).
pub fn tracking_aura_changed(vm: &mut Vm, spell: f64) {
    if tracking_types()
        .iter()
        .any(|(_, kind, id)| *kind == "spell" && *id == spell)
    {
        crate::widgets::fire_event(vm, "MINIMAP_UPDATE_TRACKING", &[]);
    }
}

fn draw_ground_textures(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        !get_state(vm, "noGroundTextures").truthy(),
    )])
}

fn set_draw_ground_textures(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = if c.arg(1).truthy() {
        Value::Nil
    } else {
        Value::Bool(true)
    };
    set_state(vm, "noGroundTextures", v);
    Ok(vec![])
}

fn is_rotate_minimap_ignored(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        get_state(vm, "ignoreRotateMinimap").truthy(),
    )])
}

fn set_ignore_rotate_minimap(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = Value::Bool(c.arg(1).truthy());
    set_state(vm, "ignoreRotateMinimap", v);
    Ok(vec![])
}

// ----------------------------------------------------------- game pad

/// `C_GamePad.GetPowerLevel([device])`: unknown (5), no pad.
fn game_pad_power_level(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(5.0)])
}

/// `C_GamePad.GetLedColor()`: black (no pad).
fn game_pad_led_color(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = vm.new_table();
    for k in ["r", "g", "b"] {
        set_field(vm, t, k, Value::Num(0.0));
    }
    Ok(vec![Value::Table(t)])
}

// ---------------------------------------------------------------- TTS

/// `C_TTSSettings.GetCharacterSettingsSaved()`: once marked saved.
fn tts_character_settings_saved(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(get_state(vm, "ttsSaved").truthy())])
}

fn tts_mark_character_settings_saved(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_state(vm, "ttsSaved", Value::Bool(true));
    Ok(vec![])
}

// ---------------------------------------------------------- heirlooms

fn heirloom_filters_checked(vm: &mut Vm, key: &str) -> Ret {
    Ok(vec![Value::Bool(!get_state(vm, key).truthy())])
}

fn are_all_collection_filters_checked(vm: &mut Vm, _c: NativeCall) -> Ret {
    heirloom_filters_checked(vm, "heirloomCollectionUnchecked")
}

fn are_all_source_filters_checked(vm: &mut Vm, _c: NativeCall) -> Ret {
    heirloom_filters_checked(vm, "heirloomSourceUnchecked")
}

fn set_heirloom_filters(vm: &mut Vm, c: &NativeCall, key: &str) -> Ret {
    let v = if c.arg(1).truthy() {
        Value::Nil
    } else {
        Value::Bool(true)
    };
    set_state(vm, key, v);
    Ok(vec![])
}

fn set_all_collection_filters(vm: &mut Vm, c: NativeCall) -> Ret {
    set_heirloom_filters(vm, &c, "heirloomCollectionUnchecked")
}

fn set_all_source_filters(vm: &mut Vm, c: NativeCall) -> Ret {
    set_heirloom_filters(vm, &c, "heirloomSourceUnchecked")
}

/// `C_HeirloomInfo.SetDefaultFilters()`: every filter checked.
fn set_default_heirloom_filters(vm: &mut Vm, _c: NativeCall) -> Ret {
    set_state(vm, "heirloomCollectionUnchecked", Value::Nil);
    set_state(vm, "heirloomSourceUnchecked", Value::Nil);
    Ok(vec![])
}

// ------------------------------------------------------- combat log

/// `CombatLog_Object_IsA(flags, mask)`: the flags meet the mask in each
/// object group (affiliation, reaction, controller, type), or in the
/// special bits (Blizzard's former Lua `CombatLog_Object_IsA`; a C
/// function on camelot, unverified the same).
fn combat_log_object_is_a(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(flags), Some(mask)) = (num(vm, c.arg(1)), num(vm, c.arg(2))) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let (flags, mask) = (flags as i64 as u64, mask as i64 as u64);
    let groups = [0x0000_000F, 0x0000_00F0, 0x0000_0300, 0x0000_FC00];
    let special = 0xFFFF_0000u64;
    let is = groups.iter().all(|g| flags & mask & g != 0) || flags & mask & special != 0;
    Ok(vec![Value::Bool(is)])
}

/// `CombatLogGetRetentionTime()`: as set, 300 seconds at first
/// (unverified).
fn combat_log_get_retention_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    let v = get_state(vm, "combatLogRetention");
    Ok(vec![if v.is_nil() { Value::Num(300.0) } else { v }])
}

fn combat_log_set_retention_time(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(n) = num(vm, c.arg(1)) {
        set_state(vm, "combatLogRetention", Value::Num(n));
    }
    Ok(vec![])
}

// ------------------------------------------------------------ secrets

/// Whether the restrictions on auras, cooldowns and totems are active
/// (combat, an encounter, a challenge mode, a PvP match: the secrecy flags'
/// rule in units.rs).
fn restricted(vm: &Vm) -> bool {
    units::active(
        &engine(vm).units,
        &[
            units::COMBAT,
            units::ENCOUNTER,
            units::CHALLENGE_MODE,
            units::PVP_MATCH,
        ],
    )
}

/// `C_Secrets.Should…BeSecret`: while restricted (the documented
/// secrecy flags' rule; unverified each).
fn secret_while_restricted(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(restricted(vm))])
}

/// `C_Secrets.ShouldUnitThreatStateBeSecret(unit [, mobUnit])` and
/// `…ThreatValuesBeSecret(unit, mobUnit)`: secret, but for no mob and for a
/// group token's threat on `"target"` (the full probe, 1.60.1, out of
/// combat and unrestricted, no target: every other pair secret, `"none"`,
/// `"target"` and a GUID on `"target"` too; unverified: other states, a
/// target that exists).
fn threat_secret(vm: &mut Vm, c: NativeCall) -> Ret {
    let token = |v: Value| match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).to_ascii_lowercase()),
        _ => None,
    };
    let Some(mob) = token(c.arg(2)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let group = token(c.arg(1)).is_some_and(|u| {
        let base = u.trim_end_matches(|ch: char| ch.is_ascii_digit());
        matches!(
            base,
            "player" | "pet" | "party" | "raid" | "partypet" | "raidpet"
        )
    });
    Ok(vec![Value::Bool(!(mob == "target" && group))])
}

/// `C_Secrets.ShouldUnitSpellCastBeSecret(unit, spell)`: another unit's
/// casts, as `SecretWhenUnitSpellCastRestricted` answers.
fn unit_spell_cast_secret(vm: &mut Vm, c: NativeCall) -> Ret {
    let secret = units::unit_arg(vm, &c, 1) != Some(units::PLAYER);
    Ok(vec![Value::Bool(secret)])
}

/// Whether comparing two units is secret: identities restricted (a
/// restricted map, an encounter, a challenge mode, a PvP match) and one
/// of them not the player's or the group's (unverified).
fn comparison_secret(vm: &Vm, c: &NativeCall) -> bool {
    let u = &engine(vm).units;
    if !units::active(
        u,
        &[
            units::MAP,
            units::ENCOUNTER,
            units::CHALLENGE_MODE,
            units::PVP_MATCH,
        ],
    ) {
        return false;
    }
    let ours = |n: usize| {
        units::unit_arg(vm, c, n).is_some_and(|id| id == units::PLAYER || u.group.contains(&id))
    };
    !(ours(1) && ours(2))
}

fn unit_comparison_secret(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(comparison_secret(vm, &c))])
}

fn can_compare_unit_tokens(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(!comparison_secret(vm, &c))])
}

// ---------------------------------------------------------- utilities

/// `C_EventUtils.IsCallbackEvent(event)`: an event its documentation marks
/// `CallbackEvent` (12, the combat log's among them); another event no
/// (the full probe, 1.60.1: `PLAYER_LOGIN` false; the marked ones
/// unverified).
fn is_callback_event(vm: &mut Vm, c: NativeCall) -> Ret {
    let callback = text(vm, c.arg(1)).is_some_and(|e| {
        engine(vm)
            .api
            .event_docs
            .get(&e)
            .and_then(|d| d.get("CallbackEvent"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    });
    Ok(vec![Value::Bool(callback)])
}

/// `C_ColorUtil.WrapTextInColorCode(text, code)`: `|c<code>text|r`
/// (Blizzard's former Lua helper).
fn wrap_text_in_color_code(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = text(vm, c.arg(1)).unwrap_or_default();
    let code = text(vm, c.arg(2)).unwrap_or_default();
    Ok(vec![vm.str(&format!("|c{code}{t}|r"))])
}

fn three(vm: &Vm, c: &NativeCall) -> (f64, f64, f64) {
    (
        num(vm, c.arg(1)).unwrap_or(0.0),
        num(vm, c.arg(2)).unwrap_or(0.0),
        num(vm, c.arg(3)).unwrap_or(0.0),
    )
}

fn nums(values: [f64; 3]) -> Vec<Value> {
    values.into_iter().map(Value::Num).collect()
}

/// `C_Texture.GetFilenameFromFileDataID(id)`: "FileData ID <id>", the
/// client's name for a file it does not list (the full probe, 1.60.1:
/// -1, 0, 1, 1.5 as 1, 2147483647; unverified: a file it names).
fn filename_from_file_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let id = num(vm, c.arg(1)).unwrap_or(0.0).trunc() as i64;
    Ok(vec![vm.str(&format!("FileData ID {id}"))])
}

// The conversions clamp their inputs (hue to 0..360, the rest to 0..1)
// and compute in single precision (the full probe, 1.60.1: hue
// 2147483647 read as 360, -1 as 0, a saturation 1.5 as 1; HSV (1, 1, 1)
// gave green 0.016666650772095, a float's 1 - (1 - 1/60)).

/// The arguments clamped: a hue (0..360) or a channel (0..1) each.
fn clamped(vm: &Vm, c: &NativeCall, hue: bool) -> (f32, f32, f32) {
    let (a, b, d) = three(vm, c);
    let a = if hue {
        a.clamp(0.0, 360.0)
    } else {
        a.clamp(0.0, 1.0)
    };
    (a as f32, b.clamp(0.0, 1.0) as f32, d.clamp(0.0, 1.0) as f32)
}

fn nums32(v: [f32; 3]) -> Vec<Value> {
    nums(v.map(f64::from))
}

/// `C_ColorUtil.ConvertRGBToHSV(r, g, b)`: hue in degrees, -1 for a gray
/// (documented), saturation and value 0..1.
fn convert_rgb_to_hsv(vm: &mut Vm, c: NativeCall) -> Ret {
    let (r, g, b) = clamped(vm, &c, false);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let s = if max > 0.0 { delta / max } else { 0.0 };
    let h = if delta == 0.0 {
        -1.0
    } else {
        let h = if max == r {
            ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        };
        h * 60.0
    };
    Ok(nums32([h, s, max]))
}

/// `C_ColorUtil.ConvertHSVToRGB(h, s, v)`: the hue's sixth of the circle
/// (the last one past 360's float rounding: hue 360 gave blue
/// -4.7683715820312e-07).
fn convert_hsv_to_rgb(vm: &mut Vm, c: NativeCall) -> Ret {
    let (h, s, v) = clamped(vm, &c, true);
    let hh = h * (1.0f32 / 60.0);
    let i = hh.floor().min(5.0);
    let f = hh - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let rgb = match i as i64 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    };
    Ok(nums32(rgb))
}

/// `C_ColorUtil.ConvertHSVToHSL(h, s, v)`: the hue kept.
fn convert_hsv_to_hsl(vm: &mut Vm, c: NativeCall) -> Ret {
    let (h, s, v) = clamped(vm, &c, true);
    let l = v * (1.0 - s / 2.0);
    let sl = if l <= 0.0 || l >= 1.0 {
        0.0
    } else {
        (v - l) / l.min(1.0 - l)
    };
    Ok(nums32([h, sl, l]))
}

/// `C_ColorUtil.ConvertHSLToHSV(h, s, l)`: the hue kept.
fn convert_hsl_to_hsv(vm: &mut Vm, c: NativeCall) -> Ret {
    let (h, s, l) = clamped(vm, &c, true);
    let v = l + s * l.min(1.0 - l);
    let sv = if v <= 0.0 { 0.0 } else { 2.0 * (1.0 - l / v) };
    Ok(nums32([h, sv, v]))
}

// ----------------------------------------------------------- encoding

fn bytes_arg(vm: &Vm, c: &NativeCall, n: usize) -> Option<Vec<u8>> {
    match vm.hidden(c.arg(n)) {
        Value::Str(s) => Some(vm.str_bytes(s).to_vec()),
        Value::Num(x) => Some(wowlua_vm::number::fmt_num(x).into_bytes()),
        _ => None,
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// `C_EncodingUtil.EncodeBase64(source [, variant])`: padded, the
/// standard or URL-safe alphabet (`Enum.Base64Variant`; unverified the
/// URL-safe padding).
fn encode_base64(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(src) = bytes_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let alphabet = if num(vm, c.arg(2)) == Some(1.0) {
        BASE64_URL
    } else {
        BASE64
    };
    let mut out = Vec::with_capacity(src.len().div_ceil(3) * 4);
    for chunk in src.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(alphabet[(n >> 18) as usize & 63]);
        out.push(alphabet[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 {
            alphabet[(n >> 6) as usize & 63]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            alphabet[n as usize & 63]
        } else {
            b'='
        });
    }
    Ok(vec![vm.bytes(&out)])
}

/// `C_EncodingUtil.DecodeBase64(source [, variant])`: either alphabet,
/// padding optional, white space skipped; nothing for bad input.
fn decode_base64(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(src) = bytes_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let value = |ch: u8| -> Option<u32> {
        let v: u8 = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        Some(u32::from(v))
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for &ch in &src {
        if ch == b'=' {
            break;
        }
        if ch.is_ascii_whitespace() {
            continue;
        }
        let Some(v) = value(ch) else {
            return Ok(vec![]);
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(vec![vm.bytes(&out)])
}

/// `C_EncodingUtil.EncodeHex(source)`: lower-case digits (unverified the
/// case).
fn encode_hex(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(src) = bytes_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let hex: String = src.iter().map(|b| format!("{b:02x}")).collect();
    Ok(vec![vm.str(&hex)])
}

/// `C_EncodingUtil.DecodeHex(source)`: either case; an odd length errors
/// (the full probe, 1.60.1: `"x"`); nothing for other bad input
/// (unverified).
fn decode_hex(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(src) = bytes_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    if src.len() % 2 != 0 {
        return Err(encoding_error(
            vm,
            "DecodeHex(): attempted to decode a string that does not have an even number of bytes",
        ));
    }
    let digit = |ch: u8| (ch as char).to_digit(16);
    let mut out = Vec::with_capacity(src.len() / 2);
    for pair in src.chunks(2) {
        match (digit(pair[0]), digit(pair[1])) {
            (Some(a), Some(b)) => out.push((a * 16 + b) as u8),
            _ => return Ok(vec![]),
        }
    }
    Ok(vec![vm.bytes(&out)])
}

/// `C_EncodingUtil.CompressString(source [, method, level])`: raw
/// deflate, zlib or gzip (`Enum.CompressionMethod`), at the level asked
/// (`Enum.CompressionLevel`; unverified the client's exact bytes).
fn compress_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(src) = bytes_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let level = match num(vm, c.arg(3)).map(|n| n as i64) {
        Some(1) => flate2::Compression::fast(),
        Some(2) => flate2::Compression::best(),
        _ => flate2::Compression::default(),
    };
    let out = match num(vm, c.arg(2)).map(|n| n as i64) {
        Some(1) => {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), level);
            e.write_all(&src).and_then(|()| e.finish())
        }
        Some(2) => {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), level);
            e.write_all(&src).and_then(|()| e.finish())
        }
        _ => {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), level);
            e.write_all(&src).and_then(|()| e.finish())
        }
    };
    Ok(match out {
        Ok(bytes) => vec![vm.bytes(&bytes)],
        Err(_) => vec![],
    })
}

/// `C_EncodingUtil.DecompressString(source [, method])`: nothing for
/// data that does not decompress.
fn decompress_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(src) = bytes_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    let r = match num(vm, c.arg(2)).map(|n| n as i64) {
        Some(1) => flate2::read::ZlibDecoder::new(&src[..]).read_to_end(&mut out),
        Some(2) => flate2::read::GzDecoder::new(&src[..]).read_to_end(&mut out),
        _ => flate2::read::DeflateDecoder::new(&src[..]).read_to_end(&mut out),
    };
    Ok(match r {
        Ok(_) => vec![vm.bytes(&out)],
        Err(_) => vec![],
    })
}

/// Whether a table is a list: its keys exactly 1..n.
fn list_len(vm: &Vm, t: TableRef) -> Option<usize> {
    let n = vm.heap.table(t).border();
    let count = vm.heap.table(t).entries().count();
    (count == n).then_some(n)
}

fn cbor_head(major: u8, n: u64, out: &mut Vec<u8>) {
    let m = major << 5;
    if n < 24 {
        out.push(m | n as u8);
    } else if n <= 0xff {
        out.push(m | 24);
        out.push(n as u8);
    } else if n <= 0xffff {
        out.push(m | 25);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else if n <= 0xffff_ffff {
        out.push(m | 26);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

/// A Lua value as CBOR: integers as integers, other numbers as doubles,
/// strings as byte strings, lists as arrays (an empty
/// table one too: recorded `\x80`), other tables as maps (unverified: the
/// client's float widths).
fn cbor_encode(
    vm: &Vm,
    v: Value,
    out: &mut Vec<u8>,
    depth: usize,
    lenient: bool,
) -> Result<(), String> {
    if depth > 100 {
        return Err("table nesting too deep".into());
    }
    match vm.hidden(v) {
        Value::Nil => out.push(0xf6),
        Value::Bool(false) => out.push(0xf4),
        Value::Bool(true) => out.push(0xf5),
        Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.2e18 => {
            if n >= 0.0 {
                cbor_head(0, n as u64, out);
            } else {
                cbor_head(1, (-1 - n as i64) as u64, out);
            }
        }
        Value::Num(n) => {
            out.push(0xfb);
            out.extend_from_slice(&n.to_bits().to_be_bytes());
        }
        // A string is a byte string, UTF-8 or not (the full probe,
        // 1.60.1: "x" as `Ax`, 0x41).
        Value::Str(s) => {
            let b = vm.str_bytes(s);
            cbor_head(2, b.len() as u64, out);
            out.extend_from_slice(b);
        }
        Value::Table(t) => {
            if let Some(n) = list_len(vm, t) {
                cbor_head(4, n as u64, out);
                for i in 1..=n {
                    let item = vm.heap.table(t).get(Value::Num(i as f64));
                    cbor_encode(vm, item, out, depth + 1, lenient)?;
                }
            } else {
                let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
                cbor_head(5, entries.len() as u64, out);
                for (k, item) in entries {
                    cbor_encode(vm, k, out, depth + 1, lenient)?;
                    cbor_encode(vm, item, out, depth + 1, lenient)?;
                }
            }
        }
        _ if lenient => out.push(0xf6),
        other => return Err(format!("cannot serialize a {}", other.type_name())),
    }
    Ok(())
}

/// The `ignoreSerializationErrors` option.
fn lenient(vm: &mut Vm, options: Value) -> bool {
    match options {
        Value::Table(t) => get_field(vm, t, "ignoreSerializationErrors").truthy(),
        _ => false,
    }
}

/// The encodings' own error: the C code's message, with the taint line
/// the client adds for an addon's call (as `DeserializeCBOR`'s).
fn encoding_error(vm: &mut Vm, msg: &str) -> wowlua_vm::LuaError {
    let mut msg = msg.to_string();
    if !crate::protection::blizzard_caller(vm) {
        msg.push_str("\nLua Taint: *** ForceTaint_Strong ***");
    }
    let v = vm.str(&msg);
    vm.raise(v)
}

/// The serializers' options: a table or nil, anything else refused with
/// the C code's usage (`SerializeJSON({}, "x")`, the full probe, 1.60.1;
/// other non-tables unverified).
fn check_options(vm: &mut Vm, c: &NativeCall, usage: &str) -> Result<(), wowlua_vm::LuaError> {
    match vm.hidden(c.arg(2)) {
        Value::Nil | Value::Table(_) => Ok(()),
        _ => Err(encoding_error(vm, usage)),
    }
}

/// `C_EncodingUtil.SerializeCBOR(value [, options])`.
fn serialize_cbor(vm: &mut Vm, c: NativeCall) -> Ret {
    check_options(
        vm,
        &c,
        "Usage: C_EncodingUtil.SerializeCBOR(value[, options])",
    )?;
    let lenient = lenient(vm, c.arg(2));
    let mut out = Vec::new();
    if let Err(e) = cbor_encode(vm, c.arg(1), &mut out, 0, lenient) {
        return Err(vm.error(&e));
    }
    Ok(vec![vm.bytes(&out)])
}

struct Cbor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cbor<'_> {
    fn byte(&mut self) -> Option<u8> {
        let v = *self.b.get(self.i)?;
        self.i += 1;
        Some(v)
    }

    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.i.checked_add(n)?;
        let s = self.b.get(self.i..end)?;
        self.i = end;
        Some(s)
    }

    /// An item's argument; `None` (inside `Some`) for an indefinite length.
    fn arg(&mut self, info: u8) -> Option<Option<u64>> {
        Some(Some(match info {
            0..=23 => u64::from(info),
            24 => u64::from(self.byte()?),
            25 => u64::from(u16::from_be_bytes(self.take(2)?.try_into().ok()?)),
            26 => u64::from(u32::from_be_bytes(self.take(4)?.try_into().ok()?)),
            27 => u64::from_be_bytes(self.take(8)?.try_into().ok()?),
            31 => return Some(None),
            _ => return None,
        }))
    }

    fn at_break(&mut self) -> bool {
        if self.b.get(self.i) == Some(&0xff) {
            self.i += 1;
            true
        } else {
            false
        }
    }
}

fn half_to_f64(h: u16) -> f64 {
    let exp = (h >> 10) & 0x1f;
    let mant = f64::from(h & 0x3ff);
    let v = match exp {
        0 => mant * 2f64.powi(-24),
        31 if mant == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        e => (mant + 1024.0) * 2f64.powi(i32::from(e) - 25),
    };
    if h & 0x8000 != 0 { -v } else { v }
}

/// One CBOR item as a Lua value (tags skipped).
fn cbor_decode(vm: &mut Vm, d: &mut Cbor<'_>, depth: usize) -> Option<Value> {
    if depth > 100 {
        return None;
    }
    let first = d.byte()?;
    let (major, info) = (first >> 5, first & 31);
    match major {
        0 => Some(Value::Num(d.arg(info)?? as f64)),
        1 => Some(Value::Num(-1.0 - d.arg(info)?? as f64)),
        2 | 3 => {
            let bytes = match d.arg(info)? {
                Some(n) => d.take(usize::try_from(n).ok()?)?.to_vec(),
                None => {
                    let mut all = Vec::new();
                    while !d.at_break() {
                        let chunk = d.byte()?;
                        let n = d.arg(chunk & 31)??;
                        all.extend_from_slice(d.take(usize::try_from(n).ok()?)?);
                    }
                    all
                }
            };
            Some(vm.bytes(&bytes))
        }
        4 => {
            let t = vm.new_table();
            let n = d.arg(info)?;
            let mut i = 0u64;
            loop {
                match n {
                    Some(n) if i >= n => break,
                    None if d.at_break() => break,
                    _ => {}
                }
                let item = cbor_decode(vm, d, depth + 1)?;
                i += 1;
                let _ = vm.heap.table_mut(t).set(Value::Num(i as f64), item);
            }
            Some(Value::Table(t))
        }
        5 => {
            let t = vm.new_table();
            let n = d.arg(info)?;
            let mut i = 0u64;
            loop {
                match n {
                    Some(n) if i >= n => break,
                    None if d.at_break() => break,
                    _ => {}
                }
                let k = cbor_decode(vm, d, depth + 1)?;
                let v = cbor_decode(vm, d, depth + 1)?;
                i += 1;
                if !k.is_nil() {
                    let _ = vm.heap.table_mut(t).set(k, v);
                }
            }
            Some(Value::Table(t))
        }
        6 => {
            d.arg(info)?;
            cbor_decode(vm, d, depth + 1)
        }
        _ => match info {
            20 => Some(Value::Bool(false)),
            21 => Some(Value::Bool(true)),
            22 | 23 => Some(Value::Nil),
            25 => {
                let h = u16::from_be_bytes(d.take(2)?.try_into().ok()?);
                Some(Value::Num(half_to_f64(h)))
            }
            26 => {
                let f = f32::from_be_bytes(d.take(4)?.try_into().ok()?);
                Some(Value::Num(f64::from(f)))
            }
            27 => {
                let f = f64::from_be_bytes(d.take(8)?.try_into().ok()?);
                Some(Value::Num(f))
            }
            _ => None,
        },
    }
}

/// `C_EncodingUtil.DeserializeCBOR(source)`: the value; bad input errors
/// as the client's C code does, with the taint of the addon code that
/// called it (recorded for "zz": "attempted to deserialize an incomplete
/// or ill-formed data item" and a "Lua Taint: *** ForceTaint_Strong ***"
/// line, as `SetMinMaxValues`' error; unverified: trailing bytes after a
/// whole item, accepted here).
fn deserialize_cbor(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(src) = bytes_arg(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    // No bytes: nothing (the full probe, 1.60.1; "x" fails).
    if src.is_empty() {
        return Ok(vec![]);
    }
    let mut d = Cbor { b: &src, i: 0 };
    match cbor_decode(vm, &mut d, 0) {
        Some(v) => Ok(vec![v]),
        None => {
            let mut msg =
                String::from("attempted to deserialize an incomplete or ill-formed data item");
            if !crate::protection::blizzard_caller(vm) {
                msg.push_str("\nLua Taint: *** ForceTaint_Strong ***");
            }
            let v = vm.str(&msg);
            Err(vm.raise(v))
        }
    }
}

/// A Lua value as JSON: lists as arrays (an empty table one too: recorded
/// `[]`), other tables as objects (keys as strings), integers without a
/// fraction (unverified: the key order).
fn to_json(vm: &Vm, v: Value, depth: usize, lenient: bool) -> Result<serde_json::Value, String> {
    use serde_json::Value as J;
    if depth > 100 {
        return Err("table nesting too deep".into());
    }
    Ok(match vm.hidden(v) {
        Value::Nil => J::Null,
        Value::Bool(b) => J::Bool(b),
        Value::Num(n) if n.fract() == 0.0 && n.abs() < 9.2e18 => J::from(n as i64),
        Value::Num(n) => match serde_json::Number::from_f64(n) {
            Some(n) => J::Number(n),
            None if lenient => J::Null,
            None => return Err("cannot serialize a non-finite number".into()),
        },
        Value::Str(s) => J::String(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        Value::Table(t) => {
            if let Some(n) = list_len(vm, t) {
                let mut items = Vec::with_capacity(n);
                for i in 1..=n {
                    let item = vm.heap.table(t).get(Value::Num(i as f64));
                    items.push(to_json(vm, item, depth + 1, lenient)?);
                }
                J::Array(items)
            } else {
                let mut map = serde_json::Map::new();
                let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
                for (k, item) in entries {
                    let key = match vm.hidden(k) {
                        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
                        Value::Num(n) => wowlua_vm::number::fmt_num(n),
                        Value::Bool(b) => b.to_string(),
                        _ if lenient => continue,
                        other => {
                            return Err(format!("cannot serialize a {} key", other.type_name()));
                        }
                    };
                    map.insert(key, to_json(vm, item, depth + 1, lenient)?);
                }
                J::Object(map)
            }
        }
        _ if lenient => J::Null,
        other => return Err(format!("cannot serialize a {}", other.type_name())),
    })
}

fn from_json(vm: &mut Vm, j: &serde_json::Value) -> Value {
    use serde_json::Value as J;
    match j {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => Value::Num(n.as_f64().unwrap_or(0.0)),
        J::String(s) => vm.str(s),
        J::Array(items) => {
            let t = vm.new_table();
            for (i, item) in items.iter().enumerate() {
                let v = from_json(vm, item);
                let _ = vm.heap.table_mut(t).set(Value::Num((i + 1) as f64), v);
            }
            Value::Table(t)
        }
        J::Object(map) => {
            let t = vm.new_table();
            for (k, item) in map {
                let key = vm.str(k);
                let v = from_json(vm, item);
                let _ = vm.heap.table_mut(t).set(key, v);
            }
            Value::Table(t)
        }
    }
}

/// `C_EncodingUtil.SerializeJSON(value [, options])`.
fn serialize_json(vm: &mut Vm, c: NativeCall) -> Ret {
    check_options(
        vm,
        &c,
        "Usage: C_EncodingUtil.SerializeJSON(value[, options])",
    )?;
    let lenient = lenient(vm, c.arg(2));
    match to_json(vm, c.arg(1), 0, lenient) {
        Ok(j) => Ok(vec![vm.str(&j.to_string())]),
        Err(e) => Err(vm.error(&e)),
    }
}

/// `C_EncodingUtil.DeserializeJSON(source)`: the value; bad input errors
/// (the full probe, 1.60.1: `""` and `"x"` "attempted to deserialize an
/// unknown json value"; other malformed input's message unverified).
fn deserialize_json(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(src) = bytes_arg(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    match serde_json::from_slice::<serde_json::Value>(&src) {
        Ok(j) => Ok(vec![from_json(vm, &j)]),
        Err(_) => Err(encoding_error(
            vm,
            "attempted to deserialize an unknown json value",
        )),
    }
}

/// `Logout()` (Blizzard's `/camp`; protected): the client starts camping,
/// `PLAYER_CAMPING` told in the same second (SuuUI probe popups, five
/// samples: Blizzard's CAMP popup shown with it). Unverified: the exact
/// delay (none here), and the logout itself after the countdown (never
/// reached in the engine).
fn logout(vm: &mut Vm, _c: NativeCall) -> Ret {
    widgets::fire_event(vm, "PLAYER_CAMPING", &[]);
    Ok(vec![])
}

// ------------------------------------------------ preferred play settings

/// How long the account server takes to answer a request for the
/// preferred play settings (unverified: SuuUI probe lfg had the answer
/// within six seconds, its time not kept).
const PREFERRED_PLAY_SETTINGS_AFTER: f64 = 0.2;

/// `C_AccountServices.RequestPreferredPlaySettings()`: started (SuuUI
/// probe lfg: `true`), the settings arrive as
/// `PREFERRED_PLAY_SETTINGS_UPDATED` (the probe's frame got it). Its
/// payload as the login recording has it (`1; 1; 1; 0; 0`): a status
/// (1, unverified what others mean), then the locale, the data center and
/// their change dates `GetPreferredPlaySettings` answers. Before the login
/// is done the login's recorded answer is the one told.
fn request_preferred_play_settings(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine(vm).world.logged_in {
        let answer = vm.native("PreferredPlaySettingsAnswer", |vm, _c| {
            let settings =
                crate::recorded::lookup_text(vm, "C_AccountServices.GetPreferredPlaySettings()")
                    .unwrap_or_default();
            let mut args = vec![Value::Num(1.0)];
            for i in 1..5 {
                args.push(settings.get(i).copied().unwrap_or(Value::Num(0.0)));
            }
            widgets::fire_event(vm, "PREFERRED_PLAY_SETTINGS_UPDATED", &args);
            Ok(vec![])
        });
        crate::clock::after_engine(vm, PREFERRED_PLAY_SETTINGS_AFTER, answer);
    }
    Ok(vec![Value::Bool(true)])
}

// ------------------------------------------------------- registration

/// `IsWindowsClient()`, `IsMacClient()`: the test's platform (decision
/// T236; recorded: the Mac's `IsMacClient` true, Windows' false).
fn is_windows_client(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        crate::engine(vm).platform == crate::Platform::Windows,
    )])
}

fn is_mac_client(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(
        crate::engine(vm).platform == crate::Platform::Mac,
    )])
}
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        // Math (C functions on camelot).
        ("Round", round),
        ("Clamp", clamp),
        ("Lerp", lerp),
        ("Saturate", saturate),
        ("RoundToSignificantDigits", round_to_significant_digits),
        // Sound: the engine plays none.
        ("PlaySound", play_sound),
        ("PlaySoundFile", play_sound),
        ("C_Sound.PlaySoundWithOptions", play_sound_with_options),
        ("StopSound", nothing),
        ("MuteSoundFile", nothing),
        ("UnmuteSoundFile", nothing),
        ("PlayMusic", nothing),
        ("StopMusic", nothing),
        ("C_Sound.PlayItemSound", nothing),
        ("C_Sound.PlayVocalErrorSound", nothing),
        ("C_Sound.IsPlaying", no),
        ("C_Sound.GetSoundScaledVolume", zero),
        ("Sound_ChatSystem_GetNumInputDrivers", zero),
        ("Sound_ChatSystem_GetNumOutputDrivers", zero),
        ("Sound_ChatSystem_GetInputDriverNameByIndex", nil),
        ("Sound_ChatSystem_GetOutputDriverNameByIndex", nil),
        ("Sound_GameSystem_RestartSoundSystem", nothing),
        // Mouse and keys: no button held between the engine's actions.
        ("IsMouseButtonDown", no),
        ("IsKeyDown", is_key_down),
        ("IsMouselooking", is_mouselooking),
        ("MouselookStart", mouselook_start),
        ("MouselookStop", mouselook_stop),
        // The cursor's look: nothing to show.
        ("SetCursor", yes),
        ("SetCursorByMode", yes),
        ("ResetCursor", nothing),
        ("SetCursorPosition", nothing),
        ("SetCursorHoveredItem", nothing),
        ("SetCursorHoveredItemTradeItem", nothing),
        ("SetCursorVirtualItem", nothing),
        ("C_Cursor.GetCursorItem", get_cursor_item),
        // The screen.
        ("GetScreenWidth", get_screen_width),
        ("GetScreenDPIScale", get_screen_dpi_scale),
        // Recorded: no notch, zeros.
        ("C_UI.GetTopLeftNotchSafeRegion", four_zeros),
        ("C_UI.GetTopRightNotchSafeRegion", four_zeros),
        ("C_UI.GetUIParent", get_ui_parent),
        ("C_UI.GetWorldFrame", get_world_frame),
        ("C_VideoOptions.SetGameWindowSize", nothing),
        ("SetUIVisibility", nothing),
        ("Screenshot", nothing),
        // Portraits: nothing drawn.
        ("SetPortraitTexture", nothing),
        ("SetPortraitTextureFromCreatureDisplayID", nothing),
        // Frame rate and clocks.
        ("GetFramerate", get_framerate),
        ("GetServerTime", get_server_time),
        ("GetSessionTime", get_session_time),
        ("C_DateAndTime.GetServerTimeLocal", get_server_time_local),
        (
            "C_DateAndTime.GetSecondsUntilDailyReset",
            seconds_until_daily_reset,
        ),
        (
            "C_DateAndTime.GetSecondsUntilWeeklyReset",
            seconds_until_weekly_reset,
        ),
        (
            "C_DateAndTime.GetWeeklyResetStartTime",
            weekly_reset_start_time,
        ),
        ("C_DateAndTime.AdjustTimeByDays", adjust_time_by_days),
        ("C_DateAndTime.AdjustTimeByMinutes", adjust_time_by_minutes),
        ("C_DateAndTime.AdjustTimeByMonths", adjust_time_by_months),
        ("C_DateAndTime.CompareCalendarTime", compare_calendar_time),
        (
            "C_DateAndTime.GetCalendarTimeFromEpoch",
            calendar_time_from_epoch,
        ),
        // The client's build: on the test's platform (decision T236), public.
        ("IsWindowsClient", is_windows_client),
        ("IsMacClient", is_mac_client),
        ("IsLinuxClient", no),
        ("IsBetaBuild", no),
        ("IsPublicBuild", yes),
        ("GetClientDisplayExpansionLevel", expansion_zero),
        ("GetAccountExpansionLevel", expansion_zero),
        ("GetExpansionForLevel", expansion_zero),
        ("GetExpansionDisplayInfo", nil),
        ("C_Glue.IsFirstLoadThisSession", no),
        // Leaving the game: nothing happens in the engine but the camping.
        ("Logout", logout),
        ("CancelLogout", nothing),
        ("Quit", nothing),
        ("ForceQuit", nothing),
        // Account services: no account save.
        ("C_AccountServices.IsAccountLockedPostSave", no),
        ("C_AccountServices.IsAccountSaveEnabled", no),
        ("C_AccountServices.IsAccountSaveInProgress", no),
        ("C_AccountServices.SetPreferredPlaySettings", nothing),
        (
            "C_AccountServices.RequestPreferredPlaySettings",
            request_preferred_play_settings,
        ),
        ("C_AccountServices.SaveAccountData", nothing),
        // Add-ons.
        ("C_AddOns.DoesAddOnExist", does_addon_exist),
        ("C_AddOns.GetAddOnEnableState", get_addon_enable_state),
        ("C_AddOns.EnableAddOn", enable_addon),
        ("C_AddOns.DisableAddOn", disable_addon),
        ("C_AddOns.EnableAllAddOns", enable_all_addons),
        ("C_AddOns.DisableAllAddOns", disable_all_addons),
        ("C_AddOns.ResetAddOns", enable_all_addons),
        ("C_AddOns.ResetDisabledAddOns", enable_all_addons),
        ("C_AddOns.SaveAddOns", nothing),
        ("C_AddOns.SetAddonVersionCheck", nothing),
        ("C_AddOns.GetScriptsDisallowedForBeta", no),
        ("C_AddOns.GetAddOnName", get_addon_name),
        ("C_AddOns.GetAddOnTitle", get_addon_title),
        ("C_AddOns.GetAddOnNotes", get_addon_notes),
        ("C_AddOns.GetAddOnDependencies", get_addon_dependencies),
        (
            "C_AddOns.GetAddOnOptionalDependencies",
            get_addon_optional_dependencies,
        ),
        ("C_AddOns.GetAddOnSecurity", get_addon_security),
        (
            "C_AddOns.GetAddOnInterfaceVersion",
            get_addon_interface_version,
        ),
        ("C_AddOns.IsAddOnLoadable", is_addon_loadable),
        ("C_AddOns.IsAddOnDefaultEnabled", is_addon_default_enabled),
        ("C_AddOns.DoesAddOnHaveLoadError", no),
        ("C_AddOns.GetAddOnLocalTable", nil),
        // Bindings.
        ("SaveBindings", save_bindings),
        ("LoadBindings", load_bindings),
        ("SetBindingClick", set_binding_click),
        ("SetBindingSpell", set_binding_spell),
        ("SetBindingItem", set_binding_item),
        ("SetBindingMacro", set_binding_macro),
        (
            "C_KeyBindings.ActivateBindingContext",
            activate_binding_context,
        ),
        (
            "C_KeyBindings.DeactivateBindingContext",
            deactivate_binding_context,
        ),
        (
            "C_KeyBindings.IsBindingContextActive",
            is_binding_context_active,
        ),
        ("C_KeyBindings.GetTurnStrafeStyle", get_turn_strafe_style),
        ("C_KeyBindings.SetTurnStrafeStyle", set_turn_strafe_style),
        (
            "C_KeyBindings.UpdateTurnStrafeBindingsForCharacter",
            nothing,
        ),
        // Combat ratings.
        ("GetHaste", get_haste),
        ("GetMastery", get_mastery),
        ("GetExpertisePercent", get_expertise_percent),
        ("GetParryChanceFromAttribute", zero),
        ("GetCritChanceProvidesParryEffect", no),
        (
            "GetCombatRatingBonusForCombatRatingValue",
            combat_rating_bonus_for_value,
        ),
        ("GetPowerRegenForPowerType", get_power_regen_for_power_type),
        // Raid marks (unverified: no mark kept).
        ("SetRaidTarget", nothing),
        ("CanBeRaidTarget", can_be_raid_target),
        // Currencies.
        ("C_CurrencyInfo.GetCurrencyInfo", get_currency_info),
        (
            "C_CurrencyInfo.GetCurrencyInfoFromLink",
            get_currency_info_from_link,
        ),
        (
            "C_CurrencyInfo.GetCurrencyIDFromLink",
            get_currency_id_from_link,
        ),
        ("C_CurrencyInfo.GetCurrencyLink", get_currency_link),
        ("C_CurrencyInfo.GetCurrencyListLink", get_currency_list_link),
        (
            "C_CurrencyInfo.GetBasicCurrencyInfo",
            get_basic_currency_info,
        ),
        (
            "C_CurrencyInfo.GetCurrencyDescription",
            get_currency_description,
        ),
        (
            "C_CurrencyInfo.PlayerHasMaxQuantity",
            player_has_max_quantity,
        ),
        (
            "C_CurrencyInfo.PlayerHasMaxWeeklyQuantity",
            player_has_max_weekly_quantity,
        ),
        (
            "C_CurrencyInfo.IsAccountWideCurrency",
            is_account_wide_currency,
        ),
        (
            "C_CurrencyInfo.IsAccountTransferableCurrency",
            is_account_transferable_currency,
        ),
        ("C_CurrencyInfo.CanTransferCurrency", can_transfer_currency),
        (
            "C_CurrencyInfo.DoesWarModeBonusApply",
            does_war_mode_bonus_apply,
        ),
        ("C_CurrencyInfo.GetAzeriteCurrencyID", azerite_currency_id),
        (
            "C_CurrencyInfo.GetWarResourcesCurrencyID",
            war_resources_currency_id,
        ),
        (
            "C_CurrencyInfo.GetDragonIslesSuppliesCurrencyID",
            dragon_isles_supplies_currency_id,
        ),
        (
            "C_CurrencyInfo.GetPlayerCurrencyCategoryInfo",
            get_player_currency_category_info,
        ),
        ("C_CurrencyInfo.GetCoinIcon", get_coin_icon),
        ("C_CurrencyInfo.GetCoinText", get_coin_text),
        ("C_CurrencyInfo.ExpandCurrencyList", nothing),
        ("C_CurrencyInfo.IsCurrencyContainer", no),
        ("C_CurrencyInfo.GetCurrencyContainerInfo", nothing),
        ("C_CurrencyInfo.GetFactionGrantedByCurrency", nil),
        // Ready (the full probe, after login).
        ("C_CurrencyInfo.IsAccountCharacterCurrencyDataReady", yes),
        (
            "C_CurrencyInfo.RequestCurrencyDataForAccountCharacters",
            nothing,
        ),
        (
            "C_CurrencyInfo.FetchCurrencyDataFromAccountCharacters",
            empty_table,
        ),
        (
            "C_CurrencyInfo.FetchCurrencyTransferTransactions",
            empty_table,
        ),
        ("C_CurrencyInfo.IsCurrencyTransferInProgress", no),
        ("C_CurrencyInfo.IsCurrencyTransferTransactionDataReady", yes),
        ("C_CurrencyInfo.GetCostToTransferCurrency", nil),
        ("C_CurrencyInfo.GetMaxTransferableAmountFromQuantity", nil),
        (
            "C_CurrencyInfo.RequestCurrencyFromAccountCharacter",
            nothing,
        ),
        ("C_CurrencyInfo.SetCurrencyUnused", nothing),
        ("C_CurrencyInfo.SetCurrencyBackpack", nothing),
        ("C_CurrencyInfo.SetCurrencyBackpackByID", nothing),
        ("C_CurrencyInfo.GetCurrencyFilter", zero),
        ("C_CurrencyInfo.SetCurrencyFilter", nothing),
        ("C_CurrencyInfo.PickupCurrency", nothing),
        // Reputation.
        ("C_Reputation.GetFactionDataByID", get_faction_data_by_id),
        // Recorded: nil (no guild reputation on Forever).
        ("C_Reputation.GetGuildFactionData", nil),
        // No guild reputation to expire: 0 (the full probe).
        ("C_Reputation.GetGuildRepExpirationTime", zero),
        ("C_Reputation.GetFactionParagonInfo", nothing),
        ("C_Reputation.IsFactionParagon", no),
        (
            "C_Reputation.RequestFactionParagonPreloadRewardData",
            nothing,
        ),
        ("C_Reputation.SetSelectedFaction", set_selected_faction),
        (
            "C_Reputation.SetWatchedFactionByIndex",
            set_watched_faction_by_index,
        ),
        (
            "C_Reputation.SetWatchedFactionByID",
            set_watched_faction_by_id,
        ),
        ("C_Reputation.ToggleFactionAtWar", toggle_faction_at_war),
        ("C_Reputation.IsFactionActive", is_faction_active),
        ("C_Reputation.SetFactionActive", set_faction_active),
        // Headers: the engine's list is the recorded one, flat
        // (unverified: collapsing hides the children).
        ("C_Reputation.CollapseFactionHeader", nothing),
        ("C_Reputation.ExpandFactionHeader", nothing),
        ("C_Reputation.CollapseAllFactionHeaders", nothing),
        ("C_Reputation.ExpandAllFactionHeaders", nothing),
        (
            "C_Reputation.AreLegacyReputationsShown",
            are_legacy_reputations_shown,
        ),
        (
            "C_Reputation.SetLegacyReputationsShown",
            set_legacy_reputations_shown,
        ),
        (
            "C_Reputation.GetReputationSortType",
            get_reputation_sort_type,
        ),
        (
            "C_Reputation.SetReputationSortType",
            set_reputation_sort_type,
        ),
        // Renown: none on Forever.
        ("C_MajorFactions.HasMaximumRenown", no),
        ("C_MajorFactions.GetMajorFactionProgressionInfo", nil),
        ("C_MajorFactions.GetMajorFactionRenownInfo", nil),
        ("C_MajorFactions.GetTotalReputationForRenownLevel", zero),
        ("C_MajorFactions.ShouldUseJourneyRewardTrack", no),
        ("C_MajorFactions.ShouldDisplayMajorFactionAsJourney", no),
        ("C_MajorFactions.GetMajorFactionIDs", empty_table),
        ("C_MajorFactions.GetRenownNPCFactionID", zero),
        ("C_MajorFactions.IsWeeklyRenownCapped", no),
        // CVars.
        ("C_CVar.SetCVarBitfield", set_cvar_bitfield),
        ("C_CVar.SetTempCVar", set_temp_cvar),
        ("C_CVar.RemoveTempCVar", remove_temp_cvar),
        ("C_CVar.ResetTestCVars", nothing),
        // Game rules and modes: Forever's one mode.
        ("C_GameRules.GetCurrentEventRealmQueues", zero),
        ("C_GameRules.GetGameModeGlueScreenName", empty_string),
        ("C_GameRules.GetCurrentGameModeRecordID", zero),
        ("C_GameRules.DoesGameModeHavePromo", no),
        ("C_GameRules.GetNumDisplayedGameModes", zero),
        ("C_GameRules.GetDisplayedGameModeRecordIDAtIndex", zero),
        ("C_GameRules.GetGameModeDisplayInfoByRecordID", nil),
        ("C_GameRules.GetCurrentGameModeDisplayInfo", nil),
        ("C_GameRules.GetGameModePromoGlobalString", empty_string),
        ("C_GameRules.IsCharacterlessLoginActive", no),
        ("C_GameRules.IsGameModeEnabled", no),
        ("C_GameRules.IsSelfFoundAllowed", no),
        ("C_GameRules.IsWoWHack", no),
        (
            "C_GameRules.IsPersonalResourceDisplayEnabled",
            is_personal_resource_display_enabled,
        ),
        (
            "C_GameRules.IsClassAllowedForGameMode",
            is_class_allowed_for_game_mode,
        ),
        ("C_GameRules.SetSDHDToggleValue", nothing),
        ("C_GameRules.SetForeverExperiencePreset", nothing),
        ("C_GameRules.AutoConnectToGameModeRealm", nothing),
        // Minimap.
        ("C_Minimap.GetViewRadius", get_view_radius),
        // Recorded 25 for the probed character; their infos were not
        // recorded, so the engine lists none (noted).
        ("C_Minimap.GetNumTrackingTypes", get_num_tracking_types),
        ("C_Minimap.GetTrackingInfo", get_tracking_info),
        ("C_Minimap.GetTrackingFilter", get_tracking_filter),
        ("C_Minimap.SetTracking", set_tracking),
        ("C_Minimap.ClearAllTracking", nothing),
        // Recorded: true.
        ("C_Minimap.CanTrackBattlePets", yes),
        // Recorded: false (filter 1024).
        ("C_Minimap.GetDefaultTrackingValue", no),
        ("C_Minimap.IsTrackingBattlePets", no),
        ("C_Minimap.IsTrackingHiddenQuests", no),
        (
            "C_Minimap.IsTrackingAccountCompletedQuests",
            tracking_account_completed_quests,
        ),
        ("C_Minimap.IsFilteredOut", is_filtered_out),
        ("C_Minimap.GetNumQuestPOIWorldEffects", zero),
        ("C_Minimap.GetPOITextureCoords", four_zeros),
        ("C_Minimap.GetUiMapID", get_minimap_ui_map_id),
        ("C_Minimap.IsInsideQuestBlob", no),
        ("C_Minimap.SetMinimapInsetInfo", nothing),
        ("C_Minimap.ClearMinimapInsetInfo", nothing),
        ("C_Minimap.GetDrawGroundTextures", draw_ground_textures),
        ("C_Minimap.SetDrawGroundTextures", set_draw_ground_textures),
        (
            "C_Minimap.IsRotateMinimapIgnored",
            is_rotate_minimap_ignored,
        ),
        (
            "C_Minimap.SetIgnoreRotateMinimap",
            set_ignore_rotate_minimap,
        ),
        // Model scenes: no model data (documented: the active scene calls
        // do nothing in public clients).
        ("C_ModelInfo.GetModelSceneInfoByID", nothing),
        ("C_ModelInfo.GetModelSceneActorInfoByID", nothing),
        ("C_ModelInfo.GetModelSceneCameraInfoByID", nothing),
        ("C_ModelInfo.GetModelSceneActorDisplayInfoByID", nothing),
        ("C_ModelInfo.AddActiveModelScene", nothing),
        ("C_ModelInfo.AddActiveModelSceneActor", nothing),
        ("C_ModelInfo.ClearActiveModelScene", nothing),
        ("C_ModelInfo.ClearActiveModelSceneActor", nothing),
        // Textures the engine has no data for.
        ("C_Texture.GetTitleIconTexture", nothing),
        ("C_Texture.IsTitleIconTextureReady", no),
        ("C_Texture.SetTitleIconTexture", nothing),
        ("C_Texture.ClearTitleIconTexture", nothing),
        ("C_Texture.SetURLTexture", nothing),
        ("C_Texture.GetAtlasElementID", zero),
        ("C_Texture.GetAtlasID", zero),
        ("C_Texture.GetAtlasElements", empty_table),
        ("C_Texture.GetFilenameFromFileDataID", filename_from_file_id),
        // Game pads: none connected.
        ("C_GamePad.IsEnabled", no),
        ("C_GamePad.GetAllDeviceIDs", empty_table),
        ("C_GamePad.GetDeviceRawState", nil),
        ("C_GamePad.GetActiveDeviceID", zero),
        ("C_GamePad.GetCombinedDeviceID", zero),
        ("C_GamePad.SetAllowHoverEventsWithFreeLook", nothing),
        ("C_GamePad.GetAllowHoverEventsWithFreeLook", no),
        ("C_GamePad.StickIndexToConfigName", nil),
        ("C_GamePad.AxisIndexToConfigName", nil),
        ("C_GamePad.ButtonIndexToConfigName", nil),
        ("C_GamePad.ButtonBindingToIndex", nil),
        ("C_GamePad.ButtonIndexToBinding", nil),
        ("C_GamePad.AddSDLMapping", no),
        ("C_GamePad.ApplyConfigs", nothing),
        ("C_GamePad.DeleteConfig", nothing),
        ("C_GamePad.GetAllConfigIDs", empty_table),
        ("C_GamePad.GetConfig", nil),
        ("C_GamePad.SetConfig", nothing),
        ("C_GamePad.GetLedColor", game_pad_led_color),
        ("C_GamePad.SetLedColor", nothing),
        ("C_GamePad.ClearLedColor", nothing),
        ("C_GamePad.GetPowerLevel", game_pad_power_level),
        ("C_GamePad.SetVibration", nothing),
        ("C_GamePad.StopVibration", nothing),
        // Text to speech: off.
        ("C_TTSSettings.SetSetting", nothing),
        ("C_TTSSettings.GetChannelEnabled", no),
        ("C_TTSSettings.SetChannelEnabled", nothing),
        ("C_TTSSettings.SetChannelKeyEnabled", nothing),
        ("C_TTSSettings.SetChatTypeEnabled", nothing),
        ("C_TTSSettings.SetDefaultSettings", nothing),
        ("C_TTSSettings.SetSpeechRate", nothing),
        ("C_TTSSettings.SetSpeechVolume", nothing),
        ("C_TTSSettings.SetVoiceOption", nothing),
        ("C_TTSSettings.SetVoiceOptionName", nothing),
        ("C_TTSSettings.GetVoiceOptionID", zero),
        ("C_TTSSettings.GetVoiceOptionName", empty_string),
        ("C_TTSSettings.ShouldOverrideMessage", no),
        (
            "C_TTSSettings.GetCharacterSettingsSaved",
            tts_character_settings_saved,
        ),
        (
            "C_TTSSettings.MarkCharacterSettingsSaved",
            tts_mark_character_settings_saved,
        ),
        // Heirlooms: every filter checked unless set.
        (
            "C_HeirloomInfo.AreAllCollectionFiltersChecked",
            are_all_collection_filters_checked,
        ),
        (
            "C_HeirloomInfo.AreAllSourceFiltersChecked",
            are_all_source_filters_checked,
        ),
        (
            "C_HeirloomInfo.SetAllCollectionFilters",
            set_all_collection_filters,
        ),
        ("C_HeirloomInfo.SetAllSourceFilters", set_all_source_filters),
        (
            "C_HeirloomInfo.SetDefaultFilters",
            set_default_heirloom_filters,
        ),
        // The combat log.
        ("CombatLogClearEntries", nothing),
        ("CombatLogGetRetentionTime", combat_log_get_retention_time),
        ("CombatLogSetRetentionTime", combat_log_set_retention_time),
        // Their twins, which Blizzard's deprecation file makes the globals
        // (the full probe: 300 both).
        (
            "C_CombatLog.GetEntryRetentionTime",
            combat_log_get_retention_time,
        ),
        (
            "C_CombatLog.SetEntryRetentionTime",
            combat_log_set_retention_time,
        ),
        ("CombatLog_Object_IsA", combat_log_object_is_a),
        ("CombatTextSetActiveUnit", nothing),
        // Secrets.
        (
            "C_Secrets.ShouldActionCooldownBeSecret",
            secret_while_restricted,
        ),
        (
            "C_Secrets.ShouldSpellBookItemCooldownBeSecret",
            secret_while_restricted,
        ),
        ("C_Secrets.ShouldTotemSlotBeSecret", secret_while_restricted),
        (
            "C_Secrets.ShouldTotemSpellBeSecret",
            secret_while_restricted,
        ),
        (
            "C_Secrets.ShouldUnitAuraIndexBeSecret",
            secret_while_restricted,
        ),
        (
            "C_Secrets.ShouldUnitAuraInstanceBeSecret",
            secret_while_restricted,
        ),
        (
            "C_Secrets.ShouldUnitAuraSlotBeSecret",
            secret_while_restricted,
        ),
        ("C_Secrets.ShouldUnitThreatStateBeSecret", threat_secret),
        ("C_Secrets.ShouldUnitThreatValuesBeSecret", threat_secret),
        (
            "C_Secrets.ShouldUnitSpellCastBeSecret",
            unit_spell_cast_secret,
        ),
        (
            "C_Secrets.ShouldUnitComparisonBeSecret",
            unit_comparison_secret,
        ),
        ("C_Secrets.CanCompareUnitTokens", can_compare_unit_tokens),
        // Utilities.
        ("C_EventUtils.IsCallbackEvent", is_callback_event),
        ("C_ColorUtil.WrapTextInColorCode", wrap_text_in_color_code),
        ("C_ColorUtil.ConvertRGBToHSV", convert_rgb_to_hsv),
        ("C_ColorUtil.ConvertHSVToRGB", convert_hsv_to_rgb),
        ("C_ColorUtil.ConvertHSVToHSL", convert_hsv_to_hsl),
        ("C_ColorUtil.ConvertHSLToHSV", convert_hsl_to_hsv),
        ("C_EncodingUtil.EncodeBase64", encode_base64),
        ("C_EncodingUtil.DecodeBase64", decode_base64),
        ("C_EncodingUtil.EncodeHex", encode_hex),
        ("C_EncodingUtil.DecodeHex", decode_hex),
        ("C_EncodingUtil.CompressString", compress_string),
        ("C_EncodingUtil.DecompressString", decompress_string),
        ("C_EncodingUtil.SerializeCBOR", serialize_cbor),
        ("C_EncodingUtil.DeserializeCBOR", deserialize_cbor),
        ("C_EncodingUtil.SerializeJSON", serialize_json),
        ("C_EncodingUtil.DeserializeJSON", deserialize_json),
        // Systems Forever lacks or the engine does not model.
        ("C_System.GetFrameStack", empty_table),
        ("C_SystemVisibilityManager.IsSystemVisible", yes),
        ("C_Navigation.GetDistance", zero),
        ("C_Navigation.GetNearestPartyMemberToken", empty_string),
        ("C_Navigation.GetTargetState", zero),
        ("C_Navigation.HasValidScreenPosition", no),
        ("C_Navigation.WasClampedToScreen", no),
        ("C_StorePublic.DoesGroupHavePurchaseableProducts", no),
        ("C_StorePublic.EventStoreUISetShown", nothing),
        ("C_Tutorial.AbandonTutorialArea", nothing),
        ("C_Tutorial.ReturnToTutorialArea", nothing),
        ("C_Tutorial.SendTutorialStartButtonTelem", nothing),
        ("C_Tutorial.GetCombatEventInfo", nothing),
        ("C_SettingsUtil.OpenSettingsPanel", nothing),
        ("C_Debug.DashboardIsEnabled", no),
        ("C_Debug.FrameXMLDebug", nothing),
        ("C_Debug.ToggleDebugCharInfo", nothing),
        ("C_Debug.ToggleWindDebugMenu", nothing),
        ("C_Debug.GetAllPortLocsForMap", empty_table),
        ("C_Debug.GetMapDebugObjects", empty_table),
        ("C_Debug.TeleportToMapDebugObject", nothing),
        ("C_Debug.TeleportToMapLocation", nothing),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        for days in [-1000, 0, 1, 59, 365, 11_000, 20_745] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2026, 10, 19), 20_745);
    }

    #[test]
    fn halves_round_away_from_zero() {
        assert_eq!(round_value(2.5), 3.0);
        assert_eq!(round_value(-2.5), -3.0);
        assert_eq!(round_value(2.4), 2.0);
    }

    #[test]
    fn half_floats() {
        assert_eq!(half_to_f64(0x3c00), 1.0);
        assert_eq!(half_to_f64(0xc000), -2.0);
        assert_eq!(half_to_f64(0x0000), 0.0);
    }
}
