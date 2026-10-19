//! `C_StringUtil`: the numeric formatters (`CreateNumericRuleFormatter`,
//! `CreateAbbreviatedNumberFormatter`, `CreateSecondsFormatter`), as
//! Blizzard's documentation describes them, and the plain string helpers.
//! A formatter given a secret value, or holding one, formats into a secret
//! string. What the documentation does not state is marked unverified
//! where it is decided.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{LuaError, NativeCall, TableRef, Value, Vm};

use crate::engine;
use crate::widgets::{self, WidgetId};

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

/// A number, secret or not: its value and whether it was secret.
fn number(vm: &Vm, v: Value) -> Option<(f64, bool)> {
    match vm.hidden(v) {
        Value::Num(x) => Some((x, matches!(v, Value::Secret(_)))),
        _ => None,
    }
}

fn number_arg(vm: &mut Vm, c: &NativeCall, n: usize) -> Result<(f64, bool), LuaError> {
    number(vm, c.arg(n)).ok_or_else(|| vm.error("a number expected"))
}

/// A string argument (a number as Lua writes it), secret or not.
fn text_arg(vm: &mut Vm, c: &NativeCall, n: usize) -> (Vec<u8>, bool) {
    let v = c.arg(n);
    let secret = matches!(v, Value::Secret(_));
    let bytes = match vm.hidden(v) {
        Value::Str(s) => vm.str_bytes(s).to_vec(),
        Value::Num(x) => lua_number(vm, x).into_bytes(),
        _ => Vec::new(),
    };
    (bytes, secret)
}

fn global_text(vm: &mut Vm, name: &str, fallback: &str) -> String {
    match vm.get_global(name) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => fallback.to_string(),
    }
}

/// A number as Lua's `tostring` writes it.
fn lua_number(vm: &mut Vm, n: f64) -> String {
    match vm.tostring(Value::Num(n)) {
        Ok(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => n.to_string(),
    }
}

/// `string.format(fmt, args…)` as the client's Lua does it (the format
/// string as given when it fails).
fn c_format(vm: &mut Vm, fmt: &str, args: &[f64]) -> String {
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
    call.extend(args.iter().map(|&a| Value::Num(a)));
    match vm.call(f, &call) {
        Ok(r) => match r.first() {
            Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(*s)).into_owned(),
            _ => fmt.to_string(),
        },
        Err(_) => fmt.to_string(),
    }
}

/// `x` rounded to multiples of `step` by `Enum.NumericRuleFormatRounding`
/// (Nearest 0, Up 1, Down 2; unverified: halves away from zero).
fn round_to(x: f64, step: f64, rounding: f64) -> f64 {
    if step <= 0.0 {
        return x;
    }
    let q = x / step;
    let q = match rounding as i64 {
        1 => q.ceil(),
        2 => q.floor(),
        _ => q.round(),
    };
    q * step
}

/// The formatter object `v` is, and its class.
fn formatter_of(vm: &Vm, v: Value) -> Option<(WidgetId, String)> {
    let id = widgets::widget_of(vm, v)?;
    Some((id, engine(vm).world.widgets[id.0].class.clone()))
}

/// `x` through the numeric formatter `formatter` (`FormatNumber`, and the
/// duration objects' `Format…Duration`): a string, secret when `secret`
/// or the formatter holds secrets.
pub fn format_with(vm: &mut Vm, formatter: Value, x: f64, secret: bool) -> Result<Value, LuaError> {
    let Some((id, class)) = formatter_of(vm, formatter) else {
        return Err(vm.error("a numeric formatter expected"));
    };
    let held = prop(vm, id, "secret").truthy();
    let text = if widgets::is_a(engine(vm), &class, "SecondsFormatter") {
        seconds_format(vm, id, x, None)
    } else if widgets::is_a(engine(vm), &class, "AbbreviatedNumberFormatter") {
        abbreviated_format(vm, id, x)
    } else {
        rule_format(vm, id, x)
    };
    let v = vm.str(&text);
    Ok(hide(vm, secret || held, v))
}

/// `FormatNumber(number)`, every formatter's.
fn format_number(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (x, s) = number_arg(vm, &c, 2)?;
    let formatter = widgets::value_of(engine(vm), id);
    Ok(vec![format_with(vm, formatter, x, s)?])
}

/// The same as a plain native, for the formatters whose class the
/// documentation does not make a `NumericFormatter` (Blizzard's aura
/// buttons call `FormatNumber` on any formatter they are given).
fn format_number_native(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = widgets::widget_of(vm, c.arg(1)) else {
        return Err(vm.error("Wrong object type for function - Usage: self:FormatNumber(number)"));
    };
    format_number(vm, id, c)
}

/// A formatter of `class`, with `FormatNumber` among its methods.
fn create_formatter(vm: &mut Vm, class: &str) -> (Value, Option<WidgetId>) {
    let meta = widgets::class_meta(vm, class);
    let index = vm.str("__index");
    if let Value::Table(methods) = vm.heap.table(meta).get(index) {
        let k = vm.str("FormatNumber");
        if vm.heap.table(methods).get(k).is_nil() {
            let f = vm.native("FormatNumber", format_number_native);
            let _ = vm.heap.table_mut(methods).set(k, f);
        }
    }
    let v = widgets::create(vm, class, None, None);
    let id = widgets::widget_of(vm, v);
    (v, id)
}

/// A copy of a table's fields `keys` (hidden values), and whether any was
/// secret.
fn copy_fields(vm: &mut Vm, from: TableRef, keys: &[&str]) -> (TableRef, bool) {
    let t = vm.new_table();
    let mut secret = false;
    for k in keys {
        let key = vm.str(k);
        let v = vm.heap.table(from).get(key);
        secret |= matches!(v, Value::Secret(_));
        let v = vm.hidden(v);
        if !v.is_nil() {
            let _ = vm.heap.table_mut(t).set(key, v);
        }
    }
    (t, secret)
}

