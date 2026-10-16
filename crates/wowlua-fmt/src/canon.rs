//! A structural fingerprint of a syntax tree: equal fingerprints mean the
//! same program (spec tooling §16.3: formatting never changes semantics).
//! Spans, comments, quote styles and the call-argument form (`f"x"` vs
//! `f("x")`) do not matter; parentheses and literal values do.

use std::fmt::Write;
use wowlua_syntax::ast::*;

pub fn fingerprint(chunk: &Chunk) -> String {
    let mut out = String::new();
    block(&chunk.block, &mut out);
    out
}

fn block(b: &Block, out: &mut String) {
    out.push('{');
    for s in &b.stmts {
        stmt(s, out);
        out.push(';');
    }
    out.push('}');
}

fn list(es: &[Expr], out: &mut String) {
    out.push('[');
    for e in es {
        expr(e, out);
        out.push(',');
    }
    out.push(']');
}

/// A type, decorator or other part without spans, node IDs or the raw
/// text of literals (quote styles): its structure and values.
fn plain<T: std::fmt::Debug>(v: &T) -> String {
    let text = format!("{v:?}");
    let re = regex_lite();
    re.iter().fold(text, |t, (pattern, with)| {
        pattern.replace_all(&t, *with).into_owned()
    })
}

fn regex_lite() -> &'static [(regex::Regex, &'static str)] {
    static PATTERNS: std::sync::OnceLock<Vec<(regex::Regex, &'static str)>> =
        std::sync::OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            (r"\w*span: Span \{ [^}]* \},? ?", ""),
            (r"id: NodeId\(\d+\),? ?", ""),
            (r"raw: \[[^\]]*\],? ?", ""),
        ]
        .iter()
        .map(|(p, w)| (regex::Regex::new(p).expect("a valid pattern"), *w))
        .collect()
    })
}

fn decorators(ds: &[Decorator], out: &mut String) {
    for d in ds {
        out.push('@');
        for n in &d.path {
            let _ = write!(out, "{}.", n.text);
        }
        if let Some(a) = &d.args {
            args(a, out);
        }
    }
}

fn ty(t: &Type, out: &mut String) {
    out.push_str(&plain(t));
}

fn func(f: &FuncBody, out: &mut String) {
    out.push_str("fn");
    out.push_str(&plain(&f.type_params));
    out.push('(');
    for p in &f.params {
        decorators(&p.decorators, out);
        let _ = write!(out, "{}", p.name.text);
        if let Some(t) = &p.ty {
            ty(t, out);
        }
        if let Some(d) = &p.default {
            out.push('=');
            expr(d, out);
        }
        out.push(',');
    }
    if let Some(v) = &f.vararg {
        out.push_str("...");
        out.push_str(&plain(&v.ty));
    }
    out.push(')');
    out.push_str(&plain(&f.ret));
    block(&f.body, out);
}

fn stmt(s: &Stmt, out: &mut String) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || stmt_inner(s, out))
}

