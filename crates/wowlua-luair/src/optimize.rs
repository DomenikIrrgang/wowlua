//! Profile-controlled IR optimizations (spec optimization §15.2):
//! `cache-globals` and the local half of `dead-code`, within the Lua-limit
//! budgets of §15.2.2.

use crate::ir::*;
use crate::minify::{LOCALS_BUDGET, UPVALUES_BUDGET};
use crate::scope::{Ref, Resolution, Use, Visitor, ident_count, resolve, walk};
use std::collections::{BTreeMap, BTreeSet, HashMap};

mod constants;
mod fields;
mod globals;
mod loops;
mod names;
mod upvalues;
mod walk;
use self::walk as optimize_walk;

/// The optimizations that shape code for Lua 5.1's compiler (spec
/// optimization §15.2.3): each where its preconditions hold, those that
/// need types where the checker proved them ([`Facts`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shaping {
    pub constants: bool,
    pub cache_fields: bool,
    pub cache_upvalues: bool,
    pub numeric_loops: bool,
    pub table_insert: bool,
    pub hoist_closures: bool,
}

impl Shaping {
    pub fn any(&self) -> bool {
        *self != Shaping::default()
    }

    /// Those that add locals (left out, with `inline`, `cache-globals` and
    /// `dedupe-strings`, when a file would exceed a Lua 5.1 limit; spec
    /// optimization §15.2.2).
    pub fn without_new_locals(self) -> Shaping {
        Shaping {
            cache_fields: false,
            cache_upvalues: false,
            numeric_loops: false,
            hoist_closures: false,
            ..self
        }
    }
}

/// Runs the shaping optimizations of `opts` on a chunk. `preserve`: the
/// output keeps the line of every instruction (`line-mode` `preserve`), so
/// closures stay where they are.
pub fn shape(chunk: &mut Chunk, opts: &Shaping, preserve: bool) {
    if !opts.any() {
        return;
    }
    let mut unique = names::uniquify(chunk);
    let g = globals::Globals::of(chunk);
    if opts.constants {
        constants::propagate(chunk);
    }
    if opts.numeric_loops {
        loops::numeric_loops(chunk, &g, &mut unique);
    }
    if opts.table_insert {
        loops::table_insert(chunk, &g);
    }
    if opts.cache_fields {
        fields::cache_fields(chunk, &g, &mut unique);
    }
    if opts.hoist_closures && !preserve {
        loops::hoist_closures(chunk, &g, &mut unique);
    }
    if opts.cache_upvalues {
        upvalues::cache_upvalues(chunk, &mut unique);
    }
    unique.restore(chunk);
    crate::hygiene::apply(chunk);
}

/// The positions of the names that are not variables: fields, methods and
/// table keys.
fn clear_names(e: &mut Expr) {
    match &mut e.kind {
        ExprKind::Field { name, .. } => name.pos = Pos::NONE,
        ExprKind::Method { name, .. } => name.pos = Pos::NONE,
        ExprKind::Table { fields, .. } => {
            for f in fields {
                match f {
                    Field::Named { name, .. } => name.pos = Pos::NONE,
                    Field::Keyed { open, .. } => *open = Pos::NONE,
                    Field::Positional(_) => {}
                }
            }
        }
        _ => {}
    }
}

/// Where an inlined call statement goes, until the inliner's walk is done:
/// an empty block, told apart by its end.
fn inlined_placeholder(index: usize) -> Stmt {
    Stmt {
        pos: Pos::NONE,
        kind: StmtKind::Do(Block {
            stmts: Vec::new(),
            end: Pos::new(u32::MAX, index as u32),
        }),
    }
}

/// Puts the inlined call statements in their placeholders.
fn fill_inlined(b: &mut Block, stmts: &mut [Option<Stmt>]) {
    optimize_walk::each_block(b, &mut |b| {
        for s in &mut b.stmts {
            if let StmtKind::Do(inner) = &s.kind
                && inner.stmts.is_empty()
                && inner.end.line == u32::MAX
                && let Some(new) = stmts.get_mut(inner.end.col as usize).and_then(Option::take)
            {
                *s = new;
            }
        }
    });
}

/// A block without positions (inlined statements, generated helpers).
pub fn clear_block_positions(b: &mut Block) {
    struct Clear;
    impl Visitor for Clear {
        fn stmt(&mut self, s: &mut Stmt) {
            s.pos = Pos::NONE;
            match &mut s.kind {
                StmtKind::While { do_pos, .. }
                | StmtKind::NumericFor { do_pos, .. }
                | StmtKind::GenericFor { do_pos, .. } => *do_pos = Pos::NONE,
                StmtKind::If { clauses, .. } => {
                    for c in clauses {
                        c.then_pos = Pos::NONE;
                    }
                }
                StmtKind::Function { path, method, func } => {
                    for p in path {
                        p.pos = Pos::NONE;
                    }
                    if let Some(m) = method {
                        m.pos = Pos::NONE;
                    }
                    func.pos = Pos::NONE;
                }
                StmtKind::LocalFunction { func, .. } => func.pos = Pos::NONE,
                _ => {}
            }
            for inner in optimize_walk::inner_blocks(s) {
                inner.end = Pos::NONE;
            }
            if let Some(f) = optimize_walk::declared_function(s) {
                f.body.end = Pos::NONE;
            }
        }
        fn expr(&mut self, e: &mut Expr) {
            e.pos = Pos::NONE;
            e.sep = Pos::NONE;
            clear_names(e);
            match &mut e.kind {
                ExprKind::Binary { op_pos, .. } => *op_pos = Pos::NONE,
                ExprKind::Paren { close, .. } | ExprKind::Table { close, .. } => *close = Pos::NONE,
                ExprKind::Call {
                    args: Args::Parens { close, .. },
                    ..
                }
                | ExprKind::Method {
                    args: Args::Parens { close, .. },
                    ..
                } => *close = Pos::NONE,
                ExprKind::Function(f) => {
                    f.pos = Pos::NONE;
                    f.body.end = Pos::NONE;
                }
                _ => {}
            }
        }
        fn ident(&mut self, id: &mut Ident, _use: Use) {
            id.pos = Pos::NONE;
        }
    }
    b.end = Pos::NONE;
    let mut c = Chunk {
        block: std::mem::take(b),
        comments: Vec::new(),
    };
    walk(&mut c, &mut Clear);
    *b = c.block;
}