/// A list's tables, in order.
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

fn num_field(vm: &mut Vm, t: TableRef, key: &str) -> Option<f64> {
    let k = vm.str(key);
    match vm.heap.table(t).get(k) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

fn str_field(vm: &mut Vm, t: TableRef, key: &str) -> Option<String> {
    let k = vm.str(key);
    match vm.heap.table(t).get(k) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        Value::Num(n) => Some(lua_number(vm, n)),
        _ => None,
    }
}

/// The formatter's stored breakpoints (copies), made on first use.
fn breakpoints(vm: &mut Vm, id: WidgetId) -> TableRef {
    if let Value::Table(t) = prop(vm, id, "breakpoints") {
        return t;
    }
    let t = vm.new_table();
    set_prop(vm, id, "breakpoints", Value::Table(t));
    t
}

/// A copy of the stored breakpoints for Lua (`GetBreakpoints`).
fn get_breakpoints(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let list = breakpoints(vm, id);
    let out = vm.new_table();
    for (i, bp) in list_tables(vm, list).into_iter().enumerate() {
        let copy = deep_copy(vm, bp);
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num(i as f64 + 1.0), Value::Table(copy));
    }
    Ok(vec![Value::Table(out)])
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

fn clear_breakpoints(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let t = vm.new_table();
    set_prop(vm, id, "breakpoints", Value::Table(t));
    Ok(vec![])
}

/// `Copy()`: a new formatter of the same class and state.
fn copy_formatter(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let class = engine(vm).world.widgets[id.0].class.clone();
    let (v, new) = create_formatter(vm, &class);
    if let Some(new) = new {
        let from = engine(vm).world.widgets[id.0].props;
        let to = engine(vm).world.widgets[new.0].props;
        let entries: Vec<(Value, Value)> = vm.heap.table(from).entries().collect();
        for (k, val) in entries {
            let val = match val {
                Value::Table(t) => Value::Table(deep_copy(vm, t)),
                v => v,
            };
            let _ = vm.heap.table_mut(to).set(k, val);
        }
    }
    Ok(vec![v])
}

// ------------------------------------------------- numeric rule formatter

fn create_rule_formatter(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![create_formatter(vm, "NumericRuleFormatter").0])
}

/// A `NumericRuleFormatBreakpoint` copied: `threshold` and `format`
/// required, `rounding` Nearest by default (the documentation), its
/// `components` copied too.
fn rule_copy(vm: &mut Vm, v: Value) -> Result<(TableRef, bool), LuaError> {
    let Value::Table(t) = vm.hidden(v) else {
        return Err(vm.error("a breakpoint table expected"));
    };
    let (copy, mut secret) = copy_fields(
        vm,
        t,
        &["threshold", "step", "rounding", "min", "max", "format"],
    );
    if num_field(vm, copy, "threshold").is_none() {
        return Err(vm.error("a breakpoint's threshold expected"));
    }
    if str_field(vm, copy, "format").is_none() {
        return Err(vm.error("a breakpoint's format expected"));
    }
    if num_field(vm, copy, "rounding").is_none() {
        let k = vm.str("rounding");
        let _ = vm.heap.table_mut(copy).set(k, Value::Num(0.0));
    }
    let k = vm.str("components");
    if let Value::Table(list) = vm.hidden(vm.heap.table(t).get(k)) {
        let out = vm.new_table();
        for (i, comp) in list_tables(vm, list).into_iter().enumerate() {
            let (c, s) = copy_fields(vm, comp, &["div", "mod", "step", "rounding"]);
            secret |= s;
            if num_field(vm, c, "rounding").is_none() {
                let k = vm.str("rounding");
                let _ = vm.heap.table_mut(c).set(k, Value::Num(0.0));
            }
            let _ = vm
                .heap
                .table_mut(out)
                .set(Value::Num(i as f64 + 1.0), Value::Table(c));
        }
        let _ = vm.heap.table_mut(copy).set(k, Value::Table(out));
    }
    Ok((copy, secret))
}

fn push_breakpoint(vm: &mut Vm, id: WidgetId, bp: TableRef, secret: bool) {
    let list = breakpoints(vm, id);
    let n = vm.heap.table(list).border() as f64;
    let _ = vm
        .heap
        .table_mut(list)
        .set(Value::Num(n + 1.0), Value::Table(bp));
    if secret {
        set_prop(vm, id, "secret", Value::Bool(true));
    }
}

fn rule_add(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (bp, secret) = rule_copy(vm, c.arg(2))?;
    push_breakpoint(vm, id, bp, secret);
    Ok(vec![])
}

/// `SetBreakpoints(list)`: all replaced, none kept when one is invalid.
fn rule_set(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Value::Table(list) = vm.hidden(c.arg(2)) else {
        return Err(vm.error("a table of breakpoints expected"));
    };
    let mut copies = Vec::new();
    for bp in list_tables(vm, list) {
        copies.push(rule_copy(vm, Value::Table(bp))?);
    }
    let t = vm.new_table();
    set_prop(vm, id, "breakpoints", Value::Table(t));
    for (bp, secret) in copies {
        push_breakpoint(vm, id, bp, secret);
    }
    Ok(vec![])
}

