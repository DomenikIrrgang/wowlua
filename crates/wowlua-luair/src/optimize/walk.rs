//! Traversals the passes of `optimize` share: statements' expressions and
//! blocks, expressions' parts, in Lua's evaluation order where it matters.

use crate::ir::*;

/// The expressions of statement `s` itself, in evaluation order (not those
/// of its blocks): a `for`'s header, a `while`'s condition, an `if`'s
/// first condition (the others are evaluated only when the earlier fail).
pub(crate) fn own_exprs(s: &mut Stmt) -> Vec<&mut Expr> {
    match &mut s.kind {
        StmtKind::Local { values, .. } => values.iter_mut().collect(),
        StmtKind::Assign { targets, values } => {
            targets.iter_mut().chain(values.iter_mut()).collect()
        }
        StmtKind::Call(e) => vec![e],
        StmtKind::While { cond, .. } => vec![cond],
        StmtKind::Repeat { cond, .. } => vec![cond],
        StmtKind::If { clauses, .. } => clauses.iter_mut().map(|c| &mut c.cond).collect(),
        StmtKind::NumericFor {
            start, limit, step, ..
        } => {
            let mut v: Vec<&mut Expr> = vec![start.as_mut(), limit.as_mut()];
            if let Some(s) = step {
                v.push(s.as_mut());
            }
            v
        }
        StmtKind::GenericFor { exprs, .. } => exprs.iter_mut().collect(),
        StmtKind::Return(values) => values.iter_mut().collect(),
        StmtKind::Do(_)
        | StmtKind::Function { .. }
        | StmtKind::LocalFunction { .. }
        | StmtKind::Break => Vec::new(),
    }
}

/// The blocks of statement `s` that run in the same function (not the
/// bodies of the functions it declares).
pub(crate) fn inner_blocks(s: &mut Stmt) -> Vec<&mut Block> {
    match &mut s.kind {
        StmtKind::Do(b)
        | StmtKind::While { body: b, .. }
        | StmtKind::Repeat { body: b, .. }
        | StmtKind::NumericFor { body: b, .. }
        | StmtKind::GenericFor { body: b, .. } => vec![b],
        StmtKind::If {
            clauses,
            else_block,
        } => clauses
            .iter_mut()
            .map(|c| &mut c.body)
            .chain(else_block.iter_mut())
            .collect(),
        _ => Vec::new(),
    }
}

/// The function a statement declares.
pub(crate) fn declared_function(s: &mut Stmt) -> Option<&mut Func> {
    match &mut s.kind {
        StmtKind::Function { func, .. } | StmtKind::LocalFunction { func, .. } => Some(func),
        _ => None,
    }
}

pub(crate) fn is_loop(s: &Stmt) -> bool {
    matches!(
        s.kind,
        StmtKind::While { .. }
            | StmtKind::Repeat { .. }
            | StmtKind::NumericFor { .. }
            | StmtKind::GenericFor { .. }
    )
}

/// The direct parts of an expression, in evaluation order.
pub(crate) fn parts(e: &mut Expr) -> Vec<&mut Expr> {
    match &mut e.kind {
        ExprKind::Index { obj, key, .. } => vec![obj.as_mut(), key.as_mut()],
        ExprKind::Field { obj, .. } => vec![obj.as_mut()],
        ExprKind::Call { callee, args } => {
            let mut v = vec![callee.as_mut()];
            v.extend(args_mut(args));
            v
        }
        ExprKind::Method { obj, args, .. } => {
            let mut v = vec![obj.as_mut()];
            v.extend(args_mut(args));
            v
        }
        ExprKind::Paren { inner, .. } => vec![inner.as_mut()],
        ExprKind::Binary { lhs, rhs, .. } => vec![lhs.as_mut(), rhs.as_mut()],
        ExprKind::Unary { operand, .. } => vec![operand.as_mut()],
        ExprKind::Table { fields, .. } => {
            let mut v = Vec::new();
            for f in fields {
                match f {
                    Field::Keyed { key, value, .. } => {
                        v.push(key.as_mut());
                        v.push(value);
                    }
                    Field::Named { value, .. } | Field::Positional(value) => v.push(value),
                }
            }
            v
        }
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Vararg
        | ExprKind::Name(_)
        | ExprKind::Function(_) => Vec::new(),
    }
}

pub(crate) fn args_mut(a: &mut Args) -> Vec<&mut Expr> {
    match a {
        Args::Parens { args, .. } => args.iter_mut().collect(),
        Args::Table(e) | Args::String(e) => vec![e.as_mut()],
    }
}

/// Calls `f` on `e` and every expression in it, outer first, including the
/// parts of function expressions' bodies when `functions` is set.
pub(crate) fn each_expr(e: &mut Expr, functions: bool, f: &mut impl FnMut(&mut Expr)) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
        f(e);
        if let ExprKind::Function(func) = &mut e.kind {
            if functions {
                each_expr_in_block(&mut func.body, true, f);
            }
            return;
        }
        for p in parts(e) {
            each_expr(p, functions, f);
        }
    })
}

