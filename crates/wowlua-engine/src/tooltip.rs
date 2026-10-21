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
/// and post-calls on), from the tests' game data, as WoW Forever 1.60.1
/// answered the full probe:
///
/// - Every data carries `dataInstanceID`, the session's count of tooltip
///   data made (the probe's answers: consecutive within each session in the
///   plan's order, a call answering nothing counting none; unverified: the
///   session's first, which the UI's own tooltips had taken before).
/// - An item's (`type` 0): `hyperlink` when asked by ID or link, its `id`,
///   the Azerite and corruption flags false; the lines its name in its
///   quality's color (`ItemName`, with `quality`), its binding
///   (`ItemBinding`, with `bonding`), "Unique". An unknown item by ID: no
///   lines and no type. No item level line (none in any recorded item's).
/// - A spell's (`type` 1): its name with its rank on the right in gray
///   (`SpellName`), then a line of cost and range, both empty where the game
///   data has none (Attack's two lines).
///
/// Unverified: the lines the client adds that the game data lacks (an
/// item's equip slot, stats, requirements, sell price; a spell's cost,
/// range, cast time, cooldown and description), the colors of other
/// units' names.
const QUALITY_RGB: [(u8, u8, u8); 8] = [
    (157, 157, 157),
    (255, 255, 255),
    (30, 255, 0),
    (0, 112, 221),
    (163, 53, 238),
    (255, 128, 0),
    (230, 204, 128),
    (0, 204, 255),
];
const WHITE: (u8, u8, u8) = (255, 255, 255);
/// `NORMAL_FONT_COLOR`.
const GOLD: (u8, u8, u8) = (255, 210, 0);
/// A spell's rank (recorded: 128 of 255).
const GRAY: (u8, u8, u8) = (128, 128, 128);
/// "Retrieving item information" (`RED_FONT_COLOR`'s 255, 32, 32).
const RED: (u8, u8, u8) = (255, 32, 32);

/// `Enum.TooltipDataLineType`'s values used here.
const LINE_NONE: f64 = 0.0;
const LINE_UNIT_NAME: f64 = 2.0;
const LINE_QUEST_OBJECTIVE: f64 = 8.0;
const LINE_SPELL_NAME: f64 = 13.0;
const LINE_QUEST_TITLE: f64 = 17.0;
const LINE_ITEM_BINDING: f64 = 20.0;
const LINE_ITEM_NAME: f64 = 22.0;
const LINE_TOY_NAME: f64 = 24.0;
const LINE_ERROR: f64 = 41.0;

/// `Enum.TooltipDataType`'s values used here.
const DATA_ITEM: f64 = 0.0;
const DATA_SPELL: f64 = 1.0;
const DATA_UNIT: f64 = 2.0;
const DATA_COMPANION_PET: f64 = 9.0;
const DATA_EQUIPMENT_SET: f64 = 14.0;
const DATA_TOY: f64 = 19.0;
const DATA_QUEST_PARTY_PROGRESS: f64 = 24.0;