/// `x` by the rule of the highest threshold at or below it (below every
/// threshold, the lowest rule's; unverified): rounded to its step, clamped
/// to its min and max, then formatted by its format string, with each
/// component's value (divided, then the remainder, then stepped) or the
/// value itself. Unverified: the remainder's sign (Lua's), halves.
fn rule_format(vm: &mut Vm, id: WidgetId, x: f64) -> String {
    let list = breakpoints(vm, id);
    let mut rules: Vec<(f64, TableRef)> = list_tables(vm, list)
        .into_iter()
        .map(|t| (num_field(vm, t, "threshold").unwrap_or(0.0), t))
        .collect();
    rules.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(&(_, rule)) = rules
        .iter()
        .rev()
        .find(|(th, _)| *th <= x)
        .or_else(|| rules.first())
    else {
        return lua_number(vm, x);
    };
    let mut v = x;
    if let Some(step) = num_field(vm, rule, "step") {
        let rounding = num_field(vm, rule, "rounding").unwrap_or(0.0);
        v = round_to(v, step, rounding);
    }
    if let Some(min) = num_field(vm, rule, "min") {
        v = v.max(min);
    }
    if let Some(max) = num_field(vm, rule, "max") {
        v = v.min(max);
    }
    let fmt = str_field(vm, rule, "format").unwrap_or_default();
    let k = vm.str("components");
    let args: Vec<f64> = match vm.heap.table(rule).get(k) {
        Value::Table(list) if vm.heap.table(list).border() > 0 => list_tables(vm, list)
            .into_iter()
            .map(|comp| {
                let mut w = v;
                if let Some(div) = num_field(vm, comp, "div").filter(|d| *d != 0.0) {
                    w /= div;
                }
                if let Some(m) = num_field(vm, comp, "mod").filter(|m| *m != 0.0) {
                    w -= (w / m).floor() * m;
                }
                if let Some(step) = num_field(vm, comp, "step") {
                    let rounding = num_field(vm, comp, "rounding").unwrap_or(0.0);
                    w = round_to(w, step, rounding);
                }
                w
            })
            .collect(),
        _ => vec![v],
    };
    c_format(vm, &fmt, &args)
}

// ---------------------------------------------- abbreviated number formatter

/// The client's breakpoints (enUS), as `AbbreviateNumbers` abbreviates
/// (recorded): for each unit, one decimal from it, whole from ten of it.
/// Unverified: their order.
fn default_breakpoints(vm: &mut Vm) -> TableRef {
    let units = [
        (1e12, "FOURTH_NUMBER_CAP_NO_SPACE"),
        (1e9, "THIRD_NUMBER_CAP_NO_SPACE"),
        (1e6, "SECOND_NUMBER_CAP_NO_SPACE"),
        (1e3, "FIRST_NUMBER_CAP_NO_SPACE"),
    ];
    let list = vm.new_table();
    let mut i = 1.0;
    for (unit, cap) in units {
        for (bp, sig, frac) in [(unit * 10.0, unit, 1.0), (unit, unit / 10.0, 10.0)] {
            let t = vm.new_table();
            let abbr = vm.str(cap);
            for (k, v) in [
                ("breakpoint", Value::Num(bp)),
                ("abbreviation", abbr),
                ("significandDivisor", Value::Num(sig)),
                ("fractionDivisor", Value::Num(frac)),
                ("abbreviationIsGlobal", Value::Bool(true)),
            ] {
                let key = vm.str(k);
                let _ = vm.heap.table_mut(t).set(key, v);
            }
            let _ = vm.heap.table_mut(list).set(Value::Num(i), Value::Table(t));
            i += 1.0;
        }
    }
    list
}

/// `C_StringUtil.GetDefaultAbbreviationBreakpoints(locale?)`: the enUS ones
/// for any locale (unverified for the others).
fn default_breakpoints_fn(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(default_breakpoints(vm))])
}

/// A new formatter starts with the client's breakpoints (unverified: as
/// `ResetBreakpoints` gives them; the documentation's example formats out
/// of the box).
fn create_abbreviated_formatter(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (v, id) = create_formatter(vm, "AbbreviatedNumberFormatter");
    if let Some(id) = id {
        let t = default_breakpoints(vm);
        set_prop(vm, id, "breakpoints", Value::Table(t));
    }
    Ok(vec![v])
}

fn reset_breakpoints(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let t = default_breakpoints(vm);
    set_prop(vm, id, "breakpoints", Value::Table(t));
    set_prop(vm, id, "secret", Value::Nil);
    Ok(vec![])
}

/// A `NumberAbbreviationBreakpoint` copied and checked (the predicate
/// `RequiresValidAbbreviationBreakpoints`, its errors after
/// `Enum.AbbreviationDataError`; unverified: the message, and "a multiple
/// of ten" read as the divisors' product).
fn abbreviation_copy(vm: &mut Vm, v: Value) -> Result<(TableRef, bool), LuaError> {
    let Value::Table(t) = vm.hidden(v) else {
        return Err(vm.error("a breakpoint table expected"));
    };
    let (copy, secret) = copy_fields(
        vm,
        t,
        &[
            "breakpoint",
            "abbreviation",
            "significandDivisor",
            "fractionDivisor",
            "abbreviationIsGlobal",
        ],
    );
    let bp = num_field(vm, copy, "breakpoint").unwrap_or(0.0);
    let sig = num_field(vm, copy, "significandDivisor").unwrap_or(0.0);
    let frac = num_field(vm, copy, "fractionDivisor").unwrap_or(0.0);
    let product = sig * frac;
    let error = if bp <= 0.0 {
        Some("InvalidBreakpoint")
    } else if sig <= 0.0 {
        Some("InvalidSignificandDivisor")
    } else if frac <= 0.0 {
        Some("InvalidFractionDivisor")
    } else if product % 10.0 != 0.0 {
        Some("NotMultipleOfTen")
    } else {
        None
    };
    if let Some(e) = error {
        return Err(vm.error(&format!("invalid abbreviation breakpoint ({e})")));
    }
    if str_field(vm, copy, "abbreviation").is_none() {
        return Err(vm.error("a breakpoint's abbreviation expected"));
    }
    let k = vm.str("abbreviationIsGlobal");
    if vm.heap.table(copy).get(k).is_nil() {
        let _ = vm.heap.table_mut(copy).set(k, Value::Bool(true));
    }
    Ok((copy, secret))
}

