//! What the client answers to calls about itself rather than the world
//! (`/stdtest engine`, `loadCalls`: functions Blizzard's code calls at
//! load, without arguments): camelot's recorded answers
//! (`data/camelot-load-calls.txt`), and neutral ones where the client
//! answered for the probed character, place or session (decision: a
//! fresh, plain world; tests set what they need).
//!
//! Unverified: the neutral answers' exact shape (`GetRestState` when not
//! rested, `GetInstanceInfo` outside any map, `GetZonePVPInfo` without a
//! zone). `GetFonts`' order: creation order here; the client's changes
//! between sessions (two runs of `/stdtest engine`).

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
    // This call's own answer first (`/stdtest engine answers`), else the
    // answer recorded at load.
    if let Some(values) = crate::recorded::lookup(vm, &name, &c) {
        return Ok(values);
    }
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

fn empty_list(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// No group: no members of any role.
fn group_member_counts(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = vm.new_table();
    for role in ["TANK", "HEALER", "DAMAGER", "NOROLE"] {
        let k = vm.str(role);
        let _ = vm.heap.table_mut(t).set(k, Value::Num(0.0));
    }
    Ok(vec![Value::Table(t)])
}

/// No role chosen for the group finder.
fn saved_roles(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = vm.new_table();
    for role in ["tank", "healer", "dps"] {
        let k = vm.str(role);
        let _ = vm.heap.table_mut(t).set(k, Value::Bool(false));
    }
    Ok(vec![Value::Table(t)])
}

fn four_zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 4])
}

/// No traffic, no latency.
fn net_stats(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 4])
}

/// `GetMirrorTimerInfo(index)`: no timer runs (`"UNKNOWN"`, which
/// Blizzard's MirrorTimer.lua skips; the other values unverified).
fn mirror_timer_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    let unknown = vm.str("UNKNOWN");
    let empty = vm.str("");
    Ok(vec![
        unknown,
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Bool(false),
        empty,
    ])
}

fn no_role(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("NONE")])
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
/// The zone's PvP status (a city's: "friendly", false, its faction),
/// else nothing (recorded: Durotar's).
fn zone_pvp_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    match engine(vm).world.character.position.pvp.clone() {
        Some((kind, ffa, faction)) => Ok(vec![vm.str(&kind), Value::Bool(ffa), vm.str(&faction)]),
        None => Ok(vec![]),
    }
}

/// The continent the player is on (its name and ID; recorded after a
/// Hearthstone: "Kalimdor", 1), else outside any: the 11 results empty.
fn instance_info(vm: &mut Vm, _c: NativeCall) -> Ret {
    let (empty, none) = (vm.str(""), vm.str("none"));
    let zero = Value::Num(0.0);
    let (name, id) = match engine(vm).world.character.position.instance.clone() {
        Some((n, id)) => (vm.str(&n), Value::Num(id)),
        None => (empty, zero),
    };
    Ok(vec![
        name,
        none,
        zero,
        empty,
        zero,
        zero,
        Value::Bool(false),
        id,
        zero,
        Value::Nil,
        Value::Bool(false),
    ])
}

fn realm_name(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("Realm")])
}

/// Not rested: state 2, normal experience.
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

/// `GetGlobalEnvironment()`: the addons' globals, from secure code too.
fn global_environment(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(engine(vm).main_globals)])
}

/// `SwapToGlobalEnvironment()`: the calling code (a secure file) goes on
/// with the addons' globals (Blizzard_StoreUI's toggle, Blizzard_PingUI).
fn swap_to_global_environment(vm: &mut Vm, _c: NativeCall) -> Ret {
    let main = engine(vm).main_globals;
    vm.set_caller_env(1, main);
    Ok(vec![])
}

