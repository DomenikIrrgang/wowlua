//! Evaluation (spec debugger §18.7): code compiled as a chunk whose
//! environment reads the frame's locals and upvalues first, then the
//! frame's environment; run with the frame's execution taint; locals and
//! upvalues it assigns written back to the frame.

use wowlua_vm::heap::FuncRef;
use wowlua_vm::{Value, Vm};

use crate::controller::{FrameRef, Paused};
use crate::protocol::{EvalContext, Variable};
use crate::values::{self, RefTarget};

/// Makes the environment: `(names, values, n, genv)` → env, where the
/// names in `names` read and write the env's own fields.
const ENV_HELPER: &[u8] = b"local names, values, n, genv = ...
local own, env = {}, {}
for i = 1, n do own[names[i]] = true; env[names[i]] = values[i] end
return setmetatable(env, {
    __index = function(_, k) if own[k] then return nil end return genv[k] end,
    __newindex = function(t, k, v) if own[k] then rawset(t, k, v) else genv[k] = v end end,
}), own";

/// The frame an evaluation runs in: a Lua frame's index and function.
type At = Option<(usize, FuncRef)>;

fn frame_at(vm: &Vm, paused: Option<&Paused>, frame: Option<u32>) -> At {
    if let (Some(p), Some(id)) = (paused, frame) {
        return match p.frames.get(id.saturating_sub(1) as usize) {
            Some(FrameRef::Lua { index, func }) => Some((*index, *func)),
            _ => None,
        };
    }
    vm.lua_frames().first().map(|f| (f.index, f.func))
}

/// The frame's variables visible to its code: upvalues, then locals (a
/// later local of a name shadows an earlier one); each with how to write
/// it back.
fn visible(vm: &Vm, at: At) -> Vec<(String, Value, Place)> {
    let Some((index, func)) = at else {
        return Vec::new();
    };
    let mut out: Vec<(String, Value, Place)> = Vec::new();
    for (i, (name, s)) in vm.upvalues(func).into_iter().enumerate() {
        out.push((name.to_string(), s.v, Place::Upvalue(func, i)));
    }
    for (name, slot) in vm.frame_locals(index) {
        if name.starts_with('(') {
            continue;
        }
        let v = vm.stack_slot(slot).v;
        out.retain(|(n, _, _)| **n != *name);
        out.push((name.to_string(), v, Place::Local(slot)));
    }
    out
}

#[derive(Clone, Copy)]
enum Place {
    Local(usize),
    Upvalue(FuncRef, usize),
}

fn compile(vm: &mut Vm, code: &str) -> Result<Value, String> {
    let as_expr = format!("return {code}");
    match vm.load(as_expr.as_bytes(), "=(debugger)") {
        Ok(f) => Ok(f),
        Err(_) => vm.load(code.as_bytes(), "=(debugger)"),
    }
}

/// Runs `code` in the frame: its results, or its error's text.
fn run(vm: &mut Vm, at: At, code: &str) -> Result<Vec<Value>, String> {
    let f = compile(vm, code)?;
    let vars = visible(vm, at);
    let names = vm.new_table();
    let vals = vm.new_table();
    for (i, (n, v, _)) in vars.iter().enumerate() {
        let k = Value::Num((i + 1) as f64);
        let n = vm.str(n);
        let _ = vm.heap.table_mut(names).set(k, n);
        let _ = vm.heap.table_mut(vals).set(k, *v);
    }
    let genv = match at {
        Some((_, func)) => vm.func_env(func),
        None => vm.globals,
    };
    let helper = vm
        .load(ENV_HELPER, "=(debugger)")
        .map_err(|e| format!("the debugger's environment: {e}"))?;
    let taint = vm.execution_taint();
    vm.with_execution_taint(taint, |vm| -> Result<Vec<Value>, String> {
        let made = vm
            .pcall(
                helper,
                &[
                    Value::Table(names),
                    Value::Table(vals),
                    Value::Num(vars.len() as f64),
                    Value::Table(genv),
                ],
            )
            .map_err(|e| values::error_text(vm, e.value))?;
        let Some(Value::Table(env)) = made.first().copied() else {
            return Err("the debugger's environment".into());
        };
        let Value::Func(fr) = f else {
            return Err("not a function".into());
        };
        vm.set_func_env(fr, env);
        let results = vm
            .pcall(f, &[])
            .map_err(|e| values::error_text(vm, e.value))?;
        // Locals and upvalues the code assigned go back to the frame.
        for (name, old, place) in &vars {
            let k = vm.str(name);
            let now = vm.heap.table(env).get(k);
            if !now.raw_eq(*old) {
                let s = vm.taint().fresh(now);
                match *place {
                    Place::Local(slot) => vm.set_stack_slot(slot, s),
                    Place::Upvalue(func, i) => vm.set_upvalue(func, i, s),
                }
            }
        }
        Ok(results)
    })
}

