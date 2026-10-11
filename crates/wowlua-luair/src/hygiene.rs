//! Hygiene (spec codegen §14.3): generated identifiers that collide with any
//! identifier of the chunk get a numeric suffix. User identifiers are never
//! renamed.

use crate::ir::*;
use std::collections::{BTreeMap, BTreeSet};

/// Renames colliding generated identifiers in `chunk`.
pub fn apply(chunk: &mut Chunk) {
    let mut user = BTreeSet::new();
    let mut generated = BTreeSet::new();
    visit_idents(&mut chunk.block, &mut |id: &mut Ident| {
        if id.generated {
            generated.insert(id.name.clone());
        } else {
            user.insert(id.name.clone());
        }
    });
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    let mut taken: BTreeSet<String> = user.union(&generated).cloned().collect();
    for name in generated.iter().filter(|n| user.contains(*n)) {
        let mut n = 1;
        let fresh = loop {
            let candidate = format!("{name}{n}");
            if !taken.contains(&candidate) {
                break candidate;
            }
            n += 1;
        };
        taken.insert(fresh.clone());
        renames.insert(name.clone(), fresh);
    }
    if renames.is_empty() {
        return;
    }
    visit_idents(&mut chunk.block, &mut |id: &mut Ident| {
        if id.generated
            && let Some(new) = renames.get(&id.name)
        {
            id.name = new.clone();
        }
    });
}

fn visit_idents(block: &mut Block, f: &mut impl FnMut(&mut Ident)) {
    for stmt in &mut block.stmts {
        match &mut stmt.kind {
            StmtKind::Local { names, values } => {
                names.iter_mut().for_each(&mut *f);
                values.iter_mut().for_each(|e| expr(e, f));
            }
            StmtKind::Assign { targets, values } => {
                targets.iter_mut().chain(values).for_each(|e| expr(e, f));
            }
            StmtKind::Call(e) => expr(e, f),
            StmtKind::Do(b) => visit_idents(b, f),
            StmtKind::While { cond, body, .. } | StmtKind::Repeat { body, cond } => {
                expr(cond, f);
                visit_idents(body, f);
            }
            StmtKind::If {
                clauses,
                else_block,
            } => {
                for clause in clauses {
                    expr(&mut clause.cond, f);
                    visit_idents(&mut clause.body, f);
                }
                if let Some(b) = else_block {
                    visit_idents(b, f);
                }
            }
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                body,
                ..
            } => {
                f(var);
                expr(start, f);
                expr(limit, f);
                if let Some(s) = step {
                    expr(s, f);
                }
                visit_idents(body, f);
            }
            StmtKind::GenericFor {
                vars, exprs, body, ..
            } => {
                vars.iter_mut().for_each(&mut *f);
                exprs.iter_mut().for_each(|e| expr(e, f));
                visit_idents(body, f);
            }
            StmtKind::Function { path, method, func } => {
                path.iter_mut().for_each(&mut *f);
                if let Some(m) = method {
                    f(m);
                }
                function(func, f);
            }
            StmtKind::LocalFunction { name, func } => {
                f(name);
                function(func, f);
            }
            StmtKind::Return(values) => values.iter_mut().for_each(|e| expr(e, f)),
            StmtKind::Break => {}
        }
    }
}

fn function(func: &mut Func, f: &mut impl FnMut(&mut Ident)) {
    func.params.iter_mut().for_each(&mut *f);
    visit_idents(&mut func.body, f);
}

fn args(a: &mut Args, f: &mut impl FnMut(&mut Ident)) {
    match a {
        Args::Parens { args, .. } => args.iter_mut().for_each(|e| expr(e, f)),
        Args::Table(e) | Args::String(e) => expr(e, f),
    }
}

fn expr(e: &mut Expr, f: &mut impl FnMut(&mut Ident)) {
    match &mut e.kind {
        ExprKind::Name(id) => f(id),
        ExprKind::Function(func) => function(func, f),
        ExprKind::Table { fields, .. } => {
            for field in fields {
                match field {
                    Field::Keyed { key, value, .. } => {
                        expr(key, f);
                        expr(value, f);
                    }
                    // Field names are not variables.
                    Field::Named { value, .. } | Field::Positional(value) => expr(value, f),
                }
            }
        }
        ExprKind::Index { obj, key } => {
            expr(obj, f);
            expr(key, f);
        }
        ExprKind::Field { obj, .. } => expr(obj, f),
        ExprKind::Call { callee, args: a } => {
            expr(callee, f);
            args(a, f);
        }
        ExprKind::Method { obj, args: a, .. } => {
            expr(obj, f);
            args(a, f);
        }
        ExprKind::Paren { inner, .. } => expr(inner, f),
        ExprKind::Binary { lhs, rhs, .. } => {
            expr(lhs, f);
            expr(rhs, f);
        }
        ExprKind::Unary { operand, .. } => expr(operand, f),
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Vararg => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_only_colliding_generated_names() {
        let name = |n: &str, generated: bool| {
            Expr::new(
                ExprKind::Name(Ident {
                    name: n.into(),
                    generated,
                    pos: Pos::NONE,
                }),
                Pos::NONE,
            )
        };
        let mut chunk = Chunk {
            block: Block {
                stmts: vec![Stmt {
                    pos: Pos::NONE,
                    kind: StmtKind::Return(vec![
                        name("__wl_t", true),
                        name("__wl_t", false),
                        name("__wl_u", true),
                    ]),
                }],
                end: Pos::NONE,
            },
            comments: Vec::new(),
        };
        apply(&mut chunk);
        let StmtKind::Return(values) = &chunk.block.stmts[0].kind else {
            panic!()
        };
        let names: Vec<&str> = values
            .iter()
            .map(|e| match &e.kind {
                ExprKind::Name(id) => id.name.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(names, ["__wl_t1", "__wl_t", "__wl_u"]);
    }
}
