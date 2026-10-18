//! The item classes' and subclasses' names (`C_Item.GetItemClassInfo`,
//! `C_Item.GetItemSubClassInfo`), the inventory types' names
//! (`C_Item.GetItemInventorySlotInfo`) and the auction house's subclasses
//! (`C_AuctionHouse.GetAuctionItemSubClasses`): the client's recorded
//! answers when there are (`/stdtest engine answers`), else the classic
//! client's English names.
//!
//! Unverified: the names below are not recorded yet (the item data's
//! `subType`s agree for those the profiles have: "Metal & Stone",
//! "Leather", "Other", "Trade Goods", "Armor"); the auction house's
//! subclasses (every non-obsolete one).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

/// `Enum.ItemClass` → its name.
const CLASSES: &[(i64, &str)] = &[
    (0, "Consumable"),
    (1, "Container"),
    (2, "Weapon"),
    (3, "Gem"),
    (4, "Armor"),
    (5, "Reagent"),
    (6, "Projectile"),
    (7, "Trade Goods"),
    (8, "Item Enhancement"),
    (9, "Recipe"),
    (10, "Money(OBSOLETE)"),
    (11, "Quiver"),
    (12, "Quest"),
    (13, "Key"),
    (14, "Permanent(OBSOLETE)"),
    (15, "Miscellaneous"),
    (16, "Glyph"),
    (17, "Battle Pets"),
    (18, "WoW Token"),
    (19, "Profession"),
];

/// (class, subclass) → its name.
const SUBCLASSES: &[(i64, i64, &str)] = &[
    (0, 0, "Consumable"),
    (0, 1, "Potion"),
    (0, 2, "Elixir"),
    (0, 3, "Flask"),
    (0, 4, "Scroll"),
    (0, 5, "Food & Drink"),
    (0, 6, "Item Enhancement"),
    (0, 7, "Bandage"),
    (0, 8, "Other"),
    (0, 15, "Item Enhancement (Temporary)"),
    (1, 0, "Bag"),
    (1, 1, "Soul Bag"),
    (1, 2, "Herb Bag"),
    (1, 3, "Enchanting Bag"),
    (1, 4, "Engineering Bag"),
    (1, 5, "Gem Bag"),
    (1, 6, "Mining Bag"),
    (1, 7, "Leatherworking Bag"),
    (1, 8, "Inscription Bag"),
    (1, 9, "Tackle Box"),
    (1, 10, "Cooking Bag"),
    (1, 11, "Reagent Bag"),
    (2, 0, "One-Handed Axes"),
    (2, 1, "Two-Handed Axes"),
    (2, 2, "Bows"),
    (2, 3, "Guns"),
    (2, 4, "One-Handed Maces"),
    (2, 5, "Two-Handed Maces"),
    (2, 6, "Polearms"),
    (2, 7, "One-Handed Swords"),
    (2, 8, "Two-Handed Swords"),
    (2, 9, "Warglaives"),
    (2, 10, "Staves"),
    (2, 11, "Bear Claws"),
    (2, 12, "Cat Claws"),
    (2, 13, "Fist Weapons"),
    (2, 14, "Miscellaneous"),
    (2, 15, "Daggers"),
    (2, 16, "Thrown"),
    (2, 17, "Spears"),
    (2, 18, "Crossbows"),
    (2, 19, "Wands"),
    (2, 20, "Fishing Poles"),
    (4, 0, "Miscellaneous"),
    (4, 1, "Cloth"),
    (4, 2, "Leather"),
    (4, 3, "Mail"),
    (4, 4, "Plate"),
    (4, 5, "Cosmetic"),
    (4, 6, "Shields"),
    (4, 7, "Librams"),
    (4, 8, "Idols"),
    (4, 9, "Totems"),
    (4, 10, "Sigils"),
    (4, 11, "Relic"),
    (5, 0, "Reagent"),
    (6, 0, "Wand(OBSOLETE)"),
    (6, 1, "Bolt(OBSOLETE)"),
    (6, 2, "Arrow"),
    (6, 3, "Bullet"),
    (6, 4, "Thrown(OBSOLETE)"),
    (7, 0, "Trade Goods"),
    (7, 1, "Parts"),
    (7, 2, "Explosives"),
    (7, 3, "Devices"),
    (7, 4, "Jewelcrafting"),
    (7, 5, "Cloth"),
    (7, 6, "Leather"),
    (7, 7, "Metal & Stone"),
    (7, 8, "Meat"),
    (7, 9, "Herb"),
    (7, 10, "Elemental"),
    (7, 11, "Other"),
    (7, 12, "Enchanting"),
    (7, 13, "Materials"),
    (7, 14, "Armor Enchantment"),
    (7, 15, "Weapon Enchantment"),
    (9, 0, "Book"),
    (9, 1, "Leatherworking"),
    (9, 2, "Tailoring"),
    (9, 3, "Engineering"),
    (9, 4, "Blacksmithing"),
    (9, 5, "Cooking"),
    (9, 6, "Alchemy"),
    (9, 7, "First Aid"),
    (9, 8, "Enchanting"),
    (9, 9, "Fishing"),
    (9, 10, "Jewelcrafting"),
    (9, 11, "Inscription"),
    (11, 0, "Quiver(OBSOLETE)"),
    (11, 1, "Bolt(OBSOLETE)"),
    (11, 2, "Quiver"),
    (11, 3, "Ammo Pouch"),
    (12, 0, "Quest"),
    (13, 0, "Key"),
    (13, 1, "Lockpick"),
    (15, 0, "Junk"),
    (15, 1, "Reagent"),
    (15, 2, "Companion Pets"),
    (15, 3, "Holiday"),
    (15, 4, "Other"),
    (15, 5, "Mount"),
    (15, 6, "Mount Equipment"),
];

