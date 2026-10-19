//! The client's secure execution API (spec emulator §17.4), as elune
//! implements it (`lbaselib.c`, `lauxlib.c`): `issecure`,
//! `issecurevariable`, `securecall`, `securecallfunction`,
//! `securecallmethod`, `secureexecuterange`, `hooksecurefunc`,
//! `forceinsecure`, `scrub`.

use crate::error::LuaError;
use crate::function::NativeCall;
use crate::stdlib::Ret;
use crate::taint::{Slot, Taint, TaintState};
use crate::value::Value;
use crate::vm::Vm;

/// The taint `forceinsecure` sets (`LUA_FORCEINSECURE_TAINT`).
pub const FORCED: &str = "*** TaintForced ***";

pub fn open(vm: &mut Vm) {
    for (name, f) in [
        ("issecure", issecure as crate::NativeFn),
        ("forceinsecure", forceinsecure),
        ("issecurevariable", issecurevariable),
        ("securecall", securecall),
        ("securecallfunction", securecallfunction),
        ("securecallmethod", securecallmethod),
        ("secureexecuterange", secureexecuterange),
        ("hooksecurefunc", hooksecurefunc),
        ("scrub", scrub),
    ] {
        let v = vm.native(name, f);
        vm.set_global(name, v);
    }
}

fn issecure(vm: &mut Vm, _c: NativeCall) -> Ret {
    Ok(vec![Value::Bool(vm.taint().stack.is_secure())])
}

fn forceinsecure(vm: &mut Vm, _c: NativeCall) -> Ret {
    if vm.taint().stack.is_secure() {
        let forced = vm.taint_named(FORCED);
        vm.taint_mut().stack = forced;
    }
    Ok(vec![])
}

/// What reading `t[k]` does to a secure execution (`luaL_gettabletaint`):
/// the field's taint, or one an `__index` metamethod brings.
fn table_taint(vm: &mut Vm, t: Value, k: Value) -> Result<Taint, LuaError> {
    let saved = vm.taint().saved();
    vm.taint_mut().stack = Taint::SECURE;
    let r = vm.index_slot(t, k, None);
    let r = r.map(|s| {
        vm.taint_mut().read(s.t);
        vm.taint().stack
    });
    vm.taint_mut().restore(saved);
    r
}

fn issecurevariable(vm: &mut Vm, c: NativeCall) -> Ret {
    let (t, k) = if matches!(c.arg(1), Value::Str(_) | Value::Num(_)) {
        (Value::Table(vm.globals), c.arg(1))
    } else {
        (c.arg(1), c.arg(2))
    };
    if !matches!(t, Value::Table(_)) || !matches!(k, Value::Str(_) | Value::Num(_)) {
        return Err(vm.error("Usage: issecurevariable([table,] \"variable\")"));
    }
    // `lua_tostring` turns a number key into a string.
    let k = match k {
        Value::Num(n) => vm.str(&crate::number::fmt_num(n)),
        k => k,
    };
    let taint = table_taint(vm, t, k)?;
    Ok(match vm.taint_name(taint) {
        None => vec![Value::Bool(true), Value::Nil],
        Some(name) => {
            let name = vm.str(&name);
            vec![Value::Bool(false), name]
        }
    })
}

/// `pcall` through the error handler, then the saved taint back; the
/// results take it (`luaL_pcallas` with the caller's state).
fn call_restoring(vm: &mut Vm, saved: TaintState, f: Slot, args: &[Slot]) -> Vec<Slot> {
    let handler = vm.error_handler_proxy();
    let outer = vm.errfunc.replace(handler);
    let r = vm.call_slots(f, args);
    vm.errfunc = outer;
    vm.taint_mut().restore(saved);
    let t = vm.taint().stack;
    r.map(|results| {
        results
            .into_iter()
            .map(|s| Slot { v: s.v, t, o: s.o })
            .collect()
    })
    .unwrap_or_default()
}

