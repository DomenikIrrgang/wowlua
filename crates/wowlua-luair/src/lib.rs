//! The Lua 5.1 IR and the back end: emitter, line modes, hygiene and source
//! maps (spec compiler §13.7, code generation §14.3–§14.6).

pub mod bytecode;
pub mod emit;
pub mod hygiene;
pub mod ir;
pub mod sourcemap;

pub use emit::{CommentMode, EmitOptions, Emitted, LineMode, emit};
