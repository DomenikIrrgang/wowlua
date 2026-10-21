//! Answers addons ask of the client that no other module models: the
//! calendar time and month, aura containers' options completed, the
//! damage meter's session length, a
//! spell's level and whether it is the current one, what kind of widget an
//! object is.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::engine;
use crate::units;

/// `C_DateAndTime.GetCurrentCalendarTime()`: the clock's local time as a
/// `CalendarTime` (recorded shape: `year, month, monthDay, weekday` with
/// Sunday 1, `hour, minute`).
fn get_current_calendar_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    let date = vm.get_global("date");
    let fmt = vm.str("*t");
    let tm = match vm.call(date, &[fmt])?.first().copied() {
        Some(Value::Table(t)) => t,
        _ => return Ok(vec![Value::Nil]),
    };
    let out = vm.new_table();
    for (from, to) in [
        ("year", "year"),
        ("month", "month"),
        ("day", "monthDay"),
        ("wday", "weekday"),
        ("hour", "hour"),
        ("min", "minute"),
    ] {
        let k = vm.str(from);
        let v = vm.heap.table(tm).get(k);
        let k = vm.str(to);
        let _ = vm.heap.table_mut(out).set(k, v);
    }
    Ok(vec![Value::Table(out)])
}

/// The clock's local year and month (`date("*t")`).
fn year_month(vm: &mut Vm) -> Result<(i64, i64), wowlua_vm::LuaError> {
    let date = vm.get_global("date");
    let fmt = vm.str("*t");
    let Some(Value::Table(tm)) = vm.call(date, &[fmt])?.first().copied() else {
        return Ok((2000, 1));
    };
    let mut get = |k: &str| {
        let k = vm.str(k);
        match vm.heap.table(tm).get(k) {
            Value::Num(n) => n as i64,
            _ => 1,
        }
    };
    Ok((get("year"), get("month")))
}

