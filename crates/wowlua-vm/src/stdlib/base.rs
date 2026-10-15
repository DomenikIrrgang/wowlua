//! Lua 5.1's base library (`lbaselib.c`).

use crate::function::{Function, NativeCall};
use crate::heap::Userdata;
use crate::stdlib::{Ret, register};
use crate::value::Value;
use crate::vm::{StackEntry, Vm};

pub fn open(vm: &mut Vm) {
    let g = Value::Table(vm.globals);
    vm.set_global("_G", g);
    let version = vm.str("Lua 5.1");
    vm.set_global("_VERSION", version);
    let funcs: &[(&'static str, crate::function::NativeFn)] = &[
        ("print", print),
        ("type", type_),
        ("tostring", tostring),
        ("tonumber", tonumber),
        ("rawget", rawget),
        ("rawset", rawset),
        ("rawequal", rawequal),
        ("setmetatable", setmetatable),
        ("getmetatable", getmetatable),
        ("assert", assert),
        ("error", error),
        ("pcall", pcall),
        ("xpcall", xpcall),
        ("select", select),
        ("unpack", unpack),
        ("next", next),
        ("loadstring", loadstring),
        ("load", load),
        ("getfenv", getfenv),
        ("setfenv", setfenv),
        ("newproxy", newproxy),
        ("collectgarbage", collectgarbage),
        ("gcinfo", gcinfo),
    ];
    for (name, f) in funcs {
        let v = vm.native(name, *f);
        vm.set_global(name, v);
    }
    // `pairs` and `ipairs` return iterators held as upvalues.
    let next_fn = vm.get_global("next");
    let pairs_fn = vm.native_with("pairs", pairs, vec![next_fn]);
    vm.set_global("pairs", pairs_fn);
    let inext = vm.native("ipairs_iterator", ipairs_aux);
    let ipairs_fn = vm.native_with("ipairs", ipairs, vec![inext]);
    vm.set_global("ipairs", ipairs_fn);
    let _ = register;
}

pub(crate) fn print(vm: &mut Vm, c: NativeCall) -> Ret {
    let tostring_fn = vm.get_global("tostring");
    let mut out = Vec::new();
    for (i, &v) in c.args.iter().enumerate() {
        let r = vm.call(tostring_fn, &[v])?;
        match r.first() {
            Some(Value::Str(s)) => {
                if i > 0 {
                    out.push(b'\t');
                }
                out.extend_from_slice(vm.heap.str_bytes(*s));
            }
            Some(Value::Num(n)) => {
                if i > 0 {
                    out.push(b'\t');
                }
                out.extend_from_slice(crate::number::fmt_num(*n).as_bytes());
            }
            _ => return Err(vm.error("'tostring' must return a string to 'print'")),
        }
    }
    out.push(b'\n');
    vm.write_output(&out);
    Ok(vec![])
}

fn type_(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = vm.check_any(&c, 1)?;
    Ok(vec![vm.str(v.type_name())])
}

fn tostring(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = vm.check_any(&c, 1)?;
    Ok(vec![vm.tostring(v)?])
}

fn tonumber(vm: &mut Vm, c: NativeCall) -> Ret {
    let base = vm.opt_int(&c, 2, 10)?;
    if base == 10 {
        let v = vm.check_any(&c, 1)?;
        return Ok(vec![vm.tonumber(v).map_or(Value::Nil, Value::Num)]);
    }
    let s = vm.check_bytes(&c, 1)?;
    if !(2..=36).contains(&base) {
        return Err(vm.arg_error(2, "base out of range"));
    }
    Ok(vec![
        strtoul(&s, base as u32).map_or(Value::Nil, Value::Num),
    ])
}

/// `strtoul` for `tonumber(s, base)`: the whole string (but spaces).
fn strtoul(s: &[u8], base: u32) -> Option<f64> {
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    if base == 16 && i + 1 < s.len() && s[i] == b'0' && (s[i + 1] | 0x20) == b'x' {
        i += 2;
    }
    let start = i;
    let mut n: u64 = 0;
    let mut overflow = false;
    while i < s.len() {
        let Some(d) = (s[i] as char).to_digit(base) else {
            break;
        };
        match n
            .checked_mul(u64::from(base))
            .and_then(|x| x.checked_add(u64::from(d)))
        {
            Some(x) => n = x,
            None => overflow = true,
        }
        i += 1;
    }
    if i == start {
        return None;
    }
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    if i != s.len() {
        return None;
    }
    let n = if overflow { u64::MAX } else { n };
    let n = if neg { n.wrapping_neg() } else { n };
    Some(n as f64)
}

// Natives below keep taints as the client's do: values read from tables
// are copied (a tainted one taints the execution), values passed through
// keep their own.

fn rawget(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let k = vm.check_any(&c, 2)?;
    let s = vm.heap.table(t).get_slot(k);
    let s = vm.taint_mut().copy(s);
    Ok(vm.return_slots(vec![s]))
}

fn rawset(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let k = vm.check_any(&c, 2)?;
    vm.check_any(&c, 3)?;
    if let Err(e) = vm.heap.table_mut(t).set_slot(k, c.slot(3)) {
        return Err(vm.rt_error(e.message()));
    }
    Ok(vm.return_slots(vec![c.slot(1)]))
}

fn rawequal(vm: &mut Vm, c: NativeCall) -> Ret {
    let a = vm.check_any(&c, 1)?;
    let b = vm.check_any(&c, 2)?;
    Ok(vec![Value::Bool(a.raw_eq(b))])
}

fn setmetatable(vm: &mut Vm, c: NativeCall) -> Ret {
    let mt = c.arg(2);
    let t = vm.check_table(&c, 1)?;
    let mt = match mt {
        Value::Nil => None,
        Value::Table(m) => Some(m),
        _ => return Err(vm.arg_error(2, "nil or table expected")),
    };
    if let Some(old) = vm.heap.table(t).meta
        && !vm.heap.table(old).get_str(vm.names.metatable).is_nil()
    {
        return Err(vm.error("cannot change a protected metatable"));
    }
    vm.heap.table_mut(t).meta = mt;
    Ok(vec![Value::Table(t)])
}

fn getmetatable(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = vm.check_any(&c, 1)?;
    let Some(mt) = vm.get_metatable(v) else {
        return Ok(vec![Value::Nil]);
    };
    let protected = vm.heap.table(mt).get_str(vm.names.metatable);
    Ok(vec![if protected.is_nil() {
        Value::Table(mt)
    } else {
        protected
    }])
}

fn assert(vm: &mut Vm, c: NativeCall) -> Ret {
    let v = vm.check_any(&c, 1)?;
    if v.truthy() {
        let all = c.slots(1);
        return Ok(vm.return_slots(all));
    }
    let msg = match vm.opt_str(&c, 2)? {
        Some(s) => String::from_utf8_lossy(vm.heap.str_bytes(s)).into_owned(),
        None => "assertion failed!".to_string(),
    };
    Err(vm.error(&msg))
}

fn error(vm: &mut Vm, c: NativeCall) -> Ret {
    let level = vm.opt_int(&c, 2, 1)?;
    let v = c.arg(1);
    if matches!(v, Value::Str(_) | Value::Num(_)) && level > 0 {
        let mut text = vm.where_(level as usize);
        vm.append_strlike(&mut text, v);
        let msg = vm.bytes(&text);
        return Err(vm.raise(msg));
    }
    Err(vm.raise(v))
}

/// `true` and the results, or `false` and the error; results keep the
/// taints the callee left them.
fn protected_results(
    vm: &mut Vm,
    r: Result<Vec<crate::taint::Slot>, crate::error::LuaError>,
) -> Ret {
    let out = match r {
        Ok(mut results) => {
            results.insert(0, vm.taint().fresh(Value::Bool(true)));
            results
        }
        Err(e) => vec![
            vm.taint().fresh(Value::Bool(false)),
            vm.taint().fresh(e.value),
        ],
    };
    Ok(vm.return_slots(out))
}

fn pcall(vm: &mut Vm, c: NativeCall) -> Ret {
    vm.check_any(&c, 1)?;
    // Without an enclosing handler (`lua_pcall` with none).
    let saved = vm.errfunc.take();
    let r = vm.call_slots(c.slot(1), &c.slots(2));
    vm.errfunc = saved;
    protected_results(vm, r)
}

fn xpcall(vm: &mut Vm, c: NativeCall) -> Ret {
    let handler = vm.check_any(&c, 2)?;
    let saved = vm.errfunc.replace(handler);
    // Lua 5.1 calls `f` without arguments; the client passes the rest.
    let args = if vm.rules.xpcall_args {
        c.slots(3)
    } else {
        Vec::new()
    };
    let r = vm.call_slots(c.slot(1), &args);
    vm.errfunc = saved;
    protected_results(vm, r)
}

fn select(vm: &mut Vm, c: NativeCall) -> Ret {
    let n = c.len() as i64;
    if let Value::Str(s) = c.arg(1)
        && vm.heap.str_bytes(s).first() == Some(&b'#')
    {
        return Ok(vec![Value::Num((n - 1) as f64)]);
    }
    let mut i = vm.check_int(&c, 1)?;
    if i < 0 {
        i += n;
    } else if i > n {
        i = n;
    }
    if i < 1 {
        return Err(vm.arg_error(1, "index out of range"));
    }
    let rest = c.slots(i as usize + 1);
    Ok(vm.return_slots(rest))
}

fn unpack(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let i = vm.opt_int(&c, 2, 1)?;
    let e = if c.arg(3).is_nil() {
        vm.heap.table(t).border() as i64
    } else {
        vm.check_int(&c, 3)?
    };
    if i > e {
        return Ok(vec![]);
    }
    let n = e - i + 1;
    if n <= 0 || n >= 8000 {
        return Err(vm.error("too many results to unpack"));
    }
    let slots: Vec<_> = (i..=e)
        .map(|k| vm.heap.table(t).get_slot(Value::Num(k as f64)))
        .collect();
    let slots = slots.into_iter().map(|s| vm.taint_mut().copy(s)).collect();
    Ok(vm.return_slots(slots))
}

fn next(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    match vm.heap.table(t).next_slot(c.arg(2)) {
        Ok(Some((k, v))) => {
            // Keys are stored secure; the value is copied.
            let k = vm.taint().fresh(k);
            let v = vm.taint_mut().copy(v);
            Ok(vm.return_slots(vec![k, v]))
        }
        Ok(None) => Ok(vec![Value::Nil]),
        Err(_) => Err(vm.rt_error("invalid key to 'next'")),
    }
}

fn pairs(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    Ok(vec![c.upvals[0], Value::Table(t), Value::Nil])
}

fn ipairs(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    Ok(vec![c.upvals[0], Value::Table(t), Value::Num(0.0)])
}

fn ipairs_aux(vm: &mut Vm, c: NativeCall) -> Ret {
    let i = vm.check_int(&c, 2)? + 1;
    let t = vm.check_table(&c, 1)?;
    let s = vm.heap.table(t).get_slot(Value::Num(i as f64));
    if s.v.is_nil() {
        Ok(vec![Value::Nil])
    } else {
        let k = vm.taint().fresh(Value::Num(i as f64));
        let s = vm.taint_mut().copy(s);
        Ok(vm.return_slots(vec![k, s]))
    }
}

fn load_result(vm: &mut Vm, r: Result<Value, Vec<u8>>) -> Ret {
    match r {
        Ok(f) => Ok(vec![f]),
        Err(msg) => {
            let m = vm.bytes(&msg);
            Ok(vec![Value::Nil, m])
        }
    }
}

pub(crate) fn loadstring(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let name = match vm.opt_str(&c, 2)? {
        Some(n) => vm.heap.str_bytes(n).to_vec(),
        None => s.to_vec(),
    };
    let r = vm.load_raw(&s, &name);
    load_result(vm, r)
}

fn load(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = vm.check_func(&c, 1)?;
    let name = match vm.opt_str(&c, 2)? {
        Some(n) => vm.heap.str_bytes(n).to_vec(),
        None => b"=(load)".to_vec(),
    };
    let mut src = Vec::new();
    loop {
        // Errors in the reader are load's result, as in Lua.
        let r = match vm.pcall(f, &[]) {
            Ok(r) => r,
            Err(e) => return Ok(vec![Value::Nil, e.value]),
        };
        match r.first().copied().unwrap_or_default() {
            Value::Nil => break,
            Value::Str(s) => {
                let b = vm.heap.str_bytes(s);
                if b.is_empty() {
                    break;
                }
                src.extend_from_slice(b);
            }
            _ => {
                let m = vm.str("reader function must return a string");
                return Ok(vec![Value::Nil, m]);
            }
        }
        if src.len() <= INCREMENTAL_LOAD_LIMIT && syntax_error_read(&src) {
            break;
        }
    }
    let r = vm.load_raw(&src, &name);
    load_result(vm, r)
}

/// Up to this size, `load` stops calling the reader at a syntax error.
const INCREMENTAL_LOAD_LIMIT: usize = 64 << 10;

/// Whether Lua's parser, reading `src` so far, has already met a syntax
/// error: one whose token ends inside it, so the lexer's lookahead
/// character was read. Lua calls the reader no further then.
fn syntax_error_read(src: &[u8]) -> bool {
    let file = wowlua_span::SourceDb::new().add("", "");
    let parse = wowlua_syntax::parse(src, file);
    parse
        .diagnostics
        .iter()
        .find(|d| d.is_error())
        .and_then(|d| d.labels.first())
        .is_some_and(|l| (l.span.end as usize) < src.len())
}

/// The function `getfenv`/`setfenv` refer to: a function, or a level.
fn fenv_target(
    vm: &mut Vm,
    c: &NativeCall,
    fname: &str,
) -> Result<Option<Value>, crate::error::LuaError> {
    if let f @ Value::Func(_) = c.arg(1) {
        return Ok(Some(f));
    }
    let level = vm.opt_int(c, 1, 1)?;
    if level < 0 {
        return Err(vm.arg_error(1, "level must be non-negative"));
    }
    if level == 0 {
        return Ok(None);
    }
    let entries = vm.stack_entries();
    match entries.get(level as usize) {
        Some(StackEntry::Lua(fi)) => Ok(Some(Value::Func(vm.cur.frames[*fi].closure))),
        Some(StackEntry::Native(ci)) => Ok(Some(Value::Func(vm.cstack[*ci].func))),
        None => {
            let _ = fname;
            Err(vm.arg_error(1, "invalid level"))
        }
    }
}

/// The `__environment` field of `env`'s metatable, read as
/// `luaL_getmetafield` reads it: a protected environment.
fn env_metafield(vm: &mut Vm, env: crate::heap::TableRef) -> Option<crate::taint::Slot> {
    let meta = vm.heap.table(env).meta?;
    let k = vm.heap.intern(b"__environment");
    let s = vm.heap.table(meta).get_str_slot(k);
    (!s.v.is_nil()).then(|| vm.taint_mut().copy(s))
}

fn getfenv(vm: &mut Vm, c: NativeCall) -> Ret {
    // Not inside `__gc` metamethods (patch 10.1).
    if vm.in_finalizer {
        return Ok(vec![]);
    }
    let target = fenv_target(vm, &c, "getfenv")?;
    let (env, f) = match target {
        Some(Value::Func(f)) => match vm.heap.func(f) {
            Function::Native(_) => (vm.globals, Some(f)),
            Function::Lua(l) => (l.env, Some(f)),
        },
        _ => (vm.globals, None),
    };
    // A protected environment answers for itself; otherwise the function's
    // own taint taints the caller.
    if let Some(protected) = env_metafield(vm, env) {
        return Ok(vm.return_slots(vec![protected]));
    }
    if let Some(f) = f {
        let t = vm.heap.func(f).taint();
        vm.taint_mut().read(t);
    }
    Ok(vec![Value::Table(env)])
}

fn setfenv(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 2)?;
    let target = if matches!(c.arg(1), Value::Num(n) if n == 0.0) {
        None
    } else {
        fenv_target(vm, &c, "setfenv")?
    };
    let current = match target {
        Some(Value::Func(f)) => vm.heap.func(f).env(),
        _ => vm.globals,
    };
    if env_metafield(vm, current).is_some() {
        return Err(vm.error("cannot change a protected environment"));
    }
    // Changing a function's environment taints the function.
    if let Some(Value::Func(f)) = target {
        let w = vm.taint().write();
        if !w.is_secure() {
            vm.heap.func_mut(f).set_taint(w);
        }
    }
    match target {
        None => {
            vm.globals = t;
            Ok(vec![])
        }
        Some(Value::Func(f)) if matches!(vm.heap.func(f), Function::Lua(_)) => {
            vm.heap.func_mut(f).set_env(t);
            Ok(vec![Value::Func(f)])
        }
        _ => Err(vm.error("'setfenv' cannot change environment of given object")),
    }
}

