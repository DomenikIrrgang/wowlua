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
//! T70). Code that runs only when a parameter is not a function
//! (`if type(p) ~= "function"`) is tagged with it: it does not run for
//! calls that pass a function (decision T207).

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{HashMap, HashSet};
use wowlua_span::{FileId, Span};
use wowlua_syntax::Parse;
use wowlua_syntax::view::*;

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

#[derive(Clone, Debug, PartialEq)]
pub struct Access {
    pub key: Key,
    pub span: Span,
    /// The function the access is in (0 = the file's top level).
    pub function: usize,
    /// An existence test or a use guarded by one.
    pub guarded: bool,
    /// Runs only when this parameter of its function is not a function
    /// (`if type(p) ~= "function" then …`, `if not p then …`).
    pub unless_function: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CallTarget {
    Local(usize),
    Path(Path),
    Import {
        binding: String,
        fields: Vec<String>,
    },
    /// A parameter of the calling function (forwarded or called).
    Param(usize),
    /// A method of a class of the package, called on an instance of it:
    /// `self:m()` in its methods, `x:m()` on a local made with `C.new(…)`
    /// (the class's method, or a subclass's override; decision T269).
    ClassMethod {
        class: String,
        method: String,
    },
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub target: CallTarget,
    pub span: Span,
    /// Function values (and parameters) passed as arguments, by argument
    /// position (a method call's object is position 0).
    pub args: Vec<(usize, CallTarget)>,
    /// The callee runs passed functions later.
    pub deferred: bool,
    /// A method call's method name: an interface of the package may mark
    /// it `@deferred` (decision T269).
    pub method: Option<String>,
    /// Runs only when this parameter of the caller is not a function.
    pub unless_function: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub calls: Vec<Call>,
    /// Parameters the function calls (positions; `self` is 0 in methods).
    pub called_params: HashSet<usize>,
    /// Parameters the function assigns: a test of one says nothing about
    /// what the caller passed.
    pub reassigned_params: HashSet<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
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
    /// Methods this file's interfaces declare with a `@deferred` callback
    /// parameter (decision T269): their callers pass it to run later.
    pub deferred_methods: Vec<String>,
    /// This file's class methods: (class, method, function).
    pub class_methods: Vec<(String, String, usize)>,
    /// This file's classes' superclasses, by name: (class, base).
    pub class_bases: Vec<(String, String)>,
    /// Two global or `ns` keys holding the same table (`ns.API = SuuUI`):
    /// the fields of one are the fields of the other.
    pub table_aliases: Vec<(Key, Key)>,
    /// Field writes into tables the analysis cannot trace (locals,
    /// parameters, call results): (field, function, span).
    pub untracked_field_writes: Vec<(String, usize, Span)>,
}

/// APIs that run passed functions later (spec §5.4.3), those that run
/// them at once, and the API functions the declarations know.
#[derive(Clone, Debug, Default)]
pub struct Markers {
    pub deferred_functions: HashSet<String>,
    pub deferred_methods: HashSet<String>,
    pub immediate_functions: HashSet<String>,
    /// Lua's and the game's C API functions (`declare function`, namespace
    /// functions): they call no function passed to them, unless listed
    /// in `immediate_functions`.
    pub api_functions: HashSet<String>,
}

impl Markers {
    pub fn from_table() -> Markers {
        let m = &wowlua_flavors::table().load_order_markers;
        Markers {
            deferred_functions: m.deferred_functions.iter().cloned().collect(),
            deferred_methods: m.deferred_methods.iter().cloned().collect(),
            immediate_functions: m.immediate_functions.iter().cloned().collect(),
            api_functions: HashSet::new(),
        }
    }

    /// Adds the functions the declaration files declare (`Name`,
    /// `Namespace.Member`); declared globals of function type (Blizzard's
    /// Lua UI code) are not among them.
    pub fn with_api(mut self, files: &[crate::FileInput]) -> Markers {
        fn members(prefix: &str, list: NsMembers<'_>, out: &mut HashSet<String>) {
            for m in list {
                match m.kind() {
                    NsMemberKind::Function { name, .. } => {
                        out.insert(format!("{prefix}.{}", name.text()));
                    }
                    NsMemberKind::Namespace {
                        name,
                        members: inner,
                    } => {
                        members(&format!("{prefix}.{}", name.text()), inner, out);
                    }
                    _ => {}
                }
            }
        }
        for f in files
            .iter()
            .filter(|f| f.kind == crate::FileKind::Declaration)
        {
            let Some(parse) = f.parse else { continue };
            for s in parse.view().block().stmts() {
                let StmtKind::Declare(d) = s.kind() else {
                    continue;
                };
                match d.kind() {
                    DeclareKind::Function { name, .. } if name.method().is_none() => {
                        let path: Vec<&str> = name.path().iter().map(|n| n.text()).collect();
                        self.api_functions.insert(path.join("."));
                    }
                    DeclareKind::Namespace {
                        name,
                        members: list,
                    } => {
                        members(name.text(), list, &mut self.api_functions);
                    }
                    _ => {}
                }
            }
        }
        self
    }

