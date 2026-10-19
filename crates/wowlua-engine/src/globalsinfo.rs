//! The client's global functions no other module answers: the game's
//! systems Forever lacks or the engine does not model (the encounter
//! journal, guild banks, looking for group, archaeology, war games,
//! vehicles, the camera), answered in their "nothing here" state; the
//! client's settings an addon sets and reads back (kept per engine); and
//! the few that follow from the engine's world (the gear the player wears,
//! the group, the pet, the player's faction).
//!
//! Each answer names its source: a recording (`/stdtest engine answers`,
//! `tests/client/forever-camelot-answers.lua`), Blizzard's documentation
//! and UI code, warcraft.wiki.gg, or the function's name. Everything not
//! from a recording is unverified.

use std::rc::Rc;

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::{engine, units, widgets};

// ------------------------------------------------------------ helpers

/// Another behaviour's answer to a call made from here (as
/// `conditionals.rs` asks), its values unwrapped; none without one.
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

/// An argument as text, if it is a string.
fn text_arg(vm: &Vm, c: &NativeCall, n: usize) -> Option<String> {
    match vm.hidden(c.arg(n)) {
        Value::Str(s) => Some(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
        _ => None,
    }
}

/// An argument as a number, if it is one.
fn num_arg(vm: &Vm, c: &NativeCall, n: usize) -> Option<f64> {
    match vm.hidden(c.arg(n)) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// The settings this module keeps (a table in the registry, one per
/// engine): what a setter set, for its getter.
fn state(vm: &mut Vm) -> TableRef {
    let registry = vm.registry;
    let key = vm.heap.intern(b"wowlua.engine.globalsinfo");
    match vm.heap.table(registry).get_str(key) {
        Value::Table(t) => t,
        _ => {
            let t = vm.new_table();
            vm.heap.table_mut(registry).set_str(key, Value::Table(t));
            t
        }
    }
}

fn get(vm: &mut Vm, key: &str) -> Value {
    let t = state(vm);
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn put(vm: &mut Vm, key: &str, v: Value) {
    let t = state(vm);
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

/// A kept setting, or `default` before any setter ran.
fn get_or(vm: &mut Vm, key: &str, default: Value) -> Value {
    match get(vm, key) {
        Value::Nil => default,
        v => v,
    }
}

// ------------------------------------------------------------ generic answers

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

fn two_zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Num(0.0)])
}

fn three_zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 3])
}

fn six_zeros(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0); 6])
}

fn two_falses(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false), Value::Bool(false)])
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

/// The table the caller passed (filled with nothing), else a new empty
/// one: the LFG lists' order and state functions fill the caller's table.
fn passed_table(vm: &mut Vm, c: NativeCall) -> Ret {
    match c.arg(1) {
        Value::Table(t) => Ok(vec![Value::Table(t)]),
        _ => Ok(vec![Value::Table(vm.new_table())]),
    }
}

// ------------------------------------------------------------ table aliases

/// Blizzard's `TableUtil.lua` makes these globals the table library's
/// functions (`TableIsEmpty = table.isempty`); before it loads, the
/// client's C functions of the same names answer as those do.
fn table_alias(vm: &mut Vm, c: &NativeCall, field: &str) -> Ret {
    let table = vm.get_global("table");
    let Value::Table(t) = table else {
        return Ok(vec![]);
    };
    let k = vm.str(field);
    let f = vm.heap.table(t).get(k);
    if f.is_nil() {
        return Ok(vec![]);
    }
    let args: Vec<Value> = (1..=c.len()).map(|i| c.arg(i)).collect();
    vm.call(f, &args)
}

fn table_is_empty(vm: &mut Vm, c: NativeCall) -> Ret {
    table_alias(vm, &c, "isempty")
}

fn count_table(vm: &mut Vm, c: NativeCall) -> Ret {
    table_alias(vm, &c, "count")
}

fn get_keys_array(vm: &mut Vm, c: NativeCall) -> Ret {
    table_alias(vm, &c, "keys")
}

fn get_values_array(vm: &mut Vm, c: NativeCall) -> Ret {
    table_alias(vm, &c, "values")
}

fn t_contains(vm: &mut Vm, c: NativeCall) -> Ret {
    table_alias(vm, &c, "contains")
}

fn t_index_of(vm: &mut Vm, c: NativeCall) -> Ret {
    table_alias(vm, &c, "indexof")
}

fn t_delete_item(vm: &mut Vm, c: NativeCall) -> Ret {
    table_alias(vm, &c, "removevalue")
}

fn t_unordered_remove(vm: &mut Vm, c: NativeCall) -> Ret {
    table_alias(vm, &c, "removeunordered")
}

// ------------------------------------------------------------ the player's gear

/// The class and subclass of the item worn in an inventory slot
/// (`C_Item.GetItemInfoInstant`'s sixth and seventh results).
fn worn_class(vm: &mut Vm, slot: i64) -> Option<(f64, f64)> {
    let id = engine(vm).world.character.equipment.get(&slot)?.id;
    let info = ask(vm, "C_Item.GetItemInfoInstant", &[Value::Num(id as f64)]);
    match (info.get(5), info.get(6)) {
        (Some(Value::Num(class)), Some(Value::Num(sub))) => Some((*class, *sub)),
        _ => None,
    }
}

const WEAPON: f64 = 2.0;
const OFF_HAND: i64 = 17;
const RANGED: i64 = 18;

/// `IsDualWielding()`: a weapon in the off hand (a shield or a held item
/// is armor). Recorded false for a character without one; unverified for
/// one with.
fn is_dual_wielding(vm: &mut Vm, _c: NativeCall) -> Ret {
    let dual = worn_class(vm, OFF_HAND).is_some_and(|(class, _)| class == WEAPON);
    Ok(vec![Value::Bool(dual)])
}

/// `IsRangedWeapon()`: a bow, gun, crossbow or thrown weapon in the ranged
/// slot (recorded false without one; unverified which weapons count:
/// warcraft.wiki.gg does not say, wands left out).
fn is_ranged_weapon(vm: &mut Vm, _c: NativeCall) -> Ret {
    let ranged = worn_class(vm, RANGED)
        .is_some_and(|(class, sub)| class == WEAPON && matches!(sub as i64, 2 | 3 | 16 | 18));
    Ok(vec![Value::Bool(ranged)])
}

/// `HasWandEquipped()`: a wand (weapon subclass 19) in the ranged slot
/// (from the name; unverified).
fn has_wand_equipped(vm: &mut Vm, _c: NativeCall) -> Ret {
    let wand = worn_class(vm, RANGED).is_some_and(|(class, sub)| class == WEAPON && sub == 19.0);
    Ok(vec![Value::Bool(wand)])
}

/// `CanDualWield()`: the player knows Dual Wield (spell 674), as the
/// spellbook answers (unverified).
fn can_dual_wield(vm: &mut Vm, _c: NativeCall) -> Ret {
    let known = ask(vm, "C_SpellBook.IsSpellKnown", &[Value::Num(674.0)])
        .first()
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(known)])
}

// ------------------------------------------------------------ the world

/// `IsUnitModelReadyForUI(unit)`: a unit that exists (recorded true for
/// the player; unverified for other units and for none).
fn is_unit_model_ready(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(units::unit_arg(vm, &c, 1).is_some())])
}

