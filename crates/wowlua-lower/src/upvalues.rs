//! The upvalue limit (spec §5.5): imports are locals of the module chunk,
//! so functions capture them as upvalues, at most 60 per function in Lua
//! 5.1. A function that would need more reads the excess imports through
//! their module tables in the runtime's registry
//! (`__wl.modules["id"].name`), which costs the one upvalue `__wl`. If the
//! limit is still exceeded, the Lua 5.1 compile check reports E5001.

use std::collections::{BTreeSet, HashMap};
use wowlua_luair::ir::{self, Pos};
use wowlua_luair::scope::{self, Use, Visitor};

/// Lua 5.1's `LUAI_MAXUPVALUES`.
const MAX_UPVALUES: usize = 60;

/// An import's local: the module it reads and the export it holds (`None`:
/// the whole export table, `import * as M`).
#[derive(Clone, Debug)]
pub(crate) struct ImportLocal {
    pub name: String,
    pub module: String,
    pub export: Option<String>,
}

/// Rewrites reads of imports in functions over the upvalue limit.
pub(crate) fn limit(chunk: &mut ir::Chunk, imports: &[ImportLocal]) {
    if imports.is_empty() {
        return;
    }
    // Imports that cannot be read through the registry (a `function M.f()`
    // statement names its table): never chosen again.
    let mut fixed: BTreeSet<usize> = BTreeSet::new();
    loop {
        let res = scope::resolve(chunk);
        // The chunk's import locals, by binding: imports come first in a
        // module, so each is the chunk's first local of its name.
        let mut by_binding: HashMap<usize, &ImportLocal> = HashMap::new();
        for import in imports {
            if let Some(b) = res
                .bindings
                .iter()
                .position(|b| b.function == 0 && b.name == import.name)
            {
                by_binding.insert(b, import);
            }
        }
        let runtime = res
            .bindings
            .iter()
            .position(|b| b.function == 0 && b.name == "__wl");
        // The outermost function over the limit with imports to move.
        let found = res.functions.iter().enumerate().find_map(|(f, info)| {
            if info.upvalues.len() <= MAX_UPVALUES {
                return None;
            }
            let movable: Vec<usize> = info
                .upvalues
                .iter()
                .copied()
                .filter(|b| by_binding.contains_key(b) && !fixed.contains(b))
                .collect();
            (!movable.is_empty()).then_some((f, info, movable))
        });
        let Some((f, info, movable)) = found else {
            return;
        };
        let Some(runtime) = runtime else { return };
        let need =
            info.upvalues.len() - MAX_UPVALUES + usize::from(!info.upvalues.contains(&runtime));
        // The last imports first, so the earlier ones stay plain locals.
        let chosen: BTreeSet<usize> = movable.iter().rev().take(need).copied().collect();
        let mut rewrite = Rewrite {
            functions: &res.functions,
            target: f,
            chosen: &chosen,
            imports: &by_binding,
            stack: Vec::new(),
            blocks: Vec::new(),
            open: Vec::new(),
            next_binding: 0,
            next_function: 0,
            stuck: BTreeSet::new(),
        };
        scope::walk(chunk, &mut rewrite);
        let stuck = rewrite.stuck;
        fixed.extend(stuck.iter().copied());
        if stuck.len() == chosen.len() {
            // Nothing moved: no progress is possible for these.
            continue;
        }
    }
}

/// Replaces the reads of the chosen bindings inside function `target` (and
/// the functions nested in it) with registry reads. Resolves names as
/// [`scope::resolve`] does, so binding and function numbers agree.
struct Rewrite<'a> {
    functions: &'a [scope::FunctionInfo],
    target: usize,
    chosen: &'a BTreeSet<usize>,
    imports: &'a HashMap<usize, &'a ImportLocal>,
    /// Locals in scope (name, binding), innermost last.
    stack: Vec<(String, usize)>,
    blocks: Vec<usize>,
    /// Open functions, innermost last.
    open: Vec<usize>,
    next_binding: usize,
    next_function: usize,
    /// Chosen bindings with an occurrence that is not an expression.
    stuck: BTreeSet<usize>,
}

impl Rewrite<'_> {
    fn lookup(&self, name: &str) -> Option<usize> {
        self.stack
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|&(_, b)| b)
    }

    /// The current function is `target` or nested in it.
    fn inside(&self) -> bool {
        let mut f = self.open.last().copied();
        while let Some(x) = f {
            if x == self.target {
                return true;
            }
            f = self.functions[x].parent;
        }
        false
    }

    fn declare(&mut self, name: &str) {
        self.stack.push((name.to_string(), self.next_binding));
        self.next_binding += 1;
    }
}

impl Visitor for Rewrite<'_> {
    fn ident(&mut self, id: &mut ir::Ident, use_: Use) {
        match use_ {
            Use::Declare => self.declare(&id.name),
            Use::Read | Use::Write => {
                if let Some(b) = self.lookup(&id.name)
                    && self.chosen.contains(&b)
                    && self.inside()
                {
                    // Reached only where the name is not an expression (a
                    // rewritten read is no longer a name).
                    self.stuck.insert(b);
                }
            }
        }
    }

    fn implicit_self(&mut self) {
        self.declare("self");
    }

    fn enter_block(&mut self) {
        self.blocks.push(self.stack.len());
    }

    fn leave_block(&mut self) {
        let len = self.blocks.pop().expect("balanced blocks");
        self.stack.truncate(len);
    }

    fn enter_function(&mut self) {
        self.open.push(self.next_function);
        self.next_function += 1;
    }

    fn leave_function(&mut self) {
        self.open.pop();
    }

    fn expr(&mut self, e: &mut ir::Expr) {
        let ir::ExprKind::Name(id) = &e.kind else {
            return;
        };
        let Some(b) = self.lookup(&id.name) else {
            return;
        };
        if !self.chosen.contains(&b) || !self.inside() {
            return;
        }
        let import = self.imports[&b];
        *e = registry_read(import, e.pos);
    }
}

/// `__wl.modules["id"]`, or `__wl.modules["id"].name` for a named import.
fn registry_read(import: &ImportLocal, pos: Pos) -> ir::Expr {
    let wl = ir::Expr::new(
        ir::ExprKind::Name(ir::Ident::new_generated("__wl", pos)),
        pos,
    );
    let modules = ir::Expr::new(
        ir::ExprKind::Field {
            obj: Box::new(wl),
            name: ir::Ident::new("modules", Pos::NONE),
        },
        pos,
    );
    let id = wowlua_sema::constant::quote(import.module.as_bytes());
    let key = ir::Expr::new(ir::ExprKind::String(id.into()), pos);
    let table = ir::Expr::new(
        ir::ExprKind::Index {
            facts: Default::default(),
            obj: Box::new(modules),
            key: Box::new(key),
        },
        pos,
    );
    match &import.export {
        None => table,
        Some(name) => ir::Expr::new(
            ir::ExprKind::Field {
                obj: Box::new(table),
                name: ir::Ident::new(name.as_str(), Pos::NONE),
            },
            pos,
        ),
    }
}