/// Lua 5.1's base functions, WoW's Lua aliases and the standard library
/// tables: the names `cache-globals: "safe"` may hoist (decision T162).
/// `print`, the environment functions and every other WoW API stay global,
/// as addons hook them.
const STABLE_GLOBALS: &[&str] = &[
    // Lua 5.1 base library.
    "assert",
    "collectgarbage",
    "error",
    "getmetatable",
    "ipairs",
    "next",
    "pairs",
    "pcall",
    "rawequal",
    "rawget",
    "rawset",
    "select",
    "setmetatable",
    "tonumber",
    "tostring",
    "type",
    "unpack",
    "xpcall",
    // Library tables.
    "bit",
    "coroutine",
    "math",
    "string",
    "table",
    // WoW's Lua aliases.
    "abs",
    "acos",
    "asin",
    "atan",
    "atan2",
    "ceil",
    "cos",
    "deg",
    "exp",
    "floor",
    "frexp",
    "ldexp",
    "log",
    "log10",
    "max",
    "min",
    "mod",
    "rad",
    "random",
    "sin",
    "sqrt",
    "tan",
    "format",
    "gmatch",
    "gsub",
    "strbyte",
    "strchar",
    "strcmputf8i",
    "strconcat",
    "strfind",
    "strjoin",
    "strlen",
    "strlenutf8",
    "strlower",
    "strmatch",
    "strrep",
    "strrev",
    "strsplit",
    "strsub",
    "strtrim",
    "strupper",
    "foreach",
    "foreachi",
    "getn",
    "sort",
    "tinsert",
    "tremove",
    "wipe",
];

/// Functions of the standard library tables `cache-globals` may hoist.
const STABLE_FIELDS: &[(&str, &[&str])] = &[
    (
        "string",
        &[
            "byte", "char", "find", "format", "gmatch", "gsub", "len", "lower", "match", "rep",
            "reverse", "sub", "upper", "split", "join", "trim",
        ],
    ),
    (
        "math",
        &[
            "abs", "acos", "asin", "atan", "atan2", "ceil", "cos", "cosh", "deg", "exp", "floor",
            "fmod", "frexp", "ldexp", "log", "log10", "max", "min", "modf", "pow", "rad", "random",
            "sin", "sinh", "sqrt", "tan", "tanh",
        ],
    ),
    (
        "table",
        &[
            "concat", "insert", "remove", "sort", "wipe", "maxn", "getn", "foreach", "foreachi",
        ],
    ),
    (
        "bit",
        &[
            "band", "bor", "bxor", "bnot", "lshift", "rshift", "arshift", "mod",
        ],
    ),
];

fn stable_global(name: &str) -> bool {
    STABLE_GLOBALS.contains(&name)
        || (name.len() > 2
            && name.starts_with("C_")
            && name[2..].starts_with(|c: char| c.is_ascii_alphabetic()))
}

fn stable_field(lib: &str, name: &str) -> bool {
    STABLE_FIELDS
        .iter()
        .any(|(l, fields)| *l == lib && fields.contains(&name))
}

/// What a hoisted local stands for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Hoisted {
    Global(String),
    Field(String, String),
}

/// Finds global reads, global writes and field writes, with the function
/// each read is in (numbered like [`resolve`]).
#[derive(Default)]
struct Census<'a> {
    res: Option<&'a Resolution>,
    next: usize,
    functions: Vec<usize>,
    count: usize,
    reads: BTreeMap<Hoisted, (usize, BTreeSet<usize>)>,
    written: BTreeSet<Hoisted>,
    /// `lib.field` reads seen through the field's `expr` callback, keyed by
    /// the occurrence of `lib`.
    pending_field: HashMap<usize, String>,
    /// Fields written with `lib.field = …`.
    target_field: HashMap<usize, String>,
}

impl Census<'_> {
    fn global(&self, occurrence: usize) -> bool {
        matches!(
            self.res.and_then(|r| r.occurrences.get(occurrence)),
            Some(Ref::Global)
        )
    }
}

impl Visitor for Census<'_> {
    fn enter_function(&mut self) {
        self.functions.push(self.count);
        self.count += 1;
    }
    fn leave_function(&mut self) {
        self.functions.pop();
    }
    fn stmt(&mut self, s: &mut Stmt) {
        if let StmtKind::Assign { targets, .. } = &mut s.kind {
            // Occurrences in the targets come first; count up to each one.
            let mut next = self.next;
            for t in targets.iter_mut() {
                if let ExprKind::Field { obj, name } = &t.kind
                    && matches!(obj.kind, ExprKind::Name(_))
                {
                    self.target_field.insert(next, name.name.to_string());
                }
                next += match &mut t.kind {
                    ExprKind::Name(_) => 1,
                    _ => ident_count(t),
                };
            }
        }
        if let StmtKind::Function { path, .. } = &s.kind
            && path.len() == 2
        {
            self.target_field
                .insert(self.next, path[1].name.to_string());
        }
    }
    fn expr(&mut self, e: &mut Expr) {
        if let ExprKind::Field { obj, name } = &e.kind
            && matches!(obj.kind, ExprKind::Name(_))
        {
            self.pending_field.insert(self.next, name.name.to_string());
        }
    }
    fn ident(&mut self, id: &mut Ident, use_: Use) {
        let index = self.next;
        self.next += 1;
        if !self.global(index) {
            return;
        }
        let function = *self.functions.last().expect("in a function");
        if let Some(field) = self.target_field.remove(&index) {
            self.written
                .insert(Hoisted::Field(id.name.to_string(), field));
        }
        match use_ {
            Use::Write => {
                self.written.insert(Hoisted::Global(id.name.to_string()));
            }
            _ => {
                let key = match self.pending_field.remove(&index) {
                    Some(field) if stable_field(&id.name, &field) => {
                        Hoisted::Field(id.name.to_string(), field)
                    }
                    _ if stable_global(&id.name) => Hoisted::Global(id.name.to_string()),
                    _ => return,
                };
                let entry = self.reads.entry(key).or_default();
                entry.0 += 1;
                entry.1.insert(function);
            }
        }
    }
}

