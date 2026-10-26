//! Scope resolution on the IR: which local each name refers to, which locals
//! must keep different names, and the locals and upvalues of every function
//! (for the Lua-limit budgets of spec optimization §15.2.2).

use crate::ir::*;
use std::collections::{BTreeSet, HashMap};

/// How an identifier occurs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    /// Declares a local (or parameter, loop variable).
    Declare,
    /// Reads a variable.
    Read,
    /// Assigns a variable.
    Write,
}

/// Callbacks of [`walk`], in Lua's scoping order.
pub trait Visitor {
    fn ident(&mut self, _id: &mut Ident, _use: Use) {}
    /// The implicit `self` parameter of a method.
    fn implicit_self(&mut self) {}
    fn enter_block(&mut self) {}
    fn leave_block(&mut self) {}
    fn enter_function(&mut self) {}
    fn leave_function(&mut self) {}
    /// Every expression, before its parts.
    fn expr(&mut self, _e: &mut Expr) {}
    /// Every statement, before its parts.
    fn stmt(&mut self, _s: &mut Stmt) {}
}

fn grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

/// Walks the main chunk: a function (with `...`) whose body is the block.
pub fn walk(chunk: &mut Chunk, v: &mut impl Visitor) {
    v.enter_function();
    v.enter_block();
    stmts(&mut chunk.block, v);
    v.leave_block();
    v.leave_function();
}

/// Walks one expression (outside any scope).
pub fn walk_expr(e: &mut Expr, v: &mut impl Visitor) {
    expr(e, v);
}

/// The identifier occurrences [`walk`] visits in `e`.
pub fn ident_count(e: &mut Expr) -> usize {
    struct Count(usize);
    impl Visitor for Count {
        fn ident(&mut self, _id: &mut Ident, _use: Use) {
            self.0 += 1;
        }
    }
    let mut c = Count(0);
    expr(e, &mut c);
    c.0
}

/// The identifier occurrences [`walk`] visits in statement `s`.
pub fn stmt_ident_count(s: &mut Stmt) -> usize {
    struct Count(usize);
    impl Visitor for Count {
        fn ident(&mut self, _id: &mut Ident, _use: Use) {
            self.0 += 1;
        }
    }
    let mut c = Count(0);
    stmt(s, &mut c);
    c.0
}

fn block(b: &mut Block, v: &mut impl Visitor) {
    v.enter_block();
    stmts(b, v);
    v.leave_block();
}

fn stmts(b: &mut Block, v: &mut impl Visitor) {
    for s in &mut b.stmts {
        grow(|| stmt(s, v));
    }
}

