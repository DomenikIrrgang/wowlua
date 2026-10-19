//! Answers addons ask of the client that no other module models: the
//! calendar time, the guild's club, the damage meter's session length, a
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

/// `C_Club.GetGuildClubId()`: none outside a guild (unverified: a guild's
/// club ID, noted as unanswered).
fn get_guild_club_id(vm: &mut Vm, c: NativeCall) -> Ret {
    if engine(vm).world.chat.guild {
        crate::note_default(vm, "C_Club.GetGuildClubId", &c, 0);
    }
    Ok(vec![Value::Nil])
}

/// `C_DamageMeter.GetSessionDurationSeconds(type)`: none before a session
/// (unverified: the engine keeps no damage meter sessions).
fn get_session_duration_seconds(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
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

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        (
            "C_DateAndTime.GetCurrentCalendarTime",
            get_current_calendar_time,
        ),
        ("C_Club.GetGuildClubId", get_guild_club_id),
        (
            "C_DamageMeter.GetSessionDurationSeconds",
            get_session_duration_seconds,
        ),
        ("C_Spell.IsCurrentSpell", is_current_spell),
        ("C_Spell.GetSpellLevelLearned", get_spell_level_learned),
        ("C_Widget.IsWidget", is_widget),
        ("C_Widget.IsFrameWidget", is_frame_widget),
        ("C_Widget.IsRenderableWidget", is_renderable_widget),
    ]
}
