//! Queries Blizzard's UI asks the client, answered by rules read from the
//! client's own answers (`/stdtest engine answers`, Forever 70425; the
//! fixture `forever-camelot-answers.lua` checks each against the
//! recording).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units;
use crate::{engine, framescript};

fn num(v: Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// `C_CombatLog.DoesObjectMatchFilter(mask, flags)`: the flags meet the
/// mask in each of the object groups (affiliation, reaction, controller,
/// type).
fn combat_log_filter(_vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(mask), Some(flags)) = (num(c.arg(1)), num(c.arg(2))) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let (mask, flags) = (mask as u64, flags as u64);
    let groups = [0x0000_000F, 0x0000_00F0, 0x0000_0300, 0x0000_FC00];
    let matches = groups.iter().all(|g| mask & flags & g != 0);
    Ok(vec![Value::Bool(matches)])
}

/// `C_CVar.GetCVarBitfield(name, index)`: bit `index` of a bitfield CVar,
/// six bits a character above `@` (`closedInfoFrames`), after a header of
/// characters below `@` (recorded `\x02`; every recorded bit fits).
fn cvar_bitfield(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Str(name), Some(index)) = (c.arg(1), num(c.arg(2))) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let value = framescript::cvar_get(vm, &name).unwrap_or_default();
    let i = (index as i64 - 1).max(0) as usize;
    let header = value.bytes().take_while(|b| *b < b'@').count();
    let set = value.as_bytes()[header..]
        .get(i / 6)
        .is_some_and(|ch| (ch.wrapping_sub(b'@') >> (i % 6)) & 1 == 1);
    Ok(vec![Value::Bool(set)])
}

/// `UnitIsConnected(unit)`, `UnitIsHumanPlayer(unit)`: a player unit that
/// exists (an NPC or no unit: false).
fn unit_is_player_online(vm: &mut Vm, c: NativeCall) -> Ret {
    let online = units::unit_arg(vm, &c, 1).is_some_and(|id| engine(vm).units.list[id.0].player);
    Ok(vec![Value::Bool(online)])
}

/// `UnitGetIncomingHeals(unit [, healer])`: none, for a unit that exists
/// (secret, as documented); nil without the unit.
fn incoming_heals(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![match units::unit_arg(vm, &c, 1) {
        Some(_) => Value::Num(0.0),
        None => Value::Nil,
    }])
}

/// `UnitGetTotalAbsorbs(unit)`, `UnitGetTotalHealAbsorbs(unit)`: none, even
/// for a unit that does not exist (recorded secret 0s).
fn no_absorbs(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// An item check of a system camelot does not have (azerite, corruption,
/// cosmetics, curios, decor, relics, conduits): false for an item, nil for
/// none (`C_Item.IsCorruptedItem(0)`).
fn not_that_item(_vm: &mut Vm, c: NativeCall) -> Ret {
    let item = match c.arg(1) {
        Value::Num(n) => n > 0.0,
        Value::Str(_) => true,
        _ => false,
    };
    Ok(vec![if item { Value::Bool(false) } else { Value::Nil }])
}

fn no(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

fn nothing(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![])
}

fn nil(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Nil])
}

/// `C_ToyBox.GetToyFromIndex(i)`: -1, no toys (camelot).
fn no_toy(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(-1.0)])
}

/// `C_Container.GetContainerItemCooldown(bag, slot)`: no cooldown; enabled
/// except in the reagent bag (bag 5; recorded).
fn container_item_cooldown(_vm: &mut Vm, c: NativeCall) -> Ret {
    let reagents = num(c.arg(1)) == Some(5.0);
    Ok(vec![
        Value::Num(0.0),
        Value::Num(0.0),
        Value::Num(if reagents { 0.0 } else { 1.0 }),
    ])
}

/// `IsInventoryItemLocked(slot)`: the equipped item's lock.
fn inventory_item_locked(vm: &mut Vm, c: NativeCall) -> Ret {
    let locked = num(c.arg(1)).is_some_and(|slot| {
        engine(vm)
            .world
            .character
            .equipment
            .get(&(slot as i64))
            .is_some_and(|h| h.locked)
    });
    Ok(vec![Value::Bool(locked)])
}

