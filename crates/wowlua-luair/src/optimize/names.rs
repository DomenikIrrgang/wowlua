//! Unique names for the passes that move code (spec optimization §15.2.3):
//! while they run, every local of the chunk has a name of its own
//! (`count\u{1}7`), so a name is its binding wherever code moves, and a
//! name without the mark is a global. [`restore`] gives the locals their
//! names back, and the locals the passes added names no other identifier of
//! the chunk has.

use crate::ir::*;
use crate::scope::{Ref, Resolution, Use, Visitor, resolve, walk};
use std::collections::BTreeSet;

/// Separates a local's name from its number.
const MARK: char = '\u{1}';

/// A chunk whose locals have unique names.
pub(crate) struct Unique {
    /// The next number a new local gets.
    next: usize,
    /// The first number of the locals the passes added.
    first_new: usize,
}

/// `name` is a local's (marked) name.
pub(crate) fn is_local(name: &str) -> bool {
    name.contains(MARK)
}

/// The name a marked name stands for.
pub(crate) fn base(name: &str) -> &str {
    name.split(MARK).next().unwrap_or(name)
}

/// Gives every local of `chunk` a unique name.
pub(crate) fn uniquify(chunk: &mut Chunk) -> Unique {
    let res = resolve(chunk);
    struct Mark<'a> {
        res: &'a Resolution,
        next: usize,
    }
    impl Visitor for Mark<'_> {
        fn ident(&mut self, id: &mut Ident, _use: Use) {
            if let Some(Ref::Local(b)) = self.res.occurrences.get(self.next) {
                let name = format!("{}{MARK}{b}", id.name);
                id.rename(name.as_str());
            }
            self.next += 1;
        }
    }
    walk(chunk, &mut Mark { res: &res, next: 0 });
    let n = res.bindings.len();
    Unique {
        next: n,
        first_new: n,
    }
}

impl Unique {
    /// A new local's name: `base` (a generated one gets a name of its own
    /// when the names come back).
    pub(crate) fn fresh(&mut self, base: &str) -> String {
        let name = format!("{base}{MARK}{}", self.next);
        self.next += 1;
        name
    }

    /// A new generated local, as an identifier.
    pub(crate) fn fresh_ident(&mut self, base: &str, pos: Pos) -> Ident {
        Ident::new_generated(self.fresh(base), pos)
    }

    /// Gives the locals their names back. Generated locals the passes added
    /// get the first `{base}`, `{base}2`, … no identifier of the chunk has.
    pub(crate) fn restore(self, chunk: &mut Chunk) {
        let mut taken: BTreeSet<String> = BTreeSet::new();
        visit_all(chunk, &mut |id: &mut Ident| {
            if !is_local(&id.name) || !new_generated(id, self.first_new) {
                taken.insert(base(&id.name).to_string());
            }
        });
        let mut chosen: std::collections::HashMap<String, String> = Default::default();
        visit_all(chunk, &mut |id: &mut Ident| {
            if !is_local(&id.name) {
                return;
            }
            let name = if new_generated(id, self.first_new) {
                let key = id.name.to_string();
                chosen
                    .entry(key)
                    .or_insert_with(|| {
                        let b = base(&id.name).to_string();
                        let mut n = 1;
                        loop {
                            let candidate = if n == 1 { b.clone() } else { format!("{b}{n}") };
                            if !taken.contains(&candidate) {
                                taken.insert(candidate.clone());
                                break candidate;
                            }
                            n += 1;
                        }
                    })
                    .clone()
            } else {
                base(&id.name).to_string()
            };
            id.rename(name.as_str());
        });
    }
}

fn new_generated(id: &Ident, first_new: usize) -> bool {
    id.generated()
        && id
            .name
            .rsplit(MARK)
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .is_some_and(|n| n >= first_new)
}

/// Every identifier of the chunk, declarations and the names of fields
/// included (only marked ones are renamed).
fn visit_all(chunk: &mut Chunk, f: &mut impl FnMut(&mut Ident)) {
    struct All<'a, F>(&'a mut F);
    impl<F: FnMut(&mut Ident)> Visitor for All<'_, F> {
        fn ident(&mut self, id: &mut Ident, _use: Use) {
            (self.0)(id);
        }
    }
    walk(chunk, &mut All(f));
}