/// `PlayerIsInCombat()`: the player in combat, as `UnitAffectingCombat`
/// answers (from the name; unverified).
fn player_is_in_combat(vm: &mut Vm, _c: NativeCall) -> Ret {
    let player = vm.str("player");
    let combat = ask(vm, "UnitAffectingCombat", &[player])
        .first()
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(combat)])
}

/// `IsOutdoors()`: not under a roof, as `IsIndoors` answers (the macro
/// conditionals' rule; unverified: caves, instances).
fn is_outdoors(vm: &mut Vm, _c: NativeCall) -> Ret {
    let indoors = ask(vm, "IsIndoors", &[])
        .first()
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(!indoors)])
}

/// `GroupHasOfflineMember()`: a member of the player's group offline
/// (from the name; unverified).
fn group_has_offline_member(vm: &mut Vm, _c: NativeCall) -> Ret {
    let u = &engine(vm).units;
    let offline = u.group.iter().any(|id| !u.list[id.0].online);
    Ok(vec![Value::Bool(offline)])
}

/// `PetHasSpellbook()`, `PetCanBeDismissed()`: the player has a pet (from
/// the names; unverified: a pet without abilities, a charmed one).
fn has_pet(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).units.pet.is_some())])
}

/// `CanLootUnit(guid)`: whether the unit has loot and the player may take
/// it: a dead unit with loot left (unverified: tapping, a group's loot
/// rules).
fn can_loot_unit(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(guid) = text_arg(vm, &c, 1) else {
        return Ok(vec![Value::Bool(false), Value::Bool(false)]);
    };
    let loot = engine(vm)
        .units
        .list
        .iter()
        .find(|u| u.guid == guid)
        .is_some_and(|u| u.dead && !u.loot.is_empty());
    Ok(vec![Value::Bool(loot), Value::Bool(loot)])
}

/// `GetDefaultLanguage()`, `GetAlternativeDefaultLanguage()`: the
/// player's faction's language and its ID, the faction as
/// `UnitFactionGroup` answers (recorded "Orcish", 1 for a Horde
/// character; the Alliance's Common, 7, unverified).
fn get_default_language(vm: &mut Vm, _c: NativeCall) -> Ret {
    let player = vm.str("player");
    let alliance = match ask(vm, "UnitFactionGroup", &[player]).first() {
        Some(Value::Str(s)) => vm.str_bytes(*s) == b"Alliance",
        _ => false,
    };
    let (name, id) = if alliance {
        ("Common", 7.0)
    } else {
        ("Orcish", 1.0)
    };
    Ok(vec![vm.str(name), Value::Num(id)])
}

/// `SelectedRealmName()`: the realm the player is on, as `GetRealmName`
/// answers (from the name; unverified).
fn selected_realm_name(vm: &mut Vm, _c: NativeCall) -> Ret {
    match ask(vm, "GetRealmName", &[]).first().copied() {
        Some(v @ Value::Str(_)) => Ok(vec![v]),
        _ => Ok(vec![vm.str("")]),
    }
}

/// `GetLocalGameTime()`: the game's time of day, as `GetGameTime` answers
/// (unverified: the client's local and server times are the same here).
fn get_local_game_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    let t = ask(vm, "GetGameTime", &[]);
    let hour = t.first().copied().unwrap_or(Value::Num(0.0));
    let minute = t.get(1).copied().unwrap_or(Value::Num(0.0));
    Ok(vec![hour, minute])
}

/// `GetDefaultScale()`: the UI scale that makes the screen 768 units high
/// at its physical height (`widgets::SCREEN`), Blizzard's long-standing
/// rule (unverified on Forever).
fn get_default_scale(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(768.0 / widgets::SCREEN.1)])
}

/// `C_Spell.GetSchoolString`'s global twin `GetSchoolString(mask)`: its
/// answer (recorded "Frost" for 16, "Fire" for 4), else a single school's
/// name (unverified).
fn get_school_string(vm: &mut Vm, c: NativeCall) -> Ret {
    let mask = c.arg(1);
    let asked = ask(vm, "C_Spell.GetSchoolString", &[mask]);
    if let Some(v @ Value::Str(_)) = asked.first().copied() {
        return Ok(vec![v]);
    }
    let name = match num_arg(vm, &c, 1).map(|n| n as i64) {
        Some(1) => "Physical",
        Some(2) => "Holy",
        Some(4) => "Fire",
        Some(8) => "Nature",
        Some(16) => "Frost",
        Some(32) => "Shadow",
        Some(64) => "Arcane",
        _ => "",
    };
    Ok(vec![vm.str(name)])
}

/// The global twin of `C_VideoOptions.IsOutlineModeSupported()`: its
/// answer (recorded false).
fn is_outline_mode_supported(vm: &mut Vm, _c: NativeCall) -> Ret {
    let supported = ask(vm, "C_VideoOptions.IsOutlineModeSupported", &[])
        .first()
        .is_some_and(|v| v.truthy());
    Ok(vec![Value::Bool(supported)])
}

/// `DoesTemplateExist(name)`: a template the loaded XML defined, as
/// `C_XMLUtil.GetTemplateInfo` knows it (from the name; unverified).
fn does_template_exist(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = c.arg(1);
    let found = ask(vm, "C_XMLUtil.GetTemplateInfo", &[name])
        .first()
        .is_some_and(|v| !v.is_nil());
    Ok(vec![Value::Bool(found)])
}

/// `GetClickFrame(name)`: the frame a global of that name holds, else nil
/// (warcraft.wiki.gg; unverified).
fn get_click_frame(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = text_arg(vm, &c, 1) else {
        return Ok(vec![Value::Nil]);
    };
    let v = vm.get_global(&name);
    if widgets::widget_of(vm, v).is_some() {
        Ok(vec![v])
    } else {
        Ok(vec![Value::Nil])
    }
}

/// `RunBinding(command [, "up"])`: runs the binding's code as a key press
/// would, down unless "up" (warcraft.wiki.gg; unverified).
fn run_binding(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(command) = text_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let state = match text_arg(vm, &c, 2).as_deref() {
        Some("up") => "up",
        _ => "down",
    };
    crate::bindings::run_command(vm, &command, state);
    Ok(vec![])
}

/// `CreateUnitHealPredictionCalculator()`: a new calculator object of the
/// documented class (its methods are the documentation's; unverified).
fn create_heal_prediction_calculator(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![widgets::create(
        vm,
        "UnitHealPredictionCalculator",
        None,
        None,
    )])
}

/// `CreateFontFamily(name, members)`: a named font object with the first
/// member's file, height and flags (the documentation's
/// `CreateFontFamilyMemberInfo`; unverified: the other alphabets' members
/// are not kept).
fn create_font_family(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = crate::lua_helper(
        vm,
        "CreateFontFamily",
        "return function(name, members)
            local font = CreateFont(name)
            local m = type(members) == 'table' and members[1]
            if type(m) == 'table' and m.file then
                font:SetFont(m.file, m.height or 12, m.flags or '')
            end
            return font
        end",
    )?;
    vm.call(f, &[c.arg(1), c.arg(2)])
}

/// `CopyToClipboard(text [, removeMarkup])`: the length copied, the text's
/// in bytes (the documentation's `length`; the engine has no clipboard;
/// unverified with markup removed).
fn copy_to_clipboard(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = match vm.hidden(c.arg(1)) {
        Value::Str(s) => vm.str_bytes(s).len(),
        _ => 0,
    };
    Ok(vec![Value::Num(n as f64)])
}

