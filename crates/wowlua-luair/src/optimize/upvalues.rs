//! `optimize.cache-upvalues` (spec optimization §15.2.3): a function that
//! reads a local of an enclosing function (an upvalue) in a loop, or at
//! least three times, while nothing ever assigns it after its declaration,
//! starts with `local name = name` and reads its own copy, a register
//! instead of an upvalue. Reads as a callee do not count (a call copies
//! the function into a register either way). Runs on a chunk with unique
//! names (`names::uniquify`).

use super::loops::{Uses, functions, uses};
use super::names::{Unique, base, is_local};
use super::walk::*;
use crate::ir::*;
use crate::minify::LOCALS_BUDGET;
use std::collections::{BTreeMap, HashSet};

/// Reads (not as a callee) in a function's own code: in all, and in loops.
#[derive(Default)]
struct Reads {
    all: BTreeMap<String, (u32, u32)>,
    declared: HashSet<String>,
    /// Names the compiler generated (the copy is generated too).
    generated: HashSet<String>,
}

fn record(e: &mut Expr, in_loop: bool, r: &mut Reads) {
    each_expr(e, false, &mut |e| {
        let callee = match &e.kind {
            ExprKind::Call { callee, .. } => match &callee.kind {
                ExprKind::Name(f) => Some(f.name.to_string()),
                _ => None,
            },
            _ => None,
        };
        for p in parts(e) {
            if let ExprKind::Name(id) = &p.kind
                && is_local(&id.name)
                && callee.as_deref() != Some(id.name.as_str())
            {
                if id.generated() {
                    r.generated.insert(id.name.to_string());
                }
                let entry = r.all.entry(id.name.to_string()).or_default();
                entry.0 += 1;
                entry.1 += u32::from(in_loop);
            }
        }
    });
}

fn scan(b: &mut Block, in_loop: bool, r: &mut Reads) {
    for s in &mut b.stmts {
        match &s.kind {
            StmtKind::Local { names, .. } => {
                r.declared.extend(names.iter().map(|n| n.name.to_string()));
            }
            StmtKind::LocalFunction { name, .. } => {
                r.declared.insert(name.name.to_string());
            }
            StmtKind::NumericFor { var, .. } => {
                r.declared.insert(var.name.to_string());
            }
            StmtKind::GenericFor { vars, .. } => {
                r.declared.extend(vars.iter().map(|n| n.name.to_string()));
            }
            _ => {}
        }
        let looping = in_loop || is_loop(s);
        let targets = match &s.kind {
            StmtKind::Assign { targets, .. } => targets.len(),
            _ => 0,
        };
        // A loop's header runs once, its condition each time.
        let header_once = matches!(
            s.kind,
            StmtKind::NumericFor { .. } | StmtKind::GenericFor { .. }
        );
        for (i, e) in own_exprs(s).into_iter().enumerate() {
            let in_loop = if header_once { in_loop } else { looping };
            if i >= targets
                && let ExprKind::Name(id) = &e.kind
                && is_local(&id.name)
            {
                if id.generated() {
                    r.generated.insert(id.name.to_string());
                }
                let entry = r.all.entry(id.name.to_string()).or_default();
                entry.0 += 1;
                entry.1 += u32::from(in_loop);
            }
            record(e, in_loop, r);
        }
        for inner in inner_blocks(s) {
            scan(inner, looping, r);
        }
    }
}

/// `cache-upvalues`.
pub(crate) fn cache_upvalues(chunk: &mut Chunk, u: &mut Unique) {
    let usage: Uses = uses(chunk);
    let mut main = Func {
        pos: Pos::NONE,
        params: Vec::new(),
        vararg: true,
        body: std::mem::take(&mut chunk.block),
    };
    let mut first = true;
    functions(&mut main, false, &mut |f, method| {
        // The main chunk has no upvalues.
        if std::mem::take(&mut first) {
            return;
        }
        let mut r = Reads::default();
        r.declared
            .extend(f.params.iter().map(|p| p.name.to_string()));
        scan(&mut f.body, false, &mut r);
        let mut room = LOCALS_BUDGET.saturating_sub(max_locals(f, method));
        let mut copies: Vec<(String, String, bool)> = Vec::new();
        for (name, (all, in_loop)) in &r.all {
            if room == 0 {
                break;
            }
            // A method's `self` is its own.
            if r.declared.contains(name)
                || (method && base(name) == "self")
                || usage.writes.contains(name)
                || (*in_loop == 0 && *all < 3)
            {
                continue;
            }
            copies.push((
                name.clone(),
                u.fresh(base(name)),
                r.generated.contains(name),
            ));
            room -= 1;
        }
        if copies.is_empty() {
            return;
        }
        let renames: std::collections::HashMap<&str, &str> = copies
            .iter()
            .map(|(o, n, _)| (o.as_str(), n.as_str()))
            .collect();
        each_name_in_block(&mut f.body, &mut |id| {
            if let Some(new) = renames.get(id.name.as_str()) {
                id.rename(*new);
            }
        });
        let pos = f.pos;
        let mut names = Vec::new();
        let mut values = Vec::new();
        // A generated local's copy is generated too: it gets a name of its
        // own, which hygiene keeps apart from the user's names.
        for (old, new, generated) in &copies {
            let mut name = Ident::new(new.as_str(), pos);
            name.set_generated(*generated);
            names.push(name);
            let mut value = Ident::new(old.as_str(), pos);
            value.set_generated(*generated);
            values.push(Expr::new(ExprKind::Name(value), pos));
        }
        f.body.stmts.insert(
            0,
            Stmt {
                pos,
                kind: StmtKind::Local { names, values },
            },
        );
    });
    chunk.block = main.body;
}
