//! Logging in as the client does (`/stdtest engine startup`, camelot): the
//! recorded login's events from the addon's `ADDON_LOADED` to three seconds
//! after `PLAYER_ENTERING_WORLD`, in the client's order and at its times,
//! the test's world filling in what describes it (the addon, its bindings,
//! its CVars, its items). Data: `data/camelot-login.txt` (`cargo xtask
//! ingest`).
//!
//! While the client loads (to `LOADING_SCREEN_DISABLED`), the clock moves
//! but no timer or OnUpdate runs; then an overdue timer fires once
//! (recorded). A /reload loads within one frame (`data/camelot-reload.txt`).
//!
//! `ReloadUI()` (`C_UI.Reload`) ends the session as the client did before a /reload
//! (`/stdtest engine scenario auction`, `bank`: the recordings' end): the
//! loading screen, the target cleared, the windows' closes, the soft
//! interaction let go, leaving the world, the addons unloading, logging
//! out, the nameplates removed, the camera's pitch saved
//! (`cameraSavedPitch`), the gamepads' configs.
//!
//! Unverified: the inventory's load, one block per item (the recording's
//! character loaded 51 blocks for 35 items); the soft friend's change with
//! no soft friend (recorded both times with an NPC to interact with).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::{engine, engine_mut, framescript, widgets};

/// The login's events (a fresh login's or a /reload's): time, event,
/// arguments.
fn data(reload: bool) -> Vec<(f64, &'static str, &'static str)> {
    parse(if reload {
        include_str!("../data/camelot-reload.txt")
    } else {
        include_str!("../data/camelot-login.txt")
    })
}

fn parse(text: &'static str) -> Vec<(f64, &'static str, &'static str)> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            let mut parts = l.splitn(3, '\t');
            let time = parts.next()?.parse().ok()?;
            let event = parts.next()?;
            Some((time, event, parts.next().unwrap_or("")))
        })
        .collect()
}

/// An argument as recorded (`string X`, `number 1`, `boolean true`,
/// `nil nil`).
fn value(vm: &mut Vm, text: &str) -> Value {
    match text.split_once(' ') {
        Some(("string", s)) => vm.str(s),
        Some(("number", n)) => n.parse().map_or(Value::Nil, Value::Num),
        Some(("boolean", b)) => Value::Bool(b == "true"),
        _ => Value::Nil,
    }
}

fn fire(vm: &mut Vm, event: &str, args: &[Value]) {
    widgets::fire_event(vm, event, args);
}

/// The inventory's load: for each item worn or carried, the player's
/// equipped spells, the backpack's update and `UNIT_INVENTORY_CHANGED`.
/// A stand-in: the client's blocks update each item's own bag (a worn
/// item's the backpack, a bank item's its tab; the bags' containers
/// updated among them, `BAG_CONTAINER_UPDATE`, `PLAYERBANKSLOTS_CHANGED(1)`)
/// in an order of the server's that is not its items' GUIDs' (recorded:
/// two hearthstones' loading screens, 49 and 95 blocks); blocks in
/// another order match the recordings less than this.
fn inventory(vm: &mut Vm) {
    if let Some(blocks) = engine_mut(vm).world.inventory_load.clone() {
        recorded_inventory(vm, &blocks);
        return;
    }
    let items = {
        let ch = &engine(vm).world.character;
        ch.equipment.len() + ch.bags.values().map(|b| b.items.len()).sum::<usize>()
    };
    let player = vm.str("player");
    for i in 0..items {
        crate::character::equipped_spells_changed(vm);
        // The actions the inventory makes usable (a weapon's strike) told
        // with the first item, alone (recorded: after its equipped spells'
        // changes, before its inventory's; which item, unrecorded).
        if i == 0 {
            crate::actions::usability_changed(vm);
        }
        fire(vm, "BAG_UPDATE", &[Value::Num(0.0)]);
        fire(vm, "BAG_UPDATE", &[Value::Num(-1.0)]);
        fire(vm, "UNIT_INVENTORY_CHANGED", &[player]);
    }
}