fn stmt_inner(s: &Stmt, out: &mut String) {
    decorators(&s.decorators, out);
    match &s.kind {
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
            let _ = write!(out, "for {}{}=", var.name.text, plain(&var.ty));
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
                let _ = write!(out, "{}{},", v.name.text, plain(&v.ty));
            }
            list(exprs, out);
            block(body, out);
        }
        StmtKind::Function { name, func: f } => {
            out.push_str("function ");
            for n in &name.path {
                let _ = write!(out, "{}.", n.text);
            }
            if let Some(m) = &name.method {
                let _ = write!(out, ":{}", m.text);
            }
            func(f, out);
        }
        StmtKind::LocalFunction { name, func: f } => {
            let _ = write!(out, "localfn {}", name.text);
            func(f, out);
        }
        StmtKind::Local { names, values } => {
            out.push_str("local ");
            for n in names {
                let _ = write!(out, "{}{},", n.name.text, plain(&n.ty));
            }
            list(values, out);
        }
        StmtKind::Compound { target, op, value } => {
            let _ = write!(out, "{op:?}");
            expr(target, out);
            expr(value, out);
        }
        StmtKind::Continue => out.push_str("continue"),
        StmtKind::Const { name, ty: t, value } => {
            let _ = write!(out, "const {}{}", name.text, plain(t));
            if let Some(v) = value {
                expr(v, out);
            }
        }
        StmtKind::LocalDestructure {
            fields,
            ty: t,
            value,
        } => {
            out.push_str("destructure");
            for f in fields {
                let _ = write!(out, "{}", f.name.text);
                if let Some(d) = &f.default {
                    expr(d, out);
                }
                out.push(',');
            }
            out.push_str(&plain(t));
            if let Some(v) = value {
                expr(v, out);
            }
        }
        StmtKind::Export(inner) => {
            out.push_str("export");
            stmt(inner, out);
        }
        StmtKind::Class(c) => {
            let _ = write!(
                out,
                "class {} {} {}{}{}",
                c.is_abstract,
                c.name.text,
                plain(&c.type_params),
                plain(&c.extends),
                plain(&c.implements)
            );
            for m in &c.members {
                decorators(&m.decorators, out);
                let _ = write!(out, "{:?}", m.modifiers);
                match &m.kind {
                    ClassMemberKind::Field {
                        name,
                        optional,
                        ty: t,
                        value,
                    } => {
                        let _ = write!(out, "field {} {optional}", name.text);
                        ty(t, out);
                        if let Some(v) = value {
                            expr(v, out);
                        }
                    }
                    ClassMemberKind::Method {
                        name,
                        func: f,
                        has_body,
                    } => {
                        let _ = write!(out, "method {} {has_body}", name.text);
                        func(f, out);
                    }
                    ClassMemberKind::Getter { name, ty: t, body } => {
                        let _ = write!(out, "get {}{}", name.text, plain(t));
                        block(body, out);
                    }
                    ClassMemberKind::Setter { name, param, body } => {
                        let _ = write!(
                            out,
                            "set {} {}{}",
                            name.text,
                            param.name.text,
                            plain(&param.ty)
                        );
                        block(body, out);
                    }
                    ClassMemberKind::Constructor { func: f, has_body } => {
                        let _ = write!(out, "constructor {has_body}");
                        func(f, out);
                    }
                }
                out.push(';');
            }
        }
        StmtKind::Interface(i) => out.push_str(&plain(i)),
        StmtKind::Enum(e) => {
            let _ = write!(out, "enum {}", e.name.text);
            for item in &e.items {
                let _ = write!(out, "{}", item.name.text);
                if let Some(v) = &item.value {
                    expr(v, out);
                }
                out.push(',');
            }
        }
        StmtKind::TypeAlias(a) => out.push_str(&plain(a)),
        StmtKind::Declare(d) => out.push_str(&plain(d)),
        StmtKind::Import(i) => {
            let _ = write!(
                out,
                "import {} {} {:?} {:?}",
                i.type_only,
                i.internal,
                plain(&i.clause),
                i.from.value
            );
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

fn args(a: &Args, out: &mut String) {
    out.push('(');
    for e in a.exprs() {
        expr(e, out);
        out.push(',');
    }
    out.push(')');
}

fn expr(e: &Expr, out: &mut String) {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || expr_inner(e, out))
}

fn expr_inner(e: &Expr, out: &mut String) {
    match &e.kind {
        ExprKind::Nil => out.push_str("nil"),
        ExprKind::True => out.push_str("true"),
        ExprKind::False => out.push_str("false"),
        ExprKind::Number(n) => {
            let _ = write!(out, "#{}", n.raw);
        }
        ExprKind::String(s) => {
            let _ = write!(out, "{:?}", s.value);
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
                        let _ = write!(out, "{}=", name.text);
                        expr(value, out);
                    }
                    TableField::Positional(v) => expr(v, out),
                }
                out.push(',');
            }
            out.push('}');
        }
        ExprKind::Name(n) => out.push_str(&n.text),
        ExprKind::Index { obj, key } => {
            expr(obj, out);
            out.push('[');
            expr(key, out);
            out.push(']');
        }
        ExprKind::Field { obj, name } => {
            expr(obj, out);
            let _ = write!(out, ".{}", name.text);
        }
        ExprKind::Call { callee, args: a } => {
            expr(callee, out);
            args(a, out);
        }
        ExprKind::Method { obj, name, args: a } => {
            expr(obj, out);
            let _ = write!(out, ":{}", name.text);
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
        ExprKind::OptField { obj, name } => {
            expr(obj, out);
            let _ = write!(out, "?.{}", name.text);
        }
        ExprKind::OptIndex { obj, key } => {
            expr(obj, out);
            out.push_str("?.[");
            expr(key, out);
            out.push(']');
        }
        ExprKind::OptMethod { obj, name, args: a } => {
            expr(obj, out);
            let _ = write!(out, "?:{}", name.text);
            args(a, out);
        }
        ExprKind::OptCall { callee, args: a } => {
            expr(callee, out);
            out.push_str("?.");
            args(a, out);
        }
        ExprKind::Cast { expr: x, ty: t } => {
            out.push('(');
            expr(x, out);
            out.push_str("::");
            ty(t, out);
            out.push(')');
        }
        ExprKind::Interp(parts) => {
            out.push('`');
            for p in parts {
                match p {
                    InterpPart::Text { value, .. } => {
                        let _ = write!(out, "{value:?}");
                    }
                    InterpPart::Expr(e) => {
                        out.push('{');
                        expr(e, out);
                        out.push('}');
                    }
                }
            }
            out.push('`');
        }
        other => {
            let _ = write!(out, "{other:?}");
        }
    }
}
