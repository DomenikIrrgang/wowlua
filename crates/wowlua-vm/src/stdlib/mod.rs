//! The standard libraries, and the argument checks native functions share
//! (Lua's `luaL_check*`, with the same messages).

pub mod base;
pub mod pattern;
pub mod string;

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
}

impl NativeCall {
    /// Argument `n` (1-based); nil when absent.
    pub fn arg(&self, n: usize) -> Value {
        self.args.get(n - 1).copied().unwrap_or_default()
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

    pub fn check_number(&mut self, c: &NativeCall, n: usize) -> Result<f64, LuaError> {
        match self.tonumber(c.arg(n)) {
            Some(x) => Ok(x),
            None => Err(self.type_arg_error(n, "number", c.args.get(n - 1).copied())),
        }
    }

    pub fn opt_number(&mut self, c: &NativeCall, n: usize, default: f64) -> Result<f64, LuaError> {
        if c.arg(n).is_nil() {
            Ok(default)
        } else {
            self.check_number(c, n)
        }
    }

    /// `luaL_checkinteger`: a number, truncated toward zero.
    pub fn check_int(&mut self, c: &NativeCall, n: usize) -> Result<i64, LuaError> {
        Ok(to_int(self.check_number(c, n)?))
    }

    pub fn opt_int(&mut self, c: &NativeCall, n: usize, default: i64) -> Result<i64, LuaError> {
        if c.arg(n).is_nil() {
            Ok(default)
        } else {
            self.check_int(c, n)
        }
    }

    /// `luaL_checklstring`: a string, or a number converted to one.
    pub fn check_str(&mut self, c: &NativeCall, n: usize) -> Result<StrRef, LuaError> {
        match c.arg(n) {
            Value::Str(s) => Ok(s),
            Value::Num(x) => Ok(self.heap.intern(fmt_num(x).as_bytes())),
            _ => Err(self.type_arg_error(n, "string", c.args.get(n - 1).copied())),
        }
    }

    pub fn check_bytes(&mut self, c: &NativeCall, n: usize) -> Result<std::rc::Rc<[u8]>, LuaError> {
        let s = self.check_str(c, n)?;
        Ok(self.heap.str_rc(s))
    }

    pub fn opt_str(&mut self, c: &NativeCall, n: usize) -> Result<Option<StrRef>, LuaError> {
        if c.arg(n).is_nil() {
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
