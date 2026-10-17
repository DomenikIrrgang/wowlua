//! `dedupe-strings` (spec optimization §15.4.1): string literals repeated
//! often enough to pay for a local become one shared local at the top of the
//! chunk, within the Lua-limit budgets of spec §15.2.2.

use super::literals::{decode_string, encode_string};
use super::{LOCALS_BUDGET, UPVALUES_BUDGET};
use crate::ir::*;
use crate::scope::{Visitor, resolve, walk};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
struct Candidate {
    count: usize,
    /// Bytes of all its literals as written.
    written: usize,
    functions: BTreeSet<usize>,
}

/// Finds the literals and the functions they are in (numbered like
/// [`crate::scope::resolve`]).
#[derive(Default)]
struct Collect {
    found: BTreeMap<Vec<u8>, Candidate>,
    functions: Vec<usize>,
    next: usize,
}

impl Visitor for Collect {
    fn enter_function(&mut self) {
        self.functions.push(self.next);
        self.next += 1;
    }
    fn leave_function(&mut self) {
        self.functions.pop();
    }
    fn expr(&mut self, e: &mut Expr) {
        if let ExprKind::String(raw) = &e.kind
            && let Some(bytes) = decode_string(raw)
        {
            let c = self.found.entry(bytes).or_default();
            c.count += 1;
            c.written += raw.len();
            c.functions
                .insert(*self.functions.last().expect("in a function"));
        }
    }
}

/// `short_names`: the locals will be renamed, so they cost one or two bytes.
pub fn apply(chunk: &mut Chunk, short_names: bool) {
    let res = resolve(chunk);
    let mut collect = Collect::default();
    walk(chunk, &mut collect);
    let name_len = if short_names { 2 } else { 8 };
    let mut ranked: Vec<(usize, Vec<u8>, Candidate)> = collect
        .found
        .into_iter()
        .filter_map(|(bytes, c)| {
            let literal = encode_string(&bytes, b'"').len();
            // Every use becomes a name; the declaration adds the literal, the
            // name and a comma on each side of `=`.
            let cost = literal + name_len + 2 + c.count * name_len;
            (c.count >= 2 && c.written > cost).then(|| (c.written - cost, bytes, c))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut locals = res.functions.first().map_or(0, |f| f.max_locals);
    let mut added: HashMap<usize, usize> = HashMap::new();
    let mut chosen: Vec<Vec<u8>> = Vec::new();
    for (_, bytes, c) in ranked {
        if locals + 1 > LOCALS_BUDGET {
            break;
        }
        // Each function using it, and each function between it and the main
        // chunk, captures the local.
        let mut capturing: BTreeSet<usize> = BTreeSet::new();
        for &f in &c.functions {
            let mut cur = Some(f);
            while let Some(g) = cur
                && g != 0
            {
                capturing.insert(g);
                cur = res.functions[g].parent;
            }
        }
        let fits = capturing.iter().all(|f| {
            res.functions[*f].upvalues.len() + added.get(f).copied().unwrap_or(0) < UPVALUES_BUDGET
        });
        if !fits {
            continue;
        }
        for f in capturing {
            *added.entry(f).or_default() += 1;
        }
        locals += 1;
        chosen.push(bytes);
    }
    if chosen.is_empty() {
        return;
    }
    let names: HashMap<Vec<u8>, String> = chosen
        .iter()
        .enumerate()
        .map(|(i, b)| (b.clone(), format!("__wl_s{}", i + 1)))
        .collect();
    struct Replace<'a>(&'a HashMap<Vec<u8>, String>);
    impl Visitor for Replace<'_> {
        fn expr(&mut self, e: &mut Expr) {
            let replaced = |raw: &[u8]| decode_string(raw).and_then(|b| self.0.get(&b));
            match &mut e.kind {
                ExprKind::String(raw) => {
                    if let Some(name) = replaced(raw) {
                        e.kind = ExprKind::Name(Ident::new_generated(name.as_str(), e.pos));
                    }
                }
                // `f"…"` needs parentheses once the string is a name.
                ExprKind::Call { args, .. } | ExprKind::Method { args, .. } => {
                    if let Args::String(s) = args
                        && let ExprKind::String(raw) = &s.kind
                        && replaced(raw).is_some()
                    {
                        let s = std::mem::replace(s.as_mut(), Expr::new(ExprKind::Nil, Pos::NONE));
                        *args = Args::Parens {
                            args: vec![s],
                            close: Pos::NONE,
                        };
                    }
                }
                _ => {}
            }
        }
    }
    walk(chunk, &mut Replace(&names));
    // The declaration takes the first statement's line, after the comments
    // before it (license headers stay first).
    let pos = chunk.block.stmts.first().map_or(Pos::NONE, |s| s.pos);
    let decl = Stmt {
        pos,
        kind: StmtKind::Local {
            names: chosen
                .iter()
                .map(|b| Ident::new_generated(names[b].as_str(), Pos::NONE))
                .collect(),
            values: chosen
                .iter()
                .map(|b| Expr::new(ExprKind::String(encode_string(b, b'"').into()), Pos::NONE))
                .collect(),
        },
    };
    chunk.block.stmts.insert(0, decl);
    crate::hygiene::apply(chunk);
}
