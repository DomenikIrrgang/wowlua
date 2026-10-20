//! Looting a corpse, as the client recorded the player's with autoloot
//! (`/stdtest engine scenario combat` and `quest`, a ping near 0.3 s):
//! `player:loot(corpse, { auto?, slotsAfter? })` readies it a round trip
//! later (`UNIT_FLAGS`, `LOOT_READY(auto)`), the window 0.25 s after that
//! (`LOOT_OPENED(auto, false)`, `LOOT_READY(auto)`), the slots answer
//! (`GetNumLootItems`, `GetLootSlotInfo`, `GetLootSlotLink`,
//! `GetLootSlotType`, `GetLootSourceInfo`). `LootSlot(i)` (autoloot asks
//! every slot, one after another) is answered by the server in three
//! packets:
//!
//! - the item: `LOOT_SLOT_CLEARED(i)` twice, the item's `ITEM_PUSH(bag,
//!   icon)` and "You receive loot", `BAG_NEW_ITEMS_UPDATED()` for a new
//!   slot (a quest's item: `QUEST_WATCH_UPDATE` before, its count after);
//!   the last slot clears once, the corpse is untargeted and
//!   `LOOT_CLOSED()` fires twice, before the push;
//! - the bags, a moment later (shared by the items pushed meanwhile): for
//!   a stack grown the equipped spells' changes and its bag's update, for
//!   a new slot its bag's update; the corpse emptied: `UNIT_LOOT(guid,
//!   false)`; `BAG_UPDATE_DELAYED()`, `ITEM_COUNT_CHANGED` for each stack
//!   grown;
//! - a new slot's item data, later still (shared too): till then its slot
//!   is taken but reads empty; then the equipped spells', the bag's and
//!   the inventory's updates (`UNIT_INVENTORY_CHANGED`).
//!
//! `CloseLoot()` (the player walking off): `LOOT_CLOSED()` once; the slots
//! asked for still arrive (no `LOOT_SLOT_CLEARED`), the corpse stays
//! targeted.
//!
//! Unverified: the packets' spacing (recorded: the slots 0.39 to 1.07 s
//! after the window opened, `slotsAfter` replays a recording's; the bags
//! 0.06 to 0.25 s after the item, the item data 0.17 to 1.18 s; one value
//! each), money, a slot left on a corpse closed early.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// The first slot's answer after the window opened, beyond a round trip,
/// and the next ones' spacing (autoloot, recorded).
const FIRST_SLOT_AFTER: f64 = 0.22;
const NEXT_SLOT_AFTER: f64 = 0.15;
/// The bags' packet after an item's, and the new items' data's.
const BAGS_AFTER: f64 = 0.08;
const ITEM_DATA_AFTER: f64 = 0.5;

/// An item a corpse holds.
#[derive(Clone, Debug)]
pub struct LootItem {
    pub id: i64,
    pub count: f64,
    pub taken: bool,
}

/// An item placed, its bags' packet waiting.
#[derive(Clone, Debug)]
pub struct Placed {
    pub id: i64,
    pub bag: i64,
    /// Onto a stack (else a new slot).
    pub stacked: bool,
}

/// The loot window.
#[derive(Default)]
pub struct Loot {
    /// The corpse looted, when the window is open.
    pub from: Option<UnitId>,
    /// The corpse the slots come from (also once the window closed).
    pub corpse: Option<UnitId>,
    pub auto: bool,
    pub items: Vec<LootItem>,
    /// The bags' packet's items, and whether it is on its way.
    pub bags: Vec<Placed>,
    pub bags_due: bool,
    /// The corpse emptied: its `UNIT_LOOT(guid, false)` with the bags.
    pub emptied: Option<UnitId>,
    /// New slots whose item data is on its way: bag, slot, item.
    pub arriving: Vec<(i64, usize, i64)>,
    pub arriving_due: bool,
    /// A recording's times of the slots' answers after the window opened
    /// (`slotsAfter`), for the next window.
    pub slots_after: Option<Vec<f64>>,
    /// A recording's delays of the bags' packets (`bagsAfter`) and the new
    /// items' data (`dataAfter`) after the item that set each off, in
    /// order (a corpse's or a gathering's).
    pub bags_after: Vec<f64>,
    pub data_after: Vec<f64>,
    /// A recording's delays of the quest log's word (`logAfter`) after the
    /// wanted item that owes it, in order; without one, with the bags.
    pub log_after: Vec<f64>,
    /// Slots whose item's bag update comes in a packet of its own, these
    /// seconds after the item (`bagsAlone`, a recording's).
    pub bags_alone: Vec<(usize, f64)>,
    /// Slots whose item comes these seconds after the slot cleared, its
    /// bag update and the corpse's emptying with it (`pushAfter`, a
    /// recording's).
    pub push_after: Vec<(usize, f64)>,
    /// The looting flags' word after the last item (`flagsAfter`, a
    /// recording's), else a round trip.
    pub flags_after: Option<f64>,
    /// A corpse the player asked to loot, its window not yet open.
    pub asked: Option<UnitId>,
    /// The looting flags owed to the echo of targeting this corpse, or
    /// told with it.
    pub flags_with_echo: Option<UnitId>,
    pub flags_told: bool,
    /// When the asked corpse's `LOOT_READY` is due (the clock's
    /// milliseconds).
    pub open_due: Option<f64>,
}