/// Items whose tooltips say "Unique" (recorded: the Hearthstone and the
/// Secure Crate; the game data has no such flag).
const UNIQUE_ITEMS: &[i64] = &[6462, 6948];

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
/// Its parts are bytes times 1/255 in single precision, as the client's
/// (recorded: 210 is 0.82352948188782, 112 0.43921571969986), without
/// alpha (recorded: no `a`, `ColorMixin`'s methods: a mixin's object the
/// parts set on, not `CreateColor`'s).
fn color(vm: &mut Vm, (r, g, b): (u8, u8, u8)) -> Value {
    let platform = crate::engine(vm).platform;
    let part = |v: u8| crate::byte_channel(platform, f32::from(v));
    let (r, g, b) = (part(r), part(g), part(b));
    let create = vm.get_global("CreateFromMixins");
    let mixin = vm.get_global("ColorMixin");
    if matches!(create, Value::Func(_))
        && matches!(mixin, Value::Table(_))
        && let Ok(v) = vm.call(create, &[mixin])
        && let Some(&Value::Table(c)) = v.first()
    {
        for (k, v) in [("r", r), ("g", g), ("b", b)] {
            set(vm, c, k, Value::Num(v));
        }
        return Value::Table(c);
    }
    let t = vm.new_table();
    for (k, v) in [("r", r), ("g", g), ("b", b)] {
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

/// Another behaviour's answer (its values unwrapped); none without one.
fn ask(vm: &mut Vm, name: &'static str, args: &[Value]) -> Vec<Value> {
    let Some(f) = engine(vm).behaviour(name) else {
        return vec![];
    };
    let full = vm.str(name);
    let call = NativeCall {
        args: args.iter().copied().collect(),
        taints: args.iter().map(|_| Default::default()).collect(),
        origins: args.iter().map(|_| Default::default()).collect(),
        upvals: std::rc::Rc::from([full]),
        upval_taints: std::rc::Rc::from([Default::default()]),
    };
    match f(vm, call) {
        Ok(values) => values.into_iter().map(|v| vm.hidden(v)).collect(),
        Err(_) => vec![],
    }
}

fn text_of(vm: &Vm, v: Value) -> Option<String> {
    match vm.hidden(v) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// A global string, else `fallback` (the engine without Blizzard's strings).
fn global_text(vm: &mut Vm, name: &str, fallback: &str) -> String {
    let v = vm.get_global(name);
    text_of(vm, v).unwrap_or_else(|| fallback.to_string())
}

/// A line: its type, its left text and color, then its other fields.
fn data_line(
    vm: &mut Vm,
    line_type: f64,
    text: &str,
    rgb: (u8, u8, u8),
    fields: &[(&str, Value)],
) -> Value {
    let t = vm.new_table();
    set(vm, t, "type", Value::Num(line_type));
    let s = vm.str(text);
    set(vm, t, "leftText", s);
    let c = color(vm, rgb);
    set(vm, t, "leftColor", c);
    for (k, v) in fields {
        set(vm, t, k, *v);
    }
    Value::Table(t)
}

/// A line with a right text.
fn double_line(
    vm: &mut Vm,
    line_type: f64,
    (left, left_rgb): (&str, (u8, u8, u8)),
    (right, right_rgb): (&str, (u8, u8, u8)),
) -> Value {
    let t = data_line(vm, line_type, left, left_rgb, &[]);
    if let Value::Table(t) = t {
        let s = vm.str(right);
        set(vm, t, "rightText", s);
        let c = color(vm, right_rgb);
        set(vm, t, "rightColor", c);
    }
    t
}

/// The session's next tooltip data instance (see above).
fn next_instance(vm: &mut Vm) -> f64 {
    let helpers = &mut crate::engine_mut(vm).lua_helpers;
    let n = match helpers.get("tooltip.dataInstanceID") {
        Some(Value::Num(n)) => n + 1.0,
        _ => 1.0,
    };
    helpers.insert("tooltip.dataInstanceID", Value::Num(n));
    n
}

/// A tooltip data: its instance, `type` unless none, its other fields and
/// its lines.
fn data(vm: &mut Vm, data_type: Option<f64>, fields: &[(&str, Value)], lines: Vec<Value>) -> Ret {
    let t = vm.new_table();
    let n = next_instance(vm);
    set(vm, t, "dataInstanceID", Value::Num(n));
    if let Some(ty) = data_type {
        set(vm, t, "type", Value::Num(ty));
    }
    for (k, v) in fields {
        set(vm, t, k, *v);
    }
    let list = vm.new_table();
    for (i, l) in lines.into_iter().enumerate() {
        let _ = vm.heap.table_mut(list).set(Value::Num(i as f64 + 1.0), l);
    }
    set(vm, t, "lines", Value::Table(list));
    Ok(vec![Value::Table(t)])
}

/// Tooltip data of one name line in white (`extras.rs`: a spell the game
/// data lacks, its spellbook entry's name).
pub fn named_data(vm: &mut Vm, data_type: f64, id: f64, name: Value) -> Vec<Value> {
    let name = text_of(vm, name).unwrap_or_default();
    let l = double_line(vm, LINE_SPELL_NAME, (&name, WHITE), ("", GRAY));
    data(vm, Some(data_type), &[("id", Value::Num(id))], vec![l]).unwrap_or_default()
}

/// How an item's tooltip was asked for: by ID or link (its `hyperlink`),
/// or held (bound items "Soulbound").
#[derive(Clone, Copy, PartialEq)]
enum Asked {
    ById,
    Held { bound: bool },
}

/// An item's lines (`None` without its data); a data the client does not
/// have yet: its retrieving line (unverified: the client also asks for the
/// data and updates the tooltip).
fn item_lines(vm: &mut Vm, id: i64, asked: Asked) -> Option<Vec<Value>> {
    let item = crate::gamedata::item_data(vm, id)?;
    let field = |vm: &mut Vm, k: &str| {
        let key = vm.str(k);
        vm.heap.table(item).get(key)
    };
    if !field(vm, "cached").truthy() {
        let text = global_text(vm, "RETRIEVING_ITEM_INFO", "Retrieving item information");
        return Some(vec![data_line(vm, LINE_ERROR, &text, RED, &[])]);
    }
    let name = field(vm, "name");
    let name = text_of(vm, name).unwrap_or_default();
    let quality = match field(vm, "quality") {
        Value::Num(q) => q,
        _ => 1.0,
    };
    let rgb = QUALITY_RGB
        .get(quality as usize)
        .copied()
        .unwrap_or(QUALITY_RGB[1]);
    let mut lines = vec![data_line(
        vm,
        LINE_ITEM_NAME,
        &name,
        rgb,
        &[("quality", Value::Num(quality))],
    )];
    // `Enum.TooltipDataItemBinding` by the item's bind type (recorded: a
    // quest item's "Quest Item" 0, the Hearthstone's "Binds when picked
    // up" 6, a worn or carried bound item's "Soulbound" 3; unverified: on
    // equip and on use).
    let bind = match field(vm, "bindType") {
        Value::Num(n) => n as i64,
        _ => 0,
    };
    let binding = match (asked, bind) {
        (_, 4) => Some((0.0, "ITEM_BIND_QUEST", "Quest Item")),
        (Asked::Held { bound: true }, _) => Some((3.0, "ITEM_SOULBOUND", "Soulbound")),
        (_, 1) => Some((6.0, "ITEM_BIND_ON_PICKUP", "Binds when picked up")),
        (_, 2) => Some((7.0, "ITEM_BIND_ON_EQUIP", "Binds when equipped")),
        (_, 3) => Some((8.0, "ITEM_BIND_ON_USE", "Binds when used")),
        _ => None,
    };
    if let Some((bonding, global, fallback)) = binding {
        let text = global_text(vm, global, fallback);
        let l = data_line(
            vm,
            LINE_ITEM_BINDING,
            &text,
            WHITE,
            &[("bonding", Value::Num(bonding))],
        );
        lines.push(l);
    }
    if UNIQUE_ITEMS.contains(&id) {
        let text = global_text(vm, "ITEM_UNIQUE", "Unique");
        lines.push(data_line(vm, LINE_NONE, &text, WHITE, &[]));
    }
    Some(lines)
}

/// An item's data fields before its lines.
fn item_fields(id: i64, link: Option<Value>) -> Vec<(&'static str, Value)> {
    let mut fields = Vec::new();
    if let Some(link) = link {
        fields.push(("hyperlink", link));
    }
    fields.push(("id", Value::Num(id as f64)));
    for flag in ["isAzeriteEmpoweredItem", "isAzeriteItem", "isCorruptedItem"] {
        fields.push((flag, Value::Bool(false)));
    }
    fields
}

/// An item's tooltip data (nothing without its data).
fn item_tooltip(vm: &mut Vm, id: i64, asked: Asked, link: Option<Value>) -> Ret {
    let Some(lines) = item_lines(vm, id, asked) else {
        return Ok(vec![]);
    };
    let fields = item_fields(id, link);
    data(vm, Some(DATA_ITEM), &fields, lines)
}

/// An item's tooltip data asked another way (`extras.rs`: an action's, a
/// merchant's goods'): as by ID, without its link (unverified).
pub fn item_data_of(vm: &mut Vm, id: i64) -> Vec<Value> {
    item_tooltip(vm, id, Asked::ById, None).unwrap_or_default()
}

/// An item link with its fields after the spec (`21:1486:…`) set: the
/// item context and the bonus IDs (recorded: 3524 on every item asked
/// for by ID with a context, or by item key).
fn link_with(link: &str, context: Option<i64>, bonuses: &[i64]) -> String {
    let Some(start) = link.find("|Hitem:") else {
        return link.to_string();
    };
    let body = start + 2;
    let end = link[body..].find('|').map_or(link.len(), |e| body + e);
    let mut parts: Vec<String> = link[body..end].split(':').map(str::to_string).collect();
    // item, ID, enchant, four gems, suffix, unique, level, spec, upgrade
    // type, context, the bonus IDs' count and the bonuses.
    if parts.len() < 14 {
        return link.to_string();
    }
    parts[12] = context.map(|c| c.to_string()).unwrap_or_default();
    if !bonuses.is_empty() {
        parts[13] = bonuses.len().to_string();
        for (i, b) in bonuses.iter().enumerate() {
            parts.insert(14 + i, b.to_string());
        }
    }
    format!("{}{}{}", &link[..body], parts.join(":"), &link[end..])
}

/// The item's link as `GetItemInfo` gives it.
fn item_link(vm: &mut Vm, id: i64) -> Option<String> {
    let v = crate::character::link_of_id(vm, id)?;
    text_of(vm, v)
}

fn int_arg(vm: &Vm, c: &NativeCall, n: usize) -> Option<i64> {
    match vm.hidden(c.arg(n)) {
        Value::Num(x) => Some(x.trunc() as i64),
        _ => None,
    }
}

/// `C_TooltipInfo.GetItemByID(itemID [, quality, itemContext,
/// treasureContextLevel])`: the item's, its link carrying the context and
/// bonus 3524 when the context is one (recorded: 1 and 1.5 so; -1, 0, nil
/// and 2147483647 the plain link; the quality and the treasure level
/// changing nothing; unverified: other contexts). An unknown item: no
/// lines, no type, no ID (recorded: -1, 0, 1, 2147483647).
fn get_item_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = int_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let Some(plain) = item_link(vm, id) else {
        return data(vm, None, &[], vec![]);
    };
    let context = int_arg(vm, &c, 3).filter(|n| (1..=255).contains(n));
    let link = match context {
        Some(ctx) => link_with(&plain, Some(ctx), &[3524]),
        None => plain,
    };
    let link = vm.str(&link);
    item_tooltip(vm, id, Asked::ById, Some(link))
}

/// `C_TooltipInfo.GetHeirloomByItemID(itemID)`: as `GetItemByID(itemID)`
/// (recorded: the Hearthstone's and the crate's, no heirlooms; an unknown
/// item no lines).
fn get_heirloom_by_item_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = int_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    match item_link(vm, id) {
        Some(link) => {
            let link = vm.str(&link);
            item_tooltip(vm, id, Asked::ById, Some(link))
        }
        None => data(vm, None, &[], vec![]),
    }
}

/// `C_TooltipInfo.GetItemKey(itemID, itemLevel, itemSuffix [,
/// requiredLevel])`: the item's, its link with bonus 3524 and no context,
/// `overrideItemLevel` the level when above 0; nothing for an unknown item
/// (recorded).
fn get_item_key(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = int_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let Some(plain) = item_link(vm, id) else {
        return Ok(vec![]);
    };
    let Some(lines) = item_lines(vm, id, Asked::ById) else {
        return Ok(vec![]);
    };
    let link = link_with(&plain, None, &[3524]);
    let link = vm.str(&link);
    let mut lines = lines;
    let mut fields = item_fields(id, Some(link));
    if let Some(level) = int_arg(vm, &c, 2).filter(|l| *l > 0) {
        fields.push(("overrideItemLevel", Value::Num(level as f64)));
    }
    // A required level above 1: its line (`Requirement`, 43), red when the
    // player's is lower (recorded: 2147483647 red and not usable, 1 and 0
    // none; a worn item's level 17 white and usable).
    if let Some(required) = int_arg(vm, &c, 4).filter(|l| *l > 1) {
        let mine = engine(vm).units.list[crate::units::PLAYER.0].level as i64;
        let usable = mine >= required;
        let text = global_text(vm, "ITEM_MIN_LEVEL", "Requires Level %d")
            .replace("%d", &required.to_string());
        let l = data_line(
            vm,
            43.0,
            &text,
            if usable { WHITE } else { RED },
            &[
                ("level", Value::Num(required as f64)),
                ("requirementType", Value::Num(5.0)),
                ("usable", Value::Bool(usable)),
            ],
        );
        lines.push(l);
    }
    data(vm, Some(DATA_ITEM), &fields, lines)
}

/// `C_TooltipInfo.GetToyByItemID(itemID)`: a toy's data, the item's name
/// as its only line (`ToyName`), "Retrieving item information" for an
/// unknown item; its `id` unless 0 (recorded, every item asked: none a
/// toy).
fn get_toy_by_item_id(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = int_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let name = crate::gamedata::item_data(vm, id).and_then(|t| {
        let key = vm.str("name");
        let v = vm.heap.table(t).get(key);
        text_of(vm, v)
    });
    let l = match name {
        Some(name) => data_line(vm, LINE_TOY_NAME, &name, WHITE, &[]),
        None => {
            let text = global_text(vm, "RETRIEVING_ITEM_INFO", "Retrieving item information");
            data_line(vm, LINE_ERROR, &text, RED, &[])
        }
    };
    let fields: Vec<(&str, Value)> = if id == 0 {
        vec![]
    } else {
        vec![("id", Value::Num(id as f64))]
    };
    data(vm, Some(DATA_TOY), &fields, vec![l])
}

/// A spell's tooltip data: a defined spell's, else the client's spell
/// table's spell's (`gamedata::scanned_spell`): its name and rank, the
/// cost and range line, then a defined spell's description (`SpellName`;
/// the description's line as Devotion Aura's, gold and wrapped; recorded:
/// Attack's two lines).
fn spell_tooltip(vm: &mut Vm, id: i64) -> Ret {
    let defined = engine(vm).world.data.spells.get(&id).copied();
    let Some(spell) = defined.or_else(|| crate::gamedata::scanned_spell(vm, id)) else {
        return Ok(vec![]);
    };
    let field = |vm: &mut Vm, k: &str| {
        let key = vm.str(k);
        let v = vm.heap.table(spell).get(key);
        text_of(vm, v)
    };
    let name = field(vm, "name").unwrap_or_default();
    let rank = field(vm, "subtext").unwrap_or_default();
    let mut lines = vec![
        double_line(vm, LINE_SPELL_NAME, (&name, WHITE), (&rank, GRAY)),
        double_line(vm, LINE_NONE, ("", WHITE), ("", WHITE)),
    ];
    if let Some(description) = field(vm, "description").filter(|d| !d.is_empty()) {
        let wrap = [("wrapText", Value::Bool(true))];
        lines.push(data_line(vm, 34.0, &description, GOLD, &wrap));
    }
    data(
        vm,
        Some(DATA_SPELL),
        &[("id", Value::Num(id as f64))],
        lines,
    )
}

/// `C_TooltipInfo.GetSpellByID(spellID, …)`: the spell's; a spell's name
/// names it too (recorded: "Attack" Attack's).
fn get_spell_by_id(vm: &mut Vm, c: NativeCall) -> Ret {
    match vm.hidden(c.arg(1)) {
        Value::Num(n) => spell_tooltip(vm, n as i64),
        Value::Str(_) => {
            let info = ask(vm, "C_Spell.GetSpellInfo", &[c.arg(1)]);
            let id = match info.first() {
                Some(Value::Table(t)) => {
                    let key = vm.str("spellID");
                    vm.heap.table(*t).get(key)
                }
                _ => Value::Nil,
            };
            match id {
                Value::Num(n) => spell_tooltip(vm, n as i64),
                _ => Ok(vec![]),
            }
        }
        _ => Ok(vec![]),
    }
}

/// `C_TooltipInfo.GetUnit(unit [, hideStatus])`: a player's name with its
/// last name (`UnitName`, gold, with the token asked), guild, "Level N
/// Race (Player)", class and faction (recorded: the player, its GUID also
/// `healthGUID`); another unit's name in its reaction's color and its
/// level (unverified).
fn get_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = crate::units::unit_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let token = c.arg(1);
    let u = &engine(vm).units.list[id.0];
    let (name, last, level, guid, reaction, player) = (
        u.name.clone(),
        u.last_name.clone(),
        u.level,
        u.guid.clone(),
        u.reaction,
        u.player,
    );
    let g = vm.str(&guid);
    if !player {
        let rgb = match reaction {
            crate::units::Reaction::Hostile => (255, 26, 26),
            crate::units::Reaction::Neutral => (255, 255, 0),
            crate::units::Reaction::Friendly => (0, 255, 0),
        };
        let lines = vec![
            data_line(vm, LINE_UNIT_NAME, &name, rgb, &[]),
            data_line(vm, LINE_NONE, &format!("Level {level}"), WHITE, &[]),
        ];
        return data(vm, Some(DATA_UNIT), &[("guid", g)], lines);
    }
    let full = match last {
        Some(last) if !last.is_empty() => format!("{name} {last}"),
        _ => name,
    };
    let race = ask(vm, "UnitRace", &[token]);
    let race = race
        .first()
        .and_then(|v| text_of(vm, *v))
        .unwrap_or_default();
    let class = ask(vm, "UnitClass", &[token]);
    let class = class
        .first()
        .and_then(|v| text_of(vm, *v))
        .unwrap_or_default();
    let faction = ask(vm, "UnitFactionGroup", &[token]);
    let faction = faction
        .get(1)
        .and_then(|v| text_of(vm, *v))
        .unwrap_or_default();
    let guild = ask(vm, "GetGuildInfo", &[token]);
    let guild = guild.first().and_then(|v| text_of(vm, *v));
    let mut name_fields = Vec::new();
    if matches!(vm.hidden(token), Value::Str(_))
        && !text_of(vm, token).unwrap_or_default().contains('-')
    {
        name_fields.push(("unitToken", token));
    }
    let mut lines = vec![data_line(vm, LINE_UNIT_NAME, &full, GOLD, &name_fields)];
    if let Some(guild) = guild.filter(|g| !g.is_empty()) {
        lines.push(data_line(vm, LINE_NONE, &guild, WHITE, &[]));
    }
    let level_line = format!("Level {level} {race} (Player)");
    lines.push(data_line(vm, LINE_NONE, &level_line, WHITE, &[]));
    lines.push(data_line(vm, LINE_NONE, &class, WHITE, &[]));
    if !faction.is_empty() {
        lines.push(data_line(vm, LINE_NONE, &faction, WHITE, &[]));
    }
    data(
        vm,
        Some(DATA_UNIT),
        &[("guid", g), ("healthGUID", g)],
        lines,
    )
}