/// `CalculateStringEditDistance(a, b)`: the Levenshtein distance between
/// the two strings' characters (from the name; unverified).
fn calculate_string_edit_distance(vm: &mut Vm, c: NativeCall) -> Ret {
    let a: Vec<char> = text_arg(vm, &c, 1).unwrap_or_default().chars().collect();
    let b: Vec<char> = text_arg(vm, &c, 2).unwrap_or_default().chars().collect();
    Ok(vec![Value::Num(edit_distance(&a, &b) as f64)])
}

fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let next = (row[j + 1] + 1).min(row[j] + 1).min(prev + cost);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// `CaseAccentInsensitiveParse(name)`: the name in lower case with the
/// Latin accents taken off, a form searches compare (from the name;
/// unverified).
fn case_accent_insensitive_parse(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = text_arg(vm, &c, 1).unwrap_or_default();
    let out: String = name.to_lowercase().chars().map(fold_accent).collect();
    Ok(vec![vm.str(&out)])
}

fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

/// `GetGameMessageInfo(index)`: the message's name, from the client's
/// `LE_GAME_ERR_*` globals (`LE_GAME_ERR_X` = index gives `ERR_X`); no
/// sound kit or voice (unverified); an unknown index nothing.
fn get_game_message_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(index) = num_arg(vm, &c, 1) else {
        return Ok(vec![]);
    };
    let name = include_str!("../data/camelot-globals.txt")
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .find(|(name, n)| name.starts_with("LE_GAME_ERR_") && n.parse::<f64>().ok() == Some(index))
        .map(|(name, _)| name.trim_start_matches("LE_GAME_"));
    match name {
        Some(name) => Ok(vec![vm.str(name), Value::Nil, Value::Nil]),
        None => Ok(vec![]),
    }
}

/// `GetMaxLevelForExpansionLevel(expansion)`: the level cap each
/// expansion had before the level squish (60 for the classic game, as
/// `GetMaxPlayerLevel` recorded; the rest unverified), 0 for one unknown.
fn get_max_level_for_expansion_level(vm: &mut Vm, c: NativeCall) -> Ret {
    const CAPS: [f64; 12] = [
        60.0, 70.0, 80.0, 85.0, 90.0, 100.0, 110.0, 120.0, 60.0, 70.0, 80.0, 90.0,
    ];
    let level = num_arg(vm, &c, 1)
        .filter(|n| *n >= 0.0)
        .and_then(|n| CAPS.get(n as usize).copied())
        .unwrap_or(0.0);
    Ok(vec![Value::Num(level)])
}

/// `GetMaxLevelForLatestExpansion()`: the classic game's cap, 60, as
/// `GetMaxLevelForPlayerExpansion` recorded (unverified).
fn sixty(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(60.0)])
}

/// `GetNumClasses()`: Classic's nine classes (warcraft.wiki.gg: on
/// Classic it is not the highest class ID; unverified on Forever).
fn get_num_classes(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(9.0)])
}

/// `GetRealmID()`, `GetNativeRealmID()`: recorded 4618 (the native realm's
/// unverified).
fn get_realm_id(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(4618.0)])
}

/// `GetWorldElapsedTime(timerID)`: no timer (recorded `"", 0, 0` for 0;
/// unverified for other IDs).
fn get_world_elapsed_time(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str(""), Value::Num(0.0), Value::Num(0.0)])
}

/// `GetNextAvailableRaidTargetMarkerIndex(start, …)`: with no marker in
/// use, the start (recorded 1 from 1, 8 from 8 backwards; unverified with
/// markers placed).
fn next_raid_target_marker(vm: &mut Vm, c: NativeCall) -> Ret {
    let start = num_arg(vm, &c, 1).unwrap_or(1.0).clamp(1.0, 8.0);
    Ok(vec![Value::Num(start.floor())])
}

/// `GetBaseDifficultyID(difficultyID)`: the difficulty itself (no
/// difficulty derives from another in the classic game; unverified).
fn get_base_difficulty_id(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Num(num_arg(vm, &c, 1).unwrap_or(0.0))])
}

/// `GetTaskInfo(questID)`: not a task quest: not in its area, not on the
/// map, no objectives (from the declaration; unverified).
fn get_task_info(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Bool(false),
        Value::Bool(false),
        Value::Num(0.0),
    ])
}

/// `InGuildParty()`: not a guild group, no guild members, none needed,
/// no bonus (from the declaration; unverified).
fn in_guild_party(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![
        Value::Bool(false),
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Num(0.0),
    ])
}

/// `GetMovieDownloadProgress(movieID)`: not downloading, nothing of
/// nothing (the documentation's shape; unverified).
fn get_movie_download_progress(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false), Value::Num(0.0), Value::Num(0.0)])
}

// ------------------------------------------------------------ chat colors

/// `ChangeChatColor(chatType, r, g, b)`: tells `UPDATE_CHAT_COLOR(chatType,
/// r, g, b)`, the event Blizzard's chat frames set `ChatTypeInfo` from
/// (their handler; unverified: when the client sends it).
fn change_chat_color(vm: &mut Vm, c: NativeCall) -> Ret {
    let args = [c.arg(1), c.arg(2), c.arg(3), c.arg(4)];
    widgets::fire_event(vm, "UPDATE_CHAT_COLOR", &args);
    Ok(vec![])
}

/// `SetChatColorNameByClass(chatType, colorNameByClass)`: tells
/// `UPDATE_CHAT_COLOR_NAME_BY_CLASS(chatType, colorNameByClass)` (the chat
/// frames' handler; unverified as above).
fn set_chat_color_name_by_class(vm: &mut Vm, c: NativeCall) -> Ret {
    let args = [c.arg(1), c.arg(2)];
    widgets::fire_event(vm, "UPDATE_CHAT_COLOR_NAME_BY_CLASS", &args);
    Ok(vec![])
}

// ------------------------------------------------------------ kept settings

/// A getter and setter pair over one kept value: the setter's first
/// argument kept, the getter's answer it or the default. Defaults are
/// from the names and warcraft.wiki.gg (unverified).
macro_rules! setting {
    ($getter:ident, $setter:ident, $key:literal, $default:expr) => {
        fn $getter(vm: &mut Vm, _c: NativeCall) -> Ret {
            Ok(vec![get_or(vm, $key, $default)])
        }

        fn $setter(vm: &mut Vm, c: NativeCall) -> Ret {
            let v = vm.hidden(c.arg(1));
            put(vm, $key, v);
            Ok(vec![])
        }
    };
}