fn newproxy(vm: &mut Vm, c: NativeCall) -> Ret {
    let meta = match c.arg(1) {
        Value::Bool(true) => Some(vm.new_table()),
        Value::Nil | Value::Bool(false) => None,
        Value::Userdata(u) => match vm.heap.userdata(u).meta {
            Some(m) => Some(m),
            None => return Err(vm.arg_error(1, "boolean or proxy expected")),
        },
        _ => return Err(vm.arg_error(1, "boolean or proxy expected")),
    };
    let u = vm.heap.alloc_userdata(Userdata {
        serial: 0,
        meta,
        env: None,
        data: Box::new(()),
    });
    Ok(vec![Value::Userdata(u)])
}

fn collectgarbage(vm: &mut Vm, c: NativeCall) -> Ret {
    const OPTS: [&str; 7] = [
        "stop",
        "restart",
        "collect",
        "count",
        "step",
        "setpause",
        "setstepmul",
    ];
    let opt = match vm.opt_str(&c, 1)? {
        Some(s) => String::from_utf8_lossy(vm.heap.str_bytes(s)).into_owned(),
        None => "collect".to_string(),
    };
    let Some(o) = OPTS.iter().position(|&x| x == opt) else {
        return Err(vm.arg_error(1, &format!("invalid option '{opt}'")));
    };
    let ex = vm.opt_int(&c, 2, 0)?;
    Ok(vec![match OPTS[o] {
        "stop" => {
            vm.gc_stopped = true;
            Value::Num(0.0)
        }
        "restart" => {
            vm.gc_stopped = false;
            Value::Num(0.0)
        }
        "collect" => {
            vm.collect()?;
            Value::Num(0.0)
        }
        "count" => Value::Num(vm.heap.estimate_bytes() as f64 / 1024.0),
        "step" => Value::Bool(vm.step_collect(ex)?),
        "setpause" => {
            let old = vm.gc_pause;
            vm.gc_pause = ex.max(0) as usize;
            Value::Num(old as f64)
        }
        _ => {
            let old = vm.gc_stepmul;
            vm.gc_stepmul = ex.max(0) as usize;
            Value::Num(old as f64)
        }
    }])
}

fn gcinfo(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num((vm.heap.estimate_bytes() / 1024) as f64)])
}