    /// Whether a call of `path` (a global function or namespace member)
    /// cannot call the functions passed to it.
    pub fn calls_no_args(&self, path: &str) -> bool {
        self.api_functions.contains(path) && !self.immediate_functions.contains(path)
    }
}

#[derive(Clone, Debug)]
enum Local {
    Plain,
    /// An instance of a class of this file (`local x = C.new(…)`).
    Instance(String),
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
    /// An instance of a class of this file.
    Instance(String),
    VarargNs,
    Other,
}

struct Walker<'a> {
    out: FileUsage,
    /// The method name of the call being recorded (`record_call`).
    next_method: Option<String>,
    /// The class whose method body is walked (its `self`).
    class_ctx: Vec<String>,
    /// The classes this file declares.
    classes_here: FxHashSet<String>,
    scopes: Vec<FxHashMap<String, Local>>,
    current: usize,
    markers: &'a Markers,
    /// Keys known to be defined here (tested by an enclosing guard).
    guards: Vec<Key>,
    /// The expression being walked is the object of a chain link (`x` in
    /// `x.f`, `x?.f`, `x(…)`, `x:m()`).
    link: bool,
    /// Local names imported from the standard library (`wowlua:*`).
    stdlib_imports: FxHashSet<String>,
    /// The functions being walked, innermost last: the scope their
    /// parameters are in, and their id.
    frames: Vec<(usize, usize)>,
    /// The code being walked runs only when this parameter of the current
    /// function is not a function.
    unless_function: Option<usize>,
}

/// The usage summary of one file.
pub fn file_usage(parse: &Parse, markers: &Markers) -> FileUsage {
    wowlua_syntax::deep::run(|| file_usage_inner(parse, markers))
}