/// `Enum.InventoryType` → its global string's name (`INVTYPE_<name>`).
const SLOTS: &[(i64, &str)] = &[
    (1, "HEAD"),
    (2, "NECK"),
    (3, "SHOULDER"),
    (4, "BODY"),
    (5, "CHEST"),
    (6, "WAIST"),
    (7, "LEGS"),
    (8, "FEET"),
    (9, "WRIST"),
    (10, "HAND"),
    (11, "FINGER"),
    (12, "TRINKET"),
    (13, "WEAPON"),
    (14, "SHIELD"),
    (15, "RANGED"),
    (16, "CLOAK"),
    (17, "2HWEAPON"),
    (18, "BAG"),
    (19, "TABARD"),
    (20, "ROBE"),
    (21, "WEAPONMAINHAND"),
    (22, "WEAPONOFFHAND"),
    (23, "HOLDABLE"),
    (24, "AMMO"),
    (25, "THROWN"),
    (26, "RANGEDRIGHT"),
    (27, "QUIVER"),
    (28, "RELIC"),
];

fn int(vm: &mut Vm, v: Value) -> Option<i64> {
    match vm.hidden(v) {
        Value::Num(n) => Some(n as i64),
        _ => None,
    }
}

fn get_item_class_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(r) = crate::recorded::lookup(vm, "C_Item.GetItemClassInfo", &c) {
        return Ok(r);
    }
    let class = int(vm, c.arg(1));
    Ok(match CLASSES.iter().find(|(k, _)| Some(*k) == class) {
        Some((_, name)) => vec![vm.str(name)],
        None => vec![],
    })
}

/// `C_Item.GetItemSubClassInfo(class, subclass)`: its name, whether it
/// is armor of a type (false).
fn get_item_sub_class_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(r) = crate::recorded::lookup(vm, "C_Item.GetItemSubClassInfo", &c) {
        return Ok(r);
    }
    let (class, sub) = (int(vm, c.arg(1)), int(vm, c.arg(2)));
    Ok(
        match SUBCLASSES
            .iter()
            .find(|(k, s, _)| Some(*k) == class && Some(*s) == sub)
        {
            Some((_, _, name)) => vec![vm.str(name), Value::Bool(false)],
            None => vec![],
        },
    )
}

fn get_item_inventory_slot_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(r) = crate::recorded::lookup(vm, "C_Item.GetItemInventorySlotInfo", &c) {
        return Ok(r);
    }
    let slot = int(vm, c.arg(1));
    let Some((_, name)) = SLOTS.iter().find(|(k, _)| Some(*k) == slot) else {
        return Ok(vec![]);
    };
    Ok(match vm.get_global(&format!("INVTYPE_{name}")) {
        Value::Str(s) => vec![Value::Str(s)],
        _ => vec![],
    })
}

/// `C_AuctionHouse.GetAuctionItemSubClasses(class)`: the class's
/// subclasses but the obsolete ones.
fn get_auction_item_sub_classes(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(r) = crate::recorded::lookup(vm, "C_AuctionHouse.GetAuctionItemSubClasses", &c) {
        return Ok(r);
    }
    let class = int(vm, c.arg(1));
    let out = vm.new_table();
    let subs = SUBCLASSES
        .iter()
        .filter(|(k, _, name)| Some(*k) == class && !name.contains("OBSOLETE"));
    for (i, (_, sub, _)) in subs.enumerate() {
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num((i + 1) as f64), Value::Num(*sub as f64));
    }
    Ok(vec![Value::Table(out)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("C_Item.GetItemClassInfo", get_item_class_info),
        ("C_Item.GetItemSubClassInfo", get_item_sub_class_info),
        (
            "C_Item.GetItemInventorySlotInfo",
            get_item_inventory_slot_info,
        ),
        (
            "C_AuctionHouse.GetAuctionItemSubClasses",
            get_auction_item_sub_classes,
        ),
    ]
}