// The guild bank's open tab, the first (warcraft.wiki.gg).
setting!(
    get_current_guild_bank_tab,
    set_current_guild_bank_tab,
    "guildBankTab",
    Value::Num(1.0)
);
// The encounter journal's tier, the first.
setting!(
    ej_get_current_tier,
    ej_select_tier,
    "ejTier",
    Value::Num(1.0)
);
// The encounter journal's difficulty, normal dungeons (1).
setting!(
    ej_get_difficulty,
    ej_set_difficulty,
    "ejDifficulty",
    Value::Num(1.0)
);
setting!(
    get_opt_out_of_loot,
    set_opt_out_of_loot,
    "optOutOfLoot",
    Value::Bool(false)
);
setting!(
    get_allow_low_level_raid,
    set_allow_low_level_raid,
    "allowLowLevelRaid",
    Value::Bool(false)
);
setting!(
    get_auto_decline_neighborhood_invites,
    set_auto_decline_neighborhood_invites,
    "autoDeclineNeighborhood",
    Value::Bool(false)
);
setting!(
    is_european_numbers,
    set_european_numbers,
    "europeanNumbers",
    Value::Bool(false)
);
setting!(
    get_error_callstack_height,
    set_error_callstack_height,
    "errorCallstackHeight",
    Value::Nil
);
setting!(
    are_dangerous_scripts_allowed,
    set_allow_dangerous_scripts,
    "dangerousScripts",
    Value::Bool(false)
);
setting!(
    get_selected_display_channel,
    set_selected_display_channel,
    "displayChannel",
    Value::Nil
);
setting!(
    get_selected_war_game_type,
    set_selected_war_game_type,
    "warGameType",
    Value::Nil
);
// The helmet and the cloak shown, as a new character has them.
setting!(showing_helm, show_helm, "showHelm", Value::Bool(true));
setting!(showing_cloak, show_cloak, "showCloak", Value::Bool(true));

/// `EJ_GetLootFilter()`: the class and specialization the journal's loot
/// is filtered by, none (0, 0) until `EJ_SetLootFilter` (unverified).
fn ej_get_loot_filter(vm: &mut Vm, _c: NativeCall) -> Ret {
    let class = get_or(vm, "ejLootClass", Value::Num(0.0));
    let spec = get_or(vm, "ejLootSpec", Value::Num(0.0));
    Ok(vec![class, spec])
}

fn ej_set_loot_filter(vm: &mut Vm, c: NativeCall) -> Ret {
    let class = vm.hidden(c.arg(1));
    let spec = vm.hidden(c.arg(2));
    put(vm, "ejLootClass", class);
    put(vm, "ejLootSpec", spec);
    Ok(vec![])
}

fn ej_reset_loot_filter(vm: &mut Vm, _c: NativeCall) -> Ret {
    put(vm, "ejLootClass", Value::Nil);
    put(vm, "ejLootSpec", Value::Nil);
    Ok(vec![])
}

/// `LoggingCombat([newState])`, `LoggingChat([newState])`: a boolean given
/// turns logging on or off; the answer is whether it is on (off at first;
/// warcraft.wiki.gg; unverified).
fn logging(vm: &mut Vm, c: &NativeCall, key: &str) -> Ret {
    if let Value::Bool(b) = vm.hidden(c.arg(1)) {
        put(vm, key, Value::Bool(b));
    }
    Ok(vec![get_or(vm, key, Value::Bool(false))])
}

fn logging_combat(vm: &mut Vm, c: NativeCall) -> Ret {
    logging(vm, &c, "loggingCombat")
}

fn logging_chat(vm: &mut Vm, c: NativeCall) -> Ret {
    logging(vm, &c, "loggingChat")
}

/// `GetSheathState()`: 1 with the weapons sheathed, as a character stands
/// out of combat, 2 drawn after `ToggleSheath()` (Blizzard's
/// `ModelSceneUtil` reads 1 as sheathed; unverified: the client draws them
/// in combat).
fn get_sheath_state(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![get_or(vm, "sheath", Value::Num(1.0))])
}

fn toggle_sheath(vm: &mut Vm, _c: NativeCall) -> Ret {
    let drawn = matches!(get(vm, "sheath"), Value::Num(n) if n == 2.0);
    put(vm, "sheath", Value::Num(if drawn { 1.0 } else { 2.0 }));
    Ok(vec![])
}

/// `ToggleSelfHighlight()`: the player's highlight switched; whether it is
/// now on (off at first; the documentation's `enabled`; unverified).
fn toggle_self_highlight(vm: &mut Vm, _c: NativeCall) -> Ret {
    let on = !get(vm, "selfHighlight").truthy();
    put(vm, "selfHighlight", Value::Bool(on));
    Ok(vec![Value::Bool(on)])
}

// ------------------------------------------------------------ the lists

