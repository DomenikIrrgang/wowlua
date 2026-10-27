//! The documented userdata script objects whose methods had no behaviour:
//! duration clocks (`C_DurationUtil.CreateManualClock`, a duration
//! object's `SetClock`), locale contexts (`C_Intl.CreateLocaleContext`),
//! abbreviation configs (`CreateAbbreviateConfig`, and the options of
//! `AbbreviateNumbers` and `AbbreviateLargeNumbers` that use them), the
//! heal prediction calculators' predicted values, and the housing
//! catalog's searchers (housing: "nothing here").
//!
//! The rules come from Blizzard's documentation (camelot's
//! `Blizzard_APIDocumentationGenerated`: `LuaDurationClockAPI`,
//! `LuaDurationManualClockAPI`, `LuaLocaleContextAPI`, `AbbreviateConfigAPI`,
//! `UnitHealPredictionCalculatorAPI`, `HousingCatalogSearcherAPI`) and
//! Blizzard's own Lua where it once did the same work; a locale's formats
//! are ICU's (CLDR) as remembered, not probed. Unverified unless a comment
//! says otherwise: none of these objects was recorded on Forever.

use std::rc::Rc;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{LuaError, NativeCall, NativeFn, TableRef, Value, Vm};

use crate::engine;
use crate::widgets::{self, MethodFn, WidgetId};

// ------------------------------------------------------------- helpers

fn prop(vm: &mut Vm, id: WidgetId, key: &str) -> Value {
    let props = engine(vm).world.widgets[id.0].props;
    let k = vm.str(key);
    vm.heap.table(props).get(k)
}

fn set_prop(vm: &mut Vm, id: WidgetId, key: &str, v: Value) {
    let props = engine(vm).world.widgets[id.0].props;
    let k = vm.str(key);
    let _ = vm.heap.table_mut(props).set(k, v);
}

fn hide(vm: &mut Vm, secret: bool, v: Value) -> Value {
    if secret { vm.secret_wrap(v) } else { v }
}

/// Argument `n` as a number (a secret one unwrapped) and whether it was
/// secret.
fn num_arg(vm: &Vm, c: &NativeCall, n: usize) -> Option<(f64, bool)> {
    let v = c.arg(n);
    match vm.hidden(v) {
        Value::Num(x) => Some((x, matches!(v, Value::Secret(_)))),
        _ => None,
    }
}

/// Argument `n` as text (a number as Lua writes it; a secret one
/// unwrapped) and whether it was secret.
fn text_arg(vm: &mut Vm, c: &NativeCall, n: usize) -> Option<(String, bool)> {
    let v = c.arg(n);
    let secret = matches!(v, Value::Secret(_));
    let text = match vm.hidden(v) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(x) => lua_number(vm, x),
        _ => return None,
    };
    Some((text, secret))
}

/// A number as Lua's `tostring` writes it.
fn lua_number(vm: &mut Vm, n: f64) -> String {
    match vm.tostring(Value::Num(n)) {
        Ok(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => n.to_string(),
    }
}

/// A global string's text, or `fallback`.
fn global_text(vm: &mut Vm, name: &str, fallback: &str) -> String {
    match vm.get_global(name) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => fallback.to_string(),
    }
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

/// A field's value, a secret one unwrapped.
fn hidden_field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let v = field(vm, t, key);
    vm.hidden(v)
}

fn set_field(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

/// A list's tables, in order (secret entries unwrapped).
fn list_tables(vm: &Vm, list: TableRef) -> Vec<TableRef> {
    let n = vm.heap.table(list).border();
    (1..=n)
        .filter_map(
            |i| match vm.hidden(vm.heap.table(list).get(Value::Num(i as f64))) {
                Value::Table(t) => Some(t),
                _ => None,
            },
        )
        .collect()
}

fn deep_copy(vm: &mut Vm, t: TableRef) -> TableRef {
    let out = vm.new_table();
    let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
    for (k, v) in entries {
        let v = match v {
            Value::Table(inner) => Value::Table(deep_copy(vm, inner)),
            v => v,
        };
        let _ = vm.heap.table_mut(out).set(k, v);
    }
    out
}

/// A list of numbers as a Lua table.
fn number_list(vm: &mut Vm, items: &[f64]) -> Value {
    let t = vm.new_table();
    for (i, n) in items.iter().enumerate() {
        let _ = vm
            .heap
            .table_mut(t)
            .set(Value::Num(i as f64 + 1.0), Value::Num(*n));
    }
    Value::Table(t)
}

/// What the engine answers for the client function `name` called with
/// `args` (its behaviour; none: nothing), as `extras::ask`: a locale
/// context's locale-free text methods answer as `C_Intl`'s.
fn ask(vm: &mut Vm, name: &'static str, args: &[Value]) -> Vec<Value> {
    let Some(f) = engine(vm).behaviour(name) else {
        return vec![];
    };
    let full = vm.str(name);
    let call = NativeCall {
        args: args.iter().copied().collect(),
        taints: args.iter().map(|_| Default::default()).collect(),
        origins: args.iter().map(|_| Default::default()).collect(),
        upvals: Rc::from([full]),
        upval_taints: Rc::from([Default::default()]),
    };
    match f(vm, call) {
        Ok(values) => values.into_iter().map(|v| vm.hidden(v)).collect(),
        Err(_) => vec![],
    }
}

/// The documented methods of `class`, as `Class:Method` names.
fn named(class: &str, list: &[(&str, MethodFn)]) -> Vec<(&'static str, MethodFn)> {
    list.iter()
        .map(|&(short, f)| {
            let full: &'static str = Box::leak(format!("{class}:{short}").into_boxed_str());
            (full, f)
        })
        .collect()
}

// --------------------------------------------------------------- clocks

/// The documentation's clock classes (`LuaDurationClockObjectAPI`,
/// `LuaDurationManualClockAPI`).
const CLOCK: &str = "DurationClockObject";
const MANUAL_CLOCK: &str = "DurationManualClock";

/// `C_DurationUtil.CreateManualClock()`: "a manually driven time source for
/// use with duration objects" (the documentation), at time 0 (unverified:
/// `ResetTime` "resets the clock to a zero time value"). The documentation
/// gives the manual clock no base, but a duration object takes it as its
/// `LuaDurationClock`, whose method is `GetTime`: the manual clock gets
/// that method too (as `formatters::create_formatter` gives formatters
/// `FormatNumber`; unverified).
fn create_manual_clock(vm: &mut Vm, _c: NativeCall) -> Ret {
    let meta = widgets::class_meta(vm, MANUAL_CLOCK);
    let index = vm.str("__index");
    if let Value::Table(methods) = vm.heap.table(meta).get(index) {
        let k = vm.str("GetTime");
        if vm.heap.table(methods).get(k).is_nil() {
            let f = vm.native("GetTime", clock_get_time_native);
            let _ = vm.heap.table_mut(methods).set(k, f);
        }
    }
    let clock = widgets::create(vm, MANUAL_CLOCK, None, None);
    if let Some(id) = widgets::widget_of(vm, clock) {
        set_prop(vm, id, "time", Value::Num(0.0));
    }
    Ok(vec![clock])
}

fn is_manual_clock(vm: &Vm, id: WidgetId) -> bool {
    widgets::is_a(
        engine(vm),
        &engine(vm).world.widgets[id.0].class,
        MANUAL_CLOCK,
    )
}

fn is_clock(vm: &Vm, id: WidgetId) -> bool {
    let e = engine(vm);
    let class = &e.world.widgets[id.0].class;
    widgets::is_a(e, class, MANUAL_CLOCK) || widgets::is_a(e, class, CLOCK)
}

/// A clock's time and whether a secret set it: a manual clock's own time,
/// any other clock's the engine's (`GetTime`'s).
fn clock_time(vm: &mut Vm, id: WidgetId) -> (f64, bool) {
    if !is_manual_clock(vm, id) {
        return (crate::clock::now(vm), false);
    }
    let time = match prop(vm, id, "time") {
        Value::Num(n) => n,
        _ => 0.0,
    };
    (time, prop(vm, id, "secret").truthy())
}

/// `clock:GetTime()`: "the current time value represented by this clock"
/// (the documentation), secret once a secret set it (unverified).
fn clock_get_time(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let (t, secret) = clock_time(vm, id);
    Ok(vec![hide(vm, secret, Value::Num(t))])
}

/// The same as a plain native, for the manual clocks' metatable.
fn clock_get_time_native(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = widgets::widget_of(vm, c.arg(1)) else {
        return Err(vm.error("Wrong object type for function - Usage: local time = self:GetTime()"));
    };
    clock_get_time(vm, id, c)
}

/// Sets a manual clock's time (a secret one marks the clock secret until
/// it is reset).
fn set_clock_time(vm: &mut Vm, id: WidgetId, time: f64, secret: bool) {
    set_prop(vm, id, "time", Value::Num(time));
    if secret {
        set_prop(vm, id, "secret", Value::Bool(true));
    }
}

fn seconds_arg(vm: &mut Vm, c: &NativeCall) -> Result<(f64, bool), LuaError> {
    num_arg(vm, c, 2).ok_or_else(|| vm.error("a number of seconds expected"))
}

/// `AdvanceTime(delta)`: "advances the clock by a specified number of
/// seconds" (the documentation; unverified: a negative delta goes back).
fn advance_time(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (delta, secret) = seconds_arg(vm, &c)?;
    let (now, _) = clock_time(vm, id);
    set_clock_time(vm, id, now + delta, secret);
    Ok(vec![])
}

/// `RewindTime(delta)`: back by that many seconds (unverified: below zero
/// allowed).
fn rewind_time(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (delta, secret) = seconds_arg(vm, &c)?;
    let (now, _) = clock_time(vm, id);
    set_clock_time(vm, id, now - delta, secret);
    Ok(vec![])
}

/// `ResetTime()`: "resets the clock to a zero time value" (and, unverified,
/// forgets a secret).
fn reset_time(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    set_prop(vm, id, "time", Value::Num(0.0));
    set_prop(vm, id, "secret", Value::Nil);
    Ok(vec![])
}

/// `SetTime(time)`: "sets the current clock timestamp to a given value".
fn set_time(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (time, secret) = seconds_arg(vm, &c)?;
    set_prop(vm, id, "secret", Value::Nil);
    set_clock_time(vm, id, time, secret);
    Ok(vec![])
}

// ------------------------------------------------- durations on a clock

/// The clock a duration object was given (`SetClock`), kept as the
/// setter's stored value (so the stored-setter rules read it back too).
fn duration_clock(vm: &mut Vm, id: WidgetId) -> Option<WidgetId> {
    let props = engine(vm).world.widgets[id.0].props;
    let v = widgets::stored_args(vm, props, "Clock")?.first().copied()?;
    widgets::widget_of(vm, v)
}

/// A duration method run on the duration's clock: the curves module
/// measures a span against the engine's time (`clock::now`), so for a
/// manual clock that time is the clock's while the method runs (to the
/// millisecond, as every engine time), the engine's again after. Without
/// a clock, or on a clock that follows the engine, the method as it is.
fn on_clock(vm: &mut Vm, id: WidgetId, c: NativeCall, f: MethodFn) -> Ret {
    let Some(clock) = duration_clock(vm, id).filter(|&k| is_manual_clock(vm, k)) else {
        return f(vm, id, c);
    };
    let (t, _) = clock_time(vm, clock);
    let saved = vm.clock.elapsed_ms;
    vm.clock.elapsed_ms = t * 1000.0;
    let out = f(vm, id, c);
    vm.clock.elapsed_ms = saved;
    out
}

fn duration_get(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    on_clock(vm, id, c, crate::curves::duration_get)
}

fn duration_evaluate(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    on_clock(vm, id, c, crate::curves::duration_evaluate)
}

fn duration_format(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    on_clock(vm, id, c, crate::curves::duration_format)
}

/// `SetClock(clock)`: the clock source; nil for "an internal default clock
/// source equivalent to GetTime()" (the documentation).
fn duration_set_clock(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let v = c.arg(2);
    if !v.is_nil() && !widgets::widget_of(vm, v).is_some_and(|k| is_clock(vm, k)) {
        return Err(vm.error("Wrong object type for function - Usage: self:SetClock(clock)"));
    }
    widgets::store(vm, id, "Clock", &[v]);
    Ok(vec![])
}

/// `GetClock()`: the clock set, nil for the default one.
fn duration_get_clock(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![match duration_clock(vm, id) {
        Some(k) => widgets::value_of(engine(vm), k),
        None => Value::Nil,
    }])
}