/// Evaluates `code` in a frame (`frame`: a stack frame's id while
/// stopped; else the running frame, or the globals between frames).
pub(crate) fn evaluate(
    vm: &mut Vm,
    paused: Option<&mut Paused>,
    frame: Option<u32>,
    code: &str,
    _context: EvalContext,
) -> Result<Variable, String> {
    let at = frame_at(vm, paused.as_deref(), frame);
    let results = run(vm, at, code)?;
    let first = results.first().copied().unwrap_or(Value::Nil);
    match paused {
        Some(p) => {
            let s = wowlua_vm::Slot::secure(first);
            let mut var = values::variable(vm, p, String::new(), s);
            if results.len() > 1 {
                let rest: Vec<String> = results[1..]
                    .iter()
                    .map(|&v| values::describe(vm, v).0)
                    .collect();
                var.value = format!("{}, {}", var.value, rest.join(", "));
            }
            Ok(var)
        }
        None => {
            let texts: Vec<String> = results.iter().map(|&v| values::describe(vm, v).0).collect();
            let (_, ty) = values::describe(vm, first);
            Ok(Variable {
                name: String::new(),
                value: if texts.is_empty() {
                    "nil".into()
                } else {
                    texts.join(", ")
                },
                ty,
                reference: 0,
                taint: "secure".into(),
                secret: matches!(first, Value::Secret(_)),
            })
        }
    }
}

/// A log message with its `{expr}` parts evaluated in the running frame.
pub(crate) fn log_message(vm: &mut Vm, msg: &str) -> String {
    let at = frame_at(vm, None, None);
    let mut out = String::new();
    let mut rest = msg;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let expr = &after[..close];
        match run(vm, at, expr) {
            Ok(r) => {
                let v = r.first().copied().unwrap_or(Value::Nil);
                let v = vm.hidden(v);
                match v {
                    Value::Str(s) => out.push_str(&String::from_utf8_lossy(vm.str_bytes(s))),
                    other => out.push_str(&values::describe(vm, other).0),
                }
            }
            Err(e) => out.push_str(&format!("<{e}>")),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Sets a variable of a reference to the value of an expression.
pub(crate) fn set_variable(
    vm: &mut Vm,
    p: &mut Paused,
    reference: u32,
    name: &str,
    value: &str,
) -> Result<Variable, String> {
    let Some(target) = p.refs.get(reference.saturating_sub(1) as usize).cloned() else {
        return Err(format!("no variables reference {reference}"));
    };
    let top = frame_at(vm, None, None);
    let at = match &target {
        RefTarget::Locals(i) | RefTarget::Varargs(i) => vm
            .lua_frames()
            .into_iter()
            .find(|f| f.index == *i)
            .map(|f| (f.index, f.func)),
        _ => top,
    };
    let v = run(vm, at, value)?.first().copied().unwrap_or(Value::Nil);
    let s = vm.taint().fresh(v);
    match target {
        RefTarget::Locals(i) => {
            let slot = vm
                .frame_locals(i)
                .into_iter()
                .rev()
                .find(|(n, _)| **n == *name)
                .map(|(_, s)| s)
                .ok_or_else(|| format!("no local {name}"))?;
            vm.set_stack_slot(slot, s);
        }
        RefTarget::Upvalues(f) => {
            let i = vm
                .upvalues(f)
                .iter()
                .position(|(n, _)| **n == *name)
                .ok_or_else(|| format!("no upvalue {name}"))?;
            vm.set_upvalue(f, i, s);
        }
        RefTarget::Table(t) | RefTarget::Widget(t) => {
            let key = match name.strip_prefix('[').and_then(|k| k.strip_suffix(']')) {
                Some(k) => run(vm, at, k)?.first().copied().unwrap_or(Value::Nil),
                None => vm.str(name),
            };
            vm.heap
                .table_mut(t)
                .set(key, v)
                .map_err(|_| "a key that cannot be set".to_string())?;
        }
        RefTarget::Varargs(_) => return Err("varargs cannot be set".into()),
    }
    Ok(values::variable(vm, p, name.to_string(), s))
}