fn securecall(vm: &mut Vm, c: NativeCall) -> Ret {
    let saved = vm.taint().saved();
    let mut f = c.slot(1);
    if let Value::Str(_) | Value::Num(_) = f.v {
        // A name: the global, read as `lua_rawget` reads it, in the
        // caller's globals (a host may run code with other globals).
        let key = match f.v {
            Value::Num(n) => vm.str(&crate::number::fmt_num(n)),
            k => k,
        };
        let g = vm.caller_env(1);
        let s = vm.heap.table(g).get_slot(key);
        f = vm.taint_mut().copy(s);
    }
    let results = call_restoring(vm, saved, f, &c.slots(2));
    Ok(vm.return_slots(results))
}

fn securecallfunction(vm: &mut Vm, c: NativeCall) -> Ret {
    let saved = vm.taint().saved();
    let results = call_restoring(vm, saved, c.slot(1), &c.slots(2));
    Ok(vm.return_slots(results))
}

/// `securecallmethod(object, name, ...)`: `object:name(...)` as
/// `securecallfunction` calls (the client has it; Blizzard's secure pools
/// release through it). The method is read as an index reads it.
/// Unverified: a missing method, the taint of the read.
fn securecallmethod(vm: &mut Vm, c: NativeCall) -> Ret {
    let saved = vm.taint().saved();
    let object = c.slot(1);
    let method = vm.index_slot(object.v, c.arg(2), None)?;
    let mut args = vec![object];
    args.extend(c.slots(3));
    let results = call_restoring(vm, saved, method, &args);
    Ok(vm.return_slots(results))
}

/// Calls `f(key, value, ...)` for each entry; errors are dropped, and the
/// taint is restored after each call (`luaL_secureforeach`). Recorded
/// (Forever 1.60.1, `forever-camelot.lua`): a callback's error reaches no
/// error handler (`secureexecuterange({ 1 }, function() error('in range')
/// end)` showed no handler call, where `securecall`'s did) and the next
/// entries still run (`n` counted 2 of 2 failing calls); warcraft.wiki.gg's
/// API_secureexecuterange says so too ("errors ... are discarded"), and
/// Blizzard's CallbackRegistry wraps each callback in `securecallfunction`
/// for its errors to be reported. The host still hears of each one
/// (`Vm::on_dropped_error`).
fn secureexecuterange(vm: &mut Vm, c: NativeCall) -> Ret {
    let t = vm.check_table(&c, 1)?;
    if !matches!(c.arg(2), Value::Func(_)) {
        return Err(vm.type_arg_error(2, "function", c.args.get(1).copied()));
    }
    let saved = vm.taint().saved();
    let f = c.slot(2);
    let extra = c.slots(3);
    let dropped = vm
        .on_dropped_error
        .map(|_| vm.native("dropped", dropped_error));
    let mut key = Value::Nil;
    loop {
        let next = match vm.heap.table(t).next_slot(key) {
            Ok(n) => n,
            Err(_) => return Err(vm.error("invalid key to 'next'")),
        };
        let Some((k, v)) = next else { break };
        let fk = vm.taint().fresh(k);
        let mut args = vec![vm.taint_mut().copy(fk), vm.taint_mut().copy(v)];
        args.extend(extra.iter().copied());
        let f = vm.taint_mut().copy(f);
        let outer = std::mem::replace(&mut vm.errfunc, dropped);
        let _ = vm.call_slots(f, &args);
        vm.errfunc = outer;
        vm.taint_mut().restore(saved);
        key = k;
    }
    Ok(vec![])
}

/// The handler of a call whose error the client drops: tells the host
/// (`Vm::on_dropped_error`) the error and where it was raised, while the
/// stack still shows it.
fn dropped_error(vm: &mut Vm, c: NativeCall) -> Ret {
    let message = match c.arg(1) {
        v @ (Value::Str(_) | Value::Num(_)) => {
            let mut b = Vec::new();
            vm.append_strlike(&mut b, v);
            String::from_utf8_lossy(&b).into_owned()
        }
        _ => "UNKNOWN ERROR".to_string(),
    };
    let stack = vm.traceback(1);
    if let Some(hook) = vm.on_dropped_error {
        hook(vm, message, stack);
    }
    Ok(vec![c.arg(1)])
}