/// `C_Scenario.GetInfo()`: not in a scenario (recorded: nil name, no
/// stages, no flags, no rewards).
fn no_scenario(_vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = |x: f64| Value::Num(x);
    Ok(vec![
        Value::Nil,
        n(0.0),
        n(0.0),
        n(0.0),
        Value::Bool(false),
        Value::Bool(false),
        Value::Bool(false),
        n(0.0),
        n(0.0),
        n(0.0),
        Value::Nil,
        Value::Nil,
        n(0.0),
    ])
}

fn empty_table(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Table(vm.new_table())])
}

fn zero(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `C_SettingsUtil.NotifySettingsLoaded()`: `SETTINGS_LOADED` (Blizzard's
/// settings registrar calls it once its settings are registered, inside
/// the login's `PLAYER_ENTERING_WORLD`; recorded nested so).
fn notify_settings_loaded(vm: &mut Vm, _c: NativeCall) -> Ret {
    crate::widgets::fire_event(vm, "SETTINGS_LOADED", &[]);
    Ok(vec![])
}

/// `UnitIsVisible(unit)`, `UnitIsInInteractRange(unit)`: the unit exists
/// (recorded for units that do not: false; for a soft-interact target that
/// does: in range). Unverified: units out of sight or range.
fn unit_exists(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(units::unit_arg(vm, &c, 1).is_some())])
}

/// `GetUnitTotalModifiedMaxHealthPercent(unit)`: no maximum health
/// modifier (recorded for the player and absent party members).
fn no_max_health_modifier(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0)])
}

/// `C_Container.ContainerIDToInventoryID(bag)`: the bag's inventory slot,
/// bag + 30 for the bags (1 to 5), bag + 58 for the bank's (6 to 11);
/// nothing for the backpack, the bank and below (recorded). Unverified:
/// past 11.
fn container_inventory_id(_vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(match num(c.arg(1)) {
        Some(bag) if (1.0..=5.0).contains(&bag) => vec![Value::Num(bag + 30.0)],
        Some(bag) if (6.0..=11.0).contains(&bag) => vec![Value::Num(bag + 58.0)],
        _ => vec![],
    })
}

/// A global string, else `fallback`.
fn global_text(vm: &mut Vm, name: &str, fallback: &str) -> String {
    match vm.get_global(name) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => fallback.to_string(),
    }
}

/// A whole number as text, its digits grouped by threes with
/// `LARGE_NUMBER_SEPERATOR` from 1000.
fn grouped(vm: &mut Vm, n: f64) -> String {
    let digits = format!("{}", n.abs().round() as u64);
    let sep = global_text(vm, "LARGE_NUMBER_SEPERATOR", ",");
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push_str(&sep);
        }
        out.push(ch);
    }
    if n < 0.0 {
        out.insert(0, '-');
    }
    out
}

/// A number as Lua writes it (`%.14g`).
fn lua_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n:.14}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// `BreakUpLargeNumbers(n, natural)`: a whole number with its digits
/// grouped; a fraction to one decimal (`999.99` → `1000.0`), or rounded
/// whole with `natural` (recorded). Unverified: fractions from 1000.
fn break_up_large_numbers(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = num(c.arg(1)).unwrap_or(0.0);
    let natural = matches!(c.arg(2), Value::Bool(true));
    let text = if n.fract() == 0.0 || natural {
        grouped(vm, n)
    } else {
        format!("{n:.1}")
    };
    Ok(vec![vm.str(&text)])
}

/// `AbbreviateLargeNumbers(n)`: by its whole digits, from 9 the millions
/// (`123 M`), from 6 the thousands (`1234 K`), else broken up (recorded).
fn abbreviate_large_numbers(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = num(c.arg(1)).unwrap_or(0.0);
    let digits = format!("{}", n.abs().trunc() as u64).len();
    let sign = if n < 0.0 { "-" } else { "" };
    let text = if digits > 8 {
        let cap = global_text(vm, "SECOND_NUMBER_CAP", " M");
        format!("{sign}{}{cap}", (n.abs() / 1e6).trunc())
    } else if digits > 5 {
        let cap = global_text(vm, "FIRST_NUMBER_CAP", " K");
        format!("{sign}{}{cap}", (n.abs() / 1e3).trunc())
    } else if n.fract() == 0.0 {
        grouped(vm, n)
    } else {
        lua_number(n)
    };
    Ok(vec![vm.str(&text)])
}