/// Days in a month of the Gregorian calendar.
fn days_in(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The weekday of a date, Sunday 1 (Sakamoto's method).
fn weekday(year: i64, month: i64, day: i64) -> i64 {
    const T: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    (y + y / 4 - y / 100 + y / 400 + T[(month - 1) as usize] + day).rem_euclid(7) + 1
}

/// The calendar's month shown, as months since year 0: the clock's until
/// `C_Calendar.SetMonth` or `SetAbsMonth` moves it.
fn calendar_view(vm: &mut Vm) -> Result<i64, wowlua_vm::LuaError> {
    if let Some(Value::Num(n)) = engine(vm).lua_helpers.get("misc.calendarView").copied() {
        return Ok(n as i64);
    }
    let (year, month) = year_month(vm)?;
    Ok(year * 12 + (month - 1))
}

fn set_calendar_view(vm: &mut Vm, index: i64) {
    crate::engine_mut(vm)
        .lua_helpers
        .insert("misc.calendarView", Value::Num(index as f64));
}

/// `C_Calendar.SetMonth(offsetMonths)`: the month shown moved by that many
/// (the full probe, 1.60.1: -1 showed September from October, 1
/// November, 0 stayed).
fn set_month(vm: &mut Vm, c: NativeCall) -> Ret {
    let offset = match vm.hidden(c.arg(1)) {
        Value::Num(n) => n.trunc() as i64,
        _ => 0,
    };
    let view = calendar_view(vm)?;
    set_calendar_view(vm, view + offset);
    Ok(vec![])
}

/// `C_Calendar.SetAbsMonth(month, year)`: that month shown (the full
/// probe, 1.60.1: 1, 2025 showed January 2025; unverified: month 0 or 13,
/// which showed February 2021 and January 2025, kept as they were).
fn set_abs_month(vm: &mut Vm, c: NativeCall) -> Ret {
    if let (Value::Num(m), Value::Num(y)) = (vm.hidden(c.arg(1)), vm.hidden(c.arg(2)))
        && (1.0..=12.0).contains(&m.trunc())
        && y >= 1.0
    {
        set_calendar_view(vm, y.trunc() as i64 * 12 + (m.trunc() as i64 - 1));
    }
    Ok(vec![])
}

/// `C_Calendar.GetMonthInfo([offsetMonths])`: the month that many from the
/// one shown: `month, year, numDays, firstWeekday` (Sunday 1, as the
/// calendar time's weekday; unverified: the client's server-time month).
fn get_month_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let offset = match c.arg(1) {
        Value::Num(n) => n as i64,
        _ => 0,
    };
    let index = calendar_view(vm)? + offset;
    let (year, month) = (index.div_euclid(12), index.rem_euclid(12) + 1);
    let out = vm.new_table();
    for (k, v) in [
        ("month", month),
        ("year", year),
        ("numDays", days_in(year, month)),
        ("firstWeekday", weekday(year, month, 1)),
    ] {
        let k = vm.str(k);
        let _ = vm.heap.table_mut(out).set(k, Value::Num(v as f64));
    }
    Ok(vec![Value::Table(out)])
}

/// `C_Calendar.GetNumDayEvents(offsetMonths, monthDay)`: none, and
/// `GetDayEvent` nothing (unverified: the engine keeps no calendar; the
/// client's holidays unrecorded).
fn get_num_day_events(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// No event: nothing (the full probe, 1.60.1: `GetDayEvent(1, 1, 1)`).
fn get_day_event(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

/// The spell an identifier names: its ID (a name only when the player's
/// current cast has it).
fn spell_id(vm: &mut Vm, v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        Value::Str(s) => {
            let name = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
            let cast = engine(vm)
                .units
                .casting
                .casts
                .get(&units::PLAYER)
                .cloned()?;
            match cast.name {
                Value::Str(n) if String::from_utf8_lossy(vm.str_bytes(n)) == name => {
                    Some(cast.spell_id)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// `C_Spell.IsCurrentSpell(spell)`: the player casting it, or it waiting
/// for the next swing (unverified: an auto-repeat spell, a spell waiting
/// for its target).
fn is_current_spell(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(id) = spell_id(vm, c.arg(1)) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let casting = &engine(vm).units.casting;
    let current = casting
        .casts
        .get(&units::PLAYER)
        .is_some_and(|c| c.spell_id == id)
        || casting.next_swing.as_ref().is_some_and(|(s, _)| *s == id);
    Ok(vec![Value::Bool(current)])
}

/// `C_Spell.GetSpellLevelLearned(spell)`: the player's class's level for
/// it, as the trainers' recordings or SuuUI's trainer data have it, else
/// 0 (unverified).
fn get_spell_level_learned(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(id) = c.arg(1) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    let class = engine(vm).units.list[units::PLAYER.0].class.clone();
    let level = crate::trainer::learned_level(&class, id as i64).unwrap_or(0.0);
    Ok(vec![Value::Num(level)])
}

/// A widget's kind for `C_Widget`: a region draws but holds no frames
/// (texture, mask, font string, line); fonts, animations and the API
/// objects are widgets that neither draw nor are frames.
enum Kind {
    Frame,
    Region,
    Other,
}

fn kind(vm: &Vm, v: Value) -> Option<Kind> {
    let id = crate::widgets::widget_of(vm, v)?;
    Some(match engine(vm).world.widgets[id.0].class.as_str() {
        "Texture" | "MaskTexture" | "FontString" | "Line" => Kind::Region,
        "Font"
        | "AnimationGroup"
        | "Animation"
        | "Alpha"
        | "Translation"
        | "Scale"
        | "Rotation"
        | "Path"
        | "ControlPoint"
        | "TextureCoordTranslation"
        | "FlipBook"
        | "VertexColor"
        | "LineTranslation"
        | "LineScale"
        | "CurveObject"
        | "ColorCurveObject"
        | "DurationObject"
        | "SecondsFormatter"
        | "NumericRuleFormatter"
        | "AbbreviatedNumberFormatter" => Kind::Other,
        _ => Kind::Frame,
    })
}

/// `C_Widget.IsWidget(object)`: any of the client's objects (the
/// restricted environment's `IsValidFrame` asks `IsFrameWidget`; the three
/// exist in the client's function list; unverified: their answers beyond
/// frames).
fn is_widget(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(kind(vm, c.arg(1)).is_some())])
}

/// `C_Widget.IsFrameWidget(object)`: a frame of any type.
fn is_frame_widget(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(matches!(
        kind(vm, c.arg(1)),
        Some(Kind::Frame)
    ))])
}

/// `C_Widget.IsRenderableWidget(object)`: a frame or a region.
fn is_renderable_widget(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(matches!(
        kind(vm, c.arg(1)),
        Some(Kind::Frame | Kind::Region)
    ))])
}

/// A documented structure's value from the one given: each field given
/// kept (a structure's processed in turn), each missing its documented
/// `Default`, else its empty value (`C_AuraContainerUtil.Process…Options`:
/// the options validated and completed; unverified: the client's
/// validation errors).
fn processed(vm: &mut Vm, ty: &str, given: Value, depth: usize) -> Value {
    let api = engine(vm).api;
    let ty = api.aliases.get(ty).map(String::as_str).unwrap_or(ty);
    let Some(fields) = api.structure_fields.get(ty) else {
        return given;
    };
    let Value::Table(input) = given else {
        let doc = serde_json::json!({ "Type": ty, "Nilable": false });
        return crate::default_value(vm, api, &doc, depth);
    };
    let out = vm.new_table();
    for f in fields {
        let Some(name) = f.get("Name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let key = vm.str(name);
        let v = vm.heap.table(input).get(key);
        let field_ty = f
            .get("Type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let v = if !v.is_nil() {
            if depth < 6 {
                processed(vm, field_ty, v, depth + 1)
            } else {
                v
            }
        } else {
            let field_ty = api
                .aliases
                .get(field_ty)
                .map(String::as_str)
                .unwrap_or(field_ty);
            match f.get("Default") {
                Some(serde_json::Value::Bool(b)) => Value::Bool(*b),
                Some(serde_json::Value::Number(n)) => Value::Num(n.as_f64().unwrap_or(0.0)),
                Some(serde_json::Value::String(s)) => vm.str(s),
                // A structure left out: its defaults, optional or not
                // (Blizzard's `SetTooltipBackdrop` reads `anchorOffsets.left`
                // after processing; documented optional).
                _ if api.structure_fields.contains_key(field_ty) && depth < 6 => {
                    processed(vm, field_ty, Value::Nil, depth + 1)
                }
                _ => crate::default_value(vm, api, f, depth + 1),
            }
        };
        if !v.is_nil() {
            let _ = vm.heap.table_mut(out).set(key, v);
        }
    }
    Value::Table(out)
}

macro_rules! option_processors {
    ($($name:ident: $ty:literal;)*) => {
        $(
            fn $name(vm: &mut Vm, c: NativeCall) -> Ret {
                Ok(vec![processed(vm, $ty, c.arg(1), 0)])
            }
        )*
    };
}

option_processors! {
    process_backdrop: "AuraContainerTooltipBackdropOptions";
    process_nine_slice: "AuraContainerTooltipNineSliceOptions";
    process_texture_slice: "AuraContainerTooltipTextureSliceOptions";
    process_application_bar: "CustomAuraButtonApplicationBarOptions";
    process_application_count: "CustomAuraButtonApplicationCountOptions";
    process_caster_name: "CustomAuraButtonCasterNameOptions";
    process_dispel_text: "CustomAuraButtonDispelTypeTextOptions";
    process_dispel_texture: "CustomAuraButtonDispelTypeTextureOptions";
    process_duration_bar: "CustomAuraButtonDurationBarOptions";
    process_duration_text: "CustomAuraButtonDurationTextOptions";
}

/// `IsPlayerInWorld()`: past the loading screen (recorded: true in the
/// world; unverified: false behind a loading screen).
fn is_player_in_world(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(!engine(vm).world.loading_screen)])
}