/// `hooksecurefunc([t,] name, hook)`: replaces `t[name]` with a secure
/// wrapper that calls the original, then `hook` with the same arguments,
/// insecure and protected, without tainting the caller.
fn hooksecurefunc(vm: &mut Vm, c: NativeCall) -> Ret {
    let (t, name, hook) = match c.arg(1) {
        Value::Table(_) => (c.arg(1), c.arg(2), c.slot(3)),
        _ => (Value::Table(vm.globals), c.arg(1), c.slot(2)),
    };
    if !matches!(name, Value::Str(_) | Value::Num(_)) || !matches!(hook.v, Value::Func(_)) {
        return Err(vm.error("Usage: hooksecurefunc([table,] \"function\", hookfunc)"));
    }
    let entry = vm.taint().saved();
    // The original, looked up from a secure execution.
    vm.taint_mut().stack = Taint::SECURE;
    let original = vm.index_slot(t, name, None);
    let original = original.map(|s| vm.taint_mut().copy(s));
    let lookup = vm.taint().stack;
    vm.taint_mut().restore(entry);
    let original = original?;
    if !matches!(original.v, Value::Func(_)) {
        let mut text = Vec::new();
        vm.append_strlike(&mut text, name);
        let msg = format!(
            "hooksecurefunc(): {} is not a function",
            String::from_utf8_lossy(&text)
        );
        return Err(vm.error(&msg));
    }
    // The original keeps the lookup's taint, the hook the caller's.
    let original = Slot {
        v: original.v,
        t: lookup,
        o: original.o,
    };
    let hook = Slot {
        v: hook.v,
        t: entry.stack,
        o: hook.o,
    };
    let wrapper = vm.native_slots("securehook", securehook, vec![original, hook]);
    let Value::Table(tr) = t else { unreachable!() };
    let _ = vm.heap.table_mut(tr).set_slot(name, Slot::secure(wrapper));
    Ok(vec![])
}

/// The wrapper `hooksecurefunc` installs (elune's `f_securehook`).
fn securehook(vm: &mut Vm, c: NativeCall) -> Ret {
    let [original, hook] = [c.upval_slot(0), c.upval_slot(1)];
    let args: Vec<Slot> = c.slots(1).iter().map(|&a| vm.taint_mut().copy(a)).collect();
    let original = vm.taint_mut().copy(original);
    let results = vm.call_slots(original, &args)?;
    // An untrusted hook does not see the secrets passed to an object whose
    // untrusted layout scripts are forbidden: the user's client (build
    // 70450) shows no error from ElvUI's `SetSize` hook on its aura
    // containers (`DisableUntrustedLayoutScriptsTemplate`) when Blizzard's
    // flow layout sizes them with `secretwrap`ped values, where the hook's
    // `mover:SetSize(width, height)` would refuse them. Unverified (one
    // sample, inferred from the aspect's documentation, "Restricts
    // execution of layout scripts"): the hook is not called at all.
    if let Some(forbidden) = vm.untrusted_layout_forbidden
        && !(hook.t.is_secure() && function_taint(vm, hook.v).is_secure())
        && c.args.iter().any(|a| matches!(a, Value::Secret(_)))
        && forbidden(vm, c.arg(1))
    {
        return Ok(vm.return_slots(results));
    }
    // The hook: protected, through the error handler, its taint undone.
    let saved = vm.taint().saved();
    let args: Vec<Slot> = c.slots(1).iter().map(|&a| vm.taint_mut().copy(a)).collect();
    let hook = vm.taint_mut().copy(hook);
    let handler = vm.error_handler_proxy();
    let outer = vm.errfunc.replace(handler);
    let _ = vm.call_slots(hook, &args);
    vm.errfunc = outer;
    vm.taint_mut().restore(saved);
    Ok(vm.return_slots(results))
}

/// A function's own taint (secure for anything else).
fn function_taint(vm: &Vm, v: Value) -> Taint {
    match v {
        Value::Func(f) => vm.heap.func(f).taint(),
        _ => Taint::SECURE,
    }
}

