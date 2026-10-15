//! The client's functions for secret values (FrameScript): wrapping,
//! testing, access checks and copies (spec emulator §17.5, as probed with
//! `/stdtest probe`).

use crate::error::LuaError;
use crate::function::NativeCall;
use crate::heap::TableRef;
use crate::stdlib::Ret;
use crate::table::{DISALLOW_TAINTED_ACCESS, SECRET_WRAP_CONTENTS};
use crate::value::Value;
use crate::vm::Vm;

pub fn open(vm: &mut Vm) {
    for (name, f) in [
        ("secretwrap", secretwrap as crate::NativeFn),
        ("issecretvalue", issecretvalue),
        ("hasanysecretvalues", hasanysecretvalues),
        ("scrubsecretvalues", scrubsecretvalues),
        ("canaccesssecrets", canaccesssecrets),
        ("canaccessvalue", canaccessvalue),
        ("canaccessallvalues", canaccessallvalues),
        ("canaccesstable", canaccesstable),
        ("issecrettable", issecrettable),
        ("dropsecretaccess", dropsecretaccess),
        ("mapvalues", mapvalues),
        ("dumpobject", dumpobject),
        ("settablesecurity", settablesecurity),
        ("securecopy", securecopy),
    ] {
        let v = vm.native(name, f);
        vm.set_global(name, v);
    }
}

fn is_secret(v: Value) -> bool {
    matches!(v, Value::Secret(_))
}

/// Every argument made secret.
fn secretwrap(vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(c.args.iter().map(|&v| vm.secret_wrap(v)).collect())
}

fn issecretvalue(_vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(is_secret(c.arg(1)))])
}

fn hasanysecretvalues(_vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(c.args.iter().any(|&v| is_secret(v)))])
}

/// The arguments with secrets replaced by nil.
fn scrubsecretvalues(_vm: &mut Vm, c: NativeCall) -> Ret {
    Ok(c.args
        .iter()
        .map(|&v| if is_secret(v) { Value::Nil } else { v })
        .collect())
}

fn canaccesssecrets(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(vm.can_access_secrets())])
}

fn canaccessvalue(vm: &mut Vm, c: NativeCall) -> Ret {
    let ok = !is_secret(c.arg(1)) || vm.can_access_secrets();
    Ok(vec![Value::Bool(ok)])
}

fn canaccessallvalues(vm: &mut Vm, c: NativeCall) -> Ret {
    let ok = !c.args.iter().any(|&v| is_secret(v)) || vm.can_access_secrets();
    Ok(vec![Value::Bool(ok)])
}

/// Whether the caller may index the table: not a secret it cannot use,
/// nor one whose security keeps tainted code out.
fn canaccesstable(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = c.arg(1);
    let access = vm.can_access_secrets();
    let ok = match v {
        Value::Secret(_) => access,
        Value::Table(t) => access || vm.heap.table(t).security & DISALLOW_TAINTED_ACCESS == 0,
        _ => true,
    };
    Ok(vec![Value::Bool(ok)])
}

/// Whether the value is a secret table, or a table whose reads give
/// secrets.
fn issecrettable(vm: &mut Vm, c: NativeCall) -> Ret {
    let secret = match c.arg(1) {
        Value::Secret(_) => true,
        Value::Table(t) => vm.heap.table(t).security & SECRET_WRAP_CONTENTS != 0,
        _ => false,
    };
    Ok(vec![Value::Bool(secret)])
}

/// The calling function gives up its access to secrets.
fn dropsecretaccess(vm: &mut Vm, _c: NativeCall) -> Ret {
    if let Some(f) = vm.cur.frames.last_mut() {
        f.secrets_dropped = true;
    }
    Ok(vec![])
}

/// `f(v)` for each value, its first result in the value's place.
fn mapvalues(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = c.arg(1);
    let mut out = Vec::with_capacity(c.len().saturating_sub(1));
    for &v in c.args.iter().skip(1) {
        let r = vm.call(f, &[v])?;
        out.push(r.first().copied().unwrap_or_default());
    }
    Ok(out)
}