/// Whether the echo of targeting `target` waits for the loot's answer
/// due in its moment: `LOOT_READY` first, then the echo and the looting
/// flags (recorded, `quest`: 4 of 4 in one moment, 21.527, 58.652,
/// 91.432, 102.091).
pub fn echo_waits(vm: &Vm, target: Option<UnitId>) -> bool {
    let l = &engine(vm).world.loot;
    target.is_some()
        && l.flags_with_echo == target
        && l.open_due == Some(vm.clock.elapsed_ms.round())
}

/// Looting a corpse the player is targeting anew: the player's flags
/// (`UNIT_FLAGS`) come with the target's echo, not with the loot's
/// `LOOT_READY` (recorded: 9 of 9, `quest`'s eight corpses and `combat`'s
/// one, the echo before, with or after `LOOT_READY`; in its moment after
/// it, 4 of 4). The target change's echo is on its way.
pub fn target_echo_coming(vm: &mut Vm) {
    flags_to_echo(vm);
}

fn flags_to_echo(vm: &mut Vm) {
    let l = &engine(vm).world.loot;
    let Some(corpse) = l.asked else { return };
    if engine(vm).units.echoing == Some(Some(corpse)) {
        engine_mut(vm).world.loot.flags_with_echo = Some(corpse);
    }
}

/// The echo of targeting `target` told: the looting flags owed with it.
pub fn target_echoed(vm: &mut Vm, target: Option<UnitId>) {
    if target.is_some() && engine(vm).world.loot.flags_with_echo == target {
        engine_mut(vm).world.loot.flags_with_echo = None;
        engine_mut(vm).world.loot.flags_told = true;
        units::fire_unit(vm, "UNIT_FLAGS", units::PLAYER, &[]);
    }
}

/// A table's numbers in order (`{ 0.08, 0.12 }`).
fn numbers(vm: &Vm, v: Value) -> Vec<f64> {
    let Value::Table(t) = v else {
        return Vec::new();
    };
    let n = vm.heap.table(t).border();
    (1..=n)
        .filter_map(|i| match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Num(x) => Some(x),
            _ => None,
        })
        .collect()
}

/// A list of seconds in the options (`slotsAfter`), if given.
fn times(vm: &mut Vm, opts: Option<wowlua_vm::TableRef>, key: &str) -> Option<Vec<f64>> {
    let v = opt_field(vm, opts, key);
    matches!(v, Value::Table(_)).then(|| numbers(vm, v))
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn later(vm: &mut Vm, seconds: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, seconds, native);
}

/// `player:loot(corpse, { auto?, opensAfter?, slotsAfter?, bagsAfter?,
/// dataAfter?, logAfter? })`: the player loots a corpse; the window opens a
/// round trip later (`opensAfter` seconds: a recording's), autoloot's
/// slots answer at the usual spacing (`slotsAfter`: a recording's times
/// after the window opened, one per slot); the bags' packets and the new
/// items' data the usual while after the item that sets each off
/// (`bagsAfter`, `dataAfter`: a recording's, in order); a quest's log's
/// word (`UNIT_QUEST_LOG_CHANGED`) with the bags' packet, or `logAfter`
/// seconds after the wanted item (a recording's, in order: `quest`
/// 103.043, with the next item's push); `bagsAlone = { [slot] = seconds }`:
/// that slot's item's bag update in a packet of its own, these seconds
/// after the item (a recording's: `quest` 84.188, a stack's grown count
/// with the new item's data, its packet after the next item's);
/// `pushAfter = { [slot] = seconds }`: that slot clears (the last: the
/// window closes) but its item comes these seconds later, in one packet
/// with its bag update first and the corpse's emptying (a recording's:
/// `combat` 22.478, the last slot's push 0.055 s after `LOOT_CLOSED`,
/// the other item's bags' packet between).
pub fn control_loot(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    if me != units::PLAYER {
        return Err(vm.error("loot: the player's"));
    }
    let Some(corpse) = units::handle_of(vm, c.arg(2)) else {
        return Err(vm.error("loot: a corpse to loot expected"));
    };
    let opts = match c.arg(3) {
        Value::Table(t) => Some(t),
        _ => None,
    };
    let auto = opt_field(vm, opts, "auto").truthy();
    let delay = match opt_field(vm, opts, "opensAfter") {
        Value::Num(n) => n,
        _ => crate::clock::server_delay(vm),
    };
    engine_mut(vm).world.loot.slots_after = times(vm, opts, "slotsAfter");
    let bags = opt_field(vm, opts, "bagsAfter");
    engine_mut(vm).world.loot.bags_after = numbers(vm, bags);
    let data = opt_field(vm, opts, "dataAfter");
    engine_mut(vm).world.loot.data_after = numbers(vm, data);
    let log = opt_field(vm, opts, "logAfter");
    engine_mut(vm).world.loot.log_after = numbers(vm, log);
    let alone = slot_seconds(vm, opts, "bagsAlone");
    engine_mut(vm).world.loot.bags_alone = alone;
    engine_mut(vm).world.loot.push_after = slot_seconds(vm, opts, "pushAfter");
    engine_mut(vm).world.loot.flags_after = opt_seconds(vm, opts, "flagsAfter");
    {
        let l = &mut engine_mut(vm).world.loot;
        l.asked = Some(corpse);
        l.flags_with_echo = None;
        l.flags_told = false;
    }
    engine_mut(vm).world.loot.open_due = Some((vm.clock.elapsed_ms + delay * 1000.0).round());
    flags_to_echo(vm);
    later(
        vm,
        delay,
        "lootOpened",
        opened,
        vec![Value::Num(corpse.0 as f64), Value::Bool(auto)],
    );
    Ok(vec![])
}

