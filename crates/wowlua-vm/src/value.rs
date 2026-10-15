//! Lua values. A value is small and `Copy`; collectable objects are handles
//! into the [`Heap`](crate::heap::Heap).

use crate::heap::{FuncRef, StrRef, TableRef, ThreadRef, UdRef};

#[derive(Clone, Copy, Debug, Default)]
pub enum Value {
    #[default]
    Nil,
    Bool(bool),
    Num(f64),
    Str(StrRef),
    Table(TableRef),
    Func(FuncRef),
    Thread(ThreadRef),
    Userdata(UdRef),
}

impl Value {
    /// The name `type()` returns.
    pub fn type_name(self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Table(_) => "table",
            Value::Func(_) => "function",
            Value::Thread(_) => "thread",
            Value::Userdata(_) => "userdata",
        }
    }

    /// Lua truthiness: everything but `nil` and `false`.
    pub fn truthy(self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn is_nil(self) -> bool {
        matches!(self, Value::Nil)
    }

    /// Primitive equality (`rawequal`): numbers by value, objects by
    /// identity, strings by content (interned).
    pub fn raw_eq(self, other: Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Table(a), Value::Table(b)) => a == b,
            (Value::Func(a), Value::Func(b)) => a == b,
            (Value::Thread(a), Value::Thread(b)) => a == b,
            (Value::Userdata(a), Value::Userdata(b)) => a == b,
            _ => false,
        }
    }
}
