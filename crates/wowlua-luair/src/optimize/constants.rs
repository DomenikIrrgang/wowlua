//! `optimize.constants` (spec optimization §15.2.3): a local assigned once,
//! where it is declared, with a constant (`nil`, a boolean, a number or a
//! short string) is that constant wherever it is read, and its declaration
//! goes. Its reads in other functions become constants instead of
//! upvalues.

use super::names::is_local;
use super::walk::*;
use crate::ir::*;
use crate::scope::resolve;
use std::collections::HashMap;

/// Strings longer than this (quotes included) stay in their local: every
/// read would repeat them in the output.
const MAX_STRING: usize = 40;

fn constant(e: &Expr) -> bool {
    match &e.kind {
        // (a string spanning lines would move the lines after its reads)
        ExprKind::String(s) => s.len() <= MAX_STRING && !s.contains(&b'\n'),
        _ => is_literal(e),
    }
}

/// Runs on a chunk with unique names (`names::uniquify`).
pub(crate) fn propagate(chunk: &mut Chunk) {
    let res = resolve(chunk);
    let by_name: HashMap<&str, usize> = res
        .bindings
        .iter()
        .enumerate()
        .map(|(i, b)| (b.name.as_str(), i))
        .collect();
    // The constant of each local declared with one and never assigned.
    let mut values: HashMap<String, Expr> = HashMap::new();
    let mut block = std::mem::take(&mut chunk.block);
    collect(&mut block, &mut |names, vals| {
        for (i, n) in names.iter().enumerate() {
            let Some(&b) = by_name.get(n.name.as_str()) else {
                continue;
            };
            if res.bindings[b].writes > 0 || !is_local(&n.name) {
                continue;
            }
            let value = if vals.is_empty() {
                Expr::new(ExprKind::Nil, Pos::NONE)
            } else if let Some(v) = vals.get(i).filter(|v| constant(v)) {
                v.clone()
            } else {
                continue;
            };
            values.insert(n.name.to_string(), value);
        }
    });
    if values.is_empty() {
        chunk.block = block;
        return;
    }
    // Reads become the constant, except as an object or a callee (a
    // literal there would need parentheses, and is an error anyway).
    replace_in_block(&mut block, &values);
    chunk.block = block;
    // Declarations whose every read was replaced go.
    let res = resolve(chunk);
    let unused: std::collections::HashSet<&str> = res
        .bindings
        .iter()
        .filter(|b| b.uses == 0 && values.contains_key(&b.name))
        .map(|b| b.name.as_str())
        .collect();
    if unused.is_empty() {
        return;
    }
    let unused: std::collections::HashSet<String> = unused.into_iter().map(String::from).collect();
    remove_declarations(&mut chunk.block, &unused);
}

/// Replaces the constants' reads in a block, its functions included.
fn replace_in_block(b: &mut Block, values: &HashMap<String, Expr>) {
    for s in &mut b.stmts {
        let targets = match &s.kind {
            StmtKind::Assign { targets, .. } => targets.len(),
            _ => 0,
        };
        for (i, e) in own_exprs(s).into_iter().enumerate() {
            if i >= targets {
                replace_name(e, values);
            }
            each_expr(e, false, &mut |e| {
                replace(e, values);
                if let ExprKind::Function(func) = &mut e.kind {
                    replace_in_block(&mut func.body, values);
                }
            });
        }
        for inner in inner_blocks(s) {
            replace_in_block(inner, values);
        }
        if let Some(func) = declared_function(s) {
            replace_in_block(&mut func.body, values);
        }
    }
}

fn replace_name(p: &mut Expr, values: &HashMap<String, Expr>) {
    if let ExprKind::Name(id) = &p.kind
        && let Some(v) = values.get(id.name.as_str())
    {
        let sep = p.sep;
        let pos = p.pos;
        *p = v.clone();
        p.pos = pos;
        p.sep = sep;
        if let ExprKind::Unary { operand, .. } = &mut p.kind {
            operand.pos = pos;
        }
    }
}

/// Replaces the names in `e`'s own parts (not `e` itself) that hold a
/// constant, but an object or a callee.
fn replace(e: &mut Expr, values: &HashMap<String, Expr>) {
    let skip_first = matches!(
        e.kind,
        ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Method { .. }
            | ExprKind::Call { .. }
    );
    for (i, p) in parts(e).into_iter().enumerate() {
        if skip_first && i == 0 {
            continue;
        }
        replace_name(p, values);
    }
}

/// Calls `f` with the names and values of every `local` statement, in
/// every block and function.
fn collect(b: &mut Block, f: &mut impl FnMut(&[Ident], &[Expr])) {
    for s in &mut b.stmts {
        if let StmtKind::Local { names, values } = &s.kind {
            f(names, values);
        }
        for e in own_exprs(s) {
            each_expr(e, false, &mut |e| {
                if let ExprKind::Function(func) = &mut e.kind {
                    collect(&mut func.body, f);
                }
            });
        }
        for inner in inner_blocks(s) {
            collect(inner, f);
        }
        if let Some(func) = declared_function(s) {
            collect(&mut func.body, f);
        }
    }
}

/// Takes the locals `unused` out of their declarations.
fn remove_declarations(b: &mut Block, unused: &std::collections::HashSet<String>) {
    for s in &mut b.stmts {
        if let StmtKind::Local { names, values } = &mut s.kind {
            let mut i = 0;
            while i < names.len() {
                if unused.contains(names[i].name.as_str())
                    && (values.is_empty() || i < values.len())
                {
                    names.remove(i);
                    if !values.is_empty() {
                        values.remove(i);
                    }
                } else {
                    i += 1;
                }
            }
        }
        for e in own_exprs(s) {
            each_expr(e, false, &mut |e| {
                if let ExprKind::Function(func) = &mut e.kind {
                    remove_declarations(&mut func.body, unused);
                }
            });
        }
        for inner in inner_blocks(s) {
            remove_declarations(inner, unused);
        }
        if let Some(func) = declared_function(s) {
            remove_declarations(&mut func.body, unused);
        }
    }
    b.stmts
        .retain(|s| !matches!(&s.kind, StmtKind::Local { names, .. } if names.is_empty()));
}