/// Inventory slot names: ID, empty slot texture, whether it checks relics
/// (`/stdtest engine`, `slotInfo`).
const INVENTORY_SLOTS: &[(&str, f64, f64, bool)] = &[
    ("AMMOSLOT", 0.0, 136520.0, false),
    ("HEADSLOT", 1.0, 136516.0, false),
    ("NECKSLOT", 2.0, 136519.0, false),
    ("SHOULDERSLOT", 3.0, 136526.0, false),
    ("SHIRTSLOT", 4.0, 136525.0, false),
    ("CHESTSLOT", 5.0, 136512.0, false),
    ("WAISTSLOT", 6.0, 136529.0, false),
    ("LEGSSLOT", 7.0, 136517.0, false),
    ("FEETSLOT", 8.0, 136513.0, false),
    ("WRISTSLOT", 9.0, 136530.0, false),
    ("HANDSSLOT", 10.0, 136515.0, false),
    ("FINGER0SLOT", 11.0, 136514.0, false),
    ("FINGER1SLOT", 12.0, 136514.0, false),
    ("TRINKET0SLOT", 13.0, 136528.0, false),
    ("TRINKET1SLOT", 14.0, 136528.0, false),
    ("BACKSLOT", 15.0, 136512.0, false),
    ("MAINHANDSLOT", 16.0, 136518.0, false),
    ("SECONDARYHANDSLOT", 17.0, 136524.0, false),
    ("RANGEDSLOT", 18.0, 136520.0, true),
    ("TABARDSLOT", 19.0, 136527.0, false),
    ("BAG0SLOT", 31.0, 4760248.0, false),
    ("BAG1SLOT", 32.0, 4760248.0, false),
    ("BAG2SLOT", 33.0, 4760248.0, false),
    ("BAG3SLOT", 34.0, 4760248.0, false),
    ("REAGENTBAG0SLOT", 35.0, 4760248.0, false),
];

/// `GetInventorySlotInfo(slotName)`: the slot's ID, its empty texture,
/// whether it checks relics; another name (the profession slots) as the
/// client answered it.
fn inventory_slot_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = match c.arg(1) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).to_uppercase(),
        _ => return Ok(vec![]),
    };
    Ok(INVENTORY_SLOTS
        .iter()
        .find(|(n, ..)| *n == name)
        .map(|(_, id, texture, relic)| {
            vec![Value::Num(*id), Value::Num(*texture), Value::Bool(*relic)]
        })
        .unwrap_or_else(|| crate::recorded::client_answer(vm, &c)))
}

/// The client's transmog slot locations (`C_TransmogOutfitInfo.
/// GetAllSlotLocationInfo`, recorded): each `{slot, slotName, isSecondary,
/// …}`.
fn slot_locations() -> Vec<&'static Vec<(String, Rec)>> {
    match recorded()
        .get("C_TransmogOutfitInfo.GetAllSlotLocationInfo")
        .and_then(|r| r.first())
    {
        Some(Rec::Table(list)) => list
            .iter()
            .filter_map(|(_, v)| match v {
                Rec::Table(fields) => Some(fields),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn rec_field<'a>(fields: &'a [(String, Rec)], key: &str) -> Option<&'a Rec> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// `C_TransmogOutfitInfo.GetTransmogOutfitSlotFromInventorySlot(slot)`
/// (0-based inventory slot): the primary transmog slot of that inventory
/// slot's name, else nothing. The client checks no argument (`/stdtest
/// argcheck`: a number for `'x'`, `{}`): a non-number counts as 0.
fn outfit_slot_from_inventory_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    let inv = match vm.hidden(c.arg(1)) {
        Value::Num(n) => n.trunc(),
        _ => 0.0,
    } + 1.0;
    let Some((name, ..)) = INVENTORY_SLOTS.iter().find(|(_, id, ..)| *id == inv) else {
        return Ok(vec![]);
    };
    for loc in slot_locations() {
        let named = matches!(rec_field(loc, "slotName"), Some(Rec::Str(s)) if s == name);
        let primary = matches!(rec_field(loc, "isSecondary"), Some(Rec::Bool(false)));
        if named
            && primary
            && let Some(Rec::Num(slot)) = rec_field(loc, "slot")
        {
            return Ok(vec![Value::Num(*slot)]);
        }
    }
    Ok(vec![])
}

/// `C_TransmogOutfitInfo.GetLinkedSlotInfo(slot)`: a slot with a secondary
/// of the same name (shoulders): `{primarySlotInfo, secondarySlotInfo}`,
/// else nothing.
fn linked_slot_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let slot = vm.check_int(&c, 1)? as f64;
    let locs = slot_locations();
    let Some(primary) = locs
        .iter()
        .find(|l| matches!(rec_field(l, "slot"), Some(Rec::Num(n)) if *n == slot))
    else {
        return Ok(vec![]);
    };
    let name = rec_field(primary, "slotName").cloned();
    let Some(secondary) = locs.iter().find(|l| {
        rec_field(l, "slotName").cloned() == name
            && matches!(rec_field(l, "isSecondary"), Some(Rec::Bool(true)))
    }) else {
        return Ok(vec![]);
    };
    let info = Rec::Table(vec![
        ("primarySlotInfo".into(), Rec::Table((*primary).clone())),
        ("secondarySlotInfo".into(), Rec::Table((*secondary).clone())),
    ]);
    Ok(vec![build(vm, &info)])
}

