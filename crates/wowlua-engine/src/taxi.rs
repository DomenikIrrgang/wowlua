//! A flight master's taxi map, as the client recorded a Horde one's
//! (`/stdtest engine scenario flight`): `defineTaxi(npc, { mapID, nodes,
//! mapNodes })`, opened through its gossip (`opens = "taxi"`):
//! `TAXIMAP_OPENED(1)` (the taxi map system: Blizzard's classic taxi
//! frame) and the interaction's `PLAYER_INTERACTION_MANAGER_FRAME_SHOW(6)`;
//! the nodes' answers (`NumTaxiNodes`, `TaxiNodeName`, `TaxiNodeGetType`,
//! `TaxiNodeCost`, `TaxiNodePosition`, `GetNumRoutes`,
//! `C_TaxiMap.GetAllTaxiNodes`, `C_TaxiMap.GetTaxiNodesForMap`, positions
//! as `CreateVector2D`'s); `CloseTaxiMap()`: `TAXIMAP_CLOSED()` and the
//! interaction's hides.
//!
//! `TakeTaxiNode(slot)`, as a Horde paladin's flight from Orgrimmar to the
//! Crossroads went (`/stdtest engine scenario taxi`): nothing at the click;
//! the server's answer a round trip later closes the map (as
//! `CloseTaxiMap`'s, `TAXIMAP_CLOSED` again after), takes the player's
//! control (`PLAYER_CONTROL_LOST`, the vehicle's `VEHICLE_ANGLE_SHOW` and
//! `VEHICLE_POWER_SHOW`), every spell but the professions' unusable; its
//! next packet (77 ms) puts the player on the taxi (`UNIT_FLAGS`), the
//! tracking spells usable again, mounted (`PLAYER_MOUNT_DISPLAY_CHANGED`),
//! the fare paid (`PLAYER_MONEY`). The node's `flightTime` later it lands:
//! `PLAYER_CONTROL_GAINED`, `VEHICLE_UPDATE`, the taxi and the mount gone,
//! the spells usable. The places on the way are the test's (`moveTo`).
//!
//! Unverified: the routes' hops, a node's state other than current,
//! reachable and unreachable; the item and durability updates the server
//! sent during the flight (a spell's cooldown, the worn items'
//! durability: their cause unknown).

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// `Enum.PlayerInteractionType.TaxiNode`.
const TAXI: f64 = 6.0;
/// `Enum.UIMapSystem.Taxi`.
const TAXI_SYSTEM: f64 = 1.0;
/// The server's packet putting the player on the taxi, after the one taking
/// the control (recorded).
const BOARDED_AFTER: f64 = 0.077;
/// The shapeshift bar's usability told again after boarding (recorded; its
/// cause unknown).
const SHAPESHIFT_AFTER: f64 = 1.096;

