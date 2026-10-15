//! Checking one file (spec chapter 2): scopes and locals, statements,
//! function bodies with their returns, and class bodies.

use super::env::{DefKind, Env, Scope};
use super::narrow::Flow;
use super::query::Mode;
use super::types::Field;
use super::types::*;
use rustc_hash::{FxHashMap, FxHashSet};
use wowlua_diagnostics::Diagnostic;
use wowlua_span::Span;
use wowlua_syntax::ast::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarId(pub u32);

#[derive(Clone, Debug)]
pub struct Var {
    pub name: Box<str>,
    /// The annotated type, if any.
    pub declared: Option<TypeId>,
    /// The type when declared (widened), the base for unannotated locals.
    pub base: TypeId,
    /// The function depth of the declaration.
    pub depth: u32,
    pub is_const: bool,
    /// Assigned after its declaration from inside a nested function.
    pub reassigned_in_closure: bool,
    /// Assigned from inside a nested function: a call may change it.
    pub assigned_in_closure: bool,
    /// Never assigned after its declaration: closures see its narrowing.
    pub never_reassigned: bool,
}

/// The function being checked.
struct FnCtx {
    /// The annotated results, if any.
    expected: Option<Pack>,
    /// The results of every `return`.
    returns: Vec<Pack>,
    /// The type of `...` (if the function is variadic).
    vararg: Option<TypeId>,
}

/// What the checker needs from the package.
pub struct FileInfo<'a> {
    pub index: usize,
    pub mode: Mode,
    /// A pure Lua file not opted into strict mode: diagnostics of the name
    /// and type ranges are warnings (spec §2.1).
    pub pure_lua: bool,
    /// Extra allowed globals (`check.globals`, SavedVariables, `@global`).
    pub extra_globals: &'a FxHashSet<String>,
    /// Globals the package assigns: reading them is fine (the assignments
    /// themselves are W1002).
    pub assigned_globals: &'a FxHashSet<String>,
    /// Imported bindings: local name → type.
    pub imports: FxHashMap<Box<str>, TypeId>,
    /// The file is a module (top-level `...` is not the addon arguments
    /// for imports; it still is in WoW).
    pub is_module: bool,
    /// Shared flavor mode: the flavors and API availability.
    pub shared: Option<&'a super::flavors::SharedFlavors>,
    /// Names bound to `wowlua:meta`'s `flavor`.
    pub meta_flavor: FxHashSet<String>,
    /// Imports bound to the standard library's `@intrinsic` ids.
    pub intrinsics: crate::stdlib::Intrinsics,
}

pub struct Checker<'e> {
    pub env: &'e mut Env,
    pub mode: Mode,
    pub scope: Scope,
    pub(crate) vars: Vec<Var>,
    scopes: Vec<FxHashMap<Box<str>, VarId>>,
    pub(crate) flow: Flow,
    /// Fields of locals that conditions narrowed (`a.b`, `a.b.c`, spec
    /// §2.5): each path is a synthetic variable, so flows narrow and join
    /// it like a local. Keyed by the root local and the dotted path.
    pub(crate) field_vars: FxHashMap<(VarId, Box<str>), VarId>,
    /// The current path can still run.
    pub(crate) live: bool,
    fns: Vec<FnCtx>,
    pub depth: u32,
    pub diagnostics: Vec<Diagnostic>,
    pub extra_globals: FxHashSet<String>,
    pub assigned_globals: FxHashSet<String>,
    /// Types of expressions already checked, by span.
    pub(crate) expr_types: FxHashMap<(u32, u32), TypeId>,
    /// Links of optional chains: their type before the short-circuit's
    /// `nil`, which the rest of the chain continues from.
    pub(crate) chain_inner: FxHashMap<(u32, u32), TypeId>,
    /// Classes whose bodies are being checked: (def, static member).
    pub(crate) classes: Vec<(DefId, bool)>,
    /// Exported values of this module.
    pub exports: FxHashMap<Box<str>, TypeId>,
    /// Name spans of accesses to renamable methods (`rename-private`), with
    /// the class that declares the method.
    pub private_methods: FxHashMap<wowlua_span::Span, DefId>,
    /// Classes whose instances or class tables this file stores in globals.
    pub global_classes: FxHashSet<DefId>,
    reassigned: FxHashSet<u32>,
    assigned_in_closures: FxHashSet<u32>,
    /// Locals assigned after their declaration (anywhere).
    assigned_later: FxHashSet<u32>,
    pure_lua: bool,
    /// `...` at the top level: (addon name, addon table).
    pub(crate) top_vararg: Option<(TypeId, TypeId)>,
    /// The file calls `setfenv`: global names resolve in environments the
    /// checker cannot see (no W1001/W1002).
    pub(crate) dynamic_env: bool,
    pub(crate) shared: Option<super::flavors::SharedFlavors>,
    pub(crate) guard: super::flavors::Guard,
    pub(crate) meta_flavor: FxHashSet<String>,
    /// Expressions that are existence tests (`if C_X then`): no E2030.
    pub(crate) probes: FxHashSet<wowlua_span::Span>,
    /// Inside code already reported as unreachable (W1005).
    in_dead: bool,
    is_module: bool,
    /// Function expressions passed as call arguments (callbacks: no E2050).
    pub(crate) callbacks: FxHashSet<wowlua_span::Span>,
    /// The next `function_body` checks a callback.
    pub(crate) callback_next: bool,
    /// Runtime checks of annotated parameters, `...: T` and `::`, by span.
    pub(crate) value_checks: FxHashMap<wowlua_span::Span, super::runtime::ValueCheck>,
    /// Imports bound to the standard library's `@intrinsic` ids, and the
    /// variables of those imports (a shadowing local is not the import).
    intrinsics: crate::stdlib::Intrinsics,
    import_vars: FxHashMap<Box<str>, VarId>,
    /// The handler type the decorators of the statement being checked give
    /// its function (`@event("X")`): (type, what it handles).
    next_handler: Option<(Func, String)>,
}

