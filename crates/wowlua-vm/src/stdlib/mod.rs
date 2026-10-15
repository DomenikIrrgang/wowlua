//! The standard libraries, and the argument checks native functions share
//! (Lua's `luaL_check*`, with the same messages).

pub mod base;
pub mod coroutine;
pub mod math;
pub mod pattern;
pub mod secrets;
pub mod secure;
pub mod string;
pub mod table;
pub mod wow;

use crate::error::LuaError;
use crate::function::{NativeCall, NativeFn};
use crate::heap::{StrRef, TableRef};
use crate::number::fmt_num;
use crate::value::Value;
use crate::vm::Vm;

pub type Ret = Result<Vec<Value>, LuaError>;

pub fn open(vm: &mut Vm) {
    base::open(vm);
    string::open(vm);
    table::open(vm);
    math::open(vm);
    coroutine::open(vm);
}

impl NativeCall {
    /// Argument `n` (1-based); nil when absent.
    pub fn arg(&self, n: usize) -> Value {
        self.args.get(n - 1).copied().unwrap_or_default()
    }

    /// Argument `n` (1-based) with its taint.
    pub fn slot(&self, n: usize) -> crate::taint::Slot {
        crate::taint::Slot {
            v: self.arg(n),
            t: self.taints.get(n - 1).copied().unwrap_or_default(),
        }
    }

    /// Upvalue `i` (0-based) with its taint.
    pub fn upval_slot(&self, i: usize) -> crate::taint::Slot {
        crate::taint::Slot {
            v: self.upvals.get(i).copied().unwrap_or_default(),
            t: self.upval_taints.get(i).copied().unwrap_or_default(),
        }
    }

    /// The arguments from `n` (1-based) on, with their taints.
    pub fn slots(&self, n: usize) -> Vec<crate::taint::Slot> {
        (n..=self.args.len()).map(|i| self.slot(i)).collect()
    }

    pub fn has(&self, n: usize) -> bool {
        n <= self.args.len()
    }

    pub fn len(&self) -> usize {
        self.args.len()
    }

    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }
}

/// Registers `funcs` in a new table, set as global `name` (unless empty).
pub fn register(vm: &mut Vm, name: &str, funcs: &[(&'static str, NativeFn)]) -> TableRef {
    let t = vm.new_table();
    for (fname, f) in funcs {
        let fv = vm.native(fname, *f);
        let k = vm.heap.intern(fname.as_bytes());
        vm.heap.table_mut(t).set_str(k, fv);
    }
    if !name.is_empty() {
        vm.set_global(name, Value::Table(t));
    }
    t
}

impl Vm {
    pub fn check_any(&mut self, c: &NativeCall, n: usize) -> Result<Value, LuaError> {
        if !c.has(n) {
            return Err(self.arg_error(n, "value expected"));
        }
        Ok(c.arg(n))
    }

    pub fn check_table(&mut self, c: &NativeCall, n: usize) -> Result<TableRef, LuaError> {
        match c.arg(n) {
            Value::Table(t) => Ok(t),
            _ => Err(self.type_arg_error(n, "table", c.args.get(n - 1).copied())),
        }
    }

    /// Argument `n` as a conversion takes it: a secret's value where it may
    /// be used (`kind`: "string" or "numeric" names the refused conversion).
    pub fn plain_arg(&mut self, c: &NativeCall, n: usize, kind: &str) -> Result<Value, LuaError> {
        match c.arg(n) {
            v @ Value::Secret(_) => {
                // A value no conversion takes is a type error ("got secret").
                let h = self.hidden(v);
                let converts = matches!(h, Value::Num(_) | Value::Str(_) | Value::Nil);
                if !converts && !self.can_access_secrets() && !self.secret_reveal {
                    let expected = if kind == "numeric" {
                        "number"
                    } else {
                        "string"
                    };
                    return Err(self.type_arg_error(n, expected, Some(v)));
                }
                self.secret_arg(v, kind)
            }
            v => Ok(v),
        }
    }

    /// Whether argument `n` is absent or nil (a secret nil too).
    pub fn arg_is_nil(&self, c: &NativeCall, n: usize) -> bool {
        self.hidden(c.arg(n)).is_nil()
    }

    pub fn check_number(&mut self, c: &NativeCall, n: usize) -> Result<f64, LuaError> {
        let v = self.plain_arg(c, n, "numeric")?;
        match self.tonumber(v) {
            Some(x) => Ok(x),
            None => Err(self.type_arg_error(n, "number", c.args.get(n - 1).map(|_| v))),
        }
    }

    pub fn opt_number(&mut self, c: &NativeCall, n: usize, default: f64) -> Result<f64, LuaError> {
        if self.arg_is_nil(c, n) {
            Ok(default)
        } else {
            self.check_number(c, n)
        }
    }

    /// `luaL_checkinteger`: a number, truncated toward zero.
    pub fn check_int(&mut self, c: &NativeCall, n: usize) -> Result<i64, LuaError> {
        let x = self.check_number(c, n)?;
        self.check_overflow(x)?;
        Ok(to_int(x))
    }

    pub fn opt_int(&mut self, c: &NativeCall, n: usize, default: i64) -> Result<i64, LuaError> {
        if self.arg_is_nil(c, n) {
            Ok(default)
        } else {
            self.check_int(c, n)
        }
    }

    /// `luaL_checklstring`: a string, or a number converted to one.
    pub fn check_str(&mut self, c: &NativeCall, n: usize) -> Result<StrRef, LuaError> {
        match self.plain_arg(c, n, "string")? {
            Value::Str(s) => Ok(s),
            Value::Num(x) => Ok(self.heap.intern(fmt_num(x).as_bytes())),
            v => Err(self.type_arg_error(n, "string", c.args.get(n - 1).map(|_| v))),
        }
    }

    pub fn check_bytes(&mut self, c: &NativeCall, n: usize) -> Result<std::rc::Rc<[u8]>, LuaError> {
        let s = self.check_str(c, n)?;
        Ok(self.heap.str_rc(s))
    }

    pub fn opt_str(&mut self, c: &NativeCall, n: usize) -> Result<Option<StrRef>, LuaError> {
        if self.arg_is_nil(c, n) {
            Ok(None)
        } else {
            self.check_str(c, n).map(Some)
        }
    }

    pub fn check_func(&mut self, c: &NativeCall, n: usize) -> Result<Value, LuaError> {
        match c.arg(n) {
            v @ Value::Func(_) => Ok(v),
            _ => Err(self.type_arg_error(n, "function", c.args.get(n - 1).copied())),
        }
    }
}

/// `lua_Number` to `lua_Integer` as the client converts it (truncation).
pub fn to_int(x: f64) -> i64 {
    if x.is_nan() { i64::MIN } else { x as i64 }
}
