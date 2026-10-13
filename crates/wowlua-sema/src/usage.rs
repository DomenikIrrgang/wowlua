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

/// A name whose definition orders files: a global, an addon-table field,
/// or a field of a table reached from one of them (`Lib.Register`,
/// `ns.UI.Icon`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Global(String),
    Ns(String),
    /// An XML template (`virtual="true"`).
    Template(String),
    /// A field path below a global or `ns` key (never empty, never below
    /// another field key).
    Field(Box<Key>, Vec<String>),
    /// A table constructor held by a local (`local Roles = {}`), named by
    /// its file, its declaration offset and the local's name. Its fields
    /// order files once a global or `ns` key holds the same table
    /// (`ns.Roles = Roles`).
    LocalTable(FileId, u32, String),
}

impl Key {
    pub fn display(&self) -> String {
        match self {
            Key::Global(n) => n.clone(),
            Key::Ns(n) => format!("ns.{n}"),
            Key::Template(n) => format!("template {n}"),
            Key::Field(root, fields) => format!("{}.{}", root.display(), fields.join(".")),
            Key::LocalTable(_, _, n) => n.clone(),
        }
    }

    /// The key of a path: the root itself, or a field key below it.
    pub fn of_path(root: Key, fields: Vec<String>) -> Key {
        if fields.is_empty() {
            root
        } else {
            Key::Field(Box::new(root), fields)
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
    /// Two global or `ns` keys holding the same table (`ns.API = SuuUI`):
    /// the fields of one are the fields of the other.
    pub table_aliases: Vec<(Key, Key)>,
    /// Field writes into tables the analysis cannot trace (locals,
    /// parameters, call results): (field, function, span).
    pub untracked_field_writes: Vec<(String, usize, Span)>,
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
    // Top-level classes and enums are hoisted locals.
    for s in &parse.chunk.block.stmts {
        let kind = match &s.kind {
            StmtKind::Export(inner) => &inner.kind,
            k => k,
        };
        match kind {
            StmtKind::Class(c) => w.declare(&c.name.text, Local::Plain),
            StmtKind::Enum(e) => w.declare(&e.name.text, Local::Plain),
            _ => {}
        }
    }
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
        // A field use is also guarded when its root or a shorter path is
        // tested (`Later and Later.y`).
        let guarded = self.guards.contains(&key)
            || match &key {
                Key::Field(root, fields) => {
                    self.guards.contains(root)
                        || (1..fields.len()).any(|n| {
                            self.guards
                                .contains(&Key::Field(root.clone(), fields[..n].to_vec()))
                        })
                }
                _ => false,
            };
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
            && plain
        {
            let key = Key::of_path(key.clone(), fields.clone());
            if let Some(access) = self.out.uses[start..]
                .iter_mut()
                .rev()
                .find(|a| a.key == key)
            {
                access.guarded = true;
            }
            return vec![key];
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
        let first = self.out.functions.len();
        grow(|| self.stmt_inner(s));
        self.decorators(&s.decorators, first);
    }

    /// User decorators are called at load time with the declared function
    /// (the first function the declaration created, if any).
    fn decorators(&mut self, decorators: &[Decorator], first: usize) {
        for d in decorators.iter().filter(|d| !d.is_core()) {
            let callee = self.expr(&d.expr());
            let target: Vec<Info> = (first < self.out.functions.len())
                .then_some(Info::Function(first))
                .into_iter()
                .collect();
            self.record_call(Self::call_target(&callee), &target, 0, d.span, false);
        }
    }

    /// A class: `extends` is read and static initializers run while the
    /// file loads; methods, accessors, the constructor and instance field
    /// initializers are functions.
    fn class_decl(&mut self, c: &ClassDecl) {
        if let Some(path) = c.extends.as_ref().and_then(crate::classes::type_path) {
            let mk = |kind: ExprKind| Expr {
                id: NodeId(0),
                span: c.name.span,
                kind,
            };
            let mut e = mk(ExprKind::Name(Name {
                text: path[0].clone().into(),
                span: c.name.span,
            }));
            for p in &path[1..] {
                e = mk(ExprKind::Field {
                    obj: Box::new(e),
                    name: Name {
                        text: p.clone().into(),
                        span: c.name.span,
                    },
                });
            }
            self.expr(&e);
        }
        let class = c.name.text.to_string();
        for m in &c.members {
            let first = self.out.functions.len();
            let is_static = m.modifiers.contains(&Modifier::Static);
            match &m.kind {
                ClassMemberKind::Field { value: Some(v), .. } if is_static => {
                    self.expr(v);
                }
                ClassMemberKind::Field {
                    name,
                    value: Some(v),
                    ..
                } => {
                    self.body_function(&format!("{class} field {}", name.text), true, &[], |w| {
                        w.expr(v);
                    });
                }
                ClassMemberKind::Method {
                    name,
                    func,
                    has_body: true,
                } => {
                    self.function(func, &format!("{class}:{}", name.text), !is_static);
                }
                ClassMemberKind::Constructor {
                    func,
                    has_body: true,
                } => {
                    self.function(func, &format!("{class} constructor"), true);
                }
                ClassMemberKind::Getter { name, body, .. } => {
                    self.body_function(
                        &format!("{class}.{} getter", name.text),
                        !is_static,
                        &[],
                        |w| w.block(body),
                    );
                }
                ClassMemberKind::Setter { name, param, body } => {
                    let p = param.name.text.to_string();
                    self.body_function(
                        &format!("{class}.{} setter", name.text),
                        !is_static,
                        &[p],
                        |w| w.block(body),
                    );
                }
                _ => {}
            }
            self.decorators(&m.decorators, first);
        }
    }

    /// A function without a `FuncBody` (accessors, field initializers).
    fn body_function(
        &mut self,
        name: &str,
        method: bool,
        params: &[String],
        f: impl FnOnce(&mut Self),
    ) -> usize {
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
        for (i, p) in params.iter().enumerate() {
            self.declare(p, Local::Param(i + usize::from(method)));
        }
        f(self);
        self.scopes.pop();
        self.current = saved;
        self.guards = saved_guards;
        id
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
                        _ => match values.get(i) {
                            Some(
                                value @ Expr {
                                    kind: ExprKind::Table(_),
                                    ..
                                },
                            ) => {
                                let path = (
                                    Key::LocalTable(
                                        n.name.span.file,
                                        n.name.span.start,
                                        n.name.text.to_string(),
                                    ),
                                    Vec::new(),
                                );
                                self.constructor_fields(&path, value);
                                Local::Alias(path)
                            }
                            _ => Local::Plain,
                        },
                    };
                    self.declare(&n.name.text, local);
                }
            }
            StmtKind::Assign { targets, values } => {
                let infos: Vec<Info> = values.iter().map(|v| self.expr(v)).collect();
                for (i, t) in targets.iter().enumerate() {
                    if let Some(path) = self.assign_target(t, infos.get(i), t.span, false)
                        && let Some(value) = values.get(i)
                    {
                        self.constructor_fields(&path, value);
                    }
                }
            }
            StmtKind::Compound { target, value, .. } => {
                self.expr(target);
                self.expr(value);
                self.assign_target(target, None, target.span, true);
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
                                self.def_key(Key::of_path(key.clone(), rest.to_vec()), s.span);
                            }
                            self.out.function_defs.push(((key, rest.to_vec()), f));
                        }
                    }
                    Some(Local::Alias((key, prefix))) => {
                        self.use_key(key.clone(), root.span);
                        let mut path = prefix.clone();
                        path.extend(fields);
                        if !path.is_empty() {
                            self.def_key(Key::of_path(key.clone(), path.clone()), s.span);
                        }
                        self.out.function_defs.push(((key, path), f));
                    }
                    Some(_) => {
                        if let Some(last) = fields.last() {
                            let current = self.current;
                            self.out
                                .untracked_field_writes
                                .push((last.clone(), current, s.span));
                        }
                    }
                    None => {
                        let key = Key::Global(root.text.to_string());
                        if fields.is_empty() {
                            self.def_key(key.clone(), root.span);
                        } else {
                            self.use_key(key.clone(), root.span);
                            self.def_key(Key::of_path(key.clone(), fields.clone()), s.span);
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
            StmtKind::Class(c) => self.class_decl(c),
            StmtKind::Break
            | StmtKind::Continue
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

    /// The global or `ns` key whose table `value` is (`SuuUI`, `ns.API`).
    fn table_of(value: Option<&Info>) -> Option<Key> {
        match value {
            Some(Info::Path((
                key @ (Key::Global(_) | Key::Ns(_) | Key::LocalTable(..)),
                fields,
            ))) if fields.is_empty() => Some(key.clone()),
            _ => None,
        }
    }

    /// `compound` is true for `X.f += 1`, which reads `X.f` rather than
    /// defining it. Returns the global or `ns` path the target defines.
    fn assign_target(
        &mut self,
        t: &Expr,
        value: Option<&Info>,
        span: Span,
        compound: bool,
    ) -> Option<Path> {
        let function = match value {
            Some(Info::Function(f)) => Some(*f),
            _ => None,
        };
        let table = Self::table_of(value);
        let (key, fields) = match &t.kind {
            ExprKind::Name(n) => match self.lookup(&n.text) {
                Some(_) => {
                    if let Some(f) = function {
                        self.declare_existing(&n.text, Local::Function(f));
                    }
                    return None;
                }
                None => (Key::Global(n.text.to_string()), Vec::new()),
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
                    (Info::Ns, Some(f)) => (Key::Ns(f), Vec::new()),
                    (Info::GlobalTable, Some(f)) => (Key::Global(f), Vec::new()),
                    (Info::Ns | Info::GlobalTable, None) => {
                        self.out.dynamic_write.get_or_insert(span);
                        return None;
                    }
                    (Info::Path((key, mut fields)), Some(f)) => {
                        fields.push(f);
                        (key, fields)
                    }
                    (Info::Other | Info::Param(_), Some(f)) if !compound => {
                        let current = self.current;
                        self.out.untracked_field_writes.push((f, current, span));
                        return None;
                    }
                    _ => return None,
                }
            }
            _ => {
                self.expr(t);
                return None;
            }
        };
        if fields.is_empty()
            && let Some(other) = table
        {
            self.out.table_aliases.push((key.clone(), other));
        }
        // `X.f += 1` reads `X.f`; a compound write to a global or `ns`
        // field still (re)defines it.
        let defines = !compound || fields.is_empty();
        if defines {
            self.def_key(Key::of_path(key.clone(), fields.clone()), span);
        }
        if let Some(func) = function {
            self.out
                .function_defs
                .push(((key.clone(), fields.clone()), func));
        }
        defines.then_some((key, fields))
    }

    /// Defines the fields of a table constructor assigned to `path`
    /// (`Lib = { Register = function() end }`).
    fn constructor_fields(&mut self, path: &Path, value: &Expr) {
        let ExprKind::Table(fields) = &value.kind else {
            return;
        };
        for field in fields {
            if let TableField::Named { name, value, span } = field {
                let mut inner = path.1.clone();
                inner.push(name.text.to_string());
                let inner = (path.0.clone(), inner);
                self.def_key(Key::of_path(inner.0.clone(), inner.1.clone()), *span);
                self.constructor_fields(&inner, value);
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
        let mut parts = vec![path.0.display()];
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
                            Key::Global(_)
                            | Key::Template(_)
                            | Key::Field(..)
                            | Key::LocalTable(..) => None,
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
                match &o {
                    Info::Path((k, fields)) => {
                        let mut fields = fields.clone();
                        fields.push(name.text.to_string());
                        self.use_key(Key::of_path(k.clone(), fields), name.span);
                    }
                    Info::Ns => self.use_key(Key::Ns(name.text.to_string()), name.span),
                    _ => {}
                }
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
                self.use_key(Key::of_path(k.clone(), fields.clone()), span);
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
