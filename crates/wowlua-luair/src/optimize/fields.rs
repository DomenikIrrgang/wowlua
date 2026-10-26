//! `optimize.cache-fields` (spec optimization §15.2.3): field reads of
//! plain tables (`self.frame.texture`, where the checker proved every
//! object of the chain a plain table) are read once into a local:
//!
//! - out of a loop, when nothing in the loop can change them (a loop that
//!   runs no code of its own and assigns none of the chain's fields or its
//!   local), and the loop's first read of them is where the loop starts;
//! - in a stretch of code, when they are read again before anything that
//!   could change them.
//!
//! Code that could change a field is a call (but of the standard library
//! functions that run no code and assign no named field), a read or a write
//! that may run a metamethod, an assignment to the field's name (of any
//! table) or to an unknown key, and an assignment to the chain's local.
//! Runs on a chunk with unique names (`names::uniquify`), function by
//! function.

use super::globals::Globals;
use super::loops::functions;
use super::names::{Unique, is_local};
use super::walk::*;
use crate::ir::*;
use crate::minify::LOCALS_BUDGET;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A chain: the local, then the fields.
type Path = Vec<String>;

/// The chain an expression reads: a local and at least one plain field.
fn path_of(e: &Expr) -> Option<Path> {
    match &e.kind {
        ExprKind::Field { obj, name } if name.facts().has(Facts::PLAIN) => {
            let mut p = match &obj.kind {
                ExprKind::Name(id) if is_local(&id.name) => vec![id.name.to_string()],
                _ => path_of(obj)?,
            };
            p.push(name.name.to_string());
            Some(p)
        }
        _ => None,
    }
}

/// The sub-expression of a chain read that reads its first `len` parts
/// (the local and `len - 1` fields).
fn prefix_mut(e: &mut Expr, total: usize, len: usize) -> &mut Expr {
    let mut cur = e;
    for _ in len..total {
        cur = match &mut cur.kind {
            ExprKind::Field { obj, .. } => obj.as_mut(),
            _ => unreachable!("a chain"),
        };
    }
    cur
}

/// A key that is a number: a numeral, arithmetic and `#` that run no
/// metamethod, a numeric loop's control variable. It cannot name a field.
fn is_number(key: &Expr, numbers: &BTreeSet<String>) -> bool {
    match &key.kind {
        ExprKind::Number(_) => true,
        ExprKind::Name(id) => numbers.contains(id.name.as_str()),
        ExprKind::Paren { inner, .. } => is_number(inner, numbers),
        ExprKind::Binary { op, facts, .. } => {
            facts.has(Facts::NO_META)
                && matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow
                )
        }
        ExprKind::Unary { op, facts, .. } => {
            facts.has(Facts::NO_META) && matches!(op, UnOp::Len | UnOp::Neg)
        }
        _ => false,
    }
}

/// Library functions that run no code (no callbacks, no metamethods) and
/// assign no named field: a call of one does not end a stretch.
fn pure_call(callee: &Expr, g: &Globals) -> bool {
    const GLOBALS: &[&str] = &[
        "type",
        "tonumber",
        "select",
        "rawget",
        "rawequal",
        "next",
        "unpack",
        "ipairs",
        "pairs",
        "getmetatable",
        "abs",
        "ceil",
        "floor",
        "max",
        "min",
        "sqrt",
        "mod",
        "exp",
        "log",
        "log10",
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "atan2",
        "deg",
        "rad",
        "random",
        "frexp",
        "ldexp",
        "format",
        "strbyte",
        "strchar",
        "strfind",
        "strlen",
        "strlower",
        "strmatch",
        "strrep",
        "strrev",
        "strsub",
        "strupper",
        "strtrim",
        "strsplit",
        "strjoin",
        "tinsert",
        "tremove",
        "getn",
    ];
    const FIELDS: &[(&str, &[&str])] = &[
        (
            "math",
            &[
                "abs", "acos", "asin", "atan", "atan2", "ceil", "cos", "cosh", "deg", "exp",
                "floor", "fmod", "frexp", "ldexp", "log", "log10", "max", "min", "modf", "pow",
                "rad", "random", "sin", "sinh", "sqrt", "tan", "tanh",
            ],
        ),
        (
            "string",
            &[
                "byte", "char", "find", "format", "len", "lower", "match", "rep", "reverse", "sub",
                "upper",
            ],
        ),
        ("table", &["concat", "insert", "remove", "getn", "maxn"]),
        (
            "bit",
            &[
                "band", "bor", "bxor", "bnot", "lshift", "rshift", "arshift", "mod",
            ],
        ),
    ];
    match &callee.kind {
        ExprKind::Name(f) => {
            !is_local(&f.name) && GLOBALS.contains(&f.name.as_str()) && g.stable(&f.name)
        }
        ExprKind::Field { obj, name } => match &obj.kind {
            ExprKind::Name(lib) if !is_local(&lib.name) => {
                FIELDS
                    .iter()
                    .any(|(l, fs)| *l == lib.name.as_str() && fs.contains(&name.name.as_str()))
                    && g.stable_field(&lib.name, &name.name)
            }
            _ => false,
        },
        _ => false,
    }
}