/// A number of seconds in the options (`flagsAfter`), if given.
fn opt_seconds(vm: &mut Vm, opts: Option<wowlua_vm::TableRef>, key: &str) -> Option<f64> {
    match opt_field(vm, opts, key) {
        Value::Num(n) => Some(n),
        _ => None,
    }
}

/// A table of seconds by slot in the options (`bagsAlone`, `pushAfter`).
fn slot_seconds(vm: &mut Vm, opts: Option<wowlua_vm::TableRef>, key: &str) -> Vec<(usize, f64)> {
    let mut out = Vec::new();
    if let Value::Table(t) = opt_field(vm, opts, key) {
        let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
        for (k, v) in entries {
            if let (Value::Num(slot), Value::Num(after)) = (k, v) {
                out.push((slot as usize, after));
            }
        }
    }
    out
}

/// A gathered object's window after its loot is ready (recorded).
const GATHERED_OPEN_AFTER: f64 = 0.267;

fn opt_field(vm: &mut Vm, opts: Option<wowlua_vm::TableRef>, key: &str) -> Value {
    match opts {
        Some(t) => {
            let k = vm.str(key);
            vm.heap.table(t).get(k)
        }
        None => Value::Nil,
    }
}

/// The cooldown events around a gathering cast: off the global cooldown
/// (`SPELL_UPDATE_COOLDOWN(spell, nil, nil, nil, nil)`), the bags' at its
/// end (recorded).
pub(crate) fn gather_cooldowns(vm: &mut Vm, spell: f64, bags: bool) {
    fire(vm, "ACTIONBAR_UPDATE_COOLDOWN", &[]);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    if bags {
        fire(vm, "BAG_UPDATE_COOLDOWN", &[]);
    }
    let nils = [
        Value::Num(spell),
        Value::Nil,
        Value::Nil,
        Value::Nil,
        Value::Nil,
    ];
    fire(vm, "SPELL_UPDATE_COOLDOWN", &nils);
    fire(vm, "UPDATE_SHAPESHIFT_COOLDOWN", &[]);
}

