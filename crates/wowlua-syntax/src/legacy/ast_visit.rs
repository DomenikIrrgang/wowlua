//! The parser's walk over the boxed tree it builds (consumers walk the
//! flat tree through [`crate::visit`]).

use super::ast::*;

/// Calls `f` on `e` and every expression inside it, including those in
/// nested function bodies.
pub(crate) fn walk_expr<'a>(e: &'a Expr, f: &mut impl FnMut(&'a Expr)) {
    crate::deep::run(|| walk(e, f, true))
}

fn walk<'a>(e: &'a Expr, f: &mut impl FnMut(&'a Expr), deep: bool) {
    let walk_expr = |x: &'a Expr, f: &mut _| walk(x, f, deep);
    f(e);
    match &e.kind {
        ExprKind::Function(body) => {
            if deep {
                walk_block(&body.body, f)
            }
        }
        ExprKind::Table(fields) => {
            for field in fields {
                match field {
                    TableField::Keyed { key, value, .. } => {
                        walk_expr(key, f);
                        walk_expr(value, f);
                    }
                    TableField::Named { value, .. } => walk_expr(value, f),
                    TableField::Positional(value) => walk_expr(value, f),
                }
            }
        }
        ExprKind::Interp(parts) => {
            for part in parts {
                if let InterpPart::Expr(x) = part {
                    walk_expr(x, f);
                }
            }
        }
        ExprKind::Index { obj, key } | ExprKind::OptIndex { obj, key } => {
            walk_expr(obj, f);
            walk_expr(key, f);
        }
        ExprKind::Field { obj, .. } | ExprKind::OptField { obj, .. } => walk_expr(obj, f),
        ExprKind::Call { callee, args } | ExprKind::OptCall { callee, args } => {
            walk_expr(callee, f);
            walk_args(args, f, deep);
        }
        ExprKind::Method { obj, args, .. } | ExprKind::OptMethod { obj, args, .. } => {
            walk_expr(obj, f);
            walk_args(args, f, deep);
        }
        ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => walk_expr(x, f),
        ExprKind::Cast { expr, .. } => walk_expr(expr, f),
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, f);
            walk_expr(rhs, f);
        }
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Vararg
        | ExprKind::Name(_)
        | ExprKind::Error => {}
    }
}

fn walk_args<'a>(args: &'a Args, f: &mut impl FnMut(&'a Expr), deep: bool) {
    match args {
        Args::Parens { args, .. } => args.iter().for_each(|a| walk(a, f, deep)),
        Args::Table(e) | Args::String(e) => walk(e, f, deep),
    }
}

/// Calls `f` on every expression in `block`.
fn walk_block<'a>(block: &'a Block, f: &mut impl FnMut(&'a Expr)) {
    for stmt in &block.stmts {
        walk_stmt(stmt, f);
    }
}

fn walk_stmt<'a>(stmt: &'a Stmt, f: &mut impl FnMut(&'a Expr)) {
    match &stmt.kind {
        StmtKind::Assign { targets, values } => {
            targets.iter().chain(values).for_each(|e| walk_expr(e, f));
        }
        StmtKind::Compound { target, value, .. } => {
            walk_expr(target, f);
            walk_expr(value, f);
        }
        StmtKind::Call(e) => walk_expr(e, f),
        StmtKind::Do(b) => walk_block(b, f),
        StmtKind::While { cond, body } => {
            walk_expr(cond, f);
            walk_block(body, f);
        }
        StmtKind::Repeat { body, cond } => {
            walk_block(body, f);
            walk_expr(cond, f);
        }
        StmtKind::If {
            cond,
            then_block,
            elseifs,
            else_block,
        } => {
            walk_expr(cond, f);
            walk_block(then_block, f);
            for (c, b) in elseifs {
                walk_expr(c, f);
                walk_block(b, f);
            }
            if let Some(b) = else_block {
                walk_block(b, f);
            }
        }
        StmtKind::NumericFor {
            start,
            limit,
            step,
            body,
            ..
        } => {
            walk_expr(start, f);
            walk_expr(limit, f);
            if let Some(s) = step {
                walk_expr(s, f);
            }
            walk_block(body, f);
        }
        StmtKind::GenericFor { exprs, body, .. } => {
            exprs.iter().for_each(|e| walk_expr(e, f));
            walk_block(body, f);
        }
        StmtKind::Function { func, .. } | StmtKind::LocalFunction { func, .. } => {
            walk_block(&func.body, f)
        }
        StmtKind::Local { values, .. } => values.iter().for_each(|e| walk_expr(e, f)),
        StmtKind::LocalDestructure { fields, value, .. } => {
            for field in fields {
                if let Some(d) = &field.default {
                    walk_expr(d, f);
                }
            }
            if let Some(v) = value {
                walk_expr(v, f);
            }
        }
        StmtKind::Const { value, .. } => {
            if let Some(v) = value {
                walk_expr(v, f);
            }
        }
        StmtKind::Return(values) => values.iter().for_each(|e| walk_expr(e, f)),
        StmtKind::Export(inner) => walk_stmt(inner, f),
        StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Class(_)
        | StmtKind::Interface(_)
        | StmtKind::Enum(_)
        | StmtKind::TypeAlias(_)
        | StmtKind::Declare(_)
        | StmtKind::Import(_)
        | StmtKind::Error => {}
    }
}