fn abbreviation_add(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let (bp, secret) = abbreviation_copy(vm, c.arg(2))?;
    push_breakpoint(vm, id, bp, secret);
    Ok(vec![])
}

fn abbreviation_set(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let Value::Table(list) = vm.hidden(c.arg(2)) else {
        return Err(vm.error("a table of breakpoints expected"));
    };
    let mut copies = Vec::new();
    for bp in list_tables(vm, list) {
        copies.push(abbreviation_copy(vm, Value::Table(bp))?);
    }
    let t = vm.new_table();
    set_prop(vm, id, "breakpoints", Value::Table(t));
    for (bp, secret) in copies {
        push_breakpoint(vm, id, bp, secret);
    }
    Ok(vec![])
}

/// `x` by the highest breakpoint at or below it: its whole significand
/// (`x` divided by the significand divisor, truncated) over the fraction
/// divisor, then the abbreviation (a global string's text unless
/// `abbreviationIsGlobal` is false); below every breakpoint, as Lua writes
/// it (`AbbreviateNumbers`, recorded).
fn abbreviated_format(vm: &mut Vm, id: WidgetId, x: f64) -> String {
    let list = breakpoints(vm, id);
    let mut best: Option<(f64, TableRef)> = None;
    for t in list_tables(vm, list) {
        let bp = num_field(vm, t, "breakpoint").unwrap_or(f64::INFINITY);
        if x >= bp && best.is_none_or(|(b, _)| bp > b) {
            best = Some((bp, t));
        }
    }
    let Some((_, t)) = best else {
        return lua_number(vm, x);
    };
    let sig = num_field(vm, t, "significandDivisor").unwrap_or(1.0);
    let frac = num_field(vm, t, "fractionDivisor").unwrap_or(1.0);
    let value = (x / sig).trunc() / frac;
    let name = str_field(vm, t, "abbreviation").unwrap_or_default();
    let k = vm.str("abbreviationIsGlobal");
    let global = !matches!(vm.heap.table(t).get(k), Value::Bool(false));
    let abbr = if global {
        // The client's enUS text where the global is not defined.
        let fallback = match name.as_str() {
            "FIRST_NUMBER_CAP_NO_SPACE" => "K",
            "SECOND_NUMBER_CAP_NO_SPACE" => "M",
            "THIRD_NUMBER_CAP_NO_SPACE" => "B",
            "FOURTH_NUMBER_CAP_NO_SPACE" => "T",
            other => other,
        };
        global_text(vm, &name, fallback)
    } else {
        name
    };
    format!("{}{abbr}", lua_number(vm, value))
}

// ---------------------------------------------------- seconds formatter

/// `Enum.SecondsFormatterInterval`'s seconds (Seconds 0 to Days 3).
const INTERVAL_SECONDS: [f64; 4] = [1.0, 60.0, 3600.0, 86400.0];

/// Each interval's format strings by `Enum.SecondsFormatterAbbreviation`
/// (None, Truncate, OneLetter): the global strings Blizzard's
/// `SecondsFormatterMixin` uses, the client's enUS text where they are not
/// defined.
const INTERVAL_FORMATS: [[(&str, &str); 3]; 4] = [
    [
        ("D_SECONDS", "%d |4Second:Seconds;"),
        ("SECONDS_ABBR", "%d |4Sec:Sec;"),
        ("SECOND_ONELETTER_ABBR", "%d s"),
    ],
    [
        ("D_MINUTES", "%d |4Minute:Minutes;"),
        ("MINUTES_ABBR", "%d |4Min:Min;"),
        ("MINUTE_ONELETTER_ABBR", "%d m"),
    ],
    [
        ("D_HOURS", "%d |4Hour:Hours;"),
        ("HOURS_ABBR", "%d |4Hr:Hr;"),
        ("HOUR_ONELETTER_ABBR", "%d h"),
    ],
    [
        ("D_DAYS", "%d |4Day:Days;"),
        ("DAYS_ABBR", "%d |4Day:Days;"),
        ("DAY_ONELETTER_ABBR", "%d d"),
    ],
];

/// A new formatter's settings (unverified: those of Blizzard's
/// `SecondsFormatterMixin:Init` without arguments, its forerunner in Lua:
/// no approximation, no abbreviation, two units from seconds to days,
/// rounding up, nothing lowered or stripped).
const SECONDS_DEFAULTS: [(&str, f64); 8] = [
    ("approximationSeconds", 0.0),
    ("defaultAbbreviation", 0.0),
    ("desiredUnitCount", 2.0),
    ("minInterval", 0.0),
    ("maxInterval", 3.0),
    ("millisecondsThreshold", 0.0),
    ("rounding", 0.0),
    ("stripIntervalWhitespace", 0.0),
];

fn create_seconds_formatter(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (v, id) = create_formatter(vm, "SecondsFormatter");
    if let Some(id) = id {
        seconds_defaults(vm, id);
    }
    Ok(vec![v])
}

fn seconds_defaults(vm: &mut Vm, id: WidgetId) {
    for (k, v) in SECONDS_DEFAULTS {
        set_prop(vm, id, k, Value::Num(v));
    }
    for k in [
        "canRoundUpIntervals",
        "canRoundUpLastUnit",
        "convertToLower",
        "desiredUnitCountCurve",
        "maxIntervalCurve",
        "minIntervalCurve",
        "secret",
    ] {
        set_prop(vm, id, k, Value::Nil);
    }
}