/// Actions and requests the engine has nothing to do for: no visible
/// state of its own changes (camera moves, requests to the server whose
/// answers it does not model, systems Forever lacks, windows' settings
/// kept by the client), so they accept and do nothing (from their names
/// and declarations; unverified that no event follows).
const NOTHING: &[&str] = &[
    // Loot, groups, duels, trade.
    "SetLootSpecialization",
    "AcceptGroup",
    "DeclineGroup",
    "ConfirmBNRequestInviteFriend",
    "RespondToInviteConfirmation",
    "GiveMasterLoot",
    "DoMasterLootRoll",
    "CancelMasterLootRoll",
    "LootMoneyNotify",
    "SetLootPortrait",
    "StartDuel",
    "AcceptDuel",
    "CancelDuel",
    "ForfeitDuel",
    "BeginTrade",
    "CancelTrade",
    "AcceptTrade",
    "CancelTradeAccept",
    "ClickTradeButton",
    "ClickTargetTradeButton",
    "SetTradeCurrency",
    "PlaceRaidMarker",
    "ClearRaidMarker",
    "RemoveRaidTargets",
    "RespondInstanceLock",
    "SetSavedInstanceExtend",
    // The client window, the camera, the console, profiling.
    "FlashClientIcon",
    "SetView",
    "SaveView",
    "ResetView",
    "NextView",
    "PrevView",
    "CameraZoomIn",
    "CameraZoomOut",
    "FlipCameraYaw",
    "CenterCamera",
    "MoveViewLeftStart",
    "MoveViewLeftStop",
    "MoveViewRightStart",
    "MoveViewRightStop",
    "MoveViewUpStart",
    "MoveViewUpStop",
    "MoveViewDownStart",
    "MoveViewDownStop",
    "MoveViewInStart",
    "MoveViewInStop",
    "MoveViewOutStart",
    "MoveViewOutStop",
    "LaunchURL",
    "LoadURLIndex",
    "ResetCPUUsage",
    "UpdateAddOnCPUUsage",
    "ConsolePrintAllMatchingCommands",
    "SetConsoleKey",
    "RestartGx",
    "UpdateWindow",
    "ToggleWindowed",
    "AutoChooseCurrentGraphicsSetting",
    "SetCurrentGraphicsSetting",
    "SetSelectedScreenResolutionIndex",
    "SetGamePadCursorControl",
    "SetGamePadFreeLook",
    "SetPreferredGamepadInteractTarget",
    "DetectWowMouse",
    "SetInWorldUIVisibility",
    "ProcessExceptionClient",
    "ReportBug",
    "ReportSuggestion",
    "ForceLogout",
    "Stuck",
    "SendSubscriptionInterstitialResponse",
    "HandleAtlasMemberCommand",
    "ClearOutage",
    "SetPOIIconOverlapDistance",
    "SetPOIIconOverlapPushDistance",
    "SetTaxiBenchmarkMode",
    "RequestBottomLeftActionBar",
    "NeutralPlayerSelectFaction",
    // The world map's action button (none in the classic game).
    "ClickWorldMapActionButton",
    "UseWorldMapActionButtonSpellOnQuest",
    // Chat windows and channels (the client keeps their settings).
    "SetChatWindowSize",
    "SetChatWindowColor",
    "SetChatWindowShown",
    "SetChatWindowDocked",
    "SetChatWindowLocked",
    "SetChatWindowAlpha",
    "SetChatWindowName",
    "SetChatWindowSavedDimensions",
    "SetChatWindowSavedPosition",
    "SetChatWindowUninteractable",
    "RedockChatWindows",
    "ResetChatWindows",
    "ResetChatColors",
    "ChannelToggleAnnouncements",
    "ChannelSetAllSilent",
    "ChannelSetPartyMemberSilent",
    "DeclineChannelInvite",
    // Tutorials, cinematics, movies, scenes.
    "TriggerTutorial",
    "FlagTutorial",
    "ResetTutorials",
    "ClearTutorials",
    "StopCinematic",
    "CinematicStarted",
    "CinematicFinished",
    "OpeningCinematic",
    "MouseOverrideCinematicDisable",
    "PreloadMovie",
    "CancelPreloadingMovie",
    "CancelScene",
    // The encounter journal (not in the classic game).
    "EJ_SelectInstance",
    "EJ_SelectEncounter",
    "EJ_SetSearch",
    "EJ_ClearSearch",
    "EJ_EndSearch",
    // Guild banks, guild management, petitions (the engine's guild has
    // no bank, ranks or roster).
    "QueryGuildBankTab",
    "QueryGuildBankLog",
    "QueryGuildBankText",
    "PickupGuildBankItem",
    "SplitGuildBankItem",
    "AutoStoreGuildBankItem",
    "DepositGuildBankMoney",
    "WithdrawGuildBankMoney",
    "PickupGuildBankMoney",
    "BuyGuildBankTab",
    "CloseGuildBankFrame",
    "SetGuildBankTabInfo",
    "SetGuildBankTabItemWithdraw",
    "SetGuildBankTabPermissions",
    "SetGuildBankText",
    "SetGuildBankWithdrawGoldLimit",
    "QueryGuildNews",
    "QueryGuildRecipes",
    "QueryGuildEventLog",
    "RequestGuildChallengeInfo",
    "RequestGuildPartyState",
    "RequestGuildRewards",
    "SetGuildNewsFilter",
    "SetGuildInfoText",
    "SetGuildMemberRank",
    "SetGuildTradeSkillCategoryFilter",
    "SetGuildTradeSkillItemNameFilter",
    "SortGuildTradeSkill",
    "CollapseGuildTradeSkillHeader",
    "ExpandGuildTradeSkillHeader",
    "CloseGuildRoster",
    "CloseGuildRegistrar",
    "ViewGuildRecipes",
    "ReplaceGuildMaster",
    "SubmitRequiredGuildRename",
    "TurnInGuildCharter",
    "BuyGuildCharter",
    "AcceptGuild",
    "DeclineGuild",
    "SetAutoDeclineGuildInvites",
    "SetAllowRecentAlliesSeeLocation",
    "RenamePetition",
    "SignPetition",
    "OfferPetition",
    "ClosePetition",
    // Looking for group, battlegrounds, arenas, war games.
    "JoinLFG",
    "LeaveLFG",
    "JoinSingleLFG",
    "LeaveSingleLFG",
    "ClearAllLFGDungeons",
    "CompleteLFGRoleCheck",
    "CompleteLFGReadyCheck",
    "AcceptProposal",
    "RejectProposal",
    "PartyLFGStartBackfill",
    "RefreshLFGList",
    "RequestLFDPartyLockInfo",
    "RequestLFDPlayerLockInfo",
    "SearchLFGJoin",
    "SearchLFGLeave",
    "SearchLFGSort",
    "JoinArena",
    "JoinRatedBattlefield",
    "JoinRatedSoloShuffle",
    "JoinSkirmish",
    "RequeueSkirmish",
    "SurrenderArena",
    "StartWarGame",
    "StartWarGameByName",
    "StartSoloShuffleWarGameByName",
    "StartSpectatorWarGame",
    "StartSpectatorSoloShuffleWarGame",
    "WarGameRespond",
    "UpdateWarGamesList",
    "CollapseWarGameHeader",
    "ExpandWarGameHeader",
    "BattlefieldMgrQueueInviteResponse",
    "BattlefieldMgrEntryInviteResponse",
    "BattlefieldMgrExitRequest",
    "BattlefieldMgrQueueRequest",
    "RequestRatedInfo",
    "RequestPVPRewards",
    "RequestPVPOptionsEnabled",
    "RequestRandomBattlegroundInstanceInfo",
    "RequestBattlegroundInstanceInfo",
    "RequestBattlefieldScoreData",
    "SetBattlefieldScoreFaction",
    "SortBattlefieldScoreData",
    "ReportPlayerIsPVPAFK",
    "SetPVPRoles",
    "ClearBattlemaster",
    // Achievements' comparison.
    "ClearAchievementComparisonUnit",
    "SetFocusedAchievement",
    "SwitchAchievementSearchTab",
    "ClearAchievementSearchString",
    // Archaeology, artifacts, glyphs, PvP talents, specializations
    // (not in the classic game).
    "SolveArtifact",
    "CloseResearch",
    "SetSelectedArtifact",
    "AttachGlyphToSpell",
    "LearnPvpTalent",
    "LearnPvpTalents",
    "RemovePvpTalent",
    "ClearFailedPVPTalentIDs",
    "ClearFailedTalentIDs",
    "UnlearnSpecialization",
    "ConfirmTalentWipe",
    // Quests' pop-ups, sounds and map icons, the greeting's choices.
    "AddAutoQuestPopUp",
    "RemoveAutoQuestPopUp",
    "AcknowledgeAutoAcceptQuest",
    "PlayAutoAcceptQuestSound",
    "ClearAutoAcceptQuestSound",
    "QuestMapUpdateAllQuests",
    "QuestPOIUpdateIcons",
    "ProcessQuestLogRewardFactions",
    "SortQuests",
    "SortQuestSortTypes",
    "QuestChooseRewardError",
    "SelectActiveQuest",
    "SelectAvailableQuest",
    // Death, spirit healers, graveyards.
    "AcceptAreaSpiritHeal",
    "CancelAreaSpiritHeal",
    "HearthAndResurrectFromArea",
    "SetCemeteryPreference",
    "PortGraveyard",
    // Vehicles (not in the classic game).
    "VehicleExit",
    "EjectPassengerFromSeat",
    "VehicleNextSeat",
    "VehiclePrevSeat",
    "VehicleAimDecrement",
    "VehicleAimIncrement",
    "VehicleAimDownStart",
    "VehicleAimDownStop",
    "VehicleAimUpStart",
    "VehicleAimUpStop",
    "VehicleAimRequestAngle",
    "VehicleAimSetNormPower",
    "CancelPetPossess",
    // Items: pending equips and bind conversions, sockets, item text.
    "EquipPendingItem",
    "CancelPendingEquip",
    "ClearPendingBindConversionItem",
    "ConvertItemToBindToAccount",
    "SocketInventoryItem",
    "UpdateInventoryAlertStatus",
    "CloseItemText",
    "ItemTextNextPage",
    "ItemTextPrevPage",
    // Macros' icons, spell confirmation prompts, surveys and tickets.
    "SetMacroItem",
    "SetMacroSpell",
    "GetLooseMacroIcons",
    "GetLooseMacroItemIcons",
    "AcceptSpellConfirmationPrompt",
    "DeclineSpellConfirmationPrompt",
    "AcknowledgeSurvey",
    "GMSurveyAnswerSubmit",
    "GMSurveyCommentSubmit",
    "GMSurveySubmit",
    "GMReportLag",
    "GMRequestPlayerInfo",
    "GMResponseResolve",
    "DeleteGMTicket",
];