/// `GetClockTime()`: "the current time of the clock source used by this
/// object" (the manual clock's exactly).
fn duration_clock_time(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let (t, secret) = match duration_clock(vm, id) {
        Some(k) => clock_time(vm, k),
        None => (crate::clock::now(vm), false),
    };
    Ok(vec![hide(vm, secret, Value::Num(t))])
}

/// The clock value stored on `from`, copied to `to` (none: the default).
fn copy_clock(vm: &mut Vm, from: WidgetId, to: WidgetId) {
    let clock = match duration_clock(vm, from) {
        Some(k) => widgets::value_of(engine(vm), k),
        None => Value::Nil,
    };
    widgets::store(vm, to, "Clock", &[clock]);
}

/// `Copy()`: a copy keeps the clock (unverified).
fn duration_copy(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let out = crate::curves::copy(vm, id, c)?;
    if let Some(new) = out.first().and_then(|v| widgets::widget_of(vm, *v)) {
        copy_clock(vm, id, new);
    }
    Ok(out)
}

/// `Assign(other)`: "copies another duration object", its clock too
/// (unverified).
fn duration_assign(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let other = widgets::widget_of(vm, c.arg(2));
    let out = crate::curves::assign(vm, id, c)?;
    if let Some(other) = other {
        copy_clock(vm, other, id);
    }
    Ok(out)
}

/// `SetToDefaults()`: a zero span on the default clock (unverified: the
/// clock goes back to the default; `Reset` keeps it, as it only "resets the
/// duration object to represent a zero duration time span").
fn duration_to_defaults(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let out = crate::curves::reset(vm, id, c)?;
    widgets::store(vm, id, "Clock", &[Value::Nil]);
    Ok(out)
}

// ------------------------------------------------------ abbreviate configs

const ABBREVIATE_CONFIG: &str = "AbbreviateConfig";

/// Checked copies of a list of `NumberAbbreviationBreakpoint`s (the
/// predicate's errors as the abbreviated number formatter's: the
/// documentation names `RequiresRestrictedAbbreviationBreakpoints`
/// without its rules, read here as `RequiresValidAbbreviationBreakpoints`'s,
/// unverified).
fn checked_breakpoints(vm: &mut Vm, v: Value) -> Result<TableRef, LuaError> {
    let Value::Table(list) = vm.hidden(v) else {
        return Err(vm.error("a table of abbreviation breakpoints expected"));
    };
    let out = vm.new_table();
    for (i, bp) in list_tables(vm, list).into_iter().enumerate() {
        let (copy, _) = crate::formatters::abbreviation_copy(vm, Value::Table(bp))?;
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num(i as f64 + 1.0), Value::Table(copy));
    }
    Ok(out)
}

/// `CreateAbbreviateConfig(data)`: "a cached config object" for
/// `AbbreviateNumbers`' options (the documentation of
/// `NumberAbbrevOptions.config`), holding the breakpoints given.
fn create_abbreviate_config(vm: &mut Vm, c: NativeCall) -> Ret {
    let data = checked_breakpoints(vm, c.arg(1))?;
    let config = widgets::create(vm, ABBREVIATE_CONFIG, None, None);
    if let Some(id) = widgets::widget_of(vm, config) {
        set_prop(vm, id, "data", Value::Table(data));
    }
    Ok(vec![config])
}

/// A config's breakpoints (the table kept, not a copy).
fn config_data(vm: &mut Vm, id: WidgetId) -> TableRef {
    match prop(vm, id, "data") {
        Value::Table(t) => t,
        _ => {
            let t = vm.new_table();
            set_prop(vm, id, "data", Value::Table(t));
            t
        }
    }
}

/// `GetAbbreviateNumberData()`: a copy of the config's breakpoints.
fn get_abbreviate_data(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let data = config_data(vm, id);
    Ok(vec![Value::Table(deep_copy(vm, data))])
}

/// `SetAbbreviateNumberData(data)`: all replaced, none when one is invalid.
fn set_abbreviate_data(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let data = checked_breakpoints(vm, c.arg(2))?;
    set_prop(vm, id, "data", Value::Table(data));
    Ok(vec![])
}

/// Korean's and Chinese's breakpoints: Blizzard's
/// `LocalizeNumberAbbreviation_Asian` (cata's `Localization.lua`, the Lua
/// these functions replaced) as (breakpoint, global, significand divisor,
/// fraction divisor).
const ASIAN_BREAKPOINTS: [(f64, &str, f64, f64); 3] = [
    (1e9, "SECOND_NUMBER_CAP_NO_SPACE", 1e7, 1.0),
    (1e7, "SECOND_NUMBER_CAP_NO_SPACE", 1e6, 10.0),
    (1e4, "FIRST_NUMBER_CAP_NO_SPACE", 1e3, 10.0),
];

/// One breakpoint: its threshold, abbreviation text, divisors.
struct Breakpoint {
    at: f64,
    text: String,
    significand: f64,
    fraction: f64,
}

/// An abbreviation's text: the global string it names (the enUS text of
/// the abbreviation globals where they are not defined, as
/// `formatters::abbreviated_format`), empty for a name that is no global
/// string (recorded: "K" abbreviates 1234 to "1.2"); the raw text with
/// `abbreviationIsGlobal` false.
fn abbreviation_text(vm: &mut Vm, name: &str, global: bool) -> String {
    if !global {
        return name.to_string();
    }
    let fallback = match name {
        "FIRST_NUMBER_CAP_NO_SPACE" => "K",
        "SECOND_NUMBER_CAP_NO_SPACE" => "M",
        "THIRD_NUMBER_CAP_NO_SPACE" => "B",
        "FOURTH_NUMBER_CAP_NO_SPACE" => "T",
        _ => "",
    };
    global_text(vm, name, fallback)
}

/// A breakpoint list's entries in their order (fields missing or not
/// numbers: skipped; unverified for a list `AbbreviateNumbers` is given
/// directly, which the documentation does not say it checks).
fn read_breakpoints(vm: &mut Vm, list: TableRef) -> Vec<Breakpoint> {
    let mut out = Vec::new();
    for t in list_tables(vm, list) {
        let num = |vm: &mut Vm, k: &str| match hidden_field(vm, t, k) {
            Value::Num(n) => Some(n),
            _ => None,
        };
        let (Some(at), Some(significand), Some(fraction)) = (
            num(vm, "breakpoint"),
            num(vm, "significandDivisor"),
            num(vm, "fractionDivisor"),
        ) else {
            continue;
        };
        let name = match hidden_field(vm, t, "abbreviation") {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        };
        let global = !matches!(
            hidden_field(vm, t, "abbreviationIsGlobal"),
            Value::Bool(false)
        );
        let text = abbreviation_text(vm, &name, global);
        out.push(Breakpoint {
            at,
            text,
            significand,
            fraction,
        });
    }
    out
}

/// `n` by the first breakpoint, in the given order ("order these from
/// largest to smallest"), that it reaches: its significand floored over
/// the fraction divisor, then the abbreviation; below them all, as Lua
/// writes it. Blizzard's Lua `AbbreviateNumbers` (cata's `UIParent.lua`).
fn abbreviate_by(vm: &mut Vm, n: f64, list: &[Breakpoint]) -> String {
    for bp in list {
        if n >= bp.at && bp.significand != 0.0 && bp.fraction != 0.0 {
            let value = (n / bp.significand).floor() / bp.fraction;
            return format!("{}{}", lua_number(vm, value), bp.text);
        }
    }
    lua_number(vm, n)
}

/// The breakpoints an options table asks for: its config's, else its
/// `breakpointData`, else Korean's and Chinese's for those locales; none
/// for the defaults.
fn option_breakpoints(vm: &mut Vm, options: Value) -> Option<(Vec<Breakpoint>, bool)> {
    let Value::Table(t) = vm.hidden(options) else {
        return None;
    };
    let config = field(vm, t, "config");
    if let Some(id) = widgets::widget_of(vm, config) {
        let data = config_data(vm, id);
        return Some((read_breakpoints(vm, data), false));
    }
    if let Value::Table(list) = hidden_field(vm, t, "breakpointData") {
        return Some((read_breakpoints(vm, list), false));
    }
    let locale = match hidden_field(vm, t, "locale") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => return None,
    };
    if !is_asian_locale(&locale) {
        return None;
    }
    let list = ASIAN_BREAKPOINTS
        .iter()
        .map(|&(at, name, significand, fraction)| Breakpoint {
            at,
            text: abbreviation_text(vm, name, true),
            significand,
            fraction,
        })
        .collect();
    Some((list, true))
}

fn is_asian_locale(locale: &str) -> bool {
    let lower = locale.to_ascii_lowercase();
    lower.starts_with("ko") || lower.starts_with("zh")
}

/// `AbbreviateNumbers(number, options?)`: with options, by their
/// breakpoints (above); without, the engine's answer
/// (`queries::abbreviate_numbers`, recorded).
fn abbreviate_numbers(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((list, _)) = option_breakpoints(vm, c.arg(2)) else {
        return crate::queries::abbreviate_numbers(vm, c);
    };
    let Some((n, secret)) = num_arg(vm, &c, 1) else {
        return crate::queries::abbreviate_numbers(vm, c);
    };
    let text = abbreviate_by(vm, n, &list);
    let v = vm.str(&text);
    Ok(vec![hide(vm, secret, v)])
}

/// `AbbreviateLargeNumbers(number, options?)`: a config's or a list's
/// breakpoints as `AbbreviateNumbers` uses them (unverified); Korean's and
/// Chinese's locales by Blizzard's Lua for them
/// (`LocalizeNumberAbbreviation_Asian`: by the digits, from 11 the
/// `SECOND_NUMBER_CAP` unit's whole, from 9 with two decimals, from 7 the
/// `FIRST_NUMBER_CAP` unit's, from 4 broken up); else the engine's answer
/// (`queries::abbreviate_large_numbers`, recorded).
fn abbreviate_large_numbers(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some((list, asian)) = option_breakpoints(vm, c.arg(2)) else {
        return crate::queries::abbreviate_large_numbers(vm, c);
    };
    let Some((n, secret)) = num_arg(vm, &c, 1) else {
        return crate::queries::abbreviate_large_numbers(vm, c);
    };
    let text = if asian {
        let digits = lua_number(vm, n);
        let len = digits.len();
        if !digits.is_ascii() {
            digits
        } else if len >= 11 {
            let cap = global_text(vm, "SECOND_NUMBER_CAP", " M");
            format!("{}{cap}", &digits[..len - 7])
        } else if len >= 9 {
            let cap = global_text(vm, "SECOND_NUMBER_CAP", " M");
            format!("{}.{}{cap}", &digits[..len - 8], &digits[len - 8..len - 6])
        } else if len >= 7 {
            let cap = global_text(vm, "FIRST_NUMBER_CAP", " K");
            format!("{}{cap}", &digits[..len - 4])
        } else if len > 3 {
            match ask(vm, "BreakUpLargeNumbers", &[Value::Num(n)]).first() {
                Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(*s)).into_owned(),
                _ => digits,
            }
        } else {
            digits
        }
    } else {
        abbreviate_by(vm, n, &list)
    };
    let v = vm.str(&text);
    Ok(vec![hide(vm, secret, v)])
}