fn seconds_num(vm: &mut Vm, id: WidgetId, key: &str) -> f64 {
    match prop(vm, id, key) {
        Value::Num(n) => n,
        _ => 0.0,
    }
}

/// A setting by a curve or a value: the curve's value at `seconds` when
/// one is set (an interval or count: whole, unverified: floored).
fn evaluated(vm: &mut Vm, id: WidgetId, key: &str, seconds: f64) -> f64 {
    let curve = prop(vm, id, &format!("{key}Curve"));
    match widgets::widget_of(vm, curve) {
        Some(cid) => crate::curves::evaluate(vm, cid, seconds).floor(),
        None => seconds_num(vm, id, key),
    }
}

fn interval_at(vm: &mut Vm, id: WidgetId, key: &str, seconds: f64) -> usize {
    evaluated(vm, id, key, seconds).clamp(0.0, 3.0) as usize
}

/// An interval's format string: lowered with `SetConvertToLower`, its
/// spaces stripped with `Strip` where the locale strips them
/// (`TIME_UTIL_WHITE_SPACE_STRIPPABLE`, as Blizzard's mixin reads it) or
/// with `StripIgnoreLocale`.
fn interval_format(vm: &mut Vm, id: WidgetId, interval: usize, abbreviation: usize) -> String {
    let (name, fallback) = INTERVAL_FORMATS[interval][abbreviation.min(2)];
    let mut f = global_text(vm, name, fallback);
    if prop(vm, id, "convertToLower").truthy() {
        f = f.to_lowercase();
    }
    let strip = match seconds_num(vm, id, "stripIntervalWhitespace") as i64 {
        1 => !matches!(
            vm.get_global("TIME_UTIL_WHITE_SPACE_STRIPPABLE"),
            Value::Bool(false)
        ),
        2 => true,
        _ => false,
    };
    if strip {
        f = f.replace(' ', "");
    }
    f
}

fn format_zero(vm: &mut Vm, id: WidgetId, abbreviation: usize) -> String {
    let min = interval_at(vm, id, "minInterval", 0.0);
    let f = interval_format(vm, id, min, abbreviation);
    c_format(vm, &f, &[0.0])
}

/// `Format(seconds, abbreviation?)`, after Blizzard's
/// `SecondsFormatterMixin:Format` (its forerunner in Lua): under the
/// milliseconds threshold, the seconds with one decimal (the
/// documentation's '3.4'); else whole seconds (rounded up, or truncated),
/// approximated (`< 2 Hours`) under the approximation seconds, or the
/// desired count of units from the largest interval down, each floored
/// but the last (ceiled with `SetCanRoundUpLastUnit`), promoted (`60 m` to
/// `1 h`) with `SetCanRoundUpIntervals`; zero as the smallest interval's
/// zero. Unverified: the client's own rules beyond the mixin's.
fn seconds_format(vm: &mut Vm, id: WidgetId, seconds: f64, abbreviation: Option<usize>) -> String {
    let abbreviation =
        abbreviation.unwrap_or_else(|| seconds_num(vm, id, "defaultAbbreviation") as usize);
    let ms = seconds_num(vm, id, "millisecondsThreshold");
    if seconds > 0.0 && seconds < ms {
        return c_format(vm, "%.1f", &[seconds]);
    }
    let seconds = if seconds_num(vm, id, "rounding") == 1.0 {
        seconds.floor()
    } else {
        seconds.ceil()
    };
    if seconds <= 0.0 {
        return format_zero(vm, id, abbreviation);
    }
    let min = interval_at(vm, id, "minInterval", seconds);
    let max = interval_at(vm, id, "maxInterval", seconds).max(min);
    let approx = seconds_num(vm, id, "approximationSeconds");
    if seconds > 0.0 && seconds < approx {
        let mut interval = min.max(1);
        while interval < max && seconds > INTERVAL_SECONDS[interval + 1] {
            interval += 1;
        }
        let f = interval_format(vm, id, interval, abbreviation);
        let unit = c_format(vm, &f, &[(seconds / INTERVAL_SECONDS[interval]).ceil()]);
        let less = global_text(vm, "LESS_THAN_OPERAND", "< %s");
        return less.replacen("%s", &unit, 1);
    }
    let desired = evaluated(vm, id, "desiredUnitCount", seconds).max(1.0) as usize;
    let round_last = prop(vm, id, "canRoundUpLastUnit").truthy();
    let mut units = [0.0f64; 4];
    let mut left = seconds;
    let mut appended = 0;
    let mut interval = max as i64;
    while appended < desired && interval >= min as i64 {
        let i = interval as usize;
        let size = INTERVAL_SECONDS[i];
        if left >= size {
            appended += 1;
            let q = left / size;
            units[i] = if round_last && (i == min || appended == desired) {
                q.ceil()
            } else {
                q.floor()
            };
            left %= size;
        }
        interval -= 1;
    }
    if prop(vm, id, "canRoundUpIntervals").truthy() {
        for i in 0..3 {
            if units[i] == INTERVAL_SECONDS[i + 1] / INTERVAL_SECONDS[i] && i < max {
                units[i + 1] += 1.0;
                units[i] = 0.0;
            }
        }
    }
    let delimiter = global_text(vm, "TIME_UNIT_DELIMITER", " ");
    let mut out = String::new();
    for i in (0..4).rev() {
        if units[i] > 0.0 {
            if !out.is_empty() {
                out.push_str(&delimiter);
            }
            let f = interval_format(vm, id, i, abbreviation);
            out.push_str(&c_format(vm, &f, &[units[i]]));
        }
    }
    if out.is_empty() {
        return format_zero(vm, id, abbreviation);
    }
    out
}

