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

fn print(vm: &mut Vm, c: NativeCall) -> Ret {
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

fn rawget(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let k = vm.check_any(&c, 2)?;
    Ok(vec![vm.heap.table(t).get(k)])
}

fn rawset(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    let k = vm.check_any(&c, 2)?;
    let v = vm.check_any(&c, 3)?;
    if let Err(e) = vm.heap.table_mut(t).set(k, v) {
        return Err(vm.rt_error(e.message()));
    }
    Ok(vec![Value::Table(t)])
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
        return Ok(c.args);
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
        let mut text = vm.where_(level as usize).into_bytes();
        vm.append_strlike(&mut text, v);
        let msg = vm.bytes(&text);
        return Err(vm.raise(msg));
    }
    Err(vm.raise(v))
}

fn pcall(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = vm.check_any(&c, 1)?;
    match vm.pcall(f, &c.args[1..]) {
        Ok(mut results) => {
            results.insert(0, Value::Bool(true));
            Ok(results)
        }
        Err(e) => Ok(vec![Value::Bool(false), e.value]),
    }
}

fn xpcall(vm: &mut Vm, c: NativeCall) -> Ret {
    let handler = vm.check_any(&c, 2)?;
    let f = c.arg(1);
    let saved = vm.errfunc.replace(handler);
    // Lua 5.1 calls `f` without arguments (WoW passes the rest).
    let r = vm.call(f, &[]);
    vm.errfunc = saved;
    match r {
        Ok(mut results) => {
            results.insert(0, Value::Bool(true));
            Ok(results)
        }
        Err(e) => Ok(vec![Value::Bool(false), e.value]),
    }
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
    Ok(c.args[i as usize..].to_vec())
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
    let table = vm.heap.table(t);
    Ok((i..=e).map(|k| table.get_num(k as f64)).collect())
}

fn next(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    match vm.heap.table(t).next(c.arg(2)) {
        Ok(Some((k, v))) => Ok(vec![k, v]),
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
    let v = vm.heap.table(t).get_num(i as f64);
    if v.is_nil() {
        Ok(vec![Value::Nil])
    } else {
        Ok(vec![Value::Num(i as f64), v])
    }
}

fn load_result(vm: &mut Vm, r: Result<Value, String>) -> Ret {
    match r {
        Ok(f) => Ok(vec![f]),
        Err(msg) => {
            let m = vm.str(&msg);
            Ok(vec![Value::Nil, m])
        }
    }
}

fn loadstring(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_bytes(&c, 1)?;
    let name = match vm.opt_str(&c, 2)? {
        Some(n) => String::from_utf8_lossy(vm.heap.str_bytes(n)).into_owned(),
        None => String::from_utf8_lossy(&s).into_owned(),
    };
    let r = vm.load(&s, &name);
    load_result(vm, r)
}

fn load(vm: &mut Vm, c: NativeCall) -> Ret {
    let f = vm.check_func(&c, 1)?;
    let name = match vm.opt_str(&c, 2)? {
        Some(n) => String::from_utf8_lossy(vm.heap.str_bytes(n)).into_owned(),
        None => "=(load)".to_string(),
    };
    let mut src = Vec::new();
    loop {
        let r = vm.call(f, &[])?;
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
    }
    let r = vm.load(&src, &name);
    load_result(vm, r)
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

fn getfenv(vm: &mut Vm, c: NativeCall) -> Ret {
    let target = fenv_target(vm, &c, "getfenv")?;
    let env = match target {
        None => vm.globals,
        Some(Value::Func(f)) => match vm.heap.func(f) {
            Function::Native(_) => vm.globals,
            Function::Lua(l) => l.env,
        },
        Some(_) => vm.globals,
    };
    Ok(vec![Value::Table(env)])
}

fn setfenv(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 2)?;
    if let Value::Num(n) = c.arg(1)
        && n == 0.0
    {
        vm.globals = t;
        return Ok(vec![]);
    }
    let target = fenv_target(vm, &c, "setfenv")?;
    match target {
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
            vm.collect();
            Value::Num(0.0)
        }
        "count" => Value::Num(vm.heap.estimate_bytes() as f64 / 1024.0),
        "step" => {
            vm.collect();
            Value::Bool(true)
        }
        "setpause" => {
            let old = vm.gc_pause;
            vm.gc_pause = ex.max(0) as usize;
            Value::Num(old as f64)
        }
        _ => Value::Num(200.0),
    }])
}

fn gcinfo(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Num((vm.heap.estimate_bytes() / 1024) as f64)])
}