/// `C_Log.LogErrorMessage(message)`: the client's log of Lua errors
/// (Blizzard's error handler writes every error there): the engine's list
/// of Blizzard's errors, first line.
fn log_error_message(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Value::Str(s) = vm.hidden(c.arg(1)) {
        let text = String::from_utf8_lossy(vm.str_bytes(s)).into_owned();
        let first = text
            .lines()
            .next()
            .unwrap_or("")
            .trim_start_matches("Lua Error: ");
        let ui = &mut crate::engine_mut(vm).world.ui;
        ui.errors.push(first.to_string());
        ui.error_stacks.push(text);
    }
    Ok(vec![])
}

/// The functions with behaviour here.
pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_Log.LogErrorMessage", log_error_message),
        (
            "C_TransmogOutfitInfo.GetTransmogOutfitSlotFromInventorySlot",
            outfit_slot_from_inventory_slot,
        ),
        ("C_TransmogOutfitInfo.GetLinkedSlotInfo", linked_slot_info),
        ("C_PaperDollInfo.GetInventorySlotInfo", inventory_slot_info),
        ("GetInventorySlotInfo", inventory_slot_info),
        ("GetGlobalEnvironment", global_environment),
        ("SwapToGlobalEnvironment", swap_to_global_environment),
        ("C_ColorOverrides.GetColorForQuality", color_for_quality),
        (
            "C_Container.CalculateTotalNumberOfFreeBagSlots",
            free_bag_slots,
        ),
        ("C_PvP.GetZonePVPInfo", zone_pvp_info),
        ("C_PetJournal.GetNumPets", zeros),
        ("C_VoiceChat.GetTtsVoices", empty_list),
        ("GetCursorPosition", zeros),
        ("GetFonts", fonts),
        ("GetGameTime", game_time),
        ("GetInstanceInfo", instance_info),
        ("GetPhysicalScreenSize", physical_screen_size),
        ("GetRealmName", realm_name),
        ("GetScreenHeight", screen_height),
        ("GetActiveLootRollIDs", empty_list),
        ("C_LFGList.GetApplications", empty_list),
        // No pet battle, no nameplates.
        ("C_PetBattles.GetNumPets", zero),
        ("C_PetBattles.GetHealth", zero),
        ("C_PetBattles.GetMaxHealth", zero),
        ("C_PetBattles.GetPower", zero),
        ("C_PetBattles.GetSpeed", zero),
        ("C_PetBattles.GetLevel", zero),
        ("C_PetBattles.GetXP", zeros),
        ("C_NamePlate.GetNamePlateForUnit", nothing),
        ("C_NamePlate.GetNamePlates", empty_list),
        ("C_LFGListRoles.GetSavedRoles", saved_roles),
        ("GetGroupMemberCounts", group_member_counts),
        ("GetNetStats", net_stats),
        ("GetMirrorTimerInfo", mirror_timer_info),
        // No group, no role.
        ("UnitGroupRolesAssigned", no_role),
        // The probed character's (`loadCalls`); the neutral world's 0.
        ("GetMaxBattlefieldID", zero),
        ("GetNumLanguages", zero),
        // No forms (a warrior's stances aside: unverified).
        ("GetNumShapeshiftForms", zero),
        ("GuildControlGetNumRanks", zero),
        ("BNGetNumFriends", four_zeros),
        ("C_LFGList.GetAvailableCategories", empty_list),
        ("C_MountJournal.GetNumMountsNeedingFanfare", zero),
        ("C_PetJournal.GetNumPetsNeedingFanfare", zero),
        ("GetSpellConfirmationPromptsInfo", empty_list),
        ("IsGuildLeader", no),
        // Not a convention kiosk.
        ("Kiosk.IsEnabled", no),
        // No money on the cursor, no trade: Blizzard's money frames
        // subtract them from GetMoney() (without an error in the client).
        ("GetCursorMoney", zero),
        ("GetPlayerTradeMoney", zero),
        ("GetTargetTradeMoney", zero),
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
