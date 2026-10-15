//! Lowering: wowlua syntax tree → Lua 5.1 IR (spec compiler §13.3.1, stage
//! 14). Implements the syntax-sugar lowerings of spec chapter 1 with the
//! "preceding statements" mechanism of codegen §14.4. Without type
//! information (milestone M4), every file uses the basic-mode forms.
//! Classes and enums are in [`class`], decorators in [`decorate`].

mod checks;
mod class;
mod debug;
mod decorate;
mod instrument;
mod reflect;
mod scope;

pub(crate) use instrument::FnMarks;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use wowlua_diagnostics::Diagnostic;
use wowlua_luair::ir::{self, Pos};
use wowlua_runtime::Slice;
use wowlua_sema::classes::{ClassInfo, EnumInfo};
use wowlua_sema::constant::{self, Const, quote};
use wowlua_sema::meta::{Lookup, Meta, MetaKind};
use wowlua_span::Span;
use wowlua_syntax::Parse;
use wowlua_syntax::ast::*;
use wowlua_syntax::token::TriviaKind;

pub use wowlua_sema::constant::{format_g14, format_number};

/// What the lowering of one file needs to know about the package.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// The module ID when the file is a module (it imports or exports).
    pub module: Option<String>,
    /// Resolved imports, by the start offset of the import statement.
    pub imports: HashMap<u32, ImportContext>,
    /// `wowlua:meta` values of the build configuration and flavor. The
    /// lowering reads them only through `Lowerer::meta`, which records the
    /// read ([`Lowered::reads_meta`]): a file lowered without reading them
    /// lowers the same with any other `meta`, and the build reuses it.
    pub meta: Option<Arc<Meta>>,
    /// The source path (`src/core/cache.wowlua`), for decorator contexts.
    pub file: String,
    /// The file's top-level classes and enums (from the package analysis).
    pub classes: HashMap<String, Arc<ClassInfo>>,
    pub enums: HashMap<String, Arc<EnumInfo>>,
    /// A debug build (enum guards, abstract stubs, `@debugOnly` kept).
    pub debug: bool,
    /// A compiler-known module (the standard library may use `@intrinsic`).
    pub compiler_known: bool,
    /// A file of a wowlua library dependency: compiled like the package's
    /// own files, but never instrumented or monitored (decision T97).
    pub dependency: bool,
    /// The profile's debugging settings (spec chapter 7).
    pub tools: Tools,
    /// The static types of `typeName(expr)` arguments, by span.
    pub type_names: HashMap<(u32, u32), String>,
    /// Runtime checks of annotated parameters, `...: T` and `::`, by span
    /// (from the type checker, spec §7.5).
    pub value_checks: HashMap<(u32, u32), wowlua_sema::check::runtime::ValueCheck>,
}

/// The debugging settings of the build profile (spec chapter 7).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tools {
    /// `assertions`: `wowlua:debug` assertions are kept.
    pub assertions: bool,
    /// `dev-console`: the dev console and window (watches).
    pub dev_console: bool,
    /// `instrument` is not `"none"`: profiler statistics exist.
    pub profiling: bool,
    /// `log-level` as a rank: 0 `trace` … 4 `error`, 5 `off`.
    pub log_level: u8,
    /// `instrument`: 0 `none`, 1 `decorated`, 2 `exports`, 3 `all`.
    pub instrument: u8,
    /// `runtime-checks`: 0 `none`, 1 `exports`, 2 `all`.
    pub runtime_checks: u8,
    /// `reflection`: 0 `none`, 1 `decorated`, 2 `all`.
    pub reflection: u8,
    /// The package imports `wowlua:reflect` (enums register their order).
    pub reflect_used: bool,
    /// `monitor`: handler installations go through the monitor.
    pub monitor: bool,
}

/// A lowered file.
#[derive(Debug)]
pub struct Lowered {
    pub chunk: ir::Chunk,
    /// `wowlua:meta` paths used as runtime values (spec §6.2).
    pub meta_used: BTreeSet<(MetaKind, Vec<String>)>,
    /// The file uses the runtime (`__wl`).
    pub uses_runtime: bool,
    /// Runtime slices the file needs.
    pub slices: BTreeSet<Slice>,
    pub warnings: Vec<Diagnostic>,
    /// The lowering read `wowlua:meta` ([`Context::meta`]).
    pub reads_meta: bool,
}

#[derive(Clone, Debug)]
pub enum ImportContext {
    /// A wowlua module, or a plain Lua file registering one.
    Module {
        id: String,
        /// The module loads after this file (a run-time cycle): bindings
        /// are read through the module table (spec §5.5).
        lazy: bool,
        exports: HashMap<String, wowlua_sema::Export>,
        /// `@deprecated` / `@debugOnly` exports.
        attrs: HashMap<String, wowlua_sema::ExportAttrs>,
        /// Exports bound to compiler behavior: name → `@intrinsic` id.
        intrinsics: HashMap<String, String>,
    },
    /// `wowlua:meta`.
    Meta,
    /// XML or plain Lua: load order only.
    LoadOnly,
}

impl Context {
    /// The same context apart from `meta`: lowering with either gives the
    /// same result when it does not read `meta` ([`Lowered::reads_meta`]).
    /// Constants compare bit for bit ([`Const::same`]).
    pub fn same_except_meta(&self, other: &Context) -> bool {
        // Destructured so that a new field cannot be left out.
        let Context {
            module,
            imports,
            meta: _,
            file,
            classes,
            enums,
            debug,
            compiler_known,
            dependency,
            tools,
            type_names,
            value_checks,
        } = self;
        *module == other.module
            && *file == other.file
            && *debug == other.debug
            && *compiler_known == other.compiler_known
            && *dependency == other.dependency
            && *tools == other.tools
            && *classes == other.classes
            && same_map(enums, &other.enums, |a, b| same_enum(a, b))
            && same_map(imports, &other.imports, same_import)
            && *type_names == other.type_names
            && *value_checks == other.value_checks
    }
}

fn same_map<K: Eq + std::hash::Hash, V>(
    a: &HashMap<K, V>,
    b: &HashMap<K, V>,
    same: impl Fn(&V, &V) -> bool,
) -> bool {
    a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| same(v, w)))
}

fn same_enum(a: &EnumInfo, b: &EnumInfo) -> bool {
    // Destructured so that a new field cannot be left out.
    let EnumInfo { name, members } = a;
    *name == b.name
        && members.len() == b.members.len()
        && members
            .iter()
            .zip(&b.members)
            .all(|((n, c), (m, d))| n == m && c.same(d))
}

fn same_export(a: &wowlua_sema::Export, b: &wowlua_sema::Export) -> bool {
    use wowlua_sema::Export;
    match (a, b) {
        (Export::Const(Some(x)), Export::Const(Some(y))) => x.same(y),
        (Export::Enum(x), Export::Enum(y)) => same_enum(x, y),
        _ => a == b,
    }
}

fn same_import(a: &ImportContext, b: &ImportContext) -> bool {
    match (a, b) {
        (
            ImportContext::Module {
                id,
                lazy,
                exports,
                attrs,
                intrinsics,
            },
            ImportContext::Module {
                id: id2,
                lazy: lazy2,
                exports: exports2,
                attrs: attrs2,
                intrinsics: intrinsics2,
            },
        ) => {
            id == id2
                && lazy == lazy2
                && attrs == attrs2
                && intrinsics == intrinsics2
                && same_map(exports, exports2, same_export)
        }
        (ImportContext::Meta, ImportContext::Meta)
        | (ImportContext::LoadOnly, ImportContext::LoadOnly) => true,
        _ => false,
    }
}

/// Lowers a parsed file without errors, as a script (no package context).
pub fn lower(parse: &Parse, src: &[u8]) -> Result<ir::Chunk, Vec<Diagnostic>> {
    lower_with(parse, src, &Context::default())
}

/// Lowers a parsed file without errors. `src` is the file's bytes.
pub fn lower_with(parse: &Parse, src: &[u8], ctx: &Context) -> Result<ir::Chunk, Vec<Diagnostic>> {
    lower_module(parse, src, ctx).map(|l| l.chunk)
}

/// Lowers a parsed file without errors, with what the build needs to know
/// about it.
pub fn lower_module(parse: &Parse, src: &[u8], ctx: &Context) -> Result<Lowered, Vec<Diagnostic>> {
    let mut lowerer = Lowerer {
        parse,
        src,
        ctx,
        scopes: scope::Scopes::new(),
        prelude: Vec::new(),
        temps: 0,
        aliases: BTreeMap::new(),
        errors: Vec::new(),
        loops: Vec::new(),
        func_depth: 0,
        pending: HashSet::new(),
        uses_runtime: false,
        slices: BTreeSet::new(),
        warnings: Vec::new(),
        classes: Vec::new(),
        widget_ctor: None,
        has_exports: false,
        meta_used: BTreeSet::new(),
        fn_names: Vec::new(),
        next_fn_name: None,
        module_tables: HashMap::new(),
        next_fn_marks: FnMarks::default(),
        class_marks: None,
        fn_marks: HashMap::new(),
        meta_read: std::cell::Cell::new(false),
        line_hint: std::cell::Cell::new(0),
        token_hint: std::cell::Cell::new(0),
    };
    // Exported functions, classes and enums are hoisted (spec §5.1,
    // decisions L24 and T76).
    let hoisted = lowerer.hoist();
    let mut stmts = lowerer.stmts(&parse.chunk.block.stmts);
    if !lowerer.errors.is_empty() {
        return Err(lowerer.errors);
    }
    // Profiling and tracing wrappers (spec §7.3, §7.4).
    lowerer.instrument(&mut stmts);
    // Prologue (spec codegen §14.2), on line 1: the runtime, the export
    // table, hoisted declarations and aliases of shadowed globals.
    let line1 = Pos::new(1, 0);
    let mut prologue: Vec<ir::Stmt> = Vec::new();
    if ctx.module.is_some() {
        lowerer.uses_runtime = true;
    }
    if lowerer.uses_runtime {
        let select = name_expr(ident("select", line1));
        let addon_table = call(
            select,
            vec![
                lit(ir::ExprKind::Number("2".into()), line1),
                lit(ir::ExprKind::Vararg, line1),
            ],
            line1,
        );
        prologue.push(local(
            generated("__wl", line1),
            field(addon_table, "__wowlua", line1),
            line1,
        ));
    }
    if let (Some(id), true) = (&ctx.module, lowerer.has_exports) {
        let module = field(name_expr(generated("__wl", line1)), "module", line1);
        let value = call(
            module,
            vec![lit(ir::ExprKind::String(quote(id.as_bytes())), line1)],
            line1,
        );
        prologue.push(local(generated("__wl_exports", line1), value, line1));
    }
    if !hoisted.is_empty() {
        prologue.push(stmt(
            ir::StmtKind::Local {
                names: hoisted.iter().map(|n| ident(n, line1)).collect(),
                values: Vec::new(),
            },
            line1,
        ));
    }
    let aliases: Vec<ir::Stmt> = lowerer
        .aliases
        .iter()
        .map(|(alias, (global, member))| {
            let mut value = ir::Expr::new(ir::ExprKind::Name(ident(global, line1)), line1);
            if let Some(m) = member {
                value = field(value, m, line1);
            }
            ir::Stmt {
                pos: line1,
                kind: ir::StmtKind::Local {
                    names: vec![generated(alias, line1)],
                    values: vec![value],
                },
            }
        })
        .collect();
    prologue.extend(aliases);
    stmts.splice(0..0, prologue);
    let mut chunk = ir::Chunk {
        block: ir::Block {
            stmts,
            end: Pos::NONE,
        },
        comments: lowerer.comments(),
    };
    wowlua_luair::hygiene::apply(&mut chunk);
    let reads_meta = lowerer.meta_read.get();
    debug_assert!(lowerer.meta_used.is_empty() || reads_meta);
    Ok(Lowered {
        chunk,
        meta_used: lowerer.meta_used,
        uses_runtime: lowerer.uses_runtime,
        slices: lowerer.slices,
        warnings: lowerer.warnings,
        reads_meta,
    })
}

/// The display name of a function declaration (`Tracker.record`, `M:load`).
fn func_name(name: &FuncName) -> String {
    let mut out = name
        .path
        .iter()
        .map(|n| n.text.as_ref())
        .collect::<Vec<_>>()
        .join(".");
    if let Some(m) = &name.method {
        out.push(':');
        out.push_str(&m.text);
    }
    out
}

fn grow<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

// ------------------------------------------------------------- helpers --