/// The `SecondsFormatter` methods: settings stored and read back (a
/// setting by a value or by a curve, the other cleared, the documentation:
/// a getter's value is nil while a curve is set), and the evaluations.
fn seconds_method(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let which = crate::curves::method_name(vm, &c);
    let bool_of = |v: Value| Value::Bool(v.truthy());
    let held = prop(vm, id, "secret").truthy();
    match which.as_str() {
        "Format"
        | "FormatZero"
        | "CanApproximate"
        | "EvaluateDesiredUnitCount"
        | "EvaluateMaxInterval"
        | "EvaluateMinInterval" => {
            let (x, s) = if which == "FormatZero" {
                (0.0, false)
            } else {
                number_arg(vm, &c, 2)?
            };
            let abbr_arg = if which == "FormatZero" { 2 } else { 3 };
            let abbreviation = number(vm, c.arg(abbr_arg)).map(|a| a.0.clamp(0.0, 2.0) as usize);
            let v = match which.as_str() {
                "Format" => {
                    let t = seconds_format(vm, id, x, abbreviation);
                    vm.str(&t)
                }
                "FormatZero" => {
                    let a = abbreviation
                        .unwrap_or_else(|| seconds_num(vm, id, "defaultAbbreviation") as usize);
                    let t = format_zero(vm, id, a);
                    vm.str(&t)
                }
                "CanApproximate" => {
                    let approx = seconds_num(vm, id, "approximationSeconds");
                    Value::Bool(x > 0.0 && x < approx)
                }
                "EvaluateDesiredUnitCount" => Value::Num(evaluated(vm, id, "desiredUnitCount", x)),
                "EvaluateMaxInterval" => Value::Num(interval_at(vm, id, "maxInterval", x) as f64),
                _ => Value::Num(interval_at(vm, id, "minInterval", x) as f64),
            };
            Ok(vec![hide(vm, s || held, v)])
        }
        "Reset" => {
            seconds_defaults(vm, id);
            Ok(vec![])
        }
        "CanRoundUpIntervals" => Ok(vec![bool_of(prop(vm, id, "canRoundUpIntervals"))]),
        "CanRoundUpLastUnit" => Ok(vec![bool_of(prop(vm, id, "canRoundUpLastUnit"))]),
        "GetConvertToLower" => Ok(vec![bool_of(prop(vm, id, "convertToLower"))]),
        "GetDesiredUnitCount" | "GetMaxInterval" | "GetMinInterval" => {
            let key = lower_first(&which["Get".len()..]);
            if !prop(vm, id, &format!("{key}Curve")).is_nil() {
                return Ok(vec![Value::Nil]);
            }
            Ok(vec![prop(vm, id, &key)])
        }
        "GetDesiredUnitCountCurve" | "GetMaxIntervalCurve" | "GetMinIntervalCurve" => {
            let key = lower_first(&which["Get".len()..]);
            Ok(vec![prop(vm, id, &key)])
        }
        "GetApproximationSeconds"
        | "GetDefaultAbbreviation"
        | "GetMillisecondsThreshold"
        | "GetRounding"
        | "GetStripIntervalWhitespace" => {
            let key = lower_first(&which["Get".len()..]);
            Ok(vec![prop(vm, id, &key)])
        }
        "SetCanRoundUpIntervals" | "SetCanRoundUpLastUnit" | "SetConvertToLower" => {
            let key = lower_first(&which["Set".len()..]);
            let v = vm.hidden(c.arg(2));
            set_prop(vm, id, &key, Value::Bool(v.truthy()));
            Ok(vec![])
        }
        "SetDesiredUnitCountCurve" | "SetMaxIntervalCurve" | "SetMinIntervalCurve" => {
            if widgets::widget_of(vm, c.arg(2)).is_none() {
                return Err(vm.error("a curve object expected"));
            }
            let key = lower_first(&which["Set".len()..]);
            set_prop(vm, id, &key, c.arg(2));
            Ok(vec![])
        }
        other if other.starts_with("Set") => {
            let key = lower_first(&other["Set".len()..]);
            let v = c.arg(2);
            if matches!(v, Value::Secret(_)) {
                set_prop(vm, id, "secret", Value::Bool(true));
            }
            // `SetStripIntervalWhitespace(true)`, as Blizzard's mixin took
            // it, is `Strip` (unverified).
            let v = match vm.hidden(v) {
                Value::Bool(b) => Value::Num(if b { 1.0 } else { 0.0 }),
                v => v,
            };
            set_prop(vm, id, &key, v);
            set_prop(vm, id, &format!("{key}Curve"), Value::Nil);
            Ok(vec![])
        }
        _ => Ok(vec![]),
    }
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(f) => f.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

// ------------------------------------------------- the plain helpers

/// A helper's string result: secret when an argument was.
fn text_result(vm: &mut Vm, bytes: &[u8], secret: bool) -> Ret {
    let v = vm.bytes(bytes);
    Ok(vec![hide(vm, secret, v)])
}

/// `EscapeDecimalNonPrintables(text)`: ASCII control characters but `\n`,
/// `\r`, `\t`, and bytes of invalid UTF-8, as `\` and their decimal value
/// (the documentation's `\127`; unverified: DEL counted a control).
fn escape_decimal_non_printables(vm: &mut Vm, c: NativeCall) -> Ret {
    let (text, secret) = text_arg(vm, &c, 1);
    let mut out = Vec::with_capacity(text.len());
    let mut rest = &text[..];
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                push_escaped(&mut out, valid.as_bytes());
                break;
            }
            Err(e) => {
                let (good, bad) = rest.split_at(e.valid_up_to());
                push_escaped(&mut out, good);
                let n = e.error_len().unwrap_or(bad.len());
                for b in &bad[..n] {
                    out.extend_from_slice(format!("\\{b}").as_bytes());
                }
                rest = &bad[n..];
            }
        }
    }
    text_result(vm, &out, secret)
}

