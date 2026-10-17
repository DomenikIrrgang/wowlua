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
/// six bits a character above `@` (`closedInfoFrames`).
fn cvar_bitfield(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Str(name), Some(index)) = (c.arg(1), num(c.arg(2))) else {
        return Ok(vec![Value::Bool(false)]);
    };
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    let value = framescript::cvar_get(vm, &name).unwrap_or_default();
    let i = (index as i64 - 1).max(0) as usize;
    let set = value
        .as_bytes()
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

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_CombatLog.DoesObjectMatchFilter", combat_log_filter),
        ("C_CVar.GetCVarBitfield", cvar_bitfield),
        ("UnitIsConnected", unit_is_player_online),
        ("UnitIsHumanPlayer", unit_is_player_online),
        ("UnitGetIncomingHeals", incoming_heals),
        ("UnitGetTotalAbsorbs", no_absorbs),
        ("UnitGetTotalHealAbsorbs", no_absorbs),
        ("UnitThreatSituation", nil),
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