/// The inventory's load in the server's recorded blocks
/// (`setInventoryLoad`): each block what it tells before its item (bags'
/// updates, their containers', the bank's slot), the player's equipped
/// spells, its item's bag's update, `UNIT_INVENTORY_CHANGED` (recorded:
/// `hearth`'s 50 blocks, the bags' containers among them).
fn recorded_inventory(vm: &mut Vm, blocks: &[String]) {
    let player = vm.str("player");
    for (i, block) in blocks.iter().enumerate() {
        let mut tokens: Vec<&str> = block.split_whitespace().collect();
        let own = tokens.pop().unwrap_or("-");
        for t in tokens {
            match t {
                "containers" => fire(vm, "BAG_CONTAINER_UPDATE", &[]),
                "bank" => fire(vm, "PLAYERBANKSLOTS_CHANGED", &[Value::Num(1.0)]),
                bag => {
                    if let Ok(bag) = bag.parse::<i64>() {
                        crate::character::bag_updated(vm, bag);
                    }
                }
            }
        }
        crate::character::equipped_spells_changed(vm);
        if i == 0 {
            crate::actions::usability_changed(vm);
        }
        if let Ok(bag) = own.parse::<i64>() {
            crate::character::bag_updated(vm, bag);
        }
        fire(vm, "UNIT_INVENTORY_CHANGED", &[player]);
    }
}

/// `setInventoryLoad(blocks)`: the server's blocks of a loading screen's
/// inventory, in its order (`recorded_inventory`).
pub fn control_set_inventory_load(vm: &mut Vm, c: NativeCall) -> Ret {
    let Value::Table(t) = c.arg(1) else {
        return Err(vm.error("setInventoryLoad: a list of blocks expected"));
    };
    let n = vm.heap.table(t).border();
    let mut blocks = Vec::new();
    for i in 1..=n {
        match vm.heap.table(t).get(Value::Num(i as f64)) {
            Value::Str(s) => blocks.push(String::from_utf8_lossy(vm.str_bytes(s)).into_owned()),
            _ => return Err(vm.error("setInventoryLoad: each block a string")),
        }
    }
    engine_mut(vm).world.inventory_load = Some(blocks);
    Ok(vec![])
}