#[derive(Default)]
pub struct Taxis {
    /// The flight masters' maps: unit → the test's table.
    pub defined: std::collections::BTreeMap<usize, TableRef>,
    pub open: Option<UnitId>,
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn field(vm: &mut Vm, t: TableRef, key: &str) -> Value {
    let k = vm.str(key);
    vm.heap.table(t).get(k)
}

fn set(vm: &mut Vm, t: TableRef, key: &str, v: Value) {
    let k = vm.str(key);
    let _ = vm.heap.table_mut(t).set(k, v);
}

/// `defineTaxi(npc, { mapID, nodes = { { name, nodeID, x, y, state, cost,
/// routes, hops, flightTime } }, mapNodes = { { name, nodeID, x, y,
/// atlasName, faction, isUndiscovered } } })`: `nodes` in the slots' order;
/// `hops` a node's path, its slots from the current one (else straight
/// there, unverified).
pub fn control_define(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(npc) = units::handle_of(vm, c.arg(1)) else {
        return Err(vm.error("defineTaxi: an NPC expected"));
    };
    let Value::Table(t) = c.arg(2) else {
        return Err(vm.error("defineTaxi: a table of the map expected"));
    };
    widgets::keep(vm, Value::Table(t));
    engine_mut(vm).world.taxis.defined.insert(npc.0, t);
    Ok(vec![])
}

/// The taxi map opens (the gossip's option chosen).
pub fn open(vm: &mut Vm, npc: UnitId) {
    if !engine(vm).world.taxis.defined.contains_key(&npc.0) {
        return;
    }
    engine_mut(vm).world.taxis.open = Some(npc);
    engine_mut(vm).units.npc = Some(npc);
    fire(vm, "TAXIMAP_OPENED", &[Value::Num(TAXI_SYSTEM)]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_SHOW",
        &[Value::Num(TAXI)],
    );
}

fn open_data(vm: &Vm) -> Option<TableRef> {
    let t = &engine(vm).world.taxis;
    t.open.and_then(|n| t.defined.get(&n.0).copied())
}

fn list(vm: &mut Vm, key: &str) -> Vec<TableRef> {
    let Some(t) = open_data(vm) else {
        return vec![];
    };
    let Value::Table(l) = field(vm, t, key) else {
        return vec![];
    };
    let n = vm.heap.table(l).border();
    (1..=n)
        .filter_map(|i| match vm.heap.table(l).get(Value::Num(i as f64)) {
            Value::Table(e) => Some(e),
            _ => None,
        })
        .collect()
}

fn node(vm: &mut Vm, c: &NativeCall) -> Option<TableRef> {
    let Value::Num(i) = vm.hidden(c.arg(1)) else {
        return None;
    };
    list(vm, "nodes").get((i as usize).checked_sub(1)?).copied()
}

fn get_taxi_map_id(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(match open_data(vm) {
        Some(t) => vec![field(vm, t, "mapID")],
        None => vec![],
    })
}

fn num_taxi_nodes(vm: &mut Vm, _c: NativeCall) -> Ret {
    let n = list(vm, "nodes").len();
    Ok(vec![Value::Num(n as f64)])
}

fn node_field(vm: &mut Vm, c: &NativeCall, key: &str) -> Ret {
    Ok(match node(vm, c) {
        Some(t) => vec![field(vm, t, key)],
        None => vec![],
    })
}

/// No such node (the map closed): "INVALID" (the full probe).
fn taxi_node_name(vm: &mut Vm, c: NativeCall) -> Ret {
    if node(vm, &c).is_none() {
        return Ok(vec![vm.str("INVALID")]);
    }
    node_field(vm, &c, "name")
}

fn taxi_node_cost(vm: &mut Vm, c: NativeCall) -> Ret {
    node_field(vm, &c, "cost")
}

/// No such node: none (the full probe).
fn get_num_routes(vm: &mut Vm, c: NativeCall) -> Ret {
    if node(vm, &c).is_none() {
        return Ok(vec![Value::Num(0.0)]);
    }
    node_field(vm, &c, "routes")
}

/// `TaxiNodeGetType(i)`: by its state (`Enum.FlightPathState`: current,
/// reachable, unreachable; recorded "CURRENT", "REACHABLE", "DISTANT").
fn taxi_node_get_type(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = node(vm, &c) else {
        return Ok(vec![vm.str("NONE")]);
    };
    let kind = match field(vm, t, "state") {
        Value::Num(0.0) => "CURRENT",
        Value::Num(1.0) => "REACHABLE",
        _ => "DISTANT",
    };
    Ok(vec![vm.str(kind)])
}

/// A node's path: its slots from the current node's (`hops`; else the
/// current node's and its own).
fn hops(vm: &mut Vm, c: &NativeCall) -> Vec<usize> {
    let Some(t) = node(vm, c) else {
        return vec![];
    };
    if let Value::Table(h) = field(vm, t, "hops") {
        let n = vm.heap.table(h).border();
        return (1..=n)
            .filter_map(|i| match vm.heap.table(h).get(Value::Num(i as f64)) {
                Value::Num(s) => Some(s as usize),
                _ => None,
            })
            .collect();
    }
    let all = list(vm, "nodes");
    let current = all
        .iter()
        .position(|n| matches!(field(vm, *n, "state"), Value::Num(0.0)));
    let Value::Num(me) = vm.hidden(c.arg(1)) else {
        return vec![];
    };
    match current {
        Some(cur) => vec![cur + 1, me as usize],
        None => vec![],
    }
}

/// `TaxiIsDirectFlight(slot)`: a node one hop away (its path from the
/// current node two slots long: `GetNumRoutes` 1). Unverified (no
/// recording): Blizzard's classic taxi frame draws a line to each node it
/// calls direct and closes the map when none is (`DrawOneHopLines`).
fn taxi_is_direct_flight(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(hops(vm, &c).len() == 2)])
}