/// What a stretch of code may change.
#[derive(Clone, Debug, Default)]
struct Kills {
    /// It may run code: anything may change.
    all: bool,
    fields: BTreeSet<String>,
    locals: BTreeSet<String>,
}

impl Kills {
    fn kills(&self, p: &Path) -> bool {
        self.all || self.locals.contains(&p[0]) || p[1..].iter().any(|f| self.fields.contains(f))
    }
}

/// A chain read once into a local.
struct Group {
    /// The chain's read at its first place (a prefix of an event's
    /// expression), before any rewrite.
    expr: Expr,
    /// The block and statement before which the local is declared.
    anchor: (usize, usize),
    /// Reads that may use it.
    reads: u32,
    /// Made before a loop: one read in it is worth it.
    hoisted: bool,
    local: Option<Ident>,
}

/// One chain read: the group of each of its prefixes (by length − 1), and
/// the one it reads through.
struct Event {
    groups: Vec<Option<usize>>,
    choice: Option<(usize, usize)>,
}

type Avail = BTreeMap<Path, usize>;

struct Pass<'g> {
    g: &'g Globals,
    /// The control variables of numeric loops: numbers.
    numbers: BTreeSet<String>,
    apply: bool,
    groups: Vec<Group>,
    events: Vec<Event>,
    next_event: usize,
    next_block: usize,
    /// The current block and statement.
    at: (usize, usize),
    /// Code ran in the current statement before this point.
    dirty: bool,
    /// Apply: anchors to insert, by block.
    inserts: HashMap<usize, Vec<(usize, usize)>>,
}

