//! Hygiene (spec codegen §14.3): generated identifiers that collide with any
//! identifier of the chunk get a numeric suffix. User identifiers are never
//! renamed.

use crate::ir::*;
use std::collections::{BTreeMap, BTreeSet};

/// Renames colliding generated identifiers in `chunk`.
pub fn apply(chunk: &mut Chunk) {
    let renames = renames(chunk);
    #[cfg(debug_assertions)]
    assert_eq!(renames, renames_of_every_name(chunk), "hygiene");
    if renames.is_empty() {
        return;
    }
    visit_idents(&mut chunk.block, &mut |id: &mut Ident| {
        if id.generated()
            && let Some(new) = renames.get(id.name.as_str())
        {
            id.rename(new.as_str());
        }
    });
}

/// The renames: each generated name that a user name also uses gets the
/// first free `{name}{n}`. Only user names that can be asked about are
/// collected, a generated name or one followed by digits, and none at all
/// in the common case of a chunk without generated names.
fn renames(chunk: &mut Chunk) -> BTreeMap<String, String> {
    let mut generated = BTreeSet::new();
    visit_idents(&mut chunk.block, &mut |id: &mut Ident| {
        if id.generated() && !generated.contains(id.name.as_str()) {
            generated.insert(id.name.to_string());
        }
    });
    if generated.is_empty() {
        return BTreeMap::new();
    }
    // The set is sorted, so the first and last names share the prefix all
    // of them share. Identifiers are ASCII (names are read in the C locale).
    let (first, last) = (
        generated.first().expect("a name"),
        generated.last().expect("a name"),
    );
    let prefix = &first[..first
        .bytes()
        .zip(last.bytes())
        .take_while(|(a, b)| a == b)
        .count()];
    let mut user = BTreeSet::new();
    visit_idents(&mut chunk.block, &mut |id: &mut Ident| {
        if !id.generated()
            && id.name.starts_with(prefix)
            && may_collide(&id.name, &generated)
            && !user.contains(id.name.as_str())
        {
            user.insert(id.name.to_string());
        }
    });
    choose(&user, &generated)
}

/// `name` is a generated name, or a generated name followed by ASCII digits:
/// the only strings `choose` looks up among the user names.
fn may_collide(name: &str, generated: &BTreeSet<String>) -> bool {
    let start = name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    (start..=name.len()).any(|k| generated.contains(&name[..k]))
}

fn choose(user: &BTreeSet<String>, generated: &BTreeSet<String>) -> BTreeMap<String, String> {
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    let mut taken: BTreeSet<String> = user.union(generated).cloned().collect();
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
    renames
}

/// The renames from every name of the chunk, as hygiene chose them before
/// it filtered the user names: debug builds check every chunk against it.
#[cfg(any(test, debug_assertions))]
fn renames_of_every_name(chunk: &mut Chunk) -> BTreeMap<String, String> {
    let mut user = BTreeSet::new();
    let mut generated = BTreeSet::new();
    visit_idents(&mut chunk.block, &mut |id: &mut Ident| {
        if id.generated() {
            generated.insert(id.name.to_string());
        } else {
            user.insert(id.name.to_string());
        }
    });
    choose(&user, &generated)
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
        ExprKind::Index { obj, key, .. } => {
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
                ExprKind::Name({
                    let mut id = Ident::new(n, Pos::NONE);
                    id.set_generated(generated);
                    id
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

    /// A chunk returning the names, each `(name, generated)`.
    fn chunk_of(names: &[(String, bool)]) -> Chunk {
        let values = names
            .iter()
            .map(|(n, generated)| {
                Expr::new(
                    ExprKind::Name({
                        let mut id = Ident::new(n.as_str(), Pos::NONE);
                        id.set_generated(*generated);
                        id
                    }),
                    Pos::NONE,
                )
            })
            .collect();
        Chunk {
            block: Block {
                stmts: vec![Stmt {
                    pos: Pos::NONE,
                    kind: StmtKind::Return(values),
                }],
                end: Pos::NONE,
            },
            comments: Vec::new(),
        }
    }

    fn renames_for(names: &[(&str, bool)]) -> BTreeMap<String, String> {
        let names: Vec<(String, bool)> = names.iter().map(|&(n, g)| (n.to_string(), g)).collect();
        let mut chunk = chunk_of(&names);
        let fast = renames(&mut chunk);
        assert_eq!(fast, renames_of_every_name(&mut chunk));
        fast
    }

    #[test]
    fn candidates_skip_user_names_with_digits() {
        let r = renames_for(&[
            ("__wl_t", true),
            ("__wl_t", false),
            ("__wl_t1", false),
            ("a", false),
        ]);
        assert_eq!(r["__wl_t"], "__wl_t2");
        // A generated name that ends in digits.
        let r = renames_for(&[("__wl_t1", true), ("__wl_t1", false), ("__wl_t11", false)]);
        assert_eq!(r["__wl_t1"], "__wl_t12");
    }

    #[test]
    fn many_generated_names() {
        let mut names: Vec<(String, bool)> =
            (0..200).map(|i| (format!("__wl_s{i}"), true)).collect();
        for user in ["__wl_s7", "__wl_s71", "__wl_s710", "__wl_s", "x"] {
            names.push((user.to_string(), false));
        }
        let mut chunk = chunk_of(&names);
        let fast = renames(&mut chunk);
        assert_eq!(fast, renames_of_every_name(&mut chunk));
        // `__wl_s71` to `__wl_s79` are generated and `__wl_s710` is a user
        // name, so `__wl_s7` gets `__wl_s711`, and `__wl_s71` the next one.
        assert_eq!(fast["__wl_s7"], "__wl_s711");
        assert_eq!(fast["__wl_s71"], "__wl_s712");
    }

    #[test]
    fn nothing_generated_nothing_renamed() {
        assert!(renames_for(&[("__wl_t", false), ("a", false)]).is_empty());
    }

    /// The filtered user names give the same renames as all of them, on
    /// pseudo-random chunks of names that collide in every way.
    #[test]
    fn same_renames_as_every_name() {
        const POOL: &[&str] = &[
            "__wl", "__wl1", "__wl_t", "__wl_t1", "__wl_t11", "__wl_t2", "__wl_s1", "__wl_s10",
            "__wl_s", "a", "a1", "b1", "_", "__w",
        ];
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..5000 {
            let len = (next() % 12) as usize;
            let names: Vec<(String, bool)> = (0..len)
                .map(|_| {
                    let base = POOL[(next() % POOL.len() as u64) as usize];
                    let name = match next() % 4 {
                        0 => format!("{base}{}", next() % 13),
                        _ => base.to_string(),
                    };
                    (name, next() % 2 == 0)
                })
                .collect();
            let mut chunk = chunk_of(&names);
            assert_eq!(
                renames(&mut chunk),
                renames_of_every_name(&mut chunk),
                "{names:?}"
            );
        }
    }
}