/// A route's (`TaxiGetNodeSlot(node, route, source)`) end's slot.
fn route_end(vm: &mut Vm, c: &NativeCall, source: bool) -> Option<usize> {
    let Value::Num(route) = vm.hidden(c.arg(2)) else {
        return None;
    };
    let path = hops(vm, c);
    let i = (route as usize).checked_sub(1)?;
    if i + 1 >= path.len() {
        return None;
    }
    Some(if source { path[i] } else { path[i + 1] })
}

fn taxi_get_node_slot(vm: &mut Vm, c: NativeCall) -> Ret {
    let source = c.arg(3).truthy();
    Ok(match route_end(vm, &c, source) {
        Some(s) => vec![Value::Num(s as f64)],
        // No such route (the map closed): slot 1 (the full probe).
        None => vec![Value::Num(1.0)],
    })
}

/// A route end's map position (`TaxiGetSrcX`, `…Y`, `TaxiGetDestX`, `…Y`).
fn route_xy(vm: &mut Vm, c: &NativeCall, source: bool, key: &str) -> Ret {
    let Some(slot) = route_end(vm, c, source) else {
        // No such route (the map closed): 0 (the full probe).
        return Ok(vec![Value::Num(0.0)]);
    };
    let Some(t) = list(vm, "nodes").get(slot - 1).copied() else {
        return Ok(vec![]);
    };
    Ok(vec![field(vm, t, key)])
}

fn taxi_get_src_x(vm: &mut Vm, c: NativeCall) -> Ret {
    route_xy(vm, &c, true, "x")
}

fn taxi_get_src_y(vm: &mut Vm, c: NativeCall) -> Ret {
    route_xy(vm, &c, true, "y")
}

fn taxi_get_dest_x(vm: &mut Vm, c: NativeCall) -> Ret {
    route_xy(vm, &c, false, "x")
}

fn taxi_get_dest_y(vm: &mut Vm, c: NativeCall) -> Ret {
    route_xy(vm, &c, false, "y")
}

fn taxi_node_position(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = node(vm, &c) else {
        return Ok(vec![]);
    };
    Ok(vec![field(vm, t, "x"), field(vm, t, "y")])
}

/// A node's position as the client gives it: Blizzard's `Vector2D`.
fn position(vm: &mut Vm, t: TableRef) -> Value {
    let x = field(vm, t, "x");
    let y = field(vm, t, "y");
    let create = vm.get_global("CreateVector2D");
    if !create.is_nil()
        && let Ok(r) = vm.call(create, &[x, y])
        && let Some(v) = r.first()
    {
        return *v;
    }
    let p = vm.new_table();
    set(vm, p, "x", x);
    set(vm, p, "y", y);
    Value::Table(p)
}

fn copy(vm: &mut Vm, from: TableRef, to: TableRef, keys: &[&str]) {
    for k in keys {
        let v = field(vm, from, k);
        set(vm, to, k, v);
    }
}

/// `C_TaxiMap.GetAllTaxiNodes(mapID)`: the master's nodes, by slot.
fn get_all_taxi_nodes(vm: &mut Vm, _c: NativeCall) -> Ret {
    let out = vm.new_table();
    for (i, n) in list(vm, "nodes").into_iter().enumerate() {
        let e = vm.new_table();
        copy(vm, n, e, &["name", "nodeID", "state"]);
        set(vm, e, "isMapLayerTransition", Value::Bool(false));
        set(vm, e, "useSpecialIcon", Value::Bool(false));
        set(vm, e, "slotIndex", Value::Num((i + 1) as f64));
        let p = position(vm, n);
        set(vm, e, "position", p);
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num((i + 1) as f64), Value::Table(e));
    }
    Ok(vec![Value::Table(out)])
}