/// Keeps numbers, strings and booleans; anything else becomes nil.
fn scrub(vm: &mut Vm, c: NativeCall) -> Ret {
    let out: Vec<Slot> = c
        .slots(1)
        .into_iter()
        .map(|s| match s.v {
            Value::Num(_) | Value::Str(_) | Value::Bool(_) => s,
            _ => vm.taint().fresh(Value::Nil),
        })
        .collect();
    Ok(vm.return_slots(out))
}

impl Vm {
    /// Calls `f` as the client's C code calls a script or a callback: from a
    /// secure execution, errors going to the error handler
    /// (`geterrorhandler()`), the taint state restored afterwards. The
    /// results, or `None` after an error.
    pub fn call_from_engine(&mut self, f: Value, args: &[Value]) -> Option<Vec<Value>> {
        let saved = self.taint().saved();
        self.taint_mut().stack = Taint::SECURE;
        let handler = self.error_handler_proxy();
        let outer = self.errfunc.replace(handler);
        let args: Vec<Slot> = args.iter().map(|&v| Slot::secure(v)).collect();
        let r = self.call_slots(Slot::secure(f), &args);
        self.errfunc = outer;
        self.taint_mut().restore(saved);
        r.ok()
            .map(|results| results.into_iter().map(|s| s.v).collect())
    }
}

impl Vm {
    /// Calls `f` as its own secure execution (the client loading a file:
    /// each one a call from C), the caller's taint state restored after,
    /// an error returned: what one file did cannot taint the next.
    pub fn call_secure(&mut self, f: Value, args: &[Value]) -> Result<Vec<Value>, LuaError> {
        let saved = self.taint().saved();
        self.taint_mut().stack = Taint::SECURE;
        let args: Vec<Slot> = args.iter().map(|&v| Slot::secure(v)).collect();
        let r = self.call_slots(Slot::secure(f), &args);
        self.taint_mut().restore(saved);
        r.map(|results| results.into_iter().map(|s| s.v).collect())
    }
}

impl Vm {
    /// The function errors reach the error handler through (elune's
    /// `f_errorhandler`): non-string errors become "UNKNOWN ERROR", and a
    /// `*:` source names the frame of the function that failed.
    pub fn error_handler_proxy(&mut self) -> Value {
        self.native("errorhandler", error_handler_proxy)
    }
}

fn error_handler_proxy(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut err = match c.arg(1) {
        v @ (Value::Str(_) | Value::Num(_)) => {
            let mut b = Vec::new();
            vm.append_strlike(&mut b, v);
            b
        }
        _ => b"UNKNOWN ERROR".to_vec(),
    };
    if let Some(at) = err.windows(2).position(|w| w == b"*:")
        && let Some(name) = error_object_name(vm)
    {
        let mut named = err[..at].to_vec();
        named.extend_from_slice(&name);
        named.extend_from_slice(&err[at + 1..]);
        err = named;
    }
    let msg = vm.bytes(&err);
    let handler = vm.error_handler;
    let r = vm.call(handler, &[msg])?;
    Ok(r.into_iter().take(1).collect())
}

/// The name of the frame whose script failed (`f_errorobjname`): the
/// function one level up comes from a `*` source, and its first local is a
/// widget table (`[0]` a userdata with `__name`).
fn error_object_name(vm: &mut Vm) -> Option<Vec<u8>> {
    let levels = vm.levels();
    let level = levels.get(1)?;
    if level.source.first() != Some(&b'*') {
        return None;
    }
    let (locals, _) = vm.level_variables(level);
    let Value::Table(t) = locals.first()?.1 else {
        return None;
    };
    let object = vm.heap.table(t).get_num(0.0);
    let Value::Userdata(_) = object else {
        return None;
    };
    let k = vm.heap.intern(b"__name");
    let tm = vm.get_tm(object, k);
    if tm.is_nil() {
        return None;
    }
    let name = vm.call(tm, &[object]).ok()?;
    match name.first() {
        Some(&v @ (Value::Str(_) | Value::Num(_))) => {
            let mut b = Vec::new();
            vm.append_strlike(&mut b, v);
            Some(b)
        }
        _ => Some(b"<unnamed>".to_vec()),
    }
}