// ----------------------------------------------------- locale contexts

const LOCALE_CONTEXT: &str = "LocaleContext";

/// A locale ID's parts. WoW's own IDs (`enUS`) read as language and
/// region; ICU's (`en_US`, `en-US`, `zh_Hant_TW`) by their subtags.
#[derive(Clone, Default)]
struct Locale {
    language: String,
    script: String,
    region: String,
    variant: String,
}

impl Locale {
    fn parse(id: &str) -> Locale {
        let b = id.as_bytes();
        // WoW's form: two small letters, two capitals (unverified: the
        // client's ICU reads it so).
        if b.len() == 4
            && b[..2].iter().all(u8::is_ascii_lowercase)
            && b[2..].iter().all(u8::is_ascii_uppercase)
        {
            return Locale {
                language: id[..2].to_string(),
                region: id[2..].to_string(),
                ..Locale::default()
            };
        }
        let mut parts = id.split(['_', '-']).filter(|p| !p.is_empty());
        let mut out = Locale {
            language: parts.next().unwrap_or_default().to_ascii_lowercase(),
            ..Locale::default()
        };
        for p in parts {
            let alpha = p.bytes().all(|c| c.is_ascii_alphabetic());
            if out.script.is_empty() && out.region.is_empty() && p.len() == 4 && alpha {
                let mut s = p.to_ascii_lowercase();
                s[..1].make_ascii_uppercase();
                out.script = s;
            } else if out.region.is_empty()
                && ((p.len() == 2 && alpha)
                    || (p.len() == 3 && p.bytes().all(|c| c.is_ascii_digit())))
            {
                out.region = p.to_ascii_uppercase();
            } else {
                if !out.variant.is_empty() {
                    out.variant.push('_');
                }
                out.variant.push_str(&p.to_ascii_uppercase());
            }
        }
        out
    }

    /// ICU's canonical form (`en_US`).
    fn id(&self) -> String {
        let mut out = self.language.clone();
        for part in [&self.script, &self.region] {
            if !part.is_empty() {
                out.push('_');
                out.push_str(part);
            }
        }
        if !self.variant.is_empty() {
            if self.region.is_empty() {
                out.push('_');
            }
            out.push('_');
            out.push_str(&self.variant);
        }
        out
    }

    /// Traditional Chinese: a `Hant` script, or Taiwan, Hong Kong, Macau.
    fn traditional(&self) -> bool {
        self.language == "zh"
            && (self.script == "Hant" || matches!(self.region.as_str(), "TW" | "HK" | "MO"))
    }
}

/// CLDR's likely subtags for WoW's languages: (language, script, region).
const LIKELY: [(&str, &str, &str); 9] = [
    ("en", "Latn", "US"),
    ("de", "Latn", "DE"),
    ("fr", "Latn", "FR"),
    ("es", "Latn", "ES"),
    ("it", "Latn", "IT"),
    ("pt", "Latn", "BR"),
    ("ru", "Cyrl", "RU"),
    ("ko", "Kore", "KR"),
    ("zh", "Hans", "CN"),
];

