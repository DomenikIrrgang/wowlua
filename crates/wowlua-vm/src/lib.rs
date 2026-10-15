//! wowlua's Lua 5.1 virtual machine (spec chapter 17, decisions T185 and
//! T190 to T192): register bytecode compiled from the syntax tree, a heap
//! of handles with a tracing collector, and stackless coroutines. Test
//! builds run on it; WoW's own Lua semantics are layered on top.

pub mod compiler;
pub mod error;
pub mod function;
pub mod gc;
pub mod heap;
pub mod luac;
pub mod number;
pub mod proto;
pub mod stdlib;
pub mod table;
pub mod taint;
pub mod thread;
pub mod value;
pub mod vm;

pub use error::LuaError;
pub use function::{NativeCall, NativeFn};
pub use heap::{FuncRef, StrRef, TableRef};
pub use taint::{Slot, Taint};
pub use value::Value;
pub use vm::Vm;
