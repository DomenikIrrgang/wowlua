//! Evaluation (spec debugger §18.7): code compiled as a chunk whose
//! environment reads the frame's locals and upvalues first, then the
//! frame's environment; run with the frame's execution taint; locals and
//! upvalues it assigns written back to the frame. A watch or a hover only
//! reads (decision T496): it assigns nothing, and calls only functions
//! known to change nothing.

use rustc_hash::FxHashSet;
use wowlua_vm::function::Function;
use wowlua_vm::heap::FuncRef;
use wowlua_vm::{CallGuard, Value, Vm};

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

/// The environment of code that only reads: assigning is an error.
const READ_ENV_HELPER: &[u8] = b"local names, values, n, genv = ...
local own, env = {}, {}
for i = 1, n do own[names[i]] = true; env[names[i]] = values[i] end
return setmetatable(env, {
    __index = function(_, k) if own[k] then return nil end return genv[k] end,
    __newindex = function(_, k) error('a watch only reads: it cannot set ' .. tostring(k), 2) end,
}), own";

/// The debugger's own chunks (the environment's helpers).
const DEBUGGER_CHUNK: &str = "=(debugger)";

/// The Lua library's functions a watch may call: they change nothing
/// (`math.random` moves the generator, `table.sort` the table: not these).
const PURE: &[(&str, &[&str])] = &[
    ("string", &["*"]),
    ("bit", &["*"]),
    (
        "math",
        &[
            "abs", "acos", "asin", "atan", "atan2", "ceil", "cos", "cosh", "deg", "exp", "floor",
            "fmod", "frexp", "ldexp", "log", "log10", "max", "min", "modf", "pow", "rad", "sin",
            "sinh", "sqrt", "tan", "tanh",
        ],
    ),
    ("table", &["concat", "getn", "maxn", "unpack"]),
    ("coroutine", &["status", "running", "isyieldable"]),
    ("os", &["clock", "date", "difftime", "time"]),
    (
        "_G",
        &[
            "type",
            "tostring",
            "tonumber",
            "select",
            "unpack",
            "next",
            "pairs",
            "ipairs",
            "rawget",
            "rawequal",
            "rawlen",
            "getmetatable",
            "assert",
            "error",
            "pcall",
            "xpcall",
            "issecretvalue",
            "issecrettable",
            "strlen",
            "strsub",
            "strfind",
            "strmatch",
            "strupper",
            "strlower",
            "strrep",
            "strrev",
            "strbyte",
            "strchar",
            "format",
            "gsub",
            "gmatch",
            "strsplit",
            "strjoin",
            "strtrim",
            "strconcat",
            "tostringall",
            "floor",
            "ceil",
            "abs",
            "max",
            "min",
            "sqrt",
            "mod",
            "date",
            "time",
        ],
    ),
];

/// The library's tables: what is in them and not pure is refused, whatever
/// its name.
const LIBRARY: &[&str] = &[
    "string",
    "bit",
    "math",
    "table",
    "coroutine",
    "os",
    "io",
    "debug",
];

/// The library's functions: the pure ones, and all of them.
fn library_functions(vm: &mut Vm) -> (FxHashSet<FuncRef>, FxHashSet<FuncRef>) {
    let mut pure = FxHashSet::default();
    let mut all = FxHashSet::default();
    let globals = vm.globals;
    let table_of = |vm: &mut Vm, name: &str| -> Option<wowlua_vm::TableRef> {
        if name == "_G" {
            return Some(globals);
        }
        match vm.get_global(name) {
            Value::Table(t) => Some(t),
            _ => None,
        }
    };
    let funcs = |vm: &Vm, t: wowlua_vm::TableRef| -> Vec<FuncRef> {
        vm.heap
            .table(t)
            .entries()
            .filter_map(|(_, v)| match v {
                Value::Func(f) => Some(f),
                _ => None,
            })
            .collect()
    };
    for name in LIBRARY {
        if let Some(t) = table_of(vm, name) {
            all.extend(funcs(vm, t));
        }
    }
    for (lib, names) in PURE {
        let Some(t) = table_of(vm, lib) else { continue };
        if names == &["*"] {
            pure.extend(funcs(vm, t));
            continue;
        }
        for name in *names {
            let key = vm.heap.intern(name.as_bytes());
            if let Value::Func(f) = vm.heap.table(t).get_str(key) {
                pure.insert(f);
            }
        }
    }
    (pure, all)
}