/// `C_TaxiMap.GetTaxiNodesForMap(mapID)`: every node the map shows. A
/// world map's are the client's (`/stdtest engine maps`, every map of
/// Forever's world recorded): each map of a continent lists all of that
/// continent's nodes (Kalimdor's 38, the Eastern Kingdoms' 36, both
/// factions', none undiscovered), Azeroth (947) both continents', Alterac
/// Valley its two, the rest none; the positions are the client's on each
/// map. A flight map never recorded (1464) shows the open flight master's
/// nodes (the test's `mapNodes`). Unverified: another character's
/// undiscovered nodes; a world map's answer while a flight master's map is
/// open (recorded closed).
fn get_taxi_nodes_for_map(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(values) = crate::recorded::lookup(vm, "C_TaxiMap.GetTaxiNodesForMap", &c) {
        return Ok(values);
    }
    let out = vm.new_table();
    for (i, n) in list(vm, "mapNodes").into_iter().enumerate() {
        let e = vm.new_table();
        copy(
            vm,
            n,
            e,
            &["name", "nodeID", "atlasName", "faction", "isUndiscovered"],
        );
        let p = position(vm, n);
        set(vm, e, "position", p);
        let _ = vm
            .heap
            .table_mut(out)
            .set(Value::Num((i + 1) as f64), Value::Table(e));
    }
    Ok(vec![Value::Table(out)])
}

/// `CloseTaxiMap()`: `TAXIMAP_CLOSED()`, the interaction's hides.
fn close_taxi_map(vm: &mut Vm, _c: NativeCall) -> Ret {
    close(vm);
    Ok(vec![])
}

fn close(vm: &mut Vm) {
    if engine_mut(vm).world.taxis.open.take().is_none() {
        return;
    }
    engine_mut(vm).units.npc = None;
    fire(vm, "TAXIMAP_CLOSED", &[]);
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(TAXI)],
    );
    fire(
        vm,
        "PLAYER_INTERACTION_MANAGER_FRAME_HIDE",
        &[Value::Num(0.0)],
    );
}

fn later(vm: &mut Vm, after: f64, name: &'static str, f: NativeFn, upvals: Vec<Value>) {
    let native = vm.native_with(name, f, upvals);
    crate::clock::after_engine(vm, after, native);
}

/// `TakeTaxiNode(slot)`: the flight to the node (its `cost`, its
/// `flightTime`), the server's a round trip later.
fn take_taxi_node(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(t) = node(vm, &c) else {
        return Ok(vec![]);
    };
    let cost = field(vm, t, "cost");
    let time = field(vm, t, "flightTime");
    let after = crate::clock::server_delay(vm);
    later(vm, after, "taxiDeparted", departed, vec![cost, time]);
    Ok(vec![])
}