/// "No" in the classic game or with nothing modeled: no vehicle, no
/// pending glyph (recorded false), no new boost (recorded false), not PvP
/// inactive (recorded false), no timerunning, no flying, no gamepad, no
/// guild rank to edit with, no LFG or war games, no archaeology, no
/// cinematic (from the names and Blizzard's documentation; the rest
/// unverified).
const NO: &[&str] = &[
    "PlayerIsTimerunning",
    "PlayerVehicleHasComboPoints",
    "PlayerIsPVPInactive",
    "PlayerIsSpellTarget",
    "IsCharacterNewlyBoosted",
    "HasPendingGlyphCast",
    "IsPendingGlyphRemoval",
    "IsCastingGlyph",
    "IsFlyableArea",
    "IsAdvancedFlyableArea",
    "IsDrivableArea",
    "IsSubmerged",
    "IsOutOfBounds",
    "IsUsingGamepad",
    "CanGamePadControlCursor",
    "IsGamePadCursorControlEnabled",
    "IsGamePadFreelookEnabled",
    "IsLeftMetaKeyDown",
    "IsRightMetaKeyDown",
    "IsUsingFixedTimeStep",
    "IsDebugBuild",
    "IsPublicTestClient",
    "IsOnTournamentRealm",
    "IsExpansionTrial",
    "IsDemonHunterAvailable",
    "CanUpgradeToCurrentExpansion",
    "ShouldShowExpansionUpgradeBanner",
    "ShouldShowSpecialSplashScreen",
    "SplashFrameCanBeShown",
    "ShouldShowIslandsWeeklyPOI",
    "ShouldDisplayStoryModeTooltipInSocialFrame",
    "RegionalUniqueNamesEnabled",
    "ConsoleIsActive",
    "IsChatAFK",
    "IsChatDND",
    "IsChatChannelRaid",
    "IsRaidMarkerActive",
    "IsRaidMarkerSystemEnabled",
    "IsArenaSkirmish",
    "IsWargame",
    "IsPartyWorldPVP",
    "IsServerControlledBackfill",
    "CanPartyLFGBackfill",
    "CanInitiateWarGame",
    "CanJoinBattlefieldAsGroup",
    "DungeonAppearsInRandomLFD",
    "InitiateRolePoll",
    "SortBGList",
    "IsInJailersTower",
    "IsInsane",
    "IsGuildMember",
    "IsGuildRankAssignmentAllowed",
    "IsInAuthenticatedRank",
    "CanEditMOTD",
    "CanEditGuildInfo",
    "CanEditGuildTabInfo",
    "CanEditPublicNote",
    "CanEditGuildEvent",
    "CanEditGuildBankTabInfo",
    "CanReplaceGuildMaster",
    "CanViewGuildRecipes",
    "CanWithdrawGuildBankMoney",
    "CanSignPetition",
    "SetAchievementComparisonUnit",
    "IsMasterLooter",
    "CanMapChangeDifficulty",
    "IsLegacyDifficulty",
    "EJ_IsValidInstanceDifficulty",
    "EJ_InstanceIsRaid",
    "EJ_IsLootListOutOfDate",
    "FlyoutHasSpell",
    "IsSelectedSpellBookItem",
    "TargetSpellReplacesBonusTree",
    "IsSpecializationActivateSpell",
    "HasLootSpecializations",
    "AreTalentsLocked",
    "CheckTalentMasterDist",
    "CanResetTutorials",
    "AreClassRolesSoftSuggestions",
    "CanShowSetRoleButton",
    "CanSolveArtifact",
    "CanScanResearchSite",
    "ItemAddedToArtifact",
    "RemoveItemFromArtifact",
    "CanItemBeSocketedToArtifact",
    "IsMovieLocal",
    "IsMoviePlayable",
    "IsMovieReadable",
    "IsInCinematicScene",
    "CanCancelScene",
    "IsUsingVehicleControls",
    "CanSwitchVehicleSeat",
    "CanSwitchVehicleSeats",
    "CanEjectPassengerFromSeat",
    "IsVehicleAimAngleAdjustable",
    "IsVehicleAimPowerAdjustable",
    "HasDualWieldPenalty",
    "HasIgnoreDualWieldWeapon",
    "HasAPEffectsSpellPower",
    "HasSPEffectsAttackPower",
    "HasKey",
    "ReagentsFromBankAllowed",
    "DoesCurrentLocaleSellExpansionLevels",
    "ItemCanTargetGarrisonFollowerAbility",
    "CannotBeResurrected",
    "IsCemeterySelectionAvailable",
    "CanPortGraveyard",
    "QuestHasPOIInfo",
    "IsBreadcrumbQuest",
    "IsStoryQuest",
    "QuestIsDaily",
    "QuestIsWeekly",
    "QuestGetAutoLaunched",
    "IsActiveQuestTrivial",
    "IsAvailableQuestTrivial",
    "ItemTextHasNextPage",
    "ItemTextIsFullPage",
    "GMEuropaBugsEnabled",
    "GMEuropaComplaintsEnabled",
    "GMEuropaSuggestionsEnabled",
    "GMEuropaTicketsEnabled",
    "GMItemRestorationButtonEnabled",
    "GMQuickTicketSystemEnabled",
    "GMQuickTicketSystemThrottled",
];

/// "Yes": the cursor shown (recorded true), a 64-bit client, the mouse in
/// use, quest data at hand (the engine's quests carry theirs; from the
/// names, unverified but the cursor).
const YES: &[&str] = &[
    "IsCursorVisible",
    "Is64BitClient",
    "IsUsingMouse",
    "HaveQuestData",
    "HaveQuestRewardData",
];

/// Counts and amounts of nothing: no LFG dungeons, scenarios, war games
/// or battleground types listed; no archaeology; no unspent PvP talents;
/// no CPU profiled; no corruption, PvP power, bandwidth, billing time,
/// eviction or boot timers; the classic game's expansion levels (0, as
/// `GetClassicExpansionLevel` and `GetExpansionLevel` recorded); no
/// declension sets for enUS (from the names and documentation; the rest
/// unverified).
const ZERO: &[&str] = &[
    "GetNumRFDungeons",
    "GetNumRandomScenarios",
    "GetNumScenarios",
    "GetNumFlexRaidDungeons",
    "GetNumDungeonForRandomSlot",
    "GetLFDLockPlayerCount",
    "GetNumWarGameTypes",
    "GetNumBattlegroundTypes",
    "GetNumTreasurePickerItems",
    "GetNumArchaeologyRaces",
    "GetNumArtifactsByRace",
    "ArchaeologyMapUpdateAll",
    "ArcheologyGetVisibleBlobID",
    "GetNumUnspentPvpTalents",
    "GetNumPetitionNames",
    "GetNumDeclensionSets",
    "GetAddOnCPUUsage",
    "GetScriptCPUUsage",
    "GetCollapsingStarCost",
    "GetCorruption",
    "GetCorruptionResistance",
    "GetPvpPowerDamage",
    "GetPvpPowerHealing",
    "GetAvailableBandwidth",
    "GetBillingTimeRested",
    "GetEvictionTimeRemaining",
    "GetInstanceBootTimeRemaining",
    "GetAreaSpiritHealerTime",
    "GetReadyCheckTimeLeft",
    "GetJailersTowerLevel",
    "GetPreviousArenaSeason",
    "GetDailyQuestsCompleted",
    "GetMaxRewardCurrencies",
    "GetNumActiveQuests",
    "GetNumAvailableQuests",
    "GetServerExpansionLevel",
    "GetMinimumExpansionLevel",
    "GetMaximumExpansionLevel",
    "EJ_GetNumTiers",
    "EJ_GetNumSearchResults",
    "EJ_GetSearchSize",
    "EJ_GetSearchProgress",
    "EJ_GetNumLoot",
    "EJ_GetNumEncountersForLootByIndex",
    "EJ_GetInvTypeSortOrder",
    "EJ_GetContentTuningID",
    "EJ_GetInstanceForMap",
    "GMSurveyNumAnswers",
    "VehicleAimGetNormPower",
    "ItemTextGetPage",
];