fn file_usage_inner(parse: &Parse, markers: &Markers) -> FileUsage {
    let mut w = Walker::new(markers);
    let block = parse.view().block();
    let module = block
        .stmts()
        .iter()
        .any(|s| matches!(s.kind(), StmtKind::Import(_) | StmtKind::Export(_)));
    // Top-level classes and enums are hoisted locals, and so are a module's
    // top-level functions (spec §5.1), except one that defines a local
    // declared above it (`local f` … `function f()`).
    let mut locals: FxHashSet<&str> = FxHashSet::default();
    for s in block.stmts() {
        let kind = match s.kind() {
            StmtKind::Export(inner) => inner.kind(),
            k => k,
        };
        match kind {
            StmtKind::Class(c) => w.declare(c.name().text(), Local::Plain),
            StmtKind::Enum(e) => w.declare(e.name().text(), Local::Plain),
            StmtKind::Function { name, .. }
                if module
                    && matches!(s.kind(), StmtKind::Function { .. })
                    && name.path().len() == 1
                    && name.method().is_none()
                    && !locals.contains(name.path().at(0).text()) =>
            {
                w.declare(name.path().at(0).text(), Local::Plain);
            }
            StmtKind::Local { names, .. } => {
                locals.extend(names.iter().map(|n| n.name.text()));
            }
            StmtKind::LocalDestructure { fields, .. } => {
                locals.extend(fields.iter().map(|f| f.name.text()));
            }
            StmtKind::LocalFunction { name, .. } | StmtKind::Const { name, .. } => {
                locals.insert(name.text());
            }
            _ => {}
        }
    }
    w.block(block);
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
            reassigned_params: HashSet::new(),
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
                    reassigned_params: HashSet::new(),
                }],
                ..FileUsage::default()
            },
            next_method: None,
            class_ctx: Vec::new(),
            classes_here: FxHashSet::default(),
            scopes: vec![FxHashMap::default()],
            current: 0,
            markers,
            guards: Vec::new(),
            link: false,
            stdlib_imports: FxHashSet::default(),
            frames: vec![(0, 0)],
            unless_function: None,
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
            self.scopes.push(FxHashMap::default());
            self.declare("self", Local::Plain);
            for s in parse.view().block().stmts() {
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

    /// The function a parameter named `name` belongs to and its position,
    /// if the name is a parameter.
    fn param_owner(&self, name: &str) -> Option<(usize, usize)> {
        let (scope, local) = self
            .scopes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, s)| s.get(name).map(|l| (i, l)))?;
        let Local::Param(p) = local else {
            return None;
        };
        let owner = self.frames.iter().rev().find(|f| f.0 <= scope)?.1;
        Some((owner, *p))
    }

    /// The position of a parameter of the current function that `e` names.
    fn own_param(&self, e: Expr<'_>) -> Option<usize> {
        let ExprKind::Name(n) = e.kind() else {
            return None;
        };
        self.param_owner(n.text())
            .filter(|(owner, _)| *owner == self.current)
            .map(|(_, p)| p)
    }

    /// The parameter that is no function when `cond` holds (`not p`,
    /// `p == nil`, `type(p) ~= "function"`), or, with `holds` false, when
    /// it does not hold (`p`, `p ~= nil`, `type(p) == "function"`).
    fn not_function_param(&self, cond: Expr<'_>, holds: bool) -> Option<usize> {
        match cond.kind() {
            ExprKind::Paren(inner) => self.not_function_param(inner, holds),
            ExprKind::Unary {
                op: UnOp::Not,
                operand,
            } => self.not_function_param(operand, !holds),
            ExprKind::Name(_) if !holds => self.own_param(cond),
            ExprKind::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                lhs,
                rhs,
            } => {
                let equal = (op == BinOp::Eq) == holds;
                if matches!(rhs.kind(), ExprKind::Nil) {
                    return equal.then(|| self.own_param(lhs)).flatten();
                }
                if matches!(lhs.kind(), ExprKind::Nil) {
                    return equal.then(|| self.own_param(rhs)).flatten();
                }
                let (operand, _) = type_test(lhs, rhs)?;
                let tested = if matches!(lhs.kind(), ExprKind::String(_)) {
                    lhs
                } else {
                    rhs
                };
                let ExprKind::String(t) = tested.kind() else {
                    return None;
                };
                (*t.value() == *b"function" && !equal)
                    .then(|| self.own_param(operand))
                    .flatten()
            }
            _ => None,
        }
    }

    /// Walks a block that runs only when parameter `param` (if any) is not
    /// a function; a condition already in force stays.
    fn unless_block(&mut self, param: Option<usize>, keys: Vec<Key>, b: Block<'_>) {
        let saved = self.unless_function;
        if saved.is_none() {
            self.unless_function = param;
        }
        self.guarded_block(keys, b);
        self.unless_function = saved;
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
            unless_function: self.unless_function,
        });
    }

    fn def_key(&mut self, key: Key, span: Span) {
        self.out.defs.push(Access {
            key,
            span,
            function: self.current,
            guarded: false,
            unless_function: self.unless_function,
        });
    }

    /// Walks an expression tested for truthiness; returns the keys that are
    /// defined when it is true.
    fn test(&mut self, e: Expr<'_>) -> Vec<Key> {
        match e.kind() {
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
            } if matches!(lhs.kind(), ExprKind::Nil) || matches!(rhs.kind(), ExprKind::Nil) => {
                let operand = if matches!(lhs.kind(), ExprKind::Nil) {
                    rhs
                } else {
                    lhs
                };
                let keys = self.test_operand(operand);
                if op == BinOp::Ne { keys } else { Vec::new() }
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
                self.expr(if matches!(lhs.kind(), ExprKind::String(_)) {
                    lhs
                } else {
                    rhs
                });
                if (op == BinOp::Eq) != nil {
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
    fn test_operand(&mut self, e: Expr<'_>) -> Vec<Key> {
        let start = self.out.uses.len();
        let info = self.expr(e);
        self.probe(start, e, &info)
    }

    /// Marks the read of `e` (walked from use `start`, giving `info`) as an
    /// existence probe.
    fn probe(&mut self, start: usize, e: Expr<'_>, info: &Info) -> Vec<Key> {
        let plain = matches!(
            e.kind(),
            ExprKind::Name(_)
                | ExprKind::Field { .. }
                | ExprKind::Index { .. }
                | ExprKind::OptField { .. }
        );
        if let Info::Path((key, fields)) = info
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

    /// Walks the object of an optional link (`X?.f`, `X?:m()`, `X?.()`): an
    /// existence probe of X.
    fn probed(&mut self, e: Expr<'_>) -> Info {
        let start = self.out.uses.len();
        let info = self.link(e);
        let keys = self.probe(start, e, &info);
        self.guards.extend(keys);
        info
    }

    /// Walks a block with extra guarded keys.
    fn guarded_block(&mut self, keys: Vec<Key>, b: Block<'_>) {
        let n = self.guards.len();
        self.guards.extend(keys);
        self.block(b);
        self.guards.truncate(n);
    }

    fn block(&mut self, b: Block<'_>) {
        self.scopes.push(FxHashMap::default());
        for s in b.stmts() {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: Stmt<'_>) {
        let first = self.out.functions.len();
        grow(|| self.stmt_inner(s));
        self.decorators(s.decorators(), first);
    }

    /// User decorators are called at load time with the declared function
    /// (the first function the declaration created, if any). The standard
    /// library's decorators only register or wrap it: it runs later.
    fn decorators(&mut self, decorators: Decorators<'_>, first: usize) {
        for d in decorators.iter().filter(|d| !d.is_core()) {
            let callee = self.expr(d.expr());
            let target: Vec<Info> = (first < self.out.functions.len())
                .then_some(Info::Function(first))
                .into_iter()
                .collect();
            let deferred = d
                .path()
                .first()
                .is_some_and(|n| self.stdlib_imports.contains(n.text()));
            self.record_call(Self::call_target(&callee), &target, 0, d.span(), deferred);
        }
    }

    /// A class: `extends` is read and static initializers run while the
    /// file loads; methods, accessors, the constructor and instance field
    /// initializers are functions.
    /// The class a value is a new instance of: `C.new(…)` or `C(…)` with
    /// `C` a class of this file.
    fn instance_of(&self, e: Expr<'_>) -> Option<String> {
        let (ExprKind::Call { callee, .. } | ExprKind::OptCall { callee, .. }) = e.kind() else {
            return None;
        };
        let class = match callee.kind() {
            ExprKind::Field { obj, name } if name.text() == "new" => match obj.kind() {
                ExprKind::Name(n) => n.text(),
                _ => return None,
            },
            ExprKind::Name(n) => n.text(),
            _ => return None,
        };
        self.classes_here.contains(class).then(|| class.to_string())
    }

    /// An interface's methods with a `@deferred` callback parameter
    /// (decision T269).
    fn interface_decl(&mut self, i: InterfaceDecl<'_>) {
        for m in &i.members() {
            if let InterfaceMember::Method { name, sig } = m
                && sig.params().iter().any(|p| {
                    p.decorators().iter().any(|d| {
                        d.path()
                            .exactly::<1>()
                            .is_some_and(|[n]| n.text() == "deferred")
                    })
                })
            {
                self.out.deferred_methods.push(name.text().to_string());
            }
        }
    }

    fn class_decl(&mut self, c: ClassDecl<'_>) {
        if let Some(path) = c.extends().and_then(crate::classes::type_path) {
            self.path_read(&path, c.name().span());
            if let Some(base) = path.last() {
                self.out
                    .class_bases
                    .push((c.name().text().to_string(), base.clone()));
            }
        }
        let class = c.name().text().to_string();
        self.classes_here.insert(class.clone());
        self.class_ctx.push(class.clone());
        for m in c.members() {
            let first = self.out.functions.len();
            let is_static = m.modifiers().contains(&Modifier::Static);
            match m.kind() {
                ClassMemberKind::Field { value: Some(v), .. } if is_static => {
                    self.expr(v);
                }
                ClassMemberKind::Field {
                    name,
                    value: Some(v),
                    ..
                } => {
                    self.body_function(&format!("{class} field {}", name.text()), true, &[], |w| {
                        w.expr(v);
                    });
                }
                ClassMemberKind::Method {
                    name,
                    func,
                    has_body: true,
                } => {
                    let id = self.function(func, &format!("{class}:{}", name.text()), !is_static);
                    if !is_static {
                        self.out
                            .class_methods
                            .push((class.clone(), name.text().to_string(), id));
                    }
                }
                ClassMemberKind::Constructor {
                    func,
                    has_body: true,
                } => {
                    self.function(func, &format!("{class} constructor"), true);
                }
                ClassMemberKind::Getter { name, body, .. } => {
                    self.body_function(
                        &format!("{class}.{} getter", name.text()),
                        !is_static,
                        &[],
                        |w| w.block(body),
                    );
                }
                ClassMemberKind::Setter { name, param, body } => {
                    let p = param.name.text().to_string();
                    self.body_function(
                        &format!("{class}.{} setter", name.text()),
                        !is_static,
                        &[p],
                        |w| w.block(body),
                    );
                }
                _ => {}
            }
            self.decorators(m.decorators(), first);
        }
        self.class_ctx.pop();
    }

    /// A function without a `Func` (accessors, field initializers).
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
            reassigned_params: HashSet::new(),
        });
        let saved = self.current;
        let saved_guards = std::mem::take(&mut self.guards);
        let saved_unless = self.unless_function.take();
        self.current = id;
        self.frames.push((self.scopes.len(), id));
        self.scopes.push(FxHashMap::default());
        if method {
            self.declare("self", Local::Param(0));
        }
        for (i, p) in params.iter().enumerate() {
            self.declare(p, Local::Param(i + usize::from(method)));
        }
        f(self);
        self.scopes.pop();
        self.frames.pop();
        self.current = saved;
        self.guards = saved_guards;
        self.unless_function = saved_unless;
        id
    }

    fn stmt_inner(&mut self, s: Stmt<'_>) {
        match s.kind() {
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
                            Some(value) if self.instance_of(value).is_some() => {
                                Local::Instance(self.instance_of(value).expect("instance"))
                            }
                            Some(value) if matches!(value.kind(), ExprKind::Table(_)) => {
                                let path = (
                                    Key::LocalTable(
                                        n.name.span().file,
                                        n.name.span().start,
                                        n.name.text().to_string(),
                                    ),
                                    Vec::new(),
                                );
                                self.constructor_fields(&path, value);
                                Local::Alias(path)
                            }
                            _ => Local::Plain,
                        },
                    };
                    self.declare(n.name.text(), local);
                }
            }
            StmtKind::Assign { targets, values } => {
                let infos: Vec<Info> = values.iter().map(|v| self.expr(v)).collect();
                for (i, t) in targets.iter().enumerate() {
                    if let Some(path) = self.assign_target(t, infos.get(i), t.span(), false)
                        && let Some(value) = values.get(i)
                    {
                        self.constructor_fields(&path, value);
                    }
                }
            }
            StmtKind::Compound { target, value, .. } => {
                self.expr(target);
                self.expr(value);
                self.assign_target(target, None, target.span(), true);
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
                self.scopes.push(FxHashMap::default());
                for s in body.stmts() {
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
                let then_unless = self.not_function_param(cond, true);
                // The other branches run only when `cond` does not hold.
                let else_unless = self.not_function_param(cond, false);
                self.unless_block(then_unless, keys, then_block);
                for (c, b) in elseifs {
                    let saved = self.unless_function;
                    self.unless_function = saved.or(else_unless);
                    let keys = self.test(c);
                    self.unless_function = saved;
                    self.unless_block(else_unless, keys, b);
                }
                if let Some(b) = else_block {
                    self.unless_block(else_unless, Vec::new(), b);
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
                self.scopes.push(FxHashMap::default());
                self.declare(var.name.text(), Local::Plain);
                self.block(body);
                self.scopes.pop();
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                for e in exprs {
                    self.expr(e);
                }
                self.scopes.push(FxHashMap::default());
                for v in vars {
                    self.declare(v.name.text(), Local::Plain);
                }
                self.block(body);
                self.scopes.pop();
            }
            StmtKind::Function { name, func } => {
                let display = func_name_display(name);
                let f = self.function(func, &display, name.method().is_some());
                let root = &name.path().at(0);
                let mut fields: Vec<String> = name
                    .path()
                    .iter()
                    .skip(1)
                    .map(|n| n.text().to_string())
                    .collect();
                if let Some(m) = name.method() {
                    fields.push(m.text().to_string());
                }
                match self.lookup(root.text()).cloned() {
                    Some(Local::Plain) | Some(Local::Function(_)) if fields.is_empty() => {
                        self.declare_existing(root.text(), Local::Function(f));
                    }
                    Some(Local::Ns) => {
                        if let Some((first, rest)) = fields.split_first() {
                            let key = Key::Ns(first.clone());
                            if rest.is_empty() {
                                self.def_key(key.clone(), root.span());
                            } else {
                                self.use_key(key.clone(), root.span());
                                self.def_key(Key::of_path(key.clone(), rest.to_vec()), s.span());
                            }
                            self.out.function_defs.push(((key, rest.to_vec()), f));
                        }
                    }
                    Some(Local::Alias((key, prefix))) => {
                        self.use_key(key.clone(), root.span());
                        let mut path = prefix.clone();
                        path.extend(fields);
                        if !path.is_empty() {
                            self.def_key(Key::of_path(key.clone(), path.clone()), s.span());
                        }
                        self.out.function_defs.push(((key, path), f));
                    }
                    Some(_) => {
                        if let Some(last) = fields.last() {
                            let current = self.current;
                            self.out
                                .untracked_field_writes
                                .push((last.clone(), current, s.span()));
                        }
                    }
                    None => {
                        let key = Key::Global(root.text().to_string());
                        if fields.is_empty() {
                            self.def_key(key.clone(), root.span());
                        } else {
                            self.use_key(key.clone(), root.span());
                            self.def_key(Key::of_path(key.clone(), fields.clone()), s.span());
                        }
                        self.out.function_defs.push(((key, fields), f));
                    }
                }
            }
            StmtKind::LocalFunction { name, func } => {
                let id = self.out.functions.len();
                self.declare(name.text(), Local::Function(id));
                self.function(func, name.text(), false);
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
                        self.expr(*d);
                    }
                    self.declare(f.name.text(), Local::Plain);
                }
            }
            StmtKind::Const { name, value, .. } => {
                let info = value.as_ref().map(|v| self.expr(*v));
                let instance = value.and_then(|v| self.instance_of(v));
                let local = match (info, instance) {
                    (Some(Info::Function(f)), _) => Local::Function(f),
                    (_, Some(class)) => Local::Instance(class),
                    _ => Local::Plain,
                };
                self.declare(name.text(), local);
            }
            StmtKind::Import(import) => {
                let stdlib = import.from().value().starts_with(b"wowlua:");
                match &import.clause() {
                    ImportClause::Names(specs) => {
                        for spec in specs {
                            let local = spec.alias.as_ref().unwrap_or(&spec.name);
                            self.declare(local.text(), Local::Import(local.text().to_string()));
                            if stdlib {
                                self.stdlib_imports.insert(local.text().to_string());
                            }
                        }
                    }
                    ImportClause::Namespace(n) => {
                        self.declare(n.text(), Local::Import(n.text().to_string()));
                        if stdlib {
                            self.stdlib_imports.insert(n.text().to_string());
                        }
                    }
                    ImportClause::SideEffect => {}
                }
            }
            StmtKind::Export(inner) => {
                match inner.kind() {
                    StmtKind::Function { name, func } => {
                        let n = name.path().at(0).text().to_string();
                        let f = self.function(func, &n, false);
                        self.declare(&n, Local::Function(f));
                        self.out.exported_functions.insert(n, f);
                        return;
                    }
                    StmtKind::Local { names, values } => {
                        self.stmt(inner);
                        for (i, n) in names.iter().enumerate() {
                            if values
                                .get(i)
                                .is_some_and(|v| matches!(v.kind(), ExprKind::Function(_)))
                                && let Some(Local::Function(f)) =
                                    self.lookup(n.name.text()).cloned()
                            {
                                self.out
                                    .exported_functions
                                    .insert(n.name.text().to_string(), f);
                            }
                        }
                        return;
                    }
                    _ => {}
                }
                self.stmt(inner);
            }
            StmtKind::Class(c) => self.class_decl(c),
            StmtKind::Interface(i) => self.interface_decl(i),
            StmtKind::Break
            | StmtKind::Continue
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
        t: Expr<'_>,
        value: Option<&Info>,
        span: Span,
        compound: bool,
    ) -> Option<Path> {
        let function = match value {
            Some(Info::Function(f)) => Some(*f),
            _ => None,
        };
        let table = Self::table_of(value);
        let (key, fields) = match t.kind() {
            ExprKind::Name(n) => match self.lookup(n.text()) {
                Some(_) => {
                    if let Some((owner, p)) = self.param_owner(n.text()) {
                        self.out.functions[owner].reassigned_params.insert(p);
                    }
                    if let Some(f) = function {
                        self.declare_existing(n.text(), Local::Function(f));
                    }
                    return None;
                }
                None => (Key::Global(n.text().to_string()), Vec::new()),
            },
            ExprKind::Field { obj, .. } | ExprKind::Index { obj, .. } => {
                let obj_info = self.expr(obj);
                let field = match t.kind() {
                    ExprKind::Field { name, .. } => Some(name.text().to_string()),
                    ExprKind::Index { key, .. } => {
                        self.expr(key);
                        match key.kind() {
                            ExprKind::String(s) => {
                                Some(String::from_utf8_lossy(&s.value()).into_owned())
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
    fn constructor_fields(&mut self, path: &Path, value: Expr<'_>) {
        let ExprKind::Table(fields) = value.kind() else {
            return;
        };
        for field in fields {
            if let TableField::Named { name, value } = field {
                let mut inner = path.1.clone();
                inner.push(name.text().to_string());
                let inner = (path.0.clone(), inner);
                self.def_key(Key::of_path(inner.0.clone(), inner.1.clone()), field.span());
                self.constructor_fields(&inner, value);
            }
        }
    }

    /// Walks a function body as a new function; returns its index.
    fn function(&mut self, f: Func<'_>, name: &str, method: bool) -> usize {
        let id = self.out.functions.len();
        self.out.functions.push(Function {
            name: name.to_string(),
            calls: Vec::new(),
            called_params: HashSet::new(),
            reassigned_params: HashSet::new(),
        });
        let saved = self.current;
        let saved_guards = std::mem::take(&mut self.guards);
        let saved_unless = self.unless_function.take();
        self.current = id;
        self.frames.push((self.scopes.len(), id));
        self.scopes.push(FxHashMap::default());
        if method {
            self.declare("self", Local::Param(0));
        }
        for (i, p) in f.params().iter().enumerate() {
            self.declare(p.name().text(), Local::Param(i + usize::from(method)));
            if let Some(d) = p.default() {
                self.expr(d);
            }
        }
        for s in f.body().stmts() {
            self.stmt(s);
        }
        self.scopes.pop();
        self.frames.pop();
        self.current = saved;
        self.guards = saved_guards;
        self.unless_function = saved_unless;
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
            method: self.next_method.take(),
            unless_function: self.unless_function,
        });
    }

    fn args(&mut self, args: Args<'_>) -> Vec<Info> {
        args.exprs().into_iter().map(|a| self.expr(a)).collect()
    }

    fn expr(&mut self, e: Expr<'_>) -> Info {
        // The probe of an optional link guards the rest of its chain: its
        // keys stay guarded until the chain's outermost link is walked.
        let chain = matches!(
            e.kind(),
            ExprKind::Field { .. }
                | ExprKind::OptField { .. }
                | ExprKind::Index { .. }
                | ExprKind::OptIndex { .. }
                | ExprKind::Call { .. }
                | ExprKind::OptCall { .. }
                | ExprKind::Method { .. }
                | ExprKind::OptMethod { .. }
        );
        let inner = std::mem::replace(&mut self.link, false);
        let n = self.guards.len();
        let info = grow(|| self.expr_inner(e));
        if chain && !inner {
            self.guards.truncate(n);
        }
        info
    }

    /// Walks the object of a chain link.
    fn link(&mut self, e: Expr<'_>) -> Info {
        self.link = true;
        self.expr(e)
    }

    /// Reads `a.b.c` as the expression of those names would, at `span`
    /// (a class's `extends`, which the tree holds as a type).
    fn path_read(&mut self, path: &[String], span: Span) {
        self.link = false;
        let n = self.guards.len();
        let mut info = self.name(&path[0], span);
        for p in &path[1..] {
            info = self.field(info, p.clone(), span);
        }
        if path.len() > 1 {
            self.guards.truncate(n);
        }
    }

    /// A name read as an expression.
    fn name(&mut self, text: &str, span: Span) -> Info {
        match self.lookup(text).cloned() {
            Some(Local::Function(f)) => Info::Function(f),
            Some(Local::Ns) => Info::Ns,
            Some(Local::Alias(p)) => Info::Path(p),
            Some(Local::Import(b)) => {
                let current = self.current;
                self.out.import_uses.push((b.clone(), current, span));
                Info::Import(b, Vec::new())
            }
            Some(Local::Param(i)) => Info::Param(i),
            Some(Local::Instance(c)) => Info::Instance(c),
            Some(Local::Plain) => Info::Other,
            None if text == "_G" => Info::GlobalTable,
            None => {
                let key = Key::Global(text.to_string());
                self.use_key(key.clone(), span);
                Info::Path((key, Vec::new()))
            }
        }
    }

    fn expr_inner(&mut self, e: Expr<'_>) -> Info {
        match e.kind() {
            ExprKind::Name(n) => self.name(n.text(), n.span()),
            ExprKind::Vararg => {
                if self.current == 0 {
                    Info::VarargNs
                } else {
                    Info::Other
                }
            }
            ExprKind::Field { obj, name } => {
                let o = self.link(obj);
                self.field(o, name.text().to_string(), name.span())
            }
            // `X?.f` probes X: it may be nil.
            ExprKind::OptField { obj, name } => {
                let o = self.probed(obj);
                self.field(o, name.text().to_string(), name.span())
            }
            ExprKind::Index { obj, key } | ExprKind::OptIndex { obj, key } => {
                let o = if matches!(e.kind(), ExprKind::OptIndex { .. }) {
                    self.probed(obj)
                } else {
                    self.link(obj)
                };
                self.expr(key);
                match key.kind() {
                    ExprKind::String(s) => self.field(
                        o,
                        String::from_utf8_lossy(&s.value()).into_owned(),
                        key.span(),
                    ),
                    _ => Info::Other,
                }
            }
            ExprKind::Call { callee, args } | ExprKind::OptCall { callee, args } => {
                let c = if matches!(e.kind(), ExprKind::OptCall { .. }) {
                    self.probed(callee)
                } else {
                    self.link(callee)
                };
                // `type(X)` tests X's existence.
                let arg_infos = match (&c, args.exprs().exactly()) {
                    (Info::Path((Key::Global(g), fields)), Some([only]))
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
                    && matches!(args.exprs().first().map(|a| a.kind()), Some(ExprKind::Number(n)) if n.value() == Some(2.0))
                    && matches!(arg_infos.get(1), Some(Info::VarargNs))
                {
                    return Info::Ns;
                }
                if let Info::Path((Key::Global(g), fields)) = &c
                    && g == "setfenv"
                    && fields.is_empty()
                {
                    self.out.dynamic_write.get_or_insert(e.span());
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
                self.record_call(Self::call_target(&c), &arg_infos, 0, e.span(), deferred);
                Info::Other
            }
            ExprKind::Method { obj, name, args } | ExprKind::OptMethod { obj, name, args } => {
                let o = if matches!(e.kind(), ExprKind::OptMethod { .. }) {
                    self.probed(obj)
                } else {
                    self.link(obj)
                };
                match &o {
                    Info::Path((k, fields)) => {
                        let mut fields = fields.clone();
                        fields.push(name.text().to_string());
                        self.use_key(Key::of_path(k.clone(), fields), name.span());
                    }
                    Info::Ns => self.use_key(Key::Ns(name.text().to_string()), name.span()),
                    _ => {}
                }
                let arg_infos = self.args(args);
                let target = match &o {
                    Info::Path((k, fields)) => {
                        let mut fields = fields.clone();
                        fields.push(name.text().to_string());
                        CallTarget::Path((k.clone(), fields))
                    }
                    Info::Ns => CallTarget::Path((Key::Ns(name.text().to_string()), Vec::new())),
                    Info::Import(b, fields) => {
                        let mut fields = fields.clone();
                        fields.push(name.text().to_string());
                        CallTarget::Import {
                            binding: b.clone(),
                            fields,
                        }
                    }
                    // `self` in a class's method (decision T269)
                    Info::Param(0)
                        if matches!(obj.kind(), ExprKind::Name(n) if n.text() == "self")
                            && !self.class_ctx.is_empty() =>
                    {
                        CallTarget::ClassMethod {
                            class: self.class_ctx.last().expect("class").clone(),
                            method: name.text().to_string(),
                        }
                    }
                    Info::Instance(class) => CallTarget::ClassMethod {
                        class: class.clone(),
                        method: name.text().to_string(),
                    },
                    _ => CallTarget::Unknown,
                };
                let deferred = self.markers.deferred_methods.contains(name.text());
                self.next_method = Some(name.text().to_string());
                self.record_call(target, &arg_infos, 1, e.span(), deferred);
                Info::Other
            }
            ExprKind::Function(f) => {
                Info::Function(self.function(f, &format!("function at {}", e.span().start), false))
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
            } if matches!(lhs.kind(), ExprKind::Nil) || matches!(rhs.kind(), ExprKind::Nil) => {
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
fn type_test<'e>(lhs: Expr<'e>, rhs: Expr<'e>) -> Option<(Expr<'e>, bool)> {
    let (call, string) = match (lhs.kind(), rhs.kind()) {
        (ExprKind::Call { .. }, ExprKind::String(s)) => (lhs, s),
        (ExprKind::String(s), ExprKind::Call { .. }) => (rhs, s),
        _ => return None,
    };
    let ExprKind::Call { callee, args } = call.kind() else {
        return None;
    };
    let ExprKind::Name(n) = callee.kind() else {
        return None;
    };
    match args.exprs().exactly() {
        Some([only]) if n.text() == "type" => Some((only, *string.value() == *b"nil")),
        _ => None,
    }
}

/// The `type` callee of a type test.
fn type_callee<'e>(lhs: Expr<'e>, rhs: Expr<'e>) -> Expr<'e> {
    let call = if matches!(lhs.kind(), ExprKind::Call { .. }) {
        lhs
    } else {
        rhs
    };
    match call.kind() {
        ExprKind::Call { callee, .. } => callee,
        _ => call,
    }
}

fn func_name_display(name: FuncName<'_>) -> String {
    let parts: Vec<&str> = name.path().iter().map(|n| n.text()).collect();
    let path = parts.join(".");
    match &name.method() {
        Some(m) => format!("{path}:{}", m.text()),
        None => path,
    }
}