/// The server took the player's control: the map closed, the spells
/// unusable.
fn departed(vm: &mut Vm, c: NativeCall) -> Ret {
    close(vm);
    engine_mut(vm).world.taxi_stage = 1;
    fire(vm, "PLAYER_CONTROL_LOST", &[]);
    fire(vm, "VEHICLE_ANGLE_SHOW", &[Value::Nil]);
    fire(vm, "VEHICLE_POWER_SHOW", &[Value::Nil]);
    fire(vm, "TAXIMAP_CLOSED", &[]);
    crate::actions::usability_changed(vm);
    fire(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    fire(vm, "SPELL_UPDATE_USABLE", &[]);
    fire(vm, "UPDATE_SHAPESHIFT_USABLE", &[]);
    later(
        vm,
        BOARDED_AFTER,
        "taxiBoarded",
        boarded,
        vec![c.upvals[0], c.upvals[1]],
    );
    Ok(vec![])
}

/// On the taxi: mounted, the fare paid; the landing a flight's time later.
fn boarded(vm: &mut Vm, c: NativeCall) -> Ret {
    fire(vm, "UPDATE_BONUS_ACTIONBAR", &[]);
    engine_mut(vm).world.on_taxi = true;
    engine_mut(vm).world.taxi_stage = 2;
    units::fire_unit(vm, "UNIT_FLAGS", units::PLAYER, &[]);
    crate::actions::usability_changed(vm);
    engine_mut(vm).world.mounted = true;
    fire(vm, "PLAYER_MOUNT_DISPLAY_CHANGED", &[]);
    if let Value::Num(cost) = c.upvals[0]
        && cost > 0.0
    {
        let money = &mut engine_mut(vm).world.character.money;
        *money = (*money - cost).max(0.0);
        fire(vm, "PLAYER_MONEY", &[]);
    }
    fire(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    later(
        vm,
        SHAPESHIFT_AFTER,
        "taxiShapeshift",
        shapeshift_usable,
        vec![],
    );
    if let Value::Num(time) = c.upvals[1] {
        later(
            vm,
            (time - BOARDED_AFTER).max(0.0),
            "taxiLanded",
            landed,
            vec![],
        );
    }
    Ok(vec![])
}

fn shapeshift_usable(vm: &mut Vm, _c: NativeCall) -> Ret {
    fire(vm, "UPDATE_SHAPESHIFT_USABLE", &[]);
    Ok(vec![])
}

/// Landed: the control back, off the taxi and the mount, the spells usable.
fn landed(vm: &mut Vm, _c: NativeCall) -> Ret {
    engine_mut(vm).world.taxi_stage = 0;
    fire(vm, "PLAYER_CONTROL_GAINED", &[]);
    fire(vm, "VEHICLE_UPDATE", &[]);
    fire(vm, "VEHICLE_ANGLE_SHOW", &[Value::Nil]);
    fire(vm, "VEHICLE_POWER_SHOW", &[Value::Nil]);
    fire(vm, "UPDATE_BONUS_ACTIONBAR", &[]);
    engine_mut(vm).world.on_taxi = false;
    units::fire_unit(vm, "UNIT_FLAGS", units::PLAYER, &[]);
    crate::actions::usability_changed(vm);
    engine_mut(vm).world.mounted = false;
    fire(vm, "PLAYER_MOUNT_DISPLAY_CHANGED", &[]);
    fire(vm, "ACTIONBAR_UPDATE_USABLE", &[]);
    fire(vm, "SPELL_UPDATE_USABLE", &[]);
    Ok(vec![])
}

/// `HasFullControl()`: not from a flight's start till it lands.
fn has_full_control(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).world.taxi_stage == 0)])
}

fn is_mounted(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(engine(vm).world.mounted)])
}

pub fn behaviours() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("GetTaxiMapID", get_taxi_map_id),
        ("NumTaxiNodes", num_taxi_nodes),
        ("TaxiNodeName", taxi_node_name),
        ("TaxiNodeCost", taxi_node_cost),
        ("TaxiNodeGetType", taxi_node_get_type),
        ("TaxiNodePosition", taxi_node_position),
        ("GetNumRoutes", get_num_routes),
        ("C_TaxiMap.GetAllTaxiNodes", get_all_taxi_nodes),
        ("C_TaxiMap.GetTaxiNodesForMap", get_taxi_nodes_for_map),
        ("CloseTaxiMap", close_taxi_map),
        ("TakeTaxiNode", take_taxi_node),
        ("TaxiGetNodeSlot", taxi_get_node_slot),
        ("TaxiIsDirectFlight", taxi_is_direct_flight),
        ("TaxiGetSrcX", taxi_get_src_x),
        ("TaxiGetSrcY", taxi_get_src_y),
        ("TaxiGetDestX", taxi_get_dest_x),
        ("TaxiGetDestY", taxi_get_dest_y),
        ("HasFullControl", has_full_control),
        ("IsMounted", is_mounted),
    ]
}
