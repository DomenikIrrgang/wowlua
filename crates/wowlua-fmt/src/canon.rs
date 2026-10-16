//! A structural fingerprint of a syntax tree: equal fingerprints mean the
//! same program (spec tooling §16.3: formatting never changes semantics).
//! Spans, comments, quote styles and the call-argument form (`f"x"` vs
//! `f("x")`) do not matter; parentheses and literal values do.

use std::fmt::Write;
use wowlua_syntax::view::*;

pub fn fingerprint(chunk: Chunk<'_>) -> String {
    let mut out = String::new();
    block(chunk.block(), &mut out);
    out
}

fn block(b: Block<'_>, out: &mut String) {
    out.push('{');
    for s in &b.stmts() {
        stmt(s, out);
        out.push(';');
    }
    out.push('}');
}

fn list(es: Exprs<'_>, out: &mut String) {
    out.push('[');
    for e in es {
        expr(e, out);
        out.push(',');
    }
    out.push(']');
}

fn func(f: Func<'_>, out: &mut String) {
    out.push_str("fn(");
    for p in &f.params() {
        let _ = write!(out, "{},", p.name().text());
    }
    if f.vararg().is_some() {
        out.push_str("...");
    }
    out.push(')');
    block(f.body(), out);
}

fn stmt(s: Stmt<'_>, out: &mut String) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || stmt_inner(s, out))
}

fn stmt_inner(s: Stmt<'_>, out: &mut String) {
    match s.kind() {
        StmtKind::Assign { targets, values } => {
            out.push_str("set");
            list(targets, out);
            list(values, out);
        }
        StmtKind::Call(e) => {
            out.push_str("call");
            expr(e, out);
        }
        StmtKind::Do(b) => {
            out.push_str("do");
            block(b, out);
        }
        StmtKind::While { cond, body } => {
            out.push_str("while");
            expr(cond, out);
            block(body, out);
        }
        StmtKind::Repeat { body, cond } => {
            out.push_str("repeat");
            block(body, out);
            expr(cond, out);
        }
        StmtKind::If {
            cond,
            then_block,
            elseifs,
            else_block,
        } => {
            out.push_str("if");
            expr(cond, out);
            block(then_block, out);
            for (c, b) in elseifs {
                out.push_str("elseif");
                expr(c, out);
                block(b, out);
            }
            if let Some(b) = else_block {
                out.push_str("else");
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
            let _ = write!(out, "for {}=", var.name.text());
            expr(start, out);
            expr(limit, out);
            if let Some(s) = step {
                expr(s, out);
            }
            block(body, out);
        }
        StmtKind::GenericFor { vars, exprs, body } => {
            out.push_str("forin ");
            for v in vars {
                let _ = write!(out, "{},", v.name.text());
            }
            list(exprs, out);
            block(body, out);
        }
        StmtKind::Function { name, func: f } => {
            out.push_str("function ");
            for n in &name.path() {
                let _ = write!(out, "{}.", n.text());
            }
            if let Some(m) = &name.method() {
                let _ = write!(out, ":{}", m.text());
            }
            func(f, out);
        }
        StmtKind::LocalFunction { name, func: f } => {
            let _ = write!(out, "localfn {}", name.text());
            func(f, out);
        }
        StmtKind::Local { names, values } => {
            out.push_str("local ");
            for n in names {
                let _ = write!(out, "{},", n.name.text());
            }
            list(values, out);
        }
        StmtKind::Return(values) => {
            out.push_str("return");
            list(values, out);
        }
        StmtKind::Break => out.push_str("break"),
        other => {
            let _ = write!(out, "{other:?}");
        }
    }
}

fn args(a: Args<'_>, out: &mut String) {
    out.push('(');
    for e in a.exprs() {
        expr(e, out);
        out.push(',');
    }
    out.push(')');
}

fn expr(e: Expr<'_>, out: &mut String) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || expr_inner(e, out))
}

fn expr_inner(e: Expr<'_>, out: &mut String) {
    match e.kind() {
        ExprKind::Nil => out.push_str("nil"),
        ExprKind::True => out.push_str("true"),
        ExprKind::False => out.push_str("false"),
        ExprKind::Number(n) => {
            let _ = write!(out, "#{}", n.raw());
        }
        ExprKind::String(s) => {
            let _ = write!(out, "{:?}", s.value());
        }
        ExprKind::Vararg => out.push_str("..."),
        ExprKind::Function(f) => func(f, out),
        ExprKind::Table(fields) => {
            out.push_str("tbl{");
            for f in fields {
                match f {
                    TableField::Keyed { key, value, .. } => {
                        out.push('[');
                        expr(key, out);
                        out.push_str("]=");
                        expr(value, out);
                    }
                    TableField::Named { name, value, .. } => {
                        let _ = write!(out, "{}=", name.text());
                        expr(value, out);
                    }
                    TableField::Positional(v) => expr(v, out),
                }
                out.push(',');
            }
            out.push('}');
        }
        ExprKind::Name(n) => out.push_str(n.text()),
        ExprKind::Index { obj, key } => {
            expr(obj, out);
            out.push('[');
            expr(key, out);
            out.push(']');
        }
        ExprKind::Field { obj, name } => {
            expr(obj, out);
            let _ = write!(out, ".{}", name.text());
        }
        ExprKind::Call { callee, args: a } => {
            expr(callee, out);
            args(a, out);
        }
        ExprKind::Method { obj, name, args: a } => {
            expr(obj, out);
            let _ = write!(out, ":{}", name.text());
            args(a, out);
        }
        ExprKind::Paren(inner) => {
            out.push('(');
            expr(inner, out);
            out.push(')');
        }
        ExprKind::Binary { op, lhs, rhs } => {
            out.push('<');
            expr(lhs, out);
            out.push_str(op.as_str());
            expr(rhs, out);
            out.push('>');
        }
        ExprKind::Unary { op, operand } => {
            let _ = write!(out, "<{}", op.as_str());
            expr(operand, out);
            out.push('>');
        }
        other => {
            let _ = write!(out, "{other:?}");
        }
    }
}
