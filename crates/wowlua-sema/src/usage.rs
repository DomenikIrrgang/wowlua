//! Usage analysis for the load order (spec §5.4.2–§5.4.3, compiler
//! §13.6.1): which globals and addon-table fields (`ns.F`) each file defines
//! and uses, and whether that happens while the file loads or later.
//!
//! A use is *load-time* when it runs while its file loads: top-level code
//! and every function called from it, followed through local functions,
//! package functions (by their global or `ns` path) and imported functions.
//! A function passed to a deferred API (`SetScript`, `C_Timer.After`, …)
//! runs later; one passed to a package function counts as called when that
//! function calls (or forwards) the parameter, and one passed to any other
//! callee counts as called.
//!
//! A name tested for truthiness (`if X`, `X and …`, `not X`, `X ~= nil`,
//! `type(X)`) and the uses guarded by that test (the `then` branch, the
//! right operand of `and`) are *guarded*: they do not order files (decision
//! T70).

use std::collections::{HashMap, HashSet};
use wowlua_span::{FileId, Span};
use wowlua_syntax::Parse;
use wowlua_syntax::ast::*;

/// A name whose definition orders files: a global or an addon-table field.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Global(String),
    Ns(String),
    /// An XML template (`virtual="true"`).
    Template(String),
}

impl Key {
    pub fn display(&self) -> String {
        match self {
            Key::Global(n) => n.clone(),
            Key::Ns(n) => format!("ns.{n}"),
            Key::Template(n) => format!("template {n}"),
        }
    }
}

/// A key and fields below it (`MyAddon.Util.round` = Global(MyAddon), [Util, round]).
pub type Path = (Key, Vec<String>);

#[derive(Clone, Debug)]
pub struct Access {
    pub key: Key,
    pub span: Span,
    /// The function the access is in (0 = the file's top level).
    pub function: usize,
    /// An existence test or a use guarded by one.
    pub guarded: bool,
}