/// `GetLocale()`: the `textLocale` CVar, else `"enUS"` (probed: enUS; its
/// locale CVars were recorded empty).
fn get_locale(vm: &mut Vm, _c: NativeCall) -> Ret {
    let locale = crate::framescript::cvar_get(vm, "textLocale").filter(|l| !l.is_empty());
    Ok(vec![vm.str(locale.as_deref().unwrap_or("enUS"))])
}

/// `GetText(token, gender?, ordinal?)`: the global string named `token`; for
/// gender 3 (female) its `_FEMALE` form when there is one (Blizzard's PvP
/// ranks, `PVP_RANK_7_0` and `PVP_RANK_7_0_FEMALE`); nil for a name that is
/// no string (unverified: the client's ordinal forms; SuuUI's inspect
/// window names an inspected player's rank with it).
fn get_text(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(token) = vm.hidden(c.arg(1)) else {
        return Ok(vec![Value::Nil]);
    };
    let token = String::from_utf8_lossy(vm.str_bytes(token)).into_owned();
    let string = |vm: &mut Vm, name: &str| match vm.get_global(name) {
        v @ Value::Str(_) => Some(v),
        _ => None,
    };
    if matches!(vm.hidden(c.arg(2)), Value::Num(g) if g == 3.0)
        && let Some(v) = string(vm, &format!("{token}_FEMALE"))
    {
        return Ok(vec![v]);
    }
    Ok(vec![string(vm, &token).unwrap_or(Value::Nil)])
}

