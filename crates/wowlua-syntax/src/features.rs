//! Which wowlua-only syntax a file uses. Pure Lua 5.1 uses none.

use crate::ast::*;
use wowlua_span::Span;

/// A wowlua language feature found in a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Feature {
    /// Type annotations, casts and generic parameters: erased.
    TypeAnnotation,
    /// `type`, `interface`, `declare`, `import type`: erased.
    TypeDeclaration,
    DefaultParameter,
    Destructuring,
    Const,
    CompoundAssignment,
    Continue,
    OptionalChaining,
    NilCoalescing,
    InterpolatedString,
    BitwiseOperator,
    IntegerDivision,
    NumericLiteral,
    Decorator,
    Class,
    Enum,
    Module,
}

impl Feature {
    /// Erased without any lowering (spec roadmap M1).
    pub fn is_type_only(self) -> bool {
        matches!(self, Feature::TypeAnnotation | Feature::TypeDeclaration)
    }

    pub fn describe(self) -> &'static str {
        match self {
            Feature::TypeAnnotation => "type annotations",
            Feature::TypeDeclaration => "type declarations",
            Feature::DefaultParameter => "default parameter values",
            Feature::Destructuring => "destructuring",
            Feature::Const => "`const`",
            Feature::CompoundAssignment => "compound assignment",
            Feature::Continue => "`continue`",
            Feature::OptionalChaining => "optional chaining",
            Feature::NilCoalescing => "`??`",
            Feature::InterpolatedString => "interpolated strings",
            Feature::BitwiseOperator => "bitwise operators",
            Feature::IntegerDivision => "`//`",
            Feature::NumericLiteral => "digit separators and binary literals",
            Feature::Decorator => "decorators",
            Feature::Class => "classes",
            Feature::Enum => "enums",
            Feature::Module => "`import`/`export`",
        }
    }

    /// The roadmap milestone that implements the feature.
    pub fn milestone(self) -> &'static str {
        match self {
            Feature::TypeAnnotation | Feature::TypeDeclaration => "M1",
            Feature::Decorator | Feature::Class | Feature::Enum => "M3",
            _ => "M2",
        }
    }
}

/// Every use of wowlua-only syntax in `chunk`, in source order.
pub fn features(chunk: &Chunk) -> Vec<(Feature, Span)> {
    let mut out = Vec::new();
    block(&chunk.block, &mut out);
    out.sort_by_key(|(_, span)| span.start);
    out
}

type Out = Vec<(Feature, Span)>;

fn block(b: &Block, out: &mut Out) {
    for s in &b.stmts {
        stmt(s, out);
    }
}

fn typed(n: &TypedName, out: &mut Out) {
    if let Some(t) = &n.ty {
        out.push((Feature::TypeAnnotation, t.span));
    }
}

fn func(f: &FuncBody, out: &mut Out) {
    if let Some(p) = f.type_params.first() {
        out.push((Feature::TypeAnnotation, p.name.span));
    }
    for p in &f.params {
        if let Some(t) = &p.ty {
            out.push((Feature::TypeAnnotation, t.span));
        }
        if let Some(d) = &p.default {
            out.push((Feature::DefaultParameter, d.span));
            expr(d, out);
        }
    }
    if let Some(Vararg { ty: Some(_), span }) = &f.vararg {
        out.push((Feature::TypeAnnotation, *span));
    }
    if f.ret.is_some() {
        out.push((Feature::TypeAnnotation, f.params_span));
    }
    block(&f.body, out);
}

