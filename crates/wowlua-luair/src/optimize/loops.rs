//! Loops and appends (spec optimization §15.2.3):
//!
//! - `numeric-loops`: `for i, v in ipairs(t)` over a plain array becomes a
//!   numeric loop that reads `t[i]` and stops at the first `nil`, as
//!   `ipairs` does;
//! - `table-insert`: `table.insert(t, v)` (or `tinsert`) of a plain table
//!   becomes `t[#t + 1] = v`;
//! - `hoist-closures`: a closure made in a loop that captures nothing the
//!   loop declares is made once, before the loop, where its identity cannot
//!   be seen: a local function only called, or a function passed to a call
//!   that calls it and keeps nothing (`pcall`, `xpcall`, `table.sort`,
//!   `string.gsub`, `table.foreach`, `table.foreachi`).
//!
//! All run on a chunk with unique names (`names::uniquify`).

use super::globals::Globals;
use super::names::{Unique, is_local};
use super::walk::*;
use crate::ir::*;
use crate::minify::LOCALS_BUDGET;
use std::collections::{HashMap, HashSet};

fn number(n: &str) -> Expr {
    Expr::new(ExprKind::Number(n.into()), Pos::NONE)
}

fn at(mut e: Expr, pos: Pos) -> Expr {
    e.pos = pos;
    e
}

/// `numeric-loops`.
pub(crate) fn numeric_loops(chunk: &mut Chunk, g: &Globals, u: &mut Unique) {
    if !g.stable("ipairs") {
        return;
    }
    each_block(&mut chunk.block, &mut |b| {
        for s in &mut b.stmts {
            if let Some(new) = numeric(s, u) {
                *s = new;
            }
        }
    });
}

fn numeric(s: &mut Stmt, u: &mut Unique) -> Option<Stmt> {
    let StmtKind::GenericFor {
        vars,
        exprs,
        do_pos,
        body,
    } = &mut s.kind
    else {
        return None;
    };
    let [call] = exprs.as_mut_slice() else {
        return None;
    };
    let ExprKind::Call { callee, args } = &mut call.kind else {
        return None;
    };
    let ExprKind::Name(f) = &callee.kind else {
        return None;
    };
    if f.name != "ipairs" || !f.facts().has(Facts::ARG_ARRAY) || !(1..=2).contains(&vars.len()) {
        return None;
    }
    let Args::Parens { args, .. } = args else {
        return None;
    };
    let [array] = args.as_mut_slice() else {
        return None;
    };
    let pos = s.pos;
    let array = std::mem::replace(array, Expr::new(ExprKind::Nil, Pos::NONE));
    // `ipairs` evaluates its argument once.
    let (setup, array) = match &array.kind {
        ExprKind::Name(id) if is_local(&id.name) => (None, array),
        _ => {
            let local = u.fresh_ident("__wl_a", pos);
            let setup = Stmt {
                pos,
                kind: StmtKind::Local {
                    names: vec![local.clone()],
                    values: vec![array],
                },
            };
            (Some(setup), name_expr(local))
        }
    };
    let index = vars[0].clone();
    let value = match vars.get(1) {
        Some(v) => v.clone(),
        None => u.fresh_ident("__wl_v", pos),
    };
    let read = Expr::new(
        ExprKind::Index {
            obj: Box::new(at(array, pos)),
            key: Box::new(name_expr(Ident {
                pos,
                ..index.clone()
            })),
            facts: Facts::PLAIN,
        },
        pos,
    );
    let mut stmts = vec![
        Stmt {
            pos,
            kind: StmtKind::Local {
                names: vec![value.clone()],
                values: vec![read],
            },
        },
        Stmt {
            pos,
            kind: StmtKind::If {
                clauses: vec![IfClause {
                    cond: Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Eq,
                            facts: Facts::NO_META,
                            op_pos: Pos::NONE,
                            lhs: Box::new(name_expr(Ident { pos, ..value })),
                            rhs: Box::new(Expr::new(ExprKind::Nil, pos)),
                        },
                        pos,
                    ),
                    then_pos: Pos::NONE,
                    body: Block {
                        stmts: vec![Stmt {
                            pos,
                            kind: StmtKind::Break,
                        }],
                        end: Pos::NONE,
                    },
                }],
                else_block: None,
            },
        },
    ];
    stmts.append(&mut body.stmts);
    // Up to the largest integer a double holds: the loop ends at the
    // first `nil`, wherever that is, as `ipairs`' does.
    let limit = Expr::new(
        ExprKind::Binary {
            op: BinOp::Pow,
            facts: Facts::NO_META,
            op_pos: Pos::NONE,
            lhs: Box::new(number("2")),
            rhs: Box::new(number("53")),
        },
        Pos::NONE,
    );
    let numeric = Stmt {
        pos,
        kind: StmtKind::NumericFor {
            var: index,
            start: Box::new(at(number("1"), pos)),
            limit: Box::new(limit),
            step: None,
            do_pos: *do_pos,
            body: Block {
                stmts,
                end: body.end,
            },
        },
    };
    Some(match setup {
        None => numeric,
        Some(setup) => Stmt {
            pos,
            kind: StmtKind::Do(Block {
                stmts: vec![setup, numeric],
                end: Pos::NONE,
            }),
        },
    })
}

