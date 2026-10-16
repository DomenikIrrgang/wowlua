//! What the client answers to calls about itself rather than the world
//! (`/stdtest engine`, `loadCalls`: functions Blizzard's code calls at
//! load, without arguments): camelot's recorded answers
//! (`data/camelot-load-calls.txt`), and neutral ones where the client
//! answered for the probed character, place or session (decision: a
//! fresh, plain world; tests set what they need).
//!
//! Unverified: the neutral answers' exact shape (`GetRestState` when not
//! rested, `GetInstanceInfo` outside any map, `GetZonePVPInfo` without a
//! zone) and `GetFonts`' order (creation order here).

use std::collections::HashMap;
use std::sync::OnceLock;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::{engine, widgets};

/// A recorded value: the probe's text of it.
#[derive(Clone, Debug, PartialEq)]
pub enum Rec {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    /// A method of the mixin the table was made with (`ColorMixin`).
    Func,
    /// Keys as the probe shows them (`1`, `r`), sorted as strings.
    Table(Vec<(String, Rec)>),
}

/// Parses one value at `s`, ending (for a string) before `stop`: `, ` at
/// the top, `,` or `}` in a table.
fn value(s: &str, top: bool) -> Option<(Rec, &str)> {
    if let Some(mut rest) = s.strip_prefix('{') {
        let mut entries = Vec::new();
        if let Some(r) = rest.strip_prefix('}') {
            return Some((Rec::Table(entries), r));
        }
        loop {
            let (key, r) = rest.split_once('=')?;
            let (v, r) = value(r, false)?;
            entries.push((key.to_string(), v));
            if let Some(r) = r.strip_prefix(',') {
                rest = r;
            } else {
                return Some((Rec::Table(entries), r.strip_prefix('}')?));
            }
        }
    }
    if let Some(r) = s.strip_prefix("function") {
        return Some((Rec::Func, r));
    }
    if let Some(r) = s.strip_prefix("nil nil") {
        return Some((Rec::Nil, r));
    }
    let (ty, r) = s.split_once(' ')?;
    let end = if top {
        r.find(", ").unwrap_or(r.len())
    } else {
        r.find([',', '}']).unwrap_or(r.len())
    };
    let (text, rest) = r.split_at(end);
    let v = match ty {
        "boolean" => Rec::Bool(text == "true"),
        "number" => Rec::Num(text.parse().ok()?),
        "string" => Rec::Str(text.to_string()),
        _ => return None,
    };
    Some((v, rest))
}

/// A call's results as the probe shows them: `2: number 0, boolean true`.
pub fn parse_results(text: &str) -> Option<Vec<Rec>> {
    let (count, mut rest) = text.split_once(": ")?;
    let count: usize = count.parse().ok()?;
    let mut out = Vec::new();
    while out.len() < count {
        let (v, r) = value(rest, true)?;
        out.push(v);
        rest = r.strip_prefix(", ").unwrap_or(r);
    }
    rest.is_empty().then_some(out)
}

/// The probe's text of results (the inverse of `parse_results`).
pub fn show_results(values: &[Rec]) -> String {
    fn show(v: &Rec, out: &mut String) {
        match v {
            Rec::Nil => out.push_str("nil nil"),
            Rec::Bool(b) => out.push_str(&format!("boolean {b}")),
            Rec::Num(n) => out.push_str(&format!("number {}", wowlua_vm::number::fmt_num(*n))),
            Rec::Str(s) => out.push_str(&format!("string {s}")),
            Rec::Func => out.push_str("function"),
            Rec::Table(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(k);
                    out.push('=');
                    show(v, out);
                }
                out.push('}');
            }
        }
    }
    let parts: Vec<String> = values
        .iter()
        .map(|v| {
            let mut s = String::new();
            show(v, &mut s);
            s
        })
        .collect();
    format!("{}: {}", values.len(), parts.join(", "))
}

fn recorded() -> &'static HashMap<String, Vec<Rec>> {
    static DATA: OnceLock<HashMap<String, Vec<Rec>>> = OnceLock::new();
    DATA.get_or_init(|| {
        include_str!("../data/camelot-load-calls.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (name, results) = l.split_once('\t')?;
                Some((name.to_string(), parse_results(results)?))
            })
            .collect()
    })
}