/// A locale's formatting data (CLDR's, as remembered; unverified).
struct LangData {
    group: &'static str,
    decimal: &'static str,
    /// Spanish does not group four digits (`1234`, `12.345`).
    min_grouping: usize,
    /// The percent pattern around the number (`{n}%`).
    percent: (&'static str, &'static str),
    /// The currency symbol before the number (else after, spaced).
    currency_first: bool,
    months: [&'static str; 12],
    months_short: [&'static str; 12],
    weekdays: [&'static str; 7],
    /// Short, Medium, Long, Full.
    dates: [&'static str; 4],
    times: [&'static str; 4],
    /// The date and time joined for Full and Long, and for Medium and
    /// Short dates.
    glue_long: &'static str,
    glue_short: &'static str,
    am_pm: [&'static str; 2],
    utc: &'static str,
}

const EN_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const EN_MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const EN_WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const NUMERIC_MONTHS: [&str; 12] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12",
];

const EN_US: LangData = LangData {
    group: ",",
    decimal: ".",
    min_grouping: 1,
    percent: ("", "%"),
    currency_first: true,
    months: EN_MONTHS,
    months_short: EN_MONTHS_SHORT,
    weekdays: EN_WEEKDAYS,
    dates: ["M/d/yy", "MMM d, y", "MMMM d, y", "EEEE, MMMM d, y"],
    times: [
        "h:mm\u{202F}a",
        "h:mm:ss\u{202F}a",
        "h:mm:ss\u{202F}a z",
        "h:mm:ss\u{202F}a zzzz",
    ],
    glue_long: "{1} 'at' {0}",
    glue_short: "{1}, {0}",
    am_pm: ["AM", "PM"],
    utc: "Coordinated Universal Time",
};

const EN_GB: LangData = LangData {
    months_short: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sept", "Oct", "Nov", "Dec",
    ],
    dates: ["dd/MM/y", "d MMM y", "d MMMM y", "EEEE d MMMM y"],
    times: ["HH:mm", "HH:mm:ss", "HH:mm:ss z", "HH:mm:ss zzzz"],
    am_pm: ["am", "pm"],
    ..EN_US
};

const DE: LangData = LangData {
    group: ".",
    decimal: ",",
    min_grouping: 1,
    percent: ("", "\u{A0}%"),
    currency_first: false,
    months: [
        "Januar",
        "Februar",
        "März",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
    months_short: [
        "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.",
        "Dez.",
    ],
    weekdays: [
        "Sonntag",
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
    ],
    dates: ["dd.MM.yy", "dd.MM.y", "d. MMMM y", "EEEE, d. MMMM y"],
    times: ["HH:mm", "HH:mm:ss", "HH:mm:ss z", "HH:mm:ss zzzz"],
    glue_long: "{1} 'um' {0}",
    glue_short: "{1}, {0}",
    am_pm: ["AM", "PM"],
    utc: "Koordinierte Weltzeit",
};

const FR: LangData = LangData {
    group: "\u{202F}",
    decimal: ",",
    min_grouping: 1,
    percent: ("", "\u{202F}%"),
    currency_first: false,
    months: [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ],
    months_short: [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ],
    weekdays: [
        "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
    ],
    dates: ["dd/MM/y", "d MMM y", "d MMMM y", "EEEE d MMMM y"],
    times: ["HH:mm", "HH:mm:ss", "HH:mm:ss z", "HH:mm:ss zzzz"],
    glue_long: "{1} 'à' {0}",
    glue_short: "{1} {0}",
    am_pm: ["AM", "PM"],
    utc: "temps universel coordonné",
};

const ES: LangData = LangData {
    group: ".",
    decimal: ",",
    min_grouping: 2,
    percent: ("", "\u{A0}%"),
    currency_first: false,
    months: [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ],
    months_short: [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
    ],
    weekdays: [
        "domingo",
        "lunes",
        "martes",
        "miércoles",
        "jueves",
        "viernes",
        "sábado",
    ],
    dates: [
        "d/M/yy",
        "d MMM y",
        "d 'de' MMMM 'de' y",
        "EEEE, d 'de' MMMM 'de' y",
    ],
    times: ["H:mm", "H:mm:ss", "H:mm:ss z", "H:mm:ss (zzzz)"],
    glue_long: "{1}, {0}",
    glue_short: "{1}, {0}",
    am_pm: ["a.\u{A0}m.", "p.\u{A0}m."],
    utc: "tiempo universal coordinado",
};

const IT: LangData = LangData {
    group: ".",
    decimal: ",",
    min_grouping: 1,
    percent: ("", "%"),
    currency_first: false,
    months: [
        "gennaio",
        "febbraio",
        "marzo",
        "aprile",
        "maggio",
        "giugno",
        "luglio",
        "agosto",
        "settembre",
        "ottobre",
        "novembre",
        "dicembre",
    ],
    months_short: [
        "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
    ],
    weekdays: [
        "domenica",
        "lunedì",
        "martedì",
        "mercoledì",
        "giovedì",
        "venerdì",
        "sabato",
    ],
    dates: ["dd/MM/yy", "d MMM y", "d MMMM y", "EEEE d MMMM y"],
    times: ["HH:mm", "HH:mm:ss", "HH:mm:ss z", "HH:mm:ss zzzz"],
    glue_long: "{1} 'alle ore' {0}",
    glue_short: "{1}, {0}",
    am_pm: ["AM", "PM"],
    utc: "Tempo coordinato universale",
};

const PT: LangData = LangData {
    group: ".",
    decimal: ",",
    min_grouping: 1,
    percent: ("", "%"),
    currency_first: true,
    months: [
        "janeiro",
        "fevereiro",
        "março",
        "abril",
        "maio",
        "junho",
        "julho",
        "agosto",
        "setembro",
        "outubro",
        "novembro",
        "dezembro",
    ],
    months_short: [
        "jan.", "fev.", "mar.", "abr.", "mai.", "jun.", "jul.", "ago.", "set.", "out.", "nov.",
        "dez.",
    ],
    weekdays: [
        "domingo",
        "segunda-feira",
        "terça-feira",
        "quarta-feira",
        "quinta-feira",
        "sexta-feira",
        "sábado",
    ],
    dates: [
        "dd/MM/y",
        "d 'de' MMM 'de' y",
        "d 'de' MMMM 'de' y",
        "EEEE, d 'de' MMMM 'de' y",
    ],
    times: ["HH:mm", "HH:mm:ss", "HH:mm:ss z", "HH:mm:ss zzzz"],
    glue_long: "{1} {0}",
    glue_short: "{1} {0}",
    am_pm: ["AM", "PM"],
    utc: "Tempo Universal Coordenado",
};

const RU: LangData = LangData {
    group: "\u{A0}",
    decimal: ",",
    min_grouping: 1,
    percent: ("", "\u{A0}%"),
    currency_first: false,
    months: [
        "января",
        "февраля",
        "марта",
        "апреля",
        "мая",
        "июня",
        "июля",
        "августа",
        "сентября",
        "октября",
        "ноября",
        "декабря",
    ],
    months_short: [
        "янв.",
        "февр.",
        "мар.",
        "апр.",
        "мая",
        "июн.",
        "июл.",
        "авг.",
        "сент.",
        "окт.",
        "нояб.",
        "дек.",
    ],
    weekdays: [
        "воскресенье",
        "понедельник",
        "вторник",
        "среда",
        "четверг",
        "пятница",
        "суббота",
    ],
    dates: [
        "dd.MM.y",
        "d MMM y 'г'.",
        "d MMMM y 'г'.",
        "EEEE, d MMMM y 'г'.",
    ],
    times: ["HH:mm", "HH:mm:ss", "HH:mm:ss z", "HH:mm:ss zzzz"],
    glue_long: "{1}, {0}",
    glue_short: "{1}, {0}",
    am_pm: ["AM", "PM"],
    utc: "Всемирное координированное время",
};

const KO: LangData = LangData {
    group: ",",
    decimal: ".",
    min_grouping: 1,
    percent: ("", "%"),
    currency_first: true,
    months: NUMERIC_MONTHS,
    months_short: NUMERIC_MONTHS,
    weekdays: [
        "일요일",
        "월요일",
        "화요일",
        "수요일",
        "목요일",
        "금요일",
        "토요일",
    ],
    dates: ["yy. M. d.", "y. M. d.", "y년 M월 d일", "y년 M월 d일 EEEE"],
    times: [
        "a h:mm",
        "a h:mm:ss",
        "a h시 m분 s초 z",
        "a h시 m분 s초 zzzz",
    ],
    glue_long: "{1} {0}",
    glue_short: "{1} {0}",
    am_pm: ["오전", "오후"],
    utc: "협정 세계시",
};

const ZH: LangData = LangData {
    group: ",",
    decimal: ".",
    min_grouping: 1,
    percent: ("", "%"),
    currency_first: true,
    months: NUMERIC_MONTHS,
    months_short: NUMERIC_MONTHS,
    weekdays: [
        "星期日",
        "星期一",
        "星期二",
        "星期三",
        "星期四",
        "星期五",
        "星期六",
    ],
    dates: ["y/M/d", "y年M月d日", "y年M月d日", "y年M月d日EEEE"],
    times: ["HH:mm", "HH:mm:ss", "z HH:mm:ss", "zzzz HH:mm:ss"],
    glue_long: "{1} {0}",
    glue_short: "{1} {0}",
    am_pm: ["上午", "下午"],
    utc: "协调世界时",
};

const ZH_TW: LangData = LangData {
    dates: ["y/M/d", "y年M月d日", "y年M月d日", "y年M月d日 EEEE"],
    times: ["ah:mm", "ah:mm:ss", "ah:mm:ss [z]", "ah:mm:ss [zzzz]"],
    utc: "世界標準時間",
    ..ZH
};

/// The data for a locale: its language's (British English's for English
/// outside the United States and its territories, traditional Chinese's
/// for Taiwan; unverified), American English's for a language without
/// data here (unverified).
fn lang_data(loc: &Locale) -> &'static LangData {
    match loc.language.as_str() {
        "en" if !matches!(loc.region.as_str(), "" | "US" | "PR" | "GU" | "VI" | "AS") => &EN_GB,
        "de" => &DE,
        "fr" => &FR,
        "es" => &ES,
        "it" => &IT,
        "pt" => &PT,
        "ru" => &RU,
        "ko" => &KO,
        "zh" if loc.traditional() => &ZH_TW,
        "zh" => &ZH,
        _ => &EN_US,
    }
}

fn create_locale_context(vm: &mut Vm, c: NativeCall) -> Ret {
    let locale = match text_arg(vm, &c, 1) {
        Some((t, _)) => t,
        None => return Err(vm.error("Usage: local context = C_Intl.CreateLocaleContext(locale)")),
    };
    let context = widgets::create(vm, LOCALE_CONTEXT, None, None);
    if let Some(id) = widgets::widget_of(vm, context) {
        let v = vm.str(&locale);
        set_prop(vm, id, "locale", v);
    }
    Ok(vec![context])
}

/// The context's locale as given.
fn locale_text(vm: &mut Vm, id: WidgetId) -> String {
    match prop(vm, id, "locale") {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => String::new(),
    }
}

fn locale_of(vm: &mut Vm, id: WidgetId) -> Locale {
    Locale::parse(&locale_text(vm, id))
}

/// Whether any of the call's arguments after `self` is secret: the
/// documentation's `SecretArguments = "AllowedWhenTainted"`, read as the
/// formatters do, a secret in makes a secret out (unverified).
fn any_secret(c: &NativeCall) -> bool {
    c.args.iter().skip(1).any(|v| matches!(v, Value::Secret(_)))
}

fn text_result(vm: &mut Vm, c: &NativeCall, text: &str) -> Ret {
    let v = vm.str(text);
    Ok(vec![hide(vm, any_secret(c), v)])
}

/// `GetLocale()`: "the locale ID of this locale context", as it was given
/// (unverified: ICU's canonical form).
fn context_get_locale(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let text = locale_text(vm, id);
    Ok(vec![vm.str(&text)])
}

/// `SetLocale(locale)`: true when it was updated (an empty ID is not:
/// unverified).
fn context_set_locale(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some((locale, _)) = text_arg(vm, &c, 2) else {
        return Ok(vec![Value::Bool(false)]);
    };
    if locale.trim().is_empty() {
        return Ok(vec![Value::Bool(false)]);
    }
    let v = vm.str(&locale);
    set_prop(vm, id, "locale", v);
    Ok(vec![Value::Bool(true)])
}

/// A locale-free text method answers as `C_Intl`'s does (the same
/// arguments after `self`; English's rules there, which no WoW locale's
/// casing differs from: Turkish is not one).
fn delegate(vm: &mut Vm, c: &NativeCall, name: &'static str) -> Ret {
    let args: Vec<Value> = c.args.iter().skip(1).copied().collect();
    let out = ask(vm, name, &args);
    let secret = any_secret(c);
    Ok(out.into_iter().map(|v| hide(vm, secret, v)).collect())
}

fn context_compare(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    delegate(vm, &c, "C_Intl.CompareStrings")
}

fn context_fold_case(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    delegate(vm, &c, "C_Intl.FoldCase")
}

fn context_length(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    delegate(vm, &c, "C_Intl.Length")
}

fn context_to_lower(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    delegate(vm, &c, "C_Intl.ToLower")
}

fn context_to_upper(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    delegate(vm, &c, "C_Intl.ToUpper")
}

fn context_to_title(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    delegate(vm, &c, "C_Intl.ToTitle")
}

// ----------------------------------------------------------- numbers

/// A whole number's digits grouped by threes with `sep` (none when it has
/// fewer than `3 + min_grouping` digits).
fn group_digits(digits: &str, sep: &str, min_grouping: usize) -> String {
    if digits.len() < 3 + min_grouping {
        return digits.to_string();
    }
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push_str(sep);
        }
        out.push(ch);
    }
    out
}

/// `|n|` with up to `places` decimals (trailing zeros dropped unless
/// `keep_zeros`), the locale's separators; and whether it is negative and
/// not zero as written (as `C_Intl.FormatNumber`'s English).
fn decimal_text(n: f64, places: usize, keep_zeros: bool, data: &LangData) -> (String, bool) {
    let text = format!("{:.*}", places, n.abs());
    let (whole, frac) = text.split_once('.').unwrap_or((text.as_str(), ""));
    let frac = if keep_zeros {
        frac.to_string()
    } else {
        frac.trim_end_matches('0').to_string()
    };
    let negative = n < 0.0 && (!whole.trim_start_matches('0').is_empty() || !frac.is_empty());
    let whole = group_digits(whole, data.group, data.min_grouping);
    let out = if frac.is_empty() {
        whole
    } else {
        format!("{whole}{}{frac}", data.decimal)
    };
    (out, negative)
}

/// `FormatNumber(number, style)` for the context's locale: `Decimal` with
/// up to three decimals, `Integer` rounded, `Percent` times 100 rounded
/// with the locale's sign, `Currency` as `FormatCurrency` with the
/// locale's currency (the documentation does not accept it; `C_Intl`'s
/// English gives dollars). English's answers are `C_Intl.FormatNumber`'s.
fn format_number_in(loc: &Locale, n: f64, style: i64) -> String {
    let data = lang_data(loc);
    match style {
        1 => {
            let (t, neg) = decimal_text(n.round(), 0, false, data);
            if neg { format!("-{t}") } else { t }
        }
        2 => {
            let (t, neg) = decimal_text((n * 100.0).round(), 0, false, data);
            let t = format!("{}{t}{}", data.percent.0, data.percent.1);
            if neg { format!("-{t}") } else { t }
        }
        3 => format_currency_in(loc, n, &default_currency(loc)),
        _ => {
            let (t, neg) = decimal_text(n, 3, false, data);
            if neg { format!("-{t}") } else { t }
        }
    }
}

fn context_format_number(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some((n, _)) = num_arg(vm, &c, 2) else {
        return Ok(vec![]);
    };
    let style = num_arg(vm, &c, 3).map_or(0, |s| s.0 as i64);
    let loc = locale_of(vm, id);
    let text = format_number_in(&loc, n, style);
    text_result(vm, &c, &text)
}

/// The currency of a locale's region (the likely region's without one).
fn default_currency(loc: &Locale) -> String {
    let region = if loc.region.is_empty() {
        LIKELY
            .iter()
            .find(|l| l.0 == loc.language)
            .map_or("US", |l| l.2)
    } else {
        loc.region.as_str()
    };
    match region {
        "GB" => "GBP",
        "DE" | "FR" | "ES" | "IT" | "PT" | "AT" | "BE" | "NL" | "IE" | "FI" | "GR" => "EUR",
        "MX" => "MXN",
        "BR" => "BRL",
        "RU" => "RUB",
        "KR" => "KRW",
        "CN" => "CNY",
        "TW" => "TWD",
        "JP" => "JPY",
        "CA" => "CAD",
        "AU" => "AUD",
        _ => "USD",
    }
    .to_string()
}

/// ISO 4217's minor digits (two unless listed).
fn currency_digits(code: &str) -> usize {
    match code {
        "JPY" | "KRW" | "CLP" | "ISK" | "VND" | "PYG" | "UGX" => 0,
        "BHD" | "KWD" | "OMR" | "JOD" | "TND" | "IQD" | "LYD" => 3,
        _ => 2,
    }
}

/// A currency's symbol in a locale (CLDR's, as remembered; unverified):
/// style 1 the narrow symbol, else the symbol; the code itself where none
/// is known.
fn currency_symbol(loc: &Locale, code: &str, narrow: bool) -> String {
    let lang = loc.language.as_str();
    if narrow {
        let s = match code {
            "USD" | "CAD" | "AUD" | "MXN" | "TWD" => "$",
            "EUR" => "€",
            "GBP" => "£",
            "JPY" | "CNY" => "¥",
            "KRW" => "₩",
            "RUB" => "₽",
            "BRL" => "R$",
            _ => code,
        };
        return s.to_string();
    }
    let s = match (code, lang) {
        ("USD", "en" | "de" | "ru") => "$",
        ("USD", "fr") => "$US",
        ("USD", "it") => "USD",
        ("USD", _) => "US$",
        ("EUR", _) => "€",
        ("GBP", _) => "£",
        ("JPY", "zh" | "ko") => "JP¥",
        ("JPY", _) => "¥",
        ("KRW", _) => "₩",
        ("CNY", "zh") if !loc.traditional() => "¥",
        ("CNY", _) => "CN¥",
        ("TWD", "zh") if loc.traditional() => "$",
        ("TWD", _) => "NT$",
        ("RUB", "ru") => "₽",
        ("BRL", _) => "R$",
        ("MXN", "es") if loc.region == "MX" => "$",
        ("MXN", _) => "MX$",
        ("CAD", _) => "CA$",
        ("AUD", _) => "A$",
        _ => code,
    };
    s.to_string()
}

/// A currency's English name (others: the code, the documentation's
/// answer "if no localized name is available").
fn currency_long_name(loc: &Locale, code: &str) -> String {
    if loc.language != "en" {
        return code.to_string();
    }
    let s = match code {
        "USD" => "US Dollar",
        "EUR" => "Euro",
        "GBP" => "British Pound",
        "JPY" => "Japanese Yen",
        "KRW" => "South Korean Won",
        "CNY" => "Chinese Yuan",
        "TWD" => "New Taiwan Dollar",
        "RUB" => "Russian Ruble",
        "BRL" => "Brazilian Real",
        "MXN" => "Mexican Peso",
        "CAD" => "Canadian Dollar",
        "AUD" => "Australian Dollar",
        _ => code,
    };
    s.to_string()
}

/// An amount in a currency: its minor digits, the locale's separators,
/// the symbol before (`$1,234.56`, `-$1.00`; a code spaced) or after
/// (`1.234,56 €`) as the locale writes it.
fn format_currency_in(loc: &Locale, n: f64, code: &str) -> String {
    let data = lang_data(loc);
    let code = code.to_ascii_uppercase();
    let (amount, negative) = decimal_text(n, currency_digits(&code), true, data);
    let symbol = currency_symbol(loc, &code, false);
    let sign = if negative { "-" } else { "" };
    if data.currency_first {
        let space = if symbol.chars().all(|ch| ch.is_ascii_uppercase()) || loc.language == "pt" {
            "\u{A0}"
        } else {
            ""
        };
        format!("{sign}{symbol}{space}{amount}")
    } else {
        format!("{sign}{amount}\u{A0}{symbol}")
    }
}

/// `FormatCurrency(number, currencyCode)`.
fn context_format_currency(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (Some((n, _)), Some((code, _))) = (num_arg(vm, &c, 2), text_arg(vm, &c, 3)) else {
        return Ok(vec![]);
    };
    if code.len() != 3 {
        return Ok(vec![]);
    }
    let loc = locale_of(vm, id);
    let text = format_currency_in(&loc, n, &code);
    text_result(vm, &c, &text)
}

/// `GetCurrencyName(currencyCode, style)`: `Enum.CurrencyNameStyle`
/// Symbol 0, NarrowSymbol 1, Long 2, FormalSymbol 3 and VariantSymbol 4
/// (both the symbol here; unverified).
fn context_currency_name(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some((code, _)) = text_arg(vm, &c, 2) else {
        return Ok(vec![]);
    };
    let code = code.to_ascii_uppercase();
    let style = num_arg(vm, &c, 3).map_or(0, |s| s.0 as i64);
    let loc = locale_of(vm, id);
    let text = match style {
        1 => currency_symbol(&loc, &code, true),
        2 => currency_long_name(&loc, &code),
        _ => currency_symbol(&loc, &code, false),
    };
    text_result(vm, &c, &text)
}

/// The number a whole localized text is (its group separators, any
/// space, dropped; its decimal separator a point), none when anything is
/// left over.
fn parse_localized(text: &str, data: &LangData) -> Option<f64> {
    let mut s: String = text
        .trim()
        .chars()
        .filter(|ch| !matches!(ch, ' ' | '\u{A0}' | '\u{202F}'))
        .collect();
    if !data.group.trim().is_empty() {
        s = s.replace(data.group, "");
    }
    if data.decimal != "." {
        s = s.replace(data.decimal, ".");
    }
    let s = s.replace('\u{2212}', "-");
    if s.is_empty()
        || !s
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+'))
    {
        return None;
    }
    s.parse::<f64>().ok()
}

/// `ParseNumber(text, style)`: the whole text read as the locale writes
/// numbers (`Percent` with its sign, divided by 100); nothing for text
/// that is not one, or for `Currency` (the documentation: "use
/// ParseCurrency instead").
fn context_parse_number(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some((text, _)) = text_arg(vm, &c, 2) else {
        return Ok(vec![]);
    };
    let style = num_arg(vm, &c, 3).map_or(0, |s| s.0 as i64);
    let loc = locale_of(vm, id);
    let data = lang_data(&loc);
    let n = match style {
        3 => None,
        2 => text
            .trim()
            .strip_suffix('%')
            .or_else(|| text.trim().strip_prefix('%'))
            .and_then(|t| parse_localized(t, data))
            .map(|n| n / 100.0),
        _ => parse_localized(&text, data),
    };
    Ok(match n {
        Some(n) => vec![hide(vm, any_secret(&c), Value::Num(n))],
        None => vec![],
    })
}

/// The currency a symbol or code in a localized amount names.
fn currency_of_symbol(loc: &Locale, symbol: &str) -> Option<String> {
    let code = match symbol {
        "$" if loc.language == "zh" && loc.traditional() => "TWD",
        "$" if loc.language == "es" && loc.region == "MX" => "MXN",
        "$" | "US$" | "$US" => "USD",
        "€" => "EUR",
        "£" => "GBP",
        "¥" if loc.language == "zh" && !loc.traditional() => "CNY",
        "¥" | "JP¥" => "JPY",
        "CN¥" => "CNY",
        "₩" => "KRW",
        "₽" => "RUB",
        "R$" => "BRL",
        "MX$" => "MXN",
        "CA$" => "CAD",
        "A$" => "AUD",
        "NT$" => "TWD",
        s if s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase()) => s,
        _ => return None,
    };
    Some(code.to_string())
}

/// `ParseCurrency(text)`: the amount and the currency its symbol (or
/// code) names, before or after the number; nothing when either is
/// missing.
fn context_parse_currency(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some((text, secret)) = text_arg(vm, &c, 2) else {
        return Ok(vec![]);
    };
    let loc = locale_of(vm, id);
    let data = lang_data(&loc);
    let text = text.trim().replace(['\u{A0}', '\u{202F}'], " ");
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest.trim().to_string()),
        None => (false, text.clone()),
    };
    let is_number = |ch: char| ch.is_ascii_digit() || matches!(ch, '.' | ',' | ' ' | '\'');
    let start = body.find(|ch: char| ch.is_ascii_digit());
    let Some(start) = start else {
        return Ok(vec![]);
    };
    let end = body[start..]
        .find(|ch: char| !is_number(ch))
        .map_or(body.len(), |e| start + e);
    let number = body[start..end].trim();
    let symbol = format!("{}{}", body[..start].trim(), body[end..].trim());
    let symbol = symbol.trim().trim_start_matches('-');
    let (Some(amount), Some(code)) = (
        parse_localized(number, data),
        currency_of_symbol(&loc, symbol),
    ) else {
        return Ok(vec![]);
    };
    let amount = if negative { -amount } else { amount };
    let t = vm.new_table();
    let a = hide(vm, secret, Value::Num(amount));
    set_field(vm, t, "amount", a);
    let code = vm.str(&code);
    let code = hide(vm, secret, code);
    set_field(vm, t, "currencyCode", code);
    Ok(vec![Value::Table(t)])
}