fn push_escaped(out: &mut Vec<u8>, valid: &[u8]) {
    for &b in valid {
        if (b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t')) || b == 0x7f {
            out.extend_from_slice(format!("\\{b}").as_bytes());
        } else {
            out.push(b);
        }
    }
}

fn escape_lua_format_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let (text, secret) = text_arg(vm, &c, 1);
    let mut out = Vec::new();
    for b in text {
        if b == b'%' {
            out.push(b'%');
        }
        out.push(b);
    }
    text_result(vm, &out, secret)
}

fn escape_lua_patterns(vm: &mut Vm, c: NativeCall) -> Ret {
    let (text, secret) = text_arg(vm, &c, 1);
    let mut out = Vec::new();
    for b in text {
        if b"^$()%.[]*+-?".contains(&b) {
            out.push(b'%');
        }
        out.push(b);
    }
    text_result(vm, &out, secret)
}

fn escape_quoted_codes(vm: &mut Vm, c: NativeCall) -> Ret {
    let (text, secret) = text_arg(vm, &c, 1);
    let mut out = Vec::new();
    for b in text {
        if b == b'|' {
            out.push(b'|');
        }
        out.push(b);
    }
    text_result(vm, &out, secret)
}

/// `FloorToNearestString(n)`, `RoundToNearestString(n)`: the whole number
/// as text (unverified: halves round up).
fn floor_to_nearest_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let (x, s) = number_arg(vm, &c, 1)?;
    let t = lua_number(vm, x.floor());
    text_result(vm, t.as_bytes(), s)
}

fn round_to_nearest_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let (x, s) = number_arg(vm, &c, 1)?;
    let t = lua_number(vm, (x + 0.5).floor());
    text_result(vm, t.as_bytes(), s)
}

/// `TruncateWhenZero(n)`: the whole number rounded down, or "" for zero.
fn truncate_when_zero(vm: &mut Vm, c: NativeCall) -> Ret {
    let (x, s) = number_arg(vm, &c, 1)?;
    let n = x.floor();
    let text = if n == 0.0 {
        String::new()
    } else {
        lua_number(vm, n)
    };
    text_result(vm, text.as_bytes(), s)
}

/// `RemoveContiguousSpaces(text, max)`: runs of spaces cut to `max`.
fn remove_contiguous_spaces(vm: &mut Vm, c: NativeCall) -> Ret {
    let (text, secret) = text_arg(vm, &c, 1);
    let max = number(vm, c.arg(2)).map_or(1.0, |n| n.0).max(0.0) as usize;
    let mut out = Vec::new();
    let mut run = 0;
    for b in text {
        if b == b' ' {
            run += 1;
            if run > max {
                continue;
            }
        } else {
            run = 0;
        }
        out.push(b);
    }
    text_result(vm, &out, secret)
}

/// `WrapString(infix, prefix?, suffix?)`: the three joined, or "" when
/// the infix is.
fn wrap_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let (infix, s1) = text_arg(vm, &c, 1);
    let (prefix, s2) = text_arg(vm, &c, 2);
    let (suffix, s3) = text_arg(vm, &c, 3);
    let secret = s1 || s2 || s3;
    if infix.is_empty() {
        return text_result(vm, b"", secret);
    }
    let out = [prefix, infix, suffix].concat();
    text_result(vm, &out, secret)
}

/// The index after the escape sequence that ends with `|<close>` from
/// `from`, or the end.
fn skip_to(text: &[u8], from: usize, close: u8) -> usize {
    let mut i = from;
    while i + 1 < text.len() {
        if text[i] == b'|' && text[i + 1] == close {
            return i + 2;
        }
        i += 1;
    }
    text.len()
}

/// The options of `StripHyperlinks`.
#[derive(Default)]
struct Strip {
    color: bool,
    brackets: bool,
    newlines: bool,
    atlases: bool,
    textures: bool,
}

/// Hyperlinks stripped to their text: `|H…|h` and `|h` removed, the
/// brackets of a link's text too unless kept (unverified: brackets
/// outside links stay), colors (`|c…`, `|r`), atlases (`|A…|a`) and
/// textures (`|T…|t`) unless kept, `|n` with `stripNewlines`.
fn strip_links(text: &[u8], o: &Strip, only_loose_textures: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    let mut in_link = false;
    while i < text.len() {
        let b = text[i];
        if b == b'|' && i + 1 < text.len() {
            let code = text[i + 1];
            match code {
                b'|' => {
                    out.extend_from_slice(b"||");
                    i += 2;
                    continue;
                }
                b'H' if !only_loose_textures => {
                    i = skip_to(text, i + 2, b'h');
                    in_link = true;
                    continue;
                }
                b'h' if !only_loose_textures => {
                    in_link = false;
                    i += 2;
                    continue;
                }
                b'c' if !o.color && !only_loose_textures => {
                    // `|cnNAME:` or `|cAARRGGBB`.
                    if text.get(i + 2) == Some(&b'n') {
                        let end = text[i..]
                            .iter()
                            .position(|&x| x == b':')
                            .map_or(text.len(), |p| i + p + 1);
                        i = end;
                    } else {
                        i = (i + 10).min(text.len());
                    }
                    continue;
                }
                b'r' if !o.color && !only_loose_textures => {
                    i += 2;
                    continue;
                }
                b'n' if o.newlines && !only_loose_textures => {
                    i += 2;
                    continue;
                }
                b'A' if !o.atlases && !only_loose_textures => {
                    i = skip_to(text, i + 2, b'a');
                    continue;
                }
                b'T' if !o.textures => {
                    let end = skip_to(text, i + 2, b't');
                    let path_end = text[i + 2..end]
                        .iter()
                        .position(|&x| x == b':' || x == b'|')
                        .map_or(end, |p| i + 2 + p);
                    let path = &text[i + 2..path_end];
                    let loose = !path.iter().all(u8::is_ascii_digit);
                    if !only_loose_textures || loose {
                        i = end;
                        continue;
                    }
                }
                _ => {}
            }
        }
        if in_link && !o.brackets && (b == b'[' || b == b']') {
            i += 1;
            continue;
        }
        out.push(b);
        i += 1;
    }
    out
}