/// Evaluating `e` runs no code, changes nothing and gives one value.
pub(crate) fn pure(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Name(_)
        | ExprKind::Function(_) => true,
        ExprKind::Paren { inner, .. } => pure(inner),
        ExprKind::Field { obj, name } => name.facts().has(Facts::PLAIN) && pure(obj),
        ExprKind::Index { obj, key, facts } => facts.has(Facts::PLAIN) && pure(obj) && pure(key),
        ExprKind::Binary {
            facts, lhs, rhs, ..
        } => facts.has(Facts::NO_META) && pure(lhs) && pure(rhs),
        ExprKind::Unary { facts, operand, .. } => facts.has(Facts::NO_META) && pure(operand),
        ExprKind::Table { fields, .. } => fields.iter().all(|f| match f {
            Field::Keyed { key, value, .. } => pure(key) && pure(value),
            Field::Named { value, .. } | Field::Positional(value) => pure(value),
        }),
        ExprKind::Vararg | ExprKind::Call { .. } | ExprKind::Method { .. } => false,
    }
}

/// A table expression that may be read twice: a name, or plain fields of
/// one.
fn rereadable(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Name(_) => true,
        ExprKind::Field { obj, name } => name.facts().has(Facts::PLAIN) && rereadable(obj),
        _ => false,
    }
}

/// `table-insert`.
pub(crate) fn table_insert(chunk: &mut Chunk, g: &Globals) {
    each_block(&mut chunk.block, &mut |b| {
        for s in &mut b.stmts {
            if let Some(new) = append(s, g) {
                *s = new;
            }
        }
    });
}

fn append(s: &mut Stmt, g: &Globals) -> Option<Stmt> {
    let StmtKind::Call(call) = &mut s.kind else {
        return None;
    };
    let ExprKind::Call { callee, args } = &mut call.kind else {
        return None;
    };
    let insert = match &callee.kind {
        ExprKind::Name(f) => {
            f.name == "tinsert" && f.facts().has(Facts::ARG_PLAIN) && g.stable("tinsert")
        }
        ExprKind::Field { obj, name } => {
            matches!(&obj.kind, ExprKind::Name(t) if t.name == "table")
                && name.name == "insert"
                && name.facts().has(Facts::ARG_PLAIN)
                && g.stable_field("table", "insert")
        }
        _ => false,
    };
    if !insert {
        return None;
    }
    let Args::Parens { args, .. } = args else {
        return None;
    };
    let [t, v] = args.as_mut_slice() else {
        return None;
    };
    // `#t` is read before `v` is evaluated, where `table.insert` reads it
    // after: `v` must change nothing.
    if !rereadable(t) || !pure(v) {
        return None;
    }
    let pos = s.pos;
    let mut t = t.clone();
    t.sep = Pos::NONE;
    let mut v = v.clone();
    v.sep = Pos::NONE;
    let len = Expr::new(
        ExprKind::Unary {
            op: UnOp::Len,
            facts: Facts::NO_META,
            operand: Box::new(t.clone()),
        },
        t.pos,
    );
    let key = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            facts: Facts::NO_META,
            op_pos: Pos::NONE,
            lhs: Box::new(len),
            rhs: Box::new(number("1")),
        },
        t.pos,
    );
    let target = Expr::new(
        ExprKind::Index {
            obj: Box::new(t.clone()),
            key: Box::new(key),
            facts: Facts::PLAIN,
        },
        t.pos,
    );
    Some(Stmt {
        pos,
        kind: StmtKind::Assign {
            targets: vec![target],
            values: vec![v],
        },
    })
}