impl Pass<'_> {
    fn kill(&self, avail: &mut Avail, k: &Kills) {
        avail.retain(|p, _| !k.kills(p));
    }

    /// A chain read in evaluation order: its prefixes, shortest first.
    fn event(&mut self, e: &mut Expr, path: Path, conditional: bool, avail: &mut Avail) {
        let id = self.next_event;
        self.next_event += 1;
        if self.apply {
            if let Some((len, g)) = self.events[id].choice {
                let local = self.groups[g]
                    .local
                    .clone()
                    .expect("a chosen group has a local");
                let target = prefix_mut(e, path.len(), len);
                let pos = target.pos;
                let sep = target.sep;
                *target = Expr::new(ExprKind::Name(Ident { pos, ..local }), pos);
                target.sep = sep;
            }
            return;
        }
        let mut groups = Vec::with_capacity(path.len() - 1);
        for len in 2..=path.len() {
            let p: Path = path[..len].to_vec();
            if let Some(&g) = avail.get(&p) {
                self.groups[g].reads += 1;
                groups.push(Some(g));
            } else if !conditional && !self.dirty {
                let mut expr = prefix_mut(e, path.len(), len).clone();
                expr.sep = Pos::NONE;
                self.groups.push(Group {
                    expr,
                    anchor: self.at,
                    reads: 1,
                    hoisted: false,
                    local: None,
                });
                let g = self.groups.len() - 1;
                avail.insert(p, g);
                groups.push(Some(g));
            } else {
                groups.push(None);
            }
        }
        self.events.push(Event {
            groups,
            choice: None,
        });
    }

    /// Code runs here: every chain may change.
    fn barrier(&mut self, avail: &mut Avail) {
        avail.clear();
        self.dirty = true;
    }

    fn expr(&mut self, e: &mut Expr, conditional: bool, avail: &mut Avail) {
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
            if let Some(path) = path_of(e) {
                self.event(e, path, conditional, avail);
                return;
            }
            match &mut e.kind {
                ExprKind::Function(_) => {}
                ExprKind::Binary {
                    op: BinOp::And | BinOp::Or,
                    lhs,
                    rhs,
                    ..
                } => {
                    self.expr(lhs, conditional, avail);
                    self.expr(rhs, true, avail);
                }
                ExprKind::Call { callee, args } => {
                    let pure = pure_call(callee, self.g);
                    self.expr(callee, conditional, avail);
                    for a in args_mut(args) {
                        self.expr(a, conditional, avail);
                    }
                    if !pure {
                        self.barrier(avail);
                    }
                }
                ExprKind::Method { obj, args, .. } => {
                    self.expr(obj, conditional, avail);
                    for a in args_mut(args) {
                        self.expr(a, conditional, avail);
                    }
                    self.barrier(avail);
                }
                ExprKind::Field { obj, name } => {
                    let plain = name.facts().has(Facts::PLAIN);
                    self.expr(obj, conditional, avail);
                    if !plain {
                        self.barrier(avail);
                    }
                }
                ExprKind::Index { obj, key, facts } => {
                    let plain = facts.has(Facts::PLAIN);
                    self.expr(obj, conditional, avail);
                    self.expr(key, conditional, avail);
                    if !plain {
                        self.barrier(avail);
                    }
                }
                ExprKind::Binary {
                    facts, lhs, rhs, ..
                } => {
                    let plain = facts.has(Facts::NO_META);
                    self.expr(lhs, conditional, avail);
                    self.expr(rhs, conditional, avail);
                    if !plain {
                        self.barrier(avail);
                    }
                }
                ExprKind::Unary { facts, operand, .. } => {
                    let plain = facts.has(Facts::NO_META);
                    self.expr(operand, conditional, avail);
                    if !plain {
                        self.barrier(avail);
                    }
                }
                _ => {
                    for p in parts(e) {
                        self.expr(p, conditional, avail);
                    }
                }
            }
        })
    }

    /// What an assignment target changes, after the statement's
    /// expressions ran. `numbers`: the control variables of numeric
    /// loops.
    fn target_kills(t: &Expr, numbers: &BTreeSet<String>, k: &mut Kills) {
        match &t.kind {
            ExprKind::Name(id) if is_local(&id.name) => {
                k.locals.insert(id.name.to_string());
            }
            ExprKind::Name(_) => {}
            ExprKind::Field { name, .. } => {
                if name.facts().has(Facts::PLAIN) {
                    k.fields.insert(name.name.to_string());
                } else {
                    k.all = true;
                }
            }
            ExprKind::Index { key, facts, .. } => {
                if !facts.has(Facts::PLAIN) {
                    k.all = true;
                } else {
                    match &key.kind {
                        _ if is_number(key, numbers) => {}
                        ExprKind::String(s) => {
                            // A quoted name is the field it names.
                            let text = String::from_utf8_lossy(s);
                            let inner = text.trim_matches(|c| c == '"' || c == '\'');
                            k.fields.insert(inner.to_string());
                        }
                        _ => k.all = true,
                    }
                }
            }
            _ => k.all = true,
        }
    }

    /// The writes of statement `s` itself (its targets).
    fn writes(s: &Stmt, numbers: &BTreeSet<String>) -> Kills {
        let mut k = Kills::default();
        match &s.kind {
            StmtKind::Assign { targets, .. } => {
                for t in targets {
                    Self::target_kills(t, numbers, &mut k);
                }
            }
            StmtKind::Function { path, method, .. } => {
                if path.len() == 1 && method.is_none() {
                    if is_local(&path[0].name) {
                        k.locals.insert(path[0].name.to_string());
                    }
                } else {
                    // `function a.b.c()` / `function a:m()`: a field write
                    // on a table that may have a metamethod.
                    k.all = true;
                }
            }
            _ => {}
        }
        k
    }

    fn block(&mut self, b: &mut Block, mut avail: Avail) -> Avail {
        let id = self.next_block;
        self.next_block += 1;
        let saved = self.at;
        let mut i = 0;
        while i < b.stmts.len() {
            self.at = (id, i);
            self.dirty = false;
            self.stmt(&mut b.stmts[i], &mut avail);
            if matches!(b.stmts[i].kind, StmtKind::Return(_) | StmtKind::Break) {
                avail.clear();
            }
            i += 1;
        }
        self.at = saved;
        if self.apply
            && let Some(mut anchors) = self.inserts.remove(&id)
        {
            // Later statements first, so the indexes stay right; at one
            // statement, in the order the groups were made.
            anchors.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
            for (index, g) in anchors {
                let group = &self.groups[g];
                let local = group.local.clone().expect("chosen");
                let pos = b.stmts.get(index).map_or(Pos::NONE, |s| s.pos);
                // On the line of the statement it is read for.
                let mut value = group.expr.clone();
                super::clear_positions(&mut value);
                value.pos = pos;
                b.stmts.insert(
                    index,
                    Stmt {
                        pos,
                        kind: StmtKind::Local {
                            names: vec![Ident { pos, ..local }],
                            values: vec![value],
                        },
                    },
                );
            }
        }
        avail
    }

    /// Processes an inner block that may run or not: the chains available
    /// after it are those available before that it changes nowhere; `None`
    /// when it never ends normally (it ends with `break` or `return`).
    fn inner(&mut self, b: &mut Block, avail: &Avail) -> Option<Avail> {
        let out = self.block(b, avail.clone());
        if matches!(
            b.stmts.last().map(|s| &s.kind),
            Some(StmtKind::Return(_) | StmtKind::Break)
        ) {
            return None;
        }
        Some(
            avail
                .iter()
                .filter(|(p, g)| out.get(*p) == Some(*g))
                .map(|(p, g)| (p.clone(), *g))
                .collect(),
        )
    }

    fn stmt(&mut self, s: &mut Stmt, avail: &mut Avail) {
        if is_loop(s) {
            self.looping(s, avail);
            return;
        }
        match &mut s.kind {
            StmtKind::If {
                clauses,
                else_block,
            } => {
                let mut after: Option<Avail> = None;
                // (a branch that leaves adds nothing)
                let meet = |a: Option<Avail>, after: &mut Option<Avail>| {
                    let Some(a) = a else { return };
                    *after = Some(match after.take() {
                        None => a,
                        Some(prev) => prev
                            .into_iter()
                            .filter(|(p, g)| a.get(p) == Some(g))
                            .collect(),
                    });
                };
                for (n, c) in clauses.iter_mut().enumerate() {
                    self.expr(&mut c.cond, n > 0, avail);
                    let out = self.inner(&mut c.body, avail);
                    meet(out, &mut after);
                }
                match else_block {
                    Some(b) => {
                        let out = self.inner(b, avail);
                        meet(out, &mut after);
                    }
                    None => meet(Some(avail.clone()), &mut after),
                }
                *avail = after.unwrap_or_default();
            }
            StmtKind::Do(b) => {
                *avail = self.inner(b, avail).unwrap_or_default();
            }
            StmtKind::Function { .. } | StmtKind::LocalFunction { .. } => {
                let k = Self::writes(s, &self.numbers);
                self.kill(avail, &k);
            }
            StmtKind::Assign { targets, values } => {
                // Lua evaluates the targets' objects and keys, then the
                // values, then assigns.
                for t in targets.iter_mut() {
                    for p in target_parts(t) {
                        self.expr(p, false, avail);
                    }
                }
                for v in values.iter_mut() {
                    self.expr(v, false, avail);
                }
                let k = Self::writes(s, &self.numbers);
                self.kill(avail, &k);
            }
            _ => {
                for e in own_exprs(s) {
                    self.expr(e, false, avail);
                }
            }
        }
    }

    /// A loop: the chains it cannot change and reads from its start are
    /// read before it; the others are cached per stretch inside.
    fn looping(&mut self, s: &mut Stmt, avail: &mut Avail) {
        // Once before the loop: a `for`'s header.
        let header: Vec<&mut Expr> = match &mut s.kind {
            StmtKind::NumericFor {
                start, limit, step, ..
            } => {
                let mut v: Vec<&mut Expr> = vec![start.as_mut(), limit.as_mut()];
                if let Some(st) = step {
                    v.push(st.as_mut());
                }
                v
            }
            StmtKind::GenericFor { exprs, .. } => exprs.iter_mut().collect(),
            _ => Vec::new(),
        };
        for e in header {
            self.expr(e, false, avail);
        }
        let summary = summarize(s, self.g, &self.numbers);
        let mut inside: Avail = avail
            .iter()
            .filter(|(p, _)| !summary.kills.kills(p))
            .map(|(p, g)| (p.clone(), *g))
            .collect();
        if !self.apply && !self.dirty {
            for (p, (expr, _)) in &summary.anticipated {
                if summary.kills.kills(p)
                    || inside.contains_key(p)
                    || summary.declared.contains(&p[0])
                {
                    continue;
                }
                let mut expr = expr.clone();
                expr.sep = Pos::NONE;
                self.groups.push(Group {
                    expr,
                    anchor: self.at,
                    reads: 0,
                    hoisted: true,
                    local: None,
                });
                inside.insert(p.clone(), self.groups.len() - 1);
            }
        }
        let after: Avail = inside.clone();
        let saved_dirty = self.dirty;
        match &mut s.kind {
            StmtKind::While { cond, body, .. } => {
                self.dirty = false;
                let mut a = inside.clone();
                // (evaluated again after every pass of the body: no group
                // starts here)
                self.expr(cond, true, &mut a);
                let _ = self.inner(body, &a);
            }
            StmtKind::Repeat { body, cond } => {
                // The condition sees the body's locals: it is part of the
                // body's block.
                let _ = self.repeat_body(body, cond, &inside);
            }
            StmtKind::NumericFor { body, .. } | StmtKind::GenericFor { body, .. } => {
                let _ = self.inner(body, &inside);
            }
            _ => {}
        }
        self.dirty = saved_dirty;
        *avail = after;
    }

    fn repeat_body(&mut self, b: &mut Block, cond: &mut Expr, avail: &Avail) -> Avail {
        // As `block`, with the condition after the last statement.
        let id = self.next_block;
        let mut a = avail.clone();
        let out = self.block(b, a.clone());
        a = out;
        let saved = self.at;
        self.at = (id, b.stmts.len());
        self.dirty = false;
        // (reads in the condition start no group: a local declared before
        // `until` would be inside the body's block, which `block` already
        // closed)
        self.cond_only(cond, &mut a);
        self.at = saved;
        a
    }

    fn cond_only(&mut self, e: &mut Expr, avail: &mut Avail) {
        self.dirty = true;
        self.expr(e, true, avail);
    }
}

