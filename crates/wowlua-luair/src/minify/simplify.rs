//! `simplify` (spec optimization §15.4.1): redundant parentheses, `not (a ==
//! b)` → `a ~= b`, and consecutive `local` declarations merged. `;` never
//! reaches the IR: the emitter writes only the ones Lua needs.

use crate::ir::*;
use crate::scope::{Visitor, walk};
use std::collections::BTreeSet;

pub fn apply(chunk: &mut Chunk, preserve_lines: bool) {
    if preserve_lines {
        // Lua gives an instruction the line of the last token read, so
        // parentheses decide the lines of multi-line expressions: preserve
        // mode rewrites only statements on one line.
        blocks(&mut chunk.block, &mut |b| {
            for s in &mut b.stmts {
                if single_line(s).is_some() {
                    in_chunk(s, |c| walk(c, &mut Exprs));
                }
            }
        });
    } else {
        walk(chunk, &mut Exprs);
    }
    blocks(&mut chunk.block, &mut |b| merge_locals(b, preserve_lines));
}

struct Exprs;

impl Visitor for Exprs {
    fn expr(&mut self, e: &mut Expr) {
        loop {
            match &mut e.kind {
                // Parentheses truncate calls and `...` to one value; any
                // others the emitter adds back where priorities need them.
                ExprKind::Paren { inner, .. }
                    if !matches!(
                        inner.kind,
                        ExprKind::Call { .. } | ExprKind::Method { .. } | ExprKind::Vararg
                    ) =>
                {
                    let inner =
                        std::mem::replace(inner.as_mut(), Expr::new(ExprKind::Nil, Pos::NONE));
                    let sep = e.sep;
                    *e = inner;
                    e.sep = sep;
                }
                // Lua 5.1 defines `a ~= b` as `not (a == b)`.
                ExprKind::Unary {
                    op: UnOp::Not,
                    operand,
                    ..
                } => {
                    let target = match &mut operand.kind {
                        ExprKind::Paren { inner, .. } => inner.as_mut(),
                        _ => operand.as_mut(),
                    };
                    let ExprKind::Binary { op, .. } = &mut target.kind else {
                        break;
                    };
                    let flipped = match op {
                        BinOp::Eq => BinOp::Ne,
                        BinOp::Ne => BinOp::Eq,
                        _ => break,
                    };
                    *op = flipped;
                    let mut binary = std::mem::replace(target, Expr::new(ExprKind::Nil, Pos::NONE));
                    binary.sep = e.sep;
                    if !binary.pos.is_some() {
                        binary.pos = e.pos;
                    }
                    *e = binary;
                }
                _ => break,
            }
        }
    }
}

/// Runs `f` on a chunk holding only statement `s`.
fn in_chunk(s: &mut Stmt, f: impl FnOnce(&mut Chunk)) {
    let placeholder = Stmt {
        pos: Pos::NONE,
        kind: StmtKind::Break,
    };
    let mut chunk = Chunk {
        block: Block {
            stmts: vec![std::mem::replace(s, placeholder)],
            end: Pos::NONE,
        },
        comments: Vec::new(),
    };
    f(&mut chunk);
    *s = chunk.block.stmts.pop().expect("the statement");
}

/// Calls `f` on every block, innermost first.
fn blocks(b: &mut Block, f: &mut impl FnMut(&mut Block)) {
    for s in &mut b.stmts {
        stmt_blocks(s, f);
    }
    f(b);
}

fn stmt_blocks(s: &mut Stmt, f: &mut impl FnMut(&mut Block)) {
    match &mut s.kind {
        StmtKind::Local { values, .. } | StmtKind::Return(values) => {
            values.iter_mut().for_each(|e| expr_blocks(e, f));
        }
        StmtKind::Assign { targets, values } => {
            targets
                .iter_mut()
                .chain(values)
                .for_each(|e| expr_blocks(e, f));
        }
        StmtKind::Call(e) => expr_blocks(e, f),
        StmtKind::Do(b) => blocks(b, f),
        StmtKind::While { cond, body, .. } | StmtKind::Repeat { body, cond } => {
            expr_blocks(cond, f);
            blocks(body, f);
        }
        StmtKind::If {
            clauses,
            else_block,
        } => {
            for c in clauses {
                expr_blocks(&mut c.cond, f);
                blocks(&mut c.body, f);
            }
            if let Some(b) = else_block {
                blocks(b, f);
            }
        }
        StmtKind::NumericFor {
            start,
            limit,
            step,
            body,
            ..
        } => {
            expr_blocks(start, f);
            expr_blocks(limit, f);
            if let Some(s) = step {
                expr_blocks(s, f);
            }
            blocks(body, f);
        }
        StmtKind::GenericFor { exprs, body, .. } => {
            exprs.iter_mut().for_each(|e| expr_blocks(e, f));
            blocks(body, f);
        }
        StmtKind::Function { func, .. } | StmtKind::LocalFunction { func, .. } => {
            blocks(&mut func.body, f);
        }
        StmtKind::Break => {}
    }
}

fn expr_blocks(e: &mut Expr, f: &mut impl FnMut(&mut Block)) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || match &mut e.kind {
        ExprKind::Function(func) => blocks(&mut func.body, f),
        ExprKind::Table { fields, .. } => {
            for field in fields {
                match field {
                    Field::Keyed { key, value, .. } => {
                        expr_blocks(key, f);
                        expr_blocks(value, f);
                    }
                    Field::Named { value, .. } | Field::Positional(value) => expr_blocks(value, f),
                }
            }
        }
        ExprKind::Index { obj, key, .. } => {
            expr_blocks(obj, f);
            expr_blocks(key, f);
        }
        ExprKind::Field { obj, .. } => expr_blocks(obj, f),
        ExprKind::Call { callee: obj, args } | ExprKind::Method { obj, args, .. } => {
            expr_blocks(obj, f);
            match args {
                Args::Parens { args, .. } => args.iter_mut().for_each(|e| expr_blocks(e, f)),
                Args::Table(e) | Args::String(e) => expr_blocks(e, f),
            }
        }
        ExprKind::Paren { inner, .. } => expr_blocks(inner, f),
        ExprKind::Binary { lhs, rhs, .. } => {
            expr_blocks(lhs, f);
            expr_blocks(rhs, f);
        }
        ExprKind::Unary { operand, .. } => expr_blocks(operand, f),
        _ => {}
    })
}