/// The functions with recorded answers (camelot).
pub fn recorded_names(flavor: &str) -> Vec<&'static str> {
    if flavor != "camelot" {
        return Vec::new();
    }
    let mut names: Vec<&'static str> = recorded().keys().map(String::as_str).collect();
    names.sort_unstable();
    names
}

/// A recorded value in the VM: list keys as numbers; a table with methods
/// gets them from its mixin (`ColorMixin`, when Blizzard's code defined it).
fn build(vm: &mut Vm, v: &Rec) -> Value {
    match v {
        Rec::Nil | Rec::Func => Value::Nil,
        Rec::Bool(b) => Value::Bool(*b),
        Rec::Num(n) => Value::Num(*n),
        Rec::Str(s) => vm.str(s),
        Rec::Table(entries) => {
            let t = vm.new_table();
            let mut mixin = false;
            for (k, inner) in entries {
                if *inner == Rec::Func {
                    mixin = true;
                    continue;
                }
                let key = match k.parse::<u32>() {
                    Ok(i) if i.to_string() == *k => Value::Num(f64::from(i)),
                    _ => vm.str(k),
                };
                let value = build(vm, inner);
                let _ = vm.heap.table_mut(t).set(key, value);
            }
            if mixin && let Value::Table(m) = vm.get_global("ColorMixin") {
                let methods: Vec<(Value, Value)> = vm.heap.table(m).entries().collect();
                for (k, f) in methods {
                    let _ = vm.heap.table_mut(t).set(k, f);
                }
            }
            Value::Table(t)
        }
    }
}

/// A recorded function: its answers, whatever the arguments.
pub fn recorded_call(vm: &mut Vm, c: NativeCall) -> Ret {
    // Its name, or (documented) its index in the API.
    let name = match c.upvals.first().copied() {
        Some(Value::Str(s)) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Some(Value::Num(i)) => engine(vm).api.functions[i as usize].name.to_string(),
        _ => return Err(vm.error("a recorded function without its name")),
    };
    let values = recorded().get(&name).cloned().unwrap_or_default();
    Ok(values.iter().map(|v| build(vm, v)).collect())
}

// ------------------------------------------------- the neutral world

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn empty_string(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("")])
}

fn empty_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0)])
}

/// An empty backpack: its 16 slots.
fn free_bag_slots(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(16.0)])
}

/// Noon.
fn game_time(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(12.0), Value::Num(0.0)])
}

/// No zone: no PvP type, not a PvP subzone, no faction.
fn zone_pvp_info(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil, Value::Bool(false), Value::Nil])
}

/// Outside any instance and map: the client's 11 results, empty.
fn instance_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (empty, none) = (vm.str(""), vm.str("none"));
    let zero = Value::Num(0.0);
    Ok(vec![
        empty,
        none,
        zero,
        empty,
        zero,
        zero,
        Value::Bool(false),
        zero,
        zero,
        Value::Nil,
        Value::Bool(false),
    ])
}

fn realm_name(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("Realm")])
}

/// Not rested: state 2, normal experience.
fn rest_state(vm: &mut Vm, _c: NativeCall) -> Ret {
    let name = vm.str("Normal");
    Ok(vec![Value::Num(2.0), name, Value::Num(1.0)])
}

fn physical_screen_size(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Num(widgets::SCREEN.0),
        Value::Num(widgets::SCREEN.1),
    ])
}

fn screen_height(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(widgets::SCREEN.1)])
}

/// The font objects made, in order: by name, else as `tostring` gives
/// them (the client lists unnamed ones so).
fn fonts(vm: &mut Vm, _c: NativeCall) -> Ret {
    let made: Vec<(Option<String>, Value)> = engine(vm)
        .world
        .widgets
        .iter()
        .filter(|w| w.class == "Font")
        .map(|w| (w.name.clone(), Value::Table(w.table)))
        .collect();
    let t = vm.new_table();
    for (i, (name, table)) in made.into_iter().enumerate() {
        let v = match name {
            Some(n) => vm.str(&n),
            None => vm.tostring(table).unwrap_or(Value::Nil),
        };
        let _ = vm.heap.table_mut(t).set(Value::Num((i + 1) as f64), v);
    }
    Ok(vec![Value::Table(t)])
}

