//! Operators, modifiers, directives and the core decorators: plain data,
//! shared by the parser, the tree and the views.

use wowlua_span::Span;

/// A directive comment before the first token (`--!strict`, spec §1.1.1).
#[derive(Clone, Debug, PartialEq)]
pub struct Directive {
    pub name: Box<str>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompoundOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Pow,
    Concat,
    BAnd,
    BOr,
    Shl,
    Shr,
    Coalesce,
}

/// Core decorators (spec §4.3): always in scope, they win over lexical
/// bindings of the same name.
pub const CORE_DECORATORS: &[&str] = &[
    "if",
    "global",
    "deprecated",
    "inline",
    "debugOnly",
    "nocheck",
    "script",
    "widget",
    "intrinsic",
    "deferred",
    "onDemand",
    "profile",
    "noprofile",
    "trace",
    "reflect",
    "checked",
    "unchecked",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Coalesce,
    BAnd,
    BOr,
    BXor,
    Shl,
    Shr,
}

impl BinOp {
    /// Left and right binding power (spec §1.4.6; Lua 5.3's table, with
    /// `??` at the level of `or`).
    pub fn priority(self) -> (u8, u8) {
        use BinOp::*;
        match self {
            Or | Coalesce => (1, 1),
            And => (2, 2),
            Eq | Ne | Lt | Le | Gt | Ge => (3, 3),
            BOr => (4, 4),
            BXor => (5, 5),
            BAnd => (6, 6),
            Shl | Shr => (7, 7),
            Concat => (9, 8),
            Add | Sub => (10, 10),
            Mul | Div | IDiv | Mod => (11, 11),
            Pow => (14, 13),
        }
    }

    pub fn as_str(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            IDiv => "//",
            Mod => "%",
            Pow => "^",
            Concat => "..",
            Eq => "==",
            Ne => "~=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            And => "and",
            Or => "or",
            Coalesce => "??",
            BAnd => "&",
            BOr => "|",
            BXor => "~",
            Shl => "<<",
            Shr => ">>",
        }
    }

    /// Exists in Lua 5.1.
    pub fn is_lua(self) -> bool {
        use BinOp::*;
        !matches!(self, IDiv | Coalesce | BAnd | BOr | BXor | Shl | Shr)
    }
}

/// Binding power of unary operators (spec §1.4.6).
pub const UNARY_PRIORITY: u8 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Len,
    BNot,
}

impl UnOp {
    pub fn as_str(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "not",
            UnOp::Len => "#",
            UnOp::BNot => "~",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Public,
    Private,
    Protected,
    Static,
    Readonly,
    Override,
    Abstract,
    Overload,
}