/// No such thing: an unknown index's or ID's info, no running macro, no
/// glyph, no tutorial next or before, no invite to confirm, no best
/// choice of an LFG list, no window made, no timerunning season (nil, as
/// the declarations' nilable results or warcraft.wiki.gg have it for
/// none; unverified).
const NIL: &[&str] = &[
    "CreateWindow",
    "PlayerGetTimerunningSeasonID",
    "GetRFDungeonInfo",
    "GetRandomScenarioInfo",
    "GetFlexRaidDungeonInfo",
    "GetDungeonForRandomSlot",
    "GetLFDLockInfo",
    "GetLFDRoleLockInfo",
    "GetLFDRoleRestrictions",
    "GetWarGameTypeInfo",
    "GetWarGameQueueStatus",
    "GetTreasurePickerItemInfo",
    "SearchLFGGetResults",
    "SearchLFGGetPartyResults",
    "SearchLFGGetEncounterResults",
    "SearchLFGGetJoinedID",
    "GetPartyLFGBackfillInfo",
    "GetBestRFChoice",
    "GetRandomScenarioBestChoice",
    "GetBestFlexRaidChoice",
    "GetSavedWorldBossInfo",
    "GetInviteConfirmationInfo",
    "GetNextPendingInviteConfirmation",
    "GetCallPetSpellInfo",
    "GetPvpTalentInfoByID",
    "GetPvpTalentInfoBySpecialization",
    "GetPvpTalentLink",
    "GetCurrentGlyphNameForSpell",
    "GetPendingGlyphName",
    "GetNextCompleatedTutorial",
    "GetPrevCompleatedTutorial",
    "GetArchaeologyRaceInfo",
    "GetArchaeologyRaceInfoByID",
    "GetArchaeologyInfo",
    "GetArtifactInfoByRace",
    "GetActiveArtifactByRace",
    "GetSelectedArtifactInfo",
    "ArchaeologyGetIconInfo",
    "GetAutoQuestPopUp",
    "GetActiveTitle",
    "GetAvailableTitle",
    "GetActiveLevel",
    "GetAvailableLevel",
    "GetActiveQuestID",
    "GetAvailableQuestInfo",
    "GetRunningMacro",
    "GetRunningMacroButton",
    "GetTargetTradeCurrency",
    "GetCurrentLevelFeatures",
    "GetCurrentEventID",
    "GetSecondsUntilParentalControlsKick",
    "GetMirrorTimerProgress",
    "GetSoundEntryCount",
    "GetCurrentCombatTextEventInfo",
    "GetResSicknessDuration",
    "ItemTextGetItem",
    "ItemTextGetText",
    "ItemTextGetCreator",
    "ItemTextGetMaterial",
    "GMSurveyQuestion",
    // No languages (`GetNumLanguages` answers 0), no world map action
    // button, no garrison followers, no specializations.
    "GetLanguageByIndex",
    "GetWorldMapActionButtonSpellInfo",
    "GetFollowerTypeIDFromSpell",
    "GetJournalInfoForSpellConfirmation",
    "GetPrimarySpecialization",
    "EJ_GetInstanceByIndex",
    "EJ_GetInstanceInfo",
    "EJ_GetEncounterInfo",
    "EJ_GetEncounterInfoByIndex",
    "EJ_GetCreatureInfo",
    "EJ_GetMapEncounter",
    "EJ_GetSearchResult",
    "EJ_GetSectionPath",
    "EJ_GetTierInfo",
    "EJ_HandleLinkPath",
];

/// Results that do not apply say nothing (the documentation marks them
/// `MayReturnNothing`: no event profiled, no source location, no vehicle
/// indicator, no unit of that creature near; unverified).
const RETURNS_NOTHING: &[&str] = &[
    "GetEventTime",
    "GetSourceLocation",
    "GetVehicleUIIndicator",
    "GetVehicleUIIndicatorSeat",
    "ClosestUnitPosition",
    "ClosestGameObjectPosition",
];

/// Two zeros: no CPU used and no calls profiled; no comparison
/// achievements; no LFG search results; no battleground points (the
/// declarations' shapes; unverified).
const TWO_ZEROS: &[&str] = &[
    "GetEventCPUUsage",
    "GetFrameCPUUsage",
    "GetFunctionCPUUsage",
    "GetNumComparisonCompletedAchievements",
    "SearchLFGGetNumResults",
    "GetBattlegroundPoints",
];

/// Empty lists: no failed talents, no credits, no corruption effects, no
/// pending invite confirmations (the documentation's table results;
/// unverified).
const EMPTY_TABLE: &[&str] = &[
    "GetFailedPVPTalentIDs",
    "GetFailedTalentIDs",
    "GetCreditsDisplayInfo",
    "GetNegativeCorruptionEffectInfo",
    "GetPendingInviteConfirmations",
];

/// The LFG lists' order and state functions: the caller's table, nothing
/// added (no dungeons listed; Blizzard's code passes the table to fill;
/// unverified).
const PASSED_TABLE: &[&str] = &[
    "GetLFDChoiceOrder",
    "GetLFDChoiceCollapseState",
    "GetLFDChoiceEnabledState",
    "GetLFRChoiceOrder",
    "GetScenariosChoiceOrder",
    "GetHomePartyInfo",
];