fn strip_hyperlinks(vm: &mut Vm, c: NativeCall) -> Ret {
    let (text, secret) = text_arg(vm, &c, 1);
    let flag = |n: usize| vm.hidden(c.arg(n)).truthy();
    let o = Strip {
        color: flag(2),
        brackets: flag(3),
        newlines: flag(4),
        atlases: flag(5),
        textures: flag(6),
    };
    let out = strip_links(&text, &o, false);
    text_result(vm, &out, secret)
}

/// `StripTextureMarkupForLooseFiles(text)`: textures by a file path
/// removed, those by a file ID kept (unverified: what "loose files" are).
fn strip_texture_markup(vm: &mut Vm, c: NativeCall) -> Ret {
    let (text, secret) = text_arg(vm, &c, 1);
    let out = strip_links(&text, &Strip::default(), true);
    text_result(vm, &out, secret)
}

pub fn behaviours() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        (
            "C_StringUtil.CreateNumericRuleFormatter",
            create_rule_formatter,
        ),
        (
            "C_StringUtil.CreateAbbreviatedNumberFormatter",
            create_abbreviated_formatter,
        ),
        (
            "C_StringUtil.CreateSecondsFormatter",
            create_seconds_formatter,
        ),
        (
            "C_StringUtil.GetDefaultAbbreviationBreakpoints",
            default_breakpoints_fn,
        ),
        (
            "C_StringUtil.EscapeDecimalNonPrintables",
            escape_decimal_non_printables,
        ),
        (
            "C_StringUtil.EscapeLuaFormatString",
            escape_lua_format_string,
        ),
        ("C_StringUtil.EscapeLuaPatterns", escape_lua_patterns),
        ("C_StringUtil.EscapeQuotedCodes", escape_quoted_codes),
        ("C_StringUtil.FloorToNearestString", floor_to_nearest_string),
        ("C_StringUtil.RoundToNearestString", round_to_nearest_string),
        ("C_StringUtil.TruncateWhenZero", truncate_when_zero),
        (
            "C_StringUtil.RemoveContiguousSpaces",
            remove_contiguous_spaces,
        ),
        ("C_StringUtil.WrapString", wrap_string),
        ("C_StringUtil.StripHyperlinks", strip_hyperlinks),
        (
            "C_StringUtil.StripTextureMarkupForLooseFiles",
            strip_texture_markup,
        ),
    ]
}

pub fn methods() -> Vec<(&'static str, widgets::MethodFn)> {
    let mut out: Vec<(&'static str, widgets::MethodFn)> = vec![
        ("NumericFormatter:FormatNumber", format_number),
        ("NumericRuleFormatter:AddBreakpoint", rule_add),
        ("NumericRuleFormatter:SetBreakpoints", rule_set),
        ("AbbreviatedNumberFormatter:AddBreakpoint", abbreviation_add),
        (
            "AbbreviatedNumberFormatter:SetBreakpoints",
            abbreviation_set,
        ),
        (
            "AbbreviatedNumberFormatter:ResetBreakpoints",
            reset_breakpoints,
        ),
    ];
    for class in ["NumericRuleFormatter", "AbbreviatedNumberFormatter"] {
        let shared: [(&str, widgets::MethodFn); 4] = [
            ("ClearBreakpoints", clear_breakpoints),
            ("Copy", copy_formatter),
            ("GetBreakpoints", get_breakpoints),
            ("FormatNumber", format_number),
        ];
        for (name, f) in shared {
            let full: &'static str = Box::leak(format!("{class}:{name}").into_boxed_str());
            out.push((full, f));
        }
    }
    for name in [
        "CanApproximate",
        "CanRoundUpIntervals",
        "CanRoundUpLastUnit",
        "EvaluateDesiredUnitCount",
        "EvaluateMaxInterval",
        "EvaluateMinInterval",
        "Format",
        "FormatZero",
        "GetApproximationSeconds",
        "GetConvertToLower",
        "GetDefaultAbbreviation",
        "GetDesiredUnitCount",
        "GetDesiredUnitCountCurve",
        "GetMaxInterval",
        "GetMaxIntervalCurve",
        "GetMillisecondsThreshold",
        "GetMinInterval",
        "GetMinIntervalCurve",
        "GetRounding",
        "GetStripIntervalWhitespace",
        "Reset",
        "SetApproximationSeconds",
        "SetCanRoundUpIntervals",
        "SetCanRoundUpLastUnit",
        "SetConvertToLower",
        "SetDefaultAbbreviation",
        "SetDesiredUnitCount",
        "SetDesiredUnitCountCurve",
        "SetMaxInterval",
        "SetMaxIntervalCurve",
        "SetMillisecondsThreshold",
        "SetMinInterval",
        "SetMinIntervalCurve",
        "SetRounding",
        "SetStripIntervalWhitespace",
        "FormatNumber",
    ] {
        let full: &'static str = Box::leak(format!("SecondsFormatter:{name}").into_boxed_str());
        let f: widgets::MethodFn = if name == "FormatNumber" {
            format_number
        } else {
            seconds_method
        };
        out.push((full, f));
    }
    out
}
