//! The engine's controls for tests (spec Tooling §16.1.4): what
//! `wowlua:mock` drives. A table of natives the test runner installs; addon
//! code never sees it.

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, Value, Vm};

use crate::{clock, engine, widgets};

/// `fireEvent(name, ...)`: fires an event the flavor knows, the payload
/// checked against its documentation (decision T202).
fn fire_event(vm: &mut Vm, c: NativeCall) -> Ret {
    let name = vm.check_str(&c, 1)?;
    let name = String::from_utf8_lossy(vm.str_bytes(name)).into_owned();
    if !engine(vm).api.events.contains(&name) {
        return Err(vm.error(&format!(
            "mock.fireEvent: \"{name}\" is not an event of this flavor"
        )));
    }
    let args: Vec<Value> = c.args.iter().skip(1).copied().collect();
    if let Some(doc) = engine(vm).api.event_docs.get(&name).cloned() {
        check_payload(vm, &name, &doc, &args)?;
    }
    widgets::fire_event(vm, &name, &args);
    Ok(vec![])
}

/// The payload's count and types against the event's documentation.
fn check_payload(
    vm: &mut Vm,
    name: &str,
    doc: &serde_json::Value,
    args: &[Value],
) -> Result<(), wowlua_vm::LuaError> {
    let payload = doc
        .get("Payload")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if args.len() > payload.len() {
        return Err(vm.error(&format!(
            "mock.fireEvent: {name} has {} payload values, {} given",
            payload.len(),
            args.len()
        )));
    }
    for (i, p) in payload.iter().enumerate() {
        let field = p
            .get("Name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?");
        let nilable = p
            .get("Nilable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let ty = p
            .get("Type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let v = args.get(i).copied().unwrap_or(Value::Nil);
        let h = vm.hidden(v);
        if h.is_nil() {
            if !nilable {
                return Err(vm.error(&format!("mock.fireEvent: {name} needs `{field}` ({ty})")));
            }
            continue;
        }
        let expected = match ty {
            "bool" => Some("boolean"),
            "cstring" | "string" | "UnitToken" | "WOWGUID" => Some("string"),
            "number" | "luaIndex" => Some("number"),
            "table" => Some("table"),
            _ => None,
        };
        if let Some(e) = expected
            && h.type_name() != e
        {
            return Err(vm.error(&format!(
                "mock.fireEvent: {name}'s `{field}` is {ty}, got {}",
                h.type_name()
            )));
        }
    }
    Ok(())
}

/// `fireUnitEvent(name, unit, ...)`: an event for a unit.
fn fire_unit_event(vm: &mut Vm, c: NativeCall) -> Ret {
    fire_event(vm, c)
}

fn advance_time(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.opt_number(&c, 1, 0.0)?;
    clock::advance(vm, s);
    Ok(vec![])
}

fn next_frame(vm: &mut Vm, c: NativeCall) -> Ret {
    let ms = vm.opt_number(&c, 1, 1000.0 / 60.0)?;
    clock::advance_frames(vm, 1, ms);
    Ok(vec![])
}

fn advance_frames(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = vm.check_int(&c, 1)?;
    let ms = vm.opt_number(&c, 2, 1000.0 / 60.0)?;
    clock::advance_frames(vm, n.max(0) as usize, ms);
    Ok(vec![])
}

fn warnings(vm: &mut Vm, _c: NativeCall) -> Ret {
    let list = engine(vm).world.warnings.clone();
    let t = vm.new_table();
    for (i, w) in list.iter().enumerate() {
        let s = vm.str(w);
        let _ = vm.heap.table_mut(t).set(Value::Num(i as f64 + 1.0), s);
    }
    Ok(vec![Value::Table(t)])
}

/// Installs the controls as global `name` (the test runner's; `wowlua:mock`
/// wraps them).
pub fn install(vm: &mut Vm, name: &str) {
    let t = vm.new_table();
    for (key, f) in [
        ("fireEvent", fire_event as wowlua_vm::NativeFn),
        ("fireUnitEvent", fire_unit_event),
        ("advanceTime", advance_time),
        ("nextFrame", next_frame),
        ("advanceFrames", advance_frames),
        ("warnings", warnings),
    ] {
        let native = vm.native(key, f);
        let k = vm.heap.intern(key.as_bytes());
        vm.heap.table_mut(t).set_str(k, native);
    }
    vm.set_global(name, Value::Table(t));
}