/// The parts of an assignment target Lua evaluates before assigning: a
/// field's object, an index's object and key.
fn target_parts(t: &mut Expr) -> Vec<&mut Expr> {
    match &mut t.kind {
        ExprKind::Field { obj, .. } => vec![obj.as_mut()],
        ExprKind::Index { obj, key, .. } => vec![obj.as_mut(), key.as_mut()],
        _ => Vec::new(),
    }
}

/// What a loop may change, which chains it reads from its start (with the
/// first read's expression), and what it declares.
struct Summary {
    kills: Kills,
    anticipated: BTreeMap<Path, (Expr, ())>,
    declared: BTreeSet<String>,
}

/// The kills of an expression's evaluation.
fn expr_kills(e: &mut Expr, g: &Globals, k: &mut Kills) {
    each_expr(e, false, &mut |e| match &e.kind {
        ExprKind::Call { callee, .. } if !pure_call(callee, g) => k.all = true,
        ExprKind::Method { .. } => k.all = true,
        ExprKind::Field { name, .. } if !name.facts().has(Facts::PLAIN) => k.all = true,
        ExprKind::Index { facts, .. } if !facts.has(Facts::PLAIN) => k.all = true,
        ExprKind::Binary { facts, .. } | ExprKind::Unary { facts, .. }
            if !facts.has(Facts::NO_META) =>
        {
            k.all = true
        }
        _ => {}
    });
}

