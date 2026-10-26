//! Which globals a chunk may change (spec optimization §15.2.3): a rewrite
//! of `ipairs`, `table.insert` or a call of `pcall` relies on the global
//! being the library's, so it is not done when the chunk assigns the name
//! (or the library table's field), or reaches other environments.

use super::names::is_local;
use super::walk::*;
use crate::ir::*;
use std::collections::BTreeSet;

#[derive(Default)]
pub(crate) struct Globals {
    /// The chunk uses `setfenv`, `getfenv`, `loadstring` or `_G`.
    dynamic: bool,
    written: BTreeSet<String>,
    /// `lib.field` assigned (`*`: any field, through an index).
    fields: BTreeSet<(String, String)>,
}

impl Globals {
    /// Of a chunk with unique names (`names::uniquify`).
    pub(crate) fn of(chunk: &mut Chunk) -> Globals {
        let mut g = Globals::default();
        let mut b = std::mem::take(&mut chunk.block);
        each_name_in_block(&mut b, &mut |id| {
            if !is_local(&id.name)
                && matches!(
                    id.name.as_str(),
                    "setfenv" | "getfenv" | "loadstring" | "_G"
                )
            {
                g.dynamic = true;
            }
        });
        stmts(&mut b, &mut g);
        chunk.block = b;
        g
    }

    /// The global `name` is the one the client defines.
    pub(crate) fn stable(&self, name: &str) -> bool {
        !self.dynamic && !self.written.contains(name)
    }

    /// `lib.field` is the library's.
    pub(crate) fn stable_field(&self, lib: &str, field: &str) -> bool {
        self.stable(lib)
            && !self.fields.contains(&(lib.to_string(), field.to_string()))
            && !self.fields.contains(&(lib.to_string(), "*".to_string()))
    }
}

fn target(e: &Expr, g: &mut Globals) {
    match &e.kind {
        ExprKind::Name(id) if !is_local(&id.name) => {
            g.written.insert(id.name.to_string());
        }
        ExprKind::Field { obj, name } => {
            if let ExprKind::Name(lib) = &obj.kind
                && !is_local(&lib.name)
            {
                g.fields
                    .insert((lib.name.to_string(), name.name.to_string()));
            }
        }
        ExprKind::Index { obj, .. } => {
            if let ExprKind::Name(lib) = &obj.kind
                && !is_local(&lib.name)
            {
                g.fields.insert((lib.name.to_string(), "*".to_string()));
            }
        }
        _ => {}
    }
}

fn stmts(b: &mut Block, g: &mut Globals) {
    for s in &mut b.stmts {
        match &s.kind {
            StmtKind::Assign { targets, .. } => {
                for t in targets {
                    target(t, g);
                }
            }
            StmtKind::Function { path, method, .. } => {
                let first = &path[0];
                if !is_local(&first.name) {
                    if path.len() == 1 && method.is_none() {
                        g.written.insert(first.name.to_string());
                    } else {
                        let field = path
                            .get(1)
                            .or(method.as_ref())
                            .map(|f| f.name.to_string())
                            .unwrap_or_default();
                        g.fields.insert((first.name.to_string(), field));
                    }
                }
            }
            _ => {}
        }
        for e in own_exprs(s) {
            each_expr(e, false, &mut |e| {
                if let ExprKind::Function(f) = &mut e.kind {
                    stmts(&mut f.body, g);
                }
            });
        }
        for inner in inner_blocks(s) {
            stmts(inner, g);
        }
        if let Some(f) = declared_function(s) {
            stmts(&mut f.body, g);
        }
    }
}