fn stmt(s: &mut Stmt, v: &mut impl Visitor) {
    v.stmt(s);
    match &mut s.kind {
        StmtKind::Local { names, values } => {
            values.iter_mut().for_each(|e| expr(e, v));
            names.iter_mut().for_each(|n| v.ident(n, Use::Declare));
        }
        StmtKind::Assign { targets, values } => {
            for t in targets.iter_mut() {
                v.expr(t);
                match &mut t.kind {
                    ExprKind::Name(id) => v.ident(id, Use::Write),
                    _ => expr_parts(t, v),
                }
            }
            values.iter_mut().for_each(|e| expr(e, v));
        }
        StmtKind::Call(e) => expr(e, v),
        StmtKind::Do(b) => block(b, v),
        StmtKind::While { cond, body, .. } => {
            expr(cond, v);
            block(body, v);
        }
        StmtKind::Repeat { body, cond } => {
            // The condition sees the body's locals.
            v.enter_block();
            stmts(body, v);
            expr(cond, v);
            v.leave_block();
        }
        StmtKind::If {
            clauses,
            else_block,
        } => {
            for c in clauses {
                expr(&mut c.cond, v);
                block(&mut c.body, v);
            }
            if let Some(b) = else_block {
                block(b, v);
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
            expr(start, v);
            expr(limit, v);
            if let Some(s) = step {
                expr(s, v);
            }
            v.enter_block();
            v.ident(var, Use::Declare);
            stmts(body, v);
            v.leave_block();
        }
        StmtKind::GenericFor {
            vars, exprs, body, ..
        } => {
            exprs.iter_mut().for_each(|e| expr(e, v));
            v.enter_block();
            vars.iter_mut().for_each(|n| v.ident(n, Use::Declare));
            stmts(body, v);
            v.leave_block();
        }
        StmtKind::Function { path, method, func } => {
            // `function a()` assigns `a`; `function a.b()` reads it.
            let write = path.len() == 1 && method.is_none();
            if let Some(first) = path.first_mut() {
                v.ident(first, if write { Use::Write } else { Use::Read });
            }
            function(func, method.is_some(), v);
        }
        StmtKind::LocalFunction { name, func } => {
            v.ident(name, Use::Declare);
            function(func, false, v);
        }
        StmtKind::Return(values) => values.iter_mut().for_each(|e| expr(e, v)),
        StmtKind::Break => {}
    }
}

fn function(f: &mut Func, method: bool, v: &mut impl Visitor) {
    v.enter_function();
    v.enter_block();
    if method {
        v.implicit_self();
    }
    f.params.iter_mut().for_each(|p| v.ident(p, Use::Declare));
    stmts(&mut f.body, v);
    v.leave_block();
    v.leave_function();
}

fn expr(e: &mut Expr, v: &mut impl Visitor) {
    grow(|| {
        v.expr(e);
        expr_parts(e, v);
    })
}

fn args(a: &mut Args, v: &mut impl Visitor) {
    match a {
        Args::Parens { args, .. } => args.iter_mut().for_each(|e| expr(e, v)),
        Args::Table(e) | Args::String(e) => expr(e, v),
    }
}

fn expr_parts(e: &mut Expr, v: &mut impl Visitor) {
    match &mut e.kind {
        ExprKind::Name(id) => v.ident(id, Use::Read),
        ExprKind::Function(f) => function(f, false, v),
        ExprKind::Table { fields, .. } => {
            for field in fields {
                match field {
                    Field::Keyed { key, value, .. } => {
                        expr(key, v);
                        expr(value, v);
                    }
                    Field::Named { value, .. } | Field::Positional(value) => expr(value, v),
                }
            }
        }
        ExprKind::Index { obj, key, .. } => {
            expr(obj, v);
            expr(key, v);
        }
        ExprKind::Field { obj, .. } => expr(obj, v),
        ExprKind::Call { callee, args: a } => {
            expr(callee, v);
            args(a, v);
        }
        ExprKind::Method { obj, args: a, .. } => {
            expr(obj, v);
            args(a, v);
        }
        ExprKind::Paren { inner, .. } => expr(inner, v),
        ExprKind::Binary { lhs, rhs, .. } => {
            expr(lhs, v);
            expr(rhs, v);
        }
        ExprKind::Unary { operand, .. } => expr(operand, v),
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Vararg => {}
    }
}

/// What an identifier occurrence refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ref {
    Local(usize),
    Global,
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub name: String,
    /// The implicit `self` of a method: it has no identifier to rename.
    pub implicit: bool,
    /// The function that declares it (0 is the main chunk).
    pub function: usize,
    /// Reads and writes, not counting the declaration.
    pub uses: u32,
    pub writes: u32,
}