fn block_kills(
    b: &mut Block,
    g: &Globals,
    numbers: &BTreeSet<String>,
    k: &mut Kills,
    declared: &mut BTreeSet<String>,
) {
    for s in &mut b.stmts {
        match &s.kind {
            StmtKind::Local { names, .. } => {
                declared.extend(names.iter().map(|n| n.name.to_string()))
            }
            StmtKind::LocalFunction { name, .. } => {
                declared.insert(name.name.to_string());
            }
            StmtKind::NumericFor { var, .. } => {
                declared.insert(var.name.to_string());
            }
            StmtKind::GenericFor { vars, .. } => {
                declared.extend(vars.iter().map(|n| n.name.to_string()))
            }
            _ => {}
        }
        if let StmtKind::GenericFor { exprs, .. } = &s.kind
            && !iterates_plainly(exprs, g)
        {
            k.all = true;
        }
        let w = Pass::writes(s, numbers);
        k.all |= w.all;
        k.fields.extend(w.fields);
        k.locals.extend(w.locals);
        for e in own_exprs(s) {
            expr_kills(e, g, k);
        }
        for inner in inner_blocks(s) {
            block_kills(inner, g, numbers, k, declared);
        }
    }
}

/// A generic `for` whose iterator runs no code: `pairs(t)`, `ipairs(t)`,
/// `next, t`.
fn iterates_plainly(exprs: &[Expr], g: &Globals) -> bool {
    match exprs {
        [call] => match &call.kind {
            ExprKind::Call { callee, .. } => matches!(&callee.kind,
                ExprKind::Name(f) if matches!(f.name.as_str(), "pairs" | "ipairs") && g.stable(&f.name)),
            _ => false,
        },
        [f, ..] => matches!(&f.kind, ExprKind::Name(n) if n.name == "next" && g.stable("next")),
        [] => false,
    }
}