impl<'e> Checker<'e> {
    pub fn new(env: &'e mut Env, info: &FileInfo, parse: &wowlua_syntax::Parse) -> Checker<'e> {
        let mut c = Checker {
            env,
            mode: info.mode,
            scope: Scope {
                module: Some(info.index),
                ..Scope::default()
            },
            vars: Vec::new(),
            scopes: vec![FxHashMap::default()],
            flow: Flow::default(),
            field_vars: FxHashMap::default(),
            live: true,
            fns: Vec::new(),
            depth: 0,
            diagnostics: Vec::new(),
            extra_globals: info.extra_globals.clone(),
            assigned_globals: info.assigned_globals.clone(),
            expr_types: FxHashMap::default(),
            chain_inner: FxHashMap::default(),
            classes: Vec::new(),
            exports: FxHashMap::default(),
            private_methods: FxHashMap::default(),
            global_classes: FxHashSet::default(),
            reassigned: reassigned_in_closures(&parse.chunk.block).0,
            assigned_in_closures: reassigned_in_closures(&parse.chunk.block).1,
            assigned_later: reassigned_in_closures(&parse.chunk.block).2,
            pure_lua: info.pure_lua,
            top_vararg: None,
            dynamic_env: calls_setfenv(&parse.chunk.block),
            shared: info.shared.cloned(),
            guard: super::flavors::Guard::default(),
            meta_flavor: info.meta_flavor.clone(),
            probes: FxHashSet::default(),
            in_dead: false,
            is_module: info.is_module,
            callbacks: FxHashSet::default(),
            callback_next: false,
            value_checks: FxHashMap::default(),
            intrinsics: info.intrinsics.clone(),
            import_vars: FxHashMap::default(),
            next_handler: None,
        };
        for (name, &t) in &info.imports {
            let id = c.declare(name, Some(t), t, true, None);
            c.import_vars.insert(name.clone(), id);
        }
        c
    }

    // ------------------------------------------------------------ errors --

    /// Reports a diagnostic of the name/type ranges, as a warning in pure
    /// Lua files (spec §2.1).
    pub fn report(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        let (code, warn) = if self.pure_lua && (code.starts_with("E1") || code.starts_with("E2")) {
            (warning_code(code), true)
        } else {
            (code, code.starts_with('W'))
        };
        let d = if warn {
            Diagnostic::warning(code, message)
        } else {
            Diagnostic::error(code, message)
        };
        self.diagnostics.push(d.with_label(span, "here"));
    }

    /// A type mismatch (E2001).
    pub fn mismatch(&mut self, span: Span, s: TypeId, t: TypeId) {
        let msg = format!(
            "`{}` is not assignable to `{}`",
            self.env.display(s),
            self.env.display(t)
        );
        self.report("E2001", span, msg);
    }

    pub fn check_assignable(&mut self, span: Span, s: TypeId, t: TypeId) {
        if !self.env.assignable(s, t, self.mode) {
            self.mismatch(span, s, t);
        }
    }

    // ------------------------------------------------------------ scopes --

    pub fn push_scope(&mut self) {
        self.scopes.push(FxHashMap::default());
    }

    pub fn pop_scope(&mut self) {
        if let Some(scope) = self.scopes.pop() {
            for v in scope.values() {
                self.flow.remove(v);
            }
        }
    }

    pub fn declare(
        &mut self,
        name: &str,
        declared: Option<TypeId>,
        ty: TypeId,
        is_const: bool,
        span: Option<Span>,
    ) -> VarId {
        self.declare_with(name, declared, ty, is_const, span, true)
    }

    /// `widen`: an unannotated variable holds its value's literals widened
    /// (`local x = "a"` is a `string`); a generic `for`'s variables take
    /// the iterator's result types as they are (spec §2.4.4).
    fn declare_with(
        &mut self,
        name: &str,
        declared: Option<TypeId>,
        ty: TypeId,
        is_const: bool,
        span: Option<Span>,
        widen: bool,
    ) -> VarId {
        let id = VarId(self.vars.len() as u32);
        let base = match declared {
            Some(d) => d,
            None if widen => self.env.types.widen(ty),
            None => ty,
        };
        self.vars.push(Var {
            name: name.into(),
            declared,
            base,
            depth: self.depth,
            is_const,
            reassigned_in_closure: span.is_some_and(|s| self.reassigned.contains(&s.start)),
            assigned_in_closure: span.is_some_and(|s| self.assigned_in_closures.contains(&s.start)),
            never_reassigned: span.is_some_and(|s| !self.assigned_later.contains(&s.start)),
        });
        self.scopes
            .last_mut()
            .expect("scope")
            .insert(name.into(), id);
        if declared.is_none() {
            // Unannotated locals hold widened literals (spec §2.4).
            let t = if is_const || !widen {
                ty
            } else {
                self.env.types.widen(ty)
            };
            self.flow.insert(id, t);
        }
        id
    }

    pub fn lookup_var(&self, name: &str) -> Option<VarId> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    /// An annotation's type; `typeof(expr)` parts are inferred first
    /// (spec §2.2.7).
    pub fn annotation(&mut self, t: &Type) -> TypeId {
        let mut exprs: Vec<&Expr> = Vec::new();
        typeof_exprs(t, &mut exprs);
        for e in exprs {
            let ty = self.infer(e);
            let ty = self.env.types.widen(ty);
            self.env.typeof_types.insert((e.span.start, e.span.end), ty);
        }
        let scope = self.scope.clone();
        self.env.resolve(t, &scope)
    }

    /// The type of `...` in the current function.
    pub fn current_vararg(&self) -> Option<TypeId> {
        self.fns.last().and_then(|f| f.vararg)
    }

    /// The declared or base type of a local.
    pub fn var_base(&self, v: VarId) -> TypeId {
        self.vars[v.0 as usize].base
    }

    /// The type of a local in a flow.
    pub fn var_type_in(&self, v: VarId, flow: &Flow) -> TypeId {
        let var = &self.vars[v.0 as usize];
        if var.depth < self.depth && var.declared.is_none() && var.reassigned_in_closure {
            return Types::ANY;
        }
        flow.get(&v).copied().unwrap_or(var.base)
    }

    pub fn var_type(&self, v: VarId) -> TypeId {
        self.var_type_in(v, &self.flow)
    }

    /// Assigns a value to a local: checks it against the annotation and
    /// updates the narrowed type.
    pub fn assign_var(&mut self, v: VarId, value: TypeId, span: Span) {
        self.forget_fields(v, None);
        let var = self.vars[v.0 as usize].clone();
        match var.declared {
            Some(d) => {
                self.check_assignable(span, value, d);
                // A declared `any` opts out, as in `local` (above).
                let narrowed = if d != Types::ANY
                    && self.env.assignable(value, d, self.mode)
                    && value != Types::ANY
                {
                    self.narrow_to_assigned(d, value)
                } else {
                    d
                };
                self.flow.insert(v, narrowed);
            }
            None => {
                let widened = self.env.types.widen(value);
                self.flow.insert(v, widened);
                // The base widens to cover every assigned value.
                let base = self.env.types.union(&[var.base, widened]);
                self.vars[v.0 as usize].base = base;
            }
        }
    }

    /// The declared type `d` after assigning `value`: the members of `d`
    /// the value's members fit (a literal stays a literal where `d` is a
    /// literal union; a record or interface stays as declared).
    fn narrow_to_assigned(&mut self, d: TypeId, value: TypeId) -> TypeId {
        let declared = self.env.types.members(d);
        let mut out = Vec::new();
        for vm in self.env.types.members(value) {
            let literal = matches!(
                self.env.types.get(vm),
                Ty::Nil | Ty::Bool(_) | Ty::Str(_) | Ty::Num(_)
            );
            if literal
                && declared
                    .iter()
                    .any(|&m| self.env.types.widen(m) == self.env.types.widen(vm))
            {
                out.push(vm);
                continue;
            }
            let fits: Vec<TypeId> = declared
                .iter()
                .copied()
                .filter(|&m| self.env.assignable(vm, m, self.mode))
                .collect();
            if fits.is_empty() {
                return d;
            }
            out.extend(fits);
        }
        self.env.types.union(&out)
    }

    // ------------------------------------------------------------ files --

    /// Checks a file's top level.
    pub fn check_file(&mut self, parse: &wowlua_syntax::Parse) {
        // `...` at the top level: (addon name, addon table) (spec §2.4.7).
        let ns = self
            .env
            .global_types
            .get(super::env::ADDON_NAMESPACE)
            .copied();
        let ns_ty = match ns {
            Some(d) => self.env.types.intern(Ty::Named(d, Vec::new())),
            None => Types::TABLE,
        };
        let top = FnCtx {
            expected: None,
            returns: Vec::new(),
            vararg: None,
        };
        self.fns.push(top);
        self.top_vararg = Some((Types::STRING, ns_ty));
        self.hoist(&parse.chunk.block);
        self.block_stmts(&parse.chunk.block);
        self.fns.pop();
    }

    /// Top-level classes and enums are hoisted (decision T76), and so are
    /// the top-level functions of a module (spec §5.1).
    fn hoist(&mut self, b: &Block) {
        for s in &b.stmts {
            let kind = match &s.kind {
                StmtKind::Export(inner) => &inner.kind,
                k => k,
            };
            match kind {
                StmtKind::Function { name, func }
                    if self.is_module && name.path.len() == 1 && name.method.is_none() =>
                {
                    let sig = self.env.func_sig(func, &self.scope.clone(), false);
                    let t = self.env.types.func(sig);
                    self.declare(&name.path[0].text, Some(t), t, false, None);
                }
                StmtKind::Class(c) => {
                    if let Some(def) = self.env.lookup_type(&c.name.text, &self.scope) {
                        let t = self.env.types.intern(Ty::ClassOf(def));
                        self.declare(&c.name.text, Some(t), t, true, None);
                    }
                }
                StmtKind::Enum(e) => {
                    if let Some(def) = self.env.lookup_type(&e.name.text, &self.scope) {
                        let t = self.env.types.intern(Ty::ClassOf(def));
                        self.declare(&e.name.text, Some(t), t, true, None);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn block(&mut self, b: &Block) {
        self.push_scope();
        self.block_stmts(b);
        self.pop_scope();
    }

    fn block_stmts(&mut self, b: &Block) {
        let outer = self.in_dead;
        for s in &b.stmts {
            // The first statement after a `return`, `break`, `continue` or
            // a call that never returns (W1005), once per region.
            if !self.live && !self.in_dead {
                self.report("W1005", s.span, "unreachable code");
                self.in_dead = true;
            }
            self.stmt(s);
        }
        self.in_dead = outer;
    }

    fn stmt(&mut self, s: &Stmt) {
        self.next_handler = self.decorated_handler(&s.decorators);
        stacker::maybe_grow(64 * 1024, 1024 * 1024, || self.stmt_inner(s));
        self.next_handler = None;
    }

    /// The standard-library `@intrinsic` id a callee (`on`, `events.on`)
    /// names through an import that no local shadows.
    /// A member of an intrinsic is named by the intrinsic's id and the
    /// member (`FramePool.new` → `pool.FramePool.new`).
    pub(crate) fn intrinsic_of(&self, callee: &Expr) -> Option<String> {
        match &callee.kind {
            ExprKind::Name(n) => self.intrinsic_path(&n.text, None),
            ExprKind::Field { obj, name } => {
                if let ExprKind::Name(m) = &obj.kind
                    && let Some(id) = self.intrinsic_path(&m.text, Some(&name.text))
                {
                    return Some(id);
                }
                self.intrinsic_of(obj)
                    .map(|id| format!("{id}.{}", name.text))
            }
            _ => None,
        }
    }

    fn intrinsic_path(&self, local: &str, member: Option<&str>) -> Option<String> {
        let id = self.intrinsics.get(local, member)?;
        match (self.lookup_var(local), self.import_vars.get(local)) {
            (Some(v), Some(&import)) if v != import => None,
            (Some(_), None) => None,
            _ => Some(id.to_string()),
        }
    }

    /// The handler type an `@event` or `@unitEvent` decorator gives the
    /// function it decorates: the event's (or message's) parameters, or with
    /// several names the name first, then anything.
    fn decorated_handler(&mut self, decorators: &[Decorator]) -> Option<(Func, String)> {
        for d in decorators {
            let id = match d.path.as_slice() {
                [n] => self.intrinsic_path(&n.text, None),
                [m, n] => self.intrinsic_path(&m.text, Some(&n.text)),
                _ => None,
            };
            let Some(id) = id else { continue };
            let args = d.args.as_ref().map(|a| a.exprs()).unwrap_or_default();
            let names: Vec<String> = args.iter().map_while(|e| literal_string(e)).collect();
            let (skip, event_names) = match id.as_str() {
                "events.event" => (0, args.len()),
                "events.unitEvent" => (0, 1),
                "events.bucket" => (1, args.len()),
                _ => (0, 0),
            };
            for e in args.iter().skip(skip).take(event_names) {
                if let Some(n) = literal_string(e) {
                    self.check_event_available(&n, e.span);
                }
            }
            if id.starts_with("hooks.") {
                self.check_hook_target(&id, &args, true);
            }
            match id.as_str() {
                "events.event" if names.len() == 1 => {
                    let f = self.env.handlers.get(names[0].as_str())?.clone();
                    return Some((f, format!("`{}`", names[0])));
                }
                "events.event" if names.len() > 1 && names.len() == args.len() => {
                    let types: Vec<TypeId> = names.iter().map(|n| self.env.types.str(n)).collect();
                    let ty = self.env.types.union(&types);
                    let f = Func {
                        type_params: Vec::new(),
                        params: vec![super::types::Param {
                            name: Some("event".into()),
                            ty,
                            optional: false,
                        }],
                        rest: Some(Rest::Variadic(Types::ANY)),
                        ret: Pack::any(),
                        predicate: None,
                        unannotated: false,
                    };
                    return Some((f, format!("`{}`", names.join("`, `"))));
                }
                "events.unitEvent" if !names.is_empty() => {
                    let f = self.env.handlers.get(names[0].as_str())?.clone();
                    return Some((f, format!("`{}`", names[0])));
                }
                _ => {}
            }
        }
        None
    }

    /// Checks a decorated handler's type against the decorator's.
    fn check_handler(&mut self, actual: TypeId, handler: &(Func, String), span: Span) {
        let expected = self.env.types.func(handler.0.clone());
        if !self.env.assignable(actual, expected, self.mode) {
            let msg = format!(
                "`{}` cannot handle {}: it is called with `{}`",
                self.env.display(actual),
                handler.1,
                self.env.display(expected)
            );
            self.report("E2001", span, msg);
        }
    }

    fn stmt_inner(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Local { names, values } => {
                let expected: Vec<Option<TypeId>> = names
                    .iter()
                    .map(|n| n.ty.as_ref().map(|t| self.annotation(t)))
                    .collect();
                // Fresh literals widen (`local s = "a"` is a string); values
                // typed elsewhere (a call's declared result, a variable,
                // a field) keep their type, unions of literals included.
                let keep: Vec<bool> = values
                    .iter()
                    .map(|v| {
                        matches!(
                            v.kind,
                            ExprKind::Call { .. }
                                | ExprKind::OptCall { .. }
                                | ExprKind::Method { .. }
                                | ExprKind::OptMethod { .. }
                                | ExprKind::Name(_)
                                | ExprKind::Field { .. }
                                | ExprKind::Index { .. }
                        )
                    })
                    .collect();
                let values = self.expr_list(values, &expected);
                let no_values = values.is_empty();
                for (i, n) in names.iter().enumerate() {
                    let value = values.get(i).copied().unwrap_or(Types::NIL);
                    let declared = expected[i];
                    // `local x: T` without a value: `nil` until assigned,
                    // checked where it is read.
                    if let Some(d) = declared
                        && !no_values
                    {
                        let span = n.name.span;
                        self.check_assignable(span, value, d);
                    }
                    let id = self.declare(&n.name.text, declared, value, false, Some(n.name.span));
                    if declared.is_none() && keep.get(i).copied().unwrap_or(false) {
                        self.flow.insert(id, value);
                    }
                    if declared.is_some() && no_values {
                        self.flow.insert(id, Types::NIL);
                    } else if let Some(d) = declared {
                        // A declared `any` opts out: it is not narrowed to
                        // its initializer's type.
                        let narrowed = if d != Types::ANY
                            && self.env.assignable(value, d, self.mode)
                            && value != Types::ANY
                            && value != Types::NIL
                        {
                            self.narrow_to_assigned(d, value)
                        } else {
                            d
                        };
                        self.flow.insert(id, narrowed);
                    }
                }
            }
            StmtKind::Assign { targets, values } => {
                let expected: Vec<Option<TypeId>> =
                    targets.iter().map(|t| self.target_type(t)).collect();
                let values = self.expr_list(values, &expected);
                for (i, t) in targets.iter().enumerate() {
                    let value = values.get(i).copied().unwrap_or(Types::NIL);
                    // Classes reachable from globals keep their method names.
                    if self.global_rooted(t) {
                        self.note_global_classes(value);
                    }
                    self.assign_target(t, value);
                }
            }
            StmtKind::Compound { target, op, value } => {
                let current = self.infer(target);
                let v = self.infer(value);
                let result = self.compound_result(*op, current, v, s.span);
                self.assign_target(target, result);
            }
            StmtKind::Call(e) => {
                let results = self.infer_multi(e);
                // `error(...)` and other functions returning `never`.
                if results.types.first() == Some(&Types::NEVER) {
                    self.live = false;
                }
                // `assert(cond)` narrows like `if cond` (spec §2.5).
                if let ExprKind::Call { callee, args } = &e.kind
                    && matches!(&callee.kind, ExprKind::Name(n) if n.text.as_ref() == "assert" && self.lookup_var("assert").is_none())
                    && let Some(cond) = args.exprs().first()
                {
                    let (t_flow, _) = self.narrow(cond, &self.flow.clone());
                    self.flow = t_flow;
                }
            }
            StmtKind::Do(b) => self.block(b),
            StmtKind::While { cond, body } => {
                self.mark_probes(cond);
                self.infer(cond);
                let before = self.flow.clone();
                self.forget_assigned(body);
                let (t_flow, f_flow) = self.narrow(cond, &self.flow.clone());
                self.flow = t_flow;
                self.block(body);
                let after_body = self.flow.clone();
                let joined = self.join(&f_flow, &after_body, true, self.live);
                self.flow = joined;
                let _ = before;
                self.live = true;
            }
            StmtKind::Repeat { body, cond } => {
                self.forget_assigned(body);
                self.push_scope();
                self.block_stmts(body);
                self.mark_probes(cond);
                self.infer(cond);
                let (_, f) = self.narrow(cond, &self.flow.clone());
                let _ = f;
                self.pop_scope();
                self.live = true;
            }
            StmtKind::If {
                cond,
                then_block,
                elseifs,
                else_block,
            } => {
                let mut clauses: Vec<(&Expr, &Block)> = vec![(cond, then_block)];
                clauses.extend(elseifs.iter().map(|(c, b)| (c, b)));
                let mut ends: Vec<(Flow, bool, super::flavors::Guard)> = Vec::new();
                for (c, b) in clauses {
                    self.mark_probes(c);
                    self.infer(c);
                    let (t_flow, f_flow) = self.narrow(c, &self.flow.clone());
                    let (t_guard, f_guard) = self.narrow_guard(c, &self.guard.clone());
                    let entry_live = self.live;
                    self.flow = t_flow;
                    self.guard = t_guard;
                    self.block(b);
                    ends.push((self.flow.clone(), self.live, self.guard.clone()));
                    self.flow = f_flow;
                    self.guard = f_guard;
                    self.live = entry_live;
                }
                match else_block {
                    Some(b) => {
                        self.block(b);
                        ends.push((self.flow.clone(), self.live, self.guard.clone()));
                    }
                    None => ends.push((self.flow.clone(), self.live, self.guard.clone())),
                }
                let mut iter = ends.into_iter();
                let (mut flow, mut live, first_guard) = iter.next().expect("a branch");
                let mut guard = if live { Some(first_guard) } else { None };
                for (f, l, g) in iter {
                    flow = self.join(&flow, &f, live, l);
                    live |= l;
                    if l {
                        guard = Some(match guard {
                            Some(prev) => prev.join(&g),
                            None => g,
                        });
                    }
                }
                self.flow = flow;
                self.live = live;
                if let Some(g) = guard {
                    self.guard = g;
                }
            }
            StmtKind::NumericFor {
                var,
                start,
                limit,
                step,
                body,
            } => {
                for e in [Some(start), Some(limit), step.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    let t = self.infer(e);
                    self.check_assignable(e.span, t, Types::NUMBER);
                }
                self.forget_assigned(body);
                self.push_scope();
                self.declare(
                    &var.name.text,
                    Some(Types::NUMBER),
                    Types::NUMBER,
                    false,
                    Some(var.name.span),
                );
                let entry = self.flow.clone();
                self.block(body);
                self.pop_scope();
                self.flow = self.join(&entry, &self.flow.clone(), true, self.live);
                self.live = true;
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                let types = self.expr_list(exprs, &[]);
                let loop_types =
                    self.iteration_types(&types, exprs.first().map(|e| e.span).unwrap_or(s.span));
                self.forget_assigned(body);
                self.push_scope();
                for (i, v) in vars.iter().enumerate() {
                    let declared = v.ty.as_ref().map(|t| self.annotation(t));
                    let t = loop_types.get(i).copied().unwrap_or(Types::ANY);
                    self.declare_with(&v.name.text, declared, t, false, Some(v.name.span), false);
                }
                let entry = self.flow.clone();
                self.block(body);
                self.pop_scope();
                self.flow = self.join(&entry, &self.flow.clone(), true, self.live);
                self.live = true;
            }
            StmtKind::Function { name, func } => self.function_stmt(name, func),
            StmtKind::LocalFunction { name, func } => {
                let sig = self.env.func_sig(func, &self.scope.clone(), false);
                let t = self.env.types.func(sig);
                let id = self.declare(&name.text, None, t, false, Some(name.span));
                let handler = self.next_handler.take();
                let actual = self.function_body(func, handler.as_ref().map(|h| h.0.clone()), None);
                if let Some(h) = &handler {
                    self.check_handler(actual, h, name.span);
                }
                self.flow.insert(id, actual);
                self.vars[id.0 as usize].base = actual;
            }
            StmtKind::Return(values) => {
                let expected = self.fns.last().and_then(|f| f.expected.clone());
                let expected_types: Vec<Option<TypeId>> = expected
                    .as_ref()
                    .map(|p| p.types.iter().map(|&t| Some(t)).collect())
                    .unwrap_or_default();
                let pack = self.expr_pack(values, &expected_types);
                if let Some(exp) = expected {
                    self.check_pack(values, &pack, &exp, s.span);
                }
                if let Some(f) = self.fns.last_mut() {
                    f.returns.push(pack);
                }
                self.live = false;
            }
            StmtKind::Break | StmtKind::Continue => self.live = false,
            StmtKind::LocalDestructure {
                fields, value, ty, ..
            } => {
                let declared = ty.as_ref().map(|t| self.annotation(t));
                let source = match value {
                    Some(v) => self.check(v, declared),
                    None => Types::ANY,
                };
                let source = declared.unwrap_or(source);
                for f in fields {
                    let field_ty = self
                        .member_type(source, &f.name.text, f.name.span, false)
                        .unwrap_or(Types::ANY);
                    let t = match &f.default {
                        Some(d) => {
                            let dt = self.infer(d);
                            let non_nil = self.env.types.non_nil(field_ty);
                            self.env.types.union(&[non_nil, dt])
                        }
                        None => field_ty,
                    };
                    self.declare(&f.name.text, None, t, false, Some(f.name.span));
                }
            }
            StmtKind::Const { name, ty, value } => {
                let declared = ty.as_ref().map(|t| self.annotation(t));
                let t = match value {
                    Some(v) => self.check(v, declared),
                    None => declared.unwrap_or(Types::ANY),
                };
                if let (Some(d), Some(v)) = (declared, value) {
                    self.check_assignable(v.span, t, d);
                }
                // `const` keeps literal types (spec §2.4).
                let id = self.declare(
                    &name.text,
                    Some(declared.unwrap_or(t)),
                    t,
                    true,
                    Some(name.span),
                );
                self.flow.insert(id, t);
            }
            StmtKind::Class(c) => self.class_body(c),
            StmtKind::Enum(_)
            | StmtKind::Interface(_)
            | StmtKind::TypeAlias(_)
            | StmtKind::Declare(_) => {}
            StmtKind::Import(_) => {}
            StmtKind::Export(inner) => {
                self.stmt(inner);
                for name in exported_names(inner) {
                    if let Some(v) = self.lookup_var(&name) {
                        let t = self.var_type(v);
                        self.exports.insert(name.into(), t);
                    }
                }
            }
            StmtKind::Error => {}
        }
    }

    /// The assignment target is a global or lies in one (`X`, `_G.X`,
    /// `X.a.b`, `X[k]`): what it stores is reachable from other addons.
    pub(crate) fn global_rooted(&self, target: &Expr) -> bool {
        let mut e = target;
        loop {
            match &e.kind {
                ExprKind::Name(n) => return self.lookup_var(&n.text).is_none(),
                ExprKind::Field { obj, .. } | ExprKind::Index { obj, .. } => e = obj,
                _ => return false,
            }
        }
    }

    /// Records the classes whose instances or class tables `t` holds.
    pub(crate) fn note_global_classes(&mut self, t: TypeId) {
        for m in self.env.types.members(t) {
            match self.env.types.get(m).clone() {
                Ty::Named(d, _) | Ty::ClassOf(d) if self.env.def(d).kind == DefKind::Class => {
                    self.global_classes.insert(d);
                }
                _ => {}
            }
        }
    }

    /// Locals assigned in a loop body lose their narrowing at the loop
    /// start (the body may run again).
    fn forget_assigned(&mut self, body: &Block) {
        let mut names = FxHashSet::default();
        wowlua_syntax::visit::walk_stmts(body, &mut |s| match &s.kind {
            StmtKind::Assign { targets, .. } => {
                for t in targets {
                    if let ExprKind::Name(n) = &t.kind {
                        names.insert(n.text.clone());
                    }
                }
            }
            StmtKind::Compound { target, .. } => {
                if let ExprKind::Name(n) = &target.kind {
                    names.insert(n.text.clone());
                }
            }
            _ => {}
        });
        for name in names {
            if let Some(v) = self.lookup_var(&name) {
                let var = self.vars[v.0 as usize].clone();
                let t = match var.declared {
                    Some(d) => d,
                    // Unknown future assignments.
                    None => Types::ANY,
                };
                self.flow.insert(v, t);
            }
        }
    }

    /// `function f()`, `function t.f()`, `function t:m()`.
    fn function_stmt(&mut self, name: &FuncName, func: &FuncBody) {
        let method = name.method.is_some();
        // The table the function is stored in gives `self`'s type.
        let mut owner: Option<TypeId> = None;
        if name.path.len() > 1 || method {
            // `t.a.f` is stored in `t.a`; `t.a:m` in `t.a` too.
            let parents = if method {
                &name.path[1..]
            } else {
                &name.path[1..name.path.len() - 1]
            };
            let mut t = self.name_type(&name.path[0]);
            for n in parents {
                t = self
                    .member_type(t, &n.text, n.span, false)
                    .unwrap_or(Types::ANY);
            }
            owner = Some(t);
        }
        let expected = match (&owner, name.path.len(), &name.method) {
            (Some(o), _, Some(m)) => self.member_type(*o, &m.text, m.span, true),
            (Some(o), n, None) if n > 1 => {
                self.member_type(*o, &name.path[n - 1].text, name.path[n - 1].span, true)
            }
            _ => None,
        };
        let self_ty = if method { owner } else { None };
        let handler = self.next_handler.take();
        let expected = expected
            .and_then(|e| self.as_func(e))
            .or_else(|| handler.as_ref().map(|h| h.0.clone()));
        let t = self.function_body(func, expected, self_ty);
        if let Some(h) = &handler {
            let span = name
                .method
                .as_ref()
                .unwrap_or(&name.path[name.path.len() - 1])
                .span;
            self.check_handler(t, h, span);
        }
        if name.path.len() == 1 && !method {
            let n = &name.path[0];
            match self.lookup_var(&n.text) {
                Some(v) => self.assign_var(v, t, n.span),
                None => self.assign_global(n, t),
            }
        } else if let Some(o) = owner {
            let last = name
                .method
                .as_ref()
                .unwrap_or_else(|| name.path.last().expect("a name"));
            // A method stored in a table takes `self` first.
            let stored = if method { self.env.with_self(t, o) } else { t };
            self.extend_open(o, &last.text, stored);
        }
    }

    pub fn as_func(&self, t: TypeId) -> Option<Func> {
        match self.env.types.get(t) {
            Ty::Func(f) => Some(Func::clone(f)),
            _ => None,
        }
    }

    /// Checks a function body; `expected` gives parameter and result types
    /// from the context (spec §2.4.3). Returns the function's type.
    pub fn function_body(
        &mut self,
        f: &FuncBody,
        expected: Option<Func>,
        self_ty: Option<TypeId>,
    ) -> TypeId {
        let mut sig = self.env.func_sig(f, &self.scope.clone(), false);
        // Strict mode: parameters of non-callback functions need
        // annotations (E2050).
        let callback = std::mem::take(&mut self.callback_next);
        if self.mode == Mode::Strict && expected.is_none() && !callback {
            for p in f
                .params
                .iter()
                .filter(|p| p.ty.is_none() && p.default.is_none())
            {
                let msg = format!("parameter `{}` needs a type annotation", p.name.text);
                self.report("E2050", p.name.span, msg);
            }
        }
        // Contextual parameter types for unannotated parameters.
        if let Some(exp) = &expected {
            for (i, p) in sig.params.iter_mut().enumerate() {
                if f.params[i].ty.is_none()
                    && let Some(ep) = exp.params.get(i)
                {
                    p.ty = ep.ty;
                }
            }
            if f.ret.is_none()
                && !matches!(exp.ret.rest, Some(Rest::Variadic(t)) if t == Types::ANY && exp.ret.types.is_empty())
            {
                sig.ret = exp.ret.clone();
            }
        }
        let annotated_ret = f.ret.is_some()
            || expected
                .as_ref()
                .is_some_and(|e| !(e.ret.types.is_empty() && e.ret.rest.is_some()));
        let saved_flow = std::mem::take(&mut self.flow);
        // Locals never assigned after their declaration keep their
        // narrowing inside the closure (it cannot change); open tables are
        // sealed for closures (spec §2.4.2), so not theirs.
        let vars = &self.vars;
        let types = &self.env.types;
        self.flow = saved_flow
            .iter()
            .filter(|(v, t)| {
                vars.get(v.0 as usize)
                    .is_some_and(|var| var.never_reassigned)
                    && !matches!(types.get(**t), Ty::Record(r) if r.open)
            })
            .map(|(&v, &t)| (v, t))
            .collect();
        let saved_live = self.live;
        let saved_scope = self.scope.clone();
        for &tp in &sig.type_params {
            let name = self.env.params[tp.0 as usize].name.clone();
            self.scope.params.push((name, tp));
        }
        self.depth += 1;
        self.live = true;
        self.push_scope();
        if let Some(st) = self_ty {
            self.declare("self", Some(st), st, false, None);
        }
        for (p, sp) in f.params.iter().zip(&sig.params) {
            let declared = p.ty.as_ref().map(|_| sp.ty);
            let t = match &p.default {
                Some(d) => {
                    let dt = self.check(d, Some(sp.ty));
                    let non_nil = self.env.types.non_nil(sp.ty);
                    if p.ty.is_some() {
                        non_nil
                    } else {
                        self.env.types.union(&[non_nil, dt])
                    }
                }
                None => sp.ty,
            };
            // Runtime checks of annotated parameters (spec §7.5), after
            // defaults: a parameter with a default is checked against `T`.
            if p.ty.is_some()
                && let Some(c) = self.value_check(t)
            {
                self.value_checks.insert(p.name.span, c);
            }
            let id = self.declare(
                &p.name.text,
                declared.or(Some(sp.ty)),
                t,
                false,
                Some(p.name.span),
            );
            self.flow.insert(id, t);
        }
        let vararg = sig.rest.as_ref().map(|r| match r {
            Rest::Variadic(t) => *t,
            Rest::Pack(_) => Types::ANY,
        });
        if let (Some(v), Some(t)) = (&f.vararg, vararg)
            && v.ty.is_some()
            && let Some(c) = self.value_check(t)
        {
            self.value_checks.insert(v.span, c);
        }
        self.fns.push(FnCtx {
            expected: annotated_ret.then(|| sig.ret.clone()),
            returns: Vec::new(),
            vararg,
        });
        self.block_stmts(&f.body);
        let ctx = self.fns.pop().expect("function");
        let falls_through = self.live;
        self.pop_scope();
        self.depth -= 1;
        self.flow = saved_flow;
        self.live = saved_live;
        self.scope = saved_scope;
        // Not all paths return (E2009).
        if annotated_ret
            && falls_through
            && sig
                .ret
                .types
                .iter()
                .any(|&t| !self.env.types.is_optional(t))
            && f.ret.is_some()
        {
            self.report("E2009", f.end_span, "not all code paths return a value");
        }
        if !annotated_ret {
            sig.ret = self.union_packs(&ctx.returns, falls_through);
        }
        if let Some(st) = self_ty {
            let _ = st;
        }
        self.env.types.func(sig)
    }

    /// The results of a function without annotation: the union of its
    /// returns, position by position.
    fn union_packs(&mut self, packs: &[Pack], falls_through: bool) -> Pack {
        if packs.is_empty() {
            return Pack::empty();
        }
        let len = packs.iter().map(|p| p.types.len()).max().unwrap_or(0);
        let mut types = Vec::new();
        for i in 0..len {
            let mut members: Vec<TypeId> = packs
                .iter()
                .map(|p| match p.types.get(i) {
                    Some(&t) => t,
                    None => match &p.rest {
                        Some(Rest::Variadic(t)) => *t,
                        _ => Types::NIL,
                    },
                })
                .collect();
            if falls_through {
                members.push(Types::NIL);
            }
            let u = self.env.types.union(&members);
            types.push(self.env.types.widen(u));
        }
        let rest = packs.iter().find_map(|p| p.rest.clone());
        Pack { types, rest }
    }

    /// Checks returned values against the annotated results.
    fn check_pack(&mut self, values: &[Expr], got: &Pack, expected: &Pack, span: Span) {
        for (i, &want) in expected.types.iter().enumerate() {
            let have = match got.types.get(i) {
                Some(&t) => t,
                None => match &got.rest {
                    Some(Rest::Variadic(t)) => *t,
                    Some(Rest::Pack(_)) => Types::ANY,
                    None => Types::NIL,
                },
            };
            let at = values.get(i).map(|v| v.span).unwrap_or(span);
            self.check_assignable(at, have, want);
        }
        if expected.rest.is_none()
            && got.types.len() > expected.types.len()
            && !expected.types.is_empty()
        {
            let at = values
                .get(expected.types.len())
                .map(|v| v.span)
                .unwrap_or(span);
            self.report(
                "E2001",
                at,
                format!(
                    "too many values returned (expected {})",
                    expected.types.len()
                ),
            );
        }
    }

    /// The bodies of a class's methods (spec chapter 3).
    fn class_body(&mut self, c: &ClassDecl) {
        let Some(def) = self.env.lookup_type(&c.name.text, &self.scope) else {
            return;
        };
        let instance = self.env.instance(def);
        let class_obj = self.env.types.intern(Ty::ClassOf(def));
        let saved = self.scope.clone();
        for &p in &self.env.def(def).params.clone() {
            let name = self.env.params[p.0 as usize].name.clone();
            self.scope.params.push((name, p));
        }
        self.scope.self_ty = Some(instance);
        if self.mode == Mode::Strict {
            self.definite_assignment(c, def);
        }
        for m in &c.members {
            let is_static = m.modifiers.contains(&Modifier::Static);
            self.classes.push((def, is_static));
            match &m.kind {
                ClassMemberKind::Field {
                    value: Some(v),
                    name,
                    ..
                } => {
                    let want = if is_static {
                        self.env
                            .def(def)
                            .statics
                            .get(name.text.as_ref())
                            .map(|m| m.ty)
                    } else {
                        self.env
                            .def(def)
                            .members
                            .get(name.text.as_ref())
                            .map(|m| m.ty)
                    };
                    // Instance initializers see `self`.
                    self.push_scope();
                    if !is_static {
                        self.declare("self", Some(instance), instance, false, None);
                    }
                    let t = self.check(v, want);
                    if let Some(w) = want {
                        self.check_assignable(v.span, t, w);
                    }
                    self.pop_scope();
                }
                ClassMemberKind::Method {
                    func,
                    has_body: true,
                    name,
                } => {
                    let expected = if is_static {
                        self.env
                            .def(def)
                            .statics
                            .get(name.text.as_ref())
                            .map(|m| m.ty)
                    } else {
                        self.env
                            .def(def)
                            .members
                            .get(name.text.as_ref())
                            .map(|m| m.ty)
                    };
                    let exp = expected.and_then(|e| self.as_func(e));
                    self.function_body(func, exp, (!is_static).then_some(instance));
                }
                ClassMemberKind::Constructor {
                    func,
                    has_body: true,
                } => {
                    self.function_body(func, None, Some(instance));
                }
                ClassMemberKind::Getter { body, ty, .. } => {
                    let fb = synthetic_body(body, ty.clone(), Vec::new());
                    self.function_body(&fb, None, (!is_static).then_some(instance));
                }
                ClassMemberKind::Setter { body, param, .. } => {
                    let fb = synthetic_body(
                        body,
                        None,
                        vec![wowlua_syntax::ast::Param {
                            decorators: Vec::new(),
                            name: param.name.clone(),
                            ty: param.ty.clone(),
                            default: None,
                        }],
                    );
                    self.function_body(&fb, None, (!is_static).then_some(instance));
                }
                _ => {}
            }
            self.classes.pop();
        }
        self.scope = saved;
        let _ = class_obj;
    }

    /// Strict mode: non-optional instance fields without an initializer
    /// must be assigned by the constructor before it returns or calls a
    /// method on `self` (E2052).
    fn definite_assignment(&mut self, c: &ClassDecl, def: DefId) {
        let required: Vec<&Name> = c
            .members
            .iter()
            .filter(|m| !m.modifiers.contains(&Modifier::Static))
            .filter_map(|m| match &m.kind {
                ClassMemberKind::Field {
                    name,
                    optional: false,
                    value: None,
                    ty,
                } if !matches!(ty.kind, TypeKind::Optional(_)) => Some(name),
                _ => None,
            })
            .collect();
        if required.is_empty() {
            return;
        }
        let ctor = c.members.iter().find_map(|m| match &m.kind {
            ClassMemberKind::Constructor {
                func,
                has_body: true,
            } => Some(func),
            _ => None,
        });
        let mut assigned: FxHashSet<Box<str>> = FxHashSet::default();
        if let Some(f) = ctor {
            for s in &f.body.stmts {
                if let StmtKind::Assign { targets, .. } = &s.kind {
                    for t in targets {
                        if let ExprKind::Field { obj, name } = &t.kind
                            && matches!(&obj.kind, ExprKind::Name(n) if n.text.as_ref() == "self")
                        {
                            assigned.insert(name.text.clone());
                        }
                    }
                }
                // A call of a method of wowlua code on `self` (which may read
                // the fields) or a return ends the check; native methods
                // (`self:SetSize(…)`) cannot read them.
                let method = match &s.kind {
                    StmtKind::Call(e) => match &e.kind {
                        ExprKind::Method { obj, name, .. } if matches!(&obj.kind, ExprKind::Name(n) if n.text.as_ref() == "self") => {
                            Some(name.text.clone())
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let calls_own = method.is_some_and(|m| {
                    self.env
                        .instance_member(def, &[], &m)
                        .is_none_or(|(_, owner)| !self.env.def(owner).native)
                });
                if calls_own || matches!(s.kind, StmtKind::Return(_)) {
                    break;
                }
            }
        }
        for name in required {
            if !assigned.contains(&name.text) {
                let msg = format!(
                    "field `{}` is not assigned in the constructor (initialize it, assign it, or make it optional)",
                    name.text
                );
                self.report("E2052", name.span, msg);
            }
        }
    }

    /// Adds a field to an open table, or checks it against a sealed one.
    pub fn extend_open(&mut self, owner: TypeId, name: &str, value: TypeId) {
        let Ty::Record(mut r) = self.env.types.get(owner).clone() else {
            return;
        };
        if !r.open || r.fields.contains_key(name) {
            return;
        }
        // The field may hold other values of the type later (as in a
        // literal without an expected type); one added as `nil` may hold
        // anything.
        let (value, optional) = if value == Types::NIL {
            (Types::ANY, true)
        } else {
            (self.env.types.widen(value), false)
        };
        r.fields = r.fields.with_appended(
            name.into(),
            Field {
                ty: value,
                optional,
                readonly: false,
            },
        );
        // The owner's local now has the extended type.
        let new = self.env.types.record(r);
        let vars: Vec<VarId> = self
            .flow
            .iter()
            .filter(|(_, t)| **t == owner)
            .map(|(v, _)| *v)
            .collect();
        for v in vars {
            self.flow.insert(v, new);
            if self.vars[v.0 as usize].base == owner {
                self.vars[v.0 as usize].base = new;
            }
        }
    }

    /// The type a class's `self` field access sees (spec §3.1.3).
    pub fn class_of_self(&self) -> Option<DefId> {
        self.classes.last().map(|(d, _)| *d)
    }

    pub fn is_class_def(&self, d: DefId) -> bool {
        self.env.def(d).kind == DefKind::Class
    }
}

/// Whether the file sets the environment of its own code: `setfenv` with a
/// stack level, or with a function the file defines (by name). Setting the
/// environment of other code (a chunk from `loadstring`) does not change
/// how this file's names resolve.
fn calls_setfenv(b: &Block) -> bool {
    // Names of the functions this file defines.
    let mut defined: FxHashSet<Box<str>> = FxHashSet::default();
    wowlua_syntax::visit::walk_stmts(b, &mut |s| match &s.kind {
        StmtKind::LocalFunction { name, .. } => {
            defined.insert(name.text.clone());
        }
        StmtKind::Function { name, .. } => {
            defined.insert(name.path[name.path.len() - 1].text.clone());
        }
        StmtKind::Local { names, values, .. } => {
            for (n, v) in names.iter().zip(values) {
                if matches!(v.kind, ExprKind::Function(_)) {
                    defined.insert(n.name.text.clone());
                }
            }
        }
        _ => {}
    });
    let mut found = false;
    wowlua_syntax::visit::walk_block(b, &mut |e| {
        if let ExprKind::Call { callee, args } = &e.kind
            && matches!(&callee.kind, ExprKind::Name(n) if n.text.as_ref() == "setfenv")
        {
            let own = match args.exprs().first().map(|a| &a.kind) {
                Some(ExprKind::Number(_)) | None => true,
                Some(ExprKind::Name(n)) => defined.contains(&n.text),
                Some(ExprKind::Function(_)) => true,
                _ => true,
            };
            if own {
                found = true;
            }
        }
    });
    found
}

/// The `typeof(expr)` parts of a type.
fn typeof_exprs<'t>(t: &'t Type, out: &mut Vec<&'t Expr>) {
    match &t.kind {
        TypeKind::Typeof(e) => out.push(e),
        TypeKind::Paren(i) | TypeKind::Optional(i) | TypeKind::Array(i) => typeof_exprs(i, out),
        TypeKind::Union(ms) | TypeKind::Intersection(ms) => {
            ms.iter().for_each(|m| typeof_exprs(m, out))
        }
        TypeKind::Table(fields) => {
            for f in fields {
                match f {
                    TableTypeField::Named { ty, .. }
                    | TableTypeField::NamedString { ty, .. }
                    | TableTypeField::Array(ty) => typeof_exprs(ty, out),
                    TableTypeField::Indexer { key, value } => {
                        typeof_exprs(key, out);
                        typeof_exprs(value, out);
                    }
                }
            }
        }
        TypeKind::Ref { args, .. } => {
            for a in args {
                if let TypeArg::Type(t) = a {
                    typeof_exprs(t, out);
                }
            }
        }
        _ => {}
    }
}

/// `E2001` → `W2001` (spec §2.1).
fn warning_code(code: &'static str) -> &'static str {
    use std::sync::{Mutex, OnceLock};
    static CODES: OnceLock<Mutex<FxHashMap<&'static str, &'static str>>> = OnceLock::new();
    let mut map = CODES.get_or_init(Default::default).lock().expect("codes");
    map.entry(code)
        .or_insert_with(|| Box::leak(format!("W{}", &code[1..]).into_boxed_str()))
}

fn exported_names(s: &Stmt) -> Vec<String> {
    match &s.kind {
        StmtKind::Local { names, .. } => names.iter().map(|n| n.name.text.to_string()).collect(),
        StmtKind::Function { name, .. } => vec![name.path[0].text.to_string()],
        StmtKind::Const { name, .. } => vec![name.text.to_string()],
        StmtKind::Class(c) => vec![c.name.text.to_string()],
        StmtKind::Enum(e) => vec![e.name.text.to_string()],
        _ => Vec::new(),
    }
}

/// A function body for an accessor (no `FuncBody` in the syntax tree).
fn synthetic_body(
    body: &Block,
    ret: Option<Type>,
    params: Vec<wowlua_syntax::ast::Param>,
) -> FuncBody {
    FuncBody {
        type_params: Vec::new(),
        params,
        vararg: None,
        ret: ret.map(RetType::Single),
        body: body.clone(),
        params_span: body.span,
        end_span: body.span,
        span: body.span,
    }
}

/// The declarations (by name start) of locals that are assigned from a
/// nested function after their declaration or captured and assigned later;
/// and of those assigned from a nested function.
fn reassigned_in_closures(b: &Block) -> (FxHashSet<u32>, FxHashSet<u32>, FxHashSet<u32>) {
    struct Walk {
        scopes: Vec<FxHashMap<Box<str>, (u32, u32)>>,
        depth: u32,
        out: FxHashSet<u32>,
        /// Locals captured by a nested function.
        captured: FxHashSet<u32>,
        /// Locals assigned after their declaration.
        assigned: FxHashSet<u32>,
    }
    impl Walk {
        fn lookup(&self, n: &str) -> Option<(u32, u32)> {
            self.scopes.iter().rev().find_map(|s| s.get(n).copied())
        }
        fn declare(&mut self, n: &Name) {
            let d = self.depth;
            self.scopes
                .last_mut()
                .expect("scope")
                .insert(n.text.clone(), (n.span.start, d));
        }
        fn assign(&mut self, n: &Name) {
            if let Some((start, d)) = self.lookup(&n.text) {
                self.assigned.insert(start);
                if d < self.depth {
                    self.out.insert(start);
                }
            }
        }
        fn read(&mut self, n: &Name) {
            if let Some((start, d)) = self.lookup(&n.text)
                && d < self.depth
            {
                self.captured.insert(start);
            }
        }
        fn block(&mut self, b: &Block) {
            self.scopes.push(FxHashMap::default());
            for s in &b.stmts {
                self.stmt(s);
            }
            self.scopes.pop();
        }
        fn func(&mut self, f: &FuncBody) {
            self.depth += 1;
            self.scopes.push(FxHashMap::default());
            for p in &f.params {
                self.declare(&p.name);
            }
            self.block(&f.body);
            self.scopes.pop();
            self.depth -= 1;
        }
        fn stmt(&mut self, s: &Stmt) {
            match &s.kind {
                StmtKind::Local { names, values } => {
                    for v in values {
                        self.expr(v);
                    }
                    for n in names {
                        self.declare(&n.name);
                    }
                }
                StmtKind::LocalFunction { name, func } => {
                    self.declare(name);
                    self.func(func);
                }
                StmtKind::Function { name, func } => {
                    if name.path.len() == 1 && name.method.is_none() {
                        self.assign(&name.path[0]);
                    }
                    self.func(func);
                }
                StmtKind::Assign { targets, values } => {
                    for t in targets {
                        match &t.kind {
                            ExprKind::Name(n) => self.assign(n),
                            _ => self.expr(t),
                        }
                    }
                    for v in values {
                        self.expr(v);
                    }
                }
                StmtKind::Compound { target, value, .. } => {
                    if let ExprKind::Name(n) = &target.kind {
                        self.assign(n);
                    }
                    self.expr(value);
                }
                StmtKind::Do(b) => self.block(b),
                StmtKind::While { cond, body } | StmtKind::Repeat { body, cond } => {
                    self.expr(cond);
                    self.block(body);
                }
                StmtKind::If {
                    cond,
                    then_block,
                    elseifs,
                    else_block,
                } => {
                    self.expr(cond);
                    self.block(then_block);
                    for (c, b) in elseifs {
                        self.expr(c);
                        self.block(b);
                    }
                    if let Some(b) = else_block {
                        self.block(b);
                    }
                }
                StmtKind::NumericFor { var, body, .. } => {
                    self.scopes.push(FxHashMap::default());
                    self.declare(&var.name);
                    self.block(body);
                    self.scopes.pop();
                }
                StmtKind::GenericFor { vars, exprs, body } => {
                    for e in exprs {
                        self.expr(e);
                    }
                    self.scopes.push(FxHashMap::default());
                    for v in vars {
                        self.declare(&v.name);
                    }
                    self.block(body);
                    self.scopes.pop();
                }
                StmtKind::Export(inner) => self.stmt(inner),
                StmtKind::Call(e) => self.expr(e),
                StmtKind::Return(values) => {
                    for v in values {
                        self.expr(v);
                    }
                }
                StmtKind::Class(c) => {
                    for m in &c.members {
                        match &m.kind {
                            ClassMemberKind::Method { func, .. }
                            | ClassMemberKind::Constructor { func, .. } => self.func(func),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        fn expr(&mut self, e: &Expr) {
            let mut names: Vec<&Name> = Vec::new();
            wowlua_syntax::visit::walk_expr_shallow(e, &mut |x| {
                if let ExprKind::Name(n) = &x.kind {
                    names.push(n);
                }
            });
            for n in names {
                self.read(n);
            }
            let mut funcs: Vec<&FuncBody> = Vec::new();
            wowlua_syntax::visit::walk_expr_shallow(e, &mut |x| {
                if let ExprKind::Function(f) = &x.kind {
                    funcs.push(f);
                }
            });
            for f in funcs {
                self.func(f);
            }
        }
    }
    let mut w = Walk {
        scopes: vec![FxHashMap::default()],
        depth: 0,
        out: FxHashSet::default(),
        captured: FxHashSet::default(),
        assigned: FxHashSet::default(),
    };
    w.block(b);
    // A captured local assigned later (a forward declaration) may hold
    // any of its values when the closure runs.
    let in_closures = w.out.clone();
    let mut out = w.out;
    out.extend(w.captured.intersection(&w.assigned).copied());
    (out, in_closures, w.assigned)
}

fn literal_string(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::String(lit) => Some(String::from_utf8_lossy(&lit.value).into_owned()),
        _ => None,
    }
}