#[derive(Clone, Debug)]
pub enum CallTarget {
    Local(usize),
    Path(Path),
    Import {
        binding: String,
        fields: Vec<String>,
    },
    /// A parameter of the calling function (forwarded or called).
    Param(usize),
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Call {
    pub target: CallTarget,
    pub span: Span,
    /// Function values (and parameters) passed as arguments, by argument
    /// position (a method call's object is position 0).
    pub args: Vec<(usize, CallTarget)>,
    /// The callee runs passed functions later.
    pub deferred: bool,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub calls: Vec<Call>,
    /// Parameters the function calls (positions; `self` is 0 in methods).
    pub called_params: HashSet<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct FileUsage {
    /// Index 0 is the file's top level.
    pub functions: Vec<Function>,
    pub defs: Vec<Access>,
    pub uses: Vec<Access>,
    /// Functions stored under a global or `ns` path, for call resolution.
    pub function_defs: Vec<(Path, usize)>,
    pub exported_functions: HashMap<String, usize>,
    /// Uses of imported bindings: (binding, function, span).
    pub import_uses: Vec<(String, usize, Span)>,
    /// A write through a dynamic name (`_G[name] = …`, `setfenv`).
    pub dynamic_write: Option<Span>,
}

/// APIs that run passed functions later (spec §5.4.3).
#[derive(Clone, Debug, Default)]
pub struct Markers {
    pub deferred_functions: HashSet<String>,
    pub deferred_methods: HashSet<String>,
}

impl Markers {
    pub fn from_table() -> Markers {
        let m = &wowlua_flavors::table().load_order_markers;
        Markers {
            deferred_functions: m.deferred_functions.iter().cloned().collect(),
            deferred_methods: m.deferred_methods.iter().cloned().collect(),
        }
    }
}

#[derive(Clone, Debug)]
enum Local {
    Plain,
    Function(usize),
    Param(usize),
    /// The addon table (`local _, ns = ...`).
    Ns,
    /// An alias of a global or `ns` path.
    Alias(Path),
    Import(String),
}

/// What an expression evaluates to, as far as the analysis cares.
#[derive(Clone, Debug)]
enum Info {
    Function(usize),
    Path(Path),
    Ns,
    GlobalTable,
    Import(String, Vec<String>),
    Param(usize),
    VarargNs,
    Other,
}

struct Walker<'a> {
    out: FileUsage,
    scopes: Vec<HashMap<String, Local>>,
    current: usize,
    markers: &'a Markers,
    /// Keys known to be defined here (tested by an enclosing guard).
    guards: Vec<Key>,
}

/// The usage summary of one file.
pub fn file_usage(parse: &Parse, markers: &Markers) -> FileUsage {
    let mut w = Walker::new(markers);
    w.block(&parse.chunk.block);
    w.out
}

/// The usage summary of a UI XML file (spec §5.4.2): named frames define
/// globals and templates while the file loads; creating an element reads
/// its mixins, handler functions and parent, runs its `<OnLoad>` code and
/// instantiates the templates it inherits. A template's needs are a
/// function that instantiating it calls.
pub fn xml_file_usage(text: &str, file: FileId, markers: &Markers) -> FileUsage {
    let usage = wowlua_xml::usage::usage(text);
    let mut w = Walker::new(markers);
    let span = |r: &std::ops::Range<usize>| Span::new(file, r.start..r.end);
    for name in &usage.globals {
        w.def_key(Key::Global(name.name.clone()), span(&name.span));
    }
    for t in &usage.templates {
        w.def_key(Key::Template(t.name.name.clone()), span(&t.name.span));
    }
    for t in &usage.templates {
        let id = w.out.functions.len();
        w.out.functions.push(Function {
            name: format!("template {}", t.name.name),
            calls: Vec::new(),
            called_params: HashSet::new(),
        });
        w.current = id;
        w.xml_uses(&t.uses, text, file);
        w.current = 0;
        w.out
            .function_defs
            .push(((Key::Template(t.name.name.clone()), Vec::new()), id));
    }
    w.xml_uses(&usage.load, text, file);
    w.out
}

fn grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

impl<'a> Walker<'a> {
    fn new(markers: &'a Markers) -> Self {
        Walker {
            out: FileUsage {
                functions: vec![Function {
                    name: "top-level code".into(),
                    calls: Vec::new(),
                    called_params: HashSet::new(),
                }],
                ..FileUsage::default()
            },
            scopes: vec![HashMap::new()],
            current: 0,
            markers,
            guards: Vec::new(),
        }
    }

    /// Records what creating XML elements needs, in the current function.
    fn xml_uses(&mut self, uses: &wowlua_xml::usage::Uses, text: &str, file: FileId) {
        let span = |r: &std::ops::Range<usize>| Span::new(file, r.start..r.end);
        for g in &uses.globals {
            let key = Key::Global(g.name.clone());
            self.use_key(key.clone(), span(&g.span));
            // A handler function runs later; a mixin is only read.
        }
        for t in &uses.inherits {
            let key = Key::Template(t.name.clone());
            self.use_key(key.clone(), span(&t.span));
            self.record_call(
                CallTarget::Path((key, Vec::new())),
                &[],
                0,
                span(&t.span),
                false,
            );
        }
        for range in &uses.scripts {
            let code = wowlua_xml::usage::script_code(text, range.clone());
            let parse = wowlua_syntax::parse(&code, file);
            if parse.has_errors() {
                continue;
            }
            // Spans of the snippet are relative to its start.
            let offset = range.start as u32;
            let (uses, defs, imports) = (
                self.out.uses.len(),
                self.out.defs.len(),
                self.out.import_uses.len(),
            );
            let functions = self.out.functions.len();
            let calls = self.out.functions[self.current].calls.len();
            self.scopes.push(HashMap::new());
            self.declare("self", Local::Plain);
            for s in &parse.chunk.block.stmts {
                self.stmt(s);
            }
            self.scopes.pop();
            let shift = |s: &mut Span| {
                s.start += offset;
                s.end += offset;
            };
            self.out.uses[uses..]
                .iter_mut()
                .for_each(|a| shift(&mut a.span));
            self.out.defs[defs..]
                .iter_mut()
                .for_each(|a| shift(&mut a.span));
            self.out.import_uses[imports..]
                .iter_mut()
                .for_each(|a| shift(&mut a.2));
            let current = self.current;
            self.out.functions[current].calls[calls..]
                .iter_mut()
                .for_each(|c| shift(&mut c.span));
            for f in &mut self.out.functions[functions..] {
                f.calls.iter_mut().for_each(|c| shift(&mut c.span));
            }
        }
    }