/// `player:gather(object, { spell, castTime?, loot = { { id, count? } },
/// auto?, succeedsAfter?, slotsAfter?, bagsAfter? })`: the player gathers a world object (a vein, a herb), as the
/// client recorded the paladin mining copper: the click asks
/// (`CURRENT_SPELL_CAST_CHANGED(false)`, `ACTIONBAR_UPDATE_STATE()`), a round
/// trip later the gathering spell is cast on it (`SENT` names it) with its
/// cooldowns' events; at its success the
/// object's loot ready (`LOOT_READY(auto)`), the player's `UNIT_FLAGS`, the
/// cooldowns' events again (the bags' too); the window a moment later
/// (`LOOT_OPENED`, `LOOT_READY`), its slots as a corpse's (`slotsAfter`:
/// a recording's times after the window opened), each item's bags'
/// packet as recorded (`bagsAfter`: seconds after the item that set each
/// off, in order);
/// the cast's success `succeedsAfter` seconds after its start as recorded
/// (else at its end: the server's answer comes up to 0.06 s either side).
pub fn control_gather(vm: &mut Vm, c: NativeCall) -> Ret {
    let me = units::this(vm, &c)?;
    if me != units::PLAYER {
        return Err(vm.error("gather: the player's"));
    }
    let Some(object) = units::handle_of(vm, c.arg(2)) else {
        return Err(vm.error("gather: an object to gather expected"));
    };
    let opts = match c.arg(3) {
        Value::Table(t) => Some(t),
        _ => None,
    };
    let Value::Num(spell) = opt_field(vm, opts, "spell") else {
        return Err(vm.error("gather: a spell expected"));
    };
    let seconds = match opt_field(vm, opts, "castTime") {
        Value::Num(s) => Some(s),
        _ => None,
    };
    let auto = opt_field(vm, opts, "auto").truthy();
    let mut items = Vec::new();
    if let Value::Table(list) = opt_field(vm, opts, "loot") {
        let n = vm.heap.table(list).border();
        for i in 1..=n {
            if let Value::Table(item) = vm.heap.table(list).get(Value::Num(i as f64)) {
                let id = opt_field(vm, Some(item), "id");
                let count = opt_field(vm, Some(item), "count");
                if let Value::Num(id) = id {
                    let count = if let Value::Num(n) = count { n } else { 1.0 };
                    items.push((id as i64, count));
                }
            }
        }
    }
    engine_mut(vm).units.list[object.0].loot = items;
    engine_mut(vm).world.loot.slots_after = times(vm, opts, "slotsAfter");
    let bags = opt_field(vm, opts, "bagsAfter");
    engine_mut(vm).world.loot.bags_after = numbers(vm, bags);
    engine_mut(vm).world.loot.data_after = Vec::new();
    engine_mut(vm).world.loot.flags_after = opt_seconds(vm, opts, "flagsAfter");
    fire(vm, "CURRENT_SPELL_CAST_CHANGED", &[Value::Bool(false)]);
    fire(vm, "ACTIONBAR_UPDATE_STATE", &[]);
    let delay = crate::clock::server_delay(vm);
    let succeeds_after = opt_field(vm, opts, "succeedsAfter");
    later(
        vm,
        delay,
        "gatherCast",
        gather_cast,
        vec![
            Value::Num(object.0 as f64),
            Value::Bool(auto),
            Value::Num(spell),
            seconds.map_or(Value::Nil, Value::Num),
            succeeds_after,
        ],
    );
    Ok(vec![])
}

/// The server casts the gathering spell.
fn gather_cast(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(object), Value::Num(spell)) = (c.upvals[0], c.upvals[2]) else {
        return Ok(vec![]);
    };
    let seconds = match c.upvals[3] {
        Value::Num(s) => Some(s),
        _ => None,
    };
    if let Some(Value::Num(after)) = c.upvals.get(4) {
        engine_mut(vm).units.casting.succeeds_after = Some(*after);
    }
    let done = vm.native_with("gathered", gathered as NativeFn, c.upvals[..3].to_vec());
    let look = crate::casting::spell_look(vm, spell, seconds);
    let object = UnitId(object as usize);
    crate::casting::cast_at(
        vm,
        units::PLAYER,
        spell,
        look,
        false,
        false,
        Some(object),
        Some(done),
    )?;
    gather_cooldowns(vm, spell, false);
    Ok(vec![])
}

/// The gathering cast succeeded: the object's loot ready, its window soon.
fn gathered(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(object), Value::Bool(auto), Value::Num(spell)) =
        (c.upvals[0], c.upvals[1], c.upvals[2])
    else {
        return Ok(vec![]);
    };
    // Its items countable at once (recorded: `GetNumLootItems` answers).
    fill(vm, UnitId(object as usize), auto);
    fire(vm, "LOOT_READY", &[Value::Bool(auto)]);
    units::fire_unit(vm, "UNIT_FLAGS", units::PLAYER, &[]);
    gather_cooldowns(vm, spell, true);
    later(
        vm,
        GATHERED_OPEN_AFTER,
        "gatheredOpened",
        gathered_opened,
        c.upvals[..2].to_vec(),
    );
    Ok(vec![])
}

fn gathered_opened(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(id), Value::Bool(auto)) = (c.upvals[0], c.upvals[1]) else {
        return Ok(vec![]);
    };
    open_window(vm, UnitId(id as usize), auto, false);
    Ok(vec![])
}

/// The server opens the loot: the corpse's items in the window.
fn opened(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(id), Value::Bool(auto)) = (c.upvals[0], c.upvals[1]) else {
        return Ok(vec![]);
    };
    open_window(vm, UnitId(id as usize), auto, true);
    Ok(vec![])
}

/// The loot's items: the corpse's or object's.
fn fill(vm: &mut Vm, corpse: UnitId, auto: bool) {
    let items: Vec<LootItem> = engine(vm).units.list[corpse.0]
        .loot
        .iter()
        .map(|(id, count)| LootItem {
            id: *id,
            count: *count,
            taken: false,
        })
        .collect();
    let l = &mut engine_mut(vm).world.loot;
    l.from = Some(corpse);
    l.corpse = Some(corpse);
    l.auto = auto;
    l.items = items;
}

/// A corpse's window after its loot is ready (recorded: 0.244 to 0.254 s
/// at each of the quest and combat scenarios' nine).
const CORPSE_OPEN_AFTER: f64 = 0.25;