/// The calls that call a function passed to them before they return and
/// keep nothing of it (the immediate ones of spec compiler §13.5.4 but
/// the secure API, which must see its own call, and the coroutine and
/// mixin functions, which keep it): the argument position of the function.
fn immediate(callee: &Expr, g: &Globals) -> Option<&'static [usize]> {
    match &callee.kind {
        ExprKind::Name(f) if !is_local(&f.name) && g.stable(&f.name) => match f.name.as_str() {
            "pcall" => Some(&[0]),
            "xpcall" => Some(&[0, 1]),
            "sort" | "foreach" | "foreachi" => Some(&[1]),
            "gsub" => Some(&[2]),
            _ => None,
        },
        ExprKind::Field { obj, name } => {
            let ExprKind::Name(lib) = &obj.kind else {
                return None;
            };
            if is_local(&lib.name) || !g.stable_field(&lib.name, &name.name) {
                return None;
            }
            match (lib.name.as_str(), name.name.as_str()) {
                ("table", "sort" | "foreach" | "foreachi") => Some(&[1]),
                ("string", "gsub") => Some(&[2]),
                _ => None,
            }
        }
        _ => None,
    }
}

/// How each local is used: reads (the name as a value or called) and
/// calls.
#[derive(Default)]
pub(crate) struct Uses {
    pub(crate) reads: HashMap<String, u32>,
    pub(crate) calls: HashMap<String, u32>,
    pub(crate) writes: HashSet<String>,
}

pub(crate) fn uses(chunk: &mut Chunk) -> Uses {
    let mut u = Uses::default();
    let b = std::mem::take(&mut chunk.block);
    // Reads and writes, through the resolution walk's uses.
    struct Count<'a>(&'a mut Uses);
    impl crate::scope::Visitor for Count<'_> {
        fn ident(&mut self, id: &mut Ident, use_: crate::scope::Use) {
            if !is_local(&id.name) {
                return;
            }
            match use_ {
                crate::scope::Use::Read => {
                    *self.0.reads.entry(id.name.to_string()).or_default() += 1
                }
                crate::scope::Use::Write => {
                    self.0.writes.insert(id.name.to_string());
                }
                crate::scope::Use::Declare => {}
            }
        }
        fn expr(&mut self, e: &mut Expr) {
            if let ExprKind::Call { callee, .. } = &e.kind
                && let ExprKind::Name(f) = &callee.kind
                && is_local(&f.name)
            {
                *self.0.calls.entry(f.name.to_string()).or_default() += 1;
            }
        }
    }
    let mut c = Chunk {
        block: b,
        comments: Vec::new(),
    };
    crate::scope::walk(&mut c, &mut Count(&mut u));
    chunk.block = c.block;
    u
}

/// The names a block declares, with nested functions' (their parameters
/// and locals).
fn declared_in(b: &mut Block) -> HashSet<String> {
    let mut out = HashSet::new();
    struct Decl<'a>(&'a mut HashSet<String>);
    impl crate::scope::Visitor for Decl<'_> {
        fn ident(&mut self, id: &mut Ident, use_: crate::scope::Use) {
            if use_ == crate::scope::Use::Declare {
                self.0.insert(id.name.to_string());
            }
        }
    }
    let mut c = Chunk {
        block: std::mem::take(b),
        comments: Vec::new(),
    };
    crate::scope::walk(&mut c, &mut Decl(&mut out));
    *b = c.block;
    out
}

