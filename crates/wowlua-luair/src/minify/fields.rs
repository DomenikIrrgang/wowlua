//! The table half of `rename-private` (spec optimization §15.4.1): fields of
//! a `local t = { … }` table that provably does not escape get short names.
//! It does not escape when `t` is never assigned and only ever used as
//! `t.field` (read, assigned, or `function t.field()`): never passed,
//! returned, indexed by an expression, iterated or called with `:`.

use super::rename::short_name;
use crate::ir::*;
use crate::scope::{Ref, Resolution, Use, Visitor, ident_count, resolve, walk};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A `local t = { … }` with only `name = value` and positional fields.
fn plain_table(s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Local { names, values } if names.len() == 1 && values.len() == 1 => {
            matches!(&values[0].kind, ExprKind::Table { fields, .. }
                if fields.iter().all(|f| !matches!(f, Field::Keyed { .. })))
        }
        _ => false,
    }
}

/// The occurrence index of the declared name of a [`plain_table`] statement
/// whose values have not been walked yet.
fn decl_index(s: &mut Stmt, next: usize) -> usize {
    let StmtKind::Local { values, .. } = &mut s.kind else {
        unreachable!()
    };
    next + ident_count(&mut values[0])
}

struct Analyze<'a> {
    res: &'a Resolution,
    next: usize,
    /// Occurrences that are the object of `t.field`.
    allowed: HashSet<usize>,
    candidates: HashSet<usize>,
    escapes: HashSet<usize>,
    /// Field names per candidate, with their use counts.
    fields: HashMap<usize, BTreeMap<String, usize>>,
}

impl Analyze<'_> {
    fn binding(&self, occurrence: usize) -> Option<usize> {
        match self.res.occurrences.get(occurrence) {
            Some(Ref::Local(b)) => Some(*b),
            _ => None,
        }
    }

    fn field(&mut self, occurrence: usize, name: &str) {
        self.allowed.insert(occurrence);
        if let Some(b) = self.binding(occurrence) {
            *self
                .fields
                .entry(b)
                .or_default()
                .entry(name.to_string())
                .or_default() += 1;
        }
    }
}

impl Visitor for Analyze<'_> {
    fn stmt(&mut self, s: &mut Stmt) {
        if plain_table(s) {
            let index = decl_index(s, self.next);
            if let Some(b) = self.binding(index) {
                self.candidates.insert(b);
                let StmtKind::Local { values, .. } = &s.kind else {
                    unreachable!()
                };
                if let ExprKind::Table { fields, .. } = &values[0].kind {
                    for f in fields {
                        if let Field::Named { name, .. } = f {
                            *self
                                .fields
                                .entry(b)
                                .or_default()
                                .entry(name.name.to_string())
                                .or_default() += 1;
                        }
                    }
                }
            }
        }
        if let StmtKind::Function { path, .. } = &s.kind
            && path.len() >= 2
        {
            let name = path[1].name.clone();
            self.field(self.next, &name);
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        if let ExprKind::Field { obj, name } = &e.kind
            && matches!(obj.kind, ExprKind::Name(_))
        {
            let name = name.name.clone();
            self.field(self.next, &name);
        }
    }

    fn ident(&mut self, _id: &mut Ident, use_: Use) {
        let index = self.next;
        self.next += 1;
        if use_ == Use::Declare {
            return;
        }
        if let Some(b) = self.binding(index)
            && !self.allowed.contains(&index)
        {
            self.escapes.insert(b);
        }
    }
}

struct Rename<'a> {
    res: &'a Resolution,
    next: usize,
    names: &'a HashMap<usize, HashMap<String, String>>,
}

impl Rename<'_> {
    fn renamed(&self, occurrence: usize, id: &mut Ident) {
        if let Some(Ref::Local(b)) = self.res.occurrences.get(occurrence)
            && let Some(new) = self.names.get(b).and_then(|m| m.get(id.name.as_str()))
        {
            id.name = new.as_str().into();
        }
    }
}

impl Visitor for Rename<'_> {
    fn stmt(&mut self, s: &mut Stmt) {
        if plain_table(s) {
            let index = decl_index(s, self.next);
            if let StmtKind::Local { values, .. } = &mut s.kind
                && let ExprKind::Table { fields, .. } = &mut values[0].kind
            {
                for f in fields {
                    if let Field::Named { name, .. } = f {
                        self.renamed(index, name);
                    }
                }
            }
        }
        if let StmtKind::Function { path, .. } = &mut s.kind
            && path.len() >= 2
        {
            let next = self.next;
            self.renamed(next, &mut path[1]);
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        if let ExprKind::Field { obj, name } = &mut e.kind
            && matches!(obj.kind, ExprKind::Name(_))
        {
            let next = self.next;
            self.renamed(next, name);
        }
    }

    fn ident(&mut self, _id: &mut Ident, _use: Use) {
        self.next += 1;
    }
}

pub fn apply(chunk: &mut Chunk) {
    let res = resolve(chunk);
    let mut analyze = Analyze {
        res: &res,
        next: 0,
        allowed: HashSet::new(),
        candidates: HashSet::new(),
        escapes: HashSet::new(),
        fields: HashMap::new(),
    };
    walk(chunk, &mut analyze);
    let mut names: HashMap<usize, HashMap<String, String>> = HashMap::new();
    for &b in &analyze.candidates {
        if analyze.escapes.contains(&b) || res.bindings[b].writes > 0 {
            continue;
        }
        let Some(fields) = analyze.fields.get(&b) else {
            continue;
        };
        let mut order: Vec<(&String, &usize)> = fields.iter().collect();
        order.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        let keywords = super::rename::KEYWORDS;
        let mut fresh = (0..)
            .map(short_name)
            .filter(|n| !keywords.contains(&n.as_str()));
        let map = order
            .into_iter()
            .map(|(old, _)| (old.clone(), fresh.next().expect("unbounded")))
            .collect();
        names.insert(b, map);
    }
    if names.is_empty() {
        return;
    }
    walk(
        chunk,
        &mut Rename {
            res: &res,
            next: 0,
            names: &names,
        },
    );
}