/// The functions that capture a local of the main chunk used in
/// `functions`: those and every function around them.
fn capturing(res: &Resolution, functions: &BTreeSet<usize>) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for &f in functions {
        let mut cur = Some(f);
        while let Some(g) = cur
            && g != 0
        {
            out.insert(g);
            cur = res.functions[g].parent;
        }
    }
    out
}

/// `cache-globals: "safe"` (spec §15.2): stable globals read at least twice
/// and never assigned become locals at the top of the chunk, the most used
/// first, within the budgets. A chunk that uses `setfenv`, `getfenv` or
/// `loadstring` (other environments) is left alone.
pub fn cache_globals(chunk: &mut Chunk) {
    let res = resolve(chunk);
    if ["setfenv", "getfenv", "loadstring", "_G"]
        .iter()
        .any(|n| res.globals.contains(*n))
    {
        return;
    }
    let mut census = Census {
        res: Some(&res),
        ..Census::default()
    };
    walk(chunk, &mut census);
    let mut ranked: Vec<(Hoisted, usize, BTreeSet<usize>)> = census
        .reads
        .into_iter()
        .filter(|(k, (n, _))| {
            *n >= 2
                && !census.written.contains(k)
                && match k {
                    Hoisted::Field(lib, _) => {
                        !census.written.contains(&Hoisted::Global(lib.clone()))
                    }
                    Hoisted::Global(_) => true,
                }
        })
        .map(|(k, (n, f))| (k, n, f))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut locals = res.functions.first().map_or(0, |f| f.max_locals);
    let mut added: HashMap<usize, usize> = HashMap::new();
    let mut chosen: Vec<Hoisted> = Vec::new();
    for (key, _, functions) in ranked {
        if locals + 1 > LOCALS_BUDGET {
            break;
        }
        let capture = capturing(&res, &functions);
        let fits = capture.iter().all(|f| {
            res.functions[*f].upvalues.len() + added.get(f).copied().unwrap_or(0) < UPVALUES_BUDGET
        });
        if !fits {
            continue;
        }
        for f in capture {
            *added.entry(f).or_default() += 1;
        }
        locals += 1;
        chosen.push(key);
    }
    if chosen.is_empty() {
        return;
    }
    let name_of = |h: &Hoisted| match h {
        Hoisted::Global(n) => n.clone(),
        Hoisted::Field(lib, f) => format!("__wl_{lib}_{f}"),
    };
    // `lib.field` reads become the generated local; globals keep their name,
    // as the local shadows them.
    let fields: HashMap<(String, String), String> = chosen
        .iter()
        .filter_map(|h| match h {
            Hoisted::Field(l, f) => Some(((l.clone(), f.clone()), name_of(h))),
            _ => None,
        })
        .collect();
    if !fields.is_empty() {
        struct Replace<'a> {
            res: &'a Resolution,
            next: usize,
            fields: &'a HashMap<(String, String), String>,
        }
        impl Visitor for Replace<'_> {
            fn expr(&mut self, e: &mut Expr) {
                if let ExprKind::Field { obj, name } = &e.kind
                    && let ExprKind::Name(lib) = &obj.kind
                    && matches!(self.res.occurrences.get(self.next), Some(Ref::Global))
                    && let Some(local) = self
                        .fields
                        .get(&(lib.name.to_string(), name.name.to_string()))
                {
                    let local = local.clone();
                    let pos = e.pos;
                    // The new name's occurrence stands in for `lib`'s.
                    e.kind = ExprKind::Name(Ident::new_generated(local, pos));
                }
            }
            fn ident(&mut self, _id: &mut Ident, _use: Use) {
                self.next += 1;
            }
        }
        walk(
            chunk,
            &mut Replace {
                res: &res,
                next: 0,
                fields: &fields,
            },
        );
    }
    let pos = chunk.block.stmts.first().map_or(Pos::NONE, |s| s.pos);
    let decl = Stmt {
        pos,
        kind: StmtKind::Local {
            names: chosen
                .iter()
                .map(|h| {
                    let mut id = Ident::new(name_of(h), Pos::NONE);
                    id.set_generated(matches!(h, Hoisted::Field(..)));
                    id
                })
                .collect(),
            values: chosen
                .iter()
                .map(|h| {
                    let global =
                        |n: &str| Expr::new(ExprKind::Name(Ident::new(n, Pos::NONE)), Pos::NONE);
                    match h {
                        Hoisted::Global(n) => global(n),
                        Hoisted::Field(l, f) => Expr::new(
                            ExprKind::Field {
                                obj: Box::new(global(l)),
                                name: Ident::new(f.clone(), Pos::NONE),
                            },
                            Pos::NONE,
                        ),
                    }
                })
                .collect(),
        },
    };
    chunk.block.stmts.insert(0, decl);
    crate::hygiene::apply(chunk);
}

/// Evaluating `e` has no side effects and cannot fail.
fn pure(e: &Expr, res: &Resolution, occurrence: &mut usize) -> bool {
    match &e.kind {
        ExprKind::Nil
        | ExprKind::True
        | ExprKind::False
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Vararg => true,
        ExprKind::Function(_) => {
            *occurrence += ident_count(&mut e.clone());
            true
        }
        // A local's value; a global read may run `__index` of the
        // environment.
        ExprKind::Name(_) => {
            let local = matches!(res.occurrences.get(*occurrence), Some(Ref::Local(_)));
            *occurrence += 1;
            local
        }
        ExprKind::Table { fields, .. } => fields.iter().all(|f| match f {
            Field::Named { value, .. } | Field::Positional(value) => pure(value, res, occurrence),
            Field::Keyed { key, value, .. } => {
                matches!(
                    key.kind,
                    ExprKind::String(_) | ExprKind::True | ExprKind::False
                ) && pure(value, res, occurrence)
            }
        }),
        ExprKind::Paren { inner, .. } => pure(inner, res, occurrence),
        _ => false,
    }
}