/// The loot window open on a corpse's or an object's items (a corpse's
/// told by `UNIT_FLAGS` and `LOOT_READY` first, the window a moment
/// later; an object's were).
fn open_window(vm: &mut Vm, corpse: UnitId, auto: bool, corpse_flags: bool) {
    fill(vm, corpse, auto);
    // Looting: the player's flags change (recorded just before), unless
    // they come with the target's echo (`target_echo_coming`).
    if corpse_flags {
        let with_echo = {
            let l = &mut engine_mut(vm).world.loot;
            l.asked = None;
            l.open_due = None;
            l.flags_with_echo.is_some() || std::mem::take(&mut l.flags_told)
        };
        if !with_echo {
            units::fire_unit(vm, "UNIT_FLAGS", units::PLAYER, &[]);
        }
        fire(vm, "LOOT_READY", &[Value::Bool(auto)]);
        later(
            vm,
            CORPSE_OPEN_AFTER,
            "lootWindow",
            window_opened,
            vec![Value::Bool(auto)],
        );
    } else {
        show_window(vm, auto);
    }
}

fn window_opened(vm: &mut Vm, c: NativeCall) -> Ret {
    show_window(vm, c.upvals[0].truthy());
    Ok(vec![])
}

/// The window shown: `LOOT_OPENED`, `LOOT_READY`; autoloot asks for every
/// slot.
fn show_window(vm: &mut Vm, auto: bool) {
    fire(vm, "LOOT_OPENED", &[Value::Bool(auto), Value::Bool(false)]);
    // Told again only when autoloot takes items (SuuUI probe loot: three
    // windows looted by hand, `LOOT_READY(false)` then `LOOT_OPENED` alone;
    // an empty window autolooted, one sample, the same).
    if auto && !engine(vm).world.loot.items.is_empty() {
        fire(vm, "LOOT_READY", &[Value::Bool(auto)]);
    }
    let recorded = engine_mut(vm).world.loot.slots_after.take();
    if auto {
        let n = engine(vm).world.loot.items.len();
        let delay = crate::clock::server_delay(vm);
        for i in 1..=n {
            let after = match recorded.as_ref().and_then(|r| r.get(i - 1)) {
                Some(&t) => t,
                None => delay + FIRST_SLOT_AFTER + NEXT_SLOT_AFTER * (i - 1) as f64,
            };
            later(vm, after, "lootTaken", taken, vec![Value::Num(i as f64)]);
        }
    }
}

/// A slot asked for: the server answers a round trip later.
fn request(vm: &mut Vm, slot: usize) {
    let delay = crate::clock::server_delay(vm);
    later(vm, delay, "lootTaken", taken, vec![Value::Num(slot as f64)]);
}

/// The item's arrival told: `ITEM_PUSH(bag, icon)` and "You receive loot",
/// a new slot's `BAG_NEW_ITEMS_UPDATED()` (a quest's objective's:
/// `QUEST_WATCH_UPDATE` before, its count after).
fn push(vm: &mut Vm, id: i64, count: f64, bag: i64, new_slot: bool) {
    crate::quests::before_item(vm, id);
    let icon = crate::character::item_field(vm, id, "icon");
    fire(
        vm,
        "ITEM_PUSH",
        &[Value::Num(crate::character::bag_inventory_slot(bag)), icon],
    );
    let link = crate::character::link_of_id(vm, id)
        .map(|v| match v {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            _ => String::new(),
        })
        .unwrap_or_default();
    // The client's own format (camelot's without a period).
    let format = |vm: &mut Vm, name: &str, default: &str| match vm.get_global(name) {
        Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
        _ => default.to_string(),
    };
    let text = if count > 1.0 {
        format(vm, "LOOT_ITEM_SELF_MULTIPLE", "You receive loot: %sx%d")
            .replacen("%s", &link, 1)
            .replacen("%d", &format!("{count}"), 1)
    } else {
        format(vm, "LOOT_ITEM_SELF", "You receive loot: %s").replacen("%s", &link, 1)
    };
    let sender = player_sender(vm);
    crate::chat::game_message(vm, "CHAT_MSG_LOOT", &text, Some(&sender));
    if new_slot {
        fire(vm, "BAG_NEW_ITEMS_UPDATED", &[]);
    }
    crate::quests::after_item(vm, id);
}

/// The player as a loot message's sender: the full name.
pub fn player_sender(vm: &Vm) -> String {
    let p = &engine(vm).units.list[units::PLAYER.0];
    match &p.last_name {
        Some(last) => format!("{} {last}", p.name),
        None => p.name.clone(),
    }
}

