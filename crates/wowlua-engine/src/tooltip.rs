//! Tooltip lines (`GameTooltip:AddLine` and the rest, which Blizzard's
//! documentation lacks): a tooltip named `Name` shows its lines in the font
//! strings `NameTextLeftN` and `NameTextRightN` (its template makes the
//! first; the client makes more as lines are added, with the first's font).
//!
//! Unverified: the colors of lines added without one (white here), and
//! what `ClearLines` leaves in the font strings (no text, hidden).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::engine;
use crate::widgets::{self, MethodFn, WidgetId};

const COUNT: &str = "__wowluaTooltipLines";

fn count(vm: &mut Vm, id: WidgetId) -> usize {
    let props = engine(vm).world.widgets[id.0].props;
    match widgets::stored_args(vm, props, COUNT).and_then(|v| v.first().copied()) {
        Some(Value::Num(n)) => n as usize,
        _ => 0,
    }
}

fn set_count(vm: &mut Vm, id: WidgetId, n: usize) {
    widgets::store(vm, id, COUNT, &[Value::Num(n as f64)]);
}

fn call(
    vm: &mut Vm,
    obj: Value,
    method: &str,
    args: &[Value],
) -> Result<Vec<Value>, wowlua_vm::LuaError> {
    let Value::Table(t) = obj else {
        return Ok(vec![]);
    };
    let key = vm.str(method);
    let f = vm.index(Value::Table(t), key, None)?;
    let mut all = vec![obj];
    all.extend_from_slice(args);
    vm.call(f, &all)
}

/// The font string of line `n` on `side` (`Left`, `Right`), made when the
/// tooltip lacks it.
fn line(vm: &mut Vm, id: WidgetId, side: &str, n: usize) -> Option<Value> {
    let name = engine(vm).world.widgets[id.0].name.clone()?;
    let global = format!("{name}Text{side}{n}");
    let existing = {
        let g = engine(vm).main_globals;
        let k = vm.str(&global);
        vm.heap.table(g).get(k)
    };
    if !existing.is_nil() {
        return Some(existing);
    }
    let fs = widgets::create(vm, "FontString", Some(global), Some(id));
    let fs_id = widgets::widget_of(vm, fs)?;
    // The first line's font, else a default.
    let first = {
        let g = engine(vm).main_globals;
        let k = vm.str(&format!("{name}Text{side}1"));
        vm.heap.table(g).get(k)
    };
    let font = widgets::widget_of(vm, first).and_then(|f| {
        let props = engine(vm).world.widgets[f.0].props;
        widgets::stored_args(vm, props, "FontObject")
            .map(|v| ("FontObject", v))
            .or_else(|| widgets::stored_args(vm, props, "Font").map(|v| ("Font", v)))
    });
    match font {
        Some((stem, args)) => widgets::store(vm, fs_id, stem, &args),
        None => {
            let path = vm.str("Fonts\\FRIZQT__.TTF");
            widgets::store(vm, fs_id, "Font", &[path, Value::Num(12.0)]);
        }
    }
    Some(fs)
}

fn show_line(
    vm: &mut Vm,
    fs: Value,
    text: Value,
    color: &[Value],
) -> Result<(), wowlua_vm::LuaError> {
    if let Some(fid) = widgets::widget_of(vm, fs)
        && !widgets::has_font(vm, fid)
    {
        let path = vm.str("Fonts\\FRIZQT__.TTF");
        widgets::store(vm, fid, "Font", &[path, Value::Num(12.0)]);
    }
    call(vm, fs, "SetText", &[text])?;
    let (r, g, b) = match color {
        [Value::Num(r), Value::Num(g), Value::Num(b), ..] => (*r, *g, *b),
        _ => (1.0, 1.0, 1.0),
    };
    call(
        vm,
        fs,
        "SetTextColor",
        &[Value::Num(r), Value::Num(g), Value::Num(b)],
    )?;
    call(vm, fs, "Show", &[])?;
    Ok(())
}

fn hide_line(vm: &mut Vm, fs: Option<Value>) -> Result<(), wowlua_vm::LuaError> {
    if let Some(fs) = fs {
        call(vm, fs, "SetText", &[Value::Nil])?;
        call(vm, fs, "Hide", &[])?;
    }
    Ok(())
}

/// `tooltip:AddLine(text [, r, g, b, wrap])`.
fn add_line(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let n = count(vm, id) + 1;
    set_count(vm, id, n);
    let color: Vec<Value> = c.args.iter().skip(2).copied().collect();
    if let Some(fs) = line(vm, id, "Left", n) {
        show_line(vm, fs, c.arg(2), &color)?;
    }
    let right = line(vm, id, "Right", n);
    hide_line(vm, right)?;
    Ok(vec![])
}

/// `tooltip:AddDoubleLine(left, right [, lr, lg, lb, rr, rg, rb])`.
fn add_double_line(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let n = count(vm, id) + 1;
    set_count(vm, id, n);
    let left_color: Vec<Value> = c.args.iter().skip(3).take(3).copied().collect();
    let right_color: Vec<Value> = c.args.iter().skip(6).take(3).copied().collect();
    if let Some(fs) = line(vm, id, "Left", n) {
        show_line(vm, fs, c.arg(2), &left_color)?;
    }
    if let Some(fs) = line(vm, id, "Right", n) {
        show_line(vm, fs, c.arg(3), &right_color)?;
    }
    Ok(vec![])
}

fn clear(vm: &mut Vm, id: WidgetId) -> Result<(), wowlua_vm::LuaError> {
    for n in 1..=count(vm, id) {
        for side in ["Left", "Right"] {
            let fs = line(vm, id, side, n);
            hide_line(vm, fs)?;
        }
    }
    set_count(vm, id, 0);
    Ok(())
}

fn clear_lines(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    clear(vm, id)?;
    Ok(vec![])
}

/// `tooltip:SetText(text [, r, g, b, a, wrap])`: the only line.
fn set_text(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    clear(vm, id)?;
    add_line(vm, id, c)
}

fn num_lines(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(count(vm, id) as f64)])
}

/// `tooltip:SetOwner(owner, anchor, x, y)`: owned, and empty.
fn set_owner(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    clear(vm, id)?;
    let args: Vec<Value> = c.args.iter().skip(1).copied().collect();
    widgets::store(vm, id, "Owner", &args);
    Ok(vec![])
}

fn get_owner(vm: &mut Vm, id: WidgetId, _c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    Ok(widgets::stored_args(vm, props, "Owner")
        .and_then(|v| v.first().copied())
        .map(|o| vec![o])
        .unwrap_or_default())
}

fn is_owned(vm: &mut Vm, id: WidgetId, c: NativeCall) -> Ret {
    let props = engine(vm).world.widgets[id.0].props;
    let owner = widgets::stored_args(vm, props, "Owner").and_then(|v| v.first().copied());
    let same = match (owner, c.arg(2)) {
        (Some(Value::Table(a)), Value::Table(b)) => a == b,
        _ => false,
    };
    Ok(vec![Value::Bool(same)])
}

/// The methods (by full name: other widgets' `SetText` stays theirs).
pub fn methods() -> Vec<(&'static str, MethodFn)> {
    vec![
        ("GameTooltip:AddLine", add_line),
        ("GameTooltip:AddDoubleLine", add_double_line),
        ("GameTooltip:ClearLines", clear_lines),
        ("GameTooltip:SetText", set_text),
        ("GameTooltip:NumLines", num_lines),
        ("GameTooltip:SetOwner", set_owner),
        ("GameTooltip:GetOwner", get_owner),
        ("GameTooltip:IsOwned", is_owned),
    ]
}