    fn lookup(&self, name: &str) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn declare(&mut self, name: &str, local: Local) {
        self.scopes
            .last_mut()
            .expect("scope")
            .insert(name.to_string(), local);
    }

    fn use_key(&mut self, key: Key, span: Span) {
        let guarded = self.guards.contains(&key);
        self.out.uses.push(Access {
            key,
            span,
            function: self.current,
            guarded,
        });
    }

    fn def_key(&mut self, key: Key, span: Span) {
        self.out.defs.push(Access {
            key,
            span,
            function: self.current,
            guarded: false,
        });
    }

    /// Walks an expression tested for truthiness; returns the keys that are
    /// defined when it is true.
    fn test(&mut self, e: &Expr) -> Vec<Key> {
        match &e.kind {
            ExprKind::Binary {
                op: BinOp::And,
                lhs,
                rhs,
            } => {
                let mut keys = self.test(lhs);
                let n = self.guards.len();
                self.guards.extend(keys.iter().cloned());
                keys.extend(self.test(rhs));
                self.guards.truncate(n);
                keys
            }
            ExprKind::Binary {
                op: BinOp::Or | BinOp::Coalesce,
                lhs,
                rhs,
            } => {
                self.test(lhs);
                self.test(rhs);
                Vec::new()
            }
            ExprKind::Unary {
                op: UnOp::Not,
                operand,
            } => {
                self.test(operand);
                Vec::new()
            }
            ExprKind::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                lhs,
                rhs,
            } if matches!(lhs.kind, ExprKind::Nil) || matches!(rhs.kind, ExprKind::Nil) => {
                let operand = if matches!(lhs.kind, ExprKind::Nil) {
                    rhs
                } else {
                    lhs
                };
                let keys = self.test_operand(operand);
                if *op == BinOp::Ne { keys } else { Vec::new() }
            }
            // `type(X) == "table"` (or `~= "nil"`) holds only when X exists.
            ExprKind::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                lhs,
                rhs,
            } if type_test(lhs, rhs).is_some() => {
                let (operand, nil) = type_test(lhs, rhs).expect("a type test");
                self.expr(type_callee(lhs, rhs));
                let keys = self.test_operand(operand);
                self.expr(if matches!(lhs.kind, ExprKind::String(_)) {
                    lhs
                } else {
                    rhs
                });
                if (*op == BinOp::Eq) != nil {
                    keys
                } else {
                    Vec::new()
                }
            }
            ExprKind::Paren(inner) => self.test(inner),
            _ => self.test_operand(e),
        }
    }

    /// Walks an operand of an existence test: a plain global or `ns` field
    /// read there is guarded.
    fn test_operand(&mut self, e: &Expr) -> Vec<Key> {
        let start = self.out.uses.len();
        let info = self.expr(e);
        let plain = matches!(
            e.kind,
            ExprKind::Name(_)
                | ExprKind::Field { .. }
                | ExprKind::Index { .. }
                | ExprKind::OptField { .. }
        );
        if let Info::Path((key, fields)) = &info
            && fields.is_empty()
            && plain
        {
            if let Some(access) = self.out.uses[start..]
                .iter_mut()
                .rev()
                .find(|a| a.key == *key)
            {
                access.guarded = true;
            }
            return vec![key.clone()];
        }
        Vec::new()
    }

    /// Walks a block with extra guarded keys.
    fn guarded_block(&mut self, keys: Vec<Key>, b: &Block) {
        let n = self.guards.len();
        self.guards.extend(keys);
        self.block(b);
        self.guards.truncate(n);
    }

    fn block(&mut self, b: &Block) {
        self.scopes.push(HashMap::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        grow(|| self.stmt_inner(s))
    }

    fn stmt_inner(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Local { names, values } => {
                let infos: Vec<Info> = values.iter().map(|v| self.expr(v)).collect();
                for (i, n) in names.iter().enumerate() {
                    let local = match infos.get(i) {
                        Some(Info::Function(f)) => Local::Function(*f),
                        Some(Info::Param(p)) => Local::Param(*p),
                        Some(Info::Path(p)) => Local::Alias(p.clone()),
                        Some(Info::Ns) => Local::Ns,
                        _ if i == 1
                            && values.len() == 1
                            && matches!(infos.first(), Some(Info::VarargNs)) =>
                        {
                            Local::Ns
                        }
                        _ => Local::Plain,
                    };
                    self.declare(&n.name.text, local);
                }
            }
            StmtKind::Assign { targets, values } => {
                let infos: Vec<Info> = values.iter().map(|v| self.expr(v)).collect();
                for (i, t) in targets.iter().enumerate() {
                    self.assign_target(t, infos.get(i), t.span);
                }
            }
            StmtKind::Compound { target, value, .. } => {
                self.expr(target);
                self.expr(value);
                self.assign_target(target, None, target.span);
            }
            StmtKind::Call(e) => {
                self.expr(e);
            }
            StmtKind::Do(b) => self.block(b),
            StmtKind::While { cond, body } => {
                let keys = self.test(cond);
                self.guarded_block(keys, body);
            }
            StmtKind::Repeat { body, cond } => {
                self.scopes.push(HashMap::new());
                for s in &body.stmts {
                    self.stmt(s);
                }
                self.test(cond);
                self.scopes.pop();
            }
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                let keys = self.test(cond);
                self.guarded_block(keys, then_block);
                for (c, b) in elseifs {
                    let keys = self.test(c);
                    self.guarded_block(keys, b);
                }
                if let Some(b) = else_block {
                    self.block(b);
                }
            }
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => {
                self.expr(start);
                self.expr(limit);
                if let Some(s) = step {
                    self.expr(s);
                }
                self.scopes.push(HashMap::new());
                self.declare(&var.name.text, Local::Plain);
                self.block(body);
                self.scopes.pop();
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                for e in exprs {
                    self.expr(e);
                }
                self.scopes.push(HashMap::new());
                for v in vars {
                    self.declare(&v.name.text, Local::Plain);
                }
                self.block(body);
                self.scopes.pop();
            }
            StmtKind::Function { name, func } => {
                let display = func_name_display(name);
                let f = self.function(func, &display, name.method.is_some());
                let root = &name.path[0];
                let mut fields: Vec<String> =
                    name.path[1..].iter().map(|n| n.text.to_string()).collect();
                if let Some(m) = &name.method {
                    fields.push(m.text.to_string());
                }
                match self.lookup(&root.text).cloned() {
                    Some(Local::Plain) | Some(Local::Function(_)) if fields.is_empty() => {
                        self.declare_existing(&root.text, Local::Function(f));
                    }
                    Some(Local::Ns) => {
                        if let Some((first, rest)) = fields.split_first() {
                            let key = Key::Ns(first.clone());
                            if rest.is_empty() {
                                self.def_key(key.clone(), root.span);
                            } else {
                                self.use_key(key.clone(), root.span);
                            }
                            self.out.function_defs.push(((key, rest.to_vec()), f));
                        }
                    }
                    Some(Local::Alias((key, prefix))) => {
                        self.use_key(key.clone(), root.span);
                        let mut path = prefix.clone();
                        path.extend(fields);
                        self.out.function_defs.push(((key, path), f));
                    }
                    Some(_) => {}
                    None => {
                        let key = Key::Global(root.text.to_string());
                        if fields.is_empty() {
                            self.def_key(key.clone(), root.span);
                        } else {
                            self.use_key(key.clone(), root.span);
                        }
                        self.out.function_defs.push(((key, fields), f));
                    }
                }
            }
            StmtKind::LocalFunction { name, func } => {
                let id = self.out.functions.len();
                self.declare(&name.text, Local::Function(id));
                self.function(func, &name.text, false);
            }
            StmtKind::Return(values) => {
                for v in values {
                    self.expr(v);
                }
            }
            StmtKind::LocalDestructure { fields, value, .. } => {
                if let Some(v) = value {
                    self.expr(v);
                }
                for f in fields {
                    if let Some(d) = &f.default {
                        self.expr(d);
                    }
                    self.declare(&f.name.text, Local::Plain);
                }
            }
            StmtKind::Const { name, value, .. } => {
                let info = value.as_ref().map(|v| self.expr(v));
                let local = match info {
                    Some(Info::Function(f)) => Local::Function(f),
                    _ => Local::Plain,
                };
                self.declare(&name.text, local);
            }
            StmtKind::Import(import) => match &import.clause {
                ImportClause::Names(specs) => {
                    for spec in specs {
                        let local = spec.alias.as_ref().unwrap_or(&spec.name);
                        self.declare(&local.text, Local::Import(local.text.to_string()));
                    }
                }
                ImportClause::Namespace(n) => {
                    self.declare(&n.text, Local::Import(n.text.to_string()))
                }
                ImportClause::SideEffect => {}
            },
            StmtKind::Export(inner) => {
                match &inner.kind {
                    StmtKind::Function { name, func } => {
                        let n = name.path[0].text.to_string();
                        let f = self.function(func, &n, false);
                        self.declare(&n, Local::Function(f));
                        self.out.exported_functions.insert(n, f);
                        return;
                    }
                    StmtKind::Local { names, values } => {
                        self.stmt(inner);
                        for (i, n) in names.iter().enumerate() {
                            if let Some(Expr {
                                kind: ExprKind::Function(_),
                                ..
                            }) = values.get(i)
                                && let Some(Local::Function(f)) = self.lookup(&n.name.text).cloned()
                            {
                                self.out
                                    .exported_functions
                                    .insert(n.name.text.to_string(), f);
                            }
                        }
                        return;
                    }
                    _ => {}
                }
                self.stmt(inner);
            }
            StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Class(_)
            | StmtKind::Interface(_)
            | StmtKind::Enum(_)
            | StmtKind::TypeAlias(_)
            | StmtKind::Declare(_)
            | StmtKind::Error => {}
        }
    }

    /// Updates the binding of an existing local (an assignment to it).
    fn declare_existing(&mut self, name: &str, local: Local) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(slot) = scope.get_mut(name) {
                *slot = local;
                return;
            }
        }
    }

    fn assign_target(&mut self, t: &Expr, value: Option<&Info>, span: Span) {
        let function = match value {
            Some(Info::Function(f)) => Some(*f),
            _ => None,
        };
        match &t.kind {
            ExprKind::Name(n) => match self.lookup(&n.text) {
                Some(_) => {
                    if let Some(f) = function {
                        self.declare_existing(&n.text, Local::Function(f));
                    }
                }
                None => {
                    let key = Key::Global(n.text.to_string());
                    self.def_key(key.clone(), span);
                    if let Some(f) = function {
                        self.out.function_defs.push(((key, Vec::new()), f));
                    }
                }
            },
            ExprKind::Field { obj, .. } | ExprKind::Index { obj, .. } => {
                let obj_info = self.expr(obj);
                let field = match &t.kind {
                    ExprKind::Field { name, .. } => Some(name.text.to_string()),
                    ExprKind::Index { key, .. } => {
                        self.expr(key);
                        match &key.kind {
                            ExprKind::String(s) => {
                                Some(String::from_utf8_lossy(&s.value).into_owned())
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match (obj_info, field) {
                    (Info::Ns, Some(f)) => {
                        self.def_key(Key::Ns(f.clone()), span);
                        if let Some(func) = function {
                            self.out
                                .function_defs
                                .push(((Key::Ns(f), Vec::new()), func));
                        }
                    }
                    (Info::GlobalTable, Some(f)) => {
                        self.def_key(Key::Global(f.clone()), span);
                        if let Some(func) = function {
                            self.out
                                .function_defs
                                .push(((Key::Global(f), Vec::new()), func));
                        }
                    }
                    (Info::Ns | Info::GlobalTable, None) => {
                        self.out.dynamic_write.get_or_insert(span);
                    }
                    (Info::Path((key, mut fields)), Some(f)) => {
                        if let Some(func) = function {
                            fields.push(f);
                            self.out.function_defs.push(((key, fields), func));
                        }
                    }
                    _ => {}
                }
            }
            _ => {
                self.expr(t);
            }
        }
    }

    /// Walks a function body as a new function; returns its index.
    fn function(&mut self, f: &FuncBody, name: &str, method: bool) -> usize {
        let id = self.out.functions.len();
        self.out.functions.push(Function {
            name: name.to_string(),
            calls: Vec::new(),
            called_params: HashSet::new(),
        });
        let saved = self.current;
        let saved_guards = std::mem::take(&mut self.guards);
        self.current = id;
        self.scopes.push(HashMap::new());
        if method {
            self.declare("self", Local::Param(0));
        }
        for (i, p) in f.params.iter().enumerate() {
            self.declare(&p.name.text, Local::Param(i + usize::from(method)));
            if let Some(d) = &p.default {
                self.expr(d);
            }
        }
        for s in &f.body.stmts {
            self.stmt(s);
        }
        self.scopes.pop();
        self.current = saved;
        self.guards = saved_guards;
        id
    }

    fn call_target(info: &Info) -> CallTarget {
        match info {
            Info::Function(f) => CallTarget::Local(*f),
            Info::Path(p) => CallTarget::Path(p.clone()),
            Info::Import(b, fields) => CallTarget::Import {
                binding: b.clone(),
                fields: fields.clone(),
            },
            Info::Param(i) => CallTarget::Param(*i),
            _ => CallTarget::Unknown,
        }
    }

    fn path_text(path: &Path) -> String {
        let mut parts = vec![match &path.0 {
            Key::Global(n) => n.clone(),
            Key::Ns(n) => format!("ns.{n}"),
            Key::Template(n) => format!("template {n}"),
        }];
        parts.extend(path.1.iter().cloned());
        parts.join(".")
    }

    /// Records a call; `offset` is 1 for method calls (the object is
    /// argument 0).
    fn record_call(
        &mut self,
        target: CallTarget,
        args: &[Info],
        offset: usize,
        span: Span,
        deferred: bool,
    ) {
        let args = args
            .iter()
            .enumerate()
            .map(|(i, a)| (i + offset, Self::call_target(a)))
            .filter(|(_, t)| !matches!(t, CallTarget::Unknown))
            .collect();
        let current = self.current;
        if let CallTarget::Param(p) = target {
            self.out.functions[current].called_params.insert(p);
        }
        self.out.functions[current].calls.push(Call {
            target,
            span,
            args,
            deferred,
        });
    }

    fn args(&mut self, args: &Args) -> Vec<Info> {
        args.exprs().into_iter().map(|a| self.expr(a)).collect()
    }

    fn expr(&mut self, e: &Expr) -> Info {
        grow(|| self.expr_inner(e))
    }

    fn expr_inner(&mut self, e: &Expr) -> Info {
        match &e.kind {
            ExprKind::Name(n) => match self.lookup(&n.text).cloned() {
                Some(Local::Function(f)) => Info::Function(f),
                Some(Local::Ns) => Info::Ns,
                Some(Local::Alias(p)) => Info::Path(p),
                Some(Local::Import(b)) => {
                    let current = self.current;
                    self.out.import_uses.push((b.clone(), current, n.span));
                    Info::Import(b, Vec::new())
                }
                Some(Local::Param(i)) => Info::Param(i),
                Some(Local::Plain) => Info::Other,
                None if n.text.as_ref() == "_G" => Info::GlobalTable,
                None => {
                    let key = Key::Global(n.text.to_string());
                    self.use_key(key.clone(), n.span);
                    Info::Path((key, Vec::new()))
                }
            },
            ExprKind::Vararg => {
                if self.current == 0 {
                    Info::VarargNs
                } else {
                    Info::Other
                }
            }
            ExprKind::Field { obj, name } | ExprKind::OptField { obj, name } => {
                let o = self.expr(obj);
                self.field(o, name.text.to_string(), name.span)
            }
            ExprKind::Index { obj, key } | ExprKind::OptIndex { obj, key } => {
                let o = self.expr(obj);
                self.expr(key);
                match &key.kind {
                    ExprKind::String(s) => {
                        self.field(o, String::from_utf8_lossy(&s.value).into_owned(), key.span)
                    }
                    _ => Info::Other,
                }
            }
            ExprKind::Call { callee, args } | ExprKind::OptCall { callee, args } => {
                let c = self.expr(callee);
                // `type(X)` tests X's existence.
                let arg_infos = match (&c, args.exprs().as_slice()) {
                    (Info::Path((Key::Global(g), fields)), [only])
                        if g == "type" && fields.is_empty() =>
                    {
                        self.test_operand(only);
                        vec![Info::Other]
                    }
                    _ => self.args(args),
                };
                // `select(2, ...)` is the addon table.
                if let Info::Path((Key::Global(g), fields)) = &c
                    && g == "select"
                    && fields.is_empty()
                    && matches!(args.exprs().first().map(|a| &a.kind), Some(ExprKind::Number(n)) if n.value == Some(2.0))
                    && matches!(arg_infos.get(1), Some(Info::VarargNs))
                {
                    return Info::Ns;
                }
                if let Info::Path((Key::Global(g), fields)) = &c
                    && g == "setfenv"
                    && fields.is_empty()
                {
                    self.out.dynamic_write.get_or_insert(e.span);
                }
                // `X.RegisterCallback(…)` matches the method markers too.
                let deferred = match &c {
                    Info::Path(p) => {
                        let last = p.1.last().or(match &p.0 {
                            Key::Ns(m) => Some(m),
                            Key::Global(_) | Key::Template(_) => None,
                        });
                        self.markers
                            .deferred_functions
                            .contains(&Self::path_text(p))
                            || last.is_some_and(|m| self.markers.deferred_methods.contains(m))
                    }
                    _ => false,
                };
                self.record_call(Self::call_target(&c), &arg_infos, 0, e.span, deferred);
                Info::Other
            }
            ExprKind::Method { obj, name, args } | ExprKind::OptMethod { obj, name, args } => {
                let o = self.expr(obj);
                let arg_infos = self.args(args);
                let target = match &o {
                    Info::Path((k, fields)) => {
                        let mut fields = fields.clone();
                        fields.push(name.text.to_string());
                        CallTarget::Path((k.clone(), fields))
                    }
                    Info::Ns => CallTarget::Path((Key::Ns(name.text.to_string()), Vec::new())),
                    Info::Import(b, fields) => {
                        let mut fields = fields.clone();
                        fields.push(name.text.to_string());
                        CallTarget::Import {
                            binding: b.clone(),
                            fields,
                        }
                    }
                    _ => CallTarget::Unknown,
                };
                let deferred = self.markers.deferred_methods.contains(name.text.as_ref());
                self.record_call(target, &arg_infos, 1, e.span, deferred);
                Info::Other
            }
            ExprKind::Function(f) => {
                Info::Function(self.function(f, &format!("function at {}", e.span.start), false))
            }
            ExprKind::Table(fields) => {
                for field in fields {
                    match field {
                        TableField::Keyed { key, value, .. } => {
                            self.expr(key);
                            self.expr(value);
                        }
                        TableField::Named { value, .. } | TableField::Positional(value) => {
                            self.expr(value);
                        }
                    }
                }
                Info::Other
            }
            ExprKind::Paren(inner) => match self.expr(inner) {
                Info::VarargNs => Info::Other,
                info => info,
            },
            ExprKind::Cast { expr, .. } => self.expr(expr),
            ExprKind::Binary {
                op: BinOp::And,
                lhs,
                rhs,
            } => {
                let keys = self.test(lhs);
                let n = self.guards.len();
                self.guards.extend(keys);
                self.expr(rhs);
                self.guards.truncate(n);
                Info::Other
            }
            ExprKind::Binary {
                op: BinOp::Or | BinOp::Coalesce,
                lhs,
                rhs,
            } => {
                self.test(lhs);
                self.expr(rhs);
                Info::Other
            }
            ExprKind::Binary {
                op: BinOp::Eq | BinOp::Ne,
                lhs,
                rhs,
            } if matches!(lhs.kind, ExprKind::Nil) || matches!(rhs.kind, ExprKind::Nil) => {
                self.test(e);
                Info::Other
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
                Info::Other
            }
            ExprKind::Unary {
                op: UnOp::Not,
                operand,
            } => {
                self.test(operand);
                Info::Other
            }
            ExprKind::Unary { operand, .. } => {
                self.expr(operand);
                Info::Other
            }
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.expr(x);
                    }
                }
                Info::Other
            }
            ExprKind::Nil
            | ExprKind::True
            | ExprKind::False
            | ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::Error => Info::Other,
        }
    }

    fn field(&mut self, obj: Info, field: String, span: Span) -> Info {
        match obj {
            Info::Ns => {
                let key = Key::Ns(field);
                self.use_key(key.clone(), span);
                Info::Path((key, Vec::new()))
            }
            Info::GlobalTable => {
                let key = Key::Global(field);
                self.use_key(key.clone(), span);
                Info::Path((key, Vec::new()))
            }
            Info::Path((k, mut fields)) => {
                fields.push(field);
                Info::Path((k, fields))
            }
            Info::Import(b, mut fields) => {
                fields.push(field);
                Info::Import(b, fields)
            }
            _ => Info::Other,
        }
    }
}