/// Names mentioned anywhere in `e`, nested functions included.
fn mentions(e: &Expr, names: &BTreeSet<&str>) -> bool {
    let mut e = e.clone();
    struct Find<'a> {
        names: &'a BTreeSet<&'a str>,
        found: bool,
    }
    impl Visitor for Find<'_> {
        fn ident(&mut self, id: &mut Ident, _use: crate::scope::Use) {
            self.found |= self.names.contains(id.name.as_str());
        }
    }
    let mut find = Find {
        names,
        found: false,
    };
    let mut chunk = Chunk {
        block: Block {
            stmts: vec![Stmt {
                pos: Pos::NONE,
                kind: StmtKind::Return(vec![std::mem::replace(
                    &mut e,
                    Expr::new(ExprKind::Nil, Pos::NONE),
                )]),
            }],
            end: Pos::NONE,
        },
        comments: Vec::new(),
    };
    walk(&mut chunk, &mut find);
    find.found
}

/// Evaluating `e` cannot raise an error, so the line it is reported on does
/// not matter.
fn cannot_fail(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Vararg
        | ExprKind::Function(_)
        | ExprKind::Name(_) => true,
        ExprKind::Table { fields, .. } => fields.iter().all(|f| match f {
            Field::Named { value, .. } | Field::Positional(value) => cannot_fail(value),
            Field::Keyed { .. } => false,
        }),
        ExprKind::Paren { inner, .. } => cannot_fail(inner),
        _ => false,
    }
}

/// The line all of a statement's tokens lie on (0 for generated code), or
/// `None` when it spans lines.
fn single_line(s: &mut Stmt) -> Option<u32> {
    struct Lines(BTreeSet<u32>);
    impl Visitor for Lines {
        fn ident(&mut self, id: &mut Ident, _use: crate::scope::Use) {
            if id.pos.is_some() {
                self.0.insert(id.pos.line);
            }
        }
        fn expr(&mut self, e: &mut Expr) {
            for pos in [e.pos, e.sep] {
                if pos.is_some() {
                    self.0.insert(pos.line);
                }
            }
            match &e.kind {
                ExprKind::Function(f) => {
                    self.0.insert(f.body.end.line);
                }
                ExprKind::Paren { close, .. }
                | ExprKind::Table { close, .. }
                | ExprKind::Call {
                    args: Args::Parens { close, .. },
                    ..
                }
                | ExprKind::Method {
                    args: Args::Parens { close, .. },
                    ..
                }
                | ExprKind::Binary { op_pos: close, .. } => {
                    self.0.insert(close.line);
                }
                _ => {}
            }
        }
    }
    let mut lines = Lines(BTreeSet::new());
    if s.pos.is_some() {
        lines.0.insert(s.pos.line);
    }
    in_chunk(s, |c| {
        blocks(&mut c.block, &mut |b| {
            if b.end.is_some() {
                lines.0.insert(b.end.line);
            }
        });
        walk(c, &mut lines);
    });
    lines.0.remove(&0);
    match lines.0.len() {
        0 => Some(0),
        1 => lines.0.first().copied(),
        _ => None,
    }
}

/// `local a = x` and `local b = y` become `local a, b = x, y`.
fn merge_locals(b: &mut Block, preserve_lines: bool) {
    let mut out: Vec<Stmt> = Vec::with_capacity(b.stmts.len());
    for mut s in std::mem::take(&mut b.stmts) {
        if let Some(prev) = out.last_mut()
            && can_merge(prev, &mut s, preserve_lines)
        {
            let StmtKind::Local { names, values } = s.kind else {
                unreachable!()
            };
            let StmtKind::Local {
                names: pn,
                values: pv,
            } = &mut prev.kind
            else {
                unreachable!()
            };
            pn.extend(names);
            pv.extend(values);
            continue;
        }
        out.push(s);
    }
    b.stmts = out;
}

fn can_merge(prev: &mut Stmt, next: &mut Stmt, preserve_lines: bool) -> bool {
    let (
        StmtKind::Local {
            names: pn,
            values: pv,
        },
        StmtKind::Local {
            names: nn,
            values: nv,
        },
    ) = (&prev.kind, &next.kind)
    else {
        return false;
    };
    // Each value must stay with its name: `local a, b = f()` would take
    // `b` from the call's second value.
    let shape = |n: &[Ident], v: &[Expr]| v.is_empty() || v.len() == n.len();
    if !shape(pn, pv) || !shape(nn, nv) || pv.is_empty() != nv.is_empty() {
        return false;
    }
    let declared: BTreeSet<&str> = pn.iter().map(|i| i.name.as_str()).collect();
    if nn.iter().any(|i| declared.contains(i.name.as_str()))
        || nv.iter().any(|v| mentions(v, &declared))
    {
        return false;
    }
    // Errors keep their line: the merged statement is on the first one's.
    let failing = !nv.iter().all(cannot_fail);
    if preserve_lines || failing {
        matches!((single_line(prev), single_line(next)), (Some(a), Some(b)) if a == b)
    } else {
        true
    }
}