/// The unconditional chain reads of an expression, until code would run.
fn first_reads(e: &mut Expr, out: &mut Vec<(Path, Expr)>, ran: &mut bool, g: &Globals) {
    if *ran {
        return;
    }
    if let Some(p) = path_of(e) {
        for len in 2..=p.len() {
            let mut x = prefix_mut(e, p.len(), len).clone();
            x.sep = Pos::NONE;
            out.push((p[..len].to_vec(), x));
        }
        return;
    }
    match &mut e.kind {
        ExprKind::Function(_) => {}
        ExprKind::Binary {
            op: BinOp::And | BinOp::Or,
            lhs,
            ..
        } => {
            first_reads(lhs, out, ran, g);
            // (the right operand may not run)
            *ran = true;
        }
        _ => {
            let runs = match &e.kind {
                ExprKind::Call { callee, .. } => !pure_call(callee, g),
                ExprKind::Method { .. } => true,
                ExprKind::Field { name, .. } => !name.facts().has(Facts::PLAIN),
                ExprKind::Index { facts, .. } => !facts.has(Facts::PLAIN),
                ExprKind::Binary { facts, .. } | ExprKind::Unary { facts, .. } => {
                    !facts.has(Facts::NO_META)
                }
                _ => false,
            };
            for p in parts(e) {
                first_reads(p, out, ran, g);
            }
            if runs {
                *ran = true;
            }
        }
    }
}