/// `SelectPlural(number, pluralType)`: CLDR's plural rules for WoW's
/// languages (`Enum.PluralType` Cardinal 0, Ordinal 1), the operands `i`
/// (the whole part) and `v` (whether it has decimals), as remembered
/// (unverified; the "many" of French and Spanish millions left out).
fn select_plural(loc: &Locale, n: f64, ordinal: bool) -> &'static str {
    let n = n.abs();
    let has_fraction = n.fract() != 0.0;
    let i = n.trunc() as u64;
    let (m10, m100) = (i % 10, i % 100);
    let lang = loc.language.as_str();
    if ordinal {
        return match lang {
            "en" if has_fraction => "other",
            "en" if m10 == 1 && m100 != 11 => "one",
            "en" if m10 == 2 && m100 != 12 => "two",
            "en" if m10 == 3 && m100 != 13 => "few",
            "fr" if !has_fraction && i == 1 => "one",
            "it" if !has_fraction && matches!(i, 8 | 11 | 80 | 800) => "many",
            _ => "other",
        };
    }
    match lang {
        "en" | "de" | "it" if i == 1 && !has_fraction => "one",
        "es" if n == 1.0 => "one",
        "fr" | "pt" if i <= 1 => "one",
        "ru" if has_fraction => "other",
        "ru" if m10 == 1 && m100 != 11 => "one",
        "ru" if (2..=4).contains(&m10) && !(12..=14).contains(&m100) => "few",
        "ru" => "many",
        _ => "other",
    }
}

fn context_select_plural(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some((n, _)) = num_arg(vm, &c, 2) else {
        return Ok(vec![]);
    };
    let ordinal = num_arg(vm, &c, 3).is_some_and(|t| t.0 == 1.0);
    let loc = locale_of(vm, id);
    let text = select_plural(&loc, n, ordinal);
    text_result(vm, &c, text)
}

// ------------------------------------------------------------- dates

/// A day count since 1970-01-01 as (year, month, day) (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A moment's local fields in a time zone.
struct Moment {
    year: i64,
    month: u32,
    day: u32,
    weekday: usize,
    hour: u32,
    minute: u32,
    second: u32,
    /// The zone's offset from UTC, in seconds.
    offset: i64,
}

fn moment(unix: f64, offset: i64) -> Moment {
    let t = unix.floor() as i64 + offset;
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Moment {
        year,
        month,
        day,
        // 1970-01-01 was a Thursday.
        weekday: (days + 4).rem_euclid(7) as usize,
        hour: (secs / 3600) as u32,
        minute: (secs % 3600 / 60) as u32,
        second: (secs % 60) as u32,
        offset,
    }
}

/// A time zone ID's offset: empty for the formatter's default (the
/// engine's local time, `date`'s), UTC and GMT and their `Etc/` names 0,
/// `GMT+2`, `UTC-05:30` as written, `Etc/GMT+5` POSIX's way round; an
/// ID the engine has no rules for, UTC (ICU's "unknown" zone;
/// unverified).
fn zone_offset(vm: &Vm, zone: &str) -> i64 {
    let zone = zone.trim();
    if zone.is_empty() {
        return vm.clock.utc_offset;
    }
    let (rest, posix) = match zone.strip_prefix("Etc/") {
        Some(r) => (r, true),
        None => (zone, false),
    };
    let rest = rest
        .strip_prefix("GMT")
        .or_else(|| rest.strip_prefix("UTC"))
        .or_else(|| rest.strip_prefix("UT"))
        .or_else(|| (rest == "Z").then_some(""));
    let Some(rest) = rest else {
        return 0;
    };
    if rest.is_empty() {
        return 0;
    }
    let (sign, digits) = match rest.as_bytes()[0] {
        b'+' => (1, &rest[1..]),
        b'-' => (-1, &rest[1..]),
        _ => return 0,
    };
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) => (h.parse::<i64>().unwrap_or(0), m.parse::<i64>().unwrap_or(0)),
        None if digits.len() > 2 => (
            digits[..digits.len() - 2].parse::<i64>().unwrap_or(0),
            digits[digits.len() - 2..].parse::<i64>().unwrap_or(0),
        ),
        None => (digits.parse::<i64>().unwrap_or(0), 0),
    };
    let offset = sign * (h * 3600 + m * 60);
    if posix { -offset } else { offset }
}

/// A zone's name: `z` short (`UTC`, `GMT+2`), `zzzz` long (the language's
/// UTC, `GMT+02:00`), ICU's localized GMT forms (unverified).
fn zone_name(data: &LangData, offset: i64, long: bool) -> String {
    if offset == 0 {
        return if long {
            data.utc.to_string()
        } else {
            "UTC".to_string()
        };
    }
    let sign = if offset < 0 { '-' } else { '+' };
    let (h, m) = (offset.abs() / 3600, offset.abs() % 3600 / 60);
    if long {
        format!("GMT{sign}{h:02}:{m:02}")
    } else if m == 0 {
        format!("GMT{sign}{h}")
    } else {
        format!("GMT{sign}{h}:{m:02}")
    }
}