/// `slice.partition_point(pred)`, searched outward from `hint` (any value;
/// past the end counts as the end). `pred` must hold on a prefix of `slice`
/// and nowhere after it, so the point is unique and the hint only decides how
/// fast it is found: a few probes next to the hint instead of a binary search
/// over a whole large file.
fn partition_point_near<T>(slice: &[T], hint: usize, pred: impl Fn(&T) -> bool) -> usize {
    let hint = hint.min(slice.len());
    // The point lies in `lo..=hi`.
    let (mut lo, mut hi) = (0, slice.len());
    let mut step = 1;
    if hint == 0 || pred(&slice[hint - 1]) {
        // At `hint` or after it: probe `hint`, `hint + 1`, `hint + 3`, …
        lo = hint;
        loop {
            let probe = hint + step - 1;
            if probe >= hi {
                break;
            }
            if !pred(&slice[probe]) {
                hi = probe;
                break;
            }
            lo = probe + 1;
            step *= 2;
        }
    } else {
        // Before `hint - 1`: probe `hint - 2`, `hint - 3`, `hint - 5`, …
        hi = hint - 1;
        while step < hint {
            let probe = hint - 1 - step;
            if pred(&slice[probe]) {
                lo = probe + 1;
                break;
            }
            hi = probe;
            step *= 2;
        }
    }
    lo + slice[lo..hi].partition_point(&pred)
}

fn ident(name: &str, pos: Pos) -> ir::Ident {
    ir::Ident::new(name, pos)
}

fn generated(name: &str, pos: Pos) -> ir::Ident {
    ir::Ident {
        name: name.to_string(),
        generated: true,
        pos,
    }
}

fn name_expr(id: ir::Ident) -> ir::Expr {
    let pos = id.pos;
    ir::Expr::new(ir::ExprKind::Name(id), pos)
}

fn field(obj: ir::Expr, name: &str, pos: Pos) -> ir::Expr {
    ir::Expr::new(
        ir::ExprKind::Field {
            obj: Box::new(obj),
            name: ident(name, Pos::NONE),
        },
        pos,
    )
}

fn call(callee: ir::Expr, args: Vec<ir::Expr>, pos: Pos) -> ir::Expr {
    ir::Expr::new(
        ir::ExprKind::Call {
            callee: Box::new(callee),
            args: ir::Args::Parens {
                args,
                close: Pos::NONE,
            },
        },
        pos,
    )
}

fn binary(op: ir::BinOp, lhs: ir::Expr, rhs: ir::Expr, pos: Pos) -> ir::Expr {
    ir::Expr::new(
        ir::ExprKind::Binary {
            op,
            op_pos: Pos::NONE,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        pos,
    )
}

fn lit(kind: ir::ExprKind, pos: Pos) -> ir::Expr {
    ir::Expr::new(kind, pos)
}

fn stmt(kind: ir::StmtKind, pos: Pos) -> ir::Stmt {
    ir::Stmt { pos, kind }
}

fn local(name: ir::Ident, value: ir::Expr, pos: Pos) -> ir::Stmt {
    stmt(
        ir::StmtKind::Local {
            names: vec![name],
            values: vec![value],
        },
        pos,
    )
}

fn assign(target: ir::Expr, value: ir::Expr, pos: Pos) -> ir::Stmt {
    stmt(
        ir::StmtKind::Assign {
            targets: vec![target],
            values: vec![value],
        },
        pos,
    )
}

fn if_stmt(cond: ir::Expr, body: Vec<ir::Stmt>, pos: Pos) -> ir::Stmt {
    stmt(
        ir::StmtKind::If {
            clauses: vec![ir::IfClause {
                cond,
                then_pos: Pos::NONE,
                body: ir::Block {
                    stmts: body,
                    end: Pos::NONE,
                },
            }],
            else_block: None,
        },
        pos,
    )
}

fn do_block(stmts: Vec<ir::Stmt>, pos: Pos) -> ir::Stmt {
    stmt(
        ir::StmtKind::Do(ir::Block {
            stmts,
            end: Pos::NONE,
        }),
        pos,
    )
}

/// The expression may call a function or read `...` when evaluated.
fn has_side_effects(e: &ir::Expr) -> bool {
    use ir::ExprKind as K;
    match &e.kind {
        K::Call { .. } | K::Method { .. } | K::Vararg => true,
        K::Nil | K::True | K::False | K::Number(_) | K::String(_) | K::Function(_) | K::Name(_) => {
            false
        }
        K::Table { fields, .. } => fields.iter().any(|f| match f {
            ir::Field::Keyed { key, value, .. } => has_side_effects(key) || has_side_effects(value),
            ir::Field::Named { value, .. } | ir::Field::Positional(value) => {
                has_side_effects(value)
            }
        }),
        K::Index { obj, key } => has_side_effects(obj) || has_side_effects(key),
        K::Field { obj, .. } => has_side_effects(obj),
        K::Paren { inner, .. } => has_side_effects(inner),
        K::Binary { lhs, rhs, .. } => has_side_effects(lhs) || has_side_effects(rhs),
        K::Unary { operand, .. } => has_side_effects(operand),
    }
}

// --------------------------------------------------------------- scopes --

#[derive(Clone, Debug)]
enum Binding {
    Local,
    /// An imported binding (immutable, spec §5.2).
    Import,
    /// An import read through its module table (`__wl_m1.Name`).
    LazyImport {
        table: String,
        name: String,
    },
    /// An exported binding (immutable, spec §5.1).
    Export,
    /// A `wowlua:meta` export.
    Meta(MetaKind),
    /// `import * as M from "wowlua:meta"`.
    MetaNamespace,
    /// A declaration removed by `@if` (E1020 when used).
    Removed(Span),
    /// A `const` folded to a value: references are replaced by it.
    Const(Const),
    /// A `const` that is a normal local at run time.
    ConstLocal,
    /// A top-level class of this file (hoisted; `exported` is immutable).
    Class {
        info: Arc<ClassInfo>,
        exported: bool,
    },
    /// An imported class.
    ImportedClass(Arc<ClassInfo>),
    /// A top-level enum of this file; `table` when it exists at run time.
    Enum {
        info: Arc<EnumInfo>,
        table: bool,
    },
    ImportedEnum(Arc<EnumInfo>),
    /// A `@debugOnly` function removed from this build: call statements
    /// are dropped (spec §4.3).
    DebugOnlyRemoved,
    /// An imported export with compiler behavior (`@intrinsic(id)`); a
    /// normal import as a value.
    Intrinsic(String),
    /// A namespace import of a module with intrinsics (`import * as debug
    /// from "wowlua:debug"`): member name → `@intrinsic` id.
    IntrinsicNamespace(Arc<HashMap<String, String>>),
}

/// How the body of the loop being lowered handles `continue` (spec §1.3.6).
#[derive(Clone)]
struct LoopInfo {
    /// The body is wrapped in `repeat … until true`.
    wrapped: bool,
    /// The flag that carries a `break` out of the wrapper.
    break_flag: Option<String>,
}

struct Lowerer<'a> {
    parse: &'a Parse,
    src: &'a [u8],
    ctx: &'a Context,
    /// Nesting of function bodies (0 = top level of the file).
    func_depth: u32,
    /// Hoisted declarations not defined yet (E1037).
    pending: HashSet<String>,
    uses_runtime: bool,
    slices: BTreeSet<Slice>,
    warnings: Vec<Diagnostic>,
    /// The classes whose bodies are being lowered (innermost last).
    classes: Vec<class::ClassCtx>,
    /// The function depth of a widget constructor body (`return` returns
    /// the frame).
    widget_ctor: Option<u32>,
    has_exports: bool,
    meta_used: BTreeSet<(MetaKind, Vec<String>)>,
    scopes: scope::Scopes,
    /// Statements that must run before the statement being lowered.
    prelude: Vec<ir::Stmt>,
    temps: u32,
    /// Hygienic aliases of shadowed globals: alias → (global, member).
    aliases: BTreeMap<String, (String, Option<String>)>,
    errors: Vec<Diagnostic>,
    loops: Vec<LoopInfo>,
    /// Names of the functions being lowered (innermost last; `None` for
    /// anonymous ones), for `location()` and the profiler.
    fn_names: Vec<Option<String>>,
    /// The name of the next function `func` lowers.
    next_fn_name: Option<String>,
    /// Imported modules → the local holding their export table, and
    /// whether that local is a generated name (else the user's binding).
    module_tables: HashMap<String, (String, bool)>,
    /// Profiling and tracing marks of the next function `func` lowers.
    next_fn_marks: FnMarks,
    /// Marks of the class being lowered (`@profile` on a class).
    class_marks: Option<FnMarks>,
    /// Marks of lowered functions, by the position of their `(`.
    fn_marks: HashMap<Pos, FnMarks>,
    /// `ctx.meta` was read ([`Lowerer::meta`]).
    meta_read: std::cell::Cell<bool>,
    /// Where the last position lookups landed in `line_starts` and `tokens`:
    /// lowering walks a file mostly forward, so the next lookup starts its
    /// search there ([`partition_point_near`]).
    line_hint: std::cell::Cell<usize>,
    token_hint: std::cell::Cell<usize>,
}

const LICENSE_WORDS: &[&str] = &["license", "copyright", "spdx-license-identifier"];

impl<'a> Lowerer<'a> {
    /// `wowlua:meta`, recording that it was read (also when there is none).
    /// Every read of `ctx.meta` goes through here: the build reuses a file's
    /// lowering in the other flavors only when it never read it.
    fn meta(&self) -> Option<&Arc<Meta>> {
        self.meta_read.set(true);
        self.ctx.meta.as_ref()
    }

    // ---------------------------------------------------------- positions --

    /// The Lua line (1-based) containing byte `offset`, the same line
    /// [`Lexed::line_of`](wowlua_syntax::Lexed::line_of) finds: line starts
    /// are strictly increasing, so it is the number of starts at or before
    /// `offset`.
    fn line_of(&self, offset: u32) -> u32 {
        let starts = &self.parse.lexed.line_starts;
        let line = partition_point_near(starts, self.line_hint.get(), |&s| s <= offset);
        self.line_hint.set(line);
        line as u32
    }

    fn pos(&self, offset: u32) -> Pos {
        let line = self.line_of(offset);
        let start = self.parse.lexed.line_starts[line as usize - 1];
        Pos::new(line, offset - start)
    }

    /// The column (0-based, in bytes) of byte `offset`.
    fn column_of(&self, offset: u32) -> u32 {
        self.pos(offset).col
    }

    fn span_pos(&self, span: Span) -> Pos {
        self.pos(span.start)
    }

    fn next_token_pos(&self, offset: u32) -> Pos {
        let tokens = &self.parse.lexed.tokens;
        let i = partition_point_near(tokens, self.token_hint.get(), |t| t.start < offset);
        self.token_hint.set(i);
        match tokens.get(i) {
            Some(t) => Pos::new(t.line, self.column_of(t.start)),
            None => Pos::NONE,
        }
    }

    fn close_pos(&self, span: Span) -> Pos {
        self.pos(span.end.saturating_sub(1))
    }

    fn comments(&self) -> Vec<ir::Comment> {
        let lexed = &self.parse.lexed;
        let first = lexed.tokens.first().map(|t| t.trivia_end).unwrap_or(0) as usize;
        let mut out = Vec::new();
        let mut leading_license = false;
        for (i, trivia) in lexed.trivia.iter().enumerate() {
            let block = match trivia.kind {
                TriviaKind::LineComment => false,
                TriviaKind::BlockComment => true,
                _ => continue,
            };
            let text = self.src[trivia.start as usize..trivia.end as usize].to_vec();
            if i < first {
                let lower = String::from_utf8_lossy(&text).to_ascii_lowercase();
                if LICENSE_WORDS.iter().any(|w| lower.contains(w)) {
                    leading_license = true;
                }
            }
            out.push((
                i < first,
                ir::Comment {
                    pos: Pos::new(trivia.line, self.column_of(trivia.start)),
                    license: text.starts_with(b"--[[!"),
                    text,
                    block,
                },
            ));
        }
        out.into_iter()
            .map(|(leading, mut c)| {
                c.license |= leading && leading_license;
                c
            })
            .collect()
    }

    // ------------------------------------------------------------- scopes --