/// The server took a slot: it clears (the window open); the item goes to
/// the bags.
fn taken(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Num(slot) = c.upvals[0] else {
        return Ok(vec![]);
    };
    let slot = slot as usize;
    let Some(item) = engine(vm).world.loot.items.get(slot - 1).cloned() else {
        return Ok(vec![]);
    };
    if item.taken {
        return Ok(vec![]);
    }
    engine_mut(vm).world.loot.items[slot - 1].taken = true;
    let last = engine(vm).world.loot.items.iter().all(|i| i.taken);
    let open = engine(vm).world.loot.from.is_some();
    let corpse = engine(vm).world.loot.corpse;
    if open && last {
        // The last: the corpse emptied, the window closed.
        fire(vm, "LOOT_SLOT_CLEARED", &[Value::Num(slot as f64)]);
        engine_mut(vm).world.loot.from = None;
        if corpse.is_some() && engine(vm).units.target == corpse {
            units::clear_target(vm);
        }
        fire(vm, "LOOT_CLOSED", &[]);
        fire(vm, "LOOT_CLOSED", &[]);
    } else if open {
        fire(vm, "LOOT_SLOT_CLEARED", &[Value::Num(slot as f64)]);
        fire(vm, "LOOT_SLOT_CLEARED", &[Value::Num(slot as f64)]);
    }
    let late = {
        let l = &mut engine_mut(vm).world.loot;
        l.push_after
            .iter()
            .position(|(s, _)| *s == slot)
            .map(|i| l.push_after.remove(i).1)
    };
    if let Some(after) = late {
        let emptied = match corpse {
            Some(corpse) if last => {
                engine_mut(vm).units.list[corpse.0].loot.clear();
                let delay = flags_delay(vm);
                later(vm, delay, "lootFlags", loot_flags, vec![]);
                Value::Num(corpse.0 as f64)
            }
            _ => Value::Nil,
        };
        let upvals = vec![Value::Num(item.id as f64), Value::Num(item.count), emptied];
        later(vm, after, "lootPushedLate", pushed_late, upvals);
        return Ok(vec![]);
    }
    let placed = crate::character::push_to_bags(vm, item.id, item.count);
    let (bag, new_slot) = placed.unwrap_or((0, None));
    push(vm, item.id, item.count, bag, new_slot.is_some());
    if engine(vm).world.quests.log_owed && !engine(vm).world.loot.log_after.is_empty() {
        let after = engine_mut(vm).world.loot.log_after.remove(0);
        later(vm, after, "lootLog", log_told, vec![]);
    }
    let mut flags = None;
    if last && let Some(corpse) = corpse {
        engine_mut(vm).units.list[corpse.0].loot.clear();
        engine_mut(vm).world.loot.emptied = Some(corpse);
        // No longer looting: the flags a round trip after the last item
        // (recorded: the profession's veins 0.30-0.34 s at a ping of 0.345,
        // the quest's corpses 0.26-0.28 s at 0.3; before or after the
        // bags' packet as that comes, in its moment after its bags'
        // updates: `profession` 8.473, `quest` 32.346).
        flags = Some(flags_delay(vm));
    }
    let alone = {
        let l = &mut engine_mut(vm).world.loot;
        l.bags_alone
            .iter()
            .position(|(s, _)| *s == slot)
            .map(|i| l.bags_alone.remove(i).1)
    };
    if let Some(after) = alone {
        let upvals = vec![
            Value::Num(item.id as f64),
            Value::Num(bag as f64),
            Value::Bool(new_slot.is_none()),
        ];
        later(vm, after, "lootBagsAlone", bags_told_alone, upvals);
    } else {
        let l = &mut engine_mut(vm).world.loot;
        l.bags.push(Placed {
            id: item.id,
            bag,
            stacked: new_slot.is_none(),
        });
    }
    if alone.is_none() && !engine(vm).world.loot.bags_due {
        engine_mut(vm).world.loot.bags_due = true;
        let l = &mut engine_mut(vm).world.loot;
        let after = if l.bags_after.is_empty() {
            BAGS_AFTER
        } else {
            l.bags_after.remove(0)
        };
        later(vm, after, "lootBags", bags_told, vec![]);
    }
    if let Some(slot) = new_slot {
        engine_mut(vm)
            .world
            .loot
            .arriving
            .push((bag, slot, item.id));
        if !engine(vm).world.loot.arriving_due {
            engine_mut(vm).world.loot.arriving_due = true;
            let l = &mut engine_mut(vm).world.loot;
            let after = if l.data_after.is_empty() {
                ITEM_DATA_AFTER
            } else {
                l.data_after.remove(0)
            };
            later(vm, after, "lootItemData", item_data_arrived, vec![]);
        }
    }
    if let Some(delay) = flags {
        later(vm, delay, "lootFlags", loot_flags, vec![]);
    }
    Ok(vec![])
}

/// The looting flags' delay after the last item: a recording's
/// (`flagsAfter`) or a round trip.
fn flags_delay(vm: &mut Vm) -> f64 {
    let recorded = engine_mut(vm).world.loot.flags_after.take();
    recorded.unwrap_or_else(|| crate::clock::server_delay(vm))
}