/// The local half of `dead-code` (spec §15.2): `local` declarations and
/// local functions nothing uses, whose values have no side effects, go,
/// until none is left.
pub fn remove_dead_locals(chunk: &mut Chunk) {
    for _ in 0..16 {
        let res = resolve(chunk);
        struct Mark<'a> {
            res: &'a Resolution,
            next: usize,
            removed: usize,
        }
        impl Mark<'_> {
            fn unused(&self, occurrence: usize) -> bool {
                match self.res.occurrences.get(occurrence) {
                    Some(Ref::Local(b)) => self.res.bindings[*b].uses == 0,
                    _ => false,
                }
            }
        }
        impl Visitor for Mark<'_> {
            fn stmt(&mut self, s: &mut Stmt) {
                let dead = match &mut s.kind {
                    StmtKind::LocalFunction { .. } => self.unused(self.next),
                    StmtKind::Local { names, values }
                        if !values.is_empty() || !names.is_empty() =>
                    {
                        let mut occurrence = self.next;
                        let values_pure = values.iter().all(|v| pure(v, self.res, &mut occurrence));
                        let decl = self.next + values.iter_mut().map(ident_count).sum::<usize>();
                        values_pure
                            && occurrence == decl
                            && (0..names.len()).all(|i| self.unused(decl + i))
                    }
                    _ => false,
                };
                if dead {
                    // Skip its occurrences, as the walk will not see them.
                    let mut inner = std::mem::replace(
                        s,
                        Stmt {
                            pos: Pos::NONE,
                            kind: StmtKind::Do(Block {
                                stmts: Vec::new(),
                                end: Pos::NONE,
                            }),
                        },
                    );
                    self.next += crate::scope::stmt_ident_count(&mut inner);
                    self.removed += 1;
                }
            }
            fn ident(&mut self, _id: &mut Ident, _use: Use) {
                self.next += 1;
            }
        }
        let mut mark = Mark {
            res: &res,
            next: 0,
            removed: 0,
        };
        walk(chunk, &mut mark);
        if mark.removed == 0 {
            return;
        }
        drop_placeholders(&mut chunk.block);
    }
}

/// Removes the empty, positionless `do end` statements [`remove_dead_locals`]
/// leaves.
fn drop_placeholders(b: &mut Block) {
    struct Drop;
    impl Visitor for Drop {
        fn stmt(&mut self, s: &mut Stmt) {
            for block in blocks_of(s) {
                block.stmts.retain(|s| !is_placeholder(s));
            }
        }
        fn expr(&mut self, e: &mut Expr) {
            if let ExprKind::Function(f) = &mut e.kind {
                f.body.stmts.retain(|s| !is_placeholder(s));
            }
        }
    }
    b.stmts.retain(|s| !is_placeholder(s));
    let mut chunk = Chunk {
        block: std::mem::take(b),
        comments: Vec::new(),
    };
    walk(&mut chunk, &mut Drop);
    *b = chunk.block;
}

fn is_placeholder(s: &Stmt) -> bool {
    matches!(&s.kind, StmtKind::Do(b) if b.stmts.is_empty() && !b.end.is_some()) && !s.pos.is_some()
}

fn blocks_of(s: &mut Stmt) -> Vec<&mut Block> {
    match &mut s.kind {
        StmtKind::Do(b)
        | StmtKind::While { body: b, .. }
        | StmtKind::Repeat { body: b, .. }
        | StmtKind::NumericFor { body: b, .. }
        | StmtKind::GenericFor { body: b, .. } => vec![b],
        StmtKind::If {
            clauses,
            else_block,
        } => clauses
            .iter_mut()
            .map(|c| &mut c.body)
            .chain(else_block.iter_mut())
            .collect(),
        StmtKind::Function { func, .. } | StmtKind::LocalFunction { func, .. } => {
            vec![&mut func.body]
        }
        _ => Vec::new(),
    }
}

/// Why a local function was not inlined (for W3031 on `@inline` ones).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotInlined {
    /// Its body is neither a single `return <expression>` without
    /// functions or `...` nor statements without `return` (but a last one
    /// without values) or `...`, or it takes `...`.
    NotAnExpression,
    Recursive,
    /// Assigned, or used other than by calling it.
    Escapes,
    /// Statements: longer than [`STATEMENT_LIMIT`], and called more than
    /// once.
    TooLarge,
    /// Statements, but every call of it uses its value (or the build keeps
    /// lines, `line-mode` `preserve`).
    ValueUsed,
    /// It reads or sets the call stack by level (`setfenv`, `getfenv`,
    /// `error(…, level)`, `debugstack`, `debuglocals`, `debug.*`), which
    /// inlining would shift.
    Stack,
}

/// `f` reads or sets the call stack by level ([`NotInlined::Stack`]).
fn uses_stack(f: &Func) -> bool {
    let mut found = false;
    let mut body = f.body.clone();
    optimize_walk::each_expr_in_block(&mut body, false, &mut |e| {
        let ExprKind::Call { callee, args } = &e.kind else {
            return;
        };
        let n_args = match args {
            Args::Parens { args, .. } => args.len(),
            _ => 1,
        };
        found |= match &callee.kind {
            ExprKind::Name(n) => match n.name.as_str() {
                "setfenv" | "getfenv" | "debugstack" | "debuglocals" => true,
                "error" => n_args >= 2,
                _ => false,
            },
            ExprKind::Field { obj, .. } => {
                matches!(&obj.kind, ExprKind::Name(d) if d.name == "debug")
            }
            _ => false,
        };
    });
    found
}

/// Statement bodies longer than this (counting nested statements) are
/// inlined only where they are called once.
pub const STATEMENT_LIMIT: usize = 12;