fn summarize(s: &mut Stmt, g: &Globals, numbers: &BTreeSet<String>) -> Summary {
    let mut kills = Kills::default();
    let mut declared = BTreeSet::new();
    let mut anticipated: BTreeMap<Path, (Expr, ())> = BTreeMap::new();
    match &mut s.kind {
        StmtKind::NumericFor { var, body, .. } => {
            declared.insert(var.name.to_string());
            block_kills(body, g, numbers, &mut kills, &mut declared);
        }
        StmtKind::GenericFor {
            vars, exprs, body, ..
        } => {
            declared.extend(vars.iter().map(|v| v.name.to_string()));
            if !iterates_plainly(exprs, g) {
                kills.all = true;
            }
            block_kills(body, g, numbers, &mut kills, &mut declared);
        }
        StmtKind::While { cond, body, .. } => {
            expr_kills(cond, g, &mut kills);
            block_kills(body, g, numbers, &mut kills, &mut declared);
        }
        StmtKind::Repeat { body, cond } => {
            block_kills(body, g, numbers, &mut kills, &mut declared);
            expr_kills(cond, g, &mut kills);
        }
        _ => {}
    }
    if kills.all {
        return Summary {
            kills,
            anticipated,
            declared,
        };
    }
    // The reads at the loop's start: its top-level statements, until one
    // that is not a plain `local`, assignment or call (an `if` or a loop
    // may narrow what the checker proved).
    let mut cond_names: BTreeSet<String> = BTreeSet::new();
    if let StmtKind::While { cond, .. } = &mut s.kind {
        each_name_in_expr(cond, &mut |id| {
            cond_names.insert(id.name.to_string());
        });
    }
    let body = match &mut s.kind {
        StmtKind::NumericFor { body, .. }
        | StmtKind::GenericFor { body, .. }
        | StmtKind::While { body, .. }
        | StmtKind::Repeat { body, .. } => body,
        _ => unreachable!("a loop"),
    };
    // Names a guard (`if v == nil then break end`) tested: the checker
    // may have narrowed them after it.
    let mut guarded: BTreeSet<String> = BTreeSet::new();
    for st in &mut body.stmts {
        if let Some(names) = guard_names(st) {
            guarded.extend(names);
            continue;
        }
        let simple = matches!(
            st.kind,
            StmtKind::Local { .. } | StmtKind::Assign { .. } | StmtKind::Call(_)
        );
        if !simple {
            break;
        }
        let mut ran = false;
        let mut reads = Vec::new();
        if let StmtKind::Assign { targets, values } = &mut st.kind {
            for t in targets.iter_mut() {
                for p in target_parts(t) {
                    first_reads(p, &mut reads, &mut ran, g);
                }
            }
            for v in values.iter_mut() {
                first_reads(v, &mut reads, &mut ran, g);
            }
        } else {
            for e in own_exprs(st) {
                first_reads(e, &mut reads, &mut ran, g);
            }
        }
        for (p, x) in reads {
            if !cond_names.contains(&p[0]) && !guarded.contains(&p[0]) {
                anticipated.entry(p).or_insert((x, ()));
            }
        }
        if ran {
            break;
        }
    }
    Summary {
        kills,
        anticipated,
        declared,
    }
}

/// An `if` without `else` whose body only leaves the loop or the function
/// (`if v == nil then break end`), whose conditions run no code: the names
/// they mention.
fn guard_names(s: &mut Stmt) -> Option<BTreeSet<String>> {
    let StmtKind::If {
        clauses,
        else_block: None,
    } = &mut s.kind
    else {
        return None;
    };
    let mut names = BTreeSet::new();
    for c in clauses {
        let leaves = matches!(
            c.body.stmts.as_slice(),
            [Stmt {
                kind: StmtKind::Break,
                ..
            }] | [Stmt {
                kind: StmtKind::Return(_),
                ..
            }]
        );
        if !leaves {
            return None;
        }
        if let [
            Stmt {
                kind: StmtKind::Return(values),
                ..
            },
        ] = c.body.stmts.as_mut_slice()
        {
            for v in values {
                if !super::loops::pure(v) {
                    return None;
                }
            }
        }
        let mut runs = false;
        each_expr(&mut c.cond, false, &mut |e| {
            runs |= match &e.kind {
                ExprKind::Call { .. } | ExprKind::Method { .. } => true,
                ExprKind::Field { name, .. } => !name.facts().has(Facts::PLAIN),
                ExprKind::Index { facts, .. } => !facts.has(Facts::PLAIN),
                ExprKind::Binary { facts, .. } | ExprKind::Unary { facts, .. } => {
                    !facts.has(Facts::NO_META)
                }
                _ => false,
            };
        });
        if runs {
            return None;
        }
        each_name_in_expr(&mut c.cond, &mut |id| {
            names.insert(id.name.to_string());
        });
    }
    Some(names)
}

