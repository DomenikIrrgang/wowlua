//! The Lua 5.1 IR and the back end: emitter, line modes, hygiene and source
//! maps (spec compiler §13.7, code generation §14.3–§14.6), and the
//! minification passes (spec optimization §15.4).

pub mod bytecode;
pub mod deep;
pub mod emit;
pub mod hygiene;
pub mod ir;
pub mod minify;
pub mod optimize;
pub mod scope;
pub mod sourcemap;

pub use emit::{CommentMode, EmitOptions, Emitted, LineMode, emit};
pub use minify::Passes;