/// `AbbreviateNumbers(n)`: from 1000 by the breakpoints (`1.2K`, `12K`,
/// `1.2M`, `1.2B`; one decimal below ten of a unit, truncated); smaller
/// numbers and negatives as Lua writes them (recorded).
fn abbreviate_numbers(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = num(c.arg(1)).unwrap_or(0.0);
    let units = [
        (1e12, "FOURTH_NUMBER_CAP_NO_SPACE", "T"),
        (1e9, "THIRD_NUMBER_CAP_NO_SPACE", "B"),
        (1e6, "SECOND_NUMBER_CAP_NO_SPACE", "M"),
        (1e3, "FIRST_NUMBER_CAP_NO_SPACE", "K"),
    ];
    for (unit, cap, fallback) in units {
        if n >= unit {
            let value = if n >= unit * 10.0 {
                (n / unit).trunc()
            } else {
                (n / (unit / 10.0)).trunc() / 10.0
            };
            let cap = global_text(vm, cap, fallback);
            let text = format!("{}{cap}", lua_number(value));
            return Ok(vec![vm.str(&text)]);
        }
    }
    Ok(vec![vm.str(&lua_number(n))])
}

/// `C_TransmogCollection.GetCategoryAppearances(category, location)`: the
/// category's appearances as recorded for the account (`/stdtest engine
/// collections`, asked without a location); none for another category.
/// Unverified: what the location changes.
fn category_appearances(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(category) = num(c.arg(1)) else {
        return Ok(vec![]);
    };
    let call = format!(
        "C_TransmogCollection.GetCategoryAppearances({})",
        category as i64
    );
    Ok(crate::recorded::lookup_text(vm, &call)
        .unwrap_or_else(|| vec![Value::Table(vm.new_table())]))
}

/// `C_TransmogCollection.GetClassFilter()`: the player's class (recorded
/// for a paladin: 2).
fn transmog_class_filter(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(
        engine(vm).units.list[units::PLAYER.0].class_id,
    )])
}

/// `C_TransmogCollection.GetValidAppearanceSourcesForClass(appearance,
/// class, category, location)`: as recorded for the class; for a class not
/// recorded, all the appearance's sources (unverified); else none.
fn class_appearance_sources(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Some(visual), Some(class)) = (num(c.arg(1)), num(c.arg(2))) else {
        return Ok(vec![]);
    };
    let category = match num(c.arg(3)) {
        Some(cat) => format!(", {}", cat as i64),
        None => String::new(),
    };
    let (visual, class) = (visual as i64, class as i64);
    let exact = format!(
        "C_TransmogCollection.GetValidAppearanceSourcesForClass({visual}, {class}{category})"
    );
    let all = format!("C_TransmogCollection.GetAppearanceSources({visual}{category})");
    Ok(crate::recorded::lookup_text(vm, &exact)
        .or_else(|| crate::recorded::lookup_text(vm, &all))
        .unwrap_or_else(|| vec![Value::Table(vm.new_table())]))
}

/// `C_LFGList.GetFilteredSearchResults()`, `GetSearchResults()`: no groups
/// found (no search made; the recording's were other players' listings).
fn no_search_results(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num(0.0), Value::Table(vm.new_table())])
}

