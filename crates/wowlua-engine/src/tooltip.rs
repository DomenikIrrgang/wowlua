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

// ------------------------------------------------- the tooltip data system

/// `C_TooltipInfo`'s getters (the client's tooltip data, which Blizzard's
/// `TooltipDataHandlerMixin` turns into lines and runs the addons' pre-
/// and post-calls on), from the tests' game data: an item's name in its
/// quality's color and its item level; a spell's name and description; a
/// unit's name in its reaction's color and its level.
///
/// Unverified: the lines' types beyond the names, the colors, and the
/// lines the client adds that are not here (requirements, flavor text,
/// sell price, …).
const QUALITY_RGB: [(f64, f64, f64); 8] = [
    (0.62, 0.62, 0.62),
    (1.0, 1.0, 1.0),
    (0.12, 1.0, 0.0),
    (0.0, 0.44, 0.87),
    (0.64, 0.21, 0.93),
    (1.0, 0.5, 0.0),
    (0.9, 0.8, 0.5),
    (0.0, 0.8, 1.0),
];

fn color_get_rgb(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    for k in ["r", "g", "b", "a"] {
        let key = vm.str(k);
        out.push(vm.heap.table(t).get(key));
    }
    Ok(out)
}

/// A color as Blizzard's code takes it: `CreateColor`'s when its UI code is
/// loaded, else a table with `r`, `g`, `b`, `a`, `GetRGB` and `GetRGBA`.
fn color(vm: &mut Vm, (r, g, b): (f64, f64, f64)) -> Value {
    let create = vm.get_global("CreateColor");
    if matches!(create, Value::Func(_))
        && let Ok(v) = vm.call(
            create,
            &[Value::Num(r), Value::Num(g), Value::Num(b), Value::Num(1.0)],
        )
        && let Some(&c) = v.first()
    {
        return c;
    }
    let t = vm.new_table();
    for (k, v) in [("r", r), ("g", g), ("b", b), ("a", 1.0)] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, Value::Num(v));
    }
    let get = vm.native("GetRGB", color_get_rgb);
    for k in ["GetRGB", "GetRGBA"] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, get);
    }
    Value::Table(t)
}

fn set(vm: &mut Vm, t: wowlua_vm::TableRef, k: &str, v: Value) {
    let key = vm.str(k);
    let _ = vm.heap.table_mut(t).set(key, v);
}

fn data_line(vm: &mut Vm, line_type: f64, text: Value, rgb: Option<(f64, f64, f64)>) -> Value {
    let t = vm.new_table();
    set(vm, t, "type", Value::Num(line_type));
    set(vm, t, "leftText", text);
    if let Some(rgb) = rgb {
        let c = color(vm, rgb);
        set(vm, t, "leftColor", c);
    }
    set(vm, t, "wrapText", Value::Bool(false));
    Value::Table(t)
}

fn tooltip_data(vm: &mut Vm, data_type: f64, key: (&str, Value), lines: Vec<Value>) -> Ret {
    let t = vm.new_table();
    set(vm, t, "type", Value::Num(data_type));
    set(vm, t, key.0, key.1);
    let list = vm.new_table();
    for (i, l) in lines.into_iter().enumerate() {
        let _ = vm.heap.table_mut(list).set(Value::Num(i as f64 + 1.0), l);
    }
    set(vm, t, "lines", Value::Table(list));
    Ok(vec![Value::Table(t)])
}

fn item_tooltip(vm: &mut Vm, id: i64) -> Ret {
    let Some(item) = engine(vm).world.data.items.get(&id).copied() else {
        return Ok(vec![]);
    };
    let field = |vm: &mut Vm, k: &str| {
        let key = vm.str(k);
        vm.heap.table(item).get(key)
    };
    // Data the client does not have yet: its retrieving line (unverified:
    // the client also asks for the data and updates the tooltip).
    if !field(vm, "cached").truthy() {
        let text = match vm.get_global("RETRIEVING_ITEM_INFO") {
            Value::Nil => vm.str("Retrieving item information"),
            v => v,
        };
        let line = data_line(vm, 0.0, text, Some((1.0, 0.1, 0.1)));
        return tooltip_data(vm, 0.0, ("id", Value::Num(id as f64)), vec![line]);
    }
    let name = field(vm, "name");
    let quality = match field(vm, "quality") {
        Value::Num(q) => q as usize,
        _ => 1,
    };
    let rgb = QUALITY_RGB.get(quality).copied().unwrap_or(QUALITY_RGB[1]);
    let mut lines = vec![data_line(vm, 0.0, name, Some(rgb))];
    if let Value::Num(level) = field(vm, "itemLevel") {
        let text = vm.str(&format!("Item Level {level}"));
        lines.push(data_line(vm, 0.0, text, Some((1.0, 0.82, 0.0))));
    }
    tooltip_data(vm, 0.0, ("id", Value::Num(id as f64)), lines)
}