/// `type(X) == "s"` in either operand order: X and whether `s` is `"nil"`.
fn type_test<'e>(lhs: &'e Expr, rhs: &'e Expr) -> Option<(&'e Expr, bool)> {
    let (call, string) = match (&lhs.kind, &rhs.kind) {
        (ExprKind::Call { .. }, ExprKind::String(s)) => (lhs, s),
        (ExprKind::String(s), ExprKind::Call { .. }) => (rhs, s),
        _ => return None,
    };
    let ExprKind::Call { callee, args } = &call.kind else {
        return None;
    };
    let ExprKind::Name(n) = &callee.kind else {
        return None;
    };
    match args.exprs().as_slice() {
        [only] if n.text.as_ref() == "type" => Some((only, *string.value == *b"nil")),
        _ => None,
    }
}

/// The `type` callee of a type test.
fn type_callee<'e>(lhs: &'e Expr, rhs: &'e Expr) -> &'e Expr {
    let call = if matches!(lhs.kind, ExprKind::Call { .. }) {
        lhs
    } else {
        rhs
    };
    match &call.kind {
        ExprKind::Call { callee, .. } => callee,
        _ => call,
    }
}

fn func_name_display(name: &FuncName) -> String {
    let parts: Vec<&str> = name.path.iter().map(|n| n.text.as_ref()).collect();
    let path = parts.join(".");
    match &name.method {
        Some(m) => format!("{path}:{}", m.text),
        None => path,
    }
}