/// The value's `__dump` metamethod's result, or nothing to show.
fn dumpobject(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = c.arg(1);
    let dump = vm.heap.intern(b"__dump");
    let tm = vm.get_tm(v, dump);
    if tm.is_nil() {
        return Ok(vec![Value::Nil]);
    }
    let r = vm.call(tm, &[v])?;
    Ok(vec![r.first().copied().unwrap_or_default()])
}

/// Sets one of the table's security options (`Enum.TableSecurityOption`);
/// an unknown option does nothing.
fn settablesecurity(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let option = vm.check_int(&c, 2)?;
    if (0..3).contains(&option) {
        vm.heap.table_mut(t).security |= 1 << option;
    }
    Ok(vec![])
}

const SECURECOPY_USAGE: &str = "Usage: local copy = securecopy([value, options])";

/// A deep copy taking the running taint: shared and recursive references
/// are kept, functions are wrapped to run with their own taint, and
/// secrets the caller cannot use, or frames, are refused.
fn securecopy(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = c.arg(1);
    if is_secret(v) && !vm.can_access_secrets() {
        return Err(vm.secret_usage_error(1, SECURECOPY_USAGE));
    }
    let mut seen = Vec::new();
    let copy = copy_value(vm, v, &mut seen, 0)?;
    Ok(vec![copy])
}

/// An error of `securecopy`, with the running taint as the client adds it.
fn copy_error(vm: &mut Vm, msg: &str) -> LuaError {
    let taint = vm.taint().stack;
    let mut text = format!("securecopy(): {msg}");
    if let Some(name) = vm.taint_name(taint) {
        text.push_str(&format!("\nLua Taint: {name}"));
    }
    let v = vm.str(&text);
    vm.raise(v)
}

fn copy_value(
    vm: &mut Vm,
    v: Value,
    seen: &mut Vec<(TableRef, TableRef)>,
    depth: usize,
) -> Result<Value, LuaError> {
    match v {
        Value::Secret(_) if !vm.can_access_secrets() => {
            let t = vm.type_of(v);
            Err(copy_error(
                vm,
                &format!("attempted to copy an inaccessible secret {t} value"),
            ))
        }
        Value::Table(t) => copy_table(vm, t, seen, depth),
        Value::Func(_) => Ok(wrap_function(vm, v)),
        other => Ok(other),
    }
}

fn copy_table(
    vm: &mut Vm,
    t: TableRef,
    seen: &mut Vec<(TableRef, TableRef)>,
    depth: usize,
) -> Result<Value, LuaError> {
    if let Some(&(_, copy)) = seen.iter().find(|(from, _)| *from == t) {
        return Ok(Value::Table(copy));
    }
    // A frame: its userdata at [0].
    if matches!(vm.heap.table(t).get(Value::Num(0.0)), Value::Userdata(_)) {
        return Err(copy_error(
            vm,
            "cannot securely copy a script object table (operation not permitted by this call)",
        ));
    }
    let copy = vm.new_table();
    seen.push((t, copy));
    let entries: Vec<(Value, Value)> = vm.heap.table(t).entries().collect();
    for (k, v) in entries {
        let k = copy_value(vm, k, seen, depth + 1)?;
        let v = copy_value(vm, v, seen, depth + 1)?;
        vm.set_index(Value::Table(copy), k, v, None)?;
    }
    Ok(Value::Table(copy))
}

/// A function the copy calls through, as `securecopy`'s
/// `wrapUntrustedFunctions` does.
fn wrap_function(vm: &mut Vm, f: Value) -> Value {
    vm.native_with("securecopy", call_through, vec![f])
}

fn call_through(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = c.upval_slot(0);
    let r = vm.call_slots(f, &c.slots(1))?;
    Ok(vm.return_slots(r))
}