/// Each item quality's color (`/stdtest engine`, `qualityColors`): red,
/// green, blue, alpha.
const QUALITY_COLORS: [[f64; 4]; 9] = [
    [0.61568629741669, 0.61568629741669, 0.61568629741669, 1.0],
    [1.0, 1.0, 1.0, 1.0],
    [0.11764706671238, 1.0, 0.0, 1.0],
    [0.0, 0.43921571969986, 0.8666667342186, 1.0],
    [0.63921570777893, 0.20784315466881, 0.93333339691162, 1.0],
    [1.0, 0.50196081399918, 0.0, 1.0],
    [0.90196084976196, 0.80000007152557, 0.50196081399918, 1.0],
    [0.0, 0.80000007152557, 1.0, 1.0],
    [0.0, 0.80000007152557, 1.0, 1.0],
];

/// `C_ColorOverrides.GetColorForQuality(quality)`: a `ColorMixin` color
/// (camelot's; an unknown quality gives nothing, unverified).
fn color_for_quality(vm: &mut Vm, c: NativeCall) -> Ret {
    let q = vm.check_int(&c, 1)?;
    let Some(rgba) = usize::try_from(q).ok().and_then(|q| QUALITY_COLORS.get(q)) else {
        return Ok(vec![Value::Nil]);
    };
    let fields: Vec<(String, Rec)> = ["a", "b", "g", "r"]
        .iter()
        .zip([rgba[3], rgba[2], rgba[1], rgba[0]])
        .map(|(k, v)| (k.to_string(), Rec::Num(v)))
        .chain(std::iter::once(("GetRGB".to_string(), Rec::Func)))
        .collect();
    Ok(vec![build(vm, &Rec::Table(fields))])
}

/// The functions with behaviour here.
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_ColorOverrides.GetColorForQuality", color_for_quality),
        (
            "C_Container.CalculateTotalNumberOfFreeBagSlots",
            free_bag_slots,
        ),
        ("C_PvP.GetZonePVPInfo", zone_pvp_info),
        ("C_PetJournal.GetNumPets", zeros),
        ("C_QuestLog.GetNumQuestLogEntries", zeros),
        ("C_VoiceChat.GetTtsVoices", empty_list),
        ("GetCursorPosition", zeros),
        ("GetFonts", fonts),
        ("GetGameTime", game_time),
        ("GetInstanceInfo", instance_info),
        ("GetMinimapZoneText", empty_string),
        ("GetPhysicalScreenSize", physical_screen_size),
        ("GetRealmName", realm_name),
        ("GetRestState", rest_state),
        ("GetScreenHeight", screen_height),
        ("GetActiveLootRollIDs", empty_list),
        ("GetSpellConfirmationPromptsInfo", empty_list),
        ("GetXPExhaustion", nothing),
        ("IsGuildLeader", no),
        // Not a convention kiosk.
        ("Kiosk.IsEnabled", no),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every recorded answer parses, and shows as the probe showed it.
    #[test]
    fn the_recorded_answers_read_back_as_recorded() {
        let mut lines = 0;
        for line in include_str!("../data/camelot-load-calls.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let (name, text) = line.split_once('\t').unwrap();
            let values = parse_results(text).unwrap_or_else(|| panic!("{name} parses"));
            assert_eq!(show_results(&values), text, "{name}");
            lines += 1;
        }
        assert_eq!(recorded().len(), lines);
    }

    #[test]
    fn results_text() {
        let r = parse_results("3: string , number 0, {1={a=boolean true,b=function}}").unwrap();
        assert_eq!(r[0], Rec::Str(String::new()));
        assert_eq!(r[1], Rec::Num(0.0));
        assert_eq!(
            r[2],
            Rec::Table(vec![(
                "1".into(),
                Rec::Table(vec![("a".into(), Rec::Bool(true)), ("b".into(), Rec::Func)])
            )])
        );
        assert_eq!(parse_results("0: ").unwrap(), vec![]);
    }
}