/// The locals a function uses that it does not declare (its own name
/// aside).
fn free_names(f: &mut Func) -> HashSet<String> {
    let mut used = HashSet::new();
    let mut declared: HashSet<String> = f.params.iter().map(|p| p.name.to_string()).collect();
    declared.extend(declared_in(&mut f.body));
    each_name_in_block(&mut f.body, &mut |id| {
        if is_local(&id.name) {
            used.insert(id.name.to_string());
        }
    });
    used.retain(|n| !declared.contains(n));
    used
}

/// `hoist-closures`.
pub(crate) fn hoist_closures(chunk: &mut Chunk, g: &Globals, u: &mut Unique) {
    let usage = uses(chunk);
    let mut renames: HashMap<String, String> = HashMap::new();
    let mut main = Func {
        pos: Pos::NONE,
        params: Vec::new(),
        vararg: true,
        body: std::mem::take(&mut chunk.block),
    };
    functions(&mut main, false, &mut |f, method| {
        let budget = LOCALS_BUDGET.saturating_sub(max_locals(f, method));
        let mut room = budget;
        hoist_in_block(&mut f.body, g, &usage, u, &mut room, &mut renames);
    });
    chunk.block = main.body;
    if !renames.is_empty() {
        each_name_in_block(&mut chunk.block, &mut |id| {
            if let Some(new) = renames.get(id.name.as_str()) {
                id.rename(new.as_str());
                id.set_generated(true);
            }
        });
    }
}

/// Calls `f` on `func` and every function in it (with whether it is a
/// method), innermost last.
pub(crate) fn functions(func: &mut Func, method: bool, f: &mut impl FnMut(&mut Func, bool)) {
    f(func, method);
    each_function_in_block(&mut func.body, &mut |inner, m| functions(inner, m, f));
}

/// The functions a block's code declares or creates (not nested ones).
pub(crate) fn each_function_in_block(b: &mut Block, f: &mut impl FnMut(&mut Func, bool)) {
    for s in &mut b.stmts {
        for e in own_exprs(s) {
            each_function(e, &mut |func| f(func, false));
        }
        let method = matches!(
            &s.kind,
            StmtKind::Function {
                method: Some(_),
                ..
            }
        );
        if let Some(func) = declared_function(s) {
            f(func, method);
        }
        for inner in inner_blocks(s) {
            each_function_in_block(inner, f);
        }
    }
}

/// Hoists out of the loops of a function's block (its own code).
fn hoist_in_block(
    b: &mut Block,
    g: &Globals,
    usage: &Uses,
    u: &mut Unique,
    room: &mut usize,
    renames: &mut HashMap<String, String>,
) {
    let mut i = 0;
    while i < b.stmts.len() {
        if is_loop(&b.stmts[i]) && *room > 0 {
            let inside = {
                let mut tmp = Block {
                    stmts: vec![std::mem::replace(&mut b.stmts[i], placeholder())],
                    end: Pos::NONE,
                };
                let d = declared_in(&mut tmp);
                b.stmts[i] = tmp.stmts.pop().expect("the loop");
                d
            };
            let mut hoisted: Vec<Stmt> = Vec::new();
            let pos = b.stmts[i].pos;
            let lp = &mut b.stmts[i];
            // Function arguments of immediate calls.
            for e in own_exprs(lp) {
                hoist_args(e, g, &inside, u, room, &mut hoisted, pos);
            }
            for inner in inner_blocks(lp) {
                hoist_in_loop(
                    inner,
                    g,
                    usage,
                    &inside,
                    u,
                    room,
                    &mut hoisted,
                    renames,
                    pos,
                );
            }
            let n = hoisted.len();
            for (k, h) in hoisted.into_iter().enumerate() {
                b.stmts.insert(i + k, h);
            }
            i += n;
        }
        // Loops further in: a closure that captures this loop's locals but
        // none of an inner one's.
        let s = &mut b.stmts[i];
        for inner in inner_blocks(s) {
            hoist_in_block(inner, g, usage, u, room, renames);
        }
        i += 1;
    }
}