/// A moment written by an ICU pattern: `y` `yy` `M`…`MMMM` `d` `dd` `EEEE`
/// `h` `hh` `H` `HH` `m` `mm` `s` `ss` `a` `z` `zzzz`, quoted literals.
fn render(pattern: &str, m: &Moment, data: &LangData) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\'' {
            // A quoted literal; '' is a quote.
            if chars.get(i + 1) == Some(&'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                out.push(chars[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        if !ch.is_ascii_alphabetic() {
            out.push(ch);
            i += 1;
            continue;
        }
        let mut n = 1;
        while chars.get(i + n) == Some(&ch) {
            n += 1;
        }
        i += n;
        let hour12 = match m.hour % 12 {
            0 => 12,
            h => h,
        };
        let text = match (ch, n) {
            ('y', 2) => format!("{:02}", m.year.rem_euclid(100)),
            ('y', _) => m.year.to_string(),
            ('M', 1) => m.month.to_string(),
            ('M', 2) => format!("{:02}", m.month),
            ('M', 3) => data.months_short[m.month as usize - 1].to_string(),
            ('M', _) => data.months[m.month as usize - 1].to_string(),
            ('d', 1) => m.day.to_string(),
            ('d', _) => format!("{:02}", m.day),
            ('E', _) => data.weekdays[m.weekday].to_string(),
            ('h', 1) => hour12.to_string(),
            ('h', _) => format!("{hour12:02}"),
            ('H', 1) => m.hour.to_string(),
            ('H', _) => format!("{:02}", m.hour),
            ('m', 1) => m.minute.to_string(),
            ('m', _) => format!("{:02}", m.minute),
            ('s', 1) => m.second.to_string(),
            ('s', _) => format!("{:02}", m.second),
            ('a', _) => data.am_pm[usize::from(m.hour >= 12)].to_string(),
            ('z', n) if n >= 4 => zone_name(data, m.offset, true),
            ('z', _) => zone_name(data, m.offset, false),
            _ => String::new(),
        };
        out.push_str(&text);
    }
    out
}

/// A moment's date and time by `Enum.DateTimeStyle` (None 0, Short 1,
/// Medium 2, Long 3, Full 4), joined as the locale joins them; empty when
/// both are None (unverified).
fn format_moment(data: &LangData, m: &Moment, date: i64, time: i64) -> String {
    let date_text = (1..=4)
        .contains(&date)
        .then(|| render(data.dates[date as usize - 1], m, data));
    let time_text = (1..=4)
        .contains(&time)
        .then(|| render(data.times[time as usize - 1], m, data));
    match (date_text, time_text) {
        (Some(d), Some(t)) => {
            let glue = if date >= 3 {
                data.glue_long
            } else {
                data.glue_short
            };
            // The glue is a pattern of its own: its quoted words literal.
            let glue = render(glue, m, data);
            glue.replace("{1}", &d).replace("{0}", &t)
        }
        (Some(d), None) => d,
        (None, Some(t)) => t,
        (None, None) => String::new(),
    }
}

fn format_date_time(
    vm: &mut Vm,
    id: WidgetId,
    c: &NativeCall,
    date: i64,
    time: i64,
    zone: usize,
) -> Ret {
    let Some((unix, _)) = num_arg(vm, c, 2) else {
        return Ok(vec![]);
    };
    let zone = text_arg(vm, c, zone).map(|z| z.0).unwrap_or_default();
    let offset = zone_offset(vm, &zone);
    let loc = locale_of(vm, id);
    let text = format_moment(lang_data(&loc), &moment(unix, offset), date, time);
    text_result(vm, c, &text)
}

/// `FormatDate(unixTimeSeconds, style, timeZone)`.
fn context_format_date(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let style = num_arg(vm, &c, 3).map_or(0, |s| s.0 as i64);
    format_date_time(vm, id, &c, style, 0, 4)
}

/// `FormatTime(unixTimeSeconds, style, timeZone)`.
fn context_format_time(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let style = num_arg(vm, &c, 3).map_or(0, |s| s.0 as i64);
    format_date_time(vm, id, &c, 0, style, 4)
}

/// `FormatDateTime(unixTimeSeconds, dateStyle, timeStyle, timeZone)`.
fn context_format_date_time(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let date = num_arg(vm, &c, 3).map_or(0, |s| s.0 as i64);
    let time = num_arg(vm, &c, 4).map_or(0, |s| s.0 as i64);
    format_date_time(vm, id, &c, date, time, 5)
}

// ----------------------------------------------------- locale names

/// WoW's languages' names in English and in themselves.
const LANGUAGE_NAMES: [(&str, &str, &str); 9] = [
    ("en", "English", "English"),
    ("de", "German", "Deutsch"),
    ("fr", "French", "français"),
    ("es", "Spanish", "español"),
    ("it", "Italian", "italiano"),
    ("pt", "Portuguese", "português"),
    ("ru", "Russian", "русский"),
    ("ko", "Korean", "한국어"),
    ("zh", "Chinese", "中文"),
];

/// Regions' names in English, and in their own languages.
const REGION_NAMES: [(&str, &str, &str); 13] = [
    ("US", "United States", "United States"),
    ("GB", "United Kingdom", "United Kingdom"),
    ("DE", "Germany", "Deutschland"),
    ("FR", "France", "France"),
    ("ES", "Spain", "España"),
    ("MX", "Mexico", "México"),
    ("IT", "Italy", "Italia"),
    ("BR", "Brazil", "Brasil"),
    ("PT", "Portugal", "Portugal"),
    ("RU", "Russia", "Россия"),
    ("KR", "South Korea", "대한민국"),
    ("CN", "China", "中国"),
    ("TW", "Taiwan", "台灣"),
];

/// `GetDisplayName(displayLocale)`: the context's locale named in the
/// display locale's language: English's names, or the locale's own for
/// its own language (`Deutsch (Deutschland)`, `한국어(대한민국)`, `中文（中国）`);
/// for another language, English's (unverified: no data here). CLDR's
/// names, as remembered.
fn context_display_name(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Some((display, _)) = text_arg(vm, &c, 2) else {
        return Ok(vec![]);
    };
    let display = Locale::parse(&display);
    let loc = locale_of(vm, id);
    let own = display.language == loc.language && display.language != "en";
    let pick = |e: &str, n: &str| if own { n.to_string() } else { e.to_string() };
    let language = LANGUAGE_NAMES
        .iter()
        .find(|l| l.0 == loc.language)
        .map_or_else(|| loc.language.clone(), |l| pick(l.1, l.2));
    let region = REGION_NAMES
        .iter()
        .find(|r| r.0 == loc.region)
        .map(|r| pick(r.1, r.2))
        .or_else(|| (!loc.region.is_empty()).then(|| loc.region.clone()));
    let text = match region {
        None => language,
        Some(r) if own && loc.language == "zh" => format!("{language}（{r}）"),
        Some(r) if own && loc.language == "ko" => format!("{language}({r})"),
        Some(r) => format!("{language} ({r})"),
    };
    text_result(vm, &c, &text)
}

/// `TransformLocale(transform)`: `Enum.LocaleTransform` Canonicalize 0,
/// AddLikelySubtags 1, RemoveLikelySubtags 2, Language 3, Script 4,
/// Region 5, Variant 6, ParentLocale 7, after ICU's `uloc_` functions
/// and CLDR's likely subtags for WoW's languages (unverified).
fn transform_locale(loc: &Locale, transform: i64) -> String {
    let likely = LIKELY.iter().find(|l| l.0 == loc.language);
    match transform {
        1 => {
            let mut out = loc.clone();
            if let Some(&(_, script, region)) = likely {
                if out.script.is_empty() {
                    out.script = if loc.traditional() {
                        "Hant".into()
                    } else {
                        script.into()
                    };
                }
                if out.region.is_empty() {
                    out.region = if out.script == "Hant" {
                        "TW".into()
                    } else {
                        region.into()
                    };
                }
            }
            out.id()
        }
        2 => {
            let mut out = loc.clone();
            if let Some(&(_, script, region)) = likely {
                let traditional = loc.traditional();
                if out.script == script
                    || (traditional && out.script == "Hant" && out.region == "TW")
                {
                    out.script.clear();
                }
                if out.region == region && !traditional {
                    out.region.clear();
                }
                if traditional {
                    out.script.clear();
                    if out.region.is_empty() {
                        out.region = "TW".into();
                    }
                }
            }
            out.id()
        }
        3 => loc.language.clone(),
        4 => loc.script.clone(),
        5 => loc.region.clone(),
        6 => loc.variant.clone(),
        7 => {
            let id = loc.id();
            match id.rfind('_') {
                Some(i) => id[..i].trim_end_matches('_').to_string(),
                None => String::new(),
            }
        }
        _ => loc.id(),
    }
}

fn context_transform_locale(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let transform = num_arg(vm, &c, 2).map_or(0, |t| t.0 as i64);
    let loc = locale_of(vm, id);
    let text = transform_locale(&loc, transform);
    text_result(vm, &c, &text)
}

// --------------------------------------------------- breaks and search

/// A combining mark or joiner: part of the character before it.
fn extends(ch: char) -> bool {
    matches!(ch as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0x200C | 0x200D)
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

/// The boundaries of `text` (byte offsets, its start and end included) by
/// `Enum.BreakType`: Character 0 (code points with their combining marks,
/// CR LF together), Word 1 (runs of letters and digits, with an
/// apostrophe or point inside a word or number, runs of spaces, every
/// other character alone), Sentence 2 (after `.`, `?`, `!` with closing
/// quotes and the spaces after, unless a small letter follows), Line 3
/// (after spaces, after a hyphen inside a word, between CJK characters).
/// Simplified from Unicode's segmentation rules (unverified against ICU).
fn breaks(text: &str, kind: i64) -> Vec<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = vec![0];
    let at = |i: usize| chars.get(i).map(|c| c.1);
    for i in 1..chars.len() {
        let (prev, cur) = (chars[i - 1].1, chars[i].1);
        let next = at(i + 1);
        let boundary = match kind {
            0 => !(extends(cur) || prev == '\u{200D}' || (prev == '\r' && cur == '\n')),
            1 => {
                if extends(cur) {
                    false
                } else {
                    let word = |c: char| c.is_alphanumeric() || c == '_';
                    let mid = |c: char| matches!(c, '\'' | '’' | '.' | ',' | ':');
                    let before = if i >= 2 { Some(chars[i - 2].1) } else { None };
                    if word(prev) && word(cur) {
                        is_cjk(prev) || is_cjk(cur)
                    } else if (word(prev) && mid(cur) && next.is_some_and(word))
                        || (mid(prev) && word(cur) && before.is_some_and(word))
                    {
                        false
                    } else if prev.is_whitespace() && cur.is_whitespace() {
                        prev == '\n' || cur == '\n'
                    } else {
                        true
                    }
                }
            }
            2 => {
                // After the terminator, its closers and spaces.
                let mut j = i;
                while j > 0 && chars[j - 1].1.is_whitespace() {
                    j -= 1;
                }
                let spaced = j < i;
                while j > 0 && matches!(chars[j - 1].1, '"' | '\'' | ')' | ']' | '»' | '”' | '’')
                {
                    j -= 1;
                }
                let ended = j > 0 && matches!(chars[j - 1].1, '.' | '?' | '!' | '。' | '？' | '！');
                let cjk_end = j > 0 && matches!(chars[j - 1].1, '。' | '？' | '！');
                ended
                    && !cur.is_whitespace()
                    && (spaced || cjk_end)
                    && !(chars[j - 1].1 == '.' && cur.is_lowercase())
            }
            _ => {
                if prev.is_whitespace() {
                    !cur.is_whitespace()
                } else if prev == '-' {
                    i >= 2 && chars[i - 2].1.is_alphabetic() && cur.is_alphabetic()
                } else {
                    (is_cjk(prev) || is_cjk(cur)) && !cur.is_whitespace() && !extends(cur)
                }
            }
        };
        if boundary {
            out.push(chars[i].0);
        }
    }
    if !text.is_empty() {
        out.push(text.len());
    }
    out
}