/// `C_TooltipInfo.GetInventoryItem(unit, slot [, hideUselessStats])`: the
/// player's worn item's (unverified: its GUID, equip slot, stats).
fn get_inventory_item(vm: &mut Vm, c: NativeCall) -> Ret {
    if crate::units::unit_arg(vm, &c, 1) != Some(crate::units::PLAYER) {
        return Ok(vec![]);
    }
    let Some(slot) = int_arg(vm, &c, 2) else {
        return Ok(vec![]);
    };
    match engine(vm).world.character.equipment.get(&slot) {
        Some(h) => {
            let (id, bound) = (h.id, h.bound);
            item_tooltip(vm, id, Asked::Held { bound }, None)
        }
        None => Ok(vec![]),
    }
}

fn get_bag_item(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(bag), Some(slot)) = (int_arg(vm, &c, 1), int_arg(vm, &c, 2)) else {
        return Ok(vec![]);
    };
    let held = engine(vm)
        .world
        .character
        .bags
        .get(&bag)
        .and_then(|b| b.items.get(&usize::try_from(slot).ok()?))
        .map(|h| (h.id, h.bound));
    match held {
        Some((id, bound)) => item_tooltip(vm, id, Asked::Held { bound }, None),
        None => Ok(vec![]),
    }
}

