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
//! Unverified: the inventory's load, one block per item (the recording's
//! character loaded 51 blocks for 35 items).

use wowlua_vm::{Value, Vm};

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
fn inventory(vm: &mut Vm) {
    let items = {
        let ch = &engine(vm).world.character;
        ch.equipment.len() + ch.bags.values().map(|b| b.items.len()).sum::<usize>()
    };
    let player = vm.str("player");
    for _ in 0..items {
        crate::character::equipped_spells_changed(vm);
        fire(vm, "BAG_UPDATE", &[Value::Num(0.0)]);
        fire(vm, "BAG_UPDATE", &[Value::Num(-1.0)]);
        fire(vm, "UNIT_INVENTORY_CHANGED", &[player]);
    }
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
        if event == "LOADING_SCREEN_ENABLED" {
            fire(vm, event, &[]);
            crate::auras::left_place(vm);
            continue;
        }
        match event {
            "$INVENTORY" => {
                engine_mut(vm).world.inventory_loading = false;
                inventory(vm);
                continue;
            }
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
            "PLAYER_ENTERING_WORLD" => {
                fire(vm, event, &[Value::Bool(!reload), Value::Bool(reload)]);
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