fn stmt(s: &Stmt, out: &mut Out) {
    // `@if` is conditional compilation (milestone M2); other decorators M3.
    if let Some(d) = s
        .decorators
        .iter()
        .find(|d| !(d.path.len() == 1 && d.path[0].text.as_ref() == "if"))
    {
        out.push((Feature::Decorator, d.span));
    }
    match &s.kind {
        StmtKind::Assign { targets, values } => {
            targets.iter().chain(values).for_each(|e| expr(e, out));
        }
        StmtKind::Compound { target, value, .. } => {
            out.push((Feature::CompoundAssignment, s.span));
            expr(target, out);
            expr(value, out);
        }
        StmtKind::Call(e) => expr(e, out),
        StmtKind::Do(b) => block(b, out),
        StmtKind::While { cond, body } => {
            expr(cond, out);
            block(body, out);
        }
        StmtKind::Repeat { body, cond } => {
            block(body, out);
            expr(cond, out);
        }
        StmtKind::If {
            cond,
            then_block,
            elseifs,
            else_block,
        } => {
            expr(cond, out);
            block(then_block, out);
            for (c, b) in elseifs {
                expr(c, out);
                block(b, out);
            }
            if let Some(b) = else_block {
                block(b, out);
            }
        }
        StmtKind::NumericFor {
            var,
            start,
            limit,
            step,
            body,
        } => {
            typed(var, out);
            expr(start, out);
            expr(limit, out);
            if let Some(e) = step {
                expr(e, out);
            }
            block(body, out);
        }
        StmtKind::GenericFor { vars, exprs, body } => {
            vars.iter().for_each(|v| typed(v, out));
            exprs.iter().for_each(|e| expr(e, out));
            block(body, out);
        }
        StmtKind::Function { func: f, .. } | StmtKind::LocalFunction { func: f, .. } => {
            func(f, out)
        }
        StmtKind::Local { names, values } => {
            names.iter().for_each(|n| typed(n, out));
            values.iter().for_each(|e| expr(e, out));
        }
        StmtKind::LocalDestructure { fields, value, .. } => {
            out.push((Feature::Destructuring, s.span));
            for f in fields {
                if let Some(d) = &f.default {
                    expr(d, out);
                }
            }
            if let Some(v) = value {
                expr(v, out);
            }
        }
        StmtKind::Const { value, .. } => {
            out.push((Feature::Const, s.span));
            if let Some(v) = value {
                expr(v, out);
            }
        }
        StmtKind::Return(values) => values.iter().for_each(|e| expr(e, out)),
        StmtKind::Break | StmtKind::Error => {}
        StmtKind::Continue => out.push((Feature::Continue, s.span)),
        StmtKind::Class(_) => out.push((Feature::Class, s.span)),
        StmtKind::Enum(_) => out.push((Feature::Enum, s.span)),
        StmtKind::Interface(_) | StmtKind::TypeAlias(_) | StmtKind::Declare(_) => {
            out.push((Feature::TypeDeclaration, s.span))
        }
        StmtKind::Import(import) => {
            if import.type_only {
                out.push((Feature::TypeDeclaration, s.span));
            } else {
                out.push((Feature::Module, s.span));
            }
        }
        StmtKind::Export(inner) => match inner.kind {
            StmtKind::Interface(_) | StmtKind::TypeAlias(_) => {
                out.push((Feature::TypeDeclaration, s.span))
            }
            _ => {
                out.push((Feature::Module, s.span));
                stmt(inner, out);
            }
        },
    }
}

fn args(a: &Args, out: &mut Out) {
    for e in a.exprs() {
        expr(e, out);
    }
}

fn expr(e: &Expr, out: &mut Out) {
    match &e.kind {
        ExprKind::Number(n) => {
            if n.raw.contains('_') || n.raw.to_ascii_lowercase().starts_with("0b") {
                out.push((Feature::NumericLiteral, e.span));
            }
        }
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::String(_)
        | ExprKind::Vararg
        | ExprKind::Name(_)
        | ExprKind::Error => {}
        ExprKind::Function(f) => func(f, out),
        ExprKind::Table(fields) => {
            for field in fields {
                match field {
                    TableField::Keyed { key, value, .. } => {
                        expr(key, out);
                        expr(value, out);
                    }
                    TableField::Named { value, .. } | TableField::Positional(value) => {
                        expr(value, out)
                    }
                }
            }
        }
        ExprKind::Interp(parts) => {
            out.push((Feature::InterpolatedString, e.span));
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    expr(x, out);
                }
            }
        }
        ExprKind::Index { obj, key } => {
            expr(obj, out);
            expr(key, out);
        }
        ExprKind::Field { obj, .. } => expr(obj, out),
        ExprKind::Call { callee, args: a } => {
            expr(callee, out);
            args(a, out);
        }
        ExprKind::Method { obj, args: a, .. } => {
            expr(obj, out);
            args(a, out);
        }
        ExprKind::OptField { obj, .. } => {
            out.push((Feature::OptionalChaining, e.span));
            expr(obj, out);
        }
        ExprKind::OptIndex { obj, key } => {
            out.push((Feature::OptionalChaining, e.span));
            expr(obj, out);
            expr(key, out);
        }
        ExprKind::OptMethod { obj, args: a, .. } => {
            out.push((Feature::OptionalChaining, e.span));
            expr(obj, out);
            args(a, out);
        }
        ExprKind::OptCall { callee, args: a } => {
            out.push((Feature::OptionalChaining, e.span));
            expr(callee, out);
            args(a, out);
        }
        ExprKind::Paren(x) => expr(x, out),
        ExprKind::Binary { op, lhs, rhs } => {
            match op {
                BinOp::Coalesce => out.push((Feature::NilCoalescing, e.span)),
                BinOp::IDiv => out.push((Feature::IntegerDivision, e.span)),
                BinOp::BAnd | BinOp::BOr | BinOp::BXor | BinOp::Shl | BinOp::Shr => {
                    out.push((Feature::BitwiseOperator, e.span))
                }
                _ => {}
            }
            expr(lhs, out);
            expr(rhs, out);
        }
        ExprKind::Unary { op, operand } => {
            if *op == UnOp::BNot {
                out.push((Feature::BitwiseOperator, e.span));
            }
            expr(operand, out);
        }
        ExprKind::Cast { expr: x, ty } => {
            out.push((Feature::TypeAnnotation, ty.span));
            expr(x, out);
        }
    }
}