/// A loading screen (a world transfer: a Hearthstone's), as the client
/// recorded it (`data/camelot-teleport.txt`): the client loading (the
/// clock moving, nothing running) to `LOADING_SCREEN_DISABLED`, the world
/// entered again (`PLAYER_ENTERING_WORLD(false, false)`), the inventory
/// reloaded; `map` the continent's instance (`PLAYER_MAP_CHANGED(-1,
/// map)`), `place` where the player arrives (`moveTo`'s table): no map
/// till the world is entered, the continent's instance as the map
/// changes, the old zone's map till the new area's event, the inventory
/// (its durability) empty till it loads (recorded). Unverified: a
/// transfer between continents, an instance.
pub fn teleport(vm: &mut Vm, map: f64, place: Option<wowlua_vm::TableRef>) {
    let mut now = None;
    let mut loading = true;
    let mut recorded = false;
    engine_mut(vm).world.loading_screen = true;
    engine_mut(vm).world.inventory_loading = true;
    for (time, event, args) in parse(include_str!("../data/camelot-teleport.txt")) {
        if let Some(prev) = now
            && time > prev
        {
            if loading {
                crate::clock::jump(vm, time - prev);
            } else {
                crate::clock::advance_frames(vm, 1, (time - prev) * 1000.0);
            }
        }
        now = Some(time);
        if event == "LOADING_SCREEN_DISABLED" {
            fire(vm, event, &[]);
            loading = false;
            crate::clock::catch_up(vm);
            continue;
        }
        // The place left: its auras gone (recorded right after the
        // loading screen shows).
        // The soft friend dropped too (recorded: its change 0.032 s after;
        // its spells' usability untold while loading).
        if event == "LOADING_SCREEN_ENABLED" {
            fire(vm, event, &[]);
            crate::auras::left_place(vm);
            if engine_mut(vm).units.soft[1].take().is_some() {
                fire(vm, "PLAYER_SOFT_FRIEND_CHANGED", &[]);
            }
            continue;
        }
        // The usability's check: the actions' changes told first (the
        // recording's `ACTION_USABLE_CHANGED`, the test's world's).
        if event == "ACTIONBAR_UPDATE_USABLE" {
            crate::actions::usability_changed(vm);
        }
        match event {
            "$INVENTORY" => {
                engine_mut(vm).world.inventory_loading = false;
                recorded = engine(vm).world.inventory_load.is_some();
                inventory(vm);
                continue;
            }
            // The bags' containers' updates after the stand-in's blocks;
            // the recorded blocks tell them among theirs.
            "BAG_CONTAINER_UPDATE" if recorded => continue,
            "PLAYER_MAP_CHANGED" => {
                if let Some(t) = place {
                    crate::character::enter_instance(vm, t);
                }
            }
            "PLAYER_ENTERING_WORLD" => engine_mut(vm).world.loading_screen = false,
            "ZONE_CHANGED_NEW_AREA" => {
                if let Some(t) = place {
                    crate::character::move_to(vm, t, false);
                }
            }
            // The class's group buffs, then their alerts (the line after).
            "HIDDEN_GROUP_BUFFS_CHANGED" => {
                crate::character::group_buffs_changed(vm);
                continue;
            }
            "GROUP_BUFF_VISUAL_ALERTS_CHANGED" => continue,
            // The guild's: its bank's withdrawal and its Discord lobby
            // told again, in a guild only (recorded once, `hearth`, the
            // player a guild's leader; unverified outside one).
            "GUILDBANK_UPDATE_WITHDRAWMONEY" | "DISCORD_GUILD_LOBBY_UPDATE"
                if !crate::guild::in_guild(vm) =>
            {
                continue;
            }
            _ => {}
        }
        let mut values = Vec::new();
        for arg in args.split("; ").filter(|a| !a.is_empty()) {
            values.push(match arg {
                "$MAP_TO" => Value::Num(map),
                "$EMPTY" => Value::Table(vm.new_table()),
                "$EDIT_MODE_LAYOUTS" => crate::editmode::layout_info(vm),
                other => value(vm, other),
            });
        }
        fire(vm, event, &values);
        // Every bar slot changed: the actions' usability as it is now.
        if event == "ACTIONBAR_SLOT_CHANGED" && args == "number 0" {
            crate::actions::bars_reloaded(vm);
        }
        // The auras sent again (recorded once, `hearth`: the player's after
        // its flags with the inventory's reload, 21.750; a party member's
        // after its area, 22.392; unverified: the new instance IDs the
        // client gave them, not modelled).
        if event == "PLAYER_FLAGS_CHANGED" && args == "string player" {
            crate::auras::fire_full_update(vm, crate::units::PLAYER);
        }
        if event == "UNIT_AREA_CHANGED"
            && let Some(token) = args.strip_prefix("string ")
            && token.starts_with("party")
            && let Some(id) = crate::units::resolve(&engine(vm).units, token)
        {
            crate::auras::fire_full_update(vm, id);
        }
    }
}

/// How long logging in takes (`login`'s clock: the recorded login's span
/// and the frame after its loading screen).
pub fn duration(reload: bool) -> f64 {
    let events = data(reload);
    match (events.first(), events.last()) {
        (Some(first), Some(last)) => last.0 - first.0 + 1.0 / 60.0,
        _ => 0.0,
    }
}