/// A slot's item after its slot cleared (`pushAfter`): its bag update,
/// the push, the corpse emptied (if it was the last), the bags' end
/// (recorded, `combat` 22.478: `BAG_UPDATE`, `ITEM_PUSH`, the loot's
/// message, `BAG_NEW_ITEMS_UPDATED`, `UNIT_LOOT`, `BAG_UPDATE_DELAYED`).
fn pushed_late(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(id), Value::Num(count)) = (c.upvals[0], c.upvals[1]) else {
        return Ok(vec![]);
    };
    let id = id as i64;
    let placed = crate::character::push_to_bags(vm, id, count);
    let (bag, new_slot) = placed.unwrap_or((0, None));
    let p = [Placed {
        id,
        bag,
        stacked: new_slot.is_none(),
    }];
    crate::quests::tell_log(vm);
    bag_updates(vm, &p);
    push(vm, id, count, bag, new_slot.is_some());
    if let Value::Num(corpse) = c.upvals[2] {
        tell_emptied(vm, UnitId(corpse as usize));
    }
    bags_end(vm, &p);
    if let Some(slot) = new_slot {
        engine_mut(vm).world.loot.arriving.push((bag, slot, id));
        if !engine(vm).world.loot.arriving_due {
            engine_mut(vm).world.loot.arriving_due = true;
            let l = &mut engine_mut(vm).world.loot;
            let after = if l.data_after.is_empty() {
                ITEM_DATA_AFTER
            } else {
                l.data_after.remove(0)
            };
            later(vm, after, "lootItemData", item_data_arrived, vec![]);
        }
    }
    Ok(vec![])
}

/// The quest log's word, apart from the bags' packet (`logAfter`).
fn log_told(vm: &mut Vm, _c: NativeCall) -> Ret {
    crate::quests::tell_log(vm);
    Ok(vec![])
}

/// The bags' packet: each item's bag updated (a stack grown: the equipped
/// spells' changes first, its count after), the corpse emptied; a quest's
/// log changed by an item first (`quests::tell_log`).
fn bags_told(vm: &mut Vm, _c: NativeCall) -> Ret {
    let placed = {
        let l = &mut engine_mut(vm).world.loot;
        l.bags_due = false;
        std::mem::take(&mut l.bags)
    };
    tell_bags(vm, &placed, true);
    Ok(vec![])
}

/// An item's bag update in a packet of its own (`bagsAlone`), without the
/// corpse's emptying (recorded: `quest` 59.260 and 84.188, the corpse
/// emptied with the other items' packet).
fn bags_told_alone(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(id), Value::Num(bag)) = (c.upvals[0], c.upvals[1]) else {
        return Ok(vec![]);
    };
    let placed = Placed {
        id: id as i64,
        bag: bag as i64,
        stacked: c.upvals[2].truthy(),
    };
    tell_bags(vm, &[placed], false);
    Ok(vec![])
}

/// A bags' packet's events for the items `placed` (`emptied`: the corpse's
/// emptying with it, if owed).
fn tell_bags(vm: &mut Vm, placed: &[Placed], emptied: bool) {
    crate::quests::tell_log(vm);
    bag_updates(vm, placed);
    if emptied && let Some(corpse) = engine_mut(vm).world.loot.emptied.take() {
        tell_emptied(vm, corpse);
    }
    bags_end(vm, placed);
}

/// Each item's bag updated: the stacks grown first (the equipped spells'
/// changes before), then the new slots.
fn bag_updates(vm: &mut Vm, placed: &[Placed]) {
    for p in placed.iter().filter(|p| p.stacked) {
        crate::character::equipped_spells_changed(vm);
        crate::character::bag_updated(vm, p.bag);
        // A stack grown in the reagent bag: the inventory's update too
        // (recorded: mined ore; the backpack's stacks without).
        if p.bag == crate::character::REAGENT_BAG {
            units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", units::PLAYER, &[]);
        }
    }
    for p in placed.iter().filter(|p| !p.stacked) {
        crate::character::bag_updated(vm, p.bag);
    }
}

/// A corpse emptied: a unit's is told; a world object is no unit.
fn tell_emptied(vm: &mut Vm, corpse: UnitId) {
    let g = engine(vm).units.list[corpse.0].guid.clone();
    if !g.starts_with("GameObject-") {
        let g = vm.str(&g);
        fire(vm, "UNIT_LOOT", &[g, Value::Bool(false)]);
    }
}

/// A bags' packet's end: `BAG_UPDATE_DELAYED`, each grown stack's count.
fn bags_end(vm: &mut Vm, placed: &[Placed]) {
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    let mut wanted = false;
    for p in placed.iter().filter(|p| p.stacked) {
        fire(vm, "ITEM_COUNT_CHANGED", &[Value::Num(p.id as f64)]);
        wanted |= crate::quests::is_quest_item(vm, p.id);
    }
    // A logged quest's item counted anew: the log's update after the
    // counts (recorded, `quest`: all 7 counts of Tough Wolf Meat, none of
    // the 3 without it).
    if wanted {
        crate::batch::fire_at_end(vm, "QUEST_LOG_UPDATE", &[]);
    }
}

