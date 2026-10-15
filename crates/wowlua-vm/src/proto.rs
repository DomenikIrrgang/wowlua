//! The VM's register bytecode (decision T190), close to Lua 5.1's: the same
//! operations and register discipline, as a Rust enum. Each instruction has a
//! source line and byte span in [`Proto`].

use std::rc::Rc;

use crate::value::Value;

/// A register or a constant operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rk {
    R(u8),
    K(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

impl ArithOp {
    pub fn event(self) -> &'static str {
        match self {
            ArithOp::Add => "__add",
            ArithOp::Sub => "__sub",
            ArithOp::Mul => "__mul",
            ArithOp::Div => "__div",
            ArithOp::Mod => "__mod",
            ArithOp::Pow => "__pow",
        }
    }
}

/// `B` and `C` counts follow Lua: `n + 1`, with 0 meaning "up to the top".
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Move {
        a: u8,
        b: u8,
    },
    LoadK {
        a: u8,
        k: u32,
    },
    LoadBool {
        a: u8,
        value: bool,
        skip: bool,
    },
    /// Registers `a..=b` to nil.
    LoadNil {
        a: u8,
        b: u8,
    },
    GetUpval {
        a: u8,
        u: u8,
    },
    GetGlobal {
        a: u8,
        k: u32,
    },
    GetTable {
        a: u8,
        b: u8,
        c: Rk,
    },
    SetGlobal {
        a: u8,
        k: u32,
    },
    SetUpval {
        a: u8,
        u: u8,
    },
    SetTable {
        a: u8,
        b: Rk,
        c: Rk,
    },
    NewTable {
        a: u8,
        narr: u32,
        nhash: u32,
    },
    /// `R(a+1) = R(b); R(a) = R(b)[c]`
    SelfOp {
        a: u8,
        b: u8,
        c: Rk,
    },
    Arith {
        op: ArithOp,
        a: u8,
        b: Rk,
        c: Rk,
    },
    Unm {
        a: u8,
        b: u8,
    },
    Not {
        a: u8,
        b: u8,
    },
    Len {
        a: u8,
        b: u8,
    },
    /// `R(a) = R(b) .. … .. R(c)`
    Concat {
        a: u8,
        b: u8,
        c: u8,
    },
    Jmp {
        sbx: i32,
    },
    /// Skip the next instruction unless `(R(b) == R(c)) == a`.
    Eq {
        a: bool,
        b: Rk,
        c: Rk,
    },
    Lt {
        a: bool,
        b: Rk,
        c: Rk,
    },
    Le {
        a: bool,
        b: Rk,
        c: Rk,
    },
    /// Skip the next instruction unless `truthy(R(a)) == c`.
    Test {
        a: u8,
        c: bool,
    },
    /// `if truthy(R(b)) == c then R(a) = R(b) else skip`
    TestSet {
        a: u8,
        b: u8,
        c: bool,
    },
    Call {
        a: u8,
        b: u8,
        c: u8,
    },
    TailCall {
        a: u8,
        b: u8,
    },
    Return {
        a: u8,
        b: u8,
    },
    ForLoop {
        a: u8,
        sbx: i32,
    },
    ForPrep {
        a: u8,
        sbx: i32,
    },
    TForLoop {
        a: u8,
        c: u8,
    },
    /// `c` is the batch number (1-based); `b` the count, 0 for "to the top".
    SetList {
        a: u8,
        b: u8,
        c: u32,
    },
    Close {
        a: u8,
    },
    Closure {
        a: u8,
        proto: u32,
    },
    Vararg {
        a: u8,
        b: u8,
    },
}

impl Op {
    /// A test that the following jump depends on (Lua's `testTMode`).
    pub fn is_test(&self) -> bool {
        matches!(
            self,
            Op::Eq { .. } | Op::Lt { .. } | Op::Le { .. } | Op::Test { .. } | Op::TestSet { .. }
        )
    }
}

/// Where a closure's upvalue comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpvalDesc {
    /// A local (register) of the enclosing function.
    Local(u8),
    /// An upvalue of the enclosing function.
    Upval(u8),
}

/// A local variable's name and the instructions it is active in.
#[derive(Clone, Debug)]
pub struct LocVar {
    pub name: Rc<str>,
    pub start_pc: usize,
    pub end_pc: usize,
}

/// A compiled function.
#[derive(Debug, Default)]
pub struct Proto {
    pub code: Vec<Op>,
    /// The source line of each instruction.
    pub lines: Vec<u32>,
    pub constants: Vec<Value>,
    pub protos: Vec<Rc<Proto>>,
    pub upvals: Vec<UpvalDesc>,
    pub upval_names: Vec<Rc<str>>,
    pub locvars: Vec<LocVar>,
    pub num_params: u8,
    pub is_vararg: bool,
    pub max_stack: u8,
    /// The chunk name (`@file`, `=name` or the source).
    pub source: Rc<str>,
    pub line_defined: u32,
    pub last_line_defined: u32,
}

impl Proto {
    /// The name of the local in register `reg` at `pc`, if one is active.
    pub fn local_name(&self, reg: u8, pc: usize) -> Option<&str> {
        let mut n = reg as usize;
        for v in &self.locvars {
            if v.start_pc > pc {
                break;
            }
            if pc < v.end_pc {
                if n == 0 {
                    return Some(&v.name);
                }
                n -= 1;
            }
        }
        None
    }
}