/// Logs in: `addon` loaded (with a `Bindings.xml` or not), a fresh login
/// or a /reload; the clock moves as the client's did.
pub fn login(vm: &mut Vm, addon: &str, has_bindings: bool, reload: bool) {
    let mut now = None;
    // While the client loads, the clock moves but nothing runs (no timer,
    // no OnUpdate); once the loading screen goes, frames run.
    let mut loading = true;
    for (time, event, args) in data(reload) {
        if let Some(prev) = now
            && time > prev
        {
            if loading {
                crate::clock::jump(vm, time - prev);
            } else {
                crate::clock::advance_frames(vm, 1, (time - prev) * 1000.0);
            }
        }
        now = Some(time);
        if event == "LOADING_SCREEN_DISABLED" {
            fire(vm, event, &[]);
            loading = false;
            crate::clock::catch_up(vm);
            crate::clock::advance_frames(vm, 1, 1000.0 / 60.0);
            continue;
        }
        // With Blizzard's UI, its settings registrar fires this itself
        // (`C_SettingsUtil.NotifySettingsLoaded`).
        if event == "SETTINGS_LOADED" && !engine(vm).world.ui.loaded.is_empty() {
            continue;
        }
        match event {
            "$INVENTORY" => {
                inventory(vm);
                continue;
            }
            // The other addons loaded at startup (`test.addons`), each
            // loaded before the addon under test, so told before it.
            "ADDON_LOADED" => {
                for (name, bindings) in crate::bindings::login_addons(vm) {
                    let n = vm.str(&name);
                    fire(vm, event, &[n, Value::Bool(bindings)]);
                }
            }
            "PLAYER_ENTERING_WORLD" => {
                fire(vm, event, &[Value::Bool(!reload), Value::Bool(reload)]);
                // A world profile's nameplates appear in the world.
                crate::units::show_waiting_nameplates(vm);
                continue;
            }
            // Edit Mode then reads the account's settings, not recorded yet:
            // its layouts wait for them.
            "EDIT_MODE_LAYOUTS_UPDATED"
                if crate::recorded::lookup_text(vm, "C_EditMode.GetAccountSettings()")
                    .is_none() =>
            {
                continue;
            }
            _ => {}
        }
        let mut values = Vec::new();
        let mut last_string = String::new();
        for arg in args.split("; ").filter(|a| !a.is_empty()) {
            let v = match arg {
                "$ADDON" => vm.str(addon),
                "$BINDINGS" => Value::Bool(has_bindings),
                "$EDIT_MODE_LAYOUTS" => crate::editmode::layout_info(vm),
                "$CVAR" => {
                    let value = framescript::cvar_get(vm, &last_string).unwrap_or_default();
                    vm.str(&value)
                }
                other => {
                    if let Some(s) = other.strip_prefix("string ") {
                        last_string = s.to_string();
                    }
                    value(vm, other)
                }
            };
            values.push(v);
        }
        fire(vm, event, &values);
    }
}

/// `C_UI.Reload()` (`ReloadUI`, `/reload`): the session ends (the
/// module's doc); nothing runs the next one.
pub fn reload_ui(vm: &mut Vm, _c: NativeCall) -> Ret {
    end_session(vm);
    Ok(vec![])
}

/// The session's end before a /reload (`reload_ui`), once; after it
/// nothing of the session runs (`clock`).
pub fn end_session(vm: &mut Vm) {
    if engine(vm).world.logged_out {
        return;
    }
    fire(vm, "LOADING_SCREEN_ENABLED", &[]);
    if engine(vm).units.target.is_some() {
        engine_mut(vm).units.target = None;
        engine_mut(vm).units.list[crate::units::PLAYER.0].target = None;
        fire(vm, "PLAYER_TARGET_CHANGED", &[]);
    }
    for e in [
        "SECURE_TRANSFER_CANCEL",
        "TAXIMAP_CLOSED",
        "SECURE_TRANSFER_CANCEL",
        "TRADE_SKILL_CLOSE",
        "SOCKET_INFO_CLOSE",
        "ARTIFACT_CLOSE",
        "NAVIGATION_FRAME_DESTROYED",
    ] {
        fire(vm, e, &[]);
    }
    let soft = engine(vm).units.soft;
    if let Some(interact) = soft[2] {
        engine_mut(vm).units.soft[2] = None;
        let guid = engine(vm).units.list[interact.0].guid.clone();
        let g = vm.str(&guid);
        fire(vm, "PLAYER_SOFT_INTERACT_CHANGED", &[g, Value::Nil]);
    }
    if soft[1].is_some() || soft[2].is_some() {
        engine_mut(vm).units.soft[1] = None;
        fire(vm, "PLAYER_SOFT_FRIEND_CHANGED", &[]);
    }
    fire(vm, "INSTANCE_LOCK_STOP", &[]);
    fire(vm, "PLAYER_LEAVING_WORLD", &[]);
    fire(vm, "ADDONS_UNLOADING", &[Value::Bool(false)]);
    fire(vm, "PLAYER_LOGOUT", &[]);
    crate::units::remove_nameplates(vm);
    let pitch = framescript::cvar_get(vm, "cameraSavedPitch").unwrap_or_default();
    let (k, v) = (vm.str("cameraSavedPitch"), vm.str(&pitch));
    fire(vm, "CVAR_UPDATE", &[k, v]);
    fire(vm, "GAME_PAD_CONFIGS_CHANGED", &[]);
    engine_mut(vm).world.logged_out = true;
}