#[derive(Clone, Debug, Default)]
pub struct FunctionInfo {
    pub parent: Option<usize>,
    /// Locals of enclosing functions it uses (directly or through a nested
    /// function).
    pub upvalues: BTreeSet<usize>,
    /// The most locals active at once.
    pub max_locals: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Resolution {
    pub bindings: Vec<Binding>,
    /// Every identifier occurrence that is a variable, in walk order.
    pub occurrences: Vec<Ref>,
    /// Pairs of locals that must keep different names: one is used where
    /// the other is in scope. Only [`resolve_with_conflicts`] fills it;
    /// [`resolve`] leaves it empty.
    pub conflicts: Vec<BTreeSet<usize>>,
    /// Names read or written as globals.
    pub globals: BTreeSet<String>,
    pub functions: Vec<FunctionInfo>,
}

#[derive(Default)]
struct Resolver {
    res: Resolution,
    /// Locals in scope, innermost last.
    stack: Vec<usize>,
    blocks: Vec<usize>,
    functions: Vec<usize>,
    /// Active locals per open function.
    active: Vec<usize>,
    /// Conflicts are recorded.
    conflicts: bool,
    /// Per binding: the latest later local its conflicts were recorded
    /// with. `stack` holds bindings in declaration order, so the later
    /// locals of a use not above it were recorded at an earlier use.
    recorded: Vec<usize>,
}

impl Resolver {
    fn declare(&mut self, name: &str, implicit: bool) -> usize {
        let id = self.res.bindings.len();
        let function = *self.functions.last().expect("inside a function");
        self.res.bindings.push(Binding {
            name: name.to_string(),
            implicit,
            function,
            uses: 0,
            writes: 0,
        });
        if self.conflicts {
            self.res.conflicts.push(BTreeSet::new());
            self.recorded.push(id);
        }
        debug_assert!(self.stack.last().is_none_or(|&top| top < id));
        self.stack.push(id);
        if let Some(a) = self.active.last_mut() {
            *a += 1;
            let info = &mut self.res.functions[function];
            info.max_locals = info.max_locals.max(*a);
        }
        id
    }
}

impl Visitor for Resolver {
    fn ident(&mut self, id: &mut Ident, use_: Use) {
        if use_ == Use::Declare {
            let b = self.declare(&id.name, false);
            self.res.occurrences.push(Ref::Local(b));
            return;
        }
        let found = self
            .stack
            .iter()
            .rposition(|&b| self.res.bindings[b].name == id.name);
        let Some(index) = found else {
            if !self.res.globals.contains(id.name.as_str()) {
                self.res.globals.insert(id.name.to_string());
            }
            self.res.occurrences.push(Ref::Global);
            return;
        };
        let b = self.stack[index];
        let binding = &mut self.res.bindings[b];
        binding.uses += 1;
        if use_ == Use::Write {
            binding.writes += 1;
        }
        let owner = binding.function;
        // Every function between the use and the declaration captures it.
        for &f in self.functions.iter().rev() {
            if f == owner {
                break;
            }
            self.res.functions[f].upvalues.insert(b);
        }
        // Locals declared after it and still in scope must not take its name.
        if self.conflicts {
            let later = &self.stack[index + 1..];
            let from = later.partition_point(|&l| l <= self.recorded[b]);
            for &l in &later[from..] {
                self.res.conflicts[b].insert(l);
                self.res.conflicts[l].insert(b);
            }
            if let Some(&top) = later.last() {
                self.recorded[b] = self.recorded[b].max(top);
            }
        }
        self.res.occurrences.push(Ref::Local(b));
    }

    fn implicit_self(&mut self) {
        self.declare("self", true);
    }

    fn enter_block(&mut self) {
        self.blocks.push(self.stack.len());
    }

    fn leave_block(&mut self) {
        let len = self.blocks.pop().expect("balanced blocks");
        let left = self.stack.len() - len;
        self.stack.truncate(len);
        if let Some(a) = self.active.last_mut() {
            *a -= left;
        }
    }

    fn enter_function(&mut self) {
        let id = self.res.functions.len();
        self.res.functions.push(FunctionInfo {
            parent: self.functions.last().copied(),
            ..FunctionInfo::default()
        });
        self.functions.push(id);
        self.active.push(0);
    }

    fn leave_function(&mut self) {
        self.functions.pop();
        self.active.pop();
    }
}

/// Resolves every variable of `chunk`, without the conflicts.
pub fn resolve(chunk: &mut Chunk) -> Resolution {
    let mut r = Resolver::default();
    walk(chunk, &mut r);
    r.res
}

/// [`resolve`] with [`Resolution::conflicts`], for renaming locals.
pub fn resolve_with_conflicts(chunk: &mut Chunk) -> Resolution {
    let mut r = Resolver {
        conflicts: true,
        ..Resolver::default()
    };
    walk(chunk, &mut r);
    r.res
}

/// Renames the locals of `chunk` (by binding) and leaves globals alone.
/// Returns the original names of the renamed identifiers by position.
pub fn rename(
    chunk: &mut Chunk,
    res: &Resolution,
    names: &HashMap<usize, String>,
) -> HashMap<Pos, String> {
    struct Renamer<'a> {
        res: &'a Resolution,
        names: &'a HashMap<usize, String>,
        next: usize,
        original: HashMap<Pos, String>,
    }
    impl Visitor for Renamer<'_> {
        fn ident(&mut self, id: &mut Ident, _use: Use) {
            if let Some(Ref::Local(b)) = self.res.occurrences.get(self.next)
                && let Some(name) = self.names.get(b)
                && *name != id.name
            {
                if id.pos.is_some() && !id.generated() {
                    self.original
                        .insert(id.pos, std::mem::take(&mut id.name).into());
                }
                id.rename(name.as_str());
            }
            self.next += 1;
        }
    }
    let mut r = Renamer {
        res,
        names,
        next: 0,
        original: HashMap::new(),
    };
    walk(chunk, &mut r);
    r.original
}