/// Chooses, for every chain read, the longest prefix whose local is worth
/// it: read at least twice, or once in the loop it was hoisted out of.
fn choose(groups: &[Group], events: &mut [Event], room: usize) -> BTreeSet<usize> {
    let mut banned: BTreeSet<usize> = BTreeSet::new();
    for _ in 0..8 {
        let mut uses: BTreeMap<usize, u32> = BTreeMap::new();
        for ev in events.iter_mut() {
            ev.choice = None;
            for (i, g) in ev.groups.iter().enumerate().rev() {
                let Some(g) = *g else { continue };
                if banned.contains(&g) {
                    continue;
                }
                let worth = if groups[g].hoisted {
                    groups[g].reads >= 1
                } else {
                    groups[g].reads >= 2
                };
                if worth {
                    ev.choice = Some((i + 2, g));
                    *uses.entry(g).or_default() += 1;
                    break;
                }
            }
        }
        let mut changed = false;
        for (&g, &n) in &uses {
            let need = if groups[g].hoisted { 1 } else { 2 };
            if n < need {
                banned.insert(g);
                changed = true;
            }
        }
        // Within the function's room for locals: the most used first.
        if !changed && uses.len() > room {
            let mut by_use: Vec<(u32, usize)> = uses.iter().map(|(&g, &n)| (n, g)).collect();
            by_use.sort();
            for (_, g) in by_use.iter().take(uses.len() - room) {
                banned.insert(*g);
            }
            changed = true;
        }
        if !changed {
            return uses.keys().copied().collect();
        }
    }
    for ev in events.iter_mut() {
        ev.choice = None;
    }
    BTreeSet::new()
}

/// `cache-fields`.
pub(crate) fn cache_fields(chunk: &mut Chunk, g: &Globals, u: &mut Unique) {
    let mut main = Func {
        pos: Pos::NONE,
        params: Vec::new(),
        vararg: true,
        body: std::mem::take(&mut chunk.block),
    };
    let mut numbers = BTreeSet::new();
    each_block(&mut main.body, &mut |b| {
        for s in &b.stmts {
            if let StmtKind::NumericFor { var, .. } = &s.kind {
                numbers.insert(var.name.to_string());
            }
        }
    });
    functions(&mut main, false, &mut |f, method| {
        // Twice: the locals of the first round may share a prefix
        // (`p.pos.x`, `p.pos.y`).
        for _ in 0..2 {
            if !cache_in(f, method, g, &numbers, u) {
                break;
            }
        }
    });
    chunk.block = main.body;
}

/// One round in one function; whether it cached anything.
fn cache_in(
    f: &mut Func,
    method: bool,
    g: &Globals,
    numbers: &BTreeSet<String>,
    u: &mut Unique,
) -> bool {
    {
        let room = LOCALS_BUDGET.saturating_sub(max_locals(f, method));
        if room == 0 {
            return false;
        }
        let mut pass = Pass {
            numbers: numbers.clone(),
            g,
            apply: false,
            groups: Vec::new(),
            events: Vec::new(),
            next_event: 0,
            next_block: 0,
            at: (0, 0),
            dirty: false,
            inserts: HashMap::new(),
        };
        pass.block(&mut f.body, Avail::new());
        let chosen = choose(&pass.groups, &mut pass.events, room);
        if chosen.is_empty() {
            return false;
        }
        for &gi in &chosen {
            let group = &mut pass.groups[gi];
            let last = match &group.expr.kind {
                ExprKind::Field { name, .. } => name.name.to_string(),
                _ => "f".to_string(),
            };
            group.local = Some(u.fresh_ident(&format!("__wl_{last}"), Pos::NONE));
            pass.inserts
                .entry(group.anchor.0)
                .or_default()
                .push((group.anchor.1, gi));
        }
        pass.apply = true;
        pass.next_event = 0;
        pass.next_block = 0;
        pass.block(&mut f.body, Avail::new());
        true
    }
}