/// `FindBreaks(text, breakType)`: the boundaries as Lua's indexes (1 for
/// the start, the length plus one for the end); nothing for text that is
/// not UTF-8.
fn context_find_breaks(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    let Value::Str(s) = vm.hidden(c.arg(2)) else {
        return Ok(vec![]);
    };
    let Ok(text) = std::str::from_utf8(vm.str_bytes(s)).map(str::to_string) else {
        return Ok(vec![]);
    };
    let kind = num_arg(vm, &c, 3).map_or(0, |k| k.0 as i64);
    let offsets: Vec<f64> = breaks(&text, kind)
        .into_iter()
        .map(|b| b as f64 + 1.0)
        .collect();
    Ok(vec![number_list(vm, &offsets)])
}

/// A character as the collation strength compares it: Primary and
/// Secondary ignore case (as `C_Intl.CompareStrings`), stronger ones not.
fn collate(ch: char, strength: i64) -> String {
    if strength < 2 {
        ch.to_lowercase().collect()
    } else {
        ch.to_string()
    }
}

/// `FindStringMatches(text, pattern, strength)`: where the pattern starts
/// in the text, as Lua's indexes, matches not overlapping (unverified:
/// ICU's string search's options); nothing for text that is not UTF-8.
fn context_find_matches(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    let (Value::Str(t), Value::Str(p)) = (vm.hidden(c.arg(2)), vm.hidden(c.arg(3))) else {
        return Ok(vec![]);
    };
    let (Ok(text), Ok(pattern)) = (
        std::str::from_utf8(vm.str_bytes(t)).map(str::to_string),
        std::str::from_utf8(vm.str_bytes(p)).map(str::to_string),
    ) else {
        return Ok(vec![]);
    };
    let strength = num_arg(vm, &c, 4).map_or(2, |s| s.0 as i64);
    let want: String = pattern.chars().map(|ch| collate(ch, strength)).collect();
    let chars: Vec<(usize, String)> = text
        .char_indices()
        .map(|(i, ch)| (i, collate(ch, strength)))
        .collect();
    let mut found = Vec::new();
    let mut i = 0;
    while !want.is_empty() && i < chars.len() {
        let mut acc = String::new();
        let mut j = i;
        while j < chars.len() && acc.len() < want.len() {
            acc.push_str(&chars[j].1);
            j += 1;
        }
        if acc == want {
            found.push(chars[i].0 as f64 + 1.0);
            i = j;
        } else {
            i += 1;
        }
    }
    Ok(vec![number_list(vm, &found)])
}

/// `GetSortKey(text, strength)`: bytes that sort as the strength compares
/// (unverified: ICU's keys are its own): the text in small letters; from
/// Tertiary, then each character's case (small first); from Quaternary,
/// then the text itself; levels split by a 0x01 byte.
fn context_sort_key(vm: &mut Vm, _id: WidgetId, c: NativeCall) -> Ret {
    let Some((text, secret)) = text_arg(vm, &c, 2) else {
        return Ok(vec![]);
    };
    let strength = num_arg(vm, &c, 3).map_or(2, |s| s.0 as i64);
    let mut key: Vec<u8> = text.to_lowercase().into_bytes();
    if strength >= 2 {
        key.push(1);
        key.extend(
            text.chars()
                .map(|ch| if ch.is_uppercase() { b'2' } else { b'1' }),
        );
    }
    if strength >= 3 {
        key.push(1);
        key.extend_from_slice(text.as_bytes());
    }
    key.retain(|&b| b != 0);
    let v = vm.bytes(&key);
    Ok(vec![hide(vm, secret, v)])
}

// ------------------------------------------- heal prediction values

/// The calculator class (`healcalc.rs` answers its other methods).
const HEAL_CALCULATOR: &str = "UnitHealPredictionCalculator";

/// `UnitHealPredictionValues`' fields, and where the calculator keeps
/// each, where `healcalc` reads them.
const PREDICTED: [(&str, &str); 6] = [
    ("health", "\u{0}health"),
    ("healthMax", "\u{0}healthMax"),
    ("totalIncomingHeals", "\u{0}totalIncomingHeals"),
    (
        "totalIncomingHealsFromHealer",
        "\u{0}totalIncomingHealsFromHealer",
    ),
    ("totalDamageAbsorbs", "\u{0}totalDamageAbsorbs"),
    ("totalHealAbsorbs", "\u{0}totalHealAbsorbs"),
];

/// The modes `healcalc`'s getters read (their setters' stored values).
const HEAL_MODES: [&str; 6] = [
    "MaximumHealthMode",
    "DamageAbsorbClampMode",
    "HealAbsorbClampMode",
    "HealAbsorbMode",
    "IncomingHealClampMode",
    "IncomingHealOverflowPercent",
];

fn kept_number(vm: &mut Vm, id: WidgetId, key: &str) -> f64 {
    let props = engine(vm).world.widgets[id.0].props;
    match widgets::stored_args(vm, props, key).and_then(|v| v.first().copied()) {
        Some(Value::Num(n)) => n,
        _ => 0.0,
    }
}

/// Whether a unit (or a secret) filled the calculator: its values are
/// secrets then (`healcalc`'s flag).
fn heal_filled(vm: &mut Vm, id: WidgetId) -> bool {
    let props = engine(vm).world.widgets[id.0].props;
    widgets::stored_args(vm, props, "\u{0}filled").is_some()
}

fn clear_prop(vm: &mut Vm, id: WidgetId, key: &str) {
    set_prop(vm, id, key, Value::Nil);
}

/// `GetPredictedValues()`: "the raw total figures used for data
/// calculations" (the documentation): what the unit or `SetPredictedValues`
/// set, 0 for the rest, secret as the calculator's amounts are.
fn get_predicted_values(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let secret = heal_filled(vm, id);
    let t = vm.new_table();
    for (name, key) in PREDICTED {
        let n = kept_number(vm, id, key);
        let v = hide(vm, secret, Value::Num(n));
        set_field(vm, t, name, v);
    }
    Ok(vec![Value::Table(t)])
}

/// `SetPredictedValues(predictedValues)`: "sets the healing values used for
/// all calculations" (missing fields 0, their documented default); a
/// secret among them makes the calculator's values secret (unverified).
/// `healcalc`'s getters work out their amounts from the values set here.
fn set_predicted_values(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Value::Table(t) = vm.hidden(c.arg(2)) else {
        return Err(vm.error("Usage: self:SetPredictedValues(predictedValues)"));
    };
    let mut secret = false;
    for (name, key) in PREDICTED {
        let v = field(vm, t, name);
        secret |= matches!(v, Value::Secret(_));
        let n = match vm.hidden(v) {
            Value::Num(n) => n,
            _ => 0.0,
        };
        widgets::store(vm, id, key, &[Value::Num(n)]);
    }
    if secret {
        widgets::store(vm, id, "\u{0}filled", &[Value::Bool(true)]);
    }
    Ok(vec![])
}

fn clear_predicted(vm: &mut Vm, id: WidgetId) {
    for (_, key) in PREDICTED {
        clear_prop(vm, id, key);
    }
}

/// `ResetPredictedValues()`: "resets all stored healing values" (to 0).
fn reset_predicted_values(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    clear_predicted(vm, id);
    Ok(vec![])
}

/// `Reset()`: "resets all stored state on the object": the values and the
/// modes (back to 0, the enumerations' first); unverified: the secret
/// flag stays, which `SetToDefaults` alone is documented to clear.
fn heal_reset(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    clear_predicted(vm, id);
    for mode in HEAL_MODES {
        clear_prop(vm, id, mode);
    }
    Ok(vec![])
}

/// `SetToDefaults()`: "resets all state on the object, and clears the
/// secret values flag".
fn heal_to_defaults(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    heal_reset(vm, id, c)?;
    clear_prop(vm, id, "\u{0}filled");
    Ok(vec![])
}

// ------------------------------------------- housing catalog searchers

const SEARCHER: &str = "HousingCatalogSearcher";

/// `C_HousingCatalog.CreateCatalogSearcher()`: Forever has no housing, so
/// a searcher finds nothing; its settings are kept as set.
fn create_catalog_searcher(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![widgets::create(vm, SEARCHER, None, None)])
}

/// A searcher's on/off filters and their values before they are set:
/// both placements, collected and uncollected on, the "only" filters off
/// (unverified: what Blizzard's `HousingCatalogFiltersMixin` counts as
/// its default state, `Blizzard_HousingCatalogFilters.lua`).
const SEARCHER_FLAGS: [(&str, bool); 8] = [
    ("AllowedIndoors", true),
    ("AllowedOutdoors", true),
    ("BaseVariantOnly", false),
    ("Collected", true),
    ("CustomizableOnly", false),
    ("FirstAcquisitionBonusOnly", false),
    ("StoredOnly", false),
    ("Uncollected", true),
];

fn searcher_flag(vm: &mut Vm, id: WidgetId, stem: &str) -> bool {
    let key = format!("flag {stem}");
    match prop(vm, id, &key) {
        Value::Bool(b) => b,
        _ => SEARCHER_FLAGS
            .iter()
            .find(|f| f.0 == stem)
            .is_some_and(|f| f.1),
    }
}

fn set_searcher_flag(vm: &mut Vm, id: WidgetId, stem: &str, on: bool) {
    set_prop(vm, id, &format!("flag {stem}"), Value::Bool(on));
}

/// `Is<Filter>Active()`, `Set<Filter>(isActive)`, `Toggle<Filter>()`.
fn searcher_flags(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let name = crate::curves::method_name(vm, &c);
    if let Some(stem) = name
        .strip_prefix("Is")
        .and_then(|s| s.strip_suffix("Active"))
    {
        return Ok(vec![Value::Bool(searcher_flag(vm, id, stem))]);
    }
    if let Some(stem) = name.strip_prefix("Toggle") {
        let on = searcher_flag(vm, id, stem);
        set_searcher_flag(vm, id, stem, !on);
        return Ok(vec![]);
    }
    if let Some(stem) = name.strip_prefix("Set") {
        let on = vm.hidden(c.arg(2)).truthy();
        set_searcher_flag(vm, id, stem, on);
    }
    Ok(vec![])
}

fn tag_key(group: f64, tag: f64) -> String {
    format!("tag {group} {tag}")
}

/// A filter tag's status: as set, else its group's as set, else on
/// (Blizzard's filters count a tag off as changed from the default).
fn tag_status(vm: &mut Vm, id: WidgetId, group: f64, tag: f64) -> bool {
    if let Value::Bool(b) = prop(vm, id, &tag_key(group, tag)) {
        return b;
    }
    !matches!(prop(vm, id, &format!("group {group}")), Value::Bool(false))
}