fn get_item_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) => item_tooltip(vm, n as i64),
        _ => Ok(vec![]),
    }
}

fn spell_tooltip(vm: &mut Vm, id: i64) -> Ret {
    let Some(spell) = engine(vm).world.data.spells.get(&id).copied() else {
        return Ok(vec![]);
    };
    let field = |vm: &mut Vm, k: &str| {
        let key = vm.str(k);
        vm.heap.table(spell).get(key)
    };
    let name = field(vm, "name");
    let mut lines = vec![data_line(vm, 0.0, name, Some((1.0, 1.0, 1.0)))];
    let description = field(vm, "description");
    if !description.is_nil() {
        lines.push(data_line(vm, 0.0, description, None));
    }
    tooltip_data(vm, 1.0, ("id", Value::Num(id as f64)), lines)
}

fn get_spell_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) => spell_tooltip(vm, n as i64),
        _ => Ok(vec![]),
    }
}

fn get_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = crate::units::unit_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let u = &engine(vm).units.list[id.0];
    let (name, level, guid, reaction) = (u.name.clone(), u.level, u.guid.clone(), u.reaction);
    let rgb = match reaction {
        crate::units::Reaction::Hostile => (1.0, 0.1, 0.1),
        crate::units::Reaction::Neutral => (1.0, 1.0, 0.0),
        crate::units::Reaction::Friendly => (0.0, 1.0, 0.0),
    };
    let n = vm.str(&name);
    let level_text = vm.str(&format!("Level {level}"));
    let lines = vec![
        data_line(vm, 2.0, n, Some(rgb)),
        data_line(vm, 0.0, level_text, Some((1.0, 1.0, 1.0))),
    ];
    let g = vm.str(&guid);
    tooltip_data(vm, 2.0, ("guid", g), lines)
}

fn get_inventory_item(vm: &mut Vm, c: NativeCall) -> Ret {
    if crate::units::unit_arg(vm, &c, 1) != Some(crate::units::PLAYER) {
        return Ok(vec![]);
    }
    let Value::Num(slot) = c.arg(2) else {
        return Ok(vec![]);
    };
    match engine(vm).world.character.equipment.get(&(slot as i64)) {
        Some(h) => {
            let id = h.id;
            item_tooltip(vm, id)
        }
        None => Ok(vec![]),
    }
}

fn get_bag_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(bag), Value::Num(slot)) = (c.arg(1), c.arg(2)) else {
        return Ok(vec![]);
    };
    let id = engine(vm)
        .world
        .character
        .bags
        .get(&(bag as i64))
        .and_then(|b| b.items.get(&(slot as usize)))
        .map(|h| h.id);
    match id {
        Some(id) => item_tooltip(vm, id),
        None => Ok(vec![]),
    }
}

/// `GetHyperlink(link)`: an item's or a spell's.
fn get_hyperlink(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(s) = vm.hidden(c.arg(1)) else {
        return Ok(vec![]);
    };
    let link = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
    let id_after = |key: &str| -> Option<i64> {
        link.split(key)
            .nth(1)?
            .split(|ch: char| !ch.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    };
    if let Some(id) = id_after("item:") {
        item_tooltip(vm, id)
    } else if let Some(id) = id_after("spell:") {
        spell_tooltip(vm, id)
    } else {
        Ok(vec![])
    }
}

/// `C_TooltipInfo`'s behaviour.
pub fn behaviours() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        ("C_TooltipInfo.GetItemByID", get_item_by_id),
        ("C_TooltipInfo.GetSpellByID", get_spell_by_id),
        ("C_TooltipInfo.GetUnit", get_unit),
        ("C_TooltipInfo.GetInventoryItem", get_inventory_item),
        ("C_TooltipInfo.GetBagItem", get_bag_item),
        ("C_TooltipInfo.GetHyperlink", get_hyperlink),
    ]
}