/// `GetHyperlink(link)`: an item's (with the item's own link) or a
/// spell's.
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
        let full = item_link(vm, id).map(|l| vm.str(&l));
        item_tooltip(vm, id, Asked::ById, full)
    } else if let Some(id) = id_after("spell:") {
        spell_tooltip(vm, id)
    } else {
        Ok(vec![])
    }
}

/// `C_TooltipInfo.GetCompanionPet(petGUID)`: a companion's data without
/// lines for any GUID (recorded: the player's and a creature's; "" and
/// "x" nothing; no companions in WoW Forever).
fn get_companion_pet(vm: &mut Vm, c: NativeCall) -> Ret {
    let guid = text_of(vm, c.arg(1)).unwrap_or_default();
    let mut parts = guid.split('-');
    let kind = parts.next().unwrap_or_default();
    if kind.is_empty() || parts.next().is_none() {
        return Ok(vec![]);
    }
    data(vm, Some(DATA_COMPANION_PET), &[], vec![])
}

/// `C_TooltipInfo.GetEquipmentSet(setID)`: the set's name with its items'
/// count on the right ("10 |4Item:Items;", gold), and how many are worn
/// (gold); its `id` unless 0 (recorded: two sets of ten worn items;
/// unverified: items in the bags or lost).
fn get_equipment_set(vm: &mut Vm, c: NativeCall) -> Ret {
    let info = ask(vm, "C_EquipmentSet.GetEquipmentSetInfo", &[c.arg(1)]);
    let (Some(name), Some(Value::Num(id)), Some(Value::Num(items)), Some(Value::Num(worn))) = (
        info.first().and_then(|v| text_of(vm, *v)),
        info.get(2).copied(),
        info.get(4).copied(),
        info.get(5).copied(),
    ) else {
        return Ok(vec![]);
    };
    let lines = vec![
        double_line(
            vm,
            LINE_NONE,
            (&name, WHITE),
            (&format!("{items} |4Item:Items;"), GOLD),
        ),
        data_line(
            vm,
            LINE_NONE,
            &format!("{worn} |4item:items; equipped"),
            GOLD,
            &[],
        ),
    ];
    let fields: Vec<(&str, Value)> = if id == 0.0 {
        vec![]
    } else {
        vec![("id", Value::Num(id))]
    };
    data(vm, Some(DATA_EQUIPMENT_SET), &fields, lines)
}