fn group_tag_args(vm: &Vm, c: &NativeCall) -> (f64, f64) {
    (
        num_arg(vm, c, 2).map_or(0.0, |v| v.0),
        num_arg(vm, c, 3).map_or(0.0, |v| v.0),
    )
}

fn get_filter_tag_status(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (group, tag) = group_tag_args(vm, &c);
    Ok(vec![Value::Bool(tag_status(vm, id, group, tag))])
}

fn set_filter_tag_status(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (group, tag) = group_tag_args(vm, &c);
    let on = vm.hidden(c.arg(4)).truthy();
    set_prop(vm, id, &tag_key(group, tag), Value::Bool(on));
    Ok(vec![])
}

fn toggle_filter_tag(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (group, tag) = group_tag_args(vm, &c);
    let on = tag_status(vm, id, group, tag);
    set_prop(vm, id, &tag_key(group, tag), Value::Bool(!on));
    Ok(vec![])
}

/// `SetAllInFilterTagGroup(groupID, active)`: every tag of the group (its
/// tags set one by one before forgotten).
fn set_all_in_tag_group(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let group = num_arg(vm, &c, 2).map_or(0.0, |v| v.0);
    let on = vm.hidden(c.arg(3)).truthy();
    let props = engine(vm).world.widgets[id.0].props;
    let prefix = format!("tag {group} ");
    let keys: Vec<Value> = vm
        .heap
        .table(props)
        .entries()
        .filter_map(|(k, _)| match k {
            Value::Str(s) if vm.str_bytes(s).starts_with(prefix.as_bytes()) => Some(k),
            _ => None,
        })
        .collect();
    for k in keys {
        let _ = vm.heap.table_mut(props).set(k, Value::Nil);
    }
    set_prop(vm, id, &format!("group {group}"), Value::Bool(on));
    Ok(vec![])
}

/// No catalog: no items, no results, no search running.
fn searcher_empty_list(vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn searcher_zero(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

fn searcher_false(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `RunSearch()`: nothing to find; the results callback is not called
/// (unverified: the client might report an empty result).
fn searcher_nothing(_vm: &mut Vm, _id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// A setting's setter keeps its value for its getter.
fn searcher_set(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let name = crate::curves::method_name(vm, &c);
    let stem = name.strip_prefix("Set").unwrap_or(&name).to_string();
    set_prop(vm, id, &format!("set {stem}"), c.arg(2));
    Ok(vec![])
}

/// A setting's getter: what was set; before that nil, the sort type
/// `Enum.HousingCatalogSortType.DateAdded` (0, unverified).
fn searcher_get(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let name = crate::curves::method_name(vm, &c);
    let stem = name.strip_prefix("Get").unwrap_or(&name).to_string();
    let v = prop(vm, id, &format!("set {stem}"));
    Ok(vec![if v.is_nil() && stem == "SortType" {
        Value::Num(0.0)
    } else {
        v
    }])
}

// ----------------------------------------------------------- the lists

/// The constructors and global functions. `AbbreviateNumbers` and
/// `AbbreviateLargeNumbers` replace `queries`' (which they fall back on
/// without options): they must be chained after `queries::behaviours()`.
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_DurationUtil.CreateManualClock", create_manual_clock),
        ("C_Intl.CreateLocaleContext", create_locale_context),
        ("CreateAbbreviateConfig", create_abbreviate_config),
        ("AbbreviateNumbers", abbreviate_numbers),
        ("AbbreviateLargeNumbers", abbreviate_large_numbers),
        (
            "C_HousingCatalog.CreateCatalogSearcher",
            create_catalog_searcher,
        ),
    ]
}

/// The methods, by `Class:Method`. The `DurationObject` ones replace
/// `curves`' (which they run on the duration's clock): they must be
/// chained after `curves::methods()`.
pub fn methods() -> Vec<(&'static str, MethodFn)> {
    let mut out = named(CLOCK, &[("GetTime", clock_get_time as MethodFn)]);
    out.extend(named(
        MANUAL_CLOCK,
        &[
            ("GetTime", clock_get_time as MethodFn),
            ("AdvanceTime", advance_time),
            ("RewindTime", rewind_time),
            ("ResetTime", reset_time),
            ("SetTime", set_time),
        ],
    ));
    let durations: [(&str, MethodFn); 25] = [
        ("SetClock", duration_set_clock),
        ("GetClock", duration_get_clock),
        ("GetClockTime", duration_clock_time),
        ("Copy", duration_copy),
        ("Assign", duration_assign),
        ("SetToDefaults", duration_to_defaults),
        ("GetStartTime", duration_get),
        ("GetEndTime", duration_get),
        ("GetTotalDuration", duration_get),
        ("GetElapsedDuration", duration_get),
        ("GetRemainingDuration", duration_get),
        ("GetElapsedPercent", duration_get),
        ("GetRemainingPercent", duration_get),
        ("HasExpired", duration_get),
        ("HasStarted", duration_get),
        ("IsActive", duration_get),
        ("EvaluateElapsedDuration", duration_evaluate),
        ("EvaluateElapsedPercent", duration_evaluate),
        ("EvaluateRemainingDuration", duration_evaluate),
        ("EvaluateRemainingPercent", duration_evaluate),
        ("EvaluateTotalDuration", duration_evaluate),
        ("FormatElapsedDuration", duration_format),
        ("FormatRemainingDuration", duration_format),
        ("FormatTotalDuration", duration_format),
        ("IsZero", duration_get),
    ];
    out.extend(named("DurationObject", &durations));
    out.extend(named(
        ABBREVIATE_CONFIG,
        &[
            ("GetAbbreviateNumberData", get_abbreviate_data as MethodFn),
            ("SetAbbreviateNumberData", set_abbreviate_data),
        ],
    ));
    let contexts: [(&str, MethodFn); 22] = [
        ("CompareStrings", context_compare),
        ("FindBreaks", context_find_breaks),
        ("FindStringMatches", context_find_matches),
        ("FoldCase", context_fold_case),
        ("FormatCurrency", context_format_currency),
        ("FormatDate", context_format_date),
        ("FormatDateTime", context_format_date_time),
        ("FormatNumber", context_format_number),
        ("FormatTime", context_format_time),
        ("GetCurrencyName", context_currency_name),
        ("GetDisplayName", context_display_name),
        ("GetLocale", context_get_locale),
        ("GetSortKey", context_sort_key),
        ("Length", context_length),
        ("ParseCurrency", context_parse_currency),
        ("ParseNumber", context_parse_number),
        ("SelectPlural", context_select_plural),
        ("SetLocale", context_set_locale),
        ("ToLower", context_to_lower),
        ("ToTitle", context_to_title),
        ("ToUpper", context_to_upper),
        ("TransformLocale", context_transform_locale),
    ];
    out.extend(named(LOCALE_CONTEXT, &contexts));
    out.extend(named(
        HEAL_CALCULATOR,
        &[
            ("GetPredictedValues", get_predicted_values as MethodFn),
            ("SetPredictedValues", set_predicted_values),
            ("ResetPredictedValues", reset_predicted_values),
            ("Reset", heal_reset),
            ("SetToDefaults", heal_to_defaults),
        ],
    ));
    let mut searcher: Vec<(&str, MethodFn)> = Vec::new();
    for (stem, _) in SEARCHER_FLAGS {
        for name in [
            format!("Is{stem}Active"),
            format!("Set{stem}"),
            format!("Toggle{stem}"),
        ] {
            let name: &'static str = Box::leak(name.into_boxed_str());
            searcher.push((name, searcher_flags as MethodFn));
        }
    }
    searcher.extend([
        ("GetFilterTagStatus", get_filter_tag_status as MethodFn),
        ("SetFilterTagStatus", set_filter_tag_status),
        ("ToggleFilterTag", toggle_filter_tag),
        ("SetAllInFilterTagGroup", set_all_in_tag_group),
        ("GetAllSearchItems", searcher_empty_list),
        ("GetCatalogSearchResults", searcher_empty_list),
        ("GetNumSearchItems", searcher_zero),
        ("GetSearchCount", searcher_zero),
        ("IsSearchInProgress", searcher_false),
        ("RunSearch", searcher_nothing),
        ("SetSearchText", searcher_set),
        ("GetSearchText", searcher_get),
        ("SetSortType", searcher_set),
        ("GetSortType", searcher_get),
        ("SetEditorModeContext", searcher_set),
        ("GetEditorModeContext", searcher_get),
        ("SetFilteredCategoryID", searcher_set),
        ("GetFilteredCategoryID", searcher_get),
        ("SetFilteredSubcategoryID", searcher_set),
        ("GetFilteredSubcategoryID", searcher_get),
        ("SetResultsUpdatedCallback", searcher_set),
        ("SetAutoUpdateOnParamChanges", searcher_set),
    ]);
    out.extend(named(SEARCHER, &searcher));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_parse() {
        let l = Locale::parse("enUS");
        assert_eq!((l.language.as_str(), l.region.as_str()), ("en", "US"));
        let l = Locale::parse("zh-Hant-TW");
        assert_eq!(l.id(), "zh_Hant_TW");
        assert!(l.traditional());
        assert_eq!(transform_locale(&Locale::parse("deDE"), 1), "de_Latn_DE");
        assert_eq!(transform_locale(&Locale::parse("de_Latn_DE"), 2), "de");
        assert_eq!(transform_locale(&Locale::parse("en_US"), 7), "en");
    }

    #[test]
    fn numbers_by_locale() {
        let de = Locale::parse("deDE");
        assert_eq!(format_number_in(&de, 1234.5, 0), "1.234,5");
        assert_eq!(format_number_in(&Locale::parse("esES"), 1234.0, 1), "1234");
        assert_eq!(format_number_in(&Locale::parse("enUS"), -0.25, 2), "-25%");
        assert_eq!(
            format_currency_in(&Locale::parse("enUS"), 1234.5, "USD"),
            "$1,234.50"
        );
        assert_eq!(format_currency_in(&de, 1234.5, "EUR"), "1.234,50\u{A0}€");
        assert_eq!(parse_localized("1.234,5", lang_data(&de)), Some(1234.5));
    }

    #[test]
    fn plurals_and_dates() {
        let en = Locale::parse("enUS");
        assert_eq!(select_plural(&en, 1.0, false), "one");
        assert_eq!(select_plural(&en, 22.0, true), "two");
        assert_eq!(select_plural(&Locale::parse("ruRU"), 3.0, false), "few");
        // 2026-10-19 15:04:05 UTC, a Monday.
        let m = moment(1_792_422_245.0, 0);
        assert_eq!((m.year, m.month, m.day, m.weekday), (2026, 10, 19, 1));
        assert_eq!(format_moment(&EN_US, &m, 2, 0), "Oct 19, 2026");
        assert_eq!(format_moment(&EN_US, &m, 1, 1), "10/19/26, 3:04\u{202F}PM");
        assert_eq!(format_moment(&DE, &m, 4, 0), "Montag, 19. Oktober 2026");
    }

    #[test]
    fn text_breaks() {
        assert_eq!(breaks("ab", 0), [0, 1, 2]);
        assert_eq!(breaks("can't stop", 1), [0, 5, 6, 10]);
        assert_eq!(breaks("Hi. Yes.", 2), [0, 4, 8]);
        assert_eq!(breaks("a b", 3), [0, 2, 3]);
    }
}