/// An inlinable local function.
struct Helper {
    params: Vec<usize>,
    /// The body: an expression (`return <expression>`), or statements.
    body: Expr,
    stmts: Option<Vec<Stmt>>,
    /// The parameters, as declared (statement bodies declare them).
    param_names: Vec<Ident>,
    /// The locals the body declares (statement bodies).
    declared: BTreeSet<usize>,
    /// The most locals active at once in the body.
    locals: usize,
    /// The references of the body's names, in walk order.
    refs: Vec<Ref>,
}

/// `e` without positions: emitted where the code around it is (inlined
/// code, generated helpers).
pub fn clear_positions(e: &mut Expr) {
    struct Clear;
    impl Visitor for Clear {
        fn expr(&mut self, e: &mut Expr) {
            e.pos = Pos::NONE;
            e.sep = Pos::NONE;
            clear_names(e);
            match &mut e.kind {
                ExprKind::Binary { op_pos, .. } => *op_pos = Pos::NONE,
                ExprKind::Paren { close, .. } | ExprKind::Table { close, .. } => *close = Pos::NONE,
                ExprKind::Call {
                    args: Args::Parens { close, .. },
                    ..
                }
                | ExprKind::Method {
                    args: Args::Parens { close, .. },
                    ..
                } => *close = Pos::NONE,
                _ => {}
            }
        }
        fn ident(&mut self, id: &mut Ident, _use: Use) {
            id.pos = Pos::NONE;
        }
    }
    crate::scope::walk_expr(e, &mut Clear);
}

/// The body of `local function f(…) return <expr> end`, if inlinable.
fn single_return(func: &Func) -> Option<&Expr> {
    if func.vararg || func.body.stmts.len() != 1 {
        return None;
    }
    let StmtKind::Return(values) = &func.body.stmts[0].kind else {
        return None;
    };
    let [e] = values.as_slice() else { return None };
    let mut clean = true;
    let mut probe = e.clone();
    struct Probe<'a>(&'a mut bool);
    impl Visitor for Probe<'_> {
        fn expr(&mut self, e: &mut Expr) {
            if matches!(e.kind, ExprKind::Function(_) | ExprKind::Vararg) {
                *self.0 = false;
            }
        }
    }
    crate::scope::walk_expr(&mut probe, &mut Probe(&mut clean));
    clean.then_some(e)
}

/// The statements of a local function's body, if they can be inlined at a
/// call statement: no `...`, no `return` but a last one without values.
fn statement_body(func: &Func) -> Option<Vec<Stmt>> {
    if func.vararg {
        return None;
    }
    let mut stmts = func.body.stmts.clone();
    if matches!(stmts.last(), Some(Stmt { kind: StmtKind::Return(v), .. }) if v.is_empty()) {
        stmts.pop();
    }
    fn clean(b: &[Stmt]) -> bool {
        b.iter().all(|s| {
            let mut s = s.clone();
            if matches!(s.kind, StmtKind::Return(_)) {
                return false;
            }
            let mut ok = true;
            for e in optimize_walk::own_exprs(&mut s) {
                optimize_walk::each_expr(e, false, &mut |e| {
                    if matches!(e.kind, ExprKind::Vararg) {
                        ok = false;
                    }
                });
            }
            ok && optimize_walk::inner_blocks(&mut s)
                .into_iter()
                .all(|b| clean(&b.stmts))
        })
    }
    clean(&stmts).then_some(stmts)
}

/// Statements in `b`, nested ones included.
fn statement_count(b: &[Stmt]) -> usize {
    b.iter()
        .map(|s| {
            let mut s = s.clone();
            1 + optimize_walk::inner_blocks(&mut s)
                .into_iter()
                .map(|b| statement_count(&b.stmts))
                .sum::<usize>()
        })
        .sum()
}

/// `optimize.inline` (spec §15.2): calls of local functions whose body is a
/// single `return <expression>` become that expression, when the function
/// is not recursive, never assigned and only called, the arguments have no
/// side effects (literals and locals) and the body's names mean the same at
/// the call; call statements of local functions whose body is statements
/// become a `do` block that declares the parameters with the arguments and
/// runs the body (unless `statements` is off: `line-mode` `preserve`). A
/// function whose calls were all inlined goes. Returns the position of
/// every local function not inlined, and why.
pub fn inline(chunk: &mut Chunk) -> Vec<(Pos, NotInlined)> {
    inline_with(chunk, true)
}

/// [`inline`], with or without statement bodies.
pub fn inline_with(chunk: &mut Chunk, statements: bool) -> Vec<(Pos, NotInlined)> {
    let mut skipped = Vec::new();
    for round in 0..4 {
        let (inlined, reasons) = inline_round(chunk, statements);
        if round == 0 {
            skipped = reasons;
        }
        if !inlined {
            break;
        }
    }
    skipped
}