/// [`each_expr`] for every expression of a block, its nested blocks, and
/// (with `functions`) the functions declared in it.
pub(crate) fn each_expr_in_block(b: &mut Block, functions: bool, f: &mut impl FnMut(&mut Expr)) {
    for s in &mut b.stmts {
        each_expr_in_stmt(s, functions, f);
    }
}

pub(crate) fn each_expr_in_stmt(s: &mut Stmt, functions: bool, f: &mut impl FnMut(&mut Expr)) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
        if let StmtKind::Repeat { body, cond } = &mut s.kind {
            each_expr_in_block(body, functions, f);
            each_expr(cond, functions, f);
            return;
        }
        for e in own_exprs(s) {
            each_expr(e, functions, f);
        }
        for b in inner_blocks(s) {
            each_expr_in_block(b, functions, f);
        }
        if functions && let Some(func) = declared_function(s) {
            each_expr_in_block(&mut func.body, true, f);
        }
    })
}

/// Every identifier of a block that is a variable (declared, read or
/// written), with nested functions.
pub(crate) fn each_name_in_block(b: &mut Block, f: &mut impl FnMut(&mut Ident)) {
    let mut chunk = Chunk {
        block: std::mem::take(b),
        comments: Vec::new(),
    };
    struct Names<'a, F>(&'a mut F);
    impl<F: FnMut(&mut Ident)> crate::scope::Visitor for Names<'_, F> {
        fn ident(&mut self, id: &mut Ident, _use: crate::scope::Use) {
            (self.0)(id);
        }
    }
    crate::scope::walk(&mut chunk, &mut Names(f));
    *b = chunk.block;
}

/// [`each_name_in_block`] for one expression.
pub(crate) fn each_name_in_expr(e: &mut Expr, f: &mut impl FnMut(&mut Ident)) {
    struct Names<'a, F>(&'a mut F);
    impl<F: FnMut(&mut Ident)> crate::scope::Visitor for Names<'_, F> {
        fn ident(&mut self, id: &mut Ident, _use: crate::scope::Use) {
            (self.0)(id);
        }
    }
    crate::scope::walk_expr(e, &mut Names(f));
}

/// A literal: `nil`, a boolean, a number, a string, or a negated number.
pub(crate) fn is_literal(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Number(_)
        | ExprKind::String(_) => true,
        ExprKind::Unary {
            op: UnOp::Neg,
            operand,
            ..
        } => matches!(operand.kind, ExprKind::Number(_)),
        _ => false,
    }
}

pub(crate) fn name_expr(id: Ident) -> Expr {
    let pos = id.pos;
    Expr::new(ExprKind::Name(id), pos)
}

pub(crate) fn placeholder() -> Stmt {
    Stmt {
        pos: Pos::NONE,
        kind: StmtKind::Break,
    }
}

/// Calls `f` on `b` and then on every block in it, the bodies of the
/// functions it declares or creates included.
pub(crate) fn each_block(b: &mut Block, f: &mut impl FnMut(&mut Block)) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
        f(b);
        for s in &mut b.stmts {
            for e in own_exprs(s) {
                each_function(e, &mut |func| each_block(&mut func.body, f));
            }
            for inner in inner_blocks(s) {
                each_block(inner, f);
            }
            if let Some(func) = declared_function(s) {
                each_block(&mut func.body, f);
            }
        }
    })
}

/// Calls `f` on the function expressions in `e` that are not inside
/// another one.
pub(crate) fn each_function(e: &mut Expr, f: &mut impl FnMut(&mut Func)) {
    each_expr(e, false, &mut |e| {
        if let ExprKind::Function(func) = &mut e.kind {
            f(func);
        }
    });
}

/// The most locals active at once in a function's own code (its nested
/// functions have their own), parameters included.
pub(crate) fn max_locals(func: &Func, method: bool) -> usize {
    fn block(b: &Block, active: usize) -> usize {
        let mut now = active;
        let mut most = active;
        for s in &b.stmts {
            match &s.kind {
                StmtKind::Local { names, .. } => now += names.len(),
                StmtKind::LocalFunction { .. } => now += 1,
                StmtKind::NumericFor { body, .. } => {
                    // (Lua keeps three hidden locals of its own)
                    most = most.max(block(body, now + 4));
                }
                StmtKind::GenericFor { vars, body, .. } => {
                    most = most.max(block(body, now + 3 + vars.len()));
                }
                StmtKind::Do(b)
                | StmtKind::While { body: b, .. }
                | StmtKind::Repeat { body: b, .. } => {
                    most = most.max(block(b, now));
                }
                StmtKind::If {
                    clauses,
                    else_block,
                } => {
                    for c in clauses {
                        most = most.max(block(&c.body, now));
                    }
                    if let Some(b) = else_block {
                        most = most.max(block(b, now));
                    }
                }
                _ => {}
            }
            most = most.max(now);
        }
        most
    }
    block(
        &func.body,
        func.params.len() + usize::from(method) + usize::from(func.vararg),
    )
}