/// The new slots' item data: each slot reads its item; the equipped
/// spells', the bag's and the inventory's updates (a quest's item: the
/// log's update after).
fn item_data_arrived(vm: &mut Vm, _c: NativeCall) -> Ret {
    let arriving = {
        let l = &mut engine_mut(vm).world.loot;
        l.arriving_due = false;
        std::mem::take(&mut l.arriving)
    };
    let mut quest = false;
    for (bag, slot, id) in arriving {
        crate::character::arrived(vm, bag, slot);
        crate::character::equipped_spells_changed(vm);
        crate::character::bag_updated(vm, bag);
        units::fire_unit(vm, "UNIT_INVENTORY_CHANGED", units::PLAYER, &[]);
        quest |= crate::quests::is_quest_item(vm, id);
    }
    fire(vm, "BAG_UPDATE_DELAYED", &[]);
    if quest {
        crate::batch::fire_at_end(vm, "QUEST_LOG_UPDATE", &[]);
    }
    Ok(vec![])
}

fn loot_flags(vm: &mut Vm, _c: NativeCall) -> Ret {
    units::fire_unit(vm, "UNIT_FLAGS", units::PLAYER, &[]);
    Ok(vec![])
}

fn open_item(vm: &Vm, c: &NativeCall) -> Option<LootItem> {
    let l = &engine(vm).world.loot;
    l.from?;
    let Value::Num(i) = c.arg(1) else { return None };
    l.items
        .get((i as usize).checked_sub(1)?)
        .filter(|it| !it.taken)
        .cloned()
}

fn get_num_loot_items(vm: &mut Vm, _c: NativeCall) -> Ret {
    let l = &engine(vm).world.loot;
    let n = if l.from.is_some() { l.items.len() } else { 0 };
    Ok(vec![Value::Num(n as f64)])
}

/// `GetLootSlotInfo(slot)`: icon, name, quantity, currency, quality,
/// locked, quest item (an item a quest in the log asks for: recorded),
/// quest, active quest, coin (recorded for items).
fn get_loot_slot_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(item) = open_item(vm, &c) else {
        return Ok(vec![]);
    };
    let icon = crate::character::item_field(vm, item.id, "icon");
    let name = crate::character::item_field(vm, item.id, "name");
    let quality = crate::character::item_field(vm, item.id, "quality");
    Ok(vec![
        icon,
        name,
        Value::Num(item.count),
        Value::Nil,
        if quality.is_nil() {
            Value::Num(1.0)
        } else {
            quality
        },
        Value::Bool(false),
        Value::Bool(crate::quests::is_quest_item(vm, item.id)),
        Value::Nil,
        Value::Nil,
        Value::Bool(false),
    ])
}

fn get_loot_slot_link(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(item) = open_item(vm, &c) else {
        // No such slot: nil (the full probe, no loot open).
        return Ok(vec![Value::Nil]);
    };
    Ok(vec![
        crate::character::link_of_id(vm, item.id).unwrap_or(Value::Nil),
    ])
}

/// `GetLootSlotType(slot)`: 1 for an item (0 none).
fn get_loot_slot_type(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Num(if open_item(vm, &c).is_some() {
        1.0
    } else {
        0.0
    })])
}

/// `GetLootSourceInfo(slot)`: the corpse's GUID and the count.
fn get_loot_source_info(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(item) = open_item(vm, &c) else {
        return Ok(vec![]);
    };
    let Some(corpse) = engine(vm).world.loot.from else {
        return Ok(vec![]);
    };
    let g = engine(vm).units.list[corpse.0].guid.clone();
    Ok(vec![vm.str(&g), Value::Num(item.count)])
}

fn is_fishing_loot(_vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(false)])
}

/// `LootSlot(slot)`: the slot asked for.
fn loot_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    if let (Some(_), Value::Num(i)) = (open_item(vm, &c), c.arg(1)) {
        request(vm, i as usize);
    }
    Ok(vec![])
}

fn loot_slot_has_item(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(open_item(vm, &c).is_some())])
}

/// `CloseLoot()`: the window closes (`LOOT_CLOSED` once, recorded: the
/// player walking off); the slots asked for still arrive.
fn close_loot(vm: &mut Vm, _c: NativeCall) -> Ret {
    if engine_mut(vm).world.loot.from.take().is_some() {
        fire(vm, "LOOT_CLOSED", &[]);
    }
    Ok(vec![])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetNumLootItems", get_num_loot_items),
        ("GetLootSlotInfo", get_loot_slot_info),
        ("GetLootSlotLink", get_loot_slot_link),
        ("GetLootSlotType", get_loot_slot_type),
        ("GetLootSourceInfo", get_loot_source_info),
        ("IsFishingLoot", is_fishing_loot),
        ("LootSlot", loot_slot),
        ("LootSlotHasItem", loot_slot_has_item),
        ("CloseLoot", close_loot),
    ]
}