fn inline_round(chunk: &mut Chunk, statements: bool) -> (bool, Vec<(Pos, NotInlined)>) {
    let res = resolve(chunk);
    // How often each local is called (as `f(…)`).
    struct Calls<'a> {
        res: &'a Resolution,
        next: usize,
        calls: HashMap<usize, u32>,
        stmt_calls: HashMap<usize, u32>,
        statement: bool,
    }
    impl Visitor for Calls<'_> {
        fn stmt(&mut self, s: &mut Stmt) {
            self.statement = matches!(s.kind, StmtKind::Call(_));
        }
        fn expr(&mut self, e: &mut Expr) {
            let statement = std::mem::take(&mut self.statement);
            if let ExprKind::Call { callee, .. } = &e.kind
                && matches!(callee.kind, ExprKind::Name(_))
                && let Some(Ref::Local(b)) = self.res.occurrences.get(self.next)
            {
                *self.calls.entry(*b).or_default() += 1;
                if statement {
                    *self.stmt_calls.entry(*b).or_default() += 1;
                }
            }
        }
        fn ident(&mut self, _id: &mut Ident, _use: Use) {
            self.next += 1;
        }
    }
    let mut calls = Calls {
        res: &res,
        next: 0,
        calls: HashMap::new(),
        stmt_calls: HashMap::new(),
        statement: false,
    };
    walk(chunk, &mut calls);
    let stmt_calls = calls.stmt_calls;
    let calls = calls.calls;

    struct Inliner<'a> {
        res: &'a Resolution,
        calls: &'a HashMap<usize, u32>,
        next: usize,
        /// Ident callbacks of inlined code to ignore.
        skip: usize,
        stack: Vec<usize>,
        blocks: Vec<usize>,
        functions: Vec<usize>,
        function_count: usize,
        helpers: HashMap<usize, Helper>,
        added: HashMap<usize, usize>,
        inlined: HashMap<usize, u32>,
        reasons: Vec<(Pos, NotInlined)>,
        /// The next expression is a call statement, which must stay a call.
        statement: bool,
        /// Statement bodies are inlined.
        statements: bool,
        /// Call statements of each local.
        stmt_calls: &'a HashMap<usize, u32>,
        /// Inlined call statements, by their placeholder's number.
        inlined_stmts: Vec<Option<Stmt>>,
    }
    impl Inliner<'_> {
        fn binding(&self, occurrence: usize) -> Option<usize> {
            match self.res.occurrences.get(occurrence) {
                Some(Ref::Local(b)) => Some(*b),
                _ => None,
            }
        }
        /// The binding `name` means here (`None`: global).
        fn visible(&self, name: &str) -> Option<usize> {
            self.stack
                .iter()
                .rev()
                .copied()
                .find(|&b| self.res.bindings[b].name == name)
        }
    }
    impl Inliner<'_> {
        /// The body's free names mean the same here, and the function
        /// the call is in has room for what it captures: the captured
        /// locals that are new to it.
        fn fits(
            &mut self,
            helper_refs: &[Ref],
            body_names: &[String],
            own: &dyn Fn(usize) -> bool,
        ) -> Option<usize> {
            let current = *self.functions.last().expect("in a function");
            let mut captures: BTreeSet<usize> = BTreeSet::new();
            for (r, name) in helper_refs.iter().zip(body_names) {
                match r {
                    Ref::Local(p) if own(*p) => {}
                    Ref::Local(x) => {
                        if self.visible(name) != Some(*x) {
                            return None;
                        }
                        if self.res.bindings[*x].function != current {
                            captures.insert(*x);
                        }
                    }
                    Ref::Global => {
                        if self.visible(name).is_some() {
                            return None;
                        }
                    }
                }
            }
            let new = captures
                .iter()
                .filter(|x| !self.res.functions[current].upvalues.contains(x))
                .count();
            if current != 0
                && self.res.functions[current].upvalues.len()
                    + self.added.get(&current).copied().unwrap_or(0)
                    + new
                    > UPVALUES_BUDGET
            {
                return None;
            }
            Some(new)
        }

        /// A call statement of a local function whose body is statements:
        /// `do local <params> = <args> <body> end`.
        fn inline_statement(&mut self, s: &mut Stmt) -> bool {
            let StmtKind::Call(call) = &s.kind else {
                return false;
            };
            let ExprKind::Call { callee, args } = &call.kind else {
                return false;
            };
            if !matches!(callee.kind, ExprKind::Name(_)) {
                return false;
            }
            let Some(b) = self.binding(self.next) else {
                return false;
            };
            let Some(helper) = self.helpers.get(&b) else {
                return false;
            };
            let Some(stmts) = helper.stmts.clone() else {
                return false;
            };
            let mut args: Vec<Expr> = match args {
                Args::Parens { args, .. } => args.clone(),
                Args::String(s) => vec![(**s).clone()],
                Args::Table(t) => vec![(**t).clone()],
            };
            // Arguments past the parameters are evaluated by the call:
            // dropped only when that does nothing.
            if args.len() > helper.params.len()
                && !args[helper.params.len()..].iter().all(|a| {
                    matches!(
                        a.kind,
                        ExprKind::Nil
                            | ExprKind::True
                            | ExprKind::False
                            | ExprKind::Number(_)
                            | ExprKind::String(_)
                            | ExprKind::Name(_)
                    )
                })
            {
                return false;
            }
            args.truncate(helper.params.len());
            let mut body_names: Vec<String> = Vec::new();
            {
                let mut probe = Block {
                    stmts: stmts.clone(),
                    end: Pos::NONE,
                };
                optimize_walk::each_name_in_block(&mut probe, &mut |id| {
                    body_names.push(id.name.to_string())
                });
            }
            let refs = helper.refs.clone();
            let params = helper.params.clone();
            let declared = helper.declared.clone();
            let own = |p: usize| params.contains(&p) || declared.contains(&p);
            let current = *self.functions.last().expect("in a function");
            let locals = helper.locals + helper.param_names.len();
            if self.res.functions[current].max_locals + locals > LOCALS_BUDGET {
                return false;
            }
            let Some(new) = self.fits(&refs, &body_names, &own) else {
                return false;
            };
            *self.added.entry(current).or_default() += new;
            let helper = self.helpers.get(&b).expect("found above");
            let pos = s.pos;
            let mut names = helper.param_names.clone();
            for n in &mut names {
                n.pos = Pos::NONE;
            }
            for a in &mut args {
                a.sep = Pos::NONE;
            }
            let mut body: Vec<Stmt> = Vec::with_capacity(stmts.len() + 1);
            if !names.is_empty() {
                body.push(Stmt {
                    pos,
                    kind: StmtKind::Local {
                        names,
                        values: args,
                    },
                });
            }
            // The body keeps its positions: no build that inlines statements
            // keeps lines, so they only map the code to its source (an
            // error in it points at the line in the function).
            body.extend(stmts);
            let new = Stmt {
                pos,
                kind: StmtKind::Do(Block {
                    stmts: body,
                    end: Pos::NONE,
                }),
            };
            // The walk goes on over an empty placeholder: the inlined code
            // takes its place once the walk is done, so its names and
            // functions are not counted against the resolution.
            let index = self.inlined_stmts.len();
            self.inlined_stmts.push(Some(new));
            let mut old = std::mem::replace(s, inlined_placeholder(index));
            self.next += crate::scope::stmt_ident_count(&mut old);
            *self.inlined.entry(b).or_default() += 1;
            true
        }
    }
    impl Visitor for Inliner<'_> {
        fn stmt(&mut self, s: &mut Stmt) {
            if self.skip > 0 {
                return;
            }
            if self.statements && self.inline_statement(s) {
                return;
            }
            if matches!(s.kind, StmtKind::Call(_)) {
                self.statement = true;
            }
            let StmtKind::LocalFunction { name, func } = &s.kind else {
                return;
            };
            let Some(b) = self.binding(self.next) else {
                return;
            };
            if uses_stack(func) {
                self.reasons.push((name.pos, NotInlined::Stack));
                return;
            }
            let expression = single_return(func);
            let statements_body = if expression.is_none() {
                statement_body(func)
            } else {
                None
            };
            if expression.is_none() && statements_body.is_none() {
                self.reasons.push((name.pos, NotInlined::NotAnExpression));
                return;
            }
            let binding = &self.res.bindings[b];
            let params_start = self.next + 1;
            let params: Vec<usize> = (0..func.params.len())
                .filter_map(|i| self.binding(params_start + i))
                .collect();
            let body_start = params_start + func.params.len();
            let (body, count) = match expression {
                Some(body) => {
                    let mut body = body.clone();
                    let count = ident_count(&mut body);
                    (body, count)
                }
                None => {
                    let mut b = Block {
                        stmts: statements_body.clone().expect("statements"),
                        end: Pos::NONE,
                    };
                    let mut count = 0;
                    optimize_walk::each_name_in_block(&mut b, &mut |_| count += 1);
                    (Expr::new(ExprKind::Nil, Pos::NONE), count)
                }
            };
            let refs: Vec<Ref> = self.res.occurrences[body_start..body_start + count].to_vec();
            if refs.contains(&Ref::Local(b)) {
                self.reasons.push((name.pos, NotInlined::Recursive));
                return;
            }
            let called = self.calls.get(&b).copied().unwrap_or(0);
            if binding.writes > 0 || binding.uses != called {
                self.reasons.push((name.pos, NotInlined::Escapes));
                return;
            }
            let mut declared = BTreeSet::new();
            let mut locals = 0;
            if let Some(stmts) = &statements_body {
                let stmt_calls = self.stmt_calls.get(&b).copied().unwrap_or(0);
                if !self.statements || stmt_calls == 0 {
                    self.reasons.push((name.pos, NotInlined::ValueUsed));
                    return;
                }
                if called > 1 && statement_count(stmts) > STATEMENT_LIMIT {
                    self.reasons.push((name.pos, NotInlined::TooLarge));
                    return;
                }
                // The locals the body declares (with its functions').
                let mut b = Block {
                    stmts: stmts.clone(),
                    end: Pos::NONE,
                };
                struct Decl<'r> {
                    refs: &'r [Ref],
                    next: usize,
                    out: &'r mut BTreeSet<usize>,
                }
                impl Visitor for Decl<'_> {
                    fn ident(&mut self, _id: &mut Ident, use_: Use) {
                        if use_ == Use::Declare
                            && let Some(Ref::Local(x)) = self.refs.get(self.next)
                        {
                            self.out.insert(*x);
                        }
                        self.next += 1;
                    }
                }
                let mut c = Chunk {
                    block: std::mem::take(&mut b),
                    comments: Vec::new(),
                };
                walk(
                    &mut c,
                    &mut Decl {
                        refs: &refs,
                        next: 0,
                        out: &mut declared,
                    },
                );
                let f = Func {
                    pos: Pos::NONE,
                    params: Vec::new(),
                    vararg: false,
                    body: c.block,
                };
                locals = optimize_walk::max_locals(&f, false);
            }
            self.helpers.insert(
                b,
                Helper {
                    params,
                    body,
                    stmts: statements_body,
                    param_names: func.params.clone(),
                    declared,
                    locals,
                    refs,
                },
            );
        }

        fn expr(&mut self, e: &mut Expr) {
            if std::mem::take(&mut self.statement) || self.skip > 0 {
                return;
            }
            let ExprKind::Call { callee, args } = &e.kind else {
                return;
            };
            if !matches!(callee.kind, ExprKind::Name(_)) {
                return;
            }
            let Some(b) = self.binding(self.next) else {
                return;
            };
            let Some(helper) = self.helpers.get(&b) else {
                return;
            };
            if helper.stmts.is_some() {
                return;
            }
            let args: Vec<Expr> = match args {
                Args::Parens { args, .. } => args.clone(),
                Args::String(s) => vec![(**s).clone()],
                Args::Table(_) => return,
            };
            // Arguments: literals and locals, evaluated once either way.
            let mut occurrence = self.next + 1;
            for a in &args {
                match &a.kind {
                    ExprKind::Nil
                    | ExprKind::True
                    | ExprKind::False
                    | ExprKind::Number(_)
                    | ExprKind::String(_) => {}
                    ExprKind::Name(_) if self.binding(occurrence).is_some() => occurrence += 1,
                    _ => return,
                }
            }
            // The body's other names must mean the same here.
            let current = *self.functions.last().expect("in a function");
            let mut captures: BTreeSet<usize> = BTreeSet::new();
            let mut body_names: Vec<String> = Vec::new();
            {
                let mut probe = helper.body.clone();
                struct Names<'a>(&'a mut Vec<String>);
                impl Visitor for Names<'_> {
                    fn ident(&mut self, id: &mut Ident, _use: Use) {
                        self.0.push(id.name.to_string());
                    }
                }
                crate::scope::walk_expr(&mut probe, &mut Names(&mut body_names));
            }
            for (r, name) in helper.refs.iter().zip(&body_names) {
                match r {
                    Ref::Local(p) if helper.params.contains(p) => {}
                    Ref::Local(x) => {
                        if self.visible(name) != Some(*x) {
                            return;
                        }
                        if self.res.bindings[*x].function != current {
                            captures.insert(*x);
                        }
                    }
                    Ref::Global => {
                        if self.visible(name).is_some() {
                            return;
                        }
                    }
                }
            }
            // The upvalue budget of the function the call is in.
            let new = captures
                .iter()
                .filter(|x| !self.res.functions[current].upvalues.contains(x))
                .count();
            if current != 0
                && self.res.functions[current].upvalues.len()
                    + self.added.get(&current).copied().unwrap_or(0)
                    + new
                    > UPVALUES_BUDGET
            {
                return;
            }
            *self.added.entry(current).or_default() += new;
            // Substitute the arguments for the parameters.
            let mut body = helper.body.clone();
            struct Substitute<'a> {
                refs: &'a [Ref],
                params: &'a [usize],
                args: &'a [Expr],
                next: usize,
            }
            impl Visitor for Substitute<'_> {
                fn expr(&mut self, e: &mut Expr) {
                    if let ExprKind::Name(_) = e.kind
                        && let Some(Ref::Local(p)) = self.refs.get(self.next)
                        && let Some(i) = self.params.iter().position(|q| q == p)
                    {
                        let mut arg = self
                            .args
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| Expr::new(ExprKind::Nil, Pos::NONE));
                        arg.sep = Pos::NONE;
                        if !matches!(arg.kind, ExprKind::Name(_)) {
                            // The walk still visits one name here.
                            self.next += 1;
                            *e = arg;
                            return;
                        }
                        *e = arg;
                    }
                }
                fn ident(&mut self, _id: &mut Ident, _use: Use) {
                    self.next += 1;
                }
            }
            clear_positions(&mut body);
            crate::scope::walk_expr(
                &mut body,
                &mut Substitute {
                    refs: &helper.refs,
                    params: &helper.params,
                    args: &args,
                    next: 0,
                },
            );
            body.pos = e.pos;
            body.sep = e.sep;
            // The call's occurrences are consumed now; the inlined code's
            // are not in the resolution.
            let original = ident_count(&mut e.clone());
            let mut replaced = body.clone();
            self.skip = ident_count(&mut replaced);
            self.next += original;
            *e = body;
            *self.inlined.entry(b).or_default() += 1;
        }

        fn ident(&mut self, _id: &mut Ident, use_: Use) {
            if self.skip > 0 {
                self.skip -= 1;
                return;
            }
            let index = self.next;
            self.next += 1;
            if use_ == Use::Declare
                && let Some(b) = self.binding(index)
            {
                self.stack.push(b);
            }
        }
        fn implicit_self(&mut self) {
            // `self` is the binding declared at this point.
            if let Some(b) = (0..self.res.bindings.len()).find(|&b| {
                self.res.bindings[b].implicit
                    && self.res.bindings[b].function
                        == *self.functions.last().expect("in a function")
            }) {
                self.stack.push(b);
            }
        }
        fn enter_block(&mut self) {
            self.blocks.push(self.stack.len());
        }
        fn leave_block(&mut self) {
            let len = self.blocks.pop().expect("balanced");
            self.stack.truncate(len);
        }
        fn enter_function(&mut self) {
            self.functions.push(self.function_count);
            self.function_count += 1;
        }
        fn leave_function(&mut self) {
            self.functions.pop();
        }
    }
    let mut inliner = Inliner {
        res: &res,
        calls: &calls,
        next: 0,
        skip: 0,
        stack: Vec::new(),
        blocks: Vec::new(),
        functions: Vec::new(),
        function_count: 0,
        helpers: HashMap::new(),
        added: HashMap::new(),
        inlined: HashMap::new(),
        reasons: Vec::new(),
        statement: false,
        statements,
        stmt_calls: &stmt_calls,
        inlined_stmts: Vec::new(),
    };
    walk(chunk, &mut inliner);
    if !inliner.inlined_stmts.is_empty() {
        let mut stmts = std::mem::take(&mut inliner.inlined_stmts);
        fill_inlined(&mut chunk.block, &mut stmts);
    }
    let any = !inliner.inlined.is_empty();
    // Helpers whose every call was inlined go.
    let gone: BTreeSet<usize> = inliner
        .inlined
        .iter()
        .filter(|(b, n)| res.bindings[**b].uses == **n)
        .map(|(b, _)| *b)
        .collect();
    let reasons = inliner.reasons;
    if !gone.is_empty() {
        let names: BTreeSet<String> = gone.iter().map(|&b| res.bindings[b].name.clone()).collect();
        remove_functions(chunk, &names);
    }
    (any, reasons)
}