    fn push_scope(&mut self) {
        self.scopes.push();
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, binding: Binding) {
        self.scopes.declare(name, binding);
    }

    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.lookup(name)
    }

    /// The binding a name expression refers to, `None` for a plain local;
    /// uses of `@deprecated` bindings are reported (W3030).
    fn name_binding(&mut self, n: &Name) -> Option<Binding> {
        if let Some(message) = self.deprecated_of(&n.text) {
            let detail = message.map(|m| format!(": {m}")).unwrap_or_default();
            self.warnings.push(
                Diagnostic::warning("W3030", format!("`{}` is deprecated{detail}", n.text))
                    .with_label(n.span, "used here"),
            );
        }
        if !self.scopes.seen().non_local {
            debug_assert!(matches!(self.lookup(&n.text), None | Some(Binding::Local)));
            return None;
        }
        match self.lookup(&n.text) {
            None | Some(Binding::Local) => None,
            Some(b) => Some(b.clone()),
        }
    }

    fn is_local(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// A fresh hygienic temporary.
    fn temp(&mut self, base: &str) -> String {
        self.temps += 1;
        format!("__wl_{base}{}", self.temps)
    }

    /// A reference to a standard global (`tostring`, `bit.band`, …), through
    /// a hygienic alias when a user binding shadows it (spec §1.4.7).
    fn global(&mut self, name: &str, member: Option<&str>, pos: Pos) -> ir::Expr {
        if self.is_local(name) {
            let alias = match member {
                Some(m) => format!("__wl_{name}_{m}"),
                None => format!("__wl_{name}"),
            };
            self.aliases.insert(
                alias.clone(),
                (name.to_string(), member.map(str::to_string)),
            );
            return name_expr(generated(&alias, pos));
        }
        let base = name_expr(ident(name, pos));
        match member {
            Some(m) => field(base, m, pos),
            None => base,
        }
    }

    fn error(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        self.errors
            .push(Diagnostic::error(code, message).with_label(span, "here"));
    }

    // ------------------------------------------------------------ prelude --

    /// Runs `f` with a fresh prelude and returns its statements, wrapped in
    /// `do … end` with the prelude when there is one (codegen §14.4).
    fn wrapped(&mut self, pos: Pos, f: impl FnOnce(&mut Self) -> Vec<ir::Stmt>) -> Vec<ir::Stmt> {
        let saved = std::mem::take(&mut self.prelude);
        let stmts = f(self);
        let pre = std::mem::replace(&mut self.prelude, saved);
        if pre.is_empty() {
            stmts
        } else {
            let mut all = pre;
            all.extend(stmts);
            vec![do_block(all, pos)]
        }
    }

    /// Lowers `f` into a separate prelude buffer (for code that must only
    /// run conditionally) and returns (buffer, result).
    fn isolated<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> (Vec<ir::Stmt>, R) {
        let saved = std::mem::take(&mut self.prelude);
        let result = f(self);
        let pre = std::mem::replace(&mut self.prelude, saved);
        (pre, result)
    }

    /// Lowers operands left to right. When one needs preceding statements,
    /// earlier operands with side effects are moved into temporaries first,
    /// so Lua's evaluation order is kept (codegen §14.4).
    fn seq(&mut self, items: &[&Expr]) -> Vec<ir::Expr> {
        let mut out: Vec<ir::Expr> = Vec::with_capacity(items.len());
        for e in items {
            let start = self.prelude.len();
            let lowered = self.expr(e);
            if self.prelude.len() > start {
                let mut hoisted = Vec::new();
                for prev in out.iter_mut() {
                    if has_side_effects(prev) {
                        let pos = prev.pos;
                        let t = self.temp("t");
                        let value = std::mem::replace(prev, name_expr(generated(&t, pos)));
                        hoisted.push(local(generated(&t, pos), value, pos));
                    }
                }
                self.prelude.splice(start..start, hoisted);
            }
            out.push(lowered);
        }
        out
    }

    /// Stores `e` in a temporary in the prelude and returns a reference.
    fn store_temp(&mut self, e: ir::Expr, base: &str) -> ir::Expr {
        let pos = e.pos;
        let t = self.temp(base);
        self.prelude.push(local(generated(&t, pos), e, pos));
        name_expr(generated(&t, pos))
    }

    // ------------------------------------------------------------- blocks --

    fn stmts(&mut self, stmts: &[Stmt]) -> Vec<ir::Stmt> {
        let mut out = Vec::new();
        for s in stmts {
            out.extend(self.stmt(s));
        }
        out
    }

    fn block(&mut self, b: &Block) -> ir::Block {
        self.push_scope();
        let stmts = self.stmts(&b.stmts);
        self.pop_scope();
        ir::Block {
            stmts,
            end: self.next_token_pos(b.span.end),
        }
    }

    fn stmt(&mut self, s: &Stmt) -> Vec<ir::Stmt> {
        if !s.decorators.is_empty() && !self.kept_by_if(s) {
            return Vec::new();
        }
        if s.decorators
            .iter()
            .any(|d| wowlua_sema::meta::if_condition(d).is_none())
        {
            return grow(|| self.decorated_stmt(s));
        }
        grow(|| self.stmt_inner(s))
    }

    /// `@if(condition)` (spec §6.4.3): a declaration whose static condition
    /// is false is removed; its names are remembered for E1020.
    fn kept_by_if(&mut self, s: &Stmt) -> bool {
        let import = matches!(s.kind, StmtKind::Import(_));
        let keep = self.kept_by_if_decorators(&s.decorators, import);
        if !keep {
            for name in declared_names(s) {
                self.declare(&name, Binding::Removed(s.span));
            }
        }
        keep
    }

    /// Evaluates the `@if` decorators of a declaration or class member.
    fn kept_by_if_decorators(&mut self, decorators: &[Decorator], import: bool) -> bool {
        let mut keep = true;
        for d in decorators {
            let Some(cond) = wowlua_sema::meta::if_condition(d) else {
                continue;
            };
            let Some(cond) = cond else {
                if !import {
                    self.error("E1040", d.span, "`@if` takes one condition");
                }
                continue;
            };
            match self.static_value_with(cond, !import) {
                Some(c) => keep &= c.truthy(),
                // Conditions on imports are checked by the module analysis.
                None if import => {}
                None if self.meta().is_some_and(|m| m.flavor.is_none())
                    && self.uses_flavor(cond) =>
                {
                    self.error(
                        "E1039",
                        cond.span,
                        "`flavor` is not a constant in shared flavor mode; use a runtime check",
                    )
                }
                None => self.error(
                    "E1040",
                    cond.span,
                    "`@if` conditions must be static: `wowlua:meta` paths, constants and literals",
                ),
            }
        }
        keep
    }

    fn uses_flavor(&self, e: &Expr) -> bool {
        let mut found = false;
        wowlua_syntax::visit::walk_expr(e, &mut |x| {
            if let Some((MetaKind::Flavor, _)) = self.meta_path(x) {
                found = true;
            }
        });
        found
    }

    fn stmt_inner(&mut self, s: &Stmt) -> Vec<ir::Stmt> {
        use ir::StmtKind as K;
        let pos = self.span_pos(s.span);
        match &s.kind {
            StmtKind::Local { names, values } => {
                let saved = std::mem::take(&mut self.prelude);
                let refs: Vec<&Expr> = values.iter().collect();
                let mut lowered = self.seq(&refs);
                self.set_separators(values, &mut lowered);
                let mut out = std::mem::replace(&mut self.prelude, saved);
                for n in names {
                    self.declare(&n.name.text, Binding::Local);
                }
                out.push(stmt(
                    K::Local {
                        names: names.iter().map(|n| self.ident(&n.name)).collect(),
                        values: lowered,
                    },
                    pos,
                ));
                out
            }
            StmtKind::Assign { targets, values } => {
                self.wrapped(pos, |l| l.assignment(targets, values, pos))
            }
            StmtKind::Compound { target, op, value } => {
                self.wrapped(pos, |l| l.compound(target, *op, value, pos))
            }
            StmtKind::Call(e) if self.is_removed_call(e) => Vec::new(),
            StmtKind::Call(e) if self.call_intrinsic(e).is_some() => {
                let id = self.call_intrinsic(e).expect("an intrinsic");
                self.wrapped(pos, |l| l.intrinsic_stmt(&id, e, pos))
            }
            StmtKind::Call(e) => self.wrapped(pos, |l| {
                if chain_has_optional(e) {
                    l.optional_chain(e, true);
                    Vec::new()
                } else {
                    vec![stmt(K::Call(l.expr(e)), pos)]
                }
            }),
            StmtKind::Do(b) => vec![stmt(K::Do(self.block(b)), pos)],
            StmtKind::While { cond, body } => {
                let do_pos = self.next_token_pos(cond.span.end);
                let (pre, c) = self.isolated(|l| l.expr(cond));
                let end = self.next_token_pos(body.span.end);
                if pre.is_empty() {
                    let body = self.loop_body(body, do_pos, end, None);
                    vec![stmt(
                        K::While {
                            cond: c,
                            do_pos,
                            body,
                        },
                        pos,
                    )]
                } else {
                    // `while true do <pre>; if not (c) then break end; body end`
                    let mut check = pre;
                    let negated = ir::Expr::new(
                        ir::ExprKind::Unary {
                            op: ir::UnOp::Not,
                            operand: Box::new(paren(c)),
                        },
                        pos,
                    );
                    check.push(if_stmt(negated, vec![stmt(K::Break, pos)], pos));
                    let body = self.loop_body(body, do_pos, end, Some(check));
                    vec![stmt(
                        K::While {
                            cond: lit(ir::ExprKind::True, pos),
                            do_pos,
                            body,
                        },
                        pos,
                    )]
                }
            }
            StmtKind::Repeat { body, cond } => {
                let until = self.next_token_pos(body.span.end);
                self.push_scope();
                let mut lowered = self.loop_body_stmts(body, pos, until);
                let (pre, c) = self.isolated(|l| l.expr(cond));
                lowered.extend(pre);
                self.pop_scope();
                vec![stmt(
                    K::Repeat {
                        body: ir::Block {
                            stmts: lowered,
                            end: until,
                        },
                        cond: c,
                    },
                    pos,
                )]
            }
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                let mut clauses: Vec<(&Expr, &Block)> = vec![(cond, then_block)];
                clauses.extend(elseifs.iter().map(|(c, b)| (c, b)));
                self.if_chain(&clauses, else_block.as_ref(), pos)
            }
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => self.wrapped(pos, |l| {
                let mut items = vec![start, limit];
                if let Some(s) = step {
                    items.push(s);
                }
                let mut header = l.seq(&items);
                let step = (header.len() == 3).then(|| header.pop().expect("step"));
                let limit_ir = header.pop().expect("limit");
                let start_ir = header.pop().expect("start");
                let last_end = match &step {
                    Some(_) => items.last().map(|e| e.span.end).unwrap_or(limit.span.end),
                    None => limit.span.end,
                };
                let do_pos = l.next_token_pos(last_end);
                let end = l.next_token_pos(body.span.end);
                l.push_scope();
                l.declare(&var.name.text, Binding::Local);
                let body_ir = l.loop_body(body, do_pos, end, None);
                l.pop_scope();
                vec![stmt(
                    K::NumericFor {
                        var: l.ident(&var.name),
                        start: start_ir,
                        limit: limit_ir,
                        step,
                        do_pos,
                        body: body_ir,
                    },
                    pos,
                )]
            }),
            StmtKind::GenericFor { vars, exprs, body } => self.wrapped(pos, |l| {
                let refs: Vec<&Expr> = exprs.iter().collect();
                let mut lowered = l.seq(&refs);
                l.set_separators(exprs, &mut lowered);
                let do_pos = exprs
                    .last()
                    .map(|e| l.next_token_pos(e.span.end))
                    .unwrap_or(Pos::NONE);
                let end = l.next_token_pos(body.span.end);
                l.push_scope();
                for v in vars {
                    l.declare(&v.name.text, Binding::Local);
                }
                let body_ir = l.loop_body(body, do_pos, end, None);
                l.pop_scope();
                vec![stmt(
                    K::GenericFor {
                        vars: vars.iter().map(|v| l.ident(&v.name)).collect(),
                        exprs: lowered,
                        do_pos,
                        body: body_ir,
                    },
                    pos,
                )]
            }),
            StmtKind::Function { name, func } => {
                if name.path.len() == 1 && name.method.is_none() {
                    self.check_assignable(&name.path[0]);
                }
                let method = name.method.is_some();
                self.next_fn_name = Some(func_name(name));
                vec![stmt(
                    K::Function {
                        path: name.path.iter().map(|n| self.ident(n)).collect(),
                        method: name.method.as_ref().map(|n| self.ident(n)),
                        func: self.func(func, method),
                    },
                    pos,
                )]
            }
            StmtKind::LocalFunction { name, func } => {
                self.declare(&name.text, Binding::Local);
                self.next_fn_name = Some(name.text.to_string());
                vec![stmt(
                    K::LocalFunction {
                        name: self.ident(name),
                        func: self.func(func, false),
                    },
                    pos,
                )]
            }
            // A widget constructor returns its frame (spec §3.2).
            StmtKind::Return(values)
                if values.is_empty() && self.widget_ctor == Some(self.func_depth) =>
            {
                vec![stmt(K::Return(vec![name_expr(ident("self", pos))]), pos)]
            }
            StmtKind::Return(values) => self.wrapped(pos, |l| {
                let refs: Vec<&Expr> = values.iter().collect();
                let mut lowered = l.seq(&refs);
                l.set_separators(values, &mut lowered);
                vec![stmt(K::Return(lowered), pos)]
            }),
            StmtKind::Break => match self.loops.last().cloned() {
                Some(LoopInfo {
                    wrapped: true,
                    break_flag: Some(flag),
                }) => vec![
                    assign(
                        name_expr(generated(&flag, pos)),
                        lit(ir::ExprKind::True, pos),
                        pos,
                    ),
                    stmt(K::Break, pos),
                ],
                _ => vec![stmt(K::Break, pos)],
            },
            // `continue` leaves the `repeat … until true` wrapper.
            StmtKind::Continue => vec![stmt(K::Break, pos)],
            StmtKind::LocalDestructure { fields, value, .. } => {
                self.destructure(fields, value.as_ref(), s.span, pos)
            }
            StmtKind::Const { name, value, .. } => {
                let Some(value) = value else {
                    return Vec::new();
                };
                if let Some(c) = self.const_value(value) {
                    self.declare(&name.text, Binding::Const(c));
                    return Vec::new();
                }
                let saved = std::mem::take(&mut self.prelude);
                let lowered = self.expr(value);
                let mut out = std::mem::replace(&mut self.prelude, saved);
                self.declare(&name.text, Binding::ConstLocal);
                out.push(stmt(
                    K::Local {
                        names: vec![self.ident(name)],
                        values: vec![lowered],
                    },
                    pos,
                ));
                out
            }
            // Type-only declarations are erased.
            StmtKind::TypeAlias(_) | StmtKind::Interface(_) | StmtKind::Declare(_) => Vec::new(),
            StmtKind::Import(import) => self.import(import, s.span, pos),
            StmtKind::Export(inner) => self.export(inner, pos),
            StmtKind::Class(c) => self.class_decl(c, &decorate::Decorations::default(), false),
            StmtKind::Enum(e) => self.enum_decl(e, &decorate::Decorations::default(), false),
            StmtKind::Error => Vec::new(),
        }
    }

    fn ident(&self, n: &Name) -> ir::Ident {
        ir::Ident::new(&*n.text, self.span_pos(n.span))
    }

    fn set_separators(&self, es: &[Expr], out: &mut [ir::Expr]) {
        for (i, e) in es.iter().enumerate().take(es.len().saturating_sub(1)) {
            if let Some(x) = out.get_mut(i) {
                x.sep = self.next_token_pos(e.span.end);
            }
        }
    }

    /// Assignment to a constant (E2010) or to an imported or exported
    /// binding (E1010) is an error.
    fn check_assignable(&mut self, name: &Name) {
        if !self.scopes.seen().non_local {
            debug_assert!(matches!(
                self.lookup(&name.text),
                None | Some(Binding::Local)
            ));
            return;
        }
        match self.lookup(&name.text) {
            Some(Binding::Const(_) | Binding::ConstLocal) => self.error(
                "E2010",
                name.span,
                format!("cannot assign to the constant `{}`", name.text),
            ),
            Some(Binding::Enum { .. }) => self.error(
                "E2010",
                name.span,
                format!("cannot assign to the enum `{}`", name.text),
            ),
            Some(
                Binding::Import
                | Binding::LazyImport { .. }
                | Binding::Meta(_)
                | Binding::MetaNamespace
                | Binding::ImportedClass(_)
                | Binding::ImportedEnum(_)
                | Binding::Intrinsic(_)
                | Binding::IntrinsicNamespace(_)
                | Binding::DebugOnlyRemoved,
            ) => self.error(
                "E1010",
                name.span,
                format!("cannot assign to the imported binding `{}`", name.text),
            ),
            Some(Binding::Export | Binding::Class { exported: true, .. }) => self.error(
                "E1010",
                name.span,
                format!(
                    "cannot assign to the exported binding `{}`; mutate a table's contents or export accessor functions",
                    name.text
                ),
            ),
            _ => {}
        }
    }

    // -------------------------------------------------------------- loops --

    /// The statements of a loop body, wrapped for `continue` when needed
    /// (spec §1.3.6). The body's own scope is opened by the caller for
    /// `repeat` (its `until` sees the body's locals).
    fn loop_body_stmts(&mut self, body: &Block, anchor: Pos, end: Pos) -> Vec<ir::Stmt> {
        let (has_continue, has_break) = loop_flags(&body.stmts);
        if !has_continue {
            self.loops.push(LoopInfo {
                wrapped: false,
                break_flag: None,
            });
            let stmts = self.stmts(&body.stmts);
            self.loops.pop();
            return stmts;
        }
        let flag = has_break.then(|| self.temp("brk"));
        self.loops.push(LoopInfo {
            wrapped: true,
            break_flag: flag.clone(),
        });
        self.push_scope();
        let inner = self.stmts(&body.stmts);
        self.pop_scope();
        self.loops.pop();
        let mut out = Vec::new();
        if let Some(flag) = &flag {
            out.push(local(
                generated(flag, anchor),
                lit(ir::ExprKind::False, anchor),
                anchor,
            ));
        }
        out.push(stmt(
            ir::StmtKind::Repeat {
                body: ir::Block { stmts: inner, end },
                cond: lit(ir::ExprKind::True, end),
            },
            anchor,
        ));
        if let Some(flag) = &flag {
            out.push(if_stmt(
                name_expr(generated(flag, end)),
                vec![stmt(ir::StmtKind::Break, end)],
                end,
            ));
        }
        out
    }

    fn loop_body(
        &mut self,
        body: &Block,
        anchor: Pos,
        end: Pos,
        check: Option<Vec<ir::Stmt>>,
    ) -> ir::Block {
        self.push_scope();
        let mut stmts = check.unwrap_or_default();
        stmts.extend(self.loop_body_stmts(body, anchor, end));
        self.pop_scope();
        ir::Block { stmts, end }
    }

    // ----------------------------------------------------------------- if --

    fn if_chain(
        &mut self,
        clauses: &[(&Expr, &Block)],
        else_block: Option<&Block>,
        pos: Pos,
    ) -> Vec<ir::Stmt> {
        // Static conditions (spec §6.4.2): a false branch is removed (not
        // lowered); a true branch becomes the final one.
        let mut kept: Vec<(&Expr, &Block)> = Vec::new();
        let mut last = else_block;
        for &(c, b) in clauses {
            match self.static_condition(c) {
                Some(v) if v.truthy() => {
                    last = Some(b);
                    break;
                }
                Some(_) => {}
                None => kept.push((c, b)),
            }
        }
        if kept.is_empty() {
            return match last {
                Some(b) => vec![stmt(ir::StmtKind::Do(self.block(b)), pos)],
                None => Vec::new(),
            };
        }
        let clauses = &kept[..];
        let else_block = last;
        let first = self.isolated(|l| l.expr(clauses[0].0));
        let (pre, s) = self.if_rest(first, clauses, else_block, pos);
        if pre.is_empty() {
            vec![s]
        } else {
            let mut all = pre;
            all.push(s);
            vec![do_block(all, pos)]
        }
    }

    /// An `if` whose first condition is already lowered (`first`: its
    /// preceding statements and value). An `elseif` with preceding
    /// statements becomes `else` + a nested `if` (codegen §14.4).
    fn if_rest(
        &mut self,
        first: (Vec<ir::Stmt>, ir::Expr),
        clauses: &[(&Expr, &Block)],
        else_block: Option<&Block>,
        pos: Pos,
    ) -> (Vec<ir::Stmt>, ir::Stmt) {
        let (pre, cond) = first;
        let then_pos = self.next_token_pos(clauses[0].0.span.end);
        let mut ir_clauses = vec![ir::IfClause {
            cond,
            then_pos,
            body: self.block(clauses[0].1),
        }];
        let mut else_ir = None;
        let mut i = 1;
        while i < clauses.len() {
            let (c, b) = clauses[i];
            let (cpre, cond) = self.isolated(|l| l.expr(c));
            if !cpre.is_empty() {
                let cpos = self.span_pos(c.span);
                let (npre, nested) = self.if_rest((cpre, cond), &clauses[i..], else_block, cpos);
                let mut stmts = npre;
                stmts.push(nested);
                else_ir = Some(ir::Block {
                    stmts,
                    end: Pos::NONE,
                });
                return (
                    pre,
                    stmt(
                        ir::StmtKind::If {
                            clauses: ir_clauses,
                            else_block: else_ir,
                        },
                        pos,
                    ),
                );
            }
            let then_pos = self.next_token_pos(c.span.end);
            ir_clauses.push(ir::IfClause {
                cond,
                then_pos,
                body: self.block(b),
            });
            i += 1;
        }
        if let Some(e) = else_block {
            else_ir = Some(self.block(e));
        }
        (
            pre,
            stmt(
                ir::StmtKind::If {
                    clauses: ir_clauses,
                    else_block: else_ir,
                },
                pos,
            ),
        )
    }

    // ---------------------------------------------------------- functions --

    fn func(&mut self, f: &FuncBody, method: bool) -> ir::Func {
        let name = self.next_fn_name.take();
        let mut marks = std::mem::take(&mut self.next_fn_marks);
        marks.label = name.clone();
        let checks = self.checks_function(&marks);
        if marks.reflect && self.ctx.tools.reflection > 0 {
            let label = name.clone().unwrap_or_else(|| "?".into());
            marks.reflect_meta = Some(self.function_meta(f, &label));
        }
        self.fn_names.push(name);
        let lowered = self.func_inner(f, method, checks);
        self.fn_names.pop();
        self.fn_marks.insert(lowered.pos, marks);
        lowered
    }

    fn func_inner(&mut self, f: &FuncBody, method: bool, checks: bool) -> ir::Func {
        self.func_depth += 1;
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_prelude = std::mem::take(&mut self.prelude);
        self.push_scope();
        if method {
            self.declare("self", Binding::Local);
        }
        for p in &f.params {
            self.declare(&p.name.text, Binding::Local);
        }
        let pos = self.span_pos(f.params_span);
        let mut stmts = self.default_params(f);
        if checks {
            // Entry checks (spec §7.5), after default values.
            let label = self
                .fn_names
                .last()
                .cloned()
                .flatten()
                .unwrap_or_else(|| "?".into());
            stmts.extend(self.entry_checks(f, &label));
        }
        stmts.extend(self.stmts(&f.body.stmts));
        self.pop_scope();
        self.loops = saved_loops;
        self.prelude = saved_prelude;
        self.func_depth -= 1;
        ir::Func {
            pos,
            params: f.params.iter().map(|p| self.ident(&p.name)).collect(),
            vararg: f.vararg.is_some(),
            body: ir::Block {
                stmts,
                end: self.span_pos(f.end_span),
            },
        }
    }

    /// Default parameter values (spec §1.3.5), on the parameter line; the
    /// parameters are declared by the caller.
    fn default_params(&mut self, f: &FuncBody) -> Vec<ir::Stmt> {
        let pos = self.span_pos(f.params_span);
        let mut stmts = Vec::new();
        for p in &f.params {
            if let Some(default) = &p.default {
                let (pre, value) = self.isolated(|l| l.expr(default));
                let name = self.ident(&p.name);
                let mut body = pre;
                body.push(assign(name_expr(name.clone()), value, pos));
                let cond = binary(
                    ir::BinOp::Eq,
                    name_expr(name),
                    lit(ir::ExprKind::Nil, pos),
                    pos,
                );
                stmts.push(if_stmt(cond, body, pos));
            }
        }
        stmts
    }

    /// A call statement of a removed `@debugOnly` function (spec §4.3).
    fn is_removed_call(&self, e: &Expr) -> bool {
        if !self.scopes.seen().non_local {
            return false;
        }
        matches!(&e.kind, ExprKind::Call { callee, .. } if matches!(&callee.kind, ExprKind::Name(n) if matches!(self.lookup(&n.text), Some(Binding::DebugOnlyRemoved))))
    }

    /// The arguments of a call as a list.
    fn args_list(&mut self, args: &Args) -> Vec<ir::Expr> {
        match args {
            Args::Parens { args, .. } => self.exprs(args),
            Args::Table(e) | Args::String(e) => vec![self.expr(e)],
        }
    }

    // ------------------------------------------------------------ modules --

    fn runtime_ref(&mut self, pos: Pos) -> ir::Expr {
        self.uses_runtime = true;
        name_expr(generated("__wl", pos))
    }

    /// `__wl.require("id")` (`, true` for lazy bindings).
    fn require(&mut self, id: &str, lazy: bool, pos: Pos) -> ir::Expr {
        let wl = self.runtime_ref(pos);
        let mut args = vec![lit(ir::ExprKind::String(quote(id.as_bytes())), pos)];
        if lazy {
            args.push(lit(ir::ExprKind::True, pos));
        }
        call(field(wl, "require", pos), args, pos)
    }

    /// An import (spec §5.2, §5.5): one local per binding, on the import's
    /// line; folded constants are inlined; types are erased.
    fn import(&mut self, import: &Import, span: Span, pos: Pos) -> Vec<ir::Stmt> {
        if import.type_only {
            return Vec::new();
        }
        let Some(target) = self.ctx.imports.get(&span.start).cloned() else {
            self.error(
                "E1015",
                span,
                "imports need the package context (build the package with `wowlua build`)",
            );
            return Vec::new();
        };
        let (id, lazy, exports, attrs, intrinsics) = match target {
            ImportContext::Module {
                id,
                lazy,
                exports,
                attrs,
                intrinsics,
            } => (id, lazy, exports, attrs, intrinsics),
            ImportContext::Meta => {
                match &import.clause {
                    ImportClause::Names(specs) => {
                        for spec in specs {
                            if let Some(kind) = MetaKind::from_export(&spec.name.text) {
                                let local_name = spec.alias.as_ref().unwrap_or(&spec.name);
                                self.declare(&local_name.text, Binding::Meta(kind));
                            }
                        }
                    }
                    ImportClause::Namespace(name) => {
                        self.declare(&name.text, Binding::MetaNamespace)
                    }
                    ImportClause::SideEffect => {}
                }
                return Vec::new();
            }
            ImportContext::LoadOnly => return Vec::new(),
        };
        match &import.clause {
            ImportClause::SideEffect => Vec::new(),
            ImportClause::Namespace(name) => {
                let binding = if intrinsics.is_empty() {
                    Binding::Import
                } else {
                    Binding::IntrinsicNamespace(Arc::new(intrinsics.clone()))
                };
                self.declare(&name.text, binding);
                self.module_tables
                    .insert(id.clone(), (name.text.to_string(), false));
                let value = self.require(&id, lazy, pos);
                vec![local(self.ident(name), value, pos)]
            }
            ImportClause::Names(specs) => {
                let mut values: Vec<&ImportSpec> = Vec::new();
                let mut kinds: Vec<(String, Binding)> = Vec::new();
                for spec in specs {
                    let local_name = spec.alias.as_ref().unwrap_or(&spec.name);
                    let attr = attrs
                        .get(spec.name.text.as_ref())
                        .cloned()
                        .unwrap_or_default();
                    if let Some(message) = attr.deprecated {
                        self.mark_deprecated(&local_name.text, message);
                    }
                    if attr.debug_only && !self.ctx.debug {
                        self.declare(&local_name.text, Binding::DebugOnlyRemoved);
                        continue;
                    }
                    match exports.get(spec.name.text.as_ref()) {
                        Some(wowlua_sema::Export::Const(Some(c))) => {
                            self.declare(&local_name.text, Binding::Const(c.clone()))
                        }
                        Some(wowlua_sema::Export::Type) => {}
                        Some(wowlua_sema::Export::Class(info)) => {
                            kinds.push((
                                local_name.text.to_string(),
                                Binding::ImportedClass(info.clone()),
                            ));
                            values.push(spec);
                        }
                        Some(wowlua_sema::Export::Enum(info)) => {
                            kinds.push((
                                local_name.text.to_string(),
                                Binding::ImportedEnum(info.clone()),
                            ));
                            values.push(spec);
                        }
                        _ if intrinsics.contains_key(spec.name.text.as_ref()) => {
                            let id = intrinsics[spec.name.text.as_ref()].clone();
                            kinds.push((local_name.text.to_string(), Binding::Intrinsic(id)));
                            values.push(spec);
                        }
                        _ => values.push(spec),
                    }
                }
                if values.is_empty() {
                    return Vec::new();
                }
                if lazy {
                    let table = self.temp("m");
                    self.module_tables.insert(id.clone(), (table.clone(), true));
                    let value = self.require(&id, true, pos);
                    for spec in &values {
                        let local_name = spec.alias.as_ref().unwrap_or(&spec.name);
                        self.declare(
                            &local_name.text,
                            Binding::LazyImport {
                                table: table.clone(),
                                name: spec.name.text.to_string(),
                            },
                        );
                    }
                    return vec![local(generated(&table, pos), value, pos)];
                }
                let mut out = Vec::new();
                let source = if values.len() == 1 && intrinsics.is_empty() {
                    self.require(&id, false, pos)
                } else {
                    let table = self.temp("m");
                    let value = self.require(&id, false, pos);
                    out.push(local(generated(&table, pos), value, pos));
                    self.module_tables.insert(id.clone(), (table.clone(), true));
                    name_expr(generated(&table, pos))
                };
                let names: Vec<ir::Ident> = values
                    .iter()
                    .map(|spec| self.ident(spec.alias.as_ref().unwrap_or(&spec.name)))
                    .collect();
                let fields: Vec<ir::Expr> = values
                    .iter()
                    .map(|spec| field(source.clone(), &spec.name.text, pos))
                    .collect();
                for spec in &values {
                    let local_name = spec.alias.as_ref().unwrap_or(&spec.name);
                    self.declare(&local_name.text, Binding::Import);
                }
                for (name, binding) in kinds {
                    self.declare(&name, binding);
                }
                out.push(stmt(
                    ir::StmtKind::Local {
                        names,
                        values: fields,
                    },
                    pos,
                ));
                out
            }
        }
    }

    fn export_assign(&mut self, name: &str, value: ir::Expr, pos: Pos) -> ir::Stmt {
        self.has_exports = true;
        let table = name_expr(generated("__wl_exports", pos));
        assign(field(table, name, pos), value, pos)
    }

    /// `export <declaration>` (spec §5.1): the declaration, then its values
    /// stored in the module's export table.
    fn export(&mut self, inner: &Stmt, pos: Pos) -> Vec<ir::Stmt> {
        match &inner.kind {
            StmtKind::Local { names, .. } => {
                let mut out = self.stmt(inner);
                for n in names {
                    self.declare(&n.name.text, Binding::Export);
                    let value = name_expr(self.ident(&n.name));
                    let s = self.export_assign(&n.name.text, value, pos);
                    out.push(s);
                }
                out
            }
            StmtKind::Function { name, func } => {
                let n = &name.path[0];
                let fpos = self.span_pos(inner.span);
                self.next_fn_name = Some(func_name(name));
                self.next_fn_marks.exported = true;
                let f = self.func(func, false);
                self.pending.remove(n.text.as_ref());
                let target = name_expr(self.ident(n));
                let value = ir::Expr::new(ir::ExprKind::Function(Box::new(f)), fpos);
                let export = self.export_assign(&n.text, name_expr(self.ident(n)), pos);
                vec![assign(target, value, fpos), export]
            }
            StmtKind::Const { name, value, .. } => {
                let mut out = self.stmt(inner);
                let exported = match self.lookup(&name.text).cloned() {
                    Some(Binding::Const(c)) => c.to_expr(pos),
                    _ => {
                        self.declare(&name.text, Binding::Export);
                        name_expr(self.ident(name))
                    }
                };
                if value.is_some() {
                    let s = self.export_assign(&name.text, exported, pos);
                    out.push(s);
                }
                out
            }
            StmtKind::Class(c) => self.class_decl(c, &decorate::Decorations::default(), true),
            StmtKind::Enum(e) => self.enum_decl(e, &decorate::Decorations::default(), true),
            // `export type`, `export interface`: types only.
            _ => Vec::new(),
        }
    }

    // -------------------------------------------------------- assignments --

    /// The parts of an assignment target, lowered: `(object, key)` for
    /// fields and indexes, `(name, None)` for names.
    fn target_parts(t: &Expr) -> Vec<&Expr> {
        match &t.kind {
            ExprKind::Index { obj, key } => vec![obj, key],
            ExprKind::Field { obj, .. } => vec![obj],
            _ => Vec::new(),
        }
    }

    /// Assignment to an enum member (E2010).
    fn check_enum_target(&mut self, t: &Expr) {
        if !self.scopes.seen().enums {
            return;
        }
        let obj = match &t.kind {
            ExprKind::Field { obj, .. } | ExprKind::Index { obj, .. } => obj,
            _ => return,
        };
        if let ExprKind::Name(n) = &obj.kind
            && matches!(
                self.lookup(&n.text),
                Some(Binding::Enum { .. } | Binding::ImportedEnum(_))
            )
        {
            self.error(
                "E2010",
                t.span,
                format!("cannot assign to a member of the enum `{}`", n.text),
            );
        }
    }

    fn assignment(&mut self, targets: &[Expr], values: &[Expr], pos: Pos) -> Vec<ir::Stmt> {
        for t in targets {
            self.check_meta_target(t);
            self.check_enum_target(t);
            if let ExprKind::Name(n) = &t.kind {
                self.check_assignable(n);
            }
        }
        // Lua evaluates the targets' subexpressions, then the values.
        let mut operands: Vec<&Expr> = Vec::new();
        let mut counts = Vec::new();
        for t in targets {
            let parts = Self::target_parts(t);
            counts.push(parts.len());
            operands.extend(parts);
        }
        operands.extend(values.iter());
        let mut lowered = self.seq(&operands).into_iter();
        let mut ir_targets = Vec::new();
        for (t, n) in targets.iter().zip(counts) {
            let tpos = self.span_pos(t.span);
            let kind = match (&t.kind, n) {
                (ExprKind::Index { .. }, 2) => {
                    let obj = lowered.next().expect("object");
                    let key = lowered.next().expect("key");
                    ir::ExprKind::Index {
                        obj: Box::new(obj),
                        key: Box::new(key),
                    }
                }
                (ExprKind::Field { name, .. }, 1) => ir::ExprKind::Field {
                    obj: Box::new(lowered.next().expect("object")),
                    name: self.ident(name),
                },
                (ExprKind::Name(n), _) => ir::ExprKind::Name(self.ident(n)),
                _ => ir::ExprKind::Nil,
            };
            ir_targets.push(ir::Expr::new(kind, tpos));
        }
        let mut ir_values: Vec<ir::Expr> = lowered.collect();
        self.set_separators(values, &mut ir_values);
        self.set_separators(targets, &mut ir_targets);
        vec![stmt(
            ir::StmtKind::Assign {
                targets: ir_targets,
                values: ir_values,
            },
            pos,
        )]
    }

    /// `a op= e` (spec §1.3.4): every subexpression of the target that
    /// contains a call is evaluated once.
    fn compound(&mut self, target: &Expr, op: CompoundOp, value: &Expr, pos: Pos) -> Vec<ir::Stmt> {
        self.check_meta_target(target);
        self.check_enum_target(target);
        let tpos = self.span_pos(target.span);
        let (write, read) = match &target.kind {
            ExprKind::Name(n) => {
                self.check_assignable(n);
                let e = name_expr(self.ident(n));
                (e.clone(), e)
            }
            ExprKind::Field { obj, name } => {
                let o = self.expr(obj);
                let o = if is_name(&o) {
                    o
                } else {
                    self.store_temp(o, "o")
                };
                let e = ir::Expr::new(
                    ir::ExprKind::Field {
                        obj: Box::new(o),
                        name: self.ident(name),
                    },
                    tpos,
                );
                (e.clone(), e)
            }
            ExprKind::Index { obj, key } => {
                let o = self.expr(obj);
                let o = if is_name(&o) {
                    o
                } else {
                    self.store_temp(o, "o")
                };
                let k = self.expr(key);
                let k = if is_simple(&k) {
                    k
                } else {
                    self.store_temp(k, "k")
                };
                let e = ir::Expr::new(
                    ir::ExprKind::Index {
                        obj: Box::new(o),
                        key: Box::new(k),
                    },
                    tpos,
                );
                (e.clone(), e)
            }
            _ => return Vec::new(),
        };
        if op == CompoundOp::Coalesce {
            // `if a == nil then a = e end`: e only runs when needed.
            let (pre, v) = self.isolated(|l| l.expr(value));
            let mut body = pre;
            body.push(assign(write, v, pos));
            let cond = binary(ir::BinOp::Eq, read, lit(ir::ExprKind::Nil, pos), pos);
            return vec![if_stmt(cond, body, pos)];
        }
        let v = self.expr(value);
        let combined = match op {
            CompoundOp::Add => binary(ir::BinOp::Add, read, v, pos),
            CompoundOp::Sub => binary(ir::BinOp::Sub, read, v, pos),
            CompoundOp::Mul => binary(ir::BinOp::Mul, read, v, pos),
            CompoundOp::Div => binary(ir::BinOp::Div, read, v, pos),
            CompoundOp::Mod => binary(ir::BinOp::Mod, read, v, pos),
            CompoundOp::Pow => binary(ir::BinOp::Pow, read, v, pos),
            CompoundOp::Concat => binary(ir::BinOp::Concat, read, v, pos),
            CompoundOp::IDiv => self.idiv(read, v, pos),
            CompoundOp::BAnd => self.bit_call("band", vec![read, v], pos),
            CompoundOp::BOr => self.bit_call("bor", vec![read, v], pos),
            CompoundOp::Shl => self.bit_call("lshift", vec![read, v], pos),
            CompoundOp::Shr => self.bit_call("rshift", vec![read, v], pos),
            CompoundOp::Coalesce => unreachable!("handled above"),
        };
        vec![assign(write, combined, pos)]
    }

    /// `local { a, b = 1 } = e` (spec §1.3.3).
    fn destructure(
        &mut self,
        fields: &[DestructField],
        value: Option<&Expr>,
        span: Span,
        pos: Pos,
    ) -> Vec<ir::Stmt> {
        let Some(value) = value else {
            self.error("E0101", span, "`local { … }` needs a value to destructure");
            return Vec::new();
        };
        let saved = std::mem::take(&mut self.prelude);
        let v = self.expr(value);
        // A local or upvalue name is read directly; anything else once, into
        // a temporary that stays in scope until the end of the block.
        let source = match &v.kind {
            ir::ExprKind::Name(id) if !id.generated && self.is_local(&id.name) => v,
            _ => self.store_temp(v, "t"),
        };
        let mut out = std::mem::replace(&mut self.prelude, saved);
        let names: Vec<ir::Ident> = fields.iter().map(|f| self.ident(&f.name)).collect();
        let values: Vec<ir::Expr> = fields
            .iter()
            .map(|f| field(source.clone(), &f.name.text, self.span_pos(f.name.span)))
            .collect();
        for f in fields {
            self.declare(&f.name.text, Binding::Local);
        }
        out.push(stmt(ir::StmtKind::Local { names, values }, pos));
        for f in fields {
            if let Some(default) = &f.default {
                let (pre, d) = self.isolated(|l| l.expr(default));
                let name = self.ident(&f.name);
                let mut body = pre;
                body.push(assign(name_expr(name.clone()), d, pos));
                let cond = binary(
                    ir::BinOp::Eq,
                    name_expr(name),
                    lit(ir::ExprKind::Nil, pos),
                    pos,
                );
                out.push(if_stmt(cond, body, pos));
            }
        }
        out
    }

    // -------------------------------------------------------- expressions --

    fn exprs(&mut self, es: &[Expr]) -> Vec<ir::Expr> {
        let refs: Vec<&Expr> = es.iter().collect();
        let mut out = self.seq(&refs);
        self.set_separators(es, &mut out);
        out
    }

    fn expr(&mut self, e: &Expr) -> ir::Expr {
        grow(|| self.expr_inner(e))
    }

    fn expr_inner(&mut self, e: &Expr) -> ir::Expr {
        use ir::ExprKind as K;
        let pos = self.span_pos(e.span);
        if let Some((kind, path)) = self.meta_path(e) {
            return self.meta_expr(kind, path, e);
        }
        if let ExprKind::Call { callee, args } = &e.kind
            && let Some((MetaKind::Asset, path)) = self.meta_path(callee)
            && path.is_empty()
        {
            return self.asset(args, e);
        }
        if chain_has_optional(e) {
            return self
                .optional_chain(e, false)
                .expect("an expression chain has a value");
        }
        if let Some(routed) = self.monitor_call(e) {
            return routed;
        }
        let kind = match &e.kind {
            ExprKind::Nil => K::Nil,
            ExprKind::True => K::True,
            ExprKind::False => K::False,
            ExprKind::Number(n) => {
                if wowlua_syntax::lexer::is_wowlua_numeral(&n.raw) {
                    K::Number(format_number(n.value.unwrap_or(0.0)))
                } else {
                    K::Number(String::from(&*n.raw))
                }
            }
            ExprKind::String(s) => K::String(s.raw.to_vec()),
            ExprKind::Vararg => K::Vararg,
            ExprKind::Function(f) => K::Function(Box::new(self.func(f, false))),
            ExprKind::Table(fields) => self.table(fields, e.span),
            ExprKind::Name(n) if n.text.as_ref() == "super" && self.is_super(e) => {
                return self.super_value(n.span, pos);
            }
            ExprKind::Name(n) => match self.name_binding(n) {
                Some(Binding::Const(c)) => return c.to_expr(pos),
                Some(Binding::Intrinsic(id)) => {
                    self.check_intrinsic_value(&id, n.span);
                    K::Name(self.ident(n))
                }
                Some(Binding::DebugOnlyRemoved) => {
                    self.error(
                        "E3032",
                        n.span,
                        format!(
                            "`{}` is `@debugOnly`: it can only be called as a statement (the calls are removed from release builds)",
                            n.text
                        ),
                    );
                    K::Nil
                }
                Some(Binding::LazyImport { table, name }) => {
                    return field(name_expr(generated(&table, pos)), &name, pos);
                }
                Some(Binding::Removed(decl)) => {
                    self.errors.push(
                        Diagnostic::error(
                            "E1020",
                            format!("`{}` was removed by `@if` for this build", n.text),
                        )
                        .with_label(n.span, "used here")
                        .with_label(decl, "removed here"),
                    );
                    K::Name(self.ident(n))
                }
                Some(Binding::Export | Binding::Class { .. } | Binding::Enum { .. })
                    if self.func_depth == 0 && self.pending.contains(n.text.as_ref()) =>
                {
                    self.error(
                        "E1037",
                        n.span,
                        format!(
                            "`{}` is used before its definition runs (hoisted functions are `nil` until then)",
                            n.text
                        ),
                    );
                    K::Name(self.ident(n))
                }
                _ => K::Name(self.ident(n)),
            },
            ExprKind::Index { obj, key }
                if matches!(&key.kind, ExprKind::String(_))
                    && let ExprKind::String(k) = &key.kind
                    && let Some(v) = self.enum_member(
                        obj,
                        &String::from_utf8_lossy(&k.value),
                        key.span,
                        pos,
                    ) =>
            {
                return v;
            }
            ExprKind::Index { obj, key } => {
                let mut parts = self.seq(&[obj, key]).into_iter();
                K::Index {
                    obj: Box::new(parts.next().expect("object")),
                    key: Box::new(parts.next().expect("key")),
                }
            }
            ExprKind::Field { obj, name } => {
                if let Some(v) = self.enum_member(obj, &name.text, name.span, pos) {
                    return v;
                }
                K::Field {
                    obj: Box::new(self.expr(obj)),
                    name: self.ident(name),
                }
            }
            ExprKind::Method { obj, name, args } if self.is_super(obj) => {
                return self.super_method(obj, name, args, pos);
            }
            ExprKind::Call { callee, args } if self.call_intrinsic(e).is_some() => {
                let id = self.call_intrinsic(e).expect("an intrinsic");
                return self.intrinsic_expr(&id, callee, args, e);
            }
            ExprKind::Call { callee, args } => {
                self.check_instantiation(callee);
                let (callee, args) = self.call_parts(callee, args);
                K::Call {
                    callee: Box::new(callee),
                    args,
                }
            }
            ExprKind::Method { obj, name, args } => {
                let (obj, args) = self.call_parts(obj, args);
                K::Method {
                    obj: Box::new(obj),
                    name: self.ident(name),
                    args,
                }
            }
            ExprKind::Paren(inner) => K::Paren {
                inner: Box::new(self.expr(inner)),
                close: self.close_pos(e.span),
            },
            ExprKind::Binary { op, lhs, rhs } => return self.binary_expr(*op, lhs, rhs, e),
            ExprKind::Unary { op, operand } => {
                if *op == UnOp::BNot {
                    if let Some(Const::Number(n)) = self.const_value(operand)
                        && let Some(v) = constant::fold_bnot(n)
                    {
                        return constant::number_expr(v, pos);
                    }
                    let x = self.expr(operand);
                    return self.bit_call("bnot", vec![x], pos);
                }
                K::Unary {
                    op: match op {
                        UnOp::Neg => ir::UnOp::Neg,
                        UnOp::Not => ir::UnOp::Not,
                        _ => ir::UnOp::Len,
                    },
                    operand: Box::new(self.expr(operand)),
                }
            }
            ExprKind::Cast { expr, .. } => {
                let value = self.expr(expr);
                return self.checked_cast(e, value);
            }
            ExprKind::Interp(parts) => return self.interp(parts, pos),
            // Handled by `optional_chain`, or rejected by the parser.
            ExprKind::OptField { .. }
            | ExprKind::OptIndex { .. }
            | ExprKind::OptMethod { .. }
            | ExprKind::OptCall { .. }
            | ExprKind::Error => K::Nil,
        };
        ir::Expr::new(kind, pos)
    }

    fn call_parts(&mut self, callee: &Expr, args: &Args) -> (ir::Expr, ir::Args) {
        match args {
            Args::Parens { args, span } => {
                let mut items: Vec<&Expr> = vec![callee];
                items.extend(args.iter());
                let mut lowered = self.seq(&items);
                let callee = lowered.remove(0);
                self.set_separators(args, &mut lowered);
                (
                    callee,
                    ir::Args::Parens {
                        args: lowered,
                        close: self.close_pos(*span),
                    },
                )
            }
            Args::Table(t) => {
                let mut lowered = self.seq(&[callee, t]);
                let table = lowered.pop().expect("table");
                (
                    lowered.pop().expect("callee"),
                    ir::Args::Table(Box::new(table)),
                )
            }
            Args::String(s) => {
                let c = self.expr(callee);
                (c, ir::Args::String(Box::new(self.expr(s))))
            }
        }
    }

    fn table(&mut self, fields: &[TableField], span: Span) -> ir::ExprKind {
        // Both lists sized up front: the fields stay in the IR until the
        // file is emitted, and pushing would leave room unused.
        let keyed = fields
            .iter()
            .filter(|f| matches!(f, TableField::Keyed { .. }))
            .count();
        let mut operands: Vec<&Expr> = Vec::with_capacity(fields.len() + keyed);
        for f in fields {
            match f {
                TableField::Keyed { key, value, .. } => {
                    operands.push(key);
                    operands.push(value);
                }
                TableField::Named { value, .. } | TableField::Positional(value) => {
                    operands.push(value)
                }
            }
        }
        let mut lowered = self.seq(&operands).into_iter();
        let mut out = Vec::with_capacity(fields.len());
        for (i, f) in fields.iter().enumerate() {
            let mut lowered_field = match f {
                TableField::Keyed { span, .. } => ir::Field::Keyed {
                    open: self.span_pos(*span),
                    key: lowered.next().expect("key"),
                    value: lowered.next().expect("value"),
                },
                TableField::Named { name, .. } => ir::Field::Named {
                    name: self.ident(name),
                    value: lowered.next().expect("value"),
                },
                TableField::Positional(_) => ir::Field::Positional(lowered.next().expect("value")),
            };
            if i + 1 < fields.len() {
                let end = match f {
                    TableField::Keyed { value, .. }
                    | TableField::Named { value, .. }
                    | TableField::Positional(value) => value.span.end,
                };
                let sep = self.next_token_pos(end);
                match &mut lowered_field {
                    ir::Field::Keyed { value, .. }
                    | ir::Field::Named { value, .. }
                    | ir::Field::Positional(value) => value.sep = sep,
                }
            }
            out.push(lowered_field);
        }
        ir::ExprKind::Table {
            fields: out,
            close: self.close_pos(span),
        }
    }

    fn binary_expr(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, e: &Expr) -> ir::Expr {
        let pos = self.span_pos(e.span);
        let op_pos = self.next_token_pos(lhs.span.end);
        match op {
            BinOp::And | BinOp::Or | BinOp::Coalesce => {
                // A static left operand simplifies the expression (spec
                // §6.4.2); a static right operand does not.
                if let Some(c) = self.static_condition(lhs) {
                    let use_rhs = match op {
                        BinOp::And => c.truthy(),
                        BinOp::Or => !c.truthy(),
                        _ => c == Const::Nil,
                    };
                    return if use_rhs {
                        self.expr(rhs)
                    } else {
                        c.to_expr(pos)
                    };
                }
                let l = self.expr(lhs);
                let (pre, r) = self.isolated(|lw| lw.expr(rhs));
                if pre.is_empty() && op != BinOp::Coalesce {
                    let ir_op = if op == BinOp::And {
                        ir::BinOp::And
                    } else {
                        ir::BinOp::Or
                    };
                    return ir::Expr::new(
                        ir::ExprKind::Binary {
                            op: ir_op,
                            op_pos,
                            lhs: Box::new(l),
                            rhs: Box::new(r),
                        },
                        pos,
                    );
                }
                // An `if` on a temporary: the right operand's statements
                // run only when it is evaluated (codegen §14.4).
                let t = self.store_temp(l, "t");
                let cond = match op {
                    BinOp::And => t.clone(),
                    BinOp::Or => ir::Expr::new(
                        ir::ExprKind::Unary {
                            op: ir::UnOp::Not,
                            operand: Box::new(t.clone()),
                        },
                        pos,
                    ),
                    _ => binary(ir::BinOp::Eq, t.clone(), lit(ir::ExprKind::Nil, pos), pos),
                };
                let mut body = pre;
                body.push(assign(t.clone(), r, pos));
                self.prelude.push(if_stmt(cond, body, pos));
                t
            }
            BinOp::BAnd | BinOp::BOr | BinOp::BXor | BinOp::Shl | BinOp::Shr => {
                if let (Some(Const::Number(a)), Some(Const::Number(b))) =
                    (self.const_value(lhs), self.const_value(rhs))
                    && let Some(Const::Number(v)) =
                        constant::fold_binary(op, &Const::Number(a), &Const::Number(b))
                {
                    return constant::number_expr(v, pos);
                }
                let name = match op {
                    BinOp::BAnd => "band",
                    BinOp::BOr => "bor",
                    BinOp::BXor => "bxor",
                    BinOp::Shl => "lshift",
                    _ => "rshift",
                };
                // `a | b | c` becomes `bit.bor(a, b, c)`.
                let mut operands = vec![rhs];
                let mut left = lhs;
                if matches!(op, BinOp::BAnd | BinOp::BOr | BinOp::BXor) {
                    while let ExprKind::Binary {
                        op: inner,
                        lhs: l,
                        rhs: r,
                    } = &left.kind
                        && *inner == op
                    {
                        operands.push(r);
                        left = l;
                    }
                }
                operands.push(left);
                operands.reverse();
                let args = self.seq(&operands);
                self.bit_call(name, args, pos)
            }
            BinOp::IDiv => {
                if let (Some(Const::Number(a)), Some(Const::Number(b))) =
                    (self.const_value(lhs), self.const_value(rhs))
                {
                    return constant::number_expr((a / b).floor(), pos);
                }
                let mut parts = self.seq(&[lhs, rhs]).into_iter();
                let (a, b) = (parts.next().expect("lhs"), parts.next().expect("rhs"));
                self.idiv(a, b, pos)
            }
            _ => {
                let mut parts = self.seq(&[lhs, rhs]).into_iter();
                ir::Expr::new(
                    ir::ExprKind::Binary {
                        op: lua_binop(op),
                        op_pos,
                        lhs: Box::new(parts.next().expect("lhs")),
                        rhs: Box::new(parts.next().expect("rhs")),
                    },
                    pos,
                )
            }
        }
    }

    fn bit_call(&mut self, name: &str, args: Vec<ir::Expr>, pos: Pos) -> ir::Expr {
        let f = self.global("bit", Some(name), pos);
        call(f, args, pos)
    }

    /// `a // b` → `math.floor(a / b)` (spec §1.4.4).
    fn idiv(&mut self, a: ir::Expr, b: ir::Expr, pos: Pos) -> ir::Expr {
        let floor = self.global("math", Some("floor"), pos);
        call(floor, vec![binary(ir::BinOp::Div, a, b, pos)], pos)
    }

    /// A backtick string (spec §1.2.4): parts joined with `..`, every
    /// embedded value through `tostring`, at most 50 operands per group.
    fn interp(&mut self, parts: &[InterpPart], pos: Pos) -> ir::Expr {
        // Constant parts (literals, `const`s, `wowlua:meta` scalars, enum
        // members) are folded into the text: their type is known.
        let constants: Vec<Option<Vec<u8>>> = parts
            .iter()
            .map(|p| match p {
                InterpPart::Expr(e) => self.constant_text(e),
                InterpPart::Text { .. } => None,
            })
            .collect();
        let exprs: Vec<&Expr> = parts
            .iter()
            .zip(&constants)
            .filter_map(|(p, c)| match (p, c) {
                (InterpPart::Expr(e), None) => Some(e),
                _ => None,
            })
            .collect();
        let mut values = self.seq(&exprs).into_iter();
        let mut operands: Vec<ir::Expr> = Vec::new();
        let mut text: Vec<u8> = Vec::new();
        let mut text_pos = Pos::NONE;
        let flush = |text: &mut Vec<u8>, text_pos: Pos, operands: &mut Vec<ir::Expr>| {
            if !text.is_empty() {
                operands.push(lit(ir::ExprKind::String(quote(text)), text_pos));
                text.clear();
            }
        };
        for (part, constant) in parts.iter().zip(constants) {
            match (part, constant) {
                (InterpPart::Text { value, span, .. }, _) => {
                    if text.is_empty() {
                        text_pos = self.span_pos(*span);
                    }
                    text.extend_from_slice(value);
                }
                (InterpPart::Expr(e), Some(value)) => {
                    if text.is_empty() {
                        text_pos = self.span_pos(e.span);
                    }
                    text.extend_from_slice(&value);
                }
                (InterpPart::Expr(e), None) => {
                    flush(&mut text, text_pos, &mut operands);
                    let v = values.next().expect("a value per part");
                    let tostring = self.global("tostring", None, self.span_pos(e.span));
                    operands.push(call(tostring, vec![v], self.span_pos(e.span)));
                }
            }
        }
        flush(&mut text, text_pos, &mut operands);
        if operands.is_empty() {
            return lit(ir::ExprKind::String(b"\"\"".to_vec()), pos);
        }
        let concat = |items: Vec<ir::Expr>| -> ir::Expr {
            let mut iter = items.into_iter().rev();
            let mut acc = iter.next().expect("at least one");
            for item in iter {
                acc = binary(ir::BinOp::Concat, item, acc, pos);
            }
            acc
        };
        if operands.len() <= 50 {
            return concat(operands);
        }
        let groups: Vec<ir::Expr> = operands
            .chunks(50)
            .map(|chunk| paren(concat(chunk.to_vec())))
            .collect();
        concat(groups)
    }

    /// The text of a constant embedded value, as `tostring` would make it.
    fn constant_text(&mut self, e: &Expr) -> Option<Vec<u8>> {
        let value = match &e.kind {
            ExprKind::Field { obj, name } if self.is_enum_object(obj) => {
                self.enum_value(obj, &name.text)?
            }
            _ => self.static_value(e)?,
        };
        Some(match value {
            Const::String(s) => s,
            Const::Number(n) if n.is_finite() => format_g14(n).into_bytes(),
            Const::Bool(b) => b.to_string().into_bytes(),
            Const::Nil => b"nil".to_vec(),
            _ => return None,
        })
    }

    /// An optional chain (spec §1.4.1), basic-mode form: the chain is
    /// evaluated into a temporary step by step, and each `?.` guards the
    /// rest with `if t ~= nil then … end`. As a statement, the final call
    /// is a call statement. Returns the temporary (expression form).
    fn optional_chain(&mut self, e: &Expr, statement: bool) -> Option<ir::Expr> {
        let pos = self.span_pos(e.span);
        let (base, links) = flatten_chain(e);
        let first_opt = links
            .iter()
            .position(|l| l.optional)
            .expect("an optional link");
        // Everything before the first `?.` evaluates normally.
        let mut value = self.expr(base);
        for link in &links[..first_opt] {
            value = self.apply_link(value, link);
        }
        let t = self.temp("t");
        let tref = || name_expr(generated(&t, pos));
        self.prelude.push(local(generated(&t, pos), value, pos));
        // Segments between optional links, innermost last.
        let mut segments: Vec<&[Link]> = Vec::new();
        let mut start = first_opt;
        for i in first_opt + 1..=links.len() {
            if i == links.len() || links[i].optional {
                segments.push(&links[start..i]);
                start = i;
            }
        }
        let mut inner: Vec<ir::Stmt> = Vec::new();
        for (index, segment) in segments.iter().enumerate().rev() {
            let last = index + 1 == segments.len();
            let (mut body, applied) = self.isolated(|l| {
                let mut v = tref();
                for link in segment.iter() {
                    v = l.apply_link(v, link);
                }
                v
            });
            if statement && last {
                body.push(stmt(ir::StmtKind::Call(applied), pos));
            } else {
                body.push(assign(tref(), applied, pos));
            }
            body.append(&mut inner);
            let cond = binary(ir::BinOp::Ne, tref(), lit(ir::ExprKind::Nil, pos), pos);
            inner = vec![if_stmt(cond, body, pos)];
        }
        self.prelude.extend(inner);
        (!statement).then(tref)
    }

    fn apply_link(&mut self, obj: ir::Expr, link: &Link) -> ir::Expr {
        let pos = obj.pos;
        let kind = match &link.kind {
            LinkKind::Field(name) => ir::ExprKind::Field {
                obj: Box::new(obj),
                name: self.ident(name),
            },
            LinkKind::Index(key) => ir::ExprKind::Index {
                obj: Box::new(obj),
                key: Box::new(self.expr(key)),
            },
            LinkKind::Call(args) => ir::ExprKind::Call {
                callee: Box::new(obj),
                args: self.args_only(args),
            },
            LinkKind::Method(name, args) => ir::ExprKind::Method {
                obj: Box::new(obj),
                name: self.ident(name),
                args: self.args_only(args),
            },
        };
        ir::Expr::new(kind, pos)
    }

    fn args_only(&mut self, args: &Args) -> ir::Args {
        match args {
            Args::Parens { args, span } => ir::Args::Parens {
                args: self.exprs(args),
                close: self.close_pos(*span),
            },
            Args::Table(t) => ir::Args::Table(Box::new(self.expr(t))),
            Args::String(s) => ir::Args::String(Box::new(self.expr(s))),
        }
    }

    // ---------------------------------------------------------- constants --

    /// The value of a constant expression (spec §1.3.2), if it is one:
    /// literals, folded constants and scalar `wowlua:meta` values.
    fn const_value(&self, e: &Expr) -> Option<Const> {
        self.static_value(e)
    }

    fn static_value(&self, e: &Expr) -> Option<Const> {
        self.static_value_with(e, true)
    }

    /// A static condition that simplifies `if` chains and `and`/`or`: it
    /// must read a `wowlua:meta` value or a `const` (decision T71), so plain
    /// Lua such as `if true then` is emitted as written.
    fn static_condition(&self, e: &Expr) -> Option<Const> {
        let found = std::cell::Cell::new(false);
        let value = constant::eval_with(e, &|leaf| {
            let v = self.leaf_value(leaf, true);
            found.set(found.get() || v.is_some());
            v
        })?;
        found.get().then_some(value)
    }

    /// A static condition (spec §6.4.1); `consts` allows folded constants
    /// (not in `@if` on imports).
    fn static_value_with(&self, e: &Expr, consts: bool) -> Option<Const> {
        constant::eval_with(e, &|leaf| self.leaf_value(leaf, consts))
    }

    /// The value of a `wowlua:meta` path or (with `consts`) a `const`.
    fn leaf_value(&self, leaf: &Expr, consts: bool) -> Option<Const> {
        if let Some((kind, path)) = self.meta_path(leaf) {
            let meta = self.meta()?;
            return match meta.lookup(kind, &path) {
                Lookup::Const(c) => Some(c),
                _ => None,
            };
        }
        if !consts {
            return None;
        }
        match &leaf.kind {
            ExprKind::Name(_) if !self.scopes.seen().consts => None,
            ExprKind::Name(n) => match self.lookup(&n.text) {
                Some(Binding::Const(c)) => Some(c.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// A constant access path into `wowlua:meta` (spec §6.2).
    fn meta_path(&self, e: &Expr) -> Option<(MetaKind, Vec<String>)> {
        if !self.scopes.seen().meta {
            // No name can resolve to a `wowlua:meta` binding.
            return None;
        }
        let mut namespace = false;
        let found = wowlua_sema::meta::access_path(e, &|name| match self.lookup(name) {
            Some(Binding::Meta(k)) => Some(*k),
            // `M.build…`: the first path element names the export; it is
            // resolved below.
            Some(Binding::MetaNamespace) => Some(MetaKind::Addon),
            _ => None,
        })?;
        let mut base = e;
        while let ExprKind::Field { obj, .. } | ExprKind::Index { obj, .. } | ExprKind::Paren(obj) =
            &base.kind
        {
            base = obj;
        }
        if let ExprKind::Name(n) = &base.kind
            && matches!(self.lookup(&n.text), Some(Binding::MetaNamespace))
        {
            namespace = true;
        }
        if namespace {
            let (_, path) = found;
            let kind = MetaKind::from_export(path.first()?)?;
            return Some((kind, path[1..].to_vec()));
        }
        Some(found)
    }

    /// A `wowlua:meta` access: a literal for scalars, the runtime tables
    /// otherwise (spec §6.2).
    fn meta_expr(&mut self, kind: MetaKind, path: Vec<String>, e: &Expr) -> ir::Expr {
        let pos = self.span_pos(e.span);
        let Some(meta) = self.meta().cloned() else {
            self.error(
                "E1015",
                e.span,
                "`wowlua:meta` needs the package context (build the package with `wowlua build`)",
            );
            return lit(ir::ExprKind::Nil, pos);
        };
        if kind == MetaKind::Asset {
            self.error(
                "E2003",
                e.span,
                "`asset` is a function: call it with an asset path",
            );
            return lit(ir::ExprKind::Nil, pos);
        }
        match meta.lookup(kind, &path) {
            Lookup::Const(c) => c.to_expr(pos),
            Lookup::Missing(message) => {
                self.error("E2003", e.span, message);
                lit(ir::ExprKind::Nil, pos)
            }
            Lookup::Runtime => {
                self.meta_used.insert((kind, path.clone()));
                let wl = self.runtime_ref(pos);
                let mut value = field(field(wl, "meta", pos), kind.name(), pos);
                for key in &path {
                    let is_name = key
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && wowlua_syntax::token::TokenKind::keyword(key.as_bytes()).is_none();
                    value = if is_name {
                        field(value, key, pos)
                    } else if let Ok(n) = key.parse::<f64>() {
                        ir::Expr::new(
                            ir::ExprKind::Index {
                                obj: Box::new(value),
                                key: Box::new(constant::number_expr(n, pos)),
                            },
                            pos,
                        )
                    } else {
                        ir::Expr::new(
                            ir::ExprKind::Index {
                                obj: Box::new(value),
                                key: Box::new(lit(
                                    ir::ExprKind::String(quote(key.as_bytes())),
                                    pos,
                                )),
                            },
                            pos,
                        )
                    };
                }
                value
            }
        }
    }

    /// `asset("path")` (spec §6.2.1): the full in-game path of an asset.
    fn asset(&mut self, args: &Args, e: &Expr) -> ir::Expr {
        let pos = self.span_pos(e.span);
        let arg = match args {
            Args::Parens { args, .. } if args.len() == 1 => Some(&args[0]),
            Args::String(s) => Some(s.as_ref()),
            _ => None,
        };
        let path = match arg.and_then(|a| self.static_value(a)) {
            Some(Const::String(p)) => String::from_utf8_lossy(&p).into_owned(),
            _ => {
                self.error("E1038", e.span, "`asset()` takes a constant path string");
                return lit(ir::ExprKind::Nil, pos);
            }
        };
        let Some(meta) = self.meta().cloned() else {
            self.error("E1015", e.span, "`wowlua:meta` needs the package context");
            return lit(ir::ExprKind::Nil, pos);
        };
        if let Some(full) = meta.assets.get(&path) {
            return lit(ir::ExprKind::String(quote(full.as_bytes())), pos);
        }
        let lower = path.to_ascii_lowercase();
        let message = match meta.assets.keys().find(|k| k.to_ascii_lowercase() == lower) {
            Some(actual) => format!("`{path}` differs in case from the asset `{actual}`"),
            None => format!("`{path}` is not an asset of the package (`build.assets`)"),
        };
        self.error("E1038", e.span, message);
        lit(ir::ExprKind::Nil, pos)
    }

    /// `wowlua:meta` values are read-only (E2010).
    fn check_meta_target(&mut self, target: &Expr) {
        if matches!(target.kind, ExprKind::Field { .. } | ExprKind::Index { .. })
            && let Some((kind, path)) = self.meta_path(target)
        {
            let shown: Vec<&str> = std::iter::once(kind.name())
                .chain(path.iter().map(String::as_str))
                .collect();
            self.error(
                "E2010",
                target.span,
                format!("`{}` is a read-only `wowlua:meta` value", shown.join(".")),
            );
        }
    }
}

/// The names a declaration statement binds (for `@if`).
fn declared_names(s: &Stmt) -> Vec<String> {
    match &s.kind {
        StmtKind::Local { names, .. } => names.iter().map(|n| n.name.text.to_string()).collect(),
        StmtKind::LocalDestructure { fields, .. } => {
            fields.iter().map(|f| f.name.text.to_string()).collect()
        }
        StmtKind::LocalFunction { name, .. } | StmtKind::Const { name, .. } => {
            vec![name.text.to_string()]
        }
        StmtKind::Function { name, .. } if name.path.len() == 1 && name.method.is_none() => {
            vec![name.path[0].text.to_string()]
        }
        StmtKind::Import(import) => match &import.clause {
            ImportClause::Names(specs) => specs
                .iter()
                .map(|s| s.alias.as_ref().unwrap_or(&s.name).text.to_string())
                .collect(),
            ImportClause::Namespace(n) => vec![n.text.to_string()],
            ImportClause::SideEffect => Vec::new(),
        },
        StmtKind::Export(inner) => declared_names(inner),
        _ => Vec::new(),
    }
}

fn paren(inner: ir::Expr) -> ir::Expr {
    let pos = inner.pos;
    ir::Expr::new(
        ir::ExprKind::Paren {
            inner: Box::new(inner),
            close: Pos::NONE,
        },
        pos,
    )
}

fn is_name(e: &ir::Expr) -> bool {
    matches!(e.kind, ir::ExprKind::Name(_))
}

/// Safe to evaluate twice: a name or a literal.
fn is_simple(e: &ir::Expr) -> bool {
    matches!(
        e.kind,
        ir::ExprKind::Name(_)
            | ir::ExprKind::Number(_)
            | ir::ExprKind::String(_)
            | ir::ExprKind::True
            | ir::ExprKind::False
            | ir::ExprKind::Nil
    )
}

fn lua_binop(op: BinOp) -> ir::BinOp {
    match op {
        BinOp::Add => ir::BinOp::Add,
        BinOp::Sub => ir::BinOp::Sub,
        BinOp::Mul => ir::BinOp::Mul,
        BinOp::Div => ir::BinOp::Div,
        BinOp::Mod => ir::BinOp::Mod,
        BinOp::Pow => ir::BinOp::Pow,
        BinOp::Concat => ir::BinOp::Concat,
        BinOp::Eq => ir::BinOp::Eq,
        BinOp::Ne => ir::BinOp::Ne,
        BinOp::Lt => ir::BinOp::Lt,
        BinOp::Le => ir::BinOp::Le,
        BinOp::Gt => ir::BinOp::Gt,
        BinOp::Ge => ir::BinOp::Ge,
        BinOp::And => ir::BinOp::And,
        _ => ir::BinOp::Or,
    }
}

/// Whether the body of the loop being lowered uses `continue` and `break`
/// for this loop (nested loops and functions have their own).
fn loop_flags(stmts: &[Stmt]) -> (bool, bool) {
    let mut cont = false;
    let mut brk = false;
    fn walk(stmts: &[Stmt], cont: &mut bool, brk: &mut bool) {
        for s in stmts {
            match &s.kind {
                StmtKind::Continue => *cont = true,
                StmtKind::Break => *brk = true,
                StmtKind::Do(b) => walk(&b.stmts, cont, brk),
                StmtKind::If {
                    then_block,
                    elseifs,
                    else_block,
                    ..
                } => {
                    walk(&then_block.stmts, cont, brk);
                    for (_, b) in elseifs {
                        walk(&b.stmts, cont, brk);
                    }
                    if let Some(b) = else_block {
                        walk(&b.stmts, cont, brk);
                    }
                }
                _ => {}
            }
        }
    }
    walk(stmts, &mut cont, &mut brk);
    (cont, brk)
}

// ------------------------------------------------------- optional chains --

struct Link<'a> {
    optional: bool,
    kind: LinkKind<'a>,
}

enum LinkKind<'a> {
    Field(&'a Name),
    Index(&'a Expr),
    Call(&'a Args),
    Method(&'a Name, &'a Args),
}

/// `e` is a chain (field, index, call or method access) that contains a
/// `?.` link (spec §1.4.1: short-circuiting covers the rest of the chain).
fn chain_has_optional(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::OptField { .. }
        | ExprKind::OptIndex { .. }
        | ExprKind::OptMethod { .. }
        | ExprKind::OptCall { .. } => true,
        ExprKind::Field { obj, .. }
        | ExprKind::Index { obj, .. }
        | ExprKind::Method { obj, .. } => chain_has_optional(obj),
        ExprKind::Call { callee, .. } => chain_has_optional(callee),
        _ => false,
    }
}

/// Splits a chain into its base expression and links, outermost last.
fn flatten_chain(e: &Expr) -> (&Expr, Vec<Link<'_>>) {
    let mut links = Vec::new();
    let mut current = e;
    loop {
        let (next, link) = match &current.kind {
            ExprKind::Field { obj, name } => (
                obj,
                Link {
                    optional: false,
                    kind: LinkKind::Field(name),
                },
            ),
            ExprKind::OptField { obj, name } => (
                obj,
                Link {
                    optional: true,
                    kind: LinkKind::Field(name),
                },
            ),
            ExprKind::Index { obj, key } => (
                obj,
                Link {
                    optional: false,
                    kind: LinkKind::Index(key),
                },
            ),
            ExprKind::OptIndex { obj, key } => (
                obj,
                Link {
                    optional: true,
                    kind: LinkKind::Index(key),
                },
            ),
            ExprKind::Call { callee, args } => (
                callee,
                Link {
                    optional: false,
                    kind: LinkKind::Call(args),
                },
            ),
            ExprKind::OptCall { callee, args } => (
                callee,
                Link {
                    optional: true,
                    kind: LinkKind::Call(args),
                },
            ),
            ExprKind::Method { obj, name, args } => (
                obj,
                Link {
                    optional: false,
                    kind: LinkKind::Method(name, args),
                },
            ),
            ExprKind::OptMethod { obj, name, args } => (
                obj,
                Link {
                    optional: true,
                    kind: LinkKind::Method(name, args),
                },
            ),
            _ => break,
        };
        links.push(link);
        current = next;
    }
    links.reverse();
    (current, links)
}

#[cfg(test)]
mod tests {
    use super::partition_point_near;

    #[test]
    fn partition_point_near_finds_the_point_from_every_hint() {
        let long: Vec<u32> = (0..200).map(|i| i / 3 * 2).collect();
        let slices: [&[u32]; 6] = [
            &[],
            &[0],
            &[4],
            &[0, 1, 2],
            &[0, 2, 2, 2, 5, 7, 7, 9, 12, 12, 12, 12, 20],
            &long,
        ];
        for slice in slices {
            let max = slice.last().copied().unwrap_or(0);
            for key in 0..=max + 2 {
                let below = slice.partition_point(|&s| s < key);
                let at_or_below = slice.partition_point(|&s| s <= key);
                for hint in (0..=slice.len() + 1).chain([usize::MAX]) {
                    assert_eq!(
                        partition_point_near(slice, hint, |&s| s < key),
                        below,
                        "{slice:?} < {key} from {hint}"
                    );
                    assert_eq!(
                        partition_point_near(slice, hint, |&s| s <= key),
                        at_or_below,
                        "{slice:?} <= {key} from {hint}"
                    );
                }
            }
        }
    }
}