/// `GetFileIDFromPath(path)`: a loose file's ID, the same for the same path
/// (any case, either slash, with or without its extension), new ones
/// negative counting down (probed: an addon's texture -1755; the client's
/// own numbering of loose files is not reproduced; a game file's real ID is
/// not known here).
///
/// A path to no file answers nil (the full probe: `""` and `"x"`; taken
/// here as a path without a folder, unverified for others).
fn get_file_id_from_path(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Str(s) = c.arg(1) else {
        return Ok(vec![Value::Num(0.0)]);
    };
    let path = String::from_utf8_lossy(vm.str_bytes(s))
        .to_ascii_lowercase()
        .replace('/', "\\");
    if !path.contains('\\') {
        return Ok(vec![Value::Nil]);
    }
    let key = match path.rsplit_once('.') {
        Some((stem, ext)) if !ext.contains('\\') => stem.to_string(),
        _ => path,
    };
    let ids = &mut crate::engine_mut(vm).world.file_ids;
    let next = -(ids.len() as f64) - 1.0;
    let id = *ids.entry(key).or_insert(next);
    Ok(vec![Value::Num(id)])
}

/// `GetCurrentRegion()`: WoW Forever's region, 110 (probed; its name
/// empty).
fn get_current_region(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(110.0)])
}

/// `GetCurrentRegionName()`: empty (recorded).
fn get_current_region_name(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

/// `C_CurrencyInfo.GetCoinTextureString(amount [, fontHeight])`: gold,
/// silver and copper each with its icon (`|T…:h:h:2:0|t`), parts that are
/// 0 left out, a zero amount as 0 copper (probed: 123456 shows as 12 gold,
/// 34 silver, 56 copper, each number before its icon, spaced; unverified:
/// the icons' exact markup, a zero amount).
fn get_coin_texture_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let amount = match vm.hidden(c.arg(1)) {
        Value::Num(n) => n.max(0.0).floor() as i64,
        _ => 0,
    };
    // A whole height (the full probe: 1.5 written as 1).
    let height = match c.arg(2) {
        Value::Num(h) => h.trunc(),
        _ => 14.0,
    };
    let icon =
        |name: &str| format!("|TInterface\\MoneyFrame\\UI-{name}Icon:{height}:{height}:2:0|t");
    let (gold, silver, copper) = (amount / 10000, amount / 100 % 100, amount % 100);
    let mut parts = Vec::new();
    if gold > 0 {
        parts.push(format!("{gold}{}", icon("Gold")));
    }
    if silver > 0 {
        parts.push(format!("{silver}{}", icon("Silver")));
    }
    if copper > 0 || parts.is_empty() {
        parts.push(format!("{copper}{}", icon("Copper")));
    }
    Ok(vec![vm.str(&parts.join(" "))])
}

/// `C_Map.GetAreaInfo(areaID)`: the area's name as the client answered
/// (`data/camelot-answers.txt`: the zones and capitals probed, 1637
/// "Orgrimmar"); an ID naming no area nil (the full probe: -1, 0,
/// 2147483647; unverified: where the areas' IDs end, here a million);
/// else none, noted as unanswered.
fn get_area_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(v) = crate::recorded::lookup(vm, "C_Map.GetAreaInfo", &c) {
        return Ok(v);
    }
    if let Value::Num(n) = vm.hidden(c.arg(1))
        && !(1.0..1_000_000.0).contains(&n)
    {
        return Ok(vec![Value::Nil]);
    }
    crate::note_default(vm, "C_Map.GetAreaInfo", &c, 1);
    Ok(vec![vm.str("")])
}

/// `ClassicExpansionAtLeast(level)`: true for every level (the user's
/// probe: true for 0 to 4; LibDualSpec's note "ClassicExpansionAtLeast is
/// always true in Forever"; unverified above 4).
fn classic_expansion_at_least(vm: &mut Vm, c: NativeCall) -> Ret {
    vm.check_number(&c, 1)?;
    Ok(vec![Value::Bool(true)])
}

/// `ClassicExpansionAtMost(level)`: false for every level (the user's
/// probe: false for 0 to 4, so Forever takes LibDualSpec's own branch;
/// unverified above 4).
fn classic_expansion_at_most(vm: &mut Vm, c: NativeCall) -> Ret {
    vm.check_number(&c, 1)?;
    Ok(vec![Value::Bool(false)])
}