/// Removes the definitions of the inlined local functions `names`: after
/// inlining, a fresh resolution finds them unused.
fn remove_functions(chunk: &mut Chunk, names: &BTreeSet<String>) {
    let fresh = resolve(chunk);
    struct Mark<'a> {
        res: &'a Resolution,
        names: &'a BTreeSet<String>,
        next: usize,
    }
    impl Visitor for Mark<'_> {
        fn stmt(&mut self, s: &mut Stmt) {
            if let StmtKind::LocalFunction { name, .. } = &s.kind
                && self.names.contains(name.name.as_str())
                && let Some(Ref::Local(b)) = self.res.occurrences.get(self.next)
                && self.res.bindings[*b].uses == 0
                && match &s.kind {
                    StmtKind::LocalFunction { func, .. } => {
                        single_return(func).is_some() || statement_body(func).is_some()
                    }
                    _ => false,
                }
            {
                let mut inner = std::mem::replace(
                    s,
                    Stmt {
                        pos: Pos::NONE,
                        kind: StmtKind::Do(Block {
                            stmts: Vec::new(),
                            end: Pos::NONE,
                        }),
                    },
                );
                self.next += crate::scope::stmt_ident_count(&mut inner);
            }
        }
        fn ident(&mut self, _id: &mut Ident, _use: Use) {
            self.next += 1;
        }
    }
    walk(
        chunk,
        &mut Mark {
            res: &fresh,
            names,
            next: 0,
        },
    );
    drop_placeholders(&mut chunk.block);
}
