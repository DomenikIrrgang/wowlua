//! Minification passes (spec optimization §15.4). The passes that change the
//! IR run here, in a fixed order; the ones that change only spacing and
//! comments are emitter options.

mod dedupe;
mod fields;
pub mod literals;
pub mod rename;
mod simplify;

use crate::emit::{CommentMode, EmitOptions, LineMode};
use crate::ir::*;
use crate::scope::{Visitor, walk};

/// Active locals a pass may let the main chunk reach (Lua allows 200).
pub const LOCALS_BUDGET: usize = 180;
/// Upvalues a pass may let a function reach (Lua allows 60).
pub const UPVALUES_BUDGET: usize = 50;

/// A set of minification passes (spec §15.4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Passes {
    pub strip_comments: bool,
    pub strip_whitespace: bool,
    pub compact: bool,
    pub shorten_literals: bool,
    pub simplify: bool,
    pub rename_locals: bool,
    pub rename_private: bool,
    pub dedupe_strings: bool,
}

impl Passes {
    /// The passes named (unknown names are ignored: validation reports them).
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Passes {
        let mut p = Passes::default();
        for name in names {
            match name {
                "strip-comments" => p.strip_comments = true,
                "strip-whitespace" => p.strip_whitespace = true,
                "compact" => p.compact = true,
                "shorten-literals" => p.shorten_literals = true,
                "simplify" => p.simplify = true,
                "rename-locals" => p.rename_locals = true,
                "rename-private" => p.rename_private = true,
                "dedupe-strings" => p.dedupe_strings = true,
                _ => {}
            }
        }
        p
    }

    pub fn is_empty(&self) -> bool {
        *self == Passes::default()
    }

    /// The emitter options for these passes in `line_mode`.
    pub fn emit_options(&self, line_mode: LineMode) -> EmitOptions {
        let whitespace = self.strip_whitespace;
        EmitOptions {
            // Readable formatting would undo the whitespace passes.
            line_mode: if whitespace && line_mode == LineMode::Pretty {
                LineMode::Statement
            } else {
                line_mode
            },
            comments: if self.strip_comments {
                CommentMode::License
            } else {
                CommentMode::All
            },
            indent: if whitespace {
                String::new()
            } else {
                "\t".into()
            },
            compact: self.compact,
        }
    }
}

/// Runs the IR passes of `passes` on `chunk`, in the compiler's order.
/// Returns the original names of renamed locals by source position (for
/// source maps).
pub fn apply(
    chunk: &mut Chunk,
    passes: &Passes,
    line_mode: LineMode,
) -> std::collections::HashMap<Pos, String> {
    if passes.simplify {
        simplify::apply(chunk, line_mode == LineMode::Preserve);
    }
    if passes.shorten_literals {
        shorten_literals(chunk, line_mode == LineMode::Preserve);
    }
    if passes.dedupe_strings {
        dedupe::apply(chunk, passes.rename_locals);
    }
    if passes.rename_private {
        fields::apply(chunk);
    }
    if passes.rename_locals {
        return rename::apply(chunk);
    }
    std::collections::HashMap::new()
}

/// `preserve_lines`: a literal spanning lines keeps them, as Lua gives the
/// instruction loading it the line the literal ends on.
fn shorten_literals(chunk: &mut Chunk, preserve_lines: bool) {
    struct Shorten(bool);
    impl Visitor for Shorten {
        fn expr(&mut self, e: &mut Expr) {
            match &mut e.kind {
                ExprKind::Number(n) => {
                    if let Some(short) = literals::shorten_number(n) {
                        *n = short;
                    }
                }
                ExprKind::String(raw) => {
                    if self.0 && raw.iter().any(|&c| c == b'\n' || c == b'\r') {
                        return;
                    }
                    if let Some(short) = literals::shorten_string(raw) {
                        *raw = short;
                    }
                }
                _ => {}
            }
        }
    }
    walk(chunk, &mut Shorten(preserve_lines));
}

/// The class half of `rename-private`: member names (`o.name`, `o:name()`,
/// `function C:name()`, `function C.name()`) at the given source positions
/// get their new names (`at` maps a position to the old and the new name).
/// Returns the positions found; with `apply` false nothing changes.
pub fn rename_members(
    chunk: &mut Chunk,
    at: &std::collections::HashMap<Pos, (String, String)>,
    apply: bool,
) -> Vec<Pos> {
    struct Members<'a> {
        at: &'a std::collections::HashMap<Pos, (String, String)>,
        apply: bool,
        found: Vec<Pos>,
    }
    impl Members<'_> {
        fn member(&mut self, id: &mut Ident) {
            if let Some((old, new)) = self.at.get(&id.pos)
                && id.name == *old
            {
                self.found.push(id.pos);
                if self.apply {
                    id.name = new.clone();
                }
            }
        }
    }
    impl Visitor for Members<'_> {
        fn expr(&mut self, e: &mut Expr) {
            if let ExprKind::Field { name, .. } | ExprKind::Method { name, .. } = &mut e.kind {
                self.member(name);
            }
        }
        fn stmt(&mut self, s: &mut Stmt) {
            if let StmtKind::Function { path, method, .. } = &mut s.kind {
                for part in path.iter_mut().skip(1) {
                    self.member(part);
                }
                if let Some(m) = method {
                    self.member(m);
                }
            }
        }
    }
    let mut m = Members {
        at,
        apply,
        found: Vec::new(),
    };
    walk(chunk, &mut m);
    m.found
}