/// `C_TooltipInfo.GetQuestPartyProgress(questID [, omitTitle,
/// ignoreActivePlayer])`: a quest in the log's title (gold, `QuestTitle`
/// with its ID) unless omitted, and the player's "On quest" (`completed`
/// false) unless ignored; nothing for a quest not in the log (recorded,
/// the player alone; unverified: `completed` for a finished quest, 1063's
/// false).
fn get_quest_party_progress(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = int_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let index = ask(
        vm,
        "C_QuestLog.GetLogIndexForQuestID",
        &[Value::Num(id as f64)],
    );
    if !matches!(index.first(), Some(Value::Num(_))) {
        return Ok(vec![]);
    }
    let mut lines = Vec::new();
    if !c.arg(2).truthy() {
        let title = ask(
            vm,
            "C_QuestLog.GetTitleForQuestID",
            &[Value::Num(id as f64)],
        );
        let title = title
            .first()
            .and_then(|v| text_of(vm, *v))
            .unwrap_or_default();
        let l = data_line(
            vm,
            LINE_QUEST_TITLE,
            &title,
            GOLD,
            &[("id", Value::Num(id as f64))],
        );
        lines.push(l);
    }
    if !c.arg(3).truthy() {
        let l = data_line(
            vm,
            LINE_QUEST_OBJECTIVE,
            "On quest",
            WHITE,
            &[("completed", Value::Bool(false))],
        );
        lines.push(l);
    }
    data(
        vm,
        Some(DATA_QUEST_PARTY_PROGRESS),
        &[("id", Value::Num(id as f64))],
        lines,
    )
}

/// `C_TooltipInfo`'s behaviour.
pub fn behaviours() -> Vec<(&'static str, wowlua_vm::NativeFn)> {
    vec![
        ("C_TooltipInfo.GetItemByID", get_item_by_id),
        ("C_TooltipInfo.GetHeirloomByItemID", get_heirloom_by_item_id),
        ("C_TooltipInfo.GetItemKey", get_item_key),
        ("C_TooltipInfo.GetToyByItemID", get_toy_by_item_id),
        ("C_TooltipInfo.GetSpellByID", get_spell_by_id),
        ("C_TooltipInfo.GetUnit", get_unit),
        ("C_TooltipInfo.GetInventoryItem", get_inventory_item),
        ("C_TooltipInfo.GetBagItem", get_bag_item),
        ("C_TooltipInfo.GetHyperlink", get_hyperlink),
        ("C_TooltipInfo.GetCompanionPet", get_companion_pet),
        ("C_TooltipInfo.GetEquipmentSet", get_equipment_set),
        (
            "C_TooltipInfo.GetQuestPartyProgress",
            get_quest_party_progress,
        ),
    ]
}