/// `SetLegacyRaidDifficultyID(difficultyID)`: the legacy raid difficulty
/// `GetLegacyRaidDifficultyID` tells (`content`; unverified: no event).
fn set_legacy_raid_difficulty_id(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Value::Num(id) = c.arg(1) {
        crate::content::set_legacy_raid_difficulty(vm, id);
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    let mut out: Vec<(&'static str, NativeFn)> = vec![
        ("TableIsEmpty", table_is_empty),
        ("SetLegacyRaidDifficultyID", set_legacy_raid_difficulty_id),
        ("CountTable", count_table),
        ("GetKeysArray", get_keys_array),
        ("GetValuesArray", get_values_array),
        ("tContains", t_contains),
        ("tIndexOf", t_index_of),
        ("tDeleteItem", t_delete_item),
        ("tUnorderedRemove", t_unordered_remove),
        ("IsDualWielding", is_dual_wielding),
        ("IsRangedWeapon", is_ranged_weapon),
        ("HasWandEquipped", has_wand_equipped),
        ("CanDualWield", can_dual_wield),
        ("IsUnitModelReadyForUI", is_unit_model_ready),
        ("PlayerIsInCombat", player_is_in_combat),
        ("IsOutdoors", is_outdoors),
        ("GroupHasOfflineMember", group_has_offline_member),
        ("PetHasSpellbook", has_pet),
        ("PetCanBeDismissed", has_pet),
        ("CanLootUnit", can_loot_unit),
        ("GetDefaultLanguage", get_default_language),
        ("GetAlternativeDefaultLanguage", get_default_language),
        ("SelectedRealmName", selected_realm_name),
        ("GetLocalGameTime", get_local_game_time),
        ("GetDefaultScale", get_default_scale),
        ("GetSchoolString", get_school_string),
        ("IsOutlineModeSupported", is_outline_mode_supported),
        ("DoesTemplateExist", does_template_exist),
        ("GetClickFrame", get_click_frame),
        ("RunBinding", run_binding),
        (
            "CreateUnitHealPredictionCalculator",
            create_heal_prediction_calculator,
        ),
        ("CreateFontFamily", create_font_family),
        ("CopyToClipboard", copy_to_clipboard),
        (
            "CalculateStringEditDistance",
            calculate_string_edit_distance,
        ),
        ("CaseAccentInsensitiveParse", case_accent_insensitive_parse),
        ("GetGameMessageInfo", get_game_message_info),
        (
            "GetMaxLevelForExpansionLevel",
            get_max_level_for_expansion_level,
        ),
        ("GetMaxLevelForLatestExpansion", sixty),
        ("GetNumClasses", get_num_classes),
        ("GetRealmID", get_realm_id),
        ("GetNativeRealmID", get_realm_id),
        ("GetWorldElapsedTime", get_world_elapsed_time),
        (
            "GetNextAvailableRaidTargetMarkerIndex",
            next_raid_target_marker,
        ),
        ("GetBaseDifficultyID", get_base_difficulty_id),
        ("GetTaskInfo", get_task_info),
        ("InGuildParty", in_guild_party),
        ("GetMovieDownloadProgress", get_movie_download_progress),
        ("ChangeChatColor", change_chat_color),
        ("SetChatColorNameByClass", set_chat_color_name_by_class),
        ("GetCurrentGuildBankTab", get_current_guild_bank_tab),
        ("SetCurrentGuildBankTab", set_current_guild_bank_tab),
        ("EJ_GetCurrentTier", ej_get_current_tier),
        ("EJ_SelectTier", ej_select_tier),
        ("EJ_GetDifficulty", ej_get_difficulty),
        ("EJ_SetDifficulty", ej_set_difficulty),
        ("EJ_GetLootFilter", ej_get_loot_filter),
        ("EJ_SetLootFilter", ej_set_loot_filter),
        ("EJ_ResetLootFilter", ej_reset_loot_filter),
        ("GetOptOutOfLoot", get_opt_out_of_loot),
        ("SetOptOutOfLoot", set_opt_out_of_loot),
        ("GetAllowLowLevelRaid", get_allow_low_level_raid),
        ("SetAllowLowLevelRaid", set_allow_low_level_raid),
        (
            "GetAutoDeclineNeighborhoodInvites",
            get_auto_decline_neighborhood_invites,
        ),
        (
            "SetAutoDeclineNeighborhoodInvites",
            set_auto_decline_neighborhood_invites,
        ),
        ("IsEuropeanNumbers", is_european_numbers),
        ("SetEuropeanNumbers", set_european_numbers),
        ("GetErrorCallstackHeight", get_error_callstack_height),
        ("SetErrorCallstackHeight", set_error_callstack_height),
        ("AreDangerousScriptsAllowed", are_dangerous_scripts_allowed),
        ("SetAllowDangerousScripts", set_allow_dangerous_scripts),
        ("GetSelectedDisplayChannel", get_selected_display_channel),
        ("SetSelectedDisplayChannel", set_selected_display_channel),
        ("SetSelectedWarGameType", set_selected_war_game_type),
        ("ShowingHelm", showing_helm),
        ("ShowHelm", show_helm),
        ("ShowingCloak", showing_cloak),
        ("ShowCloak", show_cloak),
        ("LoggingCombat", logging_combat),
        ("LoggingChat", logging_chat),
        ("GetSheathState", get_sheath_state),
        ("ToggleSheath", toggle_sheath),
        ("ToggleSelfHighlight", toggle_self_highlight),
        // The quest greeting's text: none (unverified).
        ("GetGreetingText", empty_string),
        // No cinematic playing: no summary (unverified).
        ("GetCurrentCinematicSummary", empty_string),
        // The engine's locale (`GetLocale`): enUS (unverified).
        ("GetOSLocale", os_locale),
        // A comparison of nothing, an inspected player's honor of none
        // (the declarations' shapes; unverified).
        ("GetInspectHonorData", six_zeros),
        ("GetArtifactProgress", three_zeros),
        ("CanChangePlayerDifficulty", two_falses),
        ("EJ_IsSearchFinished", yes),
        ("GetNumTotemSlots", get_num_totem_slots),
        ("GetNumFlyouts", get_num_flyouts),
        ("GetSelectedWarGameType", get_selected_war_game_type),
    ];
    let groups: [(&[&str], NativeFn); 9] = [
        (NOTHING, nothing),
        (NO, no),
        (YES, yes),
        (ZERO, zero),
        (NIL, nil),
        (RETURNS_NOTHING, nothing),
        (TWO_ZEROS, two_zeros),
        (EMPTY_TABLE, empty_table),
        (PASSED_TABLE, passed_table),
    ];
    for (names, f) in groups {
        out.extend(names.iter().map(|name| (*name, f)));
    }
    out
}

/// `GetNumTotemSlots()`: the four totem elements (Blizzard's cooldown
/// viewer loops over them; unverified for classes without totems).
fn get_num_totem_slots(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(4.0)])
}

/// The global twin of `C_Flyout.GetNumFlyouts()`: its answer, else none
/// (unverified).
fn get_num_flyouts(vm: &mut Vm, _c: NativeCall) -> Ret {
    match ask(vm, "C_Flyout.GetNumFlyouts", &[]).first().copied() {
        Some(v @ Value::Num(_)) => Ok(vec![v]),
        _ => Ok(vec![Value::Num(0.0)]),
    }
}

/// `GetOSLocale()`: enUS, the engine's locale (unverified).
fn os_locale(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![vm.str("enUS")])
}

#[cfg(test)]
mod tests {
    #[test]
    fn edit_distance() {
        let d = |a: &str, b: &str| {
            let a: Vec<char> = a.chars().collect();
            let b: Vec<char> = b.chars().collect();
            super::edit_distance(&a, &b)
        };
        assert_eq!(d("kitten", "sitting"), 3);
        assert_eq!(d("", "abc"), 3);
        assert_eq!(d("same", "same"), 0);
    }

    #[test]
    fn no_name_twice() {
        let list = super::behaviours();
        let mut names: Vec<&str> = list.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
        for name in names {
            assert!(
                !crate::protection::PROTECTED.contains(&name),
                "{name} is protected"
            );
        }
    }
}