/// Within a loop's blocks (its own code): local functions only called,
/// and function arguments of immediate calls.
#[allow(clippy::too_many_arguments)]
fn hoist_in_loop(
    b: &mut Block,
    g: &Globals,
    usage: &Uses,
    inside: &HashSet<String>,
    u: &mut Unique,
    room: &mut usize,
    hoisted: &mut Vec<Stmt>,
    renames: &mut HashMap<String, String>,
    pos: Pos,
) {
    let mut i = 0;
    while i < b.stmts.len() {
        let movable = *room > 0
            && match &mut b.stmts[i].kind {
                StmtKind::LocalFunction { name, func } => {
                    only_called(&name.name, usage) && {
                        let mut free = free_names(func);
                        free.remove(name.name.as_str());
                        free.iter().all(|n| !inside.contains(n))
                    }
                }
                StmtKind::Local { names, values } if names.len() == 1 && values.len() == 1 => {
                    only_called(&names[0].name, usage)
                        && match &mut values[0].kind {
                            ExprKind::Function(func) => {
                                free_names(func).iter().all(|n| !inside.contains(n))
                            }
                            _ => false,
                        }
                }
                _ => false,
            };
        if movable {
            let mut s = b.stmts.remove(i);
            s.pos = pos;
            let name = match &mut s.kind {
                StmtKind::LocalFunction { name, .. } => name,
                StmtKind::Local { names, .. } => &mut names[0],
                _ => unreachable!("checked above"),
            };
            let new = u.fresh(&format!("__wl_{}", super::names::base(&name.name)));
            renames.insert(name.name.to_string(), new);
            hoisted.push(s);
            *room -= 1;
            continue;
        }
        let s = &mut b.stmts[i];
        for e in own_exprs(s) {
            hoist_args(e, g, inside, u, room, hoisted, pos);
        }
        for inner in inner_blocks(s) {
            hoist_in_loop(inner, g, usage, inside, u, room, hoisted, renames, pos);
        }
        i += 1;
    }
}

fn only_called(name: &str, usage: &Uses) -> bool {
    is_local(name)
        && !usage.writes.contains(name)
        && usage.reads.get(name).copied().unwrap_or(0)
            == usage.calls.get(name).copied().unwrap_or(0)
}

/// The function arguments of immediate calls in `e` (not inside function
/// expressions) that capture nothing of the loop: each becomes a local
/// before the loop.
fn hoist_args(
    e: &mut Expr,
    g: &Globals,
    inside: &HashSet<String>,
    u: &mut Unique,
    room: &mut usize,
    hoisted: &mut Vec<Stmt>,
    pos: Pos,
) {
    each_expr(e, false, &mut |e| {
        let ExprKind::Call { callee, args } = &mut e.kind else {
            return;
        };
        let Some(positions) = immediate(callee, g) else {
            return;
        };
        let Args::Parens { args, .. } = args else {
            return;
        };
        for &p in positions {
            if *room == 0 {
                return;
            }
            let Some(arg) = args.get_mut(p) else {
                continue;
            };
            let ExprKind::Function(func) = &mut arg.kind else {
                continue;
            };
            if !free_names(func).iter().all(|n| !inside.contains(n)) {
                continue;
            }
            let local = u.fresh_ident("__wl_fn", pos);
            let sep = arg.sep;
            let mut value = std::mem::replace(
                arg,
                name_expr(Ident {
                    pos: arg.pos,
                    ..local.clone()
                }),
            );
            arg.sep = sep;
            value.sep = Pos::NONE;
            hoisted.push(Stmt {
                pos,
                kind: StmtKind::Local {
                    names: vec![local],
                    values: vec![value],
                },
            });
            *room -= 1;
        }
    });
}
