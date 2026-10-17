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
//! Unverified: flying (`TakeTaxiNode`), the routes' hops, a node's state
//! other than current, reachable and unreachable.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, TableRef, Value, Vm};

use crate::units::{self, UnitId};
use crate::{engine, engine_mut, widgets};

/// `Enum.PlayerInteractionType.TaxiNode`.
const TAXI: f64 = 6.0;
/// `Enum.UIMapSystem.Taxi`.
const TAXI_SYSTEM: f64 = 1.0;

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
/// routes } }, mapNodes = { { name, nodeID, x, y, atlasName, faction,
/// isUndiscovered } } })`: `nodes` in the slots' order.
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

fn taxi_node_name(vm: &mut Vm, c: NativeCall) -> Ret {
    node_field(vm, &c, "name")
}

fn taxi_node_cost(vm: &mut Vm, c: NativeCall) -> Ret {
    node_field(vm, &c, "cost")
}

fn get_num_routes(vm: &mut Vm, c: NativeCall) -> Ret {
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

/// `C_TaxiMap.GetTaxiNodesForMap(mapID)`: every node the map shows.
fn get_taxi_nodes_for_map(vm: &mut Vm, _c: NativeCall) -> Ret {
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
    if engine_mut(vm).world.taxis.open.take().is_none() {
        return Ok(vec![]);
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
    Ok(vec![])
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
    ]
}