/// `GetThreatStatusColor(status)`: grey, yellow, orange, red for 0 to 3,
/// magenta above, white at 4294967295, as single-precision floats
/// (warcraft.wiki.gg's values), each a byte's fraction (the full probe:
/// grey 176/255, yellow's blue 119/255; orange's 153/255 unverified).
fn get_threat_status_color(vm: &mut Vm, c: NativeCall) -> Ret {
    let status = vm.check_number(&c, 1)?;
    // A byte read back as the machine does (the Mac's 119 is
    // 0.46666669845581, decision T236).
    let platform = crate::engine(vm).platform;
    let byte = |b: f32| crate::byte_channel(platform, b);
    let rgb: [f64; 3] = match status as i64 {
        0 => [byte(176.0), byte(176.0), byte(176.0)],
        1 => [1.0, 1.0, byte(119.0)],
        2 => [1.0, byte(153.0), 0.0],
        3 => [1.0, 0.0, 0.0],
        4294967295 => [1.0, 1.0, 1.0],
        _ => [1.0, 0.0, 1.0],
    };
    Ok(rgb.iter().map(|v| Value::Num(*v)).collect())
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        (
            "C_DateAndTime.GetCurrentCalendarTime",
            get_current_calendar_time,
        ),
        ("IsPlayerInWorld", is_player_in_world),
        ("GetLocale", get_locale),
        ("GetText", get_text),
        ("ClassicExpansionAtLeast", classic_expansion_at_least),
        ("ClassicExpansionAtMost", classic_expansion_at_most),
        ("GetThreatStatusColor", get_threat_status_color),
        ("GetCurrentRegion", get_current_region),
        ("GetCurrentRegionName", get_current_region_name),
        (
            "C_CurrencyInfo.GetCoinTextureString",
            get_coin_texture_string,
        ),
        ("C_Map.GetAreaInfo", get_area_info),
        ("GetFileIDFromPath", get_file_id_from_path),
        ("C_Calendar.GetMonthInfo", get_month_info),
        ("C_Calendar.SetMonth", set_month),
        ("C_Calendar.SetAbsMonth", set_abs_month),
        (
            "C_AuraContainerUtil.ProcessAuraTooltipBackdropOptions",
            process_backdrop,
        ),
        (
            "C_AuraContainerUtil.ProcessAuraTooltipNineSliceOptions",
            process_nine_slice,
        ),
        (
            "C_AuraContainerUtil.ProcessAuraTooltipTextureSliceOptions",
            process_texture_slice,
        ),
        (
            "C_AuraContainerUtil.ProcessCustomAuraButtonApplicationBarOptions",
            process_application_bar,
        ),
        (
            "C_AuraContainerUtil.ProcessCustomAuraButtonApplicationCountOptions",
            process_application_count,
        ),
        (
            "C_AuraContainerUtil.ProcessCustomAuraButtonCasterNameOptions",
            process_caster_name,
        ),
        (
            "C_AuraContainerUtil.ProcessCustomAuraButtonDispelTypeTextOptions",
            process_dispel_text,
        ),
        (
            "C_AuraContainerUtil.ProcessCustomAuraButtonDispelTypeTextureOptions",
            process_dispel_texture,
        ),
        (
            "C_AuraContainerUtil.ProcessCustomAuraButtonDurationBarOptions",
            process_duration_bar,
        ),
        (
            "C_AuraContainerUtil.ProcessCustomAuraButtonDurationTextOptions",
            process_duration_text,
        ),
        ("C_Calendar.GetNumDayEvents", get_num_day_events),
        ("C_Calendar.GetDayEvent", get_day_event),
        ("C_Spell.IsCurrentSpell", is_current_spell),
        ("C_Spell.GetSpellLevelLearned", get_spell_level_learned),
        ("C_Widget.IsWidget", is_widget),
        ("C_Widget.IsFrameWidget", is_frame_widget),
        ("C_Widget.IsRenderableWidget", is_renderable_widget),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn calendar_arithmetic() {
        // 2026-10-19 was a Monday (recorded: weekday 2); 1 October 2026 a
        // Thursday.
        assert_eq!(super::weekday(2026, 10, 19), 2);
        assert_eq!(super::weekday(2026, 10, 1), 5);
        assert_eq!(super::days_in(2024, 2), 29);
        assert_eq!(super::days_in(2026, 2), 28);
        assert_eq!(super::days_in(2026, 10), 31);
    }
}