/// `C_ColorUtil.WrapTextInColor(text, color)`: `|cff` and the color's
/// red, green and blue as hex (rounded), the text, `|r`; the alpha is not
/// written (recorded).
fn wrap_text_in_color(vm: &mut Vm, c: NativeCall) -> Ret {
    let text = match vm.hidden(c.arg(1)) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        Value::Num(n) => format!("{n}"),
        _ => String::new(),
    };
    let Value::Table(color) = c.arg(2) else {
        return Ok(vec![vm.str(&text)]);
    };
    let mut hex = String::new();
    for k in ["r", "g", "b"] {
        let key = vm.str(k);
        let v = num(vm.heap.table(color).get(key)).unwrap_or(0.0);
        hex.push_str(&format!(
            "{:02x}",
            (v.clamp(0.0, 1.0) * 255.0).round() as u8
        ));
    }
    let out = format!("|cff{hex}{text}|r");
    Ok(vec![vm.str(&out)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        (
            "C_SettingsUtil.NotifySettingsLoaded",
            notify_settings_loaded,
        ),
        ("C_Scenario.GetInfo", no_scenario),
        ("C_ColorUtil.WrapTextInColor", wrap_text_in_color),
        ("C_LFGList.GetFilteredSearchResults", no_search_results),
        // Nothing tracked (unverified).
        ("C_ContentTracking.IsTracking", no),
        ("C_LFGList.GetSearchResults", no_search_results),
        (
            "C_TransmogCollection.GetCategoryAppearances",
            category_appearances,
        ),
        ("C_TransmogCollection.GetClassFilter", transmog_class_filter),
        (
            "C_TransmogCollection.GetValidAppearanceSourcesForClass",
            class_appearance_sources,
        ),
        // No saved instances.
        ("GetNumSavedInstances", zero),
        // No quest pop-ups (unverified: not yet asked).
        ("GetNumAutoQuestPopUps", zero),
        // No task quests (unverified: not yet asked).
        ("GetTasksTable", empty_table),
        ("C_CombatLog.DoesObjectMatchFilter", combat_log_filter),
        ("C_CVar.GetCVarBitfield", cvar_bitfield),
        ("UnitIsConnected", unit_is_player_online),
        ("UnitIsHumanPlayer", unit_is_player_online),
        ("UnitGetIncomingHeals", incoming_heals),
        ("UnitGetTotalAbsorbs", no_absorbs),
        ("UnitGetTotalHealAbsorbs", no_absorbs),
        ("UnitThreatSituation", nil),
        ("UnitIsVisible", unit_exists),
        ("UnitIsInInteractRange", unit_exists),
        // Nothing of these for any unit yet (recorded for the player
        // alone and absent party members).
        ("UnitHasVehicleUI", no),
        ("UnitHasVehiclePlayerFrameUI", no),
        ("UnitInVehicle", no),
        ("UnitIsPossessed", no),
        ("UnitIsGameObject", no),
        ("WorldLootObjectExists", no),
        ("UnitHasLootInteraction", no),
        ("C_IncomingSummon.HasIncomingSummon", no),
        ("UnitHasIncomingResurrection", no),
        ("UnitInOtherParty", no),
        ("UnitInPartyIsAI", no),
        ("UnitIsGroupLeader", no),
        ("UnitIsGroupAssistant", no),
        ("UnitIsPVP", no),
        ("UnitIsPVPFreeForAll", no),
        ("UnitPhaseReason", nil),
        // Standing on the ground (recorded).
        ("IsFalling", no),
        ("IsFlying", no),
        ("IsSwimming", no),
        (
            "GetUnitTotalModifiedMaxHealthPercent",
            no_max_health_modifier,
        ),
        (
            "C_Container.ContainerIDToInventoryID",
            container_inventory_id,
        ),
        ("BreakUpLargeNumbers", break_up_large_numbers),
        ("AbbreviateLargeNumbers", abbreviate_large_numbers),
        ("AbbreviateNumbers", abbreviate_numbers),
        ("C_AzeriteEmpoweredItem.IsAzeriteEmpoweredItemByID", no),
        ("C_ItemSocketInfo.IsArtifactRelicItem", no),
        ("C_Soulbinds.IsItemConduitByItemInfo", no),
        ("C_Item.IsCorruptedItem", not_that_item),
        ("C_Item.IsCosmeticItem", not_that_item),
        ("C_Item.IsCurioItem", not_that_item),
        ("C_Item.IsDecorItem", not_that_item),
        ("C_Item.IsRelicItem", not_that_item),
        ("C_TradeSkillUI.GetItemCraftedQualityInfo", nil),
        ("C_TradeSkillUI.GetItemReagentQualityInfo", nil),
        ("C_LevelLink.IsActionLocked", no),
        ("C_NewItems.IsNewItem", no),
        ("C_Container.GetBagSlotFlag", no),
        (
            "C_Container.GetContainerItemCooldown",
            container_item_cooldown,
        ),
        ("IsInventoryItemLocked", inventory_item_locked),
        ("C_ToyBox.GetToyFromIndex", no_toy),
        ("GetUnitPowerBarInfo", nothing),
    ]
}