/// Whether the game's function or method by this name only reads: a
/// query by its name (`GetText`, `IsShown`, `UnitHealth`,
/// `C_Spell.GetSpellInfo`), nothing it makes.
fn reads(name: &str) -> bool {
    let short = name.rsplit(['.', ':']).next().unwrap_or(name);
    ["Get", "Is", "Has", "Can", "Unit", "Should", "Does"]
        .iter()
        .any(|p| short.starts_with(p))
        && !short.contains("Create")
}

/// The check of code that only reads: the debugger's own helpers, pure
/// library functions and the game's queries are called; anything else is
/// refused (it may change the game).
fn reads_only(vm: &mut Vm) -> Box<CallGuard> {
    let (pure, library) = library_functions(vm);
    Box::new(move |vm: &Vm, f: FuncRef| match vm.heap.func(f) {
        Function::Lua(l) if &*l.proto.source == DEBUGGER_CHUNK.as_bytes() => None,
        Function::Lua(l) => Some(format!(
            "a watch only reads: it does not call {} (it may change the game); evaluate it in the console",
            if l.proto.line_defined == 0 {
                "a chunk".to_string()
            } else {
                format!(
                    "the function at {}:{}",
                    String::from_utf8_lossy(&wowlua_vm::vm::chunkid(&l.proto.source)),
                    l.proto.line_defined
                )
            }
        )),
        Function::Native(n) => {
            if pure.contains(&f) || (!library.contains(&f) && reads(n.name)) {
                None
            } else {
                Some(format!(
                    "a watch only reads: it does not call {}() (it may change the game); evaluate it in the console",
                    n.name
                ))
            }
        }
    })
}

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
    match vm.load(as_expr.as_bytes(), "=(watch)") {
        Ok(f) => Ok(f),
        Err(_) => vm.load(code.as_bytes(), "=(watch)"),
    }
}

/// Runs `code` in the frame: its results, or its error's text. Code that
/// only reads (`reading`) assigns nothing and calls nothing that may
/// change the game.
fn run(vm: &mut Vm, at: At, code: &str, reading: bool) -> Result<Vec<Value>, String> {
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
        .load(
            if reading { READ_ENV_HELPER } else { ENV_HELPER },
            DEBUGGER_CHUNK,
        )
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
        if reading {
            let guard = reads_only(vm);
            let watch = fr;
            vm.set_call_guard(Some(Box::new(
                move |vm: &Vm, g: FuncRef| {
                    if g == watch { None } else { guard(vm, g) }
                },
            )));
        }
        let results = vm.pcall(f, &[]);
        if reading {
            vm.set_call_guard(None);
        }
        let results = results.map_err(|e| values::error_text(vm, e.value))?;
        // Locals and upvalues the code assigned go back to the frame; code
        // that only reads assigns none.
        for (name, old, place) in &vars {
            let k = vm.str(name);
            let now = vm.heap.table(env).get(k);
            if !now.raw_eq(*old) && reading {
                return Err(format!("a watch only reads: it cannot set {name}"));
            }
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
    context: EvalContext,
) -> Result<Variable, String> {
    let at = frame_at(vm, paused.as_deref(), frame);
    let reading = matches!(context, EvalContext::Watch | EvalContext::Hover);
    let results = run(vm, at, code, reading)?;
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
        match run(vm, at, expr, false) {
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
    let v = run(vm, at, value, false)?
        .first()
        .copied()
        .unwrap_or(Value::Nil);
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
                Some(k) => run(vm, at, k, false)?
                    .first()
                    .copied()
                    .unwrap_or(Value::Nil),
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
